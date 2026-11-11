//! Runner recovery, terminal identification, and panic log constants.

use ratatui::style::Modifier;

// panic log
/// Use this location when the panic hook supplies none.
pub(super) const PANIC_LOCATION_UNKNOWN: &str = "an unknown location";
/// Use this message when the panic payload is not text.
pub(super) const PANIC_PAYLOAD_NOT_TEXT: &str = "Box<dyn Any>";

// repaint recovery
/// Recover half the terminal rows each second without adding an app frame.
pub(super) const RECOVERY_BATCH_MILLIS: u64 = 1_000;
/// This modifier combination forces a comparison row to differ from the rendered row.
pub(super) const REPAINT_SENTINEL: Modifier = Modifier::SLOW_BLINK
    .union(Modifier::RAPID_BLINK)
    .union(Modifier::CROSSED_OUT);

// terminal identification
/// iTerm2 sets this variable to the session profile name.
pub(super) const ITERM2_PROFILE_ENV: &str = "ITERM_PROFILE";
/// Value [`TERM_PROGRAM_ENV`] carries inside iTerm2.
pub(super) const ITERM2_TERM_PROGRAM: &str = "iTerm.app";
/// Environment variable naming the terminal emulator in use.
pub(super) const TERM_PROGRAM_ENV: &str = "TERM_PROGRAM";
