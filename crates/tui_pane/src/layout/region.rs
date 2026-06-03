//! A box tree describing one pane's vertical layout, and the layout pass that
//! resolves it to per-box rects, scroll offsets, and the single highlight's
//! location.
//!
//! A pane is boxes inside boxes — that is what is on screen, so that is the
//! model. A [`Region`] is either a list of selectable rows ([`Region::rows`])
//! or a vertical [`Region::stack`] of boxes. Each rows box has a [`Size`]:
//! [`Size::Fixed`] takes exactly its rows (and never scrolls), [`Size::Fill`]
//! takes the room the fixed boxes leave (and scrolls when its rows don't fit).
//! Exactly one box per stack is `Fill`.
//!
//! Three things are kept apart: where boxes sit (the tree), the highlighted
//! row and scrolling (the existing [`Viewport`](super::Viewport) holds the one
//! highlight; each scrolling box keeps its own offset, held by the pane across
//! frames), and drawing (the pane draws each box's rows and any chrome into
//! the rect the tree hands it).
//!
//! The single highlight is one number that walks every selectable row in tree
//! order. [`Region::locate`] maps that number to "this box, this row";
//! [`Region::place`] resolves the tree to a [`Placed`] per box, scrolling the
//! box that holds the highlight to keep it in view and leaving the other boxes
//! where they were.

use ratatui::layout::Rect;

use super::keep_visible_scroll_offset;

/// How a rows box claims vertical space within its stack.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Size {
    /// Takes exactly the box's rows (plus chrome) and never scrolls.
    Fixed,
    /// Takes the room the fixed boxes leave, and scrolls when its rows don't
    /// fit. Exactly one box per stack is `Fill`.
    Fill,
}

/// A list of selectable rows: how many rows, how it sizes, and how many chrome
/// rows (a title divider or column header) it reserves above the rows. The
/// tree only leaves room for chrome; the pane draws it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rows {
    count:  usize,
    chrome: u16,
    size:   Size,
}

impl Rows {
    /// Outer height a `Fixed` box occupies: chrome rows plus content rows.
    /// Ignored for the `Fill` box, which takes the leftover instead.
    fn outer_height(self) -> u16 {
        self.chrome
            .saturating_add(u16::try_from(self.count).unwrap_or(u16::MAX))
    }
}

/// One box in a pane's layout tree: either a list of selectable rows or a
/// vertical stack of boxes (top to bottom).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Region {
    /// A list of selectable rows.
    Rows(Rows),
    /// A vertical stack of boxes, laid out top to bottom.
    Stack(Vec<Self>),
}

/// One box resolved to screen rects plus its scroll offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Placed {
    /// Chrome rows (title divider / column header) at the box's top; the pane
    /// draws them. Zero-height when the box reserves no chrome.
    pub chrome:        Rect,
    /// Content rows — where the box's selectable rows render.
    pub content:       Rect,
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
            size,
        })
    }

    /// A vertical stack of boxes, laid out top to bottom.
    #[must_use]
    pub const fn stack(children: Vec<Self>) -> Self { Self::Stack(children) }

    /// Reserve one chrome row (a horizontal rule) above this rows box's
    /// content. The tree leaves the row; the pane draws the rule.
    #[must_use]
    pub fn rule(mut self) -> Self {
        if let Self::Rows(rows) = &mut self {
            rows.chrome = rows.chrome.saturating_add(1);
        } else {
            debug_assert!(false, "rule() applies to a rows box, not a stack");
        }
        self
    }

    /// This stack's leaf rows boxes, in tree order. A bare rows box reads as a
    /// one-box stack. Nested stacks land with `Columns` in a later phase; until
    /// then a stack's children are all rows boxes.
    fn leaves(&self) -> Vec<Rows> {
        match self {
            Self::Rows(rows) => vec![*rows],
            Self::Stack(children) => {
                let leaves: Vec<Rows> = children
                    .iter()
                    .filter_map(|child| match child {
                        Self::Rows(rows) => Some(*rows),
                        Self::Stack(_) => None,
                    })
                    .collect();
                debug_assert_eq!(
                    leaves.len(),
                    children.len(),
                    "a stack's children are all rows boxes in this phase"
                );
                leaves
            },
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

    /// Resolve the tree within `area` for the given highlight position and each
    /// box's prior scroll offset (indexed by box, tree order — a `Fixed` box
    /// ignores its entry). Sizing order: every `Fixed` box takes its exact
    /// outer height, then the one `Fill` box takes the remainder. The box that
    /// holds the highlight is scrolled to keep it visible; the others hold
    /// their prior offset (re-clamped to the box's current range).
    #[must_use]
    pub fn place(&self, area: Rect, cursor: usize, prior_offsets: &[usize]) -> Vec<Placed> {
        let leaves = self.leaves();
        let mut fixed_total: u16 = 0;
        let mut fill_index: Option<usize> = None;
        for (index, rows) in leaves.iter().enumerate() {
            match rows.size {
                Size::Fixed => fixed_total = fixed_total.saturating_add(rows.outer_height()),
                Size::Fill => {
                    debug_assert!(fill_index.is_none(), "a stack has exactly one Fill box");
                    fill_index = Some(index);
                },
            }
        }
        debug_assert!(fill_index.is_some(), "a stack has exactly one Fill box");
        let fill_outer = area.height.saturating_sub(fixed_total);

        let mut placed = Vec::with_capacity(leaves.len());
        let mut y = area.y;
        let mut selectable_start = 0;
        for (index, rows) in leaves.iter().enumerate() {
            let natural = if Some(index) == fill_index {
                fill_outer
            } else {
                rows.outer_height()
            };
            // Clamp to the room left so a too-short area can't push later boxes
            // off-screen; the trailing boxes shrink to nothing instead.
            let outer = natural.min(area.bottom().saturating_sub(y));
            let chrome_height = rows.chrome.min(outer);
            let content_height = outer.saturating_sub(chrome_height);
            let chrome = Rect {
                x: area.x,
                y,
                width: area.width,
                height: chrome_height,
            };
            let content = Rect {
                x:      area.x,
                y:      y.saturating_add(chrome_height),
                width:  area.width,
                height: content_height,
            };
            let scroll_offset = box_scroll_offset(
                *rows,
                cursor
                    .checked_sub(selectable_start)
                    .filter(|local| *local < rows.count),
                usize::from(content_height),
                prior_offsets.get(index).copied().unwrap_or(0),
            );
            placed.push(Placed {
                chrome,
                content,
                scroll_offset,
            });
            y = y.saturating_add(outer);
            selectable_start += rows.count;
        }
        placed
    }
}

/// Scroll offset for one box this frame. A `Fixed` box never scrolls. A `Fill`
/// box clamps to keep the highlight visible while it holds the cursor
/// (`local_cursor`), and otherwise holds its prior offset, re-clamped to the
/// box's current scroll range so a shrunk row count can't strand it.
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
    }
}

#[cfg(test)]
mod tests {
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
    fn rebuilding_the_tree_stays_well_under_the_frame_budget() {
        // The pane rebuilds its tree every frame; guard that 10k rebuilds cost
        // far less than a frame so per-frame construction is not a concern.
        for _ in 0..10_000 {
            let placed = cpu_tree(64).place(inner(40), 20, &[0, 0, 0, 0]);
            assert_eq!(placed.len(), 4);
        }
    }
}
