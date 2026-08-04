//! Pure conversion of one immutable process snapshot plus the shared workspace
//! index into build sessions and compile activities.
//!
//! [`classify`] is a free function: the stateful [`super::build_classifier::BuildClassifier`]
//! and its caches live in a sibling module, so nothing here has a path to
//! mutable state. Every filesystem result classification needs — canonical
//! process paths, workspace target directories, dependency manifests — is
//! resolved into the input before the call.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use super::activity::CompilationTarget;
use super::activity::CompileActivity;
use super::activity::CompileActivityId;
use super::activity::CompiledCrateIdentity;
use super::activity::CompiledCrateName;
use super::activity::CompilerAttribution;
use super::activity::CompilerKind;
use super::activity::CompilerSessionCandidates;
use super::activity::ResolvedCompilerAttribution;
use super::activity::UnattributedCompileActivity;
use super::build_classifier::BuildDirectoryLedger;
use super::build_classifier::DependencyManifestLookup;
use super::build_classifier::DependencyManifestSnapshot;
use super::build_classifier::FirstSeenLedger;
use super::build_classifier::FirstSeenLookup;
use super::build_classifier::FirstSighting;
use super::build_classifier::ObservedBuildDirectoryLookup;
use super::constants::ALL_BENCHMARKS_ARGUMENT;
use super::constants::ALL_BINARIES_ARGUMENT;
use super::constants::ALL_EXAMPLES_ARGUMENT;
use super::constants::ALL_PACKAGES_ARGUMENT;
use super::constants::ALL_TARGETS_ARGUMENT;
use super::constants::ALL_TESTS_ARGUMENT;
use super::constants::BENCHMARK_ARGUMENT;
use super::constants::BINARY_ARGUMENT;
use super::constants::BUILD_SCRIPT_PREFIX;
use super::constants::CARGO_PLUGIN_PREFIX;
use super::constants::CLIPPY_DRIVER_EXECUTABLE;
use super::constants::CRATE_NAME_ARGUMENT;
use super::constants::EXAMPLE_ARGUMENT;
use super::constants::LIBRARY_ARGUMENT;
use super::constants::MANIFEST_PATH_ARGUMENT;
use super::constants::MAX_DESCENDANT_WALK_DEPTH;
use super::constants::MAX_MEMBER_ROOT_SEARCH_DEPTH;
use super::constants::OUT_DIRECTORY_ARGUMENT;
use super::constants::PACKAGE_ARGUMENT;
use super::constants::PACKAGE_SHORT_ARGUMENT;
use super::constants::PROFILE_ARGUMENT;
use super::constants::RELEASE_ARGUMENT;
use super::constants::RUSTC_EXECUTABLE;
use super::constants::RUSTDOC_EXECUTABLE;
use super::constants::TARGET_ARGUMENT;
use super::constants::TARGET_DIRECTORY_ARGUMENT;
use super::constants::TEST_ARGUMENT;
use super::constants::WORKSPACE_ARGUMENT;
use super::session::BuildProfile;
use super::session::BuildProfileAttribution;
use super::session::BuildProfileLabel;
use super::session::BuildSession;
use super::session::BuildSessionId;
use super::session::CargoCommandSelector;
use super::session::CargoSubcommand;
use super::session::LiveOwnedRoot;
use super::session::OperativeCargoCommand;
use super::session::OwnedRootEvidence;
use super::session::ScopeAttribution;
use super::session::SessionRootObservation;
use super::session::SessionScope;
use super::session::SessionScopeRoot;
use super::session::SessionTargetDirectory;
use super::session::TargetDirectoryEvidence;
use crate::process_observation::BuildCandidateRole;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::identity::ProcessIncarnation;
use crate::process_observation::snapshot::AncestryLookup;
use crate::process_observation::snapshot::LinkerRecognition;
use crate::process_observation::snapshot::ParentWalkDepth;
use crate::process_observation::snapshot::ProcessFieldObservation;
use crate::process_observation::snapshot::ProcessIncarnationEvidence;
use crate::process_observation::snapshot::ProcessObservationSnapshot;
use crate::project::AbsolutePath;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalPackageOwnership;
use crate::project::CanonicalPathResolution;
use crate::project::CanonicalTargetDirectory;
use crate::project::CanonicalTargetOwnership;
use crate::project::CargoWorkspaceIndex;
use crate::project::CargoWorkspaceView;
use crate::project::VisibleProjectWorkspaceOwnership;

/// A monotonic cycle counter used as the first-seen value for everything the
/// ledger does not already hold.
///
/// The pure classifier cannot write the ledger, so a whole cohort discovered in
/// one cycle — enabling the monitor mid-build, or one `cargo build` spawning
/// many compilers at once — shares this value and then orders by incarnation.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct BuildClassificationCycle(u64);

impl BuildClassificationCycle {
    /// Advance to the next cycle.
    pub(crate) const fn advance(&mut self) { self.0 = self.0.saturating_add(1); }
}

/// How a Cargo subcommand read from argv classifies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CargoSubcommandRecognition {
    /// A built-in subcommand that compiles.
    Build,
    /// A built-in subcommand that never compiles.
    NonBuild,
    /// A configured alias or third-party plugin. Whether it compiles cannot be
    /// decided from argv, and resolving it would mean reading a per-checkout
    /// `.cargo/config.toml`, so it is decided by compiler descendants instead.
    Unrecognized,
}

impl CargoSubcommandRecognition {
    /// Subcommands that compile. `install`, `package`, and `publish` belong
    /// here because they compile; a compile the user can see is a compile the
    /// user can stop, even when its output lands outside the project's target
    /// directory.
    const BUILD_SUBCOMMANDS: &'static [&'static str] = &[
        "bench", "build", "check", "clippy", "doc", "fix", "install", "nextest", "package",
        "publish", "run", "rustc", "rustdoc", "test",
    ];
    /// Subcommands that never compile.
    const NON_BUILD_SUBCOMMANDS: &'static [&'static str] = &[
        "add",
        "clean",
        "config",
        "fetch",
        "generate-lockfile",
        "help",
        "init",
        "locate-project",
        "login",
        "logout",
        "metadata",
        "new",
        "owner",
        "pkgid",
        "read-manifest",
        "remove",
        "search",
        "tree",
        "uninstall",
        "update",
        "vendor",
        "verify-project",
        "version",
        "yank",
    ];

    /// Recognize a subcommand after built-in alias normalization.
    pub(crate) fn from_subcommand(subcommand: &str) -> Self {
        let subcommand = normalize_builtin_alias(subcommand);
        if Self::BUILD_SUBCOMMANDS.contains(&subcommand) {
            Self::Build
        } else if Self::NON_BUILD_SUBCOMMANDS.contains(&subcommand) {
            Self::NonBuild
        } else {
            Self::Unrecognized
        }
    }

    /// Whether argv alone proves this root compiles.
    const fn compiles_from_argv(self) -> bool { matches!(self, Self::Build) }
}

/// Cargo's own built-in aliases. These are fixed, so normalizing them needs no
/// file read; configured aliases from `.cargo/config.toml` deliberately are not
/// normalized here.
fn normalize_builtin_alias(subcommand: &str) -> &str {
    match subcommand {
        "b" => "build",
        "c" => "check",
        "d" => "doc",
        "r" => "run",
        "t" => "test",
        "rm" => "remove",
        "ver" => "version",
        subcommand => subcommand,
    }
}

/// Whether one command line named a given path at all.
///
/// [`Self::NotNamed`] is the argv-side counterpart of
/// [`CanonicalPathResolution::Unresolved`]: the command line carried no such
/// argument, which is a different fact from an argument that was named and then
/// failed to canonicalize.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ArgumentPath {
    /// The command line named this path, exactly as written.
    Named(PathBuf),
    /// The command line named no such path.
    #[default]
    NotNamed,
}

impl ArgumentPath {
    /// The path argv named, for the canonicalization that runs before the pure
    /// call.
    pub(crate) fn named_path(&self) -> Option<&Path> {
        match self {
            Self::Named(path) => Some(path),
            Self::NotNamed => None,
        }
    }
}

/// Path arguments read from one candidate's command line, before
/// canonicalization.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ObservedProcessPathArguments {
    /// Directory containing the manifest named by `--manifest-path` or by an
    /// absolute `Cargo.toml` argument.
    pub(crate) manifest_directory:        ArgumentPath,
    /// The `--target-dir` argument.
    pub(crate) target_directory_argument: ArgumentPath,
    /// The compiler `--out-dir` argument.
    pub(crate) output_directory:          ArgumentPath,
    /// The compiler's primary input source file.
    pub(crate) primary_input:             ArgumentPath,
}

/// One candidate's canonicalized paths, resolved before the pure call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CanonicalProcessPathSet {
    working_directory:         CanonicalPathResolution<AbsolutePath>,
    manifest_directory:        CanonicalPathResolution<AbsolutePath>,
    target_directory_argument: CanonicalPathResolution<AbsolutePath>,
    output_directory:          CanonicalPathResolution<AbsolutePath>,
    primary_input:             CanonicalPathResolution<AbsolutePath>,
}

impl CanonicalProcessPathSet {
    /// Record one candidate's canonicalized paths.
    pub(crate) const fn new(
        working_directory: CanonicalPathResolution<AbsolutePath>,
        manifest_directory: CanonicalPathResolution<AbsolutePath>,
        target_directory_argument: CanonicalPathResolution<AbsolutePath>,
        output_directory: CanonicalPathResolution<AbsolutePath>,
        primary_input: CanonicalPathResolution<AbsolutePath>,
    ) -> Self {
        Self {
            working_directory,
            manifest_directory,
            target_directory_argument,
            output_directory,
            primary_input,
        }
    }
}

/// Canonicalized paths for every candidate incarnation in one cycle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CanonicalProcessPaths {
    entries: BTreeMap<ProcessIncarnation, CanonicalProcessPathSet>,
}

impl CanonicalProcessPaths {
    /// Record one candidate's canonicalized paths.
    pub(crate) fn insert(
        &mut self,
        incarnation: ProcessIncarnation,
        canonical_process_path_set: CanonicalProcessPathSet,
    ) {
        self.entries.insert(incarnation, canonical_process_path_set);
    }

    fn get(&self, incarnation: &ProcessIncarnation) -> Option<&CanonicalProcessPathSet> {
        self.entries.get(incarnation)
    }
}

/// One indexed workspace's checkout root paired with the target directory it
/// writes to, resolved once per cycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IndexedWorkspaceTargetDirectory {
    canonical_checkout_root:     CanonicalPathResolution<CanonicalCheckoutRoot>,
    target_directory_resolution: CanonicalPathResolution<CanonicalTargetDirectory>,
}

impl IndexedWorkspaceTargetDirectory {
    /// Resolve one workspace view's target directory. This performs filesystem
    /// syscalls, so it runs while the input is being prepared and never inside
    /// [`classify`].
    pub(crate) fn resolve(cargo_workspace_view: &CargoWorkspaceView) -> Self {
        Self {
            canonical_checkout_root:     cargo_workspace_view.checkout_root_resolution().clone(),
            target_directory_resolution: cargo_workspace_view.target_directory_resolution(),
        }
    }
}

/// Every indexed workspace's target directory for one cycle.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct IndexedWorkspaceTargetDirectories {
    entries: Vec<IndexedWorkspaceTargetDirectory>,
}

impl IndexedWorkspaceTargetDirectories {
    /// Resolve every indexed workspace's target directory once.
    pub(crate) fn resolve(cargo_workspace_index: &CargoWorkspaceIndex) -> Self {
        Self {
            entries: cargo_workspace_index
                .workspaces()
                .map(IndexedWorkspaceTargetDirectory::resolve)
                .collect(),
        }
    }

    fn target_directory_for(
        &self,
        canonical_checkout_root: &CanonicalCheckoutRoot,
    ) -> TargetDirectoryEvidence<'_> {
        self.entries
            .iter()
            .find(|entry| {
                matches!(
                    &entry.canonical_checkout_root,
                    CanonicalPathResolution::Resolved(checkout_root)
                        if checkout_root == canonical_checkout_root
                )
            })
            .map_or(TargetDirectoryEvidence::Unobservable, |entry| match &entry
                .target_directory_resolution
            {
                CanonicalPathResolution::Resolved(canonical_target_directory) => {
                    TargetDirectoryEvidence::Determined(canonical_target_directory)
                },
                CanonicalPathResolution::Unresolved => TargetDirectoryEvidence::Unobservable,
            })
    }
}

/// One dependency source root whose package identity is not yet cached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DependencyManifestLookupRequest {
    canonical_source_root: AbsolutePath,
}

impl DependencyManifestLookupRequest {
    /// The canonical directory whose manifest chain must be read.
    pub(crate) const fn canonical_source_root(&self) -> &AbsolutePath {
        &self.canonical_source_root
    }
}

/// Everything one classification cycle reads.
///
/// The dependency-manifest and first-seen fields are shared references into the
/// classifier's own tables rather than copies: both grow one entry per distinct
/// dependency source root and per observed incarnation, and copying them would
/// reallocate thousands of map entries twenty times a minute for a function
/// that only reads them. The reference never crosses a thread — `classify` runs
/// on the thread that builds the input.
#[derive(Clone, Copy)]
pub(crate) struct BuildClassificationInput<'a> {
    process_observation_snapshot:         &'a ProcessObservationSnapshot,
    cargo_workspace_index:                &'a CargoWorkspaceIndex,
    indexed_workspace_target_directories: &'a IndexedWorkspaceTargetDirectories,
    canonical_process_paths:              &'a CanonicalProcessPaths,
    dependency_manifest_snapshot:         &'a DependencyManifestSnapshot,
    first_seen_ledger:                    &'a FirstSeenLedger,
    build_directory_ledger:               &'a BuildDirectoryLedger,
    owned_root_evidence:                  &'a OwnedRootEvidence,
    build_classification_cycle:           BuildClassificationCycle,
    cycle_instant:                        Instant,
}

/// What one cycle observed, before any build semantics are applied.
#[derive(Clone, Copy)]
pub(super) struct ObservedCycle<'a> {
    process_observation_snapshot:         &'a ProcessObservationSnapshot,
    cargo_workspace_index:                &'a CargoWorkspaceIndex,
    indexed_workspace_target_directories: &'a IndexedWorkspaceTargetDirectories,
    canonical_process_paths:              &'a CanonicalProcessPaths,
    owned_root_evidence:                  &'a OwnedRootEvidence,
}

impl<'a> ObservedCycle<'a> {
    /// Pair one process snapshot with the index and filesystem results
    /// resolved for it.
    pub(super) const fn new(
        process_observation_snapshot: &'a ProcessObservationSnapshot,
        cargo_workspace_index: &'a CargoWorkspaceIndex,
        indexed_workspace_target_directories: &'a IndexedWorkspaceTargetDirectories,
        canonical_process_paths: &'a CanonicalProcessPaths,
        owned_root_evidence: &'a OwnedRootEvidence,
    ) -> Self {
        Self {
            process_observation_snapshot,
            cargo_workspace_index,
            indexed_workspace_target_directories,
            canonical_process_paths,
            owned_root_evidence,
        }
    }
}

/// The classifier's own tables, borrowed for the length of the pure call.
#[derive(Clone, Copy)]
pub(super) struct ClassificationTables<'a> {
    dependency_manifest_snapshot: &'a DependencyManifestSnapshot,
    first_seen_ledger:            &'a FirstSeenLedger,
    build_directory_ledger:       &'a BuildDirectoryLedger,
}

impl<'a> ClassificationTables<'a> {
    /// Borrow the classifier's caches for one classification.
    pub(super) const fn new(
        dependency_manifest_snapshot: &'a DependencyManifestSnapshot,
        first_seen_ledger: &'a FirstSeenLedger,
        build_directory_ledger: &'a BuildDirectoryLedger,
    ) -> Self {
        Self {
            dependency_manifest_snapshot,
            first_seen_ledger,
            build_directory_ledger,
        }
    }
}

/// When one classification cycle ran.
#[derive(Clone, Copy, Debug)]
pub(super) struct CycleStamp {
    build_classification_cycle: BuildClassificationCycle,
    cycle_instant:              Instant,
}

impl CycleStamp {
    /// Stamp one cycle with its counter and instant.
    pub(super) const fn new(
        build_classification_cycle: BuildClassificationCycle,
        cycle_instant: Instant,
    ) -> Self {
        Self {
            build_classification_cycle,
            cycle_instant,
        }
    }
}

impl<'a> BuildClassificationInput<'a> {
    /// Assemble one cycle's immutable input.
    pub(super) const fn new(
        observed_cycle: ObservedCycle<'a>,
        classification_tables: ClassificationTables<'a>,
        cycle_stamp: CycleStamp,
    ) -> Self {
        Self {
            process_observation_snapshot:         observed_cycle.process_observation_snapshot,
            cargo_workspace_index:                observed_cycle.cargo_workspace_index,
            indexed_workspace_target_directories: observed_cycle
                .indexed_workspace_target_directories,
            canonical_process_paths:              observed_cycle.canonical_process_paths,
            dependency_manifest_snapshot:         classification_tables
                .dependency_manifest_snapshot,
            first_seen_ledger:                    classification_tables.first_seen_ledger,
            build_directory_ledger:               classification_tables.build_directory_ledger,
            owned_root_evidence:                  observed_cycle.owned_root_evidence,
            build_classification_cycle:           cycle_stamp.build_classification_cycle,
            cycle_instant:                        cycle_stamp.cycle_instant,
        }
    }
}

/// One cycle's immutable classification result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildClassification {
    build_sessions:                      Vec<BuildSession>,
    compile_activities:                  Vec<CompileActivity>,
    unattributed_compile_activities:     Vec<UnattributedCompileActivity>,
    dependency_manifest_lookup_requests: Vec<DependencyManifestLookupRequest>,
    observed_incarnations:               Vec<ProcessIncarnation>,
    promoted_root_incarnations:          Vec<ProcessIncarnation>,
    observed_session_build_directories:  Vec<SessionBuildDirectory>,
    dependency_source_roots_in_use:      Vec<AbsolutePath>,
    classification_cycle:                BuildClassificationCycle,
    cycle_instant:                       Instant,
}

impl BuildClassification {
    /// A classification that observed nothing, for tests that need a completed
    /// cycle without a fixture behind it.
    #[cfg(test)]
    pub(super) fn empty_for_test(cycle_instant: Instant) -> Self {
        Self {
            build_sessions: Vec::new(),
            compile_activities: Vec::new(),
            unattributed_compile_activities: Vec::new(),
            dependency_manifest_lookup_requests: Vec::new(),
            observed_incarnations: Vec::new(),
            promoted_root_incarnations: Vec::new(),
            observed_session_build_directories: Vec::new(),
            dependency_source_roots_in_use: Vec::new(),
            classification_cycle: BuildClassificationCycle::default(),
            cycle_instant,
        }
    }

    /// Sessions ordered by first-seen cycle, then process incarnation.
    pub(crate) fn build_sessions(&self) -> &[BuildSession] { &self.build_sessions }

    /// Attributed compile activities ordered by first-seen cycle, then process
    /// incarnation.
    pub(crate) fn compile_activities(&self) -> &[CompileActivity] { &self.compile_activities }

    /// Ambiguous and unattributed activities, rendered once in the scope-level
    /// attribution-unavailable section.
    pub(crate) fn unattributed_compile_activities(&self) -> &[UnattributedCompileActivity] {
        &self.unattributed_compile_activities
    }

    /// Dependency source roots whose manifests the classifier must read after
    /// this call so the next cycle can identify them exactly.
    pub(crate) fn dependency_manifest_lookup_requests(&self) -> &[DependencyManifestLookupRequest] {
        &self.dependency_manifest_lookup_requests
    }

    /// Every candidate incarnation observed this cycle, for ledger pruning.
    pub(crate) fn observed_incarnations(&self) -> &[ProcessIncarnation] {
        &self.observed_incarnations
    }

    /// Cargo roots proven to compile this cycle, for sticky promotion.
    pub(crate) fn promoted_root_incarnations(&self) -> &[ProcessIncarnation] {
        &self.promoted_root_incarnations
    }

    /// Build directories this cycle's own compilers were observed writing
    /// under, for the sticky profile a session keeps between compiler children.
    pub(crate) fn observed_session_build_directories(&self) -> &[SessionBuildDirectory] {
        &self.observed_session_build_directories
    }

    /// Dependency source roots this cycle resolved from cache, for revalidation.
    pub(crate) fn dependency_source_roots_in_use(&self) -> &[AbsolutePath] {
        &self.dependency_source_roots_in_use
    }

    /// The cycle counter this classification was produced with.
    #[cfg(test)]
    pub(crate) const fn build_classification_cycle(&self) -> BuildClassificationCycle {
        self.classification_cycle
    }

    /// When this cycle was taken.
    pub(crate) const fn cycle_instant(&self) -> Instant { self.cycle_instant }
}

/// One validated Cargo root process and everything argv says about it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecognizedCargoRoot {
    incarnation:             ProcessIncarnation,
    operative_cargo_command: OperativeCargoCommand,
    build_profile:           BuildProfile,
    compilation_target:      CompilationTarget,
}

impl RecognizedCargoRoot {
    /// The exec-sensitive incarnation this root was recognized in.
    pub(crate) const fn incarnation(&self) -> &ProcessIncarnation { &self.incarnation }
}

/// One validated compiler process and everything argv says about it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecognizedCompilerProcess {
    incarnation:         ProcessIncarnation,
    compiler_kind:       CompilerKind,
    compiled_crate_name: CompiledCrateName,
    compilation_target:  CompilationTarget,
}

impl RecognizedCompilerProcess {
    /// The exec-sensitive incarnation this compiler was recognized in.
    pub(crate) const fn incarnation(&self) -> &ProcessIncarnation { &self.incarnation }
}

/// Whether every candidate in this cycle carried readable argv and working
/// directory evidence.
///
/// A candidate the system-wide uniqueness test cannot see makes a "unique"
/// match false-unique, so one blind candidate degrades every uniqueness claim
/// in the cycle to ambiguous rather than authorizing a kill on the wrong
/// session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CycleIdentityCompleteness {
    /// Every candidate's argv and working directory were readable.
    Complete,
    /// At least one candidate carried insufficient identity evidence.
    Degraded,
}

/// One Cargo root while it is being classified.
#[derive(Clone)]
struct CargoRootUnderClassification {
    identity:                 ProcessIdentity,
    recognized_cargo_root:    RecognizedCargoRoot,
    session_scope:            SessionScope,
    session_target_directory: SessionTargetDirectory,
    /// The profile argv or the ledger names, resolved before compilers are
    /// attached so an orphan compiler's output directory is compared against
    /// the profile the session actually builds.
    build_profile:            BuildProfile,
}

/// Convert one immutable snapshot plus the shared index into build sessions and
/// compile activities. The result is a function of the input alone.
pub(super) fn classify(input: BuildClassificationInput<'_>) -> BuildClassification {
    let CollectedCandidates {
        mut observed_incarnations,
        cycle_identity_completeness,
        cargo_roots,
        compilers,
    } = collect_candidates(&input);
    let cargo_roots = drop_nested_cargo_roots(&input, &cargo_roots);
    let AttachedCompilers {
        mut promoted_root_incarnations,
        mut compile_activities,
        mut unattributed_compile_activities,
        mut dependency_manifest_lookup_requests,
        mut dependency_source_roots_in_use,
        mut observed_build_directories,
    } = attach_compilers(
        &input,
        &cargo_roots,
        &compilers,
        cycle_identity_completeness,
    );

    let mut build_sessions = build_sessions_for_compiling_roots(
        &input,
        &cargo_roots,
        &promoted_root_incarnations,
        &observed_build_directories,
    );

    build_sessions.sort_by(|left, right| {
        left.first_seen()
            .cmp(&right.first_seen())
            .then_with(|| left.build_session_id().cmp(right.build_session_id()))
    });
    compile_activities.sort_by(|left, right| {
        left.first_seen()
            .cmp(&right.first_seen())
            .then_with(|| left.compile_activity_id().cmp(right.compile_activity_id()))
    });
    unattributed_compile_activities
        .sort_by(|left, right| left.compile_activity_id().cmp(right.compile_activity_id()));
    dependency_manifest_lookup_requests.sort_by(|left, right| {
        left.canonical_source_root
            .as_path()
            .cmp(right.canonical_source_root.as_path())
    });
    dependency_manifest_lookup_requests.dedup();
    promoted_root_incarnations.sort();
    promoted_root_incarnations.dedup();
    observed_incarnations.sort();
    observed_incarnations.dedup();
    dependency_source_roots_in_use.sort_by(|left, right| left.as_path().cmp(right.as_path()));
    dependency_source_roots_in_use.dedup();
    observed_build_directories.sort_by(|left, right| {
        left.build_session_id
            .cmp(&right.build_session_id)
            .then_with(|| left.build_directory_name.cmp(&right.build_directory_name))
    });
    observed_build_directories.dedup();

    BuildClassification {
        build_sessions,
        compile_activities,
        unattributed_compile_activities,
        dependency_manifest_lookup_requests,
        observed_incarnations,
        promoted_root_incarnations,
        observed_session_build_directories: observed_build_directories,
        dependency_source_roots_in_use,
        classification_cycle: input.build_classification_cycle,
        cycle_instant: input.cycle_instant,
    }
}

/// One `BuildSession` per Cargo root that is compiling this cycle — a root
/// whose argv compiles, a root a compiler was attached to, or a root the
/// `first_seen_ledger` already promoted. Each session carries the profile
/// resolved against `observed_build_directories` and the root's first sighting,
/// so a session keeps the instant it was first observed across later cycles.
fn build_sessions_for_compiling_roots(
    input: &BuildClassificationInput<'_>,
    cargo_roots: &[CargoRootUnderClassification],
    promoted_root_incarnations: &[ProcessIncarnation],
    observed_build_directories: &[SessionBuildDirectory],
) -> Vec<BuildSession> {
    cargo_roots
        .iter()
        .filter(|cargo_root| {
            cargo_root
                .recognized_cargo_root
                .operative_cargo_command
                .recognition()
                .compiles_from_argv()
                || promoted_root_incarnations
                    .contains(&cargo_root.recognized_cargo_root.incarnation)
                || input
                    .first_seen_ledger
                    .is_promoted(&cargo_root.recognized_cargo_root.incarnation)
        })
        .map(|cargo_root| {
            let build_session_id =
                BuildSessionId::from_recognized_root(&cargo_root.recognized_cargo_root);
            let first_sighting =
                first_sighting_for(input, &cargo_root.recognized_cargo_root.incarnation);
            let build_profile = resolve_build_profile(
                &cargo_root.build_profile,
                &build_session_id,
                observed_build_directories,
            );
            BuildSession::new(
                build_session_id,
                cargo_root
                    .recognized_cargo_root
                    .operative_cargo_command
                    .clone(),
                cargo_root.session_scope.clone(),
                cargo_root.session_target_directory.clone(),
                build_profile,
                SessionRootObservation::new(
                    cargo_root.identity.clone(),
                    first_sighting.first_observed_at(),
                ),
                first_sighting.first_seen(),
            )
        })
        .collect()
}

/// Everything one cycle's compiler pass established.
struct AttachedCompilers {
    promoted_root_incarnations:          Vec<ProcessIncarnation>,
    compile_activities:                  Vec<CompileActivity>,
    unattributed_compile_activities:     Vec<UnattributedCompileActivity>,
    dependency_manifest_lookup_requests: Vec<DependencyManifestLookupRequest>,
    dependency_source_roots_in_use:      Vec<AbsolutePath>,
    observed_build_directories:          Vec<SessionBuildDirectory>,
}

/// The build directory one session's own compiler was observed writing under.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionBuildDirectory {
    build_session_id:     BuildSessionId,
    build_directory_name: String,
}

impl SessionBuildDirectory {
    /// The session whose own compiler wrote under this build directory.
    pub(crate) const fn build_session_id(&self) -> &BuildSessionId { &self.build_session_id }

    /// The profile-named directory immediately inside the session's target
    /// directory.
    pub(crate) fn build_directory_name(&self) -> &str { &self.build_directory_name }
}

/// Pass two: attach each compiler descendant to the session that owns it, and
/// promote the Cargo roots that a live compiler proves are building.
fn attach_compilers(
    input: &BuildClassificationInput<'_>,
    cargo_roots: &[CargoRootUnderClassification],
    compilers: &[(ProcessIdentity, RecognizedCompilerProcess)],
    cycle_identity_completeness: CycleIdentityCompleteness,
) -> AttachedCompilers {
    let mut promoted_root_incarnations = Vec::new();
    let mut compile_activities = Vec::new();
    let mut unattributed_compile_activities = Vec::new();
    let mut dependency_manifest_lookup_requests = Vec::new();
    let mut dependency_source_roots_in_use = Vec::new();
    let mut observed_build_directories = Vec::new();

    for (compiler_identity, recognized_compiler_process) in compilers {
        let compiler_attribution = attribute_compiler(
            input,
            compiler_identity,
            recognized_compiler_process,
            cargo_roots,
            cycle_identity_completeness,
        );
        let compiled_crate_identity = resolve_compiled_crate_identity(
            input,
            recognized_compiler_process,
            &mut dependency_manifest_lookup_requests,
            &mut dependency_source_roots_in_use,
        );
        let compile_activity_id =
            CompileActivityId::from_recognized_compiler(recognized_compiler_process);
        let first_seen = first_seen_for(input, recognized_compiler_process.incarnation());
        match compiler_attribution.attributed_session() {
            super::activity::AttributedSession::Session(build_session_id) => {
                if let Some(cargo_root) = cargo_roots.iter().find(|cargo_root| {
                    BuildSessionId::from_recognized_root(&cargo_root.recognized_cargo_root)
                        == *build_session_id
                }) {
                    promoted_root_incarnations
                        .push(cargo_root.recognized_cargo_root.incarnation.clone());
                    if let Some(build_directory_name) =
                        observed_build_directory(input, recognized_compiler_process, cargo_root)
                    {
                        observed_build_directories.push(SessionBuildDirectory {
                            build_session_id: build_session_id.clone(),
                            build_directory_name,
                        });
                    }
                }
                compile_activities.push(CompileActivity::new(
                    compile_activity_id,
                    recognized_compiler_process.compiler_kind,
                    compiler_attribution,
                    compiled_crate_identity,
                    recognized_compiler_process.compilation_target.clone(),
                    first_seen,
                ));
            },
            super::activity::AttributedSession::Unresolved => {
                unattributed_compile_activities.push(UnattributedCompileActivity::new(
                    compile_activity_id,
                    recognized_compiler_process.compiler_kind,
                    compiled_crate_identity,
                    compiler_attribution,
                ));
            },
        }
    }

    AttachedCompilers {
        promoted_root_incarnations,
        compile_activities,
        unattributed_compile_activities,
        dependency_manifest_lookup_requests,
        dependency_source_roots_in_use,
        observed_build_directories,
    }
}

/// The build directory a compiler's `--out-dir` sits in, when that output
/// directory is inside its own session's target directory. Under `--target` the
/// triple sits between the target directory and the build directory.
fn observed_build_directory(
    input: &BuildClassificationInput<'_>,
    recognized_compiler_process: &RecognizedCompilerProcess,
    cargo_root: &CargoRootUnderClassification,
) -> Option<String> {
    let TargetDirectoryEvidence::Determined(canonical_target_directory) =
        cargo_root.session_target_directory.evidence()
    else {
        return None;
    };
    let CanonicalPathResolution::Resolved(output_directory) = &input
        .canonical_process_paths
        .get(&recognized_compiler_process.incarnation)?
        .output_directory
    else {
        return None;
    };

    let mut output_root = canonical_target_directory.path().to_path_buf();
    if let CompilationTarget::Explicit(triple) = &recognized_compiler_process.compilation_target {
        output_root.push(triple);
    }
    output_directory
        .as_path()
        .strip_prefix(&output_root)
        .ok()?
        .components()
        .next()
        .and_then(|build_directory| build_directory.as_os_str().to_str())
        .map(str::to_owned)
}

/// Prefer the profile the Cargo command line named. Otherwise let the build
/// directory this session's own compilers were last seen writing under name it:
/// a profile that comes from a Cargo alias or a `.cargo/config.toml` never
/// appears in argv, and the output path is the only evidence of it
/// classification can see.
///
/// The ledger answer is what makes the label stable. Pass two rebuilds its
/// live-compiler evidence every cycle, so reading only that evidence would
/// alternate the label as compiler children come and go. It also runs before
/// pass two, so the profile it yields is the one
/// [`expected_output_root`] compares an orphan compiler's `--out-dir` against.
fn sticky_build_profile(
    argv_build_profile: &BuildProfile,
    build_session_id: &BuildSessionId,
    build_directory_ledger: &BuildDirectoryLedger,
) -> BuildProfile {
    if argv_build_profile.attribution() == BuildProfileAttribution::Argument {
        return argv_build_profile.clone();
    }
    match build_directory_ledger.lookup(build_session_id) {
        ObservedBuildDirectoryLookup::Observed(build_directory_name) => BuildProfile::new(
            BuildProfileLabel::from_build_directory_name(build_directory_name),
            BuildProfileAttribution::OutputDirectory,
        ),
        ObservedBuildDirectoryLookup::NotObserved => argv_build_profile.clone(),
    }
}

/// Let this cycle's own compiler output replace the sticky profile, so a
/// session that changes profile is relabeled on the cycle its first compiler
/// writes under the new build directory.
fn resolve_build_profile(
    sticky_build_profile: &BuildProfile,
    build_session_id: &BuildSessionId,
    observed_build_directories: &[SessionBuildDirectory],
) -> BuildProfile {
    if sticky_build_profile.attribution() == BuildProfileAttribution::Argument {
        return sticky_build_profile.clone();
    }
    observed_build_directories
        .iter()
        .find(|observed| &observed.build_session_id == build_session_id)
        .map_or_else(
            || sticky_build_profile.clone(),
            |observed_build_directory| {
                BuildProfile::new(
                    BuildProfileLabel::from_build_directory_name(
                        &observed_build_directory.build_directory_name,
                    ),
                    BuildProfileAttribution::OutputDirectory,
                )
            },
        )
}

/// Everything one cycle's candidate pass established, before compilers are
/// attached to roots.
struct CollectedCandidates {
    observed_incarnations:       Vec<ProcessIncarnation>,
    cycle_identity_completeness: CycleIdentityCompleteness,
    cargo_roots:                 Vec<CargoRootUnderClassification>,
    compilers:                   Vec<(ProcessIdentity, RecognizedCompilerProcess)>,
}

/// Pass one: recognize Cargo roots and compiler processes, and resolve every
/// Cargo node's scope before any nesting decision is made.
fn collect_candidates(input: &BuildClassificationInput<'_>) -> CollectedCandidates {
    let mut observed_incarnations = Vec::new();
    let mut cycle_identity_completeness = CycleIdentityCompleteness::Complete;
    let mut cargo_roots: Vec<CargoRootUnderClassification> = Vec::new();
    let mut compilers: Vec<(ProcessIdentity, RecognizedCompilerProcess)> = Vec::new();

    for (incarnation, build_candidate_role) in input
        .process_observation_snapshot
        .build_candidate_incarnations()
        .iter()
    {
        let Some(record) = input
            .process_observation_snapshot
            .strongly_identified_processes()
            .get(incarnation.identity())
        else {
            continue;
        };
        if !matches!(
            record.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong { incarnation: current, .. } if current == incarnation
        ) {
            continue;
        }
        // The snapshot contains this incarnation, so it is observed for
        // ordering and promotion regardless of which of its fields are
        // readable. Recording it only when every field reads would let one
        // unreadable-cwd cycle prune a live root's sticky promotion and
        // first-seen position out of the ledger.
        observed_incarnations.push(incarnation.clone());

        let (
            ProcessFieldObservation::Observed(argv),
            ProcessFieldObservation::Observed(executable),
        ) = (record.argv(), record.executable())
        else {
            cycle_identity_completeness = CycleIdentityCompleteness::Degraded;
            continue;
        };
        // An unreadable working directory degrades every uniqueness claim in
        // the cycle, but it does not remove the candidate: a root whose argv
        // reads still resolves its scope from `--manifest-path`, and dropping
        // it here would take a finishing build's whole row out of the pane for
        // one cycle rather than degrading the evidence that actually depends on
        // the working directory.
        if !matches!(record.cwd(), ProcessFieldObservation::Observed(_)) {
            cycle_identity_completeness = CycleIdentityCompleteness::Degraded;
        }

        match build_candidate_role {
            BuildCandidateRole::Cargo => {
                let cargo_invocation = CargoInvocation::parse(executable, argv);
                let recognized_cargo_root = RecognizedCargoRoot {
                    incarnation:             incarnation.clone(),
                    operative_cargo_command: cargo_invocation.operative_cargo_command(),
                    build_profile:           cargo_invocation.build_profile(),
                    compilation_target:      cargo_invocation.compilation_target.clone(),
                };
                let session_scope = resolve_cargo_root_scope(input, record.identity(), incarnation);
                let session_target_directory =
                    resolve_session_target_directory(input, incarnation, &session_scope);
                let build_profile = sticky_build_profile(
                    &recognized_cargo_root.build_profile,
                    &BuildSessionId::from_recognized_root(&recognized_cargo_root),
                    input.build_directory_ledger,
                );
                cargo_roots.push(CargoRootUnderClassification {
                    identity: record.identity().clone(),
                    recognized_cargo_root,
                    session_scope,
                    session_target_directory,
                    build_profile,
                });
            },
            BuildCandidateRole::Compiler | BuildCandidateRole::Wrapper => {
                let compiler_invocation =
                    CompilerInvocation::parse(build_candidate_role, executable, argv);
                compilers.push((
                    record.identity().clone(),
                    RecognizedCompilerProcess {
                        incarnation:         incarnation.clone(),
                        compiler_kind:       compiler_invocation.compiler_kind,
                        compiled_crate_name: compiler_invocation.compiled_crate_name,
                        compilation_target:  compiler_invocation.compilation_target,
                    },
                ));
            },
        }
    }

    CollectedCandidates {
        observed_incarnations,
        cycle_identity_completeness,
        cargo_roots,
        compilers,
    }
}

/// When this incarnation was first observed. An incarnation the ledger has not
/// recorded yet is being seen for the first time in this cycle, so this cycle's
/// stamp is its first sighting.
fn first_sighting_for(
    input: &BuildClassificationInput<'_>,
    incarnation: &ProcessIncarnation,
) -> FirstSighting {
    match input.first_seen_ledger.lookup(incarnation) {
        FirstSeenLookup::Recorded(first_sighting) => first_sighting,
        FirstSeenLookup::NotRecorded => {
            FirstSighting::new(input.build_classification_cycle, input.cycle_instant)
        },
    }
}

fn first_seen_for(
    input: &BuildClassificationInput<'_>,
    incarnation: &ProcessIncarnation,
) -> BuildClassificationCycle {
    first_sighting_for(input, incarnation).first_seen()
}

/// A nested Cargo belongs to the outer root only when confirmed scope and
/// termination boundary match; a plugin or alias that entered another checkout
/// becomes a separate session.
fn drop_nested_cargo_roots(
    input: &BuildClassificationInput<'_>,
    cargo_roots: &[CargoRootUnderClassification],
) -> Vec<CargoRootUnderClassification> {
    let ancestor_identities: Vec<(ProcessIdentity, Vec<ProcessIdentity>)> = cargo_roots
        .iter()
        .map(|cargo_root| {
            (
                cargo_root.identity.clone(),
                validated_ancestor_identities(input, &cargo_root.identity),
            )
        })
        .collect();

    cargo_roots
        .iter()
        .enumerate()
        .filter(|(index, cargo_root)| {
            let Some((_, ancestors)) = ancestor_identities.get(*index) else {
                return true;
            };
            !cargo_roots.iter().any(|outer| {
                outer.identity != cargo_root.identity
                    && ancestors.contains(&outer.identity)
                    && outer
                        .session_scope
                        .shares_resolved_root(&cargo_root.session_scope)
            })
        })
        .map(|(_, cargo_root)| cargo_root)
        .cloned()
        .collect()
}

fn validated_ancestor_identities(
    input: &BuildClassificationInput<'_>,
    process_identity: &ProcessIdentity,
) -> Vec<ProcessIdentity> {
    match input.process_observation_snapshot.validated_ancestry(
        process_identity,
        ParentWalkDepth::new(MAX_DESCENDANT_WALK_DEPTH),
    ) {
        AncestryLookup::Observed(validated_ancestry) => validated_ancestry
            .edges()
            .iter()
            .map(|edge| edge.parent().clone())
            .collect(),
        AncestryLookup::IdentityNotInSnapshot(_) => Vec::new(),
    }
}

fn resolve_cargo_root_scope(
    input: &BuildClassificationInput<'_>,
    process_identity: &ProcessIdentity,
    incarnation: &ProcessIncarnation,
) -> SessionScope {
    if let OwnedRootEvidence::Root(live_owned_root) = input.owned_root_evidence
        && live_owned_root.root_identity() == process_identity
        && let SessionScopeRoot::Checkout(root) =
            checkout_root_for_owned_launch(input, live_owned_root)
    {
        return SessionScope::Resolved {
            method: ScopeAttribution::OwnedRoot,
            root,
        };
    }

    let Some(canonical_process_path_set) = input.canonical_process_paths.get(incarnation) else {
        return SessionScope::Unresolved;
    };

    if let CanonicalPathResolution::Resolved(manifest_directory) =
        &canonical_process_path_set.manifest_directory
        && let SessionScopeRoot::Checkout(root) = checkout_root_for_path(input, manifest_directory)
    {
        return SessionScope::Resolved {
            method: ScopeAttribution::ManifestArgument,
            root,
        };
    }

    if let CanonicalPathResolution::Resolved(working_directory) =
        &canonical_process_path_set.working_directory
    {
        let mut candidate = working_directory.as_path();
        loop {
            if let SessionScopeRoot::Checkout(root) =
                checkout_root_for_path(input, &AbsolutePath::from(candidate.to_path_buf()))
            {
                return SessionScope::Resolved {
                    method: ScopeAttribution::WorkingDirectoryManifest,
                    root,
                };
            }
            let Some(parent) = candidate.parent() else {
                break;
            };
            candidate = parent;
        }
    }

    if let CanonicalPathResolution::Resolved(target_directory_argument) =
        &canonical_process_path_set.target_directory_argument
        && let SessionScopeRoot::Checkout(root) = checkout_root_for_target_directory(
            input,
            &CanonicalTargetDirectory::from_canonical_path(target_directory_argument.clone()),
        )
    {
        return SessionScope::Resolved {
            method: ScopeAttribution::UniqueOutputDirectory,
            root,
        };
    }

    SessionScope::Unresolved
}

fn checkout_root_for_owned_launch(
    input: &BuildClassificationInput<'_>,
    live_owned_root: &LiveOwnedRoot,
) -> SessionScopeRoot {
    checkout_root_for_path(
        input,
        &AbsolutePath::from(live_owned_root.launch_directory().to_path_buf()),
    )
}

fn checkout_root_for_path(
    input: &BuildClassificationInput<'_>,
    canonical_path: &AbsolutePath,
) -> SessionScopeRoot {
    match input
        .cargo_workspace_index
        .workspace_for_canonical_owner(canonical_path)
    {
        VisibleProjectWorkspaceOwnership::Indexed(cargo_workspace_view) => {
            match cargo_workspace_view.checkout_root_resolution() {
                CanonicalPathResolution::Resolved(checkout_root) => {
                    SessionScopeRoot::Checkout(checkout_root.clone())
                },
                CanonicalPathResolution::Unresolved => SessionScopeRoot::Unresolved,
            }
        },
        VisibleProjectWorkspaceOwnership::Ambiguous
        | VisibleProjectWorkspaceOwnership::NotIndexed => SessionScopeRoot::Unresolved,
    }
}

/// A target directory identifies a checkout only when exactly one indexed
/// workspace writes to it. Two checkouts sharing one `--target-dir` stay
/// unresolved here rather than manufacturing a unique owner.
fn checkout_root_for_target_directory(
    input: &BuildClassificationInput<'_>,
    canonical_target_directory: &CanonicalTargetDirectory,
) -> SessionScopeRoot {
    let mut matches = input
        .indexed_workspace_target_directories
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                &entry.target_directory_resolution,
                CanonicalPathResolution::Resolved(target_directory)
                    if target_directory == canonical_target_directory
            )
        })
        .filter_map(|entry| match &entry.canonical_checkout_root {
            CanonicalPathResolution::Resolved(checkout_root) => Some(checkout_root.clone()),
            CanonicalPathResolution::Unresolved => None,
        });
    match (matches.next(), matches.next()) {
        (Some(checkout_root), None) => SessionScopeRoot::Checkout(checkout_root),
        (Some(_), Some(_)) | (None, _) => SessionScopeRoot::Unresolved,
    }
}

fn resolve_session_target_directory(
    input: &BuildClassificationInput<'_>,
    incarnation: &ProcessIncarnation,
    session_scope: &SessionScope,
) -> SessionTargetDirectory {
    if let Some(canonical_process_path_set) = input.canonical_process_paths.get(incarnation)
        && let CanonicalPathResolution::Resolved(target_directory_argument) =
            &canonical_process_path_set.target_directory_argument
    {
        return SessionTargetDirectory::Argument(CanonicalTargetDirectory::from_canonical_path(
            target_directory_argument.clone(),
        ));
    }
    match session_scope {
        SessionScope::Resolved { root, .. } => {
            match input
                .indexed_workspace_target_directories
                .target_directory_for(root)
            {
                TargetDirectoryEvidence::Determined(canonical_target_directory) => {
                    SessionTargetDirectory::Indexed(canonical_target_directory.clone())
                },
                TargetDirectoryEvidence::Unobservable => SessionTargetDirectory::Unobservable,
            }
        },
        SessionScope::Unresolved => SessionTargetDirectory::Unobservable,
    }
}

fn attribute_compiler(
    input: &BuildClassificationInput<'_>,
    compiler_identity: &ProcessIdentity,
    recognized_compiler_process: &RecognizedCompilerProcess,
    cargo_roots: &[CargoRootUnderClassification],
    cycle_identity_completeness: CycleIdentityCompleteness,
) -> CompilerAttribution {
    let ancestors = validated_ancestor_identities(input, compiler_identity);
    let mut parent_chain_candidates = CompilerSessionCandidates::default();
    for cargo_root in cargo_roots {
        if ancestors.contains(&cargo_root.identity) {
            parent_chain_candidates.include(BuildSessionId::from_recognized_root(
                &cargo_root.recognized_cargo_root,
            ));
        }
    }
    if !parent_chain_candidates.is_empty() {
        return parent_chain_candidates
            .attribution(ResolvedCompilerAttribution::ValidatedParentChain);
    }

    let output_candidates =
        output_directory_candidates(input, recognized_compiler_process, cargo_roots);
    if output_candidates.len() == 1 {
        return match cycle_identity_completeness {
            CycleIdentityCompleteness::Complete => {
                output_candidates.attribution(ResolvedCompilerAttribution::OutputDirectoryMatch)
            },
            CycleIdentityCompleteness::Degraded => output_candidates.degraded_attribution(),
        };
    }

    // A shared `--target-dir` makes the target-directory test select more than
    // one session. Scope filtering would only hide the ambiguity, so the
    // compiler's own primary input path decides instead.
    let primary_input_candidates =
        primary_input_candidates(input, recognized_compiler_process, cargo_roots);
    if primary_input_candidates.len() == 1 {
        return match cycle_identity_completeness {
            CycleIdentityCompleteness::Complete => {
                primary_input_candidates.attribution(ResolvedCompilerAttribution::PrimaryInputMatch)
            },
            CycleIdentityCompleteness::Degraded => primary_input_candidates.degraded_attribution(),
        };
    }

    let mut ambiguous = output_candidates;
    for build_session_id in primary_input_candidates.candidates() {
        ambiguous.include(build_session_id.clone());
    }
    ambiguous.degraded_attribution()
}

/// `(target directory, build directory, target triple)` is one path, not three
/// independent facts: the build directory is the profile-named directory
/// immediately inside the target directory, and under `--target` the triple
/// already sits between them. A session whose target directory is unobservable
/// takes no part in the test in either direction.
fn output_directory_candidates(
    input: &BuildClassificationInput<'_>,
    recognized_compiler_process: &RecognizedCompilerProcess,
    cargo_roots: &[CargoRootUnderClassification],
) -> CompilerSessionCandidates {
    let mut candidates = CompilerSessionCandidates::default();
    let Some(canonical_process_path_set) = input
        .canonical_process_paths
        .get(&recognized_compiler_process.incarnation)
    else {
        return candidates;
    };
    let CanonicalPathResolution::Resolved(output_directory) =
        &canonical_process_path_set.output_directory
    else {
        return candidates;
    };

    for cargo_root in cargo_roots {
        let TargetDirectoryEvidence::Determined(canonical_target_directory) =
            cargo_root.session_target_directory.evidence()
        else {
            continue;
        };
        let expected = expected_output_root(
            canonical_target_directory,
            &cargo_root.build_profile,
            &recognized_compiler_process.compilation_target,
        );
        if output_directory.as_path().starts_with(&expected) {
            candidates.include(BuildSessionId::from_recognized_root(
                &cargo_root.recognized_cargo_root,
            ));
        }
    }
    candidates
}

fn expected_output_root(
    canonical_target_directory: &CanonicalTargetDirectory,
    build_profile: &BuildProfile,
    compilation_target: &CompilationTarget,
) -> PathBuf {
    let mut expected = canonical_target_directory.path().to_path_buf();
    if let CompilationTarget::Explicit(triple) = compilation_target {
        expected.push(triple);
    }
    expected.push(build_profile.label().build_directory_name());
    expected
}

/// A compiler's primary input path identifies its owning session regardless of
/// a shared target directory.
fn primary_input_candidates(
    input: &BuildClassificationInput<'_>,
    recognized_compiler_process: &RecognizedCompilerProcess,
    cargo_roots: &[CargoRootUnderClassification],
) -> CompilerSessionCandidates {
    let mut candidates = CompilerSessionCandidates::default();
    let Some(canonical_process_path_set) = input
        .canonical_process_paths
        .get(&recognized_compiler_process.incarnation)
    else {
        return candidates;
    };
    let CanonicalPathResolution::Resolved(primary_input) =
        &canonical_process_path_set.primary_input
    else {
        return candidates;
    };

    for cargo_root in cargo_roots {
        let SessionScope::Resolved {
            root: checkout_root,
            ..
        } = &cargo_root.session_scope
        else {
            continue;
        };
        if primary_input
            .as_path()
            .starts_with(checkout_root.path().as_path())
        {
            candidates.include(BuildSessionId::from_recognized_root(
                &cargo_root.recognized_cargo_root,
            ));
        }
    }
    candidates
}

fn resolve_compiled_crate_identity(
    input: &BuildClassificationInput<'_>,
    recognized_compiler_process: &RecognizedCompilerProcess,
    dependency_manifest_lookup_requests: &mut Vec<DependencyManifestLookupRequest>,
    dependency_source_roots_in_use: &mut Vec<AbsolutePath>,
) -> CompiledCrateIdentity {
    let crate_name_only = || {
        CompiledCrateIdentity::CrateNameOnly(
            recognized_compiler_process.compiled_crate_name.clone(),
        )
    };
    let Some(canonical_process_path_set) = input
        .canonical_process_paths
        .get(&recognized_compiler_process.incarnation)
    else {
        return crate_name_only();
    };
    let CanonicalPathResolution::Resolved(primary_input) =
        &canonical_process_path_set.primary_input
    else {
        return crate_name_only();
    };

    if let CanonicalTargetOwnership::Indexed(cargo_package_identity) = input
        .cargo_workspace_index
        .package_for_canonical_target_source(primary_input)
    {
        return CompiledCrateIdentity::WorkspacePackage(
            cargo_package_identity.package_id().clone(),
        );
    }

    let Some(source_root) = primary_input.as_path().parent() else {
        return crate_name_only();
    };

    // A compiler whose primary input is not itself an indexed target source —
    // an included module, a generated file, or a target the index has not
    // caught up with — still belongs to the indexed member root above it.
    if let CanonicalPackageOwnership::Indexed(cargo_package_identity) =
        package_for_containing_member_root(input.cargo_workspace_index, source_root)
    {
        return CompiledCrateIdentity::WorkspacePackage(
            cargo_package_identity.package_id().clone(),
        );
    }

    let canonical_source_root = AbsolutePath::from(source_root.to_path_buf());
    dependency_source_roots_in_use.push(canonical_source_root.clone());
    match input
        .dependency_manifest_snapshot
        .lookup(&canonical_source_root)
    {
        DependencyManifestLookup::Resolved(manifest_package_identity) => {
            CompiledCrateIdentity::DependencyPackage(manifest_package_identity.clone())
        },
        DependencyManifestLookup::NotYetLookedUp => {
            dependency_manifest_lookup_requests.push(DependencyManifestLookupRequest {
                canonical_source_root,
            });
            crate_name_only()
        },
        DependencyManifestLookup::Absent => crate_name_only(),
    }
}

/// Walk up from a compiler's source root to the nearest indexed package member
/// root. Every path compared here is already canonical, so the walk reads no
/// filesystem.
fn package_for_containing_member_root<'a>(
    cargo_workspace_index: &'a CargoWorkspaceIndex,
    canonical_source_root: &Path,
) -> CanonicalPackageOwnership<'a> {
    let mut directory = canonical_source_root;
    for _ in 0..MAX_MEMBER_ROOT_SEARCH_DEPTH {
        let ownership = cargo_workspace_index
            .package_for_canonical_member_root(&AbsolutePath::from(directory.to_path_buf()));
        match ownership {
            CanonicalPackageOwnership::Indexed(_) | CanonicalPackageOwnership::Ambiguous => {
                return ownership;
            },
            CanonicalPackageOwnership::NotIndexed => {},
        }
        let Some(parent) = directory.parent() else {
            break;
        };
        directory = parent;
    }
    CanonicalPackageOwnership::NotIndexed
}

/// Everything one Cargo command line says, after rustup-proxy, plugin, and
/// built-in-alias normalization.
struct CargoInvocation {
    subcommand:                   CargoSubcommand,
    cargo_subcommand_recognition: CargoSubcommandRecognition,
    selectors:                    Vec<CargoCommandSelector>,
    build_profile_label:          BuildProfileLabel,
    build_profile_attribution:    BuildProfileAttribution,
    compilation_target:           CompilationTarget,
}

impl CargoInvocation {
    fn parse(executable: &Path, argv: &[OsString]) -> Self {
        let mut arguments = argv.iter().skip(1).peekable();
        let plugin_subcommand = executable
            .file_stem()
            .and_then(OsStr::to_str)
            .and_then(|stem| stem.strip_prefix(CARGO_PLUGIN_PREFIX))
            .map(str::to_owned);

        let mut subcommand = plugin_subcommand;
        let mut selectors = Vec::new();
        let mut build_profile_label = BuildProfileLabel::Dev;
        let mut build_profile_attribution = BuildProfileAttribution::MetadataDefault;
        let mut compilation_target = CompilationTarget::Host;

        while let Some(argument) = arguments.next() {
            let Some(argument) = argument.to_str() else {
                continue;
            };
            if let Some(selector) = selector_argument(argument, &mut arguments) {
                selectors.push(selector);
            } else if argument == RELEASE_ARGUMENT {
                build_profile_label = BuildProfileLabel::Release;
                build_profile_attribution = BuildProfileAttribution::Argument;
            } else if let Some(value) = argument_value(argument, PROFILE_ARGUMENT, &mut arguments) {
                build_profile_label = BuildProfileLabel::from_label(&value);
                build_profile_attribution = BuildProfileAttribution::Argument;
            } else if let Some(value) = argument_value(argument, TARGET_ARGUMENT, &mut arguments) {
                compilation_target = CompilationTarget::Explicit(value);
            } else if !argument.starts_with('-')
                && !argument.starts_with('+')
                && subcommand.is_none()
            {
                subcommand = Some(argument.to_owned());
            }
        }

        Self {
            cargo_subcommand_recognition: subcommand.as_deref().map_or(
                CargoSubcommandRecognition::Unrecognized,
                CargoSubcommandRecognition::from_subcommand,
            ),
            subcommand: subcommand.map_or(CargoSubcommand::Absent, CargoSubcommand::Named),
            selectors,
            build_profile_label,
            build_profile_attribution,
            compilation_target,
        }
    }

    /// The command the pane shows and the classifier decides from, as one
    /// value.
    fn operative_cargo_command(&self) -> OperativeCargoCommand {
        OperativeCargoCommand::new(
            self.subcommand.clone(),
            self.cargo_subcommand_recognition,
            self.selectors.clone(),
        )
    }

    fn build_profile(&self) -> BuildProfile {
        BuildProfile::new(
            self.build_profile_label.clone(),
            self.build_profile_attribution,
        )
    }
}

/// Everything one compiler command line says.
struct CompilerInvocation {
    compiler_kind:       CompilerKind,
    compiled_crate_name: CompiledCrateName,
    compilation_target:  CompilationTarget,
}

impl CompilerInvocation {
    fn parse(
        build_candidate_role: BuildCandidateRole,
        executable: &Path,
        argv: &[OsString],
    ) -> Self {
        let file_stem = executable
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        // The executable name decides first. `clippy-driver` is a wrapper by
        // candidate role, so branching on the role before the name would render
        // every `cargo clippy` compile as a generic wrapper. A linker is
        // recognized by the same [`LinkerRecognition`] table that admitted it as
        // a candidate, so the two never disagree about a name like `ld64.lld`.
        let compiler_kind = match LinkerRecognition::of_executable(executable) {
            LinkerRecognition::Linker => CompilerKind::Linker,
            LinkerRecognition::NotLinker => match file_stem {
                CLIPPY_DRIVER_EXECUTABLE => CompilerKind::ClippyDriver,
                RUSTDOC_EXECUTABLE => CompilerKind::Rustdoc,
                RUSTC_EXECUTABLE => CompilerKind::Rustc,
                stem if stem.starts_with(BUILD_SCRIPT_PREFIX) => CompilerKind::BuildScript,
                _ => match build_candidate_role {
                    BuildCandidateRole::Wrapper => CompilerKind::Wrapper,
                    BuildCandidateRole::Cargo | BuildCandidateRole::Compiler => CompilerKind::Rustc,
                },
            },
        };

        let mut arguments = argv.iter().skip(1).peekable();
        let mut compiled_crate_name = CompiledCrateName::from(file_stem.to_owned());
        let mut compilation_target = CompilationTarget::Host;
        while let Some(argument) = arguments.next() {
            let Some(argument) = argument.to_str() else {
                continue;
            };
            if let Some(value) = argument_value(argument, CRATE_NAME_ARGUMENT, &mut arguments) {
                compiled_crate_name = CompiledCrateName::from(value);
            } else if let Some(value) = argument_value(argument, TARGET_ARGUMENT, &mut arguments) {
                compilation_target = CompilationTarget::Explicit(value);
            }
        }

        Self {
            compiler_kind,
            compiled_crate_name,
            compilation_target,
        }
    }
}

/// Read `--name value` or `--name=value`, consuming the separate value form.
/// Recognize one selector argument, consuming its value when it takes a
/// separate one. Selectors are recorded as argv spelled them; nothing here
/// resolves a name against the workspace.
fn selector_argument<'a>(
    argument: &str,
    arguments: &mut std::iter::Peekable<impl Iterator<Item = &'a OsString>>,
) -> Option<CargoCommandSelector> {
    match argument {
        WORKSPACE_ARGUMENT | ALL_PACKAGES_ARGUMENT => {
            return Some(CargoCommandSelector::AllPackages);
        },
        LIBRARY_ARGUMENT => return Some(CargoCommandSelector::Library),
        ALL_BINARIES_ARGUMENT => return Some(CargoCommandSelector::AllBinaries),
        ALL_EXAMPLES_ARGUMENT => return Some(CargoCommandSelector::AllExamples),
        ALL_TESTS_ARGUMENT => return Some(CargoCommandSelector::AllTests),
        ALL_BENCHMARKS_ARGUMENT => return Some(CargoCommandSelector::AllBenchmarks),
        ALL_TARGETS_ARGUMENT => return Some(CargoCommandSelector::AllTargets),
        _ => {},
    }

    if let Some(package) = argument_value(argument, PACKAGE_ARGUMENT, arguments)
        .or_else(|| argument_value(argument, PACKAGE_SHORT_ARGUMENT, arguments))
    {
        return Some(CargoCommandSelector::Package(package));
    }
    if let Some(binary) = argument_value(argument, BINARY_ARGUMENT, arguments) {
        return Some(CargoCommandSelector::Binary(binary));
    }
    if let Some(example) = argument_value(argument, EXAMPLE_ARGUMENT, arguments) {
        return Some(CargoCommandSelector::Example(example));
    }
    if let Some(test) = argument_value(argument, TEST_ARGUMENT, arguments) {
        return Some(CargoCommandSelector::Test(test));
    }
    argument_value(argument, BENCHMARK_ARGUMENT, arguments).map(CargoCommandSelector::Benchmark)
}

fn argument_value<'a>(
    argument: &str,
    name: &str,
    arguments: &mut std::iter::Peekable<impl Iterator<Item = &'a OsString>>,
) -> Option<String> {
    if argument == name {
        return arguments
            .next()
            .and_then(|value| value.to_str())
            .map(str::to_owned);
    }
    argument
        .strip_prefix(name)
        .and_then(|remainder| remainder.strip_prefix('='))
        .map(str::to_owned)
}

/// Read the path arguments the classifier must canonicalize before the pure
/// call. Shares the argument grammar with [`CargoInvocation`] and
/// [`CompilerInvocation`] so the two never disagree about which token is a
/// path.
pub(crate) fn observed_process_path_arguments(
    build_candidate_role: BuildCandidateRole,
    argv: &[OsString],
) -> ObservedProcessPathArguments {
    let mut observed = ObservedProcessPathArguments::default();
    let mut arguments = argv.iter().skip(1).peekable();
    while let Some(argument) = arguments.next() {
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if let Some(value) = argument_value(argument, MANIFEST_PATH_ARGUMENT, &mut arguments) {
            observed.manifest_directory = manifest_directory_of(&value);
        } else if let Some(value) =
            argument_value(argument, TARGET_DIRECTORY_ARGUMENT, &mut arguments)
        {
            observed.target_directory_argument = ArgumentPath::Named(PathBuf::from(value));
        } else if let Some(value) = argument_value(argument, OUT_DIRECTORY_ARGUMENT, &mut arguments)
        {
            observed.output_directory = ArgumentPath::Named(PathBuf::from(value));
        } else if Path::new(argument)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
            && Path::new(argument).is_absolute()
        {
            observed.manifest_directory = manifest_directory_of(argument);
        } else if Path::new(argument)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
            && matches!(
                build_candidate_role,
                BuildCandidateRole::Compiler | BuildCandidateRole::Wrapper
            )
            && matches!(observed.primary_input, ArgumentPath::NotNamed)
        {
            observed.primary_input = ArgumentPath::Named(PathBuf::from(argument));
        }
    }
    observed
}

/// The directory holding a named manifest. A manifest argument with no parent
/// directory names nothing this classifier can resolve a scope against.
fn manifest_directory_of(manifest_argument: &str) -> ArgumentPath {
    Path::new(manifest_argument)
        .parent()
        .map_or(ArgumentPath::NotNamed, |manifest_directory| {
            ArgumentPath::Named(manifest_directory.to_path_buf())
        })
}
