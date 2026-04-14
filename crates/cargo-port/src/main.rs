//! `cargo-port` — a TUI for inspecting and managing Rust projects.

use std::process::ExitCode;

mod cache_paths;
mod ci;
mod config;
mod constants;
mod http;
mod keymap;
mod lint;
mod perf_log;
mod project;
mod project_list;
mod scan;
mod tui;
mod watcher;

fn main() -> ExitCode { tui::run() }
