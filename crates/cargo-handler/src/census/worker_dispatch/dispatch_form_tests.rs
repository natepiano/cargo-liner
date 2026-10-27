//! Every launcher form folds in the grouped tile actually drawn by the app.

#![expect(
    clippy::expect_used,
    reason = "fixture launchers, rosters and rows must parse"
)]
#![expect(
    clippy::panic,
    reason = "a missing grouped fixture row is a test failure"
)]

use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::Value;
use tempfile::TempDir;

use super::*;
use crate::agent_cell::AgentTile;
use crate::agent_cell::{self};
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::HierarchyEdges;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::account::ConfigHome;
use crate::census::classify::ProcessEntry;
use crate::census::scan;
use crate::footer_colors::FooterColors;

const THREAD: &str = "01a14488-1234-5678-9abc-0123456789ab";

fn process(pid: u32, parent: u32, arguments: Vec<String>) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: arguments[0].clone(),
        arguments,
        started: 800,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

fn fixture(root: &str) -> TempDir {
    tempfile::Builder::new()
        .prefix(&format!("handler-forms-{root}-"))
        .tempdir()
        .expect("isolated session")
}

fn processes(
    directory: &Path,
    seat: &str,
    fresh: bool,
    phase: &str,
    job: &str,
) -> Vec<ProcessEntry> {
    let mut argv: Vec<String> = [
        "bash",
        "/home/fixture/.claude/scripts/delegate/implement.sh",
        "--phase",
        phase,
    ]
    .map(String::from)
    .into();
    if !fresh {
        argv.extend(["--to".to_string(), seat.to_string()]);
    }
    argv.extend([
        directory.display().to_string(),
        "/work/fixture".to_string(),
        "/fixture/prompt.md".to_string(),
        "impl".to_string(),
        job.to_string(),
        "impl".to_string(),
        "working".to_string(),
        "0".to_string(),
        "impl".to_string(),
    ]);
    let literal = argv
        .iter()
        .map(|word| format!("'{}'", word.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    let wrapped = format!(
        "S={}; PLAN_DELEGATE_RESOLVES_ROUND=1 source /fixture/shell-snapshots/snapshot-test.sh && eval '{}'",
        directory.display(),
        literal.replace('\'', "'\\''")
    );
    vec![
        process(600, 10, vec!["zsh".into(), "-c".into(), wrapped]),
        process(701, 600, argv),
        process(
            702,
            701,
            vec![
                "python3".into(),
                "/fixture/codex_mesh.py".into(),
                if fresh {
                    "start".into()
                } else {
                    "follow".into()
                },
                "--session-dir".into(),
                directory.display().to_string(),
                if fresh {
                    "--name".into()
                } else {
                    "--to".into()
                },
                seat.into(),
            ],
        ),
    ]
}

fn roster(directory: &Path, seat: &str, launcher: &Value) {
    fs::write(
        directory.join(ROSTER_FILE),
        serde_json::json!({seat:{"thread_id":THREAD,"launcher_pid":launcher,"role":"impl"}})
            .to_string(),
    )
    .expect("roster writes");
}

fn grouped_text(dispatch: &WorkerDispatch) -> String {
    let rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"unassigned-root","status":"busy","pid":10,"started":700,
        "directory":"/work/fixture","desktop":null,"branch":null,"launched_by":null,
        "children":[
            {"kind":"shell","depth":0,"pid":600,"name":"Dispatch fixture seat","started":800,"timer":null,"dispatch":dispatch},
            {"kind":"thread","depth":0,"pid":null,"thread_id":THREAD,"name":THREAD,"started":900,"timer":null}
        ]
    }])).expect("probe-compatible grouped rows");
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let agents = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &agents, &HashMap::new(), 1000);
    let AgentTile::Agent(entry) = &tiles[0] else {
        panic!("a root with dispatched work must own a hierarchy tile")
    };
    let area = Rect::new(
        0,
        0,
        240,
        u16::try_from(agent_cell::height(entry, 240, 1000, AccountColumn::Hidden))
            .expect("small fixture"),
    );
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

fn assert_form(root: &str, seat: &str, fresh: bool, phase: &str, job: &str) {
    let directory = fixture(root);
    // For `launcher`, fresh rosters record `codex_mesh.py` start; follow rosters record
    // `implement.sh`. The live two-accounts roster has no launcher pid at all.
    let launcher = if seat == "fixture-two-accounts-test" {
        Value::Null
    } else {
        serde_json::json!(if fresh { 702 } else { 701 })
    };
    roster(directory.path(), seat, &launcher);
    let found = scan::find_worker_dispatches(&processes(directory.path(), seat, fresh, phase, job));
    let dispatch = found.get(&600).expect("wrapper row resolves");
    let text = grouped_text(dispatch);
    let short = seat.strip_prefix("fixture-").unwrap_or(seat);
    assert_eq!(
        text.matches(&format!("{short} · {job}")).count(),
        1,
        "{text}"
    );
    assert!(
        !text.contains("Dispatch fixture seat"),
        "launcher must fold: {text}"
    );
    assert!(!text.contains(THREAD), "raw identity must not draw: {text}");
}

#[test]
fn dispatch_form_fresh_trunk_portal_start_names_the_selected_new_seat() {
    assert_form(
        "unit",
        "fixture-portal",
        true,
        "4",
        "Dispatch the portal fix to a fresh seat",
    );
}

#[test]
fn dispatch_form_fresh_widget_merge_start_folds_into_its_worker() {
    assert_form(
        "delegate",
        "fixture-merge",
        true,
        "4",
        "Resolve the widget merge",
    );
}

#[test]
fn dispatch_form_fresh_unit_board_start_folds_into_its_worker() {
    assert_form(
        "unit",
        "fixture-unit-board-impl",
        true,
        "1",
        "Implement unit-board phase 1",
    );
}

#[test]
fn dispatch_form_berth_parallel_follow_folds_both_numbered_seats() {
    for (seat, phase) in [("fixture-impl", "9"), ("fixture-impl2", "10")] {
        assert_form("unit", seat, false, phase, "Implement berth parallel phase");
    }
}

#[test]
fn dispatch_form_style_follow_folds_and_names_the_worker() {
    assert_form(
        "unit",
        "fixture-style-test",
        false,
        "4",
        "Apply the style repair",
    );
}

#[test]
fn dispatch_form_delegate_session_follow_folds_and_keeps_the_job() {
    assert_form(
        "delegate",
        "fixture-two-accounts-test",
        false,
        "4",
        "Verify the two accounts",
    );
}

#[test]
fn dispatch_form_roster_written_after_launcher_start_is_seen_on_the_next_scan() {
    let directory = fixture("unit");
    let seat = "fixture-impl";
    let processes = processes(
        directory.path(),
        seat,
        true,
        "4",
        "Implement the late roster",
    );
    let before = scan::find_worker_dispatches(&processes);
    assert!(
        before
            .get(&600)
            .is_none_or(|dispatch| *dispatch == WorkerDispatch::Plain)
    );
    roster(directory.path(), seat, &serde_json::json!(702));
    let after = scan::find_worker_dispatches(&processes);
    let text = grouped_text(after.get(&600).expect("later scan resolves wrapper"));
    assert!(text.contains("impl · Implement the late roster"), "{text}");
    assert!(!text.contains("Dispatch fixture seat"), "{text}");
}
