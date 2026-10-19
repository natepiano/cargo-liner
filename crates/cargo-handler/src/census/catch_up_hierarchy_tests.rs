//! Hierarchy refreshes leave scan publication and remote scheduling available.

#![expect(
    clippy::expect_used,
    reason = "fixture channels and rendered rows must exist"
)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::accounts::AccountsTable;
use super::hierarchy::HierarchyError;
use super::hierarchy::HierarchyRunner;
use super::remote::ProbeOutcome;
use super::remote::RemoteRunner;
use super::scan::ScanStage;
use super::*;
use crate::agent_cell;
use crate::agent_cell::AgentTile;
use crate::constants::HIERARCHY_REFRESH_INTERVAL;
use crate::footer_colors::FooterColors;
use crate::sort::SummarySort;
use crate::summary;
use crate::summary::SummaryFrame;

const PATIENCE: Duration = Duration::from_millis(200);
const FIRST_BUDGET: Duration = Duration::from_millis(2);
const MANAGER: u32 = 10;
const UNIT: u32 = 20;
const HOME: u32 = 30;
const INDEPENDENT: u32 = 40;
const SOURCE: &str = "fixture hierarchy";
const REPLACEMENT: &str = "replacement hierarchy";
const REMOTE: &str = "added-while-hierarchy-blocked";

fn rows(pids: &[u32]) -> Vec<AgentRow> {
    let mut rows: Vec<AgentRow> =
        serde_json::from_str(include_str!("fixtures/catch_up_rows.json")).expect("source rows");
    rows.retain(|row| pids.contains(&row.pid));
    rows
}

fn links() -> HierarchyEdges {
    hierarchy::parse(include_bytes!("fixtures/catch_up_links.json")).expect("manager to unit link")
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|line| {
            line.iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Drawn {
    summary: String,
    tiles:   Vec<String>,
}

fn draw(rows: &[AgentRow], hierarchy: &HierarchySnapshot) -> Drawn {
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
        ),
        u16::try_from(summary::height(&machines, &placement, &accounts)).expect("small summary"),
    );
    let mut buffer = Buffer::empty(area);
    summary::draw(&mut buffer, area, &frame);
    let summary = text(&buffer);
    let tiles = agent_cell::group_tiles(&placement, &cells, &HashMap::new(), 1000)
        .iter()
        .map(|tile| {
            let area = Rect::new(0, 0, 180, 16);
            let mut buffer = Buffer::empty(area);
            match tile {
                AgentTile::Agent(entry) => agent_cell::draw(
                    &mut buffer,
                    area,
                    entry,
                    &cells,
                    1000,
                    AccountColumn::Hidden,
                    &colors,
                ),
                AgentTile::Unassigned(entry) => agent_cell::draw_unassigned(
                    &mut buffer,
                    area,
                    entry,
                    1000,
                    AccountColumn::Hidden,
                    &colors,
                ),
            }
            text(&buffer)
        })
        .collect();
    Drawn { summary, tiles }
}

fn assert_nested(rows: &[AgentRow], hierarchy: &HierarchySnapshot) {
    let drawn = draw(rows, hierarchy);
    assert!(!drawn.summary.contains("new unit"), "{}", drawn.summary);
    let tile = drawn
        .tiles
        .iter()
        .find(|tile| tile.contains("new unit"))
        .expect("unit draws");
    assert!(
        drawn.summary.contains("production manager"),
        "{}",
        drawn.summary
    );
    assert!(
        tile.lines()
            .next()
            .expect("tile header")
            .contains(&format!("pid {MANAGER}")),
        "{tile}"
    );
    let unit = tile
        .lines()
        .find(|line| line.contains("new unit"))
        .expect("unit row");
    assert!(unit.contains("└─") || unit.contains("├─"), "{tile}");
    assert!(unit.split_whitespace().any(|word| word == "unit"), "{unit}");
    assert_eq!(
        drawn
            .tiles
            .iter()
            .map(|tile| tile.matches("new unit").count())
            .sum::<usize>(),
        1
    );
}

struct Request {
    command: String,
    reply:   Sender<Result<HierarchyEdges, HierarchyError>>,
}

impl Request {
    fn answer(self, answer: Result<HierarchyEdges, HierarchyError>) {
        self.reply.send(answer).expect("command accepts its result");
    }
}

struct Commands {
    requests: Sender<Request>,
    count:    Arc<AtomicUsize>,
}

impl HierarchyRunner for Commands {
    fn run(&self, command: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
        let (reply, result) = mpsc::channel();
        self.count.fetch_add(1, Ordering::SeqCst);
        self.requests
            .send(Request {
                command: command.into(),
                reply,
            })
            .expect("command is observed");
        result
            .recv()
            .unwrap_or(Err(HierarchyError::OutputUnreadable))
    }
}

struct Probes;

impl RemoteRunner for Probes {
    fn probe(&self, _: &str) -> ProbeOutcome {
        ProbeOutcome::Finished {
            status: Some(0),
            stdout: br#"{"schema":3,"machine":"remote","rows":[]}"#.to_vec(),
        }
    }
}

enum Scan {
    Publish {
        rows:     Vec<AgentRow>,
        now:      Instant,
        returned: Sender<()>,
    },
    Stop,
}

#[derive(Clone, Copy)]
enum UnitPlacement {
    AwaitingFirstLink,
    AlreadyLinked,
}

struct Fixture {
    scans:    Sender<Scan>,
    requests: Receiver<Request>,
    updates:  Receiver<CensusUpdate>,
    count:    Arc<AtomicUsize>,
    source:   HierarchyCommand,
    remotes:  RemoteMachines,
    now:      Instant,
}

impl Fixture {
    fn new() -> Self {
        let (scans, input) = mpsc::channel();
        let (requests, observed) = mpsc::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let source = HierarchyCommand::new(HierarchySource::Command(SOURCE.into()));
        let remotes = RemoteMachines::new(Vec::new());
        let updates = schedule::spawn_with_test_remote_and_budget(
            remotes.clone(),
            source.clone(),
            Arc::new(Commands {
                requests,
                count: Arc::clone(&count),
            }),
            Arc::new(Probes),
            FIRST_BUDGET,
            move |answers| {
                for scan in &input {
                    match scan {
                        Scan::Publish {
                            rows,
                            now,
                            returned,
                        } => {
                            answers
                                .publish_at(ScanStage::Classified, rows, now)
                                .expect("scan publishes");
                            let _ = returned.send(());
                        },
                        Scan::Stop => return,
                    }
                }
            },
        );
        Self {
            scans,
            requests: observed,
            updates,
            count,
            source,
            remotes,
            now: Instant::now(),
        }
    }

    fn publish(&self, rows: Vec<AgentRow>, now: Instant) {
        let (returned, done) = mpsc::channel();
        self.scans
            .send(Scan::Publish {
                rows,
                now,
                returned,
            })
            .expect("scan source is open");
        done.recv_timeout(PATIENCE)
            .expect("publication lock is free while hierarchy runs");
    }

    fn request(&self) -> Request {
        self.requests
            .recv_timeout(PATIENCE)
            .expect("hierarchy request starts")
    }

    fn containing(&self, pid: u32) -> (Vec<AgentRow>, HierarchySnapshot) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self
                .updates
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("scheduler publishes while the scan callback stays open")
            {
                CensusUpdate::Local {
                    rows, hierarchy, ..
                } if rows.iter().any(|row| row.pid == pid) => return (rows, hierarchy),
                CensusUpdate::Local { .. } | CensusUpdate::Remote { .. } => {},
            }
        }
    }

    fn assert_progress(&self, now: Instant, unit: UnitPlacement) {
        self.remotes.replace(vec![REMOTE.into()]);
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let CensusUpdate::Remote { host, state } = self
                .updates
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("a newly configured remote is probed while hierarchy runs")
            {
                assert_eq!(host, REMOTE);
                assert!(matches!(state, MachineState::Answered(..)), "{state:?}");
                break;
            }
        }
        let mut updated = rows(&[MANAGER, UNIT, INDEPENDENT]);
        updated
            .iter_mut()
            .find(|row| row.pid == INDEPENDENT)
            .expect("safe row")
            .name = "safe row updated".into();
        self.publish(updated, now);
        let (safe, hierarchy) = self.containing(INDEPENDENT);
        let drawn = draw(&safe, &hierarchy);
        assert!(
            drawn.summary.contains("safe row updated"),
            "{}",
            drawn.summary
        );
        if matches!(unit, UnitPlacement::AwaitingFirstLink) {
            assert!(!drawn.summary.contains("new unit"), "{}", drawn.summary);
            assert!(
                !drawn.tiles.iter().any(|tile| tile.contains("new unit")),
                "the newcomer stays held"
            );
        }
    }

    fn initial(&self, input: Vec<AgentRow>, answer: HierarchyEdges) {
        self.publish(input, self.now);
        self.request().answer(Ok(answer));
        self.containing(MANAGER);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = self.scans.send(Scan::Stop); }
}

#[test]
fn catch_up_hierarchy_followup_allows_remote_and_safe_local_publications() {
    let fixture = Fixture::new();
    fixture.publish(rows(&[MANAGER, INDEPENDENT]), fixture.now);
    let first = fixture.request();
    fixture.publish(rows(&[MANAGER, UNIT, INDEPENDENT]), fixture.now);
    first.answer(Ok(HierarchyEdges::default()));
    let followup = fixture.request();
    fixture.assert_progress(fixture.now, UnitPlacement::AwaitingFirstLink);
    followup.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    assert_nested(&rows, &hierarchy);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
}

#[test]
fn catch_up_hierarchy_new_session_refresh_does_not_hold_publication_or_remote_probes() {
    let fixture = Fixture::new();
    fixture.initial(rows(&[MANAGER, INDEPENDENT]), HierarchyEdges::default());
    fixture.publish(rows(&[MANAGER, UNIT, INDEPENDENT]), fixture.now);
    let refresh = fixture.request();
    fixture.assert_progress(fixture.now, UnitPlacement::AwaitingFirstLink);
    refresh.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    assert_nested(&rows, &hierarchy);
}

#[test]
fn catch_up_hierarchy_expired_answer_refresh_does_not_hold_publication_or_remote_probes() {
    let fixture = Fixture::new();
    fixture.initial(rows(&[MANAGER, UNIT, INDEPENDENT]), links());
    let expired = fixture.now + HIERARCHY_REFRESH_INTERVAL;
    fixture.publish(rows(&[MANAGER, UNIT, INDEPENDENT]), expired);
    let refresh = fixture.request();
    fixture.assert_progress(expired, UnitPlacement::AlreadyLinked);
    refresh.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    assert_nested(&rows, &hierarchy);
}

#[test]
fn catch_up_hierarchy_refresh_holds_the_units_whole_linked_component() {
    let fixture = Fixture::new();
    fixture.initial(rows(&[MANAGER, UNIT, INDEPENDENT]), links());
    let mut refreshing = rows(&[MANAGER, UNIT, INDEPENDENT]);
    let worker: ChildRow =
        serde_json::from_str(include_str!("fixtures/catch_up_unnamed_thread.json"))
            .expect("unnamed Codex thread");
    refreshing
        .iter_mut()
        .find(|row| row.pid == MANAGER)
        .expect("manager")
        .children
        .push(worker);
    refreshing
        .iter_mut()
        .find(|row| row.pid == INDEPENDENT)
        .expect("safe row")
        .name = "component refresh starting".into();
    let expired = fixture.now + HIERARCHY_REFRESH_INTERVAL;
    fixture.publish(refreshing.clone(), expired);
    let refresh = fixture.request();
    refreshing
        .iter_mut()
        .find(|row| row.pid == INDEPENDENT)
        .expect("safe row")
        .name = "component refresh progress".into();
    fixture.publish(refreshing, expired);
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let CensusUpdate::Local {
            rows, hierarchy, ..
        } = fixture
            .updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("safe rows publish while the component waits")
        {
            let drawn = draw(&rows, &hierarchy);
            assert!(!drawn.summary.contains("new unit"), "{}", drawn.summary);
            if drawn.tiles.iter().any(|tile| tile.contains("new unit")) {
                assert_nested(&rows, &hierarchy);
            }
            if drawn.summary.contains("component refresh progress") {
                assert!(
                    !drawn.tiles.iter().any(|tile| tile.contains("new unit")),
                    "the linked unit waits with its manager: {:?}",
                    drawn.tiles
                );
                assert!(
                    !drawn.summary.contains("production manager"),
                    "{}",
                    drawn.summary
                );
                break;
            }
        }
    }
    refresh.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    assert_nested(&rows, &hierarchy);
}

#[test]
fn catch_up_hierarchy_checked_unnamed_session_stays_visible_during_an_expired_refresh() {
    let fixture = Fixture::new();
    let input = rows(&[MANAGER, UNIT, HOME, INDEPENDENT]);
    fixture.initial(input.clone(), links());
    fixture.publish(input.clone(), fixture.now);
    fixture.request().answer(Ok(links()));
    let (checked, hierarchy) = fixture.containing(HOME);
    assert!(
        draw(&checked, &hierarchy)
            .summary
            .contains("ordinary home session")
    );

    let mut refreshing = input;
    refreshing
        .iter_mut()
        .find(|row| row.pid == INDEPENDENT)
        .expect("safe row")
        .name = "settled session refresh starting".into();
    let expired = fixture.now + HIERARCHY_REFRESH_INTERVAL;
    fixture.publish(refreshing.clone(), expired);
    let refresh = fixture.request();
    refreshing
        .iter_mut()
        .find(|row| row.pid == INDEPENDENT)
        .expect("safe row")
        .name = "settled session refresh progress".into();
    fixture.publish(refreshing, expired);
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let CensusUpdate::Local {
            rows, hierarchy, ..
        } = fixture
            .updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("settled rows publish while hierarchy runs")
        {
            let drawn = draw(&rows, &hierarchy);
            assert!(
                drawn.summary.contains("ordinary home session"),
                "a checked top-level session stays visible: {}",
                drawn.summary
            );
            assert!(
                drawn
                    .tiles
                    .iter()
                    .any(|tile| tile.contains("ordinary home session")),
                "the checked session keeps its tile row: {:?}",
                drawn.tiles
            );
            if drawn.summary.contains("settled session refresh progress") {
                break;
            }
        }
    }
    refresh.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(HOME);
    assert_nested(&rows, &hierarchy);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 3);
}

#[test]
fn catch_up_hierarchy_chained_answers_keep_unnamed_sessions_checked() {
    let fixture = Fixture::new();
    fixture.publish(rows(&[MANAGER, HOME, INDEPENDENT]), fixture.now);
    let first = fixture.request();
    fixture.publish(rows(&[MANAGER, UNIT, HOME, INDEPENDENT]), fixture.now);
    first.answer(Ok(HierarchyEdges::default()));
    let second = fixture.request();
    let mut with_new_id = rows(&[MANAGER, UNIT, HOME, INDEPENDENT]);
    let mut newcomer = with_new_id
        .iter()
        .find(|row| row.pid == HOME)
        .expect("home row")
        .clone();
    newcomer.pid = 31;
    newcomer.session_id = SessionIdentity::Claude("another-unmapped-session".into());
    with_new_id.push(newcomer);
    fixture.publish(with_new_id.clone(), fixture.now);
    second.answer(Ok(links()));
    fixture.request().answer(Ok(links()));
    let (published, hierarchy) = fixture.containing(31);
    assert_nested(&published, &hierarchy);
    fixture.publish(with_new_id, fixture.now);
    fixture.containing(31);
    assert_eq!(
        fixture.count.load(Ordering::SeqCst),
        3,
        "a refresh must keep earlier checked ids"
    );
    assert!(matches!(
        fixture.requests.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
}

#[test]
fn catch_up_hierarchy_failed_followup_releases_retained_links_and_retry_stays_async() {
    let fixture = Fixture::new();
    fixture.initial(rows(&[MANAGER, UNIT, INDEPENDENT]), links());
    let expired = fixture.now + HIERARCHY_REFRESH_INTERVAL;
    fixture.publish(rows(&[MANAGER, UNIT, HOME, INDEPENDENT]), expired);
    let failed_request = fixture.request();
    let (held, hierarchy) = fixture.containing(INDEPENDENT);
    assert!(
        !draw(&held, &hierarchy)
            .summary
            .contains("ordinary home session")
    );
    failed_request.answer(Err(HierarchyError::TimedOut));
    let deadline = Instant::now() + PATIENCE;
    let (rows, hierarchy) = loop {
        let update = fixture
            .updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("failure releases rows");
        if let CensusUpdate::Local {
            rows, hierarchy, ..
        } = update
            && matches!(
                hierarchy.notice,
                HierarchyNotice::Failed(HierarchyError::TimedOut)
            )
            && rows.iter().any(|row| row.pid == HOME)
        {
            break (rows, hierarchy);
        }
    };
    assert_nested(&rows, &hierarchy);
    let drawn = draw(&rows, &hierarchy);
    assert!(
        drawn.summary.contains("hierarchy: timed out"),
        "{}",
        drawn.summary
    );
    assert!(
        drawn.summary.contains("ordinary home session"),
        "{}",
        drawn.summary
    );
    fixture.publish(rows, expired);
    let retry = fixture.request();
    fixture.assert_progress(expired, UnitPlacement::AlreadyLinked);
    retry.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    assert_eq!(hierarchy.notice, HierarchyNotice::Clear);
    assert_nested(&rows, &hierarchy);
}

#[test]
fn catch_up_hierarchy_source_change_discards_the_pending_sources_late_links() {
    let fixture = Fixture::new();
    fixture.initial(rows(&[MANAGER, INDEPENDENT]), HierarchyEdges::default());
    fixture.publish(rows(&[MANAGER, UNIT, INDEPENDENT]), fixture.now);
    let old = fixture.request();
    assert_eq!(old.command, SOURCE);
    fixture
        .source
        .replace(HierarchySource::Command(REPLACEMENT.into()));
    old.answer(Ok(links()));
    let replacement = fixture.request();
    assert_eq!(replacement.command, REPLACEMENT);
    replacement.answer(Ok(HierarchyEdges::default()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    let drawn = draw(&rows, &hierarchy);
    assert_eq!(hierarchy.edges.0, [] as [HierarchyEdge; 0]);
    assert!(drawn.summary.contains("new unit"), "{}", drawn.summary);
}

#[test]
fn catch_up_hierarchy_disabled_source_does_not_restore_late_links() {
    let fixture = Fixture::new();
    fixture.initial(rows(&[MANAGER, INDEPENDENT]), HierarchyEdges::default());
    fixture.publish(rows(&[MANAGER, UNIT, INDEPENDENT]), fixture.now);
    let old = fixture.request();
    fixture.source.replace(HierarchySource::Off);
    old.answer(Ok(links()));
    let (rows, hierarchy) = fixture.containing(UNIT);
    assert_eq!(hierarchy.notice, HierarchyNotice::Clear);
    assert_eq!(hierarchy.edges.0, [] as [HierarchyEdge; 0]);
    let drawn = draw(&rows, &hierarchy);
    assert!(drawn.summary.contains("new unit"), "{}", drawn.summary);
    fixture.publish(rows, fixture.now);
    let (_, hierarchy) = fixture.containing(UNIT);
    assert_eq!(hierarchy.edges.0, [] as [HierarchyEdge; 0]);
    assert_eq!(fixture.count.load(Ordering::SeqCst), 2);
}
