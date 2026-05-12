//! Full status-line renderer owned by the framework.
//!
//! Binaries provide facts and policy (`uptime_secs`, `scanning`, and
//! which global actions belong in the strip). The framework resolves
//! keys, applies enabled / disabled styling, fills the line, and lays
//! out nav, pane-action, and global regions.

use std::fmt::Write as _;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::BarPalette;
use super::render as render_bar_regions;
use super::support;
use crate::Action;
use crate::AppContext;
use crate::BarRegion;
use crate::GlobalAction;
use crate::Globals;
use crate::Keymap;
use crate::ShortcutState;
use crate::Visibility;
use crate::keymap::RenderedSlot;

/// Which keymap scope a status-line global slot reads from.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StatusLineGlobalAction<A: Action> {
    /// Framework-owned global action.
    Framework(GlobalAction),
    /// App-owned global action from the registered [`Globals`] scope.
    App(A),
}

/// One global slot in the status line.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StatusLineGlobal<A: Action> {
    /// The framework or app action this slot represents.
    pub action:     StatusLineGlobalAction<A>,
    /// Whether the slot renders enabled or disabled.
    pub state:      ShortcutState,
    /// Whether the slot renders at all.
    pub visibility: Visibility,
}

impl<A: Action> StatusLineGlobal<A> {
    /// Enabled framework-global slot.
    #[must_use]
    pub const fn framework(action: GlobalAction) -> Self {
        Self {
            action:     StatusLineGlobalAction::Framework(action),
            state:      ShortcutState::Enabled,
            visibility: Visibility::Visible,
        }
    }

    /// Enabled app-global slot.
    #[must_use]
    pub const fn app(action: A) -> Self {
        Self {
            action:     StatusLineGlobalAction::App(action),
            state:      ShortcutState::Enabled,
            visibility: Visibility::Visible,
        }
    }

    /// Copy of this slot with a different enabled / disabled state.
    #[must_use]
    pub const fn with_state(mut self, state: ShortcutState) -> Self {
        self.state = state;
        self
    }

    /// Copy of this slot with a different visibility.
    #[must_use]
    pub const fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }
}

/// Dynamic status-line data supplied by the embedding app.
pub struct StatusLine<'a, A: Action> {
    /// Seconds to show in the framework-owned uptime segment.
    pub uptime_secs: u64,
    /// Whether to show the framework-owned scanning indicator.
    pub scanning:    bool,
    /// Ordered global slots for the right side of the status line.
    pub globals:     &'a [StatusLineGlobal<A>],
}

impl<'a, A: Action> StatusLine<'a, A> {
    /// Construct dynamic status-line data.
    #[must_use]
    pub const fn new(uptime_secs: u64, scanning: bool, globals: &'a [StatusLineGlobal<A>]) -> Self {
        Self {
            uptime_secs,
            scanning,
            globals,
        }
    }
}

/// Render the full status line into `area`.
///
/// The framework owns the fill, section placement, uptime/scanning
/// text, and shortcut strip composition. The app supplies dynamic
/// facts and global-slot policy through [`StatusLine`].
pub fn render<Ctx, G>(
    frame: &mut Frame,
    area: Rect,
    ctx: &Ctx,
    keymap: &Keymap<Ctx>,
    framework: &crate::Framework<Ctx>,
    palette: &BarPalette,
    status: &StatusLine<'_, G::Actions>,
) where
    Ctx: AppContext + 'static,
    G: Globals<Ctx>,
{
    frame.render_widget(Paragraph::new("").style(palette.status_line_style), area);

    let bar = render_bar_regions(framework.focused(), ctx, keymap, framework, palette);

    let mut left_spans = Vec::new();
    if status.scanning {
        left_spans.push(Span::styled(" ⟳ scanning… ", palette.status_activity_style));
    }
    left_spans.push(Span::styled(" Uptime: ", palette.status_label_style));
    left_spans.push(Span::styled(
        format!("{} ", format_progressive(status.uptime_secs)),
        palette.status_value_style,
    ));
    left_spans.extend(bar.nav);

    let center_spans = bar.pane_action;
    let right_spans = if bar.global.is_empty() {
        Vec::new()
    } else {
        status_line_global_spans::<Ctx, G>(keymap, status.globals, palette)
    };

    render_sections(frame, area, palette, left_spans, center_spans, right_spans);
}

/// Resolve and style status-line global slots.
#[must_use]
pub fn status_line_global_spans<Ctx, G>(
    keymap: &Keymap<Ctx>,
    globals: &[StatusLineGlobal<G::Actions>],
    palette: &BarPalette,
) -> Vec<Span<'static>>
where
    Ctx: AppContext + 'static,
    G: Globals<Ctx>,
{
    let mut spans = Vec::new();
    for global in globals
        .iter()
        .filter(|global| matches!(global.visibility, Visibility::Visible))
    {
        let slot = match global.action {
            StatusLineGlobalAction::Framework(action) => {
                let Some(key) = keymap.framework_globals().key_for(action).copied() else {
                    continue;
                };
                RenderedSlot {
                    region: BarRegion::Global,
                    label: action.bar_label(),
                    key,
                    state: global.state,
                    visibility: global.visibility,
                    secondary_key: None,
                }
            },
            StatusLineGlobalAction::App(action) => {
                let Some(scope) = keymap.globals::<G>() else {
                    continue;
                };
                let Some(key) = scope.key_for(action).copied() else {
                    continue;
                };
                RenderedSlot {
                    region: BarRegion::Global,
                    label: action.bar_label(),
                    key,
                    state: global.state,
                    visibility: global.visibility,
                    secondary_key: None,
                }
            },
        };
        support::push_slot(&mut spans, &slot, palette);
    }
    spans
}

fn render_sections(
    frame: &mut Frame,
    area: Rect,
    palette: &BarPalette,
    left_spans: Vec<Span<'static>>,
    center_spans: Vec<Span<'static>>,
    right_spans: Vec<Span<'static>>,
) {
    let total_width = area.width as usize;
    let left_width = left_spans.iter().map(Span::width).sum::<usize>();
    let center_width = center_spans.iter().map(Span::width).sum::<usize>();
    let right_width = right_spans.iter().map(Span::width).sum::<usize>();

    if !left_spans.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(left_spans)).style(palette.status_line_style),
            area,
        );
    }

    if !center_spans.is_empty() {
        let center_start = total_width.saturating_sub(center_width) / 2;
        if center_start >= left_width {
            let center_area = Rect {
                x:      area.x + u16::try_from(center_start).unwrap_or(u16::MAX),
                y:      area.y,
                width:  u16::try_from((total_width - center_start).min(center_width + 1))
                    .unwrap_or(u16::MAX),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(center_spans)).style(palette.status_line_style),
                center_area,
            );
        }
    }

    if !right_spans.is_empty() {
        let right_start = total_width.saturating_sub(right_width + 1);
        let right_area = Rect {
            x:      area.x + u16::try_from(right_start).unwrap_or(u16::MAX),
            y:      area.y,
            width:  u16::try_from(right_width + 1).unwrap_or(u16::MAX),
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(right_spans)).style(palette.status_line_style),
            right_area,
        );
    }
}

fn format_progressive(secs: u64) -> String {
    const WEEK: u64 = 7 * 24 * 3600;
    const DAY: u64 = 24 * 3600;
    const HOUR: u64 = 3600;
    const MINUTE: u64 = 60;

    let parts: [(u64, &str); 5] = [
        (secs / WEEK, "w"),
        ((secs % WEEK) / DAY, "d"),
        ((secs % DAY) / HOUR, "h"),
        ((secs % HOUR) / MINUTE, "m"),
        (secs % MINUTE, "s"),
    ];

    let first = parts.iter().position(|&(value, _)| value > 0);
    let last = parts.iter().rposition(|&(value, _)| value > 0);
    let Some((first, last)) = first.zip(last) else {
        return "0s".to_string();
    };

    let mut out = String::new();
    for (value, unit) in &parts[first..=last] {
        if !out.is_empty() {
            out.push(' ');
        }
        let _ = write!(out, "{value}{unit}");
    }
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::format_progressive;

    #[test]
    fn uptime_format_progresses_units() {
        assert_eq!(format_progressive(0), "0s");
        assert_eq!(format_progressive(61), "1m 1s");
        assert_eq!(format_progressive(3605), "1h 0m 5s");
    }
}
