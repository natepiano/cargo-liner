use std::path::PathBuf;

use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use rustc_hash::FxHashSet;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::def_id::LOCAL_CRATE;
use syn::Attribute;
use syn::Expr;
use syn::ExprLit;
use syn::Field;
use syn::ForeignItem;
use syn::ImplItem;
use syn::Item;
use syn::Lit;
use syn::Meta;
use syn::Stmt;
use syn::Token;
use syn::TraitItem;
use syn::Variant;
use syn::parse::ParseStream;
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::visit;
use syn::visit::Visit;

use super::source_cache::SourceCache;

/// Whether a name is written in source this compilation left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CfgExcludedReference {
    Present,
    Absent,
}

/// Whether one `#[cfg]` predicate keeps its region out of this compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CfgExclusion {
    Excluded,
    Included,
}

/// The value the `test` atom takes while a `#[cfg]` predicate is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestCfg {
    Enabled,
    Disabled,
}

/// Every identifier written in source that this compilation's `#[cfg]`
/// configuration left out.
///
/// `visibility::use_sites::collect_use_sites` walks `tcx.hir_crate_items`,
/// which holds only what survived `#[cfg]` expansion, so a consumer inside an
/// excluded region contributes no use site and
/// `VisibilityConstraintGroup::caller_required_reach` reads a caller set that
/// is missing that consumer's module. Rust has no `#[cfg]` on a visibility
/// qualifier, so no single spelling satisfies both configurations, and a
/// narrowing derived from the smaller caller set cannot be applied.
///
/// Names are collected, not resolved paths: an excluded region names no module
/// the compiler resolved, so the answer is a name match and nothing finer. It
/// is therefore approximate in the direction of reporting less.
///
/// Regions gated on `test` alone are not collected. `--all-targets` compiles
/// each crate a second time in test mode, and both reports feed one `CallerMap`,
/// so a `#[cfg(test)]` consumer already contributes its caller module. Feature
/// gates are collected: mend's check pass selects no features, so a
/// feature-gated consumer is as invisible as a platform-gated one.
///
/// `#[cfg]` written on an expression that is not a statement — inside a call
/// argument or an array literal — is not collected. [`Visit`] reaches such an
/// expression only through its parent, whose own tokens carry no attribute.
pub(super) struct CfgExcludedReferences {
    names: FxHashSet<String>,
}

impl CfgExcludedReferences {
    pub(super) fn collect(tcx: TyCtxt<'_>, source_cache: &SourceCache) -> Self {
        let compiled_files = compiled_files(tcx);
        let active_cfg = ActiveCfg::from(tcx);
        let mut names = FxHashSet::default();
        for source_file in source_cache.source_files() {
            let Some(parsed_file) = source_cache.parsed_file(source_file) else {
                continue;
            };
            if !compiled_files.contains(source_file) {
                collect_identifiers(&parsed_file.to_token_stream(), &mut names);
                continue;
            }
            // Tokenizing a node to read its outer attributes costs the node's
            // whole subtree, and a file with no `cfg` anywhere has no excluded
            // region to find. `read_source` returns the text the cache already
            // holds, so this test reads no file.
            if source_cache
                .read_source(source_file)
                .is_ok_and(|source| !source.contains("cfg"))
            {
                continue;
            }
            ExcludedNameCollector {
                active_cfg: &active_cfg,
                names:      &mut names,
            }
            .visit_file(parsed_file);
        }
        Self { names }
    }

    pub(super) fn reference(&self, name: &str) -> CfgExcludedReference {
        if self.names.contains(name) {
            CfgExcludedReference::Present
        } else {
            CfgExcludedReference::Absent
        }
    }
}

/// The `#[cfg]` atoms rustc set for this compilation.
struct ActiveCfg {
    atoms: FxHashSet<(String, Option<String>)>,
}

impl ActiveCfg {
    fn holds(&self, name: &str, value: Option<&str>) -> bool {
        self.atoms
            .iter()
            .any(|(atom, atom_value)| atom == name && atom_value.as_deref() == value)
    }
}

impl<'tcx> From<TyCtxt<'tcx>> for ActiveCfg {
    fn from(tcx: TyCtxt<'tcx>) -> Self {
        let atoms = tcx
            .sess
            .config
            .iter()
            .map(|&(name, value)| (name.to_string(), value.map(|value| value.to_string())))
            .collect();
        Self { atoms }
    }
}

struct ExcludedNameCollector<'a> {
    active_cfg: &'a ActiveCfg,
    names:      &'a mut FxHashSet<String>,
}

impl ExcludedNameCollector<'_> {
    /// Collects `node`'s identifiers when a `#[cfg]` on it excluded it, and
    /// otherwise hands `node` back to the default walk.
    ///
    /// An excluded node is not descended into: its whole token subtree was
    /// already collected, and nothing inside it can be included by a nested
    /// `#[cfg]`.
    fn visit_node<Node: ToTokens>(&mut self, node: &Node, descend: impl FnOnce(&mut Self, &Node)) {
        let tokens = node.to_token_stream();
        if outer_attributes(tokens.clone()).iter().any(|attribute| {
            attribute_exclusion(attribute, self.active_cfg) == CfgExclusion::Excluded
        }) {
            collect_identifiers(&tokens, self.names);
            return;
        }
        descend(self, node);
    }
}

impl<'ast> Visit<'ast> for ExcludedNameCollector<'_> {
    fn visit_item(&mut self, node: &'ast Item) {
        self.visit_node(node, |collector, node| visit::visit_item(collector, node));
    }

    fn visit_impl_item(&mut self, node: &'ast ImplItem) {
        self.visit_node(node, |collector, node| {
            visit::visit_impl_item(collector, node);
        });
    }

    fn visit_trait_item(&mut self, node: &'ast TraitItem) {
        self.visit_node(node, |collector, node| {
            visit::visit_trait_item(collector, node);
        });
    }

    fn visit_foreign_item(&mut self, node: &'ast ForeignItem) {
        self.visit_node(node, |collector, node| {
            visit::visit_foreign_item(collector, node);
        });
    }

    fn visit_field(&mut self, node: &'ast Field) {
        self.visit_node(node, |collector, node| visit::visit_field(collector, node));
    }

    fn visit_variant(&mut self, node: &'ast Variant) {
        self.visit_node(node, |collector, node| {
            visit::visit_variant(collector, node);
        });
    }

    fn visit_stmt(&mut self, node: &'ast Stmt) {
        self.visit_node(node, |collector, node| visit::visit_stmt(collector, node));
    }
}

/// The source files rustc compiled for the local crate.
///
/// A file the module tree declares but this list omits was reached through a
/// `mod` declaration a `#[cfg]` excluded, so every identifier in it belongs to
/// another configuration.
fn compiled_files(tcx: TyCtxt<'_>) -> FxHashSet<PathBuf> {
    tcx.sess
        .source_map()
        .files()
        .iter()
        .filter(|source_file| source_file.cnum == LOCAL_CRATE)
        .filter_map(|source_file| {
            let FileName::Real(real_file_name) = &source_file.name else {
                return None;
            };
            real_file_name
                .local_path()
                .map(|path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
        })
        .collect()
}

/// The outer attributes a node's own tokens open with.
///
/// `syn` gives every attribute-carrying node an `attrs` field but no trait that
/// reads it, and `Item`, `Expr`, and `Stmt` each spell dozens of variants.
/// `ToTokens` emits outer attributes ahead of the node, so re-parsing the
/// prefix answers for every node kind through one signature.
fn outer_attributes(tokens: TokenStream) -> Vec<Attribute> {
    Parser::parse2(
        |input: ParseStream<'_>| {
            let attributes = input.call(Attribute::parse_outer)?;
            input.parse::<TokenStream>()?;
            Ok(attributes)
        },
        tokens,
    )
    .unwrap_or_default()
}

fn attribute_exclusion(attribute: &Attribute, active_cfg: &ActiveCfg) -> CfgExclusion {
    if !attribute.path().is_ident("cfg") {
        return CfgExclusion::Included;
    }
    let Meta::List(list) = &attribute.meta else {
        return CfgExclusion::Included;
    };
    let Ok(predicates) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    else {
        return CfgExclusion::Included;
    };
    let Some(predicate) = predicates.first() else {
        return CfgExclusion::Included;
    };
    if predicate_names_an_atom(predicate)
        && [TestCfg::Enabled, TestCfg::Disabled]
            .into_iter()
            .all(|test_cfg| predicate_holds(predicate, active_cfg, test_cfg) == Some(false))
    {
        return CfgExclusion::Excluded;
    }
    CfgExclusion::Included
}

/// Whether `predicate` names any atom at all.
///
/// `#[cfg(any())]` is false in every compilation, so the region it guards is
/// not code another configuration compiles — it is code the author switched off
/// for good. Its references say nothing about how wide a visibility has to be.
fn predicate_names_an_atom(predicate: &Meta) -> bool {
    match predicate {
        Meta::Path(_) | Meta::NameValue(_) => true,
        Meta::List(list) => list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|nested| nested.iter().any(predicate_names_an_atom)),
    }
}

/// Whether `predicate` holds, or `None` when it uses a form this evaluator does
/// not model. An unmodelled predicate leaves its region included, which keeps
/// the reporting the same as before this check existed.
fn predicate_holds(predicate: &Meta, active_cfg: &ActiveCfg, test_cfg: TestCfg) -> Option<bool> {
    match predicate {
        Meta::Path(path) => {
            let name = path.get_ident()?.to_string();
            if name == "test" {
                return Some(test_cfg == TestCfg::Enabled);
            }
            Some(active_cfg.holds(&name, None))
        },
        Meta::NameValue(name_value) => {
            let name = name_value.path.get_ident()?.to_string();
            let Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) = &name_value.value
            else {
                return None;
            };
            Some(active_cfg.holds(&name, Some(&value.value())))
        },
        Meta::List(list) => {
            let combinator = list.path.get_ident()?.to_string();
            let nested = list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .ok()?;
            match combinator.as_str() {
                "all" => nested.iter().try_fold(true, |held, nested_predicate| {
                    Some(held && predicate_holds(nested_predicate, active_cfg, test_cfg)?)
                }),
                "any" => nested.iter().try_fold(false, |held, nested_predicate| {
                    Some(held || predicate_holds(nested_predicate, active_cfg, test_cfg)?)
                }),
                "not" if nested.len() == 1 => {
                    Some(!predicate_holds(nested.first()?, active_cfg, test_cfg)?)
                },
                _ => None,
            }
        },
    }
}

fn collect_identifiers(tokens: &TokenStream, names: &mut FxHashSet<String>) {
    for token in tokens.clone() {
        match token {
            TokenTree::Ident(identifier) => {
                names.insert(identifier.to_string());
            },
            TokenTree::Group(group) => collect_identifiers(&group.stream(), names),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {},
        }
    }
}
