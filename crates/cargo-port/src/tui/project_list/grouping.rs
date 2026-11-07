use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;

use indexmap::IndexMap;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

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
use crate::tui::render;

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

/// Remove shared path ends and shared words within each prefix group.
pub(in crate::tui) fn differing_path_labels(paths: &[String]) -> Vec<String> {
    let labels = shortest_unique_suffixes(paths);
    let mut segments: Vec<Vec<&str>> = labels
        .iter()
        .map(|label| display_path_segments(label))
        .collect();
    while segments.len() > 1
        && segments
            .iter()
            .all(|parts| parts.len() > 1 && parts.last() == segments[0].last())
    {
        for parts in &mut segments {
            parts.pop();
        }
    }
    let labels: Vec<String> = segments.iter().map(|parts| parts.join("/")).collect();
    let words: Vec<Vec<&str>> = labels
        .iter()
        .map(|label| label.split_inclusive(['-', '_', '.', '/']).collect())
        .collect();
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, parts) in words.iter().enumerate() {
        groups
            .entry(parts.first().copied().unwrap_or_default())
            .or_default()
            .push(index);
    }
    let mut shortened = labels.clone();
    for indices in groups.into_values().filter(|indices| indices.len() > 1) {
        let first = &words[indices[0]];
        let mut start = 0;
        while indices
            .iter()
            .all(|&index| words[index].len() > start + 1 && words[index][start] == first[start])
        {
            start += 1;
        }
        let mut end = 0;
        while indices.iter().all(|&index| {
            let parts = &words[index];
            parts.len() > start + end + 1
                && parts[parts.len() - end - 1] == first[first.len() - end - 1]
        }) {
            end += 1;
        }
        let candidates: Vec<String> = indices
            .iter()
            .map(|&index| {
                let parts = &words[index];
                parts[start..parts.len() - end]
                    .concat()
                    .trim_matches(['-', '_', '.', '/'])
                    .to_string()
            })
            .collect();
        let unique: HashSet<_> = candidates.iter().collect();
        let keep_suffix = unique.len() < candidates.len();
        for (index, candidate) in indices.into_iter().zip(candidates) {
            shortened[index] = if keep_suffix {
                words[index][start..]
                    .concat()
                    .trim_matches(['-', '_', '.', '/'])
                    .to_string()
            } else {
                candidate
            };
        }
    }
    let unique: HashSet<_> = shortened.iter().collect();
    if unique.len() < shortened.len() {
        labels
    } else {
        shortened
    }
}

/// Keep different label ends when an end cut makes the cells equal.
pub(in crate::tui) fn fit_distinct_path_labels(labels: &[String], width: usize) -> Vec<String> {
    let mut fitted: Vec<String> = labels
        .iter()
        .map(|label| render::truncate_with_ellipsis(label, width, "…"))
        .collect();
    let mut collisions: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, label) in fitted.iter().enumerate() {
        collisions.entry(label.clone()).or_default().push(index);
    }
    for indices in collisions.into_values().filter(|indices| indices.len() > 1) {
        if indices
            .iter()
            .all(|&index| labels[index] == labels[indices[0]])
        {
            continue;
        }
        for suffix_width in 1..width.saturating_sub(1) {
            let candidates: Vec<String> = indices
                .iter()
                .map(|&index| {
                    let label = &labels[index];
                    let mut suffix = String::new();
                    for ch in label.chars().rev() {
                        if suffix.width() + ch.width().unwrap_or(0) > suffix_width {
                            break;
                        }
                        suffix.insert(0, ch);
                    }
                    let prefix = render::truncate_with_ellipsis(
                        label,
                        width.saturating_sub(1 + suffix.width()),
                        "",
                    );
                    format!("{prefix}…{suffix}")
                })
                .collect();
            let unique: HashSet<_> = candidates.iter().collect();
            if unique.len() == candidates.len() {
                for (index, candidate) in indices.iter().zip(candidates) {
                    fitted[*index] = candidate;
                }
                break;
            }
        }
    }
    fitted
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

#[cfg(test)]
mod label_tests {
    use tui_pane::ColorMode;

    use super::differing_path_labels;
    use crate::tui::test_support;

    #[test]
    fn projects_keep_distinct_path_labels_after_the_width_cut() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let paths =
            ["work-a", "work_a"].map(|parent| scratch.path().join(parent).join("proof_crate"));
        let items = paths
            .each_ref()
            .map(|path| test_support::make_package("proof_crate", path));
        for width in [80, 95] {
            let mut app = test_support::make_app_with_color_mode(&items, ColorMode::Colored);
            app.ensure_visible_rows_cached();
            let buffer = test_support::draw_ui(&mut app, width)?;
            let first = test_support::position(&buffer, "work-a")?;
            let second = test_support::position(&buffer, "work_a")?;
            assert_ne!(
                first.1,
                second.1,
                "{width}: {:?}",
                test_support::rows(&buffer)
            );
        }
        Ok(())
    }

    #[test]
    fn path_labels_keep_delimiters_that_distinguish_checkouts() {
        let paths = ["/tmp/work-a/proof_crate", "/tmp/work_a/proof_crate"].map(str::to_string);
        let labels = differing_path_labels(&paths);
        assert_eq!(labels, ["work-a", "work_a"]);
    }
}
