//! What `sccache --show-stats` reports, and the line the summary cell
//! carries on its top border.
//!
//! Two things have to be true before the line appears. The stats have to
//! have been read, which costs a process, so it happens on a worker
//! rather than the render thread and no more often than
//! [`SCCACHE_POLL_SECONDS`]. And a server has to be running, which the
//! process scan answers for free -- it already reads every process's
//! name to count the compilers under each cargo, and `sccache` is one of
//! the names it looks for.
//!
//! That gate is not a nicety. `sccache --show-stats` *starts* the server
//! when none is up, so polling it unconditionally would create the very
//! thing the display is meant to report on only when it is already
//! there.

use std::process::Command;
use std::process::Output;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crate::constants::SCCACHE_BINARY;
use crate::constants::SCCACHE_CACHE_WORD;
use crate::constants::SCCACHE_HIT_RATE_LABEL;
use crate::constants::SCCACHE_HIT_RATE_WORD;
use crate::constants::SCCACHE_HITS_LABEL;
use crate::constants::SCCACHE_HITS_WORD;
use crate::constants::SCCACHE_MAX_SIZE_LABEL;
use crate::constants::SCCACHE_MISSES_LABEL;
use crate::constants::SCCACHE_MISSES_WORD;
use crate::constants::SCCACHE_POLL_SECONDS;
use crate::constants::SCCACHE_SIZE_LABEL;
use crate::constants::SCCACHE_STATS_ARG;

/// Whether an sccache server is up, as the last process scan saw it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SccacheServer {
    /// A process named [`SCCACHE_BINARY`] is running.
    Running,
    /// None is, so the stats go unread rather than starting one.
    Stopped,
}

/// The `sccache --show-stats` fields the summary cell reports, carried
/// as the strings sccache printed so the units stay sccache's own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SccacheSummary {
    /// sccache answered and reported every field.
    Reported {
        /// [`SCCACHE_HIT_RATE_LABEL`]'s value with the gap sccache
        /// leaves before the sign closed up, e.g. `40.19%`.
        hit_rate: String,
        /// [`SCCACHE_HITS_LABEL`]'s value, e.g. `16499`.
        hits:     String,
        /// [`SCCACHE_MISSES_LABEL`]'s value, e.g. `24550`.
        misses:   String,
        /// [`SCCACHE_SIZE_LABEL`]'s value, e.g. `36 GiB`.
        size:     String,
        /// [`SCCACHE_MAX_SIZE_LABEL`]'s value, e.g. `128 GiB`.
        max_size: String,
    },
    /// No server, no sccache on the path, a command that failed, or
    /// output missing one of the fields -- the border carries nothing.
    Unavailable,
}

impl SccacheSummary {
    /// The line for a top border with `room` cells to spare, in the runs
    /// it is set from, padded with the space that keeps it off the line
    /// either side.
    ///
    /// The fields go in the order they can be given up. `hits` and
    /// `misses` are the arithmetic behind the rate, so the rate alone
    /// still carries what they said; the cache span goes next because
    /// what it explains -- a rate that stopped climbing because the
    /// cache is evicting at its ceiling -- is a second reading rather
    /// than the first. `None` once even the rate will not fit.
    fn label(&self, room: u16) -> Option<Vec<LabelRun>> {
        self.rungs()
            .into_iter()
            .find(|rung| label_width(rung) <= usize::from(room))
    }

    /// Every line this summary could be written as, widest first.
    fn rungs(&self) -> Vec<Vec<LabelRun>> {
        let Self::Reported {
            hit_rate,
            hits,
            misses,
            size,
            max_size,
        } = self
        else {
            return Vec::new();
        };
        let cache = cache_span(size, max_size);
        let opening = || LabelRun::name(format!(" {SCCACHE_BINARY}  {SCCACHE_HIT_RATE_WORD} "));
        vec![
            vec![
                opening(),
                LabelRun::value(hit_rate),
                LabelRun::name(format!("  {SCCACHE_HITS_WORD} ")),
                LabelRun::value(hits),
                LabelRun::name(format!("  {SCCACHE_MISSES_WORD} ")),
                LabelRun::value(misses),
                LabelRun::name(format!("  {SCCACHE_CACHE_WORD} ")),
                LabelRun::value(&cache),
                LabelRun::name(" "),
            ],
            vec![
                opening(),
                LabelRun::value(hit_rate),
                LabelRun::name(format!("  {SCCACHE_CACHE_WORD} ")),
                LabelRun::value(&cache),
                LabelRun::name(" "),
            ],
            vec![opening(), LabelRun::value(hit_rate), LabelRun::name(" ")],
        ]
    }
}

/// One run of the border label, held apart from its neighbours so the
/// values can be set in a colour of their own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LabelRun {
    /// The text of the run.
    pub(crate) text: String,
    /// What the run carries.
    pub(crate) kind: LabelRunKind,
}

impl LabelRun {
    /// A run of the words naming the field that follows.
    fn name(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: LabelRunKind::Name,
        }
    }

    /// A run of what sccache reported for one field.
    fn value(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            kind: LabelRunKind::Value,
        }
    }
}

/// Whether a run of the border label names a field or reports it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LabelRunKind {
    /// The words naming the field that follows.
    Name,
    /// What sccache reported for it.
    Value,
}

/// Where the refresh cycle stands.
enum SccachePoll {
    /// Nothing read yet, so the first tick with a server up issues a
    /// read and the line appears without waiting out an interval.
    Never,
    /// A worker is running `sccache --show-stats`. No second read is
    /// issued while one is outstanding, so a server that has wedged
    /// stretches the period instead of stacking up processes -- and
    /// stalls nothing but the one worker parked on it.
    InFlight,
    /// The last reply landed at this instant.
    Completed(Instant),
}

/// Whether [`SccacheStats::claim_poll`] handed out a poll slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SccachePollDecision {
    /// The caller owns this interval's read.
    Claimed,
    /// No server, one already in flight, or the interval has not
    /// elapsed.
    NotDue,
}

/// The last summary read, and where the poll that refreshes it stands.
pub(crate) struct SccacheStats {
    /// What sccache last reported.
    summary: SccacheSummary,
    /// Where the refresh cycle stands.
    poll:    SccachePoll,
    /// Whether a server was up when the last scan looked.
    server:  SccacheServer,
}

impl SccacheStats {
    /// Nothing read yet and no server seen yet.
    pub(crate) const fn new() -> Self {
        Self {
            summary: SccacheSummary::Unavailable,
            poll:    SccachePoll::Never,
            server:  SccacheServer::Stopped,
        }
    }

    /// The line for a top border with `room` cells to spare, in the runs
    /// it is set from.
    pub(crate) fn label(&self, room: u16) -> Option<Vec<LabelRun>> { self.summary.label(room) }

    /// Take what the latest process scan saw, dropping the summary the
    /// moment the server goes: figures from a server that has stopped
    /// describe a cache nothing is reading or writing any more.
    pub(crate) fn observe_server(&mut self, server: SccacheServer) {
        self.server = server;
        if server == SccacheServer::Stopped {
            self.summary = SccacheSummary::Unavailable;
        }
    }

    /// Take a poll slot when one is due, marking the cycle in flight so
    /// the caller is the only spawner for this interval.
    fn claim_poll(&mut self, now: Instant) -> SccachePollDecision {
        if self.server == SccacheServer::Stopped {
            return SccachePollDecision::NotDue;
        }
        let decision = match self.poll {
            SccachePoll::Never => SccachePollDecision::Claimed,
            SccachePoll::Completed(completed_at)
                if now.saturating_duration_since(completed_at)
                    >= Duration::from_secs(SCCACHE_POLL_SECONDS) =>
            {
                SccachePollDecision::Claimed
            },
            SccachePoll::InFlight | SccachePoll::Completed(_) => SccachePollDecision::NotDue,
        };
        if decision == SccachePollDecision::Claimed {
            self.poll = SccachePoll::InFlight;
        }
        decision
    }

    /// Take a worker's reply, reporting whether the border changed.
    fn apply(&mut self, summary: SccacheSummary, now: Instant) -> bool {
        self.poll = SccachePoll::Completed(now);
        // A server that stopped while the read was in flight has already
        // cleared the summary, and this reply describes the server that
        // went. Letting it land would put the line back for one interval.
        if self.server == SccacheServer::Stopped {
            return false;
        }
        let changed = self.summary != summary;
        self.summary = summary;
        changed
    }
}

impl Default for SccacheStats {
    fn default() -> Self { Self::new() }
}

/// What the runs of one rung come to, in characters.
fn label_width(runs: &[LabelRun]) -> usize { runs.iter().map(|run| run.text.chars().count()).sum() }

/// Read the stats on a worker when one is due.
///
/// Called from the event loop every poll. `sccache --show-stats` spawns
/// a process and talks to the server over a socket, so it runs on a
/// thread of its own rather than on the one drawing frames.
pub(crate) fn refresh_if_due(
    stats: &mut SccacheStats,
    replies: &Sender<SccacheSummary>,
    now: Instant,
) {
    if stats.claim_poll(now) == SccachePollDecision::NotDue {
        return;
    }
    let replies = replies.clone();
    thread::spawn(move || {
        let _ = replies.send(read_summary());
    });
}

/// Fold a worker's reply in, reporting whether the border changed.
pub(crate) fn apply(stats: &mut SccacheStats, summary: SccacheSummary) -> bool {
    stats.apply(summary, Instant::now())
}

/// Run `sccache --show-stats` and keep only the border's fields.
fn read_summary() -> SccacheSummary {
    Command::new(SCCACHE_BINARY)
        .arg(SCCACHE_STATS_ARG)
        .output()
        .map_or(SccacheSummary::Unavailable, |output| {
            summary_from_output(&output)
        })
}

/// The summary in one command's output, or [`SccacheSummary::Unavailable`]
/// when it failed or reported none of what the border wants.
fn summary_from_output(output: &Output) -> SccacheSummary {
    if !output.status.success() {
        return SccacheSummary::Unavailable;
    }
    let lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    summary_from_lines(&lines)
}

/// Pick the border's fields out of `sccache --show-stats` output.
///
/// Each field is matched on the whole label rather than a prefix, which
/// is what separates `Cache hits` from the `Cache hits (Rust)` and
/// `Cache hits rate` lines printed beside it.
fn summary_from_lines(lines: &[String]) -> SccacheSummary {
    let field = |label: &str| {
        lines.iter().find_map(|line| {
            let (found, value) = split_aligned_stat(line.trim())?;
            (found == label).then(|| value.to_string())
        })
    };
    let (Some(hit_rate), Some(hits), Some(misses), Some(size), Some(max_size)) = (
        field(SCCACHE_HIT_RATE_LABEL),
        field(SCCACHE_HITS_LABEL),
        field(SCCACHE_MISSES_LABEL),
        field(SCCACHE_SIZE_LABEL),
        field(SCCACHE_MAX_SIZE_LABEL),
    ) else {
        return SccacheSummary::Unavailable;
    };
    SccacheSummary::Reported {
        hit_rate: closed_percent(&hit_rate),
        hits,
        misses,
        size,
        max_size,
    }
}

/// Split one `sccache --show-stats` line at its value column: the first
/// run of two or more spaces separates the label from the value.
fn split_aligned_stat(text: &str) -> Option<(&str, &str)> {
    let mut gap_start = None;
    let mut gap_len = 0;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            gap_start.get_or_insert(idx);
            gap_len += 1;
            continue;
        }
        if gap_len >= 2 {
            let start = gap_start?;
            let label = text[..start].trim_end();
            let value = text[idx..].trim();
            if !label.is_empty() && !value.is_empty() {
                return Some((label, value));
            }
        }
        gap_start = None;
        gap_len = 0;
    }
    None
}

/// sccache prints a percentage with a gap before the sign, `40.19 %`.
/// The border writes it closed up, which is a cell cheaper and reads as
/// one value rather than two.
fn closed_percent(value: &str) -> String { value.replace(' ', "") }

/// The cache span: `36 GiB` against `128 GiB` closes up to
/// `36/128 GiB`, the unit said once because both halves carry it.
///
/// A pair sccache reported in different units keeps both -- a cache
/// under a gibibyte prints as `900 MiB`, and `900/128 GiB` would read
/// as a cache seven times over its ceiling.
fn cache_span(size: &str, max_size: &str) -> String {
    let paired = size
        .split_once(' ')
        .zip(max_size.split_once(' '))
        .filter(|((_, unit), (_, max_unit))| unit == max_unit);
    paired.map_or_else(
        || format!("{size}/{max_size}"),
        |((amount, unit), (max_amount, _))| format!("{amount}/{max_amount} {unit}"),
    )
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should fail on invalid fixtures")]
mod tests {
    use super::*;

    /// `sccache --show-stats` output, cut down to the lines that matter
    /// plus the ones a whole-label match has to tell them from.
    fn stats_lines() -> Vec<String> {
        [
            "Compile requests                  85932",
            "Cache hits                        16499",
            "Cache hits (Rust)                 12665",
            "Cache misses                      24550",
            "Cache misses (Rust)               22715",
            "Cache hits rate                   40.19 %",
            "Cache hits rate (Rust)            35.80 %",
            "Cache location                  Local disk: \"/tmp/sccache\"",
            "Cache size                           36 GiB",
            "Max cache size                      128 GiB",
        ]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
    }

    fn reported() -> SccacheSummary {
        SccacheSummary::Reported {
            hit_rate: "40.19%".to_string(),
            hits:     "16499".to_string(),
            misses:   "24550".to_string(),
            size:     "36 GiB".to_string(),
            max_size: "128 GiB".to_string(),
        }
    }

    #[test]
    fn the_summary_takes_the_five_fields_the_border_carries() {
        assert_eq!(summary_from_lines(&stats_lines()), reported());
    }

    /// The qualified lines sit either side of the plain ones and a
    /// prefix match would take whichever came first.
    #[test]
    fn a_field_is_matched_on_its_whole_label_not_a_prefix() {
        let SccacheSummary::Reported { hits, misses, .. } = summary_from_lines(&stats_lines())
        else {
            panic!("the fixture reports every field");
        };

        assert_eq!(hits, "16499");
        assert_eq!(misses, "24550");
    }

    #[test]
    fn a_missing_field_leaves_the_border_with_nothing_to_say() {
        let without_max: Vec<String> = stats_lines()
            .into_iter()
            .filter(|line| !line.starts_with(SCCACHE_MAX_SIZE_LABEL))
            .collect();

        assert_eq!(
            summary_from_lines(&without_max),
            SccacheSummary::Unavailable
        );
        assert_eq!(summary_from_lines(&[]), SccacheSummary::Unavailable);
    }

    #[test]
    fn the_percentage_loses_the_gap_sccache_prints_before_the_sign() {
        assert_eq!(closed_percent("40.19 %"), "40.19%");
    }

    #[test]
    fn a_matching_pair_of_units_is_said_once() {
        assert_eq!(cache_span("36 GiB", "128 GiB"), "36/128 GiB");
    }

    #[test]
    fn a_mismatched_pair_of_units_keeps_both() {
        assert_eq!(cache_span("900 MiB", "128 GiB"), "900 MiB/128 GiB");
    }

    /// Each rung drops the fields the one above could not fit, and the
    /// widths come from the lines themselves so the test says which rung
    /// was chosen rather than restating its length.
    #[test]
    fn the_label_steps_down_as_the_border_runs_out_of_room() {
        let summary = reported();
        let rungs = summary.rungs();
        let width = |rung: &Vec<LabelRun>| u16::try_from(label_width(rung)).unwrap_or(u16::MAX);
        let [full, medium, minimal] = &rungs[..] else {
            panic!("a reported summary offers three rungs");
        };

        assert_eq!(summary.label(width(full)).as_ref(), Some(full));
        assert_eq!(summary.label(width(full) - 1).as_ref(), Some(medium));
        assert_eq!(summary.label(width(medium) - 1).as_ref(), Some(minimal));
        assert_eq!(summary.label(width(minimal) - 1), None);
    }

    /// The colour the values are set in reads off [`LabelRunKind`], so
    /// every figure has to reach the border as a run of its own.
    #[test]
    fn every_figure_is_a_run_apart_from_the_words_naming_it() {
        let summary = reported();
        let Some(runs) = summary.label(u16::MAX) else {
            panic!("the widest rung fits any room");
        };
        let values: Vec<&str> = runs
            .iter()
            .filter(|run| run.kind == LabelRunKind::Value)
            .map(|run| run.text.as_str())
            .collect();

        assert_eq!(values, ["40.19%", "16499", "24550", "36/128 GiB"]);
    }

    #[test]
    fn an_unavailable_summary_offers_no_label_at_any_width() {
        assert_eq!(SccacheSummary::Unavailable.label(u16::MAX), None);
    }

    #[test]
    fn nothing_is_read_until_a_scan_has_seen_a_server() {
        let mut stats = SccacheStats::new();

        assert_eq!(
            stats.claim_poll(Instant::now()),
            SccachePollDecision::NotDue
        );

        stats.observe_server(SccacheServer::Running);

        assert_eq!(
            stats.claim_poll(Instant::now()),
            SccachePollDecision::Claimed
        );
    }

    #[test]
    fn the_first_claim_reads_at_once_and_the_next_waits_the_interval() {
        let start = Instant::now();
        let interval = Duration::from_secs(SCCACHE_POLL_SECONDS);
        let mut stats = SccacheStats::new();
        stats.observe_server(SccacheServer::Running);

        assert_eq!(stats.claim_poll(start), SccachePollDecision::Claimed);
        // In flight: no second process while the first is outstanding.
        assert_eq!(
            stats.claim_poll(start + interval),
            SccachePollDecision::NotDue
        );

        stats.apply(reported(), start);

        let just_before_due = interval.saturating_sub(Duration::from_millis(1));
        assert_eq!(
            stats.claim_poll(start + just_before_due),
            SccachePollDecision::NotDue
        );
        assert_eq!(
            stats.claim_poll(start + interval),
            SccachePollDecision::Claimed
        );
    }

    #[test]
    fn a_repeated_reply_leaves_the_border_alone() {
        let mut stats = SccacheStats::new();
        stats.observe_server(SccacheServer::Running);

        assert!(stats.apply(reported(), Instant::now()));
        assert!(!stats.apply(reported(), Instant::now()));
    }

    #[test]
    fn a_server_that_stopped_takes_its_figures_with_it() {
        let mut stats = SccacheStats::new();
        stats.observe_server(SccacheServer::Running);
        stats.apply(reported(), Instant::now());

        stats.observe_server(SccacheServer::Stopped);

        assert_eq!(stats.label(u16::MAX), None);
    }

    /// The read is spawned before the scan that stops the server lands,
    /// so the reply arrives describing a server that has already gone.
    #[test]
    fn a_reply_landing_after_the_server_stopped_is_dropped() {
        let mut stats = SccacheStats::new();
        stats.observe_server(SccacheServer::Running);
        stats.observe_server(SccacheServer::Stopped);

        assert!(!stats.apply(reported(), Instant::now()));
        assert_eq!(stats.label(u16::MAX), None);
    }
}
