use std::collections::BTreeSet;

use syn::Attribute;
use syn::Block;
use syn::Expr;
use syn::ForeignItem;
use syn::Ident;
use syn::Item;
use syn::Meta;
use syn::MetaList;
use syn::Path;
use syn::Stmt;
use syn::Token;
use syn::ext::IdentExt;
use syn::parse::Parser;
use syn::punctuated::Punctuated;

use crate::fixes::imports;
use crate::fixes::imports::UseBinding;

pub(super) enum BlockBindings {
    Named(BTreeSet<String>),
    Unknown,
}

#[derive(PartialEq, Eq)]
pub(super) enum BlockBinding {
    MayBind,
    Absent,
}

struct BuiltInAttributes {
    names:   &'static [&'static str],
    derives: &'static [&'static str],
}

const BUILT_IN_ATTRIBUTES: BuiltInAttributes = BuiltInAttributes {
    // https://doc.rust-lang.org/reference/attributes.html#built-in-attributes-index
    names:   &[
        "allow",
        "automatically_derived",
        "cfg",
        "cfg_attr",
        "cold",
        "collapse_debuginfo",
        "crate_name",
        "crate_type",
        "debugger_visualizer",
        "deny",
        "deprecated",
        "derive",
        "diagnostic::do_not_recommend",
        "diagnostic::on_unimplemented",
        "doc",
        "expect",
        "export_name",
        "feature",
        "forbid",
        "global_allocator",
        "ignore",
        "inline",
        "instruction_set",
        "link",
        "link_name",
        "link_ordinal",
        "link_section",
        "macro_export",
        "macro_use",
        "must_use",
        "naked",
        "no_builtins",
        "no_implicit_prelude",
        "no_link",
        "no_main",
        "no_mangle",
        "no_std",
        "non_exhaustive",
        "panic_handler",
        "path",
        "proc_macro",
        "proc_macro_attribute",
        "proc_macro_derive",
        "recursion_limit",
        "repr",
        "should_panic",
        "target_feature",
        "test",
        "track_caller",
        "type_length_limit",
        "used",
        "warn",
        "windows_subsystem",
    ],
    derives: &[
        "Clone",
        "Copy",
        "Debug",
        "Default",
        "Eq",
        "Hash",
        "Ord",
        "PartialEq",
        "PartialOrd",
    ],
};

impl BlockBindings {
    pub(super) fn collect(block: &Block) -> Self {
        let mut names = BTreeSet::new();
        for statement in &block.stmts {
            let item = match statement {
                Stmt::Item(item) => item,
                Stmt::Macro(_) | Stmt::Expr(Expr::Macro(_), _) => return Self::Unknown,
                _ => continue,
            };
            if attribute_binding(imports::item_attributes(item)) == BlockBinding::MayBind {
                return Self::Unknown;
            }
            let name = match item {
                Item::Use(item_use) => {
                    for binding in imports::collect_use_bindings(&item_use.tree) {
                        match binding {
                            UseBinding::Named { name, .. } => {
                                let name = if name.starts_with("r#") {
                                    let Ok(ident) = Ident::parse_any.parse_str(&name) else {
                                        return Self::Unknown;
                                    };
                                    ident.unraw().to_string()
                                } else {
                                    name
                                };
                                names.insert(name);
                            },
                            UseBinding::Glob { .. } => return Self::Unknown,
                        }
                    }
                    continue;
                },
                Item::Const(item) => &item.ident,
                Item::Enum(item) => &item.ident,
                Item::ExternCrate(item) => {
                    item.rename.as_ref().map_or(&item.ident, |(_, name)| name)
                },
                Item::Fn(item) => &item.sig.ident,
                Item::ForeignMod(item) => {
                    let Self::Named(foreign_names) = Self::foreign(&item.items) else {
                        return Self::Unknown;
                    };
                    names.extend(foreign_names);
                    continue;
                },
                Item::Macro(item)
                    if item.mac.path.is_ident("macro_rules") && item.ident.is_some() =>
                {
                    continue;
                },
                Item::Macro(_) | Item::Verbatim(_) => return Self::Unknown,
                Item::Mod(item) => &item.ident,
                Item::Static(item) => &item.ident,
                Item::Struct(item) => &item.ident,
                Item::Trait(item) => &item.ident,
                Item::TraitAlias(item) => &item.ident,
                Item::Type(item) => &item.ident,
                Item::Union(item) => &item.ident,
                _ => continue,
            };
            names.insert(name.unraw().to_string());
        }
        Self::Named(names)
    }

    fn foreign(items: &[ForeignItem]) -> Self {
        let mut names = BTreeSet::new();
        for item in items {
            if attribute_binding(imports::foreign_item_attributes(item)) == BlockBinding::MayBind {
                return Self::Unknown;
            }
            let name = match item {
                ForeignItem::Fn(item) => &item.sig.ident,
                ForeignItem::Static(item) => &item.ident,
                ForeignItem::Type(item) => &item.ident,
                _ => return Self::Unknown,
            };
            names.insert(name.unraw().to_string());
        }
        Self::Named(names)
    }

    pub(super) fn lookup(scopes: &[Self], name: &Ident) -> BlockBinding {
        if scopes.is_empty() {
            return BlockBinding::Absent;
        }
        let name = name.unraw().to_string();
        for scope in scopes.iter().rev() {
            match scope {
                Self::Unknown => return BlockBinding::MayBind,
                Self::Named(names) if names.contains(&name) => return BlockBinding::MayBind,
                Self::Named(_) => {},
            }
        }
        BlockBinding::Absent
    }
}

fn attribute_binding(attributes: &[Attribute]) -> BlockBinding {
    if attributes
        .iter()
        .any(|attribute| meta_binding(&attribute.meta) == BlockBinding::MayBind)
    {
        BlockBinding::MayBind
    } else {
        BlockBinding::Absent
    }
}

fn meta_binding(meta: &Meta) -> BlockBinding {
    let path = meta.path();
    if path.is_ident("unsafe") {
        return meta
            .require_list()
            .and_then(MetaList::parse_args::<Meta>)
            .map_or(BlockBinding::MayBind, |inner| meta_binding(&inner));
    }

    let name = if let Some(name) = path.get_ident() {
        name.to_string()
    } else if path.leading_colon.is_none()
        && path.segments.len() == 2
        && path
            .segments
            .iter()
            .all(|segment| segment.arguments.is_empty())
    {
        format!("{}::{}", path.segments[0].ident, path.segments[1].ident)
    } else {
        return BlockBinding::MayBind;
    };
    if !BUILT_IN_ATTRIBUTES.names.contains(&name.as_str()) {
        return BlockBinding::MayBind;
    }
    let builtin = match name.as_str() {
        "cfg_attr" => meta
            .require_list()
            .and_then(|list| list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated))
            .is_ok_and(|entries| {
                entries
                    .iter()
                    .skip(1)
                    .all(|meta| meta_binding(meta) == BlockBinding::Absent)
            }),
        "derive" => meta
            .require_list()
            .and_then(|list| list.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated))
            .is_ok_and(|entries| {
                entries.iter().all(|path| {
                    path.get_ident().is_some_and(|name| {
                        BUILT_IN_ATTRIBUTES
                            .derives
                            .contains(&name.to_string().as_str())
                    })
                })
            }),
        _ => true,
    };
    if builtin {
        BlockBinding::Absent
    } else {
        BlockBinding::MayBind
    }
}

#[cfg(test)]
mod tests {
    use syn::Block;
    use syn::Ident;

    use super::BlockBinding;
    use super::BlockBindings;

    #[test]
    fn built_in_attributes_leave_the_first_path_segment_unbound() -> syn::Result<()> {
        for source in [
            r#"{
                #[link(name = "c")]
                unsafe extern "C" {
                    #[link_name = "foreign_marker"]
                    fn marker();
                }
            }"#,
            r#"{
                #[diagnostic::on_unimplemented(message = "Marker required")]
                trait Marker {}
            }"#,
            "{
                #[diagnostic::do_not_recommend]
                impl Marker for () {}
            }",
            "{
                #[cfg_attr(test, deprecated, derive(Clone, Copy))]
                struct Marker;
            }",
            "{
                #[unsafe(no_mangle)]
                fn marker() {}
            }",
        ] {
            let block: Block = syn::parse_str(source)?;
            let first_segment: Ident = syn::parse_str("std")?;
            let scopes = [BlockBindings::collect(&block)];
            assert!(
                BlockBindings::lookup(&scopes, &first_segment) == BlockBinding::Absent,
                "Expected no binding for {first_segment} in {source}"
            );
        }
        Ok(())
    }

    #[test]
    fn item_macros_can_bind_the_first_path_segment() -> syn::Result<()> {
        for (source, first_segment) in [
            (
                "{
                    #[cfg_attr(test, derive(Clone, block_binding_macros::ImportDerived))]
                    struct DeriveTarget;
                    let _ = derived::DerivedMode::Ready;
                }",
                "derived",
            ),
            (
                "{
                    #[block_binding_macros::import_attributed]
                    fn attribute_target() {}
                    let _ = attributed::AttributedMode::Ready;
                }",
                "attributed",
            ),
        ] {
            let block: Block = syn::parse_str(source)?;
            let first_segment: Ident = syn::parse_str(first_segment)?;
            let scopes = [BlockBindings::collect(&block)];
            assert!(
                BlockBindings::lookup(&scopes, &first_segment) == BlockBinding::MayBind,
                "Expected a possible binding for {first_segment} in {source}"
            );
        }
        Ok(())
    }
}
