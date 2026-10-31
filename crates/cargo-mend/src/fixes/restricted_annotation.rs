use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use super::visibility_annotation_site::VisibilityAnnotationForm;
use super::visibility_annotation_site::VisibilityAnnotationSite;
use crate::reporting::NarrowerScope;
use crate::reporting::Report;
use crate::reporting::WrittenVisibility;

pub(super) struct RestrictedAnnotationScan {
    pub fixes: Vec<UseFix>,
}

/// Whether the resolved boundary removes the annotation or gives it a new spelling.
enum VisibilityReplacement {
    Remove,
    Replace { annotation: String },
}

impl VisibilityReplacement {
    fn is_noop(&self, source: &str, site: VisibilityAnnotationSite) -> bool {
        match self {
            Self::Remove => false,
            Self::Replace { annotation } => {
                source.get(site.start..site.end) == Some(annotation.as_str())
            },
        }
    }

    fn into_fix(self, path: PathBuf, site: VisibilityAnnotationSite) -> UseFix {
        let (end, replacement) = match self {
            Self::Remove => (site.end_with_separator, String::new()),
            Self::Replace { annotation } => (site.end, annotation),
        };
        UseFix {
            path,
            start: site.start,
            end,
            replacement,
            import_group: None,
        }
    }
}

/// Rewrites a visibility annotation to the boundary the compiler pass resolved
/// for it.
///
/// The replacement is built from an exact [`NarrowerScope`] variant — never
/// from `Finding::suggestion`, which is rendered advice. These variants assert
/// that the effective scope is exact and that no facade line needs editing
/// alongside it; every other finding is left alone.
pub(super) fn scan_from_report(report: &Report) -> Result<RestrictedAnnotationScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    let mut rewritten_sites: BTreeSet<String> = BTreeSet::new();
    for finding in &report.findings {
        let replacement = match &finding.item_visibility.narrower_scope {
            NarrowerScope::ExactBoundary(def_path) => VisibilityReplacement::Replace {
                annotation: format!("pub(in crate::{def_path})"),
            },
            NarrowerScope::ExactParentBoundary(_) => VisibilityReplacement::Replace {
                annotation: String::from("pub(super)"),
            },
            NarrowerScope::CrateBoundary => VisibilityReplacement::Replace {
                annotation: String::from("pub(crate)"),
            },
            NarrowerScope::Private => VisibilityReplacement::Remove,
            NarrowerScope::PublicBoundary => VisibilityReplacement::Replace {
                annotation: String::from("pub"),
            },
            NarrowerScope::Unproposed | NarrowerScope::SuppressionKey(_) => continue,
        };
        // Whatever the annotation spells, `scan_from_report` plans its rewrite.
        // `visibility_constraint::apply_rewrite` is what decides which findings
        // arrive here carrying a boundary; this pass only locates the bytes and
        // confirms they still say what the report said. It once refused any
        // restricted spelling but `pub(crate)` and `pub(in crate)`, which left
        // every `pub(in crate::a)` narrowing suggested and never written.
        let (expected_form, expected_annotation) = match &finding.item_visibility.written {
            WrittenVisibility::Bare => (VisibilityAnnotationForm::Bare, "pub"),
            WrittenVisibility::Restricted(source) => {
                (VisibilityAnnotationForm::Restricted, source.as_str())
            },
            WrittenVisibility::Unknown => continue,
        };
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        // `annotation_agreement` guards against source changes since the
        // report was written. Only edit the exact annotation the compiler pass classified.
        if annotation_agreement(&source, site, expected_form, expected_annotation)
            == AnnotationAgreement::Changed
        {
            continue;
        }
        // `scan_from_report` rejects `replacement` when it writes back the bytes it
        // replaces: that is not a fix. It would be counted as applied, leave the
        // file untouched, and the finding would
        // return unchanged on the next run — a `--fix-all` that never converges.
        if replacement.is_noop(&source, site) {
            continue;
        }
        // `rewritten_sites` limits `scan_from_report` to one rewrite per declaration
        // site. The same item is reported once per compiled target, and
        // `suspicious_pub` words its message differently
        // for a binary than for a library, so both findings survive dedup and
        // would otherwise queue two edits over the same bytes — which fails
        // validation for the whole `--fix` batch. The file path pairs with the
        // byte offset rather than the item's def-path because `def_path_str`
        // omits the crate name, so two workspace members declaring the same
        // path would collide and silently lose one edit.
        if !rewritten_sites.insert(format!("{}:{}", finding.path, site.start)) {
            continue;
        }
        fixes.push(replacement.into_fix(absolute_path, site));
    }
    Ok(RestrictedAnnotationScan { fixes })
}

/// Guards against source changes since the report: edit only the exact annotation classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AnnotationAgreement {
    Matches,
    Changed,
}

fn annotation_agreement(
    source: &str,
    site: VisibilityAnnotationSite,
    expected_form: VisibilityAnnotationForm,
    expected_annotation: &str,
) -> AnnotationAgreement {
    if site.form == expected_form && source.get(site.start..site.end) == Some(expected_annotation) {
        AnnotationAgreement::Matches
    } else {
        AnnotationAgreement::Changed
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::AnnotationAgreement;
    use super::VisibilityAnnotationForm;
    use super::VisibilityAnnotationSite;
    use super::VisibilityReplacement;
    use super::annotation_agreement;

    #[test]
    fn restricted_rewrite_requires_the_annotation_captured_in_the_report() {
        let source = "pub(super) fn item() {}\n";
        let matches = VisibilityAnnotationSite::locate(source, 1, 1).map(|site| {
            annotation_agreement(
                source,
                site,
                VisibilityAnnotationForm::Restricted,
                "pub(crate)",
            )
        });

        assert_eq!(matches, Some(AnnotationAgreement::Changed));
    }

    #[test]
    fn removing_visibility_consumes_the_separator() {
        let source = "pub(crate)   fn item() {}\n";
        let result = VisibilityAnnotationSite::locate(source, 1, 1).map(|site| {
            let replacement = VisibilityReplacement::Remove;
            assert!(!replacement.is_noop(source, site));
            let fix = replacement.into_fix(PathBuf::from("src/lib.rs"), site);
            format!(
                "{}{}{}",
                &source[..fix.start],
                fix.replacement,
                &source[fix.end..]
            )
        });

        assert_eq!(result.as_deref(), Some("fn item() {}\n"));
    }

    #[test]
    fn replacing_visibility_preserves_the_separator_and_rejects_noops() {
        let source = "pub(crate)   fn item() {}\n";
        let result = VisibilityAnnotationSite::locate(source, 1, 1).map(|site| {
            assert!(
                VisibilityReplacement::Replace {
                    annotation: String::from("pub(crate)"),
                }
                .is_noop(source, site)
            );
            let replacement = VisibilityReplacement::Replace {
                annotation: String::from("pub(super)"),
            };
            assert!(!replacement.is_noop(source, site));
            let fix = replacement.into_fix(PathBuf::from("src/lib.rs"), site);
            format!(
                "{}{}{}",
                &source[..fix.start],
                fix.replacement,
                &source[fix.end..]
            )
        });

        assert_eq!(result.as_deref(), Some("pub(super)   fn item() {}\n"));
    }
    #[test]
    fn annotation_agreement_requires_both_form_and_current_bytes() {
        for (source, form, expected_text, expected) in [
            (
                "pub(super) fn item() {}",
                VisibilityAnnotationForm::Restricted,
                "pub(super)",
                AnnotationAgreement::Matches,
            ),
            (
                "pub(super) fn item() {}",
                VisibilityAnnotationForm::Restricted,
                "pub(crate)",
                AnnotationAgreement::Changed,
            ),
            (
                "pub fn item() {}",
                VisibilityAnnotationForm::Bare,
                "pub",
                AnnotationAgreement::Matches,
            ),
            (
                "pub fn item() {}",
                VisibilityAnnotationForm::Restricted,
                "pub",
                AnnotationAgreement::Changed,
            ),
        ] {
            let actual = VisibilityAnnotationSite::locate(source, 1, 1)
                .map(|site| annotation_agreement(source, site, form, expected_text));
            assert_eq!(actual, Some(expected));
        }
    }
}
