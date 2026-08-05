use crate::support::*;

const TARGETS: [&str; 5] = [
    "struct TextRunBatch",
    "fn world_bounds",
    "fn is_empty",
    "trait Batch",
    "struct TextRunPayload",
];

#[test]
fn fixes_crate_ancestor_private_and_dependent_signature_boundaries_together() {
    let temp = tempdir().expect("create visibility-fix fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Required);
    write_sources(
        temp.path(),
        &[
            (
                "mend.toml",
                "[visibility]\npub_in_path = \"required\"\nallow_pub_mod = [\"src/render/analytic_paths/mod.rs\"]\n\n[diagnostics]\nsuspicious_pub = true\n",
            ),
            (
                "Cargo.toml",
                r#"[package]
name = "suspicious_pub_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/lib.rs",
                "mod render;\nmod text;\npub fn entry() -> usize { render::entry() + text::entry() }\n",
            ),
            (
                "src/render/mod.rs",
                "mod analytic_paths;\nmod consumer;\nmod text_run;\npub(crate) fn entry() -> usize { consumer::entry() }\npub(crate) fn batch_for_text() -> analytic_paths::batching::TextRunBatch { analytic_paths::batching::make() }\n",
            ),
            (
                "src/render/analytic_paths/mod.rs",
                "pub(super) mod batching;\n",
            ),
            (
                "src/render/analytic_paths/batching.rs",
                "pub struct TextRunBatch { value: usize }\nimpl TextRunBatch {\n    pub fn world_bounds(&self) -> usize { self.value }\n    pub fn is_empty(&self) -> bool { self.value == 0 }\n}\npub trait Batch { type Payload; fn payload(self) -> Self::Payload; }\npub(in crate::render) struct Carrier;\nimpl Batch for Carrier { type Payload = crate::render::text_run::TextRunPayload; fn payload(self) -> Self::Payload { crate::render::text_run::TextRunPayload } }\npub(in crate::render) fn make() -> TextRunBatch { let batch = TextRunBatch { value: 1 }; let _ = batch.is_empty(); batch }\n",
            ),
            (
                "src/render/consumer.rs",
                "use super::analytic_paths::batching::{self, Batch};\nfn consume<B: Batch>(value: B) -> B::Payload { value.payload() }\npub(super) fn entry() -> usize { let batch = batching::make(); let _ = consume(batching::Carrier); batch.world_bounds() }\n",
            ),
            (
                "src/render/text_run.rs",
                "mod payload;\npub(super) use payload::TextRunPayload;\n",
            ),
            (
                "src/render/text_run/payload.rs",
                "pub struct TextRunPayload;\n",
            ),
            (
                "src/text.rs",
                "pub(crate) fn entry() -> usize { let _batch = crate::render::batch_for_text(); 1 }\n",
            ),
        ],
    );

    let manifest = temp.path().join("Cargo.toml");
    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("check visibility-fix fixture");
    assert!(
        check.status.success(),
        "fixture must compile before mend: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );
    let report = run_mend_json(&manifest);
    assert_fixable_findings(&report);

    let output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert_fixed_sources(temp.path());
    assert_findings_converged(&manifest);
}

fn assert_fixable_findings(report: &Report) {
    for target in TARGETS {
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.code == DiagnosticCode::SuspiciousPub
                    && finding.item.as_deref().is_some_and(|item| item == target)
            })
            .unwrap_or_else(|| panic!("missing suspicious_pub finding for {target}: {report:#?}"));
        assert!(
            finding
                .help
                .iter()
                .any(|line| line.contains("auto-fixable")),
            "{target} must be offered to --fix: {report:#?}",
        );
        if target == "trait Batch" {
            assert!(
                finding
                    .help
                    .iter()
                    .any(|line| line == "consider using: `pub(in crate::render)`"),
                "the trait bound in the sibling consumer must retain render visibility: {report:#?}",
            );
        }
    }
}

fn assert_fixed_sources(root: &std::path::Path) {
    let batching = fs::read_to_string(root.join("src/render/analytic_paths/batching.rs"))
        .expect("read fixed batching source");
    assert!(batching.contains("pub(crate) struct TextRunBatch"));
    assert!(batching.contains("pub(in crate::render) fn world_bounds"));
    assert!(batching.contains("    fn is_empty"));
    assert!(!batching.contains("pub fn is_empty"));
    assert!(
        batching.contains("pub(in crate::render) trait Batch"),
        "unexpected fixed trait visibility:\n{batching}",
    );

    let payload = fs::read_to_string(root.join("src/render/text_run/payload.rs"))
        .expect("read fixed payload source");
    assert_eq!(payload, "pub(in crate::render) struct TextRunPayload;\n");
}

fn assert_findings_converged(manifest: &std::path::Path) {
    let fixed_report = run_mend_json(manifest);
    assert!(
        fixed_report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::SuspiciousPub
                || !TARGETS
                    .iter()
                    .any(|target| finding.item.as_deref().is_some_and(|item| item == *target))
        }),
        "fixed visibility findings must not recur: {fixed_report:#?}",
    );
}

fn write_sources(root: &std::path::Path, sources: &[(&str, &str)]) {
    for (relative_path, source) in sources {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture source directory");
        }
        fs::write(path, source).expect("write fixture source");
    }
}
