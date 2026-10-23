//! Explicit line-oriented interaction. No raw mode, hotkeys or shell execution.
use crate::{
    context::{Context, MAX_CONTEXT_BYTES, safe_display},
    engine::{Engine, Outcome},
    history_context::{HistoryContextConfig, select_history},
    local_history::LocalHistory,
    project_memory::ProjectMemory,
    risk::{RiskPolicy, review},
    shell::ShellContext,
    structured_edit::{StructuredSuggestion, TextEdit},
    workflow::suggest_local,
};
use reedline::{History, HistoryItem};
use std::{
    io::{self, BufRead, Write},
    sync::mpsc,
    time::Duration,
};
use unicode_width::UnicodeWidthChar;

type Error = Box<dyn std::error::Error>;

struct Candidate {
    edit: StructuredSuggestion,
    source: &'static str,
    steps: Vec<String>,
    warnings: Vec<&'static str>,
}

struct MobileSession {
    engine: Engine,
    history: LocalHistory,
    history_config: HistoryContextConfig,
    shell: Option<ShellContext>,
    memory: Option<ProjectMemory>,
    policy: RiskPolicy,
    buffer: String,
    pending: Option<Context>,
    candidates: Vec<Candidate>,
    selected: Option<usize>,
    armed: bool,
}

impl MobileSession {
    fn invalidate(&mut self) {
        self.engine.clear();
        self.pending = None;
        self.candidates.clear();
        self.selected = None;
        self.armed = false;
    }

    fn context(&self) -> Result<Context, Error> {
        let mut context = Context::new(&self.buffer, self.buffer.len())?;
        if let Some(shell) = &self.shell {
            context = context.with_shell(shell.clone())?;
        }
        Ok(context)
    }

    fn request(&mut self) -> Result<Vec<String>, Error> {
        self.invalidate();
        let context = self.context()?;
        if context.is_empty() {
            return Ok(vec!["Enter some text first. No request sent.".into()]);
        }
        for line in self.history.recent(500).into_iter().rev() {
            if line.starts_with(&self.buffer)
                && line != self.buffer
                && !self
                    .candidates
                    .iter()
                    .any(|c| c.edit.edit.replacement == line)
            {
                self.candidates.push(Candidate {
                    edit: StructuredSuggestion::new(
                        &self.buffer,
                        self.buffer.len(),
                        TextEdit {
                            start: 0,
                            end: self.buffer.len(),
                            replacement: line,
                        },
                    )?,
                    source: "local history",
                    steps: vec![],
                    warnings: vec![],
                });
                if self.candidates.len() == 5 {
                    break;
                }
            }
        }
        if self.candidates.is_empty() {
            if let Some(local) = suggest_local(&context, self.memory.as_ref())? {
                self.candidates.push(Candidate {
                    edit: local.suggestion,
                    source: local.source,
                    steps: local.steps,
                    warnings: local.warnings,
                });
            } else {
                let context =
                    select_history(&context, &self.history.recent(500), &self.history_config)?
                        .into_context(context)?;
                self.engine.request_context(context.clone());
                self.pending = Some(context);
                return Ok(vec![
                    "Waiting for completion. You can edit, cancel or quit.".into(),
                ]);
            }
        }
        Ok(self.list())
    }

    fn list(&self) -> Vec<String> {
        let mut lines = vec!["Candidates (type a number to preview):".into()];
        for (index, candidate) in self.candidates.iter().enumerate() {
            let text = &candidate.edit.edit.replacement;
            let short: String = text.chars().take(120).collect();
            lines.push(format!(
                "{}. [{}] {}{}",
                index + 1,
                candidate.source,
                short,
                if text.chars().count() > 120 {
                    " [preview for full text]"
                } else {
                    ""
                }
            ));
        }
        lines.push("Type cancel to discard.".into());
        lines
    }

    fn poll(&mut self) -> Result<Vec<String>, Error> {
        let Some(context) = self.pending.as_ref() else {
            return Ok(vec![]);
        };
        match self.engine.outcome_for(context) {
            Outcome::Ready(insertion) => {
                let context = self.pending.take().expect("pending snapshot");
                if context.prefix() != self.buffer || !context.suffix().is_empty() {
                    self.invalidate();
                    return Ok(vec![]);
                }
                if insertion.is_empty() {
                    return Ok(vec!["No completion. Edit the text or try again.".into()]);
                }
                let result = context.insert(&insertion)?;
                self.candidates.push(Candidate {
                    edit: StructuredSuggestion::new(
                        &self.buffer,
                        self.buffer.len(),
                        TextEdit {
                            start: 0,
                            end: self.buffer.len(),
                            replacement: result,
                        },
                    )?,
                    source: "backend completion",
                    steps: vec![],
                    warnings: vec![],
                });
                Ok(self.list())
            }
            Outcome::Error(error) => {
                self.invalidate();
                Ok(vec![format!("Completion unavailable: {error}")])
            }
            _ => Ok(vec![]),
        }
    }

    fn preview(&self, index: usize) -> Result<Vec<String>, Error> {
        let candidate = &self.candidates[index];
        let (result, _) = candidate.edit.apply(&self.buffer, self.buffer.len())?;
        let mut lines = vec![format!("Preview {} [{}]", index + 1, candidate.source)];
        for (index, step) in candidate.steps.iter().enumerate() {
            lines.push(format!("Step {}: {step}", index + 1));
        }
        lines.push("Full command (visual wraps are not newlines):".into());
        lines.push(result.clone());
        lines.extend(candidate.warnings.iter().map(|s| s.to_string()));
        if self.policy != RiskPolicy::Off {
            let risk = review(self.shell.as_ref(), &result);
            lines.extend(risk.warnings.into_iter().map(|s| format!("Warning: {s}")));
            if risk.incomplete {
                lines.push("Review incomplete; no safety guarantee.".into());
            }
        }
        lines.push("Type accept or cancel. Never executes.".into());
        Ok(lines)
    }

    // Returns whether to exit. Every acceptance revalidates the full source.
    fn input(&mut self, input: &str) -> Result<(bool, Vec<String>), Error> {
        let was_armed = std::mem::take(&mut self.armed);
        match input.trim() {
            "quit" | ":quit" => {
                self.invalidate();
                return Ok((true, vec![]));
            }
            "cancel" => {
                self.invalidate();
                return Ok((
                    false,
                    vec!["Cancelled. Type new text or suggest to retry.".into()],
                ));
            }
            "help" => return Ok((false, help())),
            "" => return Ok((false, vec!["No input. Type help for commands.".into()])),
            "suggest" => return self.request().map(|lines| (false, lines)),
            "accept" | "confirm" => {
                let Some(index) = self.selected else {
                    return Ok((false, vec!["Choose a candidate number first.".into()]));
                };
                let candidate = &self.candidates[index];
                let (result, _) = candidate.edit.apply(&self.buffer, self.buffer.len())?;
                if input.trim() == "confirm" && !was_armed {
                    return Ok((
                        false,
                        vec!["Nothing awaiting confirmation. Preview and accept first.".into()],
                    ));
                }
                // Mobile reviews ALL candidates, including history; desktop native history is unchanged.
                if review(self.shell.as_ref(), &result).requires_confirmation(self.policy)
                    && !(input.trim() == "confirm" && was_armed)
                {
                    self.armed = true;
                    return Ok((false, vec!["Risk or incomplete review. Type confirm to accept this preview, or cancel.".into()]));
                }
                self.history
                    .save(HistoryItem::from_command_line(result.clone()))?;
                self.history.sync()?;
                self.invalidate();
                self.buffer.clear();
                return Ok((false, vec!["Accepted (not executed):".into(), result]));
            }
            "history-clear" | ":history-clear" => {
                self.invalidate();
                self.history.clear()?;
                self.history.sync()?;
                return Ok((false, vec!["Local history and cache cleared. Already-sent requests cannot be recalled.".into()]));
            }
            _ => {}
        }
        if let Ok(number) = input.trim().parse::<usize>() {
            self.selected = None;
            if number == 0 || number > self.candidates.len() {
                return Ok((
                    false,
                    vec!["No such candidate. Type a listed number.".into()],
                ));
            }
            self.selected = Some(number - 1);
            return self.preview(number - 1).map(|lines| (false, lines));
        }
        self.invalidate();
        let edit = input.strip_prefix("edit ");
        let text = edit.unwrap_or(input);
        self.buffer.clear();
        Context::new(text, text.len())?;
        self.buffer = text.to_owned();
        if edit.is_some() {
            Ok((
                false,
                vec!["Text updated. Type suggest for candidates.".into()],
            ))
        } else {
            self.request().map(|lines| (false, lines))
        }
    }
}

fn help() -> Vec<String> {
    [
        "Mobile mode: type text, then Enter for candidates.",
        "1..5: preview; accept: keep text; cancel: discard preview.",
        "edit TEXT: replace text without a request (also escapes command words).",
        "suggest: request again; help: commands; quit: exit.",
        "history-clear: clear this app's history and cache.",
        "No Ctrl/Alt/function keys needed. No commands are executed.",
    ]
    .map(String::from)
    .to_vec()
}

/// Bounded, terminal-safe, column-aware output; never truncate an accepted command.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.clamp(2, 80);
    let text = safe_display(text);
    let mut lines = vec![];
    let mut line = String::new();
    let mut columns = 0;
    for c in text.chars().take(MAX_CONTEXT_BYTES + 256) {
        let size = c.width().unwrap_or(0);
        if columns + size > width {
            lines.push(std::mem::take(&mut line));
            columns = 0;
        }
        line.push(c);
        columns += size;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn print(lines: Vec<String>) -> io::Result<()> {
    let width =
        crossterm::terminal::size().map_or(40, |(columns, _)| columns.saturating_sub(1) as usize);
    let mut stdout = io::stdout().lock();
    for text in lines {
        for line in wrap(&text, width) {
            writeln!(stdout, "{line}")?;
        }
    }
    stdout.flush()
}

// Drain oversized lines before reporting, never pass partial commands to inference.
fn read_input(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let mut oversized = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            // EOF is cancellation, not an implicit submission of partial input.
            return Ok(None);
        }
        let len = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map_or(chunk.len(), |n| n + 1);
        let done = chunk[len - 1] == b'\n';
        if bytes.len() + len > MAX_CONTEXT_BYTES + 16 {
            oversized = true;
        }
        if !oversized {
            bytes.extend_from_slice(&chunk[..len]);
        }
        reader.consume(len);
        if done {
            break;
        }
    }
    if oversized {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input exceeds the mobile line limit",
        ));
    }
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "input is not UTF-8"))
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    engine: Engine,
    history: LocalHistory,
    history_config: HistoryContextConfig,
    shell: Option<ShellContext>,
    memory: Option<ProjectMemory>,
    policy: RiskPolicy,
    network: bool,
) -> Result<(), Error> {
    print(help())?;
    print(vec![
        format!("Local risk policy: {policy:?}. Advisory only."),
        if network {
            "HTTP: submitted text is sent only when requesting candidates. Never type secrets."
                .into()
        } else {
            "Offline mock backend.".into()
        },
        format!(
            "AI history upload: {}. Target shell is explicitly selected, never inferred from SSH.",
            if history_config.enabled { "ON" } else { "OFF" }
        ),
    ])?;
    let mut session = MobileSession {
        engine,
        history,
        history_config,
        shell,
        memory,
        policy,
        buffer: String::new(),
        pending: None,
        candidates: vec![],
        selected: None,
        armed: false,
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        loop {
            let input = read_input(&mut stdin);
            let stop = matches!(&input, Ok(None))
                || input
                    .as_ref()
                    .is_err_and(|e| e.kind() != io::ErrorKind::InvalidData);
            if sender.send(input).is_err() || stop {
                break;
            }
        }
    });
    loop {
        match receiver.recv_timeout(Duration::from_millis(30)) {
            Ok(Ok(Some(input))) => match session.input(&input) {
                Ok((true, _)) => break,
                Ok((false, lines)) => print(lines)?,
                Err(error) => {
                    session.invalidate();
                    print(vec![format!("Unavailable: {error}")])?;
                }
            },
            Ok(Ok(None)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                session.invalidate();
                break;
            }
            Ok(Err(error)) => {
                session.invalidate();
                if error.kind() != io::ErrorKind::InvalidData {
                    return Err(error.into());
                }
                print(vec![format!("Input rejected: {error}")])?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        match session.poll() {
            Ok(lines) => print(lines)?,
            Err(error) => {
                session.invalidate();
                print(vec![format!("Completion unavailable: {error}")])?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::{Backend, MockBackend},
        engine::EngineConfig,
        shell::ShellKind,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    fn session(policy: RiskPolicy) -> MobileSession {
        MobileSession {
            engine: Engine::new(
                Arc::new(MockBackend),
                EngineConfig {
                    debounce: Duration::ZERO,
                    ..EngineConfig::default()
                },
            ),
            history: LocalHistory::new(None).unwrap(),
            history_config: HistoryContextConfig::default(),
            shell: Some(ShellContext::new(ShellKind::Bash)),
            memory: None,
            policy,
            buffer: String::new(),
            pending: None,
            candidates: vec![],
            selected: None,
            armed: false,
        }
    }
    fn candidate(s: &mut MobileSession, result: &str) {
        s.buffer = "intent".into();
        s.candidates.push(Candidate {
            edit: StructuredSuggestion::new(
                "intent",
                6,
                TextEdit {
                    start: 0,
                    end: 6,
                    replacement: result.into(),
                },
            )
            .unwrap(),
            source: "test",
            steps: vec![],
            warnings: vec![],
        });
        s.input("1").unwrap();
    }
    fn accepted(lines: &[String]) -> bool {
        lines.iter().any(|s| s == "Accepted (not executed):")
    }
    #[test]
    fn risky_accept_requires_distinct_confirm_and_is_single_use() {
        let mut s = session(RiskPolicy::Confirm);
        candidate(&mut s, "rm -rf /synthetic");
        assert!(!accepted(&s.input("confirm").unwrap().1));
        assert!(!accepted(&s.input("accept").unwrap().1));
        assert!(accepted(&s.input("confirm").unwrap().1));
        assert!(!accepted(&s.input("confirm").unwrap().1));
    }
    #[test]
    fn changes_and_cancellation_cannot_reuse_acknowledgment() {
        for action in ["cancel", "edit changed", "1", "help", "", "99"] {
            let mut s = session(RiskPolicy::Confirm);
            candidate(&mut s, "rm -rf /synthetic");
            s.input("accept").unwrap();
            s.input(action).unwrap();
            assert!(!accepted(&s.input("confirm").unwrap().1), "{action}");
        }
        let mut s = session(RiskPolicy::Off);
        candidate(&mut s, "echo hello");
        s.buffer = "changed".into();
        assert!(s.input("accept").is_err());
    }
    #[test]
    fn warn_and_off_accept_without_extra_step_but_require_preview() {
        for policy in [RiskPolicy::Warn, RiskPolicy::Off] {
            let mut s = session(policy);
            candidate(&mut s, "rm -rf /synthetic");
            s.selected = None;
            assert!(!accepted(&s.input("accept").unwrap().1));
            s.input("1").unwrap();
            assert!(accepted(&s.input("accept").unwrap().1));
        }
    }
    struct Count(Arc<AtomicUsize>);
    impl Backend for Count {
        fn complete(&self, _: &Context) -> Result<String, crate::backend::BackendError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok("unused".into())
        }
    }
    #[test]
    fn blank_and_local_history_do_not_call_backend() {
        let count = Arc::new(AtomicUsize::new(0));
        let mut s = session(RiskPolicy::Warn);
        s.engine = Engine::new(
            Arc::new(Count(count.clone())),
            EngineConfig {
                debounce: Duration::ZERO,
                ..EngineConfig::default()
            },
        );
        s.input("   ").unwrap();
        s.input("suggest").unwrap();
        s.history
            .save(HistoryItem::from_command_line("echo 世界"))
            .unwrap();
        s.input("echo 世").unwrap();
        assert_eq!(s.candidates.len(), 1);
        s.input("1").unwrap();
        assert!(s.input("accept").unwrap().1.contains(&"echo 世界".into()));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn bounded_unicode_rendering_and_reader_errors() {
        use unicode_width::UnicodeWidthStr;
        for width in [2, 8, 19, 39] {
            for line in wrap("echo 世界 e\u{301} 👩‍💻 \x1b[31m \u{202e}", width) {
                assert!(line.width() <= width);
                assert!(!line.contains('\x1b'));
                assert!(!line.contains('\u{202e}'));
            }
        }
        assert!(wrap(&"x".repeat(100_000), 20).concat().len() <= MAX_CONTEXT_BYTES + 256);
        let mut reader = io::Cursor::new(format!("{}\nvalid\n", "x".repeat(9000)));
        assert!(read_input(&mut reader).is_err());
        assert_eq!(read_input(&mut reader).unwrap(), Some("valid".into()));
        assert_eq!(read_input(&mut reader).unwrap(), None);
        assert_eq!(read_input(&mut io::Cursor::new("partial")).unwrap(), None);
        assert!(read_input(&mut io::Cursor::new(vec![255, 10])).is_err());
    }
    #[test]
    fn workflow_preview_has_real_steps_not_shell_splitting() {
        let mut s = session(RiskPolicy::Confirm);
        s.memory = Some(
            serde_json::from_str(include_str!("../examples/project-memory.synthetic.json"))
                .unwrap(),
        );
        s.input("run CCTV").unwrap();
        let preview = s.input("1").unwrap().1;
        assert!(preview.iter().any(|s| s.starts_with("Step 1: cd ")));
        assert!(preview.iter().any(|s| s == "Step 2: 'python' 'app.py'"));
        assert!(preview.iter().any(|s| s.contains(" && ")));
    }
    #[test]
    fn cancelled_and_edited_pending_results_stay_hidden() {
        let mut s = session(RiskPolicy::Warn);
        s.input("git st").unwrap();
        s.input("cancel").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(s.poll().unwrap().is_empty());
        assert!(s.candidates.is_empty());
        s.input("git st").unwrap();
        s.input("edit echo 世").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(s.poll().unwrap().is_empty());
        assert!(s.candidates.is_empty());
    }
    struct Failure;
    impl Backend for Failure {
        fn complete(&self, _: &Context) -> Result<String, crate::backend::BackendError> {
            Err(crate::backend::BackendError::Config("synthetic failure"))
        }
    }
    #[test]
    fn backend_error_cannot_leave_an_acceptance_candidate() {
        let mut s = session(RiskPolicy::Confirm);
        s.engine = Engine::new(
            Arc::new(Failure),
            EngineConfig {
                debounce: Duration::ZERO,
                ..EngineConfig::default()
            },
        );
        s.input("git st").unwrap();
        for _ in 0..100 {
            let lines = s.poll().unwrap();
            if !lines.is_empty() {
                assert!(lines[0].contains("Completion unavailable"));
                assert!(s.candidates.is_empty());
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("error did not arrive");
    }
}
