//! Read-only git invocations and the NUL-delimited path output they return.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::process::Output;
use std::str::FromStr;

use super::ordering;
use crate::git;
use crate::git::GitCommandExecution;
use crate::git::Reachability;
use crate::ids::GitObjectId;
use crate::ids::ReservationScopePath;

/// Whether one phase-start anchor can delimit incursion commit attribution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum IncursionAttributionAnchorState {
    /// The phase start exists and is reachable from `HEAD`.
    UsableAncestor,
    /// Git does not have the phase-start object.
    ObjectUnknown,
    /// The phase start exists but is not reachable from `HEAD`.
    NotAncestorOfHead,
}

impl From<Reachability> for IncursionAttributionAnchorState {
    fn from(reachability: Reachability) -> Self {
        match reachability {
            Reachability::Ancestor => Self::UsableAncestor,
            Reachability::NotAncestor => Self::NotAncestorOfHead,
            Reachability::ObjectUnknown => Self::ObjectUnknown,
        }
    }
}

/// One commit and its selected paths from the batched incursion log.
pub(super) struct IncursionPathCommit {
    pub(super) commit:  GitObjectId,
    pub(super) subject: String,
    pub(super) paths:   Vec<ReservationScopePath>,
}

/// A git fingerprint could not be computed or interpreted.
#[derive(Debug)]
pub(super) enum DriftFingerprintError {
    Io(std::io::Error),
    CommandFailed { command: String, stderr: String },
    GitOperation(git::GitError),
    MalformedGitOutput(String),
    NonUtf8Path(String),
    InvalidPath(String),
    Reservation(String),
}

impl Display for DriftFingerprintError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not run drift fingerprint: {error}"),
            Self::CommandFailed { command, stderr } => write!(
                formatter,
                "git {command} failed while computing drift: {stderr}"
            ),
            Self::GitOperation(error) => {
                write!(formatter, "git could not compute drift provenance: {error}")
            },
            Self::MalformedGitOutput(diagnostic) => {
                write!(
                    formatter,
                    "git returned malformed drift output: {diagnostic}"
                )
            },
            Self::NonUtf8Path(diagnostic) => {
                write!(
                    formatter,
                    "git reported a non-UTF-8 drift path: {diagnostic}"
                )
            },
            Self::InvalidPath(diagnostic) => {
                write!(
                    formatter,
                    "git reported an invalid drift path: {diagnostic}"
                )
            },
            Self::Reservation(diagnostic) => formatter.write_str(diagnostic),
        }
    }
}

impl std::error::Error for DriftFingerprintError {}

impl From<std::io::Error> for DriftFingerprintError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<git::GitError> for DriftFingerprintError {
    fn from(error: git::GitError) -> Self {
        match error {
            git::GitError::Io(error) => Self::Io(error),
            git::GitError::CommandFailed { command, stderr } => Self::CommandFailed {
                command: command.to_owned(),
                stderr,
            },
            error => Self::GitOperation(error),
        }
    }
}

pub(super) fn run_git(
    repository_root: &Path,
    arguments: &[&str],
) -> Result<Output, DriftFingerprintError> {
    completed_git_output(
        git::execute_read_only_git(repository_root, arguments),
        arguments,
    )
}

pub(super) fn completed_git_output(
    command_execution: GitCommandExecution,
    arguments: &[impl AsRef<str>],
) -> Result<Output, DriftFingerprintError> {
    let output = match command_execution {
        GitCommandExecution::Completed(output) => output,
        GitCommandExecution::CouldNotRun(error) => return Err(DriftFingerprintError::Io(error)),
    };
    if output.status.success() {
        Ok(output)
    } else {
        Err(DriftFingerprintError::CommandFailed {
            command: arguments
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(" "),
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

pub(super) fn parse_incursion_path_log(
    bytes: &[u8],
) -> Result<Vec<IncursionPathCommit>, DriftFingerprintError> {
    let fields = bytes.split(|byte| *byte == b'\0').collect::<Vec<_>>();
    let mut commits = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        while fields.get(index).is_some_and(|field| field.is_empty()) {
            index += 1;
        }
        if index == fields.len() {
            break;
        }
        if fields[index] != git::INCURSION_ATTRIBUTION_RECORD_MARKER.as_bytes() {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "incursion log record omitted its boundary marker".to_owned(),
            ));
        }
        index += 1;
        let Some(commit_field) = fields.get(index) else {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "incursion log ended before its commit".to_owned(),
            ));
        };
        index += 1;
        let Some(subject_field) = fields.get(index) else {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "incursion log ended before its subject".to_owned(),
            ));
        };
        index += 1;
        let commit = std::str::from_utf8(commit_field)
            .map_err(|error| DriftFingerprintError::MalformedGitOutput(error.to_string()))?
            .parse::<GitObjectId>()
            .map_err(|error| DriftFingerprintError::MalformedGitOutput(error.to_string()))?;
        let subject = std::str::from_utf8(subject_field)
            .map_err(|error| DriftFingerprintError::MalformedGitOutput(error.to_string()))?
            .to_owned();
        let mut paths = Vec::new();
        while index < fields.len() {
            if fields[index].is_empty() {
                index += 1;
                if fields.get(index).is_none_or(|field| {
                    *field == git::INCURSION_ATTRIBUTION_RECORD_MARKER.as_bytes()
                }) {
                    break;
                }
                continue;
            }
            let path_field = fields[index];
            let path_field = if paths.is_empty() {
                path_field.strip_prefix(b"\n").unwrap_or(path_field)
            } else {
                path_field
            };
            paths.push(parse_path(path_field)?);
            index += 1;
        }
        commits.push(IncursionPathCommit {
            commit,
            subject,
            paths,
        });
    }
    Ok(commits)
}

pub(super) fn parse_phase_committed_paths(
    bytes: &[u8],
    anchors: &[GitObjectId],
) -> Result<HashMap<GitObjectId, Vec<ReservationScopePath>>, DriftFingerprintError> {
    let anchor_set = anchors.iter().cloned().collect::<HashSet<_>>();
    let mut paths_by_anchor = anchors
        .iter()
        .cloned()
        .map(|anchor| (anchor, Vec::new()))
        .collect::<HashMap<_, _>>();
    let fields = nul_fields(bytes);
    let mut index = 0;
    while index < fields.len() {
        let anchor = std::str::from_utf8(fields[index])
            .map_err(|error| DriftFingerprintError::MalformedGitOutput(error.to_string()))?
            .parse::<GitObjectId>()
            .map_err(|error| DriftFingerprintError::MalformedGitOutput(error.to_string()))?;
        if !anchor_set.contains(&anchor) {
            return Err(DriftFingerprintError::MalformedGitOutput(format!(
                "phase diff named an unrequested anchor {anchor}"
            )));
        }
        index += 1;
        let Some(paths) = paths_by_anchor.get_mut(&anchor) else {
            return Err(DriftFingerprintError::MalformedGitOutput(format!(
                "phase diff repeated anchor {anchor}"
            )));
        };
        if !paths.is_empty() {
            return Err(DriftFingerprintError::MalformedGitOutput(format!(
                "phase diff repeated anchor {anchor}"
            )));
        }
        while index < fields.len() {
            let field = fields[index];
            if std::str::from_utf8(field)
                .ok()
                .and_then(|field| field.parse::<GitObjectId>().ok())
                .is_some_and(|next_anchor| anchor_set.contains(&next_anchor))
            {
                break;
            }
            index += 1;
            let tab_position = field.iter().position(|byte| *byte == b'\t');
            let first_path = tab_position.map(|position| &field[position + 1..]);
            let path = if let Some(path) = first_path {
                path
            } else {
                let Some(path) = fields.get(index) else {
                    return Err(DriftFingerprintError::MalformedGitOutput(
                        "phase name-status output ended before its path".to_owned(),
                    ));
                };
                index += 1;
                path
            };
            paths.push(parse_path(path)?);
        }
        ordering::normalize_paths(paths);
    }
    Ok(paths_by_anchor)
}

pub(super) fn parse_name_status_paths(
    bytes: &[u8],
) -> Result<Vec<ReservationScopePath>, DriftFingerprintError> {
    let fields = nul_fields(bytes);
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status_field = fields[index];
        index += 1;
        let tab_position = status_field.iter().position(|byte| *byte == b'\t');
        let (status, first_path) = tab_position.map_or((status_field, None), |position| {
            (
                &status_field[..position],
                Some(&status_field[position + 1..]),
            )
        });
        let path = if let Some(path) = first_path {
            path
        } else {
            let Some(path) = fields.get(index) else {
                return Err(DriftFingerprintError::MalformedGitOutput(
                    "name-status output ended before its path".to_owned(),
                ));
            };
            index += 1;
            path
        };
        paths.push(parse_path(path)?);
        if matches!(status.first(), Some(b'R' | b'C')) {
            let Some(second_path) = fields.get(index) else {
                return Err(DriftFingerprintError::MalformedGitOutput(
                    "rename or copy status ended before its second path".to_owned(),
                ));
            };
            index += 1;
            paths.push(parse_path(second_path)?);
        }
    }
    ordering::normalize_paths(&mut paths);
    Ok(paths)
}

pub(super) fn parse_status_paths(
    bytes: &[u8],
) -> Result<Vec<ReservationScopePath>, DriftFingerprintError> {
    let fields = nul_fields(bytes);
    let mut paths = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let record = fields[index];
        index += 1;
        if record.len() < 4 || record[2] != b' ' {
            return Err(DriftFingerprintError::MalformedGitOutput(
                "porcelain status record did not contain XY and a path".to_owned(),
            ));
        }
        let status = &record[..2];
        if status != b"??" && status != b"!!" {
            paths.push(parse_path(&record[3..])?);
        }
        if status.iter().any(|column| matches!(column, b'R' | b'C')) {
            let Some(second_path) = fields.get(index) else {
                return Err(DriftFingerprintError::MalformedGitOutput(
                    "porcelain rename or copy ended before its second path".to_owned(),
                ));
            };
            index += 1;
            paths.push(parse_path(second_path)?);
        }
    }
    ordering::normalize_paths(&mut paths);
    Ok(paths)
}

pub(super) fn parse_path_list(
    bytes: &[u8],
) -> Result<Vec<ReservationScopePath>, DriftFingerprintError> {
    let mut paths = nul_fields(bytes)
        .into_iter()
        .map(parse_path)
        .collect::<Result<Vec<_>, _>>()?;
    ordering::normalize_paths(&mut paths);
    Ok(paths)
}

fn nul_fields(bytes: &[u8]) -> Vec<&[u8]> {
    bytes
        .split(|byte| *byte == b'\0')
        .filter(|field| !field.is_empty())
        .collect()
}

fn parse_path(bytes: &[u8]) -> Result<ReservationScopePath, DriftFingerprintError> {
    let path = std::str::from_utf8(bytes)
        .map_err(|error| DriftFingerprintError::NonUtf8Path(error.to_string()))?;
    ReservationScopePath::from_str(path)
        .map_err(|error| DriftFingerprintError::InvalidPath(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;
    use std::process::Output;

    use super::DriftFingerprintError;
    use super::IncursionAttributionAnchorState;
    use super::completed_git_output;
    use super::parse_incursion_path_log;
    use super::parse_phase_committed_paths;
    use super::parse_status_paths;
    use crate::git;
    use crate::git::GitCommandExecution;
    use crate::git::Reachability;
    use crate::ids::GitObjectId;

    const COMMIT_OBJECT_ID: &str = "0000000000000000000000000000000000000000";
    const SECOND_COMMIT_OBJECT_ID: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn phase_diff_parser_keeps_empty_and_pathological_anchor_results_distinct()
    -> Result<(), Box<dyn std::error::Error>> {
        let first = COMMIT_OBJECT_ID.parse::<GitObjectId>()?;
        let second = SECOND_COMMIT_OBJECT_ID.parse::<GitObjectId>()?;
        let output = format!("{COMMIT_OBJECT_ID}\0M\0tab\tname\0A\0line\nname\0M\0café.rs\0");

        let parsed =
            parse_phase_committed_paths(output.as_bytes(), &[first.clone(), second.clone()])?;
        let path_names = parsed
            .into_iter()
            .map(|(anchor, paths)| {
                (
                    anchor,
                    paths.into_iter().map(|path| path.to_string()).collect(),
                )
            })
            .collect::<HashMap<_, Vec<String>>>();

        assert_eq!(
            path_names.get(&first),
            Some(&vec![
                "café.rs".to_owned(),
                "line\nname".to_owned(),
                "tab\tname".to_owned(),
            ])
        );
        assert_eq!(path_names.get(&second), Some(&Vec::new()));
        Ok(())
    }

    #[test]
    fn porcelain_parser_consumes_second_path_for_combined_rename_and_copy_statuses()
    -> Result<(), Box<dyn std::error::Error>> {
        let paths =
            parse_status_paths(b"RM renamed.txt\0original.txt\0CM copied.txt\0source.txt\0")?;
        let path_names = paths.iter().map(ToString::to_string).collect::<Vec<_>>();

        assert_eq!(
            path_names,
            vec!["copied.txt", "original.txt", "renamed.txt", "source.txt"]
        );
        Ok(())
    }

    #[test]
    fn incursion_log_parser_preserves_literal_and_non_ascii_path_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let path_names = [
            ":leading.txt",
            "star*[name].txt",
            "tab\tname.txt",
            "line\nname.txt",
            "café.txt",
        ];
        let mut output = Vec::new();
        output.push(0);
        output.extend_from_slice(git::INCURSION_ATTRIBUTION_RECORD_MARKER.as_bytes());
        output.push(0);
        output.extend_from_slice(COMMIT_OBJECT_ID.as_bytes());
        output.push(0);
        output.extend_from_slice(b"literal paths");
        output.push(0);
        output.push(0);
        for (index, path) in path_names.iter().enumerate() {
            if index > 0 {
                output.push(0);
            }
            output.extend_from_slice(path.as_bytes());
        }
        output.push(0);

        let commits = parse_incursion_path_log(&output)?;
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].commit.to_string(), COMMIT_OBJECT_ID);
        assert_eq!(commits[0].subject, "literal paths");
        assert_eq!(
            commits[0]
                .paths
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            path_names
        );
        Ok(())
    }

    #[test]
    fn anchor_reachability_maps_to_three_attribution_states() {
        assert_eq!(
            IncursionAttributionAnchorState::from(Reachability::Ancestor),
            IncursionAttributionAnchorState::UsableAncestor
        );
        assert_eq!(
            IncursionAttributionAnchorState::from(Reachability::ObjectUnknown),
            IncursionAttributionAnchorState::ObjectUnknown
        );
        assert_eq!(
            IncursionAttributionAnchorState::from(Reachability::NotAncestor),
            IncursionAttributionAnchorState::NotAncestorOfHead
        );
    }

    #[test]
    fn unavailable_and_unsuccessful_git_executions_remain_distinct() {
        let unavailable = completed_git_output(
            GitCommandExecution::CouldNotRun(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "git missing",
            )),
            &["log"],
        );
        assert!(matches!(unavailable, Err(DriftFingerprintError::Io(_))));

        let unsuccessful = completed_git_output(
            GitCommandExecution::Completed(Output {
                status: ExitStatus::from_raw(1 << 8),
                stdout: Vec::new(),
                stderr: b"rejected".to_vec(),
            }),
            &["log"],
        );
        assert!(matches!(
            unsuccessful,
            Err(DriftFingerprintError::CommandFailed { .. })
        ));
    }
}
