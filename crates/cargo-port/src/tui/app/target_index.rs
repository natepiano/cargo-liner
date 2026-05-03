//! `TargetDirIndex` — forward + reverse map from `target_directory`
//! to the projects that live under it. Built incrementally in the TUI
//! event loop as `BackgroundMsg::CargoMetadata` messages land. Used
//! by the confirm dialog to list "also affects" siblings sharing a
//! target dir.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::project::AbsolutePath;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetDirMember {
    pub project_root: AbsolutePath,
}

/// Forward map: `target_directory` → projects that resolve to it.
/// Reverse map: `project_root` → its current `target_directory`.
/// Both maps stay in sync via [`TargetDirIndex::upsert`] /
/// [`TargetDirIndex::remove`].
#[derive(Debug, Default)]
pub struct TargetDirIndex {
    by_target_dir: HashMap<AbsolutePath, Vec<TargetDirMember>>,
    by_project:    HashMap<AbsolutePath, AbsolutePath>,
}

impl TargetDirIndex {
    pub fn new() -> Self { Self::default() }

    /// Set/replace the `target_dir` for `member.project_root`. If the
    /// project previously lived under a different target dir, the
    /// stale entry is evicted before inserting the new one. Safe to
    /// call repeatedly with the same inputs.
    pub fn upsert(&mut self, member: TargetDirMember, target_dir: AbsolutePath) {
        let project_root = member.project_root.clone();
        if let Some(previous_dir) = self.by_project.get(&project_root).cloned() {
            if previous_dir == target_dir {
                if let Some(bucket) = self.by_target_dir.get_mut(&target_dir) {
                    if bucket.iter().any(|m| m.project_root == project_root) {
                        return;
                    }
                    bucket.push(member);
                    return;
                }
            } else {
                self.evict_from_bucket(&previous_dir, &project_root);
            }
        }
        self.by_project.insert(project_root, target_dir.clone());
        self.by_target_dir
            .entry(target_dir)
            .or_default()
            .push(member);
    }

    /// Every member that resolves to `target_dir`, minus any project
    /// whose root is in `exclude`. Callers that drive the confirm
    /// dialog pass their selection-set as `exclude` so self-members
    /// don't get listed as "collateral".
    pub fn siblings<'a>(
        &'a self,
        target_dir: &AbsolutePath,
        exclude: &[AbsolutePath],
    ) -> Vec<&'a TargetDirMember> {
        let excluded: HashSet<&AbsolutePath> = exclude.iter().collect();
        self.by_target_dir
            .get(target_dir)
            .map(|members| {
                members
                    .iter()
                    .filter(|m| !excluded.contains(&m.project_root))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn evict_from_bucket(&mut self, target_dir: &AbsolutePath, project_root: &AbsolutePath) {
        if let Some(bucket) = self.by_target_dir.get_mut(target_dir) {
            bucket.retain(|m| m.project_root != *project_root);
            if bucket.is_empty() {
                self.by_target_dir.remove(target_dir);
            }
        }
    }
}

// ── CleanSelection ───────────────────────────────────────────────────

/// Which projects the user is asking to clean.
///
/// A `Project` selection is the single-row case (a `VisibleRow::Root`
/// on a Rust project). `WorktreeGroup` is the worktree-group header
/// case — `primary` is the canonical checkout, `linked` lists every
/// linked worktree at selection time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanSelection {
    Project {
        root: AbsolutePath,
    },
    WorktreeGroup {
        primary: AbsolutePath,
        linked:  Vec<AbsolutePath>,
    },
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn dir(s: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(s)) }

    fn project(root: &str) -> TargetDirMember {
        TargetDirMember {
            project_root: dir(root),
        }
    }

    #[test]
    fn upsert_inserts_into_the_forward_and_reverse_maps() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a"), dir("/ws/a/target"));

        let siblings = index.siblings(&dir("/ws/a/target"), &[]);
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].project_root, dir("/ws/a"));
    }

    #[test]
    fn upsert_evicts_stale_bucket_entry_when_target_dir_changes() {
        // A project whose target_directory changes (edit .cargo/config
        // to redirect the target) must NOT leave a phantom entry in
        // the old bucket.
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a"), dir("/ws/a/target"));
        index.upsert(project("/ws/a"), dir("/tmp/custom"));

        assert!(
            index.siblings(&dir("/ws/a/target"), &[]).is_empty(),
            "stale bucket is empty after the target dir moved"
        );
        let new = index.siblings(&dir("/tmp/custom"), &[]);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].project_root, dir("/ws/a"));
    }

    #[test]
    fn upsert_is_idempotent_when_target_dir_is_unchanged() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a"), dir("/t"));
        index.upsert(project("/ws/a"), dir("/t"));

        let siblings = index.siblings(&dir("/t"), &[]);
        assert_eq!(siblings.len(), 1, "no duplicate rows for the same project");
    }

    #[test]
    fn siblings_excludes_members_named_in_the_exclude_list() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a"), dir("/shared"));
        index.upsert(project("/ws/b"), dir("/shared"));
        index.upsert(project("/ws/c"), dir("/shared"));

        let siblings = index.siblings(&dir("/shared"), &[dir("/ws/a"), dir("/ws/b")]);
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].project_root, dir("/ws/c"));
    }

    #[test]
    fn siblings_returns_empty_for_unknown_target_dir() {
        let index = TargetDirIndex::new();
        assert!(index.siblings(&dir("/nowhere"), &[]).is_empty());
    }
}
