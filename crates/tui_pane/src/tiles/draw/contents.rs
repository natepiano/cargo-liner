use super::Buffer;
use super::CellContentExtent;
use super::CellFadeProgress;
use super::CellReadoutFade;
use super::Color;
use super::Line;
use super::PaneFrameLabel;
use super::Rect;
use super::RowsReadout;
use super::SummaryFoot;
use super::TileContent;
use super::TileDemands;
/// Whether the cells are drawn with what is in them.
///
/// Hidden leaves the frames, the borders and the summary title and
/// takes away the contents, the readouts and the summary's border
/// labels -- everything that is a number rather than a place. It is
/// what the grid looks like while something else is arriving over it or
/// leaving it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TileGridContents {
    /// Draw what each cell holds.
    Shown,
    /// Draw the cells and nothing in them.
    Hidden,
}

/// What an app draws inside the cells of a [`TileGrid`](crate::TileGrid).
///
/// [`draw_tile_grid`](crate::draw_tile_grid) owns the arrangement, the borders, the readout
/// along the foot of every cell and the number an empty cell carries.
/// This is the rest: how many rows each cell's contents ask for, and
/// the contents themselves.
pub trait TileCells<Id> {
    /// The title written into the summary cell's top border.
    fn summary_title(&self) -> &str;

    /// Completeness of the source, independent of the cell's measured row demand.
    fn content_extent(&self, _content: &TileContent<Id>) -> CellContentExtent {
        CellContentExtent::Measured
    }

    /// Measured rows shown in the foot at this allocation, excluding surrendered decoration.
    /// The allocation and motion continue to use the original demand.
    fn readout_rows(&self, _content: &TileContent<Id>, _area: Rect, measured_rows: usize) -> usize {
        measured_rows
    }

    /// Readout for this surface after frame preparation, without changing its demand.
    fn rows_readout(
        &self,
        content: &TileContent<Id>,
        area: Rect,
        measured_rows: usize,
    ) -> RowsReadout {
        RowsReadout::Count(self.readout_rows(content, area, measured_rows))
    }

    /// Text drawn whole or not at all at the left end of the summary
    /// cell's readout row.
    fn summary_foot(&self) -> SummaryFoot { SummaryFoot::Empty }

    /// Fade used by the cell's contents, also applied to every foot span.
    /// Existing cell providers retain their original readout inks.
    fn readout_fade(&self, _content: &TileContent<Id>) -> CellReadoutFade {
        CellReadoutFade::Unfaded
    }

    /// Fade amount and blend rule shared with the cell's text.
    /// Providers with entry brightening can preserve its channel rounding here.
    fn readout_progress(&self, content: &TileContent<Id>) -> CellFadeProgress {
        match self.readout_fade(content) {
            CellReadoutFade::Unfaded => CellFadeProgress::new(0),
            CellReadoutFade::Fading(progress) => progress,
        }
    }

    /// Set frame-wide content widths before demand and allocation measurements.
    /// Call `allocations` only when preparation needs the current allocation layout.
    /// Areas exclude borders and foot rows, using the same calculation as painting.
    fn prepare_frame(&self, _allocations: impl FnOnce() -> Vec<(TileContent<Id>, Rect)>) {}

    /// Prepare widths using destination geometry by default. Providers whose widths
    /// depend on visible rows can opt into the tallest source/destination heights.
    /// The second callback retains destination positions and widths.
    fn prepare_moving_frame(
        &self,
        destination: impl FnOnce() -> Vec<(TileContent<Id>, Rect)>,
        _: impl FnOnce() -> Vec<(TileContent<Id>, Rect)>,
    ) {
        self.prepare_frame(destination);
    }

    /// Rows each cell's contents would take given all the room they
    /// want, each measured at the width `widths` gives that cell.
    ///
    /// Content rows only: the grid adds the readout row itself. A group
    /// `widths` does not carry yet -- one first seen on this frame --
    /// belongs at the narrowest width in `widths`, which is the width it
    /// will have once the grid has opened a cell for it.
    fn demands(&self, widths: &[(TileContent<Id>, u16)]) -> TileDemands<Id>;

    /// Rows the cell's content paints when `area` is the room available.
    ///
    /// The grid reads this answer when checking whether a moving piece
    /// fills its allocation. Content that can paint every row keeps the
    /// default.
    fn rows_drawn(&self, _content: &TileContent<Id>, area: Rect) -> u16 { area.height }

    /// Rows the grid must leave with the cell before its column mates may
    /// take the rest.
    ///
    /// Most content retains every row it paints. Content with an optional
    /// part may return less here so a mate can complete its next row step.
    fn rows_kept(&self, content: &TileContent<Id>, area: Rect) -> u16 {
        self.rows_drawn(content, area)
    }

    /// Draw `content` into `inner`, the part of its cell above the
    /// readout. `ground` is the colour the cell is painted on.
    ///
    /// Never called for [`TileContent::Empty`], whose number the grid
    /// draws itself.
    fn draw(&self, buffer: &mut Buffer, content: &TileContent<Id>, inner: Rect, ground: Color);

    /// Labels written along the summary cell's top border after its
    /// title, given the summary cell's whole box. None unless the app
    /// has some.
    fn summary_labels(&self, _rect: Rect) -> Vec<PaneFrameLabel> { Vec::new() }

    /// The title written into the top border of the cell drawing `id`,
    /// set against the corner the way [`summary_title`] is, and left off
    /// while the contents are hidden. None unless the app names its
    /// groups there.
    ///
    /// The line's and each span's styles are patched over the chrome's
    /// title style, so a title that sets only a colour keeps the weight
    /// focus gives it, and an unstyled one is drawn as the summary's
    /// title is.
    ///
    /// [`summary_title`]: Self::summary_title
    fn group_title(&self, _id: &Id) -> Option<Line<'static>> { None }
}
