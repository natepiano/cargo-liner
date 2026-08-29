//! Settling which of the emulator's windows this app is drawn in.
//!
//! Two ways of asking, and what each of them can be trusted to answer.
//! The terminal is asked outright where its window stands; failing
//! that, it is made to wear a marker title only this process knows for
//! as long as it takes to ask the window server who is wearing it.
//! [`WindowIdentificationState`] carries the passes and the data one
//! pass leaves for the next, and projects both into the report callers
//! read and the target the capture worker is given.

use std::io;
use std::io::Write;
use std::time::Instant;

use crate::backdrop::CaptureWindowTarget;
use crate::backdrop::TerminalWindowSearchOutcome;
use crate::backdrop::constants::IDENTIFY_MARKER;
use crate::backdrop::constants::IDENTIFY_PASSES;
use crate::backdrop::constants::IDENTIFY_RETRY;
use crate::backdrop::desktop;
use crate::backdrop::desktop::CaptureAttemptWindowSelection;
use crate::backdrop::desktop::TitledWindow;
use crate::backdrop::desktop::WindowTitle;
use crate::backdrop::query;
use crate::backdrop::query::TerminalWindowPosition;

/// Progress toward selecting the terminal window whose desktop should be captured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowIdentification {
    /// No identification pass has been attempted.
    NotAttempted,
    /// Identification is still retrying.
    Pending,
    /// Identification settled on this window-server id.
    Identified {
        /// The selected window-server id.
        window_id: u32,
    },
    /// Identification attempts are exhausted, so capture uses frontmost-or-size selection.
    ///
    /// This describes window selection and does not report whether a capture succeeded.
    Fallback,
}

/// Identification progress and the data retained only while a terminal-window search is active.
#[derive(Debug, Default)]
pub(super) enum WindowIdentificationState {
    /// No identification pass has been attempted.
    #[default]
    NotAttempted,
    /// The terminal position query has failed and no marker title has been installed.
    PendingBeforeMarker {
        /// How many identification passes have run.
        attempts_consumed: u32,
        /// When the most recent pass ran.
        attempted_at:      Instant,
    },
    /// A marker title is installed while later passes look for the window wearing it.
    PendingWithMarker {
        /// How many identification passes have run.
        attempts_consumed: u32,
        /// When the most recent pass ran.
        attempted_at:      Instant,
        /// The titles to restore after the marker identifies a window or the search ends.
        previous_titles:   Vec<TitledWindow>,
    },
    /// Identification settled on this exact window.
    Identified { window_id: u32 },
    /// Identification attempts are exhausted and capture should use the candidate-set heuristic.
    Fallback,
}

impl WindowIdentificationState {
    /// Project the private search state into the public identification report.
    pub(super) const fn report(&self) -> WindowIdentification {
        match self {
            Self::NotAttempted => window_identification(0, TerminalWindowSearchOutcome::NotFound),
            Self::PendingBeforeMarker {
                attempts_consumed, ..
            }
            | Self::PendingWithMarker {
                attempts_consumed, ..
            } => window_identification(*attempts_consumed, TerminalWindowSearchOutcome::NotFound),
            Self::Identified { window_id } => WindowIdentification::Identified {
                window_id: *window_id,
            },
            Self::Fallback => WindowIdentification::Fallback,
        }
    }

    /// Project the search state into the target passed through the capture worker.
    pub(super) const fn capture_window_target(&self) -> CaptureWindowTarget {
        match self {
            Self::Identified { window_id } => CaptureWindowTarget::PreferWindow {
                window_id: *window_id,
            },
            Self::NotAttempted
            | Self::PendingBeforeMarker { .. }
            | Self::PendingWithMarker { .. }
            | Self::Fallback => CaptureWindowTarget::TerminalWindowHeuristic,
        }
    }

    /// Run the next due identification pass and retain any data required by a later pass.
    pub(super) fn identify(&mut self, out: &mut impl Write) -> WindowIdentification {
        match self {
            Self::Identified { .. } | Self::Fallback => return self.report(),
            Self::PendingBeforeMarker { attempted_at, .. }
            | Self::PendingWithMarker { attempted_at, .. }
                if attempted_at.elapsed() < IDENTIFY_RETRY =>
            {
                return self.report();
            },
            Self::NotAttempted
            | Self::PendingBeforeMarker { .. }
            | Self::PendingWithMarker { .. } => {},
        }

        match std::mem::take(self) {
            Self::NotAttempted => self.run_first_attempt(out),
            Self::PendingBeforeMarker {
                attempts_consumed, ..
            } => self.run_marker_setup_attempt(out, attempts_consumed + 1, Instant::now()),
            Self::PendingWithMarker {
                attempts_consumed,
                previous_titles,
                ..
            } => self.run_marker_lookup_attempt(
                out,
                attempts_consumed + 1,
                Instant::now(),
                previous_titles,
            ),
            terminal_state @ (Self::Identified { .. } | Self::Fallback) => {
                *self = terminal_state;
            },
        }
        self.report()
    }

    /// Ask the terminal for its window position before installing a marker title.
    fn run_first_attempt(&mut self, out: &mut impl Write) {
        let attempted_at = Instant::now();
        let terminal_window_search_outcome = match query::window_origin(out) {
            TerminalWindowPosition::Reported { origin } => desktop::window_at(origin),
            TerminalWindowPosition::NotReported => TerminalWindowSearchOutcome::NotFound,
        };
        match terminal_window_search_outcome {
            TerminalWindowSearchOutcome::Found { window_id } => {
                *self = Self::Identified { window_id };
            },
            TerminalWindowSearchOutcome::NotFound => {
                self.run_marker_setup_attempt(out, 1, attempted_at);
            },
        }
    }

    /// Install the marker after retaining the titles that may need restoration.
    fn run_marker_setup_attempt(
        &mut self,
        out: &mut impl Write,
        attempts_consumed: u32,
        attempted_at: Instant,
    ) {
        let marker = format!("{IDENTIFY_MARKER}{}", std::process::id());
        let previous_titles = desktop::window_titles();
        if set_title(out, &marker).is_err() {
            *self = if attempts_consumed >= IDENTIFY_PASSES {
                Self::Fallback
            } else {
                Self::PendingBeforeMarker {
                    attempts_consumed,
                    attempted_at,
                }
            };
            return;
        }
        self.run_marker_lookup_attempt(out, attempts_consumed, attempted_at, previous_titles);
    }

    /// Look for the window wearing the installed marker and retain its original titles if needed.
    fn run_marker_lookup_attempt(
        &mut self,
        out: &mut impl Write,
        attempts_consumed: u32,
        attempted_at: Instant,
        previous_titles: Vec<TitledWindow>,
    ) {
        let marker = format!("{IDENTIFY_MARKER}{}", std::process::id());
        match desktop::window_titled(&marker) {
            TerminalWindowSearchOutcome::Found { window_id } => {
                let restored = previous_titles
                    .iter()
                    .find(|window| window.window_id == window_id)
                    .map_or("", |window| match &window.title {
                        WindowTitle::Reported(title) => title.as_str(),
                        WindowTitle::Withheld => "",
                    });
                let _ = set_title(out, restored);
                *self = Self::Identified { window_id };
            },
            TerminalWindowSearchOutcome::NotFound if attempts_consumed >= IDENTIFY_PASSES => {
                let _ = set_title(out, "");
                *self = Self::Fallback;
            },
            TerminalWindowSearchOutcome::NotFound => {
                *self = Self::PendingWithMarker {
                    attempts_consumed,
                    attempted_at,
                    previous_titles,
                };
            },
        }
    }
}

/// The window id used by the most recent successful capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LastSuccessfulCaptureWindowId {
    /// No capture has succeeded yet.
    WaitingForFirstSuccess,
    /// The most recent successful capture used this window-server id.
    Available {
        /// The window-server id used by the capture.
        window_id: u32,
    },
}

/// What the monitor knows about the latest completed attempt's window selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatestCaptureAttemptWindowSelection {
    /// No capture attempt has completed yet.
    WaitingForFirstResult,
    /// The newest completed attempt reached this window-selection state.
    Completed(CaptureAttemptWindowSelection),
}

/// Map consumed identification attempts to the progress reported to callers.
const fn window_identification(
    attempts_consumed: u32,
    terminal_window_search_outcome: TerminalWindowSearchOutcome,
) -> WindowIdentification {
    match terminal_window_search_outcome {
        TerminalWindowSearchOutcome::Found { window_id } => {
            WindowIdentification::Identified { window_id }
        },
        TerminalWindowSearchOutcome::NotFound => match attempts_consumed {
            0 => WindowIdentification::NotAttempted,
            IDENTIFY_PASSES.. => WindowIdentification::Fallback,
            _ => WindowIdentification::Pending,
        },
    }
}

/// Ask the terminal to wear `title`, and see the request out to it.
///
/// Control characters are dropped rather than sent on: a title read
/// back from the window server is text of unknown provenance, and one
/// carrying an escape of its own would be a command rather than a
/// title.
fn set_title(out: &mut impl Write, title: &str) -> io::Result<()> {
    let title: String = title
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    write!(out, "\u{1b}]2;{title}\u{7}")?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::CaptureWindowTarget;
    use super::IDENTIFY_PASSES;
    use super::TerminalWindowSearchOutcome;
    use super::WindowIdentification;
    use super::WindowIdentificationState;

    const WINDOW_ID: u32 = 42;

    #[test]
    fn identification_is_not_attempted_before_a_pass_runs() {
        assert_eq!(
            super::window_identification(0, TerminalWindowSearchOutcome::NotFound),
            WindowIdentification::NotAttempted,
        );
    }

    #[test]
    fn a_found_window_is_identified_with_its_window_id() {
        assert_eq!(
            super::window_identification(
                1,
                TerminalWindowSearchOutcome::Found {
                    window_id: WINDOW_ID,
                },
            ),
            WindowIdentification::Identified {
                window_id: WINDOW_ID,
            },
        );
    }

    #[test]
    fn the_first_spent_allowance_stays_pending() {
        assert_eq!(
            super::window_identification(1, TerminalWindowSearchOutcome::NotFound),
            WindowIdentification::Pending,
        );
    }

    #[test]
    fn the_last_remaining_allowance_stays_pending() {
        assert_eq!(
            super::window_identification(
                IDENTIFY_PASSES - 1,
                TerminalWindowSearchOutcome::NotFound,
            ),
            WindowIdentification::Pending,
        );
    }

    #[test]
    fn exhausting_every_allowance_uses_fallback() {
        assert_eq!(
            super::window_identification(IDENTIFY_PASSES, TerminalWindowSearchOutcome::NotFound,),
            WindowIdentification::Fallback,
        );
    }

    #[test]
    fn not_attempted_state_projects_report_and_capture_target() {
        let state = WindowIdentificationState::NotAttempted;

        assert_eq!(state.report(), WindowIdentification::NotAttempted);
        assert_eq!(
            state.capture_window_target(),
            CaptureWindowTarget::TerminalWindowHeuristic,
        );
    }

    #[test]
    fn pending_before_marker_state_projects_report_and_capture_target() {
        let state = WindowIdentificationState::PendingBeforeMarker {
            attempts_consumed: 1,
            attempted_at:      Instant::now(),
        };

        assert_eq!(state.report(), WindowIdentification::Pending);
        assert_eq!(
            state.capture_window_target(),
            CaptureWindowTarget::TerminalWindowHeuristic,
        );
    }

    #[test]
    fn pending_with_marker_state_projects_report_and_capture_target() {
        let state = WindowIdentificationState::PendingWithMarker {
            attempts_consumed: 1,
            attempted_at:      Instant::now(),
            previous_titles:   Vec::new(),
        };

        assert_eq!(state.report(), WindowIdentification::Pending);
        assert_eq!(
            state.capture_window_target(),
            CaptureWindowTarget::TerminalWindowHeuristic,
        );
    }

    #[test]
    fn identified_state_projects_report_and_capture_target() {
        let state = WindowIdentificationState::Identified {
            window_id: WINDOW_ID,
        };

        assert_eq!(
            state.report(),
            WindowIdentification::Identified {
                window_id: WINDOW_ID,
            },
        );
        assert_eq!(
            state.capture_window_target(),
            CaptureWindowTarget::PreferWindow {
                window_id: WINDOW_ID,
            },
        );
    }

    #[test]
    fn fallback_state_projects_report_and_capture_target() {
        let state = WindowIdentificationState::Fallback;

        assert_eq!(state.report(), WindowIdentification::Fallback);
        assert_eq!(
            state.capture_window_target(),
            CaptureWindowTarget::TerminalWindowHeuristic,
        );
    }
}
