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
//! run as `<root>/state/pids/<pid>.<timestamp>` for as long as it lives -- the pid
//! in both being the shim's own, which is an ancestor of the cargo
//! process the grid draws.
//!
//! A command that runs tests counts twice over. `cargo nextest run`
//! compiles first, under cargo's own bar, and then works through the
//! tests under a bar of its own -- `Running [ 00:00:03] ███▏ 22/24: 2
//! running, 22 passed` -- which is the same `done/total` pair, with
//! the drawn bar standing between the bracket and the counter rather
//! than in front of it. Both are read, and which of them
//! the reading came from is what the display names the phase by.
//!
//! A runner with nowhere to draw a bar still counts. Nextest draws one
//! only on a terminal, so a run started by a script or an agent -- and
//! that is most of the test runs on this machine -- has none, and the
//! count goes inline into every line it prints instead: `PASS [
//! 1.014s] (11/24) nxprobe t18`. That tally is the same reading in
//! parentheses, and reading it is what keeps those runs from sitting
//! at whatever the build last said for as long as the tests take.
//!
//! So a run reports progress when it was captured and reports none when
//! it was not, and the cell is drawn either way.
//!
//! A log says one other thing worth reading. Cargo locks the build
//! directory, so a second command against the same target waits instead
//! of failing -- it prints `Blocking waiting for file lock on build
//! directory` and then nothing, which from outside looks exactly like a
//! build that has not reached its first unit.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::env;
use std::path::Path;
use std::path::PathBuf;

use crate::capture_root::Enumeration;
use crate::capture_root::RootHistory;
use crate::capture_root::RootScan;
use crate::capture_root::SweepBudget;
use crate::capture_root::SweepDisposition;
use crate::constants::BAR_GLYPH_FIRST;
use crate::constants::BAR_GLYPH_LAST;
use crate::constants::BUILD_FINISHED_MARKER;
use crate::constants::CAPTURE_ROOT;
use crate::constants::CAPTURE_ROOT_ENV;
use crate::constants::LOCK_WAIT_MARKER;
use crate::constants::PHASE_BUILDING;
use crate::constants::PHASE_TESTING;
use crate::constants::PID_SEPARATOR;
use crate::constants::REGISTRATION_SEPARATOR;
use crate::constants::REGISTRATION_TEMP_SUFFIX;
use crate::constants::RUN_LOG_PREFIX;
use crate::constants::RUN_LOG_SUFFIX;
use crate::constants::TALLY_CLOSE;
use crate::constants::TALLY_OPEN;
use crate::constants::TEST_PHASE_MARKER;
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

    /// The same reading in tenths of a percent, rounded down the same
    /// way, so only a finished build reaches 1000.
    pub(crate) const fn percent_tenths(self) -> usize {
        self.done.saturating_mul(1000) / self.total
    }
}

/// Which counter a reading came from, which is the whole of what the
/// numbers themselves say about what the run is doing.
///
/// A command that only ever compiles stays in one of these for its
/// whole life; `cargo nextest run` passes through both, and the two
/// counters are unrelated -- the second opens at nought over the tests
/// collected the moment the first reaches its total.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    /// Cargo compiling the units of its build plan.
    Building,
    /// A test runner working through the tests it collected.
    Testing,
}

impl Phase {
    /// The word a working-directory header names this phase with.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Building => PHASE_BUILDING,
            Self::Testing => PHASE_TESTING,
        }
    }
}

/// What a captured run was doing when its log was last read.
///
/// Cargo takes a lock on the build directory, so a second command in
/// the same target waits rather than fails. It says so once and then
/// prints nothing at all, which from outside is indistinguishable from
/// a build that has not reached its first unit -- the same pid, the
/// same climbing duration, and no reading either way. Reading the wait
/// out of the log is what separates them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunState {
    /// Getting somewhere, as far along as the counter of the phase it
    /// is in says.
    Working {
        /// Which counter the reading came from.
        phase:    Phase,
        /// What that counter last said.
        progress: Progress,
    },
    /// Waiting for another cargo to give up the build directory.
    Blocked,
}

impl RunState {
    /// The reading and the phase it belongs to, for the one place that
    /// draws either: the rule along a working-directory header.
    pub(crate) const fn working(self) -> Option<(Phase, Progress)> {
        match self {
            Self::Working { phase, progress } => Some((phase, progress)),
            Self::Blocked => None,
        }
    }
}

/// Whether the process a registered run was captured under is still
/// there.
///
/// The shim clears its own file out of `<root>/state/pids` as it
/// exits, but a run killed outright never reaches that, and the file
/// then stands for a process that is gone. [`Capture::take`] is handed
/// this so a finished run stops counting as one in flight: which pids
/// are running is something [`crate::processes`] has just read off the
/// process table, so the answer is already to hand where it is asked
/// for.
pub(crate) enum RunLiveness {
    /// The pid is in the process table the caller read.
    Running,
    /// The pid is not, so the file registering it is stale and
    /// [`live_runs`] clears it away.
    Ended,
}

impl From<bool> for RunLiveness {
    fn from(running: bool) -> Self { if running { Self::Running } else { Self::Ended } }
}

/// How precisely a registration names the log belonging to its run.
#[derive(Debug, Eq, Hash, PartialEq)]
enum RegistrationGeneration {
    /// Older shims register only a pid, so their newest log wins.
    Legacy,
    /// The calendar stamp shared by a versioned registration and its log.
    Calendar(String),
}

impl RegistrationGeneration {
    /// Whether this registration explicitly names this log, even when
    /// a clock step makes it sort before another log under the same pid.
    fn names_log(&self, path: &Path, pid: u32) -> bool {
        match self {
            Self::Legacy => false,
            Self::Calendar(generation) => path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix(RUN_LOG_PREFIX))
                .and_then(|name| name.strip_suffix(RUN_LOG_SUFFIX))
                .and_then(|name| name.rsplit_once(PID_SEPARATOR))
                .is_some_and(|(stamp, named_pid)| {
                    stamp == generation
                        && named_pid.bytes().all(|byte| byte.is_ascii_digit())
                        && (named_pid == "0" || !named_pid.starts_with('0'))
                        && named_pid.parse() == Ok(pid)
                }),
        }
    }
}

thread_local! {
    /// The process scanner constructs a fresh `Capture` each pass, so root
    /// identity history belongs to its worker thread rather than that snapshot.
    static ROOT_HISTORY: RefCell<RootHistory> = RefCell::default();
}

/// Successful live registrations remain usable even when missing evidence
/// prevents this scan from proving that other runs have ended.
struct LiveRegistrations {
    /// Each readable registration whose pid is present in the process table.
    generations: HashMap<u32, HashSet<RegistrationGeneration>>,
    /// Cleanup requires every published registration to have been readable.
    evidence:    RegistrationEvidence,
}

/// Whether the registration evidence can justify treating unlisted runs as ended.
#[derive(Debug, Eq, PartialEq)]
enum RegistrationEvidence {
    /// Every published registration was sampled and read within the byte cap.
    Complete,
    /// Missing or unreadable entries prohibit deletion, but retain sibling readings.
    Incomplete,
}

/// Filename classification only; calendar text never verifies process identity.
enum RegistrationName<'name> {
    /// Older shims publish only their pid.
    Legacy(u32),
    /// A published filename also supplies a candidate log generation.
    Calendar {
        /// The shim pid preceding the first separator.
        pid:        u32,
        /// The nonempty suffix, accepted without claiming a birth stamp.
        generation: &'name str,
    },
    /// An unpublished record consumes inspection budget but cannot be removed.
    Staging,
    /// Unrelated or malformed names do not establish liveness.
    Unrelated,
}

/// Progress readings from one completed capture scan, keyed by shim pid.
/// Reads happen through that scan's directory handles; subsequent lookups
/// return these values without reopening any configured pathname.
#[derive(Default)]
pub(crate) struct Capture {
    /// The last parseable state for each selected live log in this scan.
    readings: HashMap<u32, RunState>,
}

impl Capture {
    /// Sample the capture directory before reading registrations and liveness.
    /// A log arriving after that sample is never eligible for this pass's sweep.
    /// The next scan opens the configured root again and checks its identity.
    /// `RootScan::open` materializes the bounded inventory before registration
    /// enumeration; opening a lazy iterator alone would not preserve that order.
    pub(crate) fn take(liveness: impl Fn(u32) -> RunLiveness) -> Self {
        Self::take_from(&root(), liveness)
    }

    /// Scan a supplied capture directory without changing the process environment.
    pub(crate) fn take_from(root: &Path, liveness: impl Fn(u32) -> RunLiveness) -> Self {
        Self::take_roots(&[root], &liveness)
    }

    /// One allowance covers every owned root and artifact kind in this pass.
    /// Identity history persists on the scanner thread; the budget never does.
    fn take_roots(roots: &[&Path], liveness: &impl Fn(u32) -> RunLiveness) -> Self {
        let mut capture = Self::default();
        let mut budget = SweepBudget::default();
        for root in roots {
            let Ok(scan) = ROOT_HISTORY.with_borrow_mut(|history| RootScan::open(root, history))
            else {
                continue;
            };
            capture.scan_root(&scan, liveness, &mut budget);
        }
        capture
    }

    /// Select log basenames from the bounded sample before any cleanup.
    /// Missing evidence prevents cleanup without discarding readable live runs.
    fn scan_root(
        &mut self,
        scan: &RootScan,
        liveness: &impl Fn(u32) -> RunLiveness,
        budget: &mut SweepBudget,
    ) {
        let LiveRegistrations {
            generations: live,
            evidence,
        } = live_runs(scan, liveness);
        let mut logs: HashMap<u32, &Path> = HashMap::new();
        for entry in scan.log_entries() {
            let path = entry.name();
            let Some(pid) = log_pid(path) else {
                continue;
            };
            let Some(generations) = live.get(&pid) else {
                continue;
            };
            if !generations.contains(&RegistrationGeneration::Legacy)
                && !generations
                    .iter()
                    .any(|generation| generation.names_log(path, pid))
            {
                continue;
            }
            match logs.entry(pid) {
                Entry::Vacant(slot) => {
                    slot.insert(path);
                },
                Entry::Occupied(mut held) if newer(path, held.get()) => {
                    held.insert(path);
                },
                Entry::Occupied(_) => {},
            }
        }
        for entry in scan.log_entries() {
            let Some(pid) = log_pid(entry.name()) else {
                continue;
            };
            if logs.get(&pid) == Some(&entry.name())
                && let Ok(tail) = entry.read_log()
                && let Some(state) = parse_state(&tail)
            {
                self.readings.entry(pid).or_insert(state);
            }
        }
        if evidence == RegistrationEvidence::Incomplete {
            return;
        }
        scan.sweep(
            budget,
            |path| match registration_name(path) {
                RegistrationName::Legacy(pid) | RegistrationName::Calendar { pid, .. }
                    if !live.contains_key(&pid) =>
                {
                    SweepDisposition::Remove
                },
                RegistrationName::Staging => SweepDisposition::Staging,
                _ => SweepDisposition::Preserve,
            },
            |path| Self::discard(path, &logs, &live),
        );
    }

    /// Classify an inventory basename; only the access layer's owned capability
    /// can act on this decision, charging the same budget as registrations.
    fn discard(
        path: &Path,
        logs: &HashMap<u32, &Path>,
        live: &HashMap<u32, HashSet<RegistrationGeneration>>,
    ) -> SweepDisposition {
        let Some(pid) = log_pid(path) else {
            return SweepDisposition::Preserve;
        };
        if logs.get(&pid) == Some(&path)
            || live.get(&pid).is_some_and(|generations| {
                generations
                    .iter()
                    .any(|generation| generation.names_log(path, pid))
            })
        {
            SweepDisposition::Preserve
        } else {
            SweepDisposition::Remove
        }
    }

    /// Return the state read during this scan, with no later filesystem access.
    pub(crate) fn read(&self, pid: u32) -> Option<RunState> { self.readings.get(&pid).copied() }
}

/// Resolve the shim's configured root without canonicalizing away the pathname
/// the access layer must reopen and compare on every scan.
fn root() -> PathBuf {
    env::var_os(CAPTURE_ROOT_ENV).map_or_else(|| PathBuf::from(CAPTURE_ROOT), PathBuf::from)
}

/// Read registrations after the bounded log sample, retaining successful live
/// entries independently of evidence completeness. The shim removes logs and
/// registrations on normal exit; runs killed outright can leave both artifacts
/// behind until a later sweep.
fn live_runs(scan: &RootScan, liveness: &impl Fn(u32) -> RunLiveness) -> LiveRegistrations {
    let mut evidence = match scan.registration_outcome() {
        Enumeration::Complete => RegistrationEvidence::Complete,
        Enumeration::Incomplete | Enumeration::Failed(_) => RegistrationEvidence::Incomplete,
    };
    let mut live: HashMap<u32, HashSet<RegistrationGeneration>> = HashMap::new();
    for entry in scan.registration_entries() {
        let (pid, generation) = match registration_name(entry.name()) {
            RegistrationName::Legacy(pid) => (pid, RegistrationGeneration::Legacy),
            RegistrationName::Calendar { pid, generation } => {
                (pid, RegistrationGeneration::Calendar(generation.to_owned()))
            },
            RegistrationName::Staging | RegistrationName::Unrelated => continue,
        };
        if entry.read_registration().is_err() {
            evidence = RegistrationEvidence::Incomplete;
            continue;
        }
        if matches!(liveness(pid), RunLiveness::Running) {
            live.entry(pid).or_default().insert(generation);
        }
    }
    LiveRegistrations {
        generations: live,
        evidence,
    }
}

/// Accept legacy pids and any nonempty generation suffix without interpreting
/// the suffix as process identity. Staging names never become registrations.
fn registration_name(path: &Path) -> RegistrationName<'_> {
    let Some(name) = path.to_str() else {
        return RegistrationName::Unrelated;
    };
    if name.ends_with(REGISTRATION_TEMP_SUFFIX) {
        return RegistrationName::Staging;
    }
    let (pid, generation) = name
        .split_once(REGISTRATION_SEPARATOR)
        .unwrap_or((name, ""));
    if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return RegistrationName::Unrelated;
    }
    let Ok(pid) = pid.parse() else {
        return RegistrationName::Unrelated;
    };
    if !generation.is_empty() {
        RegistrationName::Calendar { pid, generation }
    } else if name.contains(REGISTRATION_SEPARATOR) {
        RegistrationName::Unrelated
    } else {
        RegistrationName::Legacy(pid)
    }
}

/// Whether `candidate` was captured later than `held`, which their
/// names settle: a log is named for the instant it opened, in a
/// zero-padded stamp ahead of the pid, so between two names ending in
/// the same pid the later one sorts higher.
///
/// The shim deletes its log on exit, including cancellation through
/// SIGTERM. A run killed outright can leave a log behind, and a later
/// process can reuse its pid. Legacy registrations carry no generation,
/// so the newest filename is their fallback; a filename alone does not
/// verify which process wrote it.
fn newer(candidate: &Path, held: &Path) -> bool { candidate.file_name() > held.file_name() }

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

/// What the end of a log says the run is doing now.
///
/// Neither marker means anything by itself: both are printed once and
/// then stay in the log for as long as the run does. What settles each
/// is what came after it.
///
/// The wait is proof the run waited, never that it still is -- a bar, a
/// `Finished`, or the output of the binary a `cargo run` went on to
/// start each say the lock came free. A run that is still waiting has
/// written the line and then nothing, which is exactly what being
/// blocked looks like from outside.
///
/// A counter is proof the run was building, never that it still is: the
/// bar is left on screen where it stopped, so a build that finished at
/// `1/2` goes on reading 50% for as long as the process lives -- which
/// for a `cargo run` is the whole life of the app it started. Cargo's
/// own `Finished` past the counter is what retires it, and a counter
/// past *that* is a test runner's, which has every right to the column.
fn parse_state(tail: &str) -> Option<RunState> {
    if still_waiting(tail) {
        return Some(RunState::Blocked);
    }
    let (at, state) = last_counter(tail)?;
    tail.rfind(BUILD_FINISHED_MARKER)
        .is_none_or(|over| at > over)
        .then_some(state)
}

/// Whether the log ends on the wait line.
///
/// The line itself is the one line the trailing text is allowed to
/// hold. Cargo redraws its bar over carriage returns rather than
/// newlines, so a redraw that followed the wait counts as the second
/// line here just as a `Finished` would.
fn still_waiting(tail: &str) -> bool {
    tail.rfind(LOCK_WAIT_MARKER)
        .and_then(|at| tail.get(at..))
        .is_some_and(|after| after.trim_end().lines().count() == 1)
}

/// The last counter in `tail` and where it sits, which is the most
/// recent redraw of the bar.
///
/// Last rather than first because a run draws more than one bar: cargo
/// counts downloads before it counts compilations, each nested cargo a
/// command drives counts its own, and a test runner counts the tests
/// once the compiling is over. The one at the end is the one happening
/// now.
///
/// Where it sits is what says whether it still stands: a bar is left
/// on screen where it stopped, so the last redraw of a finished build
/// reads no differently from the last redraw of a running one.
fn last_counter(tail: &str) -> Option<(usize, RunState)> {
    tail.rmatch_indices(UNIT_COUNTER_LEAD)
        .find_map(|(index, lead)| {
            let after = index.saturating_add(lead.len());
            let (counter, progress) = counter_at(tail.get(after..)?)?;
            Some((
                index,
                RunState::Working {
                    phase: counter.phase(tail, index),
                    progress,
                },
            ))
        })
}

/// Which of the two counters a reading came off, which is half of
/// what says whose counter it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Counter {
    /// Drawn in a bar and closed by a colon, `149/403:`. Cargo draws
    /// one and so does a test runner, so the line it sits on is what
    /// separates them.
    Bar,
    /// A test runner's per-test tally, `(11/24)`. Nothing else writes
    /// one, so the parentheses alone settle it.
    Tally,
}

impl Counter {
    /// Which phase a counter of this kind at `index` belongs to.
    ///
    /// A tally answers for itself. A bar is read off the status word
    /// opening the line it was drawn on -- cargo says `Building`, a
    /// test runner says `Running` -- and only that line is searched: a
    /// log holds the word many times over, cargo saying `Running` of
    /// every test binary a plain `cargo test` starts, and none of those
    /// lines carries a counter.
    fn phase(self, tail: &str, index: usize) -> Phase {
        let Self::Bar = self else {
            return Phase::Testing;
        };
        let opens = tail
            .get(..index)
            .and_then(|ahead| ahead.rfind(['\n', '\r']))
            .map_or(0, |at| at.saturating_add(1));
        if tail
            .get(opens..index)
            .is_some_and(|line| line.contains(TEST_PHASE_MARKER))
        {
            Phase::Testing
        } else {
            Phase::Building
        }
    }
}

/// Read a counter off the front of `text`, past a drawn bar where one
/// stands in the way.
///
/// Cargo puts its counter straight after the bracket that closes its
/// bar. A test runner brackets its elapsed time instead and draws the
/// bar after it, so the counter is reached across the blocks the bar is
/// filled with and the blanks it is padded to width with -- and where
/// it has no bar at all, it brackets each test's own duration and puts
/// the count in parentheses beyond that.
///
/// How the two numbers are closed is what tells a counter from anything
/// else that pairs numbers with a slash: a colon for a bar, a
/// parenthesis for a tally, and a pair closed by neither is not a
/// counter at all.
fn counter_at(text: &str) -> Option<(Counter, Progress)> {
    let text = text.trim_start_matches(bar_fill);
    let (counter, text) = text
        .strip_prefix(TALLY_OPEN)
        .map_or((Counter::Bar, text), |inside| (Counter::Tally, inside));
    let (done, text) = leading_number(text.trim_start_matches(bar_fill))?;
    let (total, text) = leading_number(text.strip_prefix(UNIT_COUNTER_SEPARATOR)?)?;
    let closed = match counter {
        Counter::Bar => text.starts_with(UNIT_COUNTER_TRAILER),
        Counter::Tally => text.starts_with(TALLY_CLOSE),
    };
    (total > 0 && closed).then_some((counter, Progress { done, total }))
}

/// Whether `character` is part of a drawn bar rather than of the
/// counter beyond it.
fn bar_fill(character: char) -> bool {
    character == ' ' || (BAR_GLYPH_FIRST..=BAR_GLYPH_LAST).contains(&character)
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
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::constants::CAPTURE_INVENTORY_LIMIT;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_REGISTRATION_BYTES;
    use crate::constants::CAPTURE_SWEEP_LIMIT;

    /// The line cargo prints when another cargo holds the build
    /// directory, as it comes out of a real 1.96 run.
    const CAPTURED_WAIT: &str = "    Blocking waiting for file lock on build directory\n";

    /// The state a run reports while it is compiling `done` of `total`.
    fn compiling(done: usize, total: usize) -> RunState {
        RunState::Working {
            phase:    Phase::Building,
            progress: Progress { done, total },
        }
    }

    /// The state a run reports while it works through `done` of `total`
    /// tests.
    fn testing(done: usize, total: usize) -> RunState {
        RunState::Working {
            phase:    Phase::Testing,
            progress: Progress { done, total },
        }
    }

    /// A redraw of cargo's bar as the shim captures it, colour codes and
    /// carriage return included.
    const CAPTURED_REDRAW: &str = "\u{1b}[1m\u{1b}[92m    Building\u{1b}[0m \
         [========>                ] 149/403: globset, regex-automata\r";

    /// A redraw of nextest's bar as the shim captures it: the elapsed
    /// time bracketed, the drawn bar after it, and the counter past
    /// that.
    const CAPTURED_TEST_REDRAW: &str = "\u{1b}[32;1m     Running\u{1b}[0m \
         [ 00:00:01] \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{258b}      \
         12/24: \u{1b}[1m2\u{1b}[0m running, \u{1b}[1m12\u{1b}[0m passed\r\n";

    /// A tally as nextest writes it where it has no bar to put the
    /// count in, which is every run whose output is not a terminal.
    /// Left-padded to the width of the total, so a run of a thousand
    /// tests opens with three blanks inside the parenthesis.
    const CAPTURED_TALLY: &str = "        PASS [   1.014s] (11/24) nxprobe t18\n";

    /// The line nextest prints under its bar for each test in flight,
    /// which is what stands between the bar and the end of the log.
    const CAPTURED_TEST_ROW: &str =
        "             [ 00:00:00] \u{1b}[35;1mnxprobe\u{1b}[0m \u{1b}[34;1mt18\u{1b}[0m\r\n";

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

    /// Inspect the available live set through a fresh scan of a test fixture.
    fn registered_runs(
        root: &Path,
        liveness: impl Fn(u32) -> RunLiveness,
    ) -> HashMap<u32, HashSet<RegistrationGeneration>> {
        let scan = RootScan::open(root, &mut RootHistory::default()).unwrap();
        let live = live_runs(&scan, &liveness);
        assert_eq!(live.evidence, RegistrationEvidence::Complete);
        live.generations
    }

    #[test]
    fn a_live_run_reports_what_its_log_last_captured() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Running).read(33395),
            Some(compiling(149, 403))
        );
    }

    /// A missing live set is not evidence that every sampled log is dead.
    #[test]
    fn unavailable_registrations_preserve_sampled_logs() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);
        fs::remove_dir(root.path().join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let log = root.path().join("run-20260822-101500-33395.log");

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Ended).read(33395),
            None
        );
        assert!(log.exists());
    }

    /// An empty successful inventory and a missing directory carry different evidence.
    #[test]
    fn unavailable_and_empty_live_sets_remain_distinct() {
        let root = capture_root(&[], &[]);
        let mut history = RootHistory::default();
        let empty = RootScan::open(root.path(), &mut history).unwrap();
        assert!(matches!(
            live_runs(&empty, &|_| RunLiveness::Ended),
            LiveRegistrations {
                generations,
                evidence: RegistrationEvidence::Complete,
            } if generations.is_empty()
        ));
        fs::remove_dir(root.path().join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let missing = RootScan::open(root.path(), &mut history).unwrap();

        assert!(matches!(
            live_runs(&missing, &|_| RunLiveness::Ended),
            LiveRegistrations {
                generations,
                evidence: RegistrationEvidence::Incomplete,
            } if generations.is_empty()
        ));
    }

    /// Root identity survives `Capture` temporaries, so replacement cannot sweep
    /// on its first observation through the configured pathname.
    #[test]
    fn replacing_the_root_suppresses_its_first_sweep() {
        let parent = tempdir().unwrap();
        let configured = parent.path().join("capture");
        fs::create_dir_all(configured.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        Capture::take_from(&configured, |_| RunLiveness::Ended);
        fs::rename(&configured, parent.path().join("previous")).unwrap();
        fs::create_dir_all(configured.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let log = configured.join("run-20260822-101500-33395.log");
        fs::write(&log, CAPTURED_REDRAW).unwrap();

        Capture::take_from(&configured, |_| RunLiveness::Ended);

        assert!(log.exists());

        Capture::take_from(&configured, |_| RunLiveness::Ended);

        assert!(!log.exists());
    }

    /// A failed open invalidates the previous observation even if the same
    /// inode later returns to the configured pathname.
    #[test]
    fn access_recovery_requires_one_unswept_observation() {
        let parent = tempdir().unwrap();
        let configured = parent.path().join("capture");
        let displaced = parent.path().join("displaced");
        fs::create_dir_all(configured.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        Capture::take_from(&configured, |_| RunLiveness::Ended);
        fs::rename(&configured, &displaced).unwrap();
        assert_eq!(
            Capture::take_from(&configured, |_| RunLiveness::Ended).read(33395),
            None
        );
        fs::rename(&displaced, &configured).unwrap();
        let log = configured.join("run-20260822-101500-33395.log");
        fs::write(&log, CAPTURED_REDRAW).unwrap();

        Capture::take_from(&configured, |_| RunLiveness::Ended);

        assert!(log.exists());

        Capture::take_from(&configured, |_| RunLiveness::Ended);

        assert!(!log.exists());
    }

    /// Refusing a redirected registration directory cannot become an empty live set.
    #[test]
    fn a_symlinked_registration_directory_preserves_sampled_logs() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        let destination = tempdir().unwrap();
        fs::remove_dir(&registrations).unwrap();
        symlink(destination.path(), &registrations).unwrap();

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Ended).read(33395),
            None
        );
        assert!(root.path().join("run-20260822-101500-33395.log").exists());
    }

    /// An unrelated account can replace directory contents wherever group write is allowed.
    #[test]
    fn group_writable_registration_directories_disable_all_cleanup() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::set_permissions(&registrations, fs::Permissions::from_mode(0o770)).unwrap();

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        assert!(registrations.join("33395").exists());
        assert!(root.path().join("run-20260822-101500-33395.log").exists());
    }

    /// Entry reads can fail even when enumeration succeeds; that uncertainty protects logs.
    #[test]
    fn an_unreadable_registration_disables_cleanup() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("33395");
        fs::remove_file(&registration).unwrap();
        fs::create_dir(&registration).unwrap();

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Ended).read(33395),
            None
        );
        assert!(registration.is_dir());
        assert!(root.path().join("run-20260822-101500-33395.log").exists());
    }

    /// Normal shim exit can remove a sampled registration before its read.
    /// Readable siblings on both sides of that failure still report progress.
    #[test]
    fn a_disappearing_registration_preserves_sibling_readings_and_disables_cleanup() {
        let root = capture_root(
            &[
                (33395, CAPTURED_REDRAW),
                (33396, CAPTURED_REDRAW),
                (33397, CAPTURED_REDRAW),
                (33398, CAPTURED_TALLY),
            ],
            &[33395, 33396, 33397],
        );
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        let sampled: Vec<u32> = scan
            .registration_entries()
            .map(|entry| entry.name().to_str().unwrap().parse().unwrap())
            .collect();
        assert_eq!(sampled.len(), 3);
        assert!(matches!(scan.log_outcome(), Enumeration::Complete));
        assert!(matches!(scan.registration_outcome(), Enumeration::Complete));
        fs::remove_file(registrations.join(sampled[1].to_string())).unwrap();
        let mut capture = Capture::default();
        let mut budget = SweepBudget::default();

        capture.scan_root(&scan, &|_| RunLiveness::Running, &mut budget);

        assert_eq!(capture.read(sampled[0]), Some(compiling(149, 403)));
        assert_eq!(capture.read(sampled[1]), None);
        assert_eq!(capture.read(sampled[2]), Some(compiling(149, 403)));
        assert_eq!(capture.read(33398), None);
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 5);
        assert_eq!(fs::read_dir(&registrations).unwrap().count(), 2);
        assert!(root.path().join("run-20260822-101500-33398.log").exists());
    }

    /// The registration cap is enforced before a filename can authorize cleanup.
    #[test]
    fn an_oversized_registration_disables_cleanup() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("33395");
        let bytes = vec![b'x'; usize::try_from(CAPTURE_REGISTRATION_BYTES).unwrap() + 1];
        fs::write(&registration, bytes).unwrap();

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Ended).read(33395),
            None
        );
        assert!(registration.exists());
        assert!(root.path().join("run-20260822-101500-33395.log").exists());
    }

    /// Unrelated directory entries consume inventory capacity too; reaching the
    /// bound is an explicit incomplete sample that authorizes no deletions.
    #[test]
    fn an_incomplete_log_inventory_sweeps_nothing() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        for index in 0..CAPTURE_INVENTORY_LIMIT {
            fs::write(root.path().join(format!("unrelated-{index}")), "").unwrap();
        }
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        assert!(matches!(scan.log_outcome(), Enumeration::Incomplete));
        assert!(scan.log_entries().count() <= CAPTURE_INVENTORY_LIMIT);

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            CAPTURE_INVENTORY_LIMIT + 2
        );
        assert!(
            root.path()
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join("33395")
                .exists()
        );
    }

    /// Every sampled live log carries the same reading, regardless of directory
    /// order. The inventory cap must not hide that reading or retire stale files.
    #[test]
    fn an_incomplete_log_inventory_keeps_sampled_live_readings() {
        let root = capture_root(&[(33396, CAPTURED_TALLY)], &[33395, 33396]);
        for index in 0..CAPTURE_INVENTORY_LIMIT {
            let name =
                format!("{RUN_LOG_PREFIX}sample-{index}{PID_SEPARATOR}33395{RUN_LOG_SUFFIX}");
            fs::write(root.path().join(name), CAPTURED_REDRAW).unwrap();
        }
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        assert!(matches!(scan.log_outcome(), Enumeration::Incomplete));
        assert!(matches!(scan.registration_outcome(), Enumeration::Complete));
        assert!(scan.log_entries().count() <= CAPTURE_INVENTORY_LIMIT);
        assert!(
            scan.log_entries()
                .any(|entry| log_pid(entry.name()) == Some(33395))
        );
        let mut capture = Capture::default();
        let mut budget = SweepBudget::default();

        capture.scan_root(&scan, &|pid| RunLiveness::from(pid == 33395), &mut budget);

        assert_eq!(capture.read(33395), Some(compiling(149, 403)));
        assert_eq!(capture.read(33396), None);
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            CAPTURE_INVENTORY_LIMIT + 2
        );
        assert_eq!(
            fs::read_dir(root.path().join(CAPTURE_LIVE_RUNS_DIR))
                .unwrap()
                .count(),
            2
        );
        assert!(root.path().join("run-20260822-101500-33396.log").exists());
    }

    /// A partial registration inventory cannot classify unsampled generations as absent.
    #[test]
    fn an_incomplete_registration_inventory_sweeps_nothing() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        for index in 0..CAPTURE_INVENTORY_LIMIT {
            fs::write(registrations.join(format!("unrelated-{index}")), "").unwrap();
        }
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        assert!(matches!(
            scan.registration_outcome(),
            Enumeration::Incomplete
        ));
        assert!(matches!(
            live_runs(&scan, &|_| RunLiveness::Ended),
            LiveRegistrations {
                generations,
                evidence: RegistrationEvidence::Incomplete,
            } if generations.is_empty()
        ));

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        assert!(root.path().join("run-20260822-101500-33395.log").exists());
        assert_eq!(
            fs::read_dir(registrations).unwrap().count(),
            CAPTURE_INVENTORY_LIMIT + 1
        );
    }

    /// Captured progress belongs to the completed scan, even when its pathname changes later.
    #[test]
    fn a_capture_keeps_its_reading_without_reopening_a_replaced_log() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let capture = Capture::take_from(root.path(), |_| RunLiveness::Running);
        let log = root.path().join("run-20260822-101500-33395.log");
        let unrelated = root.path().join("unrelated");
        fs::write(&unrelated, CAPTURED_TALLY).unwrap();
        fs::remove_file(&log).unwrap();
        symlink(&unrelated, &log).unwrap();

        assert_eq!(capture.read(33395), Some(compiling(149, 403)));
        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Running).read(33395),
            None
        );
    }

    /// A log that appears after this pass sampled the directory is not
    /// this pass's to judge, and the next pass will hold its
    /// registration. Sampling the other way round is what loses a run
    /// that publishes mid-scan: absent from the liveness snapshot, its
    /// log is swept as an orphan and cargo reopens it under the
    /// caller's umask, out of reach of the operator.
    #[test]
    fn a_log_arriving_after_the_directory_sample_survives_the_pass() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let arriving = root.path().join(format!(
            "{RUN_LOG_PREFIX}20260822-101500{PID_SEPARATOR}33396{RUN_LOG_SUFFIX}"
        ));

        let capture = Capture::take_from(root.path(), |pid| {
            if pid == 33395 {
                fs::write(&arriving, CAPTURED_TALLY).unwrap();
            }
            RunLiveness::Running
        });

        assert!(arriving.exists());
        assert_eq!(capture.read(33395), Some(compiling(149, 403)));
    }

    /// Refreshing the shim leaves older captured invocations running,
    /// so both filename formats must protect their logs in one scan.
    #[test]
    fn legacy_and_versioned_registrations_keep_both_live_logs() {
        let root = capture_root(
            &[(33395, CAPTURED_REDRAW), (33396, CAPTURED_TALLY)],
            &[33395],
        );
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        let generation = "20260822-101500";
        let registration = format!("33396{REGISTRATION_SEPARATOR}{generation}");
        fs::write(markers.join(&registration), "").unwrap();

        let live = registered_runs(root.path(), |_| RunLiveness::Running);
        assert_eq!(live.len(), 2);
        assert!(live[&33395].contains(&RegistrationGeneration::Legacy));
        assert!(live[&33396].contains(&RegistrationGeneration::Calendar(generation.to_owned())));
        let capture = Capture::take_from(root.path(), |_| RunLiveness::Running);
        assert_eq!(capture.read(33395), Some(compiling(149, 403)));
        assert_eq!(capture.read(33396), Some(testing(11, 24)));
        for pid in [33395, 33396] {
            assert!(
                root.path()
                    .join(format!(
                        "{RUN_LOG_PREFIX}{generation}{PID_SEPARATOR}{pid}{RUN_LOG_SUFFIX}"
                    ))
                    .exists()
            );
        }
        assert!(markers.join(registration).exists());
    }

    /// Generation suffixes classify filenames even when they are not calendar
    /// stamps; neither their spelling nor the log match verifies process identity.
    #[test]
    fn arbitrary_nonempty_generation_suffixes_still_name_candidate_logs() {
        let generation = RegistrationGeneration::Calendar("candidate.with-extra-text".to_owned());

        assert!(generation.names_log(Path::new("run-candidate.with-extra-text-33395.log"), 33395));
        assert!(
            !generation.names_log(Path::new("run-candidate.with-extra-text-033395.log"), 33395)
        );
        assert!(
            !generation.names_log(Path::new("run-candidate.with-extra-text-+33395.log"), 33395)
        );
        assert!(matches!(
            registration_name(Path::new("33395.candidate.with-extra-text")),
            RegistrationName::Calendar {
                pid:        33395,
                generation: "candidate.with-extra-text",
            }
        ));
    }

    /// Staging and malformed names are not registrations, even when
    /// their prefixes happen to name a live process.
    #[test]
    fn staging_files_and_invalid_pids_never_establish_liveness() {
        let root = capture_root(&[], &[]);
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        for name in [
            format!("33395{REGISTRATION_SEPARATOR}20260822-101500{REGISTRATION_TEMP_SUFFIX}"),
            format!(
                "33396{REGISTRATION_SEPARATOR}20260822-101500.random{REGISTRATION_TEMP_SUFFIX}"
            ),
            format!("33397{REGISTRATION_SEPARATOR}"),
            "+33398".to_owned(),
            "4294967296".to_owned(),
            "unrelated".to_owned(),
        ] {
            fs::write(markers.join(name), "").unwrap();
        }

        assert!(registered_runs(root.path(), |_| RunLiveness::Running).is_empty());
        assert_eq!(fs::read_dir(markers).unwrap().count(), 6);
    }

    /// A stepped-back clock must not make the live generation lose to
    /// a stale log whose calendar stamp sorts later.
    #[test]
    fn a_versioned_registration_selects_its_exact_log_generation() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::write(markers.join("33395.20260822-101500"), "").unwrap();
        let stale = root.path().join("run-20260823-101500-33395.log");
        fs::write(&stale, CAPTURED_TALLY).unwrap();

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Running).read(33395),
            Some(compiling(149, 403))
        );
        assert!(!stale.exists());
    }

    /// Until process identity is verified, more than one registration
    /// under a live pid is ambiguous; neither registered log is orphaned.
    #[test]
    fn multiple_live_generations_protect_every_registered_log() {
        let root = capture_root(&[], &[]);
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        let generations = ["20260822-101500", "20260823-101500"];
        for generation in generations {
            fs::write(
                markers.join(format!("33395{REGISTRATION_SEPARATOR}{generation}")),
                "",
            )
            .unwrap();
            fs::write(
                root.path().join(format!(
                    "{RUN_LOG_PREFIX}{generation}{PID_SEPARATOR}33395{RUN_LOG_SUFFIX}"
                )),
                CAPTURED_REDRAW,
            )
            .unwrap();
        }

        Capture::take_from(root.path(), |_| RunLiveness::Running);

        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 3);
    }

    /// Retirement uses the full enumerated name, so a versioned
    /// registration cannot accidentally remove the old pid-only file.
    #[test]
    fn an_ended_versioned_registration_is_removed_by_its_exact_name() {
        let root = capture_root(&[], &[]);
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        let registration = markers.join("33395.20260822-101500");
        let staging = markers.join(format!("33395.20260822-101500{REGISTRATION_TEMP_SUFFIX}"));
        fs::write(&registration, "").unwrap();
        fs::write(&staging, "").unwrap();

        Capture::take_from(root.path(), |_| RunLiveness::Ended);
        assert!(registered_runs(root.path(), |_| RunLiveness::Ended).is_empty());
        assert!(!registration.exists());
        assert!(staging.exists());
    }

    /// Logs outlive the runs that wrote them, so a pid reused by a later
    /// process must not pick up the finished run's log.
    #[test]
    fn a_finished_runs_log_is_not_read_once_its_marker_is_gone() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Running).read(33395),
            None
        );
    }

    /// Runs killed outright can leave logs behind after the shim loses
    /// its chance to clean up. A reused pid with a legacy registration
    /// chooses the newest filename, avoiding an older run's progress.
    #[test]
    fn a_pid_with_several_logs_reads_the_newest() {
        let root = tempdir().unwrap();
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).unwrap();
        fs::write(markers.join("94218"), "").unwrap();
        for (stamp, output) in [
            ("20260823-154410", CAPTURED_TALLY),
            ("20260827-155740", CAPTURED_REDRAW),
        ] {
            let name = format!("{RUN_LOG_PREFIX}{stamp}{PID_SEPARATOR}94218{RUN_LOG_SUFFIX}");
            fs::write(root.path().join(name), output).unwrap();
        }

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Running).read(94218),
            Some(compiling(149, 403))
        );
    }

    #[test]
    fn each_live_run_reports_its_own_log_and_an_uncaptured_pid_reports_none() {
        let other = "\u{1b}[1m    Building\u{1b}[0m [==>    ] 12/48: serde\r";
        let root = capture_root(&[(33395, CAPTURED_REDRAW), (33396, other)], &[33395, 33396]);
        let capture = Capture::take_from(root.path(), |_| RunLiveness::Running);

        assert_eq!(
            capture
                .read(33395)
                .and_then(RunState::working)
                .map(|(_, progress)| progress.percent()),
            Some(36)
        );
        assert_eq!(
            capture
                .read(33396)
                .and_then(RunState::working)
                .map(|(_, progress)| progress.percent()),
            Some(25)
        );
        assert_eq!(capture.read(70001), None);
    }

    /// Nothing reads a capture once its run has ended, so a log that
    /// outlives its run is a file with no reader that every later scan
    /// pays to walk past.
    #[test]
    fn a_log_whose_run_has_ended_is_deleted_rather_than_walked_past_forever() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);
        let log = root.path().join(format!(
            "{RUN_LOG_PREFIX}20260822-101500{PID_SEPARATOR}33395{RUN_LOG_SUFFIX}"
        ));
        assert!(log.exists(), "the log is there to begin with");

        Capture::take_from(root.path(), |_| RunLiveness::Running);

        assert!(!log.exists());
    }

    /// The older of two logs under one live pid belongs to a run that
    /// ended days ago. Reading the newest is what keeps it from being
    /// reported; deleting it is what stops the choice having to be made
    /// again on every scan for the rest of the session.
    #[test]
    fn the_older_of_two_logs_under_one_live_pid_is_retired_not_merely_passed_over() {
        let root = tempdir().unwrap();
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).unwrap();
        fs::write(markers.join("94218"), "").unwrap();
        let names: Vec<PathBuf> = [
            ("20260823-154410", CAPTURED_TALLY),
            ("20260827-155740", CAPTURED_REDRAW),
        ]
        .iter()
        .map(|(stamp, output)| {
            let path = root.path().join(format!(
                "{RUN_LOG_PREFIX}{stamp}{PID_SEPARATOR}94218{RUN_LOG_SUFFIX}"
            ));
            fs::write(&path, output).unwrap();
            path
        })
        .collect();

        Capture::take_from(root.path(), |_| RunLiveness::Running);

        assert!(!names[0].exists());
        assert!(
            names[1].exists(),
            "and the one being written now is left alone"
        );
    }

    /// A complete inventory can hold more orphaned logs than one sweep
    /// allowance. Cleanup drains that backlog over several scans; an inventory
    /// reaching its enumeration limit remains unswept instead.
    #[test]
    fn a_backlog_is_cleared_over_several_scans_rather_than_holding_one_up() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let backlog = CAPTURE_SWEEP_LIMIT + CAPTURE_SWEEP_LIMIT / 2;
        for index in 0..backlog {
            let name =
                format!("{RUN_LOG_PREFIX}20260823-154410{PID_SEPARATOR}{index}{RUN_LOG_SUFFIX}");
            fs::write(root.path().join(name), CAPTURED_REDRAW).unwrap();
        }

        Capture::take_from(root.path(), |_| RunLiveness::Ended);
        let after_one = fs::read_dir(root.path()).unwrap().count();

        assert_eq!(
            after_one,
            backlog - CAPTURE_SWEEP_LIMIT + 1,
            "one scan takes its allowance and leaves the state directory"
        );

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        assert_eq!(
            fs::read_dir(root.path()).unwrap().count(),
            1,
            "the next scan clears the remaining backlog and leaves the state directory"
        );
    }

    /// Registration cleanup shares the allowance with logs and resets on the next scan.
    #[test]
    fn stale_registrations_and_logs_share_one_budget() {
        let root = capture_root(&[], &[]);
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        for pid in 0..CAPTURE_SWEEP_LIMIT {
            fs::write(markers.join(pid.to_string()), "").unwrap();
            fs::write(
                root.path().join(format!("run-20260822-101500-{pid}.log")),
                "",
            )
            .unwrap();
        }

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        let remaining = fs::read_dir(&markers).unwrap().count()
            + fs::read_dir(root.path()).unwrap().count()
            - 1;
        assert_eq!(remaining, CAPTURE_SWEEP_LIMIT);

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        assert_eq!(fs::read_dir(&markers).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }

    /// Staging inspection consumes cleanup work without claiming a paused writer has ended.
    #[test]
    fn staging_files_consume_the_shared_budget_and_remain_in_place() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[]);
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        for pid in 0..CAPTURE_SWEEP_LIMIT {
            fs::write(
                markers.join(format!("{pid}.20260822-101500{REGISTRATION_TEMP_SUFFIX}")),
                "",
            )
            .unwrap();
        }

        Capture::take_from(root.path(), |_| RunLiveness::Ended);

        assert_eq!(fs::read_dir(&markers).unwrap().count(), CAPTURE_SWEEP_LIMIT);
        assert!(root.path().join("run-20260822-101500-33395.log").exists());
    }

    /// Both roots use the production scan loop, with staging, registrations,
    /// and logs competing for the same allowance on each invocation.
    #[test]
    fn two_owned_roots_share_one_budget_for_every_artifact_kind() {
        let first = capture_root(&[], &[]);
        let second = capture_root(&[], &[]);
        let staging = CAPTURE_SWEEP_LIMIT / 4;
        let roots = [first.path(), second.path()];
        for (root, count) in [(roots[0], staging), (roots[1], CAPTURE_SWEEP_LIMIT / 2)] {
            let markers = root.join(CAPTURE_LIVE_RUNS_DIR);
            for pid in 0..count {
                fs::write(markers.join(pid.to_string()), "").unwrap();
                fs::write(root.join(format!("run-20260822-101500-{pid}.log")), "").unwrap();
            }
        }
        let markers = first.path().join(CAPTURE_LIVE_RUNS_DIR);
        for pid in 0..staging {
            fs::write(
                markers.join(format!("{pid}.generation{REGISTRATION_TEMP_SUFFIX}")),
                "",
            )
            .unwrap();
        }
        let inventory_count = || {
            roots
                .iter()
                .map(|root| {
                    fs::read_dir(root).unwrap().count() - 1
                        + fs::read_dir(root.join(CAPTURE_LIVE_RUNS_DIR))
                            .unwrap()
                            .count()
                })
                .sum::<usize>()
        };
        let before = inventory_count();

        Capture::take_roots(&roots, &|_| RunLiveness::Ended);

        assert_eq!(before - inventory_count(), CAPTURE_SWEEP_LIMIT - staging);
        assert_eq!(fs::read_dir(&markers).unwrap().count(), staging);
        let after_first = inventory_count();

        Capture::take_roots(&roots, &|_| RunLiveness::Ended);

        assert_eq!(
            after_first - inventory_count(),
            CAPTURE_SWEEP_LIMIT - staging
        );
        assert_eq!(
            inventory_count(),
            before - (CAPTURE_SWEEP_LIMIT - staging) * 2
        );
    }

    /// A root whose directories are writable by another account has no sweep
    /// capability and cannot spend the later owned root's allowance.
    #[test]
    fn an_unsweepable_root_leaves_the_budget_for_the_owned_root() {
        let untrusted = capture_root(&[], &[]);
        let owned = capture_root(&[], &[]);
        for root in [untrusted.path(), owned.path()] {
            for pid in 0..CAPTURE_SWEEP_LIMIT {
                fs::write(root.join(format!("run-20260822-101500-{pid}.log")), "").unwrap();
            }
        }
        fs::set_permissions(untrusted.path(), fs::Permissions::from_mode(0o770)).unwrap();

        Capture::take_roots(&[untrusted.path(), owned.path()], &|_| RunLiveness::Ended);

        assert_eq!(
            fs::read_dir(untrusted.path()).unwrap().count(),
            CAPTURE_SWEEP_LIMIT + 1
        );
        assert_eq!(fs::read_dir(owned.path()).unwrap().count(), 1);
    }

    /// Verified ownership permits cleanup while retaining the live run's reading
    /// and preserving staging records whose writers have not been verified as ended.
    #[test]
    fn owned_roots_remain_readable_while_stale_artifacts_are_swept() {
        let root = capture_root(
            &[(33395, CAPTURED_REDRAW), (33396, CAPTURED_TALLY)],
            &[33395, 33396],
        );
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::write(registrations.join("33397.generation.tmp"), "").unwrap();
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        let mut budget = SweepBudget::default();
        let mut capture = Capture::default();

        capture.scan_root(&scan, &|pid| (pid == 33395).into(), &mut budget);

        assert_eq!(capture.read(33395), Some(compiling(149, 403)));
        assert_eq!(capture.read(33396), None);
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 3);
        assert_eq!(fs::read_dir(registrations).unwrap().count(), 2);
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    }

    /// Directory entries named like orphan logs make unlink fail without a
    /// second uid. Each attempted removal still consumes the shared allowance.
    #[test]
    fn failed_removals_still_bound_work_across_roots() {
        let failures = capture_root(&[], &[]);
        let later = capture_root(&[], &[]);
        for pid in 0..CAPTURE_SWEEP_LIMIT {
            let name = format!("run-20260822-101500-{pid}.log");
            fs::create_dir(failures.path().join(&name)).unwrap();
            fs::write(later.path().join(name), "").unwrap();
        }

        Capture::take_roots(&[failures.path(), later.path()], &|_| RunLiveness::Ended);

        assert_eq!(
            fs::read_dir(failures.path()).unwrap().count(),
            CAPTURE_SWEEP_LIMIT + 1
        );
        assert_eq!(
            fs::read_dir(later.path()).unwrap().count(),
            CAPTURE_SWEEP_LIMIT + 1
        );
    }

    /// A run killed before it could clear its own registration leaves
    /// the file standing, and every scan after that would read the
    /// whole capture directory on the strength of it.
    #[test]
    fn a_registration_outliving_its_process_is_cleared_away() {
        let root = capture_root(&[(33395, CAPTURED_REDRAW)], &[33395]);
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("33395");

        assert_eq!(
            Capture::take_from(root.path(), |_| RunLiveness::Ended).read(33395),
            None
        );
        assert!(!registration.exists());
    }

    /// The ordinary state of a machine that never switched capture on.
    #[test]
    fn a_missing_capture_directory_reports_nothing_rather_than_failing() {
        let root = tempdir().unwrap();

        assert_eq!(
            Capture::take_from(&root.path().join("never-created"), |_| RunLiveness::Running)
                .read(33395),
            None
        );
    }

    #[test]
    fn a_captured_redraw_reports_cargos_own_counts() {
        assert_eq!(parse_state(CAPTURED_REDRAW), Some(compiling(149, 403)));
    }

    #[test]
    fn the_last_redraw_in_the_tail_is_the_one_reported() {
        let tail = format!("{CAPTURED_REDRAW}Building [=>] 7/9: serde\r");
        assert_eq!(parse_state(&tail), Some(compiling(7, 9)));
    }

    #[test]
    fn a_download_counter_reports_the_phase_that_is_running() {
        assert_eq!(
            parse_state("Downloading [==>    ] 12/40: serde, regex\r"),
            Some(compiling(12, 40))
        );
    }

    #[test]
    fn a_test_runners_tally_is_not_a_counter() {
        assert_eq!(parse_state("PASS [   0.012s] 12/345 crate::suite"), None);
    }

    #[test]
    fn a_test_runners_own_bar_reports_the_tests_it_has_got_through() {
        assert_eq!(parse_state(CAPTURED_TEST_REDRAW), Some(testing(12, 24)));
    }

    /// The bar is redrawn above the tests in flight, so the counter is
    /// never the last thing in the log while the run is going.
    #[test]
    fn the_rows_drawn_under_a_test_bar_do_not_hide_its_counter() {
        let tail = format!("{CAPTURED_TEST_REDRAW}{CAPTURED_TEST_ROW}{CAPTURED_TEST_ROW}");

        assert_eq!(parse_state(&tail), Some(testing(12, 24)));
    }

    /// What `cargo nextest run` does from end to end: cargo's units
    /// first, then the tests. Each phase is reported while it is the
    /// one running.
    #[test]
    fn a_test_run_reports_building_and_then_testing() {
        assert_eq!(parse_state(CAPTURED_REDRAW), Some(compiling(149, 403)));

        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_TEST_REDRAW}");

        assert_eq!(parse_state(&tail), Some(testing(12, 24)));
    }

    /// A run with no terminal under it -- a script, an agent, a CI job
    /// -- gets no bar from nextest at all, and the count it would have
    /// drawn there goes into every line it prints instead.
    #[test]
    fn a_tally_reports_the_tests_where_there_was_no_bar_to_draw() {
        assert_eq!(parse_state(CAPTURED_TALLY), Some(testing(11, 24)));
    }

    /// The first numbers of a run are padded out to the width of the
    /// total, blanks inside the parenthesis rather than in front of it.
    #[test]
    fn a_padded_tally_is_read_the_same_as_a_full_one() {
        assert_eq!(
            parse_state("        PASS [   1.022s] (  1/240) nxprobe t1\n"),
            Some(testing(1, 240))
        );
    }

    /// The build is what a run without a terminal reports until the
    /// tests start, cargo drawing the bar it is asked for either way.
    #[test]
    fn a_tally_after_a_build_bar_is_what_the_run_is_doing() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_TALLY}");

        assert_eq!(parse_state(&tail), Some(testing(11, 24)));
    }

    #[test]
    fn output_with_no_bar_in_it_reports_nothing() {
        assert_eq!(parse_state("   Compiling serde v1.0.0\n"), None);
    }

    #[test]
    fn a_counter_over_zero_units_is_rejected_rather_than_divided_by() {
        assert_eq!(parse_state("Building [ ] 0/0: \r"), None);
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
    fn a_run_waiting_on_the_build_directory_reports_that_it_is_blocked() {
        assert_eq!(parse_state(CAPTURED_WAIT), Some(RunState::Blocked));
    }

    /// Cargo counts its downloads before it reaches for the lock, so a
    /// blocked run has usually drawn a bar already. The bar is stale and
    /// the wait is not.
    #[test]
    fn a_wait_after_a_bar_is_what_the_run_is_doing() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_WAIT}");

        assert_eq!(parse_state(&tail), Some(RunState::Blocked));
    }

    /// The wait line is printed once and stays in the log, so a run that
    /// got its lock and started building must not still read as blocked.
    #[test]
    fn a_bar_after_a_wait_means_the_lock_came_free() {
        let tail = format!("{CAPTURED_WAIT}{CAPTURED_REDRAW}");

        assert_eq!(parse_state(&tail), Some(compiling(149, 403)));
    }

    /// A run with nothing to compile draws no bar at all, so there is no
    /// counter to weigh the wait against -- and what follows it is the
    /// output of the binary the run went on to start. That output is
    /// proof enough the lock came free.
    #[test]
    fn output_after_a_wait_means_the_lock_came_free_even_with_no_bar() {
        let tail = format!(
            "{CAPTURED_WAIT}    Finished `dev` profile [unoptimized + debuginfo] target(s) in \
             3.19s\n     Running `/rust/bevy_brp/target/debug/examples/extras_plugin`\nINFO \
             bevy_winit::system: Creating new window\n"
        );

        assert_eq!(parse_state(&tail), None);
    }

    /// Cargo takes the package cache under the same wording as the build
    /// directory and gives it straight back, so every command run beside
    /// another says this. It is not a wait anyone can see.
    #[test]
    fn a_wait_on_the_package_cache_is_not_a_state_worth_showing() {
        let tail = "    Blocking waiting for file lock on package cache\n";

        assert_eq!(parse_state(tail), None);
    }

    /// Cargo's closing line as the shim captures it, the profile in the
    /// hyperlink escape cargo wraps it in.
    const CAPTURED_FINISHED: &str = "\u{1b}[1m\u{1b}[92m    Finished\u{1b}[0m \
         \u{1b}]8;;https://doc.rust-lang.org/cargo/reference/profiles.html\u{1b}\\`dev` profile \
         [unoptimized + debuginfo]\u{1b}]8;;\u{1b}\\ target(s) in 1.49s\n";

    /// The bar is left on screen where it stopped, so the last redraw of
    /// a build that finished at `1/2` reads no differently from one still
    /// working through its second unit. A `cargo run` then lives on as
    /// the app it started, reporting 50% for hours.
    #[test]
    fn a_counter_a_finished_build_left_behind_is_not_a_reading() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_FINISHED}");

        assert_eq!(parse_state(&tail), None);
    }

    /// A test runner counts its own tests once the compiling is over, so
    /// a counter past cargo's `Finished` is the run's own and stands.
    #[test]
    fn a_counter_after_a_finished_build_is_the_test_runners() {
        let tail = format!("{CAPTURED_FINISHED}{CAPTURED_TEST_REDRAW}");

        assert_eq!(parse_state(&tail), Some(testing(12, 24)));
    }

    #[test]
    fn a_blocked_state_has_no_reading_to_draw() {
        assert_eq!(RunState::Blocked.working(), None);
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
