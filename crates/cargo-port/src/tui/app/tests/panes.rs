use super::*;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::TargetRecord;
use crate::project::WorkspaceSnapshot;
use crate::project::WorktreeHealth::Normal;
use crate::tui::columns;
use crate::tui::columns::ProjectRow;

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
    app.expanded_mut().insert(ExpandKey::Node(0));
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

    assert_eq!(app.selected_project_path(), Some(member.path().as_path()));
}

#[test]
fn name_width_with_gutter_reserves_space_before_lint() {
    assert_eq!(App::name_width_with_gutter(0), 1);
    assert_eq!(App::name_width_with_gutter(42), 43);
}

/// Upsert a minimal `WorkspaceSnapshot` into `app`'s metadata store
/// for `project_path`, naming a single Example target so the Targets
/// pane becomes tabbable. Keeps the per-test setup out of line when
/// the test's focus is pane behavior, not snapshot plumbing.
fn seed_single_example_snapshot(app: &App, project_path: &AbsolutePath, example_name: &str) {
    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;
    let pkg = PackageRecord {
        id:            PackageId {
            repr: "demo-id".into(),
        },
        name:          "demo".into(),
        version:       Version::new(0, 1, 0),
        edition:       "2021".into(),
        description:   None,
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(project_path.as_path().join("Cargo.toml")),
        targets:       vec![crate::project::TargetRecord {
            name:              example_name.to_string(),
            kinds:             vec![TargetKind::Example],
            src_path:          AbsolutePath::from(
                project_path
                    .as_path()
                    .join(format!("examples/{example_name}.rs")),
            ),
            edition:           "2021".to_string(),
            required_features: Vec::new(),
        }],
        publish:       PublishPolicy::Any,
    };
    let mut packages = std::collections::HashMap::new();
    packages.insert(pkg.id.clone(), pkg);
    app.metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .upsert(WorkspaceSnapshot {
            workspace_root: project_path.clone(),
            target_directory: AbsolutePath::from(project_path.as_path().join("target")),
            packages,
            workspace_members: Vec::new(),
            fetched_at: std::time::SystemTime::UNIX_EPOCH,
            fingerprint: ManifestFingerprint {
                manifest:       FileStamp {
                    mtime:        std::time::SystemTime::UNIX_EPOCH,
                    len:          0,
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
    // Step 3b: Targets pane requires a snapshot.
    let project_path = test_path("~/demo");
    let project = RootItem::Rust(RustProject::Package(Package {
        path: project_path.clone(),
        name: Some("demo".to_string()),
        ..Package::default()
    }));
    let mut app = make_app(std::slice::from_ref(&project));
    seed_single_example_snapshot(&app, &project_path, "example");
    app.toasts = ToastManager::default();
    app.pane_manager_mut().pane_mut(PaneId::Toasts).set_len(0);
    app.scan_state_mut().phase = ScanPhase::Complete;
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
        vec![make_ci_run(1, Conclusion::Success)],
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
        app.focus_next_pane();
        assert_eq!(app.focused_pane, pane);
    }
    app.focus_previous_pane();
    assert_eq!(
        app.focused_pane,
        expected_with_toasts[expected_with_toasts.len() - 2]
    );
}

#[test]
fn cpu_pane_selection_persists_across_project_changes() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(&[project_a, project_b]);
    app.focus_pane(PaneId::Cpu);
    app.pane_manager_mut().pane_mut(PaneId::Cpu).set_pos(1);
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_pos(1);

    app.sync_selected_project();

    assert_eq!(app.pane_manager().pane(PaneId::Cpu).pos(), 1);
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
fn snapshot_arrival_populates_selected_tree_project_targets() {
    // Step 3b: Targets pane data now comes exclusively from the
    // `cargo metadata` snapshot — the hand-parsed Cargo fallback
    // has been retired per the design plan's "Loading… without a
    // snapshot" rule. This test used to exercise the old fallback
    // (ExampleGroup on the Cargo struct); rewritten to confirm
    // the snapshot-driven path: a CargoMetadata arrival with an
    // Example target lights up the pane.
    use cargo_metadata::PackageId;
    use cargo_metadata::TargetKind;
    use cargo_metadata::semver::Version;

    let project = make_project(Some("demo"), "/never-real/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan_state_mut().phase = ScanPhase::Complete;
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_pos(0);
    app.sync_selected_project();

    app.ensure_detail_cached();
    let example_count = app
        .pane_data()
        .targets()
        .map(|d| d.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(
        example_count,
        Some(0),
        "pre-snapshot: Targets pane is empty"
    );
    assert!(!app.tabbable_panes().contains(&PaneId::Targets));

    let workspace_root = AbsolutePath::from("/never-real/demo");
    let manifest_path = AbsolutePath::from("/never-real/demo/Cargo.toml");
    let example = TargetRecord {
        name:              "tracked_row_paths".to_string(),
        kinds:             vec![TargetKind::Example],
        src_path:          AbsolutePath::from("/never-real/demo/examples/tracked_row_paths.rs"),
        edition:           "2021".to_string(),
        required_features: Vec::new(),
    };
    let pkg = PackageRecord {
        id: PackageId {
            repr: "demo-id".into(),
        },
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
    packages.insert(pkg.id.clone(), pkg);
    let snap = WorkspaceSnapshot {
        workspace_root: workspace_root.clone(),
        target_directory: AbsolutePath::from("/never-real/demo/target"),
        packages,
        workspace_members: Vec::new(),
        fetched_at: std::time::SystemTime::UNIX_EPOCH,
        fingerprint: ManifestFingerprint {
            manifest:       FileStamp {
                mtime:        std::time::SystemTime::UNIX_EPOCH,
                len:          0,
                content_hash: [0_u8; 32],
            },
            lockfile:       None,
            rust_toolchain: None,
            configs:        std::collections::BTreeMap::new(),
        },
        out_of_tree_target_bytes: None,
    };
    let generation = app
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .next_generation(&workspace_root);
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root,
        generation,
        fingerprint: snap.fingerprint.clone(),
        result: Ok(snap),
    });
    app.ensure_detail_cached();
    let example_count = app
        .pane_data()
        .targets()
        .map(|d| d.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(
        example_count,
        Some(1),
        "snapshot-arrival populates Targets from PackageRecord.targets"
    );
    assert!(app.tabbable_panes().contains(&PaneId::Targets));
}

#[test]
fn first_non_empty_tree_build_focuses_project_list() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_items(&mut app, &[project]);

    assert_eq!(app.focused_pane, PaneId::ProjectList);
    assert_eq!(app.pane_manager().pane(PaneId::ProjectList).pos(), 0);
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
        snapshots::initial_disk_roots(&super::as_entries(projects)).len(),
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

    let roots = snapshots::initial_metadata_roots(&super::as_entries(projects));
    assert_eq!(roots.len(), 3, "each Rust leaf gets its own metadata root");
}

#[test]
fn initial_metadata_roots_skips_non_rust_leaves() {
    let non_rust = RootItem::NonRust(crate::project::NonRustProject::new(
        super::test_path("~/notes"),
        Some("notes".into()),
    ));
    let pkg = make_project(Some("pkg"), "~/pkg");
    let roots = snapshots::initial_metadata_roots(&super::as_entries(vec![non_rust, pkg]));
    assert_eq!(roots.len(), 1, "non-rust leaves are not metadata roots");
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
    app.pane_manager_mut().pane_mut(PaneId::Package).set_pos(3);
    app.pane_manager_mut().pane_mut(PaneId::Git).set_pos(4);
    app.pane_manager_mut().pane_mut(PaneId::Targets).set_pos(5);
    app.panes_mut().ci_mut().viewport_mut().set_pos(6);
    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_pos(1);
    app.sync_selected_project();

    assert_eq!(app.pane_manager().pane(PaneId::Package).pos(), 0);
    assert_eq!(app.pane_manager().pane(PaneId::Git).pos(), 0);
    assert_eq!(app.pane_manager().pane(PaneId::Targets).pos(), 0);
    assert_eq!(app.panes().ci().viewport().pos(), 0);
    assert!(!app.remembers_selection(PaneId::Package));
    assert!(!app.remembers_selection(PaneId::Git));
    assert!(!app.remembers_selection(PaneId::Targets));
    assert!(!app.remembers_selection(PaneId::CiRuns));
    assert_eq!(
        app.selection()
            .paths()
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

    assert!(!app.cached_fit_widths().lint_enabled());

    cfg.lint.enabled = true;
    app.apply_config(&cfg);
    assert!(app.cached_fit_widths().lint_enabled());

    cfg.lint.enabled = false;
    app.apply_config(&cfg);
    assert!(!app.cached_fit_widths().lint_enabled());
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
        app.projects()
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

    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_pos(0);
    assert!(
        app.focused_dismiss_target().is_some(),
        "deleted top-level project should expose dismiss affordance"
    );

    let item = &app.projects()[0].item;
    let row = columns::build_row_cells(ProjectRow {
        prefix:            crate::tui::render::PREFIX_ROOT_LEAF,
        name:              &item.root_directory_name().into_string(),
        name_segments:     None,
        git_status:        app.git_status_for(item.path()),
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
        app.is_deleted(&abs_path),
        "top-level project should be deleted"
    );
    assert_eq!(
        app.projects()
            .at_path(&abs_path)
            .expect("top-level project should still exist in hierarchy")
            .visibility,
        Deleted
    );

    app.pane_manager_mut()
        .pane_mut(PaneId::ProjectList)
        .set_pos(0);
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
        app.projects()
            .at_path(&abs_path)
            .expect("top-level project should remain in hierarchy after dismiss")
            .visibility,
        Dismissed
    );
}
