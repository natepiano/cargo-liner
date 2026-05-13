use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use unicode_width::UnicodeWidthStr;

use super::ToastHitbox;
use super::ToastId;
use super::ToastStyle;
use super::ToastView;
use super::TrackedItemView;
use crate::ACTIVITY_SPINNER;
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

#[derive(Clone, Copy)]
struct StackLayout {
    width:            u16,
    gap:              u16,
    pane_focused:     bool,
    focused_toast_id: Option<ToastId>,
}

fn render_top_down(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView],
    allocated: &[u16],
    layout: StackLayout,
) -> Vec<ToastHitbox> {
    let mut hitboxes = Vec::with_capacity(toasts.len());
    let mut cursor_y = area.y;
    for (toast, &alloc_height) in toasts.iter().zip(allocated) {
        if alloc_height == 0 {
            continue;
        }
        let card_height = toast.desired_height().min(alloc_height);
        if card_height == 0
            || cursor_y.saturating_add(card_height) > area.y.saturating_add(area.height)
        {
            break;
        }
        let x = area.x + area.width.saturating_sub(layout.width);
        let card = Rect {
            x,
            y: cursor_y,
            width: layout.width,
            height: card_height,
        };
        let close_rect = render_toast(
            frame,
            area,
            card,
            toast,
            layout.pane_focused,
            layout.focused_toast_id,
        );
        hitboxes.push(ToastHitbox {
            id: toast.id(),
            card_rect: card,
            close_rect,
        });
        cursor_y = cursor_y.saturating_add(card_height + layout.gap);
    }
    hitboxes
}

fn render_bottom_up(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView],
    allocated: &[u16],
    layout: StackLayout,
) -> Vec<ToastHitbox> {
    let mut hitboxes = Vec::with_capacity(toasts.len());
    let mut cursor_y = area.y.saturating_add(area.height);
    for (toast, &alloc_height) in toasts.iter().zip(allocated).rev() {
        if alloc_height == 0 {
            continue;
        }
        let card_height = toast.desired_height().min(alloc_height);
        if card_height == 0 {
            continue;
        }
        cursor_y = cursor_y.saturating_sub(card_height);
        if cursor_y < area.y {
            break;
        }
        let x = area.x + area.width.saturating_sub(layout.width);
        let card = Rect {
            x,
            y: cursor_y,
            width: layout.width,
            height: card_height,
        };
        let close_rect = render_toast(
            frame,
            area,
            card,
            toast,
            layout.pane_focused,
            layout.focused_toast_id,
        );
        hitboxes.push(ToastHitbox {
            id: toast.id(),
            card_rect: card,
            close_rect,
        });
        cursor_y = cursor_y.saturating_sub(layout.gap);
    }
    hitboxes.reverse();
    hitboxes
}

fn allocate_toast_heights(toasts: &[ToastView], available: u16) -> Vec<u16> {
    let mut alloc = vec![0u16; toasts.len()];
    let total_min = toasts
        .iter()
        .map(ToastView::min_height)
        .fold(0u16, u16::saturating_add);

    if total_min > available {
        let mut used = 0u16;
        for (idx, toast) in toasts.iter().enumerate().rev() {
            let min_height = toast.min_height();
            if used.saturating_add(min_height) <= available {
                alloc[idx] = min_height;
                used = used.saturating_add(min_height);
            }
        }
        return alloc;
    }

    for (idx, toast) in toasts.iter().enumerate() {
        alloc[idx] = toast.min_height();
    }
    let mut remaining = available.saturating_sub(total_min);
    while remaining > 0 {
        let mut changed = false;
        for (idx, toast) in toasts.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            if alloc[idx] < toast.desired_height() {
                alloc[idx] += 1;
                remaining -= 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    alloc
}

fn render_toast(
    frame: &mut Frame,
    area: Rect,
    card: Rect,
    toast: &ToastView,
    pane_focused: bool,
    focused_toast_id: Option<ToastId>,
) -> Rect {
    let clear_rect = Rect {
        x:      card.x.saturating_sub(1),
        y:      card.y,
        width:  card
            .width
            .saturating_add(2)
            .min(area.x + area.width - card.x.saturating_sub(1)),
        height: card.height,
    };
    frame.render_widget(Clear, clear_rect);

    let focused = pane_focused && focused_toast_id == Some(toast.id());
    let is_error = toast.style() == ToastStyle::Error;
    let is_warning = toast.style() == ToastStyle::Warning;
    let accent_color = if is_error {
        ERROR_COLOR
    } else if is_warning {
        WARNING_COLOR
    } else {
        Color::White
    };
    let border_style = if focused {
        Style::default().fg(ACTIVE_BORDER_COLOR)
    } else {
        Style::default().fg(accent_color)
    };
    let text_style = if is_error || is_warning {
        Style::default().fg(accent_color)
    } else {
        border_style.add_modifier(Modifier::BOLD)
    };
    let body_style = if is_error || is_warning {
        Style::default().fg(accent_color)
    } else {
        Style::default()
    };

    let close_text = "[x]";
    let close_width = u16::try_from(close_text.len()).unwrap_or(u16::MAX);
    let title_max = usize::from(card.width.saturating_sub(close_width + 4));
    let raw_title = if is_warning {
        format!("! {}", toast.title())
    } else if is_error {
        format!("x {}", toast.title())
    } else {
        toast.title().to_owned()
    };
    let title = truncate(&raw_title, title_max);

    let block = Block::default()
        .title(Span::styled(format!(" {title} "), text_style))
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(card);
    frame.render_widget(block, card);

    let close_rect = Rect {
        x:      card.x + card.width.saturating_sub(close_width + 2),
        y:      card.y,
        width:  close_width + 1,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(close_text, text_style))),
        close_rect,
    );

    if let Some(secs) = toast.remaining_secs() {
        let countdown = format!(" Closing in {secs} ");
        let countdown_width = u16::try_from(countdown.len()).unwrap_or(u16::MAX);
        let countdown_rect = Rect {
            x:      card.x + card.width.saturating_sub(countdown_width + 1),
            y:      card.y + card.height.saturating_sub(1),
            width:  countdown_width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                countdown,
                Style::default().fg(LABEL_COLOR),
            ))),
            countdown_rect,
        );
    }

    if inner.height > 0 {
        let alloc_interior = card.height.saturating_sub(2);
        render_toast_body(frame, toast, body_style, inner, alloc_interior);
    }

    close_rect
}

fn render_toast_body(
    frame: &mut Frame,
    toast: &ToastView,
    body_style: Style,
    body_area: Rect,
    alloc_interior: u16,
) {
    let alloc_body = usize::from(alloc_interior);
    let has_action = toast.has_action() && alloc_body >= 2;
    let lines_for_body = if has_action {
        alloc_body.saturating_sub(1)
    } else {
        alloc_body
    };
    let lines = if toast.tracked_items().is_empty() {
        body_lines_plain(toast, body_style, lines_for_body)
    } else {
        body_lines_tracked(
            toast.tracked_items(),
            body_style,
            lines_for_body,
            usize::from(body_area.width),
        )
    };

    if has_action {
        let text_area = Rect {
            height: body_area.height.saturating_sub(1),
            ..body_area
        };
        frame.render_widget(
            Paragraph::new(lines)
                .style(body_style)
                .wrap(Wrap { trim: false }),
            text_area,
        );
        let hint_area = Rect {
            y: body_area.y + body_area.height.saturating_sub(1),
            height: 1,
            ..body_area
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter open",
                Style::default()
                    .fg(LABEL_COLOR)
                    .add_modifier(Modifier::ITALIC),
            ))),
            hint_area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(lines)
                .style(body_style)
                .wrap(Wrap { trim: false }),
            body_area,
        );
    }
}

fn body_lines_plain(
    toast: &ToastView,
    body_style: Style,
    lines_for_body: usize,
) -> Vec<Line<'static>> {
    let body_lines = toast.body().lines().collect::<Vec<_>>();
    let total_body = body_lines.len();
    let needs_truncation = total_body > lines_for_body;
    let (visible_body, overflow_line) = if needs_truncation && lines_for_body >= 1 {
        let show = lines_for_body.saturating_sub(1);
        let remaining = total_body.saturating_sub(show);
        (
            body_lines[..show].join("\n"),
            Some(format!("(+{remaining} more)")),
        )
    } else {
        (toast.body().to_owned(), None)
    };

    let mut result = visible_body
        .lines()
        .map(|line| {
            toast.linger_progress().map_or_else(
                || Line::from(Span::styled(line.to_owned(), body_style)),
                |progress| fade_to_color(line, f64::from(progress)),
            )
        })
        .collect::<Vec<_>>();
    if let Some(overflow) = overflow_line {
        let overflow_style = Style::default()
            .fg(LABEL_COLOR)
            .add_modifier(Modifier::ITALIC);
        result.push(toast.linger_progress().map_or_else(
            || Line::from(Span::styled(overflow.clone(), overflow_style)),
            |progress| fade_to_color(&overflow, f64::from(progress)),
        ));
    }
    result
}

fn body_lines_tracked(
    tracked: &[TrackedItemView],
    body_style: Style,
    lines_for_body: usize,
    line_width: usize,
) -> Vec<Line<'static>> {
    let total_items = tracked.len();
    let needs_truncation = total_items > lines_for_body;
    let (visible_items, overflow_line) = if needs_truncation && lines_for_body >= 1 {
        let show = lines_for_body.saturating_sub(1);
        let remaining = total_items.saturating_sub(show);
        (&tracked[..show], Some(format!("(+{remaining} more)")))
    } else {
        (tracked, None)
    };

    let mut result = visible_items
        .iter()
        .map(|item| tracked_item_line(item, body_style, line_width))
        .collect::<Vec<_>>();
    if let Some(overflow) = overflow_line {
        let overflow_style = Style::default()
            .fg(LABEL_COLOR)
            .add_modifier(Modifier::ITALIC);
        result.push(Line::from(Span::styled(overflow, overflow_style)));
    }
    result
}

fn tracked_item_line(
    item: &TrackedItemView,
    body_style: Style,
    line_width: usize,
) -> Line<'static> {
    const SPINNER_SLOT: usize = 4;

    let label_style = item.linger_progress.map_or(body_style, fade_to_style);
    let Some(elapsed) = item.elapsed else {
        return Line::from(Span::styled(item.label.clone(), label_style));
    };

    let is_running = item.linger_progress.is_none();
    let spinner_text = if is_running {
        format!(" {} ", ACTIVITY_SPINNER.frame_at(elapsed))
    } else {
        " ".repeat(SPINNER_SLOT)
    };
    let duration_text = format_elapsed(elapsed);
    let duration_suffix = format!("{duration_text} ");
    let suffix_width = duration_suffix.len();
    let label_budget = line_width.saturating_sub(suffix_width + SPINNER_SLOT + 1);
    let label = truncate_with_ellipsis(&item.label, label_budget);
    let used = label.width() + spinner_text.width() + suffix_width;
    let padding = line_width.saturating_sub(used);
    let duration_style = item
        .linger_progress
        .map_or_else(|| Style::default().fg(TITLE_COLOR), fade_to_style);
    Line::from(vec![
        Span::styled(label, label_style),
        Span::raw(" ".repeat(padding)),
        Span::styled(
            spinner_text,
            if is_running {
                Style::default().fg(ACCENT_COLOR)
            } else {
                Style::default()
            },
        ),
        Span::styled(duration_suffix, duration_style),
    ])
}

fn fade_to_style(progress: f64) -> Style {
    let p = progress.clamp(0.0, 1.0);
    let curve = p * p * p;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "value is mathematically clamped to [128, 255] before the cast"
    )]
    let value = 127.0f64.mul_add(-curve, 255.0) as u8;
    Style::default().fg(Color::Rgb(value, value, value))
}

fn fade_to_color(text: &str, progress: f64) -> Line<'static> {
    Line::from(Span::styled(text.to_owned(), fade_to_style(progress)))
}

fn truncate(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    if text.len() <= width {
        return text.to_owned();
    }
    if width <= 1 {
        return String::new();
    }
    format!("{}...", &text[..width.saturating_sub(3)])
}

fn format_elapsed(elapsed: Duration) -> String {
    let ms = elapsed.as_millis();
    if ms >= 60_000 {
        let secs = elapsed.as_secs();
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else if ms >= 10_000 {
        format!("{}s", elapsed.as_secs())
    } else if ms >= 1 {
        format!("{ms}ms")
    } else {
        format!("{}us", elapsed.as_micros())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use toml::Table;

    use super::*;
    use crate::AppContext;
    use crate::FocusedPane;
    use crate::Framework;
    use crate::NoToastAction;
    use crate::Toasts;
    use crate::TrackedItem;

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
        let mut toasts = Toasts::<TestApp>::new();
        let _ = toasts.push("one", "body");
        let _ = toasts.push("two", "body");
        let views = toasts.active_views(Instant::now(), &settings);
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
        let mut toasts = Toasts::<TestApp>::new();
        let task_id = toasts.start_task("Lints", "");
        let items = [
            TrackedItem::new("~/rust/cargo-port-api-fix", "cargo-port-api-fix"),
            TrackedItem::new("~/rust/bevy_hana", "bevy_hana"),
        ];
        assert!(toasts.set_tracked_items(task_id, &items, settings.task_linger.get()));

        let views = toasts.active_views(Instant::now() + Duration::from_secs(5), &settings);
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
