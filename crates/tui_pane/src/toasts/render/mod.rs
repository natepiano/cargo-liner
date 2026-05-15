mod card;
mod format;
mod layout;

use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;

use self::layout::StackLayout;
use self::layout::allocate_toast_heights;
use self::layout::render_bottom_up;
use self::layout::render_top_down;
use super::ToastHitbox;
use super::ToastId;
use super::ToastView;
use crate::ToastPlacement;
use crate::ToastSettings;

const ACCENT_COLOR: Color = Color::Cyan;
const ACTIVE_BORDER_COLOR: Color = Color::Yellow;
const ERROR_COLOR: Color = Color::Red;
const LABEL_COLOR: Color = Color::Rgb(150, 190, 180);
const TITLE_COLOR: Color = Color::Yellow;
const WARNING_COLOR: Color = Color::Yellow;

/// Result of rendering toast cards.
pub struct ToastRenderResult {
    /// Hitboxes for the toast card and close-button regions rendered in this pass.
    pub hitboxes: Vec<ToastHitbox>,
}

/// Render toast cards and return their hit-test regions.
pub fn render_toasts(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView],
    settings: &ToastSettings,
    pane_focused: bool,
    focused_toast_id: Option<ToastId>,
) -> ToastRenderResult {
    if !settings.enabled || toasts.is_empty() {
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
    let allocated = allocate_toast_heights(visible_toasts, available);
    let width = settings.width.get().min(area.width);

    let layout = StackLayout {
        width,
        gap,
        pane_focused,
        focused_toast_id,
    };
    let hitboxes = match settings.placement {
        ToastPlacement::TopRight => {
            render_top_down(frame, area, visible_toasts, &allocated, layout)
        },
        ToastPlacement::BottomRight => {
            render_bottom_up(frame, area, visible_toasts, &allocated, layout)
        },
    };

    ToastRenderResult { hitboxes }
}

/// Render-time context for [`super::Toasts`]'s
/// [`crate::Renderable`] impl.
///
/// Built directly by the embedding immediately before rendering.
/// `ToastSettings` lives on `Toasts` itself, so the render impl
/// reads it from `self` — the embedding only supplies wall-clock
/// time and the focused-pane bit.
pub struct ToastsRenderCtx {
    /// Wall-clock timestamp passed to `Toasts::active_views` for
    /// the prune-and-collect pass.
    pub now:          Instant,
    /// Whether the embedding's "toasts pane" focus slot is the
    /// current focus. Drives focused-toast border styling.
    pub pane_focused: bool,
}

impl<Ctx: crate::AppContext> crate::Renderable<ToastsRenderCtx> for super::Toasts<Ctx> {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &ToastsRenderCtx) {
        let focused_id = self.focused_toast_id();
        let active = self.active_views(ctx.now);
        let result = render_toasts(
            frame,
            area,
            &active,
            self.settings(),
            ctx.pane_focused,
            focused_id,
        );
        self.set_hits(result.hitboxes);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use toml::Table;

    use super::card::body_lines_tracked;
    use super::card::tracked_item_line;
    use super::*;
    use crate::ACTIVITY_SPINNER;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::NoToastAction;
    use crate::Toasts;
    use crate::TrackedItem;
    use crate::TrackedItemView;

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
            .unwrap_or_else(|_| std::process::abort());
        let settings = ToastSettings::from_table(&table).unwrap_or_else(|_| std::process::abort());
        assert_eq!(settings.gap.get(), 0);
        let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
        let _ = toasts.push("one", "body");
        let _ = toasts.push("two", "body");
        let views = toasts.active_views(Instant::now());
        let _app = TestApp {
            framework: Framework::new(FocusedPane::App(())),
        };
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        let mut result = None;

        terminal
            .draw(|frame| {
                result = Some(render_toasts(
                    frame,
                    frame.area(),
                    &views,
                    &settings,
                    false,
                    None,
                ));
            })
            .unwrap_or_else(|_| std::process::abort());

        let hitboxes = result.unwrap_or_else(|| std::process::abort()).hitboxes;
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

        let lines = body_lines_tracked(&tracked, Style::default(), 2, 20);

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "one");
        assert_eq!(line_text(&lines[1]), "(+3 more)");
    }

    #[test]
    fn tracked_task_toast_height_fits_visible_items_without_blank_bottom_row() {
        let settings = ToastSettings::default();
        let mut toasts = Toasts::<TestApp>::with_settings(settings.clone());
        let task_id = toasts.start_task("Lints", "");
        let items = [
            TrackedItem::new("~/rust/cargo-port-api-fix", "cargo-port-api-fix"),
            TrackedItem::new("~/rust/bevy_hana", "bevy_hana"),
        ];
        assert!(toasts.set_tracked_items(task_id, &items));

        let views = toasts.active_views(Instant::now() + Duration::from_secs(5));
        let backend = TestBackend::new(90, 20);
        let mut terminal = Terminal::new(backend).unwrap_or_else(|_| std::process::abort());
        let mut result = None;

        terminal
            .draw(|frame| {
                result = Some(render_toasts(
                    frame,
                    frame.area(),
                    &views,
                    &settings,
                    false,
                    None,
                ));
            })
            .unwrap_or_else(|_| std::process::abort());

        let hitboxes = result.unwrap_or_else(|| std::process::abort()).hitboxes;
        assert_eq!(hitboxes.len(), 1);
        assert_eq!(
            hitboxes[0].card_rect.height, 4,
            "two tracked rows should render as top border + two rows + bottom border"
        );
    }

    #[test]
    fn tracked_item_running_line_uses_framework_activity_spinner() {
        let elapsed = Duration::from_millis(100);
        let item = TrackedItemView {
            label:           "repo".to_string(),
            linger_progress: None,
            elapsed:         Some(elapsed),
        };

        let line = tracked_item_line(&item, Style::default(), 40);

        assert!(line_text(&line).contains(ACTIVITY_SPINNER.frame_at(elapsed)));
    }
}
