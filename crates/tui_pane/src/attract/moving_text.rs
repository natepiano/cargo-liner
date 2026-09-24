//! The keys that steer the drifting text.
//!
//! [`MovingTextAction`] is the text's own scope, a sibling of
//! [`MovingBandAction`](super::moving_band::MovingBandAction) rather
//! than a superset of it: the two animations bind the same arrows and
//! the same `v` to things of their own, and `keymap.toml` gives each a
//! table. What they share is only what the reader's hands should not
//! have to re-learn between them.
//!
//! The scope is consulted while the attract screen is what the display
//! is showing -- see [`Attract::keyed_mode`](super::Attract::keyed_mode)
//! -- whether it was asked for or came on over an idle grid.

use std::marker::PhantomData;

use crossterm::event::KeyCode;

use super::AttractHost;
use super::constants::ATTRACT_MOVING_TEXT_SCOPE;
use super::constants::ATTRACT_MOVING_TEXT_SECTION;
use crate::Bindings;
use crate::Mode;
use crate::Pane;
use crate::Shortcuts;
use crate::TabStop;

crate::action_enum! {
    /// One thing a key asks the drifting text to do.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum MovingTextAction {
        /// Drift the text left.
        TravelLeft     => ("travel_left",     "Drift the text left");
        /// Drift the text right.
        TravelRight    => ("travel_right",    "Drift the text right");
        /// Drift the text up.
        TravelUp       => ("travel_up",       "Drift the text up");
        /// Drift the text down.
        TravelDown     => ("travel_down",     "Drift the text down");
        /// Speed the text up.
        Faster         => ("faster",          "Speed the text up");
        /// Slow the text down.
        Slower         => ("slower",          "Slow the text down");
        /// Drift the lines as one, or apart.
        CycleDrift     => ("cycle_drift",     "Drift the lines as one, or apart");
        /// Fill the cells with bars, or with characters.
        CycleFill      => ("cycle_fill",      "Fill the cells with bars, or with characters");
        /// Draw the lines' speeds together.
        SpreadNarrower => ("spread_narrower", "Draw the lines' speeds together");
        /// Send the lines' speeds apart.
        SpreadWider    => ("spread_wider",    "Send the lines' speeds apart");
        /// Show the moving band.
        ShowMovingBand => ("show_moving_band", "Show the moving band");
        /// Show the moving text.
        ShowMovingText => ("show_moving_text", "Show the moving text");
        /// Show the pixelate screen.
        ShowPixelate   => ("show_pixelate",    "Show the pixelate screen");
    }
}

/// The keymap scope the drifting text's keys live in.
///
/// A [`Pane`] with no rectangle of its own, for the same reason
/// [`MovingBandPane`](super::MovingBandPane) is one: the attract screen
/// is drawn over the whole terminal, and this exists so the framework
/// has somewhere to hang a scope of its own.
pub struct MovingTextPane<A>(PhantomData<fn() -> A>);

impl<A> Default for MovingTextPane<A> {
    fn default() -> Self { Self(PhantomData) }
}

impl<A: AttractHost> Pane<A> for MovingTextPane<A> {
    const APP_PANE_ID: A::AppPaneId = A::MOVING_TEXT_PANE;

    /// The text is steered, not scrolled, so the status line's
    /// navigation region has nothing to say about it.
    fn mode() -> fn(&A) -> Mode<A> { |_app| Mode::Static }

    /// Never a Tab stop. Tab walks the tile grid, and the grid is what
    /// the attract screen is covering.
    fn tab_stop() -> TabStop<A> { TabStop::never() }
}

impl<A: AttractHost> Shortcuts<A> for MovingTextPane<A> {
    type Actions = MovingTextAction;

    const SCOPE_NAME: &'static str = ATTRACT_MOVING_TEXT_SCOPE;
    const SECTION_NAME: &'static str = ATTRACT_MOVING_TEXT_SECTION;

    /// Every key the band already spends means here what it means
    /// there, so the reader's hands carry across: the arrows point the
    /// way, `<` and `>` slow and speed it with `,` and `.` standing in
    /// for them unshifted, `v` cycles what varies, and `[` and `]` are
    /// how much it varies by.
    ///
    /// `+` and `-` are the exception, and they are unbound rather than
    /// re-purposed. What they do on the band is change how deep it
    /// stands; this fills the window, so there is nothing for them to
    /// mean -- and binding them to something invented for the sake of a
    /// full listing is worse than leaving them where they were.
    ///
    /// `1`, `2` and `3` name the animations in the order they were
    /// written. Every scope binds all three, so each is reachable from
    /// the others and none is a door that only opens one way; pressing
    /// the one already showing does nothing.
    fn defaults() -> Bindings<Self::Actions> { default_bindings() }

    fn dispatcher() -> fn(Self::Actions, &mut A) { dispatch::<A> }
}

/// Run one drifting-text action.
fn dispatch<A: AttractHost>(action: MovingTextAction, app: &mut A) {
    app.attract_mut().moving_text(action);
}

/// The keys the drifting text is steered with until `keymap.toml` says otherwise.
fn default_bindings() -> Bindings<MovingTextAction> {
    crate::bindings! {
        KeyCode::Left => MovingTextAction::TravelLeft,
        KeyCode::Right => MovingTextAction::TravelRight,
        KeyCode::Up => MovingTextAction::TravelUp,
        KeyCode::Down => MovingTextAction::TravelDown,
        ['>', '.'] => MovingTextAction::Faster,
        ['<', ','] => MovingTextAction::Slower,
        'v' => MovingTextAction::CycleDrift,
        't' => MovingTextAction::CycleFill,
        '[' => MovingTextAction::SpreadNarrower,
        ']' => MovingTextAction::SpreadWider,
        '1' => MovingTextAction::ShowMovingBand,
        '2' => MovingTextAction::ShowMovingText,
        '3' => MovingTextAction::ShowPixelate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyBind;

    /// The arrows point the way the text drifts. Read out of the table
    /// the keymap is actually built from, and asserted apart from the
    /// character keys because they are the pair the grid underneath
    /// also spends -- an arrow that failed to resolve here would move a
    /// focus ring instead, which looks like a key doing nothing.
    #[test]
    fn the_arrows_point_the_way_the_text_drifts() {
        let scope = default_bindings().into_scope_map();
        let cases = [
            (KeyCode::Left, MovingTextAction::TravelLeft),
            (KeyCode::Right, MovingTextAction::TravelRight),
            (KeyCode::Up, MovingTextAction::TravelUp),
            (KeyCode::Down, MovingTextAction::TravelDown),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key:?} should steer the text",
            );
        }
    }

    /// Every steering key resolves to the action it is meant to, read
    /// out of the table the keymap is actually built from.
    #[test]
    fn the_steering_keys_resolve_to_their_actions() {
        let scope = default_bindings().into_scope_map();
        let cases = [
            ('v', MovingTextAction::CycleDrift),
            ('t', MovingTextAction::CycleFill),
            ('[', MovingTextAction::SpreadNarrower),
            (']', MovingTextAction::SpreadWider),
            ('>', MovingTextAction::Faster),
            ('<', MovingTextAction::Slower),
            ('1', MovingTextAction::ShowMovingBand),
            ('2', MovingTextAction::ShowMovingText),
            ('3', MovingTextAction::ShowPixelate),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key} should steer the text",
            );
        }
    }

    /// The band's depth keys are the band's. Nothing here stands deeper
    /// or shallower, and a key bound to a no-op would still take it
    /// away from the grid underneath.
    #[test]
    fn the_bands_depth_keys_are_left_unbound() {
        let scope = default_bindings().into_scope_map();

        for key in ['+', '=', '-'] {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                None,
                "{key} has nothing to mean while the text fills the window",
            );
        }
    }
}
