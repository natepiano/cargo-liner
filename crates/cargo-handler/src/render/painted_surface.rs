//! `PaintedSurface` keeps the input mapping for one painted source allocation.

use ratatui::layout::Rect;
use tui_pane::AttractGridPresentation;
use tui_pane::CellPoint;
use tui_pane::CellPointer;

use super::Cells;
use super::TileLookup;
use super::selection_surface;
use crate::agent_cell::TreeColumn;
use crate::app::App;
use crate::app::expansion;
use crate::app::expansion::HandlerExpansionAction;
use crate::census;
use crate::interaction::RowClick;
use crate::interaction::SelectableSurface;
use crate::interaction::SelectionSurface;
use crate::interaction::SurfaceLookup;
use crate::interaction::SurfaceSelection;
use crate::interaction::ViewportLine;
use crate::sort;
use crate::sort::AccountsColumn;
use crate::sort::HeaderHit;
use crate::sort::HeaderSpan;
use crate::sort::SummaryColumn;
use crate::sort::TreeSort;
use crate::tiles::TileKey;

/// `PaintedSurface` retains the rows and headers used by its source allocation.
#[derive(Clone)]
pub(crate) struct PaintedSurface {
    pub(super) area:    Rect,
    pub(super) rows:    SurfaceSelection,
    pub(super) content: PaintedSurfaceContent,
}

/// `PaintedSurfaceContent` binds each surface identity to its header domain.
#[derive(Clone)]
pub(super) enum PaintedSurfaceContent {
    Tree {
        key:     TileKey,
        spans:   Vec<HeaderSpan<TreeColumn>>,
        default: TreeSort,
    },
    Summary {
        agents:   Vec<HeaderSpan<SummaryColumn>>,
        accounts: Vec<HeaderSpan<AccountsColumn>>,
    },
}

impl PaintedSurface {
    pub(super) fn surface(&self) -> SelectionSurface {
        match &self.content {
            PaintedSurfaceContent::Tree { key, .. } => SelectionSurface::Tile(key.clone()),
            PaintedSurfaceContent::Summary { .. } => SelectionSurface::Summary,
        }
    }
}

/// `cell_clicked` maps a press through the last painted headers and rows.
pub(crate) fn cell_clicked(app: &mut App, click: CellPointer<TileKey>) {
    match app.attract.configuration().grid_presentation() {
        AttractGridPresentation::ReplacesGrid => return,
        AttractGridPresentation::OverGrid => {},
    }
    let CellPointer {
        content,
        area,
        point,
        ..
    } = click;
    let SelectableSurface::Surface(surface) = selection_surface(&content) else {
        return;
    };
    let Some(painted) = app
        .painted_surfaces
        .iter()
        .rev()
        .find(|painted| painted.surface() == surface && painted.area == area)
        .cloned()
    else {
        return;
    };
    match &painted.content {
        PaintedSurfaceContent::Tree {
            key,
            spans,
            default,
        } => {
            tree_pressed(app, key, &painted, spans, *default, point);
        },
        PaintedSurfaceContent::Summary { agents, accounts } => {
            summary_pressed(app, &painted, agents, accounts, point);
        },
    }
}

fn tree_pressed(
    app: &mut App,
    id: &TileKey,
    painted: &PaintedSurface,
    spans: &[HeaderSpan<TreeColumn>],
    default: TreeSort,
    point: CellPoint,
) {
    let cells = Cells::new(
        app.census
            .machines(&app.loaded_config.config.machines.remote),
        census::unix_now(),
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
    )
    .with_expansion(&app.expansion);
    let TileLookup::Current(tile) = cells.tile(id) else {
        return;
    };
    if let HeaderHit::Column(column) = sort::header_hit(spans, point) {
        app.tree_sorts
            .entry(id.clone())
            .or_insert(default)
            .select(column);
        return;
    }
    let mut rows = painted.rows.clone();
    let hit = rows.click(ratatui::layout::Position::new(
        painted.area.x.saturating_add(point.column),
        painted.area.y.saturating_add(point.row),
    ));
    if let RowClick::Selected(identity) = hit
        && tile.row_identities().contains(&identity)
        && let SurfaceLookup::Current(state) = app.selections.surface_mut(&painted.surface())
    {
        state.select(&identity);
        expansion::dispatch(app, HandlerExpansionAction::Press(identity));
    }
}

fn summary_pressed(
    app: &mut App,
    painted: &PaintedSurface,
    agents: &[HeaderSpan<SummaryColumn>],
    accounts: &[HeaderSpan<AccountsColumn>],
    point: CellPoint,
) {
    let source = painted
        .rows
        .painted_mapping()
        .source_line(usize::from(point.row));
    let row = match source {
        ViewportLine::Header(row) | ViewportLine::Body(row) => row,
        ViewportLine::Outside => return,
    };
    let source_point = CellPoint {
        row: u16::try_from(row).unwrap_or(u16::MAX),
        ..point
    };
    if let HeaderHit::Column(column) = sort::header_hit(agents, source_point) {
        app.summary_sort.select(column);
        return;
    }
    if let HeaderHit::Column(column) = sort::header_hit(accounts, source_point) {
        app.accounts_sort.select(column);
        return;
    }
    let mut rows = painted.rows.clone();
    if let RowClick::Selected(identity) = rows.click(ratatui::layout::Position::new(
        painted.area.x.saturating_add(point.column),
        painted.area.y.saturating_add(point.row),
    )) && let SurfaceLookup::Current(state) = app.selections.surface_mut(&painted.surface())
    {
        state.select(&identity);
    }
}
