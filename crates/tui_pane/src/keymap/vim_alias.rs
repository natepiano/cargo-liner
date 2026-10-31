//! Vim motion validation, help-row conflicts and generated navigation aliases.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;

use super::KeyBind;
use super::KeySequence;
use super::KeymapHelpRow;
use super::KeymapHelpRowKind;
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
    if mode == VimMode::Enabled && is_vim_motion_bind(bind) {
        Err(ReservedVimMotionKey { bind })
    } else {
        Ok(())
    }
}

/// Every supplied action row bound to an unmodified Vim motion key, named
/// `scope.action`, in the supplied order. Header rows are skipped.
#[must_use]
pub fn vim_motion_conflicts(rows: impl IntoIterator<Item = KeymapHelpRow>) -> Vec<String> {
    rows.into_iter()
        .filter(|row| row.row_kind != KeymapHelpRowKind::Header)
        .filter_map(|row| {
            let bind = row.bind?.single_key()?;
            is_vim_motion_bind(bind).then(|| format!("{}.{}", row.scope, row.action))
        })
        .collect()
}

/// Whether a navigation binding is a generated Vim alias.
///
/// Every vim navigation alias is generated and stripped from the
/// written TOML — the letters (h/j/k/l/G) and the Ctrl page /
/// half-page motions (Ctrl-b/f/u/d). None of these are keymappable;
/// they exist only while vim mode is on (see
/// `super::nav_action::vim_letter_extras`).
#[must_use]
pub fn is_navigation_generated_vim_extra(action_key: &str, key_sequence: &KeySequence) -> bool {
    let Some(framework_key_bind) = key_sequence.single_key() else {
        return action_key == "home"
            && key_sequence.keys() == [KeyBind::from('g'), KeyBind::from('g')];
    };
    matches!(
        (action_key, framework_key_bind.code, framework_key_bind.mods),
        ("left", KeyCode::Char('h'), KeyModifiers::NONE)
            | ("down", KeyCode::Char('j'), KeyModifiers::NONE)
            | ("up", KeyCode::Char('k'), KeyModifiers::NONE)
            | ("right", KeyCode::Char('l'), KeyModifiers::NONE)
            | ("end", KeyCode::Char('G'), KeyModifiers::NONE)
            | ("page_up", KeyCode::Char('b'), KeyModifiers::CONTROL)
            | ("page_down", KeyCode::Char('f'), KeyModifiers::CONTROL)
            | ("half_page_up", KeyCode::Char('u'), KeyModifiers::CONTROL)
            | ("half_page_down", KeyCode::Char('d'), KeyModifiers::CONTROL)
    )
}

fn is_vim_motion_bind(bind: KeyBind) -> bool {
    bind.mods == KeyModifiers::NONE && matches!(bind.code, KeyCode::Char('h' | 'j' | 'k' | 'l'))
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

    #[test]
    fn motion_conflicts_keep_action_order_and_skip_headers_modified_keys_and_chords() {
        let row = |action, bind| KeymapHelpRow {
            section: "Editor",
            scope: "editor",
            action,
            description: "",
            bind,
            row_kind: KeymapHelpRowKind::Action,
        };
        let mut header = row("heading", Some('h'.into()));
        header.row_kind = KeymapHelpRowKind::Header;
        let mut rows = vec![header, row("unbound", None)];
        for (action, letter) in [("left", 'h'), ("down", 'j'), ("up", 'k'), ("right", 'l')] {
            rows.push(row(action, Some(letter.into())));
            for mods in [
                KeyModifiers::SHIFT,
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ] {
                rows.push(row(
                    action,
                    Some(
                        KeyBind {
                            code: KeyCode::Char(letter),
                            mods,
                        }
                        .into(),
                    ),
                ));
            }
        }
        rows.extend([
            row("uppercase", Some('H'.into())),
            row("arrow", Some(KeyCode::Up.into())),
            row("other", Some('x'.into())),
            row(
                "chord",
                Some(KeySequence::new(vec!['h'.into(), 'j'.into()])),
            ),
            row("empty", Some(KeySequence::new(Vec::new()))),
            row("left", Some('h'.into())),
        ]);
        assert_eq!(
            vim_motion_conflicts(rows),
            [
                "editor.left",
                "editor.down",
                "editor.up",
                "editor.right",
                "editor.left"
            ]
        );
    }

    #[test]
    fn generated_navigation_aliases_match_only_their_action_and_exact_sequence() {
        for (action, sequence) in [
            ("left", KeySequence::from('h')),
            ("down", KeySequence::from('j')),
            ("up", KeySequence::from('k')),
            ("right", KeySequence::from('l')),
            ("end", KeySequence::from('G')),
            ("page_up", KeyBind::ctrl('b').into()),
            ("page_down", KeyBind::ctrl('f').into()),
            ("half_page_up", KeyBind::ctrl('u').into()),
            ("half_page_down", KeyBind::ctrl('d').into()),
            ("home", KeySequence::new(vec!['g'.into(), 'g'.into()])),
        ] {
            assert!(is_navigation_generated_vim_extra(action, &sequence));
            assert!(!is_navigation_generated_vim_extra("other", &sequence));
            let mut modified = sequence.keys().to_vec();
            modified[0].mods |= KeyModifiers::ALT;
            assert!(!is_navigation_generated_vim_extra(
                action,
                &KeySequence::new(modified)
            ));
            let mut extended = sequence.keys().to_vec();
            extended.push('x'.into());
            assert!(!is_navigation_generated_vim_extra(
                action,
                &KeySequence::new(extended)
            ));
        }
        for (action, sequence) in [
            ("home", KeySequence::from('g')),
            ("home", KeySequence::new(vec!['G'.into(), 'G'.into()])),
            ("end", KeySequence::from('g')),
            ("left", KeySequence::from('H')),
            ("page_up", KeySequence::from('b')),
            ("left", KeyCode::Left.into()),
            ("expand_row", KeySequence::from('l')),
        ] {
            assert!(!is_navigation_generated_vim_extra(action, &sequence));
        }
        assert!(!is_navigation_generated_vim_extra(
            "home",
            &KeySequence::new(Vec::new())
        ));
    }
}
