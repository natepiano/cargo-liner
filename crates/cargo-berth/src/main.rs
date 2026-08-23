//! `cargo-berth` — a git-worktree reservation engine.

mod cli;
mod exit;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "This module is the published identifier surface; it defines every identifier the ledger format names, including those no code reads yet."
    )
)]
mod ids;
mod output;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
