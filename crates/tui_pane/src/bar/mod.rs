//! Bar primitives + framework bar renderer.
//!
//! The `bar` module owns the public bar surface: leaf primitives
//! ([`BarRegion`], [`BarSlot`], [`ShortcutState`], [`Visibility`])
//! plus [`render`] / [`StatusBar`] — the renderer that the binary
//! drives once per frame.
//!
//! The renderer's contract:
//!
//! 1. Resolve `pane_slots: Vec<RenderedSlot>` for the focused pane. Overlay-first dispatch (Keymap
//!    / Settings overlays read `framework.{keymap,settings}_pane.bar_slots()`); else
//!    [`FocusedPane::App(id)`](crate::FocusedPane::App) flows through
//!    [`Keymap::render_app_pane_bar_slots`](crate::Keymap::render_app_pane_bar_slots); else
//!    [`FocusedPane::Framework(FrameworkFocusId::Toasts)`](crate::FocusedPane::Framework) reads
//!    from `framework.toasts.bar_slots(ctx)`.
//! 2. Walk [`BarRegion::ALL`](crate::BarRegion::ALL) and dispatch to each region module. Each
//!    module owns its own suppression rule based on
//!    [`Framework::focused_pane_mode`](crate::Framework::focused_pane_mode).
//! 3. Concatenate the per-region span vectors into one [`StatusBar`].

mod global_region;
mod nav_region;
mod palette;
mod pane_action_region;
mod region;
mod slot;
mod status_bar;
mod status_line;
mod support;
mod visibility;

pub use palette::BarPalette;
pub use region::BarRegion;
pub use slot::BarSlot;
pub use slot::ShortcutState;
pub use status_bar::StatusBar;
pub use status_line::StatusLine;
pub use status_line::StatusLineGlobal;
pub use status_line::render as render_status_line;
pub use status_line::status_line_global_spans;
pub use visibility::Visibility;

use crate::Action;
use crate::AppContext;
use crate::FocusedPane;
use crate::Framework;
use crate::FrameworkFocusId;
use crate::FrameworkOverlayId;
use crate::Keymap;
use crate::ScopeMap;
use crate::ShortcutState as ShortcutStateAlias;
use crate::Toasts;
use crate::Visibility as VisibilityAlias;
use crate::keymap::RenderedSlot;

/// Resolve the framework's bar for the current frame.
///
/// `focused` is the framework's current focus (overlay open or not),
/// `ctx` is the binary's app state, `keymap` is the live keymap, and
/// `framework` is the framework aggregator. Returns one
/// [`StatusBar`] value the binary draws to its status-line area.
///
/// Mode suppression rules:
///
/// - [`Mode::Static`](crate::Mode::Static) — `Nav` suppressed; `PaneAction` and `Global` render.
/// - [`Mode::Navigable`](crate::Mode::Navigable) — every region renders.
/// - [`Mode::TextInput`](crate::Mode::TextInput) — every region suppressed (the embedded handler
///   owns the keys; advertising globals here would lie about reachability).
/// - `None` (no pane registered for the focused id) — every region suppressed.
#[must_use]
pub fn render<Ctx: AppContext + 'static>(
    focused: &FocusedPane<Ctx::AppPaneId>,
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    framework: &Framework<Ctx>,
    palette: &BarPalette,
) -> StatusBar {
    let pane_slots = pane_slots_for(focused, ctx, keymap, framework);
    let mode = framework.focused_pane_mode(ctx);

    let mut bar = StatusBar::empty();
    for region in BarRegion::ALL {
        match region {
            BarRegion::Nav => {
                bar.nav = nav_region::render::<Ctx>(mode.as_ref(), keymap, &pane_slots, palette);
            },
            BarRegion::PaneAction => {
                bar.pane_action =
                    pane_action_region::render::<Ctx>(mode.as_ref(), &pane_slots, palette);
            },
            BarRegion::Global => {
                bar.global = global_region::render::<Ctx>(mode.as_ref(), keymap, palette);
            },
        }
    }
    bar
}

/// Materialize the focused pane's bar slots, resolved to
/// `Vec<RenderedSlot>`. Overlay-first; otherwise dispatch by
/// [`FocusedPane`].
fn pane_slots_for<Ctx: AppContext + 'static>(
    focused: &FocusedPane<Ctx::AppPaneId>,
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    framework: &Framework<Ctx>,
) -> Vec<RenderedSlot> {
    if let Some(overlay) = framework.overlay() {
        return match overlay {
            FrameworkOverlayId::Keymap => {
                let scope = keymap.keymap_overlay();
                render_overlay_slots(framework.keymap_pane.bar_slots(), scope)
            },
            FrameworkOverlayId::Settings => {
                let scope = keymap.settings_overlay();
                render_overlay_slots(framework.settings_pane.bar_slots(), scope)
            },
        };
    }
    match focused {
        FocusedPane::App(id) => keymap.render_app_pane_bar_slots(*id, ctx),
        FocusedPane::Framework(FrameworkFocusId::Toasts) => {
            let scope = Toasts::<Ctx>::defaults().into_scope_map();
            render_overlay_slots(framework.toasts.bar_slots(ctx), &scope)
        },
    }
}

fn render_overlay_slots<A: Action>(
    slots: Vec<(BarRegion, crate::BarSlot<A>)>,
    scope: &ScopeMap<A>,
) -> Vec<RenderedSlot> {
    slots
        .into_iter()
        .filter_map(|(region, slot)| match slot {
            BarSlot::Single(action) => {
                let key = scope.key_for(action).copied()?;
                Some(RenderedSlot {
                    region,
                    label: action.bar_label(),
                    key,
                    state: ShortcutStateAlias::Enabled,
                    visibility: VisibilityAlias::Visible,
                    secondary_key: None,
                })
            },
            BarSlot::Paired(primary, secondary, label) => {
                let key = scope.key_for(primary).copied()?;
                let secondary_key = scope.key_for(secondary).copied()?;
                Some(RenderedSlot {
                    region,
                    label,
                    key,
                    state: ShortcutStateAlias::Enabled,
                    visibility: VisibilityAlias::Visible,
                    secondary_key: Some(secondary_key),
                })
            },
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests;
