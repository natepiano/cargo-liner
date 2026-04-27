//! The `Scan` subsystem.
//!
//! Phase 6 of the App-API carve (see `docs/app-api.md`). Absorbs
//! eleven scan-cluster fields (twelve in `#[cfg(test)]`) that
//! previously lived on `App`:
//! - `projects` (the [`ProjectList`])
//! - `scan` (the [`ScanState`] state machine)
//! - `dirty` (the [`DirtyState`] dirtiness tracker)
//! - `data_generation` (per-tick monotonic counter; bumped explicitly so detail-relevance code can
//!   invalidate caches)
//! - `discovery_shimmers` (per-path shimmer animation state)
//! - `pending_git_first_commit` (per-path first-commit string waiting for tree placement)
//! - `metadata_store` (process-wide cargo-metadata store; the `Arc<Mutex<...>>` is shared with
//!   spawned threads)
//! - `target_dir_index` (workspace-target-dir index for clean planning)
//! - `priority_fetch_path` (path the next selection-driven fetch should prefer)
//! - `confirm_verifying` (workspace root waiting for a metadata refresh before the Clean confirm
//!   popup unblocks)
//! - `lint_cache_usage` (per-tick lint-cache stat counter)
//! - `retry_spawn_mode` (test-only knob that disables retry spawning for deterministic test runs)
//!
//! Phase 6 absorbs the field cluster and rewires the existing
//! [`crate::tui::app::TreeMutation`] guard to take direct `&mut`
//! references to `Scan + Panes + Selection` so the fan-out is
//! declared at the type level (the "Mutation guard (RAII), fan-out
//! flavor" pattern in `src/tui/app/mod.rs`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use super::app::DirtyState;
use super::app::DiscoveryShimmer;
#[cfg(test)]
use super::app::RetrySpawnMode;
use super::app::ScanState;
use super::app::TargetDirIndex;
use crate::lint::CacheUsage;
use crate::project::AbsolutePath;
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
    lint_cache_usage:         CacheUsage,
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
            lint_cache_usage: CacheUsage::default(),
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

    // ── dirty tracker ───────────────────────────────────────────────

    pub(super) const fn dirty(&self) -> &DirtyState { &self.dirty }

    pub(super) const fn dirty_mut(&mut self) -> &mut DirtyState { &mut self.dirty }

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

    // ── lint cache usage ────────────────────────────────────────────

    pub(super) const fn lint_cache_usage(&self) -> &CacheUsage { &self.lint_cache_usage }

    pub(super) const fn set_lint_cache_usage(&mut self, usage: CacheUsage) {
        self.lint_cache_usage = usage;
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
