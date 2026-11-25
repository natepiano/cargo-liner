//! Prove a rename from committed declarations and retain the evidence from refused searches.

mod body_comparison;
mod search;
mod type_identity;

use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

use serde::Serialize;
use serde::Serializer;

use self::search::NameInCrate;
use self::type_identity::ExternNames;
use crate::declaration_source::DeclarationFile;
use crate::declaration_source::DeclarationSource;
use crate::declaration_source::SourceLocation;
use crate::declarations::DeclarationAt;
use crate::declarations::DeclarationOwner;
use crate::declarations::DeclaredItem;
use crate::declarations::ImplMembership;
use crate::declarations::ImplementedPath;
use crate::finding_anchor::FindingAnchor;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::NamingSearch;
use crate::ledger_read::HeldAnchor;
use crate::outside_bindings;
use crate::outside_bindings::ChildSite;
use crate::outside_mapping::MappedDeclaration;
use crate::outside_mapping::MappedSite;
use crate::outside_mapping::OutsidePatch;
use crate::same_declaration::AnchoredItem;
use crate::same_declaration::Declaration;
use crate::same_declaration::RelocationReader;
use crate::same_declaration::SameDeclaration;
use crate::tip_view::RepositoryTip;
use crate::trait_identity::TraitIdentity;
use crate::trait_identity::TraitReader;
use crate::trait_identity::TraitSite;

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum OutsideRename {
    NotApplicable,
    #[serde(untagged)]
    Read(Box<RenameEvidence>),
}

type SourceResult<T> = Result<T, OutsideSourceFailure>;

#[derive(Serialize)]
pub(crate) struct ProvenRename {
    declaration: Declaration,
    path:        String,
    line:        usize,
    proof:       RenameProof,
}

#[derive(Serialize)]
pub(crate) struct OutsideRefusal {
    mapped:     MappedSite,
    candidates: OutsideCandidates,
    reasons:    OutsideReasons,
}

impl ProvenRename {
    pub(crate) const fn declaration(&self) -> &Declaration { &self.declaration }

    pub(crate) const fn path(&self) -> &str { self.path.as_str() }

    pub(crate) const fn line(&self) -> usize { self.line }
}

impl OutsideRefusal {
    /// Whether the crate still declares the old name.
    pub(crate) fn old_name_present(&self) -> bool {
        self.reasons()
            .any(|reason| reason == OutsideFailure::OldNamePresent)
    }

    /// The declarations of the examined candidates that the destination gained.
    pub(crate) fn gained_declarations(&self) -> impl Iterator<Item = &Declaration> {
        let examined = match &self.candidates {
            OutsideCandidates::None => None,
            OutsideCandidates::Examined { items } => Some(items),
        };
        examined
            .into_iter()
            .flat_map(ExaminedCandidates::iter)
            .filter(|candidate| !candidate.checks.contains(&OutsideFailure::NotGained))
            .map(|candidate| &candidate.declaration)
    }

    fn reasons(&self) -> impl Iterator<Item = OutsideFailure> {
        std::iter::once(self.reasons.first).chain(self.reasons.rest.iter().copied())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{operation}")]
pub(crate) struct OutsideSourceFailure {
    pub(crate) path: String,
    operation:       &'static str,
    source:          FindingSourceFailure,
}

fn source_failure<'a>(
    operation: &'static str,
    path: &'a str,
) -> impl FnOnce(FindingSourceFailure) -> OutsideSourceFailure + 'a {
    move |source| {
        let (path, source) = match source {
            FindingSourceFailure::AtPath { path, source } => (path, *source),
            other => (path.to_owned(), other),
        };
        OutsideSourceFailure {
            path,
            operation,
            source,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RenameProof {
    MappedSiteProof {
        site: MappedDeclaration,
    },
    GainedFileProof {
        path:        String,
        declaration: Declaration,
    },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum OutsideCandidates {
    #[serde(rename = "no_candidates")]
    None,
    #[serde(rename = "examined_candidates")]
    Examined { items: ExaminedCandidates },
}

struct ExaminedCandidates {
    first: OutsideCandidate,
    rest:  Vec<OutsideCandidate>,
}

impl ExaminedCandidates {
    fn iter(&self) -> impl Iterator<Item = &OutsideCandidate> {
        std::iter::once(&self.first).chain(&self.rest)
    }
}

impl Serialize for ExaminedCandidates {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

struct OutsideReasons {
    first: OutsideFailure,
    rest:  Vec<OutsideFailure>,
}

impl From<OutsideFailure> for OutsideReasons {
    fn from(first: OutsideFailure) -> Self {
        Self {
            first,
            rest: Vec::new(),
        }
    }
}

impl Serialize for OutsideReasons {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(std::iter::once(&self.first).chain(&self.rest))
    }
}

#[derive(Serialize)]
struct OutsideCandidate {
    path:               String,
    declaration:        Declaration,
    line:               usize,
    unchanged:          usize,
    old_nonblank_count: usize,
    checks:             Vec<OutsideFailure>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq, Serialize)]
enum OutsideFailure {
    #[serde(rename = "old name still declared in crate")]
    OldNamePresent,
    #[serde(rename = "zero passing files")]
    NoPassingFiles,
    #[serde(rename = "several passing files")]
    SeveralFiles,
    #[serde(rename = "ambiguous destination items")]
    AmbiguousItems,
    #[serde(rename = "body check failed")]
    BodyFailed,
    #[serde(rename = "body unreadable or empty")]
    BodyUnreadable,
    #[serde(rename = "owner differs")]
    WrongOwner,
    #[serde(rename = "trait identity uncertain")]
    TraitUncertain,
    #[serde(rename = "declaration not gained")]
    NotGained,
    #[serde(rename = "mapped site not proved")]
    Unmapped,
    #[serde(rename = "child binding or scope not proved")]
    ChildUnproved,
}

#[derive(Default)]
pub(crate) struct OutsideReader {
    crate_names:  HashMap<(String, String, String), NameInCrate>,
    extern_names: HashMap<(String, String), Rc<ExternNames>>,
}

struct OutsideSearch<'a, 'repo> {
    source:       &'a mut FindingSource<'repo>,
    declarations: &'a mut DeclarationSource,
    trait_reader: &'a mut TraitReader,
    reader:       &'a mut OutsideReader,
    tip:          &'a RepositoryTip,
    read:         &'a str,
    commit:       &'a str,
    changed:      &'a HashSet<String>,
}

#[derive(Clone, Copy)]
struct ItemAtRevision<'a> {
    location: SourceLocation<'a>,
    item:     &'a DeclaredItem,
}

#[derive(Clone, Copy)]
struct OriginalSite<'a> {
    declaration: ItemAtRevision<'a>,
    file:        &'a DeclarationFile,
    line:        usize,
}

struct PassingItem<'a> {
    path:  &'a str,
    file:  Rc<DeclarationFile>,
    index: usize,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RenameEvidence {
    Proven(ProvenRename),
    #[serde(rename = "not_proven")]
    Refused(OutsideRefusal),
}

struct OutsideRevision {
    tip:     Rc<RepositoryTip>,
    changed: Rc<HashSet<String>>,
}

impl OutsideRevision {
    fn read(
        source: &mut FindingSource<'_>,
        commit: &str,
        location: SourceLocation<'_>,
    ) -> SourceResult<Self> {
        let tip = source
            .tip(commit)
            .map_err(source_failure("read the synced tip", location.path))?;
        let changed = source
            .changed_paths(location.commit, commit)
            .map_err(source_failure(
                "read changes since the source commit",
                location.path,
            ))?;
        Ok(Self { tip, changed })
    }
}

impl OutsideReader {
    pub(crate) fn prove(
        &mut self,
        source: &mut FindingSource<'_>,
        relocation: &mut RelocationReader,
        commit: &str,
        finding: &HeldAnchor,
        anchor: &FindingAnchor,
        same: &SameDeclaration,
    ) -> SourceResult<OutsideRename> {
        if !matches!(
            same,
            SameDeclaration::SeveralDestinations { .. } | SameDeclaration::AnotherCrate { .. }
        ) {
            return Ok(OutsideRename::NotApplicable);
        }
        let FindingAnchor::Anchored(anchor) = anchor else {
            return Ok(OutsideRename::NotApplicable);
        };
        let location = SourceLocation {
            commit: &anchor.commit,
            path:   &finding.path,
        };
        let revision = OutsideRevision::read(source, commit, location)?;
        let mut search = self.search(source, relocation, &revision, location.commit, commit);
        let file = search.file(
            location.commit,
            location.path,
            "read the original declarations",
        )?;
        let DeclarationAt::Named(item) = file.at(finding.line) else {
            return Ok(OutsideRename::NotApplicable);
        };
        let original = OriginalSite {
            declaration: ItemAtRevision { location, item },
            file:        &file,
            line:        finding.line,
        };
        let refusal = match search.prove(original)? {
            RenameEvidence::Proven(_) => search.refuse(original, OutsideFailure::Unmapped)?,
            RenameEvidence::Refused(refusal) => refusal,
        };
        Ok(OutsideRename::Read(Box::new(RenameEvidence::Refused(
            refusal,
        ))))
    }

    pub(crate) fn prove_relocated(
        &mut self,
        source: &mut FindingSource<'_>,
        relocation: &mut RelocationReader,
        commit: &str,
        finding: &HeldAnchor,
        original: &AnchoredItem,
    ) -> SourceResult<RenameEvidence> {
        let location = SourceLocation {
            commit: original.read(),
            path:   &finding.path,
        };
        let revision = OutsideRevision::read(source, commit, location)?;
        let mut search = self.search(source, relocation, &revision, location.commit, commit);
        search.prove(OriginalSite {
            declaration: ItemAtRevision {
                location,
                item: original.item(),
            },
            file:        original.file(),
            line:        finding.line,
        })
    }

    fn search<'a, 'repo>(
        &'a mut self,
        source: &'a mut FindingSource<'repo>,
        relocation: &'a mut RelocationReader,
        revision: &'a OutsideRevision,
        read: &'a str,
        commit: &'a str,
    ) -> OutsideSearch<'a, 'repo> {
        let (declarations, trait_reader) = relocation.source_readers();
        OutsideSearch {
            source,
            declarations,
            trait_reader,
            reader: self,
            tip: &revision.tip,
            read,
            commit,
            changed: &revision.changed,
        }
    }
}

impl<'a> OutsideSearch<'a, '_> {
    fn changed_paths(&self, path: &str) -> impl Iterator<Item = &'a str> + use<'a> {
        let tip = self.tip;
        let changed = self.changed;
        tip.crate_rust_paths(tip.crate_of(path))
            .filter(move |path| tip.has_rust_path(path) && changed.contains(*path))
    }

    fn prove(&mut self, site: OriginalSite<'_>) -> SourceResult<RenameEvidence> {
        let mut proof = match self.prove_with_owner(site)? {
            RenameEvidence::Proven(proof) => proof,
            evidence @ RenameEvidence::Refused(_) => return Ok(evidence),
        };
        let destination = self.file(self.commit, &proof.path, "read the child site destination")?;
        match outside_bindings::prove(
            site.file,
            site.declaration.item,
            site.line,
            &destination,
            &proof.declaration,
        ) {
            ChildSite::OwnerHeader => {},
            ChildSite::Retained(line) => proof.line = line,
            ChildSite::Unproved => {
                return Ok(RenameEvidence::Refused(
                    self.refuse(site, OutsideFailure::ChildUnproved)?,
                ));
            },
        }
        Ok(RenameEvidence::Proven(proof))
    }

    fn prove_with_owner(&mut self, site: OriginalSite<'_>) -> SourceResult<RenameEvidence> {
        let old = site.declaration;
        let crate_name = self.tip.crate_of(old.location.path);
        if let ImplMembership::Path(membership) = old.item.implemented_trait()
            && let Some(name) = membership.path.last()
            && matches!(self.crate_declares(crate_name, name)?, NameInCrate::Absent)
        {
            return self.prove_trait_rename(site, membership, name);
        }
        if let DeclarationOwner::ImplOrTrait(name) = old.item.owner() {
            let mut owners = site.file.items().iter().filter(|item| {
                &item.name == name
                    && matches!(item.keyword(), "struct" | "enum" | "union" | "trait")
            });
            if let Some(owner) = owners.next()
                && owners.next().is_none()
                && matches!(self.crate_declares(crate_name, name)?, NameInCrate::Absent)
            {
                return self.prove_owner_rename(site, owner);
            }
        }
        if let DeclarationAt::Named(parent) = site.file.lexical_parent(old.item)
            && parent.keyword() == "fn"
        {
            return self.prove_owner_rename(site, parent);
        }
        self.search(site, &Declaration::from(old.item))
    }

    fn prove_owner_rename(
        &mut self,
        site: OriginalSite<'_>,
        owner: &DeclaredItem,
    ) -> SourceResult<RenameEvidence> {
        let old = site.declaration;
        let owner_site = OriginalSite {
            declaration: ItemAtRevision { item: owner, ..old },
            line: owner.keyword_line(),
            ..site
        };
        let proof = match self.search(owner_site, &Declaration::from(owner))? {
            RenameEvidence::Proven(proof) => proof,
            evidence @ RenameEvidence::Refused(_) => return Ok(evidence),
        };
        let location = SourceLocation {
            commit: self.commit,
            path:   &proof.path,
        };
        let destination = self.file(location.commit, location.path, "read the renamed owner")?;
        let target = Declaration::from(old.item).under_owner(proof.declaration.name());
        let renamed_type;
        let mut owners = destination
            .items()
            .iter()
            .filter(|item| proof.declaration.matches(item));
        let required_type = match (old.item.implemented_type(), owners.next(), owners.next()) {
            (ImplMembership::Path(implemented), Some(renamed), None) => {
                renamed_type = membership_with_name(implemented, proof.declaration.name(), renamed);
                &renamed_type
            },
            (membership, _, _) => membership,
        };
        let mut children = Vec::new();
        for candidate in destination
            .items()
            .iter()
            .filter(|item| target.matches(item))
        {
            let same = self.compare_types(
                location,
                required_type,
                location,
                candidate.implemented_type(),
            )?;
            if matches!(same, TraitIdentity::Same) {
                children.push(candidate);
            }
        }
        if let [child] = children.as_slice()
            && matches!(
                self.trait_identity(
                    old,
                    ItemAtRevision {
                        location,
                        item: child
                    }
                )?,
                TraitIdentity::Same
            )
        {
            return Ok(RenameEvidence::Proven(ProvenRename {
                declaration: (*child).into(),
                line: destination.moved_line(child, site.file, old.item, site.line),
                ..proof
            }));
        }
        if children.is_empty() {
            match self.search(site, &target)? {
                RenameEvidence::Proven(child_proof) if child_proof.path == proof.path => {
                    let mut candidates = destination
                        .items()
                        .iter()
                        .filter(|item| child_proof.declaration.matches(item));
                    if let Some(candidate) = candidates.next()
                        && candidates.next().is_none()
                        && matches!(
                            self.compare_types(
                                location,
                                required_type,
                                location,
                                candidate.implemented_type()
                            )?,
                            TraitIdentity::Same
                        )
                    {
                        return Ok(RenameEvidence::Proven(child_proof));
                    }
                },
                RenameEvidence::Proven(_) => {},
                evidence @ RenameEvidence::Refused(_) => return Ok(evidence),
            }
        }
        Ok(RenameEvidence::Refused(
            self.refuse(site, OutsideFailure::ChildUnproved)?,
        ))
    }

    fn prove_trait_rename(
        &mut self,
        site: OriginalSite<'_>,
        membership: &ImplementedPath,
        name: &str,
    ) -> SourceResult<RenameEvidence> {
        let old = site.declaration;
        let failed = self.refuse(site, OutsideFailure::ChildUnproved)?;
        let original_tip = self.source.tip(self.read).map_err(source_failure(
            "read the trait owner's original tip",
            old.location.path,
        ))?;
        let crate_name = self.tip.crate_of(old.location.path);
        let paths = self
            .source
            .paths_naming(self.read, name, NamingSearch::DeclarationProof)
            .map_err(source_failure(
                "find files that name the original trait",
                old.location.path,
            ))?;
        let mut owners = Vec::new();
        for path in paths
            .iter()
            .filter(|path| original_tip.crate_of(path) == crate_name)
        {
            let location = SourceLocation {
                commit: self.read,
                path,
            };
            let file = self.file(
                location.commit,
                location.path,
                "read a possible original trait owner",
            )?;
            for (index, item) in file.items().iter().enumerate() {
                if item.keyword() != "trait" || item.name != name {
                    continue;
                }
                let owner = membership_with_name(membership, name, item);
                if matches!(
                    self.compare_traits(
                        old.location,
                        old.item.implemented_trait(),
                        location,
                        &owner
                    )?,
                    TraitIdentity::Same
                ) {
                    owners.push(PassingItem {
                        path,
                        file: Rc::clone(&file),
                        index,
                    });
                }
            }
        }
        let [owner] = owners.as_slice() else {
            return Ok(RenameEvidence::Refused(failed));
        };
        let item = &owner.file.items()[owner.index];
        let owner_site = OriginalSite {
            declaration: ItemAtRevision {
                item,
                location: SourceLocation {
                    commit: self.read,
                    path:   owner.path,
                },
            },
            file:        &owner.file,
            line:        item.keyword_line(),
        };
        let proof = match self.search(owner_site, &Declaration::from(item))? {
            RenameEvidence::Proven(proof) => proof,
            evidence @ RenameEvidence::Refused(_) => return Ok(evidence),
        };
        let location = SourceLocation {
            commit: self.commit,
            path:   &proof.path,
        };
        let destination = self.file(
            location.commit,
            location.path,
            "read the renamed trait owner",
        )?;
        let mut renamed = destination
            .items()
            .iter()
            .filter(|item| proof.declaration.matches(item));
        let Some(owner) = renamed.next() else {
            return Ok(RenameEvidence::Refused(failed));
        };
        if renamed.next().is_some() {
            return Ok(RenameEvidence::Refused(failed));
        }
        let trait_requirement = membership_with_name(membership, proof.declaration.name(), owner);
        self.prove_trait_child(site, proof, &trait_requirement, failed)
    }

    fn prove_trait_child(
        &mut self,
        site: OriginalSite<'_>,
        proof: ProvenRename,
        required: &ImplMembership,
        failed: OutsideRefusal,
    ) -> SourceResult<RenameEvidence> {
        let old = site.declaration;
        let trait_location = SourceLocation {
            commit: self.commit,
            path:   &proof.path,
        };
        let mut children = Vec::new();
        for path in self.changed_paths(old.location.path) {
            let file = self.file(self.commit, path, "read a possible trait child")?;
            let location = SourceLocation {
                commit: self.commit,
                path,
            };
            for (index, item) in file.items().iter().enumerate() {
                if old.item.same_declaration(item)
                    && matches!(
                        self.type_identity(old, ItemAtRevision { location, item })?,
                        TraitIdentity::Same
                    )
                    && matches!(
                        self.compare_traits(
                            trait_location,
                            required,
                            location,
                            item.implemented_trait()
                        )?,
                        TraitIdentity::Same
                    )
                {
                    children.push(PassingItem {
                        path,
                        file: Rc::clone(&file),
                        index,
                    });
                }
            }
        }
        let [child] = children.as_slice() else {
            return Ok(RenameEvidence::Refused(failed));
        };
        let item = &child.file.items()[child.index];
        Ok(RenameEvidence::Proven(ProvenRename {
            declaration: item.into(),
            path:        child.path.to_owned(),
            line:        child.file.moved_line(item, site.file, old.item, site.line),
            proof:       proof.proof,
        }))
    }

    fn file(
        &mut self,
        commit: &str,
        path: &str,
        operation: &'static str,
    ) -> SourceResult<Rc<DeclarationFile>> {
        self.declarations
            .file(self.source, SourceLocation { commit, path })
            .map_err(source_failure(operation, path))
    }

    fn map(&mut self, site: OriginalSite<'_>) -> SourceResult<MappedSite> {
        let path = site.declaration.location.path;
        let after = self.file(self.commit, path, "read declarations at the mapped site")?;
        let diff = self
            .source
            .myers_diff(self.read, self.commit, path)
            .map_err(source_failure(
                "read the Myers patch for the mapped site",
                path,
            ))?;
        Ok(OutsidePatch::from_myers(diff).map(path, site.line, &after))
    }

    fn refuse(
        &mut self,
        site: OriginalSite<'_>,
        reason: OutsideFailure,
    ) -> SourceResult<OutsideRefusal> {
        let mapped = self.map(site)?;
        Ok(OutsideRefusal {
            mapped,
            candidates: OutsideCandidates::None,
            reasons: reason.into(),
        })
    }

    fn trait_identity(
        &mut self,
        left: ItemAtRevision<'_>,
        right: ItemAtRevision<'_>,
    ) -> SourceResult<TraitIdentity> {
        self.compare_traits(
            left.location,
            left.item.implemented_trait(),
            right.location,
            right.item.implemented_trait(),
        )
    }

    fn compare_traits(
        &mut self,
        left: SourceLocation<'_>,
        before: &ImplMembership,
        right: SourceLocation<'_>,
        after: &ImplMembership,
    ) -> SourceResult<TraitIdentity> {
        self.trait_reader
            .compare(
                self.source,
                self.declarations,
                TraitSite {
                    location:   left,
                    membership: before.into(),
                },
                TraitSite {
                    location:   right,
                    membership: after.into(),
                },
            )
            .map_err(source_failure(
                "compare implemented trait identities",
                right.path,
            ))
    }
}

fn membership_with_name(old: &ImplementedPath, name: &str, item: &DeclaredItem) -> ImplMembership {
    ImplMembership::Path(ImplementedPath {
        path:      vec![name.to_owned()],
        arguments: old.arguments.clone(),
        position:  item.import_position().clone(),
    })
}
