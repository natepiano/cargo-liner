use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::app::App;
use super::app::HoveredPaneRow;
use super::pane::HITTABLE_Z_ORDER;
use super::pane::Hittable;
use super::pane::HittableId;
use super::pane::HoverTarget;
use super::pane::Viewport;
use super::panes::PaneId;

/// Per-toast hit-test rects produced by `toasts::render_toasts`
/// and stashed onto `ToastsPane` each frame. The `Hittable` impl
/// on `ToastsPane` walks the list directly.
#[derive(Clone, Copy, Debug)]
pub struct ToastHitbox {
    pub id:         u64,
    pub card_rect:  Rect,
    pub close_rect: Rect,
}

pub(super) fn handle_click(app: &mut App, pos: Position) -> bool {
    let Some(hit) = hit_test_at(app, pos) else {
        return false;
    };
    match hit {
        HoverTarget::PaneRow { pane, row } => {
            app.focus_mut().set(pane);
            if pane == PaneId::ProjectList {
                app.projects_mut().set_cursor(row);
            } else {
                set_pane_pos(app, pane, row);
            }
            true
        },
        HoverTarget::Dismiss(target) => {
            app.dismiss(target);
            true
        },
        HoverTarget::ToastCard(id) => {
            let active = app.toasts().active_now();
            if let Some(index) = active.iter().position(|toast| toast.id() == id) {
                app.toasts_mut().viewport_mut().set_pos(index);
                app.focus_mut().set(PaneId::Toasts);
            }
            true
        },
    }
}

pub(super) fn hovered_pane_row_at(app: &App, pos: Position) -> Option<HoveredPaneRow> {
    match hit_test_at(app, pos)? {
        HoverTarget::PaneRow { pane, row } => Some(HoveredPaneRow { pane, row }),
        HoverTarget::Dismiss(_) | HoverTarget::ToastCard(_) => None,
    }
}

/// Walk `HITTABLE_Z_ORDER` top-to-bottom and return the first pane's
/// `hit_test_at` answer. Phase 13 relocation: lives at App-level so
/// per-arm reach can resolve to whichever owner holds the pane (Panes
/// for survivors; subsystems on App after Phases 14–17).
pub(super) fn hit_test_at(app: &App, pos: Position) -> Option<HoverTarget> {
    for id in HITTABLE_Z_ORDER {
        let pane: &dyn Hittable = match id {
            HittableId::Toasts => app.toasts(),
            HittableId::Finder => app.overlays().finder_pane(),
            HittableId::Settings => app.overlays().settings_pane(),
            HittableId::Keymap => app.overlays().keymap_pane(),
            HittableId::ProjectList => app.panes().project_list(),
            HittableId::Package => app.panes().package(),
            HittableId::Lang => app.panes().lang(),
            HittableId::Cpu => app.panes().cpu(),
            HittableId::Git => app.panes().git(),
            HittableId::Targets => app.panes().targets(),
            HittableId::Lints => app.lint(),
            HittableId::CiRuns => app.ci(),
        };
        if let Some(hit) = pane.hit_test_at(pos) {
            return Some(hit);
        }
    }
    None
}

/// Set the cursor position for `id`'s viewport. Phase 13 relocation:
/// matches by `PaneId` to whichever owner holds the target viewport.
/// `ProjectList`'s cursor lives on `Selection.cursor`; callers route
/// through `app.projects_mut().set_cursor(row)`, not this fn.
pub(super) fn set_pane_pos(app: &mut App, id: PaneId, row: usize) {
    if id == PaneId::ProjectList {
        return;
    }
    viewport_mut_for(app, id).set_pos(row);
}

/// Mutable viewport accessor by `PaneId`. Phase 13 relocation —
/// per-arm reach swaps to subsystem owners as Phases 14–17 absorb
/// wrappers.
pub(super) const fn viewport_mut_for(app: &mut App, id: PaneId) -> &mut Viewport {
    match id {
        PaneId::Toasts => app.toasts_mut().viewport_mut(),
        PaneId::Cpu => app.panes_mut().cpu_mut().viewport_mut(),
        PaneId::Lang => app.panes_mut().lang_mut().viewport_mut(),
        PaneId::Lints => app.lint_mut().viewport_mut(),
        PaneId::CiRuns => app.ci_mut().viewport_mut(),
        PaneId::Package => app.panes_mut().package_mut().viewport_mut(),
        PaneId::Git => app.panes_mut().git_mut().viewport_mut(),
        PaneId::Keymap => app.overlays_mut().keymap_pane_mut().viewport_mut(),
        PaneId::Settings => app.overlays_mut().settings_pane_mut().viewport_mut(),
        PaneId::Finder => app.overlays_mut().finder_pane_mut().viewport_mut(),
        PaneId::Output => app.panes_mut().output_mut().viewport_mut(),
        PaneId::Targets => app.panes_mut().targets_mut().viewport_mut(),
        PaneId::ProjectList => app.panes_mut().project_list_mut().viewport_mut(),
    }
}

/// Push the current `hovered_pane_row` into the per-pane viewports.
/// Clears any prior hover across every pane first, then sets the row
/// on the pane indicated by `hovered_pane_row` (if any).
pub(super) const fn apply_hovered_pane_row(app: &mut App) {
    clear_all_hover(app);
    if let Some(hovered) = app.panes().hovered_row() {
        viewport_mut_for(app, hovered.pane).set_hovered(Some(hovered.row));
    }
}

const fn clear_all_hover(app: &mut App) {
    app.toasts_mut().viewport_mut().set_hovered(None);
    app.ci_mut().viewport_mut().set_hovered(None);
    app.lint_mut().viewport_mut().set_hovered(None);
    app.overlays_mut()
        .keymap_pane_mut()
        .viewport_mut()
        .set_hovered(None);
    app.overlays_mut()
        .settings_pane_mut()
        .viewport_mut()
        .set_hovered(None);
    app.overlays_mut()
        .finder_pane_mut()
        .viewport_mut()
        .set_hovered(None);
    let panes = app.panes_mut();
    panes.package_mut().viewport_mut().set_hovered(None);
    panes.lang_mut().viewport_mut().set_hovered(None);
    panes.cpu_mut().viewport_mut().set_hovered(None);
    panes.git_mut().viewport_mut().set_hovered(None);
    panes.output_mut().viewport_mut().set_hovered(None);
    panes.targets_mut().viewport_mut().set_hovered(None);
    panes.project_list_mut().viewport_mut().set_hovered(None);
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::time::Duration;
    use std::time::Instant;

    use crossterm::event::Event;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyEventKind;
    use crossterm::event::KeyModifiers;
    use crossterm::event::MouseButton;
    use crossterm::event::MouseEvent;
    use crossterm::event::MouseEventKind;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;

    use super::HoveredPaneRow;
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
    use crate::project::CheckoutInfo;
    use crate::project::ExampleGroup;
    use crate::project::FileStamp;
    use crate::project::GitStatus;
    use crate::project::ManifestFingerprint;
    use crate::project::MemberGroup;
    use crate::project::Package;
    use crate::project::PackageRecord;
    use crate::project::ProjectType;
    use crate::project::PublishPolicy;
    use crate::project::RemoteInfo;
    use crate::project::RemoteKind;
    use crate::project::RepoInfo;
    use crate::project::RootItem;
    use crate::project::RustInfo;
    use crate::project::RustProject;
    use crate::project::TargetRecord;
    use crate::project::Visibility;
    use crate::project::WorkflowPresence;
    use crate::project::Workspace;
    use crate::project::WorkspaceMetadata;
    use crate::project::WorktreeGroup;
    use crate::project::WorktreeStatus;
    use crate::scan::BackgroundMsg;
    use crate::scan::DirSizes;
    use crate::tui::app::App;
    use crate::tui::app::ConfirmAction;
    use crate::tui::app::DismissTarget;
    use crate::tui::app::ExpandKey;
    use crate::tui::finder;
    use crate::tui::input;
    use crate::tui::pane::HoverTarget;
    use crate::tui::pane::PaneRenderCtx;
    use crate::tui::pane::PaneSelectionState;
    use crate::tui::pane::Viewport;
    use crate::tui::panes;
    use crate::tui::panes::LintsData;
    use crate::tui::panes::PaneId;
    use crate::tui::project_list::ProjectList;
    use crate::tui::render;
    use crate::tui::settings::SettingOption;
    use crate::tui::toasts::ToastStyle;

    fn test_http_client() -> HttpClient {
        let rt = crate::test_support::test_runtime();
        HttpClient::new(rt.handle().clone()).unwrap_or_else(|| std::process::abort())
    }

    fn make_package(name: &str, path: &Path) -> RootItem {
        make_package_with_cargo(name, path, Cargo::default())
    }

    fn make_package_with_cargo(name: &str, path: &Path, cargo: Cargo) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(path),
            name: Some(name.to_string()),
            rust: RustInfo {
                cargo,
                ..RustInfo::default()
            },
            ..Package::default()
        }))
    }

    fn make_package_worktree(
        name: &str,
        path: &Path,
        is_linked_worktree: bool,
        primary_abs_path: Option<&Path>,
    ) -> Package {
        let worktree_status = match (is_linked_worktree, primary_abs_path) {
            (true, Some(p)) => WorktreeStatus::Linked {
                primary: AbsolutePath::from(p),
            },
            (false, Some(p)) => WorktreeStatus::Primary {
                root: AbsolutePath::from(p),
            },
            _ => WorktreeStatus::NotGit,
        };
        Package {
            path: AbsolutePath::from(path),
            name: Some(name.to_string()),
            worktree_status,
            ..Package::default()
        }
    }

    fn inline_group(members: Vec<Package>) -> MemberGroup { MemberGroup::Inline { members } }

    fn make_member(name: &str, path: &Path) -> Package {
        Package {
            path: AbsolutePath::from(path),
            name: Some(name.to_string()),
            ..Package::default()
        }
    }

    fn make_workspace_with_members(name: &str, path: &Path, groups: Vec<MemberGroup>) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: AbsolutePath::from(path),
            name: Some(name.to_string()),
            groups,
            ..Workspace::default()
        }))
    }

    fn make_git_info(url: Option<&str>) -> (CheckoutInfo, RepoInfo) {
        let checkout = CheckoutInfo {
            status:              GitStatus::Clean,
            branch:              Some("main".to_string()),
            last_commit:         Some("2024-01-02T00:00:00Z".to_string()),
            ahead_behind_local:  Some((0, 0)),
            primary_tracked_ref: Some("origin/main".to_string()),
        };
        let repo = RepoInfo {
            remotes:           vec![RemoteInfo {
                name:         "origin".to_string(),
                url:          url.map(str::to_string),
                owner:        Some("natepiano".to_string()),
                repo:         Some("demo".to_string()),
                tracked_ref:  Some("origin/main".to_string()),
                ahead_behind: Some((0, 0)),
                kind:         RemoteKind::Clone,
            }],
            workflows:         WorkflowPresence::Present,
            first_commit:      Some("2024-01-01T00:00:00Z".to_string()),
            last_fetched:      None,
            default_branch:    Some("main".to_string()),
            local_main_branch: Some("main".to_string()),
        };
        (checkout, repo)
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
        let metadata_store = Arc::new(Mutex::new(crate::project::WorkspaceMetadataStore::new()));
        let mut app = App::new(
            projects,
            bg_tx,
            bg_rx,
            &cfg,
            test_http_client(),
            Instant::now(),
            metadata_store,
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
        app.ensure_detail_cached();
        app.lint_mut().set_content(LintsData {
            runs:    runs.to_vec(),
            sizes:   vec![Some(0); runs.len()],
            is_rust: true,
        });
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        let focus_state = app.focus().pane_state(PaneId::Lints);
        let is_focused = app.focus().is(PaneId::Lints);
        let animation_elapsed = app.animation_elapsed();
        let selected_path = app
            .selected_project_path_for_render()
            .map(std::path::Path::to_path_buf);
        terminal
            .draw(|frame| {
                let area = frame.area();
                let (lint, config, projects) = app.split_lint_for_render();
                let ctx = PaneRenderCtx {
                    focus_state,
                    is_focused,
                    animation_elapsed,
                    config,
                    project_list: projects,
                    selected_project_path: selected_path.as_deref(),
                };
                panes::render_lints_pane_body(frame, area, lint, &ctx);
            })
            .unwrap_or_else(|_| std::process::abort());
    }

    fn render_ci_panel(app: &mut App, runs: &[CiRun]) {
        app.ensure_detail_cached();
        app.ci_mut().override_runs_for_test(runs.to_vec());
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        let focus_state = app.focus().pane_state(PaneId::CiRuns);
        let is_focused = app.focus().is(PaneId::CiRuns);
        let animation_elapsed = app.animation_elapsed();
        let selected_path = app
            .selected_project_path_for_render()
            .map(std::path::Path::to_path_buf);
        terminal
            .draw(|frame| {
                let area = frame.area();
                let (ci, config, projects) = app.split_ci_for_render();
                let ctx = PaneRenderCtx {
                    focus_state,
                    is_focused,
                    animation_elapsed,
                    config,
                    project_list: projects,
                    selected_project_path: selected_path.as_deref(),
                };
                panes::render_ci_pane_body(frame, area, ci, &ctx);
            })
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

    fn move_mouse(app: &mut App, column: u16, row: u16) {
        input::handle_event(
            app,
            &Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }),
        );
    }

    fn press_key(app: &mut App, code: KeyCode) {
        input::handle_event(
            app,
            &Event::Key(KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            }),
        );
    }

    fn focus_gained(app: &mut App) { input::handle_event(app, &Event::FocusGained); }

    fn row_body_point(app: &App, row_index: usize) -> (u16, u16) {
        let area = app.layout_cache.project_list_body;
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
        )
    }

    fn row_dismiss_point(app: &App, row_index: usize) -> (u16, u16) {
        let area = app.layout_cache.project_list_body;
        (
            area.x.saturating_add(area.width.saturating_sub(2)),
            area.y
                .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
        )
    }

    fn pane_row_point(pane: &Viewport, row_index: usize) -> (u16, u16) {
        let area = pane.content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
        )
    }

    fn finder_result_point(app: &App, result_index: usize) -> (u16, u16) {
        let area = app.overlays().finder_pane().viewport().content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(1)
                .saturating_add(u16::try_from(result_index).unwrap_or(u16::MAX)),
        )
    }

    fn lint_run_point(app: &App, run_index: usize) -> (u16, u16) {
        let area = app.lint().viewport().content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(1)
                .saturating_add(u16::try_from(run_index).unwrap_or(u16::MAX)),
        )
    }

    fn ci_run_point(app: &App, run_index: usize) -> (u16, u16) {
        let area = app.ci().viewport().content_area();
        (
            area.x.saturating_add(1),
            area.y
                .saturating_add(1)
                .saturating_add(u16::try_from(run_index).unwrap_or(u16::MAX)),
        )
    }

    fn toast_close_point(app: &App, toast_id: u64) -> (u16, u16) {
        let Some(rect) = app
            .toasts()
            .hits()
            .iter()
            .find(|h| h.id == toast_id)
            .map(|h| h.close_rect)
        else {
            std::process::abort();
        };
        (
            rect.x.saturating_add(rect.width.saturating_sub(1) / 2),
            rect.y.saturating_add(rect.height.saturating_sub(1) / 2),
        )
    }

    fn toast_body_point(app: &App, toast_id: u64) -> (u16, u16) {
        let Some(rect) = app
            .toasts()
            .hits()
            .iter()
            .find(|h| h.id == toast_id)
            .map(|h| h.card_rect)
        else {
            std::process::abort();
        };
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
        app.projects_mut().set_cursor(0);
        render_ui(&mut app);

        let keyboard_target = app
            .focused_dismiss_target()
            .unwrap_or_else(|| std::process::abort());
        let (x, y) = row_dismiss_point(&app, 0);
        let Some(hit) = super::hit_test_at(&app, Position::new(x, y)) else {
            std::process::abort();
        };
        let HoverTarget::Dismiss(mouse_target) = hit else {
            std::process::abort();
        };

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
        assert_eq!(app.projects().cursor(), 1);
        assert_eq!(
            app.selected_project_path().map(Path::to_path_buf),
            Some(second),
        );
    }

    // The "overlay surface beats content surface" priority is now
    // encoded by the order of `HITTABLE_Z_ORDER` in
    // `panes::dispatch`. The strum-backed
    // `z_order_covers_every_hittable_id` test pins coverage; the
    // ordering itself is enforced by the literal constant value.

    #[test]
    fn hovered_pane_row_resolves_project_list_rows() {
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
        assert_eq!(
            super::hovered_pane_row_at(&app, Position::new(x, y)),
            Some(HoveredPaneRow {
                pane: PaneId::ProjectList,
                row:  1,
            }),
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
        app.focus_mut().open_overlay(PaneId::Finder);
        app.overlays_mut().open_finder();
        render_ui(&mut app);

        let (x, y) = finder_result_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(
            app.overlays().finder_pane().viewport().pos(),
            1,
            "clicking the second rendered finder result should select result index 1, not the header-offset visual row"
        );
    }

    #[test]
    fn git_hover_uses_owner_backed_pane_surface_for_workspace_member() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let workspace = tmp.path().join("ws");
        let member = workspace.join("core");
        std::fs::create_dir_all(&member).unwrap_or_else(|_| std::process::abort());

        let root = make_workspace_with_members(
            "ws",
            &workspace,
            vec![inline_group(vec![make_member("core", &member)])],
        );
        let mut app = make_app(&[root]);
        app.expanded_mut().insert(ExpandKey::Node(0));
        app.ensure_visible_rows_cached();
        app.projects_mut().move_down();
        let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
        app.handle_repo_info(&workspace, repo);
        app.handle_checkout_info(&workspace, checkout);

        render_ui(&mut app);

        let (x, y) = pane_row_point(app.panes().git().viewport(), 0);
        assert_eq!(
            super::hovered_pane_row_at(&app, Position::new(x, y)),
            Some(HoveredPaneRow {
                pane: PaneId::Git,
                row:  0,
            }),
        );
    }

    #[test]
    fn settings_row_click_uses_setting_index_not_visual_line() {
        let mut app = make_app(&[]);
        app.focus_mut().open_overlay(PaneId::Settings);
        app.overlays_mut().open_settings();
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.overlays().settings_pane().viewport(), 5);
        click(&mut app, x, y);

        assert_eq!(
            app.overlays().settings_pane().viewport().pos(),
            SettingOption::CiRunCount as usize,
            "clicking a rendered settings option should select the logical setting, not the visual line index including spacer/header rows"
        );
    }

    #[test]
    fn keyboard_navigation_clears_stale_settings_hover() {
        let mut app = make_app(&[]);
        app.focus_mut().open_overlay(PaneId::Settings);
        app.overlays_mut().open_settings();
        render_ui(&mut app);

        let hovered_row = SettingOption::CiRunCount as usize;
        let (x, y) = pane_row_point(app.overlays().settings_pane().viewport(), 5);
        move_mouse(&mut app, x, y);
        render_ui(&mut app);

        assert_eq!(
            app.overlays()
                .settings_pane()
                .viewport()
                .selection_state(hovered_row, app.focus().pane_state(PaneId::Settings)),
            PaneSelectionState::Hovered,
        );

        press_key(&mut app, KeyCode::Down);
        render_ui(&mut app);

        assert_eq!(app.overlays().settings_pane().viewport().pos(), 1);
        assert_eq!(
            app.overlays()
                .settings_pane()
                .viewport()
                .selection_state(hovered_row, app.focus().pane_state(PaneId::Settings)),
            PaneSelectionState::Unselected,
        );
        assert_eq!(
            app.overlays()
                .settings_pane()
                .viewport()
                .selection_state(1, app.focus().pane_state(PaneId::Settings)),
            PaneSelectionState::Active,
        );
    }

    #[test]
    fn mouse_move_restores_hover_after_keyboard_navigation() {
        let mut app = make_app(&[]);
        app.focus_mut().open_overlay(PaneId::Settings);
        app.overlays_mut().open_settings();
        render_ui(&mut app);

        let hovered_row = SettingOption::CiRunCount as usize;
        let (x, y) = pane_row_point(app.overlays().settings_pane().viewport(), 5);
        move_mouse(&mut app, x, y);
        render_ui(&mut app);
        press_key(&mut app, KeyCode::Down);
        render_ui(&mut app);

        assert_eq!(
            app.overlays()
                .settings_pane()
                .viewport()
                .selection_state(hovered_row, app.focus().pane_state(PaneId::Settings)),
            PaneSelectionState::Unselected,
        );

        move_mouse(&mut app, x, y);
        render_ui(&mut app);

        assert_eq!(
            app.overlays()
                .settings_pane()
                .viewport()
                .selection_state(hovered_row, app.focus().pane_state(PaneId::Settings)),
            PaneSelectionState::Hovered,
        );
    }

    #[test]
    fn focus_gained_restores_selection_from_last_mouse_position() {
        let mut app = make_app(&[]);
        app.focus_mut().open_overlay(PaneId::Settings);
        app.overlays_mut().open_settings();
        render_ui(&mut app);

        let hovered_row = SettingOption::CiRunCount as usize;
        let (x, y) = pane_row_point(app.overlays().settings_pane().viewport(), 5);
        input::set_last_mouse_pos_for_test(Some((x, y)));
        focus_gained(&mut app);
        render_ui(&mut app);

        assert_eq!(app.overlays().settings_pane().viewport().pos(), hovered_row);
        assert_eq!(
            app.overlays()
                .settings_pane()
                .viewport()
                .selection_state(hovered_row, app.focus().pane_state(PaneId::Settings)),
            PaneSelectionState::Active,
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
            app.lint().viewport().pos(),
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
            Cargo {
                types: vec![ProjectType::Binary],
                examples: vec![ExampleGroup {
                    category: String::new(),
                    names:    vec!["example".to_string()],
                }],
                ..Cargo::default()
            },
        )]);
        let runs = vec![
            make_ci_run(1, Conclusion::Success),
            make_ci_run(2, Conclusion::Failure),
        ];
        render_ci_panel(&mut app, &runs);

        let (x, y) = ci_run_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(
            app.ci().viewport().pos(),
            1,
            "clicking the second rendered CI run should select run index 1, not the header-offset visual row"
        );
    }

    #[test]
    fn expanded_tree_rebuild_refreshes_clickable_rows() {
        let primary: AbsolutePath = "/abs/app".into();
        let linked: AbsolutePath = "/abs/app_feat".into();
        let mut app = make_app(&[RootItem::Rust(RustProject::Package(make_package_worktree(
            "app",
            &primary,
            false,
            Some(primary.as_path()),
        )))]);
        app.expanded_mut().insert(ExpandKey::Node(0));
        render_ui(&mut app);

        app.set_projects(ProjectList::new(vec![RootItem::Worktrees(
            WorktreeGroup::new_packages(
                make_package_worktree("app", &primary, false, Some(primary.as_path())),
                vec![make_package_worktree(
                    "app",
                    &linked,
                    true,
                    Some(primary.as_path()),
                )],
            ),
        )]));
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

        app.projects_mut().set_cursor(0);
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
        let toast_len = app.toasts().active_now().len();
        app.toasts_mut().viewport_mut().set_len(toast_len);
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
        let toast_len = app.toasts().active_now().len();
        app.toasts_mut().viewport_mut().set_len(toast_len);
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
        app.focus_mut().open_overlay(PaneId::Finder);
        app.overlays_mut().open_finder();
        render_ui(&mut app);

        let (x, y) = finder_result_point(&app, 1);
        click(&mut app, x, y);

        assert_eq!(app.overlays().finder_pane().viewport().pos(), 1);
    }

    #[test]
    fn settings_row_click_selects_setting() {
        let mut app = make_app(&[]);
        app.focus_mut().open_overlay(PaneId::Settings);
        app.overlays_mut().open_settings();
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.overlays().settings_pane().viewport(), 2);
        click(&mut app, x, y);

        assert_eq!(
            app.overlays().settings_pane().viewport().pos(),
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

        let (x, y) = pane_row_point(app.panes().package().viewport(), 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::Package);
        assert_eq!(app.panes().package().viewport().pos(), 1);
    }

    #[test]
    fn targets_pane_row_click_selects_target() {
        use cargo_metadata::PackageId;
        use cargo_metadata::TargetKind;
        use cargo_metadata::semver::Version;
        // Step 3b: Targets pane now sources its data from the
        // `cargo metadata` result; the old hand-parsed Cargo
        // fallback is retired. Populate two Example targets via
        // a CargoMetadata arrival so the pane has at least two
        // rows to click on.
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("demo", &project_dir)]);
        let make_target = |name: &str| TargetRecord {
            name:              name.to_string(),
            kinds:             vec![TargetKind::Example],
            src_path:          AbsolutePath::from(project_dir.join(format!("examples/{name}.rs"))),
            edition:           "2021".to_string(),
            required_features: Vec::new(),
        };
        let pkg = PackageRecord {
            id:            PackageId {
                repr: "demo-id".into(),
            },
            name:          "demo".into(),
            version:       Version::new(0, 1, 0),
            edition:       "2021".into(),
            description:   None,
            license:       None,
            homepage:      None,
            repository:    None,
            manifest_path: AbsolutePath::from(project_dir.join("Cargo.toml")),
            targets:       vec![make_target("example_a"), make_target("example_b")],
            publish:       PublishPolicy::Any,
        };
        let mut packages = std::collections::HashMap::new();
        packages.insert(pkg.id.clone(), pkg);
        app.scan()
            .metadata_store_handle()
            .lock()
            .unwrap_or_else(|_| std::process::abort())
            .upsert(WorkspaceMetadata {
                workspace_root: AbsolutePath::from(project_dir.clone()),
                target_directory: AbsolutePath::from(project_dir.join("target")),
                packages,
                workspace_members: Vec::new(),
                fetched_at: std::time::SystemTime::UNIX_EPOCH,
                fingerprint: ManifestFingerprint {
                    manifest:       FileStamp {
                        mtime:        std::time::SystemTime::UNIX_EPOCH,
                        len:          0,
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        std::collections::BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            });
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.panes().targets().viewport(), 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::Targets);
        assert_eq!(app.panes().targets().viewport().pos(), 1);
    }

    #[test]
    fn git_pane_row_click_selects_field() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[make_package("demo", &project_dir)]);
        let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
        app.handle_repo_info(&project_dir, repo);
        app.handle_checkout_info(&project_dir, checkout);
        render_ui(&mut app);

        let (x, y) = pane_row_point(app.panes().git().viewport(), 1);
        click(&mut app, x, y);

        assert_eq!(app.focused_pane(), PaneId::Git);
        assert_eq!(app.panes().git().viewport().pos(), 1);
    }

    // ── Confirm popup renders resolved target dir (Step 2) ─────────

    fn buffer_text(app: &mut App) -> String { buffer_text_sized(app, 120, 40) }

    fn buffer_text_sized(app: &mut App, width: u16, height: u16) -> String {
        app.ensure_visible_rows_cached();
        app.ensure_detail_cached();
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        terminal
            .draw(|frame| crate::tui::render::ui(frame, app))
            .unwrap_or_else(|_| std::process::abort());
        let area = terminal.size().unwrap_or_else(|_| std::process::abort());
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                text.push_str(buffer[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn clean_confirm_popup_shows_resolved_out_of_tree_target_dir() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
        let mut app = make_app(&[make_package("demo", &project_dir)]);

        let custom_target = tmp.path().join("out-of-tree-target");
        app.scan()
            .metadata_store_handle()
            .lock()
            .unwrap_or_else(|_| std::process::abort())
            .upsert(WorkspaceMetadata {
                workspace_root:           AbsolutePath::from(project_dir.clone()),
                target_directory:         AbsolutePath::from(custom_target.clone()),
                packages:                 std::collections::HashMap::new(),
                workspace_members:        Vec::new(),
                fetched_at:               std::time::SystemTime::UNIX_EPOCH,
                fingerprint:              ManifestFingerprint {
                    manifest:       FileStamp {
                        mtime:        std::time::SystemTime::UNIX_EPOCH,
                        len:          0,
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        std::collections::BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            });

        app.set_confirm(ConfirmAction::Clean(AbsolutePath::from(project_dir)));
        let rendered = buffer_text(&mut app);

        assert!(
            rendered.contains("Run cargo clean?"),
            "prompt line still renders"
        );
        let expected = crate::project::home_relative_path(custom_target.as_path());
        assert!(
            rendered.contains(&expected),
            "resolved out-of-tree target dir is shown in the popup (expected {expected:?})"
        );
    }

    fn upsert_fake_package_metadata(
        app: &App,
        project_dir: &Path,
        license: Option<&str>,
        homepage: Option<&str>,
        repository: Option<&str>,
    ) {
        use cargo_metadata::PackageId;
        use cargo_metadata::semver::Version;
        let root = AbsolutePath::from(project_dir);
        let manifest = AbsolutePath::from(project_dir.join("Cargo.toml"));
        let pkg = PackageRecord {
            id:            PackageId {
                repr: "demo-test-id".into(),
            },
            name:          "demo".into(),
            version:       Version::new(0, 1, 0),
            edition:       "2021".into(),
            description:   None,
            license:       license.map(String::from),
            homepage:      homepage.map(String::from),
            repository:    repository.map(String::from),
            manifest_path: manifest,
            targets:       Vec::new(),
            publish:       PublishPolicy::Any,
        };
        let mut packages = std::collections::HashMap::new();
        packages.insert(pkg.id.clone(), pkg);
        let workspace_metadata = WorkspaceMetadata {
            workspace_root: root,
            target_directory: AbsolutePath::from(project_dir.join("target")),
            packages,
            workspace_members: Vec::new(),
            fetched_at: std::time::SystemTime::UNIX_EPOCH,
            fingerprint: ManifestFingerprint {
                manifest:       FileStamp {
                    mtime:        std::time::SystemTime::UNIX_EPOCH,
                    len:          0,
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        std::collections::BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        };
        app.scan()
            .metadata_store_handle()
            .lock()
            .unwrap_or_else(|_| std::process::abort())
            .upsert(workspace_metadata);
    }

    #[test]
    fn package_pane_renders_metadata_edition_license_homepage_repository() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
        let mut app = make_app(&[make_package("demo", &project_dir)]);
        upsert_fake_package_metadata(
            &app,
            &project_dir,
            Some("MIT"),
            Some("a.test/hp"),
            Some("a.test/rp"),
        );
        // Use a taller backend so the package pane's full field list
        // fits without scrolling — recent steps added rows (Targets /
        // Lint / CI / Disk breakdown) ahead of Edition..Repository.
        let rendered = buffer_text_sized(&mut app, 120, 80);

        // All four Step-4 field labels must be present when their
        // corresponding value is populated (edition is always set by
        // the fake metadata). Value fragments are kept short to fit
        // the test backend's 120-column layout once the package pane
        // has split off its allotted share.
        for label in ["Edition", "License", "Homepage", "Repository"] {
            assert!(
                rendered.contains(label),
                "{label} label missing from rendered package pane"
            );
        }
        assert!(rendered.contains("2021"), "edition value (2021) missing");
        assert!(rendered.contains("MIT"), "license value missing");
        assert!(rendered.contains("a.test/hp"), "homepage value missing");
        assert!(rendered.contains("a.test/rp"), "repository value missing");
    }

    #[test]
    fn package_pane_renders_em_dash_for_missing_metadata_fields() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
        let mut app = make_app(&[make_package("demo", &project_dir)]);
        upsert_fake_package_metadata(&app, &project_dir, None, None, None);
        // Taller backend so the full field list fits — see the sibling
        // test for why the default 120×40 isn't enough here.
        let rendered = buffer_text_sized(&mut app, 120, 80);

        // Absent manifest fields render as `—` (design plan step 4).
        // Count dashes in the rendered screen — license / homepage /
        // repository are all None here, so at least three should show.
        let dash_count = rendered.matches('—').count();
        assert!(
            dash_count >= 3,
            "expected at least 3 em-dash placeholders for missing \
             license/homepage/repository, got {dash_count}"
        );
    }

    #[test]
    fn package_pane_renders_target_and_non_target_disk_breakdown() {
        // Step 5b: when the walker has reported the breakdown, the
        // Package pane shows two rows beneath `Disk` — `target/`
        // and `other` — so the user can see at a glance which half
        // of their disk is build artifact vs source. Uses the bytes
        // reported by handle_bg_msg::DiskUsageBatch.
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
        let mut app = make_app(&[make_package("demo", &project_dir)]);

        // Stage a disk-usage batch with a clearly-split breakdown.
        // 10 MiB target, 2 MiB source: assert both lines render with
        // distinct byte formatting.
        let abs_path = AbsolutePath::from(project_dir);
        let sizes = DirSizes {
            total:                 12 * 1024 * 1024,
            in_project_target:     10 * 1024 * 1024,
            in_project_non_target: 2 * 1024 * 1024,
        };
        app.handle_bg_msg(BackgroundMsg::DiskUsageBatch {
            root_path: abs_path.clone(),
            entries:   vec![(abs_path, sizes)],
        });

        let rendered = buffer_text(&mut app);
        assert!(
            rendered.contains("target/"),
            "detail pane must surface the target/ breakdown label"
        );
        assert!(
            rendered.contains("other"),
            "detail pane must surface the non-target (other) breakdown label"
        );
        assert!(
            rendered.contains("10.0 MiB"),
            "in-target value renders using format_bytes"
        );
        assert!(
            rendered.contains("2.0 MiB"),
            "non-target value renders using format_bytes"
        );
    }

    #[test]
    fn package_pane_renders_out_of_tree_target_size_for_sharer() {
        // When the workspace's target_directory sits outside
        // workspace_root (e.g. redirected via CARGO_TARGET_DIR or an
        // ancestor .cargo/config.toml), the per-project walker can't
        // reach it. The cached walk fills in the sharer target size,
        // which shows up beneath Disk as `target/ (out of tree)`.
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        let shared_target = tmp.path().join("shared-target");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
        let mut app = make_app(&[make_package("demo", &project_dir)]);

        let root = AbsolutePath::from(project_dir);
        let target = AbsolutePath::from(shared_target);
        {
            let store = app.scan().metadata_store_handle();
            let mut guard = store.lock().unwrap_or_else(|_| std::process::abort());
            guard.upsert(WorkspaceMetadata {
                workspace_root:           root,
                target_directory:         target,
                packages:                 std::collections::HashMap::new(),
                workspace_members:        Vec::new(),
                fetched_at:               std::time::SystemTime::UNIX_EPOCH,
                fingerprint:              ManifestFingerprint {
                    manifest:       FileStamp {
                        mtime:        std::time::SystemTime::UNIX_EPOCH,
                        len:          0,
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        std::collections::BTreeMap::new(),
                },
                out_of_tree_target_bytes: Some(42 * 1024 * 1024),
            });
        }

        let rendered = buffer_text(&mut app);
        assert!(
            rendered.contains("out of tree"),
            "sharer detail pane must surface the out-of-tree target label"
        );
        assert!(
            rendered.contains("42.0 MiB"),
            "out-of-tree target size renders using format_bytes"
        );
    }

    /// Helper for the shared-target popup tests: stage two project
    /// metadata "arrivals" pointing at the same `target_directory`,
    /// so the `TargetDirIndex` reports sibling B when we confirm a
    /// clean on A.
    fn upsert_shared_target_metadata(
        app: &mut App,
        primary_dir: &Path,
        sibling_dirs: &[&Path],
        target_dir: &Path,
    ) {
        use cargo_metadata::PackageId;
        use cargo_metadata::semver::Version;
        for dir in std::iter::once(primary_dir).chain(sibling_dirs.iter().copied()) {
            let root = AbsolutePath::from(dir);
            let manifest = AbsolutePath::from(dir.join("Cargo.toml"));
            let pkg_name = dir
                .file_name()
                .map_or_else(|| "demo".to_string(), |n| n.to_string_lossy().into_owned());
            let pkg = PackageRecord {
                id:            PackageId {
                    repr: format!("{pkg_name}-id"),
                },
                name:          pkg_name,
                version:       Version::new(0, 1, 0),
                edition:       "2021".into(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: manifest,
                targets:       Vec::new(),
                publish:       PublishPolicy::Any,
            };
            let mut packages = std::collections::HashMap::new();
            packages.insert(pkg.id.clone(), pkg);
            let workspace_metadata = WorkspaceMetadata {
                workspace_root: root.clone(),
                target_directory: AbsolutePath::from(target_dir),
                packages,
                workspace_members: Vec::new(),
                fetched_at: std::time::SystemTime::UNIX_EPOCH,
                fingerprint: ManifestFingerprint {
                    manifest:       FileStamp {
                        mtime:        std::time::SystemTime::UNIX_EPOCH,
                        len:          0,
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        std::collections::BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            };
            // Route through handle_bg_msg so the TargetDirIndex gets
            // refreshed alongside the store (Step 6c handler path).
            let store = app.scan().metadata_store_handle();
            let generation = store
                .lock()
                .unwrap_or_else(|_| std::process::abort())
                .next_generation(&root);
            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: root,
                generation,
                fingerprint: workspace_metadata.fingerprint.clone(),
                result: Ok(workspace_metadata),
            });
        }
    }

    #[test]
    fn clean_confirm_popup_lists_affected_siblings_on_shared_target() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let primary_dir = tmp.path().join("main");
        let sibling_dir = tmp.path().join("feat");
        let target_dir = tmp.path().join("shared-target");
        for dir in [&primary_dir, &sibling_dir] {
            std::fs::create_dir_all(dir).unwrap_or_else(|_| std::process::abort());
        }
        std::fs::create_dir_all(&target_dir).unwrap_or_else(|_| std::process::abort());

        let mut app = make_app(&[
            make_package("main", &primary_dir),
            make_package("feat", &sibling_dir),
        ]);
        upsert_shared_target_metadata(
            &mut app,
            &primary_dir,
            &[sibling_dir.as_path()],
            &target_dir,
        );

        app.set_confirm(ConfirmAction::Clean(AbsolutePath::from(primary_dir)));
        let rendered = buffer_text(&mut app);

        assert!(
            rendered.contains("Also affects:"),
            "shared-target popup should label the collateral list"
        );
        let sibling_label = crate::project::home_relative_path(sibling_dir.as_path());
        assert!(
            rendered.contains(&sibling_label),
            "sibling path should appear in the affected list (expected {sibling_label:?})"
        );
    }

    #[test]
    fn clean_confirm_popup_falls_back_to_in_tree_target_without_metadata() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let project_dir = tmp.path().join("demo");
        std::fs::create_dir_all(&project_dir).unwrap_or_else(|_| std::process::abort());
        let mut app = make_app(&[make_package("demo", &project_dir)]);

        app.set_confirm(ConfirmAction::Clean(AbsolutePath::from(
            project_dir.clone(),
        )));
        let rendered = buffer_text(&mut app);

        let fallback_target = project_dir.join("target");
        let expected = crate::project::home_relative_path(fallback_target.as_path());
        assert!(
            rendered.contains(&expected),
            "without metadata, popup shows the default <project>/target (expected {expected:?})"
        );
    }

    #[test]
    fn clean_group_confirm_popup_lists_all_checkouts() {
        // Selecting Clean on a worktree-group root should open the
        // confirm popup with every checkout listed — the UX regression
        // was that the WorktreeGroup arm was stubbed out so the popup
        // never appeared at all.
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let primary = tmp.path().join("main");
        let linked_a = tmp.path().join("feat-a");
        let linked_b = tmp.path().join("feat-b");
        for dir in [&primary, &linked_a, &linked_b] {
            std::fs::create_dir_all(dir).unwrap_or_else(|_| std::process::abort());
        }

        let mut app = make_app(&[]);
        app.set_confirm(ConfirmAction::CleanGroup {
            primary: AbsolutePath::from(primary.clone()),
            linked:  vec![
                AbsolutePath::from(linked_a.clone()),
                AbsolutePath::from(linked_b.clone()),
            ],
        });
        let rendered = buffer_text_sized(&mut app, 160, 40);

        assert!(
            rendered.contains("Run cargo clean on all checkouts?"),
            "group confirm uses the fan-out prompt"
        );
        assert!(
            rendered.contains("Checkouts:"),
            "group confirm labels the checkout list"
        );
        for dir in [&primary, &linked_a, &linked_b] {
            let label = crate::project::home_relative_path(dir.as_path());
            assert!(
                rendered.contains(&label),
                "every checkout appears in the popup (expected {label:?})"
            );
        }
    }
}
