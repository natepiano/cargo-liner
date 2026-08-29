//! What the status line says when the desktop behind the terminal
//! cannot be drawn, and the diagnostics written on the way there.
//!
//! [`classify_backdrop_notice`] answers with a [`BackdropNotice`] from
//! four inputs and nothing else, so what the reader is told about
//! capture is settled in one place rather than in the frame that draws
//! it. The diagnostic records are the same story written for the probe
//! log rather than the screen.

use std::time::Instant;

use tui_pane::BackdropStatus;
use tui_pane::CaptureAttemptSequence;
use tui_pane::CaptureAttemptWindowSelection;
use tui_pane::CaptureFailure;
use tui_pane::CompletedCaptureAttemptDiagnostic;
use tui_pane::LastSuccessfulCaptureWindowId;
use tui_pane::LatestCaptureAttemptWindowSelection;
use tui_pane::WindowIdentification;

/// What the status line should say about desktop capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackdropNotice {
    /// Do not draw a notice.
    None,
    /// Tell the reader how to grant Screen Recording access.
    ScreenRecordingAccessInstruction,
    /// Report that a capture attempt exceeded its deadline and was abandoned.
    CaptureStalled,
    /// Report that repeated worker abandonment exhausted the replacement bound.
    CaptureRecoveryStopped,
    /// Report that capture is unavailable and diagnostics recorded why.
    CaptureUnavailable,
}

/// Whether the attract screen can present a backdrop notice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AttractScreenVisibility {
    /// The ordinary working grid is visible instead of the attract screen.
    Hidden,
    /// The attract screen is visible.
    Showing,
}

/// Whether the missing-backdrop grace period has elapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BackdropGracePeriod {
    /// The grace period still gives the capture worker time to reply.
    Remaining,
    /// The grace period has elapsed without a current backdrop.
    Elapsed,
}

/// Whether the attract screen is waiting for a desktop backdrop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BackdropWait {
    /// A desktop is on screen, so no missing backdrop is being timed.
    NotWaiting,
    /// No desktop has been available since this instant.
    WaitingSince(Instant),
}

/// Whether the monitor has a desktop that renderers can use now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CurrentBackdrop {
    /// No usable desktop is available.
    Missing,
    /// A usable desktop is available, including one retained after a later failure.
    Available,
}

/// Values written together when attract backdrop diagnostics change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BackdropDiagnostic {
    /// The most recent window-selection report.
    pub(super) window_identification:   WindowIdentification,
    /// The capture worker's latest completed result.
    pub(super) backdrop_status:         BackdropStatus,
    /// The window id used by the last successful capture, if one has succeeded.
    pub(super) captured_window_id:      LastSuccessfulCaptureWindowId,
    /// What terminal window the latest completed capture attempt selected.
    pub(super) latest_window_selection: LatestCaptureAttemptWindowSelection,
}

/// Values written for each completed backdrop capture attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BackdropAttemptDiagnostic {
    /// The monitor-local sequence assigned to the attempt.
    sequence:         CaptureAttemptSequence,
    /// What terminal window the completed attempt selected.
    window_selection: CaptureAttemptWindowSelection,
    /// Whether the capture succeeded or which stage failed.
    backdrop_status:  BackdropStatus,
}

impl From<CompletedCaptureAttemptDiagnostic> for BackdropAttemptDiagnostic {
    fn from(completed_capture_attempt_diagnostic: CompletedCaptureAttemptDiagnostic) -> Self {
        let backdrop_status = completed_capture_attempt_diagnostic
            .outcome()
            .map_or_else(BackdropStatus::Failed, |()| BackdropStatus::Ready);
        Self {
            sequence: completed_capture_attempt_diagnostic.sequence(),
            window_selection: completed_capture_attempt_diagnostic.window_selection(),
            backdrop_status,
        }
    }
}

/// Format the transition-only backdrop summary record.
pub(super) fn backdrop_diagnostic_record(backdrop_diagnostic: BackdropDiagnostic) -> String {
    format!(
        "backdrop: report={:?} capture_status={:?} captured_window_id={:?} \
         latest_attempt_window_selection={:?}",
        backdrop_diagnostic.window_identification,
        backdrop_diagnostic.backdrop_status,
        backdrop_diagnostic.captured_window_id,
        backdrop_diagnostic.latest_window_selection,
    )
}

/// Write one record for every completed capture attempt in order.
pub(super) fn note_backdrop_attempts<T>(
    capture_attempts: impl IntoIterator<Item = T>,
    mut note: impl FnMut(&str),
) where
    T: Into<BackdropAttemptDiagnostic>,
{
    for capture_attempt in capture_attempts {
        let backdrop_attempt_diagnostic = capture_attempt.into();
        note(&format!(
            "backdrop_attempt: sequence={:?} window_selection={:?} capture_status={:?}",
            backdrop_attempt_diagnostic.sequence,
            backdrop_attempt_diagnostic.window_selection,
            backdrop_attempt_diagnostic.backdrop_status,
        ));
    }
}

/// Select the status-line outcome from attract visibility and capture state.
///
/// A hidden attract screen suppresses every notice. While the screen is showing, stalled recovery
/// failures take priority over a retained current backdrop; other statuses remain subject to the
/// current-backdrop and grace-period suppression.
pub(super) const fn classify_backdrop_notice(
    attract_screen_visibility: AttractScreenVisibility,
    grace_period: BackdropGracePeriod,
    current_backdrop: CurrentBackdrop,
    backdrop_status: BackdropStatus,
) -> BackdropNotice {
    match (
        attract_screen_visibility,
        grace_period,
        current_backdrop,
        backdrop_status,
    ) {
        (
            AttractScreenVisibility::Showing,
            _,
            _,
            BackdropStatus::Failed(CaptureFailure::AttemptStalled),
        ) => BackdropNotice::CaptureStalled,
        (
            AttractScreenVisibility::Showing,
            _,
            _,
            BackdropStatus::Failed(CaptureFailure::WorkerReplacementLimitReached),
        ) => BackdropNotice::CaptureRecoveryStopped,
        (AttractScreenVisibility::Hidden, _, _, _)
        | (AttractScreenVisibility::Showing, _, CurrentBackdrop::Available, _)
        | (
            AttractScreenVisibility::Showing,
            BackdropGracePeriod::Remaining,
            CurrentBackdrop::Missing,
            _,
        ) => BackdropNotice::None,
        (
            AttractScreenVisibility::Showing,
            BackdropGracePeriod::Elapsed,
            CurrentBackdrop::Missing,
            BackdropStatus::Failed(CaptureFailure::ScreenRecordingAccessNotGranted),
        ) => BackdropNotice::ScreenRecordingAccessInstruction,
        (
            AttractScreenVisibility::Showing,
            BackdropGracePeriod::Elapsed,
            CurrentBackdrop::Missing,
            BackdropStatus::WaitingForFirstResult
            | BackdropStatus::Ready
            | BackdropStatus::Failed(_),
        ) => BackdropNotice::CaptureUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use tui_pane::BackdropMonitor;
    use tui_pane::CaptureAttemptTestCase;
    use tui_pane::CaptureWindowSelectionMethod;
    use tui_pane::TerminalWindowCandidateSource;

    use super::*;

    /// Capture failure stages exercised by the notice classifier.
    const CAPTURE_FAILURES: [CaptureFailure; 12] = [
        CaptureFailure::UnsupportedPlatform,
        CaptureFailure::AttemptStalled,
        CaptureFailure::WorkerLaunchFailed,
        CaptureFailure::WorkerDisconnected,
        CaptureFailure::WorkerReplacementLimitReached,
        CaptureFailure::ScreenRecordingAccessNotGranted,
        CaptureFailure::ShareableContentQueryFailed,
        CaptureFailure::TerminalWindowNotFound,
        CaptureFailure::DisplayNotFound,
        CaptureFailure::ScreenshotFailed,
        CaptureFailure::PixelExtractionFailed,
        CaptureFailure::ImageReductionFailed,
    ];

    /// Collect records from the production per-attempt writer.
    fn capture_attempt_records<T>(capture_attempts: impl IntoIterator<Item = T>) -> Vec<String>
    where
        T: Into<BackdropAttemptDiagnostic>,
    {
        let mut records = Vec::new();
        note_backdrop_attempts(capture_attempts, |record| records.push(record.to_owned()));
        records
    }

    #[test]
    fn backdrop_summary_reports_the_latest_attempt_window_selection() {
        let latest_window_selection = LatestCaptureAttemptWindowSelection::Completed(
            CaptureAttemptWindowSelection::Selected {
                window_id: 42,
                method:    CaptureWindowSelectionMethod::PinnedWindow,
            },
        );
        let backdrop_diagnostic = BackdropDiagnostic {
            window_identification: WindowIdentification::Identified { window_id: 42 },
            backdrop_status: BackdropStatus::Ready,
            captured_window_id: LastSuccessfulCaptureWindowId::Available { window_id: 42 },
            latest_window_selection,
        };

        let record = backdrop_diagnostic_record(backdrop_diagnostic);

        assert!(record.contains(&format!(
            "latest_attempt_window_selection={latest_window_selection:?}"
        )));
    }

    #[test]
    fn failed_attempt_before_window_selection_reports_selection_not_reached() {
        let (mut monitor, capture_test_driver) = BackdropMonitor::with_capture_test_driver();
        assert_eq!(
            capture_test_driver.complete_capture_attempt(
                &mut monitor,
                CaptureAttemptTestCase::ShareableContentQueryFails,
            ),
            Ok(()),
        );

        let records = capture_attempt_records(monitor.take_completed_capture_attempt_diagnostics());

        assert_eq!(
            records,
            ["backdrop_attempt: sequence=CaptureAttemptSequence(1) \
              window_selection=SelectionNotReached \
              capture_status=Failed(ShareableContentQueryFailed)"],
        );
    }

    #[test]
    fn every_capture_window_selection_method_is_reported() {
        let (mut monitor, capture_test_driver) = BackdropMonitor::with_capture_test_driver();
        let cases = [
            (
                CaptureAttemptTestCase::PinnedWindow { window_id: 41 },
                CaptureAttemptWindowSelection::Selected {
                    window_id: 41,
                    method:    CaptureWindowSelectionMethod::PinnedWindow,
                },
            ),
            (
                CaptureAttemptTestCase::WindowOwnedByProcessAncestor { window_id: 42 },
                CaptureAttemptWindowSelection::Selected {
                    window_id: 42,
                    method:    CaptureWindowSelectionMethod::ClosestSizeMatch {
                        terminal_window_candidate_source:
                            TerminalWindowCandidateSource::ProcessAncestry,
                    },
                },
            ),
            (
                CaptureAttemptTestCase::WindowOwnedByTerminalProgram { window_id: 43 },
                CaptureAttemptWindowSelection::Selected {
                    window_id: 43,
                    method:    CaptureWindowSelectionMethod::ClosestSizeMatch {
                        terminal_window_candidate_source:
                            TerminalWindowCandidateSource::TerminalProgramName,
                    },
                },
            ),
            (
                CaptureAttemptTestCase::WindowOwnedByFrontmostApplication { window_id: 44 },
                CaptureAttemptWindowSelection::Selected {
                    window_id: 44,
                    method:    CaptureWindowSelectionMethod::ClosestSizeMatch {
                        terminal_window_candidate_source:
                            TerminalWindowCandidateSource::FrontmostApplication,
                    },
                },
            ),
        ];
        for (capture_attempt_test_case, _) in cases {
            assert_eq!(
                capture_test_driver
                    .complete_capture_attempt(&mut monitor, capture_attempt_test_case),
                Ok(()),
            );
        }
        let completed_capture_attempt_diagnostics: Vec<_> = monitor
            .take_completed_capture_attempt_diagnostics()
            .collect();

        let records =
            capture_attempt_records(completed_capture_attempt_diagnostics.iter().copied());

        assert_eq!(records.len(), cases.len());
        for ((record, completed_capture_attempt_diagnostic), (_, window_selection)) in records
            .iter()
            .zip(&completed_capture_attempt_diagnostics)
            .zip(cases)
        {
            assert_eq!(
                completed_capture_attempt_diagnostic.window_selection(),
                window_selection,
            );
            assert!(record.contains(&format!("window_selection={window_selection:?}")));
        }
    }

    #[test]
    fn identical_attempt_outcomes_write_distinct_sequence_records() {
        let (mut monitor, capture_test_driver) = BackdropMonitor::with_capture_test_driver();
        for _ in 0..2 {
            assert_eq!(
                capture_test_driver.complete_capture_attempt(
                    &mut monitor,
                    CaptureAttemptTestCase::ShareableContentQueryFails,
                ),
                Ok(()),
            );
        }
        let completed_capture_attempt_diagnostics: Vec<_> = monitor
            .take_completed_capture_attempt_diagnostics()
            .collect();

        let records =
            capture_attempt_records(completed_capture_attempt_diagnostics.iter().copied());

        assert_eq!(
            records,
            [
                "backdrop_attempt: sequence=CaptureAttemptSequence(1) \
                 window_selection=SelectionNotReached \
                 capture_status=Failed(ShareableContentQueryFailed)",
                "backdrop_attempt: sequence=CaptureAttemptSequence(2) \
                 window_selection=SelectionNotReached \
                 capture_status=Failed(ShareableContentQueryFailed)",
            ],
        );
        assert_ne!(
            completed_capture_attempt_diagnostics[0].sequence(),
            completed_capture_attempt_diagnostics[1].sequence(),
        );
    }

    #[test]
    fn hidden_attract_screen_suppresses_every_backdrop_status() {
        for grace_period in [BackdropGracePeriod::Remaining, BackdropGracePeriod::Elapsed] {
            for current_backdrop in [CurrentBackdrop::Missing, CurrentBackdrop::Available] {
                for backdrop_status in
                    [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready]
                {
                    assert_eq!(
                        classify_backdrop_notice(
                            AttractScreenVisibility::Hidden,
                            grace_period,
                            current_backdrop,
                            backdrop_status,
                        ),
                        BackdropNotice::None,
                        "grace_period={grace_period:?} current_backdrop={current_backdrop:?} \
                         backdrop_status={backdrop_status:?}",
                    );
                }
                for failure in CAPTURE_FAILURES {
                    assert_eq!(
                        classify_backdrop_notice(
                            AttractScreenVisibility::Hidden,
                            grace_period,
                            current_backdrop,
                            BackdropStatus::Failed(failure),
                        ),
                        BackdropNotice::None,
                        "grace_period={grace_period:?} current_backdrop={current_backdrop:?} \
                         failure={failure:?}",
                    );
                }
            }
        }
    }

    #[test]
    fn backdrop_notice_waits_for_the_grace_period_except_after_a_worker_stalls() {
        for backdrop_status in [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready] {
            assert_eq!(
                classify_backdrop_notice(
                    AttractScreenVisibility::Showing,
                    BackdropGracePeriod::Remaining,
                    CurrentBackdrop::Missing,
                    backdrop_status,
                ),
                BackdropNotice::None,
                "backdrop_status={backdrop_status:?}",
            );
        }
        for failure in CAPTURE_FAILURES {
            let expected = match failure {
                CaptureFailure::AttemptStalled => BackdropNotice::CaptureStalled,
                CaptureFailure::WorkerReplacementLimitReached => {
                    BackdropNotice::CaptureRecoveryStopped
                },
                CaptureFailure::UnsupportedPlatform
                | CaptureFailure::WorkerLaunchFailed
                | CaptureFailure::WorkerDisconnected
                | CaptureFailure::ScreenRecordingAccessNotGranted
                | CaptureFailure::ShareableContentQueryFailed
                | CaptureFailure::TerminalWindowNotFound
                | CaptureFailure::DisplayNotFound
                | CaptureFailure::ScreenshotFailed
                | CaptureFailure::PixelExtractionFailed
                | CaptureFailure::ImageReductionFailed => BackdropNotice::None,
            };
            assert_eq!(
                classify_backdrop_notice(
                    AttractScreenVisibility::Showing,
                    BackdropGracePeriod::Remaining,
                    CurrentBackdrop::Missing,
                    BackdropStatus::Failed(failure),
                ),
                expected,
                "failure={failure:?}",
            );
        }
    }

    #[test]
    fn overdue_missing_backdrop_reports_waiting_and_ready_as_unavailable() {
        for backdrop_status in [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready] {
            assert_eq!(
                classify_backdrop_notice(
                    AttractScreenVisibility::Showing,
                    BackdropGracePeriod::Elapsed,
                    CurrentBackdrop::Missing,
                    backdrop_status,
                ),
                BackdropNotice::CaptureUnavailable,
                "backdrop_status={backdrop_status:?}",
            );
        }
    }

    #[test]
    fn only_access_failure_selects_the_screen_recording_instruction() {
        for failure in CAPTURE_FAILURES {
            let expected = match failure {
                CaptureFailure::ScreenRecordingAccessNotGranted => {
                    BackdropNotice::ScreenRecordingAccessInstruction
                },
                CaptureFailure::AttemptStalled => BackdropNotice::CaptureStalled,
                CaptureFailure::WorkerReplacementLimitReached => {
                    BackdropNotice::CaptureRecoveryStopped
                },
                CaptureFailure::UnsupportedPlatform
                | CaptureFailure::WorkerLaunchFailed
                | CaptureFailure::WorkerDisconnected
                | CaptureFailure::ShareableContentQueryFailed
                | CaptureFailure::TerminalWindowNotFound
                | CaptureFailure::DisplayNotFound
                | CaptureFailure::ScreenshotFailed
                | CaptureFailure::PixelExtractionFailed
                | CaptureFailure::ImageReductionFailed => BackdropNotice::CaptureUnavailable,
            };
            assert_eq!(
                classify_backdrop_notice(
                    AttractScreenVisibility::Showing,
                    BackdropGracePeriod::Elapsed,
                    CurrentBackdrop::Missing,
                    BackdropStatus::Failed(failure),
                ),
                expected,
                "failure={failure:?}",
            );
        }
    }

    #[test]
    fn current_backdrop_suppresses_every_failure_notice_except_a_stalled_worker() {
        for grace_period in [BackdropGracePeriod::Remaining, BackdropGracePeriod::Elapsed] {
            for backdrop_status in [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready] {
                assert_eq!(
                    classify_backdrop_notice(
                        AttractScreenVisibility::Showing,
                        grace_period,
                        CurrentBackdrop::Available,
                        backdrop_status,
                    ),
                    BackdropNotice::None,
                    "grace_period={grace_period:?} backdrop_status={backdrop_status:?}",
                );
            }
            for failure in CAPTURE_FAILURES {
                let expected = match failure {
                    CaptureFailure::AttemptStalled => BackdropNotice::CaptureStalled,
                    CaptureFailure::WorkerReplacementLimitReached => {
                        BackdropNotice::CaptureRecoveryStopped
                    },
                    CaptureFailure::UnsupportedPlatform
                    | CaptureFailure::WorkerLaunchFailed
                    | CaptureFailure::WorkerDisconnected
                    | CaptureFailure::ScreenRecordingAccessNotGranted
                    | CaptureFailure::ShareableContentQueryFailed
                    | CaptureFailure::TerminalWindowNotFound
                    | CaptureFailure::DisplayNotFound
                    | CaptureFailure::ScreenshotFailed
                    | CaptureFailure::PixelExtractionFailed
                    | CaptureFailure::ImageReductionFailed => BackdropNotice::None,
                };
                assert_eq!(
                    classify_backdrop_notice(
                        AttractScreenVisibility::Showing,
                        grace_period,
                        CurrentBackdrop::Available,
                        BackdropStatus::Failed(failure),
                    ),
                    expected,
                    "grace_period={grace_period:?} failure={failure:?}",
                );
            }
        }
    }
}
