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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::Visibility;

    #[test]
    fn variants_distinct() {
        assert_eq!(Visibility::Visible, Visibility::Visible);
        assert_ne!(Visibility::Visible, Visibility::Hidden);
    }

    #[test]
    fn copy_round_trip() {
        let v = Visibility::Visible;
        let copied = v;
        assert_eq!(v, copied);
    }
}
