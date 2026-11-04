//! Staff file parsing, caching and live name joins.

#![expect(clippy::expect_used, reason = "fixture files and rows must exist")]

use std::collections::HashMap;
use std::fs::File;
use std::fs::FileTimes;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use serde_json::Value;

use super::*;
use crate::census::Agent;
use crate::census::RoleFallback;
use crate::census::StaffRowNames;
use crate::census::classify;
use crate::census::classify::CodexSessionLink;
use crate::census::codex::CodexThread;
use crate::census::codex_names::SessionName;
use crate::census::worker_dispatch::ParsedDispatch;
use crate::census::worker_dispatch::WorkerRoster;

const THREAD: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9d7a";

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
    classify::attach_staff_names(&mut rows, &roster, &HashMap::new());
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
    rows[1].session_id = SessionIdentity::Unrecorded;
    let thread = CodexThread {
        account_home: Path::new("/fixture/.codex").into(),
        cwd:          "/fixture".into(),
        created_ms:   1,
        name:         SessionName::Named("thread name".into()),
        first_prompt: String::new(),
        rollout:      format!("/fixture/rollout-{THREAD}.jsonl").into(),
    };
    let links = HashMap::from([(2, CodexSessionLink::Named(thread))]);
    classify::attach_staff_names(&mut rows, &roster, &links);
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["staff boss", "staff worker"]
    );
    assert_eq!(rows.len(), 2, "dead staff rows must not create census rows");
    rows[1].name = "herdr name".into();
    classify::attach_staff_names(
        &mut rows,
        &roster,
        &HashMap::from([(2, CodexSessionLink::HerdrName("herdr name".into()))]),
    );
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
        classify::attach_staff_names(&mut rows, &staff, &HashMap::new());
        assert_eq!(rows[0].name, "staff boss");
        assert_eq!(
            rows[1].name, fallback,
            "a missing staff name must preserve the fallback"
        );
    }
}

#[test]
fn staff_worker_roles_use_the_existing_roster_dispatch() {
    let directory = tempfile::tempdir().expect("mesh directory");
    fs::write(
        directory.path().join("mesh_roster.json"),
        serde_json::json!({"fixture-impl":{
            "thread_id": THREAD, "launcher_pid":701, "role":"review"
        }})
        .to_string(),
    )
    .expect("mesh roster");
    let command = format!(
        "bash ~/.claude/scripts/delegate/implement.sh --to fixture-impl '{}' /fixture /fixture/prompt.md impl 'inspect code' impl 'implementation' 0 impl",
        directory.path().display()
    );
    let parsed = ParsedDispatch::parse(&command).expect("dispatch command");
    let roster = WorkerRoster::read(&parsed.session_dir);
    let mut owner: AgentRow =
        serde_json::from_str(include_str!("fixtures/restarted_unit_codex.json"))
            .expect("worker carrier");
    owner.children[0].dispatch = parsed.resolve(&roster, 701);
    let mut claude = row(2, "unit-live", "Claude worker");
    claude.role_fallback = RoleFallback::Worker(HierarchyRole::Test);
    let edges = HierarchyEdges(vec![
        HierarchyEdge {
            manager: "boss".into(),
            member:  THREAD.into(),
            role:    HierarchyRole::Worker,
        },
        HierarchyEdge {
            manager: "boss".into(),
            member:  "unit-live".into(),
            role:    HierarchyRole::Worker,
        },
        HierarchyEdge {
            manager: "boss".into(),
            member:  "unlisted".into(),
            role:    HierarchyRole::Worker,
        },
        HierarchyEdge {
            manager: "boss".into(),
            member:  THREAD.into(),
            role:    HierarchyRole::Unit,
        },
    ])
    .with_worker_roles(&[owner, claude]);
    assert_eq!(
        edges.0.iter().map(|edge| edge.role).collect::<Vec<_>>(),
        [
            HierarchyRole::Review,
            HierarchyRole::Test,
            HierarchyRole::Worker,
            HierarchyRole::Unit
        ],
        "the roster role must replace only a generic worker kind"
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
