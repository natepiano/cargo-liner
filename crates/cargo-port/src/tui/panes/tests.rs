use ratatui::text::Line;
use tui_pane::label_color;

use super::CI_COMPACT_DURATION_WIDTH;
use super::DetailField;
use super::GitData;
use super::PackageData;
use super::pane_data as model;
use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::ci::FetchStatus::Fetched;
use crate::project::GitStatus;
use crate::tui::app::AvailabilityStatus;
use crate::tui::pane::PaneFocusState;
use crate::tui::panes;
use crate::tui::render::CiColumn;

fn package_data(is_rust_project: bool) -> PackageData {
    PackageData {
        package_title:            if is_rust_project {
            "Package".to_string()
        } else {
            "Project".to_string()
        },
        title_name:               "demo".to_string(),
        path:                     "~/demo".to_string(),
        version:                  "0.1.0".to_string(),
        description:              None,
        crates_version:           None,
        crates_downloads:         None,
        types:                    "lib".to_string(),
        disk:                     "36.3 GiB".to_string(),
        stats_rows:               Vec::new(),
        has_package:              true,
        edition:                  None,
        license:                  None,
        homepage:                 None,
        repository:               None,
        in_project_target:        None,
        in_project_non_target:    None,
        out_of_tree_target_bytes: None,
        lint_display:             super::LintDisplay::default(),
        ci_display:               super::CiDisplay::default(),
    }
}

fn git_data() -> GitData {
    GitData {
        branch:             None,
        status:             None,
        vs_local:           None,
        local_main_branch:  None,
        main_branch_label:  "main".to_string(),
        stars:              None,
        description:        None,
        inception:          None,
        last_commit:        None,
        last_fetched:       None,
        rate_limit_core:    None,
        rate_limit_graphql: None,
        github_status:      AvailabilityStatus::Reachable,
        remotes:            Vec::new(),
        worktrees:          Vec::new(),
    }
}

fn ci_run_with_jobs(jobs: Vec<CiJob>) -> CiRun {
    CiRun {
        run_id: 1,
        created_at: "2026-04-01T21:00:00-04:00".to_string(),
        branch: "feat/box-select".to_string(),
        url: "https://example.com/run/1".to_string(),
        ci_status: CiStatus::Passed,
        jobs,
        wall_clock_secs: Some(17),
        commit_title: Some("feat: add box select".to_string()),
        updated_at: None,
        fetched: Fetched,
    }
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn stats_width_cases() {
    let cases = [
        (
            "three_digit_counts",
            vec![("example", 999), ("lib", 1)],
            17,
            3,
        ),
        (
            "four_digit_counts",
            vec![("example", 1000), ("lib", 1)],
            18,
            4,
        ),
        ("short_labels", vec![("lib", 5), ("bin", 2)], 17, 3),
        ("empty_rows", vec![], 17, 3),
    ];

    for (name, rows, expected_total, expected_digits) in cases {
        let mut data = package_data(true);
        data.stats_rows = rows;
        let (total, digits) = panes::stats_column_width(&data);
        assert_eq!(total, expected_total, "{name}");
        assert_eq!(digits, expected_digits, "{name}");
    }
}

#[test]
fn package_fields_place_lint_and_ci_before_disk_for_rust_projects() {
    let data = package_data(true);
    // Step 4 adds Edition / License / Homepage / Repository at the
    // end of the Rust-package field list. They show unconditionally
    // (the pane renders `—` for unset values).
    assert_eq!(
        model::package_fields_from_data(&data)
            .into_iter()
            .map(DetailField::label)
            .collect::<Vec<_>>(),
        vec![
            "Path",
            "Disk",
            "Type",
            "Lint",
            "CI",
            "Version",
            "Edition",
            "License",
            "Homepage",
            "Repository",
        ]
    );
}

#[test]
fn package_fields_place_lint_and_ci_before_disk_for_non_rust_projects() {
    let data = package_data(false);
    assert_eq!(
        model::package_fields_from_data(&data)
            .into_iter()
            .map(DetailField::label)
            .collect::<Vec<_>>(),
        vec!["Path", "Disk", "Lint", "CI"]
    );
}

#[test]
fn package_label_width_matches_widest_visible_field() {
    let data = PackageData {
        crates_version: Some("0.0.3".to_string()),
        crates_downloads: Some(74),
        ..package_data(true)
    };
    let fields = model::package_fields_from_data(&data);
    let expected = fields.iter().map(|f| f.label().len()).max().unwrap_or(0);
    assert_eq!(panes::package_label_width(&fields), expected);
    assert!(
        expected >= "Repository".len(),
        "label column must be wide enough for Step 4 fields (Repository = 10 chars)"
    );
}

#[test]
fn description_lines_use_muted_fallback_when_missing() {
    let data = package_data(true);

    let lines = panes::description_lines(data.description.as_deref(), 80, 3);

    assert_eq!(lines.len(), 1);
    assert_eq!(line_text(&lines[0]), "No description available");
    assert_eq!(lines[0].spans[0].style.fg, Some(label_color()));
}

#[test]
fn description_lines_render_real_description_with_default_style() {
    let data = PackageData {
        description: Some("Real package description".to_string()),
        ..package_data(true)
    };

    let lines = panes::description_lines(data.description.as_deref(), 80, 3);

    assert_eq!(lines.len(), 1);
    assert_eq!(line_text(&lines[0]), "Real package description");
    assert_eq!(lines[0].spans[0].style.fg, None);
}

#[test]
fn description_lines_truncate_overflow_with_ellipsis() {
    let data = PackageData {
        description: Some("one two three four five six seven eight".to_string()),
        ..package_data(true)
    };

    let lines = panes::description_lines(data.description.as_deref(), 13, 2);

    assert_eq!(lines.len(), 2);
    assert_eq!(line_text(&lines[0]), "one two three");
    assert!(line_text(&lines[1]).ends_with('…'));
}

#[test]
fn detail_column_scroll_waits_until_cursor_reaches_bottom() {
    let focus = PaneFocusState::Active;

    assert_eq!(panes::detail_column_scroll_offset(focus, 0, 4), 0);
    assert_eq!(panes::detail_column_scroll_offset(focus, 3, 4), 0);
    assert_eq!(panes::detail_column_scroll_offset(focus, 4, 4), 1);
    assert_eq!(panes::detail_column_scroll_offset(focus, 7, 4), 4);
}

#[test]
fn detail_column_scroll_stays_at_top_when_not_active() {
    assert_eq!(
        panes::detail_column_scroll_offset(PaneFocusState::Remembered, 7, 4),
        0
    );
    assert_eq!(
        panes::detail_column_scroll_offset(PaneFocusState::Inactive, 7, 4),
        0
    );
}

#[test]
fn git_path_value_appends_status_icon() {
    let data = GitData {
        status: Some(GitStatus::Modified),
        ..git_data()
    };

    assert_eq!(DetailField::GitPath.git_value(&data), "🟠 modified");
}

#[test]
fn sync_value_uses_synced_label_when_in_sync() {
    assert_eq!(model::format_remote_status(Some((0, 0))), "☑️");
}

#[test]
fn git_label_width_uses_configured_main_label() {
    let data = GitData {
        vs_local: Some("↑11 ↓3".to_string()),
        main_branch_label: "primary".to_string(),
        ..git_data()
    };
    let fields = vec![DetailField::VsLocal];

    assert_eq!(
        panes::git_label_width(&data, &fields),
        "vs local primary".len()
    );
}

#[test]
fn ci_table_hides_durations_when_fixed_columns_overflow() {
    let runs = vec![ci_run_with_jobs(vec![
        CiJob {
            name:          "fmt".to_string(),
            ci_status:     CiStatus::Passed,
            duration:      "17s".to_string(),
            duration_secs: Some(17),
        },
        CiJob {
            name:          "clippy".to_string(),
            ci_status:     CiStatus::Passed,
            duration:      "21s".to_string(),
            duration_secs: Some(21),
        },
    ])];
    let cols = vec![CiColumn::Fmt, CiColumn::Clippy];

    assert!(!panes::ci_table_shows_durations(&runs, &cols, 20));
    assert_eq!(
        panes::ci_total_width(&runs, false),
        CI_COMPACT_DURATION_WIDTH
    );
}

#[test]
fn ci_table_keeps_durations_when_fixed_columns_fit() {
    let runs = vec![ci_run_with_jobs(vec![CiJob {
        name:          "fmt".to_string(),
        ci_status:     CiStatus::Passed,
        duration:      "17s".to_string(),
        duration_secs: Some(17),
    }])];
    let cols = vec![CiColumn::Fmt];

    assert!(panes::ci_table_shows_durations(&runs, &cols, 80));
}

// ── TargetsData::from_workspace_metadata ──────────────────────────────

#[cfg(test)]
mod targets_from_metadata {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;

    use crate::project::AbsolutePath;
    use crate::project::FileStamp;
    use crate::project::ManifestFingerprint;
    use crate::project::PackageRecord;
    use crate::project::PublishPolicy;
    use crate::project::TargetRecord;
    use crate::project::WorkspaceMetadata;
    use crate::tui::panes::TargetSource;
    use crate::tui::panes::TargetsData;

    fn target(name: &str, kinds: Vec<TargetKind>, src_path: &str) -> TargetRecord {
        TargetRecord {
            name: name.into(),
            kinds,
            src_path: AbsolutePath::from(PathBuf::from(src_path)),
        }
    }

    fn record(name: &str, manifest: &str, targets: Vec<TargetRecord>) -> PackageRecord {
        PackageRecord {
            name: name.into(),
            version: Version::new(0, 1, 0),
            edition: "2021".into(),
            description: None,
            license: None,
            homepage: None,
            repository: None,
            manifest_path: AbsolutePath::from(PathBuf::from(manifest)),
            targets,
            publish: PublishPolicy::Any,
        }
    }

    fn path(s: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(s)) }

    fn workspace(workspace_root: &str, packages: Vec<PackageRecord>) -> WorkspaceMetadata {
        let root = AbsolutePath::from(PathBuf::from(workspace_root));
        let mut map: HashMap<PackageId, PackageRecord> = HashMap::new();
        for pkg in packages {
            let id = PackageId {
                repr: format!("{}-test-id", pkg.name),
            };
            map.insert(id, pkg);
        }
        WorkspaceMetadata {
            workspace_root:           root.clone(),
            target_directory:         AbsolutePath::from(root.as_path().join("target")),
            packages:                 map,
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        }
    }

    #[test]
    fn groups_examples_by_subdirectory_and_sorts_root_first() {
        let pkg = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![
                target("top", vec![TargetKind::Example], "/ws/demo/examples/top.rs"),
                target(
                    "draw",
                    vec![TargetKind::Example],
                    "/ws/demo/examples/2d/draw.rs",
                ),
                target(
                    "mesh",
                    vec![TargetKind::Example],
                    "/ws/demo/examples/3d/mesh.rs",
                ),
                target(
                    "cube",
                    vec![TargetKind::Example],
                    "/ws/demo/examples/3d/cube.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![pkg]),
            &path("/ws/demo"),
        );

        let display_names: Vec<&str> = data
            .examples
            .iter()
            .map(|e| e.display_name.as_str())
            .collect();
        assert_eq!(
            display_names,
            vec!["top", "2d/draw", "3d/cube", "3d/mesh"],
            "root-level first, then categorized alphabetically"
        );
    }

    #[test]
    fn surfaces_benches_flat_and_sorted() {
        let pkg = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![
                target(
                    "b_zed",
                    vec![TargetKind::Bench],
                    "/ws/demo/benches/b_zed.rs",
                ),
                target(
                    "a_alpha",
                    vec![TargetKind::Bench],
                    "/ws/demo/benches/a_alpha.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![pkg]),
            &path("/ws/demo"),
        );
        let names: Vec<&str> = data.benches.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a_alpha", "b_zed"]);
    }

    #[test]
    fn standalone_package_uses_package_name_as_source_label() {
        // bevy_liminal etc. — a project with no `[workspace]` table
        // and a single package. Cargo still reports it with a
        // workspace_root pointing at the package dir, but the
        // Source column must say the package name, not "workspace".
        let pkg = record(
            "bevy_liminal",
            "/repo/bevy_liminal/Cargo.toml",
            vec![target(
                "bevy_liminal",
                vec![TargetKind::Bin],
                "/repo/bevy_liminal/src/main.rs",
            )],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/repo/bevy_liminal", vec![pkg]),
            &path("/repo/bevy_liminal"),
        );
        assert_eq!(data.binaries.len(), 1);
        assert_eq!(
            data.binaries[0].source,
            TargetSource::Member("bevy_liminal".into()),
            "standalone package must not borrow the misleading `workspace` label"
        );
    }

    #[test]
    fn primary_binary_matches_package_name_only() {
        // A bin target named "demo" is considered the default-run
        // binary; other bin targets are not lifted into the
        // workspace-aggregated binary list.
        let with_match = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![target(
                "demo",
                vec![TargetKind::Bin],
                "/ws/demo/src/main.rs",
            )],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![with_match]),
            &path("/ws/demo"),
        );
        assert_eq!(data.binaries.len(), 1);
        assert_eq!(data.binaries[0].name, "demo");

        let without_match = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![target(
                "other",
                vec![TargetKind::Bin],
                "/ws/demo/src/bin/other.rs",
            )],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![without_match]),
            &path("/ws/demo"),
        );
        assert!(
            data.binaries.is_empty(),
            "bin targets whose name != package name don't become primary"
        );
    }

    #[test]
    fn ignores_non_example_non_bench_non_bin_kinds() {
        let pkg = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![
                target("demo", vec![TargetKind::Lib], "/ws/demo/src/lib.rs"),
                target("it", vec![TargetKind::Test], "/ws/demo/tests/it.rs"),
                target(
                    "build-script",
                    vec![TargetKind::CustomBuild],
                    "/ws/demo/build.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/demo", vec![pkg]),
            &path("/ws/demo"),
        );
        assert!(data.binaries.is_empty());
        assert!(data.examples.is_empty());
        assert!(data.benches.is_empty());
    }

    /// Three-package workspace: root "ws-root" plus members "core"
    /// and "engine". Used by both the workspace-root and member-filter
    /// tests below.
    fn three_package_workspace() -> WorkspaceMetadata {
        let ws_root = record(
            "ws-root",
            "/ws/Cargo.toml",
            vec![
                target("ws-root", vec![TargetKind::Bin], "/ws/src/main.rs"),
                target(
                    "root-ex",
                    vec![TargetKind::Example],
                    "/ws/examples/root-ex.rs",
                ),
            ],
        );
        let core = record(
            "core",
            "/ws/crates/core/Cargo.toml",
            vec![
                target("core", vec![TargetKind::Bin], "/ws/crates/core/src/main.rs"),
                target(
                    "core-ex",
                    vec![TargetKind::Example],
                    "/ws/crates/core/examples/core-ex.rs",
                ),
            ],
        );
        let engine = record(
            "engine",
            "/ws/crates/engine/Cargo.toml",
            vec![target(
                "engine-ex",
                vec![TargetKind::Example],
                "/ws/crates/engine/examples/engine-ex.rs",
            )],
        );
        workspace("/ws", vec![ws_root, core, engine])
    }

    #[test]
    fn aggregates_targets_across_root_and_members_when_selected_is_workspace_root() {
        let metadata = three_package_workspace();
        let data = TargetsData::from_workspace_metadata(&metadata, &path("/ws"));

        let binary_sources: Vec<&TargetSource> = data.binaries.iter().map(|e| &e.source).collect();
        assert!(binary_sources.contains(&&TargetSource::Workspace));
        assert!(binary_sources.contains(&&TargetSource::Member("core".into())));
        assert_eq!(data.binaries.len(), 2);

        // Workspace bucket sorts before members.
        assert_eq!(data.examples[0].source, TargetSource::Workspace);
        assert_eq!(data.examples[0].name, "root-ex");
        // Members alphabetical: core before engine.
        assert_eq!(data.examples[1].source, TargetSource::Member("core".into()));
        assert_eq!(
            data.examples[2].source,
            TargetSource::Member("engine".into())
        );
    }

    #[test]
    fn filters_to_member_when_selected_is_a_member_path() {
        // When the selected project is a workspace member, the
        // Targets pane shows only that member's targets — selecting
        // sibling members or the workspace root surfaces a different
        // view. Confirms the user-visible "narrow on member" rule.
        let metadata = three_package_workspace();
        let data = TargetsData::from_workspace_metadata(&metadata, &path("/ws/crates/core"));

        assert_eq!(data.binaries.len(), 1, "only core's bin shows");
        assert_eq!(data.binaries[0].name, "core");
        assert_eq!(data.binaries[0].source, TargetSource::Member("core".into()));
        assert_eq!(data.examples.len(), 1);
        assert_eq!(data.examples[0].name, "core-ex");
        assert!(
            data.examples
                .iter()
                .all(|e| matches!(&e.source, TargetSource::Member(name) if name == "core")),
            "no entry from sibling members or the workspace root"
        );
    }

    #[test]
    fn member_filter_returns_empty_for_unknown_path() {
        // A selected path that doesn't match any member's manifest
        // dir produces an empty pane rather than falling back to the
        // workspace aggregation — selection must be unambiguous.
        let metadata = three_package_workspace();
        let data = TargetsData::from_workspace_metadata(&metadata, &path("/ws/crates/unknown"));

        assert!(data.binaries.is_empty());
        assert!(data.examples.is_empty());
        assert!(data.benches.is_empty());
    }

    #[test]
    fn virtual_workspace_has_no_workspace_source() {
        // No root package — only members. Selecting the workspace
        // root still aggregates both members, but no entry maps to
        // `TargetSource::Workspace`.
        let m1 = record(
            "m1",
            "/ws/crates/m1/Cargo.toml",
            vec![target(
                "m1-ex",
                vec![TargetKind::Example],
                "/ws/crates/m1/examples/m1-ex.rs",
            )],
        );
        let m2 = record(
            "m2",
            "/ws/crates/m2/Cargo.toml",
            vec![target(
                "m2-ex",
                vec![TargetKind::Example],
                "/ws/crates/m2/examples/m2-ex.rs",
            )],
        );
        let data =
            TargetsData::from_workspace_metadata(&workspace("/ws", vec![m1, m2]), &path("/ws"));

        assert!(
            data.examples
                .iter()
                .all(|e| !matches!(e.source, TargetSource::Workspace)),
            "no entry maps to Workspace when there's no root package"
        );
        assert_eq!(data.examples.len(), 2);
    }
}
