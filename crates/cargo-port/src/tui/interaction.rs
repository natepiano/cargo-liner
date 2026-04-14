use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::app::App;
use super::app::DismissTarget;
use super::app::VisibleRow;
use super::columns;
use super::types::Pane;
use super::types::PaneId;

const DISMISS_SUFFIX: &str = " [x]";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum UiSurface {
    Content,
    Overlay,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum UiRegion {
    Body,
    Action,
}

#[derive(Clone, Debug)]
pub(super) enum UiTarget {
    ProjectRow(VisibleRow),
    SearchRow(usize),
    PaneRow { pane: PaneId, row: usize },
    Dismiss(DismissTarget),
    ToastCard(u64),
}

#[derive(Clone, Debug)]
pub(super) struct UiHitbox {
    pub rect:    Rect,
    pub target:  UiTarget,
    surface:     UiSurface,
    region:      UiRegion,
    order_index: usize,
}

impl UiHitbox {
    const fn new(
        rect: Rect,
        target: UiTarget,
        surface: UiSurface,
        region: UiRegion,
        order_index: usize,
    ) -> Self {
        Self {
            rect,
            target,
            surface,
            region,
            order_index,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ToastHitbox {
    pub id:         u64,
    pub card_rect:  Rect,
    pub close_rect: Rect,
}

pub(super) fn register_project_list_hitboxes(app: &mut App, list_area: Rect, row_width: u16) {
    let visible_height = usize::from(list_area.height);
    let visible_start = app.list_state().offset();
    let project_row_count = if app.is_searching() && !app.search_query().is_empty() {
        app.filtered().len()
    } else {
        app.visible_rows().len()
    };
    let visible_end = project_row_count.min(visible_start.saturating_add(visible_height));
    let suffix_width = u16::try_from(columns::display_width(DISMISS_SUFFIX)).unwrap_or(u16::MAX);

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let y = list_area
            .y
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        let body_rect = Rect::new(list_area.x, y, row_width, 1);
        let body_target = if app.is_searching() && !app.search_query().is_empty() {
            Some(UiTarget::SearchRow(row_index))
        } else {
            app.visible_rows()
                .get(row_index)
                .copied()
                .map(UiTarget::ProjectRow)
        };
        let Some(body_target) = body_target else {
            continue;
        };
        let order_index = app.layout_cache().ui_hitboxes.len();
        app.layout_cache_mut().ui_hitboxes.push(UiHitbox::new(
            body_rect,
            body_target,
            UiSurface::Content,
            UiRegion::Body,
            order_index,
        ));

        let dismiss_target = if app.is_searching() && !app.search_query().is_empty() {
            app.filtered().get(row_index).and_then(|hit| {
                app.is_deleted(hit.abs_path.as_path())
                    .then(|| DismissTarget::DeletedProject(hit.abs_path.clone()))
            })
        } else {
            app.visible_rows()
                .get(row_index)
                .copied()
                .and_then(|row| app.dismiss_target_for_row(row))
        };
        if let Some(target) = dismiss_target {
            let x = list_area
                .x
                .saturating_add(row_width.saturating_sub(suffix_width));
            let action_rect = Rect::new(x, y, suffix_width, 1);
            let order_index = app.layout_cache().ui_hitboxes.len();
            app.layout_cache_mut().ui_hitboxes.push(UiHitbox::new(
                action_rect,
                UiTarget::Dismiss(target),
                UiSurface::Content,
                UiRegion::Action,
                order_index,
            ));
        }
    }
}

pub(super) fn register_toast_hitboxes(app: &mut App, toasts: &[ToastHitbox]) {
    for toast in toasts {
        let body_order = app.layout_cache().ui_hitboxes.len();
        app.layout_cache_mut().ui_hitboxes.push(UiHitbox::new(
            toast.card_rect,
            UiTarget::ToastCard(toast.id),
            UiSurface::Overlay,
            UiRegion::Body,
            body_order,
        ));
        let action_order = app.layout_cache().ui_hitboxes.len();
        app.layout_cache_mut().ui_hitboxes.push(UiHitbox::new(
            toast.close_rect,
            UiTarget::Dismiss(DismissTarget::Toast(toast.id)),
            UiSurface::Overlay,
            UiRegion::Action,
            action_order,
        ));
    }
}

pub(super) fn register_pane_row_hitbox(
    app: &mut App,
    rect: Rect,
    pane: PaneId,
    row: usize,
    surface: UiSurface,
) {
    let order_index = app.layout_cache().ui_hitboxes.len();
    app.layout_cache_mut().ui_hitboxes.push(UiHitbox::new(
        rect,
        UiTarget::PaneRow { pane, row },
        surface,
        UiRegion::Body,
        order_index,
    ));
}

pub(super) fn register_pane_row_hitboxes(
    app: &mut App,
    pane_id: PaneId,
    pane: &Pane,
    surface: UiSurface,
) {
    let area = pane.content_area();
    if area.width == 0 || area.height == 0 || pane.len() == 0 {
        return;
    }

    let visible_height = usize::from(area.height);
    let visible_start = pane.scroll_offset();
    let visible_end = pane.len().min(visible_start.saturating_add(visible_height));

    for (screen_row, row_index) in (visible_start..visible_end).enumerate() {
        let y = area
            .y
            .saturating_add(u16::try_from(screen_row).unwrap_or(u16::MAX));
        let body_rect = Rect::new(area.x, y, area.width, 1);
        register_pane_row_hitbox(app, body_rect, pane_id, row_index, surface);
    }
}

pub(super) fn handle_click(app: &mut App, pos: Position) -> bool {
    let hit = app
        .layout_cache()
        .ui_hitboxes
        .iter()
        .filter(|hitbox| hitbox.rect.contains(pos))
        .max_by(|lhs, rhs| {
            (lhs.surface, lhs.region, lhs.order_index).cmp(&(
                rhs.surface,
                rhs.region,
                rhs.order_index,
            ))
        })
        .cloned();

    let Some(hit) = hit else {
        return false;
    };

    match hit.target {
        UiTarget::ProjectRow(row) => {
            app.focus_pane(PaneId::ProjectList);
            let rows = app.visible_rows();
            if let Some(index) = rows.iter().position(|candidate| *candidate == row) {
                app.list_state_mut().select(Some(index));
            }
            true
        },
        UiTarget::SearchRow(index) => {
            app.focus_pane(PaneId::ProjectList);
            if index < app.filtered().len() {
                app.list_state_mut().select(Some(index));
            }
            true
        },
        UiTarget::PaneRow { pane, row } => {
            app.focus_pane(pane);
            match pane {
                PaneId::Finder => app.finder_mut().pane.set_pos(row),
                PaneId::Settings => app.settings_pane_mut().set_pos(row),
                PaneId::Package => app.package_pane_mut().set_pos(row),
                PaneId::Git => app.git_pane_mut().set_pos(row),
                PaneId::Targets => app.targets_pane_mut().set_pos(row),
                PaneId::Lints => app.lint_pane_mut().set_pos(row),
                PaneId::CiRuns => app.ci_pane_mut().set_pos(row),
                _ => return false,
            }
            true
        },
        UiTarget::Dismiss(target) => {
            app.dismiss(target);
            true
        },
        UiTarget::ToastCard(id) => {
            let active = app.active_toasts();
            if let Some(index) = active.iter().position(|toast| toast.id() == id) {
                app.toast_pane_mut().set_pos(index);
                app.focus_pane(PaneId::Toasts);
            }
            true
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::OnceLock;
    use std::sync::mpsc;
    use std::time::Duration;
    use std::time::Instant;

    use crossterm::event::Event;
    use crossterm::event::KeyModifiers;
    use crossterm::event::MouseButton;
    use crossterm::event::MouseEvent;
    use crossterm::event::MouseEventKind;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::UiTarget;
    use crate::ci::CiJob;
    use crate::ci::CiRun;
    use crate::ci::Conclusion;
    use crate::ci::FetchStatus;
    use crate::config::CargoPortConfig;
    use crate::http::HttpClient;
    use crate::lint::LintCommand;
    use crate::lint::LintCommandStatus;
    use crate::lint::LintRun;
    use crate::lint::LintRunStatus;
    use crate::project::AbsolutePath;
    use crate::project::Cargo;
    use crate::project::ExampleGroup;
    use crate::project::GitInfo;
    use crate::project::GitOrigin;
    use crate::project::GitPathState;
    use crate::project::GitState;
    use crate::project::PackageProject;
    use crate::project::ProjectType;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::Visibility;
    use crate::project::WorkflowPresence;
    use crate::project::WorktreeGroup;
    use crate::project_list::ProjectList;
    use crate::tui::app::App;
    use crate::tui::app::DismissTarget;
    use crate::tui::app::ExpandKey;
    use crate::tui::app::SearchHit;
    use crate::tui::app::SearchMode;
    use crate::tui::detail;
    use crate::tui::finder;
    use crate::tui::input;
    use crate::tui::render;
    use crate::tui::settings::SettingOption;
    use crate::tui::toasts::ToastStyle;
    use crate::tui::types::Pane;
    use crate::tui::types::PaneId;

    fn test_http_client() -> HttpClient {
        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let rt = TEST_RT.get_or_init(|| {
            tokio::runtime::Runtime::new().unwrap_or_else(|_| std::process::abort())
        });
        HttpClient::new(rt.handle().clone()).unwrap_or_else(|| std::process::abort())
    }

    fn make_package(name: &str, path: &Path) -> RootItem {
        make_package_with_cargo(
            name,
            path,
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
        )
    }

    fn make_package_with_cargo(name: &str, path: &Path, cargo: Cargo) -> RootItem {
        RootItem::Rust(RustProject::Package(PackageProject::new(
            AbsolutePath::from(path),
            Some(name.to_string()),
            cargo,
            Vec::new(),
            None,
            None,
        )))
    }

    fn make_package_worktree(
        name: &str,
        path: &Path,
        worktree_name: Option<&str>,
        primary_abs_path: Option<&Path>,
    ) -> PackageProject {
        PackageProject::new(
            AbsolutePath::from(path),
            Some(name.to_string()),
            Cargo::new(None, None, Vec::new(), Vec::new(), Vec::new(), 0),
            Vec::new(),
            worktree_name.map(str::to_string),
            primary_abs_path.map(AbsolutePath::from),
        )
    }

    fn make_git_info(url: Option<&str>) -> GitInfo {
        GitInfo {
            path_state:          GitPathState::default(),
            origin:              GitOrigin::Clone,
            branch:              Some("main".to_string()),
            owner:               Some("natepiano".to_string()),
            url:                 url.map(str::to_string),
            first_commit:        Some("2024-01-01T00:00:00Z".to_string()),
            last_commit:         Some("2024-01-02T00:00:00Z".to_string()),
            ahead_behind:        Some((0, 0)),
            upstream_branch:     Some("origin/main".to_string()),
            default_branch:      Some("main".to_string()),
            ahead_behind_origin: Some((0, 0)),
            local_main_branch:   Some("main".to_string()),
            ahead_behind_local:  Some((0, 0)),
            workflows:           WorkflowPresence::Present,
        }
    }

    fn make_ci_run(run_id: u64, conclusion: Conclusion) -> CiRun {
        CiRun {
            run_id,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            branch: "main".to_string(),
            url: format!("https://github.com/natepiano/demo/actions/runs/{run_id}"),
            conclusion,
            jobs: vec![CiJob {
                name: "build".to_string(),
                conclusion,
                duration: "1m".to_string(),
                duration_secs: Some(60),
            }],
            wall_clock_secs: Some(60),
            commit_title: Some("commit".to_string()),
            updated_at: None,
            fetched: FetchStatus::Fetched,
        }
    }

    fn make_lint_run(run_id: &str, status: LintRunStatus) -> LintRun {
        LintRun {
            run_id: run_id.to_string(),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            finished_at: Some("2024-01-01T00:01:00Z".to_string()),
            duration_ms: Some(60_000),
            status,
            commands: vec![LintCommand {
                name:        "clippy".to_string(),
                command:     "cargo clippy".to_string(),
                status:      LintCommandStatus::Passed,
                duration_ms: Some(1_000),
                exit_code:   Some(0),
                log_file:    "clippy.log".to_string(),
            }],
        }
    }

    fn make_app(projects: &[RootItem]) -> App {
        let mut cfg = CargoPortConfig::default();
        cfg.tui.include_dirs = vec!["/tmp/test".to_string()];
        let (bg_tx, bg_rx) = mpsc::channel();
        let mut app = App::new(
            projects,
            bg_tx,
            bg_rx,
            &cfg,
            test_http_client(),
            Instant::now(),
        );
        app.sync_selected_project();
        app
    }

    fn render_ui(app: &mut App) {
        app.ensure_visible_rows_cached();
        app.ensure_detail_cached();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        terminal
            .draw(|frame| render::ui(frame, app))
            .unwrap_or_else(|_| std::process::abort());
    }

    fn render_lints_panel(app: &mut App, runs: &[LintRun]) {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        terminal
            .draw(|frame| detail::render_lints_panel(frame, app, runs, frame.area()))
            .unwrap_or_else(|_| std::process::abort());
    }

    fn render_ci_panel(app: &mut App, runs: &[CiRun]) {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        terminal
            .draw(|frame| detail::render_ci_panel(frame, app, runs, frame.area()))
            .unwrap_or_else(|_| std::process::abort());
    }

    fn click(app: &mut App, column: u16, row: u16) {
        input::handle_event(
            app,
            &Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }),
        );
    }

    fn row_body_point(app: &App, row_index: usize) -> (u16, u16) {
        let area = app.layout_cache().project_list;
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
        )
    }

    fn row_dismiss_point(app: &App, row_index: usize) -> (u16, u16) {
        let area = app.layout_cache().project_list;
        (
            area.x.saturating_add(area.width.saturating_sub(2)),
            area.y
                .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
        )
    }

    fn pane_row_point(pane: &Pane, row_index: usize) -> (u16, u16) {
        let area = pane.content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
        )
    }

    fn finder_result_point(app: &App, result_index: usize) -> (u16, u16) {
        let area = app.finder().pane.content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(1)
                .saturating_add(u16::try_from(result_index).unwrap_or(u16::MAX)),
        )
    }

    fn lint_run_point(app: &App, run_index: usize) -> (u16, u16) {
        let area = app.lint_pane().content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(1)
                .saturating_add(u16::try_from(run_index).unwrap_or(u16::MAX)),
        )
    }

    fn ci_run_point(app: &App, run_index: usize) -> (u16, u16) {
        let area = app.ci_pane().content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(1)
                .saturating_add(u16::try_from(run_index).unwrap_or(u16::MAX)),
        )
    }

    fn toast_close_point(app: &App, toast_id: u64) -> (u16, u16) {
        let rect = app
            .layout_cache()
            .ui_hitboxes
            .iter()
            .find_map(|hitbox| match hitbox.target {
                UiTarget::Dismiss(DismissTarget::Toast(id)) if id == toast_id => Some(hitbox.rect),
                _ => None,
            })
            .unwrap_or_else(|| std::process::abort());
        (
            rect.x.saturating_add(rect.width.saturating_sub(1) / 2),
            rect.y.saturating_add(rect.height.saturating_sub(1) / 2),
        )
    }

    fn toast_body_point(app: &App, toast_id: u64) -> (u16, u16) {
        let rect = app
            .layout_cache()
            .ui_hitboxes
            .iter()
            .find_map(|hitbox| match hitbox.target {
                UiTarget::ToastCard(id) if id == toast_id => Some(hitbox.rect),
                _ => None,
            })
            .unwrap_or_else(|| std::process::abort());
        (
            rect.x.saturating_add(rect.width.saturating_sub(1) / 2),
            rect.y.saturating_add(rect.height.saturating_sub(1) / 2),
        )
    }

    fn mark_deleted(app: &mut App, path: &Path) {
        let project = app
            .projects_mut()
            .at_path_mut(path)
            .unwrap_or_else(|| std::process::abort());
        project.disk_usage_bytes = Some(0);
        project.visibility = Visibility::Deleted;
        app.dirty_mut().rows.mark_dirty();
        app.dirty_mut().fit_widths.mark_dirty();
        app.dirty_mut().disk_cache.mark_dirty();
    }

    #[test]
    fn deleted_project_row_mouse_click_dismisses_it() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let deleted_dir = tmp.path().join("deleted");
        std::fs::create_dir_all(&deleted_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("deleted", &deleted_dir)]);
        mark_deleted(&mut app, &deleted_dir);
        render_ui(&mut app);

        let (x, y) = row_dismiss_point(&app, 0);
        click(&mut app, x, y);
        render_ui(&mut app);

        assert!(
            app.visible_rows().is_empty(),
            "clicking deleted row [x] should stop rendering that row"
        );
    }

    #[test]
    fn mouse_and_keyboard_dismiss_resolve_same_deleted_project_target() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let deleted_dir = tmp.path().join("deleted");
        std::fs::create_dir_all(&deleted_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("deleted", &deleted_dir)]);
        mark_deleted(&mut app, &deleted_dir);
        app.list_state_mut().select(Some(0));
        render_ui(&mut app);

        let keyboard_target = app
            .focused_dismiss_target()
            .unwrap_or_else(|| std::process::abort());
        let mouse_target = app
            .layout_cache()
            .ui_hitboxes
            .iter()
            .find_map(|hitbox| match &hitbox.target {
                UiTarget::Dismiss(DismissTarget::DeletedProject(path)) if path == &deleted_dir => {
                    Some(DismissTarget::DeletedProject(path.clone()))
                },
                _ => None,
            })
            .unwrap_or_else(|| std::process::abort());

        let DismissTarget::DeletedProject(lhs) = keyboard_target else {
            std::process::abort();
        };
        let DismissTarget::DeletedProject(rhs) = mouse_target else {
            std::process::abort();
        };
        assert_eq!(lhs, rhs);
    }

    #[test]
    fn row_body_click_selects_clicked_project() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let first = tmp.path().join("first");
        let second = tmp.path().join("second");
        std::fs::create_dir_all(&first).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&second).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[
            make_package("first", &first),
            make_package("second", &second),
        ]);
        render_ui(&mut app);

        let (x, y) = row_body_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::ProjectList);
        assert_eq!(app.list_state().selected(), Some(1));
        assert_eq!(
            app.selected_project_path().map(Path::to_path_buf),
            Some(second),
        );
    }

    #[test]
    fn search_result_row_click_selects_correct_hit() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let alpha = tmp.path().join("alpha");
        let beta = tmp.path().join("beta");
        std::fs::create_dir_all(&alpha).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&beta).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("alpha", &alpha), make_package("beta", &beta)]);
        app.ui_modes_mut().search = SearchMode::Active;
        app.set_focused_pane(PaneId::Search);
        *app.search_query_mut() = "beta".to_string();
        *app.filtered_mut() = vec![SearchHit {
            abs_path:     beta.clone().into(),
            display_path: beta.display().to_string(),
            name:         "beta".to_string(),
            score:        100,
            is_rust:      true,
        }];
        render_ui(&mut app);

        let (x, y) = row_body_point(&app, 0);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::ProjectList);
        assert_eq!(app.list_state().selected(), Some(0));
        assert_eq!(
            app.selected_project_path().map(Path::to_path_buf),
            Some(beta),
        );
    }

    #[test]
    fn finder_row_click_uses_result_index_not_visual_table_row() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let alpha = tmp.path().join("alpha");
        let beta = tmp.path().join("beta");
        std::fs::create_dir_all(&alpha).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&beta).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("alpha", &alpha), make_package("beta", &beta)]);
        let (index, col_widths) = finder::build_finder_index(app.projects());
        let finder = app.finder_mut();
        finder.index = index;
        finder.col_widths = col_widths;
        finder.results = vec![0, 1];
        finder.total = 2;
        app.open_overlay(PaneId::Finder);
        app.open_finder();
        render_ui(&mut app);

        let (x, y) = finder_result_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(
            app.finder().pane.pos(),
            1,
            "clicking the second rendered finder result should select result index 1, not the header-offset visual row"
        );
    }

    #[test]
    fn settings_row_click_uses_setting_index_not_visual_line() {
        let mut app = make_app(&[]);
        app.open_overlay(PaneId::Settings);
        app.open_settings();
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.settings_pane(), 5);
        click(&mut app, x, y);

        assert_eq!(
            app.settings_pane().pos(),
            SettingOption::CiRunCount as usize,
            "clicking a rendered settings option should select the logical setting, not the visual line index including spacer/header rows"
        );
    }

    #[test]
    fn lint_row_click_uses_run_index_not_header_row() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("demo", &project_dir)]);
        let runs = vec![
            make_lint_run("run-1", LintRunStatus::Passed),
            make_lint_run("run-2", LintRunStatus::Failed),
        ];
        render_lints_panel(&mut app, &runs);

        let (x, y) = lint_run_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(
            app.lint_pane().pos(),
            1,
            "clicking the second rendered lint run should select run index 1, not the header-offset visual row"
        );
    }

    #[test]
    fn ci_row_click_uses_run_index_not_header_row() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package_with_cargo(
            "demo",
            &project_dir,
            Cargo::new(
                None,
                None,
                vec![ProjectType::Binary],
                vec![ExampleGroup {
                    category: String::new(),
                    names:    vec!["example".to_string()],
                }],
                Vec::new(),
                0,
            ),
        )]);
        let runs = vec![
            make_ci_run(1, Conclusion::Success),
            make_ci_run(2, Conclusion::Failure),
        ];
        render_ci_panel(&mut app, &runs);

        let (x, y) = ci_run_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(
            app.ci_pane().pos(),
            1,
            "clicking the second rendered CI run should select run index 1, not the header-offset visual row"
        );
    }

    #[test]
    fn expanded_tree_reshape_rebuilds_clickable_rows() {
        let primary: AbsolutePath = "/abs/app".into();
        let linked: AbsolutePath = "/abs/app_feat".into();
        let mut app = make_app(&[RootItem::Rust(RustProject::Package(make_package_worktree(
            "app",
            &primary,
            None,
            Some(primary.as_path()),
        )))]);
        app.expanded_mut().insert(ExpandKey::Node(0));
        render_ui(&mut app);

        app.set_projects(ProjectList::new(vec![RootItem::Worktrees(
            WorktreeGroup::new_packages(
                make_package_worktree("app", &primary, None, Some(primary.as_path())),
                vec![make_package_worktree(
                    "app",
                    &linked,
                    Some("app_feat"),
                    Some(primary.as_path()),
                )],
            ),
        )]));
        app.dirty_mut().rows.mark_dirty();
        app.dirty_mut().fit_widths.mark_dirty();
        app.dirty_mut().disk_cache.mark_dirty();
        render_ui(&mut app);

        let (x, y) = row_body_point(&app, 2);
        click(&mut app, x, y);

        assert_eq!(
            app.selected_project_path(),
            Some(linked.as_path()),
            "clicking the linked worktree row after regroup should select it"
        );
    }

    #[test]
    fn old_dismiss_click_location_does_not_dismiss_surviving_row_after_rerender() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let deleted_dir = tmp.path().join("deleted");
        let live_dir = tmp.path().join("live");
        std::fs::create_dir_all(&deleted_dir).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&live_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[
            make_package("deleted", &deleted_dir),
            make_package("live", &live_dir),
        ]);
        mark_deleted(&mut app, &deleted_dir);
        render_ui(&mut app);
        let stale_click = row_dismiss_point(&app, 0);

        app.list_state_mut().select(Some(0));
        let target = app
            .focused_dismiss_target()
            .unwrap_or_else(|| std::process::abort());
        app.dismiss(target);
        render_ui(&mut app);

        click(&mut app, stale_click.0, stale_click.1);
        render_ui(&mut app);

        assert!(
            app.projects()
                .at_path(&live_dir)
                .is_some_and(|info| info.visibility == Visibility::Visible),
            "clicking the old dismiss location after rerender must not dismiss the surviving row"
        );
        assert_eq!(
            app.selected_project_path().map(Path::to_path_buf),
            Some(live_dir),
            "the surviving row may be selected, but it must not be dismissed by stale geometry"
        );
    }

    #[test]
    fn toast_close_click_dismisses_toast() {
        let mut app = make_app(&[]);
        let toast_id =
            app.toasts_mut()
                .push_persistent("Error", "toast body", ToastStyle::Error, None, 1);
        let toast_len = app.active_toasts().len();
        app.toast_pane_mut().set_len(toast_len);
        render_ui(&mut app);

        let (x, y) = toast_close_point(&app, toast_id);
        click(&mut app, x, y);
        let after_exit = Instant::now() + Duration::from_secs(1);
        app.toasts_mut().prune(after_exit);

        assert!(
            app.toasts_mut()
                .active(after_exit)
                .into_iter()
                .all(|toast| toast.id() != toast_id),
            "clicking the toast close affordance should start dismissal and let the toast exit"
        );
    }

    #[test]
    fn toast_body_click_focuses_toast_over_underlying_content() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("demo", &project_dir)]);
        let toast_id =
            app.toasts_mut()
                .push_persistent("Error", "toast body", ToastStyle::Error, None, 1);
        let toast_len = app.active_toasts().len();
        app.toast_pane_mut().set_len(toast_len);
        render_ui(&mut app);

        let (x, y) = toast_body_point(&app, toast_id);
        click(&mut app, x, y);

        assert_eq!(
            app.focused_pane(),
            PaneId::Toasts,
            "toast body click should focus the toast surface over underlying content"
        );
    }

    #[test]
    fn finder_row_click_selects_result() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let alpha = tmp.path().join("alpha");
        let beta = tmp.path().join("beta");
        std::fs::create_dir_all(&alpha).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&beta).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("alpha", &alpha), make_package("beta", &beta)]);
        let (index, col_widths) = finder::build_finder_index(app.projects());
        let finder = app.finder_mut();
        finder.index = index;
        finder.col_widths = col_widths;
        finder.query = "a".to_string();
        finder.results = vec![0, 1];
        finder.total = 2;
        app.open_overlay(PaneId::Finder);
        app.open_finder();
        render_ui(&mut app);

        let (x, y) = finder_result_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(app.finder().pane.pos(), 1);
    }

    #[test]
    fn settings_row_click_selects_setting() {
        let mut app = make_app(&[]);
        app.open_overlay(PaneId::Settings);
        app.open_settings();
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.settings_pane(), 2);
        click(&mut app, x, y);

        assert_eq!(
            app.settings_pane().pos(),
            SettingOption::InvertScroll as usize
        );
    }

    #[test]
    fn package_pane_row_click_selects_field() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("demo", &project_dir)]);
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.package_pane(), 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::Package);
        assert_eq!(app.package_pane().pos(), 1);
    }

    #[test]
    fn targets_pane_row_click_selects_target() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let cargo = Cargo::new(
            None,
            None,
            vec![ProjectType::Binary],
            vec![ExampleGroup {
                category: String::new(),
                names:    vec!["example_a".to_string(), "example_b".to_string()],
            }],
            Vec::new(),
            0,
        );
        let mut app = make_app(&[make_package_with_cargo("demo", &project_dir, cargo)]);
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.targets_pane(), 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::Targets);
        assert_eq!(app.targets_pane().pos(), 1);
    }

    #[test]
    fn git_pane_row_click_selects_field() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("demo", &project_dir)]);
        app.projects_mut()
            .at_path_mut(&project_dir)
            .unwrap_or_else(|| std::process::abort())
            .git_state = GitState::Detected(Box::new(make_git_info(Some(
            "https://github.com/natepiano/demo",
        ))));
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.git_pane(), 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::Git);
        assert_eq!(app.git_pane().pos(), 1);
    }
}
