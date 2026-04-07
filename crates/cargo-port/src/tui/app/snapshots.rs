use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use super::types::App;
use super::types::ExpandKey;
use super::types::VisibleRow;
use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::LegacyProject;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::Project;
use crate::project::ProjectListItem;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::tui::columns;
use crate::tui::columns::COL_DISK;
use crate::tui::columns::COL_SYNC;
use crate::tui::columns::ResolvedWidths;
use crate::tui::render;
use crate::tui::render::PREFIX_GROUP_COLLAPSED;
use crate::tui::render::PREFIX_MEMBER_INLINE;
use crate::tui::render::PREFIX_MEMBER_NAMED;
use crate::tui::render::PREFIX_VENDORED;
use crate::tui::render::PREFIX_WT_COLLAPSED;
use crate::tui::render::PREFIX_WT_FLAT;
use crate::tui::render::PREFIX_WT_GROUP_COLLAPSED;
use crate::tui::render::PREFIX_WT_MEMBER_INLINE;
use crate::tui::render::PREFIX_WT_MEMBER_NAMED;
use crate::tui::render::PREFIX_WT_VENDORED;

/// Build the flat list of visible rows from the project list and expansion state.
pub(super) fn build_visible_rows(
    items: &[ProjectListItem],
    expanded: &HashSet<ExpandKey>,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (ni, item) in items.iter().enumerate() {
        if matches!(item.visibility(), Visibility::Dismissed) {
            continue;
        }
        rows.push(VisibleRow::Root { node_index: ni });
        if !expanded.contains(&ExpandKey::Node(ni)) {
            continue;
        }

        match item {
            ProjectListItem::Workspace(ws) => {
                emit_groups(&mut rows, ni, ws.groups(), expanded);
                emit_vendored_rows(&mut rows, ni, ws.vendored());
            },
            ProjectListItem::Package(pkg) => {
                emit_vendored_rows(&mut rows, ni, pkg.vendored());
            },
            ProjectListItem::NonRust(_) => {},
            ProjectListItem::WorkspaceWorktrees(wtg) => {
                emit_workspace_worktree_group(&mut rows, ni, wtg, expanded);
            },
            ProjectListItem::PackageWorktrees(wtg) => {
                emit_package_worktree_group(&mut rows, ni, wtg, expanded);
            },
        }
    }
    rows
}

fn emit_groups(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    groups: &[MemberGroup],
    expanded: &HashSet<ExpandKey>,
) {
    for (gi, group) in groups.iter().enumerate() {
        match group {
            MemberGroup::Inline { members } => {
                for (mi, _) in members.iter().enumerate() {
                    rows.push(VisibleRow::Member {
                        node_index:   ni,
                        group_index:  gi,
                        member_index: mi,
                    });
                }
            },
            MemberGroup::Named { members, .. } => {
                rows.push(VisibleRow::GroupHeader {
                    node_index:  ni,
                    group_index: gi,
                });
                if expanded.contains(&ExpandKey::Group(ni, gi)) {
                    for (mi, _) in members.iter().enumerate() {
                        rows.push(VisibleRow::Member {
                            node_index:   ni,
                            group_index:  gi,
                            member_index: mi,
                        });
                    }
                }
            },
        }
    }
}

fn emit_vendored_rows(rows: &mut Vec<VisibleRow>, ni: usize, vendored: &[Project<Package>]) {
    for (vi, _) in vendored.iter().enumerate() {
        rows.push(VisibleRow::Vendored {
            node_index:     ni,
            vendored_index: vi,
        });
    }
}

/// Emit worktree entries for a `WorktreeGroup<Workspace>`.
fn emit_workspace_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup<Workspace>,
    expanded: &HashSet<ExpandKey>,
) {
    // Primary at index 0
    if !matches!(wtg.primary().visibility(), Visibility::Dismissed) {
        let wi = 0;
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if wtg.primary().has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            emit_worktree_children(
                rows,
                ni,
                wi,
                wtg.primary().groups(),
                wtg.primary().vendored(),
                expanded,
            );
        }
    }
    // Linked at indices 1..
    for (i, linked) in wtg.linked().iter().enumerate() {
        if matches!(linked.visibility(), Visibility::Dismissed) {
            continue;
        }
        let wi = i + 1;
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if linked.has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            emit_worktree_children(rows, ni, wi, linked.groups(), linked.vendored(), expanded);
        }
    }
}

/// Emit worktree entries for a `WorktreeGroup<Package>`.
fn emit_package_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup<Package>,
    _expanded: &HashSet<ExpandKey>,
) {
    // Primary at index 0
    if !matches!(wtg.primary().visibility(), Visibility::Dismissed) {
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: 0,
        });
    }
    // Linked at indices 1..
    for (i, linked) in wtg.linked().iter().enumerate() {
        if matches!(linked.visibility(), Visibility::Dismissed) {
            continue;
        }
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: i + 1,
        });
    }
}

/// Emit group headers, members, and vendored rows for an expanded worktree entry.
fn emit_worktree_children(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wi: usize,
    groups: &[MemberGroup],
    vendored: &[Project<Package>],
    expanded: &HashSet<ExpandKey>,
) {
    for (gi, group) in groups.iter().enumerate() {
        match group {
            MemberGroup::Inline { members } => {
                for (mi, _) in members.iter().enumerate() {
                    rows.push(VisibleRow::WorktreeMember {
                        node_index:     ni,
                        worktree_index: wi,
                        group_index:    gi,
                        member_index:   mi,
                    });
                }
            },
            MemberGroup::Named { members, .. } => {
                rows.push(VisibleRow::WorktreeGroupHeader {
                    node_index:     ni,
                    worktree_index: wi,
                    group_index:    gi,
                });
                if expanded.contains(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                    for (mi, _) in members.iter().enumerate() {
                        rows.push(VisibleRow::WorktreeMember {
                            node_index:     ni,
                            worktree_index: wi,
                            group_index:    gi,
                            member_index:   mi,
                        });
                    }
                }
            },
        }
    }

    for (vi, _) in vendored.iter().enumerate() {
        rows.push(VisibleRow::WorktreeVendored {
            node_index:     ni,
            worktree_index: wi,
            vendored_index: vi,
        });
    }
}

pub(super) fn formatted_disk_snapshot(disk_usage: &HashMap<String, u64>, path: &str) -> String {
    disk_usage
        .get(path)
        .copied()
        .map_or_else(|| render::format_bytes(0), render::format_bytes)
}

/// Collect all unique display paths for a `ProjectListItem`.
fn unique_item_display_paths(item: &ProjectListItem) -> Vec<String> {
    let mut paths = Vec::new();
    let mut push_unique = |p: String| {
        if !paths.contains(&p) {
            paths.push(p);
        }
    };
    push_unique(item.display_path());
    match item {
        ProjectListItem::WorkspaceWorktrees(wtg) => {
            for linked in wtg.linked() {
                push_unique(linked.display_path());
            }
        },
        ProjectListItem::PackageWorktrees(wtg) => {
            for linked in wtg.linked() {
                push_unique(linked.display_path());
            }
        },
        _ => {},
    }
    paths
}

fn disk_bytes_for_item_snapshot(
    item: &ProjectListItem,
    disk_usage: &HashMap<String, u64>,
) -> Option<u64> {
    let paths = unique_item_display_paths(item);
    if paths.len() == 1 {
        return disk_usage.get(&paths[0]).copied();
    }
    let mut total: u64 = 0;
    let mut any_data = false;
    for path in &paths {
        if let Some(&bytes) = disk_usage.get(path.as_str()) {
            total += bytes;
            any_data = true;
        }
    }
    if any_data { Some(total) } else { None }
}

fn formatted_disk_for_item_snapshot(
    item: &ProjectListItem,
    disk_usage: &HashMap<String, u64>,
) -> String {
    disk_bytes_for_item_snapshot(item, disk_usage)
        .map_or_else(|| render::format_bytes(0), render::format_bytes)
}

pub(super) fn git_sync_snapshot(
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
    path: &str,
) -> String {
    if matches!(
        git_path_states
            .get(path)
            .copied()
            .unwrap_or(GitPathState::OutsideRepo),
        GitPathState::Untracked | GitPathState::Ignored
    ) {
        return String::new();
    }
    let Some(info) = git_info.get(path) else {
        return String::new();
    };
    match info.ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None if info.origin != GitOrigin::Local => "-".to_string(),
        None => String::new(),
    }
}

/// Snapshot of project state needed for fit-width calculations.
pub(super) struct FitWidthsState<'a> {
    pub disk_usage:      &'a HashMap<String, u64>,
    pub git_info:        &'a HashMap<String, GitInfo>,
    pub git_path_states: &'a HashMap<String, GitPathState>,
}

pub(super) fn build_fit_widths_snapshot(
    items: &[ProjectListItem],
    state: &FitWidthsState<'_>,
    lint_enabled: bool,
    generation: u64,
) -> ResolvedWidths {
    let mut widths = ResolvedWidths::new(lint_enabled);

    for item in items {
        observe_item_fit_widths(&mut widths, item, state);
    }

    widths.generation = generation;
    widths
}

fn observe_item_fit_widths(
    widths: &mut ResolvedWidths,
    item: &ProjectListItem,
    state: &FitWidthsState<'_>,
) {
    let dw = columns::display_width;
    let root_path = item.display_path();

    App::observe_name_width(widths, App::fit_name_for_item(item));
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_for_item_snapshot(item, state.disk_usage)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            state.git_info,
            state.git_path_states,
            &root_path,
        )),
    );

    match item {
        ProjectListItem::Workspace(ws) => {
            observe_new_member_group_fit_widths(widths, ws.groups(), state, false);
            observe_typed_vendored_fit_widths(
                widths,
                ws.vendored(),
                state.disk_usage,
                PREFIX_VENDORED,
            );
        },
        ProjectListItem::Package(pkg) => {
            observe_typed_vendored_fit_widths(
                widths,
                pkg.vendored(),
                state.disk_usage,
                PREFIX_VENDORED,
            );
        },
        ProjectListItem::NonRust(_) => {},
        ProjectListItem::WorkspaceWorktrees(wtg) => {
            observe_workspace_worktree_group_fit_widths(widths, wtg, state);
        },
        ProjectListItem::PackageWorktrees(wtg) => {
            observe_package_worktree_group_fit_widths(widths, wtg, state);
        },
    }
}

fn observe_new_member_group_fit_widths(
    widths: &mut ResolvedWidths,
    groups: &[MemberGroup],
    state: &FitWidthsState<'_>,
    is_worktree: bool,
) {
    let dw = columns::display_width;
    for group in groups {
        let (inline_prefix, named_prefix, group_prefix) = if is_worktree {
            (
                PREFIX_WT_MEMBER_INLINE,
                PREFIX_WT_MEMBER_NAMED,
                PREFIX_WT_GROUP_COLLAPSED,
            )
        } else {
            (
                PREFIX_MEMBER_INLINE,
                PREFIX_MEMBER_NAMED,
                PREFIX_GROUP_COLLAPSED,
            )
        };
        for member in group.members() {
            let prefix = if group.is_named() {
                named_prefix
            } else {
                inline_prefix
            };
            let member_path = member.display_path();
            App::observe_name_width(widths, dw(prefix) + dw(&member.display_name()));
            widths.observe(
                COL_DISK,
                dw(&formatted_disk_snapshot(state.disk_usage, &member_path)),
            );
            widths.observe(
                COL_SYNC,
                dw(&git_sync_snapshot(
                    state.git_info,
                    state.git_path_states,
                    &member_path,
                )),
            );
        }
        if group.is_named() {
            let label = format!("{} ({})", group.group_name(), group.members().len());
            App::observe_name_width(widths, dw(group_prefix) + dw(&label));
        }
    }
}

fn observe_typed_vendored_fit_widths(
    widths: &mut ResolvedWidths,
    vendored: &[Project<Package>],
    disk_usage: &HashMap<String, u64>,
    prefix: &str,
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = format!("{} (vendored)", project.display_name());
        let path = project.display_path();
        App::observe_name_width(widths, dw(prefix) + dw(&label));
        widths.observe(COL_DISK, dw(&formatted_disk_snapshot(disk_usage, &path)));
    }
}

fn observe_workspace_worktree_entry_fit_widths(
    widths: &mut ResolvedWidths,
    ws: &Project<Workspace>,
    state: &FitWidthsState<'_>,
) {
    let dw = columns::display_width;
    let wt_name = ws
        .worktree_name()
        .unwrap_or_else(|| ws.path().to_str().unwrap_or(""));
    let prefix = if ws.has_members() {
        PREFIX_WT_COLLAPSED
    } else {
        PREFIX_WT_FLAT
    };
    let wt_path = ws.display_path();
    App::observe_name_width(widths, dw(prefix) + dw(wt_name));
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_snapshot(state.disk_usage, &wt_path)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            state.git_info,
            state.git_path_states,
            &wt_path,
        )),
    );
    observe_new_member_group_fit_widths(widths, ws.groups(), state, true);
    observe_typed_vendored_fit_widths(widths, ws.vendored(), state.disk_usage, PREFIX_WT_VENDORED);
}

fn observe_package_worktree_entry_fit_widths(
    widths: &mut ResolvedWidths,
    pkg: &Project<Package>,
    state: &FitWidthsState<'_>,
) {
    let dw = columns::display_width;
    let wt_name = pkg
        .worktree_name()
        .unwrap_or_else(|| pkg.path().to_str().unwrap_or(""));
    let wt_path = pkg.display_path();
    App::observe_name_width(widths, dw(PREFIX_WT_FLAT) + dw(wt_name));
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_snapshot(state.disk_usage, &wt_path)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            state.git_info,
            state.git_path_states,
            &wt_path,
        )),
    );
    observe_typed_vendored_fit_widths(widths, pkg.vendored(), state.disk_usage, PREFIX_WT_VENDORED);
}

fn observe_workspace_worktree_group_fit_widths(
    widths: &mut ResolvedWidths,
    wtg: &WorktreeGroup<Workspace>,
    state: &FitWidthsState<'_>,
) {
    observe_workspace_worktree_entry_fit_widths(widths, wtg.primary(), state);
    for linked in wtg.linked() {
        observe_workspace_worktree_entry_fit_widths(widths, linked, state);
    }
}

fn observe_package_worktree_group_fit_widths(
    widths: &mut ResolvedWidths,
    wtg: &WorktreeGroup<Package>,
    state: &FitWidthsState<'_>,
) {
    observe_package_worktree_entry_fit_widths(widths, wtg.primary(), state);
    for linked in wtg.linked() {
        observe_package_worktree_entry_fit_widths(widths, linked, state);
    }
}

pub(super) fn build_disk_cache_snapshot(
    items: &[ProjectListItem],
    disk_usage: &HashMap<String, u64>,
) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut root_sorted = Vec::new();
    for item in items {
        if let Some(bytes) = disk_bytes_for_item_snapshot(item, disk_usage) {
            root_sorted.push(bytes);
        }
    }
    root_sorted.sort_unstable();

    let mut child_sorted = HashMap::new();
    for (ni, item) in items.iter().enumerate() {
        let mut values = Vec::new();
        collect_child_disk_values(item, disk_usage, &mut values);
        if !values.is_empty() {
            values.sort_unstable();
            child_sorted.insert(ni, values);
        }
    }

    (root_sorted, child_sorted)
}

/// Collect disk bytes for all children (members, vendored, worktree entries) of an item.
fn collect_child_disk_values(
    item: &ProjectListItem,
    disk_usage: &HashMap<String, u64>,
    values: &mut Vec<u64>,
) {
    match item {
        ProjectListItem::Workspace(ws) => {
            collect_member_group_disk(ws.groups(), disk_usage, values);
            collect_vendored_disk(ws.vendored(), disk_usage, values);
        },
        ProjectListItem::Package(pkg) => {
            collect_vendored_disk(pkg.vendored(), disk_usage, values);
        },
        ProjectListItem::NonRust(_) => {},
        ProjectListItem::WorkspaceWorktrees(wtg) => {
            for ws in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                let dp = ws.display_path();
                if let Some(&bytes) = disk_usage.get(&dp) {
                    values.push(bytes);
                }
                collect_member_group_disk(ws.groups(), disk_usage, values);
                collect_vendored_disk(ws.vendored(), disk_usage, values);
            }
        },
        ProjectListItem::PackageWorktrees(wtg) => {
            for pkg in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                let dp = pkg.display_path();
                if let Some(&bytes) = disk_usage.get(&dp) {
                    values.push(bytes);
                }
                collect_vendored_disk(pkg.vendored(), disk_usage, values);
            }
        },
    }
}

fn collect_member_group_disk(
    groups: &[MemberGroup],
    disk_usage: &HashMap<String, u64>,
    values: &mut Vec<u64>,
) {
    for group in groups {
        for member in group.members() {
            let dp = member.display_path();
            if let Some(&bytes) = disk_usage.get(&dp) {
                values.push(bytes);
            }
        }
    }
}

fn collect_vendored_disk(
    vendored: &[Project<Package>],
    disk_usage: &HashMap<String, u64>,
    values: &mut Vec<u64>,
) {
    for project in vendored {
        let dp = project.display_path();
        if let Some(&bytes) = disk_usage.get(&dp) {
            values.push(bytes);
        }
    }
}

pub(super) fn initial_disk_batch_count(projects: &[LegacyProject]) -> usize {
    let mut abs_paths: Vec<&str> = projects
        .iter()
        .map(|project| project.abs_path.as_str())
        .collect();
    abs_paths.sort_by(|left, right| {
        Path::new(left)
            .components()
            .count()
            .cmp(&Path::new(right).components().count())
            .then_with(|| left.cmp(right))
    });

    let mut roots: Vec<&str> = Vec::new();
    for abs_path in abs_paths {
        if roots
            .iter()
            .any(|root| Path::new(abs_path).starts_with(Path::new(root)))
        {
            continue;
        }
        roots.push(abs_path);
    }

    roots.len()
}
