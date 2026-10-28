use std::path::PathBuf;

use proc_macro2::Spacing;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CfgAtomPresence {
    Present,
    Absent,
}

/// A permanently disabled predicate such as `any()` names no configuration atom.
/// Its references do not constrain visibility in another configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomReference {
    Present,
    Absent,
}

/// Unmodelled syntax leaves its region included, preserving conservative reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PredicateEvaluation {
    Unmodelled,
    Satisfied,
    Unsatisfied,
}

impl From<CfgAtomPresence> for PredicateEvaluation {
    fn from(presence: CfgAtomPresence) -> Self {
        match presence {
            CfgAtomPresence::Present => Self::Satisfied,
            CfgAtomPresence::Absent => Self::Unsatisfied,
        }
    }
}

impl PredicateEvaluation {
    fn conjunction(self, next: impl FnOnce() -> Self) -> Self {
        match self {
            Self::Satisfied => next(),
            Self::Unsatisfied => Self::Unsatisfied,
            Self::Unmodelled => Self::Unmodelled,
        }
    }

    fn disjunction(self, next: impl FnOnce() -> Self) -> Self {
        match self {
            Self::Unsatisfied => next(),
            Self::Satisfied => Self::Satisfied,
            Self::Unmodelled => Self::Unmodelled,
        }
    }

    const fn negated(self) -> Self {
        match self {
            Self::Satisfied => Self::Unsatisfied,
            Self::Unsatisfied => Self::Satisfied,
            Self::Unmodelled => Self::Unmodelled,
        }
    }
}

#[derive(Clone, Copy)]
enum PredicateCombination {
    All,
    Any,
}

/// Every identifier and tuple index written in source that this compilation's
/// `#[cfg]` configuration left out.
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
            // `ExcludedNameCollector::visit_node` tokenizes a node's whole subtree
            // to inspect outer attributes. A file with no `cfg` has no excluded region,
            // so `CfgExcludedReferences::collect` skips that work. `SourceCache::read_source`
            // returns cached text, so this test performs no file read.
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
    fn presence(&self, name: &str, value: Option<&str>) -> CfgAtomPresence {
        if self
            .atoms
            .iter()
            .any(|(atom, atom_value)| atom == name && atom_value.as_deref() == value)
        {
            CfgAtomPresence::Present
        } else {
            CfgAtomPresence::Absent
        }
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

/// Whether the previous token is a `.`, and which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DotPosition {
    /// A lone `.`, so a literal next is a tuple index.
    FieldAccess,
    /// The first `.` of `..` or `..=`, so the `.` next ends a range operator.
    RangeStart,
    Elsewhere,
}

impl DotPosition {
    fn after(self, token: &TokenTree) -> Self {
        let TokenTree::Punct(punct) = token else {
            return Self::Elsewhere;
        };
        if punct.as_char() != '.' || self == Self::RangeStart {
            return Self::Elsewhere;
        }
        match punct.spacing() {
            Spacing::Joint => Self::RangeStart,
            Spacing::Alone => Self::FieldAccess,
        }
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
    if predicate_atom_reference(predicate) == AtomReference::Present
        && [TestCfg::Enabled, TestCfg::Disabled]
            .into_iter()
            .all(|test_cfg| {
                evaluate_predicate(predicate, active_cfg, test_cfg)
                    == PredicateEvaluation::Unsatisfied
            })
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
fn predicate_atom_reference(predicate: &Meta) -> AtomReference {
    match predicate {
        Meta::Path(_) | Meta::NameValue(_) => AtomReference::Present,
        Meta::List(list) => {
            if list
                .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
                .is_ok_and(|nested| {
                    nested.iter().any(|predicate| {
                        predicate_atom_reference(predicate) == AtomReference::Present
                    })
                })
            {
                AtomReference::Present
            } else {
                AtomReference::Absent
            }
        },
    }
}

fn evaluate_predicate(
    predicate: &Meta,
    active_cfg: &ActiveCfg,
    test_cfg: TestCfg,
) -> PredicateEvaluation {
    match predicate {
        Meta::Path(path) => {
            let Some(identifier) = path.get_ident() else {
                return PredicateEvaluation::Unmodelled;
            };
            let name = identifier.to_string();
            if name == "test" {
                return match test_cfg {
                    TestCfg::Enabled => PredicateEvaluation::Satisfied,
                    TestCfg::Disabled => PredicateEvaluation::Unsatisfied,
                };
            }
            active_cfg.presence(&name, None).into()
        },
        Meta::NameValue(name_value) => {
            let Some(identifier) = name_value.path.get_ident() else {
                return PredicateEvaluation::Unmodelled;
            };
            let name = identifier.to_string();
            let Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) = &name_value.value
            else {
                return PredicateEvaluation::Unmodelled;
            };
            active_cfg.presence(&name, Some(&value.value())).into()
        },
        Meta::List(list) => {
            let Some(identifier) = list.path.get_ident() else {
                return PredicateEvaluation::Unmodelled;
            };
            let combinator = identifier.to_string();
            let Ok(nested) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return PredicateEvaluation::Unmodelled;
            };
            match combinator.as_str() {
                "all" => {
                    evaluate_combination(&nested, active_cfg, test_cfg, PredicateCombination::All)
                },
                "any" => {
                    evaluate_combination(&nested, active_cfg, test_cfg, PredicateCombination::Any)
                },
                "not" if nested.len() == 1 => {
                    let Some(predicate) = nested.first() else {
                        return PredicateEvaluation::Unmodelled;
                    };
                    evaluate_predicate(predicate, active_cfg, test_cfg).negated()
                },
                _ => PredicateEvaluation::Unmodelled,
            }
        },
    }
}

fn evaluate_combination(
    nested: &Punctuated<Meta, Token![,]>,
    active_cfg: &ActiveCfg,
    test_cfg: TestCfg,
    combination: PredicateCombination,
) -> PredicateEvaluation {
    let mut evaluation = match combination {
        PredicateCombination::All => PredicateEvaluation::Satisfied,
        PredicateCombination::Any => PredicateEvaluation::Unsatisfied,
    };
    for predicate in nested {
        let next = || evaluate_predicate(predicate, active_cfg, test_cfg);
        evaluation = match combination {
            PredicateCombination::All => evaluation.conjunction(next),
            PredicateCombination::Any => evaluation.disjunction(next),
        };
        if evaluation == PredicateEvaluation::Unmodelled {
            break;
        }
    }
    evaluation
}

/// Collects every identifier in `tokens`, and every tuple index written after
/// a field-access `.`: rustc names a tuple field by its index, so `failure.0`
/// names the field `0` the way `failure.code` names the field `code`.
fn collect_identifiers(tokens: &TokenStream, names: &mut FxHashSet<String>) {
    let mut dot_position = DotPosition::Elsewhere;
    for token in tokens.clone() {
        match &token {
            TokenTree::Ident(identifier) => {
                names.insert(identifier.to_string());
            },
            TokenTree::Group(group) => collect_identifiers(&group.stream(), names),
            // `collect_identifiers` splits `literal` at dots because `pair.0.1`
            // lexes its indices as the single float literal `0.1`.
            TokenTree::Literal(literal) if dot_position == DotPosition::FieldAccess => {
                names.extend(
                    literal
                        .to_string()
                        .split('.')
                        .filter(|index| {
                            !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
                        })
                        .map(str::to_owned),
                );
            },
            TokenTree::Punct(_) | TokenTree::Literal(_) => {},
        }
        dot_position = dot_position.after(&token);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use syn::ItemFn;

    use super::ActiveCfg;
    use super::AtomReference;
    use super::CfgAtomPresence;
    use super::CfgExclusion;
    use super::PredicateEvaluation;
    use super::TestCfg;
    use super::attribute_exclusion;
    use super::evaluate_predicate;
    use super::predicate_atom_reference;

    fn active_cfg() -> ActiveCfg {
        ActiveCfg {
            atoms: [
                ("unix".to_string(), None),
                ("feature".to_string(), Some("enabled".to_string())),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn cfg_presence_and_predicate_evaluation_preserve_short_circuit_order() -> anyhow::Result<()> {
        let cfg = active_cfg();
        assert_eq!(cfg.presence("unix", None), CfgAtomPresence::Present);
        assert_eq!(cfg.presence("windows", None), CfgAtomPresence::Absent);
        for (source, expected) in [
            ("unix", PredicateEvaluation::Satisfied),
            ("windows", PredicateEvaluation::Unsatisfied),
            ("feature = \"enabled\"", PredicateEvaluation::Satisfied),
            ("feature = \"missing\"", PredicateEvaluation::Unsatisfied),
            ("feature = 42", PredicateEvaluation::Unmodelled),
            ("some::atom", PredicateEvaluation::Unmodelled),
            ("unknown(unix)", PredicateEvaluation::Unmodelled),
            ("all()", PredicateEvaluation::Satisfied),
            ("any()", PredicateEvaluation::Unsatisfied),
            ("all(windows, unknown())", PredicateEvaluation::Unsatisfied),
            ("all(unknown(), windows)", PredicateEvaluation::Unmodelled),
            ("any(unix, unknown())", PredicateEvaluation::Satisfied),
            ("any(unknown(), unix)", PredicateEvaluation::Unmodelled),
            ("not(unix)", PredicateEvaluation::Unsatisfied),
            ("not(windows)", PredicateEvaluation::Satisfied),
            ("not(unknown())", PredicateEvaluation::Unmodelled),
            ("not()", PredicateEvaluation::Unmodelled),
            ("not(unix, windows)", PredicateEvaluation::Unmodelled),
            ("all(=)", PredicateEvaluation::Unmodelled),
        ] {
            let predicate = syn::parse_str(source)?;
            assert_eq!(
                evaluate_predicate(&predicate, &cfg, TestCfg::Disabled),
                expected,
                "{source}"
            );
        }
        let test = syn::parse_str("test")?;
        assert_eq!(
            evaluate_predicate(&test, &cfg, TestCfg::Enabled),
            PredicateEvaluation::Satisfied
        );
        assert_eq!(
            evaluate_predicate(&test, &cfg, TestCfg::Disabled),
            PredicateEvaluation::Unsatisfied
        );
        Ok(())
    }

    #[test]
    fn typed_predicate_accumulators_do_not_evaluate_a_decided_branch() {
        let calls = Cell::new(0);
        let next = || {
            calls.set(calls.get() + 1);
            PredicateEvaluation::Unmodelled
        };
        assert_eq!(
            PredicateEvaluation::Unsatisfied.conjunction(next),
            PredicateEvaluation::Unsatisfied
        );
        assert_eq!(
            PredicateEvaluation::Satisfied.disjunction(next),
            PredicateEvaluation::Satisfied
        );
        assert_eq!(calls.get(), 0);
        assert_eq!(
            PredicateEvaluation::Satisfied.conjunction(next),
            PredicateEvaluation::Unmodelled
        );
        assert_eq!(
            PredicateEvaluation::Unsatisfied.disjunction(next),
            PredicateEvaluation::Unmodelled
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn atom_free_and_unmodelled_predicates_keep_regions_included() -> anyhow::Result<()> {
        let cfg = active_cfg();
        for (source, expected) in [
            ("any()", AtomReference::Absent),
            ("all(any())", AtomReference::Absent),
            ("all(=)", AtomReference::Absent),
            ("unix", AtomReference::Present),
            ("feature = \"missing\"", AtomReference::Present),
            ("all(any(), windows)", AtomReference::Present),
        ] {
            assert_eq!(predicate_atom_reference(&syn::parse_str(source)?), expected);
        }
        for (source, expected) in [
            ("any()", CfgExclusion::Included),
            ("test", CfgExclusion::Included),
            ("not(test)", CfgExclusion::Included),
            ("unknown(windows)", CfgExclusion::Included),
            ("all(windows, unknown())", CfgExclusion::Excluded),
            ("all(unknown(), windows)", CfgExclusion::Included),
            ("feature = \"missing\"", CfgExclusion::Excluded),
        ] {
            let item: ItemFn = syn::parse_str(&format!("#[cfg({source})] fn f() {{}}"))?;
            for attribute in &item.attrs {
                assert_eq!(attribute_exclusion(attribute, &cfg), expected, "{source}");
            }
        }
        Ok(())
    }
}
