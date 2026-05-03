use std::collections::HashMap;
use std::collections::HashSet;

use super::types::ExpandKey;
use super::types::VisibleRow;
use crate::project::AbsolutePath;
use crate::project::MemberGroup;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::WorktreeGroup;
use crate::project_list::ProjectList;
use crate::tui::columns::ProjectListWidths;
use crate::tui::panes;

/// Build the flat list of visible rows from the project list and expansion state.
pub fn build_visible_rows(
    entries: &ProjectList,
    expanded: &HashSet<ExpandKey>,
    include_non_rust: bool,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (ni, entry) in entries.iter().enumerate() {
        let item = &entry.item;
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

fn emit_vendored_rows(rows: &mut Vec<VisibleRow>, ni: usize, vendored: &[VendoredPackage]) {
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
    vendored: &[VendoredPackage],
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

/// Build the column-fit widths snapshot. The math lives in
/// `panes::widths` next to the renderer it mirrors; this is the
/// thin App-shell entry that Selection's fit-widths cache calls.
pub(super) fn build_fit_widths_snapshot(
    entries: &ProjectList,
    root_labels: &[String],
    lint_enabled: bool,
    generation: u64,
) -> ProjectListWidths {
    panes::compute_project_list_widths(entries, root_labels, lint_enabled, generation)
}

pub(super) fn build_disk_cache_snapshot(
    entries: &ProjectList,
) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
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

/// Collect workspace roots that should receive an initial `cargo metadata`
/// dispatch: every Rust leaf project (workspace or standalone package),
/// including each worktree in a group. Non-Rust projects are skipped.
pub(super) fn initial_metadata_roots(projects: &ProjectList) -> HashSet<AbsolutePath> {
    let mut roots = HashSet::new();
    projects.for_each_leaf(|entry| {
        if let RootItem::Rust(rust) = &entry.item {
            roots.insert(rust.path().clone());
        }
    });
    roots
}

pub(super) fn initial_disk_roots(projects: &ProjectList) -> HashSet<AbsolutePath> {
    let mut abs_paths: Vec<&AbsolutePath> =
        projects.iter().map(|entry| entry.item.path()).collect();
    abs_paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut roots: Vec<&AbsolutePath> = Vec::new();
    for abs_path in abs_paths {
        if roots.iter().any(|root| abs_path.starts_with(root)) {
            continue;
        }
        roots.push(abs_path);
    }

    roots.into_iter().cloned().collect()
}
