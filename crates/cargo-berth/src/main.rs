//! `cargo-berth` — a git-worktree reservation engine.

mod alert;
mod answer;
mod board;
mod cli;
mod config;
mod constants;
mod coordination_identity;
mod drift;
mod edge;
mod exit;
mod gate;
mod git;
mod ids;
mod ledger;
mod output;
mod reconcile;
mod recovery;
mod reservation;
mod scope;
mod session;
mod verb;
mod worktree;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
