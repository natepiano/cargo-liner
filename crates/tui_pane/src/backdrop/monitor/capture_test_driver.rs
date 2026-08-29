//! A synchronous capture worker standing in for the window server.
//!
//! [`BackdropMonitorCaptureTestDriver`] holds the worker side of the
//! monitor's own channels, so an acceptance test drives one capture
//! attempt at a time through the production request, sequencing and
//! replacement paths without a display to capture.

use std::sync::Arc;
use std::sync::Mutex;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;

use super::BackdropMonitor;
use super::CaptureAttemptProgress;
use super::CaptureRequest;
use super::CaptureWorkerLauncher;
use super::OutstandingCaptureAttempt;
use super::window_identification::WindowIdentificationState;
use crate::backdrop::constants::CAPTURE_ATTEMPT_DEADLINE;
use crate::backdrop::desktop;
use crate::backdrop::desktop::CaptureAttemptResult;
use crate::backdrop::desktop::CaptureAttemptTestCase;
use crate::backdrop::desktop::Metrics;

/// The capture endpoints currently available to a synchronous test driver.
#[derive(Debug)]
pub(super) enum CaptureTestWorkerEndpoints {
    /// The monitor has not installed a worker yet, or no worker remains available.
    NoActiveWorker,
    /// Endpoints corresponding to the monitor's active synthetic worker.
    Active {
        /// Requests taken from the monitor.
        requests: Receiver<CaptureRequest>,
        /// Results returned to the monitor.
        captures: Sender<CaptureAttemptResult>,
    },
}

/// Why a synchronous capture-driver operation could not reach the monitor.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureTestDriverError {
    /// The monitor did not make a capture request available to the driver.
    RequestUnavailable,
    /// The driver could not return the completed capture attempt to the monitor.
    ResultUnavailable,
    /// The driver could not access the active synthetic worker endpoints.
    WorkerEndpointsUnavailable,
}

/// A synchronous capture worker used by client-crate acceptance tests.
///
/// The driver receives requests created by [`BackdropMonitor`], preserving the monitor's normal
/// sequence assignment, then runs the production window-selection helper with synthetic windows.
#[doc(hidden)]
#[derive(Debug)]
pub struct BackdropMonitorCaptureTestDriver {
    /// The current synthetic worker's endpoints, replaced whenever the monitor abandons a worker.
    endpoints: Arc<Mutex<CaptureTestWorkerEndpoints>>,
}

impl BackdropMonitor {
    /// Build a monitor with a synchronous capture driver for a client crate's acceptance tests.
    #[doc(hidden)]
    #[must_use]
    pub fn with_capture_test_driver() -> (Self, BackdropMonitorCaptureTestDriver) {
        let endpoints = Arc::new(Mutex::new(CaptureTestWorkerEndpoints::NoActiveWorker));
        let monitor = Self::with_capture_worker_launcher(CaptureWorkerLauncher::TestDriver {
            endpoints: Arc::clone(&endpoints),
        });
        let capture_test_driver = BackdropMonitorCaptureTestDriver { endpoints };
        (monitor, capture_test_driver)
    }
}

impl BackdropMonitorCaptureTestDriver {
    /// Take the request currently waiting on the active synthetic worker.
    fn take_capture_request(&self) -> Result<CaptureRequest, CaptureTestDriverError> {
        let endpoints = self
            .endpoints
            .lock()
            .map_err(|_| CaptureTestDriverError::RequestUnavailable)?;
        match &*endpoints {
            CaptureTestWorkerEndpoints::NoActiveWorker => {
                Err(CaptureTestDriverError::RequestUnavailable)
            },
            CaptureTestWorkerEndpoints::Active { requests, .. } => requests
                .try_recv()
                .map_err(|_| CaptureTestDriverError::RequestUnavailable),
        }
    }

    /// Return a completed attempt through the active synthetic worker.
    fn send_capture_result(
        &self,
        capture_attempt_result: CaptureAttemptResult,
    ) -> Result<(), CaptureTestDriverError> {
        let endpoints = self
            .endpoints
            .lock()
            .map_err(|_| CaptureTestDriverError::ResultUnavailable)?;
        match &*endpoints {
            CaptureTestWorkerEndpoints::NoActiveWorker => {
                Err(CaptureTestDriverError::ResultUnavailable)
            },
            CaptureTestWorkerEndpoints::Active { captures, .. } => captures
                .try_send(capture_attempt_result)
                .map_err(|_| CaptureTestDriverError::ResultUnavailable),
        }
    }

    /// Start one synthetic capture attempt without returning a result.
    fn start_capture_attempt(
        &self,
        monitor: &mut BackdropMonitor,
    ) -> Result<OutstandingCaptureAttempt, CaptureTestDriverError> {
        monitor.window_identification_state = WindowIdentificationState::NotAttempted;
        monitor.request_capture_if_worker_available(Metrics::for_capture_test());
        let request = self.take_capture_request()?;
        let CaptureAttemptProgress::Outstanding(outstanding_capture_attempt) =
            monitor.capture_attempt_progress
        else {
            return Err(CaptureTestDriverError::RequestUnavailable);
        };
        if request.sequence != outstanding_capture_attempt.sequence {
            return Err(CaptureTestDriverError::RequestUnavailable);
        }
        Ok(outstanding_capture_attempt)
    }

    /// Complete one synthetic attempt and leave it waiting on the monitor's capture channel.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureTestDriverError`] when the monitor cannot exchange the request or result.
    pub fn send_capture_attempt(
        &self,
        monitor: &mut BackdropMonitor,
        capture_attempt_test_case: CaptureAttemptTestCase,
    ) -> Result<(), CaptureTestDriverError> {
        monitor.window_identification_state = match capture_attempt_test_case {
            CaptureAttemptTestCase::PinnedWindow { window_id } => {
                WindowIdentificationState::Identified { window_id }
            },
            CaptureAttemptTestCase::ShareableContentQueryFails
            | CaptureAttemptTestCase::WindowOwnedByProcessAncestor { .. }
            | CaptureAttemptTestCase::WindowOwnedByTerminalProgram { .. }
            | CaptureAttemptTestCase::WindowOwnedByFrontmostApplication { .. } => {
                WindowIdentificationState::NotAttempted
            },
        };
        monitor.request_capture_if_worker_available(Metrics::for_capture_test());
        let request = self.take_capture_request()?;
        let capture_attempt_result = desktop::capture_attempt_for_test(
            request.sequence,
            request.window_target,
            capture_attempt_test_case,
        );
        self.send_capture_result(capture_attempt_result)
    }

    /// Complete one synthetic attempt and make its diagnostic available to monitor consumers.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureTestDriverError`] when the monitor cannot exchange the request or result.
    pub fn complete_capture_attempt(
        &self,
        monitor: &mut BackdropMonitor,
        capture_attempt_test_case: CaptureAttemptTestCase,
    ) -> Result<(), CaptureTestDriverError> {
        self.send_capture_attempt(monitor, capture_attempt_test_case)?;
        monitor.receive_capture_attempt_results();
        Ok(())
    }

    /// Disconnect the active synthetic worker while one capture attempt is outstanding.
    ///
    /// The monitor observes the disconnected result channel and records the outstanding attempt as
    /// [`CaptureFailure::WorkerDisconnected`](crate::CaptureFailure::WorkerDisconnected) before
    /// installing replacement endpoints.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureTestDriverError`] when the monitor cannot start an attempt or the driver
    /// cannot access the active worker endpoints.
    pub fn disconnect_capture_worker_during_attempt(
        &self,
        monitor: &mut BackdropMonitor,
    ) -> Result<(), CaptureTestDriverError> {
        self.start_capture_attempt(monitor)?;
        {
            let mut endpoints = self
                .endpoints
                .lock()
                .map_err(|_| CaptureTestDriverError::WorkerEndpointsUnavailable)?;
            if !matches!(&*endpoints, CaptureTestWorkerEndpoints::Active { .. }) {
                return Err(CaptureTestDriverError::WorkerEndpointsUnavailable);
            }
            *endpoints = CaptureTestWorkerEndpoints::NoActiveWorker;
        }
        monitor.receive_capture_attempt_results();
        Ok(())
    }

    /// Start one synthetic attempt, return no result, and advance the monitor to its deadline.
    ///
    /// The monitor abandons the synthetic worker and installs fresh driver endpoints exactly as it
    /// abandons and replaces a production worker blocked in `ScreenCaptureKit`.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureTestDriverError`] when the monitor does not make the request available.
    pub fn abandon_capture_attempt_after_deadline(
        &self,
        monitor: &mut BackdropMonitor,
    ) -> Result<(), CaptureTestDriverError> {
        let outstanding_capture_attempt = self.start_capture_attempt(monitor)?;
        monitor.recover_stalled_capture_attempt(
            outstanding_capture_attempt.requested_at + CAPTURE_ATTEMPT_DEADLINE,
        );
        Ok(())
    }
}
