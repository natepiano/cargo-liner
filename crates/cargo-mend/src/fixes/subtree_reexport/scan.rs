use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;

use super::ancestor;
use super::crate_files;
use super::crate_files::CrateFiles;
use super::owner_edit;
use super::owner_edit::EmptiedOwner;
use super::owner_edit::OwnerFact;
use super::owner_edit::RegionEdit;
use crate::config::DiagnosticCode;
use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::CallerBuilds;
use crate::fixes::facade_redirect::FacadeRedirect;
use crate::fixes::facade_redirect::ModuleAliases;
use crate::fixes::facade_redirect::RedirectTarget;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::AncestorReexport;
use crate::reporting::FixSupport;
use crate::reporting::Report;
use crate::reporting::SubtreeReexportFixFact;
use crate::selection::Selection;

/// The `pub_use_outside_subtree` fixes of one pass.
pub(in crate::fixes) struct SubtreeReexportScan {
    /// Tagged `FixKind::Import` when combined, so the notice counts the edits
    /// this pass writes.
    pub(in crate::fixes) fixes:   ValidatedFixSet,
    /// Fixable findings this pass could not rewrite: no fact, a fact whose
    /// `use` item or leaf the source does not hold, an owner file with several
    /// parents, an owner module another fact re-exports, or edits that overlap
    /// other edits.
    pub(in crate::fixes) skipped: usize,
}

/// A fix and the facts it belongs to; a caller fix belongs to none.
struct OwnedFix {
    fix:   UseFix,
    facts: Vec<usize>,
}

/// The fixes one round builds from the facts still in play.
struct Round {
    fixes:     Vec<OwnedFix>,
    unmatched: BTreeSet<usize>,
}

pub(in crate::fixes) fn scan_selection(
    selection: &Selection,
    report: &Report,
) -> Result<SubtreeReexportScan> {
    let root = &selection.analysis_root;
    let (facts, mut skipped) = fixable_facts(report);
    let crates = crate_file_sets(root, report, &facts);
    let mut excluded = facts
        .iter()
        .enumerate()
        .filter(|(_, fact)| {
            crates
                .get(&fact.crate_root_file)
                .is_some_and(|files| files.has_several_parents(&owner_file(root, fact)))
        })
        .map(|(index, _)| index)
        .collect::<BTreeSet<_>>();
    excluded.extend(owners_behind_other_reexports(&facts));

    // Each round drops the facts it could not place or whose edits overlap
    // another fix, then rebuilds every edit without them.
    let fixes = loop {
        let round = build_round(root, &facts, &crates, &excluded)?;
        let mut rejected = round.unmatched.clone();
        rejected.extend(overlapping_facts(&round.fixes));
        if rejected.is_empty() {
            break round.fixes;
        }
        excluded.extend(rejected);
    };

    let applied = facts.len() - excluded.len();
    skipped += excluded.len();
    let mut fixes = fixes.into_iter().map(|owned| owned.fix).collect::<Vec<_>>();
    facade_redirect::dedup_fixes(&mut fixes);
    match ValidatedFixSet::try_from(fixes) {
        Ok(fixes) => Ok(SubtreeReexportScan { fixes, skipped }),
        // Two caller rewrites that overlap belong to no single fact, so the
        // whole pass stands down and every fact counts as skipped.
        Err(_) => Ok(SubtreeReexportScan {
            fixes:   ValidatedFixSet::try_from(Vec::<UseFix>::new())?,
            skipped: skipped + applied,
        }),
    }
}

/// The fact of every fixable finding, and the count of fixable findings with
/// no single fact.
fn fixable_facts(report: &Report) -> (Vec<&SubtreeReexportFixFact>, usize) {
    let mut by_site: BTreeMap<(&str, usize, usize), Vec<&SubtreeReexportFixFact>> = BTreeMap::new();
    for fact in report.facts.subtree_reexport_fix_facts.iter() {
        by_site
            .entry((fact.use_path.as_str(), fact.use_line, fact.use_column))
            .or_default()
            .push(fact);
    }
    let mut facts = Vec::new();
    let mut skipped = 0;
    for finding in &report.findings {
        if finding.diagnostic_code != DiagnosticCode::PubUseOutsideSubtree
            || finding.fix_support != FixSupport::PubUseOutsideSubtree
        {
            continue;
        }
        match by_site
            .get(&(finding.path.as_str(), finding.line, finding.column))
            .map(Vec::as_slice)
        {
            Some([fact]) => facts.push(*fact),
            _ => skipped += 1,
        }
    }
    (facts, skipped)
}

/// The files of every crate root a fact belongs to, keyed by the fact's
/// `crate_root_file`.
fn crate_file_sets(
    root: &Path,
    report: &Report,
    facts: &[&SubtreeReexportFixFact],
) -> BTreeMap<String, CrateFiles> {
    let crate_roots = facts
        .iter()
        .map(|fact| fact.crate_root_file.clone())
        .collect::<BTreeSet<_>>();
    crate_roots
        .into_iter()
        .map(|crate_root| {
            let mounts = report
                .facts
                .module_mount_facts
                .iter()
                .filter(|mount| mount.crate_root_file == crate_root)
                .map(|mount| (root.join(&mount.file), mount.module_path.clone()));
            let files = CrateFiles::resolve(&root.join(&crate_root), mounts);
            (crate_root, files)
        })
        .collect()
}

/// The facts whose owner module another fact re-exports, itself or through
/// one of its ancestors. A caller path through that other re-export is
/// redirected to the owner module's real path, which still names the item only
/// while this fact's re-export stays, so this fact waits for the next run.
fn owners_behind_other_reexports(facts: &[&SubtreeReexportFixFact]) -> BTreeSet<usize> {
    facts
        .iter()
        .enumerate()
        .filter(|(index, fact)| {
            facts.iter().enumerate().any(|(other_index, other)| {
                other_index != *index
                    && other.crate_root_file == fact.crate_root_file
                    && fact.owner_module.starts_with(&other.source_module)
                    && fact.owner_module.get(other.source_module.len())
                        == Some(&other.exported_name)
            })
        })
        .map(|(index, _)| index)
        .collect()
}

fn owner_file(root: &Path, fact: &SubtreeReexportFixFact) -> PathBuf {
    crate_files::canonical(&root.join(&fact.use_path))
}

fn build_round(
    root: &Path,
    facts: &[&SubtreeReexportFixFact],
    crates: &BTreeMap<String, CrateFiles>,
    excluded: &BTreeSet<usize>,
) -> Result<Round> {
    let active = (0..facts.len())
        .filter(|index| !excluded.contains(index))
        .collect::<Vec<_>>();
    let mut round = Round {
        fixes:     Vec::new(),
        unmatched: BTreeSet::new(),
    };

    // Step 2 runs first: the callers, per crate root, with that root's
    // redirects. It reports the owner modules a caller's `use` still names,
    // which step 1 must not delete, and the builds that compile the callers
    // each ancestor re-export serves.
    let mut outside_used: BTreeMap<usize, CallerBuilds> = BTreeMap::new();
    let mut named_owners = BTreeSet::new();
    for (crate_root, files) in crates {
        let indexes = active
            .iter()
            .copied()
            .filter(|&index| &facts[index].crate_root_file == crate_root)
            .collect::<Vec<_>>();
        if indexes.is_empty() {
            continue;
        }
        let redirects = indexes
            .iter()
            .map(|&index| redirect(facts[index]))
            .collect::<Vec<_>>();
        let module_aliases = ModuleAliases::collect(files.scannable())?;
        for (file, module_path) in files.scannable() {
            let result = facade_redirect::redirect_callers_in_file(
                file,
                module_path.to_vec(),
                &redirects,
                &module_aliases,
            )?;
            let file_test_only = files.is_test_only(file);
            for (&slot, &builds) in &result.outside_used {
                let builds = if file_test_only {
                    CallerBuilds::TestOnly
                } else {
                    builds
                };
                let widest = outside_used.entry(indexes[slot]).or_insert(builds);
                *widest = (*widest).max(builds);
            }
            named_owners.extend(
                result
                    .named_owner_modules
                    .into_iter()
                    .map(|module| (crate_root.as_str(), module)),
            );
            round
                .fixes
                .extend(result.fixes.into_iter().map(|fix| OwnedFix {
                    fix,
                    facts: Vec::new(),
                }));
        }
    }

    // Step 1: the owner edits, one region per owner file and owner module.
    let mut regions: BTreeMap<(PathBuf, &[String]), Vec<OwnerFact<'_>>> = BTreeMap::new();
    for &index in &active {
        let fact = facts[index];
        regions
            .entry((owner_file(root, fact), fact.owner_module.as_slice()))
            .or_default()
            .push(OwnerFact { index, fact });
    }
    for ((file, owner_module), region) in &regions {
        let crate_root = region[0].fact.crate_root_file.as_str();
        let Some(files) = crates.get(crate_root) else {
            continue;
        };
        let emptied = if named_owners.contains(&(crate_root, owner_module.to_vec())) {
            EmptiedOwner::Keep
        } else {
            EmptiedOwner::Delete
        };
        match owner_edit::edit_owner_region(file, region, files, emptied)? {
            RegionEdit::Edited(edits) => {
                let owners = region.iter().map(|owner| owner.index).collect::<Vec<_>>();
                round.fixes.extend(edits.into_iter().map(|fix| OwnedFix {
                    fix,
                    facts: owners.clone(),
                }));
            },
            RegionEdit::Unmatched(indexes) => round.unmatched.extend(indexes),
        }
    }
    if !round.unmatched.is_empty() {
        return Ok(round);
    }

    // Step 3: the ancestor re-export, when a caller now names the item there;
    // under `#[cfg(test)]` when only test builds compile those callers.
    for (index, builds) in outside_used {
        if let AncestorReexport::Insert(insertion) = &facts[index].ancestor_reexport {
            let file = crate_files::canonical(&root.join(&insertion.file));
            round.fixes.push(OwnedFix {
                fix:   ancestor::insertion_fix(&file, insertion, builds)?,
                facts: vec![index],
            });
        }
    }
    Ok(round)
}

/// Callers of `O::N` write `S::N` inside the target scope and `A::N` outside
/// it; callers in the owner module keep its private `use`.
fn redirect(fact: &SubtreeReexportFixFact) -> FacadeRedirect {
    let with_name = |module: &[String]| {
        let mut path = module.to_vec();
        path.push(fact.exported_name.clone());
        path
    };
    FacadeRedirect {
        facade_path:      with_name(&fact.owner_module),
        target:           RedirectTarget::Scoped {
            scope:   fact.target_scope.clone(),
            inside:  with_name(&fact.source_module),
            outside: with_name(&fact.common_ancestor),
        },
        unchanged_module: Some(fact.owner_module.clone()),
    }
}

/// The facts owning a fix that overlaps a different fix in the same file. An
/// insertion overlaps a range only strictly inside it.
fn overlapping_facts(fixes: &[OwnedFix]) -> BTreeSet<usize> {
    let mut overlapping = BTreeSet::new();
    for (position, left) in fixes.iter().enumerate() {
        for right in &fixes[position + 1..] {
            if overlaps(&left.fix, &right.fix) {
                overlapping.extend(left.facts.iter().chain(&right.facts).copied());
            }
        }
    }
    overlapping
}

/// Whether two different fixes in one file edit overlapping bytes.
fn overlaps(fix: &UseFix, other: &UseFix) -> bool {
    let identical =
        fix.start == other.start && fix.end == other.end && fix.replacement == other.replacement;
    let starts_before_other_ends = fix.start < other.end;
    let ends_after_other_starts = other.start < fix.end;
    fix.path == other.path && starts_before_other_ends && ends_after_other_starts && !identical
}
