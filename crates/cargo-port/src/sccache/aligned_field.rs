/// A whitespace run's byte offset and character count within a stats line.
struct Gap {
    start: usize,
    len:   usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SccacheStatFieldMatch<'a> {
    Field { label: &'a str, value: &'a str },
    NotAlignedField,
}

/// Split one `sccache --show-stats` line at its value column: the first run
/// of two or more whitespace characters separates the label from the value.
pub(crate) fn split_aligned_stat(text: &str) -> SccacheStatFieldMatch<'_> {
    let mut gap: Option<Gap> = None;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            match &mut gap {
                Some(gap) => gap.len += 1,
                None => {
                    gap = Some(Gap {
                        start: idx,
                        len:   1,
                    });
                },
            }
            continue;
        }
        if let Some(gap) = gap.take()
            && gap.len >= 2
        {
            let label = text[..gap.start].trim_end();
            let value = text[idx..].trim();
            if !label.is_empty() && !value.is_empty() {
                return SccacheStatFieldMatch::Field { label, value };
            }
        }
    }
    SccacheStatFieldMatch::NotAlignedField
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_stat_requires_two_whitespace_characters_and_a_label_and_value() {
        for (text, expected) in [
            ("Cache size 101 GiB", None),
            ("Cache size  101 GiB", Some(("Cache size", "101 GiB"))),
            ("Cache size   101 GiB  ", Some(("Cache size", "101 GiB"))),
            ("Cache size  ", None),
            ("Cache size", None),
            ("A B C", None),
            ("", None),
            ("  ", None),
            ("  Cache size  101 GiB", Some(("  Cache size", "101 GiB"))),
            ("Cache size\t\t101 GiB", Some(("Cache size", "101 GiB"))),
            ("Cache size\u{2003}101 GiB", None),
            (
                "Taille café\u{2003}\u{2003}101 GiB",
                Some(("Taille café", "101 GiB")),
            ),
            (
                "Cache location  Local disk:  /tmp/cache",
                Some(("Cache location", "Local disk:  /tmp/cache")),
            ),
        ] {
            let expected = expected
                .map_or(SccacheStatFieldMatch::NotAlignedField, |(label, value)| {
                    SccacheStatFieldMatch::Field { label, value }
                });
            assert_eq!(split_aligned_stat(text), expected, "{text:?}");
        }
    }
}
