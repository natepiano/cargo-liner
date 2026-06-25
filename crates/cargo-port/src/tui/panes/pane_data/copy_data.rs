use super::CiData;
use super::CopyLabel;
use super::CopyPayload;
use super::CopySelectionResult;
use super::DetailField;
use super::GitData;
use super::GitRow;
use super::GitStatus;
use super::HeadState;
use super::LintsData;
use super::PackageData;
use super::PackageRow;
use super::TargetsData;
use super::git_row_at;
use super::package_rows_from_data;
use crate::lint;
use crate::tui::panes;

fn copyable_text(text: impl Into<String>) -> Option<String> {
    let text = text.into();
    let trimmed = text.trim();
    if trimmed.is_empty() || matches!(trimmed, "-" | "—") {
        None
    } else {
        Some(text)
    }
}

fn copy_payload(text: impl Into<String>, label: CopyLabel) -> CopySelectionResult {
    copyable_text(text).map_or(CopySelectionResult::Nothing, |text| {
        CopySelectionResult::Payload(CopyPayload::new(text, label))
    })
}

/// The crates.io URL for the project, or `Nothing` when there is no
/// usable crate name.
fn crates_io_url_payload(data: &PackageData) -> CopySelectionResult {
    if data.name.trim().is_empty() || data.name == "-" {
        CopySelectionResult::Nothing
    } else {
        copy_payload(
            format!("https://crates.io/crates/{}", data.name),
            CopyLabel::Url,
        )
    }
}

pub fn copy_payload_for_package(data: &PackageData, pos: usize) -> CopySelectionResult {
    let Some(row) = package_rows_from_data(data).get(pos).copied() else {
        return CopySelectionResult::Nothing;
    };
    let PackageRow::Field(field) = row else {
        return match row {
            PackageRow::Description => copy_payload(
                data.description.as_deref().unwrap_or_default(),
                CopyLabel::Value,
            ),
            PackageRow::Structure(index) => {
                let Some((label, count)) = data.stats_rows.get(index) else {
                    return CopySelectionResult::Nothing;
                };
                copy_payload(format!("{count} {label}"), CopyLabel::Value)
            },
            PackageRow::Tests(index) => {
                let Some((label, count)) = data.test_rows.get(index) else {
                    return CopySelectionResult::Nothing;
                };
                copy_payload(format!("{count} {label}"), CopyLabel::Value)
            },
            // Every crates.io row copies the crate's crates.io URL.
            PackageRow::CratesIo(_) => crates_io_url_payload(data),
            PackageRow::Section(_) | PackageRow::Field(_) => CopySelectionResult::Nothing,
        };
    };
    match field {
        DetailField::Lint | DetailField::Ci => CopySelectionResult::Nothing,
        DetailField::Path | DetailField::GitStatus => {
            copy_payload(field.package_value(data), CopyLabel::Path)
        },
        DetailField::Homepage | DetailField::Repository => {
            copy_payload(field.package_value(data), CopyLabel::Url)
        },
        _ => copy_payload(field.package_value(data), CopyLabel::Value),
    }
}

pub fn copy_payload_for_git(data: &GitData, pos: usize) -> CopySelectionResult {
    match git_row_at(data, pos) {
        Some(GitRow::Description(description)) => copy_payload(description, CopyLabel::Value),
        Some(GitRow::Field(field)) => {
            copy_payload(git_field_copy_value(data, field), CopyLabel::Value)
        },
        Some(GitRow::PullRequest(pull_request)) => copy_payload(&pull_request.url, CopyLabel::Url),
        Some(GitRow::Remote(remote)) => copy_payload(
            remote
                .full_url
                .as_deref()
                .unwrap_or(remote.display_url.as_str()),
            CopyLabel::Url,
        ),
        Some(GitRow::Worktree(worktree)) => copy_payload(&worktree.path, CopyLabel::Path),
        None => CopySelectionResult::Nothing,
    }
}

fn git_field_copy_value(data: &GitData, field: DetailField) -> String {
    match field {
        DetailField::Head => match data.head.as_ref() {
            Some(HeadState::Branch(name)) => name.clone(),
            Some(HeadState::Detached { short_sha }) => short_sha.clone(),
            Some(HeadState::Unborn) | None => String::new(),
        },
        DetailField::GitStatus => data
            .status
            .map_or_else(String::new, GitStatus::label_with_icon),
        DetailField::Tracks => data
            .submodule_ctx
            .as_ref()
            .and_then(|context| context.tracks.clone())
            .unwrap_or_default(),
        DetailField::Pinned => data
            .submodule_ctx
            .as_ref()
            .map(|context| context.pinned_commit.clone())
            .unwrap_or_default(),
        _ => field.git_value(data),
    }
}

pub fn copy_payload_for_ci(data: &CiData, pos: usize) -> CopySelectionResult {
    let Some(run) = data.runs.get(pos) else {
        return CopySelectionResult::Nothing;
    };
    copy_payload(&run.url, CopyLabel::Url)
}

/// Join the snapshot rows in the inclusive `[min(anchor, cursor),
/// max(anchor, cursor)]` range, ANSI-stripped, into a clipboard payload.
/// `anchor` and `cursor` are clamped to the snapshot bounds. Returns
/// `Nothing` for an empty snapshot or an all-blank range.
pub fn copy_payload_for_output(
    snapshot: &[String],
    anchor: usize,
    cursor: usize,
) -> CopySelectionResult {
    let Some(last) = snapshot.len().checked_sub(1) else {
        return CopySelectionResult::Nothing;
    };
    let lo = anchor.min(cursor).min(last);
    let hi = anchor.max(cursor).min(last);
    let text = snapshot[lo..=hi]
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n");
    copy_payload(text, CopyLabel::Row)
}

/// Strip ANSI escape sequences from `raw`, leaving only the printable
/// text. Reuses the same parser the output renderer feeds, so the copied
/// text matches what is on screen.
pub(super) fn strip_ansi(raw: &str) -> String {
    let safe = sanitize_ansi_for_output(raw);
    ansi_to_tui::IntoText::into_text(&safe).map_or_else(
        |_| strip_control_sequences(&safe),
        |text| {
            text.lines
                .iter()
                .map(|line| {
                    line.spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

pub(super) fn sanitize_ansi_for_output(raw: &str) -> String { sanitize_ansi(raw, true) }

fn strip_control_sequences(raw: &str) -> String { sanitize_ansi(raw, false) }

fn sanitize_ansi(raw: &str, preserve_sgr: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut state = EscapeStripState::Ground;
    for ch in raw.chars() {
        state = state.consume(ch, &mut out, preserve_sgr);
    }
    out
}

enum EscapeStripState {
    Ground,
    Escape,
    Csi(String),
    ControlString,
    ControlStringEscape,
}

impl EscapeStripState {
    fn consume(self, ch: char, out: &mut String, preserve_sgr: bool) -> Self {
        match self {
            Self::Ground => consume_ground(ch, out),
            Self::Escape => consume_escape(ch),
            Self::Csi(mut sequence) => {
                sequence.push(ch);
                if is_csi_final(ch) {
                    if preserve_sgr && ch == 'm' {
                        out.push_str(&sequence);
                    }
                    Self::Ground
                } else {
                    Self::Csi(sequence)
                }
            },
            Self::ControlString => match ch {
                '\x07' => Self::Ground,
                '\x1b' => Self::ControlStringEscape,
                _ => Self::ControlString,
            },
            Self::ControlStringEscape => {
                if ch == '\\' {
                    Self::Ground
                } else {
                    Self::ControlString
                }
            },
        }
    }
}

fn consume_ground(ch: char, out: &mut String) -> EscapeStripState {
    match ch {
        '\x1b' => EscapeStripState::Escape,
        '\t' => {
            out.push(' ');
            EscapeStripState::Ground
        },
        _ if ch.is_control() => EscapeStripState::Ground,
        _ => {
            out.push(ch);
            EscapeStripState::Ground
        },
    }
}

fn consume_escape(ch: char) -> EscapeStripState {
    match ch {
        '[' => EscapeStripState::Csi("\x1b[".to_string()),
        ']' | 'P' | 'X' | '^' | '_' => EscapeStripState::ControlString,
        _ => EscapeStripState::Ground,
    }
}

const fn is_csi_final(ch: char) -> bool { matches!(ch, '\u{40}'..='\u{7e}') }

pub fn copy_payload_for_targets(data: &TargetsData, pos: usize) -> CopySelectionResult {
    let entries = panes::build_target_list_from_data(data);
    let Some(entry) = entries.get(pos) else {
        return CopySelectionResult::Nothing;
    };
    copy_payload(entry.src_path.display().to_string(), CopyLabel::Path)
}

pub fn copy_payload_for_lints(data: &LintsData, pos: usize) -> CopySelectionResult {
    let Some(run) = data.runs.get(pos) else {
        return CopySelectionResult::Nothing;
    };
    let Some(command) = run.commands.first() else {
        return CopySelectionResult::Nothing;
    };
    // Resolve against the checkout the run came from, so a worktree-group
    // aggregate copies each run's log from its own checkout, not the
    // primary's.
    let Some(project_root) = data.owner_path_for_run(pos) else {
        return CopySelectionResult::Nothing;
    };
    copy_payload(
        lint::project_dir(project_root.as_path())
            .join(&command.log_file)
            .display()
            .to_string(),
        CopyLabel::Path,
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
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
    use crate::project::ProjectType;
    use crate::tui::app::AvailabilityStatus;
    use crate::tui::panes::pane_data;
    use crate::tui::panes::pane_data::CiEmptyState;
    use crate::tui::panes::pane_data::LintsProjectKind;
    use crate::tui::panes::pane_data::PackagePresence;
    use crate::tui::panes::pane_data::PullRequestPolling;
    use crate::tui::panes::pane_data::PullRequestRow;
    use crate::tui::panes::pane_data::PullRequestSection;
    use crate::tui::panes::pane_data::PullRequestSectionState;
    use crate::tui::panes::pane_data::RemoteRow;
    use crate::tui::panes::pane_data::RunTargetKind;
    use crate::tui::panes::pane_data::TargetEntry;
    use crate::tui::panes::pane_data::TargetSource;
    use crate::tui::panes::pane_data::WorktreeInfo;
    use crate::tui::state::CiDisplay;
    use crate::tui::state::LintDisplay;

    fn package_data(is_rust_project: bool) -> PackageData {
        PackageData {
            title:                    if is_rust_project {
                "Package".to_string()
            } else {
                "Project".to_string()
            },
            name:                     "demo".to_string(),
            worktree_group_summary:   None,
            primary_section:          None,
            path:                     "~/demo".to_string(),
            version:                  Some("0.1.0".to_string()),
            description:              None,
            crates_io_rows:           Vec::new(),
            types:                    Some(vec![ProjectType::Library]),
            disk:                     Some(38_989_922_304),
            stats_rows:               Vec::new(),
            test_rows:                Vec::new(),
            package_presence:         PackagePresence::Present,
            edition:                  None,
            license:                  None,
            homepage:                 None,
            repository:               None,
            in_project_target:        None,
            in_project_non_target:    None,
            out_of_tree_target_bytes: None,
            lint_display:             LintDisplay::default(),
            ci_display:               CiDisplay::default(),
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
    fn package_copy_crates_io_row_uses_full_url() {
        let mut data = package_data(true);
        data.crates_io_rows = vec![("version", "0.1.0".to_string())];
        let rows = package_rows_from_data(&data);
        let pos = rows
            .iter()
            .position(|row| matches!(row, PackageRow::CratesIo(_)))
            .unwrap_or(usize::MAX);
        assert_ne!(pos, usize::MAX);

        assert_eq!(
            copy_payload_for_package(&data, pos),
            CopySelectionResult::Payload(CopyPayload::new(
                "https://crates.io/crates/demo",
                CopyLabel::Url,
            )),
        );
    }

    #[test]
    fn package_copy_lint_and_ci_rows_return_nothing() {
        let data = package_data(true);
        let rows = package_rows_from_data(&data);
        for field in [DetailField::Lint, DetailField::Ci] {
            let pos = rows
                .iter()
                .position(|candidate| matches!(candidate, PackageRow::Field(candidate) if *candidate == field))
                .unwrap_or(usize::MAX);
            assert_ne!(pos, usize::MAX);
            assert_eq!(
                copy_payload_for_package(&data, pos),
                CopySelectionResult::Nothing
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
            branch:          "main".to_string(),
            tracked_ref:     "main".to_string(),
            status:          "ok".to_string(),
            full_url:        Some("https://github.com/natepiano/cargo-port".to_string()),
            push_annotation: None,
        });
        data.worktrees.push(WorktreeInfo {
            name:         "cargo-port_style_fix".to_string(),
            path:         "/Users/natemccoy/rust/cargo-port_style_fix".to_string(),
            branch:       Some("refactor/style".to_string()),
            tracked:      Some("main".to_string()),
            ahead_behind: Some((0, 0)),
        });

        let remote_pos = pane_data::git_fields_from_data(&data).len();
        let worktree_pos = remote_pos + data.remotes.len();

        assert_eq!(
            copy_payload_for_git(&data, remote_pos),
            CopySelectionResult::Payload(CopyPayload::new(
                "https://github.com/natepiano/cargo-port",
                CopyLabel::Url,
            )),
        );
        assert_eq!(
            copy_payload_for_git(&data, worktree_pos),
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
                polling:     PullRequestPolling::Idle,
                branch:      "feature/member-vendored".to_string(),
                base:        "main".to_string(),
            }],
            ..PullRequestSection::default()
        };
        data.remotes.push(RemoteRow {
            name:            "origin".to_string(),
            icon:            "",
            display_url:     "github.com/natepiano/cargo-port".to_string(),
            branch:          "main".to_string(),
            tracked_ref:     "main".to_string(),
            status:          "ok".to_string(),
            full_url:        Some("https://github.com/natepiano/cargo-port".to_string()),
            push_annotation: None,
        });

        let pr_pos = pane_data::git_fields_from_data(&data).len();
        let remote_pos = pr_pos + data.pull_requests.rows.len();

        assert!(matches!(
            git_row_at(&data, pr_pos),
            Some(GitRow::PullRequest(row)) if row.number == 128
        ));
        assert_eq!(
            copy_payload_for_git(&data, pr_pos),
            CopySelectionResult::Payload(CopyPayload::new(
                "https://github.com/natepiano/cargo-port/pull/128",
                CopyLabel::Url,
            )),
        );
        assert!(matches!(
            git_row_at(&data, remote_pos),
            Some(GitRow::Remote(_))
        ));
    }

    #[test]
    fn ci_copy_returns_selected_run_url() {
        let data = CiData {
            runs:           vec![ci_run_with_jobs(Vec::new())],
            mode_label:     None,
            current_branch: None,
            empty_state:    CiEmptyState::NoRuns,
        };

        assert_eq!(
            copy_payload_for_ci(&data, 0),
            CopySelectionResult::Payload(CopyPayload::new(
                "https://example.com/run/1",
                CopyLabel::Url,
            )),
        );
        assert_eq!(copy_payload_for_ci(&data, 1), CopySelectionResult::Nothing);
    }

    #[test]
    fn targets_copy_returns_source_path_for_any_target_row() {
        let data = TargetsData {
            binaries: vec![TargetEntry {
                name:              "demo".to_string(),
                display_name:      "demo".to_string(),
                run_target_kind:   RunTargetKind::Binary,
                source:            TargetSource::workspace_root("demo".into()),
                project_path:      AbsolutePath::from("/ws"),
                package_name:      "demo".to_string(),
                src_path:          AbsolutePath::from("/ws/src/main.rs"),
                required_features: Vec::new(),
            }],
            examples: vec![TargetEntry {
                name:              "demo_example".to_string(),
                display_name:      "demo_example".to_string(),
                run_target_kind:   RunTargetKind::Example,
                source:            TargetSource::workspace_root("demo".into()),
                project_path:      AbsolutePath::from("/ws"),
                package_name:      "demo".to_string(),
                src_path:          AbsolutePath::from("/ws/examples/demo_example.rs"),
                required_features: Vec::new(),
            }],
            benches:  Vec::new(),
        };

        assert_eq!(
            copy_payload_for_targets(&data, 0),
            CopySelectionResult::Payload(CopyPayload::new(
                crate::project::normalize_test_path(std::path::Path::new("/ws/src/main.rs"))
                    .display()
                    .to_string(),
                CopyLabel::Path,
            )),
        );
        assert_eq!(
            copy_payload_for_targets(&data, 1),
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
            runs:         vec![LintRun {
                run_id:        "run-1".to_string(),
                started_at:    "2026-05-19T10:00:00-04:00".to_string(),
                finished_at:   Some("2026-05-19T10:01:00-04:00".to_string()),
                duration_ms:   Some(60_000),
                status:        LintRunStatus::Passed,
                commands:      vec![LintCommand {
                    name:        "clippy".to_string(),
                    command:     "cargo clippy".to_string(),
                    status:      LintCommandStatus::Passed,
                    duration_ms: Some(60_000),
                    exit_code:   Some(0),
                    log_file:    "runs/run-1/clippy.log".to_string(),
                }],
                archive_bytes: 0,
            }],
            sizes:        vec![Some(1024)],
            owner_paths:  vec![project_root.clone()],
            owner_of:     vec![0],
            project_kind: LintsProjectKind::Rust,
        };

        let expected = lint::project_dir(project_root.as_path())
            .join("runs/run-1/clippy.log")
            .display()
            .to_string();
        assert_eq!(
            copy_payload_for_lints(&data, 0),
            CopySelectionResult::Payload(CopyPayload::new(expected, CopyLabel::Path)),
        );
        assert_eq!(
            copy_payload_for_lints(&data, 1),
            CopySelectionResult::Nothing
        );
    }

    #[test]
    fn output_copy_joins_range_and_strips_ansi() {
        let snapshot = [
            "first".to_string(),
            "\u{1b}[31msecond\u{1b}[0m".to_string(),
            "third".to_string(),
            "fourth".to_string(),
        ];

        // anchor and cursor in either order select the same inclusive range,
        // joined with newlines and stripped of ANSI escapes.
        assert_eq!(
            copy_payload_for_output(&snapshot, 1, 2),
            CopySelectionResult::Payload(CopyPayload::new("second\nthird", CopyLabel::Row)),
        );
        assert_eq!(
            copy_payload_for_output(&snapshot, 2, 1),
            CopySelectionResult::Payload(CopyPayload::new("second\nthird", CopyLabel::Row)),
        );
    }

    #[test]
    fn output_copy_drops_non_sgr_escape_sequences() {
        let snapshot = [
            "before \u{1b}[6nafter".to_string(),
            "start \u{1b}Pignored\u{1b}\\end".to_string(),
        ];

        assert_eq!(
            copy_payload_for_output(&snapshot, 0, 1),
            CopySelectionResult::Payload(CopyPayload::new(
                "before after\nstart end",
                CopyLabel::Row
            )),
        );
    }

    #[test]
    fn output_copy_clamps_out_of_range_indices() {
        let snapshot = ["only".to_string(), "two".to_string()];

        // A cursor past the end clamps to the last row rather than panicking.
        assert_eq!(
            copy_payload_for_output(&snapshot, 0, 99),
            CopySelectionResult::Payload(CopyPayload::new("only\ntwo", CopyLabel::Row)),
        );
    }

    #[test]
    fn output_copy_empty_snapshot_is_nothing() {
        assert_eq!(
            copy_payload_for_output(&[], 0, 0),
            CopySelectionResult::Nothing
        );
    }
}
