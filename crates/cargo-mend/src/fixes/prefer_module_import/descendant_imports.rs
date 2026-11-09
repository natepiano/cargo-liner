use std::collections::BTreeMap;
use std::collections::BTreeSet;

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::File;
use syn::ItemMod;
use syn::ItemUse;
use syn::Macro;
use syn::Path;
use syn::UseTree;
use syn::visit::Visit;
use syn::visit::visit_item_mod;
use syn::visit::visit_path;

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
/// by name, as in `use super::spawn;`, or call `super::build_recipe()` directly.
/// The rewrite to `use super::reset_scene;` does not change these qualified paths.
/// The child's name then stops resolving (E0425), and mend rolls back the whole run.
/// Keep such imports.
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
    let mut inline = ModuleBindingAccesses::new(current_module_path.to_vec(), syntax);
    // ReferenceCollector visits inline modules with the rest of the file and
    // rewrites their glob users. Named imports and qualified paths need the parent binding.
    inline
        .accesses
        .retain(|access| matches!(&access.binding, AccessedBinding::Named(_)));
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
                Some(ModuleBindingAccesses::new(
                    module_path,
                    source.syntax().ok()?,
                ))
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

/// Imports, qualified paths, and identifiers, including identifiers in macro bodies.
struct ModuleBindingAccesses {
    accesses:    Vec<DescendantBindingAccess>,
    identifiers: BTreeSet<String>,
}

impl ModuleBindingAccesses {
    fn new(module_path: Vec<String>, syntax: &File) -> Self {
        let mut collector = DescendantAccessCollector {
            module_path,
            accesses: Vec::new(),
        };
        Visit::visit_file(&mut collector, syntax);
        let mut identifiers = BTreeSet::new();
        collect_identifiers(syntax.to_token_stream(), &mut identifiers);
        Self {
            accesses: collector.accesses,
            identifiers,
        }
    }

    fn reaches(&self, binding_module: &[String], name: &str) -> bool {
        self.identifiers.contains(name)
            && self
                .accesses
                .iter()
                .any(|access| access.reaches(binding_module, name))
    }
}

enum AccessedBinding {
    Glob,
    Named(String),
}

struct DescendantBindingAccess {
    accessing_module: Vec<String>,
    target_module:    Vec<String>,
    binding:          AccessedBinding,
}

impl DescendantBindingAccess {
    fn reaches(&self, binding_module: &[String], name: &str) -> bool {
        if self.accessing_module.len() <= self.target_module.len()
            || !self.accessing_module.starts_with(&self.target_module)
        {
            return false;
        }
        match &self.binding {
            AccessedBinding::Glob => self.target_module.starts_with(binding_module),
            AccessedBinding::Named(accessed) => {
                self.target_module == binding_module && accessed == name
            },
        }
    }
}

struct DescendantAccessCollector {
    module_path: Vec<String>,
    accesses:    Vec<DescendantBindingAccess>,
}

impl DescendantAccessCollector {
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
            UseTree::Glob(_) => self.record(prefix, AccessedBinding::Glob),
            UseTree::Name(name) => {
                self.record(prefix, AccessedBinding::Named(name.ident.to_string()));
            },
            UseTree::Rename(rename) => {
                self.record(prefix, AccessedBinding::Named(rename.ident.to_string()));
            },
        }
    }

    fn record(&mut self, prefix: &[String], binding: AccessedBinding) {
        if let Some(target) = support::resolve_to_absolute(prefix, &self.module_path) {
            self.accesses.push(DescendantBindingAccess {
                accessing_module: self.module_path.clone(),
                target_module: target,
                binding,
            });
        }
    }

    fn record_path(&mut self, segments: &[String]) {
        if let Some((name, prefix)) = segments.split_last() {
            self.record(prefix, AccessedBinding::Named(name.clone()));
        }
    }

    fn collect_macro_paths(&mut self, tokens: TokenStream) {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        let mut remaining = tokens.as_slice();
        while let Some((token, tail)) = remaining.split_first() {
            remaining = tail;
            match token {
                TokenTree::Group(group) => self.collect_macro_paths(group.stream()),
                TokenTree::Ident(first) => {
                    let mut segments = vec![first.to_string()];
                    while let [
                        TokenTree::Punct(first_colon),
                        TokenTree::Punct(second_colon),
                        TokenTree::Ident(segment),
                        tail @ ..,
                    ] = remaining
                    {
                        if first_colon.as_char() != ':' || second_colon.as_char() != ':' {
                            break;
                        }
                        segments.push(segment.to_string());
                        remaining = tail;
                    }
                    if segments.len() > 1 {
                        self.record_path(&segments);
                    }
                },
                TokenTree::Punct(_) | TokenTree::Literal(_) => {},
            }
        }
    }
}

impl Visit<'_> for DescendantAccessCollector {
    fn visit_item_mod(&mut self, node: &ItemMod) {
        if node.content.is_some() {
            self.module_path.push(node.ident.to_string());
            visit_item_mod(self, node);
            self.module_path.pop();
        }
    }

    fn visit_item_use(&mut self, node: &ItemUse) { self.collect(&node.tree, &mut Vec::new()); }

    fn visit_macro(&mut self, node: &Macro) {
        self.visit_path(&node.path);
        self.collect_macro_paths(node.tokens.clone());
    }

    fn visit_path(&mut self, node: &Path) {
        let segments: Vec<String> = node
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        self.record_path(&segments);
        visit_path(self, node);
    }
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
