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
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::Signal;
use sysinfo::System;
use sysinfo::UpdateKind;

use super::panes::RunTargetKind;
use crate::project::AbsolutePath;

pub(crate) struct RunningTargetsPoller {
    system:          System,
    last_poll:       Option<Instant>,
    poll_interval:   Duration,
    snapshot:        RunningTargets,
    /// Canonical cargo install bin directory (`~/.cargo/bin` by default).
    /// Exes living directly here are matched as installed binaries,
    /// surfaced as the `cargo` profile. `None` when it can't be resolved.
    install_bin_dir: Option<AbsolutePath>,
}

#[derive(Default)]
pub(crate) struct RunningTargets {
    by_key: HashMap<RunningKey, Vec<RunningInstance>>,
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
/// `cargo install`ed binaries in the cargo bin directory).
pub(crate) struct ProjectTargetSlice<'a> {
    pub target_dir:  &'a AbsolutePath,
    pub bench_names: &'a HashSet<String>,
    pub bin_names:   &'a HashSet<String>,
}

impl RunningTargetsPoller {
    pub(super) fn new(poll_interval: Duration) -> Self {
        Self {
            system: System::new(),
            last_poll: None,
            poll_interval,
            snapshot: RunningTargets::default(),
            install_bin_dir: cargo_install_bin_dir(),
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

        let install_bin_dir = self.install_bin_dir.as_ref().map(AbsolutePath::as_path);
        let mut by_key: HashMap<RunningKey, Vec<RunningInstance>> = HashMap::new();
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
            if let Some((key, profile)) = classify_exe(&exe, projects) {
                push_instance(&mut by_key, key, instance(pid, cpu, memory, profile));
            } else {
                for key in installed_bin_keys(&exe, projects, install_bin_dir) {
                    push_instance(
                        &mut by_key,
                        key,
                        instance(pid, cpu, memory, RunProfile::Installed),
                    );
                }
            }
        }
        // Stable per-key order so the pane's instance rows (and the cursor
        // resting on one) don't reshuffle between ticks.
        for instances in by_key.values_mut() {
            instances.sort_by_key(|inst| inst.pid);
        }
        self.snapshot = RunningTargets { by_key };
        &self.snapshot
    }

    pub(super) const fn snapshot(&self) -> &RunningTargets { &self.snapshot }

    /// Send `SIGTERM` to `pid` if it is still a live process. Returns
    /// `true` when the signal was delivered. Uses the most recent process
    /// table; a PID that has already exited returns `false`.
    pub(super) fn kill(&self, pid: u32) -> bool {
        self.system
            .process(Pid::from_u32(pid))
            .is_some_and(|process| process.kill_with(Signal::Term).unwrap_or(false))
    }

    /// Drop `pids` from the current snapshot without waiting for the next
    /// poll. After killing an instance this collapses its row immediately
    /// so the pane reflects the change on the very next render (the next
    /// poll would do the same once the process exits).
    pub(super) fn drop_instances(&mut self, pids: &[u32]) { self.snapshot.drop_pids(pids); }
}

impl RunningTargets {
    /// Running instances for `key`, sorted by PID. Empty when the target
    /// is not running.
    pub(super) fn instances(&self, key: &RunningKey) -> &[RunningInstance] {
        self.by_key.get(key).map_or(&[], Vec::as_slice)
    }

    /// Remove every instance whose PID is in `pids`, dropping any key left
    /// with no instances.
    fn drop_pids(&mut self, pids: &[u32]) {
        for instances in self.by_key.values_mut() {
            instances.retain(|inst| !pids.contains(&inst.pid));
        }
        self.by_key.retain(|_, instances| !instances.is_empty());
    }

    /// Build a snapshot directly from `(key, instances)` pairs, bypassing
    /// the process poll. Used by render/dispatch tests.
    #[cfg(test)]
    pub(crate) fn from_pairs(pairs: Vec<(RunningKey, Vec<RunningInstance>)>) -> Self {
        Self {
            by_key: pairs.into_iter().collect(),
        }
    }
}

#[cfg(test)]
impl RunningInstance {
    /// A test instance with the given PID and profile; zeroed metrics.
    pub(crate) fn for_test(pid: u32, profile: RunProfile) -> Self {
        Self {
            pid,
            cpu_percent: 0.0,
            memory_bytes: 0,
            profile,
        }
    }
}

const fn instance(pid: u32, cpu: f32, memory: u64, profile: RunProfile) -> RunningInstance {
    RunningInstance {
        pid,
        cpu_percent: cpu,
        memory_bytes: memory,
        profile,
    }
}

/// Append one process's metrics under `key`.
fn push_instance(
    by_key: &mut HashMap<RunningKey, Vec<RunningInstance>>,
    key: RunningKey,
    inst: RunningInstance,
) {
    by_key.entry(key).or_default().push(inst);
}

/// Classify an exe that lives under a project's `target_dir` as a bin /
/// example / bench, returning the unique `(RunningKey, RunProfile)`.
/// `None` for anything not under a known `target_dir` or not a runnable
/// target (`deps/<test>-<hash>`, `build/`, ...). Installed binaries are
/// handled separately by [`installed_bin_keys`].
fn classify_exe(
    exe: &Path,
    projects: &[ProjectTargetSlice<'_>],
) -> Option<(RunningKey, RunProfile)> {
    for slice in projects {
        if let Ok(rest) = exe.strip_prefix(slice.target_dir.as_path())
            && let Some((kind, name, profile)) = classify_tail(rest, slice.bench_names)
        {
            let key = RunningKey {
                target_dir: slice.target_dir.clone(),
                kind,
                name,
            };
            return Some((key, profile));
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
) -> Vec<RunningKey> {
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
        .map(|slice| RunningKey {
            target_dir: slice.target_dir.clone(),
            kind:       RunTargetKind::Binary,
            name:       stem.to_string(),
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

    fn slice<'a>(
        dir: &'a AbsolutePath,
        bench_names: &'a HashSet<String>,
        bin_names: &'a HashSet<String>,
    ) -> ProjectTargetSlice<'a> {
        ProjectTargetSlice {
            target_dir: dir,
            bench_names,
            bin_names,
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
        let (benches, bins) = (names(&[]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/debug/foo");
        let (key, profile) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Binary));
        assert_eq!(key.name, "foo");
        assert_eq!(key.target_dir, dir);
        assert_eq!(profile, RunProfile::Debug);
    }

    #[test]
    fn release_example() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&[]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/release/examples/bar");
        let (key, profile) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Example));
        assert_eq!(key.name, "bar");
        assert_eq!(profile, RunProfile::Release);
    }

    #[test]
    fn bench_with_known_name() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&["baz"]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-0123456789abcdef");
        let (key, _) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Bench));
        assert_eq!(key.name, "baz");
    }

    #[test]
    fn bench_rejects_short_hash() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&["baz"]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-shorthash");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn deps_entry_not_in_bench_set_is_unrecognized() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&["baz"]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/debug/deps/other-0123456789abcdef");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn longest_bench_name_wins() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&["my", "my-bench"]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/debug/deps/my-bench-0123456789abcdef");
        let (key, _) = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Bench));
        assert_eq!(key.name, "my-bench");
    }

    #[test]
    fn outside_target_dir_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&[]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/usr/bin/ls");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn build_artifact_under_target_ignored() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&[]), names(&[]));
        let s = slice(&dir, &benches, &bins);
        let exe = exe_path("/tmp/ws/target/debug/build/foo-1234567890abcdef/build-script-build");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn installed_bin_in_cargo_dir_matches_as_cargo_profile() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&[]), names(&["cargo-port"]));
        let s = slice(&dir, &benches, &bins);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/cargo-port");
        let keys = installed_bin_keys(&exe, std::slice::from_ref(&s), Some(&bin_dir));
        assert_eq!(keys.len(), 1);
        assert!(matches!(keys[0].kind, RunTargetKind::Binary));
        assert_eq!(keys[0].name, "cargo-port");
        assert_eq!(keys[0].target_dir, dir);
    }

    #[test]
    fn installed_bin_attributed_to_every_project_declaring_it() {
        let primary = AbsolutePath::from(PathBuf::from("/tmp/main/target"));
        let worktree = AbsolutePath::from(PathBuf::from("/tmp/wt/target"));
        let (benches, bins) = (names(&[]), names(&["cargo-port"]));
        let slices = [
            slice(&primary, &benches, &bins),
            slice(&worktree, &benches, &bins),
        ];
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/cargo-port");
        let dirs: HashSet<AbsolutePath> = installed_bin_keys(&exe, &slices, Some(&bin_dir))
            .into_iter()
            .map(|key| key.target_dir)
            .collect();
        assert_eq!(dirs, HashSet::from([primary, worktree]));
    }

    #[test]
    fn installed_bin_not_in_bin_set_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&[]), names(&["cargo-port"]));
        let s = slice(&dir, &benches, &bins);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/home/me/.cargo/bin/ripgrep");
        assert!(installed_bin_keys(&exe, std::slice::from_ref(&s), Some(&bin_dir)).is_empty());
    }

    #[test]
    fn bin_outside_cargo_dir_does_not_match_as_installed() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let (benches, bins) = (names(&[]), names(&["cargo-port"]));
        let s = slice(&dir, &benches, &bins);
        let bin_dir = exe_path("/home/me/.cargo/bin");
        let exe = exe_path("/usr/local/bin/cargo-port");
        assert!(installed_bin_keys(&exe, std::slice::from_ref(&s), Some(&bin_dir)).is_empty());
    }
}
