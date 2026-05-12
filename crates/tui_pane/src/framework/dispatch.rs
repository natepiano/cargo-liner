//! Framework's built-in dispatcher for [`GlobalAction`] variants and
//! the focus-cycle helpers that act over `&mut Ctx`.
//!
//! Sibling of `framework/mod.rs` so it can reach the `pub(super)`
//! lifecycle setters on [`Framework<Ctx>`] (`request_quit`,
//! `request_restart`) and the `pub(super)` overlay setter
//! (`open_overlay`) without widening the public surface.
//!
//! The dismiss chain and the focus reconciler live here as **free fns**
//! over `&mut Ctx` so they can route through
//! [`AppContext::set_focus`](crate::AppContext::set_focus). A method
//! on `Framework<Ctx>` could not — the framework borrow holds `&mut
//! Framework`, with no path back to `&mut Ctx`. Routing through
//! `ctx.set_focus(...)` lets binaries that override
//! [`AppContext::set_focus`](crate::AppContext::set_focus) (logging,
//! telemetry, the Focus subsystem's overlay-return memory) observe
//! every framework focus change.

use crate::AppContext;
use crate::CycleDirection;
use crate::FocusedPane;
use crate::FrameworkFocusId;
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
        GlobalAction::NextPane => focus_step(ctx, CycleDirection::Next),
        GlobalAction::PrevPane => focus_step(ctx, CycleDirection::Prev),
        GlobalAction::OpenKeymap => {
            ctx.framework_mut().open_overlay(FrameworkOverlayId::Keymap);
        },
        GlobalAction::OpenSettings => {
            ctx.framework_mut()
                .open_overlay(FrameworkOverlayId::Settings);
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
        reconcile_focus_after_toast_change(ctx);
        return true;
    }
    fallback.is_some_and(|hook| hook(ctx))
}

/// Reconcile focus after a focused-toast dismiss or a Phase-22 prune
/// empties the active set.
///
/// No-op when current focus is not on Toasts, or when toasts are still
/// active. When focus is on Toasts and the active set is empty, move
/// focus to the first live app tab stop. Routes through
/// [`AppContext::set_focus`] so binary-side overrides observe the
/// transition.
pub(crate) fn reconcile_focus_after_toast_change<Ctx: AppContext>(ctx: &mut Ctx) {
    {
        let framework = ctx.framework();
        if !matches!(
            framework.focused(),
            FocusedPane::Framework(FrameworkFocusId::Toasts)
        ) {
            return;
        }
        if framework.toasts.has_active() {
            return;
        }
    }
    let target = focus_cycle(ctx).first().copied();
    if let Some(target) = target {
        ctx.set_focus(target);
    }
}

/// Move focus to the next/previous step in the cycle. The cycle is the
/// live app tab stops plus, when [`Toasts::has_active`] returns
/// `true`, [`Framework(FrameworkFocusId::Toasts)`] appended at the end.
///
/// On entry into Toasts focus, the manager's viewport is reset to the
/// first or last toast based on direction.
fn focus_step<Ctx: AppContext>(ctx: &mut Ctx, direction: CycleDirection) {
    let current = *ctx.framework().focused();
    if matches!(current, FocusedPane::Framework(FrameworkFocusId::Toasts))
        && ctx.framework_mut().toasts.try_consume_cycle_step(direction)
    {
        return;
    }

    let cycle = focus_cycle(ctx);
    if cycle.is_empty() {
        return;
    }

    let next = cycle.iter().position(|p| *p == current).map_or_else(
        || match direction {
            CycleDirection::Next => cycle[0],
            CycleDirection::Prev => cycle[cycle.len() - 1],
        },
        |idx| match direction {
            CycleDirection::Next => cycle[(idx + 1) % cycle.len()],
            CycleDirection::Prev => cycle[(idx + cycle.len() - 1) % cycle.len()],
        },
    );

    let entering_toasts = matches!(next, FocusedPane::Framework(FrameworkFocusId::Toasts))
        && !matches!(current, FocusedPane::Framework(FrameworkFocusId::Toasts));
    ctx.set_focus(next);
    if entering_toasts {
        match direction {
            CycleDirection::Next => ctx.framework_mut().toasts.reset_to_first(),
            CycleDirection::Prev => ctx.framework_mut().toasts.reset_to_last(),
        }
    }
}

/// Build the focus cycle for the current framework state. Live app tab
/// stops come first; Toasts is appended when
/// [`Toasts::has_active`](crate::Toasts::has_active) returns `true`.
fn focus_cycle<Ctx: AppContext>(ctx: &Ctx) -> Vec<FocusedPane<Ctx::AppPaneId>> {
    ctx.framework().live_focus_cycle(ctx)
}
