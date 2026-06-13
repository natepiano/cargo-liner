use std::cmp::Reverse;
use std::collections::BTreeSet;

use syn::Item;
use syn::Visibility;
use syn::spanned::Spanned;

use super::offsets;
use super::visitor;
use crate::rust_syntax::PathAnchor;

pub(super) struct ScopeInfo {
    pub(super) span_start:              usize,
    pub(super) span_end:                usize,
    pub(super) insertion_offset:        usize,
    pub(super) indent:                  String,
    pub(super) module_path:             Vec<String>,
    pub(super) existing_imports:        BTreeSet<String>,
    pub(super) existing_reexport_names: BTreeSet<String>,
}

#[derive(Clone, Copy)]
pub(super) struct ScopeSpan {
    start: usize,
    end:   usize,
}

impl ScopeSpan {
    pub(super) const fn new(start: usize, end: usize) -> Self { Self { start, end } }
}

pub(super) struct ScopeCollectionContext<'a> {
    pub(super) text:    &'a str,
    pub(super) offsets: &'a [usize],
    pub(super) scopes:  &'a mut Vec<ScopeInfo>,
}

pub(super) fn collect_scopes(
    items: &[Item],
    span: ScopeSpan,
    module_path: &[String],
    scope_collection_context: &mut ScopeCollectionContext<'_>,
) {
    let mut existing_imports = BTreeSet::new();
    let mut existing_reexport_names = BTreeSet::new();
    let mut last_use_start = None;
    let mut last_use_end = None;
    let mut first_item_start = None;

    for item in items {
        let item_start = offsets::offset(
            scope_collection_context.text,
            scope_collection_context.offsets,
            item.span().start(),
        );
        first_item_start.get_or_insert(item_start);

        if let Item::Use(item_use) = item {
            if let Some(import_path) = visitor::flatten_use_path(&item_use.tree) {
                if !matches!(item_use.vis, Visibility::Inherited)
                    && let Some(import_name) = import_path.rsplit("::").next()
                {
                    existing_reexport_names.insert(import_name.to_string());
                }
                existing_imports.insert(import_path);
            }
            last_use_start = Some(item_start);
            let item_end = offsets::offset(
                scope_collection_context.text,
                scope_collection_context.offsets,
                item_use.span().end(),
            );
            last_use_end = Some(
                if scope_collection_context.text.as_bytes().get(item_end) == Some(&b'\n') {
                    item_end + 1
                } else {
                    item_end
                },
            );
        }
    }

    let anchor_offset = last_use_start.or(first_item_start).unwrap_or(span.start);
    let insertion_offset = last_use_end.or(first_item_start).unwrap_or(span.end);
    let indent = indentation_at(scope_collection_context.text, anchor_offset);
    scope_collection_context.scopes.push(ScopeInfo {
        span_start: span.start,
        span_end: span.end,
        insertion_offset,
        indent,
        module_path: module_path.to_vec(),
        existing_imports,
        existing_reexport_names,
    });

    for item in items {
        if let Item::Mod(item_mod) = item
            && let Some((_, child_items)) = &item_mod.content
        {
            let mut child_module_path = module_path.to_vec();
            child_module_path.push(item_mod.ident.to_string());
            collect_scopes(
                child_items,
                ScopeSpan::new(
                    offsets::offset(
                        scope_collection_context.text,
                        scope_collection_context.offsets,
                        item_mod.span().start(),
                    ),
                    offsets::offset(
                        scope_collection_context.text,
                        scope_collection_context.offsets,
                        item_mod.span().end(),
                    ),
                ),
                &child_module_path,
                scope_collection_context,
            );
        }
    }
}

pub(super) fn find_innermost_scope(scopes: &[ScopeInfo], byte_offset: usize) -> Option<usize> {
    scopes
        .iter()
        .enumerate()
        .filter(|(_, scope)| scope.span_start <= byte_offset && byte_offset < scope.span_end)
        .max_by_key(|(_, scope)| (scope.span_start, Reverse(scope.span_end)))
        .map(|(scope_id, _)| scope_id)
}

fn indentation_at(text: &str, byte_offset: usize) -> String {
    let line_start = text[..byte_offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    text[line_start..byte_offset]
        .chars()
        .take_while(char::is_ascii_whitespace)
        .collect()
}

pub(super) fn canonicalize_inserted_use_path(scope: &ScopeInfo, full_path: &str) -> String {
    let segments: Vec<&str> = full_path.split("::").collect();
    let super_count = segments
        .iter()
        .take_while(|segment| PathAnchor::from(**segment) == PathAnchor::Super)
        .count();
    if super_count < 2 || super_count > scope.module_path.len() {
        return full_path.to_string();
    }

    let mut absolute_segments = Vec::with_capacity(1 + scope.module_path.len() + segments.len());
    absolute_segments.push("crate".to_string());
    absolute_segments.extend(
        scope.module_path[..scope.module_path.len() - super_count]
            .iter()
            .cloned(),
    );
    absolute_segments.extend(
        segments[super_count..]
            .iter()
            .map(|segment| (*segment).to_string()),
    );
    absolute_segments.join("::")
}
