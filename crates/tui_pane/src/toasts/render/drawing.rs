use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::layout;
use super::layout::StackLayout;
use super::layout::ToastPaneFocus;
use crate::PaneFocusState;
use crate::ToastPlacement;
use crate::ToastSettings;
use crate::toasts::ToastHitbox;
use crate::toasts::ToastId;
use crate::toasts::ToastView;
use crate::toasts::manager;

/// Compiled-in palette consumed by toast rendering.
///
/// Toasts deliberately do NOT read from the active theme: an error
/// toast must remain legible even if a user-loaded theme is corrupt
/// or its contrast is so low the toast would vanish. This palette is
/// fixed at compile time. The roundtrip test in `mod tests` locks
/// every field against accidental drift.
pub(super) struct FallbackToastPalette {
    /// Spinner color in tracked-item rows.
    pub accent:  Color,
    /// Border + text color for error toasts.
    pub error:   Color,
    /// Border + text color for success toasts.
    pub success: Color,
    /// Border + text color for warning toasts.
    pub warning: Color,
    /// Countdown text, italic action hint, overflow rows.
    pub label:   Color,
    /// Running tracked-item duration suffix.
    pub title:   Color,
}

pub(super) const fn fallback_toast_palette() -> FallbackToastPalette {
    FallbackToastPalette {
        accent:  Color::Cyan,
        error:   Color::Red,
        success: Color::Green,
        warning: Color::Yellow,
        label:   Color::Rgb(150, 190, 180),
        title:   Color::Yellow,
    }
}

/// Result of rendering toast cards.
struct ToastRenderResult {
    /// Hitboxes for the toast card and close-button regions rendered in this pass.
    hitboxes: Vec<ToastHitbox>,
}

/// Render toast cards and return their hit-test regions.
fn render_toasts(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView],
    settings: &ToastSettings,
    pane_focus_state: PaneFocusState,
    focused_toast_id: Option<ToastId>,
) -> ToastRenderResult {
    if !settings.toasts_enabled() || toasts.is_empty() {
        return ToastRenderResult {
            hitboxes: Vec::new(),
        };
    }

    let max_visible = settings.max_visible.get().max(1);
    let start = toasts.len().saturating_sub(max_visible);
    let visible_toasts = &toasts[start..];
    let gap: u16 = 0;
    let available =
        area.height.saturating_sub(gap.saturating_mul(
            u16::try_from(visible_toasts.len().saturating_sub(1)).unwrap_or(u16::MAX),
        ));
    let allocated = layout::allocate_toast_heights(visible_toasts, available);
    let width = manager::toast_card_width(settings, area.width);

    let layout = StackLayout {
        width,
        gap,
        pane_focus: ToastPaneFocus::from(pane_focus_state),
        focused_toast_id,
    };
    let hitboxes = match settings.placement {
        ToastPlacement::TopRight => {
            layout::render_top_down(frame, area, visible_toasts, &allocated, layout)
        },
        ToastPlacement::BottomRight => {
            layout::render_bottom_up(frame, area, visible_toasts, &allocated, layout)
        },
    };

    ToastRenderResult { hitboxes }
}

/// Render-time context for [`super::super::Toasts`]'s
/// [`crate::Renderable`] impl.
///
/// Built directly by the embedding immediately before rendering.
/// `ToastSettings` lives on `Toasts` itself, so the render impl
/// reads it from `self` — the embedding only supplies wall-clock
/// time and the focused-pane state.
pub struct ToastsRenderCtx {
    /// Wall-clock timestamp passed to `Toasts::active_views` for
    /// the prune-and-collect pass.
    pub now:              Instant,
    /// Focus state for the embedding's toasts pane slot.
    pub pane_focus_state: PaneFocusState,
}

impl<Ctx: crate::AppContext> crate::Renderable<ToastsRenderCtx> for super::super::Toasts<Ctx> {
    fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        ctx: &ToastsRenderCtx,
    ) -> Option<crate::PaneFrameChrome> {
        self.set_draw_area_width(area.width);
        let focused_id = self.focused_toast_id();
        let active = self.active_views(ctx.now);
        let result = render_toasts(
            frame,
            area,
            &active,
            self.settings(),
            ctx.pane_focus_state,
            focused_id,
        );
        self.set_hits(result.hitboxes);
        None
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::symbols::line;
    use ratatui::text::Line;
    use ratatui::widgets::Paragraph;
    use toml::Table;

    use super::*;
    use crate::ACTIVITY_SPINNER;
    use crate::AppContext;
    use crate::Framework;
    use crate::PaneFocusState;
    use crate::Renderable;
    use crate::ToastId;
    use crate::Toasts;
    use crate::TrackedItem;
    use crate::TrackedItemActivity;
    use crate::constants::ELISION;
    use crate::toasts::TrackedItemView;
    use crate::toasts::render::card;

    struct TestApp {
        framework: Framework<Self>,
    }

    #[derive(Clone, Copy)]
    enum TestToastAction {
        Absent,
        Open,
    }

    impl AppContext for TestApp {
        type AppPaneId = ();
        type ToastAction = TestToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }

        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    struct RenderedToast {
        card:   Rect,
        buffer: Buffer,
    }

    fn settings_with_width(width: u16) -> ToastSettings {
        let table: Table = format!("[toasts]\nwidth = {width}\n")
            .parse()
            .expect("toast settings TOML should parse");
        ToastSettings::from_table(&table).expect("toast settings should load")
    }

    fn created_at(toasts: &Toasts<TestApp>, id: ToastId) -> Instant {
        toasts
            .entries
            .iter()
            .find(|toast| toast.id() == id)
            .expect("pushed toast should be stored")
            .created_at
    }

    fn draw_at(
        toasts: &mut Toasts<TestApp>,
        width: u16,
        height: u16,
        now: Instant,
    ) -> RenderedToast {
        let backend = TestBackend::new(width, height);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");
        terminal
            .draw(|frame| {
                Renderable::render(
                    toasts,
                    frame,
                    frame.area(),
                    &ToastsRenderCtx {
                        now,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })
            .expect("toast render test draw should complete");

        RenderedToast {
            card:   toasts.hits[0].card_rect,
            buffer: terminal.backend().buffer().clone(),
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn rendered_plain_toast(action: TestToastAction, height: u16) -> (u16, Vec<String>) {
        let table: Table = "[toasts]\nwidth = 14\n"
            .parse()
            .expect("toast settings TOML should parse");
        let settings = ToastSettings::from_table(&table).expect("toast settings should load");
        let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
        match action {
            TestToastAction::Absent => {
                let _ = toasts.push("notice", "abcdef ghijkl mnopqr");
            },
            TestToastAction::Open => {
                let _ = toasts.push_with_action(
                    "notice",
                    "abcdef ghijkl mnopqr",
                    TestToastAction::Open,
                );
            },
        }
        let views = toasts.active_views(Instant::now() + Duration::from_secs(1));
        let backend = TestBackend::new(16, height);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");
        let mut result = None;

        terminal
            .draw(|frame| {
                result = Some(render_toasts(
                    frame,
                    frame.area(),
                    &views,
                    &settings,
                    PaneFocusState::Inactive,
                    None,
                ));
            })
            .expect("toast render test draw should complete");

        let card_height = result
            .expect("toast render should return hitboxes")
            .hitboxes[0]
            .card_rect
            .height;
        let buffer = terminal.backend().buffer();
        let rows = (buffer.area.top()..buffer.area.bottom())
            .map(|row| {
                (buffer.area.left()..buffer.area.right())
                    .map(|column| buffer[(column, row)].symbol())
                    .collect()
            })
            .collect();
        (card_height, rows)
    }

    #[test]
    fn a_toast_is_as_tall_as_its_drawn_body() {
        let (plain_height, plain_rows) = rendered_plain_toast(TestToastAction::Absent, 20);
        assert_eq!(plain_height, 5);
        for word in ["abcdef", "ghijkl", "mnopqr"] {
            assert!(plain_rows.iter().any(|row| row.contains(word)));
        }

        let (action_height, action_rows) = rendered_plain_toast(TestToastAction::Open, 20);
        assert_eq!(action_height, 6);
        for text in ["abcdef", "ghijkl", "mnopqr", "Enter open"] {
            assert!(action_rows.iter().any(|row| row.contains(text)));
        }
    }

    #[test]
    fn a_toast_in_a_narrow_area_is_as_tall_as_its_drawn_body() {
        let body = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234";
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let id = toasts.push("notice", body);
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let rendered = draw_at(&mut toasts, 30, 20, settled_at);

        assert_eq!(rendered.card.height, 5);
        let drawn_body = (rendered.card.y + 1..rendered.card.bottom() - 1)
            .map(|row| {
                (rendered.card.x + 2..rendered.card.right() - 2)
                    .map(|column| rendered.buffer[(column, row)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<String>();
        assert_eq!(drawn_body, body);
        assert!(!(rendered.card.y..rendered.card.bottom()).any(|row| {
            (rendered.card.x..rendered.card.right())
                .any(|column| rendered.buffer[(column, row)].symbol() == ELISION)
        }));
    }

    #[test]
    fn an_entering_toast_in_a_narrow_area_grows_to_its_drawn_height() {
        let body = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234";
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let backend = TestBackend::new(30, 20);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");
        terminal
            .draw(|frame| {
                Renderable::render(
                    &mut toasts,
                    frame,
                    frame.area(),
                    &ToastsRenderCtx {
                        now:              Instant::now(),
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })
            .expect("empty toast render should complete");
        let id = toasts.push("notice", body);
        let created_at = created_at(&toasts, id);

        assert_eq!(draw_at(&mut toasts, 30, 20, created_at).card.height, 3);
        assert_eq!(
            draw_at(&mut toasts, 30, 20, created_at + Duration::from_secs(1))
                .card
                .height,
            5
        );
    }

    #[test]
    fn a_toast_regains_its_height_when_the_area_widens() {
        let body = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234";
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let id = toasts.push("notice", body);
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);

        assert_eq!(draw_at(&mut toasts, 30, 20, settled_at).card.height, 5);
        assert_eq!(draw_at(&mut toasts, 62, 20, settled_at).card.height, 3);
    }

    #[test]
    fn a_coloured_multiline_toast_body_wraps_each_line_in_its_colour() {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(12));
        let id = toasts.push_colored_persistent(
            "notice",
            vec!["red".to_owned(), "cyan tail".to_owned()],
            vec![Color::Red, Color::Cyan],
        );
        let settled_at = created_at(&toasts, id.toast_id()) + Duration::from_secs(1);
        let rendered = draw_at(&mut toasts, 12, 20, settled_at);

        let body_row = |row| {
            (rendered.card.x + 2..rendered.card.right() - 2)
                .map(|column| rendered.buffer[(column, row)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        };
        assert_eq!(body_row(rendered.card.y + 1), "red");
        assert_eq!(body_row(rendered.card.y + 2), "cyan");
        assert_eq!(body_row(rendered.card.y + 3), "tail");
        assert_eq!(
            rendered.buffer[(rendered.card.x + 2, rendered.card.y + 1)].fg,
            Color::Red
        );
        for row in rendered.card.y + 2..rendered.card.y + 4 {
            assert_eq!(rendered.buffer[(rendered.card.x + 2, row)].fg, Color::Cyan);
        }
    }

    #[test]
    fn a_toast_body_keeps_padded_columns_aligned() {
        let first = format!("{:<4}  {} {:>3}%", "A", "=", 5);
        let second = format!("{:<4}  {} {:>3}%", "Long", "=", 100);
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(16));
        let id = toasts.push("notice", format!("{first}\n{second}"));
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let rendered = draw_at(&mut toasts, 18, 10, settled_at);
        let body_row = |row| {
            (rendered.card.x + 2..rendered.card.right() - 2)
                .map(|column| rendered.buffer[(column, row)].symbol())
                .collect::<String>()
        };

        assert_eq!(body_row(rendered.card.y + 1), "A     =   5%");
        assert_eq!(body_row(rendered.card.y + 2), "Long  = 100%");
    }

    #[test]
    fn a_toast_body_has_a_cell_of_padding_each_side() {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(12));
        let id = toasts.push("notice", "abcdefgh");
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let rendered = draw_at(&mut toasts, 14, 10, settled_at);
        let row = rendered.card.y + 1;

        assert_eq!(rendered.buffer[(rendered.card.x + 1, row)].symbol(), " ");
        assert_eq!(rendered.buffer[(rendered.card.x + 2, row)].symbol(), "a");
        assert_eq!(
            (rendered.card.x + 2..rendered.card.right() - 2)
                .map(|column| rendered.buffer[(column, row)].symbol())
                .collect::<String>(),
            "abcdefgh"
        );
        assert_eq!(
            rendered.buffer[(rendered.card.right() - 2, row)].symbol(),
            " "
        );
    }

    #[test]
    fn a_toast_takes_the_whole_width_when_little_is_left_beside_it() {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let id = toasts.push("notice", "body");
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let rendered = draw_at(&mut toasts, 69, 10, settled_at);

        assert_eq!(rendered.card.width, 67);
        assert_eq!(rendered.card.x, 1);
    }

    #[test]
    fn a_toast_keeps_its_configured_width_when_eight_cells_remain() {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let id = toasts.push("notice", "body");
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let rendered = draw_at(&mut toasts, 70, 10, settled_at);

        assert_eq!(rendered.card.width, 60);
        assert_eq!(rendered.card.x, 9);
    }

    #[test]
    fn a_full_width_toast_does_not_clear_left_of_its_offset_area() {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(12));
        let id = toasts.push("notice", "body");
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let area = Rect::new(5, 1, 12, 10);
        let guard = Rect::new(area.x - 1, area.bottom() - 3, 1, 1);
        let backend = TestBackend::new(24, 12);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");

        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("G"), guard);
                Renderable::render(
                    &mut toasts,
                    frame,
                    area,
                    &ToastsRenderCtx {
                        now:              settled_at,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })
            .expect("offset toast render should complete");

        assert_eq!(
            terminal.backend().buffer()[(guard.x, guard.y)].symbol(),
            "G"
        );
        assert_eq!(toasts.hits[0].card_rect, Rect::new(6, 8, 10, 3));
    }

    #[test]
    fn a_toast_keeps_one_cleared_cell_on_each_side() {
        for (configured_width, area_width, expected_card_width) in [(12, 30, 12), (60, 20, 18)] {
            let area = Rect::new(5, 1, area_width, 10);
            let mut toasts =
                Toasts::<TestApp>::with_settings(settings_with_width(configured_width));
            let id = toasts.push("notice", "body");
            let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
            let backend = TestBackend::new(40, 12);
            let mut terminal =
                Terminal::new(backend).expect("toast render test terminal should initialize");

            terminal
                .draw(|frame| {
                    let painted = vec![Line::from("G".repeat(40)); 12];
                    frame.render_widget(Paragraph::new(painted), frame.area());
                    Renderable::render(
                        &mut toasts,
                        frame,
                        area,
                        &ToastsRenderCtx {
                            now:              settled_at,
                            pane_focus_state: PaneFocusState::Inactive,
                        },
                    );
                })
                .expect("offset toast render should complete");

            let card = toasts.hits[0].card_rect;
            let row = card.y.saturating_add(1);
            assert_eq!(card.width, expected_card_width);
            assert!(card.x > area.x);
            assert!(card.right() < area.right());
            assert_eq!(terminal.backend().buffer()[(card.x - 1, row)].symbol(), " ");
            assert_eq!(
                terminal.backend().buffer()[(card.right(), row)].symbol(),
                " "
            );
            assert_eq!(terminal.backend().buffer()[(area.x - 1, row)].symbol(), "G");
            assert_eq!(
                terminal.backend().buffer()[(area.right(), row)].symbol(),
                "G"
            );
        }
    }

    #[test]
    fn a_toast_joins_frame_columns_across_its_cleared_gaps() {
        let area = Rect::new(5, 1, 20, 10);
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let id = toasts.push("notice", "body");
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let backend = TestBackend::new(30, 12);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");

        terminal
            .draw(|frame| {
                for row in area.top()..area.bottom() {
                    frame.buffer_mut()[(area.left() - 1, row)].set_symbol(line::VERTICAL_RIGHT);
                    frame.buffer_mut()[(area.right(), row)].set_symbol(line::VERTICAL_LEFT);
                }
                Renderable::render(
                    &mut toasts,
                    frame,
                    area,
                    &ToastsRenderCtx {
                        now:              settled_at,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })
            .expect("toast over frame columns should render");

        let card = toasts.hits[0].card_rect;
        let buffer = terminal.backend().buffer();
        for row in card.top()..card.bottom() {
            assert_eq!(buffer[(area.left() - 1, row)].symbol(), line::VERTICAL);
            assert_eq!(buffer[(area.right(), row)].symbol(), line::VERTICAL);
            assert_eq!(buffer[(card.left() - 1, row)].symbol(), " ");
            assert_eq!(buffer[(card.right(), row)].symbol(), " ");
        }
        assert_eq!(
            buffer[(area.left() - 1, card.top() - 1)].symbol(),
            line::VERTICAL_RIGHT
        );
        assert_eq!(
            buffer[(area.right(), card.top() - 1)].symbol(),
            line::VERTICAL_LEFT
        );
    }

    #[test]
    fn a_narrow_right_aligned_toast_keeps_the_far_frame_tee() {
        let area = Rect::new(5, 1, 30, 10);
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(12));
        let id = toasts.push("notice", "body");
        let settled_at = created_at(&toasts, id) + Duration::from_secs(1);
        let backend = TestBackend::new(40, 12);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");

        terminal
            .draw(|frame| {
                for row in area.top()..area.bottom() {
                    frame.buffer_mut()[(area.left() - 1, row)].set_symbol(line::VERTICAL_RIGHT);
                    frame.buffer_mut()[(area.right(), row)].set_symbol(line::VERTICAL_LEFT);
                }
                Renderable::render(
                    &mut toasts,
                    frame,
                    area,
                    &ToastsRenderCtx {
                        now:              settled_at,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })
            .expect("narrow toast over frame columns should render");

        let card = toasts.hits[0].card_rect;
        let buffer = terminal.backend().buffer();
        for row in card.top()..card.bottom() {
            assert_eq!(
                buffer[(area.left() - 1, row)].symbol(),
                line::VERTICAL_RIGHT
            );
            assert_eq!(buffer[(area.right(), row)].symbol(), line::VERTICAL);
            assert_eq!(buffer[(card.left() - 1, row)].symbol(), " ");
            assert_eq!(buffer[(card.right(), row)].symbol(), " ");
        }
    }

    #[test]
    fn a_toast_body_without_room_ends_in_the_mark() {
        let (_, rows) = rendered_plain_toast(TestToastAction::Absent, 4);
        let last_body_row = rows[2].chars().skip(2).take(12).collect::<String>();

        assert!(last_body_row.trim_end().ends_with(ELISION));
    }

    #[test]
    fn configured_toast_gap_does_not_insert_blank_rows_between_cards() {
        let table: Table = "[toasts]\ngap = 1\n"
            .parse()
            .expect("toast settings TOML should parse");
        let settings = ToastSettings::from_table(&table).expect("toast settings table should load");
        assert_eq!(settings.gap.get(), 0);
        let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
        let _ = toasts.push("one", "body");
        let _ = toasts.push("two", "body");
        let views = toasts.active_views(Instant::now());
        let backend = TestBackend::new(80, 20);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");
        let mut result = None;

        terminal
            .draw(|frame| {
                result = Some(render_toasts(
                    frame,
                    frame.area(),
                    &views,
                    &settings,
                    PaneFocusState::Inactive,
                    None,
                ));
            })
            .expect("toast render test draw should complete");

        let hitboxes = result
            .expect("render_toasts should produce render result")
            .hitboxes;
        assert_eq!(hitboxes.len(), 2);
        assert_eq!(
            hitboxes[0].card_rect.bottom(),
            hitboxes[1].card_rect.y,
            "toast cards should be adjacent with no blank row"
        );
    }

    #[test]
    fn tracked_items_show_overflow_row_when_body_is_constrained() {
        let tracked = ["one", "two", "three", "four"]
            .into_iter()
            .map(|label| TrackedItemView {
                label:           label.to_string(),
                linger_progress: None,
                elapsed:         None,
                activity:        TrackedItemActivity::Progressing,
            })
            .collect::<Vec<_>>();

        let lines = card::body_lines_tracked(&tracked, Style::default(), 2, 20);

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "one");
        assert_eq!(line_text(&lines[1]), "(+3 more)");
    }

    #[test]
    fn tracked_task_toast_height_fits_visible_items_without_blank_bottom_row() {
        let settings = ToastSettings::default();
        let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
        let task_id = toasts.start_task("Checks", "");
        let items = [
            TrackedItem::new("~/work/service-api", "service-api"),
            TrackedItem::new("~/work/service-ui", "service-ui"),
        ];
        assert!(toasts.set_tracked_items(task_id, &items));

        let views = toasts.active_views(Instant::now() + Duration::from_secs(5));
        let backend = TestBackend::new(90, 20);
        let mut terminal =
            Terminal::new(backend).expect("toast render test terminal should initialize");
        let mut result = None;

        terminal
            .draw(|frame| {
                result = Some(render_toasts(
                    frame,
                    frame.area(),
                    &views,
                    &settings,
                    PaneFocusState::Inactive,
                    None,
                ));
            })
            .expect("toast render test draw should complete");

        let hitboxes = result
            .expect("render_toasts should produce render result")
            .hitboxes;
        assert_eq!(hitboxes.len(), 1);
        assert_eq!(
            hitboxes[0].card_rect.height, 4,
            "two tracked rows should render as top border + two rows + bottom border"
        );
    }

    #[test]
    fn fallback_toast_palette_is_pinned_to_safe_defaults() {
        // Locks the safety-pinned toast colors against drift. Plain
        // (info) toast borders and titles read from the active theme
        // (see `default_pane_chrome`); only the always-legible error
        // and warning colors live here.
        let p = fallback_toast_palette();
        assert_eq!(p.accent, Color::Cyan);
        assert_eq!(p.error, Color::Red);
        assert_eq!(p.success, Color::Green);
        assert_eq!(p.warning, Color::Yellow);
        assert_eq!(p.label, Color::Rgb(150, 190, 180));
        assert_eq!(p.title, Color::Yellow);
    }

    #[test]
    fn tracked_item_running_line_uses_framework_activity_spinner() {
        let elapsed = Duration::from_millis(100);
        let item = TrackedItemView {
            label:           "repo".to_string(),
            linger_progress: None,
            elapsed:         Some(elapsed),
            activity:        TrackedItemActivity::Progressing,
        };

        let line = card::tracked_item_line(&item, Style::default(), 40);

        assert!(line_text(&line).contains(ACTIVITY_SPINNER.frame_at(elapsed)));
    }

    #[test]
    fn stalled_tracked_item_spinner_takes_the_palette_error_color() {
        let spinner_color = |activity| {
            let item = TrackedItemView {
                label: "repo".to_string(),
                linger_progress: None,
                elapsed: Some(Duration::from_millis(100)),
                activity,
            };
            card::tracked_item_line(&item, Style::default(), 40)
                .spans
                .iter()
                .find_map(|span| {
                    span.content
                        .contains(ACTIVITY_SPINNER.frame_at(Duration::from_millis(100)))
                        .then_some(span.style.fg)
                })
                .flatten()
        };

        let palette = fallback_toast_palette();
        assert_eq!(
            spinner_color(TrackedItemActivity::Progressing),
            Some(palette.accent)
        );
        assert_eq!(
            spinner_color(TrackedItemActivity::Stalled),
            Some(palette.error)
        );
    }
}
