//! Verify that the synced tree and its blobs are available.

use std::collections::BTreeSet;
use std::io::Error;
use std::io::Seek as _;
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::string::FromUtf8Error;

use crate::finding_source::EntryKind;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::TreeBlobs;
use crate::finding_source::TreeEntries;
use crate::finding_source::TreeEntry;

pub(super) struct SourceTree {
    entries: Rc<TreeEntries>,
}

impl SourceTree {
    pub(super) fn read(
        source: &mut FindingSource<'_>,
        commit: &str,
    ) -> Result<Self, SourceReadFailure> {
        let TreeBlobs::Listed(entries) = source.tree(commit)? else {
            return Err(SourceReadFailure::Tree(commit.to_owned()));
        };
        Ok(Self {
            entries: Rc::clone(entries),
        })
    }

    pub(super) fn blobs(&self) -> impl Iterator<Item = (&String, &TreeEntry)> + Clone {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.kind == EntryKind::Blob)
    }

    pub(super) fn blob(&self, path: &str) -> Option<&str> {
        self.entries
            .get(path)
            .filter(|entry| entry.kind == EntryKind::Blob)
            .map(|entry| entry.object.as_str())
    }

    pub(super) fn verify_blobs(&self, repo: &Path) -> Result<(), SourceReadFailure> {
        // The inventory already opens the root tree and all traversed tree objects.
        let blobs = self
            .blobs()
            .map(|(_, entry)| entry.object.as_str())
            .collect::<BTreeSet<_>>();
        let mut input = tempfile::tempfile()?;
        for blob in &blobs {
            writeln!(input, "{blob}")?;
        }
        input.rewind()?;
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "--batch-check=%(objectname) %(objecttype)"])
            .stdin(input)
            .output()?;
        if !output.status.success() {
            return Err(SourceReadFailure::Verify(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let response = String::from_utf8(output.stdout)?;
        let mut lines = response.lines();
        for blob in blobs {
            if lines.next().and_then(|line| line.strip_suffix(" blob")) != Some(blob) {
                return Err(SourceReadFailure::MissingObject(blob.to_owned()));
            }
        }
        if lines.next().is_some() {
            return Err(SourceReadFailure::Verify(
                "unexpected Git object response".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SourceReadFailure {
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    // Keep the existing CLI diagnostics for I/O and decoding failures.
    #[error("cannot prepare or run the regex generator")]
    VerifyProcess(#[from] Error),
    #[error("cannot read the rg configuration or output as UTF-8")]
    VerifyEncoding(#[from] FromUtf8Error),
    #[error("candidate input object is unavailable: {0}")]
    MissingObject(String),
    #[error("cannot verify candidate input objects: {0}")]
    Verify(String),
    #[error("cannot read the candidate tree {0}")]
    Tree(String),
}
