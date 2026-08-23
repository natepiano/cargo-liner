//! `cargo-tile` — a terminal UI cargo tool built on the `tui_pane`
//! framework.

mod app;
mod cli;
mod config;
mod constants;
mod globals;
mod hook;
mod interaction;
mod iterm2;
mod keymap;
mod processes;
mod progress;
mod render;
mod roster;
mod settings;
mod terminal;
mod theme;
mod tiles;
mod wrap;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
