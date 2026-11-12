//! `RowDetails` stores the selected row values.
//! `DetailsOverlay::Open` retains the snapshot while the popup is open.
//! `RowDetailsLookup` distinguishes an available row from a missing row.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use tui_pane::Bindings;
use tui_pane::ColorMode;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::PopupFrame;
use tui_pane::ScrollDirection;
use tui_pane::Shortcuts;
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

/// Each property owns its label and its order after the drawn columns.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum DetailProperty {
    Pid,
    Directory,
    Branch,
    Model,
    Effort,
    Account,
    Name,
    Role,
    Activity,
    Status,
    Type,
    Program,
    Desktop,
    State,
    Age,
}

impl DetailProperty {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pid => "pid",
            Self::Directory => "directory",
            Self::Branch => "branch",
            Self::Model => "model",
            Self::Effort => "effort",
            Self::Account => "account",
            Self::Name => "name",
            Self::Role => "role",
            Self::Activity => "activity",
            Self::Status => "status",
            Self::Type => "type",
            Self::Program => "program",
            Self::Desktop => "desktop",
            Self::State => "state",
            Self::Age => "age",
        }
    }
}

/// Display values retain their source order and the row's age.
#[derive(Clone, Debug)]
pub(crate) struct RowDetails {
    pub(crate) properties: Vec<(DetailProperty, String)>,
}

impl RowDetails {
    fn order(&mut self, drawn: &[DetailProperty]) {
        self.properties.sort_by_key(|(property, _)| {
            (
                drawn
                    .iter()
                    .position(|column| column == property)
                    .unwrap_or(drawn.len()),
                *property,
            )
        });
    }

    /// All tests read properties through this lookup.
    #[cfg(test)]
    #[expect(clippy::expect_used, reason = "the fixture property must exist")]
    pub(crate) fn value(&self, label: &str) -> String {
        self.properties
            .iter()
            .find(|(property, _)| property.label() == label)
            .map(|(_, value)| value.clone())
            .expect("fixture property")
    }
}

/// Open the selected row with the order supplied by its painted surface.
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
    if let RowDetailsLookup::Found(mut details) =
        agent_cell::row_details(&placement, &selected, census::unix_now())
    {
        let order = app
            .painted_surfaces
            .iter()
            .rev()
            .find(|painted| painted.surface() == surface)
            .map_or(&[][..], |painted| painted.detail_order.as_slice());
        details.order(order);
        app.details_overlay.open(details);
    }
}

/// Wrap graphemes without removing spaces from a source value.
fn wrap_value(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for grapheme in text.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if grapheme == "\n" || (!line.is_empty() && used + cells > usize::from(width)) {
            lines.push(Line::from(std::mem::take(&mut line)));
            used = 0;
        }
        if grapheme != "\n" {
            line.push_str(grapheme);
            used += cells;
        }
    }
    lines.push(Line::from(line));
    lines
}

/// An absent row cannot open an empty details modal.
pub(crate) enum RowDetailsLookup {
    Unavailable,
    Found(RowDetails),
}

#[derive(Default)]
pub(crate) enum DetailsOverlay {
    #[default]
    Closed,
    Open {
        details:     RowDetails,
        offset:      u16,
        last_offset: u16,
    },
}

impl DetailsOverlay {
    fn open(&mut self, details: RowDetails) {
        *self = Self::Open {
            details,
            offset: 0,
            last_offset: 0,
        };
    }

    pub(crate) fn scroll(&mut self, direction: ScrollDirection) {
        if let Self::Open {
            offset,
            last_offset,
            ..
        } = self
        {
            *offset = match direction {
                ScrollDirection::Up => offset.saturating_sub(1),
                ScrollDirection::Down => offset.saturating_add(1).min(*last_offset),
            };
        }
    }

    pub(crate) fn render(&mut self, frame: &mut Frame, color_mode: ColorMode) {
        let Self::Open {
            details,
            offset,
            last_offset,
        } = self
        else {
            return;
        };
        let lines: Vec<_> = details
            .properties
            .iter()
            .map(|(name, value)| Line::from(format!("{}: {value}", name.label())))
            .collect();
        let width = lines
            .iter()
            .map(Line::width)
            .max()
            .unwrap_or(0)
            .saturating_add(2);
        let content_width = u16::try_from(width)
            .unwrap_or(u16::MAX)
            .min(frame.area().width)
            .saturating_sub(2);
        let lines: Vec<_> = lines
            .into_iter()
            .flat_map(|line| wrap_value(&line.to_string(), content_width))
            .collect();
        let rows = lines.len();
        let paragraph = Paragraph::new(lines).style(
            ratatui::style::Style::default()
                .fg(tui_pane::text_default())
                .bg(tui_pane::pane_background(true)),
        );
        let area = PopupFrame {
            color_mode,
            title: Some(ROW_DETAILS_TITLE.to_owned()),
            border_color: tui_pane::title_color(),
            width: u16::try_from(width).unwrap_or(u16::MAX),
            height: u16::try_from(rows.saturating_add(2)).unwrap_or(u16::MAX),
        }
        .render(frame);
        *last_offset = u16::try_from(rows)
            .unwrap_or(u16::MAX)
            .saturating_sub(area.height);
        *offset = (*offset).min(*last_offset);
        frame.render_widget(paragraph.scroll((*offset, 0)), area);
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum DetailsAction {
        Close => ("close", "Close row details");
        Up => ("up", "Scroll details up");
        Down => ("down", "Scroll details down");
    }
}

pub(crate) struct DetailsPane;

impl Pane<App> for DetailsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Details;
    fn mode() -> fn(&App) -> Mode<App> { |_| Mode::Static }
    fn tab_stop() -> TabStop<App> { TabStop::never() }
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
        }
    }
    fn dispatcher() -> fn(DetailsAction, &mut App) {
        |action, app| match action {
            DetailsAction::Close => app.details_overlay = DetailsOverlay::Closed,
            DetailsAction::Up => app.details_overlay.scroll(ScrollDirection::Up),
            DetailsAction::Down => app.details_overlay.scroll(ScrollDirection::Down),
        }
    }
}
