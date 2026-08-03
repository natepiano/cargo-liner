//! `cargo-port` — a TUI for inspecting and managing Rust projects.

use std::process::ExitCode;

// Classification has no runtime caller yet, so the module is built for the test
// binary only — the whole graph stays exercised without claiming a caller that
// does not exist.
#[cfg(test)]
mod build_monitor;
mod cache_paths;
mod channel;
mod ci;
mod config;
mod constants;
mod enrichment;
mod http;
mod lint;
mod process_observation;
mod project;
mod scan;
mod sccache;
#[cfg(test)]
mod test_support;
mod themes;
mod tui;
mod watcher;

fn main() -> ExitCode { tui::run() }
