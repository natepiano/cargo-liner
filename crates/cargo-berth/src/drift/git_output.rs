//! Read-only git invocations and the NUL-delimited path output they return.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::str::FromStr;

use super::constants::GIT_BINARY;
use super::constants::GIT_NO_OPTIONAL_LOCKS_ARGUMENT;
use super::ordering;
use crate::ids::ReservationScopePath;

/// A git fingerprint could not be computed or interpreted.
#[derive(Debug)]
pub(super) enum DriftFingerprintError {
    Io(std::io::Error),
    CommandFailed { command: String, stderr: String },
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

pub(super) fn run_git(
    repository_root: &Path,
    arguments: &[&str],
) -> Result<Output, DriftFingerprintError> {
    let output = Command::new(GIT_BINARY)
        .arg(GIT_NO_OPTIONAL_LOCKS_ARGUMENT)
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(DriftFingerprintError::CommandFailed {
            command: arguments.join(" "),
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
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
    use super::parse_status_paths;

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
}
