use super::AbsolutePath;
use super::HashMap;
use super::HashSet;
use super::Itertools;
use super::Reverse;
use super::RootItem;
use super::RustProject;
use super::WorktreeGroup;

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

    // Group linked worktrees by identity, preserving encounter order within
    // each group.
    let mut linked_by_id: HashMap<AbsolutePath, Vec<RootItem>> = extracted
        .into_iter()
        .map(|(item, id)| (id, item))
        .into_group_map();

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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::merge_worktrees_new;
    use crate::project::AbsolutePath;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::Workspace;
    use crate::project::WorktreeStatus;

    fn status_for(is_linked_worktree: bool, primary_abs: Option<&str>) -> WorktreeStatus {
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
    fn merges_virtual_workspace() {
        let primary = make_workspace(None, "/home/ws", false, Some("/home/ws"));
        let worktree = make_workspace(None, "/home/ws_feat", true, Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1, "worktree should be merged into primary");
        let RootItem::Worktrees(group) = &items[0] else {
            panic!("merged workspace should produce a worktree group")
        };
        assert!(
            matches!(&group.primary, RustProject::Workspace(_)),
            "primary should be a workspace"
        );
        assert_eq!(group.linked.len(), 1, "should have one linked worktree");
    }

    #[test]
    fn merges_named_workspace() {
        let primary = make_workspace(Some("my-ws"), "/home/ws", false, Some("/home/ws"));
        let worktree = make_workspace(Some("my-ws"), "/home/ws_feat", true, Some("/home/ws"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let RootItem::Worktrees(group) = &items[0] else {
            panic!("merged named workspace should produce a worktree group")
        };
        assert!(
            matches!(&group.primary, RustProject::Workspace(_)),
            "primary should be a workspace"
        );
        assert_eq!(group.linked.len(), 1);
    }

    #[test]
    fn merges_standalone_project() {
        let primary = make_package(Some("app"), "/home/app", false, Some("/home/app"));
        let worktree = make_package(Some("app"), "/home/app_feat", true, Some("/home/app"));
        let mut items = vec![primary, worktree];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 1);
        let RootItem::Worktrees(group) = &items[0] else {
            panic!("merged package should produce a worktree group")
        };
        assert!(
            matches!(&group.primary, RustProject::Package(_)),
            "primary should be a package"
        );
        assert_eq!(group.linked.len(), 1);
    }

    #[test]
    fn leaves_different_repos_unmerged() {
        let a = make_package(Some("a"), "/home/a", false, Some("/home/a"));
        let b = make_package(Some("b"), "/home/b", true, Some("/home/b"));
        let mut items = vec![a, b];
        merge_worktrees_new(&mut items);

        assert_eq!(items.len(), 2, "different repos should remain separate");
    }

    #[test]
    fn leaves_items_without_identity_unmerged() {
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
