//! Column alignment for numeric text.

use unicode_width::UnicodeWidthStr;

/// Integer-side column alignment for signed numeric text with optional suffixes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NumericColumnAlignment {
    numeric_left_width: usize,
}

impl NumericColumnAlignment {
    /// Measure the widest sign and integer prefix among the supplied values.
    #[must_use]
    pub fn from_values<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let numeric_left_width = values
            .into_iter()
            .filter_map(|value| match NumericValue::parse(value) {
                ParsedNumericValue::Numeric(numeric) => Some(numeric),
                ParsedNumericValue::NonNumeric => None,
            })
            .map(NumericValue::left_width)
            .max()
            .unwrap_or(0);
        Self { numeric_left_width }
    }

    /// Pad a numeric prefix to the measured width, preserving its suffix.
    ///
    /// Numeric values are trimmed; nonnumeric values are returned unchanged.
    #[must_use]
    pub fn format(self, value: &str) -> String {
        let ParsedNumericValue::Numeric(numeric) = NumericValue::parse(value) else {
            return value.to_string();
        };
        let leading = self.numeric_left_width.saturating_sub(numeric.left_width());
        format!(
            "{}{}{}{}",
            " ".repeat(leading),
            numeric.sign,
            numeric.integer,
            numeric.rest
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NumericValue<'a> {
    sign:    &'a str,
    integer: &'a str,
    rest:    &'a str,
}

enum ParsedNumericValue<'a> {
    Numeric(NumericValue<'a>),
    NonNumeric,
}

impl<'a> NumericValue<'a> {
    fn parse(value: &'a str) -> ParsedNumericValue<'a> {
        let value = value.trim();
        let mut pos = 0;
        let sign = if value.starts_with('-') || value.starts_with('+') {
            pos = 1;
            &value[..1]
        } else {
            ""
        };
        let int_start = pos;
        while let Some(ch) = value[pos..].chars().next()
            && ch.is_ascii_digit()
        {
            pos += ch.len_utf8();
        }
        if pos == int_start {
            return ParsedNumericValue::NonNumeric;
        }
        let int_end = pos;
        if value[pos..].starts_with('.') {
            pos += 1;
            let frac_start = pos;
            while let Some(ch) = value[pos..].chars().next()
                && ch.is_ascii_digit()
            {
                pos += ch.len_utf8();
            }
            if pos == frac_start {
                return ParsedNumericValue::NonNumeric;
            }
        }
        if !value[pos..].is_empty() && !value[pos..].starts_with(char::is_whitespace) {
            return ParsedNumericValue::NonNumeric;
        }
        ParsedNumericValue::Numeric(Self {
            sign,
            integer: &value[int_start..int_end],
            rest: &value[int_end..],
        })
    }

    fn left_width(self) -> usize { self.sign.width() + self.integer.width() }
}

#[cfg(test)]
mod tests {
    use super::NumericColumnAlignment;

    #[test]
    fn value_alignment_lines_up_numeric_columns() {
        let alignment =
            NumericColumnAlignment::from_values(["184", "72.42 %", "0", "2531", "0.001 s"]);

        assert_eq!(alignment.format("184"), " 184");
        assert_eq!(alignment.format("72.42 %"), "  72.42 %");
        assert_eq!(alignment.format("0"), "   0");
        assert_eq!(alignment.format("2531"), "2531");
        assert_eq!(alignment.format("0.001 s"), "   0.001 s");
    }

    #[test]
    fn value_alignment_leaves_semver_unpadded() {
        let alignment = NumericColumnAlignment::from_values(["2531"]);

        assert_eq!(alignment.format("0.14.0"), "0.14.0");
    }

    #[test]
    fn signed_values_keep_fraction_and_unicode_unit_suffixes() {
        let alignment = NumericColumnAlignment::from_values(["-123", "+7.50 界", "2"]);
        assert_eq!(alignment.format("+7.50 界"), "  +7.50 界");
        assert_eq!(
            alignment.format("\u{2003}-2.50\t界 ms\u{2003}"),
            "  -2.50\t界 ms"
        );
        assert_eq!(alignment.format("12\u{2003}界"), "  12\u{2003}界");
        assert_eq!(alignment.format("12345"), "12345");
    }

    #[test]
    fn malformed_prefixes_and_text_keep_original_whitespace() {
        let alignment = NumericColumnAlignment::from_values(["1234"]);
        for value in [
            "", "-", "+", ".5", "++1", "--1", "1.", "1. s", "12ms", "1e3", "1,000", "１２",
            " text ", " 0.14.0 ",
        ] {
            assert_eq!(alignment.format(value), value);
        }
    }

    #[test]
    fn nonnumeric_values_do_not_enlarge_the_column() {
        let alignment = NumericColumnAlignment::from_values(["long text", "0.14.0", "1 界界界"]);
        assert_eq!(alignment.format("2"), "2");
        assert_eq!(NumericColumnAlignment::from_values([]).format(" 2 "), "2");
        assert_eq!(
            NumericColumnAlignment::from_values(["text"]).format(" 2 "),
            "2"
        );
    }
}
