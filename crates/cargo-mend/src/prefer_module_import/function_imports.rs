use std::collections::BTreeSet;
use std::path::Path;

use syn::ItemUse;
use syn::spanned::Spanned;
use syn::visit::Visit;

use super::shared;

pub(super) struct RawCandidate {
    pub(super) function_name:    String,
    pub(super) module_name:      String,
    pub(super) module_path:      String,
    pub(super) absolute_module:  Vec<String>,
    pub(super) replacement_use:  String,
    pub(super) span_start:       proc_macro2::LineColumn,
    pub(super) span_end:         proc_macro2::LineColumn,
    /// True when the target module is the file's own parent module.
    /// The use statement should be deleted and references rewritten as `super::fn(...)`.
    pub(super) is_parent_module: bool,
}

pub(super) struct ImportDetector<'a> {
    pub(super) source_root:         &'a Path,
    pub(super) current_module_path: &'a [String],
    pub(super) declared_modules:    &'a BTreeSet<String>,
    pub(super) candidates:          Vec<RawCandidate>,
}

impl Visit<'_> for ImportDetector<'_> {
    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(candidate) = analyze_function_import(
            self.source_root,
            self.current_module_path,
            self.declared_modules,
            node,
        ) {
            self.candidates.push(candidate);
        }
    }
}

fn analyze_function_import(
    source_root: &Path,
    current_module_path: &[String],
    declared_modules: &BTreeSet<String>,
    node: &ItemUse,
) -> Option<RawCandidate> {
    let flat = shared::flatten_use_tree(&node.tree)?;

    if flat.rename.is_some() {
        return None;
    }

    let first = flat.segments.first()?;
    if first != "crate" && first != "super" {
        return None;
    }

    if flat.segments.len() < 3 {
        return None;
    }

    let leaf = flat.segments.last()?;
    if !shared::is_snake_case_function_name(leaf) {
        return None;
    }

    let absolute_segments = shared::resolve_to_absolute(&flat.segments, current_module_path)?;
    if shared::leaf_is_module(source_root, &absolute_segments) {
        return None;
    }

    let module_segments = &flat.segments[..flat.segments.len() - 1];
    let module_name = flat.segments[flat.segments.len() - 2].clone();
    if module_name == "super" || module_name == "crate" {
        return None;
    }
    if !shared::is_snake_case_module_name(&module_name) {
        return None;
    }
    if declared_modules.contains(&module_name) {
        return None;
    }

    let shortened_module_segments =
        shared::shorten_module_path(current_module_path, module_segments);
    let is_parent_module = shortened_module_segments.as_slice() == ["super"];
    let module_path = shortened_module_segments.join("::");
    let replacement_use = if is_parent_module {
        String::new()
    } else {
        let vis_prefix = shared::extract_visibility_prefix(node);
        format!("{vis_prefix}use {module_path};")
    };
    let span = node.span();
    let absolute_module = absolute_segments[..absolute_segments.len() - 1].to_vec();

    Some(RawCandidate {
        function_name: leaf.clone(),
        module_name,
        module_path,
        absolute_module,
        replacement_use,
        span_start: span.start(),
        span_end: span.end(),
        is_parent_module,
    })
}
