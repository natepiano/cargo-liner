//! Prove whether a finding's rename is present at the synced commit.

mod applied_reader;
mod constants;
mod file;
mod qualified;

use serde::Serialize;

pub(crate) use self::applied_reader::AppliedReader;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::TreeFile;
use crate::rename_admission::MissingPolicyFlag;
use crate::rename_admission::OuterBinding;
use crate::rename_admission::RenameRefusal;

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RenameApplied {
    AppliedInCode,
    NotApplied { path: String },
    NoAppliedCheck { kind: String },
    Blocked(OuterBinding),
    Unproved(RenameRefusal),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AppliedError {
    #[error("cannot read rename applied source {path} at {commit}")]
    Source {
        path:   String,
        commit: String,
        source: FindingSourceFailure,
    },
    #[error("unknown ticket {0}")]
    MissingTicket(String),
    #[error(transparent)]
    MissingPolicy(#[from] MissingPolicyFlag),
}

fn read_source<'a>(
    source: &'a mut FindingSource<'_>,
    commit: &str,
    path: &str,
) -> Result<TreeFile<'a>, AppliedError> {
    source
        .raw_file(commit, path)
        .map_err(|source| AppliedError::Source {
            path: path.to_owned(),
            commit: commit.to_owned(),
            source,
        })
}
