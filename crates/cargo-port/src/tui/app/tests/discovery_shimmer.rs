use std::time::Duration;
use std::time::Instant;

use super::*;

#[test]
fn discovery_shimmer_is_not_registered_before_scan_completes() {
    let mut app = make_app(&[]);

    assert!(app.handle_project_discovered(make_project(Some("demo"), "~/rust/demo",)));
    assert!(app.scan_mut().discovery_shimmers_mut().is_empty());
}

#[test]
fn discovery_shimmer_registers_and_allows_multiple_concurrent_roots() {
    let mut app = make_app(&[]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(app.handle_project_discovered(make_project(Some("alpha"), "~/rust/alpha",)));
    assert!(app.handle_project_discovered(make_project(Some("beta"), "~/rust/beta",)));

    assert!(
        app.scan()
            .discovery_shimmers()
            .contains_key(test_path("~/rust/alpha").as_path())
    );
    assert!(
        app.scan()
            .discovery_shimmers()
            .contains_key(test_path("~/rust/beta").as_path())
    );
    assert_eq!(app.scan_mut().discovery_shimmers_mut().len(), 2);
}

#[test]
fn expanded_workspace_members_use_the_parent_shimmer_owner() {
    let member = make_member(Some("crate_a"), "~/rust/ws/crates/crate_a");
    let workspace = make_workspace_with_members(
        Some("ws"),
        "~/rust/ws",
        vec![inline_group(vec![member.clone()])],
    );
    let mut app = make_app(&[]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(app.handle_project_discovered(workspace));
    app.panes_mut().project_list_mut().viewport_mut().set_pos(0);
    assert!(app.expand());
    app.ensure_visible_rows_cached();

    assert!(
        app.visible_rows().iter().any(|row| matches!(
            row,
            VisibleRow::Member {
                node_index:   0,
                group_index:  0,
                member_index: 0,
            }
        )),
        "expanded workspace should render its member row during an active shimmer"
    );
    assert!(
        app.scan()
            .discovery_shimmers()
            .contains_key(test_path("~/rust/ws").as_path()),
        "expanded member shimmer should be owned by the parent workspace session"
    );
    assert!(
        app.discovery_name_segments_for_path(
            member.path(),
            "crate_a",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_some(),
        "member should inherit the parent shimmer while expanded and active"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/ws").as_path(),
            "ws",
            Some(GitStatus::Clean),
            DiscoveryRowKind::Root,
        )
        .is_some(),
        "parent workspace row should also shimmer while the discovered member is active"
    );

    assert!(app.collapse());
    app.ensure_visible_rows_cached();
    assert!(
        !app.visible_rows()
            .iter()
            .any(|row| matches!(row, VisibleRow::Member { .. })),
        "collapsed workspace should stop rendering member rows"
    );
}

#[test]
fn newly_discovered_member_keeps_its_own_shimmer_owner() {
    let workspace =
        make_workspace_with_members(Some("ws"), "~/rust/ws", vec![inline_group(vec![])]);
    let mut app = make_app(&[workspace]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    let member_path = test_path("~/rust/ws/crates/crate_a");
    assert!(
        app.handle_project_discovered(make_project(Some("crate_a"), "~/rust/ws/crates/crate_a",))
    );

    assert!(
        app.scan_mut()
            .discovery_shimmers_mut()
            .contains_key(member_path.as_path()),
        "newly discovered member should keep its own shimmer session"
    );
}

#[test]
fn discovered_workspace_member_shimmers_parent_and_self_but_not_siblings() {
    let workspace = make_workspace_with_members(
        Some("ws"),
        "~/rust/ws",
        vec![inline_group(vec![make_member(
            Some("crate_existing"),
            "~/rust/ws/crates/crate_existing",
        )])],
    );
    let mut app = make_app(&[workspace]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(
        app.handle_project_discovered(RootItem::Rust(RustProject::Package(
            make_package_with_vendored(
                Some("crate_new"),
                "~/rust/ws/crates/crate_new",
                vec![super::make_vendored(
                    Some("helper_new"),
                    "~/rust/ws/crates/crate_new/vendor/helper_new",
                )],
            )
        )))
    );

    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/ws").as_path(),
            "ws",
            Some(GitStatus::Clean),
            DiscoveryRowKind::Root,
        )
        .is_some()
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/ws/crates/crate_new").as_path(),
            "crate_new",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_some()
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/ws/crates/crate_new/vendor/helper_new").as_path(),
            "helper_new",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_some(),
        "children of the discovered member should inherit the shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/ws/crates/crate_existing").as_path(),
            "crate_existing",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_none(),
        "existing siblings should not inherit the discovered member shimmer"
    );
}

#[test]
fn discovered_linked_worktree_shimmers_parent_and_subtree_but_not_existing_sibling() {
    let primary = make_workspace_raw_with_primary(
        Some("app"),
        "~/rust/app",
        Vec::new(),
        None,
        Some("/canonical/app"),
    );
    let linked = make_workspace_raw_with_primary(
        Some("app_feat"),
        "~/rust/app_feat",
        vec![inline_group(vec![make_member(
            Some("crate_a"),
            "~/rust/app_feat/crates/crate_a",
        )])],
        Some("app_feat"),
        Some("/canonical/app"),
    );
    let primary_item = RootItem::Rust(RustProject::Workspace(primary));
    let mut app = make_app(&[primary_item]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(app.handle_project_discovered(RootItem::Rust(RustProject::Workspace(linked))));

    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app").as_path(),
            "app",
            Some(GitStatus::Clean),
            DiscoveryRowKind::Root,
        )
        .is_some(),
        "top-level parent row should shimmer for a discovered linked worktree"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app_feat").as_path(),
            "app_feat",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_some(),
        "discovered linked worktree row should shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app_feat/crates/crate_a").as_path(),
            "crate_a",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_some(),
        "children of the discovered linked worktree should shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app").as_path(),
            "app",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_none(),
        "existing sibling worktree entry should not shimmer"
    );
}

#[test]
fn discovered_package_worktree_shimmers_parent_and_self_but_not_existing_sibling() {
    let root = make_package_worktrees_item(
        make_package_raw_with_primary(
            Some("cargo-port"),
            "~/rust/cargo-port",
            None,
            Some("/canonical/cargo-port"),
        ),
        vec![make_package_raw_with_primary(
            Some("cargo-port"),
            "~/rust/cargo-port-feat",
            Some("cargo-port-feat"),
            Some("/canonical/cargo-port"),
        )],
    );
    let mut app = make_app(&[root]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(
        app.handle_project_discovered(RootItem::Rust(RustProject::Package(
            make_package_raw_with_primary(
                Some("cargo-port"),
                "~/rust/cargo-port-test",
                Some("cargo-port-test"),
                Some("/canonical/cargo-port"),
            )
        )))
    );

    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/cargo-port").as_path(),
            "cargo-port",
            Some(GitStatus::Clean),
            DiscoveryRowKind::Root,
        )
        .is_some(),
        "top-level parent row should shimmer for a discovered package worktree"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/cargo-port-test").as_path(),
            "cargo-port-test",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_some(),
        "discovered package worktree row should shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/cargo-port-feat").as_path(),
            "cargo-port-feat",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_none(),
        "existing sibling worktree should not shimmer"
    );
}

#[test]
fn refreshed_stale_package_worktree_keeps_shimmer_after_regroup() {
    let root = make_package_worktrees_item(
        make_package_raw_with_primary(
            Some("cargo-port"),
            "~/rust/cargo-port",
            None,
            Some("/canonical/cargo-port"),
        ),
        vec![make_package_raw_with_primary(
            Some("cargo-port"),
            "~/rust/cargo-port-feat",
            Some("cargo-port-feat"),
            Some("/canonical/cargo-port"),
        )],
    );
    let mut app = make_app(&[root]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(
        app.handle_project_discovered(make_project(Some("cargo-port"), "~/rust/cargo-port-test",))
    );
    assert!(
        app.handle_project_refreshed(RootItem::Rust(RustProject::Package(
            make_package_raw_with_primary(
                Some("cargo-port"),
                "~/rust/cargo-port-test",
                Some("cargo-port-test"),
                Some("/canonical/cargo-port"),
            )
        )))
    );

    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/cargo-port-test").as_path(),
            "cargo-port-test",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_some(),
        "refreshed package worktree should keep its discovery shimmer after regroup"
    );
}

#[test]
fn discovered_worktree_member_shimmers_parent_self_and_children_but_not_siblings() {
    let root = make_workspace_worktrees_item(
        make_workspace_raw_with_primary(
            Some("app"),
            "~/rust/app",
            vec![inline_group(vec![make_member(
                Some("crate_root"),
                "~/rust/app/crates/crate_root",
            )])],
            None,
            Some("/canonical/app"),
        ),
        vec![make_workspace_raw_with_primary(
            Some("app_feat"),
            "~/rust/app_feat",
            vec![inline_group(vec![make_member(
                Some("crate_existing"),
                "~/rust/app_feat/crates/crate_existing",
            )])],
            Some("app_feat"),
            Some("/canonical/app"),
        )],
    );
    let mut app = make_app(&[root]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert!(
        app.handle_project_discovered(RootItem::Rust(RustProject::Package(
            make_package_with_vendored(
                Some("crate_new"),
                "~/rust/app_feat/crates/crate_new",
                vec![super::make_vendored(
                    Some("helper_new"),
                    "~/rust/app_feat/crates/crate_new/vendor/helper_new",
                )],
            )
        )))
    );

    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app_feat").as_path(),
            "app_feat",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_some(),
        "the containing worktree entry should shimmer as the discovered member's parent"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app_feat/crates/crate_new").as_path(),
            "crate_new",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_some(),
        "the discovered member should shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app_feat/crates/crate_new/vendor/helper_new").as_path(),
            "helper_new",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_some(),
        "children of the discovered member should shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app_feat/crates/crate_existing").as_path(),
            "crate_existing",
            Some(GitStatus::Clean),
            DiscoveryRowKind::PathOnly,
        )
        .is_none(),
        "existing siblings in the same worktree should not shimmer"
    );
    assert!(
        app.discovery_name_segments_for_path(
            test_path("~/rust/app").as_path(),
            "app",
            Some(GitStatus::Clean),
            DiscoveryRowKind::WorktreeEntry,
        )
        .is_none(),
        "peer worktree entries should not shimmer"
    );
}

#[test]
fn prune_discovery_shimmers_removes_expired_entries() {
    let mut app = make_app(&[]);
    let path = test_path("~/rust/demo");
    app.scan_mut().discovery_shimmers_mut().insert(
        crate::project::AbsolutePath::from(path.as_path()),
        DiscoveryShimmer::new(
            Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(Instant::now),
            Duration::from_secs(1),
        ),
    );

    app.scan_mut().prune_shimmers(Instant::now());

    assert!(
        !app.scan_mut()
            .discovery_shimmers_mut()
            .contains_key(path.as_path())
    );
}
