//! Launch an external editor for arbitrary paths.

use std::io::Result;
use std::path::Path;
use std::path::PathBuf;

/// Open paths in order, using the first path's parent as the working directory.
///
/// # Errors
/// Returns the I/O error when spawning the editor fails.
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
