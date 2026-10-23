use crate::{
    context::Context,
    engine::{Engine, Outcome},
    history_context::{HistoryContextConfig, select_history},
    local_history::LocalHistory,
};
use nu_ansi_term::{Color, Style};
use reedline::{
    Completer, CompletionResult, CompletionStatus, Hinter, History, Prompt, PromptEditMode,
    PromptHistorySearch, Span, Suggestion,
};
use std::borrow::Cow;

/// FullBuffer is essential: the default menu input would hide the suffix.
pub fn completion_menu() -> reedline::ColumnarMenu {
    use reedline::MenuBuilder;
    reedline::ColumnarMenu::default()
        .with_name("ai_menu")
        .with_columns(1)
        .with_input_mode(reedline::InputMode::FullBuffer)
        .with_output_mode(reedline::OutputMode::SuggestedSpan)
}

/// Native local history is synchronous; AI is a fallback, never a reranker of
/// an already displayed menu. This keeps selection stable when results arrive.
pub struct AiCompleter {
    engine: Engine,
    history: Option<LocalHistory>,
    history_config: HistoryContextConfig,
}
impl AiCompleter {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            history: None,
            history_config: HistoryContextConfig::default(),
        }
    }
    pub fn with_history(
        engine: Engine,
        history: LocalHistory,
        history_config: HistoryContextConfig,
    ) -> Self {
        Self {
            engine,
            history: Some(history),
            history_config,
        }
    }
}
impl Completer for AiCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        self.engine.invalidate_hint();
        let Ok(context) = Context::new(line, pos) else {
            self.engine.clear();
            return CompletionResult::fresh(Vec::<Suggestion>::new());
        };
        if line.starts_with(':') {
            self.engine.prefer_local(context, "", "");
            return CompletionResult::fresh(Vec::<Suggestion>::new());
        }
        if let Some(history) = &self.history {
            let mut seen = std::collections::HashSet::new();
            let mut query =
                reedline::SearchQuery::everything(reedline::SearchDirection::Backward, None);
            query.limit = Some(if pos == line.len() { 20 } else { 500 });
            query.filter = reedline::SearchFilter::from_text_search(
                reedline::CommandLineSearch::Prefix(context.prefix().into()),
                None,
            );
            let suggestions: Vec<_> = history
                .search(query)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|entry| {
                    let candidate = entry.command_line;
                    let rest = candidate.strip_prefix(context.prefix())?;
                    let insertion = rest.strip_suffix(context.suffix())?;
                    if insertion.is_empty() || !seen.insert(insertion.to_string()) {
                        return None;
                    }
                    Some(Suggestion {
                        value: insertion.into(),
                        description: Some("local history".into()),
                        span: Span::new(pos, pos),
                        append_whitespace: false,
                        ..Suggestion::default()
                    })
                })
                .take(20)
                .collect();
            if !suggestions.is_empty() {
                self.engine.prefer_local(context, "", "");
                return CompletionResult::fresh(suggestions);
            }
        }
        let context = match self.history.as_ref() {
            Some(history) => select_history(&context, &history.recent(500), &self.history_config)
                .and_then(|preview| preview.into_context(context)),
            None => Ok(context),
        };
        let outcome = match context {
            Ok(context) => self.engine.request_context(context),
            Err(_) => {
                self.engine.clear();
                Outcome::Idle
            }
        };
        match outcome {
            Outcome::Ready(text) if !text.is_empty() => CompletionResult::fresh(vec![Suggestion {
                value: text,
                description: Some("AI insertion · review before using".into()),
                span: Span::new(pos, pos),
                append_whitespace: false,
                ..Suggestion::default()
            }]),
            Outcome::Pending => CompletionResult::Pending,
            _ => CompletionResult::fresh(Vec::<Suggestion>::new()),
        }
    }
    fn poll_completion(&mut self) -> CompletionStatus {
        match self.engine.poll() {
            (true, _) => CompletionStatus::Ready,
            (_, true) => CompletionStatus::Pending,
            _ => CompletionStatus::Idle,
        }
    }
}

pub struct AiHinter {
    engine: Engine,
    native: reedline::DefaultHinter,
    history_config: HistoryContextConfig,
}
impl AiHinter {
    pub fn new(engine: Engine) -> Self {
        Self::with_history_config(engine, HistoryContextConfig::default())
    }
    pub fn with_history_config(engine: Engine, history_config: HistoryContextConfig) -> Self {
        Self {
            engine,
            native: reedline::DefaultHinter::default(),
            history_config,
        }
    }
}
impl Hinter for AiHinter {
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        history: &dyn History,
        use_ansi_coloring: bool,
        cwd: &str,
    ) -> String {
        self.engine.clear_hint_acceptance();
        self.engine.invalidate_hint();
        let Ok(context) = Context::new(line, pos) else {
            self.engine.clear();
            return String::new();
        };
        if line.starts_with(':') {
            self.engine.prefer_local(context, "", "");
            return String::new();
        }
        // Use the actual Reedline history hinter, preserving its latest-prefix
        // matching and word-token semantics; local matches never wait for AI.
        if pos == line.len() {
            let local = self.native.handle(line, pos, history, false, cwd);
            if !local.is_empty() && !local.chars().any(crate::context::unsafe_char) {
                self.engine
                    .prefer_local(context, &local, &self.native.next_hint_token());
                return paint_hint(local, use_ansi_coloring);
            }
        }
        if pos != line.len() {
            let mut matches =
                reedline::SearchQuery::everything(reedline::SearchDirection::Backward, None);
            matches.limit = Some(500);
            matches.filter = reedline::SearchFilter::from_text_search(
                reedline::CommandLineSearch::Prefix(context.prefix().into()),
                None,
            );
            let has_local = history
                .search(matches)
                .unwrap_or_default()
                .iter()
                .any(|entry| {
                    Context::new(&entry.command_line, entry.command_line.len()).is_ok()
                        && entry
                            .command_line
                            .strip_prefix(context.prefix())
                            .and_then(|rest| rest.strip_suffix(context.suffix()))
                            .is_some_and(|middle| !middle.is_empty())
                });
            if has_local {
                self.engine.prefer_local(context, "", "");
                return String::new();
            }
        }
        let mut query =
            reedline::SearchQuery::everything(reedline::SearchDirection::Backward, None);
        query.limit = Some(500);
        let mut recent: Vec<_> = if self.history_config.enabled {
            history
                .search(query)
                .unwrap_or_default()
                .into_iter()
                .map(|h| h.command_line)
                .collect()
        } else {
            Vec::new()
        };
        recent.reverse();
        let context = match select_history(&context, &recent, &self.history_config)
            .and_then(|preview| preview.into_context(context))
        {
            Ok(context) => context,
            Err(_) => {
                self.engine.clear();
                return String::new();
            }
        };
        let outcome = self.engine.request_context(context.clone());
        // Reedline paints hints at buffer end; middle insertions use the menu.
        if pos != line.len() {
            return String::new();
        }
        match outcome {
            Outcome::Ready(text) if !text.is_empty() => {
                self.engine.publish_rendered_hint(&context, &text);
                paint_hint(text, use_ansi_coloring)
            }
            _ => String::new(),
        }
    }
    fn complete_hint(&self) -> String {
        self.engine.consume_hint_acceptance()
    }
    fn next_hint_token(&self) -> String {
        self.engine.consume_hint_acceptance()
    }
}
fn paint_hint(text: String, colored: bool) -> String {
    if colored {
        Style::new().fg(Color::DarkGray).paint(text).to_string()
    } else {
        text
    }
}

/// Reject unsafe input before Reedline can paint it. Also invalidate rendered
/// hints while parsing edits, since Reedline batches edits before repainting.
pub struct SafeEmacs {
    inner: reedline::Emacs,
    engine: Engine,
}
impl SafeEmacs {
    pub fn new(bindings: reedline::Keybindings, engine: Engine) -> Self {
        Self {
            inner: reedline::Emacs::new(bindings),
            engine,
        }
    }
}
impl reedline::EditMode for SafeEmacs {
    fn parse_event(&mut self, event: reedline::ReedlineRawEvent) -> reedline::ReedlineEvent {
        let event = self.inner.parse_event(event);
        if let Some(word_only) = invokes_hint(&event) {
            let hint = if word_only {
                self.engine.take_rendered_hint_word()
            } else {
                self.engine.take_rendered_hint()
            };
            if let Some(text) = hint {
                self.engine.queue_hint_acceptance(text);
            }
        } else if !matches!(
            event,
            reedline::ReedlineEvent::None
                | reedline::ReedlineEvent::Repaint
                | reedline::ReedlineEvent::Resize(_, _)
        ) {
            self.engine.invalidate_hint();
        }
        filter_input(event)
    }
    fn edit_mode(&self) -> PromptEditMode {
        PromptEditMode::Emacs
    }
}

fn invokes_hint(event: &reedline::ReedlineEvent) -> Option<bool> {
    use reedline::ReedlineEvent as E;
    match event {
        E::HistoryHintComplete => Some(false),
        E::HistoryHintWordComplete => Some(true),
        E::UntilFound(events) => events.first().and_then(invokes_hint),
        _ => None,
    }
}

fn filter_input(event: reedline::ReedlineEvent) -> reedline::ReedlineEvent {
    use reedline::{EditCommand as C, ReedlineEvent as E};
    match event {
        E::Edit(commands) => {
            let unsafe_input = commands.iter().any(|command| match command {
                C::InsertChar(c) | C::ReplaceChar(c) => crate::context::unsafe_char(*c),
                C::InsertString(text) | C::ReplaceChars(_, text) => {
                    text.len() > crate::context::MAX_CONTEXT_BYTES
                        || text.chars().any(crate::context::unsafe_char)
                }
                C::InsertPair { open, close } => {
                    crate::context::unsafe_char(*open) || crate::context::unsafe_char(*close)
                }
                C::InsertNewline | C::InsertNewlineAbove | C::InsertNewlineBelow => true,
                _ => false,
            });
            if unsafe_input {
                E::None
            } else {
                E::Edit(commands)
            }
        }
        E::Multiple(events) => E::Multiple(events.into_iter().map(filter_input).collect()),
        E::UntilFound(events) => E::UntilFound(events.into_iter().map(filter_input).collect()),
        event => event,
    }
}

pub struct AiPrompt {
    pub engine: Engine,
    pub network: bool,
}
impl Prompt for AiPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        "reedline-ai".into()
    }
    fn render_prompt_right(&self) -> Cow<'_, str> {
        match self.engine.outcome() {
            Outcome::Pending => "completion pending".into(),
            Outcome::Error(_) => "completion unavailable".into(),
            _ if self.network => "HTTP · no execution".into(),
            _ => "offline mock · no execution".into(),
        }
    }
    fn render_prompt_indicator(&self, _: PromptEditMode) -> Cow<'_, str> {
        "> ".into()
    }
    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        "... ".into()
    }
    fn render_prompt_history_search_indicator(&self, search: PromptHistorySearch) -> Cow<'_, str> {
        reedline::DefaultPrompt::default()
            .render_prompt_history_search_indicator(search)
            .into_owned()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::MockBackend, engine::EngineConfig};
    use reedline::FileBackedHistory;
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };

    fn engine() -> Engine {
        Engine::new(
            Arc::new(MockBackend),
            EngineConfig {
                debounce: Duration::ZERO,
                ..EngineConfig::default()
            },
        )
    }
    fn ready(engine: &Engine) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(engine.outcome(), Outcome::Pending) {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn fim_uses_zero_width_span_preserving_existing_suffix() {
        let engine = engine();
        let mut completer = AiCompleter::new(engine.clone());
        let line = "git  --short";
        assert!(completer.complete(line, 4).is_pending());
        ready(&engine);
        let result = completer.complete(line, 4);
        let suggestion = &result.suggestions()[0];
        assert_eq!(suggestion.span, Span::new(4, 4));
        assert!(!suggestion.append_whitespace);
        let mut actual = line.to_string();
        actual.replace_range(
            suggestion.span.start..suggestion.span.end,
            &suggestion.value,
        );
        assert_eq!(actual, "git status --short");
    }
    #[test]
    fn input_filter_rejects_controls_and_multiline_paste_before_rendering() {
        use reedline::{EditCommand as C, ReedlineEvent as E};
        for text in ["echo \x1b]0;TITLE\x07", "a\nb", "a\rb", "x\u{202e}y"] {
            assert!(matches!(
                filter_input(E::Edit(vec![C::InsertString(text.into())])),
                E::None
            ));
        }
        assert!(matches!(
            filter_input(E::Edit(vec![C::InsertChar('\x1b')])),
            E::None
        ));
        assert!(matches!(
            filter_input(E::Edit(vec![C::InsertString("echo 世界".into())])),
            E::Edit(_)
        ));
    }

    #[test]
    fn rendered_hint_is_one_shot_and_edits_invalidate_it() {
        let engine = engine();
        let mut hinter = AiHinter::new(engine.clone());
        let history = FileBackedHistory::new(0).unwrap();
        hinter.handle("git st", 6, &history, false, "");
        ready(&engine);
        hinter.handle("git st", 6, &history, false, "");
        assert_eq!(engine.take_rendered_hint(), Some("atus".into()));
        assert_eq!(engine.take_rendered_hint(), None);
        hinter.handle("git st", 6, &history, false, "");
        engine.invalidate_hint();
        assert_eq!(engine.take_rendered_hint(), None);
    }

    #[test]
    fn hints_only_at_end_and_stale_hint_cannot_be_accepted() {
        let engine = engine();
        let mut hinter = AiHinter::new(engine.clone());
        let history = FileBackedHistory::new(0).unwrap();
        hinter.handle("git st", 6, &history, false, "/PRIVATE");
        ready(&engine);
        assert_eq!(
            hinter.handle("git st", 6, &history, false, "/PRIVATE"),
            "atus"
        );
        assert_eq!(engine.take_rendered_hint(), Some("atus".into()));
        engine.request("git d", 5);
        assert_eq!(engine.take_rendered_hint(), None);
        assert_eq!(
            hinter.handle("git  --short", 4, &history, false, "/PRIVATE"),
            ""
        );
        ready(&engine);
        assert_eq!(
            hinter.handle("git  --short", 4, &history, false, "/PRIVATE"),
            ""
        );
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    use crate::{
        backend::{Backend, BackendError},
        engine::EngineConfig,
    };
    use reedline::{FileBackedHistory, HistoryItem};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    struct CountingBackend(Arc<AtomicUsize>);
    impl Backend for CountingBackend {
        fn complete(&self, _: &Context) -> Result<String, BackendError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok("AI must not replace local".into())
        }
    }
    #[test]
    fn native_hint_and_word_are_immediate_without_ai_requests() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig {
                debounce: Duration::ZERO,
                ..EngineConfig::default()
            },
        );
        let mut history = FileBackedHistory::new(10).unwrap();
        history
            .save(HistoryItem::from_command_line("git status --short"))
            .unwrap();
        let mut hinter = AiHinter::new(engine.clone());
        assert_eq!(
            hinter.handle("git ", 4, &history, false, ""),
            "status --short"
        );
        assert_eq!(engine.take_rendered_hint_word(), Some("status".into()));
        assert_eq!(engine.take_rendered_hint(), None);
        assert_eq!(engine.outcome(), Outcome::Idle);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn native_local_menu_is_immediate_fim_and_stable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig::default(),
        );
        let mut history = LocalHistory::new(None).unwrap();
        history
            .save(HistoryItem::from_command_line("git status --short"))
            .unwrap();
        history
            .save(HistoryItem::from_command_line("git diff --short"))
            .unwrap();
        let mut completer =
            AiCompleter::with_history(engine.clone(), history, HistoryContextConfig::default());
        let completion = completer.complete("git  --short", 4);
        assert!(!completion.is_pending());
        assert_eq!(
            completion
                .suggestions()
                .iter()
                .map(|s| s.value.as_str())
                .collect::<Vec<_>>(),
            ["diff", "status"]
        );
        assert!(
            completion
                .suggestions()
                .iter()
                .all(|s| s.span == Span::new(4, 4))
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(completer.poll_completion(), CompletionStatus::Idle);
    }
    #[test]
    fn local_fim_and_hinter_do_not_queue_ai_even_with_history_upload_enabled() {
        let calls = Arc::new(AtomicUsize::new(0));
        let engine = Engine::new(
            Arc::new(CountingBackend(calls.clone())),
            EngineConfig {
                debounce: Duration::ZERO,
                ..EngineConfig::default()
            },
        );
        let mut history = LocalHistory::new(None).unwrap();
        history
            .save(HistoryItem::from_command_line("git status --short"))
            .unwrap();
        let config = HistoryContextConfig {
            enabled: true,
            ..HistoryContextConfig::default()
        };
        let mut completer =
            AiCompleter::with_history(engine.clone(), history.clone(), config.clone());
        let mut hinter = AiHinter::with_history_config(engine.clone(), config);
        assert_eq!(
            completer.complete("git  --short", 4).suggestions()[0].value,
            "status"
        );
        assert_eq!(hinter.handle("git  --short", 4, &history, false, ""), "");
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(engine.outcome(), Outcome::Idle);
    }

    #[test]
    fn native_word_binding_keeps_word_semantics_and_acceptance_is_one_shot() {
        use reedline::{KeyCode, KeyModifiers, default_emacs_keybindings};
        let bindings = default_emacs_keybindings();
        let event = bindings
            .find_binding(KeyModifiers::ALT, KeyCode::Char('f'))
            .unwrap();
        assert_eq!(invokes_hint(&event), Some(true));
        let engine = Engine::new(
            Arc::new(CountingBackend(Arc::new(AtomicUsize::new(0)))),
            EngineConfig::default(),
        );
        engine.prefer_local(Context::new("git ", 4).unwrap(), "status --short", "status");
        let text = engine.take_rendered_hint_word().unwrap();
        engine.queue_hint_acceptance(text);
        assert_eq!(engine.consume_hint_acceptance(), "status");
        assert_eq!(engine.consume_hint_acceptance(), "");
    }
}
