//! [`SettingsNavigation`]: the navigation scope that moves through the
//! settings overlay.
//!
//! The framework asks every app that registers pane shortcuts for a
//! navigation scope, and it is what makes the movement keys
//! rebindable: the action set and the default keys are framework-owned
//! ([`NavAction`]), and this scope supplies the routing. It answers for
//! the surface that is open rather than for the focused pane: an
//! overlay draws over the app without taking focus from it, so
//! `focused` would name the app's pane either way.

use std::marker::PhantomData;

use super::constants::NAVIGATION_SECTION;
use super::step::SettingStep;
use crate::AppContext;
use crate::FocusedPane;
use crate::FrameworkOverlayId;
use crate::NavAction;
use crate::Navigation;

/// An app whose settings overlay the framework's navigation scope can
/// step.
pub trait SettingsHost: AppContext {
    /// Step the settings overlay's selected row one value, then write
    /// and apply the result. A read-only row does nothing.
    fn step_setting(&mut self, step: SettingStep);
}

/// Navigation scope for an app whose only list is the settings overlay.
///
/// Left and Right step the selected value, every other movement moves
/// the selection. Registered with
/// [`KeymapBuilder::register_navigation`](crate::KeymapBuilder::register_navigation).
///
/// Left and right are the value, not the selection: a settings row has
/// no column to move into, so the sideways keys are free to mean "the
/// previous choice" and "the next one".
pub struct SettingsNavigation<A>(PhantomData<fn() -> A>);

impl<A: SettingsHost + 'static> Navigation<A> for SettingsNavigation<A> {
    const SECTION_NAME: &'static str = NAVIGATION_SECTION;

    fn dispatcher() -> fn(NavAction, FocusedPane<A::AppPaneId>, &mut A) {
        |action, _focused, app| {
            if app.framework().overlay() != Some(FrameworkOverlayId::Settings) {
                return;
            }
            match action {
                NavAction::Left => app.step_setting(SettingStep::Prev),
                NavAction::Right => app.step_setting(SettingStep::Next),
                _ => app.framework_mut().settings_pane.navigate(action),
            }
        }
    }
}
