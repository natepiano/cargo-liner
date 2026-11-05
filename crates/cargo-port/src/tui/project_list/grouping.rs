use std::cmp::Ordering;
use std::collections::HashMap;

use indexmap::IndexMap;

use super::list;
use crate::project::AbsolutePath;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectEntry;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::project::WorktreeStatus;
use crate::scan;

pub(super) fn shortest_unique_suffixes(paths: &[String]) -> Vec<String> {
    let segments: Vec<Vec<&str>> = paths
        .iter()
        .map(|path| display_path_segments(path))
        .collect();
    let mut suffix_len = vec![1usize; paths.len()];

    loop {
        let mut collisions: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, path_segments) in segments.iter().enumerate() {
            collisions
                .entry(join_suffix(path_segments, suffix_len[index]))
                .or_default()
                .push(index);
        }

        let mut changed = false;
        for indices in collisions.into_values().filter(|indices| indices.len() > 1) {
            for index in indices {
                if suffix_len[index] < segments[index].len() {
                    suffix_len[index] += 1;
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    segments
        .iter()
        .enumerate()
        .map(|(index, path_segments)| join_suffix(path_segments, suffix_len[index]))
        .collect()
}

fn display_path_segments(path: &str) -> Vec<&str> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn join_suffix(segments: &[&str], suffix_len: usize) -> String {
    let len = suffix_len.min(segments.len());
    segments[segments.len().saturating_sub(len)..].join("/")
}

pub(super) enum ProjectPlacement {
    Inserted,
    Unplaced(Box<RootItem>),
}

pub(super) fn try_attach_worktree(
    existing: &mut RootItem,
    item: Box<RootItem>,
) -> ProjectPlacement {
    let RootItem::Rust(linked) = &*item else {
        return ProjectPlacement::Unplaced(item);
    };
    if !linked.worktree_status().is_linked_worktree()
        || linked.worktree_status().primary_root() != item_worktree_identity(existing)
    {
        return ProjectPlacement::Unplaced(item);
    }
    match (existing, *item) {
        (existing @ RootItem::Rust(_), RootItem::Rust(linked)) => {
            let mut group = WorktreeGroup::new(linked, Vec::new());
            if let RootItem::Rust(primary) = existing {
                std::mem::swap(primary, &mut group.primary);
            }
            if let RootItem::Rust(linked) = std::mem::replace(existing, RootItem::Worktrees(group))
                && let RootItem::Worktrees(group) = existing
            {
                group.linked.push(linked);
            }
            ProjectPlacement::Inserted
        },
        (RootItem::Worktrees(group), RootItem::Rust(linked)) => {
            group.linked.push(linked);
            ProjectPlacement::Inserted
        },
        (_, item) => ProjectPlacement::Unplaced(Box::new(item)),
    }
}

fn item_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) => p.worktree_status().primary_root(),
        RootItem::Worktrees(group) => group.primary.worktree_status().primary_root(),
        RootItem::NonRust(_) => None,
    }
}

pub(super) fn linked_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) => match p.worktree_status() {
            WorktreeStatus::Linked { primary } => Some(primary),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn find_matching_worktree_container(
    roots: &IndexMap<AbsolutePath, ProjectEntry>,
    linked_index: usize,
    identity: &AbsolutePath,
) -> Option<usize> {
    roots.values().enumerate().find_map(|(index, entry)| {
        if index == linked_index {
            return None;
        }
        (item_worktree_identity(&entry.root_item) == Some(identity)).then_some(index)
    })
}

pub(super) fn regroup_workspace(ws: &mut Workspace, inline_dirs: &[String]) {
    // Drain every `ws.groups_mut()` group into `members` using `MemberGroup::into_members`.
    let members: Vec<Package> = ws
        .groups_mut()
        .drain(..)
        .flat_map(MemberGroup::into_members)
        .collect();

    replace_workspace_members(ws, members, inline_dirs);
}

pub(super) fn replace_workspace_members(
    ws: &mut Workspace,
    mut members: Vec<Package>,
    inline_dirs: &[String],
) {
    members.sort_by(|a, b| a.package_name().as_str().cmp(b.package_name().as_str()));

    // Build `group_map` by each member's first relative subdirectory and `inline_dirs`, then
    // replace `ws.groups_mut()` with sorted `MemberGroup` values.
    let mut group_map: HashMap<String, Vec<Package>> = std::collections::HashMap::new();
    for member in members {
        let relative = member
            .path()
            .strip_prefix(ws.path())
            .ok()
            .map(scan::normalize_workspace_path)
            .unwrap_or_default();
        let subdir = relative.split('/').next().unwrap_or("").to_string();
        let group_name = if inline_dirs.contains(&subdir) || !relative.contains('/') {
            String::new()
        } else {
            subdir
        };
        group_map.entry(group_name).or_default().push(member);
    }

    let mut groups: Vec<MemberGroup> = group_map
        .into_iter()
        .map(|(name, members)| {
            if name.is_empty() {
                MemberGroup::Inline { members }
            } else {
                MemberGroup::Named { name, members }
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        let a_inline = a.group_name().is_empty();
        let b_inline = b.group_name().is_empty();
        match (a_inline, b_inline) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => a.group_name().cmp(b.group_name()),
        }
    });

    *ws.groups_mut() = groups;
}

pub(super) fn try_insert_member(ws: &mut Workspace, item: Box<RootItem>) -> ProjectPlacement {
    if !item.path().starts_with(ws.path()) || item.path() == ws.path() {
        return ProjectPlacement::Unplaced(item);
    }
    let RootItem::Rust(RustProject::Package(pkg)) = *item else {
        return ProjectPlacement::Unplaced(item);
    };
    if let Some(current) = ws
        .groups_mut()
        .iter_mut()
        .flat_map(MemberGroup::members_mut)
        .find(|member| member.path() == pkg.path())
    {
        list::merge_retained_rust_info(&mut current.rust, pkg.rust);
        return ProjectPlacement::Inserted;
    }
    let inline = ws.groups_mut().iter_mut().find(|g| g.is_inline());
    if let Some(group) = inline {
        group.members_mut().push(pkg);
        group
            .members_mut()
            .sort_by(|a, b| a.package_name().as_str().cmp(b.package_name().as_str()));
    } else {
        ws.groups_mut()
            .push(MemberGroup::Inline { members: vec![pkg] });
    }
    ProjectPlacement::Inserted
}
