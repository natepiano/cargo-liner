use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use walkdir::WalkDir;

use super::BackgroundMsg;
use super::cargo_metadata::StreamingScanContext;
use crate::project::AbsolutePath;
use crate::project::RootItem;

pub(super) fn spawn_initial_disk_usage(
    scan_context: &StreamingScanContext,
    disk_entries: &[(String, AbsolutePath)],
) {
    for tree in group_disk_usage_trees(disk_entries) {
        spawn_disk_usage_tree(scan_context, tree);
    }
}

#[derive(Clone)]
pub(super) struct DiskUsageTree {
    pub(super) root_abs_path: AbsolutePath,
    pub(super) entries:       Vec<AbsolutePath>,
}

pub(super) fn group_disk_usage_trees(
    disk_entries: &[(String, AbsolutePath)],
) -> Vec<DiskUsageTree> {
    let mut sorted: Vec<AbsolutePath> = disk_entries.iter().map(|(_, p)| p.clone()).collect();
    sorted.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut trees: Vec<DiskUsageTree> = Vec::new();
    for abs_path in sorted {
        if let Some(tree) = trees
            .iter_mut()
            .find(|tree| abs_path.starts_with(&tree.root_abs_path))
        {
            tree.entries.push(abs_path);
        } else {
            let root = abs_path.clone();
            trees.push(DiskUsageTree {
                root_abs_path: root,
                entries:       vec![abs_path],
            });
        }
    }
    trees
}

fn spawn_disk_usage_tree(scan_context: &StreamingScanContext, tree: DiskUsageTree) {
    let handle = scan_context.client.handle.clone();
    let tx = scan_context.tx.clone();
    let disk_limit = Arc::clone(&scan_context.disk_limit);

    handle.spawn(async move {
        let queue_started = std::time::Instant::now();
        let Ok(_permit) = disk_limit.acquire_owned().await else {
            return;
        };
        let queue_elapsed = queue_started.elapsed();
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(queue_elapsed.as_millis()),
            abs_path = %tree.root_abs_path.display(),
            rows = tree.entries.len(),
            "tokio_disk_queue_wait"
        );
        let run_started = std::time::Instant::now();
        let tree_for_walk = tree.clone();
        let Ok(results) =
            tokio::task::spawn_blocking(move || dir_sizes_for_tree(&tree_for_walk)).await
        else {
            return;
        };
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(run_started.elapsed().as_millis()),
            abs_path = %tree.root_abs_path.display(),
            rows = tree.entries.len(),
            "tokio_disk_usage"
        );
        let _ = tx.send(BackgroundMsg::DiskUsageBatch {
            root_path: tree.root_abs_path,
            entries:   results,
        });
    });
}

/// Per-project disk size breakdown emitted by the tree walker.
///
/// `total = in_project_target + in_project_non_target` by construction —
/// preserves the `disk_usage_bytes` formula for every owner (target is
/// in-tree) and naturally shrinks for a sharer (its `in_project_target
/// == 0` because the real target lives elsewhere under the workspace's
/// redirected `target_directory`).
///
/// "Is this file inside a `target/` subtree?" uses the literal
/// basename heuristic (any ancestor path component named `target`).
/// A workspace that redirects via `CARGO_TARGET_DIR` /
/// `.cargo/config.toml` ends up with `in_project_target = 0` for its
/// members — the sharer semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DirSizes {
    pub total:                 u64,
    pub in_project_target:     u64,
    pub in_project_non_target: u64,
}

impl DirSizes {
    fn add_file(&mut self, bytes: u64, file_path: &Path) {
        self.total += bytes;
        if file_lives_under_target(file_path) {
            self.in_project_target += bytes;
        } else {
            self.in_project_non_target += bytes;
        }
    }
}

fn file_lives_under_target(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "target")
}

fn dir_sizes_for_tree(tree: &DiskUsageTree) -> Vec<(AbsolutePath, DirSizes)> {
    let mut totals: HashMap<AbsolutePath, DirSizes> = tree
        .entries
        .iter()
        .map(|abs_path| (abs_path.clone(), DirSizes::default()))
        .collect();

    for entry in WalkDir::new(&tree.root_abs_path).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let bytes = metadata.len();
        let file_path = entry.path();
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(sizes) = totals.get_mut(dir) {
                sizes.add_file(bytes, file_path);
            }
            if dir == tree.root_abs_path.as_path() {
                break;
            }
            current = dir.parent();
        }
    }

    tree.entries
        .iter()
        .map(|abs_path| {
            let sizes = totals.get(abs_path.as_path()).copied().unwrap_or_default();
            (abs_path.clone(), sizes)
        })
        .collect()
}

pub(crate) fn disk_usage_batch_for_item(item: &RootItem) -> Vec<(AbsolutePath, DirSizes)> {
    let entries = item
        .collect_project_info()
        .into_iter()
        .map(|(path, _)| path)
        .collect();
    let tree = DiskUsageTree {
        root_abs_path: item.path().clone(),
        entries,
    };
    dir_sizes_for_tree(&tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_disk_usage_trees_merges_nested_projects_under_one_root() {
        let trees = group_disk_usage_trees(&[
            ("~/rust/bevy".to_string(), "/home/user/rust/bevy".into()),
            (
                "~/rust/bevy/crates/bevy_ecs".to_string(),
                "/home/user/rust/bevy/crates/bevy_ecs".into(),
            ),
            (
                "~/rust/bevy/tools/ci".to_string(),
                "/home/user/rust/bevy/tools/ci".into(),
            ),
            ("~/rust/hana".to_string(), "/home/user/rust/hana".into()),
        ]);

        assert_eq!(trees.len(), 2);
        assert_eq!(trees[0].root_abs_path, *Path::new("/home/user/rust/bevy"));
        assert_eq!(trees[0].entries.len(), 3);
        assert_eq!(trees[1].root_abs_path, *Path::new("/home/user/rust/hana"));
        assert_eq!(trees[1].entries.len(), 1);
    }

    #[test]
    fn dir_sizes_for_tree_accumulates_root_and_child_sizes_from_one_walk() {
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let root: AbsolutePath = tmp.path().join("bevy").into();
        let child: AbsolutePath = root.join("crates").join("bevy_ecs").into();
        std::fs::create_dir_all(&*child).unwrap_or_else(|_| std::process::abort());
        std::fs::write(root.join("root.txt"), vec![0_u8; 5])
            .unwrap_or_else(|_| std::process::abort());
        std::fs::write(child.join("child.txt"), vec![0_u8; 7])
            .unwrap_or_else(|_| std::process::abort());

        let sizes = dir_sizes_for_tree(&DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root.clone(), child.clone()],
        });
        let sizes: HashMap<AbsolutePath, DirSizes> = sizes.into_iter().collect();

        assert_eq!(sizes.get(root.as_path()).map(|s| s.total), Some(12));
        assert_eq!(sizes.get(child.as_path()).map(|s| s.total), Some(7));
    }

    #[test]
    fn dir_sizes_for_tree_splits_target_and_non_target_bytes_in_one_pass() {
        // Confirm the single-pass walker partitions bytes between
        // `in_project_target` and `in_project_non_target` based on
        // whether any ancestor path component is named `target`. A file
        // at `<root>/target/debug/foo` is counted as in-target; one at
        // `<root>/src/main.rs` is not.
        let tmp = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let root: AbsolutePath = tmp.path().join("proj").into();
        let src = root.join("src");
        let target_debug = root.join("target").join("debug");
        std::fs::create_dir_all(&src).unwrap_or_else(|_| std::process::abort());
        std::fs::create_dir_all(&target_debug).unwrap_or_else(|_| std::process::abort());
        std::fs::write(src.join("main.rs"), vec![0_u8; 3])
            .unwrap_or_else(|_| std::process::abort());
        std::fs::write(target_debug.join("proj"), vec![0_u8; 17])
            .unwrap_or_else(|_| std::process::abort());

        let sizes = dir_sizes_for_tree(&DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root],
        });
        let (_, entry) = &sizes[0];
        assert_eq!(entry.total, 20, "total bytes = 3 (src) + 17 (target)");
        assert_eq!(entry.in_project_target, 17, "target bytes isolated");
        assert_eq!(
            entry.in_project_non_target, 3,
            "non-target bytes exclude the target/ subtree"
        );
        assert_eq!(
            entry.in_project_target + entry.in_project_non_target,
            entry.total,
            "breakdown always sums to total"
        );
    }
}
