//! Bar primitives: regions, per-action slot payloads, and input mode.

mod region;
mod slot;

pub use region::BarRegion;
pub use slot::BarSlot;
pub use slot::ShortcutState;

/// How a pane consumes keyboard input.
///
/// Controls which bar regions are emitted for the pane and whether the
/// keymap arbitration short-circuits navigation/global keys. Returned
/// by `Shortcuts::input_mode`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum InputMode {
    /// Standard navigable pane — `Nav`, `PaneAction`, and `Global`
    /// slots all render and dispatch.
    Navigable,
    /// Static (non-cursor) pane — `PaneAction` and `Global` slots
    /// render; `Nav` slots are suppressed.
    Static,
    /// Active text-entry mode — character keys are routed to the pane,
    /// only the dismiss / commit globals remain reachable.
    TextInput,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::InputMode;

    #[test]
    fn variants_distinct() {
        assert_ne!(InputMode::Navigable, InputMode::Static);
        assert_ne!(InputMode::Static, InputMode::TextInput);
        assert_ne!(InputMode::Navigable, InputMode::TextInput);
    }
}
