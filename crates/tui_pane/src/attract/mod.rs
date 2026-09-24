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
//! Neither end of that is abrupt. The panes stay on screen with
//! nothing in them for as long as the animation is arriving or leaving,
//! and [`fade_to_background`] carries them toward the colour they are
//! painted on in step with it, [`attract_ground`] standing in where a
//! cell is painted on nothing. What that buys is a background: a strip
//! fading out over bare terminal has nothing to fade into and goes dark
//! instead of going away.
//!
//! [`Updates`] is whether the display takes new work in or is held
//! still, which the grid and the animation both answer to.

mod fade;
mod updates;

pub use fade::attract_ground;
pub use fade::fade_to_background;
pub use updates::Updates;
