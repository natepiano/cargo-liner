//! Detect which cargo bin/example/bench targets are currently running.
//!
//! Each tick refreshes the system process list (exe paths only) and walks
//! every process whose exe lives under a known workspace `target_directory`.
//! The path tail is parsed against cargo's filesystem layout to classify
//! the exe as a bin / example / bench of that workspace.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

use super::panes::RunTargetKind;
use crate::project::AbsolutePath;

pub(crate) struct RunningTargetsPoller {
    system:        System,
    last_poll:     Option<Instant>,
    poll_interval: Duration,
    snapshot:      RunningTargets,
}

#[derive(Default)]
pub(crate) struct RunningTargets {
    by_key: HashMap<RunningKey, RunningMetrics>,
}

/// Aggregated resource use of a running target's process(es), summed when
/// a single target maps to more than one OS process.
#[derive(Clone, Copy, Default)]
pub(crate) struct RunningMetrics {
    /// CPU usage in percent. A busy multi-threaded process can exceed 100.
    pub cpu_percent:  f32,
    /// Resident memory in bytes.
    pub memory_bytes: u64,
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
/// target names across the workspace's packages (the safety net for
/// classifying `deps/<name>-<hash>` exes).
pub(crate) struct ProjectTargetSlice<'a> {
    pub target_dir:  &'a AbsolutePath,
    pub bench_names: &'a HashSet<String>,
}

impl RunningTargetsPoller {
    pub(super) fn new(poll_interval: Duration) -> Self {
        Self {
            system: System::new(),
            last_poll: None,
            poll_interval,
            snapshot: RunningTargets::default(),
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
                .with_cpu()
                .with_memory(),
        );

        let mut by_key: HashMap<RunningKey, RunningMetrics> = HashMap::new();
        for (pid, process) in self.system.processes() {
            let Some(exe) = process.exe() else {
                tracing::debug!(pid = pid.as_u32(), "running_targets_exe_unavailable");
                continue;
            };
            if let Some(key) = classify_exe(exe, projects) {
                let metrics = by_key.entry(key).or_default();
                metrics.cpu_percent += process.cpu_usage();
                metrics.memory_bytes += process.memory();
            }
        }
        self.snapshot = RunningTargets { by_key };
        &self.snapshot
    }

    pub(super) const fn snapshot(&self) -> &RunningTargets { &self.snapshot }
}

impl RunningTargets {
    /// Aggregated CPU/memory for `key`'s running process(es), or `None`
    /// when no matching process is in the latest snapshot (i.e. the
    /// target is not running).
    pub(super) fn metrics(&self, key: &RunningKey) -> Option<RunningMetrics> {
        self.by_key.get(key).copied()
    }
}

/// Classify `exe` against the project slices. Returns the matching slice plus
/// the parsed `(kind, name)` when the exe is under a known `target_dir` and
/// the tail segments match cargo's bin / example / bench layout. Returns
/// `None` for any other path (including `target/` entries that aren't a
/// runnable target: `deps/<test>-<hash>`, `build/`, `incremental/`, ...).
fn classify_exe(exe: &Path, projects: &[ProjectTargetSlice<'_>]) -> Option<RunningKey> {
    for slice in projects {
        if let Ok(rest) = exe.strip_prefix(slice.target_dir.as_path())
            && let Some((kind, name)) = classify_tail(rest, slice.bench_names)
        {
            return Some(RunningKey {
                target_dir: slice.target_dir.clone(),
                kind,
                name,
            });
        }
    }
    None
}

fn classify_tail(rest: &Path, bench_names: &HashSet<String>) -> Option<(RunTargetKind, String)> {
    let segments: Vec<&str> = rest
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    match segments.as_slice() {
        [profile, name] if is_profile(profile) => Some((RunTargetKind::Binary, (*name).into())),
        [profile, "examples", name] if is_profile(profile) => {
            Some((RunTargetKind::Example, (*name).into()))
        },
        [profile, "deps", basename] if is_profile(profile) => {
            parse_bench_basename(basename, bench_names).map(|name| (RunTargetKind::Bench, name))
        },
        _ => None,
    }
}

const fn is_profile(s: &str) -> bool { matches!(s.as_bytes(), b"debug" | b"release") }

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
    ) -> ProjectTargetSlice<'a> {
        ProjectTargetSlice {
            target_dir: dir,
            bench_names,
        }
    }

    /// A candidate executable path, made absolute on the host platform so it
    /// shares the same drive prefix as the `AbsolutePath` target dir it is
    /// matched against. Identity on Unix.
    fn exe_path(path: &str) -> PathBuf { crate::project::normalize_test_path(Path::new(path)) }

    fn benches(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn debug_bin() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&[]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/debug/foo");
        let key = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Binary));
        assert_eq!(key.name, "foo");
        assert_eq!(key.target_dir, dir);
    }

    #[test]
    fn release_example() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&[]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/release/examples/bar");
        let key = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Example));
        assert_eq!(key.name, "bar");
    }

    #[test]
    fn bench_with_known_name() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&["baz"]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-0123456789abcdef");
        let key = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Bench));
        assert_eq!(key.name, "baz");
    }

    #[test]
    fn bench_rejects_short_hash() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&["baz"]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/debug/deps/baz-shorthash");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn deps_entry_not_in_bench_set_is_unrecognized() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&["baz"]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/debug/deps/other-0123456789abcdef");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn longest_bench_name_wins() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&["my", "my-bench"]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/debug/deps/my-bench-0123456789abcdef");
        let key = classify_exe(&exe, std::slice::from_ref(&s)).expect("matches");
        assert!(matches!(key.kind, RunTargetKind::Bench));
        assert_eq!(key.name, "my-bench");
    }

    #[test]
    fn outside_target_dir_does_not_match() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&[]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/usr/bin/ls");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }

    #[test]
    fn build_artifact_under_target_ignored() {
        let dir = AbsolutePath::from(PathBuf::from("/tmp/ws/target"));
        let benches = benches(&[]);
        let s = slice(&dir, &benches);
        let exe = exe_path("/tmp/ws/target/debug/build/foo-1234567890abcdef/build-script-build");
        assert!(classify_exe(&exe, std::slice::from_ref(&s)).is_none());
    }
}
