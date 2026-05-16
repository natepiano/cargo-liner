use std::io::Result;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use tui_pane::AppContext;
use tui_pane::FocusedPane;
use tui_pane::FrameworkOverlayId;
use tui_pane::GlobalAction as FrameworkGlobalAction;
use tui_pane::KeyBind;

use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::tui::app::App;
use crate::tui::finder;
use crate::tui::integration::AppGlobalAction;
use crate::tui::integration::AppPaneId;
use crate::tui::settings;

pub fn open_in_editor(app: &mut App) {
    if app.project_list.selected_project_is_deleted() {
        let name = selected_project_display_name(app);
        app.show_timed_warning_toast(
            "Editor unavailable",
            format!("Can't open editor, {name} is deleted"),
        );
        return;
    }
    let Some(selected_path) = app
        .project_list
        .selected_project_path()
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    let abs_path = app
        .project_list
        .iter()
        .find_map(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws))
                if ws.groups().iter().any(|g| {
                    g.members()
                        .iter()
                        .any(|m| m.path() == selected_path.as_path())
                }) =>
            {
                Some(ws.path().to_path_buf())
            },
            _ => None,
        })
        .unwrap_or(selected_path);

    let _ = open_paths_in_editor(app.config.editor(), [&abs_path]);
}

fn open_path_in_editor(editor: &str, path: &Path) -> Result<()> {
    open_paths_in_editor(editor, [path])
}

pub(super) fn framework_overlay_editor_target_path(
    overlay: FrameworkOverlayId,
    config_path: Option<&Path>,
    keymap_path: Option<&Path>,
) -> Option<AbsolutePath> {
    match overlay {
        FrameworkOverlayId::Settings => config_path.map(AbsolutePath::from),
        FrameworkOverlayId::Keymap => keymap_path.map(AbsolutePath::from),
    }
}

pub fn open_paths_in_editor<P>(editor: &str, paths: impl IntoIterator<Item = P>) -> Result<()>
where
    P: AsRef<Path>,
{
    let owned_paths: Vec<PathBuf> = paths
        .into_iter()
        .map(|path| path.as_ref().to_path_buf())
        .collect();
    let paths: Vec<&Path> = owned_paths
        .iter()
        .map(std::path::PathBuf::as_path)
        .collect();
    open_paths_via_editor_command(editor, &paths)
}

fn open_paths_via_editor_command(editor: &str, paths: &[&Path]) -> Result<()> {
    let mut command = std::process::Command::new(editor);
    if let Some(path) = paths.first()
        && let Some(parent) = path.parent()
    {
        command.current_dir(parent);
    }
    for path in paths {
        command.arg(path);
    }
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

pub fn handle_framework_overlay_editor_key(
    app: &mut App,
    bind: &KeyBind,
    overlay: FrameworkOverlayId,
) -> bool {
    let keymap = Rc::clone(&app.framework_keymap);
    let Some(scope) = keymap.globals::<AppGlobalAction>() else {
        return false;
    };
    let Some(AppGlobalAction::OpenEditor) = scope.action_for(bind) else {
        return false;
    };

    let title = match overlay {
        FrameworkOverlayId::Settings => "Settings editor failed",
        FrameworkOverlayId::Keymap => "Keymap editor failed",
    };
    let Some(path) =
        framework_overlay_editor_target_path(overlay, app.config.path(), app.keymap.path())
    else {
        return false;
    };

    if let Err(err) = open_path_in_editor(app.config.editor(), &path) {
        app.show_timed_toast(title, err.to_string());
    }
    true
}

pub fn open_finder(app: &mut App) {
    let (index, col_widths) = finder::build_finder_index(&app.project_list);
    let finder = &mut app.project_list.finder;
    finder.index = index;
    finder.col_widths = col_widths;
    app.overlays.set_finder_return(*app.framework.focused());
    app.set_focus(FocusedPane::App(AppPaneId::Finder));
    app.overlays.open_finder();
    let finder = &mut app.project_list.finder;
    finder.query.clear();
    finder.results.clear();
    finder.total = 0;
    app.overlays.finder_pane.viewport.home();
}

fn shell_escape_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if path.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", path.replace('\'', "'\\''"))
}

pub(super) fn terminal_shell_command(command: &str, selected_path: &Path) -> String {
    command.replace("{path}", &shell_escape_path(selected_path))
}

pub fn open_settings_to_terminal_command(app: &mut App) {
    let keymap = Rc::clone(&app.framework_keymap);
    keymap.dispatch_framework_global(FrameworkGlobalAction::OpenSettings, app);
    settings::focus_terminal_command(app);
}

fn spawn_terminal_command(command: &str, cwd: &Path) -> Result<()> {
    let mut process = if cfg!(windows) {
        let mut process = std::process::Command::new("cmd");
        process.arg("/C").arg(command);
        process
    } else {
        let mut process = std::process::Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    process
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

pub fn open_terminal(app: &mut App) {
    if app.project_list.selected_project_is_deleted() {
        let name = selected_project_display_name(app);
        app.show_timed_warning_toast(
            "Terminal unavailable",
            format!("Can't open terminal, {name} is deleted"),
        );
        return;
    }
    let command = app.config.terminal_command().trim();
    if command.is_empty() {
        open_settings_to_terminal_command(app);
        return;
    }

    let Some(selected_path) = app
        .project_list
        .selected_project_path()
        .map(std::path::Path::to_path_buf)
    else {
        app.show_timed_toast("Terminal", "No selected project path");
        return;
    };

    let command = terminal_shell_command(command, &selected_path);
    if let Err(err) = spawn_terminal_command(&command, &selected_path) {
        app.show_timed_toast("Terminal failed", err.to_string());
    }
}

fn selected_project_display_name(app: &App) -> String {
    if let Some(name) = app.selected_item().and_then(RootItem::name) {
        return name.to_owned();
    }
    app.project_list
        .selected_project_path()
        .and_then(Path::file_name)
        .map_or_else(
            || "selected project".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
}
