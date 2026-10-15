#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! A director can coordinate parallel phases without combining their reservations.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Output;
use std::process::Stdio;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;
use cargo_berth_test_support::reservation_row;
use serde_json::Value;
use tempfile::TempDir;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const DIRECTOR_SESSION: &str = "dir-sid";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &["CARGO_BERTH_RUN", "CARGO_BERTH_SESSION_ID"],
};

#[test]
fn a_director_runs_two_phases_each_with_its_own_reservation() {
    let repository = initialized_repository();
    let root = repository.path();

    let first = run_berth_with_session(root, &["check", "file:a.rs", "--json"], "dir-sid.phase-15");
    assert_success(&first);
    let first = json(&first);
    let first_acquisition = &first["payload"]["data"]["acquisition"];
    let r15 = first_acquisition["reservation_id"]
        .as_str()
        .expect("phase 15 should name its reservation");
    assert_eq!(first_acquisition["kind"], "appended");
    let run = first_acquisition["coordination_run_id"]
        .as_str()
        .expect("phase 15 should name its coordination run");

    let second =
        run_berth_with_session(root, &["check", "file:b.rs", "--json"], "dir-sid.phase-16");
    assert_success(&second);
    let second = json(&second);
    let second_acquisition = &second["payload"]["data"]["acquisition"];
    let r16 = second_acquisition["reservation_id"]
        .as_str()
        .expect("phase 16 should name its reservation");
    assert_ne!(r15, r16, "a new phase must receive its own reservation");
    assert_eq!(second_acquisition["kind"], "appended");
    assert_eq!(second_acquisition["coordination_run_id"], run);

    let board = run_berth_with_session(root, &["board", "--json"], DIRECTOR_SESSION);
    assert_success(&board);
    let board = json(&board);
    assert_eq!(
        reservation_row(&board, r15)["scopes"],
        serde_json::json!([{"kind": "file", "path": "a.rs"}]),
        "opening phase 16 must leave phase 15's declaration alone"
    );

    let mapped = run_berth_with_session(
        root,
        &["check", "--reservation", r15, "file:a.rs", "--json"],
        DIRECTOR_SESSION,
    );
    assert_success(&mapped);
    assert_eq!(
        json(&mapped)["payload"]["data"]["acquisition"]["reservation_id"],
        r15
    );
    let mappings: Value = serde_json::from_slice(
        &fs::read(root.join(".git/cargo-berth/session-identities.json"))
            .expect("session mappings should read"),
    )
    .expect("session mappings should be JSON");
    assert_eq!(
        mappings["identities"][DIRECTOR_SESSION]["reservation_id"],
        r15
    );
    assert_eq!(
        mappings["identities"]["dir-sid.phase-15"]["reservation_id"],
        r15
    );
    assert_eq!(
        mappings["identities"]["dir-sid.phase-16"]["reservation_id"],
        r16
    );

    fs::write(root.join("c.rs"), "// unheld Bash write\n")
        .expect("Bash's unheld file should write");
    let before_hook = journal_events(root);
    let hook = run_post_tool_use(root);
    assert_success(&hook);
    let after_hook = journal_events(root);
    assert!(
        !after_hook[before_hook.len()..]
            .iter()
            .any(|event| event["op"] == "widen"),
        "the director's session mapping must not attribute an unheld path to either phase"
    );
    let answer = json(&hook);
    let detail = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("ambiguous drift should explain its candidates to the director");
    for expected in ["c.rs", r15, r16] {
        assert!(
            detail.contains(expected),
            "missing {expected} in drift answer: {detail}"
        );
    }

    let released = run_berth_with_session(root, &["release", r15, "--json"], "dir-sid.phase-15");
    assert_success(&released);
    let board = run_berth_with_session(root, &["board", "--json"], "dir-sid.phase-16");
    assert_success(&board);
    let board = json(&board);
    let remaining = reservation_row(&board, r16);
    assert_eq!(remaining["lifecycle"]["stage"], "active");
    assert_eq!(
        remaining["scopes"],
        serde_json::json!([{"kind": "file", "path": "b.rs"}]),
        "releasing phase 15 must leave phase 16 active with its original declaration"
    );
    assert!(
        journal_events(root)
            .iter()
            .any(|event| { event["op"] == "release" && event["reservation_id"] == r15 })
    );
}

#[test]
fn a_fresh_session_reports_left_unmapped_when_siblings_hold_every_requested_path() {
    let repository = initialized_repository();
    let root = repository.path();
    let first = run_berth_with_session(root, &["check", "file:a.rs", "--json"], "dir-sid.phase-15");
    assert_success(&first);
    let first = json(&first);
    let reservation = &first["payload"]["data"]["acquisition"]["reservation_id"];
    let events_before = journal_events(root);

    let covered =
        run_berth_with_session(root, &["check", "file:a.rs", "--json"], "dir-sid.phase-16");
    assert_success(&covered);
    let covered = json(&covered);
    let acquisition = &covered["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "already_held");
    assert_eq!(&acquisition["reservation_id"], reservation);
    assert_eq!(
        acquisition["session_mapping_publication"],
        serde_json::json!({"status": "left_unmapped"})
    );
    let mappings: Value = serde_json::from_slice(
        &fs::read(root.join(".git/cargo-berth/session-identities.json"))
            .expect("session mappings should read"),
    )
    .expect("session mappings should be JSON");
    assert!(mappings["identities"].get("dir-sid.phase-16").is_none());
    assert_eq!(journal_events(root), events_before);

    let opened =
        run_berth_with_session(root, &["check", "file:b.rs", "--json"], "dir-sid.phase-16");
    assert_success(&opened);
    let opened = json(&opened);
    let acquisition = &opened["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "appended");
    assert_ne!(&acquisition["reservation_id"], reservation);
    assert_eq!(
        acquisition["session_mapping_publication"]["status"],
        "published"
    );
}

fn initialized_repository() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository should exist");
    let root = repository.path();
    GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
    GIT.run(root, ["config", "user.name", "Berth Test"]);
    GIT.run(root, ["config", "user.email", "berth@example.invalid"]);
    fs::write(root.join("README.md"), "parallel phase fixture\n")
        .expect("initial file should write");
    GIT.run(root, ["add", "README.md"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth_with_session(root, &["init", "--json"], DIRECTOR_SESSION);
    assert_success(&initialized);
    GIT.run(root, ["add", ".claude/config/berth.toml"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "configure berth"]);
    repository
}

fn run_berth_with_session(root: &Path, arguments: &[&str], session_id: &str) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session_id)
        .output()
        .expect("cargo-berth should run")
}

fn run_post_tool_use(root: &Path) -> Output {
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "cwd": root,
        "tool_input": {"command": "printf '// unheld Bash write\\n' > c.rs"},
        "session_id": DIRECTOR_SESSION,
    });
    let mut child = berth_command(BERTH_EXECUTABLE)
        .args(["hook", "post-tool-use"])
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("post-tool-use hook should start");
    let mut stdin = child.stdin.take().expect("hook stdin should be piped");
    stdin
        .write_all(&serde_json::to_vec(&payload).expect("Bash payload should serialize"))
        .expect("Bash payload should reach the hook");
    drop(stdin);
    child
        .wait_with_output()
        .expect("post-tool-use hook should finish")
}

fn journal_events(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should be JSON"))
        .collect()
}
