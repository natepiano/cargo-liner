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
mod module_paths;
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
        let color = color_output_enabled();
        print!("{}", render::render_human_report(&outcome.report, color));
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

fn color_output_enabled() -> bool {
    if let Ok(choice) = std::env::var("CLICOLOR_FORCE")
        && choice != "0"
    {
        return true;
    }

    if let Ok(choice) = std::env::var("CARGO_TERM_COLOR") {
        match choice.to_ascii_lowercase().as_str() {
            "never" => return false,
            "always" => return true,
            _ => {},
        }
    }

    if let Ok(choice) = std::env::var("CLICOLOR")
        && choice == "0"
    {
        return false;
    }

    if std::io::stdout().is_terminal() || std::io::stderr().is_terminal() {
        return true;
    }

    std::env::var("TERM").is_ok_and(|term| term != "dumb")
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::color_output_enabled;

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
        assert!(!color_output_enabled());
    }

    #[test]
    fn cargo_term_color_always_enables_color() {
        let _guard = EnvGuard::set("CARGO_TERM_COLOR", "always");
        assert!(color_output_enabled());
    }

    #[test]
    fn clicolor_zero_disables_color() {
        let _guard = EnvGuard::set("CLICOLOR", "0");
        let _cargo_term_color = EnvGuard::remove("CARGO_TERM_COLOR");
        assert!(!color_output_enabled());
    }

    #[test]
    fn term_enables_color_when_terminal_detection_is_unavailable() {
        let _cargo_term_color = EnvGuard::remove("CARGO_TERM_COLOR");
        let _clicolor = EnvGuard::remove("CLICOLOR");
        let _clicolor_force = EnvGuard::remove("CLICOLOR_FORCE");
        let _term = EnvGuard::set("TERM", "xterm-256color");
        assert!(color_output_enabled());
    }
}
