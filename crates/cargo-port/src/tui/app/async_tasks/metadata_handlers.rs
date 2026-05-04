use crate::project;
use crate::project::AbsolutePath;
use crate::project::LanguageStats;
use crate::project::ManifestFingerprint;
use crate::project::WorkspaceMetadata;
use crate::scan;
use crate::scan::CargoMetadataError;
use crate::tui::app::App;
use crate::tui::app::target_index::TargetDirMember;

impl App {
    /// Merge an out-of-tree target walk result into the metadata cache.
    /// Declines when the cached metadata's `target_directory` has since been
    /// redirected — a fresh walk is already in flight under the new dir.
    pub(super) fn handle_out_of_tree_target_size(
        &self,
        workspace_root: &AbsolutePath,
        target_dir: &AbsolutePath,
        bytes: u64,
    ) {
        let Ok(mut store) = self.scan.metadata_store().lock() else {
            return;
        };
        if !store.set_out_of_tree_target_bytes(workspace_root, target_dir, bytes) {
            tracing::debug!(
                workspace_root = %workspace_root.as_path().display(),
                target_dir = %target_dir.as_path().display(),
                "out_of_tree_target_size_discarded_stale"
            );
        }
    }
    pub(super) fn handle_language_stats_batch(
        &mut self,
        entries: Vec<(AbsolutePath, LanguageStats)>,
    ) {
        for (path, stats) in entries {
            if let Some(project) = self.projects_mut().at_path_mut(path.as_path()) {
                project.language_stats = Some(stats);
            }
        }
    }
    /// Merge a `cargo metadata` arrival back into the process-wide store and
    /// advance the startup metadata phase. The startup path drives UI
    /// feedback via the grouped "Running cargo metadata" tracked toast
    /// created in `start_startup_detail_toasts`; post-startup per-workspace
    /// spinners land with Step 1b (watcher-triggered refresh) — until then
    /// only the startup path can arrive here.
    pub(super) fn handle_cargo_metadata_msg(
        &mut self,
        workspace_root: AbsolutePath,
        generation: u64,
        fingerprint: &ManifestFingerprint,
        result: Result<WorkspaceMetadata, CargoMetadataError>,
    ) {
        let Some(is_current) = self
            .scan
            .metadata_store()
            .lock()
            .ok()
            .map(|store| store.is_current_generation(&workspace_root, generation))
        else {
            tracing::warn!(
                workspace_root = %workspace_root.as_path().display(),
                generation,
                "cargo_metadata_store_lock_poisoned"
            );
            return;
        };
        if !is_current {
            tracing::debug!(
                workspace_root = %workspace_root.as_path().display(),
                generation,
                "cargo_metadata_msg_stale_generation"
            );
            return;
        }

        match result {
            Ok(workspace_metadata) => {
                if !self.accept_cargo_metadata(
                    &workspace_root,
                    generation,
                    fingerprint,
                    workspace_metadata,
                ) {
                    return;
                }
            },
            Err(err) => match err.user_facing_message() {
                Some(message) => {
                    let label = project::home_relative_path(workspace_root.as_path());
                    self.show_timed_toast(
                        format!("cargo metadata failed ({label})"),
                        message.to_string(),
                    );
                    tracing::warn!(
                        workspace_root = %workspace_root.as_path().display(),
                        generation,
                        error = %message,
                        "cargo_metadata_failed"
                    );
                },
                None => {
                    // `WorkspaceMissing`: the workspace root vanished
                    // between dispatch and run (typically the user just
                    // deleted a worktree). Stale-refresh race, not a real
                    // failure — suppress the toast.
                    tracing::debug!(
                        workspace_root = %workspace_root.as_path().display(),
                        generation,
                        "cargo_metadata_workspace_missing"
                    );
                },
            },
        }

        if let Some(task_id) = self.scan.scan_state_mut().startup_phases.metadata.toast {
            let key = workspace_root.to_string();
            self.toasts.mark_item_completed(task_id, &key);
        }
        // Step 6e: if the user had a confirm popup waiting on this
        // workspace's re-fingerprint, clear the Verifying flag so
        // the next render shows Ready and 'y' starts working again.
        self.scan.clear_confirm_verifying_for(&workspace_root);
        self.scan
            .scan_state_mut()
            .startup_phases
            .metadata
            .seen
            .insert(workspace_root);
        self.maybe_log_startup_phase_completions();
    }
    /// Merge a successful `cargo metadata` arrival. Returns `false` when the
    /// arrival was dropped because the captured fingerprint no longer
    /// matches what's on disk — caller should skip startup-phase bookkeeping
    /// so a later dispatch can still tick it off.
    pub(super) fn accept_cargo_metadata(
        &mut self,
        workspace_root: &AbsolutePath,
        generation: u64,
        fingerprint: &ManifestFingerprint,
        workspace_metadata: WorkspaceMetadata,
    ) -> bool {
        let current_fp =
            crate::project::ManifestFingerprint::capture(workspace_root.as_path()).ok();
        let fingerprint_drift = current_fp
            .as_ref()
            .is_some_and(|current| current != fingerprint);
        if fingerprint_drift {
            tracing::debug!(
                workspace_root = %workspace_root.as_path().display(),
                generation,
                "cargo_metadata_msg_fingerprint_drift"
            );
            return false;
        }
        let target_directory = workspace_metadata.target_directory.clone();
        let member_roots = workspace_member_roots(&workspace_metadata);
        let needs_out_of_tree_walk = !target_directory
            .as_path()
            .starts_with(workspace_root.as_path());
        // Step 3b: stamp Cargo fields (types / examples / benches /
        // test_count / publishable) from each PackageRecord onto the
        // matching Package / Workspace / VendoredPackage in the
        // project list. Retires the hand-parsed defaults left in
        // place by `from_cargo_toml`; the authoritative view is the
        // workspace metadata.
        self.apply_cargo_fields_from_workspace_metadata(&workspace_metadata);
        if let Ok(mut store) = self.scan.metadata_store().lock() {
            store.upsert(workspace_metadata);
        }
        if needs_out_of_tree_walk {
            scan::spawn_out_of_tree_target_walk(
                &self.net.http_client_ref().handle,
                self.background.bg_sender(),
                workspace_root.clone(),
                target_directory.clone(),
            );
        }
        // Refresh the target-dir index so build_clean_plan / siblings
        // lookups see the fresh membership. Every package under this
        // workspace shares `target_directory`; upsert each so a
        // subsequent clean on any member resolves to the correct dir.
        // (Members that were in *previous* metadata but not this one
        // will linger until a full scan restart — minor staleness,
        // acceptable for Step 6c.)
        for project_root in member_roots {
            self.scan
                .target_dir_index_mut()
                .upsert(TargetDirMember { project_root }, target_directory.clone());
        }
        tracing::info!(
            workspace_root = %workspace_root.as_path().display(),
            generation,
            "cargo_metadata_applied"
        );
        true
    }
    /// Step 3b: derive [`Cargo`] fields from every [`PackageRecord`] in
    /// `workspace_metadata` and stamp them onto the matching live project
    /// entry (standalone package, workspace member, or vendored package).
    /// Workspaces themselves keep the empty-default `Cargo` the parser
    /// produces — they have no single `PackageRecord`; members fan out
    /// into individual packages underneath.
    pub(super) fn apply_cargo_fields_from_workspace_metadata(
        &mut self,
        metadata: &WorkspaceMetadata,
    ) {
        use crate::project::Cargo;
        for record in metadata.packages.values() {
            let Some(manifest_dir) = record.manifest_path.as_path().parent() else {
                continue;
            };
            let cargo = Cargo::from_package_record(record);
            if let Some(rust_info) = self.projects_mut().rust_info_at_path_mut(manifest_dir) {
                rust_info.cargo = cargo.clone();
            }
            if let Some(vendored) = self.projects_mut().vendored_at_path_mut(manifest_dir) {
                vendored.cargo = cargo;
            }
        }
    }
}

/// Project root for each package covered by a [`WorkspaceMetadata`] —
/// derived from each package's `manifest_path.parent()`. Feeds the
/// `TargetDirIndex` membership update after a successful
/// `BackgroundMsg::CargoMetadata` arrival; every package under a given
/// workspace shares the metadata's `target_directory`.
fn workspace_member_roots(workspace_metadata: &WorkspaceMetadata) -> Vec<AbsolutePath> {
    workspace_metadata
        .packages
        .values()
        .filter_map(|pkg| {
            pkg.manifest_path
                .as_path()
                .parent()
                .map(|parent| AbsolutePath::from(parent.to_path_buf()))
        })
        .collect()
}
