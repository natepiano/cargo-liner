use std::path::Path;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;

use super::CiFetchKind;
use super::DetailField;
use super::PaneId;
use super::PendingCiFetch;
use super::PendingExampleRun;
use super::build_target_list_from_data;
use crate::keymap::CiRunsAction;
use crate::keymap::GitAction;
use crate::keymap::KeyBind;
use crate::keymap::LintsAction;
use crate::keymap::PackageAction;
use crate::keymap::TargetsAction;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::ProjectCiInfo;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::ConfirmAction;
use crate::tui::pane::Pane;

/// Whether to build in release or debug mode.
#[derive(Clone, Copy)]
enum BuildMode {
    Debug,
    Release,
}

fn handle_target_action(app: &mut App, mode: BuildMode) {
    let Some(targets_data) = app.pane_data().targets.clone() else {
        return;
    };
    let entries = build_target_list_from_data(&targets_data);
    if let Some(entry) = entries.get(app.pane_manager().pane(PaneId::Targets).pos())
        && let Some(abs_path) = app.selected_project_path()
    {
        let package_name = app.pane_data().package.as_ref().and_then(|d| {
            if d.title_name == "-" {
                None
            } else {
                Some(d.title_name.clone())
            }
        });
        app.set_pending_example_run(PendingExampleRun {
            abs_path: abs_path.display().to_string(),
            target_name: entry.name.clone(),
            package_name,
            kind: entry.kind,
            release: matches!(mode, BuildMode::Release),
        });
    }
}

pub fn handle_detail_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    {
        let pane = active_detail_pane(app);
        match event.code {
            KeyCode::Up => return pane.up(),
            KeyCode::Down => return pane.down(),
            KeyCode::Home => return pane.home(),
            KeyCode::End => return pane.end(),
            _ => {},
        }
    }

    // Action keys through per-pane keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    match app.base_focus() {
        PaneId::Cpu => {},
        PaneId::Targets => {
            if let Some(action) = app.current_keymap().targets.action_for(&bind) {
                match action {
                    TargetsAction::Activate => handle_detail_enter(app),
                    TargetsAction::ReleaseBuild => handle_target_action(app, BuildMode::Release),
                    TargetsAction::Clean => request_clean(app),
                }
            }
        },
        PaneId::Git => {
            if let Some(action) = app.current_keymap().git.action_for(&bind) {
                match action {
                    GitAction::Activate => handle_detail_enter(app),
                    GitAction::Clean => request_clean(app),
                }
            }
        },
        _ => {
            // Package pane (default detail pane).
            if let Some(action) = app.current_keymap().package.action_for(&bind) {
                match action {
                    PackageAction::Activate => handle_detail_enter(app),
                    PackageAction::Clean => request_clean(app),
                }
            }
        },
    }
}

fn request_clean(app: &mut App) {
    if let Some(path) = app.selected_project_path()
        && app
            .selected_item()
            .is_some_and(crate::project::RootItem::is_rust)
    {
        app.set_confirm(ConfirmAction::Clean(path.into()));
    }
}

/// Return a mutable reference to the pane that owns the cursor for the
/// currently active detail column.
fn active_detail_pane(app: &mut App) -> &mut Pane {
    match app.base_focus() {
        PaneId::Targets => app.pane_manager_mut().pane_mut(PaneId::Targets),
        PaneId::Lang => app.pane_manager_mut().pane_mut(PaneId::Lang),
        PaneId::Cpu => app.pane_manager_mut().pane_mut(PaneId::Cpu),
        PaneId::Git => app.pane_manager_mut().pane_mut(PaneId::Git),
        PaneId::Package
        | PaneId::ProjectList
        | PaneId::Lints
        | PaneId::CiRuns
        | PaneId::Output
        | PaneId::Toasts
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap => app.pane_manager_mut().pane_mut(PaneId::Package),
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App) {
    if app.is_focused(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.base_focus() == PaneId::Package {
        if let Some(pkg) = app.pane_data().package.as_ref() {
            let fields = super::package_fields_from_data(pkg);
            if matches!(
                fields.get(app.pane_manager().pane(PaneId::Package).pos()),
                Some(DetailField::CratesIo)
            ) {
                open_url(&format!("https://crates.io/crates/{}", pkg.title_name));
            }
        }
    } else if let Some(git) = app.pane_data().git.as_ref() {
        let flat_len = super::git_fields_from_data(git).len();
        let pos = app.pane_manager().pane(PaneId::Git).pos();
        if pos >= flat_len
            && let Some(remote) = git.remotes.get(pos - flat_len)
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

pub fn handle_ci_runs_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    match event.code {
        KeyCode::Up => return app.pane_manager_mut().pane_mut(PaneId::CiRuns).up(),
        KeyCode::Down => return app.pane_manager_mut().pane_mut(PaneId::CiRuns).down(),
        KeyCode::Home => return app.pane_manager_mut().pane_mut(PaneId::CiRuns).home(),
        KeyCode::End => return app.pane_manager_mut().pane_mut(PaneId::CiRuns).end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    let Some(action) = app.current_keymap().ci_runs.action_for(&bind) else {
        return;
    };
    match action {
        CiRunsAction::Activate => handle_ci_enter(app),
        CiRunsAction::FetchMore => handle_ci_fetch_more(app),
        CiRunsAction::ToggleView => {
            if let Some(path) = app.selected_project_path().map(Path::to_path_buf) {
                app.toggle_ci_display_mode_for(&path);
            }
        },
        CiRunsAction::ClearCache => {
            if let Some(path) = app.selected_ci_path() {
                clear_ci_cache(app, &path);
            }
        },
    }
}

fn handle_ci_enter(app: &App) {
    let visible_runs = app
        .pane_data()
        .ci
        .as_ref()
        .map(|data| data.runs.clone())
        .unwrap_or_default();
    let cursor_pos = app.pane_manager().pane(PaneId::CiRuns).pos();
    if let Some(run) = visible_runs.get(cursor_pos) {
        open_url(&run.url);
    }
}

fn handle_ci_fetch_more(app: &mut App) {
    let is_fetching = app
        .selected_project_path()
        .is_some_and(|path| app.ci_is_fetching(path));
    if is_fetching {
        return;
    }
    let Some(ci_path) = app.selected_ci_path() else {
        return;
    };
    let project_name = app
        .selected_project_path()
        .and_then(|path| {
            app.projects()
                .iter()
                .find(|item| item.path() == path)
                .and_then(|item| item.name().map(str::to_string))
        })
        .unwrap_or_else(|| crate::project::home_relative_path(&ci_path));
    let is_exhausted = app
        .selected_project_path()
        .is_some_and(|path| app.ci_is_exhausted(path));
    let oldest_created_at = app
        .selected_project_path()
        .map(|path| app.ci_runs_for_display(path))
        .and_then(|runs| runs.last().map(|r| r.created_at.clone()));
    // Sync when exhausted or when there are no cached runs (e.g., after
    // cache clear) — FetchOlder needs a date cursor to work.
    let kind = if is_exhausted || oldest_created_at.is_none() {
        CiFetchKind::Sync
    } else {
        CiFetchKind::FetchOlder
    };
    app.set_pending_ci_fetch(PendingCiFetch {
        project_path: ci_path.display().to_string(),
        ci_run_count: app.ci_run_count(),
        oldest_created_at,
        kind,
    });
    let task_id = app.start_task_toast("Fetching CI", &project_name);
    let item = crate::tui::toasts::TrackedItem {
        label:        project_name,
        key:          ci_path.into(),
        started_at:   Some(std::time::Instant::now()),
        completed_at: None,
    };
    app.set_task_tracked_items(task_id, &[item]);
    app.set_ci_fetch_toast(task_id);
}

pub fn handle_lints_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    match event.code {
        KeyCode::Up => return app.pane_manager_mut().pane_mut(PaneId::Lints).up(),
        KeyCode::Down => return app.pane_manager_mut().pane_mut(PaneId::Lints).down(),
        KeyCode::Home => return app.pane_manager_mut().pane_mut(PaneId::Lints).home(),
        KeyCode::End => return app.pane_manager_mut().pane_mut(PaneId::Lints).end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    let Some(action) = app.current_keymap().lints.action_for(&bind) else {
        return;
    };
    match action {
        LintsAction::Activate => open_lint_run_output(app),
        LintsAction::ClearHistory => clear_lint_history(app),
    }
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, abs: &Path) {
    if let Some(repo) = app.owner_repo_for_path(abs) {
        let _ = std::fs::remove_dir_all(scan::ci_cache_dir_pub(repo.owner(), repo.repo()));
        scan::clear_exhausted(repo.owner(), repo.repo());
        if let Ok(mut cache) = app.repo_fetch_cache().lock() {
            cache.remove(&repo);
        }
    }
    let prev_total = app.ci_data_for(abs).map_or(0, ProjectCiData::github_total);
    app.replace_ci_data_for_path(
        abs,
        ProjectCiData::Loaded(ProjectCiInfo {
            runs:         Vec::new(),
            github_total: prev_total,
            exhausted:    false,
        }),
    );
    app.complete_ci_fetch_for(abs);
    app.pane_manager_mut().pane_mut(PaneId::CiRuns).home();
    app.increment_data_generation();
}

fn clear_lint_history(app: &mut App) {
    if !app.selected_row_owns_lint() {
        return;
    }
    let Some(abs_path) = app.selected_project_path().map(Path::to_path_buf) else {
        return;
    };
    let project_cache_dir = crate::lint::project_dir(&abs_path);
    let _ = std::fs::remove_dir_all(project_cache_dir);

    if let Some(lr) = app.lint_at_path_mut(&abs_path) {
        lr.clear_runs();
    }
    app.pane_manager_mut().pane_mut(PaneId::Lints).home();
    app.focus_pane(PaneId::ProjectList);
    app.refresh_lint_cache_usage_from_disk();
    app.increment_data_generation();
}

fn open_lint_run_output(app: &App) {
    if !app.selected_row_owns_lint() {
        return;
    }
    let Some(abs_path) = app.selected_project_path() else {
        return;
    };
    let Some(runs) = app
        .pane_data()
        .lints
        .as_ref()
        .map(|data| data.runs.as_slice())
    else {
        return;
    };
    if runs.is_empty() {
        return;
    }
    let Some(run) = runs.get(app.pane_manager().pane(PaneId::Lints).pos()) else {
        return;
    };

    let project_cache_dir = crate::lint::project_dir(abs_path);
    let log_paths: Vec<AbsolutePath> = run
        .commands
        .iter()
        .map(|command| AbsolutePath::from(project_cache_dir.join(&command.log_file)))
        .filter(|path| path.exists())
        .collect();

    if log_paths.is_empty() {
        return;
    }

    let _ = crate::tui::input::open_paths_in_editor(
        app.editor(),
        std::iter::once(abs_path).chain(log_paths.iter().map(AbsolutePath::as_path)),
    );
}
