//! `cargo-port` — a TUI for inspecting and managing Rust projects.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

mod ci;
mod config;
mod constants;
mod list;
mod output;
mod project;
mod scan;
mod tui;
mod watcher;

use ci::CiArgs;
use list::ListArgs;

#[derive(Parser)]
#[command(name = "cargo-port", about = "Inspect Rust project structures")]
struct Cli {
    /// Path to the project or directory to operate on
    #[arg(default_value = ".")]
    path: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Recursively find and list all Rust projects under a path
    List(ListArgs),
    /// Show CI job durations for recent GitHub Actions runs
    Ci(CiArgs),
}

fn normalized_args() -> Vec<String> {
    let mut args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "port" {
        args.remove(1);
    }
    args
}

fn main() -> ExitCode {
    let cli = Cli::parse_from(normalized_args());
    match cli.command {
        Some(Commands::List(args)) => list::run(&cli.path, &args),
        Some(Commands::Ci(args)) => ci::run(&cli.path, &args),
        None => tui::run(&cli.path),
    }
}
