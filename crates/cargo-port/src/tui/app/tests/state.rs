use super::*;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::project::WorktreeGroup;

#[test]
fn lint_runtime_waits_for_scan_completion() {
    let project = make_project(Some("demo"), "~/demo");
    let path = project.display_path();
    let mut app = make_app(&[project]);

    assert!(app.lint_runtime_projects_snapshot().is_empty());

    app.scan.phase = ScanPhase::Complete;
    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_label, path.to_string());
}

#[test]
fn workspace_members_show_parent_owner_ci_without_storing_member_state() {
    let workspace = make_workspace_project(Some("ws"), "~/ws");
    let member = make_project(Some("core"), "~/ws/core");
    let root = make_workspace_with_members(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
    );

    let mut app = make_app(&[workspace, member]);
    apply_items(&mut app, &[root]);

    app.insert_ci_runs(
        test_path("~/ws").as_path(),
        vec![make_ci_run(1, Conclusion::Success)],
    );

    assert_eq!(
        app.ci_for(test_path("~/ws").as_path()),
        Some(Conclusion::Success)
    );
    assert!(app.ci_state.contains_key(test_path("~/ws").as_path()));
    assert_eq!(
        app.ci_for(test_path("~/ws/core").as_path()),
        Some(Conclusion::Success)
    );
    assert!(app.ci_state_for(test_path("~/ws/core").as_path()).is_some());
    assert!(!app.ci_state.contains_key(test_path("~/ws/core").as_path()));
}

#[test]
fn non_owner_member_ignores_stale_member_state_and_fetches_via_owner() {
    let workspace = make_workspace_project(Some("ws"), "~/ws");
    let member = make_project(Some("core"), "~/ws/core");
    let root = make_workspace_with_members(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
    );

    let mut app = make_app(&[workspace, member.clone()]);
    apply_items(&mut app, &[root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();
    app.select_project_in_tree(member.path());

    app.ci_state.insert(
        member.path().to_path_buf(),
        CiState::Loaded {
            runs:      vec![make_ci_run(2, Conclusion::Failure)],
            exhausted: false,
        },
    );
    app.handle_git_info(
        test_path("~/ws").as_path(),
        make_git_info(Some("https://github.com/natepiano/demo")),
    );

    assert!(app.ci_state_for(member.path()).is_none());
    assert_eq!(app.ci_for(member.path()), None);
    assert!(app.bottom_panel_available(member.path()));

    crate::tui::detail::handle_ci_runs_key(
        &mut app,
        &crossterm::event::KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
    );
    assert_eq!(
        app.pending_ci_fetch
            .as_ref()
            .map(|fetch| fetch.project_path.clone()),
        Some(test_path("~/ws").display().to_string())
    );
}

#[test]
fn ci_rollup_uses_only_root_and_immediate_worktrees() {
    let member = make_project(Some("core"), "~/ws/core");

    let primary_ws = make_workspace_raw(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
        None,
    );
    let linked_ws = make_workspace_raw(
        Some("ws_feat"),
        "~/ws_feat",
        vec![inline_group(vec![make_member(
            Some("feat_core"),
            "~/ws_feat/core",
        )])],
        Some("ws_feat"),
    );
    let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);
    let root_path = test_path("~/ws");
    let feature_path = test_path("~/ws_feat");

    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws"), member.clone()]);
    apply_items(&mut app, &[root]);

    app.ci_state.insert(
        root_path,
        CiState::Loaded {
            runs:      vec![make_ci_run(3, Conclusion::Success)],
            exhausted: false,
        },
    );
    app.ci_state.insert(
        feature_path,
        CiState::Loaded {
            runs:      vec![make_ci_run(4, Conclusion::Failure)],
            exhausted: false,
        },
    );
    app.ci_state.insert(
        member.path().to_path_buf(),
        CiState::Loaded {
            runs:      vec![make_ci_run(5, Conclusion::Success)],
            exhausted: false,
        },
    );

    assert_eq!(app.ci_for_item(&app.projects[0]), Some(Conclusion::Failure));
    assert!(app.ci_state_for(member.path()).is_some());
}

#[test]
fn ci_for_prefers_runs_matching_local_branch() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.handle_git_info(
        project.path(),
        GitInfo {
            origin:              GitOrigin::Clone,
            branch:              Some("feat/demo".to_string()),
            owner:               Some("acme".to_string()),
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        None,
            default_branch:      Some("main".to_string()),
            ahead_behind_origin: None,
            local_main_branch:   Some("main".to_string()),
            ahead_behind_local:  None,
            workflows:           WorkflowPresence::Present,
        },
    );
    app.ci_state.insert(
        project.path().to_path_buf(),
        CiState::Loaded {
            runs:      vec![
                CiRun {
                    branch: "main".to_string(),
                    ..make_ci_run(9, Conclusion::Success)
                },
                CiRun {
                    branch: "feat/demo".to_string(),
                    ..make_ci_run(8, Conclusion::Failure)
                },
            ],
            exhausted: false,
        },
    );

    assert_eq!(app.ci_for(project.path()), Some(Conclusion::Failure));
}

#[test]
fn ci_for_default_branch_uses_full_repo_run_list() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.handle_git_info(
        project.path(),
        GitInfo {
            origin:              GitOrigin::Clone,
            branch:              Some("main".to_string()),
            owner:               Some("acme".to_string()),
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        None,
            default_branch:      Some("main".to_string()),
            ahead_behind_origin: None,
            ahead_behind_local:  None,
            workflows:           WorkflowPresence::Present,
        },
    );
    app.ci_state.insert(
        project.path().to_path_buf(),
        CiState::Loaded {
            runs:      vec![
                CiRun {
                    branch: "release".to_string(),
                    ..make_ci_run(9, Conclusion::Failure)
                },
                CiRun {
                    branch: "main".to_string(),
                    ..make_ci_run(8, Conclusion::Success)
                },
            ],
            exhausted: false,
        },
    );

    assert_eq!(app.ci_for(project.path()), Some(Conclusion::Failure));
}

#[test]
fn ci_toggle_switches_non_default_branch_between_branch_only_and_all_runs() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.handle_git_info(
        project.path(),
        GitInfo {
            origin:              GitOrigin::Clone,
            branch:              Some("feat/demo".to_string()),
            owner:               Some("acme".to_string()),
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        None,
            default_branch:      Some("main".to_string()),
            ahead_behind_origin: None,
            ahead_behind_local:  None,
            workflows:           WorkflowPresence::Present,
        },
    );
    app.ci_state.insert(
        project.path().to_path_buf(),
        CiState::Loaded {
            runs:      vec![
                CiRun {
                    branch: "main".to_string(),
                    ..make_ci_run(9, Conclusion::Success)
                },
                CiRun {
                    branch: "feat/demo".to_string(),
                    ..make_ci_run(8, Conclusion::Failure)
                },
            ],
            exhausted: false,
        },
    );

    assert_eq!(app.ci_for(project.path()), Some(Conclusion::Failure));
    assert_eq!(
        app.ci_runs_for_display(project.path())
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["feat/demo"]
    );

    app.toggle_ci_display_mode_for(project.path());

    assert_eq!(app.ci_for(project.path()), Some(Conclusion::Success));
    assert_eq!(
        app.ci_runs_for_display(project.path())
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "feat/demo"]
    );
}

#[test]
fn startup_lint_expectation_tracks_running_startup_lints() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(&[project_a.clone(), project_b]);
    app.scan.phase = ScanPhase::Complete;

    app.initialize_startup_phase_tracker();

    let expected = app
        .scan
        .startup_phases
        .lint_expected
        .as_ref()
        .expect("lint expected");
    assert!(expected.is_empty());
    assert!(app.lint_toast.is_none());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_a.path().to_path_buf().into(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });

    let expected = app
        .scan
        .startup_phases
        .lint_expected
        .as_ref()
        .expect("lint expected");
    assert_eq!(expected.len(), 1);
    assert!(expected.contains(project_a.path()));
    assert!(
        !app.scan
            .startup_phases
            .lint_seen_terminal
            .contains(project_a.path())
    );
    assert!(app.running_lint_paths.contains_key(project_a.path()));
    assert!(app.lint_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_a.path().to_path_buf().into(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });

    assert!(app.scan.startup_phases.lint_complete_at.is_some());
    assert!(app.running_lint_paths.is_empty());
    app.prune_toasts();
}

#[test]
fn startup_lint_toast_body_shows_paths_then_others() {
    let expected = HashSet::from([
        test_path("~/a"),
        test_path("~/b"),
        test_path("~/c"),
        test_path("~/d"),
        test_path("~/e"),
    ]);
    let seen = HashSet::from([test_path("~/e")]);

    let body = App::startup_lint_toast_body_for(&expected, &seen);
    let lines: Vec<&str> = body.lines().collect();

    assert_eq!(lines.len(), 4);
    for line in lines {
        assert!(line.starts_with("~/"));
    }
}

#[test]
fn startup_git_expected_uses_top_level_git_directories() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let non_rust_dir = tmp.path().join(".claude");
    let workspace_dir = tmp.path().join("bevy");
    let primary_dir = tmp.path().join("cargo-port");
    let linked_dir = tmp.path().join("cargo-port_feat");
    let member_dir = workspace_dir.join("crates").join("core");

    std::fs::create_dir_all(non_rust_dir.join(".git")).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(workspace_dir.join(".git")).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(primary_dir.join(".git")).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());

    let non_rust = RootItem::NonRust(NonRustProject::new(
        non_rust_dir.clone(),
        Some(".claude".to_string()),
    ));
    let workspace = RootItem::Workspace(RustProject::<Workspace>::new(
        workspace_dir.clone(),
        Some("bevy".to_string()),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        vec![inline_group(vec![RustProject::<Package>::new(
            member_dir,
            Some("core".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            Vec::new(),
            None,
            None,
        )])],
        Vec::new(),
        None,
        None,
    ));
    let primary = RustProject::<Package>::new(
        primary_dir.clone(),
        Some("cargo-port".to_string()),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        None,
        None,
    );
    let linked = RustProject::<Package>::new(
        linked_dir,
        Some("cargo-port_feat".to_string()),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        Some("cargo-port_feat".to_string()),
        Some(primary_dir.clone()),
    );
    let worktrees = RootItem::PackageWorktrees(WorktreeGroup::new(primary, vec![linked]));

    let mut app = make_app(&[]);
    apply_items(&mut app, &[non_rust, workspace, worktrees]);
    app.scan.phase = ScanPhase::Complete;

    app.initialize_startup_phase_tracker();

    assert_eq!(
        app.scan.startup_phases.git_expected,
        HashSet::from([
            non_rust_dir.join(".git"),
            workspace_dir.join(".git"),
            primary_dir.join(".git"),
        ])
    );
}

#[test]
fn startup_git_seen_marks_owner_git_directory_for_member_updates() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let workspace_dir = tmp.path().join("bevy");
    let member_dir = workspace_dir.join("crates").join("core");
    std::fs::create_dir_all(workspace_dir.join(".git")).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());

    let workspace = RootItem::Workspace(RustProject::<Workspace>::new(
        workspace_dir.clone(),
        Some("bevy".to_string()),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        vec![inline_group(vec![RustProject::<Package>::new(
            member_dir.clone(),
            Some("core".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            Vec::new(),
            None,
            None,
        )])],
        Vec::new(),
        None,
        None,
    ));

    let mut app = make_app(&[]);
    apply_items(&mut app, &[workspace]);
    app.scan.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    app.handle_git_info(member_dir.as_path(), make_git_info(None));

    assert!(
        app.scan
            .startup_phases
            .git_seen
            .contains(&workspace_dir.join(".git"))
    );
}

#[test]
fn lint_toast_reuses_existing_on_restart() {
    let project = make_project(Some("a"), "~/a");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.phase = ScanPhase::Complete;

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path().to_path_buf().into(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });
    let first_toast = app.lint_toast;
    assert!(first_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path().to_path_buf().into(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });
    assert_eq!(app.lint_toast, first_toast);

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path().to_path_buf().into(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
    });
    assert_eq!(app.lint_toast, first_toast);
}

#[test]
fn lint_runtime_snapshot_uses_workspace_root_not_members() {
    let workspace = make_workspace_project(Some("hana"), "~/rust/hana");
    let member_a = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
    let member_b = make_project(Some("hana_ui"), "~/rust/hana/crates/hana_ui");
    let root = make_workspace_with_members(
        Some("hana"),
        "~/rust/hana",
        vec![inline_group(vec![
            make_member(Some("hana_core"), "~/rust/hana/crates/hana_core"),
            make_member(Some("hana_ui"), "~/rust/hana/crates/hana_ui"),
        ])],
    );

    let mut app = make_app(&[workspace, member_a, member_b]);
    apply_items(&mut app, &[root]);
    app.scan.phase = ScanPhase::Complete;

    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_label, "~/rust/hana");
}

#[test]
fn lint_runtime_snapshot_deduplicates_primary_worktree_path() {
    let root_item = make_package_worktrees_item(
        make_package_raw(Some("ws"), "~/ws", None),
        vec![make_package_raw(
            Some("ws_feat"),
            "~/ws_feat",
            Some("ws_feat"),
        )],
    );
    let feature_item = make_project(Some("ws_feat"), "~/ws_feat");

    let mut app = make_app(&[make_project(Some("ws"), "~/ws"), feature_item]);
    apply_items(&mut app, &[root_item]);
    app.scan.phase = ScanPhase::Complete;

    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].project_label, "~/ws");
    assert_eq!(projects[1].project_label, "~/ws_feat");
}

#[test]
fn vendored_path_dependency_becomes_cargo_active() {
    let root_item = {
        let pkg = RustProject::<Package>::new(
            test_path("~/app"),
            Some("app".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            vec![make_member(Some("helper"), "~/app/vendor/helper")],
            None,
            None,
        );
        RootItem::Package(pkg)
    };
    let vendored = make_project(Some("helper"), "~/app/vendor/helper");

    let mut app = make_app(&[make_project(Some("app"), "~/app"), vendored.clone()]);
    apply_items(&mut app, &[root_item]);

    assert!(app.is_vendored_path(vendored.path()));
    assert!(app.is_cargo_active_path(vendored.path()));
}

#[test]
fn git_path_state_suppresses_sync_for_untracked_and_ignored() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    app.handle_git_info(
        project.path(),
        GitInfo {
            origin:              GitOrigin::Clone,
            branch:              Some("feat/demo".to_string()),
            owner:               None,
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        Some((2, 0)),
            default_branch:      Some("main".to_string()),
            ahead_behind_origin: None,
            local_main_branch:   Some("main".to_string()),
            ahead_behind_local:  None,
            workflows:           WorkflowPresence::Present,
        },
    );

    app.git_path_states
        .insert(project.path().to_path_buf(), GitPathState::Untracked);
    assert!(app.git_sync(project.path()).is_empty());

    app.git_path_states
        .insert(project.path().to_path_buf(), GitPathState::Ignored);
    assert!(app.git_sync(project.path()).is_empty());
}

#[test]
fn git_sync_shows_ascii_fill_for_local_only_branch() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    app.handle_git_info(
        project.path(),
        GitInfo {
            origin:              GitOrigin::Local,
            branch:              Some("feat/demo".to_string()),
            owner:               None,
            url:                 None,
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        None,
            default_branch:      None,
            ahead_behind_origin: None,
            local_main_branch:   Some("main".to_string()),
            ahead_behind_local:  Some((3, 0)),
            workflows:           WorkflowPresence::Present,
        },
    );

    assert_eq!(app.git_sync(project.path()), NO_REMOTE_SYNC);
}

#[test]
fn git_main_shows_synced_for_non_main_branch_in_sync_with_main() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    app.handle_git_info(
        project.path(),
        GitInfo {
            origin:              GitOrigin::Clone,
            branch:              Some("feat/demo".to_string()),
            owner:               None,
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        Some((0, 0)),
            default_branch:      Some("main".to_string()),
            ahead_behind_origin: Some((0, 0)),
            local_main_branch:   Some("main".to_string()),
            ahead_behind_local:  Some((0, 0)),
            workflows:           WorkflowPresence::Present,
        },
    );

    assert_eq!(app.git_main(project.path()), IN_SYNC);
}

#[test]
fn git_first_commit_arriving_before_git_info_is_preserved() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.list_state.select(Some(0));
    app.sync_selected_project();

    apply_bg_msg(
        &mut app,
        BackgroundMsg::GitFirstCommit {
            path:         test_path("~/demo").into(),
            first_commit: Some("2026-03-12T21:18:54-04:00".to_string()),
        },
    );
    apply_bg_msg(
        &mut app,
        BackgroundMsg::GitInfo {
            path: test_path("~/demo").into(),
            info: make_git_info(Some("https://github.com/natepiano/demo")),
        },
    );

    app.ensure_detail_cached();

    assert_eq!(
        app.git_info_for(test_path("~/demo").as_path())
            .and_then(|info| info.first_commit.as_deref()),
        Some("2026-03-12T21:18:54-04:00")
    );
    assert!(
        app.cached_detail
            .as_ref()
            .and_then(|cache| cache.info.git_inception.as_ref())
            .is_some(),
        "detail panel should show Incept once git info arrives"
    );
}

#[test]
fn lints_and_ci_panes_have_distinct_input_contexts() {
    let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);
    app.focus_pane(PaneId::CiRuns);
    assert_eq!(app.input_context(), InputContext::CiRuns);

    app.focus_pane(PaneId::Lints);
    assert_eq!(app.input_context(), InputContext::Lints);
}

#[test]
fn lint_rollups_distinguish_root_from_primary_worktree() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.current_config.lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.lint_status.insert(
        test_path("~/ws"),
        LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
    );
    app.lint_status.insert(
        test_path("~/ws_feat"),
        LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
    );
    app.rebuild_lint_rollups();

    assert!(matches!(
        app.lint_status_for_rollup_key(LintRollupKey::Root { node_index: 0 }),
        Some(LintStatus::Failed(_))
    ));
    assert!(matches!(
        app.lint_status_for_rollup_key(LintRollupKey::Worktree {
            node_index:     0,
            worktree_index: 0,
        }),
        Some(LintStatus::Passed(_))
    ));
    assert!(matches!(
        app.lint_status_for_rollup_key(LintRollupKey::Worktree {
            node_index:     0,
            worktree_index: 1,
        }),
        Some(LintStatus::Failed(_))
    ));
}

#[test]
fn lint_rollup_prefers_running_root_over_member_history() {
    let root = make_workspace_with_members(
        None,
        "~/ws",
        vec![inline_group(vec![make_member(Some("a"), "~/ws/a")])],
    );

    let mut app = make_app(&[make_workspace_project(None, "~/ws")]);
    app.current_config.lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.lint_status.insert(
        test_path("~/ws"),
        LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
    );
    app.lint_status.insert(
        test_path("~/ws/a"),
        LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
    );
    app.rebuild_lint_rollups();

    assert!(matches!(
        app.lint_status_for_rollup_key(LintRollupKey::Root { node_index: 0 }),
        Some(LintStatus::Running(_))
    ));
}

#[test]
fn lint_rollup_prefers_running_worktree_over_failed_root_history() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.current_config.lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.lint_status.insert(
        test_path("~/ws"),
        LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
    );
    app.lint_status.insert(
        test_path("~/ws_feat"),
        LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
    );
    app.rebuild_lint_rollups();

    assert!(matches!(
        app.lint_status_for_rollup_key(LintRollupKey::Root { node_index: 0 }),
        Some(LintStatus::Running(_))
    ));
    assert!(matches!(
        app.lint_status_for_rollup_key(LintRollupKey::Worktree {
            node_index:     0,
            worktree_index: 1,
        }),
        Some(LintStatus::Running(_))
    ));
}
