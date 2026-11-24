//! Prove that a held finding's declaration stayed in place or moved.

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use serde::Serialize;
use serde::Serializer;

use crate::declaration_source::DeclarationFile;
use crate::declaration_source::DeclarationSource;
use crate::declaration_source::SourceLocation;
use crate::declaration_source::StatementLocation;
use crate::declaration_source::StatementRefusal;
use crate::declarations::DeclarationIndexAt;
use crate::declarations::DeclarationOwner;
use crate::declarations::DeclaredItem;
use crate::declarations::ImplMembership;
use crate::declarations::ImplementationState;
use crate::finding_anchor::FindingAnchor;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::NamingSearch;
use crate::ledger_read::HeldAnchor;
use crate::relocation_ledger::JudgingState;
use crate::relocation_ledger::LinkedBuild;
use crate::relocation_ledger::RelocationEvidence;
use crate::relocation_ledger::RelocationFindingState;
use crate::tip_view::RepositoryTip;
use crate::trait_identity::TraitIdentity;
use crate::trait_identity::TraitReader;
use crate::trait_identity::TraitRequirement;
use crate::trait_identity::TraitSite;

pub(crate) enum SameDeclaration {
    NotRead {
        cause: NotReadCause,
    },
    InPlace {
        basis: InPlaceBasis,
    },
    Moved {
        declaration: Declaration,
        path:        String,
        line:        usize,
    },
    SeveralDestinations {
        declaration: Declaration,
        paths:       Vec<String>,
    },
    AnotherCrate {
        declaration: Declaration,
        path:        String,
    },
    TraitIdentityUncertain {
        declaration: Declaration,
    },
    OpenBuild {
        ticket: String,
    },
    StatementNotFollowed {
        declaration: Declaration,
        path:        String,
        reason:      StatementRefusal,
    },
    /// No destination gained the item at the finding's original coordinate.
    NoDestination {
        declaration: Declaration,
        original:    AnchoredItem,
    },
}

/// A named item in its source file at the commit that relocation read.
pub(crate) struct AnchoredItem {
    file:  Rc<DeclarationFile>,
    index: usize,
    read:  String,
}

impl AnchoredItem {
    pub(crate) fn file(&self) -> &DeclarationFile { &self.file }

    pub(crate) fn item(&self) -> &DeclaredItem { &self.file.items()[self.index] }

    pub(crate) fn read(&self) -> &str { &self.read }
}

impl Serialize for SameDeclaration {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "status", rename_all = "snake_case")]
        enum RelocationReply<'a> {
            NotRead,
            InPlace {
                basis: &'a InPlaceBasis,
            },
            Moved {
                declaration: &'a Declaration,
                path:        &'a str,
                line:        usize,
            },
            SeveralDestinations {
                declaration: &'a Declaration,
                paths:       &'a [String],
                reason:      String,
            },
            AnotherCrate {
                declaration: &'a Declaration,
                path:        &'a str,
                reason:      String,
            },
            TraitIdentityUncertain {
                declaration: &'a Declaration,
                reason:      &'static str,
            },
            OpenBuild {
                ticket: &'a str,
                reason: String,
            },
            StatementNotFollowed {
                declaration: &'a Declaration,
                path:        &'a str,
                reason:      &'a StatementRefusal,
            },
            NoDestination {
                declaration: &'a Declaration,
            },
        }
        let reply = match self {
            Self::NotRead { .. } => RelocationReply::NotRead,
            Self::InPlace { basis } => RelocationReply::InPlace { basis },
            Self::Moved {
                declaration,
                path,
                line,
            } => RelocationReply::Moved {
                declaration,
                path,
                line: *line,
            },
            Self::SeveralDestinations { declaration, paths } => {
                RelocationReply::SeveralDestinations {
                    declaration,
                    paths,
                    reason: format!("declared in {}", paths.join(", ")),
                }
            },
            Self::AnotherCrate { declaration, path } => RelocationReply::AnotherCrate {
                declaration,
                path,
                reason: format!("declared in another crate: {path}"),
            },
            Self::TraitIdentityUncertain { declaration } => {
                RelocationReply::TraitIdentityUncertain {
                    declaration,
                    reason: "trait identity uncertain",
                }
            },
            Self::OpenBuild { ticket } => RelocationReply::OpenBuild {
                ticket,
                reason: format!("open ticket {ticket} builds it"),
            },
            Self::StatementNotFollowed {
                declaration,
                path,
                reason,
            } => RelocationReply::StatementNotFollowed {
                declaration,
                path,
                reason,
            },
            Self::NoDestination { declaration, .. } => {
                RelocationReply::NoDestination { declaration }
            },
        };
        reply.serialize(serializer)
    }
}

/// Why the declaration proof did not read the finding's item.
pub(crate) enum NotReadCause {
    /// The finding is closed, judged now, not Rust, unanchored or not an ancestor.
    Unread,
    /// The finding's old line holds no named declaration.
    UnnamedLine,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InPlaceBasis {
    UnchangedPath,
    MatchingItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct Declaration {
    keyword: String,
    name:    String,
    owner:   Owner,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Owner {
    ModuleItem,
    ImplOrTraitItem { name: String },
}

impl Declaration {
    pub(crate) const fn keyword(&self) -> &str { self.keyword.as_str() }

    pub(crate) const fn name(&self) -> &str { self.name.as_str() }

    pub(crate) const fn owner(&self) -> &Owner { &self.owner }

    /// The same keyword under another name and owner.
    pub(crate) fn renamed(&self, name: &str, owner: Owner) -> Self {
        Self {
            keyword: self.keyword.clone(),
            name: name.to_owned(),
            owner,
        }
    }

    pub(crate) fn has_owner(&self, owner: &DeclarationOwner) -> bool {
        match (&self.owner, owner) {
            (Owner::ModuleItem, DeclarationOwner::Module) => true,
            (Owner::ImplOrTraitItem { name }, DeclarationOwner::ImplOrTrait(other)) => {
                name == other
            },
            _ => false,
        }
    }

    pub(crate) fn matches(&self, item: &DeclaredItem) -> bool {
        self.keyword == item.keyword() && self.name == item.name && self.has_owner(item.owner())
    }

    pub(crate) fn under_owner(mut self, name: &str) -> Self {
        self.owner = Owner::ImplOrTraitItem {
            name: name.to_owned(),
        };
        self
    }
}

impl From<&DeclaredItem> for Declaration {
    fn from(item: &DeclaredItem) -> Self {
        let owner = match item.owner() {
            DeclarationOwner::Module => Owner::ModuleItem,
            DeclarationOwner::ImplOrTrait(name) => Owner::ImplOrTraitItem { name: name.clone() },
        };
        Self {
            keyword: item.keyword().to_owned(),
            name: item.name.clone(),
            owner,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RelocationFailure {
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error("unknown ticket {0}")]
    UnknownBuild(String),
}

#[derive(Default)]
pub(crate) struct RelocationReader {
    declarations: DeclarationSource,
    trait_reader: TraitReader,
}

pub(crate) enum MatchingItems {
    Known(Vec<usize>),
    Uncertain,
}

/// Where a finding's line lands in the one item its old item moved to.
pub(crate) enum MovedSite {
    Line(usize),
    StatementRefused(StatementRefusal),
}

struct GainedItems {
    file:  Rc<DeclarationFile>,
    items: Vec<usize>,
}

enum GainedDestinations {
    Known(BTreeMap<String, GainedItems>),
    Uncertain,
}

impl RelocationReader {
    pub(crate) const fn source_readers(&mut self) -> (&mut DeclarationSource, &mut TraitReader) {
        (&mut self.declarations, &mut self.trait_reader)
    }

    pub(crate) fn prove(
        &mut self,
        source: &mut FindingSource<'_>,
        synced_tip: &str,
        finding: &HeldAnchor,
        anchor: &FindingAnchor,
        evidence: &RelocationEvidence,
    ) -> Result<SameDeclaration, RelocationFailure> {
        match evidence.finding {
            RelocationFindingState::Closed => return Ok(not_read()),
            RelocationFindingState::Unfiled
                if matches!(evidence.judging, JudgingState::AssignedOrCheckpointed) =>
            {
                return Ok(not_read());
            },
            RelocationFindingState::Unfiled | RelocationFindingState::Filed { .. } => {},
        }
        if Path::new(&finding.path)
            .extension()
            .is_none_or(|ext| ext != "rs")
        {
            return Ok(not_read());
        }
        let FindingAnchor::Anchored(anchor) = anchor else {
            return Ok(not_read());
        };
        let read = anchor.commit.as_str();
        if !source.is_ancestor(read, synced_tip)? {
            return Ok(not_read());
        }
        let changed = source.changed_paths(read, synced_tip)?;
        let original_path = finding.path.as_str();
        if !changed.contains(original_path) {
            return Ok(SameDeclaration::InPlace {
                basis: InPlaceBasis::UnchangedPath,
            });
        }
        let old_location = SourceLocation {
            commit: read,
            path:   original_path,
        };
        let old_file = self.declarations.file(source, old_location)?;
        let DeclarationIndexAt::Named(index) = old_file.index_at(finding.line) else {
            return Ok(SameDeclaration::NotRead {
                cause: NotReadCause::UnnamedLine,
            });
        };
        let old = &old_file.items()[index];
        if matches!(
            old.implemented_trait(),
            ImplMembership::State(ImplementationState::Unreadable)
        ) {
            return Ok(uncertain_trait(old.into()));
        }

        let declaration = Declaration::from(old);
        let requirement = TraitRequirement::of(old, old_location);
        let new_location = SourceLocation {
            commit: synced_tip,
            path:   original_path,
        };
        match self.matching_at(source, &declaration, &requirement, new_location)? {
            MatchingItems::Uncertain => return Ok(uncertain_trait(old.into())),
            MatchingItems::Known(items) if !items.is_empty() => {
                return Ok(SameDeclaration::InPlace {
                    basis: InPlaceBasis::MatchingItem,
                });
            },
            MatchingItems::Known(_) => {},
        }

        let tip = source.tip(synced_tip)?;
        let named = source.paths_naming(synced_tip, &old.name, NamingSearch::DeclarationProof)?;
        let candidates = named.iter().filter(|path| {
            path.as_str() != original_path
                && changed.contains(path.as_str())
                && tip.has_rust_path(path)
        });
        let GainedDestinations::Known(gained) = self.gained_destinations(
            source,
            &declaration,
            &requirement,
            read,
            synced_tip,
            candidates,
        )?
        else {
            return Ok(uncertain_trait(old.into()));
        };

        finish_relocation(gained, &tip, finding, read, old_file, index, evidence)
    }

    fn gained_destinations<'p>(
        &mut self,
        source: &mut FindingSource<'_>,
        declaration: &Declaration,
        requirement: &TraitRequirement,
        read: &str,
        synced_tip: &str,
        candidates: impl Iterator<Item = &'p String>,
    ) -> Result<GainedDestinations, FindingSourceFailure> {
        let mut gained = BTreeMap::new();
        for path in candidates {
            let before = SourceLocation { commit: read, path };
            let after = SourceLocation {
                commit: synced_tip,
                path,
            };
            let prior = self.matching_at(source, declaration, requirement, before)?;
            let file = self.declarations.file(source, after)?;
            let items = self.matching_items(source, declaration, requirement, after, &file)?;
            let (MatchingItems::Known(prior), MatchingItems::Known(items)) = (prior, items) else {
                return Ok(GainedDestinations::Uncertain);
            };
            if prior.is_empty() && !items.is_empty() {
                gained.insert(path.clone(), GainedItems { file, items });
            }
        }
        Ok(GainedDestinations::Known(gained))
    }

    /// The items of the file at `location` that are the declaration, with the trait the
    /// requirement names.
    pub(crate) fn matching_at(
        &mut self,
        source: &mut FindingSource<'_>,
        declaration: &Declaration,
        requirement: &TraitRequirement,
        location: SourceLocation<'_>,
    ) -> Result<MatchingItems, FindingSourceFailure> {
        let file = self.declarations.file(source, location)?;
        self.matching_items(source, declaration, requirement, location, &file)
    }

    fn matching_items(
        &mut self,
        source: &mut FindingSource<'_>,
        declaration: &Declaration,
        requirement: &TraitRequirement,
        location: SourceLocation<'_>,
        file: &DeclarationFile,
    ) -> Result<MatchingItems, FindingSourceFailure> {
        let mut matched = Vec::new();
        for (index, candidate) in file.items().iter().enumerate() {
            if !declaration.matches(candidate) {
                continue;
            }
            let identity = match requirement {
                TraitRequirement::Unrestricted => TraitIdentity::Same,
                TraitRequirement::Required(required) => self.trait_reader.compare(
                    source,
                    &mut self.declarations,
                    required.site(),
                    TraitSite {
                        location,
                        membership: candidate.implemented_trait().into(),
                    },
                )?,
            };
            match identity {
                TraitIdentity::Same => matched.push(index),
                TraitIdentity::Different => {},
                TraitIdentity::Uncertain => return Ok(MatchingItems::Uncertain),
            }
        }
        Ok(MatchingItems::Known(matched))
    }
}

fn finish_relocation(
    gained: BTreeMap<String, GainedItems>,
    tip: &RepositoryTip,
    finding: &HeldAnchor,
    read: &str,
    old_file: Rc<DeclarationFile>,
    index: usize,
    evidence: &RelocationEvidence,
) -> Result<SameDeclaration, RelocationFailure> {
    let old = &old_file.items()[index];
    let declaration = Declaration::from(old);
    if gained.len() > 1 || gained.values().any(|gain| gain.items.len() > 1) {
        return Ok(SameDeclaration::SeveralDestinations {
            declaration,
            paths: gained.into_keys().collect(),
        });
    }
    let destination = gained.into_iter().next();
    if let Some((path, _)) = &destination
        && matches!(evidence.finding, RelocationFindingState::Filed { .. })
        && tip.crate_of(path) != tip.crate_of(&finding.path)
    {
        return Ok(SameDeclaration::AnotherCrate {
            declaration,
            path: path.clone(),
        });
    }
    match &evidence.build {
        LinkedBuild::Open { ticket } => {
            return Ok(SameDeclaration::OpenBuild {
                ticket: ticket.clone(),
            });
        },
        LinkedBuild::Missing { ticket } => {
            return Err(RelocationFailure::UnknownBuild(ticket.clone()));
        },
        LinkedBuild::NoOpenBuild => {},
    }
    let Some((path, gain)) = destination else {
        return Ok(SameDeclaration::NoDestination {
            declaration,
            original: AnchoredItem {
                file: old_file,
                index,
                read: read.to_owned(),
            },
        });
    };
    let item = &gain.file.items()[gain.items[0]];
    Ok(
        match moved_site(&old_file, old, finding, &gain.file, item) {
            MovedSite::Line(line) => SameDeclaration::Moved {
                declaration,
                path,
                line,
            },
            MovedSite::StatementRefused(reason) => SameDeclaration::StatementNotFollowed {
                declaration,
                path,
                reason,
            },
        },
    )
}

/// A statement line follows only to the one statement line of the destination item that holds
/// the finding's text; another line follows by its place in the item.
pub(crate) fn moved_site(
    old_file: &DeclarationFile,
    old: &DeclaredItem,
    finding: &HeldAnchor,
    file: &DeclarationFile,
    item: &DeclaredItem,
) -> MovedSite {
    if !old_file.owns_statement(old, finding.line) {
        return MovedSite::Line(file.moved_line(item, old_file, old, finding.line));
    }
    match file.statement_location(item, &finding.text) {
        StatementLocation::Matched(line) => MovedSite::Line(line),
        StatementLocation::Refused(reason) => MovedSite::StatementRefused(reason),
    }
}

const fn not_read() -> SameDeclaration {
    SameDeclaration::NotRead {
        cause: NotReadCause::Unread,
    }
}

const fn uncertain_trait(declaration: Declaration) -> SameDeclaration {
    SameDeclaration::TraitIdentityUncertain { declaration }
}
