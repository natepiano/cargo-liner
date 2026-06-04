//! `App` glue for the running-targets poller. Builds per-workspace slices
//! from cached `cargo metadata` and drives the `Panes` poller tick.

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use cargo_metadata::TargetKind;

use super::App;
use crate::project::AbsolutePath;
use crate::tui::panes::RunTargetKind;
use crate::tui::running_targets::ProjectTargetSlice;

/// One owned slice entry per known workspace. Lives across the tick call
/// so `ProjectTargetSlice<'_>` views can borrow from its fields.
struct OwnedSlice {
    target_dir:     AbsolutePath,
    workspace_root: AbsolutePath,
    bench_names:    HashSet<String>,
    bin_names:      HashSet<String>,
    /// Manifest dir of the workspace member that owns each `(kind, name)`
    /// target, so a running instance's Path column shows the member, not
    /// the shared workspace root.
    member_dirs:    HashMap<(RunTargetKind, String), AbsolutePath>,
}

impl App {
    /// Refresh the running-targets snapshot. Builds slices from the
    /// current `cargo metadata` cache, canonicalizing each
    /// `target_directory` so exe-path comparisons match symlinked
    /// entries. The poller gates its own cadence — calling on every
    /// frame is cheap when not due.
    pub fn running_targets_tick(&mut self, now: Instant) {
        let owned = self.collect_running_target_slices();
        let slices: Vec<ProjectTargetSlice<'_>> = owned
            .iter()
            .map(|entry| ProjectTargetSlice {
                target_dir:     &entry.target_dir,
                workspace_root: &entry.workspace_root,
                bench_names:    &entry.bench_names,
                bin_names:      &entry.bin_names,
                member_dirs:    &entry.member_dirs,
            })
            .collect();
        self.panes.running_targets_tick(now, &slices);
    }

    fn collect_running_target_slices(&self) -> Vec<OwnedSlice> {
        let Ok(store) = self.scan.metadata_store().lock() else {
            return Vec::new();
        };
        store
            .by_root
            .values()
            .map(|meta| {
                let target_dir = canonicalize_path(&meta.target_directory);
                let mut bench_names = HashSet::new();
                let mut bin_names = HashSet::new();
                let mut member_dirs = HashMap::new();
                for record in meta.packages.values() {
                    let member_dir = record
                        .manifest_path
                        .as_path()
                        .parent()
                        .map_or_else(|| meta.workspace_root.clone(), AbsolutePath::from);
                    for target in &record.targets {
                        for (cargo_kind, kind) in [
                            (TargetKind::Bin, RunTargetKind::Binary),
                            (TargetKind::Example, RunTargetKind::Example),
                            (TargetKind::Bench, RunTargetKind::Bench),
                        ] {
                            if target.kinds.contains(&cargo_kind) {
                                member_dirs.insert((kind, target.name.clone()), member_dir.clone());
                            }
                        }
                        if target.kinds.contains(&TargetKind::Bench) {
                            bench_names.insert(target.name.clone());
                        }
                        if target.kinds.contains(&TargetKind::Bin) {
                            bin_names.insert(target.name.clone());
                        }
                    }
                }
                OwnedSlice {
                    target_dir,
                    workspace_root: meta.workspace_root.clone(),
                    bench_names,
                    bin_names,
                    member_dirs,
                }
            })
            .collect()
    }
}

fn canonicalize_path(path: &AbsolutePath) -> AbsolutePath {
    path.as_path()
        .canonicalize()
        .map_or_else(|_| path.clone(), AbsolutePath::from)
}
