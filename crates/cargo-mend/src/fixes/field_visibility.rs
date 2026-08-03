use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::constants::RUSTC_FIELD_VIS_REMOVE_SUGGESTION;
use super::constants::RUSTC_LINT_SUGGESTION_PREFIX;
use super::imports::UseFix;
use super::visibility_annotation_site::VisibilityAnnotationSite;
use crate::config::DiagnosticCode;
use crate::reporting::Report;

pub(crate) struct FieldVisibilityFixScan {
    pub fixes: Vec<UseFix>,
}

pub(crate) fn scan_from_report(report: &Report) -> Result<FieldVisibilityFixScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::FieldVisibilityWiderThanType {
            continue;
        }
        let Some(replacement_visibility) =
            parse_replacement_from_suggestion(finding.suggestion.as_deref())
        else {
            continue;
        };
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        // A field's annotation is rewritten in place, so both a bare `pub` and
        // a restricted one are in scope here. The edit covers the whitespace
        // run after the annotation so the replacement collapses cleanly when
        // the new visibility is empty.
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        let replacement_text = if replacement_visibility.is_empty() {
            String::new()
        } else {
            format!("{replacement_visibility} ")
        };
        fixes.push(UseFix {
            path:         absolute_path,
            start:        site.start,
            end:          site.end_with_separator,
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
    if text == RUSTC_FIELD_VIS_REMOVE_SUGGESTION {
        return Some(String::new());
    }
    let rest = text.strip_prefix(RUSTC_LINT_SUGGESTION_PREFIX)?;
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::RUSTC_FIELD_VIS_REMOVE_SUGGESTION;
    use super::RUSTC_LINT_SUGGESTION_PREFIX;
    use super::parse_replacement_from_suggestion;

    #[test]
    fn parses_consider_using_with_pub_crate() {
        assert_eq!(
            parse_replacement_from_suggestion(Some(&format!(
                "{RUSTC_LINT_SUGGESTION_PREFIX}pub(crate)`"
            ))),
            Some("pub(crate)".to_string())
        );
    }

    #[test]
    fn parses_consider_using_with_pub_super() {
        assert_eq!(
            parse_replacement_from_suggestion(Some(&format!(
                "{RUSTC_LINT_SUGGESTION_PREFIX}pub(super)`"
            ))),
            Some("pub(super)".to_string())
        );
    }

    #[test]
    fn parses_remove_annotation() {
        assert_eq!(
            parse_replacement_from_suggestion(Some(RUSTC_FIELD_VIS_REMOVE_SUGGESTION)),
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
}
