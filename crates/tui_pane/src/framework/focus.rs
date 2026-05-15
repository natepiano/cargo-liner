//! Focus routing helpers used by the framework's global-action handler.
//!
//! Each fn takes `&mut Ctx` so it can route through
//! [`AppContext::set_focus`](crate::AppContext::set_focus): binaries
//! that override `set_focus` (logging, telemetry, the Focus subsystem's
//! overlay-return memory) observe every framework focus change.

use crate::AppContext;
use crate::CycleDirection;
use crate::FocusedPane;
use crate::FrameworkFocusId;

/// Reconcile focus after a focused-toast dismiss or a prune empties
/// the active set.
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
/// live app tab stops plus, when [`crate::Toasts::has_active`] returns
/// `true`, [`FocusedPane::Framework`] with [`FrameworkFocusId::Toasts`]
/// appended at the end.
///
/// On entry into Toasts focus, the manager's viewport is reset to the
/// first or last toast based on direction.
pub(super) fn focus_step<Ctx: AppContext>(ctx: &mut Ctx, direction: CycleDirection) {
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
/// [`crate::Toasts::has_active`] returns `true`.
fn focus_cycle<Ctx: AppContext>(ctx: &Ctx) -> Vec<FocusedPane<Ctx::AppPaneId>> {
    ctx.framework().live_focus_cycle(ctx)
}
