mod apply;
mod conditional_attributes;
mod gate_tracking;
mod import_scan;
mod path;
mod scan;
mod use_binding;

pub(super) use apply::apply_fixes;
pub(super) use conditional_attributes::ConditionalAttributes;
pub(super) use conditional_attributes::expr_attributes;
pub(super) use conditional_attributes::is_conditional;
pub(super) use conditional_attributes::item_attributes;
pub(super) use gate_tracking::GateSource;
pub(super) use gate_tracking::GateTracking;
pub(super) use gate_tracking::foreign_item_attributes;
pub(super) use gate_tracking::gate_tracking_visit;
pub(super) use gate_tracking::impl_item_attributes;
pub(super) use gate_tracking::item_gate_attributes;
pub(super) use gate_tracking::trait_item_attributes;
pub(super) use import_scan::ImportGroup;
pub(super) use import_scan::ImportScan;
pub(super) use import_scan::TaggedFix;
pub(super) use import_scan::UseFix;
pub(super) use import_scan::ValidatedFixSet;
pub(super) use scan::scan_selection;
use syn::UseTree;
pub(super) use use_binding::UseBinding;

pub(super) fn collect_use_bindings(tree: &UseTree) -> Vec<UseBinding> {
    use_binding::collect_use_bindings(tree)
}
