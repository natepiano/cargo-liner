//! Framework-owned navigation and focus-cycle vocabulary.
//!
//! [`ListNavigation`] is the direction enum the framework uses internally
//! when it routes resolved navigation actions to a focused list-style
//! pane (today: [`Toasts`](crate::Toasts)). Decouples the framework
//! from any specific binary's `NavigationAction` enum: the binary's
//! [`Navigation`](crate::Navigation) impl translates a resolved action
//! into [`ListNavigation`] via the trait's
//! [`list_navigation`](crate::Navigation::list_navigation) accessor,
//! and the framework operates on the result without naming the
//! binary's enum.
//!
//! [`CycleDirection`] is the closed direction set the focus cycler
//! uses for [`GlobalAction::NextPane`](crate::GlobalAction::NextPane) /
//! [`GlobalAction::PrevPane`](crate::GlobalAction::PrevPane) steps and
//! the corresponding pre-globals consume hook on
//! [`Toasts`](crate::Toasts).

/// Resolved navigation step the framework hands to a focused list-style
/// pane.
///
/// Reused by future framework list panes that need the same vocabulary.
/// Closed enum: the framework owns the directional set; the binary
/// supplies the keys via its [`Navigation`](crate::Navigation) impl.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ListNavigation {
    /// Move the cursor / viewport one step toward the start.
    Up,
    /// Move the cursor / viewport one step toward the end.
    Down,
    /// Jump to the first entry.
    Home,
    /// Jump to the last entry.
    End,
}

/// Direction of a focus-cycle step.
///
/// Used by the focus cycler and by
/// [`Toasts::try_consume_cycle_step`](crate::Toasts::try_consume_cycle_step)
/// so the consume-while-scrollable behavior can act differently on
/// "scroll down before advancing" vs. "scroll up before retreating."
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CycleDirection {
    /// Forward step (`GlobalAction::NextPane`).
    Next,
    /// Backward step (`GlobalAction::PrevPane`).
    Prev,
}
