//! Read the Git objects and line map for finding anchors.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::Error;
use std::path::Path;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

use crate::blob_reader::BlobReader;
use crate::error::ReaderError;
use crate::fact_store::FactCache;
use crate::facts::BlobSource;
use crate::source_lines;
use crate::source_lines::LineContinuity;
use crate::source_lines::SiteLine;
use crate::source_lines::SourceLines;

/// Git reads shared by the finding anchors in one request.
pub(crate) struct FindingSource<'a> {
    repo:        &'a Path,
    blob_reader: BlobReader,
    fact_cache:  FactCache,
    commits:     HashMap<String, CommitResolution>,
    trees:       HashMap<String, TreeBlobs>,
    sources:     HashMap<String, SourceText>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingSourceFailure {
    #[error(transparent)]
    Read(#[from] ReaderError),
    #[error("cannot find source blob {0}")]
    MissingBlob(String),
    #[error("cannot run git {command}")]
    GitStart {
        command: &'static str,
        #[source]
        source:  Error,
    },
    #[error("git {command} failed ({status}): {detail}")]
    GitExit {
        command: &'static str,
        status:  ExitStatus,
        detail:  String,
    },
    #[error("git {command} returned text that is not UTF-8")]
    GitText {
        command: &'static str,
        #[source]
        source:  Utf8Error,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommitResolution {
    Resolved(String),
    Unresolved,
}

enum TreeBlobs {
    Listed(HashMap<String, String>),
    Undecodable,
}

impl<'a> FindingSource<'a> {
    pub(crate) fn open(repo: &'a Path) -> Result<Self, FindingSourceFailure> {
        let blob_reader = BlobReader::open(repo)?;
        let fact_cache = blob_reader.existing_fact_cache();
        Ok(Self {
            repo,
            blob_reader,
            fact_cache,
            commits: HashMap::new(),
            trees: HashMap::new(),
            sources: HashMap::new(),
        })
    }

    pub(crate) fn resolve_commit(
        &mut self,
        revision: &str,
    ) -> Result<&CommitResolution, FindingSourceFailure> {
        let commit = match self.commits.entry(revision.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let peel = format!("{revision}^{{commit}}");
                let output = git(
                    self.repo,
                    "rev-parse",
                    &["--verify", "--quiet", "--end-of-options", &peel],
                )?;
                let commit = if output.status.code() == Some(1) {
                    CommitResolution::Unresolved
                } else {
                    CommitResolution::Resolved(
                        git_text("rev-parse", &successful("rev-parse", output)?)?
                            .trim()
                            .to_owned(),
                    )
                };
                entry.insert(commit)
            },
        };
        Ok(commit)
    }

    pub(crate) fn blob(
        &mut self,
        commit: &str,
        path: &str,
    ) -> Result<Option<&str>, FindingSourceFailure> {
        let blobs = match self.trees.entry(commit.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let trees = self.blob_reader.inventories(&[commit.to_owned()])?;
                let blobs = trees
                    .into_iter()
                    .flat_map(|tree| tree.entries)
                    .try_fold(HashMap::new(), |mut blobs, (path, _, kind, blob)| {
                        let path = String::from_utf8(path)?;
                        if kind == "blob" {
                            blobs.insert(path, blob);
                        }
                        Ok::<_, FromUtf8Error>(blobs)
                    })
                    .map_or(TreeBlobs::Undecodable, TreeBlobs::Listed);
                entry.insert(blobs)
            },
        };
        Ok(match blobs {
            TreeBlobs::Listed(blobs) => blobs.get(path).map(String::as_str),
            TreeBlobs::Undecodable => None,
        })
    }

    pub(crate) fn line(
        &mut self,
        blob: &str,
        line: usize,
    ) -> Result<SiteLine<'_>, FindingSourceFailure> {
        let source = match self.sources.entry(blob.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let source = match self.blob_reader.source(blob, &mut self.fact_cache)? {
                    BlobSource::Utf8(text) => SourceText::Utf8(text.into()),
                    BlobSource::NotUtf8 => SourceText::NotUtf8,
                    BlobSource::Missing => {
                        return Err(FindingSourceFailure::MissingBlob(blob.to_owned()));
                    },
                };
                entry.insert(source)
            },
        };
        Ok(match source {
            SourceText::Utf8(source) => source.line(line),
            SourceText::NotUtf8 => SiteLine::Unavailable,
        })
    }

    pub(crate) fn line_continuity(
        &self,
        base: &str,
        checkpoint: &str,
        path: &str,
        line: usize,
    ) -> Result<LineContinuity, FindingSourceFailure> {
        let ancestor = git(
            self.repo,
            "merge-base",
            &["--is-ancestor", base, checkpoint],
        )?;
        if ancestor.status.code() == Some(1) {
            return Ok(LineContinuity::Unproved);
        }
        let _ = successful("merge-base", ancestor)?;
        let diff = git(
            self.repo,
            "diff",
            &[
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
                "--diff-algorithm=myers",
                "--no-indent-heuristic",
                "--unified=0",
                base,
                checkpoint,
                "--",
                path,
            ],
        )?;
        let diff = successful("diff", diff)?;
        Ok(source_lines::continuity(git_text("diff", &diff)?, line))
    }
}

enum SourceText {
    Utf8(SourceLines),
    NotUtf8,
}

fn git(
    repo: &Path,
    command: &'static str,
    arguments: &[&str],
) -> Result<Output, FindingSourceFailure> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg(command)
        .args(arguments)
        .output()
        .map_err(|source| FindingSourceFailure::GitStart { command, source })
}

fn successful(command: &'static str, output: Output) -> Result<Vec<u8>, FindingSourceFailure> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(FindingSourceFailure::GitExit {
        command,
        status: output.status,
        detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

fn git_text<'a>(command: &'static str, bytes: &'a [u8]) -> Result<&'a str, FindingSourceFailure> {
    std::str::from_utf8(bytes).map_err(|source| FindingSourceFailure::GitText { command, source })
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use cargo_berth_test_support::fixture_tempdir;

    use super::*;

    #[test]
    fn a_missing_commit_has_an_unresolved_state() -> Result<(), Box<dyn Error>> {
        let directory = fixture_tempdir("cargo-style-unresolved-commit-");
        let _ = gix::init(directory.path())?;
        let mut source = FindingSource::open(directory.path())?;
        assert_eq!(
            source.resolve_commit("refs/heads/missing")?,
            &CommitResolution::Unresolved
        );
        Ok(())
    }
}
