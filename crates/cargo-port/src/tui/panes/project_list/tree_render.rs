use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::List;
use ratatui::widgets::ListItem;
use ratatui::widgets::ListState;
use ratatui::widgets::Paragraph;
use tui_pane::PaneTitleCount;
use tui_pane::PaneTitleGroup;
use tui_pane::label_color;
use tui_pane::render_overflow_affordance;

use super::ProjectListPane;
use crate::project;
use crate::project::AbsolutePath;
use crate::scan;
use crate::tui::columns;
use crate::tui::dismiss_target::DismissTarget;
use crate::tui::panes::constants::DISMISS_SUFFIX;
use crate::tui::panes::constants::TITLE_ELLIPSIS;
use crate::tui::project_list::ProjectList;
use crate::tui::project_list::VisibleRow;
use crate::tui::render;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::theme_roles;

pub(super) fn render_project_list_pane_body(
    frame: &mut Frame,
    area: Rect,
    pane: &mut ProjectListPane,
    ctx: &PaneRenderCtx<'_>,
) {
    let projects = ctx.project_list;
    let (mut items, header, summary_line, row_width) = {
        let widths = &projects.cached_fit_widths;
        let items: Vec<ListItem> = super::render_tree_items(ctx, pane, &pane.viewport, widths);
        let total_str = render::format_bytes(
            projects
                .iter()
                .filter_map(|entry| entry.root_item.disk_usage_bytes())
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
    let block = tui_pane::default_pane_chrome().block(title, pane.focus.is_focused());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 {
        pane.viewport.clear_surface();
        pane.body_rect = Rect::ZERO;
        return;
    }

    let header_area = Rect::new(inner.x, inner.y, inner.width, 1);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().fg(theme_roles::column_header_color())),
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
    let focused = pane.focus.is_focused();
    let roots = scan::resolve_include_dirs(&ctx.config.current().tui.include_dirs);

    // No directories configured (first run): point the user at Settings,
    // which auto-opens to the Include dirs field at startup.
    if roots.is_empty() {
        return project_roots_title("Configure Include dirs in Settings", max_width);
    }

    let include_non_rust = ctx.config.include_non_rust().includes_non_rust();
    let selected_root = focused
        .then(|| selected_top_level_node_index(ctx.project_list))
        .flatten();
    let groups = project_title_groups(ctx.project_list, &roots, include_non_rust, selected_root);

    let body = PaneTitleCount::Grouped(groups).body();
    project_roots_title(&body, max_width)
}

fn selected_top_level_node_index(project_list: &ProjectList) -> Option<usize> {
    let row = project_list
        .visible_rows()
        .get(project_list.cursor())
        .copied()?;
    Some(top_level_node_index(row))
}

const fn top_level_node_index(row: VisibleRow) -> usize {
    match row {
        VisibleRow::Root { node_index }
        | VisibleRow::GroupHeader { node_index, .. }
        | VisibleRow::Member { node_index, .. }
        | VisibleRow::MemberVendored { node_index, .. }
        | VisibleRow::Vendored { node_index, .. }
        | VisibleRow::Submodule { node_index, .. }
        | VisibleRow::WorktreeEntry { node_index, .. }
        | VisibleRow::WorktreeGroupHeader { node_index, .. }
        | VisibleRow::WorktreeMember { node_index, .. }
        | VisibleRow::WorktreeMemberVendored { node_index, .. }
        | VisibleRow::WorktreeVendored { node_index, .. } => node_index,
    }
}

fn project_title_groups(
    project_list: &ProjectList,
    roots: &[AbsolutePath],
    include_non_rust: bool,
    selected_root: Option<usize>,
) -> Vec<PaneTitleGroup<'static>> {
    let mut groups = Vec::new();
    for root_path in roots {
        let name = project::home_relative_path(root_path.as_path());
        let mut count = 0;
        let mut cursor = None;

        for (node_index, item) in project_list.iter().enumerate() {
            if matches!(item.visibility(), project::Visibility::Dismissed) {
                continue;
            }
            if !include_non_rust && !item.is_rust() {
                continue;
            }
            if !item.path().starts_with(root_path.as_path()) {
                continue;
            }
            if selected_root == Some(node_index) {
                cursor = Some(count);
            }
            count += 1;
        }

        groups.push(PaneTitleGroup {
            label: name.into(),
            len: count,
            cursor,
        });
    }

    groups
}

/// Build the project-list pane title from `body` (the directory list with
/// counts, or the first-run placeholder), padded with one space each side
/// and truncated with an ellipsis when it overflows `max_width`. No label
/// prefix — the home-relative paths read as directories on their own.
pub(super) fn project_roots_title(body: &str, max_width: usize) -> String {
    let full = format!(" {body} ");
    if full.len() <= max_width + 2 {
        return full;
    }

    format!(
        " {} ",
        render::truncate_with_ellipsis(body, max_width.saturating_sub(2), TITLE_ELLIPSIS)
    )
}

pub(super) fn should_pin_project_summary(
    project_rows: usize,
    has_summary: bool,
    inner_height: u16,
) -> bool {
    has_summary && project_rows.saturating_add(1) > usize::from(inner_height)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tui_pane::PaneTitleCount;

    use super::project_roots_title;
    use super::project_title_groups;
    use super::selected_top_level_node_index;
    use super::should_pin_project_summary;
    use crate::project::AbsolutePath;
    use crate::project::MemberGroup;
    use crate::project::NonRustProject;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::Visibility;
    use crate::project::Workspace;
    use crate::project::WorktreeGroup;
    use crate::tui::project_list::ExpandKey;
    use crate::tui::project_list::ProjectList;

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
    fn project_title_cursor_tracks_parent_root_for_expanded_child_rows() {
        let rust_root = path("/workspace/rust");
        let mut list = ProjectList::new(vec![
            package("/workspace/rust/alpha"),
            workspace_worktree_group("/workspace/rust/bravo", "/workspace/rust/bravo_feature"),
            package("/workspace/rust/charlie"),
        ]);
        list.expanded = HashSet::from([ExpandKey::Node(1), ExpandKey::Worktree(1, 0)]);
        list.recompute_visibility(true);
        list.set_cursor(3);

        assert_eq!(
            PaneTitleCount::Grouped(project_title_groups(
                &list,
                &[rust_root],
                true,
                selected_top_level_node_index(&list),
            ))
            .body(),
            "/workspace/rust (2 of 3)"
        );
    }

    #[test]
    fn project_title_counts_rendered_top_level_roots() {
        let rust_root = path("/workspace/rust");
        let mut hidden = Package {
            path: self::path("/workspace/rust/hidden"),
            ..Package::default()
        };
        hidden.rust.project_info.visibility = Visibility::Dismissed;
        let hidden = RootItem::Rust(RustProject::Package(hidden));

        let list = ProjectList::new(vec![
            package("/workspace/rust/alpha"),
            workspace_worktree_group("/workspace/rust/bravo", "/workspace/rust/bravo_feature"),
            non_rust("/workspace/rust/docs"),
            hidden,
        ]);

        assert_eq!(
            PaneTitleCount::Grouped(project_title_groups(
                &list,
                std::slice::from_ref(&rust_root),
                true,
                None
            ))
            .body(),
            "/workspace/rust (3)"
        );
        assert_eq!(
            PaneTitleCount::Grouped(project_title_groups(&list, &[rust_root], false, None)).body(),
            "/workspace/rust (2)"
        );
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

    fn package(path: &str) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: self::path(path),
            ..Package::default()
        }))
    }

    fn non_rust(path: &str) -> RootItem {
        RootItem::NonRust(NonRustProject::new(self::path(path), None))
    }

    fn workspace_worktree_group(primary: &str, linked: &str) -> RootItem {
        RootItem::Worktrees(WorktreeGroup::new(
            RustProject::Workspace(Workspace {
                path: self::path(primary),
                groups: vec![MemberGroup::Inline {
                    members: vec![Package {
                        path: self::path(&format!("{primary}/member")),
                        ..Package::default()
                    }],
                }],
                ..Workspace::default()
            }),
            vec![RustProject::Workspace(Workspace {
                path: self::path(linked),
                ..Workspace::default()
            })],
        ))
    }

    fn path(path: &str) -> AbsolutePath { AbsolutePath::from(path) }
}
