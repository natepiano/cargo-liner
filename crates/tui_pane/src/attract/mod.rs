//! The attract screen: what the terminal shows while the app has
//! nothing running.
//!
//! A grid with nothing in it is a screen with nothing to say, so the
//! app spends that time showing the desktop aligned under it.
//! [`BackdropMonitor`](crate::BackdropMonitor) hands back one colour per
//! character cell, taken from the captured macOS desktop or the
//! reconstructed KDE wallpaper, and [`TravelingBand`](crate::TravelingBand),
//! [`DriftingText`](crate::DriftingText) and
//! [`ResolvingPixels`](crate::ResolvingPixels) draw in those colours.
//!
//! [`Attract`] is the controller that decides when. The strip fades in
//! when the app's work empties and back out when something starts,
//! which is why [`Attract::render`] is called every frame rather than
//! only while idle -- the frames after work arrives are the ones that
//! carry it off the screen.
//!
//! Which animation is drawn is an [`AttractMode`], and the mode is also
//! the keymap scope the reader's keys resolve against while the screen
//! has been asked for: `+` widens the moving band rather than opening a
//! tile, and the other modes bind the same key to whatever they want --
//! or, as it happens, to nothing. `1`, `2` and `3` turn between them.
//! The app hosts the three scopes through [`AttractHost`] and registers
//! [`MovingBandPane`], [`MovingTextPane`] and [`PixelatePane`] with its
//! keymap.
//!
//! It can also be asked for outright, with a key the app binds to
//! [`Attract::toggle`]. A screen that only ever appears when there is
//! nothing to build is one that cannot be looked at on purpose -- and
//! the reader wanting to watch it is reason enough to show it over a
//! grid that is busy. Asked for, it takes the terminal rather than
//! sharing it: the [`AttractGrid`] that [`Attract::advance`] answers tells the
//! app's frame to leave the panes out, so what is drawn is the animation
//! and the status line and nothing else.
//!
//! Neither end of that is abrupt. [`AttractGrid::Empty`] holds the panes on
//! screen with nothing in them for as long as the animation is arriving
//! or leaving, and [`fade_to_background`] carries them toward the
//! colour they are painted on in step with it, [`attract_ground`]
//! standing in where a cell is painted on nothing. What that buys is a
//! background: a strip fading out over bare terminal has nothing to
//! fade into and goes dark instead of going away, and content appearing
//! under a strip still crossing it is the crowded look the screen exists
//! to avoid.
//!
//! Where there is no desktop to draw, [`draw_backdrop_notice`] writes the
//! [`BackdropNotice`] [`Attract::backdrop_notice`] settles on, in the
//! words the app's [`FrameProbe`](crate::FrameProbe) gives the case
//! where capture is unavailable.
//!
//! [`draw_attract_layers`] is the attract screen's share of the app's
//! frame: it advances the controller, has the app draw its panes as the
//! [`AttractGrid`] answer calls for, fades them, draws the animation over
//! them and writes the notice.
//!
//! [`Updates`] is whether the display takes new work in or is held
//! still, which the grid and the animation both answer to.

mod backdrop_notice;
mod constants;
mod controller;
mod fade;
mod frame;
mod held_key;
mod host;
mod moving_band;
mod moving_text;
mod pixelate;
mod random;
mod settings;
mod updates;

pub use backdrop_notice::BackdropNotice;
pub(crate) use constants::ATTRACT_BACKDROP_UNAVAILABLE_NOTICE;
pub(crate) use constants::NOTICE_TOAST_MIN_INTERIOR_LINES;
pub(crate) use constants::NOTICE_TOAST_VISIBLE;
pub use controller::AdjustedAttractParameterSets;
pub use controller::Attract;
pub use controller::AttractConfiguration;
pub use controller::AttractConfigurationRestoreOutcome;
pub use controller::AttractGrid;
pub use controller::AttractGridPresentation;
pub use controller::AttractMode;
pub use controller::AttractVisibilityInstruction;
pub use controller::AttractWork;
pub use controller::SettingsApplicationOutcome;
pub use fade::attract_ground;
pub use fade::draw_backdrop_notice;
pub use fade::fade_to_background;
pub use frame::draw_attract_layers;
pub use host::AttractHost;
pub use host::attract_pane_id;
pub use host::dispatch_attract_key;
pub(crate) use host::mode_label;
pub use host::undo_attract_replacement;
pub use moving_band::MovingBandAction;
pub use moving_band::MovingBandPane;
pub use moving_text::MovingTextAction;
pub use moving_text::MovingTextPane;
pub use pixelate::PixelateAction;
pub use pixelate::PixelatePane;
pub(crate) use random::EmptyIndexDomain;
pub(crate) use random::NonZeroIndexBound;
pub(crate) use random::bounded_index;
pub(crate) use random::clock_seed;
pub use settings::AttractSettings;
pub use updates::Updates;
