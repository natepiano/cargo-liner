//! Activity survives older probes and follows complete transcript events.

#![expect(clippy::expect_used, reason = "fixture probes and files must parse")]

use super::account::ConfigHome;
use super::activity;
use super::activity::ActivityRead;
use super::activity::RunRecord;
use super::classify::ProcessEntry;
use super::*;
use crate::summary;

fn process(pid: u32, parent: u32, arguments: &[&str]) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: arguments[0].to_string(),
        arguments: arguments
            .iter()
            .map(|argument| (*argument).to_string())
            .collect(),
        started: 100,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

fn run(pid: u32, started_at: f64, status: &str, thread: &str) -> RunRecord {
    serde_json::from_value(serde_json::json!({
        "run_id":format!("run-{pid}"), "status":status,
        "session_dir":"/fixture/session", "worker":"fixture-test", "thread_id":thread,
        "label":"package tests", "command":["bash","verify.sh","test","cargo-handler"],
        "cwd":"/fixture", "log":"/fixture/run.log", "started_at":started_at,
        "pid":pid, "director_dir":"/fixture/director", "done_message_file":"/fixture/done"
    }))
    .expect("real mesh-run record shape parses")
}

#[test]
fn activity_codex_tool_calls_and_reasoning_follow_the_last_event() {
    let mut read = ActivityRead::default();
    read.take_codex(&serde_json::json!({"type":"event_msg","payload":{"type":"task_started"}}));
    assert_eq!(read.activity().text(), "thinking");
    for (tool, arguments, expected) in [
        (
            "apply_patch",
            serde_json::json!("*** Begin Patch\n*** End Patch"),
            "writing",
        ),
        (
            "exec_command",
            serde_json::json!({"cmd":"rg -n needle src"}),
            "reading",
        ),
        (
            "exec_command",
            serde_json::json!({"cmd":"git commit -m fixture"}),
            "git",
        ),
        (
            "exec_command",
            serde_json::json!({"cmd":"python3 script.py"}),
            "running",
        ),
    ] {
        read.take_codex(&serde_json::json!({"type":"response_item","payload":{
            "type":"function_call", "name":tool, "arguments":arguments.to_string()
        }}));
        assert_eq!(read.activity().text(), expected, "{tool}: {arguments}");
    }
    read.take_codex(
        &serde_json::json!({"type":"response_item","payload":{"type":"reasoning","summary":[]}}),
    );
    assert_eq!(read.activity().text(), "thinking");
}

#[test]
fn activity_codex_custom_tool_input_classifies_the_live_javascript_envelope() {
    let mut read = ActivityRead::default();
    for (name, input, expected) in [
        (
            "functions.exec",
            r#"const result = await tools.exec_command({"cmd":"rg -n needle src"}); text(result);"#,
            "reading",
        ),
        (
            "functions.exec",
            r#"text(await tools.exec_command({cmd: "git commit -m fixture"}));"#,
            "git",
        ),
        (
            "functions.exec",
            r#"text(await tools.apply_patch("*** Begin Patch\n*** End Patch"));"#,
            "writing",
        ),
        (
            "functions.apply_patch",
            "*** Begin Patch\n*** End Patch",
            "writing",
        ),
    ] {
        read.take_codex(&serde_json::json!({"type":"response_item","payload":{
            "type":"custom_tool_call","name":name,"input":input,"status":"completed"
        }}));
        assert_eq!(read.activity().text(), expected, "{name}: {input}");
    }
}

#[test]
fn activity_claude_tools_report_edits_reads_delegation_web_and_shell_work() {
    for (tool, input, expected) in [
        (
            "Edit",
            serde_json::json!({"file_path":"/fixture/a.rs"}),
            "writing",
        ),
        (
            "Write",
            serde_json::json!({"file_path":"/fixture/a.rs"}),
            "writing",
        ),
        (
            "NotebookEdit",
            serde_json::json!({"notebook_path":"/fixture/a.ipynb"}),
            "writing",
        ),
        (
            "Read",
            serde_json::json!({"file_path":"/fixture/a.rs"}),
            "reading",
        ),
        ("Grep", serde_json::json!({"pattern":"needle"}), "reading"),
        ("Glob", serde_json::json!({"pattern":"*.rs"}), "reading"),
        (
            "Agent",
            serde_json::json!({"prompt":"review"}),
            "delegating",
        ),
        ("Task", serde_json::json!({"prompt":"review"}), "delegating"),
        (
            "WebFetch",
            serde_json::json!({"url":"https://example.test"}),
            "web",
        ),
        ("WebSearch", serde_json::json!({"query":"fixture"}), "web"),
        (
            "Bash",
            serde_json::json!({"command":"git rebase main"}),
            "git",
        ),
        (
            "Bash",
            serde_json::json!({"command":"sed -n '1,20p' a.rs"}),
            "reading",
        ),
        (
            "Bash",
            serde_json::json!({"command":"python3 script.py"}),
            "running",
        ),
    ] {
        let mut read = ActivityRead::default();
        read.take_claude(
            &serde_json::json!({"type":"user","message":{"role":"user","content":"do the work"}}),
        );
        read.take_claude(&serde_json::json!({"type":"assistant","message":{
            "role":"assistant", "content":[{"type":"tool_use","id":"call-1","name":tool,"input":input}],
            "stop_reason":"tool_use"
        }}));
        assert_eq!(read.activity().text(), expected, "{tool}");
    }
}

#[test]
fn activity_closed_and_aborted_turns_reset_the_previous_tool_before_restart() {
    let mut read = ActivityRead::default();
    for end in ["task_complete", "turn_aborted"] {
        read.take_codex(&serde_json::json!({"type":"event_msg","payload":{"type":"task_started"}}));
        read.take_codex(&serde_json::json!({"type":"response_item","payload":{
            "type":"function_call", "name":"apply_patch", "arguments":"{}"
        }}));
        assert_eq!(read.activity().text(), "writing");
        read.take_codex(&serde_json::json!({"type":"event_msg","payload":{"type":end}}));
        assert_eq!(read.activity().text(), "idle", "{end}");
        read.take_codex(&serde_json::json!({"type":"event_msg","payload":{"type":"task_started"}}));
        assert_eq!(
            read.activity().text(),
            "thinking",
            "a new turn forgets the old tool"
        );
    }
}

#[test]
fn activity_shell_words_describe_the_command_instead_of_words_in_its_arguments() {
    for (command, expected) in [
        ("rg pattern src", "reading"),
        ("grep pattern input", "reading"),
        ("cat input", "reading"),
        ("sed -n '1,20p' input", "reading"),
        ("ls -la", "reading"),
        ("find src -name '*.rs'", "reading"),
        ("git commit -m fixture", "git"),
        ("git merge main", "git"),
        ("git rebase main", "git"),
        ("git push origin HEAD", "git"),
        ("printf 'cat'", "running"),
        ("echo git commit", "running"),
    ] {
        assert_eq!(
            activity::tool_activity("exec_command", &serde_json::json!({"cmd":command})).text(),
            expected,
            "{command}"
        );
    }
}

#[test]
fn activity_claude_completion_and_new_user_turn_clear_the_last_tool() {
    let mut read = ActivityRead::default();
    read.take_claude(&serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{}}],"stop_reason":"tool_use"}}));
    assert_eq!(read.activity().text(), "writing");
    read.take_claude(&serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"Done"}],"stop_reason":"end_turn"}}));
    assert_eq!(read.activity().text(), "idle");
    read.take_claude(&serde_json::json!({"type":"user","message":{"content":"Next task"}}));
    assert_eq!(read.activity().text(), "thinking");
}

#[test]
fn activity_verify_steps_come_from_the_runs_descendants_including_mac_and_waiting() {
    for step in [
        "fmt", "clippy", "nextest", "build", "check", "doc", "mend", "taplo",
    ] {
        let mut processes = vec![
            process(100, 1, &["python3", "mesh_runs.py"]),
            process(101, 100, &["bash", "verify.sh"]),
        ];
        let arguments = if step == "taplo" {
            vec!["taplo", "fmt"]
        } else {
            vec!["cargo", step]
        };
        processes.push(process(102, 101, &arguments));
        // A different worker's newer child must not affect this run.
        processes.push(process(201, 200, &["cargo", "clippy"]));
        assert_eq!(activity::verify_activity(&processes, 100).text(), step);
    }
    let processes = [
        process(100, 1, &["python3", "mesh_runs.py"]),
        process(101, 100, &["ssh", "mac", "cargo nextest run"]),
    ];
    assert_eq!(
        activity::verify_activity(&processes, 100).text(),
        "nextest (mac)"
    );
    assert_eq!(
        activity::verify_activity(&processes[..1], 100).text(),
        "waiting"
    );
}

#[test]
fn activity_detached_run_uses_thread_identity_and_ignores_finished_or_dead_records() {
    let processes = [
        process(100, 1, &["bash", "verify.sh"]),
        process(101, 100, &["cargo", "clippy"]),
    ];
    let records = [
        run(100, 100.0, "running", "our-thread"),
        run(999, 999.0, "running", "our-thread"),
    ];
    assert_eq!(
        activity::run_activity(&records, &processes, "our-thread").text(),
        "clippy"
    );
    assert_eq!(
        activity::run_activity(&records, &processes, "other-thread"),
        AgentActivity::Unreported
    );
    assert_eq!(
        activity::run_activity(
            &[run(100, 100.0, "finished", "our-thread")],
            &processes,
            "our-thread"
        ),
        AgentActivity::Unreported
    );
}

#[test]
fn activity_newest_live_run_wins_then_reveals_the_older_run_when_it_finishes() {
    let processes = [
        process(100, 1, &["bash", "verify.sh"]),
        process(101, 100, &["cargo", "clippy"]),
        process(200, 1, &["bash", "verify.sh"]),
    ];
    let older = run(100, 100.0, "running", "worker");
    let newest = run(200, 200.0, "running", "worker");
    assert_eq!(
        activity::run_activity(&[newest, older], &processes, "worker").text(),
        "waiting"
    );
    assert_eq!(
        activity::run_activity(
            &[
                run(200, 200.0, "finished", "worker"),
                run(100, 100.0, "running", "worker")
            ],
            &processes,
            "worker"
        )
        .text(),
        "clippy"
    );
    assert_eq!(
        activity::run_activity(
            &[
                run(100, 100.0, "running", "worker"),
                run(200, 100.0, "running", "worker")
            ],
            &processes,
            "worker"
        )
        .text(),
        "waiting",
        "higher pid breaks equal start times"
    );
}

#[test]
fn activity_older_probe_statuses_convert_at_the_wire_boundary() {
    let rows = [Some("busy"), Some("idle"), None].map(|status| {
        serde_json::json!({
            "agent":"claude", "name":"legacy-session", "status":status,
            "started":100, "pid":42, "desktop":null, "directory":"/fixture",
            "branch":null, "launched_by":null, "children":[]
        })
    });
    let encoded = serde_json::to_vec(&serde_json::json!({
        "schema":3, "machine":"old-mac", "rows":rows
    }))
    .expect("legacy report encodes");
    let (decoded, _) = probe::parse_report(&encoded).expect("older schema-three report reads");
    let words: Vec<_> = decoded.iter().map(summary::status_text).collect();
    assert_eq!(words, ["thinking", "idle", crate::constants::MISSING_VALUE]);
    assert!(decoded.iter().all(|row| row.process_started == row.started));
}

#[test]
fn activity_probe_round_trip_keeps_the_step_location_and_legacy_child_busy_state() {
    let bytes = serde_json::to_vec(&serde_json::json!({"schema":3,"machine":"mac","rows":[{
        "agent":"claude","name":"session","status":"idle",
        "activity":{"active":{"verify":{"step":"nextest","location":"mac"}}},
        "started":100,"pid":10,"desktop":null,"directory":"/fixture","branch":null,"launched_by":null,
        "children":[{"kind":"thread","depth":0,"pid":null,"name":"worker","started":200,"timer":null,"thread_state":"busy"}]
    }]})).expect("mixed-version probe encodes");
    let (rows, _) = probe::parse_report(&bytes).expect("new activity and old child decode");
    assert_eq!(
        crate::summary::status_text(&rows[0]),
        "nextest (mac)",
        "semantic activity overrides a legacy field"
    );
    assert_eq!(rows[0].children[0].activity.text(), "thinking");
    let encoded = serde_json::to_vec(&serde_json::json!({"schema":3,"machine":"mac","rows":rows}))
        .expect("new report serializes");
    let (decoded, _) = probe::parse_report(&encoded).expect("new report reads back");
    assert_eq!(crate::summary::status_text(&decoded[0]), "nextest (mac)");
    assert_eq!(decoded[0].children[0].activity.text(), "thinking");
}
