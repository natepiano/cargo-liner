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
use std::time::Instant;

use anyhow::Result;
use config::DiagnosticsConfig;
use constants::EXIT_CODE_ERROR;
use constants::EXIT_CODE_WARNING;
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

fn run() -> Result<ExitCode, MendFailure> {
    let global_diagnostics = config::load_global_diagnostics();
    let after_help = build_diagnostics_help(&global_diagnostics);
    let cli = cli::parse(&after_help);
    let selection = selection::resolve_cargo_selection(cli.cargo.explicit_manifest_path())
        .map_err(MendFailure::Unexpected)?;
    let cargo_plan = selection::build_cargo_check_plan(&selection, &cli.cargo);
    let config = config::load_config(
        selection.manifest_dir.as_path(),
        selection.workspace_root.as_path(),
        cli.manifest.config.as_deref(),
        &global_diagnostics,
    )
    .map_err(MendFailure::Unexpected)?;
    let operation_mode = OperationMode::from_cli(&cli.fix);
    let color = color_mode();
    let start = Instant::now();
    let outcome =
        MendRunner::new(&selection, &cargo_plan, &config, color, cli.json).run(operation_mode)?;

    let fix_compiler_duration = if cli.fix.fix_compiler() || cli.fix.fix_all() {
        Some(compiler::run_cargo_fix(&cargo_plan, color).map_err(MendFailure::Unexpected)?)
    } else {
        None
    };

    let total_duration = start.elapsed();
    let check_duration = outcome.check_duration + fix_compiler_duration.unwrap_or_default();
    let mend_duration = total_duration.saturating_sub(check_duration);

    let compiler_stats = render::CompilerStats {
        warning_count: outcome.compiler_warning_count,
        fixable_count: outcome.compiler_fixable_count,
    };

    if cli.json {
        print!(
            "{}",
            cargo_json::render_report(&outcome.report, &selection)
                .map_err(MendFailure::Unexpected)?
        );
    } else {
        print!(
            "{}",
            render::render_human_report(&outcome.report, &compiler_stats, color)
        );
    }

    if !cli.json {
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

fn color_mode() -> render::ColorMode {
    if let Ok(choice) = std::env::var("CLICOLOR_FORCE")
        && choice != "0"
    {
        return render::ColorMode::Enabled;
    }

    if let Ok(choice) = std::env::var("CARGO_TERM_COLOR") {
        match choice.to_ascii_lowercase().as_str() {
            "never" => return render::ColorMode::Disabled,
            "always" => return render::ColorMode::Enabled,
            _ => {},
        }
    }

    if let Ok(choice) = std::env::var("CLICOLOR")
        && choice == "0"
    {
        return render::ColorMode::Disabled;
    }

    if std::io::stdout().is_terminal() || std::io::stderr().is_terminal() {
        return render::ColorMode::Enabled;
    }

    if std::env::var("TERM").is_ok_and(|term| term != "dumb") {
        render::ColorMode::Enabled
    } else {
        render::ColorMode::Disabled
    }
}

#[cfg(test)]
#[allow(
    clippy::used_underscore_binding,
    reason = "RAII guards use _ prefix but are held for Drop"
)]
mod tests {
    use std::ffi::OsString;

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
}
