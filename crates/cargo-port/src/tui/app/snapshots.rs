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
use crate::project::RustProject;
use crate::scan::MemberGroup;
use crate::scan::ProjectNode;
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

/// Build the flat list of visible rows from the node tree and expansion state.
pub(super) fn build_visible_rows(
    nodes: &[ProjectNode],
    expanded: &HashSet<ExpandKey>,
    dismissed: &HashSet<String>,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for (ni, node) in nodes.iter().enumerate() {
        if dismissed.contains(&node.project.path) {
            continue;
        }
        rows.push(VisibleRow::Root { node_index: ni });
        if expanded.contains(&ExpandKey::Node(ni)) {
            for (gi, group) in node.groups.iter().enumerate() {
                if group.name.is_empty() {
                    for (mi, _) in group.members.iter().enumerate() {
                        rows.push(VisibleRow::Member {
                            node_index:   ni,
                            group_index:  gi,
                            member_index: mi,
                        });
                    }
                } else {
                    rows.push(VisibleRow::GroupHeader {
                        node_index:  ni,
                        group_index: gi,
                    });
                    if expanded.contains(&ExpandKey::Group(ni, gi)) {
                        for (mi, _) in group.members.iter().enumerate() {
                            rows.push(VisibleRow::Member {
                                node_index:   ni,
                                group_index:  gi,
                                member_index: mi,
                            });
                        }
                    }
                }
            }

            for (vi, _) in node.vendored.iter().enumerate() {
                rows.push(VisibleRow::Vendored {
                    node_index:     ni,
                    vendored_index: vi,
                });
            }

            for (wi, wt) in node.worktrees.iter().enumerate() {
                rows.push(VisibleRow::WorktreeEntry {
                    node_index:     ni,
                    worktree_index: wi,
                });
                if wt.has_children() && expanded.contains(&ExpandKey::Worktree(ni, wi)) {
                    for (gi, group) in wt.groups.iter().enumerate() {
                        if group.name.is_empty() {
                            for (mi, _) in group.members.iter().enumerate() {
                                rows.push(VisibleRow::WorktreeMember {
                                    node_index:     ni,
                                    worktree_index: wi,
                                    group_index:    gi,
                                    member_index:   mi,
                                });
                            }
                        } else {
                            rows.push(VisibleRow::WorktreeGroupHeader {
                                node_index:     ni,
                                worktree_index: wi,
                                group_index:    gi,
                            });
                            if expanded.contains(&ExpandKey::WorktreeGroup(ni, wi, gi)) {
                                for (mi, _) in group.members.iter().enumerate() {
                                    rows.push(VisibleRow::WorktreeMember {
                                        node_index:     ni,
                                        worktree_index: wi,
                                        group_index:    gi,
                                        member_index:   mi,
                                    });
                                }
                            }
                        }
                    }

                    for (vi, _) in wt.vendored.iter().enumerate() {
                        rows.push(VisibleRow::WorktreeVendored {
                            node_index:     ni,
                            worktree_index: wi,
                            vendored_index: vi,
                        });
                    }
                }
            }
        }
    }
    rows
}

pub(super) fn live_worktree_count_for_node(
    node: &ProjectNode,
    deleted_projects: &HashSet<String>,
) -> usize {
    node.worktrees
        .iter()
        .filter(|wt| !deleted_projects.contains(&wt.project.path))
        .count()
}

pub(super) fn unique_node_paths(node: &ProjectNode) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    for path in std::iter::once(node.project.path.as_str())
        .chain(node.worktrees.iter().map(|wt| wt.project.path.as_str()))
    {
        if seen.insert(path) {
            paths.push(path);
        }
    }

    paths
}

pub(super) fn disk_bytes_for_node_snapshot(
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
) -> Option<u64> {
    if node.worktrees.is_empty() {
        return disk_usage.get(&node.project.path).copied();
    }
    let mut total = 0;
    let mut any_data = false;
    for path in unique_node_paths(node) {
        if let Some(&bytes) = disk_usage.get(path) {
            total += bytes;
            any_data = true;
        }
    }
    if any_data { Some(total) } else { None }
}

pub(super) fn formatted_disk_snapshot(disk_usage: &HashMap<String, u64>, path: &str) -> String {
    disk_usage
        .get(path)
        .copied()
        .map_or_else(|| render::format_bytes(0), render::format_bytes)
}

pub(super) fn formatted_disk_for_node_snapshot(
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
) -> String {
    disk_bytes_for_node_snapshot(node, disk_usage)
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

pub(super) fn build_fit_widths_snapshot(
    nodes: &[ProjectNode],
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
    deleted_projects: &HashSet<String>,
    lint_enabled: bool,
    generation: u64,
) -> ResolvedWidths {
    let mut widths = ResolvedWidths::new(lint_enabled);

    for node in nodes {
        observe_node_fit_widths(
            &mut widths,
            node,
            disk_usage,
            git_info,
            git_path_states,
            deleted_projects,
        );
    }

    widths.generation = generation;
    widths
}

pub(super) fn observe_node_fit_widths(
    widths: &mut ResolvedWidths,
    node: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
    deleted_projects: &HashSet<String>,
) {
    let dw = columns::display_width;
    App::observe_name_width(
        widths,
        App::fit_name_for_node(node, live_worktree_count_for_node(node, deleted_projects)),
    );
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_for_node_snapshot(node, disk_usage)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            git_info,
            git_path_states,
            &node.project.path,
        )),
    );

    observe_member_group_fit_widths(widths, &node.groups, disk_usage, git_info, git_path_states);
    observe_vendored_fit_widths(widths, &node.vendored, disk_usage, PREFIX_VENDORED);
    for worktree in &node.worktrees {
        observe_worktree_fit_widths(widths, worktree, disk_usage, git_info, git_path_states);
    }
}

pub(super) fn observe_member_group_fit_widths(
    widths: &mut ResolvedWidths,
    groups: &[MemberGroup],
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
) {
    let dw = columns::display_width;
    for group in groups {
        for member in &group.members {
            let prefix = if group.name.is_empty() {
                PREFIX_MEMBER_INLINE
            } else {
                PREFIX_MEMBER_NAMED
            };
            App::observe_name_width(widths, dw(prefix) + dw(&member.display_name()));
            widths.observe(
                COL_DISK,
                dw(&formatted_disk_snapshot(disk_usage, &member.path)),
            );
            widths.observe(
                COL_SYNC,
                dw(&git_sync_snapshot(git_info, git_path_states, &member.path)),
            );
        }
        if !group.name.is_empty() {
            let label = format!("{} ({})", group.name, group.members.len());
            App::observe_name_width(widths, dw(PREFIX_GROUP_COLLAPSED) + dw(&label));
        }
    }
}

pub(super) fn observe_vendored_fit_widths(
    widths: &mut ResolvedWidths,
    vendored: &[RustProject],
    disk_usage: &HashMap<String, u64>,
    prefix: &str,
) {
    let dw = columns::display_width;
    for project in vendored {
        let label = format!("{} (vendored)", project.display_name());
        App::observe_name_width(widths, dw(prefix) + dw(&label));
        widths.observe(
            COL_DISK,
            dw(&formatted_disk_snapshot(disk_usage, &project.path)),
        );
    }
}

pub(super) fn observe_worktree_fit_widths(
    widths: &mut ResolvedWidths,
    worktree: &ProjectNode,
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
) {
    let dw = columns::display_width;
    let worktree_name = worktree
        .project
        .worktree_name
        .as_deref()
        .unwrap_or(&worktree.project.path);
    let worktree_prefix = if worktree.has_children() {
        PREFIX_WT_COLLAPSED
    } else {
        PREFIX_WT_FLAT
    };
    App::observe_name_width(widths, dw(worktree_prefix) + dw(worktree_name));
    widths.observe(
        COL_DISK,
        dw(&formatted_disk_snapshot(disk_usage, &worktree.project.path)),
    );
    widths.observe(
        COL_SYNC,
        dw(&git_sync_snapshot(
            git_info,
            git_path_states,
            &worktree.project.path,
        )),
    );
    observe_worktree_group_fit_widths(
        widths,
        &worktree.groups,
        disk_usage,
        git_info,
        git_path_states,
    );
    observe_vendored_fit_widths(widths, &worktree.vendored, disk_usage, PREFIX_WT_VENDORED);
}

pub(super) fn observe_worktree_group_fit_widths(
    widths: &mut ResolvedWidths,
    groups: &[MemberGroup],
    disk_usage: &HashMap<String, u64>,
    git_info: &HashMap<String, GitInfo>,
    git_path_states: &HashMap<String, GitPathState>,
) {
    let dw = columns::display_width;
    for group in groups {
        for member in &group.members {
            let prefix = if group.name.is_empty() {
                PREFIX_WT_MEMBER_INLINE
            } else {
                PREFIX_WT_MEMBER_NAMED
            };
            App::observe_name_width(widths, dw(prefix) + dw(&member.display_name()));
            widths.observe(
                COL_DISK,
                dw(&formatted_disk_snapshot(disk_usage, &member.path)),
            );
            widths.observe(
                COL_SYNC,
                dw(&git_sync_snapshot(git_info, git_path_states, &member.path)),
            );
        }
        if !group.name.is_empty() {
            let label = format!("{} ({})", group.name, group.members.len());
            App::observe_name_width(widths, dw(PREFIX_WT_GROUP_COLLAPSED) + dw(&label));
        }
    }
}

pub(super) fn build_disk_cache_snapshot(
    nodes: &[ProjectNode],
    disk_usage: &HashMap<String, u64>,
) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    let mut root_sorted = Vec::new();
    for node in nodes {
        if let Some(bytes) = disk_bytes_for_node_snapshot(node, disk_usage) {
            root_sorted.push(bytes);
        }
    }
    root_sorted.sort_unstable();

    let mut child_sorted = HashMap::new();
    for (ni, node) in nodes.iter().enumerate() {
        let mut values = Vec::new();
        for member in App::all_group_members(node) {
            if let Some(&bytes) = disk_usage.get(&member.path) {
                values.push(bytes);
            }
        }
        for vendored in App::all_vendored_projects(node) {
            if let Some(&bytes) = disk_usage.get(&vendored.path) {
                values.push(bytes);
            }
        }
        for wt in &node.worktrees {
            if let Some(&bytes) = disk_usage.get(&wt.project.path) {
                values.push(bytes);
            }
        }
        if !values.is_empty() {
            values.sort_unstable();
            child_sorted.insert(ni, values);
        }
    }

    (root_sorted, child_sorted)
}

pub(super) fn replace_project_in_node(
    node: &mut ProjectNode,
    project_path: &str,
    project: &RustProject,
) -> bool {
    let updated = if node.project.path == project_path {
        node.project = project.clone();
        true
    } else {
        false
    };

    let mut updated = updated;

    for group in &mut node.groups {
        for member in &mut group.members {
            if member.path == project_path {
                *member = project.clone();
                updated = true;
            }
        }
    }

    for vendored in &mut node.vendored {
        if vendored.path == project_path {
            *vendored = project.clone();
            updated = true;
        }
    }

    for worktree in &mut node.worktrees {
        if worktree.project.path == project_path {
            worktree.project = project.clone();
            updated = true;
        }

        for group in &mut worktree.groups {
            for member in &mut group.members {
                if member.path == project_path {
                    *member = project.clone();
                    updated = true;
                }
            }
        }

        for vendored in &mut worktree.vendored {
            if vendored.path == project_path {
                *vendored = project.clone();
                updated = true;
            }
        }
    }

    updated
}

pub(super) fn initial_disk_batch_count(projects: &[RustProject]) -> usize {
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
