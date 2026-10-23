//! Explicit review layered over Reedline's native menu and undo-aware acceptance.
//!
//! Risky previews require F2 followed by Enter. An unapproved Enter leaves
//! the buffer untouched; Reedline then closes the menu, which must be reopened.
use reedline::{
    ColumnarMenu, Completer, Editor, Menu, MenuEvent, MenuSettings, Painter, Suggestion,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    snapshot: Option<Snapshot>,
    status: String,
    generation: u64,
    active: bool,
    ready: bool,
    rendered: bool,
    authorized: bool,
    acceptance_locked: bool,
}
struct Snapshot {
    source: String,
    cursor: usize,
    required: bool,
}

/// Shared by the completer, key parser, and menu. Approval never outlives a
/// publication, edit, selection change, cancellation, or acceptance.
#[derive(Clone, Default)]
pub struct ReviewGate(Arc<Mutex<State>>);
impl ReviewGate {
    pub fn set_status(&self, status: String) {
        self.0.lock().unwrap().status = status;
    }

    pub fn status(&self) -> String {
        self.0.lock().unwrap().status.clone()
    }

    pub fn publish(&self, source_buffer: &str, cursor: usize, needs_confirmation: bool) {
        let mut state = self.0.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.acceptance_locked = false;
        state.snapshot = Some(Snapshot {
            source: source_buffer.into(),
            cursor,
            required: needs_confirmation,
        });
        state.ready = false;
        state.rendered = false;
        state.authorized = false;
    }

    /// Withdraw the current preview while preserving the menu's active status.
    pub fn clear(&self) {
        let mut state = self.0.lock().unwrap();
        state.snapshot = None;
        state.acceptance_locked = false;
        state.status.clear();
        state.ready = false;
        state.rendered = false;
        state.authorized = false;
    }

    /// Freeze authorization when Enter is parsed. Reedline parses a whole input
    /// batch before handling it, so a later F2 must not approve an earlier Enter.
    /// Existing authorization is retained for the intended F2-then-Enter order.
    pub fn lock_acceptance(&self) {
        self.0.lock().unwrap().acceptance_locked = true;
    }

    /// Arms only the fresh preview actually displayed in an active menu.
    pub fn authorize(&self) -> bool {
        let mut state = self.0.lock().unwrap();
        if !state.acceptance_locked
            && state.active
            && state.ready
            && state.rendered
            && state.snapshot.is_some()
        {
            state.authorized = true;
            true
        } else {
            false
        }
    }

    pub fn invalidate(&self) {
        let mut state = self.0.lock().unwrap();
        state.ready = false;
        state.rendered = false;
        state.authorized = false;
    }
}

pub struct ReviewMenu {
    inner: ColumnarMenu,
    gate: ReviewGate,
    generation: u64,
}
impl ReviewMenu {
    pub fn new(inner: ColumnarMenu, gate: ReviewGate) -> Self {
        Self {
            inner,
            gate,
            generation: 0,
        }
    }

    fn synchronize(&mut self, editor: &Editor) {
        let mut state = self.gate.0.lock().unwrap();
        self.generation = state.generation;
        state.active = self.inner.is_active();
        state.ready = state.active
            && !self.inner.results_are_provisional()
            && !self.inner.get_values().is_empty()
            && state.snapshot.as_ref().is_some_and(|s| {
                s.source == editor.get_buffer()
                    && s.cursor == editor.line_buffer().insertion_point()
            });
        if !state.ready {
            state.rendered = false;
            state.authorized = false;
        }
    }

    fn needs_review(&self) -> bool {
        let state = self.gate.0.lock().unwrap();
        state.active && state.ready && state.snapshot.as_ref().is_some_and(|s| s.required)
    }
}

impl Menu for ReviewMenu {
    fn settings(&self) -> &MenuSettings {
        self.inner.settings()
    }
    fn is_active(&self) -> bool {
        self.inner.is_active()
    }
    fn set_active(&mut self, active: bool) {
        self.inner.set_active(active);
        self.gate.0.lock().unwrap().active = active;
        if !active {
            self.gate.clear();
        }
    }
    fn clear_input(&mut self) {
        self.inner.clear_input();
        self.gate.clear();
    }
    fn on_activate(&mut self) {
        self.gate.invalidate();
        self.inner.on_activate();
    }
    fn on_deactivate(&mut self) {
        self.gate.clear();
        self.inner.on_deactivate();
    }
    fn menu_event(&mut self, event: MenuEvent) {
        self.gate.invalidate();
        self.inner.menu_event(event);
        let active = self.inner.is_active();
        self.gate.0.lock().unwrap().active = active;
        if !active {
            self.gate.clear();
        }
    }
    // Even safe values must be previewed. Neither automatic route may bypass
    // publication checks or the explicit review gesture.
    fn can_quick_complete(&self) -> bool {
        false
    }
    fn can_partially_complete(&mut self, _: bool, _: &mut Editor, _: &mut dyn Completer) -> bool {
        false
    }
    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        self.gate.invalidate();
        self.inner.update_values(editor, completer);
        self.synchronize(editor);
    }
    fn reset_position(&mut self) {
        self.gate.invalidate();
        self.inner.reset_position();
    }
    fn reload(&mut self, updated: bool, editor: &mut Editor, completer: &mut dyn Completer) {
        self.gate.invalidate();
        self.inner.reload(updated, editor, completer);
        self.synchronize(editor);
    }
    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        self.inner
            .update_working_details(editor, completer, painter);
        self.synchronize(editor);
    }
    fn replace_in_buffer(&self, editor: &mut Editor) {
        let mut state = self.gate.0.lock().unwrap();
        let allowed = self.inner.is_active()
            && !self.inner.results_are_provisional()
            && state.active
            && state.ready
            && state.generation == self.generation
            && state.snapshot.as_ref().is_some_and(|s| {
                s.source == editor.get_buffer()
                    && s.cursor == editor.line_buffer().insertion_point()
                    && (!s.required || (state.rendered && state.authorized))
            });
        // Consume even a failed attempt; approval can never leak to a later edit.
        state.authorized = false;
        state.rendered = false;
        state.ready = false;
        if allowed {
            // Keep Reedline's span validation, cursor positioning and undo points.
            self.inner.replace_in_buffer(editor);
        }
    }
    fn menu_required_lines(&self, columns: u16) -> u16 {
        self.inner
            .menu_required_lines(columns)
            .saturating_add(u16::from(self.needs_review()))
    }
    fn menu_string(&self, available_lines: u16, colored: bool) -> String {
        let review = self.needs_review();
        let content = self
            .inner
            .menu_string(available_lines.saturating_sub(u16::from(review)), colored);
        let mut state = self.gate.0.lock().unwrap();
        let painted = available_lines > u16::from(review)
            && !content.is_empty()
            && state.active
            && state.ready
            && state.generation == self.generation;
        state.rendered = painted;
        if !painted {
            state.authorized = false;
        }
        if review && available_lines > 0 {
            let instruction = if state.authorized {
                "Review approved: Enter inserts, never executes."
            } else {
                "Review required: F2 then Enter inserts, never executes."
            };
            format!("{instruction}\r\n{content}")
        } else {
            content
        }
    }
    fn min_rows(&self) -> u16 {
        self.inner
            .min_rows()
            .saturating_add(u16::from(self.needs_review()))
    }
    fn get_values(&self) -> &[Suggestion] {
        self.inner.get_values()
    }
    fn results_are_provisional(&self) -> bool {
        self.inner.results_are_provisional()
    }
    fn is_awaiting_first_answer(&self) -> bool {
        self.inner.is_awaiting_first_answer()
    }
    fn set_cursor_pos(&mut self, pos: (u16, u16)) {
        self.inner.set_cursor_pos(pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reedline::{CompletionResult, InputMode, MenuBuilder, Span, UndoBehavior};

    struct Preview {
        gate: ReviewGate,
        suggestion: Suggestion,
        required: bool,
    }
    impl Completer for Preview {
        fn complete(&mut self, line: &str, cursor: usize) -> CompletionResult {
            self.gate.publish(line, cursor, self.required);
            CompletionResult::fresh(vec![self.suggestion.clone()])
        }
    }
    fn buffer(editor: &mut Editor, text: &str, cursor: usize) {
        editor.edit_buffer(
            |b| {
                b.set_buffer(text.into());
                b.set_insertion_point(cursor);
            },
            UndoBehavior::CreateUndoPoint,
        );
    }
    fn fixture(
        source: &str,
        cursor: usize,
        span: Span,
        replacement: &str,
        required: bool,
    ) -> (ReviewMenu, ReviewGate, Editor, Preview) {
        let gate = ReviewGate::default();
        let mut menu = ReviewMenu::new(
            ColumnarMenu::default()
                .with_columns(1)
                .with_input_mode(InputMode::FullBuffer),
            gate.clone(),
        );
        let mut editor = Editor::default();
        buffer(&mut editor, source, cursor);
        let mut preview = Preview {
            gate: gate.clone(),
            suggestion: Suggestion {
                value: replacement.into(),
                span,
                append_whitespace: false,
                ..Suggestion::default()
            },
            required,
        };
        menu.menu_event(MenuEvent::Activate(false));
        menu.update_values(&mut editor, &mut preview);
        (menu, gate, editor, preview)
    }
    #[test]
    fn batched_enter_cannot_be_retroactively_approved() {
        let (menu, gate, mut editor, _) = fixture("git ", 4, Span::new(4, 4), "status", true);
        menu.menu_string(10, false);
        gate.lock_acceptance(); // Parse Enter first.
        assert!(!gate.authorize()); // Parse F2 later in the same batch.
        menu.replace_in_buffer(&mut editor); // Now handle the earlier Enter.
        assert_eq!(editor.get_buffer(), "git ");
        gate.invalidate();
        assert!(gate.0.lock().unwrap().acceptance_locked);
    }

    #[test]
    fn approval_before_enter_remains_valid_and_next_publication_unlocks() {
        let (menu, gate, mut editor, _) = fixture("git ", 4, Span::new(4, 4), "status", true);
        menu.menu_string(10, false);
        assert!(gate.authorize()); // F2 precedes Enter.
        gate.lock_acceptance();
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "git status");
        gate.publish("git ", 4, true);
        assert!(!gate.0.lock().unwrap().acceptance_locked);
        assert!(!gate.authorize()); // New preview still needs to be painted.
        gate.lock_acceptance();
        gate.clear();
        assert!(!gate.0.lock().unwrap().acceptance_locked);
    }

    #[test]
    fn repaint_preserves_approval_but_pending_refresh_withdraws_it() {
        struct Pending;
        impl Completer for Pending {
            fn complete(&mut self, _: &str, _: usize) -> CompletionResult {
                CompletionResult::Pending
            }
        }
        let (mut menu, gate, mut editor, _) = fixture("git ", 4, Span::new(4, 4), "status", true);
        menu.menu_string(10, false);
        assert!(gate.authorize());
        assert!(menu.menu_string(10, false).contains("Review approved"));
        menu.update_values(&mut editor, &mut Pending);
        assert!(menu.results_are_provisional());
        menu.menu_string(10, false);
        assert!(!gate.authorize());
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "git ");
    }

    #[test]
    fn native_insertion_requires_painted_approval_and_preserves_suffix() {
        let (menu, gate, mut editor, _) =
            fixture("git  --short", 4, Span::new(4, 4), "status", true);
        assert!(!gate.authorize());
        assert!(menu.menu_string(10, false).contains("F2"));
        assert!(gate.authorize());
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "git status --short");
        assert_eq!(editor.line_buffer().insertion_point(), 10);
        assert!(!gate.authorize());
    }
    #[test]
    fn native_full_replacement_and_unapproved_enter() {
        let (mut menu, gate, mut editor, mut preview) = fixture(
            "deploy production",
            17,
            Span::new(0, 17),
            "deploy staging",
            true,
        );
        menu.menu_string(10, false);
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "deploy production");
        menu.menu_event(MenuEvent::Deactivate);
        assert!(!gate.authorize());
        menu.menu_event(MenuEvent::Activate(false));
        menu.update_values(&mut editor, &mut preview);
        menu.menu_string(10, false);
        assert!(gate.authorize());
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "deploy staging");
    }
    #[test]
    fn stale_source_cursor_cancel_and_republication_reject_approval() {
        for change in 0..5 {
            let (mut menu, gate, mut editor, _) =
                fixture("git  --short", 4, Span::new(4, 4), "status", true);
            menu.menu_string(10, false);
            assert!(gate.authorize());
            match change {
                0 => buffer(&mut editor, "git  --other", 4),
                1 => buffer(&mut editor, "git  --short", 5),
                2 => menu.menu_event(MenuEvent::Deactivate),
                3 => gate.publish("git  --short", 4, true),
                _ => gate.invalidate(),
            }
            let before = editor.get_buffer().to_string();
            menu.replace_in_buffer(&mut editor);
            assert_eq!(editor.get_buffer(), before);
        }
    }
    #[test]
    fn native_history_does_not_need_extra_approval_but_no_automatic_completion() {
        let (mut menu, _, mut editor, mut preview) =
            fixture("git  --short", 4, Span::new(4, 4), "status", false);
        assert!(!menu.can_quick_complete());
        assert!(!menu.can_partially_complete(true, &mut editor, &mut preview));
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "git status --short");
    }
    #[test]
    fn selection_changes_and_too_small_paints_do_not_authorize() {
        let (mut menu, gate, mut editor, mut preview) =
            fixture("git ", 4, Span::new(4, 4), "status", true);
        menu.menu_string(1, false);
        assert!(!gate.authorize());
        menu.menu_string(10, false);
        assert!(gate.authorize());
        menu.menu_event(MenuEvent::NextElement);
        assert!(!gate.authorize());
        menu.update_values(&mut editor, &mut preview);
        menu.menu_string(10, false);
        menu.replace_in_buffer(&mut editor);
        assert_eq!(editor.get_buffer(), "git ");
    }
}
