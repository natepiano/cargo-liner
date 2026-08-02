use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::OnceLock;
#[cfg(test)]
use std::time::Duration;
use std::time::Instant;

use tui_pane::RollingMean;

use super::RunningTargetTerminationCapability;
use super::constants::ANCESTOR_WALK_CAP;
use super::constants::CARGO_BIN_DIR;
use super::constants::MIN_HEX_HASH_LEN;
use crate::constants::CARGO_COMMAND_NAME;
use crate::constants::DOT_CARGO_DIR;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::snapshot::AncestryLookup;
use crate::process_observation::snapshot::AncestryTerminal;
use crate::process_observation::snapshot::ParentWalkDepth;
use crate::process_observation::snapshot::ProcessFieldObservation;
use crate::process_observation::snapshot::ProcessObservationSnapshot;
use crate::process_observation::snapshot::RunningProcessMetricsObservation;
use crate::process_observation::snapshot::RunningProcessMetricsRecord;
use crate::project::AbsolutePath;
use crate::tui::panes::RunTargetKind;

/// Whether Cargo's install binary directory can participate in executable matching.
enum CargoInstallBinDirectory {
    Resolved(AbsolutePath),
    Unavailable,
}

impl From<Option<AbsolutePath>> for CargoInstallBinDirectory {
    fn from(absolute_path: Option<AbsolutePath>) -> Self {
        absolute_path.map_or(Self::Unavailable, Self::Resolved)
    }
}

/// Where a process appears in the Running Targets process outline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunningProcessPlacement {
    /// The process has no parent row in the outline.
    TopLevel,
    /// The process appears directly under the shown parent process.
    ChildOf { parent_pid: u32 },
}

impl From<Option<u32>> for RunningProcessPlacement {
    fn from(parent_pid: Option<u32>) -> Self {
        parent_pid.map_or(Self::TopLevel, |parent_pid| Self::ChildOf { parent_pid })
    }
}

/// Running Targets presentation state derived from immutable observer records.
pub struct RunningTargetsState {
    snapshot:                    RunningTargets,
    /// Canonical Cargo install binary directory (`~/.cargo/bin` by default).
    /// Executables living directly in a resolved directory are matched as
    /// installed binaries and surfaced as the `cargo` profile.
    cargo_install_bin_directory: CargoInstallBinDirectory,
    /// When each tracked identity was first observed, surviving each view
    /// snapshot rebuild. Drives the Running list's newest-at-bottom
    /// ordering: insert on first sight, retain only live PIDs after each
    /// observer result, and evict on [`Self::drop_instance`].
    first_seen:                  HashMap<ProcessIdentity, Instant>,
    /// Each tracked identity's [`RollingMean`] window over CPU samples;
    /// instances carry the mean. Same lifecycle as `first_seen`: fed during
    /// completed Running cycles, retained against live identities, evicted on
    /// [`Self::drop_instance`].
    cpu_history:                 HashMap<ProcessIdentity, RollingMean>,
}

#[derive(Default)]
pub struct RunningTargets {
    by_key:   HashMap<RunningKey, RunningTarget>,
    /// Untracked processes descended from tracked instances — e.g. the
    /// `cargo` and `rustc` processes a `cargo mend` run spawns. Shown
    /// nested under their parents in the Running list's outline.
    children: Vec<ChildProcess>,
}

/// One tracked target's running state: the manifest dir of the workspace
/// member that owns the target (drives the Running list's Path column) and
/// its instances, sorted by PID.
struct RunningTarget {
    member_dir: AbsolutePath,
    instances:  Vec<RunningInstance>,
}

/// One running OS process for a target. A single target can map to more
/// than one process (multiple `cargo run` invocations); each gets its own
/// instance so the pane can list them separately and kill one by PID.
#[derive(Clone)]
pub struct RunningInstance {
    /// OS process id, used to terminate the instance.
    pub pid:                    u32,
    /// CPU usage in percent. A busy multi-threaded process can exceed 100.
    pub cpu_percent:            f32,
    /// Resident memory in bytes.
    pub memory_bytes:           u64,
    /// How the target was launched, shown as the row marker.
    pub profile:                RunProfile,
    /// When the view state first observed this identity — the Running list sorts by
    /// it so the newest instance is the bottom row.
    pub first_seen:             Instant,
    /// The process's start time in seconds since the epoch, used for display.
    pub create_time:            u64,
    /// Identity-bound authority used by the confirmation transaction.
    pub termination_capability: RunningTargetTerminationCapability,
    /// Whether this instance is top-level or nested under a shown process.
    pub placement:              RunningProcessPlacement,
}

/// One untracked process descended from a tracked instance — e.g. the
/// `cargo` and `rustc` processes a `cargo mend` run spawns. Carries the
/// same live metrics as an instance so its Running row reads the same.
pub struct ChildProcess {
    /// OS process id, used to terminate the process.
    pub pid:                    u32,
    /// The process's executable name — the Target cell of its row.
    pub name:                   String,
    /// CPU usage in percent, smoothed over the poll window.
    pub cpu_percent:            f32,
    /// Resident memory in bytes.
    pub memory_bytes:           u64,
    /// When the view state first observed this identity.
    pub first_seen:             Instant,
    /// The process's start time in seconds since the epoch, used for display.
    pub create_time:            u64,
    /// Identity-bound authority used by the confirmation transaction.
    pub termination_capability: RunningTargetTerminationCapability,
    /// The direct OS parent — always itself shown in the outline, since
    /// every ancestor up to the tracked root is on the same chain.
    pub parent_pid:             u32,
}

/// How a running target's binary was launched — the marker shown in place
/// of a bare "running" flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunProfile {
    /// Exe under `target/debug/`.
    Debug,
    /// Exe under `target/release/`.
    Release,
    /// Exe is a `cargo install`ed binary in the cargo bin directory
    /// (e.g. run via a `cargo <name>` subcommand).
    Installed,
}

impl RunProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
            Self::Installed => CARGO_COMMAND_NAME,
        }
    }

    /// Whether this is a `cargo install`ed binary — the instances the
    /// Running list groups under its collapsible `cargo` header.
    pub const fn is_installed(self) -> bool { matches!(self, Self::Installed) }
}

/// Key identifying a running target. Matched against
/// `(target_dir, run_target_kind, name)`, where `target_dir` is the
/// target-directory path used for executable matching.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct RunningKey {
    pub target_dir:      AbsolutePath,
    pub run_target_kind: RunTargetKind,
    pub name:            String,
}

/// Borrowed Running Targets attribution for one workspace or unindexed
/// visible project.
pub struct RunningTargetProjectAttribution<'a> {
    /// Target-directory path used as the executable-matching boundary.
    /// This is canonical when live canonical resolution succeeds and is the
    /// declared target-directory path when canonical resolution is unavailable.
    pub executable_match_target_directory:  &'a AbsolutePath,
    /// Project or checkout root used when no declared target owner matches.
    pub fallback_owner_root:                &'a AbsolutePath,
    /// Bench target names used to recognize `deps/<name>-<hash>` executables.
    pub bench_names:                        &'a HashSet<String>,
    /// Binary target names used to recognize `cargo install`ed executables.
    pub bin_names:                          &'a HashSet<String>,
    /// Declared owner evidence for each exact `(RunTargetKind, name)` identity.
    pub(super) exact_target_owner_evidence:
        &'a HashMap<(RunTargetKind, String), ExactRunningTargetOwnerEvidence>,
}

/// Declared member ownership evidence for one exact target identity within a
/// workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ExactRunningTargetOwnerEvidence {
    /// Every declaration and visible-target observation names this member.
    Unique(AbsolutePath),
    /// Distinct members declare or visibly own the same target identity.
    Ambiguous,
}

impl ExactRunningTargetOwnerEvidence {
    pub(super) fn include(&mut self, declared_member_root: &AbsolutePath) {
        match self {
            Self::Unique(existing_root) if existing_root == declared_member_root => {},
            Self::Unique(_) => *self = Self::Ambiguous,
            Self::Ambiguous => {},
        }
    }
}

#[derive(Clone, Copy)]
enum RunningTargetOwnerAttribution<'a> {
    DeclaredMember(&'a AbsolutePath),
    AmbiguousTarget(&'a AbsolutePath),
    UndeclaredTarget(&'a AbsolutePath),
}

impl<'a> RunningTargetOwnerAttribution<'a> {
    const fn owner_root(self) -> &'a AbsolutePath {
        match self {
            Self::DeclaredMember(owner_root)
            | Self::AmbiguousTarget(owner_root)
            | Self::UndeclaredTarget(owner_root) => owner_root,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct AttributedRunningTargetExecutable {
    key:        RunningKey,
    profile:    RunProfile,
    owner_root: AbsolutePath,
}

#[derive(Debug, Eq, PartialEq)]
enum RunningTargetExecutableClassification {
    Attributed(AttributedRunningTargetExecutable),
    AmbiguousWorkspaceOwnership,
    Unrecognized,
}

enum CrossWorkspaceExactOwnerMatch {
    None,
    One(AttributedRunningTargetExecutable),
    Multiple,
}

impl CrossWorkspaceExactOwnerMatch {
    fn include(&mut self, attribution: AttributedRunningTargetExecutable) {
        match self {
            Self::None => *self = Self::One(attribution),
            Self::One(_) => *self = Self::Multiple,
            Self::Multiple => {},
        }
    }
}

enum UndeclaredTargetFallback {
    Missing,
    First(AttributedRunningTargetExecutable),
}

impl UndeclaredTargetFallback {
    fn include_first(&mut self, attribution: AttributedRunningTargetExecutable) {
        if matches!(self, Self::Missing) {
            *self = Self::First(attribution);
        }
    }
}

impl RunningTargetProjectAttribution<'_> {
    fn owner_attribution(
        &self,
        run_target_kind: RunTargetKind,
        name: &str,
    ) -> RunningTargetOwnerAttribution<'_> {
        match self
            .exact_target_owner_evidence
            .get(&(run_target_kind, name.to_string()))
        {
            Some(ExactRunningTargetOwnerEvidence::Unique(owner_root)) => {
                RunningTargetOwnerAttribution::DeclaredMember(owner_root)
            },
            Some(ExactRunningTargetOwnerEvidence::Ambiguous) => {
                RunningTargetOwnerAttribution::AmbiguousTarget(self.fallback_owner_root)
            },
            None => RunningTargetOwnerAttribution::UndeclaredTarget(self.fallback_owner_root),
        }
    }
}

impl RunningTargetsState {
    pub fn new() -> Self {
        Self {
            snapshot:                    RunningTargets::default(),
            cargo_install_bin_directory: cargo_install_bin_directory(),
            first_seen:                  HashMap::new(),
            cpu_history:                 HashMap::new(),
        }
    }

    /// Rebuild view state from one completed identity and metrics observation.
    pub fn apply_observation(
        &mut self,
        now: Instant,
        process_observation_snapshot: &ProcessObservationSnapshot,
        project_attributions: &[RunningTargetProjectAttribution<'_>],
    ) -> &RunningTargets {
        let RunningProcessMetricsObservation::Observed(running_process_metrics) =
            process_observation_snapshot.running_process_metrics()
        else {
            return &self.snapshot;
        };

        let mut by_key = self.collect_instances(
            now,
            process_observation_snapshot,
            running_process_metrics,
            project_attributions,
        );
        // Stable per-key order so the pane's instance rows (and the cursor
        // resting on one) don't reshuffle between ticks.
        let tracked_keys: HashMap<ProcessIdentity, RunningKey> = by_key
            .iter()
            .flat_map(|(key, target)| {
                target.instances.iter().map(|instance| {
                    (
                        instance.termination_capability.process_identity().clone(),
                        key.clone(),
                    )
                })
            })
            .collect();
        let tracked: HashSet<ProcessIdentity> = tracked_keys.keys().cloned().collect();
        for (key, target) in &mut by_key {
            target.instances.sort_by_key(|instance| instance.pid);
            for instance in &mut target.instances {
                instance.placement = observed_running_process_placement_for_instance(
                    process_observation_snapshot,
                    instance.termination_capability.process_identity(),
                    &tracked,
                    &tracked_keys,
                    key,
                );
            }
        }
        // Everything a tracked instance spawned, however deep: any process
        // whose ancestor chain reaches a tracked PID joins the outline.
        let mut children = Vec::new();
        for (process_identity, process_metrics) in running_process_metrics {
            debug_assert_eq!(process_metrics.identity(), process_identity);
            if tracked.contains(process_identity) {
                continue;
            }
            let RunningProcessPlacement::ChildOf { parent_pid } =
                observed_running_process_placement(
                    process_observation_snapshot,
                    process_identity,
                    &tracked,
                )
            else {
                continue;
            };
            let first_seen = *self
                .first_seen
                .entry(process_identity.clone())
                .or_insert(now);
            let cpu_percent = smoothed_cpu(
                &mut self.cpu_history,
                process_identity,
                process_metrics.cpu_percent().get(),
            );
            children.push(ChildProcess {
                pid: process_identity.pid(),
                name: process_metrics.name().to_string(),
                cpu_percent,
                memory_bytes: process_metrics.memory_bytes(),
                first_seen,
                create_time: process_metrics.start_time(),
                termination_capability: RunningTargetTerminationCapability::from_observed_identity(
                    process_identity.clone(),
                ),
                parent_pid,
            });
        }
        // Retain only PIDs still shown, so an exited PID's slot is fresh
        // when the OS reuses the number.
        let live: HashSet<ProcessIdentity> = tracked
            .iter()
            .cloned()
            .chain(
                children
                    .iter()
                    .map(|child| child.termination_capability.process_identity().clone()),
            )
            .collect();
        self.first_seen
            .retain(|process_identity, _| live.contains(process_identity));
        self.cpu_history
            .retain(|process_identity, _| live.contains(process_identity));
        self.snapshot = RunningTargets { by_key, children };
        &self.snapshot
    }

    /// One pass over the process table: attribute every process that is a
    /// workspace target binary (or an installed copy of one) to its key.
    fn collect_instances(
        &mut self,
        now: Instant,
        process_observation_snapshot: &ProcessObservationSnapshot,
        running_process_metrics: &std::collections::BTreeMap<
            ProcessIdentity,
            RunningProcessMetricsRecord,
        >,
        project_attributions: &[RunningTargetProjectAttribution<'_>],
    ) -> HashMap<RunningKey, RunningTarget> {
        let mut by_key: HashMap<RunningKey, RunningTarget> = HashMap::new();
        for (process_identity, process_metrics) in running_process_metrics {
            let Some(process_record) = process_observation_snapshot
                .strongly_identified_processes()
                .get(process_identity)
            else {
                continue;
            };
            let ProcessFieldObservation::Observed(executable) = process_record.executable() else {
                tracing::debug!(
                    pid = process_identity.pid(),
                    "running_targets_exe_unavailable"
                );
                continue;
            };
            // `cargo run`/`cargo run --example` exec a path relative to the
            // package dir, so the kernel reports a relative exe. Resolve it
            // against the process cwd so it can be matched against absolute
            // target directories.
            let executable = if executable.is_absolute() {
                Cow::Borrowed(executable.as_path())
            } else {
                match process_record.cwd() {
                    ProcessFieldObservation::Observed(cwd) => Cow::Owned(cwd.join(executable)),
                    ProcessFieldObservation::Unavailable(_)
                    | ProcessFieldObservation::Invalidated(_) => {
                        Cow::Borrowed(executable.as_path())
                    },
                }
            };
            let pid = process_identity.pid();
            let cpu_percent = process_metrics.cpu_percent().get();
            let memory_bytes = process_metrics.memory_bytes();
            let create_time = process_metrics.start_time();
            let termination_capability = RunningTargetTerminationCapability::from_observed_identity(
                process_identity.clone(),
            );
            match classify_exe(&executable, project_attributions) {
                RunningTargetExecutableClassification::Attributed(attribution) => {
                    let first_seen = *self
                        .first_seen
                        .entry(process_identity.clone())
                        .or_insert(now);
                    let cpu_percent =
                        smoothed_cpu(&mut self.cpu_history, process_identity, cpu_percent);
                    push_instance(
                        &mut by_key,
                        attribution.key,
                        attribution.owner_root,
                        instance(
                            pid,
                            cpu_percent,
                            memory_bytes,
                            attribution.profile,
                            first_seen,
                            create_time,
                            termination_capability,
                        ),
                    );
                },
                RunningTargetExecutableClassification::AmbiguousWorkspaceOwnership => {},
                RunningTargetExecutableClassification::Unrecognized => {
                    let keys = installed_bin_keys(
                        &executable,
                        project_attributions,
                        &self.cargo_install_bin_directory,
                    );
                    if keys.is_empty() {
                        continue;
                    }
                    // One OS process: feed its sample once, however many
                    // projects the installed binary is attributed to.
                    let first_seen = *self
                        .first_seen
                        .entry(process_identity.clone())
                        .or_insert(now);
                    let cpu_percent =
                        smoothed_cpu(&mut self.cpu_history, process_identity, cpu_percent);
                    for (key, member_dir) in keys {
                        push_instance(
                            &mut by_key,
                            key,
                            member_dir,
                            instance(
                                pid,
                                cpu_percent,
                                memory_bytes,
                                RunProfile::Installed,
                                first_seen,
                                create_time,
                                termination_capability.clone(),
                            ),
                        );
                    }
                },
            }
        }
        by_key
    }

    pub const fn snapshot(&self) -> &RunningTargets { &self.snapshot }

    /// Replace the snapshot directly, bypassing the process poll. Used by
    /// render/dispatch tests.
    #[cfg(test)]
    pub fn set_snapshot_for_test(&mut self, snapshot: RunningTargets) { self.snapshot = snapshot; }

    /// Drop `pids` from the current snapshot without waiting for the next
    /// poll. After killing an instance this collapses its row immediately
    /// so the pane reflects the change on the very next render (the next
    /// poll would do the same once the process exits). Also evicts the
    /// PIDs' first-seen entries so a reused number starts a fresh slot.
    pub fn drop_instance(&mut self, capability: &RunningTargetTerminationCapability) {
        self.snapshot.drop_identity(capability);
        self.first_seen.remove(capability.process_identity());
        self.cpu_history.remove(capability.process_identity());
    }
}

impl RunningTargets {
    /// Every tracked key with its owning member dir and instances (sorted
    /// by PID). Iteration order is arbitrary; callers sort the flattened
    /// rows themselves.
    pub fn iter_targets(
        &self,
    ) -> impl Iterator<Item = (&RunningKey, &AbsolutePath, &[RunningInstance])> {
        self.by_key
            .iter()
            .map(|(key, target)| (key, &target.member_dir, target.instances.as_slice()))
    }

    /// Whether any tracked instance is currently running — keys with no
    /// live instances are dropped each tick, so a non-empty map means
    /// live processes.
    pub fn has_instances(&self) -> bool { !self.by_key.is_empty() }

    /// Untracked descendants of tracked instances, for the Running list's
    /// outline. Unordered; the row builder nests them by parent link.
    pub fn child_processes(&self) -> &[ChildProcess] { &self.children }

    /// Remove every instance and child process whose PID is in `pids`,
    /// dropping any key left with no instances.
    fn drop_identity(&mut self, capability: &RunningTargetTerminationCapability) {
        for target in self.by_key.values_mut() {
            target
                .instances
                .retain(|instance| instance.termination_capability != *capability);
        }
        self.by_key.retain(|_, target| !target.instances.is_empty());
        self.children
            .retain(|child| child.termination_capability != *capability);
    }

    /// Build a snapshot directly from `(key, instances)` pairs, bypassing
    /// the process poll. Each key's member dir is its `target_dir`'s parent
    /// (the workspace root in the standard layout). Used by render/dispatch
    /// tests.
    #[cfg(test)]
    pub fn from_pairs(pairs: Vec<(RunningKey, Vec<RunningInstance>)>) -> Self {
        Self {
            by_key:   pairs
                .into_iter()
                .map(|(key, instances)| {
                    let member_dir = key
                        .target_dir
                        .as_path()
                        .parent()
                        .map_or_else(|| key.target_dir.clone(), AbsolutePath::from);
                    (
                        key,
                        RunningTarget {
                            member_dir,
                            instances,
                        },
                    )
                })
                .collect(),
            children: Vec::new(),
        }
    }

    /// The same snapshot with untracked descendant processes attached.
    #[cfg(test)]
    pub fn with_children(mut self, children: Vec<ChildProcess>) -> Self {
        self.children = children;
        self
    }
}

#[cfg(test)]
impl ChildProcess {
    /// A test child process with zeroed metrics, the PID doubling as the
    /// first-seen order and create time, like `RunningInstance::for_test`.
    pub fn for_test(pid: u32, name: &str, parent_pid: u32) -> Self {
        Self {
            pid,
            name: name.to_string(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            first_seen: test_instant_at(pid),
            create_time: u64::from(pid),
            termination_capability: RunningTargetTerminationCapability::for_test(
                pid,
                u64::from(pid),
            ),
            parent_pid,
        }
    }
}

#[cfg(test)]
impl RunningInstance {
    /// A test instance with the given PID and profile; zeroed metrics, the
    /// PID doubling as the first-seen order (lower PID = seen earlier).
    pub fn for_test(pid: u32, profile: RunProfile) -> Self {
        Self {
            pid,
            cpu_percent: 0.0,
            memory_bytes: 0,
            profile,
            first_seen: test_instant_at(pid),
            create_time: u64::from(pid),
            termination_capability: RunningTargetTerminationCapability::for_test(
                pid,
                u64::from(pid),
            ),
            placement: RunningProcessPlacement::TopLevel,
        }
    }

    /// The same test instance nested under `parent` in the outline.
    pub const fn with_parent(mut self, parent: u32) -> Self {
        self.placement = RunningProcessPlacement::ChildOf { parent_pid: parent };
        self
    }

    /// The same test instance with live-looking metrics.
    pub const fn with_metrics(mut self, cpu_percent: f32, memory_bytes: u64) -> Self {
        self.cpu_percent = cpu_percent;
        self.memory_bytes = memory_bytes;
        self
    }
}

/// A deterministic `Instant` for test fixtures: a shared base plus `order`
/// seconds, so fixtures can express relative first-seen order.
#[cfg(test)]
pub fn test_instant_at(order: u32) -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now) + Duration::from_secs(u64::from(order))
}

/// A freshly polled instance; its outline parent is resolved after the
/// snapshot rebuild, once the tracked PID set is known.
const fn instance(
    pid: u32,
    cpu: f32,
    memory: u64,
    profile: RunProfile,
    first_seen: Instant,
    create_time: u64,
    termination_capability: RunningTargetTerminationCapability,
) -> RunningInstance {
    RunningInstance {
        pid,
        cpu_percent: cpu,
        memory_bytes: memory,
        profile,
        first_seen,
        create_time,
        termination_capability,
        placement: RunningProcessPlacement::TopLevel,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NearestTrackedAncestorObservation {
    Found {
        process_identity:  ProcessIdentity,
        direct_parent_pid: u32,
    },
    NoTrackedAncestor,
    AncestryUnavailable,
}

fn nearest_tracked_ancestor_observation(
    ancestry_lookup: AncestryLookup,
    tracked: &HashSet<ProcessIdentity>,
) -> NearestTrackedAncestorObservation {
    let AncestryLookup::Observed(validated_ancestry) = ancestry_lookup else {
        return NearestTrackedAncestorObservation::AncestryUnavailable;
    };
    let nearest_tracked_identity = validated_ancestry
        .edges()
        .iter()
        .map(crate::process_observation::snapshot::ValidatedParentEdge::parent)
        .find(|parent_identity| tracked.contains(*parent_identity));
    match (nearest_tracked_identity, validated_ancestry.edges().first()) {
        (Some(process_identity), Some(direct_parent_edge)) => {
            NearestTrackedAncestorObservation::Found {
                process_identity:  process_identity.clone(),
                direct_parent_pid: direct_parent_edge.parent().pid(),
            }
        },
        (Some(_), None) => NearestTrackedAncestorObservation::AncestryUnavailable,
        (None, _) => match validated_ancestry.terminal() {
            AncestryTerminal::Root { .. } => NearestTrackedAncestorObservation::NoTrackedAncestor,
            AncestryTerminal::DepthCapped { .. }
            | AncestryTerminal::UnavailableParent { .. }
            | AncestryTerminal::UnavailableIdentifiedParent { .. }
            | AncestryTerminal::CreationOrderUnavailable { .. }
            | AncestryTerminal::ParentEvidenceUnavailable { .. }
            | AncestryTerminal::RejectedEdge { .. }
            | AncestryTerminal::SnapshotRecordUnavailable { .. } => {
                NearestTrackedAncestorObservation::AncestryUnavailable
            },
        },
    }
}

fn observed_nearest_tracked_ancestor(
    process_observation_snapshot: &ProcessObservationSnapshot,
    process_identity: &ProcessIdentity,
    tracked: &HashSet<ProcessIdentity>,
) -> NearestTrackedAncestorObservation {
    nearest_tracked_ancestor_observation(
        process_observation_snapshot
            .validated_ancestry(process_identity, ParentWalkDepth::new(ANCESTOR_WALK_CAP)),
        tracked,
    )
}

fn observed_running_process_placement(
    process_observation_snapshot: &ProcessObservationSnapshot,
    process_identity: &ProcessIdentity,
    tracked: &HashSet<ProcessIdentity>,
) -> RunningProcessPlacement {
    match observed_nearest_tracked_ancestor(process_observation_snapshot, process_identity, tracked)
    {
        NearestTrackedAncestorObservation::Found {
            direct_parent_pid, ..
        } => RunningProcessPlacement::ChildOf {
            parent_pid: direct_parent_pid,
        },
        NearestTrackedAncestorObservation::NoTrackedAncestor
        | NearestTrackedAncestorObservation::AncestryUnavailable => {
            RunningProcessPlacement::TopLevel
        },
    }
}

fn observed_running_process_placement_for_instance(
    process_observation_snapshot: &ProcessObservationSnapshot,
    process_identity: &ProcessIdentity,
    tracked: &HashSet<ProcessIdentity>,
    tracked_keys: &HashMap<ProcessIdentity, RunningKey>,
    key: &RunningKey,
) -> RunningProcessPlacement {
    match observed_nearest_tracked_ancestor(process_observation_snapshot, process_identity, tracked)
    {
        NearestTrackedAncestorObservation::Found {
            process_identity,
            direct_parent_pid,
        } if tracked_keys.get(&process_identity) == Some(key) => RunningProcessPlacement::ChildOf {
            parent_pid: direct_parent_pid,
        },
        NearestTrackedAncestorObservation::Found { .. }
        | NearestTrackedAncestorObservation::NoTrackedAncestor
        | NearestTrackedAncestorObservation::AncestryUnavailable => {
            RunningProcessPlacement::TopLevel
        },
    }
}

/// One process's link in the ancestor walk: its parent PID (if any) and its
/// start time (seconds since the epoch), used to reject hops into reused
/// PIDs — a parent never starts after its child.
#[derive(Clone, Copy)]
#[cfg(test)]
struct ParentLink {
    parent:     Option<u32>,
    start_time: u64,
}

/// Nearest ancestor of `pid` that is itself in `tracked`, walking parent
/// links through untracked intermediates (e.g. the `cargo` shim between
/// `cargo-mend`'s orchestrator and its wrapper re-invocations). `links`
/// resolves a PID to its parent link — a plain lookup, so tests fixture it
/// with a table instead of a live process list. `None` when the chain tops
/// out, leaves the table, breaks start-time ordering (PID reuse), or
/// exceeds the depth cap.
#[cfg(test)]
fn nearest_tracked_ancestor(
    links: &impl Fn(u32) -> Option<ParentLink>,
    pid: u32,
    tracked: &HashSet<u32>,
) -> Option<u32> {
    let mut current = pid;
    let mut child_start = links(pid)?.start_time;
    for _ in 0..ANCESTOR_WALK_CAP {
        let parent = links(current)?.parent?;
        if parent == current {
            return None;
        }
        let link = links(parent)?;
        if link.start_time > child_start {
            return None;
        }
        if tracked.contains(&parent) {
            return Some(parent);
        }
        current = parent;
        child_start = link.start_time;
    }
    None
}

/// The outline parent of `pid`: its direct OS parent, provided `pid`'s
/// ancestor chain reaches a tracked instance — the condition for the row
/// to nest at all. Every ancestor between `pid` and the tracked root is on
/// that same chain, so the direct parent is always itself shown.
#[cfg(test)]
fn running_process_placement(
    links: &impl Fn(u32) -> Option<ParentLink>,
    pid: u32,
    tracked: &HashSet<u32>,
) -> RunningProcessPlacement {
    if nearest_tracked_ancestor(links, pid, tracked).is_none() {
        return RunningProcessPlacement::TopLevel;
    }
    links(pid).and_then(|parent_link| parent_link.parent).into()
}

/// The outline parent for a tracked target instance. Only nest it under
/// another tracked instance when the nearest tracked ancestor is the same
/// cargo target. This keeps examples launched by the installed
/// `cargo-port` process visible as top-level app rows instead of hiding
/// them inside the collapsed installed-cargo group.
#[cfg(test)]
fn running_process_placement_for_instance(
    links: &impl Fn(u32) -> Option<ParentLink>,
    pid: u32,
    tracked: &HashSet<u32>,
    tracked_keys: &HashMap<u32, RunningKey>,
    key: &RunningKey,
) -> RunningProcessPlacement {
    let Some(ancestor) = nearest_tracked_ancestor(links, pid, tracked) else {
        return RunningProcessPlacement::TopLevel;
    };
    if tracked_keys.get(&ancestor) != Some(key) {
        return RunningProcessPlacement::TopLevel;
    }
    links(pid).and_then(|parent_link| parent_link.parent).into()
}

/// Fold this poll's CPU sample into `pid`'s [`RollingMean`] window and
/// return the mean — the value the Running list's CPU column shows.
fn smoothed_cpu(
    history: &mut HashMap<ProcessIdentity, RollingMean>,
    process_identity: &ProcessIdentity,
    sample: f32,
) -> f32 {
    history
        .entry(process_identity.clone())
        .or_default()
        .push(sample)
}

/// Append one process's metrics under `key`, recording the owning member
/// dir the first time the key is seen.
fn push_instance(
    by_key: &mut HashMap<RunningKey, RunningTarget>,
    key: RunningKey,
    member_dir: AbsolutePath,
    inst: RunningInstance,
) {
    by_key
        .entry(key)
        .or_insert_with(|| RunningTarget {
            member_dir,
            instances: Vec::new(),
        })
        .instances
        .push(inst);
}

/// Classify an executable under a project's matching target directory as a
/// binary, example, or bench. One workspace's exact `(RunTargetKind, name)`
/// owner takes precedence over undeclared fallbacks. Exact evidence from
/// multiple workspaces is ambiguous. If no workspace declares the target,
/// the first matching attribution supplies the deterministic fallback owner.
/// Installed binaries are handled by [`installed_bin_keys`].
fn classify_exe(
    exe: &Path,
    project_attributions: &[RunningTargetProjectAttribution<'_>],
) -> RunningTargetExecutableClassification {
    let mut exact_owner_match = CrossWorkspaceExactOwnerMatch::None;
    let mut undeclared_fallback = UndeclaredTargetFallback::Missing;
    for attribution in project_attributions {
        if let Ok(rest) = exe.strip_prefix(attribution.executable_match_target_directory.as_path())
            && let Some((run_target_kind, name, profile)) =
                classify_tail(rest, attribution.bench_names)
        {
            let key = RunningKey {
                target_dir: attribution.executable_match_target_directory.clone(),
                run_target_kind,
                name,
            };
            let owner_attribution = attribution.owner_attribution(run_target_kind, &key.name);
            let executable_attribution = AttributedRunningTargetExecutable {
                key,
                profile,
                owner_root: owner_attribution.owner_root().clone(),
            };
            match owner_attribution {
                RunningTargetOwnerAttribution::DeclaredMember(_)
                | RunningTargetOwnerAttribution::AmbiguousTarget(_) => {
                    exact_owner_match.include(executable_attribution);
                },
                RunningTargetOwnerAttribution::UndeclaredTarget(_) => {
                    undeclared_fallback.include_first(executable_attribution);
                },
            }
        }
    }
    match exact_owner_match {
        CrossWorkspaceExactOwnerMatch::None => match undeclared_fallback {
            UndeclaredTargetFallback::Missing => {
                RunningTargetExecutableClassification::Unrecognized
            },
            UndeclaredTargetFallback::First(attribution) => {
                RunningTargetExecutableClassification::Attributed(attribution)
            },
        },
        CrossWorkspaceExactOwnerMatch::One(attribution) => {
            RunningTargetExecutableClassification::Attributed(attribution)
        },
        CrossWorkspaceExactOwnerMatch::Multiple => {
            RunningTargetExecutableClassification::AmbiguousWorkspaceOwnership
        },
    }
}

/// Keys for a `cargo install`ed binary: an executable living directly in a
/// resolved [`CargoInstallBinDirectory`] whose file name is a declared binary
/// target name. A binary name can be declared by more than one project (e.g.
/// the primary repo and its worktrees all build `cargo-port`), and its installed
/// source cannot be identified, so the running process is attributed to every
/// matching project. The renderer then matches whichever project is selected.
fn installed_bin_keys(
    exe: &Path,
    project_attributions: &[RunningTargetProjectAttribution<'_>],
    cargo_install_bin_directory: &CargoInstallBinDirectory,
) -> Vec<(RunningKey, AbsolutePath)> {
    let CargoInstallBinDirectory::Resolved(bin_directory) = cargo_install_bin_directory else {
        return Vec::new();
    };
    if exe.parent() != Some(bin_directory.as_path()) {
        return Vec::new();
    }
    let Some(stem) = exe.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    project_attributions
        .iter()
        .filter(|attribution| attribution.bin_names.contains(stem))
        .map(|attribution| {
            (
                RunningKey {
                    target_dir:      attribution.executable_match_target_directory.clone(),
                    run_target_kind: RunTargetKind::Binary,
                    name:            stem.to_string(),
                },
                attribution
                    .owner_attribution(RunTargetKind::Binary, stem)
                    .owner_root()
                    .clone(),
            )
        })
        .collect()
}

fn classify_tail(
    rest: &Path,
    bench_names: &HashSet<String>,
) -> Option<(RunTargetKind, String, RunProfile)> {
    let segments: Vec<&str> = rest
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    match segments.as_slice() {
        [profile, name] => {
            parse_profile(profile).map(|profile| (RunTargetKind::Binary, (*name).into(), profile))
        },
        [profile, "examples", name] => {
            parse_profile(profile).map(|profile| (RunTargetKind::Example, (*name).into(), profile))
        },
        [profile, "deps", basename] => {
            let profile = parse_profile(profile)?;
            parse_bench_basename(basename, bench_names)
                .map(|name| (RunTargetKind::Bench, name, profile))
        },
        _ => None,
    }
}

const fn parse_profile(s: &str) -> Option<RunProfile> {
    match s.as_bytes() {
        b"debug" => Some(RunProfile::Debug),
        b"release" => Some(RunProfile::Release),
        _ => None,
    }
}

/// Resolve the cargo install bin directory, honoring `CARGO_INSTALL_ROOT`
/// and `CARGO_HOME`, falling back to `~/.cargo/bin`. Canonicalized so it
/// compares equal to process exe paths reported by the OS.
fn cargo_install_bin_directory() -> CargoInstallBinDirectory {
    env::var_os("CARGO_INSTALL_ROOT")
        .map(PathBuf::from)
        .or_else(|| env::var_os("CARGO_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(DOT_CARGO_DIR)))
        .map(|base| {
            let bin_directory = base.join(CARGO_BIN_DIR);
            let canonical_bin_directory = bin_directory.canonicalize().unwrap_or(bin_directory);
            AbsolutePath::from(canonical_bin_directory)
        })
        .into()
}

/// Parse a `target/<profile>/deps/<basename>` entry as a bench. The basename
/// is `<name>-<hash>` where `<hash>` is [`MIN_HEX_HASH_LEN`]+ lowercase hex chars. The longest
/// `<name>` that is a declared bench wins (so `my-bench-...` with both `my`
/// and `my-bench` declared resolves to `my-bench`).
fn parse_bench_basename(basename: &str, bench_names: &HashSet<String>) -> Option<String> {
    let mut best: Option<String> = None;
    for (i, ch) in basename.char_indices() {
        if ch != '-' {
            continue;
        }
        let name = &basename[..i];
        let hash = &basename[i + 1..];
        if !is_hex_hash(hash) {
            continue;
        }
        if !bench_names.contains(name) {
            continue;
        }
        if best.as_ref().is_none_or(|b| name.len() > b.len()) {
            best = Some(name.to_string());
        }
    }
    best
}

fn is_hex_hash(s: &str) -> bool {
    s.len() >= MIN_HEX_HASH_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use tui_pane::CPU_SMOOTHING_WINDOW_POLLS;

    use super::*;
    use crate::process_observation::snapshot::ParentEdgeRejection;
    use crate::process_observation::snapshot::StrongParentEdge;
    use crate::process_observation::snapshot::ValidatedParentEdge;

    /// The shared empty exact-owner evidence for tests that exercise project
    /// fallback attribution.
    fn no_exact_target_owner_evidence()
    -> HashMap<(RunTargetKind, String), ExactRunningTargetOwnerEvidence> {
        HashMap::new()
    }

    fn project_attribution<'a>(
        executable_match_target_directory: &'a AbsolutePath,
        bench_names: &'a HashSet<String>,
        bin_names: &'a HashSet<String>,
        exact_target_owner_evidence: &'a HashMap<
            (RunTargetKind, String),
            ExactRunningTargetOwnerEvidence,
        >,
    ) -> RunningTargetProjectAttribution<'a> {
        RunningTargetProjectAttribution {
            executable_match_target_directory,
            fallback_owner_root: executable_match_target_directory,
            bench_names,
            bin_names,
            exact_target_owner_evidence,
        }
    }

    /// A candidate executable path, made absolute on the host platform so it
    /// shares the same drive prefix as the `AbsolutePath` target dir it is
    /// matched against. Identity on Unix.
    fn exe_path(path: &str) -> PathBuf { crate::project::normalize_test_path(Path::new(path)) }

    fn names(names: &[&str]) -> HashSet<String> { names.iter().map(|s| (*s).to_string()).collect() }

    fn process_identity(pid: u32) -> ProcessIdentity {
        ProcessIdentity::for_test(pid, u64::from(pid))
    }

    #[test]
    fn observed_ancestry_reports_nearest_tracked_ancestor() {
        let parent_identity = process_identity(10);
        let child_identity = process_identity(20);
        let ancestry_lookup = AncestryLookup::observed_for_test(
            vec![ValidatedParentEdge::for_test(
                parent_identity.clone(),
                child_identity,
            )],
            AncestryTerminal::Root {
                root: parent_identity.clone(),
            },
        );

        assert_eq!(
            nearest_tracked_ancestor_observation(
                ancestry_lookup,
                &HashSet::from([parent_identity.clone()]),
            ),
            NearestTrackedAncestorObservation::Found {
                process_identity:  parent_identity,
                direct_parent_pid: 10,
            }
        );
    }

    #[test]
    fn complete_observed_ancestry_reports_no_tracked_ancestor() {
        let parent_identity = process_identity(30);
        let child_identity = process_identity(40);
        let ancestry_lookup = AncestryLookup::observed_for_test(
            vec![ValidatedParentEdge::for_test(
                parent_identity.clone(),
                child_identity.clone(),
            )],
            AncestryTerminal::Root {
                root: parent_identity,
            },
        );

        assert_eq!(
            nearest_tracked_ancestor_observation(ancestry_lookup, &HashSet::from([child_identity]),),
            NearestTrackedAncestorObservation::NoTrackedAncestor
        );
    }

    #[test]
    fn unavailable_and_rejected_ancestry_report_unavailable() {
        let parent_identity = process_identity(50);
        let child_identity = process_identity(60);
        let tracked = HashSet::from([child_identity.clone()]);
        let missing_identity_lookup = AncestryLookup::IdentityNotInSnapshot(child_identity.clone());
        let rejected_lookup = AncestryLookup::observed_for_test(
            Vec::new(),
            AncestryTerminal::RejectedEdge {
                edge:      StrongParentEdge::for_test(parent_identity, child_identity),
                rejection: ParentEdgeRejection::CreatedAfterChild,
            },
        );

        assert_eq!(
            nearest_tracked_ancestor_observation(missing_identity_lookup, &tracked),
            NearestTrackedAncestorObservation::AncestryUnavailable
        );
        assert_eq!(
            nearest_tracked_ancestor_observation(rejected_lookup, &tracked),
            NearestTrackedAncestorObservation::AncestryUnavailable
        );
    }

    fn attributed_executable(
        classification: RunningTargetExecutableClassification,
    ) -> Result<AttributedRunningTargetExecutable, &'static str> {
        match classification {
            RunningTargetExecutableClassification::Attributed(attribution) => Ok(attribution),
            RunningTargetExecutableClassification::AmbiguousWorkspaceOwnership => {
                Err("workspace ownership should not be ambiguous")
            },
            RunningTargetExecutableClassification::Unrecognized => {
                Err("executable should be recognized")
            },
        }
    }

    fn running_key(target_dir: &str, run_target_kind: RunTargetKind, name: &str) -> RunningKey {
        RunningKey {
            target_dir: AbsolutePath::from(PathBuf::from(target_dir)),
            run_target_kind,
            name: name.to_string(),
        }
    }

    #[test]
    fn debug_bin() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_exact_target_owner_evidence());
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/foo");
        let attributed =
            attributed_executable(classify_exe(&exe, std::slice::from_ref(&attribution)))
                .expect("matches");
        assert!(matches!(
            attributed.key.run_target_kind,
            RunTargetKind::Binary
        ));
        assert_eq!(attributed.key.name, "foo");
        assert_eq!(attributed.key.target_dir, dir);
        assert_eq!(attributed.profile, RunProfile::Debug);
    }

    #[test]
    fn release_example() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_exact_target_owner_evidence());
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/release/examples/bar");
        let attributed =
            attributed_executable(classify_exe(&exe, std::slice::from_ref(&attribution)))
                .expect("matches");
        assert!(matches!(
            attributed.key.run_target_kind,
            RunTargetKind::Example
        ));
        assert_eq!(attributed.key.name, "bar");
        assert_eq!(attributed.profile, RunProfile::Release);
    }

    #[test]
    fn bench_with_known_name() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&["baz"]),
            names(&[]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-0123456789abcdef");
        let attributed =
            attributed_executable(classify_exe(&exe, std::slice::from_ref(&attribution)))
                .expect("matches");
        assert!(matches!(
            attributed.key.run_target_kind,
            RunTargetKind::Bench
        ));
        assert_eq!(attributed.key.name, "baz");
    }

    #[test]
    fn bench_rejects_short_hash() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&["baz"]),
            names(&[]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-shorthash");
        assert!(matches!(
            classify_exe(&exe, std::slice::from_ref(&attribution)),
            RunningTargetExecutableClassification::Unrecognized
        ));
    }

    #[test]
    fn deps_entry_not_in_bench_set_is_unrecognized() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&["baz"]),
            names(&[]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/other-0123456789abcdef");
        assert!(matches!(
            classify_exe(&exe, std::slice::from_ref(&attribution)),
            RunningTargetExecutableClassification::Unrecognized
        ));
    }

    #[test]
    fn longest_bench_name_wins() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&["my", "my-bench"]),
            names(&[]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/my-bench-0123456789abcdef");
        let attributed =
            attributed_executable(classify_exe(&exe, std::slice::from_ref(&attribution)))
                .expect("matches");
        assert!(matches!(
            attributed.key.run_target_kind,
            RunTargetKind::Bench
        ));
        assert_eq!(attributed.key.name, "my-bench");
    }

    #[test]
    fn outside_target_dir_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_exact_target_owner_evidence());
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/usr/bin/ls");
        assert!(matches!(
            classify_exe(&exe, std::slice::from_ref(&attribution)),
            RunningTargetExecutableClassification::Unrecognized
        ));
    }

    #[test]
    fn build_artifact_under_target_ignored() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_exact_target_owner_evidence());
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/build/foo-1234567890abcdef/build-script-build");
        assert!(matches!(
            classify_exe(&exe, std::slice::from_ref(&attribution)),
            RunningTargetExecutableClassification::Unrecognized
        ));
    }

    #[test]
    fn installed_bin_in_cargo_dir_matches_as_cargo_profile() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&[]),
            names(&["cargo-port"]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/cargo-port");
        let cargo_install_bin_directory =
            CargoInstallBinDirectory::Resolved(AbsolutePath::from(bin_dir));
        let keys = installed_bin_keys(
            &exe,
            std::slice::from_ref(&attribution),
            &cargo_install_bin_directory,
        );
        assert_eq!(keys.len(), 1);
        let (key, member_dir) = &keys[0];
        assert!(matches!(key.run_target_kind, RunTargetKind::Binary));
        assert_eq!(key.name, "cargo-port");
        assert_eq!(key.target_dir, dir);
        // No declared owner: attribution uses the project's fallback owner
        // root (the fixture points it at the target directory).
        assert_eq!(*member_dir, dir);
    }

    #[test]
    fn installed_bin_attributed_to_every_project_declaring_it() {
        let primary = AbsolutePath::from(PathBuf::from("/tmp/main/target"));
        let worktree = AbsolutePath::from(PathBuf::from("/tmp/wt/target"));
        let (benches, bins, members) = (
            names(&[]),
            names(&["cargo-port"]),
            no_exact_target_owner_evidence(),
        );
        let project_attributions = [
            project_attribution(&primary, &benches, &bins, &members),
            project_attribution(&worktree, &benches, &bins, &members),
        ];
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/cargo-port");
        let cargo_install_bin_directory =
            CargoInstallBinDirectory::Resolved(AbsolutePath::from(bin_dir));
        let dirs: HashSet<AbsolutePath> =
            installed_bin_keys(&exe, &project_attributions, &cargo_install_bin_directory)
                .into_iter()
                .map(|(key, _)| key.target_dir)
                .collect();
        assert_eq!(dirs, HashSet::from([primary, worktree]));
    }

    #[test]
    fn classified_exe_resolves_its_member_dir() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let member = AbsolutePath::from(PathBuf::from("/tmp/ws/crates/foo"));
        let (benches, bins) = (names(&[]), names(&[]));
        let members = HashMap::from([(
            (RunTargetKind::Binary, "foo".to_string()),
            ExactRunningTargetOwnerEvidence::Unique(member.clone()),
        )]);
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/foo");
        let attributed =
            attributed_executable(classify_exe(&exe, std::slice::from_ref(&attribution)))
                .expect("matches");
        assert_eq!(attributed.owner_root, member);
    }

    #[test]
    fn unknown_target_uses_the_project_fallback_owner_root() {
        // A stale artifact of a renamed target: nothing in the declared owner
        // map, so path attribution uses the project fallback owner root.
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_exact_target_owner_evidence());
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/stale");
        let attributed =
            attributed_executable(classify_exe(&exe, std::slice::from_ref(&attribution)))
                .expect("matches");
        assert_eq!(attributed.owner_root, dir);
    }

    #[test]
    fn shared_target_directory_prefers_later_declared_target_owner() {
        let target_directory = AbsolutePath::from(PathBuf::from("/tmp/shared/target"));
        let first_fallback = AbsolutePath::from(PathBuf::from("/tmp/first-checkout"));
        let later_fallback = AbsolutePath::from(PathBuf::from("/tmp/later-checkout"));
        let later_member = AbsolutePath::from(PathBuf::from("/tmp/later-checkout/crates/owned"));
        let (benches, bins) = (names(&[]), names(&[]));
        let first_target_owners = no_exact_target_owner_evidence();
        let later_target_owners = HashMap::from([(
            (RunTargetKind::Binary, "owned".to_string()),
            ExactRunningTargetOwnerEvidence::Unique(later_member.clone()),
        )]);
        let project_attributions = [
            RunningTargetProjectAttribution {
                executable_match_target_directory: &target_directory,
                fallback_owner_root:               &first_fallback,
                bench_names:                       &benches,
                bin_names:                         &bins,
                exact_target_owner_evidence:       &first_target_owners,
            },
            RunningTargetProjectAttribution {
                executable_match_target_directory: &target_directory,
                fallback_owner_root:               &later_fallback,
                bench_names:                       &benches,
                bin_names:                         &bins,
                exact_target_owner_evidence:       &later_target_owners,
            },
        ];
        let exe = exe_path("/tmp/shared/target/debug/owned");

        let attributed =
            attributed_executable(classify_exe(&exe, &project_attributions)).expect("matches");

        assert_eq!(attributed.owner_root, later_member);
    }

    #[test]
    fn shared_target_directory_exact_owners_are_ambiguous_in_both_orders() {
        let target_directory = AbsolutePath::from(PathBuf::from("/tmp/shared/target"));
        let first_fallback = AbsolutePath::from(PathBuf::from("/tmp/first-checkout"));
        let second_fallback = AbsolutePath::from(PathBuf::from("/tmp/second-checkout"));
        let first_member = AbsolutePath::from(PathBuf::from("/tmp/first-checkout/crates/shared"));
        let second_member = AbsolutePath::from(PathBuf::from("/tmp/second-checkout/crates/shared"));
        let (benches, bins) = (names(&[]), names(&[]));
        let first_target_owners = HashMap::from([(
            (RunTargetKind::Binary, "shared".to_string()),
            ExactRunningTargetOwnerEvidence::Unique(first_member),
        )]);
        let second_target_owners = HashMap::from([(
            (RunTargetKind::Binary, "shared".to_string()),
            ExactRunningTargetOwnerEvidence::Unique(second_member),
        )]);
        let first_attribution = RunningTargetProjectAttribution {
            executable_match_target_directory: &target_directory,
            fallback_owner_root:               &first_fallback,
            bench_names:                       &benches,
            bin_names:                         &bins,
            exact_target_owner_evidence:       &first_target_owners,
        };
        let second_attribution = RunningTargetProjectAttribution {
            executable_match_target_directory: &target_directory,
            fallback_owner_root:               &second_fallback,
            bench_names:                       &benches,
            bin_names:                         &bins,
            exact_target_owner_evidence:       &second_target_owners,
        };
        let exe = exe_path("/tmp/shared/target/debug/shared");

        let mut project_attributions = [first_attribution, second_attribution];
        let forward = classify_exe(&exe, &project_attributions);
        project_attributions.reverse();
        let reverse = classify_exe(&exe, &project_attributions);

        assert_eq!(
            forward,
            RunningTargetExecutableClassification::AmbiguousWorkspaceOwnership
        );
        assert_eq!(
            reverse,
            RunningTargetExecutableClassification::AmbiguousWorkspaceOwnership
        );
    }

    #[test]
    fn shared_target_directory_prefers_ambiguous_exact_evidence_over_generic_fallback() {
        let target_directory = AbsolutePath::from(PathBuf::from("/tmp/shared/target"));
        let first_fallback = AbsolutePath::from(PathBuf::from("/tmp/first-checkout"));
        let later_fallback = AbsolutePath::from(PathBuf::from("/tmp/later-checkout"));
        let (benches, bins) = (names(&[]), names(&[]));
        let first_target_owners = no_exact_target_owner_evidence();
        let later_target_owners = HashMap::from([(
            (RunTargetKind::Binary, "shared".to_string()),
            ExactRunningTargetOwnerEvidence::Ambiguous,
        )]);
        let project_attributions = [
            RunningTargetProjectAttribution {
                executable_match_target_directory: &target_directory,
                fallback_owner_root:               &first_fallback,
                bench_names:                       &benches,
                bin_names:                         &bins,
                exact_target_owner_evidence:       &first_target_owners,
            },
            RunningTargetProjectAttribution {
                executable_match_target_directory: &target_directory,
                fallback_owner_root:               &later_fallback,
                bench_names:                       &benches,
                bin_names:                         &bins,
                exact_target_owner_evidence:       &later_target_owners,
            },
        ];
        let exe = exe_path("/tmp/shared/target/debug/shared");

        let attributed =
            attributed_executable(classify_exe(&exe, &project_attributions)).expect("matches");

        assert_eq!(attributed.owner_root, later_fallback);
    }

    #[test]
    fn shared_target_directory_stale_artifact_uses_first_project_fallback() {
        let target_directory = AbsolutePath::from(PathBuf::from("/tmp/shared/target"));
        let first_fallback = AbsolutePath::from(PathBuf::from("/tmp/first-checkout"));
        let later_fallback = AbsolutePath::from(PathBuf::from("/tmp/later-checkout"));
        let (benches, bins, target_owners) =
            (names(&[]), names(&[]), no_exact_target_owner_evidence());
        let project_attributions = [
            RunningTargetProjectAttribution {
                executable_match_target_directory: &target_directory,
                fallback_owner_root:               &first_fallback,
                bench_names:                       &benches,
                bin_names:                         &bins,
                exact_target_owner_evidence:       &target_owners,
            },
            RunningTargetProjectAttribution {
                executable_match_target_directory: &target_directory,
                fallback_owner_root:               &later_fallback,
                bench_names:                       &benches,
                bin_names:                         &bins,
                exact_target_owner_evidence:       &target_owners,
            },
        ];
        let exe = exe_path("/tmp/shared/target/debug/stale");

        let attributed =
            attributed_executable(classify_exe(&exe, &project_attributions)).expect("matches");

        assert_eq!(attributed.owner_root, first_fallback);
    }

    #[test]
    fn drop_instance_evicts_the_first_seen_entry() {
        let mut running_targets_state = RunningTargetsState::new();
        let first_identity = process_identity(42);
        let second_identity = process_identity(43);
        let termination_capability = RunningTargetTerminationCapability::for_test(42, 42);
        running_targets_state
            .first_seen
            .insert(first_identity.clone(), test_instant_at(0));
        running_targets_state
            .first_seen
            .insert(second_identity.clone(), test_instant_at(1));
        running_targets_state.drop_instance(&termination_capability);
        assert!(
            !running_targets_state
                .first_seen
                .contains_key(&first_identity)
        );
        assert!(
            running_targets_state
                .first_seen
                .contains_key(&second_identity)
        );
    }

    /// A fixture process table for the ancestor walk: `(pid, parent,
    /// start_time)` rows.
    fn links_from(table: Vec<(u32, Option<u32>, u64)>) -> impl Fn(u32) -> Option<ParentLink> {
        move |pid| {
            table
                .iter()
                .find(|(candidate, _, _)| *candidate == pid)
                .map(|(_, parent, start_time)| ParentLink {
                    parent:     *parent,
                    start_time: *start_time,
                })
        }
    }

    #[test]
    fn ancestor_walk_finds_a_direct_parent() {
        let links = links_from(vec![(10, Some(1), 100), (20, Some(10), 200)]);
        let tracked = HashSet::from([10, 20]);
        assert_eq!(nearest_tracked_ancestor(&links, 20, &tracked), Some(10));
    }

    #[test]
    fn ancestor_walk_crosses_untracked_intermediates() {
        // The cargo-mend chain: orchestrator (10) → cargo shim (15,
        // untracked) → wrapper (20).
        let links = links_from(vec![
            (10, Some(1), 100),
            (15, Some(10), 150),
            (20, Some(15), 200),
        ]);
        let tracked = HashSet::from([10, 20]);
        assert_eq!(nearest_tracked_ancestor(&links, 20, &tracked), Some(10));
    }

    #[test]
    fn ancestor_walk_rejects_a_reused_pid_by_start_time() {
        // The "parent" started after its child: the original parent
        // exited and the OS reassigned its number.
        let links = links_from(vec![(10, Some(1), 900), (20, Some(10), 200)]);
        let tracked = HashSet::from([10, 20]);
        assert_eq!(nearest_tracked_ancestor(&links, 20, &tracked), None);
    }

    #[test]
    fn ancestor_walk_stops_when_the_chain_leaves_the_table() {
        let links = links_from(vec![(20, Some(15), 200)]);
        let tracked = HashSet::from([10, 20]);
        assert_eq!(nearest_tracked_ancestor(&links, 20, &tracked), None);
    }

    #[test]
    fn ancestor_walk_stops_on_a_self_parented_process() {
        // The kernel idle process is its own parent; the walk must not
        // spin on it.
        let links = links_from(vec![(0, Some(0), 0), (20, Some(0), 200)]);
        let tracked = HashSet::from([20]);
        assert_eq!(nearest_tracked_ancestor(&links, 20, &tracked), None);
    }

    #[test]
    fn ancestor_walk_is_depth_capped() {
        // A chain of untracked intermediates longer than the cap between
        // the instance and its tracked ancestor.
        let depth = u32::try_from(ANCESTOR_WALK_CAP).expect("cap fits u32") + 2;
        let mut table: Vec<(u32, Option<u32>, u64)> =
            (1..=depth).map(|pid| (pid, Some(pid - 1), 0)).collect();
        table.push((0, None, 0));
        let links = links_from(table);
        let tracked = HashSet::from([0, depth]);
        assert_eq!(nearest_tracked_ancestor(&links, depth, &tracked), None);
    }

    #[test]
    fn process_placement_uses_the_direct_parent_on_a_tracked_chain() {
        // The wrapper (30) reaches the tracked orchestrator (10) through
        // the untracked cargo shim (20); its outline parent is the shim —
        // the shim itself joins the outline as a descendant.
        let links = links_from(vec![
            (10, Some(1), 100),
            (20, Some(10), 150),
            (30, Some(20), 200),
        ]);
        let tracked = HashSet::from([10, 30]);
        assert_eq!(
            running_process_placement(&links, 30, &tracked),
            RunningProcessPlacement::ChildOf { parent_pid: 20 }
        );
        assert_eq!(
            running_process_placement(&links, 20, &tracked),
            RunningProcessPlacement::ChildOf { parent_pid: 10 }
        );
    }

    #[test]
    fn independently_started_process_has_top_level_placement() {
        // The chain tops out at the shell (1) without crossing another
        // tracked PID: the row renders top-level.
        let links = links_from(vec![(1, None, 0), (10, Some(1), 100)]);
        let tracked = HashSet::from([10]);
        assert_eq!(
            running_process_placement(&links, 10, &tracked),
            RunningProcessPlacement::TopLevel
        );
    }

    #[test]
    fn tracked_instance_does_not_nest_under_unrelated_tracked_parent() {
        // cargo-port (10) launched an unrelated workspace example (20).
        // Both are tracked targets, but the example should stay visible as
        // a top-level app row rather than hide under the installed-cargo
        // group.
        let links = links_from(vec![(10, Some(1), 100), (20, Some(10), 200)]);
        let parent_key = running_key(
            "/tmp/cargo-port/target",
            RunTargetKind::Binary,
            "cargo-port",
        );
        let child_key = running_key("/tmp/hana/target", RunTargetKind::Example, "units");
        let tracked_keys = HashMap::from([(10, parent_key), (20, child_key.clone())]);
        let tracked = HashSet::from([10, 20]);

        assert_eq!(
            running_process_placement_for_instance(&links, 20, &tracked, &tracked_keys, &child_key,),
            RunningProcessPlacement::TopLevel,
        );
    }

    #[test]
    fn tracked_instance_keeps_same_target_outline_parent() {
        // Same target through an untracked cargo shim: preserve the
        // wrapper outline so repeated invocations still group together.
        let links = links_from(vec![
            (10, Some(1), 100),
            (15, Some(10), 150),
            (20, Some(15), 200),
        ]);
        let key = running_key(
            "/tmp/cargo-mend/target",
            RunTargetKind::Binary,
            "cargo-mend",
        );
        let tracked_keys = HashMap::from([(10, key.clone()), (20, key.clone())]);
        let tracked = HashSet::from([10, 20]);

        assert_eq!(
            running_process_placement_for_instance(&links, 20, &tracked, &tracked_keys, &key,),
            RunningProcessPlacement::ChildOf { parent_pid: 15 },
        );
    }

    #[test]
    fn smoothed_cpu_averages_the_window() {
        let mut history = HashMap::new();
        let process_identity = process_identity(7);
        // First sample is the mean of one — no zero dilution.
        assert!((smoothed_cpu(&mut history, &process_identity, 20.0) - 20.0).abs() < f32::EPSILON);
        // 20 and 10 average to 15.
        assert!((smoothed_cpu(&mut history, &process_identity, 10.0) - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn smoothed_cpu_window_drops_the_oldest_sample() {
        let mut history = HashMap::new();
        let process_identity = process_identity(7);
        // Fill the window with zeros, then push spikes: once the window
        // holds only the spikes, the zeros no longer drag the mean down.
        for _ in 0..CPU_SMOOTHING_WINDOW_POLLS {
            smoothed_cpu(&mut history, &process_identity, 0.0);
        }
        let mut mean = 0.0;
        for _ in 0..CPU_SMOOTHING_WINDOW_POLLS {
            mean = smoothed_cpu(&mut history, &process_identity, 50.0);
        }
        assert!((mean - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn smoothed_cpu_tracks_pids_independently() {
        let mut history = HashMap::new();
        let first_identity = process_identity(7);
        let second_identity = process_identity(8);
        smoothed_cpu(&mut history, &first_identity, 40.0);
        // A different PID's first sample is unaffected by PID 7's window.
        assert!((smoothed_cpu(&mut history, &second_identity, 10.0) - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn drop_instance_evicts_the_cpu_history_entry() {
        let mut running_targets_state = RunningTargetsState::new();
        let first_identity = process_identity(42);
        let second_identity = process_identity(43);
        smoothed_cpu(
            &mut running_targets_state.cpu_history,
            &first_identity,
            10.0,
        );
        smoothed_cpu(
            &mut running_targets_state.cpu_history,
            &second_identity,
            10.0,
        );
        let termination_capability = RunningTargetTerminationCapability::for_test(42, 42);
        running_targets_state.drop_instance(&termination_capability);
        assert!(
            !running_targets_state
                .cpu_history
                .contains_key(&first_identity)
        );
        assert!(
            running_targets_state
                .cpu_history
                .contains_key(&second_identity)
        );
    }

    #[test]
    fn installed_bin_not_in_bin_set_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&[]),
            names(&["cargo-port"]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/ripgrep");
        let cargo_install_bin_directory =
            CargoInstallBinDirectory::Resolved(AbsolutePath::from(bin_dir));
        assert!(
            installed_bin_keys(
                &exe,
                std::slice::from_ref(&attribution),
                &cargo_install_bin_directory,
            )
            .is_empty()
        );
    }

    #[test]
    fn bin_outside_cargo_dir_does_not_match_as_installed() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (
            names(&[]),
            names(&["cargo-port"]),
            no_exact_target_owner_evidence(),
        );
        let attribution = project_attribution(&dir, &benches, &bins, &members);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/usr/local/bin/cargo-port");
        let cargo_install_bin_directory =
            CargoInstallBinDirectory::Resolved(AbsolutePath::from(bin_dir));
        assert!(
            installed_bin_keys(
                &exe,
                std::slice::from_ref(&attribution),
                &cargo_install_bin_directory,
            )
            .is_empty()
        );
    }
}
