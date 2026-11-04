#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;

use indexmap::IndexMap;
use tui_pane::ExpansionModel;

use super::app::FinderState;
use super::app::SelectionPaths;
use super::app::SelectionSync;
use super::columns::ProjectListWidths;
use super::panes::ProjectRowWidthInput;
use super::project_list::ExpandKey;
use super::project_list::VisibleRow;
use crate::project::AbsolutePath;
use crate::project::Comparison;
use crate::project::ProjectEntry;
use crate::scan::ProjectStorage;

/// Whether a project's primary ahead/behind sync value is known yet.
/// `Unresolved` means git info (checkout or repo) is still loading;
/// `Resolved(Comparison::NotCompared)` means it loaded without a comparison;
/// `Resolved(Comparison::Unreadable)` means its evidence could not be read.
pub(super) enum SyncResolution {
    Unresolved,
    Resolved(Comparison),
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
/// Roots preserve insertion order in an `IndexMap`. `path_index` routes each
/// project path to its root, checkout and nested node in constant time. Structural
/// mutation methods rebuild those routes before returning.
#[derive(Default)]
pub(super) struct ProjectList {
    pub(super) row_generation:      u64,
    pub(super) path_index:          HashMap<AbsolutePath, ProjectLocation>,
    pub(super) roots:               IndexMap<AbsolutePath, ProjectEntry>,
    pub(super) paths:               SelectionPaths,
    pub(super) sync:                SelectionSync,
    pub(super) expanded:            ExpansionModel<ExpandKey>,
    pub(super) finder:              FinderState,
    pub(super) cached_visible_rows: Vec<VisibleRow>,
    pub(super) cached_root_sorted:  Vec<u64>,
    pub(super) disk_cache_state:    ListCacheState<DiskCacheKey>,
    pub(super) cached_fit_widths:   ProjectListWidths,
    pub(super) fit_widths_state:    ListCacheState<FitWidthsCacheKey>,
    pub(super) storage:             ProjectStorage,
    #[cfg(test)]
    pub(super) disk_cache_builds:   u64,
    #[cfg(test)]
    pub(super) fit_width_builds:    Cell<u64>,
    pub(super) cursor:              usize,
}

/// Derived list data is reusable only for the inputs that produced it.
#[derive(Default)]
pub(super) enum ListCacheState<Key> {
    #[default]
    NotComputed,
    Current(Key),
}

impl<Key: PartialEq> ListCacheState<Key> {
    pub(super) fn is_current(&self, desired: &Key) -> bool {
        matches!(self, Self::Current(key) if key == desired)
    }
}

/// The disk scale depends only on byte values of rows currently rendered.
#[derive(PartialEq, Eq)]
pub(super) struct DiskCacheKey {
    pub(super) visible_bytes: Vec<u64>,
}

/// Widths read project data, manifest outcomes, visibility, settings and storage.
#[derive(PartialEq)]
pub(super) struct FitWidthsCacheKey {
    pub(super) presentation:     Vec<ProjectRowWidthInput>,
    pub(super) summary_bytes:    u64,
    pub(super) visible_rows:     Vec<VisibleRow>,
    pub(super) include_non_rust: bool,
    pub(super) lint_enabled:     bool,
    pub(super) storage:          ProjectStorage,
}

/// A stable route through the current hierarchy, rebuilt with structural edits.
#[derive(Clone, Copy, Debug)]
pub(super) struct ProjectLocation {
    pub root_index: usize,
    pub checkout:   CheckoutLocation,
    pub node:       ProjectNodeLocation,
    pub repository: RepositoryLocation,
}

/// Repository ownership is independent of Cargo membership at the same path.
#[derive(Clone, Copy, Debug)]
pub(super) enum RepositoryLocation {
    ContainingRoot,
    Submodule {
        root_index: usize,
        index:      usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum CheckoutLocation {
    Root,
    Primary,
    Linked(usize),
}

#[derive(Clone, Copy, Debug)]
pub(super) enum RustOwnerLocation {
    Checkout,
    Member { group: usize, member: usize },
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ProjectNodeLocation {
    Rust(RustOwnerLocation),
    Vendored {
        owner: RustOwnerLocation,
        index: usize,
    },
    NonRust,
    Submodule(usize),
}
