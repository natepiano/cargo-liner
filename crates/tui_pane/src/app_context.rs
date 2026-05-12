//! `AppContext` trait: the contract a binary's top-level app type
//! implements so the framework can borrow itself back through it.
//!
//! Resolves the chicken-and-egg between [`Framework<Ctx>`](crate::Framework)
//! requiring `Ctx: AppContext` and `AppContext::framework()` returning
//! `&Framework<Self>` — both types compile only when implemented
//! together.

use core::hash::Hash;

use crate::FocusedPane;
use crate::Framework;

/// The contract a binary's top-level app type implements so the
/// framework can read its own state and update focus through it.
///
/// Required impl is just two getters: [`Self::framework`] and
/// [`Self::framework_mut`]. The third method, [`Self::set_focus`],
/// ships with a default body that delegates to
/// `self.framework_mut().set_focused(focus)` — override only when the
/// binary needs side-effects (logging, telemetry, etc.) on focus
/// change.
pub trait AppContext: Sized {
    /// The binary's pane-id enum (one variant per app-side pane).
    ///
    /// Bounds mirror [`Action`](crate::Action); the `HashMap<AppPaneId,
    /// fn(&Ctx) -> Mode<Ctx>>` registry stored on `Framework<Ctx>` keys
    /// off this type.
    type AppPaneId: Copy + Eq + Hash + 'static;

    /// Domain payload attached to a toast and dispatched on Enter
    /// while focused.
    ///
    /// Apps that do not need toast activation set this to
    /// [`NoToastAction`] and inherit the default
    /// [`Self::handle_toast_action`] body.
    type ToastAction: Clone + 'static;

    /// Borrow the framework state owned by this app.
    fn framework(&self) -> &Framework<Self>;

    /// Mutably borrow the framework state owned by this app.
    fn framework_mut(&mut self) -> &mut Framework<Self>;

    /// Update the focused pane.
    ///
    /// Default body delegates to `self.framework_mut().set_focused(focus)`.
    /// Override only when the binary needs side-effects on focus
    /// change.
    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>) {
        self.framework_mut().set_focused(focus);
    }

    /// Handle a toast activation payload.
    ///
    /// Default body is a no-op for apps that use [`NoToastAction`].
    fn handle_toast_action(&mut self, _action: Self::ToastAction) {}
}

/// Uninhabited filler for apps that have no toast activation.
#[derive(Clone, Debug)]
pub enum NoToastAction {}
