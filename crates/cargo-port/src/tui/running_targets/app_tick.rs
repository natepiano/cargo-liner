//! `App` glue for the running-targets poller. Builds per-workspace slices
//! from cached `cargo metadata` and drives the `Panes` poller tick.

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use cargo_metadata::TargetKind;

use super::ProjectTargetSlice;
use crate::project::AbsolutePath;
use crate::project::WorkspaceMetadata;
use crate::tui::app::App;
use crate::tui::panes;
use crate::tui::panes::RunTargetKind;
use crate::tui::panes::TargetEntry;

const SOURCE_DIR: &str = "src";
const EXAMPLES_DIR: &str = "examples";
const BENCHES_DIR: &str = "benches";

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
        let mut owned = Vec::new();
        let Ok(store) = self.scan.metadata_store().lock() else {
            return self
                .collect_visible_target_slice(None)
                .into_iter()
                .collect();
        };
        owned.extend(store.by_root.values().map(|meta| {
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
        }));
        if let Some(visible) = self.collect_visible_target_slice(Some(&store.by_root)) {
            if let Some(existing) = owned
                .iter_mut()
                .find(|slice| slice.target_dir == visible.target_dir)
            {
                existing.merge_visible_targets(visible);
            } else {
                owned.push(visible);
            }
        }
        owned
    }

    fn collect_visible_target_slice(
        &self,
        metadata: Option<&HashMap<AbsolutePath, WorkspaceMetadata>>,
    ) -> Option<OwnedSlice> {
        let entries = self
            .panes
            .targets
            .content()
            .map(panes::build_target_list_from_data)?;
        let first = entries.first()?;
        let workspace = metadata.and_then(|store| workspace_for_target_entry(first, store));
        let target_dir = workspace
            .as_ref()
            .and_then(|workspace| {
                metadata.and_then(|store| {
                    store
                        .get(workspace)
                        .map(|meta| meta.target_directory.clone())
                })
            })
            .unwrap_or_else(|| AbsolutePath::from(first.project_path.as_path().join("target")));
        let workspace_root = workspace.unwrap_or_else(|| first.project_path.clone());
        let mut bench_names = HashSet::new();
        let mut bin_names = HashSet::new();
        let mut member_dirs = HashMap::new();
        for entry in entries {
            match entry.kind {
                RunTargetKind::Bench => {
                    bench_names.insert(entry.name.clone());
                },
                RunTargetKind::Binary => {
                    bin_names.insert(entry.name.clone());
                },
                RunTargetKind::Example => {},
            }
            let member_dir = member_dir_for_target_entry(&entry);
            member_dirs.insert((entry.kind, entry.name), member_dir);
        }
        Some(OwnedSlice {
            target_dir: canonicalize_path(&target_dir),
            workspace_root,
            bench_names,
            bin_names,
            member_dirs,
        })
    }
}

impl OwnedSlice {
    fn merge_visible_targets(&mut self, visible: Self) {
        self.bench_names.extend(visible.bench_names);
        self.bin_names.extend(visible.bin_names);
        self.member_dirs.extend(visible.member_dirs);
    }
}

fn canonicalize_path(path: &AbsolutePath) -> AbsolutePath {
    path.as_path()
        .canonicalize()
        .map_or_else(|_| path.clone(), AbsolutePath::from)
}

fn workspace_for_target_entry(
    entry: &TargetEntry,
    metadata: &HashMap<AbsolutePath, WorkspaceMetadata>,
) -> Option<AbsolutePath> {
    metadata
        .iter()
        .find(|(root, _)| entry.src_path.as_path().starts_with(root.as_path()))
        .map(|(root, _)| root.clone())
}

fn member_dir_for_target_entry(entry: &TargetEntry) -> AbsolutePath {
    let dir_name = match entry.kind {
        RunTargetKind::Binary => SOURCE_DIR,
        RunTargetKind::Example => EXAMPLES_DIR,
        RunTargetKind::Bench => BENCHES_DIR,
    };
    entry
        .src_path
        .as_path()
        .ancestors()
        .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(dir_name))
        .and_then(|target_dir| target_dir.parent())
        .map_or_else(|| entry.project_path.clone(), AbsolutePath::from)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;
    use crate::project::FileStamp;
    use crate::project::ManifestFingerprint;
    use crate::tui::panes::TargetSource;
    use crate::tui::panes::TargetsData;

    fn path(path: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(path)) }

    fn example_entry(project_path: &str, member_path: &str, name: &str) -> TargetEntry {
        TargetEntry {
            name:              name.to_string(),
            display_name:      name.to_string(),
            kind:              RunTargetKind::Example,
            source:            TargetSource::member("bevy_diegetic".to_string()),
            project_path:      path(project_path),
            package_name:      "bevy_diegetic".to_string(),
            src_path:          path(&format!("{member_path}/examples/{name}.rs")),
            required_features: Vec::new(),
        }
    }

    fn metadata(workspace_root: &str, target_directory: &str) -> WorkspaceMetadata {
        WorkspaceMetadata {
            workspace_root:           path(workspace_root),
            target_directory:         path(target_directory),
            packages:                 HashMap::new(),
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        }
    }

    #[test]
    fn visible_targets_supply_a_slice_before_metadata_lands() {
        let mut app = crate::tui::test_support::make_app(&[]);
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![example_entry(
                "/tmp/hana",
                "/tmp/hana/crates/bevy_diegetic",
                "oit_resize_repro",
            )],
            benches:  Vec::new(),
        });

        let slices = app.collect_running_target_slices();
        let slice = &slices[0];
        let key = (RunTargetKind::Example, "oit_resize_repro".to_string());

        assert_eq!(slices.len(), 1);
        assert_eq!(slice.target_dir, path("/tmp/hana/target"));
        assert_eq!(
            slice.member_dirs.get(&key),
            Some(&path("/tmp/hana/crates/bevy_diegetic"))
        );
    }

    #[test]
    fn visible_targets_augment_a_matching_metadata_slice() {
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .unwrap_or_else(|_| std::process::abort())
            .upsert(metadata("/tmp/hana", "/tmp/hana/target"));
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![example_entry(
                "/tmp/hana",
                "/tmp/hana/crates/bevy_diegetic",
                "oit_resize_repro",
            )],
            benches:  Vec::new(),
        });

        let slices = app.collect_running_target_slices();
        let key = (RunTargetKind::Example, "oit_resize_repro".to_string());

        assert_eq!(slices.len(), 1);
        assert_eq!(
            slices[0].member_dirs.get(&key),
            Some(&path("/tmp/hana/crates/bevy_diegetic"))
        );
    }
}
