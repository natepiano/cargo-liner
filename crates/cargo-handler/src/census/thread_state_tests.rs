//! Turn events, rather than the thread's age or tier, determine its activity.

#![expect(
    clippy::expect_used,
    reason = "temporary rollout fixtures must be readable"
)]

use std::fs;
use std::io::Write;
use std::path::Path;

use tempfile::TempDir;

use super::Activity;
use super::AgentActivity;
use super::ChildRow;
use super::tier::Rollouts;

const META: &str = r#"{"type":"session_meta","payload":{"timestamp":"2026-10-16T13:40:00Z"}}"#;
const START: &str = r#"{"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1"}}"#;
const COMPLETE: &str =
    r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#;
const ABORT: &str = r#"{"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"turn-1"}}"#;

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("fixture rollout opens");
    writeln!(file, "{text}").expect("rollout line writes");
}

#[test]
fn the_last_turn_event_determines_whether_a_thread_is_busy_or_idle() {
    let directory = TempDir::new().expect("temporary rollout directory");
    for (name, events, expected) in [
        (
            "started",
            vec![COMPLETE, START],
            AgentActivity::Active(Activity::Thinking),
        ),
        ("completed", vec![START, COMPLETE], AgentActivity::Idle),
        ("aborted", vec![START, ABORT], AgentActivity::Idle),
        ("no-turn", Vec::new(), AgentActivity::Idle),
    ] {
        let path = directory.path().join(format!("{name}.jsonl"));
        append(&path, META);
        for event in events {
            append(&path, event);
        }
        // A later non-event mentioning `task_started` must not change the activity returned by
        // `Rollouts::read`.
        append(
            &path,
            r#"{"type":"response_item","payload":{"type":"task_started"}}"#,
        );
        append(
            &path,
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"service_tier":"priority"}}}"#,
        );
        assert_eq!(
            Rollouts::default().read(&path).activity(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn an_incremental_completion_changes_busy_to_idle_only_after_its_line_is_complete() {
    let directory = TempDir::new().expect("temporary rollout directory");
    let path = directory.path().join("worker.jsonl");
    append(&path, META);
    append(&path, START);
    let mut rollouts = Rollouts::default();
    assert_eq!(
        rollouts.read(&path).activity(),
        AgentActivity::Active(crate::census::Activity::Thinking)
    );
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("rollout opens");
    write!(file, "{COMPLETE}").expect("partial completion writes");
    assert_eq!(
        rollouts.read(&path).activity(),
        AgentActivity::Active(crate::census::Activity::Thinking)
    );
    writeln!(file).expect("completion line ends");
    assert_eq!(rollouts.read(&path).activity(), AgentActivity::Idle);
    append(&path, START);
    assert_eq!(
        rollouts.read(&path).activity(),
        AgentActivity::Active(crate::census::Activity::Thinking)
    );
    // A truncated/replaced rollout must make `rollouts.read` discard the old busy event.
    fs::write(&path, format!("{META}\n")).expect("rollout resets");
    assert_eq!(rollouts.read(&path).activity(), AgentActivity::Idle);
}

#[test]
fn probe_threads_preserve_activity_and_older_probes_default_to_idle() {
    let mut thread: ChildRow = serde_json::from_value(serde_json::json!({
        "kind":"thread", "depth":0, "pid":null, "name":"worker",
        "started":900, "timer":null, "thread_id":"worker-thread"
    }))
    .expect("older thread probe parses");
    assert_eq!(thread.activity, AgentActivity::Idle);
    thread.activity = AgentActivity::Active(Activity::Thinking);
    let encoded = serde_json::to_string(&thread).expect("busy thread serializes");
    let decoded: ChildRow = serde_json::from_str(&encoded).expect("remote thread parses");
    assert_eq!(
        decoded.activity,
        AgentActivity::Active(crate::census::Activity::Thinking)
    );
    assert_eq!(decoded.thread_id, thread.thread_id);
}
