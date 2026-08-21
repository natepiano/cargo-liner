//! The tile grid: how many cells the pane holds, where each one sits,
//! and the motion from one arrangement to the next.
//!
//! Cells are numbered from one and fill column by column. Cell one is
//! the running-cargo table; the rest are placeholders carrying their own
//! number. [`columns`] is the whole layout rule and is pure, so the
//! arrangement at any count is a test rather than something to squint at
//! on screen.
//!
//! Splitting a rect is the framework's job, not this module's:
//! [`constraints_for_sizes`] turns per-column and per-row shares into
//! ratatui constraints and ratatui's solver tiles the rect exactly, and
//! the result is a [`ResolvedPaneLayout`] keyed by cell number -- the
//! same type [`tui_pane::render_panes`] walks. What is left here is only
//! what the framework has no opinion about: how many columns there are,
//! how tall each one is, and how a cell travels when that changes.
//!
//! Neighbours share a border, so the rects handed out overlap by the one
//! line between them rather than sitting flush. [`tui_pane::GridLines`]
//! draws that line once for both.

use std::time::Instant;

use ratatui::layout::Layout;
use ratatui::layout::Rect;
use tui_pane::PaneAxisSize;
use tui_pane::PaneFrame;
use tui_pane::ResolvedPane;
use tui_pane::ResolvedPaneLayout;
use tui_pane::constraints_for_sizes;
use tui_pane::share_borders;

use crate::constants::MIN_INITIAL_ROWS;
use crate::constants::MIN_TILE_HEIGHT;
use crate::constants::MIN_TILE_WIDTH;
use crate::constants::PROGRESS_SCALE;
use crate::constants::TILE_ANIMATION_MILLIS;

/// One cell as a single frame should draw it.
///
/// A cell crossing between columns is drawn as two of these -- the piece
/// leaving the old column and the piece arriving in the new one -- which
/// is what makes it read as sliding off one column's edge and back in at
/// the next.
pub(crate) struct Placement {
    /// The cell's number. Cell one holds the table.
    pub(crate) index: usize,
    /// Where the cell's box sits this frame, and how far it is cut off
    /// -- the framework's own account of a moving pane, which both
    /// [`tui_pane::draw_clipped`] and [`tui_pane::GridLines`] read.
    pub(crate) frame: PaneFrame,
}

/// The arrangement a transition is moving away from.
struct Transition {
    /// Cell count before the change.
    from:    usize,
    /// When the motion began.
    started: Instant,
}

/// The grid's cell count and the transition it is playing, if any.
pub(crate) struct TileGrid {
    /// Cells the grid holds, the table included. Never below one.
    count:      usize,
    /// The motion in flight, or `None` once the grid has settled.
    transition: Option<Transition>,
    /// The rect the last frame laid out, so [`Self::add`] can tell
    /// whether the cells it would create still fit on screen.
    area:       Rect,
}

impl TileGrid {
    /// A grid holding nothing but the table.
    pub(crate) const fn new() -> Self {
        Self {
            count:      1,
            transition: None,
            area:       Rect::ZERO,
        }
    }

    /// Cells the grid holds.
    #[cfg(test)]
    const fn count(&self) -> usize { self.count }

    /// Record the rect the pane just laid out.
    pub(crate) const fn set_area(&mut self, area: Rect) { self.area = area; }

    /// Retire a transition that has run its course, reporting whether
    /// the grid still wants repainting.
    pub(crate) fn tick(&mut self) -> bool {
        if self.transition.is_none() {
            return false;
        }
        if self.progress() >= PROGRESS_SCALE {
            self.transition = None;
        }
        true
    }

    /// Add a cell, unless the grid it would make no longer fits.
    ///
    /// Refusing beats filling the pane with cells too small to carry a
    /// border and a number: the grid stops growing at the point the
    /// terminal stops being able to show it.
    pub(crate) fn add(&mut self, initial_rows: usize) {
        if !self.fits(self.count.saturating_add(1), initial_rows) {
            return;
        }
        self.start(self.count.saturating_add(1));
    }

    /// Remove a cell. The table is never removed, so the grid bottoms
    /// out at one.
    pub(crate) fn remove(&mut self) {
        if self.count <= 1 {
            return;
        }
        self.start(self.count - 1);
    }

    /// Move to `count`, animating away from wherever the grid stands.
    fn start(&mut self, count: usize) {
        self.transition = Some(Transition {
            from:    self.count,
            started: Instant::now(),
        });
        self.count = count;
    }

    /// Whether every cell of a `count`-cell grid would be big enough to
    /// draw in the rect the last frame used.
    fn fits(&self, count: usize, initial_rows: usize) -> bool {
        let widths = columns(count, initial_rows);
        let (Ok(opened), Some(&tallest)) = (u16::try_from(widths.len()), widths.iter().max())
        else {
            return false;
        };
        let Ok(tallest) = u16::try_from(tallest) else {
            return false;
        };
        self.area.width >= shared_run(opened, MIN_TILE_WIDTH)
            && self.area.height >= shared_run(tallest, MIN_TILE_HEIGHT)
    }

    /// How far through the current transition the grid is, on the
    /// [`PROGRESS_SCALE`] scale. A settled grid is fully through.
    fn progress(&self) -> u32 {
        let Some(transition) = self.transition.as_ref() else {
            return PROGRESS_SCALE;
        };
        let elapsed = transition.started.elapsed().as_millis();
        let total = u128::from(TILE_ANIMATION_MILLIS);
        if total == 0 || elapsed >= total {
            return PROGRESS_SCALE;
        }
        u32::try_from(elapsed * u128::from(PROGRESS_SCALE) / total).unwrap_or(PROGRESS_SCALE)
    }

    /// Every piece to draw this frame, in cell order.
    pub(crate) fn placements(&self, area: Rect, initial_rows: usize) -> Vec<Placement> {
        let settled = Grid::new(area, self.count, initial_rows);
        let Some(transition) = self.transition.as_ref() else {
            return settled
                .resolved
                .panes
                .iter()
                .map(|resolved| Placement {
                    index: resolved.pane,
                    frame: PaneFrame::new(resolved.area),
                })
                .collect();
        };

        let progress = eased(self.progress());
        let before = Grid::new(area, transition.from, initial_rows);
        let mut placements = Vec::new();
        for index in 1..=transition.from.max(self.count) {
            moving_cell(&before, &settled, index, progress, &mut placements);
        }
        placements
    }
}

/// One arrangement resolved against a rect: the rect every cell holds,
/// and the rect every column holds.
///
/// Columns are resolved one at a time rather than through
/// [`tui_pane::PaneGridLayout`] because the grid is ragged -- column one
/// can stand a row taller than the rest -- and that type's placements
/// describe a uniform grid.
struct Grid {
    /// The rect the whole grid fills.
    area:     Rect,
    /// Rows in each column, left to right.
    widths:   Vec<usize>,
    /// Where each cell sits, keyed by cell number.
    resolved: ResolvedPaneLayout<usize>,
    /// The full-height rect each column occupies.
    columns:  Vec<Rect>,
}

impl Grid {
    /// Resolve `count` cells against `area`.
    fn new(area: Rect, count: usize, initial_rows: usize) -> Self {
        let widths = columns(count, initial_rows);
        let opened: Vec<Rect> = Layout::horizontal(constraints_for_sizes(&fills(widths.len())))
            .split(area)
            .to_vec();
        let mut panes = Vec::with_capacity(count);
        let mut index = 1;
        for (column, &height) in widths.iter().enumerate() {
            let Some(&column_rect) = opened.get(column) else {
                continue;
            };
            for &cell in Layout::vertical(constraints_for_sizes(&fills(height)))
                .split(column_rect)
                .iter()
            {
                panes.push(ResolvedPane {
                    pane: index,
                    area: share_borders(cell, area),
                });
                index += 1;
            }
        }
        Self {
            area,
            widths,
            resolved: ResolvedPaneLayout::new(panes),
            columns: opened
                .iter()
                .map(|&column| share_borders(column, area))
                .collect(),
        }
    }

    /// Where cell `index` sits, or `None` when the grid has no such cell.
    fn cell(&self, index: usize) -> Option<Rect> {
        self.resolved
            .panes
            .iter()
            .find(|resolved| resolved.pane == index)
            .map(|resolved| resolved.area)
    }

    /// The column cell `index` falls in, or `None` when the grid has no
    /// such cell.
    fn column_of(&self, index: usize) -> Option<usize> {
        position(&self.widths, index).map(|(column, _)| column)
    }

    /// The rect column `column` occupies, full height.
    fn column_rect(&self, column: usize) -> Rect {
        self.columns.get(column).copied().unwrap_or(self.area)
    }
}

/// Rows in each column, left to right, for a grid of `count` cells.
///
/// The grid grows in two regimes. Up to `initial_rows` squared, columns
/// fill greedily: each one takes `initial_rows` cells before the next
/// opens, so the first column reaches full height before the pane ever
/// splits sideways. Past that the grid grows toward the next square --
/// first every existing column gains a row, one column per cell added,
/// then a new column opens and fills. That is what makes cell 17 at four
/// initial rows rearrange into `[5, 4, 4, 4]` rather than appending to
/// the right: the row count went up, and the cells snake back one place
/// to fill it.
fn columns(count: usize, initial_rows: usize) -> Vec<usize> {
    let rows = initial_rows.max(MIN_INITIAL_ROWS);
    if count == 0 {
        return Vec::new();
    }
    if count <= rows * rows {
        // Greedy: every column but the last holds `rows` cells.
        let opened = count.div_ceil(rows);
        let filled = opened.saturating_sub(1);
        let mut widths = vec![rows; filled];
        widths.push(count - rows * filled);
        return widths;
    }

    // Past the initial square the grid heads for the next one. `side` is
    // the square being filled, `prev` the one just completed -- which is
    // also how many columns stood before this generation began.
    let side = rows.max(ceil_sqrt(count));
    let prev = side.saturating_sub(1);
    if count <= prev * side {
        // Rows growing: the leading columns have taken their extra row,
        // the rest are still a row short.
        let grown = count - prev * prev;
        let mut widths = vec![side; grown];
        widths.resize(prev, prev);
        return widths;
    }
    // Every column stands at full height, so the new one is filling.
    let mut widths = vec![side; prev];
    widths.push(count - prev * side);
    widths
}

/// Smallest `side` with `side * side >= value`.
const fn ceil_sqrt(value: usize) -> usize {
    let mut side = 1;
    while side * side < value {
        side += 1;
    }
    side
}

/// `count` equal shares of one axis.
fn fills(count: usize) -> Vec<PaneAxisSize> { vec![PaneAxisSize::Fill(1); count] }

/// Cells a run of `count` tiles needs along one axis once neighbours
/// share a border: one leading line and its content per tile, then a
/// single line closing the run.
const fn shared_run(count: u16, min_tile: u16) -> u16 {
    count
        .saturating_mul(min_tile.saturating_sub(1))
        .saturating_add(1)
}

/// Work out how one cell moves between two arrangements and push the
/// pieces that draws as.
fn moving_cell(before: &Grid, after: &Grid, index: usize, progress: u32, out: &mut Vec<Placement>) {
    match (before.cell(index), after.cell(index)) {
        (Some(from), Some(to)) => {
            if before.column_of(index) == after.column_of(index) {
                out.push(Placement {
                    index,
                    frame: PaneFrame::new(lerp_rect(from, to, progress)),
                });
                return;
            }
            wrapping_cell(before, after, index, progress, (from, to), out);
        },
        // A cell that has just appeared. It grows in from the edge its
        // column came from, so a new row rises out of the floor and a
        // new column arrives from the right.
        (None, Some(to)) => out.push(Placement {
            index,
            frame: PaneFrame::new(lerp_rect(edge_rect(before, after, index, to), to, progress)),
        }),
        // A cell on its way out, running that motion backwards.
        (Some(from), None) => out.push(Placement {
            index,
            frame: PaneFrame::new(lerp_rect(
                from,
                edge_rect(after, before, index, from),
                progress,
            )),
        }),
        (None, None) => (),
    }
}

/// Push the two pieces a cell crossing columns draws as.
///
/// The cell slides out of one column and back in at the other, against
/// the direction the grid is snaking: moving one column left it leaves
/// over the top and returns from the bottom, and moving right it does
/// the reverse. Each piece is clipped to its own column, so neither is
/// ever seen outside one.
fn wrapping_cell(
    before: &Grid,
    after: &Grid,
    index: usize,
    progress: u32,
    (from, to): (Rect, Rect),
    out: &mut Vec<Placement>,
) {
    let from_column = before.column_of(index).unwrap_or_default();
    let to_column = after.column_of(index).unwrap_or_default();
    let leaving = before.column_rect(from_column);
    let arriving = after.column_rect(to_column);
    let remaining = PROGRESS_SCALE.saturating_sub(progress);

    let (exit, entry) = if to_column < from_column {
        (
            -travel(i32::from(from.bottom()) - i32::from(leaving.y), progress),
            travel(i32::from(arriving.bottom()) - i32::from(to.y), remaining),
        )
    } else {
        (
            travel(i32::from(leaving.bottom()) - i32::from(from.y), progress),
            -travel(i32::from(to.bottom()) - i32::from(arriving.y), remaining),
        )
    };

    out.push(Placement {
        index,
        frame: PaneFrame::shifted(from, exit, leaving),
    });
    out.push(Placement {
        index,
        frame: PaneFrame::shifted(to, entry, arriving),
    });
}

/// The collapsed rect a cell absent from `before` grows out of on its
/// way to `to`.
///
/// A cell opening a new column comes in from the right edge with no
/// width; a cell filling a new row in a column that already stood rises
/// out of that column's floor with no height.
fn edge_rect(before: &Grid, after: &Grid, index: usize, to: Rect) -> Rect {
    let column = after.column_of(index).unwrap_or_default();
    if column >= before.widths.len() {
        return Rect {
            x: after.area.right(),
            width: 0,
            ..to
        };
    }
    Rect {
        y: after.column_rect(column).bottom(),
        height: 0,
        ..to
    }
}

/// `distance` scaled by `progress`, rounding toward zero.
fn travel(distance: i32, progress: u32) -> i32 {
    let scaled = i64::from(distance) * i64::from(progress) / i64::from(PROGRESS_SCALE);
    i32::try_from(scaled).unwrap_or(distance)
}

/// The column and row a one-based cell index falls in.
fn position(widths: &[usize], index: usize) -> Option<(usize, usize)> {
    let mut seen = 0;
    for (column, &height) in widths.iter().enumerate() {
        if index <= seen + height {
            return Some((column, index.checked_sub(seen + 1)?));
        }
        seen += height;
    }
    None
}

/// Interpolate each edge of `from` toward `to`.
fn lerp_rect(from: Rect, to: Rect, progress: u32) -> Rect {
    let left = lerp(from.x, to.x, progress);
    let top = lerp(from.y, to.y, progress);
    let right = lerp(from.right(), to.right(), progress);
    let bottom = lerp(from.bottom(), to.bottom(), progress);
    Rect {
        x:      left,
        y:      top,
        width:  right.saturating_sub(left),
        height: bottom.saturating_sub(top),
    }
}

/// `from` moved toward `to` by `progress`, on integers throughout.
fn lerp(from: u16, to: u16, progress: u32) -> u16 {
    let (from, to) = (u32::from(from), u32::from(to));
    let value = if to >= from {
        from + (to - from) * progress / PROGRESS_SCALE
    } else {
        from - (from - to) * progress / PROGRESS_SCALE
    };
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// Ease progress in and out, so a transition starts and lands gently
/// rather than stopping dead at both ends.
///
/// Half smoothstep, half linear. Smoothstep alone peaks at one and a
/// half times linear speed, and everything it borrows for that middle it
/// takes from the two ends -- which matters here because the grid moves
/// in whole cells. A tail that flat leaves the last cell of travel
/// waiting several frames longer than the ones before it, so the motion
/// reads as stopping and then jumping into place. Averaging with linear
/// pulls the peak down to one and a quarter, and the slowest step ends
/// up around twice the fastest rather than an order of magnitude.
fn eased(progress: u32) -> u32 {
    let scale = u64::from(PROGRESS_SCALE);
    let progress = u64::from(progress.min(PROGRESS_SCALE));
    let smoothstep =
        (3 * progress * progress * scale - 2 * progress * progress * progress) / (scale * scale);
    u32::try_from(u64::midpoint(smoothstep, progress)).unwrap_or(PROGRESS_SCALE)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::layout::Margin;

    use super::*;

    /// Width of the rect the placement tests lay their grids out in.
    const TEST_WIDTH: u16 = 80;
    /// Height of the rect the placement tests lay their grids out in.
    const TEST_HEIGHT: u16 = 40;

    /// The rect the placement tests lay their grids out in.
    const fn test_area() -> Rect { Rect::new(0, 0, TEST_WIDTH, TEST_HEIGHT) }

    /// A tally with one entry per cell of [`test_area`].
    fn blank_tally() -> Vec<u32> { vec![0; usize::from(TEST_WIDTH) * usize::from(TEST_HEIGHT)] }

    /// Count every cell of [`test_area`] that `rect` covers.
    fn paint(tally: &mut [u32], rect: Rect) {
        for y in rect.top()..rect.bottom() {
            for x in rect.left()..rect.right() {
                let index = usize::from(y) * usize::from(TEST_WIDTH) + usize::from(x);
                if let Some(cell) = tally.get_mut(index) {
                    *cell += 1;
                }
            }
        }
    }

    /// Rows per column for every count up to `count`, so a walk through
    /// the sequence reads as one table.
    fn walk(count: usize, initial_rows: usize) -> Vec<Vec<usize>> {
        (1..=count).map(|n| columns(n, initial_rows)).collect()
    }

    #[test]
    fn the_first_column_fills_before_a_second_one_opens() {
        assert_eq!(walk(4, 4), vec![vec![1], vec![2], vec![3], vec![4]]);
    }

    #[test]
    fn a_new_column_opens_at_full_height_with_one_cell() {
        assert_eq!(columns(5, 4), vec![4, 1]);
        assert_eq!(columns(6, 4), vec![4, 2]);
        assert_eq!(columns(8, 4), vec![4, 4]);
        assert_eq!(columns(9, 4), vec![4, 4, 1]);
        assert_eq!(columns(13, 4), vec![4, 4, 4, 1]);
        assert_eq!(columns(16, 4), vec![4, 4, 4, 4]);
    }

    /// The case the two-regime rule exists for: cell 17 does not open a
    /// fifth column, it makes every column a row taller, one column at a
    /// time.
    #[test]
    fn past_the_initial_square_the_grid_grows_a_row_before_a_column() {
        assert_eq!(columns(17, 4), vec![5, 4, 4, 4]);
        assert_eq!(columns(18, 4), vec![5, 5, 4, 4]);
        assert_eq!(columns(19, 4), vec![5, 5, 5, 4]);
        assert_eq!(columns(20, 4), vec![5, 5, 5, 5]);
    }

    #[test]
    fn the_fifth_column_opens_once_every_column_stands_five_tall() {
        assert_eq!(columns(21, 4), vec![5, 5, 5, 5, 1]);
        assert_eq!(columns(25, 4), vec![5, 5, 5, 5, 5]);
    }

    #[test]
    fn the_square_after_five_grows_the_same_way() {
        assert_eq!(columns(26, 4), vec![6, 5, 5, 5, 5]);
        assert_eq!(columns(30, 4), vec![6, 6, 6, 6, 6]);
        assert_eq!(columns(31, 4), vec![6, 6, 6, 6, 6, 1]);
        assert_eq!(columns(36, 4), vec![6, 6, 6, 6, 6, 6]);
    }

    #[test]
    fn every_arrangement_holds_exactly_the_cells_asked_for() {
        for initial_rows in 1..=6 {
            for count in 1..=60 {
                assert_eq!(
                    columns(count, initial_rows).iter().sum::<usize>(),
                    count,
                    "count {count} at {initial_rows} initial rows"
                );
            }
        }
    }

    /// Adding one cell snakes the grid back by exactly one place: at
    /// each boundary between columns the head of the right-hand column
    /// drops to the foot of the one to its left, and nothing else
    /// changes column. That is one move per boundary, never two at the
    /// same one, which is what keeps the motion readable.
    #[test]
    fn one_more_cell_snakes_each_column_boundary_at_most_once() {
        for initial_rows in 1..=6 {
            for count in 1..60 {
                let before = columns(count, initial_rows);
                let after = columns(count + 1, initial_rows);
                let mut left_by = vec![0_usize; before.len().max(after.len())];
                for index in 1..=count {
                    let was = position(&before, index).map(|(column, _)| column);
                    let now = position(&after, index).map(|(column, _)| column);
                    if was == now {
                        continue;
                    }
                    let (Some(was), Some(now)) = (was, now) else {
                        continue;
                    };
                    assert_eq!(
                        was,
                        now + 1,
                        "cell {index} jumped from column {was} to {now} going from {count} to {} \
                         at {initial_rows} initial rows",
                        count + 1
                    );
                    left_by[was] += 1;
                }
                assert!(
                    left_by.iter().all(|moved| *moved <= 1),
                    "more than one cell left the same column going from {count} to {} at \
                     {initial_rows} initial rows",
                    count + 1
                );
            }
        }
    }

    #[test]
    fn initial_rows_below_one_is_treated_as_one() {
        assert_eq!(columns(3, 0), columns(3, 1));
    }

    /// Cells reach every corner of the area and overlap only where they
    /// share a border: the interiors they draw into never collide, but
    /// together the cells leave nothing uncovered.
    #[test]
    fn cells_cover_their_area_and_overlap_only_on_shared_borders() {
        let area = test_area();
        for count in 1..=20 {
            let grid = Grid::new(area, count, 4);
            let mut covered = blank_tally();
            let mut interiors = blank_tally();
            for index in 1..=count {
                let rect = grid.cell(index).expect("cell is in the grid");
                paint(&mut covered, rect);
                paint(&mut interiors, rect.inner(Margin::new(1, 1)));
            }
            assert!(
                covered.iter().all(|hits| *hits >= 1),
                "count {count} leaves a gap"
            );
            assert!(
                interiors.iter().all(|hits| *hits <= 1),
                "count {count} draws two cells into the same place"
            );
        }
    }

    /// A second column starts on the same screen column the first one
    /// ends on. That single shared line is what the grid is drawn from.
    #[test]
    fn a_column_starts_where_the_one_before_it_ends() {
        let grid = Grid::new(test_area(), 5, 4);
        let first = grid.cell(1).expect("cell is in the grid");
        let second = grid.cell(5).expect("cell is in the grid");
        assert_eq!(first.right() - 1, second.left());
    }

    /// A stacked cell starts on the row the one above it ends on.
    #[test]
    fn a_row_starts_where_the_one_above_it_ends() {
        let grid = Grid::new(test_area(), 2, 4);
        let first = grid.cell(1).expect("cell is in the grid");
        let second = grid.cell(2).expect("cell is in the grid");
        assert_eq!(first.bottom() - 1, second.top());
    }

    #[test]
    fn a_settled_grid_places_one_piece_per_cell() {
        let placements = TileGrid::new().placements(test_area(), 4);
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].index, 1);
        assert_eq!(placements[0].frame.rect(), test_area());
    }

    #[test]
    fn the_table_cell_is_never_removed() {
        let mut grid = TileGrid::new();
        grid.remove();
        assert_eq!(grid.count(), 1);
    }

    #[test]
    fn a_grid_with_no_room_refuses_to_grow() {
        let mut grid = TileGrid::new();
        grid.set_area(Rect::new(0, 0, 4, 2));
        grid.add(4);
        assert_eq!(grid.count(), 1);
    }

    /// Cell 5 leaves the top of column two and arrives at the bottom of
    /// column one, so mid-transition it is drawn twice -- once in each.
    #[test]
    fn a_cell_changing_column_is_drawn_in_both() {
        let area = test_area();
        let before = Grid::new(area, 16, 4);
        let after = Grid::new(area, 17, 4);
        let mut out = Vec::new();
        moving_cell(&before, &after, 5, PROGRESS_SCALE / 2, &mut out);

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].frame.clip(), before.column_rect(1));
        assert_eq!(out[1].frame.clip(), after.column_rect(0));
        assert!(out[0].frame.shift() < 0, "the piece leaving travels upward");
        assert!(
            out[1].frame.shift() > 0,
            "the piece arriving is still below"
        );
    }

    #[test]
    fn easing_pins_both_ends() {
        assert_eq!(eased(0), 0);
        assert_eq!(eased(PROGRESS_SCALE), PROGRESS_SCALE);
        assert!(eased(PROGRESS_SCALE / 2).abs_diff(PROGRESS_SCALE / 2) <= 1);
    }
}
