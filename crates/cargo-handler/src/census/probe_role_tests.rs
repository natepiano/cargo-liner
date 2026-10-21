//! Subagent roles survive schema-3 probes without breaking older receivers.

#![expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use super::*;
use crate::census::transcript::SubagentRole;

#[test]
fn probe_roles_round_trip_inside_subagent_kinds_and_read_legacy_rows() {
    let old = serde_json::json!({
        "depth":0,"kind":"subagent","subagent_role":{"Specialist":"showrunner"},
        "pid":null,"name":"Coordinate the release","started":42
    });
    let row: ChildRow = serde_json::from_value(old).expect("legacy probe row");
    assert_eq!(
        row.kind,
        ChildKind::Subagent(SubagentRole::from_record("showrunner", ""))
    );
    let encoded = serde_json::to_value(&row).expect("current probe row");
    assert_eq!(encoded["kind"], "subagent");
    assert_eq!(
        encoded["subagent_role"],
        serde_json::json!({"Specialist":"showrunner"})
    );
    assert_eq!(
        serde_json::from_value::<ChildRow>(encoded).expect("schema-3 round trip"),
        row
    );

    let shell: ChildRow = serde_json::from_value(serde_json::json!({
        "depth":0,"kind":"shell","subagent_role":"Review",
        "pid":21,"name":"cargo test","started":42
    }))
    .expect("older non-subagent extension is ignored");
    assert_eq!(shell.kind, ChildKind::Shell);
    let encoded = serde_json::to_value(shell).expect("shell row");
    assert_eq!(encoded["kind"], "shell");
    assert!(encoded.get("subagent_role").is_none());
}

/// The schema-3 receiver's kind vocabulary, before roles became domain payloads.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum PreviousChildKind {
    Shell,
    Subagent,
    Session(Agent),
    UnderShell(Agent),
    Direct(Agent),
    Detached(Agent),
    Thread,
}

/// The previous receiver's required child fields; it ignores newer extensions.
#[derive(Debug, Deserialize)]
struct PreviousChildRow {
    depth:        u8,
    kind:         PreviousChildKind,
    #[serde(default)]
    service_tier: ServiceTier,
    pid:          Option<u32>,
    name:         String,
    started:      u64,
}

fn recorded_subagent() -> ChildRow {
    ChildRow {
        worker_task:  WorkerTask::Unrecorded,
        activity:     AgentActivity::Idle,
        dispatch:     WorkerDispatch::Plain,
        thread_id:    CodexThreadId::Unrecorded,
        depth:        2,
        kind:         ChildKind::Subagent(SubagentRole::from_record("showrunner", "")),
        service_tier: ServiceTier::Unrecorded,
        account:      Account::Unrecorded,
        pid:          None,
        name:         "Coordinate the release".to_owned(),
        started:      42,
        timer:        None,
    }
}

#[test]
fn recorded_subagent_reports_decode_in_schema_three_receivers() {
    let encoded = serde_json::to_vec(&recorded_subagent()).expect("new probe row");
    let previous: PreviousChildRow =
        serde_json::from_slice(&encoded).expect("schema-3 receiver accepts the report child");
    assert_eq!(previous.kind, PreviousChildKind::Subagent);
    assert_eq!(previous.depth, 2);
    assert_eq!(previous.service_tier, ServiceTier::Unrecorded);
    assert_eq!(previous.pid, None);
    assert_eq!(previous.name, "Coordinate the release");
    assert_eq!(previous.started, 42);
}

#[test]
fn schema_three_subagent_reports_preserve_roles_for_new_receivers() {
    let row = recorded_subagent();
    let encoded = serde_json::to_value(&row).expect("new probe row");
    assert_eq!(encoded["kind"], "subagent");
    assert_eq!(
        encoded["subagent_role"],
        serde_json::json!({"Specialist":"showrunner"})
    );
    assert_eq!(
        serde_json::from_value::<ChildRow>(encoded).expect("new receiver restores role"),
        row
    );
}

#[test]
fn schema_three_subagents_without_a_role_remain_unrecorded() {
    let old = serde_json::json!({
        "depth":0,"kind":"subagent","pid":null,"name":"Review","started":42
    });
    let row: ChildRow = serde_json::from_value(old).expect("old schema-3 row");
    assert_eq!(row.kind, ChildKind::Subagent(SubagentRole::Unrecorded));
    let encoded = serde_json::to_value(row).expect("forward old row");
    assert_eq!(encoded["kind"], "subagent");
    assert!(encoded.get("subagent_role").is_none());
}
