//! Runner recovery, terminal identification, and panic log constants.

use ratatui::style::Modifier;

// `RECOVERY_BATCH_MILLIS` and `REPAINT_SENTINEL` control terminal recovery.
/// Recover half the terminal rows each second without adding an app frame.
pub(super) const RECOVERY_BATCH_MILLIS: u64 = 1_000;
/// This modifier combination forces a comparison row to differ from the rendered row.
pub(super) const REPAINT_SENTINEL: Modifier = Modifier::SLOW_BLINK
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT);

// `TERM_PROGRAM_ENV`, `ITERM2_TERM_PROGRAM`, and `ITERM2_PROFILE_ENV` identify iTerm2.
/// Environment variable naming the terminal emulator in use.
pub(super) const TERM_PROGRAM_ENV: &str = "TERM_PROGRAM";
/// Value [`TERM_PROGRAM_ENV`] carries inside iTerm2.
pub(super) const ITERM2_TERM_PROGRAM: &str = "iTerm.app";
/// Environment variable iTerm2 sets to the name of the profile the
/// session started on.
pub(super) const ITERM2_PROFILE_ENV: &str = "ITERM_PROFILE";

// `PANIC_LOCATION_UNKNOWN` and `PANIC_PAYLOAD_NOT_TEXT` supply panic-log fallbacks.
/// Written in place of a panic's location when the hook is not given one.
pub(super) const PANIC_LOCATION_UNKNOWN: &str = "an unknown location";
/// Written in place of a panic's message when its payload is not text,
/// as the standard library's own hook prints it.
pub(super) const PANIC_PAYLOAD_NOT_TEXT: &str = "Box<dyn Any>";
