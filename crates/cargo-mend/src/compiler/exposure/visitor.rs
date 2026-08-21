use proc_macro2::Span;
use syn::Field;
use syn::Fields;
use syn::ImplItem;
use syn::Item;
use syn::ItemImpl;
use syn::Path;
use syn::TraitItem;
use syn::Visibility;
use syn::visit;
use syn::visit::Visit;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceReferenceMatch {
    Missing,
    Found,
}

struct ItemSurfaceReferenceVisitor<'a> {
    item_name: &'a str,
    found:     SurfaceReferenceMatch,
}

pub(super) enum ItemSignatureCarrier<'syntax> {
    Declaration,
    StructOrUnionField {
        field:       &'syntax Field,
        field_index: usize,
    },
}

#[derive(Clone, Copy)]
pub(super) enum OutwardDeclarationKind {
    Const,
    Enum,
    Function,
    Static,
    Struct,
    Trait,
    TypeAlias,
    Union,
}

pub(super) struct OutwardDeclaration<'syntax> {
    pub(super) item:            &'syntax Item,
    pub(super) name:            String,
    pub(super) identifier_span: Span,
    pub(super) kind:            OutwardDeclarationKind,
}

pub(super) enum OutwardDeclarationClassification<'syntax> {
    Outward(OutwardDeclaration<'syntax>),
    NotOutward,
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
        visit::visit_path(self, path);
    }
}

pub(super) fn classify_outward_declaration(item: &Item) -> OutwardDeclarationClassification<'_> {
    let (identifier, kind) = match item {
        Item::Const(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::Const)
        },
        Item::Enum(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::Enum)
        },
        Item::Fn(item) if has_explicit_visibility(&item.vis) => {
            (&item.sig.ident, OutwardDeclarationKind::Function)
        },
        Item::Static(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::Static)
        },
        Item::Struct(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::Struct)
        },
        Item::Trait(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::Trait)
        },
        Item::Type(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::TypeAlias)
        },
        Item::Union(item) if has_explicit_visibility(&item.vis) => {
            (&item.ident, OutwardDeclarationKind::Union)
        },
        _ => return OutwardDeclarationClassification::NotOutward,
    };
    let source_name = identifier.to_string();
    OutwardDeclarationClassification::Outward(OutwardDeclaration {
        item,
        name: source_name
            .strip_prefix("r#")
            .unwrap_or(&source_name)
            .to_string(),
        identifier_span: identifier.span(),
        kind,
    })
}

pub(super) fn potentially_outward_item_surface_carriers_mentioning_name<'syntax>(
    item: &'syntax Item,
    item_name: &str,
) -> Vec<ItemSignatureCarrier<'syntax>> {
    let mut carriers = Vec::new();
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    match item {
        Item::Const(item) if has_explicit_visibility(&item.vis) => {
            visitor.visit_type(&item.ty);
        },
        Item::Enum(item) if has_explicit_visibility(&item.vis) => {
            for variant in &item.variants {
                visit_field_types(&mut visitor, &variant.fields);
            }
        },
        Item::Fn(item) if has_explicit_visibility(&item.vis) => {
            visitor.visit_signature(&item.sig);
        },
        Item::Static(item) if has_explicit_visibility(&item.vis) => {
            visitor.visit_type(&item.ty);
        },
        Item::Struct(item) if has_explicit_visibility(&item.vis) => {
            match &item.fields {
                Fields::Named(fields) => carriers.extend(visible_field_carriers(
                    fields.named.iter().enumerate(),
                    item_name,
                )),
                Fields::Unnamed(fields) => carriers.extend(visible_field_carriers(
                    fields.unnamed.iter().enumerate(),
                    item_name,
                )),
                Fields::Unit => {},
            }
            return carriers;
        },
        Item::Trait(item) if has_explicit_visibility(&item.vis) => {
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
        Item::Type(item) if has_explicit_visibility(&item.vis) => {
            visitor.visit_type(&item.ty);
        },
        Item::Union(item) if has_explicit_visibility(&item.vis) => {
            carriers.extend(visible_field_carriers(
                item.fields.named.iter().enumerate(),
                item_name,
            ));
            return carriers;
        },
        _ => {},
    }
    if visitor.found == SurfaceReferenceMatch::Found {
        carriers.push(ItemSignatureCarrier::Declaration);
    }
    carriers
}

fn visible_field_carriers<'syntax>(
    fields: impl Iterator<Item = (usize, &'syntax Field)>,
    item_name: &str,
) -> Vec<ItemSignatureCarrier<'syntax>> {
    fields
        .filter(|(_, field)| has_explicit_visibility(&field.vis))
        .filter(|(_, field)| field_type_mentions_name(field, item_name))
        .map(|(field_index, field)| ItemSignatureCarrier::StructOrUnionField { field, field_index })
        .collect()
}

fn visit_field_types(visitor: &mut ItemSurfaceReferenceVisitor<'_>, fields: &Fields) {
    match fields {
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

pub(super) fn potentially_outward_impl_surface_items_mentioning_name<'syntax>(
    item_impl: &'syntax ItemImpl,
    item_name: &str,
) -> Vec<&'syntax ImplItem> {
    let outward = item_impl.trait_.is_some();

    item_impl
        .items
        .iter()
        .filter(|impl_item| outward || impl_item_has_explicit_visibility(impl_item))
        .filter(|impl_item| impl_item_surface_mentions_name(impl_item, item_name))
        .collect()
}

const fn has_explicit_visibility(visibility: &Visibility) -> bool {
    !matches!(visibility, Visibility::Inherited)
}

const fn impl_item_has_explicit_visibility(item: &ImplItem) -> bool {
    match item {
        ImplItem::Const(item) => has_explicit_visibility(&item.vis),
        ImplItem::Fn(item) => has_explicit_visibility(&item.vis),
        ImplItem::Type(item) => has_explicit_visibility(&item.vis),
        _ => false,
    }
}

fn impl_item_surface_mentions_name(item: &ImplItem, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    match item {
        ImplItem::Fn(item) => {
            visitor.visit_signature(&item.sig);
        },
        ImplItem::Const(item) => {
            visitor.visit_type(&item.ty);
        },
        ImplItem::Type(item) => {
            visitor.visit_type(&item.ty);
        },
        _ => {},
    }
    visitor.found == SurfaceReferenceMatch::Found
}

fn field_type_mentions_name(field: &Field, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    visitor.visit_type(&field.ty);
    visitor.found == SurfaceReferenceMatch::Found
}
