use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use super::types::App;
use super::types::ExpandKey;
use super::types::VisibleRow;
use crate::constants::IN_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
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
    items: &[RootItem],
    expanded: &HashSet<ExpandKey>,
    include_non_rust: bool,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (ni, item) in items.iter().enumerate() {
        if matches!(item.visibility(), Visibility::Dismissed) {
            continue;
        }
        if !include_non_rust && !item.is_rust() {
            continue;
        }
        rows.push(VisibleRow::Root { node_index: ni });
        if !expanded.contains(&ExpandKey::Node(ni)) {
            continue;
        }

        match item {
            RootItem::Workspace(ws) => {
                emit_groups(&mut rows, ni, ws.groups(), expanded);
                emit_vendored_rows(&mut rows, ni, ws.vendored());
            },
            RootItem::Package(pkg) => {
                emit_vendored_rows(&mut rows, ni, pkg.vendored());
            },
            RootItem::NonRust(_) => {},
            RootItem::WorkspaceWorktrees(wtg) => {
                if wtg.renders_as_group() {
                    emit_workspace_worktree_group(&mut rows, ni, wtg, expanded);
                } else if let Some(workspace) = wtg.single_live() {
                    emit_groups(&mut rows, ni, workspace.groups(), expanded);
                    emit_vendored_rows(&mut rows, ni, workspace.vendored());
                }
            },
            RootItem::PackageWorktrees(wtg) => {
                if wtg.renders_as_group() {
                    emit_package_worktree_group(&mut rows, ni, wtg, expanded);
                } else if let Some(package) = wtg.single_live() {
                    emit_vendored_rows(&mut rows, ni, package.vendored());
                }
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

fn emit_vendored_rows(rows: &mut Vec<VisibleRow>, ni: usize, vendored: &[RustProject<Package>]) {
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
    vendored: &[RustProject<Package>],
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

fn formatted_disk(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| render::format_bytes(0), render::format_bytes)
}

pub(super) fn git_sync_snapshot(
    git_info: Option<&GitInfo>,
    git_path_states: &HashMap<PathBuf, GitPathState>,
    path: &Path,
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
    let Some(info) = git_info else {
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
    pub git_path_states: &'a HashMap<PathBuf, GitPathState>,
}

pub(super) fn build_fit_widths_snapshot(
    items: &[RootItem],
    root_labels: &[String],
    state: &FitWidthsState<'_>,
    lint_enabled: bool,
    generation: u64,
) -> ResolvedWidths {
    let mut widths = ResolvedWidths::new(lint_enabled);

    for (index, item) in items.iter().enumerate() {
        observe_item_fit_widths(&mut widths, item, &root_labels[index], state);
    }

    widths.generation = generation;
    widths
}

fn observe_item_fit_widths(
    widths: &mut ResolvedWidths,
    item: &RootItem,
    root_label: &str,
    state: &FitWidthsState<'_>,
) {
    let dw = columns::display_width;
    let root_path = item.path();

    App::observe_name_width(widths, dw(render::PREFIX_ROOT_COLLAPSED) + dw(root_label));
    widths.observe(COL_DISK, dw(&formatted_disk(item.disk_usage_bytes())));
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            item.git_info(),
            state.git_path_states,
            root_path,
        )),
    );

    match item {
        RootItem::Workspace(ws) => {
            observe_new_member_group_fit_widths(widths, ws.groups(), state, false);
            observe_typed_vendored_fit_widths(widths, ws.vendored(), PREFIX_VENDORED);
        },
        RootItem::Package(pkg) => {
            observe_typed_vendored_fit_widths(widths, pkg.vendored(), PREFIX_VENDORED);
        },
        RootItem::NonRust(_) => {},
        RootItem::WorkspaceWorktrees(wtg) => {
            observe_workspace_worktree_group_fit_widths(widths, wtg, state);
        },
        RootItem::PackageWorktrees(wtg) => {
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
            App::observe_name_width(widths, dw(prefix) + dw(&member.display_name()));
            widths.observe(COL_DISK, dw(&formatted_disk(member.disk_usage_bytes())));
            widths.observe(
                COL_SYNC,
                dw(&git_sync_snapshot(
                    member.git_info(),
                    state.git_path_states,
                    member.path(),
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
    vendored: &[RustProject<Package>],
    prefix: &str,
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = format!("{} (vendored)", project.display_name());
        App::observe_name_width(widths, dw(prefix) + dw(&label));
        widths.observe(COL_DISK, dw(&formatted_disk(project.disk_usage_bytes())));
    }
}

fn observe_workspace_worktree_entry_fit_widths(
    widths: &mut ResolvedWidths,
    ws: &RustProject<Workspace>,
    state: &FitWidthsState<'_>,
) {
    let dw = columns::display_width;
    let wt_name = ws
        .worktree_name()
        .map_or_else(|| ws.display_name(), String::from);
    let prefix = if ws.has_members() {
        PREFIX_WT_COLLAPSED
    } else {
        PREFIX_WT_FLAT
    };
    App::observe_name_width(widths, dw(prefix) + dw(&wt_name));
    widths.observe(COL_DISK, dw(&formatted_disk(ws.disk_usage_bytes())));
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            ws.git_info(),
            state.git_path_states,
            ws.path(),
        )),
    );
    observe_new_member_group_fit_widths(widths, ws.groups(), state, true);
    observe_typed_vendored_fit_widths(widths, ws.vendored(), PREFIX_WT_VENDORED);
}

fn observe_package_worktree_entry_fit_widths(
    widths: &mut ResolvedWidths,
    pkg: &RustProject<Package>,
    state: &FitWidthsState<'_>,
) {
    let dw = columns::display_width;
    let wt_name = pkg
        .worktree_name()
        .map_or_else(|| pkg.display_name(), String::from);
    App::observe_name_width(widths, dw(PREFIX_WT_FLAT) + dw(&wt_name));
    widths.observe(COL_DISK, dw(&formatted_disk(pkg.disk_usage_bytes())));
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            pkg.git_info(),
            state.git_path_states,
            pkg.path(),
        )),
    );
    observe_typed_vendored_fit_widths(widths, pkg.vendored(), PREFIX_WT_VENDORED);
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
    items: &[RootItem],
) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut root_sorted = Vec::new();
    for item in items {
        if let Some(bytes) = item.disk_usage_bytes() {
            root_sorted.push(bytes);
        }
    }
    root_sorted.sort_unstable();

    let mut child_sorted = HashMap::new();
    for (ni, item) in items.iter().enumerate() {
        let mut values = Vec::new();
        collect_child_disk_values(item, &mut values);
        if !values.is_empty() {
            values.sort_unstable();
            child_sorted.insert(ni, values);
        }
    }

    (root_sorted, child_sorted)
}

/// Collect disk bytes for all children (members, vendored, worktree entries) of an item.
fn collect_child_disk_values(item: &RootItem, values: &mut Vec<u64>) {
    match item {
        RootItem::Workspace(ws) => {
            collect_member_group_disk(ws.groups(), values);
            collect_vendored_disk(ws.vendored(), values);
        },
        RootItem::Package(pkg) => {
            collect_vendored_disk(pkg.vendored(), values);
        },
        RootItem::NonRust(_) => {},
        RootItem::WorkspaceWorktrees(wtg) => {
            for ws in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                if let Some(bytes) = ws.disk_usage_bytes() {
                    values.push(bytes);
                }
                collect_member_group_disk(ws.groups(), values);
                collect_vendored_disk(ws.vendored(), values);
            }
        },
        RootItem::PackageWorktrees(wtg) => {
            for pkg in std::iter::once(wtg.primary()).chain(wtg.linked().iter()) {
                if let Some(bytes) = pkg.disk_usage_bytes() {
                    values.push(bytes);
                }
                collect_vendored_disk(pkg.vendored(), values);
            }
        },
    }
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

fn collect_vendored_disk(vendored: &[RustProject<Package>], values: &mut Vec<u64>) {
    for project in vendored {
        if let Some(bytes) = project.disk_usage_bytes() {
            values.push(bytes);
        }
    }
}

pub(super) fn initial_disk_batch_count(projects: &[RootItem]) -> usize {
    let mut abs_paths: Vec<&Path> = projects.iter().map(RootItem::path).collect();
    abs_paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut roots: Vec<&Path> = Vec::new();
    for abs_path in abs_paths {
        if roots.iter().any(|root| abs_path.starts_with(root)) {
            continue;
        }
        roots.push(abs_path);
    }

    roots.len()
}
