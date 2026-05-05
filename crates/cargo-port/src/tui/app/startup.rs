//! Startup-time helpers that produce seed paths for background
//! fetches. App's startup orchestration
//! (`schedule_startup_metadata`, `schedule_startup_disk_usage`)
//! calls them once at boot.

use std::collections::HashSet;

use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::tui::project_list::ProjectList;

/// Workspace roots that should receive an initial `cargo metadata`
/// dispatch: every Rust leaf project (workspace or standalone
/// package), including each worktree in a group. Non-Rust
/// projects are skipped.
pub(super) fn initial_metadata_roots(projects: &ProjectList) -> HashSet<AbsolutePath> {
    let mut roots = HashSet::new();
    projects.for_each_leaf(|entry| {
        if let RootItem::Rust(rust) = &entry.item {
            roots.insert(rust.path().clone());
        }
    });
    roots
}

/// Top-level paths for the initial disk-usage scan. Nested
/// projects are folded under their nearest ancestor so the
/// background scanner doesn't double-walk a tree.
pub(super) fn initial_disk_roots(projects: &ProjectList) -> HashSet<AbsolutePath> {
    let mut abs_paths: Vec<&AbsolutePath> =
        projects.iter().map(|entry| entry.item.path()).collect();
    abs_paths.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });

    let mut roots: Vec<&AbsolutePath> = Vec::new();
    for abs_path in abs_paths {
        if roots.iter().any(|root| abs_path.starts_with(root)) {
            continue;
        }
        roots.push(abs_path);
    }

    roots.into_iter().cloned().collect()
}
