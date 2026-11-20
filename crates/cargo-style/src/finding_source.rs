//! Read the Git objects and line maps for finding proofs.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::Error;
use std::path::Path;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::rc::Rc;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

use toml::de::Error as TomlError;

use crate::blob_reader::BlobReader;
use crate::error::ReaderError;
use crate::fact_store::FactCache;
use crate::facts::BlobSource;
use crate::facts::BlobSyntax;
use crate::source_lines::LineContinuity;
use crate::source_lines::LineMapping;
use crate::source_lines::PatchLineMap;
use crate::source_lines::SiteLine;
use crate::source_lines::SourceLines;

/// Git reads shared by the finding proofs in one request.
pub(crate) struct FindingSource<'a> {
    repo:        &'a Path,
    blob_reader: BlobReader,
    fact_cache:  FactCache,
    commits:     HashMap<String, CommitResolution>,
    trees:       HashMap<String, TreeBlobs>,
    sources:     HashMap<String, SourceText>,
    raw_blobs:   HashMap<String, Vec<u8>>,
    ancestors:   HashMap<(String, String), bool>,
    patches:     HashMap<(String, String, String), PatchLineMap>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingSourceFailure {
    #[error(transparent)]
    Read(#[from] ReaderError),
    #[error("cannot parse committed TOML at {path}")]
    InvalidTipToml {
        path:   String,
        #[source]
        source: Box<TomlError>,
    },
    #[error("the synced tree at {0} contains a path that is not UTF-8")]
    UndecodableSyncedTree(String),
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

pub(crate) enum TreeBlobs {
    Listed(Rc<TreeEntries>),
    Undecodable(BTreeMap<Vec<u8>, String>),
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) enum SearchFiles {
    Rust,
    RustAndMarkdown,
}

pub(crate) type TreeEntries = BTreeMap<String, TreeEntry>;

#[derive(PartialEq, Eq)]
pub(crate) struct TreeEntry {
    pub(crate) mode:   String,
    pub(crate) object: String,
    pub(crate) kind:   EntryKind,
}

#[derive(PartialEq, Eq)]
pub(crate) enum EntryKind {
    Blob,
    Submodule,
    Directory,
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
            raw_blobs: HashMap::new(),
            ancestors: HashMap::new(),
            patches: HashMap::new(),
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
        Ok(match self.tree(commit)? {
            TreeBlobs::Listed(entries) => entries
                .get(path)
                .filter(|entry| entry.kind == EntryKind::Blob)
                .map(|entry| entry.object.as_str()),
            TreeBlobs::Undecodable(_) => None,
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
        match self.tree(commit)? {
            TreeBlobs::Listed(entries) => entries
                .get(path)
                .filter(|entry| entry.kind == EntryKind::Blob)
                .map(|entry| entry.object.clone()),
            TreeBlobs::Undecodable(blobs) => blobs.get(path.as_bytes()).cloned(),
        }
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

    pub(crate) fn raw_file(
        &mut self,
        commit: &str,
        path: &str,
    ) -> Result<TreeFile<'_>, FindingSourceFailure> {
        let TreeBlobs::Listed(entries) = self.tree(commit)? else {
            return Err(FindingSourceFailure::UndecodableSyncedTree(
                commit.to_owned(),
            ));
        };
        match entries.get(path) {
            Some(entry) if entry.kind == EntryKind::Blob => {
                let blob = entry.object.clone();
                let bytes = match self.raw_blobs.entry(blob) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        let bytes = self.blob_reader.raw_bytes(entry.key())?.ok_or_else(|| {
                            FindingSourceFailure::MissingBlob(entry.key().clone())
                        })?;
                        entry.insert(bytes)
                    },
                };
                Ok(TreeFile::Bytes(bytes))
            },
            Some(entry) if entry.kind == EntryKind::Directory => {
                let prefix = format!("{path}/");
                let has_descendant = entries
                    .range(prefix.clone()..)
                    .take_while(|(name, _)| name.starts_with(&prefix))
                    .any(|(_, entry)| entry.kind != EntryKind::Directory);
                Ok(if has_descendant {
                    TreeFile::Folder
                } else {
                    TreeFile::Missing
                })
            },
            _ => Ok(TreeFile::Missing),
        }
    }

    pub(crate) fn tree(&mut self, commit: &str) -> Result<&TreeBlobs, FindingSourceFailure> {
        Ok(match self.trees.entry(commit.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let trees = self.blob_reader.inventories(&[commit.to_owned()])?;
                let undecodable = trees
                    .iter()
                    .flat_map(|tree| &tree.entries)
                    .any(|(path, ..)| std::str::from_utf8(path).is_err());
                let entries = trees.into_iter().flat_map(|tree| tree.entries);
                let tree = if undecodable {
                    TreeBlobs::Undecodable(
                        entries
                            .filter_map(|(path, _, kind, object)| {
                                (kind == "blob").then_some((path, object))
                            })
                            .collect(),
                    )
                } else {
                    let entries = entries
                        .map(|(path, mode, kind, object)| {
                            let kind = match kind.as_str() {
                                "tree" => EntryKind::Directory,
                                "commit" => EntryKind::Submodule,
                                _ => EntryKind::Blob,
                            };
                            Ok((String::from_utf8(path)?, TreeEntry { mode, object, kind }))
                        })
                        .collect::<Result<TreeEntries, FromUtf8Error>>()?;
                    TreeBlobs::Listed(Rc::new(entries))
                };
                entry.insert(tree)
            },
        })
    }

    pub(crate) fn line(
        &mut self,
        blob: &str,
        line: usize,
    ) -> Result<SiteLine<'_>, FindingSourceFailure> {
        Ok(match self.source(blob)? {
            SourceText::Utf8(source) => source.lines.line(line),
            SourceText::NotUtf8(_) => SiteLine::Unavailable,
        })
    }

    pub(crate) fn source(&mut self, blob: &str) -> Result<&SourceText, FindingSourceFailure> {
        Ok(match self.sources.entry(blob.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let source = match self.blob_reader.source(blob, &mut self.fact_cache)? {
                    BlobSource::Utf8 { text, syntax } => SourceText::Utf8(Rc::new(Utf8Source {
                        lines: text.into(),
                        syntax,
                    })),
                    BlobSource::NotUtf8(text) => SourceText::NotUtf8(text),
                    BlobSource::Missing => {
                        return Err(FindingSourceFailure::MissingBlob(blob.to_owned()));
                    },
                };
                entry.insert(source)
            },
        })
    }

    pub(crate) fn text(&mut self, blob: &str) -> Result<&str, FindingSourceFailure> {
        Ok(match self.source(blob)? {
            SourceText::Utf8(source) => source.lines.text(),
            SourceText::NotUtf8(text) => text,
        })
    }

    pub(crate) fn is_ancestor(
        &mut self,
        base: &str,
        tip: &str,
    ) -> Result<bool, FindingSourceFailure> {
        let key = (base.to_owned(), tip.to_owned());
        if let Some(&answer) = self.ancestors.get(&key) {
            return Ok(answer);
        }
        let output = git(self.repo, "merge-base", &["--is-ancestor", base, tip])?;
        let answer = if output.status.code() == Some(1) {
            false
        } else {
            let _ = successful("merge-base", output)?;
            true
        };
        self.ancestors.insert(key, answer);
        Ok(answer)
    }

    pub(crate) fn line_continuity(
        &mut self,
        base: &str,
        checkpoint: &str,
        path: &str,
        line: usize,
    ) -> Result<LineContinuity, FindingSourceFailure> {
        if !self.is_ancestor(base, checkpoint)? {
            return Ok(LineContinuity::Unproved);
        }
        Ok(match self.mapped_line(base, checkpoint, path, line)? {
            LineMapping::Mapped(mapped) if mapped == line => LineContinuity::Unchanged,
            _ => LineContinuity::Unproved,
        })
    }

    pub(crate) fn mapped_line(
        &mut self,
        base: &str,
        tip: &str,
        path: &str,
        line: usize,
    ) -> Result<LineMapping, FindingSourceFailure> {
        let key = (base.to_owned(), tip.to_owned(), path.to_owned());
        let line_map = match self.patches.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
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
                        tip,
                        "--",
                        path,
                    ],
                )?;
                let diff = successful("diff", diff)?;
                entry.insert(PatchLineMap::from(git_text("diff", &diff)?))
            },
        };
        Ok(line_map.at(line))
    }
}

pub(crate) enum TreeFile<'a> {
    Bytes(&'a [u8]),
    Folder,
    Missing,
}

pub(crate) enum SourceText {
    Utf8(Rc<Utf8Source>),
    NotUtf8(String),
}

pub(crate) struct Utf8Source {
    pub(crate) lines:  SourceLines,
    pub(crate) syntax: BlobSyntax,
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

fn grep_reported_missing_blob(stderr: &[u8]) -> bool {
    stderr.split(|&byte| byte == b'\n').any(|line| {
        line.starts_with(b"error: ")
            && line
                .windows(b": unable to read ".len())
                .any(|window| window == b": unable to read ")
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
