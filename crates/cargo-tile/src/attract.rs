//! The attract screen: what the terminal shows while no cargo is
//! running.
//!
//! A grid with nothing in it is a screen with nothing to say, so the
//! app spends that time showing what is behind it. [`tui_pane`]
//! captures the desktop under the terminal window and hands back one
//! colour per character cell; [`TravelingBand`] draws a strip of
//! characters crossing the grid in those colours, so the text reads as
//! cut out of whatever the window is sitting on top of.
//!
//! The strip fades in when the roster empties and back out when
//! something starts, which is why [`Attract::draw`] is called every
//! frame rather than only while idle -- the frames after work arrives
//! are the ones that carry it off the screen.
//!
//! It can also be asked for outright, with the key bound to
//! [`AppGlobalAction::Attract`](crate::globals::AppGlobalAction). A
//! screen that only ever appears when there is nothing to build is one
//! that cannot be looked at on purpose -- and the reader wanting to
//! watch it is reason enough to show it over a grid that is busy. Asked
//! for, it takes the terminal rather than sharing it:
//! [`Attract::covers_grid`] tells [`crate::render`] to leave the panes
//! out entirely, so what is drawn is the animation and the status line
//! and nothing else.

use std::time::Duration;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::BackdropMonitor;
use tui_pane::TravelingBand;
use tui_pane::pane_background;

use crate::app::Updates;
use crate::constants::ATTRACT_FADE_STEP;

/// Whether the display has any cargo to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Work {
    /// Nothing is running, so the attract screen has the terminal.
    Idle,
    /// Something is running, so the attract screen gives it back.
    Running,
}

/// The attract screen's state between frames.
pub(crate) struct Attract {
    /// Keeps the captured desktop up to date on a worker thread.
    monitor:     BackdropMonitor,
    /// The strip of characters crossing the grid.
    band:        TravelingBand,
    /// How far the strip is carried toward the ground it is drawn on,
    /// on the alpha scale [`tui_pane::blend_color`] reads. Starts at
    /// [`u8::MAX`] so the app opens with nothing over its grid.
    faded:       u8,
    /// When the strip was last moved on, so its speed is a speed rather
    /// than a step per frame.
    advanced_at: Instant,
    /// Whether the reader has asked for the strip outright, which shows
    /// it over a grid with work on it as readily as over an empty one.
    asked_for:   bool,
    /// Whether the grid is being left out of the frame altogether. Not
    /// the same as [`Self::asked_for`]: it outlasts it, by the fade the
    /// strip takes to leave.
    covering:    bool,
    /// Whether the display was being held still when the strip was last
    /// drawn, which is what says the gap since then is not travel the
    /// strip owes.
    held:        bool,
}

impl Attract {
    /// An attract screen that is not yet showing.
    pub(crate) fn new() -> Self {
        Self {
            monitor:     BackdropMonitor::new(),
            band:        TravelingBand::new(),
            faded:       u8::MAX,
            advanced_at: Instant::now(),
            asked_for:   false,
            covering:    false,
            held:        false,
        }
    }

    /// Ask for the strip, or give the grid back.
    ///
    /// Asking covers the grid from this moment rather than from the
    /// next frame: the panes are drawn before the strip is, so waiting
    /// would show one frame of the grid with the strip over it -- the
    /// very look this is here to avoid.
    pub(crate) const fn toggle(&mut self) {
        self.asked_for = !self.asked_for;
        if self.asked_for {
            self.covering = true;
        }
    }

    /// Whether the strip is being shown because it was asked for, which
    /// is what the status line says: a grid taken off the screen by the
    /// attract screen otherwise looks exactly like a grid with nothing
    /// on it.
    pub(crate) const fn asked_for(&self) -> bool { self.asked_for }

    /// Whether the panes are left out of the frame altogether.
    ///
    /// A strip of characters drawn across a grid of borders and tables
    /// reads as neither one thing nor the other, so an attract screen
    /// that was asked for replaces the grid instead of covering it.
    ///
    /// It outlasts the asking by the fade: giving the grid back the
    /// instant the key is pressed would hand the strip the panes to
    /// fade out over, which is the same bad look arriving on the way
    /// out instead of on the way in.
    pub(crate) const fn covers_grid(&self) -> bool { self.covering }

    /// Whether the strip is anywhere on the screen, which is what the
    /// event loop asks to know it owes another frame.
    ///
    /// The loop is otherwise demand-driven: nothing typed and no scan
    /// come back different means nothing repaints. An animation is the
    /// one thing on this screen that moves with no event behind it, and
    /// it runs precisely while the app is idle -- so without this it
    /// would draw one frame and stop. Fully faded out it wants nothing,
    /// which is what hands the idle app its quiet back.
    pub(crate) const fn showing(&self) -> bool { self.faded != u8::MAX }

    /// Carry the strip one frame further in or out of view and draw it
    /// over `area`.
    ///
    /// Draws nothing at all once it has faded the whole way out, which
    /// is also where it stops asking for fresh captures: an app with
    /// work on the screen has no use for what is behind it.
    pub(crate) fn draw(&mut self, buffer: &mut Buffer, area: Rect, work: Work, updates: Updates) {
        let now = Instant::now();
        // A freeze just let go of leaves a gap between this draw and
        // the one before it that the strip does not owe: the display
        // stood still, so the strip stood still with it. The gap is not
        // a frame's worth either -- the loop asks for no frames at all
        // while frozen, so the last draw before this one was the full
        // repaint on its timer, seconds back. Travelling it would carry
        // the strip most of the way across the screen the instant the
        // reader let go, which is what a held display is least expected
        // to do.
        let elapsed = if self.held {
            Duration::ZERO
        } else {
            now.duration_since(self.advanced_at)
        };
        self.advanced_at = now;
        self.held = updates == Updates::Frozen;
        if updates == Updates::Frozen {
            self.render(buffer, area);
            return;
        }
        // Asked for, the roster does not get a say: the strip comes in
        // over whatever is on the grid and stays until it is asked to
        // go, so it can be watched rather than only caught.
        let work = if self.asked_for { Work::Idle } else { work };
        self.faded = match work {
            Work::Idle => self.faded.saturating_sub(ATTRACT_FADE_STEP),
            Work::Running => self.faded.saturating_add(ATTRACT_FADE_STEP),
        };
        // The grid comes back only once the strip has gone the whole
        // way, which is also where there is nothing left to draw.
        self.covering = self.asked_for || (self.covering && self.faded != u8::MAX);
        if self.faded == u8::MAX {
            return;
        }

        self.monitor.refresh(area);
        self.band.advance(area, elapsed);
        self.band.fade(self.faded);
        self.render(buffer, area);
    }

    /// Draw the strip where it currently stands, moving nothing.
    fn render(&self, buffer: &mut Buffer, area: Rect) {
        let Some(backdrop) = self.monitor.current() else {
            return;
        };
        self.band.render(area, backdrop, ground(), buffer);
    }
}

/// The colour the strip's tail fades toward.
///
/// A profile the app is drawn transparent in paints no ground of its
/// own, and a colour with no channels is one nothing can be mixed
/// against -- so the strip fades toward black, which is what absent
/// looks like when the desktop is showing through.
fn ground() -> Color {
    match pane_background(false) {
        Color::Reset => Color::Black,
        background => background,
    }
}
