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
