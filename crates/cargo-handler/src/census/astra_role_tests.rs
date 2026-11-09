//! Astra roles use the staff hierarchy and the named Codex account.

use std::path::PathBuf;

use rusqlite::Connection;
use rusqlite::params;
use serde_json::Error;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

use super::HierarchyRole;
use super::RoleLabel;
use super::account::AccountLabels;
use super::account::ConfigHome;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::herdr::HerdrNames;
use super::herdr::PaneWork;
use super::hierarchy::StaffFileReader;
use super::scan::LocalScanner;
use super::*;
use crate::render;

const DESIGNER: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9e01";
const REVIEWER: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9e02";

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Clone, Copy)]
enum AstraProcesses {
    Present,
    Absent,
}

struct AstraFixture {
    root:     TempDir,
    home:     PathBuf,
    scanner:  LocalScanner,
    staff:    Value,
    statuses: [String; 2],
}

impl AstraFixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let home = root.path().join("astra-account");
        std::fs::create_dir_all(home.join("sessions"))?;
        let staff = root.path().join("staff.json");
        let document = json!({"schema":2,"rows":[
            {"kind":"showrunner","id":"boss","name":"production boss","parent":null,"current_session":"boss-session"},
            {"kind":"unit","id":"unit","name":"unit director","parent":"boss","current_session":"unit-session"},
            {"kind":"worker","id":"designer","name":"designer session","parent":"unit","current_session":DESIGNER,"role":"astra-designer","home":home},
            {"kind":"worker","id":"reviewer","name":"reviewer session","parent":"boss","current_session":REVIEWER,"role":"astra-reviewer","home":home},
            {"kind":"future-kind","id":false,"parent":12}
        ]});
        std::fs::write(&staff, serde_json::to_vec(&document)?)?;
        let db = Connection::open(home.join("state_6.sqlite"))?;
        // Disable durability waits for `AstraFixture`'s temporary SQLite connection.
        db.pragma_update(None, "synchronous", "OFF")?;
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE threads (id TEXT PRIMARY KEY,cwd TEXT,created_at_ms INTEGER,name TEXT,first_user_message TEXT,rollout_path TEXT,originator TEXT,thread_source TEXT);")?;
        for (id, name) in [
            (DESIGNER, "named Astra designer"),
            (REVIEWER, "named Astra reviewer"),
        ] {
            let path = home.join("sessions").join(format!("rollout-{id}.jsonl"));
            std::fs::write(
                &path,
                "{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-6-astra\",\"effort\":\"high\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
            )?;
            db.execute(
                "INSERT INTO threads VALUES (?1,'/fixture',100000,?2,'',?3,'codex_exec','worker')",
                params![id, name, path.to_string_lossy()],
            )?;
        }
        let setting =
            StaffFileSetting::new(HierarchySource::File(staff.to_string_lossy().into_owned()));
        let scanner = LocalScanner::new(AccountLabels::default())
            .with_staff_file(setting, StaffFileReader::default());
        Ok(Self {
            root,
            home,
            scanner,
            staff: document,
            statuses: ["working".into(), "idle".into()],
        })
    }

    fn save_staff(&self) -> TestResult {
        std::fs::write(
            self.root.path().join("staff.json"),
            serde_json::to_vec(&self.staff)?,
        )?;
        Ok(())
    }

    fn process(&self, pid: u32, agent: &str, id: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: Some(1),
            name: agent.into(),
            arguments: if agent == "codex" {
                vec!["codex".into(), "resume".into(), id.into()]
            } else {
                vec![agent.into()]
            },
            started: 100,
            directory: Some("/fixture".into()),
            claude_pid: None,
            config_home: ConfigHome::Named(self.home.clone()),
        }
    }

    fn scan(
        &mut self,
        presence: AstraProcesses,
    ) -> Result<(Vec<AgentRow>, HierarchyEdges), Box<dyn std::error::Error>> {
        self.save_staff()?;
        self.scan_current(presence)
    }

    fn scan_current(
        &mut self,
        presence: AstraProcesses,
    ) -> Result<(Vec<AgentRow>, HierarchyEdges), Box<dyn std::error::Error>> {
        let mut processes = vec![
            self.process(10, "claude", "boss-session"),
            self.process(20, "claude", "unit-session"),
        ];
        if matches!(presence, AstraProcesses::Present) {
            processes.extend([
                self.process(30, "codex", DESIGNER),
                self.process(40, "codex", REVIEWER),
            ]);
        }
        let panes: Vec<_> = [30, 40].into_iter().zip(&self.statuses).map(|(pid, status)| json!({
            "pane_id":pid.to_string(), "tab_id":"workers", "terminal_id":format!("terminal-{pid}"),
            "revision":1,"agent_status":status
        })).collect();
        self.scanner
            .set_herdr_for_test(HerdrNames::from_snapshot_for_test(
                &processes,
                &json!({"snapshot":{"panes":panes,"tabs":[]}}),
            ));
        let sessions: Vec<SessionRecord> = serde_json::from_value(json!([
            {"pid":10,"sessionId":"boss-session","name":"production boss"},
            {"pid":20,"sessionId":"unit-session","name":"unit director"}
        ]))?;
        Ok(self
            .scanner
            .scan_astra_for_test(&processes, &sessions, self.root.path()))
    }
}

fn draw_astra(rows: Vec<AgentRow>, edges: HierarchyEdges) -> Vec<String> {
    let buffer = render::astra_frame_for_test(rows, edges);
    vec![
        buffer
            .content
            .chunks(220)
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n"),
    ]
}

fn assert_rendered_astra_managers(rows: Vec<AgentRow>, edges: HierarchyEdges) -> TestResult {
    let buffer = render::astra_frame_for_test(rows, edges);
    let names = [
        "production boss",
        "unit director",
        "designer session",
        "reviewer session",
    ];
    let mut positions = Vec::new();
    for (y, cells) in buffer.content.chunks(220).enumerate() {
        let line: String = cells.iter().map(ratatui::buffer::Cell::symbol).collect();
        if !line.starts_with('│')
            || !(line.contains("showrunner")
                || line.contains("unit director")
                || line.contains("astra "))
        {
            continue;
        }
        for name in names {
            if let Some(byte) = line.find(name) {
                positions.push((line[..byte].chars().count(), y, name));
            }
        }
    }
    for (child, manager) in [
        ("designer session", "unit director"),
        ("reviewer session", "production boss"),
    ] {
        let at = positions
            .iter()
            .position(|(_, _, name)| *name == child)
            .ok_or("drawn Astra session")?;
        let (column, y, _) = positions[at];
        let parent = positions[..at]
            .iter()
            .rfind(|(x, _, _)| *x < column)
            .ok_or("drawn manager")?;
        assert_eq!(
            parent.2, manager,
            "{child} at row {y} must draw below its manager"
        );
        assert!(parent.1 < y);
    }
    Ok(())
}

#[test]
fn astra_role_words_survive_the_role_boundary() -> Result<(), Error> {
    for (wire, word) in [
        ("astra-designer", "astra designer"),
        ("astra-reviewer", "astra reviewer"),
    ] {
        let role: HierarchyRole = serde_json::from_value(serde_json::json!(wire))?;
        assert_eq!(RoleLabel::Stated(role).column_text(), word);
    }
    Ok(())
}

#[test]
fn astra_exec_threads_keep_roles_metadata_and_real_managers() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    let (rows, edges) = fixture.scan(AstraProcesses::Present)?;
    assert_rendered_astra_managers(rows.clone(), edges.clone())?;
    for (pid, role, manager) in [
        (30, HierarchyRole::AstraDesigner, "unit-session"),
        (40, HierarchyRole::AstraReviewer, "boss-session"),
    ] {
        let row = rows
            .iter()
            .find(|row| row.pid == pid)
            .ok_or("live Astra row")?;
        assert!(row.session_id.astra_membership().is_member());
        assert_ne!(row.model, model::ModelName::Unrecorded);
        assert_eq!(row.effort, model::ReasoningEffort::High);
        assert!(
            edges.0.iter().any(|edge| edge.role == role
                && edge.manager == manager
                && row.session_id.ids().any(|id| id == edge.member)),
            "{pid} must remain under {manager}"
        );
    }
    let (decoded, restored) = probe::parse_report(&probe::encode_astra_test_report(
        rows.clone(),
        edges.clone(),
    )?)?;
    assert_eq!(decoded, rows);
    assert_eq!(restored, edges);
    Ok(())
}

#[test]
fn astra_close_resume_release_and_work_updates_preserve_identity() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    let (rows, _) = fixture.scan(AstraProcesses::Present)?;
    let first = rows
        .iter()
        .find(|row| row.pid == 30)
        .ok_or("designer")?
        .session_id
        .ids()
        .next()
        .ok_or("thread")?
        .to_owned();
    let (closed, _) = fixture.scan(AstraProcesses::Absent)?;
    assert!(!closed.iter().any(|row| row.pid == 30));
    fixture.statuses[0] = "idle".into();
    let (resumed, _) = fixture.scan(AstraProcesses::Present)?;
    let row = resumed
        .iter()
        .find(|row| row.pid == 30)
        .ok_or("resumed designer")?;
    assert_eq!(row.session_id.ids().next(), Some(first.as_str()));
    assert_eq!(row.activity, AgentActivity::Idle);
    fixture.staff["rows"][2]["role"] = json!("impl");
    let (released, edges) = fixture.scan(AstraProcesses::Present)?;
    assert!(
        !released
            .iter()
            .find(|row| row.pid == 30)
            .ok_or("plain worker")?
            .session_id
            .astra_membership()
            .is_member()
    );
    assert!(
        edges
            .0
            .iter()
            .any(|edge| edge.member == DESIGNER && edge.role == HierarchyRole::Worker)
    );
    Ok(())
}

#[test]
fn astra_missing_home_and_unknown_role_keep_plain_staff_workers() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    fixture.staff["rows"][2]
        .as_object_mut()
        .ok_or("worker object")?
        .remove("home");
    fixture.staff["rows"][2]["account"] =
        json!({"state":"AccountUnavailable","reason":"no backend"});
    fixture.staff["rows"][3]["role"] = json!({"state":"WorkerRoleUnavailable","reason":"no role"});
    let (rows, edges) = fixture.scan(AstraProcesses::Present)?;
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .all(|row| !row.session_id.astra_membership().is_member())
    );
    assert!(
        edges
            .0
            .iter()
            .filter(|edge| edge.member == DESIGNER || edge.member == REVIEWER)
            .all(|edge| edge.role == HierarchyRole::Worker)
    );
    let texts = draw_astra(rows, edges);
    assert!(
        texts
            .iter()
            .any(|text| text.contains("designer session") && text.contains("unit director"))
    );

    Ok(())
}

#[test]
fn astra_unknown_pane_state_keeps_live_members() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    fixture.statuses[0] = "future-state".into();
    let (rows, _) = fixture.scan(AstraProcesses::Present)?;
    let row = rows
        .iter()
        .find(|row| row.pid == 30)
        .ok_or("live Astra row")?;
    let AstraSessionMembership::Member(member) = row.session_id.astra_membership() else {
        return Err("Astra membership".into());
    };
    assert_eq!(member.work(), PaneWork::Unknown);
    assert_eq!(row.activity, AgentActivity::Unreported);
    Ok(())
}

#[test]
fn astra_requests_share_one_account_read_and_invalidate_on_a_wal_change() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    let writer = Connection::open(fixture.home.join("state_6.sqlite"))?;
    writer.execute_batch("PRAGMA journal_mode=WAL;")?;
    fixture.scan(AstraProcesses::Present)?;
    let count = fixture.scanner.astra_database_reads_for_test();
    assert_eq!(count, 1, "all three requesters share one account read");
    fixture.scan(AstraProcesses::Present)?;
    assert_eq!(
        fixture.scanner.astra_database_reads_for_test(),
        count,
        "unchanged scan reads no database rows"
    );
    writer.execute(
        "UPDATE threads SET name='changed Astra' WHERE id=?1",
        params![DESIGNER],
    )?;
    fixture.scan(AstraProcesses::Present)?;
    assert_eq!(
        fixture.scanner.astra_database_reads_for_test(),
        count + 1,
        "one WAL change causes one account read"
    );
    assert!(
        codex::read_threads(&fixture.home, &(0..=u64::MAX)).is_empty(),
        "startup discovery still excludes exec workers"
    );
    Ok(())
}

#[test]
fn astra_absent_metadata_stays_unrecorded_and_crossed_homes_do_not_gain_membership() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    std::fs::write(
        fixture
            .home
            .join("sessions")
            .join(format!("rollout-{DESIGNER}.jsonl")),
        "",
    )?;
    let (rows, _) = fixture.scan(AstraProcesses::Present)?;
    let row = rows.iter().find(|row| row.pid == 30).ok_or("designer")?;
    assert_eq!(row.model, model::ModelName::Unrecorded);
    assert_eq!(row.effort, model::ReasoningEffort::Unrecorded);
    fixture.staff["rows"][2]["home"] = json!(fixture.root.path().join("other-account"));
    let (rows, _) = fixture.scan(AstraProcesses::Present)?;
    assert!(
        !rows
            .iter()
            .find(|row| row.pid == 30)
            .ok_or("worker")?
            .session_id
            .astra_membership()
            .is_member()
    );
    Ok(())
}

#[test]
fn astra_first_scan_keeps_the_rollout_fast_mark() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    let path = fixture
        .home
        .join("sessions")
        .join(format!("rollout-{DESIGNER}.jsonl"));
    std::fs::write(
        path,
        "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"service_tier\":\"fast\"}}}\n",
    )?;
    let (rows, _) = fixture.scan(AstraProcesses::Present)?;
    assert_eq!(
        rows.iter()
            .find(|row| row.pid == 30)
            .ok_or("Astra designer")?
            .service_tier,
        ServiceTier::Fast
    );
    Ok(())
}

#[test]
fn astra_nested_reviewer_keeps_its_session_when_staff_fields_are_missing() -> TestResult {
    for missing in ["none", "home", "role", "both"] {
        let mut fixture = AstraFixture::new()?;
        if missing == "home" || missing == "both" {
            fixture.staff["rows"][3]
                .as_object_mut()
                .ok_or("worker")?
                .remove("home");
        }
        if missing == "role" || missing == "both" {
            fixture.staff["rows"][3]
                .as_object_mut()
                .ok_or("worker")?
                .remove("role");
        }
        fixture.save_staff()?;
        let mut reviewer = fixture.process(40, "codex", REVIEWER);
        reviewer.parent = Some(20);
        let processes = [
            fixture.process(10, "claude", "boss-session"),
            fixture.process(20, "claude", "unit-session"),
            reviewer,
        ];
        let sessions: Vec<SessionRecord> = serde_json::from_value(json!([
            {"pid":10,"sessionId":"boss-session","name":"production boss"},
            {"pid":20,"sessionId":"unit-session","name":"unit director"}
        ]))?;
        let (rows, edges) =
            fixture
                .scanner
                .scan_astra_for_test(&processes, &sessions, fixture.root.path());
        let row = rows
            .iter()
            .find(|row| row.pid == 40)
            .ok_or_else(|| format!("nested reviewer session is missing: {missing}"))?;
        assert_eq!(row.session_id.ids().next(), Some(REVIEWER));
        assert!(
            edges
                .0
                .iter()
                .any(|edge| edge.member == REVIEWER && edge.manager == "boss-session")
        );
        if missing != "none" {
            assert!(!row.session_id.astra_membership().is_member());
        }
    }
    Ok(())
}

#[test]
fn astra_staff_membership_does_not_depend_on_the_name_wait() -> TestResult {
    let mut fixture = AstraFixture::new()?;
    let staff = fixture.root.path().join("staff.json");
    let mut document: Value = serde_json::from_slice(&std::fs::read(&staff)?)?;
    for (index, role) in [(2, "astra-designer"), (3, "astra-reviewer")] {
        document["rows"][index]["role"] = json!(role);
        document["rows"][index]["home"] = json!(fixture.home);
    }
    std::fs::write(&staff, serde_json::to_vec(&document)?)?;
    fixture.scanner = LocalScanner::new(AccountLabels::default())
        .with_staff_file(
            StaffFileSetting::new(HierarchySource::File(staff.to_string_lossy().into_owned())),
            StaffFileReader::default(),
        )
        .with_herdr_probe_wait(crate::config::HerdrProbeWait::try_from(0)?);
    let (rows, _) = fixture.scan_current(AstraProcesses::Present)?;
    for pid in [30, 40] {
        assert!(
            rows.iter()
                .find(|row| row.pid == pid)
                .ok_or("worker")?
                .session_id
                .astra_membership()
                .is_member(),
            "zero wait retains staff membership for {pid}"
        );
    }
    Ok(())
}
