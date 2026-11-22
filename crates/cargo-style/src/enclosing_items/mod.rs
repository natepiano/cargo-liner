//! Read complete item identities and the lines whose owners are proved.

mod item_reader;
mod nested_syntax;

pub(crate) use self::item_reader::EnclosingItem;
pub(crate) use self::item_reader::OwnerBody;
pub(crate) use self::item_reader::UnreadableItems;
pub(crate) use self::item_reader::read;
