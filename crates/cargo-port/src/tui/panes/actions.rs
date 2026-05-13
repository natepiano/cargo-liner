use std::path::Path;

#[cfg(test)]
use crossterm::event::KeyEvent;
use tui_pane::FocusedPane;
use tui_pane::FrameworkFocusId;
#[cfg(test)]
use tui_pane::KeyBind as FrameworkKeyBind;
use tui_pane::TrackedItem;

use super::BuildMode;
use super::CiFetchKind;
use super::DetailField;
use super::GitRow;
use super::PaneId;
use super::PendingCiFetch;
use super::PendingExampleRun;
use super::build_target_list_from_data;
use crate::keymap::CiRunsAction;
use crate::keymap::GitAction;
#[cfg(test)]
use crate::keymap::KeyBind;
use crate::keymap::LintsAction;
use crate::keymap::PackageAction;
use crate::keymap::TargetsAction;
use crate::lint;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::CleanSelection;
#[cfg(test)]
use crate::tui::framework_keymap::AppNavigation;
use crate::tui::framework_keymap::AppPaneId;
use crate::tui::framework_keymap::CpuAction;
use crate::tui::framework_keymap::LangAction;
use crate::tui::framework_keymap::NavigationAction;
use crate::tui::input;
use crate::tui::pane::Viewport;
use crate::tui::toast_adapters;

fn handle_target_action(app: &mut App, mode: BuildMode) {
    let Some(targets_data) = app.panes.targets.content().cloned() else {
        return;
    };
    let entries = build_target_list_from_data(&targets_data);
    if let Some(entry) = entries.get(app.panes.targets.viewport.pos())
        && let Some(abs_path) = app.project_list.selected_project_path()
    {
        let package_name = app.panes.package.content().and_then(|d| {
            if d.title_name == "-" {
                None
            } else {
                Some(d.title_name.clone())
            }
        });
        app.inflight.set_pending_example_run(PendingExampleRun {
            abs_path: abs_path.display().to_string(),
            target_name: entry.name.clone(),
            package_name,
            kind: entry.kind,
            build_mode: mode,
        });
    }
}

pub(super) fn dispatch_package_action(action: PackageAction, app: &mut App) {
    match action {
        PackageAction::Activate => handle_detail_enter(app),
        PackageAction::Clean => request_clean(app),
    }
}

pub(super) fn dispatch_git_action(action: GitAction, app: &mut App) {
    match action {
        GitAction::Activate => handle_detail_enter(app),
        GitAction::Clean => request_clean(app),
    }
}

pub(super) fn dispatch_targets_action(action: TargetsAction, app: &mut App) {
    match action {
        TargetsAction::Activate => handle_detail_enter(app),
        TargetsAction::ReleaseBuild => handle_target_action(app, BuildMode::Release),
        TargetsAction::Clean => request_clean(app),
    }
}

pub(super) fn dispatch_lang_action(action: LangAction, app: &mut App) {
    match action {
        LangAction::Clean => request_clean(app),
    }
}

pub(super) const fn dispatch_cpu_action(_action: CpuAction, _app: &mut App) {}

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
        CiRunsAction::ToggleView => {
            if let Some(path) = app
                .project_list
                .selected_project_path()
                .map(Path::to_path_buf)
            {
                app.toggle_ci_display_mode_for(&path);
            }
        },
        CiRunsAction::ClearCache => {
            if let Some(path) = app.project_list.selected_ci_path() {
                clear_ci_cache(app, &path);
            }
        },
    }
}

pub(super) fn dispatch_navigation_action(
    action: NavigationAction,
    focused: FocusedPane<AppPaneId>,
    app: &mut App,
) {
    match focused {
        FocusedPane::App(AppPaneId::ProjectList) => navigate_project_list(app, action),
        FocusedPane::App(
            AppPaneId::Package
            | AppPaneId::Lang
            | AppPaneId::Cpu
            | AppPaneId::Git
            | AppPaneId::Targets,
        ) => navigate_detail(app, action),
        FocusedPane::App(AppPaneId::Lints) => navigate_lints(app, action),
        FocusedPane::App(AppPaneId::CiRuns) => navigate_ci_runs(app, action),
        FocusedPane::App(AppPaneId::Output | AppPaneId::Finder) => {},
        FocusedPane::Framework(FrameworkFocusId::Toasts) => navigate_toasts(app, action),
    }
}

fn navigate_project_list(app: &mut App, action: NavigationAction) {
    let include_non_rust = app.config.include_non_rust().includes_non_rust();
    match action {
        NavigationAction::Up => app.project_list.move_up(),
        NavigationAction::Down => app.project_list.move_down(),
        NavigationAction::Home => app.project_list.move_to_top(),
        NavigationAction::End => app.project_list.move_to_bottom(),
        NavigationAction::Right => {
            if !app.expand() {
                app.project_list.move_down();
            }
        },
        NavigationAction::Left => {
            if !app.project_list.collapse(include_non_rust) {
                app.project_list.move_up();
            }
        },
    }
}

fn navigate_detail(app: &mut App, action: NavigationAction) {
    let pane = active_detail_pane(app);
    match action {
        NavigationAction::Up | NavigationAction::Left => pane.up(),
        NavigationAction::Down | NavigationAction::Right => pane.down(),
        NavigationAction::Home => pane.home(),
        NavigationAction::End => pane.end(),
    }
}

const fn navigate_lints(app: &mut App, action: NavigationAction) {
    match action {
        NavigationAction::Up | NavigationAction::Left => app.lint.viewport.up(),
        NavigationAction::Down | NavigationAction::Right => app.lint.viewport.down(),
        NavigationAction::Home => app.lint.viewport.home(),
        NavigationAction::End => app.lint.viewport.end(),
    }
}

const fn navigate_ci_runs(app: &mut App, action: NavigationAction) {
    match action {
        NavigationAction::Up | NavigationAction::Left => app.ci.viewport.up(),
        NavigationAction::Down | NavigationAction::Right => app.ci.viewport.down(),
        NavigationAction::Home => app.ci.viewport.home(),
        NavigationAction::End => app.ci.viewport.end(),
    }
}

fn navigate_toasts(app: &mut App, action: NavigationAction) {
    match action {
        NavigationAction::Up | NavigationAction::Left => app.framework.toasts.viewport.up(),
        NavigationAction::Down | NavigationAction::Right => app.framework.toasts.viewport.down(),
        NavigationAction::Home => app.framework.toasts.viewport.home(),
        NavigationAction::End => {
            let last_index = app.framework.toasts.active_now().len().saturating_sub(1);
            app.framework.toasts.viewport.set_pos(last_index);
        },
    }
}

fn request_clean(app: &mut App) {
    // Gated through App::clean_selection (design plan → gating fix);
    // see src/tui/input.rs for the symmetric site.
    if let Some(selection) = app.project_list.clean_selection() {
        match selection {
            CleanSelection::Project { root } => {
                // Step 6e: fingerprint re-check + possible Verifying
                // popup state, per src/tui/input.rs.
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
        | PaneId::Keymap => &mut app.panes.package.viewport,
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App) {
    if app.focus_is(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.base_focus() == PaneId::Package {
        if let Some(pkg) = app.panes.package.content() {
            let fields = super::package_fields_from_data(pkg);
            if matches!(
                fields.get(app.panes.package.viewport.pos()),
                Some(DetailField::CratesIo)
            ) {
                open_url(&format!("https://crates.io/crates/{}", pkg.title_name));
            }
        }
    } else if let Some(git) = app.panes.git.content() {
        let pos = app.panes.git.viewport.pos();
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
    let bind = KeyBind::new(event.code, event.modifiers);
    if let Some(action) = app.keymap.current().ci_runs.action_for(&bind) {
        match action {
            CiRunsAction::Activate => handle_ci_enter(app),
            CiRunsAction::FetchMore => handle_ci_fetch_more(app),
            CiRunsAction::ToggleView => {
                if let Some(path) = app
                    .project_list
                    .selected_project_path()
                    .map(Path::to_path_buf)
                {
                    app.toggle_ci_display_mode_for(&path);
                }
            },
            CiRunsAction::ClearCache => {
                if let Some(path) = app.project_list.selected_ci_path() {
                    clear_ci_cache(app, &path);
                }
            },
        }
        return;
    }

    // Navigation scope — Phase 16.
    let framework_bind = FrameworkKeyBind {
        code: bind.code,
        mods: bind.modifiers,
    };
    if let Some(nav_scope) = app.framework_keymap.navigation::<AppNavigation>()
        && let Some(nav_action) = nav_scope.action_for(&framework_bind)
    {
        match nav_action {
            NavigationAction::Up => app.ci.viewport.up(),
            NavigationAction::Down => app.ci.viewport.down(),
            NavigationAction::Home => app.ci.viewport.home(),
            NavigationAction::End => app.ci.viewport.end(),
            NavigationAction::Left | NavigationAction::Right => {},
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
    // Use the full cached run list (not branch-filtered) so the cursor is
    // the true oldest cached run. If we used the filtered view, FetchOlder
    // would re-fetch older-than-filtered runs that are already cached on
    // other branches, returning zero "new" runs.
    let oldest_created_at = app
        .project_list
        .selected_project_path()
        .and_then(|path| app.project_list.ci_info_for(path))
        .and_then(|info| info.runs.last().map(|r| r.created_at.clone()));
    // FetchOlder whenever we have a date cursor. Explicit F presses should
    // try to fetch more even if the repo was previously marked exhausted —
    // if it really is exhausted, FetchOlder will re-confirm and the user
    // gets a toast.
    let kind = if oldest_created_at.is_none() {
        CiFetchKind::Sync
    } else {
        CiFetchKind::FetchOlder
    };
    app.inflight.set_pending_ci_fetch(PendingCiFetch {
        project_path: ci_path.display().to_string(),
        ci_run_count: app.config.ci_run_count(),
        oldest_created_at,
        kind,
    });
    let task_id = app
        .framework
        .toasts
        .start_task("Fetching CI", &project_name);
    let item = TrackedItem {
        label:        project_name,
        key:          toast_adapters::path_key(&ci_path),
        started_at:   Some(std::time::Instant::now()),
        completed_at: None,
    };
    app.set_task_tracked_items(task_id, &[item]);
    app.ci.set_fetch_toast(Some(task_id));
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, abs: &Path) {
    if let Some(repo) = app.owner_repo_for_path(abs) {
        let _ = std::fs::remove_dir_all(scan::ci_cache_dir_pub(repo.owner(), repo.repo()));
        scan::clear_exhausted(repo.owner(), repo.repo());
        if let Ok(mut cache) = app.net.github.fetch_cache.lock() {
            cache.remove(&repo);
        }
    }
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

    let _ = input::open_paths_in_editor(
        app.config.editor(),
        std::iter::once(abs_path).chain(log_paths.iter().map(AbsolutePath::as_path)),
    );
}
