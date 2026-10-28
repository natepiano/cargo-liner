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
                .filter_entry(|entry| {
                    source_entry_scope(entry.path(), selection) == SourceEntryScope::Included
                })
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

/// Build artifacts and Git metadata are excluded from source snapshots and traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceEntryScope {
    Included,
    Excluded,
}

fn source_entry_scope(path: &Path, selection: &Selection) -> SourceEntryScope {
    if path.starts_with(&selection.target_directory)
        || path.file_name().and_then(OsStr::to_str) == Some(".git")
    {
        SourceEntryScope::Excluded
    } else {
        SourceEntryScope::Included
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::CompilerFixTransaction;
    use super::SourceEntryScope;
    use super::source_entry_scope;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;

    #[test]
    fn snapshots_prune_build_and_git_directories_and_restore_only_sources() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path();
        let selection = Selection {
            manifest_path:    root.join("Cargo.toml"),
            manifest_dir:     root.to_path_buf(),
            workspace_root:   root.to_path_buf(),
            target_directory: root.join("target"),
            analysis_root:    root.to_path_buf(),
            scope:            SelectionScope::SinglePackage,
            package_roots:    vec![root.to_path_buf()],
            packages:         Vec::new(),
        };
        for relative in ["src", "target/generated", ".git", "target-like"] {
            fs::create_dir_all(root.join(relative))?;
        }
        for relative in [
            "src/lib.rs",
            "target/generated/lib.rs",
            ".git/hook.rs",
            "target-like/a.rs",
        ] {
            fs::write(root.join(relative), "original")?;
        }
        for relative in ["target", "target/generated", ".git"] {
            assert_eq!(
                source_entry_scope(&root.join(relative), &selection),
                SourceEntryScope::Excluded
            );
        }
        let transaction = CompilerFixTransaction::capture(&selection)?;
        assert_eq!(transaction.snapshots.len(), 2);
        for relative in [
            "src/lib.rs",
            "target/generated/lib.rs",
            ".git/hook.rs",
            "target-like/a.rs",
        ] {
            fs::write(root.join(relative), "changed")?;
        }
        transaction.restore()?;
        for relative in ["src/lib.rs", "target-like/a.rs"] {
            assert_eq!(fs::read_to_string(root.join(relative))?, "original");
        }
        for relative in ["target/generated/lib.rs", ".git/hook.rs"] {
            assert_eq!(fs::read_to_string(root.join(relative))?, "changed");
        }
        Ok(())
    }
}
