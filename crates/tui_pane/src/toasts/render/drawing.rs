use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;

use super::card;
use super::layout;
use super::layout::StackLayout;
use crate::BLOCK_BORDER_WIDTH;
use crate::PaneFocusState;
use crate::ToastPlacement;
use crate::ToastSettings;
use crate::constants::MIN_PANE_ROWS_WITH_EDGE_GAP;
use crate::constants::TOAST_ROW_GAP;
use crate::constants::TOAST_SIDE_GAP;
use crate::toasts::ToastFocus;
use crate::toasts::ToastHitbox;
use crate::toasts::ToastId;
use crate::toasts::ToastStyle;
use crate::toasts::ToastView;
use crate::toasts::body;
use crate::toasts::manager;
use crate::toasts::view::ToastExitState;

/// Fallback colours for transparent cards and activity details.
///
/// Opaque cards use theme colours over the theme background.
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

/// The rows that the toast stack over a tile grid keeps between draws.
///
/// `Toasts::render_over_grid` reads and replaces it on each draw. The rows
/// stay the same while the grid viewport stays the same, no new toast joins
/// the stack, every card keeps its settled height, and no protected frame
/// foot reaches the cards. Grid cells that change height do not move a card.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::toasts) enum GridToastStack {
    /// The last draw over the grid has no rows to keep: it drew no card, or a card
    /// had less than its settled height.
    Unplaced,
    /// The last draw put every card at its settled height in `area`.
    Placed {
        /// The grid viewport of the draw that selected `area`.
        viewport: Rect,
        /// The active toasts when `area` was selected.
        toasts:   Vec<ToastId>,
        /// The rows between pane borders that hold the cards.
        area:     Rect,
        /// The cells of every card that the last draw put in `area`.
        cards:    Rect,
    },
}

/// The rows that a toast stack draws its cards in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToastStackBounds {
    /// Find the rows between pane borders inside this area.
    Search(Rect),
    /// Use the rows that an earlier draw found.
    Held(Rect),
}

/// Where a draw put its toast cards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToastStackPlacement {
    /// The draw put no card on screen.
    NoCards,
    /// A visible toast got less than its settled height, or no card.
    Cramped,
    /// Every visible toast got a card at its settled height.
    Settled {
        /// The rows between pane borders that hold the cards.
        area:  Rect,
        /// The cells of every drawn card.
        cards: Rect,
    },
}

impl ToastStackBounds {
    /// Return the rows to draw in: the kept rows, or the rows found between pane borders.
    fn area(self, frame: &mut Frame, settings: &ToastSettings, required_height: u16) -> Rect {
        match self {
            Self::Search(area) => {
                card::area_between_pane_borders(frame, area, settings, required_height)
            },
            Self::Held(area) => area,
        }
    }
}

impl ToastStackPlacement {
    /// Classify the drawn cards in `area`. `complete_toasts` holds the settled heights that
    /// `allocated` gave rows to.
    fn of_cards(
        area: Rect,
        hitboxes: &[ToastHitbox],
        complete_toasts: &[ToastView],
        allocated: &[u16],
    ) -> Self {
        let settled = hitboxes.len() == complete_toasts.len()
            && complete_toasts
                .iter()
                .zip(allocated)
                .all(|(toast, allocated)| *allocated >= toast.desired_height());
        match hitboxes
            .iter()
            .map(|hitbox| hitbox.card_rect)
            .reduce(Rect::union)
        {
            None => Self::NoCards,
            Some(cards) if settled => Self::Settled { area, cards },
            Some(_) => Self::Cramped,
        }
    }
}

/// Result of rendering toast cards.
struct ToastRenderResult {
    /// Hitboxes for the toast card and close-button regions rendered in this pass.
    hitboxes:  Vec<ToastHitbox>,
    /// The rows that hold the cards of this pass.
    placement: ToastStackPlacement,
}

/// The active toasts of one draw and where the draw put their cards.
struct DrawnToastStack {
    /// Every active toast, also a toast that had no room for a card.
    views:        Vec<ToastView>,
    /// The rows that the draw searched or kept.
    stack_bounds: ToastStackBounds,
    /// Where the draw put the cards of `views`.
    placement:    ToastStackPlacement,
}

/// Render toast cards and return their hit-test regions.
fn render_toasts(
    frame: &mut Frame,
    stack_bounds: ToastStackBounds,
    toasts: &[ToastView],
    settings: &ToastSettings,
    focus: ToastFocus,
) -> ToastRenderResult {
    if !settings.toasts_enabled() || toasts.is_empty() {
        return ToastRenderResult {
            hitboxes:  Vec::new(),
            placement: ToastStackPlacement::NoCards,
        };
    }

    let max_visible = settings.max_visible.get().max(1);
    let start = toasts.len().saturating_sub(max_visible);
    let visible_toasts = &toasts[start..];
    let mut complete_toasts: Vec<_> = visible_toasts
        .iter()
        .map(|toast| {
            let mut complete = toast.clone();
            complete.desired_height = toast.settled_height();
            complete
        })
        .collect();
    let gap = TOAST_ROW_GAP;
    let required_height = complete_toasts
        .iter()
        .map(ToastView::desired_height)
        .fold(TOAST_ROW_GAP.saturating_mul(2), u16::saturating_add)
        .saturating_add(gap.saturating_mul(
            u16::try_from(visible_toasts.len().saturating_sub(1)).unwrap_or(u16::MAX),
        ));
    let area = stack_bounds.area(frame, settings, required_height);
    let available = area
        .height
        .saturating_sub(stack_gap_rows(frame.area(), area, required_height));
    let width = stack_card_width(visible_toasts, settings, area);
    for toast in &mut complete_toasts {
        if toast.exit_state == ToastExitState::Visible {
            toast.min_height = toast.min_height.max(first_sentence_height(toast, width));
            toast.desired_height = toast.desired_height.max(toast.min_height);
        }
    }
    let allocated = layout::allocate_toast_heights(&complete_toasts, available, gap);
    let visible_toasts: Vec<_> = visible_toasts
        .iter()
        .map(|toast| {
            let mut visible = toast.clone();
            if toast.exit_state == ToastExitState::Visible {
                visible.desired_height = visible
                    .desired_height
                    .max(first_sentence_height(toast, width));
            }
            visible
        })
        .collect();
    let drawn_height = visible_toasts
        .iter()
        .zip(&allocated)
        .map(|(toast, allocated)| toast.desired_height().min(*allocated))
        .filter(|height| *height > 0)
        .reduce(|height, next| height.saturating_add(gap).saturating_add(next))
        .unwrap_or(0);
    let edge_gap = area.height.saturating_sub(drawn_height).min(TOAST_ROW_GAP);
    let layout = StackLayout {
        width,
        gap,
        edge_gap,
        focus,
    };
    let hitboxes = match settings.placement {
        ToastPlacement::TopRight => {
            layout::render_top_down(frame, area, &visible_toasts, &allocated, layout)
        },
        ToastPlacement::BottomRight => {
            layout::render_bottom_up(frame, area, &visible_toasts, &allocated, layout)
        },
    };
    let placement = ToastStackPlacement::of_cards(area, &hitboxes, &complete_toasts, &allocated);

    ToastRenderResult {
        hitboxes,
        placement,
    }
}

/// Rows of `area` that the stack keeps empty around its cards.
fn stack_gap_rows(frame_area: Rect, area: Rect, required_height: u16) -> u16 {
    let minimum_card_height =
        u16::try_from(BLOCK_BORDER_WIDTH.saturating_add(1)).unwrap_or(u16::MAX);
    if area == frame_area {
        (area.height.saturating_sub(minimum_card_height) / 2)
            .min(TOAST_ROW_GAP)
            .saturating_mul(2)
    } else {
        let stack_height = required_height.saturating_sub(TOAST_ROW_GAP.saturating_mul(2));
        let spare_rows = area
            .height
            .saturating_sub(stack_height)
            .min(TOAST_ROW_GAP.saturating_mul(2));
        if area.height >= MIN_PANE_ROWS_WITH_EDGE_GAP {
            spare_rows.max(TOAST_ROW_GAP)
        } else {
            spare_rows
        }
    }
}

/// The card width that fits every visible title in `area`.
fn stack_card_width(visible_toasts: &[ToastView], settings: &ToastSettings, area: Rect) -> u16 {
    let width = visible_toasts
        .iter()
        .map(card::title_card_width)
        .fold(manager::toast_card_width(settings, area.width), u16::max)
        .min(area.width.saturating_sub(TOAST_SIDE_GAP.saturating_mul(2)));
    visible_toasts
        .iter()
        .filter(|toast| toast.style() != ToastStyle::Normal)
        .map(card::minimum_title_card_width)
        .filter(|required| *required <= area.width.saturating_sub(TOAST_SIDE_GAP))
        .fold(width, u16::max)
}

/// Keep the first sentence together when the card has enough space.
fn first_sentence_height(toast: &ToastView, card_width: u16) -> u16 {
    if !toast.tracked_items().is_empty() {
        return toast.min_height();
    }
    let text = toast.body();
    let sentence_end = text.char_indices().find_map(|(index, character)| {
        let end = index + character.len_utf8();
        (matches!(character, '.' | '?' | '!')
            && text[end..].chars().next().is_none_or(char::is_whitespace))
        .then_some(end)
    });
    let Some(end) = sentence_end else {
        return toast.min_height();
    };
    let mut sentence = text[..end]
        .chars()
        .filter(|character| !character.is_whitespace())
        .peekable();
    let width = u16::try_from(body::card_body_width(card_width)).unwrap_or(u16::MAX);
    let mut rows = 0_u16;
    for line in toast.render_body.wrapped_lines(width, Style::default()) {
        rows = rows.saturating_add(1);
        for character in line.spans.iter().flat_map(|span| span.content.chars()) {
            if sentence.peek() == Some(&character) {
                sentence.next();
            }
        }
        if sentence.peek().is_none() {
            break;
        }
    }
    rows.saturating_add(2)
        .saturating_add(u16::from(toast.has_action()))
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
        self.render_stack(frame, area.width, ctx, |_, _| {
            ToastStackBounds::Search(area)
        });
        None
    }
}

impl<Ctx: crate::AppContext> super::super::Toasts<Ctx> {
    /// Draw toasts over a tile grid and avoid its protected frame feet.
    ///
    /// The first draw of a stack selects its rows. Later draws keep those rows
    /// while `viewport` stays the same, no new toast arrives, every card keeps
    /// its settled height and no protected frame foot reaches the cards. A card
    /// and its close button stay in place while grid cells change height.
    pub(crate) fn render_over_grid(
        &mut self,
        frame: &mut Frame<'_>,
        viewport: Rect,
        protected_frames: &[Rect],
        ctx: &ToastsRenderCtx,
    ) {
        let drawn = self.render_stack(frame, viewport.width, ctx, |toasts, active| {
            toasts
                .grid_stack
                .bounds(viewport, active, protected_frames, || {
                    toasts.area_avoiding_frames(viewport, protected_frames)
                })
        });
        self.grid_stack.record_draw(viewport, &drawn);
    }

    /// Draw the active toasts in the rows that `select_bounds` gives and store their hitboxes.
    fn render_stack(
        &mut self,
        frame: &mut Frame<'_>,
        area_width: u16,
        ctx: &ToastsRenderCtx,
        select_bounds: impl FnOnce(&Self, &[ToastView]) -> ToastStackBounds,
    ) -> DrawnToastStack {
        self.set_draw_area_width(area_width);
        let focus = self.focused_toast_id(ctx.pane_focus_state);
        let views = self.active_views(ctx.now);
        let stack_bounds = select_bounds(self, &views);
        let result = render_toasts(frame, stack_bounds, &views, self.settings(), focus);
        self.set_hits(result.hitboxes);
        DrawnToastStack {
            views,
            stack_bounds,
            placement: result.placement,
        }
    }

    /// Return the body width inside a card constrained to this area.
    #[must_use]
    pub fn body_width_for_area(&self, area: Rect) -> usize {
        body::card_body_width(manager::toast_card_width(self.settings(), area.width))
    }

    /// Return the configured placement corner within the allowed area.
    #[must_use]
    pub const fn placement_corner(&self, area: Rect) -> Position {
        let y = match self.settings().placement {
            ToastPlacement::TopRight => area.top(),
            ToastPlacement::BottomRight => area.bottom().saturating_sub(1),
        };
        Position::new(area.right().saturating_sub(1), y)
    }

    /// Keep toast cards and their cleared gaps outside protected frame feet.
    /// Frames in other columns leave the toast's original area available.
    #[must_use]
    pub fn area_avoiding_frames(&self, mut area: Rect, frames: &[Rect]) -> Rect {
        let width = manager::toast_card_width(self.settings(), area.width);
        let left = area
            .right()
            .saturating_sub(width)
            .saturating_sub(TOAST_SIDE_GAP.saturating_mul(2))
            .max(area.x);
        for frame in frames {
            if frame.right() <= left
                || frame.left() >= area.right()
                || frame.bottom() <= area.top()
                || frame.top() >= area.bottom()
            {
                continue;
            }
            let foot = foot_row(*frame);
            let above = foot.saturating_sub(area.top()).min(area.height);
            let below = area.bottom().saturating_sub(frame.bottom());
            if below > above {
                area.y = frame.bottom();
                area.height = below;
            } else {
                area.height = above;
            }
        }
        area
    }
}

impl GridToastStack {
    /// Keep the placed rows while the viewport, the toasts and the protected frame feet allow it;
    /// else search again.
    fn bounds(
        &self,
        viewport: Rect,
        active: &[ToastView],
        protected_frames: &[Rect],
        search_area: impl FnOnce() -> Rect,
    ) -> ToastStackBounds {
        match self {
            Self::Placed {
                viewport: placed_viewport,
                toasts,
                area,
                cards,
            } if *placed_viewport == viewport
                && active.iter().all(|view| toasts.contains(&view.id()))
                && !protected_frames
                    .iter()
                    .any(|frame| foot_reaches_cards(*area, *frame, *cards)) =>
            {
                ToastStackBounds::Held(*area)
            },
            Self::Placed { .. } | Self::Unplaced => ToastStackBounds::Search(search_area()),
        }
    }

    /// Record a draw over the grid in `viewport`. A kept draw updates only its card cells.
    fn record_draw(&mut self, viewport: Rect, drawn: &DrawnToastStack) {
        match (drawn.stack_bounds, drawn.placement) {
            (_, ToastStackPlacement::NoCards | ToastStackPlacement::Cramped) => {
                *self = Self::Unplaced;
            },
            (ToastStackBounds::Held(_), ToastStackPlacement::Settled { cards, .. }) => {
                if let Self::Placed {
                    cards: placed_cards,
                    ..
                } = self
                {
                    *placed_cards = cards;
                }
            },
            (ToastStackBounds::Search(_), ToastStackPlacement::Settled { area, cards }) => {
                *self = Self::Placed {
                    viewport,
                    toasts: drawn.views.iter().map(ToastView::id).collect(),
                    area,
                    cards,
                };
            },
        }
    }
}

/// The last row inside `frame`, where a protected frame draws its foot.
const fn foot_row(frame: Rect) -> u16 { crate::frame_inner(frame).bottom().saturating_sub(1) }

/// Whether the foot row of `frame` falls in the cells that a draw of `cards` in the stack
/// `area` clears. `card::render_toast` clamps the cleared gaps to `area`, so this does too.
fn foot_reaches_cards(area: Rect, frame: Rect, cards: Rect) -> bool {
    let cleared = card::cleared_surroundings(area, cards);
    let foot = foot_row(frame);
    frame.left() < cleared.right()
        && frame.right() > cleared.left()
        && (cleared.top()..cleared.bottom()).contains(&foot)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::convert::Infallible;
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use ratatui::symbols::line;
    use ratatui::symbols::line::VERTICAL_LEFT;
    use ratatui::symbols::line::VERTICAL_RIGHT;
    use ratatui::text::Line;
    use ratatui::widgets::Paragraph;
    use toml::Table;

    use super::*;
    use crate::ACTIVITY_SPINNER;
    use crate::AppContext;
    use crate::ColorMode;
    use crate::Framework;
    use crate::PaneFocusState;
    use crate::Renderable;
    use crate::ToastActivation;
    use crate::ToastId;
    use crate::ToastStyle;
    use crate::Toasts;
    use crate::TrackedItem;
    use crate::TrackedItemActivity;
    use crate::constants::ELISION;
    use crate::toasts::TrackedItemElapsed;
    use crate::toasts::TrackedItemLifecycle;
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
                    ToastStackBounds::Search(frame.area()),
                    &views,
                    &settings,
                    ToastFocus::PaneUnfocused,
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
    fn a_toast_area_avoids_only_crossing_frames_and_keeps_the_larger_region() {
        for placement in [ToastPlacement::TopRight, ToastPlacement::BottomRight] {
            let mut settings = settings_with_width(12);
            settings.placement = placement;
            let toasts = Toasts::<TestApp>::with_settings(settings);
            let area = Rect::new(1, 1, 60, 30);
            let outside = Rect::new(1, 1, 15, 10);
            assert_eq!(toasts.area_avoiding_frames(area, &[outside]), area);
            let short = Rect::new(30, 1, 31, 6);
            assert_eq!(
                toasts.area_avoiding_frames(area, &[outside, short]),
                Rect::new(1, 7, 60, 24)
            );
            let tall = Rect::new(30, 1, 31, 25);
            assert_eq!(
                toasts.area_avoiding_frames(area, &[outside, tall]),
                Rect::new(1, 1, 60, 23)
            );
        }
    }

    #[test]
    fn a_toast_keeps_pane_titles_and_connected_borders_without_blank_rows() -> Result<(), Infallible>
    {
        for width in [32, 60] {
            let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(width));
            let id = toasts.push("Startup", "Read projects");
            let now = created_at(&toasts, id) + Duration::from_secs(1);
            let mut terminal = Terminal::new(TestBackend::new(60, 16))?;
            terminal.draw(|frame| {
                let chrome = crate::default_pane_chrome(ColorMode::Colored);
                frame.render_widget(
                    chrome.block("Projects".to_string(), false),
                    Rect::new(0, 0, 30, 16),
                );
                frame.render_widget(
                    chrome.block("No lint runs".to_string(), false),
                    Rect::new(30, 11, 30, 5),
                );
                Renderable::render(
                    &mut toasts,
                    frame,
                    frame.area(),
                    &ToastsRenderCtx {
                        now,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })?;
            let buffer = terminal.backend().buffer();
            let title: String = (30..60).map(|x| buffer[(x, 11)].symbol()).collect();
            assert!(
                title.contains("No lint runs"),
                "the pane title is cut: {title:?}"
            );
            for y in 1..15 {
                assert!(
                    (0..60).any(|x| !buffer[(x, y)].symbol().trim().is_empty()),
                    "blank row {y}"
                );
                for x in 0..60 {
                    if buffer[(x, y)].symbol() == line::VERTICAL {
                        assert!(
                            [y - 1, y + 1].into_iter().any(|neighbor| {
                                matches!(
                                    buffer[(x, neighbor)].symbol(),
                                    "│" | "┌"
                                        | "┐"
                                        | "└"
                                        | "┘"
                                        | "├"
                                        | "┤"
                                        | "╭"
                                        | "╮"
                                        | "╰"
                                        | "╯"
                                )
                            }),
                            "isolated border at {x},{y}"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn a_bottom_right_toast_keeps_a_gap_from_the_last_column_pane_border() -> Result<(), Infallible>
    {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        let id = toasts.start_task("Startup", "");
        let items = [
            "Cargo metadata",
            "crates.io",
            "Disk usage",
            "GitHub repos",
            "Languages",
            "Lint history",
            "Local git repos",
            "Test counts",
        ]
        .map(|label| TrackedItem::new(label, label));
        assert!(toasts.set_tracked_items(id, &items));
        let now = Instant::now() + Duration::from_secs(5);
        let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
        let pane = Rect::new(64, 33, 96, 11);
        terminal.draw(|frame| {
            let chrome = crate::default_pane_chrome(ColorMode::Colored);
            for (title, area) in [
                ("Projects", Rect::new(0, 0, 64, 33)),
                ("Package", Rect::new(64, 0, 96, 33)),
                ("No lint runs", Rect::new(0, 33, 64, 11)),
                ("CI runs — not a git repository", pane),
            ] {
                frame.render_widget(chrome.block(title.to_string(), false), area);
            }
            Renderable::render(
                &mut toasts,
                frame,
                Rect::new(0, 0, 160, 44),
                &ToastsRenderCtx {
                    now,
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
        })?;
        let card = toasts.hits[0].card_rect;
        let buffer = terminal.backend().buffer();
        for row in [pane.top(), pane.bottom() - 1] {
            assert!(
                !(card.top()..card.bottom()).contains(&row),
                "toast {card:?} shares the pane border row {row}"
            );
        }
        for row in card.top()..card.bottom() {
            assert_eq!(buffer[(card.right(), row)].symbol(), " ", "row {row}");
            let edge: String = (card.right() - 1..=card.right())
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            for pair in ["││", "┐┐", "┘│"] {
                assert!(!edge.contains(pair), "row {row}: {edge}");
            }
        }
        let title: String = (pane.left()..pane.right())
            .map(|column| buffer[(column, pane.top())].symbol())
            .collect();
        assert!(title.contains("CI runs — not a git repository"), "{title}");
        for row in 0..44 {
            let text: String = (0..160)
                .map(|column| buffer[(column, row)].symbol())
                .collect();
            assert!(!text.trim().is_empty(), "blank row {row}");
        }
        Ok(())
    }

    #[test]
    fn a_bottom_right_stack_stays_below_the_pane_border_when_rows_are_hidden()
    -> Result<(), Infallible> {
        let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
        toasts.push(
            "Startup",
            "Metadata\nDisk\nLanguages\nGit\nTests\nLints\nRegistry\nTargets",
        );
        let mut terminal = Terminal::new(TestBackend::new(160, 45))?;
        terminal.draw(|frame| {
            let chrome = crate::default_pane_chrome(ColorMode::Colored);
            for (title, area) in [
                ("Languages", Rect::new(64, 15, 48, 18)),
                ("CPU cores 32", Rect::new(112, 15, 20, 18)),
                ("Targets", Rect::new(132, 15, 28, 18)),
                ("CI runs", Rect::new(64, 33, 96, 8)),
            ] {
                frame.render_widget(chrome.block(title.to_string(), false), area);
            }
            frame.render_widget(
                Paragraph::new("Files Code Comments"),
                Rect::new(90, 16, 21, 1),
            );
            frame.render_widget(Paragraph::new("CPU cores 32"), Rect::new(114, 16, 16, 1));
            frame.render_widget(
                Paragraph::new("Target Kind Source"),
                Rect::new(134, 16, 24, 1),
            );
            Renderable::render(
                &mut toasts,
                frame,
                Rect::new(0, 0, 160, 41),
                &ToastsRenderCtx {
                    now:              Instant::now() + Duration::from_secs(5),
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
        })?;
        let card = toasts.hits[0].card_rect;
        assert!(card.top() > 33 && card.bottom() <= 40, "{card:?}");
        let buffer = terminal.backend().buffer();
        let header: String = (90..158).map(|x| buffer[(x, 16)].symbol()).collect();
        for text in ["Files Code Comments", "CPU cores 32", "Target Kind Source"] {
            assert!(header.contains(text), "{header:?}");
        }
        assert!(
            (card.left()..card.right())
                .any(|x| buffer[(x, card.bottom() - 1)].symbol().contains('▼')),
            "the bottom border must show the shared page mark"
        );
        Ok(())
    }

    #[test]
    fn pane_borders_outside_the_card_keep_the_unrestricted_toast_rect() -> Result<(), Infallible> {
        for pane in [
            Rect::new(0, 0, 2, 23),
            Rect::new(40, 0, 23, 8),
            Rect::new(0, 8, 64, 8),
        ] {
            let mut toasts = Toasts::<TestApp>::with_settings(settings_with_width(60));
            let id = toasts.push_persistent(
                "Notice",
                "Read projects",
                ToastStyle::Normal,
                ToastActivation::Informational,
                3,
            );
            let now = created_at(&toasts, id) + Duration::from_secs(1);
            let mut terminal = Terminal::new(TestBackend::new(64, 24))?;
            terminal.draw(|frame| {
                let chrome = crate::default_pane_chrome(ColorMode::Colored);
                frame.render_widget(chrome.block(String::new(), false), pane);
                Renderable::render(
                    &mut toasts,
                    frame,
                    Rect::new(1, 1, 62, 21),
                    &ToastsRenderCtx {
                        now,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })?;
            let framed = toasts.hits[0].card_rect;
            assert_eq!(
                terminal.backend().buffer()[(framed.x, framed.y)].symbol(),
                "┌"
            );
            terminal.draw(|frame| {
                Renderable::render(
                    &mut toasts,
                    frame,
                    Rect::new(1, 1, 62, 21),
                    &ToastsRenderCtx {
                        now,
                        pane_focus_state: PaneFocusState::Inactive,
                    },
                );
            })?;
            let alone = toasts.hits[0].card_rect;
            assert_eq!(framed, alone);
            assert_eq!(
                terminal.backend().buffer()[(alone.x, alone.y)].symbol(),
                "┌"
            );
        }
        Ok(())
    }

    #[test]
    fn a_toast_keeps_one_cleared_row_above_and_below() {
        for placement in [ToastPlacement::TopRight, ToastPlacement::BottomRight] {
            for count in [1, 2] {
                let mut settings = settings_with_width(12);
                settings.placement = placement;
                let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
                for _ in 0..count {
                    let _ = toasts.push("notice", "body");
                }
                let views = toasts.active_views(Instant::now() + Duration::from_secs(1));
                let backend = TestBackend::new(30, 16);
                let mut terminal = Terminal::new(backend).expect("terminal should initialize");
                let mut hitboxes = Vec::new();
                terminal
                    .draw(|frame| {
                        let block = ratatui::widgets::Block::default()
                            .borders(ratatui::widgets::Borders::ALL);
                        let area = block.inner(frame.area());
                        frame.render_widget(block, frame.area());
                        frame.render_widget(
                            Paragraph::new(vec![Line::from("G".repeat(28)); 14]),
                            area,
                        );
                        hitboxes = render_toasts(
                            frame,
                            ToastStackBounds::Search(area),
                            &views,
                            &settings,
                            ToastFocus::PaneUnfocused,
                        )
                        .hitboxes;
                    })
                    .expect("toast stack should render");
                assert_eq!(hitboxes.len(), count);
                let buffer = terminal.backend().buffer();
                for hitbox in &hitboxes {
                    let card = hitbox.card_rect;
                    assert!(card.top() > 1);
                    assert!(card.bottom() < 15);
                    for row in [card.top() - 1, card.bottom()] {
                        for column in card.left() - 1..=card.right() {
                            assert_eq!(
                                buffer[(column, row)].symbol(),
                                " ",
                                "{placement:?}: {column},{row}"
                            );
                        }
                    }
                }
                for column in 1..29 {
                    assert_eq!(buffer[(column, 0)].symbol(), line::HORIZONTAL);
                    assert_eq!(buffer[(column, 15)].symbol(), line::HORIZONTAL);
                }
                for row in 1..15 {
                    assert_eq!(buffer[(0, row)].symbol(), line::VERTICAL);
                    assert_eq!(buffer[(29, row)].symbol(), line::VERTICAL);
                }
                if count == 2 {
                    assert_eq!(
                        hitboxes[0].card_rect.bottom() + 1,
                        hitboxes[1].card_rect.top()
                    );
                }
            }
        }
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
        let drawn_rows = (rendered.card.y + 1..rendered.card.bottom() - 1)
            .map(|row| {
                (rendered.card.x + 2..rendered.card.right() - 2)
                    .map(|column| rendered.buffer[(column, row)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        for row in &drawn_rows[..drawn_rows.len() - 1] {
            assert!(row.ends_with(ELISION), "{row}");
        }
        assert!(!drawn_rows.last().expect("last body row").contains(ELISION));
        let drawn_body = drawn_rows.join("").replace(ELISION, "");
        assert_eq!(drawn_body, body);
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
        assert_eq!(toasts.hits[0].card_rect, Rect::new(6, 7, 10, 3));
    }

    #[test]
    fn a_toast_keeps_one_cleared_cell_on_each_side() {
        for (configured_width, area_width, expected_card_width) in [(12, 30, 14), (60, 20, 18)] {
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
                    frame.buffer_mut()[(area.left() - 1, row)].set_symbol(VERTICAL_RIGHT);
                    frame.buffer_mut()[(area.right(), row)].set_symbol(VERTICAL_LEFT);
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
                    frame.buffer_mut()[(area.left() - 1, row)].set_symbol(VERTICAL_RIGHT);
                    frame.buffer_mut()[(area.right(), row)].set_symbol(VERTICAL_LEFT);
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
        let (_, rows) = rendered_plain_toast(TestToastAction::Absent, 6);
        let last_body_row = rows[3].chars().skip(2).take(12).collect::<String>();

        assert_eq!(last_body_row.trim(), "ghijkl");
        assert!(rows[4].contains('▼'), "{}", rows[4]);
    }

    #[test]
    fn a_stack_gap_is_charged_only_between_drawn_cards() {
        for placement in [ToastPlacement::TopRight, ToastPlacement::BottomRight] {
            let mut settings = settings_with_width(12);
            settings.placement = placement;
            let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
            let _ = toasts.push("older", "old");
            let newest = toasts.push("newer", "new");
            let views = toasts.active_views(Instant::now() + Duration::from_secs(1));
            let backend = TestBackend::new(30, 5);
            let mut terminal = Terminal::new(backend).expect("terminal should initialize");
            let mut hitboxes = Vec::new();
            terminal
                .draw(|frame| {
                    frame.render_widget(
                        Paragraph::new(vec![Line::from("G".repeat(30)); 5]),
                        frame.area(),
                    );
                    hitboxes = render_toasts(
                        frame,
                        ToastStackBounds::Search(frame.area()),
                        &views,
                        &settings,
                        ToastFocus::PaneUnfocused,
                    )
                    .hitboxes;
                })
                .expect("toast stack should render");
            assert_eq!(hitboxes.len(), 1, "{placement:?}");
            assert_eq!(hitboxes[0].id, newest);
            let card = hitboxes[0].card_rect;
            assert_eq!(card.y, 1);
            assert_eq!(card.height, 3);
            let buffer = terminal.backend().buffer();
            for row in [0, 4] {
                for column in card.left() - 1..=card.right() {
                    assert_eq!(buffer[(column, row)].symbol(), " ");
                }
            }
            assert_eq!(buffer[(card.left() + 2, 2)].symbol(), "n");
        }
    }

    #[test]
    fn a_toast_stack_keeps_one_cleared_row_between_cards() {
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
                    ToastStackBounds::Search(frame.area()),
                    &views,
                    &settings,
                    ToastFocus::PaneUnfocused,
                ));
            })
            .expect("toast render test draw should complete");

        let hitboxes = result
            .expect("render_toasts should produce render result")
            .hitboxes;
        assert_eq!(hitboxes.len(), 2);
        assert_eq!(
            hitboxes[0].card_rect.bottom() + 1,
            hitboxes[1].card_rect.y,
            "toast cards should have one cleared row between them"
        );
    }

    #[test]
    fn tracked_items_show_overflow_row_when_body_is_constrained() {
        let tracked = ["one", "two", "three", "four"]
            .into_iter()
            .map(|label| TrackedItemView {
                label:     label.to_string(),
                lifecycle: TrackedItemLifecycle::Running,
                elapsed:   TrackedItemElapsed::StartUnknown,
                activity:  TrackedItemActivity::Progressing,
            })
            .collect::<Vec<_>>();

        let lines = card::body_lines_tracked(&tracked, Style::default(), 2, 20, 2).lines;

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
                    ToastStackBounds::Search(frame.area()),
                    &views,
                    &settings,
                    ToastFocus::PaneUnfocused,
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
        // Keep the fallback colours stable for transparent cards and activity details.
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
            label:     "repo".to_string(),
            lifecycle: TrackedItemLifecycle::Running,
            elapsed:   TrackedItemElapsed::Measured(elapsed),
            activity:  TrackedItemActivity::Progressing,
        };

        let line = card::tracked_item_line(&item, Style::default(), 40);

        assert!(line_text(&line).contains(ACTIVITY_SPINNER.frame_at(elapsed)));
    }

    #[test]
    fn stalled_tracked_item_spinner_takes_the_palette_error_color() {
        let spinner_color = |activity| {
            let item = TrackedItemView {
                label: "repo".to_string(),
                lifecycle: TrackedItemLifecycle::Running,
                elapsed: TrackedItemElapsed::Measured(Duration::from_millis(100)),
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
