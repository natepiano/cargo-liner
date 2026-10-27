//! Fallback status-line coverage measured from the actual rendered cells.
#![expect(clippy::expect_used, reason = "render fixture setup must succeed")]
use std::sync::Arc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::Action;
use crate::AppContext;
use crate::Appearance;
use crate::Bindings;
use crate::FocusedPane;
use crate::Framework;
use crate::FrameworkOverlayId;
use crate::Globals;
use crate::Keymap;
use crate::NoToastAction;
use crate::ScanIndicator;
use crate::ShortcutState;
use crate::StatusLine;
use crate::StatusLineGlobal;
use crate::bar;
use crate::bar::BarPalette;
#[path = "status_bar_contrast_support.rs"]
mod contrast;
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum PaneId {
    Main,
}
struct BarApp {
    framework: Framework<Self>,
}
impl AppContext for BarApp {
    type AppPaneId = PaneId;
    type ToastAction = NoToastAction;
    fn framework(&self) -> &Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}
crate::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum BarAction {
        Enabled => ("enabled", "enabled", "Enabled action");
        Disabled => ("disabled", "disabled", "Disabled action");
    }
}
struct BarGlobals;
impl Globals<BarApp> for BarGlobals {
    type Actions = BarAction;
    fn render_order() -> &'static [BarAction] { BarAction::ALL }
    fn defaults() -> Bindings<BarAction> {
        crate::bindings! { 'e' => BarAction::Enabled, 'd' => BarAction::Disabled }
    }
    fn dispatcher() -> fn(BarAction, &mut BarApp) { |_, _| {} }
}
fn check(appearance: Appearance) {
    let previous = crate::theme();
    crate::set_active_theme(Arc::new(crate::fallback_theme(appearance)));
    let mut app = BarApp {
        framework: Framework::new(FocusedPane::App(PaneId::Main)),
    };
    app.framework
        .set_overlay_for_test(FrameworkOverlayId::GlobalShortcuts);
    let keymap = Keymap::builder()
        .register_globals::<BarGlobals>()
        .expect("bar globals")
        .build()
        .expect("bar keymap");
    let globals = [
        StatusLineGlobal::app(BarAction::Enabled),
        StatusLineGlobal::app(BarAction::Disabled).with_state(ShortcutState::Disabled),
    ];
    let status = StatusLine::new(65, ScanIndicator::Shown, &[], &globals);
    let mut terminal = Terminal::new(TestBackend::new(240, 1)).expect("bar terminal");
    terminal
        .draw(|frame| {
            bar::render_status_line::<BarApp, BarGlobals>(
                frame,
                frame.area(),
                &app,
                &keymap,
                &app.framework,
                &BarPalette::themed(),
                &status,
            );
        })
        .expect("bar frame");
    let failures = contrast::failures(
        terminal.backend().buffer(),
        &["scanning", "Uptime:", "1m", "e enabled", "d disabled"],
        &format!("fallback {appearance:?}"),
    );
    let drawn_ground = contrast::rgb(terminal.backend().buffer()[(0, 0)].bg);
    crate::set_active_theme(previous);
    let expected_ground = match appearance {
        Appearance::Dark => [60, 60, 60],
        Appearance::Light => [220, 220, 220],
    };
    assert_eq!(drawn_ground, expected_ground, "fallback bar ground");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[test]
fn fallback_dark_bar_keeps_enabled_disabled_activity_label_and_value_readable() {
    check(Appearance::Dark);
}
#[test]
fn fallback_light_bar_keeps_enabled_disabled_activity_label_and_value_readable() {
    check(Appearance::Light);
}
