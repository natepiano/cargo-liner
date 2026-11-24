//! Read the Git objects and line maps for finding proofs.

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

/// Git reads shared by the finding proofs in one request.
pub(crate) struct FindingSource<'a> {
    repo:        &'a Path,
    blob_reader: BlobReader,
    fact_cache:  FactCache,
    commits:     HashMap<String, CommitResolution>,
    trees:       HashMap<String, TreeBlobs>,
    sources:     HashMap<String, SourceText>,
    tips:        HashMap<String, Rc<RepositoryTip>>,
    raw_blobs:   HashMap<String, Vec<u8>>,
    ancestors:   HashMap<(String, String), bool>,
    patches:     HashMap<(String, String, String), MyersPatch>,
    changes:     HashMap<(String, String), Rc<HashSet<String>>>,
    names:       HashMap<(String, String, SearchFiles), NamingMatches>,
}

struct MyersPatch {
    text:     String,
    line_map: PatchLineMap,
}

struct NamingMatches {
    output: Output,
    paths:  OnceCell<Rc<Vec<String>>>,
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
    #[error("cannot read source at {path}")]
    AtPath { path: String, source: Box<Self> },
    #[error("cannot read repository tip {0}")]
    UnreadableTip(String),
    #[error("cannot read manifest at {path}")]
    InvalidManifest {
        path:   String,
        #[source]
        source: Box<toml::de::Error>,
    },
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

/// The search scope and diagnostic contract for a finding proof.
#[derive(Clone, Copy)]
pub(crate) enum NamingSearch {
    DeclarationProof,
    Footprint(SearchFiles),
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
            raw_blobs: HashMap::new(),
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
        &mut self,
        commit: &str,
        name: &str,
        search: NamingSearch,
    ) -> Result<Rc<Vec<String>>, FindingSourceFailure> {
        let files = match search {
            NamingSearch::DeclarationProof => SearchFiles::Rust,
            NamingSearch::Footprint(files) => files,
        };
        let matches = match self
            .names
            .entry((commit.to_owned(), name.to_owned(), files))
        {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let mut arguments = vec!["-l", "-z", "-w", "-F", "-e", name, commit, "--", "*.rs"];
                if files == SearchFiles::RustAndMarkdown {
                    arguments.push("*.md");
                }
                entry.insert(NamingMatches {
                    output: git(self.repo, "grep", &arguments)?,
                    paths:  OnceCell::new(),
                })
            },
        };
        matches.paths(commit, search)
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
        Ok(self.myers_patch(base, tip, path)?.line_map.at(line))
    }

    pub(crate) fn myers_diff(
        &mut self,
        base: &str,
        tip: &str,
        path: &str,
    ) -> Result<&str, FindingSourceFailure> {
        Ok(&self.myers_patch(base, tip, path)?.text)
    }

    fn myers_patch(
        &mut self,
        base: &str,
        tip: &str,
        path: &str,
    ) -> Result<&MyersPatch, FindingSourceFailure> {
        let key = (base.to_owned(), tip.to_owned(), path.to_owned());
        Ok(match self.patches.entry(key) {
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
                let text = git_text("diff", &diff)?.to_owned();
                let line_map = PatchLineMap::from(text.as_str());
                entry.insert(MyersPatch { text, line_map })
            },
        })
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

impl NamingMatches {
    fn paths(
        &mut self,
        commit: &str,
        search: NamingSearch,
    ) -> Result<Rc<Vec<String>>, FindingSourceFailure> {
        let output = &self.output;
        if !(output.status.success()
            || output.status.code() == Some(1)
                && !grep_reported_missing_blob(&output.stderr, search))
        {
            return Err(git_exit("grep", output));
        }
        if let Some(paths) = self.paths.get() {
            return Ok(Rc::clone(paths));
        }
        let prefix = format!("{commit}:");
        let paths = output
            .stdout
            .split(|&byte| byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                let name = path.strip_prefix(prefix.as_bytes()).unwrap_or(path);
                match search {
                    NamingSearch::DeclarationProof => {
                        let name = git_text("grep", path)?;
                        Ok(name.strip_prefix(&prefix).unwrap_or(name).to_owned())
                    },
                    NamingSearch::Footprint(_) => Ok(String::from_utf8(name.to_vec())?),
                }
            })
            .collect::<Result<_, FindingSourceFailure>>()?;
        self.output.stdout = Vec::new();
        let paths = self.paths.get_or_init(|| Rc::new(paths));
        Ok(Rc::clone(paths))
    }
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

fn git_exit(command: &'static str, output: &Output) -> FindingSourceFailure {
    FindingSourceFailure::GitExit {
        command,
        status: output.status,
        detail: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    }
}

fn grep_reported_missing_blob(stderr: &[u8], search: NamingSearch) -> bool {
    const UNREADABLE: &[u8] = b": unable to read ";
    stderr.split(|&byte| byte == b'\n').any(|line| {
        let Some(message) = line.strip_prefix(b"error: ") else {
            return false;
        };
        let Some(separator) = message
            .windows(UNREADABLE.len())
            .rposition(|part| part == UNREADABLE)
        else {
            return false;
        };
        match search {
            NamingSearch::Footprint(_) => true,
            NamingSearch::DeclarationProof => {
                let oid = &message[separator + UNREADABLE.len()..];
                let oid = oid.strip_suffix(b"\r").unwrap_or(oid);
                separator > 0
                    && (40..=64).contains(&oid.len())
                    && oid
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
            },
        }
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
