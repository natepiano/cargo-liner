//! Subagent roles survive schema-3 probes without breaking older receivers.

#![expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use serde_json::Value;

use super::*;
use crate::census::WorkIdentity;
use crate::census::model::ModelName;
use crate::census::model::ReasoningEffort;
use crate::census::transcript::SubagentRole;

#[test]
fn probe_roles_round_trip_inside_subagent_kinds_and_read_legacy_rows() {
    let old = serde_json::json!({
        "depth":0,"kind":"subagent","subagent_role":{"Specialist":"showrunner"},
        "pid":null,"name":"Coordinate the release","started":42
    });
    let row: ChildRow = serde_json::from_value(old).expect("legacy probe row");
    assert_eq!(row.kind, ChildKind::Subagent(SubagentRole::Showrunner));
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
    assert_eq!(
        shell.kind,
        ChildKind::Shell(crate::census::ShellTimerObservation::Untimed)
    );
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
        model:        ModelName::Unrecorded,
        effort:       ReasoningEffort::Unrecorded,
        worker_task:  WorkerTask::Unrecorded,
        activity:     AgentActivity::Idle,
        dispatch:     WorkerDispatch::Plain,
        thread_id:    CodexThreadId::Unrecorded,
        depth:        2,
        kind:         ChildKind::Subagent(SubagentRole::Showrunner),
        service_tier: ServiceTier::Unrecorded,
        account:      Account::Unrecorded,
        pid:          None,
        name:         "Coordinate the release".to_owned(),
        started:      42,
        work_id:      WorkIdentity::Legacy,
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

/// The checkpoint receiver decodes this closed wire vocabulary.
#[derive(Debug, Deserialize, PartialEq)]
enum CheckpointSubagentRole {
    Capture,
    Review,
    Lint,
    Test,
    Survey,
    Specialist(String),
    Unrecorded,
}

/// The checkpoint row keeps its role field during deserialization.
#[derive(Debug, Deserialize)]
struct CheckpointChildRow {
    #[serde(flatten)]
    row:           PreviousChildRow,
    subagent_role: CheckpointSubagentRole,
}

#[test]
fn legacy_probe_role_names_decode_into_closed_role_words() {
    for (wire, expected) in [
        (serde_json::json!({"Specialist":"writer77"}), "writer"),
        (serde_json::json!({"Specialist":"p36-reviewer"}), "reviewer"),
        (serde_json::json!({"Specialist":"judge"}), "agent"),
        (serde_json::json!({"Specialist":"general-purpose"}), "agent"),
        (serde_json::json!({"Specialist":""}), "agent"),
        (serde_json::json!("Review"), "reviewer"),
    ] {
        let child: ChildRow = serde_json::from_value(serde_json::json!({
            "depth":0, "kind":"subagent", "subagent_role":wire,
            "pid":null, "name":"Legacy work", "started":42
        }))
        .expect("checkpoint report");
        let ChildKind::Subagent(role) = child.kind else {
            assert!(
                matches!(child.kind, ChildKind::Subagent(_)),
                "subagent kind remains"
            );
            return;
        };
        assert_eq!(role.text(), expected, "legacy role {wire}");
    }
}

#[test]
fn current_role_reports_keep_the_checkpoint_role_wire_vocabulary() {
    for (role, word) in [
        (SubagentRole::Writer, "writer"),
        (SubagentRole::Reviewer, "reviewer"),
        (SubagentRole::Impl, "impl"),
        (SubagentRole::Test, "test"),
        (SubagentRole::Fix, "fix"),
        (SubagentRole::Capture, "capture"),
        (SubagentRole::Lint, "lint"),
        (SubagentRole::Survey, "survey"),
        (SubagentRole::Plan, "plan"),
        (SubagentRole::Explore, "explore"),
        (SubagentRole::Showrunner, "showrunner"),
        (SubagentRole::Agent, "agent"),
    ] {
        let child = ChildRow {
            kind: ChildKind::Subagent(role),
            ..recorded_subagent()
        };
        let encoded = serde_json::to_value(child).expect("current report");
        let checkpoint: CheckpointChildRow =
            serde_json::from_value(encoded).expect("checkpoint receiver accepts the report");
        assert_eq!(checkpoint.row.kind, PreviousChildKind::Subagent);
        assert_eq!(checkpoint.row.name, "Coordinate the release");
        let restored = match checkpoint.subagent_role {
            CheckpointSubagentRole::Specialist(word) => word,
            CheckpointSubagentRole::Review => "reviewer".to_owned(),
            CheckpointSubagentRole::Capture => "capture".to_owned(),
            CheckpointSubagentRole::Lint => "lint".to_owned(),
            CheckpointSubagentRole::Test => "test".to_owned(),
            CheckpointSubagentRole::Survey => "survey".to_owned(),
            other @ CheckpointSubagentRole::Unrecorded => {
                assert!(
                    matches!(
                        other,
                        CheckpointSubagentRole::Specialist(_) | CheckpointSubagentRole::Review
                    ),
                    "{other:?}"
                );
                return;
            },
        };
        assert_eq!(restored, word, "new report uses a role word for {word}");
    }
}

#[test]
fn astra_probe_extension_preserves_the_legacy_role_vocabulary()
-> Result<(), Box<dyn std::error::Error>> {
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyRole;
    use crate::census::hierarchy::AstraMembership;
    use crate::census::probe;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum OldRole {
        Worker,
    }
    #[derive(serde::Deserialize)]
    struct OldEdge {
        role: OldRole,
    }
    #[derive(serde::Deserialize)]
    struct OldReport {
        edges: Vec<OldEdge>,
        rows:  Vec<OldRow>,
    }
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum OldFallback {
        None,
    }
    #[derive(serde::Deserialize)]
    struct OldRow {
        role_fallback: OldFallback,
    }
    for role in [
        HierarchyRole::AstraDesigner,
        HierarchyRole::AstraReviewer,
        HierarchyRole::Producer,
    ] {
        let member: AstraMembership = serde_json::from_value(serde_json::json!({
            "staff_id":"designer","session":"astra-thread","home":"/fixture/codex","account":"codex1","role":"astra-designer","work":"busy"
        }))?;
        let mut row: AgentRow = serde_json::from_value(
            serde_json::json!({"agent":"codex","pid":30,"name":"Astra","started":100,"directory":"/fixture","children":[],"codex_session":{"resume_argument":"astra-thread"}}),
        )?;
        row.session_id = row.session_id.with_astra(member);
        if role == HierarchyRole::Producer {
            row.role_fallback = RoleFallback::Producer;
        }
        let edges = HierarchyEdges(vec![HierarchyEdge {
            manager: "boss".into(),
            member: "astra-thread".into(),
            role,
        }]);
        let output = probe::encode_astra_test_report(vec![row.clone()], edges.clone())?;
        let old: OldReport = serde_json::from_slice(&output)?;
        assert!(matches!(old.edges[0].role, OldRole::Worker));
        assert!(matches!(old.rows[0].role_fallback, OldFallback::None));
        let (rows, restored) = probe::parse_report(&output)?;
        assert_eq!(rows, vec![row]);
        assert_eq!(restored, edges);
    }
    Ok(())
}

#[test]
fn astra_probe_rejects_incomplete_membership_and_keeps_unrelated_threads_plain()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value = serde_json::json!({"agent":"codex","pid":30,"name":"Astra","started":100,"directory":"/fixture","children":[],
        "codex_session":{"resume_argument":"thread-a"},"astra_member":{"staff_id":"worker","session":"thread-b","home":"/fixture/codex","account":"codex1","role":"astra-reviewer","work":"unknown"}});
    let plain: AgentRow = serde_json::from_value(value.clone())?;
    assert!(!plain.session_id.astra_membership().is_member());
    for bad in [Value::Null, serde_json::json!("relative/home")] {
        value["astra_member"]["home"] = bad;
        assert!(serde_json::from_value::<AgentRow>(value.clone()).is_err());
    }
    value["astra_member"]["home"] = serde_json::json!("/fixture/codex");
    value["astra_member"]["role"] = serde_json::json!("unannounced-role");
    assert!(serde_json::from_value::<AgentRow>(value).is_err());
    Ok(())
}
