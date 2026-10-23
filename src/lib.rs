//! Asynchronous, insertion-only completion for Reedline.
//!
//! Network use is opt-in. No shell commands, history, working directory, or
//! filesystem contents are collected or executed by this crate.

pub mod backend;
pub mod context;
pub mod editor;
pub mod engine;
