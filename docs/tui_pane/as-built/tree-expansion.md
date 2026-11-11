# Shared tree expansion

`ExpansionModel<K>` stores expansion overrides for application-owned trees.
The application supplies row identity and relationships through `ExpansionTree`.
The framework owns toggle and collapse behavior.

Paths below are relative to `crates/tui_pane/src/`.

## Types and operations

`expansion.rs` defines the shared contract.
`ExpansionState::{Open, Closed}` names the effective state.
`ExpansionChange::{Changed, Unchanged}` reports whether expand or collapse changed it.
The model stores an override map and a default function of row depth.
An override takes precedence over the depth default.

```rust
pub fn new(default_open: fn(usize) -> bool) -> Self;
```

The constructor belongs to `ExpansionModel<K>`.
`expand`, `collapse`, and `toggle` change individual keys.
`expand_all` and `collapse_all` visit every expandable row through the tree interface.
`reset` removes overrides.
`retain` removes keys that no longer belong to the application tree.
`from_open_keys` and `open_keys` support persisted open-key sets.

`ExpansionTree` defines associated `Key` and `Row` types.
Its required methods are:

```rust
fn row_expansion(&self, row: &Self::Row) -> RowExpansion<Self::Key>;
fn parent(&self, row: &Self::Row) -> RowParent<Self::Row>;
fn expandable_rows(&self) -> impl Iterator<Item = (Self::Key, usize)>;
```

`RowExpansion::{Leaf, Branch { key, depth }}` names whether a row can expand.
`RowParent::{Root, Parent(row)}` names its parent relationship.
`press_row` returns `RowPressResult::{Leaf, Toggled(ExpansionState)}`.
`collapse_row` returns `CollapsedRow` with the resulting row and expansion change.
A closed or leaf row can therefore move to its parent and collapse that parent.

`keymap/expansion_action.rs` and `keymap/constants.rs` own the shared expansion actions and default keys.
`EXPANSION_VIM_EXTRAS` supplies the vim additions.
Applications keep domain-specific row kinds and visibility calculations.

## Application integration

cargo-port uses `ExpansionModel<ExpandKey>` with every depth closed by default.
cargo-handler can use an open depth-zero default for its hierarchy.
The applications choose these defaults without copying the algorithm.

A focus-only click does not call the toggle operation.
A normal branch press returns Toggled and lets the application recompute visibility.
A leaf press leaves expansion unchanged.
If the tree and model share one owner, a caller can temporarily take the model to satisfy Rust's borrowing rules.
The caller must restore that model after the operation.

## Invariants and rationale

1. Keep tree identity and selection in the application.
2. Keep expansion overrides in one model.
3. Visit hidden descendants during expand-all and collapse-all.
4. Remove retired keys after tree replacement.
5. Use the shared press operation instead of an application toggle copy.

Named outcomes keep leaves, roots, unchanged operations, and toggles distinct.
A depth default avoids storing an explicit value for every row.
