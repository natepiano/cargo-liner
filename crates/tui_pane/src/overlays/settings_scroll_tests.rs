//! Selection and activation past the first settings page.

#![expect(clippy::expect_used, reason = "tests panic on unexpected values")]

use crossterm::event::KeyCode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;

use super::SettingsPane;
use super::SettingsRowHit;
use crate::FocusedPane;
use crate::FrameworkOverlayId;
use crate::NavAction;
use crate::Navigation;
use crate::SettingsRows;
use crate::draw_settings;

fn draw(pane: &mut SettingsPane, terminal: &mut Terminal<TestBackend>, rows: &SettingsRows<usize>) {
    terminal
        .draw(|frame| draw_settings(frame, pane, rows))
        .expect("draw settings");
}

#[test]
fn settings_scroll_selects_and_activates_rows_below_the_first_page() {
    let mut rows = SettingsRows::new();
    rows.section("Editable values");
    for index in 0..24 {
        rows.text(index, &format!("row {index:02}"), format!("value {index}"));
    }
    let mut pane = SettingsPane::new();
    let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("short terminal");
    draw(&mut pane, &mut terminal, &rows);
    assert!(pane.viewport().len() > pane.viewport().visible_rows());
    pane.navigate(NavAction::End);
    draw(&mut pane, &mut terminal, &rows);
    assert_eq!(pane.viewport().pos(), 23);
    assert!(pane.viewport().scroll_offset() > 0);
    let _ = pane.handle_key(&KeyCode::Enter.into());
    assert!(pane.is_editing());
    pane.enter_browse();
    draw(&mut pane, &mut terminal, &rows);
    let area = pane.viewport().content_area();
    let buffer = terminal.backend().buffer();
    let y = (area.y..area.bottom())
        .find(|&y| {
            let line: String = (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            line.contains("row 22")
        })
        .expect("penultimate row is painted");
    // Navigation changes the live offset; hit testing must still use the painted one.
    pane.navigate(NavAction::Home);
    let hit = pane.row_at(Position::new(area.x + 3, y));
    assert_eq!(hit, SettingsRowHit::Row(22));
    pane.select_row(22);
    assert_eq!(pane.viewport().pos(), 22);
    let _ = pane.handle_key(&KeyCode::Enter.into());
    assert!(pane.is_editing());
}

#[test]
fn settings_scroll_navigation_cancels_the_edit_before_enter_on_another_row() {
    let mut rows = SettingsRows::new();
    rows.text(0, "first", "first value".into());
    rows.text(1, "second", "second value".into());
    let mut values = [String::from("first value"), String::from("second value")];
    let original = values.clone();
    let mut pane = SettingsPane::new();
    let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("short terminal");
    draw(&mut pane, &mut terminal, &rows);
    pane.begin_editing(values[0].clone());
    let _ = pane.handle_text_input(KeyCode::Char('!').into());
    pane.navigate(NavAction::Down);
    assert_eq!(pane.viewport().pos(), 1);
    // Enter commits only in edit mode; in browse mode it opens the new row's value.
    if pane.is_editing() {
        assert_eq!(
            pane.handle_text_input(KeyCode::Enter.into()),
            crate::SettingsCommand::Save
        );
        values[pane.viewport().pos()] = pane.edited_text().to_owned();
        pane.enter_browse();
    } else {
        pane.begin_editing(values[pane.viewport().pos()].clone());
    }
    assert_eq!(
        values, original,
        "navigation must discard the first row's unsaved edit"
    );
    assert_eq!(pane.edit_buffer(), "second value");
}

struct UndrawnSettingsHost {
    framework: crate::Framework<Self>,
    rows:      SettingsRows<usize>,
}

impl crate::AppContext for UndrawnSettingsHost {
    type AppPaneId = ();
    type ToastAction = crate::NoToastAction;

    fn framework(&self) -> &crate::Framework<Self> { &self.framework }
    fn framework_mut(&mut self) -> &mut crate::Framework<Self> { &mut self.framework }
}

impl crate::SettingsHost for UndrawnSettingsHost {
    fn settings_row_count(&self) -> usize { self.rows.selectable_count() }

    fn step_setting(&mut self, _step: crate::SettingStep) {}
}

#[test]
fn settings_scroll_end_before_first_draw_selects_the_last_row() {
    let mut rows = SettingsRows::new();
    rows.section("Editable values");
    for index in 0..24 {
        rows.text(index, &format!("row {index:02}"), format!("value {index}"));
    }
    let mut host = UndrawnSettingsHost {
        framework: crate::Framework::new(FocusedPane::App(())),
        rows,
    };
    host.framework
        .set_overlay_for_test(FrameworkOverlayId::Settings);
    crate::SettingsNavigation::<UndrawnSettingsHost>::dispatcher()(
        NavAction::End,
        FocusedPane::App(()),
        &mut host,
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 8)).expect("short terminal");
    draw(&mut host.framework.settings_pane, &mut terminal, &host.rows);
    assert_eq!(
        host.framework.settings_pane.viewport().pos(),
        23,
        "End before the first draw must reach the last row"
    );
    assert!(host.framework.settings_pane.viewport().scroll_offset() > 0);
}
