//! The `cargo-mend` binary with clippy's lints, registered in mend's compiler
//! driver the way `clippy-driver` registers them, so one `cargo mend` compile
//! of each workspace member runs both mend's analysis and clippy.

#![feature(rustc_private)]

extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_span;

use std::env;
use std::path::Path;
use std::process::ExitCode;

use cargo_mend::ClippyLints;
use clippy_utils::sym;
use declare_clippy_lint::LintListBuilder;
use rustc_interface::interface::Config;
use rustc_session::Session;
use rustc_span::Symbol;

/// `cargo clippy` joins its trailing arguments with this separator.
const CLIPPY_ARGS_SEPARATOR: &str = "__CLIPPY_HACKERY__";
const CLIPPY_ARGS_ENV: &str = "CLIPPY_ARGS";
const CLIPPY_CFG: [&str; 2] = ["--cfg", "clippy"];
const CLIPPY_CONF_DIR_ENV: &str = "CLIPPY_CONF_DIR";
const CARGO_MANIFEST_FILE: &str = "Cargo.toml";
/// A `cargo clippy` flag, not a rustc one; mend lints only the packages the
/// run selects, which is what it asks for.
const NO_DEPS_FLAG: &str = "--no-deps";
/// Clippy's MIR lints read unoptimized MIR, and these two passes insert checks
/// that its lints would report.
const DISABLED_MIR_PASSES: [&str; 2] = ["CheckNull", "CheckAlignment"];

fn main() -> ExitCode {
    cargo_mend::main(Some(ClippyLints {
        extend_rustc_args,
        configure,
    }))
}

fn extend_rustc_args(rustc_args: &mut Vec<String>) {
    let clippy_args = env::var(CLIPPY_ARGS_ENV).unwrap_or_default();
    rustc_args.extend(
        clippy_args
            .split(CLIPPY_ARGS_SEPARATOR)
            .filter(|arg| !arg.is_empty() && *arg != NO_DEPS_FLAG)
            .map(str::to_string),
    );
    rustc_args.extend(CLIPPY_CFG.map(str::to_string));
}

fn configure(config: &mut Config) {
    let previous = config.register_lints.take();
    let clippy_args = env::var(CLIPPY_ARGS_ENV).ok();
    config.track_state = Some(Box::new(move |session| {
        track_clippy_inputs(session, clippy_args.as_deref());
    }));
    config.register_lints = Some(Box::new(move |session, lint_store| {
        if let Some(previous) = &previous {
            previous(session, lint_store);
        }
        let mut lint_list_builder = LintListBuilder::default();
        lint_list_builder.insert(clippy_lints::declared_lints::LINTS);
        lint_list_builder.register(lint_store);
        let conf = clippy_config::Conf::load(session);
        clippy_lints::register_lint_passes(lint_store, conf);
    }));
    config.extra_symbols = sym::EXTRA_SYMBOLS.into();
    config.opts.unstable_opts.mir_opt_level = Some(0);
    config.opts.unstable_opts.mir_enable_passes = DISABLED_MIR_PASSES
        .iter()
        .map(|pass| ((*pass).to_string(), false))
        .collect();
    // Keeps `format_args!` unflattened, so the HIR matches the AST clippy's
    // format lints compare it with.
    config.opts.unstable_opts.flatten_format_args = false;
}

/// Records `CLIPPY_ARGS`, `CLIPPY_CONF_DIR` and the member's `Cargo.toml` in
/// the dep-info file, so cargo reruns the member when one of them changes.
fn track_clippy_inputs(session: &Session, clippy_args: Option<&str>) {
    let mut env_depinfo = session.env_depinfo.borrow_mut();
    env_depinfo.insert((sym::CLIPPY_ARGS, clippy_args.map(Symbol::intern)));
    env_depinfo.insert((
        sym::CLIPPY_CONF_DIR,
        env::var(CLIPPY_CONF_DIR_ENV)
            .ok()
            .map(|dir| Symbol::intern(&dir)),
    ));
    if Path::new(CARGO_MANIFEST_FILE).exists() {
        session.file_depinfo.borrow_mut().insert(sym::Cargo_toml);
    }
}
