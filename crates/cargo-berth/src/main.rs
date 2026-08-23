//! `cargo-berth` — a git-worktree reservation engine.

mod cli;
mod config;
mod exit;
mod git;
mod ids;
mod ledger;
mod output;
mod reservation;
mod scope;
mod verb;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
