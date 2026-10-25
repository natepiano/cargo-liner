//! Role widths cover the rows painted while a grid allocation shrinks.

#![expect(clippy::expect_used, reason = "rendered fixture rows must exist")]

use tui_pane::TileGrid;
use tui_pane::TileGrowth;

use super::*;
use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::HierarchyEdges;
use crate::census::MachineState;
use crate::census::transcript::SubagentRole;

struct ShrinkingCells<'a> {
    cells:        Cells<'a>,
    summary_rows: usize,
}

impl TileCells<TileKey> for ShrinkingCells<'_> {
    fn admission(&self, id: &TileKey) -> Result<AdmissibleCell<TileKey>, NotAdmissible> {
        self.cells.admission(id)
    }

    fn summary_title(&self) -> &'static str { "summary" }

    fn prepare_frame(&self, allocations: impl FnOnce() -> Vec<(TileContent, Rect)>) {
        self.cells.prepare_frame(allocations);
    }

    fn prepare_moving_frame(
        &self,
        destination: impl FnOnce() -> Vec<(TileContent, Rect)>,
        motion_bounds: impl FnOnce() -> Vec<(TileContent, Rect)>,
    ) {
        self.cells.prepare_moving_frame(destination, motion_bounds);
    }

    fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands {
        let mut demand = self.cells.demands(widths);
        demand.summary = SummaryDemand::new(
            NonZeroU16::new(u16::try_from(self.summary_rows).expect("small summary"))
                .expect("nonempty summary"),
            0,
        );
        demand
    }

    fn rows_drawn(&self, content: &TileContent, area: Rect) -> u16 {
        self.cells.rows_drawn(content, area)
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, area: Rect, color: Color) {
        self.cells.draw(buffer, content, area, color);
    }
}

#[test]
fn shrinking_tiles_keep_the_entire_role_and_its_width_during_motion() {
    let mut owner: AgentRow = serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":10, "name":"owner", "started":1,
        "session_id":"owner", "directory":"", "children":[]
    }))
    .expect("owner row");
    owner.children = ["impl", "review", "review", "review", "review", "showrunner"]
        .into_iter()
        .enumerate()
        .map(|(index, role)| ChildRow {
            pid: None,
            kind: ChildKind::Subagent(SubagentRole::from_record(role, "")),
            name: format!("work-{index}"),
            started: 10 + u64::try_from(index).expect("small index"),
            ..serde_json::from_value(serde_json::json!({
                "kind":"shell", "depth":0, "name":"", "started":1
            }))
            .expect("child defaults")
        })
        .collect();
    let state = MachineState::Answered(vec![owner], HierarchyEdges::default().into());
    let colors = FooterColors::default();
    let mut cells = ShrinkingCells {
        cells:        Cells::new(
            vec![Machine {
                name:  "fixture",
                state: &state,
            }],
            100,
            &colors,
            &HashMap::new(),
            SummarySort::default(),
        ),
        summary_rows: 1,
    };
    let growth = TileGrowth {
        initial_rows: 8,
        widen_summary: false,
        ..tui_pane::TileGrowth::default()
    };
    let area = Rect::new(0, 0, 180, 22);
    let mut grid = tui_pane::TileGrid::new();
    let paint = |grid: &mut TileGrid<TileKey>, cells: &ShrinkingCells<'_>| {
        let mut buffer = Buffer::empty(area);
        tui_pane::draw_tile_grid(
            &mut buffer,
            grid,
            area,
            growth,
            TileGridContents::Shown,
            cells,
        );
        buffer
    };
    paint(&mut grid, &cells);
    grid.settle_for_test();
    let before = paint(&mut grid, &cells);
    cells.summary_rows = 12;
    paint(&mut grid, &cells); // Establish the new destination before preparing the next frame.
    let moving = paint(&mut grid, &cells);
    let later = paint(&mut grid, &cells);
    let text = |buffer: &Buffer| -> Vec<String> {
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    };
    let before = text(&before);
    let moving = text(&moving);
    assert!(
        before.iter().any(|line| line.contains("showrunner")),
        "{before:#?}"
    );
    let manager = moving
        .iter()
        .find(|line| line.contains("work-5"))
        .expect("source-height intermediate frame still draws the seventh row");
    assert!(manager.contains("showrunner"), "{moving:#?}");
    let columns = |rows: &[String]| {
        let header = rows
            .iter()
            .find(|line| line.contains("session") && line.contains("role"))
            .expect("tree header");
        (header.find("role"), header.find("status"))
    };
    assert_eq!(columns(&before), columns(&moving));
    let later = text(&later);
    assert!(
        later.iter().any(|line| line.contains("showrunner")),
        "{later:#?}"
    );
    assert_eq!(columns(&moving), columns(&later));
    grid.settle_for_test();
    let settled = text(&paint(&mut grid, &cells));
    assert!(
        !settled.iter().any(|line| line.contains("work-5")),
        "{settled:#?}"
    );
}
