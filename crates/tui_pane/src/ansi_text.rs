/// Strip ANSI escape sequences from `raw`, leaving only the printable
/// text. Reuses the same parser the output renderer feeds, so the copied
/// text matches what is on screen.
#[must_use]
pub fn strip_ansi(raw: &str) -> String {
    let safe = sanitize_ansi_for_output(raw);
    ansi_to_tui::IntoText::into_text(&safe).map_or_else(
        |_| strip_control_sequences(&safe),
        |text| {
            text.lines
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

/// Retain SGR styling while removing other terminal controls.
#[must_use]
pub fn sanitize_ansi_for_output(raw: &str) -> String { sanitize_ansi(raw, true) }

fn strip_control_sequences(raw: &str) -> String { sanitize_ansi(raw, false) }

fn sanitize_ansi(raw: &str, preserve_sgr: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut state = AnsiScanState::Ground;
    for ch in raw.chars() {
        state = state.consume(ch, &mut out, preserve_sgr);
    }
    out
}

enum AnsiScanState {
    Ground,
    Escape,
    Csi(String),
    ControlString,
    ControlStringEscape,
}

impl AnsiScanState {
    fn consume(self, ch: char, out: &mut String, preserve_sgr: bool) -> Self {
        match self {
            Self::Ground => consume_ground(ch, out),
            Self::Escape => consume_escape(ch),
            Self::Csi(mut sequence) => {
                sequence.push(ch);
                if is_csi_final(ch) {
                    if preserve_sgr && ch == 'm' {
                        out.push_str(&sequence);
                    }
                    Self::Ground
                } else {
                    Self::Csi(sequence)
                }
            },
            Self::ControlString => match ch {
                '\x07' => Self::Ground,
                '\x1b' => Self::ControlStringEscape,
                _ => Self::ControlString,
            },
            Self::ControlStringEscape => {
                if ch == '\\' {
                    Self::Ground
                } else {
                    Self::ControlString
                }
            },
        }
    }
}

fn consume_ground(ch: char, out: &mut String) -> AnsiScanState {
    match ch {
        '\x1b' => AnsiScanState::Escape,
        '\t' => {
            out.push(' ');
            AnsiScanState::Ground
        },
        _ if ch.is_control() => AnsiScanState::Ground,
        _ => {
            out.push(ch);
            AnsiScanState::Ground
        },
    }
}

fn consume_escape(ch: char) -> AnsiScanState {
    match ch {
        '[' => AnsiScanState::Csi("\x1b[".to_string()),
        ']' | 'P' | 'X' | '^' | '_' => AnsiScanState::ControlString,
        _ => AnsiScanState::Ground,
    }
}

const fn is_csi_final(ch: char) -> bool { matches!(ch, '\u{40}'..='\u{7e}') }

#[cfg(test)]
mod tests {
    use super::sanitize_ansi_for_output;
    use super::strip_ansi;
    use super::strip_control_sequences;

    #[test]
    fn styling_survives_sanitization_but_not_plain_text_conversion() {
        let raw = "before\x1b[31mred\x1b[0mafter";
        assert_eq!(sanitize_ansi_for_output(raw), raw);
        assert_eq!(strip_ansi(raw), "beforeredafter");
        assert_eq!(strip_control_sequences(raw), "beforeredafter");
    }

    #[test]
    fn control_strings_cursor_commands_and_unterminated_sequences_are_removed() {
        for sequence in [
            "\x1b]title\x07",
            "\x1b]title\x1b\\",
            "\x1bPdata\x1b\\",
            "\x1bXdata\x1b\\",
            "\x1b^data\x1b\\",
            "\x1b_data\x1b\\",
            "\x1b[2J",
        ] {
            let raw = format!("left{sequence}right");
            assert_eq!(sanitize_ansi_for_output(&raw), "leftright");
            assert_eq!(strip_ansi(&raw), "leftright");
        }
        for suffix in ["\x1b", "\x1b[31", "\x1b]unfinished", "\x1bPunfinished\x1b"] {
            assert_eq!(sanitize_ansi_for_output(&format!("kept{suffix}")), "kept");
        }
    }

    #[test]
    fn tabs_become_spaces_and_other_control_characters_drop() {
        let raw = "a\tb\rc\nd\x00e\x07f";
        assert_eq!(sanitize_ansi_for_output(raw), "a bcdef");
        assert_eq!(strip_ansi(raw), "a bcdef");
    }
}
