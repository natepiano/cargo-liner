//! Switching the iTerm2 session profile for the life of the app.
//!
//! iTerm2 keeps window transparency, blur, and the background image on
//! the *profile*, not in any escape sequence, so an app cannot ask for
//! them directly. What it can do is ask the session to adopt a profile
//! that already carries them, which is what this module does: switch on
//! the way in, switch back on the way out.
//!
//! Everything here is a no-op outside iTerm2.

use std::env;
use std::io;
use std::io::Write;
use std::panic;

use crate::constants::ITERM2_PROFILE_ENV;
use crate::constants::ITERM2_TERM_PROGRAM;
use crate::constants::TERM_PROGRAM_ENV;

/// A profile switch that is in force, holding the profile to go back to.
///
/// Only ever constructed when the switch actually happened, so holding
/// one is the proof that something needs undoing.
pub(crate) struct ProfileSwitch {
    /// Profile the session was on before the app took it over.
    previous: String,
}

impl ProfileSwitch {
    /// Move the session onto `profile`, returning the switch to undo
    /// later, or `None` when nothing was changed.
    ///
    /// Nothing is changed unless this is iTerm2, the session reports
    /// which profile it is on, and that is not already `profile` --
    /// the last of which also means a session started under the app's
    /// own profile is left alone. Refusing on incomplete information is
    /// the point: a switch this cannot reverse is worse than no switch,
    /// because it outlives the process and lands on the user's shell.
    pub(crate) fn enter(profile: &str, out: &mut impl Write) -> io::Result<Option<Self>> {
        if !is_iterm2() || !is_writable_name(profile) {
            return Ok(None);
        }
        let previous = match env::var(ITERM2_PROFILE_ENV) {
            Ok(previous) if is_writable_name(&previous) && previous != profile => previous,
            _ => return Ok(None),
        };
        set_profile(out, profile)?;
        Ok(Some(Self { previous }))
    }

    /// Put the session back on the profile it came in on.
    pub(crate) fn leave(&self, out: &mut impl Write) -> io::Result<()> {
        set_profile(out, &self.previous)
    }
}

/// Restore the session profile if the app dies on a panic.
///
/// A panic skips the ordinary teardown, and an unrestored profile is
/// the one piece of that teardown the user cannot put right by looking
/// at it -- raw mode announces itself, a quietly different window
/// transparency does not. The name to go back to is read from the
/// environment rather than carried in, so the hook needs no state and
/// cannot itself fail on a poisoned lock.
pub(crate) fn install_panic_restore() {
    if !is_iterm2() {
        return;
    }
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if let Ok(profile) = env::var(ITERM2_PROFILE_ENV)
            && is_writable_name(&profile)
        {
            let mut stdout = io::stdout();
            let _ = set_profile(&mut stdout, &profile);
        }
        previous(info);
    }));
}

/// Whether this process is talking to iTerm2.
fn is_iterm2() -> bool {
    env::var(TERM_PROGRAM_ENV).is_ok_and(|program| program == ITERM2_TERM_PROGRAM)
}

/// Whether a profile name can be put inside an escape sequence as-is.
///
/// The name is written verbatim into a control sequence, so a control
/// character in it would end the sequence early and let the rest be
/// read as terminal commands. A name carrying one is refused outright
/// rather than stripped, because a half-matched profile name would
/// silently select the wrong profile.
fn is_writable_name(name: &str) -> bool { !name.is_empty() && !name.chars().any(char::is_control) }

/// Ask iTerm2 to move this session onto a named profile.
///
/// `OSC 1337 ; SetProfile=<name> BEL`, iTerm2's own extension. Any
/// other terminal ignores an unknown OSC, and iTerm2 ignores a name no
/// profile has, so the worst case either way is that nothing happens.
fn set_profile(out: &mut impl Write, name: &str) -> io::Result<()> {
    write!(out, "\x1b]1337;SetProfile={name}\x07")?;
    out.flush()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn set_profile_writes_the_iterm2_sequence() {
        let mut written = Vec::new();
        set_profile(&mut written, "cargo-tile").expect("writing to a vec cannot fail");
        assert_eq!(
            String::from_utf8(written).expect("the sequence is utf-8"),
            "\x1b]1337;SetProfile=cargo-tile\x07",
            "the profile name sits between iTerm2's OSC 1337 introducer and a BEL"
        );
    }

    #[test]
    fn a_name_carrying_a_control_character_is_refused() {
        assert!(
            !is_writable_name("cargo\x07tile"),
            "a BEL would end the sequence early and leave the rest to be read as commands"
        );
        assert!(
            !is_writable_name("cargo\x1btile"),
            "an escape would start a sequence of its own"
        );
        assert!(!is_writable_name(""), "an empty name selects nothing");
    }

    #[test]
    fn an_ordinary_name_is_written_as_it_stands() {
        assert!(
            is_writable_name("Default"),
            "the common case is a plain profile name"
        );
        assert!(
            is_writable_name("cargo-tile dark"),
            "spaces and dashes are ordinary in an iTerm2 profile name"
        );
    }
}
