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
