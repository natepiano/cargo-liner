use super::*;
use crate::project::WorktreeHealth::Normal;

#[test]
fn collapse_all_anchors_member_selection_to_root() {
    let workspace = make_workspace_project(Some("hana"), "~/rust/hana");
    let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
    let root = make_workspace_with_members(
        Some("hana"),
        "~/rust/hana",
        vec![inline_group(vec![make_member(
            Some("hana_core"),
            "~/rust/hana/crates/hana_core",
        )])],
    );

    let mut app = make_app(&[workspace, member.clone()]);
    apply_items(&mut app, &[root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.select_project_in_tree(member.path());

    app.collapse_all();

    assert_eq!(app.selected_row(), Some(VisibleRow::Root { node_index: 0 }));
}

#[test]
fn expand_all_preserves_selected_project_path() {
    let workspace = make_workspace_project(Some("hana"), "~/rust/hana");
    let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
    let root = make_workspace_with_members(
        Some("hana"),
        "~/rust/hana",
        vec![inline_group(vec![make_member(
            Some("hana_core"),
            "~/rust/hana/crates/hana_core",
        )])],
    );

    let mut app = make_app(&[workspace, member.clone()]);
    apply_items(&mut app, &[root]);
    app.select_project_in_tree(member.path());
    app.collapse_all();

    app.expand_all();

    assert_eq!(
        app.selected_display_path()
            .as_ref()
            .map(crate::project::DisplayPath::as_str),
        Some(member.display_path().as_str())
    );
}

#[test]
fn name_width_with_gutter_reserves_space_before_lint() {
    assert_eq!(App::name_width_with_gutter(0), 1);
    assert_eq!(App::name_width_with_gutter(42), 43);
}

#[test]
fn tabbable_panes_follow_canonical_order() {
    let project = RootItem::Rust(RustProject::Package(PackageProject::new(
        test_path("~/demo"),
        Some("demo".to_string()),
        Cargo::new(
            None,
            None,
            Vec::new(),
            vec![ExampleGroup {
                category: String::new(),
                names:    vec!["example".to_string()],
            }],
            Vec::new(),
            0,
            false,
        ),
        Vec::new(),
        None,
        None,
    )));

    let mut app = make_app(std::slice::from_ref(&project));
    app.toasts = ToastManager::default();
    app.pane_manager.toasts.set_len(0);
    app.scan.phase = ScanPhase::Complete;
    app.handle_git_info(
        project.path(),
        GitInfo {
            path_state:          GitPathState::default(),
            origin:              GitOrigin::Clone,
            branch:              None,
            owner:               None,
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        None,
            upstream_branch:     None,
            default_branch:      None,
            ahead_behind_origin: None,
            local_main_branch:   None,
            ahead_behind_local:  None,
            workflows:           WorkflowPresence::Present,
        },
    );
    app.detail_generation += 1;
    app.ensure_detail_cached();
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![make_ci_run(1, Conclusion::Success)],
        false,
        0,
    );
    app.detail_generation += 1;
    app.ensure_detail_cached();

    assert_eq!(
        app.tabbable_panes(),
        vec![
            PaneId::ProjectList,
            PaneId::Package,
            PaneId::Git,
            PaneId::Targets,
            PaneId::CiRuns,
        ]
    );

    app.show_timed_toast("Settings", "Updated");
    assert_eq!(
        app.tabbable_panes(),
        vec![
            PaneId::ProjectList,
            PaneId::Package,
            PaneId::Git,
            PaneId::Targets,
            PaneId::CiRuns,
            PaneId::Toasts,
        ]
    );

    app.focus_next_pane();
    assert_eq!(app.focused_pane, PaneId::Package);
    app.focus_next_pane();
    assert_eq!(app.focused_pane, PaneId::Git);
    app.focus_next_pane();
    assert_eq!(app.focused_pane, PaneId::Targets);
    app.focus_next_pane();
    assert_eq!(app.focused_pane, PaneId::CiRuns);
    app.focus_next_pane();
    assert_eq!(app.focused_pane, PaneId::Toasts);
    app.focus_previous_pane();
    assert_eq!(app.focused_pane, PaneId::CiRuns);
}

#[test]
fn new_toasts_do_not_steal_focus() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.focus_pane(PaneId::Git);

    app.show_timed_toast("Settings", "Updated");
    assert_eq!(app.focused_pane, PaneId::Git);

    let _task = app.start_task_toast("Startup lints", "Running startup lint jobs...");
    assert_eq!(app.focused_pane, PaneId::Git);
}

#[test]
fn project_refresh_updates_selected_tree_project_targets() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.phase = ScanPhase::Complete;
    app.list_state.select(Some(0));
    app.sync_selected_project();

    app.ensure_detail_cached();
    let example_count = app
        .pane_manager
        .targets_data
        .as_ref()
        .map(|d| d.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(example_count, Some(0));
    assert!(!app.tabbable_panes().contains(&PaneId::Targets));

    let refreshed = RootItem::Rust(RustProject::Package(PackageProject::new(
        test_path("~/demo"),
        Some("demo".to_string()),
        Cargo::new(
            None,
            None,
            Vec::new(),
            vec![ExampleGroup {
                category: String::new(),
                names:    vec!["tracked_row_paths".to_string()],
            }],
            Vec::new(),
            0,
            false,
        ),
        Vec::new(),
        None,
        None,
    )));

    app.handle_project_refreshed(refreshed);
    app.sync_selected_project();

    app.ensure_detail_cached();
    let example_count = app
        .pane_manager
        .targets_data
        .as_ref()
        .map(|d| d.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(example_count, Some(1));
    assert!(app.tabbable_panes().contains(&PaneId::Targets));
}

#[test]
fn first_non_empty_tree_build_focuses_project_list() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_items(&mut app, &[project]);

    assert_eq!(app.focused_pane, PaneId::ProjectList);
    assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn initial_disk_batch_count_groups_nested_projects_under_one_root() {
    let projects: Vec<RootItem> = [
        make_project(Some("bevy"), "~/rust/bevy"),
        make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
        make_project(Some("render"), "~/rust/bevy/crates/bevy_render"),
        make_project(Some("hana"), "~/rust/hana"),
        make_project(Some("hana_core"), "~/rust/hana/crates/hana"),
    ]
    .to_vec();

    assert_eq!(snapshots::initial_disk_batch_count(&projects), 2);
}

#[test]
fn overlays_restore_prior_focus() {
    let app_project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[app_project]);
    app.focus_pane(PaneId::Git);

    app.open_overlay(PaneId::Settings);
    app.open_settings();
    assert_eq!(app.focused_pane, PaneId::Settings);
    assert_eq!(app.return_focus, Some(PaneId::Git));

    app.close_settings();
    app.close_overlay();
    assert_eq!(app.focused_pane, PaneId::Git);
    assert!(app.return_focus.is_none());
}

#[test]
fn detail_panes_do_not_remember_selection_until_focused() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    assert!(app.remembers_selection(PaneId::ProjectList));
    assert!(!app.remembers_selection(PaneId::Package));
    assert!(!app.remembers_selection(PaneId::Git));
    assert!(!app.remembers_selection(PaneId::Targets));
    assert!(!app.remembers_selection(PaneId::CiRuns));

    app.focus_pane(PaneId::Package);
    assert!(app.remembers_selection(PaneId::Package));
}

#[test]
fn project_change_resets_project_dependent_panes() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(&[project_a, project_b]);

    app.focus_pane(PaneId::Package);
    app.focus_pane(PaneId::Git);
    app.focus_pane(PaneId::Targets);
    app.focus_pane(PaneId::CiRuns);
    app.pane_manager.package.set_pos(3);
    app.pane_manager.git.set_pos(4);
    app.pane_manager.targets.set_pos(5);
    app.pane_manager.ci.set_pos(6);

    app.list_state.select(Some(1));
    app.sync_selected_project();

    assert_eq!(app.pane_manager.package.pos(), 0);
    assert_eq!(app.pane_manager.git.pos(), 0);
    assert_eq!(app.pane_manager.targets.pos(), 0);
    assert_eq!(app.pane_manager.ci.pos(), 0);
    assert!(!app.remembers_selection(PaneId::Package));
    assert!(!app.remembers_selection(PaneId::Git));
    assert!(!app.remembers_selection(PaneId::Targets));
    assert!(!app.remembers_selection(PaneId::CiRuns));
    assert_eq!(
        app.selection_paths
            .selected_project
            .as_ref()
            .map(crate::project::AbsolutePath::as_path),
        app.selected_project_path()
    );
}

#[test]
fn apply_config_resets_column_layout_flag() {
    let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);
    let mut cfg = CargoPortConfig::default();

    assert!(!app.cached_fit_widths.lint_enabled());

    cfg.lint.enabled = true;
    app.apply_config(&cfg);
    assert!(app.cached_fit_widths.lint_enabled());

    cfg.lint.enabled = false;
    app.apply_config(&cfg);
    assert!(!app.cached_fit_widths.lint_enabled());
}

#[test]
fn zero_byte_update_marks_deleted_child_member() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let workspace_dir = tmp.path().join("hana");
    let member_dir = workspace_dir.join("crates").join("clay-layout");
    std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());

    let ws_path = workspace_dir.to_string_lossy().to_string();
    let member_path = member_dir.to_string_lossy().to_string();
    let workspace = make_workspace_project(Some("hana"), &ws_path);
    let member = make_project(Some("clay-layout"), &member_path);

    let root = make_workspace_with_members(
        Some("hana"),
        &ws_path,
        vec![inline_group(vec![make_member(
            Some("clay-layout"),
            &member_path,
        )])],
    );

    let mut app = make_app(&[workspace, member]);
    apply_items(&mut app, &[root]);

    std::fs::remove_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());
    app.handle_disk_usage(Path::new(&member_path), 0);
}

#[test]
fn top_level_deleted_project_enters_deleted_state_and_renders_as_deleted() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_dir = tmp.path().join("demo");
    std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

    let project_path = project_dir.to_string_lossy().to_string();
    let project = make_project(Some("demo"), &project_path);
    let mut app = make_app(&[project]);

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows().len(),
        1,
        "top-level project should render"
    );

    std::fs::remove_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
    app.handle_disk_usage(Path::new(&project_path), 0);

    let abs_path = AbsolutePath::from(project_path.clone());
    assert!(
        app.is_deleted(&abs_path),
        "top-level project should be deleted"
    );
    assert_eq!(
        app.projects
            .at_path(&abs_path)
            .expect("top-level project should still exist in hierarchy")
            .visibility,
        Deleted
    );

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows().len(),
        1,
        "deleted top-level project should still render before dismiss"
    );

    app.list_state.select(Some(0));
    assert!(
        app.focused_dismiss_target().is_some(),
        "deleted top-level project should expose dismiss affordance"
    );

    let item = &app.projects[0];
    let row = crate::tui::columns::build_row_cells(crate::tui::columns::ProjectRow {
        prefix:            crate::tui::render::PREFIX_ROOT_LEAF,
        name:              &item.root_directory_name().into_string(),
        name_segments:     None,
        git_path_state:    app.git_path_state_for(item.path()),
        lint_icon:         app.lint_icon_for_root(0),
        lint_style:        Style::default(),
        disk:              "0.0",
        disk_style:        Style::default(),
        disk_suffix:       Some(" [x]"),
        disk_suffix_style: Some(Style::default().fg(Color::DarkGray)),
        lang_icon:         item.lang_icon(),
        git_origin_sync:   &app.git_sync(item.path()),
        git_main:          &app.git_main(item.path()),
        ci:                app.ci_for_item(item),
        deleted:           true,
        worktree_health:   Normal,
    });
    let widths = crate::tui::columns::ResolvedWidths::new(true);
    let line = crate::tui::columns::row_to_line(&row, &widths);

    let suffix = line
        .spans
        .iter()
        .find(|span| span.content.as_ref() == " [x]")
        .expect("deleted row should render dismiss suffix");
    assert_eq!(suffix.style.fg, Some(Color::DarkGray));
    assert!(
        !suffix.style.add_modifier.contains(Modifier::CROSSED_OUT),
        "dismiss suffix should not be crossed out"
    );

    let crossed_out_non_suffix = line
        .spans
        .iter()
        .filter(|span| span.content.as_ref() != " [x]")
        .all(|span| span.style.add_modifier.contains(Modifier::CROSSED_OUT));
    assert!(
        crossed_out_non_suffix,
        "deleted row content should be crossed out"
    );
}

#[test]
fn top_level_deleted_project_can_be_dismissed_and_stops_rendering() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_dir = tmp.path().join("demo");
    std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

    let project_path = project_dir.to_string_lossy().to_string();
    let project = make_project(Some("demo"), &project_path);
    let mut app = make_app(&[project]);

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows().len(),
        1,
        "top-level project should render"
    );

    std::fs::remove_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
    app.handle_disk_usage(Path::new(&project_path), 0);

    let abs_path = AbsolutePath::from(project_path.clone());
    assert!(
        app.is_deleted(&abs_path),
        "top-level project should be deleted"
    );
    assert_eq!(
        app.projects
            .at_path(&abs_path)
            .expect("top-level project should still exist in hierarchy")
            .visibility,
        Deleted
    );

    app.list_state.select(Some(0));
    let target = app
        .focused_dismiss_target()
        .expect("deleted top-level project should be dismissable");
    app.dismiss(target);

    app.ensure_visible_rows_cached();
    assert_eq!(
        app.visible_rows().len(),
        0,
        "dismissed top-level deleted project should no longer render"
    );
    assert_eq!(
        app.projects
            .at_path(&abs_path)
            .expect("top-level project should remain in hierarchy after dismiss")
            .visibility,
        Dismissed
    );
}
