//! Context openings do not replace the worker's actual assignment.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and rendered labels must exist"
)]
use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::agent_cell;
use crate::agent_cell::AgentTile;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::HierarchySnapshot;
use crate::census::Machine;
use crate::census::MachineState;
use crate::footer_colors::FooterColors;

fn drawn_job(prompt: &str) -> String {
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude", "name":"director", "pid":10, "started":100, "directory":"/fixture",
        "children":[{"kind":"thread", "depth":0, "pid":null, "name":"worker", "started":200}]
    }]))
    .expect("worker rows");
    rows[0].children[0].worker_task = WorkerTask::from_prompt(prompt);
    let state = MachineState::Answered(rows, HierarchySnapshot::default());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let agents = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &agents, &HashMap::new(), 1000);
    let entry = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Agent(entry) => Some(entry),
            AgentTile::Unassigned(_) => None,
        })
        .expect("root with work has its own tile");
    let area = Rect::new(0, 0, 220, 12);
    let mut buffer = Buffer::empty(area);
    agent_cell::draw(
        &mut buffer,
        area,
        entry,
        &agents,
        1000,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_job(prompt: &str, expected: &str) {
    let drawn = drawn_job(prompt);
    assert!(drawn.contains(expected), "{drawn}");
    assert!(!drawn.contains("Worktree:"), "{drawn}");
    assert!(!drawn.contains("From organon"), "{drawn}");
    assert_eq!(
        JobText::parse(prompt, JobSource::DispatchJob).label("worker"),
        expected
    );
}

#[test]
fn worktree_context_yields_the_phase_title_in_worker_rows() {
    assert_job(
        "Worktree: /home/natepiano/worktrees/claude-build-followups\n\n# Phase 15 — repair role colors\nImplement the requested work.",
        "worker · Phase 15 — repair role colors",
    );
}

#[test]
fn director_context_yields_the_ticket_title_in_worker_rows() {
    assert_job(
        "From organon (unit director). You own this repair.\nWorktree: /home/natepiano/worktrees/hana\n\nTicket: repair offset rendering\nKeep every assertion.",
        "worker · Ticket: repair offset rendering",
    );
}

#[test]
fn context_without_a_job_keeps_the_role_only_label() {
    assert_job(
        "Worktree: /fixture\nFrom organon (unit director). You are the test worker.\nRun the checks and report.",
        "worker",
    );
    assert_eq!(
        WorkerTask::from_prompt("Worktree: /fixture\nFrom organon (unit director)."),
        WorkerTask::RoleOnly
    );
}
