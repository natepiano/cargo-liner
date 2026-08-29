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
use std::thread;
use std::time::Instant;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use ratatui::layout::Rect;

use super::Backdrop;
use super::constants::CAPTURE_REFRESH;
use super::constants::CAPTURE_RETRY;
use super::constants::IDENTIFY_MARKER;
use super::constants::IDENTIFY_PASSES;
use super::constants::IDENTIFY_RETRY;
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

/// Whether an identification pass settled on a window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowSearchOutcome {
    /// No window was found.
    NotFound,
    /// A window was found with this window-server id.
    Found { window_id: u32 },
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

/// Why a synchronous capture-driver operation could not reach the monitor.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureTestDriverError {
    /// The monitor did not make a capture request available to the driver.
    CaptureRequestUnavailable,
    /// The driver could not return the completed capture attempt to the monitor.
    CaptureResultUnavailable,
}

/// A synchronous capture worker used by client-crate acceptance tests.
///
/// The driver receives requests created by [`BackdropMonitor`], preserving the monitor's normal
/// sequence assignment, then runs the production window-selection helper with synthetic windows.
#[doc(hidden)]
#[derive(Debug)]
pub struct BackdropMonitorCaptureTestDriver {
    /// Requests sent by the monitor under test.
    requests: Receiver<Request>,
    /// Completed attempts returned to the monitor under test.
    captures: Sender<CaptureAttemptResult>,
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

/// A backdrop kept up to date on two worker threads.
///
/// Every channel holds a single message: a request that arrives while
/// its worker is busy is dropped rather than queued, because an answer
/// nobody is waiting for any more is worth nothing. Neither worker is
/// ever joined -- each exits when the monitor drops and its request
/// channel disconnects, and a window server that never answers must
/// not be able to hold up the app's exit.
#[derive(Debug)]
pub struct BackdropMonitor {
    /// What the capture worker should capture for next.
    requests:                Sender<Request>,
    /// Outcomes the capture worker has finished producing.
    captures:                Receiver<CaptureAttemptResult>,
    /// Windows the position worker should look up next.
    watches:                 Sender<u32>,
    /// Where the position worker last found the window it was given, or
    /// [`None`] where the window server would not describe it.
    frames:                  Receiver<Option<Frame>>,
    /// The newest desktop that captured successfully, retained across failures.
    last_successful_desktop: LastSuccessfulDesktop,
    /// The outcome of the newest completed capture attempt.
    status:                  BackdropStatus,
    /// What terminal window the newest completed capture attempt selected.
    latest_window_selection: LatestCaptureAttemptWindowSelection,
    /// Completed attempt diagnostics not yet taken by the caller.
    completed_attempts:      VecDeque<CompletedCaptureAttemptDiagnostic>,
    /// The sequence to assign to the next accepted capture request.
    next_sequence:           CaptureAttemptSequence,
    /// The newest window frame that has arrived, a frame or two behind
    /// where the window is now.
    frame:                   Option<Frame>,
    /// The area of the newest capture the caller is drawing over, read
    /// afresh every frame.
    current:                 Option<Backdrop>,
    /// When the last capture request went out, whether or not it was
    /// answered.
    requested_at:            Option<Instant>,
    /// Where the window stood on the previous frame, which is how a
    /// window being dragged is told from one standing still.
    placement:               Option<Placement>,
    /// The window this app was found to be drawn in, once
    /// [`identify`](Self::identify) has settled it.
    pinned:                  Option<u32>,
    /// How many passes at settling on a window have been made, so that
    /// a terminal which will not wear a title is given up on rather
    /// than asked once a frame for the length of the run.
    attempts:                u32,
    /// When the last pass was made, which is what paces them: a title
    /// needs time to reach the emulator and the window server, and
    /// asking again inside that time only loses the same race twice.
    attempted_at:            Option<Instant>,
    /// Whether the emulator has been asked outright where its window
    /// stands.
    ///
    /// Asked once and no more, unlike the marker title. A title loses
    /// races -- to a busy emulator, to a title the reader pinned --
    /// and losing one says nothing about the next. The query races
    /// nothing: it is flushed, so the emulator has it in hand before
    /// the wait starts, and a terminal that did not answer it then
    /// does not know it.
    asked:                   bool,
    /// What every window was titled before this app's own put the
    /// marker on, so that the window found wearing it can be given its
    /// title back.
    ///
    /// Read once, on the pass that sets the marker, and kept because
    /// the marker outlives that pass -- see
    /// [`identify`](Self::identify).
    titles:                  Option<Vec<(u32, Option<String>)>>,
}

/// One capture, as the worker is asked for it.
#[derive(Clone, Copy, Debug)]
struct Request {
    /// The monitor-local sequence assigned to this attempt.
    sequence: CaptureAttemptSequence,
    /// The cell sizes to reduce the capture to.
    metrics:  Metrics,
    /// The window to capture the display behind, where one has been
    /// settled on.
    window:   Option<u32>,
}

impl Default for BackdropMonitor {
    fn default() -> Self { Self::new() }
}

impl BackdropMonitor {
    /// Spawn the worker thread. No capture is taken until the first
    /// [`refresh`](Self::refresh).
    #[must_use]
    pub fn new() -> Self {
        let (requests, incoming) = crossbeam_channel::bounded(1);
        let (outgoing, captures) = crossbeam_channel::bounded(1);
        // A failed spawn drops `outgoing` with the unrun closure, which
        // leaves `captures` disconnected and the status waiting for its
        // first result, while every `current` answers `None`.
        drop(
            thread::Builder::new()
                .name("backdrop-capture".to_string())
                .spawn(move || capture_loop(&incoming, &outgoing)),
        );
        Self::with_capture_channels(requests, captures)
    }

    /// Build a monitor with a synchronous capture driver for a client crate's acceptance tests.
    #[doc(hidden)]
    #[must_use]
    pub fn with_capture_test_driver() -> (Self, BackdropMonitorCaptureTestDriver) {
        let (requests, incoming) = crossbeam_channel::bounded(1);
        let (outgoing, captures) = crossbeam_channel::bounded(1);
        let monitor = Self::with_capture_channels(requests, captures);
        let capture_test_driver = BackdropMonitorCaptureTestDriver {
            requests: incoming,
            captures: outgoing,
        };
        (monitor, capture_test_driver)
    }

    /// Build the monitor around a capture worker's two channel endpoints.
    fn with_capture_channels(
        requests: Sender<Request>,
        captures: Receiver<CaptureAttemptResult>,
    ) -> Self {
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
            requests,
            captures,
            watches,
            frames,
            last_successful_desktop: LastSuccessfulDesktop::default(),
            status: BackdropStatus::WaitingForFirstResult,
            latest_window_selection: LatestCaptureAttemptWindowSelection::WaitingForFirstResult,
            completed_attempts: VecDeque::new(),
            next_sequence: CaptureAttemptSequence::FIRST,
            frame: None,
            current: None,
            requested_at: None,
            placement: None,
            pinned: None,
            attempts: 0,
            attempted_at: None,
            asked: false,
            titles: None,
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
        if let Some(window_id) = self.pinned {
            return window_identification(self.attempts, WindowSearchOutcome::Found { window_id });
        }
        if self.attempts >= IDENTIFY_PASSES {
            return window_identification(self.attempts, WindowSearchOutcome::NotFound);
        }
        // Paced rather than run back to back: what a pass is waiting on
        // is the emulator draining whatever stands between it and the
        // marker, and nothing about that is faster for being asked
        // again immediately.
        if self
            .attempted_at
            .is_some_and(|at| at.elapsed() < IDENTIFY_RETRY)
        {
            return window_identification(self.attempts, WindowSearchOutcome::NotFound);
        }
        self.attempts += 1;
        self.attempted_at = Some(Instant::now());
        // Asking the emulator where it is comes first, and settles it
        // outright where the emulator answers. Nothing below runs then
        // -- no title is set, and no window wears a marker that has to
        // be taken off again.
        if !self.asked {
            self.asked = true;
            self.pinned = query::window_origin(out).and_then(desktop::window_at);
            if let Some(window_id) = self.pinned {
                return window_identification(
                    self.attempts,
                    WindowSearchOutcome::Found { window_id },
                );
            }
        }
        let marker = format!("{IDENTIFY_MARKER}{}", std::process::id());
        // The marker goes on once and is left on until something
        // answers it.
        //
        // What a pass is waiting for is the emulator drawing the title
        // and the window server coming to see it, and the pause between
        // passes is the only part of this long enough for that to
        // happen. A marker put on and taken off inside one pass is worn
        // for as long as the asking takes -- a fraction of a
        // millisecond, now that the window server is asked the cheap
        // way -- and no emulator is quick enough to be caught wearing
        // it.
        if self.titles.is_none() {
            // What every window is titled now, so that the one found to
            // be wearing the marker can be given its own title back.
            // Read before the marker goes on, and kept only once it
            // has: a marker that could not be set is not worn, and a
            // pass that took this for worn would look for a title
            // nothing ever wore.
            let titles = desktop::window_titles();
            if set_title(out, &marker).is_err() {
                return window_identification(self.attempts, WindowSearchOutcome::NotFound);
            }
            self.titles = Some(titles);
        }
        let found = desktop::window_titled(&marker);
        // The marker comes off once it has been answered, and once
        // there is no pass left to answer it. Nothing else takes it
        // off, so a run ending between those two leaves the emulator
        // wearing the marker until whatever usually writes the title --
        // a shell prompt, in every ordinary setup -- writes it again.
        // That is the price of the marker being worn long enough to be
        // seen at all.
        if found.is_some() || self.attempts >= IDENTIFY_PASSES {
            let restored = found
                .and_then(|window| self.titles.as_ref()?.iter().find(|(id, _)| *id == window))
                .and_then(|(_, title)| title.as_deref());
            // An empty title is what a window that had none goes back
            // to, and it is also all there is to offer for a window the
            // window server would not describe -- the emulator settles
            // what to show in its place.
            let _ = set_title(out, restored.unwrap_or(""));
        }
        self.pinned = found;
        let window_search_outcome = found.map_or(WindowSearchOutcome::NotFound, |window_id| {
            WindowSearchOutcome::Found { window_id }
        });
        window_identification(self.attempts, window_search_outcome)
    }

    /// Take delivery of anything either worker has finished, read `area`
    /// out of the capture at where the window stands, and ask for
    /// whatever is due next. Never blocks, and never calls the window
    /// server itself.
    pub fn refresh(&mut self, area: Rect) {
        self.receive_capture_attempt_results();
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
        let waited = self.requested_at.map_or(CAPTURE_REFRESH, |at| at.elapsed());
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
        let request = Request {
            sequence: self.next_sequence,
            metrics,
            window: self.pinned,
        };
        // A full channel means the worker is still on the last
        // request; dropping this one is the point of the bound.
        if self.requests.try_send(request).is_ok() {
            self.requested_at = Some(Instant::now());
            self.next_sequence = self.next_sequence.following();
        }
    }

    /// Move every capture worker result currently available into retained monitor state.
    fn receive_capture_attempt_results(&mut self) {
        while let Ok(capture_attempt_result) = self.captures.try_recv() {
            self.record_capture_attempt_result(capture_attempt_result);
        }
    }

    /// Retain the outcome and diagnostic values from one completed attempt.
    fn record_capture_attempt_result(&mut self, capture_attempt_result: CaptureAttemptResult) {
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
    /// Maximum completed-attempt diagnostics retained by a monitor between drains.
    pub const MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS: usize =
        MAX_RETAINED_CAPTURE_ATTEMPT_DIAGNOSTICS;

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
        monitor.pinned = match capture_attempt_test_case {
            CaptureAttemptTestCase::PinnedWindow { window_id } => Some(window_id),
            CaptureAttemptTestCase::ShareableContentQueryFails
            | CaptureAttemptTestCase::WindowOwnedByProcessAncestor { .. }
            | CaptureAttemptTestCase::WindowOwnedByTerminalProgram { .. }
            | CaptureAttemptTestCase::WindowOwnedByFrontmostApplication { .. } => None,
        };
        monitor.request_capture_if_worker_available(Metrics::for_capture_test());
        let request = self
            .requests
            .try_recv()
            .map_err(|_| CaptureTestDriverError::CaptureRequestUnavailable)?;
        let capture_attempt_result = desktop::capture_attempt_for_test(
            request.sequence,
            request.window,
            capture_attempt_test_case,
        );
        self.captures
            .try_send(capture_attempt_result)
            .map_err(|_| CaptureTestDriverError::CaptureResultUnavailable)
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
}

/// Map consumed identification attempts to the progress reported to callers.
const fn window_identification(
    attempts_consumed: u32,
    window_search_outcome: WindowSearchOutcome,
) -> WindowIdentification {
    match window_search_outcome {
        WindowSearchOutcome::Found { window_id } => WindowIdentification::Identified { window_id },
        WindowSearchOutcome::NotFound => match attempts_consumed {
            0 => WindowIdentification::NotAttempted,
            IDENTIFY_PASSES.. => WindowIdentification::Fallback,
            _ => WindowIdentification::Pending,
        },
    }
}

/// Worker loop: capture the display for each cell size asked for and
/// send the result back. Exits when the monitor drops and the request
/// channel disconnects.
fn capture_loop(requests: &Receiver<Request>, captures: &Sender<CaptureAttemptResult>) {
    while let Ok(request) = requests.recv() {
        if captures
            .send(Desktop::capture(
                request.metrics,
                request.window,
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
    use super::IDENTIFY_PASSES;
    use super::WindowIdentification;
    use super::WindowSearchOutcome;
    use super::window_identification;

    const WINDOW_ID: u32 = 42;

    #[test]
    fn identification_is_not_attempted_before_a_pass_runs() {
        assert_eq!(
            window_identification(0, WindowSearchOutcome::NotFound),
            WindowIdentification::NotAttempted,
        );
    }

    #[test]
    fn a_found_window_is_identified_with_its_window_id() {
        assert_eq!(
            window_identification(
                1,
                WindowSearchOutcome::Found {
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
            window_identification(1, WindowSearchOutcome::NotFound),
            WindowIdentification::Pending,
        );
    }

    #[test]
    fn the_last_remaining_allowance_stays_pending() {
        assert_eq!(
            window_identification(IDENTIFY_PASSES - 1, WindowSearchOutcome::NotFound),
            WindowIdentification::Pending,
        );
    }

    #[test]
    fn exhausting_every_allowance_uses_fallback() {
        assert_eq!(
            window_identification(IDENTIFY_PASSES, WindowSearchOutcome::NotFound),
            WindowIdentification::Fallback,
        );
    }
}
