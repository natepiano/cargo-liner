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
//! Which animation is drawn is an [`AttractMode`], and the mode is also
//! the keymap scope the reader's keys resolve against while the screen
//! has been asked for: `+` widens the moving band rather than opening a
//! tile, and a second mode added later binds the same key to whatever
//! it wants. See [`moving_band`].
//!
//! It can also be asked for outright, with the key bound to
//! [`AppGlobalAction::Attract`](crate::globals::AppGlobalAction). A
//! screen that only ever appears when there is nothing to build is one
//! that cannot be looked at on purpose -- and the reader wanting to
//! watch it is reason enough to show it over a grid that is busy. Asked
//! for, it takes the terminal rather than sharing it: [`Attract::grid`]
//! tells [`crate::render`] to leave the panes out, so what is drawn is
//! the animation and the status line and nothing else.
//!
//! Neither end of that is abrupt. [`Grid::Empty`] holds the panes on
//! screen with nothing in them for as long as the strip is arriving or
//! leaving, and carries them toward the colour they are painted on in
//! step with it. What that buys is a background: a strip fading out
//! over bare terminal has nothing to fade into and goes dark instead of
//! going away, and content appearing under a strip still crossing it is
//! the crowded look the screen exists to avoid.

mod moving_band;

use std::io;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::BackdropMonitor;
use tui_pane::BandDirection;
use tui_pane::TravelingBand;
use tui_pane::pane_background;

use self::moving_band::HeldKey;
use self::moving_band::MovingBandAction;
pub(crate) use self::moving_band::MovingBandPane;
use crate::app::Updates;
use crate::constants::ATTRACT_FADE_STEP;
use crate::constants::BAND_SPEED_STEP;
use crate::constants::BAND_TAIL_SPEED_STEP;
use crate::constants::BAND_WIDTH_STEP;
use crate::probe;

/// What [`crate::render`] should do with the tile grid this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Grid {
    /// Draw it in full. The attract screen is either off the terminal
    /// or decorating an idle grid rather than replacing it.
    Full,
    /// Draw the panes with nothing in them, carried this far toward the
    /// colour they are painted on.
    ///
    /// Zero is the grid's own chrome at full strength, which is the
    /// first frame after the strip is asked for and the last before it
    /// finishes leaving; [`u8::MAX`] is that chrome gone.
    Empty(u8),
    /// Leave it out of the frame altogether. The strip has the terminal.
    Off,
}

/// Whether the display has any cargo to show.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Work {
    /// Nothing is running, so the attract screen has the terminal.
    Idle,
    /// Something is running, so the attract screen gives it back.
    Running,
}

/// What the reader has said about the strip, which outranks what the
/// roster says about it.
///
/// Two answers would not be enough. The strip comes on by itself over
/// an idle grid, so "not asked for" and "asked to go" are the same
/// state to the roster and opposite ones to the reader -- and reading
/// them as one is what left `a` unable to put the strip away at
/// exactly the moment it is being watched.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Asked {
    /// Nothing either way, which leaves it to the roster.
    #[default]
    Nothing,
    /// For the strip, which brings it in over a grid with work on it as
    /// readily as over an empty one.
    For,
    /// Against it, which sends it away over an idle grid, where the
    /// roster would otherwise be keeping it.
    Against,
}

/// Which animation the attract screen is drawing.
///
/// Also the keymap scope its keys resolve against: each variant is an
/// [`AppPaneId::Attract`](crate::app::AppPaneId) of its own, so two
/// animations can bind the same key to different things and
/// `keymap.toml` keeps a table for each.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) enum AttractMode {
    /// A lit strip of characters crossing the grid, drawn in the
    /// colours of the desktop behind the window.
    #[default]
    MovingBand,
}

/// The attract screen's state between frames.
pub(crate) struct Attract {
    /// Keeps the captured desktop up to date on a worker thread.
    monitor:     BackdropMonitor,
    /// Which animation is being drawn, and which keymap scope the
    /// reader's keys resolve against while it is on screen.
    mode:        AttractMode,
    /// The strip of characters crossing the grid.
    band:        TravelingBand,
    /// How far into a run of presses of one steering key the reader is,
    /// which is what lets a held key move the band further per press.
    held_key:    HeldKey,
    /// How far the strip is carried toward the ground it is drawn on,
    /// on the alpha scale [`tui_pane::blend_color`] reads. Starts at
    /// [`u8::MAX`] so the app opens with nothing over its grid.
    faded:       u8,
    /// When the strip was last moved on, so its speed is a speed rather
    /// than a step per frame.
    advanced_at: Instant,
    /// What the reader has said about the strip, which the roster does
    /// not get to overrule either way.
    asked:       Asked,
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
            mode:        AttractMode::default(),
            band:        TravelingBand::new(),
            held_key:    HeldKey::new(),
            faded:       u8::MAX,
            advanced_at: Instant::now(),
            asked:       Asked::Nothing,
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
        self.asked = match self.asked {
            Asked::For => Asked::Against,
            Asked::Nothing | Asked::Against => Asked::For,
        };
        if matches!(self.asked, Asked::For) {
            self.covering = true;
        }
    }

    /// Whether the strip is being shown because it was asked for, which
    /// is what the status line says: a grid taken off the screen by the
    /// attract screen otherwise looks exactly like a grid with nothing
    /// on it.
    pub(crate) const fn asked_for(&self) -> bool { matches!(self.asked, Asked::For) }

    /// Which animation is taking the reader's keys, or [`None`] while
    /// the screen is not being shown on purpose.
    ///
    /// Only an attract screen that was asked for owns the keyboard. Left
    /// to come on by itself over an idle grid it is decoration, and
    /// decoration that quietly changed what `s` did would be worse than
    /// no animation at all -- a developer who has stopped typing has not
    /// stopped meaning "settings".
    pub(crate) const fn keyed_mode(&self) -> Option<AttractMode> {
        if matches!(self.asked, Asked::For) {
            Some(self.mode)
        } else {
            None
        }
    }

    /// Steer the moving band.
    ///
    /// The step comes from [`HeldKey`], so the same action does more per
    /// press the longer its key is held. Direction is not stepped -- it
    /// is one of four answers, and there is no such thing as being more
    /// left -- and neither is the varying trailing edge, which is on or
    /// off.
    fn moving_band(&mut self, action: MovingBandAction) {
        let step = self.held_key.step(action, Instant::now());
        match action {
            MovingBandAction::Wider => self.band.widen(step * BAND_WIDTH_STEP),
            MovingBandAction::Thinner => self.band.narrow(step * BAND_WIDTH_STEP),
            MovingBandAction::Faster => self.band.speed_up(step * BAND_SPEED_STEP),
            MovingBandAction::Slower => self.band.slow_down(step * BAND_SPEED_STEP),
            MovingBandAction::TravelLeft => self.band.set_direction(BandDirection::Left),
            MovingBandAction::TravelRight => self.band.set_direction(BandDirection::Right),
            MovingBandAction::TravelUp => self.band.set_direction(BandDirection::Up),
            MovingBandAction::TravelDown => self.band.set_direction(BandDirection::Down),
            MovingBandAction::VaryTail => self.band.toggle_variable_tail(),
            MovingBandAction::TailFaster => self.band.tail_faster(step * BAND_TAIL_SPEED_STEP),
            MovingBandAction::TailSlower => self.band.tail_slower(step * BAND_TAIL_SPEED_STEP),
        }
    }

    /// What the grid should do this frame.
    ///
    /// A strip of characters drawn across a grid of borders and tables
    /// reads as neither one thing nor the other, so an attract screen
    /// that was asked for replaces the grid instead of covering it. But
    /// it takes the whole fade to arrive, and a strip arriving over
    /// nothing has nothing to arrive over -- so the panes stay, emptied
    /// of their contents, and go the rest of the way out as the strip
    /// comes the rest of the way in. Leaving runs the same thing
    /// backwards: the panes come back bare under a strip still crossing
    /// them, and only fill once it has gone.
    pub(crate) const fn grid(&self) -> Grid {
        if !self.covering {
            return Grid::Full;
        }
        if self.faded == 0 {
            return Grid::Off;
        }
        Grid::Empty(u8::MAX - self.faded)
    }

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

    /// Settle which of the emulator's windows this app is drawn in.
    ///
    /// Tried once, on the first poll the strip is showing on: a run
    /// that never shows it never pays the round trips, and a terminal
    /// that will not wear a title is not asked twice.
    pub(crate) fn identify(&mut self) {
        /// Whether the outcome has been written down, so it is noted
        /// once rather than on every frame the strip is up.
        static NOTED: OnceLock<()> = OnceLock::new();

        if !self.showing() {
            return;
        }
        // Cheap after the first: the monitor asks the window server
        // once and answers from what it settled on after that.
        let settled = self.monitor.identify(&mut io::stdout());
        if NOTED.set(()).is_ok() {
            probe::note(&format!("identify: settled={settled}"));
        }
    }

    /// Carry the strip one frame further in or out of view, and say
    /// what the grid should do underneath it.
    ///
    /// Moving the fade on before the grid is decided rather than after
    /// is what closes the frame the strip finishes leaving on. The loop
    /// repaints only while [`Self::showing`], and that goes quiet the
    /// moment the strip is gone -- so a grid still deciding on the last
    /// frame's answer would come back empty and stay that way until
    /// something unrelated asked for a repaint.
    ///
    /// Stops asking for fresh captures once the strip has faded the
    /// whole way out: an app with work on the screen has no use for
    /// what is behind it.
    pub(crate) fn advance(&mut self, area: Rect, work: Work, updates: Updates) -> Grid {
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
            return self.grid();
        }
        // Something actually running clears a dismissal. What was put
        // away was the strip standing over an idle grid, and the grid
        // has not been idle since -- so the screen re-arms and comes
        // back by itself once this finishes, as it would have before.
        if work == Work::Running {
            self.asked = match self.asked {
                Asked::Against => Asked::Nothing,
                asked => asked,
            };
        }
        // Asked for, the roster does not get a say: the strip comes in
        // over whatever is on the grid and stays until it is asked to
        // go, so it can be watched rather than only caught. Asked
        // against, the roster does not get a say either -- an idle grid
        // is exactly when the strip is being watched, and handing the
        // answer back to a roster that reads idle as "come in" is what
        // left the key unable to put it away at all.
        let work = match self.asked {
            Asked::For => Work::Idle,
            Asked::Against => Work::Running,
            Asked::Nothing => work,
        };
        self.faded = match work {
            Work::Idle => self.faded.saturating_sub(ATTRACT_FADE_STEP),
            Work::Running => self.faded.saturating_add(ATTRACT_FADE_STEP),
        };
        // Once the strip is the whole of what is on the screen, rather
        // than on the first frame it shows on. The frames either side
        // of that are the fade, which draws the grid underneath as
        // well -- so a trace started there measures the arrival and
        // runs out before reaching what the animation costs while it
        // is simply running, which is what is being looked at.
        if self.faded == 0 {
            /// Whether the trace has been started, so it is started on
            /// the first frame the strip stands alone and not again.
            static SETTLED: OnceLock<()> = OnceLock::new();

            if SETTLED.set(()).is_ok() {
                probe::trace();
            }
        }
        // The grid comes back only once the strip has gone the whole
        // way, which is also where there is nothing left to draw.
        self.covering =
            matches!(self.asked, Asked::For) || (self.covering && self.faded != u8::MAX);
        if self.faded == u8::MAX {
            return self.grid();
        }

        probe::timed(probe::Phase::Refresh, || self.monitor.refresh(area));
        self.band.advance(area, elapsed);
        self.band.fade(self.faded);
        self.grid()
    }

    /// Draw the strip where it currently stands, moving nothing.
    ///
    /// Drawn after the grid, so the panes it is arriving over or
    /// leaving over are already painted and it has a colour to settle
    /// into. [`ground`] only stands in for a cell painted on nothing at
    /// all.
    pub(crate) fn render(&self, buffer: &mut Buffer, area: Rect) {
        if self.faded == u8::MAX {
            return;
        }
        let Some(backdrop) = self.monitor.current() else {
            return;
        };
        self.band.render(area, backdrop, ground(), buffer);
    }
}

/// The colour anything leaving the attract screen fades toward where
/// the cell it sits on is painted on nothing.
///
/// A profile the app is drawn transparent in paints no ground of its
/// own, and a colour with no channels is one nothing can be mixed
/// against -- so what leaves fades toward black, which is what absent
/// looks like when the desktop is showing through.
pub(crate) fn ground() -> Color {
    match pane_background(false) {
        Color::Reset => Color::Black,
        background => background,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    /// The area the strip is advanced against. Any non-empty rectangle
    /// will do -- nothing here reads what is drawn, only how far the
    /// fade has walked.
    const AREA: Rect = Rect::new(0, 0, 80, 24);
    /// Frames to run before giving up on a fade that should have
    /// finished. The whole range at a step per frame, and then some.
    const FRAMES: u32 = 1000;

    /// Carry `attract` forward until the strip is the whole of what is
    /// on the screen, and answer how it went.
    fn settle(attract: &mut Attract, work: Work) -> u8 {
        for _ in 0..FRAMES {
            attract.advance(AREA, work, Updates::Live);
        }
        attract.faded
    }

    /// Asking for the strip over an idle grid and then asking again has
    /// to put it away. The roster reads an idle grid as a reason to
    /// show the strip, and an idle grid is exactly what is underneath
    /// it while it is being watched -- so a dismissal that handed the
    /// answer back to the roster was overruled on the same frame, and
    /// the key did nothing at all.
    #[test]
    fn asking_again_puts_the_strip_away_over_a_grid_with_nothing_on_it() {
        let mut attract = Attract::new();

        attract.toggle();
        assert_eq!(settle(&mut attract, Work::Idle), 0, "the strip comes in");

        attract.toggle();

        assert_eq!(
            settle(&mut attract, Work::Idle),
            u8::MAX,
            "and asking again sends it away, idle grid underneath or not"
        );
        assert_eq!(
            attract.grid(),
            Grid::Full,
            "which is what gives the panes back"
        );
    }

    /// A dismissal is of the strip standing over an idle grid, so work
    /// arriving and finishing re-arms it: the grid has not been idle in
    /// between, and the screen that comes on by itself is not something
    /// the reader turned off for good.
    #[test]
    fn work_arriving_re_arms_a_strip_that_was_put_away() {
        let mut attract = Attract::new();
        attract.toggle();
        settle(&mut attract, Work::Idle);
        attract.toggle();
        settle(&mut attract, Work::Idle);

        attract.advance(AREA, Work::Running, Updates::Live);

        assert_eq!(
            settle(&mut attract, Work::Idle),
            0,
            "the strip comes back by itself once the work is done"
        );
    }
}
