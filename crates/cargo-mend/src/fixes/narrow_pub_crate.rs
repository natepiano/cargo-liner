use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use super::visibility_annotation_site::VisibilityAnnotationForm;
use super::visibility_annotation_site::VisibilityAnnotationSite;
use crate::config::DiagnosticCode;
use crate::reporting::Report;

pub(crate) struct NarrowPubCrateScan {
    pub fixes: Vec<UseFix>,
}

pub(crate) fn scan_from_report(report: &Report) -> Result<NarrowPubCrateScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::NarrowToPubCrate {
            continue;
        }
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        // Only a bare `pub` is widened past the crate. A restricted annotation
        // is already at most `pub(crate)`, so rewriting it would be a no-op at
        // best and a widening at worst.
        if site.form != VisibilityAnnotationForm::Bare {
            continue;
        }
        fixes.push(UseFix {
            path:         absolute_path,
            start:        site.start,
            end:          site.end,
            replacement:  "pub(crate)".to_string(),
            import_group: None,
        });
    }
    Ok(NarrowPubCrateScan { fixes })
}
