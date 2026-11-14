use std::ops::Range;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::Span;
use syn::Attribute;
use syn::Block;
use syn::Ident;
use syn::ImplItemFn;
use syn::ItemFn;
use syn::ItemImpl;
use syn::ItemMod;
use syn::ItemTrait;
use syn::TraitItemFn;
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::fixes::imports::ImportSource;
use crate::rust_syntax::LexicalRegions;

struct FunctionCollector<'a> {
    source:           ImportSource<'a>,
    lexical_regions:  LexicalRegions,
    length_exemption: LengthExemption,
    functions:        Vec<FunctionBody>,
    result:           Result<()>,
}

impl FunctionCollector<'_> {
    fn with_exemption(&mut self, attrs: &[Attribute], visit: impl FnOnce(&mut Self)) {
        let previous = self.length_exemption;
        self.length_exemption = exemption(attrs, previous);
        visit(self);
        self.length_exemption = previous;
    }

    fn record(&mut self, name: &Ident, span: Span, block: &Block, attrs: &[Attribute]) {
        let result = (|| {
            let full = self
                .source
                .locate(span.start(), span.end())
                .context("function span outside source")?;
            let body_span = block.span();
            let body = self
                .source
                .locate(body_span.start(), body_span.end())
                .context("function body outside source")?;
            self.functions.push(FunctionBody {
                name:             name.to_string(),
                range:            full.start()..full.end(),
                body_bytes:       body.end() - body.start(),
                lines:            self
                    .lexical_regions
                    .code_lines(body.start() + 1..body.end() - 1),
                length_exemption: exemption(attrs, self.length_exemption),
            });
            Ok(())
        })();
        if self.result.is_ok() {
            self.result = result;
        }
    }
}

impl<'ast> Visit<'ast> for FunctionCollector<'_> {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        self.record(&item.sig.ident, item.span(), &item.block, &item.attrs);
        self.with_exemption(&item.attrs, |visitor| {
            syn::visit::visit_item_fn(visitor, item);
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast ImplItemFn) {
        self.record(&item.sig.ident, item.span(), &item.block, &item.attrs);
        self.with_exemption(&item.attrs, |visitor| {
            syn::visit::visit_impl_item_fn(visitor, item);
        });
    }

    fn visit_trait_item_fn(&mut self, item: &'ast TraitItemFn) {
        if let Some(block) = &item.default {
            self.record(&item.sig.ident, item.span(), block, &item.attrs);
        }
        self.with_exemption(&item.attrs, |visitor| {
            syn::visit::visit_trait_item_fn(visitor, item);
        });
    }

    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        self.with_exemption(&item.attrs, |visitor| {
            syn::visit::visit_item_mod(visitor, item);
        });
    }

    fn visit_item_impl(&mut self, item: &'ast ItemImpl) {
        self.with_exemption(&item.attrs, |visitor| {
            syn::visit::visit_item_impl(visitor, item);
        });
    }

    fn visit_item_trait(&mut self, item: &'ast ItemTrait) {
        self.with_exemption(&item.attrs, |visitor| {
            syn::visit::visit_item_trait(visitor, item);
        });
    }
}

pub(super) fn collect(text: &str) -> Result<Vec<FunctionBody>> {
    let syntax = syn::parse_file(text)?;
    let mut collector = FunctionCollector {
        source:           ImportSource::new(text),
        lexical_regions:  LexicalRegions::from(text.to_owned()),
        length_exemption: exemption(&syntax.attrs, LengthExemption::Limited),
        functions:        Vec::new(),
        result:           Ok(()),
    };
    collector.visit_file(&syntax);
    collector.result?;
    Ok(collector.functions)
}

pub(super) struct FunctionBody {
    pub(super) name:             String,
    pub(super) range:            Range<usize>,
    pub(super) body_bytes:       usize,
    pub(super) lines:            usize,
    pub(super) length_exemption: LengthExemption,
}

#[derive(Clone, Copy)]
pub(super) enum LengthExemption {
    Limited,
    Exempt,
}

fn exemption(attrs: &[Attribute], inherited: LengthExemption) -> LengthExemption {
    attrs.iter().fold(inherited, |current, attr| {
        let level = if attr.path().is_ident("allow") || attr.path().is_ident("expect") {
            LengthExemption::Exempt
        } else if attr.path().is_ident("deny")
            || attr.path().is_ident("warn")
            || attr.path().is_ident("forbid")
        {
            LengthExemption::Limited
        } else {
            return current;
        };
        let names_length_lint = attr
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .is_ok_and(|items| {
                items.iter().any(|meta| {
                    let segments = &meta.path().segments;
                    segments.len() == 2
                        && segments[0].ident == "clippy"
                        && segments[1].ident == "too_many_lines"
                })
            });
        if names_length_lint { level } else { current }
    })
}

#[cfg(test)]
mod tests {
    use super::LengthExemption;
    use super::collect;

    #[test]
    fn nearer_lint_levels_restore_function_length_protection() -> anyhow::Result<()> {
        for level in ["deny", "warn", "forbid"] {
            let source = format!(
                "#![allow(clippy::too_many_lines)]\n#[{level}(clippy::too_many_lines)]\nfn protected() {{ run(); }}\n"
            );
            let functions = collect(&source)?;
            assert!(
                matches!(functions[0].length_exemption, LengthExemption::Limited),
                "{level}"
            );
        }
        Ok(())
    }

    #[test]
    fn function_body_count_uses_literal_and_nested_comment_regions() -> anyhow::Result<()> {
        let source = concat!(
            "fn measured() {\n",
            "let marker = \"/*\";\n",
            "run();\n",
            "/* outer /* nested */\n",
            "still a comment */ run();\n",
            "// comment only\n\n",
            "}\n",
        );
        assert_eq!(collect(source)?[0].lines, 3);
        Ok(())
    }
}
