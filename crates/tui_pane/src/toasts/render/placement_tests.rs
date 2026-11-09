use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::ToastsRenderCtx;
use super::card;
use super::test_support::TestApp;
use crate::ColorMode;
use crate::PaneFocusState;
use crate::Renderable;
use crate::Toasts;
use crate::toasts::ToastCountdownState;
use crate::toasts::ToastFocus;

fn row(buffer: &Buffer, y: u16) -> String {
    (buffer.area.left()..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn an_opening_startup_card_keeps_its_bottom_gap_when_it_fits() -> Result<(), Box<dyn Error>> {
    for mode in [ColorMode::Colored, ColorMode::NoColor] {
        let mut toasts = Toasts::<TestApp>::new();
        toasts.push(
            "Startup",
            "Metadata\nDisk\nLanguages\nGit\nTests\nLints\nRegistry",
        );
        let created = toasts.entries[0].created_at;
        let pane = Rect::new(46, 22, 34, 7);
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
                    now:              created,
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
            mode.clear_colors(frame.buffer_mut());
        })?;
        let card = toasts.hits()[0].card_rect;
        let buffer = terminal.backend().buffer();
        assert_eq!(card.height, 3);
        assert_eq!(card.bottom(), pane.bottom() - 2, "{mode:?}: {card:?}");
        for x in pane.left() + 1..pane.right() - 1 {
            assert_eq!(buffer[(x, card.bottom())].symbol(), " ", "{mode:?}: {x}");
        }
        assert!(row(buffer, card.y).contains("Startup"));
        assert!(row(buffer, card.y + 1).contains("Metadata"));
        assert!(row(buffer, card.bottom() - 1).contains('▼'));
        assert!(row(buffer, pane.top()).contains("CI runs"));
    }
    Ok(())
}

#[test]
fn a_small_corner_pane_keeps_its_title_and_border_rows() -> Result<(), Box<dyn Error>> {
    for inside in [3, 4] {
        let mut toasts = Toasts::<TestApp>::new();
        toasts.push("Startup", "Metadata\nDisk\nLanguages\nGit\nTests\nLints");
        let pane = Rect::new(0, 2, 62, inside + 2);
        let mut terminal = Terminal::new(TestBackend::new(62, pane.bottom()))?;
        terminal.draw(|frame| {
            frame.render_widget(
                crate::default_pane_chrome(ColorMode::Colored).block("CI runs".to_owned(), false),
                pane,
            );
            Renderable::render(
                &mut toasts,
                frame,
                frame.area(),
                &ToastsRenderCtx {
                    now:              Instant::now() + Duration::from_secs(1),
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
        })?;
        let buffer = terminal.backend().buffer();
        assert!(row(buffer, pane.y).contains("CI runs"));
        for hit in toasts.hits() {
            assert!(hit.card_rect.top() > pane.top(), "{:?}", hit.card_rect);
            assert!(
                hit.card_rect.bottom() < pane.bottom(),
                "{:?}",
                hit.card_rect
            );
        }
        assert_eq!(buffer[(0, pane.y)].symbol(), "┌");
        assert_eq!(buffer[(61, pane.bottom() - 1)].symbol(), "┘");
    }
    Ok(())
}

#[test]
fn a_short_corner_pane_uses_plain_gaps_for_body_rows() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<TestApp>::new();
    toasts.push("Startup", "Metadata\nDisk\nLanguages\nGit\nTests\nLints");
    let mut terminal = Terminal::new(TestBackend::new(62, 7))?;
    terminal.draw(|frame| {
        frame.render_widget(
            crate::default_pane_chrome(ColorMode::Colored).block("CI runs".to_owned(), false),
            frame.area(),
        );
        Renderable::render(
            &mut toasts,
            frame,
            frame.area(),
            &ToastsRenderCtx {
                now:              Instant::now() + Duration::from_secs(1),
                pane_focus_state: PaneFocusState::Inactive,
            },
        );
    })?;
    let card = toasts.hits()[0].card_rect;
    assert_eq!(card.height, 5);
    let buffer = terminal.backend().buffer();
    for (offset, text) in ["Metadata", "Disk", "Languages"].into_iter().enumerate() {
        assert!(row(buffer, card.y + 1 + u16::try_from(offset)?).contains(text));
    }
    assert!(row(buffer, 0).contains("CI runs"));
    Ok(())
}

#[test]
fn a_page_mark_uses_all_border_cells_before_the_countdown() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<TestApp>::new();
    toasts.push("Startup", "Metadata\nDisk\nLanguages\nGit\nTests\nLints");
    let mut toast = toasts
        .active_views(Instant::now() + Duration::from_secs(1))
        .remove(0);
    toast.countdown = ToastCountdownState::CountingDown { remaining_secs: 12 };
    let mut terminal = Terminal::new(TestBackend::new(25, 6))?;
    terminal.draw(|frame| {
        card::render_toast(
            frame,
            frame.area(),
            frame.area(),
            &toast,
            ToastFocus::PaneUnfocused,
            6,
        );
        ColorMode::Colored.clear_colors(frame.buffer_mut());
    })?;
    let bottom = row(terminal.backend().buffer(), 5);
    assert!(bottom.contains("1 of 2 ▼"), "{bottom}");
    assert!(bottom.contains(" Closing in 12 "), "{bottom}");
    Ok(())
}

#[test]
fn the_page_count_stays_fixed_during_the_open_animation() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<TestApp>::new();
    let id = toasts.push(
        "Startup",
        "Metadata\nDisk\nLanguages\nGit\nTests\nLints\nRegistry\nTargets",
    );
    let created = toasts
        .entries
        .iter()
        .find(|toast| toast.id() == id)
        .ok_or("toast")?
        .created_at;
    let mut pages = Vec::new();
    for now in [created, created + Duration::from_secs(1)] {
        let mut terminal = Terminal::new(TestBackend::new(62, 8))?;
        terminal.draw(|frame| {
            Renderable::render(
                &mut toasts,
                frame,
                frame.area(),
                &ToastsRenderCtx {
                    now,
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
            ColorMode::Colored.clear_colors(frame.buffer_mut());
        })?;
        let card = toasts.hits()[0].card_rect;
        let bottom = row(terminal.backend().buffer(), card.bottom() - 1);
        let page = bottom
            .split('─')
            .find(|part| part.contains('▼'))
            .ok_or("page mark")?;
        pages.push(page.to_owned());
    }
    assert_eq!(pages[0], pages[1]);
    Ok(())
}

#[test]
fn stacked_toasts_clear_only_their_cards_and_gap_cells() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<TestApp>::new();
    toasts.push("Registry", "Metadata is unavailable");
    toasts.push("GitHub", "CI data is unavailable");
    let mut terminal = Terminal::new(TestBackend::new(160, 24))?;
    let mut before = Buffer::empty(Rect::new(0, 0, 160, 24));
    terminal.draw(|frame| {
        frame.render_widget(
            crate::default_pane_chrome(ColorMode::Colored).block("CI runs".to_owned(), false),
            frame.area(),
        );
        let lines: Vec<_> = (0..22)
            .map(|n| {
                Line::from(format!(
                    "commit-{n:02} branch-{n:02} {}",
                    "table values ".repeat(12),
                ))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), Rect::new(1, 1, 158, 22));
        before.clone_from(frame.buffer_mut());
        Renderable::render(
            &mut toasts,
            frame,
            frame.area(),
            &ToastsRenderCtx {
                now:              Instant::now() + Duration::from_secs(1),
                pane_focus_state: PaneFocusState::Inactive,
            },
        );
    })?;
    assert_eq!(toasts.hits().len(), 2);
    let buffer = terminal.backend().buffer();
    for y in 0..24 {
        for x in 0..160 {
            let covered = toasts.hits().iter().any(|hit| {
                let card = hit.card_rect;
                x >= card.left().saturating_sub(crate::constants::TOAST_SIDE_GAP)
                    && x < card.right() + crate::constants::TOAST_SIDE_GAP
                    && y >= card.top().saturating_sub(crate::constants::TOAST_ROW_GAP)
                    && y < card.bottom() + crate::constants::TOAST_ROW_GAP
            });
            if !covered {
                assert_eq!(buffer[(x, y)], before[(x, y)], "cell {x},{y}");
            }
        }
    }
    Ok(())
}
