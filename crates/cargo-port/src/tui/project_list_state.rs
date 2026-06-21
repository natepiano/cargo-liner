use std::collections::HashMap;
use std::collections::HashSet;

use indexmap::IndexMap;

use super::app::FinderState;
use super::app::SelectionPaths;
use super::app::SelectionSync;
use super::columns::ProjectListWidths;
use super::project_list::ExpandKey;
use super::project_list::VisibleRow;
use crate::project::AbsolutePath;
use crate::project::ProjectEntry;

/// Whether a project's primary ahead/behind sync value is known yet.
/// `Unresolved` means git info (checkout or repo) is still loading;
/// `Resolved(None)` means it loaded and there is no remote-tracking branch.
pub(super) enum SyncResolution {
    Unresolved,
    Resolved(Option<(usize, usize)>),
}

pub(super) struct LintRuntimeRootEntry {
    pub(super) path:                AbsolutePath,
    pub(super) linked_primary_root: Option<AbsolutePath>,
}

/// Owning wrapper around the project hierarchy plus all project-list
/// navigation state (cursor, expansion set, finder, sort/width caches).
///
/// `ProjectList` is the single source of truth for project data and the
/// per-pane state that navigates that data. Mutations go through its
/// methods; derived state (e.g. `cached_visible_rows`) is computed from
/// it on demand or refreshed by the `SelectionMutation` guard.
///
/// The underlying store is `IndexMap<AbsolutePath, ProjectEntry>` keyed by
/// each root's absolute path. The map preserves insertion order so
/// iteration stays deterministic, and gives O(1) root-path lookups via
/// `get` without a separate index that would have to be kept in sync by
/// convention. Every mutation site updates keys and values together, so
/// the "key matches the root's own path" invariant cannot silently drift.
#[derive(Default)]
pub(super) struct ProjectList {
    pub(super) roots:               IndexMap<AbsolutePath, ProjectEntry>,
    pub(super) paths:               SelectionPaths,
    pub(super) sync:                SelectionSync,
    pub(super) expanded:            HashSet<ExpandKey>,
    pub(super) finder:              FinderState,
    pub(super) cached_visible_rows: Vec<VisibleRow>,
    pub(super) cached_root_sorted:  Vec<u64>,
    pub(super) cached_child_sorted: HashMap<usize, Vec<u64>>,
    pub(super) cached_fit_widths:   ProjectListWidths,
    pub(super) cursor:              usize,
}
