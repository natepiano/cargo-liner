//! Read a declaration that no destination gained: built renames, a filed rename's new name, and
//! the outside-rename answer.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::rc::Rc;

use super::approvals::ApprovalSnapshot;
use super::approvals::ReadAnswers;
use super::approvals::Unobserved;
use super::built;
use super::built::BuiltFailure;
use super::built::BuiltRename;
use super::built::BuiltScope;
use super::built::ItemHead;
use super::built::ItemRenameReading;
use super::built_renames::AfterBuilds;
use super::built_renames::BuiltRenameError;
use super::built_renames::BuiltRenameResolution;
use super::built_renames::BuiltRenames;
use super::outcome::DeclarationOutcome;
use super::outcome::OutsideEvidence;
use crate::declaration_source::DeclarationFile;
use crate::declaration_source::SourceLocation;
use crate::declarations::DeclaredItem;
use crate::declarations::ImplMembership;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::NamingSearch;
use crate::ledger_read::BuiltRow;
use crate::ledger_read::FindingKind;
use crate::ledger_read::HeldRename;
use crate::outside_rename::RenameEvidence;
use crate::relocation_ledger::RelocationFindingState;
use crate::rename_applied;
use crate::rename_applied::AppliedError;
use crate::rename_applied::RenameApplied;
use crate::same_declaration;
use crate::same_declaration::Declaration;
use crate::same_declaration::MatchingItems;
use crate::same_declaration::MovedSite;
use crate::same_declaration::Owner;
use crate::same_declaration::RelocationReader;
use crate::source_sites;
use crate::tip_view::RepositoryTip;
use crate::trait_identity::TraitRequirement;
use crate::word_syntax;

/// One finding whose declaration no destination gained.
pub(super) struct Branches<'a, 's> {
    pub(super) source:     &'a mut FindingSource<'s>,
    pub(super) relocation: &'a mut RelocationReader,
    pub(super) approvals:  &'a ApprovalSnapshot,
    pub(super) answers:    &'a mut ReadAnswers,
    pub(super) renames:    &'a [BuiltRename],
    pub(super) rows:       &'a [BuiltRow],
    pub(super) finding:    &'a HeldRename,
    pub(super) state:      &'a RelocationFindingState,
    pub(super) read:       &'a str,
    pub(super) tip_commit: &'a str,
    pub(super) tip:        Rc<RepositoryTip>,
    pub(super) changed:    Rc<HashSet<String>>,
}

/// Why a finding's declaration reading stopped before its outcome.
pub(super) enum Stop {
    Unobserved(String),
    Source(FindingSourceFailure),
    Applied(AppliedError),
    Built(BuiltFailure),
}

impl From<FindingSourceFailure> for Stop {
    fn from(failure: FindingSourceFailure) -> Self { Self::Source(failure) }
}

impl From<AppliedError> for Stop {
    fn from(failure: AppliedError) -> Self { Self::Applied(failure) }
}

impl From<Unobserved> for Stop {
    fn from(Unobserved(path): Unobserved) -> Self { Self::Unobserved(path) }
}

impl From<BuiltRenameError> for Stop {
    fn from(error: BuiltRenameError) -> Self {
        match error {
            BuiltRenameError::Approval(error) => error.into(),
            BuiltRenameError::Source(error) => error.into(),
        }
    }
}

/// The result of a search for the item under a new name.
enum SiteSearch {
    Outcome(DeclarationOutcome),
    NotFound,
}

/// The files at the tip that gained a declaration since the read commit, with the matching items.
enum Gained {
    Known(BTreeMap<String, Vec<usize>>),
    Uncertain,
}

enum Destinations {
    NoFile,
    One { path: String, item: usize },
    Several(Vec<String>),
}

/// The one file that gained the item, else the search result that settles the reading.
enum Destination {
    One { path: String, item: usize },
    Settled(SiteSearch),
}

/// Whether a destination in another crate settles the reading as another crate's item.
#[derive(Clone, Copy)]
enum OtherCrate {
    Settles,
    Follows,
}

/// Whether a filed finding's file remains at the tip, so the finding keeps its decision.
#[derive(Clone, Copy)]
enum FiledFile {
    Kept,
    NotKept,
}

impl Branches<'_, '_> {
    /// The outcome of a declaration that no destination gained.
    pub(super) fn outcome(
        mut self,
        old: &DeclaredItem,
        declaration: &Declaration,
        outside: &RenameEvidence,
    ) -> Result<DeclarationOutcome, Stop> {
        let site = SourceLocation {
            commit: self.read,
            path:   &self.finding.anchor.path,
        };
        let (declarations, traits) = self.relocation.source_readers();
        let built = BuiltRenames {
            source: self.source,
            declarations,
            traits,
            approvals: self.approvals,
            answers: self.answers,
            renames: self.renames,
            edit: &self.finding.edit,
            site,
            tip: &self.tip,
        }
        .resolve(old, declaration)?;
        let after = match built {
            BuiltRenameResolution::Settled(after) => after,
            BuiltRenameResolution::NeedsDestinationProof(candidates) => {
                for candidate in candidates {
                    match self.gaining_files(&candidate, &TraitRequirement::Unrestricted)? {
                        Gained::Known(found) if found.is_empty() => {},
                        Gained::Known(_) | Gained::Uncertain => {
                            return Ok(DeclarationOutcome::TraitIdentityUncertain {
                                declaration: declaration.clone(),
                            });
                        },
                    }
                }
                AfterBuilds::Refused
            },
        };
        if let SiteSearch::Outcome(outcome) = self.renamed_site(old, declaration, &after)? {
            return Ok(outcome);
        }
        if let AfterBuilds::Renamed {
            declaration: renamed,
            requirement,
        } = &after
            && let SiteSearch::Outcome(outcome) = self.followed_rename(old, renamed, requirement)?
        {
            return Ok(outcome);
        }
        let path = self.finding.anchor.path.as_str();
        let file = if matches!(self.state, RelocationFindingState::Filed { .. })
            && self.tip.blob(path).is_some()
        {
            FiledFile::Kept
        } else {
            FiledFile::NotKept
        };
        if matches!(file, FiledFile::Kept) && !self.tip.has_rust_path(path) {
            return Ok(gone(declaration, OutsideEvidence::None));
        }
        // A refused subject question keeps the established transition, without a ledger-wide name
        // veto.
        if matches!(after, AfterBuilds::Renamed { .. } | AfterBuilds::Refused)
            || self.invalid_type(declaration)
        {
            return Ok(removed(declaration, file, OutsideEvidence::None));
        }
        match outside {
            RenameEvidence::Proven(rename) => {
                if matches!(file, FiledFile::Kept)
                    && (self.scoped_pair_names_target(declaration, &[rename.declaration()])?
                        || self.competing_pair(old, declaration, rename.declaration())?)
                {
                    return Ok(gone(declaration, OutsideEvidence::None));
                }
                Ok(DeclarationOutcome::Moved {
                    declaration: rename.declaration().clone(),
                    path:        rename.path().to_owned(),
                    line:        rename.line(),
                })
            },
            RenameEvidence::Refused(refusal) => {
                let targets: Vec<_> = refusal.gained_declarations().collect();
                if matches!(file, FiledFile::Kept)
                    && self.scoped_pair_names_target(declaration, &targets)?
                {
                    return Ok(gone(declaration, OutsideEvidence::None));
                }
                if !refusal.old_name_present() {
                    return Ok(DeclarationOutcome::OutsideRenameUnresolved {
                        declaration: declaration.clone(),
                    });
                }
                Ok(removed(
                    declaration,
                    file,
                    OutsideEvidence::OutsideRenameNotProven,
                ))
            },
        }
    }

    fn approved(&mut self, proposal: &str) -> Result<bool, Stop> {
        Ok(self.approvals.approved(proposal, self.answers)?)
    }

    fn same_crate(&self, path: &str) -> bool {
        self.tip.crate_of(path) == self.tip.crate_of(&self.finding.anchor.path)
    }

    /// A built type finding at the finding's file whose old head names the declaration but whose
    /// edit is no item rename.
    fn invalid_type(&self, declaration: &Declaration) -> bool {
        self.rows.iter().any(|row| {
            row.path == self.finding.anchor.path
                && matches!(row.edit.kind, FindingKind::Type)
                && matches!(built::item_rename(&row.edit), ItemRenameReading::NotRenamed)
                && matches!(
                    built::item_head(source_sites::trim(&row.edit.before)),
                    ItemHead::Head { keyword, name, .. }
                        if keyword == declaration.keyword() && name == declaration.name()
                )
        })
    }

    fn scoped_pair_names_target(
        &mut self,
        declaration: &Declaration,
        targets: &[&Declaration],
    ) -> Result<bool, Stop> {
        let renames = self.renames;
        for rename in renames {
            if !self.same_crate(&rename.path)
                || matches!(rename.scope, BuiltScope::Unqualified)
                || rename.old != declaration.name()
            {
                continue;
            }
            if !self.approved(&rename.proposal)? {
                continue;
            }
            if targets.iter().any(|target| {
                target.keyword() == declaration.keyword()
                    && target.owner() == declaration.owner()
                    && target.name() == rename.new
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// An approved built pair from another crate that renames the declaration to the outside
    /// rename's name, with a scope that holds the declaration.
    fn competing_pair(
        &mut self,
        old: &DeclaredItem,
        declaration: &Declaration,
        outside: &Declaration,
    ) -> Result<bool, Stop> {
        let module = built::declaring_module(&self.finding.anchor.path);
        let renames = self.renames;
        for rename in renames {
            if self.same_crate(&rename.path) || !self.approved(&rename.proposal)? {
                continue;
            }
            if rename.old != declaration.name() || rename.new != outside.name() {
                continue;
            }
            let holds = match (&rename.scope, declaration.owner()) {
                (BuiltScope::Unqualified, _) => true,
                (BuiltScope::ModuleQualified(scope), _) => *scope == module,
                (BuiltScope::TypeQualified(type_name), Owner::ImplOrTraitItem { name }) => {
                    type_name == name
                        || matches!(
                            old.implemented_trait(),
                            ImplMembership::Path(implemented)
                                if implemented.path.last() == Some(type_name)
                        )
                },
                (BuiltScope::TypeQualified(_), Owner::ModuleItem) => false,
            };
            if holds {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Where a filed rename's new name is declared at the tip with the keyword and owner of its
    /// gone declaration, else with the owner that the built renames gave it.
    fn renamed_site(
        &mut self,
        old: &DeclaredItem,
        declaration: &Declaration,
        after: &AfterBuilds,
    ) -> Result<SiteSearch, Stop> {
        let RelocationFindingState::Filed { proposal } = self.state else {
            return Ok(SiteSearch::NotFound);
        };
        let finding = self.finding;
        let edit = &finding.edit;
        let old_name = built::last_segment(&edit.before);
        let new_name = built::last_segment(&edit.after);
        // A variant or field line reads as its enum or struct, so its before name is not the
        // declaration's name.
        if !matches!(edit.kind, FindingKind::Rename)
            || old_name != declaration.name()
            || new_name == old_name
            || !word_syntax::identifier(new_name)
        {
            return Ok(SiteSearch::NotFound);
        }
        let mut renamed = declaration.renamed(new_name, declaration.owner().clone());
        let requirement = TraitRequirement::of(old, self.read_location());
        let mut found = self.gaining_files(&renamed, &requirement)?;
        // The owner's own name comes first: a built rename can give the owner a name that the
        // code does not use.
        if let (
            Gained::Known(gained),
            AfterBuilds::Renamed {
                declaration: after_declaration,
                requirement: after_requirement,
            },
        ) = (&found, after)
            && gained.is_empty()
            && (after_declaration.owner() != declaration.owner()
                || *after_requirement != requirement)
        {
            renamed = declaration.renamed(new_name, after_declaration.owner().clone());
            found = self.gaining_files(&renamed, after_requirement)?;
        }
        let Gained::Known(found) = found else {
            return Ok(SiteSearch::Outcome(
                DeclarationOutcome::TraitIdentityUncertain {
                    declaration: renamed,
                },
            ));
        };
        let (path, item) = match self.destination(found, &renamed, OtherCrate::Settles) {
            Destination::One { path, item } => (path, item),
            Destination::Settled(search) => return Ok(search),
        };
        let applied = rename_applied::applied_at(edit, &path, self.tip_commit, self.source)?;
        if matches!(applied, RenameApplied::NotApplied { .. }) {
            return Ok(SiteSearch::Outcome(gone(
                declaration,
                OutsideEvidence::None,
            )));
        }
        if !self.approved(proposal)? {
            return Ok(SiteSearch::Outcome(
                DeclarationOutcome::RenameAwaitingApproval,
            ));
        }
        let old_file = self.file(self.read, &finding.anchor.path)?;
        let destination = self.file(self.tip_commit, &path)?;
        let line = destination.moved_line(
            &destination.items()[item],
            &old_file,
            old,
            finding.anchor.line,
        );
        Ok(SiteSearch::Outcome(DeclarationOutcome::Renamed {
            declaration: declaration.clone(),
            renamed,
            path,
            line,
        }))
    }

    /// Where the item is declared at the tip under the new names that the built renames gave it.
    fn followed_rename(
        &mut self,
        old: &DeclaredItem,
        renamed: &Declaration,
        requirement: &TraitRequirement,
    ) -> Result<SiteSearch, Stop> {
        let Gained::Known(found) = self.gaining_files(renamed, requirement)? else {
            return Ok(SiteSearch::Outcome(
                DeclarationOutcome::TraitIdentityUncertain {
                    declaration: renamed.clone(),
                },
            ));
        };
        let other_crate = if matches!(self.state, RelocationFindingState::Filed { .. }) {
            OtherCrate::Settles
        } else {
            OtherCrate::Follows
        };
        let (path, item) = match self.destination(found, renamed, other_crate) {
            Destination::One { path, item } => (path, item),
            Destination::Settled(search) => return Ok(search),
        };
        let finding = self.finding;
        let old_file = self.file(self.read, &finding.anchor.path)?;
        let destination = self.file(self.tip_commit, &path)?;
        let site = same_declaration::moved_site(
            &old_file,
            old,
            &finding.anchor,
            &destination,
            &destination.items()[item],
        );
        Ok(SiteSearch::Outcome(match site {
            MovedSite::Line(line) => DeclarationOutcome::Moved {
                declaration: renamed.clone(),
                path,
                line,
            },
            MovedSite::StatementRefused(reason) => DeclarationOutcome::StatementNotFollowed {
                declaration: renamed.clone(),
                path,
                reason,
            },
        }))
    }

    /// The one file that gained the renamed item, else the outcome for several files or for a file
    /// in another crate.
    fn destination(
        &self,
        found: BTreeMap<String, Vec<usize>>,
        renamed: &Declaration,
        other_crate: OtherCrate,
    ) -> Destination {
        let (path, item) = match destinations(found) {
            Destinations::NoFile => return Destination::Settled(SiteSearch::NotFound),
            Destinations::Several(paths) => {
                return Destination::Settled(SiteSearch::Outcome(
                    DeclarationOutcome::SeveralFiles {
                        declaration: renamed.clone(),
                        paths,
                        evidence: OutsideEvidence::None,
                    },
                ));
            },
            Destinations::One { path, item } => (path, item),
        };
        if matches!(other_crate, OtherCrate::Settles) && !self.same_crate(&path) {
            return Destination::Settled(SiteSearch::Outcome(DeclarationOutcome::AnotherCrate {
                declaration: renamed.clone(),
                path,
                evidence: OutsideEvidence::None,
            }));
        }
        Destination::One { path, item }
    }

    /// The files that gained the declaration since the read commit; the finding's own file comes
    /// first and settles the search when it gained it.
    fn gaining_files(
        &mut self,
        renamed: &Declaration,
        requirement: &TraitRequirement,
    ) -> Result<Gained, Stop> {
        let own = self.finding.anchor.path.clone();
        let named = self.source.paths_naming(
            self.tip_commit,
            renamed.name(),
            NamingSearch::DeclarationProof,
        )?;
        let mut found = BTreeMap::new();
        for path in std::iter::once(&own).chain(named.iter().filter(|path| **path != own)) {
            if *path != own && (!self.tip.has_rust_path(path) || !self.changed.contains(path)) {
                continue;
            }
            let prior = self.matching(renamed, requirement, self.read, path)?;
            let current = self.matching(renamed, requirement, self.tip_commit, path)?;
            let (MatchingItems::Known(prior), MatchingItems::Known(current)) = (prior, current)
            else {
                return Ok(Gained::Uncertain);
            };
            if !current.is_empty() && prior.is_empty() {
                if *path == own {
                    return Ok(Gained::Known(BTreeMap::from([(own, current)])));
                }
                found.insert(path.clone(), current);
            }
        }
        Ok(Gained::Known(found))
    }

    fn matching(
        &mut self,
        declaration: &Declaration,
        requirement: &TraitRequirement,
        commit: &str,
        path: &str,
    ) -> Result<MatchingItems, Stop> {
        let location = SourceLocation { commit, path };
        Ok(self
            .relocation
            .matching_at(self.source, declaration, requirement, location)?)
    }

    fn file(&mut self, commit: &str, path: &str) -> Result<Rc<DeclarationFile>, Stop> {
        let (declarations, _) = self.relocation.source_readers();
        Ok(declarations.file(self.source, SourceLocation { commit, path })?)
    }

    fn read_location(&self) -> SourceLocation<'_> {
        SourceLocation {
            commit: self.read,
            path:   &self.finding.anchor.path,
        }
    }
}

fn destinations(found: BTreeMap<String, Vec<usize>>) -> Destinations {
    if found.len() > 1 || found.values().any(|items| items.len() != 1) {
        return Destinations::Several(found.into_keys().collect());
    }
    match found.into_iter().next() {
        Some((path, items)) => Destinations::One {
            path,
            item: items[0],
        },
        None => Destinations::NoFile,
    }
}

fn gone(declaration: &Declaration, evidence: OutsideEvidence) -> DeclarationOutcome {
    DeclarationOutcome::GoneFromFile {
        declaration: declaration.clone(),
        evidence,
    }
}

/// A filed finding whose file remains keeps its decision; any other finding's declaration is
/// removed.
fn removed(
    declaration: &Declaration,
    file: FiledFile,
    evidence: OutsideEvidence,
) -> DeclarationOutcome {
    match file {
        FiledFile::Kept => gone(declaration, evidence),
        FiledFile::NotKept => DeclarationOutcome::Removed {
            declaration: declaration.clone(),
            evidence,
        },
    }
}
