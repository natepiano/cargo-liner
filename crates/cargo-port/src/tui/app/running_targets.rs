//! `App` glue for the running-targets poller. Builds per-workspace slices
//! from cached `cargo metadata` and drives the `Panes` poller tick.

use std::collections::HashSet;
use std::time::Instant;

use cargo_metadata::TargetKind;

use super::App;
use crate::project::AbsolutePath;
use crate::tui::running_targets::ProjectTargetSlice;

/// One owned slice entry per known workspace. Lives across the tick call
/// so `ProjectTargetSlice<'_>` views can borrow from its fields.
struct OwnedSlice {
    target_dir:  AbsolutePath,
    bench_names: HashSet<String>,
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
                target_dir:  &entry.target_dir,
                bench_names: &entry.bench_names,
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
                for record in meta.packages.values() {
                    for target in &record.targets {
                        if target.kinds.contains(&TargetKind::Bench) {
                            bench_names.insert(target.name.clone());
                        }
                    }
                }
                OwnedSlice {
                    target_dir,
                    bench_names,
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
