use crate::common::*;

#[test]
fn pub_in_private_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() { helpers::internal_fn(); }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(
        narrow_findings.len(),
        1,
        "expected 1 narrow_to_pub_crate finding, got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_eq!(narrow_findings[0].item.as_deref(), Some("fn internal_fn"));
    assert_summary_matches_findings(&report);
}

#[test]
fn re_exported_item_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_reexport_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\npub use helpers::exported_fn;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn exported_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert!(
        narrow_findings.is_empty(),
        "re-exported item should not be flagged: {narrow_findings:?}"
    );
}

#[test]
fn mixed_items_only_non_exported_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_mixed_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\npub use helpers::exported_fn;\n\npub fn entry() { helpers::internal_fn(); }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn exported_fn() {}\npub fn internal_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(narrow_findings.len(), 1);
    assert_eq!(narrow_findings[0].item.as_deref(), Some("fn internal_fn"));
    assert_summary_matches_findings(&report);
}

#[test]
fn mod_rs_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_modrs_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/helpers")).expect("create src/helpers");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() { helpers::internal_fn(); }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers/mod.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers mod.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(
        narrow_findings.len(),
        1,
        "expected 1 narrow_to_pub_crate finding for mod.rs, got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn binary_crate_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_bin_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod helpers;\nfn main() { helpers::internal_fn(); }\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(
        narrow_findings.len(),
        1,
        "expected 1 narrow_to_pub_crate finding in binary crate, got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn dry_run_reports_fix_count() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_dryrun_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn first() {}\npub fn second() {}\n",
    )
    .expect("write helpers");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix --dry-run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains('2'),
        "expected dry-run to mention 2 fixes: {combined}"
    );

    // Verify files were NOT modified
    let helpers = fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read helpers");
    assert!(
        helpers.contains("pub fn first()"),
        "dry-run should not modify files"
    );
}

#[test]
fn fix_replaces_pub_with_pub_crate() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\npub use helpers::exported_fn;\n\npub fn entry() {\n    helpers::internal_fn();\n    let _ = helpers::InternalStruct;\n}\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn exported_fn() {}\npub fn internal_fn() {}\npub struct InternalStruct;\n",
    )
    .expect("write helpers");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let helpers = fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read fixed file");
    assert!(
        helpers.contains("pub fn exported_fn()"),
        "re-exported item should stay `pub`: {helpers}"
    );
    assert!(
        helpers.contains("pub(crate) fn internal_fn()"),
        "non-exported fn should be narrowed to pub(crate): {helpers}"
    );
    assert!(
        helpers.contains("pub(crate) struct InternalStruct"),
        "non-exported struct should be narrowed to pub(crate): {helpers}"
    );
}

#[test]
fn methods_on_re_exported_type_are_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_impl_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod types;\npub use types::MyWidget;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/types.rs"),
        r#"pub struct MyWidget;

impl MyWidget {
    pub fn exported_method() -> Self { Self }
    pub fn another_method(&self) -> i32 { 42 }
}
"#,
    )
    .expect("write types");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert!(
        narrow_findings.is_empty(),
        "methods on re-exported type should not be flagged: {narrow_findings:?}"
    );
}

#[test]
fn type_reachable_via_reexported_enum_variant_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_reachable_variant_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod types;\npub use types::Icon;\npub use types::DEFAULT_FRAME;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/types.rs"),
        r#"pub struct FrameCycle {
    frames: &'static [&'static str],
}

impl FrameCycle {
    pub const fn new(frames: &'static [&'static str]) -> Self {
        Self { frames }
    }

    pub fn first(&self) -> &'static str {
        self.frames[0]
    }
}

pub enum Icon {
    Static(&'static str),
    Animated(FrameCycle),
}

pub const DEFAULT_FRAME: FrameCycle = FrameCycle::new(&["a", "b"]);
"#,
    )
    .expect("write types");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    let frame_cycle_flagged: Vec<_> = narrow_findings
        .iter()
        .filter(|f| {
            f.item.as_deref() == Some("struct FrameCycle") || f.item.as_deref() == Some("fn first")
        })
        .collect();
    assert!(
        frame_cycle_flagged.is_empty(),
        "struct FrameCycle and its `first` method are reachable through the re-exported \
         `Icon::Animated` variant and the `DEFAULT_FRAME` const, so they must not be flagged \
         for narrowing: {frame_cycle_flagged:?}"
    );
}

#[test]
fn methods_on_non_exported_type_are_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_impl_internal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() -> i32 {\n    let helper = helpers::InternalHelper;\n    helper.do_work()\n}\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        r#"pub struct InternalHelper;

impl InternalHelper {
    pub fn do_work(&self) -> i32 { 42 }
}
"#,
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    // Both the struct and its method should be flagged
    assert_eq!(
        narrow_findings.len(),
        2,
        "expected 2 narrow_to_pub_crate findings (struct + method), got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn integration_test_support_module_is_not_narrowed() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_tests_support_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "").expect("write lib");
    fs::create_dir_all(temp.path().join("tests")).expect("create tests");
    fs::write(temp.path().join("tests/support.rs"), "pub fn helper() {}\n").expect("write support");
    fs::write(
        temp.path().join("tests/consumer.rs"),
        "mod support;\n\n#[test]\nfn uses_support() { support::helper(); }\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--all-targets")
        .arg("--json")
        .output()
        .expect("run cargo-mend --all-targets --json");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_mend_json_output(&output.stdout);

    let narrow_on_support: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.path.ends_with("tests/support.rs")
        })
        .collect();
    assert!(
        narrow_on_support.is_empty(),
        "narrow_to_pub_crate should not fire in tests/: {narrow_on_support:?}",
    );

    let forbidden_on_support: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.path.ends_with("tests/support.rs")
        })
        .collect();
    assert!(
        forbidden_on_support.is_empty(),
        "forbidden_pub_crate should not fire on pub items in tests/: {forbidden_on_support:?}",
    );
}

#[test]
fn pub_at_depth_3_is_narrowed_when_parent_caps_at_pub_crate() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_nested_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo/mod.rs"),
        "mod bar;\npub(crate) use bar::helper;\n",
    )
    .expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/mod.rs"),
        "mod baz;\npub(crate) use baz::helper;\n\
         pub(crate) fn use_helper() { let _ = helper; }\n",
    )
    .expect("write foo/bar/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/baz.rs"),
        "pub fn helper() {}\n",
    )
    .expect("write foo/bar/baz.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_count = report
        .findings
        .iter()
        .filter(|f| {
            f.code == DiagnosticCode::NarrowToPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
        })
        .count();
    assert_eq!(
        narrow_count, 1,
        "bare `pub` at depth 3 should be flagged for narrowing when the parent facade re-exports \
         as `pub(crate) use`: {:?}",
        report.findings,
    );
}

#[test]
fn fix_compiler_does_not_remove_reexport_used_only_by_cfg_test_code() {
    // The `cargo fix` invocation underneath `cargo mend --fix-compiler`
    // binds a localhost TCP socket for its diagnostic server. Sandboxed
    // runners (style-fix worktrees, restricted CI) refuse that bind with
    // `Operation not permitted (os error 1)`. Skip when we detect we're
    // in such a sandbox so the failure does not block automated runs.
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_compiler_does_not_remove_reexport_used_only_by_cfg_test_code: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    // Regression test for the cfg(test) reachability bug. The `pub use` in
    // lib.rs is referenced only from `#[cfg(test)] mod tests`. Under
    // lib-only compilation rustc emits `unused_imports` because cfg(test)
    // is stripped. Today, `cargo mend --fix-compiler` chains `cargo fix`,
    // which deletes the re-export — and then the test target stops
    // compiling because `crate::helper` no longer resolves.
    //
    // After the redesign, mend's analysis pass must compile under
    // `--all-targets` so the test caller is visible and rustc does NOT emit
    // the `unused_imports` warning. The re-export must survive.
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_test_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/inner_facade")).expect("create src/inner_facade");
    // The `pub use` lives inside a private parent module, so its visibility
    // is effectively private — exactly the case where rustc fires the
    // `unused_imports` lint when the only callers are stripped by cfg.
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod inner_facade;\n\
         pub fn entry() { inner_facade::live() }\n\
         \n\
         #[cfg(test)]\n\
         mod tests {\n\
             #[test]\n\
             fn calls_helper() { crate::inner_facade::helper(7); }\n\
         }\n",
    )
    .expect("write lib.rs");
    fs::write(
        temp.path().join("src/inner_facade/mod.rs"),
        "mod child;\n\
         pub use child::helper;\n\
         \n\
         pub fn live() {}\n",
    )
    .expect("write inner_facade/mod.rs");
    fs::write(
        temp.path().join("src/inner_facade/child.rs"),
        "pub fn helper(_n: i32) {}\n",
    )
    .expect("write inner_facade/child.rs");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-compiler")
        .output()
        .expect("run cargo-mend --fix-compiler");
    assert!(
        output.status.success(),
        "cargo-mend --fix-compiler failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facade_after = fs::read_to_string(temp.path().join("src/inner_facade/mod.rs"))
        .expect("read inner_facade/mod.rs");
    assert!(
        facade_after.contains("pub use child::helper"),
        "the re-export reached only from #[cfg(test)] must NOT be removed; inner_facade/mod.rs after:\n{facade_after}",
    );

    // And the project must still compile under `cargo nextest run --no-run` —
    // i.e. mend left the tree in a state where every target builds.
    let test_build = std::process::Command::new("cargo")
        .arg("nextest")
        .arg("run")
        .arg("--no-run")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run cargo nextest run --no-run");
    assert!(
        test_build.status.success(),
        "test target must still compile after --fix-compiler:\n{}\n{}",
        String::from_utf8_lossy(&test_build.stdout),
        String::from_utf8_lossy(&test_build.stderr)
    );
}

#[test]
fn fix_does_not_narrow_pub_fn_used_only_from_cfg_test_caller() {
    // Regression for the cross-compilation merge issue: a `pub fn` whose
    // only outside-of-parent-subtree caller lives in a `#[cfg(test)] mod
    // tests` block must NOT be narrowed to `pub(super)`. The lib
    // compilation strips cfg(test) and sees no external callers, so it
    // would emit a `suspicious_pub` finding. The lib-test compilation
    // sees the test caller and emits no finding. Mend's
    // cross-compilation merge takes the intersection for narrowing-style
    // findings, so the lib's finding is suppressed.
    //
    // Likewise, the parent re-export in `panes/mod.rs` is referenced by
    // the test caller via `super::panes::cpu_required_pane_height(...)`,
    // so it must NOT be flagged as a stale internal facade.
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cross_compile_merge_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/panes")).expect("create src/tui/panes");
    fs::write(temp.path().join("src/lib.rs"), "mod tui;\n").expect("write lib");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod panes;\nmod render;\n",
    )
    .expect("write tui/mod");
    fs::write(
        temp.path().join("src/tui/panes/mod.rs"),
        "mod cpu;\npub use cpu::cpu_required_pane_height;\n\npub fn entry() { let _ = cpu::compute(0); }\n",
    )
    .expect("write panes/mod");
    fs::write(
        temp.path().join("src/tui/panes/cpu.rs"),
        "pub fn cpu_required_pane_height(_n: u16) -> u16 { compute(_n) }\npub fn compute(_n: u16) -> u16 { 1 }\n",
    )
    .expect("write panes/cpu");
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { let _ = crate::tui::panes::cpu_required_pane_height(12); }\n}\n",
    )
    .expect("write tui/render");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let bad_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (f.code == DiagnosticCode::SuspiciousPub
                && f.path.contains("panes/cpu.rs")
                && f.item.as_deref() == Some("fn cpu_required_pane_height"))
                || (f.code == DiagnosticCode::InternalParentPubUseFacade
                    && f.path.contains("panes/mod.rs"))
        })
        .collect();
    assert!(
        bad_findings.is_empty(),
        "items reachable only from #[cfg(test)] callers must not be flagged for narrowing or pub-use removal; got: {bad_findings:#?}",
    );
}

#[test]
fn fix_does_not_narrow_pub_fn_for_cfg_test_gated_pub_super_reexport() {
    // Exact reproduction of cargo-port-api-fix's failing case: a binary
    // crate with a `#[cfg(test)] pub(super) use ...` re-export in the
    // parent mod and a `#[cfg(test)] mod tests` caller outside the
    // parent subtree. Mend must NOT flag the underlying `pub fn` as
    // narrowable nor the re-export as removable.
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "binary_cfg_test_pub_super_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/panes")).expect("create dirs");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod tui;\nfn main() { tui::panes::entry() }\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "pub mod panes;\nmod render;\n",
    )
    .expect("write tui/mod");
    fs::write(
        temp.path().join("src/tui/panes/mod.rs"),
        "mod cpu;\n#[cfg(test)]\npub(super) use cpu::cpu_required_pane_height;\n\npub fn entry() { let _ = cpu::compute(0); }\n",
    )
    .expect("write panes/mod");
    fs::write(
        temp.path().join("src/tui/panes/cpu.rs"),
        "pub fn cpu_required_pane_height(_n: u16) -> u16 { compute(_n) }\npub fn compute(_n: u16) -> u16 { 1 }\n",
    )
    .expect("write panes/cpu");
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { let _ = crate::tui::panes::cpu_required_pane_height(12); }\n}\n",
    )
    .expect("write tui/render");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let bad_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (f.code == DiagnosticCode::SuspiciousPub
                && f.path.contains("panes/cpu.rs")
                && f.item.as_deref() == Some("fn cpu_required_pane_height"))
                || (f.code == DiagnosticCode::InternalParentPubUseFacade
                    && f.path.contains("panes/mod.rs"))
        })
        .collect();
    assert!(
        bad_findings.is_empty(),
        "items reachable only from #[cfg(test)] callers must not be flagged for narrowing or pub-use removal; got: {bad_findings:#?}",
    );
}

#[test]
fn fix_does_not_narrow_pub_fn_called_only_from_cfg_test_assert_macro() {
    // The hard case: the cfg(test) test caller invokes the function
    // *inside* an `assert_eq!` macro. syn's AST walker doesn't descend
    // into macro tokens, so without macro-aware analysis the source-level
    // facade scanner reports the re-export as "unused" and the analyzer
    // proposes narrowing the function plus removing the re-export. That
    // fix breaks the test build (E0425).
    //
    // This test passes when either (a) the source-level scanner walks
    // macro token streams, or (b) HIR-level reachability is used.
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "macro_caller_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/panes")).expect("create dirs");
    fs::write(temp.path().join("src/main.rs"), "mod tui;\nfn main() {}\n").expect("write main");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod panes;\nmod render;\n",
    )
    .expect("write tui/mod");
    fs::write(
        temp.path().join("src/tui/panes/mod.rs"),
        "mod cpu;\n#[cfg(test)]\npub(super) use cpu::cpu_required_pane_height;\n\npub fn entry() { let _ = cpu::compute(0); }\n",
    )
    .expect("write panes/mod");
    fs::write(
        temp.path().join("src/tui/panes/cpu.rs"),
        "pub fn cpu_required_pane_height(_n: u16) -> u16 { compute(_n) }\npub fn compute(_n: u16) -> u16 { 1 }\n",
    )
    .expect("write panes/cpu");
    // Caller invokes the function inside an assert_eq! — the path lives
    // in the macro token stream, not in the parsed AST.
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { assert_eq!(crate::tui::panes::cpu_required_pane_height(12), 1); }\n}\n",
    )
    .expect("write tui/render");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let bad_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (f.code == DiagnosticCode::SuspiciousPub
                && f.path.contains("panes/cpu.rs")
                && f.item.as_deref() == Some("fn cpu_required_pane_height"))
                || (f.code == DiagnosticCode::InternalParentPubUseFacade
                    && f.path.contains("panes/mod.rs"))
        })
        .collect();
    assert!(
        bad_findings.is_empty(),
        "items reachable only via a macro-wrapped #[cfg(test)] caller must not be flagged for narrowing or pub-use removal; got: {bad_findings:#?}",
    );
}
