//! Full status-line renderer owned by the framework.
//!
//! Binaries provide facts and policy (`uptime_secs`, scan indicator, and
//! which global actions belong in the strip). The framework resolves
//! keys, applies enabled / disabled styling, fills the line, and lays
//! out nav, pane-action, and global regions.

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
    pub action:         StatusLineGlobalAction<A>,
    /// Whether the slot renders enabled or disabled.
    pub shortcut_state: ShortcutState,
    /// Whether the slot renders at all.
    pub visibility:     Visibility,
}

impl<A: Action> StatusLineGlobal<A> {
    /// Enabled framework-global slot.
    #[must_use]
    pub const fn framework(action: GlobalAction) -> Self {
        Self {
            action:         StatusLineGlobalAction::Framework(action),
            shortcut_state: ShortcutState::Enabled,
            visibility:     Visibility::Visible,
        }
    }

    /// Enabled framework-global slot for the built-in shortcut help
    /// overlay.
    #[must_use]
    pub const fn global_shortcuts_help() -> Self {
        Self::framework(GlobalAction::OpenGlobalShortcuts)
    }

    /// Enabled app-global slot.
    #[must_use]
    pub const fn app(action: A) -> Self {
        Self {
            action:         StatusLineGlobalAction::App(action),
            shortcut_state: ShortcutState::Enabled,
            visibility:     Visibility::Visible,
        }
    }

    /// Copy of this slot with a different enabled / disabled state.
    #[must_use]
    pub const fn with_state(mut self, shortcut_state: ShortcutState) -> Self {
        self.shortcut_state = shortcut_state;
        self
    }

    /// Copy of this slot with a different visibility.
    #[must_use]
    pub const fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }
}

/// Whether the status line shows its framework-owned scan indicator.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ScanIndicator {
    /// Show the scanning activity segment.
    Shown,
    /// Hide the scanning activity segment.
    Hidden,
}

/// Dynamic status-line data supplied by the embedding app.
pub struct StatusLine<'a, A: Action> {
    /// Seconds to show in the framework-owned uptime segment.
    pub uptime_secs:    u64,
    /// Framework-owned scanning indicator state.
    pub scan_indicator: ScanIndicator,
    /// Ordered global slots for the right side of the status line.
    pub globals:        &'a [StatusLineGlobal<A>],
}

impl<'a, A: Action> StatusLine<'a, A> {
    /// Construct dynamic status-line data.
    #[must_use]
    pub const fn new(
        uptime_secs: u64,
        scan_indicator: ScanIndicator,
        globals: &'a [StatusLineGlobal<A>],
    ) -> Self {
        Self {
            uptime_secs,
            scan_indicator,
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
    if matches!(status.scan_indicator, ScanIndicator::Shown) {
        left_spans.push(Span::styled(" ⟳ scanning… ", palette.status_activity_style));
    }
    left_spans.push(Span::styled(" Uptime: ", palette.status_label_style));
    left_spans.push(Span::styled(
        format!("{} ", crate::format_progressive(status.uptime_secs)),
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
                let Some(key) = keymap.framework_globals().key_for(action).cloned() else {
                    continue;
                };
                RenderedSlot {
                    region: BarRegion::Global,
                    label: action.bar_label(),
                    key,
                    shortcut_state: global.shortcut_state,
                    visibility: global.visibility,
                    secondary_key: None,
                }
            },
            StatusLineGlobalAction::App(action) => {
                let Some(scope) = keymap.globals::<G>() else {
                    continue;
                };
                let Some(key) = scope.key_for(action).cloned() else {
                    continue;
                };
                RenderedSlot {
                    region: BarRegion::Global,
                    label: action.bar_label(),
                    key,
                    shortcut_state: global.shortcut_state,
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
        // Right boundary the center text may not cross. Without a right
        // region it's the full bar width; with one, it's the column the
        // right region starts at — so the right region's keys never
        // paint over center labels.
        let right_boundary = if right_spans.is_empty() {
            total_width
        } else {
            total_width.saturating_sub(right_width + 1)
        };
        // Natural centered start, then shift left as far as needed to
        // fit before `right_boundary`. If even a left-flush start
        // overflows, clip the rightmost text — start sits one cell past
        // nav so labels never overlap nav keys.
        let centered = total_width.saturating_sub(center_width) / 2;
        let shifted = centered.min(right_boundary.saturating_sub(center_width));
        let center_start = shifted.max(left_width);
        if center_start < right_boundary {
            let available = right_boundary - center_start;
            let center_area = Rect {
                x:      area.x + u16::try_from(center_start).unwrap_or(u16::MAX),
                y:      area.y,
                width:  u16::try_from(available.min(center_width + 1)).unwrap_or(u16::MAX),
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
