//! Full status-line renderer owned by the framework.
//!
//! Binaries provide facts and policy (`uptime_secs`, scan indicator, and
//! which global actions belong in the strip). The framework resolves
//! keys, applies enabled / disabled styling, fills the line, and lays
//! out nav, pane-action, and global regions.

#[cfg(test)]
#[path = "status_bar_contrast_tests.rs"]
mod status_bar_contrast_tests;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::BarPalette;
use super::constants::CPU_LABEL;
use super::constants::DISK_READ_LABEL;
use super::constants::STATUS_NOTE_MIN_GAP;
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
    /// A fixed modal key uses the same slot styling as mapped actions.
    Fixed {
        key:   &'static str,
        label: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum GlobalSlotFit {
    Required,
    AfterUptime,
}

/// A toggle records its value independently of shortcut availability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HintToggleState {
    /// The action is off.
    Off,
    /// The action is on.
    On,
}

/// Ordinary actions do not have an on or off value.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum HintValue {
    Action,
    Toggle(HintToggleState),
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
    fit:                GlobalSlotFit,
    hint_value:         HintValue,
}

impl<A: Action> StatusLineGlobal<A> {
    /// Enabled framework-global slot.
    #[must_use]
    pub const fn framework(action: GlobalAction) -> Self {
        Self {
            action:         StatusLineGlobalAction::Framework(action),
            shortcut_state: ShortcutState::Enabled,
            visibility:     Visibility::Visible,
            fit:            GlobalSlotFit::Required,
            hint_value:     HintValue::Action,
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
            fit:            GlobalSlotFit::Required,
            hint_value:     HintValue::Action,
        }
    }

    /// A fixed modal key rendered through the ordinary hint path.
    #[must_use]
    pub const fn fixed(key: &'static str, label: &'static str) -> Self {
        Self {
            action:         StatusLineGlobalAction::Fixed { key, label },
            shortcut_state: ShortcutState::Enabled,
            visibility:     Visibility::Visible,
            fit:            GlobalSlotFit::Required,
            hint_value:     HintValue::Action,
        }
    }

    /// Record a toggle value without changing shortcut availability.
    #[must_use]
    pub const fn with_toggle(mut self, state: HintToggleState) -> Self {
        self.hint_value = HintValue::Toggle(state);
        self
    }

    /// Read the semantic hint value for shortcut presentation.
    pub(crate) const fn hint_value(self) -> HintValue { self.hint_value }

    /// Name the action in its saved keymap scope.
    #[must_use]
    pub fn action_key(self) -> &'static str {
        match self.action {
            StatusLineGlobalAction::App(action) => action.toml_key(),
            StatusLineGlobalAction::Framework(action) => action.toml_key(),
            StatusLineGlobalAction::Fixed { key, .. } => key,
        }
    }

    /// Read the action's plain display label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self.action {
            StatusLineGlobalAction::App(action) => action.bar_label(),
            StatusLineGlobalAction::Framework(action) => action.bar_label(),
            StatusLineGlobalAction::Fixed { label, .. } => label,
        }
    }

    /// Ordinary actions have no toggle label.
    #[must_use]
    pub const fn toggle_label(self) -> &'static str {
        match self.hint_value {
            HintValue::Action => "",
            HintValue::Toggle(HintToggleState::Off) => "off",
            HintValue::Toggle(HintToggleState::On) => "on",
        }
    }

    /// Show this slot only when uptime and the other status items also fit.
    #[must_use]
    pub const fn after_uptime(mut self) -> Self {
        self.fit = GlobalSlotFit::AfterUptime;
        self
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum HintFit {
    #[default]
    WholeGroups,
    IndividualHints,
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
    hint_fit:           HintFit,
    hint_notes:         &'a [StatusLineNote],
}

struct StatusLineItems {
    uptime:     Vec<Span<'static>>,
    disk_reads: Vec<Span<'static>>,
    navigation: Vec<Span<'static>>,
    center:     Vec<Span<'static>>,
    notes:      Vec<Vec<Span<'static>>>,
    globals:    Vec<Span<'static>>,
    with_hints: Vec<Span<'static>>,
    hint_fit:   HintFit,
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
            hint_fit: HintFit::WholeGroups,
            hint_notes: &[],
        }
    }

    /// Fit whole hints individually when this row has notes.
    ///
    /// Remove hints from the right and keep three blank cells before notes.
    /// Rows without notes keep complete hint groups.
    #[must_use]
    pub const fn with_individual_note_hints(mut self) -> Self {
        self.hint_fit = HintFit::IndividualHints;
        self
    }

    /// Append mode notes with the enabled shortcut label style.
    #[must_use]
    pub const fn with_hint_notes(mut self, notes: &'a [StatusLineNote]) -> Self {
        self.hint_notes = notes;
        self
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

    uptime_spans.push(Span::styled(CPU_LABEL, palette.status_label_style));
    uptime_spans.push(Span::styled(
        format!("{} ", framework.status_cpu_reading().label()),
        palette.status_value_style,
    ));

    let disk_read_spans = vec![
        Span::styled(DISK_READ_LABEL, palette.status_label_style),
        Span::styled(
            format!("{} ", framework.status_disk_read_rate().label()),
            palette.status_value_style,
        ),
    ];

    let navigation_spans = bar.nav;
    let center_spans = bar.pane_action;
    let note_items = status
        .notes
        .iter()
        .map(|note| status_line_note_spans(std::slice::from_ref(note), palette))
        .chain(status.hint_notes.iter().map(|note| {
            note_spans(
                note,
                palette.enabled_label_style,
                palette.enabled_label_style,
            )
            .to_vec()
        }))
        .collect();
    let required_globals = status
        .globals
        .iter()
        .copied()
        .filter(|global| global.fit == GlobalSlotFit::Required)
        .collect::<Vec<_>>();
    let global_spans = if bar.global.is_empty() {
        Vec::new()
    } else {
        status_line_global_spans::<Ctx, G>(keymap, &required_globals, palette)
    };
    let with_hints = if bar.global.is_empty() || required_globals.len() == status.globals.len() {
        Vec::new()
    } else {
        status_line_global_spans::<Ctx, G>(keymap, status.globals, palette)
    };

    let items = StatusLineItems {
        uptime: uptime_spans,
        disk_reads: disk_read_spans,
        navigation: navigation_spans,
        center: center_spans,
        notes: note_items,
        globals: global_spans,
        with_hints,
        hint_fit: status.hint_fit,
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
        .flat_map(|note| note_spans(note, palette.status_label_style, palette.status_value_style))
        .collect()
}

/// Format a note with its label style and value style.
fn note_spans(note: &StatusLineNote, label: Style, value: Style) -> [Span<'static>; 2] {
    [
        Span::styled(note.label.clone(), label),
        Span::styled(format!("{} ", note.value), value),
    ]
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
    let mut hint_ends = Vec::new();
    for global in globals
        .iter()
        .filter(|global| matches!(global.visibility, Visibility::Visible))
    {
        let slot = match global.action {
            StatusLineGlobalAction::Fixed { key, label } => {
                support::push_hint(&mut spans, key, label, global.shortcut_state, palette);
                if global.fit == GlobalSlotFit::AfterUptime {
                    hint_ends.push(spans.len());
                }
                continue;
            },
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
        if global.fit == GlobalSlotFit::AfterUptime {
            hint_ends.push(spans.len());
        }
    }
    for end in hint_ends {
        if let Some(separator) = spans.get_mut(end) {
            // Keep two cells between hints, including the next key's leading cell.
            separator.content = " ".into();
        }
    }
    spans
}

fn render_sections(
    frame: &mut Frame,
    area: Rect,
    palette: &BarPalette,
    mut items: StatusLineItems,
) {
    let hint_fit = if items.notes.is_empty() {
        HintFit::WholeGroups
    } else {
        items.hint_fit
    };
    let total_width = area.width as usize;
    let right_spans = fitted_right_spans(total_width, &mut items);
    let right_width = right_spans.iter().map(Span::width).sum::<usize>();
    let right_start = if right_spans.is_empty() {
        total_width
    } else {
        total_width.saturating_sub(right_width + 1)
    };
    let right_gap = if hint_fit == HintFit::IndividualHints {
        let leading_blank = right_spans
            .iter()
            .flat_map(|span| span.content.chars())
            .take_while(|character| character.is_whitespace())
            .count();
        STATUS_NOTE_MIN_GAP.saturating_sub(leading_blank)
    } else {
        1
    };
    let left_room = if right_spans.is_empty() {
        total_width
    } else {
        right_start.saturating_sub(right_gap)
    };
    let left_spans = fitted_left_spans(left_room, &mut items, hint_fit);
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
        // Reserve the note gap when the caller selects individual hints.
        let right_boundary = if hint_fit == HintFit::IndividualHints && !right_spans.is_empty() {
            right_start.saturating_sub(right_gap)
        } else {
            right_start
        };
        // Other rows keep the complete hint group.
        if hint_fit == HintFit::IndividualHints {
            fit_hint_prefix(&mut items.center, right_boundary.saturating_sub(left_width));
        }
        let center_width = items.center.iter().map(Span::width).sum::<usize>();
        let centered = total_width.saturating_sub(center_width) / 2;
        let shifted = centered.min(right_boundary.saturating_sub(center_width));
        let center_start = shifted.max(left_width);
        if center_width > 0 && center_start.saturating_add(center_width) <= right_boundary {
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
fn fitted_left_spans(
    room: usize,
    items: &mut StatusLineItems,
    hint_fit: HintFit,
) -> Vec<Span<'static>> {
    let uptime_width = items.uptime.iter().map(Span::width).sum::<usize>();
    if uptime_width > room {
        return Vec::new();
    }
    let navigation_width = items.navigation.iter().map(Span::width).sum::<usize>();
    let disk_read_width = items.disk_reads.iter().map(Span::width).sum::<usize>();
    if uptime_width + navigation_width + disk_read_width <= room {
        items.uptime.append(&mut items.disk_reads);
    }
    let uptime_width = items.uptime.iter().map(Span::width).sum::<usize>();
    let navigation_room = room.saturating_sub(uptime_width);
    if hint_fit == HintFit::IndividualHints {
        fit_hint_prefix(&mut items.navigation, navigation_room);
    } else if items.navigation.iter().map(Span::width).sum::<usize>() > navigation_room {
        return std::mem::take(&mut items.uptime);
    }
    std::mem::take(&mut items.uptime)
        .into_iter()
        .chain(std::mem::take(&mut items.navigation))
        .collect()
}

/// Remove whole hints from the right until their spans fit.
fn fit_hint_prefix(spans: &mut Vec<Span<'static>>, room: usize) {
    while spans.iter().map(Span::width).sum::<usize>() > room {
        if let Some(separator) = spans
            .iter()
            .rposition(|span| span.content == super::constants::SEPARATOR)
        {
            spans.truncate(separator);
        } else {
            spans.clear();
        }
    }
}

/// Right-side items that fit with the trailing status-line cell retained.
fn fitted_right_spans(total_width: usize, items: &mut StatusLineItems) -> Vec<Span<'static>> {
    let hinted_width = items
        .uptime
        .iter()
        .chain(items.notes.iter().flatten())
        .chain(&items.with_hints)
        .map(Span::width)
        .sum::<usize>();
    if !items.with_hints.is_empty() && hinted_width.saturating_add(2) <= total_width {
        items.globals = std::mem::take(&mut items.with_hints);
    }
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
    use crate::bar::disk_reads::DiskReadRate;

    const FULL_ROW_EXTRA: usize = 3;
    const TEST_HEIGHT: u16 = 1;
    const DISK_READ_TEXT: &str = "Reads/s: 70 MB ";

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
                disk_reads: if uptime.is_empty() {
                    Vec::new()
                } else {
                    vec![Span::raw(DISK_READ_TEXT)]
                },
                uptime,
                navigation,
                center,
                notes,
                globals,
                with_hints: Vec::new(),
                hint_fit: HintFit::WholeGroups,
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

    fn check_disk_read_priority_at_every_width() {
        let palette = BarPalette::default();
        let uptime = vec![
            Span::styled(" Uptime: 10s ", palette.status_label_style),
            Span::styled(CPU_LABEL, palette.status_label_style),
            Span::styled("— ", palette.status_value_style),
        ];
        let disk_label = DiskReadRate::Measured(70_000_000.0).label();
        let disk_reads = vec![
            Span::styled(DISK_READ_LABEL, palette.status_label_style),
            Span::styled(format!("{disk_label} "), palette.status_value_style),
        ];
        let mut navigation = Vec::new();
        for (key, label) in [("↑/↓", "nav"), ("=/-", "all"), ("tab", "pane")] {
            support::push_hint(
                &mut navigation,
                key,
                label,
                ShortcutState::Enabled,
                &palette,
            );
        }
        for hint_fit in [HintFit::WholeGroups, HintFit::IndividualHints] {
            let mut disk_dropped = false;
            for width in (0..=100).rev() {
                let mut items = StatusLineItems {
                    uptime: uptime.clone(),
                    disk_reads: disk_reads.clone(),
                    navigation: navigation.clone(),
                    center: Vec::new(),
                    notes: Vec::new(),
                    globals: Vec::new(),
                    with_hints: Vec::new(),
                    hint_fit,
                };
                let row = text(&fitted_left_spans(width, &mut items, hint_fit));
                if row.contains(DISK_READ_LABEL) {
                    assert!(!disk_dropped, "{hint_fit:?}, width {width}: {row:?}");
                    assert!(row.ends_with(&text(&navigation)), "{row:?}");
                } else {
                    disk_dropped = true;
                }
                for (room, expected_navigation) in [
                    (48, " ↑/↓ nav   =/- all"),
                    (49, " ↑/↓ nav   =/- all   tab pane"),
                ] {
                    if width == room && hint_fit == HintFit::IndividualHints {
                        assert_eq!(row, format!("{}{}", text(&uptime), expected_navigation));
                    }
                }
            }
        }
    }

    #[test]
    fn every_status_line_item_is_whole_or_absent_at_every_width() {
        check_disk_read_priority_at_every_width();
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
            (DISK_READ_TEXT.to_string(), 'R'),
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
            .saturating_add(DISK_READ_TEXT.len())
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

#[cfg(test)]
#[path = "status_line_gap_tests.rs"]
mod gap_tests;
