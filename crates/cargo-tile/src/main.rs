//! `cargo-tile` — a terminal UI cargo tool built on the `tui_pane`
//! framework.

mod app;
mod config;
mod constants;
mod globals;
mod keymap;
mod processes;
mod render;
mod settings;
mod terminal;
mod theme;

use std::process::ExitCode;

fn main() -> ExitCode { terminal::run() }
