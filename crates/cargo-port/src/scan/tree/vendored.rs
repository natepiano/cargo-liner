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
    for &(vi, vendored_destination) in &vendored_map {
        let vendored = match &items[vi] {
            RootItem::Rust(RustProject::Package(p)) => VendoredPackage {
                path:            p.path.clone(),
                name:            p.name.clone(),
                worktree_status: p.worktree_status.clone(),
                project_info:    p.rust.project_info.clone(),
                cargo:           p.rust.cargo.clone(),
                crates_io:       p.rust.crates_io.clone(),
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
        vendored_projects.push((vendored_destination, vendored));
    }

    for &idx in remove_indices.iter().rev() {
        items.remove(idx);
    }

    for (vendored_destination, vendored) in vendored_projects {
        push_vendored_item(
            items,
            vendored_destination.adjusted(&remove_indices),
            vendored,
        );
    }

    for root_item in items {
        sort_vendored_lists(root_item);
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

    for (node_index, root_item) in items.iter().enumerate() {
        let RootItem::Rust(RustProject::Workspace(ws)) = root_item else {
            continue;
        };
        if !vendored_path.starts_with(ws.path().as_path()) {
            continue;
        }

        let workspace_dependencies = workspace_path_dependencies(ws.path().as_path());
        for (group_index, member_group) in ws.groups().iter().enumerate() {
            for (member_index, package) in member_group.members().iter().enumerate() {
                let dependencies =
                    package_path_dependencies(package.path().as_path(), &workspace_dependencies);
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
        [vendored_destination] => Some(*vendored_destination),
        _ => None,
    }
}

fn contained_vendored_destination(
    vendored_root_item: &RootItem,
    vendored_index: usize,
    parent_paths: &[(usize, AbsolutePath)],
) -> Option<VendoredDestination> {
    parent_paths
        .iter()
        .find_map(|&(node_index, ref parent_path)| {
            if node_index != vendored_index
                && vendored_root_item.path().starts_with(parent_path)
                && vendored_root_item.path() != parent_path
            {
                Some(VendoredDestination::Root { node_index })
            } else {
                None
            }
        })
}
fn push_vendored_item(
    items: &mut [RootItem],
    vendored_destination: VendoredDestination,
    vendored_package: VendoredPackage,
) {
    let Some(item) = items.get_mut(match vendored_destination {
        VendoredDestination::Root { node_index }
        | VendoredDestination::WorkspaceMember { node_index, .. } => node_index,
    }) else {
        return;
    };

    match (item, vendored_destination) {
        (RootItem::Rust(RustProject::Workspace(ws)), VendoredDestination::Root { .. }) => {
            ws.vendored_mut().push(vendored_package);
        },
        (
            RootItem::Rust(RustProject::Workspace(ws)),
            VendoredDestination::WorkspaceMember {
                group_index,
                member_index,
                ..
            },
        ) => {
            if let Some(package) = ws
                .groups_mut()
                .get_mut(group_index)
                .and_then(|member_group| member_group.members_mut().get_mut(member_index))
            {
                package.vendored_mut().push(vendored_package);
            }
        },
        (RootItem::Rust(RustProject::Package(package)), VendoredDestination::Root { .. }) => {
            package.vendored_mut().push(vendored_package);
        },
        _ => {},
    }
}

fn sort_vendored_lists(root_item: &mut RootItem) {
    match root_item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            ws.vendored_mut()
                .sort_by(|left_vendored_package, b| left_vendored_package.path().cmp(b.path()));
            for member_group in ws.groups_mut() {
                for package in member_group.members_mut() {
                    package.vendored_mut().sort_by(|left_vendored_package, b| {
                        left_vendored_package.path().cmp(b.path())
                    });
                }
            }
        },
        RootItem::Rust(RustProject::Package(package)) => {
            package
                .vendored_mut()
                .sort_by(|left_vendored_package, b| left_vendored_package.path().cmp(b.path()));
        },
        _ => {},
    }
}
