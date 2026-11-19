//! Find the committed files that need an edit for a finding.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::path::Path;

use fancy_regex::Error;
use fancy_regex::Regex;
use serde::Serialize;

use super::checkout_crates::CheckoutCrates;
use super::checkout_crates::CrateRoot;
use super::constants::UNREAD_PATH_STRING;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SearchFiles;
use crate::ledger_read::HeldFinding;
use crate::module_scan;
use crate::module_scan::ModuleScanReading;
use crate::module_sites;
use crate::module_sites::CrateLibrary;
use crate::module_sites::FileModule;
use crate::module_sites::ModuleMove;
use crate::module_sites::ModuleSiteReading;
use crate::module_sites::ModuleSiteRefusal;
use crate::module_sites::ModuleSites;
use crate::rename_classifiers;
use crate::rename_classifiers::FileRename;
use crate::rename_classifiers::FindingChange;
use crate::rename_classifiers::RenameClassifiers;
use crate::tokenizer;
use crate::tokenizer::Token;

/// The extra commit to search after the synced tip, if the caller supplied one.
pub(crate) enum FootprintBase {
    SyncedOnly,
    WithBase(String),
}

/// The paths to edit.
/// `uncertain` holds files with an unread `#[path]` attribute.
#[derive(Serialize)]
pub(crate) struct Footprint {
    files:     Vec<String>,
    uncertain: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FootprintError {
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error("cannot classify the finding edit")]
    ClassifyFinding(#[source] Box<Error>),
    #[error("cannot match a module path")]
    MatchModulePath(#[source] Box<Error>),
}

impl FootprintError {
    fn classify_finding(error: Error) -> Self { Self::ClassifyFinding(Box::new(error)) }

    fn match_module_path(error: Error) -> Self { Self::MatchModulePath(Box::new(error)) }
}

#[derive(Clone, Copy)]
enum UnreadPathString {
    Present,
    Absent,
}

impl From<bool> for UnreadPathString {
    fn from(present: bool) -> Self { if present { Self::Present } else { Self::Absent } }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct NamedVersion {
    path: String,
    blob: String,
}

struct ModuleSyntax {
    text:         Vec<char>,
    tokens:       Vec<Token>,
    declarations: OnceCell<ModuleScanReading>,
}

#[derive(Default)]
struct FootprintPaths {
    paths: Vec<String>,
    seen:  HashSet<String>,
}

impl FootprintPaths {
    fn add(&mut self, path: &str) {
        if self.seen.insert(path.to_owned()) {
            self.paths.push(path.to_owned());
        }
    }
}

/// The source reads and checkout inputs shared by every footprint in one request.
pub(crate) struct FootprintReader<'a> {
    crates:       CheckoutCrates<'a>,
    commits:      Vec<String>,
    classifiers:  RenameClassifiers,
    unread_path:  Regex,
    names:        HashMap<String, Vec<NamedVersion>>,
    qualified:    HashMap<String, Vec<String>>,
    unread_blobs: HashMap<String, UnreadPathString>,
    syntax:       HashMap<String, ModuleSyntax>,
}

impl<'a> FootprintReader<'a> {
    pub(crate) fn new(
        checkout: &'a Path,
        synced_tip: String,
        base: FootprintBase,
    ) -> Result<Self, FootprintError> {
        let mut commits = vec![synced_tip];
        if let FootprintBase::WithBase(commit) = base
            && commits[0] != commit
        {
            commits.push(commit);
        }
        Ok(Self {
            crates: CheckoutCrates::new(checkout),
            commits,
            classifiers: RenameClassifiers::new().map_err(FootprintError::classify_finding)?,
            unread_path: Regex::new(UNREAD_PATH_STRING)
                .map_err(FootprintError::match_module_path)?,
            names: HashMap::new(),
            qualified: HashMap::new(),
            unread_blobs: HashMap::new(),
            syntax: HashMap::new(),
        })
    }

    pub(crate) fn read(
        &mut self,
        finding: &HeldFinding,
        source: &mut FindingSource<'_>,
    ) -> Result<Footprint, FootprintError> {
        let path = &finding.anchor.path;
        let mut files = FootprintPaths::default();
        let mut uncertain = FootprintPaths::default();
        files.add(path);
        match self
            .classifiers
            .classify(path, &finding.edit)
            .map_err(FootprintError::classify_finding)?
        {
            FindingChange::FileMove(moved) => {
                files.add(&moved.new);
                self.add_move_sites(source, &moved, &mut files, &mut uncertain)?;
            },
            FindingChange::QualifiedRename { old } => {
                self.add_qualified_paths(source, old, &mut files)?;
            },
            FindingChange::ModuleOwners => self.add_owners(source, path, &mut files)?,
            FindingChange::RecordedPaths => {},
        }
        for need in &finding.needs {
            files.add(need);
        }
        Ok(Footprint {
            files:     files.paths,
            uncertain: uncertain.paths,
        })
    }

    fn add_move_sites(
        &mut self,
        source: &mut FindingSource<'_>,
        moved: &FileRename<'_>,
        files: &mut FootprintPaths,
        uncertain: &mut FootprintPaths,
    ) -> Result<(), FootprintError> {
        let root = self.crates.root_for(moved.old);
        for named in self.declaring_versions(source, rename_classifiers::stem(moved.old))? {
            let CrateRoot::Found { folder, library } = &root else {
                files.add(&named.path);
                continue;
            };
            let file_module = FileModule::for_path(&named.path, folder);
            if matches!(file_module, FileModule::Outside(_))
                && !outside_candidate(source.text(&named.blob)?, moved.old, library)
                && matches!(
                    self.unread_path_string(source, &named.blob)?,
                    UnreadPathString::Absent
                )
            {
                continue;
            }
            let FileModule::Crate { mut path } = FileModule::for_path(moved.old, folder) else {
                files.add(&named.path);
                continue;
            };
            path.pop();
            let query = ModuleMove::new(
                moved.old.to_owned(),
                moved.new.clone(),
                file_module,
                path,
                library.clone(),
            );
            let syntax = self.module_syntax(source, named.blob)?;
            for reading in
                module_sites::read_tokens(&syntax.text, &syntax.tokens, &named.path, &[query])
            {
                match reading {
                    ModuleSiteReading::Sites {
                        module_sites: ModuleSites::Absent,
                    } => {},
                    ModuleSiteReading::Refused(ModuleSiteRefusal::UncertainModulePath) => {
                        files.add(&named.path);
                        uncertain.add(&named.path);
                    },
                    _ => files.add(&named.path),
                }
            }
        }
        Ok(())
    }

    fn add_owners(
        &mut self,
        source: &mut FindingSource<'_>,
        path: &str,
        files: &mut FootprintPaths,
    ) -> Result<(), FootprintError> {
        let root = self.crates.root_for(path);
        for named in self.declaring_versions(source, rename_classifiers::module_name(path))? {
            if named.path == path {
                continue;
            }
            if let CrateRoot::Found { folder, .. } = &root
                && let FileModule::Crate { path: mut parent } = FileModule::for_path(path, folder)
            {
                parent.pop();
                let syntax = self.module_syntax(source, named.blob)?;
                let declarations = syntax
                    .declarations
                    .get_or_init(|| module_scan::read_tokens(&syntax.tokens));
                if let ModuleScanReading::Value(reading) = declarations
                    && !reading.declares(
                        &named.path,
                        &FileModule::for_path(&named.path, folder),
                        &parent,
                        path,
                    )
                {
                    continue;
                }
            }
            files.add(&named.path);
        }
        Ok(())
    }

    fn declaring_versions(
        &mut self,
        source: &mut FindingSource<'_>,
        name: &str,
    ) -> Result<Vec<NamedVersion>, FootprintError> {
        let paths = self.named_versions(source, "path")?;
        let mut versions = self.named_versions(source, name)?;
        let mut seen: HashSet<_> = versions.iter().cloned().collect();
        for named in paths {
            if matches!(
                self.unread_path_string(source, &named.blob)?,
                UnreadPathString::Present
            ) && seen.insert(named.clone())
            {
                versions.push(named);
            }
        }
        Ok(versions)
    }

    fn add_qualified_paths(
        &mut self,
        source: &FindingSource<'_>,
        name: &str,
        files: &mut FootprintPaths,
    ) -> Result<(), FootprintError> {
        let paths = match self.qualified.entry(name.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let mut paths = FootprintPaths::default();
                for commit in &self.commits {
                    for path in source.paths_naming(commit, name, SearchFiles::RustAndMarkdown)? {
                        paths.add(&path);
                    }
                }
                entry.insert(paths.paths)
            },
        };
        for path in paths {
            files.add(path);
        }
        Ok(())
    }

    fn named_versions(
        &mut self,
        source: &mut FindingSource<'_>,
        name: &str,
    ) -> Result<Vec<NamedVersion>, FootprintError> {
        let versions = match self.names.entry(name.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let mut versions = Vec::new();
                let mut seen = HashSet::new();
                for commit in &self.commits {
                    for path in source.paths_naming(commit, name, SearchFiles::Rust)? {
                        let blob = source.candidate_blob(commit, &path)?;
                        let _ = source.text(&blob)?;
                        let named = NamedVersion { path, blob };
                        if seen.insert(named.clone()) {
                            versions.push(named);
                        }
                    }
                }
                entry.insert(versions)
            },
        };
        Ok(versions.clone())
    }

    fn unread_path_string(
        &mut self,
        source: &mut FindingSource<'_>,
        blob: &str,
    ) -> Result<UnreadPathString, FootprintError> {
        Ok(match self.unread_blobs.entry(blob.to_owned()) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let present = self
                    .unread_path
                    .is_match(source.text(blob)?)
                    .map_err(FootprintError::match_module_path)?;
                *entry.insert(present.into())
            },
        })
    }

    fn module_syntax(
        &mut self,
        source: &mut FindingSource<'_>,
        blob: String,
    ) -> Result<&ModuleSyntax, FootprintError> {
        Ok(match self.syntax.entry(blob) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let text: Vec<_> = source.text(entry.key())?.chars().collect();
                let tokens = tokenizer::read_with_comments(&text).tokens;
                entry.insert(ModuleSyntax {
                    text,
                    tokens,
                    declarations: OnceCell::new(),
                })
            },
        })
    }
}

fn outside_candidate(text: &str, moved: &str, library: &CrateLibrary) -> bool {
    text.contains(rename_classifiers::filename(moved))
        || matches!(library, CrateLibrary::Named { name } if text.contains(name))
}
