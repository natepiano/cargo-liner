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

pub(super) struct FieldVisibilityFixScan {
    pub fixes: Vec<UseFix>,
}

#[derive(Debug, PartialEq, Eq)]
enum FieldVisibilityReplacement {
    Remove,
    Replace { annotation: String },
}

impl FieldVisibilityReplacement {
    fn into_source(self) -> String {
        match self {
            Self::Remove => String::new(),
            Self::Replace { annotation } => format!("{annotation} "),
        }
    }
}

pub(super) fn scan_from_report(report: &Report) -> Result<FieldVisibilityFixScan> {
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
        // `VisibilityAnnotationSite::locate` locates the field annotation for an
        // in-place rewrite, so both a bare `pub` and a restricted one are in scope
        // here. The edit covers the whitespace
        // run after the annotation so the replacement collapses cleanly when
        // the new visibility is empty.
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        fixes.push(UseFix {
            path:         absolute_path,
            start:        site.start,
            end:          site.end_with_separator,
            replacement:  replacement_visibility.into_source(),
            import_group: None,
        });
    }
    Ok(FieldVisibilityFixScan { fixes })
}

/// Parse the suggestion text emitted by the `field_visibility_wider_than_type`
/// lint, keeping removal distinct from an annotation replacement.
fn parse_replacement_from_suggestion(
    suggestion: Option<&str>,
) -> Option<FieldVisibilityReplacement> {
    let text = suggestion?;
    if text == RUSTC_FIELD_VIS_REMOVE_SUGGESTION {
        return Some(FieldVisibilityReplacement::Remove);
    }
    let rest = text.strip_prefix(RUSTC_LINT_SUGGESTION_PREFIX)?;
    let end = rest.find('`')?;
    Some(if end == 0 {
        FieldVisibilityReplacement::Remove
    } else {
        FieldVisibilityReplacement::Replace {
            annotation: rest[..end].to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::FieldVisibilityReplacement;
    use super::RUSTC_FIELD_VIS_REMOVE_SUGGESTION;
    use super::RUSTC_LINT_SUGGESTION_PREFIX;
    use super::parse_replacement_from_suggestion;

    #[test]
    fn parses_consider_using_with_pub_crate() {
        assert_eq!(
            parse_replacement_from_suggestion(Some(&format!(
                "{RUSTC_LINT_SUGGESTION_PREFIX}pub(crate)`"
            ))),
            Some(FieldVisibilityReplacement::Replace {
                annotation: "pub(crate)".to_string(),
            })
        );
    }

    #[test]
    fn parses_consider_using_with_pub_super() {
        assert_eq!(
            parse_replacement_from_suggestion(Some(&format!(
                "{RUSTC_LINT_SUGGESTION_PREFIX}pub(super)`"
            ))),
            Some(FieldVisibilityReplacement::Replace {
                annotation: "pub(super)".to_string(),
            })
        );
    }

    #[test]
    fn parses_remove_annotation() {
        assert_eq!(
            parse_replacement_from_suggestion(Some(RUSTC_FIELD_VIS_REMOVE_SUGGESTION)),
            Some(FieldVisibilityReplacement::Remove)
        );
    }

    #[test]
    fn empty_annotation_suggestion_removes_visibility_without_a_separator() {
        let suggestion = format!("{RUSTC_LINT_SUGGESTION_PREFIX}`");
        let replacement = parse_replacement_from_suggestion(Some(&suggestion));
        assert_eq!(replacement, Some(FieldVisibilityReplacement::Remove));
        assert_eq!(
            replacement.map(FieldVisibilityReplacement::into_source),
            Some(String::new())
        );
    }

    #[test]
    fn replacement_source_preserves_annotation_and_one_separator() {
        let replacement = FieldVisibilityReplacement::Replace {
            annotation: "pub(super)".to_string(),
        };
        assert_eq!(replacement.into_source(), "pub(super) ");
        assert_eq!(FieldVisibilityReplacement::Remove.into_source(), "");
    }

    #[test]
    fn rejects_other_suggestion_text() {
        let unterminated = format!("{RUSTC_LINT_SUGGESTION_PREFIX}pub(crate)");
        assert_eq!(parse_replacement_from_suggestion(Some(&unterminated)), None);
        assert_eq!(
            parse_replacement_from_suggestion(Some("something else")),
            None
        );
        assert_eq!(parse_replacement_from_suggestion(None), None);
    }
}
