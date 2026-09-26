//! `cargo-tile` — a terminal UI cargo tool built on the `tui_pane`
//! framework.

// The tests compile from `tests/unit_tests.rs`. Explicit `--bins` or `--all-targets`
// still build this binary as a test harness despite `test = false`; empty, it neither
// reruns those tests nor checks their code a second time.
#![cfg(not(test))]

mod app;
mod birth_stamp;
mod capture;
mod census;
mod cli;
mod config;
mod constants;
mod globals;
mod hook;
mod interaction;
mod keymap;
mod probe;
mod progress;
mod registration;
mod render;
mod root_scan;
mod roster;
mod sccache;
mod settings;
mod terminal;
mod theme;
mod tiles;
mod wrap;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
