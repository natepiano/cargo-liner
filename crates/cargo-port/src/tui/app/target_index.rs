//! `TargetDirIndex` — forward + reverse map from `target_directory`
//! to the projects that live under it. Built incrementally in the TUI
//! event loop as `BackgroundMsg::CargoMetadata` messages land.
//!
//! See `docs/cargo_metadata.md` → **Target-dir index** for the design
//! rationale; this module is the Phase-2 data primitive consumed by
//! `build_clean_plan` (Step 6) and the worktree-group fan-out clean
//! (Step 7).

#![allow(
    dead_code,
    reason = "Step 6c wires these types into the App/handler; for now they \
              stand alone with thorough tests"
)]

use std::collections::HashMap;
use std::collections::HashSet;

use crate::project::AbsolutePath;
use crate::project::WorkspaceMetadataStore;

/// Classification used by the confirm dialog to decide whether a
/// sibling sharing a target dir is a real "collateral" project or a
/// nested crate that should be collapsed into a summary line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in super::super) enum MemberKind {
    /// A first-class project row — listed explicitly in the confirm
    /// dialog when it shares a target dir with the selection.
    Project,
    /// A submodule inside another project. Collapsed into the "N
    /// nested crate(s) also share this target" line.
    Submodule,
    /// A vendored crate nested inside another project. Same collapse
    /// treatment as [`MemberKind::Submodule`].
    Vendored,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct TargetDirMember {
    pub project_root: AbsolutePath,
    pub kind:         MemberKind,
}

/// Forward map: `target_directory` → projects that resolve to it.
/// Reverse map: `project_root` → its current `target_directory`.
/// Both maps stay in sync via [`TargetDirIndex::upsert`] /
/// [`TargetDirIndex::remove`].
#[derive(Debug, Default)]
pub(in super::super) struct TargetDirIndex {
    by_target_dir: HashMap<AbsolutePath, Vec<TargetDirMember>>,
    by_project:    HashMap<AbsolutePath, AbsolutePath>,
}

impl TargetDirIndex {
    pub(in super::super) fn new() -> Self { Self::default() }

    /// Set/replace the `target_dir` for `member.project_root`. If the
    /// project previously lived under a different target dir, the
    /// stale entry is evicted before inserting the new one. Safe to
    /// call repeatedly with the same inputs.
    pub(in super::super) fn upsert(
        &mut self,
        member: TargetDirMember,
        target_dir: AbsolutePath,
    ) {
        let project_root = member.project_root.clone();
        if let Some(previous_dir) = self.by_project.get(&project_root).cloned() {
            if previous_dir == target_dir {
                // No churn — just refresh kind in case it changed.
                if let Some(bucket) = self.by_target_dir.get_mut(&target_dir) {
                    if let Some(existing) = bucket
                        .iter_mut()
                        .find(|m| m.project_root == project_root)
                    {
                        existing.kind = member.kind;
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

    /// Remove all entries for `project_root`. Called when a project
    /// disappears from the scan set.
    pub(in super::super) fn remove(&mut self, project_root: &AbsolutePath) {
        if let Some(dir) = self.by_project.remove(project_root) {
            self.evict_from_bucket(&dir, project_root);
        }
    }

    /// Every member that resolves to `target_dir`, minus any project
    /// whose root is in `exclude`. The caller (usually `build_clean_plan`)
    /// passes its selection-set here so self-members don't get listed
    /// as "collateral" in the confirm dialog.
    pub(in super::super) fn siblings<'a>(
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

    /// Current target dir for `project_root`, if known.
    pub(in super::super) fn target_dir_for(
        &self,
        project_root: &AbsolutePath,
    ) -> Option<&AbsolutePath> {
        self.by_project.get(project_root)
    }

    fn evict_from_bucket(
        &mut self,
        target_dir: &AbsolutePath,
        project_root: &AbsolutePath,
    ) {
        if let Some(bucket) = self.by_target_dir.get_mut(target_dir) {
            bucket.retain(|m| m.project_root != *project_root);
            if bucket.is_empty() {
                self.by_target_dir.remove(target_dir);
            }
        }
    }
}

// ── CleanPlan / build_clean_plan ─────────────────────────────────────

/// Which projects the user is asking to clean.
///
/// A `Project` selection is the single-row case (a `VisibleRow::Root`
/// on a Rust project). `WorktreeGroup` is the worktree-group header
/// case — `primary` is the canonical checkout, `linked` lists every
/// linked worktree at selection time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) enum CleanSelection {
    Project {
        root: AbsolutePath,
    },
    WorktreeGroup {
        primary: AbsolutePath,
        linked:  Vec<AbsolutePath>,
    },
}

impl CleanSelection {
    fn selection_set(&self) -> Vec<AbsolutePath> {
        match self {
            Self::Project { root } => vec![root.clone()],
            Self::WorktreeGroup { primary, linked } => std::iter::once(primary.clone())
                .chain(linked.iter().cloned())
                .collect(),
        }
    }
}

/// Why a project in the selection was dropped from the plan without
/// contributing a [`CleanTarget`]. Surfaced to the confirm dialog and
/// the progress toast.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) enum SkipReason {
    /// No `cargo metadata` snapshot covers this project yet — clean
    /// can't resolve the right target dir.
    NoMetadata,
    /// A worktree whose directory is gone from disk but still listed
    /// by git. Group-level cleans carry on; single-project cleans
    /// surface it as the selection being invalid.
    DeletedWorktree,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) enum CleanMethod {
    /// Shell out to `cargo clean` with `cwd = project root`. Cleans
    /// the entire `target_directory` resolved from that cwd — any
    /// sibling sharing the dir is wiped alongside. The confirm dialog
    /// must list those siblings so the user knows.
    CargoClean { cwd: AbsolutePath },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in super::super) struct CleanTarget {
    pub target_directory:  AbsolutePath,
    pub exists_on_disk:    bool,
    pub method:            CleanMethod,
    /// Every selected project whose target dir resolves here. For
    /// shared targets this lists multiple worktrees; for unique
    /// targets it's a single entry.
    pub covering_projects: Vec<AbsolutePath>,
    /// Projects *outside* the selection that share this target dir
    /// (populated from the index via `siblings(…, selection_set)`).
    /// Excludes the nested-crate kinds — those go in `nested_extras`.
    pub affected_extras:   Vec<AbsolutePath>,
    /// Submodule / Vendored members sharing this target dir. The
    /// confirm dialog collapses these behind "N nested crate(s) also
    /// share this target."
    pub nested_extras:     Vec<AbsolutePath>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in super::super) struct CleanPlan {
    /// Deduped on `target_directory`.
    pub targets:         Vec<CleanTarget>,
    /// Flat union of every `CleanTarget.affected_extras` — convenience
    /// for callers that just need "who else gets wiped."
    pub affected_extras: Vec<AbsolutePath>,
    pub skipped:         Vec<(AbsolutePath, SkipReason)>,
}

impl CleanPlan {
    /// "Already clean" (per design): every target dir is absent on
    /// disk **and** no collateral share is listed. Prevents the
    /// shared-target regression where the first clean wipes the dir
    /// and every subsequent click wrongly toasts "Already clean."
    pub(in super::super) fn is_already_clean(&self) -> bool {
        self.targets.iter().all(|t| !t.exists_on_disk)
            && self.affected_extras.is_empty()
    }
}

/// Build the [`CleanPlan`] for `selection`. Pure function — takes the
/// [`TargetDirIndex`] and [`WorkspaceMetadataStore`] by reference,
/// returns an owned plan. The caller wires the result into the
/// confirm dialog and (on user accept) the clean execution pipeline.
///
/// Uses the index's [`siblings`](TargetDirIndex::siblings) accessor
/// with `selection_set` as the `exclude` argument so the selection's
/// own members don't appear as "collateral extras" (design plan →
/// **Mixed worktree groups → Selection exclusion**).
pub(in super::super) fn build_clean_plan(
    index: &TargetDirIndex,
    store: &WorkspaceMetadataStore,
    selection: &CleanSelection,
) -> CleanPlan {
    let selection_set = selection.selection_set();
    let mut plan = CleanPlan::default();
    // Dedupe on target_directory as we walk the selection.
    let mut by_target: HashMap<AbsolutePath, CleanTarget> = HashMap::new();
    let mut affected_extras_set: HashSet<AbsolutePath> = HashSet::new();

    for project_root in &selection_set {
        // Resolve the target dir: prefer the index (authoritative when
        // metadata has landed); fall back to the store's resolution
        // path; surface `NoMetadata` when neither covers it.
        let Some(target_dir) = index
            .target_dir_for(project_root)
            .cloned()
            .or_else(|| store.resolved_target_dir(project_root).cloned())
        else {
            plan.skipped
                .push((project_root.clone(), SkipReason::NoMetadata));
            continue;
        };

        let entry = by_target
            .entry(target_dir.clone())
            .or_insert_with(|| CleanTarget {
                target_directory: target_dir.clone(),
                exists_on_disk: target_dir.as_path().exists(),
                method: CleanMethod::CargoClean {
                    cwd: project_root.clone(),
                },
                covering_projects: Vec::new(),
                affected_extras: Vec::new(),
                nested_extras: Vec::new(),
            });
        if !entry.covering_projects.contains(project_root) {
            entry.covering_projects.push(project_root.clone());
        }
    }

    // Populate affected_extras + nested_extras once per unique target.
    for target in by_target.values_mut() {
        for sibling in index.siblings(&target.target_directory, &selection_set) {
            match sibling.kind {
                MemberKind::Project => {
                    if !target.affected_extras.contains(&sibling.project_root) {
                        target.affected_extras.push(sibling.project_root.clone());
                    }
                    affected_extras_set.insert(sibling.project_root.clone());
                },
                MemberKind::Submodule | MemberKind::Vendored => {
                    if !target.nested_extras.contains(&sibling.project_root) {
                        target.nested_extras.push(sibling.project_root.clone());
                    }
                },
            }
        }
        target.affected_extras.sort_by(|a, b| a.as_path().cmp(b.as_path()));
        target.nested_extras.sort_by(|a, b| a.as_path().cmp(b.as_path()));
    }

    let mut targets: Vec<CleanTarget> = by_target.into_values().collect();
    targets.sort_by(|a, b| a.target_directory.as_path().cmp(b.target_directory.as_path()));
    plan.targets = targets;
    let mut affected_extras: Vec<AbsolutePath> = affected_extras_set.into_iter().collect();
    affected_extras.sort_by(|a, b| a.as_path().cmp(b.as_path()));
    plan.affected_extras = affected_extras;
    plan
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

    fn project(root: &str, kind: MemberKind) -> TargetDirMember {
        TargetDirMember {
            project_root: dir(root),
            kind,
        }
    }

    #[test]
    fn upsert_inserts_into_the_forward_and_reverse_maps() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/ws/a/target"));

        let siblings = index.siblings(&dir("/ws/a/target"), &[]);
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].project_root, dir("/ws/a"));
        assert_eq!(siblings[0].kind, MemberKind::Project);
        assert_eq!(
            index.target_dir_for(&dir("/ws/a")).cloned(),
            Some(dir("/ws/a/target"))
        );
    }

    #[test]
    fn upsert_evicts_stale_bucket_entry_when_target_dir_changes() {
        // A project whose target_directory changes (edit .cargo/config
        // to redirect the target) must NOT leave a phantom entry in
        // the old bucket.
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/ws/a/target"));
        index.upsert(
            project("/ws/a", MemberKind::Project),
            dir("/tmp/custom"),
        );

        assert!(
            index.siblings(&dir("/ws/a/target"), &[]).is_empty(),
            "stale bucket is empty after the target dir moved"
        );
        let new = index.siblings(&dir("/tmp/custom"), &[]);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].project_root, dir("/ws/a"));
    }

    #[test]
    fn upsert_refreshes_kind_without_duplicating_when_target_dir_is_unchanged() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/t"));
        index.upsert(project("/ws/a", MemberKind::Vendored), dir("/t"));

        let siblings = index.siblings(&dir("/t"), &[]);
        assert_eq!(siblings.len(), 1, "no duplicate rows for the same project");
        assert_eq!(siblings[0].kind, MemberKind::Vendored, "kind was refreshed");
    }

    #[test]
    fn siblings_excludes_members_named_in_the_exclude_list() {
        // `build_clean_plan` passes `selection_set` as `exclude` to
        // avoid reporting its own members as collateral.
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/shared"));
        index.upsert(project("/ws/b", MemberKind::Project), dir("/shared"));
        index.upsert(project("/ws/c", MemberKind::Project), dir("/shared"));

        let siblings = index.siblings(&dir("/shared"), &[dir("/ws/a"), dir("/ws/b")]);
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].project_root, dir("/ws/c"));
    }

    #[test]
    fn remove_fully_evicts_from_both_maps() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/t"));
        index.upsert(project("/ws/b", MemberKind::Project), dir("/t"));

        index.remove(&dir("/ws/a"));
        let siblings = index.siblings(&dir("/t"), &[]);
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].project_root, dir("/ws/b"));
        assert!(index.target_dir_for(&dir("/ws/a")).is_none());
    }

    #[test]
    fn remove_is_noop_for_unknown_project() {
        let mut index = TargetDirIndex::new();
        index.remove(&dir("/never/added"));
    }

    #[test]
    fn remove_last_project_empties_the_bucket_completely() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/t"));
        index.remove(&dir("/ws/a"));
        assert!(index.siblings(&dir("/t"), &[]).is_empty());
        assert_eq!(index.by_target_dir.len(), 0);
    }

    #[test]
    fn siblings_returns_empty_for_unknown_target_dir() {
        let index = TargetDirIndex::new();
        assert!(index.siblings(&dir("/nowhere"), &[]).is_empty());
    }

    // ── build_clean_plan matrix ──────────────────────────────────────

    fn empty_store() -> WorkspaceMetadataStore { WorkspaceMetadataStore::new() }

    #[test]
    fn plan_for_single_project_with_no_sharing_emits_one_target() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/ws/a/target"));

        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::Project { root: dir("/ws/a") },
        );

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].target_directory, dir("/ws/a/target"));
        assert_eq!(plan.targets[0].covering_projects, vec![dir("/ws/a")]);
        assert!(plan.targets[0].affected_extras.is_empty());
        assert!(plan.affected_extras.is_empty());
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn plan_reports_selection_without_metadata_as_skipped_no_metadata() {
        let index = TargetDirIndex::new();
        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::Project { root: dir("/ws/never") },
        );
        assert!(plan.targets.is_empty());
        assert_eq!(
            plan.skipped,
            vec![(dir("/ws/never"), SkipReason::NoMetadata)]
        );
    }

    #[test]
    fn group_of_three_worktrees_each_with_own_target_dedupes_to_three_targets() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/ws/a/target"));
        index.upsert(project("/ws/b", MemberKind::Project), dir("/ws/b/target"));
        index.upsert(project("/ws/c", MemberKind::Project), dir("/ws/c/target"));

        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::WorktreeGroup {
                primary: dir("/ws/a"),
                linked:  vec![dir("/ws/b"), dir("/ws/c")],
            },
        );

        assert_eq!(plan.targets.len(), 3);
        for target in &plan.targets {
            assert_eq!(target.covering_projects.len(), 1);
            assert!(target.affected_extras.is_empty());
        }
    }

    #[test]
    fn group_with_shared_target_collapses_into_single_clean_target() {
        // Worktree group {a, b, c} where a & b share `/shared`,
        // c has its own `/c/target`.
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/shared"));
        index.upsert(project("/ws/b", MemberKind::Project), dir("/shared"));
        index.upsert(project("/ws/c", MemberKind::Project), dir("/ws/c/target"));

        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::WorktreeGroup {
                primary: dir("/ws/a"),
                linked:  vec![dir("/ws/b"), dir("/ws/c")],
            },
        );

        assert_eq!(plan.targets.len(), 2);
        let shared = plan
            .targets
            .iter()
            .find(|t| t.target_directory == dir("/shared"))
            .expect("shared target present");
        assert_eq!(
            shared.covering_projects,
            vec![dir("/ws/a"), dir("/ws/b")],
            "both worktrees are covering projects on the shared bucket"
        );
        assert!(
            shared.affected_extras.is_empty(),
            "selection exclusion keeps selected members out of affected_extras"
        );
    }

    #[test]
    fn selection_excludes_self_from_affected_extras_but_outsiders_leak_in() {
        // {a, b} share `/shared` with a third outsider `/ws/c` that
        // also shares `/shared`. Selection = {a, b}: c must appear as
        // affected_extras (collateral), not as covering.
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws/a", MemberKind::Project), dir("/shared"));
        index.upsert(project("/ws/b", MemberKind::Project), dir("/shared"));
        index.upsert(project("/ws/c", MemberKind::Project), dir("/shared"));

        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::WorktreeGroup {
                primary: dir("/ws/a"),
                linked:  vec![dir("/ws/b")],
            },
        );

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].affected_extras, vec![dir("/ws/c")]);
        assert_eq!(plan.affected_extras, vec![dir("/ws/c")]);
    }

    #[test]
    fn nested_submodule_and_vendored_members_go_to_nested_extras() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/ws", MemberKind::Project), dir("/ws/target"));
        index.upsert(
            project("/ws/vendored/foo", MemberKind::Vendored),
            dir("/ws/target"),
        );
        index.upsert(
            project("/ws/submodules/bar", MemberKind::Submodule),
            dir("/ws/target"),
        );

        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::Project { root: dir("/ws") },
        );

        assert_eq!(plan.targets.len(), 1);
        let target = &plan.targets[0];
        assert!(
            target.affected_extras.is_empty(),
            "nested crates are NOT surfaced as collateral projects"
        );
        assert_eq!(
            target.nested_extras,
            vec![dir("/ws/submodules/bar"), dir("/ws/vendored/foo")],
            "nested crates collapse into their own bucket, sorted"
        );
    }

    #[test]
    fn plan_with_all_unique_targets_yields_distinct_cargo_clean_methods() {
        let mut index = TargetDirIndex::new();
        index.upsert(project("/a", MemberKind::Project), dir("/a/target"));
        index.upsert(project("/b", MemberKind::Project), dir("/b/target"));

        let plan = build_clean_plan(
            &index,
            &empty_store(),
            &CleanSelection::WorktreeGroup {
                primary: dir("/a"),
                linked:  vec![dir("/b")],
            },
        );

        // CleanMethod::CargoClean is the single-variant invariant
        // (design plan → "Why CleanMethod has only one variant"). If
        // a future PR adds another variant, this test will fail to
        // pattern-match and force the conversation back to the doc.
        for target in &plan.targets {
            let CleanMethod::CargoClean { cwd } = &target.method;
            assert!(
                target.covering_projects.contains(cwd),
                "cwd points at one of the covering projects"
            );
        }
    }

    #[test]
    fn is_already_clean_requires_every_target_missing_and_no_collateral() {
        let mut plan = CleanPlan::default();
        // Missing target dir + no collateral → already clean.
        plan.targets.push(CleanTarget {
            target_directory:  dir("/gone"),
            exists_on_disk:    false,
            method:            CleanMethod::CargoClean { cwd: dir("/gone") },
            covering_projects: vec![dir("/gone")],
            affected_extras:   Vec::new(),
            nested_extras:     Vec::new(),
        });
        assert!(plan.is_already_clean());

        // Any target that still exists → not clean.
        plan.targets[0].exists_on_disk = true;
        assert!(!plan.is_already_clean());
        plan.targets[0].exists_on_disk = false;

        // Collateral present → not clean (the user must see the list).
        plan.affected_extras.push(dir("/other"));
        assert!(!plan.is_already_clean());
    }
}
