use std::error::Error;
use std::time::Duration;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use super::ToastsRenderCtx;
use super::card;
use super::test_support::TestApp;
use crate::ColorMode;
use crate::PaneFocusState;
use crate::Renderable;
use crate::Toasts;
use crate::toasts::RunBreaks;
use crate::toasts::ToastActivation;
use crate::toasts::ToastFocus;
use crate::toasts::ToastLingerState;
use crate::toasts::ToastRun;
use crate::toasts::ToastStyle;
use crate::toasts::body;

#[test]
fn every_toast_style_keeps_a_blank_cell_before_the_close_button() -> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        for style in [
            ToastStyle::Normal,
            ToastStyle::Success,
            ToastStyle::Warning,
            ToastStyle::Error,
        ] {
            let mut toasts = Toasts::<TestApp>::new();
            toasts.push_persistent(
                "crates.io unreachable with a long title",
                "Metadata is unavailable.",
                style,
                ToastActivation::Informational,
                1,
            );
            let now = toasts.entries[0].created_at + Duration::from_secs(1);
            let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
            terminal.draw(|frame| {
                Renderable::render(
                    &mut toasts,
                    frame,
                    Rect::new(46, 0, 34, 30),
                    &ToastsRenderCtx {
                        now,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
                mode.clear_colors(frame.buffer_mut());
            })?;
            let hit = toasts.hits().first().ok_or("toast card")?;
            let buffer = terminal.backend().buffer();
            assert_eq!(
                buffer[(hit.close_rect.x - 1, hit.close_rect.y)].symbol(),
                " ",
                "{mode:?}: {style:?}"
            );
            let close: String = (hit.close_rect.x..hit.close_rect.x + 3)
                .map(|x| buffer[(x, hit.close_rect.y)].symbol())
                .collect();
            assert_eq!(close, "[x]");
            let title: String = (hit.card_rect.left() + 1..hit.close_rect.x - 1)
                .map(|x| buffer[(x, hit.close_rect.y)].symbol())
                .collect();
            assert!(title.contains("crates.io"), "{title}");
        }
    }
    Ok(())
}

#[test]
fn an_opening_error_toast_keeps_its_first_sentence_on_one_page() -> Result<(), Box<dyn Error>> {
    for (sentence, pane, expected) in [
        (
            "Crate metadata is unavailable until crates.io recovers.",
            Rect::new(46, 20, 34, 9),
            vec![
                "Crate metadata is",
                "unavailable until",
                "crates.io recovers.",
            ],
        ),
        (
            "abcdefghijklmnopqr.",
            Rect::new(66, 16, 13, 13),
            vec!["abcd…", "efgh…", "ijkl…", "mnop…", "qr."],
        ),
    ] {
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            let mut toasts = Toasts::<TestApp>::new();
            toasts.push_persistent(
                "crates.io unreachable",
                sentence,
                ToastStyle::Error,
                ToastActivation::Informational,
                1,
            );
            let created = toasts.entries[0].created_at;
            for elapsed in [
                Duration::ZERO,
                Duration::from_millis(40),
                Duration::from_secs(1),
            ] {
                let mut terminal = Terminal::new(TestBackend::new(80, 30))?;
                terminal.draw(|frame| {
                    frame.render_widget(
                        crate::default_pane_chrome(mode).block("CI runs".to_owned(), false),
                        pane,
                    );
                    Renderable::render(
                        &mut toasts,
                        frame,
                        pane.inner(ratatui::layout::Margin::new(1, 1)),
                        &ToastsRenderCtx {
                            now:              created + elapsed,
                            pane_focus_state: PaneFocusState::Inactive,
                        },
                    );
                    mode.clear_colors(frame.buffer_mut());
                })?;
                let card = toasts.hits().first().ok_or("error card")?.card_rect;
                let buffer = terminal.backend().buffer();
                let body: Vec<_> = (card.top() + 1..card.bottom() - 1)
                    .map(|y| {
                        (card.left() + 1..card.right() - 1)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                            .trim()
                            .to_owned()
                    })
                    .collect();
                assert_eq!(body, expected, "{mode:?}: {elapsed:?}");
                if pane.width == 13 {
                    assert_eq!(
                        body.join(" ").replace("… ", ""),
                        sentence,
                        "{mode:?}: {elapsed:?}"
                    );
                    assert_eq!(body.last().map(String::as_str), Some("qr."));
                    assert_eq!(body.len(), 5);
                }
                let bottom: String = (card.left()..card.right())
                    .map(|x| buffer[(x, card.bottom() - 1)].symbol())
                    .collect();
                assert!(!bottom.contains('▼'), "{bottom}");
                assert!(card.top() > pane.top());
                assert!(card.bottom() < pane.bottom());
                for x in card.left()..card.right() {
                    assert_eq!(buffer[(x, card.bottom())].symbol(), " ");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn narrow_toasts_measure_and_draw_the_same_marked_words() -> Result<(), Box<dyn Error>> {
    for (window, width, title, body, expected) in [
        (60, 9, "Startup", "metadata", vec!["meta…", "data"]),
        (60, 9, "Startup", "     metadata", vec!["", "meta…", "data"]),
        (60, 10, "   a", "metadata", vec!["metad…", "ata"]),
        (80, 30, "Startup…", "metadata", vec!["metadata"]),
        (60, 9, "Startup", "東京abcdef", vec!["東京…", "abcd…", "ef"]),
        (
            60,
            9,
            "Startup",
            "\u{1}東京abcdef",
            vec!["東京…", "abcd…", "ef"],
        ),
        (
            60,
            9,
            "Startup",
            "東京東京東京",
            vec!["東京…", "東京…", "東京"],
        ),
        (
            60,
            9,
            "Startup",
            "e\u{301}abcdef",
            vec!["e\u{301}abc…", "def"],
        ),
        (
            80,
            30,
            "Startup",
            "Crate metadata is unavailable.",
            vec!["Crate metadata is", "unavailable."],
        ),
    ] {
        for mode in [ColorMode::Colored, ColorMode::NoColor] {
            for styled in [false, true] {
                let mut toasts = Toasts::<TestApp>::new();
                if styled {
                    toasts.push_timed_runs(
                        title,
                        vec![ToastRun::new(body, Style::default(), RunBreaks::AtSpaces)],
                        Duration::from_secs(30),
                        1,
                        ToastStyle::Normal,
                    );
                } else {
                    toasts.push(title, body);
                }
                let toast = &toasts.entries[0];
                let now = toast.created_at + Duration::from_secs(1);
                let mut view = toast.view(now, toasts.settings(), width);
                assert_eq!(usize::from(view.desired_height()), expected.len() + 2);
                assert_eq!(
                    view.render_body
                        .wrapped_line_count(body::card_body_width(width)),
                    expected.len()
                );
                for linger in [
                    ToastLingerState::Live,
                    ToastLingerState::Lingering { progress: 0.5 },
                ] {
                    view.linger = linger;
                    let card = Rect::new(window - width - 1, 23, width, view.desired_height());
                    let mut terminal = Terminal::new(TestBackend::new(window, 30))?;
                    terminal.draw(|frame| {
                        card::render_toast(
                            frame,
                            frame.area(),
                            card,
                            &view,
                            ToastFocus::PaneUnfocused,
                            30,
                        );
                        mode.clear_colors(frame.buffer_mut());
                    })?;
                    let buffer = terminal.backend().buffer();
                    let rows =
                        drawn_body_rows(buffer, card.inner(ratatui::layout::Margin::new(2, 1)));
                    assert_eq!(rows, expected, "{window}: {mode:?}: {linger:?}");
                    let drawn_title: String = (card.left()..card.right())
                        .map(|x| buffer[(x, card.top())].symbol())
                        .collect();
                    assert!(drawn_title.contains("[x]"), "{drawn_title}");
                    if width <= 10 {
                        assert!(
                            !drawn_title.contains('…')
                                && !drawn_title.contains("Startup")
                                && !drawn_title.contains('a'),
                            "{drawn_title}"
                        );
                    } else {
                        assert!(drawn_title.contains(title), "{drawn_title}");
                    }
                }
            }
        }
    }
    Ok(())
}

/// Read drawn symbols without the cells that follow wide symbols.
fn drawn_body_rows(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.top()..area.bottom())
        .map(|y| {
            let mut row = String::new();
            let mut x = area.left();
            while x < area.right() {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                x += u16::try_from(symbol.width().max(1)).unwrap_or(u16::MAX);
            }
            row.trim_end().to_owned()
        })
        .collect()
}
