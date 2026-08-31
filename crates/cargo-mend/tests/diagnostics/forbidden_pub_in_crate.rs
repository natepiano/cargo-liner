use tempfile::TempDir;

use crate::support::*;

#[test]
fn restricted_visibility_annotations_are_rejected_once() {
    let temp = tempdir().expect("create temp fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "restricted_visibility_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"forbidden\"\n"),
            (
                "src/lib.rs",
                "mod fields;\nmod outer;\nmod use_line;\npub(crate) struct Imported;\npub(in crate) fn crate_wide() {}\n",
            ),
            ("src/outer.rs", "mod child;\nmod grandchild;\n"),
            (
                "src/outer/child.rs",
                "pub(in super) fn parent_only() {}\npub(in self) fn current_only() {}\n",
            ),
            (
                "src/outer/grandchild.rs",
                "pub(in super::super) fn root_only() {}\n",
            ),
            ("src/fields.rs", "mod inner;\n"),
            (
                "src/fields/inner.rs",
                "struct Restricted {\n    pub(in crate) crate_wide: u8,\n    pub(in super) parent: u8,\n    pub(in self) current: u8,\n    pub(in super::super) root: u8,\n}\n",
            ),
            ("src/use_line.rs", "pub(in super) use super::Imported;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    assert_rejected_annotations(&report);
}

#[test]
fn glob_blocker_precedes_repair_for_restricted_visibility_annotations() {
    let temp = tempdir().expect("create temp fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "glob_blocker_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"forbidden\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::*;\n"),
            (
                "src/a/b/c.rs",
                "pub(in super) fn parent_only() {}\npub(in crate::a) fn a_only() {}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let blocked = report
        .findings
        .iter()
        .filter(|finding| {
            finding.path.ends_with("src/a/b/c.rs")
                && finding.headline
                    == "parent facade does not provide a resolvable visibility boundary"
        })
        .collect::<Vec<_>>();
    assert_eq!(blocked.len(), 2, "unexpected glob findings: {report:#?}");
    for finding in blocked {
        assert_eq!(
            finding.help,
            [
                "facade at a/b.rs:2 uses `*`; replace it with an explicit re-export before using `pub(in ...)`"
            ],
        );
    }
}

#[test]
fn public_signature_requires_bare_pub_for_every_pub_in_spelling() {
    let temp = tempdir().expect("create public signature fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "public_pub_in_glob_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod a;\npub use a::{make_current, make_parent, make_relative, make_rooted};\n",
            ),
            (
                "src/a.rs",
                "mod b;\npub use b::{make_current, make_parent, make_relative, make_rooted};\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub use c::{make_current, make_parent, make_relative, make_rooted};\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(in super) struct ParentTarget;\npub(in self) struct CurrentTarget;\npub(in super::super) struct RelativeTarget;\npub(in crate::a) struct RootedTarget;\npub fn make_parent() -> ParentTarget { ParentTarget }\npub fn make_current() -> CurrentTarget { CurrentTarget }\npub fn make_relative() -> RelativeTarget { RelativeTarget }\npub fn make_rooted() -> RootedTarget { RootedTarget }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for line_start in 1..=4 {
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubInCrate
                    && finding.path == "src/a/b/c.rs"
                    && finding.line_start == line_start
            })
            .unwrap_or_else(|| {
                panic!("missing public signature finding at line {line_start}: {report:#?}")
            });
        let expected_help = if line_start == 4 {
            "consider using: `pub`"
        } else {
            "this item is exposed through a public signature; consider using `pub`"
        };
        assert!(
            finding.help.iter().any(|line| line == expected_help),
            "public signature advice must require bare pub at line {line_start}: {report:#?}",
        );
        assert!(
            finding
                .help
                .iter()
                .all(|line| !line.contains("replace it with an explicit re-export")),
            "public signature advice must bypass the glob blocker at line {line_start}: {report:#?}",
        );
    }
}

#[test]
fn public_signature_precedes_a_constructible_glob_blocker() {
    let temp = tempdir().expect("create public signature glob fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "public_pub_in_glob_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub use a::make;\n"),
            (
                "src/a.rs",
                "mod b;\npub use b::make;\npub(in crate::a) use b::*;\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub use c::make;\npub(in crate::a) use c::Target;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(in crate::a) struct Target;\npub fn make() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path == "src/a/b/c.rs"
                && finding.line_start == 1
        })
        .unwrap_or_else(|| panic!("missing public signature glob finding: {report:#?}"));
    assert!(
        finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub`"),
        "public signature advice must require bare pub: {report:#?}",
    );
    assert!(
        finding
            .help
            .iter()
            .all(|line| !line.contains("replace it with an explicit re-export")),
        "public signature advice must bypass the glob blocker: {report:#?}",
    );
}

#[test]
fn restricted_signature_retains_glob_blocker_for_pub_in_annotation() {
    let temp = tempdir().expect("create restricted signature glob fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "restricted_pub_in_glob_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\npub(crate) use a::make;\n"),
            (
                "src/a.rs",
                "mod b;\npub(crate) use b::make;\npub(in crate::a) use b::*;\n",
            ),
            (
                "src/a/b.rs",
                "mod c;\npub(crate) use c::make;\npub(in crate::a) use c::Target;\n",
            ),
            (
                "src/a/b/c.rs",
                "pub(in crate::a) struct Target;\npub(crate) fn make() -> Target { Target }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let target_finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path == "src/a/b/c.rs"
                && finding.line_start == 1
        })
        .unwrap_or_else(|| panic!("missing restricted signature glob finding: {report:#?}"));
    assert_eq!(
        target_finding.headline,
        "parent facade does not provide a resolvable visibility boundary"
    );
    assert!(
        target_finding.help.iter().any(|line| {
            line
                == "facade at a.rs:3 uses `*`; replace it with an explicit re-export before using `pub(in ...)`"
        }),
        "restricted signature advice must retain the glob blocker: {report:#?}",
    );
}

#[test]
fn exact_crate_rooted_boundary_is_accepted_when_enabled() {
    for (pub_in_path, expected_codes) in [
        ("forbidden", vec![DiagnosticCode::ForbiddenPubInCrate]),
        ("permitted", Vec::new()),
        ("required", Vec::new()),
    ] {
        let temp = tempdir().expect("create temp fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "exact_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
                ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
                ("src/a/b/c.rs", "pub(in crate::a) fn exact() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a/b/c.rs", &expected_codes);
        if pub_in_path == "forbidden" {
            assert_headline_and_help(
                &report,
                "src/a/b/c.rs",
                "use of `pub(in crate::a)` is disabled by project visibility policy",
                "consider using: `pub`; or set `pub_in_path = \"permitted\"`",
            );
        }
    }
}

/// A `use` item earns the escape hatch on the same terms a declaration does.
///
/// The annotation already sits at the parent facade boundary, so the spelling
/// is the entire complaint and permitting the spelling answers it. Both the
/// acceptance gate and the help line used to be gated on
/// `ItemCategory::Declaration`, which left a re-export line with no suggestion
/// and no setting that could clear it.
#[test]
fn exact_crate_rooted_boundary_on_a_use_item_is_accepted_when_enabled() {
    for (pub_in_path, expected_codes) in [
        ("forbidden", vec![DiagnosticCode::ForbiddenPubInCrate]),
        ("permitted", Vec::new()),
        ("required", Vec::new()),
    ] {
        let temp = tempdir().expect("create temp fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "exact_boundary_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
                ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
                ("src/a/b/c.rs", "mod d;\npub(in crate::a) use d::exact;\n"),
                ("src/a/b/c/d.rs", "pub fn exact() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a/b/c.rs", &expected_codes);
        if pub_in_path == "forbidden" {
            // No `consider using: `pub`` here: a `pub use` cannot be wider than
            // the item it re-exports, and mend has not proven this target wide
            // enough to take one.
            assert_headline_and_help(
                &report,
                "src/a/b/c.rs",
                "use of `pub(in crate::a)` is disabled by project visibility policy",
                "set `pub_in_path = \"permitted\"`",
            );
        }
    }
}

/// A `use` item no facade covers still names the repair it can act on.
///
/// Resolved paths name the imported target, not the alias, so no caller set is
/// available and every caller-derived repair is out of reach — the branch used
/// to return no suggestion at all. The facade is not caller-derived: its path is
/// read off the annotation. `cfg(test)` on the re-export and on its only
/// consumer is what keeps the cross-crate pass from resolving a boundary, and a
/// resolved boundary would overwrite the suggestion under test.
#[test]
fn no_facade_use_item_names_the_facade_that_would_allow_it() {
    let temp = tempdir().expect("create temp fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "no_facade_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\nmod c;\n"),
            (
                "src/a/b.rs",
                "mod d;\n#[cfg(test)]\npub(in crate::a) use d::Thing;\n",
            ),
            ("src/a/b/d.rs", "pub(in crate::a) struct Thing;\n"),
            (
                "src/a/c.rs",
                "#[cfg(test)]\nmod tests {\n    use crate::a::b::Thing;\n\n    #[test]\n    \
                 fn constructs() {\n        let _ = Thing;\n    }\n}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(
        &report,
        "src/a/b.rs",
        &[DiagnosticCode::ForbiddenPubInCrate],
    );
    assert_headline_and_help(
        &report,
        "src/a/b.rs",
        "`pub(in crate::a)` is not the path callers use to name this item",
        "add `pub(in crate::a) use b::Thing;` to `crate::a`, then rerun `cargo mend`",
    );
    assert_note(
        &report,
        "src/a/b.rs",
        "`pub_in_path` accepts this boundary only when the item is nameable as \
         `crate::a::Thing`. Callers reach it as `crate::a::b::Thing` instead, below the \
         `crate::a` the annotation names",
    );
}

#[test]
fn required_setting_reviews_bare_pub_behind_restricted_facade() {
    for (pub_in_path, expected_codes) in [
        ("forbidden", Vec::new()),
        ("permitted", Vec::new()),
        ("required", vec![DiagnosticCode::SuspiciousPub]),
    ] {
        let temp = tempdir().expect("create required-path fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "required_path_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
                ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
                ("src/a/b/c.rs", "pub fn exact() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a/b/c.rs", &expected_codes);
        if pub_in_path == "required" {
            assert_headline_and_help(
                &report,
                "src/a/b/c.rs",
                "`pub` is broader than this nested module boundary",
                "consider using: `pub(in crate::a)`",
            );
        }
    }
}

#[test]
fn fix_rewrites_a_required_bare_pub_to_the_exact_boundary() {
    let temp = tempdir().expect("create required-path fix fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "required_path_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"required\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
            ("src/a/b/c.rs", "pub fn exact() {}\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(&report, "src/a/b/c.rs", &[DiagnosticCode::SuspiciousPub]);
    assert_eq!(
        report.summary.fixable_with_fix, 1,
        "a required-mode bare `pub` with a resolved boundary must be offered to `--fix`: {report:#?}"
    );

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(temp.path().join("src/a/b/c.rs")).expect("read fixed declaration"),
        "pub(in crate::a) fn exact() {}\n",
        "only the annotation may change, and it must name the resolved boundary"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/a/b.rs")).expect("read facade"),
        "mod c;\npub(super) use c::exact;\n",
        "the facade line must be left byte-identical"
    );
}

#[test]
fn fix_leaves_an_accepted_restricted_annotation_alone() {
    // The mirror of the rewrite above: once the declaration already spells the
    // exact boundary, nothing is fixable and `--fix` must not edit the file.
    // A fixer that matched `pub` textually would strip the `(in crate::a)`.
    let temp = tempdir().expect("create accepted-boundary fix fixture dir");
    let declaration = "pub(in crate::a) fn exact() {}\n";
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "accepted_boundary_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"required\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
            ("src/a/b/c.rs", declaration),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(&report, "src/a/b/c.rs", &[]);
    assert_eq!(report.summary.fixable_with_fix, 0, "{report:#?}");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(temp.path().join("src/a/b/c.rs")).expect("read declaration"),
        declaration,
    );
}

#[test]
fn the_exact_boundary_rewrite_is_offered_only_under_required() {
    // One fixture across all three settings, with nothing varying but
    // `pub_in_path`. The rewrite spells `pub(in crate::a)`: under `forbidden`
    // that is exactly what `forbidden_pub_in_crate` reports as an error on the
    // next run, and under `permitted` it is one accepted spelling rather than
    // the one the policy asks for, so only `required` may hand the declaration
    // to `--fix`.
    //
    // Two independent gates produce that outcome, and keeping the `required`
    // arm alongside the other two is what makes their absence meaningful:
    // `policy::exact_boundary_narrowing` elevates a bare `pub` behind a resolved
    // facade only under `required`, so the other settings report nothing at all
    // for this declaration, and `rewrites_annotation_only` in `scan/record.rs`
    // re-checks the setting before granting `FixSupport::RestrictedAnnotation`.
    // If the elevation ever stops depending on the setting, the two quiet arms
    // start seeing a finding and fail on the declaration bytes below instead of
    // writing an annotation the same run rejects.
    let declaration = "pub fn exact() {}\n";
    for (pub_in_path, expected_codes, expected_declaration) in [
        ("forbidden", Vec::new(), declaration),
        ("permitted", Vec::new(), declaration),
        (
            "required",
            vec![DiagnosticCode::SuspiciousPub],
            "pub(in crate::a) fn exact() {}\n",
        ),
    ] {
        let temp = tempdir().expect("create pub-in-path mode fix fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "pub_in_path_mode_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
                ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
                ("src/a/b/c.rs", declaration),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a/b/c.rs", &expected_codes);
        assert_eq!(
            report.summary.fixable_with_fix,
            usize::from(expected_declaration != declaration),
            "`pub_in_path = \"{pub_in_path}\"` advertised the wrong `--fix` route count: {report:#?}"
        );

        let output = mend_command()
            .arg("--manifest-path")
            .arg(temp.path().join("Cargo.toml"))
            .arg("--fix")
            .output()
            .expect("run cargo-mend --fix");

        assert_eq!(
            fs::read_to_string(temp.path().join("src/a/b/c.rs")).expect("read declaration"),
            expected_declaration,
            "`pub_in_path = \"{pub_in_path}\"` left the wrong declaration: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("src/a/b.rs")).expect("read facade"),
            "mod c;\npub(super) use c::exact;\n",
            "`pub_in_path = \"{pub_in_path}\"` must leave the facade line byte-identical"
        );
    }
}

#[test]
fn boundary_mismatch_precedes_redundant_spelling_at_every_setting() {
    for pub_in_path in ["forbidden", "permitted", "required"] {
        let temp = tempdir().expect("create boundary-mismatch fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "boundary_mismatch_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "mod b;\n"),
                ("src/a/b.rs", "mod c;\npub(super) use c::wide;\n"),
                ("src/a/b/c.rs", "pub(in crate) fn wide() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(
            &report,
            "src/a/b/c.rs",
            &[DiagnosticCode::OverbroadPubCrate],
        );
        assert_headline_and_help(
            &report,
            "src/a/b/c.rs",
            if pub_in_path == "forbidden" {
                "`pub(in crate)` is wider than the exact parent facade boundary"
            } else {
                "`pub(crate)` is broader than required"
            },
            "consider using: `pub(in crate::a)`",
        );
    }
}

#[test]
fn crate_boundary_uses_the_canonical_pub_crate_spelling_at_every_setting() {
    for pub_in_path in ["forbidden", "permitted", "required"] {
        let temp = tempdir().expect("create crate-boundary fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "crate_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\npub(crate) use a::helper;\n"),
                ("src/a.rs", "pub(in super) fn helper() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a.rs", &[DiagnosticCode::ForbiddenPubInCrate]);
        assert_headline_and_help(
            &report,
            "src/a.rs",
            "parent facade caps reach at `pub(crate)`",
            "consider using: `pub(crate)`",
        );
    }
}

#[test]
fn written_visibility_syntaxes_are_tightened_at_every_setting() {
    for pub_in_path in ["forbidden", "permitted", "required"] {
        let temp = tempdir().expect("create written-syntax matrix fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "written-syntax-matrix-fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "pub(in crate) fn crate_wide() {}\nmod b;\n"),
                (
                    "src/a/b.rs",
                    "pub(in self) fn current_only() {}\npub(in super) fn parent_only() {}\nmod c;\n",
                ),
                (
                    "src/a/b/c.rs",
                    "pub(in super::super) fn grandparent_only() {}\n",
                ),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a.rs", &[DiagnosticCode::OverbroadPubCrate]);
        assert_headline_and_help(
            &report,
            "src/a.rs",
            "`pub(crate)` is broader than required",
            "consider removing the visibility",
        );
        assert_codes(
            &report,
            "src/a/b.rs",
            &[
                DiagnosticCode::ForbiddenPubInCrate,
                DiagnosticCode::ForbiddenPubInCrate,
            ],
        );
        assert_headline_and_help(
            &report,
            "src/a/b.rs",
            "`pub(in self)` is a redundant spelling of `pub(self)`",
            "consider using: `pub(self)`",
        );
        assert_headline_and_help(
            &report,
            "src/a/b.rs",
            "`pub(in super)` is a redundant spelling of `pub(super)`",
            "consider using: `pub(super)`",
        );
        assert_codes(
            &report,
            "src/a/b/c.rs",
            &[DiagnosticCode::ForbiddenPubInCrate],
        );
        assert_headline_and_help(
            &report,
            "src/a/b/c.rs",
            "`pub(in super::super)` is not the path callers use to name this item",
            "consider removing the visibility",
        );
    }
}

#[test]
fn pub_in_crate_is_accepted_at_a_required_crate_boundary() {
    for pub_in_path in ["forbidden", "permitted", "required"] {
        let temp = tempdir().expect("create canonical crate-boundary fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "canonical-pub-in-crate-fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\npub(crate) use a::helper;\n"),
                ("src/a.rs", "pub(in crate) fn helper() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_codes(&report, "src/a.rs", &[]);
    }
}

#[test]
fn relative_exact_boundary_suggests_crate_rooted_spelling() {
    for pub_in_path in ["forbidden", "permitted", "required"] {
        let temp = tempdir().expect("create relative exact-boundary fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    r#"[package]
name = "relative_exact_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
                ),
                (
                    "mend.toml",
                    &format!("[visibility]\npub_in_path = \"{pub_in_path}\"\n"),
                ),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", "mod b;\n"),
                ("src/a/b.rs", "mod c;\npub(super) use c::relative;\n"),
                ("src/a/b/c.rs", "pub(in super::super) fn relative() {}\n"),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert_headline_and_help(
            &report,
            "src/a/b/c.rs",
            "use of `pub(in super::super)` does not use the canonical crate-rooted boundary",
            "consider using: `pub(in crate::a)`",
        );
    }
}

#[test]
fn declaration_narrower_than_facade_is_rejected_by_rustc() {
    let temp = tempdir().expect("create too-narrow compile fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "too_narrow_compile_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", "mod b;\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::narrow;\n"),
            ("src/a/b/c.rs", "pub(super) fn narrow() {}\n"),
        ],
    );

    let output = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run rustc compile-fail control");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("E0364"),
        "expected E0364 for a facade wider than its declaration:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn no_facade_callers_select_only_compiling_advice() {
    for (package_name, a_source, b_source, c_source, expected_headline, expected_help) in [
        (
            "no_facade_local_callers_fixture",
            "mod b;\n",
            "mod c;\n",
            "pub(in crate::a) fn helper() {}\nfn local() { helper(); }\n",
            "`pub(in crate::a)` is not the boundary this item's callers require",
            "consider removing the visibility",
        ),
        (
            "no_facade_parent_callers_fixture",
            "mod b;\n",
            "mod c;\nfn parent() { c::helper(); }\n",
            "pub(in crate::a) fn helper() {}\n",
            "`pub(in crate::a)` is not the boundary this item's callers require",
            "consider using: `pub(super)`",
        ),
        (
            "no_facade_outer_callers_fixture",
            "mod b;\nfn above_parent() { b::c::helper(); }\n",
            "pub(super) mod c;\n",
            "pub(in crate::a) fn helper() {}\n",
            "accepted",
            "accepted",
        ),
    ] {
        let temp = tempdir().expect("create no-facade fixture dir");
        write_sources(
            &temp,
            &[
                (
                    "Cargo.toml",
                    &format!(
                        "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
                    ),
                ),
                ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", a_source),
                ("src/a/b.rs", b_source),
                ("src/a/b/c.rs", c_source),
            ],
        );

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        if expected_headline == "accepted" {
            assert_codes(&report, "src/a/b/c.rs", &[]);
        } else {
            assert_headline_and_help(&report, "src/a/b/c.rs", expected_headline, expected_help);
        }
    }
}

/// The cross-crate pass resolves `crate::root::panel` for a declaration written
/// `pub(in crate::root)`, and mend both advertises and applies the narrowing.
///
/// The applier once refused any restricted spelling but `pub(crate)` and
/// `pub(in crate)`, so this finding was reported forever and never written. The
/// rollback that motivated the refusal came from narrowing an item a wider
/// item's signature still named, which `signature_requirement` now folds into
/// the boundary before it is ever suggested.
#[test]
fn caller_derived_boundary_is_directly_fixable() {
    let temp = tempdir().expect("create caller-boundary fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "caller_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod root;\npub fn entry() { root::entry(); }\n",
            ),
            (
                "src/root/mod.rs",
                "mod panel;\npub(crate) fn entry() { panel::entry(); }\n",
            ),
            (
                "src/root/panel/mod.rs",
                "mod conversion;\nmod diegetic;\npub(super) fn entry() { diegetic::run(); }\n",
            ),
            (
                "src/root/panel/conversion/mod.rs",
                "pub(super) mod saved;\n",
            ),
            (
                "src/root/panel/conversion/saved.rs",
                "pub(in crate::root) fn apply() {}\n",
            ),
            (
                "src/root/panel/diegetic.rs",
                "use super::conversion::saved;\npub(super) fn run() { saved::apply(); }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(
        report.summary.fixable_with_fix, 1,
        "a resolved boundary must advertise the fix that writes it: {report:#?}",
    );
    // The scanned crate saw no repair and stamped the structural headline. The
    // cross-crate pass then resolved `crate::root::panel`, so that headline no
    // longer describes this finding — it would deny the visibility the help
    // line goes on to name.
    assert_headline_and_help(
        &report,
        "src/root/panel/conversion/saved.rs",
        "`pub(in crate::root)` is not the boundary this item's callers require",
        "consider using: `pub(in crate::root::panel)`",
    );

    mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");

    assert_eq!(
        fs::read_to_string(temp.path().join("src/root/panel/conversion/saved.rs"))
            .expect("read fixed saved module"),
        "pub(in crate::root::panel) fn apply() {}\n",
        "the resolved boundary must be the one written, not a wider one"
    );
}

#[test]
fn signature_only_exact_boundary_needs_no_facade() {
    let temp = create_signature_only_boundary_fixture(NamingCaller::Absent);
    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(&report, "src/a/b/c.rs", &[]);
}

#[test]
fn exact_boundary_accepts_a_caller_naming_the_item() {
    let temp = create_signature_only_boundary_fixture(NamingCaller::Present);
    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(&report, "src/a/b/c.rs", &[]);
}

#[test]
fn a_sibling_binary_keeps_an_exact_signature_boundary_accepted() {
    let lib_only = create_cross_target_signature_only_fixture(SiblingBinary::Absent);
    let lib_only_report = run_mend_json(&lib_only.path().join("Cargo.toml"));
    assert_codes(&lib_only_report, "src/a/target.rs", &[]);

    let with_binary = create_cross_target_signature_only_fixture(SiblingBinary::Present);
    let with_binary_report = run_mend_json(&with_binary.path().join("Cargo.toml"));
    assert_codes(&with_binary_report, "src/a/target.rs", &[]);
}

#[test]
fn sibling_binary_caller_accepts_its_exact_boundary() {
    let lib_only = create_cross_target_no_facade_fixture(SiblingBinary::Absent);
    let lib_only_report = run_mend_json(&lib_only.path().join("Cargo.toml"));
    assert_headline_and_help(
        &lib_only_report,
        "src/a/c.rs",
        "`pub(in crate::a)` is not the boundary this item's callers require",
        "consider removing the visibility",
    );

    let with_binary = create_cross_target_no_facade_fixture(SiblingBinary::Present);
    let with_binary_report = run_mend_json(&with_binary.path().join("Cargo.toml"));
    assert_codes(&with_binary_report, "src/a/c.rs", &[]);
}

#[test]
fn cross_target_resolved_facade_preserves_joined_reexport_boundary() {
    let signature_only = create_cross_target_facade_signature_fixture(SiblingBinary::Absent);
    let signature_only_report = run_mend_json(&signature_only.path().join("Cargo.toml"));
    assert_headline_and_help(
        &signature_only_report,
        "src/a/target.rs",
        "`pub(in crate::a)` is not the boundary this item's callers require",
        "consider using: `pub(super)`",
    );

    let with_facade = create_cross_target_facade_signature_fixture(SiblingBinary::Present);
    let with_facade_report = run_mend_json(&with_facade.path().join("Cargo.toml"));
    assert_headline_and_help(
        &with_facade_report,
        "src/a/target.rs",
        "no allowed visibility keeps this item reachable from its callers: private and `pub(super)` are too narrow, and `pub` needs a re-export to cap it",
        "move the item into `crate::a::b`, or add `pub(in crate::a::b) use c::d::Target;` to `crate::a::b`, then rerun `cargo mend`",
    );
    assert!(
        with_facade_report.findings.iter().all(|finding| {
            finding.path != "src/a/target.rs"
                || finding
                    .help
                    .iter()
                    .all(|line| line != "consider using: `pub(super)`")
        }),
        "`pub(super)` would make the binary facade fail with E0364 or E0365: {with_facade_report:#?}",
    );
}

#[test]
fn workspace_member_callers_do_not_cross_contaminate_no_facade_advice() {
    let temp = tempdir().expect("create workspace caller-isolation fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[workspace]
members = ["member-a", "member-b"]
resolver = "3"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "member-a/Cargo.toml",
                r#"[package]
name = "caller-isolation-member-a"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "member-a/src/lib.rs",
                "mod a {\n    mod b {\n        pub(in crate::a) fn helper() {}\n    }\n}\n",
            ),
            (
                "member-b/Cargo.toml",
                r#"[package]
name = "caller-isolation-member-b"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "member-b/src/lib.rs",
                "mod a {\n    mod b {\n        pub(in crate::a) fn helper() {}\n    }\n    fn caller() { b::helper(); }\n}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_headline_and_help(
        &report,
        "member-a/src/lib.rs",
        "`pub(in crate::a)` is not the boundary this item's callers require",
        "consider removing the visibility",
    );
    assert_codes(&report, "member-b/src/lib.rs", &[]);
}

#[test]
fn exact_field_boundary_is_accepted_for_ancestor_access() {
    let temp = tempdir().expect("create field-caller fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "field-caller-fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            (
                "src/a.rs",
                "mod b;\nfn reads_field() { let record = b::Record { value: 1 }; let _ = record.value; }\n",
            ),
            (
                "src/a/b.rs",
                "pub(super) struct Record { pub(in crate::a) value: u8 }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(&report, "src/a/b.rs", &[]);
}

#[test]
fn caller_analysis_tracks_intermediate_modules() {
    let temp = tempdir().expect("create module-caller fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "module-caller-fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            (
                "src/a.rs",
                "mod b;\nfn calls_child() { b::child::helper(); }\n",
            ),
            ("src/a/b.rs", "pub(in crate::a) mod child;\n"),
            ("src/a/b/child.rs", "pub(in crate::a) fn helper() {}\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_codes(
        &report,
        "src/a/b.rs",
        &[DiagnosticCode::ForbiddenPubInCrate],
    );
    assert_headline_and_help(
        &report,
        "src/a/b.rs",
        "`pub(in crate::a)` is not the path callers use to name this item",
        "consider using: `pub(super)`",
    );
}

/// A `use` item is never handed a replacement visibility.
///
/// `reads_alias` names the target through `b::Thing`, so the caller set mend can
/// resolve describes `child::Thing` rather than the alias. It cannot say whether
/// the alias still has users, and acting on it would tell this line to drop or
/// narrow the visibility holding the re-export up. The facade is the one repair
/// that survives, because its path is read off the annotation.
#[test]
fn caller_analysis_withholds_replacement_for_use_items() {
    let temp = tempdir().expect("create use-item caller fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "use-item-caller-fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            (
                "src/a.rs",
                "mod b;\nfn reads_alias() { let _: b::Thing = b::Thing; }\n",
            ),
            (
                "src/a/b.rs",
                "mod child;\npub(in crate::a) use child::Thing;\n",
            ),
            ("src/a/b/child.rs", "pub(in crate::a) struct Thing;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.path.ends_with("src/a/b.rs")
                && finding.code == DiagnosticCode::ForbiddenPubInCrate
        })
        .unwrap_or_else(|| panic!("missing restricted use finding: {report:#?}"));
    assert!(
        finding.help.iter().any(|line| line
            == "add `pub(in crate::a) use b::Thing;` to `crate::a`, then rerun `cargo mend`"),
        "a `use` item must be offered the re-export: {finding:#?}"
    );
    assert!(
        !finding.help.iter().any(|line| {
            line.contains("consider removing the visibility") || line.contains("consider using")
        }),
        "a `use` item must not be told to remove or narrow its visibility: {finding:#?}"
    );
}

/// Following the facade suggestion has to end somewhere.
///
/// `caller_analysis_withholds_replacement_for_use_items` establishes the
/// suggestion this fixture applies: add `pub(in crate::a) use b::Thing;` to
/// `crate::a`. The added line is itself a `pub(in crate::a) use`, and no facade
/// stands above it, so the same advice fires on the repair unless
/// `pub_in_boundary_is_the_items_own_module` accepts a boundary that names the
/// alias's own module. Without it the reader is sent to add a re-export of
/// `Thing` into the module the re-export already sits in.
///
/// `src/a/local.rs` is the control the acceptance must not swallow. Its
/// annotation also names its own module, but it sits on a declaration, and
/// `pub_in_boundary_is_the_items_own_module` is limited to `ItemCategory::Use`
/// so that declarations keep going through
/// `facade_less_boundary_matches_callers`, which can measure their callers and
/// answer. Dropping that limit silences this half while the assertions above
/// stay green.
#[test]
fn advised_facade_converges_once_it_reaches_the_boundary() {
    let temp = tempdir().expect("create converging facade fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "converging-facade-fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            (
                "src/a.rs",
                "mod b;\nmod local;\nmod reader;\npub(in crate::a) use b::Thing;\n",
            ),
            (
                "src/a/b.rs",
                "mod child;\npub(in crate::a) use child::Thing;\n",
            ),
            ("src/a/b/child.rs", "pub(in crate::a) struct Thing;\n"),
            (
                "src/a/local.rs",
                "pub(in crate::a::local) struct Local;\npub(in crate::a::local) fn reads_local() -> Local { Local }\n",
            ),
            (
                "src/a/reader.rs",
                "fn reads_facade() { let _: crate::a::Thing = crate::a::Thing; }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let unconverged: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && !finding.path.ends_with("src/a/local.rs")
        })
        .collect();
    assert!(
        unconverged.is_empty(),
        "the applied facade must leave no forbidden `pub(in ...)` behind: {unconverged:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path.ends_with("src/a/local.rs")
        }),
        "a declaration whose boundary names its own module must stay flagged: {report:#?}",
    );
}

#[derive(Clone, Copy)]
enum SiblingBinary {
    Absent,
    Present,
}

/// Whether a caller writes `Target`'s own path across the `crate::a` boundary.
/// The two fixtures differ in nothing else: `expose` names `Target` in its
/// signature either way, so the declared reach stays exactly what the signature
/// demands.
#[derive(Clone, Copy)]
enum NamingCaller {
    Absent,
    Present,
}

fn create_signature_only_boundary_fixture(naming_caller: NamingCaller) -> TempDir {
    let temp = tempdir().expect("create signature-only boundary fixture dir");
    let a_source = match naming_caller {
        NamingCaller::Absent => "mod b;\nfn calls_expose() { let _ = b::expose(); }\n",
        NamingCaller::Present => "mod b;\nfn names_target() { let _ = b::c::Target; }\n",
    };
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "signature_only_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            ("src/lib.rs", "mod a;\n"),
            ("src/a.rs", a_source),
            (
                "src/a/b.rs",
                "pub(super) mod c;\npub(super) fn expose() -> c::Target { c::Target }\n",
            ),
            ("src/a/b/c.rs", "pub(in crate::a) struct Target;\n"),
        ],
    );
    temp
}

fn create_cross_target_signature_only_fixture(sibling_binary: SiblingBinary) -> TempDir {
    let temp = tempdir().expect("create cross-target signature-only fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "cross_target_signature_only_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod a {\n    mod b {\n        mod c { include!(\"a/target.rs\"); }\n        \
                 pub(super) fn expose() -> c::Target { c::Target }\n    }\n    pub(crate) fn run() \
                 { let _ = b::expose(); }\n}\n",
            ),
            ("src/a/target.rs", "pub(in crate::a) struct Target;\n"),
        ],
    );
    if matches!(sibling_binary, SiblingBinary::Present) {
        write_sources(
            &temp,
            &[(
                "src/bin/probe.rs",
                "mod a {\n    mod b {\n        mod c { include!(\"../a/target.rs\"); }\n        \
                 pub(super) use c::Target;\n        pub(super) fn expose() -> c::Target { \
                 c::Target }\n    }\n    pub(super) fn names_target() { let _ = b::Target; }\n}\nfn \
                 main() { a::names_target(); }\n",
            )],
        );
    }
    temp
}

fn create_cross_target_no_facade_fixture(sibling_binary: SiblingBinary) -> TempDir {
    let temp = tempdir().expect("create cross-target fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "cross_target_no_facade_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
            (
                "src/lib.rs",
                "mod a {\n    mod b {\n        mod c { include!(\"a/c.rs\"); }\n    }\n}\n",
            ),
            ("src/a/c.rs", "pub(\n    in crate::a\n) fn helper() {}\n"),
        ],
    );
    if matches!(sibling_binary, SiblingBinary::Present) {
        write_sources(
            &temp,
            &[(
                "src/bin/probe.rs",
                "mod a {\n    mod b {\n        mod c { include!(\"../a/c.rs\"); }\n        pub(super) use c::helper;\n    }\n    pub(super) fn caller() { b::helper(); }\n}\nfn main() { a::caller(); }\n",
            )],
        );
    }
    temp
}

fn create_cross_target_facade_signature_fixture(sibling_binary: SiblingBinary) -> TempDir {
    let temp = tempdir().expect("create cross-target facade fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "cross_target_facade_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"forbidden\"\n"),
            (
                "src/lib.rs",
                "mod a {\n    mod b {\n        mod c {\n            mod d { include!(\"a/target.rs\"); }\n        }\n    }\n}\n",
            ),
            (
                "src/a/target.rs",
                "pub(in crate::a) struct Target;\npub(super) fn expose() -> Target { Target }\n",
            ),
        ],
    );
    if matches!(sibling_binary, SiblingBinary::Present) {
        write_sources(
            &temp,
            &[(
                "src/bin/probe.rs",
                "mod a {\n    mod b {\n        mod c {\n            mod d { include!(\"../a/target.rs\"); }\n            pub(super) use d::Target;\n        }\n    }\n}\nfn main() {}\n",
            )],
        );
    }
    temp
}

fn assert_rejected_annotations(report: &Report) {
    assert_codes(report, "src/lib.rs", &[DiagnosticCode::OverbroadPubCrate]);
    assert_codes(
        report,
        "src/outer/child.rs",
        &[
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::ForbiddenPubInCrate,
        ],
    );
    assert_codes(
        report,
        "src/outer/grandchild.rs",
        &[DiagnosticCode::ForbiddenPubInCrate],
    );
    assert_codes(
        report,
        "src/use_line.rs",
        &[DiagnosticCode::ForbiddenPubInCrate],
    );
    assert_codes(
        report,
        "src/fields/inner.rs",
        &[
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::OverbroadPubCrate,
        ],
    );

    assert_headline_and_help(
        report,
        "src/lib.rs",
        "`pub(crate)` is broader than required",
        "consider removing the visibility",
    );
    assert_headline_and_help(
        report,
        "src/outer/child.rs",
        "`pub(in super)` is a redundant spelling of `pub(super)`",
        "consider using: `pub(super)`",
    );
    assert_headline_and_help(
        report,
        "src/outer/child.rs",
        "`pub(in self)` is a redundant spelling of `pub(self)`",
        "consider using: `pub(self)`",
    );
    assert_headline_and_help(
        report,
        "src/outer/grandchild.rs",
        "`pub(in super::super)` is not the path callers use to name this item",
        "consider removing the visibility",
    );
    assert_headline_and_help(
        report,
        "src/use_line.rs",
        "`pub(in super)` is a redundant spelling of `pub(super)`",
        "consider using: `pub(super)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "`pub(crate)` is broader than required",
        "consider removing the visibility",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "`pub(in super)` is a redundant spelling of `pub(super)`",
        "consider using: `pub(super)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "`pub(in self)` is a redundant spelling of `pub(self)`",
        "consider using: `pub(self)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "`pub(in super::super)` is not the path callers use to name this item",
        "consider removing the visibility",
    );
}

#[test]
fn a_lib_and_bin_target_pair_advertises_and_applies_one_rewrite() {
    // `src/a/b/c.rs` is compiled twice — once for the library, once for the
    // binary — so the same declaration produces two analysis passes over one
    // byte range. The finding must be advertised once, and `--fix` must edit the
    // site once: a second edit would append a second annotation to the same
    // line. No other Required-mode fixture declares both targets.
    let temp = tempdir().expect("create lib and bin required-mode fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "required_lib_and_bin_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("src/lib.rs", "mod a;\n"),
            ("src/main.rs", "mod a;\n\nfn main() {}\n"),
            ("src/a.rs", "mod b;\nfn use_exact() { b::exact(); }\n"),
            ("src/a/b.rs", "mod c;\npub(super) use c::exact;\n"),
            ("src/a/b/c.rs", "pub fn exact() {}\n"),
        ],
    );
    pin_pub_in_path(temp.path(), PubInPath::Required);

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    // Each target reports the declaration once — the library and the binary are
    // separate crates that happen to share the file. What must not double is the
    // advertised fix: `retain_one_restricted_annotation_fix_per_site` demotes the
    // duplicate so only one finding carries the `--fix` note.
    assert_codes(&report, "src/a/b/c.rs", &[DiagnosticCode::SuspiciousPub]);
    assert_eq!(
        report.summary.fixable_with_fix, 1,
        "two compilations of one declaration must advertise one fix: {report:#?}"
    );

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(temp.path().join("src/a/b/c.rs")).expect("read fixed declaration"),
        "pub(in crate::a) fn exact() {}\n",
        "the shared declaration must be rewritten exactly once"
    );
}

/// A bounded `pub(in crate::…)` is auto-fixable, but only by removing it, and
/// only on a declaration.
///
/// `pub_in_path = "forbidden"` is the one setting under which this diagnostic
/// fires, and `exact_boundary_acceptance` used to fold that setting in, so no
/// `forbidden_pub_in_crate` finding could ever be promoted to fixable. Removing
/// an annotation spells no path, so the setting does not bear on it — hence the
/// split into `annotation_edit_acceptance`. The module keeps its error: a
/// module's visibility caps every item inside it, while the caller scan proves
/// only what names the module itself. The three declarations are all repaired:
/// `Identity`, `Installation`, and its field lose their annotations entirely,
/// and `install` is retargeted to `pub(super)`, the one module that calls it.
#[test]
fn bounded_pub_in_path_is_fixable_on_declarations_but_not_on_a_module() {
    let module_declaration = "pub(in crate::animation) mod installation;\n\npub(super) fn run() { installation::install(); }\n";
    let temp = tempdir().expect("create bounded pub-in-path fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "bounded_pub_in_path_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            ("mend.toml", "[visibility]\npub_in_path = \"forbidden\"\n"),
            (
                "src/lib.rs",
                "mod animation;\n\npub fn start() { animation::drive(); }\n",
            ),
            (
                "src/animation.rs",
                "mod sequence;\n\npub(crate) fn drive() { sequence::run(); }\n",
            ),
            ("src/animation/sequence.rs", module_declaration),
            (
                "src/animation/sequence/installation.rs",
                "pub(in crate::animation) struct Identity;\n\npub(in crate::animation) struct \
                 Installation {\n    pub(in crate::animation) identity: Identity,\n}\n\npub(in \
                 crate::animation) fn install() {\n    let installation = Installation { identity: \
                 Identity };\n    let _ = &installation.identity;\n}\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for suffix in [
        "src/animation/sequence.rs",
        "src/animation/sequence/installation.rs",
    ] {
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.path.ends_with(suffix)
                    && finding.code == DiagnosticCode::ForbiddenPubInCrate),
            "expected a forbidden_pub_in_crate finding for {suffix}: {report:#?}"
        );
    }

    // The run exits non-zero: the module declaration is still reported and is
    // not repairable here, which is the other half of what this asserts.
    mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");

    assert_eq!(
        fs::read_to_string(temp.path().join("src/animation/sequence/installation.rs"))
            .expect("read fixed declarations"),
        "struct Identity;\n\nstruct Installation {\n    identity: Identity,\n}\n\npub(super) \
         fn install() {\n    let installation = Installation { identity: Identity };\n    let _ = \
         &installation.identity;\n}\n",
        "every declaration must land on the boundary its callers proved"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/animation/sequence.rs"))
            .expect("read module declaration"),
        module_declaration,
        "a module declaration must not be rewritten"
    );
}

/// A `use` re-export lands on the boundary its callers proved; a `mod`
/// declaration carrying the same annotation does not.
///
/// Both items here are written `pub(in crate::animation)` and both resolve to
/// the same `pub(super)`, so `ItemCategory` is the only thing separating
/// them. Until `annotation_edit_acceptance` accepted `ItemCategory::Use`, the
/// re-export printed that boundary under `help: consider using:` and then
/// declined to write it — reported on every run and repaired on none.
///
/// The module line stays out: a `mod` visibility governs a whole subtree, which
/// is a review decision rather than a rewrite.
///
/// This fixture deliberately writes no `mend.toml`. The default
/// `pub_in_path = "required"` is what routes the re-export to its caller-derived
/// boundary; under `"forbidden"` the same sources draw a facade demand instead
/// and never reach the branch under test.
///
/// `Interruption` reaches `crate::animation::sequence` through the
/// `Playback::interruption` field. Without that signature exposure the
/// constraint carries no required reach, and the finding stops short of the
/// caller-derived boundary this asserts.
#[test]
fn bounded_pub_in_path_is_fixable_on_a_use_reexport_but_not_on_a_module() {
    let module_declaration = "pub(in crate::animation) mod playback;\n\nuse \
                              playback::Playback;\n\npub(super) fn run() { let _ = \
                              Playback::new().interruption; }\n";
    let repaired_reexport = "mod lifecycle;\n\npub(super) use \
                             lifecycle::Interruption;\n\npub(super) struct Playback {\n    \
                             pub(super) interruption: Interruption,\n}\n\nimpl Playback {\n    \
                             pub(super) fn new() -> Self { Self { interruption: Interruption } \
                             }\n}\n";
    let temp = tempdir().expect("create re-export boundary fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "reexport_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/lib.rs",
                "mod animation;\n\npub fn start() { animation::drive(); }\n",
            ),
            (
                "src/animation.rs",
                "mod sequence;\n\npub(crate) fn drive() { sequence::run(); }\n",
            ),
            ("src/animation/sequence.rs", module_declaration),
            (
                "src/animation/sequence/playback.rs",
                "mod lifecycle;\n\npub(in crate::animation) use \
                 lifecycle::Interruption;\n\npub(super) struct Playback {\n    pub(super) \
                 interruption: Interruption,\n}\n\nimpl Playback {\n    pub(super) fn new() -> \
                 Self { Self { interruption: Interruption } }\n}\n",
            ),
            (
                "src/animation/sequence/playback/lifecycle.rs",
                "pub(in crate::animation) struct Interruption;\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for suffix in [
        "src/animation/sequence.rs",
        "src/animation/sequence/playback.rs",
    ] {
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.path.ends_with(suffix)
                    && finding.code == DiagnosticCode::ForbiddenPubInCrate),
            "expected a forbidden_pub_in_crate finding for {suffix}: {report:#?}"
        );
    }

    mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");

    assert_eq!(
        fs::read_to_string(temp.path().join("src/animation/sequence/playback.rs"))
            .expect("read fixed re-export"),
        repaired_reexport,
        "a `use` re-export must land on the boundary its callers proved"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/animation/sequence.rs"))
            .expect("read module declaration"),
        module_declaration,
        "a module declaration must not be rewritten, whatever boundary resolves"
    );
}

/// A bounded `pub(in crate::…)` is auto-fixable whether the resolved boundary
/// widens its reach or only respells it, and the two carry different headlines.
///
/// `Pose` reaches the crate root through `Carrier::pose`, so the boundary the
/// item needs is `crate` — a widening, and the headline names the annotation
/// that does not carry that reach. `build` is the other case: every call sits
/// in `crate::animation`, which is its own parent module, so `pub(super)`
/// reaches exactly what `pub(in crate::animation)` already reaches. Nothing
/// about that item's reach changes, so the policy headline about the spelling
/// is still the whole finding.
#[test]
fn bounded_pub_in_path_is_fixable_when_the_boundary_widens_or_respells_it() {
    let sequence_source = "pub(in crate::animation) struct Pose;\n\npub(in crate::animation) fn \
                           build() -> Pose {\n    Pose\n}\n";
    let temp = create_widening_boundary_fixture(sequence_source);

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding_at = |line_start: usize| {
        report
            .findings
            .iter()
            .find(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubInCrate
                    && finding.path.ends_with("src/animation/sequence.rs")
                    && finding.line_start == line_start
            })
            .unwrap_or_else(|| panic!("missing finding at sequence.rs:{line_start}: {report:#?}"))
    };

    let widening = finding_at(1);
    assert!(
        widening
            .help
            .iter()
            .any(|line| line == "consider using: `pub(crate)`"),
        "the crate-rooted boundary must be suggested: {report:#?}"
    );
    // The policy headline describes a spelling rule, which is no longer what
    // this finding is about once a wider boundary is resolved, and the note is
    // the only line that says where `pub(crate)` came from.
    assert_eq!(
        widening.headline,
        "`pub(in crate::animation)` is not the boundary this item's callers require",
        "a resolved wider boundary must replace the policy headline: {report:#?}"
    );
    assert!(
        widening.help.iter().any(|line| line
            == "`pub(crate)` is the narrowest visibility that covers everything reaching this \
                item"),
        "the resolved boundary must be explained: {report:#?}"
    );
    assert_eq!(
        AdvertisedFix::from_notes(widening.help.iter().map(String::as_str)),
        AdvertisedFix::WithFix,
        "a widening retarget must advertise a fix: {report:#?}"
    );

    let respelling = finding_at(3);
    assert!(
        respelling
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`"),
        "the parent boundary must be suggested: {report:#?}"
    );
    // `crate::animation` is this item's parent module, so `pub(super)` reaches
    // exactly what the annotation already reaches. Nothing about the boundary
    // changed, and the policy that rejects the spelling is still the whole
    // finding.
    assert_eq!(
        respelling.headline,
        "`pub(in crate::animation)` is not the path callers use to name this item",
        "a respelling of the same reach must keep the policy headline: {report:#?}"
    );
    assert_eq!(
        AdvertisedFix::from_notes(respelling.help.iter().map(String::as_str)),
        AdvertisedFix::WithFix,
        "a respelling retarget must advertise a fix: {report:#?}"
    );

    // Severity is derived from fixability, so both land as warnings without
    // either being stored that way.
    assert_eq!(
        (report.summary.errors, report.summary.warnings),
        (0, 2),
        "severity must follow fixability: {report:#?}"
    );

    mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");

    assert_eq!(
        fs::read_to_string(temp.path().join("src/animation/sequence.rs"))
            .expect("read fixed sequence module"),
        "pub(crate) struct Pose;\n\npub(super) fn build() -> Pose {\n    Pose\n}\n",
        "each declaration must land on its own resolved boundary"
    );
}

/// The widening-boundary fixture: `Pose` is used only inside
/// `crate::animation`, but `Carrier::pose` carries it to the crate root, so
/// `Pose` needs a wider boundary than its annotation while `build` needs only a
/// respelling of the reach it already has.
fn create_widening_boundary_fixture(sequence_source: &str) -> TempDir {
    let temp = tempdir().expect("create widening boundary fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "widening_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            // `overbroad_pub_crate` is off because it fires on `Carrier::pose`,
            // whose `pub(crate)` no caller outside `crate::animation` reads.
            // That field is what holds `Pose` open at the crate root, so the
            // fixture cannot drop it without losing the case under test.
            (
                "mend.toml",
                "[diagnostics]\noverbroad_pub_crate = false\n\n[visibility]\npub_in_path = \
                 \"forbidden\"\n",
            ),
            (
                "src/lib.rs",
                "mod animation;\n\npub fn start() {\n    let carrier = animation::carrier();\n    \
                 let _ = size_of_val(&carrier);\n}\n",
            ),
            (
                "src/animation.rs",
                "mod sequence;\n\nuse sequence::Pose;\n\npub(crate) struct Carrier {\n    \
                 pub(crate) pose: Pose,\n}\n\npub(crate) fn carrier() -> Carrier {\n    Carrier \
                 {\n        pose: sequence::build(),\n    }\n}\n",
            ),
            ("src/animation/sequence.rs", sequence_source),
        ],
    );
    temp
}

fn write_sources(temp: &TempDir, sources: &[(&str, &str)]) {
    for (relative_path, source) in sources {
        let path = temp.path().join(relative_path);
        fs::create_dir_all(path.parent().expect("source path has a parent"))
            .expect("create source parent directory");
        fs::write(path, source).expect("write fixture source");
    }
}

fn assert_codes(report: &Report, suffix: &str, expected: &[DiagnosticCode]) {
    let codes = report
        .findings
        .iter()
        .filter(|finding| finding.path.ends_with(suffix))
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert_eq!(
        codes, expected,
        "unexpected diagnostic code set for {suffix}: {:?}",
        report.findings,
    );
}

fn assert_headline_and_help(report: &Report, suffix: &str, headline: &str, help: &str) {
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.path.ends_with(suffix) && finding.headline == headline)
        .unwrap_or_else(|| {
            panic!(
                "missing headline {headline:?} for {suffix}: {:?}",
                report.findings,
            )
        });
    assert!(
        finding.help.iter().any(|line| line == help),
        "missing help {help:?} for {headline:?}: {:?}",
        finding.help,
    );
}

/// Asserts the finding reported for `suffix` carries `note`.
///
/// The note says why the suggestion is the one being offered;
/// [`assert_headline_and_help`] covers the suggestion itself.
fn assert_note(report: &Report, suffix: &str, note: &str) {
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing finding for {suffix}: {:?}", report.findings));
    assert!(
        finding.help.iter().any(|line| line == note),
        "missing note {note:?} for {suffix}: {:?}",
        finding.help,
    );
}

/// Widening is the repair mend reaches for when an item's callers need more
/// reach than its annotation gives, and it used to be applied unconditionally:
/// every name that already resolved still resolves. That covers name
/// resolution and not rustc's private-interfaces rule. `Thing` is the self type
/// of `impl TryFrom<u8> for Thing`, whose associated error type is the narrower
/// `PrepError`, so widening `Thing` to `pub` widens that impl past `PrepError`
/// and the build stops with E0446 — taking every other fix in the batch down
/// with it. The finding still names `pub`, because `pub` is still the answer;
/// what it no longer does is claim mend can get there in one edit.
#[test]
fn widening_past_a_trait_impl_interface_is_reported_without_a_fix() {
    let temp = create_interface_ceiling_fixture();

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let widening = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate && finding.path.ends_with("pose.rs")
        })
        .unwrap_or_else(|| {
            panic!("missing forbidden_pub_in_crate finding for `Thing`: {report:#?}")
        });

    assert!(
        widening
            .help
            .iter()
            .any(|line| line == "consider using: `pub`"),
        "the resolved boundary must still be suggested: {report:#?}"
    );
    assert!(
        widening.help.iter().any(|line| line
            == "`pub` would expose `crate::a::b::c::PrepError` through `impl TryFrom<u8> for \
                Thing`, which rustc rejects (E0446) — widen `crate::a::b::c::PrepError` first"),
        "the note must name the type that would leak and the impl exposing it: {report:#?}"
    );
    assert_eq!(
        AdvertisedFix::from_notes(widening.help.iter().map(String::as_str)),
        AdvertisedFix::NotOffered,
        "a widening past an interface ceiling must not advertise a fix: {report:#?}"
    );

    mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");

    assert!(
        fs::read_to_string(temp.path().join("src/a/b/c/pose.rs"))
            .expect("read fixed pose module")
            .contains("pub(in crate::a) struct Thing"),
        "`Thing` must keep the annotation whose widening rustc would reject"
    );
    // The other half: the ceiling must cap `Thing` alone, not stand down every
    // repair in the crate. `Playback` reaches no further than its parent
    // module, and that narrowing still lands in the same run.
    assert!(
        fs::read_to_string(temp.path().join("src/a/b/c/mod.rs"))
            .expect("read fixed c module")
            .contains("pub(super) struct Playback"),
        "an unrelated repair in the same crate must still be applied"
    );
}

/// The interface-ceiling fixture. `Handle` is genuine public API whose field
/// names `Thing`, which is what puts `pub` on `Thing` as the required reach —
/// no caller can name `Thing` outside `crate::a`, because rustc forbids using a
/// value of a restricted type across the boundary at all. The trait in
/// `impl TryFrom<u8> for Thing` has to be fully public: a restricted trait
/// argument would cap the impl below `PrepError` and rustc would have no
/// complaint.
fn create_interface_ceiling_fixture() -> TempDir {
    let temp = tempdir().expect("create interface ceiling fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "interface_ceiling_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "mend.toml",
                "[visibility]\npub_in_path = \"forbidden\"\nallow_pub_mod = [\"src/lib.rs\", \
                 \"src/a/mod.rs\", \"src/a/b/mod.rs\", \"src/a/b/c/mod.rs\"]\n",
            ),
            (
                "src/lib.rs",
                "pub mod a;\n\npub fn run() -> bool { a::drive() }\n",
            ),
            (
                "src/a/mod.rs",
                "pub mod b;\n\npub(crate) fn drive() -> bool { b::drive() }\n",
            ),
            (
                "src/a/b/mod.rs",
                r#"pub mod c;

pub(super) struct Cam {
    pub(super) value: u8,
}

pub(super) fn drive() -> bool {
    let cam = Cam { value: 3 };
    c::Playback::prepare(&cam).is_ok_and(|playback| playback.start.value() > 0)
}
"#,
            ),
            (
                "src/a/b/c/mod.rs",
                r#"pub mod pose;

use pose::Thing;

use super::Cam;

pub(in crate::a) enum PrepError {
    Unrepresentable,
}

pub(crate) struct Playback {
    pub(super) start: Thing,
}

impl Playback {
    pub(super) fn prepare(cam: &Cam) -> Result<Self, PrepError> {
        Ok(Self { start: Thing::try_from(cam.value)? })
    }
}
"#,
            ),
            (
                "src/a/b/c/pose.rs",
                r#"use super::PrepError;

#[derive(Clone, Copy)]
pub(in crate::a) struct Thing(pub(super) u8);

impl Thing {
    pub(in crate::a::b) const fn value(self) -> u8 { self.0 }
}

impl TryFrom<u8> for Thing {
    type Error = PrepError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value > 0 { Ok(Self(value)) } else { Err(PrepError::Unrepresentable) }
    }
}

/// Public API whose field names `Thing`, so rustc requires `Thing` to be `pub`.
pub struct Handle {
    pub thing: Thing,
}
"#,
            ),
        ],
    );
    temp
}

#[test]
fn cfg_excluded_caller_keeps_pub_in_path() {
    let temp = tempdir().expect("create cfg fixture dir");
    // Three `pub(in crate::desktop)` items on three lines of `candidate.rs`,
    // one per outcome. `gated_only` is called only from a `#[cfg]`-excluded
    // module, so the post-expansion HIR reports no caller outside `candidate`
    // and the boundary looks removable — taking that advice would stop the
    // excluded configuration compiling, so nothing is reported. `local_only`
    // is the control: it is named nowhere excluded and really is used only
    // inside `candidate`, so it must still be reported. `caller` is used from
    // `crate::desktop`, which is exactly the boundary it spells.
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "cfg_excluded_pub_in_path_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/lib.rs",
                "mod desktop;\n\npub fn entry() -> u32 { desktop::entry() }\n",
            ),
            (
                "src/desktop.rs",
                "mod candidate;\n\n#[cfg(target_os = \"solaris\")]\nmod platform;\n\npub(crate) fn entry() -> u32 { candidate::caller() }\n",
            ),
            (
                "src/desktop/platform.rs",
                "pub(super) fn use_it() -> u32 { super::candidate::gated_only() }\n",
            ),
            (
                "src/desktop/candidate.rs",
                "pub(in crate::desktop) fn gated_only() -> u32 { 1 }\npub(in crate::desktop) fn local_only() -> u32 { 2 }\npub(in crate::desktop) fn caller() -> u32 { local_only() }\n",
            ),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let reported_lines: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path.ends_with("src/desktop/candidate.rs")
        })
        .map(|finding| finding.line_start)
        .collect();
    assert_eq!(
        reported_lines,
        vec![2],
        "line 1's only caller is `#[cfg]`-excluded and line 3 spells the boundary its caller \
         needs, so only line 2 is reported: {report:#?}",
    );
}
