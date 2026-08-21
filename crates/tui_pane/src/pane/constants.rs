// pane tint
//
// Every pane paints its own background, focused or not, so focus is a
// difference between two opaque colours rather than the difference
// between a painted pane and a bare one.
//
// That matters under a transparent terminal window. A cell with no
// background of its own is the *default* background, and terminals that
// composite a window over the desktop -- iTerm2 among them -- treat
// that cell specially: with "Only the default background color uses
// transparency" ticked, a bare cell shows the desktop while a painted
// one goes solid. Painting every pane takes both panes out of that
// special case, so untick that option and the window's transparency
// applies to the whole grid evenly. The tints below then read as
// colour through that transparency, which is the two-stage composite:
// the alphas here mix a pane against the theme background, and the
// terminal mixes the result against whatever is behind the window.
//
// Leaving the option ticked instead makes the whole grid opaque, since
// nothing is bare any more.

/// Where a background stops counting as dark and starts counting as
/// light, on the average of its three channels.
pub(super) const PANE_TINT_BRIGHTNESS_MIDPOINT: u16 = 128;
/// A fully opaque overlay, against which the alphas below are read.
pub(super) const PANE_TINT_ALPHA_WHOLE: u16 = 100;
/// How much of [`PANE_TINT_DARK_OVERLAY`] lies over a dark background
/// when the pane has focus.
pub(super) const PANE_TINT_DARK_FOCUSED_ALPHA: u16 = 26;
/// The same over a dark background without focus: enough to lift the
/// pane off whatever shows through the window, far enough below
/// [`PANE_TINT_DARK_FOCUSED_ALPHA`] to keep focus obvious.
pub(super) const PANE_TINT_DARK_UNFOCUSED_ALPHA: u16 = 8;
/// How much of [`PANE_TINT_LIGHT_OVERLAY`] lies over a light
/// background when the pane has focus.
pub(super) const PANE_TINT_LIGHT_FOCUSED_ALPHA: u16 = 20;
/// The same over a light background without focus.
pub(super) const PANE_TINT_LIGHT_UNFOCUSED_ALPHA: u16 = 6;
/// Laid over a dark background, so a pane lifts away from black toward
/// a cool white rather than flattening into grey.
pub(super) const PANE_TINT_DARK_OVERLAY: (u8, u8, u8) = (180, 180, 220);
/// Laid over a light background: the same hue at the other end, so a
/// pane settles away from white the way it lifts away from black.
pub(super) const PANE_TINT_LIGHT_OVERLAY: (u8, u8, u8) = (100, 100, 145);

// shared pane frame
/// Cells one border line occupies, on one side of a pane.
pub(super) const BORDER_LINE_WIDTH: u16 = 1;
