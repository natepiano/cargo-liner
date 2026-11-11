//! Staff file parsing, caching and live name joins.

#![expect(clippy::expect_used, reason = "fixture files and rows must exist")]

use std::fs::File;
use std::fs::FileTimes;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use serde_json::Value;

use super::*;
use crate::census::Agent;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::ProvenCodexThread;
use crate::census::StaffRowNames;
use crate::census::classify;

const THREAD: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9d7a";

#[test]
fn staff_identity_resolves_the_current_session_and_manager() {
    let roster = StaffRoster::parse(&document()).expect("checked staff");
    let worker = WorkerReference::Staff {
        staff_id: "worker".into(),
    };
    assert_eq!(worker.session(&roster), Some(THREAD));
    assert_eq!(worker.parent_session(&roster), Some("unit-live"));
    let legacy = WorkerReference::LegacyThread {
        thread_id: "worker".into(),
    };
    assert_eq!(legacy.session(&roster), Some("worker"));
    assert_eq!(legacy.parent_session(&roster), None);
    let root = WorkerReference::Staff {
        staff_id: "boss".into(),
    };
    assert_eq!(root.session(&roster), Some("claude-live"));
    assert_eq!(root.parent_session(&roster), None);
    let missing = WorkerReference::Staff {
        staff_id: "unlisted".into(),
    };
    assert_eq!(missing.session(&roster), None);
    assert_eq!(missing.parent_session(&roster), None);
}

#[test]
fn staff_roles_and_units_come_from_the_checked_member() -> Result<(), &'static str> {
    for (role, expected) in [
        (Value::Null, StaffWorkerRole::Unknown),
        (serde_json::json!("future"), StaffWorkerRole::Unknown),
        (serde_json::json!(12), StaffWorkerRole::Unknown),
        (
            serde_json::json!("impl"),
            StaffWorkerRole::Known(WorkerRole::Impl),
        ),
        (
            serde_json::json!("test"),
            StaffWorkerRole::Known(WorkerRole::Test),
        ),
        (
            serde_json::json!("review"),
            StaffWorkerRole::Known(WorkerRole::Review),
        ),
        (
            serde_json::json!("fix"),
            StaffWorkerRole::Known(WorkerRole::Fix),
        ),
        (
            serde_json::json!("astra-designer"),
            StaffWorkerRole::Known(WorkerRole::AstraDesigner),
        ),
        (
            serde_json::json!("astra-reviewer"),
            StaffWorkerRole::Known(WorkerRole::AstraReviewer),
        ),
    ] {
        let mut value: Value = serde_json::from_slice(&document()).expect("staff fixture");
        value["rows"][2]["role"] = role;
        let roster = StaffRoster::parse(&serde_json::to_vec(&value).expect("staff document"))
            .expect("checked roles");
        let StaffMemberLookup::Managed { member, manager } = roster.member(THREAD) else {
            return Err("a worker must retain its manager");
        };
        assert_eq!(member.id, "worker");
        assert_eq!(member.session, THREAD);
        assert_eq!(member.role, expected);
        assert_eq!(manager, "unit-live");
        assert_eq!(
            roster
                .edges
                .0
                .iter()
                .find(|edge| edge.member == THREAD)
                .expect("worker edge")
                .role,
            expected.hierarchy()
        );
        assert!(matches!(
            roster.member("claude-live"),
            StaffMemberLookup::Root(_)
        ));
        assert!(matches!(
            roster.member("unlisted"),
            StaffMemberLookup::Unavailable
        ));
    }
    Ok(())
}

fn document() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schema":2,"rows":[
        {"id":"boss","kind":"showrunner","name":"staff boss","parent":null,"current_session":"claude-live"},
        {"id":"unit","kind":"unit","name":"staff unit","parent":"boss","current_session":"unit-live"},
        {"id":"worker","kind":"worker","name":"staff worker","parent":"unit","current_session":THREAD},
        {"id":"morty","kind":"agent","name":"morty","parent":null,"current_session":"agent-live"},
        {"id":"future","kind":"future","name":"future","parent":"boss","current_session":"future-live"},
        {"id":"dead","kind":"worker","name":"dead worker","parent":"unit","current_session":"dead-session"}
    ]})).expect("staff JSON")
}

fn row(pid: u32, session: &str, fallback: &str) -> AgentRow {
    serde_json::from_value(serde_json::json!({
        "agent":"claude", "session_id":session, "name":fallback,
        "started":1, "pid":pid, "directory":"/fixture", "children":[]
    }))
    .expect("live row")
}

#[test]
fn staff_schema_two_maps_kinds_and_ignores_unknown_kinds() {
    let roster = StaffRoster::parse(&document()).expect("schema two");
    assert_eq!(
        roster
            .edges
            .0
            .iter()
            .map(|edge| edge.role)
            .collect::<Vec<_>>(),
        [
            HierarchyRole::Worker,
            HierarchyRole::Unit,
            HierarchyRole::Worker
        ]
    );
    assert!(matches!(
        roster.name("agent-live"),
        StaffName::Named("morty")
    ));
    assert!(matches!(roster.name("future-live"), StaffName::Unlisted));
}

#[test]
fn a_missing_staff_file_preserves_live_names_without_an_error() {
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("missing-staff.json");
    let source = HierarchySource::File(path.to_string_lossy().into_owned());
    let reader = StaffFileReader::default();
    let roster = reader.roster(&source);
    let mut rows = vec![row(1, "claude-live", "herdr name")];
    classify::attach_staff_names(&mut rows, &roster);
    assert_eq!(roster.names, BTreeMap::new());
    assert_eq!(rows[0].name, "herdr name");
    assert_eq!(rows[0].staff_names, StaffRowNames::default());
    assert!(matches!(
        reader.publication(&source),
        StaffPublication::Ready(HierarchySnapshot {
            edges: HierarchyEdges(edges),
            notice: HierarchyNotice::Clear,
        }) if edges.is_empty()
    ));
}

#[test]
fn staff_file_reuses_mtime_and_reloads_changed_and_missing_files() {
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("staff.json");
    let source = HierarchySource::File(path.to_string_lossy().into_owned());
    let reader = StaffFileReader::default();
    let missing = reader.roster(&source);
    assert_eq!(missing.edges, HierarchyEdges::default());
    assert!(matches!(
        reader.publication(&source),
        StaffPublication::Ready(HierarchySnapshot {
            notice: HierarchyNotice::Clear,
            ..
        })
    ));
    fs::write(&path, document()).expect("staff file");
    let first_time = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    File::options()
        .write(true)
        .open(&path)
        .expect("staff handle")
        .set_times(FileTimes::new().set_modified(first_time))
        .expect("staff mtime");
    let first = reader.roster(&source);
    assert_eq!(
        first.edges,
        HierarchyEdges(vec![
            HierarchyEdge {
                manager: "unit-live".into(),
                member:  "dead-session".into(),
                role:    HierarchyRole::Worker,
            },
            HierarchyEdge {
                manager: "claude-live".into(),
                member:  "unit-live".into(),
                role:    HierarchyRole::Unit,
            },
            HierarchyEdge {
                manager: "unit-live".into(),
                member:  THREAD.into(),
                role:    HierarchyRole::Worker,
            },
        ])
    );
    fs::write(&path, br#"{"schema":2,"rows":[]}"#).expect("staff replacement");
    File::options()
        .write(true)
        .open(&path)
        .expect("staff handle")
        .set_times(FileTimes::new().set_modified(first_time))
        .expect("same mtime");
    assert!(
        Arc::ptr_eq(&first, &reader.roster(&source)),
        "an unchanged mtime must reuse the parsed copy"
    );
    File::options()
        .write(true)
        .open(&path)
        .expect("staff handle")
        .set_times(FileTimes::new().set_modified(first_time + Duration::from_secs(1)))
        .expect("new mtime");
    let changed = reader.roster(&source);
    assert!(!Arc::ptr_eq(&first, &changed));
    assert_eq!(changed.edges, HierarchyEdges::default());
    fs::remove_file(&path).expect("remove staff file");
    assert_eq!(reader.roster(&source).edges, HierarchyEdges::default());
}

#[test]
fn staff_foreign_schema_and_unreadable_files_publish_no_error_row() {
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("staff.json");
    for bytes in [br#"{"schema":3,"rows":[]}"#.as_slice(), b"broken JSON"] {
        fs::write(&path, bytes).expect("invalid staff file");
        let reader = StaffFileReader::default();
        let source = HierarchySource::File(path.to_string_lossy().into_owned());
        assert!(
            matches!(reader.publication(&source), StaffPublication::Ready(HierarchySnapshot { edges: HierarchyEdges(edges), notice: HierarchyNotice::Clear }) if edges.is_empty())
        );
        assert!(matches!(
            reader.roster(&source).name("claude-live"),
            StaffName::Unlisted
        ));
    }
    let source = HierarchySource::File(directory.path().to_string_lossy().into_owned());
    assert!(
        matches!(
            StaffFileReader::default().publication(&source),
            StaffPublication::Ready(HierarchySnapshot {
                edges: HierarchyEdges(edges), notice: HierarchyNotice::Clear
            }) if edges.is_empty()
        ),
        "a directory cannot supply staff names"
    );
}

#[test]
fn staff_names_join_live_claude_and_proven_codex_without_adding_dead_rows() {
    let roster = StaffRoster::parse(&document()).expect("staff roster");
    let mut rows = vec![
        row(1, "claude-live", "old Claude name"),
        row(2, "", "thread name"),
    ];
    rows[1].agent = Agent::Codex;
    rows[1].session_id = SessionIdentity::Codex(ProvenCodexThread::ResumeArgument(THREAD.into()));
    classify::attach_staff_names(&mut rows, &roster);
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["staff boss", "staff worker"]
    );
    assert_eq!(rows.len(), 2, "dead staff rows must not create census rows");
    rows[1].name = "herdr name".into();
    rows[1].session_id = SessionIdentity::Unrecorded;
    classify::attach_staff_names(&mut rows, &roster);
    assert_eq!(
        rows[1].name, "herdr name",
        "an unproven thread must not receive a staff name"
    );
}

#[test]
fn staff_name_precedes_thread_herdr_and_pid_fallback_names() {
    let staff = StaffRoster::parse(&document()).expect("staff roster");
    for fallback in ["proven thread name", "herdr name", "pid 1"] {
        let mut rows = vec![
            row(1, "claude-live", fallback),
            row(2, "unlisted", fallback),
        ];
        classify::attach_staff_names(&mut rows, &staff);
        assert_eq!(rows[0].name, "staff boss");
        assert_eq!(
            rows[1].name, fallback,
            "a missing staff name must preserve the fallback"
        );
    }
}

#[test]
fn staff_worker_roles_use_the_checked_staff_snapshot() {
    for (kind, expected) in [("producer", "producer"), ("agent", "")] {
        let source = StaffRoster::parse(&serde_json::to_vec(&serde_json::json!({"schema":2,"rows":[
            {"id":"producer","kind":kind,"name":"source-name","parent":null,"current_session":"source-session"}
        ]})).expect("source document")).expect("checked source");
        let mut rows = vec![row(1, "source-session", "fallback")];
        classify::attach_staff_names(&mut rows, &source);
        let state = MachineState::Answered(rows, source.edges.into());
        assert_eq!(
            crate::agent_cell::detail_property_for_test(
                &[Machine {
                    name:  "fixture",
                    state: &state,
                }],
                "source-name",
                "role",
                100
            ),
            expected
        );
    }

    let roster = StaffRoster::parse(&serde_json::to_vec(&serde_json::json!({"schema":2,"rows":[
        {"id":"boss","kind":"showrunner","name":"boss","parent":null,"current_session":"boss"},
        {"id":"a-review","kind":"worker","name":"fixture-impl","parent":"boss","current_session":THREAD,"role":"review"},
        {"id":"b-test","kind":"worker","name":"Claude worker","parent":"boss","current_session":"unit-live","role":"test"},
        {"id":"c-unknown","kind":"worker","name":"unlisted","parent":"boss","current_session":"unlisted"},
        {"id":"d-unit","kind":"unit","name":"unit","parent":"boss","current_session":"nonworker","role":"fix"}
    ]})).expect("staff role document")).expect("checked staff roles");
    let edges = roster.edges;
    assert_eq!(
        edges.0.iter().map(|edge| edge.role).collect::<Vec<_>>(),
        [
            HierarchyRole::Review,
            HierarchyRole::Test,
            HierarchyRole::Worker,
            HierarchyRole::Unit
        ],
        "staff roles apply to worker rows while unit rows retain their kind"
    );
}

#[test]
fn unknown_staff_kinds_skip_field_validation_and_preserve_known_rows() {
    let mut value: Value = serde_json::from_slice(&document()).expect("staff document");
    value["rows"]
        .as_array_mut()
        .expect("rows")
        .push(serde_json::json!({"kind":"future"}));
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("staff.json");
    fs::write(&path, serde_json::to_vec(&value).expect("JSON")).expect("staff file");
    let roster = StaffFileReader::default()
        .roster(&HierarchySource::File(path.to_string_lossy().into_owned()));
    assert_eq!(
        roster.names,
        StaffRoster::parse(&document()).expect("known rows").names
    );
    assert_eq!(
        roster.edges,
        StaffRoster::parse(&document()).expect("known rows").edges
    );
}

fn assert_rejected_staff(mut change: impl FnMut(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&document()).expect("staff document");
    change(&mut value);
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("staff.json");
    fs::write(&path, serde_json::to_vec(&value).expect("JSON")).expect("staff file");
    let reader = StaffFileReader::default();
    let source = HierarchySource::File(path.to_string_lossy().into_owned());
    assert_eq!(reader.roster(&source).names, BTreeMap::new());
    assert_eq!(reader.roster(&source).edges, HierarchyEdges::default());
    assert!(
        matches!(reader.publication(&source), StaffPublication::Ready(HierarchySnapshot {
        edges: HierarchyEdges(edges), notice: HierarchyNotice::Clear
    }) if edges.is_empty())
    );
}

#[test]
fn a_worker_cannot_publish_as_a_staff_root() {
    assert_rejected_staff(|value| value["rows"][2]["parent"] = Value::Null);
}

#[test]
fn a_unit_cannot_publish_below_a_worker() {
    assert_rejected_staff(|value| {
        value["rows"][5]["parent"] = "boss".into();
        value["rows"][1]["parent"] = "dead".into();
    });
}

#[test]
fn two_staff_ids_cannot_publish_the_same_current_session() {
    assert_rejected_staff(|value| value["rows"][2]["current_session"] = "unit-live".into());
}

fn set_staff_mtime(path: &Path, time: SystemTime) {
    File::open(path)
        .expect("staff path")
        .set_times(FileTimes::new().set_modified(time))
        .expect("staff mtime");
}

#[test]
fn staff_read_failure_recovers_without_an_mtime_change() {
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("staff.json");
    fs::create_dir(&path).expect("unreadable staff source");
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    set_staff_mtime(&path, time);
    let source = HierarchySource::File(path.to_string_lossy().into_owned());
    let reader = StaffFileReader::default();
    assert_eq!(reader.roster(&source).names, BTreeMap::new());
    fs::remove_dir(&path).expect("remove unreadable source");
    fs::write(&path, document()).expect("restore staff file");
    set_staff_mtime(&path, time);
    assert_eq!(
        reader.roster(&source).names,
        StaffRoster::parse(&document()).expect("valid staff").names
    );
}

#[test]
fn staff_read_failure_clears_names_without_an_mtime_change() {
    let directory = tempfile::tempdir().expect("staff directory");
    let path = directory.path().join("staff.json");
    fs::write(&path, document()).expect("staff file");
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
    set_staff_mtime(&path, time);
    let source = HierarchySource::File(path.to_string_lossy().into_owned());
    let reader = StaffFileReader::default();
    assert_eq!(
        reader.roster(&source).names,
        StaffRoster::parse(&document()).expect("valid staff").names
    );
    fs::remove_file(&path).expect("remove staff file");
    fs::create_dir(&path).expect("replace with unreadable source");
    set_staff_mtime(&path, time);
    assert_eq!(reader.roster(&source).names, BTreeMap::new());
    assert_eq!(reader.roster(&source).edges, HierarchyEdges::default());
}
