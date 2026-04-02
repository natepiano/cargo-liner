//! `cargo-port` — a TUI for inspecting and managing Rust projects.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

mod cache_paths;
mod ci;
mod config;
mod constants;
mod http;
mod lint_runtime;
mod perf_log;
mod port_report;
mod project;
mod scan;
mod tui;
mod watcher;

#[derive(Parser)]
#[command(name = "cargo-port", about = "Inspect Rust project structures")]
struct Cli {
    /// Path to the project or directory to operate on
    #[arg(default_value = ".")]
    path: PathBuf,
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
    tui::run(&cli.path)
}
