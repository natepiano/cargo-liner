//! Expansion overrides for application trees.
//!
//! A key identifies one expandable row. The application supplies its depth.
//! An override takes precedence over the default rule at that depth.
//! The model stores only overrides. The application owns the tree and selection.

use std::collections::HashMap;
use std::hash::Hash;

/// The effective or explicit state of an expandable row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionState {
    /// The row shows its children.
    Open,
    /// The row hides its children.
    Closed,
}

/// Whether an operation changed the effective expansion state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionChange {
    /// The effective state changed.
    Changed,
    /// The effective state stayed the same.
    Unchanged,
}

/// Whether a row can expand, with its key and depth when it can.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowExpansion<K> {
    /// The row has no expandable children.
    Leaf,
    /// The row has children at the given tree depth.
    Branch {
        /// The application's identity for this expandable row.
        key:   K,
        /// The row's depth, with forest roots at zero.
        depth: usize,
    },
}

/// The parent row, or the absence of a parent at a forest root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RowParent<R> {
    /// The row has no parent in this forest.
    Root,
    /// The application identifies this row as the parent.
    Parent(R),
}

/// Application row identities and relationships used by expansion operations.
pub trait ExpansionTree {
    /// The application's stable identity for an expandable row.
    type Key: Eq + Hash + Clone;
    /// The application's identity for any row, including leaves.
    type Row: Clone;

    /// Return the row's expansion key and depth, or identify a leaf.
    fn row_expansion(&self, row: &Self::Row) -> RowExpansion<Self::Key>;

    /// Return the row's visible parent, or identify a forest root.
    fn parent(&self, row: &Self::Row) -> RowParent<Self::Row>;

    /// Visit every expandable row, including rows below closed ancestors.
    fn expandable_rows(&self) -> impl Iterator<Item = (Self::Key, usize)>;
}

/// The selected row and whether a collapse changed its expansion state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollapsedRow<R> {
    /// The row that keeps or receives the selection.
    pub selected: R,
    /// Whether the operation closed an open row.
    pub changed:  ExpansionChange,
}

/// One override per key, with a depth rule for rows without overrides.
#[derive(Clone, Debug)]
pub struct ExpansionModel<K> {
    overrides:    HashMap<K, ExpansionState>,
    default_open: fn(usize) -> bool,
}

impl<K: Eq + Hash + Clone> ExpansionModel<K> {
    /// Construct a model with the application's default rule.
    #[must_use]
    pub fn new(default_open: fn(usize) -> bool) -> Self {
        Self {
            overrides: HashMap::new(),
            default_open,
        }
    }

    /// Construct a closed-by-default model from explicitly open keys.
    #[must_use]
    pub fn from_open_keys(keys: impl IntoIterator<Item = K>) -> Self {
        let mut model = Self::default();
        for key in keys {
            model.expand(key, 0);
        }
        model
    }

    /// Visit keys with an explicit open override.
    pub fn open_keys(&self) -> impl Iterator<Item = &K> {
        self.overrides
            .iter()
            .filter_map(|(key, state)| (*state == ExpansionState::Open).then_some(key))
    }

    /// Return the effective state at the row's current depth.
    #[must_use]
    pub fn is_open(&self, key: &K, depth: usize) -> bool {
        match self.overrides.get(key) {
            Some(ExpansionState::Open) => true,
            Some(ExpansionState::Closed) => false,
            None => (self.default_open)(depth),
        }
    }

    /// Open a row. Return whether its effective state changed.
    pub fn expand(&mut self, key: K, depth: usize) -> ExpansionChange {
        let changed = if self.is_open(&key, depth) {
            ExpansionChange::Unchanged
        } else {
            ExpansionChange::Changed
        };
        self.overrides.insert(key, ExpansionState::Open);
        changed
    }

    /// Close a row. Return whether its effective state changed.
    pub fn collapse(&mut self, key: K, depth: usize) -> ExpansionChange {
        let changed = if self.is_open(&key, depth) {
            ExpansionChange::Changed
        } else {
            ExpansionChange::Unchanged
        };
        self.overrides.insert(key, ExpansionState::Closed);
        changed
    }

    /// Flip the effective state. Return the new state.
    pub fn toggle(&mut self, key: K, depth: usize) -> ExpansionState {
        if self.is_open(&key, depth) {
            self.collapse(key, depth);
            ExpansionState::Closed
        } else {
            self.expand(key, depth);
            ExpansionState::Open
        }
    }

    /// Open every expandable row in the tree.
    pub fn expand_all(&mut self, tree: &impl ExpansionTree<Key = K>) {
        for (key, depth) in tree.expandable_rows() {
            self.expand(key, depth);
        }
    }

    /// Close every expandable row, including rows that are open by default.
    pub fn collapse_all(&mut self, tree: &impl ExpansionTree<Key = K>) {
        self.reset();
        for (key, depth) in tree.expandable_rows() {
            self.collapse(key, depth);
        }
    }

    /// Restore the default rule for every row.
    pub fn reset(&mut self) { self.overrides.clear(); }

    /// Remove overrides whose keys no longer occur in the tree.
    pub fn retain(&mut self, mut exists: impl FnMut(&K) -> bool) {
        self.overrides.retain(|key, _| exists(key));
    }

    /// Close an open row, or select and close its parent for a closed row or leaf.
    pub fn collapse_row<T: ExpansionTree<Key = K>>(
        &mut self,
        tree: &T,
        row: T::Row,
    ) -> CollapsedRow<T::Row> {
        if let RowExpansion::Branch { key, depth } = tree.row_expansion(&row)
            && self.is_open(&key, depth)
        {
            let changed = self.collapse(key, depth);
            return CollapsedRow {
                selected: row,
                changed,
            };
        }
        match tree.parent(&row) {
            RowParent::Root => CollapsedRow {
                selected: row,
                changed:  ExpansionChange::Unchanged,
            },
            RowParent::Parent(parent) => {
                let changed = match tree.row_expansion(&parent) {
                    RowExpansion::Branch { key, depth } => self.collapse(key, depth),
                    RowExpansion::Leaf => ExpansionChange::Unchanged,
                };
                CollapsedRow {
                    selected: parent,
                    changed,
                }
            },
        }
    }
}

impl<K: Eq + Hash + Clone> Default for ExpansionModel<K> {
    fn default() -> Self { Self::new(|_| false) }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree;

    impl ExpansionTree for Tree {
        type Key = &'static str;
        type Row = &'static str;

        fn row_expansion(&self, row: &Self::Row) -> RowExpansion<Self::Key> {
            match *row {
                "root" | "other" => RowExpansion::Branch {
                    key:   *row,
                    depth: 0,
                },
                "child" => RowExpansion::Branch {
                    key:   *row,
                    depth: 1,
                },
                _ => RowExpansion::Leaf,
            }
        }

        fn parent(&self, row: &Self::Row) -> RowParent<Self::Row> {
            match *row {
                "child" => RowParent::Parent("root"),
                "leaf" => RowParent::Parent("child"),
                _ => RowParent::Root,
            }
        }

        fn expandable_rows(&self) -> impl Iterator<Item = (Self::Key, usize)> {
            [("root", 0), ("other", 0), ("child", 1)].into_iter()
        }
    }

    #[test]
    fn closed_defaults_support_expand_collapse_and_toggle() {
        let mut model = ExpansionModel::new(|_| false);
        assert!(!model.is_open(&"root", 0));
        assert!(!model.is_open(&"child", 1));
        assert_eq!(model.expand("root", 0), ExpansionChange::Changed);
        assert_eq!(model.expand("root", 0), ExpansionChange::Unchanged);
        assert!(model.is_open(&"root", 0));
        assert_eq!(model.toggle("child", 1), ExpansionState::Open);
        assert_eq!(model.toggle("child", 1), ExpansionState::Closed);
        assert_eq!(model.collapse("root", 0), ExpansionChange::Changed);
        assert_eq!(model.collapse("root", 0), ExpansionChange::Unchanged);
        assert!(!model.is_open(&"root", 0));
    }

    #[test]
    fn open_keys_constructor_restores_exact_choices() {
        let mut model = ExpansionModel::from_open_keys(["root", "child"]);
        assert!(model.is_open(&"root", 0));
        assert!(model.is_open(&"child", 1));
        assert!(!model.is_open(&"other", 0));
        model.collapse("child", 1);
        assert_eq!(model.open_keys().copied().collect::<Vec<_>>(), ["root"]);
        model.reset();
        assert!(!model.is_open(&"root", 0));
    }

    #[test]
    fn root_defaults_preserve_explicit_choices_when_depth_changes() {
        let mut model = ExpansionModel::new(|depth| depth == 0);
        assert!(model.is_open(&"root", 0));
        assert!(!model.is_open(&"child", 1));
        assert_eq!(model.toggle("root", 0), ExpansionState::Closed);
        assert_eq!(model.expand("child", 1), ExpansionChange::Changed);
        assert!(!model.is_open(&"root", 0));
        assert!(!model.is_open(&"root", 1));
        assert!(model.is_open(&"child", 2));
        assert!(model.is_open(&"new", 0));
        assert!(!model.is_open(&"new", 3));
    }

    #[test]
    fn bulk_operations_override_both_default_rules() {
        let defaults: [fn(usize) -> bool; 2] = [|_| false, |depth| depth == 0];
        for default in defaults {
            let mut model = ExpansionModel::new(default);
            model.expand_all(&Tree);
            assert!(
                Tree.expandable_rows()
                    .all(|(key, depth)| model.is_open(&key, depth))
            );
            model.collapse_all(&Tree);
            assert!(
                Tree.expandable_rows()
                    .all(|(key, depth)| !model.is_open(&key, depth))
            );
            model.reset();
            assert!(
                Tree.expandable_rows()
                    .all(|(key, depth)| model.is_open(&key, depth) == default(depth))
            );
        }
    }

    #[test]
    fn retain_drops_choices_for_removed_rows() {
        let mut model = ExpansionModel::new(|depth| depth == 0);
        model.collapse("root", 0);
        model.expand("child", 1);
        model.retain(|key| *key != "root");
        assert!(
            model.is_open(&"root", 0),
            "a returning row uses its default"
        );
        assert!(
            model.is_open(&"child", 1),
            "a surviving row keeps its choice"
        );
    }

    #[test]
    fn collapse_selects_parent_only_for_closed_rows_and_leaves() {
        let mut model = ExpansionModel::new(|depth| depth == 0);
        model.expand("child", 1);
        assert_eq!(
            model.collapse_row(&Tree, "child"),
            CollapsedRow {
                selected: "child",
                changed:  ExpansionChange::Changed,
            }
        );
        assert_eq!(
            model.collapse_row(&Tree, "child"),
            CollapsedRow {
                selected: "root",
                changed:  ExpansionChange::Changed,
            }
        );
        assert!(!model.is_open(&"root", 0));
        model.expand("child", 1);
        assert_eq!(
            model.collapse_row(&Tree, "leaf"),
            CollapsedRow {
                selected: "child",
                changed:  ExpansionChange::Changed,
            }
        );
        assert!(!model.is_open(&"child", 1));
        assert_eq!(
            model.collapse_row(&Tree, "root"),
            CollapsedRow {
                selected: "root",
                changed:  ExpansionChange::Unchanged,
            }
        );
    }
}
