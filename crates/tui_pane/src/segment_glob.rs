//! Byte-based glob matching within a segment.

/// Match a whole segment with `*` for any run of bytes (including empty)
/// and `?` for exactly one byte.
#[must_use]
pub fn glob_segment_matches(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        match pattern.split_first() {
            None => value.is_empty(),
            Some((b'*', rest)) => {
                matches(rest, value) || (!value.is_empty() && matches(pattern, &value[1..]))
            },
            Some((b'?', rest)) => !value.is_empty() && matches(rest, &value[1..]),
            Some((head, rest)) => {
                !value.is_empty() && *head == value[0] && matches(rest, &value[1..])
            },
        }
    }

    matches(pattern.as_bytes(), value.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_consumes_the_whole_segment_including_empty_inputs() {
        for (pattern, value, expected) in [
            ("", "", true),
            ("", "a", false),
            ("a", "", false),
            ("*", "", true),
            ("**", "", true),
            ("a*", "a", true),
            ("a*b?", "axybz", true),
            ("a*b?", "axyb", false),
            ("build", "rebuild", false),
            ("build", "build-extra", false),
            ("*a*b", "aaab", true),
            ("*a*b", "aaabc", false),
        ] {
            assert_eq!(
                glob_segment_matches(pattern, value),
                expected,
                "{pattern:?} / {value:?}"
            );
        }
    }

    #[test]
    fn question_marks_match_bytes_and_stars_can_consume_utf8() {
        assert!(!glob_segment_matches("?", "é"));
        assert!(glob_segment_matches("??", "é"));
        assert!(glob_segment_matches("*", "é"));
        assert!(glob_segment_matches("é?", "éx"));
    }
}
