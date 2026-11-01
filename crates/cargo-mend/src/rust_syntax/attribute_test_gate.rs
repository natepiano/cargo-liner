use syn::Attribute;
use syn::Ident;

/// Recognition of a direct `#[cfg(test)]` attribute, without interpreting compound gates.
/// `NotDirectCfgTest` makes no claim about whether other attributes permit every build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttributeTestGate {
    DirectCfgTest,
    NotDirectCfgTest,
}

pub(crate) fn attribute_test_gate(attributes: &[Attribute]) -> AttributeTestGate {
    if attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<Ident>()
                .is_ok_and(|predicate_ident| predicate_ident == "test")
    }) {
        AttributeTestGate::DirectCfgTest
    } else {
        AttributeTestGate::NotDirectCfgTest
    }
}
