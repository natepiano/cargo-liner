//! Prepare each frame only when its source inputs change.

use super::ANCESTRY_GAP_HEIGHT;
use super::AncestryRowUse;
use super::AncestrySource;
use super::Arc;
use super::ArgumentDetail;
use super::CellTableGap;
use super::CellTableLayout;
use super::Color;
use super::CommandReading;
use super::CommandView;
use super::DirectoryHeading;
use super::DirectorySeparation;
use super::FrameColumnGrids;
use super::FrameTableColumnGrid;
use super::FreeDiskSpace;
use super::GROUP_GAP_HEIGHT;
use super::GROUP_HEADER_HEIGHT;
use super::GibibytePrecision;
use super::GroupRowUse;
use super::HashMap;
use super::InvocationId;
use super::Line;
use super::PaintStage;
use super::PaintedCellContents;
use super::PathGroup;
use super::PinnedGroup;
use super::PreparedCommandCells;
use super::ProcessHierarchy;
use super::Rc;
use super::Rect;
use super::RefCell;
use super::RosterRevision;
use super::SUMMARY_DISK_LABEL;
use super::SUMMARY_DISK_SEPARATOR;
use super::Span;
use super::Style;
use super::SummaryFoot;
use super::SummaryMemoryTotal;
use super::TABLE_HEADER_HEIGHT;
use super::TableKind;
use super::TableRowEnd;
use super::TableSeparators;
use super::TableStateColumn;
use super::Theme;
use super::TileContent;
use super::TrackedRow;
use super::VisibleRoster;
use super::WholeShortCommandTable;
use super::ancestry_allocation;
use super::ancestry_levels;
use super::cleared;
use super::drawn_ancestry;
use super::gibibytes_label;
use super::group_by_path;
use super::label_color;
use super::memory_total;
use super::pane_background;
use super::process_row_lines;
use super::state_width;
use super::summary_rows;
use super::table_grid_after_ancestry;
use super::table_grid_for_whole_commands;
#[cfg(test)]
use super::tests;
use super::text_default;
use crate::census::invocation_cpu_accounting::CompileMemoryShares;
use crate::probe;
use crate::roster::Roster;

/// Retained table preparation, with its complete source identity.
#[derive(Default)]
pub(crate) struct PreparedTables {
    state:        RetainedFrame,
    cell:         CellFramePreparation,
    commands:     Rc<RefCell<PreparedCommandCells>>,
    summary_foot: SummaryFootPreparation,
}

#[cfg(test)]
mod expanded_cache_tests {
    use super::*;

    #[test]
    fn an_expanded_cell_reuses_the_grid_command_measurements()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut tables = PreparedTables::default();
        let mut roster = Roster::default();
        let view = CommandView::long();
        tables.for_cell_frame(&mut roster, &[], view, FreeDiskSpace::Unavailable);
        let CellFramePreparation::Prepared(cell) = &tables.cell else {
            return Err("the cell frame must be prepared".into());
        };
        assert!(
            Rc::ptr_eq(&tables.commands, &cell.commands),
            "grid and cell share one command cache"
        );
        Ok(())
    }
}

/// Expanded tables retain their own allocation without changing the ordinary grid.
#[derive(Default)]
enum CellFramePreparation {
    #[default]
    Unprepared,
    Prepared(Box<PreparedTables>),
}

#[derive(Default)]
enum RetainedFrame {
    #[default]
    Unprepared,
    Prepared(Box<PreparedFrame>),
}

/// Measured widths and styled rows are valid only for these exact inputs.
struct PreparedFrame {
    revision: RosterRevision,
    hidden:   Vec<String>,
    view:     CommandView,
    height:   u16,
    theme:    Arc<Theme>,
    tables:   PreparedFrameTables,
}

/// Immutable source rows and their reusable width and style preparation.
#[derive(Clone)]
pub(super) struct PreparedFrameTables {
    pub(super) roster:     Rc<VisibleRoster>,
    pub(super) grids:      Rc<FrameColumnGrids>,
    pub(super) row_uses:   Rc<RefCell<HashMap<CellMeasurementKey, PreparedCellRows>>>,
    pub(super) allocation: Rc<RefCell<FrameAllocationPreparation>>,
    pub(super) paintings:  Rc<RefCell<HashMap<CellRowIdentity, PaintedCellContents>>>,
}

impl PreparedFrameTables {
    pub(super) fn new(
        roster: VisibleRoster,
        view: &CommandView,
        height: u16,
        commands: Rc<RefCell<PreparedCommandCells>>,
    ) -> Self {
        commands.borrow_mut().retain_visible(&roster);
        let grid_timer = probe::paint_timer(PaintStage::GridPreparation);
        let grids = FrameColumnGrids::for_frame(&roster, view, height, commands);
        drop(grid_timer);
        Self {
            grids:      Rc::new(grids),
            roster:     Rc::new(roster),
            row_uses:   Rc::new(RefCell::new(HashMap::new())),
            allocation: Rc::new(RefCell::new(FrameAllocationPreparation::Unprepared)),
            paintings:  Rc::new(RefCell::new(HashMap::new())),
        }
    }
}

/// Table preparation and the independently prepared summary foot for one draw.
#[derive(Clone)]
pub(super) struct PreparedPaneFrame {
    pub(super) tables:       PreparedFrameTables,
    pub(super) summary_foot: Rc<SummaryFoot>,
}

impl PreparedPaneFrame {
    pub(super) fn new(
        tables: PreparedFrameTables,
        free_disk_space: FreeDiskSpace,
        summary_foot: &mut SummaryFootPreparation,
    ) -> Self {
        Self {
            summary_foot: summary_foot.for_frame(&tables.roster, free_disk_space),
            tables,
        }
    }
}

/// The disk text that can change the summary foot.
#[derive(Eq, PartialEq)]
pub(crate) enum SummaryDiskSpace {
    Available(String),
    Unavailable,
}

impl From<FreeDiskSpace> for SummaryDiskSpace {
    fn from(free_disk_space: FreeDiskSpace) -> Self {
        match free_disk_space {
            FreeDiskSpace::Available(bytes) => {
                Self::Available(gibibytes_label(bytes, GibibytePrecision::WholeFromTen))
            },
            FreeDiskSpace::Unavailable => Self::Unavailable,
        }
    }
}

/// Reuse the foot while its displayed values and theme stay equal.
#[derive(Default)]
pub(super) enum SummaryFootPreparation {
    #[default]
    Unprepared,
    Prepared {
        memory:          SummaryMemoryTotal,
        memory_shares:   CompileMemoryShares,
        free_disk_space: SummaryDiskSpace,
        theme:           Arc<Theme>,
        foot:            Rc<SummaryFoot>,
    },
}

impl SummaryFootPreparation {
    pub(super) fn for_frame(
        &mut self,
        roster: &VisibleRoster,
        free_disk_space: FreeDiskSpace,
    ) -> Rc<SummaryFoot> {
        let memory = memory_total(roster);
        let free_disk_space = SummaryDiskSpace::from(free_disk_space);
        let theme = tui_pane::theme();
        if let Self::Prepared {
            memory: previous_memory,
            memory_shares: previous_shares,
            free_disk_space: previous_disk,
            theme: previous_theme,
            foot,
        } = self
            && *previous_memory == memory
            && *previous_shares == roster.memory_shares()
            && *previous_disk == free_disk_space
            && Arc::ptr_eq(previous_theme, &theme)
        {
            return Rc::clone(foot);
        }
        let foot = match &free_disk_space {
            SummaryDiskSpace::Unavailable => memory.foot(),
            SummaryDiskSpace::Available(value) => {
                let prefix = memory.line();
                let mut suffix = Line::default();
                if prefix.width() > 0 {
                    suffix.spans.push(Span::raw(SUMMARY_DISK_SEPARATOR));
                }
                suffix.spans.push(Span::styled(
                    SUMMARY_DISK_LABEL,
                    Style::default().fg(label_color()),
                ));
                suffix.spans.push(Span::styled(
                    value.clone(),
                    Style::default().fg(text_default()),
                ));
                if prefix.spans.is_empty() {
                    SummaryFoot::Text(suffix)
                } else {
                    SummaryFoot::WithOptionalSuffix { prefix, suffix }
                }
            },
        };
        let foot = Rc::new(foot);
        *self = Self::Prepared {
            memory,
            memory_shares: roster.memory_shares(),
            free_disk_space,
            theme,
            foot: Rc::clone(&foot),
        };
        foot
    }
}

/// New input grants access to preparation; prepared input grants drawing only.
enum FramePreparation<'frame> {
    NeedsPreparation(FrameInputs<'frame>),
    Prepared(PreparedFrameTables),
}
struct FrameInputs<'frame> {
    roster:   &'frame mut Roster,
    revision: RosterRevision,
    hidden:   &'frame [String],
    view:     &'frame CommandView,
    height:   u16,
    theme:    Arc<Theme>,
}
impl PreparedTables {
    pub(super) fn for_cell_frame(
        &mut self,
        roster: &mut Roster,
        hidden: &[String],
        view: &CommandView,
        free_disk_space: FreeDiskSpace,
    ) -> PreparedPaneFrame {
        let mut preparation = std::mem::take(&mut self.cell);
        let mut tables = match preparation {
            CellFramePreparation::Unprepared => Box::new(Self {
                commands: Rc::clone(&self.commands),
                ..Self::default()
            }),
            CellFramePreparation::Prepared(tables) => tables,
        };
        let prepared = tables.for_table_frame(roster, hidden, view, u16::MAX);
        preparation = CellFramePreparation::Prepared(tables);
        self.cell = preparation;
        PreparedPaneFrame::new(prepared, free_disk_space, &mut self.summary_foot)
    }

    pub(super) fn for_frame(
        &mut self,
        roster: &mut Roster,
        hidden: &[String],
        view: &CommandView,
        height: u16,
        free_disk_space: FreeDiskSpace,
    ) -> PreparedPaneFrame {
        let tables = self.for_table_frame(roster, hidden, view, height);
        PreparedPaneFrame::new(tables, free_disk_space, &mut self.summary_foot)
    }

    fn for_table_frame(
        &mut self,
        roster: &mut Roster,
        hidden: &[String],
        view: &CommandView,
        height: u16,
    ) -> PreparedFrameTables {
        let revision = roster.revision();
        let theme = tui_pane::theme();
        probe::paint_work(|work| match &self.state {
            RetainedFrame::Unprepared => work.unprepared += 1,
            RetainedFrame::Prepared(inputs) => {
                work.revision += u64::from(inputs.revision != revision);
                work.hidden += u64::from(inputs.hidden != hidden);
                work.view += u64::from(inputs.view != *view);
                work.height += u64::from(inputs.height != height);
                work.theme += u64::from(!Arc::ptr_eq(&inputs.theme, &theme));
            },
        });
        let preparation = match &self.state {
            RetainedFrame::Prepared(inputs)
                if inputs.revision == revision
                    && inputs.hidden == hidden
                    && inputs.view == *view
                    && inputs.height == height
                    && Arc::ptr_eq(&inputs.theme, &theme) =>
            {
                FramePreparation::Prepared(inputs.tables.clone())
            },
            RetainedFrame::Unprepared | RetainedFrame::Prepared(_) => {
                FramePreparation::NeedsPreparation(FrameInputs {
                    roster,
                    revision,
                    hidden,
                    view,
                    height,
                    theme,
                })
            },
        };
        match preparation {
            FramePreparation::Prepared(tables) => tables,
            FramePreparation::NeedsPreparation(inputs) => self.prepare(inputs),
        }
    }
    fn prepare(&mut self, inputs: FrameInputs<'_>) -> PreparedFrameTables {
        let mut tables = PreparedFrameTables::new(
            inputs.roster.visible(inputs.hidden),
            inputs.view,
            inputs.height,
            Rc::clone(&self.commands),
        );
        if let RetainedFrame::Prepared(previous) = &self.state {
            tables.paintings = Rc::clone(&previous.tables.paintings);
            tables
                .paintings
                .borrow_mut()
                .retain(|identity, _| match identity {
                    CellRowIdentity::Command(id) => {
                        tables.roster.groups().iter().any(|group| &group.id == id)
                    },
                    CellRowIdentity::Summary | CellRowIdentity::Empty => true,
                });
        }
        self.state = RetainedFrame::Prepared(Box::new(PreparedFrame {
            revision: inputs.revision,
            hidden:   inputs.hidden.to_vec(),
            view:     inputs.view.clone(),
            height:   inputs.height,
            theme:    inputs.theme,
            tables:   tables.clone(),
        }));
        tables
    }
}

#[cfg(test)]
mod prepared_tables_tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use tui_pane::TileCells;

    use super::*;
    use crate::census::Measurement;
    use crate::constants::BYTES_PER_GIBIBYTE;
    use crate::render;
    use crate::render::TileGridContents;
    use crate::render::tests;
    use crate::render::tests::CELL_HEIGHT_PREPARATIONS;
    use crate::render::tests::CONTINUATION_PROBES;
    use crate::render::tests::TABLE_COLUMN_SOLVES;
    use crate::render::tests::TABLE_LAYOUT_PREPARATIONS;
    use crate::sccache::SccacheStats;

    #[test]
    fn disk_labels_use_whole_gibibytes_from_ten_and_tenths_below() -> Result<(), &'static str> {
        for (bytes, expected) in [
            (BYTES_PER_GIBIBYTE * 3022 / 10, "302G"),
            (BYTES_PER_GIBIBYTE * 95 / 10, "9.5G"),
            (BYTES_PER_GIBIBYTE * 4 / 10, "0.4G"),
            (BYTES_PER_GIBIBYTE * 996 / 100, "10G"),
        ] {
            let SummaryDiskSpace::Available(text) =
                SummaryDiskSpace::from(FreeDiskSpace::Available(bytes))
            else {
                return Err("available bytes must have a disk label");
            };
            assert_eq!(text, expected);
        }
        Ok(())
    }

    #[test]
    fn disk_text_changes_reuse_table_preparation_and_update_only_the_foot() {
        let mut roster = tests::memory_roster(&[(4100, Measurement::Reading(BYTES_PER_GIBIBYTE))]);
        let stats = SccacheStats::new();
        let mut preparation = PreparedTables::default();
        let mut previous: Option<PreparedPaneFrame> = None;
        for (bytes, expected) in [
            (BYTES_PER_GIBIBYTE, "mem 1.0G  disk 1.0G"),
            (BYTES_PER_GIBIBYTE + 1, "mem 1.0G  disk 1.0G"),
            (BYTES_PER_GIBIBYTE * 5 / 2, "mem 1.0G  disk 2.5G"),
        ] {
            let prepared = preparation.for_frame(
                &mut roster,
                &[],
                CommandView::long(),
                37,
                FreeDiskSpace::Available(bytes),
            );
            if let Some(before) = previous.as_ref() {
                assert!(Rc::ptr_eq(&before.tables.grids, &prepared.tables.grids));
                assert!(Rc::ptr_eq(
                    &before.tables.row_uses,
                    &prepared.tables.row_uses
                ));
                assert!(Rc::ptr_eq(
                    &before.tables.allocation,
                    &prepared.tables.allocation
                ));
                assert!(Rc::ptr_eq(
                    &before.tables.paintings,
                    &prepared.tables.paintings
                ));
                assert_eq!(
                    Rc::ptr_eq(&before.summary_foot, &prepared.summary_foot),
                    bytes == BYTES_PER_GIBIBYTE + 1
                );
            }
            let cells = render::Cells::from_prepared(prepared.clone(), &stats);
            assert_eq!(tests::summary_foot_text(cells.summary_foot()), expected);
            let (_, buffer) = tests::settled_buffer_with_cells(&cells, Rect::new(0, 0, 160, 40));
            assert!(
                tests::buffer_rows(&buffer)
                    .iter()
                    .any(|row| row.contains(expected))
            );
            previous = Some(prepared);
        }
    }

    #[test]
    fn unchanged_frames_reuse_their_cell_height_probes_and_allocation_plan() {
        let mut app = tests::argument_toggle_app();
        let Ok(mut terminal) = Terminal::new(TestBackend::new(180, 40));
        for _ in 0..4 {
            app.tiles.settle_for_test();
            let Ok(_) = terminal.draw(|frame| {
                render::draw_panes(frame, &mut app, frame.area(), TileGridContents::Shown);
            });
        }
        let before = terminal.backend().buffer().clone();
        CELL_HEIGHT_PREPARATIONS.with(|count| count.set(0));
        CONTINUATION_PROBES.with(|count| count.set(0));
        let Ok(_) = terminal.draw(|frame| {
            render::draw_panes(frame, &mut app, frame.area(), TileGridContents::Shown);
        });
        assert_eq!(&before, terminal.backend().buffer());
        CELL_HEIGHT_PREPARATIONS.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "unchanged inputs reuse cell height measurements"
            );
        });
        CONTINUATION_PROBES.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "unchanged allocations reuse continuation decisions"
            );
        });
    }

    #[test]
    fn unchanged_frames_reuse_solved_table_layouts() {
        for (width, height) in [(180, 40), (160, 80)] {
            let mut app = tests::argument_toggle_app();
            let Ok(mut terminal) = Terminal::new(TestBackend::new(width, height));
            for _ in 0..4 {
                app.tiles.settle_for_test();
                let Ok(_) = terminal.draw(|frame| {
                    render::draw_panes(frame, &mut app, frame.area(), TileGridContents::Shown);
                });
            }
            let before = terminal.backend().buffer().clone();
            TABLE_LAYOUT_PREPARATIONS.with(|count| count.set(0));
            let Ok(_) = terminal.draw(|frame| {
                render::draw_panes(frame, &mut app, frame.area(), TileGridContents::Shown);
            });
            assert_eq!(&before, terminal.backend().buffer());
            TABLE_LAYOUT_PREPARATIONS.with(|count| {
                assert_eq!(
                    count.get(),
                    0,
                    "unchanged table inputs reuse their solved columns"
                );
            });
        }
    }

    #[test]
    fn unchanged_frames_paint_prepared_columns_without_solving_again() {
        let mut app = tests::argument_toggle_app();
        let Ok(mut terminal) = Terminal::new(TestBackend::new(180, 40));
        for _ in 0..4 {
            app.tiles.settle_for_test();
            let Ok(_) = terminal.draw(|frame| {
                render::draw_panes(frame, &mut app, frame.area(), TileGridContents::Shown);
            });
        }
        let before = terminal.backend().buffer().clone();
        TABLE_COLUMN_SOLVES.with(|count| count.set(0));
        let Ok(_) = terminal.draw(|frame| {
            render::draw_panes(frame, &mut app, frame.area(), TileGridContents::Shown);
        });
        assert_eq!(&before, terminal.backend().buffer());
        TABLE_COLUMN_SOLVES.with(|count| {
            assert_eq!(
                count.get(),
                0,
                "paint uses the columns already solved for wrapping"
            );
        });
    }
}

/// A measured cell is valid only for the reservation and gap policy that produced it.
#[derive(Eq, Hash, PartialEq)]
pub(super) struct CellMeasurementKey {
    identity: CellRowIdentity,
    width:    u16,
    policy:   CellMeasurementPolicy,
}

impl CellMeasurementKey {
    pub(super) fn new(content: &TileContent, width: u16, grids: &FrameColumnGrids) -> Self {
        let grid = match content {
            TileContent::Group(_) => &grids.commands.0,
            TileContent::Summary | TileContent::Empty(_) => &grids.summary.0,
        };
        Self {
            identity: CellRowIdentity::from(content),
            width,
            policy: CellMeasurementPolicy::of(grid),
        }
    }
}

/// All mutable policy inputs to one table's width and height measurements.
#[derive(Eq, Hash, PartialEq)]
pub(super) struct CellMeasurementPolicy {
    row_end:            TableRowEnd,
    frame_row_end:      TableRowEnd,
    gap:                CellTableGap,
    whole_short_tables: Vec<WholeShortCommandTable>,
}

impl CellMeasurementPolicy {
    pub(super) fn of(grid: &FrameTableColumnGrid) -> Self {
        Self {
            row_end:            grid.row_end.get(),
            frame_row_end:      grid.frame_row_end.get(),
            gap:                grid.cell_gap.get(),
            whole_short_tables: grid.whole_short_tables.borrow().clone(),
        }
    }

    pub(super) fn apply(&self, grid: &FrameTableColumnGrid) {
        grid.row_end.set(self.row_end);
        grid.frame_row_end.set(self.frame_row_end);
        grid.cell_gap.set(self.gap);
        grid.whole_short_tables
            .borrow_mut()
            .clone_from(&self.whole_short_tables);
    }
}

/// The last allocation's decisions belong to the unchanged prepared source snapshot.
pub(super) enum FrameAllocationPreparation {
    Unprepared,
    Prepared(Box<PreparedFrameAllocation>),
}

pub(super) struct PreparedFrameAllocation {
    pub(super) allocations: Vec<(TileContent, Rect)>,
    pub(super) summary:     CellMeasurementPolicy,
    pub(super) commands:    CellMeasurementPolicy,
}

/// Stable cell identity for the frame's indexed width layouts.
#[derive(Eq, Hash, PartialEq)]
pub(super) enum CellRowIdentity {
    Summary,
    Command(InvocationId),
    Empty,
}

impl From<&TileContent> for CellRowIdentity {
    fn from(content: &TileContent) -> Self {
        match content {
            TileContent::Summary => Self::Summary,
            TileContent::Group(id) => Self::Command(id.clone()),
            TileContent::Empty(_) => Self::Empty,
        }
    }
}

/// Prepared ancestry and table measurements for one frame cell.
pub(super) enum PreparedCellRows {
    Empty,
    Summary(PreparedSummaryRows),
    Command {
        ancestry:     PreparedAncestryRows,
        tables:       AncestryTableRows,
        table_demand: usize,
        reading_rows: u16,
    },
}

/// The parent column can change when no ancestry line fits above the table.
pub(super) enum AncestryTableRows {
    StableParentColumn(PreparedTableRows),
    ParentColumnFollowsAncestry {
        above: PreparedTableRows,
        alone: PreparedTableRows,
    },
}

impl AncestryTableRows {
    fn at(&self, ancestry_rows: u16, height: u16) -> GroupRowUse {
        match self {
            Self::StableParentColumn(table) => table.at(height),
            Self::ParentColumnFollowsAncestry { above, alone } => {
                if ancestry_rows == 0 {
                    alone.at(height)
                } else {
                    above.at(height)
                }
            },
        }
    }
}

impl PreparedCellRows {
    pub(super) fn new(
        roster: &VisibleRoster,
        content: &TileContent,
        width: u16,
        grids: &FrameColumnGrids,
    ) -> Self {
        #[cfg(test)]
        tests::CELL_HEIGHT_PREPARATIONS.with(|count| count.set(count.get() + 1));
        match content {
            TileContent::Summary => {
                let rows = summary_rows(roster);
                let mut summary = PreparedSummaryRows::new(&rows, width, &grids.summary.0);
                summary.memory_rows =
                    u16::try_from(super::summary_memory_rows(roster).count()).unwrap_or(u16::MAX);
                Self::Summary(summary)
            },
            TileContent::Group(id) => {
                let Some(group) = roster.groups().iter().find(|group| &group.id == id) else {
                    return Self::Empty;
                };
                let grid = &grids.commands.0;

                let rows: Vec<_> = group.rows().collect();
                let walk = drawn_ancestry(group, &grid.view);
                let ancestry = PreparedAncestryRows::new(
                    AncestrySource::for_group(&walk, group, &grid.view),
                    width,
                );
                let pinned = PinnedGroup::Lead(&group.lead.process);
                let tables = if walk.as_slice().is_empty() {
                    AncestryTableRows::StableParentColumn(PreparedTableRows::new(
                        &rows,
                        width,
                        pinned,
                        &table_grid_after_ancestry(grid, 0),
                    ))
                } else if grid.view.hierarchy == ProcessHierarchy::Shown {
                    AncestryTableRows::ParentColumnFollowsAncestry {
                        above: PreparedTableRows::new(&rows, width, pinned, grid),
                        alone: PreparedTableRows::new(
                            &rows,
                            width,
                            pinned,
                            &table_grid_after_ancestry(grid, 0),
                        ),
                    }
                } else {
                    AncestryTableRows::StableParentColumn(PreparedTableRows::new(
                        &rows, width, pinned, grid,
                    ))
                };
                let table_demand = match &tables {
                    AncestryTableRows::StableParentColumn(table) => table.demand(),
                    AncestryTableRows::ParentColumnFollowsAncestry { above, .. } => above.demand(),
                };
                Self::Command {
                    ancestry,
                    tables,
                    table_demand,
                    reading_rows: CommandReading::for_group(group, width, grid, Color::Reset)
                        .rows(),
                }
            },
            TileContent::Empty(_) => Self::Empty,
        }
    }

    pub(super) fn demand(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Summary(table) => table.demand(),
            Self::Command {
                ancestry,
                table_demand,
                reading_rows,
                ..
            } => {
                let ancestry = if ancestry.heights.len() > 1 {
                    ancestry
                        .heights
                        .last()
                        .copied()
                        .unwrap_or_default()
                        .saturating_add(usize::from(ANCESTRY_GAP_HEIGHT))
                } else {
                    0
                };
                ancestry
                    .saturating_add(*table_demand)
                    .saturating_add(usize::from(*reading_rows))
                    .saturating_add(usize::from(GROUP_HEADER_HEIGHT))
            },
        }
    }

    pub(super) fn at(&self, height: u16) -> GroupRowUse {
        match self {
            Self::Empty => GroupRowUse {
                drawn:                  0,
                kept:                   0,
                separator_rows_yielded: 0,
            },
            Self::Summary(table) => table.at(height),
            Self::Command {
                ancestry,
                tables,
                table_demand,
                reading_rows,
            } => {
                let heading = GROUP_HEADER_HEIGHT.min(height);
                let height = height.saturating_sub(heading);
                let reading = (*reading_rows).min(height);
                let height = height.saturating_sub(reading);
                let ancestry = ancestry.at(height, *table_demand);
                let table = tables.at(ancestry.drawn, height.saturating_sub(ancestry.drawn));
                let mut rows = GroupRowUse::with_ancestry(&ancestry, table.drawn);
                rows.kept = rows
                    .kept
                    .saturating_sub(table.drawn.saturating_sub(table.kept));
                rows.separator_rows_yielded = table.separator_rows_yielded;
                rows.drawn = rows.drawn.saturating_add(heading).saturating_add(reading);
                rows.kept = rows.kept.saturating_add(heading).saturating_add(reading);
                rows
            },
        }
    }
}

/// Heights of every retained ancestry form at this width.
pub(super) struct PreparedAncestryRows {
    heights: Vec<usize>,
}

impl PreparedAncestryRows {
    fn new(source: AncestrySource<'_>, width: u16) -> Self {
        let width = cleared(Rect::new(0, 0, width, 0)).width;
        let heights = (0..=source.levels.len())
            .map(|count| {
                let mut levels = ancestry_levels(source.levels, count);
                source.mark(&mut levels);
                source.height(&levels, width)
            })
            .collect();
        Self { heights }
    }

    fn at(&self, height: u16, table: usize) -> AncestryRowUse {
        let levels = self.heights.len().saturating_sub(1);
        let allocation = ancestry_allocation(height, table);
        let gap = if levels <= 1 || self.heights[levels] <= allocation.content_rows {
            allocation.gap_rows
        } else {
            0
        };
        let budget = usize::from(height)
            .saturating_sub(table)
            .saturating_sub(usize::from(gap));
        let mut count = budget.min(levels);
        loop {
            if count <= 1 && budget >= 2 && levels > 1 {
                count = 2;
                break;
            }
            if count <= 1 || self.heights[count] <= budget {
                break;
            }
            count = count.saturating_sub(1);
        }
        if count == 0 {
            return AncestryRowUse {
                drawn:    0,
                required: 0,
            };
        }
        let drawn = u16::try_from(self.heights[count].min(budget.max(1)))
            .unwrap_or(u16::MAX)
            .saturating_add(gap);
        let required = if table > 0 && budget == 1 && gap == 0 {
            0
        } else {
            drawn
        };
        AncestryRowUse { drawn, required }
    }
}

/// Painted and retained rows at any height of one solved table width.
enum TableHeightUse {
    EmptyTable,
    NoReadableColumns,
    ProcessRows {
        groups:     Vec<DirectoryRowHeights>,
        body_rows:  u16,
        gap_rows:   u16,
        exact_rows: usize,
    },
}

/// `DirectoryRowHeights` retains heading placement and command row prefixes for height measurement.
struct DirectoryRowHeights {
    heading:   DirectoryHeading,
    rows:      Vec<u16>,
    arguments: ArgumentDetail,
    kind:      TableKind,
}

impl TableHeightUse {
    fn new(groups: &[PathGroup<'_>], layout: &CellTableLayout) -> Self {
        if groups.is_empty() {
            return Self::EmptyTable;
        }
        if layout.columns.is_empty() {
            return Self::NoReadableColumns;
        }
        #[cfg(test)]
        tests::LIMIT_ROW_USE_LAYOUTS.with(|limit| {
            if limit.get() {
                let rows: Vec<_> = groups
                    .iter()
                    .flat_map(|group| group.rows.iter().copied())
                    .collect();
                tests::record_row_use_layout(&rows, layout.table_width);
            }
        });
        let mut process_rows = 0_usize;
        let groups: Vec<DirectoryRowHeights> = groups
            .iter()
            .map(|group| {
                let mut prefix = vec![0_u16];
                let mut used = 0_u16;
                for row in &group.rows {
                    let lines = process_row_lines(row, layout);
                    process_rows = process_rows.saturating_add(usize::from(lines));
                    used = used.saturating_add(lines);
                    prefix.push(used);
                }
                DirectoryRowHeights {
                    heading:   group.heading,
                    rows:      prefix,
                    arguments: layout.view.arguments,
                    kind:      layout.kind,
                }
            })
            .collect();
        let body_rows = groups
            .iter()
            .map(|group| {
                group
                    .heading
                    .rows()
                    .saturating_add(*group.rows.last().unwrap_or(&0))
            })
            .fold(0, u16::saturating_add);
        let gap_rows = u16::try_from(groups.len().saturating_sub(1))
            .unwrap_or(u16::MAX)
            .saturating_mul(GROUP_GAP_HEIGHT);
        let exact_rows = usize::from(TABLE_HEADER_HEIGHT)
            .saturating_add(process_rows)
            .saturating_add(
                groups
                    .iter()
                    .map(|group| usize::from(group.heading.rows()))
                    .sum::<usize>(),
            )
            .saturating_add(
                groups
                    .len()
                    .saturating_sub(1)
                    .saturating_mul(usize::from(GROUP_GAP_HEIGHT)),
            );
        Self::ProcessRows {
            groups,
            body_rows,
            gap_rows,
            exact_rows,
        }
    }

    fn exact_rows(&self) -> usize {
        match self {
            Self::EmptyTable => 0,
            Self::NoReadableColumns => usize::from(TABLE_HEADER_HEIGHT),
            Self::ProcessRows { exact_rows, .. } => *exact_rows,
        }
    }

    fn at(&self, height: u16, separation: DirectorySeparation) -> GroupRowUse {
        match self {
            Self::EmptyTable => {
                let drawn = height.min(TABLE_HEADER_HEIGHT.saturating_add(GROUP_HEADER_HEIGHT));
                GroupRowUse {
                    drawn,
                    kept: drawn,
                    separator_rows_yielded: 0,
                }
            },
            Self::NoReadableColumns => {
                let drawn = height.min(TABLE_HEADER_HEIGHT);
                GroupRowUse {
                    drawn,
                    kept: drawn,
                    separator_rows_yielded: 0,
                }
            },
            Self::ProcessRows {
                groups,
                body_rows,
                gap_rows,
                ..
            } => measured_table_row_use(groups, *body_rows, *gap_rows, height, separation),
        }
    }
}

/// `measured_table_row_use` applies the same `CommandRowBudget` as `reserve_command_rows`.
fn measured_table_row_use(
    groups: &[DirectoryRowHeights],
    body_rows: u16,
    gap_rows: u16,
    height: u16,
    separation: DirectorySeparation,
) -> GroupRowUse {
    let header = height.min(TABLE_HEADER_HEIGHT);
    let available = height.saturating_sub(header);
    let total = body_rows;
    let gaps = gap_rows;
    let retained_gaps = if total.saturating_add(gaps) <= available {
        gaps
    } else {
        0
    };
    if groups.iter().all(|group| {
        group.arguments == ArgumentDetail::Default
            && group
                .rows
                .windows(2)
                .all(|pair| pair[1].saturating_sub(pair[0]) <= 1)
    }) {
        return single_line_table_use(groups, height, retained_gaps, separation);
    }
    let processes = groups
        .iter()
        .map(|group| group.rows.len().saturating_sub(1))
        .sum();
    let headings = groups
        .iter()
        .map(|group| usize::from(group.heading.rows()))
        .sum();
    let mut budget =
        CommandRowBudget::new(available.saturating_sub(retained_gaps), processes, headings);
    let mut hidden = 0;
    let mut shown = 0;
    for group in groups {
        if !budget.heading(group.heading.rows()) {
            hidden += group.rows.len().saturating_sub(1);
            continue;
        }
        for pair in group.rows.windows(2) {
            if budget.row(pair[1].saturating_sub(pair[0])) == 0 {
                hidden += 1;
            } else {
                shown += 1;
            }
        }
    }
    let drawn = if shown == 0 {
        header
    } else if hidden > 0 {
        height
    } else {
        header.saturating_add(available.saturating_sub(budget.remaining))
    };
    let lendable = if separation == DirectorySeparation::YieldEarliest {
        retained_gaps
    } else {
        0
    };
    GroupRowUse {
        drawn,
        kept: drawn
            .saturating_sub(lendable)
            .saturating_sub(if hidden > 0 { budget.remaining } else { 0 }),
        separator_rows_yielded: if separation == DirectorySeparation::YieldEarliest {
            gaps.saturating_sub(retained_gaps)
        } else {
            0
        },
    }
}

/// Summary row heights change only when a newly visible row owns a state slot.
pub(super) struct PreparedSummaryRows {
    memory_rows: u16,
    layouts:     Vec<SummaryRowsAtHeight>,
    separation:  DirectorySeparation,
}

/// The first allocation height that uses these summary command widths.
struct SummaryRowsAtHeight {
    height: u16,
    rows:   TableHeightUse,
}

impl PreparedSummaryRows {
    fn new(rows: &[&TrackedRow], width: u16, grid: &FrameTableColumnGrid) -> Self {
        let groups = group_by_path(rows, PinnedGroup::Unpinned);
        let prepare = |height| {
            let area = Rect::new(0, 0, width, height);
            let layout = CellTableLayout::for_table_slots(grid, area, Color::Reset, rows);
            SummaryRowsAtHeight {
                height,
                rows: if rows.is_empty() {
                    TableHeightUse::EmptyTable
                } else if width == 0 {
                    TableHeightUse::NoReadableColumns
                } else {
                    TableHeightUse::new(&groups, &layout)
                },
            }
        };
        let mut layouts = vec![prepare(0)];
        let mut state = TableStateColumn::Absent;
        let mut height = TABLE_HEADER_HEIGHT;
        for group in &groups {
            height = height.saturating_add(GROUP_HEADER_HEIGHT);
            for row in &group.rows {
                height = height.saturating_add(1);
                let next = if grid.row_progress.contains(&row.process.invocation_id)
                    || state == TableStateColumn::ProgressReading
                {
                    TableStateColumn::ProgressReading
                } else if state_width(&row.process.state) > 0
                    || state == TableStateColumn::FilledValue
                {
                    TableStateColumn::FilledValue
                } else {
                    TableStateColumn::Absent
                };
                if next != state {
                    layouts.push(prepare(height));
                    state = next;
                }
            }
        }
        Self {
            memory_rows: 0,
            layouts,
            separation: grid.separation,
        }
    }

    fn demand(&self) -> usize {
        self.layouts
            .last()
            .map_or(0, |layout| layout.rows.exact_rows())
            .saturating_add(usize::from(self.memory_rows))
    }

    fn at(&self, height: u16) -> GroupRowUse {
        let table_rows = self.demand().saturating_sub(usize::from(self.memory_rows));
        let table_height = u16::try_from(table_rows).unwrap_or(u16::MAX);
        let available = if self.memory_rows == 0 {
            height
        } else {
            height.min(table_height)
        };
        let mut rows = self
            .layouts
            .iter()
            .rev()
            .find(|layout| layout.height <= available)
            .map_or(
                GroupRowUse {
                    drawn:                  0,
                    kept:                   0,
                    separator_rows_yielded: 0,
                },
                |layout| layout.rows.at(available, self.separation),
            );
        let memory = height.saturating_sub(table_height).min(self.memory_rows);
        rows.drawn = rows.drawn.saturating_add(memory);
        rows.kept = rows.kept.saturating_add(memory);
        rows
    }
}

/// Width layout shared by every height considered for this frame table.
pub(super) struct PreparedTableRows {
    demand:     usize,
    rows:       TableHeightUse,
    separation: DirectorySeparation,
}

impl PreparedTableRows {
    fn new(
        rows: &[&TrackedRow],
        width: u16,
        pinned: PinnedGroup<'_>,
        grid: &FrameTableColumnGrid,
    ) -> Self {
        let lines = if rows.is_empty() {
            TableHeightUse::EmptyTable
        } else if width == 0 {
            TableHeightUse::NoReadableColumns
        } else {
            let grid = table_grid_for_whole_commands(grid, width, rows);
            let area = Rect::new(0, 0, width, u16::MAX);
            let layout = CellTableLayout::from_grid(&grid, area, pane_background(false));
            if layout.columns.is_empty() {
                TableHeightUse::NoReadableColumns
            } else {
                TableHeightUse::new(&group_by_path(rows, pinned), &layout)
            }
        };
        Self {
            demand:     lines.exact_rows(),
            rows:       lines,
            separation: grid.separation,
        }
    }

    const fn demand(&self) -> usize { self.demand }

    fn at(&self, height: u16) -> GroupRowUse { self.rows.at(height, self.separation) }
}

/// `ProcessRowVisibility` records whether the table height can contain every process.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ProcessRowVisibility {
    AllFit,
    SomeHidden,
}

/// `CommandRowBudget` shares process and argument reservations between measurement and painting.
struct CommandRowBudget {
    remaining:  u16,
    extra:      u16,
    visibility: ProcessRowVisibility,
}

impl CommandRowBudget {
    fn new(height: u16, processes: usize, headings: usize) -> Self {
        let minimum = processes.saturating_add(headings);
        let visibility = if minimum > usize::from(height) {
            ProcessRowVisibility::SomeHidden
        } else {
            ProcessRowVisibility::AllFit
        };
        Self {
            remaining: height.saturating_sub(u16::from(
                visibility == ProcessRowVisibility::SomeHidden
                    && usize::from(height) > usize::from(headings > 0).saturating_add(1),
            )),
            extra: height.saturating_sub(u16::try_from(minimum).unwrap_or(u16::MAX)),
            visibility,
        }
    }

    const fn heading(&mut self, height: u16) -> bool {
        if self.remaining <= height {
            return false;
        }
        self.remaining = self.remaining.saturating_sub(height);
        true
    }

    fn row(&mut self, wanted: u16) -> u16 {
        let continuation = self.extra.min(wanted.saturating_sub(1));
        let budget = if self.visibility == ProcessRowVisibility::SomeHidden {
            self.remaining.min(1)
        } else {
            1 + continuation
        };
        self.extra = self.extra.saturating_sub(continuation);
        self.remaining = self.remaining.saturating_sub(budget);
        budget
    }
}

/// `reserve_command_rows` applies `CommandRowBudget` before the painting path selects visible rows.
pub(super) fn reserve_command_rows(
    groups: &mut [Vec<super::LaidOutProcessRow<'_>>],
    directories: &[PathGroup<'_>],
    height: u16,
) -> usize {
    if groups
        .iter()
        .flatten()
        .all(|row| row.lines <= 1 && row.view.arguments == ArgumentDetail::Default)
    {
        return 0;
    }
    let process_count = groups.iter().map(Vec::len).sum();
    let headings = directories
        .iter()
        .map(|group| usize::from(group.heading.rows()))
        .sum();
    let mut budget = CommandRowBudget::new(height, process_count, headings);
    let mut hidden = 0;
    for (rows, directory) in groups.iter_mut().zip(directories) {
        if !budget.heading(directory.heading.rows()) {
            hidden += rows.len();
            continue;
        }
        for row in rows.iter_mut() {
            let kept = budget.row(row.lines);
            if kept == 0 {
                hidden += 1;
            }
            if kept > 0 && kept < row.lines {
                let cut = row.cut_command_continuation(usize::from(kept));
                let mut lines = row.cells[super::COMMAND_COLUMN].lines
                    [..usize::from(kept.saturating_sub(1))]
                    .to_vec();
                lines.push(cut);
                row.cells[super::COMMAND_COLUMN] = super::Text::from(lines);
            }
            row.lines = kept.max(1);
        }
    }
    hidden
}

/// `single_line_table_use` preserves the default argument view's process capacity.
fn single_line_table_use(
    groups: &[DirectoryRowHeights],
    height: u16,
    gaps: u16,
    separation: DirectorySeparation,
) -> GroupRowUse {
    let header = height.min(TABLE_HEADER_HEIGHT);
    let available = height.saturating_sub(header);
    let mut remaining = available;
    let mut visible = 0;
    for group in groups {
        let gap = if visible > 0 && gaps > 0 {
            GROUP_GAP_HEIGHT
        } else {
            0
        };
        if remaining < gap.saturating_add(group.heading.rows()).saturating_add(1) {
            break;
        }
        visible += 1;
        remaining = remaining
            .saturating_sub(gap)
            .saturating_sub(group.heading.rows())
            .saturating_sub(*group.rows.last().unwrap_or(&0));
    }
    let separators = TableSeparators {
        first:  if gaps > 0 { 1 } else { groups.len() },
        groups: groups.len(),
    }
    .for_visible_groups(visible, &mut remaining);
    let retained = separators.rows(visible);
    let count = u16::from(
        visible > 0
            && visible < groups.len()
            && remaining > 0
            && groups
                .first()
                .is_some_and(|group| group.kind == TableKind::Summary),
    );
    remaining = remaining.saturating_sub(count);
    let drawn = header.saturating_add(available.saturating_sub(remaining));
    let lendable = if separation == DirectorySeparation::YieldEarliest {
        retained
    } else {
        0
    };
    GroupRowUse {
        drawn,
        kept: drawn.saturating_sub(lendable).saturating_sub(count),
        separator_rows_yielded: if separation == DirectorySeparation::YieldEarliest {
            u16::try_from(groups.len().saturating_sub(1))
                .unwrap_or(u16::MAX)
                .saturating_mul(GROUP_GAP_HEIGHT)
                .saturating_sub(retained)
        } else {
            0
        },
    }
}
