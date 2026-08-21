use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use crate::fixes::imports::ValidatedFixSet;

/// The text every file carried before `--fix` first wrote to it, keyed by path.
///
/// `MendRunner::apply` runs once per convergence pass (`main` loops it up to
/// `FIX_CONVERGENCE_MAX_PASSES` times), but a rollback answers for the whole
/// invocation: a pass that validated and left its edits on disk is still part
/// of what `RollbackStatus::Restored` promises the user. A per-pass snapshot
/// restores only to the previous pass's output and leaves earlier edits behind.
/// `MendRunner` therefore owns one `SessionSnapshot` across every pass and
/// records into it before each pass writes.
pub(super) struct SessionSnapshot {
    originals: BTreeMap<PathBuf, String>,
}

impl SessionSnapshot {
    pub(super) const fn new() -> Self {
        Self {
            originals: BTreeMap::new(),
        }
    }

    /// Reads every file `fixes` is about to edit, keeping the earliest text
    /// recorded for each path. A path already present holds an earlier pass's
    /// pre-edit content — the state a rollback has to return to — so a later
    /// pass must not replace it with the text it is about to overwrite.
    pub(super) fn record(&mut self, fixes: &ValidatedFixSet) -> Result<()> {
        for fix in fixes.iter() {
            let Entry::Vacant(slot) = self.originals.entry(fix.path.clone()) else {
                continue;
            };
            let text = fs::read_to_string(slot.key())
                .with_context(|| format!("failed to read {}", slot.key().display()))?;
            slot.insert(text);
        }
        Ok(())
    }

    /// Writes every recorded original back. Each file is attempted even after
    /// one fails, so a single unwritable path does not strand the rest; the
    /// first failure is what the caller sees, and it is what turns the reported
    /// rollback into `RollbackStatus::RestoreFailed`.
    pub(super) fn restore(&self) -> Result<()> {
        let mut first_failure = None;
        for (path, original) in &self.originals {
            if let Err(err) = fs::write(path, original)
                .with_context(|| format!("failed to restore {}", path.display()))
            {
                first_failure.get_or_insert(err);
            }
        }
        first_failure.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::SessionSnapshot;
    use crate::fixes::imports::UseFix;
    use crate::fixes::imports::ValidatedFixSet;

    /// The fix set a pass would hand `apply_fixes`: one edit replacing the
    /// whole of `path`.
    fn whole_file_rewrite(path: &Path, replacement: &str) -> Result<ValidatedFixSet> {
        let end = fs::read_to_string(path)?.len();
        ValidatedFixSet::try_from(vec![UseFix {
            path: path.to_path_buf(),
            start: 0,
            end,
            replacement: replacement.to_string(),
            import_group: None,
        }])
    }

    /// The two-pass rollback contract: pass one's edit validated and stayed on
    /// disk, pass two edited the same file again plus a new one and then
    /// failed. Restoring must return both files to their pre-invocation text,
    /// not to pass one's output.
    #[test]
    fn restore_returns_files_to_their_state_before_the_first_pass() -> Result<()> {
        let temp = tempdir()?;
        let alpha = temp.path().join("alpha.rs");
        let beta = temp.path().join("beta.rs");
        fs::write(&alpha, "alpha pristine\n")?;
        fs::write(&beta, "beta pristine\n")?;

        let mut session_snapshot = SessionSnapshot::new();

        session_snapshot.record(&whole_file_rewrite(&alpha, "alpha pass one\n")?)?;
        fs::write(&alpha, "alpha pass one\n")?;

        session_snapshot.record(&whole_file_rewrite(&alpha, "alpha pass two\n")?)?;
        session_snapshot.record(&whole_file_rewrite(&beta, "beta pass two\n")?)?;
        fs::write(&alpha, "alpha pass two\n")?;
        fs::write(&beta, "beta pass two\n")?;

        session_snapshot.restore()?;

        assert_eq!(fs::read_to_string(&alpha)?, "alpha pristine\n");
        assert_eq!(fs::read_to_string(&beta)?, "beta pristine\n");
        Ok(())
    }
}
