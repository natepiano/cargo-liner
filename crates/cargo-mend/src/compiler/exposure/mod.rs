mod detect;
mod visitor;

pub(super) use detect::child_item_is_exposed_by_other_crate_visible_signature;
pub(super) use detect::child_item_is_exposed_by_sibling_boundary_signature;
pub(super) use detect::impl_item_is_exposed_by_exported_self_type;
pub(super) use detect::parent_boundary_public_signature_exposes_child_used_outside_parent;
