//! Settings navigation follows rendered lines, including wrapped account diagnostics.

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::BTreeSet;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Position;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use tui_pane::AppContext;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::FrameworkHit;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::GlobalAction;
    use tui_pane::Keymap;
    use tui_pane::NavAction;
    use tui_pane::NoToastAction;
    use tui_pane::PaneFocusState;
    use tui_pane::SettingsLineTarget;
    use tui_pane::SettingsPane;
    use tui_pane::SettingsRenderOptions;
    use tui_pane::SettingsRow;
    use tui_pane::SettingsRowHit;
    use tui_pane::SettingsRowPayload;
    use tui_pane::SettingsSelectionLine;

    struct SettingsApp {
        framework: Framework<Self>,
    }

    impl AppContext for SettingsApp {
        type AppPaneId = ();
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }

        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    fn render(pane: &mut SettingsPane, rows: &[SettingsRow], area: Rect) -> Vec<String> {
        let rendered = pane.render_rows(rows, options(usize::from(area.width)));
        let viewport = pane.viewport_mut();
        viewport.set_len(rendered.selectable_count);
        viewport.set_content_area(area);
        viewport.set_viewport_rows(usize::from(area.height));
        pane.update_scroll();
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom()))
            .expect("create settings terminal");
        terminal
            .draw(|frame| pane.render_lines(frame, rendered.lines.clone()))
            .expect("draw settings content");
        rendered.lines.iter().map(ToString::to_string).collect()
    }

    fn visible(pane: &SettingsPane, lines: &[String]) -> String {
        lines
            .iter()
            .skip(pane.viewport().scroll_offset())
            .take(usize::from(pane.viewport().content_area().height))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn options(content_width: usize) -> SettingsRenderOptions<'static> {
        SettingsRenderOptions {
            focus: PaneFocusState::Active,
            inline_error: None,
            content_width,
            section_header_indent: "",
            section_item_indent: "",
            title_style: Style::default(),
            label_style: Style::default(),
            muted_style: Style::default(),
            success_style: Style::default(),
            error_style: Style::default(),
            inline_error_style: Style::default(),
            active_style: Style::default(),
            remembered_style: Style::default(),
            hovered_style: Style::default(),
        }
    }

    fn account_rows(count: usize) -> Vec<SettingsRow> {
        let mut rows = vec![SettingsRow::section("Capture")];
        rows.extend(
            (0..count)
                .map(|index| SettingsRow::value(index, "account", format!("account-{index:02}"))),
        );
        rows.push(SettingsRow::section("Commands"));
        rows.push(SettingsRow::value(count, "excluded", "nextest"));
        rows
    }

    fn diagnostics() -> Vec<String> {
        (0..18)
            .map(|index| format!("diagnostic-{index:02}"))
            .collect()
    }

    #[test]
    fn settings_pane_can_be_constructed_in_a_const_binding() {
        const PANE: SettingsPane = SettingsPane::new();

        let pane = PANE;
        assert_eq!(pane.viewport().pos(), 0);
        assert_eq!(pane.line_target(0), SettingsLineTarget::OutsideContent);
        assert_eq!(pane.row_at(Position::new(0, 0)), SettingsRowHit::Missed);
    }

    #[test]
    fn prepared_settings_content_does_not_accept_clicks_before_first_draw() {
        let mut pane = SettingsPane::new();
        let area = Rect::new(7, 5, 28, 3);
        let rows = [
            SettingsRow::value(0, "account", "account-00"),
            SettingsRow::value(1, "excluded", "nextest"),
        ];
        let rendered = pane.render_rows(&rows, options(usize::from(area.width)));
        assert_eq!(rendered.selectable_count, 2);
        assert_eq!(
            pane.line_target(0),
            SettingsLineTarget::Row(SettingsRowPayload::new(0))
        );
        let viewport = pane.viewport_mut();
        viewport.set_content_area(area);
        viewport.set_viewport_rows(usize::from(area.height));
        pane.select_row(1);

        assert_eq!(
            pane.row_at(Position::new(area.x + 1, area.y)),
            SettingsRowHit::Missed,
            "layout and scroll updates have not painted any settings row"
        );
        assert_eq!(pane.viewport().pos(), 1);
    }

    #[test]
    fn click_after_down_uses_the_drawn_offset_until_the_next_repaint() {
        let mut pane = SettingsPane::new();
        let area = Rect::new(7, 5, 13, 3);
        let rows = [
            SettingsRow::value(0, "row", "part-0 part-1 part-2 part-3"),
            SettingsRow::value(1, "row", "next"),
            SettingsRow::value(2, "row", "last"),
        ];
        let rendered = pane.render_rows(&rows, options(usize::from(area.width)));
        let viewport = pane.viewport_mut();
        viewport.set_len(rendered.selectable_count);
        viewport.set_content_area(area);
        viewport.set_viewport_rows(usize::from(area.height));
        pane.select_row(1);
        let click = Position::new(area.x + 1, area.y + 1);
        assert_eq!(pane.row_at(click), SettingsRowHit::Missed);
        let rendered = pane.render_rows(&rows, options(usize::from(area.width)));
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom()))
            .expect("create settings terminal");
        terminal
            .draw(|frame| pane.render_lines(frame, rendered.lines.clone()))
            .expect("draw the initial settings selection");
        assert_eq!(
            (0..rendered.lines.len())
                .map(|line| pane.line_target(line))
                .collect::<Vec<_>>(),
            [0, 0, 0, 0, 1, 2].map(|row| SettingsLineTarget::Row(SettingsRowPayload::new(row)))
        );
        assert_eq!(pane.viewport().visible_rows(), 3);
        assert_eq!(pane.viewport().scroll_offset(), 2);
        assert_eq!(pane.viewport().pos(), 1);
        let painted = terminal.backend().buffer().clone();
        let clicked_line = (area.x..area.right())
            .map(|x| painted[(x, click.y)].symbol())
            .collect::<String>();
        assert!(clicked_line.contains("part-3"), "{clicked_line}");

        // Both inputs arrive before the terminal is drawn again.
        pane.navigate(NavAction::Down);
        assert_eq!(pane.viewport().scroll_offset(), 3);
        assert_eq!(pane.viewport().pos(), 2);
        let hit = pane.row_at(click);
        assert_eq!(hit, SettingsRowHit::Row(0));
        if let SettingsRowHit::Row(row) = hit {
            pane.select_row(row);
        }
        assert_eq!(pane.viewport().pos(), 0);
        assert_eq!(terminal.backend().buffer(), &painted);

        let rendered = pane.render_rows(&rows, options(usize::from(area.width)));
        pane.update_scroll();
        terminal
            .draw(|frame| pane.render_lines(frame, rendered.lines.clone()))
            .expect("repaint settings after the click");
        assert_eq!(pane.viewport().scroll_offset(), 3);
        let repainted = terminal.backend().buffer();
        let clicked_line = (area.x..area.right())
            .map(|x| repainted[(x, click.y)].symbol())
            .collect::<String>();
        assert!(clicked_line.contains("next"), "{clicked_line}");
        assert_ne!(repainted, &painted);
        assert_eq!(pane.row_at(click), SettingsRowHit::Row(1));
    }

    #[test]
    fn drawing_records_an_offset_set_directly_by_the_consumer() {
        let mut pane = SettingsPane::new();
        let area = Rect::new(7, 5, 28, 3);
        let rows = account_rows(8);
        let rendered = pane.render_rows(&rows, options(usize::from(area.width)));
        let viewport = pane.viewport_mut();
        viewport.set_content_area(area);
        viewport.set_viewport_rows(usize::from(area.height));
        viewport.set_scroll_offset(2);
        let mut terminal = Terminal::new(TestBackend::new(area.right(), area.bottom()))
            .expect("create settings terminal");
        terminal
            .draw(|frame| pane.render_lines(frame, rendered.lines.clone()))
            .expect("draw the consumer-selected offset");
        let click = Position::new(area.x + 1, area.y + 1);
        assert_eq!(pane.row_at(click), SettingsRowHit::Row(2));

        pane.viewport_mut().set_scroll_offset(3);
        assert_eq!(pane.row_at(click), SettingsRowHit::Row(2));
        terminal
            .draw(|frame| pane.render_lines(frame, rendered.lines.clone()))
            .expect("repaint the updated offset");
        assert_eq!(pane.row_at(click), SettingsRowHit::Row(3));
    }

    #[test]
    fn decoration_and_missing_rendered_content_have_named_outcomes() {
        let mut pane = SettingsPane::new();
        assert_eq!(
            pane.line_for_selection(0),
            SettingsSelectionLine::NotRendered
        );
        assert_eq!(pane.line_target(0), SettingsLineTarget::OutsideContent);
        assert_eq!(pane.row_at(Position::new(0, 0)), SettingsRowHit::Missed);
        let rows = vec![
            SettingsRow::section("Capture"),
            SettingsRow::value(0, "account", "account-00"),
        ];
        let area = Rect::new(2, 3, 40, 6);
        let lines = render(&mut pane, &rows, area);
        let heading = lines
            .iter()
            .position(|line| line.contains("Capture"))
            .expect("render section heading");
        assert_eq!(pane.line_target(heading), SettingsLineTarget::Decoration);
        let account = lines
            .iter()
            .position(|line| line.contains("account-00"))
            .expect("render account row");
        assert_eq!(
            pane.line_target(account),
            SettingsLineTarget::Row(SettingsRowPayload::new(0))
        );
        assert_eq!(
            pane.line_for_selection(0),
            SettingsSelectionLine::Rendered(account)
        );
        assert_eq!(
            pane.line_for_selection(1),
            SettingsSelectionLine::NotRendered
        );
        assert_eq!(
            pane.line_target(lines.len()),
            SettingsLineTarget::OutsideContent
        );
        assert_eq!(
            pane.row_at(Position::new(
                area.x,
                area.y + u16::try_from(heading).expect("heading fits terminal coordinates")
            )),
            SettingsRowHit::Missed
        );
    }

    #[test]
    fn every_short_account_and_following_setting_is_visible_in_both_directions() {
        let count = 24;
        let rows = account_rows(count);
        for height in [1, 3, 7] {
            let area = Rect::new(4, 2, 40, height);
            let mut pane = SettingsPane::new();
            for selection in 0..=count {
                let lines = render(&mut pane, &rows, area);
                let expected = if selection == count {
                    "nextest".to_owned()
                } else {
                    format!("account-{selection:02}")
                };
                assert_eq!(pane.viewport().pos(), selection);
                assert!(
                    visible(&pane, &lines).contains(&expected),
                    "{expected}: {height}"
                );
                pane.navigate(NavAction::Down);
            }
            for selection in (0..count).rev() {
                pane.navigate(NavAction::Up);
                let lines = render(&mut pane, &rows, area);
                assert_eq!(pane.viewport().pos(), selection);
                assert!(visible(&pane, &lines).contains(&format!("account-{selection:02}")));
            }
        }
    }

    #[test]
    fn navigation_exposes_every_line_of_an_account_taller_than_the_popup() {
        let markers = diagnostics();
        let rows = vec![
            SettingsRow::value(0, "account", markers.join(" ")),
            SettingsRow::value(1, "excluded", "nextest"),
        ];
        for height in [1, 3, 5] {
            let area = Rect::new(2, 4, 28, height);
            let mut pane = SettingsPane::new();
            let lines = render(&mut pane, &rows, area);
            assert!(lines.len() > usize::from(height));
            let mut seen = BTreeSet::new();
            for _ in 0..=lines.len() {
                let lines = render(&mut pane, &rows, area);
                let text = visible(&pane, &lines);
                seen.extend(
                    markers
                        .iter()
                        .filter(|marker| text.contains(marker.as_str())),
                );
                if pane.viewport().pos() == 1 {
                    assert!(text.contains("nextest"));
                    break;
                }
                pane.navigate(NavAction::Down);
            }
            assert_eq!(
                pane.viewport().pos(),
                1,
                "navigation must leave the tall account"
            );
            assert_eq!(seen, markers.iter().collect(), "a continuation was skipped");
            seen.clear();
            for _ in 0..=lines.len() {
                pane.navigate(NavAction::Up);
                let lines = render(&mut pane, &rows, area);
                let text = visible(&pane, &lines);
                seen.extend(
                    markers
                        .iter()
                        .filter(|marker| text.contains(marker.as_str())),
                );
            }
            assert_eq!(pane.viewport().pos(), 0);
            assert_eq!(
                seen,
                markers.iter().collect(),
                "upward navigation skips a continuation"
            );
            let lines = render(&mut pane, &rows, area);
            assert!(visible(&pane, &lines).contains(&markers[0]));
        }
    }

    #[test]
    fn selected_account_remains_visible_after_resize_and_status_growth() {
        let mut rows = account_rows(20);
        let mut pane = SettingsPane::new();
        render(&mut pane, &rows, Rect::new(3, 2, 64, 8));
        pane.select_row(12);
        let lines = render(&mut pane, &rows, Rect::new(3, 2, 64, 8));
        assert!(visible(&pane, &lines).contains("account-12"));
        for area in [Rect::new(5, 3, 28, 2), Rect::new(1, 1, 80, 12)] {
            let lines = render(&mut pane, &rows, area);
            assert_eq!(pane.viewport().pos(), 12);
            assert!(visible(&pane, &lines).contains("account-12"));
        }
        rows[1].value = diagnostics().join(" ");
        let area = Rect::new(5, 3, 28, 2);
        let lines = render(&mut pane, &rows, area);
        assert_eq!(pane.viewport().pos(), 12);
        assert!(visible(&pane, &lines).contains("account-12"));
        let offset = pane.viewport().scroll_offset();
        let lines = render(&mut pane, &rows, area);
        assert_eq!(
            pane.viewport().scroll_offset(),
            offset,
            "refresh resets scrolling"
        );
        assert!(visible(&pane, &lines).contains("account-12"));
    }

    #[test]
    fn framework_click_on_visible_continuation_selects_its_account() {
        let mut app = SettingsApp {
            framework: Framework::new(FocusedPane::App(())),
        };
        let keymap = Keymap::<SettingsApp>::builder()
            .build()
            .expect("build settings keymap");
        keymap.dispatch_framework_global(GlobalAction::OpenSettings, &mut app);
        let markers = diagnostics();
        let rows = vec![
            SettingsRow::section("Capture"),
            SettingsRow::value(0, "account", markers.join(" ")),
            SettingsRow::value(1, "excluded", "nextest"),
        ];
        let area = Rect::new(7, 5, 28, 3);
        render(&mut app.framework.settings_pane, &rows, area);
        app.framework.settings_pane.select_row(1);
        let lines = render(&mut app.framework.settings_pane, &rows, area);
        assert_eq!(app.framework.settings_pane.viewport().pos(), 1);
        let continuation = lines
            .iter()
            .position(|line| line.contains(&markers[markers.len() - 1]))
            .expect("last diagnostic continuation is rendered");
        let offset = app.framework.settings_pane.viewport().scroll_offset();
        assert!(continuation >= offset && continuation < offset + usize::from(area.height));
        assert!(
            !lines[continuation].contains("account"),
            "click must hit a continuation"
        );
        let pos = Position::new(
            area.x + 1,
            area.y
                + u16::try_from(continuation - offset)
                    .expect("visible continuation fits terminal coordinates"),
        );
        let hit = app.framework.hit_test_at(pos);
        assert_eq!(
            hit,
            Some(FrameworkHit::Overlay {
                id:  FrameworkOverlayId::Settings,
                row: 0,
            })
        );
        if let Some(FrameworkHit::Overlay { row, .. }) = hit {
            app.framework.settings_pane.select_row(row);
        }
        let lines = render(&mut app.framework.settings_pane, &rows, area);
        assert_eq!(app.framework.settings_pane.viewport().pos(), 0);
        assert!(
            visible(&app.framework.settings_pane, &lines).contains(&markers[markers.len() - 1])
        );
        assert_eq!(
            app.framework.hit_test_at(Position::new(area.x - 1, area.y)),
            Some(FrameworkHit::ModalMissed)
        );
    }
}
