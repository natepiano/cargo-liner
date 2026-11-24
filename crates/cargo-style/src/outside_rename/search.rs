//! Examine candidates and prove the destination of one source declaration.

use std::collections::HashSet;
use std::rc::Rc;

use super::ExaminedCandidates;
use super::ItemAtRevision;
use super::OriginalSite;
use super::OutsideCandidate;
use super::OutsideCandidates;
use super::OutsideFailure;
use super::OutsideReasons;
use super::OutsideRefusal;
use super::OutsideSearch;
use super::PassingItem;
use super::ProvenRename;
use super::RenameEvidence;
use super::RenameProof;
use super::SourceResult;
use super::body_comparison;
use super::body_comparison::ReadableBody;
use crate::declaration_source::DeclarationFile;
use crate::declaration_source::SourceLocation;
use crate::outside_mapping::MappedSite;
use crate::outside_mapping::OutsidePatch;
use crate::same_declaration::Declaration;
use crate::trait_identity::TraitIdentity;

#[derive(Clone, Copy)]
pub(super) enum NameInCrate {
    Declared,
    Absent,
}

impl OutsideSearch<'_, '_> {
    pub(super) fn crate_declares(
        &mut self,
        crate_name: &str,
        name: &str,
    ) -> SourceResult<NameInCrate> {
        let key = (
            self.commit.to_owned(),
            crate_name.to_owned(),
            name.to_owned(),
        );
        if let Some(&found) = self.reader.crate_names.get(&key) {
            return Ok(found);
        }
        let mut found = NameInCrate::Absent;
        for path in self
            .tip
            .crate_rust_paths(crate_name)
            .filter(|path| self.tip.has_rust_path(path))
        {
            let file = self.file(
                self.commit,
                path,
                "check for the original name in the crate",
            )?;
            if file.items().iter().any(|item| item.name == name) {
                found = NameInCrate::Declared;
                break;
            }
        }
        self.reader.crate_names.insert(key, found);
        Ok(found)
    }

    pub(super) fn search(
        &mut self,
        site: OriginalSite<'_>,
        target: &Declaration,
    ) -> SourceResult<RenameEvidence> {
        let mapped = self.map(site)?;
        let old = site.declaration;
        let crate_name = self.tip.crate_of(old.location.path);
        if matches!(
            self.crate_declares(crate_name, &old.item.name)?,
            NameInCrate::Declared
        ) {
            return Ok(RenameEvidence::Refused(OutsideRefusal {
                mapped,
                candidates: OutsideCandidates::None,
                reasons: OutsideFailure::OldNamePresent.into(),
            }));
        }
        let mut examined = Vec::new();
        let mut passing = Vec::new();
        let old_body = body_comparison::read_body(site.file, old.item);
        for path in self.changed_paths(old.location.path) {
            let file = self.file(self.commit, path, "read candidate declarations")?;
            for (index, item) in file.items().iter().enumerate() {
                if target.keyword() != item.keyword()
                    || old.item.name == item.name
                    || (!target.has_owner(item.owner()) && item.owner() != old.item.owner())
                {
                    continue;
                }
                let candidate = ItemAtRevision {
                    location: SourceLocation {
                        commit: self.commit,
                        path,
                    },
                    item,
                };
                let row = self.examine(site, &old_body, candidate, &file, target)?;
                if row.checks.is_empty() {
                    passing.push(PassingItem {
                        path,
                        file: Rc::clone(&file),
                        index,
                    });
                }
                examined.push(row);
            }
        }
        examined.sort_by(|left, right| {
            (
                &left.path,
                left.declaration.keyword(),
                left.declaration.name(),
                left.line,
            )
                .cmp(&(
                    &right.path,
                    right.declaration.keyword(),
                    right.declaration.name(),
                    right.line,
                ))
        });
        Ok(finish_search(site, mapped, examined, &passing))
    }

    fn examine(
        &mut self,
        original: OriginalSite<'_>,
        old_body: &ReadableBody,
        candidate: ItemAtRevision<'_>,
        file: &DeclarationFile,
        target: &Declaration,
    ) -> SourceResult<OutsideCandidate> {
        let old = original.declaration;
        let item = candidate.item;
        let mut checks = Vec::new();
        if !target.has_owner(item.owner())
            || (target.has_owner(old.item.owner())
                && !matches!(self.type_identity(old, candidate)?, TraitIdentity::Same))
        {
            checks.push(OutsideFailure::WrongOwner);
        } else if !old.item.is_module_item() {
            match self.trait_identity(old, candidate)? {
                TraitIdentity::Same => {},
                TraitIdentity::Different => checks.push(OutsideFailure::WrongOwner),
                TraitIdentity::Uncertain => checks.push(OutsideFailure::TraitUncertain),
            }
        }
        let before = self.file(
            self.read,
            candidate.location.path,
            "read prior candidate declarations",
        )?;
        if before
            .items()
            .iter()
            .any(|prior| prior.same_declaration(item))
        {
            checks.push(OutsideFailure::NotGained);
        }
        let body = body_comparison::read_body(file, item);
        let old_nonblank_count = match old_body {
            ReadableBody::Lines(lines) => lines.count,
            ReadableBody::Unreadable => 0,
        };
        let mut unchanged = 0;
        match (old_body, body) {
            (ReadableBody::Lines(old), ReadableBody::Lines(new))
                if old.count > 0 && new.count > 0 =>
            {
                unchanged = body_comparison::preserved_lines(
                    original.file.name_sites(original.declaration.item),
                    old,
                    &new,
                    &item.name,
                );
                if unchanged < old.count.div_ceil(2) {
                    checks.push(OutsideFailure::BodyFailed);
                }
            },
            _ => checks.push(OutsideFailure::BodyUnreadable),
        }
        Ok(OutsideCandidate {
            path: candidate.location.path.to_owned(),
            declaration: item.into(),
            line: item.keyword_line(),
            unchanged,
            old_nonblank_count,
            checks,
        })
    }
}

fn finish_search(
    site: OriginalSite<'_>,
    mapped: MappedSite,
    examined: Vec<OutsideCandidate>,
    passing: &[PassingItem<'_>],
) -> RenameEvidence {
    let mut examined = examined.into_iter();
    let candidates = examined.next().map_or(OutsideCandidates::None, |first| {
        OutsideCandidates::Examined {
            items: ExaminedCandidates {
                first,
                rest: examined.collect(),
            },
        }
    });
    let [selected] = passing else {
        let reasons = rejected_candidates(passing, &candidates);
        return RenameEvidence::Refused(OutsideRefusal {
            mapped,
            candidates,
            reasons,
        });
    };
    let item = &selected.file.items()[selected.index];
    let path = selected.path;
    let declaration = Declaration::from(item);
    let (proof, located) = if path == site.declaration.location.path {
        let MappedSite::MappedDeclaration { site: mapped_site } = &mapped else {
            return RenameEvidence::Refused(OutsideRefusal {
                mapped,
                candidates,
                reasons: OutsideFailure::Unmapped.into(),
            });
        };
        if mapped_site.declaration != declaration || !item.lines().contains(&mapped_site.line) {
            return RenameEvidence::Refused(OutsideRefusal {
                mapped,
                candidates,
                reasons: OutsideFailure::Unmapped.into(),
            });
        }
        // OutsideSearch::map reads the Myers patch for this path.
        // finish_search reuses that coordinate, as Python reuses the cached patch.
        (
            RenameProof::MappedSiteProof {
                site: mapped_site.clone(),
            },
            mapped,
        )
    } else {
        let diff = OutsidePatch::from_normalized_lines(site.file, &selected.file);
        (
            RenameProof::GainedFileProof {
                path:        path.to_owned(),
                declaration: declaration.clone(),
            },
            diff.map(path, site.line, &selected.file),
        )
    };
    let line = match located {
        MappedSite::MappedDeclaration { site: mapped_site }
            if mapped_site.declaration == declaration
                && item.lines().contains(&mapped_site.line) =>
        {
            mapped_site.line
        },
        _ => selected
            .file
            .moved_line(item, site.file, site.declaration.item, site.line),
    };
    RenameEvidence::Proven(ProvenRename {
        declaration,
        path: path.to_owned(),
        line,
        proof,
    })
}

fn rejected_candidates(
    passing: &[PassingItem<'_>],
    candidates: &OutsideCandidates,
) -> OutsideReasons {
    if let Some(first) = passing.first() {
        return if passing.iter().any(|item| item.path != first.path) {
            OutsideFailure::SeveralFiles
        } else {
            OutsideFailure::AmbiguousItems
        }
        .into();
    }
    let mut reasons = OutsideReasons::from(OutsideFailure::NoPassingFiles);
    if let OutsideCandidates::Examined { items } = candidates {
        let mut seen = HashSet::new();
        reasons.rest.extend(
            std::iter::once(&items.first)
                .chain(&items.rest)
                .flat_map(|item| item.checks.iter().copied())
                .filter(|check| seen.insert(*check)),
        );
    }
    reasons
}
