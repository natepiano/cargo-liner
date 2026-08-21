use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use walkdir::WalkDir;

use crate::selection::Selection;

pub(super) struct CompilerFixTransaction {
    snapshots: Vec<(PathBuf, String)>,
}

impl CompilerFixTransaction {
    pub(super) fn capture(selection: &Selection) -> Result<Self> {
        let mut source_files = BTreeSet::new();
        for package_root in &selection.package_roots {
            for entry in WalkDir::new(package_root)
                .into_iter()
                .filter_entry(|entry| source_entry_is_in_scope(entry.path(), selection))
            {
                let entry = entry.with_context(|| {
                    format!(
                        "failed to inspect Rust sources under {}",
                        package_root.display()
                    )
                })?;
                let path = entry.path();
                if entry.file_type().is_file()
                    && path.extension().and_then(OsStr::to_str) == Some("rs")
                {
                    source_files.insert(path.to_path_buf());
                }
            }
        }

        let snapshots = source_files
            .into_iter()
            .map(|path| {
                let text = fs::read_to_string(&path)
                    .with_context(|| format!("failed to snapshot {}", path.display()))?;
                Ok((path, text))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { snapshots })
    }

    pub(super) fn restore(&self) -> Result<()> {
        for (path, original) in &self.snapshots {
            if fs::read_to_string(path).is_ok_and(|current| current == *original) {
                continue;
            }
            fs::write(path, original)
                .with_context(|| format!("failed to restore {}", path.display()))?;
        }
        Ok(())
    }
}

fn source_entry_is_in_scope(path: &Path, selection: &Selection) -> bool {
    !path.starts_with(&selection.target_directory)
        && path.file_name().and_then(OsStr::to_str) != Some(".git")
}
