//! Regressions at conversation and detached-run boundaries.

use std::error::Error;
use std::io::Write;

use super::*;
use crate::census::account::ConfigHome;

type TestResult = Result<(), Box<dyn Error>>;

fn process(pid: u32, parent: u32, arguments: &[&str]) -> ProcessEntry {
    ProcessEntry {
        pid,
        parent: Some(parent),
        name: arguments[0].into(),
        arguments: arguments.iter().map(|word| (*word).to_owned()).collect(),
        started: 100,
        directory: None,
        claude_pid: None,
        config_home: ConfigHome::Unread,
    }
}

fn record(pid: u32, started_at: f64, command: &[&str]) -> Result<RunRecord, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "pid":pid,"started_at":started_at,"thread_id":"worker","status":"running","command":command
    }))
}

#[test]
fn unrelated_detached_commands_never_override_a_tool_or_an_older_gate() -> TestResult {
    let processes = [
        process(100, 1, &["bash", "/fixture/verify.sh"]),
        process(101, 100, &["cargo", "clippy"]),
        process(200, 1, &["python3", "send.py"]),
    ];
    let unrelated = record(200, 200.0, &["python3", "send.py", "--text", "test"])?;
    assert_eq!(
        run_activity(std::slice::from_ref(&unrelated), &processes, "worker"),
        AgentActivity::Unreported
    );
    let mentioned = record(200, 200.0, &["echo", "/fixture/verify.sh"])?;
    assert_eq!(
        run_activity(&[mentioned], &processes, "worker"),
        AgentActivity::Unreported
    );
    assert_eq!(
        run_activity(
            &[
                record(100, 100.0, &["bash", "/fixture/verify.sh", "lint"])?,
                unrelated
            ],
            &processes,
            "worker"
        )
        .text(),
        "clippy"
    );
    Ok(())
}

#[test]
fn shell_reads_keep_their_word_with_pipes_expansions_and_redirections() {
    for command in [
        "rg needle src | head -20",
        "cat \"$file\"",
        "sed -n 1,5p f > out",
        "find src 2>/dev/null",
        "grep \"$(cat pattern)\" input",
    ] {
        assert_eq!(
            tool_activity("exec_command", &serde_json::json!({"cmd":command})).text(),
            "reading",
            "{command}"
        );
    }
    assert_eq!(
        tool_activity(
            "Bash",
            &serde_json::json!({"command":"echo 'git commit' | cat"})
        )
        .text(),
        "running"
    );
}

#[test]
fn claude_text_block_user_turn_reopens_after_completion_but_tool_results_do_not() {
    let mut read = ActivityRead::default();
    read.take_claude(
        &serde_json::json!({"type":"assistant","message":{"content":[],"stop_reason":"end_turn"}}),
    );
    read.take_claude(&serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"done"}]}}));
    assert_eq!(read.activity(), AgentActivity::Idle);
    read.take_claude(&serde_json::json!({"type":"user","message":{"content":[{"type":"text","text":"Next task"}]}}));
    assert_eq!(read.activity().text(), "thinking");
}

#[test]
fn transcript_cursor_reuses_unchanged_input_and_consumes_only_complete_appends() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("transcript.jsonl");
    let message = |tool| {
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":tool,"input":{}}]}}).to_string()
    };
    std::fs::write(&path, format!("{}\n", message("Read")))?;
    let mut read = TranscriptRead::new(AgentActivity::Unreported);
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 1);
    assert_eq!(read.activity().text(), "reading");
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 0);
    let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
    write!(file, "{}", message("Edit"))?;
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 0);
    assert_eq!(read.activity().text(), "reading");
    writeln!(file)?;
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 1);
    assert_eq!(read.activity().text(), "writing");
    std::fs::write(
        &path,
        "{\"type\":\"user\",\"message\":{\"content\":\"restart\"}}\n",
    )?;
    assert_eq!(read.advance(&path, AgentActivity::Unreported), 1);
    assert_eq!(read.activity().text(), "thinking");
    Ok(())
}

#[test]
fn incomplete_run_identity_and_start_time_never_enter_live_gate_selection() -> TestResult {
    let processes = [
        process(100, 1, &["bash", "verify.sh"]),
        process(101, 100, &["cargo", "check"]),
    ];
    let complete = serde_json::json!({"pid":100,"started_at":100.0,"thread_id":"worker","status":"running","command":["bash","verify.sh","check"]});
    for missing in ["thread_id", "started_at"] {
        let mut payload = complete.clone();
        if let Some(fields) = payload.as_object_mut() {
            fields.remove(missing);
        }
        let record = serde_json::from_value(payload)?;
        assert_eq!(
            run_activity(&[record], &processes, "worker"),
            AgentActivity::Unreported
        );
    }
    Ok(())
}

#[test]
fn remote_step_unknown_uses_the_verify_request_without_changing_local_waits() -> TestResult {
    let records = [record(
        100,
        100.0,
        &["bash", "/fixture/verify.sh", "test", "cargo-handler"],
    )?];
    let processes = [
        process(100, 1, &["bash", "verify.sh"]),
        process(101, 100, &["ssh", "mac", "remote-dispatch"]),
    ];
    assert_eq!(
        run_activity(&records, &processes, "worker").text(),
        "nextest (mac)"
    );
    assert_eq!(
        run_activity(&records, &processes[..1], "worker").text(),
        "waiting"
    );
    Ok(())
}

#[test]
fn verification_words_have_the_same_meaning_in_cargo_and_ssh_commands() {
    for word in [
        "fmt", "clippy", "nextest", "build", "check", "doc", "mend", "taplo",
    ] {
        let local = [
            process(100, 1, &["bash", "verify.sh"]),
            process(101, 100, &["cargo", word]),
        ];
        let remote_command = format!("cargo {word}");
        let remote = [
            process(100, 1, &["bash", "verify.sh"]),
            process(101, 100, &["ssh", "mac", &remote_command]),
        ];
        assert_eq!(verify_activity(&local, 100).text(), word);
        assert_eq!(
            verify_activity(&remote, 100).text(),
            format!("{word} (mac)")
        );
    }
}

#[test]
fn remote_verification_finds_a_step_after_shell_setup_without_matching_substrings() {
    let root = process(100, 1, &["bash", "verify.sh"]);
    let processes = [
        root.clone(),
        process(
            101,
            100,
            &["ssh", "mac", "cd /work && cargo nextest run -p fixture"],
        ),
    ];
    assert_eq!(verify_activity(&processes, 100).text(), "nextest (mac)");
    let unrelated = [
        root,
        process(101, 100, &["ssh", "mac", "cat /work/nextest-results.log"]),
    ];
    assert_eq!(verify_activity(&unrelated, 100).text(), "waiting");
}

fn run_file(directory: &Path, name: &str, pid: u32) -> TestResult {
    fs::create_dir_all(directory.join("mesh_runs"))?;
    fs::write(
        directory.join("mesh_runs").join(name),
        serde_json::to_vec(&serde_json::json!({
            "pid":pid,"started_at":1,"thread_id":"worker","status":"running",
            "command":["bash","verify.sh","test"]
        }))?,
    )?;
    Ok(())
}

#[test]
fn idle_run_records_parse_unchanged_files_only_once() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::default();
    let mut parses = 0;
    for _ in 0..2 {
        let answer = records.read_with(directory.path(), |bytes| {
            parses += 1;
            serde_json::from_slice(bytes).ok()
        });
        assert_eq!(answer.len(), 1);
    }
    assert_eq!(parses, 1, "unchanged records must not be parsed again");
    Ok(())
}

#[test]
fn idle_run_records_parse_changed_and_new_files_only() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::default();
    records.read(directory.path());
    run_file(directory.path(), "one.json", 12345)?;
    run_file(directory.path(), "two.json", 2)?;
    let mut parses = 0;
    let answer = records.read_with(directory.path(), |bytes| {
        parses += 1;
        serde_json::from_slice(bytes).ok()
    });
    assert_eq!(parses, 2);
    assert_eq!(answer.len(), 2);
    let mut unchanged = 0;
    records.read_with(directory.path(), |bytes| {
        unchanged += 1;
        serde_json::from_slice(bytes).ok()
    });
    assert_eq!(
        unchanged, 0,
        "changed records are retained after their first parse"
    );
    Ok(())
}

#[test]
fn idle_run_records_remove_deleted_files_from_the_cache() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::default();
    assert_eq!(records.read(directory.path()).len(), 1);
    fs::remove_file(directory.path().join("mesh_runs/one.json"))?;
    assert!(
        records.read(directory.path()).is_empty(),
        "removed records leave the cache"
    );
    Ok(())
}

#[test]
fn idle_run_records_keep_sessions_separate_and_retry_revised_partial_json() -> TestResult {
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    run_file(first.path(), "one.json", 1)?;
    run_file(second.path(), "two.json", 2)?;
    let mut records = RunRecords::default();
    assert_eq!(records.read(first.path()).len(), 1);
    assert_eq!(records.read(second.path()).len(), 1);
    let partial = first.path().join("mesh_runs/partial.json");
    fs::write(&partial, "{")?;
    let mut parses = 0;
    for _ in 0..2 {
        assert_eq!(
            records
                .read_with(first.path(), |bytes| {
                    parses += 1;
                    serde_json::from_slice(bytes).ok()
                })
                .len(),
            1
        );
    }
    assert_eq!(parses, 1, "one parse per partial revision");
    run_file(first.path(), "partial.json", 3)?;
    assert_eq!(records.read(first.path()).len(), 2);
    records.retain_sessions(&HashSet::from([second.path().to_owned()]));
    assert_eq!(records.sessions.len(), 1);
    Ok(())
}

#[test]
fn idle_run_records_detect_same_size_rewrites_and_replacements() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let path = directory.path().join("mesh_runs/one.json");
    let modified = fs::metadata(&path)?.modified()?;
    let mut records = RunRecords::default();
    records.read(directory.path());
    for (pid, replace) in [(2, false), (3, true)] {
        if replace {
            run_file(directory.path(), "replacement.json", pid)?;
            fs::rename(directory.path().join("mesh_runs/replacement.json"), &path)?;
        } else {
            run_file(directory.path(), "one.json", pid)?;
        }
        fs::File::options()
            .write(true)
            .open(&path)?
            .set_times(fs::FileTimes::new().set_modified(modified))?;
        let answer = records.read(directory.path());
        assert!(
            matches!(&answer[..], [RunRecord::Verify(run)] if run.pid == pid),
            "equal-size records with restored mtime must replace the old answer"
        );
    }
    Ok(())
}

#[test]
fn idle_run_records_retry_failed_io_but_keep_malformed_revisions() -> TestResult {
    let directory = tempfile::tempdir()?;
    run_file(directory.path(), "one.json", 1)?;
    let mut records = RunRecords::default();
    let parse = |bytes: &[u8]| serde_json::from_slice(bytes).ok();
    assert!(
        records
            .read_with_io(directory.path(), parse, |_| {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            })
            .is_empty()
    );
    assert_eq!(
        records.read(directory.path()).len(),
        1,
        "a transient read failure must retry even with unchanged metadata"
    );
    fs::write(directory.path().join("mesh_runs/one.json"), "{")?;
    let mut parses = 0;
    for _ in 0..2 {
        records.read_with(directory.path(), |bytes| {
            parses += 1;
            serde_json::from_slice(bytes).ok()
        });
    }
    assert_eq!(
        parses, 1,
        "malformed JSON retries only after its revision changes"
    );
    Ok(())
}

#[test]
fn idle_run_records_copy_only_live_verification_candidates() -> TestResult {
    let directory = tempfile::tempdir()?;
    for pid in 1..=3 {
        run_file(directory.path(), &format!("{pid}.json"), pid)?;
    }
    let mut records = RunRecords::default();
    for _ in 0..2 {
        let answer = records.read_for_processes(directory.path(), &HashSet::from([2]));
        assert_eq!(
            answer.len(),
            1,
            "only live runs are copied out for worker matching"
        );
        assert!(matches!(&answer[..], [RunRecord::Verify(run)] if run.pid == 2));
    }
    Ok(())
}
