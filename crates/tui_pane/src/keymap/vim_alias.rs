//! Validation for the four unmodified Vim motion keys.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use super::KeyBind;
use super::VimMode;

/// A binding reserved for Vim motion while Vim navigation is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservedVimMotionKey {
    bind: KeyBind,
}

impl Display for ReservedVimMotionKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "\"{}\" reserved for vim navigation",
            self.bind.display()
        )
    }
}

impl Error for ReservedVimMotionKey {}

/// Reject unmodified h/j/k/l only when Vim navigation is enabled.
///
/// # Errors
/// Returns the rejected binding when it is reserved for Vim navigation.
pub fn validate_vim_motion_bind(mode: VimMode, bind: KeyBind) -> Result<(), ReservedVimMotionKey> {
    if mode == VimMode::Enabled
        && bind.mods == KeyModifiers::NONE
        && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l'))
    {
        Err(ReservedVimMotionKey { bind })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_vim_rejects_only_unmodified_motion_letters() {
        for letter in ['h', 'j', 'k', 'l'] {
            let bind = KeyBind::from(letter);
            assert_eq!(
                validate_vim_motion_bind(VimMode::Enabled, bind),
                Err(ReservedVimMotionKey { bind })
            );
            assert_eq!(
                ReservedVimMotionKey { bind }.to_string(),
                format!("\"{letter}\" reserved for vim navigation")
            );
            assert_eq!(validate_vim_motion_bind(VimMode::Disabled, bind), Ok(()));
            for mods in [
                KeyModifiers::SHIFT,
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ] {
                assert_eq!(
                    validate_vim_motion_bind(
                        VimMode::Enabled,
                        KeyBind {
                            code: bind.code,
                            mods
                        }
                    ),
                    Ok(())
                );
            }
        }
        for code in [
            KeyCode::Char('x'),
            KeyCode::Char('H'),
            KeyCode::Up,
            KeyCode::Esc,
        ] {
            assert_eq!(
                validate_vim_motion_bind(VimMode::Enabled, KeyBind::from(code)),
                Ok(())
            );
        }
    }
}
