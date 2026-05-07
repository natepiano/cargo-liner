//! Bar-slot payloads: per-action units a pane emits to render in the bar.
//!
//! [`BarSlot<A>`] carries either one action ([`Self::Single`]) or two
//! actions joined by a static separator ([`Self::Paired`], used for
//! pairs like `j/k` or `←/→`). [`ShortcutState`] is the orthogonal
//! enabled/disabled axis returned by the pane's
//! `Shortcuts::state` method (added in Phase 7).

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
/// pane's `Shortcuts::label` override (Phase 7).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BarSlot<A> {
    /// One action, one label.
    Single(A),
    /// Two actions joined by a static separator (e.g. `"/"` between
    /// `Up`/`Down` to render `j/k`). The separator is the third field.
    Paired(A, A, &'static str),
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
}
