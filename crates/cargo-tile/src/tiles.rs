//! The tile grid: how many cells the pane holds, where each one sits,
//! and the motion from one arrangement to the next.
//!
//! Cells are numbered from one and fill column by column. Cell one is
//! the summary table; each of the rest belongs to one running command,
//! or stands empty waiting for one. [`columns`] is the whole layout rule
//! and is pure, so the arrangement at any count is a test rather than
//! something to squint at on screen.
//!
//! Which cell a command lands in is [`TileGrid::sync`]'s answer, and it
//! is kept apart from the geometry on purpose: cells are identified by
//! the [`Slot`] they hold rather than by their number, so a command
//! finishing in the middle animates every cell after it one place
//! forward instead of shuffling contents between cells that never
//! moved.
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

use std::collections::VecDeque;
use std::time::Instant;

use ratatui::layout::Layout;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::PaneAxisSize;
use tui_pane::PaneFrame;
use tui_pane::ResolvedPane;
use tui_pane::ResolvedPaneLayout;
use tui_pane::constraints_for_sizes;
use tui_pane::share_borders;

use crate::constants::MAX_PENDING_STEPS;
use crate::constants::MIN_INITIAL_ROWS;
use crate::constants::MIN_STEP_MILLIS;
use crate::constants::MIN_TILE_HEIGHT;
use crate::constants::MIN_TILE_WIDTH;
use crate::constants::PROGRESS_SCALE;
use crate::constants::TABLE_CELL;
use crate::constants::TILE_ANIMATION_MILLIS;

/// What a cell is showing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TileContent {
    /// The summary table, which every grid opens with.
    Summary,
    /// One command's tile, keyed by the group whose rows it draws.
    Group(u32),
    /// A cell opened with `+` that no command has claimed, carrying the
    /// number it currently sits at.
    Empty(usize),
    /// The hole a finished command left, on its way out of the grid. It
    /// draws nothing: what the eye follows is the cells trading places
    /// with it, one step at a time, until it reaches the end.
    Gap,
}

/// One cell as a single frame should draw it.
///
/// A cell crossing between columns is drawn as two of these -- the piece
/// leaving the old column and the piece arriving in the new one -- which
/// is what makes it read as sliding off one column's edge and back in at
/// the next.
pub(crate) struct Placement {
    /// What the cell draws.
    pub(crate) content: TileContent,
    /// Where the cell's box sits this frame, and how far it is cut off
    /// -- the framework's own account of a moving pane, which both
    /// [`tui_pane::draw_clipped`] and [`tui_pane::GridLines`] read.
    pub(crate) frame:   PaneFrame,
}

/// A cell after the summary.
///
/// The identity here is what makes the motion work. A cell is animated
/// from where its slot stood to where the same slot stands now, so a
/// command finishing in the middle of the grid draws every cell after it
/// travelling one place forward, rather than the contents jumping
/// between cells that stayed put.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Slot {
    /// The tile drawing one command's rows.
    Group(u32),
    /// A cell opened with `+` and not yet claimed. The number is only
    /// an identity -- it is never shown, and no two empties share one --
    /// so an empty cell can be told from its neighbour while the grid
    /// closes up around them.
    Empty(u64),
    /// The hole a finished command left. It walks to the end of the
    /// grid a neighbour at a time and is taken off there, which is how
    /// closing one cell in the middle reads as a single cell moving
    /// rather than as every cell after it sliding at once.
    Gap(u64),
}

/// The cell holding focus.
///
/// Focus is held by identity rather than by cell number for the same
/// reason the cells are: the command a developer is watching keeps the
/// ring as the grid closes up around it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Focus {
    /// The summary, where focus starts and where it falls back to.
    Summary,
    /// One of the cells after it.
    Cell(Slot),
}

/// One step of the focus ring, named for the arrow that asks for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Direction {
    /// Toward the column on the left.
    Left,
    /// Toward the column on the right.
    Right,
    /// Toward the cell above, within one column.
    Up,
    /// Toward the cell below, within one column.
    Down,
}

/// What a cell draws and whether it holds focus -- everything a
/// [`Placement`] carries that is not geometry.
#[derive(Clone, Copy)]
struct Drawn {
    /// What the cell draws.
    content: TileContent,
    /// Whether the cell holds focus, which is what lights its border.
    focused: bool,
}

impl Drawn {
    /// This cell drawn at `frame`, focus carried onto it.
    const fn at(self, frame: PaneFrame) -> Placement {
        Placement {
            content: self.content,
            frame:   frame.with_focus(self.focused),
        }
    }
}

/// The arrangement a transition is moving away from.
struct Transition {
    /// The cells as they stood before the change.
    from:    Vec<Slot>,
    /// When the motion began.
    started: Instant,
    /// How long this one step runs for.
    millis:  u64,
}

/// One arrangement waiting its turn, and how long the move into it
/// takes once it comes up.
struct Step {
    /// The cells as this step leaves them.
    slots:  Vec<Slot>,
    /// How long the move into `slots` runs for.
    millis: u64,
}

/// The grid's cells and the transition it is playing, if any.
pub(crate) struct TileGrid {
    /// Cells after the summary, in cell order. The summary is cell one
    /// and is not held here, so the grid never falls below one cell.
    slots:        Vec<Slot>,
    /// Arrangements waiting their turn, each one cell's move from the
    /// one before it. Only ever one of them is in flight, which is what
    /// makes a change propagate through the grid instead of happening
    /// to every cell at once.
    pending:      VecDeque<Step>,
    /// The motion in flight, or `None` once the grid has settled.
    transition:   Option<Transition>,
    /// The rect the last frame laid out, so [`Self::add`] can tell
    /// whether the cells it would create still fit on screen.
    area:         Rect,
    /// Rows the first column opens with, as the last frame had it. Held
    /// so a mouse click can resolve the same geometry the frame drew
    /// without the caller carrying the setting to every hit test.
    initial_rows: usize,
    /// Identity for the next cell opened or emptied.
    next_slot:    u64,
    /// The cell the focus ring is on.
    focus:        Focus,
}

impl TileGrid {
    /// A grid holding nothing but the summary.
    pub(crate) const fn new() -> Self {
        Self {
            slots:        Vec::new(),
            pending:      VecDeque::new(),
            transition:   None,
            area:         Rect::ZERO,
            initial_rows: 0,
            next_slot:    0,
            focus:        Focus::Summary,
        }
    }

    /// Cells the grid holds, the summary included.
    const fn count(&self) -> usize { self.slots.len() + TABLE_CELL }

    /// Record the geometry the pane just laid out.
    pub(crate) const fn set_layout(&mut self, area: Rect, initial_rows: usize) {
        self.area = area;
        self.initial_rows = initial_rows;
    }

    /// Take the grid to its next step once the one in flight has run
    /// its course, reporting whether it still wants repainting.
    ///
    /// One step at a time, always: a change that takes several of them
    /// propagates through the grid like a snake instead of every cell
    /// sliding past every other one at once.
    pub(crate) fn tick(&mut self) -> bool {
        if self.transition.is_none() {
            return false;
        }
        if self.progress() >= PROGRESS_SCALE {
            self.advance();
        }
        true
    }

    /// Give every running command a cell and take back the cells whose
    /// command has gone.
    ///
    /// A command arriving takes the first cell `+` opened and nothing has
    /// claimed, so a developer who made room in advance sees the build
    /// land in it. With no empty cell waiting it opens one of its own at
    /// the end. A command leaving closes its cell wherever that sat, and
    /// the cells after it move up into the hole one at a time.
    ///
    /// The comparison is against where the grid is *headed* rather than
    /// where it stands, so a scan arriving mid-ripple adds to the queue
    /// instead of starting the same change over.
    pub(crate) fn sync(&mut self, ids: &[u32], initial_rows: usize) {
        let mut arrangement = self.target();
        let mut steps: Vec<Vec<Slot>> = Vec::new();
        while let Some(index) = arrangement.iter().position(|slot| match *slot {
            Slot::Group(id) => !ids.contains(&id),
            Slot::Empty(_) | Slot::Gap(_) => false,
        }) {
            self.close(&mut arrangement, index, &mut steps);
        }
        for &id in ids {
            if arrangement.contains(&Slot::Group(id)) {
                continue;
            }
            match arrangement
                .iter()
                .position(|slot| matches!(*slot, Slot::Empty(_)))
            {
                Some(index) => {
                    arrangement[index] = Slot::Group(id);
                    steps.push(arrangement.clone());
                },
                // Refusing beats a cell too small to read: the command
                // is still in the summary, which is never crowded out.
                None if self.fits(arrangement.len() + TABLE_CELL + 1, initial_rows) => {
                    arrangement.push(Slot::Group(id));
                    steps.push(arrangement.clone());
                },
                None => (),
            }
        }
        self.queue(steps);
    }

    /// Push the steps that close the cell at `index`.
    ///
    /// The cell empties where it stands, and the hole it leaves trades
    /// places with the cell after it, then the one after that, until it
    /// reaches the end of the grid and is taken off. That is what makes
    /// a command finishing in the middle read as one cell moving at a
    /// time rather than as the whole tail sliding forward at once.
    fn close(&mut self, arrangement: &mut Vec<Slot>, index: usize, steps: &mut Vec<Vec<Slot>>) {
        arrangement[index] = Slot::Gap(self.next_slot);
        self.next_slot = self.next_slot.saturating_add(1);
        steps.push(arrangement.clone());
        for position in index..arrangement.len().saturating_sub(1) {
            arrangement.swap(position, position + 1);
            steps.push(arrangement.clone());
        }
        let _ = arrangement.pop();
        steps.push(arrangement.clone());
    }

    /// Open an empty cell at the end, unless the grid it would make no
    /// longer fits.
    ///
    /// Refusing beats filling the pane with cells too small to carry a
    /// border and a row: the grid stops growing at the point the
    /// terminal stops being able to show it.
    pub(crate) fn add(&mut self, initial_rows: usize) {
        let mut arrangement = self.target();
        if !self.fits(arrangement.len() + TABLE_CELL + 1, initial_rows) {
            return;
        }
        arrangement.push(Slot::Empty(self.next_slot));
        self.next_slot = self.next_slot.saturating_add(1);
        self.queue(vec![arrangement]);
    }

    /// Close an empty cell: the focused one when focus is on one,
    /// otherwise the last one `+` opened.
    ///
    /// Only an empty cell goes. The cells carrying commands are the
    /// display itself, so `-` undoes `+` rather than hiding a running
    /// build, and the summary is never removable at all. Falling back to
    /// the last empty cell is what keeps `+` and `-` a pair from the
    /// summary, where focus starts and where it returns.
    ///
    /// The cell leaves the way a finished command's does, through
    /// [`Self::close`], so taking one out of the middle ripples rather
    /// than snapping the tail forward.
    pub(crate) fn remove(&mut self) {
        let mut arrangement = self.target();
        let Some(index) = self.removable(&arrangement) else {
            return;
        };
        let mut steps = Vec::new();
        self.close(&mut arrangement, index, &mut steps);
        self.queue(steps);
    }

    /// Which cell `-` takes out of `arrangement`, or `None` when it
    /// holds no empty one.
    fn removable(&self, arrangement: &[Slot]) -> Option<usize> {
        if let Focus::Cell(slot @ Slot::Empty(_)) = self.focus
            && let Some(index) = arrangement.iter().position(|held| *held == slot)
        {
            return Some(index);
        }
        arrangement
            .iter()
            .rposition(|slot| matches!(*slot, Slot::Empty(_)))
    }

    /// The arrangement the grid is headed for: the last step queued, or
    /// what it is showing when nothing is queued.
    fn target(&self) -> Vec<Slot> {
        self.pending
            .back()
            .map_or_else(|| self.slots.clone(), |step| step.slots.clone())
    }

    /// Queue `steps`, starting the first one when nothing is in flight.
    ///
    /// One change is spread over [`TILE_ANIMATION_MILLIS`] however many
    /// steps it takes, down to [`MIN_STEP_MILLIS`] a step, so a single
    /// cell moving keeps the full unhurried travel and a long ripple
    /// runs at one steady pace instead of dragging. Past
    /// [`MAX_PENDING_STEPS`] the grid gives the ripple up and settles
    /// the rest in one move: a whole suite finishing at once would take
    /// longer to walk through than anyone would watch.
    fn queue(&mut self, steps: Vec<Vec<Slot>>) {
        if steps.is_empty() {
            return;
        }
        let millis = u64::try_from(steps.len())
            .ok()
            .and_then(|count| TILE_ANIMATION_MILLIS.checked_div(count))
            .unwrap_or(TILE_ANIMATION_MILLIS)
            .max(MIN_STEP_MILLIS);
        self.pending
            .extend(steps.into_iter().map(|slots| Step { slots, millis }));
        if self.pending.len() > MAX_PENDING_STEPS
            && let Some(last) = self.pending.pop_back()
        {
            self.pending.clear();
            self.pending.push_back(Step {
                millis: TILE_ANIMATION_MILLIS,
                ..last
            });
        }
        if self.transition.is_none() {
            self.advance();
        }
    }

    /// Move into the next queued step, or settle when there is none.
    fn advance(&mut self) {
        let Some(step) = self.pending.pop_front() else {
            self.transition = None;
            return;
        };
        let previous = std::mem::replace(&mut self.slots, step.slots);
        self.settle_focus(&previous);
        self.transition = Some(Transition {
            from:    previous,
            started: Instant::now(),
            millis:  step.millis,
        });
    }

    /// Keep the ring on a cell that is still there and hand it back to
    /// the summary when it is not.
    ///
    /// The one cell that changes identity without going anywhere is an
    /// empty one a command has just claimed. The developer opened that
    /// cell to watch for exactly this, so the ring stays on it; every
    /// other way a slot can vanish is the cell itself leaving.
    fn settle_focus(&mut self, previous: &[Slot]) {
        let Focus::Cell(slot) = self.focus else {
            return;
        };
        if self.slots.contains(&slot) {
            return;
        }
        self.focus = previous
            .iter()
            .position(|held| *held == slot)
            .and_then(|index| self.slots.get(index))
            .filter(|taken| claims(slot, **taken))
            .map_or(Focus::Summary, |&taken| Focus::Cell(taken));
    }

    /// Play every queued step at once, for tests that care where the
    /// grid ends up rather than how it gets there.
    #[cfg(test)]
    fn settle(&mut self) {
        while !self.pending.is_empty() {
            self.advance();
        }
        self.transition = None;
    }

    /// Move focus one cell in `direction`, staying put at the edges.
    ///
    /// The grid is ragged -- the first column can stand a row taller
    /// than the last -- so a sideways step keeps the row it can and
    /// lands on the bottom cell of a shorter column rather than
    /// refusing to move at all.
    pub(crate) fn focus_step(&mut self, direction: Direction, initial_rows: usize) {
        let widths = columns(self.count(), initial_rows);
        let Some((column, row)) = self
            .focused_cell()
            .and_then(|index| position(&widths, index))
        else {
            return;
        };
        let last = widths.len().saturating_sub(1);
        let (column, row) = match direction {
            Direction::Left => (column.saturating_sub(1), row),
            Direction::Right => (column.saturating_add(1).min(last), row),
            Direction::Up => (column, row.saturating_sub(1)),
            Direction::Down => (column, row.saturating_add(1)),
        };
        let Some(&height) = widths.get(column) else {
            return;
        };
        let row = row.min(height.saturating_sub(1));
        let cell = widths
            .iter()
            .take(column)
            .sum::<usize>()
            .saturating_add(row)
            .saturating_add(TABLE_CELL);
        self.focus = self.focus_at(cell);
    }

    /// The cell number focus rests on, or `None` while the focused slot
    /// is on its way out of the grid.
    fn focused_cell(&self) -> Option<usize> {
        match self.focus {
            Focus::Summary => Some(TABLE_CELL),
            Focus::Cell(slot) => cell_of(&self.slots, slot),
        }
    }

    /// What focus becomes on landing at cell `index`, left where it is
    /// when the grid has no such cell.
    fn focus_at(&self, index: usize) -> Focus {
        if index == TABLE_CELL {
            return Focus::Summary;
        }
        index
            .checked_sub(TABLE_CELL + 1)
            .and_then(|position| self.slots.get(position))
            .filter(|slot| !matches!(slot, Slot::Gap(_)))
            .map_or(self.focus, |&slot| Focus::Cell(slot))
    }

    /// Put focus on cell `index`, leaving it where it is when the grid
    /// has no such cell.
    pub(crate) fn focus_cell(&mut self, index: usize) { self.focus = self.focus_at(index); }

    /// The cell `pos` lands in, or `None` when the point is outside the
    /// grid.
    ///
    /// Answered against the settled grid rather than against a
    /// transition in flight: a cell mid-travel is a transient, and
    /// clicking one is asking for where it is going.
    pub(crate) fn cell_at(&self, pos: Position) -> Option<usize> {
        let grid = Grid::new(self.area, self.count(), self.initial_rows);
        grid.resolved
            .panes
            .iter()
            .find(|resolved| resolved.area.contains(pos))
            .map(|resolved| resolved.pane)
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
        let total = u128::from(transition.millis);
        if total == 0 || elapsed >= total {
            return PROGRESS_SCALE;
        }
        u32::try_from(elapsed * u128::from(PROGRESS_SCALE) / total).unwrap_or(PROGRESS_SCALE)
    }

    /// Every piece to draw this frame, in cell order.
    pub(crate) fn placements(&self, area: Rect, initial_rows: usize) -> Vec<Placement> {
        let settled = Grid::new(area, self.count(), initial_rows);
        let focused = self.focused_cell();
        let Some(transition) = self.transition.as_ref() else {
            return cells(&self.slots)
                .into_iter()
                .filter_map(|(content, index)| {
                    Some(Placement {
                        content,
                        frame: PaneFrame::new(settled.cell(index)?)
                            .with_focus(focused == Some(index)),
                    })
                })
                .collect();
        };

        let progress = eased(self.progress());
        let before = Grid::new(area, transition.from.len() + TABLE_CELL, initial_rows);
        let mut placements = Vec::new();
        // The summary keeps cell one throughout, but the grid around it
        // resizes, so it still has somewhere to travel.
        moving_cell(
            &before,
            &settled,
            (Some(TABLE_CELL), Some(TABLE_CELL)),
            progress,
            Drawn {
                content: TileContent::Summary,
                focused: self.focus == Focus::Summary,
            },
            &mut placements,
        );
        for slot in union(&transition.from, &self.slots) {
            // A slot handed over in place is not a cell arriving and a
            // cell leaving, however it looks in the arrangement: the
            // cell stands where it stood and only its contents changed.
            // Taken at face value the pair would draw the old one
            // collapsing while a new one rose out of the column floor to
            // meet it, which is motion the grid never made.
            let (old, new) = match (cell_of(&transition.from, slot), cell_of(&self.slots, slot)) {
                (None, Some(index)) if before.cell(index).is_some() => (Some(index), Some(index)),
                (Some(index), None) if settled.cell(index).is_some() => continue,
                pair => pair,
            };
            let content = content_of(slot, new.or(old).unwrap_or(TABLE_CELL));
            moving_cell(
                &before,
                &settled,
                (old, new),
                progress,
                Drawn {
                    content,
                    focused: self.focus == Focus::Cell(slot),
                },
                &mut placements,
            );
        }
        placements
    }
}

/// What each slot draws and the cell it sits at, the summary first.
fn cells(slots: &[Slot]) -> Vec<(TileContent, usize)> {
    let mut out = vec![(TileContent::Summary, TABLE_CELL)];
    out.extend(slots.iter().enumerate().map(|(position, &slot)| {
        let index = position + TABLE_CELL + 1;
        (content_of(slot, index), index)
    }));
    out
}

/// The cell `slot` sits at, or `None` when this arrangement has no such
/// slot.
fn cell_of(slots: &[Slot], slot: Slot) -> Option<usize> {
    slots
        .iter()
        .position(|held| *held == slot)
        .map(|position| position + TABLE_CELL + 1)
}

/// What a slot draws once it knows the cell it landed at.
const fn content_of(slot: Slot, index: usize) -> TileContent {
    match slot {
        Slot::Group(id) => TileContent::Group(id),
        Slot::Empty(_) => TileContent::Empty(index),
        Slot::Gap(_) => TileContent::Gap,
    }
}

/// Whether `taken` is a command landing in the empty cell `held` stood
/// for, which is the one way a slot changes identity without moving.
const fn claims(held: Slot, taken: Slot) -> bool {
    matches!((held, taken), (Slot::Empty(_), Slot::Group(_)))
}

/// Every slot either arrangement holds, the ones that survive first so
/// the pieces come out roughly in cell order.
fn union(from: &[Slot], to: &[Slot]) -> Vec<Slot> {
    let mut out = to.to_vec();
    out.extend(from.iter().copied().filter(|slot| !to.contains(slot)));
    out
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
///
/// The two cell numbers are read separately because a cell keeps its
/// slot but not its number: closing a cell in the middle of the grid
/// moves every cell after it one place forward, and animating that means
/// travelling from the old number's rect to the new number's.
fn moving_cell(
    before: &Grid,
    after: &Grid,
    (old, new): (Option<usize>, Option<usize>),
    progress: u32,
    drawn: Drawn,
    out: &mut Vec<Placement>,
) {
    let was = old.and_then(|index| Some((index, before.cell(index)?)));
    let is = new.and_then(|index| Some((index, after.cell(index)?)));
    match (was, is) {
        (Some((old_index, from)), Some((new_index, to))) => {
            let columns = (before.column_of(old_index), after.column_of(new_index));
            if columns.0 == columns.1 {
                out.push(drawn.at(PaneFrame::new(lerp_rect(from, to, progress))));
                return;
            }
            let columns = (columns.0.unwrap_or_default(), columns.1.unwrap_or_default());
            wrapping_cell(before, after, columns, progress, (from, to), drawn, out);
        },
        // A cell that has just appeared. It grows in from the edge its
        // column came from, so a new row rises out of the floor and a
        // new column arrives from the right.
        (None, Some((new_index, to))) => out.push(drawn.at(PaneFrame::new(lerp_rect(
            edge_rect(before, after, new_index, to),
            to,
            progress,
        )))),
        // A cell on its way out, closing onto the edge whatever
        // takes its place expands over.
        (Some((old_index, from)), None) => out.push(drawn.at(PaneFrame::new(lerp_rect(
            from,
            closing_rect(before, after, old_index, from),
            progress,
        )))),
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
    (from_column, to_column): (usize, usize),
    progress: u32,
    (from, to): (Rect, Rect),
    drawn: Drawn,
    out: &mut Vec<Placement>,
) {
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

    out.push(drawn.at(PaneFrame::shifted(from, exit, leaving)));
    out.push(drawn.at(PaneFrame::shifted(to, entry, arriving)));
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

/// The collapsed rect a cell on its way out shrinks into.
///
/// A departing cell is only ever drawn when nothing takes the number it
/// held -- a slot handed over in place is skipped by
/// [`TileGrid::placements`] -- so it always sits past the end of the
/// grid it is leaving, at the foot of the last column. Which of that
/// column's edges it closes onto is the whole question, and it is
/// [`edge_rect`] run backwards either way.
///
/// A cell whose column goes with it is squeezed off the right edge with
/// no width. The columns to its left widen into the space as it goes,
/// so its left edge travels alongside their right one and the pair read
/// as a single line sweeping the column away; closing it downward
/// instead left one line sliding down while another slid sideways,
/// which is two movements for one cell leaving.
///
/// A cell whose column stays closes onto that column's floor with no
/// height, and the cells above simply expand down onto it.
fn closing_rect(before: &Grid, after: &Grid, index: usize, from: Rect) -> Rect {
    let Some(column) = before.column_of(index) else {
        return Rect {
            y: from.bottom(),
            height: 0,
            ..from
        };
    };
    if column >= after.widths.len() {
        return Rect {
            x: after.area.right(),
            width: 0,
            ..from
        };
    }
    Rect {
        y: before.column_rect(column).bottom(),
        height: 0,
        ..from
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
        assert_eq!(placements[0].content, TileContent::Summary);
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
        grid.set_layout(Rect::new(0, 0, 4, 2), 4);
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
        moving_cell(
            &before,
            &after,
            (Some(5), Some(5)),
            PROGRESS_SCALE / 2,
            Drawn {
                content: TileContent::Empty(5),
                focused: false,
            },
            &mut out,
        );

        assert_eq!(out.len(), 2);
        assert_eq!(out[0].frame.clip(), before.column_rect(1));
        assert_eq!(out[1].frame.clip(), after.column_rect(0));
        assert!(out[0].frame.shift() < 0, "the piece leaving travels upward");
        assert!(
            out[1].frame.shift() > 0,
            "the piece arriving is still below"
        );
    }

    /// Closing the one cell a column holds takes the column with it, so
    /// the cell is squeezed off the right edge by the columns widening
    /// behind it -- one line sweeping sideways, not a second one
    /// sliding down at the same time.
    #[test]
    fn a_cell_taking_its_column_with_it_closes_off_the_right_edge() {
        let area = test_area();
        let before = Grid::new(area, 9, 4);
        let after = Grid::new(area, 8, 4);
        assert_eq!(before.widths, vec![4, 4, 1], "cell nine is a column of one");
        let from = before.cell(9).expect("cell nine is in the grid");

        let closed = closing_rect(&before, &after, 9, from);
        assert_eq!(closed.width, 0, "it closes to nothing");
        assert_eq!(closed.x, area.right(), "against the right edge");
        assert_eq!(
            (closed.y, closed.height),
            (from.y, from.height),
            "and travels nowhere vertically"
        );
    }

    /// Closing the last cell of a column the grid keeps drops it onto
    /// that column's floor, which the cell above expands down onto.
    #[test]
    fn a_cell_leaving_a_column_that_stays_closes_onto_its_floor() {
        let area = test_area();
        let before = Grid::new(area, 6, 4);
        let after = Grid::new(area, 5, 4);
        assert_eq!(before.widths, vec![4, 2], "cell six sits under cell five");
        let from = before.cell(6).expect("cell six is in the grid");

        let closed = closing_rect(&before, &after, 6, from);
        assert_eq!(closed.height, 0, "it closes to nothing");
        assert_eq!(
            closed.y,
            before.column_rect(1).bottom(),
            "against the column floor"
        );
        assert_eq!(
            (closed.x, closed.width),
            (from.x, from.width),
            "and travels nowhere sideways"
        );
    }

    /// A grid the tests can grow without a terminal under it.
    fn seeded_grid() -> TileGrid {
        let mut grid = TileGrid::new();
        grid.set_layout(test_area(), 4);
        grid
    }

    /// What each cell after the summary is showing, settled.
    fn shown(grid: &TileGrid) -> Vec<TileContent> {
        cells(&grid.slots)
            .into_iter()
            .skip(1)
            .map(|(content, _)| content)
            .collect()
    }

    #[test]
    fn a_command_arriving_opens_its_own_cell() {
        let mut grid = seeded_grid();
        grid.sync(&[7], 4);
        grid.settle();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    /// A developer who pressed `+` made room deliberately, so the next
    /// build lands in it rather than opening another cell beside it.
    #[test]
    fn a_command_arriving_takes_the_first_empty_cell() {
        let mut grid = seeded_grid();
        grid.add(4);
        grid.add(4);
        grid.sync(&[7], 4);
        grid.settle();

        assert_eq!(shown(&grid).len(), 2, "no third cell opened");
        assert_eq!(shown(&grid)[0], TileContent::Group(7));
        assert_eq!(shown(&grid)[1], TileContent::Empty(3));
    }

    /// The cells after a departed one each move one place forward, which
    /// is what the ripple animates them doing, one at a time.
    #[test]
    fn a_command_leaving_the_middle_moves_the_rest_forward() {
        let mut grid = seeded_grid();
        grid.sync(&[7, 8, 9], 4);
        grid.settle();
        grid.sync(&[7, 9], 4);
        grid.settle();

        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7), TileContent::Group(9)]
        );
    }

    /// A command leaving the middle is one step per cell between it and
    /// the end, and one more to close the grid up -- never one step
    /// moving all of them.
    #[test]
    fn a_command_leaving_hands_its_cell_back_one_step_at_a_time() {
        let mut grid = seeded_grid();
        grid.sync(&[7, 8, 9], 4);
        grid.settle();

        grid.sync(&[7, 9], 4);
        assert_eq!(
            shown(&grid),
            vec![
                TileContent::Group(7),
                TileContent::Gap,
                TileContent::Group(9)
            ],
            "the cell empties where it stood"
        );
        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![
                TileContent::Group(7),
                TileContent::Group(9),
                TileContent::Gap
            ],
            "the cell after it moves up into the hole"
        );
        grid.advance();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7), TileContent::Group(9)],
            "and the grid closes up behind it"
        );
    }

    #[test]
    fn a_cell_travels_from_the_number_it_held_to_the_one_it_takes() {
        let mut grid = seeded_grid();
        grid.sync(&[7, 8, 9], 4);
        grid.settle();
        // The first step only empties cell three; the surviving command
        // travels on the second, which is the one under test.
        grid.sync(&[7, 9], 4);
        grid.advance();

        let moved = grid
            .placements(test_area(), 4)
            .into_iter()
            .find(|placement| placement.content == TileContent::Group(9))
            .expect("the surviving command is still drawn");
        let before = Grid::new(test_area(), 4, 4);
        assert_eq!(
            moved.frame.rect(),
            before.cell(4).expect("cell four exists"),
            "it starts where it stood, cell four, and animates to cell three"
        );
    }

    #[test]
    fn removing_refuses_to_close_a_cell_holding_a_command() {
        let mut grid = seeded_grid();
        grid.sync(&[7], 4);
        grid.settle();
        grid.remove();
        grid.settle();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    #[test]
    fn removing_closes_the_last_cell_that_plus_opened() {
        let mut grid = seeded_grid();
        grid.sync(&[7], 4);
        grid.add(4);
        grid.remove();
        grid.settle();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    #[test]
    fn the_summary_holds_focus_to_begin_with() {
        assert_eq!(TileGrid::new().focus, Focus::Summary);
    }

    #[test]
    fn focus_walks_the_grid_and_stops_at_its_edges() {
        let mut grid = seeded_grid();
        grid.sync(&[7, 8], 4);
        grid.settle();

        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focused_cell(), Some(2));
        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focused_cell(), Some(3));
        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focused_cell(), Some(3), "the last cell is the floor");
        grid.focus_step(Direction::Up, 4);
        grid.focus_step(Direction::Up, 4);
        assert_eq!(grid.focus, Focus::Summary, "and the summary is the ceiling");
    }

    /// Focus is held by identity, so the cell it is on keeps it while
    /// the grid closes up around it.
    #[test]
    fn focus_rides_a_cell_through_a_ripple() {
        let mut grid = seeded_grid();
        grid.sync(&[7, 8, 9], 4);
        grid.settle();
        grid.focus_step(Direction::Down, 4);
        grid.focus_step(Direction::Down, 4);
        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(9)));

        grid.sync(&[7, 9], 4);
        grid.settle();
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(9)));
        assert_eq!(grid.focused_cell(), Some(3), "one place forward");
    }

    #[test]
    fn focus_falls_back_to_the_summary_when_its_command_ends() {
        let mut grid = seeded_grid();
        grid.sync(&[7], 4);
        grid.settle();
        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

        grid.sync(&[], 4);
        grid.settle();
        assert_eq!(grid.focus, Focus::Summary);
    }

    /// The developer opened the cell to watch for exactly this, so the
    /// ring stays on it when a command lands there.
    #[test]
    fn focus_stays_on_an_empty_cell_a_command_claims() {
        let mut grid = seeded_grid();
        grid.add(4);
        grid.settle();
        grid.focus_step(Direction::Down, 4);
        assert!(matches!(grid.focus, Focus::Cell(Slot::Empty(_))));

        grid.sync(&[7], 4);
        grid.settle();
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
    }

    /// A click is the other way onto a cell, and lands on the same ring
    /// the arrows walk.
    #[test]
    fn a_click_inside_a_cell_takes_the_focus_ring() {
        let mut grid = seeded_grid();
        grid.sync(&[7, 8], 4);
        grid.settle();

        let second = Grid::new(test_area(), grid.count(), 4)
            .cell(2)
            .expect("the grid has three cells");
        let inside = Position::new(second.x + second.width / 2, second.y + second.height / 2);
        let index = grid.cell_at(inside).expect("the point is inside cell two");
        grid.focus_cell(index);

        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));
    }

    #[test]
    fn a_click_outside_the_grid_finds_no_cell() {
        let grid = seeded_grid();
        assert_eq!(grid.cell_at(Position::new(TEST_WIDTH, TEST_HEIGHT)), None);
    }

    /// `-` takes out the cell the ring is on when `+` opened it, rather
    /// than the last one opened.
    #[test]
    fn minus_takes_out_the_focused_empty_cell() {
        let mut grid = seeded_grid();
        grid.add(4);
        grid.add(4);
        grid.settle();
        let first = grid.slots[0];
        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focus, Focus::Cell(first));

        grid.remove();
        grid.settle();
        assert_eq!(grid.slots.len(), 1, "one cell went");
        assert_ne!(grid.slots[0], first, "and it was the focused one");
        assert_eq!(grid.focus, Focus::Summary, "the ring falls back");
    }

    /// A cell carrying a command is the display itself, so `-` leaves it
    /// where it is and takes the last empty cell instead.
    #[test]
    fn minus_leaves_a_running_command_alone() {
        let mut grid = seeded_grid();
        grid.sync(&[7], 4);
        grid.add(4);
        grid.settle();
        grid.focus_step(Direction::Down, 4);
        assert_eq!(grid.focus, Focus::Cell(Slot::Group(7)));

        grid.remove();
        grid.settle();
        assert_eq!(
            shown(&grid),
            vec![TileContent::Group(7)],
            "the command keeps its cell and the empty one goes"
        );
    }

    #[test]
    fn minus_with_no_empty_cell_changes_nothing() {
        let mut grid = seeded_grid();
        grid.sync(&[7], 4);
        grid.settle();

        grid.remove();
        grid.settle();
        assert_eq!(shown(&grid), vec![TileContent::Group(7)]);
    }

    #[test]
    fn easing_pins_both_ends() {
        assert_eq!(eased(0), 0);
        assert_eq!(eased(PROGRESS_SCALE), PROGRESS_SCALE);
        assert!(eased(PROGRESS_SCALE / 2).abs_diff(PROGRESS_SCALE / 2) <= 1);
    }
}
