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
use crate::ci;
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
    let Some(info) = app.cached_detail.as_ref().map(|c| c.info.clone()) else {
        return;
    };
    let entries = build_target_list(&info);
    if let Some(entry) = entries.get(app.targets_pane.pos())
        && let Some(abs_path) = app.selected_project_path()
    {
        let package_name = if info.name == "-" {
            None
        } else {
            Some(info.name)
        };
        app.pending_example_run = Some(PendingExampleRun {
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
            if let Some(action) = app.current_keymap.targets.action_for(&bind) {
                match action {
                    TargetsAction::Activate => handle_detail_enter(app),
                    TargetsAction::ReleaseBuild => handle_target_action(app, BuildMode::Release),
                    TargetsAction::Clean => request_clean(app),
                }
            }
        },
        PaneId::Git => {
            if let Some(action) = app.current_keymap.git.action_for(&bind) {
                match action {
                    GitAction::Activate => handle_detail_enter(app),
                    GitAction::Clean => request_clean(app),
                }
            }
        },
        _ => {
            // Package pane (default detail pane).
            if let Some(action) = app.current_keymap.package.action_for(&bind) {
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
            .is_some_and(crate::project::ProjectListItem::is_rust)
    {
        app.confirm = Some(ConfirmAction::Clean(path.display().to_string()));
    }
}

/// Return a mutable reference to the pane that owns the cursor for the
/// currently active detail column.
fn active_detail_pane(app: &mut App) -> &mut Pane {
    match app.base_focus() {
        PaneId::Targets => &mut app.targets_pane,
        PaneId::Git => &mut app.git_pane,
        PaneId::Package
        | PaneId::ProjectList
        | PaneId::Lints
        | PaneId::CiRuns
        | PaneId::Toasts
        | PaneId::Search
        | PaneId::Settings
        | PaneId::Finder
        | PaneId::Keymap => &mut app.package_pane,
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App) {
    if app.is_focused(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.base_focus() == PaneId::Package {
        let info = app.cached_detail.as_ref().map(|c| c.info.clone());
        let fields = info.as_ref().map(package_fields).unwrap_or_default();
        match fields.get(app.package_pane.pos()) {
            Some(DetailField::CratesIo) => {
                if let Some(info) = info.as_ref() {
                    open_url(&format!("https://crates.io/crates/{}", info.name));
                }
            },
            Some(field) if field.is_from_cargo_toml() => open_cargo_toml(app),
            _ => {},
        }
    } else if let Some(info) = app.cached_detail.as_ref().map(|c| &c.info)
        && matches!(
            git_fields(info).get(app.git_pane.pos()),
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
        KeyCode::Up => return app.ci_pane.up(),
        KeyCode::Down => return app.ci_pane.down(),
        KeyCode::Home => return app.ci_pane.home(),
        KeyCode::End => return app.ci_pane.end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    let Some(action) = app.current_keymap.ci_runs.action_for(&bind) else {
        return;
    };
    match action {
        CiRunsAction::Activate => handle_ci_enter(app),
        CiRunsAction::ClearCache => {
            if let Some(path) = app.selected_ci_path().map(Path::to_path_buf) {
                clear_ci_cache(app, &path);
            }
        },
    }
}

fn handle_ci_enter(app: &mut App) {
    let ci_state = app.selected_ci_state();
    let run_count = ci_state.map_or(0, |state| state.runs().len());
    let is_fetching = ci_state.is_some_and(CiState::is_fetching);
    let is_exhausted = ci_state.is_some_and(CiState::is_exhausted);

    let cursor_pos = app.ci_pane.pos();
    if cursor_pos < run_count {
        if let Some(runs) = ci_state.map(CiState::runs)
            && let Some(run) = runs.get(cursor_pos)
        {
            open_url(&run.url);
        }
    } else if cursor_pos == run_count
        && !is_fetching
        && let Some(ci_path) = app.selected_ci_path().map(Path::to_path_buf)
    {
        let current_count = u32::try_from(run_count).unwrap_or(u32::MAX);
        let kind = if is_exhausted {
            CiFetchKind::Refresh
        } else {
            CiFetchKind::FetchOlder
        };
        app.pending_ci_fetch = Some(PendingCiFetch {
            project_path: ci_path.display().to_string(),
            current_count,
            kind,
        });
    }
}

pub fn handle_lints_key(app: &mut App, event: &KeyEvent) {
    // Navigation keys stay hardcoded.
    match event.code {
        KeyCode::Up => return app.lint_pane.up(),
        KeyCode::Down => return app.lint_pane.down(),
        KeyCode::Home => return app.lint_pane.home(),
        KeyCode::End => return app.lint_pane.end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::new(event.code, event.modifiers);
    let Some(action) = app.current_keymap.lints.action_for(&bind) else {
        return;
    };
    match action {
        LintsAction::Activate => open_lint_run_output(app),
        LintsAction::ClearHistory => clear_lint_history(app),
    }
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, abs: &Path) {
    if let Some(git) = app.git_info.get(abs)
        && let Some(url) = git.url.as_deref()
        && let Some((owner, repo)) = ci::parse_owner_repo(url)
    {
        let _ =
            std::fs::remove_dir_all(scan::ci_cache_dir_pub(&owner, &repo, git.branch.as_deref()));
    }

    app.ci_state.insert(
        abs.to_path_buf(),
        CiState::Loaded {
            runs:      Vec::new(),
            exhausted: false,
        },
    );
    app.ci_pane.home();
    app.data_generation += 1;
}

fn clear_lint_history(app: &mut App) {
    let Some(abs_path) = app.selected_project_path().map(Path::to_path_buf) else {
        return;
    };
    let project_cache_dir = crate::lint::project_dir(&abs_path);
    let _ = std::fs::remove_dir_all(project_cache_dir);

    app.lint_runs.remove(abs_path.as_path());
    app.lint_pane.home();
    app.refresh_lint_cache_usage_from_disk();
    app.data_generation += 1;
}

fn open_lint_run_output(app: &App) {
    let Some(abs_path) = app.selected_project_path() else {
        return;
    };
    let runs = match app.lint_runs.get(abs_path) {
        Some(runs) if !runs.is_empty() => runs,
        _ => return,
    };
    let Some(run) = runs.get(app.lint_pane.pos()) else {
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

fn open_cargo_toml(app: &App) {
    let Some(abs_path) = app.selected_project_path().map(Path::to_path_buf) else {
        return;
    };
    let project_dir = app
        .projects
        .iter()
        .find_map(|item| match item {
            crate::project::ProjectListItem::Workspace(ws)
                if ws
                    .groups()
                    .iter()
                    .any(|g| g.members().iter().any(|m| m.path() == abs_path.as_path())) =>
            {
                Some(ws.path().to_path_buf())
            },
            _ => None,
        })
        .unwrap_or_else(|| abs_path.clone());

    let cargo_toml = abs_path.join("Cargo.toml");
    let _ = std::process::Command::new(app.editor())
        .arg(&project_dir)
        .arg(&cargo_toml)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
