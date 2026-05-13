use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;

use toml::Table;
use toml::Value;
use walkdir::WalkDir;

use crate::project::AbsolutePath;
use crate::project::CargoParseResult;
use crate::project::MemberGroup;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::WorktreeGroup;

pub(crate) fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Build a project tree from a flat list of discovered `RootItem`s.
///
/// The input must contain only `Rust(Workspace)`, `Rust(Package)`, and `NonRust` variants
/// (discovery does not produce worktree groups). This function:
/// 1. Nests workspace members into their parent workspace's `groups`
/// 2. Detects vendored crates nested inside other projects
/// 3. Merges worktree checkouts into `WorktreeGroup` variants
pub(crate) fn build_tree(items: &[RootItem], inline_dirs: &[String]) -> Vec<RootItem> {
    let workspace_paths: Vec<&AbsolutePath> = items
        .iter()
        .filter(|item| matches!(item, RootItem::Rust(RustProject::Workspace(_))))
        .map(RootItem::path)
        .collect();

    let mut result: Vec<RootItem> = Vec::new();
    let mut consumed: HashSet<usize> = HashSet::new();

    // Identify top-level workspaces (not nested inside another workspace).
    let top_level_workspaces: HashSet<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(item, RootItem::Rust(RustProject::Workspace(_)))
                && !workspace_paths
                    .iter()
                    .any(|ws| *ws != item.path() && item.path().starts_with(ws.as_path()))
        })
        .map(|(i, _)| i)
        .collect();

    for (i, item) in items.iter().enumerate() {
        if !top_level_workspaces.contains(&i) {
            continue;
        }
        let RootItem::Rust(RustProject::Workspace(ws)) = item else {
            continue;
        };
        let ws_path = ws.path().to_path_buf();
        let member_paths = workspace_member_paths_new(&ws_path, items);

        let mut all_members: Vec<Package> = items
            .iter()
            .enumerate()
            .filter(|(j, candidate)| {
                *j != i
                    && !top_level_workspaces.contains(j)
                    && member_paths.contains(candidate.path())
            })
            .filter_map(|(j, candidate)| {
                consumed.insert(j);
                if let RootItem::Rust(RustProject::Package(pkg)) = candidate {
                    Some(pkg.clone())
                } else if let RootItem::Rust(RustProject::Workspace(nested_ws)) = candidate {
                    // Nested workspace treated as a package member
                    Some(Package {
                        path:            nested_ws.path().clone(),
                        name:            nested_ws.name().map(str::to_string),
                        worktree_status: nested_ws.worktree_status().clone(),
                        rust:            RustInfo {
                            cargo: nested_ws.cargo.clone(),
                            ..RustInfo::default()
                        },
                    })
                } else {
                    None
                }
            })
            .collect();

        all_members.sort_by(|a, b| a.package_name().as_str().cmp(b.package_name().as_str()));

        let groups = group_members_new(&ws_path, all_members, inline_dirs);

        let mut new_ws = ws.clone();
        *new_ws.groups_mut() = groups;
        consumed.insert(i);
        result.push(RootItem::Rust(RustProject::Workspace(new_ws)));
    }

    for (i, item) in items.iter().enumerate() {
        if consumed.contains(&i) {
            continue;
        }
        result.push(item.clone());
    }

    result.sort_by(|a, b| a.path().cmp(b.path()));

    extract_vendored_new(&mut result);
    merge_worktrees_new(&mut result);

    result
}

fn workspace_member_paths_new(ws_path: &Path, items: &[RootItem]) -> HashSet<AbsolutePath> {
    let manifest = ws_path.join("Cargo.toml");
    let Some((members, excludes)) = workspace_member_patterns(&manifest) else {
        return items
            .iter()
            .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
            .map(|item| item.path().clone())
            .collect();
    };

    items
        .iter()
        .filter(|item| item.path().starts_with(ws_path) && item.path() != ws_path)
        .filter_map(|item| {
            item.path().strip_prefix(ws_path).ok().and_then(|relative| {
                let relative_str = normalize_workspace_path(relative);
                let included = members
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative_str));
                let is_excluded = excludes
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, &relative_str));
                if included && !is_excluded {
                    Some(item.path().clone())
                } else {
                    None
                }
            })
        })
        .collect()
}

fn workspace_member_patterns(manifest_path: &Path) -> Option<(Vec<String>, Vec<String>)> {
    let contents = std::fs::read_to_string(manifest_path).ok()?;
    let table: Table = contents.parse().ok()?;
    let workspace = table.get("workspace")?.as_table()?;

    let members = workspace
        .get("members")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let excludes = workspace
        .get("exclude")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some((members, excludes))
}

pub(crate) fn normalize_workspace_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn workspace_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern_segments: Vec<&str> = pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let path_segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    workspace_pattern_matches_segments(&pattern_segments, &path_segments)
}

fn workspace_pattern_matches_segments(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((&"**", rest)) => {
            workspace_pattern_matches_segments(rest, path)
                || (!path.is_empty() && workspace_pattern_matches_segments(pattern, &path[1..]))
        },
        Some((segment, rest)) => {
            !path.is_empty()
                && workspace_pattern_matches_segment(segment, path[0])
                && workspace_pattern_matches_segments(rest, &path[1..])
        },
    }
}

fn workspace_pattern_matches_segment(pattern: &str, value: &str) -> bool {
    fn matches(pattern: &[u8], value: &[u8]) -> bool {
        match pattern.split_first() {
            None => value.is_empty(),
            Some((b'*', rest)) => {
                matches(rest, value) || (!value.is_empty() && matches(pattern, &value[1..]))
            },
            Some((b'?', rest)) => !value.is_empty() && matches(rest, &value[1..]),
            Some((head, rest)) => {
                !value.is_empty() && *head == value[0] && matches(rest, &value[1..])
            },
        }
    }

    matches(pattern.as_bytes(), value.as_bytes())
}

/// Group worktree checkouts under their primary project.
///
/// Projects sharing the same `worktree_primary_abs_path` are wrapped in a
/// `Worktrees(WorktreeGroup { primary, linked })`. Each entry independently
/// carries its own `RustProject` kind, so a primary `Package` can hold a
/// linked `Workspace` (or vice versa) when one checkout has been converted
/// and another has not. `NonRust` projects are not grouped.
fn item_worktree_identity(item: &RootItem) -> Option<&AbsolutePath> {
    match item {
        RootItem::Rust(p) => p.worktree_status().primary_root(),
        _ => None,
    }
}

fn item_is_linked(item: &RootItem) -> bool {
    match item {
        RootItem::Rust(p) => p.worktree_status().is_linked_worktree(),
        _ => false,
    }
}

pub(super) fn merge_worktrees_new(items: &mut Vec<RootItem>) {
    let mut primary_indices: HashMap<AbsolutePath, usize> = HashMap::new();
    let mut worktree_indices: Vec<usize> = Vec::new();

    for (i, item) in items.iter().enumerate() {
        let Some(identity) = item_worktree_identity(item) else {
            continue;
        };
        let is_linked = item_is_linked(item);
        if is_linked {
            worktree_indices.push(i);
        } else {
            primary_indices.insert(identity.clone(), i);
        }
    }

    let identities_with_worktrees: HashSet<AbsolutePath> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            item_worktree_identity(&items[wi])
                .filter(|id| primary_indices.contains_key(id.as_path()))
                .cloned()
        })
        .collect();

    if identities_with_worktrees.is_empty() {
        return;
    }

    // Extract worktree items (highest index first to preserve lower indices)
    let mut moves: Vec<(usize, AbsolutePath)> = worktree_indices
        .iter()
        .filter_map(|&wi| {
            let id = item_worktree_identity(&items[wi])?.clone();
            primary_indices.get(id.as_path())?;
            Some((wi, id))
        })
        .collect();
    moves.sort_by_key(|entry| Reverse(entry.0));

    let mut extracted: Vec<(RootItem, AbsolutePath)> = Vec::new();
    for (wi, id) in moves {
        let item = items.remove(wi);
        extracted.push((item, id));
    }

    // Rebuild primary_indices after removals
    let mut primary_map: HashMap<AbsolutePath, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(id) = item_worktree_identity(item)
            .filter(|id| identities_with_worktrees.contains(*id))
            .filter(|_| !item_is_linked(item))
        {
            primary_map.insert(id.clone(), i);
        }
    }

    // Group linked worktrees by identity, preserving order
    let mut linked_by_id: HashMap<AbsolutePath, Vec<RootItem>> = HashMap::new();
    for (item, id) in extracted {
        linked_by_id.entry(id).or_default().push(item);
    }

    // Replace each primary with a WorktreeGroup wrapping primary + linked.
    // Process in reverse to avoid index shifting.
    let mut replacements: Vec<(usize, RootItem)> = Vec::new();
    for (id, idx) in &primary_map {
        let linked = linked_by_id.remove(id).unwrap_or_default();
        let RootItem::Rust(primary) = &items[*idx] else {
            continue;
        };
        let linked_projects: Vec<RustProject> = linked
            .into_iter()
            .filter_map(|item| match item {
                RootItem::Rust(p) => Some(p),
                _ => None,
            })
            .collect();
        let replacement = RootItem::Worktrees(WorktreeGroup::new(primary.clone(), linked_projects));
        replacements.push((*idx, replacement));
    }

    for (idx, replacement) in replacements {
        items[idx] = replacement;
    }
}

/// Find standalone items whose path lives inside another item's directory
/// and move them into that item's `vendored` list.
fn extract_vendored_new(items: &mut Vec<RootItem>) {
    let parent_paths: Vec<(usize, AbsolutePath)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (i, item.path().clone()))
        .collect();

    let mut vendored_map: Vec<(usize, usize)> = Vec::new();

    for (vi, vitem) in items.iter().enumerate() {
        let has_structure = match vitem {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.groups().iter().any(|g| !g.members().is_empty())
            },
            RootItem::Worktrees(_) => true,
            _ => false,
        };
        if has_structure {
            continue;
        }
        for &(ni, ref parent_path) in &parent_paths {
            if ni == vi {
                continue;
            }
            if vitem.path().starts_with(parent_path) && vitem.path() != parent_path {
                vendored_map.push((vi, ni));
                break;
            }
        }
    }

    if vendored_map.is_empty() {
        return;
    }

    let mut remove_indices: Vec<usize> = vendored_map.iter().map(|&(vi, _)| vi).collect();
    remove_indices.sort_unstable();
    remove_indices.dedup();

    // Convert vendored items to `VendoredPackage`
    let mut vendored_projects: Vec<(usize, VendoredPackage)> = Vec::new();
    for &(vi, ni) in &vendored_map {
        let vendored = match &items[vi] {
            RootItem::Rust(RustProject::Package(p)) => VendoredPackage {
                path:             p.path.clone(),
                name:             p.name.clone(),
                worktree_status:  p.worktree_status.clone(),
                info:             p.rust.info.clone(),
                cargo:            p.rust.cargo.clone(),
                crates_version:   p.rust.crates_version.clone(),
                crates_downloads: p.rust.crates_downloads,
            },
            RootItem::Rust(RustProject::Workspace(ws)) => VendoredPackage {
                path: ws.path().clone(),
                name: ws.name().map(String::from),
                worktree_status: ws.worktree_status().clone(),
                cargo: ws.cargo.clone(),
                ..VendoredPackage::default()
            },
            RootItem::NonRust(nr) => VendoredPackage {
                path: nr.path().clone(),
                name: nr.name().map(String::from),
                ..VendoredPackage::default()
            },
            _ => continue,
        };
        vendored_projects.push((ni, vendored));
    }

    for &idx in remove_indices.iter().rev() {
        items.remove(idx);
    }

    for (ni, vendored) in vendored_projects {
        let adjusted_ni = remove_indices.iter().filter(|&&r| r < ni).count();
        let target_ni = ni - adjusted_ni;
        if let Some(item) = items.get_mut(target_ni) {
            match item {
                RootItem::Rust(RustProject::Workspace(ws)) => ws.vendored_mut().push(vendored),
                RootItem::Rust(RustProject::Package(p)) => p.vendored_mut().push(vendored),
                _ => {},
            }
        }
    }

    // Sort vendored lists
    for item in items {
        match item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                pkg.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            },
            _ => {},
        }
    }
}

fn group_members_new(
    workspace_path: &Path,
    members: Vec<Package>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let mut group_map: HashMap<String, Vec<Package>> = HashMap::new();

    for member in members {
        let relative = member
            .path()
            .strip_prefix(workspace_path)
            .ok()
            .map(normalize_workspace_path)
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

    groups
}

/// Convert a `CargoProject` (from `from_cargo_toml()`) into a `RootItem`.
pub(crate) fn cargo_project_to_item(cp: CargoParseResult) -> RootItem {
    match cp {
        CargoParseResult::Workspace(ws) => RootItem::Rust(RustProject::Workspace(ws)),
        CargoParseResult::Package(pkg) => RootItem::Rust(RustProject::Package(pkg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Workspace;
    use crate::project::WorktreeStatus;

    fn status_for(
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> WorktreeStatus {
        match (is_linked_worktree, primary_abs) {
            (_, None) => WorktreeStatus::NotGit,
            (true, Some(p)) => WorktreeStatus::Linked {
                primary: AbsolutePath::from(p.to_string()),
            },
            (false, Some(p)) => WorktreeStatus::Primary {
                root: AbsolutePath::from(p.to_string()),
            },
        }
    }

    fn make_workspace(
        name: Option<&str>,
        abs_path: &str,
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: AbsolutePath::from(abs_path),
            name: name.map(String::from),
            worktree_status: status_for(is_linked_worktree, primary_abs),
            ..Workspace::default()
        }))
    }

    fn make_package(
        name: Option<&str>,
        abs_path: &str,
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(abs_path),
            name: name.map(String::from),
            worktree_status: status_for(is_linked_worktree, primary_abs),
            ..Package::default()
        }))
    }

    #[test]
    fn merge_virtual_workspace() {
        let primary = make_workspace(None, "/home/ws", false, Some("/home/ws"));
        let worktree = make_workspace(None, "/home/ws_feat", true, Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1, "worktree should be merged into primary");
        let RootItem::Worktrees(group) = &items[0] else {
            std::process::abort()
        };
        assert!(
            matches!(&group.primary, RustProject::Workspace(_)),
            "primary should be a workspace"
        );
        assert_eq!(group.linked.len(), 1, "should have one linked worktree");
    }

    #[test]
    fn merge_named_workspace() {
        let primary = make_workspace(Some("my-ws"), "/home/ws", false, Some("/home/ws"));
        let worktree = make_workspace(Some("my-ws"), "/home/ws_feat", true, Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let RootItem::Worktrees(group) = &items[0] else {
            std::process::abort()
        };
        assert!(
            matches!(&group.primary, RustProject::Workspace(_)),
            "primary should be a workspace"
        );
        assert_eq!(group.linked.len(), 1);
    }

    #[test]
    fn build_tree_only_nests_manifest_members() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let workspace_dir = tmp.path().join("hana");
        let included_dir = workspace_dir.join("crates").join("hana");
        let vendored_dir = workspace_dir.join("crates").join("clay-layout");

        std::fs::create_dir_all(&included_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&vendored_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::write(
            workspace_dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/hana\"]\n",
        )
        .unwrap_or_else(|_| std::process::abort());

        let workspace = make_workspace(Some("hana"), &workspace_dir.to_string_lossy(), false, None);
        let included = make_package(
            Some("hana-node-api"),
            &included_dir.to_string_lossy(),
            false,
            None,
        );
        let vendored = make_package(
            Some("clay-layout"),
            &vendored_dir.to_string_lossy(),
            false,
            None,
        );

        let items = build_tree(&[workspace, included, vendored], &["crates".to_string()]);

        let ws_item = items
            .iter()
            .find(|item| item.path() == workspace_dir.as_path())
            .unwrap_or_else(|| std::process::abort());
        let RootItem::Rust(RustProject::Workspace(ws)) = ws_item else {
            std::process::abort()
        };
        assert_eq!(ws.groups().len(), 1);
        assert_eq!(ws.groups()[0].members().len(), 1);
        assert_eq!(ws.groups()[0].members()[0].path(), included_dir.as_path());
        assert!(
            ws.groups()
                .iter()
                .flat_map(|group| group.members().iter())
                .all(|member| member.path() != vendored_dir.as_path()),
            "non-member crate should not be grouped as a workspace member"
        );
        assert_eq!(ws.vendored().len(), 1);
        assert_eq!(ws.vendored()[0].path(), vendored_dir.as_path());
    }

    #[test]
    fn merge_standalone_project() {
        let primary = make_package(Some("app"), "/home/app", false, Some("/home/app"));
        let worktree = make_package(Some("app"), "/home/app_feat", true, Some("/home/app"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let RootItem::Worktrees(group) = &items[0] else {
            std::process::abort()
        };
        assert!(
            matches!(&group.primary, RustProject::Package(_)),
            "primary should be a package"
        );
        assert_eq!(group.linked.len(), 1);
    }

    #[test]
    fn no_merge_different_repos() {
        let a = make_package(Some("a"), "/home/a", false, Some("/home/a"));
        let b = make_package(Some("b"), "/home/b", true, Some("/home/b"));
        let mut items = vec![a, b];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 2, "different repos should remain separate");
    }

    #[test]
    fn no_merge_none_identity() {
        let a = make_package(Some("x"), "/home/x", false, None);
        let b = make_package(Some("x"), "/home/x2", true, None);
        let mut items = vec![a, b];
        merge_worktrees_new(&mut items);

        assert_eq!(
            items.len(),
            2,
            "nodes without identity should not be merged"
        );
    }
}
