//! The `Scan` subsystem.
//!
//! Owns the scan-cluster fields: project list and tree, scan state
//! machine, dirtiness tracker, per-tick data generation counter,
//! discovery shimmer animations, pending first-commit strings,
//! cargo-metadata store, workspace-target-dir index, priority-fetch
//! path, the workspace root awaiting Clean confirm, and (in test
//! builds) the retry-spawn knob.
//!
//! [`crate::tui::app::TreeMutation`] takes direct `&mut` references
//! to `Scan + Panes + Selection` so the mutation fan-out is declared
//! at the type level (see the "Mutation guard (RAII), fan-out
//! flavor" pattern in `src/tui/app/mod.rs`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use super::app::DirtyState;
use super::app::DiscoveryShimmer;
#[cfg(test)]
use super::app::RetrySpawnMode;
use super::app::ScanState;
use super::app::TargetDirIndex;
use crate::project::AbsolutePath;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataHandle;
use crate::project::WorkspaceMetadataStore;
use crate::project_list::ProjectList;

pub(super) struct Scan {
    projects:                 ProjectList,
    state:                    ScanState,
    dirty:                    DirtyState,
    data_generation:          u64,
    discovery_shimmers:       HashMap<AbsolutePath, DiscoveryShimmer>,
    pending_git_first_commit: HashMap<AbsolutePath, String>,
    metadata_store:           Arc<Mutex<WorkspaceMetadataStore>>,
    target_dir_index:         TargetDirIndex,
    priority_fetch_path:      Option<AbsolutePath>,
    confirm_verifying:        Option<AbsolutePath>,
    #[cfg(test)]
    retry_spawn_mode:         RetrySpawnMode,
}

impl Scan {
    pub(super) fn new(
        projects: ProjectList,
        state: ScanState,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Self {
        Self {
            projects,
            state,
            dirty: DirtyState::initial(),
            data_generation: 0,
            discovery_shimmers: HashMap::new(),
            pending_git_first_commit: HashMap::new(),
            metadata_store,
            target_dir_index: TargetDirIndex::new(),
            priority_fetch_path: None,
            confirm_verifying: None,
            #[cfg(test)]
            retry_spawn_mode: RetrySpawnMode::Enabled,
        }
    }

    // ── projects ────────────────────────────────────────────────────

    pub(super) const fn projects(&self) -> &ProjectList { &self.projects }

    pub(super) const fn projects_mut(&mut self) -> &mut ProjectList { &mut self.projects }

    // ── scan-state machine ──────────────────────────────────────────

    pub(super) const fn scan_state(&self) -> &ScanState { &self.state }

    pub(super) const fn scan_state_mut(&mut self) -> &mut ScanState { &mut self.state }

    pub(super) const fn is_complete(&self) -> bool { self.state.phase.is_complete() }

    // ── dirty tracker ───────────────────────────────────────────────

    pub(super) const fn terminal_is_dirty(&self) -> bool { self.dirty.terminal.is_dirty() }

    pub(super) const fn mark_terminal_dirty(&mut self) { self.dirty.terminal.mark_dirty(); }

    pub(super) const fn clear_terminal_dirty(&mut self) { self.dirty.terminal.mark_clean(); }

    // ── data generation ─────────────────────────────────────────────

    pub(super) const fn generation(&self) -> u64 { self.data_generation }

    pub(super) const fn bump_generation(&mut self) { self.data_generation += 1; }

    // ── discovery shimmers ──────────────────────────────────────────

    pub(super) const fn discovery_shimmers(&self) -> &HashMap<AbsolutePath, DiscoveryShimmer> {
        &self.discovery_shimmers
    }

    pub(super) const fn discovery_shimmers_mut(
        &mut self,
    ) -> &mut HashMap<AbsolutePath, DiscoveryShimmer> {
        &mut self.discovery_shimmers
    }

    pub(super) fn prune_shimmers(&mut self, now: Instant) {
        self.discovery_shimmers
            .retain(|_, shimmer| shimmer.is_active_at(now));
    }

    // ── pending git first-commit cache ──────────────────────────────

    #[cfg(test)]
    pub(super) const fn pending_git_first_commit(&self) -> &HashMap<AbsolutePath, String> {
        &self.pending_git_first_commit
    }

    pub(super) const fn pending_git_first_commit_mut(
        &mut self,
    ) -> &mut HashMap<AbsolutePath, String> {
        &mut self.pending_git_first_commit
    }

    // ── metadata store ──────────────────────────────────────────────

    pub(super) const fn metadata_store(&self) -> &Arc<Mutex<WorkspaceMetadataStore>> {
        &self.metadata_store
    }

    /// Clone of the process-wide metadata store handle. Used by scan
    /// dispatchers and async-task spawners that need a `Send` handle
    /// independent of the borrow on `Scan`.
    pub(super) fn metadata_store_handle(&self) -> Arc<Mutex<WorkspaceMetadataStore>> {
        Arc::clone(&self.metadata_store)
    }

    /// Resolve a [`WorkspaceMetadataHandle`] to a cloned
    /// [`WorkspaceMetadata`], or `None` when the workspace has no
    /// metadata yet. Locks the store, releases before return.
    #[allow(
        dead_code,
        reason = "consumed in later steps; kept now so WorkspaceMetadataHandle has a resolve path \
                  in place before handle-carrying RustInfo lands"
    )]
    pub(super) fn resolve_metadata(
        &self,
        handle: &WorkspaceMetadataHandle,
    ) -> Option<WorkspaceMetadata> {
        self.metadata_store
            .lock()
            .ok()
            .and_then(|store| store.get(&handle.workspace_root).cloned())
    }

    /// Resolve the owning workspace's `target_directory` for any path
    /// inside a known workspace.
    pub(super) fn resolve_target_dir(&self, path: &AbsolutePath) -> Option<AbsolutePath> {
        self.metadata_store
            .lock()
            .ok()
            .and_then(|store| store.resolved_target_dir(path).cloned())
    }

    // ── target-dir index ────────────────────────────────────────────

    pub(super) const fn target_dir_index(&self) -> &TargetDirIndex { &self.target_dir_index }

    pub(super) const fn target_dir_index_mut(&mut self) -> &mut TargetDirIndex {
        &mut self.target_dir_index
    }

    // ── priority fetch path ─────────────────────────────────────────

    pub(super) const fn priority_fetch_path(&self) -> Option<&AbsolutePath> {
        self.priority_fetch_path.as_ref()
    }

    pub(super) fn set_priority_fetch_path(&mut self, path: Option<AbsolutePath>) {
        self.priority_fetch_path = path;
    }

    // ── confirm-verifying slot ──────────────────────────────────────

    pub(super) const fn confirm_verifying(&self) -> Option<&AbsolutePath> {
        self.confirm_verifying.as_ref()
    }

    pub(super) fn set_confirm_verifying(&mut self, path: Option<AbsolutePath>) {
        self.confirm_verifying = path;
    }

    /// Clear `confirm_verifying` if it currently points to
    /// `workspace_root`. Called when a verifying clean for that
    /// workspace finishes (regardless of outcome).
    pub(super) fn clear_confirm_verifying_for(&mut self, workspace_root: &AbsolutePath) {
        if self.confirm_verifying.as_ref() == Some(workspace_root) {
            self.confirm_verifying = None;
        }
    }

    // ── retry-spawn mode (test-only) ────────────────────────────────

    #[cfg(test)]
    pub(super) const fn retry_spawn_mode(&self) -> RetrySpawnMode { self.retry_spawn_mode }

    #[cfg(test)]
    pub(super) const fn set_retry_spawn_mode(&mut self, mode: RetrySpawnMode) {
        self.retry_spawn_mode = mode;
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;
    use std::time::Instant;

    use super::*;

    fn fresh() -> Scan {
        Scan::new(
            ProjectList::new(Vec::new()),
            ScanState::new(Instant::now()),
            Arc::new(Mutex::new(WorkspaceMetadataStore::new())),
        )
    }

    fn abs(p: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(p)) }

    #[test]
    fn new_starts_with_zero_generation_and_clean_dirty() {
        let scan = fresh();
        assert_eq!(scan.generation(), 0);
        assert!(scan.discovery_shimmers().is_empty());
        assert!(scan.pending_git_first_commit().is_empty());
        assert!(scan.priority_fetch_path().is_none());
        assert!(scan.confirm_verifying().is_none());
    }

    #[test]
    fn bump_generation_increments_monotonically() {
        let mut scan = fresh();
        scan.bump_generation();
        scan.bump_generation();
        assert_eq!(scan.generation(), 2);
    }

    #[test]
    fn priority_fetch_path_round_trip() {
        let mut scan = fresh();
        let p = abs("/tmp/proj");
        scan.set_priority_fetch_path(Some(p.clone()));
        assert_eq!(scan.priority_fetch_path(), Some(&p));
        scan.set_priority_fetch_path(None);
        assert!(scan.priority_fetch_path().is_none());
    }

    #[test]
    fn confirm_verifying_round_trip() {
        let mut scan = fresh();
        let p = abs("/tmp/ws");
        scan.set_confirm_verifying(Some(p.clone()));
        assert_eq!(scan.confirm_verifying(), Some(&p));
        scan.set_confirm_verifying(None);
        assert!(scan.confirm_verifying().is_none());
    }

    #[test]
    fn discovery_shimmers_independent_of_pending_first_commit() {
        let mut scan = fresh();
        let p = abs("/tmp/proj");
        scan.discovery_shimmers_mut().insert(
            p.clone(),
            DiscoveryShimmer::new(Instant::now(), Duration::from_millis(50)),
        );
        assert!(scan.discovery_shimmers().contains_key(&p));
        assert!(scan.pending_git_first_commit().is_empty());
    }

    #[test]
    fn pending_git_first_commit_round_trip() {
        let mut scan = fresh();
        let p = abs("/tmp/proj");
        scan.pending_git_first_commit_mut()
            .insert(p.clone(), "abc123".to_string());
        assert_eq!(
            scan.pending_git_first_commit().get(&p).map(String::as_str),
            Some("abc123")
        );
    }

    #[test]
    fn metadata_store_returns_shared_arc() {
        let scan = fresh();
        let arc1 = Arc::clone(scan.metadata_store());
        let arc2 = Arc::clone(scan.metadata_store());
        assert!(Arc::ptr_eq(&arc1, &arc2));
    }
}
