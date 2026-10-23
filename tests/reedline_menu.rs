//! Exercise the real Reedline menu, not only our replacement arithmetic.
use reedline::{Editor, Menu, UndoBehavior};
use reedline_ai::{
    backend::MockBackend,
    editor::{AiCompleter, completion_menu},
    engine::{Engine, EngineConfig, Outcome},
};
use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn set_buffer(editor: &mut Editor, text: &str, cursor: usize) {
    editor.edit_buffer(
        |buffer| {
            buffer.set_buffer(text.into());
            buffer.set_insertion_point(cursor);
        },
        UndoBehavior::CreateUndoPoint,
    );
}
fn await_completion(engine: &Engine) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while matches!(engine.outcome(), Outcome::Pending) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}
fn engine() -> Engine {
    Engine::new(
        Arc::new(MockBackend),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    )
}

#[test]
fn full_buffer_menu_preserves_suffix_and_places_cursor_after_insertion() {
    let engine = engine();
    let mut completer = AiCompleter::new(engine.clone());
    let mut menu = completion_menu();
    let mut editor = Editor::default();
    set_buffer(&mut editor, "git  --short", 4);
    menu.update_values(&mut editor, &mut completer);
    await_completion(&engine);
    menu.update_values(&mut editor, &mut completer);
    assert_eq!(menu.get_values()[0].value, "status");
    menu.replace_in_buffer(&mut editor);
    assert_eq!(editor.get_buffer(), "git status --short");
    assert_eq!(editor.line_buffer().insertion_point(), 10);
}

#[test]
fn menu_does_not_apply_a_result_to_an_edited_buffer() {
    let engine = engine();
    let mut completer = AiCompleter::new(engine.clone());
    let mut menu = completion_menu();
    let mut editor = Editor::default();
    set_buffer(&mut editor, "git st", 6);
    menu.update_values(&mut editor, &mut completer);
    await_completion(&engine);
    menu.update_values(&mut editor, &mut completer);
    assert_eq!(menu.get_values()[0].value, "atus");
    set_buffer(&mut editor, "echo safe", 9);
    menu.replace_in_buffer(&mut editor);
    assert_eq!(editor.get_buffer(), "echo safe");
}

#[test]
fn unicode_insertion_works_through_reedline() {
    let engine = engine();
    let mut completer = AiCompleter::new(engine.clone());
    let mut menu = completion_menu();
    let mut editor = Editor::default();
    set_buffer(&mut editor, "echo 界", 5);
    menu.update_values(&mut editor, &mut completer);
    await_completion(&engine);
    menu.update_values(&mut editor, &mut completer);
    menu.replace_in_buffer(&mut editor);
    assert_eq!(editor.get_buffer(), "echo 世界");
    assert_eq!(editor.line_buffer().insertion_point(), 8);
}

#[test]
fn generated_whole_result_risk_cannot_use_hint_acceptance() {
    use reedline::{Completer, FileBackedHistory, Hinter};
    use reedline_ai::{
        backend::{Backend, BackendError},
        context::Context,
        editor::AiHinter,
        review_menu::ReviewGate,
        risk::RiskPolicy,
        shell::{ShellContext, ShellKind},
    };
    struct Fixed;
    impl Backend for Fixed {
        fn complete(&self, _: &Context) -> Result<String, BackendError> {
            Ok("value".into())
        }
    }
    let engine = Engine::new(
        Arc::new(Fixed),
        EngineConfig {
            debounce: Duration::ZERO,
            ..EngineConfig::default()
        },
    );
    let gate = ReviewGate::default();
    let shell = Some(ShellContext::new(ShellKind::Bash));
    let mut hinter = AiHinter::new(engine.clone())
        .with_local_context(shell.clone(), None)
        .with_risk_review(RiskPolicy::Confirm, gate.clone());
    let history = FileBackedHistory::new(0).unwrap();
    let line = "echo > /synthetic/output ";
    hinter.handle(line, line.len(), &history, false, "");
    await_completion(&engine);
    assert_eq!(hinter.handle(line, line.len(), &history, false, ""), "");
    assert!(gate.status().contains("overwrite"));
    assert!(engine.take_rendered_hint().is_none());
    assert!(engine.take_rendered_hint_word().is_none());
    let mut completer = AiCompleter::new(engine)
        .with_local_context(shell, None)
        .with_risk_review(RiskPolicy::Confirm, gate);
    let result = completer.complete(line, line.len());
    assert_eq!(result.suggestions()[0].value, "value");
    assert!(
        result.suggestions()[0]
            .description
            .as_ref()
            .unwrap()
            .contains("overwrite")
    );
}
