use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use itertools::Itertools;
use toml::Table;
use toml::Value;
use walkdir::WalkDir;

use crate::constants::CARGO_TOML;
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

mod build;
mod dependencies;
mod vendored;
mod workspace;
mod worktrees;

pub(crate) use build::build_tree;
pub(crate) use build::cargo_project_to_item;
pub(crate) use build::dir_size;
use dependencies::package_path_dependencies;
use dependencies::workspace_path_dependencies;
use vendored::extract_vendored_new;
pub(crate) use workspace::normalize_workspace_path;
use workspace::workspace_member_paths_new;
pub(super) fn merge_worktrees_new(items: &mut Vec<RootItem>) {
    worktrees::merge_worktrees_new(items);
}

#[cfg(test)]
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
    fn build_tree_assigns_workspace_path_dependency_to_member() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let workspace_dir = tmp.path().join("bevy_hana");
        let member_dir = workspace_dir.join("crates").join("bevy_diegetic");
        let sibling_dir = workspace_dir.join("crates").join("bevy_lagrange");
        let vendored_dir = workspace_dir.join("vendor").join("clay-layout");

        std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&sibling_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&vendored_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::write(
            workspace_dir.join("Cargo.toml"),
            "[workspace]\n\
             members = [\"crates/*\"]\n\
             exclude = [\"vendor/clay-layout\"]\n\
             \n\
             [workspace.dependencies]\n\
             clay-layout = { path = \"vendor/clay-layout\" }\n",
        )
        .unwrap_or_else(|_| std::process::abort());
        std::fs::write(
            member_dir.join("Cargo.toml"),
            "[package]\n\
             name = \"bevy_diegetic\"\n\
             version = \"0.1.0\"\n\
             \n\
             [dev-dependencies]\n\
             clay-layout = { workspace = true }\n",
        )
        .unwrap_or_else(|_| std::process::abort());
        std::fs::write(
            sibling_dir.join("Cargo.toml"),
            "[package]\nname = \"bevy_lagrange\"\nversion = \"0.1.0\"\n",
        )
        .unwrap_or_else(|_| std::process::abort());

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
            std::process::abort()
        };
        assert!(ws.vendored().is_empty());
        let member = ws.groups()[0]
            .members()
            .iter()
            .find(|member| member.path() == member_dir.as_path())
            .unwrap_or_else(|| std::process::abort());
        assert_eq!(member.vendored().len(), 1);
        assert_eq!(member.vendored()[0].path(), vendored_dir.as_path());
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
