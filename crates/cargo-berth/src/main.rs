//! `cargo-berth` — a git-worktree reservation engine.

macro_rules! declare_wire_enum {
    (
        $(#[$enum_metadata:meta])*
        $visibility:vis enum $name:ident {
            $(
                $(#[$variant_metadata:meta])*
                $variant:ident => $wire_name:literal;
            )+
        }
    ) => {
        $(#[$enum_metadata])*
        $visibility enum $name {
            $($(#[$variant_metadata])* $variant,)+
        }

    };
}

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
#[cfg(test)]
mod output_contract;
mod presentation;
mod reconcile;
mod recovery;
mod reservation;
mod scope;
mod session;
mod verb;
mod worktree;

use std::process::ExitCode;

fn main() -> ExitCode { cli::Cli::parse_arguments().run() }
