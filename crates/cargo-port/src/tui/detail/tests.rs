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
use crate::lint::LintCommand;
use crate::lint::LintCommandStatus;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::project::ExampleGroup;
use crate::project::GitPathState;
use crate::tui::render::CiColumn;

fn detail_info(is_rust_project: bool, lint_label: &str) -> DetailInfo {
    DetailInfo {
        package_title:    if is_rust_project {
            "Package".to_string()
        } else {
            "Project".to_string()
        },
        name:             "demo".to_string(),
        path:             "~/demo".to_string(),
        version:          "0.1.0".to_string(),
        description:      None,
        crates_version:   None,
        crates_downloads: None,
        types:            "lib".to_string(),
        disk:             "36.3 GiB".to_string(),
        lint_label:       lint_label.to_string(),
        ci:               None,
        stats_rows:       Vec::new(),
        git_branch:       None,
        git_path:         GitPathState::OutsideRepo,
        git_sync:         None,
        git_vs_origin:    None,
        git_vs_local:     None,
        default_branch:   None,
        git_origin:       None,
        git_owner:        None,
        git_url:          None,
        git_stars:        None,
        repo_description: None,
        git_inception:    None,
        git_last_commit:  None,
        worktree_label:   None,
        worktree_names:   Vec::new(),
        is_binary:        false,
        binary_name:      None,
        examples:         Vec::<ExampleGroup>::new(),
        benches:          Vec::new(),
        has_package:      true,
        cargo_active:     true,
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
    let info = detail_info(true, "🟢");
    assert_eq!(
        model::package_fields(&info)
            .into_iter()
            .map(DetailField::label)
            .collect::<Vec<_>>(),
        vec![
            "Name", "Path", "Targets", "Lint", "CI", "Disk", "Version", "Desc",
        ]
    );
}

#[test]
fn package_fields_place_lint_and_ci_before_disk_for_non_rust_projects() {
    let info = detail_info(false, "🔴");
    assert_eq!(
        model::package_fields(&info)
            .into_iter()
            .map(DetailField::label)
            .collect::<Vec<_>>(),
        vec!["Name", "Path", "Lint", "CI", "Disk"]
    );
}

#[test]
fn package_label_width_expands_for_crates_io() {
    let info = DetailInfo {
        crates_version: Some("0.0.3".to_string()),
        crates_downloads: Some(74),
        ..detail_info(true, "🟢")
    };
    let fields = model::package_fields(&info);
    assert_eq!(render::package_label_width(&fields), "crates.io".len());
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
fn lint_commands_summary_for_passed_run() {
    let run = run_with_commands(
        LintRunStatus::Passed,
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
                status:      LintCommandStatus::Passed,
                duration_ms: Some(2_000),
                exit_code:   Some(0),
                log_file:    "clippy-latest.log".to_string(),
            },
        ],
    );

    assert_eq!(lints_panel::format_lints_commands(&run), "mend, clippy");
    assert_eq!(lints_panel::format_lints_pending(&run), "0");
    assert_eq!(lints_panel::format_lints_slowest(&run), "clippy 0:02");
}

#[test]
fn lint_commands_summary_for_failed_run() {
    let run = run_with_commands(
        LintRunStatus::Failed,
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
                status:      LintCommandStatus::Failed,
                duration_ms: Some(2_000),
                exit_code:   Some(101),
                log_file:    "clippy-latest.log".to_string(),
            },
        ],
    );

    assert_eq!(lints_panel::format_lints_commands(&run), "mend, clippy");
    assert_eq!(lints_panel::format_lints_pending(&run), "0");
    assert_eq!(lints_panel::format_lints_slowest(&run), "clippy 0:02");
}

#[test]
fn lint_commands_summary_for_running_run() {
    let run = run_with_commands(
        LintRunStatus::Running,
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
                status:      LintCommandStatus::Pending,
                duration_ms: None,
                exit_code:   None,
                log_file:    "clippy-latest.log".to_string(),
            },
        ],
    );

    assert_eq!(lints_panel::format_lints_commands(&run), "mend, clippy");
    assert_eq!(lints_panel::format_lints_pending(&run), "1");
    assert_eq!(lints_panel::format_lints_slowest(&run), "mend 0:01");
}
