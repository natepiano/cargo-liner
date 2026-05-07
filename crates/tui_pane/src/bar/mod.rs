//! Bar primitives: regions, per-action slot payloads, and visibility.

mod region;
mod slot;
mod visibility;

pub use region::BarRegion;
pub use slot::BarSlot;
pub use slot::ShortcutState;
pub use visibility::Visibility;
