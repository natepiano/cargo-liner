use crate::AppContext;
use crate::CycleDirection;
use crate::keymap::KeyBind;

/// `fn` pointer stored per registered pane to query the pane's
/// current input mode.
pub(crate) type ModeQuery<Ctx> = fn(&Ctx) -> Mode<Ctx>;

/// `fn` pointer stored per registered pane that owns a focus ring of
/// its own, offered each Tab step before focus leaves the pane.
pub(crate) type CycleStep<Ctx> = fn(&mut Ctx, CycleDirection) -> bool;

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
