use std::fs;
use std::path::Path;

use anyhow::Context;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use ratatui::style::Modifier;

use crate::config::CargoPortConfig;
use crate::project::AbsolutePath;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeGroup;
use crate::project::WorktreeStatus;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::scan_state::ScanPhase;
use crate::tui::dismiss_target::DismissTarget;
use crate::tui::hit_test::HoverTarget;
use crate::tui::interaction;
use crate::tui::project_list::ExpandTarget;
use crate::tui::project_list::ProjectPathLookup;
use crate::tui::render;
use crate::tui::terminal;
use crate::tui::test_support;

const TERMINAL_WIDTH: u16 = 160;
const TERMINAL_HEIGHT: u16 = 50;

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::MouseButton;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use toml::Value;

use crate::config::ScrollDirection;
use crate::project::PinnedCommit;
use crate::project::Submodule;
use crate::tui::input;

fn package(path: &Path, name: &str) -> anyhow::Result<Package> {
    match test_support::make_package(name, path) {
        RootItem::Rust(RustProject::Package(package)) => Ok(package),
        _ => anyhow::bail!("make_package must return a package"),
    }
}

fn root(path: &Path, name: &str) -> RootItem { test_support::make_package(name, path) }

fn shows_path(app: &App, path: &Path) -> bool {
    app.project_list
        .visible_rows()
        .iter()
        .any(|row| app.project_list.path_for_row(*row) == Some(path))
}

fn complete_rescan(app: &mut App, projects: Vec<RootItem>) {
    app.prepare_rescan();
    let _ = app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects,
        disk_entries: Vec::new(),
    });
    assert!(matches!(app.scan.state.phase, ScanPhase::Complete));
    app.ensure_visible_rows_cached();
}

fn assert_deleted_row(app: &mut App, path: &Path, name: &str) -> anyhow::Result<()> {
    let project = app
        .project_list
        .at_path(path)
        .expect_found("retained project");
    assert_eq!(
        project.visibility,
        Visibility::Deleted,
        "the completed rescan must retain the Deleted state"
    );
    assert!(
        shows_path(app, path),
        "the retained project must remain visible: {path:?}"
    );
    app.ensure_detail_cached();
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    terminal.draw(|frame| render::ui(frame, app))?;
    let buffer = terminal.backend().buffer();
    let body = app.panes.project_list.body_rect;
    let (start_x, y) = test_support::find_text_cell(buffer, body, name)
        .context("the retained project name must render")?;
    for offset in 0..u16::try_from(name.chars().count())? {
        assert!(
            buffer[(start_x + offset, y)]
                .modifier
                .contains(Modifier::CROSSED_OUT),
            "every letter of the Deleted row must remain struck through"
        );
    }
    Ok(())
}

fn dismiss_and_check_late_replies(app: &mut App, path: &Path) -> anyhow::Result<()> {
    app.project_list.select_project_in_tree(path, false);
    let target = app
        .focused_dismiss_target()
        .context("the Deleted row must be dismissable")?;
    app.dismiss(target);
    for bytes in [0, 42] {
        let _ = app.handle_bg_msg(BackgroundMsg::DiskUsage {
            path: AbsolutePath::from(path),
            bytes,
        });
        app.ensure_visible_rows_cached();
        let project = app
            .project_list
            .at_path(path)
            .expect_found("stored dismissal");
        assert_eq!(
            project.visibility,
            Visibility::Dismissed,
            "a late {bytes}-byte reply must preserve the dismissal"
        );
        assert!(
            !shows_path(app, path),
            "a late disk reply must not restore the dismissed row"
        );
    }
    Ok(())
}

#[test]
fn deleted_plain_project_survives_completed_rescan_and_late_disk_replies() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let path = scratch.path().join("deleted-plain");
    fs::create_dir(&path)?;
    let mut app = test_support::make_app(&[root(&path, "deleted-plain")]);
    fs::remove_dir(&path)?;
    app.handle_disk_usage(&path, 0);

    complete_rescan(&mut app, Vec::new());

    assert_deleted_row(&mut app, &path, "deleted-plain")?;
    dismiss_and_check_late_replies(&mut app, &path)
}

#[test]
fn deleted_member_keeps_selection_and_open_group_when_rescan_reorders_roots() -> anyhow::Result<()>
{
    let scratch = tempfile::tempdir()?;
    let workspace_path = scratch.path().join("z-workspace");
    let member_path = workspace_path.join("crates/deleted-member");
    fs::create_dir_all(&member_path)?;
    let workspace = Workspace {
        path: AbsolutePath::from(workspace_path.as_path()),
        ..Workspace::default()
    };
    let mut config = CargoPortConfig::default();
    config.tui.inline_dirs.clear();
    let mut app = test_support::make_app_with_config(
        &[
            RootItem::Rust(RustProject::Workspace(workspace)),
            root(&member_path, "deleted-member"),
        ],
        &config,
    );
    let expanded = vec![
        ExpandTarget::Root(AbsolutePath::from(workspace_path.as_path())),
        ExpandTarget::Group(
            AbsolutePath::from(workspace_path.as_path()),
            "crates".to_string(),
        ),
    ];
    app.project_list.apply_expanded(&expanded);
    app.ensure_visible_rows_cached();
    selection_key(&mut app, KeyCode::End);
    assert!(matches!(
        app.project_list.at_path(&member_path),
        ProjectPathLookup::Found(_)
    ));
    assert_eq!(
        app.project_list.selected_project_path(),
        Some(member_path.as_path())
    );
    for target in &expanded {
        assert!(app.project_list.export_expanded().contains(target));
    }
    fs::remove_dir_all(workspace_path.join("crates"))?;
    app.handle_disk_usage(&member_path, 0);
    let fresh_workspace = Workspace {
        path: AbsolutePath::from(workspace_path.as_path()),
        ..Workspace::default()
    };

    complete_rescan(
        &mut app,
        vec![
            root(&scratch.path().join("a-new-root"), "a-new-root"),
            RootItem::Rust(RustProject::Workspace(fresh_workspace)),
        ],
    );

    assert_deleted_row(&mut app, &member_path, "deleted-member")?;
    assert_eq!(
        app.project_list.selected_project_path(),
        Some(member_path.as_path())
    );
    for target in expanded {
        assert!(app.project_list.export_expanded().contains(&target));
    }
    dismiss_and_check_late_replies(&mut app, &member_path)
}

#[test]
fn deleted_worktree_survives_completed_rescan_and_late_disk_replies() -> anyhow::Result<()> {
    let scratch = tempfile::tempdir()?;
    let primary_path = scratch.path().join("primary");
    let linked_path = scratch.path().join("deleted-worktree");
    fs::create_dir(&primary_path)?;
    fs::create_dir(&linked_path)?;
    let mut primary = package(&primary_path, "primary")?;
    primary.worktree_status = WorktreeStatus::Primary {
        root: primary.path.clone(),
    };
    let mut linked = package(&linked_path, "deleted-worktree")?;
    linked.worktree_status = WorktreeStatus::Linked {
        primary: primary.path.clone(),
    };
    let group = RootItem::Worktrees(WorktreeGroup::new(
        RustProject::Package(primary.clone()),
        vec![RustProject::Package(linked)],
    ));
    let mut app = test_support::make_app(&[group]);
    app.project_list
        .apply_expanded(&[ExpandTarget::Root(AbsolutePath::from(
            primary_path.as_path(),
        ))]);
    app.ensure_visible_rows_cached();
    fs::remove_dir(&linked_path)?;
    app.handle_disk_usage(&linked_path, 0);

    complete_rescan(
        &mut app,
        vec![RootItem::Rust(RustProject::Package(primary))],
    );

    assert_deleted_row(&mut app, &linked_path, "deleted-worktree")?;
    dismiss_and_check_late_replies(&mut app, &linked_path)
}

#[test]
fn removed_discovery_folder_keeps_both_projects_and_dismissals_after_rescan() -> anyhow::Result<()>
{
    let scratch = tempfile::tempdir()?;
    let discovery = scratch.path().join("discovery");
    let paths = [
        discovery.join("deleted-first"),
        discovery.join("deleted-second"),
    ];
    let names = ["deleted-first", "deleted-second"];
    let projects = paths
        .iter()
        .zip(names)
        .map(|(path, name)| root(path, name))
        .collect::<Vec<_>>();
    for path in &paths {
        fs::create_dir_all(path)?;
    }
    let mut app = test_support::make_app(&projects);
    fs::remove_dir_all(&discovery)?;
    for path in &paths {
        app.handle_disk_usage(path, 0);
    }

    complete_rescan(&mut app, Vec::new());

    for (path, name) in paths.iter().zip(names) {
        assert_deleted_row(&mut app, path, name)?;
        dismiss_and_check_late_replies(&mut app, path)?;
    }
    complete_rescan(&mut app, Vec::new());
    for path in &paths {
        for bytes in [0, 42] {
            let _ = app.handle_bg_msg(BackgroundMsg::DiskUsage {
                path: AbsolutePath::from(path.as_path()),
                bytes,
            });
        }
        let project = app
            .project_list
            .at_path(path)
            .expect_found("dismissal after rescan");
        assert_eq!(project.visibility, Visibility::Dismissed);
    }
    assert_eq!(app.project_list.row_count(), 0);
    Ok(())
}

#[test]
fn rescan_restores_live_selection_after_retained_row_clamps_cursor() {
    let live = Path::new("/z-live");
    let deleted = Path::new("/a-deleted");
    let mut app = test_support::make_app(&[root(deleted, "a-deleted"), root(live, "z-live")]);
    app.project_list
        .at_path_mut(deleted)
        .expect_found("deleted fixture")
        .visibility = Visibility::Deleted;
    selection_key(&mut app, KeyCode::End);

    app.prepare_rescan();
    app.sync_selected_project();
    let _ = app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     vec![root(live, "z-live")],
        disk_entries: Vec::new(),
    });

    assert_eq!(app.project_list.selected_project_path(), Some(live));
}

#[test]
fn submodule_refresh_preserves_dismissal_after_rescan() {
    let owner = Path::new("/repo");
    let child = Path::new("/repo/child");
    let submodule = Submodule {
        path:          AbsolutePath::from(child),
        name:          "child".to_string(),
        relative_path: "child".to_string(),
        url:           None,
        branch:        None,
        commit:        PinnedCommit::Absent,
        project_info:  crate::project::ProjectInfo::default(),
        git_repo:      None,
    };
    let mut app = test_support::make_app(&[root(owner, "repo")]);
    let _ = app.handle_bg_msg(BackgroundMsg::Submodules {
        path:       AbsolutePath::from(owner),
        submodules: vec![submodule.clone()],
    });
    app.project_list
        .at_path_mut(child)
        .expect_found("child fixture")
        .visibility = Visibility::Dismissed;
    complete_rescan(&mut app, vec![root(owner, "repo")]);
    let _ = app.handle_bg_msg(BackgroundMsg::Submodules {
        path:       AbsolutePath::from(owner),
        submodules: vec![submodule],
    });
    let _ = app.handle_bg_msg(BackgroundMsg::DiskUsage {
        path:  AbsolutePath::from(child),
        bytes: 42,
    });

    assert_eq!(
        app.project_list
            .at_path(child)
            .expect_found("retained child")
            .visibility,
        Visibility::Dismissed
    );
    assert!(!shows_path(&app, child));
}

fn app_with_deleted_member() -> App {
    let mut config = CargoPortConfig::default();
    config.tui.inline_dirs.clear();
    let workspace = Workspace {
        path: AbsolutePath::from("/workspace"),
        ..Workspace::default()
    };
    let member = Path::new("/workspace/crates/member");
    let mut app = test_support::make_app_with_config(
        &[
            RootItem::Rust(RustProject::Workspace(workspace)),
            root(member, "member"),
        ],
        &config,
    );
    app.project_list
        .at_path_mut(member)
        .expect_found("member fixture")
        .visibility = Visibility::Deleted;
    app
}

#[test]
fn rescan_keeps_deleted_member_when_workspace_becomes_non_rust() {
    let mut app = app_with_deleted_member();
    let non_rust = crate::project::NonRustProject::new(AbsolutePath::from("/workspace"), None);
    complete_rescan(&mut app, vec![RootItem::NonRust(non_rust)]);
    assert_eq!(
        app.project_list
            .at_path(Path::new("/workspace/crates/member"))
            .expect_found("retained member")
            .visibility,
        Visibility::Deleted
    );
}

#[test]
fn project_refresh_keeps_deleted_member_missing_from_probe() {
    let mut app = app_with_deleted_member();
    let workspace = Workspace {
        path: AbsolutePath::from("/workspace"),
        ..Workspace::default()
    };
    assert!(app.handle_project_refreshed(RootItem::Rust(RustProject::Workspace(workspace))));
    assert_eq!(
        app.project_list
            .at_path(Path::new("/workspace/crates/member"))
            .expect_found("retained member")
            .visibility,
        Visibility::Deleted
    );
}

#[test]
fn rescan_keeps_saved_selection_when_discovery_precedes_selection_sync() {
    let live = Path::new("/z-live");
    let deleted = Path::new("/a-deleted");
    let mut app = test_support::make_app(&[root(deleted, "a-deleted"), root(live, "z-live")]);
    app.project_list
        .at_path_mut(deleted)
        .expect_found("deleted fixture")
        .visibility = Visibility::Deleted;
    selection_key(&mut app, KeyCode::End);

    app.prepare_rescan();
    let _ = app.handle_bg_msg(BackgroundMsg::ProjectDiscovered {
        item: root(live, "z-live"),
    });
    input::handle_event(&mut app, &Event::Resize(TERMINAL_WIDTH, TERMINAL_HEIGHT));
    assert_eq!(app.project_list.paths.last_selected.as_deref(), Some(live));
    let _ = app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects:     vec![root(live, "z-live")],
        disk_entries: Vec::new(),
    });

    assert_eq!(app.project_list.selected_project_path(), Some(live));
}

fn selection_key(app: &mut App, code: KeyCode) {
    input::handle_event(
        app,
        &Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        )),
    );
}

fn selection_roots() -> Vec<RootItem> {
    vec![
        root(Path::new("/a-choice"), "a-choice"),
        root(Path::new("/z-choice"), "z-choice"),
    ]
}

fn selection_fixture(visibility: Visibility) -> App {
    let mut config = CargoPortConfig::default();
    config.mouse.invert_scroll = ScrollDirection::Normal;
    let mut app = test_support::make_app_with_config(&selection_roots(), &config);
    app.project_list.paths.last_selected = None;
    for path in ["/a-choice", "/z-choice"] {
        app.project_list
            .at_path_mut(Path::new(path))
            .expect_found("selection fixture")
            .visibility = visibility;
    }
    selection_key(&mut app, KeyCode::End);
    app
}

fn finish_selection_scan(app: &mut App, projects: Vec<RootItem>) {
    let _ = app.handle_bg_msg(BackgroundMsg::ScanResult {
        projects,
        disk_entries: Vec::new(),
    });
}

fn assert_selection_choice(app: &App, path: &Path) {
    assert_eq!(app.project_list.selected_project_path(), Some(path));
    assert_eq!(app.project_list.paths.last_selected.as_deref(), Some(path));
}

#[test]
fn rescan_keeps_user_keyboard_choice_of_deleted_row() {
    let mut app = selection_fixture(Visibility::Deleted);
    app.prepare_rescan();
    selection_key(&mut app, KeyCode::Home);
    finish_selection_scan(&mut app, Vec::new());
    assert_selection_choice(&app, Path::new("/a-choice"));
}

#[test]
fn rescan_keeps_user_keyboard_choice_of_live_row() {
    let mut app = selection_fixture(Visibility::Visible);
    app.prepare_rescan();
    let projects = discover_selection_roots(&mut app);
    selection_key(&mut app, KeyCode::Home);
    finish_selection_scan(&mut app, projects);
    assert_selection_choice(&app, Path::new("/a-choice"));
}

fn startup_selection_choice(persisted: Option<AbsolutePath>) {
    let mut app = selection_fixture(Visibility::Visible);
    app.scan.state.phase = ScanPhase::Running;
    app.project_list.paths.last_selected = persisted;
    selection_key(&mut app, KeyCode::Home);
    finish_selection_scan(&mut app, selection_roots());
    assert_selection_choice(&app, Path::new("/a-choice"));
}

#[test]
fn startup_scan_keeps_user_choice_over_persisted_selection() {
    startup_selection_choice(Some(AbsolutePath::from("/z-choice")));
}

#[test]
fn startup_scan_records_user_choice_without_persisted_selection() {
    startup_selection_choice(None);
}

#[test]
fn rescan_click_on_clamped_row_records_user_choice() -> anyhow::Result<()> {
    let mut app = selection_fixture(Visibility::Visible);
    app.project_list
        .at_path_mut(Path::new("/a-choice"))
        .expect_found("retained row")
        .visibility = Visibility::Deleted;
    app.prepare_rescan();
    app.sync_selected_project();
    assert_eq!(
        app.project_list.selected_project_path(),
        Some(Path::new("/a-choice"))
    );
    selection_mouse(&mut app, MouseEventKind::Down(MouseButton::Left))?;
    finish_selection_scan(&mut app, vec![root(Path::new("/z-choice"), "z-choice")]);
    assert_selection_choice(&app, Path::new("/a-choice"));
    Ok(())
}

#[test]
fn second_rescan_does_not_save_clamped_row_as_user_choice() {
    let mut app = selection_fixture(Visibility::Visible);
    app.project_list
        .at_path_mut(Path::new("/a-choice"))
        .expect_found("retained row")
        .visibility = Visibility::Deleted;
    app.prepare_rescan();
    app.sync_selected_project();
    app.prepare_rescan();
    finish_selection_scan(&mut app, vec![root(Path::new("/z-choice"), "z-choice")]);
    assert_selection_choice(&app, Path::new("/z-choice"));
}

#[test]
fn completed_scan_uses_first_visible_row_without_saving_missing_choice() {
    for visibility in [Visibility::Visible, Visibility::Dismissed] {
        let mut app = selection_fixture(visibility);
        app.scan.state.phase = ScanPhase::Running;
        app.project_list.paths.last_selected = Some(AbsolutePath::from("/missing-choice"));
        finish_selection_scan(
            &mut app,
            vec![
                root(Path::new("/b-fallback"), "b-fallback"),
                root(Path::new("/c-fallback"), "c-fallback"),
            ],
        );
        assert_eq!(
            app.project_list.selected_project_path(),
            Some(Path::new("/b-fallback"))
        );
        assert_eq!(
            app.project_list.paths.last_selected.as_deref(),
            Some(Path::new("/missing-choice"))
        );
    }
}

fn selection_row_position(app: &mut App, name: &str) -> anyhow::Result<(u16, u16)> {
    app.ensure_detail_cached();
    let mut terminal = Terminal::new(TestBackend::new(TERMINAL_WIDTH, TERMINAL_HEIGHT))?;
    terminal.draw(|frame| render::ui(frame, app))?;
    test_support::find_text_cell(
        terminal.backend().buffer(),
        app.panes.project_list.body_rect,
        name,
    )
    .context("the selection row must render")
}

fn selection_mouse(app: &mut App, kind: MouseEventKind) -> anyhow::Result<()> {
    let (column, row) = selection_row_position(app, "a-choice")?;
    input::handle_event(
        app,
        &Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }),
    );
    Ok(())
}

#[test]
fn rescan_keeps_user_wheel_choice() -> anyhow::Result<()> {
    let mut app = selection_fixture(Visibility::Deleted);
    app.prepare_rescan();
    selection_mouse(&mut app, MouseEventKind::ScrollUp)?;
    finish_selection_scan(&mut app, Vec::new());
    assert_selection_choice(&app, Path::new("/a-choice"));
    Ok(())
}

#[test]
fn rescan_keeps_user_finder_choice() {
    let mut app = selection_fixture(Visibility::Visible);
    app.prepare_rescan();
    let projects = discover_selection_roots(&mut app);
    selection_key(&mut app, KeyCode::Char('/'));
    for letter in "a-choice".chars() {
        selection_key(&mut app, KeyCode::Char(letter));
    }
    selection_key(&mut app, KeyCode::Enter);
    finish_selection_scan(&mut app, projects);
    assert_selection_choice(&app, Path::new("/a-choice"));
}

#[test]
fn rescan_keeps_user_expansion_choice() {
    let mut app = selection_fixture(Visibility::Deleted);
    app.prepare_rescan();
    selection_key(&mut app, KeyCode::Left);
    finish_selection_scan(&mut app, Vec::new());
    assert_selection_choice(&app, Path::new("/a-choice"));
}

#[test]
fn completed_scan_does_not_save_fallback_for_dismissed_choice() {
    let mut app = selection_fixture(Visibility::Deleted);
    app.prepare_rescan();
    app.project_list
        .at_path_mut(Path::new("/z-choice"))
        .expect_found("dismissed choice")
        .visibility = Visibility::Dismissed;
    finish_selection_scan(&mut app, Vec::new());
    assert_eq!(
        app.project_list.selected_project_path(),
        Some(Path::new("/a-choice"))
    );
    assert_eq!(
        app.project_list.paths.last_selected.as_deref(),
        Some(Path::new("/z-choice"))
    );
}

fn discover_selection_roots(app: &mut App) -> Vec<RootItem> {
    let projects = selection_roots();
    for item in &projects {
        let _ = app.handle_bg_msg(BackgroundMsg::ProjectDiscovered { item: item.clone() });
    }
    projects
}

#[test]
fn keyboard_dismissal_keeps_worktree_parent_selected_after_rescan() -> anyhow::Result<()> {
    let parent = Path::new("/z-parent");
    let linked = Path::new("/z-linked");
    let unrelated = Path::new("/a-unrelated");
    let mut primary = package(parent, "z-parent")?;
    primary.worktree_status = WorktreeStatus::Primary {
        root: primary.path.clone(),
    };
    let mut child = package(linked, "z-linked")?;
    child.worktree_status = WorktreeStatus::Linked {
        primary: primary.path.clone(),
    };
    child.rust.project_info.visibility = Visibility::Deleted;
    let mut app = test_support::make_app(&[
        root(unrelated, "a-unrelated"),
        RootItem::Worktrees(WorktreeGroup::new(
            RustProject::Package(primary.clone()),
            vec![RustProject::Package(child)],
        )),
    ]);
    app.project_list
        .apply_expanded(&[ExpandTarget::Root(AbsolutePath::from(parent))]);
    app.ensure_visible_rows_cached();
    selection_key(&mut app, KeyCode::End);
    selection_key(&mut app, KeyCode::Up);
    assert_selection_choice(&app, linked);
    app.project_list.mark_sync_stable();

    selection_key(&mut app, KeyCode::Char('x'));

    assert_selection_choice(&app, parent);
    assert!(app.project_list.sync().is_changed());
    assert!(!shows_path(&app, linked));
    complete_rescan(
        &mut app,
        vec![
            root(unrelated, "a-unrelated"),
            RootItem::Rust(RustProject::Package(primary)),
        ],
    );
    assert_selection_choice(&app, parent);
    Ok(())
}

#[test]
fn dismissal_button_keeps_clamped_selection_after_rescan() -> anyhow::Result<()> {
    let mut app = selection_fixture(Visibility::Deleted);
    app.project_list.mark_sync_stable();
    let (_, row) = selection_row_position(&mut app, "z-choice")?;
    let body = app.panes.project_list.body_rect;
    let position = (body.x + body.width.saturating_sub(2), row);
    assert!(matches!(
        interaction::hit_test_at(&app, Position::new(position.0, position.1)),
        Some(HoverTarget::Dismiss(DismissTarget::DeletedProject(path)))
            if path.as_path() == Path::new("/z-choice")
    ));
    input::handle_event(
        &mut app,
        &Event::Mouse(MouseEvent {
            kind:      MouseEventKind::Down(MouseButton::Left),
            column:    position.0,
            row:       position.1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }),
    );

    assert_selection_choice(&app, Path::new("/a-choice"));
    assert!(app.project_list.sync().is_changed());
    assert!(!shows_path(&app, Path::new("/z-choice")));
    complete_rescan(&mut app, Vec::new());
    assert_selection_choice(&app, Path::new("/a-choice"));
    Ok(())
}

#[test]
fn focus_gained_during_rescan_saves_selected_project_row() -> anyhow::Result<()> {
    let mut app = selection_fixture(Visibility::Deleted);
    app.prepare_rescan();
    app.project_list.mark_sync_stable();
    let position = selection_row_position(&mut app, "a-choice")?;
    input::set_last_mouse_pos_for_test(Some(position));

    input::handle_event(&mut app, &Event::FocusGained);
    input::set_last_mouse_pos_for_test(None);

    assert_selection_choice(&app, Path::new("/a-choice"));
    assert!(app.project_list.sync().is_changed());
    finish_selection_scan(&mut app, Vec::new());
    assert_selection_choice(&app, Path::new("/a-choice"));
    Ok(())
}

#[test]
fn dismissing_last_row_clears_saved_selection() {
    let path = Path::new("/last-deleted");
    let mut app = test_support::make_app(&[root(path, "last-deleted")]);
    app.project_list
        .at_path_mut(path)
        .expect_found("last Deleted row")
        .visibility = Visibility::Deleted;
    selection_key(&mut app, KeyCode::End);
    assert_selection_choice(&app, path);
    app.project_list.mark_sync_stable();

    selection_key(&mut app, KeyCode::Char('x'));

    assert_eq!(app.project_list.selected_project_path(), None);
    assert_eq!(app.project_list.paths.last_selected, None);
    assert!(app.project_list.sync().is_changed());
    complete_rescan(&mut app, Vec::new());
    assert_eq!(app.project_list.paths.last_selected, None);
}

#[test]
fn unrelated_input_during_rescan_keeps_saved_selection() {
    let mut app = selection_fixture(Visibility::Visible);
    app.project_list
        .at_path_mut(Path::new("/a-choice"))
        .expect_found("retained fallback")
        .visibility = Visibility::Deleted;
    app.prepare_rescan();
    assert_eq!(
        app.project_list.selected_project_path(),
        Some(Path::new("/a-choice")),
    );
    app.project_list.mark_sync_stable();

    selection_key(&mut app, KeyCode::F(12));

    assert_eq!(
        app.project_list.paths.last_selected.as_deref(),
        Some(Path::new("/z-choice")),
    );
    assert!(!app.project_list.sync().is_changed());
    finish_selection_scan(&mut app, vec![root(Path::new("/z-choice"), "z-choice")]);
    assert_selection_choice(&app, Path::new("/z-choice"));
}

#[test]
fn rescan_without_saved_choice_returns_to_first_visible_row() {
    let alpha = Path::new("/alpha");
    let doomed = Path::new("/doomed");
    let mut app = test_support::make_app(&[root(alpha, "alpha"), root(doomed, "doomed")]);
    app.project_list.paths.last_selected = None;
    app.project_list
        .at_path_mut(doomed)
        .expect_found("Deleted project")
        .visibility = Visibility::Deleted;
    assert_eq!(app.project_list.selected_project_path(), Some(alpha));

    app.prepare_rescan();
    finish_selection_scan(&mut app, vec![root(alpha, "alpha")]);

    assert_eq!(app.project_list.selected_project_path(), Some(alpha));
    assert_eq!(app.project_list.paths.last_selected, None);
}

#[test]
fn startup_scan_restores_persisted_selection_without_input() {
    let mut app = test_support::make_app(&selection_roots());
    app.scan.state.phase = ScanPhase::Running;
    let (persisted, expanded) =
        terminal::parse_tree_state_for_test("selected = '/z-choice'\nexpanded = []\n");
    app.project_list.paths.last_selected = persisted;
    app.project_list.paths.pending_expanded = expanded;
    assert_eq!(
        app.project_list.selected_project_path(),
        Some(Path::new("/a-choice")),
    );

    finish_selection_scan(&mut app, selection_roots());

    assert_selection_choice(&app, Path::new("/z-choice"));
}

#[test]
fn rescan_and_quit_without_choice_do_not_persist_fallback() -> anyhow::Result<()> {
    let alpha = Path::new("/alpha");
    let doomed = Path::new("/doomed");
    let mut app = test_support::make_app(&[root(alpha, "alpha"), root(doomed, "doomed")]);
    app.project_list.paths.last_selected = None;
    app.project_list
        .at_path_mut(doomed)
        .expect_found("Deleted project")
        .visibility = Visibility::Deleted;

    input::handle_event(
        &mut app,
        &Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL,
        )),
    );
    let _ = app.handle_bg_msg(BackgroundMsg::ProjectDiscovered {
        item: root(alpha, "alpha"),
    });
    finish_selection_scan(&mut app, vec![root(alpha, "alpha")]);
    selection_key(&mut app, KeyCode::Char('q'));

    assert!(app.framework.quit_requested());
    let serialized = terminal::serialize_tree_state_for_test(&app)?;
    let state: Value = toml::from_str(&serialized)?;
    assert!(state.get("selected").is_none(), "{serialized}");
    assert_eq!(app.project_list.paths.last_selected, None);
    Ok(())
}
