use super::*;

#[test]
fn visible_rows_workspace_with_worktrees() {
    let member_a = make_member(Some("a"), "~/ws/a");
    let member_b = make_member(Some("b"), "~/ws/b");

    let primary = make_workspace_raw(
        None,
        "~/ws",
        vec![inline_group(vec![member_a.clone(), member_b.clone()])],
        None,
    );
    let linked = make_workspace_raw(
        None,
        "~/ws_feat",
        vec![named_group("crates", vec![member_a, member_b])],
        Some("ws_feat"),
    );
    let root = make_workspace_worktrees_item(primary, vec![linked]);

    let expanded: HashSet<ExpandKey> = [
        ExpandKey::Node(0),
        ExpandKey::Worktree(0, 0),
        ExpandKey::Worktree(0, 1),
        ExpandKey::WorktreeGroup(0, 1, 0),
    ]
    .into();

    let rows = snapshots::build_visible_rows(&[root], &expanded, true);

    assert_eq!(rows.len(), 8, "expected 8 rows, got: {rows:?}");
    assert!(matches!(rows[0], VisibleRow::Root { node_index: 0 }));
    assert!(matches!(
        rows[1],
        VisibleRow::WorktreeEntry {
            node_index:     0,
            worktree_index: 0,
        }
    ));
    assert!(matches!(
        rows[2],
        VisibleRow::WorktreeMember {
            node_index:     0,
            worktree_index: 0,
            group_index:    0,
            member_index:   0,
        }
    ));
    assert!(matches!(
        rows[4],
        VisibleRow::WorktreeEntry {
            node_index:     0,
            worktree_index: 1,
        }
    ));
    assert!(matches!(
        rows[5],
        VisibleRow::WorktreeGroupHeader {
            node_index:     0,
            worktree_index: 1,
            group_index:    0,
        }
    ));
    assert!(matches!(
        rows[7],
        VisibleRow::WorktreeMember {
            node_index:     0,
            worktree_index: 1,
            group_index:    0,
            member_index:   1,
        }
    ));
}

#[test]
fn expand_linked_workspace_worktree_renders_its_members() {
    let member_a = make_member(Some("a"), "~/ws/a");
    let member_b = make_member(Some("b"), "~/ws/b");

    let primary = make_workspace_raw(
        None,
        "~/ws",
        vec![inline_group(vec![member_a.clone(), member_b.clone()])],
        None,
    );
    let linked = make_workspace_raw(
        None,
        "~/ws_feat",
        vec![named_group("crates", vec![member_a, member_b])],
        Some("ws_feat"),
    );
    let root = make_workspace_worktrees_item(primary, vec![linked]);
    let mut app = make_app(&[root]);

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows(),
        &[VisibleRow::Root { node_index: 0 }],
        "workspace worktree group should start collapsed"
    );

    assert!(app.expand(), "root workspace worktree group should expand");
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
        ],
        "expanding the root should show primary and linked worktree rows"
    );

    app.list_state.select(Some(2));
    assert!(app.expand(), "linked workspace worktree row should expand");
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
            VisibleRow::WorktreeGroupHeader {
                node_index:     0,
                worktree_index: 1,
                group_index:    0,
            },
        ],
        "expanding the linked workspace worktree should show its member group"
    );

    app.list_state.select(Some(3));
    assert!(app.expand(), "linked workspace member group should expand");
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
            VisibleRow::WorktreeGroupHeader {
                node_index:     0,
                worktree_index: 1,
                group_index:    0,
            },
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 1,
                group_index:    0,
                member_index:   0,
            },
            VisibleRow::WorktreeMember {
                node_index:     0,
                worktree_index: 1,
                group_index:    0,
                member_index:   1,
            },
        ],
        "expanding the linked workspace group should render its members"
    );
}

#[test]
fn visible_rows_non_workspace_worktrees() {
    let build_root = || {
        make_package_worktrees_item(
            make_package_raw(Some("app"), "~/app", None),
            vec![make_package_raw(
                Some("app"),
                "~/app_feat",
                Some("app_feat"),
            )],
        )
    };

    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&[build_root()], &expanded, true);

    assert_eq!(rows.len(), 3, "got: {rows:?}");
    assert!(matches!(rows[0], VisibleRow::Root { .. }));
    assert!(matches!(rows[1], VisibleRow::WorktreeEntry { .. }));
    assert!(matches!(rows[2], VisibleRow::WorktreeEntry { .. }));

    let expanded2: HashSet<ExpandKey> = [ExpandKey::Node(0), ExpandKey::Worktree(0, 0)].into();
    let rows2 = snapshots::build_visible_rows(&[build_root()], &expanded2, true);
    assert_eq!(rows2.len(), 3, "no extra rows for non-workspace worktree");
}

#[test]
fn worktree_section_collapses_when_one_dismissed() {
    let root = make_package_worktrees_item(
        make_package_raw(Some("app"), "~/app", None),
        vec![make_package_raw(
            Some("app"),
            "~/app_feat",
            Some("app_feat"),
        )],
    );

    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();

    let items = vec![root.clone()];
    let rows = snapshots::build_visible_rows(&items, &expanded, true);
    assert_eq!(rows.len(), 3, "root + 2 worktree entries");

    let mut items = vec![root];
    let linked_path = match &items[0] {
        RootItem::Worktrees(WorktreeGroup::Packages { linked, .. }) => {
            linked[0].path().to_path_buf()
        },
        _ => unreachable!("expected package worktrees"),
    };
    items[0]
        .at_path_mut(&linked_path)
        .expect("linked worktree should exist")
        .visibility = Dismissed;
    let rows = snapshots::build_visible_rows(&items, &expanded, true);
    assert_eq!(
        rows.len(),
        1,
        "only the root should remain when one worktree is left"
    );
    assert_eq!(rows, vec![VisibleRow::Root { node_index: 0 }]);
}

#[test]
fn dismissing_deleted_linked_worktree_promotes_primary_back_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_feat");
    std::fs::create_dir_all(&primary_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());

    let primary_path = primary_dir.to_string_lossy().to_string();
    let linked_path = linked_dir.to_string_lossy().to_string();
    let root = make_package_worktrees_item(
        make_package_raw(Some("app"), &primary_path, None),
        vec![make_package_raw(
            Some("app"),
            &linked_path,
            Some("app_feat"),
        )],
    );
    let mut app = make_app(&[root]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root worktree group should expand");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3, "root + 2 worktree entries");

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    app.handle_disk_usage(Path::new(&linked_path), 0);

    let linked_abs = AbsolutePath::from(linked_path.clone());
    assert!(
        app.is_deleted(&linked_abs),
        "linked worktree should be deleted"
    );

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows().len(),
        3,
        "deleted worktree should still render until dismissed"
    );

    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked worktree should be dismissable");
    app.dismiss(target);

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows(),
        &[VisibleRow::Root { node_index: 0 }],
        "dismissing the deleted worktree should collapse the group to the root row"
    );
    assert_eq!(
        match &app.projects[0] {
            RootItem::Worktrees(wtg @ WorktreeGroup::Packages { .. }) => {
                assert_eq!(wtg.live_entry_count(), 1);
                usize::from(wtg.renders_as_group())
            },
            RootItem::Rust(_)
            | RootItem::NonRust(_)
            | RootItem::Worktrees(WorktreeGroup::Workspaces { .. }) => 0,
        },
        0,
        "the remaining primary should no longer render as a worktree group"
    );
    assert_eq!(
        app.selected_project_path(),
        Some(Path::new(&primary_path)),
        "selection should move back to the surviving top-level project"
    );
    assert_eq!(
        app.projects
            .at_path(&linked_abs)
            .expect("linked worktree should remain in the hierarchy")
            .visibility,
        Dismissed
    );
}

#[test]
fn dismissing_deleted_linked_workspace_worktree_promotes_primary_back_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("ws");
    let linked_dir = tmp.path().join("ws_feat");
    std::fs::create_dir_all(&primary_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());

    let primary_path = primary_dir.to_string_lossy().to_string();
    let linked_path = linked_dir.to_string_lossy().to_string();
    let root = make_workspace_worktrees_item(
        make_workspace_raw(Some("ws"), &primary_path, Vec::new(), None),
        vec![make_workspace_raw(
            Some("ws"),
            &linked_path,
            Vec::new(),
            Some("ws_feat"),
        )],
    );
    let mut app = make_app(&[root]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root worktree group should expand");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3, "root + 2 worktree entries");

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(linked_path.clone()),
            bytes: 0,
        },
    );

    let linked_abs = AbsolutePath::from(linked_path);
    assert!(
        app.is_deleted(&linked_abs),
        "linked workspace should be deleted"
    );
    assert_eq!(
        app.visible_rows().len(),
        3,
        "deleted linked workspace should still render until dismissed"
    );

    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked workspace should be dismissable");
    app.dismiss(target);
    app.ensure_visible_rows_cached();

    assert_eq!(
        app.visible_rows(),
        &[VisibleRow::Root { node_index: 0 }],
        "dismissing the deleted workspace worktree should collapse to the root row"
    );
    assert_eq!(
        match &app.projects[0] {
            RootItem::Worktrees(wtg @ WorktreeGroup::Workspaces { .. }) => {
                assert_eq!(wtg.live_entry_count(), 1);
                usize::from(wtg.renders_as_group())
            },
            RootItem::Rust(_)
            | RootItem::NonRust(_)
            | RootItem::Worktrees(WorktreeGroup::Packages { .. }) => 0,
        },
        0,
        "the remaining primary should no longer render as a worktree group"
    );
}

#[test]
fn dismissing_deleted_linked_workspace_worktree_keeps_primary_member_rows_rendered() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_style_fix");
    std::fs::create_dir_all(&primary_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());

    let primary_path = primary_dir.to_string_lossy().to_string();
    let linked_path = linked_dir.to_string_lossy().to_string();
    let primary = make_workspace_raw(
        Some("bevy_brp"),
        &primary_path,
        vec![inline_group(vec![make_member(
            Some("bevy_brp"),
            &format!("{primary_path}/crates/bevy_brp"),
        )])],
        None,
    );
    let linked = make_workspace_raw(
        Some("bevy_brp"),
        &linked_path,
        Vec::new(),
        Some("bevy_brp_style_fix"),
    );
    let root = make_workspace_worktrees_item(primary, vec![linked]);
    let mut app = make_app(&[root]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root worktree group should expand");
    app.ensure_visible_rows_cached();

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(linked_path),
            bytes: 0,
        },
    );

    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked workspace should be dismissable");
    app.dismiss(target);

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
        ],
        "expanded root should keep rendering the surviving primary workspace members"
    );

    let rendered = rendered_root_name_cells(&mut app);
    assert!(
        rendered.iter().any(|line| line.contains("bevy_brp")),
        "member row should render its name instead of blank output: {rendered:?}"
    );
}

#[test]
fn dismissing_deleted_linked_workspace_worktree_preserves_primary_member_disk_sizes() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_style_fix");
    let member_dir = primary_dir.join("crates").join("bevy_brp");
    std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());

    let primary_path = primary_dir.to_string_lossy().to_string();
    let linked_path = linked_dir.to_string_lossy().to_string();
    let member_path = member_dir.to_string_lossy().to_string();
    let primary = make_workspace_raw(
        Some("bevy_brp"),
        &primary_path,
        vec![inline_group(vec![make_member(
            Some("bevy_brp"),
            &member_path,
        )])],
        None,
    );
    let linked = make_workspace_raw(
        Some("bevy_brp"),
        &linked_path,
        Vec::new(),
        Some("bevy_brp_style_fix"),
    );
    let root = make_workspace_worktrees_item(primary, vec![linked]);
    let mut app = make_app(&[root]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root worktree group should expand");
    app.handle_disk_usage(Path::new(&primary_path), 2_000_000);
    app.handle_disk_usage(Path::new(&member_path), 1_234_567);
    assert_eq!(
        app.projects
            .at_path(Path::new(&member_path))
            .and_then(|info| info.disk_usage_bytes),
        Some(1_234_567)
    );
    app.ensure_visible_rows_cached();

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(linked_path),
            bytes: 0,
        },
    );

    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked workspace should be dismissable");
    app.dismiss(target);
    app.ensure_visible_rows_cached();

    assert_eq!(
        app.projects
            .at_path(Path::new(&member_path))
            .and_then(|info| info.disk_usage_bytes),
        Some(1_234_567),
        "member disk usage should remain stored after dismiss"
    );

    let rendered = rendered_root_name_cells(&mut app);
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("1.2 MiB") || line.contains("1.2 Mi")),
        "surviving member row should keep its disk usage after dismiss: {rendered:?}"
    );
}

#[test]
fn deleted_linked_workspace_children_render_crossed_out_before_dismiss() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_test");
    std::fs::create_dir_all(&primary_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());

    let primary_path = primary_dir.to_string_lossy().to_string();
    let linked_path = linked_dir.to_string_lossy().to_string();
    let primary = make_workspace_raw(
        Some("bevy_brp"),
        &primary_path,
        vec![inline_group(vec![make_member(
            Some("bevy_brp_extras"),
            &format!("{primary_path}/bevy_brp_extras"),
        )])],
        None,
    );
    let linked = make_workspace_raw(
        Some("bevy_brp"),
        &linked_path,
        vec![inline_group(vec![make_member(
            Some("bevy_brp_extras"),
            &format!("{linked_path}/bevy_brp_extras"),
        )])],
        Some("bevy_brp_test"),
    );
    let root = make_workspace_worktrees_item(primary, vec![linked]);
    let mut app = make_app(&[root]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root worktree group should expand");
    app.ensure_visible_rows_cached();
    app.list_state.select(Some(2));
    assert!(app.expand(), "linked worktree row should expand");
    app.ensure_visible_rows_cached();

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(linked_path.clone()),
            bytes: 0,
        },
    );

    assert!(
        app.is_deleted(Path::new(&linked_path)),
        "linked workspace should be marked deleted"
    );
    assert!(
        matches!(app.visible_rows()[3], VisibleRow::WorktreeMember { .. }),
        "expanded linked workspace member row should still be visible before dismiss"
    );

    let (buffer, widths) = render_tree_buffer(&mut app);
    assert!(
        row_has_crossed_out_content(&buffer, &widths, 2),
        "deleted linked workspace row should be crossed out"
    );
    assert!(
        row_has_crossed_out_content(&buffer, &widths, 3),
        "deleted linked workspace member row should inherit crossed-out styling"
    );
}

#[test]
fn dismissing_deleted_linked_workspace_member_dismisses_whole_worktree() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("bevy_brp");
    let linked_dir = tmp.path().join("bevy_brp_test");
    std::fs::create_dir_all(&primary_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());

    let primary_path = primary_dir.to_string_lossy().to_string();
    let linked_path = linked_dir.to_string_lossy().to_string();
    let primary = make_workspace_raw(
        Some("bevy_brp"),
        &primary_path,
        vec![inline_group(vec![make_member(
            Some("bevy_brp_extras"),
            &format!("{primary_path}/bevy_brp_extras"),
        )])],
        None,
    );
    let linked = make_workspace_raw(
        Some("bevy_brp"),
        &linked_path,
        vec![inline_group(vec![make_member(
            Some("bevy_brp_extras"),
            &format!("{linked_path}/bevy_brp_extras"),
        )])],
        Some("bevy_brp_test"),
    );
    let root = make_workspace_worktrees_item(primary, vec![linked]);
    let mut app = make_app(&[root]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root worktree group should expand");
    app.ensure_visible_rows_cached();
    app.list_state.select(Some(2));
    assert!(app.expand(), "linked worktree row should expand");
    app.ensure_visible_rows_cached();

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  AbsolutePath::from(linked_path.clone()),
            bytes: 0,
        },
    );

    app.list_state.select(Some(3));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked workspace member should dismiss its worktree");
    match &target {
        DismissTarget::DeletedProject(path) => assert_eq!(path, Path::new(&linked_path)),
        DismissTarget::Toast(_) => panic!("expected deleted project target"),
    }
    app.dismiss(target);
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
        ],
        "dismissing a deleted linked workspace member should dismiss the whole linked worktree"
    );
}

#[test]
fn worktree_count_uses_visibility() {
    let root = make_package_worktrees_item(
        make_package_raw(Some("app"), "~/app", None),
        vec![make_package_raw(
            Some("app"),
            "~/app_feat",
            Some("app_feat"),
        )],
    );

    let items = vec![root];
    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&items, &expanded, true);
    assert_eq!(rows.len(), 3, "root + 2 worktree entries");
}

#[test]
fn workspace_worktree_fit_widths_use_display_name_for_primary_entry() {
    let item = make_workspace_worktrees_item(
        make_workspace_raw(
            Some("obsidian_knife"),
            "/tmp/really/long/path/to/obsidian_knife",
            Vec::new(),
            None,
        ),
        vec![make_workspace_raw(
            Some("obsidian_knife"),
            "/tmp/really/long/path/to/obsidian_knife_test",
            Vec::new(),
            Some("obsidian_knife_test"),
        )],
    );
    let git_path_states = std::collections::HashMap::new();
    let root_label = resolved_root_label(&item);
    let widths = snapshots::build_fit_widths_snapshot(
        std::slice::from_ref(&item),
        std::slice::from_ref(&root_label),
        &snapshots::FitWidthsState {
            git_path_states: &git_path_states,
        },
        true,
        0,
    );
    let root_width = crate::tui::columns::display_width(crate::tui::render::PREFIX_ROOT_COLLAPSED)
        + crate::tui::columns::display_width(&root_label);
    let primary_entry_width =
        crate::tui::columns::display_width(crate::tui::render::PREFIX_WT_FLAT)
            + crate::tui::columns::display_width("obsidian_knife");
    let linked_entry_width = crate::tui::columns::display_width(crate::tui::render::PREFIX_WT_FLAT)
        + crate::tui::columns::display_width("obsidian_knife_test");

    assert_eq!(
        widths.get(crate::tui::columns::COL_NAME),
        App::name_width_with_gutter(root_width.max(primary_entry_width).max(linked_entry_width)),
        "fit widths should use rendered worktree labels, not the absolute primary worktree path"
    );
}

#[test]
fn package_worktree_fit_widths_use_display_name_for_primary_entry() {
    let item = make_package_worktrees_item(
        make_package_raw(
            Some("cargo-port"),
            "/tmp/really/long/path/to/cargo-port",
            None,
        ),
        vec![make_package_raw(
            Some("cargo-port"),
            "/tmp/really/long/path/to/cargo-port_test",
            Some("cargo-port_test"),
        )],
    );
    let git_path_states = std::collections::HashMap::new();
    let root_label = resolved_root_label(&item);
    let widths = snapshots::build_fit_widths_snapshot(
        std::slice::from_ref(&item),
        std::slice::from_ref(&root_label),
        &snapshots::FitWidthsState {
            git_path_states: &git_path_states,
        },
        true,
        0,
    );
    let root_width = crate::tui::columns::display_width(crate::tui::render::PREFIX_ROOT_COLLAPSED)
        + crate::tui::columns::display_width(&root_label);
    let primary_entry_width =
        crate::tui::columns::display_width(crate::tui::render::PREFIX_WT_FLAT)
            + crate::tui::columns::display_width("cargo-port");
    let linked_entry_width = crate::tui::columns::display_width(crate::tui::render::PREFIX_WT_FLAT)
        + crate::tui::columns::display_width("cargo-port_test");

    assert_eq!(
        widths.get(crate::tui::columns::COL_NAME),
        App::name_width_with_gutter(root_width.max(primary_entry_width).max(linked_entry_width)),
        "fit widths should use rendered worktree labels, not the absolute primary worktree path"
    );
}

#[test]
fn root_rows_disambiguate_same_directory_leaves_with_parent_suffix() {
    let mut app = make_app(&[
        make_project(Some("cargo-port"), "/tmp/rust/cargo-port"),
        make_project(Some("cargo-port"), "/tmp/archive/cargo-port"),
    ]);

    let names = rendered_root_name_cells(&mut app);

    assert!(
        names
            .iter()
            .any(|name| name.contains("cargo-port [rust/cargo-port]")),
        "colliding dir-leaf roots should disambiguate by parent path: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.contains("cargo-port [archive/cargo-port]")),
        "colliding dir-leaf roots should disambiguate by parent path: {names:?}"
    );
    assert_ne!(
        names[0], names[1],
        "colliding roots should render distinctly"
    );
}

#[test]
fn root_rows_extend_dir_suffix_until_same_leaf_dirs_become_unique() {
    let mut app = make_app(&[
        make_package_worktrees_item(
            make_package_raw(Some("cargo-port"), "/tmp/rust/cargo-port", None),
            vec![make_package_raw(
                Some("cargo-port"),
                "/tmp/rust/cargo-port_test",
                Some("cargo-port_test"),
            )],
        ),
        make_project(Some("cargo-port"), "/tmp/archive/cargo-port"),
    ]);

    let names = rendered_root_name_cells(&mut app);

    assert!(
        names
            .iter()
            .any(|name| name.contains("cargo-port [rust/cargo-port]")),
        "root label should prepend parents until the suffix becomes unique: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.contains("cargo-port [archive/cargo-port]")),
        "root label should prepend parents until the suffix becomes unique: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.contains(crate::constants::WORKTREE)),
        "worktree root should still render its badge after disambiguation: {names:?}"
    );
    assert_ne!(
        names[0], names[1],
        "same-name same-leaf roots should render distinctly"
    );
}

#[test]
fn visible_rows_workspace_no_worktrees() {
    let root = make_workspace_with_members(
        None,
        "~/ws",
        vec![inline_group(vec![
            make_member(Some("a"), "~/ws/a"),
            make_member(Some("b"), "~/ws/b"),
        ])],
    );

    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&[root], &expanded, true);

    assert_eq!(rows.len(), 3, "got: {rows:?}");
    assert!(matches!(rows[0], VisibleRow::Root { .. }));
    assert!(matches!(
        rows[1],
        VisibleRow::Member {
            member_index: 0,
            ..
        }
    ));
    assert!(matches!(
        rows[2],
        VisibleRow::Member {
            member_index: 1,
            ..
        }
    ));
}

#[test]
fn visible_rows_include_vendored_children() {
    let ws = WorkspaceProject::new(
        test_path("~/ws"),
        None,
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        vec![inline_group(vec![make_member(
            Some("member"),
            "~/ws/member",
        )])],
        vec![make_member(Some("vendored"), "~/ws/vendor/helper")],
        None,
        None,
    );
    let root = RootItem::Rust(RustProject::Workspace(ws));

    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&[root], &expanded, true);

    assert_eq!(rows.len(), 3, "got: {rows:?}");
    assert!(matches!(rows[0], VisibleRow::Root { .. }));
    assert!(matches!(rows[1], VisibleRow::Member { .. }));
    assert!(matches!(
        rows[2],
        VisibleRow::Vendored {
            node_index:     0,
            vendored_index: 0,
        }
    ));
}
