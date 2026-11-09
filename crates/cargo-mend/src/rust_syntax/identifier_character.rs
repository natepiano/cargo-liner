/// Membership in the scanner's identifier characters: Unicode alphanumerics or underscore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdentifierCharacter {
    Member,
    NonMember,
}

pub(crate) fn identifier_character(character: char) -> IdentifierCharacter {
    if character.is_alphanumeric() || character == '_' {
        IdentifierCharacter::Member
    } else {
        IdentifierCharacter::NonMember
    }
}
