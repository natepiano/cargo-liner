use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::iter::Skip;
use std::vec::IntoIter;

use anyhow::Result;
use syn::Item;
use syn::ItemUse;
use syn::Visibility;
use syn::spanned::Spanned;

use super::offsets;
use crate::fixes::imports;
use crate::fixes::imports::ConditionalAttributes;
use crate::fixes::imports::ImportSource;
use crate::rust_syntax::PathAnchor;

pub(super) struct ScopeInfo {
    pub(super) span_start:              usize,
    pub(super) span_end:                usize,
    pub(super) insertion_offset:        usize,
    pub(super) indent:                  String,
    pub(super) module_path:             Vec<String>,
    /// Import path of each existing `use` binding, mapped to the cfg gates of
    /// every `use` item that binds it.
    pub(super) existing_imports:        BTreeMap<String, BTreeSet<ConditionalAttributes>>,
    private_imports:                    BTreeMap<String, BTreeSet<String>>,
    import_attributes:                  BTreeMap<String, BTreeSet<ConditionalAttributes>>,
    /// Name bound by each existing `pub use` (or other re-export), mapped to
    /// the cfg gates of every re-export that binds it.
    pub(super) existing_reexport_names: BTreeMap<String, BTreeSet<ConditionalAttributes>>,
}

impl ScopeInfo {
    pub(super) fn has_private_import_conflict(&self, name: &str, path: &str) -> bool {
        self.private_imports
            .get(name)
            .is_some_and(|paths| paths.iter().any(|existing| existing != path))
    }

    /// True when the only existing bindings for this path or name are cfg-gated
    /// behind attributes the occurrence does not share — e.g. a
    /// `#[cfg(test)] use` while the occurrence compiles in non-test builds.
    /// Rewriting the qualified path would then reference an import that is
    /// configured out, and inserting an ungated `use` would collide with the
    /// gated one, so such occurrences must be left fully qualified.
    pub(super) fn existing_binding_gated_out(
        &self,
        import_path: &str,
        import_name: &str,
        occurrence_attributes: &ConditionalAttributes,
    ) -> bool {
        binding_gated_out(
            self.existing_imports.get(import_path),
            occurrence_attributes,
        ) || binding_gated_out(
            self.existing_reexport_names.get(import_name),
            occurrence_attributes,
        )
    }

    fn import_attributes(&self, name: &str) -> Option<ConditionalAttributes> {
        let attributes = self.import_attributes.get(name)?;
        if attributes.contains(&ConditionalAttributes::default()) {
            return Some(ConditionalAttributes::default());
        }
        (attributes.len() == 1)
            .then(|| attributes.first().cloned())
            .flatten()
    }
}

pub(super) struct ParentImport {
    pub(super) path:                   String,
    pub(super) conditional_attributes: ConditionalAttributes,
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
    pub(super) text:   &'a str,
    pub(super) scopes: &'a mut Vec<ScopeInfo>,
}

#[derive(Default)]
struct ScopeImports {
    existing_imports:        BTreeMap<String, BTreeSet<ConditionalAttributes>>,
    private_imports:         BTreeMap<String, BTreeSet<String>>,
    import_attributes:       BTreeMap<String, BTreeSet<ConditionalAttributes>>,
    existing_reexport_names: BTreeMap<String, BTreeSet<ConditionalAttributes>>,
}

impl ScopeImports {
    fn record(&mut self, source: &ImportSource<'_>, item_use: &ItemUse) {
        let attributes = ConditionalAttributes::from_attributes(source, &item_use.attrs);
        for binding in imports::collect_use_bindings(&item_use.tree) {
            let Some(name) = binding.name() else {
                continue;
            };
            self.import_attributes
                .entry(name.to_string())
                .or_default()
                .insert(attributes.clone());
            if binding.binds_path_leaf() {
                self.existing_imports
                    .entry(binding.path().to_string())
                    .or_default()
                    .insert(attributes.clone());
            }
            match &item_use.vis {
                Visibility::Inherited => {
                    self.private_imports
                        .entry(name.to_string())
                        .or_default()
                        .insert(binding.path().to_string());
                },
                _ => {
                    self.existing_reexport_names
                        .entry(name.to_string())
                        .or_default()
                        .insert(attributes.clone());
                },
            }
        }
    }
}

pub(super) fn collect_scopes(
    items: &[Item],
    span: ScopeSpan,
    module_path: &[String],
    scope_collection_context: &mut ScopeCollectionContext<'_>,
    source: &ImportSource<'_>,
) -> Result<()> {
    let mut scope_imports = ScopeImports::default();
    let mut last_use_start = None;
    let mut last_use_end = None;
    let mut first_item_start = None;

    for item in items {
        let item_start = offsets::offset(source, item.span().start())?;
        first_item_start.get_or_insert(item_start);

        if let Item::Use(item_use) = item {
            scope_imports.record(source, item_use);
            last_use_start = Some(item_start);
            let item_end = offsets::offset(source, item_use.span().end())?;
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
        existing_imports: scope_imports.existing_imports,
        private_imports: scope_imports.private_imports,
        import_attributes: scope_imports.import_attributes,
        existing_reexport_names: scope_imports.existing_reexport_names,
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
                    offsets::offset(source, item_mod.span().start())?,
                    offsets::offset(source, item_mod.span().end())?,
                ),
                &child_module_path,
                scope_collection_context,
                source,
            )?;
        }
    }
    Ok(())
}

pub(super) fn find_innermost_scope(scopes: &[ScopeInfo], byte_offset: usize) -> Option<usize> {
    scopes
        .iter()
        .enumerate()
        .filter(|(_, scope)| scope.span_start <= byte_offset && byte_offset < scope.span_end)
        .max_by_key(|(_, scope)| (scope.span_start, Reverse(scope.span_end)))
        .map(|(scope_id, _)| scope_id)
}

pub(super) fn qualify_through_parent_scope(
    scopes: &[ScopeInfo],
    scope_id: usize,
    import_path: &str,
) -> Option<ParentImport> {
    let leading = import_path.split("::").next()?;
    if PathAnchor::from(leading) != PathAnchor::Name {
        return None;
    }

    let current = &scopes[scope_id];
    let (parent, conditional_attributes) = scopes
        .iter()
        .filter_map(|scope| {
            if scope.module_path.len() >= current.module_path.len()
                || !current.module_path.starts_with(&scope.module_path)
            {
                return None;
            }
            scope
                .import_attributes(leading)
                .map(|attributes| (scope, attributes))
        })
        .max_by_key(|(scope, _)| scope.module_path.len())?;
    let levels = current.module_path.len() - parent.module_path.len();
    Some(ParentImport {
        path: format!("{}{import_path}", "super::".repeat(levels)),
        conditional_attributes,
    })
}

/// A binding is gated out when it exists but none of its `use` items are
/// active in every configuration the occurrence compiles in.
fn binding_gated_out(
    attribute_sets: Option<&BTreeSet<ConditionalAttributes>>,
    occurrence_attributes: &ConditionalAttributes,
) -> bool {
    attribute_sets.is_some_and(|sets| {
        !sets
            .iter()
            .any(|attributes| attributes.is_subset_of(occurrence_attributes))
    })
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

/// Classification of a relative import at the module-depth boundary.
/// One-level paths keep their spelling; paths escaping the available ancestry
/// cannot be converted to an absolute crate path and also keep their spelling.
enum LocalImportRelation<'a> {
    AsWritten(&'a str),
    OutsideAncestry(&'a str),
    Ancestor {
        module_prefix: &'a [String],
        suffix:        Skip<IntoIter<&'a str>>,
    },
}

impl<'a> LocalImportRelation<'a> {
    fn new(scope: &'a ScopeInfo, full_path: &'a str) -> Self {
        let segments: Vec<&str> = full_path.split("::").collect();
        let super_count = segments
            .iter()
            .take_while(|segment| PathAnchor::from(**segment) == PathAnchor::Super)
            .count();
        if super_count < 2 {
            return Self::AsWritten(full_path);
        }
        let Some(prefix_len) = scope.module_path.len().checked_sub(super_count) else {
            return Self::OutsideAncestry(full_path);
        };
        Self::Ancestor {
            module_prefix: &scope.module_path[..prefix_len],
            suffix:        segments.into_iter().skip(super_count),
        }
    }

    fn render(self) -> String {
        match self {
            Self::AsWritten(path) | Self::OutsideAncestry(path) => path.to_string(),
            Self::Ancestor {
                module_prefix,
                suffix,
            } => {
                let mut absolute_segments =
                    Vec::with_capacity(1 + module_prefix.len() + suffix.len());
                absolute_segments.push("crate".to_string());
                absolute_segments.extend(module_prefix.iter().cloned());
                absolute_segments.extend(suffix.map(str::to_string));
                absolute_segments.join("::")
            },
        }
    }
}

pub(super) fn canonicalize_inserted_use_path(scope: &ScopeInfo, full_path: &str) -> String {
    LocalImportRelation::new(scope, full_path).render()
}

#[cfg(test)]
mod tests {
    use crate::fixes::imports::ImportSource;

    fn scopes(text: &str) -> anyhow::Result<Vec<super::ScopeInfo>> {
        let syntax = syn::parse_file(text)?;
        let source = ImportSource::new(text);
        let mut scopes = Vec::new();
        let mut context = super::ScopeCollectionContext {
            text,
            scopes: &mut scopes,
        };
        super::collect_scopes(
            &syntax.items,
            super::ScopeSpan::new(0, text.len()),
            &[],
            &mut context,
            &source,
        )?;
        Ok(scopes)
    }

    #[test]
    fn import_relations_name_unchanged_valid_and_escaping_paths() -> anyhow::Result<()> {
        let scopes = scopes("mod a { mod b { mod c {} } }")?;
        let deepest = &scopes[3];
        assert!(matches!(
            super::LocalImportRelation::new(deepest, "super::Thing"),
            super::LocalImportRelation::AsWritten("super::Thing")
        ));
        assert!(matches!(
            super::LocalImportRelation::new(deepest, "super::super::Thing"),
            super::LocalImportRelation::Ancestor { .. }
        ));
        assert!(matches!(
            super::LocalImportRelation::new(deepest, "super::super::super::super::Thing"),
            super::LocalImportRelation::OutsideAncestry(_)
        ));
        for (input, expected) in [
            ("super::Thing", "super::Thing"),
            ("super::super::Thing", "crate::a::Thing"),
            ("super::super::super::Thing", "crate::Thing"),
            (
                "super::super::super::super::Thing",
                "super::super::super::super::Thing",
            ),
        ] {
            assert_eq!(
                super::canonicalize_inserted_use_path(deepest, input),
                expected
            );
        }
        Ok(())
    }

    #[test]
    fn collection_rejects_syntax_from_a_different_source() -> anyhow::Result<()> {
        let syntax = syn::parse_file("\nmod a {}")?;
        let source = ImportSource::new("");
        let mut scopes = Vec::new();
        let mut context = super::ScopeCollectionContext {
            text:   "",
            scopes: &mut scopes,
        };
        assert!(
            super::collect_scopes(
                &syntax.items,
                super::ScopeSpan::new(0, 0),
                &[],
                &mut context,
                &source,
            )
            .is_err()
        );
        Ok(())
    }
}
