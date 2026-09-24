//! What an app does to carry the favorites overlay: where its
//! [`FavoritesOverlay`] is kept, the app pane id the overlay's keys are
//! registered under, and the helpers its runner hooks call.

use std::time::Instant;

use super::FavoritesOverlay;
use super::FavoritesOverlayFrameOutcome;
use crate::AttractHost;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::Keymap;
use crate::Repaint;

/// An app that carries the favorites overlay on top of its attract screen.
///
/// Naming contract: the app's globals scope
/// ([`KeymapEditContext::AppGlobals`](crate::KeymapEditContext::AppGlobals))
/// must name its save and open actions `save_favorite` and
/// `open_favorites`. The overlay's retry prompts and empty-list hint look
/// both up by those TOML names; an app spelling them otherwise gets the
/// unbound (blank) key label, never an error.
pub trait FavoritesHost: AttractHost {
    /// The app pane id the overlay's `[favorites]` keys are registered under.
    const FAVORITES_PANE: Self::AppPaneId;

    /// The app's favorites overlay.
    fn favorites_overlay(&self) -> &FavoritesOverlay;

    /// The app's favorites overlay, to open, steer and close.
    fn favorites_overlay_mut(&mut self) -> &mut FavoritesOverlay;
}

/// Offer `bind` to an open favorites overlay, for
/// [`TerminalApp::modal_key`](crate::TerminalApp::modal_key).
///
/// The favorites modal owns every key while it is open: a key its scope
/// does not bind is still its own, and disarms a pending delete rather
/// than reaching a global.
pub fn dispatch_favorites_key<A: FavoritesHost>(
    app: &mut A,
    keymap: &Keymap<A>,
    bind: &KeyBind,
) -> KeyOutcome {
    if !app.favorites_overlay().is_open() {
        return KeyOutcome::Unhandled;
    }
    if keymap.dispatch_app_pane(A::FAVORITES_PANE, bind, app) == KeyOutcome::Unhandled {
        app.favorites_overlay_mut().handle_unmapped_key();
    }
    KeyOutcome::Consumed
}

/// Re-mark the row matching the reclamped attract parameters, for
/// [`TerminalApp::resize_settled`](crate::TerminalApp::resize_settled).
///
/// The attract screen reclamps its parameters to the new size, so an
/// open favorites table re-marks which row matches them.
pub fn favorites_resize_settled<A: FavoritesHost>(app: &mut A) {
    if !app.favorites_overlay().is_open() {
        return;
    }
    let current_parameters = app.attract_mut().current_settings().into();
    app.favorites_overlay_mut()
        .refresh_current_parameters(current_parameters);
}

/// Advance a removal fade and commit a finished one to the favorites
/// file, for [`PollWork::poll`](crate::PollWork::poll).
pub fn poll_favorites<A: FavoritesHost>(app: &mut A, now: Instant) -> Repaint {
    match app.favorites_overlay_mut().advance(now) {
        FavoritesOverlayFrameOutcome::Quiet => Repaint::NotNeeded,
        FavoritesOverlayFrameOutcome::Repaint => Repaint::Needed,
        FavoritesOverlayFrameOutcome::CommitRemoval(removal_target) => {
            let result = crate::remove_favorite::<A::Identity>(removal_target.clone());
            app.favorites_overlay_mut()
                .finish_removal(removal_target, result);
            Repaint::Needed
        },
    }
}
