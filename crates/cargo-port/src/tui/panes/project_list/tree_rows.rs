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
use crate::tui::panes::constants::PREFIX_ROOT_COLLAPSED;
use crate::tui::panes::constants::PREFIX_ROOT_EXPANDED;
use crate::tui::panes::constants::PREFIX_ROOT_LEAF;
use crate::tui::panes::constants::TREE_PREFIX_BLANK;
use crate::tui::panes::constants::TREE_PREFIX_BRANCH;
use crate::tui::panes::constants::TREE_PREFIX_COLLAPSED;
use crate::tui::panes::constants::TREE_PREFIX_CONTINUATION;
use crate::tui::panes::constants::TREE_PREFIX_EXPANDED;
use crate::tui::panes::constants::TREE_PREFIX_LAST;
use crate::tui::panes::constants::TREE_PREFIX_LEAF_EXTENSION;
use crate::tui::panes::lang;
use crate::tui::project_list::ProjectList;
use crate::tui::render_context::PaneRenderCtx;
use crate::tui::state;
use crate::tui::state::Lint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeSegment {
    Root(usize),
    Group(usize, usize),
    Member(usize, usize, usize),
    MemberVendored(usize, usize, usize, usize),
    Vendored(usize, usize),
    Worktree(usize, usize),
    WorktreeGroup(usize, usize, usize),
    WorktreeMember(usize, usize, usize, usize),
    WorktreeMemberVendored(usize, usize, usize, usize, usize),
    WorktreeVendored(usize, usize, usize),
    Submodule(usize, usize),
}

type TreeLineage = Vec<TreeSegment>;

fn tree_lineage(project_list: &ProjectList, row: VisibleRow) -> TreeLineage {
    match row {
        VisibleRow::Root { node_index } => root_lineage(node_index),
        VisibleRow::GroupHeader {
            node_index,
            group_index,
        } => group_lineage(node_index, group_index),
        VisibleRow::Member {
            node_index,
            group_index,
            member_index,
        } => member_lineage(project_list, node_index, group_index, member_index),
        VisibleRow::MemberVendored {
            node_index,
            group_index,
            member_index,
            vendored_index,
        } => member_vendored_lineage(
            project_list,
            MemberVendoredRow {
                node:     node_index,
                group:    group_index,
                member:   member_index,
                vendored: vendored_index,
            },
        ),
        VisibleRow::Vendored {
            node_index,
            vendored_index,
        } => vendored_lineage(node_index, vendored_index),
        VisibleRow::WorktreeEntry {
            node_index,
            worktree_index,
        } => worktree_lineage(node_index, worktree_index),
        VisibleRow::WorktreeGroupHeader {
            node_index,
            worktree_index,
            group_index,
        } => worktree_group_lineage(node_index, worktree_index, group_index),
        VisibleRow::WorktreeMember {
            node_index,
            worktree_index,
            group_index,
            member_index,
        } => worktree_member_lineage(
            project_list,
            node_index,
            worktree_index,
            group_index,
            member_index,
        ),
        VisibleRow::WorktreeMemberVendored {
            node_index,
            worktree_index,
            group_index,
            member_index,
            vendored_index,
        } => worktree_member_vendored_lineage(
            project_list,
            WorktreeMemberVendoredRow {
                node:     node_index,
                worktree: worktree_index,
                group:    group_index,
                member:   member_index,
                vendored: vendored_index,
            },
        ),
        VisibleRow::WorktreeVendored {
            node_index,
            worktree_index,
            vendored_index,
        } => worktree_vendored_lineage(node_index, worktree_index, vendored_index),
        VisibleRow::Submodule {
            node_index,
            submodule_index,
        } => submodule_lineage(node_index, submodule_index),
    }
}

fn root_lineage(node_index: usize) -> TreeLineage { vec![TreeSegment::Root(node_index)] }

fn group_lineage(node_index: usize, group_index: usize) -> TreeLineage {
    vec![
        TreeSegment::Root(node_index),
        TreeSegment::Group(node_index, group_index),
    ]
}

fn vendored_lineage(node_index: usize, vendored_index: usize) -> TreeLineage {
    vec![
        TreeSegment::Root(node_index),
        TreeSegment::Vendored(node_index, vendored_index),
    ]
}

fn worktree_lineage(node_index: usize, worktree_index: usize) -> TreeLineage {
    vec![
        TreeSegment::Root(node_index),
        TreeSegment::Worktree(node_index, worktree_index),
    ]
}

fn worktree_group_lineage(
    node_index: usize,
    worktree_index: usize,
    group_index: usize,
) -> TreeLineage {
    vec![
        TreeSegment::Root(node_index),
        TreeSegment::Worktree(node_index, worktree_index),
        TreeSegment::WorktreeGroup(node_index, worktree_index, group_index),
    ]
}

fn worktree_vendored_lineage(
    node_index: usize,
    worktree_index: usize,
    vendored_index: usize,
) -> TreeLineage {
    vec![
        TreeSegment::Root(node_index),
        TreeSegment::Worktree(node_index, worktree_index),
        TreeSegment::WorktreeVendored(node_index, worktree_index, vendored_index),
    ]
}

fn submodule_lineage(node_index: usize, submodule_index: usize) -> TreeLineage {
    vec![
        TreeSegment::Root(node_index),
        TreeSegment::Submodule(node_index, submodule_index),
    ]
}

fn member_vendored_lineage(project_list: &ProjectList, row: MemberVendoredRow) -> TreeLineage {
    let MemberVendoredRow {
        node,
        group,
        member,
        vendored,
    } = row;
    let mut lineage = member_lineage(project_list, node, group, member);
    lineage.push(TreeSegment::MemberVendored(node, group, member, vendored));
    lineage
}

fn worktree_member_vendored_lineage(
    project_list: &ProjectList,
    row: WorktreeMemberVendoredRow,
) -> TreeLineage {
    let WorktreeMemberVendoredRow {
        node,
        worktree,
        group,
        member,
        vendored,
    } = row;
    let mut lineage = worktree_member_lineage(project_list, node, worktree, group, member);
    lineage.push(TreeSegment::WorktreeMemberVendored(
        node, worktree, group, member, vendored,
    ));
    lineage
}

fn member_lineage(
    project_list: &ProjectList,
    node_index: usize,
    group_index: usize,
    member_index: usize,
) -> TreeLineage {
    let mut lineage = vec![TreeSegment::Root(node_index)];
    if !root_group_is_inline(project_list, node_index, group_index) {
        lineage.push(TreeSegment::Group(node_index, group_index));
    }
    lineage.push(TreeSegment::Member(node_index, group_index, member_index));
    lineage
}

fn worktree_member_lineage(
    project_list: &ProjectList,
    node_index: usize,
    worktree_index: usize,
    group_index: usize,
    member_index: usize,
) -> TreeLineage {
    let mut lineage = vec![
        TreeSegment::Root(node_index),
        TreeSegment::Worktree(node_index, worktree_index),
    ];
    if !worktree_group_is_inline(project_list, node_index, worktree_index, group_index) {
        lineage.push(TreeSegment::WorktreeGroup(
            node_index,
            worktree_index,
            group_index,
        ));
    }
    lineage.push(TreeSegment::WorktreeMember(
        node_index,
        worktree_index,
        group_index,
        member_index,
    ));
    lineage
}

fn root_group_is_inline(project_list: &ProjectList, node_index: usize, group_index: usize) -> bool {
    let Some(item) = project_list.get(node_index) else {
        return true;
    };
    match &item.root_item {
        RootItem::Rust(RustProject::Workspace(ws)) => ws
            .groups()
            .get(group_index)
            .is_none_or(|group| !group.is_named()),
        RootItem::Worktrees(worktree_group) if !worktree_group.renders_as_group() => worktree_group
            .single_live_workspace()
            .and_then(|ws| ws.groups().get(group_index))
            .is_none_or(|group| !group.is_named()),
        _ => true,
    }
}

fn worktree_group_is_inline(
    project_list: &ProjectList,
    node_index: usize,
    worktree_index: usize,
    group_index: usize,
) -> bool {
    let Some(item) = project_list.get(node_index) else {
        return true;
    };
    match &item.root_item {
        RootItem::Worktrees(group) => match group.entry(worktree_index) {
            Some(RustProject::Workspace(ws)) => ws
                .groups()
                .get(group_index)
                .is_none_or(|group| !group.is_named()),
            _ => true,
        },
        _ => true,
    }
}

fn tree_prefix(
    project_list: &ProjectList,
    lineages: &[TreeLineage],
    row_index: usize,
    row: VisibleRow,
) -> String {
    let Some(lineage) = lineages.get(row_index) else {
        return String::new();
    };
    if lineage.len() <= 1 {
        return root_prefix(project_list, row).to_string();
    }

    let mut prefix = String::new();
    for segment_index in 1..lineage.len() - 1 {
        if has_later_sibling(lineages, row_index, lineage, segment_index) {
            prefix.push_str(TREE_PREFIX_CONTINUATION);
        } else {
            prefix.push_str(TREE_PREFIX_BLANK);
        }
    }

    let row_segment_index = lineage.len() - 1;
    if has_later_sibling(lineages, row_index, lineage, row_segment_index) {
        prefix.push_str(TREE_PREFIX_BRANCH);
    } else {
        prefix.push_str(TREE_PREFIX_LAST);
    }
    if let Some(key) = project_list.expand_key_for_row(row) {
        if project_list.expanded.contains(&key) {
            prefix.push_str(TREE_PREFIX_EXPANDED);
        } else {
            prefix.push_str(TREE_PREFIX_COLLAPSED);
        }
    } else {
        prefix.push_str(TREE_PREFIX_LEAF_EXTENSION);
    }
    prefix
}

fn root_prefix(project_list: &ProjectList, row: VisibleRow) -> &'static str {
    let VisibleRow::Root { node_index } = row else {
        return "";
    };
    let Some(item) = project_list.get(node_index) else {
        return PREFIX_ROOT_LEAF;
    };
    if !item.has_children() {
        return PREFIX_ROOT_LEAF;
    }
    if project_list.expanded.contains(&ExpandKey::Node(node_index)) {
        PREFIX_ROOT_EXPANDED
    } else {
        PREFIX_ROOT_COLLAPSED
    }
}

fn has_later_sibling(
    lineages: &[TreeLineage],
    row_index: usize,
    lineage: &[TreeSegment],
    segment_index: usize,
) -> bool {
    lineages
        .iter()
        .skip(row_index.saturating_add(1))
        .any(|candidate| {
            candidate.len() > segment_index
                && candidate[..segment_index] == lineage[..segment_index]
                && candidate[segment_index] != lineage[segment_index]
        })
}

fn render_root_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    prefix: &str,
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
    let deleted = ctx.project_list.is_deleted(item.path());
    let worktree_health = item.worktree_health();
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, worktree_health);
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
        worktree_health,
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
    prefix: &str,
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
    prefix: &str,
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

    let (worktree_name, _) = worktree_entry_name_and_expandable(item, wi, &dp);
    let worktree_path = abs_path.as_deref().unwrap_or_else(|| Path::new(""));
    let disk = disk::formatted_disk(ctx.project_list, worktree_path);
    let disk_bytes = worktree_entry_disk_bytes(item, wi);
    let ds = disk::disk_color(disk::disk_percentile(disk_bytes, sorted));
    let lang = item.lang_icon();
    let lint_cell = state::lint_cell_for(
        &Lint::status_for_worktree(&item.root_item, wi),
        ctx.config,
        ctx.animation_elapsed,
    );
    let ci = ctx
        .project_list
        .ci_status_using_lookup(worktree_path, ctx.ci_status_lookup);
    let origin_sync = ctx.project_list.git_sync(worktree_path);
    let main_sync = ctx.project_list.git_main(worktree_path);
    let deleted = ctx.project_list.is_deleted(worktree_path);
    let git_status = ctx.project_list.git_status_for(worktree_path);
    let worktree_health = worktree_health_for_entry(item, wi);
    let (disk_text, disk_suffix, disk_suffix_style) =
        disk_suffix_for_state(&disk, deleted, worktree_health);
    let row = columns::build_row_cells(ProjectRow {
        prefix,
        name: &worktree_name,
        name_segments: app::discovery_name_segments_for_path_with_refs(
            ctx.scan,
            ctx.config,
            ctx.project_list,
            worktree_path,
            &worktree_name,
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
        worktree_health,
    });
    ListItem::new(columns::row_to_line(&row, widths))
}

fn worktree_entry_disk_bytes(item: &RootItem, wi: usize) -> Option<u64> {
    let RootItem::Worktrees(group) = item else {
        return item.disk_usage_bytes();
    };
    group.entry(wi).and_then(ProjectFields::disk_usage_bytes)
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
        RustProject::Workspace(ws) => ws.has_members() || !ws.vendored().is_empty(),
        RustProject::Package(pkg) => !pkg.vendored().is_empty(),
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
    prefix: &str,
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
    let label = format!("{group_name} ({member_count})");
    let row = columns::build_group_header_cells(prefix, &label);
    ListItem::new(columns::row_to_line(&row, widths))
}

fn render_wt_member<'a>(
    ctx: &PaneRenderCtx<'_>,
    row: WorktreeMemberRow,
    prefix: &str,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'a> {
    let WorktreeMemberRow {
        node: ni,
        worktree: wi,
        group: gi,
        member: mi,
    } = row;
    let item = &ctx.project_list[ni];
    let empty = Vec::new();
    let sorted = child_sorted.get(&ni).unwrap_or(&empty);

    let (member, member_name) = match &item.root_item {
        RootItem::Worktrees(group) => match group.entry(wi).unwrap_or(&group.primary) {
            RustProject::Workspace(ws) => {
                let g = &ws.groups()[gi];
                let m = &g.members()[mi];
                (Some(m), m.package_name().into_string())
            },
            RustProject::Package(_) => (None, String::new()),
        },
        _ => (None, String::new()),
    };
    member.map_or_else(
        || {
            let row = columns::build_group_header_cells(prefix, &member_name);
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
                prefix,
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
    prefix: &str,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let (member, member_name) = match &item.root_item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string())
        },
        RootItem::Worktrees(worktree_group) if !worktree_group.renders_as_group() => {
            let Some(ws) = worktree_group.single_live_workspace() else {
                return ListItem::new(columns::row_to_line(
                    &columns::build_group_header_cells(prefix, ""),
                    widths,
                ));
            };
            let group = &ws.groups()[group_index];
            let m = &group.members()[member_index];
            (Some(m), m.package_name().into_string())
        },
        _ => (None, String::new()),
    };
    member.map_or_else(
        || {
            let row = columns::build_group_header_cells(prefix, &member_name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |m| {
            let inherited_deleted = ctx.project_list.is_deleted(item.path());
            render_child_item(
                ctx,
                m,
                &member_name,
                sorted,
                prefix,
                inherited_deleted,
                widths,
            )
        },
    )
}

fn render_member_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    row: MemberVendoredRow,
    prefix: &str,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let MemberVendoredRow {
        node: node_index,
        group: group_index,
        member: member_index,
        vendored: vendored_index,
    } = row;
    let item = &ctx.project_list[node_index];
    let empty = Vec::new();
    let sorted = child_sorted.get(&node_index).unwrap_or(&empty);
    let vendored = match &item.root_item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            let member = &group.members()[member_index];
            member.vendored().get(vendored_index)
        },
        RootItem::Worktrees(worktree_group) if !worktree_group.renders_as_group() => {
            let Some(ws) = worktree_group.single_live_workspace() else {
                return render_missing_vendored(prefix, widths);
            };
            let group = &ws.groups()[group_index];
            let member = &group.members()[member_index];
            member.vendored().get(vendored_index)
        },
        _ => None,
    };
    render_vendored_child(ctx, item.path(), vendored, prefix, sorted, widths)
}

fn render_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    vendored_index: usize,
    prefix: &str,
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
        RootItem::Worktrees(worktree_group) if !worktree_group.renders_as_group() => {
            let entry = worktree_group
                .single_live()
                .unwrap_or(&worktree_group.primary);
            let v = &entry.rust_info().vendored()[vendored_index];
            (Some(v), v.package_name().into_string())
        },
        _ => (None, String::new()),
    };
    let name = format!("{vendored_display_name} (v)");
    vendored.map_or_else(
        || {
            let row = columns::build_group_header_cells(prefix, &name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = ctx.project_list.is_deleted(item.path());
            render_child_item(ctx, v, &name, sorted, prefix, inherited_deleted, widths)
        },
    )
}

fn render_missing_vendored(prefix: &str, widths: &ProjectListWidths) -> ListItem<'static> {
    let row = columns::build_group_header_cells(prefix, "");
    ListItem::new(columns::row_to_line(&row, widths))
}

fn render_vendored_child(
    ctx: &PaneRenderCtx<'_>,
    inherited_deleted_path: &Path,
    vendored: Option<&VendoredPackage>,
    prefix: &str,
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
    prefix: &str,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let Some(submodule) = item.submodules().get(submodule_index) else {
        let row = columns::build_group_header_cells(prefix, "");
        return ListItem::new(columns::row_to_line(&row, widths));
    };
    let name = format!("{} (s)", submodule.name);
    let sorted = child_sorted.get(&node_index).map_or(&[][..], Vec::as_slice);
    render_path_only_entry(ctx, submodule, item.path(), prefix, &name, sorted, widths)
}

fn render_path_only_entry(
    ctx: &PaneRenderCtx<'_>,
    entry: &impl crate::project::ProjectFields,
    inherited_deleted_path: &Path,
    prefix: &str,
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
    prefix: &str,
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
            let row = columns::build_group_header_cells(prefix, &name);
            ListItem::new(columns::row_to_line(&row, widths))
        },
        |v| {
            let inherited_deleted = match &item.root_item {
                RootItem::Worktrees(group) => ctx
                    .project_list
                    .is_deleted(group.entry(worktree_index).unwrap_or(&group.primary).path()),
                _ => false,
            };
            render_child_item(ctx, v, &name, sorted, prefix, inherited_deleted, widths)
        },
    )
}

fn render_wt_member_vendored_item(
    ctx: &PaneRenderCtx<'_>,
    row: WorktreeMemberVendoredRow,
    prefix: &str,
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
    let (vendored, inherited_deleted_path) = match &item.root_item {
        RootItem::Worktrees(worktree_group) => {
            let entry = worktree_group
                .entry(worktree)
                .unwrap_or(&worktree_group.primary);
            let RustProject::Workspace(ws) = entry else {
                return render_missing_vendored(prefix, widths);
            };
            let member_group = &ws.groups()[group_index];
            let member = &member_group.members()[member_index];
            (member.vendored().get(vendored), entry.path())
        },
        _ => (None, item.path()),
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
struct MemberVendoredRow {
    node:     usize,
    group:    usize,
    member:   usize,
    vendored: usize,
}

#[derive(Clone, Copy)]
struct WorktreeMemberRow {
    node:     usize,
    worktree: usize,
    group:    usize,
    member:   usize,
}

#[derive(Clone, Copy)]
struct WorktreeMemberVendoredRow {
    node:     usize,
    worktree: usize,
    group:    usize,
    member:   usize,
    vendored: usize,
}

#[derive(Clone, Copy)]
enum ProjectTreeRow {
    Root {
        node_index: usize,
    },
    GroupHeader {
        node_index:  usize,
        group_index: usize,
    },
    Member {
        node_index:   usize,
        group_index:  usize,
        member_index: usize,
    },
    MemberVendored {
        node_index:     usize,
        group_index:    usize,
        member_index:   usize,
        vendored_index: usize,
    },
    Vendored {
        node_index:     usize,
        vendored_index: usize,
    },
    Submodule {
        node_index:      usize,
        submodule_index: usize,
    },
}

#[derive(Clone, Copy)]
enum WorktreeTreeRow {
    Entry {
        node_index:     usize,
        worktree_index: usize,
    },
    GroupHeader {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
    },
    Member {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
        member_index:   usize,
    },
    MemberVendored {
        node_index:     usize,
        worktree_index: usize,
        group_index:    usize,
        member_index:   usize,
        vendored_index: usize,
    },
    Vendored {
        node_index:     usize,
        worktree_index: usize,
        vendored_index: usize,
    },
}

#[derive(Clone, Copy)]
enum TreeRenderRow {
    Project(ProjectTreeRow),
    Worktree(WorktreeTreeRow),
}

impl From<VisibleRow> for TreeRenderRow {
    fn from(row: VisibleRow) -> Self {
        match row {
            VisibleRow::Root { node_index } => Self::Project(ProjectTreeRow::Root { node_index }),
            VisibleRow::GroupHeader {
                node_index,
                group_index,
            } => Self::Project(ProjectTreeRow::GroupHeader {
                node_index,
                group_index,
            }),
            VisibleRow::Member {
                node_index,
                group_index,
                member_index,
            } => Self::Project(ProjectTreeRow::Member {
                node_index,
                group_index,
                member_index,
            }),
            VisibleRow::MemberVendored {
                node_index,
                group_index,
                member_index,
                vendored_index,
            } => Self::Project(ProjectTreeRow::MemberVendored {
                node_index,
                group_index,
                member_index,
                vendored_index,
            }),
            VisibleRow::Vendored {
                node_index,
                vendored_index,
            } => Self::Project(ProjectTreeRow::Vendored {
                node_index,
                vendored_index,
            }),
            VisibleRow::Submodule {
                node_index,
                submodule_index,
            } => Self::Project(ProjectTreeRow::Submodule {
                node_index,
                submodule_index,
            }),
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => Self::Worktree(WorktreeTreeRow::Entry {
                node_index,
                worktree_index,
            }),
            VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                group_index,
            } => Self::Worktree(WorktreeTreeRow::GroupHeader {
                node_index,
                worktree_index,
                group_index,
            }),
            VisibleRow::WorktreeMember {
                node_index,
                worktree_index,
                group_index,
                member_index,
            } => Self::Worktree(WorktreeTreeRow::Member {
                node_index,
                worktree_index,
                group_index,
                member_index,
            }),
            VisibleRow::WorktreeMemberVendored {
                node_index,
                worktree_index,
                group_index,
                member_index,
                vendored_index,
            } => Self::Worktree(WorktreeTreeRow::MemberVendored {
                node_index,
                worktree_index,
                group_index,
                member_index,
                vendored_index,
            }),
            VisibleRow::WorktreeVendored {
                node_index,
                worktree_index,
                vendored_index,
            } => Self::Worktree(WorktreeTreeRow::Vendored {
                node_index,
                worktree_index,
                vendored_index,
            }),
        }
    }
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
    let lineages: Vec<_> = rows
        .iter()
        .copied()
        .map(|row| tree_lineage(ctx.project_list, row))
        .collect();
    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            let prefix = tree_prefix(ctx.project_list, &lineages, row_index, *row);
            let item = render_tree_item(
                ctx,
                *row,
                &prefix,
                &root_labels,
                root_sorted,
                child_sorted,
                widths,
            );
            item.style(
                tui_pane::selection_state_for(viewport, cursor, row_index, pane_focus_state)
                    .overlay_style(),
            )
        })
        .collect()
}

fn render_tree_item(
    ctx: &PaneRenderCtx<'_>,
    row: VisibleRow,
    prefix: &str,
    root_labels: &[String],
    root_sorted: &[u64],
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    match TreeRenderRow::from(row) {
        TreeRenderRow::Project(row) => render_project_tree_item(
            ctx,
            row,
            prefix,
            root_labels,
            root_sorted,
            child_sorted,
            widths,
        ),
        TreeRenderRow::Worktree(row) => {
            render_worktree_tree_item(ctx, row, prefix, child_sorted, widths)
        },
    }
}

fn render_project_tree_item(
    ctx: &PaneRenderCtx<'_>,
    row: ProjectTreeRow,
    prefix: &str,
    root_labels: &[String],
    root_sorted: &[u64],
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    match row {
        ProjectTreeRow::Root { node_index } => {
            render_root_item(ctx, node_index, prefix, root_labels, root_sorted, widths)
        },
        ProjectTreeRow::GroupHeader {
            node_index,
            group_index,
        } => render_group_header(ctx, node_index, group_index, prefix, widths),
        ProjectTreeRow::Member {
            node_index,
            group_index,
            member_index,
        } => render_member_item(
            ctx,
            node_index,
            group_index,
            member_index,
            prefix,
            child_sorted,
            widths,
        ),
        ProjectTreeRow::MemberVendored {
            node_index,
            group_index,
            member_index,
            vendored_index,
        } => render_member_vendored_item(
            ctx,
            MemberVendoredRow {
                node:     node_index,
                group:    group_index,
                member:   member_index,
                vendored: vendored_index,
            },
            prefix,
            child_sorted,
            widths,
        ),
        ProjectTreeRow::Vendored {
            node_index,
            vendored_index,
        } => render_vendored_item(
            ctx,
            node_index,
            vendored_index,
            prefix,
            child_sorted,
            widths,
        ),
        ProjectTreeRow::Submodule {
            node_index,
            submodule_index,
        } => render_submodule_item(
            ctx,
            node_index,
            submodule_index,
            prefix,
            child_sorted,
            widths,
        ),
    }
}

fn render_worktree_tree_item(
    ctx: &PaneRenderCtx<'_>,
    row: WorktreeTreeRow,
    prefix: &str,
    child_sorted: &HashMap<usize, Vec<u64>>,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    match row {
        WorktreeTreeRow::Entry {
            node_index,
            worktree_index,
        } => render_worktree_entry(
            ctx,
            node_index,
            worktree_index,
            prefix,
            child_sorted,
            widths,
        ),
        WorktreeTreeRow::GroupHeader {
            node_index,
            worktree_index,
            group_index,
        } => render_wt_group_header(ctx, node_index, worktree_index, group_index, prefix, widths),
        WorktreeTreeRow::Member {
            node_index,
            worktree_index,
            group_index,
            member_index,
        } => render_wt_member(
            ctx,
            WorktreeMemberRow {
                node:     node_index,
                worktree: worktree_index,
                group:    group_index,
                member:   member_index,
            },
            prefix,
            child_sorted,
            widths,
        ),
        WorktreeTreeRow::MemberVendored {
            node_index,
            worktree_index,
            group_index,
            member_index,
            vendored_index,
        } => render_wt_member_vendored_item(
            ctx,
            WorktreeMemberVendoredRow {
                node:     node_index,
                worktree: worktree_index,
                group:    group_index,
                member:   member_index,
                vendored: vendored_index,
            },
            prefix,
            child_sorted,
            widths,
        ),
        WorktreeTreeRow::Vendored {
            node_index,
            worktree_index,
            vendored_index,
        } => render_wt_vendored_item(
            ctx,
            node_index,
            worktree_index,
            vendored_index,
            prefix,
            child_sorted,
            widths,
        ),
    }
}

fn render_group_header(
    ctx: &PaneRenderCtx<'_>,
    node_index: usize,
    group_index: usize,
    prefix: &str,
    widths: &ProjectListWidths,
) -> ListItem<'static> {
    let item = &ctx.project_list[node_index];
    let (group_name, member_count) = match &item.root_item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            let group = &ws.groups()[group_index];
            (group.group_name().to_string(), group.members().len())
        },
        RootItem::Worktrees(worktree_group) if !worktree_group.renders_as_group() => {
            let Some(ws) = worktree_group.single_live_workspace() else {
                return ListItem::new(columns::row_to_line(
                    &columns::build_group_header_cells(prefix, ""),
                    widths,
                ));
            };
            let group = &ws.groups()[group_index];
            (group.group_name().to_string(), group.members().len())
        },
        _ => (String::new(), 0),
    };
    let label = format!("{group_name} ({member_count})");
    let row = columns::build_group_header_cells(prefix, &label);
    ListItem::new(columns::row_to_line(&row, widths))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::project::AbsolutePath;
    use crate::project::Package;
    use crate::project::ProjectInfo;
    use crate::project::RustInfo;
    use crate::project::WorktreeStatus;
    use crate::tui::project_list::ProjectList;

    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;

    fn package(path: &str, bytes: u64, worktree_status: WorktreeStatus) -> Package {
        Package {
            path: AbsolutePath::from(path.to_string()),
            worktree_status,
            rust: RustInfo {
                project_info: ProjectInfo {
                    disk_usage_bytes: Some(bytes),
                    ..ProjectInfo::default()
                },
                ..RustInfo::default()
            },
            ..Package::default()
        }
    }

    #[test]
    fn worktree_entry_disk_percentile_uses_checkout_bytes_on_global_scale() {
        let linked_bytes = 470 * MIB;
        let peer_bytes = 500 * MIB;
        let primary_bytes = 80 * GIB;
        let primary_path = AbsolutePath::from("/repo".to_string());
        let peer = RootItem::Rust(RustProject::Package(package(
            "/peer",
            peer_bytes,
            WorktreeStatus::NotGit,
        )));
        let group = RootItem::Worktrees(WorktreeGroup::new(
            RustProject::Package(package(
                "/repo",
                primary_bytes,
                WorktreeStatus::Primary {
                    root: primary_path.clone(),
                },
            )),
            vec![RustProject::Package(package(
                "/repo_linked",
                linked_bytes,
                WorktreeStatus::Linked {
                    primary: primary_path,
                },
            ))],
        ));
        let list = ProjectList::new(vec![peer, group.clone()]);

        let (all_sorted, child_sorted) = disk::compute_disk_cache(&list);
        let group_sorted = child_sorted
            .get(&1)
            .expect("worktree group should have child disk cache");

        assert_eq!(
            group_sorted, &all_sorted,
            "worktree rows should use the same disk scale as root rows"
        );
        assert_eq!(worktree_entry_disk_bytes(&group, 1), Some(linked_bytes));
        assert_eq!(group.disk_usage_bytes(), Some(primary_bytes + linked_bytes));

        let linked_percentile =
            disk::disk_percentile(Some(linked_bytes), group_sorted).expect("linked percentile");
        let peer_percentile =
            disk::disk_percentile(Some(peer_bytes), &all_sorted).expect("peer percentile");
        let group_percentile = disk::disk_percentile(group.disk_usage_bytes(), group_sorted)
            .expect("group percentile");

        assert!(
            linked_percentile < peer_percentile,
            "linked checkout should grade below the larger peer row"
        );
        assert!(
            (group_percentile - 1.0).abs() < f64::EPSILON,
            "the aggregate worktree group remains the largest disk value"
        );
    }
}
