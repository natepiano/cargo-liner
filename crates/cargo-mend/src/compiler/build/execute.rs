use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
#[cfg(not(target_os = "linux"))]
use std::env;
use std::env::consts::EXE_SUFFIX;
use std::ffi::OsString;
use std::fs;
use std::fs::File;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;
use std::io::ErrorKind;
use std::io::Seek;
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
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use anyhow::ensure;
use serde_json::to_string;

use super::cache;
use super::cache::AnalysisCache;
use super::driver_compiler::DriverCompiler;
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
    error_files:            BTreeSet<PathBuf>,
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

/// The open executable inode retained before the run resolves Cargo metadata.
pub(crate) struct RunningExecutable {
    file: File,
}

impl RunningExecutable {
    pub(crate) fn open() -> Result<Self> {
        #[cfg(target_os = "linux")]
        let source = PathBuf::from("/proc/self/exe");
        #[cfg(not(target_os = "linux"))]
        let source = env::current_exe().context("failed to locate the running mend executable")?;
        Self::open_path(&source)
    }

    fn open_path(source: &Path) -> Result<Self> {
        let file = fs::File::open(source).with_context(|| {
            format!(
                "failed to open the running mend executable {}",
                source.display()
            )
        })?;
        Ok(Self { file })
    }
}

/// The independent executable copy shared by every driver command in one run.
pub(crate) struct RunWrapper {
    path: PathBuf,
}

impl RunWrapper {
    pub(crate) fn resolve(
        target_directory: &Path,
        executable: &mut RunningExecutable,
    ) -> Result<Self> {
        let path = wrapper_alias(target_directory, executable).with_context(|| {
            format!(
                "failed to retain the open mend executable in {}",
                target_directory.display()
            )
        })?;
        Ok(Self { path })
    }

    fn configure(&self, command: &mut Command) -> Result<()> {
        ensure!(
            self.path.is_file(),
            "the retained mend wrapper is missing: {}",
            self.path.display()
        );
        let compiler = DriverCompiler::for_build()?;
        #[cfg(target_os = "linux")]
        compiler.configure_runtime_libraries(command)?;
        compiler.configure(command, &self.path);
        Ok(())
    }
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
    wrapper: &RunWrapper,
) -> Result<SelectionResult, MendFailure> {
    let command_outcome = check_with_cache_recovery(
        selection,
        cargo_plan,
        loaded_config,
        output_mode,
        color_mode,
        toolchain_probes,
        wrapper,
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
    wrapper: &RunWrapper,
) -> Result<CommandOutcome, MendFailure> {
    let analyzing_dir = analyzing::analyzing_dir(cargo_plan.target_directory.as_path());

    let mut command_outcome = run_cargo_check(
        cargo_plan,
        loaded_config,
        &analyzing_dir,
        output_mode,
        color_mode,
        toolchain_probes,
        wrapper,
    )
    .map_err(driver_setup_failure)?;

    if !command_outcome.exit_status.success() {
        return Err(MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck {
                error_files: command_outcome.error_files,
            },
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
        output_mode,
        color_mode,
        toolchain_probes,
        wrapper,
    )
    .map_err(driver_setup_failure)?;
    command_outcome.duration += previous_duration;
    if !command_outcome.exit_status.success() {
        return Err(MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck {
                error_files: command_outcome.error_files,
            },
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

/// Run analysis and fix validation with mend as `RUSTC_WORKSPACE_WRAPPER`.
///
/// `DriverCompiler` sets the workspace wrapper and selects rustc from the build sysroot together.
/// This prevents metadata mismatches when the project selects another compiler release.
/// Cargo retains artifacts for each compiler in the same target directory under separate hashes.
/// The run opens its executable before Cargo metadata resolution.
/// It retains one wrapper copy across analysis, cache recovery, and fix validation.
fn run_cargo_check(
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    analyzing_dir: &Path,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
    toolchain_probes: &ToolchainProbes,
    wrapper: &RunWrapper,
) -> Result<CommandOutcome> {
    let mut command = Command::new(CARGO_BIN);
    command.arg(CARGO_SUBCOMMAND_CHECK);
    command.args(&cargo_plan.cargo_args);
    command.arg(CARGO_FLAG_MESSAGE_FORMAT_JSON_RENDER_DIAGNOSTICS);

    wrapper.configure(&mut command)?;
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
        .env(SCOPE_FINGERPRINT_ENV, scope_fingerprint_for(cargo_plan))
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
/// protocol. Executable length and modification time distinguish development
/// rebuilds that retain the analysis fingerprint. Reuse needs only metadata
/// checks, not a read of the whole binary on every mend invocation.
/// Report the cause when the executable copy cannot be created.
fn wrapper_alias(target_directory: &Path, executable: &mut RunningExecutable) -> Result<PathBuf> {
    let revision = WrapperExecutableRevision::read(&executable.file)
        .context("failed to inspect the open mend executable")?;
    let alias_dir = target_directory
        .join(WRAPPER_DIR_NAME)
        .join(settings::current_analysis_fingerprint())
        .join("independent-copy-v1")
        .join(revision.directory_name());
    fs::create_dir_all(&alias_dir)
        .with_context(|| format!("failed to create wrapper directory {}", alias_dir.display()))?;
    let alias = alias_dir.join(format!("{}{EXE_SUFFIX}", env!("CARGO_PKG_NAME")));
    if wrapper_alias_resolution(&alias) == WrapperAliasResolution::Current {
        return Ok(alias);
    }
    replace_wrapper_alias(executable, &alias)
        .with_context(|| format!("failed to copy wrapper to {}", alias.display()))?;
    Ok(alias)
}

/// Identifies executable rebuilds from the open inode without hashing its bytes.
struct WrapperExecutableRevision {
    length:   u64,
    modified: SystemTime,
}

impl WrapperExecutableRevision {
    fn read(executable: &File) -> io::Result<Self> {
        let metadata = executable.metadata()?;
        Ok(Self {
            length:   metadata.len(),
            modified: metadata.modified()?,
        })
    }

    fn directory_name(&self) -> String {
        let (era, timestamp) = match self.modified.duration_since(UNIX_EPOCH) {
            Ok(timestamp) => ("after", timestamp),
            Err(error) => ("before", error.duration()),
        };
        format!("{}-{era}-{}", self.length, timestamp.as_nanos())
    }
}

/// The fingerprint and executable revision supply the build identity. Reject
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
    fn create(executable: &mut RunningExecutable, alias: &Path) -> io::Result<Self> {
        let path = alias.with_extension(format!(
            "{WRAPPER_ALIAS_STAGING_EXTENSION}-{}",
            process::id()
        ));
        match fs::remove_file(&path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        copy_wrapper_executable(&mut executable.file, &path)
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
fn replace_wrapper_alias(executable: &mut RunningExecutable, alias: &Path) -> io::Result<()> {
    StagedWrapperCopy::create(executable, alias)?.publish(alias)
}

fn copy_wrapper_executable(source: &mut File, destination: &Path) -> io::Result<()> {
    let mut destination = fs::File::create_new(destination)?;
    // A clone shares extents, never the inode. Other filesystems copy bytes.
    #[cfg(target_os = "linux")]
    if rustix::fs::ioctl_ficlone(&destination, &*source).is_ok() {
        return destination.set_permissions(source.metadata()?.permissions());
    }
    source.rewind()?;
    io::copy(source, &mut destination)?;
    destination.set_permissions(source.metadata()?.permissions())
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
            CompilerFailureCause::CargoCheck {
                error_files: BTreeSet::new(),
            },
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
    configure_cargo_color(&mut command, color_mode);
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

    configure_cargo_color(&mut command, color_mode);

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

/// `configure_cargo_color` forces Cargo color only for `ColorMode::Enabled`.
/// `ColorMode::Disabled` preserves the inherited environment and existing command overrides.
fn configure_cargo_color(command: &mut Command, color_mode: ColorMode) {
    match color_mode {
        ColorMode::Enabled => {
            command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
        },
        ColorMode::Disabled => {},
    }
}

fn run_cargo_command(
    command: &mut Command,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
    analyzing_dir: &Path,
) -> Result<CommandOutcome> {
    configure_cargo_color(command, color_mode);
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
        error_files: stderr_outcome.error_files,
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
    use std::ffi::OsStr;
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
    use std::path::PathBuf;
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;

    use anyhow::Context;
    use tempfile::tempdir;

    use super::CargoFixFeatures;
    use super::FeatureSelection;
    use super::WrapperAliasResolution;
    use super::cargo_fix_features_for;
    use super::configure_cargo_color;
    use super::feature_selection;
    use super::wrapper_alias_resolution;
    use crate::compiler::constants::CARGO_BIN;
    use crate::compiler::constants::CARGO_FLAG_ALL_FEATURES;
    use crate::compiler::constants::CARGO_FLAG_FEATURES;
    use crate::compiler::constants::CARGO_FLAG_MANIFEST_PATH;
    use crate::compiler::constants::CARGO_FLAG_NO_DEFAULT_FEATURES;
    use crate::reporting::AllFeaturesCoverage;
    use crate::reporting::CARGO_TERM_COLOR_ALWAYS;
    use crate::reporting::CARGO_TERM_COLOR_ENV;
    use crate::reporting::CARGO_TERM_COLOR_NEVER;
    use crate::reporting::ColorMode;

    #[test]
    fn enabled_cargo_color_overrides_the_command_environment() {
        let mut command = Command::new(CARGO_BIN);
        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_NEVER);

        configure_cargo_color(&mut command, ColorMode::Enabled);

        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            [(
                OsStr::new(CARGO_TERM_COLOR_ENV),
                Some(OsStr::new(CARGO_TERM_COLOR_ALWAYS)),
            )]
        );
    }

    #[test]
    fn disabled_cargo_color_preserves_inheritance_and_command_overrides() {
        let mut command = Command::new(CARGO_BIN);

        configure_cargo_color(&mut command, ColorMode::Disabled);

        assert_eq!(command.get_envs().next(), None);

        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_NEVER);
        configure_cargo_color(&mut command, ColorMode::Disabled);
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            [(
                OsStr::new(CARGO_TERM_COLOR_ENV),
                Some(OsStr::new(CARGO_TERM_COLOR_NEVER)),
            )]
        );

        command.env_remove(CARGO_TERM_COLOR_ENV);
        configure_cargo_color(&mut command, ColorMode::Disabled);
        assert_eq!(
            command.get_envs().collect::<Vec<_>>(),
            [(OsStr::new(CARGO_TERM_COLOR_ENV), None)]
        );
    }

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
        super::replace_wrapper_alias(&mut super::RunningExecutable::open_path(&source)?, &alias)?;
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
        let alias = super::wrapper_alias(
            directory.path(),
            &mut super::RunningExecutable::open_path(&source)?,
        )
        .context("create alias")?;
        fs::remove_file(&alias)?;
        fs::hard_link(&source, &alias)?;
        assert_eq!(
            wrapper_alias_resolution(&alias),
            WrapperAliasResolution::NeedsReplacement
        );
        assert_eq!(
            super::wrapper_alias(
                directory.path(),
                &mut super::RunningExecutable::open_path(&source)?
            )?,
            alias
        );
        let inode = fs::metadata(&alias)?.ino();
        assert_ne!(inode, fs::metadata(&source)?.ino());
        assert_eq!(fs::metadata(&alias)?.nlink(), 1);
        assert_eq!(
            super::wrapper_alias(
                directory.path(),
                &mut super::RunningExecutable::open_path(&source)?
            )?,
            alias
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
        let alias = super::wrapper_alias(
            directory.path(),
            &mut super::RunningExecutable::open_path(&source)?,
        )
        .context("create copy")?;
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
        let alias = super::wrapper_alias(
            directory.path(),
            &mut super::RunningExecutable::open_path(&source)?,
        )
        .context("create copy")?;
        let file = fs::OpenOptions::new().write(true).open(&source)?;
        let mut executable = super::RunningExecutable { file };
        assert!(super::replace_wrapper_alias(&mut executable, &alias).is_err());
        assert_eq!(fs::read(&alias)?, b"retained build");
        let staging = alias.with_extension(format!(
            "{}-{}",
            super::WRAPPER_ALIAS_STAGING_EXTENSION,
            std::process::id()
        ));
        assert!(!staging.try_exists()?);
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
        let alias = super::wrapper_alias(
            directory.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )
        .context("create alias")?;
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
    fn wrapper_alias_uses_the_rebuilt_executable_with_the_same_fingerprint() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let installed = directory.path().join("cargo-mend");
        fs::write(&installed, b"first build")?;
        let original = super::wrapper_alias(
            directory.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )
        .context("first alias")?;
        let modified = fs::metadata(&installed)?.modified()? + std::time::Duration::from_secs(1);

        // Keep the fingerprint and length unchanged. Set the time explicitly
        // so filesystem clock resolution cannot decide whether this is a rebuild.
        fs::write(&installed, b"fresh build")?;
        fs::File::options()
            .write(true)
            .open(&installed)?
            .set_modified(modified)?;
        let rebuilt = super::wrapper_alias(
            directory.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )
        .context("rebuilt alias")?;
        assert_eq!(fs::read(&rebuilt)?, b"fresh build");
        assert_ne!(rebuilt, original);
        assert_eq!(fs::read(original)?, b"first build");
        Ok(())
    }

    #[test]
    fn run_wrapper_stays_the_same_after_source_replacement_and_removal() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let installed = directory.path().join("cargo-mend");
        fs::write(&installed, b"running build")?;
        #[cfg(unix)]
        fs::set_permissions(&installed, fs::Permissions::from_mode(0o755))?;
        let wrapper = super::RunWrapper::resolve(
            directory.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )?;
        let first = configured_wrapper_path(&wrapper)?;
        let replacement = directory.path().join("replacement");
        fs::write(&replacement, b"replacement build")?;
        fs::rename(replacement, &installed)?;
        let second = configured_wrapper_path(&wrapper)?;
        assert_eq!(second, first);
        assert!(second.is_file());
        assert_eq!(fs::read(&second)?, b"running build");
        fs::remove_file(&installed)?;
        let third = configured_wrapper_path(&wrapper)?;
        assert_eq!(third, first);
        assert!(third.is_file());
        assert_eq!(fs::read(third)?, b"running build");
        Ok(())
    }

    #[test]
    fn run_wrapper_resolves_open_executable_after_source_replacement_and_removal()
    -> anyhow::Result<()> {
        let directory = tempdir()?;
        let installed = directory.path().join("cargo-mend");
        fs::write(&installed, b"running build")?;
        let mut executable = super::RunningExecutable::open_path(&installed)?;
        let replacement = directory.path().join("replacement");
        fs::write(&replacement, b"replacement build")?;
        fs::rename(replacement, &installed)?;
        fs::remove_file(&installed)?;
        let wrapper = super::RunWrapper::resolve(directory.path(), &mut executable)?;
        assert_eq!(fs::read(&wrapper.path)?, b"running build");
        drop(executable);
        Ok(())
    }

    fn configured_wrapper_path(wrapper: &super::RunWrapper) -> anyhow::Result<PathBuf> {
        let mut command = Command::new(CARGO_BIN);
        wrapper.configure(&mut command)?;
        command
            .get_envs()
            .find_map(|(key, value)| {
                if key == crate::compiler::constants::RUSTC_WORKSPACE_WRAPPER_ENV {
                    value.map(std::path::PathBuf::from)
                } else {
                    None
                }
            })
            .context("the driver command must have a wrapper")
    }

    #[test]
    fn run_wrapper_reports_source_and_copy_failures() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let source = directory.path().join("cargo-mend");
        let missing = super::RunningExecutable::open_path(&source)
            .err()
            .context("a missing source must fail")?;
        assert!(format!("{missing:#}").contains(&source.display().to_string()));
        fs::write(&source, b"running build")?;
        let target = directory.path().join("not-a-directory");
        fs::write(&target, b"occupied")?;
        let blocked =
            super::RunWrapper::resolve(&target, &mut super::RunningExecutable::open_path(&source)?)
                .err()
                .context("an invalid copy directory must fail")?;
        assert!(format!("{blocked:#}").contains(&target.display().to_string()));
        Ok(())
    }

    #[test]
    fn run_wrapper_rejects_a_missing_copy_before_configuring_cargo() -> anyhow::Result<()> {
        let directory = tempdir()?;
        let source = directory.path().join("cargo-mend");
        fs::write(&source, b"running build")?;
        let wrapper = super::RunWrapper::resolve(
            directory.path(),
            &mut super::RunningExecutable::open_path(&source)?,
        )?;
        fs::remove_file(&wrapper.path)?;
        let mut command = Command::new(CARGO_BIN);
        let error = wrapper
            .configure(&mut command)
            .err()
            .context("a missing wrapper must fail")?;
        assert!(
            error
                .to_string()
                .contains(&wrapper.path.display().to_string())
        );
        assert_eq!(command.get_envs().next(), None);
        Ok(())
    }

    #[test]
    fn wrapper_alias_retains_its_build_when_the_installed_binary_is_replaced() -> anyhow::Result<()>
    {
        let temp = tempdir()?;
        let installed = temp.path().join("cargo-mend");
        fs::write(&installed, b"first build")?;
        let alias = super::wrapper_alias(
            temp.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )
        .context("create alias")?;
        let replacement = temp.path().join("replacement");
        fs::write(&replacement, b"next build")?;
        fs::rename(replacement, &installed)?;
        assert_eq!(fs::read(&alias)?, b"first build");
        let rebuilt = super::wrapper_alias(
            temp.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )
        .context("rebuilt alias")?;
        assert_ne!(rebuilt, alias);
        assert_eq!(fs::read(rebuilt)?, b"next build");
        assert_eq!(fs::read(alias)?, b"first build");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn wrapper_alias_replaces_a_legacy_symlink_with_the_retained_build() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let installed = temp.path().join("cargo-mend");
        fs::write(&installed, b"current build")?;
        let alias = super::wrapper_alias(
            temp.path(),
            &mut super::RunningExecutable::open_path(&installed)?,
        )
        .context("create alias")?;
        fs::remove_file(&alias)?;
        std::os::unix::fs::symlink(&installed, &alias)?;
        assert_eq!(
            super::wrapper_alias(
                temp.path(),
                &mut super::RunningExecutable::open_path(&installed)?
            )?,
            alias
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
