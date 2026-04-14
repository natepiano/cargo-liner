use std::path::Path;
use std::path::PathBuf;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;

use super::CiFetchKind;
use super::DetailField;
use super::PendingCiFetch;
use super::PendingExampleRun;
use super::build_target_list;
use super::git_fields;
use super::package_fields;
use crate::keymap::CiRunsAction;
use crate::keymap::GitAction;
use crate::keymap::KeyBind;
use crate::keymap::LintsAction;
use crate::keymap::PackageAction;
use crate::keymap::TargetsAction;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::CiState;
use crate::tui::app::ConfirmAction;
use crate::tui::types::Pane;
use crate::tui::types::PaneId;

/// Whether to build in release or debug mode.
#[derive(Clone, Copy)]
enum BuildMode {
    Debug,
    Release,
}

fn handle_target_action(app: &mut App, mode: BuildMode) {
    let Some(info) = app.cached_detail().map(|c| c.info.clone()) else {
        return;
    };
    let entries = build_target_list(&info);
    if let Some(entry) = entries.get(app.targets_pane().pos())
        && let Some(abs_path) = app.selected_project_path()
    {
        let package_name = if info.name == "-" {
            None
        } else {
            Some(info.name)
        };
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
        PaneId::Targets => app.targets_pane_mut(),
        PaneId::Git => app.git_pane_mut(),
        PaneId::Package
        | PaneId::ProjectList
        | PaneId::Lints
        | PaneId::CiRuns
        | PaneId::Output
        | PaneId::Toasts
        | PaneId::Search
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap => app.package_pane_mut(),
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App) {
    if app.is_focused(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.base_focus() == PaneId::Package {
        let info = app.cached_detail().map(|c| c.info.clone());
        let fields = info.as_ref().map(package_fields).unwrap_or_default();
        if matches!(
            fields.get(app.package_pane().pos()),
            Some(DetailField::CratesIo)
        ) && let Some(info) = info.as_ref()
        {
            open_url(&format!("https://crates.io/crates/{}", info.name));
        }
    } else if let Some(info) = app.cached_detail().map(|c| &c.info)
        && matches!(
            git_fields(info).get(app.git_pane().pos()),
            Some(DetailField::Repo)
        )
        && let Some(url) = info.git_url.as_deref()
    {
        open_url(url);
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
        KeyCode::Up => return app.ci_pane_mut().up(),
        KeyCode::Down => return app.ci_pane_mut().down(),
        KeyCode::Home => return app.ci_pane_mut().home(),
        KeyCode::End => return app.ci_pane_mut().end(),
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
        .selected_project_path()
        .map_or_else(Vec::new, |path| app.ci_runs_for_display(path));
    let cursor_pos = app.ci_pane().pos();
    if let Some(run) = visible_runs.get(cursor_pos) {
        open_url(&run.url);
    }
}

fn handle_ci_fetch_more(app: &mut App) {
    let ci_state = app.selected_ci_state();
    if ci_state.is_some_and(CiState::is_fetching) {
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
    let is_exhausted = ci_state.is_some_and(CiState::is_exhausted);
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
        KeyCode::Up => return app.lint_pane_mut().up(),
        KeyCode::Down => return app.lint_pane_mut().down(),
        KeyCode::Home => return app.lint_pane_mut().home(),
        KeyCode::End => return app.lint_pane_mut().end(),
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
    let owner_paths = app
        .owner_repo_for_path(abs)
        .map(|repo| {
            let _ = std::fs::remove_dir_all(scan::ci_cache_dir_pub(repo.owner(), repo.repo()));
            scan::clear_exhausted(repo.owner(), repo.repo());
            if let Ok(mut cache) = app.repo_fetch_cache().lock() {
                cache.remove(&repo);
            }
            app.owner_paths_for_repo(&repo)
        })
        .filter(|paths| !paths.is_empty())
        .unwrap_or_else(|| vec![crate::project::AbsolutePath::from(abs)]);

    let prev_totals: Vec<_> = owner_paths
        .iter()
        .map(|p| {
            app.ci_state_mut()
                .get(p.as_path())
                .map_or(0, CiState::github_total)
        })
        .collect();
    for (owner_path, prev_total) in owner_paths.iter().zip(prev_totals) {
        app.ci_state_mut().insert(
            owner_path.clone(),
            CiState::Loaded {
                runs:         Vec::new(),
                exhausted:    false,
                github_total: prev_total,
            },
        );
    }
    app.ci_pane_mut().home();
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
    app.lint_pane_mut().home();
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
    let Some(lr) = app.lint_at_path(abs_path) else {
        return;
    };
    let runs = lr.runs();
    if runs.is_empty() {
        return;
    }
    let Some(run) = runs.get(app.lint_pane().pos()) else {
        return;
    };

    let project_cache_dir = crate::lint::project_dir(abs_path);
    let log_paths: Vec<PathBuf> = run
        .commands
        .iter()
        .map(|command| project_cache_dir.join(&command.log_file))
        .filter(|path| path.exists())
        .collect();

    if log_paths.is_empty() {
        return;
    }

    let mut cmd = std::process::Command::new(app.editor());
    cmd.arg(abs_path);
    for path in &log_paths {
        cmd.arg(path);
    }
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
