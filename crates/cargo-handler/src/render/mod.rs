//! Frame rendering: the tile grid, the framework status line along the
//! bottom, and whichever overlay is open above them.

use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(test)]
use std::num::NonZeroU16;
use std::rc::Rc;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tui_pane::AdmissibleCell;
use tui_pane::AttractGridPresentation;
use tui_pane::AttractWork;
use tui_pane::BarPalette;
use tui_pane::CellClick;
use tui_pane::CellPoint;
use tui_pane::FocusLocation;
use tui_pane::FrameLayoutPreparation;
use tui_pane::FramePlacement;
use tui_pane::Keymap;
use tui_pane::NavAction;
use tui_pane::NotAdmissible;
use tui_pane::PaneFocusState;
use tui_pane::ScanIndicator;
use tui_pane::ScrollDirection;
use tui_pane::StatusLine;
use tui_pane::StatusLineGlobal;
use tui_pane::StatusLineNote;
#[cfg(test)]
use tui_pane::SummaryDemand;
use tui_pane::SummaryRowAllocation;
use tui_pane::TileCells;
use tui_pane::TileDemand;
use tui_pane::TileGridContents;
use tui_pane::TileWidthFloor;
use tui_pane::Updates;
use tui_pane::draw_attract_layers;
use tui_pane::label_color;
use tui_pane::render_status_line;
use tui_pane::title_color;

use crate::agent_cell;
use crate::agent_cell::AgentEntry;
use crate::agent_cell::AgentTile;
use crate::agent_cell::TreeColumn;
use crate::agent_cell::TreeMeasurement;
use crate::agent_cell::TreeMetrics;
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
use crate::footer_colors::FooterTint;
use crate::globals::AppGlobalAction;
use crate::interaction::RowLocation;
use crate::interaction::SelectableSurface;
use crate::interaction::SelectedSpan;
use crate::interaction::SelectionSurface;
use crate::interaction::Selections;
use crate::interaction::SurfaceLookup;
use crate::interaction::TileSelection;
use crate::interaction::ViewportLine;
use crate::interaction::ViewportMapping;
use crate::settings;
use crate::sort;
use crate::sort::AccountsColumn;
use crate::sort::AccountsSort;
use crate::sort::HeaderHit;
use crate::sort::HeaderSpan;
use crate::sort::SummaryColumn;
use crate::sort::SummarySort;
use crate::sort::TreeSort;
use crate::summary;
use crate::summary::PaintedSummary;
use crate::summary::SummaryFrame;
use crate::summary::SummaryLayout;
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

/// Paint the production grid with one controlled frame clock.
#[cfg(test)]
pub(crate) fn draw_at_for_test(frame: &mut Frame, app: &mut App, keymap: &Keymap<App>, now: u64) {
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
            .areas(frame.area());
    draw_panes_at(frame, app, body, TileGridContents::Shown, now);
    draw_status_line(frame, app, keymap, status);
}

/// Draw the tile grid into the body above the status line.
///
/// [`tui_pane::draw_tile_grid`] decides where each cell goes, how far
/// through a transition it is, and draws the frames, the numbers of the
/// empty cells and every cell's readout; [`Cells`] is what goes inside
/// the summary and each agent's cell.
fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect, contents: TileGridContents) {
    draw_panes_at(frame, app, area, contents, census::unix_now());
}

fn draw_panes_at(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    contents: TileGridContents,
    now: u64,
) {
    app.tiles
        .set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
    let growth = app.loaded_config.config.tiles.growth();
    app.selection_area = area;
    let cells = Cells::new(
        app.census
            .machines(&app.loaded_config.config.machines.remote),
        now,
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
    )
    .with_accounts(&app.accounts.table)
    .with_accounts_sort(app.accounts_sort)
    .with_selections(app.selections.clone(), SelectableSurface::EmptyCell);
    let minimum = cells.measure_trees(area);
    app.tiles.set_min_tile_width(minimum);
    app.tiles
        .set_tile_width_floor(TileWidthFloor::CompressSingleColumn);
    let focused = app
        .tiles
        .placements(area, growth)
        .into_iter()
        .find(|placement| placement.frame.is_focused())
        .map_or(SelectableSurface::EmptyCell, |placement| {
            selection_surface(&placement.content)
        });
    let followed = match &focused {
        SelectableSurface::EmptyCell => TileSelection::NoRows,
        SelectableSurface::Surface(surface) => app.selections.surface(surface).map_or(
            TileSelection::NoRows,
            super::interaction::SurfaceSelection::selection,
        ),
    };
    *cells.focused.borrow_mut() = focused.clone();
    app.tiles.set_view(app.loaded_config.config.tiles.view);
    tui_pane::draw_tile_grid(
        frame.buffer_mut(),
        &mut app.tiles,
        area,
        growth,
        contents,
        &cells,
    );
    *app.painted_summary.get_mut() = cells.painted_summary.into_inner();
    app.tree_layouts = cells.layouts.into_inner();
    app.selections = cells.selections.into_inner();
    if let TileSelection::Row(identity) = followed
        && let RowLocation::Found(destination) = app.selections.containing(&identity)
        && focused != SelectableSurface::Surface(destination.clone())
        && let Some(placement) = app
            .tiles
            .placements(area, growth)
            .into_iter()
            .find(|placement| {
                selection_surface(&placement.content)
                    == SelectableSurface::Surface(destination.clone())
            })
    {
        let rect = placement.frame.rect();
        if let Some(index) = app.tiles.cell_at(ratatui::layout::Position::new(
            rect.x + rect.width / 2,
            rect.y + rect.height / 2,
        )) {
            app.tiles.focus_cell(index);
        }
    }
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
    let header_area = Rect::new(0, 0, area.width, area.height);
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
    for tile in &cells.tiles {
        if let Some(metrics) = app.tree_layouts.get(&tile.key()) {
            tile.share_metrics(metrics.clone());
        }
    }
    match content {
        TileContent::Summary => match summary_header_hit(app, header_area, point) {
            SummaryHeaderHit::Missed => {},
            SummaryHeaderHit::AgentColumn(column) => {
                app.summary_sort.select(column);
                return;
            },
            SummaryHeaderHit::AccountColumn(column) => {
                app.accounts_sort.select(column);
                return;
            },
        },
        TileContent::Group(ref id) => {
            if let Some(tile) = cells.tile(id)
                && let HeaderHit::Column(column) =
                    sort::header_hit(&cells.header_spans(tile, header_area), point)
            {
                let default = tile.default_sort();
                app.tree_sorts
                    .entry(id.clone())
                    .or_insert(default)
                    .select(column);
                return;
            }
        },
        TileContent::Empty(_) => return,
    }
    if let SelectableSurface::Surface(surface) = selection_surface(&content)
        && let SurfaceLookup::Current(state) = app.selections.surface_mut(&surface)
    {
        state.click(ratatui::layout::Position::new(
            area.x.saturating_add(point.column),
            area.y.saturating_add(point.row),
        ));
    }
}

/// The two independent tables share one painted summary surface.
enum SummaryHeaderHit {
    Missed,
    AgentColumn(SummaryColumn),
    AccountColumn(AccountsColumn),
}

/// Header-only summary draws retain their layout without a selectable row surface.
fn summary_header_hit(app: &App, area: Rect, point: CellPoint) -> SummaryHeaderHit {
    let painted = app.painted_summary.borrow();
    let PaintedSummary::Drawn(layout) = &*painted else {
        return SummaryHeaderHit::Missed;
    };
    let source = match app.selections.surface(&SelectionSurface::Summary) {
        SurfaceLookup::Current(state) => {
            state.painted_mapping().source_line(usize::from(point.row))
        },
        SurfaceLookup::Retired => ViewportMapping::Linear {
            offset: 0,
            height: usize::from(area.height),
        }
        .source_line(usize::from(point.row)),
    };
    let row = match source {
        ViewportLine::Header(row) | ViewportLine::Body(row) => row,
        ViewportLine::Outside => return SummaryHeaderHit::Missed,
    };
    let point = CellPoint {
        row: u16::try_from(row).unwrap_or(u16::MAX),
        ..point
    };
    let area = Rect {
        height: u16::try_from(layout.height()).unwrap_or(u16::MAX),
        ..area
    };
    if let HeaderHit::Column(column) = sort::header_hit(&layout.header_spans(area), point) {
        SummaryHeaderHit::AgentColumn(column)
    } else if let HeaderHit::Column(column) =
        sort::header_hit(&layout.accounts_header_spans(area), point)
    {
        SummaryHeaderHit::AccountColumn(column)
    } else {
        SummaryHeaderHit::Missed
    }
}

fn selection_surface(content: &TileContent) -> SelectableSurface {
    match content {
        TileContent::Summary => SelectableSurface::Surface(SelectionSurface::Summary),
        TileContent::Group(key) => SelectableSurface::Surface(SelectionSurface::Tile(key.clone())),
        TileContent::Empty(_) => SelectableSurface::EmptyCell,
    }
}

pub(crate) fn navigate_selection(app: &mut App, action: NavAction) {
    let FocusLocation::Cell(_) = app.tiles.focused_cell() else {
        return;
    };
    let growth = app.loaded_config.config.tiles.growth();
    if let SelectableSurface::Surface(surface) = app
        .tiles
        .placements(app.selection_area, growth)
        .into_iter()
        .find(|placement| placement.frame.is_focused())
        .map_or(SelectableSurface::EmptyCell, |placement| {
            selection_surface(&placement.content)
        })
        && let SurfaceLookup::Current(state) = app.selections.surface_mut(&surface)
    {
        state.navigate(action);
    }
}

pub(crate) fn cell_scrolled(app: &mut App, cell: &CellClick<TileKey>, direction: ScrollDirection) {
    if let SelectableSurface::Surface(surface) = selection_surface(&cell.content)
        && let SurfaceLookup::Current(state) = app.selections.surface_mut(&surface)
    {
        state.scroll(direction);
    }
}

/// Only the current allocation is retained while the grid resolves this frame.
#[derive(Default)]
enum FrameSummaryLayout {
    #[default]
    Unallocated,
    Allocated(Rc<SummaryLayout>),
    WithAlternative {
        allocated:   Rc<SummaryLayout>,
        alternative: Rc<SummaryLayout>,
    },
}

/// Direct drawing does not create selection state; a prepared frame owns its row layout.
#[derive(Default)]
enum SummarySelectionLayout {
    #[default]
    DirectDraw,
    Prepared(Rc<SummaryLayout>),
}

/// The per-session summary source and the production-role or pooled tiles.
struct Cells<'a> {
    summary_layout:    RefCell<FrameSummaryLayout>,
    summary_selection: RefCell<SummarySelectionLayout>,
    painted_summary:   RefCell<PaintedSummary>,
    selections:        RefCell<Selections>,
    focused:           RefCell<SelectableSurface>,
    layouts:           RefCell<HashMap<TileKey, TreeMetrics>>,
    measurements:      RefCell<HashMap<TileKey, TreeMeasurement>>,
    /// This machine, then each configured remote.
    machines:          Vec<Machine<'a>>,
    /// Individual sessions in default hue order, independent of tile grouping.
    agents:            Vec<AgentEntry<'a>>,
    /// The account columns and agent labels shared by every cell.
    account_column:    AccountColumn,
    /// The forest shared by cell order and summary roots.
    placement:         AgentPlacement<'a>,
    /// Grouped tiles, apart from the per-session summary hue lookup.
    tiles:             Vec<AgentTile<'a>>,
    /// The unix second the ages are measured to.
    now:               u64,
    /// Footer colors already loaded by the app.
    footer_colors:     &'a FooterColors,
    /// Ordering shared by all machine groups in the summary.
    summary_sort:      SummarySort,
    /// Account readings share the summary cell, independent of agent placement.
    accounts:          &'a AccountsTable,
    /// Source order until an accounts header is selected.
    accounts_sort:     AccountsSort,
}

/// Render fixed content allocations through the production preparation path.
#[cfg(test)]
pub(crate) fn draw_allocated_tiles_for_test(
    machines: Vec<Machine<'_>>,
    now: u64,
    allocations: &[(TileKey, Rect)],
    buffer: &mut Buffer,
) {
    let colors = FooterColors::default();
    let cells = Cells::new(
        machines,
        now,
        &colors,
        &HashMap::new(),
        SummarySort::default(),
    );
    cells.prepare_frame(|| {
        allocations
            .iter()
            .map(|(key, area)| (TileContent::Group(key.clone()), *area))
            .collect()
    });
    for (key, area) in allocations {
        cells.draw(
            buffer,
            &TileContent::Group(key.clone()),
            *area,
            Color::Reset,
        );
    }
}

impl<'a> Cells<'a> {
    /// Set each `SurfaceSelection` area and tile viewport from the painted allocations.
    fn prepare_selection_areas(&self, allocations: &[(TileContent, Rect)]) {
        let mut selections = self.selections.borrow_mut();
        for (content, area) in allocations {
            if let SelectableSurface::Surface(surface) = selection_surface(content)
                && let SurfaceLookup::Current(state) = selections.surface_mut(&surface)
            {
                match content {
                    TileContent::Group(key) => {
                        if let Some(tile) = self.tile(key) {
                            state.set_area(tile.row_area(*area, self.now, self.account_column));
                            tile.set_viewport(state.viewport());
                        }
                    },
                    TileContent::Summary => state.set_area(*area),
                    TileContent::Empty(_) => {},
                }
            }
        }
    }

    /// Source widths are retained for allocation fitting in this frame.
    fn measure_trees(&self, area: Rect) -> u16 {
        let mut measurements = self.measurements.borrow_mut();
        for tile in &self.tiles {
            measurements
                .entry(tile.key())
                .or_insert_with(|| tile.measure(area, self.now, self.account_column));
        }
        // Any tile can receive the full grid height less its frame. Project the
        // saved cursor into that envelope through the allocation path itself.
        // This can reserve width for rows a shorter allocation does not paint.
        let tallest = tui_pane::frame_inner(area);
        let selections = self.selections.borrow();
        let unselected = crate::interaction::SurfaceSelection::default();
        measurements
            .iter()
            .map(|(key, measurement)| {
                let selection = selections
                    .surface(&SelectionSurface::Tile(key.clone()))
                    .map_or(&unselected, |state| state);
                measurement
                    .allocated(tallest, selection)
                    .minimum_tile_width()
            })
            .max()
            .unwrap_or(0)
    }

    /// Paint the selected rows after their tile content.
    fn paint_selection(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect) {
        if let SelectableSurface::Surface(surface) = selection_surface(content) {
            let selections = self.selections.borrow();
            if let SurfaceLookup::Current(state) = selections.surface(&surface)
                && !matches!(state.selection(), TileSelection::NoRows)
                && let SelectedSpan::Row(span) = state.selected_span()
            {
                let area = match content {
                    TileContent::Group(key) => self.tile(key).map_or(inner, |tile| {
                        tile.row_area(inner, self.now, self.account_column)
                    }),
                    _ => inner,
                };
                for visible in span.visible_lines(state.painted_mapping()) {
                    let focus =
                        if *self.focused.borrow() == SelectableSurface::Surface(surface.clone()) {
                            PaneFocusState::Active
                        } else {
                            PaneFocusState::Remembered
                        };
                    let style = tui_pane::selection_style(focus);
                    let row = Rect::new(
                        area.x,
                        area.y + u16::try_from(visible).unwrap_or(u16::MAX),
                        area.width,
                        1,
                    )
                    .intersection(area)
                    .intersection(buffer.area);
                    buffer.set_style(row, style);
                    if let Some(ground) = style.bg {
                        for position in row.positions() {
                            let cell = &mut buffer[position];
                            cell.fg =
                                crate::footer_colors::SelectedGroundInk::resolve(cell.fg, ground)
                                    .foreground();
                        }
                    }
                }
            }
        }
    }

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
            summary_layout: RefCell::new(FrameSummaryLayout::Unallocated),
            summary_selection: RefCell::new(SummarySelectionLayout::DirectDraw),
            painted_summary: RefCell::new(PaintedSummary::NotDrawn),
            selections: RefCell::new(Selections::default()),
            focused: RefCell::new(SelectableSurface::EmptyCell),
            layouts: RefCell::new(HashMap::new()),
            measurements: RefCell::new(HashMap::new()),
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

    fn with_selections(mut self, selections: Selections, focused: SelectableSurface) -> Self {
        self.selections = RefCell::new(selections);
        self.focused = RefCell::new(focused);
        self.restore_tree_viewports();
        self
    }

    /// Restore navigation before source measurement or allocation fitting reads it.
    fn restore_tree_viewports(&self) {
        let selections = self.selections.borrow();
        for tile in &self.tiles {
            if let SurfaceLookup::Current(state) =
                selections.surface(&SelectionSurface::Tile(tile.key()))
            {
                tile.set_viewport(state.viewport());
            }
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

    /// Grid demand can propose a new width; all readers of that allocation share its lines.
    fn summary_layout(&self, width: u16) -> Rc<SummaryLayout> {
        let mut current = self.summary_layout.borrow_mut();
        match &*current {
            FrameSummaryLayout::Allocated(layout) if layout.allocation_width() == width => {
                return Rc::clone(layout);
            },
            FrameSummaryLayout::WithAlternative { allocated, .. }
                if allocated.allocation_width() == width =>
            {
                return Rc::clone(allocated);
            },
            FrameSummaryLayout::WithAlternative { alternative, .. }
                if alternative.allocation_width() == width =>
            {
                return Rc::clone(alternative);
            },
            _ => {},
        }
        let layout = Rc::new(summary::SummaryLayout::new(&self.summary_frame(), width));
        *current = match &*current {
            FrameSummaryLayout::Unallocated => FrameSummaryLayout::Allocated(Rc::clone(&layout)),
            FrameSummaryLayout::Allocated(allocated)
            | FrameSummaryLayout::WithAlternative { allocated, .. } => {
                FrameSummaryLayout::WithAlternative {
                    allocated:   Rc::clone(allocated),
                    alternative: Rc::clone(&layout),
                }
            },
        };
        layout
    }

    /// Motion may paint a narrower allocation than its metric bounds.
    fn resolve_selection_rows(&self, layout: &SummaryLayout) {
        let summary_rows = summary::selection_rows(&self.summary_frame(), layout);
        let mut rows: Vec<_> = self
            .tiles
            .iter()
            .map(|tile| {
                let rows = tile
                    .row_identities()
                    .into_iter()
                    .enumerate()
                    .map(|(line, identity)| (identity, crate::interaction::RowSpan::single(line)))
                    .collect();
                (SelectionSurface::Tile(tile.key()), rows)
            })
            .collect();
        rows.push((SelectionSurface::Summary, summary_rows));
        let mut selections = self.selections.borrow_mut();
        let followed = match &*self.focused.borrow() {
            SelectableSurface::EmptyCell => TileSelection::NoRows,
            SelectableSurface::Surface(surface) => selections.surface(surface).map_or(
                TileSelection::NoRows,
                super::interaction::SurfaceSelection::selection,
            ),
        };
        selections.resolve(rows);
        if let SurfaceLookup::Current(state) = selections.surface_mut(&SelectionSurface::Summary) {
            state.set_pin(layout.section_pin());
        }
        if let TileSelection::Row(identity) = followed
            && let RowLocation::Found(destination) = selections.follow_focused(&identity)
        {
            *self.focused.borrow_mut() = SelectableSurface::Surface(destination);
        }
        drop(selections);
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

#[derive(Debug)]
struct ObservedAgentTile;

impl TileCells<TileKey> for Cells<'_> {
    fn admission(&self, id: &TileKey) -> Result<AdmissibleCell<TileKey>, NotAdmissible<TileKey>> {
        self.tile(id)
            .map(|tile| AdmissibleCell::new(tile.key(), std::iter::empty(), ObservedAgentTile))
            .ok_or_else(|| NotAdmissible::MissingEvidence(id.clone()))
    }

    fn prepare_moving_frame(
        &self,
        _: impl FnOnce() -> Vec<(TileContent, Rect)>,
        motion_bounds: impl FnOnce() -> Vec<(TileContent, Rect)>,
    ) {
        self.prepare_frame(motion_bounds);
    }

    fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent, Rect)>) {
        let allocations = allocations();
        let summary_width = allocations
            .iter()
            .find_map(|(content, area)| {
                matches!(content, TileContent::Summary).then_some(area.width)
            })
            .unwrap_or(0);
        let layout = self.summary_layout(summary_width);
        self.resolve_selection_rows(&layout);
        *self.summary_selection.borrow_mut() = SummarySelectionLayout::Prepared(layout);

        let tiles: Vec<_> = allocations
            .iter()
            .filter_map(|(content, area)| match content {
                TileContent::Group(id) => self.tile(id).map(|tile| (tile, *area)),
                _ => None,
            })
            .collect();
        self.restore_tree_viewports();
        self.measure_trees(Rect::new(0, 0, u16::MAX, u16::MAX));
        let source = self.measurements.borrow();
        let measurements =
            agent_cell::shared_tree_metrics(tiles.iter().filter_map(|(tile, area)| {
                let selections = self.selections.borrow();
                selections
                    .surface(&SelectionSurface::Tile(tile.key()))
                    .map_or(None, |state| {
                        source
                            .get(&tile.key())
                            .map(|source| source.allocated(*area, state))
                    })
            }));
        let mut layouts = self.layouts.borrow_mut();
        layouts.clear();
        for ((tile, _), metrics) in tiles.into_iter().zip(measurements) {
            tile.share_metrics(metrics.clone());
            layouts.insert(tile.key(), metrics);
        }
        self.prepare_selection_areas(&allocations);
    }

    fn prepare_painted_frame(
        &self,
        _: &FramePlacement<TileKey>,
        allocations: impl FnOnce() -> Vec<(TileContent, Rect)>,
    ) -> FrameLayoutPreparation {
        self.prepare_selection_areas(&allocations());
        FrameLayoutPreparation::Ready
    }

    fn summary_title(&self) -> &str { SUMMARY_CELL_TITLE }

    /// Measure each tile at its allocated content width. Account rows choose
    /// their full, two-line or stacked layout from the summary's width.
    /// Cells absent from `widths` use the narrowest supplied width, or zero.
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
        let summary_width = widths
            .iter()
            .find(|(content, _)| matches!(content, TileContent::Summary))
            .map_or(narrowest, |&(_, width)| width);
        let layout = self.summary_layout(summary_width);
        TileDemands {
            summary:       layout.demand(),
            summary_width: layout.preferred_width(&self.summary_frame()),
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

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, inner: Rect, ground: Color) {
        match content {
            TileContent::Summary => {
                let layout = self.summary_layout(inner.width);
                let selection_prepared = match &*self.summary_selection.borrow() {
                    SummarySelectionLayout::DirectDraw => false,
                    SummarySelectionLayout::Prepared(previous) => {
                        if !Rc::ptr_eq(previous, &layout) {
                            self.resolve_selection_rows(&layout);
                        }
                        true
                    },
                };
                if selection_prepared {
                    *self.summary_selection.borrow_mut() =
                        SummarySelectionLayout::Prepared(Rc::clone(&layout));
                    if let SurfaceLookup::Current(state) = self
                        .selections
                        .borrow_mut()
                        .surface_mut(&SelectionSurface::Summary)
                    {
                        state.set_pin(layout.section_pin());
                        state.set_area(inner);
                    }
                }
                let selections = self.selections.borrow();
                let mapping = selections.surface(&SelectionSurface::Summary).map_or(
                    ViewportMapping::Linear {
                        offset: 0,
                        height: usize::from(inner.height),
                    },
                    |state| state.painted_mapping().clone(),
                );
                let height = layout.height();
                let virtual_area = Rect::new(
                    inner.x,
                    0,
                    inner.width,
                    u16::try_from(height).unwrap_or(u16::MAX),
                );
                let mut complete = Buffer::empty(virtual_area);
                complete.set_style(virtual_area, Style::default().bg(ground));
                summary::draw_measured(&mut complete, virtual_area, &self.summary_frame(), &layout);
                *self.painted_summary.borrow_mut() = PaintedSummary::Drawn(layout);
                for y in 0..inner.height {
                    let source = match mapping.source_line(usize::from(y)) {
                        ViewportLine::Header(source) | ViewportLine::Body(source) => source,
                        ViewportLine::Outside => continue,
                    };
                    if source >= usize::from(virtual_area.height) {
                        continue;
                    }
                    for x in inner.x..inner.right() {
                        buffer[(x, inner.y + y)] =
                            complete[(x, u16::try_from(source).unwrap_or(u16::MAX))].clone();
                    }
                }
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
        self.paint_selection(buffer, content, inner);
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
                    Role::Session(entry.hue).style(),
                ),
                Span::styled(
                    entry.role_label.suffix(),
                    match entry.role_label {
                        RoleLabel::Stated(role) | RoleLabel::Inferred(role) => {
                            self.footer_colors.hierarchy_role(role)
                        },
                        RoleLabel::ReviewFallback | RoleLabel::Unlabeled => FooterTint::Plain,
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
    use crate::census::WorkIdentity;
    use crate::census::hierarchy::ReviewerState;
    use crate::census::transcript::SubagentRole;
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
        "│        │   Focus the tile above                                    │         │",
        "│        │   Focus the tile below                                    │         │",
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
        "│              │   Focus the tile above                        │               │",
        "│              │   Focus the tile below                        │               │",
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
        "│              │   Select one page above             pageup    │               │",
        "│              │   Select one page below             pagedown  │ 1  r/c: 21/78 │",
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
        "│ session ▲       status  directory                                        age │",
        "│ boss of bosses  idle    ~/rust/hana_catalyst/docs/hana                   21h │",
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
            work_id: WorkIdentity::Legacy,
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
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
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
                    ChildKind::Subagent(SubagentRole::Unrecorded),
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

    /// Resolve the role header in the current frame after each sort selection.
    fn role_header_click(app: &App) -> CellClick<TileKey> {
        let area = Rect::new(0, 0, 220, 30);
        let (id, point) = {
            let cells = Cells::new(
                app.census.machines(&[]),
                census::unix_now(),
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
        CellClick {
            index: 2,
            content: TileContent::Group(id),
            area: Rect::new(8, 5, area.width, area.height),
            point,
        }
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
        let mut click = role_header_click(&app);
        cell_clicked(&mut app, click.clone());
        assert_eq!(app.tree_sorts.len(), 1, "only the clicked tile gets a sort");
        let id = app
            .tree_sorts
            .keys()
            .next()
            .cloned()
            .expect("the clicked tile has a saved sort");
        assert_eq!(
            app.tree_sorts[&id],
            TreeSort {
                column:    TreeColumn::Role,
                direction: SortDirection::Ascending,
            }
        );
        assert_eq!(app.summary_sort, SummarySort::default());
        click = role_header_click(&app);
        cell_clicked(&mut app, click.clone());
        assert_eq!(app.tree_sorts[&id].direction, SortDirection::Descending);
        let selected = app.tree_sorts[&id];
        let point = click.point;
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
                Role::Session(crate::theme::SessionHue::First)
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
            rows:      vec![
                AgentRow {
                    children: Vec::new(),
                    ..boss(21 * 60 * 60)
                },
                AgentRow {
                    pid: 1_579_023,
                    name: "a selected neighbor".to_string(),
                    children: Vec::new(),
                    ..boss(20 * 60 * 60)
                },
            ],
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
        assert_eq!(
            colors,
            [Role::Session(crate::theme::SessionHue::First)
                .style()
                .fg
                .expect("the session name has a hue"); 1]
        );
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
#[path = "accounts_grid_tests.rs"]
mod accounts_grid_tests;

#[cfg(test)]
#[path = "role_title_tests.rs"]
mod role_title_tests;

#[cfg(test)]
#[path = "moving_role_tests.rs"]
mod moving_role_tests;

#[cfg(test)]
mod selected_ink_tests;
#[cfg(test)]
mod selection_tests;

#[cfg(test)]
mod status_bar_contrast_tests;

#[cfg(test)]
mod pinned_accounts_tests;

#[cfg(test)]
mod narrow_width_tests;
