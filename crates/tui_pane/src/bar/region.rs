//! `BarRegion`: which region of the bottom bar a slot belongs to.
//!
//! Panes emit slots tagged with one of three regions; the renderer
//! groups slots by region in `ALL` order before laying them out.

/// Bar region a [`BarSlot`](super::BarSlot) belongs to.
///
/// Three regions are rendered in fixed declaration order:
/// [`Self::Nav`] → [`Self::PaneAction`] → [`Self::Global`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BarRegion {
    /// Navigation slots (cursor movement, selection).
    Nav,
    /// Pane-specific action slots.
    PaneAction,
    /// Application-wide global slots.
    Global,
}

impl BarRegion {
    /// Every region in canonical render order.
    pub const ALL: &'static [Self] = &[Self::Nav, Self::PaneAction, Self::Global];
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::BarRegion;

    #[test]
    fn all_in_canonical_order() {
        assert_eq!(
            BarRegion::ALL,
            &[BarRegion::Nav, BarRegion::PaneAction, BarRegion::Global]
        );
    }

    #[test]
    fn copy_eq_hash_round_trip() {
        let r = BarRegion::Nav;
        let copied = r;
        assert_eq!(r, copied);
        assert_eq!(BarRegion::Global, BarRegion::Global);
        assert_ne!(BarRegion::Nav, BarRegion::PaneAction);
    }
}
