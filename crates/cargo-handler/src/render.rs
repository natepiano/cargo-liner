//! Frame rendering: the tile grid, the framework status line along the
//! bottom, and whichever overlay is open above them.

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tui_pane::AttractGridPresentation;
use tui_pane::AttractWork;
use tui_pane::BarPalette;
use tui_pane::CellClick;
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
use tui_pane::label_color;
use tui_pane::render_status_line;
use tui_pane::title_color;

use crate::agent_cell;
use crate::agent_cell::AgentEntry;
use crate::agent_cell::AgentTile;
use crate::agent_cell::TreeColumn;
use crate::app::App;
use crate::census;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::Machine;
use crate::census::RoleLabel;
use crate::census::accounts::AccountsTable;
#[cfg(test)]
use crate::census::scan::ScanStage;
use crate::constants::AGENT_CELL_TITLE_LEAD;
use crate::constants::APP_NAME;
use crate::constants::APP_VERSION;
use crate::constants::ATTRACT_NOTE_LABEL;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::STATUS_LINE_HEIGHT;
use crate::constants::SUMMARY_CELL_TITLE;
use crate::constants::UNASSIGNED_CELL_TITLE;
use crate::footer_colors::FooterColors;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::globals::AppGlobalAction;
use crate::settings;
use crate::sort;
use crate::sort::AccountsSort;
use crate::sort::HeaderHit;
use crate::sort::HeaderSpan;
use crate::sort::SummarySort;
use crate::sort::TreeSort;
use crate::summary;
use crate::summary::SummaryFrame;
use crate::theme::Role;
use crate::tiles::TileContent;
use crate::tiles::TileDemands;
use crate::tiles::TileKey;

/// Draw one frame: the grid fills the body above the status line, and
/// toasts are drawn inside the grid's outer frame. The favorites modal
/// and any framework overlay float above both.
pub(crate) fn draw(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());

    // Agent rows and visible account readings or failures keep the summary
    // present; only an empty watcher returns to the attract screen.
    let work = if app
        .census
        .lists_an_agent(&app.loaded_config.config.machines.remote)
        || app.accounts.table.is_visible()
        || app
            .census
            .has_hierarchy_notice(&app.loaded_config.config.machines.remote)
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
    tui_pane::render_toasts(frame, &mut app.framework, tui_pane::frame_inner(body));
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
    let growth = app.loaded_config.config.tiles.growth();
    let cells = Cells::new(
        app.census
            .machines(&app.loaded_config.config.machines.remote),
        census::unix_now(),
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
    )
    .with_accounts(&app.accounts.table)
    .with_accounts_sort(app.accounts_sort);
    app.tiles.set_view(app.loaded_config.config.tiles.view);
    tui_pane::draw_tile_grid(
        frame.buffer_mut(),
        &mut app.tiles,
        area,
        growth,
        contents,
        &cells,
    );
}

/// Map a content press through the same census and header layouts as drawing.
pub(crate) fn cell_clicked(app: &mut App, click: CellClick<TileKey>) {
    match app.attract.configuration().grid_presentation() {
        AttractGridPresentation::ReplacesGrid => return,
        AttractGridPresentation::OverGrid => {},
    }
    let CellClick {
        content,
        area,
        point,
        ..
    } = click;
    let area = Rect::new(0, 0, area.width, area.height);
    let cells = Cells::new(
        app.census
            .machines(&app.loaded_config.config.machines.remote),
        census::unix_now(),
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
    )
    .with_accounts(&app.accounts.table)
    .with_accounts_sort(app.accounts_sort);
    match content {
        TileContent::Summary => {
            if let HeaderHit::Column(column) =
                sort::header_hit(&summary::header_spans(area, &cells.summary_frame()), point)
            {
                app.summary_sort.select(column);
            } else if let HeaderHit::Column(column) = sort::header_hit(
                &summary::accounts_header_spans(area, &cells.summary_frame()),
                point,
            ) {
                app.accounts_sort.select(column);
            }
        },
        TileContent::Group(id) => {
            if let Some(tile) = cells.tile(&id)
                && let HeaderHit::Column(column) =
                    sort::header_hit(&cells.header_spans(tile, area), point)
            {
                app.tree_sorts.entry(id).or_default().select(column);
            }
        },
        TileContent::Empty(_) => {},
    }
}

/// The per-session summary source and the production-role or pooled tiles.
struct Cells<'a> {
    /// This machine, then each configured remote.
    machines:       Vec<Machine<'a>>,
    /// Individual sessions in default hue order, independent of tile grouping.
    agents:         Vec<AgentEntry<'a>>,
    /// The account columns and agent labels shared by every cell.
    account_column: AccountColumn,
    /// The forest shared by cell order and summary roots.
    placement:      AgentPlacement<'a>,
    /// Grouped tiles, apart from the per-session summary hue lookup.
    tiles:          Vec<AgentTile<'a>>,
    /// The unix second the ages are measured to.
    now:            u64,
    /// Footer colors already loaded by the app.
    footer_colors:  &'a FooterColors,
    /// Ordering shared by all machine groups in the summary.
    summary_sort:   SummarySort,
    /// Account readings share the summary cell, independent of agent placement.
    accounts:       &'a AccountsTable,
    /// Source order until an accounts header is selected.
    accounts_sort:  AccountsSort,
}

impl<'a> Cells<'a> {
    /// Build each default tree once, then reorder only tiles with a saved sort.
    fn new(
        machines: Vec<Machine<'a>>,
        now: u64,
        footer_colors: &'a FooterColors,
        sorts: &HashMap<TileKey, TreeSort>,
        summary_sort: SummarySort,
    ) -> Self {
        let placement = AgentPlacement::from_machines(&machines);
        let agents = agent_cell::cell_order_placed(&placement);
        let tiles = agent_cell::group_tiles(&placement, &agents, sorts, now);
        let account_column = agent_cell::account_column(&agents);
        Self {
            machines,
            agents,
            account_column,
            placement,
            tiles,
            now,
            footer_colors,
            summary_sort,
            accounts: &AccountsTable::NotConfigured,
            accounts_sort: AccountsSort::SourceOrder,
        }
    }

    /// Attach the UI's current account reading to this frame's cells.
    const fn with_accounts(mut self, accounts: &'a AccountsTable) -> Self {
        self.accounts = accounts;
        self
    }

    /// Attach the accounts selection without changing the machine-group selection.
    const fn with_accounts_sort(mut self, sort: AccountsSort) -> Self {
        self.accounts_sort = sort;
        self
    }

    /// The inputs shared by summary drawing and header hit testing.
    fn summary_frame(&self) -> SummaryFrame<'_, 'a> {
        SummaryFrame {
            accounts:       self.accounts,
            accounts_sort:  self.accounts_sort,
            machines:       &self.machines,
            placement:      &self.placement,
            cells:          &self.agents,
            now:            self.now,
            account_column: self.account_column,
            footer_colors:  self.footer_colors,
            sort:           self.summary_sort,
        }
    }

    /// The tile named by a stable session or machine-pool identity.
    fn tile(&self, id: &TileKey) -> Option<&AgentTile<'a>> {
        self.tiles.iter().find(|tile| tile.key() == *id)
    }

    /// Resolve either tile's headers through its shared drawing layout.
    fn header_spans(&self, tile: &AgentTile<'_>, area: Rect) -> Vec<HeaderSpan<TreeColumn>> {
        match tile {
            AgentTile::Agent(entry) => {
                agent_cell::header_spans(entry, area, self.now, self.account_column)
            },
            AgentTile::Unassigned(entry) => {
                agent_cell::unassigned_header_spans(entry, area, self.now, self.account_column)
            },
        }
    }
}

impl TileCells<TileKey> for Cells<'_> {
    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    /// `TileCells::demands` measures summary rows as single lines: long names
    /// are truncated, not wrapped. `TileDemands.summary_width` is the widest
    /// summary row; the grid uses it to widen the summary when
    /// `tiles.widen_summary` is enabled. `agent_cell::height` measures each
    /// agent cell at its current width, since headers, directories and names
    /// can require more rows at narrower widths. Cells absent from `widths`
    /// use the narrowest supplied width, or zero when `widths` is empty.
    fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands {
        let narrowest = widths
            .iter()
            .map(|&(_, width)| width)
            .min()
            .unwrap_or_default();
        let width_of = |id: &TileKey| {
            widths
                .iter()
                .find(|(content, _)| matches!(content, TileContent::Group(group) if group == id))
                .map_or(narrowest, |&(_, width)| width)
        };
        TileDemands {
            summary:       summary::height(&self.machines, &self.placement, self.accounts),
            summary_width: summary::width(
                &self.machines,
                &self.placement,
                self.now,
                self.account_column,
                self.summary_sort,
                self.accounts,
                self.accounts_sort,
            ),
            groups:        self
                .tiles
                .iter()
                .map(|tile| {
                    let id = tile.key();
                    let width = width_of(&id);
                    let rows = match tile {
                        AgentTile::Agent(entry) => {
                            agent_cell::height(entry, width, self.now, self.account_column)
                        },
                        AgentTile::Unassigned(entry) => agent_cell::unassigned_height(entry),
                    };
                    TileDemand { id, rows }
                })
                .collect(),
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, _ground: Color) {
        match content {
            TileContent::Summary => {
                summary::draw(buffer, inner, &self.summary_frame());
            },
            TileContent::Group(id) => match self.tile(id) {
                Some(AgentTile::Agent(entry)) => agent_cell::draw(
                    buffer,
                    inner,
                    entry,
                    &self.agents,
                    self.now,
                    self.account_column,
                    self.footer_colors,
                ),
                Some(AgentTile::Unassigned(entry)) => agent_cell::draw_unassigned(
                    buffer,
                    inner,
                    entry,
                    self.now,
                    self.account_column,
                    self.footer_colors,
                ),
                None => {},
            },
            // The grid draws an empty cell's number itself.
            TileContent::Empty(_) => {},
        }
    }

    /// Session titles retain their hue; pooled titles use the theme's title color.
    fn group_title(&self, id: &TileKey) -> Option<Line<'static>> {
        self.tile(id).map(|tile| match tile {
            AgentTile::Unassigned(entry) => Line::from(Span::styled(
                format!(
                    "{AGENT_CELL_TITLE_LEAD}{}{HEADING_SEPARATOR}{}",
                    UNASSIGNED_CELL_TITLE, entry.machine
                ),
                Style::default().fg(title_color()),
            )),
            AgentTile::Agent(entry) => Line::from(vec![
                Span::styled(
                    format!("{AGENT_CELL_TITLE_LEAD}{}", entry.row.name),
                    Role::Rainbow(entry.hue).style(),
                ),
                Span::styled(
                    entry.role_label.suffix(),
                    match entry.role_label {
                        RoleLabel::Stated(role) | RoleLabel::Inferred(role) => {
                            self.footer_colors.role(role.into())
                        },
                        RoleLabel::ReviewFallback => self.footer_colors.role(FooterRole::Worker),
                        RoleLabel::Unlabeled => FooterTint::Plain,
                    }
                    .style(Style::default().fg(label_color())),
                ),
            ]),
        })
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

    use HierarchyRole::Fix;
    use HierarchyRole::Review;
    use HierarchyRole::Showrunner;
    use HierarchyRole::Test;
    use HierarchyRole::Unit;
    use RoleLabel::Inferred;
    use RoleLabel::Stated;
    use RoleLabel::Unlabeled;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Cell;
    use tui_pane::CellPoint;
    use tui_pane::FavoritesFileState;
    use tui_pane::dispatch_key;

    use super::*;
    use crate::agent_cell::TreeColumn;
    use crate::census::Account;
    use crate::census::Agent;
    use crate::census::AgentActivity;
    use crate::census::AgentRow;
    use crate::census::CensusUpdate;
    use crate::census::ChildKind;
    use crate::census::ChildRow;
    use crate::census::CodexThreadId;
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyRole;
    use crate::census::MachineState;
    use crate::census::RoleFallback;
    use crate::census::RoleLabel;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::hierarchy::ReviewerState;
    use crate::census::worker_dispatch::WorkerDispatch;
    use crate::census::worker_dispatch::WorkerTask;
    use crate::constants::HEADING_SEPARATOR;
    use crate::sort::SortDirection;
    use crate::sort::SummaryColumn;

    /// Width of every frame the goldens draw.
    const WIDTH: u16 = 80;
    /// Height of every frame the goldens draw.
    const HEIGHT: u16 = 24;
    /// Rows above the status line.
    const BODY_ROWS: usize = 23;

    /// The status line's left end over the grid: the uptime alone.
    const GRID_STATUS: &str = " Uptime: 0s";
    /// An overlay status line draws uptime before navigation and omits the
    /// centre action when it has no whole space before the right-side notes.
    const OVERLAY_STATUS: &str = " Uptime: 0s  ↑/↓ nav   tab pane";

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
        "│                                                   content rows: 0  r/c: 6/78 │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│ 3                                                                            │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                   content rows: 0  r/c: 6/78 │",
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
        "│                                                  content rows: 1  r/c: 10/78 │",
        "├──────────────────────────────────────────────────────────────────────────────┤",
        "│ 2                                                                            │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 0  r/c: 10/78 │",
        "└──────────────────────────────────────────────────────────────────────────────┘",
    ];

    /// The settings overlay: Appearance, Tiles, Machines, Census and Files; paths under
    /// the test build's stand-in config root.
    const SETTINGS_OVERLAY: [&str; BODY_ROWS] = [
        "┌ summary──────────────────────────────────────────────────────────────────────┐",
        "│ natedev · no agents                                                          │",
        "│       ┌ Settings ────────────────────────────────────────────────────┐       │",
        "│       │ Appearance:                                                  │       │",
        "│       │ ▶ mode               < auto >                                │       │",
        "│       │   light theme        < Default Light >                       │       │",
        "│       │   dark theme         < Default Dark >                        │       │",
        "│       │   transparent        < true >                                │       │",
        "│       │ Tiles:                                                       │       │",
        "│       │   initial rows       < 4 >                                   │       │",
        "│       │   fill               < redistribute >                        │       │",
        "│       │   view               < cells >                               │       │",
        "│       │   widen summary      < true >                                │       │",
        "│       │ Machines:                                                    │       │",
        "│       │   remote             none                                    │       │",
        "│       │ Census:                                                      │       │",
        "│       │   hierarchy command                                          │       │",
        "│       │ Files:                                                       │       │",
        "│       │   config             /<config>/cargo-handler/config.toml     │       │",
        "│       │   themes             /<config>/cargo-handler/themes          │       │",
        "│       │   keymap             /<config>/cargo-handler/keymap.toml     │       │",
        "│       └──────────────────────────────────────────────────────────────┘ 21/78 │",
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

    /// The grid with one unassigned session: summary, then the machine's
    /// pool with its flat session row. The summary excludes that session.
    const ONE_AGENT_CELL: [&str; BODY_ROWS] = [
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
        "│                                                  content rows: 1  r/c: 10/78 │",
        "├ unassigned · natedev─────────────────────────────────────────────────────────┤",
        "│ session         role  status  branch  directory                       age ▼  │",
        "│ boss of bosses  —     idle    —       ~/rust/hana_catalyst/docs/hana  21h    │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                                              │",
        "│                                                  content rows: 2  r/c: 10/78 │",
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
        let growth = app.loaded_config.config.tiles.growth();
        app.tiles
            .set_layout(Rect::new(0, 0, WIDTH, HEIGHT - STATUS_LINE_HEIGHT), growth);
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
    fn a_toast_never_covers_the_status_line() {
        let mut app = App::new_for_test().expect("test app should build");
        let _ = app.framework.toasts.push(
            "notice",
            "This toast occupies the bottom of the body while the status line stays visible.",
        );

        let rows = drawn_rows(&mut app, 1);

        assert_eq!(rows.last(), Some(&status_row(GRID_STATUS, None)));
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
            worker_task: WorkerTask::Unrecorded,
            dispatch: WorkerDispatch::Plain,
            activity: AgentActivity::Idle,
            thread_id: CodexThreadId::Unrecorded,
            depth,
            kind,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            pid,
            name: name.to_string(),
            started: NOW - age,
            timer: None,
        };
        AgentRow {
            role_fallback:   RoleFallback::None,
            session_id:      SessionIdentity::Unrecorded,
            agent:           Agent::Claude,
            service_tier:    ServiceTier::Unrecorded,
            account:         Account::Unrecorded,
            name:            "boss of bosses".to_string(),
            activity:        AgentActivity::from(Some("idle")),
            started:         NOW - age,
            process_started: NOW - age,
            pid:             1_579_022,
            desktop:         None,
            directory:       "~/rust/hana_catalyst/docs/hana".to_string(),
            branch:          None,
            launched_by:     None,
            children:        vec![
                child(
                    0,
                    ChildKind::Shell,
                    Some(2_406_969),
                    "Launch the Phase 2 implementation seat",
                    12 * 60,
                ),
                child(
                    1,
                    ChildKind::UnderShell(Agent::Codex),
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

    /// Give each layout fixture root a real production responsibility.
    fn production_roots(mut roots: Vec<AgentRow>) -> (Vec<AgentRow>, HierarchyEdges) {
        let mut members = Vec::new();
        let mut edges = Vec::new();
        for root in &mut roots {
            let manager = format!("root-{}", root.pid);
            let member = format!("member-{}", root.pid);
            root.session_id = SessionIdentity::Claude(manager.clone());
            members.push(AgentRow {
                role_fallback: RoleFallback::None,
                session_id: SessionIdentity::Claude(member.clone()),
                pid: root.pid + 1_000_000,
                name: member.clone(),
                children: Vec::new(),
                ..root.clone()
            });
            edges.push(HierarchyEdge {
                reviewer: ReviewerState::NotPool,
                manager,
                member,
                role: HierarchyRole::Impl,
            });
        }
        roots.extend(members);
        (roots, HierarchyEdges(edges))
    }

    /// The host geometry selects only the clicked tile's header preference.
    #[test]
    fn content_clicks_select_and_reverse_only_the_named_tile_sort() {
        let mut app = App::new_for_test().expect("a test app");
        app.census.apply(CensusUpdate::Local {
            rows:      vec![boss(900)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        let area = Rect::new(0, 0, 220, 30);
        let (id, point) = {
            let cells = Cells::new(
                app.census.machines(&[]),
                NOW,
                app.footer_colors.colors(),
                &app.tree_sorts,
                app.summary_sort,
            );
            let tile = &cells.tiles[0];
            let span = cells
                .header_spans(tile, area)
                .into_iter()
                .find(|span| span.column == TreeColumn::Role)
                .expect("a role header");
            (
                tile.key(),
                CellPoint {
                    column: span.columns.start,
                    row:    span.row,
                },
            )
        };
        let click = CellClick {
            index: 2,
            content: TileContent::Group(id.clone()),
            area: Rect::new(8, 5, area.width, area.height),
            point,
        };
        cell_clicked(&mut app, click.clone());
        assert_eq!(
            app.tree_sorts[&id],
            TreeSort {
                column:    TreeColumn::Role,
                direction: SortDirection::Ascending,
            }
        );
        assert_eq!(app.summary_sort, SummarySort::default());
        cell_clicked(&mut app, click.clone());
        assert_eq!(app.tree_sorts[&id].direction, SortDirection::Descending);
        let selected = app.tree_sorts[&id];
        cell_clicked(
            &mut app,
            CellClick {
                point: CellPoint {
                    row: point.row + 1,
                    ..point
                },
                ..click
            },
        );
        assert_eq!(
            app.tree_sorts[&id], selected,
            "a data row keeps the preference"
        );
    }

    /// Stale grid hit areas cannot change hidden preferences during replacement or departure.
    #[test]
    fn attract_replacement_and_departure_ignore_tree_and_summary_header_clicks() {
        let mut app = App::new_for_test().expect("a test app");
        app.census.apply(CensusUpdate::Local {
            rows:      vec![boss(900)],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        let area = Rect::new(0, 0, 220, 30);
        let clicks = {
            let cells = Cells::new(
                app.census.machines(&[]),
                NOW,
                app.footer_colors.colors(),
                &HashMap::new(),
                SummarySort::default(),
            );
            let tile = &cells.tiles[0];
            let tree = cells
                .header_spans(tile, area)
                .into_iter()
                .find(|span| span.column == TreeColumn::Role)
                .expect("tree header");
            let summary = summary::header_spans(area, &cells.summary_frame())
                .into_iter()
                .find(|span| span.column == SummaryColumn::Agent)
                .expect("summary header");
            [
                CellClick {
                    index: 1,
                    content: TileContent::Group(tile.key()),
                    area,
                    point: CellPoint {
                        column: tree.columns.start,
                        row:    tree.row,
                    },
                },
                CellClick {
                    index: 0,
                    content: TileContent::Summary,
                    area,
                    point: CellPoint {
                        column: summary.columns.start,
                        row:    summary.row,
                    },
                },
            ]
        };
        app.attract.request_show();
        for click in &clicks {
            cell_clicked(&mut app, click.clone());
        }
        assert!(app.tree_sorts.is_empty());
        assert_eq!(app.summary_sort, SummarySort::default());
        app.attract.toggle();
        assert!(
            !app.attract.asked_for(),
            "departure retains replacement presentation"
        );
        for click in &clicks {
            cell_clicked(&mut app, click.clone());
        }
        assert!(app.tree_sorts.is_empty());
        assert_eq!(app.summary_sort, SummarySort::default());
    }

    /// The grid of `app` drawn into the body of an 80×24 frame with ages
    /// measured to [`NOW`].
    fn drawn_buffer(app: &mut App) -> Buffer {
        let body = Rect::new(0, 0, WIDTH, HEIGHT - STATUS_LINE_HEIGHT);
        drawn_buffer_in(app, body)
    }

    /// The grid at a chosen body size with ages measured to [`NOW`].
    fn drawn_buffer_in(app: &mut App, body: Rect) -> Buffer {
        let growth = app.loaded_config.config.tiles.growth();
        let cells = Cells::new(
            app.census
                .machines(&app.loaded_config.config.machines.remote),
            NOW,
            app.footer_colors.colors(),
            &HashMap::new(),
            SummarySort::default(),
        );
        let mut buffer = Buffer::empty(body);
        tui_pane::draw_tile_grid(
            &mut buffer,
            &mut app.tiles,
            body,
            growth,
            TileGridContents::Shown,
            &cells,
        );
        buffer
    }

    /// [`drawn_buffer`] as rows of text.
    fn drawn_grid(app: &mut App) -> Vec<String> {
        let buffer = drawn_buffer(app);
        let body = buffer.area;
        (body.top()..body.bottom())
            .map(|row| {
                (body.left()..body.right())
                    .filter_map(|column| buffer.cell((column, row)).map(Cell::symbol))
                    .collect()
            })
            .collect()
    }

    /// An unassigned session opens the pool below the summary, with its
    /// own values in a flat row above the tree of what it runs.
    #[test]
    fn an_unassigned_agent_gets_a_flat_row_in_the_machine_pool() {
        let mut app = laid_out_app();
        app.census.apply(CensusUpdate::Local {
            rows:      vec![AgentRow {
                children: Vec::new(),
                ..boss(21 * 60 * 60)
            }],
            hierarchy: crate::census::HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        });
        drawn_grid(&mut app);
        app.tiles.settle_for_test();

        assert_eq!(drawn_grid(&mut app), ONE_AGENT_CELL);
    }

    /// A manager and member whose accepted relationship can change between scans.
    fn relationship_rows() -> Vec<AgentRow> {
        vec![
            AgentRow {
                role_fallback: crate::census::RoleFallback::None,
                session_id: SessionIdentity::Claude("production".to_string()),
                name: "production".to_string(),
                pid: 1,
                children: Vec::new(),
                ..boss(3 * 60 * 60)
            },
            AgentRow {
                role_fallback: crate::census::RoleFallback::None,
                session_id: SessionIdentity::Claude("member".to_string()),
                name: "member".to_string(),
                pid: 2,
                launched_by: Some(1),
                children: Vec::new(),
                ..boss(2 * 60 * 60)
            },
        ]
    }

    /// Tile titles and nested lines follow the current relationships, retaining name and label
    /// colors.
    #[test]
    fn title_and_tree_role_labels_change_with_accepted_relationships() {
        let mut app = laid_out_app();
        let rows = relationship_rows();
        for (role, manager_role_label, member_role_label) in [
            (None, Unlabeled, Unlabeled),
            (Some(Unit), Inferred(Showrunner), Stated(Unit)),
            (Some(Test), Inferred(Unit), Stated(Test)),
            (Some(Review), Inferred(Unit), Stated(Review)),
            (Some(Fix), Inferred(Unit), Stated(Fix)),
            (None, Unlabeled, Unlabeled),
        ] {
            let manager_label = manager_role_label.suffix();
            let edges = HierarchyEdges(
                role.into_iter()
                    .map(|role| HierarchyEdge {
                        reviewer: ReviewerState::NotPool,
                        manager: "production".to_string(),
                        member: "member".to_string(),
                        role,
                    })
                    .collect(),
            );
            app.census.apply(CensusUpdate::Local {
                rows:      rows.clone(),
                hierarchy: edges.into(),
                stage:     ScanStage::Classified,
            });
            let body = Rect::new(0, 0, 160, 40);
            drawn_buffer_in(&mut app, body);
            app.tiles.settle_for_test();
            let buffer = drawn_buffer_in(&mut app, body);
            let rendered: Vec<String> = (0..body.height)
                .map(|y| (0..body.width).map(|x| buffer[(x, y)].symbol()).collect())
                .collect();
            let title_text = format!("{AGENT_CELL_TITLE_LEAD}production{manager_label}");
            let (title_y, title_line) = rendered
                .iter()
                .enumerate()
                .find(|(_, line)| line.contains(&title_text) && line.contains('─'))
                .expect("the grid draws the current title");
            let prefix = title_line
                .split_once(&title_text)
                .expect("the title is present")
                .0
                .chars()
                .count();
            let title_x = u16::try_from(prefix).expect("the column fits");
            let title_y = u16::try_from(title_y).expect("the row fits");
            assert_eq!(
                buffer[(title_x + 1, title_y)].fg,
                Role::Rainbow(crate::theme::RainbowHue::Red)
                    .style()
                    .fg
                    .expect("the title has a hue")
            );
            let after_name = title_x
                + u16::try_from(format!("{AGENT_CELL_TITLE_LEAD}production").chars().count())
                    .expect("the title fits");
            if manager_label.is_empty() {
                assert_eq!(
                    buffer[(after_name, title_y)].symbol(),
                    "─",
                    "launch-only groups have no role suffix"
                );
            } else {
                for offset in 0..manager_label.chars().count() {
                    assert_eq!(
                        buffer[(
                            after_name + u16::try_from(offset).expect("the suffix fits"),
                            title_y
                        )]
                            .fg,
                        label_color()
                    );
                }
            }
            let member_line = rendered
                .iter()
                .find(|line| line.contains("└─ member"))
                .expect("the child has a tree line");
            assert!(member_line.contains(member_role_label.column_text()));
            assert!(!member_line.contains(&format!("member{HEADING_SEPARATOR}")));
        }
    }

    #[test]
    fn the_live_width_summary_spans_bands_without_an_interior_divider() {
        let body = Rect::new(0, 0, 429, 89);
        let mut app = laid_out_app();
        app.loaded_config.config.tiles.widen_summary = true;
        let agents = (0..5)
            .map(|index| AgentRow {
                role_fallback: RoleFallback::None,
                pid: 1_579_022 + index,
                name: format!("agent {index}"),
                directory: format!(
                    "~/rust/cargo-liner-handler-enh/{}",
                    "long-directory/".repeat(14)
                ),
                children: Vec::new(),
                ..boss(21 * 60 * 60)
            })
            .collect();
        let (agents, edges) = production_roots(agents);
        app.census.apply(CensusUpdate::Local {
            rows:      agents,
            hierarchy: edges.into(),
            stage:     ScanStage::Classified,
        });
        drawn_buffer_in(&mut app, body);
        app.tiles.settle_for_test();
        let buffer = drawn_buffer_in(&mut app, body);

        let (left, top) = (body.top()..body.bottom())
            .find_map(|y| {
                (body.left()..body.right()).find_map(|x| {
                    let corner: String = (x..(x + 9).min(body.right()))
                        .map(|column| buffer[(column, y)].symbol())
                        .collect();
                    (corner == "┌ summary").then_some((x, y))
                })
            })
            .expect("summary's top-left corner is drawn");
        let right = (left + 1..body.right())
            .find(|&x| matches!(buffer[(x, top)].symbol(), "┐" | "┬"))
            .expect("summary's top-right corner is drawn");
        let bottom = (top + 1..body.bottom())
            .find(|&y| matches!(buffer[(left, y)].symbol(), "├" | "└"))
            .expect("summary's bottom border is drawn");
        assert!(
            (left + 1..right).any(|x| buffer[(x, bottom)].symbol() == "┬"),
            "summary spans a column line that continues below it"
        );
        for y in top + 1..bottom {
            for x in left + 1..right {
                assert!(
                    !buffer[(x, y)]
                        .symbol()
                        .chars()
                        .any(|glyph| ('\u{2500}'..='\u{257f}').contains(&glyph)),
                    "summary has a lattice glyph at ({x}, {y})"
                );
            }
        }
    }

    /// The real grid names each pool by machine and uses the theme's title color.
    #[test]
    fn unassigned_grid_titles_name_the_machine_and_use_theme_title_color() {
        let mut app = laid_out_app();
        app.census.apply(CensusUpdate::Local {
            rows:      vec![AgentRow {
                children: Vec::new(),
                ..boss(21 * 60 * 60)
            }],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        });
        app.loaded_config.config.machines.remote = vec!["mac".to_string()];
        app.census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(
                vec![AgentRow {
                    role_fallback: crate::census::RoleFallback::None,
                    name: "remote-session".to_string(),
                    children: Vec::new(),
                    ..boss(3600)
                }],
                HierarchyEdges::default().into(),
            ),
        });
        let body = Rect::new(0, 0, 160, 45);
        drawn_buffer_in(&mut app, body);
        app.tiles.settle_for_test();
        let buffer = drawn_buffer_in(&mut app, body);
        for machine in ["natedev", "mac"] {
            let title = format!("{UNASSIGNED_CELL_TITLE}{HEADING_SEPARATOR}{machine}");
            let mut found = false;
            for y in body.y..body.bottom() {
                let text: String = (body.x..body.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                if let Some(start) = text.find(&title) {
                    assert!(text.contains('─'), "the label is drawn in a tile frame");
                    let x =
                        u16::try_from(text[..start].chars().count()).expect("title column fits");
                    assert_eq!(buffer[(x, y)].fg, title_color());
                    assert_eq!(
                        buffer[(
                            x + u16::try_from(title.chars().count() - 1).expect("title fits"),
                            y
                        )]
                            .fg,
                        title_color()
                    );
                    found = true;
                }
            }
            assert!(found, "the grid draws the pool title for {machine}");
        }
    }

    /// A pooled session keeps the same name hue in the summary and flat list.
    #[test]
    fn an_unassigned_name_and_its_summary_row_share_a_hue() {
        let mut app = laid_out_app();
        app.census.apply(CensusUpdate::Local {
            rows:      vec![AgentRow {
                children: Vec::new(),
                ..boss(21 * 60 * 60)
            }],
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Classified,
        });
        drawn_buffer(&mut app);
        app.tiles.settle_for_test();
        let buffer = drawn_buffer(&mut app);
        let mut colors = Vec::new();
        for y in 0..buffer.area.height {
            let text: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            if let Some(start) = text.find("boss of bosses") {
                let x = u16::try_from(text[..start].chars().count()).expect("name column fits");
                colors.push(buffer[(x, y)].fg);
            }
        }
        assert_eq!(colors, [Color::Rgb(255, 95, 95); 1]);
    }

    /// Each agent's cell asks for the rows it takes at the width its
    /// cell has, and an agent whose cell is not open yet for the rows it
    /// takes at the narrowest width: the width its cell will open at.
    #[test]
    fn an_agent_cell_asks_for_its_rows_at_its_own_width() {
        let mut natedev = MachineState::Answered(
            vec![
                boss(21 * 60 * 60),
                AgentRow {
                    role_fallback: crate::census::RoleFallback::None,
                    pid: 428_044,
                    name: "enh/handler".to_string(),
                    directory: "~/rust/handler".to_string(),
                    children: Vec::new(),
                    ..boss(20 * 60 * 60)
                },
            ],
            crate::census::HierarchyEdges::default().into(),
        );
        if let MachineState::Answered(rows, edges) = &mut natedev {
            (*rows, edges.edges) = production_roots(std::mem::take(rows));
        }
        let footer_colors = FooterColors::default();
        let cells = Cells::new(
            vec![Machine {
                name:  "natedev",
                state: &natedev,
            }],
            NOW,
            &footer_colors,
            &HashMap::new(),
            SummarySort::default(),
        );
        let [boss, handler] = [&cells.agents[0], &cells.agents[1]];
        let narrow = 30;

        let demands = cells.demands(&[
            (TileContent::Summary, WIDTH - 2),
            (TileContent::Group(TileKey::Agent(boss.id.clone())), narrow),
        ]);

        let rows: Vec<usize> = demands.groups.iter().map(|group| group.rows).collect();
        assert_eq!(
            rows,
            [
                agent_cell::height(boss, narrow, NOW, AccountColumn::Hidden),
                agent_cell::height(handler, narrow, NOW, AccountColumn::Hidden),
            ]
        );
        assert!(
            rows[0] < agent_cell::height(boss, 160, NOW, AccountColumn::Hidden),
            "boss compresses narrow and asks for fewer rows than wide"
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

#[cfg(test)]
#[path = "render/accounts_grid_tests.rs"]
mod accounts_grid_tests;

#[cfg(test)]
#[path = "render/role_title_tests.rs"]
mod role_title_tests;
