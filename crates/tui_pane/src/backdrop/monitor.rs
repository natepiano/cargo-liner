//! Keeping a backdrop up to date on two clocks.
//!
//! What is behind the terminal and where the terminal is are two
//! different questions with two very different prices, and
//! [`BackdropMonitor`] is what keeps them apart.
//!
//! Capturing the display is a round trip to the window server, far
//! longer than the frame the render loop has to fill, so it goes to a
//! worker thread and runs no more often than [`CAPTURE_REFRESH`]. What
//! makes it stale is the desktop behind the window changing, and that
//! happens on the order of seconds.
//!
//! Where the window stands is asked every frame, because a window being
//! dragged changes it every frame, and it reads an image already in
//! hand -- so the colours travel with the window instead of trailing a
//! capture behind it.
//!
//! That question goes to a thread of its own rather than being asked
//! from the render loop. On its own it is cheap, a few hundred
//! microseconds, but a process has one connection to the window server
//! and it is served in order: asked while the capture worker is midway
//! through a screenshot, the same call takes tens of milliseconds and
//! the frame it was asked from is already late. Asking from elsewhere
//! puts that wait on a thread with nothing to draw, and the render loop
//! reuses the last answer for the frame or two it takes to arrive.

use std::collections::VecDeque;
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use crossbeam_channel::TryRecvError;
use crossbeam_channel::TrySendError;
use ratatui::layout::Rect;

use super::Backdrop;
use super::CaptureWindowTarget;
use super::TerminalWindowSearchOutcome;
use super::constants::CAPTURE_ATTEMPT_DEADLINE;
use super::constants::CAPTURE_REFRESH;
use super::constants::CAPTURE_RETRY;
use super::constants::IDENTIFY_MARKER;
use super::constants::IDENTIFY_PASSES;
use super::constants::IDENTIFY_RETRY;
use super::constants::MAX_CAPTURE_WORKER_REPLACEMENTS;
use super::constants::MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS;
use super::desktop;
use super::desktop::CaptureAttemptResult;
use super::desktop::CaptureAttemptSequence;
use super::desktop::CaptureAttemptTestCase;
use super::desktop::CaptureAttemptWindowSelection;
use super::desktop::CaptureFailure;
use super::desktop::CompletedCaptureAttemptDiagnostic;
use super::desktop::Desktop;
use super::desktop::Frame;
use super::desktop::Metrics;
use super::desktop::Placement;
use super::query;

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
enum WindowIdentificationState {
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
        previous_titles:   Vec<(u32, Option<String>)>,
    },
    /// Identification settled on this exact window.
    Identified { window_id: u32 },
    /// Identification attempts are exhausted and capture should use the candidate-set heuristic.
    Fallback,
}

impl WindowIdentificationState {
    /// Project the private search state into the public identification report.
    const fn report(&self) -> WindowIdentification {
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
    const fn capture_window_target(&self) -> CaptureWindowTarget {
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
    fn identify(&mut self, out: &mut impl Write) -> WindowIdentification {
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
        let terminal_window_search_outcome = query::window_origin(out)
            .map_or(TerminalWindowSearchOutcome::NotFound, desktop::window_at);
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
        previous_titles: Vec<(u32, Option<String>)>,
    ) {
        let marker = format!("{IDENTIFY_MARKER}{}", std::process::id());
        match desktop::window_titled(&marker) {
            TerminalWindowSearchOutcome::Found { window_id } => {
                let restored = previous_titles
                    .iter()
                    .find(|(id, _)| *id == window_id)
                    .and_then(|(_, title)| title.as_deref());
                let _ = set_title(out, restored.unwrap_or(""));
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

/// The result of the capture worker's latest completed attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackdropStatus {
    /// No capture attempt has returned yet.
    WaitingForFirstResult,
    /// The latest capture attempt produced a usable desktop.
    Ready,
    /// The latest capture attempt failed at the reported stage.
    Failed(CaptureFailure),
}

/// What the monitor knows about the latest completed attempt's window selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatestCaptureAttemptWindowSelection {
    /// No capture attempt has completed yet.
    WaitingForFirstResult,
    /// The newest completed attempt reached this window-selection state.
    Completed(CaptureAttemptWindowSelection),
}

/// The most recent desktop known to have captured successfully.
#[derive(Debug, Default)]
enum LastSuccessfulDesktop {
    /// No capture has succeeded yet.
    #[default]
    WaitingForFirstSuccess,
    /// The desktop produced by the most recent successful attempt.
    Available {
        /// The successful desktop.
        desktop:   Arc<Desktop>,
        /// The window-server id the desktop was captured for.
        window_id: u32,
    },
}

/// The request and result endpoints owned by one live capture worker.
#[derive(Debug)]
struct ActiveCaptureWorker {
    /// Requests sent to this worker.
    requests: Sender<CaptureRequest>,
    /// Completed attempts returned by this worker.
    captures: Receiver<CaptureAttemptResult>,
}

/// Whether the monitor still has a capture worker it can request from.
#[derive(Debug)]
enum CaptureWorkerAvailability {
    /// Requests and results are connected to this worker.
    Active(ActiveCaptureWorker),
    /// No more capture requests can be made.
    PermanentlyUnavailable,
}

/// How a monitor creates its initial and replacement capture workers.
#[derive(Debug)]
enum CaptureWorkerLauncher {
    /// Spawn an operating-system thread running [`capture_loop`].
    Threaded,
    /// Install fresh synchronous endpoints for [`BackdropMonitorCaptureTestDriver`].
    TestDriver {
        /// The endpoint slot shared with the test driver.
        endpoints: Arc<Mutex<CaptureTestWorkerEndpoints>>,
    },
}

/// The capture endpoints currently available to a synchronous test driver.
#[derive(Debug)]
enum CaptureTestWorkerEndpoints {
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

/// The most recent accepted capture request time used to pace later requests.
#[derive(Clone, Copy, Debug)]
enum CaptureRequestCadence {
    /// No request has been accepted, or a replacement worker should run immediately.
    DueImmediately,
    /// The most recent request was accepted at this instant.
    RequestedAt(Instant),
}

impl CaptureRequestCadence {
    /// Time elapsed since the latest request, or the ordinary refresh interval before the first.
    fn elapsed(self) -> Duration {
        match self {
            Self::DueImmediately => CAPTURE_REFRESH,
            Self::RequestedAt(requested_at) => requested_at.elapsed(),
        }
    }
}

/// A capture request accepted by the active worker and still awaiting its result.
#[derive(Clone, Copy, Debug)]
struct OutstandingCaptureAttempt {
    /// The monitor-local sequence assigned to this attempt.
    sequence:     CaptureAttemptSequence,
    /// When the worker accepted this attempt.
    requested_at: Instant,
}

/// Whether the active capture worker owes the monitor an attempt result.
#[derive(Clone, Copy, Debug)]
enum CaptureAttemptProgress {
    /// No capture result is outstanding.
    Idle,
    /// This attempt has not returned a result yet.
    Outstanding(OutstandingCaptureAttempt),
}

/// Why a synchronous capture-driver operation could not reach the monitor.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureTestDriverError {
    /// The monitor did not make a capture request available to the driver.
    CaptureRequestUnavailable,
    /// The driver could not return the completed capture attempt to the monitor.
    CaptureResultUnavailable,
    /// The driver could not access the active synthetic worker endpoints.
    CaptureWorkerEndpointsUnavailable,
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

impl LastSuccessfulDesktop {
    /// The successful desktop, where one has arrived.
    fn available(&self) -> Option<&Desktop> {
        match self {
            Self::WaitingForFirstSuccess => None,
            Self::Available { desktop, .. } => Some(desktop.as_ref()),
        }
    }
}

impl CaptureWorkerLauncher {
    /// Create connected channels and start or install their worker-side endpoints.
    fn launch(&self) -> Result<ActiveCaptureWorker, CaptureFailure> {
        let (requests, incoming) = crossbeam_channel::bounded(1);
        let (outgoing, captures) = crossbeam_channel::bounded(1);
        match self {
            Self::Threaded => {
                thread::Builder::new()
                    .name("backdrop-capture".to_string())
                    .spawn(move || capture_loop(&incoming, &outgoing))
                    .map_err(|_| CaptureFailure::CaptureWorkerLaunchFailed)?;
            },
            Self::TestDriver { endpoints } => {
                let Ok(mut endpoints) = endpoints.lock() else {
                    return Err(CaptureFailure::CaptureWorkerLaunchFailed);
                };
                *endpoints = CaptureTestWorkerEndpoints::Active {
                    requests: incoming,
                    captures: outgoing,
                };
            },
        }
        Ok(ActiveCaptureWorker { requests, captures })
    }
}

/// A backdrop kept up to date on two worker threads.
///
/// Every channel holds a single message. The monitor sends no second capture request while one is
/// outstanding, while a position request that arrives during the previous lookup is dropped. No
/// worker is joined, so a window server that never answers cannot hold up app exit. A capture
/// worker abandoned after [`CAPTURE_ATTEMPT_DEADLINE`] may remain blocked for the life of the
/// process; [`MAX_CAPTURE_WORKER_REPLACEMENTS`] bounds how many such threads the monitor creates.
#[derive(Debug)]
pub struct BackdropMonitor {
    /// The current worker endpoints, or that capture can no longer continue.
    capture_worker:              CaptureWorkerAvailability,
    /// How initial and replacement capture workers are launched.
    capture_worker_launcher:     CaptureWorkerLauncher,
    /// Successful replacement launches after the initial worker.
    worker_replacements:         usize,
    /// Windows the position worker should look up next.
    watches:                     Sender<u32>,
    /// Where the position worker last found the window it was given, or
    /// [`None`] where the window server would not describe it.
    frames:                      Receiver<Option<Frame>>,
    /// The newest desktop that captured successfully, retained across failures.
    last_successful_desktop:     LastSuccessfulDesktop,
    /// The outcome of the newest completed capture attempt.
    status:                      BackdropStatus,
    /// What terminal window the newest completed capture attempt selected.
    latest_window_selection:     LatestCaptureAttemptWindowSelection,
    /// Completed attempt diagnostics not yet taken by the caller.
    completed_attempts:          VecDeque<CompletedCaptureAttemptDiagnostic>,
    /// The sequence to assign to the next accepted capture request.
    next_sequence:               CaptureAttemptSequence,
    /// The newest window frame that has arrived, a frame or two behind
    /// where the window is now.
    frame:                       Option<Frame>,
    /// The area of the newest capture the caller is drawing over, read
    /// afresh every frame.
    current:                     Option<Backdrop>,
    /// Request timing used to pace routine and retry captures.
    capture_request_cadence:     CaptureRequestCadence,
    /// The attempt the active worker has not returned yet.
    capture_attempt_progress:    CaptureAttemptProgress,
    /// Where the window stood on the previous frame, which is how a
    /// window being dragged is told from one standing still.
    placement:                   Option<Placement>,
    /// Progress and phase-dependent data for terminal-window identification.
    window_identification_state: WindowIdentificationState,
}

/// One capture, as the worker is asked for it.
#[derive(Clone, Copy, Debug)]
struct CaptureRequest {
    /// The monitor-local sequence assigned to this attempt.
    sequence:      CaptureAttemptSequence,
    /// The cell sizes to reduce the capture to.
    metrics:       Metrics,
    /// Which terminal window the capture should prefer or find through the candidate heuristic.
    window_target: CaptureWindowTarget,
}

impl Default for BackdropMonitor {
    fn default() -> Self { Self::new() }
}

impl BackdropMonitor {
    /// Spawn the worker thread. No capture is taken until the first
    /// [`refresh`](Self::refresh).
    #[must_use]
    pub fn new() -> Self { Self::with_capture_worker_launcher(CaptureWorkerLauncher::Threaded) }

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

    /// Build the monitor and launch its initial capture worker.
    fn with_capture_worker_launcher(capture_worker_launcher: CaptureWorkerLauncher) -> Self {
        let (capture_worker, status) = match capture_worker_launcher.launch() {
            Ok(active_capture_worker) => (
                CaptureWorkerAvailability::Active(active_capture_worker),
                BackdropStatus::WaitingForFirstResult,
            ),
            Err(failure) => (
                CaptureWorkerAvailability::PermanentlyUnavailable,
                BackdropStatus::Failed(failure),
            ),
        };
        let (watches, asked) = crossbeam_channel::bounded(1);
        let (located, frames) = crossbeam_channel::bounded(1);
        // A failed spawn leaves `frames` disconnected, no frame ever
        // arrives, and `placement` stays `None` -- the drawing stops
        // rather than being placed somewhere it is not.
        drop(
            thread::Builder::new()
                .name("backdrop-position".to_string())
                .spawn(move || position_loop(&asked, &located)),
        );
        Self {
            capture_worker,
            capture_worker_launcher,
            worker_replacements: 0,
            watches,
            frames,
            last_successful_desktop: LastSuccessfulDesktop::default(),
            status,
            latest_window_selection: LatestCaptureAttemptWindowSelection::WaitingForFirstResult,
            completed_attempts: VecDeque::new(),
            next_sequence: CaptureAttemptSequence::FIRST,
            frame: None,
            current: None,
            capture_request_cadence: CaptureRequestCadence::DueImmediately,
            capture_attempt_progress: CaptureAttemptProgress::Idle,
            placement: None,
            window_identification_state: WindowIdentificationState::default(),
        }
    }

    /// Settle which of the emulator's windows this app is drawn in.
    ///
    /// Answers whether identification has not started, is pending, has
    /// settled on a window, or has exhausted its passes and handed
    /// selection to the frontmost-or-size fallback.
    ///
    /// Two ways of asking, in order of how much they can be trusted.
    ///
    /// The terminal is asked outright where its window stands, and the
    /// window standing there is this app's. This is asked once: it
    /// races nothing, so a terminal that did not answer does not know
    /// the query.
    ///
    /// Failing that, the terminal is made to wear a title only this
    /// process knows for as long as it takes to ask the window server
    /// who is wearing it. This is tried again on a pace, up to
    /// `IDENTIFY_PASSES`, because it does race: a terminal that will
    /// not wear a title will not wear one on the second ask either,
    /// but a title that merely lost a race to a busy emulator will be
    /// worn on a later one, and only the passes tell the two apart.
    /// The size heuristic carries the run once they are spent.
    ///
    /// A title is the weaker of the two for a reason the reader
    /// supplies: it is the one piece of a window's state both they and
    /// the emulator can pin, and a pinned title is never replaced by
    /// the marker.
    ///
    /// # Cost
    ///
    /// One round trip over the pty, then two to the window server. The
    /// window server is asked the cheap way, so what this costs is very
    /// nearly all the pty: a terminal that answers costs the reply, and
    /// one that does not costs the wait for a reply that never comes.
    /// It belongs where the backdrop is first wanted rather than in a
    /// frame that has to be drawn on time.
    ///
    /// # Invariants
    ///
    /// `out` must be the terminal this app is drawn on, and nothing
    /// else may write to it or read from it while this runs. A title
    /// is set with an escape sequence, and a sequence cut in half sets
    /// no title and leaves its tail on the screen; the query is
    /// answered on this app's own input, and a second reader takes the
    /// bytes this one is waiting for.
    pub fn identify(&mut self, out: &mut impl Write) -> WindowIdentification {
        self.window_identification_state.identify(out)
    }

    /// Take delivery of anything either worker has finished, read `area`
    /// out of the capture at where the window stands, and ask for
    /// whatever is due next. Never blocks, and never calls the window
    /// server itself.
    pub fn refresh(&mut self, area: Rect) {
        self.receive_capture_attempt_results();
        self.recover_stalled_capture_attempt(Instant::now());
        while let Ok(frame) = self.frames.try_recv() {
            self.frame = frame;
        }
        // Ask where the window is for the next frame. A full channel
        // means the position worker is still on the last ask -- which
        // is what a capture in flight ahead of it leaves -- and the
        // frame already in hand carries this frame instead.
        if let LastSuccessfulCaptureWindowId::Available { window_id } = self.captured_window_id() {
            let _ = self.watches.try_send(window_id);
        }
        let metrics = Metrics::read();
        // A capture whose placement cannot be read is one whose window
        // has closed or moved to another display, and either way a
        // fresh one is what answers.
        let placement = match (self.last_successful_desktop.available(), self.frame) {
            (Some(desktop), Some(frame)) => desktop.placement(frame),
            _ => None,
        };
        if let (Some(desktop), Some(placement)) =
            (self.last_successful_desktop.available(), placement)
        {
            self.current = Some(Backdrop::read(desktop, placement, area));
        }
        // A window that has moved since the last frame is one being
        // dragged, and a drag is the one thing that changes nothing a
        // capture holds: the capture covers the whole display with this
        // window taken out of it, so where the window is does not enter
        // into it. Asking the window server to composite the display
        // again while it is already busy moving a window is work for
        // nothing, paid at the exact moment the animation can least
        // afford it -- so a moving window asks for no captures, and the
        // one owed is taken as soon as it comes to rest.
        let moving = placement.is_some() && placement != self.placement;
        self.placement = placement;
        let usable = placement.is_some()
            && self
                .last_successful_desktop
                .available()
                .is_some_and(|desktop| Some(desktop.metrics()) == metrics);
        let waited = self.capture_request_cadence.elapsed();
        // A capture that cannot be used is worth asking to replace
        // sooner than the routine cycle, but not every frame: each
        // attempt costs the worker the same long round trip whether or
        // not it succeeds.
        let due = !moving && (waited >= CAPTURE_REFRESH || (!usable && waited >= CAPTURE_RETRY));
        if let (true, Some(metrics)) = (due, metrics) {
            self.request_capture_if_worker_available(metrics);
        }
    }

    /// Send a sequenced capture request unless the worker is still busy with the previous one.
    fn request_capture_if_worker_available(&mut self, metrics: Metrics) {
        if matches!(
            self.capture_attempt_progress,
            CaptureAttemptProgress::Outstanding(_)
        ) {
            return;
        }
        let request = CaptureRequest {
            sequence: self.next_sequence,
            metrics,
            window_target: self.window_identification_state.capture_window_target(),
        };
        let send_result = match &self.capture_worker {
            CaptureWorkerAvailability::Active(active_capture_worker) => {
                active_capture_worker.requests.try_send(request)
            },
            CaptureWorkerAvailability::PermanentlyUnavailable => return,
        };
        match send_result {
            Ok(()) => {
                let requested_at = Instant::now();
                self.capture_request_cadence = CaptureRequestCadence::RequestedAt(requested_at);
                self.capture_attempt_progress =
                    CaptureAttemptProgress::Outstanding(OutstandingCaptureAttempt {
                        sequence: request.sequence,
                        requested_at,
                    });
                self.next_sequence = self.next_sequence.following();
            },
            Err(TrySendError::Full(_)) => {},
            Err(TrySendError::Disconnected(_)) => self.handle_disconnected_capture_worker(),
        }
    }

    /// Move every capture worker result currently available into retained monitor state.
    fn receive_capture_attempt_results(&mut self) {
        loop {
            let receive_result = match &self.capture_worker {
                CaptureWorkerAvailability::Active(active_capture_worker) => {
                    active_capture_worker.captures.try_recv()
                },
                CaptureWorkerAvailability::PermanentlyUnavailable => return,
            };
            match receive_result {
                Ok(capture_attempt_result) => {
                    self.record_capture_attempt_result(capture_attempt_result);
                },
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.handle_disconnected_capture_worker();
                    return;
                },
            }
        }
    }

    /// Record and abandon the outstanding attempt once its deadline has elapsed.
    fn recover_stalled_capture_attempt(&mut self, now: Instant) {
        let CaptureAttemptProgress::Outstanding(outstanding_capture_attempt) =
            self.capture_attempt_progress
        else {
            return;
        };
        if now.saturating_duration_since(outstanding_capture_attempt.requested_at)
            < CAPTURE_ATTEMPT_DEADLINE
        {
            return;
        }
        self.record_capture_attempt_result(CaptureAttemptResult::failed(
            outstanding_capture_attempt.sequence,
            CaptureAttemptWindowSelection::SelectionNotReached,
            CaptureFailure::CaptureAttemptStalled,
        ));
        self.replace_capture_worker();
    }

    /// Record a disconnected worker and replace it when the replacement allowance remains.
    fn handle_disconnected_capture_worker(&mut self) {
        match self.capture_attempt_progress {
            CaptureAttemptProgress::Idle => {
                self.status = BackdropStatus::Failed(CaptureFailure::CaptureWorkerDisconnected);
            },
            CaptureAttemptProgress::Outstanding(outstanding_capture_attempt) => {
                self.record_capture_attempt_result(CaptureAttemptResult::failed(
                    outstanding_capture_attempt.sequence,
                    CaptureAttemptWindowSelection::SelectionNotReached,
                    CaptureFailure::CaptureWorkerDisconnected,
                ));
            },
        }
        self.replace_capture_worker();
    }

    /// Drop the abandoned worker endpoints and launch a fresh worker within the process bound.
    fn replace_capture_worker(&mut self) {
        self.capture_worker = CaptureWorkerAvailability::PermanentlyUnavailable;
        if self.worker_replacements == MAX_CAPTURE_WORKER_REPLACEMENTS {
            self.status =
                BackdropStatus::Failed(CaptureFailure::CaptureWorkerReplacementLimitReached);
            return;
        }
        match self.capture_worker_launcher.launch() {
            Ok(active_capture_worker) => {
                self.capture_worker = CaptureWorkerAvailability::Active(active_capture_worker);
                self.worker_replacements += 1;
                self.capture_request_cadence = CaptureRequestCadence::DueImmediately;
            },
            Err(failure) => self.status = BackdropStatus::Failed(failure),
        }
    }

    /// Retain the outcome and diagnostic values from one completed attempt.
    fn record_capture_attempt_result(&mut self, capture_attempt_result: CaptureAttemptResult) {
        if matches!(
            self.capture_attempt_progress,
            CaptureAttemptProgress::Outstanding(outstanding_capture_attempt)
                if outstanding_capture_attempt.sequence == capture_attempt_result.sequence()
        ) {
            self.capture_attempt_progress = CaptureAttemptProgress::Idle;
        }
        let (completed_capture_attempt_diagnostic, desktop_result) =
            capture_attempt_result.into_diagnostic_and_desktop_result();
        self.latest_window_selection = LatestCaptureAttemptWindowSelection::Completed(
            completed_capture_attempt_diagnostic.window_selection(),
        );
        match desktop_result {
            Ok(desktop) => {
                let window_id = desktop.window_id();
                self.last_successful_desktop =
                    LastSuccessfulDesktop::Available { desktop, window_id };
                self.status = BackdropStatus::Ready;
            },
            Err(failure) => self.status = BackdropStatus::Failed(failure),
        }
        if self.completed_attempts.len() == MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS {
            let _ = self.completed_attempts.pop_front();
        }
        self.completed_attempts
            .push_back(completed_capture_attempt_diagnostic);
    }

    /// The newest backdrop, or [`None`] until one arrives.
    ///
    /// A backdrop read before the terminal was resized is still handed
    /// back: [`Backdrop::color_at`] refuses cells outside it, so the
    /// drawing thins out rather than stopping while a fresh capture is
    /// on its way.
    #[must_use]
    pub const fn current(&self) -> Option<&Backdrop> { self.current.as_ref() }

    /// The window id used by the most recent successful capture.
    ///
    /// A later capture failure does not discard this id because the monitor retains the last
    /// successful desktop separately from the latest attempt status.
    #[must_use]
    pub const fn captured_window_id(&self) -> LastSuccessfulCaptureWindowId {
        match &self.last_successful_desktop {
            LastSuccessfulDesktop::WaitingForFirstSuccess => {
                LastSuccessfulCaptureWindowId::WaitingForFirstSuccess
            },
            LastSuccessfulDesktop::Available { window_id, .. } => {
                LastSuccessfulCaptureWindowId::Available {
                    window_id: *window_id,
                }
            },
        }
    }

    /// The result of the latest capture attempt completed by the worker.
    #[must_use]
    pub const fn status(&self) -> BackdropStatus { self.status }

    /// What the latest completed capture attempt selected, or that none has completed yet.
    #[must_use]
    pub const fn latest_capture_attempt_window_selection(
        &self,
    ) -> LatestCaptureAttemptWindowSelection {
        self.latest_window_selection
    }

    /// Take every retained completed-attempt diagnostic not returned by an earlier call.
    ///
    /// Diagnostics are returned in capture order, including consecutive attempts with identical
    /// selections and outcomes. A caller that drains them before the retention bound is reached
    /// loses none. If more diagnostics accumulate, the monitor discards the oldest first instead
    /// of retaining a queue without limit. Each diagnostic is lightweight and does not retain a
    /// captured desktop.
    pub fn take_completed_capture_attempt_diagnostics(
        &mut self,
    ) -> impl Iterator<Item = CompletedCaptureAttemptDiagnostic> + '_ {
        self.receive_capture_attempt_results();
        self.completed_attempts.drain(..)
    }
}

impl BackdropMonitorCaptureTestDriver {
    /// Maximum replacement workers launched after the initial capture worker.
    pub const MAX_CAPTURE_WORKER_REPLACEMENTS: usize = MAX_CAPTURE_WORKER_REPLACEMENTS;
    /// Maximum completed-attempt diagnostics retained by a monitor between drains.
    pub const MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS: usize =
        MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS;

    /// Take the request currently waiting on the active synthetic worker.
    fn take_capture_request(&self) -> Result<CaptureRequest, CaptureTestDriverError> {
        let endpoints = self
            .endpoints
            .lock()
            .map_err(|_| CaptureTestDriverError::CaptureRequestUnavailable)?;
        match &*endpoints {
            CaptureTestWorkerEndpoints::NoActiveWorker => {
                Err(CaptureTestDriverError::CaptureRequestUnavailable)
            },
            CaptureTestWorkerEndpoints::Active { requests, .. } => requests
                .try_recv()
                .map_err(|_| CaptureTestDriverError::CaptureRequestUnavailable),
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
            .map_err(|_| CaptureTestDriverError::CaptureResultUnavailable)?;
        match &*endpoints {
            CaptureTestWorkerEndpoints::NoActiveWorker => {
                Err(CaptureTestDriverError::CaptureResultUnavailable)
            },
            CaptureTestWorkerEndpoints::Active { captures, .. } => captures
                .try_send(capture_attempt_result)
                .map_err(|_| CaptureTestDriverError::CaptureResultUnavailable),
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
            return Err(CaptureTestDriverError::CaptureRequestUnavailable);
        };
        if request.sequence != outstanding_capture_attempt.sequence {
            return Err(CaptureTestDriverError::CaptureRequestUnavailable);
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
    /// [`CaptureFailure::CaptureWorkerDisconnected`] before installing replacement endpoints.
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
                .map_err(|_| CaptureTestDriverError::CaptureWorkerEndpointsUnavailable)?;
            if !matches!(&*endpoints, CaptureTestWorkerEndpoints::Active { .. }) {
                return Err(CaptureTestDriverError::CaptureWorkerEndpointsUnavailable);
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

/// Worker loop: capture the display for each cell size asked for and
/// send the result back. Exits when the monitor drops and the request
/// channel disconnects.
fn capture_loop(requests: &Receiver<CaptureRequest>, captures: &Sender<CaptureAttemptResult>) {
    while let Ok(request) = requests.recv() {
        if captures
            .send(Desktop::capture(
                request.metrics,
                request.window_target,
                request.sequence,
            ))
            .is_err()
        {
            break;
        }
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

/// Worker loop: look up each window asked about and send back where it
/// stands, or [`None`] where the window server will not describe it.
/// Exits when the monitor drops and the request channel disconnects.
///
/// Nothing paces this: it is asked once per frame and answers once per
/// frame, and while the capture worker is holding the window server it
/// simply waits there rather than in the render loop.
fn position_loop(watches: &Receiver<u32>, frames: &Sender<Option<Frame>>) {
    while let Ok(window) = watches.recv() {
        if frames.send(desktop::window_frame(window)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::CaptureWindowTarget;
    use super::IDENTIFY_PASSES;
    use super::TerminalWindowSearchOutcome;
    use super::WindowIdentification;
    use super::WindowIdentificationState;
    use super::window_identification;

    const WINDOW_ID: u32 = 42;

    #[test]
    fn identification_is_not_attempted_before_a_pass_runs() {
        assert_eq!(
            window_identification(0, TerminalWindowSearchOutcome::NotFound),
            WindowIdentification::NotAttempted,
        );
    }

    #[test]
    fn a_found_window_is_identified_with_its_window_id() {
        assert_eq!(
            window_identification(
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
            window_identification(1, TerminalWindowSearchOutcome::NotFound),
            WindowIdentification::Pending,
        );
    }

    #[test]
    fn the_last_remaining_allowance_stays_pending() {
        assert_eq!(
            window_identification(IDENTIFY_PASSES - 1, TerminalWindowSearchOutcome::NotFound,),
            WindowIdentification::Pending,
        );
    }

    #[test]
    fn exhausting_every_allowance_uses_fallback() {
        assert_eq!(
            window_identification(IDENTIFY_PASSES, TerminalWindowSearchOutcome::NotFound,),
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
