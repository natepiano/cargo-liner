use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::config::DiagnosticCode;
use super::constants::PUB_VISIBILITY_TOKEN;
use super::diagnostics::Report;
use super::imports::UseFix;

pub(crate) struct FieldVisibilityFixScan {
    pub fixes: Vec<UseFix>,
}

pub(crate) fn scan_from_report(report: &Report) -> Result<FieldVisibilityFixScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.code != DiagnosticCode::FieldVisibilityWiderThanType {
            continue;
        }
        let Some(replacement_vis) =
            parse_replacement_from_suggestion(finding.suggestion.as_deref())
        else {
            continue;
        };
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(line_start) = line_byte_offset(&source, finding.line) else {
            continue;
        };
        let line_end = source[line_start..]
            .find('\n')
            .map_or(source.len(), |pos| line_start + pos);
        let line_text = &source[line_start..line_end];
        // finding.column is 1-based; the field's `pub` annotation begins there.
        let column_offset = finding.column.saturating_sub(1);
        if column_offset > line_text.len() {
            continue;
        }
        let Some(vis_len) = vis_annotation_byte_len(&line_text[column_offset..]) else {
            continue;
        };
        // Include the single space (or whitespace run) following the annotation
        // so the replacement collapses cleanly when the new vis is empty.
        let trailing_ws_len = line_text[column_offset + vis_len..]
            .chars()
            .take_while(|c| c.is_whitespace() && *c != '\n')
            .map(char::len_utf8)
            .sum::<usize>();
        let absolute_start = line_start + column_offset;
        let absolute_end = absolute_start + vis_len + trailing_ws_len;
        let replacement_text = if replacement_vis.is_empty() {
            String::new()
        } else {
            format!("{replacement_vis} ")
        };
        fixes.push(UseFix {
            path:         absolute_path,
            start:        absolute_start,
            end:          absolute_end,
            replacement:  replacement_text,
            import_group: None,
        });
    }
    Ok(FieldVisibilityFixScan { fixes })
}

/// Parse the suggestion text emitted by the `field_visibility_wider_than_type`
/// lint. Returns the new visibility annotation (empty string when the
/// suggestion is to remove the annotation entirely).
fn parse_replacement_from_suggestion(suggestion: Option<&str>) -> Option<String> {
    let text = suggestion?;
    if text == "remove the field's visibility annotation" {
        return Some(String::new());
    }
    let prefix = "consider using: `";
    let rest = text.strip_prefix(prefix)?;
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

/// Return the byte length of a `pub`, `pub(crate)`, `pub(super)`, `pub(self)`,
/// or `pub(in <path>)` annotation at the start of `text`. Returns None when
/// `text` does not begin with `pub`.
fn vis_annotation_byte_len(text: &str) -> Option<usize> {
    let rest = text.strip_prefix(PUB_VISIBILITY_TOKEN)?;
    let mut chars = rest.char_indices();
    match chars.next() {
        Some((_, '(')) => {
            // Walk to the matching `)`. `pub(in crate::foo)` may contain
            // colons and identifiers but never nested parens.
            let close = rest.find(')')?;
            Some(PUB_VISIBILITY_TOKEN.len() + close + 1)
        },
        Some((_, c)) if c.is_whitespace() || c == ':' => Some(PUB_VISIBILITY_TOKEN.len()),
        None => Some(PUB_VISIBILITY_TOKEN.len()),
        _ => None,
    }
}

fn line_byte_offset(source: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    source
        .match_indices('\n')
        .nth(line - 2)
        .map(|(pos, _)| pos + 1)
}

#[cfg(test)]
mod tests {
    use super::parse_replacement_from_suggestion;
    use super::vis_annotation_byte_len;

    #[test]
    fn parses_consider_using_with_pub_crate() {
        assert_eq!(
            parse_replacement_from_suggestion(Some("consider using: `pub(crate)`")),
            Some("pub(crate)".to_string())
        );
    }

    #[test]
    fn parses_consider_using_with_pub_super() {
        assert_eq!(
            parse_replacement_from_suggestion(Some("consider using: `pub(super)`")),
            Some("pub(super)".to_string())
        );
    }

    #[test]
    fn parses_remove_annotation() {
        assert_eq!(
            parse_replacement_from_suggestion(Some("remove the field's visibility annotation")),
            Some(String::new())
        );
    }

    #[test]
    fn rejects_other_suggestion_text() {
        assert_eq!(
            parse_replacement_from_suggestion(Some("something else")),
            None
        );
        assert_eq!(parse_replacement_from_suggestion(None), None);
    }

    #[test]
    fn vis_len_bare_pub() {
        assert_eq!(vis_annotation_byte_len("pub leaked: u32"), Some(3));
    }

    #[test]
    fn vis_len_pub_crate() {
        assert_eq!(vis_annotation_byte_len("pub(crate) x"), Some(10));
    }

    #[test]
    fn vis_len_pub_super() {
        assert_eq!(vis_annotation_byte_len("pub(super) name"), Some(10));
    }

    #[test]
    fn vis_len_pub_in_path() {
        assert_eq!(
            vis_annotation_byte_len("pub(in crate::foo::bar) y"),
            Some("pub(in crate::foo::bar)".len())
        );
    }

    #[test]
    fn vis_len_no_pub() {
        assert_eq!(vis_annotation_byte_len("private_field: u32"), None);
    }
}
