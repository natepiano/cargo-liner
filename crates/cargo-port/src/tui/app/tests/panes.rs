use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use tui_pane::GlobalAction;

use super::*;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::TargetRecord;
use crate::project::WorkspaceMetadata;
use crate::project::WorktreeHealth::Normal;
use crate::tui::app::startup;
use crate::tui::columns;
use crate::tui::columns::ProjectRow;
use crate::tui::input;

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    input::handle_event(app, &Event::Key(KeyEvent::new(code, modifiers)));
}

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
    app.project_list.expanded.insert(ExpandKey::Node(0));
    app.project_list
        .select_project_in_tree(member.path(), false);

    app.project_list.collapse_all(false);

    assert_eq!(
        app.project_list.selected_row(),
        Some(VisibleRow::Root { node_index: 0 })
    );
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
    app.project_list
        .select_project_in_tree(member.path(), false);
    app.project_list.collapse_all(false);

    app.project_list.expand_all(false);

    assert_eq!(
        app.project_list.selected_project_path(),
        Some(member.path().as_path())
    );
}

#[test]
fn name_width_with_gutter_reserves_space_before_lint() {
    assert_eq!(crate::tui::panes::name_width_with_gutter(0), 1);
    assert_eq!(crate::tui::panes::name_width_with_gutter(42), 43);
}

/// Upsert minimal `WorkspaceMetadata` into `app`'s metadata store
/// for `project_path`, naming a single Example target so the Targets
/// pane becomes tabbable. Keeps the per-test setup out of line when
/// the test's focus is pane behavior, not metadata plumbing.
fn seed_single_example_metadata(app: &App, project_path: &AbsolutePath, example_name: &str) {
    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;
    let pkg_id = PackageId {
        repr: "demo-id".into(),
    };
    let pkg = PackageRecord {
        name:          "demo".into(),
        version:       Version::new(0, 1, 0),
        edition:       "2021".into(),
        description:   None,
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(project_path.as_path().join("Cargo.toml")),
        targets:       vec![crate::project::TargetRecord {
            name:     example_name.to_string(),
            kinds:    vec![TargetKind::Example],
            src_path: AbsolutePath::from(
                project_path
                    .as_path()
                    .join(format!("examples/{example_name}.rs")),
            ),
        }],
        publish:       PublishPolicy::Any,
    };
    let mut packages = std::collections::HashMap::new();
    packages.insert(pkg_id, pkg);
    app.scan
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .upsert(WorkspaceMetadata {
            workspace_root: project_path.clone(),
            target_directory: AbsolutePath::from(project_path.as_path().join("target")),
            packages,
            fingerprint: ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        std::collections::BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        });
}

#[test]
fn tabbable_panes_follow_canonical_order() {
    // Targets pane requires workspace metadata.
    let project_path = test_path("~/demo");
    let project = RootItem::Rust(RustProject::Package(Package {
        path: project_path.clone(),
        name: Some("demo".to_string()),
        ..Package::default()
    }));
    let mut app = make_app(std::slice::from_ref(&project));
    seed_single_example_metadata(&app, &project_path, "example");
    app.framework.toasts = tui_pane::Toasts::default();
    app.framework.toasts.viewport.set_len(0);
    app.scan.state.phase = ScanPhase::Complete;
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              None,
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: None,
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        None,
                    repo:         Some("demo".to_string()),
                    tracked_ref:  None,
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    None,
                local_main_branch: None,
            },
        ),
    );
    app.ensure_detail_cached();
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![make_ci_run(1, CiStatus::Passed)],
        false,
        0,
    );
    app.ensure_detail_cached();

    let expected_without_toasts = app.tabbable_panes();
    assert!(expected_without_toasts.contains(&PaneId::Cpu));
    let cpu_index = expected_without_toasts
        .iter()
        .position(|pane| *pane == PaneId::Cpu)
        .unwrap_or_else(|| std::process::abort());
    let targets_index = expected_without_toasts
        .iter()
        .position(|pane| *pane == PaneId::Targets)
        .unwrap_or_else(|| std::process::abort());
    assert!(cpu_index < targets_index);

    app.show_timed_toast("Settings", "Updated");
    let expected_with_toasts = app.tabbable_panes();

    assert_eq!(
        expected_with_toasts,
        expected_without_toasts
            .iter()
            .copied()
            .chain(std::iter::once(PaneId::Toasts))
            .collect::<Vec<_>>()
    );

    for &pane in &expected_with_toasts[1..] {
        press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(app.focused_pane_id(), pane);
    }
    press(&mut app, KeyCode::Tab, KeyModifiers::SHIFT);
    assert_eq!(
        app.focused_pane_id(),
        expected_with_toasts[expected_with_toasts.len() - 2]
    );
}

#[test]
fn cpu_pane_selection_persists_across_project_changes() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(&[project_a, project_b]);
    app.set_focus_to_pane(PaneId::Cpu);
    app.panes.cpu.viewport.set_pos(1);
    app.project_list.set_cursor(1);

    app.sync_selected_project();

    assert_eq!(app.panes.cpu.viewport.pos(), 1);
}

#[test]
fn new_toasts_do_not_steal_focus() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);
    app.set_focus_to_pane(PaneId::Git);

    app.show_timed_toast("Settings", "Updated");
    assert_eq!(app.focused_pane_id(), PaneId::Git);

    let _task = app
        .framework
        .toasts
        .start_task("Startup lints", "Running startup lint jobs...");
    assert_eq!(app.focused_pane_id(), PaneId::Git);
}

#[test]
fn metadata_arrival_populates_selected_tree_project_targets() {
    // Targets pane data comes exclusively from the `cargo metadata`
    // result. A CargoMetadata arrival with an Example target lights up
    // the pane.
    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;

    let project = make_project(Some("demo"), "/never-real/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.state.phase = ScanPhase::Complete;
    app.project_list.set_cursor(0);
    app.sync_selected_project();

    app.ensure_detail_cached();
    let example_count = app
        .panes
        .targets
        .content()
        .map(|d| d.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(
        example_count,
        Some(0),
        "pre-metadata: Targets pane is empty"
    );
    assert!(!app.tabbable_panes().contains(&PaneId::Targets));

    let workspace_root = AbsolutePath::from("/never-real/demo");
    let manifest_path = AbsolutePath::from("/never-real/demo/Cargo.toml");
    let example = TargetRecord {
        name:     "tracked_row_paths".to_string(),
        kinds:    vec![TargetKind::Example],
        src_path: AbsolutePath::from("/never-real/demo/examples/tracked_row_paths.rs"),
    };
    let pkg_id = PackageId {
        repr: "demo-id".into(),
    };
    let pkg = PackageRecord {
        name: "demo".into(),
        version: Version::new(0, 1, 0),
        edition: "2021".into(),
        description: None,
        license: None,
        homepage: None,
        repository: None,
        manifest_path,
        targets: vec![example],
        publish: PublishPolicy::Any,
    };
    let mut packages = std::collections::HashMap::new();
    packages.insert(pkg_id, pkg);
    let workspace_metadata = WorkspaceMetadata {
        workspace_root: workspace_root.clone(),
        target_directory: AbsolutePath::from("/never-real/demo/target"),
        packages,
        fingerprint: ManifestFingerprint {
            manifest:       FileStamp {
                content_hash: [0_u8; 32],
            },
            lockfile:       None,
            rust_toolchain: None,
            configs:        std::collections::BTreeMap::new(),
        },
        out_of_tree_target_bytes: None,
    };
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .next_generation(&workspace_root);
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root,
        generation,
        fingerprint: workspace_metadata.fingerprint.clone(),
        result: Ok(workspace_metadata),
    });
    app.ensure_detail_cached();
    let example_count = app
        .panes
        .targets
        .content()
        .map(|d| d.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(
        example_count,
        Some(1),
        "metadata-arrival populates Targets from PackageRecord.targets"
    );
    assert!(app.tabbable_panes().contains(&PaneId::Targets));
}

#[test]
fn first_non_empty_tree_build_focuses_project_list() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_items(&mut app, &[project]);

    assert_eq!(app.focused_pane_id(), PaneId::ProjectList);
    assert_eq!(app.project_list.cursor(), 0);
}

#[test]
fn initial_disk_roots_groups_nested_projects_under_one_root() {
    let projects: Vec<RootItem> = [
        make_project(Some("bevy"), "~/rust/bevy"),
        make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
        make_project(Some("render"), "~/rust/bevy/crates/bevy_render"),
        make_project(Some("hana"), "~/rust/hana"),
        make_project(Some("hana_core"), "~/rust/hana/crates/hana"),
    ]
    .to_vec();

    assert_eq!(
        crate::tui::app::startup::initial_disk_roots(&super::as_entries(projects)).len(),
        2
    );
}

#[test]
fn initial_metadata_roots_collects_every_rust_leaf() {
    // Contrast with `initial_disk_roots`: metadata needs one dispatch per
    // leaf (each Cargo.toml has its own resolved target_directory), not a
    // deduped-by-prefix set.
    let projects: Vec<RootItem> = [
        make_project(Some("bevy"), "~/rust/bevy"),
        make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
        make_project(Some("hana"), "~/rust/hana"),
    ]
    .to_vec();

    let roots = startup::initial_metadata_roots(&super::as_entries(projects));
    assert_eq!(roots.len(), 3, "each Rust leaf gets its own metadata root");
}

#[test]
fn initial_metadata_roots_skips_non_rust_leaves() {
    let non_rust = RootItem::NonRust(crate::project::NonRustProject::new(
        super::test_path("~/notes"),
        Some("notes".into()),
    ));
    let pkg = make_project(Some("pkg"), "~/pkg");
    let roots = startup::initial_metadata_roots(&super::as_entries(vec![non_rust, pkg]));
    assert_eq!(roots.len(), 1, "non-rust leaves are not metadata roots");
}

#[test]
fn overlays_restore_prior_focus() {
    let app_project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[app_project]);
    app.set_focus_to_pane(PaneId::Git);

    app.dispatch_framework_global_action(GlobalAction::OpenSettings);
    assert_eq!(app.focused_pane_id(), PaneId::Git);
    assert_eq!(
        app.framework.overlay(),
        Some(tui_pane::FrameworkOverlayId::Settings)
    );

    app.dispatch_framework_global_action(GlobalAction::Dismiss);
    assert_eq!(app.focused_pane_id(), PaneId::Git);
    assert_eq!(app.framework.overlay(), None);
}

#[test]
fn detail_panes_do_not_remember_selection_until_focused() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(&[project]);

    assert_eq!(
        app.pane_focus_state(PaneId::ProjectList),
        PaneFocusState::Active
    );
    assert_eq!(
        app.pane_focus_state(PaneId::Package),
        PaneFocusState::Inactive
    );
    assert_eq!(app.pane_focus_state(PaneId::Git), PaneFocusState::Inactive);
    assert_eq!(
        app.pane_focus_state(PaneId::Targets),
        PaneFocusState::Inactive
    );
    assert_eq!(
        app.pane_focus_state(PaneId::CiRuns),
        PaneFocusState::Inactive
    );

    app.set_focus_to_pane(PaneId::Package);
    app.set_focus_to_pane(PaneId::ProjectList);
    assert_eq!(
        app.pane_focus_state(PaneId::Package),
        PaneFocusState::Remembered
    );
}

#[test]
fn project_change_resets_project_dependent_panes() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(&[project_a, project_b]);

    app.set_focus_to_pane(PaneId::Package);
    app.set_focus_to_pane(PaneId::Git);
    app.set_focus_to_pane(PaneId::Targets);
    app.set_focus_to_pane(PaneId::CiRuns);
    app.panes.package.viewport.set_pos(3);
    app.panes.git.viewport.set_pos(4);
    app.panes.targets.viewport.set_pos(5);
    app.ci.viewport.set_pos(6);
    app.project_list.set_cursor(1);
    app.sync_selected_project();

    assert_eq!(app.panes.package.viewport.pos(), 0);
    assert_eq!(app.panes.git.viewport.pos(), 0);
    assert_eq!(app.panes.targets.viewport.pos(), 0);
    assert_eq!(app.ci.viewport.pos(), 0);
    assert_eq!(
        app.pane_focus_state(PaneId::Package),
        PaneFocusState::Inactive
    );
    assert_eq!(app.pane_focus_state(PaneId::Git), PaneFocusState::Inactive);
    assert_eq!(
        app.pane_focus_state(PaneId::Targets),
        PaneFocusState::Inactive
    );
    assert_eq!(
        app.pane_focus_state(PaneId::CiRuns),
        PaneFocusState::Inactive
    );
    assert_eq!(
        app.project_list
            .paths
            .selected_project
            .as_ref()
            .map(crate::project::AbsolutePath::as_path),
        app.project_list.selected_project_path()
    );
}

#[test]
fn apply_config_resets_column_layout_flag() {
    let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);
    let mut cfg = CargoPortConfig::default();

    assert!(!app.project_list.cached_fit_widths.lint_enabled());

    cfg.lint.enabled = true;
    app.apply_config(&cfg);
    assert!(app.project_list.cached_fit_widths.lint_enabled());

    cfg.lint.enabled = false;
    app.apply_config(&cfg);
    assert!(!app.project_list.cached_fit_widths.lint_enabled());
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
        app.project_list.is_deleted(&abs_path),
        "top-level project should be deleted"
    );
    assert_eq!(
        app.project_list
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

    app.project_list.set_cursor(0);
    assert!(
        app.focused_dismiss_target().is_some(),
        "deleted top-level project should expose dismiss affordance"
    );

    let item = &app.project_list[0].item;
    let row = columns::build_row_cells(ProjectRow {
        prefix:            crate::tui::panes::PREFIX_ROOT_LEAF,
        name:              &item.root_directory_name().into_string(),
        name_segments:     None,
        git_status:        app.project_list.git_status_for(item.path()),
        lint:              app.lint_cell(&crate::tui::state::Lint::status_for_root(item)),
        disk:              "0.0",
        disk_style:        Style::default(),
        disk_suffix:       Some(" [x]"),
        disk_suffix_style: Some(Style::default().fg(Color::DarkGray)),
        lang_icon:         item.lang_icon(),
        git_origin_sync:   &app.project_list.git_sync(item.path()),
        git_main:          &app.project_list.git_main(item.path()),
        ci:                app
            .project_list
            .ci_status_for_root_item_using_lookup(item, &app.ci.status_lookup()),
        deleted:           true,
        worktree_health:   Normal,
    });
    let widths = crate::tui::columns::ProjectListWidths::new(true);
    let line = columns::row_to_line(&row, &widths);

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
        app.project_list.is_deleted(&abs_path),
        "top-level project should be deleted"
    );
    assert_eq!(
        app.project_list
            .at_path(&abs_path)
            .expect("top-level project should still exist in hierarchy")
            .visibility,
        Deleted
    );

    app.project_list.set_cursor(0);
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
        app.project_list
            .at_path(&abs_path)
            .expect("top-level project should remain in hierarchy after dismiss")
            .visibility,
        Dismissed
    );
}
