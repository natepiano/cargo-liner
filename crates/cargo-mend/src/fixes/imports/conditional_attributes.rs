use std::collections::BTreeSet;

use proc_macro2::Span;
use syn::Attribute;
use syn::Expr;
use syn::Item;
use syn::Meta;
use syn::Token;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

use super::path::ImportSource;

#[derive(Clone, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::fixes) struct ConditionalAttributes {
    source: Vec<String>,
}

impl ConditionalAttributes {
    pub(in crate::fixes) fn from_attributes(
        indexed: &ImportSource<'_>,
        attributes: &[Attribute],
    ) -> Self {
        if attributes.is_empty() {
            return Self::default();
        }
        let source = attributes
            .iter()
            .filter_map(|attribute| conditional_attribute_source(indexed, attribute))
            .collect();
        Self { source }
    }

    #[cfg(test)]
    pub(in crate::fixes) fn contains(attributes: &[Attribute]) -> bool {
        attributes.iter().any(is_conditional)
    }

    /// The gate a synthesized import must carry to cover every occurrence in
    /// `candidates`. One ungated occurrence forces an ungated import; a single
    /// agreed gate is reproduced verbatim; gates that disagree yield `None`,
    /// leaving the caller to skip the rewrite rather than insert an import
    /// that resolves in only some configurations.
    pub(in crate::fixes) fn covering(candidates: &BTreeSet<Self>) -> Option<Self> {
        let ungated_conditional_attributes = Self::default();
        if candidates.contains(&ungated_conditional_attributes) {
            return Some(ungated_conditional_attributes);
        }
        if candidates.len() == 1 {
            return candidates.first().cloned();
        }
        None
    }

    pub(in crate::fixes) fn extend(&mut self, other: Self) { self.source.extend(other.source); }

    /// True when every gate in `self` also appears in `other` — an item gated
    /// by `self` is compiled in every configuration where a site gated by
    /// `other` is. Gates are compared by source text, so differently spelled
    /// but equivalent predicates conservatively compare unequal.
    pub(in crate::fixes) fn is_subset_of(&self, other: &Self) -> bool {
        self.source
            .iter()
            .all(|attribute| other.source.contains(attribute))
    }

    pub(in crate::fixes) const fn is_empty(&self) -> bool { self.source.is_empty() }

    pub(in crate::fixes) const fn len(&self) -> usize { self.source.len() }

    pub(in crate::fixes) fn render(&self, indent: &str) -> String {
        let mut rendered = String::new();
        for attribute in &self.source {
            rendered.push_str(indent);
            rendered.push_str(attribute);
            rendered.push('\n');
        }
        rendered
    }

    pub(in crate::fixes) fn truncate(&mut self, len: usize) { self.source.truncate(len); }
}

pub(in crate::fixes) fn is_conditional(attribute: &Attribute) -> bool {
    ConditionalMeta::from(&attribute.meta).applies_cfg()
}

/// Attributes written on a statement attach to the statement's expression:
/// `#[cfg(test)] app.init_resource::<Injected>();` parses as a `Stmt::Expr`
/// holding an `ExprMethodCall` whose `attrs` carry the `cfg`. `syn::Expr`
/// publishes no accessor for those, so every variant that owns an `attrs`
/// field is named here. `Expr` is `#[non_exhaustive]`; a variant syn adds
/// later reports no gates, which keeps the enclosing occurrence fully
/// qualified rather than importing it under the wrong `cfg`.
pub(in crate::fixes) fn expr_attributes(expr: &Expr) -> &[Attribute] {
    match expr {
        Expr::Array(expr_array) => &expr_array.attrs,
        Expr::Assign(expr_assign) => &expr_assign.attrs,
        Expr::Async(expr_async) => &expr_async.attrs,
        Expr::Await(expr_await) => &expr_await.attrs,
        Expr::Binary(expr_binary) => &expr_binary.attrs,
        Expr::Block(expr_block) => &expr_block.attrs,
        Expr::Break(expr_break) => &expr_break.attrs,
        Expr::Call(expr_call) => &expr_call.attrs,
        Expr::Cast(expr_cast) => &expr_cast.attrs,
        Expr::Closure(expr_closure) => &expr_closure.attrs,
        Expr::Const(expr_const) => &expr_const.attrs,
        Expr::Continue(expr_continue) => &expr_continue.attrs,
        Expr::Field(expr_field) => &expr_field.attrs,
        Expr::ForLoop(expr_for_loop) => &expr_for_loop.attrs,
        Expr::Group(expr_group) => &expr_group.attrs,
        Expr::If(expr_if) => &expr_if.attrs,
        Expr::Index(expr_index) => &expr_index.attrs,
        Expr::Infer(expr_infer) => &expr_infer.attrs,
        Expr::Let(expr_let) => &expr_let.attrs,
        Expr::Lit(expr_lit) => &expr_lit.attrs,
        Expr::Loop(expr_loop) => &expr_loop.attrs,
        Expr::Macro(expr_macro) => &expr_macro.attrs,
        Expr::Match(expr_match) => &expr_match.attrs,
        Expr::MethodCall(expr_method_call) => &expr_method_call.attrs,
        Expr::Paren(expr_paren) => &expr_paren.attrs,
        Expr::Path(expr_path) => &expr_path.attrs,
        Expr::Range(expr_range) => &expr_range.attrs,
        Expr::RawAddr(expr_raw_addr) => &expr_raw_addr.attrs,
        Expr::Reference(expr_reference) => &expr_reference.attrs,
        Expr::Repeat(expr_repeat) => &expr_repeat.attrs,
        Expr::Return(expr_return) => &expr_return.attrs,
        Expr::Struct(expr_struct) => &expr_struct.attrs,
        Expr::Try(expr_try) => &expr_try.attrs,
        Expr::TryBlock(expr_try_block) => &expr_try_block.attrs,
        Expr::Tuple(expr_tuple) => &expr_tuple.attrs,
        Expr::Unary(expr_unary) => &expr_unary.attrs,
        Expr::Unsafe(expr_unsafe) => &expr_unsafe.attrs,
        Expr::While(expr_while) => &expr_while.attrs,
        Expr::Yield(expr_yield) => &expr_yield.attrs,
        _ => &[],
    }
}

/// Attributes on an item, for the same reason `expr_attributes` exists:
/// `syn::Item` publishes no accessor and is `#[non_exhaustive]`.
pub(in crate::fixes) fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item_const) => &item_const.attrs,
        Item::Enum(item_enum) => &item_enum.attrs,
        Item::ExternCrate(item_extern_crate) => &item_extern_crate.attrs,
        Item::Fn(item_fn) => &item_fn.attrs,
        Item::ForeignMod(item_foreign_mod) => &item_foreign_mod.attrs,
        Item::Impl(item_impl) => &item_impl.attrs,
        Item::Macro(item_macro) => &item_macro.attrs,
        Item::Mod(item_mod) => &item_mod.attrs,
        Item::Static(item_static) => &item_static.attrs,
        Item::Struct(item_struct) => &item_struct.attrs,
        Item::Trait(item_trait) => &item_trait.attrs,
        Item::TraitAlias(item_trait_alias) => &item_trait_alias.attrs,
        Item::Type(item_type) => &item_type.attrs,
        Item::Union(item_union) => &item_union.attrs,
        Item::Use(item_use) => &item_use.attrs,
        _ => &[],
    }
}

/// Parsed conditional meaning, shared by gate detection and source rendering.
enum ConditionalMeta {
    Cfg {
        source: Span,
    },
    CfgAttr {
        predicate: Span,
        nested:    Vec<Self>,
    },
    Ignored,
}

impl From<&Meta> for ConditionalMeta {
    fn from(meta: &Meta) -> Self {
        if meta.path().is_ident("cfg") {
            return Self::Cfg {
                source: meta.span(),
            };
        }
        if !meta.path().is_ident("cfg_attr") {
            return Self::Ignored;
        }
        let list = match meta {
            Meta::List(list) => list,
            Meta::NameValue(_) | Meta::Path(_) => return Self::Ignored,
        };
        let Ok(metas) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        else {
            return Self::Ignored;
        };
        let Some(predicate) = metas.first() else {
            return Self::Ignored;
        };
        Self::CfgAttr {
            predicate: predicate.span(),
            nested:    metas.iter().skip(1).map(Self::from).collect(),
        }
    }
}

impl ConditionalMeta {
    fn applies_cfg(&self) -> bool {
        match self {
            Self::Cfg { .. } => true,
            Self::CfgAttr { nested, .. } => nested.iter().any(Self::applies_cfg),
            Self::Ignored => false,
        }
    }

    fn gating_source(&self, indexed: &ImportSource<'_>) -> Option<String> {
        match self {
            Self::Cfg { source } => Some(source_for_span(indexed, *source)?.to_string()),
            Self::CfgAttr { predicate, nested } => {
                let gating_attributes = nested
                    .iter()
                    .filter_map(|nested| nested.gating_source(indexed))
                    .collect::<Vec<_>>();
                if gating_attributes.is_empty() {
                    return None;
                }
                Some(format!(
                    "cfg_attr({}, {})",
                    source_for_span(indexed, *predicate)?,
                    gating_attributes.join(", ")
                ))
            },
            Self::Ignored => None,
        }
    }
}

fn conditional_attribute_source(
    source: &ImportSource<'_>,
    attribute: &Attribute,
) -> Option<String> {
    let conditional = ConditionalMeta::from(&attribute.meta);
    match conditional {
        ConditionalMeta::Cfg { .. } => Some(source_for_span(source, attribute.span())?.to_string()),
        ConditionalMeta::CfgAttr { .. } | ConditionalMeta::Ignored => conditional
            .gating_source(source)
            .map(|meta| format!("#[{meta}]")),
    }
}

fn source_for_span<'a>(source: &ImportSource<'a>, span: Span) -> Option<&'a str> {
    Some(source.locate(span.start(), span.end())?.text())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use syn::ItemFn;

    use super::ConditionalAttributes;

    fn gates(source: &str) -> (bool, String) {
        let item: ItemFn = syn::parse_str(source).expect("parse attributed function");
        let indexed = super::ImportSource::new(source);
        (
            ConditionalAttributes::contains(&item.attrs),
            ConditionalAttributes::from_attributes(&indexed, &item.attrs).render(""),
        )
    }

    #[test]
    fn nested_cfg_attr_keeps_only_gating_attributes_and_predicates() {
        let source = "#[cfg_attr(feature = \"x\", allow(dead_code), \
                      cfg_attr(test, cfg(unix), inline), cfg(windows))]\nfn f() {}";
        assert_eq!(
            gates(source),
            (
                true,
                "#[cfg_attr(feature = \"x\", cfg_attr(test, cfg(unix)), cfg(windows))]\n"
                    .to_string()
            )
        );
        assert_eq!(
            gates("#[cfg( any(unix, windows) )]\nfn f() {}"),
            (true, "#[cfg( any(unix, windows) )]\n".to_string())
        );
    }

    #[test]
    fn unicode_before_a_gate_preserves_its_source() {
        let source = "#[doc = \"é\"] #[cfg(unix)] fn f() {}";
        assert_eq!(gates(source), (true, "#[cfg(unix)]\n".to_string()));
    }

    #[test]
    fn malformed_or_nongating_meta_does_not_create_an_import_gate() {
        for attribute in [
            "#[cfg_attr]",
            "#[cfg_attr()]",
            "#[cfg_attr(test)]",
            "#[cfg_attr = \"test\"]",
            "#[cfg_attr(test, cfg_attr(broken => cfg(unix)))]",
            "#[cfg_attr(test, allow(dead_code))]",
            "#[allow(dead_code)]",
        ] {
            assert_eq!(
                gates(&format!("{attribute}\nfn f() {{}}")),
                (false, String::new()),
                "{attribute}"
            );
        }
    }
}
