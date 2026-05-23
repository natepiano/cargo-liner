//! Generic input-event utilities shared across embedding apps.
//!
//! The keyboard dispatch ladder itself stays in each app — every rung
//! (early-return special cases, navigation chord state machine,
//! app-modal overlays) reaches into app-domain state. What lives here
//! is the framework-pure subset: a string label for tracing, the
//! `FocusGained` synthetic-click trick, and small predicates over
//! [`Framework`] state.

use std::sync::Mutex;

use crossterm::event::Event;

use crate::AppContext;
use crate::Framework;
use crate::FrameworkOverlayId;
use crate::GlobalAction;
use crate::KeyBind;
use crate::KeyOutcome;
use crate::ToastCommand;

/// Last known mouse position.
///
/// Updated from every mouse event so a `FocusGained` event can
/// synthesize a click at that position — iTerm2 (and other
/// terminals) consume the mouse-down event that caused the focus
/// change, so the app would otherwise miss it.
static LAST_MOUSE_POS: Mutex<Option<(u16, u16)>> = Mutex::new(None);

/// Record the mouse position observed from the current event.
pub fn record_mouse_pos(column: u16, row: u16) {
    if let Ok(mut last) = LAST_MOUSE_POS.lock() {
        *last = Some((column, row));
    }
}

/// Read the last known mouse position. Used to synthesize a click on
/// `FocusGained`.
#[must_use]
pub fn last_mouse_pos() -> Option<(u16, u16)> {
    LAST_MOUSE_POS.lock().ok().and_then(|guard| *guard)
}

/// Test-only setter for the last-mouse-pos cell. Exposed
/// unconditionally so downstream crates can call it from their own
/// tests without depending on `tui_pane`'s `#[cfg(test)]` items.
#[doc(hidden)]
pub fn set_last_mouse_pos_for_test(pos: Option<(u16, u16)>) {
    if let Ok(mut last) = LAST_MOUSE_POS.lock() {
        *last = pos;
    }
}

/// Stable string label for a crossterm event. Use as the `kind` field
/// when logging slow input events.
#[must_use]
pub fn event_label(event: &Event) -> String {
    match event {
        Event::Key(key) => format!("key:{:?}:{:?}", key.kind, key.code),
        Event::Mouse(mouse) => format!("mouse:{:?}", mouse.kind),
        Event::Resize(width, height) => format!("resize:{width}x{height}"),
        Event::FocusGained => "focus_gained".to_string(),
        Event::FocusLost => "focus_lost".to_string(),
        Event::Paste(text) => format!("paste:{}", text.len()),
    }
}

/// True when `action` is the global "open" key for the currently-open
/// `overlay`.
///
/// Dispatch uses this so pressing the open key while an overlay is
/// already up falls through to the global handler, which toggles the
/// overlay closed — keeping every framework overlay symmetric on
/// open/close regardless of what the overlay's local scope binds.
#[must_use]
pub const fn matches_open_overlay_toggle(
    action: GlobalAction,
    overlay: FrameworkOverlayId,
) -> bool {
    matches!(
        (action, overlay),
        (GlobalAction::OpenKeymap, FrameworkOverlayId::Keymap)
            | (GlobalAction::OpenSettings, FrameworkOverlayId::Settings)
            | (
                GlobalAction::OpenGlobalShortcuts,
                FrameworkOverlayId::GlobalShortcuts
            )
    )
}

/// True when the given framework overlay is currently in a text-input
/// mode (settings editing, keymap binding capture).
///
/// Used to gate the open-overlay-toggle short-circuit — a keypress
/// that would close the overlay must not hijack text input.
#[must_use]
pub const fn overlay_is_in_text_mode<Ctx: AppContext>(
    framework: &Framework<Ctx>,
    overlay: FrameworkOverlayId,
) -> bool {
    match overlay {
        FrameworkOverlayId::Settings => framework.settings_pane.is_editing(),
        FrameworkOverlayId::Keymap => framework.keymap_pane.is_capturing(),
        FrameworkOverlayId::GlobalShortcuts => false,
    }
}

/// Run the toast stack's keyboard handler when focus is on toasts.
///
/// Returns `true` if the toasts pane consumed the key; otherwise the
/// caller continues the dispatch ladder. An emitted
/// [`ToastCommand::Activate`] is routed through
/// [`AppContext::handle_toast_action`].
pub fn dispatch_focused_toasts<Ctx>(ctx: &mut Ctx, bind: &KeyBind) -> bool
where
    Ctx: AppContext,
{
    let (outcome, command) = ctx.framework_mut().toasts.handle_key_command(bind);
    if let ToastCommand::Activate(action) = command {
        ctx.handle_toast_action(action);
    }
    matches!(outcome, KeyOutcome::Consumed)
}
