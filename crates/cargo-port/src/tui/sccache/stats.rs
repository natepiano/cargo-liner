pub(super) use tui_pane::NumericColumnAlignment as ValueAlignment;

use crate::sccache;

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
    if let Some((label, value)) = sccache::split_aligned_stat(trimmed) {
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

pub(super) fn value_alignment_for_lines(lines: &[ParsedStatLine]) -> ValueAlignment {
    ValueAlignment::from_values(lines.iter().filter_map(|line| match line {
        ParsedStatLine::Field { value, .. } => Some(value.as_str()),
        ParsedStatLine::Subheader { .. } | ParsedStatLine::Text(_) => None,
    }))
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
    fn value_alignment_for_lines_measures_only_fields() {
        let lines = [
            ParsedStatLine::Text("123456".to_string()),
            ParsedStatLine::Subheader {
                text:    "12345".to_string(),
                context: StatContext::Stats,
            },
            ParsedStatLine::Field {
                label: "Count".to_string(),
                value: "12".to_string(),
            },
        ];
        assert_eq!(value_alignment_for_lines(&lines).format("1"), " 1");
    }
}
