//! `Visibility`: per-slot show/hide axis, separate from per-action
//! enabled/disabled (`ShortcutState`).

/// Whether a bar slot renders this frame.
///
/// Hidden slots are dropped from the rendered bar entirely. Use this
/// for data-dependent suppression — e.g. an `Activate` slot that
/// disappears when the underlying list has no selection. Distinct from
/// [`ShortcutState`](super::ShortcutState), which keeps the slot in the
/// bar but grays it out.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Visibility {
    /// Slot is rendered.
    Visible,
    /// Slot is dropped from the bar this frame.
    Hidden,
}
