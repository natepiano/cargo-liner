//! Track enclosing cfg conditions through each parsed source node.
//!
//! Visitors use one shared traversal to preserve conditions on generated imports.
//! Each override restores the outer conditions after it visits the node.

use syn::Attribute;
use syn::ForeignItem;
use syn::ImplItem;
use syn::Item;
use syn::Pat;
use syn::TraitItem;
use syn::Type;

use super::ConditionalAttributes;

/// A borrowed checked source owner. Gate extraction shares the traversal's
/// index rather than rebuilding it or trusting an independent offsets slice.
/// `ConditionalAttributes` reads the original gate bytes rather than re-rendering
/// `Meta`, so moved imports keep the predicates as written.
pub(in crate::fixes) struct GateSource<'a, 'source> {
    pub(in crate::fixes) indexed: &'a super::ImportSource<'source>,
}

/// A `syn::Visit` implementor that accumulates the `#[cfg]` gates enclosing the
/// node it is currently visiting. Implement the two accessors; get the
/// push/restore pair from the defaults and the traversal from
/// [`gate_tracking_visit`].
pub(in crate::fixes) trait GateTracking {
    fn gate_source(&self) -> GateSource<'_, '_>;

    fn conditional_attributes_mut(&mut self) -> &mut ConditionalAttributes;

    /// Add `attributes`' gates to the enclosing set and return the length to
    /// hand back to [`Self::restore_gates`] once the subtree is visited.
    fn push_gates(&mut self, attributes: &[Attribute]) -> usize {
        if attributes.is_empty() {
            return self.conditional_attributes_mut().len();
        }
        let gates = {
            let source = self.gate_source();
            ConditionalAttributes::from_attributes(source.indexed, attributes)
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

/// Attributes on patterns, including expression nodes used as patterns.
/// Function arguments reach `PatType` directly instead of passing through `Pat`.
pub(in crate::fixes) fn pattern_attributes(pattern: &Pat) -> &[Attribute] {
    match pattern {
        Pat::Const(node) => &node.attrs,
        Pat::Guard(node) => &node.attrs,
        Pat::Ident(node) => &node.attrs,
        Pat::Lit(node) => &node.attrs,
        Pat::Macro(node) => &node.attrs,
        Pat::Or(node) => &node.attrs,
        Pat::Paren(node) => &node.attrs,
        Pat::Path(node) => &node.attrs,
        Pat::Range(node) => &node.attrs,
        Pat::Reference(node) => &node.attrs,
        Pat::Rest(node) => &node.attrs,
        Pat::Slice(node) => &node.attrs,
        Pat::Struct(node) => &node.attrs,
        Pat::Tuple(node) => &node.attrs,
        Pat::TupleStruct(node) => &node.attrs,
        Pat::Type(node) => &node.attrs,
        Pat::Wild(node) => &node.attrs,
        _ => &[],
    }
}

/// Type nodes can contain expressions through array lengths and generic arguments.
/// Syn currently parses their own attribute lists as empty.
pub(in crate::fixes) fn type_attributes(ty: &Type) -> &[Attribute] {
    match ty {
        Type::Array(node) => &node.attrs,
        Type::FnPtr(node) => &node.attrs,
        Type::Group(node) => &node.attrs,
        Type::ImplTrait(node) => &node.attrs,
        Type::Infer(node) => &node.attrs,
        Type::Macro(node) => &node.attrs,
        Type::Never(node) => &node.attrs,
        Type::Paren(node) => &node.attrs,
        Type::Path(node) => &node.attrs,
        Type::Ptr(node) => &node.attrs,
        Type::Reference(node) => &node.attrs,
        Type::Slice(node) => &node.attrs,
        Type::TraitObject(node) => &node.attrs,
        Type::Tuple(node) => &node.attrs,
        _ => &[],
    }
}

/// Generate the shared gate traversal inside an implementation of `Visit`.
/// Each gate belongs to one entry point, even when syn shares concrete node types.
macro_rules! gate_tracking_visit {
    () => {
        crate::fixes::imports::gate_tracking_visit! {
            @nodes
            visit_item(Item, node) => crate::fixes::imports::item_gate_attributes(node);
            visit_impl_item(ImplItem, node) => crate::fixes::imports::impl_item_attributes(node);
            visit_trait_item(TraitItem, node) => crate::fixes::imports::trait_item_attributes(node);
            visit_foreign_item(ForeignItem, node) => crate::fixes::imports::foreign_item_attributes(node);
            visit_expr(Expr, node) => crate::fixes::imports::expr_attributes(node);
            visit_type(Type, node) => crate::fixes::imports::type_attributes(node);
            visit_pat(Pat, node) => crate::fixes::imports::pattern_attributes(node);
            visit_stmt(Stmt, node) => match node {
                syn::Stmt::Local(local) => local.attrs.as_slice(),
                syn::Stmt::Macro(stmt_macro) => stmt_macro.attrs.as_slice(),
                syn::Stmt::Expr(..) | syn::Stmt::Item(..) => &[],
            };
            visit_fn_arg(FnArg, node) => match node {
                syn::FnArg::Receiver(receiver) => receiver.attrs.as_slice(),
                syn::FnArg::Typed(typed) => typed.attrs.as_slice(),
            };
            visit_generic_param(GenericParam, node) => match node {
                syn::GenericParam::Lifetime(lifetime) => lifetime.attrs.as_slice(),
                syn::GenericParam::Type(ty) => ty.attrs.as_slice(),
                syn::GenericParam::Const(constant) => constant.attrs.as_slice(),
            };
            visit_field(Field, node) => &node.attrs;
            visit_field_value(FieldValue, node) => &node.attrs;
            visit_field_pat(FieldPat, node) => &node.attrs;
            visit_variant(Variant, node) => &node.attrs;
            visit_arm(Arm, node) => &node.attrs;
            visit_named_arg(NamedArg, node) => &node.attrs;
            visit_variadic(Variadic, node) => &node.attrs;
            visit_predicate_type(PredicateType, node) => &node.attrs;
        }
    };
    (@nodes $($method:ident($kind:ident, $node:ident) => $attributes:expr;)+) => {
        $(
            fn $method(&mut self, $node: &syn::$kind) {
                let previous_gates = crate::fixes::imports::GateTracking::push_gates(self, $attributes);
                syn::visit::$method(self, $node);
                crate::fixes::imports::GateTracking::restore_gates(self, previous_gates);
            }
        )+
    };
}

// `gate_tracking_visit` is bound privately by `macro_rules!`, so a `use` re-export is the only
// way to name it by path. It has to be `pub(crate)`: `imports/mod.rs` narrows it
// back to `pub(super)` for `crate::fixes`, and a re-export cannot widen. Mend
// forbids `pub(in crate::…)` on a `use` item outright — resolved paths name the
// imported target, not the alias, so it cannot measure the alias's reach.
pub(crate) use gate_tracking_visit;
