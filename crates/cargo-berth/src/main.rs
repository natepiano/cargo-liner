//! `cargo-berth` — a git-worktree reservation engine.

mod alert;
mod cli;
mod config;
mod exit;
mod git;
mod ids;
mod ledger;
mod output;
mod reconcile;
mod recovery;
mod reservation;
mod scope;
mod verb;
mod worktree;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
