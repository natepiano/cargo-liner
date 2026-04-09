use super::*;

#[test]
fn detail_cache_separates_root_and_worktree_rows_with_same_path() {
    let primary_ws = make_workspace_raw(
        None,
        "~/ws",
        vec![inline_group(vec![make_member(Some("a"), "~/ws/a")])],
        None,
    );
    let linked_ws = make_workspace_raw(
        None,
        "~/ws_feat",
        vec![inline_group(vec![make_member(Some("b"), "~/ws_feat/b")])],
        Some("ws_feat"),
    );
    let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);

    let mut app = make_app(&[make_workspace_project(None, "~/ws")]);
    app.current_config.lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();

    app.lint_status.insert(
        test_path("~/ws"),
        LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
    );
    app.lint_status.insert(
        test_path("~/ws_feat"),
        LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
    );
    app.rebuild_lint_rollups();

    app.list_state.select(Some(0));
    app.sync_selected_project();
    app.ensure_detail_cached();
    assert_eq!(
        app.cached_detail
            .as_ref()
            .map(|cache| cache.info.lint_label.as_str()),
        Some("🔴")
    );

    app.list_state.select(Some(1));
    app.sync_selected_project();
    app.ensure_detail_cached();
    assert_eq!(
        app.cached_detail
            .as_ref()
            .map(|cache| cache.info.lint_label.as_str()),
        Some("🟢")
    );
}

#[test]
fn linked_worktree_entry_builds_detail_for_selected_row() {
    let primary_ws = make_workspace_raw(
        Some("cargo-port"),
        "~/rust/cargo-port",
        vec![inline_group(vec![make_member(
            Some("cargo-port"),
            "~/rust/cargo-port/crates/cargo-port",
        )])],
        None,
    );
    let linked_ws = make_workspace_raw_with_primary(
        Some("cargo-port_speedup"),
        "~/rust/cargo-port_speedup",
        vec![inline_group(vec![make_member(
            Some("cargo-port"),
            "~/rust/cargo-port_speedup/crates/cargo-port",
        )])],
        Some("cargo-port_speedup"),
        Some("~/rust/cargo-port"),
    );
    let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws.clone()]);

    let mut app = make_app(&[]);
    apply_items(&mut app, &[root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();

    assert_eq!(
        app.visible_rows(),
        vec![
            VisibleRow::Root { node_index: 0 },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 0,
            },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 1,
            },
        ]
    );

    app.list_state.select(Some(2));
    app.sync_selected_project();
    app.ensure_detail_cached();

    assert_eq!(
        app.selected_project_path().map(Path::to_path_buf),
        Some(linked_ws.path().to_path_buf())
    );
    assert_eq!(
        app.cached_detail
            .as_ref()
            .map(|cache| cache.info.path.as_str()),
        Some("~/rust/cargo-port_speedup")
    );
    assert!(
        app.tabbable_panes().contains(&PaneId::Package),
        "linked worktree selection should expose the package pane"
    );
}

#[test]
fn disk_rollup_deduplicates_primary_worktree_path() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    apply_items(&mut app, &[root]);
    app.handle_disk_usage(test_path("~/ws").as_path(), 15);
    app.handle_disk_usage(test_path("~/ws_feat").as_path(), 21);

    assert_eq!(app.projects[0].disk_usage_bytes(), Some(36));
    assert_eq!(
        App::formatted_disk_for_item(&app.projects[0]),
        crate::tui::render::format_bytes(36)
    );
}

#[test]
fn handle_project_discovered_deduplicates_by_path() {
    let mut app = make_app(&[]);

    let pkg1 = RootItem::Package(make_package_raw(Some("foo"), "/abs/foo", None));
    let pkg2 = RootItem::Package(make_package_raw(Some("foo"), "/abs/foo", None));
    let pkg3 = RootItem::Package(make_package_raw(Some("bar"), "/abs/bar", None));

    app.handle_project_discovered(pkg1);
    app.handle_project_discovered(pkg2);
    app.handle_project_discovered(pkg3);
    assert_eq!(app.projects.len(), 2);
}

#[test]
fn handle_project_discovered_creates_worktree_group_from_single_primary() {
    expect_synthetic_discovery_creates_group(WorktreeProjectKind::Package);
}

#[test]
fn handle_project_discovered_slots_new_worktree_into_existing_group() {
    expect_synthetic_discovery_appends_existing_group(WorktreeProjectKind::Package);
}

#[test]
fn handle_project_discovered_creates_workspace_worktree_group_from_single_primary() {
    expect_synthetic_discovery_creates_group(WorktreeProjectKind::Workspace);
}

#[test]
fn handle_project_discovered_slots_new_workspace_worktree_into_existing_group() {
    expect_synthetic_discovery_appends_existing_group(WorktreeProjectKind::Workspace);
}

#[test]
fn background_discovery_from_real_package_worktree_creates_group() {
    expect_real_discovery_creates_group(WorktreeProjectKind::Package);
}

#[test]
fn background_discovery_from_real_workspace_worktree_creates_group() {
    expect_real_discovery_creates_group(WorktreeProjectKind::Workspace);
}

#[test]
fn discovered_workspace_worktree_with_members_expands_as_worktree_then_workspace() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_test");
    init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/brp");
    let linked_item =
        crate::scan::discover_project_item(&linked_dir).unwrap_or_else(|| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered { item: linked_item },
    );

    let RootItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected discovered workspace worktree to form a worktree group");
    };
    assert_eq!(group.linked().len(), 1);
    assert!(
        group.linked()[0].has_members(),
        "linked workspace worktree should arrive with member groups populated"
    );

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into worktree entries");
    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows(),
        &[
            VisibleRow::Root { node_index: 0 },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 0,
            },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 1,
            },
        ]
    );

    app.list_state.select(Some(2));
    assert!(
        app.expand(),
        "linked workspace worktree should expand into its workspace members"
    );
    app.ensure_visible_rows_cached();
    assert!(
        app.visible_rows().iter().any(|row| matches!(
            row,
            VisibleRow::WorktreeMember {
                node_index: 0,
                worktree_index: 1,
                ..
            }
        )),
        "expanded linked workspace worktree should show member rows"
    );
}

#[test]
fn expanded_workspace_root_discovery_immediately_renders_primary_workspace_and_linked_row() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_test");
    init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

    let mut primary_item = item_from_project_dir(&primary_dir);
    let RootItem::Workspace(primary_ws) = &mut primary_item else {
        panic!("expected primary workspace root item");
    };
    *primary_ws.groups_mut() = vec![inline_group(vec![make_member(
        Some("extras"),
        &primary_dir.join("extras").to_string_lossy(),
    )])];
    let mut app = make_app(&[]);
    apply_items(&mut app, &[primary_item]);

    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows(),
        &[
            VisibleRow::Root { node_index: 0 },
            VisibleRow::Member {
                node_index:   0,
                group_index:  0,
                member_index: 0,
            },
        ]
    );

    add_git_worktree(&primary_dir, &linked_dir, "test/brp");
    let linked_item =
        crate::scan::discover_project_item(&linked_dir).unwrap_or_else(|| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered { item: linked_item },
    );

    assert_eq!(
        app.visible_rows(),
        &[
            VisibleRow::Root { node_index: 0 },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 0,
            },
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 0,
                group_index:    0,
                member_index:   0,
            },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 1,
            },
        ],
        "discovering a linked workspace worktree while the primary root is expanded should preserve the primary workspace subtree immediately"
    );

    let rendered = rendered_root_name_cells(&mut app);
    assert!(
        rendered
            .iter()
            .any(|row| row.contains("bevy_brp") && row.contains(":2")),
        "root row should still render the worktree badge after discovery: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|row| row.contains("bevy_brp_test")),
        "linked worktree row should render immediately without a collapse/expand cycle: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|row| row.contains("extras")),
        "primary workspace member rows should remain visible after the root becomes a worktree group: {rendered:?}"
    );
}

#[test]
fn project_discovery_updates_cached_rows_for_expanded_workspace_immediately() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_test");
    init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

    let mut primary_item = item_from_project_dir(&primary_dir);
    let RootItem::Workspace(primary_ws) = &mut primary_item else {
        panic!("expected primary workspace root item");
    };
    *primary_ws.groups_mut() = vec![inline_group(vec![make_member(
        Some("extras"),
        &primary_dir.join("extras").to_string_lossy(),
    )])];

    let mut app = make_app(&[]);
    apply_items(&mut app, &[primary_item]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();

    add_git_worktree(&primary_dir, &linked_dir, "test/brp");
    let linked_item =
        crate::scan::discover_project_item(&linked_dir).unwrap_or_else(|| std::process::abort());

    assert!(
        app.handle_bg_msg(BackgroundMsg::ProjectDiscovered { item: linked_item }),
        "discovery should request a derived-state rebuild"
    );

    assert_eq!(
        app.visible_rows(),
        &[
            VisibleRow::Root { node_index: 0 },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 0,
            },
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 0,
                group_index:    0,
                member_index:   0,
            },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 1,
            },
        ],
        "cached visible rows should switch to worktree rows immediately after discovery"
    );
}

#[test]
fn stale_workspace_regroup_immediately_renders_primary_workspace_and_linked_row() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_test");
    init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

    let mut primary_item = item_from_project_dir(&primary_dir);
    let RootItem::Workspace(primary_ws) = &mut primary_item else {
        panic!("expected primary workspace root item");
    };
    *primary_ws.groups_mut() = vec![inline_group(vec![make_member(
        Some("extras"),
        &primary_dir.join("extras").to_string_lossy(),
    )])];
    let mut app = make_app(&[]);
    apply_items(&mut app, &[primary_item]);

    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows(),
        &[
            VisibleRow::Root { node_index: 0 },
            VisibleRow::Member {
                node_index:   0,
                group_index:  0,
                member_index: 0,
            },
        ]
    );

    add_git_worktree(&primary_dir, &linked_dir, "test/brp");
    let stale_discovery = RootItem::Workspace(make_workspace_raw(
        Some("bevy_brp"),
        &linked_dir.to_string_lossy(),
        Vec::new(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );

    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    assert_eq!(
        app.visible_rows(),
        &[
            VisibleRow::Root { node_index: 0 },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 0,
            },
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 0,
                group_index:    0,
                member_index:   0,
            },
            VisibleRow::WorktreeEntry {
                node_index:     0,
                worktree_index: 1,
            },
        ],
        "refresh regroup should preserve the expanded primary workspace subtree immediately"
    );

    let rendered = rendered_root_name_cells(&mut app);
    assert!(
        rendered.iter().any(|row| row.contains("bevy_brp_test")),
        "regrouped linked worktree row should render immediately without a collapse/expand cycle: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|row| row.contains("extras")),
        "regrouped primary workspace member rows should remain visible: {rendered:?}"
    );
}

#[test]
fn background_discovery_from_real_package_worktree_appends_existing_group() {
    expect_real_discovery_appends_existing_group(WorktreeProjectKind::Package);
}

#[test]
fn background_discovery_from_real_workspace_worktree_appends_existing_group() {
    expect_real_discovery_appends_existing_group(WorktreeProjectKind::Workspace);
}

#[test]
fn refreshed_workspace_worktree_metadata_regroups_stale_top_level_discovery() {
    expect_refresh_regroups_stale_top_level_discovery(WorktreeProjectKind::Workspace);
}

#[test]
fn refreshed_package_worktree_metadata_regroups_stale_top_level_discovery() {
    expect_refresh_regroups_stale_top_level_discovery(WorktreeProjectKind::Package);
}

#[test]
fn refreshed_workspace_worktree_metadata_appends_into_existing_group() {
    expect_refresh_appends_stale_discovery_into_existing_group(WorktreeProjectKind::Workspace);
}

#[test]
fn refreshed_package_worktree_metadata_appends_into_existing_group() {
    expect_refresh_appends_stale_discovery_into_existing_group(WorktreeProjectKind::Package);
}

#[test]
fn stale_discovery_refresh_then_delete_dismiss_workspace_returns_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

    let stale_discovery = RootItem::Workspace(make_workspace_raw(
        Some("obsidian_knife"),
        &linked_dir.to_string_lossy(),
        Vec::new(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );
    assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
}

#[test]
fn stale_discovery_refresh_then_delete_dismiss_package_returns_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/app");

    let stale_discovery = RootItem::Package(make_package_raw(
        Some("app"),
        &linked_dir.to_string_lossy(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );
    assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
}

#[test]
fn background_disk_zero_from_real_package_worktree_can_be_dismissed_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);
    add_git_worktree(&primary_dir, &linked_dir, "test/app");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_item = item_from_project_dir(&linked_dir);
    let mut app = make_app(&[primary_item, linked_item]);

    assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
}

#[test]
fn background_disk_zero_from_real_workspace_worktree_can_be_dismissed_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);
    add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_item = item_from_project_dir(&linked_dir);
    let mut app = make_app(&[primary_item, linked_item]);

    assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
}

#[test]
fn handle_project_discovered_does_not_allocate_per_comparison() {
    let mut app = make_app(&[]);
    let start = std::time::Instant::now();
    for i in 0..200 {
        let path = format!("/abs/project_{i}");
        let item = RootItem::Package(make_package_raw(None, &path, None));
        app.handle_project_discovered(item);
    }
    let elapsed = start.elapsed();
    assert_eq!(app.projects.len(), 200);
    assert!(
        elapsed.as_millis() < 100,
        "discovery of 200 projects took {elapsed:?} - possible display_path allocation regression"
    );
}

#[test]
fn is_deleted_does_not_allocate_display_paths() {
    let mut app = make_app(&[]);
    for i in 0..200 {
        let path = format!("/abs/project_{i}");
        let item = RootItem::Package(make_package_raw(None, &path, None));
        app.projects.push(item);
    }
    let target = app.projects[100].path().to_path_buf();
    app.projects
        .at_path_mut(&target)
        .expect("target project should exist")
        .visibility = Deleted;
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = app.is_deleted(&target);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 100,
        "1000 is_deleted calls took {elapsed:?} -- possible display_path allocation regression"
    );
}
