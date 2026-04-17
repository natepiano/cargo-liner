use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use super::App;
use super::types::ExpandKey;
use super::types::VisibleRow;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::AbsolutePath;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitStatus;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::tui::columns;
use crate::tui::columns::COL_DISK;
use crate::tui::columns::COL_MAIN;
use crate::tui::columns::COL_SYNC;
use crate::tui::columns::ResolvedWidths;
use crate::tui::render;
use crate::tui::render::PREFIX_GROUP_COLLAPSED;
use crate::tui::render::PREFIX_MEMBER_INLINE;
use crate::tui::render::PREFIX_MEMBER_NAMED;
use crate::tui::render::PREFIX_SUBMODULE;
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
            RootItem::Rust(RustProject::Workspace(ws)) => {
                emit_groups(&mut rows, ni, ws.groups(), expanded);
                emit_vendored_rows(&mut rows, ni, ws.vendored());
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                emit_vendored_rows(&mut rows, ni, pkg.vendored());
            },
            RootItem::NonRust(_) => {},
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. }) => {
                if wtg.renders_as_group() {
                    emit_workspace_worktree_group(&mut rows, ni, wtg, expanded);
                } else if let Some(workspace) = wtg.single_live_workspace() {
                    emit_groups(&mut rows, ni, workspace.groups(), expanded);
                    emit_vendored_rows(&mut rows, ni, workspace.vendored());
                }
            },
            RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. }) => {
                if wtg.renders_as_group() {
                    emit_package_worktree_group(&mut rows, ni, wtg, expanded);
                } else if let Some(package) = wtg.single_live_package() {
                    emit_vendored_rows(&mut rows, ni, package.vendored());
                }
            },
        }
        emit_submodule_rows(&mut rows, ni, item.submodules());
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

fn emit_vendored_rows(rows: &mut Vec<VisibleRow>, ni: usize, vendored: &[Package]) {
    for (vi, _) in vendored.iter().enumerate() {
        rows.push(VisibleRow::Vendored {
            node_index:     ni,
            vendored_index: vi,
        });
    }
}

fn emit_submodule_rows(rows: &mut Vec<VisibleRow>, ni: usize, submodules: &[Submodule]) {
    for (si, _) in submodules.iter().enumerate() {
        rows.push(VisibleRow::Submodule {
            node_index:      ni,
            submodule_index: si,
        });
    }
}

/// Emit worktree entries for a `WorktreeGroup::Workspaces`.
fn emit_workspace_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup,
    expanded: &HashSet<ExpandKey>,
) {
    let WorktreeGroup::Workspaces {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    // Primary at index 0
    if !matches!(primary.visibility(), Visibility::Dismissed) {
        let wi = 0;
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if primary.has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            emit_worktree_children(rows, ni, wi, primary.groups(), primary.vendored(), expanded);
        }
    }
    // Linked at indices 1..
    for (i, ws) in linked.iter().enumerate() {
        if matches!(ws.visibility(), Visibility::Dismissed) {
            continue;
        }
        let wi = i + 1;
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: wi,
        });
        if ws.has_members() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
            emit_worktree_children(rows, ni, wi, ws.groups(), ws.vendored(), expanded);
        }
    }
}

/// Emit worktree entries for a `WorktreeGroup::Packages`.
fn emit_package_worktree_group(
    rows: &mut Vec<VisibleRow>,
    ni: usize,
    wtg: &WorktreeGroup,
    _expanded: &HashSet<ExpandKey>,
) {
    let WorktreeGroup::Packages {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    // Primary at index 0
    if !matches!(primary.visibility(), Visibility::Dismissed) {
        rows.push(VisibleRow::WorktreeEntry {
            node_index:     ni,
            worktree_index: 0,
        });
    }
    // Linked at indices 1..
    for (i, pkg) in linked.iter().enumerate() {
        if matches!(pkg.visibility(), Visibility::Dismissed) {
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
    vendored: &[Package],
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

pub(super) fn git_sync_snapshot(git_info: Option<&GitInfo>) -> String {
    let Some(info) = git_info else {
        return String::new();
    };
    if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
        return String::new();
    }
    match info.ahead_behind {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None if info.origin != GitOrigin::Local => "-".to_string(),
        None => NO_REMOTE_SYNC.to_string(),
    }
}

pub(super) fn git_main_snapshot(git_info: Option<&GitInfo>) -> String {
    let Some(info) = git_info else {
        return String::new();
    };
    if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
        return String::new();
    }
    match info.ahead_behind_local {
        Some((0, 0)) => IN_SYNC.to_string(),
        Some((a, 0)) => format!("{SYNC_UP}{a}"),
        Some((0, b)) => format!("{SYNC_DOWN}{b}"),
        Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
        None => String::new(),
    }
}

pub(super) fn build_fit_widths_snapshot(
    items: &[RootItem],
    root_labels: &[String],
    lint_enabled: bool,
    generation: u64,
) -> ResolvedWidths {
    let mut widths = ResolvedWidths::new(lint_enabled);

    for (index, item) in items.iter().enumerate() {
        observe_item_fit_widths(&mut widths, item, &root_labels[index]);
    }

    widths.generation = generation;
    widths
}

fn observe_item_fit_widths(widths: &mut ResolvedWidths, item: &RootItem, root_label: &str) {
    let dw = columns::display_width;

    App::observe_name_width(widths, dw(render::PREFIX_ROOT_COLLAPSED) + dw(root_label));
    widths.observe(COL_DISK, dw(&formatted_disk(item.disk_usage_bytes())));
    widths.observe(COL_SYNC, dw(&git_sync_snapshot(item.git_info())));
    widths.observe(COL_MAIN, dw(&git_main_snapshot(item.git_info())));

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            observe_new_member_group_fit_widths(widths, ws.groups(), false);
            observe_typed_vendored_fit_widths(widths, ws.vendored(), PREFIX_VENDORED);
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            observe_typed_vendored_fit_widths(widths, pkg.vendored(), PREFIX_VENDORED);
        },
        RootItem::NonRust(_) => {},
        RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. }) => {
            observe_workspace_worktree_group_fit_widths(widths, wtg);
        },
        RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. }) => {
            observe_package_worktree_group_fit_widths(widths, wtg);
        },
    }
    for submodule in item.submodules() {
        let label = format!("{} (s)", submodule.name);
        App::observe_name_width(widths, dw(PREFIX_SUBMODULE) + dw(&label));
        widths.observe(
            COL_DISK,
            dw(&formatted_disk(submodule.info.disk_usage_bytes)),
        );
    }
}

fn observe_new_member_group_fit_widths(
    widths: &mut ResolvedWidths,
    groups: &[MemberGroup],
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
            App::observe_name_width(widths, dw(prefix) + dw(member.package_name().as_str()));
            widths.observe(COL_DISK, dw(&formatted_disk(member.disk_usage_bytes())));
        }
        if group.is_named() {
            let label = format!("{} ({})", group.group_name(), group.members().len());
            App::observe_name_width(widths, dw(group_prefix) + dw(&label));
        }
    }
}

fn observe_typed_vendored_fit_widths(
    widths: &mut ResolvedWidths,
    vendored: &[Package],
    prefix: &str,
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = format!("{} (vendored)", project.package_name());
        App::observe_name_width(widths, dw(prefix) + dw(&label));
        widths.observe(COL_DISK, dw(&formatted_disk(project.disk_usage_bytes())));
    }
}

fn observe_workspace_worktree_entry_fit_widths(widths: &mut ResolvedWidths, ws: &Workspace) {
    let dw = columns::display_width;
    let wt_name = ws
        .worktree_name()
        .map_or_else(|| ws.root_directory_name().into_string(), String::from);
    let prefix = if ws.has_members() {
        PREFIX_WT_COLLAPSED
    } else {
        PREFIX_WT_FLAT
    };
    App::observe_name_width(widths, dw(prefix) + dw(&wt_name));
    widths.observe(COL_DISK, dw(&formatted_disk(ws.disk_usage_bytes())));
    widths.observe(COL_SYNC, dw(&git_sync_snapshot(ws.git_info())));
    widths.observe(COL_MAIN, dw(&git_main_snapshot(ws.git_info())));
    observe_new_member_group_fit_widths(widths, ws.groups(), true);
    observe_typed_vendored_fit_widths(widths, ws.vendored(), PREFIX_WT_VENDORED);
}

fn observe_package_worktree_entry_fit_widths(widths: &mut ResolvedWidths, pkg: &Package) {
    let dw = columns::display_width;
    let wt_name = pkg
        .worktree_name()
        .map_or_else(|| pkg.root_directory_name().into_string(), String::from);
    App::observe_name_width(widths, dw(PREFIX_WT_FLAT) + dw(&wt_name));
    widths.observe(COL_DISK, dw(&formatted_disk(pkg.disk_usage_bytes())));
    widths.observe(COL_SYNC, dw(&git_sync_snapshot(pkg.git_info())));
    widths.observe(COL_MAIN, dw(&git_main_snapshot(pkg.git_info())));
    observe_typed_vendored_fit_widths(widths, pkg.vendored(), PREFIX_WT_VENDORED);
}

fn observe_workspace_worktree_group_fit_widths(widths: &mut ResolvedWidths, wtg: &WorktreeGroup) {
    let WorktreeGroup::Workspaces {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    observe_workspace_worktree_entry_fit_widths(widths, primary);
    for ws in linked {
        observe_workspace_worktree_entry_fit_widths(widths, ws);
    }
}

fn observe_package_worktree_group_fit_widths(widths: &mut ResolvedWidths, wtg: &WorktreeGroup) {
    let WorktreeGroup::Packages {
        primary, linked, ..
    } = wtg
    else {
        return;
    };
    observe_package_worktree_entry_fit_widths(widths, primary);
    for pkg in linked {
        observe_package_worktree_entry_fit_widths(widths, pkg);
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

/// Collect disk bytes for all children (members, vendored, worktree entries,
/// submodules) of an item.
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
    collect_submodule_disk(item.submodules(), values);
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

fn collect_vendored_disk(vendored: &[Package], values: &mut Vec<u64>) {
    for project in vendored {
        if let Some(bytes) = project.disk_usage_bytes() {
            values.push(bytes);
        }
    }
}

fn collect_submodule_disk(submodules: &[Submodule], values: &mut Vec<u64>) {
    for submodule in submodules {
        if let Some(bytes) = submodule.info.disk_usage_bytes {
            values.push(bytes);
        }
    }
}

pub(super) fn initial_disk_batch_count(projects: &[RootItem]) -> usize {
    let mut abs_paths: Vec<&AbsolutePath> = projects.iter().map(RootItem::path).collect();
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
