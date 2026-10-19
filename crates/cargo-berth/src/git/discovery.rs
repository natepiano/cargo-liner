//! The hook directory Git resolves after applying `core.hooksPath`.

use std::path::Path;
use std::path::PathBuf;

use super::command;
use super::constants::GIT_HOOKS_PATH;
use super::constants::GIT_PATH_ARG;
use super::constants::GIT_PATH_FORMAT_ABSOLUTE_ARG;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::error::GitError;

/// Resolve the hook directory Git uses after applying `core.hooksPath`.
pub(crate) fn hooks_directory(repository_root: &Path) -> Result<PathBuf, GitError> {
    let output = command::git_output(
        repository_root,
        [
            GIT_REV_PARSE_COMMAND,
            GIT_PATH_FORMAT_ABSOLUTE_ARG,
            GIT_PATH_ARG,
            GIT_HOOKS_PATH,
        ],
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let hooks_directory = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    Ok(PathBuf::from(hooks_directory.trim()))
}
