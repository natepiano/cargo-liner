use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use crate::config::DiagnosticCode;
use crate::reporting::Report;

pub(crate) struct UnusedPubScan {
    pub fixes: Vec<UseFix>,
}

pub(crate) fn scan_from_report(report: &Report) -> Result<UnusedPubScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::UnusedPub {
            continue;
        }
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
        let column_offset = finding.column.saturating_sub(1);
        if column_offset > line_text.len() {
            continue;
        }
        let Some(visibility_len) = bare_pub_annotation_byte_len(&line_text[column_offset..]) else {
            continue;
        };
        let trailing_whitespace_len = line_text[column_offset + visibility_len..]
            .chars()
            .take_while(|c| c.is_whitespace() && *c != '\n')
            .map(char::len_utf8)
            .sum::<usize>();
        let absolute_start = line_start + column_offset;
        fixes.push(UseFix {
            path:         absolute_path,
            start:        absolute_start,
            end:          absolute_start + visibility_len + trailing_whitespace_len,
            replacement:  String::new(),
            import_group: None,
        });
    }
    Ok(UnusedPubScan { fixes })
}

fn bare_pub_annotation_byte_len(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("pub")?;
    match rest.chars().next() {
        Some(c) if c.is_whitespace() => Some("pub".len()),
        None => Some("pub".len()),
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
    use super::bare_pub_annotation_byte_len;

    #[test]
    fn accepts_bare_pub() {
        assert_eq!(bare_pub_annotation_byte_len("pub fn helper() {}"), Some(3));
    }

    #[test]
    fn rejects_restricted_pub() {
        assert_eq!(
            bare_pub_annotation_byte_len("pub(crate) fn helper() {}"),
            None
        );
        assert_eq!(
            bare_pub_annotation_byte_len("pub(super) fn helper() {}"),
            None
        );
    }
}
