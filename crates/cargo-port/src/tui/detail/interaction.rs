use std::path::PathBuf;

use crossterm::event::KeyCode;

use super::super::app::App;
use super::super::app::CiState;
use super::super::app::ConfirmAction;
use super::super::types::Pane;
use super::super::types::PaneId;
use super::CiFetchKind;
use super::DetailField;
use super::PendingCiFetch;
use super::PendingExampleRun;
use super::build_detail_info;
use super::build_target_list;
use super::git_fields;
use super::package_fields;
use crate::ci;
use crate::project::ProjectLanguage;
use crate::scan;

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
            abs_path: project.abs_path.clone(),
            target_name: entry.name.clone(),
            package_name: project.name.clone(),
            kind: entry.kind,
            release: matches!(mode, BuildMode::Release),
        });
    }
}

pub fn handle_detail_key(app: &mut App, key: KeyCode) {
    let pane = active_detail_pane(app);

    match key {
        KeyCode::Up => pane.up(),
        KeyCode::Down => pane.down(),
        KeyCode::Home => pane.home(),
        KeyCode::End => pane.end(),
        KeyCode::Enter => handle_detail_enter(app),
        KeyCode::Char('r') => {
            if app.is_focused(PaneId::Targets) {
                handle_target_action(app, BuildMode::Release);
            }
        },
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project()
                && project.is_rust == ProjectLanguage::Rust
            {
                app.confirm = Some(ConfirmAction::Clean(project.abs_path.clone()));
            }
        },
        _ => {},
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
        | PaneId::ScanLog
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
    if app.showing_port_report() {
        match key {
            KeyCode::Up => app.port_report_pane.up(),
            KeyCode::Down => app.port_report_pane.down(),
            KeyCode::Home => app.port_report_pane.home(),
            KeyCode::End => app.port_report_pane.end(),
            KeyCode::Char('p') => app.toggle_bottom_panel(),
            _ => {},
        }
        return;
    }

    let ci_state = app
        .selected_project()
        .and_then(|project| app.ci_state_for(project));
    let run_count = ci_state.map_or(0, |state| state.runs().len());
    let is_fetching = ci_state.is_some_and(CiState::is_fetching);
    let is_exhausted = ci_state.is_some_and(CiState::is_exhausted);

    match key {
        KeyCode::Up => app.ci_pane.up(),
        KeyCode::Down => app.ci_pane.down(),
        KeyCode::Home => app.ci_pane.home(),
        KeyCode::End => app.ci_pane.end(),
        KeyCode::Enter => {
            let cursor_pos = app.ci_pane.pos();
            if cursor_pos < run_count {
                if let Some(runs) = ci_state.map(CiState::runs)
                    && let Some(run) = runs.get(cursor_pos)
                {
                    open_url(&run.url);
                }
            } else if cursor_pos == run_count
                && !is_fetching
                && let Some(project) = app.selected_project()
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
        },
        KeyCode::Char('c') => {
            if let Some(project) = app.selected_project() {
                let path = project.path.clone();
                clear_ci_cache(app, &path);
            }
        },
        KeyCode::Char('p') => app.toggle_bottom_panel(),
        _ => {},
    }
}

/// Clear CI cache for a project and remove its runs from the app.
fn clear_ci_cache(app: &mut App, project_path: &str) {
    if let Some(git) = app.git_info.get(project_path)
        && let Some(url) = git.url.as_deref()
        && let Some((owner, repo)) = ci::parse_owner_repo(url)
    {
        let _ = std::fs::remove_dir_all(scan::repo_cache_dir_pub(&owner, &repo));
    }

    app.ci_state.insert(
        project_path.to_string(),
        CiState::Loaded {
            runs: Vec::new(),
            exhausted: false,
        },
    );
    app.ci_pane.home();
    app.data_generation += 1;
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
