use std::path::Path;

use crate::config::NonRustInclusion;
use crate::project::RootItem;
use crate::scan;
use crate::scan::MetadataDispatchContext;
use crate::tui::panes::Panes;
use crate::tui::project_list::ProjectList;

/// RAII guard for structural mutations of the project tree.
/// Obtained via `App::mutate_tree`; dropped at end of scope (or
/// earlier via `drop`), at which point all tree-derived caches are
/// invalidated.
///
/// **Type-level invariant:** the guard borrows `&mut ProjectList +
/// &mut Panes` simultaneously. New tree-mutation paths added here
/// force the cache-clear to fire on `Drop` — there is no way to
/// forget invalidation. `Drop` runs on every exit path, including
/// panics and early returns.
///
/// Mutation guard (RAII), fan-out flavor. See "Recurring patterns"
/// in [`crate::tui::app`] for the pattern.
pub struct TreeMutation<'a> {
    pub projects: &'a mut ProjectList,
    pub panes:    &'a mut Panes,
    pub non_rust: NonRustInclusion,
}

impl TreeMutation<'_> {
    /// Replace the entire project list (used by tree-build paths).
    pub fn replace_all(&mut self, projects: ProjectList) {
        self.projects.replace_roots_from(projects);
    }

    /// Insert a discovered project into the existing tree, returning
    /// `true` if the insertion changed the tree. Requires the dispatch
    /// context and schedules a `cargo metadata` refresh for the item's
    /// Rust roots — insertion and dispatch are one step, so a project
    /// can never land in the list with unscheduled metadata.
    pub fn insert_into_hierarchy(
        &mut self,
        item: RootItem,
        dispatch: &MetadataDispatchContext,
    ) -> bool {
        let roots = scan::cargo_metadata_roots_for_item(&item);
        let changed = self.projects.insert_into_hierarchy(item);
        for root in roots {
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), root);
        }
        changed
    }

    /// Replace a single leaf at `path` with `item`. Returns the previous
    /// item if one was found. Like `insert_into_hierarchy`, the dispatch
    /// context is required and the item's Rust roots get a fresh
    /// `cargo metadata` — a probed replacement arrives with a default
    /// `Cargo`, so without this its Type/edition/targets would blank out.
    pub fn replace_leaf_by_path(
        &mut self,
        path: &Path,
        item: RootItem,
        dispatch: &MetadataDispatchContext,
    ) -> Option<RootItem> {
        let roots = scan::cargo_metadata_roots_for_item(&item);
        let previous = self.projects.replace_leaf_by_path(path, item);
        for root in roots {
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), root);
        }
        previous
    }

    /// Re-bucket workspace members under inline-dir groups.
    pub fn regroup_members(&mut self, inline_dirs: &[String]) {
        self.projects.regroup_members(inline_dirs);
    }

    /// Re-detect worktree groupings at the top level after a structural
    /// change (insert / replace / remove).
    pub fn regroup_top_level_worktrees(&mut self) { self.projects.regroup_top_level_worktrees(); }
}

impl Drop for TreeMutation<'_> {
    /// Fan out across the two subsystems whose derived state depends
    /// on tree structure:
    /// 1. [`Panes::clear_for_tree_change`] drops `worktree_summary_cache`.
    /// 2. [`ProjectList::recompute_visibility`] rebuilds `cached_visible_rows` against the new
    ///    tree.
    fn drop(&mut self) {
        self.panes.clear_for_tree_change();
        self.projects
            .recompute_visibility(self.non_rust.includes_non_rust());
    }
}
