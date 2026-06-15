use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StatContext {
    Stats,
    NonCacheableReasons,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ParsedStatLine {
    Field {
        label: String,
        value: String,
    },
    Subheader {
        text:    String,
        context: StatContext,
    },
    Text(String),
}

pub(super) fn parse_stat_lines(raw_lines: &[String]) -> Vec<ParsedStatLine> {
    let mut context = StatContext::Stats;
    raw_lines
        .iter()
        .map(|line| parse_stat_line(line, &mut context))
        .collect()
}

fn parse_stat_line(text: &str, context: &mut StatContext) -> ParsedStatLine {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ParsedStatLine::Text(String::new());
    }
    if trimmed == "Non-cacheable reasons:" {
        return ParsedStatLine::Subheader {
            text:    trimmed.to_string(),
            context: StatContext::NonCacheableReasons,
        };
    }
    if let Some((label, value)) = split_aligned_stat(trimmed) {
        if label == "Cache location" {
            *context = StatContext::Stats;
        }
        let (label, value) = normalize_stat_field(label, value, *context);
        return ParsedStatLine::Field { label, value };
    }
    if trimmed.ends_with(':') {
        return ParsedStatLine::Subheader {
            text:    trimmed.to_string(),
            context: StatContext::Stats,
        };
    }
    ParsedStatLine::Text(trimmed.to_string())
}

fn normalize_stat_field(label: &str, value: &str, context: StatContext) -> (String, String) {
    if context == StatContext::NonCacheableReasons {
        return (normalize_reason_label(label), value.to_string());
    }
    if label == "Cache location" {
        return normalize_cache_location(value);
    }
    (label.to_string(), value.to_string())
}

fn normalize_reason_label(label: &str) -> String {
    match label {
        "-" => "(no reason reported)".to_string(),
        "-o" => "compiler flag -o".to_string(),
        _ => label.to_string(),
    }
}

fn normalize_cache_location(value: &str) -> (String, String) {
    const LOCAL_DISK_PREFIX: &str = "Local disk:";
    let value = value.trim();
    if let Some(path) = value.strip_prefix(LOCAL_DISK_PREFIX) {
        return (
            "Cache location".to_string(),
            path.trim().trim_matches('"').to_string(),
        );
    }
    ("Cache location".to_string(), value.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ValueAlignment {
    numeric_left_width: usize,
}

impl ValueAlignment {
    pub(super) fn for_lines(lines: &[ParsedStatLine]) -> Self {
        let numeric_left_width = lines
            .iter()
            .filter_map(|line| match line {
                ParsedStatLine::Field { value, .. } => NumericValue::parse(value),
                ParsedStatLine::Subheader { .. } | ParsedStatLine::Text(_) => None,
            })
            .map(NumericValue::left_width)
            .max()
            .unwrap_or(0);
        Self { numeric_left_width }
    }

    pub(super) fn format(self, value: &str) -> String {
        let Some(numeric) = NumericValue::parse(value) else {
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

impl<'a> NumericValue<'a> {
    fn parse(value: &'a str) -> Option<Self> {
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
            return None;
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
                return None;
            }
        }
        if !value[pos..].is_empty() && !value[pos..].starts_with(char::is_whitespace) {
            return None;
        }
        Some(Self {
            sign,
            integer: &value[int_start..int_end],
            rest: &value[int_end..],
        })
    }

    fn left_width(self) -> usize { self.sign.width() + self.integer.width() }
}

fn split_aligned_stat(text: &str) -> Option<(&str, &str)> {
    let mut gap_start = None;
    let mut gap_len = 0;
    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            gap_start.get_or_insert(idx);
            gap_len += 1;
            continue;
        }
        if gap_len >= 2 {
            let start = gap_start?;
            let label = text[..start].trim_end();
            let value = text[idx..].trim();
            if !label.is_empty() && !value.is_empty() {
                return Some((label, value));
            }
        }
        gap_start = None;
        gap_len = 0;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_stats(text: &str) -> ParsedStatLine { parse_stat_line(text, &mut StatContext::Stats) }

    fn parse_reason(text: &str) -> ParsedStatLine {
        parse_stat_line(text, &mut StatContext::NonCacheableReasons)
    }

    #[test]
    fn parse_stat_line_preserves_percent_values() {
        assert_eq!(
            parse_stats("Cache hits rate (Rust)        78.20 %"),
            ParsedStatLine::Field {
                label: "Cache hits rate (Rust)".to_string(),
                value: "78.20 %".to_string(),
            },
        );
    }

    #[test]
    fn parse_stat_line_uses_aligned_value_column() {
        assert_eq!(
            parse_stats("Compile requests              2090"),
            ParsedStatLine::Field {
                label: "Compile requests".to_string(),
                value: "2090".to_string(),
            },
        );
    }

    #[test]
    fn parse_stat_line_keeps_units_with_values() {
        assert_eq!(
            parse_stats("Average cache write           0.001 s"),
            ParsedStatLine::Field {
                label: "Average cache write".to_string(),
                value: "0.001 s".to_string(),
            },
        );
        assert_eq!(
            parse_stats("Cache size                    31 GiB"),
            ParsedStatLine::Field {
                label: "Cache size".to_string(),
                value: "31 GiB".to_string(),
            },
        );
    }

    #[test]
    fn parse_stat_line_keeps_location_kind_with_value() {
        assert_eq!(
            parse_stats(
                "Cache location                Local disk: \"/Users/natemccoy/Library/Caches/Mozilla.sccache\"",
            ),
            ParsedStatLine::Field {
                label: "Cache location".to_string(),
                value: "/Users/natemccoy/Library/Caches/Mozilla.sccache".to_string(),
            },
        );
    }

    #[test]
    fn parse_stat_line_treats_reason_header_as_subheader() {
        assert_eq!(
            parse_stats("Non-cacheable reasons:"),
            ParsedStatLine::Subheader {
                text:    "Non-cacheable reasons:".to_string(),
                context: StatContext::NonCacheableReasons,
            },
        );
    }

    #[test]
    fn parse_stat_line_explains_raw_reason_keys() {
        assert_eq!(
            parse_reason("-                                   297"),
            ParsedStatLine::Field {
                label: "(no reason reported)".to_string(),
                value: "297".to_string(),
            },
        );
        assert_eq!(
            parse_reason("-o                                    6"),
            ParsedStatLine::Field {
                label: "compiler flag -o".to_string(),
                value: "6".to_string(),
            },
        );
    }

    #[test]
    fn value_alignment_lines_up_numeric_columns() {
        let lines = vec![
            ParsedStatLine::Field {
                label: "Cache misses".to_string(),
                value: "184".to_string(),
            },
            ParsedStatLine::Field {
                label: "Cache hits rate".to_string(),
                value: "72.42 %".to_string(),
            },
            ParsedStatLine::Field {
                label: "Cache timeouts".to_string(),
                value: "0".to_string(),
            },
            ParsedStatLine::Field {
                label: "Compile requests".to_string(),
                value: "2531".to_string(),
            },
            ParsedStatLine::Field {
                label: "Average cache write".to_string(),
                value: "0.001 s".to_string(),
            },
        ];
        let alignment = ValueAlignment::for_lines(&lines);

        assert_eq!(alignment.format("184"), " 184");
        assert_eq!(alignment.format("72.42 %"), "  72.42 %");
        assert_eq!(alignment.format("0"), "   0");
        assert_eq!(alignment.format("2531"), "2531");
        assert_eq!(alignment.format("0.001 s"), "   0.001 s");
    }

    #[test]
    fn value_alignment_leaves_semver_unpadded() {
        let alignment = ValueAlignment {
            numeric_left_width: 4,
        };

        assert_eq!(alignment.format("0.14.0"), "0.14.0");
    }
}
