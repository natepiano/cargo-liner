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
mod imports;
mod render;
mod selection;

use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Result;

fn main() -> ExitCode {
    if std::env::var_os("VISCHECK_DRIVER").is_some() {
        return compiler::driver_main();
    }

    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("vischeck: {err:#}");
            ExitCode::from(2)
        },
    }
}

fn run() -> Result<ExitCode> {
    let cli = cli::parse();
    let selection = selection::resolve_cargo_selection(cli.manifest_path.as_deref())?;
    let config = config::load_config(
        selection.manifest_dir.as_path(),
        selection.workspace_root.as_path(),
        cli.config.as_deref(),
    )?;
    let report = if cli.fix {
        let import_scan = imports::scan_selection(&selection)?;
        let snapshots = imports::snapshot_files(&import_scan.fixes)?;
        let applied = imports::apply_fixes(&import_scan.fixes)?;
        if applied > 0 {
            eprintln!("vischeck: applied {applied} import fix(es)");
        }
        match build_report(&selection, &config) {
            Ok(report) => report,
            Err(err) => {
                imports::restore_files(&snapshots)?;
                anyhow::bail!("rolled back import fixes after failed cargo check\n\n{err:#}");
            },
        }
    } else {
        build_report(&selection, &config)?
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!(
            "{}",
            render::render_human_report(&report, std::io::stdout().is_terminal())
        );
    }

    if report.has_errors() {
        return Ok(ExitCode::from(1));
    }

    if cli.fail_on_warn && report.has_warnings() {
        return Ok(ExitCode::from(2));
    }

    Ok(ExitCode::SUCCESS)
}

fn build_report(
    selection: &selection::Selection,
    config: &config::LoadedConfig,
) -> Result<diagnostics::Report> {
    let mut report = compiler::run_selection(selection, config)?;
    let import_scan = imports::scan_selection(selection)?;
    report.findings.extend(import_scan.findings);
    report.findings.sort_by(|a, b| {
        (
            a.severity,
            &a.path,
            a.line,
            a.column,
            &a.code,
            &a.item,
            &a.message,
            &a.suggestion,
        )
            .cmp(&(
                b.severity,
                &b.path,
                b.line,
                b.column,
                &b.code,
                &b.item,
                &b.message,
                &b.suggestion,
            ))
    });
    report.findings.dedup_by(|a, b| {
        a.severity == b.severity
            && a.code == b.code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
            && a.suggestion == b.suggestion
    });
    Ok(report)
}
