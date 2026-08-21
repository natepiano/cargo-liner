//! `cargo-tile` — a terminal UI cargo tool built on the `tui_pane`
//! framework.

mod app;
mod config;
mod constants;
mod globals;
mod interaction;
mod iterm2;
mod keymap;
mod processes;
mod render;
mod roster;
mod settings;
mod terminal;
mod theme;
mod tiles;

use std::process::ExitCode;

fn main() -> ExitCode { terminal::run() }
