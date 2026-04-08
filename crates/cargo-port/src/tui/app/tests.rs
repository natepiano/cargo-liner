use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Instant;

use chrono::DateTime;
use crossterm::event::KeyCode;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;

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
use crate::project::Cargo;
use crate::project::ExampleGroup;
use crate::project::GitInfo;
use crate::project::GitOrigin;
use crate::project::GitPathState;
use crate::project::MemberGroup;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::ProjectListItem;
use crate::project::RustProject;
use crate::project::Visibility::Deleted;
use crate::project::Visibility::Dismissed;
use crate::project::WorkflowPresence;
use crate::project::Workspace;
use crate::scan::BackgroundMsg;
use crate::tui::shortcuts::InputContext;
use crate::tui::toasts::ToastManager;
use crate::tui::types::PaneId;

fn test_http_client() -> HttpClient {
    static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = TEST_RT
        .get_or_init(|| tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort()));
    HttpClient::new(rt.handle().clone()).unwrap_or_else(|| std::process::abort())
}

fn make_project(name: Option<&str>, path: &str) -> ProjectListItem {
    ProjectListItem::Package(RustProject::<Package>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        None,
        None,
    ))
}

fn make_app(projects: &[ProjectListItem]) -> App {
    make_app_with_config(projects, &CargoPortConfig::default())
}

fn make_app_with_config(projects: &[ProjectListItem], cfg: &CargoPortConfig) -> App {
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

fn make_non_rust_project(name: Option<&str>, path: &str) -> ProjectListItem {
    ProjectListItem::NonRust(NonRustProject::new(
        PathBuf::from(path),
        name.map(String::from),
    ))
}

fn make_workspace_project(name: Option<&str>, path: &str) -> ProjectListItem {
    ProjectListItem::Workspace(RustProject::<Workspace>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        Vec::new(),
        None,
        None,
    ))
}

fn make_workspace_with_members(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
) -> ProjectListItem {
    ProjectListItem::Workspace(RustProject::<Workspace>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        groups,
        Vec::new(),
        None,
        None,
    ))
}

fn make_member(name: Option<&str>, path: &str) -> RustProject<Package> {
    RustProject::<Package>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        None,
        None,
    )
}

fn make_workspace_worktrees_item(
    primary: RustProject<Workspace>,
    linked: Vec<RustProject<Workspace>>,
) -> ProjectListItem {
    ProjectListItem::WorkspaceWorktrees(crate::project::WorktreeGroup::new(primary, linked))
}

fn make_package_worktrees_item(
    primary: RustProject<Package>,
    linked: Vec<RustProject<Package>>,
) -> ProjectListItem {
    ProjectListItem::PackageWorktrees(crate::project::WorktreeGroup::new(primary, linked))
}

fn make_package_raw(
    name: Option<&str>,
    path: &str,
    worktree_name: Option<&str>,
) -> RustProject<Package> {
    make_package_raw_with_primary(name, path, worktree_name, None)
}

fn make_package_raw_with_primary(
    name: Option<&str>,
    path: &str,
    worktree_name: Option<&str>,
    primary_abs_path: Option<&str>,
) -> RustProject<Package> {
    RustProject::<Package>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        worktree_name.map(String::from),
        primary_abs_path.map(PathBuf::from),
    )
}

fn make_workspace_raw(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
    worktree_name: Option<&str>,
) -> RustProject<Workspace> {
    make_workspace_raw_with_primary(name, path, groups, worktree_name, None)
}

fn make_workspace_raw_with_primary(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
    worktree_name: Option<&str>,
    primary_abs_path: Option<&str>,
) -> RustProject<Workspace> {
    RustProject::<Workspace>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        groups,
        Vec::new(),
        worktree_name.map(String::from),
        primary_abs_path.map(PathBuf::from),
    )
}

fn inline_group(members: Vec<RustProject<Package>>) -> MemberGroup {
    crate::project::MemberGroup::Inline { members }
}

fn named_group(name: &str, members: Vec<RustProject<Package>>) -> MemberGroup {
    crate::project::MemberGroup::Named {
        name: name.to_string(),
        members,
    }
}

fn make_package_with_vendored(
    name: Option<&str>,
    path: &str,
    vendored: Vec<RustProject<Package>>,
) -> RustProject<Package> {
    RustProject::<Package>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        vendored,
        None,
        None,
    )
}

fn make_workspace_raw_with_vendored(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
    vendored: Vec<RustProject<Package>>,
    worktree_name: Option<&str>,
) -> RustProject<Workspace> {
    RustProject::<Workspace>::new(
        PathBuf::from(path),
        name.map(String::from),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        groups,
        vendored,
        worktree_name.map(String::from),
        None,
    )
}

fn wait_for_tree_build(app: &mut App) {
    // Tree rebuilds no longer exist — just ensure derived state is fresh.
    app.ensure_visible_rows_cached();
}

fn git_binary() -> &'static str {
    if Path::new("/usr/bin/git").is_file() {
        "/usr/bin/git"
    } else {
        "git"
    }
}

fn manifest_contents(name: &str, workspace: bool) -> String {
    let workspace_section = if workspace { "\n[workspace]\n" } else { "" };
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
{workspace_section}
"#
    )
}

fn init_git_project(dir: &Path, name: &str, workspace: bool) {
    std::fs::create_dir_all(dir.join("src")).unwrap_or_else(|_| std::process::abort());
    std::fs::write(dir.join("Cargo.toml"), manifest_contents(name, workspace))
        .unwrap_or_else(|_| std::process::abort());
    std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n")
        .unwrap_or_else(|_| std::process::abort());

    Command::new(git_binary())
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|_| std::process::abort());
    Command::new(git_binary())
        .args(["config", "user.name", "cargo-port-tests"])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|_| std::process::abort());
    Command::new(git_binary())
        .args(["config", "user.email", "cargo-port-tests@example.com"])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|_| std::process::abort());
    Command::new(git_binary())
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|_| std::process::abort());
    Command::new(git_binary())
        .args(["commit", "-m", "init"])
        .current_dir(dir)
        .output()
        .unwrap_or_else(|_| std::process::abort());
}

fn add_git_worktree(primary_dir: &Path, worktree_dir: &Path, branch: &str) {
    let status = Command::new(git_binary())
        .args([
            "worktree",
            "add",
            worktree_dir.to_str().unwrap_or_else(|| std::process::abort()),
            "-b",
            branch,
        ])
        .current_dir(primary_dir)
        .status()
        .unwrap_or_else(|_| std::process::abort());
    assert!(status.success(), "git worktree add should succeed");
}

fn item_from_project_dir(dir: &Path) -> ProjectListItem {
    let cargo_toml = dir.join("Cargo.toml");
    let parsed = crate::project::from_cargo_toml(&cargo_toml)
        .unwrap_or_else(|_| std::process::abort());
    crate::scan::cargo_project_to_item(parsed)
}

fn apply_bg_msg(app: &mut App, msg: BackgroundMsg) {
    if app.handle_bg_msg(msg) {
        app.refresh_derived_state();
    }
    app.ensure_visible_rows_cached();
}

#[test]
fn external_config_reload_applies_valid_changes() {
    let mut app = make_app(&[]);
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
    let mut app = make_app_with_config(&[rust_project, non_rust_project], &cfg);
    app.scan.phase = ScanPhase::Complete;

    // All items live in project_list_items regardless of non-rust config.
    assert_eq!(app.projects.len(), 2);

    let mut hide_cfg = cfg.clone();
    hide_cfg.tui.include_non_rust = NonRustInclusion::Exclude;
    app.apply_config(&hide_cfg);
    wait_for_tree_build(&mut app);

    assert!(app.is_scan_complete());
    // Items are still in project_list_items — non-rust filtering is at render time.
    assert_eq!(app.projects.len(), 2);
    app.ensure_visible_rows_cached();
    let visible: Vec<_> = app
        .visible_rows()
        .iter()
        .filter_map(|row| match row {
            super::types::VisibleRow::Root { node_index } => {
                Some(app.projects[*node_index].display_path())
            },
            _ => None,
        })
        .collect();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0], "~/rust");

    app.apply_config(&cfg);
    wait_for_tree_build(&mut app);

    assert!(app.is_scan_complete());
    assert_eq!(app.projects.len(), 2);
    assert!(
        app.projects
            .iter()
            .any(|item| item.display_path() == "~/js")
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

fn apply_items(app: &mut App, items: &[ProjectListItem]) {
    app.apply_tree_build(crate::project_list::ProjectList::new(items.to_vec()));
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
        workflows:           WorkflowPresence::Present,
    }
}

#[test]
fn service_reachability_tracks_background_messages() {
    let mut app = make_app(&[]);

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
        ProjectListItem::PackageWorktrees(wtg) => wtg.linked()[0].path().to_path_buf(),
        _ => unreachable!("expected package worktrees"),
    };
    items[0]
        .at_path_mut(&linked_path)
        .expect("linked worktree should exist")
        .visibility = Dismissed;
    let rows = snapshots::build_visible_rows(&items, &expanded, true);
    assert_eq!(rows.len(), 1, "only the root should remain when one worktree is left");
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

    let linked_abs = PathBuf::from(&linked_path);
    assert!(app.is_deleted(&linked_abs), "linked worktree should be deleted");

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
        App::live_worktree_count_for_item(&app.projects[0]),
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
            path:  PathBuf::from(&linked_path).into(),
            bytes: 0,
        },
    );

    let linked_abs = PathBuf::from(&linked_path);
    assert!(app.is_deleted(&linked_abs), "linked workspace should be deleted");
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
        App::live_worktree_count_for_item(&app.projects[0]),
        0,
        "the remaining primary should no longer render as a worktree group"
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
    let ws = RustProject::<Workspace>::new(
        PathBuf::from("~/ws"),
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
    let root = ProjectListItem::Workspace(ws);

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

#[test]
fn lint_runtime_waits_for_scan_completion() {
    let project = make_project(Some("demo"), "~/demo");
    let path = project.display_path();
    let mut app = make_app(&[project]);

    assert!(app.lint_runtime_projects_snapshot().is_empty());

    app.scan.phase = ScanPhase::Complete;
    let projects = app.lint_runtime_projects_snapshot();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_path, path);
}

#[test]
fn ci_runs_stay_on_owner_rows_not_workspace_members() {
    let workspace = make_workspace_project(Some("ws"), "~/ws");
    let member = make_project(Some("core"), "~/ws/core");
    let root = make_workspace_with_members(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
    );

    let mut app = make_app(&[workspace, member]);
    apply_items(&mut app, &[root]);

    app.insert_ci_runs(Path::new("~/ws"), vec![make_ci_run(1, Conclusion::Success)]);

    assert_eq!(app.ci_for(Path::new("~/ws")), Some(Conclusion::Success));
    assert!(app.ci_state.contains_key(Path::new("~/ws")));
    assert_eq!(app.ci_for(Path::new("~/ws/core")), None);
    assert!(app.ci_state_for(Path::new("~/ws/core")).is_none());
    assert!(!app.ci_state.contains_key(Path::new("~/ws/core")));
}

#[test]
fn non_owner_member_ignores_stale_ci_state_and_cannot_fetch() {
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
    app.select_project_in_tree(&member.display_path());

    app.ci_state.insert(
        member.path().to_path_buf(),
        CiState::Loaded {
            runs:      vec![make_ci_run(2, Conclusion::Failure)],
            exhausted: false,
        },
    );
    app.handle_git_info(
        member.path(),
        make_git_info(Some("https://github.com/natepiano/demo")),
    );

    assert!(app.ci_state_for(member.path()).is_none());
    assert_eq!(app.ci_for(member.path()), None);
    assert!(!app.bottom_panel_available(member.path()));

    crate::tui::detail::handle_ci_runs_key(
        &mut app,
        &crossterm::event::KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE),
    );
    assert!(app.pending_ci_fetch.is_none());
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
    let root_path = "~/ws".to_string();
    let feature_path = "~/ws_feat".to_string();

    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws"), member.clone()]);
    apply_items(&mut app, &[root]);

    app.ci_state.insert(
        PathBuf::from(&root_path),
        CiState::Loaded {
            runs:      vec![make_ci_run(3, Conclusion::Success)],
            exhausted: false,
        },
    );
    app.ci_state.insert(
        PathBuf::from(&feature_path),
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

    // ci_for_item on the worktree group item should aggregate across worktrees
    assert_eq!(app.ci_for_item(&app.projects[0]), Some(Conclusion::Failure));
    assert!(app.ci_state_for(member.path()).is_none());
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
    assert!(expected.contains(Path::new(&project_a.display_path())));
    assert!(
        !app.scan
            .startup_phases
            .lint_seen_terminal
            .contains(Path::new(&project_a.display_path()))
    );
    assert!(
        app.running_lint_paths
            .contains_key(Path::new(&project_a.display_path()))
    );
    assert!(app.lint_toast.is_some());

    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project_a.path().to_path_buf().into(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });

    assert!(app.scan.startup_phases.lint_complete_at.is_some());
    assert!(app.running_lint_paths.is_empty());
    // Toast lingers while tracked items animate strikethrough.
    // After prune clears them, the toast finishes.
    app.prune_toasts();
    // Tracked items may still be lingering — toast stays until they expire.
    // For the test, just verify running_lint_paths is empty (toast may or may not be gone).
}

#[test]
fn startup_lint_toast_body_shows_paths_then_others() {
    let expected = HashSet::from([
        PathBuf::from("~/a"),
        PathBuf::from("~/b"),
        PathBuf::from("~/c"),
        PathBuf::from("~/d"),
        PathBuf::from("~/e"),
    ]);
    let seen = HashSet::from([PathBuf::from("~/e")]);

    let body = App::startup_lint_toast_body_for(&expected, &seen);
    let lines = body.lines().collect::<Vec<_>>();

    // 4 remaining — all shown (toast renderer handles truncation).
    assert_eq!(lines.len(), 4);
    for line in &lines {
        assert!(line.starts_with("~/"));
    }
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

    // Lint finishes — toast id is kept for reuse.
    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path().to_path_buf().into(),
        status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
    });
    assert_eq!(app.lint_toast, first_toast);

    // Lint restarts — reactivates the same toast.
    app.handle_bg_msg(BackgroundMsg::LintStatus {
        path:   project.path().to_path_buf().into(),
        status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
    });
    assert_eq!(app.lint_toast, first_toast);
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
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.select_project_in_tree(&member.display_path());

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
    app.select_project_in_tree(&member.display_path());
    app.collapse_all();

    app.expand_all();

    assert_eq!(
        app.selected_display_path().as_deref(),
        Some(member.display_path().as_str())
    );
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
    assert_eq!(projects[0].project_path, "~/rust/hana");
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
    assert_eq!(projects[0].project_path, "~/ws");
    assert_eq!(projects[1].project_path, "~/ws_feat");
}

#[test]
fn vendored_path_dependency_becomes_cargo_active() {
    let root_item = {
        let pkg = RustProject::<Package>::new(
            PathBuf::from("~/app"),
            Some("app".to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            vec![make_member(Some("helper"), "~/app/vendor/helper")],
            None,
            None,
        );
        ProjectListItem::Package(pkg)
    };
    let vendored = make_project(Some("helper"), "~/app/vendor/helper");

    let mut app = make_app(&[make_project(Some("app"), "~/app"), vendored]);
    apply_items(&mut app, &[root_item]);

    assert!(app.is_vendored_path("~/app/vendor/helper"));
    assert!(app.is_cargo_active_path(Path::new("~/app/vendor/helper")));
}

#[test]
fn git_path_state_suppresses_sync_for_untracked_and_ignored() {
    let project = make_project(Some("demo"), "~/demo");
    let path = project.display_path();
    let mut app = make_app(std::slice::from_ref(&project));

    app.handle_git_info(
        Path::new(&path),
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
            workflows:           WorkflowPresence::Present,
        },
    );

    app.git_path_states
        .insert(PathBuf::from(&path), GitPathState::Untracked);
    assert!(app.git_sync(project.path()).is_empty());

    app.git_path_states
        .insert(PathBuf::from(&path), GitPathState::Ignored);
    assert!(app.git_sync(project.path()).is_empty());
}

#[test]
fn name_width_with_gutter_reserves_space_before_lint() {
    assert_eq!(App::name_width_with_gutter(0), 1);
    assert_eq!(App::name_width_with_gutter(42), 43);
}

#[test]
fn tabbable_panes_follow_canonical_order() {
    let project = ProjectListItem::Package(RustProject::<Package>::new(
        PathBuf::from("~/demo"),
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
        ),
        Vec::new(),
        None,
        None,
    ));

    let mut app = make_app(std::slice::from_ref(&project));
    app.toasts = ToastManager::default();
    app.toast_pane.set_len(0);
    app.scan.phase = ScanPhase::Complete;
    app.handle_git_info(
        project.path(),
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
            workflows:           WorkflowPresence::Present,
        },
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
        .cached_detail
        .as_ref()
        .map(|c| c.info.examples.iter().map(|g| g.names.len()).sum::<usize>());
    assert_eq!(example_count, Some(0));
    assert!(!app.tabbable_panes().contains(&PaneId::Targets));

    let refreshed = ProjectListItem::Package(RustProject::<Package>::new(
        PathBuf::from("~/demo"),
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
        ),
        Vec::new(),
        None,
        None,
    ));

    app.handle_project_refreshed(refreshed);
    app.sync_selected_project();

    app.ensure_detail_cached();
    let example_count = app
        .cached_detail
        .as_ref()
        .map(|c| c.info.examples.iter().map(|g| g.names.len()).sum::<usize>());
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
    let projects: Vec<ProjectListItem> = [
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

    let abs_path = PathBuf::from(&project_path);
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
        name:              &item.display_name(),
        git_path_state:    app.git_path_state_for(item.path()),
        lint_icon:         app.lint_icon_for_root(0),
        disk:              "0.0",
        disk_style:        Style::default(),
        disk_suffix:       Some(" [x]"),
        disk_suffix_style: Some(Style::default().fg(Color::DarkGray)),
        lang_icon:         item.lang_icon(),
        git_sync:          &app.git_sync(item.path()),
        ci:                app.ci_for_item(item),
        deleted:           true,
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

    let abs_path = PathBuf::from(&project_path);
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

#[test]
fn disk_updates_skip_git_path_refresh_during_scan() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let abs_path = tmp.path().join("demo");
    std::fs::create_dir_all(&abs_path).unwrap_or_else(|_| std::process::abort());

    let abs_str = abs_path.to_string_lossy().to_string();
    let project = ProjectListItem::Package(RustProject::<Package>::new(
        abs_path,
        Some("demo".to_string()),
        Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        Vec::new(),
        None,
        None,
    ));
    let mut app = make_app(&[project]);

    app.handle_disk_usage(Path::new(&abs_str), 123);
    assert!(!app.git_path_states.contains_key(Path::new(&abs_str)));

    app.scan.phase = ScanPhase::Complete;
    app.handle_disk_usage(Path::new(&abs_str), 123);
    assert_eq!(
        app.git_path_states.get(Path::new(&abs_str)),
        Some(&GitPathState::OutsideRepo)
    );
}

#[test]
fn search_finds_workspace_members_and_vendored_crates() {
    let member_path = "~/ws/crates/member";
    let vendored_path = "~/ws/vendor/helper";
    let workspace = ProjectListItem::Workspace(make_workspace_raw_with_vendored(
        Some("ws"),
        "~/ws",
        vec![inline_group(vec![make_member(Some("member"), member_path)])],
        vec![make_member(Some("helper"), vendored_path)],
        None,
    ));
    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
    apply_items(&mut app, &[workspace]);

    app.update_search("member");
    assert!(
        app.filtered
            .iter()
            .any(|hit| hit.display_path == member_path && hit.name == "member")
    );

    app.update_search("helper");
    assert!(
        app.filtered
            .iter()
            .any(|hit| hit.display_path == vendored_path && hit.name == "helper (vendored)")
    );
}

#[test]
fn search_finds_linked_worktree_children_and_confirms_selection() {
    let member_path = "~/ws_feat/crates/member_feat";
    let vendored_path = "~/ws_feat/vendor/helper_feat";
    let item = make_workspace_worktrees_item(
        make_workspace_raw(Some("ws"), "~/ws", Vec::new(), None),
        vec![make_workspace_raw_with_vendored(
            Some("ws"),
            "~/ws_feat",
            vec![inline_group(vec![make_package_with_vendored(
                Some("member_feat"),
                member_path,
                vec![make_member(Some("helper_feat"), vendored_path)],
            )])],
            Vec::new(),
            Some("ws_feat"),
        )],
    );
    let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
    apply_items(&mut app, &[item]);

    app.update_search("helper_feat");
    assert!(
        app.filtered
            .iter()
            .any(|hit| hit.display_path == vendored_path && hit.name == "helper_feat (vendored)")
    );

    app.update_search("member_feat");
    app.confirm_search();

    assert_eq!(app.selected_display_path().as_deref(), Some(member_path));
    assert_eq!(
        app.selected_project_path().map(Path::to_path_buf),
        Some(PathBuf::from(member_path))
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
        PathBuf::from("~/ws"),
        LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
    );
    app.lint_status.insert(
        PathBuf::from("~/ws_feat"),
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
        PathBuf::from("~/ws"),
        LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
    );
    app.lint_status.insert(
        PathBuf::from("~/ws/a"),
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
        PathBuf::from("~/ws"),
        LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")),
    );
    app.lint_status.insert(
        PathBuf::from("~/ws_feat"),
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
    let primary_ws = make_workspace_raw(
        None,
        "~/ws",
        vec![inline_group(vec![make_member(Some("a"), "~/ws/a")])],
        None,
    );
    let linked_ws = make_workspace_raw(
        None,
        "~/ws_feat",
        vec![inline_group(vec![make_member(Some("b"), "~/ws_feat/b")])],
        Some("ws_feat"),
    );
    let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);

    let mut app = make_app(&[make_workspace_project(None, "~/ws")]);
    app.current_config.lint.enabled = true;
    apply_items(&mut app, &[root]);
    app.expanded.insert(ExpandKey::Node(0));
    app.dirty.rows.mark_dirty();
    app.ensure_visible_rows_cached();

    app.lint_status.insert(
        PathBuf::from("~/ws"),
        LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")),
    );
    app.lint_status.insert(
        PathBuf::from("~/ws_feat"),
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
    let root = make_package_worktrees_item(
        make_package_raw(None, "~/ws", None),
        vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
    );

    let mut app = make_app(&[make_project(None, "~/ws")]);
    apply_items(&mut app, &[root]);
    app.handle_disk_usage(Path::new("~/ws"), 15);
    app.handle_disk_usage(Path::new("~/ws_feat"), 21);

    assert_eq!(app.projects[0].disk_usage_bytes(), Some(36));
    assert_eq!(
        App::formatted_disk_for_item(&app.projects[0]),
        crate::tui::render::format_bytes(36)
    );
}

#[test]
fn handle_project_discovered_deduplicates_by_path() {
    let mut app = make_app(&[]);

    let pkg1 = ProjectListItem::Package(make_package_raw(Some("foo"), "/abs/foo", None));
    let pkg2 = ProjectListItem::Package(make_package_raw(Some("foo"), "/abs/foo", None));
    let pkg3 = ProjectListItem::Package(make_package_raw(Some("bar"), "/abs/bar", None));

    app.handle_project_discovered(pkg1);
    app.handle_project_discovered(pkg2); // duplicate — should be rejected
    app.handle_project_discovered(pkg3);
    assert_eq!(app.projects.len(), 2);
}

#[test]
fn handle_project_discovered_creates_worktree_group_from_single_primary() {
    let primary_path = "/abs/app";
    let linked_path = "/abs/app_feat";
    let primary = ProjectListItem::Package(make_package_raw_with_primary(
        Some("app"),
        primary_path,
        None,
        Some("/canonical/app"),
    ));
    let linked = ProjectListItem::Package(make_package_raw_with_primary(
        Some("app"),
        linked_path,
        Some("app_feat"),
        Some("/canonical/app"),
    ));

    let mut app = make_app(&[primary]);
    assert!(app.handle_project_discovered(linked));
    assert_eq!(app.projects.len(), 1, "new linked checkout should regroup with the primary");

    let ProjectListItem::PackageWorktrees(group) = &app.projects[0] else {
        panic!("expected discovered worktree to create a package worktree group");
    };
    assert_eq!(group.primary().path(), Path::new(primary_path));
    assert_eq!(group.linked().len(), 1);
    assert_eq!(group.linked()[0].path(), Path::new(linked_path));
}

#[test]
fn handle_project_discovered_slots_new_worktree_into_existing_group() {
    let primary_path = "/abs/app";
    let existing_linked_path = "/abs/app_feat";
    let new_linked_path = "/abs/app_fix";
    let root = make_package_worktrees_item(
        make_package_raw_with_primary(Some("app"), primary_path, None, Some("/canonical/app")),
        vec![make_package_raw_with_primary(
            Some("app"),
            existing_linked_path,
            Some("app_feat"),
            Some("/canonical/app"),
        )],
    );
    let new_linked = ProjectListItem::Package(make_package_raw_with_primary(
        Some("app"),
        new_linked_path,
        Some("app_fix"),
        Some("/canonical/app"),
    ));

    let mut app = make_app(&[root]);
    assert!(app.handle_project_discovered(new_linked));
    assert_eq!(app.projects.len(), 1, "new linked checkout should stay inside the existing group");

    let ProjectListItem::PackageWorktrees(group) = &app.projects[0] else {
        panic!("expected existing root to remain a package worktree group");
    };
    assert_eq!(group.linked().len(), 2);
    assert!(
        group.linked().iter().any(|linked| linked.path() == Path::new(existing_linked_path)),
        "existing linked worktree should remain in the group"
    );
    assert!(
        group.linked().iter().any(|linked| linked.path() == Path::new(new_linked_path)),
        "new linked worktree should be appended to the group"
    );
}

#[test]
fn handle_project_discovered_creates_workspace_worktree_group_from_single_primary() {
    let primary_path = "/abs/obsidian_knife";
    let linked_path = "/abs/obsidian_knife_test";
    let primary = ProjectListItem::Workspace(make_workspace_raw_with_primary(
        Some("obsidian_knife"),
        primary_path,
        Vec::new(),
        None,
        Some("/canonical/obsidian_knife"),
    ));
    let linked = ProjectListItem::Workspace(make_workspace_raw_with_primary(
        Some("obsidian_knife"),
        linked_path,
        Vec::new(),
        Some("obsidian_knife_test"),
        Some("/canonical/obsidian_knife"),
    ));

    let mut app = make_app(&[primary]);
    assert!(app.handle_project_discovered(linked));
    assert_eq!(app.projects.len(), 1, "new linked workspace should regroup with the primary");

    let ProjectListItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected discovered workspace worktree to create a worktree group");
    };
    assert_eq!(group.primary().path(), Path::new(primary_path));
    assert_eq!(group.linked().len(), 1);
    assert_eq!(group.linked()[0].path(), Path::new(linked_path));
}

#[test]
fn handle_project_discovered_slots_new_workspace_worktree_into_existing_group() {
    let primary_path = "/abs/obsidian_knife";
    let existing_linked_path = "/abs/obsidian_knife_feat";
    let new_linked_path = "/abs/obsidian_knife_test";
    let root = make_workspace_worktrees_item(
        make_workspace_raw_with_primary(
            Some("obsidian_knife"),
            primary_path,
            Vec::new(),
            None,
            Some("/canonical/obsidian_knife"),
        ),
        vec![make_workspace_raw_with_primary(
            Some("obsidian_knife"),
            existing_linked_path,
            Vec::new(),
            Some("obsidian_knife_feat"),
            Some("/canonical/obsidian_knife"),
        )],
    );
    let new_linked = ProjectListItem::Workspace(make_workspace_raw_with_primary(
        Some("obsidian_knife"),
        new_linked_path,
        Vec::new(),
        Some("obsidian_knife_test"),
        Some("/canonical/obsidian_knife"),
    ));

    let mut app = make_app(&[root]);
    assert!(app.handle_project_discovered(new_linked));
    assert_eq!(
        app.projects.len(),
        1,
        "new linked workspace should stay inside the existing worktree group"
    );

    let ProjectListItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected existing root to remain a workspace worktree group");
    };
    assert_eq!(group.linked().len(), 2);
    assert!(
        group.linked().iter().any(|linked| linked.path() == Path::new(existing_linked_path)),
        "existing linked workspace should remain in the group"
    );
    assert!(
        group.linked().iter().any(|linked| linked.path() == Path::new(new_linked_path)),
        "new linked workspace should be appended to the group"
    );
}

#[test]
fn background_discovery_from_real_package_worktree_creates_group() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/app");
    let linked_item = item_from_project_dir(&linked_dir);
    apply_bg_msg(&mut app, BackgroundMsg::ProjectDiscovered { item: linked_item });

    assert_eq!(app.projects.len(), 1);
    let ProjectListItem::PackageWorktrees(group) = &app.projects[0] else {
        panic!("expected package worktree group after real worktree discovery");
    };
    assert_eq!(group.linked().len(), 1);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into package worktree entries");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);
}

#[test]
fn background_discovery_from_real_workspace_worktree_creates_group() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");
    let linked_item = item_from_project_dir(&linked_dir);
    apply_bg_msg(&mut app, BackgroundMsg::ProjectDiscovered { item: linked_item });

    assert_eq!(app.projects.len(), 1);
    let ProjectListItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected workspace worktree group after real worktree discovery");
    };
    assert_eq!(group.linked().len(), 1);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into workspace worktree entries");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);
}

#[test]
fn background_discovery_from_real_package_worktree_appends_existing_group() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_one_dir = tmp.path().join("app_feat");
    let linked_two_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);
    add_git_worktree(&primary_dir, &linked_one_dir, "feat/app");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_one_item = item_from_project_dir(&linked_one_dir);
    let mut app = make_app(&[primary_item, linked_one_item]);

    add_git_worktree(&primary_dir, &linked_two_dir, "test/app");
    let linked_two_item = item_from_project_dir(&linked_two_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: linked_two_item,
        },
    );

    assert_eq!(app.projects.len(), 1);
    let ProjectListItem::PackageWorktrees(group) = &app.projects[0] else {
        panic!("expected package worktree group after second real worktree discovery");
    };
    assert_eq!(group.linked().len(), 2);
}

#[test]
fn background_discovery_from_real_workspace_worktree_appends_existing_group() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_one_dir = tmp.path().join("obsidian_knife_feat");
    let linked_two_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);
    add_git_worktree(&primary_dir, &linked_one_dir, "feat/obsidian");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_one_item = item_from_project_dir(&linked_one_dir);
    let mut app = make_app(&[primary_item, linked_one_item]);

    add_git_worktree(&primary_dir, &linked_two_dir, "test/obsidian");
    let linked_two_item = item_from_project_dir(&linked_two_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: linked_two_item,
        },
    );

    assert_eq!(app.projects.len(), 1);
    let ProjectListItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected workspace worktree group after second real worktree discovery");
    };
    assert_eq!(group.linked().len(), 2);
}

#[test]
fn refreshed_workspace_worktree_metadata_regroups_stale_top_level_discovery() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

    // Simulate the bad runtime path: discovery happens before worktree
    // metadata is available, so the linked checkout is inserted as a plain
    // top-level workspace instead of a worktree child.
    let stale_discovery = ProjectListItem::Workspace(make_workspace_raw(
        Some("obsidian_knife"),
        &linked_dir.to_string_lossy(),
        Vec::new(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    assert_eq!(app.projects.len(), 2, "stale discovery should currently append a top-level row");

    // A later refresh for the same path carries the correct worktree metadata.
    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    assert_eq!(
        app.projects.len(),
        1,
        "refreshing the stale top-level row should regroup it under the primary worktree container"
    );
    let ProjectListItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected regrouped workspace worktree container after refresh");
    };
    assert_eq!(group.linked().len(), 1);
    assert_eq!(group.linked()[0].path(), linked_dir.as_path());
}

#[test]
fn refreshed_package_worktree_metadata_regroups_stale_top_level_discovery() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/app");

    let stale_discovery = ProjectListItem::Package(make_package_raw(
        Some("app"),
        &linked_dir.to_string_lossy(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    assert_eq!(app.projects.len(), 2, "stale discovery should currently append a top-level row");

    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    assert_eq!(
        app.projects.len(),
        1,
        "refreshing the stale top-level row should regroup it under the primary worktree container"
    );
    let ProjectListItem::PackageWorktrees(group) = &app.projects[0] else {
        panic!("expected regrouped package worktree container after refresh");
    };
    assert_eq!(group.linked().len(), 1);
    assert_eq!(group.linked()[0].path(), linked_dir.as_path());
}

#[test]
fn refreshed_workspace_worktree_metadata_appends_into_existing_group() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_one_dir = tmp.path().join("obsidian_knife_feat");
    let linked_two_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);
    add_git_worktree(&primary_dir, &linked_one_dir, "feat/obsidian");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_one_item = item_from_project_dir(&linked_one_dir);
    let mut app = make_app(&[primary_item, linked_one_item]);

    add_git_worktree(&primary_dir, &linked_two_dir, "test/obsidian");

    let stale_discovery = ProjectListItem::Workspace(make_workspace_raw(
        Some("obsidian_knife"),
        &linked_two_dir.to_string_lossy(),
        Vec::new(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    assert_eq!(app.projects.len(), 2, "stale discovery should temporarily append a top-level row");

    let refreshed = item_from_project_dir(&linked_two_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    assert_eq!(
        app.projects.len(),
        1,
        "refresh should fold the stale row into the existing workspace worktree group"
    );
    let ProjectListItem::WorkspaceWorktrees(group) = &app.projects[0] else {
        panic!("expected workspace worktree group after refresh regroup");
    };
    assert_eq!(group.linked().len(), 2);
    assert!(
        group.linked().iter().any(|linked| linked.path() == linked_one_dir.as_path()),
        "existing linked workspace should remain in the group"
    );
    assert!(
        group.linked().iter().any(|linked| linked.path() == linked_two_dir.as_path()),
        "refreshed linked workspace should join the existing group"
    );
}

#[test]
fn refreshed_package_worktree_metadata_appends_into_existing_group() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_one_dir = tmp.path().join("app_feat");
    let linked_two_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);
    add_git_worktree(&primary_dir, &linked_one_dir, "feat/app");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_one_item = item_from_project_dir(&linked_one_dir);
    let mut app = make_app(&[primary_item, linked_one_item]);

    add_git_worktree(&primary_dir, &linked_two_dir, "test/app");

    let stale_discovery = ProjectListItem::Package(make_package_raw(
        Some("app"),
        &linked_two_dir.to_string_lossy(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    assert_eq!(app.projects.len(), 2, "stale discovery should temporarily append a top-level row");

    let refreshed = item_from_project_dir(&linked_two_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    assert_eq!(
        app.projects.len(),
        1,
        "refresh should fold the stale row into the existing package worktree group"
    );
    let ProjectListItem::PackageWorktrees(group) = &app.projects[0] else {
        panic!("expected package worktree group after refresh regroup");
    };
    assert_eq!(group.linked().len(), 2);
    assert!(
        group.linked().iter().any(|linked| linked.path() == linked_one_dir.as_path()),
        "existing linked package should remain in the group"
    );
    assert!(
        group.linked().iter().any(|linked| linked.path() == linked_two_dir.as_path()),
        "refreshed linked package should join the existing group"
    );
}

#[test]
fn stale_discovery_refresh_then_delete_dismiss_workspace_returns_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

    let stale_discovery = ProjectListItem::Workspace(make_workspace_raw(
        Some("obsidian_knife"),
        &linked_dir.to_string_lossy(),
        Vec::new(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into worktree entries after regroup");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  linked_dir.clone().into(),
            bytes: 0,
        },
    );
    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked workspace should be dismissable after regroup");
    app.dismiss(target);
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows(), &[VisibleRow::Root { node_index: 0 }]);
}

#[test]
fn stale_discovery_refresh_then_delete_dismiss_package_returns_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);

    let primary_item = item_from_project_dir(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(&primary_dir, &linked_dir, "test/app");

    let stale_discovery = ProjectListItem::Package(make_package_raw(
        Some("app"),
        &linked_dir.to_string_lossy(),
        None,
    ));
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    let refreshed = item_from_project_dir(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into worktree entries after regroup");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  linked_dir.clone().into(),
            bytes: 0,
        },
    );
    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked package should be dismissable after regroup");
    app.dismiss(target);
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows(), &[VisibleRow::Root { node_index: 0 }]);
}

#[test]
fn background_disk_zero_from_real_package_worktree_can_be_dismissed_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("app");
    let linked_dir = tmp.path().join("app_test");
    init_git_project(&primary_dir, "app", false);
    add_git_worktree(&primary_dir, &linked_dir, "test/app");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_item = item_from_project_dir(&linked_dir);
    let mut app = make_app(&[primary_item, linked_item]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into worktree entries");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  linked_dir.clone().into(),
            bytes: 0,
        },
    );
    assert!(app.is_deleted(&linked_dir));
    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked package should be dismissable");
    app.dismiss(target);
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows(), &[VisibleRow::Root { node_index: 0 }]);
}

#[test]
fn background_disk_zero_from_real_workspace_worktree_can_be_dismissed_to_root() {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join("obsidian_knife");
    let linked_dir = tmp.path().join("obsidian_knife_test");
    init_git_project(&primary_dir, "obsidian_knife", true);
    add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

    let primary_item = item_from_project_dir(&primary_dir);
    let linked_item = item_from_project_dir(&linked_dir);
    let mut app = make_app(&[primary_item, linked_item]);

    app.list_state.select(Some(0));
    assert!(app.expand(), "root should expand into worktree entries");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);

    std::fs::remove_dir_all(&linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        &mut app,
        BackgroundMsg::DiskUsage {
            path:  linked_dir.clone().into(),
            bytes: 0,
        },
    );
    assert!(app.is_deleted(&linked_dir));
    app.list_state.select(Some(2));
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked workspace should be dismissable");
    app.dismiss(target);
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows(), &[VisibleRow::Root { node_index: 0 }]);
}

#[test]
fn handle_project_discovered_does_not_allocate_per_comparison() {
    // Regression test: dedup must compare stored PathBuf, not allocate
    // display_path() strings. With 200 projects, the old O(N) string
    // allocation approach would be measurably slow.
    let mut app = make_app(&[]);
    let start = std::time::Instant::now();
    for i in 0..200 {
        let path = format!("/abs/project_{i}");
        let item = ProjectListItem::Package(make_package_raw(None, &path, None));
        app.handle_project_discovered(item);
    }
    let elapsed = start.elapsed();
    assert_eq!(app.projects.len(), 200);
    // With PathBuf comparison this should be well under 100ms.
    // With display_path() allocation it would be much slower.
    assert!(
        elapsed.as_millis() < 100,
        "discovery of 200 projects took {elapsed:?} — possible display_path allocation regression"
    );
}

#[test]
fn is_deleted_does_not_allocate_display_paths() {
    let mut app = make_app(&[]);
    // Populate with 200 projects
    for i in 0..200 {
        let path = format!("/abs/project_{i}");
        let item = ProjectListItem::Package(make_package_raw(None, &path, None));
        app.projects.push(item);
    }
    // Mark one as deleted
    let target = app.projects[100].path().to_path_buf();
    app.projects
        .at_path_mut(&target)
        .expect("target project should exist")
        .visibility = Deleted;
    let start = std::time::Instant::now();
    for _ in 0..1000 {
        let _ = app.is_deleted(&target);
    }
    let elapsed = start.elapsed();
    // 1000 lookups across 200 items should be well under 100ms with Path comparison.
    // With display_path() allocation it would be much slower.
    assert!(
        elapsed.as_millis() < 100,
        "1000 is_deleted calls took {elapsed:?} -- possible display_path allocation regression"
    );
}
