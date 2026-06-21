//! Detect which cargo bin/example/bench targets are currently running.
//!
//! Each tick refreshes the system process list (exe paths only) and walks
//! every process whose exe lives under a known workspace `target_directory`.
//! The path tail is parsed against cargo's filesystem layout to classify
//! the exe as a bin / example / bench of that workspace.

mod app_tick;
mod constants;
mod state;

pub(super) use state::*;
