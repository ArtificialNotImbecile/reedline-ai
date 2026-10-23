use crate::{
    context::Context,
    engine::{Engine, Outcome},
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

pub struct AiCompleter {
    engine: Engine,
}
impl AiCompleter {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}
impl Completer for AiCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        self.engine.invalidate_hint();
        match self.engine.request(line, pos) {
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
}
impl AiHinter {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }
}
impl Hinter for AiHinter {
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        _history: &dyn History,
        use_ansi_coloring: bool,
        _cwd: &str,
    ) -> String {
        let outcome = self.engine.request(line, pos);
        self.engine.invalidate_hint();
        // Reedline paints hints after the buffer. Never display middle insertion
        // there; use the FullBuffer completion menu for FIM instead.
        if pos != line.len() {
            return String::new();
        }
        match outcome {
            Outcome::Ready(text) if !text.is_empty() => {
                if let Ok(context) = Context::new(line, pos) {
                    self.engine.publish_rendered_hint(&context, &text);
                }
                if use_ansi_coloring {
                    Style::new().fg(Color::DarkGray).paint(text).to_string()
                } else {
                    text
                }
            }
            _ => String::new(),
        }
    }
    fn complete_hint(&self) -> String {
        // Native acceptance has no access to the current buffer inside an input
        // batch. SafeEmacs consumes the shown token into an ordered InsertString.
        String::new()
    }
    fn next_hint_token(&self) -> String {
        String::new()
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
        if invokes_hint(&event) {
            if let Some(text) = self.engine.take_rendered_hint() {
                return reedline::ReedlineEvent::Edit(vec![
                    // Match Reedline's native hint acceptance: collapse any
                    // selection and append, rather than replacing selected text.
                    reedline::EditCommand::MoveToEnd { select: false },
                    reedline::EditCommand::InsertString(text),
                ]);
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

fn invokes_hint(event: &reedline::ReedlineEvent) -> bool {
    use reedline::ReedlineEvent as E;
    match event {
        E::HistoryHintComplete | E::HistoryHintWordComplete => true,
        E::UntilFound(events) => matches!(
            events.first(),
            Some(E::HistoryHintComplete | E::HistoryHintWordComplete)
        ),
        _ => false,
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
    fn render_prompt_history_search_indicator(&self, _: PromptHistorySearch) -> Cow<'_, str> {
        "search disabled> ".into()
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
