use ratatui::layout::Rect;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelectionResult;
use tui_pane::PaneFocusState;
use tui_pane::label_color;

use super::CiEmptyState;
use super::DetailField;
use super::EmptyDescriptionBehavior;
use super::GitData;
use super::LintsData;
use super::PackageData;
use super::PublishStatus;
use super::PullRequestRow;
use super::PullRequestSection;
use super::PullRequestSectionState;
use super::RemoteRow;
use super::RunTargetKind;
use super::TargetEntry;
use super::TargetSource;
use super::TargetsData;
use super::WorktreeInfo;
use super::pane_data as model;
use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::ci::FetchStatus::Fetched;
use crate::lint;
use crate::lint::LintCommand;
use crate::lint::LintCommandStatus;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::project::AbsolutePath;
use crate::project::BisectProgress;
use crate::project::GitStatus;
use crate::project::ProjectType;
use crate::tui::app::AvailabilityStatus;
use crate::tui::panes;
use crate::tui::state::ServiceStatus;

fn package_data(is_rust_project: bool) -> PackageData {
    PackageData {
        package_title:            if is_rust_project {
            "Package".to_string()
        } else {
            "Project".to_string()
        },
        title_name:               "demo".to_string(),
        worktree_group_summary:   None,
        primary_section:          None,
        path:                     "~/demo".to_string(),
        version:                  Some("0.1.0".to_string()),
        description:              None,
        crates_version:           None,
        crates_downloads:         None,
        publish_status:           PublishStatus::NotPublishable,
        crates_io_service:        ServiceStatus::Available,
        types:                    Some(vec![ProjectType::Library]),
        disk:                     Some(38_989_922_304),
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
        head:               None,
        head_relation:      None,
        bisect:             None,
        submodule_ctx:      None,
        status:             None,
        vs_local:           None,
        stars:              None,
        description:        None,
        inception:          None,
        last_commit:        None,
        last_fetched:       None,
        rate_limit_core:    None,
        rate_limit_graphql: None,
        github_status:      AvailabilityStatus::Reachable,
        pull_requests:      PullRequestSection::default(),
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

#[test]
fn crates_io_row_hidden_when_publishable_but_service_available() {
    // Pre-fetch state: no version yet, service is fine. We still
    // suppress the row — the existing UX before any fetch lands.
    let mut data = package_data(true);
    data.crates_version = None;
    data.crates_downloads = None;
    data.publish_status = PublishStatus::Publishable;
    data.crates_io_service = ServiceStatus::Available;
    let fields = model::package_fields_from_data(&data);
    assert!(
        !fields.contains(&DetailField::CratesIo),
        "no row before data lands while service is reachable"
    );
    assert!(!fields.contains(&DetailField::Downloads));
}

#[test]
fn crates_io_row_shows_warning_when_unreachable_and_no_version() {
    // Confirmed-down state with no version yet: the row surfaces
    // with the warning placeholder text so the user knows why the
    // value is empty.
    let mut data = package_data(true);
    data.crates_version = None;
    data.crates_downloads = None;
    data.publish_status = PublishStatus::Publishable;
    data.crates_io_service = ServiceStatus::Unreachable;
    let fields = model::package_fields_from_data(&data);
    assert!(
        fields.contains(&DetailField::CratesIo),
        "row must surface during outage so the user sees the placeholder"
    );
    assert!(fields.contains(&DetailField::Downloads));
    assert_eq!(
        DetailField::CratesIo.package_value(&data),
        "crates.io unreachable",
    );
    assert_eq!(
        DetailField::Downloads.package_value(&data),
        "crates.io unreachable",
    );
    assert!(model::crates_io_value_is_unreachable_placeholder(&data));
}

#[test]
fn crates_io_row_shows_normal_value_when_version_present_during_outage() {
    // Data landed before the outage (or during a brief recovery).
    // Even with the service currently unreachable, the row renders
    // the real value — not the warning placeholder.
    let mut data = package_data(true);
    data.crates_version = Some("0.1.0".to_string());
    data.crates_downloads = Some(123);
    data.publish_status = PublishStatus::Publishable;
    data.crates_io_service = ServiceStatus::Unreachable;
    let fields = model::package_fields_from_data(&data);
    assert!(fields.contains(&DetailField::CratesIo));
    assert!(fields.contains(&DetailField::Downloads));
    assert_eq!(DetailField::CratesIo.package_value(&data), "0.1.0");
    assert!(!model::crates_io_value_is_unreachable_placeholder(&data));
}

#[test]
fn crates_io_row_hidden_for_non_publishable_even_during_outage() {
    // `publish = false` packages never fire a crates.io fetch, so
    // surfacing the warning row for them would be misleading.
    let mut data = package_data(true);
    data.crates_version = None;
    data.crates_downloads = None;
    data.publish_status = PublishStatus::NotPublishable;
    data.crates_io_service = ServiceStatus::Unreachable;
    let fields = model::package_fields_from_data(&data);
    assert!(
        !fields.contains(&DetailField::CratesIo),
        "non-publishable rows must stay hidden during outage"
    );
    assert!(!fields.contains(&DetailField::Downloads));
    assert!(!model::crates_io_value_is_unreachable_placeholder(&data));
}

#[test]
fn stars_row_hidden_when_github_reachable_and_no_data() {
    // Pre-fetch state on a reachable GitHub: no stars yet, suppress
    // the row — same UX as the crates.io row pre-data on a healthy
    // service. The placeholder helper must report false.
    let mut data = git_data();
    data.stars = None;
    data.github_status = AvailabilityStatus::Reachable;
    let fields = model::git_fields_from_data(&data);
    assert!(
        !fields.contains(&DetailField::Stars),
        "no Stars row before data lands while GitHub is reachable"
    );
    assert!(!model::github_stars_is_unreachable_placeholder(&data));
}

#[test]
fn stars_row_shows_warning_when_github_unreachable_and_no_data() {
    // Outage state with no stars yet: the row surfaces with the
    // "github unreachable" placeholder so the user knows why the
    // value is empty. Mirrors `crates_io_row_shows_warning_...`.
    let mut data = git_data();
    data.stars = None;
    data.github_status = AvailabilityStatus::Unreachable;
    let fields = model::git_fields_from_data(&data);
    assert!(
        fields.contains(&DetailField::Stars),
        "Stars row must surface during outage so the user sees the placeholder"
    );
    assert!(model::github_stars_is_unreachable_placeholder(&data));
    assert!(
        DetailField::Stars.git_value(&data).is_empty(),
        "git_value stays empty — the placeholder is added by the renderer overlay"
    );
}

#[test]
fn stars_row_shows_warning_when_github_rate_limited_and_no_data() {
    // Rate-limit collapses to the same UX as unreachable on the
    // render side — the value cell isn't going to land, so warn the
    // user instead of leaving an invisible gap.
    let mut data = git_data();
    data.stars = None;
    data.github_status = AvailabilityStatus::RateLimited;
    let fields = model::git_fields_from_data(&data);
    assert!(fields.contains(&DetailField::Stars));
    assert!(model::github_stars_is_unreachable_placeholder(&data));
}

#[test]
fn stars_row_shows_real_value_when_data_present_during_outage() {
    // Stars landed before the outage (or during a brief recovery).
    // Even with GitHub currently unreachable, the row renders the
    // real value — not the warning placeholder.
    let mut data = git_data();
    data.stars = Some(42);
    data.github_status = AvailabilityStatus::Unreachable;
    let fields = model::git_fields_from_data(&data);
    assert!(fields.contains(&DetailField::Stars));
    assert_eq!(DetailField::Stars.git_value(&data), "⭐ 42");
    assert!(
        !model::github_stars_is_unreachable_placeholder(&data),
        "real value present — no placeholder"
    );
}

#[test]
fn stars_row_hidden_when_github_unauthenticated() {
    // Unauthenticated is not a service outage — don't surface the
    // "github unreachable" placeholder on the Stars row. The rate-limit
    // rows and the startup toast carry the `gh auth login` hint instead.
    let mut data = git_data();
    data.stars = None;
    data.github_status = AvailabilityStatus::Unauthenticated;
    let fields = model::git_fields_from_data(&data);
    assert!(
        !fields.contains(&DetailField::Stars),
        "no Stars placeholder when merely unauthenticated"
    );
    assert!(!model::github_stars_is_unreachable_placeholder(&data));
}

#[test]
fn package_copy_crates_io_row_uses_full_url() {
    let mut data = package_data(true);
    data.crates_version = Some("0.1.0".to_string());
    let rows = model::package_rows_from_data(&data);
    let pos = rows
        .iter()
        .position(|row| matches!(row, model::PackageRow::Field(DetailField::CratesIo)))
        .unwrap_or(usize::MAX);
    assert_ne!(pos, usize::MAX);

    assert_eq!(
        model::copy_payload_for_package(&data, pos),
        CopySelectionResult::Payload(CopyPayload::new(
            "https://crates.io/crates/demo",
            CopyLabel::Url,
        )),
    );
}

#[test]
fn package_copy_lint_and_ci_rows_return_nothing() {
    let data = package_data(true);
    let rows = model::package_rows_from_data(&data);
    for field in [DetailField::Lint, DetailField::Ci] {
        let pos = rows
            .iter()
            .position(|candidate| matches!(candidate, model::PackageRow::Field(candidate) if *candidate == field))
            .unwrap_or(usize::MAX);
        assert_ne!(pos, usize::MAX);
        assert_eq!(
            model::copy_payload_for_package(&data, pos),
            CopySelectionResult::Nothing,
        );
    }
}

#[test]
fn git_copy_remote_uses_full_url_and_worktree_uses_path() {
    let mut data = git_data();
    data.remotes.push(RemoteRow {
        name:            "origin".to_string(),
        icon:            "",
        display_url:     "github.com/natepiano/cargo-port".to_string(),
        tracked_ref:     "main".to_string(),
        status:          "ok".to_string(),
        full_url:        Some("https://github.com/natepiano/cargo-port".to_string()),
        push_annotation: None,
    });
    data.worktrees.push(WorktreeInfo {
        name:         "cargo-port_style_fix".to_string(),
        path:         "/Users/natemccoy/rust/cargo-port_style_fix".to_string(),
        branch:       Some("refactor/style".to_string()),
        ahead_behind: Some((0, 0)),
    });

    let remote_pos = model::git_fields_from_data(&data).len();
    let worktree_pos = remote_pos + data.remotes.len();

    assert_eq!(
        model::copy_payload_for_git(&data, remote_pos),
        CopySelectionResult::Payload(CopyPayload::new(
            "https://github.com/natepiano/cargo-port",
            CopyLabel::Url,
        )),
    );
    assert_eq!(
        model::copy_payload_for_git(&data, worktree_pos),
        CopySelectionResult::Payload(CopyPayload::new(
            "/Users/natemccoy/rust/cargo-port_style_fix",
            CopyLabel::Path,
        )),
    );
}

#[test]
fn git_copy_pull_request_uses_url_and_routes_before_remotes() {
    let mut data = git_data();
    data.pull_requests = PullRequestSection {
        state: PullRequestSectionState::Loaded,
        rows: vec![PullRequestRow {
            number:      128,
            title:       "Show vendored workspace member packages".to_string(),
            url:         "https://github.com/natepiano/cargo-port/pull/128".to_string(),
            state_label: "draft",
            is_polling:  false,
            branch:      "feature/member-vendored".to_string(),
            base:        "main".to_string(),
        }],
        ..PullRequestSection::default()
    };
    data.remotes.push(RemoteRow {
        name:            "origin".to_string(),
        icon:            "",
        display_url:     "github.com/natepiano/cargo-port".to_string(),
        tracked_ref:     "main".to_string(),
        status:          "ok".to_string(),
        full_url:        Some("https://github.com/natepiano/cargo-port".to_string()),
        push_annotation: None,
    });

    let pr_pos = model::git_fields_from_data(&data).len();
    let remote_pos = pr_pos + data.pull_requests.rows.len();

    assert!(matches!(
        model::git_row_at(&data, pr_pos),
        Some(model::GitRow::PullRequest(row)) if row.number == 128
    ));
    assert_eq!(
        model::copy_payload_for_git(&data, pr_pos),
        CopySelectionResult::Payload(CopyPayload::new(
            "https://github.com/natepiano/cargo-port/pull/128",
            CopyLabel::Url,
        )),
    );
    assert!(matches!(
        model::git_row_at(&data, remote_pos),
        Some(model::GitRow::Remote(_))
    ));
}

#[test]
fn ci_copy_returns_selected_run_url() {
    let data = super::CiData {
        runs:           vec![ci_run_with_jobs(Vec::new())],
        mode_label:     None,
        current_branch: None,
        empty_state:    CiEmptyState::NoRuns,
    };

    assert_eq!(
        model::copy_payload_for_ci(&data, 0),
        CopySelectionResult::Payload(CopyPayload::new(
            "https://example.com/run/1",
            CopyLabel::Url,
        )),
    );
    assert_eq!(
        model::copy_payload_for_ci(&data, 1),
        CopySelectionResult::Nothing,
    );
}

#[test]
fn targets_copy_returns_source_path_for_any_target_row() {
    let data = TargetsData {
        binaries: vec![TargetEntry {
            name:         "demo".to_string(),
            display_name: "demo".to_string(),
            kind:         RunTargetKind::Binary,
            source:       TargetSource::Workspace,
            src_path:     AbsolutePath::from("/ws/src/main.rs"),
        }],
        examples: vec![TargetEntry {
            name:         "demo_example".to_string(),
            display_name: "demo_example".to_string(),
            kind:         RunTargetKind::Example,
            source:       TargetSource::Workspace,
            src_path:     AbsolutePath::from("/ws/examples/demo_example.rs"),
        }],
        benches:  Vec::new(),
    };

    assert_eq!(
        model::copy_payload_for_targets(&data, 0, &|_| false),
        CopySelectionResult::Payload(CopyPayload::new(
            crate::project::normalize_test_path(std::path::Path::new("/ws/src/main.rs"))
                .display()
                .to_string(),
            CopyLabel::Path,
        )),
    );
    assert_eq!(
        model::copy_payload_for_targets(&data, 1, &|_| false),
        CopySelectionResult::Payload(CopyPayload::new(
            crate::project::normalize_test_path(std::path::Path::new(
                "/ws/examples/demo_example.rs"
            ))
            .display()
            .to_string(),
            CopyLabel::Path,
        )),
    );
}

#[test]
fn lints_copy_returns_selected_run_log_path() {
    let project_root = AbsolutePath::from("/Users/natemccoy/rust/demo");
    let data = LintsData {
        runs:    vec![LintRun {
            run_id:      "run-1".to_string(),
            started_at:  "2026-05-19T10:00:00-04:00".to_string(),
            finished_at: Some("2026-05-19T10:01:00-04:00".to_string()),
            duration_ms: Some(60_000),
            status:      LintRunStatus::Passed,
            commands:    vec![LintCommand {
                name:        "clippy".to_string(),
                command:     "cargo clippy".to_string(),
                status:      LintCommandStatus::Passed,
                duration_ms: Some(60_000),
                exit_code:   Some(0),
                log_file:    "runs/run-1/clippy.log".to_string(),
            }],
        }],
        sizes:   vec![Some(1024)],
        is_rust: true,
    };

    let expected = lint::project_dir(project_root.as_path())
        .join("runs/run-1/clippy.log")
        .display()
        .to_string();
    assert_eq!(
        model::copy_payload_for_lints(&data, 0, project_root.as_path()),
        CopySelectionResult::Payload(CopyPayload::new(expected, CopyLabel::Path)),
    );
    assert_eq!(
        model::copy_payload_for_lints(&data, 1, project_root.as_path()),
        CopySelectionResult::Nothing,
    );
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

/// Helper: outer pane area sized so `DescriptionBlock::for_pane` yields
/// the desired inner column width. Outer width = `inner_width` + 2 (borders)
/// + 2 (padding). Outer height = `inner_height` + 2 (borders).
fn description_area(column_width: u16, inner_height: u16) -> Rect {
    Rect {
        x:      0,
        y:      0,
        width:  column_width.saturating_add(4),
        height: inner_height.saturating_add(2),
    }
}

#[test]
fn description_block_uses_muted_placeholder_when_missing() {
    let data = package_data(true);
    let block = panes::DescriptionBlock::for_pane(
        data.description.as_deref(),
        description_area(80, 3),
        EmptyDescriptionBehavior::ShowPlaceholder,
    );

    assert_eq!(block.rows(), &[panes::placeholder_text().to_string()]);
    assert_eq!(block.style().fg, Some(label_color()));
}

#[test]
fn description_block_empty_behavior_render_empty_produces_no_rows() {
    let block = panes::DescriptionBlock::for_pane(
        None,
        description_area(80, 3),
        EmptyDescriptionBehavior::RenderEmpty,
    );

    assert!(block.rows().is_empty());
    assert_eq!(block.natural_sync_height(), 0);
}

#[test]
fn description_block_renders_real_description_with_default_style() {
    let data = PackageData {
        description: Some("Real package description".to_string()),
        ..package_data(true)
    };
    let block = panes::DescriptionBlock::for_pane(
        data.description.as_deref(),
        description_area(80, 3),
        EmptyDescriptionBehavior::ShowPlaceholder,
    );

    assert_eq!(block.rows(), &["Real package description".to_string()]);
    assert_eq!(block.style().fg, None);
}

#[test]
fn description_block_wraps_overflowing_text_into_rows() {
    let data = PackageData {
        description: Some("one two three four five six seven eight".to_string()),
        ..package_data(true)
    };
    let block = panes::DescriptionBlock::for_pane(
        data.description.as_deref(),
        description_area(13, 5),
        EmptyDescriptionBehavior::ShowPlaceholder,
    );

    // Pre-truncation rows — the render path's ellipsis is applied
    // when `max_height` clamps below `rows.len()`. natural_sync_height
    // reflects what feeds the inter-pane sync.
    assert!(block.rows().len() > 2);
    assert_eq!(block.rows()[0], "one two three");
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

    assert_eq!(DetailField::GitStatus.git_value(&data), "🟠 modified");
}

#[test]
fn git_bisect_value_mirrors_git_phrasing() {
    let data = GitData {
        bisect: Some(BisectProgress::Narrowing {
            revisions: 6,
            steps:     3,
        }),
        ..git_data()
    };

    assert_eq!(
        DetailField::Bisect.git_value(&data),
        "6 revisions left · ~3 steps"
    );
}

#[test]
fn git_bisect_value_pluralizes_singular_counts() {
    let data = GitData {
        bisect: Some(BisectProgress::Narrowing {
            revisions: 1,
            steps:     1,
        }),
        ..git_data()
    };

    assert_eq!(
        DetailField::Bisect.git_value(&data),
        "1 revision left · ~1 step"
    );
}

#[test]
fn git_bisect_awaiting_value_prompts_for_bounds() {
    let data = GitData {
        bisect: Some(BisectProgress::Awaiting),
        ..git_data()
    };

    assert_eq!(
        DetailField::Bisect.git_value(&data),
        "bisecting — mark a known-good & known-bad commit"
    );
}

#[test]
fn git_path_label_is_status() {
    assert_eq!(DetailField::GitStatus.label(), "Status");
}

#[test]
fn sync_value_uses_synced_label_when_in_sync() {
    assert_eq!(model::format_remote_status(Some((0, 0))), "☑️");
}

#[test]
fn local_ahead_behind_values_name_the_compared_branch() {
    let cases = [
        ((8, 0), "↑8 ahead of main"),
        ((0, 2), "↓2 behind main"),
        ((8, 2), "↑8 ↓2 diverged from main"),
        ((0, 0), "☑️ up to date with main"),
    ];

    for (ahead_behind, expected) in cases {
        assert_eq!(
            model::format_ahead_behind_against(ahead_behind, "main"),
            expected
        );
    }
}

#[test]
fn git_label_width_uses_ahead_behind_label() {
    let fields = vec![DetailField::VsLocal];

    assert_eq!(panes::git_label_width(&fields), "Ahead/Behind".len());
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
    fn multi_file_examples_are_not_categorized_by_their_own_directory() {
        let pkg = record(
            "bevy_window_manager",
            "/ws/bwm/Cargo.toml",
            vec![
                target(
                    "restore_window",
                    vec![TargetKind::Example],
                    "/ws/bwm/examples/restore_window/main.rs",
                ),
                target(
                    "custom_app_name",
                    vec![TargetKind::Example],
                    "/ws/bwm/examples/custom_app_name/main.rs",
                ),
            ],
        );
        let data = TargetsData::from_workspace_metadata(
            &workspace("/ws/bwm", vec![pkg]),
            &path("/ws/bwm"),
        );

        let display_names: Vec<&str> = data
            .examples
            .iter()
            .map(|e| e.display_name.as_str())
            .collect();
        assert_eq!(
            display_names,
            vec!["custom_app_name", "restore_window"],
            "examples/<name>/main.rs is the example's own directory, not a category"
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
