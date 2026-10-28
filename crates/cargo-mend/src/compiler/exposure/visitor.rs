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

/// Trait members are outward through their contract; inherent members need explicit visibility.
#[derive(Clone, Copy)]
enum ImplSurfaceAccess {
    TraitContract,
    ExplicitMembers,
}

#[derive(Debug, PartialEq, Eq)]
enum DeclarationVisibility {
    Inherited,
    Explicit,
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
        Item::Const(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            (&item.ident, OutwardDeclarationKind::Const)
        },
        Item::Enum(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            (&item.ident, OutwardDeclarationKind::Enum)
        },
        Item::Fn(item) if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit => {
            (&item.sig.ident, OutwardDeclarationKind::Function)
        },
        Item::Static(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            (&item.ident, OutwardDeclarationKind::Static)
        },
        Item::Struct(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            (&item.ident, OutwardDeclarationKind::Struct)
        },
        Item::Trait(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            (&item.ident, OutwardDeclarationKind::Trait)
        },
        Item::Type(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            (&item.ident, OutwardDeclarationKind::TypeAlias)
        },
        Item::Union(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
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
        Item::Const(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            visitor.visit_type(&item.ty);
        },
        Item::Enum(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            for variant in &item.variants {
                visit_field_types(&mut visitor, &variant.fields);
            }
        },
        Item::Fn(item) if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit => {
            visitor.visit_signature(&item.sig);
        },
        Item::Static(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            visitor.visit_type(&item.ty);
        },
        Item::Struct(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
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
        Item::Trait(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
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
        Item::Type(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
            visitor.visit_type(&item.ty);
        },
        Item::Union(item)
            if declaration_visibility(&item.vis) == DeclarationVisibility::Explicit =>
        {
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
        .filter(|(_, field)| declaration_visibility(&field.vis) == DeclarationVisibility::Explicit)
        .filter(|(_, field)| field_type_reference(field, item_name) == SurfaceReferenceMatch::Found)
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
    let access = match &item_impl.trait_ {
        Some(_) => ImplSurfaceAccess::TraitContract,
        None => ImplSurfaceAccess::ExplicitMembers,
    };

    item_impl
        .items
        .iter()
        .filter(|impl_item| match access {
            ImplSurfaceAccess::TraitContract => true,
            ImplSurfaceAccess::ExplicitMembers => {
                impl_item_visibility(impl_item) == DeclarationVisibility::Explicit
            },
        })
        .filter(|impl_item| {
            impl_item_surface_reference(impl_item, item_name) == SurfaceReferenceMatch::Found
        })
        .collect()
}

const fn declaration_visibility(visibility: &Visibility) -> DeclarationVisibility {
    match visibility {
        Visibility::Inherited => DeclarationVisibility::Inherited,
        Visibility::Public(_) | Visibility::Restricted(_) => DeclarationVisibility::Explicit,
    }
}

const fn impl_item_visibility(item: &ImplItem) -> DeclarationVisibility {
    match item {
        ImplItem::Const(item) => declaration_visibility(&item.vis),
        ImplItem::Fn(item) => declaration_visibility(&item.vis),
        ImplItem::Type(item) => declaration_visibility(&item.vis),
        // syn::ImplItem is non-exhaustive; unsupported syntax has no known visibility.
        _ => DeclarationVisibility::Inherited,
    }
}

fn impl_item_surface_reference(item: &ImplItem, item_name: &str) -> SurfaceReferenceMatch {
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
    visitor.found
}

fn field_type_reference(field: &Field, item_name: &str) -> SurfaceReferenceMatch {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    visitor.visit_type(&field.ty);
    visitor.found
}

#[cfg(test)]
mod tests {
    use syn::ImplItem;
    use syn::Item;
    use syn::ItemImpl;
    use syn::Visibility;

    use super::DeclarationVisibility;
    use super::ItemSignatureCarrier;
    use super::OutwardDeclarationClassification;
    use super::SurfaceReferenceMatch;
    use super::classify_outward_declaration;
    use super::declaration_visibility;
    use super::field_type_reference;
    use super::impl_item_surface_reference;
    use super::impl_item_visibility;
    use super::potentially_outward_impl_surface_items_mentioning_name;
    use super::potentially_outward_item_surface_carriers_mentioning_name;

    #[test]
    fn declaration_visibility_keeps_restricted_items_outward() -> anyhow::Result<()> {
        for spelling in ["pub", "pub(crate)", "pub(super)", "pub(in crate::parent)"] {
            let visibility: Visibility = syn::parse_str(spelling)?;
            assert_eq!(
                declaration_visibility(&visibility),
                DeclarationVisibility::Explicit
            );
            let item: Item = syn::parse_str(&format!("{spelling} struct r#type;"))?;
            let OutwardDeclarationClassification::Outward(declaration) =
                classify_outward_declaration(&item)
            else {
                anyhow::bail!("explicit declaration was rejected");
            };
            assert_eq!(declaration.name, "type");
        }
        assert_eq!(
            declaration_visibility(&syn::Visibility::Inherited),
            DeclarationVisibility::Inherited,
        );
        let item: Item = syn::parse_str("struct Private;")?;
        assert!(matches!(
            classify_outward_declaration(&item),
            OutwardDeclarationClassification::NotOutward,
        ));
        Ok(())
    }

    #[test]
    fn trait_contract_and_inherent_visibility_select_the_same_signature_members()
    -> anyhow::Result<()> {
        let members = concat!(
            "fn hidden() -> Target { loop {} } ",
            "pub(crate) fn visible() -> Target { loop {} } ",
            "pub fn body_only() { let _: Target; } ",
            "const VALUE: Target = loop {}; type Alias = Target;"
        );
        let inherent: ItemImpl = syn::parse_str(&format!("impl Owner {{ {members} }}"))?;
        let contract: ItemImpl =
            syn::parse_str(&format!("impl Contract for Owner {{ {members} }}"))?;
        assert_eq!(
            potentially_outward_impl_surface_items_mentioning_name(&inherent, "Target").len(),
            1,
        );
        assert_eq!(
            potentially_outward_impl_surface_items_mentioning_name(&contract, "Target").len(),
            4,
        );
        for (source, visibility, reference) in [
            (
                "fn f() -> Target {}",
                DeclarationVisibility::Inherited,
                SurfaceReferenceMatch::Found,
            ),
            (
                "pub fn f() {}",
                DeclarationVisibility::Explicit,
                SurfaceReferenceMatch::Missing,
            ),
            (
                "make!();",
                DeclarationVisibility::Inherited,
                SurfaceReferenceMatch::Missing,
            ),
        ] {
            let item: ImplItem = syn::parse_str(source)?;
            assert_eq!(impl_item_visibility(&item), visibility);
            assert_eq!(impl_item_surface_reference(&item, "Target"), reference);
        }
        Ok(())
    }

    #[test]
    fn field_selection_keeps_only_visible_type_references_and_their_indices() -> anyhow::Result<()>
    {
        let item: Item = syn::parse_str(concat!(
            "pub struct Owner { hidden: Target, pub visible: Vec<Target>, ",
            "pub unrelated: usize, pub(crate) nested: Option<Target> }"
        ))?;
        let carriers = potentially_outward_item_surface_carriers_mentioning_name(&item, "Target");
        let mut indices = Vec::new();
        for carrier in carriers {
            match carrier {
                ItemSignatureCarrier::Declaration => anyhow::bail!("field became declaration"),
                ItemSignatureCarrier::StructOrUnionField { field, field_index } => {
                    assert_eq!(
                        field_type_reference(field, "Target"),
                        SurfaceReferenceMatch::Found
                    );
                    assert_eq!(
                        field_type_reference(field, "Missing"),
                        SurfaceReferenceMatch::Missing
                    );
                    indices.push(field_index);
                },
            }
        }
        assert_eq!(indices, [1, 3]);
        let private: Item = syn::parse_str("struct Owner(pub Target);")?;
        assert!(
            potentially_outward_item_surface_carriers_mentioning_name(&private, "Target")
                .is_empty()
        );
        Ok(())
    }
}
