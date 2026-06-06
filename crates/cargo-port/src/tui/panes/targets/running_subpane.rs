//! The Running sub-pane at the bottom of the Targets pane: one row per
//! running instance across every tracked workspace — newest at the bottom,
//! `cargo install`ed instances folded under a collapsible `cargo` header
//! row — plus, nested under each instance, every process it spawned (the
//! `cargo` / `rustc` chains of a `cargo mend` run), as a collapsible
//! outline.
//!
//! `build_running_rows` flattens the poller snapshot into the render order;
//! `build_running_list` folds it into the navigable rows for the current
//! [`CargoGroup`] and outline state; `render_running_subpane` draws the
//! divider, the column header, and the visible row slice into the box the
//! layout tree placed; `resolve_kill_request` maps the pane's highlight to
//! the one process `K` terminates.

use std::collections::HashMap;
use std::collections::HashSet;
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
use tui_pane::label_color;
use tui_pane::success_color;
use tui_pane::text_default;
use unicode_width::UnicodeWidthStr;

use crate::constants::CARGO_COMMAND_NAME;
use crate::project::DisplayPath;
use crate::tui::render;
use crate::tui::running_targets::RunProfile;
use crate::tui::running_targets::RunningTargets;
use crate::tui::theme_roles;

/// Cap on the Target column width so a single long target name can't
/// crowd out the metric columns. Overflow truncates with an ellipsis.
const TARGET_COL_MAX: usize = 24;
/// Header text for the Target column — also its minimum width.
const TARGET_HEADER: &str = "Target";
/// Width consumed by one outline depth: two leading spaces.
const OUTLINE_DEPTH_INDENT_WIDTH: usize = 2;
/// Width consumed by an outline glyph plus its following gap.
const OUTLINE_PARENT_PREFIX_WIDTH: usize = 2;
/// Width consumed by a one-digit child-count suffix, such as ` (9)`.
const OUTLINE_SINGLE_DIGIT_SUFFIX_WIDTH: usize = 4;
/// Width of the Profile column: the widest profile label (`release`).
const PROFILE_COL_WIDTH: usize = 7;
/// Width of the PID column: Linux PIDs reach seven digits.
const PID_COL_WIDTH: usize = 7;
/// Width of the CPU column: `476%` — a busy multi-threaded process can
/// exceed 100.
const CPU_COL_WIDTH: usize = 4;
/// Width of the MEM column: `999.9 MiB`.
const MEM_COL_WIDTH: usize = 9;

/// One process in the Running list: a tracked target instance or an
/// untracked process one spawned. `parent_pid`/`depth` place the row in
/// the sub-process outline — children render directly under their parent,
/// indented.
pub struct RunningRow {
    pub name:         String,
    pub pid:          u32,
    pub cpu_percent:  f32,
    pub memory_bytes: u64,
    pub first_seen:   Instant,
    pub create_time:  u64,
    pub parent_pid:   Option<u32>,
    pub depth:        usize,
    pub kind:         RunningRowKind,
}

/// What a Running row is: a tracked target instance — with its launch
/// profile and the member-relative path that tells same-named targets
/// apart — or an untracked child process, whose Profile and Path cells
/// stay blank.
pub enum RunningRowKind {
    Target {
        profile:      RunProfile,
        display_path: DisplayPath,
    },
    Child,
}

impl RunningRow {
    /// The Profile cell: the launch profile for a target instance, blank
    /// for a child process.
    const fn profile_label(&self) -> &'static str {
        match &self.kind {
            RunningRowKind::Target { profile, .. } => profile.label(),
            RunningRowKind::Child => "",
        }
    }

    /// The Path cell: the member-relative path for a target instance,
    /// blank for a child process.
    fn path_str(&self) -> &str {
        match &self.kind {
            RunningRowKind::Target { display_path, .. } => display_path.as_str(),
            RunningRowKind::Child => "",
        }
    }

    /// Whether this row is an installed (`cargo`-profile) target
    /// instance — the roots of the Running list's `cargo` group.
    const fn is_installed_target(&self) -> bool {
        matches!(&self.kind, RunningRowKind::Target { profile, .. } if profile.is_installed())
    }
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

/// Flatten the poller snapshot into the Running rows: every tracked
/// instance across every workspace, plus every untracked process one
/// spawned. Installed-`cargo` instances sort first (contiguous, so
/// [`build_running_list`] can fold their subtrees under the header); the
/// rest follow oldest-first so the newest instance is the bottom row. An
/// installed binary attributed to several projects is one OS process — it
/// gets one row, with the attribution chosen by the lowest path so it
/// doesn't flicker between ticks. Child processes append after the sort
/// (stably, by PID) and nest under their parents in the tree pass.
pub fn build_running_rows(running: &RunningTargets) -> Vec<RunningRow> {
    let mut rows: Vec<RunningRow> = running
        .iter_targets()
        .flat_map(|(key, member_dir, instances)| {
            instances.iter().map(|inst| RunningRow {
                name:         key.name.clone(),
                pid:          inst.pid,
                cpu_percent:  inst.cpu_percent,
                memory_bytes: inst.memory_bytes,
                first_seen:   inst.first_seen,
                create_time:  inst.create_time,
                parent_pid:   inst.parent_pid,
                depth:        0,
                kind:         RunningRowKind::Target {
                    profile:      inst.profile,
                    display_path: member_dir.display_path(),
                },
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        a.pid
            .cmp(&b.pid)
            .then_with(|| a.path_str().cmp(b.path_str()))
    });
    rows.dedup_by(|next, kept| next.pid == kept.pid);
    rows.sort_by(|a, b| {
        b.is_installed_target()
            .cmp(&a.is_installed_target())
            .then_with(|| a.first_seen.cmp(&b.first_seen))
            .then_with(|| a.pid.cmp(&b.pid))
    });
    let mut children: Vec<RunningRow> = running
        .child_processes()
        .iter()
        .map(|child| RunningRow {
            name:         child.name.clone(),
            pid:          child.pid,
            cpu_percent:  child.cpu_percent,
            memory_bytes: child.memory_bytes,
            first_seen:   child.first_seen,
            create_time:  child.create_time,
            parent_pid:   Some(child.parent_pid),
            depth:        0,
            kind:         RunningRowKind::Child,
        })
        .collect();
    children.sort_by_key(|row| row.pid);
    rows.extend(children);
    tree_ordered(rows)
}

/// Reorder the sorted rows into outline order: each row whose parent is in
/// the list moves directly under that parent (depth-first) with `depth`
/// set to its nesting level; top-level rows keep the incoming order. A row
/// whose parent is absent from the list stays top-level. Children share
/// their parent's profile (the poller resolves parents within one
/// profile), so the installed-first prefix stays contiguous.
fn tree_ordered(rows: Vec<RunningRow>) -> Vec<RunningRow> {
    let pids: HashSet<u32> = rows.iter().map(|row| row.pid).collect();
    let capacity = rows.len();
    let mut children: HashMap<u32, Vec<RunningRow>> = HashMap::new();
    let mut top_level: Vec<RunningRow> = Vec::new();
    for row in rows {
        match row
            .parent_pid
            .filter(|parent| *parent != row.pid && pids.contains(parent))
        {
            Some(parent) => children.entry(parent).or_default().push(row),
            None => top_level.push(row),
        }
    }
    let mut ordered = Vec::with_capacity(capacity);
    for row in top_level {
        append_subtree(row, 0, &mut children, &mut ordered);
    }
    // Parent links form a forest (the poller's walk validates start-time
    // ordering), so nothing is left stranded; drain defensively anyway.
    debug_assert!(children.is_empty(), "parent links form a forest");
    for orphans in children.into_values() {
        ordered.extend(orphans);
    }
    ordered
}

/// Emit `row` at `depth`, then its children (depth-first) directly below.
fn append_subtree(
    row: RunningRow,
    depth: usize,
    children: &mut HashMap<u32, Vec<RunningRow>>,
    ordered: &mut Vec<RunningRow>,
) {
    let pid = row.pid;
    ordered.push(RunningRow { depth, ..row });
    for child in children.remove(&pid).unwrap_or_default() {
        append_subtree(child, depth + 1, children, ordered);
    }
}

/// Fold the instance rows into the Running list's navigable rows for the
/// current [`CargoGroup`] and outline state: with any installed-`cargo`
/// instances the header row leads, hiding their subtrees while collapsed
/// and preceding them while expanded; the remaining rows are one each,
/// minus the subtrees of collapsed outline parents. No header when
/// nothing installed runs.
pub fn build_running_list(
    rows: &[RunningRow],
    cargo_group: CargoGroup,
    expanded_parents: &HashSet<u32>,
) -> Vec<RunningListRow> {
    let visible = visible_indices(rows, expanded_parents);
    let cargo_count = cargo_segment_len(rows);
    if cargo_count == 0 {
        return visible.into_iter().map(RunningListRow::Instance).collect();
    }
    let mut list = vec![RunningListRow::CargoHeader { count: cargo_count }];
    if matches!(cargo_group, CargoGroup::Expanded) {
        list.extend(
            visible
                .iter()
                .copied()
                .filter(|index| *index < cargo_count)
                .map(RunningListRow::Instance),
        );
    }
    list.extend(
        visible
            .into_iter()
            .filter(|index| *index >= cargo_count)
            .map(RunningListRow::Instance),
    );
    list
}

/// Rows in the `cargo` group: the prefix of subtrees rooted at installed
/// instances. The top-level sort puts installed roots first and the tree
/// pass keeps each subtree contiguous, so the segment ends at the first
/// top-level row that is not an installed instance — everything spawned
/// by an installed instance folds with it.
fn cargo_segment_len(rows: &[RunningRow]) -> usize {
    rows.iter()
        .take_while(|row| row.depth > 0 || row.is_installed_target())
        .count()
}

/// Row indices visible under the outline state: a collapsed parent hides
/// its whole subtree — the contiguous run of deeper rows below it.
fn visible_indices(rows: &[RunningRow], expanded_parents: &HashSet<u32>) -> Vec<usize> {
    let mut visible = Vec::with_capacity(rows.len());
    // Depth of the nearest collapsed ancestor; rows deeper than it are
    // inside its subtree and hidden.
    let mut collapsed_depth: Option<usize> = None;
    for (index, row) in rows.iter().enumerate() {
        if let Some(depth) = collapsed_depth {
            if row.depth > depth {
                continue;
            }
            collapsed_depth = None;
        }
        visible.push(index);
        if outline_subtree_len(rows, index) > 0 && !expanded_parents.contains(&row.pid) {
            collapsed_depth = Some(row.depth);
        }
    }
    visible
}

/// Direct and indirect children of the row at `index`: the contiguous run
/// of deeper rows that follows it in tree order. Zero for a leaf.
pub fn outline_subtree_len(rows: &[RunningRow], index: usize) -> usize {
    let Some(row) = rows.get(index) else {
        return 0;
    };
    rows[index + 1..]
        .iter()
        .take_while(|next| next.depth > row.depth)
        .count()
}

/// Resolve the kill request for the pane's highlight: `Some` only when the
/// highlight sits on a Running instance row, carrying that one process's
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
    let label = match &row.kind {
        RunningRowKind::Target { profile, .. } => format!("{} ({})", row.name, profile.label()),
        RunningRowKind::Child => format!("{} (process)", row.name),
    };
    Some(KillRequest {
        label,
        pid: row.pid,
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
    pub rows:             &'a [RunningRow],
    /// The navigable rows for this frame — what the box scrolls and the
    /// highlight walks.
    pub list:             &'a [RunningListRow],
    pub cargo_group:      CargoGroup,
    /// Outline parents the user has expanded; collapsed parents show
    /// their subtree's aggregate metrics.
    pub expanded_parents: &'a HashSet<u32>,
    pub viewport:         &'a Viewport,
    pub focus:            PaneFocusState,
    /// Selectable rows before the Running box — its rows' logical indices
    /// start here.
    pub table_len:        usize,
    pub border_style:     Style,
    pub title_style:      Style,
}

/// Column widths for the Running list, computed once per frame from the
/// rows and the box width. All fields are character widths.
struct RunningColumns {
    target: usize,
}

impl RunningColumns {
    fn new(rows: &[RunningRow], expanded_parents: &HashSet<u32>) -> Self {
        let widest_name = (0..rows.len())
            .map(|index| outline_name_width(rows, index, expanded_parents))
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
    let columns = RunningColumns::new(context.rows, context.expanded_parents);
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
    let header_style = Style::default().fg(theme_roles::column_header_color());
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
            RunningListRow::Instance(row_index) => instance_line(
                context.rows,
                row_index,
                context.expanded_parents,
                columns,
                area.width,
            ),
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
        Span::styled(CARGO_COMMAND_NAME, Style::default().fg(success_color())),
        Span::styled(format!(" ({count})"), Style::default().fg(label_color())),
    ])
}

/// The Target cell text: the bare name for a top-level row, depth-indented
/// with a `└` marker for a nested one.
fn indented_name(name: &str, depth: usize) -> String {
    if depth == 0 {
        return name.to_string();
    }
    format!("{}\u{2514} {name}", "  ".repeat(depth - 1))
}

/// The Target cell for the row at `index`: leaves indent with the `└`
/// marker; outline parents carry the expand/collapse glyph and their
/// subtree's row count, the `▶ cargo (N)` idiom.
fn outline_name(rows: &[RunningRow], index: usize, expanded_parents: &HashSet<u32>) -> String {
    let Some(row) = rows.get(index) else {
        return String::new();
    };
    let children = outline_subtree_len(rows, index);
    if children == 0 {
        return indented_name(&row.name, row.depth);
    }
    let glyph = if expanded_parents.contains(&row.pid) {
        "\u{25bc}"
    } else {
        "\u{25b6}"
    };
    format!(
        "{}{glyph} {} ({children})",
        "  ".repeat(row.depth),
        row.name
    )
}

/// The width to reserve for row `index`'s Target cell. A leaf may gain a
/// short-lived child between polls, so reserve the one-digit outline form
/// even before the child exists. Two-digit counts can still grow the
/// column when they occur.
fn outline_name_width(rows: &[RunningRow], index: usize, expanded_parents: &HashSet<u32>) -> usize {
    let Some(row) = rows.get(index) else {
        return 0;
    };
    outline_name(rows, index, expanded_parents)
        .width()
        .max(single_digit_outline_width(row))
}

fn single_digit_outline_width(row: &RunningRow) -> usize {
    row.depth
        .saturating_mul(OUTLINE_DEPTH_INDENT_WIDTH)
        .saturating_add(OUTLINE_PARENT_PREFIX_WIDTH)
        .saturating_add(row.name.width())
        .saturating_add(OUTLINE_SINGLE_DIGIT_SUFFIX_WIDTH)
}

/// The metrics the row at `index` displays: its own while expanded or a
/// leaf; the subtree aggregate while collapsed, so hidden children's load
/// still shows on their parent.
fn displayed_metrics(
    rows: &[RunningRow],
    index: usize,
    expanded_parents: &HashSet<u32>,
) -> (f32, u64) {
    let Some(row) = rows.get(index) else {
        return (0.0, 0);
    };
    let children = outline_subtree_len(rows, index);
    if children == 0 || expanded_parents.contains(&row.pid) {
        return (row.cpu_percent, row.memory_bytes);
    }
    let subtree = &rows[index..=index + children];
    (
        subtree.iter().map(|node| node.cpu_percent).sum(),
        subtree.iter().map(|node| node.memory_bytes).sum(),
    )
}

/// One running instance's columns, with the member path left-truncated to
/// the room the fixed columns leave.
fn instance_line(
    rows: &[RunningRow],
    index: usize,
    expanded_parents: &HashSet<u32>,
    columns: &RunningColumns,
    width: u16,
) -> Line<'static> {
    let Some(row) = rows.get(index) else {
        return Line::default();
    };
    let path = left_truncate_with_ellipsis(row.path_str(), columns.path_budget(width), "\u{2026}");
    let target = outline_name(rows, index, expanded_parents);
    let (cpu_percent, memory_bytes) = displayed_metrics(rows, index, expanded_parents);
    columns.line(&RunningCells {
        target: &target,
        profile: row.profile_label(),
        pid: row.pid.to_string(),
        cpu: format!("{cpu_percent:.0}%"),
        mem: render::format_bytes(memory_bytes),
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
    use crate::tui::running_targets::ChildProcess;
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
        assert_eq!(rows[0].path_str(), "/tmp/main");
    }

    #[test]
    fn rows_carry_the_member_relative_path() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/ws/target", "app"),
            vec![RunningInstance::for_test(5, RunProfile::Debug)],
        )]);
        let rows = build_running_rows(&running);
        // `from_pairs` derives the member dir from the target dir's parent.
        assert_eq!(rows[0].path_str(), "/tmp/ws");
    }

    #[test]
    fn kill_resolves_only_on_a_running_row() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/ws/target", "app"),
            vec![RunningInstance::for_test(5, RunProfile::Debug)],
        )]);
        let rows = build_running_rows(&running);
        let list = build_running_list(&rows, CargoGroup::Collapsed, &HashSet::new());
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
        let list = build_running_list(&rows, CargoGroup::Collapsed, &HashSet::new());
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
        let list = build_running_list(&rows, CargoGroup::Expanded, &HashSet::new());
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
        let list = build_running_list(&rows, CargoGroup::Collapsed, &HashSet::new());
        assert_eq!(list, vec![RunningListRow::Instance(0)]);
    }

    #[test]
    fn kill_does_not_resolve_on_the_cargo_header() {
        let rows = rows_with_cargo_group();
        let list = build_running_list(&rows, CargoGroup::Expanded, &HashSet::new());
        // List row 0 is the header — nothing to kill.
        assert!(resolve_kill_request(3, &rows, &list, 3).is_none());
        // List row 1 is the oldest installed instance.
        let request = resolve_kill_request(3, &rows, &list, 4).expect("instance row");
        assert_eq!(request.pid, 3);
        assert_eq!(request.label, "cargo-port (cargo)");
    }

    #[test]
    fn children_nest_directly_under_their_parent() {
        // first_seen order is 10, 20, 30 (PID doubles as the order in
        // `for_test`); 30 is a child of 10, so it leaves the bottom and
        // nests under its parent.
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/a/target", "mend"),
            vec![
                RunningInstance::for_test(10, RunProfile::Debug),
                RunningInstance::for_test(20, RunProfile::Debug),
                RunningInstance::for_test(30, RunProfile::Debug).with_parent(10),
            ],
        )]);
        let rows = build_running_rows(&running);
        let outline: Vec<(u32, usize)> = rows.iter().map(|row| (row.pid, row.depth)).collect();
        assert_eq!(outline, vec![(10, 0), (30, 1), (20, 0)]);
    }

    #[test]
    fn grandchildren_nest_two_deep() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/a/target", "mend"),
            vec![
                RunningInstance::for_test(10, RunProfile::Debug),
                RunningInstance::for_test(20, RunProfile::Debug).with_parent(10),
                RunningInstance::for_test(30, RunProfile::Debug).with_parent(20),
            ],
        )]);
        let rows = build_running_rows(&running);
        let outline: Vec<(u32, usize)> = rows.iter().map(|row| (row.pid, row.depth)).collect();
        assert_eq!(outline, vec![(10, 0), (20, 1), (30, 2)]);
    }

    #[test]
    fn a_row_whose_parent_is_absent_stays_top_level() {
        // Parent 99 exited (or was never tracked): the child renders as a
        // top-level row in its normal position.
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/a/target", "app"),
            vec![
                RunningInstance::for_test(10, RunProfile::Debug),
                RunningInstance::for_test(20, RunProfile::Debug).with_parent(99),
            ],
        )]);
        let rows = build_running_rows(&running);
        let outline: Vec<(u32, usize)> = rows.iter().map(|row| (row.pid, row.depth)).collect();
        assert_eq!(outline, vec![(10, 0), (20, 0)]);
    }

    #[test]
    fn nested_names_indent_with_a_marker() {
        assert_eq!(indented_name("mend", 0), "mend");
        assert_eq!(indented_name("mend", 1), "\u{2514} mend");
        assert_eq!(indented_name("mend", 2), "  \u{2514} mend");
    }

    /// One orchestrator (10) with two wrapper children (20, 30) plus an
    /// unrelated debug instance (40): the fixture behind the outline
    /// tests below.
    fn rows_with_outline() -> Vec<RunningRow> {
        build_running_rows(&RunningTargets::from_pairs(vec![
            (
                key("/tmp/a/target", "mend"),
                vec![
                    RunningInstance::for_test(10, RunProfile::Debug).with_metrics(2.0, 100),
                    RunningInstance::for_test(20, RunProfile::Debug)
                        .with_parent(10)
                        .with_metrics(30.0, 800),
                    RunningInstance::for_test(30, RunProfile::Debug)
                        .with_parent(10)
                        .with_metrics(28.0, 700),
                ],
            ),
            (
                key("/tmp/b/target", "app"),
                vec![RunningInstance::for_test(40, RunProfile::Debug)],
            ),
        ]))
    }

    #[test]
    fn collapsed_parent_hides_its_subtree() {
        let rows = rows_with_outline();
        let list = build_running_list(&rows, CargoGroup::Collapsed, &HashSet::new());
        // Rows are [10, 20, 30, 40] in tree order; 20 and 30 hide under
        // their collapsed parent.
        assert_eq!(
            list,
            vec![RunningListRow::Instance(0), RunningListRow::Instance(3)],
        );
    }

    #[test]
    fn expanded_parent_shows_its_subtree() {
        let rows = rows_with_outline();
        let list = build_running_list(&rows, CargoGroup::Collapsed, &HashSet::from([10]));
        assert_eq!(
            list,
            vec![
                RunningListRow::Instance(0),
                RunningListRow::Instance(1),
                RunningListRow::Instance(2),
                RunningListRow::Instance(3),
            ],
        );
    }

    #[test]
    fn outline_subtree_len_counts_the_contiguous_deeper_run() {
        let rows = rows_with_outline();
        assert_eq!(outline_subtree_len(&rows, 0), 2);
        assert_eq!(outline_subtree_len(&rows, 1), 0);
        assert_eq!(outline_subtree_len(&rows, 3), 0);
    }

    #[test]
    fn collapsed_parent_aggregates_its_subtree_metrics() {
        let rows = rows_with_outline();
        let (cpu, mem) = displayed_metrics(&rows, 0, &HashSet::new());
        assert!((cpu - 60.0).abs() < f32::EPSILON);
        assert_eq!(mem, 1600);
    }

    #[test]
    fn expanded_parent_shows_its_own_metrics() {
        let rows = rows_with_outline();
        let (cpu, mem) = displayed_metrics(&rows, 0, &HashSet::from([10]));
        assert!((cpu - 2.0).abs() < f32::EPSILON);
        assert_eq!(mem, 100);
    }

    #[test]
    fn parent_rows_carry_the_outline_glyph_and_count() {
        let rows = rows_with_outline();
        assert_eq!(outline_name(&rows, 0, &HashSet::new()), "\u{25b6} mend (2)");
        assert_eq!(
            outline_name(&rows, 0, &HashSet::from([10])),
            "\u{25bc} mend (2)"
        );
        // Leaves keep the plain / indented name.
        assert_eq!(outline_name(&rows, 1, &HashSet::new()), "\u{2514} mend");
        assert_eq!(outline_name(&rows, 3, &HashSet::new()), "app");
    }

    fn line_text(line: &Line<'_>) -> String {
        let mut text = String::new();
        for span in &line.spans {
            text.push_str(span.content.as_ref());
        }
        text
    }

    fn display_column(text: &str, needle: &str) -> Option<usize> {
        text.find(needle).map(|index| text[..index].width())
    }

    #[test]
    fn target_columns_do_not_move_for_one_digit_child_counts() {
        let without_child = build_running_rows(&RunningTargets::from_pairs(vec![(
            key("/tmp/a/target", "cargo-port"),
            vec![RunningInstance::for_test(10, RunProfile::Debug)],
        )]));
        let with_child = build_running_rows(
            &RunningTargets::from_pairs(vec![(
                key("/tmp/a/target", "cargo-port"),
                vec![RunningInstance::for_test(10, RunProfile::Debug)],
            )])
            .with_children(vec![ChildProcess::for_test(20, "cargo", 10)]),
        );

        let without_columns = RunningColumns::new(&without_child, &HashSet::new());
        let with_columns = RunningColumns::new(&with_child, &HashSet::new());
        let without_text = line_text(&instance_line(
            &without_child,
            0,
            &HashSet::new(),
            &without_columns,
            100,
        ));
        let with_text = line_text(&instance_line(
            &with_child,
            0,
            &HashSet::new(),
            &with_columns,
            100,
        ));

        assert_eq!(
            display_column(&without_text, "debug"),
            display_column(&with_text, "debug"),
            "Profile column should stay fixed when one child appears",
        );
        assert_eq!(
            display_column(&without_text, "10"),
            display_column(&with_text, "10"),
            "PID column should stay fixed when one child appears",
        );
    }

    #[test]
    fn child_processes_nest_under_their_tracked_parent() {
        // mend (10) spawned cargo (20), which spawned rustc (30): the
        // whole chain renders nested, with blank Profile/Path cells on
        // the untracked rows.
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/a/target", "mend"),
            vec![RunningInstance::for_test(10, RunProfile::Debug)],
        )])
        .with_children(vec![
            ChildProcess::for_test(20, "cargo", 10),
            ChildProcess::for_test(30, "rustc", 20),
        ]);
        let rows = build_running_rows(&running);
        let outline: Vec<(u32, usize)> = rows.iter().map(|row| (row.pid, row.depth)).collect();
        assert_eq!(outline, vec![(10, 0), (20, 1), (30, 2)]);
        assert_eq!(rows[1].name, "cargo");
        assert_eq!(rows[1].profile_label(), "");
        assert_eq!(rows[1].path_str(), "");
    }

    #[test]
    fn cargo_segment_folds_descendants_of_installed_roots() {
        let running = RunningTargets::from_pairs(vec![
            (
                key("/tmp/b/target", "cargo-port"),
                vec![RunningInstance::for_test(3, RunProfile::Installed)],
            ),
            (
                key("/tmp/a/target", "app"),
                vec![RunningInstance::for_test(5, RunProfile::Debug)],
            ),
        ])
        .with_children(vec![ChildProcess::for_test(9, "cargo", 3)]);
        let rows = build_running_rows(&running);
        // Tree order: the installed root and its child, then the debug app.
        let outline: Vec<(u32, usize)> = rows.iter().map(|row| (row.pid, row.depth)).collect();
        assert_eq!(outline, vec![(3, 0), (9, 1), (5, 0)]);
        // The header folds the root's whole subtree (count 2); with the
        // group expanded, the default-collapsed outline still hides the
        // child row.
        let list = build_running_list(&rows, CargoGroup::Expanded, &HashSet::new());
        assert_eq!(
            list,
            vec![
                RunningListRow::CargoHeader { count: 2 },
                RunningListRow::Instance(0),
                RunningListRow::Instance(2),
            ],
        );
    }

    #[test]
    fn kill_label_names_a_child_process() {
        let running = RunningTargets::from_pairs(vec![(
            key("/tmp/a/target", "mend"),
            vec![RunningInstance::for_test(10, RunProfile::Debug)],
        )])
        .with_children(vec![ChildProcess::for_test(20, "rustc", 10)]);
        let rows = build_running_rows(&running);
        // Expand the parent so the child row is navigable (list index 1).
        let list = build_running_list(&rows, CargoGroup::Collapsed, &HashSet::from([10]));
        let request = resolve_kill_request(0, &rows, &list, 1).expect("child row");
        assert_eq!(request.pid, 20);
        assert_eq!(request.label, "rustc (process)");
    }

    #[test]
    fn collapsed_outline_inside_the_expanded_cargo_group() {
        // An installed orchestrator + wrapper inside the cargo group: the
        // header still counts both instances, while the collapsed outline
        // hides the wrapper row.
        let rows = build_running_rows(&RunningTargets::from_pairs(vec![(
            key("/tmp/b/target", "cargo-mend"),
            vec![
                RunningInstance::for_test(3, RunProfile::Installed),
                RunningInstance::for_test(7, RunProfile::Installed).with_parent(3),
            ],
        )]));
        let collapsed = build_running_list(&rows, CargoGroup::Expanded, &HashSet::new());
        assert_eq!(
            collapsed,
            vec![
                RunningListRow::CargoHeader { count: 2 },
                RunningListRow::Instance(0),
            ],
        );
        let expanded = build_running_list(&rows, CargoGroup::Expanded, &HashSet::from([3]));
        assert_eq!(
            expanded,
            vec![
                RunningListRow::CargoHeader { count: 2 },
                RunningListRow::Instance(0),
                RunningListRow::Instance(1),
            ],
        );
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
