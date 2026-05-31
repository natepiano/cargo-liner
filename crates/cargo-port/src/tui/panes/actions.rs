use std::path::Path;

#[cfg(test)]
use crossterm::event::KeyEvent;
use tui_pane::FocusedPane;
use tui_pane::FrameworkFocusId;
#[cfg(test)]
use tui_pane::KeyBind as TuiKeyBind;
use tui_pane::TrackedItem;
use tui_pane::Viewport;

use super::BuildMode;
use super::CiFetchKind;
use super::DetailField;
use super::GitRow;
use super::PaneId;
use super::PendingCiFetch;
use super::PendingExampleRun;
use super::TargetSource;
use super::build_target_display_rows;
use super::build_target_list_from_data;
use super::resolve_kill_request;
use crate::lint;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::app::CleanSelection;
use crate::tui::input;
use crate::tui::integration;
use crate::tui::integration::AppPaneId;
use crate::tui::integration::NavAction;
use crate::tui::keymap::CiRunsAction;
use crate::tui::keymap::GitAction;
#[cfg(test)]
use crate::tui::keymap::KeyBind;
use crate::tui::keymap::LintsAction;
use crate::tui::keymap::PackageAction;
use crate::tui::keymap::TargetsAction;

fn handle_target_action(app: &mut App, mode: BuildMode) {
    let Some(targets_data) = app.panes.targets.content().cloned() else {
        return;
    };
    let entries = build_target_list_from_data(&targets_data);
    // The pane cursor indexes display rows (which include per-instance
    // child rows), so resolve it back to the owning target entry before
    // running. Running from an instance child row launches another
    // instance of that same target.
    let display_rows = build_target_display_rows(
        &entries,
        app.panes.running_targets.snapshot(),
        app.panes.detail_target_dir.as_ref(),
    );
    let entry_index = display_rows
        .get(app.panes.targets.viewport.pos())
        .map(|row| row.entry_index);
    if let Some(entry) = entry_index.and_then(|index| entries.get(index))
        && let Some(abs_path) = app.project_list.selected_project_path()
    {
        // Member-owned targets carry the owning package's name in
        // `TargetSource::Member`, which downstream cargo invocations
        // pass as `--package <name>`. Workspace-root targets fall back
        // to the selected project's package title (cargo runs against
        // the default-run package when no `-p` is given).
        let package_name = match &entry.source {
            TargetSource::Member(name) => Some(name.clone()),
            TargetSource::Workspace => app
                .panes
                .package
                .content()
                .and_then(|d| (d.title_name != "-").then(|| d.title_name.clone())),
        };
        app.inflight.set_pending_example_run(PendingExampleRun {
            abs_path: abs_path.display().to_string(),
            target_name: entry.name.clone(),
            package_name,
            kind: entry.kind,
            build_mode: mode,
            required_features: entry.required_features.clone(),
        });
    }
}

pub(super) fn dispatch_package_action(action: PackageAction, app: &mut App) {
    match action {
        PackageAction::Activate => handle_detail_enter(app),
    }
}

pub(super) fn dispatch_git_action(action: GitAction, app: &mut App) {
    match action {
        GitAction::Activate => handle_detail_enter(app),
    }
}

pub(super) fn dispatch_targets_action(action: TargetsAction, app: &mut App) {
    match action {
        TargetsAction::Activate => handle_detail_enter(app),
        TargetsAction::ReleaseBuild => handle_target_action(app, BuildMode::Release),
        TargetsAction::Kill => handle_target_kill(app),
    }
}

/// Open a confirm dialog to `SIGTERM` the running instance(s) under the
/// selected Targets row. An instance child row (or a single-instance
/// target row) kills that one PID; a multi-instance parent row kills every
/// instance of the target. A no-op when the selected row has nothing
/// running.
fn handle_target_kill(app: &mut App) {
    let Some(targets_data) = app.panes.targets.content().cloned() else {
        return;
    };
    let entries = build_target_list_from_data(&targets_data);
    let dir = app.panes.detail_target_dir.clone();
    let display_rows =
        build_target_display_rows(&entries, app.panes.running_targets.snapshot(), dir.as_ref());
    let request = resolve_kill_request(
        &entries,
        &display_rows,
        app.panes.running_targets.snapshot(),
        dir.as_ref(),
        app.panes.targets.viewport.pos(),
    );
    if let Some(request) = request {
        app.request_kill_confirm(request.label, request.pids);
    }
}

/// Send `SIGTERM` to `pids` (the confirmed kill), drop them from the
/// running snapshot so their rows collapse on the next render, and keep
/// the Targets cursor anchored to the same target. Without the anchor,
/// killing one of several instances would let the cursor slide onto the
/// next target when the instance rows collapse away.
pub(super) fn execute_target_kill(app: &mut App, pids: &[u32]) {
    for pid in pids {
        app.panes.running_targets.kill(*pid);
    }
    let anchor_entry = targets_cursor_entry(app);
    app.panes.running_targets.drop_instances(pids);
    if let Some(entry_index) = anchor_entry {
        anchor_targets_cursor_to_entry(app, entry_index);
    }
}

/// The target entry index under the Targets cursor, or `None` when there
/// is no target data or the cursor is out of range.
fn targets_cursor_entry(app: &App) -> Option<usize> {
    let data = app.panes.targets.content()?;
    let entries = build_target_list_from_data(data);
    let rows = build_target_display_rows(
        &entries,
        app.panes.running_targets.snapshot(),
        app.panes.detail_target_dir.as_ref(),
    );
    rows.get(app.panes.targets.viewport.pos())
        .map(|row| row.entry_index)
}

/// Move the Targets cursor to the first display row that renders
/// `entry_index` (its target or multi-instance parent row), clamping to
/// the last row if the entry is gone.
fn anchor_targets_cursor_to_entry(app: &mut App, entry_index: usize) {
    let Some(data) = app.panes.targets.content().cloned() else {
        return;
    };
    let entries = build_target_list_from_data(&data);
    let rows = build_target_display_rows(
        &entries,
        app.panes.running_targets.snapshot(),
        app.panes.detail_target_dir.as_ref(),
    );
    if let Some(pos) = super::display_row_for_entry(&rows, entry_index) {
        app.panes.targets.viewport.set_pos(pos);
    }
}

pub(super) fn dispatch_lints_action(action: LintsAction, app: &mut App) {
    match action {
        LintsAction::Activate => open_lint_run_output(app),
        LintsAction::ClearHistory => clear_lint_history(app),
    }
}

pub(super) fn dispatch_ci_runs_action(action: CiRunsAction, app: &mut App) {
    match action {
        CiRunsAction::Activate => handle_ci_enter(app),
        CiRunsAction::FetchMore => handle_ci_fetch_more(app),
        CiRunsAction::ShowBranch => set_ci_display_mode(app, CiRunDisplayMode::BranchOnly),
        CiRunsAction::ShowAll => set_ci_display_mode(app, CiRunDisplayMode::All),
        CiRunsAction::ClearCache => {
            if let Some(path) = app.project_list.selected_ci_path() {
                clear_ci_cache(app, &path);
            }
        },
    }
}

fn set_ci_display_mode(app: &mut App, mode: CiRunDisplayMode) {
    if let Some(path) = app
        .project_list
        .selected_project_path()
        .map(Path::to_path_buf)
    {
        app.set_ci_display_mode_for(&path, mode);
    }
}

pub(super) fn dispatch_navigation_action(
    action: NavAction,
    focused: FocusedPane<AppPaneId>,
    app: &mut App,
) {
    let edge_advance = edge_scroll_probe(action, focused, app);

    match focused {
        FocusedPane::App(AppPaneId::ProjectList) => navigate_project_list(app, action),
        FocusedPane::App(AppPaneId::Package) => navigate_package_detail(app, action),
        FocusedPane::App(
            AppPaneId::Lang | AppPaneId::Cpu | AppPaneId::Git | AppPaneId::Targets,
        ) => navigate_detail(app, action),
        FocusedPane::App(AppPaneId::Lints) => navigate_lints(app, action),
        FocusedPane::App(AppPaneId::CiRuns) => navigate_ci_runs(app, action),
        FocusedPane::App(AppPaneId::Output) => navigate_output(app, action),
        FocusedPane::App(AppPaneId::Finder) => {},
        FocusedPane::Framework(FrameworkFocusId::Toasts) => navigate_toasts(app, action),
    }

    // When the cursor could not move — the list was already at its edge —
    // roll focus to the adjacent pane in tab order instead of stopping.
    if let Some((advance, cursor_before)) = edge_advance
        && list_cursor(focused, app) == Some(cursor_before)
    {
        match advance {
            EdgeAdvance::Next => tui_pane::focus_next(app),
            EdgeAdvance::Prev => tui_pane::focus_prev(app),
        }
    }
}

/// Direction to roll focus when a vertical scroll runs off a list edge.
enum EdgeAdvance {
    Next,
    Prev,
}

/// Decide whether this navigation should advance the focused pane on a
/// no-op edge hit, and capture the cursor position to compare against
/// afterward. Returns `None` when the edge-scroll setting is off, the
/// action is not a single vertical step, or the focused pane has no
/// participating list.
fn edge_scroll_probe(
    action: NavAction,
    focused: FocusedPane<AppPaneId>,
    app: &App,
) -> Option<(EdgeAdvance, usize)> {
    if !app.config.edge_scroll().advances_pane() {
        return None;
    }
    let advance = match action {
        NavAction::Up => EdgeAdvance::Prev,
        NavAction::Down => EdgeAdvance::Next,
        NavAction::Left
        | NavAction::Right
        | NavAction::Home
        | NavAction::End
        | NavAction::PageUp
        | NavAction::PageDown
        | NavAction::HalfPageUp
        | NavAction::HalfPageDown => return None,
    };
    list_cursor(focused, app).map(|cursor| (advance, cursor))
}

/// Current cursor row for the focused pane's list, or `None` for panes
/// that do not take part in edge-scroll focus advance (text input,
/// static output, and non-list framework panes).
fn list_cursor(focused: FocusedPane<AppPaneId>, app: &App) -> Option<usize> {
    match focused {
        FocusedPane::App(AppPaneId::ProjectList) => Some(app.project_list.cursor()),
        FocusedPane::App(AppPaneId::Package) => Some(app.panes.package.viewport.pos()),
        FocusedPane::App(
            AppPaneId::Lang | AppPaneId::Cpu | AppPaneId::Git | AppPaneId::Targets,
        ) => Some(active_detail_viewport(app).pos()),
        FocusedPane::App(AppPaneId::Lints) => Some(app.lint.viewport.pos()),
        FocusedPane::App(AppPaneId::CiRuns) => Some(app.ci.viewport.pos()),
        FocusedPane::Framework(FrameworkFocusId::Toasts) => app
            .framework
            .toasts
            .has_active()
            .then_some(app.framework.toasts.viewport.pos()),
        FocusedPane::App(AppPaneId::Output | AppPaneId::Finder) => None,
    }
}

fn navigate_project_list(app: &mut App, action: NavAction) {
    let include_non_rust = app.config.include_non_rust().includes_non_rust();
    match action {
        NavAction::Up => app.project_list.move_up(),
        NavAction::Down => app.project_list.move_down(),
        NavAction::Home => app.project_list.move_to_top(),
        NavAction::End => app.project_list.move_to_bottom(),
        NavAction::PageUp => {
            if let Some(step) = project_list_page_step(app) {
                app.project_list.move_up_by(step);
            }
        },
        NavAction::PageDown => {
            if let Some(step) = project_list_page_step(app) {
                app.project_list.move_down_by(step);
            }
        },
        NavAction::HalfPageUp => {
            if let Some(step) = project_list_half_page_step(app) {
                app.project_list.move_up_by(step);
            }
        },
        NavAction::HalfPageDown => {
            if let Some(step) = project_list_half_page_step(app) {
                app.project_list.move_down_by(step);
            }
        },
        NavAction::Right => {
            if !app.expand() {
                app.project_list.move_down();
            }
        },
        NavAction::Left => {
            if !app.project_list.collapse(include_non_rust) {
                app.project_list.move_up();
            }
        },
    }
}

fn navigate_detail(app: &mut App, action: NavAction) {
    let pane = active_detail_pane(app);
    navigate_viewport(pane, action);
}

fn navigate_viewport(pane: &mut Viewport, action: NavAction) {
    match action {
        NavAction::Up | NavAction::Left => pane.up(),
        NavAction::Down | NavAction::Right => pane.down(),
        NavAction::Home => pane.home(),
        NavAction::End => pane.end(),
        NavAction::PageUp => pane.page_up(),
        NavAction::PageDown => pane.page_down(),
        NavAction::HalfPageUp => pane.half_page_up(),
        NavAction::HalfPageDown => pane.half_page_down(),
    }
}

pub(super) fn navigate_package_detail(app: &mut App, action: NavAction) {
    let Some(package) = app.panes.package.content() else {
        navigate_viewport(&mut app.panes.package.viewport, action);
        return;
    };

    let rows = super::package_rows_from_data(package);
    let current = app
        .panes
        .package
        .viewport
        .pos()
        .min(rows.len().saturating_sub(1));
    let target = match action {
        NavAction::Up | NavAction::Left => {
            super::package_selectable_row_at_or_before(&rows, current.saturating_sub(1))
                .or_else(|| super::package_first_selectable_row(&rows))
        },
        NavAction::Down | NavAction::Right => {
            super::package_selectable_row_at_or_after(&rows, current.saturating_add(1))
                .or_else(|| super::package_last_selectable_row(&rows))
        },
        NavAction::Home => super::package_first_selectable_row(&rows),
        NavAction::End => super::package_last_selectable_row(&rows),
        NavAction::PageUp => {
            let step = app
                .panes
                .package
                .viewport
                .visible_rows()
                .saturating_sub(1)
                .max(1);
            let target = current.saturating_sub(step);
            super::package_selectable_row_at_or_before(&rows, target)
                .or_else(|| super::package_selectable_row_at_or_after(&rows, target))
        },
        NavAction::PageDown => {
            let step = app
                .panes
                .package
                .viewport
                .visible_rows()
                .saturating_sub(1)
                .max(1);
            let target = current
                .saturating_add(step)
                .min(rows.len().saturating_sub(1));
            super::package_selectable_row_at_or_after(&rows, target)
                .or_else(|| super::package_selectable_row_at_or_before(&rows, target))
        },
        NavAction::HalfPageUp => {
            let step = (app.panes.package.viewport.visible_rows() / 2).max(1);
            let target = current.saturating_sub(step);
            super::package_selectable_row_at_or_before(&rows, target)
                .or_else(|| super::package_selectable_row_at_or_after(&rows, target))
        },
        NavAction::HalfPageDown => {
            let step = (app.panes.package.viewport.visible_rows() / 2).max(1);
            let target = current
                .saturating_add(step)
                .min(rows.len().saturating_sub(1));
            super::package_selectable_row_at_or_after(&rows, target)
                .or_else(|| super::package_selectable_row_at_or_before(&rows, target))
        },
    };
    if let Some(pos) = target {
        app.panes.package.viewport.set_pos(pos);
    }
}

fn navigate_lints(app: &mut App, action: NavAction) {
    match action {
        NavAction::Up | NavAction::Left => app.lint.viewport.up(),
        NavAction::Down | NavAction::Right => app.lint.viewport.down(),
        NavAction::Home => app.lint.viewport.home(),
        NavAction::End => app.lint.viewport.end(),
        NavAction::PageUp => app.lint.viewport.page_up(),
        NavAction::PageDown => app.lint.viewport.page_down(),
        NavAction::HalfPageUp => app.lint.viewport.half_page_up(),
        NavAction::HalfPageDown => app.lint.viewport.half_page_down(),
    }
}

fn navigate_ci_runs(app: &mut App, action: NavAction) {
    match action {
        NavAction::Up | NavAction::Left => app.ci.viewport.up(),
        NavAction::Down | NavAction::Right => app.ci.viewport.down(),
        NavAction::Home => app.ci.viewport.home(),
        NavAction::End => app.ci.viewport.end(),
        NavAction::PageUp => app.ci.viewport.page_up(),
        NavAction::PageDown => app.ci.viewport.page_down(),
        NavAction::HalfPageUp => app.ci.viewport.half_page_up(),
        NavAction::HalfPageDown => app.ci.viewport.half_page_down(),
    }
}

/// Drive the output pane cursor through the shared viewport navigation —
/// the same handler every scroll pane uses, so vim keys and page/half-page
/// motions come for free. Follow-vs-frozen is derived from the cursor:
/// moving up off the last row frees the view, returning to it (Down at the
/// tail, `End`) follows again. While a selection is active the same motions
/// extend the range against the frozen snapshot (the cursor is one end, the
/// anchor the other).
fn navigate_output(app: &mut App, action: NavAction) {
    navigate_viewport(&mut app.panes.output.viewport, action);
}

fn navigate_toasts(app: &mut App, action: NavAction) {
    let active_count = app.framework.toasts.active_now().len();
    app.framework.toasts.viewport.set_len(active_count);
    match action {
        NavAction::Up | NavAction::Left => app.framework.toasts.viewport.up(),
        NavAction::Down | NavAction::Right => app.framework.toasts.viewport.down(),
        NavAction::Home => app.framework.toasts.viewport.home(),
        NavAction::PageUp => app.framework.toasts.viewport.page_up(),
        NavAction::PageDown => app.framework.toasts.viewport.page_down(),
        NavAction::HalfPageUp => app.framework.toasts.viewport.half_page_up(),
        NavAction::HalfPageDown => app.framework.toasts.viewport.half_page_down(),
        NavAction::End => {
            let last_index = active_count.saturating_sub(1);
            app.framework.toasts.viewport.set_pos(last_index);
        },
    }
}

fn project_list_page_step(app: &App) -> Option<usize> {
    let rows = app.panes.project_list.viewport.visible_rows();
    (rows > 0).then(|| rows.saturating_sub(1).max(1))
}

fn project_list_half_page_step(app: &App) -> Option<usize> {
    let rows = app.panes.project_list.viewport.visible_rows();
    (rows > 0).then(|| (rows / 2).max(1))
}

pub(super) fn request_clean(app: &mut App) {
    // Gated through App::clean_selection — the single source of truth
    // for clean eligibility, regardless of which pane currently owns
    // focus.
    if let Some(selection) = app.project_list.clean_selection() {
        match selection {
            CleanSelection::Project { root } => {
                app.request_clean_confirm(root);
            },
            CleanSelection::WorktreeGroup { primary, linked } => {
                app.request_clean_group_confirm(primary, linked);
            },
        }
    }
}

/// Return a mutable reference to the pane that owns the cursor for the
/// currently active detail column.
fn active_detail_pane(app: &mut App) -> &mut Viewport {
    match app.base_focus() {
        PaneId::Targets => &mut app.panes.targets.viewport,
        PaneId::Lang => &mut app.panes.lang.viewport,
        PaneId::Cpu => &mut app.panes.cpu.viewport,
        PaneId::Git => &mut app.panes.git.viewport,
        PaneId::Package
        | PaneId::ProjectList
        | PaneId::Lints
        | PaneId::CiRuns
        | PaneId::Output
        | PaneId::Toasts
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap
        | PaneId::Sccache => &mut app.panes.package.viewport,
    }
}

/// Shared-reference counterpart to [`active_detail_pane`], used to read
/// the cursor row without taking a mutable borrow.
fn active_detail_viewport(app: &App) -> &Viewport {
    match app.base_focus() {
        PaneId::Targets => &app.panes.targets.viewport,
        PaneId::Lang => &app.panes.lang.viewport,
        PaneId::Cpu => &app.panes.cpu.viewport,
        PaneId::Git => &app.panes.git.viewport,
        PaneId::Package
        | PaneId::ProjectList
        | PaneId::Lints
        | PaneId::CiRuns
        | PaneId::Output
        | PaneId::Toasts
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap
        | PaneId::Sccache => &app.panes.package.viewport,
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App) {
    if app.focus_is(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.base_focus() == PaneId::Package {
        if let Some(pkg) = app.panes.package.content()
            && matches!(
                super::package_field_at(pkg, app.panes.package.viewport.pos()),
                Some(DetailField::CratesIo)
            )
        {
            open_url(&format!("https://crates.io/crates/{}", pkg.title_name));
        }
    } else if let Some(git) = app.panes.git.content() {
        let pos = app.panes.git.viewport.pos();
        if let Some(GitRow::PullRequest(pull_request)) = super::git_row_at(git, pos) {
            open_url(&pull_request.url);
            return;
        }
        if let Some(GitRow::Remote(remote)) = super::git_row_at(git, pos)
            && let Some(url) = remote.full_url.as_deref()
        {
            open_url(url);
        }
    }
}

fn open_url(url: &str) {
    let _ = std::process::Command::new(if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "start"
    } else {
        "xdg-open"
    })
    .arg(url)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn();
}

#[cfg(test)]
pub fn handle_ci_runs_key(app: &mut App, event: &KeyEvent) {
    // Pane scope first — TOML rebinds win over navigation defaults.
    let bind = KeyBind::from_parts(event.code, event.modifiers);
    if let Some(action) = app.keymap.current().ci_runs.action_for(&bind) {
        match action {
            CiRunsAction::Activate => handle_ci_enter(app),
            CiRunsAction::FetchMore => handle_ci_fetch_more(app),
            CiRunsAction::ShowBranch => set_ci_display_mode(app, CiRunDisplayMode::BranchOnly),
            CiRunsAction::ShowAll => set_ci_display_mode(app, CiRunDisplayMode::All),
            CiRunsAction::ClearCache => {
                if let Some(path) = app.project_list.selected_ci_path() {
                    clear_ci_cache(app, &path);
                }
            },
        }
        return;
    }

    // Navigation scope.
    let dispatch_bind = TuiKeyBind::from_key_event(*event);
    if let Some(nav_scope) = app.framework_keymap.navigation()
        && let Some(nav_action) = nav_scope.action_for(&dispatch_bind)
    {
        match nav_action {
            NavAction::Up => app.ci.viewport.up(),
            NavAction::Down => app.ci.viewport.down(),
            NavAction::Home => app.ci.viewport.home(),
            NavAction::End => app.ci.viewport.end(),
            NavAction::PageUp => app.ci.viewport.page_up(),
            NavAction::PageDown => app.ci.viewport.page_down(),
            NavAction::HalfPageUp => app.ci.viewport.half_page_up(),
            NavAction::HalfPageDown => app.ci.viewport.half_page_down(),
            NavAction::Left | NavAction::Right => {},
        }
    }
}

fn handle_ci_enter(app: &App) {
    let visible_runs = app
        .ci
        .content()
        .map(|data| data.runs.clone())
        .unwrap_or_default();
    let cursor_pos = app.ci.viewport.pos();
    if let Some(run) = visible_runs.get(cursor_pos) {
        open_url(&run.url);
    }
}

fn handle_ci_fetch_more(app: &mut App) {
    let is_fetching = app
        .project_list
        .selected_project_path()
        .is_some_and(|path| app.ci.fetch_tracker.is_fetching(path));
    if is_fetching {
        return;
    }
    let Some(ci_path) = app.project_list.selected_ci_path() else {
        return;
    };
    let project_name = app
        .project_list
        .selected_project_path()
        .and_then(|path| {
            app.project_list
                .iter()
                .find(|item| item.path() == path)
                .and_then(|item| item.name().map(str::to_string))
        })
        .unwrap_or_else(|| project::home_relative_path(&ci_path));
    // Always start with Sync: pick up runs newer than the cached set. If
    // Sync surfaces nothing, `poll_ci_fetches` automatically chains a
    // FetchOlder using the cached tail as the cursor.
    app.inflight.set_pending_ci_fetch(PendingCiFetch {
        project_path:      ci_path.display().to_string(),
        ci_run_count:      app.config.ci_run_count(),
        oldest_created_at: None,
        kind:              CiFetchKind::Sync,
    });
    let task_id = app
        .framework
        .toasts
        .start_task("Fetching CI", &project_name);
    let item = TrackedItem {
        label:        project_name,
        key:          integration::path_key(&ci_path),
        started_at:   Some(std::time::Instant::now()),
        completed_at: None,
    };
    app.set_task_tracked_items(task_id, &[item]);
    app.ci.set_fetch_toast(Some(task_id));
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, abs: &Path) {
    let (title, body) = if let Some(repo) = app.owner_repo_for_path(abs) {
        let dir = scan::ci_cache_dir_pub(repo.owner(), repo.repo());
        let result = std::fs::remove_dir_all(&dir);
        scan::clear_exhausted(repo.owner(), repo.repo());
        if let Ok(mut cache) = app.net.github.fetch_cache.lock() {
            cache.remove(&repo);
        }
        match result {
            Ok(()) => (
                "CI cache cleared",
                format!("{}/{}", repo.owner(), repo.repo()),
            ),
            Err(err) => ("CI cache clear failed", format!("{}: {err}", dir.display())),
        }
    } else {
        (
            "CI cache clear failed",
            format!("no owner/repo for {}", abs.display()),
        )
    };
    let _ = app.framework.toasts.push_status(title, body);
    let prev_total = app
        .project_list
        .ci_data_for(abs)
        .map_or(0, ProjectCiData::github_total);
    app.project_list.replace_ci_data_for_path(
        abs,
        ProjectCiData::Loaded(ProjectCiInfo {
            runs:         Vec::new(),
            github_total: prev_total,
            exhausted:    false,
        }),
    );
    app.ci.fetch_tracker.complete(abs);
    app.ci.viewport.home();
    app.scan.bump_generation();
}

fn clear_lint_history(app: &mut App) {
    if !app.selected_row_owns_lint() {
        return;
    }
    let Some(abs_path) = app
        .project_list
        .selected_project_path()
        .map(Path::to_path_buf)
    else {
        return;
    };
    let project_cache_dir = lint::project_dir(&abs_path);
    let _ = std::fs::remove_dir_all(project_cache_dir);

    if let Some(lr) = app.lint_at_path_mut(&abs_path) {
        lr.clear_runs();
    }
    app.lint.viewport.home();
    app.set_focus_to_pane(PaneId::ProjectList);
    app.refresh_lint_cache_usage_from_disk();
    app.scan.bump_generation();
}

fn open_lint_run_output(app: &App) {
    if !app.selected_row_owns_lint() {
        return;
    }
    let Some(abs_path) = app.project_list.selected_project_path() else {
        return;
    };
    let Some(runs) = app.lint.content().map(|data| data.runs.as_slice()) else {
        return;
    };
    if runs.is_empty() {
        return;
    }
    let Some(run) = runs.get(app.lint.viewport.pos()) else {
        return;
    };

    let project_cache_dir = lint::project_dir(abs_path);
    let log_paths: Vec<AbsolutePath> = run
        .commands
        .iter()
        .map(|command| AbsolutePath::from(project_cache_dir.join(&command.log_file)))
        .filter(|path| path.exists())
        .collect();

    if log_paths.is_empty() {
        return;
    }

    for path in log_paths {
        let _ = input::open_paths_in_editor(app.config.editor(), [path.as_path()]);
    }
}
