//! `Pane<Ctx>`: per-pane identity and input-mode surface.
//!
//! Supertrait of [`Shortcuts<Ctx>`](crate::Shortcuts). Carries the
//! pane's stable [`Self::APP_PANE_ID`] and a `fn(&Ctx) -> Mode<Ctx>`
//! query the framework stores in its per-pane registry. Pane identity
//! and mode are kept separate from shortcut configuration so non-shortcut
//! framework consumers (e.g. text-input routing) can depend on
//! `Pane<Ctx>` alone.

use crate::AppContext;
use crate::TabStop;
use crate::keymap::KeyBind;

/// `fn` pointer stored per registered pane to query the pane's
/// current input mode.
pub(crate) type ModeQuery<Ctx> = fn(&Ctx) -> Mode<Ctx>;

/// Per-pane identity + input mode. Implemented by every app pane type.
///
/// The framework keys per-pane registries on
/// [`AppContext::AppPaneId`](crate::AppContext::AppPaneId), which is
/// why each impl declares its [`Self::APP_PANE_ID`] variant.
/// [`Self::mode`] returns a `fn` pointer so the framework can store it
/// keyed by `AppPaneId` without lifetime grief.
///
/// `'static` is required because the framework keys its registries on
/// `TypeId<P>` and stores `fn` pointers — both demand `'static`.
pub trait Pane<Ctx: AppContext>: 'static {
    /// Stable per-pane identity used by the framework's per-pane
    /// query registry. The trait covers app panes only — framework
    /// panes (Keymap, Settings, Toasts) are special-cased — so the
    /// variant is always an `AppPaneId`.
    const APP_PANE_ID: Ctx::AppPaneId;

    /// Pane's current input mode (`Navigable` / `Static` / `TextInput`).
    /// Drives bar-region suppression, the structural Esc gate, and
    /// per-key text-input routing.
    ///
    /// Returns `fn(&Ctx) -> Mode<Ctx>` so the framework can store the
    /// pointer in its per-pane registry, keyed by `AppPaneId`. The
    /// framework holds `&Ctx` and an `AppPaneId` at query time, never a
    /// typed `&PaneStruct`, so the closure does the navigation from
    /// `Ctx` to whatever pane state determines the mode.
    ///
    /// Default returns [`Mode::Navigable`]. Panes whose mode varies
    /// with `Ctx` state override.
    #[must_use]
    fn mode() -> fn(&Ctx) -> Mode<Ctx> { |_ctx| Mode::Navigable }

    /// Pane's tab-cycle metadata. Defaults to registration order and
    /// always reachable. Apps override when pane order must be stable
    /// independent of registration, or when runtime state can hide a
    /// pane from `NextPane` / `PrevPane` traversal.
    #[must_use]
    fn tab_stop() -> TabStop<Ctx> { TabStop::registration_order() }
}

/// How a pane consumes keyboard input.
///
/// Controls which bar regions are emitted for the pane and whether the
/// keymap arbitration short-circuits navigation/global keys.
/// [`Self::TextInput`] bundles the per-key handler in the variant so
/// that a text-input pane without a handler is unrepresentable.
///
/// Not `PartialEq` / `Eq` / `Hash` — the [`Self::TextInput`] payload
/// is a `fn` pointer, which does not compare cleanly across instances.
/// Tests use `matches!` instead of `==`.
#[derive(Clone, Copy, Debug)]
pub enum Mode<Ctx: AppContext> {
    /// Static (non-cursor) pane — `PaneAction` and `Global` slots
    /// render; `Nav` slots are suppressed.
    Static,
    /// Standard navigable pane — `Nav`, `PaneAction`, and `Global`
    /// slots all render and dispatch.
    Navigable,
    /// Active text-entry mode — character keys are routed to the
    /// embedded handler, only the dismiss / commit globals remain
    /// reachable. The bundled `fn(KeyBind, &mut Ctx)` makes
    /// "`TextInput` pane without handler" unrepresentable.
    TextInput(fn(KeyBind, &mut Ctx)),
}
