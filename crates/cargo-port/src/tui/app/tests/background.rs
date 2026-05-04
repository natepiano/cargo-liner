use super::*;
use crate::project::AbsolutePath;
use crate::watcher::WatcherMsg;

#[test]
fn scan_result_registers_linked_worktrees_with_watcher() {
    let primary = make_workspace_raw_with_primary(
        Some("bevy_window_manager"),
        "~/rust/bevy_window_manager",
        vec![inline_group(vec![Package {
            path: test_path("~/rust/bevy_window_manager/crates/bevy_window_manager"),
            name: Some("bevy_window_manager".to_string()),
            ..Package::default()
        }])],
        None,
        None,
    );
    let linked = make_workspace_raw_with_primary(
        Some("bevy_window_manager_style_fix"),
        "~/rust/bevy_window_manager_style_fix",
        vec![inline_group(vec![Package {
            path: test_path("~/rust/bevy_window_manager_style_fix/crates/bevy_window_manager"),
            name: Some("bevy_window_manager".to_string()),
            worktree_status: WorktreeStatus::Linked {
                primary: test_path("~/rust/bevy_window_manager"),
            },
            ..Package::default()
        }])],
        Some("bevy_window_manager_style_fix"),
        Some("~/rust/bevy_window_manager"),
    );
    let mut app = make_app(&[]);
    let (watch_tx, watch_rx) = mpsc::channel();
    app.background_mut().replace_watcher_sender(watch_tx);

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
    app.background_mut().replace_watcher_sender(watch_tx);

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

    app.config_mut().force_reload_from(path);
    app.maybe_reload_config_from_disk();

    assert_eq!(app.config().editor(), "helix");
    assert_eq!(app.config().ci_run_count(), 9);
    assert_eq!(app.current_config().cpu.poll_ms, 1500);
    assert_eq!(app.config().invert_scroll(), ScrollDirection::Normal);
    assert_eq!(app.current_config().tui.editor, "helix");
    assert_eq!(app.current_config().tui.ci_run_count, 9);
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

    app.config_mut().force_reload_from(path.clone());
    app.maybe_reload_config_from_disk();

    std::fs::write(&path, "[tui\neditor = \"vim\"\n").unwrap_or_else(|_| std::process::abort());
    app.config_mut().force_reload_from(path);
    app.maybe_reload_config_from_disk();

    assert_eq!(app.config().editor(), "zed");
    assert_eq!(app.current_config().tui.editor, "zed");
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
    app.scan_state_mut().phase = ScanPhase::Complete;

    assert_eq!(app.projects().len(), 2);

    let mut hide_cfg = cfg.clone();
    hide_cfg.tui.include_non_rust = NonRustInclusion::Exclude;
    app.apply_config(&hide_cfg);
    wait_for_tree_build(&mut app);

    assert!(app.is_scan_complete());
    assert_eq!(app.projects().len(), 2);
    app.ensure_visible_rows_cached();
    let visible: Vec<_> = app
        .visible_rows()
        .iter()
        .filter_map(|row| match row {
            VisibleRow::Root { node_index } => Some(app.projects()[*node_index].path().clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0], test_path("~/rust"));

    app.apply_config(&cfg);
    wait_for_tree_build(&mut app);

    assert!(app.is_scan_complete());
    assert_eq!(app.projects().len(), 2);
    assert!(
        app.projects()
            .iter()
            .any(|entry| entry.item.path() == test_path("~/js").as_path())
    );
}

#[test]
fn completed_scan_rescans_when_enabling_non_rust_without_cached_projects() {
    let rust_project = make_project(Some("rust"), "~/rust");
    let mut app = make_app(&[rust_project]);
    app.scan_state_mut().phase = ScanPhase::Complete;

    let mut cfg = app.current_config().clone();
    cfg.tui.include_non_rust = NonRustInclusion::Include;
    app.apply_config(&cfg);

    assert!(app.projects().is_empty());
    assert!(!app.is_scan_complete());
}

#[test]
fn service_reachability_tracks_background_messages() {
    let mut app = make_app(&[]);

    assert!(!app.net().github().availability().is_unavailable());
    assert!(!app.net().crates_io().availability().is_unavailable());

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::GitHub,
    }));
    assert!(app.net().github().availability().is_unavailable());

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::CratesIo,
    }));
    assert!(app.net().crates_io().availability().is_unavailable());

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::GitHub,
    }));
    assert!(!app.net().github().availability().is_unavailable());
    assert!(app.net().crates_io().availability().is_unavailable());

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::CratesIo,
    }));
    assert!(!app.net().github().availability().is_unavailable());
    assert!(!app.net().crates_io().availability().is_unavailable());
}

#[test]
fn successful_request_dismisses_stuck_unreachable_toast() {
    // Regression: `Reachable` signals used to be no-ops when the
    // service was already marked unavailable. That left the
    // persistent toast stuck whenever the retry probe couldn't
    // complete (tight 1s HEAD timeout on a slow link, graphql quota
    // quirks, etc.) even while real data fetches were succeeding.
    // A successful request is authoritative evidence the service
    // works — it must clear the toast.
    let mut app = make_app(&[]);

    app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::GitHub,
    });
    let toast_id = app
        .net()
        .github()
        .availability()
        .toast_id()
        .expect("first unreachable signal pushes a toast");
    assert!(app.toasts().is_alive(toast_id));
    assert!(app.net().github().availability().is_unavailable());

    app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::GitHub,
    });
    assert!(
        !app.net().github().availability().is_unavailable(),
        "reachable signal should flip status back to available"
    );
    assert!(
        !app.toasts().is_alive(toast_id),
        "reachable signal must dismiss the persistent unreachable toast"
    );
}

#[test]
fn unreachable_toast_reappears_after_user_dismissal() {
    // Regression: dismissing the persistent unreachable toast by hand
    // left `ServiceAvailability.unavailable_toast` holding a stale id.
    // Subsequent `ServiceUnreachable` signals saw `needs_toast()` =
    // false and silently did nothing, so the user had no visible
    // indicator that GitHub was still unreachable.
    let mut app = make_app(&[]);

    app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::GitHub,
    });
    let toast_id = app
        .net()
        .github()
        .availability()
        .toast_id()
        .expect("first unreachable signal pushes a toast");

    // User dismisses the toast and waits long enough for the exit
    // animation to complete so the toast is evicted from the manager.
    app.dismiss_toast(toast_id);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    app.prune_toasts();
    assert!(
        !app.toasts().is_alive(toast_id),
        "dismissed toast should no longer be alive after exit animation"
    );

    // Another unreachable signal (e.g. the next fetch fails) must
    // re-push a fresh toast instead of silently doing nothing.
    app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::GitHub,
    });
    let new_id = app
        .net()
        .github()
        .availability()
        .toast_id()
        .expect("second unreachable signal should retain a toast id");
    assert_ne!(
        new_id, toast_id,
        "a fresh toast should be pushed with a new id"
    );
    assert!(
        app.toasts().is_alive(new_id),
        "the new toast should be visible"
    );
}
