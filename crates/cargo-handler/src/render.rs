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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Instant;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;
    use tui_pane::FavoritesFileState;
    use tui_pane::dispatch_key;

    use super::*;

    /// Width of every frame the goldens draw.
    const WIDTH: u16 = 80;
    /// Height of every frame the goldens draw.
    const HEIGHT: u16 = 24;
    /// Rows above the status line.
    const BODY_ROWS: usize = 23;

    /// The status line's left end over the grid: the uptime alone.
    const GRID_STATUS: &str = " Uptime: 0s";
    /// The status line's left end while a framework overlay is open:
    /// the uptime, then the overlay's own keys.
    const OVERLAY_STATUS: &str = " Uptime: 0s  ↑/↓ nav   tab pane enter edit";

    /// One favorite, saved in a past year: a timestamp from any year
    /// but the current one is drawn with its year, so the row reads the
    /// same whenever the test runs.
    const FAVORITE: &str = r#"
[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2020-08-26T11:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
"#;

    /// The first frame: the summary alone, its note under a blank line, and
    /// its readout on the last row.
    const FIRST_FRAME: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│                                                                              │",
        "│ nothing to show yet                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 0  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// After `+` `+`: the summary over cells 2 and 3, each numbered.
    const AFTER_TWO_ADDITIONS: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│                                                                              │",
        "│ nothing to show yet                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                   content rows: 0  r/c: 7/78 │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│ 2                                                                            │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                   content rows: 0  r/c: 7/78 │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│ 3                                                                            │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                   content rows: 0  r/c: 5/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// After `+` `+` `-`: the summary over cell 2.
    const AFTER_TWO_ADDITIONS_AND_A_REMOVAL: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│                                                                              │",
        "│ nothing to show yet                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 0  r/c: 11/78 │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│ 2                                                                            │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                   content rows: 0  r/c: 9/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The settings overlay: Appearance, Tiles and Files, the paths under
    /// the test build's stand-in config root.
    const SETTINGS_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│                                                                              │",
        "│ nothing to show yet                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│       ┌ Settings ────────────────────────────────────────────────────┐       │",
        "│       │ Appearance:                                                  │       │",
        "│       │ ▶ mode          < auto >                                     │       │",
        "│       │   light theme   < Default Light >                            │       │",
        "│       │   dark theme    < Default Dark >                             │       │",
        "│       │ Tiles:                                                       │       │",
        "│       │   initial rows  < 4 >                                        │       │",
        "│       │ Files:                                                       │       │",
        "│       │   config        /<config>/cargo-handler/config.toml          │       │",
        "│       │   themes        /<config>/cargo-handler/themes               │       │",
        "│       │   keymap        /<config>/cargo-handler/keymap.toml          │       │",
        "│       └──────────────────────────────────────────────────────────────┘       │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 0  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The keymap overlay's first screen.
    const KEYMAP_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│                                                                              │",
        "│ nothing┌ Keymap ───────────────────────────────────────────────────┐         │",
        "│        │                                                           │         │",
        "│        │ Global Navigation:                                        │         │",
        "│        │ ▸ Next pane                                     tab       │         │",
        "│        │   Previous pane                                 shift-tab │         │",
        "│        │ Global Shortcuts:                                         │         │",
        "│        │   Add a tile                                    +         │         │",
        "│        │   Dismiss overlay / output                      x         │         │",
        "│        │   Focus the tile above                          up        │         │",
        "│        │   Focus the tile below                          down      │         │",
        "│        │   Focus the tile to the left                    left      │         │",
        "│        │   Focus the tile to the right                   right     │         │",
        "│        │   Open attract favorites                        ctrl-o    │         │",
        "│        │   Open keymap viewer                            ctrl-k    │         │",
        "│        │   Open settings                                 s         │         │",
        "│        │   Quit                                          q         │         │",
        "│        │   Randomize the attract screen                  r         │         │",
        "│        │   Remove an empty tile                          -         │         │",
        "│        └─────────────────────────1 of 6 ▼──────────────────────────┘         │",
        "│                                                  content rows: 0  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The `?` overlay's first screen, with no `x` Dismiss row.
    const SHORTCUTS_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│              ┌ Global Shortcuts ─────────────────────────────┐               │",
        "│ nothing to sh│                                               │               │",
        "│              │ Global Navigation:                            │               │",
        "│              │ ▸ Next pane                         tab       │               │",
        "│              │   Previous pane                     shift-tab │               │",
        "│              │ Global Shortcuts:                             │               │",
        "│              │   Add a tile                        +         │               │",
        "│              │   Focus the tile above              up        │               │",
        "│              │   Focus the tile below              down      │               │",
        "│              │   Focus the tile to the left        left      │               │",
        "│              │   Focus the tile to the right       right     │               │",
        "│              │   Open attract favorites            ctrl-o    │               │",
        "│              │   Open keymap viewer                ctrl-k    │               │",
        "│              │   Open settings                     s         │               │",
        "│              │   Quit                              q         │               │",
        "│              │   Randomize the attract screen      r         │               │",
        "│              │   Remove an empty tile              -         │               │",
        "│              │   Restart                           R         │               │",
        "│              │   Save attract parameters           ctrl-s    │               │",
        "│              │   Show a random favorite            m         │               │",
        "│              │   Show global shortcuts             ?         │ 0  r/c: 21/78 │",
        "└──────────────└───────────────────1 of 2 ▼────────────────────┘───────────────┘",
    ];

    /// The favorites overlay on one saved moving-band favorite.
    const FAVORITES_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│                                                                              │",
        "│ nothing to show yet                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│ ┌ Favorites -- 1 saved -- ● matches the current parameters ────────────────┐ │",
        "│ │Attract: Moving Band                                                      │ │",
        "│ │   Saved                 Direction  Width  Speed  Tail  Fraying           │ │",
        "│ │                         ←↑↓→       -/+    </>    [/]   v                 │ │",
        "│ │▸  26 Aug 2020 11:02:44  left       10     32     72    leading           │ │",
        "│ │enter load   x delete   Esc close                                         │ │",
        "│ └──────────────────────────────────────────────────────────────────────────┘ │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 0  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    fn key(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)
    }

    fn ctrl(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
    }

    /// An app whose grid is laid out in the body of an 80×24 frame, as
    /// a drawn frame leaves it, so `+` has room for a cell.
    fn laid_out_app() -> App {
        let mut app = App::new_for_test().expect("test app should build");
        let initial_rows = app.loaded_config.config.tiles.initial_rows();
        app.tiles.set_layout(
            Rect::new(0, 0, WIDTH, HEIGHT - STATUS_LINE_HEIGHT),
            initial_rows,
        );
        app
    }

    /// Draw `frames` frames of `app` at 80×24 and return the last one's
    /// rows. The start time is reset first, so the uptime reads 0s.
    fn drawn_rows(app: &mut App, frames: usize) -> Vec<String> {
        app.started = Instant::now();
        let keymap = Rc::clone(&app.keymap);
        let mut terminal =
            Terminal::new(TestBackend::new(WIDTH, HEIGHT)).expect("the test terminal opens");
        for _ in 0..frames {
            terminal
                .draw(|frame| draw(frame, app, &keymap))
                .expect("the test terminal draws");
        }
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        (area.top()..area.bottom())
            .map(|row| {
                (area.left()..area.right())
                    .filter_map(|column| buffer.cell((column, row)).map(Cell::symbol))
                    .collect()
            })
            .collect()
    }

    /// The status line: `left` at the left edge, and the app's name, its
    /// version, `flag` when there is one and the `?` shortcut at the
    /// right edge.
    fn status_row(left: &str, flag: Option<&str>) -> String {
        let flag = flag.map_or_else(String::new, |flag| format!(" {flag}"));
        let notes = format!("cargo-handler {APP_VERSION}{flag}  ? shortcuts ");
        format!(
            "{left:<width$}{notes}",
            width = usize::from(WIDTH) - notes.chars().count()
        )
    }

    /// A whole expected frame: `body`, then the status line starting
    /// with `status_left`.
    fn frame(body: &[&str; BODY_ROWS], status_left: &str) -> Vec<String> {
        body.iter()
            .copied()
            .map(str::to_string)
            .chain([status_row(status_left, None)])
            .collect()
    }

    #[test]
    fn the_first_frame_shows_the_empty_summary() {
        let mut app = App::new_for_test().expect("test app should build");

        assert_eq!(drawn_rows(&mut app, 1), frame(&FIRST_FRAME, GRID_STATUS));
    }

    #[test]
    fn two_additions_open_cells_two_and_three() {
        let mut app = laid_out_app();
        dispatch_key(&mut app, key('+'));
        dispatch_key(&mut app, key('+'));
        app.tiles.settle_for_test();

        assert_eq!(
            drawn_rows(&mut app, 1),
            frame(&AFTER_TWO_ADDITIONS, GRID_STATUS)
        );
    }

    #[test]
    fn a_removal_after_two_additions_leaves_cell_two() {
        let mut app = laid_out_app();
        dispatch_key(&mut app, key('+'));
        dispatch_key(&mut app, key('+'));
        dispatch_key(&mut app, key('-'));
        app.tiles.settle_for_test();

        assert_eq!(
            drawn_rows(&mut app, 1),
            frame(&AFTER_TWO_ADDITIONS_AND_A_REMOVAL, GRID_STATUS)
        );
    }

    #[test]
    fn the_settings_overlay_lists_appearance_tiles_and_files() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, key('s'));

        assert_eq!(
            drawn_rows(&mut app, 1),
            frame(&SETTINGS_OVERLAY, OVERLAY_STATUS)
        );
    }

    #[test]
    fn the_keymap_overlay_opens_on_its_first_screen() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, ctrl('k'));

        assert_eq!(
            drawn_rows(&mut app, 1),
            frame(&KEYMAP_OVERLAY, OVERLAY_STATUS)
        );
    }

    #[test]
    fn the_shortcuts_overlay_opens_on_its_first_screen() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, key('?'));

        assert_eq!(
            drawn_rows(&mut app, 1),
            frame(&SHORTCUTS_OVERLAY, OVERLAY_STATUS)
        );
    }

    #[test]
    fn the_favorites_overlay_lists_its_one_favorite() {
        let mut app = App::new_for_test().expect("test app should build");
        let rows =
            tui_pane::parse_favorite_rows_for_test(FAVORITE).expect("the favorite should parse");
        tui_pane::open_favorites_on_state_for_test(
            &mut app,
            FavoritesFileState::Loaded {
                path: PathBuf::from("/tmp/favorites.toml"),
                rows,
            },
        );

        assert_eq!(
            drawn_rows(&mut app, 1),
            frame(&FAVORITES_OVERLAY, GRID_STATUS)
        );
    }

    /// Three frames after `a`, the status line says the grid is being
    /// drawn over. The body is left out: what the attract screen draws
    /// there depends on the desktop behind the terminal.
    #[test]
    fn an_attract_screen_asked_for_is_named_on_the_status_line() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, key('a'));

        let rows = drawn_rows(&mut app, 3);

        assert_eq!(
            rows.last(),
            Some(&status_row(GRID_STATUS, Some(ATTRACT_NOTE_LABEL)))
        );
    }
}
