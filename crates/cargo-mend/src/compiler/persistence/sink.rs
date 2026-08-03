use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::schema::StoredFinding;
use super::schema::StoredPubUseFixFact;
use super::schema::UseSite;
use super::visibility_constraint::StoredVisibilityConstraint;
use crate::compiler::constants::FINDINGS_DIR_NAME;

#[derive(Default)]
pub struct FindingsSink {
    pub findings:               Vec<StoredFinding>,
    pub visibility_constraints: Vec<StoredVisibilityConstraint>,
    pub pub_use_fix_facts:      Vec<StoredPubUseFixFact>,
    pub use_sites:              UseSiteIndex,
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
pub struct UseSiteIndex(BTreeMap<String, BTreeSet<String>>);

impl UseSiteIndex {
    pub fn insert(&mut self, target_def_path: String, caller_module_def_path: String) {
        self.0
            .entry(target_def_path)
            .or_default()
            .insert(caller_module_def_path);
    }

    /// The modules referencing `target_def_path` in this compilation.
    pub fn callers(&self, target_def_path: &str) -> Option<&BTreeSet<String>> {
        self.0.get(target_def_path)
    }

    /// Flatten to the persisted form: one record per distinct pair, in a
    /// stable order so the findings file is byte-identical across runs that
    /// analyzed the same crate.
    pub fn into_use_sites(self) -> Vec<UseSite> {
        self.0
            .into_iter()
            .flat_map(|(target_def_path, callers)| {
                callers
                    .into_iter()
                    .map(move |caller_module_def_path| UseSite {
                        target_def_path: target_def_path.clone(),
                        caller_module_def_path,
                    })
            })
            .collect()
    }
}

pub fn prepare_findings_dir(target_directory: &Path) -> Result<PathBuf> {
    let findings_dir = target_directory.join(FINDINGS_DIR_NAME);
    fs::create_dir_all(&findings_dir).with_context(|| {
        format!(
            "failed to create findings directory {}",
            findings_dir.display()
        )
    })?;
    Ok(findings_dir)
}
