//! The traits an app implements to be run: [`TerminalApp`] for the app
//! itself, [`PollWork`] for the work it folds in on every pass, and
//! [`FrameProbe`] for timing the loop.

use std::io;
use std::io::Stdout;
use std::rc::Rc;
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::deadline::VisualDeadline;
use crate::AppIdentity;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Keymap;
use crate::KeymapEditContext;
use crate::SettingsHost;

/// An app [`run_terminal`](crate::run_terminal) can run.
///
/// The three required methods are the app's keymap, its frame, and a
/// left click. The rest are hooks the loop and the key ladder call at
/// fixed points, each defaulting to doing nothing, listed in the order
/// they are reached.
pub trait TerminalApp: KeymapEditContext + SettingsHost + 'static {
    /// Who the app is: [`AppIdentity::BINARY_NAME`] prefixes every
    /// message the runner prints and is the executable a restart falls
    /// back to.
    type Identity: AppIdentity;
    /// How the loop's frames are timed and its output counted.
    /// [`NoProbe`] where nothing is.
    type Probe: FrameProbe;

    /// The bindings in force.
    ///
    /// Re-read at every frame and every key: rebinding a key in the
    /// keymap overlay swaps the whole map out from under the loop.
    fn keymap(&self) -> Rc<Keymap<Self>>;

    /// Draw one frame.
    fn draw(&mut self, frame: &mut Frame, keymap: &Keymap<Self>);

    /// A left press at `position`. Every other mouse event only records
    /// where the pointer is.
    fn click(&mut self, position: Position);

    /// Called ahead of every frame, drawn or not, outside the draw
    /// itself, for work too slow to spend inside one.
    fn before_draw(&mut self) {}

    /// The earliest moment the app needs a frame without an event
    /// behind it. The loop waits no later than this or the toasts'
    /// own deadline, whichever falls first.
    fn visual_deadline(&self, _now: Instant, _frame_period: Duration) -> VisualDeadline {
        VisualDeadline::NoVisualChangeScheduled
    }

    /// First refusal on every key, for an app modal that owns the
    /// keyboard while it is open. Returning
    /// [`KeyOutcome::Consumed`] ends the key there, ahead of the
    /// framework overlays and every global.
    fn modal_key(&mut self, _keymap: &Keymap<Self>, _bind: &KeyBind) -> KeyOutcome {
        KeyOutcome::Unhandled
    }

    /// A key no framework overlay took, offered ahead of the globals.
    /// Returning [`KeyOutcome::Consumed`] ends it there.
    fn attract_key(&mut self, _keymap: &Keymap<Self>, _bind: &KeyBind) -> KeyOutcome {
        KeyOutcome::Unhandled
    }

    /// The terminal is now `area`, once per resize event.
    fn resized(&mut self, _area: Rect) {}

    /// A burst of events that resized the terminal has been drained;
    /// called once per burst, before the full repaint that follows.
    fn resize_settled(&mut self) {}

    /// Whether the periodic full repaint waits for now, for a screen
    /// that already writes every cell it covers.
    fn holds_full_repaint(&self) -> bool { false }

    /// The loop has ended, before the terminal is restored.
    fn before_exit(&mut self) {}
}

/// Work an app folds in on every pass of the loop: channels to drain,
/// timers to advance.
///
/// Kept apart from the app so the channels stay out of it, and built by
/// [`run_terminal`](crate::run_terminal)'s `start` once the terminal is
/// set up, so nothing it spawns runs before then.
pub trait PollWork<A> {
    /// Fold in whatever has arrived by `now`, saying whether the screen
    /// needs another frame for it.
    fn poll(&mut self, app: &mut A, now: Instant) -> Repaint;
}

/// Whether a pass of the loop changed something the screen shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Repaint {
    /// Draw another frame.
    Needed,
    /// Nothing on screen changed.
    NotNeeded,
}

/// A stretch of one frame a [`FrameProbe`] times on its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FramePhase {
    /// Everything `terminal.draw` does, the flush to the tty included.
    /// The runner times this one itself.
    Draw,
    /// Moving the attract screen on by a frame, the backdrop monitor's
    /// own per-frame work included.
    Advance,
    /// Reading the newest capture at where the window stands.
    Refresh,
    /// Drawing the panes, with or without their contents.
    Panes,
    /// Drawing the attract animation over them.
    Band,
}

/// Timing and output counting for the loop, for finding a stall.
pub trait FrameProbe {
    /// Where the terminal's output is written.
    type Output: io::Write;

    /// Wrap standard output once the terminal is set up, so the bytes
    /// the setup wrote are not counted.
    fn output(stdout: Stdout) -> Self::Output;

    /// A pass of the loop is starting, `gap` after the previous one
    /// started.
    fn frame_started(_gap: Duration) {}

    /// Run `body`, recording how long it took as `phase`.
    fn timed<T>(_phase: FramePhase, body: impl FnOnce() -> T) -> T { body() }

    /// Write `line` to the probe's log as a line of its own, for what
    /// happens once rather than every frame.
    fn note(_line: &str) {}

    /// Start recording every frame in full rather than only the slow
    /// ones.
    fn trace() {}
}

/// A [`FrameProbe`] that times nothing and writes straight to standard
/// output.
#[derive(Clone, Copy, Debug)]
pub enum NoProbe {}

impl FrameProbe for NoProbe {
    type Output = Stdout;

    fn output(stdout: Stdout) -> Self::Output { stdout }
}
