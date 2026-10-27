//! Scan-time integration chooses detached gates over the conversation's tool.

#![expect(clippy::expect_used, reason = "isolated scan fixture files must parse")]

use std::io::Write;

use super::*;
use crate::census::account::ConfigHome;
use crate::config::AccountsConfig;

const THREAD: &str = "01a1437d-1234-5678-9abc-0123456789ab";

fn process(pid: u32, parent: u32, args: &[&str]) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: args[0].into(),
        arguments: args.iter().map(|arg| (*arg).to_owned()).collect(),
        started: 100,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

#[test]
fn activity_scan_detached_gate_overrides_tool_then_finished_run_reveals_the_tool() {
    let root = tempfile::tempdir().expect("isolated session root");
    let directory = tempfile::Builder::new()
        .prefix("handler-activity-")
        .tempdir_in(root.path())
        .expect("isolated session");
    let run_dir = directory.path().join("mesh_runs");
    fs::create_dir(&run_dir).expect("run directory");
    let run_path = run_dir.join("gate.json");
    let write_run = |status: &str| {
        fs::write(&run_path,serde_json::json!({
        "worker":"fixture-test","thread_id":THREAD,"pid":100,"command":["bash","verify.sh","test","cargo-handler"],"status":status,"started_at":100.0
    }).to_string()).expect("run record writes");
    };
    write_run("running");
    let rollout = directory.path().join(format!("rollout-{THREAD}.jsonl"));
    let lines = [
        serde_json::json!({"type":"session_meta","payload":{"cwd":"/work/fixture"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":format!("Write fixtures\nSummary: {}/phases/4/summary.txt",directory.path().display())}]}}),
        serde_json::json!({"type":"event_msg","payload":{"type":"task_started"}}),
        serde_json::json!({"type":"response_item","payload":{"type":"function_call","name":"apply_patch","arguments":"{}"}}),
    ];
    fs::write(
        &rollout,
        format!(
            "{}\n",
            lines
                .iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .expect("rollout writes");
    let mut scanner = LocalScanner::new(AccountLabels::new(&AccountsConfig::default(), None));
    scanner.session_roots = SessionRoots::new([root.path().to_path_buf()]);
    scanner.rollouts.read(&rollout);
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"root","pid":10,"started":10,"status":"busy","desktop":null,"directory":"/work/fixture","branch":null,"launched_by":null,
        "children":[{"kind":"thread","depth":0,"pid":null,"thread_id":THREAD,"name":THREAD,"started":20,"timer":null}]
    }])).expect("scan rows");
    let processes = [
        process(100, 1, &["bash", "verify.sh"]),
        process(101, 100, &["cargo", "nextest", "run"]),
    ];
    scanner.attach_activity_and_names(&processes, &HashMap::new(), &mut rows);
    assert_eq!(rows[0].children[0].activity.text(), "nextest");
    write_run("finished");
    scanner.attach_activity_and_names(&processes, &HashMap::new(), &mut rows);
    assert_eq!(rows[0].children[0].activity.text(), "writing");
    assert_eq!(rows[0].children[0].name, "worker · Write fixtures");
}

#[test]
fn activity_scan_claude_transcript_updates_and_its_descendant_gate_takes_precedence() {
    let directory = tempfile::tempdir().expect("transcript fixture");
    let transcript = directory.path().join("claude.jsonl");
    let write_tool = |tool: &str| {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&transcript)
            .expect("transcript opens");
        writeln!(file,"{}",serde_json::json!({
        "type":"assistant","message":{"content":[{"type":"tool_use","name":tool,"input":{"file_path":"/fixture/input.rs"}}],"stop_reason":"tool_use"}
    })).expect("transcript writes");
    };
    write_tool("Read");
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","name":"root","pid":10,"started":10,"status":"busy","desktop":null,"directory":"/work/fixture","branch":null,"launched_by":null,"children":[]
    }])).expect("Claude row");
    let transcripts = HashMap::from([(10, transcript.clone())]);
    let mut scanner = LocalScanner::new(AccountLabels::new(&AccountsConfig::default(), None));
    scanner.attach_activity_and_names(&[], &transcripts, &mut rows);
    assert_eq!(rows[0].activity.text(), "reading");
    write_tool("Edit");
    scanner.attach_activity_and_names(&[], &transcripts, &mut rows);
    assert_eq!(
        rows[0].activity.text(),
        "writing",
        "a subsequent scan reads the changed transcript"
    );
    let processes = [
        process(10, 1, &["claude"]),
        process(
            100,
            10,
            &["bash", "/fixture/verify.sh", "lint", "cargo-handler"],
        ),
        process(101, 100, &["cargo", "clippy"]),
    ];
    scanner.attach_activity_and_names(&processes, &transcripts, &mut rows);
    assert_eq!(rows[0].activity.text(), "clippy");
    scanner.attach_activity_and_names(&processes[..1], &transcripts, &mut rows);
    assert_eq!(
        rows[0].activity.text(),
        "writing",
        "a completed gate exposes the last tool"
    );
}
