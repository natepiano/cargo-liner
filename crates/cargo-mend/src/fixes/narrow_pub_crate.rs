use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::imports::UseFix;
use super::visibility_annotation_site::VisibilityAnnotationForm;
use super::visibility_annotation_site::VisibilityAnnotationSite;
use crate::config::DiagnosticCode;
use crate::reporting::Report;

pub(super) struct NarrowPubCrateScan {
    pub fixes: Vec<UseFix>,
}

/// A bare `pub` annotation eligible for narrowing to `pub(crate)`.
/// Restricted annotations are already at most crate-wide; rewriting them
/// here could be a no-op or widen their visibility.
struct PubCrateNarrowingSite {
    start: usize,
    end:   usize,
}

impl PubCrateNarrowingSite {
    fn locate(source: &str, line: usize, column: usize) -> Option<Self> {
        let site = VisibilityAnnotationSite::locate(source, line, column)?;
        match site.form {
            VisibilityAnnotationForm::Bare => Some(Self {
                start: site.start,
                end:   site.end,
            }),
            VisibilityAnnotationForm::Restricted => None,
        }
    }

    fn into_fix(self, path: PathBuf) -> UseFix {
        UseFix {
            path,
            start: self.start,
            end: self.end,
            replacement: "pub(crate)".to_string(),
            import_group: None,
        }
    }
}

pub(super) fn scan_from_report(report: &Report) -> Result<NarrowPubCrateScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::NarrowToPubCrate {
            continue;
        }
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(site) = PubCrateNarrowingSite::locate(&source, finding.line, finding.column)
        else {
            continue;
        };
        fixes.push(site.into_fix(absolute_path));
    }
    Ok(NarrowPubCrateScan { fixes })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::PubCrateNarrowingSite;
    use crate::fixes::constants::TAB_DISPLAY_WIDTH;

    #[test]
    fn narrowing_accepts_only_bare_visibility_and_preserves_the_separator() {
        let source = "\tpub   fn item() {}\n";
        let result = PubCrateNarrowingSite::locate(source, 1, TAB_DISPLAY_WIDTH + 1).map(|site| {
            let fix = site.into_fix(PathBuf::from("src/lib.rs"));
            format!(
                "{}{}{}",
                &source[..fix.start],
                fix.replacement,
                &source[fix.end..]
            )
        });
        assert_eq!(result.as_deref(), Some("\tpub(crate)   fn item() {}\n"));
        for source in [
            "pub(crate) fn item() {}",
            "pub(super) fn item() {}",
            "pub(in crate::a) fn item() {}",
            "pub(\n    crate) fn item() {}",
            "fn item() {}",
        ] {
            assert!(PubCrateNarrowingSite::locate(source, 1, 1).is_none());
        }
    }
}
