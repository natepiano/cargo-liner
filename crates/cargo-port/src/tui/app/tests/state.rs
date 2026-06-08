use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use cargo_metadata::PackageId;
use cargo_metadata::TargetKind;
use cargo_metadata::semver::Version;

use super::*;
use crate::config;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::lint::CachedLintStatus;
use crate::lint::LintRun;
use crate::lint::LintRunStatus;
use crate::project::AbsolutePath;
use crate::project::FileStamp;
use crate::project::HeadState;
use crate::project::ManifestFingerprint;
use crate::project::PackageRecord;
use crate::project::ProjectPrData;
use crate::project::ProjectPrInfo;
use crate::project::PublishPolicy;
use crate::project::PullRequestCompleteness;
use crate::project::PullRequestGoneReason;
use crate::project::PullRequestInfo;
use crate::project::PullRequestState;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorkspaceMetadata;
use crate::project::WorktreeGroup;
use crate::project::WorktreeStatus;
use crate::scan::CargoMetadataError;
use crate::tui::app::phase_state::Denominator;
use crate::tui::app::target_index::CleanSelection;
use crate::tui::constants::STARTUP_ROW_MIN_VISIBLE;
use crate::tui::keymap::CiRunsAction;
use crate::tui::keymap::LintsAction;
use crate::tui::panes;
use crate::tui::state::StartupNetworkReadiness;
use crate::tui::terminal::CleanMsg;

fn test_pull_request_info(number: u32, title: &str) -> PullRequestInfo {
    test_pull_request_info_with_state(number, title, PullRequestState::Ready)
}

fn test_pull_request_info_with_state(
    number: u32,
    title: &str,
    state: PullRequestState,
) -> PullRequestInfo {
    PullRequestInfo {
        number,
        title: title.to_string(),
        url: format!("https://github.com/natepiano/cargo-port/pull/{number}"),
        state,
        head: "feat/open-prs".to_string(),
        head_owner: Some("natepiano".to_string()),
        head_repo: Some("cargo-port".to_string()),
        base: "main".to_string(),
    }
}

fn test_pr_info(open: Vec<PullRequestInfo>) -> ProjectPrInfo {
    ProjectPrInfo {
        open,
        default_branch: "main".to_string(),
        fetched_at: "2026-05-27T20:51:11Z".to_string(),
        completeness: PullRequestCompleteness::Complete,
        viewer_login: "natepiano".to_string(),
        owner_repo: crate::ci::OwnerRepo::new("natepiano", "cargo-port"),
    }
}

fn test_pr_data(open: Vec<PullRequestInfo>) -> ProjectPrData {
    ProjectPrData::Loaded(test_pr_info(open))
}

#[test]
fn lint_runtime_waits_for_scan_completion() {
    let project = make_project(Some("demo"), "~/demo");
    let abs_path = test_path("~/demo");
    let mut app = make_app(&[project]);

    assert!(app.lint_runtime_projects().is_empty());

    app.scan.state.phase = ScanPhase::Complete;
    let projects = app.lint_runtime_projects();
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
        vec![make_ci_run(1, CiStatus::Passed)],
        0,
    );

    assert_eq!(
        app.project_list
            .ci_status_using_lookup(test_path("~/ws").as_path(), &app.ci.status_lookup()),
        Some(CiStatus::Passed)
    );
    assert!(matches!(
        app.project_list.ci_data_for(test_path("~/ws").as_path()),
        Some(crate::project::ProjectCiData::Loaded(_))
    ));
    assert_eq!(
        app.project_list
            .ci_status_using_lookup(test_path("~/ws/core").as_path(), &app.ci.status_lookup()),
        Some(CiStatus::Passed)
    );
    assert!(
        app.project_list
            .ci_info_for(test_path("~/ws/core").as_path())
            .is_some()
    );
    // Member resolves to the same entry-level ci_data as the workspace root.
    assert!(matches!(
        app.project_list
            .ci_data_for(test_path("~/ws/core").as_path()),
        Some(crate::project::ProjectCiData::Loaded(_))
    ));
}

#[test]
fn workspace_member_ci_toggle_branch_and_mode_match_workspace_root() {
    let workspace = make_workspace_project(Some("ws"), "~/ws");
    let member = make_project(Some("core"), "~/ws/core");
    let root = make_workspace_with_members(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
    );
    let mut app = make_app(&[workspace, member]);
    apply_items(&mut app, &[root]);

    apply_git_info(
        &mut app,
        test_path("~/ws").as_path(),
        make_git_info(Some("https://github.com/natepiano/ws")),
    );
    app.insert_ci_runs(
        test_path("~/ws").as_path(),
        vec![
            CiRun {
                branch: "main".to_string(),
                ..make_ci_run(1, CiStatus::Passed)
            },
            CiRun {
                branch: "feature".to_string(),
                ..make_ci_run(2, CiStatus::Failed)
            },
        ],
        0,
    );

    let ws = test_path("~/ws");
    let core = test_path("~/ws/core");

    // The member resolves its CI branch and toggle to the workspace root,
    // so the all/branch filter is offered on the member just like on the
    // parent, and the default branch-only view filters the shared runs to
    // the workspace branch.
    assert!(app.ci_toggle_available_for(ws.as_path()));
    assert!(app.ci_toggle_available_for(core.as_path()));
    assert_eq!(
        app.project_list.current_branch_for(core.as_path()),
        Some("main")
    );
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(core.as_path(), &app.ci)
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );

    // Toggling on the member writes the owner's mode, so the workspace
    // root sees All too — the toggle state is shared, not per-row.
    app.set_ci_display_mode_for(core.as_path(), CiRunDisplayMode::All);
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(ws.as_path(), &app.ci)
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "feature"]
    );
}

#[test]
fn vendored_crate_ci_toggle_and_branch_resolve_to_checkout_root() {
    let vendored_path = "~/app/vendor/helper";
    let member = make_package_with_vendored(
        Some("member"),
        "~/app/crates/member",
        vec![super::make_vendored(Some("helper"), vendored_path)],
    );
    let root_item = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
        Some("app"),
        "~/app",
        vec![inline_group(vec![member])],
        None,
    )));
    let mut app = make_app(&[make_workspace_project(Some("app"), "~/app")]);
    apply_items(&mut app, &[root_item]);

    apply_git_info(
        &mut app,
        test_path("~/app").as_path(),
        make_git_info(Some("https://github.com/natepiano/app")),
    );
    app.insert_ci_runs(
        test_path("~/app").as_path(),
        vec![
            CiRun {
                branch: "main".to_string(),
                ..make_ci_run(1, CiStatus::Passed)
            },
            CiRun {
                branch: "feature".to_string(),
                ..make_ci_run(2, CiStatus::Failed)
            },
        ],
        0,
    );

    let helper = test_path(vendored_path);

    // A vendored crate is not a lint owner, but it still lives inside the
    // workspace checkout — so its CI branch and toggle resolve to the
    // workspace root just like a member's.
    assert!(app.ci_toggle_available_for(helper.as_path()));
    assert_eq!(
        app.project_list.current_branch_for(helper.as_path()),
        Some("main")
    );
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(helper.as_path(), &app.ci)
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );
}

#[test]
fn pull_request_disappearance_pushes_deleted_toast() {
    let project = make_project(Some("cargo-port"), "~/cargo-port");
    let path = test_path("~/cargo-port");
    let mut app = make_app(&[project]);
    apply_git_info(
        &mut app,
        path.as_path(),
        make_git_info(Some("https://github.com/natepiano/cargo-port")),
    );
    let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");

    apply_bg_msg(
        &mut app,
        BackgroundMsg::PullRequests {
            repo: repo.clone(),
            data: test_pr_data(vec![test_pull_request_info(1, "test: exercise PR toast")]),
        },
    );
    assert!(
        app.framework
            .toasts
            .active_now()
            .iter()
            .all(|toast| !toast.title().starts_with("Pull request")),
        "initial PR load should not announce deletion"
    );
    apply_bg_msg(
        &mut app,
        BackgroundMsg::PullRequests {
            repo: repo.clone(),
            data: ProjectPrData::Loading(Some(test_pr_info(vec![test_pull_request_info(
                1,
                "test: exercise PR toast",
            )]))),
        },
    );
    assert!(
        app.framework
            .toasts
            .active_now()
            .iter()
            .all(|toast| !toast.title().starts_with("Pull request")),
        "loading refresh should preserve the old PR without announcing deletion"
    );

    apply_bg_msg(
        &mut app,
        BackgroundMsg::PullRequests {
            repo: repo.clone(),
            data: test_pr_data(Vec::new()),
        },
    );

    apply_bg_msg(
        &mut app,
        BackgroundMsg::PullRequestDisappeared {
            repo,
            pull_request: test_pull_request_info(1, "test: exercise PR toast"),
            reason: PullRequestGoneReason::Merged {
                base: "main".to_string(),
            },
        },
    );

    let toast = app
        .framework
        .toasts
        .active_now()
        .into_iter()
        .find(|toast| toast.title() == "Pull request merged")
        .expect("merged PR toast should be visible");
    assert!(toast.body().contains("natepiano/cargo-port"));
    assert!(toast.body().contains("#1 test: exercise PR toast"));
    assert!(toast.body().contains("merged into main"));
}

#[test]
fn open_pull_request_count_does_not_change_project_list_label() {
    let project = make_project(Some("cargo-port"), "~/cargo-port");
    let path = test_path("~/cargo-port");
    let mut app = make_app(&[project]);
    apply_git_info(
        &mut app,
        path.as_path(),
        make_git_info(Some("https://github.com/natepiano/cargo-port")),
    );
    apply_bg_msg(
        &mut app,
        BackgroundMsg::PullRequests {
            repo: crate::ci::OwnerRepo::new("natepiano", "cargo-port"),
            data: test_pr_data(vec![test_pull_request_info(5, "feat: poll PR check state")]),
        },
    );

    let labels = app
        .project_list
        .resolved_root_labels(app.config.include_non_rust().includes_non_rust());

    assert_eq!(labels, vec!["cargo-port"]);
}

#[test]
fn pull_request_checks_finished_pushes_toast() {
    let project = make_project(Some("cargo-port"), "~/cargo-port");
    let path = test_path("~/cargo-port");
    let mut app = make_app(&[project]);
    apply_git_info(
        &mut app,
        path.as_path(),
        make_git_info(Some("https://github.com/natepiano/cargo-port")),
    );
    let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");
    app.net.github.insert_pr_check_poll(repo.clone(), 7);

    apply_bg_msg(
        &mut app,
        BackgroundMsg::PullRequests {
            repo,
            data: test_pr_data(vec![test_pull_request_info_with_state(
                7,
                "test: exercise PR check marker",
                PullRequestState::Ready,
            )]),
        },
    );

    let toast = app
        .framework
        .toasts
        .active_now()
        .into_iter()
        .find(|toast| toast.title() == "Pull request checks finished")
        .expect("checks-finished toast should be visible");
    assert!(toast.body().contains("#7 test: exercise PR check marker"));
    assert!(toast.body().contains("is ready"));
}

#[test]
fn active_pull_request_check_poll_keeps_animation_tick_live() {
    let project = make_project(Some("cargo-port"), "~/cargo-port");
    let mut app = make_app(&[project]);
    app.scan.state.phase = ScanPhase::Complete;

    assert_eq!(app.animation_timeout(), Duration::from_secs(1));

    app.net
        .github
        .insert_pr_check_poll(crate::ci::OwnerRepo::new("natepiano", "cargo-port"), 7);

    assert_eq!(app.animation_timeout(), Duration::from_millis(80));
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
    app.project_list.expanded.insert(ExpandKey::Node(0));
    app.ensure_visible_rows_cached();
    app.project_list
        .select_project_in_tree(member.path(), false);

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
        app.inflight
            .pending_ci_fetch_ref()
            .as_ref()
            .map(|fetch| fetch.project_path.clone()),
        Some(test_path("~/ws").display().to_string())
    );
}

#[test]
fn linked_worktree_shares_github_metadata_with_primary_after_repo_meta_fetch() {
    // Regression: a linked worktree on a branch without an upstream
    // never fires its own GitHub fetch, so the About field would stay
    // empty even after the primary's fetch landed. `github_info` lives
    // on `GitRepo` (per ProjectEntry) so all checkouts of the same repo
    // see the same description.
    let primary_ws = make_workspace_raw(Some("ws"), "~/ws", vec![], None);
    let linked_ws = make_workspace_raw(Some("ws_feat"), "~/ws_feat", vec![], Some("ws_feat"));
    let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");

    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
    apply_items(&mut app, &[root]);

    app.project_list
        .handle_repo_meta(primary_path.as_path(), 42, Some("a great repo".to_string()));

    let read_description = |p: &Path| {
        app.project_list
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
        vec![make_ci_run(3, CiStatus::Passed)],
        false,
        0,
    );

    // Linked worktree resolves to the same per-repo ci_data slot.
    assert!(matches!(
        app.project_list.ci_data_for(feature_path.as_path()),
        Some(crate::project::ProjectCiData::Loaded(_))
    ));
    // Member inside the workspace also shares the entry-level ci_data.
    assert!(app.project_list.ci_info_for(member.path()).is_some());
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
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
                ..make_ci_run(9, CiStatus::Passed)
            },
            CiRun {
                branch: "feat/demo".to_string(),
                ..make_ci_run(8, CiStatus::Failed)
            },
        ],
        false,
        0,
    );

    assert_eq!(
        app.project_list
            .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
        Some(CiStatus::Failed)
    );
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
                head:                HeadState::Branch("main".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
                ..make_ci_run(9, CiStatus::Failed)
            },
            CiRun {
                branch: "main".to_string(),
                ..make_ci_run(8, CiStatus::Passed)
            },
        ],
        false,
        0,
    );

    assert_eq!(
        app.project_list
            .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
        Some(CiStatus::Passed)
    );
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(project.path(), &app.ci)
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
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
                ..make_ci_run(9, CiStatus::Passed)
            },
            CiRun {
                branch: "feat/demo".to_string(),
                ..make_ci_run(8, CiStatus::Failed)
            },
        ],
        false,
        0,
    );

    assert_eq!(
        app.project_list
            .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
        Some(CiStatus::Failed)
    );
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(project.path(), &app.ci)
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["feat/demo"]
    );

    app.set_ci_display_mode_for(project.path(), CiRunDisplayMode::All);

    assert_eq!(
        app.project_list
            .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
        Some(CiStatus::Passed)
    );
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(project.path(), &app.ci)
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main", "feat/demo"]
    );
}

#[test]
fn startup_lint_history_completes_when_loaded_from_disk() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(&[project_a.clone(), project_b.clone()]);
    app.scan.state.phase = ScanPhase::Complete;

    app.initialize_startup_phase_tracker();

    // The "Lint history" row is seeded with every Rust project whose history
    // will be read from disk — it tracks the load, never live lint runs.
    let expected = app
        .startup
        .lint_phase
        .expected
        .keys()
        .expect("lint expected");
    assert_eq!(expected.len(), 2);
    assert!(expected.contains(project_a.path().as_path()));
    assert!(expected.contains(project_b.path().as_path()));
    assert!(app.startup.lint_phase.complete_at.is_none());

    // The single off-thread history-load batch marks every project seen and
    // completes the row.
    app.handle_bg_msg(BackgroundMsg::LintHistoryLoaded {
        entries: vec![
            (project_a.path().to_path_buf().into(), Vec::new()),
            (project_b.path().to_path_buf().into(), Vec::new()),
        ],
    });

    assert!(app.startup.lint_phase.complete_at.is_some());
    app.prune_toasts();
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
    let worktrees = RootItem::Worktrees(WorktreeGroup::new(
        RustProject::Package(primary),
        vec![RustProject::Package(linked)],
    ));

    let mut app = make_app(&[]);
    apply_items(&mut app, &[non_rust, workspace, worktrees]);
    app.scan.state.phase = ScanPhase::Complete;

    app.initialize_startup_phase_tracker();

    assert_eq!(
        app.startup.git.expected.keys(),
        Some(&HashSet::from([
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
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    apply_git_info(&mut app, member_dir.as_path(), make_git_info(None));

    assert!(
        app.startup
            .git
            .seen
            .contains(workspace_dir.join(".git").as_path())
    );
}

#[test]
fn lint_toast_reuses_existing_on_restart() {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = temp_dir.path().join("a");
    std::fs::create_dir_all(&project_dir).expect("project dir");
    std::fs::write(
        project_dir.join("Cargo.toml"),
        "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("cargo toml");
    let project = item_from_project_dir(&project_dir);
    let project_path = project.path().clone();
    let mut app = make_app(&[project]);
    app.config.current_mut().lint.enabled = true;
    app.config.current_mut().lint.include = vec![project_path.to_string_lossy().to_string()];
    app.scan.state.phase = ScanPhase::Complete;

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_path.clone(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });
    let first_toast = app.lint.running_toast_id();
    assert!(first_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_path.clone(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });
    assert_eq!(app.lint.running_toast_id(), first_toast);

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_path,
        status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
    });
    assert_eq!(app.lint.running_toast_id(), first_toast);
}

#[test]
fn lint_toast_prunes_entries_that_are_not_running_in_project_state() {
    let project = make_project(Some("a"), "~/a");
    let mut app = make_app(std::slice::from_ref(&project));
    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   test_path("~/a"),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });
    assert!(
        app.lint
            .running_toast_contains_path(test_path("~/a").as_path())
    );

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   test_path("~/a"),
        status: LintStatus::NoLog,
    });

    assert!(app.lint.running_toast_is_empty());
    assert!(lint_toast_running_items(&app).is_empty());
}

#[test]
fn startup_catch_up_batch_titles_running_toast_distinctly() {
    let project = make_project(Some("a"), "~/a");
    let mut app = make_app(std::slice::from_ref(&project));

    // The startup kickoff queues the catch-up batch; the first running status
    // then creates the single running-lint toast. It must read "Catch-up
    // lints" — no separate one-shot toast and no plain "Lints" toast.
    app.lint.queue_catch_up_lints();
    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   test_path("~/a"),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });

    let titles: Vec<String> = app
        .framework
        .toasts
        .active_now()
        .iter()
        .map(|toast| toast.title().to_string())
        .collect();
    assert!(
        titles.iter().any(|title| title == "Catch-up lints"),
        "the catch-up batch titles the running toast distinctly: {titles:?}"
    );
    assert!(
        !titles.iter().any(|title| title == "Lints"),
        "no separate plain Lints toast is created for the catch-up batch: {titles:?}"
    );
}

#[test]
fn startup_lint_status_does_not_overwrite_live_running_lint() {
    let project = make_project(Some("a"), "~/a");
    let project_path = project.path().clone();
    let mut app = make_app(&[project]);
    app.config.current_mut().lint.enabled = true;
    app.scan.state.phase = ScanPhase::Complete;

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_path.clone(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });
    let first_toast = app.lint.running_toast_id();

    app.handle_bg_msg(BackgroundMsg::LintStartupStatus {
        path:   project_path.clone(),
        status: CachedLintStatus::NoLog,
    });

    assert!(matches!(
        crate::tui::state::Lint::status_for_root(&app.project_list[0].item),
        LintStatus::Running(_)
    ));
    assert_eq!(app.lint.running_toast_id(), first_toast);
    assert!(app.lint.running_toast_contains_path(project_path.as_path()));
}

#[test]
fn live_lint_status_updates_project_model_and_detail_cache() {
    let project = make_project(Some("a"), "~/a");
    let project_path = project.path().clone();
    let mut app = make_app(&[project]);
    app.config.current_mut().lint.enabled = true;
    app.scan.state.phase = ScanPhase::Complete;
    app.project_list.set_cursor(0);
    app.sync_selected_project();
    app.ensure_detail_cached();

    assert!(matches!(
        &app.panes.package.content().unwrap().lint_display,
        panes::LintDisplay::NoRuns
    ));
    let generation_before = app.scan.generation();

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_path.clone(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });

    assert!(
        app.scan.generation() > generation_before,
        "live lint status must invalidate cached detail panes"
    );
    assert!(matches!(
        crate::tui::state::Lint::status_for_root(&app.project_list[0].item),
        LintStatus::Running(_)
    ));
    assert!(app.lint.running_toast_contains_path(project_path.as_path()));

    app.ensure_detail_cached();
    let display = app.panes.package.content().unwrap().lint_display.clone();
    assert!(
        matches!(
            display,
            panes::LintDisplay::Runs {
                count:  0,
                status: LintStatus::Running(_),
            }
        ),
        "{display:?}"
    );
}

#[test]
fn lint_runtime_projects_uses_workspace_root_not_members() {
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
    app.scan.state.phase = ScanPhase::Complete;

    let projects = app.lint_runtime_projects();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].abs_path, test_path("~/rust/hana"));
    assert_eq!(
        projects[0].project_label,
        crate::project::home_relative_path(test_path("~/rust/hana").as_path())
    );
}

#[test]
fn lint_runtime_projects_deduplicates_primary_worktree_path() {
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
    app.scan.state.phase = ScanPhase::Complete;

    let projects = app.lint_runtime_projects();
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

    assert!(app.project_list.is_vendored_path(vendored.path()));
    assert!(
        app.project_list.entry_containing(vendored.path()).is_some(),
        "vendored path should resolve to an owning ProjectEntry"
    );
}

#[test]
fn member_vendored_path_receives_project_info_updates() {
    let vendored_path = test_path("~/app/vendor/helper");
    let member = make_package_with_vendored(
        Some("member"),
        "~/app/crates/member",
        vec![super::make_vendored(Some("helper"), "~/app/vendor/helper")],
    );
    let root_item = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
        Some("app"),
        "~/app",
        vec![inline_group(vec![member])],
        None,
    )));
    let mut app = make_app(&[make_workspace_project(Some("app"), "~/app")]);
    apply_items(&mut app, &[root_item]);

    app.handle_disk_usage(vendored_path.as_path(), 4097);
    app.project_list.handle_language_stats_batch(vec![(
        vendored_path.clone(),
        crate::project::LanguageStats {
            entries: vec![crate::project::LangEntry {
                language: "Rust".to_string(),
                files:    1,
                code:     7,
                comments: 0,
                blanks:   0,
                children: Vec::new(),
            }],
        },
    )]);
    app.project_list.handle_crates_io_version_msg(
        vendored_path.as_path(),
        "0.4.0".to_string(),
        None,
        3_208,
    );

    let vendored = app
        .project_list
        .vendored_at_path(vendored_path.as_path())
        .expect("member-owned vendored package should be addressable by path");
    assert_eq!(vendored.info.disk_usage_bytes, Some(4097));
    assert_eq!(
        vendored
            .info
            .language_stats
            .as_ref()
            .map(|s| s.entries.len()),
        Some(1)
    );
    assert_eq!(vendored.crates_version(), Some("0.4.0"));
    assert_eq!(vendored.crates_downloads(), Some(3_208));
}

#[test]
fn project_refresh_preserves_crates_io_version() {
    let path = test_path("~/demo");
    let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);

    app.project_list.handle_crates_io_version_msg(
        path.as_path(),
        "0.20.2".to_string(),
        Some("0.21.0-rc.2".to_string()),
        663,
    );

    // A filesystem-triggered refresh re-scans the project. The fresh item has
    // no crates.io data (it is never persisted), so the refresh handler must
    // transfer the prior values rather than re-fetch from crates.io.
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed {
            item: make_project(Some("demo"), "~/demo"),
        },
    );

    let rust_info = app
        .project_list
        .rust_info_at_path(path.as_path())
        .expect("package should remain addressable after refresh");
    assert_eq!(rust_info.crates_version(), Some("0.20.2"));
    assert_eq!(rust_info.crates_prerelease(), Some("0.21.0-rc.2"));
    assert_eq!(rust_info.crates_downloads(), Some(663));
}

#[test]
fn member_vendored_path_receives_cargo_metadata_fields() {
    let workspace_path = test_path("~/app");
    let vendored_path = test_path("~/app/vendor/helper");
    let member = make_package_with_vendored(
        Some("member"),
        "~/app/crates/member",
        vec![super::make_vendored(Some("helper"), "~/app/vendor/helper")],
    );
    let root_item = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
        Some("app"),
        "~/app",
        vec![inline_group(vec![member])],
        None,
    )));
    let mut app = make_app(&[make_workspace_project(Some("app"), "~/app")]);
    apply_items(&mut app, &[root_item]);

    let record_id = PackageId {
        repr: "helper-id".into(),
    };
    let record = PackageRecord {
        name:          "helper".into(),
        version:       Version::new(0, 4, 0),
        edition:       "2024".into(),
        description:   None,
        license:       None,
        homepage:      None,
        repository:    None,
        manifest_path: AbsolutePath::from(vendored_path.as_path().join("Cargo.toml")),
        targets:       vec![crate::project::TargetRecord {
            name:              "helper".into(),
            kinds:             vec![TargetKind::Lib],
            required_features: vec![],
            src_path:          AbsolutePath::from(
                vendored_path.as_path().join("src").join("lib.rs"),
            ),
        }],
        publish:       PublishPolicy::Never,
    };
    let mut packages = HashMap::new();
    packages.insert(record_id, record);
    let workspace_metadata = WorkspaceMetadata {
        workspace_root: workspace_path,
        target_directory: test_path("~/app/target"),
        packages,
        fingerprint: fake_fingerprint(),
        out_of_tree_target_bytes: None,
    };

    app.project_list
        .apply_cargo_fields_from_workspace_metadata(&workspace_metadata);

    let cargo = &app
        .project_list
        .vendored_at_path(vendored_path.as_path())
        .expect("member-owned vendored package should receive cargo metadata")
        .cargo;
    assert!(
        cargo
            .types()
            .contains(&crate::project::ProjectType::Library)
    );
    assert!(!cargo.publishable());
}

#[test]
fn git_status_suppresses_sync_for_untracked_and_ignored() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));

    let base_info = || -> (CheckoutInfo, RepoInfo) {
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
    assert!(app.project_list.git_sync(project.path()).is_empty());

    apply_git_info(&mut app, project.path(), {
        let mut info = base_info();
        info.0.status = GitStatus::Ignored;
        info
    });
    assert!(app.project_list.git_sync(project.path()).is_empty());
}

#[test]
fn background_git_info_updates_rendered_git_status() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.state.phase = ScanPhase::Complete;

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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
            },
        },
    );
    assert_eq!(
        app.project_list.git_status_for(project.path()),
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
            },
        },
    );
    assert_eq!(
        app.project_list.git_status_for(project.path()),
        Some(GitStatus::Clean)
    );
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
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  Some((3, 0)),
                primary_tracked_ref: None,
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    None,
                local_main_branch: Some("main".to_string()),
            },
        ),
    );

    assert_eq!(app.project_list.git_sync(project.path()), NO_REMOTE_SYNC);
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
                head:                HeadState::Branch("feature/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  Some((2, 1)),
                primary_tracked_ref: None,
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );

    assert_eq!(app.project_list.git_sync(project.path()), NO_REMOTE_SYNC);
}

#[test]
fn ci_pane_shows_all_runs_for_unpublished_branch_without_toggle() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.state.phase = ScanPhase::Complete;
    app.project_list.set_cursor(0);
    app.sync_selected_project();
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                head:                HeadState::Branch("enh/various".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: None,
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
            ..make_ci_run(9, CiStatus::Passed)
        }],
        false,
        0,
    );

    // Unpublished branch (no upstream, not the default): the all/branch
    // toggle doesn't apply, so the pane shows every run unfiltered.
    assert!(!app.ci_toggle_available_for(project.path()));
    assert_eq!(
        app.project_list
            .ci_runs_for_ci_pane(project.path(), &app.ci)
            .iter()
            .map(|run| run.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["main"]
    );

    let ci_data = panes::build_ci_data(&app);
    assert!(ci_data.mode_label.is_none());
    assert_eq!(ci_data.runs.len(), 1);
}

#[test]
fn package_details_show_unpublished_branch_for_ci_when_branch_has_no_upstream() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.scan.state.phase = ScanPhase::Complete;
    app.project_list.set_cursor(0);
    app.sync_selected_project();

    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                head:                HeadState::Branch("enh/various".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: None,
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
            ..make_ci_run(57, CiStatus::Passed)
        }],
        false,
        1,
    );
    app.ensure_detail_cached();

    let display = app
        .panes
        .package
        .content()
        .unwrap_or_else(|| std::process::abort())
        .ci_display;

    assert_eq!(display, crate::tui::state::CiDisplay::UnpublishedBranch);
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
                head:                HeadState::Branch("feat/demo".to_string()),
                last_commit:         None,
                ahead_behind_local:  Some((0, 0)),
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      None,
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            },
        ),
    );

    assert_eq!(app.project_list.git_main(project.path()), IN_SYNC);
}

#[test]
fn git_first_commit_arriving_before_git_info_is_preserved() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.project_list.set_cursor(0);
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
        app.project_list
            .repo_info_for(test_path("~/demo").as_path())
            .and_then(|repo| repo.first_commit.as_deref()),
        Some("2026-03-12T21:18:54-04:00")
    );
    assert!(
        app.panes
            .git
            .content()
            .and_then(|g| g.inception.as_ref())
            .is_some(),
        "detail panel should show Incept once git info arrives"
    );
}

#[test]
fn git_info_invalidates_selected_git_pane_cache() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.project_list.set_cursor(0);
    app.sync_selected_project();
    app.ensure_detail_cached();

    assert_eq!(
        app.panes
            .git
            .content()
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
        app.panes
            .git
            .content()
            .and_then(|data| data.remotes.first())
            .and_then(|row| row.full_url.as_deref()),
        Some("https://github.com/natepiano/demo")
    );
}

#[test]
fn ensure_detail_cached_short_circuits_when_nothing_changed() {
    let project_a = make_project(Some("alpha"), "~/alpha");
    let project_b = make_project(Some("beta"), "~/beta");
    let mut app = make_app(&[project_a, project_b]);
    app.project_list.set_cursor(0);
    app.sync_selected_project();

    // Seed the cache.
    app.ensure_detail_cached();
    let after_seed = app.panes.pane_data.detail_build_count();
    assert!(after_seed >= 1, "first call must build");

    // Unchanged selection and generation — must NOT rebuild.
    app.ensure_detail_cached();
    app.ensure_detail_cached();
    assert_eq!(
        app.panes.pane_data.detail_build_count(),
        after_seed,
        "idle frames must not rebuild the detail set"
    );

    // Bumping the data generation invalidates the stamp → must rebuild.
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let after_generation_bump = app.panes.pane_data.detail_build_count();
    assert_eq!(
        after_generation_bump,
        after_seed + 1,
        "generation bump must trigger exactly one rebuild"
    );

    // Changing the selected row invalidates the stamp → must rebuild.
    app.project_list.set_cursor(1);
    app.sync_selected_project();
    app.ensure_detail_cached();
    assert_eq!(
        app.panes.pane_data.detail_build_count(),
        after_generation_bump + 1,
        "selection change must trigger exactly one rebuild"
    );

    // Same selection, same generation, twice more — still no rebuild.
    app.ensure_detail_cached();
    app.ensure_detail_cached();
    assert_eq!(
        app.panes.pane_data.detail_build_count(),
        after_generation_bump + 1,
        "further idle frames must not rebuild"
    );
}

#[test]
fn worktree_summary_or_compute_caches_until_tree_mutation() {
    // Two distinct call sites for the *same* group root must hit the
    // cache on the second call — the closure must not run twice.
    let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);
    let group_root = test_path("~/demo");
    let counter = std::sync::atomic::AtomicUsize::new(0);

    let _ = app
        .panes
        .git
        .worktree_summary_or_compute(group_root.as_path(), || {
            counter.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        });
    let _ = app
        .panes
        .git
        .worktree_summary_or_compute(group_root.as_path(), || {
            counter.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        });
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "second lookup must hit the cache, not recompute"
    );

    // A `TreeMutation` guard going out of scope must invalidate the
    // cache via its `Drop` impl, regardless of whether any actual
    // mutation methods were called. This is the type-level guarantee
    // the guard exists to provide.
    {
        let _guard = app.mutate_tree();
        // Drop here.
    }

    let _ = app
        .panes
        .git
        .worktree_summary_or_compute(group_root.as_path(), || {
            counter.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        });
    assert_eq!(
        counter.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "after TreeMutation drops, the next lookup must recompute"
    );
}

#[test]
fn background_message_for_unselected_path_does_not_invalidate_detail() {
    let project_a = make_project(Some("alpha"), "~/alpha");
    let project_b = make_project(Some("beta"), "~/beta");
    let mut app = make_app(&[project_a, project_b]);
    app.project_list.set_cursor(0);
    app.sync_selected_project();
    app.ensure_detail_cached();
    let baseline = app.panes.pane_data.detail_build_count();

    // A disk-usage message for a *different* project must not bump the
    // detail-cache key. Watchers fire dozens of these per second; if they
    // each invalidate, the cache reduces to a no-op (the original
    // regression).
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  test_path("~/beta"),
            bytes: 1024,
        },
    );
    app.ensure_detail_cached();
    assert_eq!(
        app.panes.pane_data.detail_build_count(),
        baseline,
        "unrelated background messages must not invalidate the detail cache"
    );

    // A message for the *selected* path must still invalidate.
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  test_path("~/alpha"),
            bytes: 2048,
        },
    );
    app.ensure_detail_cached();
    assert_eq!(
        app.panes.pane_data.detail_build_count(),
        baseline + 1,
        "messages affecting the selected path must rebuild exactly once"
    );
}

#[test]
fn lint_rollups_distinguish_root_from_primary_worktree() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.project_list
        .lint_at_path_mut(&test_path("~/ws"))
        .unwrap()
        .set_status(LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")));
    app.project_list
        .lint_at_path_mut(&test_path("~/ws_feat"))
        .unwrap()
        .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));

    let root_status = app.project_list.first().unwrap().lint_rollup_status();
    assert!(matches!(root_status, LintStatus::Failed(_)));

    let RootItem::Worktrees(g) = &app.project_list.first().unwrap().item else {
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
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.project_list
        .lint_at_path_mut(&test_path("~/ws"))
        .unwrap()
        .set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));

    let root_status = app.project_list.first().unwrap().lint_rollup_status();
    assert!(matches!(root_status, LintStatus::Running(_)));
}

#[test]
fn lint_rollup_prefers_running_worktree_over_failed_root_history() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.project_list
        .lint_at_path_mut(&test_path("~/ws"))
        .unwrap()
        .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));
    app.project_list
        .lint_at_path_mut(&test_path("~/ws_feat"))
        .unwrap()
        .set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));

    let root_status = app.project_list.first().unwrap().lint_rollup_status();
    assert!(matches!(root_status, LintStatus::Running(_)));

    let RootItem::Worktrees(g) = &app.project_list.first().unwrap().item else {
        panic!("expected Worktrees");
    };
    assert!(matches!(
        g.lint_status_for_worktree(1),
        LintStatus::Running(_)
    ));
}

#[test]
fn worktree_group_detail_lint_rollup_ignores_deleted_worktrees() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");
    app.project_list
        .at_path_mut(linked_path.as_path())
        .expect("linked worktree should exist")
        .visibility = Deleted;

    let make_lint_run = |run_id: &str, status| LintRun {
        run_id: run_id.to_string(),
        started_at: "2026-03-30T16:12:18-05:00".to_string(),
        finished_at: Some("2026-03-30T16:13:18-05:00".to_string()),
        duration_ms: Some(60_000),
        status,
        commands: Vec::new(),
        archive_bytes: 0,
    };
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_runs(vec![make_lint_run("primary", LintRunStatus::Passed)]);
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_status(LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")));
    app.project_list
        .lint_at_path_mut(&linked_path)
        .unwrap()
        .set_runs(vec![make_lint_run("linked", LintRunStatus::Failed)]);
    app.project_list
        .lint_at_path_mut(&linked_path)
        .unwrap()
        .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));

    app.project_list.set_cursor(0);
    app.sync_selected_project();
    app.ensure_detail_cached();

    let display = app.panes.package.content().unwrap().lint_display.clone();
    assert!(
        matches!(
            display,
            panes::LintDisplay::Runs {
                count:  1,
                status: LintStatus::Passed(_),
            }
        ),
        "{display:?}"
    );
}

#[test]
fn worktree_group_lints_pane_aggregates_every_checkout_newest_first() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");

    let run = |run_id: &str, started_at: &str| LintRun {
        run_id:        run_id.to_string(),
        started_at:    started_at.to_string(),
        finished_at:   None,
        duration_ms:   None,
        status:        LintRunStatus::Passed,
        commands:      Vec::new(),
        archive_bytes: 0,
    };
    // Primary has one (older) run; the linked checkout has two newer ones.
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_runs(vec![run("primary-1", "2026-03-30T10:00:00-04:00")]);
    app.project_list
        .lint_at_path_mut(&linked_path)
        .unwrap()
        .set_runs(vec![
            run("linked-2", "2026-03-30T12:00:00-04:00"),
            run("linked-1", "2026-03-30T11:00:00-04:00"),
        ]);

    // Select the group parent (header) row.
    app.project_list.set_cursor(0);
    app.sync_selected_project();

    let data = panes::build_lints_data(&app);

    // Every checkout's runs are merged, newest-first across checkouts.
    let ids: Vec<&str> = data.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["linked-2", "linked-1", "primary-1"]);

    // Both checkouts are owners; each run resolves to the checkout it came
    // from, so its logs open against the right cache directory.
    assert_eq!(data.owner_paths.len(), 2);
    assert_eq!(data.owner_path_for_run(0), Some(&linked_path));
    assert_eq!(data.owner_path_for_run(1), Some(&linked_path));
    assert_eq!(data.owner_path_for_run(2), Some(&primary_path));
}

#[test]
fn worktree_group_lints_pane_reindexes_when_a_new_run_lands() {
    // The owner index is not maintained incrementally — every new run bumps
    // the generation, which invalidates the detail cache and rebuilds the
    // whole merged list. This test drives that real refresh chain and
    // checks the rebuilt list re-sorts and the owner index follows.
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");

    let run = |run_id: &str, started_at: &str| LintRun {
        run_id:        run_id.to_string(),
        started_at:    started_at.to_string(),
        finished_at:   None,
        duration_ms:   None,
        status:        LintRunStatus::Passed,
        commands:      Vec::new(),
        archive_bytes: 0,
    };
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_runs(vec![run("primary-1", "2026-03-30T10:00:00-04:00")]);
    app.project_list
        .lint_at_path_mut(&linked_path)
        .unwrap()
        .set_runs(vec![run("linked-1", "2026-03-30T11:00:00-04:00")]);

    app.project_list.set_cursor(0);
    app.sync_selected_project();
    app.ensure_detail_cached();

    let before: Vec<&str> = app
        .lint
        .content()
        .unwrap()
        .runs
        .iter()
        .map(|r| r.run_id.as_str())
        .collect();
    assert_eq!(before, vec!["linked-1", "primary-1"]);

    // A newer run lands on the primary checkout (the loader replaces the
    // whole history per path). Bumping the generation invalidates the cache.
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_runs(vec![
            run("primary-2", "2026-03-30T12:00:00-04:00"),
            run("primary-1", "2026-03-30T10:00:00-04:00"),
        ]);
    app.scan.bump_generation();
    app.ensure_detail_cached();

    let data = app.lint.content().unwrap();
    let ids: Vec<&str> = data.runs.iter().map(|r| r.run_id.as_str()).collect();
    // Rebuilt newest-first, and the owner index realigns with the new order.
    assert_eq!(ids, vec!["primary-2", "linked-1", "primary-1"]);
    assert_eq!(data.owner_path_for_run(0), Some(&primary_path));
    assert_eq!(data.owner_path_for_run(1), Some(&linked_path));
    assert_eq!(data.owner_path_for_run(2), Some(&primary_path));
}

#[test]
fn clear_history_on_group_parent_clears_every_checkout() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");

    let run = |run_id: &str, started_at: &str| LintRun {
        run_id:        run_id.to_string(),
        started_at:    started_at.to_string(),
        finished_at:   None,
        duration_ms:   None,
        status:        LintRunStatus::Passed,
        commands:      Vec::new(),
        archive_bytes: 0,
    };
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_runs(vec![run("primary-1", "2026-03-30T10:00:00-04:00")]);
    app.project_list
        .lint_at_path_mut(&linked_path)
        .unwrap()
        .set_runs(vec![run("linked-1", "2026-03-30T11:00:00-04:00")]);

    // Select the group parent (header) row, where the pane aggregates every
    // checkout's history.
    app.project_list.set_cursor(0);
    app.sync_selected_project();

    panes::dispatch_lints_action(LintsAction::ClearHistory, &mut app);

    // Both checkouts' histories are gone — not just the primary's — so the
    // rebuilt aggregate is empty instead of re-showing the linked runs.
    assert!(
        app.project_list
            .lint_at_path_mut(&primary_path)
            .unwrap()
            .runs()
            .is_empty()
    );
    assert!(
        app.project_list
            .lint_at_path_mut(&linked_path)
            .unwrap()
            .runs()
            .is_empty()
    );
    assert!(panes::build_lints_data(&app).runs.is_empty());
}

#[test]
fn clear_history_toasts_run_count_and_freed_bytes_across_group() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    let primary_path = test_path("~/ws");
    let linked_path = test_path("~/ws_feat");

    let run = |run_id: &str, started_at: &str, archive_bytes: u64| LintRun {
        run_id: run_id.to_string(),
        started_at: started_at.to_string(),
        finished_at: None,
        duration_ms: None,
        status: LintRunStatus::Passed,
        commands: Vec::new(),
        archive_bytes,
    };
    // Two runs on the primary checkout, one on the linked checkout: three runs
    // totalling 3072 bytes (3.0 KiB) across the aggregate.
    app.project_list
        .lint_at_path_mut(&primary_path)
        .unwrap()
        .set_runs(vec![
            run("primary-2", "2026-03-30T12:00:00-04:00", 1024),
            run("primary-1", "2026-03-30T10:00:00-04:00", 1024),
        ]);
    app.project_list
        .lint_at_path_mut(&linked_path)
        .unwrap()
        .set_runs(vec![run("linked-1", "2026-03-30T11:00:00-04:00", 1024)]);

    app.project_list.set_cursor(0);
    app.sync_selected_project();

    panes::dispatch_lints_action(LintsAction::ClearHistory, &mut app);

    let toast = app
        .framework
        .toasts
        .active()
        .last()
        .expect("clearing lint history emits a toast");
    assert_eq!(toast.title(), "Lint history cleared");
    assert_eq!(toast.body_text(), "3 runs, 3.0 KiB freed");
}

#[test]
fn clear_ci_cache_toasts_removed_run_count() {
    // Point the app cache root at a tempdir so the real on-disk CI cache is
    // untouched and `remove_dir_all` lands on the success branch (where the
    // run count is reported).
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let mut cfg = CargoPortConfig::default();
    cfg.cache.root = tmp.path().to_string_lossy().into_owned();
    config::set_active_config(&cfg);

    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    apply_git_info(
        &mut app,
        project.path(),
        (
            CheckoutInfo {
                status:              GitStatus::Clean,
                head:                HeadState::Branch("main".to_string()),
                last_commit:         None,
                ahead_behind_local:  None,
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
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
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
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
            make_ci_run(9, CiStatus::Passed),
            make_ci_run(8, CiStatus::Failed),
        ],
        false,
        2,
    );
    // The repo's cache dir must exist for the clear to reach the success path.
    std::fs::create_dir_all(scan::ci_cache_dir_pub("acme", "demo").as_path())
        .unwrap_or_else(|_| std::process::abort());

    app.project_list.set_cursor(0);
    app.sync_selected_project();

    panes::dispatch_ci_runs_action(CiRunsAction::ClearCache, &mut app);

    let toast = app
        .framework
        .toasts
        .active()
        .last()
        .expect("clearing CI cache emits a toast");
    assert_eq!(toast.title(), "CI cache cleared");
    assert_eq!(toast.body_text(), "acme/demo: 2 runs");

    config::set_active_config(&CargoPortConfig::default());
}

#[test]
fn worktree_group_detail_lint_rollup_rebuilds_when_linked_worktree_finishes() {
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    app.config.current_mut().lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.project_list.set_cursor(0);
    app.sync_selected_project();

    let linked_path = test_path("~/ws_feat");
    let linked_lints = app.project_list.lint_at_path_mut(&linked_path).unwrap();
    linked_lints.set_runs(vec![LintRun {
        run_id:        "previous".to_string(),
        started_at:    "2026-03-30T16:12:18-05:00".to_string(),
        finished_at:   Some("2026-03-30T16:13:18-05:00".to_string()),
        duration_ms:   Some(60_000),
        status:        LintRunStatus::Passed,
        commands:      Vec::new(),
        archive_bytes: 0,
    }]);
    linked_lints.set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));
    app.scan.bump_generation();
    app.ensure_detail_cached();
    let running_display = app.panes.package.content().unwrap().lint_display.clone();
    assert!(
        matches!(
            running_display,
            panes::LintDisplay::Runs {
                status: LintStatus::Running(_),
                ..
            }
        ),
        "{running_display:?}"
    );

    apply_bg_msg(
        &mut app,
        BackgroundMsg::LintStatus {
            path:   linked_path,
            status: LintStatus::Passed(parse_ts("2026-03-30T16:23:18-05:00")),
        },
    );
    app.ensure_detail_cached();
    let finished_display = app.panes.package.content().unwrap().lint_display.clone();

    assert!(
        !matches!(
            finished_display,
            panes::LintDisplay::Runs {
                status: LintStatus::Running(_),
                ..
            }
        ),
        "{finished_display:?}"
    );
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
        vec![make_ci_run(5, CiStatus::Passed)],
        false,
        10,
    );

    // Sync returns the same run — no new runs found.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::Loaded {
            runs:         vec![make_ci_run(5, CiStatus::Passed)],
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
        vec![make_ci_run(5, CiStatus::Passed)],
        false,
        10,
    );

    // FetchOlder returns the same run — no new runs found.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::Loaded {
            runs:         vec![make_ci_run(5, CiStatus::Passed)],
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
        vec![make_ci_run(5, CiStatus::Passed)],
        false,
        57,
    );

    // CacheOnly (network failed) should preserve the previous github_total.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::CacheOnly(vec![make_ci_run(5, CiStatus::Passed)]),
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
        vec![make_ci_run(5, CiStatus::Passed)],
        true,
        10,
    );

    // Sync finds a new run — should clear exhaustion.
    app.handle_ci_fetch_complete(
        &path,
        CiFetchResult::Loaded {
            runs:         vec![
                make_ci_run(6, CiStatus::Passed),
                make_ci_run(5, CiStatus::Passed),
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

    app.project_list
        .select_project_in_tree(project.path(), false);

    panes::handle_ci_runs_key(
        &mut app,
        &crossterm::event::KeyEvent::new(KeyCode::Char('f'), crossterm::event::KeyModifiers::NONE),
    );

    let fetch = app
        .inflight
        .pending_ci_fetch_ref()
        .expect("fetch should be set");
    assert!(
        matches!(fetch.kind, CiFetchKind::Sync),
        "should use Sync when no cached runs exist"
    );
}

// ── Cargo metadata phase + arrival handling ─────────────────────────

fn fake_fingerprint() -> ManifestFingerprint {
    // Fields are irrelevant to the handler's accept path: if
    // `capture()` on the workspace_root succeeds at runtime it will
    // produce a real fingerprint that (almost certainly) differs from
    // this one and the arrival gets dropped as drift. Tests use
    // workspace_root paths that don't exist on disk so `capture()`
    // fails (returns None) and the drift check becomes a no-op.
    ManifestFingerprint {
        manifest:       FileStamp {
            content_hash: [0_u8; 32],
        },
        lockfile:       None,
        rust_toolchain: None,
        configs:        BTreeMap::new(),
    }
}

fn fake_metadata(workspace_root: &AbsolutePath) -> WorkspaceMetadata {
    WorkspaceMetadata {
        workspace_root:           workspace_root.clone(),
        target_directory:         AbsolutePath::from(workspace_root.as_path().join("target")),
        packages:                 HashMap::new(),
        fingerprint:              fake_fingerprint(),
        out_of_tree_target_bytes: None,
    }
}

fn lint_toast_running_items(app: &App) -> Vec<String> {
    app.framework
        .toasts
        .active_now()
        .iter()
        .find(|toast| toast.title() == "Lints")
        .map(|toast| {
            toast
                .tracked_items()
                .iter()
                .filter(|item| item.linger_progress.is_none())
                .map(|item| item.label.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

#[test]
fn initialize_startup_phase_seeds_metadata_expected() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let project_b = make_project(Some("b"), "~/never-real/b");
    let mut app = make_app(&[project_a.clone(), project_b.clone()]);
    app.scan.state.phase = ScanPhase::Complete;

    app.initialize_startup_phase_tracker();

    let expected = app
        .startup
        .metadata
        .expected
        .keys()
        .expect("metadata expected set is seeded at startup");
    assert_eq!(
        expected.len(),
        2,
        "one expected entry per Rust leaf, matching crate::tui::app::startup::initial_metadata_roots"
    );
    assert!(expected.contains(project_a.path()));
    assert!(expected.contains(project_b.path()));
}

/// Happy path: a successful arrival at the current generation inserts
/// the metadata into the store and advances `metadata.seen`.
#[test]
fn successful_metadata_arrival_advances_phase() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("store lock")
        .next_generation(&workspace_root);

    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation,
        fingerprint: fake_fingerprint(),
        result: Ok(fake_metadata(&workspace_root)),
    });

    assert!(
        app.startup.metadata.seen.contains(&workspace_root),
        "metadata.seen records the arrived workspace"
    );
    assert!(
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("store lock")
            .get(&workspace_root)
            .is_some(),
        "successful metadata was upserted into the store"
    );
    assert!(
        app.startup.metadata.complete_at.is_some(),
        "with only one expected root, the phase completes on arrival"
    );
}

/// Race guard: an arrival stamped with a generation older than the
/// current one is dropped. `metadata.seen` must not advance and the
/// store must not upsert.
#[test]
fn stale_generation_metadata_arrival_is_dropped() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
    let store = app.scan.metadata_store_handle();
    let stale_gen = store
        .lock()
        .expect("store")
        .next_generation(&workspace_root);
    // A later dispatch bumps the generation; the stale arrival below
    // should be rejected.
    let _newer_gen = store
        .lock()
        .expect("store")
        .next_generation(&workspace_root);

    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation:     stale_gen,
        fingerprint:    fake_fingerprint(),
        result:         Ok(fake_metadata(&workspace_root)),
    });

    assert!(
        !app.startup.metadata.seen.contains(&workspace_root),
        "stale-generation arrival does not advance metadata.seen"
    );
    assert!(
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("store")
            .get(&workspace_root)
            .is_none(),
        "stale-generation arrival does not upsert"
    );
}

/// Error path: a failed arrival surfaces a "cargo metadata failed"
/// timed toast and still ticks the phase forward (so startup doesn't
/// wedge on a permanent failure).
#[test]
fn failed_metadata_arrival_surfaces_error_toast() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("store")
        .next_generation(&workspace_root);

    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation,
        fingerprint: fake_fingerprint(),
        result: Err(CargoMetadataError::Other(
            "could not read Cargo.toml".into(),
        )),
    });

    let error_toast_present = app
        .framework
        .toasts
        .active_now()
        .iter()
        .any(|toast| toast.title().starts_with("cargo metadata failed"));
    assert!(
        error_toast_present,
        "failure raises a timed error toast starting with 'cargo metadata failed'"
    );
    assert!(
        app.startup.metadata.seen.contains(&workspace_root),
        "failure still ticks the phase forward so startup doesn't wedge"
    );
}

/// `WorkspaceMissing` (workspace deleted between dispatch and run, e.g. after
/// the user removes a worktree) must NOT raise a user-facing toast — it's a
/// stale-refresh race, not a real failure. Compare with the prior test which
/// asserts `Other` does raise a toast: the two together pin down the
/// dispatch contract on `CargoMetadataError`.
#[test]
fn cargo_metadata_workspace_missing_does_not_raise_toast() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let workspace_root = AbsolutePath::from(tmp.path().join("deleted_workspace"));
    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: workspace_root.clone(),
        name: Some("ghost".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);
    app.startup
        .metadata
        .reset_with_expected(std::iter::once(workspace_root.clone()).collect());

    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("store")
        .next_generation(&workspace_root);

    let toasts_before = app.framework.toasts.active_now().len();
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation,
        fingerprint: fake_fingerprint(),
        result: Err(CargoMetadataError::WorkspaceMissing),
    });

    assert_eq!(
        app.framework.toasts.active_now().len(),
        toasts_before,
        "WorkspaceMissing must not add any toast"
    );
    assert!(
        app.startup.metadata.seen.contains(&workspace_root),
        "WorkspaceMissing must still tick the phase forward"
    );
}

/// `start_clean` must prefer the workspace's resolved `target_directory`
/// (from the metadata store) over the default `<project>/target` — that
/// is the whole point of Step 2. Exercises three scenarios on a real
/// tempdir to catch regressions in both the metadata lookup and the
/// filesystem existence check.
#[test]
fn start_clean_prefers_resolved_target_dir_over_hardcoded_literal() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_path = AbsolutePath::from(tmp.path().join("proj"));
    let custom_target = AbsolutePath::from(tmp.path().join("out-of-tree-target"));
    std::fs::create_dir_all(project_path.as_path()).unwrap_or_else(|_| std::process::abort());
    std::fs::create_dir_all(custom_target.as_path()).unwrap_or_else(|_| std::process::abort());

    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: project_path.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);

    // Inject metadata pointing the project at the out-of-tree target.
    app.scan
        .metadata_store_handle()
        .lock()
        .expect("store")
        .upsert(WorkspaceMetadata {
            workspace_root:           project_path.clone(),
            target_directory:         custom_target,
            packages:                 HashMap::new(),
            fingerprint:              fake_fingerprint(),
            out_of_tree_target_bytes: None,
        });

    assert!(
        app.start_clean(&project_path),
        "out-of-tree target dir exists → clean is queued (would have missed with join(\"target\"))"
    );
    assert!(
        app.inflight
            .clean()
            .running
            .contains_key(project_path.as_path())
    );
}

#[test]
fn start_clean_reports_already_clean_when_resolved_target_is_missing() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_path = AbsolutePath::from(tmp.path().join("proj"));
    let custom_target = AbsolutePath::from(tmp.path().join("out-of-tree-target"));
    std::fs::create_dir_all(project_path.as_path()).unwrap_or_else(|_| std::process::abort());
    // Also create the default `<project>/target` — this must NOT make the
    // check pass, because the *resolved* target sits elsewhere and doesn't
    // exist on disk.
    std::fs::create_dir_all(project_path.as_path().join("target"))
        .unwrap_or_else(|_| std::process::abort());

    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: project_path.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);
    app.scan
        .metadata_store_handle()
        .lock()
        .expect("store")
        .upsert(WorkspaceMetadata {
            workspace_root:           project_path.clone(),
            target_directory:         custom_target,
            packages:                 HashMap::new(),
            fingerprint:              fake_fingerprint(),
            out_of_tree_target_bytes: None,
        });

    assert!(
        !app.start_clean(&project_path),
        "resolved target doesn't exist → already clean; in-tree target/ decoy must not trip it"
    );
    assert!(app.inflight.clean().is_empty());
}

#[test]
fn start_clean_falls_back_to_literal_target_when_no_metadata_yet() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_path = AbsolutePath::from(tmp.path().join("proj"));
    std::fs::create_dir_all(project_path.as_path().join("target"))
        .unwrap_or_else(|_| std::process::abort());

    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: project_path.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);

    assert!(
        app.start_clean(&project_path),
        "no metadata → falls back to <project>/target, which exists → clean queued"
    );
    assert!(
        app.inflight
            .clean()
            .running
            .contains_key(project_path.as_path())
    );
}

#[test]
fn disk_usage_update_does_not_finish_running_clean() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_path = AbsolutePath::from(tmp.path().join("proj"));
    std::fs::create_dir_all(project_path.as_path().join("target"))
        .unwrap_or_else(|_| std::process::abort());

    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: project_path.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);

    assert!(app.start_clean(&project_path));
    app.handle_disk_usage(project_path.as_path(), 0);

    assert!(
        app.inflight
            .clean()
            .running
            .contains_key(project_path.as_path()),
        "disk usage can update before cargo clean exits, so it must not clear the running clean"
    );
}

#[test]
fn clean_finished_message_finishes_running_clean() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let project_path = AbsolutePath::from(tmp.path().join("proj"));
    std::fs::create_dir_all(project_path.as_path().join("target"))
        .unwrap_or_else(|_| std::process::abort());

    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: project_path.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);

    assert!(app.start_clean(&project_path));
    app.background
        .clean_sender()
        .send(CleanMsg::Finished(project_path.clone()))
        .expect("send clean finish");
    app.poll_background();

    assert!(
        !app.inflight
            .clean()
            .running
            .contains_key(project_path.as_path()),
        "cargo clean process exit should clear the running clean"
    );
}

/// The metadata phase gates startup readiness: with disk, git, repo
/// phases all resolved but metadata still pending, startup must not be
/// marked complete. Once metadata arrives, the startup phase can close.
#[test]
fn startup_ready_waits_on_metadata_phase() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");

    // Force every phase except metadata complete (empty denominators
    // complete immediately) so only metadata gates startup readiness.
    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.maybe_complete_startup_disk(now, scan_started);
    app.maybe_complete_startup_git(now, scan_started);
    app.maybe_complete_startup_repo(now, scan_started);

    assert!(
        app.startup.disk.complete_at.is_some()
            && app.startup.git.complete_at.is_some()
            && app.startup.repo.complete_at.is_some(),
        "disk/git/repo phases are now complete"
    );
    assert!(
        app.startup.metadata.complete_at.is_none(),
        "metadata still pending"
    );
    app.maybe_complete_startup_ready(now, scan_started);
    assert!(
        app.startup.is_collecting(),
        "startup doesn't complete while metadata is still pending"
    );

    // Dispatch the metadata arrival → phase completes → startup ready.
    let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .expect("store")
        .next_generation(&workspace_root);
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation,
        fingerprint: fake_fingerprint(),
        result: Ok(fake_metadata(&workspace_root)),
    });

    assert!(
        app.startup.metadata.complete_at.is_some(),
        "metadata phase completes after the arrival"
    );
    // Every phase has resolved, but the panel holds each row visible until
    // its minimum-visible floor elapses; advance past it and re-check.
    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        !app.startup.is_collecting(),
        "startup is ready once every phase has resolved and the floor elapses"
    );
}

/// The languages row starts with project-root completion tokens, can add
/// file-level progress tokens, and marks `seen` as progress and final stats
/// batches apply. The test-count row stays keyed on project roots.
#[test]
fn startup_languages_and_tests_rows_track_their_batches() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let root = project_a.path().clone();
    assert!(
        app.startup
            .languages
            .expected
            .keys()
            .is_some_and(|expected| expected.contains(root.as_path())),
        "languages denominator is seeded from the project roots at scan start"
    );
    assert!(
        app.startup
            .tests
            .expected
            .keys()
            .is_some_and(|expected| expected.contains(root.as_path())),
        "tests denominator is seeded from the project roots at scan start"
    );
    assert!(app.startup.languages.seen.is_empty());
    assert!(app.startup.tests.seen.is_empty());

    let language_file: AbsolutePath = root.join("src").join("lib.rs").into();
    app.handle_bg_msg(BackgroundMsg::LanguageStatsProgressPlan {
        entries: vec![language_file.clone()],
    });
    assert!(
        app.startup
            .languages
            .expected
            .keys()
            .is_some_and(|expected| expected.contains(language_file.as_path())),
        "language progress plans add file-level tokens to the row denominator"
    );

    app.handle_bg_msg(BackgroundMsg::LanguageStatsProgressBatch {
        entries: vec![language_file.clone()],
    });
    assert!(
        app.startup.languages.seen.contains(language_file.as_path()),
        "language progress batches mark file-level tokens seen"
    );

    app.handle_bg_msg(BackgroundMsg::LanguageStatsBatch {
        entries: vec![(
            root.clone(),
            crate::project::LanguageStats { entries: vec![] },
        )],
    });
    app.handle_bg_msg(BackgroundMsg::TestCountsBatch {
        entries: vec![(root.clone(), crate::project::TestCounts::default())],
    });

    assert!(
        app.startup.languages.seen.contains(root.as_path()),
        "a language-stats batch marks its project root seen on the languages row"
    );
    assert!(
        app.startup.tests.seen.contains(root.as_path()),
        "a test-counts batch marks its project root seen on the tests row"
    );
}

/// The crates.io row seeds its denominator upfront and holds the panel open
/// until every seeded fetch reports complete.
#[test]
fn startup_crates_io_row_gates_until_fetches_complete() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");

    // Isolate crates.io: force every other row to an empty (immediately
    // complete) denominator, and seed crates.io with one expected crate.
    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::from(["serde".to_string()]));
    app.startup.crates_io.stamp_first_seen(now);
    app.maybe_log_startup_phase_completions();

    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        app.startup.is_collecting(),
        "panel stays open while a crates.io fetch is still pending"
    );

    app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
        name: "serde".to_string(),
    });
    assert!(
        app.startup.crates_io.seen.contains("serde"),
        "a crates.io fetch-complete marks the crate seen on the row"
    );

    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        !app.startup.is_collecting(),
        "panel closes once the crates.io row finishes and the floor elapses"
    );
}

/// Regression for startup completing before the startup crates.io plan had
/// been installed: zero-lint completion can fire immediately after startup
/// begins, but the planned crates.io denominator must already be present.
#[test]
fn startup_plan_installs_crates_io_before_zero_lint_completion_can_close() {
    let project_a = make_project(Some("demo"), "~/never-real/demo");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    assert!(
        app.startup
            .crates_io
            .expected
            .keys()
            .is_some_and(|expected| expected.contains("demo")),
        "startup plan seeds the crates.io row before completion checks can run"
    );

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");
    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.startup.details_declared.expected = Denominator::Stable(HashSet::new());
    app.handle_bg_msg(BackgroundMsg::LintStartupStatus {
        path:   project_a.path().clone(),
        status: CachedLintStatus::NoLog,
    });
    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);

    assert!(
        app.startup.is_collecting(),
        "zero-lint completion cannot close Startup while planned crates.io work is pending"
    );

    app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
        name: "demo".to_string(),
    });
    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        !app.startup.is_collecting(),
        "Startup can close after the planned crates.io fetch completes"
    );
}

#[test]
fn startup_readiness_waits_for_project_detail_declarations() {
    let project_a = make_project(Some("demo"), "~/never-real/demo");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");
    let detail_path = AbsolutePath::from(project_a.path().as_path().to_path_buf());

    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.startup.details_declared.expected =
        Denominator::Stable(HashSet::from([detail_path.clone()]));
    app.startup.details_declared.complete_at = None;
    app.maybe_log_startup_phase_completions();
    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);

    assert!(
        app.startup.is_collecting(),
        "Startup cannot close before planned detail workers declare follow-up work"
    );

    app.handle_bg_msg(BackgroundMsg::ProjectDetailsDeclared { path: detail_path });
    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        !app.startup.is_collecting(),
        "Startup can close after detail declarations are complete"
    );
}

/// A repo fetch queued after the GitHub row already completed reopens the
/// row, so the panel waits for the late fetch instead of closing early.
#[test]
fn startup_late_repo_fetch_reopens_github_row() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");

    // Git terminal + empty repo set → the GitHub row completes.
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.maybe_complete_startup_git(now, scan_started);
    app.maybe_complete_startup_repo(now, scan_started);
    assert!(
        app.startup.repo.complete_at.is_some(),
        "GitHub row completes when git is terminal and no repos are queued"
    );

    // A repo fetch queued after that completion reopens the row.
    let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");
    app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });
    assert!(
        app.startup.repo.complete_at.is_none(),
        "a late repo fetch reopens the completed GitHub row"
    );
    assert!(
        app.startup
            .repo
            .expected
            .keys()
            .is_some_and(|expected| expected.contains(&repo)),
        "the late repo joins the GitHub denominator"
    );

    // Completing it marks it seen and re-completes the row.
    app.handle_bg_msg(BackgroundMsg::RepoFetchComplete { repo: repo.clone() });
    assert!(
        app.startup.repo.seen.contains(&repo) && app.startup.repo.complete_at.is_some(),
        "completing the late fetch marks it seen and re-completes the row"
    );
}

/// A re-fetch of an already-seen repo un-marks it and reopens the GitHub row,
/// so the panel cannot close while the live tracker still contains that repo.
#[test]
fn startup_repo_refetch_reopens_completed_github_row() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");
    let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.maybe_complete_startup_git(now, scan_started);
    app.startup.repo.expected = Denominator::Stable(HashSet::from([repo.clone()]));
    app.startup.repo.seen.insert(repo.clone());
    app.maybe_complete_startup_repo(now, scan_started);
    assert!(
        app.startup.repo.complete_at.is_some(),
        "the seeded repo row starts complete"
    );

    app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });
    assert!(
        !app.startup.repo.seen.contains(&repo),
        "a queued re-fetch un-marks the repo"
    );
    assert!(
        app.startup.repo.complete_at.is_none(),
        "a queued re-fetch reopens the completed GitHub row"
    );
}

/// A crates.io fetch queued while the startup panel is open joins the
/// denominator, and a re-fetch of an already-seen name un-marks it — so
/// the row cannot read done while any registered fetch is in flight.
#[test]
fn startup_late_crates_io_fetch_reopens_row() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    // Seed one expected crate and complete it — the row is done.
    app.startup.crates_io.expected = Denominator::Stable(HashSet::from(["serde".to_string()]));
    app.startup
        .crates_io
        .stamp_first_seen(std::time::Instant::now());
    app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
        name: "serde".to_string(),
    });
    assert!(
        app.startup.crates_io.complete_at.is_some(),
        "row completes once the seeded fetch reports"
    );

    // A re-fetch of the same name un-marks it and reopens the row.
    app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
        name: "serde".to_string(),
    });
    assert!(
        !app.startup.crates_io.seen.contains("serde"),
        "a queued re-fetch un-marks the name"
    );
    assert!(
        app.startup.crates_io.complete_at.is_none(),
        "a queued re-fetch reopens the completed row"
    );

    // A fetch for a name outside the plan joins the denominator.
    app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
        name: "tokio".to_string(),
    });
    assert!(
        app.startup
            .crates_io
            .expected
            .keys()
            .is_some_and(|expected| expected.contains("tokio")),
        "a late fetch joins the crates.io denominator"
    );

    // Completing both re-completes the row.
    app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
        name: "serde".to_string(),
    });
    app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
        name: "tokio".to_string(),
    });
    assert!(
        app.startup.crates_io.complete_at.is_some(),
        "completing the late fetches re-completes the row"
    );
}

// ── network-toast stage (startup-owned vs steady state) ────────────

/// The network-toast stage is a three-state machine: it starts `StartupOwned`,
/// `begin_steady_state_network_toasts` installs the slots, and
/// `set_network_toasts_startup_owned` removes them again (the rescan path).
#[test]
fn network_toast_stage_round_trips_startup_owned_and_steady() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));

    assert!(
        app.net.network_toasts().is_none(),
        "construction starts in the startup-owned stage — no standalone slot"
    );
    begin_steady_state_network_toasts_for_test(&mut app);
    assert!(
        app.net.network_toasts().is_some(),
        "entering steady state installs the standalone-toast slots"
    );
    app.net.set_network_toasts_startup_owned();
    assert!(
        app.net.network_toasts().is_none(),
        "returning to startup-owned discards the slots"
    );
}

/// While the startup panel owns the network rows the stage is `StartupOwned`:
/// a queued crates.io fetch is still tracked in flight (the panel's detail row
/// reads it), but no standalone-toast slot exists, so the "Fetching crates.io
/// info" toast cannot be created.
#[test]
fn startup_owned_stage_suppresses_crates_io_standalone_toast() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    assert!(
        app.net.network_toasts().is_none(),
        "the open startup panel owns the network rows — no standalone slot exists"
    );

    app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
        name: "serde".to_string(),
    });

    assert!(
        app.net.crates_io_running().running.contains_key("serde"),
        "the queued fetch is still tracked in flight for the panel's detail row"
    );
    assert!(
        app.net.network_toasts().is_none(),
        "no standalone crates.io toast slot is created while the panel owns the row"
    );
}

/// While the startup panel owns the network rows, a queued GitHub fetch is
/// tracked for the panel detail line but cannot create the standalone
/// "Retrieving GitHub repo details" toast.
#[test]
fn startup_owned_stage_suppresses_github_standalone_toast() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    assert!(
        app.net.network_toasts().is_none(),
        "the open startup panel owns the network rows"
    );

    let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");
    app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });

    assert!(
        app.net.github_running().running.contains_key(&repo),
        "the queued fetch is tracked in flight for the panel's detail row"
    );
    assert!(
        app.net.network_toasts().is_none(),
        "no standalone GitHub toast slot is created while startup owns the row"
    );
}

/// Even if row bookkeeping regresses and every visible startup row looks
/// gate-satisfied, startup readiness is not constructible while a
/// startup-owned GitHub tracker still has running work.
#[test]
fn startup_readiness_waits_for_running_github_tracker() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");
    let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.maybe_log_startup_phase_completions();
    app.net
        .github_running_mut()
        .insert(repo, std::time::Instant::now());

    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        app.startup.is_collecting(),
        "startup cannot close while startup-owned GitHub work is still running"
    );
    assert!(
        app.net.network_toasts().is_none(),
        "the failed handoff does not install standalone network-toast slots"
    );
}

/// The spawn→queue window: a repo-fetch worker is registered in
/// `repo_fetch_in_flight` at spawn but only reaches the `github_running`
/// tracker once it sends `RepoFetchQueued`. Startup must not hand off to
/// steady state in that window, or the queue message lands after the panel
/// closes and leaks a standalone "Retrieving GitHub repo details" toast.
#[test]
fn startup_readiness_waits_for_spawned_but_unqueued_repo_fetch() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");
    let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());

    // The worker thread is spawned (registered in flight) but has not yet sent
    // `RepoFetchQueued`, so the `github_running` tracker stays empty — the row
    // and the network gate would both read drained without this guard. In the
    // real flow `RepoInfo` registers the fetch before the `CheckoutInfo` that
    // marks git terminal, so the row never completes first; clear the row's
    // init-time completion to model that ordering.
    app.net.github.repo_fetch_in_flight_mut().insert(repo);
    app.startup.repo.complete_at = None;
    app.maybe_log_startup_phase_completions();

    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        app.startup.is_collecting(),
        "startup cannot close while a repo fetch is spawned but not yet queued"
    );
    assert!(
        app.net.network_toasts().is_none(),
        "the panel keeps owning the network rows until the spawned fetch drains"
    );
}

/// The reported leak: a crates.io fetch processed before the scan completes
/// must not pop a standalone toast, and initializing the startup tracker must
/// seed the row from that already-running startup-owned tracker.
#[test]
fn crates_io_fetch_before_startup_panel_is_suppressed() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    // No `initialize_startup_phase_tracker`: the scan has not completed.

    assert!(
        app.net.network_toasts().is_none(),
        "the network-toast stage starts `StartupOwned` before any panel exists"
    );

    app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
        name: "serde".to_string(),
    });

    assert!(
        app.net.crates_io_running().running.contains_key("serde"),
        "the fetch is tracked in flight even before the panel opens"
    );
    assert!(
        app.net.network_toasts().is_none(),
        "a fetch processed before the panel exists cannot leak a standalone toast"
    );

    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();
    assert!(
        app.startup
            .crates_io
            .expected
            .keys()
            .is_some_and(|expected| expected.contains("serde")),
        "startup initialization preserves the pre-panel crates.io obligation"
    );
}

/// Same pre-panel leak guard for GitHub: a repo queued before the Startup
/// panel exists is owned by the startup network stage and seeds the GitHub row
/// when the tracker initializes.
#[test]
fn github_fetch_before_startup_panel_seeds_startup_row() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

    assert!(
        app.net.network_toasts().is_none(),
        "the network-toast stage starts `StartupOwned` before any panel exists"
    );

    app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });

    assert!(
        app.net.github_running().running.contains_key(&repo),
        "the fetch is tracked in flight before the panel opens"
    );
    assert!(
        app.net.network_toasts().is_none(),
        "a pre-panel GitHub fetch cannot create a standalone toast"
    );

    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();
    assert!(
        app.startup
            .repo
            .expected
            .keys()
            .is_some_and(|expected| expected.contains(&repo)),
        "startup initialization preserves the pre-panel GitHub obligation"
    );
}

/// When startup completes, the panel hands the network rows back: the stage
/// flips to `SteadyState`, installing the toast slots. A crates.io fetch
/// queued afterward then creates its standalone toast.
#[test]
fn startup_completion_enters_steady_state_and_emits_crates_io_toast() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");

    // Force every row to an empty (immediately complete) denominator so the
    // panel can close once the minimum-visible floor elapses.
    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.maybe_log_startup_phase_completions();

    assert!(
        app.net.network_toasts().is_none(),
        "the panel still owns the rows until it closes"
    );

    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        !app.startup.is_collecting(),
        "the panel closes once every row is complete past its floor"
    );
    assert!(
        app.net
            .network_toasts()
            .is_some_and(|toasts| toasts.crates_io.is_none()),
        "panel close enters steady state with empty slots — no fetch has run yet"
    );
    let startup_toast = app
        .framework
        .toasts
        .active_now()
        .into_iter()
        .find(|toast| toast.title() == "Startup")
        .expect("Startup countdown toast should still be visible");
    assert_eq!(
        startup_toast.linger_progress(),
        None,
        "Startup countdown must not use task linger fade"
    );
    assert!(
        startup_toast.remaining_secs().is_some(),
        "Startup countdown should still show Closing in N"
    );

    app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
        name: "serde".to_string(),
    });
    assert!(
        app.net
            .network_toasts()
            .is_some_and(|toasts| toasts.crates_io.is_some()),
        "a steady-state crates.io fetch creates the standalone toast"
    );
}

/// In steady state a GitHub repo fetch creates the standalone "Retrieving
/// GitHub repo details" toast — the mirror of the crates.io path.
#[test]
fn steady_state_repo_fetch_emits_github_toast() {
    let project_a = make_project(Some("a"), "~/never-real/a");
    let mut app = make_app(std::slice::from_ref(&project_a));
    finish_startup_for_test(&mut app);

    let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");
    app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo });

    assert!(
        app.net
            .network_toasts()
            .is_some_and(|toasts| toasts.github.is_some()),
        "a steady-state repo fetch creates the standalone GitHub toast"
    );
}

fn begin_steady_state_network_toasts_for_test(app: &mut App) {
    let StartupNetworkReadiness::Ready(ready) = app.net.startup_network_readiness(false, false)
    else {
        panic!("startup network should be ready");
    };
    app.net.begin_steady_state_network_toasts(&ready);
}

fn finish_startup_for_test(app: &mut App) {
    app.scan.state.phase = ScanPhase::Complete;
    app.initialize_startup_phase_tracker();

    let now = std::time::Instant::now();
    let scan_started = app.startup.scan_complete_at.expect("scan complete at");
    app.startup.disk.expected = Denominator::Stable(HashSet::new());
    app.startup.git.expected = Denominator::Stable(HashSet::new());
    app.startup.repo.expected = Denominator::Stable(HashSet::new());
    app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
    app.startup.metadata.expected = Denominator::Stable(HashSet::new());
    app.startup.languages.expected = Denominator::Stable(HashSet::new());
    app.startup.tests.expected = Denominator::Stable(HashSet::new());
    app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
    app.maybe_log_startup_phase_completions();
    app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
    assert!(
        !app.startup.is_collecting(),
        "test setup should close startup"
    );
}

// ── App::clean_selection (Step 6c gating) ──────────────────────────

#[test]
fn clean_selection_on_root_rust_project_returns_project_selection() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    app.project_list.set_cursor(0);

    let selection = app
        .project_list
        .clean_selection()
        .expect("Rust root should be clean-eligible");
    match selection {
        CleanSelection::Project { root } => {
            assert_eq!(root, test_path("~/demo"));
        },
        CleanSelection::WorktreeGroup { .. } => {
            panic!("single Rust root should not yield a worktree-group selection")
        },
    }
}

#[test]
fn clean_selection_on_non_rust_root_is_none() {
    // The gating fix must not regress non-Rust rows: they stay
    // clean-ineligible so the shortcut is dimmed in the status bar.
    let non_rust = make_non_rust_project(Some("notes"), "~/notes");
    let mut app = make_app(std::slice::from_ref(&non_rust));
    app.project_list.set_cursor(0);
    assert!(app.project_list.clean_selection().is_none());
}

#[test]
fn clean_selection_on_worktree_group_root_fans_out_to_primary_and_linked() {
    // A Root row whose RootItem is a WorktreeGroup produces a
    // CleanSelection::WorktreeGroup naming the primary checkout plus
    // every linked worktree. build_clean_plan then dedupes on
    // target_directory — shared-target worktrees collapse into a
    // single CleanTarget with multiple covering_projects.
    let primary_path = test_path("~/cargo-port");
    let linked_path = test_path("~/cargo-port_feat");
    let primary = crate::project::Package {
        path: primary_path.clone(),
        name: Some("cargo-port".to_string()),
        worktree_status: WorktreeStatus::Primary {
            root: primary_path.clone(),
        },
        ..crate::project::Package::default()
    };
    let linked = crate::project::Package {
        path: linked_path.clone(),
        name: Some("cargo-port_feat".to_string()),
        worktree_status: WorktreeStatus::Linked {
            primary: primary_path.clone(),
        },
        ..crate::project::Package::default()
    };
    let worktrees = RootItem::Worktrees(crate::project::WorktreeGroup::new(
        RustProject::Package(primary),
        vec![crate::project::RustProject::Package(linked)],
    ));
    let mut app = make_app(std::slice::from_ref(&worktrees));
    app.project_list.set_cursor(0);

    match app
        .project_list
        .clean_selection()
        .expect("group root is clean-eligible")
    {
        CleanSelection::WorktreeGroup { primary, linked } => {
            assert_eq!(primary, primary_path);
            assert_eq!(linked, vec![linked_path]);
        },
        CleanSelection::Project { .. } => {
            panic!("WorktreeGroup root should fan out, not reduce to a single Project")
        },
    }
}

#[test]
fn request_clean_confirm_opens_ready_when_fingerprint_matches() {
    // When the stored metadata's fingerprint still matches disk, the
    // confirm popup opens immediately — no verifying state, no extra
    // metadata dispatch. Covers the happy path.
    let project = make_project(Some("demo"), "~/never-real/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    let workspace_root = AbsolutePath::from(project.path().as_path().to_path_buf());

    // Seed metadata with a fingerprint the real disk can't match
    // (the project path doesn't exist). capture() will fail on the
    // non-existent path, and `should_verify_before_clean` treats
    // capture failure as "no drift" → Ready.
    app.scan
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .upsert(fake_metadata(&workspace_root));

    app.request_clean_confirm(workspace_root);

    assert!(
        app.scan.confirm_verifying().is_none(),
        "capture failure (test path doesn't exist) → no verifying state"
    );
    assert!(app.confirm().is_some(), "popup opens immediately in Ready");
}

#[test]
fn request_clean_confirm_marks_verifying_when_no_metadata_covers_path() {
    // No metadata → nothing to verify against → flag stays Verifying
    // until metadata arrives. `request_clean_confirm` also spawns
    // a cargo metadata refresh; we don't assert on the spawn here
    // (the async task may race), but the `confirm_verifying` flag
    // must be set synchronously.
    let project = make_project(Some("demo"), "~/never-real/demo");
    let mut app = make_app(std::slice::from_ref(&project));
    let workspace_root = AbsolutePath::from(project.path().as_path().to_path_buf());

    app.request_clean_confirm(workspace_root.clone());

    assert_eq!(
        app.scan.confirm_verifying(),
        Some(&workspace_root),
        "missing metadata → confirm opens in Verifying state, \
         pending on this workspace root"
    );

    // Simulate the arrival: synthetic CargoMetadata Ok arrival must
    // clear the Verifying flag — "Verifying target dir…" transitions
    // to Ready on metadata arrival.
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .next_generation(&workspace_root);
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation,
        fingerprint: fake_fingerprint(),
        result: Ok(fake_metadata(&workspace_root)),
    });
    assert!(
        app.scan.confirm_verifying().is_none(),
        "successful arrival clears the Verifying flag"
    );
}

#[test]
fn out_of_tree_target_size_message_stamps_metadata() {
    // Inject metadata with an out-of-tree target, then route an
    // OutOfTreeTargetSize arrival through handle_bg_msg. The byte total
    // should land on `WorkspaceMetadata::out_of_tree_target_bytes`.
    let workspace_root = AbsolutePath::from(PathBuf::from("/ws"));
    let target_dir = AbsolutePath::from(PathBuf::from("/elsewhere/target"));
    let pkg = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: workspace_root.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg]);
    {
        let store = app.scan.metadata_store_handle();
        let mut guard = store.lock().unwrap_or_else(|_| std::process::abort());
        guard.upsert(WorkspaceMetadata {
            workspace_root:           workspace_root.clone(),
            target_directory:         target_dir.clone(),
            packages:                 HashMap::new(),
            fingerprint:              fake_fingerprint(),
            out_of_tree_target_bytes: None,
        });
    }

    app.handle_bg_msg(BackgroundMsg::OutOfTreeTargetSize {
        workspace_root: workspace_root.clone(),
        target_dir,
        bytes: 1_234_567,
    });

    let stamped = app
        .scan
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .get(&workspace_root)
        .and_then(|s| s.out_of_tree_target_bytes);
    assert_eq!(stamped, Some(1_234_567));
}

#[test]
fn cargo_metadata_arrival_stamps_cargo_fields_onto_package() {
    let project_path = AbsolutePath::from(PathBuf::from("/abs/demo"));
    let pkg_item = RootItem::Rust(RustProject::Package(crate::project::Package {
        path: project_path.clone(),
        name: Some("demo".into()),
        ..crate::project::Package::default()
    }));
    let mut app = make_app(&[pkg_item]);

    // Before metadata arrival: Cargo::default() → publishable true but
    // empty types / examples / benches.
    let pre_types = app
        .project_list
        .rust_info_at_path(project_path.as_path())
        .map_or(0, |r| r.cargo.types().len());
    assert_eq!(pre_types, 0, "pre-metadata types stay empty");

    let manifest_path = AbsolutePath::from(project_path.as_path().join("Cargo.toml"));
    let example_src = AbsolutePath::from(project_path.as_path().join("examples").join("hello.rs"));
    let bin_src = AbsolutePath::from(project_path.as_path().join("src").join("main.rs"));
    let record_id = PackageId {
        repr: "demo-id".into(),
    };
    let record = PackageRecord {
        name: "demo".into(),
        version: Version::new(0, 1, 0),
        edition: "2024".into(),
        description: None,
        license: None,
        homepage: None,
        repository: None,
        manifest_path,
        targets: vec![
            crate::project::TargetRecord {
                name:              "demo".into(),
                kinds:             vec![TargetKind::Bin],
                required_features: vec![],
                src_path:          bin_src,
            },
            crate::project::TargetRecord {
                name:              "hello".into(),
                kinds:             vec![TargetKind::Example],
                required_features: vec![],
                src_path:          example_src,
            },
        ],
        publish: PublishPolicy::Never,
    };
    let mut packages = HashMap::new();
    packages.insert(record_id, record);

    let workspace_metadata = WorkspaceMetadata {
        workspace_root: project_path.clone(),
        target_directory: AbsolutePath::from(project_path.as_path().join("target")),
        packages,
        fingerprint: fake_fingerprint(),
        out_of_tree_target_bytes: None,
    };
    let generation = app
        .scan
        .metadata_store_handle()
        .lock()
        .unwrap_or_else(|_| std::process::abort())
        .next_generation(&project_path);
    app.handle_bg_msg(BackgroundMsg::CargoMetadata {
        workspace_root: project_path.clone(),
        generation,
        fingerprint: workspace_metadata.fingerprint.clone(),
        result: Ok(workspace_metadata),
    });

    let cargo = app
        .project_list
        .rust_info_at_path(project_path.as_path())
        .map_or_else(|| std::process::abort(), |r| r.cargo.clone());
    assert!(
        cargo.types().contains(&crate::project::ProjectType::Binary),
        "Bin TargetKind → ProjectType::Binary stamped from metadata"
    );
    assert_eq!(
        cargo.example_count(),
        1,
        "Example TargetKind populates Cargo.examples"
    );
    assert!(
        !cargo.publishable(),
        "PublishPolicy::Never → Cargo.publishable false after metadata"
    );
}

#[test]
fn apply_lint_config_change_fans_out_to_inflight_scan_and_selection() {
    let project = make_project(Some("demo"), "~/demo");
    let project_path = project.path().clone();
    let mut app = make_app(&[project]);

    // Projection: seed a real project-model running lint so we can prove the
    // orchestrator clears project lint state and reconciles the toast from it.
    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_path,
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });
    assert!(!app.lint.running_toast_is_empty());

    // App-shell scan state: capture the pre-call generation.
    let gen_before = app.scan.generation();

    // Selection: replace fit_widths with a sentinel generation so we
    // can prove reset_fit_widths fired (reset re-seeds with
    // `generation: u64::MAX`, which is the construct-time default).
    {
        let widths = app.project_list.fit_widths_mut();
        widths.generation = 0;
    }
    assert_eq!(app.project_list.cached_fit_widths.generation, 0);

    let cfg = app.config.current().clone();
    app.apply_lint_config_change(&cfg);

    // Projection: running-lint paths cleared, lint runtime present
    // (re-spawned).
    assert!(
        app.lint.running_toast_is_empty(),
        "apply_lint_config_change must clear running lint projection"
    );
    // Scan: data_generation bumped exactly once.
    assert_eq!(
        app.scan.generation(),
        gen_before + 1,
        "apply_lint_config_change must bump data_generation"
    );
    // Selection: fit_widths reset (back to construct-time sentinel).
    assert_eq!(
        app.project_list.cached_fit_widths.generation,
        u64::MAX,
        "apply_lint_config_change must reset fit_widths"
    );
}
