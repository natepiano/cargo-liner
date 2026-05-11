//! Tab-cycle metadata for app panes registered with the framework.

use crate::AppContext;

/// Stable tab-cycle ordering policy for an app pane.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TabOrder {
    /// Keep the pane in registration order after explicit tab stops.
    Registration,
    /// Place the pane at the given app-defined order, with
    /// registration order breaking ties.
    Explicit(i16),
    /// Exclude the pane from focus cycling.
    Never,
}

/// Per-pane tab-cycle metadata.
///
/// Stored by [`Framework`](crate::Framework) at registration time and
/// queried live when the user fires
/// [`GlobalAction::NextPane`](crate::GlobalAction::NextPane) or
/// [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane).
pub struct TabStop<Ctx: AppContext> {
    order:       TabOrder,
    is_tabbable: fn(&Ctx) -> bool,
}

impl<Ctx: AppContext> Clone for TabStop<Ctx> {
    fn clone(&self) -> Self { *self }
}

impl<Ctx: AppContext> Copy for TabStop<Ctx> {}

impl<Ctx: AppContext> TabStop<Ctx> {
    /// Keep the pane after explicit tab stops in registration order.
    #[must_use]
    pub const fn registration_order() -> Self {
        Self {
            order:       TabOrder::Registration,
            is_tabbable: always_tabbable::<Ctx>,
        }
    }

    /// Place the pane at `order` when `is_tabbable` returns `true`.
    #[must_use]
    pub const fn ordered(order: i16, is_tabbable: fn(&Ctx) -> bool) -> Self {
        Self {
            order: TabOrder::Explicit(order),
            is_tabbable,
        }
    }

    /// Place the pane at `order` unconditionally.
    #[must_use]
    pub const fn always(order: i16) -> Self {
        Self {
            order:       TabOrder::Explicit(order),
            is_tabbable: always_tabbable::<Ctx>,
        }
    }

    /// Exclude the pane from focus cycling.
    #[must_use]
    pub const fn never() -> Self {
        Self {
            order:       TabOrder::Never,
            is_tabbable: always_tabbable::<Ctx>,
        }
    }

    pub(super) const fn order(&self) -> TabOrder { self.order }

    pub(super) fn is_tabbable(&self, ctx: &Ctx) -> bool { (self.is_tabbable)(ctx) }
}

const fn always_tabbable<Ctx: AppContext>(_: &Ctx) -> bool { true }

pub(super) struct RegisteredTabStop<Ctx: AppContext> {
    id:                 Ctx::AppPaneId,
    registration_index: usize,
    tab_stop:           TabStop<Ctx>,
}

impl<Ctx: AppContext> RegisteredTabStop<Ctx> {
    pub(super) const fn new(
        id: Ctx::AppPaneId,
        registration_index: usize,
        tab_stop: TabStop<Ctx>,
    ) -> Self {
        Self {
            id,
            registration_index,
            tab_stop,
        }
    }

    pub(super) const fn id(&self) -> Ctx::AppPaneId { self.id }

    pub(super) const fn registration_index(&self) -> usize { self.registration_index }

    pub(super) const fn tab_stop(&self) -> TabStop<Ctx> { self.tab_stop }
}
