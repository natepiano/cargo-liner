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

/// Rewrites a bare `pub` to the exact `pub(in crate::…)` boundary the compiler
/// pass resolved for it.
///
/// The replacement is built from [`NarrowerScope::ExactBoundary`], a def-path
/// the pass computed — never from `Finding::suggestion`, which is rendered
/// advice. That variant is the pass's assertion that the scope is an exact
/// module boundary and that no facade line needs editing alongside it; every
/// other finding is left alone.
pub(crate) fn scan_from_report(report: &Report) -> Result<RestrictedAnnotationScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    let mut rewritten_sites: BTreeSet<String> = BTreeSet::new();
    for finding in &report.findings {
        let NarrowerScope::ExactBoundary(narrower_scope_def_path) =
            &finding.item_visibility.narrower_scope
        else {
            continue;
        };
        // A restricted annotation is already narrower than `pub`; replacing it
        // is the stale-boundary repair, which also has a facade to move and so
        // stays manual.
        if finding.item_visibility.written != WrittenVisibility::Bare {
            continue;
        }
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        // The source may have moved on since the report was written. Only edit
        // what still reads as the bare `pub` the pass classified.
        if site.form != VisibilityAnnotationForm::Bare {
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
            path:         absolute_path,
            start:        site.start,
            end:          site.end,
            replacement:  format!("pub(in crate::{narrower_scope_def_path})"),
            import_group: None,
        });
    }
    Ok(RestrictedAnnotationScan { fixes })
}
