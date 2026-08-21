//! One definition of where `#[cfg]` gates live in a syn tree.
//!
//! Two visitors used to track gates by hand, each overriding its own set of
//! `visit_*` methods, and both missed the same node: a gate written on a
//! statement attaches to the statement's expression, so
//! `#[cfg(test)] app.init_resource::<Injected>();` reported no gate and the
//! synthesized `use crate::reporter::Injected;` landed ungated (E0432 in any
//! build where the item is configured out).
//!
//! The list of nodes is now single. syn's `Visit` dispatches every
//! attribute-bearing node through one of nine entry points — `visit_item` calls
//! `visit_item_fn`, `visit_stmt` calls `visit_local`, and so on — so
//! [`gate_tracking_visit`] overrides those nine and every finer-grained
//! override a visitor writes for its own work inherits the gate already pushed
//! above it. A visitor opts in by implementing [`GateTracking`] and invoking the
//! macro inside its `impl Visit`; it cannot cover a different set of nodes than
//! its sibling.

use syn::Attribute;
use syn::ForeignItem;
use syn::ImplItem;
use syn::Item;
use syn::TraitItem;

use super::ConditionalAttributes;

/// The file text a visitor reads gate source from. `ConditionalAttributes`
/// reproduces a gate verbatim from the original bytes rather than re-rendering
/// the parsed `Meta`, so it needs the text and the line offsets that map a
/// span back into it.
pub(in crate::fixes) struct GateSource<'a> {
    pub(in crate::fixes) text:         &'a str,
    pub(in crate::fixes) line_offsets: &'a [usize],
}

/// A `syn::Visit` implementor that accumulates the `#[cfg]` gates enclosing the
/// node it is currently visiting. Implement the two accessors; get the
/// push/restore pair from the defaults and the traversal from
/// [`gate_tracking_visit`].
pub(in crate::fixes) trait GateTracking {
    fn gate_source(&self) -> GateSource<'_>;

    fn conditional_attributes_mut(&mut self) -> &mut ConditionalAttributes;

    /// Add `attributes`' gates to the enclosing set and return the length to
    /// hand back to [`Self::restore_gates`] once the subtree is visited.
    fn push_gates(&mut self, attributes: &[Attribute]) -> usize {
        let gates = {
            let source = self.gate_source();
            ConditionalAttributes::from_attributes(source.text, source.line_offsets, attributes)
        };
        let conditional_attributes = self.conditional_attributes_mut();
        let previous_len = conditional_attributes.len();
        conditional_attributes.extend(gates);
        previous_len
    }

    fn restore_gates(&mut self, previous_len: usize) {
        self.conditional_attributes_mut().truncate(previous_len);
    }
}

/// The gates an item contributes to imports synthesized inside it.
///
/// `Item::Mod` contributes nothing. A synthesized import is inserted in the
/// innermost enclosing module, so an inline `#[cfg(test)] mod tests` already
/// has its gate in effect at the insertion point — pushing it here would render
/// `#[cfg(test)]` a second time on the `use` line just below the `mod` that
/// carries it.
pub(in crate::fixes) fn item_gate_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Mod(_) => &[],
        other => super::item_attributes(other),
    }
}

/// Attributes on an associated item, for the same reason [`item_attributes`]
/// exists: syn publishes no accessor and the enum is `#[non_exhaustive]`.
///
/// [`item_attributes`]: super::item_attributes
pub(in crate::fixes) fn impl_item_attributes(impl_item: &ImplItem) -> &[Attribute] {
    match impl_item {
        ImplItem::Const(node) => &node.attrs,
        ImplItem::Fn(node) => &node.attrs,
        ImplItem::Macro(node) => &node.attrs,
        ImplItem::Type(node) => &node.attrs,
        _ => &[],
    }
}

pub(in crate::fixes) fn trait_item_attributes(trait_item: &TraitItem) -> &[Attribute] {
    match trait_item {
        TraitItem::Const(node) => &node.attrs,
        TraitItem::Fn(node) => &node.attrs,
        TraitItem::Macro(node) => &node.attrs,
        TraitItem::Type(node) => &node.attrs,
        _ => &[],
    }
}

pub(in crate::fixes) fn foreign_item_attributes(foreign_item: &ForeignItem) -> &[Attribute] {
    match foreign_item {
        ForeignItem::Fn(node) => &node.attrs,
        ForeignItem::Macro(node) => &node.attrs,
        ForeignItem::Static(node) => &node.attrs,
        ForeignItem::Type(node) => &node.attrs,
        _ => &[],
    }
}

/// Emit the `syn::Visit` overrides that keep a [`GateTracking`] visitor's gate
/// set in step with the traversal. Invoke once inside `impl Visit for`; the
/// visitor's own overrides for finer-grained nodes (`visit_item_fn`,
/// `visit_expr_path`) run underneath these and see the gates already pushed.
///
/// `visit_stmt` deliberately reports nothing for `Stmt::Expr` and `Stmt::Item`:
/// those attributes belong to the expression and the item, which `visit_expr`
/// and `visit_item` push in turn. Pushing here as well would render the same
/// `#[cfg]` twice on a synthesized import.
macro_rules! gate_tracking_visit {
    () => {
        fn visit_item(&mut self, node: &syn::Item) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(
                self,
                crate::fixes::imports::item_gate_attributes(node),
            );
            syn::visit::visit_item(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_impl_item(&mut self, node: &syn::ImplItem) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(
                self,
                crate::fixes::imports::impl_item_attributes(node),
            );
            syn::visit::visit_impl_item(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_trait_item(&mut self, node: &syn::TraitItem) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(
                self,
                crate::fixes::imports::trait_item_attributes(node),
            );
            syn::visit::visit_trait_item(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_foreign_item(&mut self, node: &syn::ForeignItem) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(
                self,
                crate::fixes::imports::foreign_item_attributes(node),
            );
            syn::visit::visit_foreign_item(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_stmt(&mut self, node: &syn::Stmt) {
            let attributes = match node {
                syn::Stmt::Local(local) => local.attrs.as_slice(),
                syn::Stmt::Macro(stmt_macro) => stmt_macro.attrs.as_slice(),
                syn::Stmt::Expr(..) | syn::Stmt::Item(..) => &[],
            };
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(self, attributes);
            syn::visit::visit_stmt(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_expr(&mut self, node: &syn::Expr) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(
                self,
                crate::fixes::imports::expr_attributes(node),
            );
            syn::visit::visit_expr(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_field(&mut self, node: &syn::Field) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(self, &node.attrs);
            syn::visit::visit_field(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_variant(&mut self, node: &syn::Variant) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(self, &node.attrs);
            syn::visit::visit_variant(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }

        fn visit_arm(&mut self, node: &syn::Arm) {
            let previous_gates = crate::fixes::imports::GateTracking::push_gates(self, &node.attrs);
            syn::visit::visit_arm(self, node);
            crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
        }
    };
}

// `macro_rules!` binds privately no matter what, so a `use` re-export is the only
// way to name it by path. It has to be `pub(crate)`: `imports/mod.rs` narrows it
// back to `pub(super)` for `crate::fixes`, and a re-export cannot widen. Mend
// forbids `pub(in crate::…)` on a `use` item outright — resolved paths name the
// imported target, not the alias, so it cannot measure the alias's reach.
pub(crate) use gate_tracking_visit;
