//! The keys that steer the pixelate screen.
//!
//! [`PixelateAction`] is the screen's own scope, a sibling of
//! [`MovingBandAction`](super::moving_band::MovingBandAction) and
//! [`MovingTextAction`](super::moving_text::MovingTextAction) rather
//! than a superset of either: the three animations bind the same arrows
//! and the same `v` to things of their own, and `keymap.toml` gives
//! each a table.
//!
//! The scope is consulted while the attract screen is what the display
//! is showing -- see [`Attract::keyed_mode`](super::Attract::keyed_mode)
//! -- whether it was asked for or came on over an idle grid.

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::Shortcuts;
use tui_pane::TabStop;

use super::AttractMode;
use crate::app::App;
use crate::app::AppPaneId;
use crate::constants::ATTRACT_PIXELATE_SCOPE;
use crate::constants::ATTRACT_PIXELATE_SECTION;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum PixelateAction {
        SweepLeft      => ("sweep_left",      "Sweep the coarseness left");
        SweepRight     => ("sweep_right",     "Sweep the coarseness right");
        SweepUp        => ("sweep_up",        "Sweep the coarseness up");
        SweepDown      => ("sweep_down",      "Sweep the coarseness down");
        Faster         => ("faster",          "Speed the wave up");
        Slower         => ("slower",          "Slow the wave down");
        Coarser        => ("coarser",         "Draw the blocks bigger");
        Sharper        => ("sharper",         "Draw the blocks smaller");
        CycleResolve   => ("cycle_resolve",   "Cycle how a block gives its cells back");
        CycleFill      => ("cycle_fill",      "Fill the cells solid, or with shading");
        WaveNarrower   => ("wave_narrower",   "Draw the wave narrower");
        WaveWider      => ("wave_wider",      "Draw the wave wider");
        ShowMovingBand => ("show_moving_band", "Show the moving band");
        ShowMovingText => ("show_moving_text", "Show the moving text");
        ShowPixelate   => ("show_pixelate",    "Show the pixelate screen");
    }
}

/// The keymap scope the pixelate screen's keys live in.
///
/// A [`Pane`] with no rectangle of its own, for the same reason
/// [`MovingBandPane`](super::MovingBandPane) is one: the attract screen
/// is drawn over the whole terminal, and this exists so the framework
/// has somewhere to hang a scope of its own.
pub(crate) struct PixelatePane;

impl Pane<App> for PixelatePane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Attract(AttractMode::Pixelate);

    /// The wave is steered, not scrolled, so the status line's
    /// navigation region has nothing to say about it.
    fn mode() -> fn(&App) -> Mode<App> { |_app| Mode::Static }

    /// Never a Tab stop. Tab walks the tile grid, and the grid is what
    /// the attract screen is covering.
    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for PixelatePane {
    type Actions = PixelateAction;

    const SCOPE_NAME: &'static str = ATTRACT_PIXELATE_SCOPE;
    const SECTION_NAME: &'static str = ATTRACT_PIXELATE_SECTION;

    /// Every key the other two spend means here what it means there, so
    /// the reader's hands carry across: the arrows point the way, `<`
    /// and `>` slow and speed it with `,` and `.` standing in for them
    /// unshifted, `v` cycles what varies, `t` changes what a cell is
    /// drawn with, and `[` and `]` are how much it varies by.
    ///
    /// `+` and `-` are bound here where the drifting text leaves them
    /// unbound, and they mean what they mean on the band: more of this,
    /// less of this. What there is more of is the block -- the one
    /// thing on this screen that has a size worth steering -- and `=`
    /// stands in for `+` on the key it shares.
    ///
    /// `1`, `2` and `3` name the animations in the order they were
    /// written. Every scope binds all three, so each is reachable from
    /// the others and none is a door that only opens one way; pressing
    /// the one already showing does nothing.
    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            KeyCode::Left => PixelateAction::SweepLeft,
            KeyCode::Right => PixelateAction::SweepRight,
            KeyCode::Up => PixelateAction::SweepUp,
            KeyCode::Down => PixelateAction::SweepDown,
            ['>', '.'] => PixelateAction::Faster,
            ['<', ','] => PixelateAction::Slower,
            ['+', '='] => PixelateAction::Coarser,
            '-' => PixelateAction::Sharper,
            'v' => PixelateAction::CycleResolve,
            't' => PixelateAction::CycleFill,
            '[' => PixelateAction::WaveNarrower,
            ']' => PixelateAction::WaveWider,
            '1' => PixelateAction::ShowMovingBand,
            '2' => PixelateAction::ShowMovingText,
            '3' => PixelateAction::ShowPixelate,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one pixelate action.
fn dispatch(action: PixelateAction, app: &mut App) { app.attract.pixelate(action); }

#[cfg(test)]
mod tests {
    use tui_pane::KeyBind;

    use super::*;

    /// The arrows point the way the coarseness sweeps. Asserted apart
    /// from the character keys because they are the pair the grid
    /// underneath also spends -- an arrow that failed to resolve here
    /// would move a focus ring instead, which looks like a key doing
    /// nothing.
    #[test]
    fn the_arrows_point_the_way_the_coarseness_sweeps() {
        let scope = PixelatePane::defaults().into_scope_map();
        let cases = [
            (KeyCode::Left, PixelateAction::SweepLeft),
            (KeyCode::Right, PixelateAction::SweepRight),
            (KeyCode::Up, PixelateAction::SweepUp),
            (KeyCode::Down, PixelateAction::SweepDown),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key:?} should sweep the coarseness",
            );
        }
    }

    /// Every steering key resolves to the action it is meant to, read
    /// out of the table the keymap is actually built from.
    #[test]
    fn the_steering_keys_resolve_to_their_actions() {
        let scope = PixelatePane::defaults().into_scope_map();
        let cases = [
            ('v', PixelateAction::CycleResolve),
            ('t', PixelateAction::CycleFill),
            ('[', PixelateAction::WaveNarrower),
            (']', PixelateAction::WaveWider),
            ('+', PixelateAction::Coarser),
            ('=', PixelateAction::Coarser),
            ('-', PixelateAction::Sharper),
            ('>', PixelateAction::Faster),
            ('<', PixelateAction::Slower),
            ('1', PixelateAction::ShowMovingBand),
            ('2', PixelateAction::ShowMovingText),
            ('3', PixelateAction::ShowPixelate),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key} should steer the pixelate screen",
            );
        }
    }
}
