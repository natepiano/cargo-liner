use super::FixScans;
use super::MendRunner;
use crate::config::OperationIntent;
use crate::reporting::ExecutionNotice;
use crate::reporting::FixKind;
use crate::reporting::FixNotice;
use crate::reporting::NoticeKind;
use crate::reporting::PubUseNotice;
use crate::reporting::Report;

impl FixScans<'_> {
    /// Only the fixers that move or rewrite a `use` item share one total. Each
    /// remaining fixer edits a visibility annotation, so it reports under its
    /// own `FixKind` rather than being counted as an import fix.
    fn notice_counts(self) -> [(FixKind, Option<usize>); 5] {
        [
            (FixKind::Import, self.import_fix_notice_count()),
            (
                FixKind::PubRemoval,
                self.unused_pub.map(|scan| scan.fixes.len()),
            ),
            (
                FixKind::Narrowing,
                self.narrowed_pub.map(|scan| scan.fixes.len()),
            ),
            (
                FixKind::Annotation,
                self.restricted_annotation.map(|scan| scan.fixes.len()),
            ),
            (
                FixKind::FieldVisibility,
                self.field_visibility.map(|scan| scan.fixes.len()),
            ),
        ]
    }

    fn import_fix_notice_count(self) -> Option<usize> {
        [
            self.imports.map(|scan| scan.findings.len()),
            self.module_imports.map(|scan| scan.findings.len()),
            self.inline_types.map(|scan| scan.findings.len()),
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
        let enabled = fix_scans
            .notice_counts()
            .into_iter()
            .filter_map(|(fix_kind, count)| count.map(|count| (fix_kind, count)))
            .collect::<Vec<_>>();

        // Name only the kinds that edited something, so a run that removes one
        // `pub` does not also announce four kinds it had no work for. When every
        // enabled kind sits at zero the first one still speaks, keeping a clean
        // run's single "nothing available" line instead of going silent.
        let mut reported = enabled
            .iter()
            .copied()
            .filter(|&(_, count)| count > 0)
            .collect::<Vec<_>>();
        if reported.is_empty() {
            reported.extend(enabled.first().copied());
        }

        let mut notices = reported
            .into_iter()
            .map(|(fix_kind, count)| {
                NoticeKind::Fixes(FixNotice::from_intent(intent, fix_kind, count))
            })
            .collect::<Vec<_>>();

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
    use crate::fixes::unused_pub::UnusedPubScan;

    fn fix_scans_with_field_visibility(field_visibility: &FieldVisibilityFixScan) -> FixScans<'_> {
        FixScans {
            imports:               None,
            module_imports:        None,
            inline_types:          None,
            unused_pub:            None,
            narrowed_pub:          None,
            restricted_annotation: None,
            field_visibility:      Some(field_visibility),
            imports_at_top:        None,
            pub_use:               None,
        }
    }

    fn fix_scans_with_unused_pub_and_field_visibility<'a>(
        unused_pub: &'a UnusedPubScan,
        field_visibility: &'a FieldVisibilityFixScan,
    ) -> FixScans<'a> {
        FixScans {
            unused_pub: Some(unused_pub),
            ..fix_scans_with_field_visibility(field_visibility)
        }
    }

    fn field_visibility_scan(fixes: Vec<UseFix>) -> FieldVisibilityFixScan {
        FieldVisibilityFixScan { fixes }
    }

    fn unused_pub_scan(fixes: Vec<UseFix>) -> UnusedPubScan { UnusedPubScan { fixes } }

    fn use_fix() -> UseFix {
        UseFix {
            path:         PathBuf::from("src/lib.rs"),
            start:        10,
            end:          10,
            replacement:  String::new(),
            import_group: None,
        }
    }

    #[test]
    fn field_visibility_scan_names_the_rewrite_rather_than_an_import_fix() {
        let field_visibility = field_visibility_scan(vec![use_fix()]);
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_field_visibility(&field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: applied 1 field visibility rewrite(s)".to_string())
        );
    }

    #[test]
    fn empty_field_visibility_scan_emits_noop_field_visibility_notice() {
        let field_visibility = field_visibility_scan(Vec::new());
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_field_visibility(&field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: no field visibility rewrites available".to_string())
        );
    }

    #[test]
    fn a_kind_that_applied_nothing_is_left_out_when_another_kind_applied_something() {
        let unused_pub = unused_pub_scan(vec![use_fix()]);
        let field_visibility = field_visibility_scan(Vec::new());
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_unused_pub_and_field_visibility(&unused_pub, &field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: applied 1 `pub` removal(s)".to_string())
        );
    }

    #[test]
    fn several_enabled_kinds_at_zero_emit_only_the_first_kinds_clause() {
        let unused_pub = unused_pub_scan(Vec::new());
        let field_visibility = field_visibility_scan(Vec::new());
        let notice = MendRunner::build_fix_notice(
            OperationIntent::Apply,
            None,
            fix_scans_with_unused_pub_and_field_visibility(&unused_pub, &field_visibility),
        );

        assert_eq!(
            notice.map(|notice| notice.render()),
            Some("mend: no `pub` removals available".to_string())
        );
    }
}
