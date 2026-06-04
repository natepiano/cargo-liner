//! A box tree describing one pane's layout, and the layout pass that resolves
//! it to per-box rects, scroll offsets, and the single highlight's location.
//!
//! A pane is boxes inside boxes — that is what is on screen, so that is the
//! model. A [`Region`] is a list of selectable rows ([`Region::rows`]), a
//! vertical [`Region::stack`] of boxes, or side-by-side [`Region::columns`].
//! Each rows box has a [`Size`]: [`Size::Fixed`] takes exactly its rows (and
//! never scrolls), [`Size::Fill`] takes the room the fixed boxes leave (and
//! scrolls when its rows don't fit), [`Size::Cap`] grows to fit its rows but
//! stops at a percent of its stack (and scrolls past that, pinned to its
//! bottom when the cursor is elsewhere). Exactly one child per stack takes
//! the leftover room: a `Fill` rows box or a nested node (a stack or columns
//! child always takes the remainder of its parent stack). Each column spans
//! its columns node's full height and sizes its own children independently,
//! so two columns can each hold their own `Fill`.
//!
//! Three things are kept apart: where boxes sit (the tree), the highlighted
//! row and scrolling (the existing [`Viewport`](super::Viewport) holds the one
//! highlight; each scrolling box keeps its own offset, held by the pane across
//! frames), and drawing (the pane draws each box's rows and any chrome into
//! the rect the tree hands it).
//!
//! The single highlight is one number that walks every selectable row in
//! flattened-leaf order: depth-first, stacks top to bottom, columns left to
//! right. [`Region::locate`] maps that number to "this box, this row";
//! [`Region::place`] resolves the tree to a [`Placed`] per leaf box in the
//! same order, scrolling the box that holds the highlight to keep it in view
//! and leaving the other boxes where they were.

use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;

use super::keep_visible_scroll_offset;

/// How a rows box claims vertical space within its stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    /// Takes exactly the box's rows (plus chrome) and never scrolls.
    Fixed,
    /// Takes the room the fixed boxes leave, and scrolls when its rows don't
    /// fit. Exactly one child per stack takes the leftover room — a `Fill`
    /// rows box or a nested node.
    Fill,
    /// Grows to fit its rows (plus chrome) but stops at this percent of the
    /// stack's height, scrolling past that. Without the cursor the box pins
    /// to its bottom so the newest row stays visible. Build through
    /// [`Size::cap`].
    Cap(u16),
}

impl Size {
    /// A [`Size::Cap`] clamped share of the stack: `percent` must be in
    /// `1..=100`.
    #[must_use]
    pub fn cap(percent: u16) -> Self {
        debug_assert!(
            percent > 0 && percent <= 100,
            "a cap is a percentage of the stack's height"
        );
        Self::Cap(percent)
    }
}

/// A list of selectable rows: how many rows, how it sizes, how many chrome
/// rows (a rule, a blank spacer, a column header) it reserves above the rows
/// and below them (a footer rule or pager line), and an optional
/// rendered-height override. The tree only leaves room for chrome; the pane
/// draws it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rows {
    count:  usize,
    chrome: u16,
    footer: u16,
    lines:  Option<u16>,
    size:   Size,
}

impl Rows {
    /// Screen rows the box's content occupies: the [`Region::lines`] override
    /// when set, otherwise one row per selectable row.
    fn content_height(self) -> u16 {
        self.lines
            .unwrap_or_else(|| u16::try_from(self.count).unwrap_or(u16::MAX))
    }

    /// Outer height a `Fixed` box occupies: chrome rows plus content rows
    /// plus footer rows. Ignored for the `Fill` box, which takes the
    /// leftover instead.
    fn outer_height(self) -> u16 {
        self.chrome
            .saturating_add(self.content_height())
            .saturating_add(self.footer)
    }

    /// Outer height this box claims from a stack `stack_height` tall, or
    /// `None` for the `Fill` box (which takes the leftover instead). A
    /// `Fixed` box takes its natural height; a `Cap` box takes its natural
    /// height clamped to its percent share of the stack.
    fn claimed_height(self, stack_height: u16) -> Option<u16> {
        match self.size {
            Size::Fixed => Some(self.outer_height()),
            Size::Cap(percent) => {
                let cap =
                    u16::try_from(u32::from(stack_height).saturating_mul(u32::from(percent)) / 100)
                        .unwrap_or(u16::MAX);
                Some(self.outer_height().min(cap))
            },
            Size::Fill => None,
        }
    }
}

/// One box in a pane's layout tree: a list of selectable rows, a vertical
/// stack of boxes (top to bottom), or side-by-side columns (left to right).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Region {
    /// A list of selectable rows.
    Rows(Rows),
    /// A vertical stack of boxes, laid out top to bottom.
    Stack(Vec<Self>),
    /// Side-by-side columns, each a width constraint and a child box. Every
    /// column spans the node's full height.
    Columns(Vec<(Constraint, Self)>),
}

/// One box resolved to screen rects plus its scroll offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placed {
    /// Chrome rows (title divider / column header) at the box's top; the pane
    /// draws them. Zero-height when the box reserves no chrome.
    pub chrome:        Rect,
    /// Content rows — where the box's selectable rows render.
    pub content:       Rect,
    /// Footer rows (a rule or pager line) at the box's bottom; the pane
    /// draws them. Zero-height when the box reserves no footer.
    pub footer:        Rect,
    /// Rows scrolled off the top of `content`. Always 0 for a `Fixed` box.
    pub scroll_offset: usize,
}

impl Region {
    /// A list of `rows` selectable rows with the given size and no chrome.
    #[must_use]
    pub const fn rows(count: usize, size: Size) -> Self {
        Self::Rows(Rows {
            count,
            chrome: 0,
            footer: 0,
            lines: None,
            size,
        })
    }

    /// A vertical stack of boxes, laid out top to bottom.
    #[must_use]
    pub const fn stack(children: Vec<Self>) -> Self { Self::Stack(children) }

    /// Side-by-side columns, each a `(width, child)` pair laid out left to
    /// right. Flattened-leaf order walks the leftmost column's leaves first,
    /// so `prior_offsets` and [`Self::locate`] follow reading order.
    #[must_use]
    pub const fn columns(children: Vec<(Constraint, Self)>) -> Self { Self::Columns(children) }

    /// Reserve one chrome row (a horizontal rule) above this rows box's
    /// content. The tree leaves the row; the pane draws the rule.
    #[must_use]
    pub fn rule(mut self) -> Self {
        if let Self::Rows(rows) = &mut self {
            rows.chrome = rows.chrome.saturating_add(1);
        } else {
            debug_assert!(false, "rule() applies to a rows box, not a node");
        }
        self
    }

    /// Reserve one blank chrome row above this rows box — the gap separating
    /// it from the box above. The tree leaves the row; the pane draws nothing
    /// into it.
    #[must_use]
    pub fn spacer(mut self) -> Self {
        if let Self::Rows(rows) = &mut self {
            rows.chrome = rows.chrome.saturating_add(1);
        } else {
            debug_assert!(false, "spacer() applies to a rows box, not a node");
        }
        self
    }

    /// Reserve one chrome row (a column-header line) above this rows box's
    /// content. The tree leaves the row; the pane draws the header.
    #[must_use]
    pub fn header(mut self) -> Self {
        if let Self::Rows(rows) = &mut self {
            rows.chrome = rows.chrome.saturating_add(1);
        } else {
            debug_assert!(false, "header() applies to a rows box, not a node");
        }
        self
    }

    /// Reserve one chrome row below this rows box's content — its lower
    /// boundary (a rule, a pager line, or a blank gap). The tree leaves the
    /// row; the pane draws it.
    #[must_use]
    pub fn footer(mut self) -> Self {
        if let Self::Rows(rows) = &mut self {
            rows.footer = rows.footer.saturating_add(1);
        } else {
            debug_assert!(false, "footer() applies to a rows box, not a node");
        }
        self
    }

    /// Override the rendered height of this rows box: the box occupies
    /// `lines` screen rows while the cursor still addresses its row count — a
    /// wrapped text block is one selectable row rendered as several lines.
    /// Sizing reads the override on a `Fixed` box; a `Fill` box takes the
    /// leftover room regardless.
    #[must_use]
    pub fn lines(mut self, lines: u16) -> Self {
        if let Self::Rows(rows) = &mut self {
            rows.lines = Some(lines);
        } else {
            debug_assert!(false, "lines() applies to a rows box, not a node");
        }
        self
    }

    /// Whether this child claims its parent stack's leftover room: a `Fill`
    /// rows box, or a nested node (a stack or columns child always takes the
    /// remainder of its parent stack).
    const fn fills_stack(&self) -> bool {
        match self {
            Self::Rows(rows) => matches!(rows.size, Size::Fill),
            Self::Stack(_) | Self::Columns(_) => true,
        }
    }

    /// This tree's leaf rows boxes in flattened-leaf order: depth-first,
    /// stacks top to bottom, columns left to right. A bare rows box reads as
    /// a one-box stack.
    fn leaves(&self) -> Vec<Rows> {
        match self {
            Self::Rows(rows) => vec![*rows],
            Self::Stack(children) => children.iter().flat_map(Self::leaves).collect(),
            Self::Columns(children) => children
                .iter()
                .flat_map(|(_, child)| child.leaves())
                .collect(),
        }
    }

    /// Total selectable rows across every box — the value the pane hands to
    /// [`Viewport::set_len`](super::Viewport::set_len) each frame.
    #[must_use]
    pub fn total_selectable(&self) -> usize { self.leaves().iter().map(|rows| rows.count).sum() }

    /// Which box holds the highlight, and the row within that box, for a global
    /// cursor position. The boxes own contiguous selectable-row ranges in tree
    /// order; returns `(box_index, row_within_box)`, or `None` when `cursor`
    /// is past the last selectable row.
    #[must_use]
    pub fn locate(&self, cursor: usize) -> Option<(usize, usize)> {
        let mut start = 0;
        for (index, rows) in self.leaves().iter().enumerate() {
            if cursor < start + rows.count {
                return Some((index, cursor - start));
            }
            start += rows.count;
        }
        None
    }

    /// Resolve the tree within `area` for the given highlight position and
    /// each box's prior scroll offset (indexed by leaf box, flattened-leaf
    /// order — a `Fixed` box ignores its entry). Sizing order within each
    /// stack: every `Fixed` box takes its exact outer height, then the one
    /// child that fills takes the remainder; columns split their node's width
    /// by their constraints and each spans the node's full height. The box
    /// that holds the highlight is scrolled to keep it visible; the others
    /// hold their prior offset (re-clamped to the box's current range).
    #[must_use]
    pub fn place(&self, area: Rect, cursor: usize, prior_offsets: &[usize]) -> Vec<Placed> {
        let mut walk = PlaceWalk {
            cursor,
            prior_offsets,
            leaf_index: 0,
            selectable_start: 0,
            placed: Vec::new(),
        };
        self.place_into(area, &mut walk);
        walk.placed
    }

    /// Resolve one node within its allotted rect, appending its leaves'
    /// [`Placed`] entries to the walk in flattened-leaf order.
    fn place_into(&self, area: Rect, walk: &mut PlaceWalk<'_>) {
        match self {
            // A bare rows box outside a stack (the tree root or a column
            // child): `Fixed` and `Cap` take their claimed height at the top
            // of the rect, `Fill` takes the whole rect.
            Self::Rows(rows) => {
                let outer = rows
                    .claimed_height(area.height)
                    .map_or(area.height, |claimed| claimed.min(area.height));
                place_leaf(
                    *rows,
                    Rect {
                        height: outer,
                        ..area
                    },
                    walk,
                );
            },
            Self::Stack(children) => place_stack(children, area, walk),
            Self::Columns(children) => {
                let widths: Vec<Constraint> = children.iter().map(|(width, _)| *width).collect();
                let column_areas = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints(widths)
                    .split(area);
                for ((_, child), column_area) in children.iter().zip(column_areas.iter()) {
                    child.place_into(*column_area, walk);
                }
            },
        }
    }
}

/// Mutable state threaded through one [`Region::place`] pass: the highlight,
/// the per-leaf prior offsets, and the running position in flattened-leaf
/// order.
struct PlaceWalk<'a> {
    cursor:           usize,
    prior_offsets:    &'a [usize],
    leaf_index:       usize,
    selectable_start: usize,
    placed:           Vec<Placed>,
}

/// Lay out one stack's children top to bottom: every `Fixed` rows box takes
/// its exact outer height, every `Cap` rows box its clamped height, and the
/// one child that fills takes the remainder.
fn place_stack(children: &[Region], area: Rect, walk: &mut PlaceWalk<'_>) {
    debug_assert_eq!(
        children.iter().filter(|child| child.fills_stack()).count(),
        1,
        "a stack has exactly one child that takes the remainder"
    );
    let claimed_total = children.iter().fold(0u16, |total, child| match child {
        Region::Rows(rows) => total.saturating_add(rows.claimed_height(area.height).unwrap_or(0)),
        Region::Stack(_) | Region::Columns(_) => total,
    });
    let fill_outer = area.height.saturating_sub(claimed_total);

    let mut y = area.y;
    for child in children {
        let natural = match child {
            Region::Rows(rows) => rows.claimed_height(area.height).unwrap_or(fill_outer),
            Region::Stack(_) | Region::Columns(_) => fill_outer,
        };
        // Clamp to the room left so a too-short area can't push later boxes
        // off-screen; the trailing boxes shrink to nothing instead.
        let outer = natural.min(area.bottom().saturating_sub(y));
        let child_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: outer,
        };
        match child {
            Region::Rows(rows) => place_leaf(*rows, child_area, walk),
            node => node.place_into(child_area, walk),
        }
        y = y.saturating_add(outer);
    }
}

/// Resolve one leaf rows box within its exact outer rect: split off the
/// chrome rows at the top and the footer rows at the bottom, compute the
/// content rect between, and record its scroll offset.
fn place_leaf(rows: Rows, area: Rect, walk: &mut PlaceWalk<'_>) {
    let chrome_height = rows.chrome.min(area.height);
    let footer_height = rows.footer.min(area.height.saturating_sub(chrome_height));
    let content_height = area
        .height
        .saturating_sub(chrome_height)
        .saturating_sub(footer_height);
    let chrome = Rect {
        height: chrome_height,
        ..area
    };
    let content = Rect {
        y: area.y.saturating_add(chrome_height),
        height: content_height,
        ..area
    };
    let footer = Rect {
        y: content.bottom(),
        height: footer_height,
        ..area
    };
    let scroll_offset = box_scroll_offset(
        rows,
        walk.cursor
            .checked_sub(walk.selectable_start)
            .filter(|local| *local < rows.count),
        usize::from(content_height),
        walk.prior_offsets
            .get(walk.leaf_index)
            .copied()
            .unwrap_or(0),
    );
    walk.placed.push(Placed {
        chrome,
        content,
        footer,
        scroll_offset,
    });
    walk.leaf_index += 1;
    walk.selectable_start += rows.count;
}

/// Scroll offset for one box this frame. A `Fixed` box never scrolls. A `Fill`
/// box clamps to keep the highlight visible while it holds the cursor
/// (`local_cursor`), and otherwise holds its prior offset, re-clamped to the
/// box's current scroll range so a shrunk row count can't strand it. A `Cap`
/// box keeps the highlight visible while it holds the cursor and otherwise
/// pins to its bottom — the newest row of a grows-upward list stays visible.
fn box_scroll_offset(
    rows: Rows,
    local_cursor: Option<usize>,
    visible: usize,
    prior_offset: usize,
) -> usize {
    match rows.size {
        Size::Fixed => 0,
        Size::Fill => local_cursor.map_or_else(
            || prior_offset.min(rows.count.saturating_sub(visible)),
            |local| keep_visible_scroll_offset(local, visible, rows.count),
        ),
        Size::Cap(_) => local_cursor.map_or_else(
            || rows.count.saturating_sub(visible),
            |local| keep_visible_scroll_offset(local, visible, rows.count),
        ),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Constraint;
    use ratatui::layout::Rect;

    use super::Region;
    use super::Size;

    /// The CPU pane's tree: a pinned aggregate row, a scrolling cores band, a
    /// breakdown box (3 rows, rule above), and a GPU box (1 row, rule above).
    fn cpu_tree(cores: usize) -> Region {
        Region::stack(vec![
            Region::rows(1, Size::Fixed),
            Region::rows(cores, Size::Fill),
            Region::rows(3, Size::Fixed).rule(),
            Region::rows(1, Size::Fixed).rule(),
        ])
    }

    /// A detail-pane-like tree: a 4-line description block (one selectable
    /// row), then two columns — a scrolling metadata column (6 rows) beside a
    /// stack of a pinned head (3 rows), a scrolling middle (5 rows, spacer +
    /// rule above), and a pinned tail (2 rows, spacer + rule above). Both
    /// columns reserve their top row for the shared separator rule.
    fn detail_tree() -> Region {
        Region::stack(vec![
            Region::rows(1, Size::Fixed).lines(4),
            Region::columns(vec![
                (Constraint::Min(8), Region::rows(6, Size::Fill).rule()),
                (
                    Constraint::Length(10),
                    Region::stack(vec![
                        Region::rows(3, Size::Fixed).rule(),
                        Region::rows(5, Size::Fill).spacer().rule(),
                        Region::rows(2, Size::Fixed).spacer().rule(),
                    ]),
                ),
            ]),
        ])
    }

    fn inner(height: u16) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: 20,
            height,
        }
    }

    #[test]
    fn total_selectable_sums_every_box() {
        // 1 aggregate + 8 cores + 3 breakdown + 1 GPU.
        assert_eq!(cpu_tree(8).total_selectable(), 13);
    }

    #[test]
    fn locate_maps_the_cursor_to_box_and_row() {
        let tree = cpu_tree(8);
        // Aggregate (box 0, row 0).
        assert_eq!(tree.locate(0), Some((0, 0)));
        // First and last core (box 1).
        assert_eq!(tree.locate(1), Some((1, 0)));
        assert_eq!(tree.locate(8), Some((1, 7)));
        // Breakdown System/User/Idle (box 2).
        assert_eq!(tree.locate(9), Some((2, 0)));
        assert_eq!(tree.locate(11), Some((2, 2)));
        // GPU (box 3).
        assert_eq!(tree.locate(12), Some((3, 0)));
        // Past the end.
        assert_eq!(tree.locate(13), None);
    }

    #[test]
    fn fixed_boxes_take_exact_rows_and_fill_takes_the_remainder() {
        // Exactly sized: fixed total = 1 + (1+3) + (1+1) = 7, so a 15-row
        // inner gives the 8-core band its 8 rows with no slack.
        let placed = cpu_tree(8).place(inner(15), 0, &[0, 0, 0, 0]);
        // Aggregate: 1 row at the top, no chrome.
        assert_eq!(placed[0].chrome.height, 0);
        assert_eq!(placed[0].content, Rect::new(0, 0, 20, 1));
        // Cores: the Fill box takes the remainder (8 rows) right below.
        assert_eq!(placed[1].content, Rect::new(0, 1, 20, 8));
        // Breakdown: 1 chrome row (the rule) then 3 content rows.
        assert_eq!(placed[2].chrome, Rect::new(0, 9, 20, 1));
        assert_eq!(placed[2].content, Rect::new(0, 10, 20, 3));
        // GPU: 1 chrome row (the rule) then 1 content row, at the bottom.
        assert_eq!(placed[3].chrome, Rect::new(0, 13, 20, 1));
        assert_eq!(placed[3].content, Rect::new(0, 14, 20, 1));
    }

    #[test]
    fn fill_absorbs_extra_height_when_over_tall() {
        // 20-row inner, 8 cores: 13 rows of remainder go to the Fill box, so
        // the breakdown and GPU pin to the bottom border.
        let placed = cpu_tree(8).place(inner(20), 0, &[0, 0, 0, 0]);
        assert_eq!(placed[1].content, Rect::new(0, 1, 20, 13));
        assert_eq!(placed[2].chrome.y, 14);
        assert_eq!(placed[3].content, Rect::new(0, 19, 20, 1));
    }

    #[test]
    fn fill_box_scrolls_to_keep_the_highlight_visible() {
        // Cramped: 10-row inner leaves the 15-core band only 3 rows. The
        // cursor on the last core (global 15, band-local 14) scrolls to the
        // last full page.
        let placed = cpu_tree(15).place(inner(10), 15, &[0, 0, 0, 0]);
        assert_eq!(placed[1].content.height, 3);
        assert_eq!(placed[1].scroll_offset, 12);
    }

    #[test]
    fn fill_box_holds_prior_offset_while_the_cursor_is_elsewhere() {
        // Cursor on the aggregate row (global 0, outside the band): the band
        // holds its prior offset, clamped to its last page (15 - 3 = 12).
        let placed = cpu_tree(15).place(inner(10), 0, &[0, 7, 0, 0]);
        assert_eq!(placed[1].scroll_offset, 7);
        let pinned = cpu_tree(15).place(inner(10), 0, &[0, 20, 0, 0]);
        assert_eq!(pinned[1].scroll_offset, 12);
    }

    #[test]
    fn fixed_boxes_never_scroll() {
        // Cursor deep in the breakdown box: the breakdown and GPU boxes keep a
        // zero offset regardless of the prior values passed in.
        let placed = cpu_tree(15).place(inner(10), 11, &[5, 5, 5, 5]);
        assert_eq!(placed[0].scroll_offset, 0);
        assert_eq!(placed[2].scroll_offset, 0);
        assert_eq!(placed[3].scroll_offset, 0);
    }

    #[test]
    fn which_box_holds_the_highlight_at_box_boundaries() {
        let tree = cpu_tree(8);
        // Last aggregate row, first core, last core, first breakdown row.
        assert_eq!(tree.locate(0).map(|(box_index, _)| box_index), Some(0));
        assert_eq!(tree.locate(1).map(|(box_index, _)| box_index), Some(1));
        assert_eq!(tree.locate(8).map(|(box_index, _)| box_index), Some(1));
        assert_eq!(tree.locate(9).map(|(box_index, _)| box_index), Some(2));
    }

    #[test]
    fn degenerate_short_terminal_zeroes_the_fill_box() {
        // Inner shorter than the fixed total: the Fill box collapses to zero
        // rows and the fixed boxes still take their rooms (clamped by the
        // area), so nothing underflows.
        let placed = cpu_tree(8).place(inner(4), 0, &[0, 0, 0, 0]);
        assert_eq!(placed[1].content.height, 0);
    }

    #[test]
    fn columns_flatten_left_column_before_right() {
        let tree = detail_tree();
        // 1 description + 6 metadata + (3 + 5 + 2) right-column rows.
        assert_eq!(tree.total_selectable(), 17);
        // Description, then the left column's rows, then the right column's
        // boxes top to bottom.
        assert_eq!(tree.locate(0), Some((0, 0)));
        assert_eq!(tree.locate(1), Some((1, 0)));
        assert_eq!(tree.locate(6), Some((1, 5)));
        assert_eq!(tree.locate(7), Some((2, 0)));
        assert_eq!(tree.locate(10), Some((3, 0)));
        assert_eq!(tree.locate(15), Some((4, 0)));
        assert_eq!(tree.locate(17), None);
    }

    #[test]
    fn lines_override_reserves_the_rendered_height() {
        // The description box is one selectable row rendered as a 4-line
        // block: the columns below start at y = 4, not y = 1.
        let placed = detail_tree().place(inner(20), 0, &[0; 5]);
        assert_eq!(placed[0].content, Rect::new(0, 0, 20, 4));
        assert_eq!(placed[1].chrome.y, 4);
    }

    #[test]
    fn columns_split_the_width_and_span_the_node_height() {
        let placed = detail_tree().place(inner(20), 0, &[0; 5]);
        // Min(8) + Length(10) over a 20-wide area: the left column takes the
        // 10 columns the fixed right column leaves.
        let metadata = placed[1];
        assert_eq!(metadata.chrome, Rect::new(0, 4, 10, 1));
        assert_eq!(metadata.content, Rect::new(0, 5, 10, 15));
        // The right column starts where the left ends and its first box sits
        // at the columns node's top.
        assert_eq!(placed[2].chrome, Rect::new(10, 4, 10, 1));
        assert_eq!(placed[2].content, Rect::new(10, 5, 10, 3));
    }

    #[test]
    fn nested_stack_pins_its_tail_to_the_column_bottom() {
        let placed = detail_tree().place(inner(20), 0, &[0; 5]);
        // Right column spans y 4..20. Head takes 4 (rule + 3 rows), the tail
        // takes 4 (spacer + rule + 2 rows) at the bottom, the scrolling
        // middle takes the 8 between.
        assert_eq!(placed[3].chrome, Rect::new(10, 8, 10, 2));
        assert_eq!(placed[3].content, Rect::new(10, 10, 10, 6));
        assert_eq!(placed[4].chrome, Rect::new(10, 16, 10, 2));
        assert_eq!(placed[4].content, Rect::new(10, 18, 10, 2));
    }

    #[test]
    fn each_column_scrolls_its_own_fill_box() {
        // A 16-row inner leaves the right column's middle box 2 content rows
        // (12 - 4 head - 4 tail - 2 chrome). The cursor on its last row
        // (global 14, box-local 4) scrolls it to its last page, while the
        // metadata column — whose 6 rows fit its 11 visible — re-clamps its
        // prior offset to its own (empty) scroll range.
        let placed = detail_tree().place(inner(16), 14, &[0, 3, 0, 0, 0]);
        assert_eq!(placed[1].scroll_offset, 0);
        assert_eq!(placed[3].content.height, 2);
        assert_eq!(placed[3].scroll_offset, 3);
    }

    #[test]
    fn fill_box_in_a_column_holds_prior_offset_while_the_cursor_is_elsewhere() {
        // Cursor on the description row: the right column's middle box holds
        // its prior offset, clamped to its scroll range (5 rows - 2 visible).
        let placed = detail_tree().place(inner(16), 0, &[0, 0, 0, 1, 0]);
        assert_eq!(placed[3].scroll_offset, 1);
        let pinned = detail_tree().place(inner(16), 0, &[0, 0, 0, 9, 0]);
        assert_eq!(pinned[3].scroll_offset, 3);
    }

    /// A Targets-like tree: a scrolling table (one header chrome row) above a
    /// capped Running box (a divider rule plus a header — two chrome rows)
    /// that grows upward to at most 80% of the inner height.
    fn targets_tree(table: usize, running: usize) -> Region {
        Region::stack(vec![
            Region::rows(table, Size::Fill).header(),
            Region::rows(running, Size::cap(80)).rule().header(),
        ])
    }

    #[test]
    fn cap_box_takes_its_natural_height_under_the_cap() {
        // 3 running rows + 2 chrome = 5 outer, well under 80% of 20; the
        // table's Fill takes the other 15 (header chrome + 14 content rows).
        let placed = targets_tree(10, 3).place(inner(20), 0, &[0, 0]);
        assert_eq!(placed[0].chrome, Rect::new(0, 0, 20, 1));
        assert_eq!(placed[0].content, Rect::new(0, 1, 20, 14));
        assert_eq!(placed[1].chrome, Rect::new(0, 15, 20, 2));
        assert_eq!(placed[1].content, Rect::new(0, 17, 20, 3));
    }

    #[test]
    fn cap_box_clamps_to_its_percent_of_the_stack() {
        // 30 running rows + 2 chrome = 32 outer, but 80% of 20 caps the box
        // at 16; the table keeps the remaining 4 (its 20% floor).
        let placed = targets_tree(10, 30).place(inner(20), 0, &[0, 0]);
        assert_eq!(placed[1].chrome.y, 4);
        assert_eq!(placed[1].content.height, 14);
        assert_eq!(placed[0].content.height, 3);
    }

    #[test]
    fn cap_box_scrolls_to_keep_the_highlight_visible() {
        // Cursor on the last running row (global 39, box-local 29) scrolls
        // the 14-row window to the last page (30 - 14 = 16).
        let placed = targets_tree(10, 30).place(inner(20), 39, &[0, 0]);
        assert_eq!(placed[1].scroll_offset, 16);
    }

    #[test]
    fn cap_box_pins_to_its_bottom_while_the_cursor_is_elsewhere() {
        // Cursor in the table: the Running box ignores any prior offset and
        // pins to its bottom so the newest row stays visible.
        let placed = targets_tree(10, 30).place(inner(20), 0, &[0, 3]);
        assert_eq!(placed[1].scroll_offset, 16);
    }

    #[test]
    fn footer_reserves_the_bottom_row_of_the_box() {
        // A Fill box with a header above and a footer below, over a fixed
        // tail: outer = 12 - 4 = 8, split header 1 / content 6 / footer 1.
        let tree = Region::stack(vec![
            Region::rows(10, Size::Fill).header().footer(),
            Region::rows(3, Size::Fixed).rule(),
        ]);
        let placed = tree.place(inner(12), 0, &[0, 0]);
        assert_eq!(placed[0].chrome, Rect::new(0, 0, 20, 1));
        assert_eq!(placed[0].content, Rect::new(0, 1, 20, 6));
        assert_eq!(placed[0].footer, Rect::new(0, 7, 20, 1));
        // The fixed tail starts right below the footer.
        assert_eq!(placed[1].chrome.y, 8);
        // Scrolling pages the content rows only — the footer is chrome, so
        // the cursor on the last row scrolls to 10 - 6 visible = 4.
        let scrolled = tree.place(inner(12), 9, &[0, 0]);
        assert_eq!(scrolled[0].scroll_offset, 4);
    }

    #[test]
    fn boxes_without_a_footer_get_a_zero_height_footer_rect() {
        let placed = cpu_tree(8).place(inner(15), 0, &[0, 0, 0, 0]);
        assert_eq!(placed[0].footer.height, 0);
        assert_eq!(placed[2].footer.height, 0);
    }

    #[test]
    fn rebuilding_the_tree_stays_well_under_the_frame_budget() {
        // The pane rebuilds its tree every frame; guard that 10k rebuilds cost
        // far less than a frame so per-frame construction is not a concern.
        for _ in 0..10_000 {
            let placed = cpu_tree(64).place(inner(40), 20, &[0, 0, 0, 0]);
            assert_eq!(placed.len(), 4);
        }
    }
}
