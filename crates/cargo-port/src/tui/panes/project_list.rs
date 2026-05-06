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

use super::constants::PREFIX_GROUP_COLLAPSED;
use super::constants::PREFIX_GROUP_EXPANDED;
use super::constants::PREFIX_MEMBER_INLINE;
use super::constants::PREFIX_MEMBER_NAMED;
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
use super::constants::PREFIX_WT_VENDORED;
use super::lang;
use super::spec::PaneId;
use crate::project;
use crate::project::MemberGroup;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::WorktreeGroup;
use crate::project::WorktreeHealth;
use crate::project::WorktreeHealth::Normal;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::DiscoveryRowKind;
use crate::tui::app::DismissTarget;
use crate::tui::app::ExpandKey;
use crate::tui::app::ProjectListWidths;
use crate::tui::app::VisibleRow;
use crate::tui::columns;
use crate::tui::columns::ProjectRow;
use crate::tui::constants::COLUMN_HEADER_COLOR;
use crate::tui::constants::ERROR_COLOR;
use crate::tui::constants::LABEL_COLOR;
use crate::tui::pane;
use crate::tui::pane::PaneTitleCount;
use crate::tui::pane::PaneTitleGroup;
use crate::tui::project_list::ProjectList;
use crate::tui::render;

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

/// Compute a color for a disk value: green (smallest) → white (middle) → red (largest).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "values are clamped to 0.0..=255.0 before cast"
)]
fn disk_color(percentile: Option<f64>) -> Style {
    let Some(pos) = percentile else {
        return Style::default().fg(LABEL_COLOR);
    };

    // Green (0.0) → White (0.5) → Red (1.0)
    let (r, g, b) = if pos < 0.5 {
        // Green to white: increase R and B
        let t = pos * 2.0;
        (
            155.0f64.mul_add(t, 100.0).clamp(0.0, 255.0) as u8,
            35.0f64.mul_add(t, 220.0).clamp(0.0, 255.0) as u8,
            155.0f64.mul_add(t, 100.0).clamp(0.0, 255.0) as u8,
        )
    } else {
        // White to red: decrease G and B
        let t = (pos - 0.5) * 2.0;
        let gb = 155.0f64.mul_add(-t, 255.0).clamp(0.0, 255.0) as u8;
        (255, gb, gb)
    };

    Style::default().fg(Color::Rgb(r, g, b))
}

pub fn formatted_disk(app: &App, path: &Path) -> String {
    let bytes = app
        .project_list
        .at_path(path)
        .and_then(|project| project.disk_usage_bytes)
        .unwrap_or(0);
    render::format_bytes(bytes)
}

pub fn formatted_disk_for_item(item: &RootItem) -> String {
    item.disk_usage_bytes()
        .map_or_else(|| render::format_bytes(0), render::format_bytes)
}

pub fn render_project_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let (mut items, header, summary_line, row_width) = {
        let widths = &app.project_list.cached_fit_widths;
        let items: Vec<ListItem> = render_tree_items(app, widths);
        let total_str = render::format_bytes(
            app.project_list
                .iter()
                .filter_map(|entry| entry.item.disk_usage_bytes())
                .sum(),
        );
        let header = columns::header_line(widths, " Projects");
        let summary = columns::build_summary_cells(widths, &total_str);
        let summary_line = Some(columns::row_to_line(&summary, widths));
        let row_width = u16::try_from(widths.total_width()).unwrap_or(u16::MAX);
        (items, header, summary_line, row_width)
    };

    let total_project_rows = items.len();

    let title = project_panel_title_with_counts(app, area.width.saturating_sub(2).into());
    let block = pane::default_pane_chrome().block(title, app.focus.is(PaneId::ProjectList));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        clear_project_list_surface(app);
        app.layout_cache.project_list_body = Rect::ZERO;
        return;
    }

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(COLUMN_HEADER_COLOR)),
        header_area,
    );

    let content_area = if inner.height > 1 {
        Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1)
    } else {
        Rect::new(inner.x, inner.y, inner.width, 0)
    };
    if content_area.height == 0 {
        clear_project_list_surface(app);
        app.layout_cache.project_list_body = Rect::ZERO;
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
    app.panes.project_list.viewport.set_len(total_project_rows);
    app.panes.project_list.viewport.set_content_area(list_area);
    app.panes
        .project_list
        .viewport
        .set_viewport_rows(usize::from(list_area.height));
    let project_list = List::new(items);
    let mut list_state = ListState::default().with_selected(Some(app.project_list.cursor()));
    *list_state.offset_mut() = app.panes.project_list.viewport.scroll_offset();
    frame.render_stateful_widget(project_list, list_area, &mut list_state);
    app.layout_cache.project_list_body = list_area;
    app.panes
        .project_list
        .viewport
        .set_scroll_offset(list_state.offset());
    app.project_list
        .set_cursor(list_state.selected().unwrap_or(0));
    set_project_list_dismiss_actions(app, list_area, row_width);

    if pin_summary && let Some(line) = summary_line {
        render_project_list_footer(frame, content_area, line);
    }

    pane::render_overflow_affordance(frame, area, &app.panes.project_list.viewport);
}

const DISMISS_SUFFIX: &str = " [x]";

fn set_project_list_dismiss_actions(app: &mut App, list_area: Rect, row_width: u16) {
    let visible_height = usize::from(list_area.height);
    let visible_start = app.panes.project_list.viewport.scroll_offset();
    let visible_end = app
        .panes
        .project_list
        .viewport
        .len()
        .min(visible_start.saturating_add(visible_height));
    let suffix_width = u16::try_from(columns::display_width(DISMISS_SUFFIX)).unwrap_or(u16::MAX);

    let mut actions: Vec<(Rect, DismissTarget)> = Vec::new();
    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let dismiss_target = app
            .visible_rows()
            .get(row_index)
            .copied()
            .and_then(|row| app.dismiss_target_for_row(row));
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
    app.panes.project_list.set_dismiss_actions(actions);
}

const fn clear_project_list_surface(app: &mut App) {
    app.panes.project_list.viewport.clear_surface();
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

fn project_panel_title_with_counts(app: &App, max_width: usize) -> String {
    let focused = app.focus.is(PaneId::ProjectList);
    let cursor = app.project_list.cursor();
    let roots = scan::resolve_include_dirs(&app.config.current().tui.include_dirs);

    // Count visible rows per root directory and determine which root the
    // cursor is in.
    let mut root_counts: Vec<(String, usize, usize)> = Vec::new(); // (name, count, start_row)
    for root_path in &roots {
        let name = project::home_relative_path(root_path.as_path());
        let count = app
            .project_list
            .iter()
            .filter(|item| item.path().starts_with(root_path.as_path()))
            .count();
        let start_row = root_counts
            .last()
            .map_or(0, |(_, prev_count, prev_start)| prev_start + prev_count);
        root_counts.push((name, count, start_row));
    }

    let prefix = "Roots: ";
    let inner_max = max_width.saturating_sub(2);
    if inner_max <= prefix.len() {
        return format!(" {prefix} ");
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
    let full = format!(" {prefix}{body} ");
    if full.len() <= max_width + 2 {
        return full;
    }
    // Truncate if too long.
    format!(
        " {prefix}{} ",
        render::truncate_to_width(&body, inner_max.saturating_sub(prefix.len()))
    )
}

fn should_pin_project_summary(project_rows: usize, has_summary: bool, inner_height: u16) -> bool {
    has_summary && project_rows.saturating_add(1) > usize::from(inner_height)
}

fn render_root_item(
    app: &App,
    node_index: usize,
    root_labels: &[String],
    root_sorted: &[u64],
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &app.project_list[node_index];
    let name = &root_labels[node_index];
    let disk = formatted_disk_for_item(item);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, root_sorted));
    let ci = app.ci_for_item(item);
    let lang = if item.is_rust() {
        item.lang_icon()
    } else {
        app.project_list
            .at_path(item.path())
            .and_then(|p| p.language_stats.as_ref())
            .and_then(|ls| ls.entries.first())
            .map_or("  ", |e| lang::language_icon(&e.language))
    };
    let lint_cell = app.lint_cell(&crate::tui::lint_state::Lint::status_for_root(&item.item));
    let origin_sync = app.project_list.git_sync(item.path());
    let main_sync = app.project_list.git_main(item.path());
    let git_status = app.project_list.git_status_for_item(item);
    let prefix = if item.has_children() {
        if app
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
    let deleted = app.project_list.is_deleted(item.path());
    let wt_health = item.worktree_health();
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name,
        name_segments: app.discovery_name_segments_for_path(
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
    app: &App,
    project: &P,
    name: &str,
    child_sorted: &[u64],
    prefix: &'static str,
    inherited_deleted: bool,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let path = project.path();
    let disk = formatted_disk(app, path);
    let disk_bytes = project.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, child_sorted));
    let lang = project::Package::lang_icon();
    let lint_cell = if app.project_list.is_rust_at_path(path) {
        app.lint_cell(&crate::tui::lint_state::Lint::status_for_path(
            &app.project_list,
            path,
        ))
    } else {
        crate::tui::columns::LintCell::hidden()
    };
    let ci = app.ci_for(path);
    let hide_git_status = app.project_list.is_workspace_member_path(path);
    let origin_sync = if hide_git_status
        || matches!(
            app.project_list.git_status_for(path),
            Some(crate::project::GitStatus::Untracked | crate::project::GitStatus::Ignored)
        ) {
        String::new()
    } else {
        app.project_list.git_sync(path)
    };
    let main_sync = if hide_git_status
        || matches!(
            app.project_list.git_status_for(path),
            Some(crate::project::GitStatus::Untracked | crate::project::GitStatus::Ignored)
        ) {
        String::new()
    } else {
        app.project_list.git_main(path)
    };
    let deleted = inherited_deleted || app.project_list.is_deleted(project.path());
    let git_status = app.project_list.git_status_for(path);
    let (disk_text, disk_suffix, disk_suffix_style) = if deleted {
        ("0.0", Some(" [x]"), Some(Style::default().fg(LABEL_COLOR)))
    } else {
        (disk.as_str(), None, None)
    };
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name,
        name_segments: app.discovery_name_segments_for_path(
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
    app: &App,
    ni: usize,
    wi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let item = &app.project_list[ni];
    let display_path = app
        .project_list
        .display_path_for_row(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
    let dp = display_path.unwrap_or_default().to_string();
    let abs_path = app
        .project_list
        .abs_path_for_row(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let (wt_name, has_expandable_children) = worktree_entry_name_and_expandable(item, wi, &dp);

    let prefix = if has_expandable_children {
        if app
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
    let disk = formatted_disk(app, wt_abs);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk_color(disk_percentile(disk_bytes, sorted));
    let lang = item.lang_icon();
    let lint_cell = app.lint_cell(&crate::tui::lint_state::Lint::status_for_worktree(
        &item.item, wi,
    ));
    let ci = app.ci_for(wt_abs);
    let origin_sync = app.project_list.git_sync(wt_abs);
    let main_sync = app.project_list.git_main(wt_abs);
    let deleted = app.project_list.is_deleted(wt_abs);
    let git_status = app.project_list.git_status_for(wt_abs);
    let wt_health = worktree_health_for_entry(item, wi);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, wt_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name: &wt_name,
        name_segments: app.discovery_name_segments_for_path(
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
    let name = match item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            ws.root_directory_name().into_string()
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            let pkg = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            pkg.root_directory_name().into_string()
        },
        _ => fallback.to_string(),
    };

    let expandable = match item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            ws.has_members()
        },
        _ => false,
    };

    (name, expandable)
}

fn disk_suffix_for_state(
    disk: &str,
    deleted: bool,
    health: project::WorktreeHealth,
) -> (&str, Option<&'static str>, Option<Style>) {
    if deleted {
        ("0.0", Some(" [x]"), Some(Style::default().fg(LABEL_COLOR)))
    } else if matches!(health, project::WorktreeHealth::Broken) {
        (
            disk,
            Some(" [broken]"),
            Some(Style::default().fg(Color::White).bg(ERROR_COLOR)),
        )
    } else {
        (disk, None, None)
    }
}

fn worktree_health_for_entry(item: &RootItem, wi: usize) -> WorktreeHealth {
    match item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            if wi == 0 {
                primary.worktree_health()
            } else {
                linked
                    .get(wi - 1)
                    .map_or(Normal, ProjectFields::worktree_health)
            }
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            if wi == 0 {
                primary.worktree_health()
            } else {
                linked
                    .get(wi - 1)
                    .map_or(Normal, ProjectFields::worktree_health)
            }
        },
        _ => Normal,
    }
}

fn render_wt_group_header<'a>(
    app: &App,
    ni: usize,
    wi: usize,
    gi: usize,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let item = &app.project_list[ni];
    let (group_name, member_count) = match &item.item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            let group = &ws.groups()[gi];
            (group.group_name().to_string(), group.members().len())
        },
        _ => (String::new(), 0),
    };
    let prefix = if app
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
    app: &App,
    ni: usize,
    wi: usize,
    gi: usize,
    mi: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let item = &app.project_list[ni];
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let (member, member_name, is_named_group) = match &item.item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            let ws = if wi == 0 {
                primary
            } else {
                linked.get(wi - 1).unwrap_or(primary)
            };
            let group = &ws.groups()[gi];
            let m = &group.members()[mi];
            (Some(m), m.package_name().into_string(), group.is_named())
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
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    let ws = if wi == 0 {
                        primary
                    } else {
                        linked.get(wi - 1).unwrap_or(primary)
                    };
                    app.project_list.is_deleted(ws.path())
                },
                _ => false,
            };
            render_child_item(
                app,
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
    app: &App,
    node_index: usize,
    group_index: usize,
    member_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &app.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (member, member_name, is_named) = match &item.item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string(), group.is_named())
        },
        RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { primary, .. })
            if !wtg.renders_as_group() =>
        {
            let ws = wtg.single_live_workspace().unwrap_or(primary);
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
            let inherited_deleted = app.project_list.is_deleted(item.path());
            render_child_item(
                app,
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

fn render_vendored_item(
    app: &App,
    node_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &app.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (vendored, vendored_display_name) = match &item.item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { primary, .. })
            if !wtg.renders_as_group() =>
        {
            let ws = wtg.single_live_workspace().unwrap_or(primary);
            let v = &ws.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            let v = &pkg.vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        RootItem::Worktrees(wtg @ WorktreeGroup::Packages { primary, .. })
            if !wtg.renders_as_group() =>
        {
            let pkg = wtg.single_live_package().unwrap_or(primary);
            let v = &pkg.vendored()[vendored_index];
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
            let inherited_deleted = app.project_list.is_deleted(item.path());
            render_child_item(
                app,
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

fn render_submodule_item(
    app: &App,
    node_index: usize,
    submodule_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &app.project_list[node_index];
    let Some(submodule) = item.submodules().get(submodule_index) else {
        let row = columns::build_group_header_cells(PREFIX_SUBMODULE, "");
        return ListItem::new(columns::row_to_line(&row, widths));
    };
    let name = format!("{} (s)", submodule.name);
    let sorted = child_sorted.get(&node_index).map_or(&[][..], Vec::as_slice);
    render_path_only_entry(
        app,
        submodule,
        item.path(),
        PREFIX_SUBMODULE,
        &name,
        sorted,
        widths,
    )
}

fn render_path_only_entry(
    app: &App,
    entry: &impl crate::project::ProjectFields,
    inherited_deleted_path: &Path,
    prefix: &'static str,
    name: &str,
    sorted: &[u64],
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let path = entry.path().as_path();
    let disk = formatted_disk(app, path);
    let ds = disk_color(disk_percentile(entry.info().disk_usage_bytes, sorted));
    let git_status = app.project_list.git_status_for(path);
    let deleted =
        app.project_list.is_deleted(inherited_deleted_path) || app.project_list.is_deleted(path);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, entry.info().worktree_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name,
        name_segments: app.discovery_name_segments_for_path(
            path,
            name,
            git_status,
            DiscoveryRowKind::PathOnly,
        ),
        git_status,
        lint: crate::tui::columns::LintCell::hidden(),
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
    app: &App,
    node_index: usize,
    worktree_index: usize,
    vendored_index: usize,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &app.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let vendored_pkg = match &item.item {
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            let ws = if worktree_index == 0 {
                primary
            } else {
                linked.get(worktree_index - 1).unwrap_or(primary)
            };
            ws.vendored().get(vendored_index)
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            let pkg = if worktree_index == 0 {
                primary
            } else {
                linked.get(worktree_index - 1).unwrap_or(primary)
            };
            pkg.vendored().get(vendored_index)
        },
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
                RootItem::Worktrees(WorktreeGroup::Workspaces {
                    primary, linked, ..
                }) => {
                    let ws = if worktree_index == 0 {
                        primary
                    } else {
                        linked.get(worktree_index - 1).unwrap_or(primary)
                    };
                    app.project_list.is_deleted(ws.path())
                },
                RootItem::Worktrees(WorktreeGroup::Packages {
                    primary, linked, ..
                }) => {
                    let pkg = if worktree_index == 0 {
                        primary
                    } else {
                        linked.get(worktree_index - 1).unwrap_or(primary)
                    };
                    app.project_list.is_deleted(pkg.path())
                },
                _ => false,
            };
            render_child_item(
                app,
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

pub fn render_tree_items(app: &App, widths: &ProjectListWidths) -> Vec<ListItem<'static>> {
    let root_sorted = &app.project_list.cached_root_sorted;
    let child_sorted = &app.project_list.cached_child_sorted;
    let root_labels = app
        .project_list
        .resolved_root_labels(app.config.include_non_rust().includes_non_rust());
    let focus = app.focus.pane_state(PaneId::ProjectList);
    let pane = &app.panes.project_list.viewport;
    let cursor = app.project_list.cursor();

    let rows = app.visible_rows();
    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let item = render_tree_item(app, row, &root_labels, root_sorted, child_sorted, widths);
            item.style(
                pane.selection_state_for(cursor, row_index, focus)
                    .overlay_style(),
            )
        })
        .collect()
}

fn render_tree_item(
    app: &App,
    row: &VisibleRow,
    root_labels: &[String],
    root_sorted: &[u64],
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    match row {
        VisibleRow::Root { node_index } => {
            render_root_item(app, *node_index, root_labels, root_sorted, widths)
        },
        VisibleRow::GroupHeader {
            node_index,
            group_index,
        } => {
            let item = &app.project_list[*node_index];
            let (group_name, member_count) = match &item.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    let group = &ws.groups()[*group_index];
                    (group.group_name().to_string(), group.members().len())
                },
                _ => (String::new(), 0),
            };
            let prefix = if app
                .project_list
                .expanded
                .contains(&ExpandKey::Group(*node_index, *group_index))
            {
                PREFIX_GROUP_EXPANDED
            } else {
                PREFIX_GROUP_COLLAPSED
            };
            let label = format!("{group_name} ({member_count})");
            let row = columns::build_group_header_cells(prefix, &label);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        VisibleRow::Member {
            node_index,
            group_index,
            member_index,
        } => render_member_item(
            app,
            *node_index,
            *group_index,
            *member_index,
            child_sorted,
            widths,
        ),
        VisibleRow::Vendored {
            node_index,
            vendored_index,
        } => render_vendored_item(app, *node_index, *vendored_index, child_sorted, widths),
        VisibleRow::WorktreeEntry {
            node_index,
            worktree_index,
        } => render_worktree_entry(app, *node_index, *worktree_index, child_sorted, widths),
        VisibleRow::WorktreeGroupHeader {
            node_index,
            worktree_index,
            group_index,
        } => render_wt_group_header(app, *node_index, *worktree_index, *group_index, widths),
        VisibleRow::WorktreeMember {
            node_index,
            worktree_index,
            group_index,
            member_index,
        } => render_wt_member(
            app,
            *node_index,
            *worktree_index,
            *group_index,
            *member_index,
            child_sorted,
            widths,
        ),
        VisibleRow::WorktreeVendored {
            node_index,
            worktree_index,
            vendored_index,
        } => render_wt_vendored_item(
            app,
            *node_index,
            *worktree_index,
            *vendored_index,
            child_sorted,
            widths,
        ),
        VisibleRow::Submodule {
            node_index,
            submodule_index,
        } => render_submodule_item(app, *node_index, *submodule_index, child_sorted, widths),
    }
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
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::WorktreeGroup;
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            collect_member_group_disk(ws.groups(), values);
            collect_vendored_disk(ws.vendored(), values);
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            collect_vendored_disk(pkg.vendored(), values);
        },
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            for ws in std::iter::once(primary).chain(linked.iter()) {
                if let Some(bytes) = ws.disk_usage_bytes() {
                    values.push(bytes);
                }
                collect_member_group_disk(ws.groups(), values);
                collect_vendored_disk(ws.vendored(), values);
            }
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            for pkg in std::iter::once(primary).chain(linked.iter()) {
                if let Some(bytes) = pkg.disk_usage_bytes() {
                    values.push(bytes);
                }
                collect_vendored_disk(pkg.vendored(), values);
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
    use super::should_pin_project_summary;

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
