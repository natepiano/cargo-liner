use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Instant;

use ratatui::text::Line;

use super::ci_panel;
use super::ci_panel::CI_COMPACT_DURATION_WIDTH;
use super::lints_panel;
use super::model;
use super::model::DetailField;
use super::model::DetailInfo;
use super::render;
use crate::ci::CiJob;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::ci::FetchStatus::Fetched;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::lint::LintCommand;
use crate::lint::LintCommandStatus;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::project::ExampleGroup;
use crate::project::GitPathState;
use crate::project::WorktreeHealth::Normal;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::render::CiColumn;
use crate::tui::types::PaneFocusState;

fn test_http_client() -> HttpClient {
    static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = TEST_RT
        .get_or_init(|| tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort()));
    HttpClient::new(rt.handle().clone()).unwrap_or_else(|| std::process::abort())
}

fn test_app() -> App {
    let (bg_tx, bg_rx) = mpsc::channel::<BackgroundMsg>();
    let scan_root =
        std::env::temp_dir().join(format!("cargo-port-detail-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scan_root);
    App::new(
        scan_root,
        &[],
        bg_tx,
        bg_rx,
        &CargoPortConfig::default(),
        test_http_client(),
        Instant::now(),
    )
}

fn detail_info(is_rust_project: bool) -> DetailInfo {
    DetailInfo {
        package_title:     if is_rust_project {
            "Package".to_string()
        } else {
            "Project".to_string()
        },
        name:              "demo".to_string(),
        title_name:        "demo".to_string(),
        abs_path:          PathBuf::from("/tmp/demo"),
        path:              "~/demo".to_string(),
        version:           "0.1.0".to_string(),
        description:       None,
        crates_version:    None,
        crates_downloads:  None,
        types:             "lib".to_string(),
        disk:              "36.3 GiB".to_string(),
        ci:                None,
        stats_rows:        Vec::new(),
        git_branch:        None,
        git_path:          GitPathState::OutsideRepo,
        git_sync:          None,
        git_vs_origin:     None,
        git_vs_local:      None,
        local_main_branch: None,
        main_branch_label: "main".to_string(),
        git_origin:        None,
        git_owner:         None,
        git_url:           None,
        git_stars:         None,
        repo_description:  None,
        git_inception:     None,
        git_last_commit:   None,
        worktree_label:    None,
        worktree_health:   Normal,
        worktree_names:    Vec::new(),
        is_binary:         false,
        binary_name:       None,
        examples:          Vec::<ExampleGroup>::new(),
        benches:           Vec::new(),
        has_package:       true,
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

fn run_with_commands(status: LintRunStatus, commands: Vec<LintCommand>) -> LintRun {
    LintRun {
        run_id: "run-1".to_string(),
        started_at: "2026-04-01T21:00:00-04:00".to_string(),
        finished_at: Some("2026-04-01T21:00:10-04:00".to_string()),
        duration_ms: Some(10_000),
        status,
        commands,
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
        let (total, digits) = render::stats_column_width(&rows);
        assert_eq!(total, expected_total, "{name}");
        assert_eq!(digits, expected_digits, "{name}");
    }
}

#[test]
fn package_fields_place_lint_and_ci_before_disk_for_rust_projects() {
    let info = detail_info(true);
    assert_eq!(
        model::package_fields(&info)
            .into_iter()
            .map(DetailField::label)
            .collect::<Vec<_>>(),
        vec!["Path", "Targets", "Lint", "CI", "Disk", "Version"]
    );
}

#[test]
fn package_fields_place_lint_and_ci_before_disk_for_non_rust_projects() {
    let info = detail_info(false);
    assert_eq!(
        model::package_fields(&info)
            .into_iter()
            .map(DetailField::label)
            .collect::<Vec<_>>(),
        vec!["Path", "Lint", "CI", "Disk"]
    );
}

#[test]
fn package_label_width_expands_for_crates_io() {
    let info = DetailInfo {
        crates_version: Some("0.0.3".to_string()),
        crates_downloads: Some(74),
        ..detail_info(true)
    };
    let fields = model::package_fields(&info);
    assert_eq!(render::package_label_width(&fields), "crates.io".len());
}

#[test]
fn project_panel_title_uses_title_name() {
    let info = DetailInfo {
        package_title: "Workspace".to_string(),
        name: "-".to_string(),
        title_name: "hana".to_string(),
        ..detail_info(true)
    };

    assert_eq!(render::project_panel_title(&info), " Workspace - hana ");
}

#[test]
fn description_lines_use_muted_fallback_when_missing() {
    let info = detail_info(true);

    let lines = render::description_lines(&info, 80, 3);

    assert_eq!(lines.len(), 1);
    assert_eq!(line_text(&lines[0]), "No description available");
    assert_eq!(lines[0].spans[0].style.fg, Some(LABEL_COLOR));
}

#[test]
fn description_lines_render_real_description_with_default_style() {
    let info = DetailInfo {
        description: Some("Real package description".to_string()),
        ..detail_info(true)
    };

    let lines = render::description_lines(&info, 80, 3);

    assert_eq!(lines.len(), 1);
    assert_eq!(line_text(&lines[0]), "Real package description");
    assert_eq!(lines[0].spans[0].style.fg, None);
}

#[test]
fn description_lines_truncate_overflow_with_ellipsis() {
    let info = DetailInfo {
        description: Some("one two three four five six seven eight".to_string()),
        ..detail_info(true)
    };

    let lines = render::description_lines(&info, 13, 2);

    assert_eq!(lines.len(), 2);
    assert_eq!(line_text(&lines[0]), "one two three");
    assert!(line_text(&lines[1]).ends_with('…'));
}

#[test]
fn detail_column_scroll_waits_until_cursor_reaches_bottom() {
    let focus = PaneFocusState::Active;

    assert_eq!(render::detail_column_scroll_offset(focus, 0, 4), 0);
    assert_eq!(render::detail_column_scroll_offset(focus, 3, 4), 0);
    assert_eq!(render::detail_column_scroll_offset(focus, 4, 4), 1);
    assert_eq!(render::detail_column_scroll_offset(focus, 7, 4), 4);
}

#[test]
fn detail_column_scroll_stays_at_top_when_not_active() {
    assert_eq!(
        render::detail_column_scroll_offset(PaneFocusState::Remembered, 7, 4),
        0
    );
    assert_eq!(
        render::detail_column_scroll_offset(PaneFocusState::Inactive, 7, 4),
        0
    );
}

#[test]
fn git_path_value_appends_status_icon() {
    let app = test_app();
    let info = DetailInfo {
        git_path: GitPathState::Modified,
        ..detail_info(true)
    };

    assert_eq!(DetailField::GitPath.value(&info, &app), "🟠 modified");
}

#[test]
fn sync_value_uses_synced_label_when_in_sync() {
    assert_eq!(model::format_remote_status(Some((0, 0))), "☑️");
}

#[test]
fn git_label_width_uses_origin_and_configured_main_labels() {
    let info = DetailInfo {
        git_vs_origin: Some("origin/main (local cached ref)".to_string()),
        git_vs_local: Some("↑11 ↓3".to_string()),
        main_branch_label: "primary".to_string(),
        ..detail_info(true)
    };
    let fields = vec![DetailField::VsOrigin, DetailField::VsLocal];

    assert_eq!(
        render::git_label_width(&info, &fields),
        "vs local primary".len()
    );
}

#[test]
fn git_fields_show_explicit_remote_and_local_rows_for_unpublished_branch() {
    let info = DetailInfo {
        git_sync: Some(crate::constants::NO_REMOTE_SYNC.to_string()),
        git_vs_origin: Some("none".to_string()),
        git_vs_local: Some("↑11 ↓3".to_string()),
        ..detail_info(true)
    };

    assert_eq!(
        model::git_fields(&info),
        vec![
            DetailField::VsOrigin,
            DetailField::Sync,
            DetailField::VsLocal
        ]
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

    assert!(!ci_panel::ci_table_shows_durations(&runs, &cols, 20));
    assert_eq!(
        ci_panel::ci_total_width(&runs, false),
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

    assert!(ci_panel::ci_table_shows_durations(&runs, &cols, 80));
}

#[test]
fn lint_commands_summary_cases() {
    struct Case {
        name:               &'static str,
        status:             LintRunStatus,
        clippy_status:      LintCommandStatus,
        clippy_duration_ms: Option<u64>,
        clippy_exit_code:   Option<i32>,
        expected_pending:   &'static str,
        expected_slowest:   &'static str,
    }

    let cases = [
        Case {
            name:               "passed",
            status:             LintRunStatus::Passed,
            clippy_status:      LintCommandStatus::Passed,
            clippy_duration_ms: Some(2_000),
            clippy_exit_code:   Some(0),
            expected_pending:   "0",
            expected_slowest:   "clippy 0:02",
        },
        Case {
            name:               "failed",
            status:             LintRunStatus::Failed,
            clippy_status:      LintCommandStatus::Failed,
            clippy_duration_ms: Some(2_000),
            clippy_exit_code:   Some(101),
            expected_pending:   "0",
            expected_slowest:   "clippy 0:02",
        },
        Case {
            name:               "running",
            status:             LintRunStatus::Running,
            clippy_status:      LintCommandStatus::Pending,
            clippy_duration_ms: None,
            clippy_exit_code:   None,
            expected_pending:   "1",
            expected_slowest:   "mend 0:01",
        },
    ];

    for case in cases {
        let run = run_with_commands(
            case.status,
            vec![
                LintCommand {
                    name:        "mend".to_string(),
                    command:     "cargo mend".to_string(),
                    status:      LintCommandStatus::Passed,
                    duration_ms: Some(1_000),
                    exit_code:   Some(0),
                    log_file:    "mend-latest.log".to_string(),
                },
                LintCommand {
                    name:        "clippy".to_string(),
                    command:     "cargo clippy".to_string(),
                    status:      case.clippy_status,
                    duration_ms: case.clippy_duration_ms,
                    exit_code:   case.clippy_exit_code,
                    log_file:    "clippy-latest.log".to_string(),
                },
            ],
        );

        assert_eq!(
            lints_panel::format_lints_commands(&run),
            "mend, clippy",
            "{}",
            case.name
        );
        assert_eq!(
            lints_panel::format_lints_pending(&run),
            case.expected_pending,
            "{}",
            case.name
        );
        assert_eq!(
            lints_panel::format_lints_slowest(&run),
            case.expected_slowest,
            "{}",
            case.name
        );
    }
}
