//! Text width changes preserve the grid until its rows, tiles, or terminal size change.

use std::collections::BTreeSet;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::json;
use tui_pane::TerminalApp;
use tui_pane::TileCells;
use tui_pane::TileView;

use super::Cells;
use super::draw_at_for_test;
use crate::app::App;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::AgentRow;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::activity::RunLocation;
use crate::census::activity::VerifyStep;
use crate::census::scan::ScanStage;
use crate::tiles::TileContent;
use crate::tiles::TileKey;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const NOW: u64 = 1_794_096_000;
const LINER_PID: u32 = 346_919;

fn source_accounts() -> TestResult<AccountsTable> {
    let mut accounts = [
        json!({"label":"claude1","tool":"claude","weekly_left_percent":0,"roles":[]}),
        json!({"label":"claude2","tool":"claude","weekly_left_percent":1,"roles":[]}),
        json!({"label":"claude3","tool":"claude","weekly_left_percent":89,"roles":["units"],
            "role_counts":[{"busy":3,"idle":41,"paused":0,"exited":0,"total":44}]}),
        json!({"label":"claude4","tool":"claude","weekly_left_percent":1,"roles":[]}),
        json!({"label":"claude5","tool":"claude","weekly_left_percent":63,"roles":["showrunners","units"],
            "role_counts":[{"busy":1,"idle":6,"paused":0,"exited":0,"total":7},{"busy":0,"idle":0,"paused":4,"exited":0,"total":4}]}),
        json!({"label":"codex1","tool":"codex","weekly_left_percent":90,"roles":["workers"],
            "role_counts":[{"busy":0,"idle":0,"paused":0,"exited":0,"total":0}],"credits":0}),
        json!({"label":"codex2","tool":"codex","weekly_left_percent":0,"roles":["spare"],"credits":781}),
    ];
    for account in &mut accounts {
        account["use_credits"] = json!(true);
        account["read_ok"] = json!(true);
        account["resets_at"] = json!("2026-11-14T06:00:00Z");
    }
    Ok(AccountsTable::Ready(
        AccountsReading::parse(&serde_json::to_vec(
            &json!({"accounts":accounts,"problems":[]}),
        )?)
        .map_err(|error| error.to_string())?,
    ))
}

struct SourceFrameState {
    app:   App,
    rows:  Vec<AgentRow>,
    edges: HierarchyEdges,
}

impl SourceFrameState {
    fn new() -> TestResult<Self> {
        let mut app = App::new_for_test()?;
        app.loaded_config.config.tiles.view = TileView::Cells;
        let roots = [
            (346_277, "natedev", "/etc/nixos", 183_629),
            (346_735, "hana", "~/rust/hana_catalyst", 169_380),
            (LINER_PID, "cargo-liner", "~/rust/cargo-liner", 111_738),
        ];
        let mut rows: Vec<AgentRow> = Vec::new();
        for (pid, name, directory, age) in roots {
            rows.push(serde_json::from_value(json!({
                "agent":"claude", "pid":pid, "session_id":name, "name":name,
                "started":NOW-age, "directory":directory, "activity":"idle",
                "account":{"home":"~/.claude5", "label":"claude5"}, "children":[]
            }))?);
        }
        let units: [(&str, &[&str]); 3] = [
            (
                "natedev",
                &[
                    "accounts",
                    "builds",
                    "efficiency",
                    "messaging",
                    "postmaster",
                    "sdlc",
                    "security",
                    "spacetime",
                    "watcher",
                ],
            ),
            (
                "hana",
                &[
                    "assembly",
                    "cables",
                    "demo",
                    "fastest",
                    "fps",
                    "hana-migration",
                    "inspectors",
                    "lfo",
                    "materials",
                    "rendering",
                    "scenes",
                    "textures",
                    "windows",
                    "world",
                ],
            ),
            (
                "cargo-liner",
                &["cargo-liner-style", "handler", "port", "tile"],
            ),
        ];
        let mut edges = Vec::new();
        for (manager, members) in units {
            for name in members {
                let pid = 400_000 + u32::try_from(rows.len())?;
                rows.push(serde_json::from_value(json!({
                    "agent":"claude", "pid":pid, "session_id":name, "name":name,
                    "started":NOW-1000, "directory":"~/rust/cargo-liner", "activity":"idle",
                    "account":{"home":"~/.claude3", "label":"claude3"}, "children":[]
                }))?);
                edges.push(json!({"manager":manager, "member":name, "role":"unit"}));
            }
        }
        let liner = rows
            .iter_mut()
            .find(|row| row.pid == LINER_PID)
            .ok_or("source cargo-liner row")?;
        liner.children = serde_json::from_value(json!([
            {"depth":0,"kind":"shell","pid":500_001,"name":"Capture cargo-handler at 80x45","started":NOW-10},
            {"depth":0,"kind":"shell","pid":500_002,"name":"Write and review changes","started":NOW-10}
        ]))?;
        for index in 0..45 {
            let pid = 1_075_065 + index;
            rows.push(serde_json::from_value(json!({
                "agent":"codex", "pid":pid, "name":format!("pid {pid}"),
                "started":NOW-200, "directory":"~/rust/cargo-liner", "activity":"idle",
                "account":{"home":"~/.codex1", "label":"codex1"},
                "branch":"remove-running-section-handler", "children":[]
            }))?);
        }
        app.accounts.table = source_accounts()?;
        let mut source = Self {
            app,
            rows,
            edges: serde_json::from_value(json!(edges))?,
        };
        source.set_status(AgentActivity::Active(Activity::Waiting))?;
        Ok(source)
    }

    fn publish(&mut self) {
        self.app.census.apply(CensusUpdate::Local {
            rows:      self.rows.clone(),
            hierarchy: self.edges.clone().into(),
            stage:     ScanStage::Completed,
        });
    }

    fn set_status(&mut self, status: AgentActivity) -> TestResult {
        self.rows
            .iter_mut()
            .find(|row| row.pid == LINER_PID)
            .ok_or("live cargo-liner row")?
            .activity = status;
        self.publish();
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RequestedRows {
    summary: usize,
    groups:  Vec<(TileKey, usize)>,
}

struct DrawnLayout {
    groups:       Vec<(TileContent, Rect)>,
    summary_rows: usize,
    width_floor:  u16,
    requested:    Vec<RequestedRows>,
    text:         String,
}

impl DrawnLayout {
    fn columns(&self) -> usize {
        self.groups
            .iter()
            .map(|(_, area)| area.x)
            .collect::<BTreeSet<_>>()
            .len()
    }

    fn agent_area(&self, pid: u32) -> TestResult<Rect> {
        self.groups
            .iter()
            .find_map(|(content, area)| match content {
                TileContent::Group(TileKey::Agent(agent)) if agent.pid == pid => Some(*area),
                TileContent::Summary | TileContent::Group(_) | TileContent::Empty(_) => None,
            })
            .ok_or_else(|| format!("missing tile for process {pid}").into())
    }
}

fn frame_text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn paint(app: &mut App, width: u16, height: u16) -> TestResult<DrawnLayout> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    let keymap = app.keymap();
    // Check one unchanged frame after the initial allocation settles.
    for sample in 0..4 {
        app.tiles.settle_for_test();
        terminal.draw(|frame| draw_at_for_test(frame, app, &keymap, NOW))?;
        if sample > 0 && !app.tiles.has_pending_motion_for_test() {
            break;
        }
    }
    assert!(
        !app.tiles.has_pending_motion_for_test(),
        "the sampled grid is settled"
    );
    let groups = app
        .tiles
        .placements(app.selection_area, app.loaded_config.config.tiles.growth())
        .into_iter()
        .filter(|tile| matches!(tile.content, TileContent::Group(_)))
        .map(|tile| (tile.content, tile.frame.inner()))
        .collect();
    let cells = Cells::new(
        app.census.machines(&[]),
        NOW,
        app.footer_colors.colors(),
        &app.tree_sorts,
        app.summary_sort,
        &app.accounts.table,
    )
    .with_expansion(&app.expansion);
    let width_floor = cells.measure_trees(app.selection_area);
    let mut requested = Vec::new();
    for tile_width in [38, 39, 78] {
        let mut widths = vec![(TileContent::Summary, width.saturating_sub(2))];
        widths.extend(
            cells
                .tiles
                .iter()
                .map(|tile| (TileContent::Group(tile.key()), tile_width)),
        );
        let demand = cells.demands(&widths);
        requested.push(RequestedRows {
            summary: demand.summary,
            groups:  demand
                .groups
                .into_iter()
                .map(|group| (group.id, group.rows))
                .collect(),
        });
    }
    let summary_rows = requested.first().ok_or("summary measurement")?.summary;
    Ok(DrawnLayout {
        groups,
        summary_rows,
        width_floor,
        requested,
        text: frame_text(terminal.backend().buffer()),
    })
}

#[test]
fn waiting_clippy_mac_waiting_keeps_two_columns_at_80_by_45() -> TestResult {
    let mut source = SourceFrameState::new()?;
    let waiting = paint(&mut source.app, 80, 45)?;
    source.set_status(AgentActivity::Active(Activity::Verify {
        step:     VerifyStep::Clippy,
        location: RunLocation::Mac,
    }))?;
    let clippy = paint(&mut source.app, 80, 45)?;
    source.set_status(AgentActivity::Active(Activity::Waiting))?;
    let returned = paint(&mut source.app, 80, 45)?;
    assert_eq!(
        waiting.requested, clippy.requested,
        "status text does not change requested rows"
    );
    assert_eq!(
        waiting.requested, returned.requested,
        "returned status keeps requested rows"
    );
    assert!(!waiting.text.split_whitespace().any(|word| word == "age"));
    assert!(!clippy.text.split_whitespace().any(|word| word == "age"));
    let machines = source.app.census.machines(&[]);
    assert_eq!(
        crate::agent_cell::detail_property_for_test(&machines, "cargo-liner", "state", NOW),
        "waiting"
    );
    source.set_status(AgentActivity::Active(Activity::Verify {
        step:     VerifyStep::Clippy,
        location: RunLocation::Mac,
    }))?;
    let machines = source.app.census.machines(&[]);
    assert_eq!(
        crate::agent_cell::detail_property_for_test(&machines, "cargo-liner", "state", NOW),
        "clippy (mac)"
    );
    assert_eq!(waiting.summary_rows, clippy.summary_rows);
    assert_eq!(
        waiting.columns(),
        2,
        "frame 5 starts with two columns: {}",
        waiting.text
    );
    assert_eq!(
        [waiting.columns(), clippy.columns(), returned.columns()],
        [2, 2, 2],
        "the status cycle keeps columns; measure_trees/set_min_tile_width: [{}, {}, {}]\n{}",
        waiting.width_floor,
        clippy.width_floor,
        returned.width_floor,
        clippy.text
    );
    assert_eq!(
        waiting.groups, clippy.groups,
        "status width keeps tile positions"
    );
    assert_eq!(
        waiting.groups, returned.groups,
        "the return keeps tile positions"
    );
    Ok(())
}

#[test]
fn gained_and_lost_work_rows_change_the_tile_allocation() -> TestResult {
    let mut source = SourceFrameState::new()?;
    let before = paint(&mut source.app, 80, 60)?;
    let original = source.rows.clone();
    let liner = source
        .rows
        .iter_mut()
        .find(|row| row.pid == LINER_PID)
        .ok_or("live cargo-liner row")?;
    for index in 0..32 {
        liner.children.push(serde_json::from_value(json!({
            "depth":0,"kind":"shell","pid":600_000+index,
            "name":format!("additional work {index}"),"started":NOW-10
        }))?);
    }
    source.publish();
    let gained = paint(&mut source.app, 80, 60)?;
    assert_ne!(
        before.requested, gained.requested,
        "new work increases requested rows"
    );
    assert!(
        gained.agent_area(LINER_PID)?.height > before.agent_area(LINER_PID)?.height,
        "new work receives more rows: before={:?}, gained={:?}",
        before.groups,
        gained.groups
    );
    source.rows = original;
    source.publish();
    let lost = paint(&mut source.app, 80, 60)?;
    assert_eq!(before.requested, lost.requested);
    assert_eq!(
        before.groups, lost.groups,
        "removing work restores the smaller allocation"
    );
    Ok(())
}

#[test]
fn added_and_removed_tiles_change_the_grid_allocation() -> TestResult {
    let mut source = SourceFrameState::new()?;
    let before = paint(&mut source.app, 80, 45)?;
    let original_rows = source.rows.clone();
    let original_edges = source.edges.clone();
    let added: Vec<AgentRow> = serde_json::from_value(json!([
        {"agent":"claude","pid":700_000,"session_id":"added-showrunner","name":"new-showrunner",
         "started":NOW-100,"directory":"/fixture","activity":"idle","children":[]},
        {"agent":"claude","pid":700_001,"session_id":"added-unit","name":"new-unit",
         "started":NOW-100,"directory":"/fixture","activity":"idle","children":[]}
    ]))?;
    source.rows.extend(added);
    source.edges.0.push(serde_json::from_value(json!({
        "manager":"added-showrunner","member":"added-unit","role":"unit"
    }))?);
    source.publish();
    let added = paint(&mut source.app, 80, 45)?;
    assert_eq!(
        added.groups.len(),
        before.groups.len() + 1,
        "a new showrunner receives one tile"
    );
    let added_area = added.agent_area(700_000)?;
    assert!(
        added_area.width > 0 && added_area.height > 0,
        "the new tile has a visible allocation"
    );
    assert!(
        before.groups.iter().any(|(key, area)| {
            added
                .groups
                .iter()
                .any(|(next_key, next_area)| next_key == key && next_area != area)
        }),
        "the new tile receives space from the existing grid"
    );
    source.rows = original_rows;
    source.edges = original_edges;
    source.publish();
    let removed = paint(&mut source.app, 80, 45)?;
    assert_eq!(
        before.groups, removed.groups,
        "removing the tile restores its space"
    );
    Ok(())
}

#[test]
fn terminal_width_and_height_changes_reselect_the_layout() -> TestResult {
    let mut source = SourceFrameState::new()?;
    let before = paint(&mut source.app, 80, 45)?;
    let narrow = paint(&mut source.app, 50, 45)?;
    assert_eq!(
        (before.columns(), narrow.columns()),
        (2, 1),
        "a narrow terminal requires one column"
    );
    let returned = paint(&mut source.app, 80, 45)?;
    assert_eq!(
        before.groups, returned.groups,
        "restoring terminal width restores the grid"
    );
    let tall = paint(&mut source.app, 80, 70)?;
    assert!(
        tall.agent_area(346_735)?.height > before.agent_area(346_735)?.height,
        "a taller terminal exposes more hana rows: before={:?}, tall={:?}",
        before.groups,
        tall.groups
    );
    let short = paint(&mut source.app, 80, 45)?;
    assert_eq!(
        before.groups, short.groups,
        "restoring terminal height restores the grid"
    );
    Ok(())
}
