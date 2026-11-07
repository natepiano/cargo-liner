//! Narrow grids keep whole core columns and use rows only for drawn content.

#![expect(
    clippy::expect_used,
    reason = "fixture records and painted surfaces must exist"
)]

use ratatui::layout::Position;
use tui_pane::NavAction;
use tui_pane::TerminalApp;
use tui_pane::TileView;

use super::*;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::ChildKind;
use crate::census::HierarchyEdges;
use crate::census::MachineState;
use crate::census::ShellTimerObservation;
use crate::census::Timer;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::scan::ScanStage;
use crate::census::transcript::SubagentRole;
use crate::interaction::RowIdentity;
use crate::interaction::TileSelection;

const NOW: u64 = 1_792_166_400;
const CORE_WIDTH: u16 = 28;
const ERROR: &str = "claude1: scratch reading failed";
const LABELS: [&str; 6] = [
    "claude1", "claude2", "claude3", "claude4", "codex1", "codex2",
];

fn fixture(count: u32) -> App {
    let mut app = App::new_for_test().expect("narrow app");
    app.loaded_config.config.tiles.view = TileView::Cells;
    app.loaded_config.config.machines.remote = vec!["narrow-remote".into()];
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("busy grid rows");
    let mut rows: Vec<AgentRow> = (1..=count)
        .map(|id| {
            let mut row: AgentRow = serde_json::from_value(serde_json::json!({
                "agent":"claude", "pid":1_107_200+id,
                "session_id":format!("narrow-{id}"),
                "name":format!("long-session-{id}-with-a-readable-tail"),
                "activity":"running", "started":NOW-300,
                "directory":"/fixture/long-project-directory", "branch":"feature/narrow-columns",
                "account":{"home":"~/.claude1", "label":"long-account-label"},
                "children":[
                    {"depth":0,"kind":"shell","pid":2_107_200+id,"name":"review-the-long-project-change","started":NOW-60},
                    {"depth":0,"kind":"shell","pid":3_107_200+id,"name":"sleep 120","started":NOW-60}
                ]
            })).expect("busy session");
            row.children[0].kind = ChildKind::Subagent(SubagentRole::from_record("review", ""));
            row.children[1].kind = ChildKind::Shell(ShellTimerObservation::ObservedCountdown(Timer {
                started: NOW-60, deadline: NOW+60,
            }));
            row
        })
        .collect();
    let remote = rows.split_off(rows.len() / 2);
    app.census.apply(CensusUpdate::Local {
        rows,
        hierarchy: HierarchyEdges::default().into(),
        stage: ScanStage::Completed,
    });
    app.census.apply(CensusUpdate::Remote {
        host:  "narrow-remote".into(),
        state: MachineState::Answered(remote, HierarchyEdges::default().into()),
    });
    app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(
            &serde_json::to_vec(&serde_json::json!({
                "accounts": LABELS.map(|label| serde_json::json!({
                    "label":label,"tool":if label.starts_with("codex") {"codex"} else {"claude"},
                    "roles":["workers"],"weekly_left_percent":60,"credits":13_446,
                    "use_credits":true,"resets_at":"2026-12-01T10:00:00Z","read_at":null
                })),
                "problems":[ERROR]
            }))
            .expect("account JSON"),
        )
        .expect("account reading"),
    );
    app
}

fn paint(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("narrow terminal");
    for settle in [false, true] {
        if settle {
            app.tiles.settle_for_test();
        }
        terminal
            .draw(|frame| {
                draw_panes_at(
                    frame,
                    app,
                    Rect::new(0, 0, width, height),
                    TileGridContents::Shown,
                    NOW,
                );
            })
            .expect("narrow frame");
    }
    terminal.backend().buffer().clone()
}

/// Record the geometry supplied by the real grid before each demand call.
struct ObservedCells<'a> {
    cells:       Cells<'a>,
    allocations: RefCell<Vec<(TileContent, Rect)>>,
    widths:      RefCell<Vec<Vec<(TileContent, u16)>>>,
}

impl TileCells<TileKey> for ObservedCells<'_> {
    fn summary_title(&self) -> &str { self.cells.summary_title() }

    fn prepare_moving_frame(
        &self,
        _: impl FnOnce() -> Vec<(TileContent, Rect)>,
        motion_bounds: impl FnOnce() -> Vec<(TileContent, Rect)>,
    ) {
        let allocations = motion_bounds();
        self.allocations.replace(allocations.clone());
        self.cells.prepare_frame(|| allocations);
    }

    fn demands(&self, widths: &[(TileContent, u16)]) -> TileDemands {
        assert!(
            !self.allocations.borrow().is_empty(),
            "prepare before demand"
        );
        self.widths.borrow_mut().push(widths.to_vec());
        self.cells.demands(widths)
    }

    fn rows_readout(&self, content: &TileContent, area: Rect, rows: usize) -> RowsReadout {
        self.cells.rows_readout(content, area, rows)
    }

    fn draw(&self, buffer: &mut Buffer, content: &TileContent, area: Rect, ground: Color) {
        self.cells.draw(buffer, content, area, ground);
    }

    fn group_title(&self, id: &TileKey) -> Option<Line<'static>> { self.cells.group_title(id) }
}

fn observed_cells(app: &App) -> ObservedCells<'_> {
    ObservedCells {
        cells:       Cells::new(
            app.census
                .machines(&app.loaded_config.config.machines.remote),
            NOW,
            app.footer_colors.colors(),
            &app.tree_sorts,
            app.summary_sort,
        )
        .with_expansion(&app.expansion)
        .with_accounts(&app.accounts.table)
        .with_accounts_sort(app.accounts_sort)
        .with_selections(app.selections.clone(), SelectableSurface::EmptyCell),
        allocations: RefCell::new(Vec::new()),
        widths:      RefCell::new(Vec::new()),
    }
}

/// Unchanged source data keeps the first destination on the next draw.
#[test]
fn unchanged_account_inputs_keep_the_first_selected_destination() {
    assert_account_destination(&fixture(6));
}

fn assert_account_destination(source: &App) {
    for width in [200, 80, 50, 40] {
        let mut app = fixture(6);
        let area = Rect::new(0, 0, width, 70);
        let growth = app.loaded_config.config.tiles.growth();
        app.tiles
            .set_summary_row_allocation(SummaryRowAllocation::BeforeTiles);
        app.tiles
            .set_tile_width_floor(TileWidthFloor::CompressSingleColumn);
        let mut destinations = Vec::new();
        for _ in 0..2 {
            let observed = observed_cells(source);
            app.tiles
                .set_min_tile_width(observed.cells.measure_trees(area));
            tui_pane::draw_tile_grid(
                &mut Buffer::empty(area),
                &mut app.tiles,
                area,
                growth,
                TileGridContents::Shown,
                &observed,
                ColorMode::Colored,
            );
            app.tiles.settle_for_test();
            destinations.push(app.tiles.placements(area, growth));
        }
        assert_eq!(
            destinations[1], destinations[0],
            "the next draw keeps the first destination"
        );
    }
}

#[test]
fn absent_accounts_keep_the_first_selected_destination() {
    let mut source = fixture(6);
    source.accounts.table = AccountsTable::NotConfigured;
    assert_account_destination(&source);
}

#[test]
fn failed_accounts_keep_the_first_selected_destination() {
    let mut source = fixture(6);
    source.accounts.table = AccountsTable::Failed(AccountsFailure::InvalidReading);
    assert_account_destination(&source);
}

#[test]
fn account_refresh_keeps_the_destination_through_animation_frames() {
    for (width, search_calls, pending_motion) in
        [(200, 7, true), (80, 4, true), (50, 2, true), (40, 2, true)]
    {
        let mut app = fixture(6);
        app.accounts.table = AccountsTable::Ready(
        AccountsReading::parse(br#"{"accounts":[{"label":"claude1","tool":"claude","roles":["workers"],"weekly_left_percent":60,"use_credits":false}],"problems":[]}"#)
            .expect("the initial account reading"),
    );
        paint(&mut app, width, 70);
        app.tiles.settle_for_test();
        app.tiles.set_instant_motion_for_test();
        let source = fixture(6);
        let area = Rect::new(0, 0, width, 70);
        let growth = app.loaded_config.config.tiles.growth();
        for frame in 0..32 {
            let observed = observed_cells(&source);
            app.tiles
                .set_min_tile_width(observed.cells.measure_trees(area));
            let mut buffer = Buffer::empty(area);
            tui_pane::draw_tile_grid(
                &mut buffer,
                &mut app.tiles,
                area,
                growth,
                TileGridContents::Shown,
                &observed,
                ColorMode::Colored,
            );
            let text = lines(&buffer, area).join("\n");
            assert!(
                text.contains("accounts"),
                "accounts during frame {frame}: {text}"
            );
            assert!(
                text.contains("role"),
                "tree headers during frame {frame}: {text}"
            );
            assert_eq!(
                observed.widths.borrow().len(),
                if frame == 0 { search_calls } else { 2 },
                "demand calls at width {width} on frame {frame}",
            );
            if frame == 0 {
                assert_eq!(
                    app.tiles.has_pending_motion_for_test(),
                    pending_motion,
                    "the refresh has pending motion at width {width}",
                );
            }
            app.tiles.tick();
            if frame >= 3 && !app.tiles.has_pending_motion_for_test() {
                break;
            }
        }
        assert!(
            !app.tiles.has_pending_motion_for_test(),
            "tick completes the refresh motion at width {width}"
        );
        assert_account_arrival(&mut app, &source, area);
    }
}

fn assert_account_arrival(app: &mut App, source: &App, area: Rect) {
    let width = area.width;
    let growth = app.loaded_config.config.tiles.growth();
    let selected = app.tiles.placements(area, growth);
    let observed = observed_cells(source);
    let mut buffer = Buffer::empty(area);
    tui_pane::draw_tile_grid(
        &mut buffer,
        &mut app.tiles,
        area,
        growth,
        TileGridContents::Shown,
        &observed,
        ColorMode::Colored,
    );
    assert!(
        !app.tiles.has_pending_motion_for_test(),
        "arrival starts no second motion"
    );
    assert_eq!(
        observed.widths.borrow().len(),
        2,
        "an arrival draw measures only the two displays at width {width}"
    );
    assert_eq!(
        app.tiles.placements(area, growth),
        selected,
        "arrival starts no second motion"
    );
    let text = lines(&buffer, area).join("\n");
    for label in LABELS {
        assert!(
            text.contains(label),
            "account after arrival: {label}: {text}"
        );
    }
    assert!(
        text.contains(ERROR),
        "the account error remains visible: {text}"
    );
}

fn lines(buffer: &Buffer, area: Rect) -> Vec<String> {
    let area = area.intersection(buffer.area);
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

fn assert_core_columns(text: &[String], width: u16) {
    let header = text
        .iter()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("whole tree headers");
    assert!(
        header.contains("role"),
        "core role header at {width}: {text:?}"
    );
    for word in header
        .split_whitespace()
        .filter(|word| !matches!(*word, "▲" | "▼"))
    {
        assert!(
            [
                "session",
                "type",
                "role",
                "status",
                "account",
                "branch",
                "directory",
                "age"
            ]
            .contains(&word),
            "cut or joined header {word:?} at {width}: {header}"
        );
    }
    let role = header.find("role").expect("role column");
    let status = header.find("status").expect("status column");
    assert!(
        status > role + "role".len(),
        "core columns need a gap: {header}"
    );
    assert!(
        text.iter().any(|line| line.contains("review")),
        "whole role at {width}: {text:?}"
    );
    if width == 200 {
        assert!(
            header.contains("type"),
            "type must not drop in a wide tile: {text:?}"
        );
    }
}

fn assert_width(width: u16) {
    let mut app = fixture(6);
    let buffer = paint(&mut app, width, 70);
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let groups: Vec<_> = placements
        .iter()
        .filter(|tile| matches!(tile.content, TileContent::Group(_)))
        .collect();
    assert!(
        !groups.is_empty(),
        "cells view must stay active at {width}; {}",
        lines(&buffer, buffer.area).join("\n")
    );
    for tile in &groups {
        let area = tile.frame.inner().intersection(tile.frame.clip());
        let text = lines(&buffer, area);
        assert!(
            tile.frame.clip().width >= CORE_WIDTH,
            "width={width}, tile={:?}, core={CORE_WIDTH}, drawn={}/{}, {text:?}",
            tile.frame.clip(),
            groups.len(),
            6
        );
        assert_core_columns(&text, width);
    }
    assert_eq!(
        groups.len(),
        6,
        "all six fixture tiles stay visible at {width}; summary priority must not hide a tile"
    );
    let summary = placements
        .iter()
        .find(|tile| matches!(tile.content, TileContent::Summary))
        .expect("summary");
    let text = lines(&buffer, summary.frame.inner()).join("\n");
    for number in text
        .split_whitespace()
        .filter(|word| word.chars().all(|ch| ch.is_ascii_digit()))
    {
        if number.len() >= 5 {
            assert!(
                (1_107_201..=1_107_206).any(|pid| number == pid.to_string()),
                "a pid must be whole at {width}: {number:?}: {text}"
            );
        }
    }
    for label in LABELS {
        assert!(
            text.contains(label),
            "lost {label} at {width}, drawn={}/6: {text}",
            groups.len()
        );
    }
    assert!(text.contains(ERROR), "lost error at {width}: {text}");
}

/// The viewport fits the accounts while narrow tiles still overflow their work rows.
fn assert_constrained_width(width: u16) {
    let mut app = fixture(6);
    let height = if width <= 50 { 59 } else { 51 };
    let buffer = paint(&mut app, width, height);
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    let groups: Vec<_> = placements
        .iter()
        .filter(|tile| matches!(tile.content, TileContent::Group(_)))
        .collect();
    assert_eq!(groups.len(), 6, "all six tiles at {width}x50");
    for tile in &groups {
        let area = tile.frame.inner().intersection(tile.frame.clip());
        let text = lines(&buffer, area);
        assert!(
            tile.frame.clip().width >= CORE_WIDTH,
            "core width: {text:?}"
        );
        let header = text
            .iter()
            .find(|line| line.contains("session") && line.contains("status"))
            .expect("whole core headers at height 50");
        assert!(header.contains("role"), "core role header: {text:?}");
        if width <= 50 {
            assert!(
                text.iter().any(|line| line.contains("▼ long-ses…")),
                "the truncated root remains visible: {text:?}"
            );
            assert!(
                (3..=4).contains(&area.height),
                "the constrained tile has three or four rows: {text:?}"
            );
            assert_eq!(
                text.iter().any(|line| line.contains("review")),
                area.height == 4,
                "four rows show the review; three rows show only the root: {text:?}"
            );
            assert!(
                !text.iter().any(|line| line.contains("1:00")),
                "the timer also overflows: {text:?}"
            );
        } else {
            assert_core_columns(&text, width);
        }
    }
    let summary = placements
        .iter()
        .find(|tile| matches!(tile.content, TileContent::Summary))
        .expect("summary at height 50");
    let text = lines(&buffer, summary.frame.inner()).join("\n");
    for number in text
        .split_whitespace()
        .filter(|word| word.chars().all(|ch| ch.is_ascii_digit()))
    {
        if number.len() >= 5 {
            assert!(
                (1_107_201..=1_107_206).any(|pid| number == pid.to_string()),
                "a pid must be whole at {width}: {number:?}: {text}"
            );
        }
    }
    for label in LABELS {
        assert!(
            text.contains(label),
            "account {label} remains visible at {width}x50: {text}"
        );
    }
    assert!(
        text.contains(ERROR),
        "the error remains visible at {width}x50: {text}"
    );
}

#[test]
fn narrow_grid_at_200_keeps_core_headers_and_optional_type() { assert_constrained_width(200); }
#[test]
fn narrow_grid_at_80_keeps_core_headers_and_accounts() { assert_constrained_width(80); }
#[test]
fn narrow_grid_at_50_keeps_cells_and_core_headers() { assert_constrained_width(50); }
#[test]
fn narrow_grid_at_40_keeps_cells_and_core_headers() { assert_constrained_width(40); }

#[test]
fn grid_at_height_70_keeps_all_work_rows_and_accounts() {
    for width in [200, 80, 50, 40] {
        assert_width(width);
    }
}

#[test]
fn narrow_summary_34_by_50_omits_undrawable_sessions_and_keeps_six_accounts() {
    let mut app = fixture(20);
    let buffer = paint(&mut app, 34, 50);
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    assert!(
        placements
            .iter()
            .any(|tile| matches!(tile.content, TileContent::Group(_))),
        "twenty-tile fixture stays in cells view"
    );
    let area = app
        .selections
        .surface(&SelectionSurface::Summary)
        .expect("summary selection")
        .viewport()
        .content_area();
    let text = lines(&buffer, area);
    for label in LABELS {
        assert!(
            text.iter().any(|line| line.contains(label)),
            "missing {label} in 34x50: {text:?}"
        );
    }
    assert!(text.join("\n").contains(ERROR), "summary error: {text:?}");
    let heading = text
        .iter()
        .position(|line| line.trim() == "accounts")
        .expect("accounts heading");
    assert!(
        text[..heading]
            .iter()
            .filter(|line| line.trim().is_empty())
            .count()
            <= 3,
        "omitted sessions must not consume blank rows: {text:?}"
    );
}

#[test]
fn narrow_summary_resize_keeps_account_clicks_and_end_selection_on_painted_rows() {
    let mut app = fixture(2);
    paint(&mut app, 200, 50);
    let buffer = paint(&mut app, 40, 50);
    let area = app
        .selections
        .surface(&SelectionSurface::Summary)
        .expect("summary")
        .viewport()
        .content_area();
    let row = (area.y..area.bottom())
        .find(|&y| lines(&buffer, Rect::new(area.x, y, area.width, 1))[0].contains("codex2"))
        .expect("drawn last account");
    app.click(Position::new(area.x + 1, row));
    assert_eq!(
        app.selections
            .surface(&SelectionSurface::Summary)
            .expect("clicked summary")
            .selection(),
        TileSelection::Row(RowIdentity::SummaryAccount {
            account: "codex2".into(),
        })
    );
    app.loaded_config.config.tiles.view = TileView::Summary;
    paint(&mut app, 40, 10);
    app.selections
        .surface_mut(&SelectionSurface::Summary)
        .expect("summary navigation")
        .navigate(NavAction::End);
    let buffer = paint(&mut app, 34, 10);
    let area = app
        .selections
        .surface(&SelectionSurface::Summary)
        .expect("resized summary")
        .viewport()
        .content_area();
    let text = lines(&buffer, area);
    assert!(
        text.join("\n").contains("codex2"),
        "End account remains drawn after resize: {text:?}"
    );
    assert_eq!(
        text[0].trim(),
        "accounts",
        "resized accounts header stays pinned: {text:?}"
    );
    assert_eq!(
        text.iter().filter(|line| line.trim() == "accounts").count(),
        1
    );
    for x in area.x..area.right() {
        assert_ne!(
            buffer[(x, area.y)].bg,
            tui_pane::active_focus_color(),
            "pinned heading never takes the selection band"
        );
    }
}

#[test]
fn narrow_review_below_core_terminal_keeps_agent_tiles() {
    let mut app = fixture(2);
    let buffer = paint(&mut app, 22, 50);
    let placements = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth());
    assert!(
        placements
            .iter()
            .any(|tile| matches!(tile.content, TileContent::Group(_))),
        "below-core terminals must retain agent tiles: {:?}",
        lines(&buffer, buffer.area)
    );
}

#[test]
fn narrow_review_end_measures_the_restored_viewport() {
    let mut app = fixture(6);
    app.loaded_config.config.tiles.view = TileView::Cells;
    let mut owner: AgentRow = serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":1_107_206, "session_id":"scroll-owner",
        "name":"owner", "activity":"running", "started":NOW-300,
        "directory":"/fixture", "children":[]
    }))
    .expect("owner");
    let child = serde_json::from_value(serde_json::json!({
        "depth":0,"kind":"shell","pid":2_107_206,"name":"work","started":NOW-60
    }))
    .expect("child");
    owner.children = vec![child; 70];
    for (index, child) in owner.children.iter_mut().enumerate() {
        child.name = format!("work-{index}");
        child.kind = ChildKind::Subagent(SubagentRole::from_record(
            if index == 69 {
                "rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr"
            } else {
                "review"
            },
            "",
        ));
        child.activity = AgentActivity::Active(Activity::Waiting);
    }
    let neighbors = [1_107_207, 1_107_208].map(|pid| AgentRow {
        pid,
        session_id: format!("neighbor-{pid}").into(),
        children: owner.children[..1].to_vec(),
        ..owner.clone()
    });
    app.census.apply(CensusUpdate::Local {
        rows:      std::iter::once(owner).chain(neighbors).collect(),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    paint(&mut app, 80, 50);
    let tile = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find_map(|tile| match tile.content {
            TileContent::Group(key) => Some(key),
            _ => None,
        })
        .expect("agent tile");
    app.selections
        .surface_mut(&SelectionSurface::Tile(tile.clone()))
        .expect("tile state")
        .navigate(NavAction::End);
    let buffer = paint(&mut app, 80, 50);
    let placement = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|placement| placement.content == TileContent::Group(tile.clone()))
        .expect("scrolled tile");
    let text = lines(&buffer, placement.frame.inner()).join("\n");
    assert!(
        text.contains("rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr"),
        "visible role must set the core width: {text}"
    );
    assert!(
        text.lines()
            .any(|line| line.contains("rrrrr") && line.contains("waiting")),
        "status must fit beside the revealed role: {text}"
    );
}

fn growth_app() -> (App, crate::interaction::SelectionSurface) {
    let mut app = fixture(6);
    let mut owner: AgentRow = serde_json::from_value(serde_json::json!({
        "agent":"claude", "pid":1_107_206, "session_id":"growth-owner",
        "name":"owner", "activity":"running", "started":NOW-300,
        "directory":"/fixture", "children":[]
    }))
    .expect("growth owner");
    let child = serde_json::from_value(serde_json::json!({
        "depth":0,"kind":"shell","pid":2_107_206,"name":"work","started":NOW-60
    }))
    .expect("growth child");
    owner.children = vec![child; 71];
    for (index, child) in owner.children.iter_mut().enumerate() {
        child.name = format!("work-{index}");
        child.kind = ChildKind::Subagent(SubagentRole::from_record(
            if index == 60 {
                "rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr"
            } else {
                "review"
            },
            "",
        ));
        child.activity = AgentActivity::Active(Activity::Waiting);
    }
    let neighbors = [1_107_207, 1_107_208].map(|pid| AgentRow {
        pid,
        session_id: format!("growth-neighbor-{pid}").into(),
        children: owner.children[..1].to_vec(),
        ..owner.clone()
    });
    app.census.apply(CensusUpdate::Local {
        rows:      std::iter::once(owner).chain(neighbors).collect(),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    paint(&mut app, 80, 50);
    let key = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find_map(|tile| match tile.content {
            TileContent::Group(key) => Some(key),
            _ => None,
        })
        .expect("growth tile");
    let surface = SelectionSurface::Tile(key);
    let state = app
        .selections
        .surface_mut(&surface)
        .expect("growth selection");
    state.set_area(Rect::new(0, 0, 80, 5));
    state.navigate(NavAction::End);
    assert_eq!(state.viewport().scroll_offset(), 67);
    (app, surface)
}

#[test]
fn narrow_growth_at_end_includes_a_wide_role_above_the_saved_window() {
    let (mut app, surface) = growth_app();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 80))
        .expect("growth terminal");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| {
            draw_panes_at(
                frame,
                &mut app,
                Rect::new(0, 0, 80, 80),
                TileGridContents::Shown,
                NOW,
            );
        })
        .expect("first enlarged frame");
    let buffer = terminal.backend().buffer();
    let state = app.selections.surface(&surface).expect("grown selection");
    assert!(
        state.viewport().visible_rows() >= 15,
        "allocation must grow"
    );
    let placement = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .find(|tile| {
            selection_surface(&tile.content) == SelectableSurface::Surface(surface.clone())
        })
        .expect("grown tile");
    let text = lines(buffer, placement.frame.inner()).join("\n");
    assert!(
        text.lines().any(|line| line
            .contains("rrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrrr")
            && line.contains("waiting")),
        "growth must show the whole role and status on one line: {text}"
    );
}

#[test]
fn narrow_growth_envelope_contains_end_top_and_cursor_windows() {
    let (app, surface) = growth_app();
    let colors = FooterColors::default();
    let sorts = HashMap::new();
    let cells = Cells::new(
        app.census
            .machines(&app.loaded_config.config.machines.remote),
        NOW,
        &colors,
        &sorts,
        app.summary_sort,
    )
    .with_selections(app.selections.clone(), SelectableSurface::EmptyCell);
    let area = Rect::new(0, 0, 80, 50);
    let tile = cells
        .tiles
        .iter()
        .find(|tile| SelectionSurface::Tile(tile.key()) == surface)
        .expect("source tile");
    let saved = app.selections.surface(&surface).expect("saved selection");
    for anchor in [NavAction::End, NavAction::Home, NavAction::Up] {
        let mut state = saved.clone();
        state.navigate(anchor);
        tile.set_viewport(state.viewport());
        let source = tile.measure(area, NOW, cells.account_column);
        let tallest_area = tui_pane::frame_inner(area);
        let tallest = source.allocated(tallest_area, &state);
        cells.measurements.borrow_mut().clear();
        let minimum = {
            let mut selections = cells.selections.borrow_mut();
            *selections.surface_mut(&surface).expect("source selection") = state.clone();
            drop(selections);
            cells.measure_trees(area)
        };
        for height in 1..=tallest_area.height {
            let mut shorter = state.clone();
            shorter.set_area(Rect::new(0, 0, 80, height));
            let mut taller = state.clone();
            taller.set_area(Rect::new(0, 0, 80, 50));
            let short_start = shorter.viewport().scroll_offset();
            let short_end = (short_start + usize::from(height)).min(71);
            let tall_start = taller.viewport().scroll_offset();
            let tall_end = (tall_start + 50).min(71);
            assert!(
                tall_start <= short_start && short_end <= tall_end,
                "{anchor:?}, height {height}: {short_start}..{short_end} outside {tall_start}..{tall_end}"
            );
            let allocated = source.allocated(Rect::new(0, 0, 80, height), &state);
            assert!(tallest.minimum_tile_width() >= allocated.minimum_tile_width());
            assert!(
                minimum >= allocated.minimum_tile_width(),
                "pre-layout width {minimum} misses {anchor:?} allocation at height {height}: {}",
                allocated.minimum_tile_width()
            );
        }
    }
}

#[test]
fn narrow_growth_reports_fixture_counts() {
    for (count, width, expected_groups, expected_summary_rows) in [
        (6, 200, 6, 28),
        (6, 80, 6, 28),
        (6, 50, 6, 29),
        (6, 40, 6, 29),
        (20, 34, 2, 43),
    ] {
        let mut app = fixture(count);
        paint(&mut app, width, 50);
        let placements = app
            .tiles
            .placements(app.selection_area, app.loaded_config.config.tiles.growth());
        let groups = placements
            .iter()
            .filter(|tile| matches!(tile.content, TileContent::Group(_)))
            .count();
        let summary = placements
            .iter()
            .find(|tile| matches!(tile.content, TileContent::Summary))
            .expect("summary");
        assert_eq!(
            (groups, summary.frame.inner().height),
            (expected_groups, expected_summary_rows),
            "fixture count={count} width={width}: tile count and summary allocation stay unchanged"
        );
    }
}
