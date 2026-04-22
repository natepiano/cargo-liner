use super::*;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::project::AbsolutePath;
use crate::project::WorktreeGroup;
use crate::tui::panes;
use crate::tui::panes::DetailField;

#[test]
fn lint_runtime_waits_for_scan_completion() {
    let project = make_project(Some("demo"), "~/demo");
    let abs_path = test_path("~/demo");
    let mut app = make_app(&[project]);

    assert!(app.lint_runtime_projects_snapshot().is_empty());

    app.scan.phase = ScanPhase::Complete;
    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].abs_path, abs_path);
    assert_eq!(
        projects[0].project_label,
        crate::project::home_relative_path(&abs_path)
    );
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
        0,
    );

    assert_eq!(
        app.ci_for(test_path("~/ws").as_path()),
        Some(Conclusion::Success)
    );
    assert!(matches!(
        app.ci_data_for(test_path("~/ws").as_path()),
        Some(crate::project::ProjectCiData::Loaded(_))
    ));
    assert_eq!(
        app.ci_for(test_path("~/ws/core").as_path()),
        Some(Conclusion::Success)
    );
    assert!(app.ci_info_for(test_path("~/ws/core").as_path()).is_some());
    // Member resolves to the same entry-level ci_data as the workspace root.
    assert!(matches!(
        app.ci_data_for(test_path("~/ws/core").as_path()),
        Some(crate::project::ProjectCiData::Loaded(_))
    ));
}

#[test]
fn ci_fetch_on_member_targets_workspace_owner_path() {
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
    app.ensure_visible_rows_cached();
    app.select_project_in_tree(member.path());

    apply_git_info(
        &mut app,
        test_path("~/ws").as_path(),
        make_git_info(Some("https://github.com/natepiano/demo")),
    );

    panes::handle_ci_runs_key(
        &mut app,
        &crossterm::event::KeyEvent::new(KeyCode::Char('f'), crossterm::event::KeyModifiers::NONE),
    );
    assert_eq!(
        app.pending_ci_fetch
            .as_ref()
            .map(|fetch| fetch.project_path.clone()),
        Some(test_path("~/ws").display().to_string())
    );
}

#[test]
fn linked_worktree_shares_github_metadata_with_primary_after_repo_meta_fetch() {
    // Regression: previously `github_info` lived on each checkout's
    // `ProjectInfo` independently. A linked worktree on a branch without
    // an upstream never fired its own GitHub fetch, so the About field
    // stayed empty even after the primary's fetch landed. Stage 1 moves
    // `github_info` onto `GitRepo` (per ProjectEntry) so all checkouts of
    // the same repo see the same description.
    let primary_ws = make_workspace_raw(Some("ws"), "~/ws", vec![], None);
    let linked_ws = make_workspace_raw(Some("ws_feat"), "~/ws_feat", vec![], Some("ws_feat"));
    let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");

    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
    apply_items(&mut app, &[root]);

    app.handle_repo_meta(primary_path.as_path(), 42, Some("a great repo".to_string()));

    let read_description = |p: &std::path::Path| {
        app.projects()
            .entry_containing(p)
            .and_then(|entry| entry.git_repo.as_ref())
            .and_then(|repo| repo.github_info.as_ref())
            .and_then(|gh| gh.description.clone())
    };

    assert_eq!(
        read_description(primary_path.as_path()),
        Some("a great repo".to_string()),
    );
    assert_eq!(
        read_description(linked_path.as_path()),
        Some("a great repo".to_string()),
        "linked worktree should see the primary's fetched description",
    );
}

#[test]
fn worktree_group_shares_ci_data_across_primary_and_linked() {
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

    set_loaded_ci(
        &mut app,
        root_path.as_path(),
        vec![make_ci_run(3, Conclusion::Success)],
        false,
        0,
    );

    // Linked worktree resolves to the same per-repo ci_data slot.
    assert!(matches!(
        app.ci_data_for(feature_path.as_path()),
        Some(crate::project::ProjectCiData::Loaded(_))
    ));
    // Member inside the workspace also shares the entry-level ci_data.
    assert!(app.ci_info_for(member.path()).is_some());
}

#[test]
fn ci_for_prefers_runs_matching_local_branch() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        Some("acme".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![
            CiRun {
                branch: "main".to_string(),
                ..make_ci_run(9, Conclusion::Success)
            },
            CiRun {
                branch: "feat/demo".to_string(),
                ..make_ci_run(8, Conclusion::Failure)
            },
        ],
        false,
        0,
    );

    assert_eq!(app.ci_for(project.path()), Some(Conclusion::Failure));
}

#[test]
fn ci_for_default_branch_prefers_matching_branch_runs() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("main".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        Some("acme".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![
            CiRun {
                branch: "release".to_string(),
                ..make_ci_run(9, Conclusion::Failure)
            },
            CiRun {
                branch: "main".to_string(),
                ..make_ci_run(8, Conclusion::Success)
            },
        ],
        false,
        0,
    );

    assert_eq!(app.ci_for(project.path()), Some(Conclusion::Success));
    assert_eq!(
        app.ci_runs_for_display(project.path())
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );
}

#[test]
fn ci_toggle_switches_non_default_branch_between_branch_only_and_all_runs() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        Some("acme".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![
            CiRun {
                branch: "main".to_string(),
                ..make_ci_run(9, Conclusion::Success)
            },
            CiRun {
                branch: "feat/demo".to_string(),
                ..make_ci_run(8, Conclusion::Failure)
            },
        ],
        false,
        0,
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
        .lint
        .expected
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
        .lint
        .expected
        .as_ref()
        .expect("lint expected");
    assert_eq!(expected.len(), 1);
    assert!(expected.contains(project_a.path().as_path()));
    assert!(
        !app.scan
            .startup_phases
            .lint
            .seen
            .contains(project_a.path().as_path())
    );
    assert!(app.running_lint_paths.contains_key(project_a.path()));
    assert!(app.lint_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_a.path().to_path_buf().into(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });

    assert!(app.scan.startup_phases.lint.complete_at.is_some());
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
        AbsolutePath::from(non_rust_dir.clone()),
        Some(".claude".to_string()),
    ));
    let workspace = RootItem::Rust(RustProject::Workspace(Workspace {
        path: AbsolutePath::from(workspace_dir.clone()),
        name: Some("bevy".to_string()),
        groups: vec![inline_group(vec![Package {
            path: AbsolutePath::from(member_dir),
            name: Some("core".to_string()),
            ..Package::default()
        }])],
        ..Workspace::default()
    }));
    let primary = Package {
        path: AbsolutePath::from(primary_dir.clone()),
        name: Some("cargo-port".to_string()),
        worktree_status: WorktreeStatus::Primary {
            root: AbsolutePath::from(primary_dir.clone()),
        },
        ..Package::default()
    };
    let linked = Package {
        path: AbsolutePath::from(linked_dir),
        name: Some("cargo-port_feat".to_string()),
        worktree_status: WorktreeStatus::Linked {
            primary: AbsolutePath::from(primary_dir.clone()),
        },
        ..Package::default()
    };
    let worktrees = RootItem::Worktrees(WorktreeGroup::new_packages(primary, vec![linked]));

    let mut app = make_app(&[]);
    apply_items(&mut app, &[non_rust, workspace, worktrees]);
    app.scan.phase = ScanPhase::Complete;

    app.initialize_startup_phase_tracker();

    assert_eq!(
        app.scan.startup_phases.git.expected,
        Some(HashSet::from([
            AbsolutePath::from(non_rust_dir.join(".git")),
            AbsolutePath::from(workspace_dir.join(".git")),
            AbsolutePath::from(primary_dir.join(".git")),
        ]))
    );
}

#[test]
fn startup_git_seen_marks_owner_git_directory_for_member_updates() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let workspace_dir = tmp.path().join("bevy");
    let member_dir = workspace_dir.join("crates").join("core");
    std::fs::create_dir_all(workspace_dir.join(".git")).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());

    let workspace = RootItem::Rust(RustProject::Workspace(Workspace {
        path: AbsolutePath::from(workspace_dir.clone()),
        name: Some("bevy".to_string()),
        groups: vec![inline_group(vec![Package {
            path: AbsolutePath::from(member_dir.clone()),
            name: Some("core".to_string()),
            ..Package::default()
        }])],
        ..Workspace::default()
    }));

    let mut app = make_app(&[]);
    apply_items(&mut app, &[workspace]);
    app.scan.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    apply_git_info(&mut app, member_dir.as_path(), make_git_info(None));

    assert!(
        app.scan
            .startup_phases
            .git
            .seen
            .contains(workspace_dir.join(".git").as_path())
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
    assert_eq!(projects[0].abs_path, test_path("~/rust/hana"));
    assert_eq!(
        projects[0].project_label,
        crate::project::home_relative_path(test_path("~/rust/hana").as_path())
    );
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
    assert_eq!(projects[0].abs_path, test_path("~/ws"));
    assert_eq!(projects[1].abs_path, test_path("~/ws_feat"));
    assert_eq!(
        projects[0].project_label,
        crate::project::home_relative_path(test_path("~/ws").as_path())
    );
    assert_eq!(
        projects[1].project_label,
        crate::project::home_relative_path(test_path("~/ws_feat").as_path())
    );
}

#[test]
fn vendored_path_dependency_becomes_ci_owner() {
    let root_item = {
        let pkg = Package {
            path: test_path("~/app"),
            name: Some("app".to_string()),
            rust: RustInfo {
                vendored: vec![super::make_vendored(Some("helper"), "~/app/vendor/helper")],
                ..RustInfo::default()
            },
            ..Package::default()
        };
        RootItem::Rust(RustProject::Package(pkg))
    };
    let vendored = make_project(Some("helper"), "~/app/vendor/helper");

    let mut app = make_app(&[make_project(Some("app"), "~/app"), vendored.clone()]);
    apply_items(&mut app, &[root_item]);

    assert!(app.is_vendored_path(vendored.path()));
    assert!(
        app.projects().entry_containing(vendored.path()).is_some(),
        "vendored path should resolve to an owning ProjectEntry"
    );
}

#[test]
fn git_status_suppresses_sync_for_untracked_and_ignored() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    let base_info = || -> (CheckoutInfo, RepoInfo) {
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        None,
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: Some((2, 0)),
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        )
    };

    apply_git_info(&mut app, project.path(), base_info());

    apply_git_info(&mut app, project.path(), {
        let mut info = base_info();
        info.0.status = GitStatus::Untracked;
        info
    });
    assert!(app.git_sync(project.path()).is_empty());

    apply_git_info(&mut app, project.path(), {
        let mut info = base_info();
        info.0.status = GitStatus::Ignored;
        info
    });
    assert!(app.git_sync(project.path()).is_empty());
}

#[test]
fn background_git_info_updates_rendered_git_status() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.phase = ScanPhase::Complete;

    apply_bg_msg(
        &mut app,
        BackgroundMsg::RepoInfo {
            path: project.path().to_path_buf().into(),
            info: RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        None,
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: Some((1, 0)),
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        },
    );
    apply_bg_msg(
        &mut app,
        BackgroundMsg::CheckoutInfo {
            path: project.path().to_path_buf().into(),
            info: CheckoutInfo {
                status:              GitStatus::Modified,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
            },
        },
    );
    assert_eq!(
        app.git_status_for(project.path()),
        Some(GitStatus::Modified)
    );

    apply_bg_msg(
        &mut app,
        BackgroundMsg::RepoInfo {
            path: project.path().to_path_buf().into(),
            info: RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        None,
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: Some((1, 0)),
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        },
    );
    apply_bg_msg(
        &mut app,
        BackgroundMsg::CheckoutInfo {
            path: project.path().to_path_buf().into(),
            info: CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
            },
        },
    );
    assert_eq!(app.git_status_for(project.path()), Some(GitStatus::Clean));
}

#[test]
fn git_sync_shows_ascii_fill_for_local_only_branch() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  Some((3, 0)),
                primary_tracked_ref: None,
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          None,
                    owner:        None,
                    repo:         None,
                    tracked_ref:  None,
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    None,
                local_main_branch: Some("main".to_string()),
            },
        ),
    );

    assert_eq!(app.git_sync(project.path()), NO_REMOTE_SYNC);
}

#[test]
fn git_sync_shows_ascii_fill_for_branch_without_upstream() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feature/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  Some((2, 1)),
                primary_tracked_ref: None,
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/natepiano/demo".to_string()),
                    owner:        Some("natepiano".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  None,
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );

    assert_eq!(app.git_sync(project.path()), NO_REMOTE_SYNC);
}

#[test]
fn ci_empty_state_reports_unpublished_branch_when_no_upstream_exists() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.phase = ScanPhase::Complete;
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("enh/various".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: None,
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/natepiano/demo".to_string()),
                    owner:        Some("natepiano".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  None,
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![CiRun {
            branch: "main".to_string(),
            ..make_ci_run(9, Conclusion::Success)
        }],
        false,
        0,
    );

    let ci_data = panes::build_ci_data(&app);
    assert_eq!(
        ci_data.empty_state.title(),
        " No CI runs for unpublished branch enh/various "
    );
}

#[test]
fn package_details_show_unpublished_branch_for_ci_when_branch_has_no_upstream() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.phase = ScanPhase::Complete;
    app.pane_manager.pane_mut(PaneId::ProjectList).set_pos(0);
    app.sync_selected_project();

    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("enh/various".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: None,
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/natepiano/demo".to_string()),
                    owner:        Some("natepiano".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  None,
                    ahead_behind: None,
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );
    set_loaded_ci(
        &mut app,
        project.path(),
        vec![CiRun {
            branch: "main".to_string(),
            ..make_ci_run(57, Conclusion::Success)
        }],
        false,
        1,
    );
    app.ensure_detail_cached();

    let value = DetailField::Ci.package_value(
        app.pane_data
            .package
            .as_ref()
            .unwrap_or_else(|| std::process::abort()),
        &app,
    );

    assert_eq!(value, crate::constants::NO_CI_UNPUBLISHED_BRANCH);
}

#[test]
fn git_main_shows_synced_for_non_main_branch_in_sync_with_main() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                branch:              Some("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  Some((0, 0)),
                primary_tracked_ref: Some("origin/main".to_string()),
            },
            RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          Some("https://github.com/acme/demo".to_string()),
                    owner:        None,
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: Some((0, 0)),
                    kind:         RemoteKind::Clone,
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );

    assert_eq!(app.git_main(project.path()), IN_SYNC);
}

#[test]
fn git_first_commit_arriving_before_git_info_is_preserved() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.pane_manager.pane_mut(PaneId::ProjectList).set_pos(0);
    app.sync_selected_project();

    apply_bg_msg(
        &mut app,
        BackgroundMsg::GitFirstCommit {
            path:         test_path("~/demo"),
            first_commit: Some("2026-03-12T21:18:54-04:00".to_string()),
        },
    );
    let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::RepoInfo {
            path: test_path("~/demo"),
            info: repo,
        },
    );
    apply_bg_msg(
        &mut app,
        BackgroundMsg::CheckoutInfo {
            path: test_path("~/demo"),
            info: checkout,
        },
    );

    app.ensure_detail_cached();

    assert_eq!(
        app.repo_info_for(test_path("~/demo").as_path())
            .and_then(|repo| repo.first_commit.as_deref()),
        Some("2026-03-12T21:18:54-04:00")
    );
    assert!(
        app.pane_data
            .git
            .as_ref()
            .and_then(|g| g.inception.as_ref())
            .is_some(),
        "detail panel should show Incept once git info arrives"
    );
}

#[test]
fn git_info_invalidates_selected_git_pane_cache() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.pane_manager.pane_mut(PaneId::ProjectList).set_pos(0);
    app.sync_selected_project();
    app.ensure_detail_cached();

    assert_eq!(
        app.pane_data
            .git
            .as_ref()
            .and_then(|data| data.remotes.first())
            .and_then(|row| row.full_url.as_deref()),
        None
    );

    apply_git_info(
        &mut app,
        test_path("~/demo").as_path(),
        make_git_info(Some("https://github.com/natepiano/demo")),
    );
    app.ensure_detail_cached();

    assert_eq!(
        app.pane_data
            .git
            .as_ref()
            .and_then(|data| data.remotes.first())
            .and_then(|row| row.full_url.as_deref()),
        Some("https://github.com/natepiano/demo")
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
    app.projects_mut()
        .lint_at_path_mut(&test_path("~/ws"))
        .unwrap()
        .set_status(LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")));
    app.projects_mut()
        .lint_at_path_mut(&test_path("~/ws_feat"))
        .unwrap()
        .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));

    let root_status = app.projects().first().unwrap().lint_rollup_status();
    assert!(matches!(root_status, LintStatus::Failed(_)));

    let RootItem::Worktrees(g) = &app.projects().first().unwrap().item else {
        panic!("expected Worktrees");
    };
    assert!(matches!(
        g.lint_status_for_worktree(0),
        LintStatus::Passed(_)
    ));
    assert!(matches!(
        g.lint_status_for_worktree(1),
        LintStatus::Failed(_)
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
    app.projects_mut()
        .lint_at_path_mut(&test_path("~/ws"))
        .unwrap()
        .set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));

    let root_status = app.projects().first().unwrap().lint_rollup_status();
    assert!(matches!(root_status, LintStatus::Running(_)));
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
    app.projects_mut()
        .lint_at_path_mut(&test_path("~/ws"))
        .unwrap()
        .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));
    app.projects_mut()
        .lint_at_path_mut(&test_path("~/ws_feat"))
        .unwrap()
        .set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));

    let root_status = app.projects().first().unwrap().lint_rollup_status();
    assert!(matches!(root_status, LintStatus::Running(_)));

    let RootItem::Worktrees(g) = &app.projects().first().unwrap().item else {
        panic!("expected Worktrees");
    };
    assert!(matches!(
        g.lint_status_for_worktree(1),
        LintStatus::Running(_)
    ));
}

// ── CI fetch pipeline tests ───────────────────────────────────────────

#[test]
fn sync_does_not_mark_exhausted_when_no_new_runs() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    let path = project.path().display().to_string();

    set_loaded_ci(
        &mut app,
        project.path(),
        vec![make_ci_run(5, Conclusion::Success)],
        false,
        10,
    );

    // Sync returns the same run — no new runs found.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::Loaded {
            runs:         vec![make_ci_run(5, Conclusion::Success)],
            github_total: 10,
        },
        CiFetchKind::Sync,
    );

    let state = loaded_ci(&app, project.path());
    assert!(
        !state.exhausted,
        "Sync should not mark exhausted when no new runs found"
    );
}

#[test]
fn fetch_older_marks_exhausted_when_no_new_runs() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    let path = project.path().display().to_string();

    set_loaded_ci(
        &mut app,
        project.path(),
        vec![make_ci_run(5, Conclusion::Success)],
        false,
        10,
    );

    // FetchOlder returns the same run — no new runs found.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::Loaded {
            runs:         vec![make_ci_run(5, Conclusion::Success)],
            github_total: 10,
        },
        CiFetchKind::FetchOlder,
    );

    let state = loaded_ci(&app, project.path());
    assert!(
        state.exhausted,
        "FetchOlder should mark exhausted when no new runs found"
    );
}

#[test]
fn cache_only_preserves_github_total() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    let path = project.path().display().to_string();

    set_loaded_ci(
        &mut app,
        project.path(),
        vec![make_ci_run(5, Conclusion::Success)],
        false,
        57,
    );

    // CacheOnly (network failed) should preserve the previous github_total.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::CacheOnly(vec![make_ci_run(5, Conclusion::Success)]),
        CiFetchKind::Sync,
    );

    let state = loaded_ci(&app, project.path());
    assert_eq!(
        state.github_total, 57,
        "CacheOnly should preserve previous github_total"
    );
}

#[test]
fn sync_clears_exhaustion_when_new_runs_found() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    let path = project.path().display().to_string();

    set_loaded_ci(
        &mut app,
        project.path(),
        vec![make_ci_run(5, Conclusion::Success)],
        true,
        10,
    );

    // Sync finds a new run — should clear exhaustion.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::Loaded {
            runs:         vec![
                make_ci_run(6, Conclusion::Success),
                make_ci_run(5, Conclusion::Success),
            ],
            github_total: 11,
        },
        CiFetchKind::Sync,
    );

    let state = loaded_ci(&app, project.path());
    assert!(
        !state.exhausted,
        "Sync should clear exhaustion when new runs found"
    );
    assert_eq!(state.runs.len(), 2);
}

#[test]
fn fetch_more_uses_sync_when_no_cached_runs() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_git_info(
        &mut app,
        project.path(),
        make_git_info(Some("https://github.com/natepiano/demo")),
    );

    // Empty CI state — no cached runs.
    set_loaded_ci(&mut app, project.path(), Vec::new(), false, 57);

    app.select_project_in_tree(project.path());

    panes::handle_ci_runs_key(
        &mut app,
        &crossterm::event::KeyEvent::new(KeyCode::Char('f'), crossterm::event::KeyModifiers::NONE),
    );

    let fetch = app.pending_ci_fetch.as_ref().expect("fetch should be set");
    assert!(
        matches!(fetch.kind, CiFetchKind::Sync),
        "should use Sync when no cached runs exist"
    );
}
