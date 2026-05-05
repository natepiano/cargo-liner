#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

mod cargo_json;
mod cli;
mod compiler;
mod config;
mod constants;
mod diagnostics;
mod display_filter;
mod field_visibility_fix;
mod fix_support;
mod imports;
mod inline_path_qualified_type;
mod module_paths;
mod narrow_pub_crate;
mod outcome;
mod prefer_module_import;
mod pub_use_fixes;
mod render;
mod run_mode;
mod runner;
mod selection;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use cli::FixExecution;
use config::DiagnosticsConfig;
use constants::EXIT_CODE_ERROR;
use constants::EXIT_CODE_WARNING;
use display_filter::DisplayFilter;
use outcome::ExecutionOutcome;
use outcome::MendFailure;
use render::ColorMode;
use render::CompilerStats;
use render::OutputFormat;
use run_mode::OperationMode;
use runner::MendRunner;

/// Maximum number of mend passes during `--fix-all`. Prevents an infinite
/// loop if a fix oscillates; in practice convergence happens in 1–2 passes.
const FIX_ALL_MAX_PASSES: usize = 5;

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

fn build_diagnostics_help(diagnostics: &DiagnosticsConfig) -> String {
    let config_path = config::global_config_path()
        .map_or_else(|| "(unavailable)".to_string(), |p| p.display().to_string());

    let mut lines = vec![String::new(), "Diagnostics:".to_string()];
    for (code, enabled) in diagnostics.entries() {
        let name = code.as_str();
        let status = if enabled { "enabled" } else { "disabled" };
        lines.push(format!("  {name:<40} {status}"));
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
    let cli = cli::parse(&after_help);
    if cli.build_info {
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
    let output = if cli.json {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    };
    let start = Instant::now();
    let runner = MendRunner::new(&selection, &cargo_plan, &loaded_config, color, output);
    let mut outcome = runner.run(operation_mode.clone())?;
    let mut total_compiler_fix_duration = Duration::ZERO;
    let mut total_extra_check_duration = Duration::ZERO;

    let want_loop = matches!(cli.fix.execution, FixExecution::ApplyAll);
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
            && outcome.saw_unused_import_warnings;

        if user_asked_for_compiler_fix || pub_use_self_heal {
            total_compiler_fix_duration +=
                compiler::run_cargo_fix(&cargo_plan, color).map_err(MendFailure::Unexpected)?;
        }

        if !want_loop || passes >= FIX_ALL_MAX_PASSES {
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
    let mend_duration = total_duration.saturating_sub(check_duration);

    // Apply display filter — narrows reported findings according to the
    // user's `--lib`, `--bin`, `--example`, `--test`, `--bench` flags.
    // Analysis already ran with `--all-targets`; the filter is purely a
    // display narrower.
    let display_filter = DisplayFilter::from_cli(&cli.cargo, &selection.packages);
    display_filter.apply(&mut outcome.report);

    let compiler_stats = CompilerStats {
        warnings: outcome.compiler_warnings,
        fixable:  outcome.compiler_fixable,
    };

    match output {
        OutputFormat::Json => {
            print!(
                "{}",
                cargo_json::render_report(&outcome.report, &selection)
                    .map_err(MendFailure::Unexpected)?
            );
        },
        OutputFormat::Human => {
            print!(
                "{}",
                render::render_human_report(&outcome.report, &compiler_stats, color)
            );
        },
    }

    if output == OutputFormat::Human {
        eprintln!(
            "{}",
            render::render_timing(total_duration, check_duration, mend_duration, color)
        );
    }

    if let Some(notice) = outcome.notice {
        eprintln!("{}", notice.render());
    }

    if outcome.report.has_errors() {
        return Ok(ExitCode::from(EXIT_CODE_ERROR));
    }

    if cli.fail_on_warn && outcome.report.has_warnings() {
        return Ok(ExitCode::from(EXIT_CODE_WARNING));
    }

    Ok(ExitCode::SUCCESS)
}

fn color_mode() -> ColorMode {
    if let Ok(choice) = std::env::var("CLICOLOR_FORCE")
        && choice != "0"
    {
        return ColorMode::Enabled;
    }

    if let Ok(choice) = std::env::var("CARGO_TERM_COLOR") {
        let color_mode = match choice.to_ascii_lowercase().as_str() {
            "never" => Some(ColorMode::Disabled),
            "always" => Some(ColorMode::Enabled),
            _ => None,
        };
        if let Some(color_mode) = color_mode {
            return color_mode;
        }
    }

    if let Ok(choice) = std::env::var("CLICOLOR")
        && choice == "0"
    {
        return ColorMode::Disabled;
    }

    if std::io::stdout().is_terminal() || std::io::stderr().is_terminal() {
        return ColorMode::Enabled;
    }

    if std::env::var("TERM").is_ok_and(|term| term != "dumb") {
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
    use super::render::ColorMode;

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
        let _guard = EnvGuard::set("CARGO_TERM_COLOR", "never");
        assert!(matches!(color_mode(), ColorMode::Disabled));
    }

    #[test]
    fn cargo_term_color_always_enables_color() {
        let _guard = EnvGuard::set("CARGO_TERM_COLOR", "always");
        assert!(matches!(color_mode(), ColorMode::Enabled));
    }

    #[test]
    fn clicolor_zero_disables_color() {
        let _guard = EnvGuard::set("CLICOLOR", "0");
        let _cargo_term_color = EnvGuard::remove("CARGO_TERM_COLOR");
        assert!(matches!(color_mode(), ColorMode::Disabled));
    }

    #[test]
    fn term_enables_color_when_terminal_detection_is_unavailable() {
        let _cargo_term_color = EnvGuard::remove("CARGO_TERM_COLOR");
        let _clicolor = EnvGuard::remove("CLICOLOR");
        let _clicolor_force = EnvGuard::remove("CLICOLOR_FORCE");
        let _term = EnvGuard::set("TERM", "xterm-256color");
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
