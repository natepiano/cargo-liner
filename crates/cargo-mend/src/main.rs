#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

mod compiler;
mod config;
mod fixes;
mod reporting;
mod runner;
mod rust_syntax;
mod selection;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use compiler::DRIVER_ENV;
use config::BuildInfoMode;
use config::DiagnosticsConfig;
use config::FixExecution;
use config::OperationMode;
use config::WarningPolicy;
use reporting::BuildOutcome;
use reporting::CARGO_TERM_COLOR_ALWAYS;
use reporting::CARGO_TERM_COLOR_ENV;
use reporting::CARGO_TERM_COLOR_NEVER;
use reporting::CLICOLOR_DISABLED_VALUE;
use reporting::CLICOLOR_ENV;
use reporting::CLICOLOR_FORCE_ENV;
use reporting::ColorMode;
use reporting::CompilerStats;
use reporting::CompilerWarningFacts;
use reporting::DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH;
use reporting::EXIT_CODE_ERROR;
use reporting::EXIT_CODE_WARNING;
use reporting::ExecutionOutcome;
use reporting::MendFailure;
use reporting::OutputFormat;
use reporting::TERM_DUMB_VALUE;
use reporting::TERM_ENV;
use runner::FIX_ALL_MAX_PASSES;
use runner::MendRunner;
use selection::DisplayFilter;
use selection::Selection;

fn main() -> ExitCode {
    if std::env::var_os(DRIVER_ENV).is_some() {
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

fn build_diagnostics_help(diagnostics: &DiagnosticsConfig) -> String {
    let config_path = config::global_config_path()
        .map_or_else(|| "(unavailable)".to_string(), |p| p.display().to_string());

    let mut lines = vec![String::new(), "Diagnostics:".to_string()];
    for (code, enabled) in diagnostics.entries() {
        let name = code.as_str();
        let status = enabled.label();
        lines.push(format!(
            "  {name:<DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH$} {status}"
        ));
    }
    lines.push(String::new());
    lines.push(format!("Config: {config_path}"));
    lines.join("\n")
}

fn build_info_text() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let git_hash = option_env!("MEND_GIT_HASH").unwrap_or("unknown");
    let build_id = option_env!("MEND_BUILD_ID").unwrap_or("unknown");
    let build_sysroot = option_env!("MEND_BUILD_SYSROOT").unwrap_or("unknown");
    format!(
        "cargo-mend {version}\n\
         git_hash: {git_hash}\n\
         build_id: {build_id}\n\
         build_sysroot: {build_sysroot}"
    )
}

/// Sum of all fix-eligible items across the three categories.
const fn total_fixables(outcome: &ExecutionOutcome) -> usize {
    outcome.report.summary.fixable_with_fix
        + outcome.report.summary.fixable_with_fix_pub_use
        + outcome.compiler_fixable
}

fn run() -> Result<ExitCode, MendFailure> {
    let global_diagnostics = config::load_global_diagnostics();
    let after_help = build_diagnostics_help(&global_diagnostics);
    let cli = config::parse(&after_help);
    if cli.build_info == BuildInfoMode::Show {
        println!("{}", build_info_text());
        return Ok(ExitCode::SUCCESS);
    }
    let selection = selection::resolve_cargo_selection(cli.cargo.explicit_manifest_path())
        .map_err(MendFailure::Unexpected)?;
    let cargo_plan = selection::build_cargo_check_plan(&selection, &cli.cargo);
    let loaded_config = config::load_config(
        selection.manifest_dir.as_path(),
        selection.workspace_root.as_path(),
        cli.manifest.config.as_deref(),
        &global_diagnostics,
    )
    .map_err(MendFailure::Unexpected)?;
    let operation_mode = OperationMode::from(&cli.fix);
    let color = color_mode();
    let output_format = cli.output_format;
    let start = Instant::now();
    let runner = MendRunner::new(
        &selection,
        &cargo_plan,
        &loaded_config,
        color,
        output_format,
    );
    let mut outcome = runner.run(operation_mode.clone())?;
    let mut total_compiler_fix_duration = Duration::ZERO;
    let mut total_extra_check_duration = Duration::ZERO;

    let mut passes = 1;

    loop {
        // Decide whether to chain `cargo fix`. Two trigger conditions:
        //  1. The user explicitly asked for it (`--fix-compiler` / `--fix-all`).
        //  2. `--fix-pub-use` (or its bundle inside `--fix-all`) just applied edits that produced
        //     `unused import` warnings; auto-clean them.
        let user_asked_for_compiler_fix = cli.fix.runs_compiler_fix();
        let pub_use_self_heal = matches!(
            cli.fix.execution,
            FixExecution::ApplyRequested | FixExecution::ApplyAll
        ) && outcome.applied_pub_use > 0
            && matches!(
                outcome.compiler_warning_facts,
                CompilerWarningFacts::UnusedImportWarnings
            );

        if user_asked_for_compiler_fix || pub_use_self_heal {
            total_compiler_fix_duration +=
                compiler::run_cargo_fix(&cargo_plan, color).map_err(MendFailure::Unexpected)?;
        }

        if !matches!(cli.fix.execution, FixExecution::ApplyAll) || passes >= FIX_ALL_MAX_PASSES {
            break;
        }

        // For `--fix-all` convergence: re-scan and decide if another pass
        // would strictly reduce remaining fixables.
        let next = runner.run(operation_mode.clone())?;
        let next_total = total_fixables(&next);
        let prev_total = total_fixables(&outcome);
        total_extra_check_duration += next.check_duration;
        if next_total == 0 || next_total >= prev_total {
            outcome = next;
            break;
        }
        outcome = next;
        passes += 1;
    }

    let total_duration = start.elapsed();
    let check_duration =
        outcome.check_duration + total_extra_check_duration + total_compiler_fix_duration;

    // Apply display filter — narrows reported findings according to the
    // user's `--lib`, `--bin`, `--example`, `--test`, `--bench` flags.
    // Analysis already ran with `--all-targets`; the filter is purely a
    // display narrower.
    let display_filter = DisplayFilter::from_cli(&cli.cargo, &selection.packages);
    display_filter.apply(&mut outcome.report);

    render_outcome(
        &outcome,
        &selection,
        output_format,
        color,
        total_duration,
        check_duration,
    )?;

    if outcome.report.outcome() == BuildOutcome::Failed {
        return Ok(ExitCode::from(EXIT_CODE_ERROR));
    }
    if cli.warning_policy == WarningPolicy::Fail && outcome.report.has_warnings() {
        return Ok(ExitCode::from(EXIT_CODE_WARNING));
    }
    Ok(ExitCode::SUCCESS)
}

fn render_outcome(
    outcome: &ExecutionOutcome,
    selection: &Selection,
    output_format: OutputFormat,
    color: ColorMode,
    total_duration: Duration,
    check_duration: Duration,
) -> Result<(), MendFailure> {
    let compiler_stats = CompilerStats {
        warnings: outcome.compiler_warnings,
        fixable:  outcome.compiler_fixable,
    };

    match output_format {
        OutputFormat::Json => {
            print!(
                "{}",
                reporting::render_report(&outcome.report, selection)
                    .map_err(MendFailure::Unexpected)?
            );
        },
        OutputFormat::Human => {
            print!(
                "{}",
                reporting::render_human_report(&outcome.report, &compiler_stats, color)
            );
        },
    }

    if output_format == OutputFormat::Human {
        let mend_duration = total_duration.saturating_sub(check_duration);
        eprintln!(
            "{}",
            reporting::render_timing(total_duration, check_duration, mend_duration, color)
        );
    }

    if let Some(notice) = outcome.notice.as_ref() {
        eprintln!("{}", notice.render());
    }

    Ok(())
}

fn color_mode() -> ColorMode {
    if let Ok(choice) = std::env::var(CLICOLOR_FORCE_ENV)
        && choice != CLICOLOR_DISABLED_VALUE
    {
        return ColorMode::Enabled;
    }

    if let Ok(choice) = std::env::var(CARGO_TERM_COLOR_ENV) {
        let color_mode = match choice.to_ascii_lowercase().as_str() {
            CARGO_TERM_COLOR_NEVER => Some(ColorMode::Disabled),
            CARGO_TERM_COLOR_ALWAYS => Some(ColorMode::Enabled),
            _ => None,
        };
        if let Some(color_mode) = color_mode {
            return color_mode;
        }
    }

    if let Ok(choice) = std::env::var(CLICOLOR_ENV)
        && choice == CLICOLOR_DISABLED_VALUE
    {
        return ColorMode::Disabled;
    }

    if std::io::stdout().is_terminal() || std::io::stderr().is_terminal() {
        return ColorMode::Enabled;
    }

    if std::env::var(TERM_ENV).is_ok_and(|term| term != TERM_DUMB_VALUE) {
        ColorMode::Enabled
    } else {
        ColorMode::Disabled
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::build_info_text;
    use super::color_mode;
    use crate::reporting::CARGO_TERM_COLOR_ALWAYS;
    use crate::reporting::CARGO_TERM_COLOR_ENV;
    use crate::reporting::CARGO_TERM_COLOR_NEVER;
    use crate::reporting::CLICOLOR_DISABLED_VALUE;
    use crate::reporting::CLICOLOR_ENV;
    use crate::reporting::CLICOLOR_FORCE_ENV;
    use crate::reporting::ColorMode;
    use crate::reporting::TERM_ENV;

    struct EnvGuard {
        key:      &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe { std::env::set_var(key, value) };
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            unsafe { std::env::remove_var(key) };
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                unsafe { std::env::set_var(self.key, previous) };
            } else {
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    #[test]
    fn cargo_term_color_never_disables_color() {
        let _guard = EnvGuard::set(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_NEVER);
        assert!(matches!(color_mode(), ColorMode::Disabled));
    }

    #[test]
    fn cargo_term_color_always_enables_color() {
        let _guard = EnvGuard::set(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
        assert!(matches!(color_mode(), ColorMode::Enabled));
    }

    #[test]
    fn clicolor_zero_disables_color() {
        let _guard = EnvGuard::set(CLICOLOR_ENV, CLICOLOR_DISABLED_VALUE);
        let _cargo_term_color = EnvGuard::remove(CARGO_TERM_COLOR_ENV);
        assert!(matches!(color_mode(), ColorMode::Disabled));
    }

    #[test]
    fn term_enables_color_when_terminal_detection_is_unavailable() {
        let _cargo_term_color = EnvGuard::remove(CARGO_TERM_COLOR_ENV);
        let _clicolor = EnvGuard::remove(CLICOLOR_ENV);
        let _clicolor_force = EnvGuard::remove(CLICOLOR_FORCE_ENV);
        let _term = EnvGuard::set(TERM_ENV, "xterm-256color");
        assert!(matches!(color_mode(), ColorMode::Enabled));
    }

    #[test]
    fn build_info_contains_expected_fields() {
        let build_info = build_info_text();

        assert!(build_info.starts_with(&format!("cargo-mend {}", env!("CARGO_PKG_VERSION"))));
        assert!(build_info.contains("\ngit_hash: "));
        assert!(build_info.contains("\nbuild_id: "));
        assert!(build_info.contains("\nbuild_sysroot: "));
    }
}
