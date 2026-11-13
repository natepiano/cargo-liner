//! Review tasks remain readable through the probe and in visible agent rows.

#![expect(
    clippy::expect_used,
    reason = "fixture input and rendered rows must exist"
)]
#![expect(clippy::panic, reason = "a fixture probe must produce an answer")]

use std::collections::HashMap;
use std::fs;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use worker_dispatch::JobText;
use worker_dispatch::WorkerTask;

use super::accounts::EMPTY_SESSION_STATES;
use super::tier::RolloutStart;
use super::tier::Rollouts;
use super::*;
use crate::agent_cell::AgentTile;
use crate::agent_cell::{self};
use crate::footer_colors::FooterColors;

const FIRST_TASK: &str = "Review the first worker change";
const SECOND_TASK: &str = "Review the second worker change";
const START: u64 = 1_792_180_800;

/// These variants are the worker-task contract before pool reviewers existed.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyWorkerTask {
    #[default]
    Unrecorded,
    RoleOnly,
    Task(String),
}

#[derive(serde::Deserialize)]
struct LegacyChild {
    name:        String,
    #[serde(default)]
    worker_task: LegacyWorkerTask,
}

#[derive(serde::Deserialize)]
struct LegacyRow {
    children: Vec<LegacyChild>,
}

#[derive(serde::Deserialize)]
struct LegacyReport {
    rows: Vec<LegacyRow>,
}

fn leased_rows() -> Vec<AgentRow> {
    let mut rows = rows();
    let mut reviewer = rows[2].children.remove(0);
    reviewer.worker_task = WorkerTask::Reviewer {
        name: "code3".into(),
        job:  JobText::Recorded(SECOND_TASK.into()),
    };
    // A carrier can already nest the live thread under a visible agent.
    rows.truncate(2);
    rows[1].children.push(reviewer);
    rows
}

fn reviewer_role(text: &str) -> String {
    let header = text
        .lines()
        .take_while(|line| !line.contains("code3"))
        .filter(|line| line.contains("session") && line.contains("status"))
        .last()
        .expect("the reviewer owner's tree header");
    let column = header[..header.find("activity").expect("activity column")]
        .chars()
        .count();
    text.lines()
        .find(|line| line.contains("code3"))
        .expect("reviewer row")
        .chars()
        .skip(column)
        .take_while(|character| !character.is_whitespace())
        .collect()
}

#[test]
fn reviewer_lease_draws_review_under_a_visible_agent_without_a_hierarchy_edge() {
    let text = drawn(leased_rows(), HierarchyEdges::default().into());
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
    assert_eq!(reviewer_role(&text), "reviewer", "{text}");
}

#[test]
fn reviewer_schema_three_task_remains_readable_by_the_previous_receiver() {
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema":3,"machine":"remote","rows":leased_rows(),"edges":[],
    }))
    .expect("current probe row encoding");
    let legacy: LegacyReport = serde_json::from_slice(&payload)
        .expect("the previous receiver accepts the current worker task");
    let child = &legacy.rows[1].children[0];
    assert_eq!(child.name, "old raw reviewer");
    match &child.worker_task {
        LegacyWorkerTask::Task(job) => assert_ne!(job, ""),
        LegacyWorkerTask::Unrecorded | LegacyWorkerTask::RoleOnly => {},
    }
    let MachineState::Answered(rows, snapshot) =
        RemoteHierarchyAnswers::default().apply("remote", probe::parse_answer(&payload))
    else {
        panic!("current receiver accepts the probe");
    };
    let text = drawn(rows, snapshot);
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
}

fn rows() -> Vec<AgentRow> {
    serde_json::from_value(serde_json::json!([
        {"agent":"claude","session_id":"fixture-showrunner","name":"production","pid":10,"started":100,"directory":"/work/fixture","children":[]},
        {"agent":"claude","session_id":"fixture-unit","name":"handler-unit","pid":20,"started":101,"directory":"/work/fixture","children":[]},
        {"agent":"codex_thread_host","name":"app-server","pid":30,"started":102,"directory":"/work/fixture","children":[
            {"kind":"thread","depth":0,"pid":null,"thread_id":"review-thread","name":"old raw reviewer","started":103}
        ]}
    ])).expect("review host and unit rows")
}

fn drawn(rows: Vec<AgentRow>, hierarchy: HierarchySnapshot) -> String {
    let state = MachineState::Answered(rows, hierarchy);
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    agent_cell::group_tiles(&placement, &cells, &HashMap::new(), START + 1000)
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
                    START + 1000,
                    AccountColumn::Hidden,
                    &FooterColors::default(),
                ),
                AgentTile::Unassigned(entry) => agent_cell::draw_unassigned(
                    &mut buffer,
                    area,
                    entry,
                    START + 1000,
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
        .collect::<Vec<_>>()
        .join("\n")
}

fn task(text: &str, second: u32) -> String {
    format!(
        "{}\n",
        serde_json::json!({"timestamp":format!("2026-10-16T20:00:{second:02}Z"),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
    )
}

fn head() -> String {
    format!(
        "{}\n{}",
        serde_json::json!({"type":"session_meta","payload":{"timestamp":"2026-10-16T20:00:00Z","cwd":"/work/fixture"}}),
        task(FIRST_TASK, 1)
    )
}

fn activity() -> &'static str {
    "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"service_tier\":\"default\"}}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"rg reviewer src\\\"}\"}}\n"
}

#[test]
fn reviewer_rollout_start_records_session_time_and_keeps_missing_time_unavailable() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let recorded = directory.path().join("recorded.jsonl");
    fs::write(&recorded, format!("{}{}", head(), activity())).expect("recorded start");
    let missing = directory.path().join("missing.jsonl");
    fs::write(&missing, task(SECOND_TASK, 20)).expect("no session metadata");
    let mut rollouts = Rollouts::default();
    assert_eq!(
        rollouts.read(&recorded).began,
        RolloutStart::Recorded(START)
    );
    assert_eq!(rollouts.read(&missing).began, RolloutStart::Unavailable);
}
