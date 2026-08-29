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
//! or, as it happens, to nothing. `1`, `2` and `3` turn between them.
//! See [`moving_band`], [`moving_text`] and [`pixelate`].
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
mod pixelate;

use std::io;
use std::mem;
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use AdjustedAttractParameterSets as Adjusted;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::BackdropMonitor;
use tui_pane::BackdropStatus;
use tui_pane::BandDirection;
use tui_pane::BandSettings;
use tui_pane::CaptureFailure;
use tui_pane::DriftingText;
use tui_pane::LastSuccessfulCaptureWindowId;
use tui_pane::PixelSettings;
use tui_pane::ResolvingPixels;
use tui_pane::TextSettings;
use tui_pane::TravelingBand;
use tui_pane::WindowIdentification;
use tui_pane::pane_background;

use self::held_key::HeldKey;
use self::moving_band::MovingBandAction;
pub(crate) use self::moving_band::MovingBandPane;
use self::moving_text::MovingTextAction;
pub(crate) use self::moving_text::MovingTextPane;
use self::pixelate::PixelateAction;
pub(crate) use self::pixelate::PixelatePane;
use crate::app::Updates;
use crate::constants::ATTRACT_BACKDROP_GRACE;
use crate::constants::ATTRACT_FADE_STEP;
use crate::constants::ATTRACT_RETURN_QUIET;
use crate::constants::BAND_SPEED_STEP;
use crate::constants::BAND_TAIL_SPEED_STEP;
use crate::constants::BAND_WIDTH_STEP;
use crate::constants::PIXEL_BLOCK_STEP;
use crate::constants::PIXEL_SPEED_STEP;
use crate::constants::PIXEL_WAVE_STEP;
use crate::constants::TEXT_SPEED_STEP;
use crate::constants::TEXT_SPREAD_STEP;
use crate::favorites::AttractSettings;
use crate::probe;
use crate::probe::Phase;
use crate::random;
use crate::random::NonZeroIndexBound;

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

/// What the status line should say about a missing desktop capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackdropNotice {
    /// Do not draw a notice.
    None,
    /// Tell the reader how to grant Screen Recording access.
    ScreenRecordingAccessInstruction,
    /// Report that capture is unavailable and diagnostics recorded why.
    CaptureUnavailable,
}

/// Whether the missing-backdrop grace period has elapsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackdropGracePeriod {
    /// The grace period still gives the capture worker time to reply.
    Remaining,
    /// The grace period has elapsed without a current backdrop.
    Elapsed,
}

/// Whether the attract screen is waiting for a desktop backdrop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackdropWait {
    /// A desktop is on screen, so no missing backdrop is being timed.
    NotWaiting,
    /// No desktop has been available since this instant.
    WaitingSince(Instant),
}

/// Whether the monitor has a desktop that renderers can use now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CurrentBackdrop {
    /// No usable desktop is available.
    Missing,
    /// A usable desktop is available, including one retained after a later failure.
    Available,
}

/// Values written together when attract backdrop diagnostics change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BackdropDiagnostic {
    /// The most recent window-selection report.
    window_identification: WindowIdentification,
    /// The capture worker's latest completed result.
    backdrop_status:       BackdropStatus,
    /// The window id used by the last successful capture, if one has succeeded.
    captured_window_id:    LastSuccessfulCaptureWindowId,
}

/// Select the status-line outcome from capture timing, availability, and status.
const fn classify_backdrop_notice(
    grace_period: BackdropGracePeriod,
    current_backdrop: CurrentBackdrop,
    backdrop_status: BackdropStatus,
) -> BackdropNotice {
    match (grace_period, current_backdrop, backdrop_status) {
        (_, CurrentBackdrop::Available, _)
        | (BackdropGracePeriod::Remaining, CurrentBackdrop::Missing, _) => BackdropNotice::None,
        (
            BackdropGracePeriod::Elapsed,
            CurrentBackdrop::Missing,
            BackdropStatus::Failed(CaptureFailure::ScreenRecordingAccessNotGranted),
        ) => BackdropNotice::ScreenRecordingAccessInstruction,
        (
            BackdropGracePeriod::Elapsed,
            CurrentBackdrop::Missing,
            BackdropStatus::WaitingForFirstResult
            | BackdropStatus::Ready
            | BackdropStatus::Failed(_),
        ) => BackdropNotice::CaptureUnavailable,
    }
}

/// What the reader has instructed the attract screen to do, which
/// outranks what the roster says about it.
///
/// Two answers would not be enough. The strip comes on by itself over
/// an idle grid, so "not asked for" and "asked to go" are the same
/// state to the roster and opposite ones to the reader -- and reading
/// them as one is what left `a` unable to put the strip away at
/// exactly the moment it is being watched.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AttractVisibilityInstruction {
    /// Follow whether the roster is idle or working.
    #[default]
    FollowRoster,
    /// Show the screen over a grid with work as readily as over an
    /// empty one.
    Show,
    /// Hide the screen even over an idle grid, where the roster would
    /// otherwise keep it visible.
    Hide,
}

/// Whether the attract screen is drawn over the grid or replaces it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AttractGridPresentation {
    /// Draw the attract screen over the grid.
    #[default]
    OverGrid,
    /// Leave the grid out while the attract screen has the terminal.
    ReplacesGrid,
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

/// Durable presentation state that survives a wholesale parameter replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttractPresentation {
    /// The reader's standing instruction to the attract screen.
    pub(crate) visibility_instruction: AttractVisibilityInstruction,
    /// Whether the attract screen covers or replaces the grid.
    pub(crate) grid_presentation:      AttractGridPresentation,
}

/// Complete semantic attract configuration at one instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AttractConfiguration {
    /// Animation selected for display and keyboard input.
    mode:                    AttractMode,
    /// Moving-band parameters.
    band:                    BandSettings,
    /// Moving-text parameters.
    text:                    TextSettings,
    /// Pixelate parameters.
    pixels:                  PixelSettings,
    /// Durable presentation state.
    pub(crate) presentation: AttractPresentation,
}

/// Complete configuration displaced by the most recent wholesale replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AttractConfigurationBeforeReplacement(AttractConfiguration);

/// Parameter sets adjusted while restoring a complete attract configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdjustedAttractParameterSets {
    /// Only moving-band parameters were adjusted.
    MovingBand,
    /// Only moving-text parameters were adjusted.
    MovingText,
    /// Only pixelate parameters were adjusted.
    Pixelate,
    /// Moving-band and moving-text parameters were adjusted.
    MovingBandAndMovingText,
    /// Moving-band and pixelate parameters were adjusted.
    MovingBandAndPixelate,
    /// Moving-text and pixelate parameters were adjusted.
    MovingTextAndPixelate,
    /// All three parameter sets were adjusted.
    MovingBandAndMovingTextAndPixelate,
}

impl AdjustedAttractParameterSets {
    /// Reader-facing names of the parameter sets adjusted during restore.
    pub(crate) const fn names(self) -> &'static str {
        match self {
            Self::MovingBand => "moving band",
            Self::MovingText => "moving text",
            Self::Pixelate => "pixelate",
            Self::MovingBandAndMovingText => "moving band and moving text",
            Self::MovingBandAndPixelate => "moving band and pixelate",
            Self::MovingTextAndPixelate => "moving text and pixelate",
            Self::MovingBandAndMovingTextAndPixelate => "moving band, moving text, and pixelate",
        }
    }
}

/// Availability of the one-step wholesale-replacement undo point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplacementUndoState {
    /// No wholesale replacement is available to restore.
    Unavailable,
    /// The complete configuration displaced by the latest replacement.
    Available(AttractConfigurationBeforeReplacement),
}

/// Result of trying to restore the configuration before the latest replacement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AttractConfigurationRestoreOutcome {
    /// No replacement was available to undo.
    NothingToUndo,
    /// The complete configuration was restored unchanged.
    RestoredExactly {
        /// Mode selected by the restored configuration.
        mode: AttractMode,
    },
    /// Current terminal bounds adjusted one or more restored parameter sets.
    RestoredWithAdjustments {
        /// Mode selected by the restored configuration.
        mode:                    AttractMode,
        /// Parameter sets adjusted to current animation bounds.
        adjusted_parameter_sets: AdjustedAttractParameterSets,
    },
}

impl AttractConfigurationRestoreOutcome {
    fn from_configurations(
        requested: AttractConfiguration,
        effective: AttractConfiguration,
    ) -> Self {
        let adjusted_parameter_sets = match (
            requested.band == effective.band,
            requested.text == effective.text,
            requested.pixels == effective.pixels,
        ) {
            (true, true, true) => {
                return Self::RestoredExactly {
                    mode: effective.mode,
                };
            },
            (false, true, true) => Adjusted::MovingBand,
            (true, false, true) => Adjusted::MovingText,
            (true, true, false) => Adjusted::Pixelate,
            (false, false, true) => Adjusted::MovingBandAndMovingText,
            (false, true, false) => Adjusted::MovingBandAndPixelate,
            (true, false, false) => Adjusted::MovingTextAndPixelate,
            (false, false, false) => Adjusted::MovingBandAndMovingTextAndPixelate,
        };
        Self::RestoredWithAdjustments {
            mode: effective.mode,
            adjusted_parameter_sets,
        }
    }
}

/// Terminal area most recently passed through the app's frame layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameArea {
    /// No frame has reached [`Attract::advance`] yet.
    NeverLaidOut,
    /// The terminal area used by the most recent frame.
    LaidOut(Rect),
}

/// Terminal resize received since the most recent frame layout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PendingTerminalResize {
    /// No resize input is waiting for the next frame.
    #[default]
    NotReported,
    /// New terminal area reported by resize input.
    Reported(Rect),
}

/// Area an animation's internal buffers were most recently sized to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AnimationArea {
    /// The animation has not received a terminal area yet.
    #[default]
    NeverSized,
    /// The animation has been sized to this terminal area.
    Sized(Rect),
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
    /// The desktop drawn as itself, with a band of coarseness sweeping
    /// across it that takes the picture to blocks and gives it back.
    Pixelate,
}

impl AttractMode {
    const ALL: [Self; 3] = [Self::MovingBand, Self::MovingText, Self::Pixelate];
    const INDEX_BOUND: NonZeroIndexBound = match NonZeroIndexBound::try_from_len(Self::ALL.len()) {
        Ok(bound) => bound,
        Err(_) => panic!("AttractMode::ALL must contain at least one mode"),
    };

    fn draw(seed: u64) -> Self {
        let index = random::bounded_index(seed, Self::INDEX_BOUND);
        Self::ALL[index]
    }
}

/// Result of applying attract settings through the selected animation's clamp setters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsApplicationOutcome {
    /// Every requested value was accepted unchanged.
    AppliedExactly,
    /// One or more requested values were corrected for the current animation bounds.
    AppliedWithAdjustments {
        /// Values requested by the caller.
        requested: AttractSettings,
        /// Values the animation accepted after applying its bounds.
        effective: AttractSettings,
    },
}

/// Last terminal area applied to each attract animation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct AnimationSizing {
    /// Area applied to the moving band.
    band:   AnimationArea,
    /// Area applied to the drifting text.
    text:   AnimationArea,
    /// Area applied to the pixelating desktop.
    pixels: AnimationArea,
}

impl AnimationSizing {
    const fn area(self, attract_mode: AttractMode) -> AnimationArea {
        match attract_mode {
            AttractMode::MovingBand => self.band,
            AttractMode::MovingText => self.text,
            AttractMode::Pixelate => self.pixels,
        }
    }

    const fn record(&mut self, attract_mode: AttractMode, area: Rect) {
        let sized_area = AnimationArea::Sized(area);
        match attract_mode {
            AttractMode::MovingBand => self.band = sized_area,
            AttractMode::MovingText => self.text = sized_area,
            AttractMode::Pixelate => self.pixels = sized_area,
        }
    }
}

/// The attract screen's state between frames.
pub(crate) struct Attract {
    /// Keeps the captured desktop up to date on a worker thread.
    monitor:                BackdropMonitor,
    /// Which animation is being drawn, and which keymap scope the
    /// reader's keys resolve against while it is on screen.
    mode:                   AttractMode,
    /// Terminal area used to size the current animation before its
    /// parameters are read or drawn.
    laid_out_area:          FrameArea,
    /// Resize input received after the most recent frame layout.
    pending_resize:         PendingTerminalResize,
    /// Last terminal area applied to each animation's internal buffers.
    animation_sizing:       AnimationSizing,
    /// Complete configuration displaced by the latest wholesale replacement.
    replacement_undo:       ReplacementUndoState,
    /// The strip of characters crossing the grid.
    band:                   TravelingBand,
    /// The window of characters drifting line by line.
    text:                   DriftingText,
    /// The desktop drawn as itself, coarsening under a travelling wave.
    pixels:                 ResolvingPixels,
    /// How far into a run of presses of one of the band's steering keys
    /// the reader is, which is what lets a held key move it further per
    /// press.
    held_band:              HeldKey<MovingBandAction>,
    /// The same for the text's own keys. One run each, so turning
    /// between the animations does not hand the second whatever speed
    /// the first was climbing at.
    held_text:              HeldKey<MovingTextAction>,
    /// And the same again for the pixelate screen's.
    held_pixels:            HeldKey<PixelateAction>,
    /// How far the strip is carried toward the ground it is drawn on,
    /// on the alpha scale [`tui_pane::blend_color`] reads. Starts at
    /// [`u8::MAX`] so the app opens with nothing over its grid.
    faded:                  u8,
    /// When the strip was last moved on, so its speed is a speed rather
    /// than a step per frame.
    advanced_at:            Instant,
    /// What the reader has told the screen to do, which the roster does
    /// not get to overrule either way.
    visibility_instruction: AttractVisibilityInstruction,
    /// Whether the grid is drawn under the attract screen or left out
    /// of the frame altogether.
    grid_presentation:      AttractGridPresentation,
    /// Whether the display was being held still when the strip was last
    /// drawn, which is what says the gap since then is not travel the
    /// strip owes.
    held:                   bool,
    /// Where the screen stands with the roster, which is what keeps a
    /// hand-over from turning around part way through it.
    standing:               Standing,
    /// Whether the screen is waiting for a backdrop, including when that wait began.
    backdrop_wait:          BackdropWait,
    /// The attract backdrop values most recently written to the probe, so
    /// unchanged capture and window-selection results do not repeat every frame.
    noted_backdrop:         BackdropDiagnostic,
    /// The last reading written to the frame log, so the log carries a
    /// line where the screen changed its mind rather than one per
    /// frame. See [`Attract::note_standing`].
    noted:                  Option<Reading>,
}

/// What the screen decided on a frame, in the terms that decide whether
/// it is on the terminal at all.
///
/// The three together are the whole of that answer: what the roster
/// said, what the reader said over the top of it, and where the fade
/// ended up between them. Kept as a value so a frame that decided the
/// same thing as the one before writes nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Reading {
    /// What the fade was moved toward.
    work:        Work,
    /// Where the screen stands with the roster.
    standing:    Standing,
    /// What the reader has instructed.
    instruction: AttractVisibilityInstruction,
    /// Whether the screen is anywhere on the terminal.
    showing:     bool,
}

impl Attract {
    /// An attract screen that is not yet showing.
    pub(crate) fn new() -> Self {
        Self {
            monitor:                BackdropMonitor::new(),
            mode:                   AttractMode::default(),
            laid_out_area:          FrameArea::NeverLaidOut,
            pending_resize:         PendingTerminalResize::NotReported,
            animation_sizing:       AnimationSizing::default(),
            replacement_undo:       ReplacementUndoState::Unavailable,
            band:                   TravelingBand::new(),
            text:                   DriftingText::new(),
            pixels:                 ResolvingPixels::new(),
            held_band:              HeldKey::new(),
            held_text:              HeldKey::new(),
            held_pixels:            HeldKey::new(),
            faded:                  u8::MAX,
            advanced_at:            Instant::now(),
            visibility_instruction: AttractVisibilityInstruction::FollowRoster,
            grid_presentation:      AttractGridPresentation::OverGrid,
            held:                   false,
            standing:               Standing::Showing,
            backdrop_wait:          BackdropWait::NotWaiting,
            noted_backdrop:         BackdropDiagnostic {
                window_identification: WindowIdentification::NotAttempted,
                backdrop_status:       BackdropStatus::WaitingForFirstResult,
                captured_window_id:    LastSuccessfulCaptureWindowId::WaitingForFirstSuccess,
            },
            noted:                  None,
        }
    }

    /// Ask for the strip, or give the grid back.
    ///
    /// Asking covers the grid from this moment rather than from the
    /// next frame: the panes are drawn before the strip is, so waiting
    /// would show one frame of the grid with the strip over it -- the
    /// very look this is here to avoid.
    pub(crate) const fn toggle(&mut self) {
        self.visibility_instruction = match self.visibility_instruction {
            AttractVisibilityInstruction::Show => AttractVisibilityInstruction::Hide,
            AttractVisibilityInstruction::FollowRoster | AttractVisibilityInstruction::Hide => {
                AttractVisibilityInstruction::Show
            },
        };
        if matches!(
            self.visibility_instruction,
            AttractVisibilityInstruction::Show
        ) {
            self.grid_presentation = AttractGridPresentation::ReplacesGrid;
        }
    }

    /// Ask for the attract screen regardless of its current fade direction.
    pub(crate) const fn request_show(&mut self) {
        self.visibility_instruction = AttractVisibilityInstruction::Show;
        self.grid_presentation = AttractGridPresentation::ReplacesGrid;
    }

    /// Draw and apply a fresh mode and parameters, then show the result.
    pub(crate) fn randomize(&mut self) { self.randomize_from_seed(random::clock_seed()); }

    fn randomize_from_seed(&mut self, seed: u64) {
        self.size_all_animations();
        let drawn_mode = AttractMode::draw(seed);
        let drawn_settings = self.draw_random_settings(drawn_mode, seed);
        // Applied unconditionally: the check below compiles out of release
        // builds, so the call cannot live inside it. An adjusted outcome is a
        // drawing bug rather than something the viewer should be made to see,
        // and a clamped attract screen still draws, so release keeps running.
        let outcome = self.apply_settings(drawn_settings);
        debug_assert_eq!(
            outcome,
            SettingsApplicationOutcome::AppliedExactly,
            "settings drawn after size_all_animations must satisfy the animation bounds",
        );
        self.request_show();
    }

    fn draw_random_settings(&self, mode: AttractMode, seed: u64) -> AttractSettings {
        match mode {
            AttractMode::MovingBand => AttractSettings::MovingBand(self.band.random_settings(seed)),
            AttractMode::MovingText => AttractSettings::MovingText(self.text.random_settings(seed)),
            AttractMode::Pixelate => AttractSettings::Pixelate(self.pixels.random_settings(seed)),
        }
    }

    /// Whether the strip is being shown because it was asked for, which
    /// is what the status line says: a grid taken off the screen by the
    /// attract screen otherwise looks exactly like a grid with nothing
    /// on it.
    pub(crate) const fn asked_for(&self) -> bool {
        matches!(
            self.visibility_instruction,
            AttractVisibilityInstruction::Show
        )
    }

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
        if matches!(
            self.visibility_instruction,
            AttractVisibilityInstruction::Show
        ) || self.faded == 0
        {
            Some(self.mode)
        } else {
            None
        }
    }

    /// Settings the current attract mode is running with now.
    ///
    /// Applies the latest [`FrameArea`] or [`PendingTerminalResize`]
    /// first so the returned values already match the next frame,
    /// including a mode switch with no frame in between.
    pub(crate) fn current_settings(&mut self) -> AttractSettings {
        self.size_current_animation();
        match self.mode {
            AttractMode::MovingBand => AttractSettings::MovingBand(self.band.settings()),
            AttractMode::MovingText => AttractSettings::MovingText(self.text.settings()),
            AttractMode::Pixelate => AttractSettings::Pixelate(self.pixels.settings()),
        }
    }

    /// Apply mode-specific settings after sizing their animation to the latest terminal area.
    pub(crate) fn apply_settings(
        &mut self,
        requested: AttractSettings,
    ) -> SettingsApplicationOutcome {
        self.size_all_animations();
        self.replacement_undo = ReplacementUndoState::Available(
            AttractConfigurationBeforeReplacement(self.configuration()),
        );
        self.mode = requested.mode();
        let effective = match requested {
            AttractSettings::MovingBand(settings) => {
                self.band.apply(settings);
                AttractSettings::MovingBand(self.band.settings())
            },
            AttractSettings::MovingText(settings) => {
                self.text.apply(settings);
                AttractSettings::MovingText(self.text.settings())
            },
            AttractSettings::Pixelate(settings) => {
                self.pixels.apply(settings);
                AttractSettings::Pixelate(self.pixels.settings())
            },
        };
        if effective == requested {
            SettingsApplicationOutcome::AppliedExactly
        } else {
            SettingsApplicationOutcome::AppliedWithAdjustments {
                requested,
                effective,
            }
        }
    }

    /// Restore the complete configuration displaced by the latest replacement.
    pub(crate) fn restore_configuration_before_last_replacement(
        &mut self,
    ) -> AttractConfigurationRestoreOutcome {
        let checkpoint = mem::replace(
            &mut self.replacement_undo,
            ReplacementUndoState::Unavailable,
        );
        let ReplacementUndoState::Available(AttractConfigurationBeforeReplacement(requested)) =
            checkpoint
        else {
            return AttractConfigurationRestoreOutcome::NothingToUndo;
        };

        self.size_all_animations();
        self.band.apply(requested.band);
        self.text.apply(requested.text);
        self.pixels.apply(requested.pixels);
        self.mode = requested.mode;
        self.visibility_instruction = requested.presentation.visibility_instruction;
        self.grid_presentation = requested.presentation.grid_presentation;

        AttractConfigurationRestoreOutcome::from_configurations(requested, self.configuration())
    }

    pub(crate) const fn configuration(&self) -> AttractConfiguration {
        AttractConfiguration {
            mode:         self.mode,
            band:         self.band.settings(),
            text:         self.text.settings(),
            pixels:       self.pixels.settings(),
            presentation: AttractPresentation {
                visibility_instruction: self.visibility_instruction,
                grid_presentation:      self.grid_presentation,
            },
        }
    }

    /// Size the selected animation to the latest frame or resize area
    /// without moving it.
    fn size_current_animation(&mut self) {
        let FrameArea::LaidOut(area) = self.latest_sizing_area() else {
            return;
        };
        let attract_mode = self.mode;
        self.size_animation(attract_mode, area);
    }

    /// Size every animation to the latest frame or resize area without moving it.
    fn size_all_animations(&mut self) {
        let FrameArea::LaidOut(area) = self.latest_sizing_area() else {
            return;
        };
        for attract_mode in AttractMode::ALL {
            self.size_animation(attract_mode, area);
        }
    }

    const fn latest_sizing_area(&self) -> FrameArea {
        match (self.pending_resize, self.laid_out_area) {
            (PendingTerminalResize::Reported(area), _) | (_, FrameArea::LaidOut(area)) => {
                FrameArea::LaidOut(area)
            },
            (PendingTerminalResize::NotReported, FrameArea::NeverLaidOut) => {
                FrameArea::NeverLaidOut
            },
        }
    }

    fn size_animation(&mut self, attract_mode: AttractMode, area: Rect) {
        if self.animation_sizing.area(attract_mode) == AnimationArea::Sized(area) {
            return;
        }
        match attract_mode {
            AttractMode::MovingBand => self.band.advance(area, Duration::ZERO),
            AttractMode::MovingText => self.text.advance(area, Duration::ZERO),
            AttractMode::Pixelate => self.pixels.advance(area, Duration::ZERO),
        }
        self.animation_sizing.record(attract_mode, area);
    }

    /// Record an input-reported terminal area before queued keys are
    /// dispatched.
    pub(crate) const fn record_terminal_resize(&mut self, area: Rect) {
        self.pending_resize = PendingTerminalResize::Reported(area);
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
            MovingBandAction::ShowPixelate => self.mode = AttractMode::Pixelate,
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
            MovingTextAction::ShowPixelate => self.mode = AttractMode::Pixelate,
        }
    }

    /// Steer the pixelate screen.
    ///
    /// The step comes from the screen's own [`HeldKey`], so the same
    /// action does more per press the longer its key is held. Direction
    /// is not stepped -- it is one of four answers -- and neither is
    /// how a block gives its cells back or what a cell is drawn with,
    /// each of which is a cycle rather than a range.
    fn pixelate(&mut self, action: PixelateAction) {
        let step = self.held_pixels.step(action, Instant::now());
        match action {
            PixelateAction::SweepLeft => self.pixels.set_direction(BandDirection::Left),
            PixelateAction::SweepRight => self.pixels.set_direction(BandDirection::Right),
            PixelateAction::SweepUp => self.pixels.set_direction(BandDirection::Up),
            PixelateAction::SweepDown => self.pixels.set_direction(BandDirection::Down),
            PixelateAction::Faster => self.pixels.speed_up(step * PIXEL_SPEED_STEP),
            PixelateAction::Slower => self.pixels.slow_down(step * PIXEL_SPEED_STEP),
            PixelateAction::Coarser => self.pixels.coarsen(step * PIXEL_BLOCK_STEP),
            PixelateAction::Sharper => self.pixels.sharpen(step * PIXEL_BLOCK_STEP),
            PixelateAction::WaveWider => self.pixels.wider(step * PIXEL_WAVE_STEP),
            PixelateAction::WaveNarrower => self.pixels.narrower(step * PIXEL_WAVE_STEP),
            PixelateAction::CycleResolve => self.pixels.cycle_resolve(),
            PixelateAction::CycleFill => self.pixels.cycle_fill(),
            PixelateAction::ShowMovingBand => self.mode = AttractMode::MovingBand,
            PixelateAction::ShowMovingText => self.mode = AttractMode::MovingText,
            PixelateAction::ShowPixelate => self.mode = AttractMode::Pixelate,
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
        if matches!(self.grid_presentation, AttractGridPresentation::OverGrid) {
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
        if !self.showing() {
            return;
        }
        // Cheap once it has settled: the monitor answers from what it
        // found and asks the window server nothing more.
        let backdrop_diagnostic = BackdropDiagnostic {
            window_identification: self.monitor.identify(&mut io::stdout()),
            backdrop_status:       self.monitor.status(),
            captured_window_id:    self.monitor.captured_window_id(),
        };
        // Noted when any reported value changes, so paced identification
        // retries and an unchanged capture failure do not write one line per frame.
        if self.noted_backdrop != backdrop_diagnostic {
            self.noted_backdrop = backdrop_diagnostic;
            probe::note(&format!(
                "backdrop: report={:?} capture_status={:?} captured_window_id={:?}",
                backdrop_diagnostic.window_identification,
                backdrop_diagnostic.backdrop_status,
                backdrop_diagnostic.captured_window_id,
            ));
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
        self.laid_out_area = FrameArea::LaidOut(area);
        self.pending_resize = PendingTerminalResize::NotReported;
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
            self.visibility_instruction = match self.visibility_instruction {
                AttractVisibilityInstruction::Hide => AttractVisibilityInstruction::FollowRoster,
                instruction => instruction,
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
        let work = match self.visibility_instruction {
            AttractVisibilityInstruction::Show => Work::Idle,
            AttractVisibilityInstruction::Hide => Work::Running,
            AttractVisibilityInstruction::FollowRoster => standing,
        };
        self.faded = match work {
            Work::Idle => self.faded.saturating_sub(ATTRACT_FADE_STEP),
            Work::Running => self.faded.saturating_add(ATTRACT_FADE_STEP),
        };
        self.note_standing(work);
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
        self.grid_presentation = if matches!(
            self.visibility_instruction,
            AttractVisibilityInstruction::Show
        ) || (matches!(
            self.grid_presentation,
            AttractGridPresentation::ReplacesGrid
        ) && self.faded != u8::MAX)
        {
            AttractGridPresentation::ReplacesGrid
        } else {
            AttractGridPresentation::OverGrid
        };
        if self.faded == u8::MAX {
            self.size_current_animation();
            return self.grid();
        }

        probe::timed(Phase::Refresh, || self.monitor.refresh(area));
        // A capture takes a few frames to arrive and is re-taken on a
        // timer, so having none for a moment is ordinary. Having none
        // for longer than that is the animation drawing nothing at all,
        // which from outside is indistinguishable from an attract
        // screen that never came on -- see [`Self::backdrop_notice`].
        let backdrop_wait = match (self.monitor.current(), self.backdrop_wait) {
            (Some(_), _) => BackdropWait::NotWaiting,
            (None, BackdropWait::WaitingSince(since)) => BackdropWait::WaitingSince(since),
            (None, BackdropWait::NotWaiting) => BackdropWait::WaitingSince(now),
        };
        if std::mem::discriminant(&backdrop_wait) != std::mem::discriminant(&self.backdrop_wait) {
            probe::note(&format!(
                "attract: backdrop={}",
                matches!(backdrop_wait, BackdropWait::NotWaiting),
            ));
        }
        self.backdrop_wait = backdrop_wait;
        // Only the animation on screen is carried forward. The other
        // holds wherever it was left, which is what makes turning
        // between them a turn rather than a restart.
        let attract_mode = self.mode;
        match attract_mode {
            AttractMode::MovingBand => {
                self.band.advance(area, elapsed);
                self.band.fade(self.faded);
            },
            AttractMode::MovingText => {
                self.text.advance(area, elapsed);
                self.text.fade(self.faded);
            },
            AttractMode::Pixelate => {
                self.pixels.advance(area, elapsed);
                self.pixels.fade(self.faded);
            },
        }
        self.animation_sizing.record(attract_mode, area);
        self.grid()
    }

    /// Write this frame's reading to the frame log, where it differs
    /// from the last one written.
    ///
    /// An attract screen that never comes on looks from outside exactly
    /// like one that came on with nothing to draw, and the two are
    /// fixed in different places -- the first upstream in what the
    /// roster reports, the second in the desktop capture. Nothing on
    /// the terminal separates them, so the separation is recorded here
    /// instead. Costs nothing with the probe off, and with it on writes
    /// a line where the answer changed rather than one per frame.
    fn note_standing(&mut self, work: Work) {
        let reading = Reading {
            work,
            standing: self.standing,
            instruction: self.visibility_instruction,
            showing: self.showing(),
        };
        if self.noted == Some(reading) {
            return;
        }
        self.noted = Some(reading);
        probe::note(&format!(
            "attract: work={:?} standing={:?} instruction={:?} showing={} faded={}",
            reading.work, reading.standing, reading.instruction, reading.showing, self.faded,
        ));
    }

    /// What the status line should report about a missing desktop capture.
    ///
    /// Every animation here draws in the colours of the desktop behind
    /// the terminal, so with no capture there is nothing to draw and
    /// [`Self::render`] returns having drawn none of it. Left at that,
    /// an attract screen that is running perfectly well and simply has
    /// no picture looks exactly like one that never started. The wait is
    /// reported once it is long enough to mean something, and the latest
    /// capture status selects the notice. A retained current backdrop
    /// suppresses the notice even when the latest attempt failed.
    pub(crate) fn backdrop_notice(&self, now: Instant) -> BackdropNotice {
        let grace_period = match self.backdrop_wait {
            BackdropWait::WaitingSince(since)
                if self.showing() && now.duration_since(since) >= ATTRACT_BACKDROP_GRACE =>
            {
                BackdropGracePeriod::Elapsed
            },
            BackdropWait::NotWaiting | BackdropWait::WaitingSince(_) => {
                BackdropGracePeriod::Remaining
            },
        };
        let current_backdrop = match self.monitor.current() {
            Some(_) => CurrentBackdrop::Available,
            None => CurrentBackdrop::Missing,
        };
        classify_backdrop_notice(grace_period, current_backdrop, self.monitor.status())
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
            AttractMode::Pixelate => self.pixels.render(area, backdrop, ground(), buffer),
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
#[expect(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::collections::HashSet;

    use ratatui::layout::Rect;
    use tui_pane::BandDirection;
    use tui_pane::BandFraying;
    use tui_pane::FRAME_POLL_MILLIS;
    use tui_pane::PixelFill;
    use tui_pane::PixelResolve;
    use tui_pane::TextDrift;
    use tui_pane::TextFill;

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
    /// Capture failure stages exercised by the notice classifier.
    const CAPTURE_FAILURES: [CaptureFailure; 8] = [
        CaptureFailure::UnsupportedPlatform,
        CaptureFailure::ScreenRecordingAccessNotGranted,
        CaptureFailure::ShareableContentQueryFailed,
        CaptureFailure::TerminalWindowNotFound,
        CaptureFailure::DisplayNotFound,
        CaptureFailure::ScreenshotCaptureFailed,
        CaptureFailure::PixelExtractionFailed,
        CaptureFailure::ImageReductionFailed,
    ];

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

    #[test]
    fn switched_mode_settings_are_sized_before_the_next_frame() {
        const NARROW_AREA: Rect = Rect::new(0, 0, 20, 10);

        let mut attract = Attract::new();
        let unsized_settings = attract.band.settings();
        let now = Instant::now();

        attract.advance(NARROW_AREA, Work::Running, Updates::Live, now);
        attract.moving_text(MovingTextAction::ShowMovingBand);
        let saved = attract.current_settings();

        assert!(
            matches!(
                saved, AttractSettings::MovingBand(settings)
                    if settings.width < unsized_settings.width
            ),
            "reading the switched mode removes the band's sentinel width",
        );

        attract.toggle();
        attract.advance(NARROW_AREA, Work::Running, Updates::Live, now + POLL);

        assert_eq!(
            attract.current_settings(),
            saved,
            "the first frame keeps the parameters read before it",
        );
    }

    #[test]
    fn draw_reaches_every_mode_declared_in_all() {
        let drawn_modes = (0..=4095).map(AttractMode::draw).collect::<HashSet<_>>();

        assert_eq!(drawn_modes, AttractMode::ALL.into_iter().collect());
    }

    #[test]
    fn random_settings_corpus_reaches_every_variant_and_applies_every_draw() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        let mut modes = HashSet::new();
        let mut band_directions = HashSet::new();
        let mut band_fraying = HashSet::new();
        let mut text_directions = HashSet::new();
        let mut text_drift = HashSet::new();
        let mut text_fill = HashSet::new();
        let mut pixel_directions = HashSet::new();
        let mut pixel_resolve = HashSet::new();
        let mut pixel_fill = HashSet::new();

        for seed in 0..=4095 {
            attract.randomize_from_seed(seed);
            let target = attract.draw_random_settings(AttractMode::draw(seed), seed);

            assert_eq!(attract.current_settings(), target);
            modes.insert(target.mode());
            match target {
                AttractSettings::MovingBand(settings) => {
                    assert!((1..=400).contains(&settings.speed));
                    assert!((8..=2000).contains(&settings.tail_speed));
                    let width_ceiling = match settings.direction {
                        BandDirection::Left | BandDirection::Right => u32::from(AREA.width),
                        BandDirection::Up | BandDirection::Down => u32::from(AREA.height),
                    };
                    assert!((1..=width_ceiling).contains(&settings.width));
                    band_directions.insert(settings.direction);
                    band_fraying.insert(settings.fraying);
                },
                AttractSettings::MovingText(settings) => {
                    assert!((1..=200).contains(&settings.speed));
                    assert!((0..=100).contains(&settings.spread));
                    text_directions.insert(settings.direction);
                    text_drift.insert(settings.drift);
                    text_fill.insert(settings.fill);
                },
                AttractSettings::Pixelate(settings) => {
                    assert!((1..=200).contains(&settings.speed));
                    assert!((5..=200).contains(&settings.wave_percent));
                    assert!((2..=48).contains(&settings.block_columns));
                    pixel_directions.insert(settings.direction);
                    pixel_resolve.insert(settings.resolve);
                    pixel_fill.insert(settings.fill);
                },
            }
        }

        let every_direction = [
            BandDirection::Left,
            BandDirection::Right,
            BandDirection::Up,
            BandDirection::Down,
        ]
        .into_iter()
        .collect::<HashSet<_>>();
        assert_eq!(modes, AttractMode::ALL.into_iter().collect());
        assert_eq!(band_directions, every_direction);
        assert_eq!(text_directions, every_direction);
        assert_eq!(pixel_directions, every_direction);
        assert_eq!(
            band_fraying,
            [
                BandFraying::Trailing,
                BandFraying::Both,
                BandFraying::Leading,
                BandFraying::Neither,
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(
            text_drift,
            [TextDrift::Together, TextDrift::Apart]
                .into_iter()
                .collect()
        );
        assert_eq!(
            text_fill,
            [TextFill::Bars, TextFill::Glyphs].into_iter().collect()
        );
        assert_eq!(
            pixel_resolve,
            [
                PixelResolve::Blend,
                PixelResolve::Step,
                PixelResolve::Scatter,
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(
            pixel_fill,
            [PixelFill::Solid, PixelFill::Shades].into_iter().collect()
        );
    }

    #[test]
    fn never_shown_band_is_sized_before_its_random_width_is_drawn() {
        const NARROW_AREA: Rect = Rect::new(0, 0, 9, 4);

        let Some(seed) =
            (0..=4095).find(|seed| AttractMode::draw(*seed) == AttractMode::MovingBand)
        else {
            panic!("the fixed seed corpus should reach the moving band");
        };
        let mut attract = Attract::new();
        let unsized_width = attract.band.settings().width;
        attract.record_terminal_resize(NARROW_AREA);

        attract.randomize_from_seed(seed);

        let AttractSettings::MovingBand(settings) = attract.current_settings() else {
            panic!("the selected seed should draw moving-band settings");
        };
        let width_ceiling = match settings.direction {
            BandDirection::Left | BandDirection::Right => u32::from(NARROW_AREA.width),
            BandDirection::Up | BandDirection::Down => u32::from(NARROW_AREA.height),
        };
        assert!((1..=width_ceiling).contains(&settings.width));
        assert!(settings.width < unsized_width);
    }

    #[test]
    fn settings_application_reports_exact_and_adjusted_values() {
        let mut attract = Attract::new();
        attract.mode = AttractMode::MovingBand;
        attract.record_terminal_resize(AREA);
        let exact = attract.current_settings();

        assert_eq!(
            attract.apply_settings(exact),
            SettingsApplicationOutcome::AppliedExactly
        );

        let AttractSettings::MovingBand(mut oversized) = exact else {
            panic!("moving-band mode should expose moving-band settings");
        };
        oversized.width = u32::MAX;
        let requested = AttractSettings::MovingBand(oversized);
        let outcome = attract.apply_settings(requested);
        let SettingsApplicationOutcome::AppliedWithAdjustments {
            requested: reported,
            effective,
        } = outcome
        else {
            panic!("oversized width should be adjusted");
        };

        assert_eq!(reported, requested);
        assert_ne!(effective, requested);
        assert_eq!(attract.current_settings(), effective);
    }

    #[test]
    fn settings_application_captures_the_complete_configuration_it_replaces() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.request_show();
        let before_first_sizing = attract.configuration();
        let replacement = AttractSettings::MovingText(attract.text.settings());

        attract.apply_settings(replacement);

        let before = attract.configuration();
        assert_ne!(
            before.band, before_first_sizing.band,
            "the first all-mode sizing adjusts the never-shown moving band before capture"
        );
        assert_eq!(
            attract.replacement_undo,
            ReplacementUndoState::Available(AttractConfigurationBeforeReplacement(before))
        );
        assert_eq!(attract.mode, AttractMode::MovingText);
    }

    #[test]
    fn randomize_captures_the_mode_being_replaced_before_drawing_a_new_one() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.size_all_animations();
        let before = attract.configuration();
        let Some(seed) = (0..=4095).find(|seed| AttractMode::draw(*seed) != before.mode) else {
            panic!("the fixed seed corpus should draw a different mode");
        };

        attract.randomize_from_seed(seed);

        let ReplacementUndoState::Available(AttractConfigurationBeforeReplacement(captured)) =
            attract.replacement_undo
        else {
            panic!("randomization should leave an undo point");
        };
        assert_eq!(captured, before);
        assert_ne!(attract.mode, before.mode);

        assert_eq!(
            attract.restore_configuration_before_last_replacement(),
            AttractConfigurationRestoreOutcome::RestoredExactly { mode: before.mode }
        );
        assert_eq!(attract.configuration(), before);
    }

    #[test]
    fn restore_consumes_the_only_undo_point() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.apply_settings(AttractSettings::MovingBand(attract.band.settings()));

        assert!(matches!(
            attract.restore_configuration_before_last_replacement(),
            AttractConfigurationRestoreOutcome::RestoredExactly { .. }
        ));
        assert_eq!(attract.replacement_undo, ReplacementUndoState::Unavailable);
        assert_eq!(
            attract.restore_configuration_before_last_replacement(),
            AttractConfigurationRestoreOutcome::NothingToUndo
        );
    }

    #[test]
    fn restore_uses_the_checkpoint_from_the_latest_of_two_replacements() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.size_all_animations();
        let original = attract.configuration();

        attract.apply_settings(AttractSettings::MovingBand(attract.band.settings()));
        let between_replacements = attract.configuration();
        attract.apply_settings(AttractSettings::Pixelate(attract.pixels.settings()));
        let after_second_replacement = attract.configuration();

        assert_ne!(original, between_replacements);
        assert_ne!(between_replacements, after_second_replacement);
        assert_ne!(original, after_second_replacement);
        assert_eq!(between_replacements.mode, AttractMode::MovingBand);
        assert_eq!(after_second_replacement.mode, AttractMode::Pixelate);

        assert_eq!(
            attract.restore_configuration_before_last_replacement(),
            AttractConfigurationRestoreOutcome::RestoredExactly {
                mode: between_replacements.mode,
            }
        );
        assert_eq!(attract.configuration(), between_replacements);
        assert_ne!(attract.configuration(), original);
    }

    #[test]
    fn adjusted_parameter_set_names_are_reader_facing_text() {
        let cases = [
            (AdjustedAttractParameterSets::MovingBand, "moving band"),
            (AdjustedAttractParameterSets::MovingText, "moving text"),
            (AdjustedAttractParameterSets::Pixelate, "pixelate"),
            (
                AdjustedAttractParameterSets::MovingBandAndMovingText,
                "moving band and moving text",
            ),
            (
                AdjustedAttractParameterSets::MovingBandAndPixelate,
                "moving band and pixelate",
            ),
            (
                AdjustedAttractParameterSets::MovingTextAndPixelate,
                "moving text and pixelate",
            ),
            (
                AdjustedAttractParameterSets::MovingBandAndMovingTextAndPixelate,
                "moving band, moving text, and pixelate",
            ),
        ];

        for (adjusted_parameter_sets, expected_names) in cases {
            assert_eq!(adjusted_parameter_sets.names(), expected_names);
        }
    }

    #[test]
    fn shrinking_before_restore_reports_every_parameter_set_that_moves() {
        const SMALL_AREA: Rect = Rect::new(0, 0, 4, 3);

        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        let before_first_sizing = attract.configuration();
        attract.apply_settings(AttractSettings::MovingBand(attract.band.settings()));
        let ReplacementUndoState::Available(AttractConfigurationBeforeReplacement(before)) =
            attract.replacement_undo
        else {
            panic!("settings application should leave an undo point");
        };
        assert_ne!(
            before.band, before_first_sizing.band,
            "the first all-mode sizing adjusts the never-shown moving band before capture"
        );
        assert_eq!(
            attract.animation_sizing,
            AnimationSizing {
                band:   AnimationArea::Sized(AREA),
                text:   AnimationArea::Sized(AREA),
                pixels: AnimationArea::Sized(AREA),
            },
            "even modes never shown are sized before capture"
        );
        attract.record_terminal_resize(SMALL_AREA);

        let outcome = attract.restore_configuration_before_last_replacement();
        let AttractConfigurationRestoreOutcome::RestoredWithAdjustments {
            mode,
            adjusted_parameter_sets,
        } = outcome
        else {
            panic!("the smaller terminal should adjust the restored configuration");
        };

        assert_eq!(mode, before.mode);
        assert!(adjusted_parameter_sets.names().contains("moving band"));
        assert_eq!(
            attract.animation_sizing,
            AnimationSizing {
                band:   AnimationArea::Sized(SMALL_AREA),
                text:   AnimationArea::Sized(SMALL_AREA),
                pixels: AnimationArea::Sized(SMALL_AREA),
            },
            "every animation is sized before restore"
        );
    }

    #[test]
    fn restore_keeps_fade_progress_and_show_instruction_brings_the_screen_in() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.visibility_instruction = AttractVisibilityInstruction::Show;
        attract.grid_presentation = AttractGridPresentation::ReplacesGrid;
        attract.faded = 150;
        attract.apply_settings(AttractSettings::MovingBand(attract.band.settings()));
        attract.visibility_instruction = AttractVisibilityInstruction::Hide;
        let fade_at_restore = attract.faded;

        attract.restore_configuration_before_last_replacement();

        assert_eq!(attract.faded, fade_at_restore);
        assert_eq!(
            attract.visibility_instruction,
            AttractVisibilityInstruction::Show
        );
        attract.advance(AREA, Work::Running, Updates::Live, Instant::now());
        assert!(attract.faded < fade_at_restore);
    }

    #[test]
    fn restore_keeps_fade_progress_and_hide_instruction_sends_the_screen_out() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.visibility_instruction = AttractVisibilityInstruction::Hide;
        attract.grid_presentation = AttractGridPresentation::ReplacesGrid;
        attract.faded = 100;
        attract.apply_settings(AttractSettings::MovingBand(attract.band.settings()));
        attract.visibility_instruction = AttractVisibilityInstruction::Show;
        let fade_at_restore = attract.faded;

        attract.restore_configuration_before_last_replacement();

        assert_eq!(attract.faded, fade_at_restore);
        assert_eq!(
            attract.visibility_instruction,
            AttractVisibilityInstruction::Hide
        );
        attract.advance(AREA, Work::Idle, Updates::Live, Instant::now());
        assert!(attract.faded > fade_at_restore);
    }

    #[test]
    fn restored_follow_roster_uses_the_rosters_standing_at_restore_time() {
        let mut attract = Attract::new();
        attract.record_terminal_resize(AREA);
        attract.visibility_instruction = AttractVisibilityInstruction::FollowRoster;
        attract.grid_presentation = AttractGridPresentation::OverGrid;
        attract.standing = Standing::Showing;
        attract.faded = 100;
        attract.apply_settings(AttractSettings::MovingBand(attract.band.settings()));
        attract.visibility_instruction = AttractVisibilityInstruction::Show;
        attract.grid_presentation = AttractGridPresentation::ReplacesGrid;
        attract.standing = Standing::Working;
        let fade_at_restore = attract.faded;

        attract.restore_configuration_before_last_replacement();

        assert_eq!(attract.faded, fade_at_restore);
        assert_eq!(
            attract.visibility_instruction,
            AttractVisibilityInstruction::FollowRoster
        );
        assert_eq!(attract.grid_presentation, AttractGridPresentation::OverGrid);
        assert_eq!(attract.standing, Standing::Working);
        attract.advance(AREA, Work::Running, Updates::Live, Instant::now());
        assert!(attract.faded > fade_at_restore);
    }

    #[test]
    fn request_show_reverses_a_fade_out() {
        let mut attract = Attract::new();
        attract.request_show();
        assert_eq!(settle(&mut attract, Work::Idle), 0);
        attract.toggle();
        let now = Instant::now();
        attract.advance(AREA, Work::Idle, Updates::Live, now);
        let fading_out = attract.faded;

        assert!(attract.showing());
        assert!(fading_out > 0);
        attract.request_show();
        attract.advance(AREA, Work::Idle, Updates::Live, now + POLL);

        assert!(attract.asked_for());
        assert!(attract.faded < fading_out);
    }

    #[test]
    fn frozen_frame_records_its_laid_out_area() {
        let mut attract = Attract::new();
        let resized = Rect::new(0, 0, AREA.width.saturating_sub(7), AREA.height);

        attract.advance(resized, Work::Running, Updates::Frozen, Instant::now());

        assert_eq!(attract.laid_out_area, FrameArea::LaidOut(resized));
    }

    #[test]
    fn frozen_frame_does_not_change_band_glyphs() {
        let mut attract = Attract::new();
        attract.mode = AttractMode::MovingBand;
        attract.record_terminal_resize(AREA);
        let _ = attract.current_settings();
        let band_before_frame = attract.band.clone();

        attract.advance(AREA, Work::Idle, Updates::Frozen, Instant::now());

        assert_eq!(attract.band, band_before_frame);
    }

    #[test]
    fn live_visible_frame_advances_band_once() {
        let mut attract = Attract::new();
        attract.mode = AttractMode::MovingBand;
        attract.record_terminal_resize(AREA);
        let _ = attract.current_settings();
        attract.faded = 0;
        let previous = Instant::now();
        attract.advanced_at = previous;
        let mut expected = attract.band.clone();
        expected.advance(AREA, POLL);
        expected.fade(0);

        attract.advance(AREA, Work::Idle, Updates::Live, previous + POLL);

        assert_eq!(attract.band, expected);
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

    /// A showing attract screen reports no notice when its backdrop wait starts,
    /// then reports the unavailable capture at the exact grace-period boundary.
    #[test]
    fn a_screen_with_no_backdrop_to_draw_says_so_rather_than_drawing_nothing() {
        let mut attract = Attract::new();
        let started = Instant::now();
        let grace_elapsed = started + ATTRACT_BACKDROP_GRACE;

        attract.backdrop_wait = BackdropWait::WaitingSince(started);
        assert_eq!(
            attract.backdrop_notice(grace_elapsed),
            BackdropNotice::None,
            "a hidden screen does not report a missing backdrop",
        );

        attract.backdrop_wait = BackdropWait::NotWaiting;
        attract.advance(AREA, Work::Idle, Updates::Live, started);
        assert!(attract.showing(), "the screen is on");
        assert_eq!(
            attract.backdrop_notice(started),
            BackdropNotice::None,
            "a capture is not late the instant it is wanted",
        );
        assert_eq!(
            attract.backdrop_notice(grace_elapsed),
            BackdropNotice::CaptureUnavailable,
            "a capture missing at the grace boundary is reported",
        );
    }

    #[test]
    fn backdrop_notice_waits_for_the_grace_period_for_every_status() {
        for backdrop_status in [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready] {
            assert_eq!(
                classify_backdrop_notice(
                    BackdropGracePeriod::Remaining,
                    CurrentBackdrop::Missing,
                    backdrop_status,
                ),
                BackdropNotice::None,
                "backdrop_status={backdrop_status:?}",
            );
        }
        for failure in CAPTURE_FAILURES {
            assert_eq!(
                classify_backdrop_notice(
                    BackdropGracePeriod::Remaining,
                    CurrentBackdrop::Missing,
                    BackdropStatus::Failed(failure),
                ),
                BackdropNotice::None,
                "failure={failure:?}",
            );
        }
    }

    #[test]
    fn overdue_missing_backdrop_reports_waiting_and_ready_as_unavailable() {
        for backdrop_status in [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready] {
            assert_eq!(
                classify_backdrop_notice(
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
                CaptureFailure::UnsupportedPlatform
                | CaptureFailure::ShareableContentQueryFailed
                | CaptureFailure::TerminalWindowNotFound
                | CaptureFailure::DisplayNotFound
                | CaptureFailure::ScreenshotCaptureFailed
                | CaptureFailure::PixelExtractionFailed
                | CaptureFailure::ImageReductionFailed => BackdropNotice::CaptureUnavailable,
            };
            assert_eq!(
                classify_backdrop_notice(
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
    fn current_backdrop_suppresses_notice_after_the_latest_attempt_failed() {
        for grace_period in [BackdropGracePeriod::Remaining, BackdropGracePeriod::Elapsed] {
            for backdrop_status in [BackdropStatus::WaitingForFirstResult, BackdropStatus::Ready] {
                assert_eq!(
                    classify_backdrop_notice(
                        grace_period,
                        CurrentBackdrop::Available,
                        backdrop_status,
                    ),
                    BackdropNotice::None,
                    "grace_period={grace_period:?} backdrop_status={backdrop_status:?}",
                );
            }
            for failure in CAPTURE_FAILURES {
                assert_eq!(
                    classify_backdrop_notice(
                        grace_period,
                        CurrentBackdrop::Available,
                        BackdropStatus::Failed(failure),
                    ),
                    BackdropNotice::None,
                    "grace_period={grace_period:?} failure={failure:?}",
                );
            }
        }
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
