use super::*;
use crate::project::AbsolutePath;
use crate::watcher::WatcherMsg;

#[test]
fn scan_result_registers_linked_worktrees_with_watcher() {
    let primary = make_workspace_raw_with_primary(
        Some("bevy_window_manager"),
        "~/rust/bevy_window_manager",
        vec![inline_group(vec![Package::new(
            test_path("~/rust/bevy_window_manager/crates/bevy_window_manager"),
            Some("bevy_window_manager".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0, false),
            Vec::new(),
            false,
            None,
        )])],
        None,
        None,
    );
    let linked = make_workspace_raw_with_primary(
        Some("bevy_window_manager_style_fix"),
        "~/rust/bevy_window_manager_style_fix",
        vec![inline_group(vec![Package::new(
            test_path("~/rust/bevy_window_manager_style_fix/crates/bevy_window_manager"),
            Some("bevy_window_manager".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0, false),
            Vec::new(),
            true,
            None,
        )])],
        Some("bevy_window_manager_style_fix"),
        Some("~/rust/bevy_window_manager"),
    );
    let mut app = make_app(&[]);
    let (watch_tx, watch_rx) = mpsc::channel();
    app.watch_tx = watch_tx;

    apply_bg_msg(
        &mut app,
        BackgroundMsg::ScanResult {
            projects:     vec![make_workspace_worktrees_item(
                primary.clone(),
                vec![linked.clone()],
            )],
            disk_entries: Vec::new(),
        },
    );

    let messages: Vec<_> = watch_rx.try_iter().collect();
    let watched_paths: HashSet<AbsolutePath> = messages
        .iter()
        .filter_map(|msg| match msg {
            WatcherMsg::Register(req) => Some(req.abs_path.clone()),
            WatcherMsg::InitialRegistrationComplete => None,
        })
        .collect();
    let completion_count = messages
        .iter()
        .filter(|msg| matches!(msg, WatcherMsg::InitialRegistrationComplete))
        .count();

    assert!(
        watched_paths.contains(primary.path().as_path()),
        "primary worktree root should be registered with watcher"
    );
    assert!(
        watched_paths.contains(linked.path().as_path()),
        "linked worktree root should be registered with watcher"
    );
    assert_eq!(
        completion_count, 1,
        "scan result should finish the watcher registration batch"
    );
}

#[test]
fn empty_scan_result_finishes_watcher_registration_batch() {
    let mut app = make_app(&[]);
    let (watch_tx, watch_rx) = mpsc::channel();
    app.watch_tx = watch_tx;

    apply_bg_msg(
        &mut app,
        BackgroundMsg::ScanResult {
            projects:     Vec::new(),
            disk_entries: Vec::new(),
        },
    );

    let messages: Vec<_> = watch_rx.try_iter().collect();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        messages[0],
        WatcherMsg::InitialRegistrationComplete
    ));
}

#[test]
fn external_config_reload_applies_valid_changes() {
    let mut app = make_app(&[]);
    let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let path = dir.path().join("config.toml");

    let mut cfg = CargoPortConfig::default();
    cfg.tui.editor = "helix".to_string();
    cfg.tui.ci_run_count = 9;
    cfg.cpu.poll_ms = 1500;
    cfg.mouse.invert_scroll = ScrollDirection::Normal;
    std::fs::write(
        &path,
        toml::to_string_pretty(&cfg).unwrap_or_else(|_| std::process::abort()),
    )
    .unwrap_or_else(|_| std::process::abort());

    app.config_path = Some(AbsolutePath::from(path));
    app.config_last_seen = None;
    app.maybe_reload_config_from_disk();

    assert_eq!(app.editor(), "helix");
    assert_eq!(app.ci_run_count(), 9);
    assert_eq!(app.current_config.cpu.poll_ms, 1500);
    assert_eq!(app.invert_scroll(), ScrollDirection::Normal);
    assert_eq!(app.current_config.tui.editor, "helix");
    assert_eq!(app.current_config.tui.ci_run_count, 9);
}

#[test]
fn external_config_reload_keeps_last_good_config_on_parse_error() {
    let mut app = make_app(&[]);
    let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let path = dir.path().join("config.toml");

    let mut cfg = CargoPortConfig::default();
    cfg.tui.editor = "zed".to_string();
    std::fs::write(
        &path,
        toml::to_string_pretty(&cfg).unwrap_or_else(|_| std::process::abort()),
    )
    .unwrap_or_else(|_| std::process::abort());

    app.config_path = Some(AbsolutePath::from(path.clone()));
    app.config_last_seen = None;
    app.maybe_reload_config_from_disk();

    std::fs::write(&path, "[tui\neditor = \"vim\"\n").unwrap_or_else(|_| std::process::abort());
    app.config_last_seen = None;
    app.maybe_reload_config_from_disk();

    assert_eq!(app.editor(), "zed");
    assert_eq!(app.current_config.tui.editor, "zed");
    assert!(matches!(
        app.status_flash.as_ref(),
        Some((msg, _)) if msg.contains("Config reload failed")
    ));
}

#[test]
fn completed_scan_hides_and_restores_cached_non_rust_projects_without_rescan() {
    let rust_project = make_project(Some("rust"), "~/rust");
    let non_rust_project = make_non_rust_project(Some("js"), "~/js");
    let mut cfg = CargoPortConfig::default();
    cfg.tui.include_non_rust = NonRustInclusion::Include;
    cfg.tui.include_dirs = vec!["/tmp/test".to_string()];
    let mut app = make_app_with_config(&[rust_project, non_rust_project], &cfg);
    app.scan.phase = ScanPhase::Complete;

    assert_eq!(app.projects.len(), 2);

    let mut hide_cfg = cfg.clone();
    hide_cfg.tui.include_non_rust = NonRustInclusion::Exclude;
    app.apply_config(&hide_cfg);
    wait_for_tree_build(&mut app);

    assert!(app.is_scan_complete());
    assert_eq!(app.projects.len(), 2);
    app.ensure_visible_rows_cached();
    let visible: Vec<_> = app
        .visible_rows()
        .iter()
        .filter_map(|row| match row {
            VisibleRow::Root { node_index } => Some(app.projects[*node_index].path().clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0], test_path("~/rust"));

    app.apply_config(&cfg);
    wait_for_tree_build(&mut app);

    assert!(app.is_scan_complete());
    assert_eq!(app.projects.len(), 2);
    assert!(
        app.projects
            .iter()
            .any(|item: &RootItem| item.path() == test_path("~/js").as_path())
    );
}

#[test]
fn completed_scan_rescans_when_enabling_non_rust_without_cached_projects() {
    let rust_project = make_project(Some("rust"), "~/rust");
    let mut app = make_app(&[rust_project]);
    app.scan.phase = ScanPhase::Complete;

    let mut cfg = app.current_config.clone();
    cfg.tui.include_non_rust = NonRustInclusion::Include;
    app.apply_config(&cfg);

    assert!(app.projects.is_empty());
    assert!(!app.is_scan_complete());
}

#[test]
fn service_reachability_tracks_background_messages() {
    let mut app = make_app(&[]);

    assert!(app.unreachable_services.is_empty());

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::GitHub,
    }));
    assert!(app.unreachable_services.contains(&ServiceKind::GitHub));

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::CratesIo,
    }));
    assert!(app.unreachable_services.contains(&ServiceKind::CratesIo));

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::GitHub,
    }));
    assert!(!app.unreachable_services.contains(&ServiceKind::GitHub));
    assert!(app.unreachable_services.contains(&ServiceKind::CratesIo));

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::CratesIo,
    }));
    assert!(app.unreachable_services.is_empty());
}
