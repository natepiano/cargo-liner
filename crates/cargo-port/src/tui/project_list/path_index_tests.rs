#![allow(
    clippy::expect_used,
    reason = "tests require valid fixture paths and queued messages"
)]

use super::*;
use crate::project::MemberGroup;
use crate::project::PinnedCommit;
use crate::project::RepoInfo;
use crate::project::Submodule;
use crate::scan::BackgroundMsg;
use crate::tui::test_support;

fn package(path: &str) -> Package {
    Package {
        path: AbsolutePath::from(path),
        ..Package::default()
    }
}

fn assert_matches_linear(list: &ProjectList, paths: &[&str]) {
    for path in paths {
        let path = Path::new(path);
        let linear = list.iter().find_map(|entry| entry.root_item.at_path(path));
        assert!(linear.is_some(), "fixture path {path:?}");
        assert!(std::ptr::eq(
            list.at_path(path).expect("indexed path"),
            linear.expect("linear path")
        ));
    }
}

#[test]
fn indexed_paths_survive_insert_replace_regroup_and_rescan() {
    let mut workspace = Workspace {
        path: AbsolutePath::from("/workspace"),
        ..Workspace::default()
    };
    let mut member = package("/workspace/crates/member");
    member.vendored.push(VendoredPackage {
        path: AbsolutePath::from("/workspace/crates/member/vendor/dep"),
        ..VendoredPackage::default()
    });
    workspace.groups.push(MemberGroup::Inline {
        members: vec![member],
    });
    workspace.vendored.push(VendoredPackage {
        path: AbsolutePath::from("/workspace/vendor/root-dep"),
        ..VendoredPackage::default()
    });
    let mut list = ProjectList::new(vec![RootItem::Rust(RustProject::Workspace(workspace))]);
    let paths = [
        "/workspace",
        "/workspace/crates/member",
        "/workspace/crates/member/vendor/dep",
        "/workspace/vendor/root-dep",
    ];
    assert_matches_linear(&list, &paths);
    list.insert_into_hierarchy(RootItem::Rust(RustProject::Package(package(
        "/workspace/crates/new",
    ))));
    assert_matches_linear(&list, &["/workspace/crates/new"]);
    list.regroup_members(&["crates".to_string()]);
    assert_matches_linear(&list, &paths);
    let mut replacement = package("/workspace");
    replacement.vendored.push(VendoredPackage {
        path: AbsolutePath::from("/workspace/vendor/replaced"),
        ..VendoredPackage::default()
    });
    list.replace_leaf_by_path(
        Path::new("/workspace"),
        RootItem::Rust(RustProject::Package(replacement)),
    );
    assert_matches_linear(&list, &["/workspace", "/workspace/vendor/replaced"]);
    assert!(list.at_path(Path::new(paths[1])).is_none());
    list.replace_roots_from(ProjectList::new(vec![RootItem::Rust(
        RustProject::Package(package("/rescanned")),
    )]));
    assert_matches_linear(&list, &["/rescanned"]);
    assert!(list.at_path(Path::new("/workspace")).is_none());
}

#[test]
fn root_insert_and_worktree_regroup_rebuild_shifted_routes() {
    let mut primary = package("/z-primary");
    primary.worktree_status = WorktreeStatus::Primary {
        root: primary.path.clone(),
    };
    let mut linked = package("/linked");
    linked.worktree_status = WorktreeStatus::Linked {
        primary: primary.path.clone(),
    };
    let mut list = ProjectList::new(vec![
        RootItem::Rust(RustProject::Package(primary)),
        RootItem::Rust(RustProject::Package(linked)),
    ]);
    list.insert_into_hierarchy(RootItem::Rust(RustProject::Package(package("/a-first"))));
    assert_matches_linear(&list, &["/a-first", "/z-primary", "/linked"]);
    list.regroup_top_level_worktrees();
    assert_eq!(list.len(), 2);
    assert_matches_linear(&list, &["/a-first", "/z-primary", "/linked"]);
    let linked_entry = list
        .entry_containing(Path::new("/linked"))
        .expect("linked owner");
    assert_eq!(
        linked_entry.root_item.path().as_path(),
        Path::new("/z-primary")
    );
    assert!(matches!(
        list.worktree_status_for(Path::new("/linked")),
        Some(WorktreeStatus::Linked { .. })
    ));
}

#[test]
fn direct_root_precedes_a_duplicate_descendant_during_rescan() {
    let mut workspace = Workspace {
        path: AbsolutePath::from("/workspace"),
        ..Workspace::default()
    };
    workspace.groups.push(MemberGroup::Inline {
        members: vec![package("/workspace/member")],
    });
    let mut root = package("/workspace/member");
    root.project_info.disk_usage_bytes = Some(42);
    let list = ProjectList::new(vec![
        RootItem::Rust(RustProject::Workspace(workspace)),
        RootItem::Rust(RustProject::Package(root)),
    ]);
    assert_eq!(
        list.at_path(Path::new("/workspace/member"))
            .expect("direct root")
            .disk_usage_bytes,
        Some(42)
    );
}

#[test]
fn index_routes_linked_members_vendored_and_submodules_mutably() {
    let mut primary = package("/primary");
    primary.project_info.submodules.push(Submodule {
        name:          "child".into(),
        path:          AbsolutePath::from("/primary/child"),
        relative_path: "child".into(),
        url:           None,
        branch:        None,
        commit:        PinnedCommit::Absent,
        project_info:  ProjectInfo::default(),
        git_repo:      None,
    });
    primary.vendored.push(VendoredPackage {
        path: AbsolutePath::from("/primary/vendor/dep"),
        ..VendoredPackage::default()
    });
    let mut linked = Workspace {
        path: AbsolutePath::from("/linked"),
        ..Workspace::default()
    };
    linked.groups.push(MemberGroup::Named {
        name:    "crates".into(),
        members: vec![package("/linked/crates/member")],
    });
    let group = WorktreeGroup::new(
        RustProject::Package(primary),
        vec![RustProject::Workspace(linked)],
    );
    let mut list = ProjectList::new(vec![RootItem::Worktrees(group)]);
    list.insert_into_hierarchy(RootItem::NonRust(crate::project::NonRustProject::new(
        AbsolutePath::from("/non-rust"),
        None,
    )));
    let paths = [
        "/primary",
        "/primary/vendor/dep",
        "/linked",
        "/linked/crates/member",
        "/primary/child",
        "/non-rust",
    ];
    assert_matches_linear(&list, &paths);
    for (index, path) in paths.iter().enumerate() {
        list.at_path_mut(Path::new(path))
            .expect("mutable indexed node")
            .disk_usage_bytes = Some(index as u64 + 10);
        assert_eq!(
            list.at_path(Path::new(path))
                .expect("same indexed node")
                .disk_usage_bytes,
            Some(index as u64 + 10)
        );
    }
    assert!(std::ptr::eq(
        list.rust_info_at_path(Path::new(paths[3]))
            .expect("member rust info"),
        list.iter()
            .find_map(|entry| entry.root_item.rust_info_at_path(Path::new(paths[3])))
            .expect("linear rust info")
    ));
    assert!(std::ptr::eq(
        list.vendored_at_path(Path::new(paths[1]))
            .expect("vendored info"),
        list.iter()
            .find_map(|entry| entry.root_item.vendored_at_path(Path::new(paths[1])))
            .expect("linear vendored info")
    ));
    assert!(std::ptr::eq(
        list.lint_at_path(Path::new(paths[3]))
            .expect("workspace lint"),
        list.iter()
            .find_map(|entry| entry.root_item.lint_at_path(Path::new(paths[3])))
            .expect("linear lint")
    ));
    assert!(list.at_path(Path::new("/unknown")).is_none());
    list.clear();
    assert!(list.at_path(Path::new(paths[3])).is_none());
}

#[test]
fn workspace_member_retains_its_submodule_repository_role() {
    let member_path = AbsolutePath::from("/workspace/member");
    let mut workspace = Workspace {
        path: AbsolutePath::from("/workspace"),
        ..Workspace::default()
    };
    workspace.submodules.push(Submodule {
        name:          "member".into(),
        path:          member_path.clone(),
        relative_path: "member".into(),
        url:           None,
        branch:        None,
        commit:        PinnedCommit::Absent,
        project_info:  ProjectInfo::default(),
        git_repo:      None,
    });
    // App startup builds member groups from flat discovery results.
    let mut app = test_support::make_app(&[
        RootItem::Rust(RustProject::Workspace(workspace)),
        RootItem::Rust(RustProject::Package(package("/workspace/member"))),
    ]);
    assert_matches_linear(&app.project_list, &["/workspace/member"]);
    assert!(app.project_list.is_workspace_member_path(&member_path));
    assert!(app.project_list.is_submodule_path(&member_path));
    let info = RepoInfo {
        last_fetched: Some("child-fetch".into()),
        ..crate::project::RepoInfo::default()
    };
    app.background
        .background_sender()
        .send(BackgroundMsg::RepoInfo {
            path: member_path.clone(),
            info,
        })
        .expect("queue repository reply");
    app.poll_background();
    assert_eq!(
        app.project_list
            .repo_info_for(&member_path)
            .expect("submodule repository retained")
            .last_fetched
            .as_deref(),
        Some("child-fetch")
    );
    assert!(
        app.project_list
            .repo_info_for(Path::new("/workspace"))
            .is_none()
    );
    let child = &app.project_list[0].root_item.submodules()[0];
    assert!(std::ptr::eq(
        app.project_list
            .git_repo_for(&member_path)
            .expect("indexed repository"),
        child.git_repo.as_ref().expect("child repository")
    ));
    assert!(app.project_list.rust_info_at_path(&member_path).is_some());
}
