//! Frame rendering: the tile grid, the framework status line along the
//! bottom, and whichever overlay is open above them.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::AttractWork;
use tui_pane::BarPalette;
use tui_pane::Keymap;
use tui_pane::ScanIndicator;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::StatusLineNote;
use tui_pane::TileCells;
use tui_pane::TileDemand;
use tui_pane::TileGridContents;
use tui_pane::Updates;
use tui_pane::draw_attract_layers;
use tui_pane::render_status_line;

use crate::agent_cell;
use crate::agent_cell::AgentEntry;
use crate::app::App;
use crate::census;
use crate::census::Machine;
use crate::constants::AGENT_CELL_TITLE_LEAD;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::ATTRACT_NOTE_LABEL;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::globals::AppGlobalAction;
use crate::settings;
use crate::summary;
use crate::tiles::AgentCell;
use crate::tiles::TileContent;
use crate::tiles::TileDemands;

/// Draw one frame: the grid fills the terminal above the status line,
/// and the toasts, the favorites modal and any framework overlay float
/// above both.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());

    // The listed agents are the grid's work, so the attract screen comes
    // on by itself only once no machine lists any.
    let work = if app
        .census
        .lists_an_agent(&app.loaded_config.config.machines.remote)
    {
        AttractWork::Running
    } else {
        AttractWork::Idle
    };
    draw_attract_layers(
        frame,
        app,
        body,
        work,
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
/// the summary and each agent's cell.
fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect, contents: TileGridContents) {
    let initial_rows = app.loaded_config.config.tiles.initial_rows();
    let cells = Cells::new(
        app.census
            .machines(&app.loaded_config.config.machines.remote),
        census::unix_now(),
    );
    tui_pane::draw_tile_grid(
        frame.buffer_mut(),
        &mut app.tiles,
        area,
        initial_rows,
        contents,
        &cells,
    );
}

/// What the tile grid's cells hold: the summary of every machine's
/// agents, and a cell for each agent.
struct Cells<'a> {
    /// This machine, then each configured remote.
    machines: Vec<Machine<'a>>,
    /// Every agent's cell, in grid order.
    agents:   Vec<AgentEntry<'a>>,
    /// The unix second the ages are measured to.
    now:      u64,
}

impl<'a> Cells<'a> {
    /// The cells for `machines`, with ages measured to `now`.
    fn new(machines: Vec<Machine<'a>>, now: u64) -> Self {
        let agents = agent_cell::cell_order(&machines);
        Self {
            machines,
            agents,
            now,
        }
    }

    /// The agent the cell `id` draws, while it is still listed.
    fn agent(&self, id: &AgentCell) -> Option<&AgentEntry<'a>> {
        self.agents.iter().find(|entry| entry.id == *id)
    }
}

impl TileCells<AgentCell> for Cells<'_> {
    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    /// Every row of the summary and of an agent's cell is one line at any
    /// width: a name too long for its column is cut, not wrapped.
    fn demands(&self, _widths: &[(TileContent, u16)]) -> TileDemands {
        TileDemands {
            summary: summary::height(&self.machines),
            groups:  self
                .agents
                .iter()
                .map(|entry| TileDemand {
                    id:   entry.id.clone(),
                    rows: agent_cell::height(entry.row),
                })
                .collect(),
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, _ground: Color) {
        match content {
            TileContent::Summary => summary::draw(buffer, inner, &self.machines, self.now),
            TileContent::Group(id) => {
                if let Some(entry) = self.agent(id) {
                    agent_cell::draw(
                        buffer,
                        inner,
                        entry.row,
                        entry.launcher,
                        entry.machine,
                        self.now,
                    );
                }
            },
            // The grid draws an empty cell's number itself.
            TileContent::Empty(_) => {},
        }
    }

    /// An agent's cell is titled with the agent's name.
    fn group_title(&self, id: &AgentCell) -> Option<String> {
        self.agent(id)
            .map(|entry| format!("{AGENT_CELL_TITLE_LEAD}{}", entry.row.name))
    }
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
    use crate::census::Agent;
    use crate::census::AgentRow;
    use crate::census::CensusUpdate;
    use crate::census::ChildKind;
    use crate::census::ChildRow;

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

    /// The first frame: the summary alone, this machine's heading on its
    /// first row and its readout on the last.
    const FIRST_FRAME: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · no agents                                                          │",
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
        "│                                                                              │",
        "│                                                  content rows: 1  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// After `+` `+`: the summary over cells 2 and 3, each numbered.
    const AFTER_TWO_ADDITIONS: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · no agents                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                   content rows: 1  r/c: 7/78 │",
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
        "│ natedev · no agents                                                          │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 1  r/c: 11/78 │",
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
        "│ natedev · no agents                                                          │",
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
        "│       │ Machines:                                                    │       │",
        "│       │   remote        none                                         │       │",
        "│       │ Files:                                                       │       │",
        "│       │   config        /<config>/cargo-handler/config.toml          │       │",
        "│       │   themes        /<config>/cargo-handler/themes               │       │",
        "│       │   keymap        /<config>/cargo-handler/keymap.toml          │       │",
        "│       └──────────────────────────────────────────────────────────────┘       │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 1  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The keymap overlay's first screen.
    const KEYMAP_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · no agents                                                          │",
        "│        ┌ Keymap ───────────────────────────────────────────────────┐         │",
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
        "│                                                  content rows: 1  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The `?` overlay's first screen, with no `x` Dismiss row.
    const SHORTCUTS_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · no ┌ Global Shortcuts ─────────────────────────────┐               │",
        "│              │                                               │               │",
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
        "│              │   Show global shortcuts             ?         │ 1  r/c: 21/78 │",
        "└──────────────└───────────────────1 of 2 ▼────────────────────┘───────────────┘",
    ];

    /// The favorites overlay on one saved moving-band favorite.
    const FAVORITES_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · no agents                                                          │",
        "│                                                                              │",
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
        "│                                                  content rows: 1  r/c: 21/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The unix second the agent cell golden measures ages to.
    const NOW: u64 = 1_790_372_800;

    /// The grid with one agent listed: the summary, then the agent's
    /// cell, titled with its name, holding a shell running a Codex app
    /// server with one thread, and a subagent.
    const ONE_AGENT_CELL: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · 1 agent                                                            │",
        "│ pid      agent   name            status  age  directory                      │",
        "│ 1579022  claude  boss of bosses  idle    21h  ~/rust/hana_catalyst/docs/hana │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 3  r/c: 11/78 │",
        "├ boss of bosses───────────────────────────────────────────────────────────────┤",
        "│ pid 1579022 · claude · idle · 21h · natedev                                  │",
        "│ ~/rust/hana_catalyst/docs/hana                                               │",
        "│                                                                              │",
        "│ pid      kind        name                                    age             │",
        "│ 2406969  shell       Launch the Phase 2 implementation seat  12m             │",
        "│ 2407001    codex     app-server                              12m             │",
        "│ —            thread  tool-based-ui-geometry-material-impl    12m             │",
        "│ —        subagent    Survey the tile grid                    5m 3s           │",
        "│                                                   content rows: 8  r/c: 9/78 │",
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

    /// boss of bosses, running a shell that runs a Codex app server with
    /// one thread, and a subagent, started `age` seconds before [`NOW`].
    fn boss(age: u64) -> AgentRow {
        let child = |depth, kind, pid, name: &str, age: u64| ChildRow {
            depth,
            kind,
            pid,
            name: name.to_string(),
            started: NOW - age,
        };
        AgentRow {
            agent:       Agent::Claude,
            name:        "boss of bosses".to_string(),
            status:      Some("idle".to_string()),
            started:     NOW - age,
            pid:         1_579_022,
            directory:   "~/rust/hana_catalyst/docs/hana".to_string(),
            launched_by: None,
            children:    vec![
                child(
                    0,
                    ChildKind::Shell,
                    Some(2_406_969),
                    "Launch the Phase 2 implementation seat",
                    12 * 60,
                ),
                child(
                    1,
                    ChildKind::Process(Agent::Codex),
                    Some(2_407_001),
                    "app-server",
                    12 * 60,
                ),
                child(
                    2,
                    ChildKind::Thread,
                    None,
                    "tool-based-ui-geometry-material-impl",
                    12 * 60,
                ),
                child(
                    0,
                    ChildKind::Subagent,
                    None,
                    "Survey the tile grid",
                    5 * 60 + 3,
                ),
            ],
        }
    }

    /// The grid of `app` drawn into the body of an 80×24 frame with ages
    /// measured to [`NOW`], as rows of text.
    fn drawn_grid(app: &mut App) -> Vec<String> {
        let body = Rect::new(0, 0, WIDTH, HEIGHT - STATUS_LINE_HEIGHT);
        let initial_rows = app.loaded_config.config.tiles.initial_rows();
        let cells = Cells::new(
            app.census
                .machines(&app.loaded_config.config.machines.remote),
            NOW,
        );
        let mut buffer = Buffer::empty(body);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut app.tiles,
            body,
            initial_rows,
            TileGridContents::Shown,
            &cells,
        );
        (body.top()..body.bottom())
            .map(|row| {
                (body.left()..body.right())
                    .filter_map(|column| buffer.cell((column, row)).map(Cell::symbol))
                    .collect()
            })
            .collect()
    }

    /// A listed agent opens a cell of its own below the summary, titled
    /// with its name, holding its header and the tree of what it runs.
    #[test]
    fn a_listed_agent_gets_a_cell_titled_with_its_name() {
        let mut app = laid_out_app();
        app.census
            .apply(CensusUpdate::Local(vec![boss(21 * 60 * 60)]));
        drawn_grid(&mut app);
        app.tiles.settle_for_test();

        assert_eq!(drawn_grid(&mut app), ONE_AGENT_CELL);
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
