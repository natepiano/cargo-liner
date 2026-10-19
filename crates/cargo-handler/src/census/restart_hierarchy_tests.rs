//! Resumed units keep their production placement in each staged answer.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and rendered labels must exist"
)]

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde::Deserialize;

use super::account::AccountLabels;
use super::account::ConfigHome;
use super::accounts::AccountsTable;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::hierarchy::HierarchyCache;
use super::hierarchy::HierarchyError;
use super::hierarchy::HierarchyRunner;
use super::scan::ScanStage;
use super::tree::TreeSources;
use super::*;
use crate::agent_cell;
use crate::agent_cell::AgentTile;
use crate::constants::HIERARCHY_REFRESH_INTERVAL;
use crate::constants::LOCAL_SCAN_INTERVAL;
use crate::footer_colors::FooterColors;
use crate::sort::AccountsSort;
use crate::sort::SummarySort;
use crate::summary;
use crate::summary::SummaryFrame;

const PUBLICATION_PATIENCE: Duration = Duration::from_millis(300);
const FIXTURE_FIRST_BUDGET: Duration = Duration::from_millis(5);

#[derive(Deserialize)]
struct ProcessFixture {
    pid:       u32,
    parent:    u32,
    arguments: Vec<String>,
    directory: PathBuf,
}

#[derive(Deserialize)]
struct RestartFixture {
    processes: Vec<ProcessFixture>,
    sessions:  Vec<SessionRecord>,
}

fn fixture() -> (Vec<ProcessEntry>, Vec<SessionRecord>) {
    let fixture: RestartFixture =
        serde_json::from_str(include_str!("fixtures/restarted_unit_sessions.json"))
            .expect("resumed unit process tree");
    let processes = fixture
        .processes
        .into_iter()
        .map(|process| ProcessEntry {
            pid:         process.pid,
            parent:      Some(process.parent),
            name:        "claude".into(),
            arguments:   process.arguments,
            started:     100,
            directory:   Some(process.directory),
            claude_pid:  None,
            config_home: ConfigHome::Unread,
        })
        .collect();
    (processes, fixture.sessions)
}

fn classified(processes: &[ProcessEntry], sessions: &[SessionRecord]) -> Vec<AgentRow> {
    let mut rows = classify::agent_rows(
        processes,
        sessions,
        &[],
        Some(Path::new("/home/fixture")),
        &AccountLabels::default(),
    );
    tree::attach_children(
        &mut rows,
        processes,
        &TreeSources {
            sessions,
            ..TreeSources::default()
        },
    );
    rows
}

fn text(buffer: &Buffer) -> String {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn summary_text(rows: &[AgentRow], hierarchy: &HierarchySnapshot) -> String {
    let state = MachineState::Answered(rows.to_vec(), hierarchy.clone());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let accounts = AccountsTable::NotConfigured;
    let colors = FooterColors::default();
    let frame = SummaryFrame {
        accounts_sort:  AccountsSort::SourceOrder,
        accounts:       &accounts,
        machines:       &machines,
        placement:      &placement,
        cells:          &cells,
        now:            1000,
        account_column: AccountColumn::Hidden,
        footer_colors:  &colors,
        sort:           SummarySort::default(),
    };
    let area = Rect::new(
        0,
        0,
        summary::width(
            &machines,
            &placement,
            1000,
            AccountColumn::Hidden,
            SummarySort::default(),
            &accounts,
            AccountsSort::SourceOrder,
        ),
        u16::try_from(summary::height(&machines, &placement, &accounts)).expect("small summary"),
    );
    let mut buffer = Buffer::empty(area);
    summary::draw(&mut buffer, area, &frame);
    text(&buffer)
}

fn tiles(rows: &[AgentRow], hierarchy: &HierarchySnapshot) -> Vec<String> {
    let state = MachineState::Answered(rows.to_vec(), hierarchy.clone());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    agent_cell::group_tiles(&placement, &cells, &HashMap::new(), 1000)
        .iter()
        .map(|tile| {
            let area = Rect::new(0, 0, 240, 18);
            let mut buffer = Buffer::empty(area);
            match tile {
                AgentTile::Agent(entry) => agent_cell::draw(
                    &mut buffer,
                    area,
                    entry,
                    &cells,
                    1000,
                    AccountColumn::Hidden,
                    &FooterColors::default(),
                ),
                AgentTile::Unassigned(entry) => agent_cell::draw_unassigned(
                    &mut buffer,
                    area,
                    entry,
                    1000,
                    AccountColumn::Hidden,
                    &FooterColors::default(),
                ),
            }
            text(&buffer)
        })
        .collect()
}

fn production_rows(unit_pid: u32) -> Vec<AgentRow> {
    let (mut processes, mut sessions) = fixture();
    processes.retain(|process| process.pid != 30);
    sessions.retain(|session| session.pid != 30);
    processes
        .iter_mut()
        .find(|process| process.pid == 20)
        .expect("unit process")
        .pid = unit_pid;
    sessions
        .iter_mut()
        .find(|session| session.pid == 20)
        .expect("unit session")
        .pid = unit_pid;
    classified(&processes, &sessions)
}

fn production_edges() -> HierarchyEdges {
    hierarchy::parse(include_bytes!("fixtures/restarted_unit_hierarchy.json"))
        .expect("live shutdown-unit hierarchy edge")
}

struct Answers(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);

impl HierarchyRunner for Answers {
    fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
        self.0
            .lock()
            .expect("hierarchy answers")
            .pop_front()
            .expect("next hierarchy answer")
    }
}

fn assert_unit_is_nested(rows: &[AgentRow], hierarchy: &HierarchySnapshot) {
    let summary = summary_text(rows, hierarchy);
    assert!(summary.contains("natedev"), "{summary}");
    assert!(summary.contains("1 top-level agent"), "{summary}");
    assert!(!summary.contains("shutdown-unit"), "{summary}");
    let drawn = tiles(rows, hierarchy);
    assert_eq!(drawn.len(), 1, "the unit has no separate tile");
    let tile = &drawn[0];
    assert_eq!(tile.matches("shutdown-unit").count(), 1, "{tile}");
    let unit = tile
        .lines()
        .find(|line| line.contains("shutdown-unit"))
        .expect("unit row");
    assert!(unit.contains("└─") || unit.contains("├─"), "{tile}");
    assert!(unit.split_whitespace().any(|word| word == "unit"), "{tile}");
}

#[test]
fn restart_hierarchy_first_publication_never_draws_a_known_unit_as_top_level() {
    let runner = Arc::new(Answers(Mutex::new(VecDeque::from([
        Ok(production_edges()),
    ]))));
    let updates = schedule::spawn_with_test_source(
        HierarchyCommand::new(HierarchySource::Command("fixture production tree".into())),
        runner,
        |answers| {
            answers
                .publish(ScanStage::Classified, production_rows(200))
                .expect("classified rows");
            answers
                .publish(ScanStage::Completed, production_rows(200))
                .expect("completed rows");
        },
    );
    let publications: Vec<_> = updates.into_iter().collect();
    assert!(!publications.is_empty(), "the scan publishes rows");
    let mut unit_publications = 0;
    for update in publications {
        if let CensusUpdate::Local {
            rows, hierarchy, ..
        } = update
            && rows.iter().any(|row| row.pid == 200)
        {
            unit_publications += 1;
            assert_unit_is_nested(&rows, &hierarchy);
        }
    }
    assert!(unit_publications > 0, "the first scan publishes the unit");
}

#[test]
fn restart_hierarchy_new_pid_keeps_the_unit_nested_in_every_staged_publication() {
    let runner = Arc::new(Answers(Mutex::new(VecDeque::from([
        Ok(production_edges()),
    ]))));
    let updates = schedule::spawn_with_test_source(
        HierarchyCommand::new(HierarchySource::Command("fixture production tree".into())),
        runner,
        |answers| {
            answers
                .publish(ScanStage::Classified, production_rows(20))
                .expect("original classified rows");
            answers
                .publish(ScanStage::Completed, production_rows(20))
                .expect("original completed rows");
            answers
                .publish(ScanStage::Classified, production_rows(200))
                .expect("resumed classified rows");
            answers
                .publish(ScanStage::Completed, production_rows(200))
                .expect("resumed completed rows");
        },
    );
    let mut resumed = 0;
    for update in updates {
        if let CensusUpdate::Local {
            rows, hierarchy, ..
        } = update
            && rows.iter().any(|row| row.pid == 200)
        {
            resumed += 1;
            assert_unit_is_nested(&rows, &hierarchy);
        }
    }
    assert!(resumed > 0, "the resumed pid reaches the frame");
}

#[test]
fn restart_hierarchy_new_session_is_placed_before_its_first_published_frame() {
    let runner = Arc::new(Answers(Mutex::new(VecDeque::from([
        Ok(HierarchyEdges::default()),
        Ok(production_edges()),
    ]))));
    let updates = schedule::spawn_with_test_source(
        HierarchyCommand::new(HierarchySource::Command("fixture production tree".into())),
        runner,
        |answers| {
            let mut before = production_rows(20);
            before.retain(|row| row.pid != 20);
            let now = Instant::now();
            answers
                .publish_at(ScanStage::Classified, before.clone(), now)
                .expect("showrunner classified");
            answers
                .settle_test_request()
                .expect("accept fixture hierarchy reply");
            answers
                .publish_at(ScanStage::Completed, before, now)
                .expect("showrunner completed");
            answers
                .settle_test_request()
                .expect("accept fixture hierarchy reply");
            let resumed = now + LOCAL_SCAN_INTERVAL;
            answers
                .publish_at(ScanStage::Classified, production_rows(200), resumed)
                .expect("new unit classified");
            answers
                .settle_test_request()
                .expect("accept fixture hierarchy reply");
            answers
                .publish_at(ScanStage::Completed, production_rows(200), resumed)
                .expect("new unit completed");
            answers
                .settle_test_request()
                .expect("accept fixture hierarchy reply");
        },
    );
    let mut unit_publications = 0;
    for update in updates {
        if let CensusUpdate::Local {
            rows, hierarchy, ..
        } = update
            && rows.iter().any(|row| row.pid == 200)
        {
            unit_publications += 1;
            assert_unit_is_nested(&rows, &hierarchy);
        }
    }
    assert!(unit_publications > 0, "the new unit reaches the frame");
}

#[test]
fn restart_hierarchy_unmapped_home_session_stays_in_the_top_level_table() {
    let (processes, sessions) = fixture();
    let rows = classified(&processes, &sessions);
    let hierarchy = HierarchySnapshot::from(production_edges());
    let summary = summary_text(&rows, &hierarchy);
    assert!(summary.contains("ordinary home session"), "{summary}");
    assert!(summary.contains("2 top-level agents"), "{summary}");
    assert!(!summary.contains("shutdown-unit"), "{summary}");
    let drawn = tiles(&rows, &hierarchy);
    assert_eq!(drawn.len(), 2);
    assert!(
        drawn
            .iter()
            .any(|tile| tile.contains("ordinary home session"))
    );
}

#[test]
fn restart_hierarchy_refresh_failure_keeps_the_resumed_unit_nested() {
    let pool_root = tempfile::tempdir().expect("empty reviewer pools");
    let mut cache = HierarchyCache::with_pool_root(pool_root.path().to_owned());
    let runner = Answers(Mutex::new(VecDeque::from([
        Ok(production_edges()),
        Err(HierarchyError::TimedOut),
        Ok(production_edges()),
    ])));
    let source = HierarchySource::Command("fixture production tree".into());
    let now = Instant::now();
    let original = production_rows(20);
    let initial = cache.snapshot(source.clone(), &original, &runner, now);
    assert_unit_is_nested(&original, &initial);
    let resumed = production_rows(200);
    let failed = cache.snapshot(
        source.clone(),
        &resumed,
        &runner,
        now + HIERARCHY_REFRESH_INTERVAL,
    );
    assert_eq!(
        failed.notice,
        HierarchyNotice::Failed(HierarchyError::TimedOut)
    );
    assert_unit_is_nested(&resumed, &failed);
    let recovered = cache.snapshot(
        source,
        &resumed,
        &runner,
        now + HIERARCHY_REFRESH_INTERVAL + Duration::from_millis(1),
    );
    assert_eq!(recovered.notice, HierarchyNotice::Clear);
    assert_unit_is_nested(&resumed, &recovered);
}

#[test]
fn restart_hierarchy_missing_unit_edge_refreshes_on_the_next_scan() {
    let pool_root = tempfile::tempdir().expect("empty reviewer pools");
    let mut cache = HierarchyCache::with_pool_root(pool_root.path().to_owned());
    let runner = Answers(Mutex::new(VecDeque::from([
        Ok(HierarchyEdges::default()),
        Ok(production_edges()),
    ])));
    let source = HierarchySource::Command("fixture production tree".into());
    let rows = production_rows(200);
    let now = Instant::now();
    cache.snapshot(source.clone(), &rows, &runner, now);
    let linked = cache.snapshot(source, &rows, &runner, now);
    assert_eq!(
        runner.0.lock().expect("answers").len(),
        0,
        "the missing edge is checked before the normal cache expires"
    );
    assert_unit_is_nested(&rows, &linked);
}

#[test]
fn restart_hierarchy_unmapped_session_gets_one_refresh_until_a_new_id_arrives() {
    let pool_root = tempfile::tempdir().expect("empty reviewer pools");
    let mut cache = HierarchyCache::with_pool_root(pool_root.path().to_owned());
    let runner = Answers(Mutex::new(VecDeque::from([
        Ok(production_edges()),
        Ok(production_edges()),
        Ok(production_edges()),
    ])));
    let source = HierarchySource::Command("fixture production tree".into());
    let (processes, sessions) = fixture();
    let mut rows = classified(&processes, &sessions);
    let now = Instant::now();
    cache.snapshot(source.clone(), &rows, &runner, now);
    let checked = cache.snapshot(source.clone(), &rows, &runner, now + LOCAL_SCAN_INTERVAL);
    assert_eq!(
        runner.0.lock().expect("answers").len(),
        1,
        "the unnamed id receives one extra check"
    );
    let summary = summary_text(&rows, &checked);
    assert!(summary.contains("ordinary home session"), "{summary}");
    assert!(summary.contains("2 top-level agents"), "{summary}");
    for tick in 2..=4 {
        cache.snapshot(
            source.clone(),
            &rows,
            &runner,
            now + LOCAL_SCAN_INTERVAL * tick,
        );
        assert_eq!(
            runner.0.lock().expect("answers").len(),
            1,
            "the same unnamed id does not refresh on each scan"
        );
    }
    rows.iter_mut()
        .find(|row| row.pid == 30)
        .expect("home session")
        .session_id = SessionIdentity::Claude("another-ordinary-session".into());
    let new_session = cache.snapshot(
        source.clone(),
        &rows,
        &runner,
        now + LOCAL_SCAN_INTERVAL * 4,
    );
    assert_eq!(
        runner.0.lock().expect("answers").len(),
        0,
        "a new id receives its own check at the same observation time"
    );
    for tick in 5..=7 {
        cache.snapshot(
            source.clone(),
            &rows,
            &runner,
            now + LOCAL_SCAN_INTERVAL * tick,
        );
    }
    let summary = summary_text(&rows, &new_session);
    assert!(summary.contains("ordinary home session"), "{summary}");
    assert!(summary.contains("2 top-level agents"), "{summary}");
}

/// The test controls each scan and answer while the scheduler stays alive.
enum FixtureScan {
    Publish {
        rows:         Vec<AgentRow>,
        acknowledged: Sender<()>,
    },
    Stop,
}

struct ControlledAnswers {
    requested: Sender<()>,
    replies:   Mutex<Receiver<Result<HierarchyEdges, HierarchyError>>>,
}

impl HierarchyRunner for ControlledAnswers {
    fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
        self.requested.send(()).expect("record hierarchy request");
        self.replies
            .lock()
            .expect("answer channel")
            .recv()
            .unwrap_or(Err(HierarchyError::OutputUnreadable))
    }
}

struct RunningFixture {
    scans:     Sender<FixtureScan>,
    requested: Receiver<()>,
    replies:   Sender<Result<HierarchyEdges, HierarchyError>>,
    updates:   Receiver<CensusUpdate>,
}

impl RunningFixture {
    fn new() -> Self {
        let (scans, commands) = mpsc::channel();
        let (request, requested) = mpsc::channel();
        let (replies, answers) = mpsc::channel();
        let runner = Arc::new(ControlledAnswers {
            requested: request,
            replies:   Mutex::new(answers),
        });
        let updates = schedule::spawn_with_test_source_and_budget(
            HierarchyCommand::new(HierarchySource::Command("fixture production tree".into())),
            runner,
            FIXTURE_FIRST_BUDGET,
            move |answers| {
                for command in &commands {
                    match command {
                        FixtureScan::Publish { rows, acknowledged } => {
                            answers
                                .publish(ScanStage::Classified, rows)
                                .expect("fixture scan publishes");
                            acknowledged.send(()).expect("scan acknowledged");
                        },
                        FixtureScan::Stop => return,
                    }
                }
            },
        );
        Self {
            scans,
            requested,
            replies,
            updates,
        }
    }

    fn publish(&self, rows: Vec<AgentRow>) {
        let (acknowledged, published) = mpsc::channel();
        self.scans
            .send(FixtureScan::Publish { rows, acknowledged })
            .expect("fixture is running");
        published
            .recv_timeout(PUBLICATION_PATIENCE)
            .expect("scan returned with the answer still pending");
    }

    fn answer(&self, edges: HierarchyEdges) {
        self.requested
            .recv_timeout(PUBLICATION_PATIENCE)
            .expect("hierarchy command requested");
        self.replies
            .send(Ok(edges))
            .expect("answer reaches the running scheduler");
    }

    fn containing(&self, pid: u32) -> (Vec<AgentRow>, HierarchySnapshot) {
        loop {
            let update = self
                .updates
                .recv_timeout(PUBLICATION_PATIENCE)
                .expect("the live scheduler publishes without another scan or Finished");
            if let CensusUpdate::Local {
                rows, hierarchy, ..
            } = update
                && rows.iter().any(|row| row.pid == pid)
            {
                return (rows, hierarchy);
            }
        }
    }
}

impl Drop for RunningFixture {
    fn drop(&mut self) { let _ = self.scans.send(FixtureScan::Stop); }
}

fn independent_codex() -> AgentRow {
    serde_json::from_value(serde_json::json!({
        "agent":"codex", "name":"independent codex", "pid":40,
        "started":100, "directory":"/work/fixture", "children":[]
    }))
    .expect("an unrelated row can publish before hierarchy")
}

#[test]
fn restart_hierarchy_late_first_answer_publishes_other_rows_and_holds_the_unit() {
    let fixture = RunningFixture::new();
    let mut input = production_rows(200);
    input.push(independent_codex());
    fixture.publish(input);
    let (rows, hierarchy) = fixture.containing(40);
    assert!(
        !rows.iter().any(|row| row.pid == 200),
        "the pending unit is held back"
    );
    let summary = summary_text(&rows, &hierarchy);
    assert!(summary.contains("independent codex"), "{summary}");
    assert!(!summary.contains("shutdown-unit"), "{summary}");
    assert!(
        !tiles(&rows, &hierarchy)
            .iter()
            .any(|tile| tile.contains("shutdown-unit"))
    );

    fixture.answer(production_edges());
    let (mut rows, hierarchy) = fixture.containing(200);
    rows.retain(|row| row.pid != 40);
    assert_unit_is_nested(&rows, &hierarchy);
    // The fixture sends neither another scan nor Stop before this assertion.
}

#[test]
fn restart_hierarchy_codex_worker_waits_for_its_units_first_answer() {
    let fixture = RunningFixture::new();
    let mut input = production_rows(200);
    input.push(independent_codex());
    input.push(
        serde_json::from_str(include_str!("fixtures/restarted_unit_codex.json"))
            .expect("visible Codex owner with a managed worker"),
    );
    fixture.publish(input);
    let (rows, hierarchy) = fixture.containing(40);
    assert!(
        !rows
            .iter()
            .flat_map(|row| &row.children)
            .any(|child| child.name == "compile crates"),
        "the worker does not draw under its old owner before discovery"
    );
    assert!(
        !tiles(&rows, &hierarchy)
            .iter()
            .any(|tile| tile.contains("compile crates"))
    );

    let edges = hierarchy::parse(include_bytes!(
        "fixtures/restarted_unit_worker_hierarchy.json"
    ))
    .expect("unit and worker management links");
    fixture.answer(edges);
    let (rows, hierarchy) = fixture.containing(200);
    let drawn = tiles(&rows, &hierarchy);
    let production = drawn
        .iter()
        .find(|tile| tile.contains("shutdown-unit"))
        .expect("unit tile");
    assert!(production.contains("compile crates"), "{production}");
    assert_eq!(
        drawn
            .iter()
            .map(|tile| tile.matches("compile crates").count())
            .sum::<usize>(),
        1
    );
    let worker = production
        .lines()
        .find(|line| line.contains("compile crates"))
        .expect("worker row");
    assert!(
        worker.split_whitespace().any(|word| word == "impl"),
        "{production}"
    );
    let unit_line = production
        .lines()
        .position(|line| line.contains("shutdown-unit"))
        .expect("unit line");
    let worker_line = production
        .lines()
        .position(|line| line.contains("compile crates"))
        .expect("worker line");
    assert!(unit_line < worker_line, "{production}");
    let unit = production.lines().nth(unit_line).expect("unit row");
    let unit_rail = unit
        .chars()
        .position(|letter| matches!(letter, '└' | '├'))
        .expect("unit tree rail");
    let worker_rail = worker
        .chars()
        .position(|letter| matches!(letter, '└' | '├'))
        .expect("worker tree rail");
    assert!(
        worker_rail > unit_rail,
        "the worker is below its unit: {production}"
    );
}

#[test]
fn restart_hierarchy_session_seen_during_pending_answer_refreshes_before_it_draws() {
    let fixture = RunningFixture::new();
    let mut first_rows = production_rows(200);
    first_rows.retain(|row| row.pid != 200);
    first_rows.push(independent_codex());
    fixture.publish(first_rows);
    fixture.containing(40);

    let mut later_rows = production_rows(200);
    later_rows.push(independent_codex());
    fixture.publish(later_rows);
    fixture.containing(40);
    fixture.answer(HierarchyEdges::default());
    fixture.answer(production_edges());
    let (mut rows, hierarchy) = fixture.containing(200);
    rows.retain(|row| row.pid != 40);
    assert_unit_is_nested(&rows, &hierarchy);
}
