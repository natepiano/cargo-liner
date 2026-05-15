//! [`GlobalAction`] handler. Sibling of `framework/mod.rs` so it can
//! reach the `pub(super)` lifecycle setters on [`Framework<Ctx>`]
//! (`request_quit`, `request_restart`) and the `pub(super)` overlay
//! setter (`open_overlay`) without widening the public surface.
//!
//! `dismiss_chain` lives here because [`GlobalAction::Dismiss`] is its
//! only entry point. Both are free fns over `&mut Ctx` so they can
//! route through [`AppContext::set_focus`](crate::AppContext::set_focus):
//! a method on `Framework<Ctx>` could not — the framework borrow holds
//! `&mut Framework`, with no path back to `&mut Ctx`. Routing through
//! `ctx.set_focus(...)` lets binaries that override
//! [`AppContext::set_focus`](crate::AppContext::set_focus) (logging,
//! telemetry, the Focus subsystem's overlay-return memory) observe
//! every framework focus change.

use super::focus;
use crate::AppContext;
use crate::CycleDirection;
use crate::FrameworkOverlayId;
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
/// - [`GlobalAction::NextPane`] / [`GlobalAction::PrevPane`] walk the focus cycle (registered app
///   panes plus, when active, [`Toasts`](crate::Toasts)) and update focus.
/// - [`GlobalAction::OpenKeymap`] / [`GlobalAction::OpenSettings`] open the matching framework
///   overlay over the focused pane (orthogonal modal layer; focus does not move).
/// - [`GlobalAction::Dismiss`] runs [`dismiss_chain`] (focused-toast pop → close overlay → fall
///   through to the binary's optional `dismiss_fallback` hook).
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
        GlobalAction::NextPane => focus::focus_step(ctx, CycleDirection::Next),
        GlobalAction::PrevPane => focus::focus_step(ctx, CycleDirection::Prev),
        GlobalAction::OpenKeymap => {
            let framework = ctx.framework_mut();
            if framework.overlay() == Some(FrameworkOverlayId::Keymap) {
                let _ = framework.close_overlay();
            } else {
                framework.open_overlay(FrameworkOverlayId::Keymap);
            }
        },
        GlobalAction::OpenSettings => {
            let framework = ctx.framework_mut();
            if framework.overlay() == Some(FrameworkOverlayId::Settings) {
                let _ = framework.close_overlay();
            } else {
                framework.open_overlay(FrameworkOverlayId::Settings);
            }
        },
        GlobalAction::Dismiss => {
            let _ = dismiss_chain(ctx, keymap.dismiss_fallback_hook());
        },
    }
}

/// Run the framework dismiss chain and reconcile focus.
///
/// 1. Call `framework.dismiss_framework()` (focused-toast → close overlay). If something was
///    dismissed, the framework borrow drops and we route focus repair through
///    [`AppContext::set_focus`] so binary-side overrides observe the transition.
/// 2. Otherwise, fall through to the binary's `dismiss_fallback` hook.
///
/// `pub(crate)` so [`crate::Keymap::dispatch_framework_global`] can
/// call it via [`dispatch_global`]; the binary cannot bypass this
/// routing.
pub(crate) fn dismiss_chain<Ctx: AppContext>(
    ctx: &mut Ctx,
    fallback: Option<fn(&mut Ctx) -> bool>,
) -> bool {
    if ctx.framework_mut().dismiss_framework() {
        focus::reconcile_focus_after_toast_change(ctx);
        return true;
    }
    fallback.is_some_and(|hook| hook(ctx))
}
