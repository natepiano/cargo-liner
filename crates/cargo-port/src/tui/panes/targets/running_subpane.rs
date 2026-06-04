//! The Running sub-pane at the bottom of the Targets pane: one row per
//! running instance across every tracked workspace, newest at the bottom,
//! with `cargo install`ed instances folded under a collapsible `cargo`
//! header row.
//!
//! `build_running_rows` flattens the poller snapshot into the render order;
//! `build_running_list` folds it into the navigable rows for the current
//! [`CargoGroup`] state; `render_running_subpane` draws the divider, the
//! column header, and the visible row slice into the box the layout tree
//! placed; `resolve_kill_request` maps the pane's highlight to the one
//! instance `K` terminates.

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::PaneFocusState;
use tui_pane::PaneTitleCount;
use tui_pane::Placed;
use tui_pane::RuleTitle;
use tui_pane::Viewport;
use tui_pane::column_header_color;
use tui_pane::label_color;
use tui_pane::success_color;
use tui_pane::text_default;
use unicode_width::UnicodeWidthStr;

use crate::project::DisplayPath;
use crate::tui::render;
use crate::tui::running_targets::RunProfile;
use crate::tui::running_targets::RunningTargets;

/// Cap on the Target column width so a single long target name can't
/// crowd out the metric columns. Overflow truncates with an ellipsis.
const TARGET_COL_MAX: usize = 24;
/// Header text for the Target column — also its minimum width.
const TARGET_HEADER: &str = "Target";
/// Width of the Profile column: the widest profile label (`release`).
const PROFILE_COL_WIDTH: usize = 7;
/// Width of the PID column: Linux PIDs reach seven digits.
const PID_COL_WIDTH: usize = 7;
/// Width of the CPU column: `476%` — a busy multi-threaded process can
/// exceed 100.
const CPU_COL_WIDTH: usize = 4;
/// Width of the MEM column: `999.9 MiB`.
const MEM_COL_WIDTH: usize = 9;

/// One running instance, flattened for the Running list: the target name,
/// how it was launched, its live metrics, and the member-relative path
/// that tells same-named targets apart.
pub struct RunningRow {
    pub name:         String,
    pub profile:      RunProfile,
    pub pid:          u32,
    pub cpu_percent:  f32,
    pub memory_bytes: u64,
    pub display_path: DisplayPath,
    pub first_seen:   Instant,
    pub create_time:  u64,
}

/// The one instance a kill request terminates: the confirm-dialog label
/// plus the PID and the create time it is verified against before
/// `SIGTERM`.
pub struct KillRequest {
    pub label:       String,
    pub pid:         u32,
    pub create_time: u64,
}

/// Expansion state of the Running list's `cargo` group. Installed
/// (`cargo`) instances are always running on a working setup — cargo-port
/// itself at minimum — so the group defaults to collapsed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CargoGroup {
    #[default]
    Collapsed,
    Expanded,
}

impl CargoGroup {
    /// The opposite state — `Enter` on the header row flips it.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Collapsed => Self::Expanded,
            Self::Expanded => Self::Collapsed,
        }
    }
}

/// One navigable row of the Running list: the collapsible `cargo` group
/// header, or an index into the flattened instance rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunningListRow {
    /// The `cargo` group header, carrying the count of installed-cargo
    /// instances folded under it.
    CargoHeader { count: usize },
    /// One running instance — the index into `build_running_rows`'s order.
    Instance(usize),
}

/// Flatten the poller snapshot into the Running instances: every tracked
/// instance across every workspace. Installed-`cargo` instances sort
/// first (contiguous, so [`build_running_list`] can fold them under the
/// header); the rest follow oldest-first so the newest instance is the
/// bottom row. An installed binary attributed to several projects is one
/// OS process — it gets one row, with the attribution chosen by the
/// lowest path so it doesn't flicker between ticks.
pub fn build_running_rows(running: &RunningTargets) -> Vec<RunningRow> {
    let mut rows: Vec<RunningRow> = running
        .iter_targets()
        .flat_map(|(key, member_dir, instances)| {
            instances.iter().map(|inst| RunningRow {
                name:         key.name.clone(),
                profile:      inst.profile,
                pid:          inst.pid,
                cpu_percent:  inst.cpu_percent,
                memory_bytes: inst.memory_bytes,
                display_path: member_dir.display_path(),
                first_seen:   inst.first_seen,
                create_time:  inst.create_time,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        a.pid
            .cmp(&b.pid)
            .then_with(|| a.display_path.as_str().cmp(b.display_path.as_str()))
    });
    rows.dedup_by(|next, kept| next.pid == kept.pid);
    rows.sort_by(|a, b| {
        b.profile
            .is_installed()
            .cmp(&a.profile.is_installed())
            .then_with(|| a.first_seen.cmp(&b.first_seen))
            .then_with(|| a.pid.cmp(&b.pid))
    });
    rows
}

/// Fold the instance rows into the Running list's navigable rows for the
/// current [`CargoGroup`] state: with any installed-`cargo` instances the
/// header row leads, hiding them while collapsed and preceding them while
/// expanded; the remaining instances are one row each. No header when
/// nothing installed runs.
pub fn build_running_list(rows: &[RunningRow], cargo_group: CargoGroup) -> Vec<RunningListRow> {
    let cargo_count = rows.iter().filter(|row| row.profile.is_installed()).count();
    if cargo_count == 0 {
        return (0..rows.len()).map(RunningListRow::Instance).collect();
    }
    let mut list = vec![RunningListRow::CargoHeader { count: cargo_count }];
    if matches!(cargo_group, CargoGroup::Expanded) {
        list.extend((0..cargo_count).map(RunningListRow::Instance));
    }
    list.extend((cargo_count..rows.len()).map(RunningListRow::Instance));
    list
}

/// Resolve the kill request for the pane's highlight: `Some` only when the
/// highlight sits on a Running instance row, carrying that one instance's
/// PID and create time. Table rows and the `cargo` group header have
/// nothing to kill.
pub fn resolve_kill_request(
    table_len: usize,
    running_rows: &[RunningRow],
    list: &[RunningListRow],
    selected: usize,
) -> Option<KillRequest> {
    let local = selected.checked_sub(table_len)?;
    let RunningListRow::Instance(index) = list.get(local)? else {
        return None;
    };
    let row = running_rows.get(*index)?;
    Some(KillRequest {
        label:       format!("{} ({})", row.name, row.profile.label()),
        pid:         row.pid,
        create_time: row.create_time,
    })
}

/// `started Ns/Nm/Nh/Nd ago` for the kill confirm body, from the
/// process's create time (seconds since the epoch) and the current epoch
/// second.
pub fn format_start_age(create_time: u64, now_epoch: u64) -> String {
    let secs = now_epoch.saturating_sub(create_time);
    let age = if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    };
    format!("started {age} ago")
}

/// Everything the Running sub-pane needs from the pane's render pass.
pub(super) struct RunningSubpaneRender<'a> {
    pub rows:         &'a [RunningRow],
    /// The navigable rows for this frame — what the box scrolls and the
    /// highlight walks.
    pub list:         &'a [RunningListRow],
    pub cargo_group:  CargoGroup,
    pub viewport:     &'a Viewport,
    pub focus:        PaneFocusState,
    /// Selectable rows before the Running box — its rows' logical indices
    /// start here.
    pub table_len:    usize,
    pub border_style: Style,
    pub title_style:  Style,
}

/// Column widths for the Running list, computed once per frame from the
/// rows and the box width. All fields are character widths.
struct RunningColumns {
    target: usize,
}

impl RunningColumns {
    fn new(rows: &[RunningRow]) -> Self {
        let widest_name = rows
            .iter()
            .map(|row| row.name.width())
            .max()
            .unwrap_or(0)
            .max(TARGET_HEADER.width())
            .min(TARGET_COL_MAX);
        Self {
            target: widest_name,
        }
    }

    /// One row's six columns formatted into styled spans. `None` styles
    /// render the header.
    fn line(&self, cells: &RunningCells<'_>) -> Line<'static> {
        let target = render::truncate_with_ellipsis(cells.target, self.target, "\u{2026}");
        let tw = self.target;
        Line::from(vec![
            Span::styled(format!(" {target:<tw$}"), cells.target_style),
            Span::styled(
                format!(" {:<PROFILE_COL_WIDTH$}", cells.profile),
                cells.profile_style,
            ),
            Span::styled(
                format!(" {:>PID_COL_WIDTH$}", cells.pid),
                cells.metric_style,
            ),
            Span::styled(
                format!(" {:>CPU_COL_WIDTH$}", cells.cpu),
                cells.metric_style,
            ),
            Span::styled(
                format!(" {:>MEM_COL_WIDTH$}", cells.mem),
                cells.metric_style,
            ),
            Span::styled(format!("  {}", cells.path), cells.path_style),
        ])
    }

    /// Columns left of Path, plus the leading pad and inter-column gaps —
    /// what's left of the box width is the Path column's budget.
    const fn path_budget(&self, box_width: u16) -> usize {
        let fixed = 1
            + self.target
            + 1
            + PROFILE_COL_WIDTH
            + 1
            + PID_COL_WIDTH
            + 1
            + CPU_COL_WIDTH
            + 1
            + MEM_COL_WIDTH
            + 2;
        (box_width as usize).saturating_sub(fixed)
    }
}

/// One row's (or the header's) cell texts and styles.
struct RunningCells<'a> {
    target:        &'a str,
    profile:       &'a str,
    pid:           String,
    cpu:           String,
    mem:           String,
    path:          String,
    target_style:  Style,
    profile_style: Style,
    metric_style:  Style,
    path_style:    Style,
}

/// Draw the Running box into its placed rect: the `├─ Running (N) ─┤`
/// divider across the pane (chrome row 0), the column header (chrome row
/// 1), and the visible row slice — newest at the bottom. Pushes a hit-test
/// rect per visible row.
pub(super) fn render_running_subpane(
    frame: &mut Frame,
    context: &RunningSubpaneRender<'_>,
    placed: Placed,
    pane_area: Rect,
    row_rects: &mut Vec<(Rect, usize)>,
) {
    render_divider(frame, context, placed, pane_area);
    let columns = RunningColumns::new(context.rows);
    render_header(frame, &columns, placed);
    render_rows(frame, context, &columns, placed, row_rects);
}

/// The divider rule, full pane width so its `├`/`┤` endcaps tee into the
/// side borders, titled with the instance count — or the highlighted
/// instance's position among all instances while the highlight sits on
/// one (the `cargo` header shows the plain count).
fn render_divider(
    frame: &mut Frame,
    context: &RunningSubpaneRender<'_>,
    placed: Placed,
    pane_area: Rect,
) {
    if placed.chrome.height == 0 {
        return;
    }
    let cursor = context
        .viewport
        .pos()
        .checked_sub(context.table_len)
        .and_then(|local| context.list.get(local))
        .and_then(|row| match row {
            RunningListRow::Instance(index) => Some(*index),
            RunningListRow::CargoHeader { .. } => None,
        });
    let title = format!(
        "Running {}",
        PaneTitleCount::Single {
            len: context.rows.len(),
            cursor,
        }
        .body()
    );
    tui_pane::render_horizontal_rule(
        frame,
        Rect {
            x:      pane_area.x,
            y:      placed.chrome.y,
            width:  pane_area.width,
            height: 1,
        },
        context.border_style,
        Some(RuleTitle {
            text:  &title,
            style: context.title_style,
        }),
        None,
    );
}

fn render_header(frame: &mut Frame, columns: &RunningColumns, placed: Placed) {
    if placed.chrome.height < 2 {
        return;
    }
    let header_style = Style::default().fg(column_header_color());
    let line = columns.line(&RunningCells {
        target:        TARGET_HEADER,
        profile:       "Profile",
        pid:           "PID".to_string(),
        cpu:           "CPU".to_string(),
        mem:           "MEM".to_string(),
        path:          "Path".to_string(),
        target_style:  header_style,
        profile_style: header_style,
        metric_style:  header_style,
        path_style:    header_style,
    });
    let header_area = Rect {
        y: placed.chrome.bottom().saturating_sub(1),
        height: 1,
        ..placed.chrome
    };
    frame.render_widget(Paragraph::new(line), header_area);
}

fn render_rows(
    frame: &mut Frame,
    context: &RunningSubpaneRender<'_>,
    columns: &RunningColumns,
    placed: Placed,
    row_rects: &mut Vec<(Rect, usize)>,
) {
    let visible = usize::from(placed.content.height);
    let end = placed
        .scroll_offset
        .saturating_add(visible)
        .min(context.list.len());
    for (slot, index) in (placed.scroll_offset..end).enumerate() {
        let logical_row = context.table_len + index;
        let area = Rect {
            x:      placed.content.x,
            y:      placed
                .content
                .y
                .saturating_add(u16::try_from(slot).unwrap_or(u16::MAX)),
            width:  placed.content.width,
            height: 1,
        };
        let line = match context.list[index] {
            RunningListRow::CargoHeader { count } => cargo_header_line(context.cargo_group, count),
            RunningListRow::Instance(row_index) => {
                instance_line(&context.rows[row_index], columns, area.width)
            },
        };
        let selection = tui_pane::selection_state(context.viewport, logical_row, context.focus);
        frame.render_widget(Paragraph::new(line).style(selection.overlay_style()), area);
        row_rects.push((area, logical_row));
    }
}

/// The `cargo` group header row: the expand/collapse glyph plus the count
/// of installed-cargo instances folded under it.
fn cargo_header_line(cargo_group: CargoGroup, count: usize) -> Line<'static> {
    let glyph = match cargo_group {
        CargoGroup::Collapsed => "\u{25b6}",
        CargoGroup::Expanded => "\u{25bc}",
    };
    Line::from(vec![
        Span::styled(format!(" {glyph} "), Style::default().fg(label_color())),
        Span::styled("cargo", Style::default().fg(success_color())),
        Span::styled(format!(" ({count})"), Style::default().fg(label_color())),
    ])
}

/// One running instance's columns, with the member path left-truncated to
/// the room the fixed columns leave.
fn instance_line(row: &RunningRow, columns: &RunningColumns, width: u16) -> Line<'static> {
    let path = left_truncate_with_ellipsis(
        row.display_path.as_str(),
        columns.path_budget(width),
        "\u{2026}",
    );
    columns.line(&RunningCells {
        target: &row.name,
        profile: row.profile.label(),
        pid: row.pid.to_string(),
        cpu: format!("{:.0}%", row.cpu_percent),
        mem: render::format_bytes(row.memory_bytes),
        path,
        target_style: Style::default().fg(text_default()),
        profile_style: Style::default().fg(success_color()),
        metric_style: Style::default().fg(label_color()),
        path_style: Style::default()
            .fg(label_color())
            .add_modifier(Modifier::DIM),
    })
}

/// Truncate `text` from the left so its tail fits `max_width` columns —
/// the rightmost path segment (the member name) stays visible.
fn left_truncate_with_ellipsis(text: &str, max_width: usize, ellipsis: &str) -> String {
    if text.width() <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    if max_width <= ellipsis.width() {
        return ellipsis.to_string();
    }
    let budget = max_width - ellipsis.width();
    let mut tail = String::new();
    for ch in text.chars().rev() {
        let next_width = tail.width() + ch.to_string().width();
        if next_width > budget {
            break;
        }
        tail.insert(0, ch);
    }
    format!("{ellipsis}{tail}")
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::project::AbsolutePath;
    use crate::tui::panes::RunTargetKind;
    use crate::tui::running_targets::RunningInstance;
    use crate::tui::running_targets::RunningKey;

    fn key(dir: &str, name: &str) -> RunningKey {
        RunningKey {
            target_dir: AbsolutePath::from(PathBuf::from(dir)),
            kind:       RunTargetKind::Binary,
            name:       name.into(),
        }
    }

    #[test]
    fn rows_flatten_every_workspace_newest_at_the_bottom() {
        // Two workspaces; instance PIDs double as first-seen order in
        // `for_test`, so pid 30 (seen last) must be the bottom row.
        let running = RunningTargets::from_pairs(vec![
            (
                key("/tmp/a/target", "app"),
                vec![
                    RunningInstance::for_test(30, RunProfile::Debug),
                    RunningInstance::for_test(10, RunProfile::Debug),
                ],
            ),
            (
                key("/tmp/b/target", "other"),
                vec![RunningInstance::for_test(20, RunProfile::Release)],
            ),
        ]);
        let rows = build_running_rows(&running);
        let pids: Vec<u32> = rows.iter().map(|row| row.pid).collect();
        assert_eq!(pids, vec![10, 20, 30]);
        assert_eq!(rows[1].name, "other");
    }

    #[test]
    fn one_process_attributed_to_several_projects_is_one_row() {
        // An installed binary lands under both the primary repo and its
        // worktree; the Running list shows the one OS process once.
        let running = RunningTargets::from_pairs(vec![
            (
                key("/tmp/main/target", "cargo-port"),
                vec![RunningInstance::for_test(7, RunProfile::Installed)],
            ),
            (
                key("/tmp/wt/target", "cargo-port"),
                vec![RunningInstance::for_test(7, RunProfile::Installed)],
            ),
        ]);
        let rows = build_running_rows(&running);
        assert_eq!(rows.len(), 1);
        // The lowest path wins so the attribution is stable across ticks.
        assert_eq!(rows[0].display_path.as_str(), "/tmp/main");
    }

    #[test]
    fn rows_carry_the_member_relative_path() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/ws/target", "app"),
            vec![RunningInstance::for_test(5, RunProfile::Debug)],
        )]);
        let rows = build_running_rows(&running);
        // `from_pairs` derives the member dir from the target dir's parent.
        assert_eq!(rows[0].display_path.as_str(), "/tmp/ws");
    }

    #[test]
    fn kill_resolves_only_on_a_running_row() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/ws/target", "app"),
            vec![RunningInstance::for_test(5, RunProfile::Debug)],
        )]);
        let rows = build_running_rows(&running);
        let list = build_running_list(&rows, CargoGroup::Collapsed);
        // Table rows (0..3) have nothing to kill.
        assert!(resolve_kill_request(3, &rows, &list, 0).is_none());
        assert!(resolve_kill_request(3, &rows, &list, 2).is_none());
        // The first Running row resolves to its single PID + create time.
        let request = resolve_kill_request(3, &rows, &list, 3).expect("running row");
        assert_eq!(request.pid, 5);
        assert_eq!(request.create_time, 5);
        assert_eq!(request.label, "app (debug)");
        // Past the list: nothing.
        assert!(resolve_kill_request(3, &rows, &list, 4).is_none());
    }

    /// Two installed (`cargo`) instances plus one debug instance: the
    /// fixture behind the group tests below.
    fn rows_with_cargo_group() -> Vec<RunningRow> {
        build_running_rows(&RunningTargets::from_pairs(vec![
            (
                key("/tmp/a/target", "app"),
                vec![RunningInstance::for_test(9, RunProfile::Debug)],
            ),
            (
                key("/tmp/b/target", "cargo-port"),
                vec![
                    RunningInstance::for_test(3, RunProfile::Installed),
                    RunningInstance::for_test(7, RunProfile::Installed),
                ],
            ),
        ]))
    }

    #[test]
    fn installed_instances_sort_before_the_rest() {
        let rows = rows_with_cargo_group();
        let pids: Vec<u32> = rows.iter().map(|row| row.pid).collect();
        // Installed first (oldest-first within), then the debug instance.
        assert_eq!(pids, vec![3, 7, 9]);
    }

    #[test]
    fn collapsed_list_folds_installed_instances_under_the_header() {
        let rows = rows_with_cargo_group();
        let list = build_running_list(&rows, CargoGroup::Collapsed);
        assert_eq!(
            list,
            vec![
                RunningListRow::CargoHeader { count: 2 },
                RunningListRow::Instance(2),
            ],
        );
    }

    #[test]
    fn expanded_list_shows_installed_instances_under_the_header() {
        let rows = rows_with_cargo_group();
        let list = build_running_list(&rows, CargoGroup::Expanded);
        assert_eq!(
            list,
            vec![
                RunningListRow::CargoHeader { count: 2 },
                RunningListRow::Instance(0),
                RunningListRow::Instance(1),
                RunningListRow::Instance(2),
            ],
        );
    }

    #[test]
    fn list_without_installed_instances_has_no_header() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/ws/target", "app"),
            vec![RunningInstance::for_test(5, RunProfile::Debug)],
        )]);
        let rows = build_running_rows(&running);
        let list = build_running_list(&rows, CargoGroup::Collapsed);
        assert_eq!(list, vec![RunningListRow::Instance(0)]);
    }

    #[test]
    fn kill_does_not_resolve_on_the_cargo_header() {
        let rows = rows_with_cargo_group();
        let list = build_running_list(&rows, CargoGroup::Expanded);
        // List row 0 is the header — nothing to kill.
        assert!(resolve_kill_request(3, &rows, &list, 3).is_none());
        // List row 1 is the oldest installed instance.
        let request = resolve_kill_request(3, &rows, &list, 4).expect("instance row");
        assert_eq!(request.pid, 3);
        assert_eq!(request.label, "cargo-port (cargo)");
    }

    #[test]
    fn start_age_scales_units() {
        assert_eq!(format_start_age(100, 130), "started 30s ago");
        assert_eq!(format_start_age(100, 220), "started 2m ago");
        assert_eq!(format_start_age(100, 7300), "started 2h ago");
        assert_eq!(format_start_age(100, 200_000), "started 2d ago");
    }

    #[test]
    fn left_truncation_keeps_the_member_segment() {
        assert_eq!(
            left_truncate_with_ellipsis("~/rust/bevy_window_manager/foo", 10, "\u{2026}"),
            "\u{2026}nager/foo"
        );
        assert_eq!(
            left_truncate_with_ellipsis("~/short", 10, "\u{2026}"),
            "~/short"
        );
    }
}
