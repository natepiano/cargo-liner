//! Reading the tmux sessions a shell call opens: the name each
//! `new-session` gives with `-s`, where it is written as a literal.
//!
//! The command is split into words the way a shell splits it, as far
//! as reading those names needs: quotes are taken off, and a word with
//! anything the shell expands in it -- a `$` or a backquote outside
//! single quotes -- is no literal, since the name it gives is known
//! only once the shell has run it.

use crate::constants::TMUX_ARGUMENT_FLAGS;
use crate::constants::TMUX_NEW_SESSION;
use crate::constants::TMUX_OPTIONS_END;
use crate::constants::TMUX_SESSION_NAME_FLAG;

/// One piece of a shell command, as reading `-s` needs it.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    /// A word, with its quotes taken off.
    Word {
        /// The word's text.
        text:     String,
        /// Whether the shell expands something in it.
        expanded: bool,
    },
    /// What ends a command: `;`, `&`, `|`, `(`, `)` or a line break.
    Break,
}

/// How each `new-session` in `command` names the session it opens, in
/// the order they are written: the name where `-s` gives it as a
/// literal, else none -- no `-s`, or one built by the shell.
pub(super) fn opened_sessions(command: &str) -> Vec<Option<String>> {
    let tokens = tokens(command);
    tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| matches!(token, Token::Word { text, .. } if text == TMUX_NEW_SESSION))
        .map(|(index, _)| session_name(&tokens[index + 1..]))
        .collect()
}

/// The literal `-s` gives among `options`, the words after a
/// `new-session`, read the way tmux reads them: flags may be clustered,
/// a flag's argument is the rest of its word or else the next word, and
/// the options end at the first word that is not one.
fn session_name(options: &[Token]) -> Option<String> {
    let mut words = options.iter();
    while let Some(Token::Word { text, expanded }) = words.next() {
        if text == TMUX_OPTIONS_END {
            return None;
        }
        let flags = text.strip_prefix('-').filter(|flags| !flags.is_empty())?;
        for (at, flag) in flags.char_indices() {
            if !TMUX_ARGUMENT_FLAGS.contains(flag) {
                continue;
            }
            let joined = &flags[at + flag.len_utf8()..];
            let (argument, built) = if joined.is_empty() {
                match words.next() {
                    Some(Token::Word { text, expanded }) => (text.as_str(), *expanded),
                    _ => return None,
                }
            } else {
                (joined, *expanded)
            };
            if flag == TMUX_SESSION_NAME_FLAG {
                return (!built).then(|| argument.to_string());
            }
            // The rest of the cluster, or the next word, was this flag's
            // argument.
            break;
        }
    }
    None
}

/// `command` split into words and command breaks.
fn tokens(command: &str) -> Vec<Token> {
    let mut words = Words::default();
    let mut characters = command.chars();
    while let Some(character) = characters.next() {
        match character {
            ' ' | '\t' => words.end_word(),
            '\n' | ';' | '&' | '|' | '(' | ')' => {
                words.end_word();
                words.tokens.push(Token::Break);
            },
            '\'' => {
                words.open = true;
                words
                    .text
                    .extend(characters.by_ref().take_while(|&inner| inner != '\''));
            },
            '"' => {
                words.open = true;
                while let Some(inner) = characters.next() {
                    match inner {
                        '"' => break,
                        '\\' => match characters.next() {
                            Some(escaped @ ('$' | '`' | '"' | '\\')) => words.text.push(escaped),
                            Some('\n') | None => {},
                            Some(other) => {
                                words.text.push('\\');
                                words.text.push(other);
                            },
                        },
                        '$' | '`' => {
                            words.expanded = true;
                            words.text.push(inner);
                        },
                        other => words.text.push(other),
                    }
                }
            },
            '\\' => {
                words.open = true;
                if let Some(escaped) = characters.next().filter(|&escaped| escaped != '\n') {
                    words.text.push(escaped);
                }
            },
            '$' | '`' => {
                words.open = true;
                words.expanded = true;
                words.text.push(character);
            },
            other => {
                words.open = true;
                words.text.push(other);
            },
        }
    }
    words.end_word();
    words.tokens
}

/// The words read so far, and the one being read.
#[derive(Debug, Default)]
struct Words {
    /// Every word and break finished so far.
    tokens:   Vec<Token>,
    /// The word being read.
    text:     String,
    /// Whether the shell expands something in the word being read.
    expanded: bool,
    /// Whether a word is being read, which an empty pair of quotes
    /// starts though it adds no text.
    open:     bool,
}

impl Words {
    /// Finish the word being read, if one is.
    fn end_word(&mut self) {
        if self.open {
            self.tokens.push(Token::Word {
                text:     std::mem::take(&mut self.text),
                expanded: self.expanded,
            });
        }
        self.expanded = false;
        self.open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A literal name counts wherever `-s` sits among the options and
    /// however it is written: after other flags and their arguments,
    /// quoted, clustered with another flag, or joined to its flag.
    #[test]
    fn a_literal_name_is_read_from_the_options() {
        let cases = [
            ("tmux new-session -d -s scratch zsh", "scratch"),
            (
                "tmux new-session -d -x 200 -y 50 -s 'wide one' zsh",
                "wide one",
            ),
            (r#"tmux new-session -d -s "trunk" -c $dir zsh"#, "trunk"),
            ("tmux new-session -ds arrange", "arrange"),
            ("tmux new-session -sjoined", "joined"),
            (r"tmux new-session -s back\ slashed", "back slashed"),
        ];
        for (command, name) in cases {
            assert_eq!(
                opened_sessions(command),
                [Some(name.to_string())],
                "{command}"
            );
        }
    }

    /// A name the shell builds is no literal: a variable, bare or quoted
    /// or inside a longer word, or a command's output. Single quotes
    /// keep a `$` literal.
    #[test]
    fn a_built_name_is_no_literal() {
        for command in [
            "tmux new-session -d -s $name zsh",
            r#"tmux new-session -d -s "$name" zsh"#,
            r#"tmux new-session -d -s "tool-based-ui-$name" zsh"#,
            "tmux new-session -d -s `hostname` zsh",
        ] {
            assert_eq!(opened_sessions(command), [None], "{command}");
        }
        assert_eq!(
            opened_sessions("tmux new-session -s '$name'"),
            [Some("$name".to_string())]
        );
    }

    /// A `new-session` with no `-s` among its options names nothing: an
    /// `-s` after the command it runs, or after `--`, belongs to that
    /// command.
    #[test]
    fn a_session_with_no_name_among_its_options_names_nothing() {
        for command in [
            "tmux new-session -d",
            "tmux new-session -d zsh -s other",
            "tmux new-session -d -- -s other",
            "tmux new-session -d; echo -s other",
        ] {
            assert_eq!(opened_sessions(command), [None], "{command}");
        }
    }

    /// Each `new-session` in a call is read on its own, and a command
    /// with none opens nothing.
    #[test]
    fn every_session_a_call_opens_is_read() {
        let script = "launch() {\n  $TM new-session -d -s $name zsh\n}\n\
                      $TM new-session -d -s keepalive && launch trunk";
        assert_eq!(
            opened_sessions(script),
            [None, Some("keepalive".to_string())]
        );
        assert_eq!(opened_sessions("tmux ls"), [] as [Option<String>; 0]);
    }
}
