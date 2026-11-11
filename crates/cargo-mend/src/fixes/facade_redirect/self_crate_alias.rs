use syn::Ident;
use syn::Item;

/// Whether an item binds an alias for the current crate.
pub(in crate::fixes) enum SelfCrateAlias<'a> {
    Named(&'a Ident),
    Other,
}

impl<'a> SelfCrateAlias<'a> {
    pub(in crate::fixes) fn classify(item: &'a Item) -> Self {
        match item {
            Item::ExternCrate(item) if item.ident == "self" => item
                .rename
                .as_ref()
                .map_or(Self::Other, |(_, name)| Self::Named(name)),
            _ => Self::Other,
        }
    }
}
