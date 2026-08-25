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

use std::io;
use std::io::Write;
use std::thread;
use std::time::Instant;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use ratatui::layout::Rect;

use super::Backdrop;
use super::constants::CAPTURE_REFRESH;
use super::constants::CAPTURE_RETRY;
use super::constants::IDENTIFY_ATTEMPTS;
use super::constants::IDENTIFY_MARKER;
use super::desktop;
use super::desktop::Desktop;
use super::desktop::Frame;
use super::desktop::Metrics;
use super::desktop::Placement;

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
    requests:     Sender<Request>,
    /// Displays the capture worker has finished capturing and reducing.
    captures:     Receiver<Desktop>,
    /// Windows the position worker should look up next.
    watches:      Sender<u32>,
    /// Where the position worker last found the window it was given, or
    /// [`None`] where the window server would not describe it.
    frames:       Receiver<Option<Frame>>,
    /// The newest capture that has arrived.
    desktop:      Option<Desktop>,
    /// The newest window frame that has arrived, a frame or two behind
    /// where the window is now.
    frame:        Option<Frame>,
    /// The area of the newest capture the caller is drawing over, read
    /// afresh every frame.
    current:      Option<Backdrop>,
    /// When the last capture request went out, whether or not it was
    /// answered.
    requested_at: Option<Instant>,
    /// Where the window stood on the previous frame, which is how a
    /// window being dragged is told from one standing still.
    placement:    Option<Placement>,
    /// The window this app was found to be drawn in, once
    /// [`identify`](Self::identify) has settled it.
    pinned:       Option<u32>,
    /// Whether settling on a window has been tried, so that a terminal
    /// which will not wear a title is asked once rather than once a
    /// frame.
    attempted:    bool,
}

/// One capture, as the worker is asked for it.
#[derive(Clone, Copy, Debug)]
struct Request {
    /// The cell sizes to reduce the capture to.
    metrics: Metrics,
    /// The window to capture the display behind, where one has been
    /// settled on.
    window:  Option<u32>,
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
            desktop: None,
            frame: None,
            current: None,
            requested_at: None,
            placement: None,
            pinned: None,
            attempted: false,
        }
    }

    /// Settle which of the emulator's windows this app is drawn in, by
    /// having the terminal wear a title only this process knows for as
    /// long as it takes to ask the window server who is wearing it.
    ///
    /// Answers whether a window has been settled on. Tried once and
    /// once only: a terminal that will not wear a title will not wear
    /// one on the second ask either, and the size heuristic behind
    /// this is what carries the run then.
    ///
    /// Without this the window is picked by size, and two windows of
    /// the same size cannot be told apart that way: what arrives then
    /// is the desktop behind a sibling window rather than behind this
    /// one.
    ///
    /// # Cost
    ///
    /// Several round trips to the window server, a few hundred
    /// milliseconds in all. It belongs where the backdrop is first
    /// wanted rather than in a frame that has to be drawn on time.
    ///
    /// # Invariants
    ///
    /// `out` must be the terminal this app is drawn on, and nothing
    /// else may write to it while this runs: the title is set with an
    /// escape sequence, and a sequence cut in half sets no title and
    /// leaves its tail on the screen.
    pub fn identify(&mut self, out: &mut impl Write) -> bool {
        if self.attempted {
            return self.pinned.is_some();
        }
        self.attempted = true;
        let marker = format!("{IDENTIFY_MARKER}{}", std::process::id());
        // What every window is titled now, so that the one found to be
        // wearing the marker can be given its own title back.
        let titles = desktop::window_titles();
        if set_title(out, &marker).is_err() {
            return false;
        }
        // The title has to reach the emulator, be drawn, and reach the
        // window server before it can be asked about, and none of that
        // is instant. Nothing paces the attempts because each one is
        // itself a long round trip.
        let found = (0..IDENTIFY_ATTEMPTS).find_map(|_| desktop::window_titled(&marker));
        let restored = found
            .and_then(|window| titles.iter().find(|(id, _)| *id == window))
            .and_then(|(_, title)| title.as_deref());
        // An empty title is what a window that had none goes back to,
        // and it is also all there is to offer for a window the window
        // server would not describe -- the emulator settles what to
        // show in its place.
        let _ = set_title(out, restored.unwrap_or(""));
        self.pinned = found;
        found.is_some()
    }

    /// Take delivery of anything either worker has finished, read `area`
    /// out of the capture at where the window stands, and ask for
    /// whatever is due next. Never blocks, and never calls the window
    /// server itself.
    pub fn refresh(&mut self, area: Rect) {
        while let Ok(desktop) = self.captures.try_recv() {
            self.desktop = Some(desktop);
        }
        while let Ok(frame) = self.frames.try_recv() {
            self.frame = frame;
        }
        // Ask where the window is for the next frame. A full channel
        // means the position worker is still on the last ask -- which
        // is what a capture in flight ahead of it leaves -- and the
        // frame already in hand carries this frame instead.
        if let Some(window) = self.desktop.as_ref().map(Desktop::window) {
            let _ = self.watches.try_send(window);
        }
        let metrics = Metrics::read();
        // A capture whose placement cannot be read is one whose window
        // has closed or moved to another display, and either way a
        // fresh one is what answers.
        let placement = match (self.desktop.as_ref(), self.frame) {
            (Some(desktop), Some(frame)) => desktop.placement(frame),
            _ => None,
        };
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
            let request = Request {
                metrics,
                window: self.pinned,
            };
            // A full channel means the worker is still on the last
            // request; dropping this one is the point of the bound.
            if self.requests.try_send(request).is_ok() {
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
fn capture_loop(requests: &Receiver<Request>, captures: &Sender<Desktop>) {
    while let Ok(request) = requests.recv() {
        let Some(desktop) = Desktop::capture(request.metrics, request.window) else {
            continue;
        };
        if captures.send(desktop).is_err() {
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
