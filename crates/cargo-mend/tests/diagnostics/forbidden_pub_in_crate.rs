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
        "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
        "re-export `b::Thing` from `crate::a` so callers can name it there, then rerun `cargo \
         mend`",
    );
    assert_note(
        &report,
        "src/a/b.rs",
        "every caller in `crate::a` may use this item, but they must still spell it \
         `crate::a::b::Thing`. `pub_in_path` allows this boundary only when a re-export publishes \
         the item as `crate::a::Thing`",
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
            "use of `pub(in super::super)` outside an exact facade boundary is forbidden by policy",
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
            "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
            "consider removing the visibility",
        ),
        (
            "no_facade_parent_callers_fixture",
            "mod b;\n",
            "mod c;\nfn parent() { c::helper(); }\n",
            "pub(in crate::a) fn helper() {}\n",
            "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
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

/// The cross-crate pass resolves an exact boundary here, but the declaration is
/// already written `pub(in crate::root)` and `fixes::restricted_annotation`
/// rewrites only a bare `pub`, `pub(crate)`, and `pub(in crate)`. Advertising a
/// fix anyway is what made `--fix-all` loop: every run reported it fixable,
/// wrote nothing, and reported it again. Applying it unconditionally is not the
/// answer either — that was tried on a real workspace and rustc rejected the
/// narrowed declaration with E0446, restricted type in public interface.
#[test]
fn caller_derived_boundary_is_reported_without_advertising_a_fix() {
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
        report.summary.fixable_with_fix, 0,
        "an already-restricted annotation must not advertise a fix mend cannot \
         apply: {report:#?}",
    );
    // The scanned crate saw no repair and stamped the structural headline. The
    // cross-crate pass then resolved `crate::root::panel`, so that headline no
    // longer describes this finding — it would deny the visibility the help
    // line goes on to name.
    assert_headline_and_help(
        &report,
        "src/root/panel/conversion/saved.rs",
        "use of `pub(in crate::root)` outside an exact facade boundary is forbidden by policy",
        "consider using: `pub(in crate::root::panel)`",
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
        "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
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
        "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
        "consider using: `pub(super)`",
    );

    let with_facade = create_cross_target_facade_signature_fixture(SiblingBinary::Present);
    let with_facade_report = run_mend_json(&with_facade.path().join("Cargo.toml"));
    assert_headline_and_help(
        &with_facade_report,
        "src/a/target.rs",
        "no allowed visibility keeps this item reachable from its callers: private and `pub(super)` are too narrow, and `pub` needs a re-export to cap it",
        "move the item into `crate::a::b`, or re-export `c::d::Target` from `crate::a::b` so callers can name it there, then rerun `cargo mend`",
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
        "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
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
        "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
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
            == "re-export `b::Thing` from `crate::a` so callers can name it there, then rerun \
                `cargo mend`"),
        "a `use` item must be offered the re-export: {finding:#?}"
    );
    assert!(
        !finding.help.iter().any(|line| {
            line.contains("consider removing the visibility") || line.contains("consider using")
        }),
        "a `use` item must not be told to remove or narrow its visibility: {finding:#?}"
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
        "use of `pub(in super::super)` outside an exact facade boundary is forbidden by policy",
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
        "use of `pub(in super::super)` outside an exact facade boundary is forbidden by policy",
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
