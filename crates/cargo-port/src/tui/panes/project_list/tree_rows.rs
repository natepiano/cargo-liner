use std::collections::HashMap;
use std::path::Path;

use ratatui::style::Style;
use ratatui::widgets::ListItem;
use tui_pane::Viewport;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::text_default;

use super::ProjectListPane;
use super::disk;
use crate::project;
use crate::project::GitStatus;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::WorktreeGroup;
use crate::project::WorktreeHealth;
use crate::project::WorktreeHealth::Normal;
use crate::tui::app;
use crate::tui::app::DiscoveryRowKind;
use crate::tui::app::ExpandKey;
use crate::tui::app::ProjectListWidths;
use crate::tui::app::VisibleRow;
use crate::tui::columns;
use crate::tui::columns::LintCell;
use crate::tui::columns::ProjectRow;
use crate::tui::columns::RowLifecycle;
use crate::tui::panes::constants::PREFIX_GROUP_COLLAPSED;
use crate::tui::panes::constants::PREFIX_GROUP_EXPANDED;
use crate::tui::panes::constants::PREFIX_MEMBER_INLINE;
use crate::tui::panes::constants::PREFIX_MEMBER_NAMED;
use crate::tui::panes::constants::PREFIX_MEMBER_VENDORED_INLINE;
use crate::tui::panes::constants::PREFIX_MEMBER_VENDORED_NAMED;
use crate::tui::panes::constants::PREFIX_ROOT_COLLAPSED;
use crate::tui::panes::constants::PREFIX_ROOT_EXPANDED;
use crate::tui::panes::constants::PREFIX_ROOT_LEAF;
use crate::tui::panes::constants::PREFIX_SUBMODULE;
use crate::tui::panes::constants::PREFIX_VENDORED;
use crate::tui::panes::constants::PREFIX_WT_COLLAPSED;
use crate::tui::panes::constants::PREFIX_WT_EXPANDED;
use crate::tui::panes::constants::PREFIX_WT_FLAT;
use crate::tui::panes::constants::PREFIX_WT_GROUP_COLLAPSED;
use crate::tui::panes::constants::PREFIX_WT_GROUP_EXPANDED;
use crate::tui::panes::constants::PREFIX_WT_MEMBER_INLINE;
use crate::tui::panes::constants::PREFIX_WT_MEMBER_NAMED;
use crate::tui::panes::constants::PREFIX_WT_MEMBER_VENDORED_INLINE;
use crate::tui::panes::constants::PREFIX_WT_MEMBER_VENDORED_NAMED;
use crate::tui::panes::constants::PREFIX_WT_VENDORED;
use crate::tui::panes::lang;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::state;
use crate::tui::state::Lint;

fn render_root_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    root_labels: &[String],
    root_sorted: &[u64],
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let name = &root_labels[node_index];
    let disk = disk::formatted_disk_for_item(item);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk::disk_color(disk::disk_percentile(disk_bytes, root_sorted));
    let ci = ctx
        .project_list
        .ci_status_for_root_item_using_lookup(&item.root_item, ctx.ci_status_lookup);
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
        &Lint::status_for_root(&item.root_item),
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
        lifecycle: RowLifecycle::from(deleted),
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
    let disk = disk::formatted_disk(ctx.project_list, path);
    let disk_bytes = project.disk_usage_bytes();
    let ds = disk::disk_color(disk::disk_percentile(disk_bytes, child_sorted));
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
        lifecycle: RowLifecycle::from(deleted),
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
    let disk = disk::formatted_disk(ctx.project_list, wt_abs);
    let disk_bytes = item.disk_usage_bytes();
    let ds = disk::disk_color(disk::disk_percentile(disk_bytes, sorted));
    let lang = item.lang_icon();
    let lint_cell = state::lint_cell_for(
        &Lint::status_for_worktree(&item.root_item, wi),
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
        lifecycle: RowLifecycle::from(deleted),
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
    let mut name = entry.root_directory_name().into_string();
    if renders_primary_marker(group, wi) {
        name.push_str(" (p)");
    }
    let expandable = match entry {
        RustProject::Workspace(ws) => ws.has_members(),
        RustProject::Package(_) => false,
    };
    (name, expandable)
}

fn renders_primary_marker(group: &WorktreeGroup, wi: usize) -> bool {
    wi == 0
        && group.visible_entry_count() > 2
        && group
            .entry(wi)
            .is_some_and(|entry| entry.visibility() == Visibility::Visible)
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
    let (group_name, member_count) = match &item.root_item {
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

    let (member, member_name, is_named_group) = match &item.root_item {
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
            let inherited_deleted = match &item.root_item {
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
    let (member, member_name, is_named) = match &item.root_item {
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
    let (vendored, is_named) = match &item.root_item {
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
    let (vendored, vendored_display_name) = match &item.root_item {
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
    let disk = disk::formatted_disk(ctx.project_list, path);
    let ds = disk::disk_color(disk::disk_percentile(entry.info().disk_usage_bytes, sorted));
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
        lifecycle: RowLifecycle::from(deleted),
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
    let vendored_pkg = match &item.root_item {
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
            let inherited_deleted = match &item.root_item {
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
    let (vendored, is_named, inherited_deleted_path) = match &item.root_item {
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

pub(super) fn render_tree_items(
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
    let pane_focus_state = pane.focus.pane_focus_state;
    let cursor = ctx.project_list.cursor();

    let rows = ctx.project_list.visible_rows();
    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let item = render_tree_item(ctx, row, &root_labels, root_sorted, child_sorted, widths);
            item.style(
                tui_pane::selection_state_for(viewport, cursor, row_index, pane_focus_state)
                    .overlay_style(),
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
    let (group_name, member_count) = match &item.root_item {
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
