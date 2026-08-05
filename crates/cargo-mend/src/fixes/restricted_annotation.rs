use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use super::visibility_annotation_site::VisibilityAnnotationForm;
use super::visibility_annotation_site::VisibilityAnnotationSite;
use crate::reporting::NarrowerScope;
use crate::reporting::Report;
use crate::reporting::WrittenVisibility;

pub(crate) struct RestrictedAnnotationScan {
    pub fixes: Vec<UseFix>,
}

/// Rewrites a bare `pub` or an eligible `pub(crate)` to the exact boundary the
/// compiler pass resolved for it.
///
/// The replacement is built from an exact [`NarrowerScope`] variant — never
/// from `Finding::suggestion`, which is rendered advice. These variants assert
/// that the effective scope is exact and that no facade line needs editing
/// alongside it; every other finding is left alone.
pub(crate) fn scan_from_report(report: &Report) -> Result<RestrictedAnnotationScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    let mut rewritten_sites: BTreeSet<String> = BTreeSet::new();
    for finding in &report.findings {
        let replacement = match &finding.item_visibility.narrower_scope {
            NarrowerScope::ExactBoundary(def_path) => format!("pub(in crate::{def_path})"),
            NarrowerScope::ExactParentBoundary(_) => String::from("pub(super)"),
            NarrowerScope::CrateBoundary => String::from("pub(crate)"),
            NarrowerScope::Private => String::new(),
            NarrowerScope::PublicBoundary => String::from("pub"),
            NarrowerScope::Unproposed | NarrowerScope::SuppressionKey(_) => continue,
        };
        let (expected_form, expected_annotation) = match &finding.item_visibility.written {
            WrittenVisibility::Bare => (VisibilityAnnotationForm::Bare, "pub"),
            WrittenVisibility::Restricted(source)
                if source == "pub(crate)" || source == "pub(in crate)" =>
            {
                (VisibilityAnnotationForm::Restricted, source.as_str())
            },
            WrittenVisibility::Unknown | WrittenVisibility::Restricted(_) => continue,
        };
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        // The source may have moved on since the report was written. Only edit
        // the exact annotation the compiler pass classified.
        if !matches_expected_annotation(&source, site, expected_form, expected_annotation) {
            continue;
        }
        // One rewrite per declaration site. The same item is reported once per
        // compiled target, and `suspicious_pub` words its message differently
        // for a binary than for a library, so both findings survive dedup and
        // would otherwise queue two edits over the same bytes — which fails
        // validation for the whole `--fix` batch. The file path pairs with the
        // byte offset rather than the item's def-path because `def_path_str`
        // omits the crate name, so two workspace members declaring the same
        // path would collide and silently lose one edit.
        if !rewritten_sites.insert(format!("{}:{}", finding.path, site.start)) {
            continue;
        }
        fixes.push(UseFix {
            path: absolute_path,
            start: site.start,
            end: if replacement.is_empty() {
                site.end_with_separator
            } else {
                site.end
            },
            replacement,
            import_group: None,
        });
    }
    Ok(RestrictedAnnotationScan { fixes })
}

fn matches_expected_annotation(
    source: &str,
    site: VisibilityAnnotationSite,
    expected_form: VisibilityAnnotationForm,
    expected_annotation: &str,
) -> bool {
    site.form == expected_form && source.get(site.start..site.end) == Some(expected_annotation)
}

#[cfg(test)]
mod tests {
    use super::VisibilityAnnotationForm;
    use super::VisibilityAnnotationSite;
    use super::matches_expected_annotation;

    #[test]
    fn restricted_rewrite_requires_the_annotation_captured_in_the_report() {
        let source = "pub(super) fn item() {}\n";
        let matches = VisibilityAnnotationSite::locate(source, 1, 1).map(|site| {
            matches_expected_annotation(
                source,
                site,
                VisibilityAnnotationForm::Restricted,
                "pub(crate)",
            )
        });

        assert_eq!(matches, Some(false));
    }
}
