use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::ToastsRenderCtx;
use super::test_support::TestApp;
use crate::ColorMode;
use crate::PaneFocusState;
use crate::Renderable;
use crate::Toasts;
use crate::constants::TOAST_ANIMATION_MILLIS;

const STARTUP_ROWS: [&str; 8] = [
    "Cargo metadata",
    "crates.io",
    "Disk usage",
    "GitHub repos",
    "Languages",
    "Lint history",
    "Local git repos",
    "Test counts",
];

fn draw_startup(
    toasts: &mut Toasts<TestApp>,
    now: Instant,
    mode: ColorMode,
    interior_rows: u16,
) -> Result<(Buffer, Rect), Box<dyn Error>> {
    let pane = Rect::new(96, 43 - interior_rows, 64, interior_rows + 2);
    let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
    terminal.draw(|frame| {
        frame.render_widget(
            crate::default_pane_chrome(mode).block("CI runs".to_owned(), false),
            pane,
        );
        let before = frame.buffer_mut().clone();
        Renderable::render(
            toasts,
            frame,
            frame.area(),
            &ToastsRenderCtx {
                now,
                pane_focus_state: PaneFocusState::Inactive,
            },
        );
        mode.clear_colors(frame.buffer_mut());
        for y in pane.top()..pane.bottom() {
            for x in pane.left()..pane.right() {
                if y == pane.top()
                    || y == pane.bottom() - 1
                    || x == pane.left()
                    || x == pane.right() - 1
                {
                    assert_eq!(
                        frame.buffer_mut()[(x, y)].symbol(),
                        before[(x, y)].symbol(),
                        "{mode:?}: border {x},{y} at {now:?}",
                    );
                }
            }
        }
        for x in pane.left() + 1..pane.right() - 1 {
            assert_eq!(
                frame.buffer_mut()[(x, pane.bottom() - 2)].symbol(),
                " ",
                "{mode:?}: bottom gap {x} at {now:?}",
            );
        }
    })?;
    let card = toasts.hits().first().ok_or("Startup card")?.card_rect;
    assert_eq!(card.bottom(), pane.bottom() - 2, "{mode:?}: {card:?}");
    Ok((terminal.backend().buffer().clone(), card))
}

fn assert_body(buffer: &Buffer, card: Rect, rows: &[String], page_count: usize) {
    let row = |y| {
        (card.left() + 1..card.right() - 1)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
    };
    assert!(row(card.top()).contains("Startup"));
    for (y, expected) in (card.top() + 1..card.bottom() - 1).zip(rows) {
        assert_eq!(row(y).trim(), expected);
        let x = (card.left() + 1..card.right() - 1)
            .find(|x| buffer[(*x, y)].symbol() != " ")
            .unwrap_or_else(|| card.left() + 1);
        assert_eq!(buffer[(x, y)].fg, Color::Green);
    }
    assert!(row(card.bottom() - 1).contains(&format!("1 of {page_count} ▼")));
}

#[test]
fn colored_startup_keeps_the_bottom_gap_through_every_opening_height() -> Result<(), Box<dyn Error>>
{
    let rows: Vec<String> = STARTUP_ROWS.into_iter().map(str::to_owned).collect();
    let mut toasts = Toasts::<TestApp>::new();
    toasts.push_colored_persistent("Startup", rows.clone(), vec![Color::Green; rows.len()]);
    let created = toasts.entries[0].created_at;
    let row_step_millis = TOAST_ANIMATION_MILLIS.max(1);
    for (millis, body_rows) in [
        (0, 1),
        (row_step_millis * 3 - 1, 1),
        (row_step_millis * 3, 2),
        (row_step_millis * 4, 3),
        (row_step_millis * 5, 4),
        (row_step_millis * 6, 5),
        (row_step_millis * 7, 6),
        (row_step_millis * 8, 6),
        (row_step_millis * 9, 6),
        (row_step_millis * 10, 6),
    ] {
        let (buffer, card) = draw_startup(
            &mut toasts,
            created + Duration::from_millis(millis),
            ColorMode::Colored,
            9,
        )?;
        assert_eq!(usize::from(card.height), body_rows + 2, "{millis}");
        assert_body(&buffer, card, &rows[..body_rows], 2);
    }
    Ok(())
}

#[test]
fn thirteen_colored_rows_use_six_rows_per_page_in_a_nine_row_pane() -> Result<(), Box<dyn Error>> {
    let rows: Vec<String> = (1..=13)
        .map(|index| format!("Startup task {index:02}"))
        .collect();
    let mut toasts = Toasts::<TestApp>::new();
    toasts.push_colored_persistent("Startup", rows.clone(), vec![Color::Green; rows.len()]);
    let now = toasts.entries[0].created_at + Duration::from_secs(3);
    let (buffer, card) = draw_startup(&mut toasts, now, ColorMode::Colored, 9)?;
    assert_eq!(card.height, 8);
    assert_body(&buffer, card, &rows[..6], 3);
    Ok(())
}

#[test]
fn arriving_colored_lines_keep_the_gap_and_update_the_page_count() -> Result<(), Box<dyn Error>> {
    let mut toasts = Toasts::<TestApp>::new();
    let id = toasts.push_colored_persistent("Startup", Vec::new(), Vec::new());
    let created = toasts.entries[0].created_at;
    draw_startup(&mut toasts, created, ColorMode::Colored, 9)?;
    for (count, pages) in [(8, 2), (13, 3)] {
        let rows: Vec<String> = (1..=count)
            .map(|index| format!("Startup task {index:02}"))
            .collect();
        assert!(toasts.update_colored(id, rows.clone(), vec![Color::Green; rows.len()]));
        let (buffer, card) = draw_startup(
            &mut toasts,
            created + Duration::from_secs(3),
            ColorMode::Colored,
            9,
        )?;
        assert_eq!(card.height, 8);
        assert_body(&buffer, card, &rows[..6], pages);
    }
    Ok(())
}
