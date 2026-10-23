//! Asynchronous, insertion-only completion for Reedline.
//!
//! Network use and history sharing are separate opt-ins. No shell commands
//! are executed and no shell history, cwd, or filesystem context is imported.

pub mod backend;
pub mod context;
pub mod editor;
pub mod engine;

pub mod history_context;
pub mod local_history;
