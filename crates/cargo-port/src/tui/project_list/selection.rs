use std::collections::HashSet;

use super::ExpandKey;
use super::ProjectList;
use crate::tui::app::FinderState;

/// RAII guard for visibility-changing [`ProjectList`] mutations.
/// Obtained via [`ProjectList::mutate`]; `Drop` recomputes
/// `cached_visible_rows`. Mutation guard (RAII) — self-only flavor.
#[allow(
    dead_code,
    reason = "guard ships alongside ProjectList so the type is in place \
              while call sites still use the direct accessors"
)]
pub struct SelectionMutation<'a> {
    pub(super) project_list:     &'a mut ProjectList,
    pub(super) include_non_rust: bool,
}

#[allow(
    dead_code,
    reason = "guard methods ship alongside the type while call sites \
              still use the direct accessors"
)]
impl SelectionMutation<'_> {
    /// Toggle membership of `key` in the expansion set. Returns `true`
    /// if the key was newly inserted.
    pub(super) fn toggle_expand(&mut self, key: ExpandKey) -> bool {
        if self.project_list.expanded.contains(&key) {
            self.project_list.expanded.remove(&key);
            false
        } else {
            self.project_list.expanded.insert(key);
            true
        }
    }

    /// Insert `key` into the expansion set. Returns `true` if the key
    /// was newly inserted.
    pub fn expand(&mut self, key: ExpandKey) -> bool { self.project_list.expanded.insert(key) }

    /// Remove `key` from the expansion set. Returns `true` if the key
    /// was present.
    pub fn collapse(&mut self, key: &ExpandKey) -> bool { self.project_list.expanded.remove(key) }

    /// Mutable access to the underlying expansion set, for bulk
    /// operations (e.g. `clear`, multi-key inserts) that still want
    /// the drop-recompute to fire afterward.
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        &mut self.project_list.expanded
    }

    /// Mutable access to the finder state, for callers that update
    /// the finder query / results inline. The drop-recompute fires
    /// on guard release.
    pub(super) const fn finder_mut(&mut self) -> &mut FinderState { &mut self.project_list.finder }
}

impl Drop for SelectionMutation<'_> {
    fn drop(&mut self) {
        self.project_list
            .recompute_visibility(self.include_non_rust);
    }
}
