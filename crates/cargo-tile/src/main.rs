//! `cargo-tile` — a terminal UI cargo tool built on the `tui_pane`
//! framework.

mod app;
mod attract;
mod cli;
mod config;
mod constants;
#[expect(
    dead_code,
    reason = "the favorites file API is written before the key handlers that call it"
)]
mod favorites;
mod globals;
mod hook;
mod interaction;
mod iterm2;
mod keymap;
mod navigation;
mod probe;
mod processes;
mod progress;
mod render;
mod roster;
mod sccache;
mod settings;
mod terminal;
mod theme;
mod tiles;
mod wrap;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
