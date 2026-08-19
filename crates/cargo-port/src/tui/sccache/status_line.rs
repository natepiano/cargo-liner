use std::time::Duration;
use std::time::Instant;

use tui_pane::StatusLineNote;

use super::constants::NOTE_LABEL;
use super::constants::REFRESH_INTERVAL_SECS;
use crate::scan::BackgroundMsg;
use crate::sccache;
use crate::sccache::SccacheSummary;
use crate::tui::app::App;

/// Where the status-line segment stands in its poll cycle.
enum SccachePoll {
    /// No poll has run yet, so the first event-loop tick issues one and the
    /// segment appears without waiting out a full interval.
    Never,
    /// A worker is running `sccache --show-stats`; its reply arrives as
    /// [`BackgroundMsg::SccacheSummary`]. No second poll is issued while
    /// one is outstanding, so a slow sccache stretches the period instead
    /// of stacking up processes.
    InFlight,
    /// The last reply landed at this instant.
    Completed(Instant),
}

/// Whether [`SccacheStatusLine::claim_poll`] handed out a poll slot.
enum SccachePollDecision {
    Claimed,
    NotDue,
}

/// The sccache segment on the right of the status line: the last summary
/// sccache reported plus the poll cycle that refreshes it every
/// [`REFRESH_INTERVAL_SECS`].
pub(in crate::tui) struct SccacheStatusLine {
    summary: SccacheSummary,
    poll:    SccachePoll,
}

impl SccacheStatusLine {
    pub(in crate::tui) const fn new() -> Self {
        Self {
            summary: SccacheSummary::Unavailable,
            poll:    SccachePoll::Never,
        }
    }

    /// The status-line segment, or `None` while sccache has reported
    /// nothing to show — not installed, not answering, or missing the
    /// fields.
    pub(in crate::tui) fn note(&self) -> Option<StatusLineNote> {
        match &self.summary {
            SccacheSummary::Reported {
                cache_size,
                hit_rate,
            } => Some(StatusLineNote {
                label: NOTE_LABEL.to_string(),
                value: format!("{cache_size} - hit rate {hit_rate}"),
            }),
            SccacheSummary::Unavailable => None,
        }
    }

    /// Take a poll slot when one is due, marking the cycle in-flight so the
    /// caller is the only spawner for this interval.
    fn claim_poll(&mut self, now: Instant) -> SccachePollDecision {
        let decision = match self.poll {
            SccachePoll::Never => SccachePollDecision::Claimed,
            SccachePoll::Completed(completed_at)
                if now.saturating_duration_since(completed_at)
                    >= Duration::from_secs(REFRESH_INTERVAL_SECS) =>
            {
                SccachePollDecision::Claimed
            },
            SccachePoll::InFlight | SccachePoll::Completed(_) => SccachePollDecision::NotDue,
        };
        if matches!(decision, SccachePollDecision::Claimed) {
            self.poll = SccachePoll::InFlight;
        }
        decision
    }

    fn apply(&mut self, summary: SccacheSummary, now: Instant) {
        self.summary = summary;
        self.poll = SccachePoll::Completed(now);
    }
}

impl Default for SccacheStatusLine {
    fn default() -> Self { Self::new() }
}

/// Re-read the sccache summary once per [`REFRESH_INTERVAL_SECS`].
///
/// Called from the background poll every frame. The event loop's 1 s idle
/// heartbeat is what keeps this reachable while nothing else is happening,
/// so the segment stays current on an idle screen. `sccache --show-stats`
/// spawns a process, so it runs on a worker rather than the render thread.
pub(in crate::tui) fn refresh_summary_if_due(app: &mut App, now: Instant) {
    if matches!(
        app.sccache_status_line.claim_poll(now),
        SccachePollDecision::NotDue
    ) {
        return;
    }
    let sender = app.background.background_sender();
    std::thread::spawn(move || {
        let _ = sender.send(BackgroundMsg::SccacheSummary(sccache::read_summary()));
    });
}

pub(in crate::tui) fn apply_summary(app: &mut App, summary: SccacheSummary) {
    app.sccache_status_line.apply(summary, Instant::now());
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use super::*;

    fn reported() -> SccacheSummary {
        SccacheSummary::Reported {
            cache_size: "101 GiB".to_string(),
            hit_rate:   "4.90 %".to_string(),
        }
    }

    #[test]
    fn first_claim_polls_immediately_and_the_next_waits_the_interval() {
        let start = Instant::now();
        let mut status_line = SccacheStatusLine::new();

        assert!(matches!(
            status_line.claim_poll(start),
            SccachePollDecision::Claimed
        ));
        // In flight: no second process while the first is outstanding.
        assert!(matches!(
            status_line.claim_poll(start + Duration::from_secs(REFRESH_INTERVAL_SECS)),
            SccachePollDecision::NotDue
        ));

        status_line.apply(reported(), start);
        let just_before_due =
            Duration::from_secs(REFRESH_INTERVAL_SECS).saturating_sub(Duration::from_millis(1));
        assert!(matches!(
            status_line.claim_poll(start + just_before_due),
            SccachePollDecision::NotDue
        ));
        assert!(matches!(
            status_line.claim_poll(start + Duration::from_secs(REFRESH_INTERVAL_SECS)),
            SccachePollDecision::Claimed
        ));
    }

    #[test]
    fn note_carries_the_size_and_hit_rate_sccache_reported() {
        let mut status_line = SccacheStatusLine::new();
        assert_eq!(status_line.note(), None);

        status_line.apply(reported(), Instant::now());
        let note = status_line.note().expect("a reported summary has a note");

        assert_eq!(note.label, NOTE_LABEL);
        assert_eq!(note.value, "101 GiB - hit rate 4.90 %");
    }

    #[test]
    fn an_unavailable_reply_clears_the_note() {
        let mut status_line = SccacheStatusLine::new();
        status_line.apply(reported(), Instant::now());

        status_line.apply(SccacheSummary::Unavailable, Instant::now());

        assert_eq!(status_line.note(), None);
    }
}
