//! `ProjectList` pane render bodies.
//!
//! `render_project_list` and its tree-row helpers live alongside
//! `ProjectListPane`. The renderer is a free function (no `Pane`
//! trait impl).

use std::collections::HashMap;
use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use tui_pane::PaneTitleCount;
use tui_pane::PaneTitleGroup;
use tui_pane::Viewport;
use tui_pane::column_header_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;
use tui_pane::text_default;

use super::constants::PREFIX_GROUP_COLLAPSED;
use super::constants::PREFIX_GROUP_EXPANDED;
use super::constants::PREFIX_MEMBER_INLINE;
use super::constants::PREFIX_MEMBER_NAMED;
use super::constants::PREFIX_MEMBER_VENDORED_INLINE;
use super::constants::PREFIX_MEMBER_VENDORED_NAMED;
use super::constants::PREFIX_ROOT_COLLAPSED;
use super::constants::PREFIX_ROOT_EXPANDED;
use super::constants::PREFIX_ROOT_LEAF;
use super::constants::PREFIX_SUBMODULE;
use super::constants::PREFIX_VENDORED;
use super::constants::PREFIX_WT_COLLAPSED;
use super::constants::PREFIX_WT_EXPANDED;
use super::constants::PREFIX_WT_FLAT;
use super::constants::PREFIX_WT_GROUP_COLLAPSED;
use super::constants::PREFIX_WT_GROUP_EXPANDED;
use super::constants::PREFIX_WT_MEMBER_INLINE;
use super::constants::PREFIX_WT_MEMBER_NAMED;
use super::constants::PREFIX_WT_MEMBER_VENDORED_INLINE;
use super::constants::PREFIX_WT_MEMBER_VENDORED_NAMED;
use super::constants::PREFIX_WT_VENDORED;
use super::lang;
use super::pane_impls::ProjectListPane;
use crate::project;
use crate::project::GitStatus;
use crate::project::MemberGroup;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::WorktreeHealth;
use crate::project::WorktreeHealth::Normal;
use crate::scan;
use crate::tui::app;
use crate::tui::app::DiscoveryRowKind;
use crate::tui::app::ExpandKey;
use crate::tui::app::ProjectListWidths;
use crate::tui::app::VisibleRow;
use crate::tui::columns;
use crate::tui::columns::LintCell;
use crate::tui::columns::ProjectRow;
use crate::tui::pane::DismissTarget;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::project_list::ProjectList;
use crate::tui::render;
use crate::tui::state;
use crate::tui::state::Lint;

const TITLE_ELLIPSIS: &str = "\u{2026}";

/// Compute the percentile rank of `bytes` within `sorted_values` (0.0 to 1.0).
#[allow(
    clippy::cast_precision_loss,
    reason = "display-only — index-to-float ratio for color interpolation"
)]
fn disk_percentile(bytes: Option<u64>, sorted_values: &[u64]) -> Option<f64> {
    let bytes = bytes?;
    if sorted_values.len() <= 1 {
        return None;
    }
    let rank = sorted_values
        .iter()
        .position(|&v| v >= bytes)
        .unwrap_or(sorted_values.len() - 1);
    Some(rank as f64 / (sorted_values.len() - 1) as f64)
}

/// Compute a color for a disk value by interpolating the active
/// theme's three `disk_usage` stops: low (smallest) → mid → high
/// (largest). Modifiers on the theme stops are ignored.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to 0.0..=255.0 before cast"
)]
fn disk_color(percentile: Option<f64>) -> Style {
    let Some(pos) = percentile else {
        return Style::default().fg(label_color());
    };

    let theme = tui_pane::theme();
    let stops = &theme.disk_usage;
    let (start, end, t) = if pos < 0.5 {
        (stops.low.color, stops.mid.color, pos * 2.0)
    } else {
        (stops.mid.color, stops.high.color, (pos - 0.5) * 2.0)
    };
    let (sr, sg, sb) = rgb_channels(start);
    let (er, eg, eb) = rgb_channels(end);
    let lerp = |a: u8, b: u8| -> u8 {
        let af = f64::from(a);
        let bf = f64::from(b);
        (bf - af).mul_add(t, af).clamp(0.0, 255.0) as u8
    };

    Style::default().fg(Color::Rgb(lerp(sr, er), lerp(sg, eg), lerp(sb, eb)))
}

/// Extract RGB channels from a [`Color`], converting named/indexed
/// colors into their nearest ANSI 24-bit equivalents. Used by
/// [`disk_color`] so a theme can supply a named color (e.g. `Green`)
/// as a gradient stop and still interpolate smoothly.
const fn rgb_channels(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset | Color::Black => (0, 0, 0),
        Color::Red => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::Yellow => (170, 85, 0),
        Color::Blue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::Gray | Color::Indexed(_) => (170, 170, 170),
        Color::DarkGray => (85, 85, 85),
        Color::LightRed => (255, 85, 85),
        Color::LightGreen => (85, 255, 85),
        Color::LightYellow => (255, 255, 85),
        Color::LightBlue => (85, 85, 255),
        Color::LightMagenta => (255, 85, 255),
        Color::LightCyan => (85, 255, 255),
        Color::White => (255, 255, 255),
    }
}

fn formatted_disk(projects: &ProjectList, path: &Path) -> String {
    let bytes = projects
        .at_path(path)
        .and_then(|project| project.disk_usage_bytes)
        .unwrap_or(0);
    render::format_bytes(bytes)
}

pub fn formatted_disk_for_item(item: &RootItem) -> String {
    item.disk_usage_bytes()
        .map_or_else(|| render::format_bytes(0), render::format_bytes)
}

/// Body of `ProjectListPane::render`. Same pattern as the
/// other Phase-4 absorptions: typed parameters through `ctx`.
pub fn render_project_list_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut ProjectListPane,
    ctx: &PaneRenderCtx<'_>,
) {
    let projects = ctx.project_list;
    let (mut items, header, summary_line, row_width) = {
        let widths = &projects.cached_fit_widths;
        let items: Vec<ListItem> = render_tree_items(ctx, pane, &pane.viewport, widths);
        let total_str = render::format_bytes(
            projects
                .iter()
                .filter_map(|entry| entry.item.disk_usage_bytes())
                .sum(),
        );
        let header = columns::header_line(widths, "  Projects");
        let summary = columns::build_summary_cells(widths, &total_str);
        let summary_line = Some(columns::row_to_line(&summary, widths));
        let row_width = u16::try_from(widths.total_width()).unwrap_or(u16::MAX);
        (items, header, summary_line, row_width)
    };

    let total_project_rows = items.len();

    let title = project_panel_title_with_counts(pane, ctx, area.width.saturating_sub(2).into());
    let block = tui_pane::default_pane_chrome().block(title, pane.focus.is_focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        pane.viewport.clear_surface();
        pane.body_rect = Rect::ZERO;
        return;
    }

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(column_header_color())),
        header_area,
    );

    let content_area = if inner.height > 1 {
        Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1)
    } else {
        Rect::new(inner.x, inner.y, inner.width, 0)
    };
    if content_area.height == 0 {
        pane.viewport.clear_surface();
        pane.body_rect = Rect::ZERO;
        return;
    }

    let pin_summary = should_pin_project_summary(
        total_project_rows,
        summary_line.is_some(),
        content_area.height,
    );

    if !pin_summary && let Some(ref line) = summary_line {
        items.push(ListItem::new(line.clone()));
    }

    let list_area = if pin_summary && content_area.height > 1 {
        Rect::new(
            content_area.x,
            content_area.y,
            content_area.width,
            content_area.height - 1,
        )
    } else {
        content_area
    };
    pane.viewport.set_len(total_project_rows);
    pane.viewport.set_content_area(list_area);
    pane.viewport
        .set_viewport_rows(usize::from(list_area.height));
    let project_list = List::new(items);
    let mut list_state = ListState::default().with_selected(Some(projects.cursor()));
    *list_state.offset_mut() = pane.viewport.scroll_offset();
    frame.render_stateful_widget(project_list, list_area, &mut list_state);
    pane.body_rect = list_area;
    pane.viewport.set_scroll_offset(list_state.offset());
    pane.viewport.set_pos(projects.cursor());
    // The pre-Phase-5 implementation also called
    // `ctx.project_list.set_cursor(list_state.selected().unwrap_or(0))`
    // here; that mutation was a no-op (the cursor we passed in via
    // `with_selected` is the same value `selected()` returns) and
    // it required `&mut ProjectList`, which the trait dispatch
    // path doesn't supply. Dropped.
    set_project_list_dismiss_actions(pane, ctx, list_area, row_width);

    if pin_summary && let Some(line) = summary_line {
        render_project_list_footer(frame, content_area, line);
    }

    render_overflow_affordance(
        frame,
        area,
        pane.viewport.overflow(),
        Style::default().fg(label_color()),
    );
}

const DISMISS_SUFFIX: &str = " [x]";

fn set_project_list_dismiss_actions(
    pane: &mut ProjectListPane,
    ctx: &PaneRenderCtx<'_>,
    list_area: Rect,
    row_width: u16,
) {
    let visible_height = usize::from(list_area.height);
    let visible_start = pane.viewport.scroll_offset();
    let visible_end = pane
        .viewport
        .len()
        .min(visible_start.saturating_add(visible_height));
    let suffix_width = u16::try_from(columns::display_width(DISMISS_SUFFIX)).unwrap_or(u16::MAX);

    let mut actions: Vec<(Rect, DismissTarget)> = Vec::new();
    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let dismiss_target = ctx
            .project_list
            .visible_rows()
            .get(row_index)
            .copied()
            .and_then(|row| ctx.project_list.dismiss_target_for_row_inner(row));
        let Some(target) = dismiss_target else {
            continue;
        };
        let y = list_area
            .y
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        let x = list_area
            .x
            .saturating_add(row_width.saturating_sub(suffix_width));
        actions.push((Rect::new(x, y, suffix_width, 1), target));
    }
    pane.set_dismiss_actions(actions);
}

fn render_project_list_footer(frame: &mut Frame, content_area: Rect, line: Line<'static>) {
    let footer_area = Rect::new(
        content_area.x,
        content_area.y + content_area.height.saturating_sub(1),
        content_area.width,
        1,
    );
    frame.render_widget(Paragraph::new(line), footer_area);
}

fn project_panel_title_with_counts(
    pane: &ProjectListPane,
    ctx: &PaneRenderCtx<'_>,
    max_width: usize,
) -> String {
    let focused = pane.focus.is_focused;
    let cursor = ctx.project_list.cursor();
    let roots = scan::resolve_include_dirs(&ctx.config.current().tui.include_dirs);

    // No directories configured (first run): point the user at Settings,
    // which auto-opens to the Include dirs field at startup.
    if roots.is_empty() {
        return project_roots_title("Configure Include dirs in Settings", max_width);
    }

    // Count visible rows per root directory and determine which root the
    // cursor is in.
    let mut root_counts: Vec<(String, usize, usize)> = Vec::new(); // (name, count, start_row)
    for root_path in &roots {
        let name = project::home_relative_path(root_path.as_path());
        let count = ctx
            .project_list
            .iter()
            .filter(|item| item.path().starts_with(root_path.as_path()))
            .count();
        let start_row = root_counts
            .last()
            .map_or(0, |(_, prev_count, prev_start)| prev_start + prev_count);
        root_counts.push((name, count, start_row));
    }

    let groups = root_counts
        .iter()
        .map(|(name, count, start)| PaneTitleGroup {
            label:  name.clone().into(),
            len:    *count,
            cursor: focused
                .then_some(cursor)
                .filter(|cursor| *cursor >= *start && *cursor < *start + *count)
                .map(|cursor| cursor - *start),
        })
        .collect();

    let body = PaneTitleCount::Grouped(groups).body();
    project_roots_title(&body, max_width)
}

/// Build the project-list pane title from `body` (the directory list with
/// counts, or the first-run placeholder), padded with one space each side
/// and truncated with an ellipsis when it overflows `max_width`. No label
/// prefix — the home-relative paths read as directories on their own.
fn project_roots_title(body: &str, max_width: usize) -> String {
    let full = format!(" {body} ");
    if full.len() <= max_width + 2 {
        return full;
    }

    format!(
        " {} ",
        render::truncate_with_ellipsis(body, max_width.saturating_sub(2), TITLE_ELLIPSIS)
    )
}

fn should_pin_project_summary(project_rows: usize, has_summary: bool, inner_height: u16) -> bool {
    has_summary && project_rows.saturating_add(1) > usize::from(inner_height)
}

fn render_root_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    root_labels: &[String],
    root_sorted: &[u64],
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let name = &root_labels[node_index];
    let disk = formatted_disk_for_item(item);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, root_sorted));
    let ci = ctx
        .project_list
        .ci_status_for_root_item_using_lookup(&item.item, ctx.ci_status_lookup);
    let lang = if item.is_rust() {
        item.lang_icon()
    } else {
        ctx.project_list
            .at_path(item.path())
            .and_then(|p| p.language_stats.as_ref())
            .and_then(|ls| ls.entries.first())
            .map_or("  ", |e| lang::language_icon(&e.language))
    };
    let lint_cell = state::lint_cell_for(
        &Lint::status_for_root(&item.item),
        ctx.config,
        ctx.animation_elapsed,
    );
    let origin_sync = ctx.project_list.git_sync(item.path());
    let main_sync = ctx.project_list.git_main(item.path());
    let git_status = ctx.project_list.git_status_for_item(item);
    let prefix = if item.has_children() {
        if ctx
            .project_list
            .expanded
            .contains(&ExpandKey::Node(node_index))
        {
            PREFIX_ROOT_EXPANDED
        } else {
            PREFIX_ROOT_COLLAPSED
        }
    } else {
        PREFIX_ROOT_LEAF
    };
    let deleted = ctx.project_list.is_deleted(item.path());
    let wt_health = item.worktree_health();
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name,
        name_segments: app::discovery_name_segments_for_path_with_refs(
            ctx.scan,
            ctx.config,
            ctx.project_list,
            item.path(),
            name,
            git_status,
            DiscoveryRowKind::Root,
        ),
        git_status,
        lint: lint_cell,
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
        worktree_health: wt_health,
    });
    ListItem::new(columns::row_to_line(&row, widths))
}

/// Build a `ListItem` for a child project (workspace member, vendored crate,
/// or worktree).
fn render_child_item<P: project::ProjectFields>(
    ctx: &PaneRenderCtx<'_>,
    project: &P,
    name: &str,
    child_sorted: &[u64],
    prefix: &'static str,
    inherited_deleted: bool,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let path = project.path();
    let disk = formatted_disk(ctx.project_list, path);
    let disk_bytes = project.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, child_sorted));
    let lang = project::Package::lang_icon();
    let is_workspace_member = ctx.project_list.is_workspace_member_path(path);
    let is_vendored = ctx.project_list.is_vendored_path(path);
    let lint_cell = if ctx.project_list.is_rust_at_path(path) && !is_workspace_member {
        state::lint_cell_for(
            &Lint::status_for_path(ctx.project_list, path),
            ctx.config,
            ctx.animation_elapsed,
        )
    } else {
        LintCell::hidden()
    };
    let ci = if is_workspace_member || is_vendored {
        None
    } else {
        ctx.project_list
            .ci_status_using_lookup(path, ctx.ci_status_lookup)
    };
    // Members and vendored crates share their owning checkout's `.git`, so a
    // per-row probe of either resolves up to the worktree and reports the
    // worktree's own branch/main delta — not anything about the child. Blank
    // both git columns so that leaked delta never renders on a child row.
    let hide_git_status = is_workspace_member || is_vendored;
    let origin_sync = if hide_git_status
        || matches!(
            ctx.project_list.git_status_for(path),
            Some(GitStatus::Untracked | GitStatus::Ignored)
        ) {
        String::new()
    } else {
        ctx.project_list.git_sync(path)
    };
    let main_sync = if hide_git_status
        || matches!(
            ctx.project_list.git_status_for(path),
            Some(GitStatus::Untracked | GitStatus::Ignored)
        ) {
        String::new()
    } else {
        ctx.project_list.git_main(path)
    };
    let deleted = inherited_deleted || ctx.project_list.is_deleted(project.path());
    let git_status = ctx.project_list.git_status_for(path);
    let (disk_text, disk_suffix, disk_suffix_style) = if deleted {
        (
            "0.0",
            Some(" [x]"),
            Some(Style::default().fg(label_color())),
        )
    } else {
        (disk.as_str(), None, None)
    };
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name,
        name_segments: app::discovery_name_segments_for_path_with_refs(
            ctx.scan,
            ctx.config,
            ctx.project_list,
            path,
            name,
            git_status,
            DiscoveryRowKind::PathOnly,
        ),
        git_status,
        lint: lint_cell,
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
        worktree_health: project.worktree_health(),
    });
    ListItem::new(columns::row_to_line(&row, widths))
}

fn render_worktree_entry<'a>(
    ctx: &PaneRenderCtx<'_>,
    ni: usize,
    wi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let item = &ctx.project_list[ni];
    let display_path = ctx
        .project_list
        .display_path_for_row(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
    let dp = display_path.unwrap_or_default().to_string();
    let abs_path = ctx
        .project_list
        .abs_path_for_row(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let (wt_name, has_expandable_children) = worktree_entry_name_and_expandable(item, wi, &dp);

    let prefix = if has_expandable_children {
        if ctx
            .project_list
            .expanded
            .contains(&ExpandKey::Worktree(ni, wi))
        {
            PREFIX_WT_EXPANDED
        } else {
            PREFIX_WT_COLLAPSED
        }
    } else {
        PREFIX_WT_FLAT
    };
    let wt_abs = abs_path.as_deref().unwrap_or_else(|| Path::new(""));
    let disk = formatted_disk(ctx.project_list, wt_abs);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, sorted));
    let lang = item.lang_icon();
    let lint_cell = state::lint_cell_for(
        &Lint::status_for_worktree(&item.item, wi),
        ctx.config,
        ctx.animation_elapsed,
    );
    let ci = ctx
        .project_list
        .ci_status_using_lookup(wt_abs, ctx.ci_status_lookup);
    let origin_sync = ctx.project_list.git_sync(wt_abs);
    let main_sync = ctx.project_list.git_main(wt_abs);
    let deleted = ctx.project_list.is_deleted(wt_abs);
    let git_status = ctx.project_list.git_status_for(wt_abs);
    let wt_health = worktree_health_for_entry(item, wi);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name: &wt_name,
        name_segments: app::discovery_name_segments_for_path_with_refs(
            ctx.scan,
            ctx.config,
            ctx.project_list,
            wt_abs,
            &wt_name,
            git_status,
            DiscoveryRowKind::WorktreeEntry,
        ),
        git_status,
        lint: lint_cell,
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: lang,
        git_origin_sync: &origin_sync,
        git_main: &main_sync,
        ci,
        deleted,
        worktree_health: wt_health,
    });
    ListItem::new(columns::row_to_line(&row, widths))
}

fn worktree_entry_name_and_expandable(
    item: &RootItem,
    wi: usize,
    fallback: &str,
) -> (String, bool) {
    let RootItem::Worktrees(group) = item else {
        return (fallback.to_string(), false);
    };
    let entry = group.entry(wi).unwrap_or(&group.primary);
    let name = entry.root_directory_name().into_string();
    let expandable = match entry {
        RustProject::Workspace(ws) => ws.has_members(),
        RustProject::Package(_) => false,
    };
    (name, expandable)
}

fn disk_suffix_for_state(
    disk: &str,
    deleted: bool,
    health: project::WorktreeHealth,
) -> (&str, Option<&'static str>, Option<Style>) {
    if deleted {
        (
            "0.0",
            Some(" [x]"),
            Some(Style::default().fg(label_color())),
        )
    } else if matches!(health, project::WorktreeHealth::Broken) {
        (
            disk,
            Some(" [broken]"),
            Some(Style::default().fg(text_default()).bg(error_color())),
        )
    } else {
        (disk, None, None)
    }
}

fn worktree_health_for_entry(item: &RootItem, wi: usize) -> WorktreeHealth {
    let RootItem::Worktrees(group) = item else {
        return Normal;
    };
    group
        .entry(wi)
        .map_or(Normal, ProjectFields::worktree_health)
}

fn render_wt_group_header<'a>(
    ctx: &PaneRenderCtx<'_>,
    ni: usize,
    wi: usize,
    gi: usize,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let item = &ctx.project_list[ni];
    let (group_name, member_count) = match &item.item {
        RootItem::Worktrees(group) => match group.entry(wi).unwrap_or(&group.primary) {
            RustProject::Workspace(ws) => {
                let g = &ws.groups()[gi];
                (g.group_name().to_string(), g.members().len())
            },
            RustProject::Package(_) => (String::new(), 0),
        },
        _ => (String::new(), 0),
    };
    let prefix = if ctx
        .project_list
        .expanded
        .contains(&ExpandKey::WorktreeGroup(ni, wi, gi))
    {
        PREFIX_WT_GROUP_EXPANDED
    } else {
        PREFIX_WT_GROUP_COLLAPSED
    };
    let label = format!("{group_name} ({member_count})");
    let row = columns::build_group_header_cells(prefix, &label);
    ListItem::new(columns::row_to_line(&row, widths))
}

fn render_wt_member<'a>(
    ctx: &PaneRenderCtx<'_>,
    ni: usize,
    wi: usize,
    gi: usize,
    mi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let item = &ctx.project_list[ni];
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let (member, member_name, is_named_group) = match &item.item {
        RootItem::Worktrees(group) => match group.entry(wi).unwrap_or(&group.primary) {
            RustProject::Workspace(ws) => {
                let g = &ws.groups()[gi];
                let m = &g.members()[mi];
                (Some(m), m.package_name().into_string(), g.is_named())
            },
            RustProject::Package(_) => (None, String::new(), false),
        },
        _ => (None, String::new(), false),
    };
    let indent = if is_named_group {
        PREFIX_WT_MEMBER_NAMED
    } else {
        PREFIX_WT_MEMBER_INLINE
    };
    member.map_or_else(
        || {
            let row = columns::build_group_header_cells(indent, &member_name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |m| {
            let inherited_deleted = match &item.item {
                RootItem::Worktrees(group) => ctx
                    .project_list
                    .is_deleted(group.entry(wi).unwrap_or(&group.primary).path()),
                _ => false,
            };
            render_child_item(
                ctx,
                m,
                &member_name,
                sorted,
                indent,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_member_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    group_index: usize,
    member_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (member, member_name, is_named) = match &item.item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string(), group.is_named())
        },
        RootItem::Worktrees(wtg) if !wtg.renders_as_group() => {
            let Some(ws) = wtg.single_live_workspace() else {
                return ListItem::new(columns::row_to_line(
                    &columns::build_group_header_cells(PREFIX_MEMBER_INLINE, ""),
                    widths,
                ));
            };
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string(), group.is_named())
        },
        _ => (None, String::new(), false),
    };
    let indent = if is_named {
        PREFIX_MEMBER_NAMED
    } else {
        PREFIX_MEMBER_INLINE
    };
    member.map_or_else(
        || {
            let row = columns::build_group_header_cells(indent, &member_name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |m| {
            let inherited_deleted = ctx.project_list.is_deleted(item.path());
            render_child_item(
                ctx,
                m,
                &member_name,
                sorted,
                indent,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_member_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    group_index: usize,
    member_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (vendored, is_named) = match &item.item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            let member = &group.members()[member_index];
            (member.vendored().get(vendored_index), group.is_named())
        },
        RootItem::Worktrees(wtg) if !wtg.renders_as_group() => {
            let Some(ws) = wtg.single_live_workspace() else {
                return render_missing_vendored(PREFIX_MEMBER_VENDORED_INLINE, widths);
            };
            let group = &ws.groups()[group_index];
            let member = &group.members()[member_index];
            (member.vendored().get(vendored_index), group.is_named())
        },
        _ => (None, false),
    };
    let prefix = if is_named {
        PREFIX_MEMBER_VENDORED_NAMED
    } else {
        PREFIX_MEMBER_VENDORED_INLINE
    };
    render_vendored_child(ctx, item.path(), vendored, prefix, sorted, widths)
}

fn render_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (vendored, vendored_display_name) = match &item.item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            let v = &pkg.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        RootItem::Worktrees(wtg) if !wtg.renders_as_group() => {
            let entry = wtg.single_live().unwrap_or(&wtg.primary);
            let v = &entry.rust_info().vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        _ => (None, String::new()),
    };
    let name = format!("{vendored_display_name} (v)");
    vendored.map_or_else(
        || {
            let row = columns::build_group_header_cells(PREFIX_VENDORED, &name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = ctx.project_list.is_deleted(item.path());
            render_child_item(
                ctx,
                v,
                &name,
                sorted,
                PREFIX_VENDORED,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_missing_vendored(prefix: &'static str, widths: &ProjectListWidths) -> ListItem<'static> {
    let row = columns::build_group_header_cells(prefix, "");
    ListItem::new(columns::row_to_line(&row, widths))
}

fn render_vendored_child(
    ctx: &PaneRenderCtx<'_>,
    inherited_deleted_path: &Path,
    vendored: Option<&VendoredPackage>,
    prefix: &'static str,
    sorted: &[u64],
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let Some(vendored) = vendored else {
        return render_missing_vendored(prefix, widths);
    };
    let name = format!("{} (v)", vendored.package_name());
    let inherited_deleted = ctx.project_list.is_deleted(inherited_deleted_path);
    render_child_item(
        ctx,
        vendored,
        &name,
        sorted,
        prefix,
        inherited_deleted,
        widths,
    )
}

fn render_submodule_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    submodule_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let Some(submodule) = item.submodules().get(submodule_index) else {
        let row = columns::build_group_header_cells(PREFIX_SUBMODULE, "");
        return ListItem::new(columns::row_to_line(&row, widths));
    };
    let name = format!("{} (s)", submodule.name);
    let sorted = child_sorted.get(&node_index).map_or(&[][..], Vec::as_slice);
    render_path_only_entry(
        ctx,
        submodule,
        item.path(),
        PREFIX_SUBMODULE,
        &name,
        sorted,
        widths,
    )
}

fn render_path_only_entry(
    ctx: &PaneRenderCtx<'_>,
    entry: &impl crate::project::ProjectFields,
    inherited_deleted_path: &Path,
    prefix: &'static str,
    name: &str,
    sorted: &[u64],
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let path = entry.path().as_path();
    let disk = formatted_disk(ctx.project_list, path);
    let ds = disk_color(disk_percentile(entry.info().disk_usage_bytes, sorted));
    let git_status = ctx.project_list.git_status_for(path);
    let deleted =
        ctx.project_list.is_deleted(inherited_deleted_path) || ctx.project_list.is_deleted(path);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, entry.info().worktree_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name,
        name_segments: app::discovery_name_segments_for_path_with_refs(
            ctx.scan,
            ctx.config,
            ctx.project_list,
            path,
            name,
            git_status,
            DiscoveryRowKind::PathOnly,
        ),
        git_status,
        lint: LintCell::hidden(),
        disk: disk_text,
        disk_style: ds,
        disk_suffix,
        disk_suffix_style,
        lang_icon: "  ",
        git_origin_sync: "",
        git_main: "",
        ci: None,
        deleted,
        worktree_health: entry.info().worktree_health,
    });
    ListItem::new(columns::row_to_line(&row, widths))
}

fn render_wt_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    worktree_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let vendored_pkg = match &item.item {
        RootItem::Worktrees(group) => group
            .entry(worktree_index)
            .unwrap_or(&group.primary)
            .rust_info()
            .vendored()
            .get(vendored_index),
        _ => None,
    };
    let vendored_display_name =
        vendored_pkg.map_or_else(String::new, |p| p.package_name().into_string());
    let name = format!("{vendored_display_name} (v)");
    vendored_pkg.map_or_else(
        || {
            let row = columns::build_group_header_cells(PREFIX_WT_VENDORED, &name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = match &item.item {
                RootItem::Worktrees(group) => ctx
                    .project_list
                    .is_deleted(group.entry(worktree_index).unwrap_or(&group.primary).path()),
                _ => false,
            };
            render_child_item(
                ctx,
                v,
                &name,
                sorted,
                PREFIX_WT_VENDORED,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_wt_member_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    row: WorktreeMemberVendoredRow,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let WorktreeMemberVendoredRow {
        node,
        worktree,
        group: group_index,
        member: member_index,
        vendored,
    } = row;
    let item = &ctx.project_list[node];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node).unwrap_or(&empty);
    let (vendored, is_named, inherited_deleted_path) = match &item.item {
        RootItem::Worktrees(wtg) => {
            let entry = wtg.entry(worktree).unwrap_or(&wtg.primary);
            let RustProject::Workspace(ws) = entry else {
                return render_missing_vendored(PREFIX_WT_MEMBER_VENDORED_INLINE, widths);
            };
            let member_group = &ws.groups()[group_index];
            let member = &member_group.members()[member_index];
            (
                member.vendored().get(vendored),
                member_group.is_named(),
                entry.path(),
            )
        },
        _ => (None, false, item.path()),
    };
    let prefix = if is_named {
        PREFIX_WT_MEMBER_VENDORED_NAMED
    } else {
        PREFIX_WT_MEMBER_VENDORED_INLINE
    };
    render_vendored_child(
        ctx,
        inherited_deleted_path,
        vendored,
        prefix,
        sorted,
        widths,
    )
}

#[derive(Clone, Copy)]
struct WorktreeMemberVendoredRow {
    node:     usize,
    worktree: usize,
    group:    usize,
    member:   usize,
    vendored: usize,
}

pub fn render_tree_items(
    ctx: &PaneRenderCtx<'_>,
    pane: &ProjectListPane,
    viewport: &Viewport,
    widths: &ProjectListWidths,
) -> Vec<ListItem<'static>> {
    let root_sorted = &ctx.project_list.cached_root_sorted;
    let child_sorted = &ctx.project_list.cached_child_sorted;
    let root_labels = ctx
        .project_list
        .resolved_root_labels(ctx.config.include_non_rust().includes_non_rust());
    let focus = pane.focus.state;
    let cursor = ctx.project_list.cursor();

    let rows = ctx.project_list.visible_rows();
    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let item = render_tree_item(ctx, row, &root_labels, root_sorted, child_sorted, widths);
            item.style(
                tui_pane::selection_state_for(viewport, cursor, row_index, focus).overlay_style(),
            )
        })
        .collect()
}

fn render_tree_item(
    ctx: &PaneRenderCtx<'_>,
    row: &VisibleRow,
    root_labels: &[String],
    root_sorted: &[u64],
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    match row {
        VisibleRow::Root { node_index } => {
            render_root_item(ctx, *node_index, root_labels, root_sorted, widths)
        },
        VisibleRow::GroupHeader {
            node_index,
            group_index,
        } => render_group_header(ctx, *node_index, *group_index, widths),
        VisibleRow::Member {
            node_index,
            group_index,
            member_index,
        } => render_member_item(
            ctx,
            *node_index,
            *group_index,
            *member_index,
            child_sorted,
            widths,
        ),
        VisibleRow::MemberVendored {
            node_index,
            group_index,
            member_index,
            vendored_index,
        } => render_member_vendored_item(
            ctx,
            *node_index,
            *group_index,
            *member_index,
            *vendored_index,
            child_sorted,
            widths,
        ),
        VisibleRow::Vendored {
            node_index,
            vendored_index,
        } => render_vendored_item(ctx, *node_index, *vendored_index, child_sorted, widths),
        VisibleRow::WorktreeEntry {
            node_index,
            worktree_index,
        } => render_worktree_entry(ctx, *node_index, *worktree_index, child_sorted, widths),
        VisibleRow::WorktreeGroupHeader {
            node_index,
            worktree_index,
            group_index,
        } => render_wt_group_header(ctx, *node_index, *worktree_index, *group_index, widths),
        VisibleRow::WorktreeMember {
            node_index,
            worktree_index,
            group_index,
            member_index,
        } => render_wt_member(
            ctx,
            *node_index,
            *worktree_index,
            *group_index,
            *member_index,
            child_sorted,
            widths,
        ),
        VisibleRow::WorktreeMemberVendored {
            node_index,
            worktree_index,
            group_index,
            member_index,
            vendored_index,
        } => render_wt_member_vendored_item(
            ctx,
            WorktreeMemberVendoredRow {
                node:     *node_index,
                worktree: *worktree_index,
                group:    *group_index,
                member:   *member_index,
                vendored: *vendored_index,
            },
            child_sorted,
            widths,
        ),
        VisibleRow::WorktreeVendored {
            node_index,
            worktree_index,
            vendored_index,
        } => render_wt_vendored_item(
            ctx,
            *node_index,
            *worktree_index,
            *vendored_index,
            child_sorted,
            widths,
        ),
        VisibleRow::Submodule {
            node_index,
            submodule_index,
        } => render_submodule_item(ctx, *node_index, *submodule_index, child_sorted, widths),
    }
}

fn render_group_header(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    group_index: usize,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let (group_name, member_count) = match &item.item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            (group.group_name().to_string(), group.members().len())
        },
        _ => (String::new(), 0),
    };
    let prefix = if ctx
        .project_list
        .expanded
        .contains(&ExpandKey::Group(node_index, group_index))
    {
        PREFIX_GROUP_EXPANDED
    } else {
        PREFIX_GROUP_COLLAPSED
    };
    let label = format!("{group_name} ({member_count})");
    let row = columns::build_group_header_cells(prefix, &label);
    ListItem::new(columns::row_to_line(&row, widths))
}
// ── Disk-cache ───────────────────────────────────────────────────────
//
// Builds the per-row sorted disk-usage values that `disk_color` /
// `disk_percentile` consume to color the disk column.

pub fn compute_disk_cache(entries: &ProjectList) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut root_sorted = Vec::new();
    for entry in entries {
        if let Some(bytes) = entry.item.disk_usage_bytes() {
            root_sorted.push(bytes);
        }
    }
    root_sorted.sort_unstable();

    let mut child_sorted = HashMap::new();
    for (ni, entry) in entries.iter().enumerate() {
        let mut values = Vec::new();
        collect_child_disk_values(&entry.item, &mut values);
        if !values.is_empty() {
            values.sort_unstable();
            child_sorted.insert(ni, values);
        }
    }

    (root_sorted, child_sorted)
}

fn collect_child_disk_values(item: &RootItem, values: &mut Vec<u64>) {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            collect_member_group_disk(ws.groups(), values);
            collect_vendored_disk(ws.vendored(), values);
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            collect_vendored_disk(pkg.vendored(), values);
        },
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(group) => {
            for entry in group.iter_entries() {
                if let Some(bytes) = entry.disk_usage_bytes() {
                    values.push(bytes);
                }
                if let RustProject::Workspace(ws) = entry {
                    collect_member_group_disk(ws.groups(), values);
                }
                collect_vendored_disk(entry.rust_info().vendored(), values);
            }
        },
    }
    collect_project_list_entry_disk(item.submodules(), values);
}

fn collect_member_group_disk(groups: &[MemberGroup], values: &mut Vec<u64>) {
    for group in groups {
        for member in group.members() {
            if let Some(bytes) = member.disk_usage_bytes() {
                values.push(bytes);
            }
            collect_vendored_disk(member.vendored(), values);
        }
    }
}

fn collect_vendored_disk(vendored: &[VendoredPackage], values: &mut Vec<u64>) {
    for project in vendored {
        if let Some(bytes) = project.disk_usage_bytes() {
            values.push(bytes);
        }
    }
}

fn collect_project_list_entry_disk(
    entries: &[impl crate::project::ProjectFields],
    values: &mut Vec<u64>,
) {
    for entry in entries {
        if let Some(bytes) = entry.info().disk_usage_bytes {
            values.push(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::project_roots_title;
    use super::should_pin_project_summary;

    #[test]
    fn project_roots_title_adds_ellipsis_when_roots_overflow() {
        let title = project_roots_title("~/rust (12)  ~/work (7)", 20);

        assert_eq!(title, " ~/rust (12)  ~/wo… ");
    }

    #[test]
    fn project_roots_title_keeps_full_body_when_roots_fit() {
        let title = project_roots_title("~/rust (12)", 24);

        assert_eq!(title, " ~/rust (12) ");
    }

    #[test]
    fn project_roots_title_shows_configure_hint_when_no_roots() {
        let title = project_roots_title("Configure Include dirs in Settings", 50);

        assert_eq!(title, " Configure Include dirs in Settings ");
    }

    #[test]
    fn project_summary_stays_inline_when_everything_fits() {
        assert!(!should_pin_project_summary(5, true, 6));
    }

    #[test]
    fn project_summary_pins_when_list_overflows() {
        assert!(should_pin_project_summary(6, true, 6));
    }

    #[test]
    fn project_summary_does_not_pin_without_summary_content() {
        assert!(!should_pin_project_summary(100, false, 6));
    }
}
