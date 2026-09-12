//! Current activity and counters read from captured output.

use super::Progress;
use super::capture_diagnostic::CaptureFailure;
use crate::constants::BAR_GLYPH_FIRST;
use crate::constants::BAR_GLYPH_LAST;
use crate::constants::BUILD_FINISHED_MARKER;
use crate::constants::LOCK_WAIT_MARKER;
use crate::constants::PHASE_BUILDING;
use crate::constants::PHASE_TESTING;
use crate::constants::TALLY_CLOSE;
use crate::constants::TALLY_OPEN;
use crate::constants::TEST_PHASE_MARKER;
use crate::constants::UNIT_COUNTER_LEAD;
use crate::constants::UNIT_COUNTER_SEPARATOR;
use crate::constants::UNIT_COUNTER_TRAILER;

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

/// Whether this invocation has a capture, independently of the latest log contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureLookup {
    /// No accepted registration was associated with this invocation in the scan.
    Unregistered,
    /// The registration exists, even when its log has no current progress.
    Registered(CaptureRead),
}

/// Three distinct observations of the log associated with a registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureRead {
    /// The latest output describes work or a build-directory wait.
    Progress(RunState),
    /// The log was read but has no current counter, including after Finished.
    NoCurrentProgress,
    /// Keep the failure so settings can distinguish access trouble from silence.
    Unreadable(CaptureFailure),
}

impl From<std::io::Result<String>> for CaptureRead {
    fn from(read: std::io::Result<String>) -> Self {
        match read {
            Ok(tail) => match parse_state(&tail) {
                CurrentProgress::Active(state) => Self::Progress(state),
                CurrentProgress::RetiredCounter | CurrentProgress::Unrecognized => {
                    Self::NoCurrentProgress
                },
            },
            Err(error) => Self::Unreadable(error.into()),
        }
    }
}

/// Current activity distinguishes a retired counter from an unrecognized tail.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CurrentProgress {
    /// A counter or a trailing build-directory wait still describes current work.
    Active(RunState),
    /// A recognized Finished marker follows the last recognized counter.
    RetiredCounter,
    /// The tail has neither a recognized counter nor a current build-directory wait.
    Unrecognized,
}

/// The last recognized counter retains its location for Finished comparison.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TailCounter {
    /// Position and activity belong to the same recognized counter.
    Recognized { at: usize, state: RunState },
    /// No candidate in the tail parses as a counter.
    Unrecognized,
}

/// A candidate line either supplies a complete counter or no recognized reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParsedCounter {
    /// Delimiters and a nonzero total validate the counter.
    Recognized {
        counter:  Counter,
        progress: Progress,
    },
    /// Missing, malformed, overflowing, or zero-total text supplies no counter.
    Unrecognized,
}

/// Leading decimal digits distinguish missing digits from an unrepresentable value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeadingNumber<'text> {
    /// Parsed digits and the remainder of the input.
    Digits { number: usize, rest: &'text str },
    /// The input is empty or its first character is not an ASCII digit.
    NoLeadingDigits,
    /// The leading digit run exceeds the range of usize.
    Overflow,
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
fn parse_state(tail: &str) -> CurrentProgress {
    if still_waiting(tail) {
        return CurrentProgress::Active(RunState::Blocked);
    }
    let TailCounter::Recognized { at, state } = last_counter(tail) else {
        return CurrentProgress::Unrecognized;
    };
    if tail
        .rfind(BUILD_FINISHED_MARKER)
        .is_some_and(|over| at <= over)
    {
        CurrentProgress::RetiredCounter
    } else {
        CurrentProgress::Active(state)
    }
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
fn last_counter(tail: &str) -> TailCounter {
    for (index, lead) in tail.rmatch_indices(UNIT_COUNTER_LEAD) {
        let after = index.saturating_add(lead.len());
        let Some(text) = tail.get(after..) else {
            continue;
        };
        let ParsedCounter::Recognized { counter, progress } = counter_at(text) else {
            continue;
        };
        return TailCounter::Recognized {
            at:    index,
            state: RunState::Working {
                phase: counter.phase(tail, index),
                progress,
            },
        };
    }
    TailCounter::Unrecognized
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
fn counter_at(text: &str) -> ParsedCounter {
    let text = text.trim_start_matches(bar_fill);
    let (counter, text) = text
        .strip_prefix(TALLY_OPEN)
        .map_or((Counter::Bar, text), |inside| (Counter::Tally, inside));
    let LeadingNumber::Digits {
        number: done,
        rest: text,
    } = leading_number(text.trim_start_matches(bar_fill))
    else {
        return ParsedCounter::Unrecognized;
    };
    let Some(text) = text.strip_prefix(UNIT_COUNTER_SEPARATOR) else {
        return ParsedCounter::Unrecognized;
    };
    let LeadingNumber::Digits {
        number: total,
        rest: text,
    } = leading_number(text)
    else {
        return ParsedCounter::Unrecognized;
    };
    let closed = match counter {
        Counter::Bar => text.starts_with(UNIT_COUNTER_TRAILER),
        Counter::Tally => text.starts_with(TALLY_CLOSE),
    };
    if total > 0 && closed {
        ParsedCounter::Recognized {
            counter,
            progress: Progress { done, total },
        }
    } else {
        ParsedCounter::Unrecognized
    }
}

/// Whether `character` is part of a drawn bar rather than of the
/// counter beyond it.
fn bar_fill(character: char) -> bool {
    character == ' ' || (BAR_GLYPH_FIRST..=BAR_GLYPH_LAST).contains(&character)
}

/// The digits `text` opens with, and what follows them.
fn leading_number(text: &str) -> LeadingNumber<'_> {
    let end = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    if end == 0 {
        return LeadingNumber::NoLeadingDigits;
    }
    let (digits, rest) = text.split_at(end);
    digits
        .parse()
        .map_or(LeadingNumber::Overflow, |number| LeadingNumber::Digits {
            number,
            rest,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_captured_redraw_reports_cargos_own_counts() {
        assert_eq!(
            parse_state(CAPTURED_REDRAW),
            CurrentProgress::Active(compiling(149, 403))
        );
    }

    #[test]
    fn the_last_redraw_in_the_tail_is_the_one_reported() {
        let tail = format!("{CAPTURED_REDRAW}Building [=>] 7/9: serde\r");
        assert_eq!(parse_state(&tail), CurrentProgress::Active(compiling(7, 9)));
    }

    #[test]
    fn a_download_counter_reports_the_phase_that_is_running() {
        assert_eq!(
            parse_state("Downloading [==>    ] 12/40: serde, regex\r"),
            CurrentProgress::Active(compiling(12, 40))
        );
    }

    #[test]
    fn a_test_runners_tally_is_not_a_counter() {
        assert_eq!(
            parse_state("PASS [   0.012s] 12/345 crate::suite"),
            CurrentProgress::Unrecognized
        );
    }

    #[test]
    fn a_test_runners_own_bar_reports_the_tests_it_has_got_through() {
        assert_eq!(
            parse_state(CAPTURED_TEST_REDRAW),
            CurrentProgress::Active(testing(12, 24))
        );
    }

    /// The bar is redrawn above the tests in flight, so the counter is
    /// never the last thing in the log while the run is going.
    #[test]
    fn the_rows_drawn_under_a_test_bar_do_not_hide_its_counter() {
        let tail = format!("{CAPTURED_TEST_REDRAW}{CAPTURED_TEST_ROW}{CAPTURED_TEST_ROW}");

        assert_eq!(parse_state(&tail), CurrentProgress::Active(testing(12, 24)));
    }

    /// What `cargo nextest run` does from end to end: cargo's units
    /// first, then the tests. Each phase is reported while it is the
    /// one running.
    #[test]
    fn a_test_run_reports_building_and_then_testing() {
        assert_eq!(
            parse_state(CAPTURED_REDRAW),
            CurrentProgress::Active(compiling(149, 403))
        );

        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_TEST_REDRAW}");

        assert_eq!(parse_state(&tail), CurrentProgress::Active(testing(12, 24)));
    }

    /// A run with no terminal under it -- a script, an agent, a CI job
    /// -- gets no bar from nextest at all, and the count it would have
    /// drawn there goes into every line it prints instead.
    #[test]
    fn a_tally_reports_the_tests_where_there_was_no_bar_to_draw() {
        assert_eq!(
            parse_state(CAPTURED_TALLY),
            CurrentProgress::Active(testing(11, 24))
        );
    }

    /// The first numbers of a run are padded out to the width of the
    /// total, blanks inside the parenthesis rather than in front of it.
    #[test]
    fn a_padded_tally_is_read_the_same_as_a_full_one() {
        assert_eq!(
            parse_state("        PASS [   1.022s] (  1/240) nxprobe t1\n"),
            CurrentProgress::Active(testing(1, 240))
        );
    }

    /// The build is what a run without a terminal reports until the
    /// tests start, cargo drawing the bar it is asked for either way.
    #[test]
    fn a_tally_after_a_build_bar_is_what_the_run_is_doing() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_TALLY}");

        assert_eq!(parse_state(&tail), CurrentProgress::Active(testing(11, 24)));
    }

    #[test]
    fn output_with_no_bar_in_it_reports_nothing() {
        assert_eq!(
            parse_state("   Compiling serde v1.0.0\n"),
            CurrentProgress::Unrecognized
        );
    }

    #[test]
    fn a_counter_over_zero_units_is_rejected_rather_than_divided_by() {
        assert_eq!(
            parse_state("Building [ ] 0/0: \r"),
            CurrentProgress::Unrecognized
        );
    }

    #[test]
    fn a_run_waiting_on_the_build_directory_reports_that_it_is_blocked() {
        assert_eq!(
            parse_state(CAPTURED_WAIT),
            CurrentProgress::Active(RunState::Blocked)
        );
    }

    /// Cargo counts its downloads before it reaches for the lock, so a
    /// blocked run has usually drawn a bar already. The bar is stale and
    /// the wait is not.
    #[test]
    fn a_wait_after_a_bar_is_what_the_run_is_doing() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_WAIT}");

        assert_eq!(
            parse_state(&tail),
            CurrentProgress::Active(RunState::Blocked)
        );
    }

    /// The wait line is printed once and stays in the log, so a run that
    /// got its lock and started building must not still read as blocked.
    #[test]
    fn a_bar_after_a_wait_means_the_lock_came_free() {
        let tail = format!("{CAPTURED_WAIT}{CAPTURED_REDRAW}");

        assert_eq!(
            parse_state(&tail),
            CurrentProgress::Active(compiling(149, 403))
        );
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

        assert_eq!(parse_state(&tail), CurrentProgress::Unrecognized);
    }

    /// Cargo takes the package cache under the same wording as the build
    /// directory and gives it straight back, so every command run beside
    /// another says this. It is not a wait anyone can see.
    #[test]
    fn a_wait_on_the_package_cache_is_not_a_state_worth_showing() {
        let tail = "    Blocking waiting for file lock on package cache\n";

        assert_eq!(parse_state(tail), CurrentProgress::Unrecognized);
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

        assert_eq!(parse_state(&tail), CurrentProgress::RetiredCounter);
    }

    /// A test runner counts its own tests once the compiling is over, so
    /// a counter past cargo's `Finished` is the run's own and stands.
    #[test]
    fn a_counter_after_a_finished_build_is_the_test_runners() {
        let tail = format!("{CAPTURED_FINISHED}{CAPTURED_TEST_REDRAW}");

        assert_eq!(parse_state(&tail), CurrentProgress::Active(testing(12, 24)));
    }

    #[test]
    fn finished_without_a_recognized_counter_is_unrecognized_activity() {
        assert_eq!(
            parse_state(CAPTURED_FINISHED),
            CurrentProgress::Unrecognized
        );
    }

    #[test]
    fn a_tail_without_any_recognized_counter_names_its_absence() {
        assert_eq!(last_counter("ordinary output"), TailCounter::Unrecognized);
        assert_eq!(last_counter("Building [ ] 3/0:"), TailCounter::Unrecognized);
    }

    #[test]
    fn malformed_counter_candidates_name_their_rejection() {
        for text in ["", "word", "1", "1/x:", "1/0:", "1/2", "1/2)", "(1/2:"] {
            assert_eq!(counter_at(text), ParsedCounter::Unrecognized, "{text}");
        }
    }

    #[test]
    fn leading_number_distinguishes_missing_digits_from_overflow() {
        for text in ["", "word", "é1", "-1"] {
            assert_eq!(leading_number(text), LeadingNumber::NoLeadingDigits);
        }
        let overflow = format!("{}0", usize::MAX);
        assert_eq!(leading_number(&overflow), LeadingNumber::Overflow);
        assert_eq!(
            leading_number("12/remainder"),
            LeadingNumber::Digits {
                number: 12,
                rest:   "/remainder",
            }
        );
        assert_eq!(
            counter_at(&format!("{overflow}/2:")),
            ParsedCounter::Unrecognized
        );
        assert_eq!(
            counter_at(&format!("1/{overflow}:")),
            ParsedCounter::Unrecognized
        );
    }
}
