//! Mouse wheel routing for the framework's modal overlays.

use ratatui::layout::Position;

use crate::FrameworkOverlayId;
use crate::NavAction;
use crate::ScrollDirection;
use crate::SettingsHost;
use crate::SettingsNavigation;

/// Whether a framework overlay absorbed a wheel event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverlayWheel {
    /// An open overlay consumed the event, including outside its content
    /// or when its selection cannot move further.
    Consumed,
    /// No framework overlay is open; the app can scroll the pane below.
    NoOverlay,
}

/// Scroll the open framework overlay without allowing wheel events to
/// reach the panes below it. Apps apply their scroll inversion before calling.
pub fn wheel_framework_overlay<Ctx: SettingsHost>(
    ctx: &mut Ctx,
    position: Position,
    direction: ScrollDirection,
) -> OverlayWheel {
    match ctx.framework().overlay() {
        Some(FrameworkOverlayId::Settings) => {
            if ctx
                .framework()
                .settings_pane
                .viewport()
                .content_area()
                .contains(position)
            {
                let action = match direction {
                    ScrollDirection::Up => NavAction::Up,
                    ScrollDirection::Down => NavAction::Down,
                };
                SettingsNavigation::navigate(action, ctx);
            }
        },
        Some(FrameworkOverlayId::Keymap) => {
            ctx.framework_mut()
                .keymap_pane
                .viewport_mut()
                .scroll_if_contains(position, direction);
        },
        Some(FrameworkOverlayId::GlobalShortcuts) => {
            ctx.framework_mut()
                .global_shortcuts_pane
                .viewport_mut()
                .scroll_if_contains(position, direction);
        },
        None => return OverlayWheel::NoOverlay,
    }
    OverlayWheel::Consumed
}

#[cfg(test)]
#[allow(
    clippy::unreachable,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::NoToastAction;
    use crate::SettingStep;

    struct TestHost<'a> {
        framework: Framework<Self>,
        row_count: &'a usize,
    }

    impl AppContext for TestHost<'_> {
        type AppPaneId = ();
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }
        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    impl SettingsHost for TestHost<'_> {
        fn settings_row_count(&self) -> usize { *self.row_count }
        fn step_setting(&mut self, _: SettingStep) {
            unreachable!("wheel navigation must move selection without changing a setting");
        }
    }

    fn host(row_count: &usize) -> TestHost<'_> {
        let mut framework = Framework::new(FocusedPane::App(()));
        framework
            .settings_pane
            .set_content_area(Rect::new(4, 4, 8, 2));
        for viewport in [
            framework.keymap_pane.viewport_mut(),
            framework.global_shortcuts_pane.viewport_mut(),
        ] {
            viewport.set_content_area(Rect::new(4, 4, 8, 2));
            viewport.set_len(*row_count);
        }
        TestHost {
            framework,
            row_count,
        }
    }

    fn selections(host: &TestHost<'_>) -> [usize; 3] {
        [
            host.framework.settings_pane.viewport().pos(),
            host.framework.keymap_pane.viewport().pos(),
            host.framework.global_shortcuts_pane.viewport().pos(),
        ]
    }

    #[test]
    fn wheel_without_overlay_returns_no_overlay_and_moves_nothing() {
        let row_count = 3;
        let mut host = host(&row_count);
        assert_eq!(
            wheel_framework_overlay(&mut host, Position::new(5, 5), ScrollDirection::Down),
            OverlayWheel::NoOverlay,
        );
        assert_eq!(selections(&host), [0, 0, 0]);
    }

    #[test]
    fn wheel_overlays_consume_inside_outside_and_at_selection_limits() {
        let row_count = 3;
        for (index, overlay) in [
            FrameworkOverlayId::Settings,
            FrameworkOverlayId::Keymap,
            FrameworkOverlayId::GlobalShortcuts,
        ]
        .into_iter()
        .enumerate()
        {
            let mut host = host(&row_count);
            host.framework.open_overlay(overlay);
            let inside = Position::new(5, 5);
            let outside = Position::new(3, 5);
            for position in [inside, outside] {
                assert_eq!(
                    wheel_framework_overlay(&mut host, position, ScrollDirection::Up),
                    OverlayWheel::Consumed,
                    "{overlay:?} must consume even when the selection cannot move",
                );
                assert_eq!(selections(&host), [0, 0, 0]);
            }
            assert_eq!(
                wheel_framework_overlay(&mut host, outside, ScrollDirection::Down),
                OverlayWheel::Consumed,
            );
            assert_eq!(selections(&host), [0, 0, 0]);
            assert_eq!(
                wheel_framework_overlay(&mut host, inside, ScrollDirection::Down),
                OverlayWheel::Consumed,
            );
            let mut expected = [0, 0, 0];
            expected[index] = 1;
            assert_eq!(selections(&host), expected, "{overlay:?}");
            assert_eq!(
                wheel_framework_overlay(&mut host, outside, ScrollDirection::Down),
                OverlayWheel::Consumed,
            );
            assert_eq!(selections(&host), expected);
            assert_eq!(
                wheel_framework_overlay(&mut host, inside, ScrollDirection::Up),
                OverlayWheel::Consumed,
            );
            assert_eq!(selections(&host), [0, 0, 0]);
        }
    }
}
