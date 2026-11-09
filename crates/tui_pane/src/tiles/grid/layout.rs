use std::num::NonZeroU16;

use super::CellRowDemand;
use super::ColumnFocus;
use super::ColumnHead;
use super::HeldCellLayout;
use super::Layout;
use super::PaneAxisSize;
use super::Rect;
use super::ResolvedPane;
use super::ResolvedPaneLayout;
use super::Slot;
use super::SummaryRowAllocation;
use super::TABLE_CELL;
use super::TileContent;
use super::TileGrowth;
use super::TileSettings;
use super::TileWidthFloor;
use super::cells;
use super::column_focus;
use super::columns;
use super::constraints_for_sizes;
use super::draw;
use super::fills;
use super::frame_inner;
use super::position;
use super::share_borders;
use super::shared_run;
use super::shares;
use super::shares_with_directional_row_usage;
use super::summary_before_tiles;
/// One arrangement resolved against a rect: the rect every cell holds,
/// and the rect every column holds.
///
/// Columns are resolved one at a time rather than through
/// [`crate::PaneGridLayout`] because the grid is ragged -- its columns
/// need not hold the same number of cells -- and that type's placements
/// describe a uniform grid.
pub(super) struct Grid {
    /// The rect the whole grid fills.
    pub(super) area:     Rect,
    /// Rows in each column, left to right.
    pub(super) widths:   Vec<usize>,
    /// Positions represented by the summary, including its first.
    pub(super) depth:    usize,
    /// Where each cell sits, keyed by cell number.
    pub(super) resolved: ResolvedPaneLayout<usize>,
    /// The rect each column's cells divide: the column's whole height,
    /// or what the summary leaves below it in a column it widens over.
    pub(super) columns:  Vec<Rect>,
    /// Columns the summary covers, its own included.
    pub(super) span:     usize,
    /// Floor and allocation rules under which this endpoint was checked.
    pub(super) rules:    EndpointLayoutRules,
}

/// Geometry rules that must still hold before a cached endpoint can be reused.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct EndpointLayoutRules {
    pub(super) growth:             TileGrowth,
    pub(super) min_tile_width:     u16,
    pub(super) min_tile_height:    NonZeroU16,
    pub(super) width_floor:        TileWidthFloor,
    pub(super) summary_allocation: SummaryRowAllocation,
}

/// Admission of the held, moving and queued arrangements under one rule stamp.
pub(super) struct CheckedLayoutAdmission {
    pub(super) area:  Rect,
    pub(super) rules: EndpointLayoutRules,
    pub(super) fits:  bool,
}

impl EndpointLayoutRules {
    pub(super) const fn new(growth: TileGrowth, settings: &TileSettings) -> Self {
        Self {
            growth,
            min_tile_width: settings.min_tile_width,
            min_tile_height: settings.min_tile_height,
            width_floor: settings.tile_width_floor,
            summary_allocation: settings.summary_row_allocation,
        }
    }
}

/// Choose retained content for donors and painted content for receiving steps.
#[derive(Clone, Copy)]
pub(super) enum RowUsePurpose {
    RetainContent,
    PaintContent,
}

/// Geometry needed to turn one proposed share into its content area.
struct RowProbeLayout<'a> {
    area:         Rect,
    band:         Rect,
    first_cell:   usize,
    cells:        usize,
    opened:       &'a [Rect],
    widths:       &'a [usize],
    summary_span: usize,
    settings:     &'a TileSettings,
}

impl RowProbeLayout<'_> {
    /// The content area a row would receive at this disjoint height.
    pub(super) fn content_area(&self, row: usize, rows: u16) -> Rect {
        let last = row.saturating_add(1) == self.cells;
        let y = if last {
            self.band.bottom().saturating_sub(rows)
        } else {
            self.band.y
        };
        let mut cell = Rect {
            x: self.band.x,
            y,
            width: self.band.width,
            height: rows,
        };
        if self.first_cell.saturating_add(row) == TABLE_CELL {
            let reached = match self.settings.summary_row_allocation {
                SummaryRowAllocation::Shared => reach(
                    self.opened,
                    self.widths,
                    cell,
                    self.summary_span,
                    self.settings,
                ),
                SummaryRowAllocation::BeforeTiles => self.summary_span,
            };
            cell = widened(cell, self.opened.get(reached.saturating_sub(1)).copied());
        }
        draw::content_area(frame_inner(share_borders(cell, self.area)))
    }
}

/// A checked arrangement, or the floor that prevents it from being drawn.
pub(super) enum ColumnPlan {
    Columns(RowAdmissibleColumnLayout),
    InsufficientWidth,
    InsufficientRows,
}

/// A proposed distribution of logical positions, before its row budget is checked.
pub(super) struct ColumnLayout {
    pub(super) widths:        Vec<usize>,
    pub(super) summary_depth: usize,
    pub(super) summary_span:  usize,
}

/// Every physical column can hold its residents at the configured height floor.
pub(super) struct RowAdmissibleColumnLayout {
    pub(super) columns: ColumnLayout,
}

impl ColumnLayout {
    /// Count the summary once, however many logical positions it occupies.
    pub(super) fn residents(&self, column: usize) -> usize {
        if column == 0 {
            self.widths[0] - self.summary_depth + 1
        } else {
            self.widths[column]
        }
    }

    /// The only boundary that admits a proposed column layout for drawing.
    fn checked(self, area: Rect, settings: &TileSettings, summary_rows: u16) -> ColumnPlan {
        let opened =
            Layout::horizontal(constraints_for_sizes(&fills(self.widths.len()))).split(area);
        if area.width == 0
            || opened.iter().any(|&column| {
                share_borders(column, area).width < settings.min_tile_width
                    && !(self.widths.len() == 1
                        && settings.tile_width_floor == TileWidthFloor::CompressSingleColumn)
            })
        {
            return ColumnPlan::InsufficientWidth;
        }
        let gap = usize::from(settings.min_tile_height.get() - 1);
        for column in 0..self.widths.len() {
            let residents = self.residents(column);
            let (height, reservation, mates) = if column == 0 {
                // Keep held growth positions and any larger content reservation.
                // The last piece owns its closing row; other pieces share it.
                let content_gap = summary_rows.saturating_sub(u16::from(residents == 1));
                (
                    area.height,
                    usize::from(content_gap).max(self.summary_depth * gap),
                    residents - 1,
                )
            } else {
                let height = if column < self.summary_span {
                    area.height.saturating_sub(summary_rows)
                } else {
                    area.height
                };
                (height, 0, residents)
            };
            let available = usize::from(height.saturating_sub(1));
            if height == 0 || reservation > available || mates > (available - reservation) / gap {
                return ColumnPlan::InsufficientRows;
            }
        }
        ColumnPlan::Columns(RowAdmissibleColumnLayout { columns: self })
    }
}

/// Measure each candidate at the widths its physical columns provide.
pub(super) fn candidate_content_widths<Id: Clone>(
    target: &[Slot<Id>],
    area: Rect,
    base: &ColumnLayout,
    columns: usize,
    span: usize,
) -> Vec<(TileContent<Id>, u16)> {
    let depth = base.summary_depth;
    let positions = target.len() + TABLE_CELL + depth - 1;
    let opened = Layout::horizontal(constraints_for_sizes(&fills(columns))).split(area);
    let mut widths = Vec::with_capacity(positions);
    for (column, &rect) in opened.iter().enumerate() {
        let count = if columns == base.widths.len() {
            base.widths[column]
        } else {
            positions / columns + usize::from(column < positions % columns)
        };
        widths.extend(std::iter::repeat_n(
            frame_inner(share_borders(rect, area)).width,
            count,
        ));
    }
    if depth > 1 {
        widths.drain(1..depth);
    }
    if let Some(last) = opened.get(span - 1) {
        widths[0] = frame_inner(share_borders(
            Rect {
                width: last.right().saturating_sub(area.x),
                ..area
            },
            area,
        ))
        .width;
    }
    cells(target)
        .into_iter()
        .map(|(content, _)| content)
        .zip(widths)
        .collect()
}

/// Use the deepest held summary whose physical residents fit the viewport.
pub(super) fn column_layout(
    count: usize,
    held_depth: usize,
    growth: TileGrowth,
    area: Rect,
    settings: &TileSettings,
) -> ColumnPlan {
    for depth in (1..=held_depth.max(1)).rev() {
        let positions = count.saturating_add(depth - 1);
        let natural = columns(positions, growth);
        if depth > 1 && natural.first().is_none_or(|&height| depth > height) {
            continue;
        }
        let Some(opened) = (1..=natural.len()).rev().find(|&candidate| {
            let columns = Layout::horizontal(constraints_for_sizes(&fills(candidate))).split(area);
            columns
                .iter()
                .all(|&column| share_borders(column, area).width >= settings.min_tile_width)
                || (candidate == 1
                    && area.width > 0
                    && settings.tile_width_floor == TileWidthFloor::CompressSingleColumn)
        }) else {
            return ColumnPlan::InsufficientWidth;
        };
        let widths = if opened == natural.len() {
            natural
        } else {
            let low = positions / opened;
            let extra = positions % opened;
            (0..opened)
                .map(|column| low + usize::from(column < extra))
                .collect()
        };
        if depth > widths[0] {
            continue;
        }
        return ColumnLayout {
            widths,
            summary_depth: depth,
            summary_span: 1,
        }
        .checked(area, settings, 0);
    }
    ColumnPlan::InsufficientRows
}

impl RowAdmissibleColumnLayout {
    /// Recheck each redistributed candidate, including bands below the summary.
    pub(super) fn for_summary(
        self,
        area: Rect,
        held: &HeldCellLayout,
        settings: &TileSettings,
    ) -> ColumnPlan {
        self.for_summary_rows(area, &held.rows, held.summary_span, settings)
    }

    /// Borrow the summary reservation without copying the held row record.
    pub(super) fn for_summary_rows(
        self,
        area: Rect,
        rows: &[CellRowDemand],
        summary_span: usize,
        settings: &TileSettings,
    ) -> ColumnPlan {
        let columns = self.columns;
        let initial_columns = columns.widths.len();
        if settings.summary_row_allocation != SummaryRowAllocation::BeforeTiles {
            return ColumnLayout {
                summary_span: summary_span.min(initial_columns).max(1),
                ..columns
            }
            .checked(area, settings, 0);
        }
        let positions = columns.widths.iter().sum::<usize>();
        for count in (1..=initial_columns).rev() {
            let widths = if count == initial_columns {
                columns.widths.clone()
            } else {
                let low = positions / count;
                let extra = positions % count;
                (0..count)
                    .map(|column| low + usize::from(column < extra))
                    .collect()
            };
            let depth = columns.summary_depth.min(widths[0]);
            let candidate = ColumnLayout {
                widths,
                summary_depth: depth,
                summary_span: summary_span
                    .saturating_mul(count)
                    .div_ceil(initial_columns)
                    .max(1),
            };
            let wants = &rows[..candidate.residents(0).min(rows.len())];
            let shared = shares(
                wants,
                area.height,
                ColumnHead::Summary(depth),
                ColumnFocus::Outside,
                settings.min_tile_height.get(),
            );
            let reserved =
                summary_before_tiles(wants, area.height, settings.min_tile_height.get(), shared);
            let summary_rows = reserved.first().map_or(0, |size| match size {
                PaneAxisSize::Fixed(rows) => *rows,
                PaneAxisSize::Fill(_) => 0,
            });
            let plan = candidate.checked(area, settings, summary_rows);
            if matches!(plan, ColumnPlan::Columns(_)) {
                return plan;
            }
        }
        ColumnPlan::InsufficientRows
    }
}

/// Content readings used to divide one grid into shared columns.
enum GridRowReadings<'a> {
    #[cfg(test)]
    Intrinsic,
    Measured(&'a mut dyn FnMut(usize, Rect, RowUsePurpose) -> u16),
}

impl GridRowReadings<'_> {
    fn column_shares(
        &mut self,
        wants: &[CellRowDemand],
        probe: &RowProbeLayout<'_>,
        head: ColumnHead,
        focused: ColumnFocus,
    ) -> Vec<PaneAxisSize> {
        let shared = match self {
            #[cfg(test)]
            Self::Intrinsic => shares(
                wants,
                probe.band.height,
                head,
                focused,
                probe.settings.min_tile_height.get(),
            ),
            Self::Measured(rows_use) => {
                let first = probe.first_cell - TABLE_CELL;
                let mut unused_rows = |row: usize, rows: u16, purpose: RowUsePurpose| {
                    let available = probe.content_area(row, rows);
                    let kept = rows_use(first.saturating_add(row), available, purpose)
                        .min(available.height);
                    available.height.saturating_sub(kept)
                };
                shares_with_directional_row_usage(
                    wants,
                    probe.band.height,
                    head,
                    focused,
                    probe.settings.min_tile_height.get(),
                    &mut unused_rows,
                )
            },
        };
        if probe.settings.summary_row_allocation == SummaryRowAllocation::BeforeTiles
            && matches!(head, ColumnHead::Summary(_))
            && !wants.is_empty()
        {
            summary_before_tiles(
                wants,
                probe.band.height,
                probe.settings.min_tile_height.get(),
                shared,
            )
        } else {
            shared
        }
    }
}

impl Grid {
    /// Construction checked the bands once; unchanged rules preserve that guarantee.
    pub(super) fn is_current_endpoint(
        &self,
        area: Rect,
        growth: TileGrowth,
        settings: &TileSettings,
    ) -> bool {
        self.area == area && self.rules == EndpointLayoutRules::new(growth, settings)
    }

    /// Resolve the cells `held` describes against `area`.
    ///
    /// The first column is divided before the rest, because the summary
    /// at its top settles how far it reaches: it keeps the height its
    /// own column gives it and widens over as many of the next columns
    /// as `held` asks and [`reach`] allows, each of which then divides
    /// only what is left below it.
    #[cfg(test)]
    pub(super) fn new(
        area: Rect,
        held: &HeldCellLayout,
        growth: TileGrowth,
        settings: &TileSettings,
    ) -> Self {
        Self::with_row_readings(area, held, growth, settings, GridRowReadings::Intrinsic)
    }

    /// Resolve cells while moving room only in steps their contents retain.
    pub(super) fn with_row_use(
        area: Rect,
        held: &HeldCellLayout,
        growth: TileGrowth,
        settings: &TileSettings,
        rows_use: &mut impl FnMut(usize, Rect, RowUsePurpose) -> u16,
    ) -> Self {
        Self::with_row_readings(
            area,
            held,
            growth,
            settings,
            GridRowReadings::Measured(rows_use),
        )
    }

    /// Resolve measured content or intrinsic demands at the same column geometry.
    fn with_row_readings(
        area: Rect,
        held: &HeldCellLayout,
        growth: TileGrowth,
        settings: &TileSettings,
        mut readings: GridRowReadings<'_>,
    ) -> Self {
        let count = held.rows.len();
        let ColumnPlan::Columns(layout) =
            column_layout(count, held.summary_depth, growth, area, settings)
        else {
            return Self::summary_only(area, growth, settings);
        };
        let ColumnPlan::Columns(layout) = layout.for_summary(area, held, settings) else {
            return Self::summary_only(area, growth, settings);
        };
        let ColumnLayout {
            widths,
            summary_depth: depth,
            summary_span: planned_span,
        } = layout.columns;
        let opened: Vec<Rect> = Layout::horizontal(constraints_for_sizes(&fills(widths.len())))
            .split(area)
            .to_vec();
        let mut bands = opened.clone();
        let mut span = 1;
        let mut panes = Vec::with_capacity(count);
        let mut position = TABLE_CELL;
        for (column, &height) in widths.iter().enumerate() {
            let opens_at = position;
            let ends_at = opens_at.saturating_add(height);
            let column_head = if column == 0 {
                ColumnHead::Summary(depth)
            } else {
                ColumnHead::Cell
            };
            let (first_cell, pieces) = match column_head {
                ColumnHead::Summary(depth) => (TABLE_CELL, height - depth + 1),
                ColumnHead::Cell => (opens_at - depth + 1, height),
            };
            let first = first_cell - TABLE_CELL;
            let last = (first + pieces).min(held.rows.len());
            let wants = held.rows.get(first..last).unwrap_or(&[]);
            // The ring is held against the whole grid; a column divides
            // itself, so it is told only about the one cell of its own
            // that carries it.
            let focused = column_focus(held.focused, first_cell, pieces);
            position = ends_at;
            let Some(&band) = bands.get(column) else {
                continue;
            };
            let probe = RowProbeLayout {
                area,
                band,
                first_cell,
                cells: wants.len(),
                opened: &opened,
                widths: &widths,
                summary_span: planned_span,
                settings,
            };
            let shares = readings.column_shares(wants, &probe, column_head, focused);
            for (row, &cell) in Layout::vertical(constraints_for_sizes(&shares))
                .split(band)
                .iter()
                .enumerate()
            {
                let index = first_cell + row;
                let cell = if index == TABLE_CELL {
                    span = reach(&opened, &widths, cell, planned_span, settings);
                    for (band, column) in bands.iter_mut().zip(&opened).take(span).skip(1) {
                        *band = below(*column, cell);
                    }
                    widened(cell, opened.get(span.saturating_sub(1)).copied())
                } else {
                    cell
                };
                panes.push(ResolvedPane {
                    pane: index,
                    area: share_borders(cell, area),
                });
            }
        }
        Self {
            area,
            widths,
            depth,
            resolved: ResolvedPaneLayout::new(panes),
            columns: bands
                .iter()
                .map(|&column| share_borders(column, area))
                .collect(),
            span,
            rules: EndpointLayoutRules::new(growth, settings),
        }
    }

    /// No column arrangement meets the configured floors in this area.
    fn summary_only(area: Rect, growth: TileGrowth, settings: &TileSettings) -> Self {
        Self {
            area,
            widths: vec![1],
            depth: 1,
            resolved: ResolvedPaneLayout::new(vec![ResolvedPane {
                pane: TABLE_CELL,
                area,
            }]),
            columns: Vec::new(),
            span: 1,
            rules: EndpointLayoutRules::new(growth, settings),
        }
    }

    /// The cells each column reads down, for the arrows: its own, with
    /// the summary heading every column it widens over as well as its
    /// own.
    pub(super) fn lanes(&self) -> Vec<Vec<usize>> {
        let mut opens_at = TABLE_CELL;
        self.widths
            .iter()
            .enumerate()
            .map(|(column, &height)| {
                let own = opens_at..opens_at.saturating_add(height);
                opens_at = own.end;
                let under_summary = column > 0 && column < self.span;
                std::iter::once(TABLE_CELL)
                    .filter(|_| under_summary)
                    .chain(own.map(|position| {
                        if position <= self.depth {
                            TABLE_CELL
                        } else {
                            position - self.depth + 1
                        }
                    }))
                    .collect()
            })
            .collect()
    }

    /// Where cell `index` sits, or `None` when the grid has no such cell.
    pub(super) fn cell(&self, index: usize) -> Option<Rect> {
        self.resolved
            .panes
            .iter()
            .find(|resolved| resolved.pane == index)
            .map(|resolved| resolved.area)
    }

    /// The column cell `index` falls in, or `None` when the grid has no
    /// such cell.
    pub(super) fn column_of(&self, index: usize) -> Option<usize> {
        let slot = if index == TABLE_CELL {
            TABLE_CELL
        } else {
            index + self.depth - 1
        };
        position(&self.widths, slot).map(|(column, _)| column)
    }

    /// The rect column `column` occupies, full height.
    #[cfg(test)]
    pub(super) fn column_rect(&self, column: usize) -> Rect {
        self.columns.get(column).copied().unwrap_or(self.area)
    }
}

/// Columns the summary asks to cover, its own included, for its widest
/// line to fit: the fewest of `columns` equal columns across `area` whose
/// combined interior is at least `width` wide, or all of them when even
/// that is too narrow.
pub(super) fn summary_span(area: Rect, columns: usize, width: u16) -> usize {
    let opened = Layout::horizontal(constraints_for_sizes(&fills(columns))).split(area);
    let Some(&first) = opened.first() else {
        return 1;
    };
    opened
        .iter()
        .position(|column| {
            let covered = widened(first, Some(*column));
            frame_inner(share_borders(covered, area)).width >= width
        })
        .map_or(opened.len(), |last| last.saturating_add(1))
}

/// Columns the summary covers once `summary`, its rect in the first
/// column, has its height: as many of the `wanted` as `opened` holds,
/// stopping at the first column whose cells would no longer fit below
/// it at [`TileSettings::min_tile_height`] -- the same floor that stops
/// the grid growing a cell it cannot show.
fn reach(
    opened: &[Rect],
    widths: &[usize],
    summary: Rect,
    wanted: usize,
    settings: &TileSettings,
) -> usize {
    let mut span = 1;
    while span < wanted.min(opened.len()) {
        let (Some(column), Some(&cells)) = (opened.get(span), widths.get(span)) else {
            break;
        };
        let room = column.bottom().saturating_sub(summary.bottom());
        let cells = u16::try_from(cells).unwrap_or(u16::MAX);
        if room < shared_run(cells, settings.min_tile_height.get()) {
            break;
        }
        span += 1;
    }
    span
}

/// `summary` carried right to the far edge of `last`, the last column it
/// covers; left as it is without one.
fn widened(summary: Rect, last: Option<Rect>) -> Rect {
    last.map_or(summary, |last| Rect {
        width: last.right().saturating_sub(summary.x),
        ..summary
    })
}

/// What `column` has left below `summary` for its own cells.
pub(super) const fn below(column: Rect, summary: Rect) -> Rect {
    Rect {
        y: summary.bottom(),
        height: column.bottom().saturating_sub(summary.bottom()),
        ..column
    }
}
