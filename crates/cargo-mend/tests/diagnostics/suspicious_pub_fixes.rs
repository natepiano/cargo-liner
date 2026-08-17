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

#[test]
fn fix_keeps_pub_required_by_public_trait_impl_interface() {
    // Regression: `impl ToolFn for LaunchTarget { type Output = LaunchResult; }`
    // names `LaunchResult` in an interface whose trait and self type are both
    // declared `pub`, so rustc rejects a narrower declaration (E0446).
    // `use_sites::record_trait_impl_interface` records the crate root as that
    // interface's caller module, which on its own reads as `pub(crate)`:
    // suspicious_pub proposed the narrowing, `--fix` wrote it, the re-check
    // failed, and the whole batch rolled back. `BuildPlan` is named by no
    // interface and must still be narrowed.
    let temp = tempdir().expect("create trait-impl-interface fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Required);
    write_sources(
        temp.path(),
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "public_trait_impl_interface_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/main.rs",
                "mod app_tools;\nmod tool;\n\nuse tool::ToolFn;\n\nfn main() {\n    let handler = app_tools::create_launch_handler();\n    println!(\"{} {}\", handler.run().pid, app_tools::plan_steps());\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "mod handler;\n\npub use handler::ToolFn;\n",
            ),
            (
                "src/tool/handler.rs",
                "pub trait ToolFn {\n    type Output;\n\n    fn run(&self) -> Self::Output;\n}\n",
            ),
            (
                "src/app_tools/mod.rs",
                "mod launch;\nmod launch_handlers;\nmod planner;\n\npub use launch_handlers::create_launch_handler;\n\npub fn plan_steps() -> usize { planner::plan().steps }\n",
            ),
            (
                "src/app_tools/planner.rs",
                "use super::launch;\nuse super::launch::BuildPlan;\n\npub(super) fn plan() -> BuildPlan { launch::build_plan() }\n",
            ),
            (
                "src/app_tools/launch/mod.rs",
                "mod config;\n\npub(super) use config::BuildPlan;\npub(super) use config::LaunchResult;\n\npub(super) fn build_plan() -> BuildPlan { BuildPlan { steps: 2 } }\npub(super) fn launch_target() -> LaunchResult { LaunchResult { pid: 7 } }\n",
            ),
            (
                "src/app_tools/launch/config.rs",
                "pub struct BuildPlan {\n    pub steps: usize,\n}\n\npub struct LaunchResult {\n    pub pid: u32,\n}\n",
            ),
            (
                "src/app_tools/launch_handlers.rs",
                "use super::launch;\nuse super::launch::LaunchResult;\nuse crate::tool::ToolFn;\n\npub struct LaunchTarget;\n\nimpl ToolFn for LaunchTarget {\n    type Output = LaunchResult;\n\n    fn run(&self) -> Self::Output { launch::launch_target() }\n}\n\npub fn create_launch_handler() -> LaunchTarget { LaunchTarget }\n",
            ),
        ],
    );

    let manifest = temp.path().join("Cargo.toml");
    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("check trait-impl-interface fixture");
    assert!(
        check.status.success(),
        "fixture must compile before mend: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );

    let report = run_mend_json(&manifest);
    let narrowed: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::SuspiciousPub)
        .filter_map(|finding| finding.item.as_deref())
        .collect();
    assert!(
        !narrowed.contains(&"struct LaunchResult"),
        "`LaunchResult` is named by a public trait impl interface and must keep `pub`: {report:#?}",
    );
    assert!(
        narrowed.contains(&"struct BuildPlan"),
        "`BuildPlan` reaches no interface and must still be narrowed: {report:#?}",
    );

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

    let config = fs::read_to_string(temp.path().join("src/app_tools/launch/config.rs"))
        .expect("read fixed config source");
    assert!(
        config.contains("pub struct LaunchResult"),
        "`LaunchResult` must keep `pub` after --fix:\n{config}",
    );
    assert!(
        config.contains("pub(in crate::app_tools) struct BuildPlan"),
        "`BuildPlan` must be narrowed to its interface boundary after --fix:\n{config}",
    );
}

#[test]
fn fix_keeps_pub_required_by_a_wider_declarations_own_signature() {
    // Regression: rustc's `private_interfaces` compares a named type's reach
    // against the *declaration* that names it, not against that declaration's
    // callers. `pub fn plan(&self) -> BuildPlan` on a type re-exported at
    // `pub(crate)` needs `BuildPlan` usable crate-wide even though nothing
    // outside `crate::app_tools` calls it, and `pub launch: LaunchReport` needs
    // `LaunchReport` usable wherever the field is reachable. Signature types
    // were only recorded at call sites, so `--fix` narrowed both to the facade
    // boundary and the run came back with fresh `private_interfaces` warnings.
    let temp = tempdir().expect("create declaration interface fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Required);
    write_sources(
        temp.path(),
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "declaration_interface_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/main.rs",
                "mod app_tools;\n\nuse app_tools::Planner;\n\nfn main() {\n    let planner = Planner;\n    println!(\"{} {}\", planner.plan().steps, app_tools::summary().launch.pid);\n}\n",
            ),
            (
                "src/app_tools/mod.rs",
                "mod launch;\nmod planner;\nmod report;\n\npub(crate) use planner::Planner;\n\npub(crate) fn summary() -> report::Summary { report::summary() }\n",
            ),
            (
                "src/app_tools/planner.rs",
                "use super::launch::BuildPlan;\nuse super::launch::Internal;\n\npub(crate) struct Planner;\n\nimpl Planner {\n    pub fn plan(&self) -> BuildPlan { super::launch::build_plan() }\n}\n\npub(super) fn internal() -> Internal { super::launch::internal() }\n",
            ),
            (
                "src/app_tools/report.rs",
                "use super::launch::LaunchReport;\n\npub(crate) struct Summary {\n    pub launch: LaunchReport,\n}\n\npub(crate) fn summary() -> Summary { Summary { launch: super::launch::launch() } }\n",
            ),
            (
                "src/app_tools/launch/mod.rs",
                "mod config;\n\npub(super) use config::BuildPlan;\npub(super) use config::Internal;\npub(super) use config::LaunchReport;\n\npub(super) fn build_plan() -> BuildPlan { BuildPlan { steps: 2 } }\npub(super) fn internal() -> Internal { Internal { id: 3 } }\npub(super) fn launch() -> LaunchReport { LaunchReport { pid: 7 } }\n",
            ),
            (
                "src/app_tools/launch/config.rs",
                "pub struct BuildPlan {\n    pub steps: usize,\n}\n\npub struct LaunchReport {\n    pub pid: u32,\n}\n\npub struct Internal {\n    pub id: u32,\n}\n",
            ),
        ],
    );

    let manifest = temp.path().join("Cargo.toml");
    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("check declaration interface fixture");
    assert!(
        check.status.success(),
        "fixture must compile before mend: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );

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

    let config = fs::read_to_string(temp.path().join("src/app_tools/launch/config.rs"))
        .expect("read fixed config source");
    assert!(
        config.contains("pub(crate) struct BuildPlan"),
        "`BuildPlan` is named by a `pub(crate)`-reachable method and must stay usable there:\n{config}",
    );
    assert!(
        config.contains("pub(crate) struct LaunchReport"),
        "`LaunchReport` is named by a `pub(crate)`-reachable field and must stay usable there:\n{config}",
    );
    assert!(
        config.contains("pub(in crate::app_tools) struct Internal"),
        "`Internal` reaches no wider declaration and must still be narrowed:\n{config}",
    );

    let fixed_check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("check fixed declaration interface fixture");
    let fixed_stderr = String::from_utf8_lossy(&fixed_check.stderr);
    assert!(
        fixed_check.status.success() && !fixed_stderr.contains("more private than"),
        "the fixed fixture must compile without `private_interfaces` warnings: {fixed_stderr}",
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
