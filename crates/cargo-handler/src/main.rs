//! `cargo-handler` — a terminal UI cargo tool built on the `tui_pane`
//! framework.

mod app;
mod cli;
mod config;
mod constants;
mod globals;
mod interaction;
mod keymap;
mod render;
mod settings;
mod terminal;
mod theme;
mod tiles;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
