//! `KeyOutcome`: dispatch result returned by both app- and
//! framework-pane key handlers.
//!
//! Unifies the dispatch loop so the caller reads one return type across
//! [`Keymap::dispatch_app_pane`](super::Keymap::dispatch_app_pane)
//! (app panes) and the framework panes' inherent `handle_key` methods.
//! [`Self::Unhandled`] tells the caller to continue down the dispatch
//! chain (globals → dismiss → fallback).

/// Outcome of dispatching a key against a single scope.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum KeyOutcome {
    /// The key matched a binding in this scope and the dispatcher fired.
    Consumed,
    /// No binding for this key in this scope; the caller continues to
    /// the next stage of the dispatch chain.
    Unhandled,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::KeyOutcome;

    #[test]
    fn variants_are_distinct() {
        assert_ne!(KeyOutcome::Consumed, KeyOutcome::Unhandled);
    }

    #[test]
    fn copy_does_not_consume() {
        let outcome = KeyOutcome::Consumed;
        let copy = outcome;
        assert_eq!(outcome, copy);
    }
}
