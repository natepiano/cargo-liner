use ratatui::text::Line;

use super::CI_COMPACT_DURATION_WIDTH;
use super::DetailField;
use super::GitData;
use super::PackageData;
use super::support as model;
use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::ci::FetchStatus::Fetched;
use crate::project::GitStatus;
use crate::tui::app::AvailabilityStatus;
use crate::tui::constants::LABEL_COLOR;
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
        ci:                       None,
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
        ci_display:               String::new(),
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
        conclusion: Conclusion::Success,
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
    assert_eq!(lines[0].spans[0].style.fg, Some(LABEL_COLOR));
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
            conclusion:    Conclusion::Success,
            duration:      "17s".to_string(),
            duration_secs: Some(17),
        },
        CiJob {
            name:          "clippy".to_string(),
            conclusion:    Conclusion::Success,
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
        conclusion:    Conclusion::Success,
        duration:      "17s".to_string(),
        duration_secs: Some(17),
    }])];
    let cols = vec![CiColumn::Fmt];

    assert!(panes::ci_table_shows_durations(&runs, &cols, 80));
}

// ── TargetsData::from_package_record ──────────────────────────────────

#[cfg(test)]
mod targets_from_snapshot {
    use std::path::PathBuf;

    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;

    use crate::project::AbsolutePath;
    use crate::project::PackageRecord;
    use crate::project::PublishPolicy;
    use crate::project::TargetRecord;
    use crate::tui::panes::TargetsData;

    fn target(name: &str, kinds: Vec<TargetKind>, src_path: &str) -> TargetRecord {
        TargetRecord {
            name: name.into(),
            kinds,
            src_path: AbsolutePath::from(PathBuf::from(src_path)),
            edition: "2021".into(),
            required_features: Vec::new(),
        }
    }

    fn record(name: &str, manifest: &str, targets: Vec<TargetRecord>) -> PackageRecord {
        PackageRecord {
            id: PackageId {
                repr: format!("{name}-id"),
            },
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
        let data = TargetsData::from_package_record(&pkg, "demo");

        assert_eq!(data.examples.len(), 3, "root + 2d + 3d");
        assert!(
            data.examples[0].category.is_empty(),
            "root-level group first (Bevy-style convention)"
        );
        assert_eq!(data.examples[0].names, vec!["top"]);
        assert_eq!(data.examples[1].category, "2d");
        assert_eq!(data.examples[1].names, vec!["draw"]);
        assert_eq!(data.examples[2].category, "3d");
        assert_eq!(data.examples[2].names, vec!["cube", "mesh"]);
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
        let data = TargetsData::from_package_record(&pkg, "demo");
        assert_eq!(data.benches, vec!["a_alpha", "b_zed"]);
    }

    #[test]
    fn primary_binary_matches_title_name_only() {
        // A bin target named "demo" is considered the default-run
        // binary; other bin targets are not lifted into
        // primary_binary (they'd show up in a separate bin list
        // that today isn't surfaced by TargetsData).
        let with_match = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![target(
                "demo",
                vec![TargetKind::Bin],
                "/ws/demo/src/main.rs",
            )],
        );
        assert_eq!(
            TargetsData::from_package_record(&with_match, "demo").primary_binary,
            Some("demo".to_string())
        );

        let without_match = record(
            "demo",
            "/ws/demo/Cargo.toml",
            vec![target(
                "other",
                vec![TargetKind::Bin],
                "/ws/demo/src/bin/other.rs",
            )],
        );
        assert_eq!(
            TargetsData::from_package_record(&without_match, "demo").primary_binary,
            None,
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
        let data = TargetsData::from_package_record(&pkg, "demo");
        assert!(data.primary_binary.is_none());
        assert!(data.examples.is_empty());
        assert!(data.benches.is_empty());
    }
}
