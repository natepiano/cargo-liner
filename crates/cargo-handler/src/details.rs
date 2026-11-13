//! `RowDetails` retains the values of one selected row.
//! `DetailsOverlay` selects value lines within four sections.

use std::collections::BTreeMap;

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use tui_pane::Bindings;
use tui_pane::ClipboardBackend;
use tui_pane::ColorMode;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelection;
use tui_pane::CopySelectionResult;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::PaneFocusState;
use tui_pane::PopupFrame;
use tui_pane::ScrollDirection;
use tui_pane::Shortcuts;
use tui_pane::SystemClipboard;
use tui_pane::TabStop;
use tui_pane::Viewport;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::agent_cell;
use crate::app::App;
use crate::app::AppPaneId;
use crate::census;
use crate::census::AgentPlacement;
use crate::constants::ROW_DETAILS_TITLE;
use crate::interaction::RowSpan;
use crate::interaction::SelectableSurface;
use crate::interaction::SurfaceSelection;
use crate::interaction::TileSelection;
use crate::render;

/// Each property owns its section and its position within that section.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DetailProperty {
    Account,
    Model,
    Effort,
    Type,
    Showrunner,
    Unit,
    Worker,
    Name,
    Role,
    Directory,
    Branch,
    Desktop,
    Pid,
    Activity,
    Status,
    Age,
}

impl DetailProperty {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Model => "model",
            Self::Effort => "effort",
            Self::Type => "type",
            Self::Showrunner => "showrunner",
            Self::Unit => "unit",
            Self::Worker => "worker",
            Self::Name => "name",
            Self::Role => "role",
            Self::Directory => "directory",
            Self::Branch => "branch",
            Self::Desktop => "desktop",
            Self::Pid => "pid",
            Self::Activity => "activity",
            Self::Status => "status",
            Self::Age => "age",
        }
    }

    const fn section(self) -> &'static str {
        match self {
            Self::Account | Self::Model | Self::Effort | Self::Type => "agent",
            Self::Showrunner | Self::Unit | Self::Worker | Self::Name | Self::Role => "hierarchy",
            Self::Directory | Self::Branch | Self::Desktop | Self::Pid => "location",
            Self::Activity | Self::Status | Self::Age => "now",
        }
    }
}

/// The details snapshot always includes an effort value.
#[derive(Clone, Debug)]
pub(crate) struct RowDetails {
    pub(crate) properties:  Vec<(DetailProperty, String)>,
    pub(crate) value_lines: BTreeMap<DetailProperty, Line<'static>>,
}

impl RowDetails {
    #[cfg(test)]
    #[expect(clippy::expect_used, reason = "the fixture property must exist")]
    pub(crate) fn value(&self, label: &str) -> String {
        self.properties
            .iter()
            .find(|(property, _)| property.label() == label)
            .map(|(_, value)| value.clone())
            .expect("fixture property")
    }

    fn label_width(&self) -> usize {
        self.properties
            .iter()
            .map(|(property, _)| property.label().len())
            .max()
            .unwrap_or(0)
    }

    fn width(&self) -> usize {
        let values = self
            .properties
            .iter()
            .flat_map(|(_, value)| value.lines())
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0);
        self.label_width() + values + 6
    }

    /// Keep each source span's ink when a value wraps.
    fn value_spans(
        &self,
        property: DetailProperty,
        text: &str,
        offset: usize,
    ) -> Vec<Span<'static>> {
        let Some(line) = self.value_lines.get(&property) else {
            return vec![Span::raw(text.to_owned())];
        };
        let mut spans = Vec::new();
        let mut consumed = 0;
        for span in &line.spans {
            let span_start = consumed;
            consumed += span.content.len();
            let start = offset.max(span_start);
            let end = (offset + text.len()).min(consumed);
            if start < end {
                spans.push(Span::styled(
                    span.content[start - span_start..end - span_start].to_owned(),
                    line.style.patch(span.style),
                ));
            }
        }
        spans
    }

    fn lines(&self, width: u16, selected: usize, color_mode: ColorMode) -> DetailLines {
        let labels = self.label_width();
        let value_start = labels + 3;
        let value_width = usize::from(width).saturating_sub(value_start + 1).max(1);
        let mut lines = Vec::new();
        let mut section = "";
        let mut selected_start = 0;
        let mut value_heights = Vec::new();
        let mut rows = Vec::new();
        for (index, (property, value)) in self.properties.iter().enumerate() {
            if property.section() != section {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                section = property.section();
                lines.push(Line::styled(
                    format!(" {section}"),
                    Style::default()
                        .fg(tui_pane::title_color())
                        .add_modifier(Modifier::BOLD),
                ));
            }
            let start = lines.len();
            let mut value_offset = 0;
            for (part, text) in wrap_value(value, value_width).into_iter().enumerate() {
                let prefix = if part == 0 {
                    format!(
                        " {}:{}",
                        property.label(),
                        " ".repeat(labels - property.label().len() + 1),
                    )
                } else {
                    " ".repeat(value_start)
                };
                let mut spans = vec![Span::raw(prefix)];
                spans.extend(self.value_spans(*property, &text, value_offset));
                spans.push(Span::raw(" "));
                value_offset += text.len();
                if value[value_offset..].starts_with('\n') {
                    value_offset += 1;
                }
                let line = Line::from(spans);
                lines.push(if index == selected {
                    line.style(tui_pane::selection_style(
                        PaneFocusState::Active,
                        color_mode,
                    ))
                } else {
                    line
                });
            }
            if index == selected {
                selected_start = start;
            }
            value_heights.push(lines.len() - start);
            rows.push(RowSpan::single(start).with_continuations(lines.len() - start - 1));
        }
        DetailLines {
            lines,
            selected_start,
            value_heights,
            rows,
        }
    }
}

/// Rendered rows retain each value's height and the selected value's start.
struct DetailLines {
    lines:          Vec<Line<'static>>,
    selected_start: usize,
    value_heights:  Vec<usize>,
    rows:           Vec<RowSpan>,
}

/// Open the selected row without changing the underlying focus.
pub(crate) fn open_selected(app: &mut App) {
    let SelectableSurface::Surface(surface) = render::focused_surface(app, app.selection_area)
    else {
        return;
    };
    let TileSelection::Row(selected) = app
        .selections
        .surface(&surface)
        .map_or(TileSelection::NoRows, SurfaceSelection::selection)
    else {
        return;
    };
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines, app.accounts.table.session_states());
    if let RowDetailsLookup::Found(details) = agent_cell::row_details_with_inks(
        &placement,
        &selected,
        census::unix_now(),
        app.footer_colors.colors(),
    ) {
        app.details_overlay.open(details);
    }
}

/// Wrap values on grapheme boundaries without removing source spaces.
fn wrap_value(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if grapheme == "\n" || (!line.is_empty() && used + cells > width) {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if grapheme != "\n" {
            line.push_str(grapheme);
            used += cells;
        }
    }
    lines.push(line);
    lines
}

pub(crate) enum RowDetailsLookup {
    Unavailable,
    Found(RowDetails),
}

#[derive(Default)]
pub(crate) enum DetailsOverlay {
    #[default]
    Closed,
    Open {
        details:       RowDetails,
        selected:      usize,
        wrapped_row:   usize,
        value_heights: Vec<usize>,
        viewport:      Viewport,
        painted_rows:  Vec<RowSpan>,
    },
}

impl DetailsOverlay {
    fn open(&mut self, details: RowDetails) {
        let value_heights = vec![1; details.properties.len()];
        *self = Self::Open {
            details,
            selected: 0,
            wrapped_row: 0,
            value_heights,
            viewport: Viewport::new(),
            painted_rows: Vec::new(),
        };
    }

    /// Select a value through the last rendered viewport.
    pub(crate) fn click(&mut self, position: Position) {
        let Self::Open {
            selected,
            wrapped_row,
            viewport,
            painted_rows,
            ..
        } = self
        else {
            return;
        };
        let Some(line) = tui_pane::table_row_at(viewport, position) else {
            return;
        };
        if let Some((index, span)) = painted_rows
            .iter()
            .enumerate()
            .find(|(_, span)| span.start() <= line && line < span.end())
        {
            *selected = index;
            *wrapped_row = line - span.start();
        }
    }

    pub(crate) fn scroll(&mut self, direction: ScrollDirection) {
        if let Self::Open {
            details,
            selected,
            wrapped_row,
            value_heights,
            ..
        } = self
        {
            match direction {
                ScrollDirection::Up if *wrapped_row > 0 => *wrapped_row -= 1,
                ScrollDirection::Up if *selected > 0 => {
                    *selected -= 1;
                    *wrapped_row = value_heights[*selected].saturating_sub(1);
                },
                ScrollDirection::Down if *wrapped_row + 1 < value_heights[*selected] => {
                    *wrapped_row += 1;
                },
                ScrollDirection::Down if *selected + 1 < details.properties.len() => {
                    *selected += 1;
                    *wrapped_row = 0;
                },
                ScrollDirection::Up | ScrollDirection::Down => {},
            }
        }
    }

    pub(crate) fn render(&mut self, frame: &mut Frame, color_mode: ColorMode) {
        self.render_for_focus(frame, color_mode, PaneFocusState::Active);
    }

    pub(crate) fn render_for_focus(
        &mut self,
        frame: &mut Frame,
        color_mode: ColorMode,
        focus: PaneFocusState,
    ) {
        let Self::Open {
            details,
            selected,
            wrapped_row,
            value_heights,
            viewport,
            painted_rows,
        } = self
        else {
            return;
        };
        let width = u16::try_from(details.width())
            .unwrap_or(u16::MAX)
            .min(frame.area().width);
        let mut rendered = details.lines(width.saturating_sub(2), *selected, color_mode);
        let selection_style = tui_pane::selection_style(focus, color_mode);
        for line in rendered
            .lines
            .iter_mut()
            .skip(rendered.selected_start)
            .take(rendered.value_heights[*selected])
        {
            line.style = selection_style;
        }
        let popup = PopupFrame {
            color_mode,
            title: Some(ROW_DETAILS_TITLE.to_owned()),
            border_color: tui_pane::title_color(),
            width,
            height: u16::try_from(rendered.lines.len().saturating_add(2)).unwrap_or(u16::MAX),
        }
        .render_with_areas(frame);
        let area = popup.inner;
        *value_heights = rendered.value_heights;
        *wrapped_row = (*wrapped_row).min(value_heights[*selected].saturating_sub(1));
        let cursor =
            u16::try_from(rendered.selected_start.saturating_add(*wrapped_row)).unwrap_or(u16::MAX);
        let offset = u16::try_from(viewport.scroll_offset())
            .unwrap_or(u16::MAX)
            .min(cursor)
            .max(cursor.saturating_add(1).saturating_sub(area.height));
        viewport.set_len(rendered.lines.len());
        // The top border is the header for the shared table hit test.
        viewport.set_content_area(Rect::new(
            area.x,
            popup.outer.y,
            area.width,
            area.height.saturating_add(1),
        ));
        viewport.set_viewport_rows(usize::from(area.height));
        viewport.set_scroll_offset(usize::from(offset));
        *painted_rows = rendered.rows;
        let paragraph = Paragraph::new(rendered.lines).style(
            Style::default()
                .fg(tui_pane::text_default())
                .bg(tui_pane::pane_background(true)),
        );
        frame.render_widget(paragraph.scroll((offset, 0)), area);
        let selected_end = u16::try_from(
            rendered
                .selected_start
                .saturating_add(value_heights[*selected]),
        )
        .unwrap_or(u16::MAX);
        let visible_start = u16::try_from(rendered.selected_start)
            .unwrap_or(u16::MAX)
            .max(offset);
        let visible_end = selected_end.min(offset.saturating_add(area.height));
        let selected_area = Rect::new(
            area.x,
            area.y.saturating_add(visible_start.saturating_sub(offset)),
            area.width,
            visible_end.saturating_sub(visible_start),
        )
        .intersection(area);
        render::apply_row_selection(frame.buffer_mut(), selected_area, selection_style);
    }
}

/// Copy the details target explicitly because Main retains framework focus.
fn copy_selected<B: ClipboardBackend>(app: &mut App, backend: &mut B) {
    let outcome = app
        .framework
        .copy_pane_selection(AppPaneId::Details, app, backend);
    tui_pane::show_copy_outcome(&mut app.framework.toasts, outcome);
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum DetailsAction {
        Close => ("close", "Close row details");
        Up => ("up", "Select previous value");
        Down => ("down", "Select next value");
        Copy => ("copy", "Copy selected value");
    }
}

pub(crate) struct DetailsPane;

impl Pane<App> for DetailsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Details;
    fn mode() -> fn(&App) -> Mode<App> { |_| Mode::Static }
    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl CopySelection<App> for DetailsPane {
    fn copy_selection(app: &App) -> CopySelectionResult {
        match &app.details_overlay {
            DetailsOverlay::Closed => CopySelectionResult::Nothing,
            DetailsOverlay::Open {
                details, selected, ..
            } => CopySelectionResult::Payload(CopyPayload::new(
                details.properties[*selected].1.clone(),
                CopyLabel::Value,
            )),
        }
    }
}

impl Shortcuts<App> for DetailsPane {
    type Actions = DetailsAction;
    const SCOPE_NAME: &'static str = "details";
    const SECTION_NAME: &'static str = "Row details";
    fn defaults() -> Bindings<DetailsAction> {
        tui_pane::bindings! {
            KeyCode::Esc => DetailsAction::Close,
            KeyCode::Up => DetailsAction::Up,
            KeyCode::Down => DetailsAction::Down,
            KeyCode::Char('y') => DetailsAction::Copy,
        }
    }
    fn dispatcher() -> fn(DetailsAction, &mut App) {
        |action, app| action.apply(app, &mut SystemClipboard::new())
    }
}

impl DetailsAction {
    pub(crate) fn apply<B: ClipboardBackend>(self, app: &mut App, backend: &mut B) {
        match self {
            Self::Close => app.details_overlay = DetailsOverlay::Closed,
            Self::Up => app.details_overlay.scroll(ScrollDirection::Up),
            Self::Down => app.details_overlay.scroll(ScrollDirection::Down),
            Self::Copy => copy_selected(app, backend),
        }
    }
}
