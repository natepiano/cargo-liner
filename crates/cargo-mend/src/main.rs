#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

mod cli;
mod compiler;
mod config;
mod diagnostics;
mod fix_support;
mod imports;
mod outcome;
mod pub_use_fixes;
mod render;
mod run_mode;
mod runner;
mod selection;

use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Result;
use outcome::MendFailure;
use run_mode::OperationMode;
use runner::MendRunner;

fn main() -> ExitCode {
    if std::env::var_os("MEND_DRIVER").is_some() {
        return compiler::driver_main();
    }

    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("mend: {err}");
            MendFailure::exit_code()
        },
    }
}

fn run() -> Result<ExitCode, MendFailure> {
    let cli = cli::parse();
    let selection = selection::resolve_cargo_selection(cli.manifest_path.as_deref())
        .map_err(MendFailure::Unexpected)?;
    let config = config::load_config(
        selection.manifest_dir.as_path(),
        selection.workspace_root.as_path(),
        cli.config.as_deref(),
    )
    .map_err(MendFailure::Unexpected)?;
    let operation_mode = OperationMode::from_cli(&cli.fix).map_err(MendFailure::Unexpected)?;
    let outcome = MendRunner::new(&selection, &config).run(operation_mode)?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.report)
                .map_err(|err| MendFailure::Unexpected(err.into()))?
        );
    } else {
        print!(
            "{}",
            render::render_human_report(&outcome.report, std::io::stdout().is_terminal())
        );
    }
    if let Some(notice) = outcome.notice {
        eprintln!("{}", notice.render());
    }

    if outcome.report.has_errors() {
        return Ok(ExitCode::from(1));
    }

    if cli.fail_on_warn && outcome.report.has_warnings() {
        return Ok(ExitCode::from(2));
    }

    Ok(ExitCode::SUCCESS)
}
