//! Read the Git objects and line map for finding anchors.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
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
use crate::declarations;
use crate::error::ReaderError;
use crate::fact_store::FactCache;
use crate::facts::BlobSource;
use crate::facts::BlobSyntax;
use crate::facts::StoredSyntax;
use crate::source_lines::LineContinuity;
use crate::source_lines::LineMapping;
use crate::source_lines::PatchLineMap;
use crate::source_lines::SiteLine;
use crate::source_lines::SourceLines;
use crate::tip_view::RepositoryTip;
use crate::tip_view::TipReading;
use crate::tokenizer;

/// Git reads shared by the finding anchors in one request.
pub(crate) struct FindingSource<'a> {
    repo:        &'a Path,
    blob_reader: BlobReader,
    fact_cache:  FactCache,
    commits:     HashMap<String, CommitResolution>,
    trees:       HashMap<String, TreeBlobs>,
    sources:     HashMap<String, SourceText>,
    tips:        HashMap<String, Rc<RepositoryTip>>,
    ancestors:   HashMap<(String, String), bool>,
    patches:     HashMap<(String, String, String), PatchLineMap>,
    changes:     HashMap<(String, String), Rc<HashSet<String>>>,
    names:       HashMap<(String, String), Rc<Vec<String>>>,
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
    #[error("cannot read repository tip {0}")]
    UnreadableTip(String),
    #[error("cannot read manifest at {path}")]
    InvalidManifest {
        path:   String,
        #[source]
        source: Box<toml::de::Error>,
    },
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
    Undecodable,
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
            tips: HashMap::new(),
            ancestors: HashMap::new(),
            patches: HashMap::new(),
            changes: HashMap::new(),
            names: HashMap::new(),
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
            TreeBlobs::Undecodable => None,
        })
    }

    pub(crate) fn tree(&mut self, commit: &str) -> Result<&TreeBlobs, FindingSourceFailure> {
        Ok(match self.trees.entry(commit.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let trees = self.blob_reader.inventories(&[commit.to_owned()])?;
                let entries = trees
                    .into_iter()
                    .flat_map(|tree| tree.entries)
                    .map(|(path, mode, kind, object)| {
                        let kind = match kind.as_str() {
                            "tree" => EntryKind::Directory,
                            "commit" => EntryKind::Submodule,
                            _ => EntryKind::Blob,
                        };
                        Ok((String::from_utf8(path)?, TreeEntry { mode, object, kind }))
                    })
                    .collect::<Result<TreeEntries, FromUtf8Error>>()
                    .map_or(TreeBlobs::Undecodable, |entries| {
                        TreeBlobs::Listed(Rc::new(entries))
                    });
                entry.insert(entries)
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
                    BlobSource::Utf8 { text, syntax } => {
                        SourceText::Utf8(Rc::new(Utf8Source::new(text, syntax)))
                    },
                    BlobSource::NotUtf8(text) => SourceText::NotUtf8(text),
                    BlobSource::Missing => {
                        return Err(FindingSourceFailure::MissingBlob(blob.to_owned()));
                    },
                };
                entry.insert(source)
            },
        })
    }

    pub(crate) fn tip(&mut self, commit: &str) -> Result<Rc<RepositoryTip>, FindingSourceFailure> {
        if let Some(tip) = self.tips.get(commit) {
            return Ok(Rc::clone(tip));
        }
        let TipReading::Read(tip) = RepositoryTip::read(self, commit)? else {
            return Err(FindingSourceFailure::UnreadableTip(commit.to_owned()));
        };
        let tip = Rc::new(tip);
        self.tips.insert(commit.to_owned(), Rc::clone(&tip));
        Ok(tip)
    }

    pub(crate) fn changed_paths(
        &mut self,
        read: &str,
        tip: &str,
    ) -> Result<Rc<HashSet<String>>, FindingSourceFailure> {
        let paths = match self.changes.entry((read.to_owned(), tip.to_owned())) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let output = git(
                    self.repo,
                    "diff",
                    &["--no-renames", "--name-only", "-z", read, tip],
                )?;
                let bytes = successful("diff", output)?;
                let paths = git_text("diff", &bytes)?
                    .split('\0')
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .collect();
                entry.insert(Rc::new(paths))
            },
        };
        Ok(Rc::clone(paths))
    }

    pub(crate) fn files_naming(
        &mut self,
        commit: &str,
        name: &str,
    ) -> Result<Rc<Vec<String>>, FindingSourceFailure> {
        let paths = match self.names.entry((commit.to_owned(), name.to_owned())) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let output = git(
                    self.repo,
                    "grep",
                    &["-l", "-z", "-w", "-F", "-e", name, commit, "--", "*.rs"],
                )?;
                let bytes =
                    if output.status.code() == Some(1) && !grep_unreadable_blob(&output.stderr) {
                        output.stdout
                    } else {
                        successful("grep", output)?
                    };
                let prefix = format!("{commit}:");
                let paths = bytes
                    .split(|&byte| byte == 0)
                    .filter(|path| !path.is_empty())
                    .map(|path| {
                        let path = git_text("grep", path)?;
                        Ok(path.strip_prefix(&prefix).unwrap_or(path).to_owned())
                    })
                    .collect::<Result<_, FindingSourceFailure>>()?;
                entry.insert(Rc::new(paths))
            },
        };
        Ok(Rc::clone(paths))
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

pub(crate) enum SourceText {
    Utf8(Rc<Utf8Source>),
    NotUtf8(String),
}

pub(crate) struct Utf8Source {
    pub(crate) lines: SourceLines,
    syntax:           BlobSyntax,
    parsed:           OnceCell<StoredSyntax>,
}

impl Utf8Source {
    pub(crate) fn new(text: String, syntax: BlobSyntax) -> Self {
        Self {
            lines: text.into(),
            syntax,
            parsed: OnceCell::new(),
        }
    }

    pub(crate) fn parsed_syntax(&self) -> &StoredSyntax {
        match &self.syntax {
            BlobSyntax::Stored(syntax) => syntax,
            BlobSyntax::Unparsed => self.parsed.get_or_init(|| {
                let chars: Vec<_> = self.lines.text().chars().collect();
                let tokens = tokenizer::read_with_comments(&chars).tokens;
                let declarations = Rc::new(declarations::read_tokens(&tokens));
                StoredSyntax {
                    tokens,
                    declarations,
                }
            }),
        }
    }
}

fn grep_unreadable_blob(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).lines().any(|line| {
        line.strip_prefix("error: ")
            .and_then(|message| message.rsplit_once(": unable to read "))
            .is_some_and(|(path, oid)| {
                !path.is_empty()
                    && (40..=64).contains(&oid.len())
                    && oid
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
    })
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
