//! How far a build has got, read out of the output the cargo shim
//! captured for it.
//!
//! Cargo already counts the work: while it compiles it draws
//! `Building [====>    ] 149/403: serde, regex` on stderr, and those two
//! numbers are the unit graph's completed and total counts. Nothing here
//! estimates anything -- the display shows cargo's own arithmetic.
//!
//! Reaching it takes a capture, because the invocations in the grid
//! belong to other terminals: [`crate::processes`] finds them by
//! scanning the process table, and a process's stdout is not readable
//! from outside it. The shim installed at `~/.rustup/toolchains/*/bin/cargo`
//! is what closes that gap. It runs each command under a pty, mirrors
//! the output into `<root>/run-<timestamp>-<pid>.log`, and registers the
//! run as `<root>/state/pids/<pid>` for as long as it lives -- the pid
//! in both being the shim's own, which is an ancestor of the cargo
//! process the grid draws.
//!
//! So a run reports progress when it was captured and reports none when
//! it was not, and the cell is drawn either way.

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;

use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::constants::CAPTURE_ROOT;
use crate::constants::CAPTURE_ROOT_ENV;
use crate::constants::PID_SEPARATOR;
use crate::constants::RUN_LOG_PREFIX;
use crate::constants::RUN_LOG_SUFFIX;
use crate::constants::RUN_LOG_TAIL_BYTES;
use crate::constants::UNIT_COUNTER_LEAD;
use crate::constants::UNIT_COUNTER_SEPARATOR;
use crate::constants::UNIT_COUNTER_TRAILER;

/// Cargo's count of the work in front of it, as its progress bar reports
/// it: units finished out of units planned.
///
/// A unit is one compilation of one crate target, which is what the
/// build is actually made of -- not a package and not a source file. A
/// unit already fresh counts as finished the moment cargo checks it, so
/// an incremental build opens near its total rather than at zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Progress {
    /// Units cargo has finished.
    pub(crate) done:  usize,
    /// Units in the build plan.
    pub(crate) total: usize,
}

impl Progress {
    /// How far along, rounded down, so only a finished build reads 100.
    pub(crate) const fn percent(self) -> usize {
        // `total` is never zero: `parse_counter` rejects a counter that
        // would divide by it.
        self.done.saturating_mul(100) / self.total
    }
}

/// The captured runs progress can currently be read from, keyed by the
/// pid of the shim that captured each one.
///
/// Empty whenever capture is switched off, which is the ordinary state
/// of a machine that never turned it on -- so building one costs a
/// single failed directory read, and every lookup against it misses.
#[derive(Default)]
pub(crate) struct Capture {
    /// Log file per live shim pid.
    logs: HashMap<u32, PathBuf>,
}

impl Capture {
    /// Take stock of the capture directory: which runs are still live,
    /// and which log belongs to each.
    ///
    /// The live set is read first because it is the cheap half and it
    /// decides the rest: logs are never deleted, so the directory holds
    /// every run since the last reboot, and matching against a live pid
    /// is what keeps a finished run's log from being read as a running
    /// one that happens to have inherited its pid.
    pub(crate) fn take() -> Self { Self::take_from(&root()) }

    /// [`Capture::take`] against a given directory, which is what makes
    /// the directory layout testable without moving the real one.
    fn take_from(root: &Path) -> Self {
        let live = live_runs(root);
        if live.is_empty() {
            return Self::default();
        }
        let Ok(entries) = fs::read_dir(root) else {
            return Self::default();
        };
        let logs = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let pid = log_pid(&path)?;
                live.contains(&pid).then_some((pid, path))
            })
            .collect();
        Self { logs }
    }

    /// What the run captured under `pid` last reported, or `None` when
    /// no run is captured under it.
    pub(crate) fn read(&self, pid: u32) -> Option<Progress> {
        parse_counter(&tail(self.logs.get(&pid)?)?)
    }
}

/// Where the shim writes its captures, which an environment variable
/// moves for a second instance of the grid.
fn root() -> PathBuf {
    env::var_os(CAPTURE_ROOT_ENV).map_or_else(|| PathBuf::from(CAPTURE_ROOT), PathBuf::from)
}

/// The shim pids with a run still in flight, one file each, removed by
/// the shim as it exits.
fn live_runs(root: &Path) -> HashSet<u32> {
    let Ok(entries) = fs::read_dir(root.join(CAPTURE_LIVE_RUNS_DIR)) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect()
}

/// The shim pid a log file is named for: `run-<timestamp>-<pid>.log`.
fn log_pid(path: &Path) -> Option<u32> {
    path.file_name()?
        .to_str()?
        .strip_prefix(RUN_LOG_PREFIX)?
        .strip_suffix(RUN_LOG_SUFFIX)?
        .rsplit(PID_SEPARATOR)
        .next()?
        .parse()
        .ok()
}

/// The end of a log, which is as much of it as a counter can be in.
///
/// Reading the whole file would mean re-reading megabytes several times
/// a second: cargo redraws its bar continuously, so the last counter is
/// always within the last few hundred bytes of compiler output, and the
/// window is sized to survive a burst of diagnostics between redraws.
fn tail(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(RUN_LOG_TAIL_BYTES)))
        .ok()?;
    let mut captured = Vec::new();
    file.read_to_end(&mut captured).ok()?;
    Some(String::from_utf8_lossy(&captured).into_owned())
}

/// The last counter in `tail`, which is the most recent redraw of the
/// bar.
///
/// Last rather than first because a run draws more than one bar: cargo
/// counts downloads before it counts compilations, and each nested cargo
/// a command drives counts its own. The one at the end is the one
/// happening now.
fn parse_counter(tail: &str) -> Option<Progress> {
    tail.rmatch_indices(UNIT_COUNTER_LEAD)
        .find_map(|(index, lead)| counter_at(tail.get(index.saturating_add(lead.len())..)?))
}

/// Read `<done>/<total>:` off the front of `text`.
///
/// The trailing colon is what separates cargo's counter from anything
/// else that pairs two numbers with a slash -- a test runner's tally,
/// most of all, which writes the same two numbers with a space after
/// them.
fn counter_at(text: &str) -> Option<Progress> {
    let (done, text) = leading_number(text)?;
    let (total, text) = leading_number(text.strip_prefix(UNIT_COUNTER_SEPARATOR)?)?;
    (total > 0 && text.starts_with(UNIT_COUNTER_TRAILER)).then_some(Progress { done, total })
}

/// The digits `text` opens with, and what follows them.
fn leading_number(text: &str) -> Option<(usize, &str)> {
    let end = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    Some((text.get(..end)?.parse().ok()?, text.get(end..)?))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;

    /// A redraw of cargo's bar as the shim captures it, colour codes and
    /// carriage return included.
    const CAPTURED_REDRAW: &str = "\u{1b}[1m\u{1b}[92m    Building\u{1b}[0m \
         [========>                ] 149/403: globset, regex-automata\r";

    /// A capture directory holding one log per run named, and a live
    /// marker for each pid in `live`.
    fn capture_root(runs: &[(u32, &str)], live: &[u32]) -> TempDir {
        let root = tempdir().unwrap();
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).unwrap();
        for (pid, output) in runs {
            let name =
                format!("{RUN_LOG_PREFIX}20260822-101500{PID_SEPARATOR}{pid}{RUN_LOG_SUFFIX}");
            fs::write(root.path().join(name), output).unwrap();
        }
        for pid in live {
            fs::write(markers.join(pid.to_string()), "").unwrap();
        }
        root
    }

    #[test]
    fn a_live_run_reports_what_its_log_last_captured() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);

        assert_eq!(
            Capture::take_from(root.path()).read(33395),
            Some(Progress {
                done:  149,
                total: 403,
            })
        );
    }

    /// Logs outlive the runs that wrote them, so a pid reused by a later
    /// process must not pick up the finished run's log.
    #[test]
    fn a_finished_runs_log_is_not_read_once_its_marker_is_gone() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);

        assert_eq!(Capture::take_from(root.path()).read(33395), None);
    }

    #[test]
    fn each_live_run_reports_its_own_log_and_an_uncaptured_pid_reports_none() {
        let other = "\u{1b}[1m    Building\u{1b}[0m [==>    ] 12/48: serde\r";
        let root = capture_root(&[(33395, CAPTURED_REDRAW), (33396, other)], &[33395, 33396]);
        let capture = Capture::take_from(root.path());

        assert_eq!(capture.read(33395).map(Progress::percent), Some(36));
        assert_eq!(capture.read(33396).map(Progress::percent), Some(25));
        assert_eq!(capture.read(70001), None);
    }

    /// The ordinary state of a machine that never switched capture on.
    #[test]
    fn a_missing_capture_directory_reports_nothing_rather_than_failing() {
        let root = tempdir().unwrap();

        assert_eq!(
            Capture::take_from(&root.path().join("never-created")).read(33395),
            None
        );
    }

    #[test]
    fn a_captured_redraw_reports_cargos_own_counts() {
        assert_eq!(
            parse_counter(CAPTURED_REDRAW),
            Some(Progress {
                done:  149,
                total: 403,
            })
        );
    }

    #[test]
    fn the_last_redraw_in_the_tail_is_the_one_reported() {
        let tail = format!("{CAPTURED_REDRAW}Building [=>] 7/9: serde\r");
        assert_eq!(parse_counter(&tail), Some(Progress { done: 7, total: 9 }));
    }

    #[test]
    fn a_download_counter_reports_the_phase_that_is_running() {
        assert_eq!(
            parse_counter("Downloading [==>    ] 12/40: serde, regex\r"),
            Some(Progress {
                done:  12,
                total: 40,
            })
        );
    }

    #[test]
    fn a_test_runners_tally_is_not_a_counter() {
        assert_eq!(parse_counter("PASS [   0.012s] 12/345 crate::suite"), None);
    }

    #[test]
    fn output_with_no_bar_in_it_reports_nothing() {
        assert_eq!(parse_counter("   Compiling serde v1.0.0\n"), None);
    }

    #[test]
    fn a_counter_over_zero_units_is_rejected_rather_than_divided_by() {
        assert_eq!(parse_counter("Building [ ] 0/0: \r"), None);
    }

    #[test]
    fn percent_rounds_down_so_only_a_finished_build_reads_full() {
        assert_eq!(
            Progress {
                done:  402,
                total: 403,
            }
            .percent(),
            99
        );
        assert_eq!(
            Progress {
                done:  403,
                total: 403,
            }
            .percent(),
            100
        );
    }

    #[test]
    fn a_log_file_is_keyed_by_the_shim_pid_its_name_ends_with() {
        assert_eq!(
            log_pid(Path::new("/tmp/cargo-tile/run-20260820-191029-33395.log")),
            Some(33395)
        );
        assert_eq!(log_pid(Path::new("/tmp/cargo-tile/pane-errors.log")), None);
    }
}
