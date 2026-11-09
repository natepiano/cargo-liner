#[cfg(test)]
#[path = "allocation_tests.rs"]
mod tests;

use super::BinaryHeap;
use super::CellRowDemand;
use super::ColumnFocus;
use super::ColumnPlan;
use super::Debug;
use super::FocusLocation;
use super::MIN_INITIAL_ROWS;
use super::NonZeroU16;
use super::OnceCell;
use super::Ordering;
use super::PaneAxisSize;
use super::Rect;
use super::RefCell;
use super::Reverse;
use super::RowUsePurpose;
use super::SUMMARY_SLOT_SHORTFALL_DIVISOR;
use super::Slot;
use super::TABLE_CELL;
use super::TileDemands;
use super::TileFill;
use super::TileGrowth;
use super::TileSettings;
use super::column_layout;
/// Rows in each column, left to right, for a grid of `count` cells.
///
/// Two regimes. Up to `growth.initial_rows` the grid is a single
/// column, one cell taller each time: three things on a display read
/// down one column rather than spread across two. Past that the grid
/// opens as many columns as the smallest square holding the cells
/// needs -- `ceil_sqrt` rows to a column, filled a column at a time --
/// and `growth.fill` says how the cells spread over them.
///
/// Crossing into the second regime rearranges what is already on the
/// screen rather than appending to it. At three initial rows the fourth
/// cell turns a column of three into a two-by-two, and the tenth turns
/// a full three-by-three into three columns of four. Between those
/// points [`TileFill::AddNew`] moves nothing at all: the cell arriving
/// lands at the foot of the last column, or opens the next one alone.
/// [`TileFill::Redistribute`] keeps the columns within one cell of each
/// other instead, the taller ones first: the thirteenth cell at three
/// initial rows opens a fourth column and deals the thirteen out as
/// four, three, three and three rather than standing in the fourth
/// column alone.
///
/// Nothing here is remembered, so the grid comes back down through the
/// arrangements it went up through -- take the fourth cell away at
/// three initial rows and the two-by-two is a column of three again.
pub(super) fn columns(count: usize, growth: TileGrowth) -> Vec<usize> {
    let rows = growth.initial_rows.max(MIN_INITIAL_ROWS);
    if count == 0 {
        return Vec::new();
    }
    // The height every column but the last stands at under `AddNew`:
    // the whole grid while it is still growing to `initial_rows`, the
    // side of the smallest square holding it once it is past them.
    let tallest = if count <= rows {
        count
    } else {
        ceil_sqrt(count)
    };
    let opened = count.div_ceil(tallest);
    match growth.fill {
        TileFill::AddNew => {
            let filled = opened.saturating_sub(1);
            let mut widths = vec![tallest; filled];
            widths.push(count - tallest * filled);
            widths
        },
        TileFill::Redistribute => {
            let (shortest, taller) = (count / opened, count % opened);
            (0..opened)
                .map(|column| shortest + usize::from(column < taller))
                .collect()
        },
    }
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
pub(super) fn fills(count: usize) -> Vec<PaneAxisSize> { vec![PaneAxisSize::Fill(1); count] }

/// What occupies the first piece of a divided column.
#[derive(Clone, Copy)]
pub(super) enum ColumnHead {
    Summary(usize),
    Cell,
}

/// The focus ring's row within one column.
pub(super) fn column_focus(
    focused: FocusLocation,
    first_cell: usize,
    pieces: usize,
) -> ColumnFocus {
    match focused {
        FocusLocation::Cell(cell) => cell
            .checked_sub(first_cell)
            .filter(|at| *at < pieces)
            .map_or(ColumnFocus::Outside, ColumnFocus::Row),
        FocusLocation::Departing => ColumnFocus::Outside,
    }
}

/// One column's cells as axis lengths: the even division, then room
/// moved off the cells that are not using theirs and onto the cells
/// that have run out of it.
///
/// The even division is the starting point rather than the answer, and
/// a cell only gives room up when another cell is actually short of it.
/// A column of cells all asking for less than their share is divided
/// evenly, because nothing in it is asking for the room going spare --
/// which is what keeps a cell that grew standing where it grew until
/// something else needs the space.
///
/// A cell holds on to what its contents need and hands over the rest,
/// so a summary showing six rows in a column of eighty gives up the
/// other seventy-odd rather than half its share. `floor` -- the grid's
/// [`TileSettings::min_tile_height`] -- is the one floor: below it a
/// cell has stopped reading as a cell at all.
/// There is no ceiling -- a cell can only ever take what its neighbours
/// are not using, and they are not using it precisely because they do
/// not need it.
///
/// Under [`ColumnHead::Summary`] the summary's shortfall is served first
/// out of the room going spare, so the depth it needs never depends on
/// where the focus is.
///
/// `focused` names the cell holding the focus ring, which is served
/// next out of what is left. That settles nothing while there is enough
/// to go round -- every short cell is filled either way -- and decides
/// it where there is not: the cell being watched gets what it asked for
/// and the others divide what is left.
pub(super) fn shares(
    wants: &[CellRowDemand],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
) -> Vec<PaneAxisSize> {
    solve_column_shares(
        wants,
        height,
        head,
        focused,
        floor,
        &mut DemandOnlyColumnRows {
            exact: disjoint_demands(
                wants.iter().map(|demand| demand.exact_framed()),
                height,
                floor,
            ),
        },
    )
}

/// Keep the shared allocation unless the summary needs more rows than its share.
pub(super) fn summary_before_tiles(
    wants: &[CellRowDemand],
    height: u16,
    floor: u16,
    shared: Vec<PaneAxisSize>,
) -> Vec<PaneAxisSize> {
    let exact = disjoint_demands(
        wants.iter().map(|demand| demand.exact_framed()),
        height,
        floor,
    );
    if shared
        .first()
        .is_some_and(|share| matches!(share, PaneAxisSize::Fixed(rows) if *rows >= exact[0]))
    {
        return shared;
    }
    let reserved = exact[0].min(height);
    let mut rows = vec![PaneAxisSize::Fixed(reserved)];
    if wants.len() > 1 {
        let remaining = height.saturating_sub(reserved);
        rows.extend(shares(
            &wants[1..],
            remaining,
            ColumnHead::Cell,
            ColumnFocus::Outside,
            floor,
        ));
    }
    rows
}

/// Row-use readings available to a column's shared allocation solver.
pub(super) trait ColumnRowReadings {
    fn unused(&mut self, index: usize, rows: u16, purpose: RowUsePurpose) -> u16;
}

/// Intrinsic content fills each allocation only up to its exact demand.
struct DemandOnlyColumnRows {
    exact: Vec<u16>,
}

impl ColumnRowReadings for DemandOnlyColumnRows {
    fn unused(&mut self, index: usize, rows: u16, _: RowUsePurpose) -> u16 {
        rows.saturating_sub(self.exact[index])
    }
}

/// Whether a column has already asked for one content allocation.
#[derive(Clone, Copy)]
enum ColumnRowAnswer {
    Unmeasured,
    Measured(u16),
}

/// Lazily measured content use, indexed by tile, height and purpose.
pub(super) struct ColumnRowUse<'m, F> {
    measure: &'m mut F,
    answers: Vec<Vec<[ColumnRowAnswer; 2]>>,
}

impl<'m, F: FnMut(usize, u16, RowUsePurpose) -> u16> ColumnRowUse<'m, F> {
    pub(super) fn new(count: usize, measure: &'m mut F) -> Self {
        Self {
            measure,
            answers: vec![Vec::new(); count],
        }
    }
}

impl<F: FnMut(usize, u16, RowUsePurpose) -> u16> ColumnRowReadings for ColumnRowUse<'_, F> {
    fn unused(&mut self, index: usize, rows: u16, purpose: RowUsePurpose) -> u16 {
        let purpose_index = match purpose {
            RowUsePurpose::RetainContent => 0,
            RowUsePurpose::PaintContent => 1,
        };
        let answers = &mut self.answers[index];
        if answers.len() <= usize::from(rows) {
            answers.resize(usize::from(rows) + 1, [ColumnRowAnswer::Unmeasured; 2]);
        }
        let answer = &mut answers[usize::from(rows)][purpose_index];
        match *answer {
            ColumnRowAnswer::Measured(unused) => unused,
            ColumnRowAnswer::Unmeasured => {
                let unused = (self.measure)(index, rows, purpose);
                *answer = ColumnRowAnswer::Measured(unused);
                unused
            },
        }
    }
}

pub(super) fn shares_with_directional_row_usage(
    wants: &[CellRowDemand],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
    unused_rows: &mut impl FnMut(usize, u16, RowUsePurpose) -> u16,
) -> Vec<PaneAxisSize> {
    let mut measured = ColumnRowUse::new(wants.len(), unused_rows);
    solve_column_shares(wants, height, head, focused, floor, &mut measured)
}

fn solve_column_shares(
    wants: &[CellRowDemand],
    height: u16,
    head: ColumnHead,
    focused: ColumnFocus,
    floor: u16,
    measured: &mut impl ColumnRowReadings,
) -> Vec<PaneAxisSize> {
    let count = wants.len();
    if count == 0 {
        return Vec::new();
    }
    let mut weights = vec![1; count];
    if let ColumnHead::Summary(depth) = head {
        weights[0] = u16::try_from(depth).unwrap_or(u16::MAX);
    }
    let base = apportion_shared_run(&weights, height);
    let want = disjoint_demands(
        wants.iter().map(|demand| demand.stepped_framed()),
        height,
        floor,
    );
    let exact = disjoint_demands(
        wants.iter().map(|demand| demand.exact_framed()),
        height,
        floor,
    );
    let mut minimum = disjoint_demands(std::iter::repeat_n(floor, count), height, floor);

    let mut short: Vec<u16> = want
        .iter()
        .zip(&base)
        .map(|(asked, share)| asked.saturating_sub(*share))
        .collect();
    let spare: Vec<u16> = base
        .iter()
        .zip(&want)
        .map(|(share, asked)| share.saturating_sub(*asked))
        .collect();

    // The summary first when it heads the column, then the focus ring,
    // then the rest of the short cells over whatever they left, each in
    // proportion to how short it is.
    let mut room = spare.iter().copied().fold(0, u16::saturating_add);
    let mut taken = vec![0; count];
    if matches!(head, ColumnHead::Summary(_)) {
        let served = short[0].min(room);
        room = room.saturating_sub(served);
        short[0] = 0;
        taken[0] = served;
    }
    if let ColumnFocus::Row(at) = focused {
        let served = short.get(at).copied().unwrap_or_default().min(room);
        room = room.saturating_sub(served);
        if let Some(cell) = short.get_mut(at) {
            *cell = 0;
        }
        if let Some(cell) = taken.get_mut(at) {
            *cell = cell.saturating_add(served);
        }
    }
    let wanted = short.iter().copied().fold(0, u16::saturating_add);
    for (cell, part) in taken.iter_mut().zip(apportion(&short, wanted.min(room))) {
        *cell = cell.saturating_add(part);
    }

    let moved = taken.iter().copied().fold(0, u16::saturating_add);
    let given = apportion(&spare, moved);
    let mut settled: Vec<u16> = base
        .into_iter()
        .enumerate()
        .map(|(index, share)| {
            share
                .saturating_add(taken.get(index).copied().unwrap_or_default())
                .saturating_sub(given.get(index).copied().unwrap_or_default())
        })
        .collect();
    if let ColumnHead::Summary(depth) = head {
        let positions = count.saturating_add(depth.saturating_sub(1));
        let held_summary = settled[0];
        lend_to_summary(&mut settled, &exact, height, positions, &minimum);
        if settled[0] > held_summary
            && settled[0] >= exact[0]
            && measured.unused(0, settled[0], RowUsePurpose::PaintContent) == 0
        {
            minimum[0] = minimum[0].max(exact[0].saturating_sub(measured.unused(
                0,
                exact[0],
                RowUsePurpose::RetainContent,
            )));
        }
    }
    serve_directional_shortfalls(&mut settled, &exact, &minimum, measured);
    settled.into_iter().map(PaneAxisSize::Fixed).collect()
}

/// Convert framed demands into disjoint layout rows. Every piece except the
/// last receives its closing frame row from [`share_borders`](crate::share_borders).
fn disjoint_demands(framed: impl IntoIterator<Item = u16>, height: u16, floor: u16) -> Vec<u16> {
    let mut rows: Vec<u16> = framed
        .into_iter()
        .map(|asked| asked.max(floor).min(height).saturating_sub(1))
        .collect();
    if height > 0
        && let Some(last) = rows.last_mut()
    {
        *last = last.saturating_add(1);
    }
    rows
}

/// Give a nearby summary shortfall rows from its tallest eligible mate.
fn lend_to_summary(
    settled: &mut [u16],
    wants: &[u16],
    height: u16,
    positions: usize,
    floors: &[u16],
) {
    let Some((&held, &asked)) = settled.first().zip(wants.first()) else {
        return;
    };
    let short = asked.saturating_sub(held);
    let framed_slot = apportion_shared_run(&vec![1; positions], height)
        .first()
        .copied()
        .unwrap_or_default()
        .saturating_add(u16::from(height > 0 && settled.len() > 1));
    let available = settled
        .iter()
        .zip(floors)
        .skip(1)
        .map(|(rows, floor)| rows.saturating_sub(*floor))
        .fold(0, u16::saturating_add);
    if short == 0 || short > framed_slot / SUMMARY_SLOT_SHORTFALL_DIVISOR || available < short {
        return;
    }
    for _ in 0..short {
        let Some(donor) = (1..settled.len())
            .filter(|&index| settled[index] > floors[index])
            .max_by_key(|&index| {
                settled[index].saturating_add(u16::from(index + 1 < settled.len()))
            })
        else {
            return;
        };
        settled[0] = settled[0].saturating_add(1);
        settled[donor] = settled[donor].saturating_sub(1);
    }
}

/// A donor's fixed, nonempty supply of rows that preserve protected content.
#[derive(Eq, Ord, PartialEq, PartialOrd)]
struct DonorSupply {
    framed_height: u16,
    index:         usize,
    remaining:     NonZeroU16,
}

/// A recipient step that increases retained content, or preserves it and paints more.
struct RecipientContentGain {
    index:      usize,
    rows:       u16,
    protection: RowUsePurpose,
}

/// Whether a candidate allocation increases the column's content measure.
enum RecipientRowStep {
    NoContentGain,
    Gain(RecipientContentGain),
}

impl RecipientRowStep {
    pub(super) fn measure(
        index: usize,
        rows: u16,
        painted: u16,
        retained: u16,
        measured: &mut impl ColumnRowReadings,
    ) -> Self {
        let next_painted =
            rows.saturating_sub(measured.unused(index, rows, RowUsePurpose::PaintContent));
        if next_painted <= painted {
            return Self::NoContentGain;
        }
        let next_retained =
            rows.saturating_sub(measured.unused(index, rows, RowUsePurpose::RetainContent));
        let protection = match next_retained.cmp(&retained) {
            Ordering::Greater => RowUsePurpose::RetainContent,
            Ordering::Equal => RowUsePurpose::PaintContent,
            Ordering::Less => return Self::NoContentGain,
        };
        Self::Gain(RecipientContentGain {
            index,
            rows,
            protection,
        })
    }
}

/// A content gain funded entirely by the plan's fixed donor supplies.
enum ColumnTransfer<'a> {
    Unfunded,
    Funded {
        gain:          RecipientContentGain,
        donor_heights: &'a [u16],
    },
}

impl ColumnTransfer<'_> {
    pub(super) const fn apply(self, settled: &mut [u16]) -> bool {
        match self {
            Self::Unfunded => false,
            Self::Funded {
                gain,
                donor_heights,
            } => {
                settled.copy_from_slice(donor_heights);
                settled[gain.index] = gain.rows;
                true
            },
        }
    }
}

/// An ordered sequence of donor rows, spending only its initial supplies.
struct ColumnDonations {
    heights:   Vec<u16>,
    available: BinaryHeap<DonorSupply>,
    donated:   u16,
}

impl ColumnDonations {
    pub(super) fn new(
        recipient: usize,
        settled: &[u16],
        exact: &[u16],
        minimum: &[u16],
        protection: RowUsePurpose,
        measured: &mut impl ColumnRowReadings,
    ) -> Self {
        let mut available = BinaryHeap::new();
        for index in 0..settled.len() {
            if index == recipient {
                continue;
            }
            let free = directional_spare_rows(index, settled, exact, minimum, protection, measured);
            if let Some(remaining) = NonZeroU16::new(free) {
                available.push(DonorSupply {
                    framed_height: settled[index]
                        .saturating_add(u16::from(index + 1 < settled.len())),
                    index,
                    remaining,
                });
            }
        }
        Self {
            heights: settled.to_vec(),
            available,
            donated: 0,
        }
    }

    fn transfer(&mut self, gain: RecipientContentGain) -> ColumnTransfer<'_> {
        let rows = gain.rows.saturating_sub(self.heights[gain.index]);
        while self.donated < rows {
            let Some(mut donor) = self.available.pop() else {
                return ColumnTransfer::Unfunded;
            };
            self.heights[donor.index] -= 1;
            self.donated += 1;
            if let Some(remaining) = NonZeroU16::new(donor.remaining.get() - 1) {
                donor.remaining = remaining;
                donor.framed_height -= 1;
                self.available.push(donor);
            }
        }
        ColumnTransfer::Funded {
            gain,
            donor_heights: &self.heights,
        }
    }
}

pub(super) fn serve_directional_shortfalls(
    settled: &mut [u16],
    exact: &[u16],
    minimum: &[u16],
    measured: &mut impl ColumnRowReadings,
) {
    loop {
        let mut recipients: Vec<usize> = (0..settled.len())
            .filter(|&index| settled[index] < exact[index])
            .collect();
        recipients
            .sort_by_key(|&index| Reverse((exact[index].saturating_sub(settled[index]), index)));
        let mut moved = false;
        for recipient in recipients {
            let held = settled[recipient];
            let painted =
                held.saturating_sub(measured.unused(recipient, held, RowUsePurpose::PaintContent));
            let retained =
                held.saturating_sub(measured.unused(recipient, held, RowUsePurpose::RetainContent));
            let available = (0..settled.len())
                .filter(|&index| index != recipient)
                .map(|index| {
                    donation_row_limit(
                        index,
                        settled,
                        exact,
                        minimum,
                        RowUsePurpose::RetainContent,
                        measured,
                    )
                })
                .fold(0, u16::saturating_add);
            let retained_donations = OnceCell::new();
            let painted_donations = OnceCell::new();
            for step in 1..=exact[recipient].saturating_sub(held).min(available) {
                let rows = held.saturating_add(step);
                let RecipientRowStep::Gain(gain) =
                    RecipientRowStep::measure(recipient, rows, painted, retained, measured)
                else {
                    continue;
                };
                let plan = match gain.protection {
                    RowUsePurpose::RetainContent => &retained_donations,
                    RowUsePurpose::PaintContent => &painted_donations,
                };
                let mut donations = plan
                    .get_or_init(|| {
                        RefCell::new(ColumnDonations::new(
                            recipient,
                            settled,
                            exact,
                            minimum,
                            gain.protection,
                            measured,
                        ))
                    })
                    .borrow_mut();
                if donations.transfer(gain).apply(settled) {
                    moved = true;
                    break;
                }
            }
            if moved {
                break;
            }
        }
        if !moved {
            return;
        }
    }
}

/// Consecutive removable rows whose lower allocations keep protected content.
/// Paint-only gains also preserve donor retained content, so every transfer
/// increases (total retained use, total painted use) in lexicographic order.
pub(super) fn directional_spare_rows(
    index: usize,
    settled: &[u16],
    exact: &[u16],
    minimum: &[u16],
    protection: RowUsePurpose,
    measured: &mut impl ColumnRowReadings,
) -> u16 {
    let rows = settled[index];
    let limit = donation_row_limit(index, settled, exact, minimum, protection, measured);
    if limit == 0 {
        return 0;
    }
    let used = rows.saturating_sub(measured.unused(index, rows, protection));
    let retained = match protection {
        RowUsePurpose::RetainContent => used,
        RowUsePurpose::PaintContent => {
            rows.saturating_sub(measured.unused(index, rows, RowUsePurpose::RetainContent))
        },
    };
    (1..=limit)
        .take_while(|&donated| {
            let lower = rows - donated;
            lower.saturating_sub(measured.unused(index, lower, protection)) == used
                && (matches!(protection, RowUsePurpose::RetainContent)
                    || lower.saturating_sub(measured.unused(
                        index,
                        lower,
                        RowUsePurpose::RetainContent,
                    )) == retained)
        })
        .last()
        .unwrap_or_default()
}

/// An upper bound on donor rows before lower allocations are measured.
fn donation_row_limit(
    index: usize,
    settled: &[u16],
    exact: &[u16],
    minimum: &[u16],
    protection: RowUsePurpose,
    measured: &mut impl ColumnRowReadings,
) -> u16 {
    let rows = settled[index];
    let used = rows.saturating_sub(measured.unused(index, rows, protection));
    rows.saturating_sub(exact[index].min(used).max(minimum[index]))
}

/// Whether the positions of a grid retain a readable minimum size.
pub(super) fn fits(area: Rect, count: usize, growth: TileGrowth, settings: &TileSettings) -> bool {
    matches!(
        column_layout(count, 1, growth, area, settings),
        ColumnPlan::Columns(_)
    )
}

/// Rows the first column allocates to a summary of `depth` positions.
pub(super) fn summary_share(
    wants: &[CellRowDemand],
    height: u16,
    depth: usize,
    growth: TileGrowth,
    floor: u16,
    area: Rect,
    settings: &TileSettings,
) -> u16 {
    let ColumnPlan::Columns(plan) = column_layout(wants.len(), depth, growth, area, settings)
    else {
        return 0;
    };
    let pieces = plan.columns.residents(0);
    let shares = shares(
        &wants[..pieces.min(wants.len())],
        height,
        ColumnHead::Summary(plan.columns.summary_depth),
        ColumnFocus::Outside,
        floor,
    );
    match shares.first() {
        Some(PaneAxisSize::Fixed(rows)) => {
            rows.saturating_add(u16::from(height > 0 && shares.len() > 1))
        },
        _ => 0,
    }
}

/// Fewest summary positions that hold its current demand within the fit limit.
pub(super) fn summary_depth(
    area: Rect,
    wants: &[CellRowDemand],
    growth: TileGrowth,
    settings: &TileSettings,
) -> usize {
    let Some(asked) = wants.first().map(|demand| demand.stepped_framed()) else {
        return 1;
    };
    let stepped: Vec<CellRowDemand> = wants
        .iter()
        .map(|demand| CellRowDemand::uniform(demand.stepped_framed()))
        .collect();
    let mut depth = 1;
    loop {
        if summary_share(
            &stepped,
            area.height,
            depth,
            growth,
            settings.min_tile_height.get(),
            area,
            settings,
        ) >= asked
        {
            return depth;
        }
        let next = depth + 1;
        let ColumnPlan::Columns(plan) = column_layout(wants.len(), next, growth, area, settings)
        else {
            return depth;
        };
        if next > plan.columns.summary_depth {
            return depth;
        }
        depth = next;
    }
}

/// `total` split between `weights` in proportion, the units integer
/// division leaves over going to the largest remainders so the parts
/// always add back up to `total` exactly.
pub(super) fn apportion(weights: &[u16], total: u16) -> Vec<u16> {
    let sum: u64 = weights.iter().copied().map(u64::from).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let total = u64::from(total);
    let scaled: Vec<u64> = weights
        .iter()
        .map(|&weight| total * u64::from(weight))
        .collect();
    let mut parts: Vec<u64> = scaled.iter().map(|value| value / sum).collect();
    let mut spare = total.saturating_sub(parts.iter().sum::<u64>());
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&index| Reverse(scaled[index] % sum));
    for index in order {
        if spare == 0 {
            break;
        }
        if let Some(part) = parts.get_mut(index) {
            *part += 1;
            spare -= 1;
        }
    }
    parts
        .into_iter()
        .map(|part| u16::try_from(part).unwrap_or(u16::MAX))
        .collect()
}

/// Disjoint layout sizes for panes whose neighbouring frames share one row.
///
/// Every pane except the last receives its lower frame row later through
/// [`share_borders`](crate::share_borders). The last pane owns the column's closing row, so it is
/// reserved before the rest is divided and restored to the last share.
pub(super) fn apportion_shared_run(weights: &[u16], total: u16) -> Vec<u16> {
    let mut shares = apportion(weights, total.saturating_sub(1));
    if total > 0
        && let Some(last) = shares.last_mut()
    {
        *last = last.saturating_add(1);
    }
    shares
}

/// The exact and stepped framed-row demands for every cell, the summary's
/// first. An empty cell asks for nothing beyond its border, which [`shares`]
/// takes as a cell with room to spare rather than one to shrink.
pub(super) fn cell_wants<Id: Clone + Eq + Debug>(
    demands: &TileDemands<Id>,
    slots: &[Slot<Id>],
    settings: &TileSettings,
) -> Vec<CellRowDemand> {
    let mut wants = Vec::with_capacity(slots.len() + TABLE_CELL);
    wants.push(settings.demanded_rows(demands.summary));
    wants.extend(slots.iter().map(|slot| match slot {
        Slot::Group(id) => settings.demanded_rows(demands.rows_for(id)),
        Slot::Empty(_) => settings.demanded_rows(0),
    }));
    wants
}

/// Cells a run of `count` tiles needs along one axis once neighbours
/// share a border: one leading line and its content per tile, then a
/// single line closing the run.
pub(super) const fn shared_run(count: u16, min_tile: u16) -> u16 {
    count
        .saturating_mul(min_tile.saturating_sub(1))
        .saturating_add(1)
}
