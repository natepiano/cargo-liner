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

    pub(in crate::fixes) fn contains(attributes: &[Attribute]) -> bool {
        attributes.iter().any(is_conditional)
    }

    /// The gate a synthesized import must carry to cover every occurrence in
    /// `candidates`. One ungated occurrence forces an ungated import; a single
    /// agreed gate is reproduced verbatim; gates that disagree yield `None`,
    /// leaving the caller to skip the rewrite rather than insert an import
    /// that resolves in only some configurations.
    pub(in crate::fixes) fn covering(candidates: &BTreeSet<Self>) -> Option<Self> {
        let ungated = Self::default();
        if candidates.contains(&ungated) {
            return Some(ungated);
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
        Expr::Array(node) => &node.attrs,
        Expr::Assign(node) => &node.attrs,
        Expr::Async(node) => &node.attrs,
        Expr::Await(node) => &node.attrs,
        Expr::Binary(node) => &node.attrs,
        Expr::Block(node) => &node.attrs,
        Expr::Break(node) => &node.attrs,
        Expr::Call(node) => &node.attrs,
        Expr::Cast(node) => &node.attrs,
        Expr::Closure(node) => &node.attrs,
        Expr::Const(node) => &node.attrs,
        Expr::Continue(node) => &node.attrs,
        Expr::Field(node) => &node.attrs,
        Expr::ForLoop(node) => &node.attrs,
        Expr::Group(node) => &node.attrs,
        Expr::If(node) => &node.attrs,
        Expr::Index(node) => &node.attrs,
        Expr::Infer(node) => &node.attrs,
        Expr::Let(node) => &node.attrs,
        Expr::Lit(node) => &node.attrs,
        Expr::Loop(node) => &node.attrs,
        Expr::Macro(node) => &node.attrs,
        Expr::Match(node) => &node.attrs,
        Expr::MethodCall(node) => &node.attrs,
        Expr::Paren(node) => &node.attrs,
        Expr::Path(node) => &node.attrs,
        Expr::Range(node) => &node.attrs,
        Expr::RawAddr(node) => &node.attrs,
        Expr::Reference(node) => &node.attrs,
        Expr::Repeat(node) => &node.attrs,
        Expr::Return(node) => &node.attrs,
        Expr::Struct(node) => &node.attrs,
        Expr::Try(node) => &node.attrs,
        Expr::TryBlock(node) => &node.attrs,
        Expr::Tuple(node) => &node.attrs,
        Expr::Unary(node) => &node.attrs,
        Expr::Unsafe(node) => &node.attrs,
        Expr::While(node) => &node.attrs,
        Expr::Yield(node) => &node.attrs,
        _ => &[],
    }
}

/// Attributes on an item, for the same reason `expr_attributes` exists:
/// `syn::Item` publishes no accessor and is `#[non_exhaustive]`.
pub(in crate::fixes) fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(node) => &node.attrs,
        Item::Enum(node) => &node.attrs,
        Item::ExternCrate(node) => &node.attrs,
        Item::Fn(node) => &node.attrs,
        Item::ForeignMod(node) => &node.attrs,
        Item::Impl(node) => &node.attrs,
        Item::Macro(node) => &node.attrs,
        Item::Mod(node) => &node.attrs,
        Item::Static(node) => &node.attrs,
        Item::Struct(node) => &node.attrs,
        Item::Trait(node) => &node.attrs,
        Item::TraitAlias(node) => &node.attrs,
        Item::Type(node) => &node.attrs,
        Item::Union(node) => &node.attrs,
        Item::Use(node) => &node.attrs,
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
