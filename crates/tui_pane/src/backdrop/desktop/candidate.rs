//! Which of the emulator's windows a capture should be taken behind.
//!
//! Every window of one emulator answers to the same application, so a
//! candidate set is classified first -- by process ancestry, then by
//! the name `TERM_PROGRAM` gives, then by the frontmost owner -- and
//! the closest size match is taken from whichever set answered.
//! [`CaptureWindowTarget`] carries a window the monitor has already
//! pinned, which skips the heuristic entirely.

use super::capture_attempt::CaptureAttemptResult;
use super::capture_attempt::CaptureAttemptSequence;
use super::capture_attempt::CaptureAttemptTestCase;
use super::capture_attempt::CaptureAttemptWindowSelection;
use super::capture_attempt::CaptureFailure;
use super::capture_attempt::CaptureWindowSelectionMethod;
use super::capture_attempt::TerminalWindowCandidateSource;
use crate::backdrop::constants::CAPTURE_TEST_FRONTMOST_OWNER_PID;
use crate::backdrop::constants::CAPTURE_TEST_PROCESS_ANCESTOR_PID;
use crate::backdrop::constants::CAPTURE_TEST_TERMINAL_PROGRAM_OWNER_PID;

/// Which terminal window a capture should prefer before using the candidate-set heuristic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::backdrop) enum CaptureWindowTarget {
    /// Prefer this exact window-server id while it remains available.
    PreferWindow { window_id: u32 },
    /// Select a window from the classified terminal-window candidates.
    TerminalWindowHeuristic,
}

/// Whether a completed terminal-window lookup found a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::backdrop) enum TerminalWindowSearchOutcome {
    /// No terminal window satisfied the lookup.
    NotFound,
    /// The lookup found this window-server id.
    Found { window_id: u32 },
}

/// Which application the window server says owns a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::backdrop::desktop) enum TerminalWindowOwner {
    /// The window server named no owning application, so the window
    /// cannot be matched to a process at all.
    Unnamed,
    /// The window belongs to the application running under this pid.
    Application {
        /// The owning application's process id.
        pid: i32,
    },
}

/// One of the emulator's windows and the title it wore when the list
/// was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::backdrop) struct TitledWindow {
    /// The window server's own number for the window.
    pub(in crate::backdrop) window_id: u32,
    /// What the window server says the window is titled.
    pub(in crate::backdrop) title:     WindowTitle,
}

/// What the window server will say a window is titled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::backdrop) enum WindowTitle {
    /// The window server would not say, which is what it does without
    /// Screen Recording permission. Nothing here can make it answer.
    Withheld,
    /// The window server reports this title.
    Reported(String),
}

/// Window-server facts used to classify terminal-window candidates.
pub(in crate::backdrop::desktop) trait TerminalWindowCandidate {
    /// Which application the window server says owns the window.
    fn owner(&self) -> TerminalWindowOwner;

    /// Whether this window can identify the frontmost application.
    fn frontmost(&self) -> bool;
}

/// Candidate terminal windows and the source that produced them.
pub(in crate::backdrop::desktop) struct TerminalWindowCandidates<'a, W> {
    /// Where this candidate set came from.
    source:                                   TerminalWindowCandidateSource,
    /// The windows available to closest-size matching.
    pub(in crate::backdrop::desktop) windows: Vec<&'a W>,
}

/// Classify terminal-window candidates by process ancestry, terminal name, then frontmost owner.
pub(in crate::backdrop::desktop) fn terminal_window_candidates<W: TerminalWindowCandidate>(
    windows: &[W],
    process_is_ancestor: impl Fn(i32) -> bool,
    window_is_owned_by_terminal_program: impl Fn(&W) -> bool,
) -> TerminalWindowCandidates<'_, W> {
    let process_ancestry_windows = windows_owned_by(windows, process_is_ancestor);
    if !process_ancestry_windows.is_empty() {
        return TerminalWindowCandidates {
            source:  TerminalWindowCandidateSource::ProcessAncestry,
            windows: process_ancestry_windows,
        };
    }
    let terminal_program_windows = windows
        .iter()
        .filter(|window| window_is_owned_by_terminal_program(window))
        .collect::<Vec<_>>();
    if !terminal_program_windows.is_empty() {
        return TerminalWindowCandidates {
            source:  TerminalWindowCandidateSource::TerminalProgramName,
            windows: terminal_program_windows,
        };
    }
    let frontmost_application_windows = match frontmost_owner(windows) {
        TerminalWindowOwner::Application { pid } => windows_owned_by(windows, |owner| owner == pid),
        TerminalWindowOwner::Unnamed => Vec::new(),
    };
    TerminalWindowCandidates {
        source:  TerminalWindowCandidateSource::FrontmostApplication,
        windows: frontmost_application_windows,
    }
}

/// Every window whose owning application's pid `wanted` accepts.
fn windows_owned_by<W: TerminalWindowCandidate>(
    windows: &[W],
    wanted: impl Fn(i32) -> bool,
) -> Vec<&W> {
    windows
        .iter()
        .filter(|window| match window.owner() {
            TerminalWindowOwner::Application { pid } => wanted(pid),
            TerminalWindowOwner::Unnamed => false,
        })
        .collect()
}

/// Which application owns the frontmost candidate window.
fn frontmost_owner<W: TerminalWindowCandidate>(windows: &[W]) -> TerminalWindowOwner {
    windows
        .iter()
        .find(|window| window.frontmost())
        .map_or(TerminalWindowOwner::Unnamed, TerminalWindowCandidate::owner)
}

/// Select a pinned window or the closest candidate and report how it was selected.
pub(in crate::backdrop::desktop) fn select_capture_window<'a, W>(
    windows: &'a [W],
    capture_window_target: CaptureWindowTarget,
    terminal_window_candidates: &TerminalWindowCandidates<'a, W>,
    window_id: impl Fn(&W) -> u32,
    closest_size_match: impl FnOnce() -> Result<&'a W, CaptureFailure>,
) -> Result<(&'a W, CaptureWindowSelectionMethod), CaptureFailure> {
    let preferred_window = match capture_window_target {
        CaptureWindowTarget::PreferWindow {
            window_id: preferred_id,
        } => windows
            .iter()
            .find(|window| window_id(window) == preferred_id)
            .map(|window| (window, CaptureWindowSelectionMethod::PinnedWindow)),
        CaptureWindowTarget::TerminalWindowHeuristic => None,
    };
    preferred_window.map_or_else(
        || {
            closest_size_match().map(|window| {
                (
                    window,
                    CaptureWindowSelectionMethod::ClosestSizeMatch {
                        terminal_window_candidate_source: terminal_window_candidates.source,
                    },
                )
            })
        },
        Ok,
    )
}

/// Build the failure produced before the capture path selects a terminal window.
pub(in crate::backdrop::desktop) const fn capture_failure_before_window_selection(
    sequence: CaptureAttemptSequence,
    failure: CaptureFailure,
) -> CaptureAttemptResult {
    CaptureAttemptResult::failed(
        sequence,
        CaptureAttemptWindowSelection::SelectionNotReached,
        failure,
    )
}

/// The ownership fact that selects one terminal-window candidate source.
#[derive(Clone, Copy)]
enum CaptureAttemptTestWindowOwnership {
    /// The window is owned by a process ancestor.
    ProcessAncestor,
    /// The window is owned by the named terminal program.
    TerminalProgram,
    /// The window is owned by the frontmost application.
    FrontmostApplication,
}

/// A synthetic window supplied to the shared candidate classifier.
struct CaptureAttemptTestWindow {
    /// The window-server id used by the selection result.
    window_id: u32,
    /// How the synthetic window relates to the terminal process and active application.
    ownership: CaptureAttemptTestWindowOwnership,
}

impl TerminalWindowCandidate for CaptureAttemptTestWindow {
    fn owner(&self) -> TerminalWindowOwner {
        TerminalWindowOwner::Application {
            pid: match self.ownership {
                CaptureAttemptTestWindowOwnership::ProcessAncestor => {
                    CAPTURE_TEST_PROCESS_ANCESTOR_PID
                },
                CaptureAttemptTestWindowOwnership::TerminalProgram => {
                    CAPTURE_TEST_TERMINAL_PROGRAM_OWNER_PID
                },
                CaptureAttemptTestWindowOwnership::FrontmostApplication => {
                    CAPTURE_TEST_FRONTMOST_OWNER_PID
                },
            },
        }
    }

    fn frontmost(&self) -> bool {
        matches!(
            self.ownership,
            CaptureAttemptTestWindowOwnership::FrontmostApplication
        )
    }
}

/// Run a client acceptance test through the same selection helper as the platform backends.
pub(in crate::backdrop) fn capture_attempt_for_test(
    sequence: CaptureAttemptSequence,
    capture_window_target: CaptureWindowTarget,
    capture_attempt_test_case: CaptureAttemptTestCase,
) -> CaptureAttemptResult {
    let capture_attempt_test_window = match capture_attempt_test_case {
        CaptureAttemptTestCase::DisplayCaptureFails => {
            return capture_failure_before_window_selection(
                sequence,
                CaptureFailure::DisplayCaptureFailed,
            );
        },
        CaptureAttemptTestCase::PinnedWindow { window_id }
        | CaptureAttemptTestCase::WindowOwnedByProcessAncestor { window_id } => {
            CaptureAttemptTestWindow {
                window_id,
                ownership: CaptureAttemptTestWindowOwnership::ProcessAncestor,
            }
        },
        CaptureAttemptTestCase::WindowOwnedByTerminalProgram { window_id } => {
            CaptureAttemptTestWindow {
                window_id,
                ownership: CaptureAttemptTestWindowOwnership::TerminalProgram,
            }
        },
        CaptureAttemptTestCase::WindowOwnedByFrontmostApplication { window_id } => {
            CaptureAttemptTestWindow {
                window_id,
                ownership: CaptureAttemptTestWindowOwnership::FrontmostApplication,
            }
        },
    };
    let windows = [capture_attempt_test_window];
    let terminal_window_candidates = terminal_window_candidates(
        &windows,
        |owner| owner == CAPTURE_TEST_PROCESS_ANCESTOR_PID,
        |window| {
            matches!(
                window.ownership,
                CaptureAttemptTestWindowOwnership::TerminalProgram
            )
        },
    );
    let selected = select_capture_window(
        &windows,
        capture_window_target,
        &terminal_window_candidates,
        |window| window.window_id,
        || {
            terminal_window_candidates
                .windows
                .first()
                .copied()
                .ok_or(CaptureFailure::TerminalWindowNotFound)
        },
    );
    let Ok((selected_window, method)) = selected else {
        return capture_failure_before_window_selection(
            sequence,
            CaptureFailure::TerminalWindowNotFound,
        );
    };
    CaptureAttemptResult::failed(
        sequence,
        CaptureAttemptWindowSelection::Selected {
            window_id: selected_window.window_id,
            method,
        },
        CaptureFailure::DisplayNotFound,
    )
}
