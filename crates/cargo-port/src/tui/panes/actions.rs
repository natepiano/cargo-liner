use std::path::Path;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;

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
use crate::tui::input;
use crate::tui::pane::Viewport;
use crate::tui::toasts::TrackedItem;

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
    match app.focus.base() {
        PaneId::Cpu => {},
        PaneId::Targets => {
            if let Some(action) = app.keymap.current().targets.action_for(&bind) {
                match action {
                    TargetsAction::Activate => handle_detail_enter(app),
                    TargetsAction::ReleaseBuild => handle_target_action(app, BuildMode::Release),
                    TargetsAction::Clean => request_clean(app),
                }
            }
        },
        PaneId::Git => {
            if let Some(action) = app.keymap.current().git.action_for(&bind) {
                match action {
                    GitAction::Activate => handle_detail_enter(app),
                    GitAction::Clean => request_clean(app),
                }
            }
        },
        _ => {
            // Package pane (default detail pane).
            if let Some(action) = app.keymap.current().package.action_for(&bind) {
                match action {
                    PackageAction::Activate => handle_detail_enter(app),
                    PackageAction::Clean => request_clean(app),
                }
            }
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
    match app.focus.base() {
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
    if app.focus.is(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.focus.base() == PaneId::Package {
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

pub fn handle_ci_runs_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    match event.code {
        KeyCode::Up => return app.ci.viewport.up(),
        KeyCode::Down => return app.ci.viewport.down(),
        KeyCode::Home => return app.ci.viewport.home(),
        KeyCode::End => return app.ci.viewport.end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    let Some(action) = app.keymap.current().ci_runs.action_for(&bind) else {
        return;
    };
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
    let task_id = app.toasts.start_task("Fetching CI", &project_name);
    let item = TrackedItem {
        label:        project_name,
        key:          ci_path.into(),
        started_at:   Some(std::time::Instant::now()),
        completed_at: None,
    };
    app.set_task_tracked_items(task_id, &[item]);
    app.ci.set_fetch_toast(Some(task_id));
}

pub fn handle_lints_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    match event.code {
        KeyCode::Up => return app.lint.viewport.up(),
        KeyCode::Down => return app.lint.viewport.down(),
        KeyCode::Home => return app.lint.viewport.home(),
        KeyCode::End => return app.lint.viewport.end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    let Some(action) = app.keymap.current().lints.action_for(&bind) else {
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
    app.focus.set(PaneId::ProjectList);
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
