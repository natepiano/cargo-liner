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
mod pub_use_fixes;
mod render;
mod selection;

use std::io::IsTerminal;
use std::process::ExitCode;

use anyhow::Result;

#[derive(Clone, Copy)]
enum FixMode {
    None,
    Imports { dry_run: bool },
    PubUse { dry_run: bool },
}

enum PlannedRun {
    Report {
        report:          diagnostics::Report,
        post_run_notice: Option<String>,
    },
    ApplyImports {
        fixes: Vec<imports::UseFix>,
    },
    ApplyPubUse {
        scan: pub_use_fixes::PubUseFixScan,
    },
}

fn main() -> ExitCode {
    if std::env::var_os("MEND_DRIVER").is_some() {
        return compiler::driver_main();
    }

    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("mend: {err:#}");
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
    let fix_mode = fix_mode_from_cli(&cli.fix)?;
    let planned = plan_run(&selection, &config, fix_mode)?;
    let (report, post_run_notice) = execute_plan(&selection, &config, planned)?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!(
            "{}",
            render::render_human_report(&report, std::io::stdout().is_terminal())
        );
    }
    if let Some(notice) = post_run_notice {
        eprintln!("{notice}");
    }

    if report.has_errors() {
        return Ok(ExitCode::from(1));
    }

    if cli.fail_on_warn && report.has_warnings() {
        return Ok(ExitCode::from(2));
    }

    Ok(ExitCode::SUCCESS)
}

fn fix_mode_from_cli(fix_cli: &cli::FixCli) -> Result<FixMode> {
    match (fix_cli.fix, fix_cli.fix_pub_use, fix_cli.dry_run) {
        (false, false, false) => Ok(FixMode::None),
        (false, false, true) => anyhow::bail!("`--dry-run` requires `--fix` or `--fix-pub-use`"),
        (true, false, dry_run) => Ok(FixMode::Imports { dry_run }),
        (false, true, dry_run) => Ok(FixMode::PubUse { dry_run }),
        (true, true, _) => anyhow::bail!("`--fix` and `--fix-pub-use` cannot be combined"),
    }
}

fn plan_run(
    selection: &selection::Selection,
    config: &config::LoadedConfig,
    fix_mode: FixMode,
) -> Result<PlannedRun> {
    match fix_mode {
        FixMode::None => Ok(PlannedRun::Report {
            report:          build_report(selection, config, compiler::BuildOutputMode::Full)?,
            post_run_notice: None,
        }),
        FixMode::Imports { dry_run } => {
            let import_scan = imports::scan_selection(selection)?;
            if import_scan.fixes.is_empty() {
                Ok(PlannedRun::Report {
                    report:          build_report(
                        selection,
                        config,
                        compiler::BuildOutputMode::Full,
                    )?,
                    post_run_notice: Some("mend: no import fixes available".to_string()),
                })
            } else if dry_run {
                Ok(PlannedRun::Report {
                    report:          build_report(
                        selection,
                        config,
                        compiler::BuildOutputMode::Full,
                    )?,
                    post_run_notice: Some(format!(
                        "mend: would apply {} import fix(es) in dry run",
                        import_scan.fixes.len()
                    )),
                })
            } else {
                Ok(PlannedRun::ApplyImports {
                    fixes: import_scan.fixes,
                })
            }
        },
        FixMode::PubUse { dry_run } => {
            let initial_report = build_report(
                selection,
                config,
                compiler::BuildOutputMode::SuppressUnusedImportWarnings,
            )?;
            let scan = pub_use_fixes::scan_selection(selection, &initial_report)?;
            if scan.fixes.is_empty() {
                Ok(PlannedRun::Report {
                    report:          initial_report,
                    post_run_notice: Some("mend: no `pub use` fixes available".to_string()),
                })
            } else if dry_run {
                Ok(PlannedRun::Report {
                    report:          initial_report,
                    post_run_notice: Some(format!(
                        "mend: would apply {} `pub use` fix(es) in dry run",
                        scan.applied_count
                    )),
                })
            } else {
                Ok(PlannedRun::ApplyPubUse { scan })
            }
        },
    }
}

fn execute_plan(
    selection: &selection::Selection,
    config: &config::LoadedConfig,
    planned: PlannedRun,
) -> Result<(diagnostics::Report, Option<String>)> {
    match planned {
        PlannedRun::Report {
            report,
            post_run_notice,
        } => Ok((report, post_run_notice)),
        PlannedRun::ApplyImports { fixes } => {
            let snapshots = imports::snapshot_files(&fixes)?;
            let applied = imports::apply_fixes(&fixes)?;
            match build_report(selection, config, compiler::BuildOutputMode::Full) {
                Ok(report) => Ok((
                    report,
                    (applied > 0).then(|| format!("mend: applied {applied} import fix(es)")),
                )),
                Err(err) => {
                    imports::restore_files(&snapshots)?;
                    anyhow::bail!("rolled back import fixes after failed cargo check\n\n{err:#}");
                },
            }
        },
        PlannedRun::ApplyPubUse { scan } => {
            let snapshots = imports::snapshot_files(&scan.fixes)?;
            let _applied = imports::apply_fixes(&scan.fixes)?;
            match build_report(selection, config, compiler::BuildOutputMode::Full) {
                Ok(report) => Ok((
                    report,
                    (scan.applied_count > 0)
                        .then(|| format!("mend: applied {} `pub use` fix(es)", scan.applied_count)),
                )),
                Err(err) => {
                    imports::restore_files(&snapshots)?;
                    anyhow::bail!(
                        "rolled back `pub use` fixes after failed cargo check\n\n{err:#}"
                    );
                },
            }
        },
    }
}

fn build_report(
    selection: &selection::Selection,
    config: &config::LoadedConfig,
    output_mode: compiler::BuildOutputMode,
) -> Result<diagnostics::Report> {
    let mut report = compiler::run_selection(selection, config, output_mode)?;
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
    report.refresh_summary();
    Ok(report)
}
