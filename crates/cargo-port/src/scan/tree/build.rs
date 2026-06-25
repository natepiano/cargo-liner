use super::AbsolutePath;
use super::CargoParseResult;
use super::HashMap;
use super::HashSet;
use super::Itertools;
use super::MemberGroup;
use super::Ordering;
use super::Package;
use super::Path;
use super::ProjectFields;
use super::RootItem;
use super::RustInfo;
use super::RustProject;
use super::WalkDir;
use super::extract_vendored_new;
use super::merge_worktrees_new;
use super::normalize_workspace_path;
use super::workspace_member_paths_new;

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
fn group_members_new(
    workspace_path: &Path,
    members: Vec<Package>,
    inline_dirs: &[String],
) -> Vec<MemberGroup> {
    let group_map: HashMap<String, Vec<Package>> =
        members.into_iter().into_group_map_by(|member| {
            let relative = member
                .path()
                .strip_prefix(workspace_path)
                .ok()
                .map(normalize_workspace_path)
                .unwrap_or_default();
            let subdir = relative.split('/').next().unwrap_or("").to_string();
            if inline_dirs.contains(&subdir) || !relative.contains('/') {
                String::new()
            } else {
                subdir
            }
        });

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
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
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
    fn only_nests_manifest_members() {
        let tmp = tempfile::tempdir().expect("create manifest-member test tempdir");
        let workspace_dir = tmp.path().join("hana");
        let included_dir = workspace_dir.join("crates").join("hana");
        let vendored_dir = workspace_dir.join("crates").join("clay-layout");

        std::fs::create_dir_all(&included_dir).expect("create included workspace member directory");
        std::fs::create_dir_all(&vendored_dir).expect("create vendored crate directory");
        std::fs::write(
            workspace_dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/hana\"]\n",
        )
        .expect("write workspace manifest fixture");

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
            .expect("find workspace root item");
        let RootItem::Rust(RustProject::Workspace(ws)) = ws_item else {
            panic!("workspace root item should be a workspace")
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
    fn assigns_workspace_path_dependency_to_member() {
        let tmp = tempfile::tempdir().expect("create path-dependency test tempdir");
        let workspace_dir = tmp.path().join("bevy_hana");
        let member_dir = workspace_dir.join("crates").join("bevy_diegetic");
        let sibling_dir = workspace_dir.join("crates").join("bevy_lagrange");
        let vendored_dir = workspace_dir.join("vendor").join("clay-layout");

        std::fs::create_dir_all(&member_dir).expect("create workspace member directory");
        std::fs::create_dir_all(&sibling_dir).expect("create sibling workspace member directory");
        std::fs::create_dir_all(&vendored_dir).expect("create vendored dependency directory");
        std::fs::write(
            workspace_dir.join("Cargo.toml"),
            "[workspace]\n\
             members = [\"crates/*\"]\n\
             exclude = [\"vendor/clay-layout\"]\n\
             \n\
             [workspace.dependencies]\n\
             clay-layout = { path = \"vendor/clay-layout\" }\n",
        )
        .expect("write workspace dependency manifest fixture");
        std::fs::write(
            member_dir.join("Cargo.toml"),
            "[package]\n\
             name = \"bevy_diegetic\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dev-dependencies]\n\
             clay-layout = { workspace = true }\n",
        )
        .expect("write member manifest fixture");
        std::fs::write(
            sibling_dir.join("Cargo.toml"),
            "[package]\nname = \"bevy_lagrange\"\nversion = \"0.1.0\"\n",
        )
        .expect("write sibling manifest fixture");

        let workspace = make_workspace(
            Some("bevy_hana"),
            &workspace_dir.to_string_lossy(),
            false,
            None,
        );
        let member = make_package(
            Some("bevy_diegetic"),
            &member_dir.to_string_lossy(),
            false,
            None,
        );
        let sibling = make_package(
            Some("bevy_lagrange"),
            &sibling_dir.to_string_lossy(),
            false,
            None,
        );
        let vendored = make_package(
            Some("clay-layout"),
            &vendored_dir.to_string_lossy(),
            false,
            None,
        );

        let items = build_tree(
            &[workspace, member, sibling, vendored],
            &["crates".to_string()],
        );

        let RootItem::Rust(RustProject::Workspace(ws)) = &items[0] else {
            panic!("tree root should be a workspace")
        };
        assert!(ws.vendored().is_empty());
        let member = ws.groups()[0]
            .members()
            .iter()
            .find(|member| member.path() == member_dir.as_path())
            .expect("find workspace member with path dependency");
        assert_eq!(member.vendored().len(), 1);
        assert_eq!(member.vendored()[0].path(), vendored_dir.as_path());
    }
}
