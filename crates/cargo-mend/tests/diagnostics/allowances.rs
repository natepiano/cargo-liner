use serde_json::Value;
use tempfile::TempDir;

use crate::support::*;

fn write_minimal_manifest(temp: &TempDir, package_name: &str) {
    fs::write(
        temp.path().join("Cargo.toml"),
        format!("[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .expect("write fixture manifest");
}

fn write_allowance_sources(temp: &TempDir, sources: &[(&str, &str)]) {
    for (relative_path, source) in sources {
        let path = temp.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture source directory");
        }
        fs::write(path, source).expect("write fixture source");
    }
}

#[derive(Clone, Copy)]
enum ForbiddenVisibilityPersistenceExpectation<'a> {
    AcceptedCrate {
        visibility_annotation: &'a str,
        item_def_path:         &'a str,
        item_module_def_path:  &'a str,
    },
    AcceptedRestricted {
        visibility_annotation:      &'a str,
        item_def_path:              &'a str,
        item_module_def_path:       &'a str,
        required_boundary_def_path: &'a str,
    },
    Public {
        visibility_annotation: &'a str,
        item_def_path:         &'a str,
    },
    Restricted {
        visibility_annotation:       &'a str,
        item_def_path:               &'a str,
        item_module_def_path:        &'a str,
        signature_boundary_def_path: &'a str,
    },
    ResolvedFacadeRestricted {
        visibility_annotation:      &'a str,
        item_def_path:              &'a str,
        item_module_def_path:       &'a str,
        required_boundary_def_path: &'a str,
    },
    StructuralBlocker {
        visibility_annotation: &'a str,
        item_def_path:         &'a str,
    },
}

impl<'a> ForbiddenVisibilityPersistenceExpectation<'a> {
    const fn visibility_annotation(self) -> &'a str {
        match self {
            Self::AcceptedCrate {
                visibility_annotation,
                ..
            }
            | Self::AcceptedRestricted {
                visibility_annotation,
                ..
            }
            | Self::Public {
                visibility_annotation,
                ..
            }
            | Self::Restricted {
                visibility_annotation,
                ..
            }
            | Self::ResolvedFacadeRestricted {
                visibility_annotation,
                ..
            }
            | Self::StructuralBlocker {
                visibility_annotation,
                ..
            } => visibility_annotation,
        }
    }

    const fn item_def_path(self) -> &'a str {
        match self {
            Self::AcceptedCrate { item_def_path, .. }
            | Self::AcceptedRestricted { item_def_path, .. }
            | Self::Public { item_def_path, .. }
            | Self::Restricted { item_def_path, .. }
            | Self::ResolvedFacadeRestricted { item_def_path, .. }
            | Self::StructuralBlocker { item_def_path, .. } => item_def_path,
        }
    }

    const fn item_module_def_path(self) -> Option<&'a str> {
        match self {
            Self::AcceptedCrate {
                item_module_def_path,
                ..
            }
            | Self::AcceptedRestricted {
                item_module_def_path,
                ..
            }
            | Self::Restricted {
                item_module_def_path,
                ..
            }
            | Self::ResolvedFacadeRestricted {
                item_module_def_path,
                ..
            } => Some(item_module_def_path),
            Self::Public { .. } | Self::StructuralBlocker { .. } => None,
        }
    }

    const fn outcome(self) -> &'static str {
        match self {
            Self::AcceptedCrate { .. } | Self::AcceptedRestricted { .. } => "accepted",
            Self::Public { .. }
            | Self::Restricted { .. }
            | Self::ResolvedFacadeRestricted { .. }
            | Self::StructuralBlocker { .. } => "finding",
        }
    }

    const fn required_boundary(self) -> Option<&'a str> {
        match self {
            Self::AcceptedRestricted {
                required_boundary_def_path,
                ..
            }
            | Self::ResolvedFacadeRestricted {
                required_boundary_def_path,
                ..
            } => Some(required_boundary_def_path),
            Self::Restricted {
                signature_boundary_def_path,
                ..
            } => Some(signature_boundary_def_path),
            Self::Public { .. } => Some("crate-external"),
            Self::AcceptedCrate { .. } | Self::StructuralBlocker { .. } => None,
        }
    }

    const fn declared_boundary(self) -> Option<&'static str> {
        match self {
            Self::AcceptedCrate { .. } => Some("crate"),
            Self::AcceptedRestricted { .. }
            | Self::Public { .. }
            | Self::Restricted { .. }
            | Self::ResolvedFacadeRestricted { .. }
            | Self::StructuralBlocker { .. } => None,
        }
    }

    const fn facade_kind(self) -> Option<&'static str> {
        match self {
            Self::ResolvedFacadeRestricted { .. } => Some("resolved"),
            Self::StructuralBlocker { .. } => Some("blocked"),
            Self::AcceptedCrate { .. }
            | Self::AcceptedRestricted { .. }
            | Self::Public { .. }
            | Self::Restricted { .. } => None,
        }
    }
}

fn stored_reach_boundary(stored_reach: &Value) -> Option<&str> {
    match stored_reach.get("kind").and_then(Value::as_str) {
        Some("public") => Some("crate-external"),
        Some("crate") => Some("crate"),
        Some("restricted") => stored_reach.get("boundary").and_then(Value::as_str),
        Some(_) | None => None,
    }
}

fn constraint_has_requirement(constraint: &Value, boundary: &str) -> bool {
    constraint
        .get("signature_requirement")
        .and_then(stored_reach_boundary)
        == Some(boundary)
        || constraint
            .get("facade")
            .and_then(|facade| facade.get("required"))
            .and_then(stored_reach_boundary)
            == Some(boundary)
}

fn constraint_matches_location(
    constraint: &Value,
    diagnostic_code: &str,
    finding_path: &str,
    finding_line: u64,
) -> bool {
    constraint.get("diagnostic_code").and_then(Value::as_str) == Some(diagnostic_code)
        && constraint
            .get("source")
            .and_then(|source| source.get("path"))
            .and_then(Value::as_str)
            .is_some_and(|path| path.ends_with(finding_path))
        && constraint
            .get("source")
            .and_then(|source| source.get("line"))
            .and_then(Value::as_u64)
            == Some(finding_line)
}

fn assert_constraint_matches(
    constraint: &Value,
    expectation: ForbiddenVisibilityPersistenceExpectation<'_>,
    stored_report: &Value,
) {
    assert_eq!(
        constraint
            .get("visibility_annotation")
            .and_then(Value::as_str),
        Some(expectation.visibility_annotation()),
        "{stored_report:#?}"
    );
    assert_eq!(
        constraint
            .get("declaration")
            .and_then(|declaration| declaration.get("item_def_path"))
            .and_then(Value::as_str),
        Some(expectation.item_def_path()),
        "{stored_report:#?}"
    );
    if let Some(item_module_def_path) = expectation.item_module_def_path() {
        assert_eq!(
            constraint
                .get("declaration")
                .and_then(|declaration| declaration.get("item_module_def_path"))
                .and_then(Value::as_str),
            Some(item_module_def_path),
            "{stored_report:#?}"
        );
    }
    assert_eq!(
        constraint.get("outcome").and_then(Value::as_str),
        Some(expectation.outcome()),
        "{stored_report:#?}"
    );
    if let Some(boundary) = expectation.required_boundary() {
        assert!(
            constraint_has_requirement(constraint, boundary),
            "{stored_report:#?}"
        );
    }
    if let Some(boundary) = expectation.declared_boundary() {
        assert_eq!(
            constraint
                .get("declared_reach")
                .and_then(stored_reach_boundary),
            Some(boundary),
            "{stored_report:#?}"
        );
    }
    if let Some(facade_kind) = expectation.facade_kind() {
        assert_eq!(
            constraint
                .get("facade")
                .and_then(|facade| facade.get("kind"))
                .and_then(Value::as_str),
            Some(facade_kind),
            "{stored_report:#?}"
        );
    }
}

fn assert_stored_forbidden_visibility_advice(
    temp: &TempDir,
    diagnostic_code: &str,
    finding_path: &str,
    finding_line: u64,
    expectation: ForbiddenVisibilityPersistenceExpectation<'_>,
) {
    let findings_dir = temp.path().join("target/mend-findings");
    let mut matches = 0;
    for entry in fs::read_dir(findings_dir).expect("read stored findings directory") {
        let path = entry.expect("read stored finding entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read stored findings report");
        let stored_report = serde_json::from_slice::<Value>(&bytes).expect("parse stored report");
        let stored_text = stored_report.to_string();
        assert!(!stored_text.contains("cargo-mend-accepted-visibility"));
        assert!(!stored_text.contains("cargo-mend-forbidden-visibility"));
        let constraints = stored_report
            .get("visibility_constraints")
            .and_then(Value::as_array)
            .expect("read stored visibility constraints");
        for constraint in constraints {
            if !constraint_matches_location(constraint, diagnostic_code, finding_path, finding_line)
            {
                continue;
            }
            matches += 1;
            assert_constraint_matches(constraint, expectation, &stored_report);
        }
    }
    assert!(
        matches > 0,
        "missing stored {diagnostic_code} at {finding_path}:{finding_line}"
    );
}

fn assert_stored_finding_has_no_refinement_metadata(
    temp: &TempDir,
    diagnostic_code: &str,
    finding_path: &str,
) {
    let findings_dir = temp.path().join("target/mend-findings");
    let mut matches = 0;
    for entry in fs::read_dir(findings_dir).expect("read stored findings directory") {
        let path = entry.expect("read stored finding entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read stored findings report");
        let stored_report = serde_json::from_slice::<Value>(&bytes).expect("parse stored report");
        let findings = stored_report
            .get("findings")
            .and_then(Value::as_array)
            .expect("read stored findings");
        for finding in findings {
            if finding.get("diagnostic_code").and_then(Value::as_str) != Some(diagnostic_code)
                || !finding
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|path| path.ends_with(finding_path))
            {
                continue;
            }
            matches += 1;
            assert!(finding.get("item_def_path").is_none(), "{stored_report:#?}");
            assert!(
                finding.get("narrower_scope_def_path").is_none(),
                "{stored_report:#?}"
            );
            assert!(
                finding
                    .get("caller_refinement_signature_requirement")
                    .is_none(),
                "v21 findings must not contain caller-refinement state: {stored_report:#?}",
            );
        }
    }
    assert!(
        matches > 0,
        "missing stored {diagnostic_code} at {finding_path}"
    );
}

fn assert_no_stored_pub_use_fix_facts(temp: &TempDir) {
    let findings_dir = temp.path().join("target/mend-findings");
    let mut stored_reports = 0;
    for entry in fs::read_dir(findings_dir).expect("read stored findings directory") {
        let path = entry.expect("read stored finding entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        stored_reports += 1;
        let bytes = fs::read(&path).expect("read stored findings report");
        let stored_report = serde_json::from_slice::<Value>(&bytes).expect("parse stored report");
        let fix_facts = stored_report
            .get("pub_use_fix_facts")
            .and_then(Value::as_array)
            .expect("read stored pub-use fix facts");
        assert!(fix_facts.is_empty(), "{stored_report:#?}");
    }
    assert!(stored_reports > 0, "missing stored findings report");
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_other_crate_visible_signature() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_signature_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod utils;

fn main() {
    let repo = utils::collect_repository_files();
    consumer::consume(repo);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn consume(_: impl Sized) {}
"#,
    )
    .expect("write fixture consumer");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod file_utils;

pub use file_utils::{collect_repository_files, RepositoryFiles};
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/file_utils.rs"),
        r#"pub struct RepositoryFiles;

pub fn collect_repository_files() -> RepositoryFiles {
    RepositoryFiles
}
"#,
    )
    .expect("write child module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/utils/file_utils.rs"),
        "expected no suspicious_pub for child type exposed by another crate-visible signature, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_sibling_boundary_field() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/app_tools")).expect("create src/app_tools");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_sibling_boundary_field_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod app_tools;

fn main() {
    consumer::run(None);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run(_: Option<crate::app_tools::LaunchParams>) {}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/app_tools.rs"),
        r#"mod count;
mod launch_params;

pub use launch_params::LaunchParams;
"#,
    )
    .expect("write parent facade");
    fs::write(
        temp.path().join("src/app_tools/count.rs"),
        r#"pub struct Count(pub u16);
"#,
    )
    .expect("write child count");
    fs::write(
        temp.path().join("src/app_tools/launch_params.rs"),
        r#"use super::count::Count;

pub struct LaunchParams {
    pub count: Count,
}
"#,
    )
    .expect("write sibling boundary");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/app_tools/count.rs"),
        "expected no suspicious_pub for child type exposed by sibling boundary field, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_ancestor_boundary_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/brp_tools/tools")).expect("create nested fixture");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_ancestor_boundary_field_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod brp_tools;

fn main() {
    consumer::run(None);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run(_: Option<crate::brp_tools::ClickParams>) {}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/brp_tools.rs"),
        r#"mod types;
mod tools;

pub use tools::ClickParams;
"#,
    )
    .expect("write ancestor boundary");
    fs::write(
        temp.path().join("src/brp_tools/types.rs"),
        r#"pub enum MouseButtonWrapper {
    Left,
}
"#,
    )
    .expect("write child type");
    fs::write(
        temp.path().join("src/brp_tools/tools/mod.rs"),
        r#"mod click;

pub use click::ClickParams;
"#,
    )
    .expect("write immediate boundary");
    fs::write(
        temp.path().join("src/brp_tools/tools/click.rs"),
        r#"use crate::brp_tools::types::MouseButtonWrapper;

pub struct ClickParams {
    pub button: MouseButtonWrapper,
}
"#,
    )
    .expect("write sibling boundary");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/brp_tools/types.rs"),
        "expected no suspicious_pub for child type exposed by sibling boundary field through ancestor re-export, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn suspicious_pub_is_suppressed_for_cross_file_public_field_exposure_via_ancestor_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/brp_tools/tools")).expect("create nested fixture");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cross_file_public_field_exposure_via_ancestor_reexport_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod brp_tools;

fn main() {
    consumer::run(None);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run(_: Option<crate::brp_tools::ClickParams>) {}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/brp_tools.rs"),
        r#"mod types;
mod tools;

pub use tools::brp_extras_click_mouse::ClickParams;
"#,
    )
    .expect("write ancestor boundary");
    fs::write(
        temp.path().join("src/brp_tools/types.rs"),
        r#"pub enum MouseButtonWrapper {
    Left,
}
"#,
    )
    .expect("write child type");
    fs::write(
        temp.path().join("src/brp_tools/tools/mod.rs"),
        r#"pub mod brp_extras_click_mouse;
"#,
    )
    .expect("write immediate boundary");
    fs::write(
        temp.path()
            .join("src/brp_tools/tools/brp_extras_click_mouse.rs"),
        r#"use crate::brp_tools::types::MouseButtonWrapper;

pub struct ClickParams {
    pub button: MouseButtonWrapper,
}
"#,
    )
    .expect("write sibling outward type");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/brp_tools/types.rs"
                && finding.item.as_deref() == Some("enum MouseButtonWrapper")
        }),
        "expected no suspicious_pub for child type exposed by sibling boundary field through ancestor re-export without immediate parent pub use, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn suspicious_pub_is_suppressed_when_grandparent_reexports_through_pub_super_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/brp_tools/tools")).expect("create nested fixture");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "grandparent_reexport_pub_super_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod brp_tools;

fn main() {
    consumer::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"use crate::brp_tools::BrpExecute;

pub fn run() {
    let _ = BrpExecute;
}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/brp_tools/mod.rs"),
        r#"mod tools;

pub use tools::brp_execute::BrpExecute;
"#,
    )
    .expect("write grandparent boundary");
    fs::write(
        temp.path().join("src/brp_tools/tools/mod.rs"),
        r#"pub(super) mod brp_execute;
"#,
    )
    .expect("write immediate parent boundary");
    fs::write(
        temp.path().join("src/brp_tools/tools/brp_execute.rs"),
        r#"pub struct BrpExecute;
"#,
    )
    .expect("write leaf module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::SuspiciousPub
                && finding.path.contains("brp_execute.rs")),
        "expected no suspicious_pub when grandparent re-exports through pub(super) module, got: {:#?}",
        report.findings
    );
}

#[test]
fn suspicious_pub_is_suppressed_for_cross_file_public_field_exposure() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/guide")).expect("create nested fixture");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cross_file_public_field_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod guide;

fn main() {
    consumer::run(None);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run(_: Option<crate::guide::TypeGuideResponse>) {}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/guide.rs"),
        r#"mod response_types;

pub use response_types::TypeGuideResponse;
"#,
    )
    .expect("write guide boundary");
    fs::write(
        temp.path().join("src/guide/response_types.rs"),
        r#"pub struct TypeGuideResponse {
    pub summary: TypeGuideSummary,
}

pub struct TypeGuideSummary {
    pub total_requested: usize,
}
"#,
    )
    .expect("write response types");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/guide/response_types.rs"
                && finding.item.as_deref() == Some("struct TypeGuideSummary")
        }),
        "expected no suspicious_pub for child type exposed by cross-file public field, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_exported_method_signatures() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_method_signature_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod utils;

fn main() {
    consumer::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run() {
    let (_, _) = crate::utils::load_cache();
    let mut cache = crate::utils::Sha256Cache;
    let _ = cache.get_or_update();
}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod sha256_cache;

pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, Sha256Cache};

pub fn load_cache() -> (Sha256Cache, CacheFileStatus) {
    Sha256Cache::load_or_create()
}
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/sha256_cache.rs"),
        r#"pub enum CacheFileStatus {
    Present,
}

pub enum CacheEntryStatus {
    Fresh,
}

pub struct Sha256Cache;

impl Sha256Cache {
    pub fn load_or_create() -> (Self, CacheFileStatus) {
        (Self, CacheFileStatus::Present)
    }

    pub fn get_or_update(&mut self) -> CacheEntryStatus {
        CacheEntryStatus::Fresh
    }
}
"#,
    )
    .expect("write child module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/utils/sha256_cache.rs"),
        "expected no suspicious_pub for child types exposed by exported method signatures, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_parent_boundary_signature() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/wikilink")).expect("create src/wikilink");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_parent_boundary_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod wikilink;

fn main() {
    consumer::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run() {
    let extracted: crate::wikilink::ParsedExtractedWikilinks = crate::wikilink::extract();
    let _ = extracted.valid.len();
}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/wikilink.rs"),
        r#"mod wikilink_types;

pub use wikilink_types::{ParsedExtractedWikilinks, ParsedInvalidWikilink};

pub fn extract() -> ParsedExtractedWikilinks {
    ParsedExtractedWikilinks { valid: vec![], invalid: vec![] }
}
"#,
    )
    .expect("write wikilink facade");
    fs::write(
        temp.path().join("src/wikilink/wikilink_types.rs"),
        r#"pub struct ParsedExtractedWikilinks {
    pub valid: Vec<String>,
    pub invalid: Vec<ParsedInvalidWikilink>,
}

pub struct ParsedInvalidWikilink;
"#,
    )
    .expect("write child module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/wikilink/wikilink_types.rs"),
        "expected no suspicious_pub for child types exposed by parent boundary signatures, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}

#[test]
fn suspicious_pub_is_suppressed_for_parent_facade_used_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "facade_positive_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_parent;

use crate::private_parent::PublicContainer;

fn main() {
    let _ = std::mem::size_of::<PublicContainer>();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_still_warns_for_parent_facade_unused_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "facade_negative_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 1);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);
    assert_eq!(report.findings.len(), 1);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["suspicious_pub"]));
}

#[test]
fn internal_parent_pub_use_facade_warns_for_parent_facade_used_inside_parent_subtree() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "internal_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\nmod sibling;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/private_parent/sibling.rs"),
        "fn sibling_uses_facade() {\n    let _ = std::mem::size_of::<super::PublicContainer>();\n}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 1);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["internal_parent_pub_use_facade"]));
}

#[test]
fn internal_parent_pub_use_facade_warns_for_parent_facade_imported_inside_parent_subtree() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "internal_facade_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\nmod sibling;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/private_parent/sibling.rs"),
        "use super::PublicContainer;\n\nfn sibling_uses_facade() {\n    let _ = std::mem::size_of::<PublicContainer>();\n}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 1);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["internal_parent_pub_use_facade"]));
}

#[test]
fn parent_facade_is_allowed_for_function_local_use_outside_parent_subtree() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");
    fs::create_dir_all(temp.path().join("src/consumer")).expect("create consumer dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "function_local_facade_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\nmod consumer;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/consumer/mod.rs"),
        "use crate::private_parent::PublicContainer;\n\nfn consume() {\n    let _ = std::mem::size_of::<PublicContainer>();\n}\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_for_internal_parent_super_facade_in_mod_rs() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "internal_super_facade_mod_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\nmod sibling;\npub(super) use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/private_parent/sibling.rs"),
        "use super::PublicContainer;\n\nfn sibling_uses_facade() {\n    let _ = std::mem::size_of::<PublicContainer>();\n}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_when_child_boundary_file_is_mod_rs_and_parent_facade_is_used() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/parent/child")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "child_mod_rs_parent_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nuse parent::BoundaryType;\n\nfn main() {\n    let _ = \
         BoundaryType;\n}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::BoundaryType;\n",
    )
    .expect("write parent boundary");
    fs::write(
        temp.path().join("src/parent/child/mod.rs"),
        "pub struct BoundaryType;\n",
    )
    .expect("write child boundary");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_for_internal_parent_super_facade_in_file_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "internal_super_facade_file_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\nmod sibling;\npub(super) use child::PublicContainer;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/private_parent/sibling.rs"),
        "use super::PublicContainer;\n\nfn sibling_uses_facade() {\n    let _ = std::mem::size_of::<PublicContainer>();\n}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn crate_relative_parent_facade_use_inside_parent_subtree_stays_fixable() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "crate_relative_internal_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\nmod sibling;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/private_parent/sibling.rs"),
        "use crate::private_parent::PublicContainer;\n\nfn sibling_uses_facade() {\n    let _ = std::mem::size_of::<PublicContainer>();\n}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 2);
    assert_eq!(report.summary.fixable_with_fix, 1);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        codes,
        BTreeSet::from([
            "internal_parent_pub_use_facade",
            "shorten_local_crate_import"
        ])
    );
}

#[test]
fn suspicious_pub_is_suppressed_for_file_parent_facade_used_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "file_facade_positive_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_parent;

use crate::private_parent::PublicContainer;

fn main() {
    let _ = std::mem::size_of::<PublicContainer>();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 0);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_for_tool_contract_attribute_output_type() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("app/src/tools")).expect("create app fixture dirs");
    fs::create_dir_all(temp.path().join("macros/src")).expect("create macros dirs");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["app", "macros"]
resolver = "3"
"#,
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("app/Cargo.toml"),
        r#"[package]
name = "tool_contract_fixture"
version = "0.1.0"
edition = "2024"

[dependencies]
macros_fixture = { path = "../macros" }
"#,
    )
    .expect("write app manifest");
    fs::write(
        temp.path().join("app/src/main.rs"),
        r#"mod tools;

use crate::tools::ListThings;

fn main() {
    let _ = std::mem::size_of::<ListThings>();
}
"#,
    )
    .expect("write app main");
    fs::write(
        temp.path().join("app/src/tools.rs"),
        "mod list_things;\npub use list_things::ListThings;\n",
    )
    .expect("write tools facade");
    fs::write(
        temp.path().join("app/src/tools/list_things.rs"),
        r#"use macros_fixture::tool_fn;

pub struct ListThingsResult;

#[tool_fn(output = "ListThingsResult")]
pub struct ListThings;
"#,
    )
    .expect("write tool child");
    fs::write(
        temp.path().join("macros/Cargo.toml"),
        r#"[package]
name = "macros_fixture"
version = "0.1.0"
edition = "2024"

[lib]
proc-macro = true
"#,
    )
    .expect("write macros manifest");
    fs::write(
        temp.path().join("macros/src/lib.rs"),
        r#"use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn tool_fn(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
"#,
    )
    .expect("write macros lib");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "app/src/tools/list_things.rs"
                && finding.item.as_deref() == Some("struct ListThingsResult")
        }),
        "expected no suspicious_pub for tool output referenced by public attribute metadata, got: {:#?}",
        report.findings
    );
}

#[test]
fn suspicious_pub_is_suppressed_for_explicit_trait_impl_output_type() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/tools")).expect("create tool fixture dirs");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "explicit_tool_contract_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod tools;

use crate::tools::ListThings;

fn main() {
    let _ = std::mem::size_of::<ListThings>();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/tools.rs"),
        "mod list_things;\npub use list_things::ListThings;\npub trait ToolFn { type Output; }\n",
    )
    .expect("write tools facade");
    fs::write(
        temp.path().join("src/tools/list_things.rs"),
        r#"pub struct ListThingsResult;

pub struct ListThings;

impl super::ToolFn for ListThings {
    type Output = ListThingsResult;
}
"#,
    )
    .expect("write tool child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/tools/list_things.rs"
                && finding.item.as_deref() == Some("struct ListThingsResult")
        }),
        "expected no suspicious_pub for output type referenced by explicit trait impl, got: {:#?}",
        report.findings
    );
}

#[test]
fn suspicious_pub_is_suppressed_for_methods_on_type_exposed_by_public_enum_variant() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/api")).expect("create api fixture dirs");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "enum_variant_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod api;

use crate::api::ResponseStatus;

fn main() {
    let _ = std::mem::size_of::<ResponseStatus>();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/api.rs"),
        "mod types;\npub use types::ResponseStatus;\n",
    )
    .expect("write api facade");
    fs::write(
        temp.path().join("src/api/types.rs"),
        r#"pub struct ClientError {
    message: String,
}

impl ClientError {
    pub fn get_message(&self) -> &str { &self.message }
}

pub enum ResponseStatus {
    Error(ClientError),
}
"#,
    )
    .expect("write api child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/api/types.rs"
                && finding.item.as_deref() == Some("fn get_message")
        }),
        "expected no suspicious_pub for method on type exposed by public enum variant, got: {:#?}",
        report.findings
    );
}

#[test]
fn suspicious_pub_still_warns_for_file_parent_facade_unused_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "file_facade_negative_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 1);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);
    assert_eq!(report.findings.len(), 1);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["suspicious_pub"]));
}

#[test]
fn wildcard_parent_pub_use_warns() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "wildcard_parent_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::*;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("wildcard_parent_pub_use"));
}

#[test]
fn suspicious_pub_not_triggered_for_impl_methods_on_type_defined_in_sibling_child_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/tui/app")).expect("create src/tui/app");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "impl_method_sibling_type_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod tui;

fn main() {
    let app = tui::app::App::new();
    tui::run(&app);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/tui.rs"),
        r#"pub mod app;

pub fn run(app: &app::App) {
    let _ = app.is_searching();
}
"#,
    )
    .expect("write tui facade");
    fs::write(
        temp.path().join("src/tui/app.rs"),
        r#"mod types;
mod focus;

pub use types::App;
"#,
    )
    .expect("write app facade");
    fs::write(
        temp.path().join("src/tui/app/types.rs"),
        r#"pub struct App {
    pub searching: bool,
}

impl App {
    pub fn new() -> Self {
        Self { searching: false }
    }
}
"#,
    )
    .expect("write types child");
    fs::write(
        temp.path().join("src/tui/app/focus.rs"),
        r#"use super::App;

impl App {
    pub fn is_searching(&self) -> bool {
        self.searching
    }
}
"#,
    )
    .expect("write focus child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let focus_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/tui/app/focus.rs"
        })
        .collect();
    assert!(
        focus_findings.is_empty(),
        "expected no suspicious_pub for impl method on type defined in sibling child module \
         and re-exported from parent, got: {focus_findings:#?}",
    );
}

fn assert_path_sibling_signature_exposure_uses_logical_identity() {
    let sibling = tempdir().expect("create path sibling exposure fixture dir");
    pin_pub_in_path(sibling.path(), PubInPath::Permitted);
    fs::create_dir_all(sibling.path().join("src/a/odd")).expect("create fixture modules");
    write_minimal_manifest(&sibling, "path_sibling_signature_exposure_fixture");
    fs::write(sibling.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(sibling.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        sibling.path().join("src/a/b.rs"),
        "#[path = \"odd/target.rs\"]\nmod target;\n#[path = \"odd/exposer.rs\"]\nmod exposer;\npub use exposer::Container;\n",
    )
    .expect("write logical parent boundary");
    fs::write(
        sibling.path().join("src/a/odd/target.rs"),
        "pub(crate) struct Target;\n",
    )
    .expect("write target module");
    fs::write(
        sibling.path().join("src/a/odd/exposer.rs"),
        "pub struct Container { pub target: super::target::Target }\n",
    )
    .expect("write sibling signature");

    let sibling_report = run_mend_json(&sibling.path().join("Cargo.toml"));
    let target_finding = sibling_report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.path == "src/a/odd/target.rs"
        })
        .expect("find target visibility finding");
    assert!(
        target_finding
            .help
            .iter()
            .any(|line| {
                line
                    == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
            }),
        "logical sibling signature exposure must preserve its private-ancestor cap: {sibling_report:#?}"
    );
}

fn assert_split_definition_lookup_uses_hir_identity() {
    let split_definition = tempdir().expect("create split definition fixture dir");
    pin_pub_in_path(split_definition.path(), PubInPath::Permitted);
    fs::create_dir_all(split_definition.path().join("src/a/types"))
        .expect("create type fixture module");
    fs::create_dir_all(split_definition.path().join("src/a/impls"))
        .expect("create impl fixture module");
    write_minimal_manifest(
        &split_definition,
        "path_split_definition_signature_exposure_fixture",
    );
    fs::write(
        split_definition.path().join("src/main.rs"),
        "mod a;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(split_definition.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        split_definition.path().join("src/a/b.rs"),
        "#[path = \"types/widget.rs\"]\nmod types;\n#[path = \"impls/widget.rs\"]\nmod methods;\npub use types::Widget;\n",
    )
    .expect("write split definition boundary");
    fs::write(
        split_definition.path().join("src/a/types/widget.rs"),
        "pub struct Widget;\n",
    )
    .expect("write split type definition");
    fs::write(
        split_definition.path().join("src/a/impls/widget.rs"),
        "use super::types::Widget;\nimpl Widget { pub fn activate(&self) {} }\n",
    )
    .expect("write split inherent implementation");

    let split_report = run_mend_json(&split_definition.path().join("Cargo.toml"));
    assert!(
        !split_report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/impls/widget.rs"
                && finding.item.as_deref() == Some("fn activate")
        }),
        "HIR definition lookup must find the re-exported self type: {split_report:#?}"
    );
}

fn assert_parent_signature_exposure_uses_logical_identity() {
    let parent_signature = tempdir().expect("create parent signature fixture dir");
    pin_pub_in_path(parent_signature.path(), PubInPath::Permitted);
    fs::create_dir_all(parent_signature.path().join("src/a/odd"))
        .expect("create parent signature modules");
    write_minimal_manifest(&parent_signature, "path_parent_signature_exposure_fixture");
    fs::write(
        parent_signature.path().join("src/main.rs"),
        "mod a;\nfn main() { let _: a::b::Response = a::b::make(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        parent_signature.path().join("src/a.rs"),
        "pub(crate) mod b;\n",
    )
    .expect("write outer module");
    fs::write(
        parent_signature.path().join("src/a/b.rs"),
        "#[path = \"odd/response.rs\"]\nmod child;\npub use child::Response;\npub fn make() -> Response { Response }\n",
    )
    .expect("write parent signature boundary");
    fs::write(
        parent_signature.path().join("src/a/odd/response.rs"),
        "pub struct Response;\n",
    )
    .expect("write parent signature type");

    let parent_report = run_mend_json(&parent_signature.path().join("Cargo.toml"));
    assert!(
        !parent_report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/odd/response.rs"
                && finding.item.as_deref() == Some("struct Response")
        }),
        "logical parent signature usage must suppress suspicious_pub: {parent_report:#?}"
    );
}

#[test]
fn path_modules_use_logical_identity_for_signature_exposure() {
    assert_path_sibling_signature_exposure_uses_logical_identity();
    assert_split_definition_lookup_uses_hir_identity();
    assert_parent_signature_exposure_uses_logical_identity();
}

#[test]
fn restricted_signature_exposure_accepts_its_exact_facade_boundary() {
    let temp = tempdir().expect("create signature exposure fixture dir");
    fs::create_dir_all(temp.path().join("src/video_plane/plane")).expect("create fixture modules");
    write_minimal_manifest(&temp, "restricted_signature_exposure_fixture");
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write fixture visibility config");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod video_plane;\n\nfn main() { video_plane::caller(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/video_plane.rs"),
        "mod plane;\n\npub(crate) fn caller() { let _ = plane::make(); }\n",
    )
    .expect("write facade caller");
    fs::write(
        temp.path().join("src/video_plane/plane.rs"),
        "mod camera_panel;\npub(super) use camera_panel::{StructCarrier, UnionCarrier, Widget, make};\n",
    )
    .expect("write restricted facade");
    fs::write(
        temp.path().join("src/video_plane/plane/camera_panel.rs"),
        "#[derive(Clone, Copy)]\npub(in crate::video_plane) struct Widget;\n\npub struct SignatureOnly;\n\npub(in crate::video_plane) struct StructCarrier { pub widget: Widget }\n\npub(in crate::video_plane) union UnionCarrier { pub widget: Widget }\n\npub(in crate::video_plane) fn make() -> (Widget, SignatureOnly, StructCarrier, UnionCarrier) {\n    (Widget, SignatureOnly, StructCarrier { widget: Widget }, UnionCarrier { widget: Widget })\n}\n",
    )
    .expect("write signature carrier");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            matches!(
                finding.code,
                DiagnosticCode::ForbiddenPubCrate | DiagnosticCode::ForbiddenPubInCrate
            ) && finding.path == "src/video_plane/plane/camera_panel.rs"
        }),
        "exact restricted signature reach should be accepted: {report:#?}",
    );
    let signature_only_findings = report
        .findings
        .iter()
        .filter(|finding| finding.item.as_deref() == Some("struct SignatureOnly"))
        .collect::<Vec<_>>();
    assert_eq!(
        signature_only_findings.len(),
        1,
        "equal signature exposure must suppress unused_pub: {report:#?}",
    );
    let signature_only_finding = signature_only_findings[0];
    assert_eq!(signature_only_finding.code, DiagnosticCode::SuspiciousPub);
    assert!(
        signature_only_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::video_plane`, or add an explicit facade at `crate::video_plane` and rerun `cargo mend`"
        }),
        "equal exposure advice must name its signature boundary: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_in_crate",
        "src/video_plane/plane/camera_panel.rs",
        2,
        ForbiddenVisibilityPersistenceExpectation::AcceptedRestricted {
            visibility_annotation:      "pub(in crate::video_plane)",
            item_def_path:              "video_plane::plane::camera_panel::Widget",
            item_module_def_path:       "video_plane::plane::camera_panel",
            required_boundary_def_path: "crate::video_plane",
        },
    );
}

#[test]
fn crate_wide_signature_requirement_is_persisted_without_rendering_a_finding() {
    let temp = tempdir().expect("create accepted crate reach fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "accepted_crate_signature_fixture"
version = "0.1.0"
edition = "2024"

[lib]
test = false
doctest = false
bench = false
"#,
            ),
            (
                "src/lib.rs",
                "mod a;\npub(crate) use a::Target;\npub(crate) fn make() -> Target { Target }\n",
            ),
            ("src/a.rs", "pub(crate) struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            matches!(
                finding.code,
                DiagnosticCode::ForbiddenPubCrate | DiagnosticCode::ForbiddenPubInCrate
            )
        }),
        "accepted evidence must not render: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_crate",
        "src/a.rs",
        1,
        ForbiddenVisibilityPersistenceExpectation::AcceptedCrate {
            visibility_annotation: "pub(crate)",
            item_def_path:         "a::Target",
            item_module_def_path:  "a",
        },
    );
}

#[test]
fn wider_signature_exposure_names_why_an_exact_facade_annotation_must_change() {
    let temp = tempdir().expect("create wider signature fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "wider_signature_exact_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub use a::expose;\n"),
            ("src/a.rs", "mod b;\npub use b::expose;\n"),
            (
                "src/a/b.rs",
                "mod c;\npub(super) use c::Target;\npub use c::expose;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(in crate::a) struct Target;\npub fn expose() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs" && finding.code == DiagnosticCode::ForbiddenPubInCrate
        })
        .unwrap_or_else(|| panic!("missing exact-facade signature finding: {report:#?}"));
    assert_eq!(
        target_finding.headline,
        "signature exposure requires the wider `pub` annotation"
    );
    assert!(
        target_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub`"),
        "the signature requirement and joined suggestion must agree: {report:#?}",
    );
}

#[test]
fn narrower_signature_exposure_keeps_the_facade_boundary_required() {
    let temp = tempdir().expect("create narrower exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "narrower_signature_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::exercise(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn exercise() { b::exercise(); let _ = b::Target; }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\nmod d;\npub(super) use c::Target;\npub(self) use d::make;\npub(super) fn exercise() { let _ = make(); }\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(in crate::a) struct Target;\npub struct SignatureOnly;\npub struct UnusedControl;\n",
            ),
            (
                "src/a/b/d.rs",
                "pub(super) fn make() -> (super::c::Target, super::c::SignatureOnly) {\n    (super::c::Target, super::c::SignatureOnly)\n}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.path == "src/a/b/c.rs"
                && matches!(
                    finding.code,
                    DiagnosticCode::ForbiddenPubCrate | DiagnosticCode::ForbiddenPubInCrate
                )
        }),
        "the wider facade boundary must remain the required reach: {report:#?}",
    );
    assert!(
        !report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct SignatureOnly")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "narrower signature exposure must suppress unused_pub: {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct UnusedControl")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the unexposed control must retain unused_pub: {report:#?}",
    );
}

#[test]
fn sibling_signature_exposure_joins_at_the_common_ancestor() {
    let temp = tempdir().expect("create sibling exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "sibling_signature_exposure_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::exercise(); }\n"),
            (
                "src/a.rs",
                "mod left;\nmod right;\npub(crate) fn exercise() { right::exercise(); }\n",
            ),
            (
                "src/a/left.rs",
                "pub(in crate::a) struct Target;\npub struct UnusedControl;\n",
            ),
            (
                "src/a/right.rs",
                "pub(super) fn make() -> super::left::Target { super::left::Target }\npub(super) fn exercise() { let _ = make(); }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/left.rs" && finding.code == DiagnosticCode::ForbiddenPubInCrate
        })
        .expect("find sibling-exposed target finding");
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "the sibling signature must anchor its common-ancestor reach at crate::a: {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct UnusedControl")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the unexposed sibling control must retain unused_pub: {report:#?}",
    );
}

#[test]
fn sibling_impl_surface_uses_the_containing_module_reach() {
    let temp = tempdir().expect("create sibling impl exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "sibling_impl_signature_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            ("src/a.rs", "mod b;\npub(crate) fn run() {}\n"),
            ("src/a/b.rs", "pub(super) mod sibling;\nmod target;\n"),
            (
                "src/a/b/sibling.rs",
                "pub(in crate::a) struct Carrier;\nimpl Carrier { pub(in crate::a) fn expose() -> super::target::Target { super::target::Target } }\n",
            ),
            (
                "src/a/b/target.rs",
                "pub struct Target;\npub struct ControlTarget;\nstruct LocalCarrier;\nimpl LocalCarrier { pub(in crate::a) fn expose_control() -> ControlTarget { ControlTarget } }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| finding.item.as_deref() == Some("struct Target"))
        .unwrap_or_else(|| panic!("missing sibling-impl target finding: {report:#?}"));
    assert_eq!(target_finding.code, DiagnosticCode::SuspiciousPub);
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the sibling impl must require the containing module's crate::a reach: {report:#?}",
    );
    let control_findings = report
        .findings
        .iter()
        .filter(|finding| finding.item.as_deref() == Some("struct ControlTarget"))
        .collect::<Vec<_>>();
    assert_eq!(
        control_findings.len(),
        1,
        "the same-module impl control must have one finding: {report:#?}",
    );
    assert_eq!(control_findings[0].code, DiagnosticCode::UnusedPub);
}

#[test]
fn raw_identifier_impl_signature_carriers_resolve_to_hir_identity() {
    let temp = tempdir().expect("create raw impl identifier fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "raw_impl_identifier_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            ("src/a.rs", "mod b;\npub(crate) fn run() {}\n"),
            ("src/a/b.rs", "pub(super) mod sibling;\nmod target;\n"),
            (
                "src/a/b/sibling.rs",
                r#"pub(in crate::a) struct Carrier;

impl Carrier {
    pub(in crate::a) fn r#type(
        _: super::target::RawParameter,
    ) -> super::target::RawReturn {
        super::target::RawReturn
    }

    pub(in crate::a) fn expose(
        _: super::target::NonRawParameter,
    ) -> super::target::NonRawReturn {
        super::target::NonRawReturn
    }
}
"#,
            ),
            (
                "src/a/b/target.rs",
                r#"pub struct RawParameter;
pub struct RawReturn;
pub struct NonRawParameter;
pub struct NonRawReturn;
"#,
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for (item, carrier_spelling) in [
        ("struct RawParameter", "raw"),
        ("struct RawReturn", "raw"),
        ("struct NonRawParameter", "non-raw"),
        ("struct NonRawReturn", "non-raw"),
    ] {
        let target_findings = report
            .findings
            .iter()
            .filter(|finding| finding.item.as_deref() == Some(item))
            .collect::<Vec<_>>();
        assert_eq!(
            target_findings.len(),
            1,
            "missing {carrier_spelling} impl signature target {item}: {report:#?}",
        );
        let target_finding = target_findings[0];
        assert_eq!(target_finding.code, DiagnosticCode::SuspiciousPub);
        assert_eq!(target_finding.fix_support, FixSupport::None);
        assert!(
            target_finding.help.iter().any(|line| {
                line
                    == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
            }),
            "{carrier_spelling} impl signature target must retain crate::a reach: {report:#?}",
        );
        assert!(
            target_finding
                .help
                .iter()
                .all(|line| !line.contains("remov") && !line.contains("pub(super)")),
            "{carrier_spelling} impl signature target must not receive narrowing advice: {report:#?}",
        );
    }
}

#[test]
fn qualified_and_aliased_impl_self_types_use_compiler_identity() {
    let temp = tempdir().expect("create impl self identity fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "impl_self_identity_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::QualifiedCarrier::expose(); let _ = b::ImportedCarrier::expose(); let _ = b::OtherCarrier::expose(); }\n",
            ),
            (
                "src/a/b.rs",
                "pub(super) mod carrier;\npub(super) mod impls;\nmod target;\npub(super) use carrier::{Carrier as OtherCarrier, ImportedCarrier, QualifiedCarrier};\n",
            ),
            (
                "src/a/b/carrier.rs",
                "pub(in crate::a) struct QualifiedCarrier;\npub(in crate::a) struct ImportedCarrier;\npub(in crate::a) struct Carrier;\n",
            ),
            (
                "src/a/b/impls.rs",
                r#"impl crate::a::b::carrier::QualifiedCarrier {
    pub(in crate::a) fn expose() -> super::target::QualifiedTarget {
        super::target::QualifiedTarget
    }
}

use super::carrier::ImportedCarrier as CarrierAlias;

impl CarrierAlias {
    pub(in crate::a) fn expose() -> super::target::ImportedTarget {
        super::target::ImportedTarget
    }
}

impl crate::a::b::carrier::Carrier {
    pub(in crate::a) fn expose() -> super::target::Carrier {
        super::target::Carrier
    }
}
"#,
            ),
            (
                "src/a/b/target.rs",
                "pub struct QualifiedTarget;\npub struct ImportedTarget;\npub struct Carrier;\npub struct UnusedControl;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for target in [
        "struct QualifiedTarget",
        "struct ImportedTarget",
        "struct Carrier",
    ] {
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.item.as_deref() == Some(target))
            .unwrap_or_else(|| panic!("missing {target} identity finding: {report:#?}"));
        assert_eq!(finding.code, DiagnosticCode::SuspiciousPub);
        assert_eq!(finding.fix_support, FixSupport::None);
        assert!(
            finding.help.iter().any(|line| {
                line
                    == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
            }),
            "{target} must retain exact crate::a signature reach: {report:#?}",
        );
        assert!(
            finding.help.iter().all(|line| {
                line != "this item is exposed through a public signature; consider using `pub`"
            }),
            "{target} must not exceed crate::a reach: {report:#?}",
        );
    }
    assert!(
        report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct UnusedControl")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the unrelated control must remain unused: {report:#?}",
    );
}

#[test]
fn parent_boundary_impl_surfaces_preserve_method_self_and_trait_caps() {
    let temp = tempdir().expect("create parent boundary impl fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "parent_boundary_impl_surface_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "pub mod a;\npub use a::TraitCarrier;\n"),
            (
                "src/a.rs",
                "mod b;\npub use b::{Carrier, TraitCarrier};\npub fn run() { let _ = Carrier::expose(); b::exercise_private(); }\n",
            ),
            (
                "src/a/b.rs",
                r#"mod c;

pub struct Carrier;

impl Carrier {
    pub(super) fn expose() -> c::InherentTarget { c::InherentTarget }
    fn private_expose() -> c::PrivateTarget { c::PrivateTarget }
}

pub(super) fn exercise_private() { let _ = Carrier::private_expose(); }

pub struct TraitCarrier;
pub(super) trait RestrictedContract { type Output; }
impl RestrictedContract for TraitCarrier { type Output = c::TraitTarget; }
"#,
            ),
            (
                "src/a/b/c.rs",
                "pub(crate) struct InherentTarget;\npub(crate) struct TraitTarget;\npub(crate) struct PrivateTarget;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for (line_start, carrier) in [(1, "inherent method"), (2, "restricted trait")] {
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.path == "src/a/b/c.rs"
                    && finding.code == DiagnosticCode::ForbiddenPubCrate
                    && finding.line_start == line_start
            })
            .unwrap_or_else(|| panic!("missing {carrier} target finding: {report:#?}"));
        assert_eq!(finding.fix_support, FixSupport::None);
        assert!(
            finding.help.iter().any(|line| {
                line
                    == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
            }),
            "the {carrier} must require exact crate::a reach: {report:#?}",
        );
        assert!(
            finding.help.iter().all(|line| {
                line != "this item is exposed through a public signature; consider using `pub`"
            }),
            "the {carrier} must remain capped below public reach: {report:#?}",
        );
    }

    let private_control = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs"
                && finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.line_start == 3
        })
        .unwrap_or_else(|| panic!("missing private method control: {report:#?}"));
    assert!(
        private_control.help.iter().any(|line| {
            line == "consider using `pub(super)` or removing `pub(crate)` entirely"
        }),
        "the private method must contribute no outward reach: {report:#?}",
    );
}

#[test]
fn private_ancestor_caps_sibling_module_path_exposure() {
    let temp = tempdir().expect("create module path cap fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "module_path_private_ancestor_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::make(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\nmod carrier;\npub(super) use carrier::make;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(crate) struct Target;\npub struct SignatureOnly;\npub struct UnusedControl;\n",
            ),
            (
                "src/a/b/carrier.rs",
                "pub fn make() -> (super::c::Target, super::c::SignatureOnly) {\n    (super::c::Target, super::c::SignatureOnly)\n}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs" && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .expect("find capped module-path exposure finding");
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the parent facade must preserve the crate::a signature boundary: {report:#?}",
    );
    assert!(
        target_finding
            .help
            .iter()
            .all(|line| !line.contains("consider using `pub(super)`")),
        "pub(super) would be below the crate::a signature boundary: {report:#?}",
    );
    assert!(
        !report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct SignatureOnly")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the sibling signature must suppress unused_pub: {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct UnusedControl")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the unexposed sibling control must retain unused_pub: {report:#?}",
    );
}

#[test]
fn private_ancestor_caps_parent_boundary_exposure() {
    let temp = tempdir().expect("create parent boundary cap fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "parent_boundary_private_ancestor_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::make(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub fn make() -> c::Target { c::Target }\n",
            ),
            ("src/a/b/c.rs", "pub struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| finding.item.as_deref() == Some("struct Target"))
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "parent-boundary exposure must suppress unused_pub without becoming public: {report:#?}",
    );
    assert_eq!(target_findings[0].code, DiagnosticCode::SuspiciousPub);
    assert_eq!(target_findings[0].fix_support, FixSupport::None);
    assert!(
        target_findings[0].help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the private b module must cap make at crate::a: {report:#?}",
    );
}

#[test]
fn private_ancestor_caps_public_reexport_fallback() {
    let temp = tempdir().expect("create public re-export cap fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "public_reexport_private_ancestor_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            ("src/a.rs", "mod b;\nmod hidden;\npub(crate) fn run() {}\n"),
            (
                "src/a/b.rs",
                "mod c;\n#[derive(Default)]\npub struct Carrier { pub target: c::Target }\n",
            ),
            (
                "src/a/b/c.rs",
                "#[derive(Default)]\npub struct Target;\npub struct UnusedControl;\n",
            ),
            ("src/a/hidden.rs", "pub use super::b::*;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| finding.item.as_deref() == Some("struct Target"))
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "the capped public re-export must preserve a restricted signature reach: {report:#?}",
    );
    assert_eq!(target_findings[0].code, DiagnosticCode::SuspiciousPub);
    assert_eq!(target_findings[0].fix_support, FixSupport::None);
    assert!(
        target_findings[0].help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the private hidden module must cap its public re-export at crate::a: {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct UnusedControl")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the unexposed control must retain unused_pub: {report:#?}",
    );
}

#[test]
fn named_exported_module_gives_only_its_public_descendants_external_reach() {
    let temp = tempdir().expect("create exported ancestor module fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "exported_ancestor_module_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod hidden;\nmod same_name;\npub use hidden::api;\npub(crate) fn retain_controls() { let _ = hidden::api::restricted::make(); let _ = same_name::api::make(); }\n",
            ),
            ("src/hidden.rs", "pub mod api;\n"),
            (
                "src/hidden/api.rs",
                "mod public_target;\npub(crate) mod restricted;\nuse public_target::PublicTarget;\npub fn make() -> PublicTarget { PublicTarget }\n",
            ),
            (
                "src/hidden/api/public_target.rs",
                "pub(crate) struct PublicTarget;\n",
            ),
            (
                "src/hidden/api/restricted.rs",
                "mod target;\nuse target::RestrictedTarget;\npub(crate) fn make() -> RestrictedTarget { RestrictedTarget }\n",
            ),
            (
                "src/hidden/api/restricted/target.rs",
                "pub(crate) struct RestrictedTarget;\n",
            ),
            ("src/same_name.rs", "pub mod api;\n"),
            (
                "src/same_name/api.rs",
                "mod target;\nuse target::SameNameTarget;\npub fn make() -> SameNameTarget { SameNameTarget }\n",
            ),
            (
                "src/same_name/api/target.rs",
                "pub(crate) struct SameNameTarget;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = |path: &str| {
        report
            .findings
            .iter()
            .find(|finding| {
                finding.path == path && finding.code == DiagnosticCode::ForbiddenPubCrate
            })
            .unwrap_or_else(|| panic!("missing signature target finding at {path}: {report:#?}"))
    };
    assert!(
        target_finding("src/hidden/api/public_target.rs")
            .help
            .iter()
            .any(|line| {
                line == "this item is exposed through a public signature; consider using `pub`"
            }),
        "the resolved hidden::api export must give its public make function external reach: {report:#?}",
    );
    for path in [
        "src/hidden/api/restricted/target.rs",
        "src/same_name/api/target.rs",
    ] {
        let finding = target_finding(path);
        assert!(
            finding.help.iter().all(|line| {
                line != "this item is exposed through a public signature; consider using `pub`"
            }),
            "restricted descendants and the same-name module must remain below public reach at {path}: {report:#?}",
        );
        assert!(
            finding.help.iter().any(|line| {
                line
                    == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
            }),
            "the restricted or unexported path must retain only crate reach at {path}: {report:#?}",
        );
    }
}

#[test]
fn outer_glob_hop_widens_nested_signature_reach() {
    let temp = tempdir().expect("create outer glob signature fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "outer_glob_signature_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\nmod hidden;\npub use hidden::*;\npub(crate) fn run() { let _ = std::mem::size_of::<Carrier>(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\n#[derive(Default)]\npub struct Carrier { pub target: c::Target }\n",
            ),
            (
                "src/a/b/c.rs",
                "#[derive(Default)]\npub(in crate::a) struct Target;\npub struct UnusedControl;\n",
            ),
            ("src/a/hidden.rs", "pub use super::b::*;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate && finding.path == "src/a/b/c.rs"
        })
        .unwrap_or_else(|| panic!("missing outer-glob target finding: {report:#?}"));
    assert_eq!(target_finding.code, DiagnosticCode::ForbiddenPubInCrate);
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
        }),
        "the outer glob must widen the nested signature requirement to pub(crate): {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct UnusedControl")
                && finding.code == DiagnosticCode::UnusedPub
        }),
        "the unrelated nested control must retain unused_pub: {report:#?}",
    );
}

#[test]
fn restricted_glob_child_does_not_give_signature_type_public_reach() {
    let temp = tempdir().expect("create restricted glob child fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "restricted_glob_child_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod api;\npub use api::outward::*;\npub(crate) fn retain(_: api::Carrier) {}\n",
            ),
            (
                "src/api.rs",
                "mod carrier;\npub mod outward { pub(crate) use super::carrier::Carrier; }\npub(crate) use carrier::Carrier;\n",
            ),
            (
                "src/api/carrier.rs",
                "mod target;\nuse target::Target;\n#[derive(Default)]\npub struct Carrier { pub target: Target }\n",
            ),
            (
                "src/api/carrier/target.rs",
                "#[derive(Default)]\npub(crate) struct Target;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| {
            finding.path == "src/api/carrier/target.rs"
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "the nested signature type must have one finding: {report:#?}",
    );
    assert!(
        target_findings[0].help.iter().any(|line| {
            line
                == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
        }),
        "the restricted carrier must cap its nested signature type at crate reach: {report:#?}",
    );
    assert_eq!(target_findings[0].fix_support, FixSupport::None);
    assert!(
        target_findings[0]
            .help
            .iter()
            .all(|line| !line.contains("crate-external")),
        "the nested signature type must not receive public-reach advice: {report:#?}",
    );
}

#[test]
fn direct_public_glob_uses_each_exact_childs_effective_reach() {
    let temp = tempdir().expect("create direct public glob fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "direct_public_glob_child_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod container;\nmod facade;\npub use facade::*;\n",
            ),
            (
                "src/facade.rs",
                "pub(crate) use crate::container::carrier::RestrictedCarrier;\npub use crate::container::carrier::PublicCarrier;\n",
            ),
            ("src/container.rs", "pub(crate) mod carrier;\n"),
            (
                "src/container/carrier.rs",
                "mod targets;\nuse targets::{PublicTarget, RestrictedTarget};\n#[derive(Default)]\npub struct RestrictedCarrier { pub target: RestrictedTarget }\n#[derive(Default)]\npub struct PublicCarrier { pub target: PublicTarget }\n",
            ),
            (
                "src/container/carrier/targets.rs",
                "#[derive(Default)]\npub(crate) struct RestrictedTarget;\n#[derive(Default)]\npub struct PublicTarget;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let restricted_targets = report
        .findings
        .iter()
        .filter(|finding| {
            finding.path == "src/container/carrier/targets.rs"
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .collect::<Vec<_>>();
    assert_eq!(
        restricted_targets.len(),
        1,
        "the restricted nested target must have one finding: {report:#?}",
    );
    let restricted_target = restricted_targets[0];
    assert!(
        restricted_target.help.iter().any(|line| {
            line
                == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
        }),
        "the restricted exact glob child must cap its signature type at crate reach: {report:#?}",
    );
    assert!(
        restricted_target.help.iter().all(|line| {
            line != "consider using: `pub(super)`" && !line.contains("public signature")
        }),
        "the restricted exact glob child must not provide public signature reach: {report:#?}",
    );

    assert!(
        !report.findings.iter().any(|finding| {
            finding.item.as_deref() == Some("struct PublicTarget")
                && matches!(
                    finding.code,
                    DiagnosticCode::SuspiciousPub | DiagnosticCode::UnusedPub
                )
        }),
        "the public sibling must remain accepted through its public signature reach: {report:#?}",
    );
}

#[test]
fn shadowed_public_glob_does_not_expose_signature_types() {
    let temp = tempdir().expect("create shadowed public glob fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "shadowed_public_glob_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = std::mem::size_of::<b::Carrier>(); let _ = std::mem::size_of::<b::ControlCarrier>(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod control_target;\nmod original;\nmod target;\npub struct Carrier;\npub use original::*;\n",
            ),
            (
                "src/a/b/original.rs",
                "use super::control_target::ControlTarget;\nuse super::target::Target;\n#[derive(Default)]\npub struct Carrier { pub target: Target }\n#[derive(Default)]\npub struct ControlCarrier { pub target: ControlTarget }\n",
            ),
            (
                "src/a/b/target.rs",
                "#[derive(Default)]\npub struct Target;\n",
            ),
            (
                "src/a/b/control_target.rs",
                "#[derive(Default)]\npub struct ControlTarget;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for (path, item) in [
        ("src/a/b/original.rs", "struct Carrier"),
        ("src/a/b/target.rs", "struct Target"),
    ] {
        assert!(
            !report.findings.iter().any(|finding| {
                finding.path == path
                    && finding.item.as_deref() == Some(item)
                    && matches!(
                        finding.code,
                        DiagnosticCode::SuspiciousPub
                            | DiagnosticCode::ForbiddenPubCrate
                            | DiagnosticCode::ForbiddenPubInCrate
                    )
            }),
            "the used same-name declaration must not give the shadowed {item} outward reach: {report:#?}",
        );
    }
    let control_target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/control_target.rs"
                && finding.item.as_deref() == Some("struct ControlTarget")
        })
        .unwrap_or_else(|| panic!("missing valid glob control finding: {report:#?}"));
    assert_eq!(control_target_finding.code, DiagnosticCode::SuspiciousPub);
    assert_eq!(control_target_finding.fix_support, FixSupport::None);
    assert!(
        control_target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the valid used glob must contribute its crate::a reach: {report:#?}",
    );
}

#[test]
fn used_parent_facade_reach_joins_independent_public_globs() {
    let temp = tempdir().expect("create joined facade routes fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "joined_facade_routes_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/main.rs",
                "mod a;\npub use a::public_api::*;\nfn main() { a::run(); }\n",
            ),
            (
                "src/a.rs",
                "mod b;\npub mod public_api { pub use super::b::outward::*; }\npub(crate) fn run() { let _ = std::mem::size_of::<b::Carrier>(); let _ = std::mem::size_of::<b::RestrictedCarrier>(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod exported;\nmod restricted;\npub mod outward { pub use super::exported::*; }\npub(in crate::a) use exported::Carrier;\npub(in crate::a) use restricted::RestrictedCarrier;\n",
            ),
            (
                "src/a/b/exported.rs",
                "mod target;\nuse target::Target;\n#[derive(Default)]\npub struct Carrier { pub target: Target }\n",
            ),
            (
                "src/a/b/exported/target.rs",
                "#[derive(Default)]\npub(in crate::a) struct Target;\n",
            ),
            (
                "src/a/b/restricted.rs",
                "mod target;\nuse target::RestrictedTarget;\n#[derive(Default)]\npub struct RestrictedCarrier { pub target: RestrictedTarget }\n",
            ),
            (
                "src/a/b/restricted/target.rs",
                "#[derive(Default)]\npub(in crate::a) struct RestrictedTarget;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/exported/target.rs"
                && finding.code == DiagnosticCode::ForbiddenPubInCrate
        })
        .unwrap_or_else(|| panic!("missing independently exported target finding: {report:#?}"));
    assert_eq!(target_finding.code, DiagnosticCode::ForbiddenPubInCrate);
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert_eq!(
        target_finding.headline,
        "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy"
    );
    assert!(
        target_finding.help.iter().any(|line| {
            line == "this item is exposed through a public signature; consider using `pub`"
        }),
        "the independent public glob must require bare pub: {report:#?}",
    );
    let restricted_target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/restricted/target.rs"
                && finding.code == DiagnosticCode::ForbiddenPubInCrate
        })
        .unwrap_or_else(|| panic!("missing restricted-only control finding: {report:#?}"));
    assert_eq!(restricted_target_finding.fix_support, FixSupport::None);
    assert!(
        restricted_target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the used restricted facade alone must retain its crate::a reach: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_in_crate",
        "src/a/b/exported/target.rs",
        2,
        ForbiddenVisibilityPersistenceExpectation::Public {
            visibility_annotation: "pub(in crate::a)",
            item_def_path:         "a::b::exported::target::Target",
        },
    );
    assert_no_stored_pub_use_fix_facts(&temp);
}

#[test]
fn public_signature_exposure_still_requires_bare_pub() {
    let temp = tempdir().expect("create public exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "public_signature_exposure_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub use a::make;\n"),
            ("src/a.rs", "mod b;\npub use b::make;\n"),
            ("src/a/b.rs", "mod c;\npub use c::make;\n"),
            (
                "src/a/b/c.rs",
                "pub(crate) struct Target;\npub fn make() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs" && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .expect("find public signature exposure finding");
    assert!(
        finding
            .help
            .iter()
            .any(|line| line.contains("consider using `pub`")),
        "public exposure must require bare pub: {report:#?}",
    );
}

#[test]
fn resolved_facade_requirements_with_and_without_signature_are_persisted() {
    let temp = tempdir().expect("create resolved-facade persistence fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "resolved_facade_persistence_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub fn run() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::make(); b::helper(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub(super) use c::{helper, make, Target};\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(crate) struct Target;\npub(in crate::a) fn make() -> Target { Target }\npub(crate) fn helper() {}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs"
                && finding.line_start == 1
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .unwrap_or_else(|| panic!("missing resolved-facade pub(crate) finding: {report:#?}"));
    assert!(
        target_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(in crate::a)`"),
        "resolved-facade advice must keep the legal exact boundary: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_crate",
        "src/a/b/c.rs",
        1,
        ForbiddenVisibilityPersistenceExpectation::ResolvedFacadeRestricted {
            visibility_annotation:      "pub(crate)",
            item_def_path:              "a::b::c::Target",
            item_module_def_path:       "a::b::c",
            required_boundary_def_path: "crate::a",
        },
    );
    let helper_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs"
                && finding.line_start == 3
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .unwrap_or_else(|| panic!("missing facade-only pub(crate) finding: {report:#?}"));
    assert!(
        helper_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(in crate::a)`"),
        "resolved-facade advice without a signature floor must keep the legal exact boundary: \
         {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_crate",
        "src/a/b/c.rs",
        3,
        ForbiddenVisibilityPersistenceExpectation::ResolvedFacadeRestricted {
            visibility_annotation:      "pub(crate)",
            item_def_path:              "a::b::c::helper",
            item_module_def_path:       "a::b::c",
            required_boundary_def_path: "crate::a",
        },
    );
}

#[test]
fn public_signature_precedes_an_unresolvable_parent_facade_blocker() {
    let temp = tempdir().expect("create public blocked-facade fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "public_signature_blocked_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub use a::make;\n"),
            (
                "src/a.rs",
                "mod b;\npub use b::make;\npub(crate) use b::*;\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub use c::make;\npub(crate) use c::Target;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(crate) struct Target;\npub fn make() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs"
                && finding.line_start == 1
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .unwrap_or_else(|| panic!("missing public blocked-facade finding: {report:#?}"));
    assert_eq!(
        target_finding.headline,
        "use of `pub(crate)` is forbidden by policy"
    );
    assert!(
        target_finding.help.iter().any(|line| {
            line == "this item is exposed through a public signature; consider using `pub`"
        }),
        "public signature advice must require bare pub: {report:#?}",
    );
    assert!(
        target_finding
            .help
            .iter()
            .all(|line| !line.contains("replace it with an explicit re-export")),
        "public signature advice must bypass the facade blocker: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_crate",
        "src/a/b/c.rs",
        1,
        ForbiddenVisibilityPersistenceExpectation::Public {
            visibility_annotation: "pub(crate)",
            item_def_path:         "a::b::c::Target",
        },
    );
}

#[test]
fn restricted_signature_retains_an_unresolvable_parent_facade_blocker() {
    let temp = tempdir().expect("create restricted blocked-facade fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "restricted_signature_blocked_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub(crate) use a::make;\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) use b::make;\npub(crate) use b::*;\n",
            ),
            ("src/a/b.rs", "mod c;\npub(crate) use c::{make, Target};\n"),
            (
                "src/a/b/c.rs",
                "pub(crate) struct Target;\npub(crate) fn make() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs"
                && finding.line_start == 1
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .unwrap_or_else(|| panic!("missing restricted blocked-facade finding: {report:#?}"));
    assert!(
        target_finding.help.iter().any(|line| {
            line == "facade at a.rs:3 uses `*`; replace it with an explicit re-export"
        }),
        "restricted signature advice must retain the facade blocker: {report:#?}",
    );
    assert!(
        target_finding.help.iter().all(|line| {
            line != "this item is exposed through a public signature; consider using `pub`"
        }),
        "restricted signature advice must not require public reach: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_crate",
        "src/a/b/c.rs",
        1,
        ForbiddenVisibilityPersistenceExpectation::StructuralBlocker {
            visibility_annotation: "pub(crate)",
            item_def_path:         "a::b::c::Target",
        },
    );
}

#[test]
fn required_mode_reviews_bare_pub_on_a_restricted_exported_self_type() {
    let temp = tempdir().expect("create associated exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "associated_signature_exposure_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"required\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let value = b::Widget; value.activate(); }\n",
            ),
            ("src/a/b.rs", "mod c;\npub(super) use c::Widget;\n"),
            (
                "src/a/b/c.rs",
                "pub(in crate::a) struct Widget;\nimpl Widget { pub fn activate(&self) {} }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let method_codes = report
        .findings
        .iter()
        .filter(|finding| {
            finding.path == "src/a/b/c.rs" && finding.item.as_deref() == Some("fn activate")
        })
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert_eq!(
        method_codes,
        vec![DiagnosticCode::SuspiciousPub],
        "restricted self-type exposure must not excuse bare pub: {report:#?}",
    );
    let method_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/c.rs"
                && finding.item.as_deref() == Some("fn activate")
                && finding.code == DiagnosticCode::SuspiciousPub
        })
        .expect("find associated method visibility finding");
    assert_eq!(
        method_finding.headline,
        "`pub` is broader than this nested module boundary"
    );
    assert!(
        method_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(in crate::a)`"),
        "unexpected associated method help: {report:#?}",
    );
}

#[test]
fn same_line_impl_methods_keep_distinct_signature_reaches() {
    let temp = tempdir().expect("create same-line impl fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "same_line_impl_signature_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let wide = b::Wide; let _ = wide.expose(); b::use_narrow(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub(super) use c::Wide;\npub(self) use c::Narrow;\npub(super) fn use_narrow() { let narrow = Narrow; let _ = narrow.expose(); }\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(in crate::a::b) struct Narrow; pub(in crate::a) struct Wide; pub(crate) struct NarrowTarget; pub(crate) struct WideTarget;\n\t/* λ */ impl Narrow { pub(super) fn expose(&self) -> NarrowTarget { NarrowTarget } }\timpl Wide { pub(in crate::a) fn expose(&self) -> WideTarget { WideTarget } }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
        })
        .collect::<Vec<_>>();
    assert_eq!(target_findings.len(), 2, "target findings: {report:#?}");
    assert!(
        target_findings.iter().any(|finding| {
            finding
                .help
                .iter()
                .any(|line| line == "consider using: `pub(super)`")
        }),
        "the first expose method must retain its crate::a::b reach: {report:#?}",
    );
    assert!(
        target_findings.iter().any(|finding| {
            finding.help.iter().any(|line| {
                line
                    == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
            })
        }),
        "the second expose method must retain its crate::a reach: {report:#?}",
    );
    assert!(
        target_findings
            .iter()
            .all(|finding| finding.fix_support == FixSupport::None),
        "same-line target findings must remain structural: {report:#?}",
    );
}

#[test]
fn signature_exposure_does_not_admit_pub_in_without_a_facade() {
    let temp = tempdir().expect("create no-facade exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "signature_exposure_without_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::expose(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub(super) fn expose() -> c::Target { c::Target }\n",
            ),
            ("src/a/b/c.rs", "pub(in crate::a) struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| finding.path == "src/a/b/c.rs")
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "signature exposure without a facade must be rejected: {report:#?}",
    );
    let target_finding = target_findings[0];
    assert_eq!(target_finding.code, DiagnosticCode::ForbiddenPubInCrate);
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "floor-constrained pub(in ...) advice must be structural: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_in_crate",
        "src/a/b/c.rs",
        1,
        ForbiddenVisibilityPersistenceExpectation::Restricted {
            visibility_annotation:       "pub(in crate::a)",
            item_def_path:               "a::b::c::Target",
            item_module_def_path:        "a::b::c",
            signature_boundary_def_path: "crate::a",
        },
    );
}

fn assert_canonical_pub_in_spelling_respects_signature_reach(
    package_name: &str,
    annotation: &str,
    canonical_annotation: &str,
) {
    let temp = tempdir().expect("create canonical spelling signature fixture dir");
    write_minimal_manifest(&temp, package_name);
    let target_source = format!(
        "{annotation} struct Target;\npub(in crate::a) fn expose() -> Target {{ Target }}\n"
    );
    write_allowance_sources(
        &temp,
        &[
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() {}\n"),
            ("src/a.rs", "mod b;\npub(self) use b::expose;\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::expose;\n"),
            ("src/a/b/c.rs", &target_source),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate && finding.path == "src/a/b/c.rs"
        })
        .unwrap_or_else(|| panic!("missing canonical spelling finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "signature reach must replace canonical narrowing advice: {report:#?}",
    );
    assert!(
        !target_finding
            .help
            .iter()
            .any(|line| line == &format!("consider using: `{canonical_annotation}`")),
        "canonical advice must not narrow below the signature reach: {report:#?}",
    );
    assert_stored_forbidden_visibility_advice(
        &temp,
        "forbidden_pub_in_crate",
        "src/a/b/c.rs",
        1,
        ForbiddenVisibilityPersistenceExpectation::Restricted {
            visibility_annotation:       annotation,
            item_def_path:               "a::b::c::Target",
            item_module_def_path:        "a::b::c",
            signature_boundary_def_path: "crate::a",
        },
    );
}

#[test]
fn pub_in_super_canonicalization_respects_signature_reach() {
    assert_canonical_pub_in_spelling_respects_signature_reach(
        "pub_in_super_signature_reach_fixture",
        "pub(in super)",
        "pub(super)",
    );
}

#[test]
fn pub_in_self_canonicalization_respects_signature_reach() {
    assert_canonical_pub_in_spelling_respects_signature_reach(
        "pub_in_self_signature_reach_fixture",
        "pub(in self)",
        "pub(self)",
    );
}

#[test]
fn stale_facade_fix_is_disabled_when_signature_reach_exceeds_pub_super() {
    let temp = tempdir().expect("create stale facade signature fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_facade_wider_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::make(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\nmod carrier;\npub(super) use c::Target;\npub(super) use carrier::make;\n",
            ),
            ("src/a/b/c.rs", "pub struct Target;\n"),
            (
                "src/a/b/carrier.rs",
                "pub fn make() -> super::c::Target { super::c::Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("find stale-facade signature finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the wider signature must replace pub(super) fixer advice: {report:#?}",
    );
    assert_stored_finding_has_no_refinement_metadata(&temp, "suspicious_pub", "src/a/b/c.rs");
    assert_no_stored_pub_use_fix_facts(&temp);
}

#[test]
fn stale_restricted_facade_preserves_an_independent_signature_floor() {
    let temp = tempdir().expect("create stale restricted signature fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_restricted_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::make(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\nmod carrier;\npub(super) use c::Target;\npub(super) use carrier::make;\n",
            ),
            ("src/a/b/c.rs", "pub(in crate::a) struct Target;\n"),
            (
                "src/a/b/carrier.rs",
                "pub fn make() -> super::c::Target { super::c::Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("missing stale restricted signature finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the independent signature must preserve the crate::a floor: {report:#?}",
    );
    assert!(
        target_finding.help.iter().all(|line| {
            !line.contains("now-unneeded") && !line.contains("remove the parent facade")
        }),
        "the required annotation must not be described as removable: {report:#?}",
    );
}

#[test]
fn stale_public_facade_is_reviewed_when_joined_reach_equals_public() {
    let temp = tempdir().expect("create equal joined reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_facade_equal_joined_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\n"),
            (
                "src/a/b.rs",
                "mod child;\npub use child::Target as ExportedTarget;\n",
            ),
            (
                "src/a/b/child.rs",
                "pub struct Target;\npub(super) fn expose() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("missing equal-reach stale facade finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "stale facade cleanup must preserve the independent parent signature reach: {report:#?}",
    );
    assert!(
        target_finding
            .help
            .iter()
            .all(|line| line != "consider removing the visibility"),
        "stale facade cleanup must not narrow below the signature reach: {report:#?}",
    );
    assert_no_stored_pub_use_fix_facts(&temp);
}

#[test]
fn stale_facade_fix_is_disabled_when_a_retained_facade_exceeds_pub_super() {
    let temp = tempdir().expect("create retained facade reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_facade_retained_outer_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() {}\n"),
            ("src/a.rs", "mod b;\n"),
            (
                "src/a/b.rs",
                "pub(super) mod c;\npub(in crate::a) use c::d::Target;\n",
            ),
            ("src/a/b/c.rs", "pub(super) mod d;\npub use d::Target;\n"),
            ("src/a/b/c/d.rs", "pub struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("find retained-facade finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the retained outer facade must replace pub(super) fixer advice: {report:#?}",
    );
    assert_stored_finding_has_no_refinement_metadata(&temp, "suspicious_pub", "src/a/b/c/d.rs");
    assert_no_stored_pub_use_fix_facts(&temp);
}

#[test]
fn stale_restricted_facade_preserves_a_retained_outer_facade_requirement() {
    let temp = tempdir().expect("create restricted retained facade fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_restricted_retained_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() {}\n"),
            ("src/a.rs", "mod b;\n"),
            (
                "src/a/b.rs",
                "pub(super) mod c;\npub(in crate::a) use c::d::Target;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(super) mod d;\npub(in crate::a) use d::Target;\n",
            ),
            ("src/a/b/c/d.rs", "pub(in crate::a) struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("missing restricted retained-facade finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the retained outer facade must preserve the crate::a requirement: {report:#?}",
    );
    assert!(
        target_finding
            .help
            .iter()
            .all(|line| !line.contains("now-unneeded")),
        "the retained outer facade must keep the annotation required: {report:#?}",
    );
}

#[test]
fn stale_restricted_facade_joins_signature_and_retained_facade_reach() {
    let temp = tempdir().expect("create joined stale-facade reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_restricted_joined_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::c::make(); }\n",
            ),
            (
                "src/a/b.rs",
                "pub(super) mod c;\npub(in crate::a::b) use c::d::Target;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(super) mod d;\nmod carrier;\npub(in crate::a) use d::Target;\npub(in crate::a) use carrier::make;\n",
            ),
            ("src/a/b/c/d.rs", "pub(in crate::a) struct Target;\n"),
            (
                "src/a/b/c/carrier.rs",
                "pub fn make() -> super::d::Target { super::d::Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("missing joined stale-facade finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the signature and retained facade reaches must join at crate::a: {report:#?}",
    );
    assert!(
        target_finding
            .help
            .iter()
            .all(|line| !line.contains("now-unneeded")),
        "the joined reach must keep the annotation required: {report:#?}",
    );
}

#[test]
fn stale_restricted_facade_without_remaining_reach_keeps_safe_cleanup_advice() {
    let temp = tempdir().expect("create stale restricted cleanup control dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "stale_restricted_cleanup_control_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::Target;\n"),
            ("src/a/b/c.rs", "pub(in crate::a) struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Target")
        })
        .unwrap_or_else(|| panic!("missing stale restricted cleanup control: {report:#?}"));
    assert!(
        target_finding.help.iter().any(|line| {
            line == "remove the parent facade and the now-unneeded `pub(in crate::a)` annotation"
        }),
        "stale cleanup without an independent reach must keep the existing advice: {report:#?}",
    );
}

#[test]
fn stale_facade_in_unresolvable_chain_requires_structure() {
    let temp = tempdir().expect("create unresolvable stale facade fixture dir");
    write_minimal_manifest(&temp, "unresolvable_stale_facade_fixture");
    write_allowance_sources(
        &temp,
        &[
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() {}\n"),
            ("src/a.rs", "mod b;\npub use b::*;\n"),
            ("src/a/b.rs", "mod child;\npub use child::Thing;\n"),
            ("src/a/b/child.rs", "pub struct Thing;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("struct Thing")
        })
        .unwrap_or_else(|| panic!("missing unresolvable stale facade finding: {report:#?}"));
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding
            .help
            .iter()
            .any(|line| line.contains("uses `*`; replace it with an explicit re-export")),
        "unresolvable stale facade advice must identify the chain blocker: {report:#?}",
    );
    assert!(
        !target_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "unresolvable stale facade advice must not recommend pub(super): {report:#?}",
    );
    assert_stored_finding_has_no_refinement_metadata(&temp, "suspicious_pub", "src/a/b/child.rs");
    assert_no_stored_pub_use_fix_facts(&temp);
}

#[test]
fn no_facade_advice_does_not_narrow_below_the_signature_floor() {
    let temp = tempdir().expect("create no-facade advice fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "no_facade_signature_floor_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::expose(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub(super) fn expose() -> c::Target { c::Target }\n",
            ),
            ("src/a/b/c.rs", "pub struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| finding.item.as_deref() == Some("struct Target"))
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "signature exposure must suppress unused_pub: {report:#?}",
    );
    let target_finding = target_findings[0];
    assert_eq!(target_finding.code, DiagnosticCode::SuspiciousPub);
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "no-facade advice must use a structural outcome at crate::a: {report:#?}",
    );
    assert!(
        !target_finding.help.iter().any(|line| {
            matches!(
                line.as_str(),
                "consider using: `pub(super)`"
                    | "consider using: `pub(crate)`"
                    | "consider using: `pub(in crate::a)`"
            )
        }),
        "no-facade advice must not propose a visibility below the signature floor: {report:#?}",
    );
}

#[test]
fn same_module_parent_visible_signature_sets_the_parent_floor() {
    let temp = tempdir().expect("create same-module signature fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "same_module_parent_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\n"),
            (
                "src/a/b.rs",
                "pub(in crate::a) struct Target;\npub(in crate::a) struct UnexposedControl;\npub(super) fn make() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b.rs"
                && finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.headline.contains("pub(in crate::a)")
                && finding
                    .help
                    .iter()
                    .any(|line| line == "consider using: `pub(super)`")
        })
        .unwrap_or_else(|| panic!("missing same-module signature finding: {report:#?}"));
    assert!(
        target_finding
            .help
            .iter()
            .all(|line| line != "consider removing the visibility"),
        "the direct pub(super) signature must preserve the parent boundary: {report:#?}",
    );

    let control_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b.rs"
                && finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding
                    .help
                    .iter()
                    .any(|line| line == "consider removing the visibility")
        })
        .unwrap_or_else(|| panic!("missing unexposed same-module control: {report:#?}"));
    assert!(
        control_finding
            .help
            .iter()
            .all(|line| line != "consider using: `pub(super)`"),
        "the unexposed control must remain removable: {report:#?}",
    );
}

#[test]
fn trait_impl_signature_reach_is_capped_by_trait_visibility() {
    let temp = tempdir().expect("create trait reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "trait_impl_reach_cap_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "pub mod api;\n"),
            (
                "src/api.rs",
                "mod private_case;\nmod public_case;\nmod restricted_case;\npub use private_case::PrivateCarrier;\npub use public_case::PublicCarrier;\npub use restricted_case::RestrictedCarrier;\npub trait PublicContract { type Output; }\n",
            ),
            (
                "src/api/private_case.rs",
                "pub struct PrivateCarrier;\npub(in crate::api) struct PrivateSignature;\ntrait PrivateContract { type Output; }\nimpl PrivateContract for PrivateCarrier { type Output = PrivateSignature; }\n",
            ),
            (
                "src/api/restricted_case.rs",
                "pub struct RestrictedCarrier;\npub(in crate::api) struct RestrictedSignature;\npub(super) trait RestrictedContract { type Output; }\nimpl RestrictedContract for RestrictedCarrier { type Output = RestrictedSignature; }\n",
            ),
            (
                "src/api/public_case.rs",
                "pub struct PublicCarrier;\npub struct PublicSignature;\nimpl super::PublicContract for PublicCarrier { type Output = PublicSignature; }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let private_finding = report
        .findings
        .iter()
        .find(|finding| finding.path == "src/api/private_case.rs")
        .unwrap_or_else(|| panic!("missing private-trait finding: {report:#?}"));
    assert!(
        private_finding
            .help
            .iter()
            .any(|line| line == "consider removing the visibility"),
        "a private trait must not expose its associated signature type: {report:#?}",
    );

    let restricted_finding = report
        .findings
        .iter()
        .find(|finding| finding.path == "src/api/restricted_case.rs")
        .unwrap_or_else(|| panic!("missing restricted-trait finding: {report:#?}"));
    assert!(
        restricted_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "the restricted trait must expose its signature type only to crate::api: {report:#?}",
    );
    assert!(
        restricted_finding
            .help
            .iter()
            .all(|line| line != "consider removing the visibility"),
        "the restricted trait's signature boundary must survive refinement: {report:#?}",
    );

    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.path != "src/api/public_case.rs"),
        "the public trait must retain public signature reach: {report:#?}",
    );
}

#[test]
fn restricted_sibling_reexports_add_common_ancestor_signature_reach() {
    for (package_name, sibling_facades) in [
        (
            "restricted_sibling_reexport_first_fixture",
            "pub(super) use super::carriers::Carrier;\npub(self) use super::carriers::Carrier as PrivateCarrierAlias;\npub(super) use super::carriers::UnrelatedCarrier;\n",
        ),
        (
            "restricted_sibling_private_first_fixture",
            "pub(self) use super::carriers::Carrier as PrivateCarrierAlias;\npub(super) use super::carriers::Carrier;\npub(super) use super::carriers::UnrelatedCarrier;\n",
        ),
    ] {
        let temp = tempdir().expect("create restricted sibling reach fixture dir");
        write_allowance_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    &format!(
                        "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
                    ),
                ),
                ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
                ("src/lib.rs", "pub mod api;\n"),
                ("src/api.rs", "mod carriers;\nmod sibling;\n"),
                (
                    "src/api/carriers.rs",
                    "mod targets;\nuse targets::{RestrictedTarget, UnrelatedTarget};\npub struct Carrier { pub target: RestrictedTarget }\npub struct UnrelatedCarrier { pub target: UnrelatedTarget }\n",
                ),
                (
                    "src/api/carriers/targets.rs",
                    "pub(in crate::api) struct RestrictedTarget;\npub(in crate::api) struct PrivateTarget;\npub(in crate::api) struct UnrelatedTarget;\n",
                ),
                ("src/api/sibling.rs", sibling_facades),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        let target_finding = |line_start| {
            report
                .findings
                .iter()
                .find(|finding| {
                    finding.code == DiagnosticCode::ForbiddenPubInCrate
                        && finding.path == "src/api/carriers/targets.rs"
                        && finding.line_start == line_start
                })
                .unwrap_or_else(|| {
                    panic!("missing restricted sibling target at line {line_start}: {report:#?}")
                })
        };
        assert_eq!(
            target_finding(1).headline,
            "no visibility annotation allowed by policy preserves this item's current callers"
        );
        assert!(
            target_finding(1).help.iter().any(|line| {
                line
                    == "move the item into `crate::api`, or add an explicit facade at `crate::api` and rerun `cargo mend`"
            }),
            "the restricted sibling re-export must contribute crate::api reach: {report:#?}",
        );
        assert!(
            target_finding(2)
                .help
                .iter()
                .any(|line| line == "consider removing the visibility"),
            "the private-equivalent re-export must contribute no reach: {report:#?}",
        );
        assert!(
            target_finding(3).help.iter().any(|line| {
                line
                    == "move the item into `crate::api`, or add an explicit facade at `crate::api` and rerun `cargo mend`"
            }),
            "the unrelated carrier must retain only its own re-export reach: {report:#?}",
        );
    }
}

#[test]
fn local_trait_reexports_set_associated_signature_reach() {
    let temp = tempdir().expect("create trait re-export reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "trait_reexport_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "pub mod api;\n"),
            (
                "src/api.rs",
                "mod private_case;\nmod public_case;\nmod public_contract;\nmod restricted_case;\nmod restricted_contract;\npub use private_case::PrivateCarrier;\npub use public_case::PublicCarrier;\npub use public_contract::r#type as PublicContract;\n",
            ),
            (
                "src/api/private_case.rs",
                "pub struct PrivateCarrier;\npub(in crate::api) struct PrivateSignature;\ntrait PrivateContract { type Output; }\nimpl PrivateContract for PrivateCarrier { type Output = PrivateSignature; }\n",
            ),
            (
                "src/api/public_case.rs",
                "pub struct PublicCarrier;\npub struct PublicSignature;\nimpl super::PublicContract for PublicCarrier { type Output = PublicSignature; }\n",
            ),
            (
                "src/api/public_contract.rs",
                "pub trait r#type { type Output; }\n",
            ),
            (
                "src/api/restricted_case.rs",
                "pub(in crate::api) struct RestrictedCarrier;\npub(in crate::api) struct RestrictedSignature;\nimpl super::restricted_contract::RestrictedContract for RestrictedCarrier { type Output = RestrictedSignature; }\n",
            ),
            (
                "src/api/restricted_contract.rs",
                "mod nested { pub trait RestrictedContract { type Output; } }\npub(super) use nested::RestrictedContract;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().all(|finding| {
            !(finding.path == "src/api/public_case.rs"
                && finding.item.as_deref() == Some("struct PublicSignature"))
        }),
        "the public raw-identifier trait re-export must require and accept public reach: {report:#?}",
    );
    let restricted_signature = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path == "src/api/restricted_case.rs"
                && finding.line_start == 2
        })
        .unwrap_or_else(|| panic!("missing restricted trait signature finding: {report:#?}"));
    assert!(
        restricted_signature
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "the restricted trait re-export must contribute crate::api reach: {report:#?}",
    );
    let private_signature = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path == "src/api/private_case.rs"
                && finding.line_start == 2
        })
        .unwrap_or_else(|| panic!("missing private trait signature control: {report:#?}"));
    assert!(
        private_signature
            .help
            .iter()
            .any(|line| line == "consider removing the visibility"),
        "the unreexported trait must contribute no associated signature reach: {report:#?}",
    );
}

#[test]
fn private_equivalent_inherent_items_do_not_expose_signature_types() {
    let temp = tempdir().expect("create inherent item reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "inherent_item_private_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "pub mod api;\n"),
            (
                "src/api.rs",
                "mod in_self_case;\nmod parent_case;\nmod self_case;\npub use in_self_case::InSelfCarrier;\npub use parent_case::ParentCarrier;\npub use self_case::SelfCarrier;\n",
            ),
            (
                "src/api/self_case.rs",
                "pub struct SelfCarrier;\npub(in crate::api) struct SelfSignature;\nimpl SelfCarrier { pub(self) fn expose() -> SelfSignature { SelfSignature } }\n",
            ),
            (
                "src/api/in_self_case.rs",
                "pub struct InSelfCarrier;\npub(in crate::api) struct InSelfSignature;\nimpl InSelfCarrier { pub(in self) fn expose() -> InSelfSignature { InSelfSignature } }\n",
            ),
            (
                "src/api/parent_case.rs",
                "pub struct ParentCarrier;\npub(in crate::api) struct ParentSignature;\nimpl ParentCarrier { pub(super) fn expose() -> ParentSignature { ParentSignature } }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for private_path in ["src/api/self_case.rs", "src/api/in_self_case.rs"] {
        assert!(
            report.findings.iter().any(|finding| {
                finding.path == private_path
                    && finding
                        .help
                        .iter()
                        .any(|line| line == "consider removing the visibility")
            }),
            "{private_path} must receive no signature exposure: {report:#?}",
        );
    }

    let parent_finding = report
        .findings
        .iter()
        .find(|finding| finding.path == "src/api/parent_case.rs")
        .unwrap_or_else(|| panic!("missing visible inherent-item control: {report:#?}"));
    assert!(
        parent_finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "pub(super) must expose its signature type to crate::api: {report:#?}",
    );
    assert!(
        parent_finding
            .help
            .iter()
            .all(|line| line != "consider removing the visibility"),
        "the visible inherent-item control must retain its parent boundary: {report:#?}",
    );
}

#[test]
fn private_equivalent_sibling_declarations_do_not_add_signature_exposure() {
    let temp = tempdir().expect("create sibling declaration reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "sibling_declaration_private_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { b::right::exercise(); }\n",
            ),
            ("src/a/b.rs", "mod left;\npub(super) mod right;\n"),
            (
                "src/a/b/left.rs",
                "pub(crate) struct SelfSignature;\npub(crate) struct InSelfSignature;\npub(crate) struct OutwardSignature;\n",
            ),
            (
                "src/a/b/right.rs",
                "pub(self) fn self_carrier() -> super::left::SelfSignature { super::left::SelfSignature }\npub(in self) fn in_self_carrier() -> super::left::InSelfSignature { super::left::InSelfSignature }\npub(in crate::a) fn outward_carrier() -> super::left::OutwardSignature { super::left::OutwardSignature }\npub(in crate::a) fn exercise() { let _ = self_carrier(); let _ = in_self_carrier(); let _ = outward_carrier(); }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for line_start in [1, 2] {
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.path == "src/a/b/left.rs"
                    && finding.line_start == line_start
                    && finding.code == DiagnosticCode::ForbiddenPubCrate
            })
            .unwrap_or_else(|| panic!("missing private-equivalent sibling finding: {report:#?}"));
        assert!(
            finding.help.iter().any(|line| {
                line == "consider using `pub(super)` or removing `pub(crate)` entirely"
            }),
            "line {line_start} must receive only the nested-location recommendation: {report:#?}",
        );
        assert!(
            finding
                .help
                .iter()
                .all(|line| line != "consider using: `pub(super)`"),
            "line {line_start} must not receive a signature-exposure requirement: {report:#?}",
        );
    }

    let outward_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path == "src/a/b/left.rs"
                && finding.line_start == 3
                && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .unwrap_or_else(|| panic!("missing outward sibling control: {report:#?}"));
    assert!(
        outward_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the outward sibling must retain its crate::a signature requirement: {report:#?}",
    );
}

#[test]
fn unresolvable_restricted_facade_retains_its_occurrence_reach() {
    let temp = tempdir().expect("create unresolved facade reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "unresolved_restricted_facade_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::make(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod carrier;\nmod target;\npub(super) use carrier::*;\n",
            ),
            (
                "src/a/b/carrier.rs",
                "pub fn make() -> super::target::Target { super::target::Target }\n",
            ),
            ("src/a/b/target.rs", "pub struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| finding.item.as_deref() == Some("struct Target"))
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "restricted unresolved exposure must suppress unused_pub without becoming public: {report:#?}",
    );
    let target_finding = target_findings[0];
    assert_eq!(target_finding.code, DiagnosticCode::SuspiciousPub);
    assert_eq!(target_finding.fix_support, FixSupport::None);
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "the unresolved pub(super) facade must retain crate::a reach: {report:#?}",
    );
}

#[test]
fn signature_exposure_does_not_admit_pub_in_through_an_unresolvable_facade() {
    let temp = tempdir().expect("create blocked exposure fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "signature_exposure_blocked_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/main.rs", "mod a;\nfn main() { a::run(); }\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { let _ = b::expose(); }\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub(super) use c::*;\npub(super) fn expose() -> Target { Target }\n",
            ),
            ("src/a/b/c.rs", "pub(in crate::a) struct Target;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_findings = report
        .findings
        .iter()
        .filter(|finding| finding.path == "src/a/b/c.rs")
        .collect::<Vec<_>>();
    assert_eq!(
        target_findings.len(),
        1,
        "unexpected blocked-facade findings: {report:#?}",
    );
    let target_finding = target_findings[0];
    assert_eq!(target_finding.code, DiagnosticCode::ForbiddenPubInCrate);
    assert_eq!(
        target_finding.headline,
        "parent facade does not provide a resolvable visibility boundary"
    );
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "facade at a/b.rs:2 uses `*`; replace it with an explicit re-export before using `pub(in ...)`"
        }),
        "unexpected blocked-facade help: {report:#?}",
    );
}

#[test]
fn struct_and_union_field_signature_exposure_uses_each_field_reach() {
    let temp = tempdir().expect("create field signature reach fixture dir");
    write_allowance_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "field_signature_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "pub mod api;\n"),
            (
                "src/api.rs",
                "mod struct_case;\nmod union_case;\npub use struct_case::StructCarrier;\npub use union_case::UnionCarrier;\n",
            ),
            (
                "src/api/struct_case.rs",
                r#"#[derive(Clone, Copy)]
pub(in crate::api) struct StructSelfSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct StructInSelfSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct StructParentSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct StructExactSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct StructCrateSignature;
#[derive(Clone, Copy)]
pub(crate) struct StructPublicSignature;

pub struct StructCarrier {
    pub(self) self_value: StructSelfSignature,
    pub(in self) in_self_value: StructInSelfSignature,
    pub(super) parent_value: StructParentSignature,
    pub(in crate::api) exact_value: StructExactSignature,
    pub(crate) crate_value: StructCrateSignature,
    pub public_value: StructPublicSignature,
}
"#,
            ),
            (
                "src/api/union_case.rs",
                r#"#[derive(Clone, Copy)]
pub(in crate::api) struct UnionSelfSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct UnionInSelfSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct UnionParentSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct UnionExactSignature;
#[derive(Clone, Copy)]
pub(in crate::api) struct UnionCrateSignature;
#[derive(Clone, Copy)]
pub(crate) struct UnionPublicSignature;

pub union UnionCarrier {
    pub(self) self_value: UnionSelfSignature,
    pub(in self) in_self_value: UnionInSelfSignature,
    pub(super) parent_value: UnionParentSignature,
    pub(in crate::api) exact_value: UnionExactSignature,
    pub(crate) crate_value: UnionCrateSignature,
    pub public_value: UnionPublicSignature,
}
"#,
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for carrier in ["Struct", "Union"] {
        assert_field_signature_reach(&report, carrier);
    }
}

fn assert_field_signature_reach(report: &Report, carrier: &str) {
    let finding_path = format!("src/api/{}_case.rs", carrier.to_ascii_lowercase());
    for (signature, line_start) in [("Self", 2), ("InSelf", 4)] {
        let item = format!("struct {carrier}{signature}Signature");
        let findings = report
            .findings
            .iter()
            .filter(|finding| finding.path == finding_path && finding.line_start == line_start)
            .collect::<Vec<_>>();
        assert_eq!(
            findings.len(),
            1,
            "missing private field signature-type finding for {item}: {report:#?}",
        );
        assert!(
            findings[0]
                .help
                .iter()
                .any(|line| line == "consider removing the visibility"),
            "{item} must not receive outward signature exposure: {report:#?}",
        );
    }

    for (signature, line_start) in [("Parent", 6), ("Exact", 8)] {
        let item = format!("struct {carrier}{signature}Signature");
        let findings = report
            .findings
            .iter()
            .filter(|finding| finding.path == finding_path && finding.line_start == line_start)
            .collect::<Vec<_>>();
        assert_eq!(
            findings.len(),
            1,
            "missing restricted field signature-type finding for {item}: {report:#?}",
        );
        assert!(
            findings[0]
                .help
                .iter()
                .any(|line| line == "consider using: `pub(super)`"),
            "{item} must retain only the crate::api field boundary: {report:#?}",
        );
    }

    let crate_item = format!("struct {carrier}CrateSignature");
    let crate_finding = report
        .findings
        .iter()
        .find(|finding| finding.path == finding_path && finding.line_start == 10)
        .unwrap_or_else(|| panic!("missing field signature finding: {report:#?}"));
    assert!(
        crate_finding.help.iter().any(|line| {
            line
                == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
        }),
        "{crate_item} must retain only the crate field boundary: {report:#?}",
    );

    let public_item = format!("struct {carrier}PublicSignature");
    let public_finding = report
        .findings
        .iter()
        .find(|finding| finding.path == finding_path && finding.line_start == 12)
        .unwrap_or_else(|| panic!("missing field signature finding: {report:#?}"));
    assert!(
        public_finding
            .help
            .iter()
            .any(|line| line.contains("consider using `pub`")),
        "{public_item} must retain public signature reach: {report:#?}",
    );
}

#[test]
fn restricted_struct_and_union_fields_remain_forbidden_without_facades() {
    let temp = tempdir().expect("create field visibility fixture dir");
    fs::create_dir_all(temp.path().join("src/outer")).expect("create fixture modules");
    write_minimal_manifest(&temp, "restricted_field_visibility_fixture");
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write fixture visibility config");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod outer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/outer.rs"), "mod records;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/outer/records.rs"),
        "pub struct Record { pub(in crate::outer) marker: u8 }\n\npub union Bits { pub(in crate::outer) flag: u8 }\n",
    )
    .expect("write field declarations");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let field_findings = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path == "src/outer/records.rs"
        })
        .count();
    assert_eq!(field_findings, 2, "field findings: {report:#?}");
}

#[test]
fn no_facade_pub_crate_advice_uses_current_pass_callers() {
    let temp = tempdir().expect("create current-pass caller fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "current_pass_pub_crate_callers_fixture"
version = "0.1.0"
edition = "2024"

[lib]
test = false
doctest = false
bench = false
"#,
    )
    .expect("write single-target caller manifest");
    write_allowance_sources(
        &temp,
        &[
            ("src/lib.rs", "mod a;\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) fn run() { b::use_parent(); let _ = core::mem::size_of::<b::c::OuterTarget>(); }\n",
            ),
            (
                "src/a/b.rs",
                "pub(super) mod c;\npub(super) fn use_parent() { let _ = core::mem::size_of::<c::ParentTarget>(); }\n",
            ),
            (
                "src/a/b/c.rs",
                r#"pub(crate) struct ParentTarget;
pub(crate) struct OuterTarget;

pub(super) fn parent_surface(_: ParentTarget) {}
pub(super) fn outer_surface(_: OuterTarget) {}
"#,
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding_at = |line_start| {
        report
            .findings
            .iter()
            .find(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate
                    && finding.path == "src/a/b/c.rs"
                    && finding.line_start == line_start
            })
            .unwrap_or_else(|| {
                panic!("missing current-pass caller target at line {line_start}: {report:#?}")
            })
    };

    let parent_target = finding_at(1);
    assert!(
        parent_target
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "callers within the parent must require pub(super): {report:#?}",
    );

    let outer_target = finding_at(2);
    assert_eq!(
        outer_target.headline,
        "no visibility annotation allowed by policy preserves this item's current callers",
        "a caller above the parent must require structural advice: {report:#?}",
    );
    assert!(
        outer_target
            .help
            .iter()
            .any(|line| line.contains("add an explicit facade")),
        "the structural advice must identify the facade alternative: {report:#?}",
    );
}

#[test]
fn signature_exposure_resolves_each_active_declaration_identity() {
    let temp = tempdir().expect("create declaration identity fixture dir");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "signature_declaration_identity_fixture"
version = "0.1.0"
edition = "2024"

[lib]
test = false
doctest = false
bench = false
"#,
    )
    .expect("write single-target declaration manifest");
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write fixture visibility config");
    write_allowance_sources(
        &temp,
        &[
            ("src/lib.rs", "mod a;\n"),
            (
                "src/a.rs",
                "pub(crate) mod b;\npub(crate) fn call_active_surface() { let _ = b::shared_surface(); }\n",
            ),
            (
                "src/a/b.rs",
                r#"pub(crate) mod c;

#[cfg(any())]
pub(crate) fn shared_surface() -> c::InactiveTarget { c::InactiveTarget }

#[cfg(not(any()))]
pub(super) fn shared_surface() -> c::ActiveTarget { c::ActiveTarget }
"#,
            ),
            (
                "src/a/b/c.rs",
                r#"pub(crate) struct InactiveTarget;
pub(crate) struct ActiveTarget;
pub(crate) struct TypeTarget;
pub(crate) struct ValueTarget;

pub(super) type SharedSurface = TypeTarget;

pub(crate) const SharedSurface: ValueTarget = ValueTarget;
"#,
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_declaration_identity_findings(&report);
}

fn assert_declaration_identity_findings(report: &Report) {
    let finding_at = |line_start| {
        let matches = report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate
                    && finding.path == "src/a/b/c.rs"
                    && finding.line_start == line_start
            })
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "missing signature target at src/a/b/c.rs:{line_start}: {report:#?}",
        );
        matches[0]
    };

    let inactive_target = finding_at(1);
    assert!(
        inactive_target.help.iter().any(|line| {
            line == "consider using `pub(super)` or removing `pub(crate)` entirely"
        }),
        "inactive cfg declaration must not expose InactiveTarget: {report:#?}",
    );

    let active_target = finding_at(2);
    assert_eq!(
        active_target.headline,
        "no visibility annotation allowed by policy preserves this item's current callers",
        "the active parent-boundary declaration and its above-parent caller require structure: {report:#?}",
    );
    assert!(
        active_target.help.iter().any(|line| {
            line
                == "move the item into `crate::a`, or add an explicit facade at `crate::a` and rerun `cargo mend`"
        }),
        "ActiveTarget must retain the active declaration's crate::a reach: {report:#?}",
    );

    let type_target = finding_at(3);
    assert!(
        type_target
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "the type namespace must retain its parent-only alias reach: {report:#?}",
    );

    let value_target = finding_at(4);
    assert_eq!(
        value_target.headline,
        "no visibility annotation allowed by policy preserves this item's current callers",
        "the value namespace must retain its crate-visible const reach: {report:#?}",
    );
    assert!(
        value_target.help.iter().any(|line| {
            line
                == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
        }),
        "ValueTarget must retain the same-named const's crate reach: {report:#?}",
    );
}
