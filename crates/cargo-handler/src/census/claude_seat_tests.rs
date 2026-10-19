//! Plan reviewers and Claude dispatches use the grouped rows that users see.

#![expect(clippy::expect_used, reason = "fixture rows and files must exist")]

use std::collections::HashMap;
use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tempfile::TempDir;
use tree::TreeSources;
use worker_dispatch::WorkerDispatch;

use super::account::AccountLabels;
use super::account::ConfigHome;
use super::classify::ProcessEntry;
use super::classify::SessionRecord;
use super::*;
use crate::agent_cell::AgentTile;
use crate::agent_cell::{self};
use crate::footer_colors::FooterColors;

fn process(pid: u32, parent: u32, arguments: &[&str]) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: arguments[0].into(),
        arguments: arguments.iter().map(|word| (*word).into()).collect(),
        started: 100,
        directory: Some("/work/fixture".into()),
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

fn grouped(rows: Vec<AgentRow>, edges: HierarchyEdges) -> Vec<String> {
    let state = MachineState::Answered(rows, edges.into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    agent_cell::group_tiles(&placement, &cells, &HashMap::new(), 1000)
        .iter()
        .map(|tile| {
            let area = Rect::new(0, 0, 240, 20);
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
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

fn classified(processes: &[ProcessEntry], sessions: &[SessionRecord]) -> Vec<AgentRow> {
    let mut rows = classify::agent_rows(processes, sessions, &[], None, &AccountLabels::default());
    tree::attach_children(
        &mut rows,
        processes,
        &TreeSources {
            sessions,
            ..tree::TreeSources::default()
        },
    );
    rows
}

#[test]
fn reviewer_plan_process_without_session_record_draws_review_in_the_unassigned_tile() {
    for arguments in [
        vec!["claude", "--print", "--permission-mode", "plan"],
        vec!["claude", "-p", "--permission-mode=plan"],
    ] {
        let rows = classified(&[process(20, 1, &arguments)], &[]);
        assert_eq!(rows.len(), 1, "an anonymous reviewer reaches the census");
        let state = MachineState::Answered(rows.clone(), HierarchyEdges::default().into());
        let machines = [Machine {
            name:  "fixture",
            state: &state,
        }];
        let placement = AgentPlacement::from_machines(&machines);
        let cells = agent_cell::cell_order_placed(&placement);
        let grouped_tiles = agent_cell::group_tiles(&placement, &cells, &HashMap::new(), 1000);
        assert!(matches!(
            grouped_tiles.as_slice(),
            [AgentTile::Unassigned(_)]
        ));
        let tiles = grouped(rows, HierarchyEdges::default());
        assert_eq!(tiles.len(), 1);
        let text = &tiles[0];
        let header = text
            .lines()
            .position(|line| {
                line.contains("session") && line.contains("role") && line.contains("status")
            })
            .expect("tree column header");
        let role_column = text
            .lines()
            .nth(header)
            .expect("header")
            .find("role")
            .expect("role column");
        let row = text
            .lines()
            .nth(header + 1)
            .expect("anonymous reviewer row");
        let role: String = row
            .chars()
            .skip(role_column)
            .take_while(|character| !character.is_whitespace())
            .collect();
        assert_eq!(role, "review", "{text}");
    }
}

#[test]
fn reviewer_plan_process_under_an_agent_is_visible_once_and_hierarchy_role_wins() {
    let processes = [
        process(10, 1, &["claude"]),
        process(20, 10, &["claude", "--print", "--permission-mode", "plan"]),
    ];
    let sessions: Vec<SessionRecord> = serde_json::from_value(serde_json::json!([
        {"pid":10,"sessionId":"fixture-root","name":"director"},
        {"pid":20,"sessionId":"fixture-reviewer","name":"plan-reviewer"}
    ]))
    .expect("session records");
    let rows = classified(&processes, &sessions);
    let edges: HierarchyEdges = serde_json::from_value(serde_json::json!([
        {"manager":"fixture-root","member":"fixture-reviewer","role":"test"}
    ]))
    .expect("hierarchy role");
    let text = grouped(rows, edges).join("\n");
    assert_eq!(text.matches("plan-reviewer").count(), 1, "{text}");
    let line = text
        .lines()
        .find(|line| line.contains("plan-reviewer"))
        .expect("reviewer row");
    assert!(line.split_whitespace().any(|word| word == "test"), "{text}");
    assert!(
        !line.split_whitespace().any(|word| word == "review"),
        "{text}"
    );
}

fn seat_session() -> TempDir {
    let session = tempfile::tempdir().expect("seat directory");
    std::fs::write(
        session.path().join("impl_worker_impl.json"),
        include_str!("fixtures/claude_seat_worker.json"),
    )
    .expect("worker identity");
    std::fs::write(
        session.path().join("impl_claim_impl.json"),
        include_str!("fixtures/claude_seat_claim.json"),
    )
    .expect("live claim");
    session
}

fn dispatch_processes(directory: &Path) -> Vec<ProcessEntry> {
    let path = directory.to_str().expect("fixture path");
    vec![process(
        701,
        600,
        &[
            "bash",
            "/fixture/.claude/scripts/delegate/implement.sh",
            "--phase",
            "8",
            "--to",
            "fixture-impl",
            path,
            "/work/fixture",
            "/fixture/prompt.md",
            "impl",
            "Repair the Claude seat",
            "impl",
            "working",
            "0",
            "impl",
        ],
    )]
}

fn seat_rows(dispatch: WorkerDispatch) -> Vec<AgentRow> {
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([
        {"agent":"claude","session_id":"fixture-root","name":"director","pid":10,"started":100,"directory":"/work/fixture","children":[
            {"kind":"shell","depth":0,"pid":701,"name":"Claude dispatch shell","started":200},
            {"kind":"thread","depth":0,"pid":null,"thread_id":"shared-worker-id","name":"separate Codex worker","started":200}
        ]},
        {"agent":"claude","session_id":"shared-worker-id","name":"original Claude title","pid":30,"started":200,"directory":"/work/fixture","launched_by":10,"children":[]}
    ])).expect("worker rows");
    rows[0].children[0].dispatch = dispatch;
    rows[1].role_fallback = RoleFallback::Review;
    rows
}

#[test]
fn reviewer_claude_seat_folds_into_its_session_without_matching_a_codex_thread_with_the_same_id() {
    let session = seat_session();
    let processes = dispatch_processes(session.path());
    let dispatches = scan::find_worker_dispatches(&processes);
    let dispatch = dispatches
        .get(&701)
        .expect("the live Claude claim joins its dispatch");
    let text = grouped(seat_rows(dispatch.clone()), HierarchyEdges::default()).join("\n");
    assert!(text.contains("impl · Repair the Claude seat"), "{text}");
    let worker = text
        .lines()
        .find(|line| line.contains("impl · Repair the Claude seat"))
        .expect("folded seat row");
    assert!(
        worker.split_whitespace().any(|word| word == "impl"),
        "{text}"
    );
    assert!(
        !worker.split_whitespace().any(|word| word == "review"),
        "a seat role wins over the plan fallback: {text}"
    );
    assert!(!text.contains("Claude dispatch shell"), "{text}");
    assert!(!text.contains("original Claude title"), "{text}");
    assert!(
        text.contains("separate Codex worker"),
        "the family boundary keeps the Codex row: {text}"
    );
}

#[test]
fn reviewer_claude_identity_without_a_live_matching_claim_leaves_the_dispatch_visible() {
    let session = seat_session();
    let processes = dispatch_processes(session.path());
    for claim in [
        serde_json::json!({"pid":999,"worker":"fixture-impl","turn_id":"old-turn"}),
        serde_json::json!({"pid":701,"worker":"another-seat","turn_id":"other-turn"}),
    ] {
        std::fs::write(
            session.path().join("impl_claim_impl.json"),
            claim.to_string(),
        )
        .expect("stale claim");
        let dispatches = scan::find_worker_dispatches(&processes);
        let dispatch = dispatches.get(&701).cloned().unwrap_or_default();
        let text = grouped(seat_rows(dispatch), HierarchyEdges::default()).join("\n");
        assert!(text.contains("Claude dispatch shell"), "{text}");
        assert!(text.contains("original Claude title"), "{text}");
        assert!(!text.contains("impl · Repair the Claude seat"), "{text}");
    }
}

#[test]
fn reviewer_claude_seat_review_pass_overrides_its_impl_slot_role() {
    let session = seat_session();
    let mut processes = dispatch_processes(session.path());
    // The seat stays impl. Only the dispatch pass changes to review.
    let pass = processes[0].arguments.len() - 4;
    processes[0].arguments[pass] = "review".into();
    let dispatches = scan::find_worker_dispatches(&processes);
    let dispatch = dispatches.get(&701).expect("live Claude review pass");
    let mut rows = seat_rows(dispatch.clone());
    rows[1].role_fallback = RoleFallback::None;
    let text = grouped(rows, HierarchyEdges::default()).join("\n");
    let column = text
        .lines()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("tree header")
        .find("role")
        .expect("role column");
    let worker = text
        .lines()
        .find(|line| line.contains("impl · Repair the Claude seat"))
        .expect("folded impl seat");
    let role: String = worker
        .chars()
        .skip(column)
        .take_while(|character| !character.is_whitespace())
        .collect();
    assert_eq!(role, "review", "{text}");
    assert!(!text.contains("Claude dispatch shell"), "{text}");
}
