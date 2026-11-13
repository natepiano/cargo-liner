//! `RowDetails` retains the values of one selected row.
//! `DetailsOverlay` selects value lines within four sections.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
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
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::agent_cell;
use crate::app::App;
use crate::app::AppPaneId;
use crate::census;
use crate::census::AgentPlacement;
use crate::constants::ROW_DETAILS_TITLE;
use crate::interaction::SelectableSurface;
use crate::interaction::SurfaceSelection;
use crate::interaction::TileSelection;
use crate::render;

/// Each property owns its section and its position within that section.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DetailProperty {
    Account,
    Family,
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
    State,
    Age,
}

impl DetailProperty {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Family => "family",
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
            Self::State => "state",
            Self::Age => "age",
        }
    }

    const fn section(self) -> &'static str {
        match self {
            Self::Account | Self::Family | Self::Model | Self::Effort | Self::Type => "agent",
            Self::Showrunner | Self::Unit | Self::Worker | Self::Name | Self::Role => "hierarchy",
            Self::Directory | Self::Branch | Self::Desktop | Self::Pid => "location",
            Self::Activity | Self::Status | Self::State | Self::Age => "now",
        }
    }
}

/// The details snapshot always includes an effort value.
#[derive(Clone, Debug)]
pub(crate) struct RowDetails {
    pub(crate) properties: Vec<(DetailProperty, String)>,
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

    fn lines(&self, width: u16, selected: usize, color_mode: ColorMode) -> DetailLines {
        let labels = self.label_width();
        let value_start = labels + 3;
        let value_width = usize::from(width).saturating_sub(value_start + 1).max(1);
        let mut lines = Vec::new();
        let mut section = "";
        let mut selected_start = 0;
        let mut value_heights = Vec::new();
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
            for (part, text) in wrap_value(value, value_width).into_iter().enumerate() {
                let text = if part == 0 {
                    format!(
                        " {}:{}{} ",
                        property.label(),
                        " ".repeat(labels - property.label().len() + 1),
                        text
                    )
                } else {
                    format!("{}{text} ", " ".repeat(value_start))
                };
                let line = Line::from(text);
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
        }
        DetailLines {
            lines,
            selected_start,
            value_heights,
        }
    }
}

/// Rendered rows retain each value's height and the selected value's start.
struct DetailLines {
    lines:          Vec<Line<'static>>,
    selected_start: usize,
    value_heights:  Vec<usize>,
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
    if let RowDetailsLookup::Found(details) =
        agent_cell::row_details(&placement, &selected, census::unix_now())
    {
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
        offset:        u16,
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
            offset: 0,
        };
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
        let Self::Open {
            details,
            selected,
            wrapped_row,
            value_heights,
            offset,
        } = self
        else {
            return;
        };
        let width = u16::try_from(details.width())
            .unwrap_or(u16::MAX)
            .min(frame.area().width);
        let rendered = details.lines(width.saturating_sub(2), *selected, color_mode);
        let area = PopupFrame {
            color_mode,
            title: Some(ROW_DETAILS_TITLE.to_owned()),
            border_color: tui_pane::title_color(),
            width,
            height: u16::try_from(rendered.lines.len().saturating_add(2)).unwrap_or(u16::MAX),
        }
        .render(frame);
        *value_heights = rendered.value_heights;
        *wrapped_row = (*wrapped_row).min(value_heights[*selected].saturating_sub(1));
        let cursor =
            u16::try_from(rendered.selected_start.saturating_add(*wrapped_row)).unwrap_or(u16::MAX);
        *offset = (*offset)
            .min(cursor)
            .max(cursor.saturating_add(1).saturating_sub(area.height));
        let paragraph = Paragraph::new(rendered.lines).style(
            Style::default()
                .fg(tui_pane::text_default())
                .bg(tui_pane::pane_background(true)),
        );
        frame.render_widget(paragraph.scroll((*offset, 0)), area);
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
