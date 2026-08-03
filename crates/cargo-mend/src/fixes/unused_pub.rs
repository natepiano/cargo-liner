use std::fs;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use super::visibility_annotation_site::VisibilityAnnotationForm;
use super::visibility_annotation_site::VisibilityAnnotationSite;
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
        let Some(site) = VisibilityAnnotationSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        // This fix deletes the annotation outright, so it applies to a bare
        // `pub` only. Deleting a `pub(crate)` or `pub(in ...)` would throw away
        // a narrower visibility the author wrote deliberately.
        if site.form != VisibilityAnnotationForm::Bare {
            continue;
        }
        fixes.push(UseFix {
            path:         absolute_path,
            start:        site.start,
            end:          site.end_with_separator,
            replacement:  String::new(),
            import_group: None,
        });
    }
    Ok(UnusedPubScan { fixes })
}
