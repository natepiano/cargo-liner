use super::FixScans;
use super::MendRunner;
use crate::config::OperationIntent;
use crate::reporting::ExecutionNotice;
use crate::reporting::FixNotice;
use crate::reporting::NoticeKind;
use crate::reporting::PubUseNotice;
use crate::reporting::Report;

impl FixScans<'_> {
    fn import_fix_notice_count(self) -> Option<usize> {
        [
            self.imports.map(|scan| scan.findings.len()),
            self.module_imports.map(|scan| scan.findings.len()),
            self.inline_types.map(|scan| scan.findings.len()),
            self.unused_pub.map(|scan| scan.fixes.len()),
            self.narrowed_pub.map(|scan| scan.fixes.len()),
            self.field_visibility.map(|scan| scan.fixes.len()),
            self.imports_at_top.map(|scan| scan.findings.len()),
        ]
        .into_iter()
        .flatten()
        .reduce(|total, count| total + count)
    }
}

impl MendRunner<'_> {
    pub(super) fn build_fix_notice(
        intent: OperationIntent,
        report: Option<&Report>,
        fix_scans: FixScans<'_>,
    ) -> Option<ExecutionNotice> {
        let mut notices = Vec::new();
        if let Some(import_fix_count) = fix_scans.import_fix_notice_count() {
            notices.push(NoticeKind::ImportFixes(FixNotice::from_intent(
                intent,
                import_fix_count,
            )));
        }

        if let Some(scan) = fix_scans.pub_use {
            notices.push(NoticeKind::PubUseFixes(PubUseNotice::from_intent(
                intent,
                scan.applied,
                scan.skipped,
            )));
        }

        // The historical `ImportCleanupSuggested` notice is gone; the
        // orchestrator runs `cargo fix` automatically when `--fix-pub-use`
        // applied edits and `unused import` warnings followed.
        let _ = report;

        match notices.len() {
            0 => None,
            1 => notices.into_iter().next().map(ExecutionNotice::from),
            _ => Some(ExecutionNotice::from(notices)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::FixScans;
    use super::MendRunner;
    use crate::config::OperationIntent;
    use crate::fixes::field_visibility::FieldVisibilityFixScan;
    use crate::fixes::imports::UseFix;

    fn fix_scans_with_field_visibility(field_visibility: &FieldVisibilityFixScan) -> FixScans<'_> {
        FixScans {
            imports:          None,
            module_imports:   None,
            inline_types:     None,
            unused_pub:       None,
            narrowed_pub:     None,
            field_visibility: Some(field_visibility),
            imports_at_top:   None,
            pub_use:          None,
        }
    }

    fn field_visibility_scan(fixes: Vec<UseFix>) -> FieldVisibilityFixScan {
        FieldVisibilityFixScan { fixes }
    }

    fn field_visibility_fix() -> UseFix {
        UseFix {
            path:         PathBuf::from("src/lib.rs"),
            start:        10,
            end:          10,
            replacement:  String::new(),
            import_group: None,
        }
    }

    #[test]
    fn field_visibility_scan_emits_import_fix_notice() {
        let field_visibility = field_visibility_scan(vec![field_visibility_fix()]);
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_field_visibility(&field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: applied 1 import fix(es)".to_string())
        );
    }

    #[test]
    fn empty_field_visibility_scan_emits_noop_import_fix_notice() {
        let field_visibility = field_visibility_scan(Vec::new());
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_field_visibility(&field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: no import fixes available".to_string())
        );
    }
}
