use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::schema::StoredFinding;
use super::schema::StoredPubUseFixFact;
use super::schema::UseSite;
use crate::compiler::constants::FINDINGS_DIR_NAME;

#[derive(Default)]
pub struct FindingsSink {
    pub findings:          Vec<StoredFinding>,
    pub pub_use_fix_facts: Vec<StoredPubUseFixFact>,
    pub use_sites:         Vec<UseSite>,
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
