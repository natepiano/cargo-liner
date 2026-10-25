use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::parent_boundary::ParentBoundaryKey;
use crate::fixes::facade_redirect;
use crate::fixes::facade_redirect::FacadeRedirect;
use crate::fixes::facade_redirect::ModuleAliases;
use crate::fixes::facade_redirect::RedirectTarget;
use crate::fixes::imports::UseFix;
use crate::rust_syntax::ParsedSources;

pub(super) struct ValidatedPubUsePlan {
    pub(super) parent_boundary:    ParentBoundaryKey,
    pub(super) child_file:         PathBuf,
    pub(super) child_module:       String,
    pub(super) exported_name:      String,
    pub(super) parent_module_path: Vec<String>,
    pub(super) target_item_path:   Vec<String>,
    pub(super) child_narrowing:    UseFix,
}

pub(super) fn rewrite_subtree_imports_for_plans(
    plans: &[ValidatedPubUsePlan],
    sources: &ParsedSources,
) -> Result<Vec<UseFix>> {
    let mut plan_groups: BTreeMap<PathBuf, Vec<&ValidatedPubUsePlan>> = BTreeMap::new();
    for plan in plans {
        plan_groups
            .entry(plan.parent_boundary.parent_module.clone())
            .or_default()
            .push(plan);
    }

    let mut fixes = Vec::new();
    for (parent_module, parent_plans) in plan_groups {
        let parent_dir = parent_module
            .parent()
            .context("candidate parent boundary had no parent directory")?;
        let source_root = facade_redirect::find_source_root(&parent_module).with_context(|| {
            format!(
                "failed to determine src root for parent boundary {}",
                parent_module.display()
            )
        })?;
        let module_map = sources.module_map(&source_root);
        let redirects = parent_plans
            .iter()
            .copied()
            .map(facade_redirect)
            .collect::<Vec<_>>();
        for file in rust_source_files(parent_dir)? {
            if file == parent_module {
                continue;
            }
            if parent_plans.iter().any(|plan| plan.child_file == file) {
                continue;
            }
            let Some(module_path) = module_map.scannable_module_path(&source_root, &file)? else {
                continue;
            };
            fixes.extend(
                facade_redirect::redirect_callers_in_file(
                    &file,
                    sources,
                    module_path,
                    &redirects,
                    &ModuleAliases::default(),
                )?
                .fixes,
            );
        }
    }

    facade_redirect::dedup_fixes(&mut fixes);
    Ok(fixes)
}

/// Callers inside the parent's subtree stop reaching the item through the
/// parent's `pub use` and name the declaring child module instead.
fn facade_redirect(plan: &ValidatedPubUsePlan) -> FacadeRedirect {
    let mut facade_path = plan.parent_module_path.clone();
    facade_path.push(plan.exported_name.clone());
    FacadeRedirect {
        facade_path,
        target: RedirectTarget::Everywhere(plan.target_item_path.clone()),
        unchanged_module: None,
    }
}

fn rust_source_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(dir, &mut files)?;
    Ok(files)
}

enum SourceEntryKind {
    Directory,
    RustFile,
    Other,
}

impl From<&Path> for SourceEntryKind {
    fn from(path: &Path) -> Self {
        if path.is_dir() {
            Self::Directory
        } else if path.extension().and_then(OsStr::to_str) == Some("rs") {
            Self::RustFile
        } else {
            Self::Other
        }
    }
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        match SourceEntryKind::from(path.as_path()) {
            SourceEntryKind::Directory => collect_rust_source_files(&path, files)?,
            SourceEntryKind::RustFile => files.push(path),
            SourceEntryKind::Other => {},
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_collection_recurses_through_directories_with_rust_extensions() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let nested = dir.path().join("nested.rs");
        std::fs::create_dir(&nested)?;
        std::fs::write(nested.join("child.rs"), "")?;
        std::fs::write(dir.path().join("root.rs"), "")?;
        std::fs::write(dir.path().join("notes.txt"), "")?;
        let mut files = super::rust_source_files(dir.path())?;
        files.sort();
        let mut expected = vec![nested.join("child.rs"), dir.path().join("root.rs")];
        expected.sort();
        assert_eq!(files, expected);
        Ok(())
    }
}
