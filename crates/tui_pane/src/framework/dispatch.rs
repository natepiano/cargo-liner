//! Framework's built-in dispatcher for [`GlobalAction`] variants.
//!
//! Sibling of `framework/mod.rs` so it can reach the `pub(super)`
//! lifecycle setters on [`Framework<Ctx>`] (`request_quit`,
//! `request_restart`) and the `pub(super)` `pane_order()` getter
//! without widening the public surface.

use crate::AppContext;
use crate::FocusedPane;
use crate::FrameworkPaneId;
use crate::GlobalAction;
use crate::Keymap;

/// Dispatch one [`GlobalAction`] through the framework's built-in
/// behavior.
///
/// Each variant maps to a small fixed action:
///
/// - [`GlobalAction::Quit`] / [`GlobalAction::Restart`] flip the matching lifecycle flag on
///   [`Framework<Ctx>`](crate::Framework) and then fire the optional `on_quit` / `on_restart` hook
///   the binary registered on the builder.
/// - [`GlobalAction::NextPane`] / [`GlobalAction::PrevPane`] walk the registered pane order and
///   update focus.
/// - [`GlobalAction::OpenKeymap`] / [`GlobalAction::OpenSettings`] open the matching framework
///   overlay over the focused pane (orthogonal modal layer; focus does not move).
/// - [`GlobalAction::Dismiss`] closes any open overlay first; if no overlay was open, falls through
///   to the binary's optional `dismiss_fallback` hook. Phase 11 inserts the toasts arm in front of
///   the overlay-clear step.
///
/// Re-exported at `crate::framework::dispatch_global` so the keymap's
/// public dispatch entry point can route to it.
pub(crate) fn dispatch_global<Ctx: AppContext>(
    action: GlobalAction,
    keymap: &Keymap<Ctx>,
    ctx: &mut Ctx,
) {
    match action {
        GlobalAction::Quit => {
            ctx.framework_mut().request_quit();
            if let Some(hook) = keymap.on_quit_hook() {
                hook(ctx);
            }
        },
        GlobalAction::Restart => {
            ctx.framework_mut().request_restart();
            if let Some(hook) = keymap.on_restart_hook() {
                hook(ctx);
            }
        },
        GlobalAction::NextPane => focus_step(ctx, 1),
        GlobalAction::PrevPane => focus_step(ctx, -1),
        GlobalAction::OpenKeymap => {
            ctx.framework_mut().open_overlay(FrameworkPaneId::Keymap);
        },
        GlobalAction::OpenSettings => {
            ctx.framework_mut().open_overlay(FrameworkPaneId::Settings);
        },
        GlobalAction::Dismiss => {
            if ctx.framework_mut().close_overlay() {
                return;
            }
            if let Some(hook) = keymap.dismiss_fallback_hook() {
                let _ = hook(ctx);
            }
        },
    }
}

/// Move focus to the next/previous registered app pane. `direction`
/// is `+1` for next, `-1` for prev. No-op when the registered pane
/// order is empty or focus is on a framework overlay.
fn focus_step<Ctx: AppContext>(ctx: &mut Ctx, direction: i32) {
    let panes = ctx.framework().pane_order().to_vec();
    if panes.is_empty() {
        return;
    }
    let current = match ctx.framework().focused() {
        FocusedPane::App(id) => *id,
        FocusedPane::Framework(_) => return,
    };
    let Some(idx) = panes.iter().position(|p| *p == current) else {
        return;
    };
    let len = i32::try_from(panes.len()).unwrap_or(i32::MAX);
    let cur_i = i32::try_from(idx).unwrap_or(0);
    let next_i = ((cur_i + direction).rem_euclid(len)) as usize;
    if let Some(next) = panes.get(next_i).copied() {
        ctx.set_focus(FocusedPane::App(next));
    }
}
