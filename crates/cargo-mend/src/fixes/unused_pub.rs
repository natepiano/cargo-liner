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

pub(super) struct UnusedPubScan {
    pub fixes: Vec<UseFix>,
}

/// A bare `pub` annotation whose removal includes its trailing separator.
/// Restricted annotations are excluded because deleting them would discard
/// a narrower visibility the author deliberately wrote.
struct UnusedPubRemovalSite {
    start: usize,
    end:   usize,
}

impl UnusedPubRemovalSite {
    fn locate(source: &str, line: usize, column: usize) -> Option<Self> {
        let site = VisibilityAnnotationSite::locate(source, line, column)?;
        match site.form {
            VisibilityAnnotationForm::Bare => Some(Self {
                start: site.start,
                end:   site.end_with_separator,
            }),
            VisibilityAnnotationForm::Restricted => None,
        }
    }

    const fn into_fix(self, path: PathBuf) -> UseFix {
        UseFix {
            path,
            start: self.start,
            end: self.end,
            replacement: String::new(),
            import_group: None,
        }
    }
}

pub(super) fn scan_from_report(report: &Report) -> Result<UnusedPubScan> {
    let root = Path::new(&report.root);
    let mut fixes = Vec::new();
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::UnusedPub {
            continue;
        }
        let absolute_path = root.join(&finding.path);
        let source = fs::read_to_string(&absolute_path)
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        let Some(site) = UnusedPubRemovalSite::locate(&source, finding.line, finding.column) else {
            continue;
        };
        fixes.push(site.into_fix(absolute_path));
    }
    Ok(UnusedPubScan { fixes })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::UnusedPubRemovalSite;

    #[test]
    fn removal_accepts_only_bare_visibility_and_consumes_the_separator() {
        let source = "    pub \t  fn item() {}\n";
        let result = UnusedPubRemovalSite::locate(source, 1, 5).map(|site| {
            let fix = site.into_fix(PathBuf::from("src/lib.rs"));
            format!(
                "{}{}{}",
                &source[..fix.start],
                fix.replacement,
                &source[fix.end..]
            )
        });
        assert_eq!(result.as_deref(), Some("    fn item() {}\n"));
        for source in [
            "pub(crate) fn item() {}",
            "pub(super) fn item() {}",
            "pub(in crate::a) fn item() {}",
            "pub(\n    crate) fn item() {}",
            "fn item() {}",
        ] {
            assert!(UnusedPubRemovalSite::locate(source, 1, 1).is_none());
        }
    }
}
