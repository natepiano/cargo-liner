//! Constants for the runner: the full-repaint cadence and its marker,
//! and the environment iTerm2 announces itself through.

use ratatui::style::Modifier;

// repaint
/// Laid over the comparison buffer to make every cell differ from
/// anything a frame can render, which is what turns the next draw into
/// a full repaint.
///
/// The difference is carried by modifiers rather than by the symbol.
/// An unrenderable symbol would have been the obvious choice, but
/// ratatui measures every symbol's display width and rejects control
/// characters on the way, so the only symbols it accepts are ones a
/// frame could legitimately hold. Nothing these apps draw blinks, so
/// the combination below is one no rendered cell ever carries.
pub(super) const REPAINT_SENTINEL: Modifier = Modifier::SLOW_BLINK
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT);
/// How often the screen is redrawn cell for cell rather than by
/// difference.
///
/// ratatui writes only the cells that changed since the last frame, so
/// anything put on this terminal by something other than this app --
/// a pane manager splitting the window, a stray line landing on the
/// same tty -- stays where it is for good: both buffers agree those
/// cells already hold what they should, and nothing ever writes over
/// them. A redraw on this cadence is what repairs that, and it is far
/// enough apart to cost nothing while being well inside the time it
/// takes to notice a smear.
pub(super) const FULL_REPAINT_SECONDS: u64 = 2;

// iterm2
/// Environment variable naming the terminal emulator in use.
pub(super) const TERM_PROGRAM_ENV: &str = "TERM_PROGRAM";
/// Value [`TERM_PROGRAM_ENV`] carries inside iTerm2.
pub(super) const ITERM2_TERM_PROGRAM: &str = "iTerm.app";
/// Environment variable iTerm2 sets to the name of the profile the
/// session started on.
pub(super) const ITERM2_PROFILE_ENV: &str = "ITERM_PROFILE";
