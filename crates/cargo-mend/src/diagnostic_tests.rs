//! Diagnostic coverage: each module drives a fixture workspace through a
//! single mend diagnostic and asserts the findings and fixes it reports.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]
#![allow(
    clippy::needless_raw_string_hashes,
    reason = "test fixtures use raw strings with varying hash counts for readability"
)]

use std::path::Path;
use std::process::Output;

use crate::compiler;
use crate::fixes;
use crate::support;
use crate::support::Report;

#[path = "../tests/diagnostics/allowances.rs"]
mod allowances;
#[path = "../tests/diagnostics/facade_signature_reach.rs"]
mod facade_signature_reach;
#[path = "../tests/diagnostics/facade_subjects.rs"]
mod facade_subjects;
#[path = "../tests/diagnostics/field_visibility_wider_than_type.rs"]
mod field_visibility_wider_than_type;
#[path = "../tests/diagnostics/fix_rollback.rs"]
mod fix_rollback;
#[path = "../tests/diagnostics/forbidden_pub_in_crate.rs"]
mod forbidden_pub_in_crate;
#[path = "../tests/diagnostics/import_fixes.rs"]
mod import_fixes;
#[path = "../tests/diagnostics/imports_at_top.rs"]
mod imports_at_top;
#[path = "../tests/diagnostics/inline_path_fixes.rs"]
mod inline_path_fixes;
#[path = "../tests/diagnostics/narrow_pub_crate.rs"]
mod narrow_pub_crate;
#[path = "../tests/diagnostics/overbroad_pub_crate.rs"]
mod overbroad_pub_crate;
#[path = "../tests/diagnostics/prefer_module_import.rs"]
mod prefer_module_import;
#[path = "../tests/diagnostics/prelude_pub_mod.rs"]
mod prelude_pub_mod;
#[path = "../tests/diagnostics/pub_use_fixes.rs"]
mod pub_use_fixes;
#[path = "../tests/diagnostics/pub_use_outside_subtree.rs"]
mod pub_use_outside_subtree;
#[path = "../tests/diagnostics/pub_use_outside_subtree_fixes.rs"]
mod pub_use_outside_subtree_fixes;
#[path = "../tests/diagnostics/rendering.rs"]
mod rendering;
#[path = "../tests/diagnostics/shared_source_visibility.rs"]
mod shared_source_visibility;
#[path = "../tests/diagnostics/suspicious_pub_fixes.rs"]
mod suspicious_pub_fixes;
#[path = "../tests/diagnostics/target_identity.rs"]
mod target_identity;
#[path = "../tests/diagnostics/unused_pub.rs"]
mod unused_pub;

/// Use the real compiler analysis without a Cargo compilation or a CLI process.
fn compiled_report(manifest: &Path) -> Report {
    let output = compiler::analyze_fixture_json(manifest)
        .expect("analyze fixture with the production compiler callbacks");
    support::parse_mend_json_output(output.as_bytes())
}

/// Scan and fix in process, then validate every target with real Cargo.
fn rewrite_module_import_fixture(manifest: &Path) -> Output {
    let selection = compiler::read_fixture_selection(manifest).expect("read the fixture selection");
    fixes::apply_fixture_module_imports(&selection).expect("scan and fix the module imports");
    support::cargo_command()
        .args(["check", "--all-targets", "--manifest-path"])
        .arg(manifest)
        .output()
        .expect("run cargo check for the rewritten fixture")
}

/// Validate the rewritten type paths with Cargo after the production syntax plan.
fn rewrite_inline_path_fixture(manifest: &Path) -> Output {
    let selection = compiler::read_fixture_selection(manifest).expect("read the fixture selection");
    fixes::apply_fixture_inline_paths(&selection).expect("scan and fix the inline type paths");
    support::cargo_command()
        .args(["check", "--all-targets", "--manifest-path"])
        .arg(manifest)
        .output()
        .expect("run cargo check for the rewritten fixture")
}
