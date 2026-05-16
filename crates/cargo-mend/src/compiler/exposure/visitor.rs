use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::Attribute;
use syn::Fields;
use syn::ImplItem;
use syn::Item;
use syn::ItemImpl;
use syn::Path;
use syn::TraitItem;
use syn::Type;
use syn::Visibility;
use syn::visit::Visit;

pub(super) fn public_item_name(item: &Item) -> Option<String> {
    match item {
        Item::Const(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Enum(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Fn(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.sig.ident.to_string())
        },
        Item::Static(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Struct(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Trait(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Type(item) if matches!(item.vis, Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        _ => None,
    }
}

pub(super) fn public_item_surface_mentions_name(item: &Item, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    match item {
        Item::Const(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        Item::Enum(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for variant in &item.variants {
                match &variant.fields {
                    Fields::Named(fields) => {
                        for field in &fields.named {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    Fields::Unnamed(fields) => {
                        for field in &fields.unnamed {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    Fields::Unit => {},
                }
            }
        },
        Item::Fn(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_signature(&item.sig);
        },
        Item::Static(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        Item::Struct(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            match &item.fields {
                Fields::Named(fields) => {
                    for field in &fields.named {
                        visitor.visit_type(&field.ty);
                    }
                },
                Fields::Unnamed(fields) => {
                    for field in &fields.unnamed {
                        visitor.visit_type(&field.ty);
                    }
                },
                Fields::Unit => {},
            }
        },
        Item::Trait(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for trait_item in &item.items {
                match trait_item {
                    TraitItem::Fn(item) => visitor.visit_signature(&item.sig),
                    TraitItem::Type(item) => {
                        if let Some((_, ty)) = &item.default {
                            visitor.visit_type(ty);
                        }
                    },
                    TraitItem::Const(item) => visitor.visit_type(&item.ty),
                    _ => {},
                }
            }
        },
        Item::Type(item) if matches!(item.vis, Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        _ => {},
    }
    visitor.found == SurfaceReferenceMatch::Found
}

pub(super) fn impl_self_type_name(item_impl: &ItemImpl) -> Option<String> {
    let Type::Path(type_path) = item_impl.self_ty.as_ref() else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(super) fn outward_impl_surface_mentions_name(item_impl: &ItemImpl, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    let mut public_surface_status = PublicSurfaceStatus::Missing;
    let outward = item_impl.trait_.is_some();

    for impl_item in &item_impl.items {
        match impl_item {
            ImplItem::Fn(item) if outward || matches!(item.vis, Visibility::Public(_)) => {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_signature(&item.sig);
                public_surface_status = PublicSurfaceStatus::Found;
            },
            ImplItem::Const(item) if outward || matches!(item.vis, Visibility::Public(_)) => {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                public_surface_status = PublicSurfaceStatus::Found;
            },
            ImplItem::Type(item) if outward || matches!(item.vis, Visibility::Public(_)) => {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                public_surface_status = PublicSurfaceStatus::Found;
            },
            _ => {},
        }
    }

    public_surface_status.is_found() && visitor.found == SurfaceReferenceMatch::Found
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicSurfaceStatus {
    Missing,
    Found,
}

impl PublicSurfaceStatus {
    const fn is_found(self) -> bool { matches!(self, Self::Found) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceReferenceMatch {
    Missing,
    Found,
}

struct ItemSurfaceReferenceVisitor<'a> {
    item_name: &'a str,
    found:     SurfaceReferenceMatch,
}

impl<'a> ItemSurfaceReferenceVisitor<'a> {
    const fn new(item_name: &'a str) -> Self {
        Self {
            item_name,
            found: SurfaceReferenceMatch::Missing,
        }
    }
}

impl<'ast> Visit<'ast> for ItemSurfaceReferenceVisitor<'_> {
    fn visit_path(&mut self, path: &'ast Path) {
        if self.found == SurfaceReferenceMatch::Found {
            return;
        }
        if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == self.item_name)
        {
            self.found = SurfaceReferenceMatch::Found;
            return;
        }
        syn::visit::visit_path(self, path);
    }
}

fn attributes_mention_name(attrs: &[Attribute], item_name: &str) -> bool {
    attrs
        .iter()
        .any(|attr| attribute_tokens_mention_name(attr, item_name))
}

fn attribute_tokens_mention_name(attr: &Attribute, item_name: &str) -> bool {
    fn token_tree_mentions_name(tree: &TokenTree, item_name: &str) -> bool {
        match tree {
            TokenTree::Group(group) => group
                .stream()
                .into_iter()
                .any(|tree| token_tree_mentions_name(&tree, item_name)),
            TokenTree::Ident(ident) => ident == item_name,
            TokenTree::Literal(literal) => {
                literal
                    .to_string()
                    .trim_matches('"')
                    .trim_matches('r')
                    .trim_matches('#')
                    == item_name
            },
            TokenTree::Punct(_) => false,
        }
    }

    attr.meta
        .to_token_stream()
        .into_iter()
        .any(|tree| token_tree_mentions_name(&tree, item_name))
}
