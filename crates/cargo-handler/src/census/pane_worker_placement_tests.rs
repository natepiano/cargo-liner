//! Pane workers use proven thread identities for staff placement and probe transport.

#![expect(clippy::expect_used, reason = "fixture rows and files must exist")]

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use rusqlite::Connection;
use rusqlite::params;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

use super::account::AccountLabels;
use super::account::ConfigHome;
use super::accounts::EMPTY_SESSION_STATES;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::hierarchy::StaffDiscovery;
use super::hierarchy::StaffFileReader;
use super::scan::LocalScanner;
use super::*;
use crate::agent_cell;
use crate::agent_cell::AgentTile;
use crate::footer_colors::FooterColors;

const FIRST_THREAD: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9d7a";
const SECOND_THREAD: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9d7b";
#[cfg(target_os = "linux")]
const UNLISTED_THREAD: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9d7c";

struct Fixture {
    root:    TempDir,
    setting: StaffFileSetting,
}

impl Fixture {
    fn new() -> Self {
        let root = TempDir::new().expect("fixture directory");
        let staff = root.path().join("staff.json");
        std::fs::write(
            &staff,
            serde_json::to_vec(&json!({"schema": 2, "rows": [
                {"id":"boss", "kind":"showrunner", "name":"staff boss",
                 "parent":null, "current_session":"boss-session"},
                {"id":"first-unit", "kind":"unit", "name":"first staff unit",
                 "parent":"boss", "current_session":"first-unit-session"},
                {"id":"second-unit", "kind":"unit", "name":"second staff unit",
                 "parent":"boss", "current_session":"second-unit-session"},
                {"id":"first-worker", "kind":"worker", "name":"first staff worker",
                 "parent":"first-unit", "current_session":FIRST_THREAD, "role":null},
                {"id":"second-worker", "kind":"worker", "name":"second staff worker",
                 "parent":"second-unit", "current_session":SECOND_THREAD, "role":null}
            ]}))
            .expect("staff JSON"),
        )
        .expect("staff file");
        let setting =
            StaffFileSetting::new(HierarchySource::File(staff.to_string_lossy().into_owned()));
        Self { root, setting }
    }

    fn process(&self, pid: u32, arguments: &[&str]) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: Some(1),
            name: arguments[0].into(),
            arguments: arguments.iter().map(|word| (*word).into()).collect(),
            started: 100,
            directory: Some("/work/fixture".into()),
            claude_pid: None,
            config_home: ConfigHome::Named(self.root.path().join(".codex")),
        }
    }

    fn thread(&self, id: &str) -> PathBuf {
        let home = self.root.path().join(".codex");
        let sessions = home.join("sessions");
        std::fs::create_dir_all(&sessions).expect("sessions directory");
        let rollout = sessions.join(format!("rollout-2026-11-07-{id}.jsonl"));
        std::fs::write(&rollout, "").expect("rollout file");
        let database = Connection::open(home.join("state_6.sqlite")).expect("Codex database");
        database
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY, cwd TEXT NOT NULL, created_at_ms INTEGER,
                name TEXT, first_user_message TEXT, rollout_path TEXT,
                originator TEXT DEFAULT 'codex-tui')",
            )
            .expect("threads table");
        database.execute(
            "INSERT INTO threads (id, cwd, created_at_ms, name, first_user_message, rollout_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, "/work/fixture", 100_000, "thread fallback", "", rollout.to_string_lossy()],
        ).expect("stored thread");
        rollout
    }

    fn scan(&self, workers: Vec<ProcessEntry>) -> (Vec<AgentRow>, HierarchyEdges) {
        let mut processes = vec![
            self.process(10, &["claude"]),
            self.process(20, &["claude"]),
            self.process(30, &["claude"]),
        ];
        processes.extend(workers);
        let sessions: Vec<SessionRecord> = serde_json::from_value(json!([
            {"pid":10, "sessionId":"boss-session", "name":"boss fallback"},
            {"pid":20, "sessionId":"first-unit-session", "name":"first unit fallback"},
            {"pid":30, "sessionId":"second-unit-session", "name":"second unit fallback"}
        ]))
        .expect("Claude sessions");
        let mut scanner = LocalScanner::new(AccountLabels::default())
            .with_staff_file(self.setting.clone(), StaffFileReader::default());
        let mut rows = Vec::new();
        scanner.scan_stages(
            &processes,
            &sessions,
            Some(self.root.path()),
            |answer| rows = answer,
            |_, _| {},
        );
        let snapshot = StaffDiscovery::File {
            setting: self.setting.clone(),
            reader:  StaffFileReader::default(),
        }
        .snapshot();
        let edges = snapshot.publication().matched(&rows).edges;
        (rows, edges)
    }
}

enum DrawnTile {
    Managed(String),
    Unassigned(String),
}

fn drawn_tiles(rows: Vec<AgentRow>, edges: HierarchyEdges) -> Vec<DrawnTile> {
    let state = MachineState::Answered(rows, edges.into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    agent_cell::group_tiles(&placement, &cells, &HashMap::new(), 1000)
        .iter()
        .map(|tile| {
            let area = Rect::new(0, 0, 240, 24);
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
            let text = (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            match tile {
                AgentTile::Agent(_) => DrawnTile::Managed(text),
                AgentTile::Unassigned(_) => DrawnTile::Unassigned(text),
            }
        })
        .collect()
}

fn assert_managed_worker(tiles: &[DrawnTile], worker: &str, unit: &str) {
    let text = tiles
        .iter()
        .find_map(|tile| match tile {
            DrawnTile::Managed(text) if text.contains(worker) => Some(text),
            DrawnTile::Managed(_) | DrawnTile::Unassigned(_) => None,
        })
        .expect("worker belongs to a managed tile");
    let line = text
        .lines()
        .find(|line| line.contains(worker))
        .expect("worker row");
    assert!(
        text.contains(unit),
        "worker must share its manager's tile: {text}"
    );
    assert!(
        line.split_whitespace().any(|word| word == "worker"),
        "worker role: {line}"
    );
    assert!(
        line.replace(worker, "")
            .split_whitespace()
            .any(|word| word == "worker"),
        "the role column must show worker separately from the name: {line}"
    );
    assert!(line.contains('─'), "worker must have a tree rail: {line}");
    assert!(
        !tiles
            .iter()
            .any(|tile| matches!(tile, DrawnTile::Unassigned(text) if text.contains(worker)))
    );
}

#[test]
fn resumed_pane_worker_draws_its_staff_name_and_role_below_its_unit() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
    ]);
    assert_managed_worker(
        &drawn_tiles(rows, edges),
        "first staff worker",
        "first staff unit",
    );
}

#[test]
fn a_resume_directory_argument_does_not_prove_a_staff_thread() {
    let fixture = Fixture::new();
    let (rows, edges) = fixture.scan(vec![fixture.process(
        40,
        &["codex", "--no-daemon", "--cd", "resume", FIRST_THREAD],
    )]);
    let worker = rows
        .iter()
        .find(|row| row.pid == 40)
        .expect("live Codex worker");
    assert_eq!(worker.session_id, SessionIdentity::Unrecorded);
    let name = worker.name.clone();
    assert!(
        drawn_tiles(rows, edges)
            .iter()
            .any(|tile| matches!(tile, DrawnTile::Unassigned(text) if text.contains(&name)),)
    );
}

#[test]
fn remote_option_values_do_not_prove_a_staff_thread() {
    for option in ["--remote-auth-token-env", "--remote"] {
        let fixture = Fixture::new();
        let (rows, edges) = fixture.scan(vec![
            fixture.process(40, &["codex", option, "resume", FIRST_THREAD]),
        ]);
        let worker = rows
            .iter()
            .find(|row| row.pid == 40)
            .expect("live Codex worker");
        assert_eq!(worker.session_id, SessionIdentity::Unrecorded, "{option}");
        let name = worker.name.clone();
        assert!(
            drawn_tiles(rows, edges)
                .iter()
                .any(|tile| matches!(tile, DrawnTile::Unassigned(text) if text.contains(&name))),
            "{option} must leave the worker unassigned"
        );
    }
}

#[test]
fn resumed_pane_worker_uses_command_line_proof_without_a_thread_database() {
    let fixture = Fixture::new();
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
    ]);
    assert_managed_worker(
        &drawn_tiles(rows, edges),
        "first staff worker",
        "first staff unit",
    );
}

#[test]
fn an_unknown_codex_switch_keeps_the_resumed_worker_under_its_unit() {
    let fixture = Fixture::new();
    let (rows, edges) = fixture.scan(vec![fixture.process(
        40,
        &[
            "codex",
            "--dangerously-bypass-hook-trust",
            "resume",
            FIRST_THREAD,
        ],
    )]);
    assert_managed_worker(
        &drawn_tiles(rows, edges),
        "first staff worker",
        "first staff unit",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn pane_worker_holding_its_staff_rollout_draws_below_its_unit() {
    let fixture = Fixture::new();
    let rollout = fixture.thread(FIRST_THREAD);
    let held = std::fs::File::open(rollout).expect("held rollout");
    let (rows, edges) = fixture.scan(vec![fixture.process(std::process::id(), &["codex"])]);
    assert_managed_worker(
        &drawn_tiles(rows, edges),
        "first staff worker",
        "first staff unit",
    );
    drop(held);
}

#[test]
fn pane_worker_without_a_held_rollout_does_not_use_cwd_or_start_time_as_staff_proof() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    let (rows, edges) = fixture.scan(vec![fixture.process(40, &["codex"])]);
    let worker = rows
        .iter()
        .find(|row| row.pid == 40)
        .expect("live Codex worker");
    assert_eq!(worker.agent, Agent::Codex);
    assert_eq!(worker.session_id, SessionIdentity::Unrecorded);
    let name = worker.name.clone();
    let tiles = drawn_tiles(rows, edges);
    assert!(
        tiles
            .iter()
            .any(|tile| matches!(tile, DrawnTile::Unassigned(text) if text.contains(&name)))
    );
    assert!(!tiles.iter().any(|tile| match tile {
        DrawnTile::Managed(text) | DrawnTile::Unassigned(text) =>
            text.contains("first staff worker"),
    }));
}

#[cfg(target_os = "linux")]
#[test]
fn pane_worker_holding_an_unlisted_rollout_does_not_claim_a_staff_thread() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    let rollout = fixture.thread(UNLISTED_THREAD);
    let held = std::fs::File::open(rollout).expect("held unlisted rollout");
    let pid = std::process::id();
    let (rows, edges) = fixture.scan(vec![fixture.process(pid, &["codex"])]);
    let worker = rows
        .iter()
        .find(|row| row.pid == pid)
        .expect("live Codex worker");
    assert_eq!(worker.session_id, SessionIdentity::Unrecorded);
    let name = worker.name.clone();
    let tiles = drawn_tiles(rows, edges);
    assert!(
        tiles
            .iter()
            .any(|tile| matches!(tile, DrawnTile::Unassigned(text) if text.contains(&name)))
    );
    assert!(!tiles.iter().any(|tile| match tile {
        DrawnTile::Managed(text) | DrawnTile::Unassigned(text) =>
            text.contains("first staff worker"),
    }));
    drop(held);
}

#[test]
fn resumed_pane_workers_keep_their_distinct_staff_managers() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    fixture.thread(SECOND_THREAD);
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
        fixture.process(50, &["codex", "resume", SECOND_THREAD]),
    ]);
    let first = rows.iter().find(|row| row.pid == 40).expect("first worker");
    let second = rows
        .iter()
        .find(|row| row.pid == 50)
        .expect("second worker");
    assert_eq!(first.name, "first staff worker");
    assert_eq!(second.name, "second staff worker");
    let tiles = drawn_tiles(rows.clone(), edges.clone());
    let state = MachineState::Answered(rows, edges.into());
    let placement = AgentPlacement::from_machines(
        &[Machine {
            name:  "fixture",
            state: &state,
        }],
        &EMPTY_SESSION_STATES,
    );
    for (worker, manager) in [(40, 20), (50, 30)] {
        let row = placement
            .rows
            .iter()
            .find(|row| row.row.pid == worker)
            .expect("placed worker");
        assert!(
            matches!(row.placement, Placement::Managed { manager: identity, .. } if identity.pid == manager)
        );
    }
    assert_managed_worker(&tiles, "first staff worker", "first staff unit");
    assert_managed_worker(&tiles, "second staff worker", "second staff unit");
}

#[test]
fn a_pane_codex_identity_matches_staff_when_no_claude_row_is_present() {
    let fixture = Fixture::new();
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
    ]);
    let workers: Vec<_> = rows.into_iter().filter(|row| row.pid == 40).collect();
    assert_eq!(workers.len(), 1);
    let snapshot = hierarchy::HierarchySnapshot::from(edges).matched(&workers);
    assert_eq!(snapshot.notice, hierarchy::HierarchyNotice::Clear);
}

#[test]
fn a_pane_codex_identity_preserves_its_specific_worker_role() {
    let fixture = Fixture::new();
    let path = fixture.root.path().join("staff.json");
    let mut staff: Value =
        serde_json::from_slice(&std::fs::read(&path).expect("staff document")).expect("staff JSON");
    staff["rows"][3]["role"] = serde_json::json!("test");
    std::fs::write(&path, staff.to_string()).expect("staff worker role");
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
    ]);
    let edge = edges
        .0
        .iter()
        .find(|edge| edge.member == FIRST_THREAD)
        .expect("staff worker edge");
    assert_eq!(edge.role, HierarchyRole::Test);
    let state = MachineState::Answered(rows, edges.into());
    let placement = AgentPlacement::from_machines(
        &[Machine {
            name:  "fixture",
            state: &state,
        }],
        &EMPTY_SESSION_STATES,
    );
    let worker = placement
        .rows
        .iter()
        .find(|row| row.row.pid == 40)
        .expect("placed worker");
    assert!(
        matches!(worker.placement, Placement::Managed { manager, role: HierarchyRole::Test } if manager.pid == 20)
    );
}

#[test]
fn staff_worker_without_a_live_process_draws_no_row() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    let (rows, edges) = fixture.scan(Vec::new());
    assert_eq!(rows.len(), 3);
    assert!(!drawn_tiles(rows, edges).iter().any(|tile| match tile {
        DrawnTile::Managed(text) | DrawnTile::Unassigned(text) => text.contains("staff worker"),
    }));
}

#[test]
fn pane_worker_without_a_thread_stays_unassigned() {
    let fixture = Fixture::new();
    let (rows, edges) = fixture.scan(vec![fixture.process(40, &["codex"])]);
    let worker = rows
        .iter()
        .find(|row| row.pid == 40)
        .expect("live Codex worker");
    assert_eq!(worker.agent, Agent::Codex);
    assert_eq!(worker.session_id, SessionIdentity::Unrecorded);
    let name = worker.name.clone();
    assert!(
        drawn_tiles(rows, edges)
            .iter()
            .any(|tile| matches!(tile, DrawnTile::Unassigned(text) if text.contains(&name)))
    );
}

#[derive(Deserialize)]
struct PreviousReport {
    schema:  u32,
    machine: String,
    rows:    Vec<PreviousRow>,
    edges:   HierarchyEdges,
}

#[derive(Deserialize)]
struct PreviousRow {
    session_id: Option<String>,
    name:       String,
    pid:        u32,
}

#[test]
fn previous_probe_receiver_accepts_a_pane_worker_with_codex_identity() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
    ]);
    let envelope = json!({"schema":3, "machine":"fixture", "rows":rows, "edges":edges});
    let previous: PreviousReport =
        serde_json::from_value(envelope).expect("previous probe receiver");
    assert_eq!(previous.schema, 3);
    assert_eq!(previous.machine, "fixture");
    let worker = previous
        .rows
        .iter()
        .find(|row| row.pid == 40)
        .expect("worker received");
    assert_eq!(worker.session_id, None);
    assert_eq!(worker.name, "first staff worker");
    assert_eq!(previous.rows.len(), 4);
    assert_eq!(previous.edges, edges);
}

#[test]
fn current_probe_receiver_keeps_a_pane_worker_under_its_unit() {
    let fixture = Fixture::new();
    fixture.thread(FIRST_THREAD);
    let (rows, edges) = fixture.scan(vec![
        fixture.process(40, &["codex", "resume", FIRST_THREAD]),
    ]);
    let encoded = serde_json::to_vec(&rows).expect("probe rows");
    let decoded: Vec<AgentRow> = serde_json::from_slice(&encoded).expect("current probe receiver");
    assert_eq!(decoded, rows);
    assert_managed_worker(
        &drawn_tiles(decoded, edges),
        "first staff worker",
        "first staff unit",
    );
}
