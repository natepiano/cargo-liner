use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;
use std::io::ErrorKind;
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
use super::progress::ProgressActivity;
use super::progress::ProgressDisplay;
use super::source_transaction::CompilerFixTransaction;
use super::stderr;
use super::stdout;
use super::stdout::AnalysisUnit;
use crate::compiler::ToolchainProbes;
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
    /// Retain the checked tree's warning counts for the next convergence pass.
    Validation,
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

/// Only an independent copy in this build's copy namespace can be reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapperAliasResolution {
    Current,
    NeedsReplacement,
}

/// Explicit feature flags preserve the user's selection during the fix pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeatureSelection {
    Explicit,
    Automatic,
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
    toolchain_probes: &ToolchainProbes,
) -> Result<SelectionResult, MendFailure> {
    let command_outcome = check_with_cache_recovery(
        selection,
        cargo_plan,
        loaded_config,
        output_mode,
        color_mode,
        toolchain_probes,
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
    toolchain_probes: &ToolchainProbes,
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
        toolchain_probes,
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
        toolchain_probes,
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
    toolchain_probes: &ToolchainProbes,
) -> Result<CommandOutcome> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new(CARGO_BIN);
    command.arg(CARGO_SUBCOMMAND_CHECK);
    command.args(&cargo_plan.cargo_args);
    command.arg(CARGO_FLAG_MESSAGE_FORMAT_JSON_RENDER_DIAGNOSTICS);

    let wrapper =
        wrapper_alias(cargo_plan.target_directory.as_path(), &current_exe).unwrap_or(current_exe);
    configure_rustc_wrapper(&mut command, &wrapper);
    toolchain_probes.configure(&mut command);

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
/// an independent executable inode is equally necessary: a symlink follows
/// later installs, and a hard link shares in-place writes with the installed
/// binary. Those writes invalidate running code signatures on macOS. A copy
/// keeps the driver attached to the build its directory names.
///
/// The copy namespace excludes aliases published by the former hard-link
/// protocol. The build fingerprint identifies the contents, so reuse needs
/// metadata checks, not a read of the whole binary on every mend invocation.
/// Returns `None` when the copy cannot be created.
fn wrapper_alias(target_directory: &Path, current_exe: &Path) -> Option<PathBuf> {
    let alias_dir = target_directory
        .join(WRAPPER_DIR_NAME)
        .join(settings::current_analysis_fingerprint())
        .join("independent-copy-v1");
    fs::create_dir_all(&alias_dir).ok()?;
    let alias = alias_dir.join(current_exe.file_name()?);
    if wrapper_alias_resolution(&alias) == WrapperAliasResolution::Current {
        return Some(alias);
    }
    replace_wrapper_alias(current_exe, &alias).ok()?;
    Some(alias)
}

/// The build-specific copy namespace supplies the build identity. Reject
/// symlinks and multiply linked files; neither owns its executable inode.
fn wrapper_alias_resolution(alias: &Path) -> WrapperAliasResolution {
    let independent = fs::symlink_metadata(alias).is_ok_and(|metadata| {
        #[cfg(unix)]
        let owns_inode = metadata.nlink() == 1;
        #[cfg(not(unix))]
        let owns_inode = true;
        metadata.file_type().is_file() && owns_inode
    });
    if independent {
        WrapperAliasResolution::Current
    } else {
        WrapperAliasResolution::NeedsReplacement
    }
}

/// An unpublished executable with its own inode. Only copying can construct
/// this state; publication cannot accidentally rename a hard-linked alias.
struct StagedWrapperCopy {
    path: PathBuf,
}

impl StagedWrapperCopy {
    fn create(current_exe: &Path, alias: &Path) -> io::Result<Self> {
        let path = alias.with_extension(format!(
            "{WRAPPER_ALIAS_STAGING_EXTENSION}-{}",
            process::id()
        ));
        match fs::remove_file(&path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        copy_wrapper_executable(current_exe, &path)
            .and_then(|()| {
                if wrapper_alias_resolution(&path) == WrapperAliasResolution::Current {
                    Ok(())
                } else {
                    Err(io::Error::other("wrapper copy does not own its inode"))
                }
            })
            .inspect_err(|_| drop(fs::remove_file(&path)))?;
        Ok(Self { path })
    }

    fn publish(self, alias: &Path) -> io::Result<()> { fs::rename(&self.path, alias) }
}

impl Drop for StagedWrapperCopy {
    fn drop(&mut self) { drop(fs::remove_file(&self.path)); }
}

/// Publish a complete copy without ever leaving the alias path absent.
/// Cargo can execute the old or new copy during the rename. The process id
/// prevents separate mend runs from sharing an unpublished staging path.
fn replace_wrapper_alias(current_exe: &Path, alias: &Path) -> io::Result<()> {
    StagedWrapperCopy::create(current_exe, alias)?.publish(alias)
}

#[cfg(target_os = "linux")]
fn copy_wrapper_executable(source: &Path, destination: &Path) -> io::Result<()> {
    let mut source = fs::File::open(source)?;
    let mut destination = fs::File::create_new(destination)?;
    // FICLONE shares extents, never the inode. Unsupported filesystems use an
    // ordinary byte copy of the same opened source, retaining execute bits.
    if rustix::fs::ioctl_ficlone(&destination, &source).is_err() {
        io::copy(&mut source, &mut destination)?;
    }
    destination.set_permissions(source.metadata()?.permissions())
}

#[cfg(not(target_os = "linux"))]
fn copy_wrapper_executable(source: &Path, destination: &Path) -> io::Result<()> {
    // On macOS std::fs::copy first uses fclonefileat (clonefile), with a
    // copyfile fallback off APFS. The staging path is absent to permit cloning.
    fs::copy(source, destination).map(drop)
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
    match (feature_selection(cargo_args), all_features_coverage) {
        (FeatureSelection::Automatic, AllFeaturesCoverage::Superset) => CargoFixFeatures::All,
        (
            FeatureSelection::Explicit,
            AllFeaturesCoverage::Superset | AllFeaturesCoverage::NotGuaranteed,
        )
        | (FeatureSelection::Automatic, AllFeaturesCoverage::NotGuaranteed) => {
            CargoFixFeatures::Plan
        },
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

fn feature_selection(cargo_args: &[OsString]) -> FeatureSelection {
    if cargo_args.iter().any(|arg| {
        arg == CARGO_FLAG_ALL_FEATURES
            || arg == CARGO_FLAG_FEATURES
            || arg == CARGO_FLAG_NO_DEFAULT_FEATURES
            || arg
                .to_str()
                .and_then(|arg| arg.strip_prefix(CARGO_FLAG_FEATURES))
                .is_some_and(|suffix| suffix.starts_with('='))
    }) {
        FeatureSelection::Explicit
    } else {
        FeatureSelection::Automatic
    }
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
    if progress.activity() == ProgressActivity::Active {
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
    #[cfg(unix)]
    use std::io::BufRead;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;

    use anyhow::Context;
    use tempfile::tempdir;

    use super::CargoFixFeatures;
    use super::FeatureSelection;
    use super::WrapperAliasResolution;
    use super::cargo_fix_features_for;
    use super::feature_selection;
    use super::wrapper_alias_resolution;
    use crate::compiler::constants::CARGO_FLAG_ALL_FEATURES;
    use crate::compiler::constants::CARGO_FLAG_FEATURES;
    use crate::compiler::constants::CARGO_FLAG_MANIFEST_PATH;
    use crate::compiler::constants::CARGO_FLAG_NO_DEFAULT_FEATURES;
    use crate::reporting::AllFeaturesCoverage;

    #[test]
    fn alias_resolution_reuses_only_regular_independent_copies() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("mend");
        let alias = directory.path().join("retained-mend");
        fs::write(&source, "current")?;
        assert_eq!(
            wrapper_alias_resolution(&alias),
            WrapperAliasResolution::NeedsReplacement
        );
        super::replace_wrapper_alias(&source, &alias)?;
        assert_eq!(
            wrapper_alias_resolution(&alias),
            WrapperAliasResolution::Current
        );
        // The source path can disappear; the build-specific copy remains valid.
        fs::remove_file(&source)?;
        assert_eq!(
            wrapper_alias_resolution(&alias),
            WrapperAliasResolution::Current
        );
        assert_eq!(
            wrapper_alias_resolution(directory.path()),
            WrapperAliasResolution::NeedsReplacement
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_alias_replaces_legacy_hard_links_and_reuses_the_copy() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let source = directory.path().join("cargo-mend");
        fs::write(&source, b"current build")?;
        let alias = super::wrapper_alias(directory.path(), &source).context("create alias")?;
        fs::remove_file(&alias)?;
        fs::hard_link(&source, &alias)?;
        assert_eq!(
            wrapper_alias_resolution(&alias),
            WrapperAliasResolution::NeedsReplacement
        );
        assert_eq!(
            super::wrapper_alias(directory.path(), &source),
            Some(alias.clone())
        );
        let inode = fs::metadata(&alias)?.ino();
        assert_ne!(inode, fs::metadata(&source)?.ino());
        assert_eq!(fs::metadata(&alias)?.nlink(), 1);
        assert_eq!(
            super::wrapper_alias(directory.path(), &source),
            Some(alias.clone())
        );
        assert_eq!(fs::metadata(&alias)?.ino(), inode);
        fs::write(&source, b"changed build")?;
        assert_eq!(fs::read(alias)?, b"current build");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_copy_ignores_a_legacy_slot_after_its_source_was_unlinked() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let source = directory.path().join("cargo-mend");
        let legacy_directory = directory
            .path()
            .join(crate::compiler::constants::WRAPPER_DIR_NAME)
            .join(crate::compiler::settings::current_analysis_fingerprint());
        fs::create_dir_all(&legacy_directory)?;
        let legacy = legacy_directory.join("cargo-mend");
        fs::write(&source, b"legacy overwritten contents")?;
        fs::hard_link(&source, &legacy)?;
        fs::remove_file(&source)?;
        fs::write(&source, b"current build")?;
        let alias = super::wrapper_alias(directory.path(), &source).context("create copy")?;
        assert_ne!(alias, legacy);
        assert_eq!(fs::read(alias)?, b"current build");
        assert_eq!(fs::read(legacy)?, b"legacy overwritten contents");
        Ok(())
    }

    #[test]
    fn failed_wrapper_copy_keeps_the_published_executable() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let source = directory.path().join("cargo-mend");
        fs::write(&source, b"retained build")?;
        let alias = super::wrapper_alias(directory.path(), &source).context("create copy")?;
        fs::remove_file(&source)?;
        assert!(super::replace_wrapper_alias(&source, &alias).is_err());
        assert_eq!(fs::read(alias)?, b"retained build");
        Ok(())
    }

    #[test]
    fn explicit_features_and_incomplete_coverage_keep_the_plan() {
        for coverage in [
            AllFeaturesCoverage::Superset,
            AllFeaturesCoverage::NotGuaranteed,
        ] {
            assert_eq!(
                cargo_fix_features_for(&[OsString::from(CARGO_FLAG_ALL_FEATURES)], coverage),
                CargoFixFeatures::Plan
            );
        }
        assert_eq!(
            cargo_fix_features_for(&[], AllFeaturesCoverage::Superset),
            CargoFixFeatures::All
        );
        assert_eq!(
            cargo_fix_features_for(&[], AllFeaturesCoverage::NotGuaranteed),
            CargoFixFeatures::Plan
        );
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_copy_survives_source_rewrite_before_exec() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let installed = directory.path().join("cargo-mend");
        fs::write(&installed, b"#!/bin/sh\nprintf 'retained build'\n")?;
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o755))?;
        let alias = super::wrapper_alias(directory.path(), &installed).context("create alias")?;
        // The child announces that it is about to exec this alias, then waits
        // for the source rewrite. No clock or scheduler timing decides the test.
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "printf 'ready\\n'; read -r go; exec \"$1\"",
                "wrapper",
            ])
            .arg(&alias)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().context("child stdout")?;
        let mut reader = std::io::BufReader::new(stdout);
        let mut ready = String::new();
        reader.read_line(&mut ready)?;
        assert_eq!(ready, "ready\n");
        fs::write(&installed, b"#!/bin/sh\nprintf 'rewritten build'\n")?;
        child
            .stdin
            .take()
            .context("child stdin")?
            .write_all(b"go\n")?;
        let mut output = String::new();
        std::io::Read::read_to_string(&mut reader, &mut output)?;
        assert!(child.wait()?.success());
        assert_eq!(output, "retained build");
        Ok(())
    }

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
        assert_eq!(fs::read(alias)?, b"first build");
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
            assert_eq!(feature_selection(&cargo_args), FeatureSelection::Explicit);
        }
    }

    #[test]
    fn ignores_cargo_arguments_without_feature_selection() {
        let cargo_args = vec![
            OsString::from(CARGO_FLAG_MANIFEST_PATH),
            OsString::from("Cargo.toml"),
        ];

        assert_eq!(feature_selection(&cargo_args), FeatureSelection::Automatic);
    }
}
