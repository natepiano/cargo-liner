use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::schema::DeclarationIdentity;
use super::schema::StoredFinding;
use super::schema::StoredModuleMountFact;
use super::schema::StoredPubUseFixFact;
use super::schema::StoredSubtreeReexportFixFact;
use super::schema::UseSite;
use super::schema::UseSiteReference;
use super::visibility_constraint::StoredVisibilityConstraint;

#[derive(Default)]
pub(in crate::compiler) struct FindingsSink {
    pub findings:                   Vec<StoredFinding>,
    pub visibility_constraints:     Vec<StoredVisibilityConstraint>,
    pub pub_use_fix_facts:          Vec<StoredPubUseFixFact>,
    pub use_sites:                  UseSiteIndex,
    pub subtree_reexport_fix_facts: Vec<StoredSubtreeReexportFixFact>,
    pub module_mount_facts:         Vec<StoredModuleMountFact>,
}

struct DeclarationCallers {
    declaration: DeclarationIdentity,
    callers:     BTreeMap<String, UseSiteReference>,
}

/// The set of modules that reference each item, keyed by the referenced
/// item's def-path.
///
/// Both consumers want exactly that set — the current pass asks which
/// modules call one item, and cross-target reconciliation unions the sets
/// across compilations. Keeping one record per *occurrence* instead stores
/// the same pair once per syntactic reference, so an item named ten
/// thousand times costs ten thousand identical records that every consumer
/// immediately collapses back to one.
#[derive(Default)]
pub(in crate::compiler) struct UseSiteIndex(BTreeMap<String, DeclarationCallers>);

impl UseSiteIndex {
    /// A semantic path use wins over signature-only reach, which wins over an
    /// import declaration retained to keep visibility rewrites safe.
    pub(in crate::compiler) fn insert(
        &mut self,
        declaration: DeclarationIdentity,
        caller_module_def_path: String,
        use_site_reference: UseSiteReference,
    ) {
        let recorded = self
            .0
            .entry(declaration.def_path.clone())
            .or_insert_with(|| DeclarationCallers {
                declaration,
                callers: BTreeMap::new(),
            })
            .callers
            .entry(caller_module_def_path)
            .or_insert(use_site_reference);
        if matches!(
            (*recorded, use_site_reference),
            (_, UseSiteReference::Named)
                | (
                    UseSiteReference::PrivateImport | UseSiteReference::RestrictedImport,
                    UseSiteReference::ThroughSignature
                )
                | (UseSiteReference::DeclarationInterface, _)
        ) {
            *recorded = use_site_reference;
        }
    }

    /// The modules referencing `target_def_path` in this compilation, however
    /// they reach it.
    pub(in crate::compiler) fn callers(&self, target_def_path: &str) -> BTreeSet<String> {
        self.0
            .get(target_def_path)
            .map(|entry| entry.callers.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Flatten to the persisted form: one record per distinct pair, in a
    /// stable order so the findings file is byte-identical across runs that
    /// analyzed the same crate.
    pub(in crate::compiler) fn into_use_sites(self) -> Vec<UseSite> {
        self.0
            .into_values()
            .flat_map(
                |DeclarationCallers {
                     declaration,
                     callers,
                 }| {
                    callers
                        .into_iter()
                        .map(
                            move |(caller_module_def_path, use_site_reference)| UseSite {
                                declaration: declaration.clone(),
                                caller_module_def_path,
                                reference: use_site_reference,
                            },
                        )
                },
            )
            .collect()
    }
}
