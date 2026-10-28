use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::process;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use serde_json::to_string;

use super::cache;
use super::cache::AnalysisCache;
use super::progress::CargoProgress;
use super::progress::ProgressDisplay;
use super::source_transaction::CompilerFixTransaction;
use super::stderr;
use super::stdout;
use super::stdout::AnalysisUnit;
use crate::compiler::analyzing;
use crate::compiler::constants::ANALYZING_DIR_ENV;
use crate::compiler::constants::CARGO_BIN;
use crate::compiler::constants::CARGO_FLAG_ALL_FEATURES;
use crate::compiler::constants::CARGO_FLAG_ALL_TARGETS;
use crate::compiler::constants::CARGO_FLAG_ALLOW_DIRTY;
use crate::compiler::constants::CARGO_FLAG_ALLOW_STAGED;
use crate::compiler::constants::CARGO_FLAG_FEATURES;
use crate::compiler::constants::CARGO_FLAG_MESSAGE_FORMAT_JSON_RENDER_DIAGNOSTICS;
use crate::compiler::constants::CARGO_FLAG_NO_DEFAULT_FEATURES;
use crate::compiler::constants::CARGO_FLAG_TESTS;
use crate::compiler::constants::CARGO_SUBCOMMAND_CHECK;
use crate::compiler::constants::CARGO_SUBCOMMAND_FIX;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WHEN_ALWAYS;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WHEN_ENV;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WIDTH;
use crate::compiler::constants::CARGO_TERM_PROGRESS_WIDTH_ENV;
use crate::compiler::constants::CONFIG_FINGERPRINT_ENV;
use crate::compiler::constants::CONFIG_JSON_ENV;
use crate::compiler::constants::CONFIG_ROOT_ENV;
use crate::compiler::constants::DRIVER_ENV;
use crate::compiler::constants::DRIVER_ENV_ENABLED;
use crate::compiler::constants::PASSTHROUGH_RUSTC_WRAPPER_ENV;
use crate::compiler::constants::RUSTC_WORKSPACE_WRAPPER_ENV;
use crate::compiler::constants::SCOPE_FINGERPRINT_ENV;
use crate::compiler::constants::WRAPPER_ALIAS_STAGING_EXTENSION;
use crate::compiler::constants::WRAPPER_DIR_NAME;
use crate::compiler::persistence;
use crate::compiler::persistence::AnalysisEvidence;
use crate::compiler::settings;
use crate::config::LoadedConfig;
use crate::constants::FINGERPRINT_HEX_WIDTH;
use crate::reporting::AllFeaturesCoverage;
use crate::reporting::AnalysisFailure;
use crate::reporting::AppliedFixes;
use crate::reporting::CARGO_TERM_COLOR_ALWAYS;
use crate::reporting::CARGO_TERM_COLOR_ENV;
use crate::reporting::ColorMode;
use crate::reporting::CompilerFailureCause;
use crate::reporting::CompilerWarningFacts;
use crate::reporting::FixValidationFailure;
use crate::reporting::MendFailure;
use crate::reporting::Report;
use crate::reporting::RollbackStatus;
use crate::selection::CargoCheckPlan;
use crate::selection::Selection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildOutputMode {
    Full,
    Json,
    SuppressUnusedImportWarnings,
    Quiet,
}

#[derive(Debug)]
struct CommandOutcome {
    exit_status:            ExitStatus,
    compiler_warning_facts: CompilerWarningFacts,
    duration:               Duration,
    compiler_warnings:      usize,
    compiler_fixable:       usize,
    /// Metadata and ownership of every unit Cargo listed, fresh or rebuilt.
    units:                  Vec<AnalysisUnit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CargoFixFeatures {
    All,
    Plan,
}

pub(crate) struct SelectionResult {
    pub report:            Report,
    pub check_duration:    Duration,
    pub compiler_warnings: usize,
    pub compiler_fixable:  usize,
}

pub(crate) fn run_selection(
    selection: &Selection,
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
) -> Result<SelectionResult, MendFailure> {
    let command_outcome = check_with_cache_recovery(
        selection,
        cargo_plan,
        loaded_config,
        output_mode,
        color_mode,
    )?;
    let report_paths: Vec<_> = command_outcome
        .units
        .iter()
        .map(stdout::AnalysisUnit::report_path)
        .collect();
    let loaded = persistence::load_report(&report_paths, selection, &loaded_config.fingerprint);

    match loaded.analysis_evidence {
        AnalysisEvidence::Absent => {
            return Err(MendFailure::Analysis(AnalysisFailure {
                cause: CompilerFailureCause::NoAnalysisProduced,
            }));
        },
        AnalysisEvidence::Present => {},
    }

    let mut report = loaded.report;
    report.facts.compiler_warning_facts = command_outcome.compiler_warning_facts;
    Ok(SelectionResult {
        report,
        check_duration: command_outcome.duration,
        compiler_warnings: command_outcome.compiler_warnings,
        compiler_fixable: command_outcome.compiler_fixable,
    })
}

fn check_with_cache_recovery(
    selection: &Selection,
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
) -> Result<CommandOutcome, MendFailure> {
    let analyzing_dir = analyzing::analyzing_dir(cargo_plan.target_directory.as_path());
    let scope_fingerprint = scope_fingerprint_for(cargo_plan);

    let mut command_outcome = run_cargo_check(
        cargo_plan,
        loaded_config,
        &analyzing_dir,
        &scope_fingerprint,
        output_mode,
        color_mode,
    )
    .map_err(driver_setup_failure)?;

    if !command_outcome.exit_status.success() {
        return Err(MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck,
        }));
    }

    let paths = match AnalysisCache::inspect(
        &command_outcome.units,
        selection,
        &loaded_config.fingerprint,
    ) {
        AnalysisCache::Compatible => return Ok(command_outcome),
        AnalysisCache::RebuildRequired(paths) => paths,
        AnalysisCache::Unrepairable => {
            return Err(MendFailure::Analysis(AnalysisFailure {
                cause: CompilerFailureCause::NoAnalysisProduced,
            }));
        },
    };
    cache::invalidate_metadata(&paths).map_err(driver_setup_failure)?;
    let previous_duration = command_outcome.duration;
    command_outcome = run_cargo_check(
        cargo_plan,
        loaded_config,
        &analyzing_dir,
        &scope_fingerprint,
        output_mode,
        color_mode,
    )
    .map_err(driver_setup_failure)?;
    command_outcome.duration += previous_duration;
    if !command_outcome.exit_status.success() {
        return Err(MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck,
        }));
    }
    match AnalysisCache::inspect(
        &command_outcome.units,
        selection,
        &loaded_config.fingerprint,
    ) {
        AnalysisCache::Compatible => {},
        AnalysisCache::RebuildRequired(_) | AnalysisCache::Unrepairable => {
            return Err(MendFailure::Analysis(AnalysisFailure {
                cause: CompilerFailureCause::NoAnalysisProduced,
            }));
        },
    }
    Ok(command_outcome)
}

const fn driver_setup_failure(error: Error) -> MendFailure {
    MendFailure::Analysis(AnalysisFailure {
        cause: CompilerFailureCause::DriverSetup(error),
    })
}

/// Runs `cargo check` with `RUSTC_WORKSPACE_WRAPPER` pointing to the mend binary.
///
/// The wrapper uses nightly's `rustc_driver::run_compiler` to analyze workspace members,
/// while dependencies are compiled by the project's default toolchain (typically stable).
/// This relies on nightly's `rustc_driver` being able to read `.rmeta` files produced by
/// stable — which works across close toolchain versions but is not guaranteed by rustc.
///
/// If a future rustc update breaks `.rmeta` compatibility between the mend binary's
/// toolchain and the project's default, this function would need to force the mend
/// binary's toolchain for the entire `cargo check` (via `RUSTUP_TOOLCHAIN`) and isolate
/// artifacts in a separate target directory (via `CARGO_TARGET_DIR`) to avoid corrupting
/// the project's build cache. See git history for the prior toolchain-selection design.
fn run_cargo_check(
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    analyzing_dir: &Path,
    scope_fingerprint: &str,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
) -> Result<CommandOutcome> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new(CARGO_BIN);
    command.arg(CARGO_SUBCOMMAND_CHECK);
    command.args(&cargo_plan.cargo_args);
    command.arg(CARGO_FLAG_MESSAGE_FORMAT_JSON_RENDER_DIAGNOSTICS);

    let wrapper =
        wrapper_alias(cargo_plan.target_directory.as_path(), &current_exe).unwrap_or(current_exe);
    configure_rustc_wrapper(&mut command, &wrapper);

    command
        .env(DRIVER_ENV, DRIVER_ENV_ENABLED)
        .env(CONFIG_ROOT_ENV, &loaded_config.root)
        .env(
            CONFIG_JSON_ENV,
            to_string(&loaded_config.visibility_config)
                .context("failed to serialize mend config for compiler driver")?,
        )
        .env(CONFIG_FINGERPRINT_ENV, &loaded_config.fingerprint)
        .env(ANALYZING_DIR_ENV, analyzing_dir)
        .env(SCOPE_FINGERPRINT_ENV, scope_fingerprint)
        .stdin(Stdio::inherit());

    run_cargo_command(&mut command, output_mode, color_mode, analyzing_dir)
        .context("failed to run cargo check for mend")
}

/// Names a per-build alias of the mend binary for cargo to invoke as the
/// wrapper, in place of the binary's own path.
///
/// The build-specific path changes Cargo's unit ids after an install. Retaining
/// the executable itself is equally necessary: a symlink to the installed path
/// follows later installs, letting a parent from build A execute build B's
/// driver and reject every report it just produced. A hard link (or copy across
/// filesystems) keeps this alias attached to the build its directory names.
///
/// Returns `None` when the alias cannot be created; the caller then passes the
/// binary's own path and behaves as before.
fn wrapper_alias(target_directory: &Path, current_exe: &Path) -> Option<PathBuf> {
    let alias_dir = target_directory
        .join(WRAPPER_DIR_NAME)
        .join(settings::current_analysis_fingerprint());
    fs::create_dir_all(&alias_dir).ok()?;
    let alias = alias_dir.join(current_exe.file_name()?);
    if alias_matches_executable(&alias, current_exe) {
        return Some(alias);
    }
    replace_wrapper_alias(current_exe, &alias).ok()?;
    Some(alias)
}

#[cfg(unix)]
fn alias_matches_executable(alias: &Path, current_exe: &Path) -> bool {
    let Ok(alias_metadata) = fs::symlink_metadata(alias) else {
        return false;
    };
    let Ok(executable_metadata) = fs::metadata(current_exe) else {
        return false;
    };
    alias_metadata.file_type().is_file()
        && alias_metadata.dev() == executable_metadata.dev()
        && alias_metadata.ino() == executable_metadata.ino()
}

#[cfg(not(unix))]
fn alias_matches_executable(alias: &Path, current_exe: &Path) -> bool {
    fs::symlink_metadata(alias).is_ok_and(|metadata| metadata.file_type().is_file())
        && fs::read(alias)
            .and_then(|alias_bytes| fs::read(current_exe).map(|bytes| bytes == alias_bytes))
            .unwrap_or(false)
}

/// Points `alias` at `current_exe` without ever leaving the alias path absent.
///
/// Cargo bakes this path into a unit fingerprint and executes it once per
/// compilation unit, so unlinking before relinking opens a window in which those
/// executions fail with `ENOENT` and the build aborts before any code is
/// checked. Renaming a staged link over the alias closes the window: the path
/// resolves to the old binary or the new one and is never missing. The staging
/// name carries the process id so two mend runs on one target directory cannot
/// stage onto each other.
fn replace_wrapper_alias(current_exe: &Path, alias: &Path) -> io::Result<()> {
    let staged = alias.with_extension(format!(
        "{WRAPPER_ALIAS_STAGING_EXTENSION}-{}",
        process::id()
    ));
    drop(fs::remove_file(&staged));
    link_wrapper_alias(current_exe, &staged)?;
    fs::rename(&staged, alias).inspect_err(|_| drop(fs::remove_file(&staged)))
}

fn link_wrapper_alias(current_exe: &Path, alias: &Path) -> io::Result<()> {
    fs::hard_link(current_exe, alias).or_else(|_| fs::copy(current_exe, alias).map(drop))
}

/// Installs mend as the workspace wrapper, leaving any ambient `RUSTC_WRAPPER`
/// in place.
///
/// The workspace slot is the one cargo records in a unit's fingerprint, so it is
/// the only slot in which [`wrapper_alias`] can do its job. Taking the plain
/// `RUSTC_WRAPPER` slot instead leaves the alias unfingerprinted, and a freshly
/// installed mend then finds every unit up to date and produces no analysis.
///
/// It is also the narrower slot: it wraps workspace members and not their
/// dependencies, which are the only crates mend analyzes. Cargo invokes the two
/// wrappers outermost-first, so a user's `RUSTC_WRAPPER` keeps caching
/// dependency builds and mend runs beneath it for workspace members.
fn configure_rustc_wrapper(command: &mut Command, current_exe: &Path) {
    command
        .env(RUSTC_WORKSPACE_WRAPPER_ENV, current_exe)
        .env_remove(PASSTHROUGH_RUSTC_WRAPPER_ENV);
}

fn scope_fingerprint_for(cargo_plan: &CargoCheckPlan) -> String {
    let mut hasher = DefaultHasher::new();
    cargo_plan.manifest_path.hash(&mut hasher);
    for arg in &cargo_plan.cargo_args {
        arg.hash(&mut hasher);
    }
    format!(
        "{:0width$x}",
        hasher.finish(),
        width = FINGERPRINT_HEX_WIDTH
    )
}

pub(crate) fn run_cargo_fix(
    selection: &Selection,
    cargo_plan: &CargoCheckPlan,
    color_mode: ColorMode,
    all_features_coverage: AllFeaturesCoverage,
) -> Result<Duration, MendFailure> {
    let start = Instant::now();
    let transaction =
        CompilerFixTransaction::capture(selection).map_err(MendFailure::Unexpected)?;
    let cargo_fix_features = cargo_fix_features_for(&cargo_plan.cargo_args, all_features_coverage);
    let applied_features = match apply_cargo_fix(cargo_plan, color_mode, cargo_fix_features) {
        Ok(applied_features) => applied_features,
        Err(error) => {
            return Err(compiler_fix_failure(
                &transaction,
                CompilerFailureCause::Unexpected(error),
            ));
        },
    };
    let validation = run_cargo_fix_validation(cargo_plan, color_mode, applied_features);
    match validation {
        Ok(status) if status.success() => Ok(start.elapsed()),
        Ok(_) => Err(compiler_fix_failure(
            &transaction,
            CompilerFailureCause::CargoCheck,
        )),
        Err(error) => Err(compiler_fix_failure(
            &transaction,
            CompilerFailureCause::Unexpected(error),
        )),
    }
}

fn apply_cargo_fix(
    cargo_plan: &CargoCheckPlan,
    color_mode: ColorMode,
    cargo_fix_features: CargoFixFeatures,
) -> Result<CargoFixFeatures> {
    let status = run_cargo_fix_command(cargo_plan, color_mode, cargo_fix_features)?;
    if status.success() {
        return Ok(cargo_fix_features);
    }

    if cargo_fix_features == CargoFixFeatures::All {
        eprintln!(
            "cargo fix with --all-features exited with {status}; retrying without --all-features"
        );
        let retry_status = run_cargo_fix_command(cargo_plan, color_mode, CargoFixFeatures::Plan)?;
        if retry_status.success() {
            return Ok(CargoFixFeatures::Plan);
        }
    }

    bail!("cargo fix failed");
}

fn run_cargo_fix_validation(
    cargo_plan: &CargoCheckPlan,
    color_mode: ColorMode,
    cargo_fix_features: CargoFixFeatures,
) -> Result<ExitStatus> {
    let mut command = Command::new(CARGO_BIN);
    command
        .arg(CARGO_SUBCOMMAND_CHECK)
        .args(&cargo_plan.cargo_args);
    if cargo_fix_features == CargoFixFeatures::All {
        command.arg(CARGO_FLAG_ALL_FEATURES);
    }
    if color_mode.is_enabled() {
        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
    }
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to validate cargo fix")
}

fn compiler_fix_failure(
    transaction: &CompilerFixTransaction,
    cause: CompilerFailureCause,
) -> MendFailure {
    let rollback_status = transaction
        .restore()
        .map_or(RollbackStatus::RestoreFailed, |()| RollbackStatus::Restored);
    MendFailure::FixValidation(FixValidationFailure {
        rollback_status,
        cause,
        applied_fixes: AppliedFixes::Compiler,
    })
}

fn cargo_fix_features_for(
    cargo_args: &[OsString],
    all_features_coverage: AllFeaturesCoverage,
) -> CargoFixFeatures {
    match (
        has_explicit_feature_selection(cargo_args),
        all_features_coverage,
    ) {
        (false, AllFeaturesCoverage::Superset) => CargoFixFeatures::All,
        (true, _) | (false, AllFeaturesCoverage::NotGuaranteed) => CargoFixFeatures::Plan,
    }
}

fn run_cargo_fix_command(
    cargo_plan: &CargoCheckPlan,
    color_mode: ColorMode,
    cargo_fix_features: CargoFixFeatures,
) -> Result<ExitStatus> {
    let mut command = Command::new(CARGO_BIN);
    command
        .arg(CARGO_SUBCOMMAND_FIX)
        .arg(CARGO_FLAG_ALLOW_DIRTY)
        .arg(CARGO_FLAG_ALLOW_STAGED);

    // Replace `--all-targets` with `--tests` for the fix pass. Rationale:
    // `cargo fix --all-targets` runs the lib (non-test) compilation
    // alongside others; that compilation strips `#[cfg(test)]` blocks and
    // emits `unused_imports` warnings for items reached only from test
    // code. Cargo fix then deletes those imports, breaking the test build
    // (E0425 cascade — the original bug). Running with `--tests` only
    // compiles each target in test mode, where every cfg(test)-protected
    // call site is live, so genuinely-needed imports are never removed.
    // The same loss happens when an import is reached only from a
    // feature-gated module: default-feature compilation reports it unused
    // and cargo fix removes it. Add `--all-features` unless the plan already
    // selects features with `--features`, `--all-features`, or
    // `--no-default-features`; an explicit user selection must remain intact.
    // `SourceCache` also withholds `--all-features` when a `cfg` or `cfg_attr`
    // predicate negates a feature. Enabling that feature can disable its
    // module, so the plan's feature configuration preserves more source.
    // Platform gates such as `#[cfg(windows)] mod win;` remain invisible on
    // macOS because no feature flag can make them active.
    // Dependency feature unification is another residual limit. `--all-features`
    // can enable a dependency feature whose `not(feature = "...")` gate removes
    // an item imported by this crate. `SourceCache` scans only this crate's
    // sources, so it cannot see that dependency gate.
    // Trade-off: imports that are unused in BOTH lib and test mode (rare)
    // won't be pruned by `--fix-compiler`; users can clean those manually.
    for arg in &cargo_plan.cargo_args {
        if arg == CARGO_FLAG_ALL_TARGETS {
            command.arg(CARGO_FLAG_TESTS);
        } else {
            command.arg(arg);
        }
    }
    if cargo_fix_features == CargoFixFeatures::All {
        command.arg(CARGO_FLAG_ALL_FEATURES);
    }

    if color_mode.is_enabled() {
        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
    }

    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to run cargo fix")
}

fn has_explicit_feature_selection(cargo_args: &[OsString]) -> bool {
    cargo_args.iter().any(|arg| {
        arg == CARGO_FLAG_ALL_FEATURES
            || arg == CARGO_FLAG_FEATURES
            || arg == CARGO_FLAG_NO_DEFAULT_FEATURES
            || arg
                .to_str()
                .and_then(|arg| arg.strip_prefix(CARGO_FLAG_FEATURES))
                .is_some_and(|suffix| suffix.starts_with('='))
    })
}

fn run_cargo_command(
    command: &mut Command,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
    analyzing_dir: &Path,
) -> Result<CommandOutcome> {
    if color_mode.is_enabled() {
        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
    }
    let progress = CargoProgress::start(output_mode, analyzing_dir);
    // The status line takes its counter from cargo's bar, which cargo draws on
    // a pipe only when asked to and given a width to draw it at.
    if progress.is_active() {
        command
            .env(
                CARGO_TERM_PROGRESS_WHEN_ENV,
                CARGO_TERM_PROGRESS_WHEN_ALWAYS,
            )
            .env(CARGO_TERM_PROGRESS_WIDTH_ENV, CARGO_TERM_PROGRESS_WIDTH);
    }
    command.stdin(Stdio::inherit());
    command.stderr(Stdio::piped());
    command.stdout(Stdio::piped());
    let start = Instant::now();
    let mut child = command.spawn().context("failed to spawn cargo command")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture cargo stderr")?;
    let stdout = child
        .stdout
        .take()
        .context("failed to capture cargo stdout")?;
    // Cargo writes both pipes as the build runs; reading stdout on its own
    // thread keeps a full stdout pipe from stalling cargo while this thread
    // is still streaming stderr.
    let stdout_reader = thread::spawn(move || stdout::collect_units(stdout, output_mode));
    let stderr_outcome = stderr::stream_cargo_stderr(stderr, output_mode, progress)?;
    let units = stdout_reader
        .join()
        .map_err(|_| anyhow!("cargo stdout reader panicked"))?
        .context("failed to read cargo stdout")?;
    let exit_status = child.wait().context("failed to wait for cargo command")?;
    let duration = start.elapsed();
    Ok(CommandOutcome {
        exit_status,
        compiler_warning_facts: stderr_outcome.compiler_warning_facts,
        duration,
        compiler_warnings: stderr_outcome.warning_count,
        compiler_fixable: stderr_outcome.fixable_count,
        units,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;

    use anyhow::Context;
    use tempfile::tempdir;

    use super::has_explicit_feature_selection;
    use crate::compiler::constants::CARGO_FLAG_ALL_FEATURES;
    use crate::compiler::constants::CARGO_FLAG_FEATURES;
    use crate::compiler::constants::CARGO_FLAG_MANIFEST_PATH;
    use crate::compiler::constants::CARGO_FLAG_NO_DEFAULT_FEATURES;

    #[test]
    fn wrapper_alias_retains_its_build_when_the_installed_binary_is_replaced() -> anyhow::Result<()>
    {
        let temp = tempdir()?;
        let installed = temp.path().join("cargo-mend");
        fs::write(&installed, b"first build")?;
        let alias = super::wrapper_alias(temp.path(), &installed).context("create alias")?;
        let replacement = temp.path().join("replacement");
        fs::write(&replacement, b"next build")?;
        fs::rename(replacement, &installed)?;
        assert_eq!(fs::read(&alias)?, b"first build");
        assert_eq!(
            super::wrapper_alias(temp.path(), &installed),
            Some(alias.clone())
        );
        assert_eq!(fs::read(alias)?, b"next build");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_alias_replaces_a_legacy_symlink_with_the_retained_build() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let installed = temp.path().join("cargo-mend");
        fs::write(&installed, b"current build")?;
        let alias = super::wrapper_alias(temp.path(), &installed).context("create alias")?;
        fs::remove_file(&alias)?;
        std::os::unix::fs::symlink(&installed, &alias)?;
        assert_eq!(
            super::wrapper_alias(temp.path(), &installed),
            Some(alias.clone())
        );
        assert!(!fs::symlink_metadata(alias)?.file_type().is_symlink());
        Ok(())
    }

    #[test]
    fn detects_explicit_cargo_feature_selections() {
        let selections = [
            vec![
                OsString::from(CARGO_FLAG_FEATURES),
                OsString::from("hidden"),
            ],
            vec![OsString::from(format!("{CARGO_FLAG_FEATURES}=hidden"))],
            vec![OsString::from(CARGO_FLAG_ALL_FEATURES)],
            vec![OsString::from(CARGO_FLAG_NO_DEFAULT_FEATURES)],
        ];
        for cargo_args in selections {
            assert!(has_explicit_feature_selection(&cargo_args));
        }
    }

    #[test]
    fn ignores_cargo_arguments_without_feature_selection() {
        let cargo_args = vec![
            OsString::from(CARGO_FLAG_MANIFEST_PATH),
            OsString::from("Cargo.toml"),
        ];

        assert!(!has_explicit_feature_selection(&cargo_args));
    }
}
