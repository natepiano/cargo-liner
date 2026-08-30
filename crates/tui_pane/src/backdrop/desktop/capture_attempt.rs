//! One capture attempt: what it was asked for, what window it settled
//! on, and how it ended.
//!
//! The worker returns a [`CaptureAttemptResult`], which owns the
//! captured desktop until the monitor takes it and keeps the
//! [`CompletedCaptureAttemptDiagnostic`] behind -- a record light
//! enough to retain a run of them without holding any image alive.

use std::fmt;
use std::fmt::Formatter;
use std::sync::Arc;

use super::Desktop;

/// Where the candidate terminal-window set came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalWindowCandidateSource {
    /// Windows owned by this process or one of its ancestors.
    ProcessAncestry,
    /// Windows owned by the terminal application named by `TERM_PROGRAM`.
    TerminalProgramName,
    /// Windows owned by the application with the frontmost window.
    FrontmostApplication,
}

/// A synthetic capture situation used to exercise the production selection path.
///
/// This support type exists for a client crate's acceptance tests, which cannot use the private
/// window-server types that the macOS capture backend receives.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureAttemptTestCase {
    /// The shareable-content query fails before any window can be selected.
    ShareableContentQueryFails,
    /// The monitor supplies an id that exists in the full window list.
    PinnedWindow { window_id: u32 },
    /// The closest-size candidate is owned by a process ancestor.
    WindowOwnedByProcessAncestor { window_id: u32 },
    /// The closest-size candidate is owned by the terminal program.
    WindowOwnedByTerminalProgram { window_id: u32 },
    /// The closest-size candidate is owned by the frontmost application.
    WindowOwnedByFrontmostApplication { window_id: u32 },
}

/// How a completed capture attempt selected its terminal window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureWindowSelectionMethod {
    /// The monitor supplied a pinned window id and that window still existed.
    PinnedWindow,
    /// The closest-size window was chosen from the reported candidate set.
    ClosestSizeMatch {
        /// Where the terminal-window candidates came from.
        terminal_window_candidate_source: TerminalWindowCandidateSource,
    },
}

/// What terminal window a completed capture attempt selected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureAttemptWindowSelection {
    /// The attempt failed before a terminal window could be selected.
    SelectionNotReached,
    /// The attempt selected this terminal window.
    Selected {
        /// The selected window-server id.
        window_id: u32,
        /// How the window id was selected.
        method:    CaptureWindowSelectionMethod,
    },
}

/// The monitor-local sequence number assigned to one capture attempt.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CaptureAttemptSequence(u64);

impl CaptureAttemptSequence {
    /// The first attempt sequence assigned by a monitor.
    pub(in crate::backdrop) const FIRST: Self = Self(1);

    /// The numeric sequence value.
    #[must_use]
    pub const fn number(self) -> u64 { self.0 }

    /// The sequence assigned after this one.
    pub(in crate::backdrop) const fn following(self) -> Self { Self(self.0.wrapping_add(1)) }
}

impl From<u64> for CaptureAttemptSequence {
    fn from(number: u64) -> Self { Self(number) }
}

/// Why desktop capture did not produce a new image or cannot continue.
///
/// The failure records only the stage or worker-lifecycle event that stopped capture, so it remains
/// cheap to send from the capture worker and retain as monitor state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureFailure {
    /// This platform has no desktop-capture backend.
    UnsupportedPlatform,
    /// The capture worker missed this attempt's deadline and was kept because a first capture of
    /// a display can legitimately take seconds.
    AttemptStalled,
    /// The capture worker missed a second consecutive deadline without returning anything in
    /// between, so it was abandoned and replaced.
    CaptureWorkerReplaced,
    /// A capture worker could not be launched.
    WorkerLaunchFailed,
    /// The capture worker's result channel disconnected.
    WorkerDisconnected,
    /// The monitor abandoned its maximum number of stalled or disconnected capture workers.
    WorkerReplacementLimitReached,
    /// The shareable-content query failed while the Screen Recording access check reported that
    /// access was not granted. The check gives the same answer when the process has never prompted
    /// for access and when the user has refused it.
    ScreenRecordingAccessNotGranted,
    /// `ScreenCaptureKit` could not list the shareable displays and windows.
    ShareableContentQueryFailed,
    /// The shareable displays and windows were requested, but macOS did not answer within the call
    /// deadline, so the request was abandoned.
    ShareableContentQueryTimedOut,
    /// No window could be matched to the terminal running the app.
    TerminalWindowNotFound,
    /// No display could be matched to the selected terminal window.
    DisplayNotFound,
    /// Another process's display capture was still in flight when the call deadline ran out, so
    /// this attempt never asked for one.
    ScreenshotTurnTimedOut,
    /// `ScreenCaptureKit` could not capture the selected display.
    ScreenshotFailed,
    /// The display capture was requested, but macOS did not answer within the call deadline, so the
    /// request was abandoned.
    ScreenshotTimedOut,
    /// The captured image could not expose its RGBA pixel bytes.
    PixelExtractionFailed,
    /// The captured pixels could not be reduced to terminal-cell colors.
    ImageReductionFailed,
}

/// The capture worker's result for one completed attempt.
///
/// A successful result owns the captured desktop until the backdrop monitor retains it and
/// converts the result into a [`CompletedCaptureAttemptDiagnostic`].
#[derive(Clone)]
pub struct CaptureAttemptResult {
    /// The monitor-local sequence assigned when the attempt was requested.
    sequence:         CaptureAttemptSequence,
    /// What terminal window the completed attempt selected.
    window_selection: CaptureAttemptWindowSelection,
    /// The desktop produced on success or the stage that failed.
    outcome:          CaptureAttemptOutcome,
}

impl CaptureAttemptResult {
    /// Build a completed attempt from the platform capture result.
    #[cfg(target_os = "macos")]
    pub(in crate::backdrop::desktop) fn from_desktop_result(
        sequence: CaptureAttemptSequence,
        window_selection: CaptureAttemptWindowSelection,
        desktop_result: Result<Desktop, CaptureFailure>,
    ) -> Self {
        match desktop_result {
            Ok(desktop) => Self {
                sequence,
                window_selection,
                outcome: CaptureAttemptOutcome::Succeeded(Arc::new(desktop)),
            },
            Err(failure) => Self::failed(sequence, window_selection, failure),
        }
    }

    /// Build a completed failed attempt.
    pub(in crate::backdrop) const fn failed(
        sequence: CaptureAttemptSequence,
        window_selection: CaptureAttemptWindowSelection,
        failure: CaptureFailure,
    ) -> Self {
        Self {
            sequence,
            window_selection,
            outcome: CaptureAttemptOutcome::Failed(failure),
        }
    }

    /// The monitor-local sequence assigned to this attempt.
    #[must_use]
    pub const fn sequence(&self) -> CaptureAttemptSequence { self.sequence }

    /// What terminal window this completed attempt selected.
    #[must_use]
    pub const fn window_selection(&self) -> CaptureAttemptWindowSelection { self.window_selection }

    /// Whether the attempt succeeded or which capture stage failed.
    ///
    /// # Errors
    ///
    /// Returns the classified [`CaptureFailure`] when the attempt did not produce a desktop.
    pub const fn outcome(&self) -> Result<(), CaptureFailure> {
        match &self.outcome {
            #[cfg(target_os = "macos")]
            CaptureAttemptOutcome::Succeeded(_) => Ok(()),
            CaptureAttemptOutcome::Failed(failure) => Err(*failure),
        }
    }

    /// Separate the lightweight diagnostic from the desktop retained by a successful attempt.
    #[cfg(target_os = "macos")]
    pub(in crate::backdrop) fn into_diagnostic_and_desktop_result(
        self,
    ) -> (
        CompletedCaptureAttemptDiagnostic,
        Result<Arc<Desktop>, CaptureFailure>,
    ) {
        let completed_capture_attempt_diagnostic = CompletedCaptureAttemptDiagnostic {
            sequence:         self.sequence,
            window_selection: self.window_selection,
            outcome:          self.outcome(),
        };
        let desktop_result = match self.outcome {
            #[cfg(target_os = "macos")]
            CaptureAttemptOutcome::Succeeded(desktop) => Ok(desktop),
            CaptureAttemptOutcome::Failed(failure) => Err(failure),
        };
        (completed_capture_attempt_diagnostic, desktop_result)
    }

    /// Separate the lightweight diagnostic from the failed attempt result.
    #[cfg(not(target_os = "macos"))]
    pub(in crate::backdrop) const fn into_diagnostic_and_desktop_result(
        self,
    ) -> (
        CompletedCaptureAttemptDiagnostic,
        Result<Arc<Desktop>, CaptureFailure>,
    ) {
        let completed_capture_attempt_diagnostic = CompletedCaptureAttemptDiagnostic {
            sequence:         self.sequence,
            window_selection: self.window_selection,
            outcome:          self.outcome(),
        };
        let desktop_result = match self.outcome {
            CaptureAttemptOutcome::Failed(failure) => Err(failure),
        };
        (completed_capture_attempt_diagnostic, desktop_result)
    }
}

impl fmt::Debug for CaptureAttemptResult {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CaptureAttemptResult")
            .field("sequence", &self.sequence)
            .field("window_selection", &self.window_selection)
            .field("outcome", &self.outcome())
            .finish()
    }
}

/// The internal capture payload retained behind a public attempt result.
#[derive(Clone)]
enum CaptureAttemptOutcome {
    /// The attempt produced this desktop.
    #[cfg(target_os = "macos")]
    Succeeded(Arc<Desktop>),
    /// The attempt failed at this capture stage.
    Failed(CaptureFailure),
}

/// A completed capture attempt's diagnostic values without its captured desktop.
///
/// The backdrop monitor can retain these records for a diagnostic consumer without extending the
/// lifetime of any historical desktop image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletedCaptureAttemptDiagnostic {
    /// The monitor-local sequence assigned when the attempt was requested.
    sequence:         CaptureAttemptSequence,
    /// What terminal window the completed attempt selected.
    window_selection: CaptureAttemptWindowSelection,
    /// Whether the attempt succeeded or which capture stage failed.
    outcome:          Result<(), CaptureFailure>,
}

impl CompletedCaptureAttemptDiagnostic {
    /// The monitor-local sequence assigned to this attempt.
    #[must_use]
    pub const fn sequence(self) -> CaptureAttemptSequence { self.sequence }

    /// What terminal window this completed attempt selected.
    #[must_use]
    pub const fn window_selection(self) -> CaptureAttemptWindowSelection { self.window_selection }

    /// Whether the attempt succeeded or which capture stage failed.
    ///
    /// # Errors
    ///
    /// Returns the classified [`CaptureFailure`] when the attempt did not produce a desktop.
    pub const fn outcome(self) -> Result<(), CaptureFailure> { self.outcome }
}
