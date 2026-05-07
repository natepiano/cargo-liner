//! Pane identity types: framework's built-in panes and the
//! discriminant covering both framework and binary-supplied panes.
//!
//! Phase 6 of the workspace plan. Used by [`Framework<Ctx>`](crate::Framework)
//! to track focus without naming the binary's concrete pane enum.

/// One of the framework's built-in overlay panes.
///
/// The set is closed — the framework owns these three and binaries
/// cannot extend it. App-side panes carry their own enum, exposed via
/// [`AppContext::AppPaneId`](crate::AppContext::AppPaneId).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FrameworkPaneId {
    /// The keymap viewer overlay.
    Keymap,
    /// The settings overlay.
    Settings,
    /// The toasts (transient notification stack) overlay.
    Toasts,
}

/// Currently focused pane — either one of the binary's app panes or
/// one of the framework's built-in overlays.
///
/// Generic over the binary's pane-id enum so the framework can route
/// focus changes without naming the binary's concrete enum.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FocusedPane<AppPaneId> {
    /// A binary-supplied pane, identified by its `AppPaneId` variant.
    App(AppPaneId),
    /// A framework-supplied overlay pane.
    Framework(FrameworkPaneId),
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::FocusedPane;
    use super::FrameworkPaneId;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum DummyPaneId {
        Foo,
        Bar,
    }

    #[test]
    fn framework_pane_id_variants_distinct() {
        assert_ne!(FrameworkPaneId::Keymap, FrameworkPaneId::Settings);
        assert_ne!(FrameworkPaneId::Settings, FrameworkPaneId::Toasts);
        assert_ne!(FrameworkPaneId::Keymap, FrameworkPaneId::Toasts);
    }

    #[test]
    fn focused_pane_app_arm_round_trips() {
        let f = FocusedPane::App(DummyPaneId::Foo);
        assert_eq!(f, FocusedPane::App(DummyPaneId::Foo));
        assert_ne!(f, FocusedPane::App(DummyPaneId::Bar));
    }

    #[test]
    fn focused_pane_framework_arm_round_trips() {
        let f: FocusedPane<DummyPaneId> = FocusedPane::Framework(FrameworkPaneId::Keymap);
        assert_eq!(f, FocusedPane::Framework(FrameworkPaneId::Keymap));
        assert_ne!(f, FocusedPane::Framework(FrameworkPaneId::Settings));
    }

    #[test]
    fn app_and_framework_arms_are_distinct() {
        let app: FocusedPane<DummyPaneId> = FocusedPane::App(DummyPaneId::Foo);
        let fw: FocusedPane<DummyPaneId> = FocusedPane::Framework(FrameworkPaneId::Keymap);
        assert_ne!(app, fw);
    }
}
