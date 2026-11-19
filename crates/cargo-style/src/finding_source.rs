//! Read the Git objects and line maps for finding proofs.

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

/// Git reads shared by the finding proofs in one request.
pub(crate) struct FindingSource<'a> {
    repo:        &'a Path,
    blob_reader: BlobReader,
    fact_cache:  FactCache,
    commits:     HashMap<String, CommitResolution>,
    trees:       HashMap<String, FindingTree>,
    sources:     HashMap<String, SourceText>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingSourceFailure {
    #[error(transparent)]
    Read(#[from] ReaderError),
    #[error("cannot find source blob {0}")]
    MissingBlob(String),
    #[error("git ls-tree {commit}: no blob for {path}")]
    MissingPath { commit: String, path: String },
    #[error("git grep returned a path that is not UTF-8")]
    SearchPath(#[from] FromUtf8Error),
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

struct FindingTree {
    blobs: HashMap<Vec<u8>, String>,
    paths: TreePathEncoding,
}

enum TreePathEncoding {
    Utf8,
    NonUtf8,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum SearchFiles {
    Rust,
    RustAndMarkdown,
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
        let tree = self.tree(commit)?;
        Ok(match tree.paths {
            TreePathEncoding::Utf8 => tree.blobs.get(path.as_bytes()).map(String::as_str),
            TreePathEncoding::NonUtf8 => None,
        })
    }

    fn tree(&mut self, commit: &str) -> Result<&FindingTree, FindingSourceFailure> {
        Ok(match self.trees.entry(commit.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let trees = self.blob_reader.inventories(&[commit.to_owned()])?;
                let mut paths = TreePathEncoding::Utf8;
                let mut blobs = HashMap::new();
                for (path, _, kind, blob) in trees.into_iter().flat_map(|tree| tree.entries) {
                    if std::str::from_utf8(&path).is_err() {
                        paths = TreePathEncoding::NonUtf8;
                    }
                    if kind == "blob" {
                        blobs.insert(path, blob);
                    }
                }
                entry.insert(FindingTree { blobs, paths })
            },
        })
    }

    pub(crate) fn candidate_blob(
        &mut self,
        commit: &str,
        path: &str,
    ) -> Result<String, FindingSourceFailure> {
        let missing_path = || FindingSourceFailure::MissingPath {
            commit: commit.to_owned(),
            path:   path.to_owned(),
        };
        // Python normalizes carriage returns in the module candidate tree listing.
        if path.contains('\r') {
            return Err(missing_path());
        }
        self.tree(commit)?
            .blobs
            .get(path.as_bytes())
            .cloned()
            .ok_or_else(missing_path)
    }

    pub(crate) fn paths_naming(
        &self,
        commit: &str,
        name: &str,
        files: SearchFiles,
    ) -> Result<Vec<String>, FindingSourceFailure> {
        let mut arguments = vec!["-l", "-z", "-w", "-F", "-e", name, commit, "--", "*.rs"];
        if files == SearchFiles::RustAndMarkdown {
            arguments.push("*.md");
        }
        let output = git(self.repo, "grep", &arguments)?;
        let paths =
            if output.status.code() == Some(1) && !grep_reported_missing_blob(&output.stderr) {
                output.stdout
            } else {
                successful("grep", output)?
            };
        let prefix = format!("{commit}:");
        paths
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                let path = path.strip_prefix(prefix.as_bytes()).unwrap_or(path);
                Ok(String::from_utf8(path.to_vec())?)
            })
            .collect()
    }

    pub(crate) fn line(
        &mut self,
        blob: &str,
        line: usize,
    ) -> Result<SiteLine<'_>, FindingSourceFailure> {
        Ok(match self.source(blob)? {
            SourceText::Utf8(source) => source.line(line),
            SourceText::Lossy(_) => SiteLine::Unavailable,
        })
    }

    pub(crate) fn text(&mut self, blob: &str) -> Result<&str, FindingSourceFailure> {
        Ok(match self.source(blob)? {
            SourceText::Utf8(source) => source.text(),
            SourceText::Lossy(text) => text,
        })
    }

    fn source(&mut self, blob: &str) -> Result<&SourceText, FindingSourceFailure> {
        Ok(match self.sources.entry(blob.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let source = match self.blob_reader.source(blob, &mut self.fact_cache)? {
                    BlobSource::Utf8(text) => SourceText::Utf8(text.into()),
                    BlobSource::NotUtf8(text) => SourceText::Lossy(text),
                    BlobSource::Missing => {
                        return Err(FindingSourceFailure::MissingBlob(blob.to_owned()));
                    },
                };
                entry.insert(source)
            },
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
    Lossy(String),
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
    Err(git_exit(command, &output))
}

fn grep_reported_missing_blob(stderr: &[u8]) -> bool {
    stderr.split(|&byte| byte == b'\n').any(|line| {
        line.starts_with(b"error: ")
            && line
                .windows(b": unable to read ".len())
                .any(|window| window == b": unable to read ")
    })
}

fn git_exit(command: &'static str, output: &Output) -> FindingSourceFailure {
    FindingSourceFailure::GitExit {
        command,
        status: output.status,
        detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
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
