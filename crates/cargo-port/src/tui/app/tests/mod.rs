use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use chrono::DateTime;
use chrono::FixedOffset;
use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::List;
use ratatui::widgets::Widget;

pub(super) use super::App;
use super::DismissTarget;
use super::types::*;
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::ci::FetchStatus;
use crate::config::CargoPortConfig;
use crate::config::NonRustInclusion;
use crate::config::ScrollDirection;
use crate::http::ServiceKind;
use crate::lint::LintStatus;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::GitStatus;
use crate::project::MemberGroup;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::project::ProjectFields;
use crate::project::RemoteInfo;
use crate::project::RemoteKind;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::VendoredPackage;
use crate::project::Visibility::Deleted;
use crate::project::Visibility::Dismissed;
use crate::project::WorkflowPresence;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::project::WorktreeStatus;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CiFetchResult;
use crate::tui::columns::ProjectListWidths;
use crate::tui::pane::PaneFocusState;
use crate::tui::panes::CiFetchKind;
use crate::tui::panes::PaneId;
pub(super) use crate::tui::project_list::ExpandKey;
use crate::tui::project_list::ProjectList;
pub(super) use crate::tui::project_list::VisibleRow;
use crate::tui::test_support as tui_test_support;

mod background;
mod discovery_shimmer;
mod framework_keymap;
mod interaction;
mod panes;
mod rows;
mod state;
mod worktrees;

fn test_path(path: &str) -> AbsolutePath {
    let pb = if path == "~" {
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(path))
    } else if let Some(rest) = path.strip_prefix("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(rest)
    } else {
        PathBuf::from(path)
    };
    AbsolutePath::from(pb)
}

fn status_for(worktree_marker: Option<&str>, primary_abs_path: Option<&str>) -> WorktreeStatus {
    match (worktree_marker, primary_abs_path) {
        (None, None) => WorktreeStatus::NotGit,
        (Some(_), Some(p)) => WorktreeStatus::Linked {
            primary: test_path(p),
        },
        (None, Some(p)) => WorktreeStatus::Primary { root: test_path(p) },
        (Some(_), None) => WorktreeStatus::Linked {
            primary: test_path("~/unknown-primary"),
        },
    }
}

fn make_project(name: Option<&str>, path: &str) -> RootItem {
    RootItem::Rust(RustProject::Package(Package {
        path: test_path(path),
        name: name.map(String::from),
        ..Package::default()
    }))
}

fn make_app(projects: &[RootItem]) -> App { tui_test_support::make_app(projects) }

fn make_app_with_config(projects: &[RootItem], cfg: &CargoPortConfig) -> App {
    tui_test_support::make_app_with_config(projects, cfg)
}

fn set_loaded_ci(app: &mut App, path: &Path, runs: Vec<CiRun>, exhausted: bool, github_total: u32) {
    let entry = app
        .project_list
        .entry_containing_mut(path)
        .unwrap_or_else(|| std::process::abort());
    let repo = entry.git_repo.get_or_insert_with(Default::default);
    repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
        runs,
        github_total,
        exhausted,
    });
}

fn loaded_ci<'a>(app: &'a App, path: &Path) -> &'a ProjectCiInfo {
    match &app
        .project_list
        .entry_containing(path)
        .and_then(|entry| entry.git_repo.as_ref())
        .unwrap_or_else(|| std::process::abort())
        .ci_data
    {
        ProjectCiData::Loaded(info) => info,
        ProjectCiData::Unfetched => std::process::abort(),
    }
}

fn rendered_root_name_cells(app: &mut App) -> Vec<String> {
    app.ensure_visible_rows_cached();
    let labels = app
        .project_list
        .resolved_root_labels(app.config.include_non_rust().includes_non_rust());
    let widths = crate::tui::panes::compute_project_list_widths(
        &app.project_list,
        &labels,
        app.config.lint_enabled(),
        0,
    );
    let items = crate::tui::panes::render_tree_items(app, &widths);
    let area = Rect::new(
        0,
        0,
        u16::try_from(widths.total_width()).unwrap_or(u16::MAX),
        u16::try_from(items.len()).unwrap_or(u16::MAX),
    );
    let mut buffer = Buffer::empty(area);
    List::new(items).render(area, &mut buffer);

    (0..area.height)
        .map(|y| {
            let mut row = String::new();
            for x in 0..area.width {
                row.push_str(buffer[(x, y)].symbol());
            }
            row.trim_end().to_string()
        })
        .collect()
}

fn render_tree_buffer(app: &mut App) -> (Buffer, ProjectListWidths) {
    app.ensure_visible_rows_cached();
    let labels = app
        .project_list
        .resolved_root_labels(app.config.include_non_rust().includes_non_rust());
    let widths = crate::tui::panes::compute_project_list_widths(
        &app.project_list,
        &labels,
        app.config.lint_enabled(),
        0,
    );
    let items = crate::tui::panes::render_tree_items(app, &widths);
    let area = Rect::new(
        0,
        0,
        u16::try_from(widths.total_width()).unwrap_or(u16::MAX),
        u16::try_from(items.len()).unwrap_or(u16::MAX),
    );
    let mut buffer = Buffer::empty(area);
    List::new(items).render(area, &mut buffer);
    (buffer, widths)
}

fn row_has_crossed_out_content(buffer: &Buffer, widths: &ProjectListWidths, row: usize) -> bool {
    (0..widths.total_width()).any(|x| {
        let cell = &buffer[(
            u16::try_from(x).unwrap_or(u16::MAX),
            u16::try_from(row).unwrap_or(u16::MAX),
        )];
        !cell.symbol().trim().is_empty()
            && cell.style().add_modifier.contains(Modifier::CROSSED_OUT)
    })
}

fn resolved_root_label(item: &RootItem) -> String {
    ProjectList::new(vec![item.clone()]).resolved_root_labels(true)[0].clone()
}

/// Wrap owned `RootItem`s in a `ProjectList` for test helpers that pass
/// them to finder/widths functions.
pub(super) fn as_entries(items: Vec<RootItem>) -> ProjectList {
    crate::tui::project_list::ProjectList::new(items)
}

fn make_non_rust_project(name: Option<&str>, path: &str) -> RootItem {
    RootItem::NonRust(NonRustProject::new(test_path(path), name.map(String::from)))
}

fn make_workspace_project(name: Option<&str>, path: &str) -> RootItem {
    RootItem::Rust(RustProject::Workspace(Workspace {
        path: test_path(path),
        name: name.map(String::from),
        ..Workspace::default()
    }))
}

fn make_workspace_with_members(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
) -> RootItem {
    RootItem::Rust(RustProject::Workspace(Workspace {
        path: test_path(path),
        name: name.map(String::from),
        groups,
        ..Workspace::default()
    }))
}

fn make_member(name: Option<&str>, path: &str) -> Package {
    Package {
        path: test_path(path),
        name: name.map(String::from),
        ..Package::default()
    }
}

fn make_workspace_worktrees_item(primary: Workspace, linked: Vec<Workspace>) -> RootItem {
    RootItem::Worktrees(WorktreeGroup::new(
        RustProject::Workspace(primary),
        linked.into_iter().map(RustProject::Workspace).collect(),
    ))
}

fn make_package_worktrees_item(primary: Package, linked: Vec<Package>) -> RootItem {
    RootItem::Worktrees(WorktreeGroup::new(
        RustProject::Package(primary),
        linked.into_iter().map(RustProject::Package).collect(),
    ))
}

fn make_package_raw(name: Option<&str>, path: &str, worktree_marker: Option<&str>) -> Package {
    make_package_raw_with_primary(name, path, worktree_marker, None)
}

fn make_package_raw_with_primary(
    name: Option<&str>,
    path: &str,
    worktree_marker: Option<&str>,
    primary_abs_path: Option<&str>,
) -> Package {
    Package {
        path: test_path(path),
        name: name.map(String::from),
        worktree_status: status_for(worktree_marker, primary_abs_path),
        ..Package::default()
    }
}

fn make_workspace_raw(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
    worktree_marker: Option<&str>,
) -> Workspace {
    make_workspace_raw_with_primary(name, path, groups, worktree_marker, None)
}

fn make_workspace_raw_with_primary(
    name: Option<&str>,
    path: &str,
    groups: Vec<MemberGroup>,
    worktree_marker: Option<&str>,
    primary_abs_path: Option<&str>,
) -> Workspace {
    Workspace {
        path: test_path(path),
        name: name.map(String::from),
        worktree_status: status_for(worktree_marker, primary_abs_path),
        groups,
        ..Workspace::default()
    }
}

fn inline_group(members: Vec<Package>) -> MemberGroup { MemberGroup::Inline { members } }

fn named_group(name: &str, members: Vec<Package>) -> MemberGroup {
    MemberGroup::Named {
        name: name.to_string(),
        members,
    }
}

fn make_package_with_vendored(
    name: Option<&str>,
    path: &str,
    vendored: Vec<VendoredPackage>,
) -> Package {
    Package {
        path: test_path(path),
        name: name.map(String::from),
        rust: RustInfo {
            vendored,
            ..RustInfo::default()
        },
        ..Package::default()
    }
}

fn make_vendored(name: Option<&str>, path: &str) -> VendoredPackage {
    VendoredPackage {
        path: test_path(path),
        name: name.map(String::from),
        ..VendoredPackage::default()
    }
}

fn wait_for_tree_build(app: &mut App) {
    // Tree rebuilds no longer exist - just ensure derived state is fresh.
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

fn init_workspace_git_project_with_member(dir: &Path, name: &str, member_name: &str) {
    let member_dir = dir.join(member_name);
    std::fs::create_dir_all(member_dir.join("src")).unwrap_or_else(|_| std::process::abort());
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[workspace]\nmembers = [\"{member_name}\"]\n\n[workspace.package]\nrepository = \"https://example.com/{name}\"\n"
        ),
    )
    .unwrap_or_else(|_| std::process::abort());
    std::fs::write(
        member_dir.join("Cargo.toml"),
        format!("[package]\nname = \"{member_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .unwrap_or_else(|_| std::process::abort());
    std::fs::write(member_dir.join("src").join("lib.rs"), "pub fn demo() {}\n")
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
            worktree_dir
                .to_str()
                .unwrap_or_else(|| std::process::abort()),
            "-b",
            branch,
        ])
        .current_dir(primary_dir)
        .status()
        .unwrap_or_else(|_| std::process::abort());
    assert!(status.success(), "git worktree add should succeed");
}

fn item_from_project_dir(dir: &Path) -> RootItem {
    let cargo_toml = dir.join("Cargo.toml");
    let parsed = project::from_cargo_toml(&cargo_toml).unwrap_or_else(|_| std::process::abort());
    scan::cargo_project_to_item(parsed)
}

fn apply_bg_msg(app: &mut App, msg: BackgroundMsg) {
    if app.handle_bg_msg(msg) {
        app.scan.bump_generation();
    }
    app.ensure_visible_rows_cached();
}

fn apply_items(app: &mut App, items: &[RootItem]) {
    app.apply_tree_build(ProjectList::new(items.to_vec()));
    app.ensure_visible_rows_cached();
}

fn parse_ts(ts: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(ts).unwrap_or_else(|_| std::process::abort())
}

fn make_ci_run(run_id: u64, conclusion: CiStatus) -> CiRun {
    CiRun {
        run_id,
        created_at: "2026-03-30T14:22:18Z".to_string(),
        branch: "main".to_string(),
        url: format!("https://github.com/natepiano/demo/actions/runs/{run_id}"),
        ci_status: conclusion,
        jobs: Vec::new(),
        wall_clock_secs: Some(1),
        commit_title: Some(format!("run {run_id}")),
        updated_at: None,
        fetched: FetchStatus::Fetched,
    }
}

fn make_git_info(url: Option<&str>) -> (CheckoutInfo, RepoInfo) {
    let checkout = CheckoutInfo {
        status:              GitStatus::Clean,
        branch:              Some("main".to_string()),
        last_commit:         None,
        ahead_behind_local:  None,
        primary_tracked_ref: Some("origin/main".to_string()),
    };
    let repo = RepoInfo {
        remotes:           vec![RemoteInfo {
            name:         "origin".to_string(),
            url:          url.map(String::from),
            owner:        Some("natepiano".to_string()),
            repo:         None,
            tracked_ref:  Some("origin/main".to_string()),
            ahead_behind: None,
            kind:         RemoteKind::Clone,
        }],
        workflows:         WorkflowPresence::Present,
        first_commit:      None,
        last_fetched:      None,
        default_branch:    Some("main".to_string()),
        local_main_branch: Some("main".to_string()),
    };
    (checkout, repo)
}

/// Apply a `(CheckoutInfo, RepoInfo)` bundle through the same
/// `BackgroundMsg` dispatch the runtime uses: `RepoInfo` first, then
/// `CheckoutInfo`. Routing through `apply_bg_msg` keeps the helper in sync
/// with generation bumps and any other dispatch-wide invariants.
fn apply_git_info(app: &mut App, path: &Path, (checkout, repo): (CheckoutInfo, RepoInfo)) {
    let abs = AbsolutePath::from(path);
    apply_bg_msg(
        app,
        BackgroundMsg::RepoInfo {
            path: abs.clone(),
            info: repo,
        },
    );
    apply_bg_msg(
        app,
        BackgroundMsg::CheckoutInfo {
            path: abs,
            info: checkout,
        },
    );
}

#[derive(Clone, Copy)]
enum WorktreeProjectKind {
    Package,
    Workspace,
}

impl WorktreeProjectKind {
    fn primary_name(self) -> &'static str {
        match self {
            Self::Package => "app",
            Self::Workspace => "obsidian_knife",
        }
    }

    fn linked_name(self) -> &'static str {
        match self {
            Self::Package => "app_test",
            Self::Workspace => "obsidian_knife_test",
        }
    }

    fn feature_name(self) -> &'static str {
        match self {
            Self::Package => "app_feat",
            Self::Workspace => "obsidian_knife_feat",
        }
    }

    fn branch_prefix(self) -> &'static str {
        match self {
            Self::Package => "app",
            Self::Workspace => "obsidian",
        }
    }

    fn init_primary_repo(self, dir: &Path) {
        init_git_project(dir, self.primary_name(), matches!(self, Self::Workspace));
    }

    fn root_item(dir: &Path) -> RootItem { item_from_project_dir(dir) }

    fn assert_group_layout(self, app: &App, linked_len: usize, context: &str) {
        assert_eq!(app.project_list.len(), 1, "{context}");
        let RootItem::Worktrees(group) = &app.project_list[0].item else {
            panic!("expected worktree group: {context}");
        };
        match (self, &group.primary) {
            (Self::Package, RustProject::Package(_))
            | (Self::Workspace, RustProject::Workspace(_)) => {
                assert_eq!(group.linked.len(), linked_len, "{context}");
            },
            (Self::Package, _) => panic!("expected package worktree group: {context}"),
            (Self::Workspace, _) => panic!("expected workspace worktree group: {context}"),
        }
    }
}

fn expect_real_discovery_creates_group(kind: WorktreeProjectKind) {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join(kind.primary_name());
    let linked_dir = tmp.path().join(kind.linked_name());
    kind.init_primary_repo(&primary_dir);

    let primary_item = WorktreeProjectKind::root_item(&primary_dir);
    let mut app = make_app(&[primary_item]);

    add_git_worktree(
        &primary_dir,
        &linked_dir,
        &format!("test/{}", kind.branch_prefix()),
    );
    let linked_item = WorktreeProjectKind::root_item(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered { item: linked_item },
    );

    kind.assert_group_layout(
        &app,
        1,
        "real worktree discovery should create a worktree group",
    );

    app.project_list.set_cursor(0);
    assert!(app.expand(), "root should expand into worktree entries");
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);
}

fn expect_real_discovery_appends_existing_group(kind: WorktreeProjectKind) {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join(kind.primary_name());
    let linked_one_dir = tmp.path().join(kind.feature_name());
    let linked_two_dir = tmp.path().join(kind.linked_name());
    kind.init_primary_repo(&primary_dir);
    add_git_worktree(
        &primary_dir,
        &linked_one_dir,
        &format!("feat/{}", kind.branch_prefix()),
    );

    let primary_item = WorktreeProjectKind::root_item(&primary_dir);
    let linked_one_item = WorktreeProjectKind::root_item(&linked_one_dir);
    let mut app = make_app(&[primary_item, linked_one_item]);

    add_git_worktree(
        &primary_dir,
        &linked_two_dir,
        &format!("test/{}", kind.branch_prefix()),
    );
    let linked_two_item = WorktreeProjectKind::root_item(&linked_two_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: linked_two_item,
        },
    );

    kind.assert_group_layout(
        &app,
        2,
        "second real worktree discovery should append inside the existing group",
    );
}

fn expect_synthetic_discovery_creates_group(kind: WorktreeProjectKind) {
    match kind {
        WorktreeProjectKind::Package => {
            let primary_path = "/abs/app";
            let linked_path = "/abs/app_feat";
            let primary = RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                Some("app"),
                primary_path,
                None,
                Some("/canonical/app"),
            )));
            let linked = RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                Some("app"),
                linked_path,
                Some("app_feat"),
                Some("/canonical/app"),
            )));

            let mut app = make_app(&[primary]);
            assert!(app.handle_project_discovered(linked));
            assert_eq!(app.project_list.len(), 1);

            let RootItem::Worktrees(group) = &app.project_list[0].item else {
                panic!("expected discovered worktree to create a package worktree group");
            };
            assert!(matches!(&group.primary, RustProject::Package(_)));
            assert_eq!(group.primary.path(), Path::new(primary_path));
            assert_eq!(group.linked.len(), 1);
            assert_eq!(group.linked[0].path(), Path::new(linked_path));
        },
        WorktreeProjectKind::Workspace => {
            let primary_path = "/abs/obsidian_knife";
            let linked_path = "/abs/obsidian_knife_test";
            let primary = RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_primary(
                Some("obsidian_knife"),
                primary_path,
                Vec::new(),
                None,
                Some("/canonical/obsidian_knife"),
            )));
            let linked = RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_primary(
                Some("obsidian_knife"),
                linked_path,
                Vec::new(),
                Some("obsidian_knife_test"),
                Some("/canonical/obsidian_knife"),
            )));

            let mut app = make_app(&[primary]);
            assert!(app.handle_project_discovered(linked));
            assert_eq!(app.project_list.len(), 1);

            let RootItem::Worktrees(group) = &app.project_list[0].item else {
                panic!("expected discovered workspace worktree to create a worktree group");
            };
            assert!(matches!(&group.primary, RustProject::Workspace(_)));
            assert_eq!(group.primary.path(), Path::new(primary_path));
            assert_eq!(group.linked.len(), 1);
            assert_eq!(group.linked[0].path(), Path::new(linked_path));
        },
    }
}

fn expect_synthetic_discovery_appends_existing_group(kind: WorktreeProjectKind) {
    match kind {
        WorktreeProjectKind::Package => {
            let primary_path = "/abs/app";
            let existing_linked_path = "/abs/app_feat";
            let new_linked_path = "/abs/app_fix";
            let root = make_package_worktrees_item(
                make_package_raw_with_primary(
                    Some("app"),
                    primary_path,
                    None,
                    Some("/canonical/app"),
                ),
                vec![make_package_raw_with_primary(
                    Some("app"),
                    existing_linked_path,
                    Some("app_feat"),
                    Some("/canonical/app"),
                )],
            );
            let new_linked = RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                Some("app"),
                new_linked_path,
                Some("app_fix"),
                Some("/canonical/app"),
            )));

            let mut app = make_app(&[root]);
            assert!(app.handle_project_discovered(new_linked));
            assert_eq!(app.project_list.len(), 1);

            let RootItem::Worktrees(group) = &app.project_list[0].item else {
                panic!("expected existing root to remain a package worktree group");
            };
            assert!(matches!(&group.primary, RustProject::Package(_)));
            assert_eq!(group.linked.len(), 2);
            assert!(
                group
                    .linked
                    .iter()
                    .any(|l| l.path() == Path::new(existing_linked_path))
            );
            assert!(
                group
                    .linked
                    .iter()
                    .any(|l| l.path() == Path::new(new_linked_path))
            );
        },
        WorktreeProjectKind::Workspace => {
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
            let new_linked =
                RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_primary(
                    Some("obsidian_knife"),
                    new_linked_path,
                    Vec::new(),
                    Some("obsidian_knife_test"),
                    Some("/canonical/obsidian_knife"),
                )));

            let mut app = make_app(&[root]);
            assert!(app.handle_project_discovered(new_linked));
            assert_eq!(app.project_list.len(), 1);

            let RootItem::Worktrees(group) = &app.project_list[0].item else {
                panic!("expected existing root to remain a workspace worktree group");
            };
            assert!(matches!(&group.primary, RustProject::Workspace(_)));
            assert_eq!(group.linked.len(), 2);
            assert!(
                group
                    .linked
                    .iter()
                    .any(|l| l.path() == Path::new(existing_linked_path))
            );
            assert!(
                group
                    .linked
                    .iter()
                    .any(|l| l.path() == Path::new(new_linked_path))
            );
        },
    }
}

fn expect_refresh_regroups_stale_top_level_discovery(kind: WorktreeProjectKind) {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join(kind.primary_name());
    let linked_dir = tmp.path().join(kind.linked_name());
    kind.init_primary_repo(&primary_dir);

    let primary_item = WorktreeProjectKind::root_item(&primary_dir);
    let mut app = make_app(&[primary_item]);
    add_git_worktree(
        &primary_dir,
        &linked_dir,
        &format!("test/{}", kind.branch_prefix()),
    );

    let stale_discovery = match kind {
        WorktreeProjectKind::Package => RootItem::Rust(RustProject::Package(make_package_raw(
            Some(kind.primary_name()),
            &linked_dir.to_string_lossy(),
            None,
        ))),
        WorktreeProjectKind::Workspace => {
            RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                Some(kind.primary_name()),
                &linked_dir.to_string_lossy(),
                Vec::new(),
                None,
            )))
        },
    };
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    assert_eq!(app.project_list.len(), 2);

    let refreshed = WorktreeProjectKind::root_item(&linked_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    kind.assert_group_layout(
        &app,
        1,
        "refreshing the stale top-level row should regroup it under the primary worktree container",
    );
    let RootItem::Worktrees(group) = &app.project_list[0].item else {
        unreachable!();
    };
    let _ = kind;
    assert_eq!(group.linked[0].path(), linked_dir.as_path());
}

fn expect_refresh_appends_stale_discovery_into_existing_group(kind: WorktreeProjectKind) {
    let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
    let primary_dir = tmp.path().join(kind.primary_name());
    let linked_one_dir = tmp.path().join(kind.feature_name());
    let linked_two_dir = tmp.path().join(kind.linked_name());
    kind.init_primary_repo(&primary_dir);
    add_git_worktree(
        &primary_dir,
        &linked_one_dir,
        &format!("feat/{}", kind.branch_prefix()),
    );

    let primary_item = WorktreeProjectKind::root_item(&primary_dir);
    let linked_one_item = WorktreeProjectKind::root_item(&linked_one_dir);
    let mut app = make_app(&[primary_item, linked_one_item]);

    add_git_worktree(
        &primary_dir,
        &linked_two_dir,
        &format!("test/{}", kind.branch_prefix()),
    );
    let stale_discovery = match kind {
        WorktreeProjectKind::Package => RootItem::Rust(RustProject::Package(make_package_raw(
            Some(kind.primary_name()),
            &linked_two_dir.to_string_lossy(),
            None,
        ))),
        WorktreeProjectKind::Workspace => {
            RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                Some(kind.primary_name()),
                &linked_two_dir.to_string_lossy(),
                Vec::new(),
                None,
            )))
        },
    };
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectDiscovered {
            item: stale_discovery,
        },
    );
    assert_eq!(app.project_list.len(), 2);

    let refreshed = WorktreeProjectKind::root_item(&linked_two_dir);
    apply_bg_msg(
        &mut app,
        BackgroundMsg::ProjectRefreshed { item: refreshed },
    );

    kind.assert_group_layout(
        &app,
        2,
        "refresh should fold the stale row into the existing worktree group",
    );
    let RootItem::Worktrees(group) = &app.project_list[0].item else {
        unreachable!();
    };
    let _ = kind;
    assert!(
        group
            .linked
            .iter()
            .any(|l| l.path() == linked_one_dir.as_path())
    );
    assert!(
        group
            .linked
            .iter()
            .any(|l| l.path() == linked_two_dir.as_path())
    );
}

fn assert_deleted_linked_worktree_dismisses_to_root(app: &mut App, linked_dir: &Path) {
    app.project_list.set_cursor(0);
    assert!(
        app.expand(),
        "root should expand into worktree entries after regroup"
    );
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows().len(), 3);

    std::fs::remove_dir_all(linked_dir).unwrap_or_else(|_| std::process::abort());
    apply_bg_msg(
        app,
        BackgroundMsg::DiskUsage {
            path:  linked_dir.to_path_buf().into(),
            bytes: 0,
        },
    );
    assert!(app.project_list.is_deleted(linked_dir));
    app.project_list.set_cursor(2);
    let target = app
        .focused_dismiss_target()
        .expect("deleted linked worktree should be dismissable");
    app.dismiss(target);
    app.ensure_visible_rows_cached();
    assert_eq!(app.visible_rows(), &[VisibleRow::Root { node_index: 0 }]);
}
