use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::LineColumn;
use syn::ItemMod;
use syn::ItemUse;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::visit::visit_item_mod;

use super::support;
use crate::rust_syntax::PathAnchor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportTarget {
    ParentModule,
    OtherModule,
}

pub(super) struct RawCandidate {
    pub(super) function_name:   String,
    pub(super) module_name:     String,
    pub(super) module_path:     String,
    pub(super) absolute_module: Vec<String>,
    pub(super) replacement_use: String,
    pub(super) span_start:      LineColumn,
    pub(super) span_end:        LineColumn,
    /// True when the target module is the file's own parent module.
    /// The use statement should be deleted and references rewritten as `super::fn(...)`.
    pub(super) import_target:   ImportTarget,
    /// Inline `mod` chain containing the `use` — empty at file top level.
    /// An import inside `mod tests` binds nothing at file top level (and vice
    /// versa), so dedup and reuse decisions must compare scopes.
    pub(super) inline_scope:    Vec<String>,
}

pub(super) struct ImportDetector<'a> {
    pub(super) source_root:         &'a Path,
    pub(super) current_module_path: Vec<String>,
    pub(super) inline_scope:        Vec<String>,
    pub(super) declared_modules:    &'a BTreeSet<String>,
    pub(super) candidates:          Vec<RawCandidate>,
}

impl Visit<'_> for ImportDetector<'_> {
    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(candidate) = analyze_function_import(
            self.source_root,
            &self.current_module_path,
            &self.inline_scope,
            self.declared_modules,
            node,
        ) {
            self.candidates.push(candidate);
        }
    }

    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.current_module_path.push(node.ident.to_string());
            self.inline_scope.push(node.ident.to_string());
            visit_item_mod(self, node);
            self.inline_scope.pop();
            self.current_module_path.pop();
        } else {
            visit_item_mod(self, node);
        }
    }
}

fn analyze_function_import(
    source_root: &Path,
    current_module_path: &[String],
    inline_scope: &[String],
    declared_modules: &BTreeSet<String>,
    node: &ItemUse,
) -> Option<RawCandidate> {
    let flat = support::flatten_use_tree(&node.tree)?;

    if flat.rename.is_some() {
        return None;
    }

    let path_anchor = PathAnchor::first(&flat.segments)?;
    if !path_anchor.is_crate_relative() {
        return None;
    }

    if flat.segments.len() < 3 {
        return None;
    }

    let leaf = flat.segments.last()?;
    if !support::is_snake_case_function_name(leaf) {
        return None;
    }

    let absolute_segments = support::resolve_to_absolute(&flat.segments, current_module_path)?;
    if support::leaf_is_module(source_root, &absolute_segments) {
        return None;
    }

    let module_segments = &flat.segments[..flat.segments.len() - 1];
    let module_name = flat.segments[flat.segments.len() - 2].clone();
    if PathAnchor::from(module_name.as_str()).is_crate_relative() {
        return None;
    }
    if !support::is_snake_case_module_name(&module_name) {
        return None;
    }
    if declared_modules.contains(&module_name) {
        return None;
    }

    let shortened_module_segments =
        support::shorten_module_path(current_module_path, module_segments);
    let import_target = match shortened_module_segments.as_slice() {
        [segment] if PathAnchor::from(segment.as_str()) == PathAnchor::Super => {
            ImportTarget::ParentModule
        },
        _ => ImportTarget::OtherModule,
    };
    let module_path = shortened_module_segments.join("::");
    let replacement_use = if import_target == ImportTarget::ParentModule {
        String::new()
    } else {
        let visibility_prefix = support::extract_visibility_prefix(node);
        format!("{visibility_prefix}use {module_path};")
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
        import_target,
        inline_scope: inline_scope.to_vec(),
    })
}
