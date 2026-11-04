use std::collections::BTreeMap;
use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::File;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::visit::Visit;
use syn::visit::visit_item_mod;

use super::function_imports::RawCandidate;
use super::support;
use crate::rust_syntax::FileModulePath;
use crate::rust_syntax::ModuleMap;
use crate::rust_syntax::ParsedSources;

/// Drop candidates that a descendant module reaches by name or through a glob.
///
/// `use super::*;` in `tests.rs` makes the parent's
/// `use super::reset_scene::build_recipe;` binding visible there, so a bare
/// `build_recipe()` in `tests.rs` names it. A child can also import the binding
/// by name, as in `use super::spawn;`. The rewrite to `use super::reset_scene;`
/// changes only the references in the scanned file. The child's name then stops
/// resolving (E0425), and mend rolls back the whole run. Keep such imports.
pub(super) fn drop_candidates_reached_by_descendants(
    syntax: &File,
    module_map: &ModuleMap,
    sources: &ParsedSources,
    current_module_path: &[String],
    module_to_functions: &mut BTreeMap<String, Vec<RawCandidate>>,
) {
    let names: BTreeSet<String> = module_to_functions
        .values()
        .flatten()
        .map(|candidate| candidate.function_name.clone())
        .collect();
    if names.is_empty() {
        return;
    }
    let mut inline = ImportConsumer::new(current_module_path.to_vec(), syntax);
    // The reference collector visits inline modules with the rest of the file and
    // rewrites their glob users. Only a named import in an inline module can break.
    inline
        .imports
        .retain(|import| matches!(&import.binding, ImportedBinding::Named(_)));
    let mut descendants = vec![inline];
    descendants.extend(
        module_map
            .declared_files()
            .filter_map(|(file, module_path)| {
                let FileModulePath::Known(module_path) = module_path else {
                    return None;
                };
                if module_path.len() <= current_module_path.len()
                    || !module_path.starts_with(current_module_path)
                {
                    return None;
                }
                let source = sources.source(file).ok()?;
                let text = source.text();
                if !names.iter().any(|name| text.contains(name.as_str())) {
                    return None;
                }
                Some(ImportConsumer::new(module_path, source.syntax().ok()?))
            }),
    );

    module_to_functions.retain(|_, functions| {
        functions.retain(|candidate| {
            let mut binding_module = current_module_path.to_vec();
            binding_module.extend(candidate.inline_scope.iter().cloned());
            !descendants
                .iter()
                .any(|descendant| descendant.reaches(&binding_module, &candidate.function_name))
        });
        !functions.is_empty()
    });
}

/// Imports and identifiers in a file, including identifiers in macro bodies.
struct ImportConsumer {
    imports:     Vec<DescendantImport>,
    identifiers: BTreeSet<String>,
}

impl ImportConsumer {
    fn new(module_path: Vec<String>, syntax: &File) -> Self {
        let mut collector = ImportCollector {
            module_path,
            imports: Vec::new(),
        };
        Visit::visit_file(&mut collector, syntax);
        let mut identifiers = BTreeSet::new();
        collect_identifiers(syntax.to_token_stream(), &mut identifiers);
        Self {
            imports: collector.imports,
            identifiers,
        }
    }

    fn reaches(&self, binding_module: &[String], name: &str) -> bool {
        self.identifiers.contains(name)
            && self
                .imports
                .iter()
                .any(|import| import.reaches(binding_module, name))
    }
}

enum ImportedBinding {
    Glob,
    Named(String),
}

struct DescendantImport {
    importing_module: Vec<String>,
    target:           Vec<String>,
    binding:          ImportedBinding,
}

impl DescendantImport {
    fn reaches(&self, binding_module: &[String], name: &str) -> bool {
        if self.importing_module.len() <= self.target.len()
            || !self.importing_module.starts_with(&self.target)
        {
            return false;
        }
        match &self.binding {
            ImportedBinding::Glob => self.target.starts_with(binding_module),
            ImportedBinding::Named(imported) => self.target == binding_module && imported == name,
        }
    }
}

struct ImportCollector {
    module_path: Vec<String>,
    imports:     Vec<DescendantImport>,
}

impl ImportCollector {
    fn collect(&mut self, tree: &UseTree, prefix: &mut Vec<String>) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.collect(&path.tree, prefix);
                prefix.pop();
            },
            UseTree::Group(group) => {
                for item in &group.items {
                    self.collect(item, prefix);
                }
            },
            UseTree::Glob(_) => self.record(prefix, ImportedBinding::Glob),
            UseTree::Name(name) => {
                self.record(prefix, ImportedBinding::Named(name.ident.to_string()));
            },
            UseTree::Rename(rename) => {
                self.record(prefix, ImportedBinding::Named(rename.ident.to_string()));
            },
        }
    }

    fn record(&mut self, prefix: &[String], binding: ImportedBinding) {
        if let Some(target) = support::resolve_to_absolute(prefix, &self.module_path) {
            self.imports.push(DescendantImport {
                importing_module: self.module_path.clone(),
                target,
                binding,
            });
        }
    }
}

impl Visit<'_> for ImportCollector {
    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.module_path.push(node.ident.to_string());
            visit_item_mod(self, node);
            self.module_path.pop();
        }
    }

    fn visit_item_use(&mut self, node: &ItemUse) { self.collect(&node.tree, &mut Vec::new()); }
}

fn collect_identifiers(tokens: TokenStream, identifiers: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(ident) => {
                identifiers.insert(ident.to_string());
            },
            TokenTree::Group(group) => collect_identifiers(group.stream(), identifiers),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {},
        }
    }
}
