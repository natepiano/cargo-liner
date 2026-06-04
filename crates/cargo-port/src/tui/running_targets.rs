//! Detect which cargo bin/example/bench targets are currently running.
//!
//! Each tick refreshes the system process list (exe paths only) and walks
//! every process whose exe lives under a known workspace `target_directory`.
//! The path tail is parsed against cargo's filesystem layout to classify
//! the exe as a bin / example / bench of that workspace.

use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::Signal;
use sysinfo::System;
use sysinfo::UpdateKind;
use tui_pane::RollingMean;

use super::panes::RunTargetKind;
use crate::project::AbsolutePath;

/// Ceiling on the ancestor walk, against parent-link cycles from PID reuse.
/// Real process trees are nowhere near this deep.
const ANCESTOR_WALK_CAP: usize = 32;

pub(crate) struct RunningTargetsPoller {
    system:          System,
    last_poll:       Option<Instant>,
    poll_interval:   Duration,
    snapshot:        RunningTargets,
    /// Canonical cargo install bin directory (`~/.cargo/bin` by default).
    /// Exes living directly here are matched as installed binaries,
    /// surfaced as the `cargo` profile. `None` when it can't be resolved.
    install_bin_dir: Option<AbsolutePath>,
    /// When each tracked PID was first observed, surviving the per-poll
    /// snapshot rebuild. Drives the Running list's newest-at-bottom
    /// ordering: insert on first sight, retain only live PIDs after each
    /// poll, and evict on [`Self::drop_instances`].
    first_seen:      HashMap<u32, Instant>,
    /// Each tracked PID's [`RollingMean`] window over CPU samples;
    /// instances carry the mean. Same lifecycle as `first_seen`: fed during
    /// the poll loop, retained against live PIDs, evicted on
    /// [`Self::drop_instances`].
    cpu_history:     HashMap<u32, RollingMean>,
}

#[derive(Default)]
pub(crate) struct RunningTargets {
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
#[derive(Clone, Copy)]
pub(crate) struct RunningInstance {
    /// OS process id, used to terminate the instance.
    pub pid:          u32,
    /// CPU usage in percent. A busy multi-threaded process can exceed 100.
    pub cpu_percent:  f32,
    /// Resident memory in bytes.
    pub memory_bytes: u64,
    /// How the target was launched, shown as the row marker.
    pub profile:      RunProfile,
    /// When the poller first observed this PID — the Running list sorts by
    /// it so the newest instance is the bottom row.
    pub first_seen:   Instant,
    /// The process's start time in seconds since the epoch, from the OS.
    /// Verified immediately before `SIGTERM` so a kill never lands on a
    /// reused PID.
    pub create_time:  u64,
    /// The direct OS parent, when this instance descends from another
    /// tracked instance (the parent is then itself shown in the outline —
    /// a tracked instance or a [`ChildProcess`] on the same chain).
    /// `None` for an independently started, top-level instance.
    pub parent_pid:   Option<u32>,
}

/// One untracked process descended from a tracked instance — e.g. the
/// `cargo` and `rustc` processes a `cargo mend` run spawns. Carries the
/// same live metrics as an instance so its Running row reads the same.
pub(crate) struct ChildProcess {
    /// OS process id, used to terminate the process.
    pub pid:          u32,
    /// The process's executable name — the Target cell of its row.
    pub name:         String,
    /// CPU usage in percent, smoothed over the poll window.
    pub cpu_percent:  f32,
    /// Resident memory in bytes.
    pub memory_bytes: u64,
    /// When the poller first observed this PID.
    pub first_seen:   Instant,
    /// The process's start time in seconds since the epoch, for the
    /// pre-`SIGTERM` reuse check.
    pub create_time:  u64,
    /// The direct OS parent — always itself shown in the outline, since
    /// every ancestor up to the tracked root is on the same chain.
    pub parent_pid:   u32,
}

/// How a running target's binary was launched — the marker shown in place
/// of a bare "running" flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunProfile {
    /// Exe under `target/debug/`.
    Debug,
    /// Exe under `target/release/`.
    Release,
    /// Exe is a `cargo install`ed binary in the cargo bin directory
    /// (e.g. run via a `cargo <name>` subcommand).
    Installed,
}

impl RunProfile {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
            Self::Installed => "cargo",
        }
    }

    /// Whether this is a `cargo install`ed binary — the instances the
    /// Running list groups under its collapsible `cargo` header.
    pub(super) const fn is_installed(self) -> bool { matches!(self, Self::Installed) }
}

/// Key identifying a running target. Matched against `(target_dir, kind, name)`
/// derived from the project's metadata at render time.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub(crate) struct RunningKey {
    pub target_dir: AbsolutePath,
    pub kind:       RunTargetKind,
    pub name:       String,
}

/// One workspace's slice fed to the poller per tick. `target_dir` is the
/// canonicalized `target_directory`; `bench_names` is the union of bench
/// target names (the safety net for classifying `deps/<name>-<hash>`
/// exes); `bin_names` is the union of bin target names (used to match
/// `cargo install`ed binaries in the cargo bin directory); `member_dirs`
/// maps each `(kind, name)` target to the manifest dir of the workspace
/// member that owns it, with `workspace_root` as the fallback for exes
/// whose target the metadata no longer declares.
pub(crate) struct ProjectTargetSlice<'a> {
    pub target_dir:     &'a AbsolutePath,
    pub workspace_root: &'a AbsolutePath,
    pub bench_names:    &'a HashSet<String>,
    pub bin_names:      &'a HashSet<String>,
    pub member_dirs:    &'a HashMap<(RunTargetKind, String), AbsolutePath>,
}

impl ProjectTargetSlice<'_> {
    /// Manifest dir of the member that owns `(kind, name)`, falling back to
    /// the workspace root when the metadata no longer declares the target
    /// (a stale build artifact).
    fn member_dir(&self, kind: RunTargetKind, name: &str) -> AbsolutePath {
        self.member_dirs
            .get(&(kind, name.to_string()))
            .unwrap_or(self.workspace_root)
            .clone()
    }
}

impl RunningTargetsPoller {
    pub(super) fn new(poll_interval: Duration) -> Self {
        Self {
            system: System::new(),
            last_poll: None,
            poll_interval,
            snapshot: RunningTargets::default(),
            install_bin_dir: cargo_install_bin_dir(),
            first_seen: HashMap::new(),
            cpu_history: HashMap::new(),
        }
    }

    /// Refresh if due. Returns the current snapshot regardless of cadence.
    pub(super) fn tick(
        &mut self,
        now: Instant,
        projects: &[ProjectTargetSlice<'_>],
    ) -> &RunningTargets {
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < self.poll_interval)
        {
            return &self.snapshot;
        }
        self.last_poll = Some(now);

        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_exe(UpdateKind::Always)
                .with_cwd(UpdateKind::Always)
                .with_cpu()
                .with_memory(),
        );

        let mut by_key = self.collect_instances(now, projects);
        // Stable per-key order so the pane's instance rows (and the cursor
        // resting on one) don't reshuffle between ticks.
        let links = |pid: u32| {
            self.system
                .process(Pid::from_u32(pid))
                .map(|process| ParentLink {
                    parent:     process.parent().map(Pid::as_u32),
                    start_time: process.start_time(),
                })
        };
        let tracked: HashSet<u32> = by_key
            .values()
            .flat_map(|target| target.instances.iter().map(|inst| inst.pid))
            .collect();
        for target in by_key.values_mut() {
            target.instances.sort_by_key(|inst| inst.pid);
            for inst in &mut target.instances {
                inst.parent_pid = shown_parent(&links, inst.pid, &tracked);
            }
        }
        // Everything a tracked instance spawned, however deep: any process
        // whose ancestor chain reaches a tracked PID joins the outline.
        let mut children = Vec::new();
        for (pid, process) in self.system.processes() {
            let pid = pid.as_u32();
            if tracked.contains(&pid) {
                continue;
            }
            let Some(parent_pid) = shown_parent(&links, pid, &tracked) else {
                continue;
            };
            let first_seen = *self.first_seen.entry(pid).or_insert(now);
            let cpu = smoothed_cpu(&mut self.cpu_history, pid, process.cpu_usage());
            children.push(ChildProcess {
                pid,
                name: process.name().to_string_lossy().into_owned(),
                cpu_percent: cpu,
                memory_bytes: process.memory(),
                first_seen,
                create_time: process.start_time(),
                parent_pid,
            });
        }
        // Retain only PIDs still shown, so an exited PID's slot is fresh
        // when the OS reuses the number.
        let live: HashSet<u32> = tracked
            .iter()
            .copied()
            .chain(children.iter().map(|child| child.pid))
            .collect();
        self.first_seen.retain(|pid, _| live.contains(pid));
        self.cpu_history.retain(|pid, _| live.contains(pid));
        self.snapshot = RunningTargets { by_key, children };
        &self.snapshot
    }

    /// One pass over the process table: attribute every process that is a
    /// workspace target binary (or an installed copy of one) to its key.
    fn collect_instances(
        &mut self,
        now: Instant,
        projects: &[ProjectTargetSlice<'_>],
    ) -> HashMap<RunningKey, RunningTarget> {
        let install_bin_dir = self.install_bin_dir.as_ref().map(AbsolutePath::as_path);
        let mut by_key: HashMap<RunningKey, RunningTarget> = HashMap::new();
        for (pid, process) in self.system.processes() {
            let Some(exe) = process.exe() else {
                tracing::debug!(pid = pid.as_u32(), "running_targets_exe_unavailable");
                continue;
            };
            // `cargo run`/`cargo run --example` exec a path relative to the
            // package dir, so the kernel reports a relative exe. Resolve it
            // against the process cwd so it can be matched against absolute
            // target directories.
            let exe = if exe.is_absolute() {
                Cow::Borrowed(exe)
            } else {
                process
                    .cwd()
                    .map_or(Cow::Borrowed(exe), |cwd| Cow::Owned(cwd.join(exe)))
            };
            let pid = pid.as_u32();
            let cpu = process.cpu_usage();
            let memory = process.memory();
            let create_time = process.start_time();
            if let Some((key, profile, member_dir)) = classify_exe(&exe, projects) {
                let first_seen = *self.first_seen.entry(pid).or_insert(now);
                let cpu = smoothed_cpu(&mut self.cpu_history, pid, cpu);
                push_instance(
                    &mut by_key,
                    key,
                    member_dir,
                    instance(pid, cpu, memory, profile, first_seen, create_time),
                );
            } else {
                let keys = installed_bin_keys(&exe, projects, install_bin_dir);
                if keys.is_empty() {
                    continue;
                }
                // One OS process: feed its sample once, however many
                // projects the installed binary is attributed to.
                let first_seen = *self.first_seen.entry(pid).or_insert(now);
                let cpu = smoothed_cpu(&mut self.cpu_history, pid, cpu);
                for (key, member_dir) in keys {
                    push_instance(
                        &mut by_key,
                        key,
                        member_dir,
                        instance(
                            pid,
                            cpu,
                            memory,
                            RunProfile::Installed,
                            first_seen,
                            create_time,
                        ),
                    );
                }
            }
        }
        by_key
    }

    pub(super) const fn snapshot(&self) -> &RunningTargets { &self.snapshot }

    /// Replace the snapshot directly, bypassing the process poll. Used by
    /// render/dispatch tests.
    #[cfg(test)]
    pub(crate) fn set_snapshot_for_test(&mut self, snapshot: RunningTargets) {
        self.snapshot = snapshot;
    }

    /// Send `SIGTERM` to `pid` if it is still the process the kill request
    /// named. Refreshes that one PID and verifies the process's start time
    /// matches `create_time` first — a PID the OS reassigned between the
    /// confirm dialog opening and the user pressing `y` fails the check and
    /// the kill is skipped. Returns `true` when the signal was delivered.
    pub(super) fn kill(&mut self, pid: u32, create_time: u64) -> bool {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        self.system
            .process(Pid::from_u32(pid))
            .filter(|process| process.start_time() == create_time)
            .is_some_and(|process| process.kill_with(Signal::Term).unwrap_or(false))
    }

    /// Drop `pids` from the current snapshot without waiting for the next
    /// poll. After killing an instance this collapses its row immediately
    /// so the pane reflects the change on the very next render (the next
    /// poll would do the same once the process exits). Also evicts the
    /// PIDs' first-seen entries so a reused number starts a fresh slot.
    pub(super) fn drop_instances(&mut self, pids: &[u32]) {
        self.snapshot.drop_pids(pids);
        for pid in pids {
            self.first_seen.remove(pid);
            self.cpu_history.remove(pid);
        }
    }
}

impl RunningTargets {
    /// Every tracked key with its owning member dir and instances (sorted
    /// by PID). Iteration order is arbitrary; callers sort the flattened
    /// rows themselves.
    pub(super) fn iter_targets(
        &self,
    ) -> impl Iterator<Item = (&RunningKey, &AbsolutePath, &[RunningInstance])> {
        self.by_key
            .iter()
            .map(|(key, target)| (key, &target.member_dir, target.instances.as_slice()))
    }

    /// Whether any tracked instance is currently running — keys with no
    /// live instances are dropped each tick, so a non-empty map means
    /// live processes.
    pub(super) fn has_instances(&self) -> bool { !self.by_key.is_empty() }

    /// Untracked descendants of tracked instances, for the Running list's
    /// outline. Unordered; the row builder nests them by parent link.
    pub(super) fn child_processes(&self) -> &[ChildProcess] { &self.children }

    /// Remove every instance and child process whose PID is in `pids`,
    /// dropping any key left with no instances.
    fn drop_pids(&mut self, pids: &[u32]) {
        for target in self.by_key.values_mut() {
            target.instances.retain(|inst| !pids.contains(&inst.pid));
        }
        self.by_key.retain(|_, target| !target.instances.is_empty());
        self.children.retain(|child| !pids.contains(&child.pid));
    }

    /// Build a snapshot directly from `(key, instances)` pairs, bypassing
    /// the process poll. Each key's member dir is its `target_dir`'s parent
    /// (the workspace root in the standard layout). Used by render/dispatch
    /// tests.
    #[cfg(test)]
    pub(crate) fn from_pairs(pairs: Vec<(RunningKey, Vec<RunningInstance>)>) -> Self {
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
    pub(crate) fn with_children(mut self, children: Vec<ChildProcess>) -> Self {
        self.children = children;
        self
    }
}

#[cfg(test)]
impl ChildProcess {
    /// A test child process with zeroed metrics, the PID doubling as the
    /// first-seen order and create time, like `RunningInstance::for_test`.
    pub(crate) fn for_test(pid: u32, name: &str, parent_pid: u32) -> Self {
        Self {
            pid,
            name: name.to_string(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            first_seen: test_instant_at(pid),
            create_time: u64::from(pid),
            parent_pid,
        }
    }
}

#[cfg(test)]
impl RunningInstance {
    /// A test instance with the given PID and profile; zeroed metrics, the
    /// PID doubling as the first-seen order (lower PID = seen earlier).
    pub(crate) fn for_test(pid: u32, profile: RunProfile) -> Self {
        Self {
            pid,
            cpu_percent: 0.0,
            memory_bytes: 0,
            profile,
            first_seen: test_instant_at(pid),
            create_time: u64::from(pid),
            parent_pid: None,
        }
    }

    /// The same test instance nested under `parent` in the outline.
    pub(crate) const fn with_parent(mut self, parent: u32) -> Self {
        self.parent_pid = Some(parent);
        self
    }

    /// The same test instance with live-looking metrics.
    pub(crate) const fn with_metrics(mut self, cpu_percent: f32, memory_bytes: u64) -> Self {
        self.cpu_percent = cpu_percent;
        self.memory_bytes = memory_bytes;
        self
    }
}

/// A deterministic `Instant` for test fixtures: a shared base plus `order`
/// seconds, so fixtures can express relative first-seen order.
#[cfg(test)]
pub(crate) fn test_instant_at(order: u32) -> Instant {
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
) -> RunningInstance {
    RunningInstance {
        pid,
        cpu_percent: cpu,
        memory_bytes: memory,
        profile,
        first_seen,
        create_time,
        parent_pid: None,
    }
}

/// One process's link in the ancestor walk: its parent PID (if any) and its
/// start time (seconds since the epoch), used to reject hops into reused
/// PIDs — a parent never starts after its child.
#[derive(Clone, Copy)]
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
/// that same chain, so the direct parent is always itself shown. `None`
/// for an independently started process (its row renders top-level).
fn shown_parent(
    links: &impl Fn(u32) -> Option<ParentLink>,
    pid: u32,
    tracked: &HashSet<u32>,
) -> Option<u32> {
    nearest_tracked_ancestor(links, pid, tracked)?;
    links(pid)?.parent
}

/// Fold this poll's CPU sample into `pid`'s [`RollingMean`] window and
/// return the mean — the value the Running list's CPU column shows.
fn smoothed_cpu(history: &mut HashMap<u32, RollingMean>, pid: u32, sample: f32) -> f32 {
    history.entry(pid).or_default().push(sample)
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

/// Classify an exe that lives under a project's `target_dir` as a bin /
/// example / bench, returning the unique `(RunningKey, RunProfile)` plus
/// the manifest dir of the workspace member that owns the target. `None`
/// for anything not under a known `target_dir` or not a runnable target
/// (`deps/<test>-<hash>`, `build/`, ...). Installed binaries are handled
/// separately by [`installed_bin_keys`].
fn classify_exe(
    exe: &Path,
    projects: &[ProjectTargetSlice<'_>],
) -> Option<(RunningKey, RunProfile, AbsolutePath)> {
    for slice in projects {
        if let Ok(rest) = exe.strip_prefix(slice.target_dir.as_path())
            && let Some((kind, name, profile)) = classify_tail(rest, slice.bench_names)
        {
            let member_dir = slice.member_dir(kind, &name);
            let key = RunningKey {
                target_dir: slice.target_dir.clone(),
                kind,
                name,
            };
            return Some((key, profile, member_dir));
        }
    }
    None
}

/// Keys for a `cargo install`ed binary: an exe living directly in
/// `install_bin_dir` whose file name is a declared bin target name. A bin
/// name can be declared by more than one project (e.g. the primary repo
/// and its worktrees all build `cargo-port`), and we can't tell which one
/// was installed — so the running process is attributed to *every*
/// matching project. The render side then matches whichever is selected.
fn installed_bin_keys(
    exe: &Path,
    projects: &[ProjectTargetSlice<'_>],
    install_bin_dir: Option<&Path>,
) -> Vec<(RunningKey, AbsolutePath)> {
    let Some(bin_dir) = install_bin_dir else {
        return Vec::new();
    };
    if exe.parent() != Some(bin_dir) {
        return Vec::new();
    }
    let Some(stem) = exe.file_stem().and_then(|s| s.to_str()) else {
        return Vec::new();
    };
    projects
        .iter()
        .filter(|slice| slice.bin_names.contains(stem))
        .map(|slice| {
            (
                RunningKey {
                    target_dir: slice.target_dir.clone(),
                    kind:       RunTargetKind::Binary,
                    name:       stem.to_string(),
                },
                slice.member_dir(RunTargetKind::Binary, stem),
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
fn cargo_install_bin_dir() -> Option<AbsolutePath> {
    let base = env::var_os("CARGO_INSTALL_ROOT")
        .map(PathBuf::from)
        .or_else(|| env::var_os("CARGO_HOME").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))?;
    let bin = base.join("bin");
    let canonical = bin.canonicalize().unwrap_or(bin);
    Some(AbsolutePath::from(canonical))
}

/// Parse a `target/<profile>/deps/<basename>` entry as a bench. The basename
/// is `<name>-<hash>` where `<hash>` is 16+ lowercase hex chars. The longest
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
    s.len() >= 16
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

    use super::*;

    /// The shared empty member-dir map for slices whose tests don't exercise
    /// member resolution — every lookup falls back to the workspace root.
    fn no_member_dirs() -> HashMap<(RunTargetKind, String), AbsolutePath> { HashMap::new() }

    fn slice<'a>(
        dir: &'a AbsolutePath,
        bench_names: &'a HashSet<String>,
        bin_names: &'a HashSet<String>,
        member_dirs: &'a HashMap<(RunTargetKind, String), AbsolutePath>,
    ) -> ProjectTargetSlice<'a> {
        ProjectTargetSlice {
            target_dir: dir,
            workspace_root: dir,
            bench_names,
            bin_names,
            member_dirs,
        }
    }

    /// A candidate executable path, made absolute on the host platform so it
    /// shares the same drive prefix as the `AbsolutePath` target dir it is
    /// matched against. Identity on Unix.
    fn exe_path(path: &str) -> PathBuf { crate::project::normalize_test_path(Path::new(path)) }

    fn names(names: &[&str]) -> HashSet<String> { names.iter().map(|s| (*s).to_string()).collect() }

    #[test]
    fn debug_bin() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/foo");
        let (key, profile, _member) =
            classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Binary));
        assert_eq!(key.name, "foo");
        assert_eq!(key.target_dir, dir);
        assert_eq!(profile, RunProfile::Debug);
    }

    #[test]
    fn release_example() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/release/examples/bar");
        let (key, profile, _member) =
            classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Example));
        assert_eq!(key.name, "bar");
        assert_eq!(profile, RunProfile::Release);
    }

    #[test]
    fn bench_with_known_name() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&["baz"]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-0123456789abcdef");
        let (key, _, _) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Bench));
        assert_eq!(key.name, "baz");
    }

    #[test]
    fn bench_rejects_short_hash() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&["baz"]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-shorthash");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn deps_entry_not_in_bench_set_is_unrecognized() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&["baz"]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/other-0123456789abcdef");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn longest_bench_name_wins() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&["my", "my-bench"]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/deps/my-bench-0123456789abcdef");
        let (key, _, _) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Bench));
        assert_eq!(key.name, "my-bench");
    }

    #[test]
    fn outside_target_dir_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/usr/bin/ls");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn build_artifact_under_target_ignored() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/build/foo-1234567890abcdef/build-script-build");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn installed_bin_in_cargo_dir_matches_as_cargo_profile() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&["cargo-port"]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/cargo-port");
        let keys = installed_bin_keys(&exe, std::slice::from_ref(&s), Some(&bin_dir));
        assert_eq!(keys.len(), 1);
        let (key, member_dir) = &keys[0];
        assert!(matches!(key.kind, RunTargetKind::Binary));
        assert_eq!(key.name, "cargo-port");
        assert_eq!(key.target_dir, dir);
        // No member-dir entry: attribution falls back to the slice's
        // workspace root (the fixture points it at the target dir).
        assert_eq!(*member_dir, dir);
    }

    #[test]
    fn installed_bin_attributed_to_every_project_declaring_it() {
        let primary = AbsolutePath::from(PathBuf::from("/tmp/main/target"));
        let worktree = AbsolutePath::from(PathBuf::from("/tmp/wt/target"));
        let (benches, bins, members) = (names(&[]), names(&["cargo-port"]), no_member_dirs());
        let slices = [
            slice(&primary, &benches, &bins, &members),
            slice(&worktree, &benches, &bins, &members),
        ];
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/cargo-port");
        let dirs: HashSet<AbsolutePath> = installed_bin_keys(&exe, &slices, Some(&bin_dir))
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
        let members = HashMap::from([((RunTargetKind::Binary, "foo".to_string()), member.clone())]);
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/foo");
        let (_, _, member_dir) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert_eq!(member_dir, member);
    }

    #[test]
    fn unknown_target_falls_back_to_the_workspace_root() {
        // A stale artifact of a renamed target: nothing in the member map,
        // so the path attribution falls back to the workspace root.
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&[]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let exe = exe_path("/tmp/ws/target/debug/stale");
        let (_, _, member_dir) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert_eq!(member_dir, dir);
    }

    #[test]
    fn drop_instances_evicts_the_first_seen_entry() {
        let mut poller = RunningTargetsPoller::new(Duration::from_secs(1));
        poller.first_seen.insert(42, test_instant_at(0));
        poller.first_seen.insert(43, test_instant_at(1));
        poller.drop_instances(&[42]);
        assert!(!poller.first_seen.contains_key(&42));
        assert!(poller.first_seen.contains_key(&43));
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
    fn shown_parent_is_the_direct_parent_on_a_tracked_chain() {
        // The wrapper (30) reaches the tracked orchestrator (10) through
        // the untracked cargo shim (20); its outline parent is the shim —
        // the shim itself joins the outline as a descendant.
        let links = links_from(vec![
            (10, Some(1), 100),
            (20, Some(10), 150),
            (30, Some(20), 200),
        ]);
        let tracked = HashSet::from([10, 30]);
        assert_eq!(shown_parent(&links, 30, &tracked), Some(20));
        assert_eq!(shown_parent(&links, 20, &tracked), Some(10));
    }

    #[test]
    fn shown_parent_is_none_for_an_independently_started_process() {
        // The chain tops out at the shell (1) without crossing another
        // tracked PID: the row renders top-level.
        let links = links_from(vec![(1, None, 0), (10, Some(1), 100)]);
        let tracked = HashSet::from([10]);
        assert_eq!(shown_parent(&links, 10, &tracked), None);
    }

    #[test]
    fn smoothed_cpu_averages_the_window() {
        let mut history = HashMap::new();
        // First sample is the mean of one — no zero dilution.
        assert!((smoothed_cpu(&mut history, 7, 20.0) - 20.0).abs() < f32::EPSILON);
        // 20 and 10 average to 15.
        assert!((smoothed_cpu(&mut history, 7, 10.0) - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn smoothed_cpu_window_drops_the_oldest_sample() {
        let mut history = HashMap::new();
        // Fill the window with zeros, then push spikes: once the window
        // holds only the spikes, the zeros no longer drag the mean down.
        for _ in 0..tui_pane::CPU_SMOOTHING_WINDOW_POLLS {
            smoothed_cpu(&mut history, 7, 0.0);
        }
        let mut mean = 0.0;
        for _ in 0..tui_pane::CPU_SMOOTHING_WINDOW_POLLS {
            mean = smoothed_cpu(&mut history, 7, 50.0);
        }
        assert!((mean - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn smoothed_cpu_tracks_pids_independently() {
        let mut history = HashMap::new();
        smoothed_cpu(&mut history, 7, 40.0);
        // A different PID's first sample is unaffected by PID 7's window.
        assert!((smoothed_cpu(&mut history, 8, 10.0) - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn drop_instances_evicts_the_cpu_history_entry() {
        let mut poller = RunningTargetsPoller::new(Duration::from_secs(1));
        smoothed_cpu(&mut poller.cpu_history, 42, 10.0);
        smoothed_cpu(&mut poller.cpu_history, 43, 10.0);
        poller.drop_instances(&[42]);
        assert!(!poller.cpu_history.contains_key(&42));
        assert!(poller.cpu_history.contains_key(&43));
    }

    #[test]
    fn installed_bin_not_in_bin_set_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&["cargo-port"]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/ripgrep");
        assert!(installed_bin_keys(&exe, std::slice::from_ref(&s), Some(&bin_dir)).is_empty());
    }

    #[test]
    fn bin_outside_cargo_dir_does_not_match_as_installed() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins, members) = (names(&[]), names(&["cargo-port"]), no_member_dirs());
        let s = slice(&dir, &benches, &bins, &members);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/usr/local/bin/cargo-port");
        assert!(installed_bin_keys(&exe, std::slice::from_ref(&s), Some(&bin_dir)).is_empty());
    }
}
