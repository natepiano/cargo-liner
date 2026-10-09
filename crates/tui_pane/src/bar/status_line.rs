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

/// One informational label/value segment on the right of the status line.
///
/// Notes render before the global shortcut slots and carry no key binding,
/// so unlike those slots they stay visible while the focused pane is in
/// [`Mode::TextInput`](crate::Mode::TextInput). The framework styles them
/// with the same label/value pair as the left-side uptime segment; the app
/// supplies only the text.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StatusLineNote {
    /// Leading label, including any padding spaces — the framework adds none.
    pub label: String,
    /// Value following the label.
    pub value: String,
}

impl StatusLineNote {
    /// A note that is its label alone, with an empty value: a flag
    /// saying some state is in force, such as a paused display.
    #[must_use]
    pub fn flag(label: &str) -> Self {
        Self {
            label: label.to_string(),
            value: String::new(),
        }
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
    /// Informational segments for the right side, rendered before
    /// `globals`.
    pub notes:          &'a [StatusLineNote],
    /// Ordered global slots for the right side of the status line.
    pub globals:        &'a [StatusLineGlobal<A>],
}

struct StatusLineItems {
    uptime:     Vec<Span<'static>>,
    navigation: Vec<Span<'static>>,
    center:     Vec<Span<'static>>,
    notes:      Vec<Vec<Span<'static>>>,
    globals:    Vec<Span<'static>>,
}

impl<'a, A: Action> StatusLine<'a, A> {
    /// Construct dynamic status-line data.
    #[must_use]
    pub const fn new(
        uptime_secs: u64,
        scan_indicator: ScanIndicator,
        notes: &'a [StatusLineNote],
        globals: &'a [StatusLineGlobal<A>],
    ) -> Self {
        Self {
            uptime_secs,
            scan_indicator,
            notes,
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

    let mut uptime_spans = Vec::new();
    if matches!(status.scan_indicator, ScanIndicator::Shown) {
        uptime_spans.push(Span::styled(" ⟳ scanning… ", palette.status_activity_style));
    }
    uptime_spans.push(Span::styled(" Uptime: ", palette.status_label_style));
    uptime_spans.push(Span::styled(
        format!("{} ", crate::format_progressive(status.uptime_secs)),
        palette.status_value_style,
    ));

    let navigation_spans = bar.nav;
    let center_spans = bar.pane_action;
    let note_items = status
        .notes
        .iter()
        .map(|note| status_line_note_spans(std::slice::from_ref(note), palette))
        .collect();
    let global_spans = if bar.global.is_empty() {
        Vec::new()
    } else {
        status_line_global_spans::<Ctx, G>(keymap, status.globals, palette)
    };

    let items = StatusLineItems {
        uptime:     uptime_spans,
        navigation: navigation_spans,
        center:     center_spans,
        notes:      note_items,
        globals:    global_spans,
    };
    render_sections(frame, area, palette, items);
}

/// Style informational status-line notes with the label/value pair the
/// uptime segment uses.
#[must_use]
pub fn status_line_note_spans(
    notes: &[StatusLineNote],
    palette: &BarPalette,
) -> Vec<Span<'static>> {
    notes
        .iter()
        .flat_map(|note| {
            [
                Span::styled(note.label.clone(), palette.status_label_style),
                Span::styled(format!("{} ", note.value), palette.status_value_style),
            ]
        })
        .collect()
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
    mut items: StatusLineItems,
) {
    let total_width = area.width as usize;
    let center_width = items.center.iter().map(Span::width).sum::<usize>();
    let right_spans = fitted_right_spans(total_width, &mut items);
    let right_width = right_spans.iter().map(Span::width).sum::<usize>();
    let right_start = if right_spans.is_empty() {
        total_width
    } else {
        total_width.saturating_sub(right_width + 1)
    };
    let left_room = if right_spans.is_empty() {
        total_width
    } else {
        right_start.saturating_sub(1)
    };
    let left_spans = fitted_left_spans(left_room, &mut items);
    let left_width = left_spans.iter().map(Span::width).sum::<usize>();

    if !left_spans.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(left_spans)).style(palette.status_line_style),
            Rect {
                width: u16::try_from(left_width).unwrap_or(u16::MAX),
                ..area
            },
        );
    }

    if !items.center.is_empty() {
        // Right boundary the center text may not cross. Without a right
        // region it's the full bar width; with one, it's the column the
        // right region starts at — so the right region's keys never
        // paint over center labels.
        let right_boundary = right_start;
        // Start at the natural centre, shifted left when the right side
        // needs the room. The left item sets the earliest possible start;
        // without a whole interval between both sides, the centre stays off.
        let centered = total_width.saturating_sub(center_width) / 2;
        let shifted = centered.min(right_boundary.saturating_sub(center_width));
        let center_start = shifted.max(left_width);
        if center_start.saturating_add(center_width) <= right_boundary {
            let center_area = Rect {
                x:      area.x + u16::try_from(center_start).unwrap_or(u16::MAX),
                y:      area.y,
                width:  u16::try_from(center_width).unwrap_or(u16::MAX),
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(items.center)).style(palette.status_line_style),
                center_area,
            );
        }
    }

    if !right_spans.is_empty() {
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

/// Left-side items that fit before the right-side region.
fn fitted_left_spans(room: usize, items: &mut StatusLineItems) -> Vec<Span<'static>> {
    let uptime_width = items.uptime.iter().map(Span::width).sum::<usize>();
    let navigation_width = items.navigation.iter().map(Span::width).sum::<usize>();
    if uptime_width.saturating_add(navigation_width) <= room {
        return std::mem::take(&mut items.uptime)
            .into_iter()
            .chain(std::mem::take(&mut items.navigation))
            .collect();
    }
    if uptime_width <= room {
        return std::mem::take(&mut items.uptime);
    }
    Vec::new()
}

/// Right-side items that fit with the trailing status-line cell retained.
fn fitted_right_spans(total_width: usize, items: &mut StatusLineItems) -> Vec<Span<'static>> {
    let room = total_width.saturating_sub(1);
    let global_width = items.globals.iter().map(Span::width).sum::<usize>();
    if global_width > room {
        return Vec::new();
    }
    let mut width = global_width;
    let mut fitted = std::mem::take(&mut items.globals);
    for note in std::mem::take(&mut items.notes).into_iter().rev() {
        let note_width = note.iter().map(Span::width).sum::<usize>();
        if width.saturating_add(note_width) > room {
            break;
        }
        width += note_width;
        fitted.splice(0..0, note);
    }
    fitted
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::KeyBind;

    const FULL_ROW_EXTRA: usize = 3;
    const TEST_HEIGHT: u16 = 1;

    fn slot(label: &'static str, key: char, palette: &BarPalette) -> Vec<Span<'static>> {
        let rendered = RenderedSlot {
            region: BarRegion::Global,
            label,
            key: KeyBind::from(key).into(),
            shortcut_state: ShortcutState::Enabled,
            visibility: Visibility::Visible,
            secondary_key: None,
        };
        let mut spans = Vec::new();
        support::push_slot(&mut spans, &rendered, palette);
        spans
    }

    fn text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn drawn(
        width: u16,
        uptime: Vec<Span<'static>>,
        navigation: Vec<Span<'static>>,
        center: Vec<Span<'static>>,
        notes: Vec<Vec<Span<'static>>>,
        globals: Vec<Span<'static>>,
    ) -> String {
        if width == 0 {
            return String::new();
        }
        let backend = TestBackend::new(width, TEST_HEIGHT);
        let mut terminal = match Terminal::new(backend) {
            Ok(terminal) => terminal,
            Err(error) => match error {},
        };
        let result = terminal.draw(|frame| {
            let items = StatusLineItems {
                uptime,
                navigation,
                center,
                notes,
                globals,
            };
            render_sections(
                frame,
                Rect::new(0, 0, width, TEST_HEIGHT),
                &BarPalette::default(),
                items,
            );
        });
        assert!(result.is_ok(), "status line draws in the test backend");
        (0..width)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
            .collect()
    }

    #[test]
    fn every_status_line_item_is_whole_or_absent_at_every_width() {
        let palette = BarPalette::default();
        let uptime = status_line_note_spans(
            &[StatusLineNote {
                label: " UUUU".to_string(),
                value: "1111".to_string(),
            }],
            &palette,
        );
        let navigation = slot("NNNN", 'n', &palette);
        let center = slot("CCCC", 'c', &palette);
        let notes = [
            StatusLineNote {
                label: " AAAA".to_string(),
                value: "2222".to_string(),
            },
            StatusLineNote::flag(" FFFF "),
        ]
        .iter()
        .map(|note| status_line_note_spans(std::slice::from_ref(note), &palette))
        .collect::<Vec<_>>();
        let globals = slot("GGGG", 'g', &palette);
        let items = [
            (text(&uptime), 'U'),
            (text(&navigation), 'N'),
            (text(&center), 'C'),
            (text(&notes[0]), 'A'),
            (text(&notes[1]), 'F'),
            (text(&globals), 'G'),
        ];
        let right_width = notes
            .iter()
            .flatten()
            .chain(&globals)
            .map(Span::width)
            .sum::<usize>();
        let natural = uptime
            .iter()
            .map(Span::width)
            .sum::<usize>()
            .saturating_add(navigation.iter().map(Span::width).sum::<usize>())
            .saturating_add(center.iter().map(Span::width).sum::<usize>())
            .saturating_add(right_width)
            .saturating_add(FULL_ROW_EXTRA);
        for width in 0..=u16::try_from(natural).unwrap_or(u16::MAX) {
            let row = drawn(
                width,
                uptime.clone(),
                navigation.clone(),
                center.clone(),
                notes.clone(),
                globals.clone(),
            );
            for (item, marker) in &items {
                assert!(
                    row.contains(item) || !row.contains(*marker),
                    "width {width} cuts {item:?} in {row:?}"
                );
            }
            if row.contains(&items[0].0) && row.contains(&items[1].0) {
                assert!(
                    row.find(&items[0].0) < row.find(&items[1].0),
                    "width {width} draws navigation before uptime in {row:?}"
                );
            }
        }
    }

    #[test]
    fn the_uptime_outlasts_the_navigation() {
        let palette = BarPalette::default();
        let uptime = status_line_note_spans(
            &[StatusLineNote {
                label: " UUUU".to_string(),
                value: "1111".to_string(),
            }],
            &palette,
        );
        let navigation = slot("NNNN", 'n', &palette);
        let uptime_text = text(&uptime);
        let navigation_text = text(&navigation);
        let uptime_width = uptime.iter().map(Span::width).sum::<usize>();
        let full_width =
            uptime_width.saturating_add(navigation.iter().map(Span::width).sum::<usize>());

        for width in uptime_width..full_width {
            let width = u16::try_from(width).unwrap_or(u16::MAX);
            let row = drawn(
                width,
                uptime.clone(),
                navigation.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            assert!(row.contains(&uptime_text), "width {width} drops the uptime");
            assert!(
                !row.contains(&navigation_text),
                "width {width} keeps the navigation in {row:?}"
            );
        }
    }

    #[test]
    fn globals_outlast_status_line_notes() {
        let palette = BarPalette::default();
        let notes = vec![status_line_note_spans(
            &[StatusLineNote::flag(" NNNN ")],
            &palette,
        )];
        let globals = slot("GGGG", 'g', &palette);
        let note = text(&notes[0]);
        let global = text(&globals);
        let full_width = note.len().saturating_add(global.len()).saturating_add(1);
        for width in 0..=u16::try_from(full_width).unwrap_or(u16::MAX) {
            let row = drawn(
                width,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                notes.clone(),
                globals.clone(),
            );
            if row.contains(&note) {
                assert!(row.contains(&global), "width {width} kept a note first");
            }
        }
    }
}
