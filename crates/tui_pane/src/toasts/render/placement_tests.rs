use std::error::Error;
use std::time::Duration;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use super::GridStability;
use super::ToastsRenderCtx;
use super::card;
use super::test_support::TestApp;
use crate::ColorMode;
use crate::Hittable as _;
use crate::PaneFocusState;
use crate::Renderable;
use crate::Toasts;
use crate::constants::FRAME_POLL_MILLIS;
use crate::constants::TOAST_ROW_GAP;
use crate::toasts::ToastCountdownState;
use crate::toasts::ToastFocus;
use crate::toasts::ToastHit;
use crate::toasts::ToastId;
use crate::toasts::ToastStyle;
use crate::toasts::ToastVisualDeadline;

fn row(buffer: &Buffer, y: u16) -> String {
    (buffer.area.left()..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

/// The grid in the toast-over-grid tests; a summary cell starts at its top-left corner.
const GRID: Rect = Rect::new(0, 0, 80, 30);
/// The lifetime of the startup notice in the toast-over-grid tests.
const NOTICE_SECONDS: u64 = 12;
const MULTILINE_NOTICE_BODY: &str = "Metadata\nDisk\nLanguages\nGit\nTests";

/// A startup notice and the instant when its entrance is complete.
struct GridNotice {
    toasts:  Toasts<TestApp>,
    id:      ToastId,
    settled: Instant,
}

impl GridNotice {
    fn new() -> Self { Self::with_body("Read projects and check the registry.") }

    fn with_body(body: &str) -> Self {
        let mut toasts = Toasts::<TestApp>::new();
        let id = toasts.push_timed_styled(
            "Startup",
            body,
            Duration::from_secs(NOTICE_SECONDS),
            1,
            ToastStyle::Success,
        );
        let settled = toasts.entries[0].created_at + Duration::from_secs(1);
        Self {
            toasts,
            id,
            settled,
        }
    }

    /// Draw a settled grid with a summary cell `summary_height` rows tall, `elapsed` seconds
    /// after the entrance, then the notice over it. Return the notice card.
    fn draw(
        &mut self,
        terminal: &mut Terminal<TestBackend>,
        summary_height: u16,
        elapsed: u64,
    ) -> Result<Rect, Box<dyn Error>> {
        self.draw_over(terminal, GridStability::Settled, summary_height, elapsed)
    }

    /// Draw the grid in `grid_stability` with a summary cell `summary_height` rows tall,
    /// `elapsed` seconds after the entrance, then the notice over it. Return the notice card.
    fn draw_over(
        &mut self,
        terminal: &mut Terminal<TestBackend>,
        grid_stability: GridStability,
        summary_height: u16,
        elapsed: u64,
    ) -> Result<Rect, Box<dyn Error>> {
        self.draw_over_panes(terminal, grid_stability, summary_height, &[], elapsed)
    }

    fn draw_over_panes(
        &mut self,
        terminal: &mut Terminal<TestBackend>,
        grid_stability: GridStability,
        summary_height: u16,
        panes: &[Rect],
        elapsed: u64,
    ) -> Result<Rect, Box<dyn Error>> {
        let now = self.settled + Duration::from_secs(elapsed);
        terminal.draw(|frame| {
            let chrome = crate::default_pane_chrome(ColorMode::Colored);
            frame.render_widget(chrome.block("Grid".to_owned(), false), GRID);
            let summary = Rect::new(GRID.x, GRID.y, GRID.width, summary_height);
            frame.render_widget(chrome.block("Summary".to_owned(), false), summary);
            for pane in panes {
                frame.render_widget(chrome.block("Command".to_owned(), false), *pane);
            }
            self.toasts.render_over_grid(
                frame,
                crate::frame_inner(GRID),
                &[summary],
                grid_stability,
                &ToastsRenderCtx {
                    now,
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
        })?;
        let hit = self.toasts.hits().first().ok_or("the notice is drawn")?;
        let buffer = terminal.backend().buffer();
        let remaining = NOTICE_SECONDS - 1 - elapsed;
        let close = Position::new(hit.close_rect.x + 1, hit.close_rect.y);
        assert!(row(buffer, hit.card_rect.y).contains("Startup"));
        assert!(
            row(buffer, hit.card_rect.bottom() - 1).contains(&format!("Closing in {remaining}"))
        );
        assert_eq!(buffer[close].symbol(), "x");
        assert_eq!(
            self.toasts.hit_test_at(close),
            Some(ToastHit::Close(self.id))
        );
        Ok(hit.card_rect)
    }
}

#[test]
fn a_drawn_toast_keeps_its_position_when_grid_frames_change_height() -> Result<(), Box<dyn Error>> {
    let mut notice = GridNotice::new();
    let mut terminal = Terminal::new(TestBackend::new(GRID.width, GRID.height))?;
    let first = notice.draw(&mut terminal, 8, 0)?;
    // The summary foot and bottom border stay above the cleared gap over the card.
    let tall = first.top() - TOAST_ROW_GAP;
    for (elapsed, summary_height) in (1..).zip([tall, 12]) {
        let card = notice.draw(&mut terminal, summary_height, elapsed)?;
        assert_eq!(card, first, "summary height {summary_height}");
    }
    Ok(())
}

#[test]
fn a_toast_placed_while_the_grid_moves_moves_once_on_the_first_settled_frame()
-> Result<(), Box<dyn Error>> {
    let mut terminal = Terminal::new(TestBackend::new(GRID.width, GRID.height))?;
    let settled = GridNotice::new().draw(&mut terminal, 8, 0)?;
    let mut notice = GridNotice::new();
    // A summary cell 20 rows tall leaves more rows above its foot than below it, so the
    // search on the moving frame puts the card above the foot.
    let moving = notice.draw_over(&mut terminal, GridStability::Moving, 20, 0)?;
    assert_ne!(moving, settled);
    // Between two countdown ticks, only the kept place asks for the next frame poll.
    let between_ticks = notice.settled + Duration::from_millis(500);
    let next_poll =
        ToastVisualDeadline::At(between_ticks + Duration::from_millis(FRAME_POLL_MILLIS));
    assert_eq!(
        notice.toasts.next_visual_change_deadline(between_ticks),
        next_poll
    );
    assert_eq!(notice.draw(&mut terminal, 8, 1)?, settled);
    assert_ne!(
        notice.toasts.next_visual_change_deadline(between_ticks),
        next_poll
    );
    // The summary foot and bottom border stay above the cleared gap over the card.
    let tall = settled.top() - TOAST_ROW_GAP;
    let fresh = GridNotice::new().draw(&mut terminal, tall, 0)?;
    assert_ne!(fresh, settled, "fresh placement at summary height {tall}");
    for (elapsed, (grid_stability, summary_height)) in (2..).zip([
        (GridStability::Settled, tall),
        (GridStability::Moving, 12),
        (GridStability::Settled, tall),
    ]) {
        let card = notice.draw_over(&mut terminal, grid_stability, summary_height, elapsed)?;
        assert_eq!(
            card, settled,
            "{grid_stability:?} summary height {summary_height}"
        );
    }
    Ok(())
}

#[test]
fn a_startup_notice_stays_visible_as_the_protected_foot_rises() -> Result<(), Box<dyn Error>> {
    let mut terminal = Terminal::new(TestBackend::new(GRID.width, GRID.height))?;
    for initial_grid_stability in [GridStability::Moving, GridStability::Settled] {
        let mut notice = GridNotice::with_body(MULTILINE_NOTICE_BODY);
        let first = notice.draw_over(&mut terminal, initial_grid_stability, 20, 0)?;
        assert!(first.height > 3, "{first:?}");
        let reaching = first.top() + 3;
        for summary_height in (reaching..=GRID.height).rev() {
            let card = notice.draw_over(&mut terminal, GridStability::Moving, summary_height, 0)?;
            assert_eq!(card, first, "summary height {summary_height}");
        }
        let between_ticks = notice.settled + Duration::from_millis(500);
        let next_poll =
            ToastVisualDeadline::At(between_ticks + Duration::from_millis(FRAME_POLL_MILLIS));
        assert_eq!(
            notice.toasts.next_visual_change_deadline(between_ticks),
            next_poll
        );
        let settled = notice.draw(&mut terminal, reaching, 1)?;
        assert_ne!(settled, first);
        assert_eq!(settled.height, first.height);
        assert_ne!(
            notice.toasts.next_visual_change_deadline(between_ticks),
            next_poll
        );
        assert_eq!(notice.draw(&mut terminal, reaching, 2)?, settled);
    }
    Ok(())
}

#[test]
fn a_startup_notice_keeps_its_place_when_the_settled_corner_is_too_short()
-> Result<(), Box<dyn Error>> {
    let mut terminal = Terminal::new(TestBackend::new(GRID.width, GRID.height))?;
    let mut notice = GridNotice::with_body(MULTILINE_NOTICE_BODY);
    let first = notice.draw_over(&mut terminal, GridStability::Moving, 20, 0)?;
    // The Command pane leaves one row below its top border inside the grid viewport.
    let command = Rect::new(GRID.x, GRID.bottom() - 3, GRID.width, 3);
    for elapsed in 1..=2 {
        let card = notice.draw_over_panes(
            &mut terminal,
            GridStability::Settled,
            8,
            &[command],
            elapsed,
        )?;
        assert_eq!(card, first);
    }
    Ok(())
}

#[test]
fn a_startup_notice_first_drawn_short_keeps_its_place_during_motion() -> Result<(), Box<dyn Error>>
{
    let mut terminal = Terminal::new(TestBackend::new(GRID.width, GRID.height))?;
    let mut notice = GridNotice::with_body(MULTILINE_NOTICE_BODY);
    let full_height = notice.toasts.active_views(notice.settled)[0].settled_height();
    let command = Rect::new(GRID.x, GRID.bottom() - 7, GRID.width, 7);
    let first = notice.draw_over_panes(&mut terminal, GridStability::Moving, 8, &[command], 0)?;
    assert!(first.height < full_height);
    for summary_height in [GRID.height, 20, 8] {
        let card = notice.draw_over(&mut terminal, GridStability::Moving, summary_height, 0)?;
        assert_eq!(card, first, "summary height {summary_height}");
    }
    assert_eq!(notice.draw(&mut terminal, 8, 1)?.height, full_height);
    Ok(())
}

#[test]
fn a_held_toast_moves_once_when_a_protected_foot_reaches_it() -> Result<(), Box<dyn Error>> {
    let mut notice = GridNotice::new();
    let mut terminal = Terminal::new(TestBackend::new(GRID.width, GRID.height))?;
    let first = notice.draw(&mut terminal, 8, 0)?;
    // The summary foot is the row under the card's top border.
    let reaching = first.top() + 3;
    let moved = notice.draw(&mut terminal, reaching, 1)?;
    assert_ne!(moved, first);
    assert!(moved.bottom() + TOAST_ROW_GAP <= reaching - 2, "{moved:?}");
    for (elapsed, summary_height) in (2..).zip([reaching, 8]) {
        let card = notice.draw(&mut terminal, summary_height, elapsed)?;
        assert_eq!(card, moved, "summary height {summary_height}");
    }
    Ok(())
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
