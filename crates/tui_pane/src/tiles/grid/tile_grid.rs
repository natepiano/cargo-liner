#[cfg(test)]
#[path = "tile_grid_tests.rs"]
mod tests;

#[cfg(test)]
use super::AtomicOrdering;
#[cfg(test)]
use super::AtomicUsize;
use super::BackgroundTransparency;
use super::Buffer;
use super::CandidateDepthSelection;
use super::CandidateMeasurementFreshness;
use super::CellAppearance;
use super::CellHit;
use super::CellIdentity;
use super::CellIndex;
use super::CellRowDemand;
use super::CellTransition;
use super::CellTravel;
use super::CheckedLayoutAdmission;
use super::ColumnLayout;
use super::ColumnPlan;
use super::ColumnPreparation;
use super::ColumnRanks;
use super::CycleDirection;
use super::Debug;
use super::DemandSync;
use super::DestinationDepthStatus;
use super::DestinationHeightBounds;
#[cfg(test)]
use super::DestinationRowUse;
use super::Direction;
use super::Duration;
use super::EndpointLayoutRules;
use super::FilledColumnFit;
use super::Focus;
use super::FocusLocation;
use super::FocusedPaneTint;
use super::FrameCellRowAnswers;
use super::FrameCellRowUsage;
use super::FrameHeightFit;
use super::FrameRowAnswer;
use super::Grid;
use super::GridDisplay;
use super::GridMotion;
use super::GridVisualState;
use super::HeightSnap;
use super::HeldCellLayout;
use super::Instant;
use super::LastPainted;
use super::LastSettledCellGeometry;
use super::Layout;
use super::LayoutDemandMeasurement;
use super::LayoutSelection;
use super::LayoutSelectionInputs;
use super::MOVING_HEIGHT_PROBE_STEP;
use super::MotionClock;
use super::MotionHeightPath;
use super::MotionSpanBounds;
#[cfg(test)]
use super::MotionTiming;
use super::NonZeroU16;
use super::OnceLock;
use super::Ordering;
use super::PROGRESS_SCALE;
use super::PaneFrame;
use super::PieceHeightPath;
use super::PieceHeightSnap;
use super::PieceName;
use super::Position;
use super::QueuedArrangement;
use super::Rect;
use super::Repaint;
use super::ResolvedPane;
use super::ResolvedPaneLayout;
use super::RetainedRows;
use super::RetargetedHeightConstraints;
use super::RetargetedMotionDestination;
use super::RowDemandChange;
use super::RowUsePurpose;
use super::SelectedLayoutMeasurements;
use super::SelectedLayoutPlan;
use super::Slot;
use super::SummaryRowAllocation;
use super::TABLE_CELL;
use super::TileAction;
use super::TileColumnOrder;
use super::TileContent;
use super::TileDemands;
use super::TileDrawing;
use super::TileGridContents;
use super::TileGrowth;
use super::TileMeasurements;
use super::TilePiece;
use super::TilePlacement;
use super::TileSettings;
use super::TileView;
use super::TileWidthFloor;
use super::Turns;
use super::VecDeque;
use super::candidate_content_widths;
use super::cell_of;
use super::cell_wants;
use super::cells;
use super::claims;
use super::column_bands;
use super::column_layout;
use super::constraints_for_sizes;
use super::content_of;
use super::draw;
use super::eased;
use super::fill_retargeted_columns;
use super::fills;
use super::fits;
use super::frame_height_fit;
use super::frame_inner;
use super::frame_painted_rows;
use super::frame_retained_rows;
use super::host;
use super::kept;
use super::lerp_rect;
use super::measure_cell_retained_rows;
use super::measure_destination_rows;
use super::measure_frame_row_use;
use super::moving_cell;
use super::place_band_pieces;
use super::queued_cells_keep_endpoint_columns;
use super::share_borders;
use super::summary_depth;
use super::summary_share;
use super::summary_span;
use super::travel_progress;
use super::union;
/// The grid's cells and the transition it is playing, if any.
///
/// Cell one is the summary and is always there; every other cell draws
/// one group, keyed by the app's `Id`, or stands empty. The app feeds
/// the grid what each cell is asking for through [`Self::sync`], draws
/// what [`Self::placements`] hands back, and moves it on with
/// [`Self::tick`] and [`Self::apply`].
pub struct TileGrid<Id> {
    /// Cells after the summary, in cell order. The summary is cell one
    /// and is not held here, so the grid never falls below one cell.
    pub(super) slots:                  Vec<Slot<Id>>,
    /// Summary depth of `slots`.
    pub(super) depth:                  usize,
    /// Arrangements waiting their turn, each one cell's move from the
    /// one before it. Only ever one of them is in flight, which is what
    /// makes a change propagate through the grid instead of happening
    /// to every cell at once.
    pub(super) pending:                VecDeque<Step<Id>>,
    /// What each cell is currently drawn at, which is what a change
    /// animates away from. Empty until the first scan settles it.
    pub(super) held:                   HeldCellLayout,
    /// What every cell is asking for, as the last scan left it.
    pub(super) demands:                TileDemands<Id>,
    /// Whether the geometry is settled or playing a timed move.
    pub(super) motion:                 GridMotion<Id>,
    /// The rect the last frame laid out, so [`Self::add`] can tell
    /// whether the cells it would create still fit on screen.
    pub(super) area:                   Rect,
    /// How the grid grows, as the last frame had it. Retained so a
    /// mouse click can resolve the same geometry the frame drew without
    /// the caller carrying the settings to every hit test.
    pub(super) growth:                 TileGrowth,
    /// The view drawing, hit-testing and focus movement share until a
    /// settled frame commits another one.
    pub(super) held_display:           GridDisplay,
    /// Identity for the next cell opened or emptied.
    pub(super) next_slot:              u64,
    /// The cell the focus ring is on.
    pub(super) focus:                  Focus<Id>,
    /// The sizes and timings the grid is laid out and animated with.
    pub(super) settings:               TileSettings,
    /// The body paint used for both visual comparison and input.
    pub(super) last_painted:           LastPainted<Id>,
    pub(super) motion_clock:           MotionClock,
    /// Content-row usage at every size inspected during the current frame.
    pub(super) frame_row_usage:        Vec<FrameCellRowUsage<Id>>,
    /// Cell geometry drawn by the last settled frame.
    pub(super) last_settled_geometry:  LastSettledCellGeometry,
    /// One-way heights selected by the current motion's earlier frames.
    pub(super) motion_heights:         MotionHeightPath,
    /// Filled destination geometry constrained by a content change in flight.
    pub(super) retargeted_destination: RetargetedMotionDestination,
    /// Admission is checked once until an arrangement or its rules change.
    pub(super) layout_admission:       OnceLock<CheckedLayoutAdmission>,
    /// One owner retains the candidate choice and its reuse inputs.
    pub(super) selected_plan:          SelectedLayoutPlan<Id>,
    /// Detail ranks are independent of width-dependent row demands.
    pub(super) column_ranks:           ColumnRanks<Id>,
    #[cfg(test)]
    pub(super) admission_checks:       AtomicUsize,
}

/// The arrangement a transition is moving away from.
pub(super) struct Transition<Id> {
    /// The cells as they stood before the change.
    pub(super) from:         Vec<Slot<Id>>,
    /// What those cells were drawn at. Snapshotted rather than
    /// recomputed, because by the time a transition starts the demands
    /// it is moving away from have already been replaced by the ones it
    /// is moving toward.
    pub(super) held:         HeldCellLayout,
    /// Exact cell geometry drawn before the motion began.
    pub(super) source:       Box<Grid>,
    /// When the motion began.
    pub(super) started:      Instant,
    /// How long this one step runs for.
    pub(super) millis:       u64,
    /// Whether the destination depth has seen its drawn widths.
    pub(super) depth_status: DestinationDepthStatus,
}

/// One arrangement waiting its turn, and how long the move into it
/// takes once it comes up.
pub(super) struct Step<Id> {
    /// The cells as this step leaves them.
    pub(super) slots:        Vec<Slot<Id>>,
    /// Summary depth at the end of this step.
    pub(super) depth:        usize,
    /// How long the move into `slots` runs for.
    pub(super) millis:       u64,
    /// Whether the destination depth has seen its drawn widths.
    pub(super) depth_status: DestinationDepthStatus,
}

impl<Id: Clone + Eq + Debug> Default for TileGrid<Id> {
    fn default() -> Self { Self::new() }
}

impl<Id: Clone + Eq + Debug> TileGrid<Id> {
    /// A grid holding nothing but the summary.
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots:                         Vec::new(),
            depth:                         1,
            pending:                       VecDeque::new(),
            held:                          HeldCellLayout {
                rows:          Vec::new(),
                focused:       FocusLocation::Departing,
                summary_span:  1,
                summary_depth: 1,
            },
            demands:                       TileDemands::default(),
            motion:                        GridMotion::Settled,
            area:                          Rect::ZERO,
            growth:                        TileGrowth::default(),
            held_display:                  GridDisplay::Cells,
            next_slot:                     0,
            focus:                         Focus::Summary,
            settings:                      TileSettings::default(),
            last_painted:                  LastPainted::NeverPainted,
            motion_clock:                  MotionClock::Elapsed,
            frame_row_usage:               Vec::new(),
            last_settled_geometry:         LastSettledCellGeometry::default(),
            motion_heights:                MotionHeightPath::default(),
            retargeted_destination:        RetargetedMotionDestination::default(),
            layout_admission:              OnceLock::new(),
            selected_plan:                 SelectedLayoutPlan::Unmeasured,
            column_ranks:                  ColumnRanks::SourceOrder,
            #[cfg(test)]
            admission_checks:              AtomicUsize::new(0),
        }
    }

    /// Sets the core width, including its frame. The floor is required by default.
    pub fn set_min_tile_width(&mut self, width: u16) {
        let width = width.max(super::constants::MIN_TILE_WIDTH);
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                min_tile_width: width,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Sets whether a terminal below the core width keeps a compressed column.
    pub fn set_tile_width_floor(&mut self, floor: TileWidthFloor) {
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                width_floor: floor,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Sets the rule that chooses command cells or the summary alone.
    ///
    /// A motion retains the view it began with. A settled grid applies
    /// a view at once.
    pub const fn set_view(&mut self, view: TileView) {
        self.settings.view = view;
        if !matches!(self.motion, GridMotion::Settled) {
            return;
        }
        match view {
            TileView::Summary => self.held_display = GridDisplay::SummaryAlone,
            TileView::Cells => self.held_display = GridDisplay::Cells,
        }
    }

    /// Sets the shortest height a tile may occupy, including its frame.
    pub fn set_min_tile_height(&mut self, height: u16) {
        let height =
            NonZeroU16::MIN.saturating_add(height.max(super::constants::MIN_TILE_HEIGHT) - 1);
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                min_tile_height: height,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Choose whether the summary keeps its measured rows before the tiles.
    pub fn set_summary_row_allocation(&mut self, allocation: SummaryRowAllocation) {
        self.update_layout_rules(
            self.area,
            EndpointLayoutRules {
                summary_allocation: allocation,
                ..EndpointLayoutRules::new(self.growth, &self.settings)
            },
        );
    }

    /// Probe destination row steps, then nearby heights this moving frame may use.
    pub(in crate::tiles) fn measure_row_steps(
        &mut self,
        area: Rect,
        growth: TileGrowth,
        progress: u32,
        mut rows_drawn: impl FnMut(&TileContent<Id>, Rect) -> u16,
        mut rows_kept: impl FnMut(&TileContent<Id>, Rect) -> u16,
    ) {
        self.frame_row_usage.clear();
        if !self.holds_in(area, growth) {
            if matches!(self.motion, GridMotion::Settled) {
                self.held_display = GridDisplay::SummaryAlone;
                self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
            }
            return;
        }
        let contents: Vec<TileContent<Id>> = cells(&self.slots)
            .into_iter()
            .map(|(content, _)| content)
            .collect();
        let mut frame_row_usage: Vec<_> = contents
            .iter()
            .cloned()
            .map(|content| FrameCellRowUsage {
                content,
                rows: FrameCellRowAnswers::default(),
            })
            .collect();
        let held = self.drawn_held();
        let cells_grid = Grid::with_row_use(
            area,
            &held,
            growth,
            &self.settings,
            &mut |index, area, purpose| {
                contents.get(index).map_or(area.height, |_| {
                    let kept = measure_cell_retained_rows(
                        &mut frame_row_usage[index],
                        area,
                        &mut rows_drawn,
                        &mut rows_kept,
                    );
                    match purpose {
                        RowUsePurpose::RetainContent => kept,
                        RowUsePurpose::PaintContent => {
                            match frame_row_usage[index].rows.answer(area) {
                                FrameRowAnswer::Measured(rows) => rows.drawn,
                                FrameRowAnswer::Unmeasured => area.height,
                            }
                        },
                    }
                })
            },
        );
        let cells_grid = self.bound_retargeted_destination(
            cells_grid,
            &contents,
            &mut frame_row_usage,
            &mut rows_drawn,
            &mut rows_kept,
        );
        self.frame_row_usage = frame_row_usage;
        self.commit_display();
        if matches!(self.motion, GridMotion::Settled) {
            self.last_settled_geometry = if self.held_display == GridDisplay::Cells {
                LastSettledCellGeometry::Drawn {
                    area,
                    growth,
                    grid: cells_grid,
                }
            } else {
                LastSettledCellGeometry::NoCellsFrame
            };
            self.motion_heights = MotionHeightPath::default();
            return;
        }
        if self.held_display != GridDisplay::Cells || !matches!(self.motion, GridMotion::Moving(_))
        {
            self.motion_heights = MotionHeightPath::default();
            return;
        }
        if progress >= PROGRESS_SCALE {
            self.motion_heights = MotionHeightPath::default();
            return;
        }
        self.measure_moving_height_rows(area, growth, progress, &mut rows_drawn, &mut rows_kept);
    }

    /// Keep a filled content retarget from reversing unchanged resident heights.
    fn bound_retargeted_destination(
        &mut self,
        mut grid: Grid,
        contents: &[TileContent<Id>],
        row_usage: &mut Vec<FrameCellRowUsage<Id>>,
        rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
        rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    ) -> Grid {
        if matches!(
            self.retargeted_destination,
            RetargetedMotionDestination::Filled { .. }
        ) {
            let ordinary_panes = grid.resolved.panes.clone();
            if self.apply_retargeted_destination(&mut grid) {
                let measured = measure_destination_rows(
                    &grid.resolved.panes,
                    contents,
                    row_usage,
                    rows_drawn,
                    rows_kept,
                );
                if let RetargetedMotionDestination::Filled { row_use, .. } =
                    &self.retargeted_destination
                    && *row_use == measured
                {
                    return grid;
                }
                grid.resolved = ResolvedPaneLayout::new(ordinary_panes);
            }
            self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        }
        let GridMotion::Moving(transition) = &self.motion else {
            return grid;
        };
        if self.motion_heights.piece_heights.is_empty() || grid.columns != transition.source.columns
        {
            return grid;
        }
        let wants = cell_wants(&self.demands, &self.slots, &self.settings);
        let changed = contents
            .iter()
            .enumerate()
            .map(|(index, content)| {
                let source_cell = match content {
                    TileContent::Summary => Some(TABLE_CELL),
                    TileContent::Group(id) => cell_of(&transition.from, &Slot::Group(id.clone())),
                    TileContent::Empty(_) => None,
                };
                source_cell
                    .and_then(|cell| transition.held.rows.get(cell.saturating_sub(TABLE_CELL)))
                    .is_some_and(|source| source.exact_framed() != wants[index].exact_framed())
            })
            .collect::<Vec<_>>();
        if !changed.iter().any(|&changed| changed) {
            return grid;
        }
        let mut panes = grid.resolved.panes.clone();
        let DestinationHeightBounds::Constrained(bounds) =
            self.bound_resident_heights(&grid, contents, &changed, row_usage, &mut panes)
        else {
            return grid;
        };
        let constraints = RetargetedHeightConstraints {
            contents,
            row_demands: &wants,
            changed_demands: &changed,
            bounds: &bounds,
        };
        if fill_retargeted_columns(
            &grid,
            &mut panes,
            &constraints,
            row_usage,
            rows_drawn,
            rows_kept,
        ) == FilledColumnFit::NoFit
        {
            return grid;
        }
        grid.resolved = ResolvedPaneLayout::new(panes.clone());
        let row_use = measure_destination_rows(&panes, contents, row_usage, rows_drawn, rows_kept);
        self.retargeted_destination = RetargetedMotionDestination::Filled {
            area: grid.area,
            growth: self.growth,
            rows: wants,
            columns: grid.columns.clone(),
            span: grid.span,
            panes,
            row_use,
        };
        grid
    }

    /// Restrict resident heights to their drawn direction and empty height floor.
    fn bound_resident_heights(
        &self,
        grid: &Grid,
        contents: &[TileContent<Id>],
        changed: &[bool],
        row_usage: &[FrameCellRowUsage<Id>],
        panes: &mut [ResolvedPane<usize>],
    ) -> DestinationHeightBounds {
        let GridMotion::Moving(transition) = &self.motion else {
            return DestinationHeightBounds::Ordinary;
        };
        let drawing = self.drawing_at_with_height_snap(
            grid.area,
            self.growth,
            self.motion_heights.progress,
            &HeightSnap::Off,
        );
        let mut bounds = vec![
            MotionSpanBounds {
                smallest: grid.rules.min_tile_height.get().saturating_sub(1),
                largest:  grid.area.height.saturating_sub(1),
            };
            panes.len()
        ];
        let mut constrained = false;
        for (index, pane) in panes.iter_mut().enumerate() {
            let Some(content) = contents.get(pane.pane.saturating_sub(TABLE_CELL)) else {
                continue;
            };
            let empty = match content {
                TileContent::Summary => false,
                TileContent::Group(id) => {
                    !self.demands.holds(id)
                        && row_usage.iter().any(|cell| {
                            &cell.content == content
                                && cell.rows.values().any(|rows| {
                                    rows.height > 0 && rows.fit == FrameHeightFit::Whole
                                })
                                && cell.rows.values().all(|rows| rows.drawn == 0)
                        })
                },
                TileContent::Empty(_) => true,
            };
            if changed[pane.pane.saturating_sub(TABLE_CELL)] && !empty {
                continue;
            }
            let source_cell = match content {
                TileContent::Summary => Some(TABLE_CELL),
                TileContent::Group(id) => cell_of(&transition.from, &Slot::Group(id.clone())),
                TileContent::Empty(cell) => cell
                    .checked_sub(TABLE_CELL + 1)
                    .and_then(|index| self.slots.get(index))
                    .and_then(|slot| cell_of(&transition.from, slot)),
            };
            let Some(source) = source_cell.and_then(|cell| transition.source.cell(cell)) else {
                continue;
            };
            let Some(previous) = drawing
                .pieces
                .iter()
                .position(|piece| &piece.placement.content == content)
                .and_then(|piece| self.motion_heights.piece_heights.get(piece))
                .copied()
            else {
                continue;
            };
            let previous_span = previous
                .saturating_sub(1)
                .max(bounds[index].smallest)
                .min(bounds[index].largest);
            if empty {
                bounds[index].largest = previous_span;
            } else {
                match previous.cmp(&source.height) {
                    Ordering::Less => bounds[index].largest = previous_span,
                    Ordering::Greater => bounds[index].smallest = previous_span,
                    Ordering::Equal => {},
                }
            }
            let span = pane.area.height.saturating_sub(1);
            let retained = span.clamp(bounds[index].smallest, bounds[index].largest);
            constrained |= retained != span;
            pane.area.height = retained.saturating_add(1);
        }
        if constrained {
            DestinationHeightBounds::Constrained(bounds)
        } else {
            DestinationHeightBounds::Ordinary
        }
    }

    /// Apply a filled endpoint only while its layout inputs still agree.
    fn apply_retargeted_destination(&self, grid: &mut Grid) -> bool {
        let RetargetedMotionDestination::Filled {
            area,
            growth,
            rows,
            columns,
            span,
            panes,
            ..
        } = &self.retargeted_destination
        else {
            return false;
        };
        if *area != grid.area
            || *growth != self.growth
            || *rows != cell_wants(&self.demands, &self.slots, &self.settings)
            || *columns != grid.columns
            || *span != grid.span
            || panes
                .iter()
                .any(|pane| pane.area.height < grid.rules.min_tile_height.get())
        {
            return false;
        }
        grid.resolved = ResolvedPaneLayout::new(panes.clone());
        true
    }

    /// Probe the one-way candidates used to place this moving frame.
    fn measure_moving_height_rows(
        &mut self,
        area: Rect,
        growth: TileGrowth,
        progress: u32,
        rows_drawn: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
        rows_kept: &mut impl FnMut(&TileContent<Id>, Rect) -> u16,
    ) {
        let drawing = self.drawing_at_with_height_snap(area, growth, progress, &HeightSnap::Off);
        if self.motion_heights.area != area
            || self.motion_heights.growth != growth
            || progress < self.motion_heights.progress
            || self.motion_heights.piece_heights.len() != drawing.pieces.len()
        {
            self.motion_heights = MotionHeightPath::default();
        }
        let probes = drawing
            .pieces
            .into_iter()
            .enumerate()
            .filter_map(|(index, piece)| match piece.height_snap {
                PieceHeightSnap::Search { start, end } => Some((
                    index,
                    piece.placement.content,
                    piece.placement.frame.clip(),
                    start,
                    end,
                )),
                PieceHeightSnap::Settled | PieceHeightSnap::KeepStartingHeight => None,
            })
            .collect::<Vec<_>>();
        let mut frame_row_usage = std::mem::take(&mut self.frame_row_usage);
        for (index, content, rect, start, end) in probes {
            let previous = self
                .motion_heights
                .piece_heights
                .get(index)
                .copied()
                .unwrap_or(start);
            let mut lower = previous.min(end);
            let upper = previous.max(end);
            let interpolated = rect.height.clamp(lower, upper);
            let previous_area = draw::content_area(frame_inner(Rect {
                height: previous,
                ..rect
            }));
            measure_frame_row_use(
                &mut frame_row_usage,
                &content,
                previous_area,
                rows_drawn,
                rows_kept,
            );
            if frame_height_fit(&frame_row_usage, &content, previous_area)
                == FrameHeightFit::LeavesGap
            {
                let filled = frame_painted_rows(&frame_row_usage, &content, previous_area)
                    .saturating_add(previous.saturating_sub(previous_area.height));
                lower = lower.min(filled);
            }
            let probe_from = if frame_height_fit(&frame_row_usage, &content, previous_area)
                == FrameHeightFit::Whole
            {
                previous
            } else {
                interpolated
            };
            let toward_end = match probe_from.cmp(&end) {
                Ordering::Less => probe_from.saturating_add(MOVING_HEIGHT_PROBE_STEP),
                Ordering::Equal => probe_from,
                Ordering::Greater => probe_from.saturating_sub(MOVING_HEIGHT_PROBE_STEP),
            }
            .clamp(lower, upper);
            for height in [interpolated, previous, toward_end, end] {
                let area = draw::content_area(frame_inner(Rect { height, ..rect }));
                let drawn = measure_frame_row_use(
                    &mut frame_row_usage,
                    &content,
                    area,
                    rows_drawn,
                    rows_kept,
                );
                if frame_height_fit(&frame_row_usage, &content, area) == FrameHeightFit::Filled {
                    continue;
                }
                let frame_rows = height.saturating_sub(area.height);
                let filled_height = drawn.saturating_add(frame_rows).clamp(lower, upper);
                let filled_area = draw::content_area(frame_inner(Rect {
                    height: filled_height,
                    ..rect
                }));
                measure_frame_row_use(
                    &mut frame_row_usage,
                    &content,
                    filled_area,
                    rows_drawn,
                    rows_kept,
                );
            }
        }
        self.frame_row_usage = frame_row_usage;
        self.select_moving_heights(area, growth, progress);
    }

    /// Select and retain heights for this measured motion frame.
    fn select_moving_heights(&mut self, area: Rect, growth: TileGrowth, progress: u32) {
        let previous_heights = self.motion_heights.piece_heights.clone();
        let height_snap = HeightSnap::Frame {
            row_usage: &self.frame_row_usage,
            path:      PieceHeightPath::AdvanceFrom(&previous_heights),
        };
        let drawing = self.drawing_at_with_height_snap(area, growth, progress, &height_snap);
        self.motion_heights = MotionHeightPath {
            area,
            growth,
            progress,
            piece_heights: drawing
                .pieces
                .iter()
                .map(|piece| piece.placement.frame.clip().height)
                .collect(),
        };
    }

    /// Commit the view for a settled frame whose cells fit the area.
    ///
    /// Only the area decides: rows a cell cannot show go to its hidden
    /// count rather than switching the view.
    const fn commit_display(&mut self) {
        if !matches!(self.motion, GridMotion::Settled) {
            return;
        }
        self.held_display = match self.settings.view {
            TileView::Summary => GridDisplay::SummaryAlone,
            TileView::Cells => GridDisplay::Cells,
        };
    }

    /// The stored retained-row answer, or every row when unmeasured.
    fn frame_rows_kept(&self, content: &TileContent<Id>, area: Rect) -> u16 {
        match frame_retained_rows(&self.frame_row_usage, content, area) {
            RetainedRows::Unmeasured => area.height,
            RetainedRows::Measured(kept) => kept,
        }
    }

    /// Resolve the destination with the step probes gathered for it.
    pub(super) fn resolved_grid(
        &self,
        area: Rect,
        held: &HeldCellLayout,
        growth: TileGrowth,
    ) -> Grid {
        self.resolved_grid_for(area, &self.slots, held, growth)
    }

    /// Resolve one endpoint arrangement with this pass's retained rows.
    fn resolved_grid_for(
        &self,
        area: Rect,
        slots: &[Slot<Id>],
        held: &HeldCellLayout,
        growth: TileGrowth,
    ) -> Grid {
        let contents: Vec<TileContent<Id>> = cells(slots)
            .into_iter()
            .map(|(content, _)| content)
            .collect();
        let mut grid = Grid::with_row_use(
            area,
            held,
            growth,
            &self.settings,
            &mut |index, area, purpose| {
                contents
                    .get(index)
                    .map_or(area.height, |content| match purpose {
                        RowUsePurpose::RetainContent => self.frame_rows_kept(content, area),
                        RowUsePurpose::PaintContent => {
                            frame_painted_rows(&self.frame_row_usage, content, area)
                        },
                    })
            },
        );
        if slots == self.slots && self.apply_retargeted_destination(&mut grid) {
            return grid;
        }
        grid
    }

    /// What the grid is drawn at, worked out fresh whenever no settled
    /// frame has left an answer behind -- a grid that has never been
    /// synced, or one whose cell count has just changed under the
    /// snapshot.
    pub(super) fn drawn_held(&self) -> HeldCellLayout {
        if self.held.rows.len() == self.count() {
            return self.held.clone();
        }
        HeldCellLayout {
            rows:          cell_wants(&self.demands, &self.slots, &self.settings),
            focused:       self.focused_cell(),
            summary_span:  self.wanted_span(),
            summary_depth: self.depth,
        }
    }

    /// What the grid would be drawn at once this scan is folded in.
    ///
    /// The cells keep the rows they hold while every one of them is
    /// asking for no more than it already has. A single cell asking for
    /// more is the encroachment the whole grid is re-divided on, so
    /// every cell drops back to what it is showing at once rather than
    /// the short one taking its room from whichever neighbour happens
    /// to be next to it.
    fn settled_held(&self) -> HeldCellLayout {
        let wants = cell_wants(&self.demands, &self.slots, &self.settings);
        let focused = self.focused_cell();
        let summary_span = self.wanted_span();
        if wants.len() != self.held.rows.len()
            || self.depth != self.held.summary_depth
            || wants
                .iter()
                .zip(&self.held.rows)
                .any(|(want, held)| want.stepped_framed() > held.stepped_framed())
        {
            return HeldCellLayout {
                rows: wants,
                focused,
                summary_span,
                summary_depth: self.depth,
            };
        }
        let rows = self
            .held
            .rows
            .iter()
            .zip(wants)
            .map(|(&held, current)| held.with_exact_from(current))
            .collect();
        HeldCellLayout {
            rows,
            focused,
            summary_span,
            summary_depth: self.depth,
        }
    }

    /// Columns the summary asks to reach across at the current cell
    /// count, as the last frame's rect and settings have it.
    pub(super) fn wanted_span(&self) -> usize {
        if !self.growth.widen_summary {
            return 1;
        }
        match column_layout(
            self.count(),
            self.depth,
            self.growth,
            self.area,
            &self.settings,
        ) {
            ColumnPlan::Columns(layout) => summary_span(
                self.area,
                layout.columns.widths.len(),
                self.demands.summary_width,
            ),
            ColumnPlan::InsufficientWidth | ColumnPlan::InsufficientRows => 1,
        }
    }

    /// Cells the grid holds, the summary included.
    pub(super) const fn count(&self) -> usize { self.slots.len() + TABLE_CELL }

    /// The interior width of the cell drawing each thing on screen.
    ///
    /// The cell count and resolved summary depth settle the columns
    /// before any column is divided vertically. That lets the app
    /// measure a cell's contents at the width they will be wrapped to
    /// and hand the result back as the demand this same grid divides by.
    pub(in crate::tiles) fn content_widths(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<Id>, u16)> {
        let held = self.demand_measurement_held();
        let ColumnPlan::Columns(layout) = column_layout(
            self.count(),
            held.summary_depth,
            growth,
            area,
            &self.settings,
        ) else {
            return vec![(TileContent::Summary, frame_inner(area).width)];
        };
        let ColumnPlan::Columns(layout) = layout.for_summary(area, &held, &self.settings) else {
            return vec![(TileContent::Summary, frame_inner(area).width)];
        };
        let ColumnLayout {
            widths: column_heights,
            summary_depth: depth,
            ..
        } = layout.columns;
        let opened: Vec<Rect> =
            Layout::horizontal(constraints_for_sizes(&fills(column_heights.len())))
                .split(area)
                .to_vec();
        let mut widths = Vec::with_capacity(self.count());
        for (column, &cells_here) in column_heights.iter().enumerate() {
            let inner = opened
                .get(column)
                .map_or(0, |&rect| frame_inner(share_borders(rect, area)).width);
            widths.extend(std::iter::repeat_n(inner, cells_here));
        }
        if depth > 1 {
            widths.drain(1..depth);
        }
        // The summary alone can reach past its own column, which only
        // the laid-out grid knows.
        let summary = self
            .resolved_grid(area, &held, growth)
            .cell(TABLE_CELL)
            .map(|rect| frame_inner(rect).width);
        cells(&self.slots)
            .into_iter()
            .zip(widths)
            .map(|((content, _), width)| {
                let width = match content {
                    TileContent::Summary => summary.unwrap_or(width),
                    TileContent::Group(_) | TileContent::Empty(_) => width,
                };
                (content, width)
            })
            .collect()
    }

    /// Existing allocation layout used to prepare widths before the demand pass.
    pub(in crate::tiles) fn preparation_allocations(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<Id>, Rect)> {
        if self.display() == GridDisplay::SummaryAlone {
            return vec![(TileContent::Summary, draw::content_area(frame_inner(area)))];
        }
        let layout = self.resolved_grid(area, &self.demand_measurement_held(), growth);
        cells(&self.slots)
            .into_iter()
            .filter_map(|(content, index)| {
                layout
                    .cell(index)
                    .map(|rect| (content, draw::content_area(frame_inner(rect))))
            })
            .collect()
    }

    /// Destination allocations with enough rows for either endpoint of a move.
    /// Source geometry is retained for the whole transition, so intermediate
    /// heights cannot change a provider's measured visible slice.
    pub(in crate::tiles) fn motion_preparation_allocations(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<(TileContent<Id>, Rect)> {
        let mut allocations = self.preparation_allocations(area, growth);
        if let GridMotion::Moving(transition) = &self.motion {
            let source_cells = cells(&transition.from);
            for (content, allocation) in &mut allocations {
                if let Some((_, index)) = source_cells.iter().find(|(source, _)| source == content)
                    && let Some(source) = transition.source.cell(*index)
                {
                    let source = draw::content_area(frame_inner(source));
                    allocation.height = allocation.height.max(source.height);
                }
            }
        }
        allocations
    }

    /// Destination state used to ask cells for their wrapped row demands.
    fn demand_measurement_held(&self) -> HeldCellLayout {
        let mut held = self.drawn_held();
        if matches!(
            self.motion,
            GridMotion::Moving(Transition {
                depth_status: DestinationDepthStatus::AwaitingMeasuredWidths,
                ..
            })
        ) {
            held.summary_depth = 1;
        }
        held
    }

    /// Record the geometry the pane just laid out.
    pub fn set_layout(&mut self, area: Rect, growth: TileGrowth) {
        self.update_layout_rules(area, EndpointLayoutRules::new(growth, &self.settings));
    }

    /// The only rule-writing boundary also retires geometry made under the old rules.
    fn update_layout_rules(&mut self, area: Rect, rules: EndpointLayoutRules) {
        if self.area != area || EndpointLayoutRules::new(self.growth, &self.settings) != rules {
            self.area = area;
            self.growth = rules.growth;
            self.settings.min_tile_width = rules.min_tile_width;
            self.settings.min_tile_height = rules.min_tile_height;
            self.settings.tile_width_floor = rules.width_floor;
            self.settings.summary_row_allocation = rules.summary_allocation;
            self.replan_layout_endpoints();
        }
    }

    /// A changed allocation cannot inherit geometry measured under the old floors.
    /// Keep the requested final cell order, then resolve it through the checked planner.
    fn replan_layout_endpoints(&mut self) {
        self.layout_admission.take();
        let depth = self.target_depth();
        self.complete_arrangement(self.target(), depth);
        self.pending.clear();
        self.motion = GridMotion::Settled;
        self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
        self.frame_row_usage.clear();
        self.motion_heights = MotionHeightPath::default();
        self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        let rows = cell_wants(&self.demands, &self.slots, &self.settings);
        self.depth = depth;
        self.held = HeldCellLayout {
            rows,
            focused: self.focused_cell(),
            summary_span: self.wanted_span(),
            summary_depth: self.depth,
        };
        self.commit_display();
    }

    /// Record one drawn layout, completing a move that began in another area.
    pub(in crate::tiles) fn set_draw_layout(&mut self, area: Rect, growth: TileGrowth) {
        let area_changed = self.area != area;
        self.set_layout(area, growth);
        if area_changed {
            self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        }
        if area_changed && matches!(self.motion, GridMotion::Moving(_)) {
            self.advance();
        }
    }

    /// Take the grid to its next step once the one in flight has run
    /// its course, reporting whether it still wants repainting.
    ///
    /// One step at a time, always. A scan that closes two cells and
    /// opens a third is three steps, and they play in order rather than
    /// at once -- three separate travels are followable where three
    /// overlaid on each other are not.
    pub fn tick(&mut self) -> bool {
        if !matches!(self.motion, GridMotion::Settled) && self.progress() >= PROGRESS_SCALE {
            self.advance();
        }
        self.visual_repaint() == Repaint::Needed
    }

    /// Whether the next body differs from the last paint. Polling does not consume it.
    #[must_use]
    pub fn visual_repaint(&self) -> crate::Repaint { self.visual_repaint_at(self.progress()) }

    pub(super) fn visual_repaint_at(&self, progress: u32) -> crate::Repaint {
        let LastPainted::Painted { visual, .. } = &self.last_painted else {
            return if matches!(self.motion, GridMotion::Settled) {
                Repaint::NotNeeded
            } else {
                Repaint::Needed
            };
        };
        let drawing = self.visible_drawing(visual.contents, progress);
        // Height-search hints affect measurement, not pixels. The selected frames carry
        // the visible heights; title ownership and column bands also affect pixels.
        let same_pieces = drawing.pieces.len() == visual.drawing.pieces.len()
            && drawing
                .pieces
                .iter()
                .zip(&visual.drawing.pieces)
                .all(|(current, painted)| {
                    current.identity == painted.identity
                        && current.placement == painted.placement
                        && current.name == painted.name
                });
        if same_pieces
            && drawing.column_bands == visual.drawing.column_bands
            && visual.theme == crate::theme()
            && visual.transparent == BackgroundTransparency::current()
            && visual.tint == FocusedPaneTint::current()
        {
            Repaint::NotNeeded
        } else {
            Repaint::Needed
        }
    }

    pub(in crate::tiles) fn visible_drawing(
        &self,
        contents: crate::TileGridContents,
        progress: u32,
    ) -> TileDrawing<Id> {
        if self.display() == GridDisplay::SummaryAlone
            && (contents == TileGridContents::Hidden || crate::frame_inner(self.area).is_empty())
        {
            TileDrawing {
                pieces:       Vec::new(),
                column_bands: Vec::new(),
            }
        } else {
            self.drawing_at(self.area, self.growth, progress)
        }
    }

    /// Give every running command a cell and take back the cells whose
    /// command has gone.
    ///
    /// A command arriving takes the first cell `+` opened and nothing has
    /// claimed, so a developer who made room in advance sees the build
    /// land in it. With no empty cell waiting it opens one of its own. A
    /// command leaving closes its cell wherever that sat, and the grid
    /// comes together over it.
    ///
    /// The cells read in the order `ids` gives, which is the order every
    /// table on the screen reads in: oldest work first. That order is not
    /// fixed once a cell is open -- a driver's cell sorts by the earliest
    /// work still under it, so the last of a queue finishing carries the
    /// cell past whatever started in between. Reaching the new order by
    /// moving cells drew the two of them travelling through each other,
    /// which is the one motion the grid never makes. So a cell the order
    /// has overtaken closes instead, and comes back in behind the cell
    /// that passed it; every cell after it does the same, which is what
    /// carries the change down the rest of the grid a cell at a time.
    ///
    /// The comparison is against where the grid is *headed* rather than
    /// where it stands, so a scan arriving mid-travel adds to the queue
    /// instead of starting the same change over.
    ///
    /// A scan that moves no cell can still re-divide a column: what the
    /// cells are asking for is part of the arrangement, so a command
    /// that has taken on enough new invocations to want another unit of
    /// its column travels there the same way a cell opening does.
    pub fn sync(&mut self, demands: &TileDemands<Id>, growth: TileGrowth) {
        self.sync_demands(demands, growth, DemandSync::Independent);
    }

    /// Fold fixed render demands into the destination in internal tests.
    #[cfg(test)]
    pub(in crate::tiles) fn sync_rendered_demands(
        &mut self,
        demands: &TileDemands<Id>,
        growth: TileGrowth,
    ) {
        self.sync_demands(demands, growth, DemandSync::CurrentRender);
    }

    /// Reconcile fixed demands from a scan or an internal test.
    fn sync_demands(&mut self, demands: &TileDemands<Id>, growth: TileGrowth, sync: DemandSync) {
        self.sync_selected_demands(demands, growth, sync, Self::fixed_selection);
    }

    /// Measure current widths before searching changed layout inputs.
    pub(in crate::tiles) fn sync_measured_demands(
        &mut self,
        growth: TileGrowth,
        order: TileColumnOrder<Id>,
        mut measure: impl FnMut(&[(TileContent<Id>, u16)]) -> TileDemands<Id>,
    ) -> (SelectedLayoutMeasurements<Id>, ColumnPreparation) {
        let previous_slots = self.slots.clone();
        let previous_depth = self.depth;
        let ranks = self.column_ranks.refresh(order);
        if self.column_ranks != ranks {
            self.column_ranks = ranks;
            self.selected_plan = SelectedLayoutPlan::Unmeasured;
        }
        let widths = self.content_widths(self.area, growth);
        let initial = measure(&widths);
        let mut plan = std::mem::replace(&mut self.selected_plan, SelectedLayoutPlan::Unmeasured);
        let selected = self.sync_selected_demands(
            &initial,
            growth,
            DemandSync::CurrentRender,
            |grid, target| {
                let mut inputs = grid.selection_inputs(target, &widths, &initial);
                let mut freshness = CandidateMeasurementFreshness::CurrentWidth;
                if let SelectedLayoutPlan::Selected(previous) = &mut plan {
                    freshness = inputs.refresh_measurements(&previous.inputs, &mut measure);
                    if previous.inputs == inputs {
                        previous.inputs = inputs;
                        return previous.selected.clone();
                    }
                }
                if matches!(freshness, CandidateMeasurementFreshness::CurrentWidth) {
                    inputs.measurements = vec![LayoutDemandMeasurement {
                        widths:  widths.clone(),
                        demands: initial.clone(),
                    }];
                }
                let mut selected =
                    grid.search_destination(target, initial.summary_width, &mut |candidate| {
                        if let Some(measured) = inputs
                            .measurements
                            .iter()
                            .find(|measured| measured.widths == candidate)
                        {
                            return measured.demands.clone();
                        }
                        let demands = measure(candidate);
                        inputs.measurements.push(LayoutDemandMeasurement {
                            widths:  candidate.to_vec(),
                            demands: demands.clone(),
                        });
                        demands
                    });
                if selected.widths.is_empty() {
                    selected.widths.clone_from(&widths);
                }
                inputs.target.clone_from(&selected.slots);
                plan = SelectedLayoutPlan::Selected(Box::new(LayoutSelection {
                    inputs,
                    selected: selected.clone(),
                }));
                selected
            },
        );
        self.selected_plan = plan;
        let preparation = if !self.column_ranks.keeps_source_order()
            && (self.slots != previous_slots || self.depth != previous_depth)
        {
            ColumnPreparation::Reassigned
        } else {
            ColumnPreparation::Prepared
        };
        (selected, preparation)
    }

    /// Read external inputs after the grid reconciles target slots.
    fn selection_inputs(
        &self,
        target: &[Slot<Id>],
        widths: &[(TileContent<Id>, u16)],
        demands: &TileDemands<Id>,
    ) -> LayoutSelectionInputs<Id> {
        LayoutSelectionInputs {
            area:         self.area,
            target:       target.to_vec(),
            growth:       self.growth,
            rules:        EndpointLayoutRules::new(self.growth, &self.settings),
            measurements: vec![LayoutDemandMeasurement {
                widths:  widths.to_vec(),
                demands: demands.clone(),
            }],
        }
    }

    /// Select depth from fixed demands when no provider can measure other widths.
    fn fixed_selection(&self, target: &[Slot<Id>]) -> SelectedLayoutMeasurements<Id> {
        SelectedLayoutMeasurements {
            widths:  Vec::new(),
            demands: self.demands.clone(),
            slots:   target.to_vec(),
            depth:   summary_depth(
                self.area,
                &cell_wants(&self.demands, target, &self.settings),
                self.growth,
                &self.settings,
            ),
        }
    }

    /// Select the first depth that holds the summary at its own measured widths.
    fn search_destination(
        &self,
        target: &[Slot<Id>],
        preferred_width: u16,
        measure: &mut impl FnMut(&[(TileContent<Id>, u16)]) -> TileDemands<Id>,
    ) -> SelectedLayoutMeasurements<Id> {
        let mut selected = self.fixed_selection(target);
        for depth in 1..=usize::from(self.area.height).max(1) {
            let ColumnPlan::Columns(base) = column_layout(
                target.len() + TABLE_CELL,
                depth,
                self.growth,
                self.area,
                &self.settings,
            ) else {
                break;
            };
            if base.columns.summary_depth != depth {
                break;
            }
            if let CandidateDepthSelection::Selected(candidate) =
                self.select_depth(target, &base.columns, preferred_width, measure)
            {
                let stepped: Vec<_> =
                    cell_wants(&candidate.demands, &candidate.slots, &self.settings)
                        .into_iter()
                        .map(|want| CellRowDemand::uniform(want.stepped_framed()))
                        .collect();
                let asked = stepped.first().map_or(0, |want| want.stepped_framed());
                let capacity = summary_share(
                    &stepped,
                    self.area.height,
                    depth,
                    self.growth,
                    self.settings.min_tile_height.get(),
                    self.area,
                    &self.settings,
                );
                selected = candidate;
                if capacity >= asked {
                    break;
                }
            }
        }
        selected
    }

    /// Test each permitted column count and summary span at their own widths.
    fn select_depth(
        &self,
        target: &[Slot<Id>],
        base: &ColumnLayout,
        preferred_width: u16,
        measure: &mut impl FnMut(&[(TileContent<Id>, u16)]) -> TileDemands<Id>,
    ) -> CandidateDepthSelection<Id> {
        for columns in (1..=base.widths.len()).rev() {
            let spans = if self.growth.widen_summary {
                columns
            } else {
                1
            };
            for span in 1..=spans {
                let slots = self.ranked_candidate(target, base, columns);
                let widths = candidate_content_widths(&slots, self.area, base, columns, span);
                let mut demands = measure(&widths);
                demands.summary_width = preferred_width;
                let held = self.candidate_held(&slots, base, &demands);
                let layout = Grid::with_row_use(
                    self.area,
                    &held,
                    self.growth,
                    &self.settings,
                    &mut |_, area, _| area.height,
                );
                let destination: Vec<_> = cells(&slots)
                    .into_iter()
                    .filter_map(|(content, index)| {
                        layout
                            .cell(index)
                            .map(|rect| (content, frame_inner(rect).width))
                    })
                    .collect();
                if widths == destination {
                    return CandidateDepthSelection::Selected(SelectedLayoutMeasurements {
                        widths,
                        demands,
                        depth: base.summary_depth,
                        slots,
                    });
                }
            }
        }
        CandidateDepthSelection::NoFit
    }

    /// Judge a candidate without retained rows or previous frame measurements.
    fn candidate_held(
        &self,
        target: &[Slot<Id>],
        base: &ColumnLayout,
        demands: &TileDemands<Id>,
    ) -> HeldCellLayout {
        HeldCellLayout {
            rows:          cell_wants(demands, target, &self.settings),
            focused:       FocusLocation::Departing,
            summary_span:  if self.growth.widen_summary {
                summary_span(self.area, base.widths.len(), demands.summary_width)
            } else {
                1
            },
            summary_depth: base.summary_depth,
        }
    }

    /// Replace demands and retire admission checks that used the old demands.
    fn replace_demands(&mut self, demands: &TileDemands<Id>) {
        if self.demands != *demands {
            self.layout_admission.take();
            self.demands = demands.clone();
        }
    }

    /// Reconcile one demand observation with the headed arrangement.
    fn sync_selected_demands(
        &mut self,
        demands: &TileDemands<Id>,
        growth: TileGrowth,
        demand_sync: DemandSync,
        select: impl FnOnce(&Self, &[Slot<Id>]) -> SelectedLayoutMeasurements<Id>,
    ) -> SelectedLayoutMeasurements<Id> {
        let row_change = RowDemandChange::between(&self.demands, demands);
        self.replace_demands(demands);
        self.set_layout(self.area, growth);
        if demand_sync == DemandSync::CurrentRender {
            self.fold_rendered_claims();
        }
        let mut arrangement = self.target();
        let mut steps: Vec<Vec<Slot<Id>>> = Vec::new();
        let live = demands.ids();
        Self::remove_departed(&mut arrangement, &live, &mut steps);
        // Worked out against the cells the departed have already given
        // back, so a command arriving in the same scan another leaves
        // takes the room that leaving freed.
        let ids = self.admitted(&arrangement, live, growth);
        // Every cell from the first one out of order onward goes, and
        // the cells before it are the run the grid gets to keep.
        let standing = if self.column_ranks.keeps_source_order() {
            kept(&arrangement, &ids)
        } else {
            ids.len()
        };
        let mut reopening: Vec<Id> = Vec::new();
        while let Some((index, id)) =
            arrangement
                .iter()
                .enumerate()
                .find_map(|(index, slot)| match slot {
                    Slot::Group(id) if !ids.iter().take(standing).any(|held| held == id) => {
                        Some((index, id.clone()))
                    },
                    _ => None,
                })
        {
            reopening.push(id);
            Self::close(&mut arrangement, index, &mut steps);
        }
        for id in ids {
            if arrangement
                .iter()
                .any(|slot| matches!(slot, Slot::Group(held) if held == &id))
            {
                continue;
            }
            // Behind every cell already carrying a command, because the
            // ones still standing are exactly the commands that sort
            // ahead of this one. An empty cell keeps its place: it is a
            // place the reader made, and only a command genuinely
            // arriving claims one -- a cell that closed to let the order
            // through brings its own back rather than eating it.
            let behind = arrangement
                .iter()
                .rposition(|slot| matches!(*slot, Slot::Group(_)))
                .map_or(0, |index| index.saturating_add(1));
            let claimed = if reopening.contains(&id) {
                None
            } else {
                arrangement
                    .iter()
                    .skip(behind)
                    .position(|slot| matches!(*slot, Slot::Empty(_)))
                    .map(|offset| behind.saturating_add(offset))
            };
            match claimed {
                Some(index) => {
                    let empty = arrangement[index].clone();
                    arrangement[index] = Slot::Group(id.clone());
                    if demand_sync != DemandSync::CurrentRender
                        || !self.claim_moving_empty(&empty, &id)
                    {
                        steps.push(arrangement.clone());
                    }
                },
                // Refusing beats a cell too small to read: the command
                // is still in the summary, which is never crowded out.
                None if self.fits(arrangement.len() + TABLE_CELL + 1, growth) => {
                    arrangement.insert(behind, Slot::Group(id));
                    steps.push(arrangement.clone());
                },
                None => (),
            }
        }
        // Nothing moved, but the cells standing still may have changed
        // what they are asking for. Re-dividing the column is a step
        // like any other, and queued as one so it travels rather than
        // snapping.
        let selected = select(self, &arrangement);
        if selected.slots != arrangement {
            steps.push(selected.slots.clone());
        }
        let wanted_depth = selected.depth;
        self.replace_demands(&selected.demands);
        if steps.is_empty() && matches!(self.motion, GridMotion::Moving(_)) {
            if demand_sync == DemandSync::CurrentRender {
                self.resolve_selected_destination_depth(wanted_depth);
            }
            return selected;
        }
        let settled = self.reconciled_held(demand_sync, row_change);
        if steps.is_empty()
            && wanted_depth == self.target_depth()
            && matches!(self.motion, GridMotion::Settled)
            && settled.needs_motion_from(&self.held)
        {
            self.queue_row_resize(settled);
            return selected;
        } else if steps.is_empty()
            && wanted_depth == self.target_depth()
            && matches!(self.motion, GridMotion::Settled)
        {
            if self.held != settled {
                self.layout_admission.take();
            }
            self.held = settled;
        }
        self.queue_with_depth(steps, wanted_depth, DestinationDepthStatus::Resolved);
        selected
    }

    /// Remove retired groups without changing the identities of empty cells.
    fn remove_departed(
        arrangement: &mut Vec<Slot<Id>>,
        live: &[Id],
        steps: &mut Vec<Vec<Slot<Id>>>,
    ) {
        while let Some(index) = arrangement.iter().position(|slot| match slot {
            Slot::Group(id) => !live.contains(id),
            Slot::Empty(_) => false,
        }) {
            Self::close(arrangement, index, steps);
        }
    }

    /// Keep each candidate's summary and empty slots while assigning detail ranks.
    fn ranked_candidate(
        &self,
        target: &[Slot<Id>],
        base: &ColumnLayout,
        columns: usize,
    ) -> Vec<Slot<Id>> {
        self.column_ranks.arrange(
            target,
            &base.column_capacities(target.len(), columns),
            base.summary_depth,
        )
    }

    /// Changed render rows release retained height before the grid divides its columns.
    fn reconciled_held(&self, sync: DemandSync, change: RowDemandChange) -> HeldCellLayout {
        match (sync, change) {
            (DemandSync::CurrentRender, RowDemandChange::Changed) => self.current_demands_held(),
            _ => self.settled_held(),
        }
    }

    /// A reassigned tile receives its own measured demand at the destination.
    fn current_demands_held(&self) -> HeldCellLayout {
        HeldCellLayout {
            rows:          cell_wants(&self.demands, &self.slots, &self.settings),
            focused:       self.focused_cell(),
            summary_span:  self.wanted_span(),
            summary_depth: self.depth,
        }
    }

    /// Queue a resize from the drawn source, then install its measured destination.
    fn queue_row_resize(&mut self, destination: HeldCellLayout) {
        self.queue_with_depth(
            vec![self.slots.clone()],
            destination.summary_depth,
            DestinationDepthStatus::Resolved,
        );
        self.held = destination;
        self.layout_admission.take();
    }

    /// Which of `live` the grid can actually give a cell to: the ones
    /// already holding one, plus as many of the rest as there is room
    /// to open.
    ///
    /// Settled before anything closes, because a command the grid has
    /// no room for is left out of the order entirely. Left in, it would
    /// stop the order at its own place and send every cell behind it
    /// through a close and an open to make way for a cell that never
    /// opens.
    fn admitted(&self, arrangement: &[Slot<Id>], live: Vec<Id>, growth: TileGrowth) -> Vec<Id> {
        let mut spare = arrangement
            .iter()
            .filter(|slot| matches!(slot, Slot::Empty(_)))
            .count();
        let mut count = spare;
        let mut retained = Vec::new();
        let all_fit = self.fits(arrangement.len() + TABLE_CELL, growth);
        for slot in arrangement {
            if let Slot::Group(id) = slot
                && (all_fit || self.fits(count + TABLE_CELL + 1, growth))
            {
                retained.push(id);
                count += 1;
            }
        }
        live.into_iter()
            .filter(|id| {
                if retained.contains(&id) {
                    return true;
                }
                if arrangement
                    .iter()
                    .any(|slot| matches!(slot, Slot::Group(held) if held == id))
                {
                    return false;
                }
                if spare > 0 {
                    spare -= 1;
                    return true;
                }
                if self.fits(count + TABLE_CELL + 1, growth) {
                    count += 1;
                    return true;
                }
                false
            })
            .collect()
    }

    /// Push the step that closes the cell at `index`.
    ///
    /// The cell goes and the grid comes together over the space it
    /// held: what stood above it and what stood below meet, every cell
    /// after it moving up one place at once. Keying the cells by slot
    /// is what makes that a single travel each rather than contents
    /// shuffling between cells that never moved.
    ///
    /// The hole is not walked to the end of the grid a neighbour at a
    /// time, which is what this used to do. Every swap along the way
    /// drew the hole travelling down through the cell travelling up --
    /// two boxes crossing, where the eye was looking for one closing.
    fn close(arrangement: &mut Vec<Slot<Id>>, index: usize, steps: &mut Vec<Vec<Slot<Id>>>) {
        let _ = arrangement.remove(index);
        steps.push(arrangement.clone());
    }

    /// Open an empty cell at the end, unless the grid it would make no
    /// longer fits.
    ///
    /// Refusing beats filling the pane with cells too small to carry a
    /// border and a row: the grid stops growing at the point the
    /// terminal stops being able to show it.
    pub(super) fn add(&mut self, growth: TileGrowth) {
        if self.open_waiting_commands_in_current_motion() {
            return;
        }
        let mut arrangement = self.target();
        if !self.fits(arrangement.len() + TABLE_CELL + 1, growth) {
            return;
        }
        arrangement.push(Slot::Empty(self.next_slot));
        self.next_slot = self.next_slot.saturating_add(1);
        self.queue(vec![arrangement]);
    }

    /// A requested opening serves commands already waiting for their cells.
    fn open_waiting_commands_in_current_motion(&mut self) -> bool {
        if !matches!(self.motion, GridMotion::Moving(_)) || self.pending.is_empty() {
            return false;
        }
        let target = self.target();
        if target.len() <= self.slots.len() {
            return false;
        }
        let mut preceding = &self.slots;
        for step in &self.pending {
            if !preceding.iter().all(|slot| step.slots.contains(slot))
                || step
                    .slots
                    .iter()
                    .any(|slot| !preceding.contains(slot) && matches!(slot, Slot::Empty(_)))
            {
                return false;
            }
            preceding = &step.slots;
        }
        let progress = self.motion_heights.progress;
        let drawn = self.drawing_at(self.area, self.growth, progress);
        self.pending.clear();
        self.complete_arrangement(target, self.depth);
        self.resolve_key_destination_depth();
        if !self.motion_heights.piece_heights.is_empty() {
            let next = self.drawing_at_with_height_snap(
                self.area,
                self.growth,
                progress,
                &HeightSnap::Off,
            );
            self.motion_heights.piece_heights = next
                .pieces
                .iter()
                .map(|piece| {
                    let placement = &piece.placement;
                    drawn
                        .pieces
                        .iter()
                        .map(|piece| &piece.placement)
                        .find(|previous| {
                            previous.content == placement.content
                                && previous.frame.clip().x == placement.frame.clip().x
                        })
                        .map_or(1, |previous| previous.frame.clip().height)
                })
                .collect();
        }
        true
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
    /// [`Self::close`], so taking one out of the middle closes the grid
    /// over it rather than leaving a hole behind to travel.
    pub(super) fn remove(&mut self) {
        let mut arrangement = self.target();
        let Some(index) = self.removable(&arrangement) else {
            return;
        };
        let mut steps = Vec::new();
        Self::close(&mut arrangement, index, &mut steps);
        self.queue(steps);
    }

    /// Which cell `-` takes out of `arrangement`, or `None` when it
    /// holds no empty one.
    fn removable(&self, arrangement: &[Slot<Id>]) -> Option<usize> {
        if let Focus::Cell(slot @ Slot::Empty(_)) = &self.focus
            && let Some(index) = arrangement.iter().position(|held| held == slot)
        {
            return Some(index);
        }
        arrangement
            .iter()
            .rposition(|slot| matches!(*slot, Slot::Empty(_)))
    }

    /// The arrangement the grid is headed for: the last step queued, or
    /// what it is showing when nothing is queued.
    pub(super) fn target(&self) -> Vec<Slot<Id>> {
        self.pending
            .back()
            .map_or_else(|| self.slots.clone(), |step| step.slots.clone())
    }

    /// Summary depth at the end of the queued arrangement.
    pub(super) fn target_depth(&self) -> usize {
        self.pending.back().map_or(self.depth, |step| step.depth)
    }

    /// Queue `steps`, starting the first one when nothing is in flight.
    ///
    /// One scan's worth of change is spread over
    /// [`TileSettings::animation_millis`] however many steps it takes,
    /// down to [`TileSettings::min_step_millis`] a step, so a single cell
    /// closing keeps the full unhurried travel and a burst of them runs
    /// at one steady pace instead of dragging. Past
    /// [`TileSettings::max_pending_steps`] the grid
    /// gives the sequence up and settles the rest in one move: a whole
    /// suite finishing at once would take longer to play through than
    /// anyone would watch.
    pub(super) fn queue(&mut self, steps: Vec<Vec<Slot<Id>>>) {
        let final_slots = steps.last().cloned().unwrap_or_else(|| self.target());
        let wants = cell_wants(&self.demands, &final_slots, &self.settings);
        let wanted = summary_depth(self.area, &wants, self.growth, &self.settings);
        self.queue_with_depth(
            steps,
            wanted,
            DestinationDepthStatus::AwaitingMeasuredWidths,
        );
    }

    /// Queue arrangements with the summary depth each one ends at.
    ///
    /// A slot move and its depth change share one step. With no slot move,
    /// each depth unit remains its own step.
    pub(super) fn queue_with_depth(
        &mut self,
        steps: Vec<Vec<Slot<Id>>>,
        wanted: usize,
        final_depth_status: DestinationDepthStatus,
    ) {
        let start_slots = self.target();
        let final_slots = steps.last().cloned().unwrap_or_else(|| self.target());
        let headed = self.target_depth();
        let mut depth = headed;
        let mut moves = Vec::new();
        if steps.is_empty() {
            while depth != wanted {
                if depth > wanted {
                    depth -= 1;
                } else {
                    depth += 1;
                }
                moves.push((start_slots.clone(), depth, DestinationDepthStatus::Resolved));
            }
        } else {
            let step_count = steps.len();
            for (index, slots) in steps.into_iter().enumerate() {
                let final_step = index + 1 == step_count;
                depth = if final_step {
                    wanted
                } else {
                    summary_depth(
                        self.area,
                        &cell_wants(&self.demands, &slots, &self.settings),
                        self.growth,
                        &self.settings,
                    )
                };
                let depth_status = if final_step {
                    final_depth_status
                } else {
                    DestinationDepthStatus::Resolved
                };
                moves.push((slots, depth, depth_status));
            }
        }
        moves.retain(|(slots, depth, _)| {
            queued_cells_keep_endpoint_columns(
                &QueuedArrangement {
                    slots:         &start_slots,
                    summary_depth: headed,
                },
                &QueuedArrangement {
                    slots:         &final_slots,
                    summary_depth: wanted,
                },
                &QueuedArrangement {
                    slots,
                    summary_depth: *depth,
                },
                self.growth,
                self.area,
                &self.settings,
            )
        });
        self.queue_steps(moves);
    }

    /// Give the active and queued destinations their current rendered demands.
    pub(super) fn resolve_key_destination_depth(&mut self) {
        let wanted = summary_depth(
            self.area,
            &cell_wants(&self.demands, &self.slots, &self.settings),
            self.growth,
            &self.settings,
        );
        self.resolve_selected_destination_depth(wanted);
    }

    /// Resolve the active destination from candidate measurements.
    fn resolve_selected_destination_depth(&mut self, wanted: usize) {
        let GridMotion::Moving(transition) = &mut self.motion else {
            return;
        };
        transition.depth_status = DestinationDepthStatus::Resolved;
        if self.depth != wanted {
            self.layout_admission.take();
        }
        self.depth = wanted;
        let held = self.settled_held();
        if self.held != held {
            self.layout_admission.take();
        }
        self.held = held;
        let final_step = self.pending.len().saturating_sub(1);
        for (index, step) in self.pending.iter_mut().enumerate() {
            let depth = if index == final_step {
                wanted
            } else {
                summary_depth(
                    self.area,
                    &cell_wants(&self.demands, &step.slots, &self.settings),
                    self.growth,
                    &self.settings,
                )
            };
            if step.depth != depth {
                self.layout_admission.take();
                step.depth = depth;
            }
        }
    }

    /// Fold measured empty-cell handovers into the motion already drawing them.
    fn fold_rendered_claims(&mut self) {
        if !matches!(self.motion, GridMotion::Moving(_)) {
            return;
        }
        let mut preceding = &self.slots;
        let mut claimed_steps = Vec::new();
        let mut handovers = Vec::new();
        for (index, step) in self.pending.iter().enumerate() {
            let only_claims = preceding.len() == step.slots.len()
                && preceding
                    .iter()
                    .zip(&step.slots)
                    .all(|(before, after)| before == after || claims(before, after));
            if only_claims {
                let start = handovers.len();
                for (before, after) in preceding.iter().zip(&step.slots) {
                    if let (Slot::Empty(_), Slot::Group(id)) = (before, after) {
                        handovers.push((before.clone(), id.clone()));
                    }
                }
                if start < handovers.len() {
                    claimed_steps.push(index);
                }
            }
            preceding = &step.slots;
        }
        for (empty, id) in handovers {
            self.claim_moving_empty(&empty, &id);
        }
        let mut index = 0;
        self.pending.retain(|_| {
            let keep = !claimed_steps.contains(&index);
            index += 1;
            keep
        });
    }

    /// Replace the same empty identity wherever a running motion carries it.
    fn claim_moving_empty(&mut self, empty: &Slot<Id>, id: &Id) -> bool {
        let GridMotion::Moving(transition) = &mut self.motion else {
            return false;
        };
        self.layout_admission.take();
        let claim = |slots: &mut [Slot<Id>]| {
            for slot in slots {
                if slot == empty {
                    *slot = Slot::Group(id.clone());
                }
            }
        };
        claim(&mut transition.from);
        claim(&mut self.slots);
        for step in &mut self.pending {
            claim(&mut step.slots);
        }
        if self.focus == Focus::Cell(empty.clone()) {
            self.focus = Focus::Cell(Slot::Group(id.clone()));
        }
        true
    }

    /// Queue arrangements paired with their summary depth.
    fn queue_steps(&mut self, steps: Vec<(Vec<Slot<Id>>, usize, DestinationDepthStatus)>) {
        if steps.is_empty() {
            return;
        }
        self.layout_admission.take();
        let millis = u64::try_from(steps.len())
            .ok()
            .and_then(|count| self.settings.animation_millis.checked_div(count))
            .unwrap_or(self.settings.animation_millis)
            .max(self.settings.min_step_millis);
        self.pending
            .extend(steps.into_iter().map(|(slots, depth, depth_status)| Step {
                slots,
                depth,
                millis,
                depth_status,
            }));
        if self.pending.len() > self.settings.max_pending_steps
            && let Some(last) = self.pending.pop_back()
        {
            self.pending.clear();
            self.pending.push_back(Step {
                millis: self.settings.animation_millis,
                ..last
            });
        }
        if matches!(self.motion, GridMotion::Settled) {
            self.advance();
        }
    }

    /// Move into the next queued step, or settle when there is none.
    ///
    /// The units the step leaves are worked out here rather than by the
    /// caller, so a step that only re-divides a column and a step that
    /// moves cells land the same way: the arrangement being left keeps
    /// the units it was drawn at, and the one arriving takes whatever
    /// the current demands and focus come to.
    pub(super) fn advance(&mut self) {
        self.layout_admission.take();
        let drawn_held = self.drawn_held();
        let Some(step) = self.pending.pop_front() else {
            self.motion_heights = MotionHeightPath::default();
            self.motion = GridMotion::Settled;
            self.held = drawn_held;
            self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
            return;
        };
        let was_settled = matches!(self.motion, GridMotion::Settled);
        let last_settled_geometry = std::mem::take(&mut self.last_settled_geometry);
        let source = match (was_settled, last_settled_geometry) {
            (true, LastSettledCellGeometry::Drawn { area, growth, grid })
                if area == self.area
                    && growth == self.growth
                    && grid.is_current_endpoint(self.area, self.growth, &self.settings) =>
            {
                grid
            },
            _ => self.resolved_grid(self.area, &drawn_held, self.growth),
        };
        let source = self.last_drawn_motion_source(source, &step.slots);
        self.motion_heights = MotionHeightPath::default();
        let previous = self.complete_arrangement(step.slots, step.depth);
        self.motion = GridMotion::Moving(Transition {
            from:         previous,
            held:         drawn_held,
            source:       Box::new(source),
            started:      Instant::now(),
            millis:       step.millis,
            depth_status: step.depth_status,
        });
    }

    /// Install an arrival and settle its identity-dependent state against the slots it left.
    fn complete_arrangement(&mut self, slots: Vec<Slot<Id>>, depth: usize) -> Vec<Slot<Id>> {
        self.layout_admission.take();
        self.retargeted_destination = RetargetedMotionDestination::Unconstrained;
        let previous = std::mem::replace(&mut self.slots, slots);
        self.depth = depth;
        self.settle_focus(&previous);
        self.held = if !self.column_ranks.keeps_source_order() && previous != self.slots {
            self.current_demands_held()
        } else {
            self.settled_held()
        };
        previous
    }

    /// Keep the drawn heights when a queued key follows a column's motion.
    fn last_drawn_motion_source(&self, mut source: Grid, next_slots: &[Slot<Id>]) -> Grid {
        if self.motion_heights.piece_heights.is_empty()
            || self.motion_heights.area != self.area
            || self.motion_heights.growth != self.growth
        {
            return source;
        }
        let drawing = self.drawing_at(self.area, self.growth, self.motion_heights.progress);
        if drawing.column_bands != source.columns {
            return source;
        }
        let contents = cells(&self.slots);
        let mut panes = Vec::with_capacity(contents.len());
        for (content, pane) in &contents {
            let mut pieces = drawing
                .pieces
                .iter()
                .filter(|piece| &piece.placement.content == content);
            let Some(piece) = pieces.next() else {
                return source;
            };
            let frame = piece.placement.frame;
            let survives = pane
                .checked_sub(TABLE_CELL + 1)
                .and_then(|index| self.slots.get(index))
                .is_none_or(|slot| next_slots.contains(slot));
            if pieces.next().is_some()
                || frame.shift() != 0
                || (survives && frame.clip().height < source.rules.min_tile_height.get())
                || source.cell(*pane).is_none_or(|endpoint| {
                    frame.clip().x != endpoint.x || frame.clip().width != endpoint.width
                })
            {
                return source;
            }
            panes.push(ResolvedPane {
                pane: *pane,
                area: frame.clip(),
            });
        }
        if drawing.pieces.iter().any(|piece| {
            !contents
                .iter()
                .any(|(content, _)| content == &piece.placement.content)
                && draw::has_visible_body_row(piece.placement.frame)
        }) {
            return source;
        }
        source.resolved = ResolvedPaneLayout::new(panes);
        source
    }

    /// Re-divide the column focus just landed in or left, when the
    /// change asks for one.
    ///
    /// Queued at [`TileSettings::focus_animation_millis`] rather than through
    /// [`Self::queue`]: the ring is a key the developer is already
    /// pressing again, and the full travel a cell opening gets would
    /// leave the grid settling behind them.
    fn resize_for_focus(&mut self) {
        if matches!(self.motion, GridMotion::Moving(_)) || self.settled_held() == self.held {
            return;
        }
        self.pending.push_back(Step {
            slots:        self.slots.clone(),
            depth:        self.depth,
            millis:       self.settings.focus_animation_millis,
            depth_status: DestinationDepthStatus::Resolved,
        });
        self.advance();
    }

    /// Keep the ring on a cell that is still there and hand it back to
    /// the summary when it is not.
    ///
    /// The one cell that changes identity without going anywhere is an
    /// empty one a command has just claimed. The developer opened that
    /// cell to watch for exactly this, so the ring stays on it; every
    /// other way a slot can vanish is the cell itself leaving.
    ///
    /// Except a cell that closed to let the order through, which is
    /// coming back a step or two later. The command is still running
    /// and still the one being watched, so the ring waits for it rather
    /// than dropping to the summary and making the developer find it
    /// again. [`Self::focused_cell`] already answers `FocusLocation::Departing` while the
    /// slot is off the grid, so nothing is drawn holding it meanwhile.
    fn settle_focus(&mut self, previous: &[Slot<Id>]) {
        let Focus::Cell(slot) = &self.focus else {
            return;
        };
        if self.slots.contains(slot) {
            return;
        }
        if let Slot::Group(id) = slot
            && self.demands.holds(id)
        {
            return;
        }
        self.focus = previous
            .iter()
            .position(|held| held == slot)
            .and_then(|index| self.slots.get(index))
            .filter(|taken| claims(slot, taken))
            .map_or(Focus::Summary, |taken| Focus::Cell(taken.clone()));
    }

    /// Report active or queued motion for application render tests.
    #[doc(hidden)]
    #[must_use]
    pub fn has_pending_motion_for_test(&self) -> bool {
        !matches!(self.motion, GridMotion::Settled) || !self.pending.is_empty()
    }

    /// Hold active and later motion at half progress for render tests.
    /// Keep the area and layout rules unchanged while motion is held.
    #[doc(hidden)]
    pub const fn hold_motion_at_half_for_test(&mut self) {
        self.motion_clock = MotionClock::HeldAtHalf;
    }

    /// Set zero-duration motion steps for tests that advance the grid through `tick()`.
    #[doc(hidden)]
    pub const fn set_instant_motion_for_test(&mut self) {
        self.motion_clock = MotionClock::Elapsed;
        self.settings.animation_millis = 0;
        self.settings.min_step_millis = 0;
        self.settings.focus_animation_millis = 0;
    }

    /// Play every queued step at once, for tests that care where the
    /// grid ends up rather than how it gets there -- a render golden
    /// among them, since each step otherwise runs for as long as its
    /// travel takes on the wall clock.
    #[doc(hidden)]
    pub fn settle_for_test(&mut self) {
        self.layout_admission.take();
        while !self.pending.is_empty() {
            self.advance();
        }
        self.motion = GridMotion::Settled;
        self.last_settled_geometry = LastSettledCellGeometry::NoCellsFrame;
        self.motion_heights = MotionHeightPath::default();
    }

    /// Make a multi-step test transition run directly between its endpoints.
    #[cfg(test)]
    pub(in crate::tiles) const fn collapse_queued_changes_for_test(&mut self) {
        self.settings.max_pending_steps = 1;
    }

    /// Complete only the active motion so tests can inspect each queued arrival.
    #[cfg(test)]
    pub(in crate::tiles) fn finish_motion_for_test(&mut self) { self.advance(); }

    /// Read a motion's clock without sampling elapsed time.
    #[cfg(test)]
    pub(in crate::tiles) const fn motion_timing_for_test(&self) -> MotionTiming {
        match &self.motion {
            GridMotion::Settled => MotionTiming::Settled,
            GridMotion::Moving(transition) => MotionTiming::Active {
                started: transition.started,
                millis:  transition.millis,
            },
        }
    }

    /// Exact framed summary demand at the active destination.
    #[cfg(test)]
    pub(in crate::tiles) fn destination_summary_demand_for_test(&self) -> u16 {
        self.held
            .rows
            .first()
            .copied()
            .unwrap_or_default()
            .exact_framed()
    }

    /// Retain a destination measured by the current render fixture.
    #[cfg(test)]
    pub(in crate::tiles) fn retain_measured_destination_for_test(&mut self) {
        let grid = self.resolved_grid(self.area, &self.drawn_held(), self.growth);
        let contents = cells(&self.slots);
        let row_use = grid
            .resolved
            .panes
            .iter()
            .map(|pane| {
                let content = &contents[pane.pane.saturating_sub(TABLE_CELL)].0;
                let area = draw::content_area(frame_inner(pane.area));
                DestinationRowUse {
                    drawn: frame_painted_rows(&self.frame_row_usage, content, area),
                    kept:  self.frame_rows_kept(content, area),
                }
            })
            .collect();
        self.retargeted_destination = RetargetedMotionDestination::Filled {
            area: grid.area,
            growth: self.growth,
            rows: cell_wants(&self.demands, &self.slots, &self.settings),
            columns: grid.columns,
            span: grid.span,
            panes: grid.resolved.panes,
            row_use,
        };
    }

    /// Whether a filled destination remains retained for this motion.
    #[cfg(test)]
    pub(in crate::tiles) const fn has_retargeted_destination_for_test(&self) -> bool {
        matches!(
            self.retargeted_destination,
            RetargetedMotionDestination::Filled { .. }
        )
    }

    /// Record a settled one-column fixture's shared spans.
    #[cfg(test)]
    pub(in crate::tiles) fn record_settled_spans_for_test(&mut self, spans: &[u16]) {
        let mut grid = self.resolved_grid(self.area, &self.drawn_held(), self.growth);
        let mut top = self.area.top();
        let panes = spans
            .iter()
            .enumerate()
            .map(|(index, &span)| {
                let area = Rect {
                    y: top,
                    height: span.saturating_add(1),
                    ..self.area
                };
                top = top.saturating_add(span);
                ResolvedPane {
                    pane: index.saturating_add(TABLE_CELL),
                    area,
                }
            })
            .collect();
        assert_eq!(top, self.area.bottom().saturating_sub(1));
        grid.resolved = ResolvedPaneLayout::new(panes);
        self.last_settled_geometry = LastSettledCellGeometry::Drawn {
            area: self.area,
            growth: self.growth,
            grid,
        };
    }

    /// Move focus one cell in `direction`, staying put at the edges.
    ///
    /// The grid is ragged -- a column can hold fewer cells than the one
    /// beside it -- so a sideways step keeps the row it can and lands
    /// on the bottom cell of a shorter column rather than refusing to
    /// move at all.
    ///
    /// A summary widened over the next columns heads each of them as
    /// well as its own, so up from the top of one of them reaches it,
    /// and a sideways step from it goes on to the first column it does
    /// not cover.
    pub(super) fn focus_step(&mut self, direction: Direction, growth: TileGrowth) {
        if self.display() == GridDisplay::SummaryAlone {
            return;
        }
        let FocusLocation::Cell(cell) = self.focused_cell() else {
            return;
        };
        let lanes = self
            .resolved_grid(self.area, &self.drawn_held(), growth)
            .lanes();
        let Some((column, row)) = lanes.iter().enumerate().find_map(|(column, lane)| {
            lane.iter()
                .position(|&held| held == cell)
                .map(|row| (column, row))
        }) else {
            return;
        };
        let landing = |column: usize, row: usize| {
            lanes
                .get(column)
                .and_then(|lane| lane.get(row.min(lane.len().saturating_sub(1))))
                .copied()
                .filter(|&landed| landed != cell)
        };
        let next = match direction {
            Direction::Left => (0..column).rev().find_map(|to| landing(to, row)),
            Direction::Right => {
                (column.saturating_add(1)..lanes.len()).find_map(|to| landing(to, row))
            },
            Direction::Up => (0..row).rev().find_map(|above| landing(column, above)),
            Direction::Down => (row.saturating_add(1)..lanes[column].len())
                .find_map(|below| landing(column, below)),
        };
        let Some(next) = next else {
            return;
        };
        self.focus = self.focus_at(next);
        self.resize_for_focus();
    }

    /// Take the focus ring one cell along the grid's own order, the
    /// way Tab asks for it, wrapping at either end.
    ///
    /// Where the arrows read the grid as rows and columns -- a column
    /// to the left, the cell above -- this reads it as the list it is
    /// numbered as: the summary, then every command's cell in turn,
    /// then back to the summary. Reports whether it took the step, so
    /// the framework knows the key was spent here rather than on the
    /// pane cycle.
    pub fn cycle_focus(&mut self, direction: CycleDirection) -> bool {
        if self.display() == GridDisplay::SummaryAlone {
            return false;
        }
        let last = self.count();
        let FocusLocation::Cell(cell) = self.focused_cell() else {
            return false;
        };
        let next = match direction {
            CycleDirection::Next if cell >= last => TABLE_CELL,
            CycleDirection::Next => cell.saturating_add(1),
            CycleDirection::Prev if cell <= TABLE_CELL => last,
            CycleDirection::Prev => cell.saturating_sub(1),
        };
        self.focus = self.focus_at(next);
        self.resize_for_focus();
        true
    }

    /// The cell number focus rests on, or its departure from the arrangement.
    pub fn focused_cell(&self) -> FocusLocation {
        match &self.focus {
            Focus::Summary => FocusLocation::Cell(TABLE_CELL),
            Focus::Cell(slot) => {
                cell_of(&self.slots, slot).map_or(FocusLocation::Departing, FocusLocation::Cell)
            },
        }
    }

    /// What focus becomes on landing at cell `index`, left where it is
    /// when the grid has no such cell.
    fn focus_at(&self, index: usize) -> Focus<Id> {
        if index == TABLE_CELL {
            return Focus::Summary;
        }
        index
            .checked_sub(TABLE_CELL + 1)
            .and_then(|position| self.slots.get(position))
            .map_or_else(|| self.focus.clone(), |slot| Focus::Cell(slot.clone()))
    }

    /// Put focus on cell `index`, leaving it where it is when the grid
    /// has no such cell.
    pub fn focus_cell(&mut self, index: usize) {
        if self.display() == GridDisplay::SummaryAlone {
            return;
        }
        self.focus = self.focus_at(index);
        self.resize_for_focus();
    }

    /// Resolve content in the current slots without using painted geometry.
    #[must_use]
    pub fn cell_index(&self, content: &TileContent<Id>) -> CellIndex {
        match content {
            TileContent::Summary => CellIndex::Current(TABLE_CELL),
            TileContent::Group(id) => cell_of(&self.slots, &Slot::Group(id.clone()))
                .map_or(CellIndex::Missing, CellIndex::Current),
            TileContent::Empty(index) => index
                .checked_sub(TABLE_CELL + 1)
                .and_then(|position| self.slots.get(position))
                .filter(|slot| matches!(slot, Slot::Empty(_)))
                .map_or(CellIndex::Missing, |_| CellIndex::Current(*index)),
        }
    }

    /// The current cell under `pos` in the last painted body.
    /// Before the first paint, use the current drawing allocation.
    /// Retired pieces and points outside painted pieces return `None`.
    pub fn cell_at(&self, pos: Position) -> Option<usize> {
        match self.cell_hit(pos) {
            CellHit::Current { index, .. } => Some(index),
            CellHit::Missed | CellHit::Retired => None,
        }
    }

    /// Whether every cell of a `count`-cell grid would be big enough to
    /// draw in the rect the last frame used.
    pub(super) fn fits(&self, count: usize, growth: TileGrowth) -> bool {
        if !fits(self.area, count, growth, &self.settings) {
            return false;
        }
        if self.settings.summary_row_allocation == SummaryRowAllocation::Shared {
            return true;
        }
        let ColumnPlan::Columns(plan) = column_layout(count, 1, growth, self.area, &self.settings)
        else {
            return false;
        };
        let mut rows = vec![self.settings.demanded_rows(0); count];
        rows[0] = self.settings.demanded_rows(self.demands.summary);
        let held = HeldCellLayout {
            rows,
            focused: FocusLocation::Departing,
            summary_span: if growth.widen_summary {
                summary_span(
                    self.area,
                    plan.columns.widths.len(),
                    self.demands.summary_width,
                )
            } else {
                1
            },
            summary_depth: 1,
        };
        matches!(
            plan.for_summary(self.area, &held, &self.settings),
            ColumnPlan::Columns(_)
        )
    }

    /// Whether every arrangement that may still be drawn fits in `area`.
    pub(super) fn holds_in(&self, area: Rect, growth: TileGrowth) -> bool {
        let admission = self
            .layout_admission
            .get_or_init(|| self.check_admission(area, growth));
        if admission.area == area
            && admission.rules == EndpointLayoutRules::new(growth, &self.settings)
        {
            admission.fits
        } else {
            self.check_admission(area, growth).fits
        }
    }

    /// Stamp a changed arrangement once, preserving concurrent read-only access.
    fn check_admission(&self, area: Rect, growth: TileGrowth) -> CheckedLayoutAdmission {
        #[cfg(test)]
        self.admission_checks.fetch_add(1, AtomicOrdering::Relaxed);
        CheckedLayoutAdmission {
            area,
            rules: EndpointLayoutRules::new(growth, &self.settings),
            fits: self.check_arrangements(area, growth),
        }
    }

    /// Validate changed arrangements, borrowing held row demands when available.
    fn check_arrangements(&self, area: Rect, growth: TileGrowth) -> bool {
        let arrangement_fits = |slots: &[Slot<Id>], depth: usize, held: &HeldCellLayout| {
            let ColumnPlan::Columns(plan) = column_layout(
                slots.len() + TABLE_CELL,
                depth,
                growth,
                area,
                &self.settings,
            ) else {
                return false;
            };
            let measured;
            let rows = if held.rows.len() == slots.len() + TABLE_CELL {
                &held.rows
            } else {
                measured = cell_wants(&self.demands, slots, &self.settings);
                &measured
            };
            let span = if growth.widen_summary {
                summary_span(area, plan.columns.widths.len(), self.demands.summary_width)
                    .max(held.summary_span)
            } else {
                1
            };
            matches!(
                plan.for_summary_rows(area, rows, span, &self.settings),
                ColumnPlan::Columns(_)
            )
        };
        arrangement_fits(&self.slots, self.depth, &self.held)
            && match &self.motion {
                GridMotion::Settled => true,
                GridMotion::Moving(transition) => {
                    transition
                        .source
                        .is_current_endpoint(area, growth, &self.settings)
                        && arrangement_fits(
                            &transition.from,
                            transition.held.summary_depth,
                            &transition.held,
                        )
                },
            }
            && self
                .pending
                .iter()
                .all(|step| arrangement_fits(&step.slots, step.depth, &HeldCellLayout::default()))
    }

    /// The one display state shared by drawing, hit-testing and focus movement.
    pub(in crate::tiles) fn display(&self) -> GridDisplay {
        if self.holds_in(self.area, self.growth) {
            self.held_display
        } else {
            GridDisplay::SummaryAlone
        }
    }

    /// How far through the current transition the grid is, on the
    /// [`PROGRESS_SCALE`] scale. A settled grid is fully through.
    pub(in crate::tiles) fn progress(&self) -> u32 {
        let GridMotion::Moving(transition) = &self.motion else {
            return PROGRESS_SCALE;
        };
        match self.motion_clock {
            MotionClock::Elapsed => {
                travel_progress(transition.started.elapsed(), transition.millis)
            },
            MotionClock::HeldAtHalf => PROGRESS_SCALE / 2,
        }
    }

    /// Every piece to draw this frame, in cell order.
    pub fn placements(&self, area: Rect, growth: TileGrowth) -> Vec<TilePlacement<Id>> {
        self.drawing(area, growth)
            .pieces
            .into_iter()
            .map(|piece| piece.placement)
            .collect()
    }

    /// Advance covered grid geometry without painting its cells.
    pub fn prepare_covered(
        &mut self,
        area: Rect,
        growth: TileGrowth,
        cells: &impl draw::TileCells<Id>,
    ) {
        draw::prepare_covered_grid(self, area, growth, cells);
    }

    /// Duration of one full cell travel.
    #[must_use]
    pub const fn cell_travel_duration(&self) -> Duration {
        Duration::from_millis(self.settings.animation_millis)
    }

    /// Paint one cell over the grid using its existing travel and cell painter.
    /// The returned allocation supports continuous reversal. Repaint stops at the endpoint.
    pub fn draw_cell_view(
        &self,
        buffer: &mut Buffer,
        content: &TileContent<Id>,
        travel: CellTravel,
        cells: &impl draw::TileCells<Id>,
        mode: crate::ColorMode,
    ) -> (Rect, Repaint) {
        let progress = travel_progress(travel.elapsed, self.settings.animation_millis);
        let rect = lerp_rect(travel.from, travel.to, eased(progress)).intersection(buffer.area);
        let frame = PaneFrame::new(rect).with_focus(true);
        let identity = match content {
            TileContent::Summary => CellIdentity::Summary,
            TileContent::Group(id) => CellIdentity::Slot(Slot::Group(id.clone())),
            TileContent::Empty(index) => {
                CellIdentity::Slot(Slot::Empty(u64::try_from(*index).unwrap_or(u64::MAX)))
            },
        };
        let drawing = TileDrawing {
            pieces:       vec![TilePiece {
                identity,
                placement: TilePlacement {
                    content: content.clone(),
                    frame,
                },
                name: PieceName::Shown,
                height_snap: PieceHeightSnap::Settled,
            }],
            column_bands: Vec::new(),
        };
        let widths = vec![(content.clone(), frame.inner().width)];
        cells.prepare_frame(|| vec![(content.clone(), frame.inner())]);
        let mut demands = cells.demands(&widths);
        draw::add_readout_rows(&mut demands, &widths);
        for y in rect.top()..rect.bottom().min(buffer.area.bottom()) {
            for x in rect.left()..rect.right().min(buffer.area.right()) {
                if buffer.area.contains(Position::new(x, y)) {
                    buffer[(x, y)].reset();
                }
            }
        }
        draw::draw_placements(
            buffer,
            rect,
            &drawing,
            &TileMeasurements {
                demands: &demands,
                widths:  &widths,
            },
            TileGridContents::Shown,
            cells,
            mode,
        );
        (
            rect,
            if progress < PROGRESS_SCALE {
                Repaint::Needed
            } else {
                Repaint::NotNeeded
            },
        )
    }

    /// Summary clip rectangles painted by the most recent `draw_tile_grid` call.
    #[must_use]
    pub fn painted_summary_frames(&self) -> &[Rect] {
        match &self.last_painted {
            LastPainted::NeverPainted => &[],
            LastPainted::Painted { summary_frames, .. } => summary_frames,
        }
    }

    /// Commit the body that was painted. Status draws and polls never call this.
    pub(in crate::tiles) fn record_painted_body(
        &mut self,
        drawing: TileDrawing<Id>,
        contents: crate::TileGridContents,
        summary_frames: Vec<Rect>,
    ) {
        self.last_painted = LastPainted::Painted {
            visual: GridVisualState {
                drawing,
                contents,
                theme: crate::theme(),
                transparent: BackgroundTransparency::current(),
                tint: FocusedPaneTint::current(),
            },
            summary_frames,
        };
    }

    /// Resolve the topmost painted piece before letting input change focus.
    pub(in crate::tiles) fn cell_hit(&self, pos: Position) -> CellHit<Id> {
        let initial_drawing;
        let drawing = match &self.last_painted {
            LastPainted::Painted { visual, .. } => &visual.drawing,
            LastPainted::NeverPainted => {
                initial_drawing = self.drawing(self.area, self.growth);
                &initial_drawing
            },
        };
        let Some(piece) = drawing.pieces.iter().rev().find(|piece| {
            draw::has_visible_body_row(piece.placement.frame)
                && piece.placement.frame.clip().contains(pos)
        }) else {
            return CellHit::Missed;
        };
        let index = match &piece.identity {
            CellIdentity::Summary => TABLE_CELL,
            CellIdentity::Slot(slot) => {
                let Some(index) = cell_of(&self.slots, slot) else {
                    return CellHit::Retired;
                };
                index
            },
        };
        let content = match &piece.identity {
            CellIdentity::Summary => TileContent::Summary,
            CellIdentity::Slot(slot) => content_of(slot, index),
        };
        CellHit::Current {
            index,
            content,
            frame: piece.placement.frame,
        }
    }

    /// Visible content and title rectangles from the last painted current tiles.
    /// Borders and reserved readout rows do not belong to these rectangles.
    #[must_use]
    pub fn pointer_regions(&self) -> Vec<Rect> {
        let LastPainted::Painted { visual, .. } = &self.last_painted else {
            return Vec::new();
        };
        visual
            .drawing
            .pieces
            .iter()
            .filter(|piece| draw::has_visible_body_row(piece.placement.frame))
            .filter(|piece| match &piece.identity {
                CellIdentity::Summary => true,
                CellIdentity::Slot(slot) => cell_of(&self.slots, slot).is_some(),
            })
            .flat_map(|piece| host::painted_regions(piece.placement.frame))
            .collect()
    }

    /// Every piece and column frame to draw at one reading of the
    /// transition clock.
    pub(in crate::tiles) fn drawing(&self, area: Rect, growth: TileGrowth) -> TileDrawing<Id> {
        self.drawing_at(area, growth, self.progress())
    }

    /// The summary filling an area whose cells do not fit.
    fn summary_alone_drawing(area: Rect) -> TileDrawing<Id> {
        TileDrawing {
            pieces:       vec![TilePiece {
                identity:    CellIdentity::Summary,
                placement:   TilePlacement {
                    content: TileContent::Summary,
                    frame:   PaneFrame::new(area).with_focus(true),
                },
                name:        PieceName::Shown,
                height_snap: PieceHeightSnap::Settled,
            }],
            column_bands: Vec::new(),
        }
    }

    /// The grid's pieces after its current arrangement has settled.
    fn settled_drawing(&self, settled: &Grid) -> TileDrawing<Id> {
        let focused = self.focused_cell();
        let pieces = cells(&self.slots)
            .into_iter()
            .filter_map(|(content, index)| {
                Some(TilePiece {
                    identity:    if index == TABLE_CELL {
                        CellIdentity::Summary
                    } else {
                        CellIdentity::Slot(self.slots[index - TABLE_CELL - 1].clone())
                    },
                    placement:   TilePlacement {
                        content,
                        frame: PaneFrame::new(settled.cell(index)?)
                            .with_focus(focused == FocusLocation::Cell(index)),
                    },
                    name:        PieceName::Shown,
                    height_snap: PieceHeightSnap::Settled,
                })
            })
            .collect();
        TileDrawing {
            pieces,
            column_bands: settled.columns.clone(),
        }
    }

    /// Both sides of each change to a fixed provider's integer geometry inputs.
    #[cfg(test)]
    pub(in crate::tiles) fn distinct_motion_steps_for_test(
        &self,
        area: Rect,
        growth: TileGrowth,
    ) -> Vec<u32> {
        let destination = self.resolved_grid(area, &self.drawn_held(), growth);
        let mut previous = None;
        let mut steps = vec![0, PROGRESS_SCALE];
        for raw in 0..=PROGRESS_SCALE {
            let drawing = self.drawing_from_destination(area, raw, &HeightSnap::Off, &destination);
            let geometry = drawing
                .pieces
                .iter()
                .map(|piece| {
                    let placement = &piece.placement;
                    (
                        placement.content.clone(),
                        placement.frame.rect(),
                        placement.frame.clip(),
                        placement.frame.shift(),
                    )
                })
                .collect::<Vec<_>>();
            if previous.as_ref() != Some(&geometry) {
                steps.extend([raw.saturating_sub(1), raw]);
                previous = Some(geometry);
            }
        }
        steps.sort_unstable();
        steps.dedup();
        steps
    }

    /// Every piece and column frame at an exact point in the transition.
    pub(in crate::tiles) fn drawing_at(
        &self,
        area: Rect,
        growth: TileGrowth,
        raw: u32,
    ) -> TileDrawing<Id> {
        let height_snap = if raw >= PROGRESS_SCALE || self.motion_heights.piece_heights.is_empty() {
            HeightSnap::Off
        } else if raw == self.motion_heights.progress {
            HeightSnap::Frame {
                row_usage: &self.frame_row_usage,
                path:      PieceHeightPath::DrawSelected(&self.motion_heights.piece_heights),
            }
        } else {
            HeightSnap::Frame {
                row_usage: &self.frame_row_usage,
                path:      PieceHeightPath::AdvanceFrom(&self.motion_heights.piece_heights),
            }
        };
        self.drawing_at_with_height_snap(area, growth, raw, &height_snap)
    }

    /// Draw one exact motion point with or without its content-aware height pass.
    pub(super) fn drawing_at_with_height_snap(
        &self,
        area: Rect,
        growth: TileGrowth,
        raw: u32,
        height_snap: &HeightSnap<'_, Id>,
    ) -> TileDrawing<Id> {
        if self.held_display == GridDisplay::SummaryAlone || !self.holds_in(area, growth) {
            return Self::summary_alone_drawing(area);
        }
        let settled = self.resolved_grid(area, &self.drawn_held(), growth);
        self.drawing_from_destination(area, raw, height_snap, &settled)
    }

    /// Use the destination already resolved with this measurement pass's rows.
    fn drawing_from_destination(
        &self,
        area: Rect,
        raw: u32,
        height_snap: &HeightSnap<'_, Id>,
        destination: &Grid,
    ) -> TileDrawing<Id> {
        let GridMotion::Moving(transition) = &self.motion else {
            return self.settled_drawing(destination);
        };
        let before = transition.source.as_ref();
        if !before.is_current_endpoint(area, self.growth, &self.settings) {
            return self.settled_drawing(destination);
        }

        let progress = eased(raw);
        let column_bands = column_bands(before, destination, progress);
        let turns = Turns::of(before, destination);
        let mut pieces = Vec::new();
        // The summary keeps cell one throughout, but the grid around it
        // resizes, so it still has somewhere to travel.
        moving_cell(
            before,
            destination,
            CellTransition::PresentAtBothEnds {
                before_cell: TABLE_CELL,
                after_cell:  TABLE_CELL,
            },
            eased(turns.summary(raw)),
            CellAppearance {
                identity: CellIdentity::Summary,
                content:  TileContent::Summary,
                focused:  self.focus == Focus::Summary,
            },
            &mut pieces,
        );
        for slot in union(&transition.from, &self.slots) {
            // A slot handed over in place is not a cell arriving and a
            // cell leaving, however it looks in the arrangement: the
            // cell stands where it stood and only its contents changed.
            // Taken at face value the pair would draw the old one
            // collapsing while a new one rose out of the column floor to
            // meet it, which is motion the grid never made.
            let (old, new) = (
                cell_of(&transition.from, &slot),
                cell_of(&self.slots, &slot),
            );
            let transition = match (old, new) {
                (None, Some(index))
                    if index
                        .checked_sub(TABLE_CELL + 1)
                        .and_then(|position| transition.from.get(position))
                        .is_some_and(|held| claims(held, &slot)) =>
                {
                    CellTransition::PresentAtBothEnds {
                        before_cell: index,
                        after_cell:  index,
                    }
                },
                (Some(index), None)
                    if index
                        .checked_sub(TABLE_CELL + 1)
                        .and_then(|position| self.slots.get(position))
                        .is_some_and(|taken| {
                            claims(&slot, taken) && !transition.from.contains(taken)
                        }) =>
                {
                    continue;
                },
                (Some(before_cell), Some(after_cell)) => CellTransition::PresentAtBothEnds {
                    before_cell,
                    after_cell,
                },
                (None, Some(after_cell)) => CellTransition::Arriving { after_cell },
                (Some(before_cell), None) => CellTransition::Departing { before_cell },
                (None, None) => continue,
            };
            let content = content_of(&slot, new.or(old).unwrap_or(TABLE_CELL));
            let piece_progress = if turns.covers(before, destination, transition) {
                eased(turns.covered(raw))
            } else {
                progress
            };
            moving_cell(
                before,
                destination,
                transition,
                piece_progress,
                CellAppearance {
                    identity: CellIdentity::Slot(slot.clone()),
                    content,
                    focused: self.focus == Focus::Cell(slot),
                },
                &mut pieces,
            );
        }
        let pieces = place_band_pieces(before, destination, &column_bands, pieces, height_snap);
        TileDrawing {
            pieces,
            column_bands,
        }
    }

    /// Carry out one grid action: open or close a cell, or move the
    /// focus ring a step. `growth` decides how many cells each column
    /// holds, which decides whether a new cell still fits and where a
    /// step of the ring lands.
    pub fn apply(&mut self, action: TileAction, growth: TileGrowth) {
        self.set_layout(self.area, growth);
        match action {
            TileAction::Add => self.add(growth),
            TileAction::Remove => self.remove(),
            TileAction::FocusLeft => self.focus_step(Direction::Left, growth),
            TileAction::FocusRight => self.focus_step(Direction::Right, growth),
            TileAction::FocusUp => self.focus_step(Direction::Up, growth),
            TileAction::FocusDown => self.focus_step(Direction::Down, growth),
        }
    }
}
