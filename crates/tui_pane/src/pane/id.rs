//! Pane identity types: framework's built-in panes and the
//! discriminant covering both framework and binary-supplied panes.
//!
//! Used by [`Framework<Ctx>`](crate::Framework) to track focus and the
//! open overlay without naming the binary's concrete pane enum. The
//! overlay layer and the focus layer carry separate enums so the type
//! system rules out invalid states by construction:
//!
//! - [`FrameworkOverlayId`] covers the framework overlay panes
//!   ([`Keymap`](FrameworkOverlayId::Keymap), [`Settings`](FrameworkOverlayId::Settings),
//!   [`GlobalShortcuts`](FrameworkOverlayId::GlobalShortcuts)). Toasts is not an overlay, so its
//!   variant is absent.
//! - [`FrameworkFocusId`] covers the framework panes that can be reached as a focus target. Today
//!   only [`Toasts`](FrameworkFocusId::Toasts) qualifies; overlays receive input through the
//!   overlay layer, not the focused-pane chain.

/// One of the framework's overlay panes.
///
/// The set is closed — the framework owns these and binaries
/// cannot extend it. App-side panes carry their own enum, exposed via
/// [`AppContext::AppPaneId`](crate::AppContext::AppPaneId).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FrameworkOverlayId {
    /// The keymap viewer overlay.
    Keymap,
    /// The settings overlay.
    Settings,
    /// The read-only global shortcuts overlay.
    GlobalShortcuts,
}

/// A framework-owned pane that can be reached as a focus target.
///
/// Distinct from [`FrameworkOverlayId`] because the overlay layer and
/// the focus layer are orthogonal. Toasts is the only framework pane
/// that is Tab-focusable; the overlays receive their input through the
/// overlay-layer dispatcher rather than via [`FocusedPane::Framework`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FrameworkFocusId {
    /// The toasts (transient notification stack) focus target. Active
    /// when [`Toasts::has_active`](crate::Toasts::has_active) returns
    /// `true`.
    Toasts,
}

/// Currently focused pane — either one of the binary's app panes or
/// one of the framework's focus-target panes.
///
/// Generic over the binary's pane-id enum so the framework can route
/// focus changes without naming the binary's concrete enum.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FocusedPane<AppPaneId> {
    /// A binary-supplied pane, identified by its `AppPaneId` variant.
    App(AppPaneId),
    /// A framework-supplied focus target.
    Framework(FrameworkFocusId),
}
