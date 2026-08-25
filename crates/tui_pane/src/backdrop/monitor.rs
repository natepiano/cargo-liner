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
//! dragged changes it every frame. It costs a fraction of a millisecond
//! and reads an image already in hand, so the colours travel with the
//! window instead of trailing a capture behind it.

use std::thread;
use std::time::Instant;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use ratatui::layout::Rect;

use super::Backdrop;
use super::constants::CAPTURE_REFRESH;
use super::constants::CAPTURE_RETRY;
use super::desktop::Desktop;
use super::desktop::Metrics;
use super::desktop::Placement;

/// A backdrop kept up to date on a worker thread.
///
/// Both channels hold a single message: a request that arrives while
/// the worker is busy is dropped rather than queued, because a capture
/// nobody is waiting for any more is worth nothing. The worker is never
/// joined -- it exits when the monitor drops and its request channel
/// disconnects, and a capture the window server never answers must not
/// be able to hold up the app's exit.
#[derive(Debug)]
pub struct BackdropMonitor {
    /// Cell sizes the worker should capture for next.
    requests:     Sender<Metrics>,
    /// Displays the worker has finished capturing and reducing.
    captures:     Receiver<Desktop>,
    /// The newest capture that has arrived.
    desktop:      Option<Desktop>,
    /// The area of the newest capture the caller is drawing over, read
    /// afresh every frame.
    current:      Option<Backdrop>,
    /// When the last request went out, whether or not it was answered.
    requested_at: Option<Instant>,
    /// Where the window stood on the previous frame, which is how a
    /// window being dragged is told from one standing still.
    placement:    Option<Placement>,
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
        // leaves `captures` disconnected and every `current` answering
        // `None` -- the same as a platform with no capture backend.
        drop(
            thread::Builder::new()
                .name("backdrop-capture".to_string())
                .spawn(move || capture_loop(&incoming, &outgoing)),
        );
        Self {
            requests,
            captures,
            desktop: None,
            current: None,
            requested_at: None,
            placement: None,
        }
    }

    /// Take delivery of anything the worker has finished, read `area`
    /// out of it at where the window stands now, and ask for a fresh
    /// capture when one is due. Never blocks.
    pub fn refresh(&mut self, area: Rect) {
        while let Ok(desktop) = self.captures.try_recv() {
            self.desktop = Some(desktop);
        }
        let metrics = Metrics::read();
        // Where the window is, every frame. A capture whose placement
        // cannot be read is one whose window has closed or moved to
        // another display, and either way a fresh one is what answers.
        let placement = self.desktop.as_ref().and_then(Desktop::placement);
        if let (Some(desktop), Some(placement)) = (self.desktop.as_ref(), placement) {
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
                .desktop
                .as_ref()
                .is_some_and(|desktop| Some(desktop.metrics()) == metrics);
        let waited = self.requested_at.map_or(CAPTURE_REFRESH, |at| at.elapsed());
        // A capture that cannot be used is worth asking to replace
        // sooner than the routine cycle, but not every frame: each
        // attempt costs the worker the same long round trip whether or
        // not it succeeds.
        let due = !moving && (waited >= CAPTURE_REFRESH || (!usable && waited >= CAPTURE_RETRY));
        if let (true, Some(metrics)) = (due, metrics) {
            // A full channel means the worker is still on the last
            // request; dropping this one is the point of the bound.
            if self.requests.try_send(metrics).is_ok() {
                self.requested_at = Some(Instant::now());
            }
        }
    }

    /// The newest backdrop, or [`None`] until one arrives.
    ///
    /// A backdrop read before the terminal was resized is still handed
    /// back: [`Backdrop::color_at`] refuses cells outside it, so the
    /// drawing thins out rather than stopping while a fresh capture is
    /// on its way.
    #[must_use]
    pub const fn current(&self) -> Option<&Backdrop> { self.current.as_ref() }
}

/// Worker loop: capture the display for each cell size asked for and
/// send the result back. Exits when the monitor drops and the request
/// channel disconnects.
fn capture_loop(requests: &Receiver<Metrics>, captures: &Sender<Desktop>) {
    while let Ok(metrics) = requests.recv() {
        let Some(desktop) = Desktop::capture(metrics) else {
            continue;
        };
        if captures.send(desktop).is_err() {
            break;
        }
    }
}
