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
//! something starts, which is why [`Attract::render`] is called every
//! frame rather than only while idle -- the frames after work arrives
//! are the ones that carry it off the screen.
//!
//! Which animation is drawn is an [`AttractMode`], and the mode is also
//! the keymap scope the reader's keys resolve against while the screen
//! has been asked for: `+` widens the moving band rather than opening a
//! tile, and the other mode binds the same key to whatever it wants --
//! or, as it happens, to nothing. `1` and `2` turn between them. See
//! [`moving_band`] and [`moving_text`].
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

mod held_key;
mod moving_band;
mod moving_text;

use std::io;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::BackdropMonitor;
use tui_pane::BandDirection;
use tui_pane::DriftingText;
use tui_pane::TravelingBand;
use tui_pane::pane_background;

use self::held_key::HeldKey;
use self::moving_band::MovingBandAction;
pub(crate) use self::moving_band::MovingBandPane;
use self::moving_text::MovingTextAction;
pub(crate) use self::moving_text::MovingTextPane;
use crate::app::Updates;
use crate::constants::ATTRACT_FADE_STEP;
use crate::constants::ATTRACT_RETURN_QUIET;
use crate::constants::BAND_SPEED_STEP;
use crate::constants::BAND_TAIL_SPEED_STEP;
use crate::constants::BAND_WIDTH_STEP;
use crate::constants::TEXT_SPEED_STEP;
use crate::constants::TEXT_SPREAD_STEP;
use crate::probe;
use crate::probe::Phase;

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
enum Asked {
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

/// Where the screen stands with the roster, which is not the same as
/// where its fade stands.
///
/// The fade alone was not enough to say. Read frame by frame, a roster
/// that empties and fills again inside the half-second the fade takes
/// turned the screen around part way through and left it hanging over a
/// grid that was drawing cells and moving them about -- neither the
/// animation nor the display, for as long as the commands kept coming.
/// So the hand-over is a decision the screen makes once and then keeps:
/// work turns up, the screen goes, and it is the whole way gone before
/// the roster is asked anything again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Standing {
    /// On the screen, or on its way on: the grid has nothing to show.
    Showing,
    /// On its way off, and not turning back whatever the grid does
    /// before it gets there.
    Leaving,
    /// Off the screen, with something running.
    Working,
    /// Off the screen, with the grid quiet since the instant held. What
    /// the screen waits out before coming back, so a command that starts
    /// and stops inside a couple of seconds does not hand the terminal
    /// over and take it again.
    Settling(Instant),
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
    MovingBand,
    /// The whole window filled with characters instead, every line of
    /// them drifting at a speed of its own, in those same colours.
    #[default]
    MovingText,
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
    /// The window of characters drifting line by line.
    text:        DriftingText,
    /// How far into a run of presses of one of the band's steering keys
    /// the reader is, which is what lets a held key move it further per
    /// press.
    held_band:   HeldKey<MovingBandAction>,
    /// The same for the text's own keys. One run each, so turning
    /// between the animations does not hand the second whatever speed
    /// the first was climbing at.
    held_text:   HeldKey<MovingTextAction>,
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
    /// Where the screen stands with the roster, which is what keeps a
    /// hand-over from turning around part way through it.
    standing:    Standing,
}

impl Attract {
    /// An attract screen that is not yet showing.
    pub(crate) fn new() -> Self {
        Self {
            monitor:     BackdropMonitor::new(),
            mode:        AttractMode::default(),
            band:        TravelingBand::new(),
            text:        DriftingText::new(),
            held_band:   HeldKey::new(),
            held_text:   HeldKey::new(),
            faded:       u8::MAX,
            advanced_at: Instant::now(),
            asked:       Asked::Nothing,
            covering:    false,
            held:        false,
            standing:    Standing::Showing,
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
    /// there is a grid on screen for them to mean what they usually do.
    ///
    /// An attract screen that was asked for owns the keyboard from the
    /// moment it is asked for, before it has finished arriving. One
    /// that came on by itself owns it once it has arrived, which is
    /// when there is nothing else on the screen: the animations fill
    /// the window, so an arrow that reached the grid instead would move
    /// a focus ring nobody can see, around cells that are empty -- an
    /// idle grid is what brought the screen on in the first place.
    ///
    /// Never while it is arriving or leaving on its own account. A
    /// screen going out is one work has just arrived under, and the
    /// grid coming back is what the reader's keys are for.
    ///
    /// Only the keys an animation actually binds are taken either way,
    /// so `s` still opens settings and `a` still gives the grid back --
    /// a developer who has stopped typing has not stopped meaning
    /// "settings".
    pub(crate) const fn keyed_mode(&self) -> Option<AttractMode> {
        if matches!(self.asked, Asked::For) || self.faded == 0 {
            Some(self.mode)
        } else {
            None
        }
    }

    /// Steer the moving band.
    ///
    /// The step comes from the band's own [`HeldKey`], so the same
    /// action does more per press the longer its key is held. Direction
    /// is not stepped -- it is one of four answers, and there is no
    /// such thing as being more left -- and neither is which of the
    /// edges fray, which is a cycle rather than a range.
    fn moving_band(&mut self, action: MovingBandAction) {
        let step = self.held_band.step(action, Instant::now());
        match action {
            MovingBandAction::Wider => self.band.widen(step * BAND_WIDTH_STEP),
            MovingBandAction::Thinner => self.band.narrow(step * BAND_WIDTH_STEP),
            MovingBandAction::Faster => self.band.speed_up(step * BAND_SPEED_STEP),
            MovingBandAction::Slower => self.band.slow_down(step * BAND_SPEED_STEP),
            MovingBandAction::TravelLeft => self.band.set_direction(BandDirection::Left),
            MovingBandAction::TravelRight => self.band.set_direction(BandDirection::Right),
            MovingBandAction::TravelUp => self.band.set_direction(BandDirection::Up),
            MovingBandAction::TravelDown => self.band.set_direction(BandDirection::Down),
            MovingBandAction::CycleFraying => self.band.cycle_fraying(),
            MovingBandAction::TailFaster => self.band.tail_faster(step * BAND_TAIL_SPEED_STEP),
            MovingBandAction::TailSlower => self.band.tail_slower(step * BAND_TAIL_SPEED_STEP),
            MovingBandAction::ShowMovingBand => self.mode = AttractMode::MovingBand,
            MovingBandAction::ShowMovingText => self.mode = AttractMode::MovingText,
        }
    }

    /// Steer the drifting text.
    ///
    /// The step comes from the text's own [`HeldKey`], so the same
    /// action does more per press the longer its key is held. Direction
    /// is not stepped -- it is one of four answers -- and neither is
    /// whether the lines drift as one, which is on or off.
    ///
    /// Turning to the other animation leaves this one exactly as it was
    /// steered, so coming back finds it where it was left rather than
    /// at its defaults.
    fn moving_text(&mut self, action: MovingTextAction) {
        let step = self.held_text.step(action, Instant::now());
        match action {
            MovingTextAction::TravelLeft => self.text.set_direction(BandDirection::Left),
            MovingTextAction::TravelRight => self.text.set_direction(BandDirection::Right),
            MovingTextAction::TravelUp => self.text.set_direction(BandDirection::Up),
            MovingTextAction::TravelDown => self.text.set_direction(BandDirection::Down),
            MovingTextAction::Faster => self.text.speed_up(step * TEXT_SPEED_STEP),
            MovingTextAction::Slower => self.text.slow_down(step * TEXT_SPEED_STEP),
            MovingTextAction::CycleDrift => self.text.cycle_drift(),
            MovingTextAction::CycleFill => self.text.cycle_fill(),
            MovingTextAction::SpreadWider => self.text.spread_wider(step * TEXT_SPREAD_STEP),
            MovingTextAction::SpreadNarrower => self.text.spread_narrower(step * TEXT_SPREAD_STEP),
            MovingTextAction::ShowMovingBand => self.mode = AttractMode::MovingBand,
            MovingTextAction::ShowMovingText => self.mode = AttractMode::MovingText,
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
    const fn grid(&self) -> Grid {
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

    /// Whether the screen is due back, which is the one frame the event
    /// loop owes it while it is off the terminal.
    ///
    /// The quiet a screen waits out is time nothing else repaints for:
    /// the grid is empty and standing still, and [`Self::showing`] has
    /// gone quiet with it. So the loop is asked for a frame at the end
    /// of the quiet rather than through it -- one draw, on which
    /// [`Self::advance`] turns the screen back on and [`Self::showing`]
    /// carries the frames from there.
    pub(crate) fn due_back(&self, now: Instant) -> bool {
        match self.standing {
            Standing::Settling(since) => now.duration_since(since) >= ATTRACT_RETURN_QUIET,
            Standing::Showing | Standing::Leaving | Standing::Working => false,
        }
    }

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

    /// Move the screen's standing with the roster on one frame, and
    /// answer what the fade should do about it.
    ///
    /// The roster's own reading is taken once, at the top of a
    /// hand-over, and not consulted again until the screen is the whole
    /// way off: [`Standing::Leaving`] answers `Running` however empty
    /// the grid goes in the meantime. What that buys is a hand-over that
    /// finishes -- work turning up and going away inside the fade used
    /// to turn the screen around part way through, and a run of
    /// short-lived commands left it hanging over a grid that was busy
    /// opening cells and shuffling them about.
    ///
    /// Coming back is the same decision the other way, and it is not
    /// made on the first quiet frame. [`Standing::Settling`] holds when
    /// the grid went quiet, and the screen returns once that has stood
    /// for [`ATTRACT_RETURN_QUIET`] -- so a watcher firing every few
    /// seconds keeps the display rather than trading it back and forth
    /// with the animation.
    fn stand(&mut self, work: Work, now: Instant) -> Work {
        // A departure that has arrived is over, and this frame's reading
        // of the roster is the first one to count since it began.
        if matches!(self.standing, Standing::Leaving) && self.faded == u8::MAX {
            self.standing = Standing::Working;
        }
        self.standing = match self.standing {
            // Nothing reaches inside a departure still in flight.
            Standing::Leaving => return Work::Running,
            Standing::Showing => match work {
                Work::Idle => Standing::Showing,
                // Already gone, so there is no departure to make --
                // which is the app opening onto a grid with work on it.
                Work::Running if self.faded == u8::MAX => Standing::Working,
                Work::Running => Standing::Leaving,
            },
            Standing::Working => match work {
                Work::Running => Standing::Working,
                Work::Idle => Standing::Settling(now),
            },
            Standing::Settling(since) => match work {
                Work::Running => Standing::Working,
                Work::Idle if now.duration_since(since) >= ATTRACT_RETURN_QUIET => {
                    Standing::Showing
                },
                Work::Idle => Standing::Settling(since),
            },
        };
        match self.standing {
            Standing::Showing => Work::Idle,
            Standing::Leaving | Standing::Working | Standing::Settling(_) => Work::Running,
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
    ///
    /// `now` comes from the caller rather than the clock so a test can
    /// walk the quiet in [`Standing::Settling`] without standing
    /// through it.
    pub(crate) fn advance(
        &mut self,
        area: Rect,
        work: Work,
        updates: Updates,
        now: Instant,
    ) -> Grid {
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
        // Read every frame, whatever the reader has said, so the
        // standing describes the roster rather than the last frame the
        // roster had the answer.
        let standing = self.stand(work, now);
        let work = match self.asked {
            Asked::For => Work::Idle,
            Asked::Against => Work::Running,
            Asked::Nothing => standing,
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

        probe::timed(Phase::Refresh, || self.monitor.refresh(area));
        // Only the animation on screen is carried forward. The other
        // holds wherever it was left, which is what makes turning
        // between them a turn rather than a restart.
        match self.mode {
            AttractMode::MovingBand => {
                self.band.advance(area, elapsed);
                self.band.fade(self.faded);
            },
            AttractMode::MovingText => {
                self.text.advance(area, elapsed);
                self.text.fade(self.faded);
            },
        }
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
        match self.mode {
            AttractMode::MovingBand => self.band.render(area, backdrop, ground(), buffer),
            AttractMode::MovingText => self.text.render(area, backdrop, ground(), buffer),
        }
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
    use tui_pane::FRAME_POLL_MILLIS;

    use super::*;

    /// The area the strip is advanced against. Any non-empty rectangle
    /// will do -- nothing here reads what is drawn, only how far the
    /// fade has walked.
    const AREA: Rect = Rect::new(0, 0, 80, 24);
    /// Frames to run before giving up on a fade that should have
    /// finished. The whole range at a step per frame, and then some.
    const FRAMES: u32 = 1000;
    /// The gap between two frames, which is what the tests here walk the
    /// clock by. The event loop's own interval, so a run of `FRAMES`
    /// covers several seconds -- long enough to outlast the quiet a
    /// screen waits before coming back.
    const POLL: Duration = Duration::from_millis(FRAME_POLL_MILLIS);

    /// Carry `attract` forward until the strip is the whole of what is
    /// on the screen, and answer how it went.
    fn settle(attract: &mut Attract, work: Work) -> u8 {
        let mut now = Instant::now();
        for _ in 0..FRAMES {
            now += POLL;
            attract.advance(AREA, work, Updates::Live, now);
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

    /// A screen that came on by itself takes the reader's keys once it
    /// has arrived. The animations fill the window, so an arrow reaching
    /// the grid instead would move a focus ring nobody can see around
    /// cells with nothing in them -- an idle grid is what brought the
    /// screen on in the first place.
    #[test]
    fn a_screen_that_came_on_by_itself_still_steers() {
        let mut attract = Attract::new();
        assert_eq!(attract.keyed_mode(), None, "nothing is on screen yet");

        assert_eq!(settle(&mut attract, Work::Idle), 0, "it comes on by itself");

        assert_eq!(attract.keyed_mode(), Some(attract.mode));
        assert!(
            !attract.asked_for(),
            "and the status line still says it was not asked for"
        );
    }

    /// A screen still arriving or leaving on its own account takes
    /// nothing. One going out is one work has just arrived under, and
    /// the grid coming back is what the reader's keys are for.
    #[test]
    fn a_screen_part_way_in_or_out_takes_no_keys() {
        let mut attract = Attract::new();
        attract.advance(AREA, Work::Idle, Updates::Live, Instant::now());

        assert!(attract.faded > 0, "it has only started arriving");
        assert_eq!(attract.keyed_mode(), None);
    }

    /// Asking for it hands the keys over at once, before it has
    /// finished arriving: a reader who pressed `a` is already steering.
    #[test]
    fn asking_for_the_screen_takes_the_keys_before_it_arrives() {
        let mut attract = Attract::new();

        attract.toggle();

        assert_eq!(attract.faded, u8::MAX, "it has not started arriving");
        assert_eq!(attract.keyed_mode(), Some(attract.mode));
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

        attract.advance(AREA, Work::Running, Updates::Live, Instant::now());

        assert_eq!(
            settle(&mut attract, Work::Idle),
            0,
            "the strip comes back by itself once the work is done"
        );
    }

    /// Work that turns up and goes away again inside the fade does not
    /// turn the screen around part way through it. Read frame by frame,
    /// a roster that empties before the hand-over finishes used to send
    /// the screen back in over a grid that was opening cells and moving
    /// them about -- neither the animation nor the display, for as long
    /// as short-lived commands kept arriving.
    #[test]
    fn work_that_comes_and_goes_does_not_turn_a_hand_over_around() {
        let mut attract = Attract::new();
        let mut now = Instant::now();
        assert_eq!(settle(&mut attract, Work::Idle), 0, "the screen is on");

        // One frame of work, then an empty grid for the rest of the
        // fade: exactly the command that starts and stops too quickly.
        now += POLL;
        attract.advance(AREA, Work::Running, Updates::Live, now);
        for _ in 0..FRAMES {
            now += POLL;
            attract.advance(AREA, Work::Idle, Updates::Live, now);
            if attract.faded == u8::MAX {
                break;
            }
            assert_ne!(
                attract.faded, 0,
                "the screen turned back rather than finishing its exit",
            );
        }

        assert_eq!(attract.faded, u8::MAX, "and it goes the whole way off");
        assert_eq!(attract.grid(), Grid::Full, "which gives the panes back");
    }

    /// Having gone, the screen waits out a quiet grid before coming
    /// back. Returning on the first idle frame would put it in front of
    /// the next command in the run and start the whole hand-over again.
    #[test]
    fn the_screen_waits_out_a_quiet_grid_before_coming_back() {
        let mut attract = Attract::new();
        let mut now = Instant::now();
        settle(&mut attract, Work::Idle);
        now += POLL;
        attract.advance(AREA, Work::Running, Updates::Live, now);
        while attract.faded != u8::MAX {
            now += POLL;
            attract.advance(AREA, Work::Idle, Updates::Live, now);
        }

        // Short of the quiet, the grid keeps the terminal.
        now += ATTRACT_RETURN_QUIET / 2;
        attract.advance(AREA, Work::Idle, Updates::Live, now);
        assert_eq!(attract.faded, u8::MAX, "not back yet");
        assert!(!attract.due_back(now), "and the loop is owed no frame");

        now += ATTRACT_RETURN_QUIET;
        assert!(attract.due_back(now), "past the quiet, one frame is owed");
        assert_eq!(
            settle(&mut attract, Work::Idle),
            0,
            "and the screen comes back on",
        );
    }

    /// The reader outranks a hand-over the roster started. `a` pressed
    /// while the screen is on its way off brings it straight back, and
    /// waits out none of the quiet.
    #[test]
    fn asking_for_the_screen_outranks_a_hand_over_in_progress() {
        let mut attract = Attract::new();
        let mut now = Instant::now();
        settle(&mut attract, Work::Idle);
        now += POLL;
        attract.advance(AREA, Work::Running, Updates::Live, now);
        assert!(attract.faded > 0, "it has started leaving");

        attract.toggle();

        assert_eq!(
            settle(&mut attract, Work::Running),
            0,
            "asked for, it comes back over a grid with work on it",
        );
    }
}
