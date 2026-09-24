//! What an app does to carry the attract screen: the three app pane ids
//! its animations' keys are registered under, and where its [`Attract`]
//! is kept.

use super::constants::NOTICE_TOAST_MIN_INTERIOR_LINES;
use super::constants::NOTICE_TOAST_VISIBLE;
use super::controller::Attract;
use super::controller::AttractConfigurationRestoreOutcome;
use super::controller::AttractMode;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Keymap;
use crate::TerminalApp;

/// An app that shows the attract screen.
///
/// Each [`AttractMode`] is a keymap scope of its own, so two animations
/// can bind the same key to different things and `keymap.toml` keeps a
/// table for each. A scope hangs off an app pane id, and the app names
/// the three here: the ids of
/// [`MovingBandPane`](super::MovingBandPane),
/// [`MovingTextPane`](super::MovingTextPane) and
/// [`PixelatePane`](super::PixelatePane), which it registers with its
/// keymap. They are panes with no rectangle of their own -- the attract
/// screen is drawn over the whole terminal -- so none of them is ever a
/// Tab stop.
pub trait AttractHost: TerminalApp {
    /// The app pane id the moving band's keys are registered under.
    const MOVING_BAND_PANE: Self::AppPaneId;
    /// The app pane id the drifting text's keys are registered under.
    const MOVING_TEXT_PANE: Self::AppPaneId;
    /// The app pane id the pixelate screen's keys are registered under.
    const PIXELATE_PANE: Self::AppPaneId;

    /// The app's attract screen.
    fn attract(&self) -> &Attract<Self::Probe>;

    /// The app's attract screen, to steer.
    fn attract_mut(&mut self) -> &mut Attract<Self::Probe>;
}

/// The app pane id `mode`'s keys are registered under.
#[must_use]
pub const fn attract_pane_id<A: AttractHost>(mode: AttractMode) -> A::AppPaneId {
    match mode {
        AttractMode::MovingBand => A::MOVING_BAND_PANE,
        AttractMode::MovingText => A::MOVING_TEXT_PANE,
        AttractMode::Pixelate => A::PIXELATE_PANE,
    }
}

/// Offer `bind` to the attract screen's keys, for
/// [`TerminalApp::attract_key`].
///
/// An attract screen that is what the display is showing owns the
/// keyboard, and it owns it ahead of everything else: the keys that
/// steer it are the arrows and `+` `-`, which a grid underneath spends
/// on focus and on opening and closing a tile. A band that could not be
/// steered because a grid nobody can see moved its focus ring would not
/// be steerable at all. Only the keys it actually binds are taken --
/// the rest reach the app's globals and the framework's as usual.
pub fn dispatch_attract_key<A: AttractHost>(
    app: &mut A,
    keymap: &Keymap<A>,
    bind: &KeyBind,
) -> KeyOutcome {
    let Some(mode) = app.attract().keyed_mode() else {
        return KeyOutcome::Unhandled;
    };
    keymap.dispatch_app_pane(attract_pane_id::<A>(mode), bind, app)
}

/// Put back the attract configuration the latest wholesale replacement
/// displaced, and say what happened in a toast.
///
/// For the app global that undoes a randomize or a favorite load. The
/// toast names the animation restored and, where the terminal is too
/// small for some of the restored values, which parameter sets were
/// adjusted to fit it.
pub fn undo_attract_replacement<A: AttractHost>(app: &mut A) {
    let outcome = app
        .attract_mut()
        .restore_configuration_before_last_replacement();
    let (title, body) = match outcome {
        AttractConfigurationRestoreOutcome::NothingToUndo => (
            "Nothing to undo",
            "No attract replacement is available to undo".to_string(),
        ),
        AttractConfigurationRestoreOutcome::RestoredExactly { mode } => (
            "Attract restored",
            format!("{} configuration restored", mode_label(mode)),
        ),
        AttractConfigurationRestoreOutcome::RestoredWithAdjustments {
            mode,
            adjusted_parameter_sets,
        } => (
            "Attract restored with adjustments",
            format!(
                "{} configuration restored; terminal size adjusted {} parameters",
                mode_label(mode),
                adjusted_parameter_sets.names(),
            ),
        ),
    };
    app.framework_mut().toasts.push_timed(
        title,
        &body,
        NOTICE_TOAST_VISIBLE,
        NOTICE_TOAST_MIN_INTERIOR_LINES,
    );
}

/// How a toast names `attract_mode` at the start of a sentence.
pub(crate) const fn mode_label(attract_mode: AttractMode) -> &'static str {
    match attract_mode {
        AttractMode::MovingBand => "Moving band",
        AttractMode::MovingText => "Moving text",
        AttractMode::Pixelate => "Pixelate",
    }
}
