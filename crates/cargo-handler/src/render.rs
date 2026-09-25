//! Frame rendering: the tile grid, the framework status line along the
//! bottom, and whichever overlay is open above them.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use tui_pane::AttractWork;
use tui_pane::BarPalette;
use tui_pane::Keymap;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::ScanIndicator;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::StatusLineNote;
use tui_pane::TileCells;
use tui_pane::TileGridContents;
use tui_pane::Updates;
use tui_pane::draw_attract_layers;
use tui_pane::label_color;
use tui_pane::render_status_line;

use crate::app::App;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::ATTRACT_NOTE_LABEL;
use crate::constants::EMPTY_SUMMARY_NOTE;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::globals::AppGlobalAction;
use crate::settings;
use crate::tiles::NoGroup;
use crate::tiles::TileContent;
use crate::tiles::TileDemands;

/// Draw one frame: the grid fills the terminal above the status line,
/// and the toasts, the favorites modal and any framework overlay float
/// above both.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());

    // Nothing runs under this app yet, so the attract screen comes on
    // after its quiet spell the way it does over an idle grid.
    draw_attract_layers(
        frame,
        app,
        body,
        AttractWork::Idle,
        Updates::Live,
        |frame, app, contents| draw_panes(frame, app, body, contents),
    );
    draw_status_line(frame, app, keymap, status);
    tui_pane::render_toasts(frame, &mut app.framework);
    app.favorites_overlay.render(frame);
    tui_pane::draw_framework_overlay(frame, app, keymap, settings::rows);
}

/// Draw the tile grid into the body above the status line.
///
/// [`tui_pane::draw_tile_grid`] decides where each cell goes, how far
/// through a transition it is, and draws the frames, the numbers of the
/// empty cells and every cell's readout; [`Cells`] is what goes inside
/// the summary.
fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect, contents: TileGridContents) {
    let initial_rows = app.loaded_config.config.tiles.initial_rows();
    tui_pane::draw_tile_grid(
        frame.buffer_mut(),
        &mut app.tiles,
        area,
        initial_rows,
        contents,
        &Cells,
    );
}

/// What the tile grid's cells hold. Only the summary has contents, and
/// those say there is nothing to show.
struct Cells;

impl TileCells<NoGroup> for Cells {
    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    /// The note is drawn in whatever room the summary has, so it asks
    /// for none.
    fn demands(&self, _widths: &[(TileContent, u16)]) -> TileDemands { TileDemands::default() }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, _ground: Color) {
        match content {
            TileContent::Summary => draw_empty_summary(buffer, inner),
            // No group claims a cell -- `NoGroup` has no values -- and
            // the grid draws an empty cell's number itself.
            TileContent::Group(_) | TileContent::Empty(_) => {},
        }
    }
}

/// A blank line, then the note that there is nothing to show, indented
/// as a section's first line would be.
fn draw_empty_summary(buffer: &mut Buffer, area: Rect) {
    Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("{SECTION_HEADER_INDENT}{EMPTY_SUMMARY_NOTE}"),
            Style::default().fg(label_color()),
        )),
    ])
    .render(area, buffer);
}

/// The status line: the app's name and version, `attract` while the
/// attract screen was asked for, the uptime, and the `?` shortcut.
fn draw_status_line(frame: &mut Frame, app: &App, keymap: &Keymap<App>, area: Rect) {
    let globals = [StatusLineGlobal::global_shortcuts_help()];
    let mut notes = vec![StatusLineNote {
        label: APP_NAME.to_string(),
        value: APP_VERSION.to_string(),
    }];
    // A grid drawn over looks exactly like an empty one: say that the
    // grid is behind the animation rather than absent.
    if app.attract.asked_for() {
        notes.push(StatusLineNote::flag(ATTRACT_NOTE_LABEL));
    }
    let status = StatusLine::new(
        app.started.elapsed().as_secs(),
        ScanIndicator::Hidden,
        &notes,
        &globals,
    );
    render_status_line::<App, AppGlobalAction>(
        frame,
        area,
        app,
        keymap,
        &app.framework,
        &BarPalette::themed(),
        &status,
    );
}
