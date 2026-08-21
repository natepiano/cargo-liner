use crate::support::*;

/// A type re-exported by a `pub(super) use` facade and named in the signature of
/// a function that is itself reachable further out. The facade alone would put
/// the type at `crate::fit::overlay::render`; the caller that names the function
/// from `crate::fit::overlay` reaches the type through that signature, so the
/// declaration has to sit at `crate::fit::overlay` or rustc reports
/// `private_interfaces` and rolls the whole `--fix` batch back.
const SIGNATURE_REACHED_TYPE: &str = "pub(in crate::fit::overlay) struct FitMarginPercents";

/// The control: a sibling behind the same facade whose only callers live inside
/// `render`. Nothing widens it, so the facade boundary still applies and the
/// narrowing must still fire.
const FACADE_BOUNDED_TYPE: &str = "pub(in crate::fit::overlay::render) struct FitOverlayBudget";

#[test]
fn facade_type_named_in_a_wider_signature_keeps_the_caller_boundary() {
    let temp = tempdir().expect("create facade signature fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Required);
    write_sources(
        temp.path(),
        &[
            ("mend.toml", "[visibility]\npub_in_path = \"required\"\n"),
            (
                "Cargo.toml",
                r#"[package]
name = "facade_signature_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/lib.rs",
                "mod fit;\n\npub struct Query<T>(T);\npub struct With<T>(T);\n\npub fn entry() { fit::overlay::build(); }\n",
            ),
            ("src/fit/mod.rs", "pub mod overlay;\n"),
            (
                "src/fit/overlay/mod.rs",
                "mod render;\n\npub fn build() { let _system = render::cleanup_orphan_fit_overlay_visuals; }\n",
            ),
            (
                "src/fit/overlay/render/mod.rs",
                "mod bounds;\nmod reconciliation;\n\npub use reconciliation::cleanup_orphan_fit_overlay_visuals;\n",
            ),
            (
                "src/fit/overlay/render/bounds/mod.rs",
                "mod target_bounds;\n\npub(super) use target_bounds::{FitMarginPercents, FitOverlayBudget};\n",
            ),
            (
                "src/fit/overlay/render/bounds/target_bounds.rs",
                "pub struct FitMarginPercents { pub top: f32 }\npub struct FitOverlayBudget { pub cells: usize }\n",
            ),
            (
                "src/fit/overlay/render/reconciliation.rs",
                "use super::bounds::{FitMarginPercents, FitOverlayBudget};\nuse crate::{Query, With};\n\npub fn cleanup_orphan_fit_overlay_visuals(_stale: Query<With<FitMarginPercents>>) -> usize {\n    budget_cells(&FitOverlayBudget { cells: 1 })\n}\n\nfn budget_cells(budget: &FitOverlayBudget) -> usize { budget.cells }\n",
            ),
        ],
    );

    let manifest = temp.path().join("Cargo.toml");
    assert_fixture_compiles(&manifest, "fixture must compile before mend");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("compiler failed after applying mend fixes"),
        "--fix must not roll back: {stdout}\n{stderr}",
    );
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {stdout}\n{stderr}",
    );

    let target_bounds = fs::read_to_string(
        temp.path()
            .join("src/fit/overlay/render/bounds/target_bounds.rs"),
    )
    .expect("read fixed target_bounds source");
    assert!(
        target_bounds.contains(SIGNATURE_REACHED_TYPE),
        "the signature caller must widen the facade boundary:\n{target_bounds}",
    );
    assert!(
        target_bounds.contains(FACADE_BOUNDED_TYPE),
        "the facade-bounded sibling must still narrow:\n{target_bounds}",
    );

    assert_fixture_compiles(&manifest, "fixed sources must compile");
}

fn assert_fixture_compiles(manifest: &std::path::Path, context: &str) {
    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(manifest)
        .output()
        .expect("check facade signature fixture");
    assert!(
        check.status.success(),
        "{context}: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
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
