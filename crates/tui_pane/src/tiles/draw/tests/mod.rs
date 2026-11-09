mod motion;

use std::ops::Range;

use ratatui::symbols::line;
use ratatui::widgets::Widget;

use super::*;
use crate::ColorMode;
use crate::TileAction;
use crate::TileDemand;
use crate::TileFill;
use crate::TileView;
use crate::tiles::constants::COLUMN_FOOT_TEST_AREA;
use crate::tiles::constants::COLUMN_FOOT_TEST_GROUP_COUNT;
use crate::tiles::constants::COLUMN_FOOT_TEST_KEY_SUMMARY_ROWS;
use crate::tiles::constants::COLUMN_FOOT_TEST_MIN_HEIGHT;
use crate::tiles::constants::CONTENT_CHANGE_TEST_COLUMN_ENTRIES;
use crate::tiles::constants::CONTENT_CHANGE_TEST_COLUMN_ROWS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_COMMAND_ROWS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_EMPTY_CELLS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_ENTRIES;
use crate::tiles::constants::CONTENT_CHANGE_TEST_GROUPS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_PROGRESS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_PROJECT_ROWS;
use crate::tiles::constants::CONTENT_CHANGE_TEST_ROW;
use crate::tiles::constants::CONTENT_CHANGE_TEST_SOURCE_SPANS;
use crate::tiles::constants::DEPARTED_ENTRY_TEST_SPANS;
use crate::tiles::constants::EMPTY_DONOR_TEST_SPANS;
use crate::tiles::constants::LENDING_TEST_CUT_GROUP;
use crate::tiles::constants::LENDING_TEST_DEMANDS;
use crate::tiles::constants::LENDING_TEST_DRAWN;
use crate::tiles::constants::LENDING_TEST_ENDPOINT_HEIGHTS;
use crate::tiles::constants::LENDING_TEST_GAP_HEIGHT;
use crate::tiles::constants::LENDING_TEST_INITIAL_ROWS;
use crate::tiles::constants::LENDING_TEST_SUMMARY_TITLE;
use crate::tiles::constants::LENDING_TEST_WHOLE_GROUP;
use crate::tiles::constants::MIN_TILE_HEIGHT;
use crate::tiles::constants::PROGRESS_SCALE;
use crate::tiles::constants::SLIDING_TEST_INITIAL_ROWS;
use crate::tiles::constants::SLIDING_TEST_INTERVALS;
use crate::tiles::constants::SLIDING_TEST_MIN_CHANGES;
use crate::tiles::constants::SLIDING_TEST_ROWS;
use crate::tiles::constants::SUMMARY_STEP_TEST_ENTRY_ROWS;
use crate::tiles::constants::SUMMARY_STEP_TEST_HEADER_ROWS;
use crate::tiles::grid::PieceHeightSnap;
use crate::tiles::grid::TilePiece;
use crate::tiles::grid::TilePlacement;

pub(super) const FOOT_TEXT: &str = "mem 2.5G";

pub(super) const GROUP_TITLE_GLYPHS: &[u8] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub(super) const MOTION_SNAPSHOTS: u32 = 24;

pub(super) const TITLE_AREA: Rect = Rect::new(0, 0, 200, 50);

pub(super) const TEST_TILE_WIDTH: u16 = 40;

pub(super) const STAYING_GROWTH: TileGrowth = TileGrowth {
    initial_rows:  4,
    fill:          TileFill::Redistribute,
    widen_summary: false,
};

pub(super) struct StubCells {
    pub(super) summary_foot:  SummaryFoot,
    pub(super) demands:       TileDemands<u32>,
    pub(super) row_counts:    Vec<StubRowCount>,
    pub(super) readout_fades: Vec<(TileContent<u32>, CellReadoutFade)>,
}

pub(super) struct StubRowCount {
    pub(super) content: TileContent<u32>,
    pub(super) count:   u16,
}

impl StubCells {
    pub(super) fn new(groups: impl IntoIterator<Item = u32>) -> Self {
        Self::demanding(1, groups.into_iter().map(|id| (id, 1)))
    }

    pub(super) fn demanding(
        summary: usize,
        groups: impl IntoIterator<Item = (u32, usize)>,
    ) -> Self {
        Self {
            summary_foot:  SummaryFoot::Empty,
            demands:       TileDemands {
                summary,
                summary_width: 0,
                groups: groups
                    .into_iter()
                    .map(|(id, rows)| TileDemand { id, rows })
                    .collect(),
            },
            row_counts:    Vec::new(),
            readout_fades: Vec::new(),
        }
    }

    pub(super) fn with_summary_foot(mut self, summary_foot: SummaryFoot) -> Self {
        self.summary_foot = summary_foot;
        self
    }

    pub(super) fn with_rows_drawn(mut self, content: TileContent<u32>, count: u16) -> Self {
        self.row_counts.push(StubRowCount { content, count });
        self
    }
}

impl TileCells<u32> for StubCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn summary_foot(&self) -> SummaryFoot { self.summary_foot.clone() }

    fn readout_fade(&self, content: &TileContent<u32>) -> CellReadoutFade {
        self.readout_fades
            .iter()
            .find(|(cell, _)| cell == content)
            .map_or(CellReadoutFade::Unfaded, |(_, fade)| *fade)
    }

    fn group_title(&self, id: &u32) -> Option<Line<'static>> { Some(Line::raw(group_title(*id))) }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> { self.demands.clone() }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        self.row_counts
            .iter()
            .find(|count| &count.content == content)
            .map_or_else(
                || {
                    let rows = match content {
                        TileContent::Summary => self.demands.summary,
                        TileContent::Group(id) => self
                            .demands
                            .groups
                            .iter()
                            .find(|group| &group.id == id)
                            .map_or(0, |group| group.rows),
                        TileContent::Empty(_) => 0,
                    };
                    u16::try_from(rows).unwrap_or(u16::MAX).min(area.height)
                },
                |count| count.count.min(area.height),
            )
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        let text = match content {
            TileContent::Summary => "table body",
            TileContent::Group(_) => "group body",
            TileContent::Empty(_) => return,
        };
        Paragraph::new(text).render(inner, buffer);
    }
}

pub(super) struct Toast64MotionCells {
    pub(super) groups:       u32,
    pub(super) summary_rows: usize,
}

impl TileCells<u32> for Toast64MotionCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       self.summary_rows,
            summary_width: 0,
            groups:        (1..=self.groups)
                .map(|id| TileDemand { id, rows: 18 })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        // The grid reserves the foot row outside this content allocation.
        let drawn = match content {
            TileContent::Summary if area.width >= 80 => 13.min(area.height),
            TileContent::Summary if area.height == u16::MAX => 18,
            TileContent::Summary if area.height >= 18 => 18,
            TileContent::Summary if area.height >= 13 => 13,
            TileContent::Summary if area.height >= 9 => 9,
            TileContent::Summary if area.height >= 5 => 5,
            TileContent::Summary => area.height,
            TileContent::Group(_) => 18.min(area.height),
            TileContent::Empty(_) => 0,
        };
        match content {
            TileContent::Summary => drawn.min(u16::try_from(self.summary_rows).unwrap_or(u16::MAX)),
            TileContent::Group(_) | TileContent::Empty(_) => drawn,
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

pub(super) fn draw_stub_rows(buffer: &mut Buffer, area: Rect, rows: u16) {
    if area.width == 0 {
        return;
    }
    for y in area.top()..area.top().saturating_add(rows.min(area.height)) {
        buffer[(area.x, y)].set_symbol("x");
    }
}

/// One row of `buffer` as text, with the blanks to the right of it
/// trimmed off.
pub(super) fn buffer_line(buffer: &Buffer, y: u16) -> String {
    let line: String = (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    line.trim_end().to_string()
}

pub(super) fn buffer_text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| buffer_line(buffer, y))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The text inside `area`, one buffer row at a time.
pub(super) fn area_lines(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.top()..area.bottom())
        .map(|y| {
            (area.left()..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// The one-cell title used by motion fixtures, which stays whole
/// until a closing column has no title cell left.
pub(super) fn group_title(id: u32) -> String {
    let index = usize::try_from(id).unwrap_or_default() % GROUP_TITLE_GLYPHS.len();
    char::from(GROUP_TITLE_GLYPHS[index]).to_string()
}

pub(super) fn is_lattice_glyph(symbol: &str) -> bool {
    [
        line::HORIZONTAL,
        line::VERTICAL,
        line::TOP_LEFT,
        line::TOP_RIGHT,
        line::BOTTOM_LEFT,
        line::BOTTOM_RIGHT,
        line::VERTICAL_RIGHT,
        line::VERTICAL_LEFT,
        line::HORIZONTAL_DOWN,
        line::HORIZONTAL_UP,
        line::CROSS,
    ]
    .contains(&symbol)
}

/// Draw hand-built moving pieces inside hand-built column frames.
pub(super) fn draw_motion_fixture(area: Rect, drawing: &TileDrawing<u32>) -> Buffer {
    draw_motion_fixture_with_width(area, drawing, area.width.saturating_sub(2))
}

/// One hand-built piece and its inseparable name role.
pub(super) fn tile_piece(
    content: TileContent<u32>,
    frame: PaneFrame,
    name: PieceName,
) -> TilePiece<u32> {
    TilePiece::for_test(content, frame, name)
}

/// Mark content coverage independently of a tile's focus tint.
pub(super) struct MotionCoverageCells(pub(super) StubCells);

impl TileCells<u32> for MotionCoverageCells {
    fn summary_title(&self) -> &str { self.0.summary_title() }

    fn group_title(&self, id: &u32) -> Option<Line<'static>> { self.0.group_title(id) }

    fn demands(&self, widths: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        self.0.demands(widths)
    }

    fn draw(
        &self,
        buffer: &mut Buffer,
        content: &TileContent<u32>,
        inner: Rect,
        background: Color,
    ) {
        buffer.set_style(inner, Style::default().bg(Color::Magenta));
        self.0.draw(buffer, content, inner, background);
    }
}

/// Draw moving pieces with the width their contents were measured at.
pub(super) fn draw_motion_fixture_with_width(
    area: Rect,
    drawing: &TileDrawing<u32>,
    measured: u16,
) -> Buffer {
    let groups = (7..=18).collect::<Vec<_>>();
    let demands = TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        groups
            .iter()
            .map(|&id| TileDemand { id, rows: 1 })
            .collect(),
    };
    let widths = groups
        .iter()
        .map(|&id| (TileContent::Group(id), measured))
        .collect::<Vec<_>>();
    let cells = MotionCoverageCells(StubCells::new(groups));
    let mut buffer = Buffer::empty(area);
    draw_placements(
        &mut buffer,
        area,
        drawing,
        &TileMeasurements {
            demands: &demands,
            widths:  &widths,
        },
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    buffer
}

/// A full three-column grid opening a fourth column with three pieces.
pub(super) fn column_transition(area: Rect, opening: bool) -> (TileGrid<u32>, TileGrowth) {
    let growth = TileGrowth {
        initial_rows:  3,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    };
    let groups = |ids: &[u32]| TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        ids.iter().map(|&id| TileDemand { id, rows: 1 }).collect(),
    };
    let (before, after): (&[u32], &[u32]) = if opening {
        (
            &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
            &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18],
        )
    } else {
        (
            &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18],
            &[7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17],
        )
    };
    let mut grid = TileGrid::new();
    grid.set_layout(area, growth);
    grid.sync(&groups(before), growth);
    grid.settle_for_test();
    grid.sync(&groups(after), growth);
    (grid, growth)
}

/// Twenty-four evenly spaced readings of the raw motion clock.
pub(super) fn motion_snapshots() -> impl Iterator<Item = u32> {
    (0..MOTION_SNAPSHOTS).map(|step| step * PROGRESS_SCALE / (MOTION_SNAPSHOTS - 1))
}

/// The visible pane-interior rows after its shift and clip are applied.
pub(super) fn visible_body_rows(frame: PaneFrame) -> Option<Range<u16>> {
    let body = frame.inner();
    let clip = frame_inner(frame.clip());
    let top = (i64::from(body.top()) + i64::from(frame.shift())).max(i64::from(clip.top()));
    let bottom =
        (i64::from(body.bottom()) + i64::from(frame.shift())).min(i64::from(clip.bottom()));
    let top = u16::try_from(top).ok()?;
    let bottom = u16::try_from(bottom).ok()?;
    (top < bottom).then_some(top..bottom)
}

pub(super) fn doubled_line_failure(
    buffer: &Buffer,
    area: Rect,
    motion: &str,
    raw: u32,
) -> Option<String> {
    let row = |y| {
        let start = buffer.index_of(area.x, y);
        &buffer.content[start..start + usize::from(area.width)]
    };
    for y in area.top()..area.bottom().saturating_sub(1) {
        for (upper, lower) in row(y).windows(3).zip(row(y + 1).windows(3)) {
            if upper
                .iter()
                .chain(lower)
                .all(|cell| cell.symbol() == line::HORIZONTAL)
            {
                return Some(format!(
                    "{motion} at raw {raw} draws horizontal lines on rows {y} and {}",
                    y + 1
                ));
            }
        }
    }

    let right_edges = [
        line::VERTICAL,
        line::VERTICAL_LEFT,
        line::TOP_RIGHT,
        line::BOTTOM_RIGHT,
    ];
    let left_edges = [
        line::VERTICAL,
        line::VERTICAL_RIGHT,
        line::TOP_LEFT,
        line::BOTTOM_LEFT,
    ];
    for y in area.top()..area.bottom() {
        for (offset, pair) in row(y).windows(2).enumerate() {
            if right_edges.contains(&pair[0].symbol()) && left_edges.contains(&pair[1].symbol()) {
                let x = usize::from(area.x) + offset;
                return Some(format!(
                    "{motion} at raw {raw} draws vertical lines side by side at ({x}, {y})"
                ));
            }
        }
    }
    None
}

/// Whether a row has content, a frame, or an inactive cell's untinted foot.
pub(super) fn row_has_coverage(
    buffer: &Buffer,
    drawing: &TileDrawing<u32>,
    ground: Color,
    cells: Range<u16>,
    y: u16,
) -> bool {
    cells.into_iter().any(|x| {
        let cell = &buffer[(x, y)];
        cell.bg != ground
            || is_lattice_glyph(cell.symbol())
            || drawing.pieces.iter().any(|piece| {
                piece_title_covers(piece, cell.symbol(), x, y)
                    || untinted_foot_contains(buffer, piece.placement.frame, x, y)
            })
    })
}

/// An inactive foot needs its fixture count unless no readout text fits.
pub(super) fn untinted_foot_contains(buffer: &Buffer, frame: PaneFrame, x: u16, y: u16) -> bool {
    let ReadoutRow::Reserved(readout) = readout_row(frame.inner()) else {
        return false;
    };
    let clip = frame_inner(frame.clip());
    let contains = !frame.is_focused()
        && clip.contains((x, y).into())
        && x >= frame.inner().left()
        && x < frame.inner().right()
        && i64::from(y) == i64::from(readout.y) + i64::from(frame.shift());
    if !contains {
        return false;
    }
    // Motion fixtures reserve their one measured row for the foot.
    let count = format!("{TILE_ROWS_CONTENT_LABEL}0");
    if count.len() > usize::from(readout.width) {
        return true;
    }
    let text: String = (readout.left().max(buffer.area.left())
        ..readout.right().min(buffer.area.right()))
        .map(|column| buffer[(column, y)].symbol())
        .collect();
    text.contains(&count)
}

/// Whether this cell contains the exact title character this piece owns.
pub(super) fn piece_title_covers(piece: &TilePiece<u32>, symbol: &str, x: u16, y: u16) -> bool {
    if piece.name != PieceName::Shown {
        return false;
    }
    let clip = piece.placement.frame.clip();
    let Some(offset) = x.checked_sub(clip.left().saturating_add(1)) else {
        return false;
    };
    if y != clip.top() || x >= clip.right().saturating_sub(1) {
        return false;
    }
    let title = match &piece.placement.content {
        TileContent::Summary => "summary".to_string(),
        TileContent::Group(id) => group_title(*id),
        TileContent::Empty(_) => return false,
    };
    title
        .chars()
        .nth(usize::from(offset))
        .is_some_and(|character| {
            let mut encoded = [0; 4];
            symbol == character.encode_utf8(&mut encoded)
        })
}

/// A band row with no painted cell, frame line or title.
pub(super) fn bare_band_row_failure(
    buffer: &Buffer,
    drawing: &TileDrawing<u32>,
    motion: &str,
    raw: u32,
) -> Option<String> {
    let ground = screen_ground().unwrap_or(Color::Reset);
    for &band in &drawing.column_bands {
        let inner = frame_inner(band);
        if inner.is_empty() {
            continue;
        }
        for y in inner.top()..inner.bottom() {
            if !row_has_coverage(buffer, drawing, ground, inner.left()..inner.right(), y) {
                let frames = drawing
                    .placements()
                    .filter(|placement| {
                        let clip = placement.frame.clip();
                        clip.x == band.x && clip.width == band.width
                    })
                    .map(|placement| {
                        (
                            &placement.content,
                            placement.frame.rect(),
                            placement.frame.shift(),
                            placement.frame.clip(),
                        )
                    })
                    .collect::<Vec<_>>();
                return Some(format!(
                    "{motion} at raw {raw} leaves band {band:?} row {y} on the screen ground; \
                         frames={frames:?}"
                ));
            }
        }
    }
    None
}

/// Draw a settled grid with `cells` into a new buffer.
pub(super) fn settled_grid(area: Rect, cells: &StubCells) -> (TileGrid<u32>, Buffer) {
    let growth = TileGrowth::default();
    let mut grid = TileGrid::new();
    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
    (grid, buffer)
}

/// Draw twice around settling, with a selected view rule.
pub(super) fn settled_grid_in_view(
    area: Rect,
    cells: &StubCells,
    view: TileView,
    min_tile_height: u16,
) -> (TileGrid<u32>, Buffer) {
    let growth = TileGrowth::default();
    let mut grid = TileGrid::new();
    grid.set_view(view);
    grid.set_min_tile_height(min_tile_height);
    let mut buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
    grid.settle_for_test();
    buffer = Buffer::empty(area);
    draw_tile_grid(
        &mut buffer,
        &mut grid,
        area,
        growth,
        TileGridContents::Shown,
        cells,
        ColorMode::Colored,
    );
    (grid, buffer)
}

/// Resolve one exact motion point through the production measurement path.
pub(super) fn fixed_motion_drawing(
    grid: &mut TileGrid<u32>,
    cells: &impl TileCells<u32>,
    area: Rect,
    growth: TileGrowth,
    raw: u32,
) -> TileDrawing<u32> {
    grid.set_layout(area, growth);
    let cells_widths = measurement_widths(grid, area, growth, GridDisplay::Cells);
    let mut cells_demands = cells.demands(&cells_widths);
    add_readout_rows(&mut cells_demands, &cells_widths);
    grid.sync_rendered_demands(&cells_demands, growth);
    measured_motion_drawing(grid, cells, area, growth, raw)
}

/// Measure geometry after the fixture has published its content change.
pub(super) fn measured_motion_drawing(
    grid: &mut TileGrid<u32>,
    cells: &impl TileCells<u32>,
    area: Rect,
    growth: TileGrowth,
    raw: u32,
) -> TileDrawing<u32> {
    grid.measure_row_steps(
        area,
        growth,
        raw,
        |content, area| cells.rows_drawn(content, area),
        |content, area| cells.rows_kept(content, area),
    );
    grid.drawing_at(area, growth, raw)
}

/// Whether one placement uses all its rows or shows its whole value.
pub(super) fn placement_fits(cells: &impl TileCells<u32>, placement: &TilePlacement<u32>) -> bool {
    let area = content_area(placement.frame.inner());
    let drawn = cells.rows_drawn(&placement.content, area);
    let whole = cells.rows_drawn(
        &placement.content,
        Rect {
            height: u16::MAX,
            ..area
        },
    );
    drawn == area.height || drawn >= whole
}

/// Draw every sampled frame with the row probes used by the renderer.
pub(super) fn assert_column_foot_has_no_doubled_line(
    grid: &mut TileGrid<u32>,
    cells: &impl TileCells<u32>,
    growth: TileGrowth,
) {
    for raw in motion_snapshots() {
        let drawing = fixed_motion_drawing(grid, cells, COLUMN_FOOT_TEST_AREA, growth, raw);
        let widths = measurement_widths(grid, COLUMN_FOOT_TEST_AREA, growth, GridDisplay::Cells);
        let mut demands = cells.demands(&widths);
        add_readout_rows(&mut demands, &widths);
        let mut buffer = Buffer::empty(COLUMN_FOOT_TEST_AREA);
        draw_placements(
            &mut buffer,
            COLUMN_FOOT_TEST_AREA,
            &drawing,
            &TileMeasurements {
                demands: &demands,
                widths:  &widths,
            },
            TileGridContents::Shown,
            cells,
            ColorMode::Colored,
        );
        assert_eq!(
            doubled_line_failure(&buffer, COLUMN_FOOT_TEST_AREA, "column foot", raw),
            None,
            "frames={:?}",
            drawing
                .placements()
                .map(|placement| (&placement.content, placement.frame))
                .collect::<Vec<_>>()
        );
    }
}

/// A summary whose compact entries omit separators while commands stay whole.
#[derive(Clone, Copy)]
pub(super) struct ChangingSummaryCells {
    pub(super) entries: u16,
    pub(super) groups:  u32,
}

impl TileCells<u32> for ChangingSummaryCells {
    fn summary_title(&self) -> &'static str { LENDING_TEST_SUMMARY_TITLE }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> {
        TileDemands {
            summary:       usize::from(self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS),
            summary_width: 0,
            groups:        (1..=self.groups)
                .map(|id| TileDemand {
                    id,
                    rows: usize::from(CONTENT_CHANGE_TEST_COMMAND_ROWS),
                })
                .collect(),
        }
    }

    fn rows_drawn(&self, content: &TileContent<u32>, area: Rect) -> u16 {
        match content {
            TileContent::Summary => {
                let whole = self.entries * CONTENT_CHANGE_TEST_PROJECT_ROWS;
                if area.height >= whole {
                    whole
                } else {
                    let header = area.height.min(SUMMARY_STEP_TEST_HEADER_ROWS);
                    header
                        + (area.height.saturating_sub(header) / SUMMARY_STEP_TEST_ENTRY_ROWS)
                            .min(self.entries)
                            * SUMMARY_STEP_TEST_ENTRY_ROWS
                }
            },
            TileContent::Group(_) => CONTENT_CHANGE_TEST_COMMAND_ROWS.min(area.height),
            TileContent::Empty(_) => 0,
        }
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent<u32>, inner: Rect, _: Color) {
        draw_stub_rows(buffer, inner, self.rows_drawn(content, inner));
    }
}

/// Begin from a drawn settled column with the requested empty cells.
pub(super) fn changing_summary_grid(cells: ChangingSummaryCells, empties: usize) -> TileGrid<u32> {
    let mut grid = TileGrid::new();
    grid.set_view(TileView::Cells);
    grid.set_min_tile_width(TEST_TILE_WIDTH);
    grid.set_min_tile_height(COLUMN_FOOT_TEST_MIN_HEIGHT);
    let growth = changing_summary_growth();
    fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        growth,
        PROGRESS_SCALE,
    );
    grid.settle_for_test();
    for _ in 0..empties {
        fixed_motion_drawing(
            &mut grid,
            &cells,
            COLUMN_FOOT_TEST_AREA,
            growth,
            PROGRESS_SCALE,
        );
        grid.apply(TileAction::Add, growth);
        grid.settle_for_test();
    }
    fixed_motion_drawing(
        &mut grid,
        &cells,
        COLUMN_FOOT_TEST_AREA,
        growth,
        PROGRESS_SCALE,
    );
    grid
}

pub(super) fn changing_summary_growth() -> TileGrowth {
    TileGrowth {
        initial_rows:  SLIDING_TEST_INITIAL_ROWS,
        fill:          TileFill::Redistribute,
        widen_summary: false,
    }
}

/// Read the painted content back after measuring the exact motion frame.
pub(super) fn assert_motion_content_is_filled(
    grid: &mut TileGrid<u32>,
    cells: ChangingSummaryCells,
    raw: u32,
) -> TileDrawing<u32> {
    let area = COLUMN_FOOT_TEST_AREA;
    let drawing = fixed_motion_drawing(grid, &cells, area, changing_summary_growth(), raw);
    let mut buffer = Buffer::empty(area);
    let widths = measurement_widths(grid, area, changing_summary_growth(), GridDisplay::Cells);
    draw_placements(
        &mut buffer,
        area,
        &drawing,
        &TileMeasurements {
            demands: &cells.demands(&widths),
            widths:  &widths,
        },
        TileGridContents::Shown,
        &cells,
        ColorMode::Colored,
    );
    assert_no_unusable_row(&cells, &drawing, 0, raw);
    for placement in drawing.placements() {
        let content = content_area(placement.frame.inner());
        let whole = cells.rows_drawn(
            &placement.content,
            Rect {
                height: u16::MAX,
                ..content
            },
        );
        if cells.rows_drawn(&placement.content, content) < whole && content.height > 0 {
            let row = buffer_line(&buffer, content.bottom().saturating_sub(1));
            assert!(row.contains(CONTENT_CHANGE_TEST_ROW), "{raw}: {row}");
        }
    }
    drawing
}

pub(super) fn assert_drawing_tiles_bands(drawing: &TileDrawing<u32>, raw: u32) {
    for placement in drawing.placements() {
        let clip = placement.frame.clip();
        if placement.content != TileContent::Summary {
            assert!(
                drawing
                    .column_bands
                    .iter()
                    .any(|band| band.intersection(clip) == clip),
                "{raw}: {:?} {clip:?} leaves {:?}",
                placement.content,
                drawing.column_bands
            );
        }
    }
    for band in &drawing.column_bands {
        let mut pieces = drawing
            .placements()
            .filter_map(|piece| {
                let clip = piece.frame.clip();
                if (clip.x == band.x && clip.width == band.width)
                    || (piece.content == TileContent::Summary
                        && clip.top() == band.top()
                        && clip.x == band.x)
                {
                    Some(clip)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        pieces.sort_by_key(|rect| rect.top());
        assert_eq!(
            pieces.first().map(|rect| rect.top()),
            Some(band.top()),
            "{raw}: {pieces:?}, {band:?}"
        );
        assert_eq!(
            pieces.last().map(|rect| rect.bottom()),
            Some(band.bottom()),
            "{raw}: {pieces:?}, {band:?}"
        );
        for pair in pieces.windows(2) {
            assert_eq!(
                pair[0].bottom().saturating_sub(1),
                pair[1].top(),
                "{raw}: {pair:?}"
            );
        }
    }
    let summary = drawing
        .placements()
        .find(|piece| piece.content == TileContent::Summary);
    for command in drawing
        .placements()
        .filter(|piece| piece.content != TileContent::Summary)
    {
        if let Some(summary) = &summary {
            let overlap = summary.frame.clip().intersection(command.frame.clip());
            assert!(
                overlap.height <= 1 || overlap.width <= 1,
                "{raw}: summary {:?} overlaps {:?} in {overlap:?}",
                summary.frame.clip(),
                command.frame.clip()
            );
        }
    }
}

pub(super) fn assert_no_givable_spare_row(
    cells: &impl TileCells<u32>,
    drawing: &TileDrawing<u32>,
    endpoint: &TileDrawing<u32>,
    raw: u32,
) {
    for piece in &drawing.pieces {
        if raw < PROGRESS_SCALE
            && matches!(
                piece.height_snap,
                PieceHeightSnap::Search { start: 1, .. } | PieceHeightSnap::Search { end: 1, .. }
            )
        {
            continue;
        }
        let donor = &piece.placement;
        let area = content_area(donor.frame.inner());
        let kept = cells.rows_kept(&donor.content, area);
        let end = endpoint
            .placements()
            .find(|piece| piece.content == donor.content);
        let end_height = end.map_or(0, |piece| piece.frame.clip().height);
        let needs_row_at_endpoint = piece.height_snap != PieceHeightSnap::Settled
            && donor.frame.clip().height <= end_height;
        if kept >= area.height
            || donor.frame.clip().height <= COLUMN_FOOT_TEST_MIN_HEIGHT
            || needs_row_at_endpoint
        {
            continue;
        }
        for recipient in drawing
            .pieces
            .iter()
            .filter(|piece| {
                raw >= PROGRESS_SCALE
                    || !matches!(
                        piece.height_snap,
                        PieceHeightSnap::Search { start: 1, .. }
                            | PieceHeightSnap::Search { end: 1, .. }
                    )
            })
            .map(|piece| &piece.placement)
        {
            let recipient_area = content_area(recipient.frame.inner());
            if donor.content == recipient.content
                || donor.frame.clip().x != recipient.frame.clip().x
            {
                continue;
            }
            if raw < PROGRESS_SCALE
                && endpoint
                    .placements()
                    .find(|piece| piece.content == recipient.content)
                    .is_some_and(|end| {
                        recipient.frame.clip().height.saturating_add(1) > end.frame.clip().height
                    })
            {
                continue;
            }
            let rows = cells.rows_drawn(&recipient.content, recipient_area);
            let next_rows = cells.rows_drawn(
                &recipient.content,
                Rect {
                    height: recipient_area.height.saturating_add(1),
                    ..recipient_area
                },
            );
            assert!(
                next_rows <= rows,
                "{raw}: {:?} keeps {kept}/{} beside {:?} drawing {rows}/{next_rows}; {:?}",
                donor.content,
                area.height,
                recipient.content,
                column_geometry(drawing)
            );
        }
    }
}

/// No incomplete piece leaves an unused row beside a cut column mate.
pub(super) fn assert_no_unusable_row(
    cells: &impl TileCells<u32>,
    drawing: &TileDrawing<u32>,
    key: usize,
    raw: u32,
) {
    for placement in drawing.placements() {
        let area = content_area(placement.frame.inner());
        let drawn = cells.rows_drawn(&placement.content, area);
        let whole = cells.rows_drawn(
            &placement.content,
            Rect {
                height: u16::MAX,
                ..area
            },
        );
        assert!(
            drawn == area.height || drawn >= whole,
            "key {key} at {raw}: {:?} holds span {} but draws {drawn}/{} of {whole}; column {:?}",
            placement.content,
            placement.frame.clip().height.saturating_sub(1),
            area.height,
            column_geometry(drawing),
        );
    }
}

/// Cell identities and their visible frames in column order.
pub(super) fn column_geometry(drawing: &TileDrawing<u32>) -> Vec<(TileContent<u32>, Rect)> {
    drawing
        .placements()
        .filter(|placement| has_visible_body_row(placement.frame))
        .map(|placement| (placement.content.clone(), placement.frame.clip()))
        .collect()
}

/// Check each sampled frame's body rows and the unchanged cells' paths.
pub(super) fn assert_key_frame_keeps_height_rules(
    cells: &impl TileCells<u32>,
    before: &TileDrawing<u32>,
    after: &TileDrawing<u32>,
    drawing: &TileDrawing<u32>,
    previous: &mut [(TileContent<u32>, u16)],
) {
    let visible = drawing
        .placements()
        .filter(|placement| has_visible_body_row(placement.frame))
        .map(|placement| placement.frame.clip())
        .collect::<Vec<_>>();
    for pair in visible.windows(2) {
        assert_eq!(
            pair[0].bottom().saturating_sub(1),
            pair[1].top(),
            "neighbours share one edge"
        );
    }
    assert_eq!(
        visible.last().map(|frame| frame.bottom()),
        Some(COLUMN_FOOT_TEST_AREA.bottom())
    );
    for placement in drawing.placements() {
        let height = placement.frame.clip().height;
        assert!(
            height == 1 || height >= MIN_TILE_HEIGHT,
            "a piece takes no edge without a body"
        );
        assert!(
            placement_fits(cells, placement),
            "a piece fills its rows or is whole"
        );
        let start = before
            .placements()
            .find(|held| held.content == placement.content);
        let end = after
            .placements()
            .find(|held| held.content == placement.content);
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        let (start, end) = (start.frame.clip(), end.frame.clip());
        assert!(
            (start.height.min(end.height)..=start.height.max(end.height)).contains(&height),
            "{:?} leaves its endpoint heights",
            placement.content
        );
        assert!(placement.frame.clip().x == start.x || placement.frame.clip().x == end.x);
        if let Some((_, preceding)) = previous
            .iter_mut()
            .find(|(content, _)| *content == placement.content)
        {
            assert!(if start.height <= end.height {
                height >= *preceding
            } else {
                height <= *preceding
            });
            *preceding = height;
        }
    }
}

pub(super) fn title_demands(ids: &[u32]) -> TileDemands<u32> {
    TileDemands {
        summary:       0,
        summary_width: 0,
        groups:        ids.iter().map(|&id| TileDemand { id, rows: 1 }).collect(),
    }
}

pub(super) struct SegmentedTitleCells;

impl TileCells<u32> for SegmentedTitleCells {
    fn summary_title(&self) -> &'static str { "summary" }

    fn demands(&self, _: &[(TileContent<u32>, u16)]) -> TileDemands<u32> { title_demands(&[7]) }

    fn group_title(&self, _: &u32) -> Option<Line<'static>> {
        Some(Line::from(vec![
            Span::styled("worker", Style::default().fg(Color::Blue)),
            Span::styled(" · impl", Style::default().fg(Color::Yellow)),
        ]))
    }

    fn draw(&self, _: &mut Buffer, _: &TileContent<u32>, _: Rect, _: Color) {}
}
