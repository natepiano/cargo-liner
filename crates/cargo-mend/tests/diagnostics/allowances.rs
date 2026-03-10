use crate::common::*;

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_other_crate_visible_signature() {
    let temp = tempdir().expect("create temp fixture dir");
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
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/utils/file_utils.rs"),
        "expected no suspicious_pub for child type exposed by another crate-visible signature, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_sibling_boundary_field() {
    let temp = tempdir().expect("create temp fixture dir");
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
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/app_tools/count.rs"),
        "expected no suspicious_pub for child type exposed by sibling boundary field, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_ancestor_boundary_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
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
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/brp_tools/types.rs"),
        "expected no suspicious_pub for child type exposed by sibling boundary field through ancestor re-export, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn suspicious_pub_is_suppressed_for_cross_file_public_field_exposure_via_ancestor_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
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
            finding.code == "suspicious_pub"
                && finding.path == "src/brp_tools/types.rs"
                && finding.item.as_deref() == Some("enum MouseButtonWrapper")
        }),
        "expected no suspicious_pub for child type exposed by sibling boundary field through ancestor re-export without immediate parent pub use, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn suspicious_pub_is_suppressed_for_cross_file_public_field_exposure() {
    let temp = tempdir().expect("create temp fixture dir");
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
            finding.code == "suspicious_pub"
                && finding.path == "src/guide/response_types.rs"
                && finding.item.as_deref() == Some("struct TypeGuideSummary")
        }),
        "expected no suspicious_pub for child type exposed by cross-file public field, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_exported_method_signatures() {
    let temp = tempdir().expect("create temp fixture dir");
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
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/utils/sha256_cache.rs"),
        "expected no suspicious_pub for child types exposed by exported method signatures, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_parent_boundary_signature() {
    let temp = tempdir().expect("create temp fixture dir");
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
    let extracted = crate::wikilink::extract();
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
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/wikilink/wikilink_types.rs"),
        "expected no suspicious_pub for child types exposed by parent boundary signatures, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn suspicious_pub_is_suppressed_for_parent_facade_used_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_still_warns_for_parent_facade_unused_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 1);
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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
        "fn consume() {\n    use crate::private_parent::PublicContainer;\n    let _ = std::mem::size_of::<PublicContainer>();\n}\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_for_internal_parent_super_facade_in_mod_rs() {
    let temp = tempdir().expect("create temp fixture dir");
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_when_child_boundary_file_is_mod_rs_and_parent_facade_is_used() {
    let temp = tempdir().expect("create temp fixture dir");
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
        "mod parent;\n\nfn main() {\n    let _ = parent::BoundaryType;\n}\n",
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_for_internal_parent_super_facade_in_file_module() {
    let temp = tempdir().expect("create temp fixture dir");
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn crate_relative_parent_facade_use_inside_parent_subtree_stays_fixable() {
    let temp = tempdir().expect("create temp fixture dir");
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 2);
    assert_eq!(report.summary.fixable_with_fix_count, 1);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_is_suppressed_for_tool_contract_attribute_output_type() {
    let temp = tempdir().expect("create temp fixture dir");
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
            finding.code == "suspicious_pub"
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
            finding.code == "suspicious_pub"
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
            finding.code == "suspicious_pub"
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
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 1);
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
