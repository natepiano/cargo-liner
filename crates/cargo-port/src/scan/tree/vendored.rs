use super::AbsolutePath;
use super::ProjectFields;
use super::RootItem;
use super::RustProject;
use super::VendoredPackage;
use super::package_path_dependencies;
use super::workspace_path_dependencies;

/// Find standalone items whose path lives inside another item's directory
/// and move them into that item's `vendored` list.
pub(super) fn extract_vendored_new(items: &mut Vec<RootItem>) {
    let parent_paths: Vec<(usize, AbsolutePath)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (i, item.path().clone()))
        .collect();

    let mut vendored_map: Vec<(usize, VendoredDestination)> = Vec::new();

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

        if let Some(destination) = dependency_vendored_destination(items, vi) {
            vendored_map.push((vi, destination));
            continue;
        }

        if let Some(destination) = contained_vendored_destination(vitem, vi, &parent_paths) {
            vendored_map.push((vi, destination));
        }
    }

    if vendored_map.is_empty() {
        return;
    }

    let mut remove_indices: Vec<usize> = vendored_map.iter().map(|&(vi, _)| vi).collect();
    remove_indices.sort_unstable();
    remove_indices.dedup();

    // Convert vendored items to `VendoredPackage`
    let mut vendored_projects: Vec<(VendoredDestination, VendoredPackage)> = Vec::new();
    for &(vi, destination) in &vendored_map {
        let vendored = match &items[vi] {
            RootItem::Rust(RustProject::Package(p)) => VendoredPackage {
                path:              p.path.clone(),
                name:              p.name.clone(),
                worktree_status:   p.worktree_status.clone(),
                project_info:      p.rust.project_info.clone(),
                cargo:             p.rust.cargo.clone(),
                crates_version:    p.rust.crates_version.clone(),
                crates_prerelease: p.rust.crates_prerelease.clone(),
                crates_downloads:  p.rust.crates_downloads,
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
        vendored_projects.push((destination, vendored));
    }

    for &idx in remove_indices.iter().rev() {
        items.remove(idx);
    }

    for (destination, vendored) in vendored_projects {
        push_vendored_item(items, destination.adjusted(&remove_indices), vendored);
    }

    for item in items {
        sort_vendored_lists(item);
    }
}

#[derive(Clone, Copy)]
enum VendoredDestination {
    Root {
        node_index: usize,
    },
    WorkspaceMember {
        node_index:   usize,
        group_index:  usize,
        member_index: usize,
    },
}

impl VendoredDestination {
    fn adjusted(self, remove_indices: &[usize]) -> Self {
        let adjust = |node_index| {
            node_index
                - remove_indices
                    .iter()
                    .filter(|&&removed_index| removed_index < node_index)
                    .count()
        };
        match self {
            Self::Root { node_index } => Self::Root {
                node_index: adjust(node_index),
            },
            Self::WorkspaceMember {
                node_index,
                group_index,
                member_index,
            } => Self::WorkspaceMember {
                node_index: adjust(node_index),
                group_index,
                member_index,
            },
        }
    }
}

fn dependency_vendored_destination(
    items: &[RootItem],
    vendored_index: usize,
) -> Option<VendoredDestination> {
    let vendored_path = items[vendored_index].path();
    let mut consumers = Vec::new();

    for (node_index, item) in items.iter().enumerate() {
        let RootItem::Rust(RustProject::Workspace(ws)) = item else {
            continue;
        };
        if !vendored_path.starts_with(ws.path().as_path()) {
            continue;
        }

        let workspace_dependencies = workspace_path_dependencies(ws.path().as_path());
        for (group_index, group) in ws.groups().iter().enumerate() {
            for (member_index, member) in group.members().iter().enumerate() {
                let dependencies =
                    package_path_dependencies(member.path().as_path(), &workspace_dependencies);
                if dependencies.contains(vendored_path) {
                    consumers.push(VendoredDestination::WorkspaceMember {
                        node_index,
                        group_index,
                        member_index,
                    });
                }
            }
        }
    }

    match consumers.as_slice() {
        [destination] => Some(*destination),
        _ => None,
    }
}

fn contained_vendored_destination(
    vitem: &RootItem,
    vendored_index: usize,
    parent_paths: &[(usize, AbsolutePath)],
) -> Option<VendoredDestination> {
    parent_paths
        .iter()
        .find_map(|&(node_index, ref parent_path)| {
            if node_index != vendored_index
                && vitem.path().starts_with(parent_path)
                && vitem.path() != parent_path
            {
                Some(VendoredDestination::Root { node_index })
            } else {
                None
            }
        })
}
fn push_vendored_item(
    items: &mut [RootItem],
    destination: VendoredDestination,
    vendored: VendoredPackage,
) {
    let Some(item) = items.get_mut(match destination {
        VendoredDestination::Root { node_index }
        | VendoredDestination::WorkspaceMember { node_index, .. } => node_index,
    }) else {
        return;
    };

    match (item, destination) {
        (RootItem::Rust(RustProject::Workspace(ws)), VendoredDestination::Root { .. }) => {
            ws.vendored_mut().push(vendored);
        },
        (
            RootItem::Rust(RustProject::Workspace(ws)),
            VendoredDestination::WorkspaceMember {
                group_index,
                member_index,
                ..
            },
        ) => {
            if let Some(member) = ws
                .groups_mut()
                .get_mut(group_index)
                .and_then(|group| group.members_mut().get_mut(member_index))
            {
                member.vendored_mut().push(vendored);
            }
        },
        (RootItem::Rust(RustProject::Package(pkg)), VendoredDestination::Root { .. }) => {
            pkg.vendored_mut().push(vendored);
        },
        _ => {},
    }
}

fn sort_vendored_lists(item: &mut RootItem) {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            ws.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
            for group in ws.groups_mut() {
                for member in group.members_mut() {
                    member.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
                }
            }
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            pkg.vendored_mut().sort_by(|a, b| a.path().cmp(b.path()));
        },
        _ => {},
    }
}
