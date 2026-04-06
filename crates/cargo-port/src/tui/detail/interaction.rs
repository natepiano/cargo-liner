use std::path::PathBuf;

use crossterm::event::KeyCode;

use super::CiFetchKind;
use super::DetailField;
use super::PendingCiFetch;
use super::PendingExampleRun;
use super::build_detail_info;
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
use crate::project::ProjectLanguage;
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
    let Some(project) = app.selected_project() else {
        return;
    };
    let info = build_detail_info(app, project);
    let entries = build_target_list(&info);
    if let Some(entry) = entries.get(app.targets_pane.pos())
        && let Some(project) = app.selected_project()
    {
        app.pending_example_run = Some(PendingExampleRun {
            abs_path:     project.abs_path.clone(),
            target_name:  entry.name.clone(),
            package_name: project.name.clone(),
            kind:         entry.kind,
            release:      matches!(mode, BuildMode::Release),
        });
    }
}

pub fn handle_detail_key(app: &mut App, key: KeyCode) {
    // Navigation keys stay hardcoded.
    {
        let pane = active_detail_pane(app);
        match key {
            KeyCode::Up => return pane.up(),
            KeyCode::Down => return pane.down(),
            KeyCode::Home => return pane.home(),
            KeyCode::End => return pane.end(),
            _ => {},
        }
    }

    // Action keys through per-pane keymap.
    let bind = KeyBind::plain(key);
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
    if let Some(project) = app.selected_project()
        && project.is_rust == ProjectLanguage::Rust
    {
        app.confirm = Some(ConfirmAction::Clean(project.abs_path.clone()));
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
        | PaneId::CiRuns
        | PaneId::Toasts
        | PaneId::Search
        | PaneId::Settings
        | PaneId::Finder => &mut app.package_pane,
    }
}

/// Handle the Enter key in the detail panel.
fn handle_detail_enter(app: &mut App) {
    if app.is_focused(PaneId::Targets) {
        handle_target_action(app, BuildMode::Debug);
    } else if app.base_focus() == PaneId::Package {
        let info = app
            .selected_project()
            .map(|project| build_detail_info(app, project));
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
    } else if let Some(info) = app
        .selected_project()
        .map(|project| build_detail_info(app, project))
        && matches!(
            git_fields(&info).get(app.git_pane.pos()),
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

pub fn handle_ci_runs_key(app: &mut App, key: KeyCode) {
    if app.showing_lints() {
        handle_lints_key(app, key);
        return;
    }

    // Navigation keys stay hardcoded.
    match key {
        KeyCode::Up => return app.ci_pane.up(),
        KeyCode::Down => return app.ci_pane.down(),
        KeyCode::Home => return app.ci_pane.home(),
        KeyCode::End => return app.ci_pane.end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::plain(key);
    let Some(action) = app.current_keymap.ci_runs.action_for(&bind) else {
        return;
    };
    match action {
        CiRunsAction::Activate => handle_ci_enter(app),
        CiRunsAction::ClearCache => {
            if let Some(project) = app.selected_ci_project() {
                let path = project.path.clone();
                clear_ci_cache(app, &path);
            }
        },
        CiRunsAction::TogglePanel => app.toggle_bottom_panel(),
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
        && let Some(project) = app.selected_ci_project()
    {
        let current_count = u32::try_from(run_count).unwrap_or(u32::MAX);
        let kind = if is_exhausted {
            CiFetchKind::Refresh
        } else {
            CiFetchKind::FetchOlder
        };
        app.pending_ci_fetch = Some(PendingCiFetch {
            project_path: project.path.clone(),
            current_count,
            kind,
        });
    }
}

fn handle_lints_key(app: &mut App, key: KeyCode) {
    // Navigation keys stay hardcoded.
    match key {
        KeyCode::Up => return app.lint_pane.up(),
        KeyCode::Down => return app.lint_pane.down(),
        KeyCode::Home => return app.lint_pane.home(),
        KeyCode::End => return app.lint_pane.end(),
        _ => {},
    }

    // Action keys through keymap.
    let bind = KeyBind::plain(key);
    let Some(action) = app.current_keymap.lints.action_for(&bind) else {
        return;
    };
    match action {
        LintsAction::Activate => open_lint_run_output(app),
        LintsAction::ClearHistory => clear_lint_history(app),
        LintsAction::TogglePanel => app.toggle_bottom_panel(),
    }
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, project_path: &str) {
    if let Some(git) = app.git_info.get(project_path)
        && let Some(url) = git.url.as_deref()
        && let Some((owner, repo)) = ci::parse_owner_repo(url)
    {
        let _ =
            std::fs::remove_dir_all(scan::ci_cache_dir_pub(&owner, &repo, git.branch.as_deref()));
    }

    app.ci_state.insert(
        project_path.to_string(),
        CiState::Loaded {
            runs:      Vec::new(),
            exhausted: false,
        },
    );
    app.ci_pane.home();
    app.data_generation += 1;
}

fn clear_lint_history(app: &mut App) {
    let Some(project) = app.selected_project() else {
        return;
    };
    let project_cache_dir = crate::lint::project_dir(std::path::Path::new(&project.abs_path));
    let _ = std::fs::remove_dir_all(project_cache_dir);

    let path = project.path.clone();
    app.lint_runs.remove(&path);
    app.lint_pane.home();
    app.refresh_lint_cache_usage_from_disk();
    app.data_generation += 1;
}

fn open_lint_run_output(app: &App) {
    let Some(project) = app.selected_project() else {
        return;
    };
    let runs = match app.lint_runs.get(&project.path) {
        Some(runs) if !runs.is_empty() => runs,
        _ => return,
    };
    let Some(run) = runs.get(app.lint_pane.pos()) else {
        return;
    };

    let project_cache_dir = crate::lint::project_dir(std::path::Path::new(&project.abs_path));
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
    cmd.arg(&project.abs_path);
    for path in &log_paths {
        cmd.arg(path);
    }
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn open_cargo_toml(app: &App) {
    let Some(project) = app.selected_project() else {
        return;
    };
    let project_dir = app
        .nodes
        .iter()
        .find(|node| {
            node.groups.iter().any(|group| {
                group
                    .members
                    .iter()
                    .any(|member| member.path == project.path)
            })
        })
        .map_or_else(
            || project.abs_path.clone(),
            |node| node.project.abs_path.clone(),
        );

    let cargo_toml = PathBuf::from(&project.abs_path).join("Cargo.toml");
    let _ = std::process::Command::new(app.editor())
        .arg(&project_dir)
        .arg(&cargo_toml)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
