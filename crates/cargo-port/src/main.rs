//! `cargo-port` — a TUI for inspecting and managing Rust projects.

use std::process::ExitCode;

mod cache_paths;
mod channel;
mod ci;
mod config;
mod constants;
mod enrichment;
mod http;
mod lint;
mod project;
mod scan;
mod sccache;
#[cfg(test)]
mod test_support;
mod themes;
mod tui;
mod watcher;

fn main() -> ExitCode { tui::run() }
