use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::str::FromStr;

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::parse_file;
use syn::visit::Visit;
use syn::visit::visit_item_mod;

use super::function_imports::RawCandidate;
use super::support;
use crate::rust_syntax::FileModulePath;
use crate::rust_syntax::ModuleMap;

/// Drop candidates a descendant file module reaches through a glob import.
///
/// `use super::*;` in `tests.rs` makes the parent file's
/// `use super::reset_scene::build_recipe;` binding visible there, so a bare
/// `build_recipe()` in `tests.rs` names it. The rewrite to
/// `use super::reset_scene;` changes only the references in the scanned file,
/// so that call stops resolving (E0425) and the whole run is rolled back.
/// Leave such imports untouched.
///
/// Inline modules need no check: the reference collector visits them with the
/// rest of the file and rewrites their references along with the import.
pub(super) fn drop_candidates_reached_by_descendant_globs(
    module_map: &ModuleMap,
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
    let descendants: Vec<DescendantFile> = module_map
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
            let text = fs::read_to_string(file).ok()?;
            if !names.iter().any(|name| text.contains(name.as_str())) {
                return None;
            }
            DescendantFile::parse(module_path, &text)
        })
        .collect();
    if descendants.is_empty() {
        return;
    }

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

/// A file module below the scanned file: its glob imports, and every
/// identifier its tokens contain (macro bodies included).
struct DescendantFile {
    globs:       Vec<GlobImport>,
    identifiers: BTreeSet<String>,
}

impl DescendantFile {
    fn parse(module_path: Vec<String>, text: &str) -> Option<Self> {
        let syntax = parse_file(text).ok()?;
        let mut collector = GlobCollector {
            module_path,
            globs: Vec::new(),
        };
        Visit::visit_file(&mut collector, &syntax);
        if collector.globs.is_empty() {
            return None;
        }
        let mut identifiers = BTreeSet::new();
        collect_identifiers(TokenStream::from_str(text).ok()?, &mut identifiers);
        Some(Self {
            globs: collector.globs,
            identifiers,
        })
    }

    /// Whether a glob here imports from `binding_module`, or from a module
    /// between it and the glob that may re-export its names through a glob of
    /// its own, and the file names `name`.
    fn reaches(&self, binding_module: &[String], name: &str) -> bool {
        self.identifiers.contains(name)
            && self.globs.iter().any(|glob| {
                glob.target.starts_with(binding_module)
                    && glob.importing_module.len() > glob.target.len()
                    && glob.importing_module.starts_with(&glob.target)
            })
    }
}

/// A glob `use`: the module it is written in and the module whose names it
/// imports.
struct GlobImport {
    importing_module: Vec<String>,
    target:           Vec<String>,
}

struct GlobCollector {
    module_path: Vec<String>,
    globs:       Vec<GlobImport>,
}

impl GlobCollector {
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
            UseTree::Glob(_) => {
                if let Some(target) = support::resolve_to_absolute(prefix, &self.module_path) {
                    self.globs.push(GlobImport {
                        importing_module: self.module_path.clone(),
                        target,
                    });
                }
            },
            UseTree::Name(_) | UseTree::Rename(_) => {},
        }
    }
}

impl Visit<'_> for GlobCollector {
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
