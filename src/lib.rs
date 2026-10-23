//! Native-first Reedline completion plus explicit, local structured workflow previews.
//!
//! Network use and history sharing are separate opt-ins. No shell commands
//! are executed and no shell history, cwd, or filesystem context is imported.

pub mod backend;
pub mod context;
pub mod editor;
pub mod engine;

pub mod history_context;
pub mod local_history;

pub mod commands;
pub mod project_memory;
pub mod shell;
pub mod structured_edit;
pub mod workflow;

pub mod review_menu;
pub mod risk;
