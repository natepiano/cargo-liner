//! Bar-slot payloads: per-action units a pane emits to render in the bar.
//!
//! [`BarSlot<A>`] carries either one action ([`Self::Single`]) or two
//! actions rendered with one shared label ([`Self::Paired`], used for
//! pairs like `j/k nav` or `←/→ expand`). [`ShortcutState`] is the orthogonal
//! enabled/disabled axis returned by the pane's `Shortcuts::state`
//! method.

/// Whether a bar slot is currently active or greyed out.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ShortcutState {
    /// Active — the action will fire when its key is pressed.
    Enabled,
    /// Greyed out — the action is registered but not currently usable.
    Disabled,
}

/// Per-slot payload emitted by a pane and consumed by the bar renderer.
///
/// Generic over an action type `A` (typically `A: Action`); the
/// renderer pairs each action with its `Action::bar_label` and the
/// pane's `Shortcuts::label` override.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BarSlot<A> {
    /// One action, one label.
    Single(A),
    /// Two actions rendered as a paired key row with one shared label
    /// (e.g. `Up` / `Down` render as `j/k nav` when the third field is
    /// `"nav"`).
    Paired(A, A, &'static str),
}

impl<A: Copy> BarSlot<A> {
    /// The slot's primary action — the one used for label, key, and
    /// state lookup.
    ///
    /// `Single(a)` returns `a`; `Paired(a, _, _)` returns `a`. The
    /// second action in a paired slot is rendered alongside as the
    /// "alternate" indicator and does not get a separate state lookup.
    #[must_use]
    pub const fn primary(self) -> A {
        match self {
            Self::Single(a) | Self::Paired(a, _, _) => a,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::BarSlot;
    use super::ShortcutState;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum DummyAction {
        Up,
        Down,
    }

    #[test]
    fn single_construction() {
        let slot = BarSlot::Single(DummyAction::Up);
        assert_eq!(slot, BarSlot::Single(DummyAction::Up));
    }

    #[test]
    fn paired_construction() {
        let slot = BarSlot::Paired(DummyAction::Up, DummyAction::Down, "/");
        assert_eq!(
            slot,
            BarSlot::Paired(DummyAction::Up, DummyAction::Down, "/")
        );
    }

    #[test]
    fn shortcut_state_distinct() {
        assert_ne!(ShortcutState::Enabled, ShortcutState::Disabled);
    }

    #[test]
    fn single_primary_returns_inner_action() {
        let slot = BarSlot::Single(DummyAction::Up);
        assert_eq!(slot.primary(), DummyAction::Up);
    }

    #[test]
    fn paired_primary_returns_first_action() {
        let slot = BarSlot::Paired(DummyAction::Up, DummyAction::Down, "/");
        assert_eq!(slot.primary(), DummyAction::Up);
    }
}
