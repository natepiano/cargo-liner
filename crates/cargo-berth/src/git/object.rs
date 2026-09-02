//! Resolving revision expressions to commit objects, and reading whether those
//! objects are still present.
//!
//! Every cluster in this module starts from one of these two questions, so the
//! resolution types live here rather than inside any one of them.

use std::fmt::Write;
use std::path::Path;
use std::str::FromStr;

use super::command;
use super::constants::GIT_AMBIGUOUS_OBJECT_SUFFIX;
use super::constants::GIT_BATCH_CHECK_ARG;
use super::constants::GIT_BATCH_CHECK_OBJECT_FORMAT_ARG;
use super::constants::GIT_CAT_FILE_COMMAND;
use super::constants::GIT_COMMIT_OBJECT_TYPE;
use super::constants::GIT_COMMIT_PEEL_SUFFIX;
use super::constants::GIT_EXISTS_ARG;
use super::constants::GIT_MISSING_OBJECT_SUFFIX;
use super::constants::GIT_REV_PARSE_COMMAND;
use super::error;
use super::error::GitError;
use crate::ids::GitObjectId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CommitObjectResolution {
    Resolved(GitObjectId),
    Missing,
    Ambiguous,
    WrongType { object_type: String },
}

#[derive(Clone, Copy)]
pub(super) enum CommitAvailability {
    Available,
    ObjectUnknown,
}

pub(super) fn object_id(repository_root: &Path, revision: &str) -> Result<GitObjectId, GitError> {
    let output = error::completed_git_command(
        command::git_output(repository_root, [GIT_REV_PARSE_COMMAND, revision]).into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_REV_PARSE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let object_id = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    GitObjectId::from_str(object_id.trim()).map_err(GitError::InvalidObjectId)
}

pub(super) fn commit_object_resolutions(
    repository_root: &Path,
    expressions: &[String],
) -> Result<Vec<CommitObjectResolution>, GitError> {
    let input = expressions
        .iter()
        .fold(String::new(), |mut input, expression| {
            let _ = writeln!(input, "{expression}");
            input
        });
    let arguments = [
        GIT_CAT_FILE_COMMAND.to_owned(),
        GIT_BATCH_CHECK_OBJECT_FORMAT_ARG.to_owned(),
    ];
    let output = error::completed_git_command(
        command::git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())
            .into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_CAT_FILE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let resolutions = output_text
        .lines()
        .map(commit_object_resolution)
        .collect::<Result<Vec<_>, _>>()?;
    if resolutions.len() != expressions.len() {
        return Err(GitError::InvalidBatchObjectCount {
            expected: expressions.len(),
            actual:   resolutions.len(),
        });
    }
    Ok(resolutions)
}

pub(super) fn commit_object_resolution(line: &str) -> Result<CommitObjectResolution, GitError> {
    if line.ends_with(GIT_MISSING_OBJECT_SUFFIX) {
        return Ok(CommitObjectResolution::Missing);
    }
    if line.ends_with(GIT_AMBIGUOUS_OBJECT_SUFFIX) {
        return Ok(CommitObjectResolution::Ambiguous);
    }
    let Some((object_id, object_type)) = line.split_once(' ') else {
        return Err(GitError::InvalidBatchObjectLine {
            line: line.to_owned(),
        });
    };
    if object_type != GIT_COMMIT_OBJECT_TYPE {
        return Ok(CommitObjectResolution::WrongType {
            object_type: object_type.to_owned(),
        });
    }
    object_id
        .parse()
        .map(CommitObjectResolution::Resolved)
        .map_err(GitError::InvalidObjectId)
}

pub(super) fn commit_availability(
    repository_root: &Path,
    object_ids: &[GitObjectId],
) -> Result<Vec<CommitAvailability>, GitError> {
    let input = object_ids
        .iter()
        .fold(String::new(), |mut input, object_id| {
            let _ = writeln!(input, "{object_id}{GIT_COMMIT_PEEL_SUFFIX}");
            input
        });
    let arguments = [
        GIT_CAT_FILE_COMMAND.to_owned(),
        GIT_BATCH_CHECK_ARG.to_owned(),
    ];
    let output = error::completed_git_command(
        command::git_output_dynamic_with_input(repository_root, &arguments, input.as_bytes())
            .into(),
    )?;
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: GIT_CAT_FILE_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let output_text = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
    let availability = output_text
        .lines()
        .map(|line| {
            if line.ends_with(GIT_MISSING_OBJECT_SUFFIX) {
                CommitAvailability::ObjectUnknown
            } else {
                CommitAvailability::Available
            }
        })
        .collect::<Vec<_>>();
    if availability.len() != object_ids.len() {
        return Err(GitError::InvalidBatchObjectCount {
            expected: object_ids.len(),
            actual:   availability.len(),
        });
    }
    Ok(availability)
}

/// Return whether git can still read one commit object.
pub(crate) fn commit_is_available(
    repository_root: &Path,
    object_id: &GitObjectId,
) -> Result<bool, GitError> {
    let revision = format!("{object_id}{GIT_COMMIT_PEEL_SUFFIX}");
    let output = command::git_output(
        repository_root,
        [GIT_CAT_FILE_COMMAND, GIT_EXISTS_ARG, &revision],
    )?;
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::CommitObjectResolution;
    use super::commit_object_resolution;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::UNAVAILABLE_OBJECT_ID;
    use crate::ids::GitObjectId;

    #[test]
    fn batch_object_records_preserve_every_resolution_failure() -> FixtureResult {
        let object_id = UNAVAILABLE_OBJECT_ID.parse::<GitObjectId>()?;
        assert_eq!(
            commit_object_resolution(&format!("{object_id} commit"))?,
            CommitObjectResolution::Resolved(object_id.clone())
        );
        assert_eq!(
            commit_object_resolution("missing-expression missing")?,
            CommitObjectResolution::Missing
        );
        assert_eq!(
            commit_object_resolution("ambiguous-expression ambiguous")?,
            CommitObjectResolution::Ambiguous
        );
        assert_eq!(
            commit_object_resolution(&format!("{object_id} tree"))?,
            CommitObjectResolution::WrongType {
                object_type: "tree".to_owned(),
            }
        );
        Ok(())
    }
}
