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

/// Compiled-in palette consumed by toast rendering.
///
/// Toasts deliberately do NOT read from the active theme: an error
/// toast must remain legible even if a user-loaded theme is corrupt
/// or its contrast is so low the toast would vanish. This palette is
/// fixed at compile time. The roundtrip test in `mod tests` locks
/// every field against accidental drift.
pub struct FallbackToastPalette {
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

pub const fn fallback_toast_palette() -> FallbackToastPalette {
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
    let width = settings.width.get().min(area.width);

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
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ToastsRenderCtx) {
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
    use ratatui::style::Style;
    use ratatui::text::Line;
    use toml::Table;

    use super::*;
    use crate::ACTIVITY_SPINNER;
    use crate::AppContext;
    use crate::Framework;
    use crate::NoToastAction;
    use crate::PaneFocusState;
    use crate::Toasts;
    use crate::TrackedItem;
    use crate::toasts::TrackedItemView;
    use crate::toasts::render::card;

    struct TestApp {
        framework: Framework<Self>,
    }

    impl AppContext for TestApp {
        type AppPaneId = ();
        type ToastAction = NoToastAction;

        fn framework(&self) -> &Framework<Self> { &self.framework }

        fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
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
        };

        let line = card::tracked_item_line(&item, Style::default(), 40);

        assert!(line_text(&line).contains(ACTIVITY_SPINNER.frame_at(elapsed)));
    }
}
