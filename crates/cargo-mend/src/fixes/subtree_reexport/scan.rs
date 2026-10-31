use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::ancestor;
use super::crate_files;
use super::crate_files::CrateFiles;
use super::crate_files::OwnerCrate;
use super::owner_edit;
use super::owner_edit::EmptiedOwner;
use super::owner_edit::OwnerFact;
use super::owner_edit::RegionEdit;
use crate::compiler::SourceModuleMount;
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
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;
use crate::selection;
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

/// Facts stay with the crate files resolved for them. Owner editing cannot
/// silently skip a region because a separately supplied crate map lacks it.
struct ResolvedCrateFacts<'a> {
    files: CrateFiles,
    facts: Vec<OwnerFact<'a>>,
}

/// A region may combine facts from several crates sharing one canonical file and module.
/// Its lowest global fact index selects the crate root and files used for retention;
/// facts remain in global index order so crate-name ordering cannot change that choice.
struct ResolvedOwnerFacts<'a, 'f> {
    crate_root: &'a str,
    files:      &'a CrateFiles,
    facts:      Vec<OwnerFact<'f>>,
}

pub(in crate::fixes) fn scan_selection(
    selection: &Selection,
    report: &Report,
    sources: &ParsedSources,
) -> Result<SubtreeReexportScan> {
    let root = &selection.analysis_root;
    let (facts, mut skipped) = fixable_facts(root, report);
    if facts.is_empty() {
        return Ok(SubtreeReexportScan {
            fixes: ValidatedFixSet::try_from(Vec::<UseFix>::new())?,
            skipped,
        });
    }
    let crates = resolved_crate_facts(root, report, &facts, sources);
    let mounts = ReexportMountInventory::collect(selection, report, sources)?;
    let mut excluded = crates
        .values()
        .flat_map(|resolved| {
            resolved.facts.iter().filter_map(|owner| {
                resolved
                    .files
                    .has_several_parents(&owner_file(root, owner.fact))
                    .then_some(owner.index)
            })
        })
        .collect::<BTreeSet<_>>();
    excluded.extend(owners_behind_other_reexports(&facts));

    // `scan_selection` adds facts that `build_round` could not place or whose edits
    // overlap another fix to `excluded`, then rebuilds every edit without them.
    let fixes = loop {
        let round = build_mount_rounds(root, &crates, &excluded, sources)?;
        let mut rejected = round.unmatched.clone();
        rejected.extend(overlapping_facts(&round.fixes));
        rejected.extend(mounts.unagreed_facts(root, &facts, &round.fixes));
        if rejected.is_empty() {
            break round.fixes;
        }
        excluded.extend(rejected);
    };

    let sites = |indexes: &BTreeSet<usize>| {
        indexes
            .iter()
            .map(|&index| ReexportSite::of(root, facts[index]))
            .collect::<BTreeSet<_>>()
            .len()
    };
    let applied = facts
        .iter()
        .map(|fact| ReexportSite::of(root, fact))
        .collect::<BTreeSet<_>>()
        .len()
        - sites(&excluded);
    skipped += sites(&excluded);
    let mut fixes = fixes.into_iter().map(|owned| owned.fix).collect::<Vec<_>>();
    facade_redirect::dedup_fixes(&mut fixes);
    match ValidatedFixSet::try_from(fixes) {
        Ok(fixes) => Ok(SubtreeReexportScan { fixes, skipped }),
        // Two overlapping caller rewrites belong to no single fact. If they make
        // `ValidatedFixSet::try_from` fail, the whole pass stands down and every fact
        // counts as skipped.
        Err(_) => Ok(SubtreeReexportScan {
            fixes:   ValidatedFixSet::try_from(Vec::<UseFix>::new())?,
            skipped: skipped + applied,
        }),
    }
}

/// A physical re-export is reported once even when its mounts name different modules.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct ReexportSite {
    file:   PathBuf,
    line:   usize,
    column: usize,
}

impl ReexportSite {
    fn of(root: &Path, fact: &SubtreeReexportFixFact) -> Self {
        Self {
            file:   owner_file(root, fact),
            line:   fact.use_line,
            column: fact.use_column,
        }
    }
}

/// Keep each mount's proposal for comparison; count a site with no proposal once.
fn fixable_facts<'a>(root: &Path, report: &'a Report) -> (Vec<&'a SubtreeReexportFixFact>, usize) {
    let mut by_site: BTreeMap<ReexportSite, Vec<&SubtreeReexportFixFact>> = BTreeMap::new();
    for fact in report.facts.subtree_reexport_fix_facts.iter() {
        by_site
            .entry(ReexportSite::of(root, fact))
            .or_default()
            .push(fact);
    }
    let sites = report
        .findings
        .iter()
        .filter(|finding| {
            finding.diagnostic_code == DiagnosticCode::PubUseOutsideSubtree
                && finding.fix_support() == FixSupport::PubUseOutsideSubtree
        })
        .map(|finding| ReexportSite {
            file:   crate_files::canonical(&root.join(&finding.path)),
            line:   finding.line,
            column: finding.column,
        })
        .collect::<BTreeSet<_>>();
    let mut facts = Vec::new();
    let mut skipped = 0;
    for site in sites {
        match by_site.remove(&site) {
            Some(proposals) => facts.extend(proposals),
            None => skipped += 1,
        }
    }
    (facts, skipped)
}

/// Unobserved packages veto only sources they could mount. An unresolved
/// include keeps the package's reach unknown and therefore cannot authorize edits.
enum UnobservedPackageReach {
    Known(BTreeSet<PathBuf>),
    Unresolved,
}

impl UnobservedPackageReach {
    fn may_mount(&self, source: &Path) -> bool {
        match self {
            Self::Known(files) => files.contains(source),
            Self::Unresolved => true,
        }
    }
}

/// Complete compiler evidence, including sources in packages without a finding.
/// Crate roots identify the evidence; `SourceModuleMount` identifies the module,
/// so a library and its test build do not become two independent mounts.
struct ReexportMountInventory {
    sources:    BTreeMap<PathBuf, BTreeMap<PathBuf, BTreeSet<SourceModuleMount>>>,
    unobserved: Vec<UnobservedPackageReach>,
}

impl ReexportMountInventory {
    fn collect(selection: &Selection, report: &Report, parsed: &ParsedSources) -> Result<Self> {
        let root = &selection.analysis_root;
        let workspace =
            selection::resolve_cargo_selection(Some(&selection.workspace_root.join("Cargo.toml")))?;
        let packages: BTreeMap<_, _> = workspace
            .packages
            .iter()
            .flat_map(|package| {
                package.targets.iter().map(|target| {
                    (
                        crate_files::canonical(&target.src_path),
                        crate_files::canonical(&package.root)
                            .to_string_lossy()
                            .into_owned(),
                    )
                })
            })
            .collect();
        let mut sources: BTreeMap<PathBuf, BTreeMap<PathBuf, BTreeSet<SourceModuleMount>>> =
            BTreeMap::new();
        for mount in report.facts.module_mount_facts.iter() {
            let crate_root = crate_files::canonical(&root.join(&mount.crate_root_file));
            let package = packages
                .get(&crate_root)
                .context("module mount belongs to no selected Cargo target")?;
            sources
                .entry(crate_files::canonical(&root.join(&mount.file)))
                .or_default()
                .entry(crate_root)
                .or_default()
                .insert(SourceModuleMount::new(
                    package.clone(),
                    mount.module_path.clone(),
                ));
        }
        let observed: BTreeSet<_> = report
            .facts
            .module_mount_facts
            .iter()
            .filter_map(|mount| {
                packages.get(&crate_files::canonical(&root.join(&mount.crate_root_file)))
            })
            .collect();
        let unobserved = workspace
            .packages
            .iter()
            .filter_map(|package| {
                let root = crate_files::canonical(&package.root);
                if observed.contains(&root.to_string_lossy().into_owned()) {
                    return None;
                }
                Some(
                    ModuleMap::reachable_sources(
                        package.targets.iter().map(|target| target.src_path.clone()),
                        parsed,
                    )
                    .map_or(
                        UnobservedPackageReach::Unresolved,
                        UnobservedPackageReach::Known,
                    ),
                )
            })
            .collect();
        Ok(Self {
            sources,
            unobserved,
        })
    }

    /// Compare the physical edits produced independently by each mount. A silent
    /// mount, a differing replacement, or an unobserved package that can reach
    /// this source vetoes them.
    fn unagreed_facts(
        &self,
        root: &Path,
        facts: &[&SubtreeReexportFixFact],
        fixes: &[OwnedFix],
    ) -> BTreeSet<usize> {
        let mut rejected = BTreeSet::new();
        for (source, crates) in &self.sources {
            let expected = crates.values().flatten().collect::<BTreeSet<_>>();
            let mut proposals: BTreeMap<&SourceModuleMount, BTreeSet<(usize, usize, &str)>> =
                BTreeMap::new();
            let mut owners = BTreeSet::new();
            for owned in fixes.iter().filter(|owned| &owned.fix.path == source) {
                for &index in &owned.facts {
                    let fact = facts[index];
                    owners.insert(index);
                    let crate_root = crate_files::canonical(&root.join(&fact.crate_root_file));
                    if let Some(mount) = crates.get(&crate_root).and_then(|mounts| {
                        mounts
                            .iter()
                            .filter(|mount| fact.owner_module.starts_with(mount.module_path()))
                            .max_by_key(|mount| mount.module_path().len())
                    }) {
                        proposals.entry(mount).or_default().insert((
                            owned.fix.start,
                            owned.fix.end,
                            &owned.fix.replacement,
                        ));
                    }
                }
            }
            let mut edits = proposals.values();
            let unanimous = edits
                .next()
                .is_some_and(|first| edits.all(|next| next == first));
            if self
                .unobserved
                .iter()
                .any(|package| package.may_mount(source))
                || (expected.len() > 1
                    && (proposals.keys().copied().collect::<BTreeSet<_>>() != expected
                        || !unanimous))
            {
                rejected.extend(owners);
            }
        }
        rejected
    }
}

/// Resolve each crate once and retain the facts that required that resolution.
fn resolved_crate_facts<'a>(
    root: &Path,
    report: &Report,
    facts: &[&'a SubtreeReexportFixFact],
    sources: &ParsedSources,
) -> BTreeMap<String, ResolvedCrateFacts<'a>> {
    let mut grouped: BTreeMap<String, Vec<OwnerFact<'a>>> = BTreeMap::new();
    for (index, &fact) in facts.iter().enumerate() {
        grouped
            .entry(fact.crate_root_file.clone())
            .or_default()
            .push(OwnerFact { index, fact });
    }
    grouped
        .into_iter()
        .map(|(crate_root, facts)| {
            let mounts = report
                .facts
                .module_mount_facts
                .iter()
                .filter(|mount| mount.crate_root_file == crate_root)
                .map(|mount| (root.join(&mount.file), mount.module_path.clone()));
            let files = CrateFiles::resolve(&root.join(&crate_root), mounts, sources);
            (crate_root, ResolvedCrateFacts { files, facts })
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

fn build_mount_rounds(
    root: &Path,
    crates: &BTreeMap<String, ResolvedCrateFacts<'_>>,
    excluded: &BTreeSet<usize>,
    sources: &ParsedSources,
) -> Result<Round> {
    let mut round = Round {
        fixes:     Vec::new(),
        unmatched: BTreeSet::new(),
    };
    for (crate_root, resolved) in crates {
        let scoped = BTreeMap::from([(crate_root.as_str(), resolved)]);
        let proposed = build_resolved_round(root, &scoped, excluded, sources)?;
        round.fixes.extend(proposed.fixes);
        round.unmatched.extend(proposed.unmatched);
    }
    Ok(round)
}

#[cfg(test)]
fn build_round(
    root: &Path,
    crates: &BTreeMap<String, ResolvedCrateFacts<'_>>,
    excluded: &BTreeSet<usize>,
    sources: &ParsedSources,
) -> Result<Round> {
    let crates = crates
        .iter()
        .map(|(name, resolved)| (name.as_str(), resolved))
        .collect();
    build_resolved_round(root, &crates, excluded, sources)
}

fn build_resolved_round(
    root: &Path,
    crates: &BTreeMap<&str, &ResolvedCrateFacts<'_>>,
    excluded: &BTreeSet<usize>,
    sources: &ParsedSources,
) -> Result<Round> {
    let mut round = Round {
        fixes:     Vec::new(),
        unmatched: BTreeSet::new(),
    };

    let mut named_owners = BTreeSet::new();

    // `facade_redirect::redirect_callers_in_file` runs first, per crate root, with that
    // root's `redirects`. It reports owner modules a caller's `use` still names,
    // which `owner_edit::edit_owner_region` must not delete, and the builds that
    // compile the callers each ancestor re-export serves.
    let mut outside_used: BTreeMap<usize, (&OwnerFact<'_>, CallerBuilds)> = BTreeMap::new();
    for (crate_root, resolved) in crates {
        let files = &resolved.files;
        let active = resolved
            .facts
            .iter()
            .filter(|owner| !excluded.contains(&owner.index))
            .collect::<Vec<_>>();
        if active.is_empty() {
            continue;
        }
        let redirects = active
            .iter()
            .map(|owner| redirect(owner.fact))
            .collect::<Vec<_>>();
        let module_aliases = ModuleAliases::collect(files.scannable(), sources)?;
        for (file, module_path) in files.scannable() {
            let result = facade_redirect::redirect_callers_in_file(
                file,
                sources,
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
                let owner = active[slot];
                let (_, widest) = outside_used.entry(owner.index).or_insert((owner, builds));
                *widest = (*widest).max(builds);
            }
            named_owners.extend(
                result
                    .named_owner_modules
                    .into_iter()
                    .map(|module| ((*crate_root).to_string(), module)),
            );
            round
                .fixes
                .extend(result.fixes.into_iter().map(|fix| OwnedFix {
                    fix,
                    facts: Vec::new(),
                }));
        }
    }

    // `owner_edit::edit_owner_region` builds owner edits, one region per owner file and owner
    // module.
    let regions = resolved_regions(root, crates, excluded);
    for ((file, owner_module), region) in &regions {
        let crate_root = region.crate_root;
        let emptied = if named_owners.contains(&(crate_root.to_string(), owner_module.to_vec())) {
            EmptiedOwner::Keep
        } else {
            EmptiedOwner::Delete
        };
        let owner_crate = OwnerCrate {
            files: region.files,
            sources,
        };
        match owner_edit::edit_owner_region(file, &region.facts, owner_crate, emptied)? {
            RegionEdit::Edited(edits) => {
                let owners = region
                    .facts
                    .iter()
                    .map(|owner| owner.index)
                    .collect::<Vec<_>>();
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

    // `ancestor::insertion_fix` adds the ancestor re-export, when a caller now names the item
    // there; under `#[cfg(test)]` when only test builds compile those callers.
    for (index, (owner, builds)) in outside_used {
        if let AncestorReexport::Insert(insertion) = &owner.fact.ancestor_reexport {
            let file = crate_files::canonical(&root.join(&insertion.file));
            round.fixes.push(OwnedFix {
                fix:   ancestor::insertion_fix(&file, insertion, builds, sources)?,
                facts: vec![index],
            });
        }
    }
    Ok(round)
}

/// Group globally ordered facts with their resolved crate; the first selects region metadata.
#[cfg(test)]
fn resolved_owner_regions<'a, 'f>(
    root: &Path,
    crates: &'a BTreeMap<String, ResolvedCrateFacts<'f>>,
    excluded: &BTreeSet<usize>,
) -> BTreeMap<(PathBuf, &'f [String]), ResolvedOwnerFacts<'a, 'f>> {
    let crates = crates
        .iter()
        .map(|(name, resolved)| (name.as_str(), resolved))
        .collect();
    resolved_regions(root, &crates, excluded)
}

fn resolved_regions<'a, 'f>(
    root: &Path,
    crates: &BTreeMap<&'a str, &'a ResolvedCrateFacts<'f>>,
    excluded: &BTreeSet<usize>,
) -> BTreeMap<(PathBuf, &'f [String]), ResolvedOwnerFacts<'a, 'f>> {
    let active = crates
        .iter()
        .flat_map(|(crate_root, resolved)| {
            resolved.facts.iter().filter_map(move |owner| {
                (!excluded.contains(&owner.index))
                    .then_some((owner.index, (owner, *crate_root, &resolved.files)))
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut regions: BTreeMap<(PathBuf, &[String]), ResolvedOwnerFacts<'_, '_>> = BTreeMap::new();
    for (owner, crate_root, files) in active.into_values() {
        regions
            .entry((
                owner_file(root, owner.fact),
                owner.fact.owner_module.as_slice(),
            ))
            .or_insert_with(|| ResolvedOwnerFacts {
                crate_root,
                files,
                facts: Vec::new(),
            })
            .facts
            .push(OwnerFact {
                index: owner.index,
                fact:  owner.fact,
            });
    }
    regions
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::reporting::AncestorReexport;
    use crate::reporting::ModuleForm;
    use crate::reporting::Report;
    use crate::reporting::SubtreeReexportFixFact;
    use crate::rust_syntax::ParsedSources;

    fn fact(crate_root: &str, owner: &str) -> SubtreeReexportFixFact {
        SubtreeReexportFixFact {
            use_path:          format!("{owner}/owner.rs"),
            use_line:          1,
            use_column:        1,
            exported_name:     "Widget".to_string(),
            written_path:      "crate::source::Widget".to_string(),
            owner_module:      vec!["owner".to_string()],
            owner_module_form: ModuleForm::File,
            source_module:     vec!["source".to_string()],
            target_scope:      vec!["source".to_string()],
            common_ancestor:   Vec::new(),
            crate_root_file:   crate_root.to_string(),
            ancestor_reexport: AncestorReexport::NotRequired,
        }
    }

    #[test]
    fn resolved_crates_keep_interleaved_facts_and_edit_every_owner() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        for name in ["a", "b"] {
            std::fs::create_dir(dir.path().join(name))?;
            std::fs::write(
                dir.path().join(format!("{name}/lib.rs")),
                "mod owner; mod source { pub struct Widget; }\n",
            )?;
            std::fs::write(
                dir.path().join(format!("{name}/owner.rs")),
                "pub use crate::source::Widget;\n",
            )?;
        }
        let a = fact("a/lib.rs", "a");
        let b = fact("b/lib.rs", "b");
        let facts = [&b, &a];
        let sources = ParsedSources::default();
        let crates = super::resolved_crate_facts(dir.path(), &Report::default(), &facts, &sources);
        assert_eq!(crates.len(), 2);
        for (root, resolved) in &crates {
            assert_eq!(resolved.facts.len(), 1);
            let owner = &resolved.facts[0];
            assert_eq!(&owner.fact.crate_root_file, root);
            assert!(std::ptr::eq(owner.fact, facts[owner.index]));
            let file = super::owner_file(dir.path(), owner.fact);
            assert!(resolved.files.scannable().any(|(path, _)| path == file));
        }
        let round = super::build_round(dir.path(), &crates, &BTreeSet::new(), &sources)?;
        assert!(round.unmatched.is_empty());
        assert_eq!(round.fixes.len(), 2);
        assert_eq!(round.fixes[0].facts, [1]);
        assert_eq!(round.fixes[1].facts, [0]);
        for owned in round.fixes {
            assert_eq!(owned.fix.start, 0);
            assert_eq!(owned.fix.end, "pub use crate::source::Widget;\n".len());
            assert_eq!(owned.fix.replacement, "");
        }
        let excluded = BTreeSet::from([0]);
        let round = super::build_round(dir.path(), &crates, &excluded, &sources)?;
        assert_eq!(round.fixes.len(), 1);
        assert_eq!(round.fixes[0].facts, [1]);
        Ok(())
    }

    #[test]
    fn shared_owner_uses_the_globally_first_crate_to_keep_its_empty_module() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let source = concat!(
            "pub mod owner {\n",
            "    #[cfg(feature = \"b\")]\n",
            "    pub use crate::source::Widget;\n",
            "    #[cfg(feature = \"a\")]\n",
            "    pub use crate::source::Gadget;\n",
            "}\n",
        );
        std::fs::write(dir.path().join("shared.rs"), source)?;
        for name in ["a", "b"] {
            std::fs::create_dir(dir.path().join(name))?;
            let named_owner = if name == "b" {
                "pub use shared::owner;\n"
            } else {
                ""
            };
            std::fs::write(
                dir.path().join(format!("{name}/lib.rs")),
                format!(
                    concat!(
                        "#[path = \"../shared.rs\"] pub mod shared;\n",
                        "mod source {{ pub struct Widget; pub struct Gadget; }}\n{}",
                    ),
                    named_owner,
                ),
            )?;
        }
        let mut b = fact("b/lib.rs", "b");
        b.use_path = "shared.rs".to_string();
        b.use_line = 3;
        b.use_column = 5;
        b.owner_module = vec!["shared".to_string(), "owner".to_string()];
        b.owner_module_form = ModuleForm::Inline;
        let mut a = b.clone();
        a.crate_root_file = "a/lib.rs".to_string();
        a.use_line = 5;
        a.exported_name = "Gadget".to_string();
        a.written_path = "crate::source::Gadget".to_string();
        let sources = ParsedSources::default();
        let crates =
            super::resolved_crate_facts(dir.path(), &Report::default(), &[&b, &a], &sources);
        let excluded = BTreeSet::new();
        let regions = super::resolved_owner_regions(dir.path(), &crates, &excluded);
        assert_eq!(regions.len(), 1);
        for region in regions.values() {
            assert_eq!(region.crate_root, "b/lib.rs");
            assert!(std::ptr::eq(
                region.files,
                &raw const crates["b/lib.rs"].files
            ));
            assert_eq!(
                region
                    .facts
                    .iter()
                    .map(|owner| owner.index)
                    .collect::<Vec<_>>(),
                [0, 1]
            );
        }
        let round = super::build_round(dir.path(), &crates, &excluded, &sources)?;
        assert!(round.unmatched.is_empty());
        let file = super::crate_files::canonical(&dir.path().join("shared.rs"));
        let first_start = "pub mod owner {\n".len();
        let second_start = first_start
            + concat!(
                "    #[cfg(feature = \"b\")]\n",
                "    pub use crate::source::Widget;\n",
            )
            .len();
        let expected = vec![
            (file.clone(), first_start, second_start, "", vec![0, 1]),
            (file, second_start, source.len() - 2, "", vec![0, 1]),
        ];
        let edits = round
            .fixes
            .iter()
            .map(|owned| {
                (
                    owned.fix.path.clone(),
                    owned.fix.start,
                    owned.fix.end,
                    owned.fix.replacement.as_str(),
                    owned.facts.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(edits, expected);
        let mut rewritten = source.to_string();
        for owned in round.fixes.iter().rev() {
            rewritten.replace_range(owned.fix.start..owned.fix.end, &owned.fix.replacement);
        }
        assert_eq!(rewritten, "pub mod owner {\n}\n");
        Ok(())
    }
}
