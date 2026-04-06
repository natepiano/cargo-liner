use std::collections::HashSet;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use chrono::DateTime;
use crossterm::event::KeyCode;

use super::snapshots;
use super::types::*;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::ci::FetchStatus;
use crate::config::CargoPortConfig;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::lint::LintStatus;
use crate::project::ExampleGroup;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::ProjectLanguage;
use crate::project::RustProject;
use crate::project::WorkspaceStatus;
use crate::scan::BackgroundMsg;
use crate::scan::MemberGroup;
use crate::scan::ProjectNode;
use crate::tui::shortcuts::InputContext;
use crate::tui::toasts::ToastManager;
use crate::tui::types::PaneId;

fn test_http_client() -> HttpClient {
    static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = TEST_RT
        .get_or_init(|| tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort()));
    HttpClient::new(rt.handle().clone()).unwrap_or_else(|| std::process::abort())
}

fn make_project(name: Option<&str>, path: &str) -> RustProject {
    RustProject {
        path:                      path.to_string(),
        abs_path:                  path.to_string(),
        name:                      name.map(String::from),
        version:                   None,
        description:               None,
        worktree_name:             None,
        worktree_primary_abs_path: None,
        is_workspace:              WorkspaceStatus::Standalone,
        types:                     Vec::new(),
        examples:                  Vec::new(),
        benches:                   Vec::new(),
        test_count:                0,
        is_rust:                   ProjectLanguage::Rust,
        local_dependency_paths:    Vec::new(),
    }
}

fn make_node(project: RustProject) -> ProjectNode {
    ProjectNode {
        project,
        groups: Vec::new(),
        worktrees: Vec::new(),
        vendored: Vec::new(),
    }
}

fn make_app(projects: Vec<RustProject>) -> App {
    make_app_with_config(projects, &CargoPortConfig::default())
}

fn make_app_with_config(projects: Vec<RustProject>, cfg: &CargoPortConfig) -> App {
    let (bg_tx, bg_rx) = mpsc::channel();
    let scan_root =
        std::env::temp_dir().join(format!("cargo-port-polish-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&scan_root);
    let mut app = App::new(
        scan_root,
        projects,
        bg_tx,
        bg_rx,
        cfg,
        test_http_client(),
        Instant::now(),
    );
    app.retry_spawn_mode = RetrySpawnMode::Disabled;
    app.sync_selected_project();
    app
}

fn make_non_rust_project(name: Option<&str>, path: &str) -> RustProject {
    let mut project = make_project(name, path);
    project.is_rust = ProjectLanguage::NonRust;
    project
}

fn wait_for_tree_build(app: &mut App) {
    for _ in 0..100 {
        let _ = app.poll_tree_builds();
        if app.builds.tree.active.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    app.ensure_visible_rows_cached();
}

#[test]
fn external_config_reload_applies_valid_changes() {
    let mut app = make_app(Vec::new());
    let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let path = dir.path().join("config.toml");

    let mut cfg = CargoPortConfig::default();
    cfg.tui.editor = "helix".to_string();
    cfg.tui.ci_run_count = 9;
    cfg.mouse.invert_scroll = ScrollDirection::Normal;
    std::fs::write(
        &path,
        toml::to_string_pretty(&cfg).unwrap_or_else(|_| std::process::abort()),
    )
    .unwrap_or_else(|_| std::process::abort());

    app.config_path = Some(path);
    app.config_last_seen = None;
    app.maybe_reload_config_from_disk();

    assert_eq!(app.editor(), "helix");
    assert_eq!(app.ci_run_count(), 9);
    assert_eq!(app.invert_scroll(), ScrollDirection::Normal);
    assert_eq!(app.current_config.tui.editor, "helix");
    assert_eq!(app.current_config.tui.ci_run_count, 9);
}

#[test]
fn external_config_reload_keeps_last_good_config_on_parse_error() {
    let mut app = make_app(Vec::new());
    let dir = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let path = dir.path().join("config.toml");

    let mut cfg = CargoPortConfig::default();
    cfg.tui.editor = "zed".to_string();
    std::fs::write(
        &path,
        toml::to_string_pretty(&cfg).unwrap_or_else(|_| std::process::abort()),
    )
    .unwrap_or_else(|_| std::process::abort());

    app.config_path = Some(path.clone());
    app.config_last_seen = None;
    app.maybe_reload_config_from_disk();

    std::fs::write(&path, "[tui\neditor = \"vim\"\n").unwrap_or_else(|_| std::process::abort());
    app.config_last_seen = None;
    app.maybe_reload_config_from_disk();

    assert_eq!(app.editor(), "zed");
    assert_eq!(app.current_config.tui.editor, "zed");
    assert!(
        app.status_flash
            .as_ref()
            .is_some_and(|(msg, _)| msg.contains("Config reload failed"))
    );
}

#[test]
fn completed_scan_hides_and_restores_cached_non_rust_projects_without_rescan() {
    let rust_project = make_project(Some("rust"), "~/rust");
    let non_rust_project = make_non_rust_project(Some("js"), "~/js");
    let mut cfg = CargoPortConfig::default();
    cfg.tui.include_non_rust = NonRustInclusion::Include;
    let mut app = make_app_with_config(vec![rust_project.clone(), non_rust_project.clone()], &cfg);
    app.scan.phase = ScanPhase::Complete;

    assert_eq!(app.all_projects.len(), 2);
    assert_eq!(app.nodes.len(), 2);

    let mut hide_cfg = cfg.clone();
    hide_cfg.tui.include_non_rust = NonRustInclusion::Exclude;
    app.apply_config(&hide_cfg);
    wait_for_tree_build(&mut app);

    assert_eq!(app.all_projects.len(), 2);
    assert!(app.is_scan_complete());
    assert_eq!(app.nodes.len(), 1);
    assert_eq!(app.nodes[0].project.path, rust_project.path);

    app.apply_config(&cfg);
    wait_for_tree_build(&mut app);

    assert_eq!(app.all_projects.len(), 2);
    assert!(app.is_scan_complete());
    assert_eq!(app.nodes.len(), 2);
    assert!(
        app.nodes
            .iter()
            .any(|node| node.project.path == non_rust_project.path)
    );
}

#[test]
fn completed_scan_rescans_when_enabling_non_rust_without_cached_projects() {
    let rust_project = make_project(Some("rust"), "~/rust");
    let mut app = make_app(vec![rust_project]);
    app.scan.phase = ScanPhase::Complete;

    let mut cfg = app.current_config.clone();
    cfg.tui.include_non_rust = NonRustInclusion::Include;
    app.apply_config(&cfg);

    assert!(app.all_projects.is_empty());
    assert!(!app.is_scan_complete());
}

fn apply_nodes(app: &mut App, nodes: Vec<ProjectNode>) {
    let flat_entries = crate::scan::build_flat_entries(&nodes);
    app.apply_tree_build(nodes, flat_entries);
    app.ensure_visible_rows_cached();
}

fn parse_ts(ts: &str) -> DateTime<chrono::FixedOffset> {
    DateTime::parse_from_rfc3339(ts).unwrap_or_else(|_| std::process::abort())
}

fn make_ci_run(run_id: u64, conclusion: Conclusion) -> CiRun {
    CiRun {
        run_id,
        created_at: "2026-03-30T14:22:18Z".to_string(),
        branch: "main".to_string(),
        url: format!("https://github.com/natepiano/demo/actions/runs/{run_id}"),
        conclusion,
        jobs: Vec::new(),
        wall_clock_secs: Some(1),
        commit_title: Some(format!("run {run_id}")),
        fetched: FetchStatus::Fetched,
    }
}

fn make_git_info(url: Option<&str>) -> GitInfo {
    GitInfo {
        origin:              GitOrigin::Clone,
        branch:              Some("main".to_string()),
        owner:               Some("natepiano".to_string()),
        url:                 url.map(String::from),
        first_commit:        None,
        last_commit:         None,
        ahead_behind:        None,
        default_branch:      Some("main".to_string()),
        ahead_behind_origin: None,
        ahead_behind_local:  None,
    }
}

#[test]
fn service_reachability_tracks_background_messages() {
    let mut app = make_app(Vec::new());

    assert!(app.unreachable_services.is_empty());
    assert!(app.unreachable_service_message().is_none());

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::GitHub,
    }));
    assert!(app.unreachable_services.contains(&ServiceKind::GitHub));
    assert_eq!(
        app.unreachable_service_message().as_deref(),
        Some(" GitHub unreachable ")
    );

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
        service: ServiceKind::CratesIo,
    }));
    assert!(app.unreachable_services.contains(&ServiceKind::CratesIo));
    assert_eq!(
        app.unreachable_service_message().as_deref(),
        Some(" GitHub and crates.io unreachable ")
    );

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::GitHub,
    }));
    assert!(!app.unreachable_services.contains(&ServiceKind::GitHub));
    assert_eq!(
        app.unreachable_service_message().as_deref(),
        Some(" crates.io unreachable ")
    );

    assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
        service: ServiceKind::CratesIo,
    }));
    assert!(app.unreachable_services.is_empty());
    assert!(app.unreachable_service_message().is_none());
}

#[test]
fn visible_rows_workspace_with_worktrees() {
    // A workspace whose groups have been moved to worktree entries
    let mut root = make_node(make_project(None, "~/ws"));
    let member_a = make_project(Some("a"), "~/ws/a");
    let member_b = make_project(Some("b"), "~/ws/b");

    // Primary-as-worktree with inline members
    let mut wt0 = make_node(make_project(None, "~/ws"));
    wt0.project.worktree_name = Some("ws".to_string());
    wt0.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member_a.clone(), member_b.clone()],
    }];

    // Actual worktree with a named group
    let mut wt1 = make_node(make_project(None, "~/ws_feat"));
    wt1.project.worktree_name = Some("ws_feat".to_string());
    wt1.groups = vec![MemberGroup {
        name:    "crates".to_string(),
        members: vec![member_a, member_b],
    }];

    root.worktrees = vec![wt0, wt1];

    // Expand everything: node, both worktrees, and the named group
    let expanded: HashSet<ExpandKey> = [
        ExpandKey::Node(0),
        ExpandKey::Worktree(0, 0),
        ExpandKey::Worktree(0, 1),
        ExpandKey::WorktreeGroup(0, 1, 0),
    ]
    .into();

    let rows = snapshots::build_visible_rows(&[root], &expanded, &HashSet::new());

    // Expected:
    // 0: Root(0)
    // 1: WorktreeEntry(0, 0)
    // 2: WorktreeMember(0, 0, 0, 0)  — inline member a
    // 3: WorktreeMember(0, 0, 0, 1)  — inline member b
    // 4: WorktreeEntry(0, 1)
    // 5: WorktreeGroupHeader(0, 1, 0) — "crates"
    // 6: WorktreeMember(0, 1, 0, 0)
    // 7: WorktreeMember(0, 1, 0, 1)
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
fn visible_rows_non_workspace_worktrees() {
    let build_root = || {
        let mut root = make_node(make_project(Some("app"), "~/app"));
        let mut wt0 = make_node(make_project(Some("app"), "~/app"));
        wt0.project.worktree_name = Some("app".to_string());
        let mut wt1 = make_node(make_project(Some("app"), "~/app_feat"));
        wt1.project.worktree_name = Some("app_feat".to_string());
        root.worktrees = vec![wt0, wt1];
        root
    };

    // Standalone project with worktrees — flat, not expandable
    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&[build_root()], &expanded, &HashSet::new());

    // Root + 2 flat worktree entries
    assert_eq!(rows.len(), 3, "got: {rows:?}");
    assert!(matches!(rows[0], VisibleRow::Root { .. }));
    assert!(matches!(rows[1], VisibleRow::WorktreeEntry { .. }));
    assert!(matches!(rows[2], VisibleRow::WorktreeEntry { .. }));

    // Expanding a worktree with no groups produces no additional rows
    let expanded2: HashSet<ExpandKey> = [ExpandKey::Node(0), ExpandKey::Worktree(0, 0)].into();
    let rows2 = snapshots::build_visible_rows(&[build_root()], &expanded2, &HashSet::new());
    assert_eq!(rows2.len(), 3, "no extra rows for non-workspace worktree");
}

#[test]
fn visible_rows_workspace_no_worktrees() {
    // Workspace with groups, no worktrees — regression test
    let member_a = make_project(Some("a"), "~/ws/a");
    let member_b = make_project(Some("b"), "~/ws/b");
    let mut root = make_node(make_project(None, "~/ws"));
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member_a, member_b],
    }];

    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&[root], &expanded, &HashSet::new());

    // Root + 2 inline members
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
    let member = make_project(Some("member"), "~/ws/member");
    let vendored = make_project(Some("vendored"), "~/ws/vendor/helper");
    let mut root = make_node(make_project(None, "~/ws"));
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member],
    }];
    root.vendored = vec![vendored];

    let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
    let rows = snapshots::build_visible_rows(&[root], &expanded, &HashSet::new());

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

#[test]
fn lint_runtime_waits_for_scan_completion() {
    let project = make_project(Some("demo"), "~/demo");
    let path = project.path.clone();
    let mut app = make_app(vec![project]);

    assert!(app.lint_runtime_projects_snapshot().is_empty());

    app.scan.phase = ScanPhase::Complete;
    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_path, path);
}

#[test]
fn ci_runs_stay_on_owner_rows_not_workspace_members() {
    let workspace = make_project(Some("ws"), "~/ws");
    let member = make_project(Some("core"), "~/ws/core");

    let mut root = make_node(workspace.clone());
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member.clone()],
    }];

    let mut app = make_app(vec![workspace.clone(), member.clone()]);
    apply_nodes(&mut app, vec![root]);

    app.insert_ci_runs(
        workspace.path.clone(),
        vec![make_ci_run(1, Conclusion::Success)],
    );

    assert_eq!(app.ci_for(&workspace), Some(Conclusion::Success));
    assert!(app.ci_state.contains_key(&workspace.path));
    assert_eq!(app.ci_for(&member), None);
    assert!(app.ci_state_for(&member).is_none());
    assert!(!app.ci_state.contains_key(&member.path));
}

#[test]
fn non_owner_member_ignores_stale_ci_state_and_cannot_fetch() {
    let workspace = make_project(Some("ws"), "~/ws");
    let member = make_project(Some("core"), "~/ws/core");

    let mut root = make_node(workspace.clone());
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member.clone()],
    }];

    let mut app = make_app(vec![workspace, member.clone()]);
    apply_nodes(&mut app, vec![root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();
    app.select_project_in_tree(&member.path);

    app.ci_state.insert(
        member.path.clone(),
        CiState::Loaded {
            runs:      vec![make_ci_run(2, Conclusion::Failure)],
            exhausted: false,
        },
    );
    app.git_info.insert(
        member.path.clone(),
        make_git_info(Some("https://github.com/natepiano/demo")),
    );

    assert!(app.ci_state_for(&member).is_none());
    assert_eq!(app.ci_for(&member), None);
    assert!(!app.bottom_panel_available(&member));

    crate::tui::detail::handle_ci_runs_key(
        &mut app,
        &crossterm::event::KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
    );
    assert!(app.pending_ci_fetch.is_none());
}

#[test]
fn ci_rollup_uses_only_root_and_immediate_worktrees() {
    let mut root = make_node(make_project(Some("ws"), "~/ws"));
    let member = make_project(Some("core"), "~/ws/core");
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member.clone()],
    }];

    let mut feature = make_node(make_project(Some("ws_feat"), "~/ws_feat"));
    feature.project.worktree_name = Some("ws_feat".to_string());
    feature.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![make_project(Some("feat_core"), "~/ws_feat/core")],
    }];
    let feature_path = feature.project.path.clone();
    root.worktrees = vec![feature];

    let mut app = make_app(vec![root.project.clone(), member.clone()]);
    apply_nodes(&mut app, vec![root.clone()]);

    app.ci_state.insert(
        root.project.path.clone(),
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
        member.path.clone(),
        CiState::Loaded {
            runs:      vec![make_ci_run(5, Conclusion::Success)],
            exhausted: false,
        },
    );

    assert_eq!(app.ci_for_node(&root), Some(Conclusion::Failure));
    assert!(app.ci_state_for(&member).is_none());
}

#[test]
fn ci_for_prefers_runs_matching_local_branch() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(vec![project.clone()]);
    app.git_info.insert(
        project.path.clone(),
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
        },
    );
    app.ci_state.insert(
        project.path.clone(),
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

    assert_eq!(app.ci_for(&project), Some(Conclusion::Failure));
}

#[test]
fn startup_lint_expectation_tracks_running_startup_lints() {
    let project_a = make_project(Some("a"), "~/a");
    let project_b = make_project(Some("b"), "~/b");
    let mut app = make_app(vec![project_a.clone(), project_b]);
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
        path:   project_a.path.clone(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });

    let expected = app
        .scan
        .startup_phases
        .lint_expected
        .as_ref()
        .expect("lint expected");
    assert_eq!(expected.len(), 1);
    assert!(expected.contains(&project_a.path));
    assert!(
        !app.scan
            .startup_phases
            .lint_seen_terminal
            .contains(&project_a.path)
    );
    assert!(app.running_lint_paths.contains(&project_a.path));
    assert!(app.lint_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_a.path,
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });

    assert!(app.scan.startup_phases.lint_complete_at.is_some());
    assert!(app.running_lint_paths.is_empty());
    assert!(app.lint_toast.is_none());
}

#[test]
fn startup_lint_toast_body_shows_paths_then_others() {
    let expected = HashSet::from([
        "~/a".to_string(),
        "~/b".to_string(),
        "~/c".to_string(),
        "~/d".to_string(),
        "~/e".to_string(),
    ]);
    let seen = HashSet::from(["~/e".to_string()]);

    let body = App::startup_lint_toast_body_for(&expected, &seen);
    let lines = body.lines().collect::<Vec<_>>();

    // 4 remaining, 3 visible + suffix on last.
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("~/"));
    assert!(lines[1].starts_with("~/"));
    assert!(lines[2].contains("(+ 1 others)"));
}

#[test]
fn lint_toast_reappears_for_new_running_lints() {
    let project = make_project(Some("a"), "~/a");
    let mut app = make_app(vec![project.clone()]);
    app.scan.phase = ScanPhase::Complete;

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path.clone(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
    });
    let first_toast = app.lint_toast;
    assert!(first_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path.clone(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });
    assert!(app.lint_toast.is_none());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path,
        status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
    });
    assert!(app.lint_toast.is_some());
    assert_ne!(app.lint_toast, first_toast);
}

#[test]
fn collapse_all_anchors_member_selection_to_root() {
    let workspace = make_project(Some("hana"), "~/rust/hana");
    let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");

    let mut root = make_node(workspace.clone());
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member.clone()],
    }];

    let mut app = make_app(vec![workspace, member.clone()]);
    apply_nodes(&mut app, vec![root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.select_project_in_tree(&member.path);

    app.collapse_all();

    assert_eq!(app.selected_row(), Some(VisibleRow::Root { node_index: 0 }));
}

#[test]
fn expand_all_preserves_selected_project_path() {
    let workspace = make_project(Some("hana"), "~/rust/hana");
    let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");

    let mut root = make_node(workspace.clone());
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member.clone()],
    }];

    let mut app = make_app(vec![workspace, member.clone()]);
    apply_nodes(&mut app, vec![root]);
    app.select_project_in_tree(&member.path);
    app.collapse_all();

    app.expand_all();

    assert_eq!(
        app.selected_project().map(|project| project.path.as_str()),
        Some(member.path.as_str())
    );
}

#[test]
fn lint_runtime_snapshot_uses_workspace_root_not_members() {
    let mut workspace = make_project(Some("hana"), "~/rust/hana");
    workspace.is_workspace = WorkspaceStatus::Workspace;
    let member_a = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
    let member_b = make_project(Some("hana_ui"), "~/rust/hana/crates/hana_ui");

    let mut root = make_node(workspace.clone());
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member_a.clone(), member_b.clone()],
    }];

    let mut app = make_app(vec![workspace.clone(), member_a, member_b]);
    apply_nodes(&mut app, vec![root]);
    app.scan.phase = ScanPhase::Complete;

    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_path, workspace.path);
}

#[test]
fn lint_runtime_snapshot_deduplicates_primary_worktree_path() {
    let root_project = make_project(Some("ws"), "~/ws");
    let mut root = make_node(root_project.clone());

    let mut primary = make_node(root_project.clone());
    primary.project.worktree_name = Some("ws".to_string());

    let mut feature = make_node(make_project(Some("ws_feat"), "~/ws_feat"));
    feature.project.worktree_name = Some("ws_feat".to_string());

    root.worktrees = vec![primary, feature.clone()];

    let mut app = make_app(vec![root_project.clone(), feature.project.clone()]);
    apply_nodes(&mut app, vec![root]);
    app.scan.phase = ScanPhase::Complete;

    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 2);
    assert_eq!(projects[0].project_path, root_project.path);
    assert_eq!(projects[1].project_path, feature.project.path);
}

#[test]
fn vendored_path_dependency_becomes_cargo_active() {
    let mut root_project = make_project(Some("app"), "~/app");
    let vendored = make_project(Some("helper"), "~/app/vendor/helper");
    root_project.local_dependency_paths = vec![vendored.path.clone()];

    let mut root = make_node(root_project.clone());
    root.vendored = vec![vendored.clone()];

    let mut app = make_app(vec![root_project, vendored.clone()]);
    apply_nodes(&mut app, vec![root]);

    assert!(app.is_vendored_path(&vendored.path));
    assert!(app.is_cargo_active_path(&vendored.path));
}

#[test]
fn git_path_state_suppresses_sync_for_untracked_and_ignored() {
    let project = make_project(Some("demo"), "~/demo");
    let path = project.path.clone();
    let mut app = make_app(vec![project.clone()]);

    app.git_info.insert(
        path.clone(),
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
            ahead_behind_local:  None,
        },
    );

    app.git_path_states
        .insert(path.clone(), GitPathState::Untracked);
    assert!(app.git_sync(&project).is_empty());

    app.git_path_states.insert(path, GitPathState::Ignored);
    assert!(app.git_sync(&project).is_empty());
}

#[test]
fn name_width_with_gutter_reserves_space_before_lint() {
    assert_eq!(App::name_width_with_gutter(0), 1);
    assert_eq!(App::name_width_with_gutter(42), 43);
}

#[test]
fn tabbable_panes_follow_canonical_order() {
    let mut project = make_project(Some("demo"), "~/demo");
    project.examples = vec![ExampleGroup {
        category: String::new(),
        names:    vec!["example".to_string()],
    }];

    let mut app = make_app(vec![project.clone()]);
    app.toasts = ToastManager::default();
    app.toast_pane.set_len(0);
    app.scan.phase = ScanPhase::Complete;
    app.git_info.insert(
        project.path,
        GitInfo {
            origin:              GitOrigin::Clone,
            branch:              None,
            owner:               None,
            url:                 Some("https://github.com/acme/demo".to_string()),
            first_commit:        None,
            last_commit:         None,
            ahead_behind:        None,
            default_branch:      None,
            ahead_behind_origin: None,
            ahead_behind_local:  None,
        },
    );

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
    let mut app = make_app(vec![project]);
    app.focus_pane(PaneId::Git);

    app.show_timed_toast("Settings", "Updated");
    assert_eq!(app.focused_pane, PaneId::Git);

    let _task = app.start_task_toast("Startup lints", "Running startup lint jobs...");
    assert_eq!(app.focused_pane, PaneId::Git);
}

#[test]
fn project_refresh_updates_selected_tree_project_targets() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(vec![project.clone()]);
    app.scan.phase = ScanPhase::Complete;
    app.list_state.select(Some(0));
    app.sync_selected_project();

    assert_eq!(
        app.selected_project().map(RustProject::example_count),
        Some(0)
    );
    assert!(!app.tabbable_panes().contains(&PaneId::Targets));

    let mut refreshed = project;
    refreshed.examples = vec![ExampleGroup {
        category: String::new(),
        names:    vec!["tracked_row_paths".to_string()],
    }];

    assert!(app.handle_project_refreshed(&refreshed));
    app.sync_selected_project();

    assert_eq!(
        app.selected_project().map(RustProject::example_count),
        Some(1)
    );
    assert!(app.tabbable_panes().contains(&PaneId::Targets));
}

#[test]
fn first_non_empty_tree_build_focuses_project_list() {
    let project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(vec![project.clone()]);
    apply_nodes(&mut app, vec![make_node(project)]);

    assert_eq!(app.focused_pane, PaneId::ProjectList);
    assert_eq!(app.list_state.selected(), Some(0));
}

#[test]
fn initial_disk_batch_count_groups_nested_projects_under_one_root() {
    let projects = vec![
        make_project(Some("bevy"), "~/rust/bevy"),
        make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
        make_project(Some("render"), "~/rust/bevy/crates/bevy_render"),
        make_project(Some("hana"), "~/rust/hana"),
        make_project(Some("hana_core"), "~/rust/hana/crates/hana"),
    ];

    assert_eq!(snapshots::initial_disk_batch_count(&projects), 2);
}

#[test]
fn overlays_restore_prior_focus() {
    let app_project = make_project(Some("demo"), "~/demo");
    let mut app = make_app(vec![app_project]);
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
    let mut app = make_app(vec![project]);

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
    let mut app = make_app(vec![project_a, project_b]);

    app.focus_pane(PaneId::Package);
    app.focus_pane(PaneId::Git);
    app.focus_pane(PaneId::Targets);
    app.focus_pane(PaneId::CiRuns);
    app.package_pane.set_pos(3);
    app.git_pane.set_pos(4);
    app.targets_pane.set_pos(5);
    app.ci_pane.set_pos(6);

    app.list_state.select(Some(1));
    app.sync_selected_project();

    assert_eq!(app.package_pane.pos(), 0);
    assert_eq!(app.git_pane.pos(), 0);
    assert_eq!(app.targets_pane.pos(), 0);
    assert_eq!(app.ci_pane.pos(), 0);
    assert!(!app.remembers_selection(PaneId::Package));
    assert!(!app.remembers_selection(PaneId::Git));
    assert!(!app.remembers_selection(PaneId::Targets));
    assert!(!app.remembers_selection(PaneId::CiRuns));
    assert_eq!(app.selection_paths.selected_project.as_deref(), Some("~/b"));
}

#[test]
fn apply_config_resets_column_layout_flag() {
    let mut app = make_app(vec![make_project(Some("demo"), "~/demo")]);
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

    let mut workspace = make_project(Some("hana"), "~/rust/hana");
    workspace.abs_path = workspace_dir.to_string_lossy().to_string();
    workspace.is_workspace = WorkspaceStatus::Workspace;

    let mut member = make_project(Some("clay-layout"), "~/rust/hana/crates/clay-layout");
    member.abs_path = member_dir.to_string_lossy().to_string();

    let mut root = make_node(workspace.clone());
    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member.clone()],
    }];

    let mut app = make_app(vec![workspace, member.clone()]);
    apply_nodes(&mut app, vec![root]);

    std::fs::remove_dir_all(&member_dir).unwrap_or_else(|_| std::process::abort());
    app.handle_disk_usage(member.path.clone(), 0);

    assert!(app.deleted_projects.contains(&member.path));
}

#[test]
fn disk_updates_skip_git_path_refresh_during_scan() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let abs_path = tmp.path().join("demo");
    std::fs::create_dir_all(&abs_path).unwrap_or_else(|_| std::process::abort());

    let mut project = make_project(Some("demo"), "~/demo");
    project.abs_path = abs_path.to_string_lossy().to_string();
    let path = project.path.clone();
    let mut app = make_app(vec![project]);

    app.handle_disk_usage(path.clone(), 123);
    assert!(!app.git_path_states.contains_key(&path));

    app.scan.phase = ScanPhase::Complete;
    app.handle_disk_usage(path.clone(), 123);
    assert_eq!(
        app.git_path_states.get(&path),
        Some(&GitPathState::OutsideRepo)
    );
}

#[test]
fn bottom_panel_changes_input_context_for_lower_pane() {
    let mut app = make_app(vec![make_project(Some("demo"), "~/demo")]);
    app.focus_pane(PaneId::CiRuns);
    assert_eq!(app.input_context(), InputContext::CiRuns);

    app.toggle_bottom_panel();
    assert_eq!(app.input_context(), InputContext::Lints);
}

#[test]
fn lint_rollups_distinguish_root_from_primary_worktree() {
    let mut root = make_node(make_project(None, "~/ws"));
    let mut primary = make_node(make_project(None, "~/ws"));
    primary.project.worktree_name = Some("ws".to_string());

    let mut feature = make_node(make_project(None, "~/ws_feat"));
    feature.project.worktree_name = Some("ws_feat".to_string());

    root.worktrees = vec![primary, feature];

    let mut app = make_app(vec![root.project.clone()]);
    app.current_config.lint.enabled = true;
    apply_nodes(&mut app, vec![root]);
    app.lint_status.insert(
        "~/ws".to_string(),
        LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
    );
    app.lint_status.insert(
        "~/ws_feat".to_string(),
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
    let mut root = make_node(make_project(None, "~/ws"));
    let member = make_project(Some("a"), "~/ws/a");

    root.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member],
    }];

    let mut app = make_app(vec![root.project.clone()]);
    app.current_config.lint.enabled = true;
    apply_nodes(&mut app, vec![root]);
    app.lint_status.insert(
        "~/ws".to_string(),
        LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
    );
    app.lint_status.insert(
        "~/ws/a".to_string(),
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
    let mut root = make_node(make_project(None, "~/ws"));
    let mut primary = make_node(make_project(None, "~/ws"));
    primary.project.worktree_name = Some("ws".to_string());

    let mut feature = make_node(make_project(None, "~/ws_feat"));
    feature.project.worktree_name = Some("ws_feat".to_string());

    root.worktrees = vec![primary, feature];

    let mut app = make_app(vec![root.project.clone()]);
    app.current_config.lint.enabled = true;
    apply_nodes(&mut app, vec![root]);
    app.lint_status.insert(
        "~/ws".to_string(),
        LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
    );
    app.lint_status.insert(
        "~/ws_feat".to_string(),
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

#[test]
fn detail_cache_separates_root_and_worktree_rows_with_same_path() {
    let mut root = make_node(make_project(None, "~/ws"));
    let member_a = make_project(Some("a"), "~/ws/a");
    let member_b = make_project(Some("b"), "~/ws_feat/b");

    let mut primary = make_node(make_project(None, "~/ws"));
    primary.project.worktree_name = Some("ws".to_string());
    primary.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member_a],
    }];

    let mut feature = make_node(make_project(None, "~/ws_feat"));
    feature.project.worktree_name = Some("ws_feat".to_string());
    feature.groups = vec![MemberGroup {
        name:    String::new(),
        members: vec![member_b],
    }];

    root.worktrees = vec![primary, feature];

    let mut app = make_app(vec![root.project.clone()]);
    app.current_config.lint.enabled = true;
    apply_nodes(&mut app, vec![root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();

    app.lint_status.insert(
        "~/ws".to_string(),
        LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
    );
    app.lint_status.insert(
        "~/ws_feat".to_string(),
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
fn disk_rollup_deduplicates_primary_worktree_path() {
    let mut root = make_node(make_project(None, "~/ws"));
    let mut primary = make_node(make_project(None, "~/ws"));
    primary.project.worktree_name = Some("ws".to_string());
    let mut feature = make_node(make_project(None, "~/ws_feat"));
    feature.project.worktree_name = Some("ws_feat".to_string());
    root.worktrees = vec![primary, feature];

    let mut app = make_app(vec![root.project.clone()]);
    apply_nodes(&mut app, vec![root.clone()]);
    app.disk_usage.insert("~/ws".to_string(), 15);
    app.disk_usage.insert("~/ws_feat".to_string(), 21);

    assert_eq!(app.disk_bytes_for_node(&root), Some(36));
    assert_eq!(
        snapshots::disk_bytes_for_node_snapshot(&root, &app.disk_usage),
        Some(36)
    );
    assert_eq!(
        app.formatted_disk_for_node(&root),
        crate::tui::render::format_bytes(36)
    );
}
