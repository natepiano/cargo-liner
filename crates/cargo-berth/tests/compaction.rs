//! End-to-end checks that `init --compact-journal` removes superseded merge extent
//! observations without changing anything a reader reports.
//!
//! One fixture holds a reservation whose holder committed between board runs, so several
//! holder observations accumulate, then an incursion, its resolution, and a release.

use std::error::Error;
use std::io::Write as _;
use std::path::Path;
use std::process::Output;
use std::process::Stdio;

use cargo_berth_test_support::DirectorySnapshot;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::claim_id;
use cargo_berth_test_support::json;
use cargo_berth_test_support::write_file;
use serde_json::Value;
use tempfile::TempDir;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[
        "CARGO_BERTH_RUN",
        "CARGO_BERTH_SESSION_ID",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
    ],
};
/// Fixture commits skip the managed hooks, which no check here is about.
const HOOKS_DISABLED: &str = "core.hooksPath=/dev/null";
const HELD_PATH: &str = "src/held.rs";
const HOLDER_COMMITS: usize = 3;
const HOLDER_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const STRAYING_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a2c";
const JOURNAL_BYTE_OFFSET: &str = "journal_byte_offset";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LAST_ACTIVITY_AT: &str = "last_activity_at";
const SESSION: &str = "compaction";

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[test]
fn compaction_changes_no_reader_output_and_a_second_run_finds_nothing() -> TestResult {
    let fixture = CompactionFixture::new()?;
    let root = fixture.root.path();
    let repository = DirectorySnapshot::capture(root);
    let worktrees = DirectorySnapshot::capture(fixture.worktrees.path());
    let before = fixture.reader_outputs()?;
    // Readers append records of their own, so compaction starts from the state the first
    // reads saw.
    repository.restore(root);
    worktrees.restore(fixture.worktrees.path());
    let journal_before = std::fs::metadata(root.join(JOURNAL_PATH))?.len();

    let compacted = run(root, &["init", "--compact-journal", "--json"])?;
    assert_success(&compacted);
    let compacted = json(&compacted);
    assert_eq!(compacted["status"], "journal_compacted", "{compacted}");
    let data = &compacted["payload"]["data"];
    assert_eq!(data["status"], "compacted", "{compacted}");
    let removed = data["removed"]["records"]
        .as_u64()
        .ok_or("removed records")?;
    assert!(removed > 0, "{compacted}");
    let removed_bytes = data["removed"]["bytes"].as_u64().ok_or("removed bytes")?;
    let remaining_bytes = data["remaining"]["bytes"]
        .as_u64()
        .ok_or("remaining bytes")?;
    assert_eq!(removed_bytes + remaining_bytes, journal_before);
    assert_eq!(
        std::fs::metadata(root.join(JOURNAL_PATH))?.len(),
        remaining_bytes
    );

    let second = run(root, &["init", "--compact-journal", "--json"])?;
    assert_success(&second);
    let second = json(&second);
    assert_eq!(second["status"], "journal_compacted", "{second}");
    assert_eq!(
        second["payload"]["data"],
        serde_json::json!({"status": "nothing_to_compact"})
    );

    let after = fixture.reader_outputs()?;
    for ((command, before), (_, after)) in before.iter().zip(&after) {
        assert_eq!(normalized(after)?, normalized(before)?, "{command}");
    }
    assert_ne!(
        before[0].1, after[0].1,
        "the board reports the shorter journal"
    );
    Ok(())
}

/// A repository whose journal holds superseded holder observations beside an incursion, its
/// resolution, and a release.
struct CompactionFixture {
    root:      TempDir,
    worktrees: TempDir,
}

impl CompactionFixture {
    fn new() -> TestResult<Self> {
        let fixture = Self {
            root:      tempfile::tempdir()?,
            worktrees: tempfile::tempdir()?,
        };
        let root = fixture.root.path();
        GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
        GIT.run(root, ["config", "user.name", "Berth Test"]);
        GIT.run(root, ["config", "user.email", "berth@example.invalid"]);
        GIT.run(root, ["config", "gc.auto", "0"]);
        write_file(root, "README.md", "initial\n");
        write_file(root, HELD_PATH, "held\n");
        GIT.run(root, ["add", "README.md", HELD_PATH]);
        commit(root, "initial");
        assert_success(&run(root, &["init", "--json"])?);
        GIT.run(root, ["add", ".claude/config/berth.toml"]);
        commit(root, "track berth configuration");
        let holder = fixture.worktrees.path().join("holder");
        GIT.add_worktree_without_hooks(root, &holder, "holder", "main");
        let holder = std::fs::canonicalize(holder)?;

        let holder_id = claim_id(&run(
            &holder,
            &["claim", HELD_PATH, "--run", HOLDER_RUN, "--json"],
        )?);
        let straying_id = claim_id(&run(
            root,
            &["claim", "owned.txt", "--run", STRAYING_RUN, "--json"],
        )?);
        // Each commit moves the holder's merge extent, so each board run observes it again and
        // supersedes the observation before.
        for edit in 1..=HOLDER_COMMITS {
            write_file(&holder, HELD_PATH, &format!("held\nedit {edit}\n"));
            GIT.run(&holder, ["add", HELD_PATH]);
            commit(&holder, &format!("holder edit {edit}"));
            assert_success(&run(root, &["board", "--json"])?);
        }

        write_file(root, HELD_PATH, "entered\n");
        let drift = run(
            root,
            &["drift", "--full", "--reservation", &straying_id, "--json"],
        )?;
        assert_eq!(json(&drift)["status"], "incursion");
        let resolved = run(
            root,
            &["resolve", &straying_id, "--every-incursion", "--json"],
        )?;
        assert_success(&resolved);
        let released = berth_command(EXECUTABLE)
            .args(["release", &holder_id, "--json"])
            .current_dir(&holder)
            .env("CARGO_BERTH_RUN", HOLDER_RUN)
            .env_remove("CARGO_BERTH_SESSION_ID")
            .output()?;
        assert_success(&released);
        // An unclaimed file gives the PostToolUse hook a widening to report.
        write_file(root, "unclaimed.txt", "unclaimed\n");
        Ok(fixture)
    }

    /// Every reader response the compaction must leave unchanged, labelled by command.
    fn reader_outputs(&self) -> TestResult<Vec<(&'static str, Output)>> {
        let root = self.root.path();
        let canonical_root = std::fs::canonicalize(root)?;
        let session_start = serde_json::json!({
            "hook_event_name": "SessionStart",
            "cwd": canonical_root,
            "source": "startup",
            "session_id": SESSION,
        });
        let post_tool_use = serde_json::json!({
            "tool_name": "Bash",
            "cwd": canonical_root,
            "tool_input": {"command": "true"},
            "session_id": SESSION,
        });
        Ok(vec![
            ("board", run(root, &["board", "--json"])?),
            (
                "check",
                run(root, &["check", &format!("file:{HELD_PATH}"), "--json"])?,
            ),
            ("drift", run(root, &["drift", "--full", "--json"])?),
            (
                "session-start",
                hook(root, "session-start", &session_start)?,
            ),
            (
                "post-tool-use",
                hook(root, "post-tool-use", &post_tool_use)?,
            ),
        ])
    }
}

/// Run `cargo-berth` in `checkout` without this process's coordination run or session.
fn run(checkout: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(berth_command(EXECUTABLE)
        .args(arguments)
        .current_dir(checkout)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()?)
}

fn commit(checkout: &Path, message: &str) {
    GIT.run(
        checkout,
        ["-c", HOOKS_DISABLED, "commit", "--quiet", "-m", message],
    );
}

fn hook(checkout: &Path, event: &str, payload: &Value) -> TestResult<Output> {
    let mut child = berth_command(EXECUTABLE)
        .args(["hook", event])
        .current_dir(checkout)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("hook stdin is piped")?
        .write_all(&serde_json::to_vec(payload)?)?;
    Ok(child.wait_with_output()?)
}

/// Standard output with its `journal_byte_offset` values, the one field a compaction may
/// change, and its `last_activity_at` values replaced, including inside hook text that quotes
/// the board.
///
/// A reader renews the activity of the reservation it reads for, so each set of reads records
/// its own clock time there.
fn normalized(output: &Output) -> TestResult<String> {
    let mut text = String::from_utf8(output.stdout.clone())?;
    for field in [JOURNAL_BYTE_OFFSET, LAST_ACTIVITY_AT] {
        text = with_values_replaced(&text, field)?;
    }
    Ok(text)
}

/// `text` with the number or timestamp after each occurrence of `field` replaced by `{field}`.
fn with_values_replaced(text: &str, field: &str) -> TestResult<String> {
    let is_value = |character: char| character.is_ascii_digit() || "-:.TZ".contains(character);
    let mut replaced = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(field) {
        let after_name = start + field.len();
        let value_start = after_name
            + rest[after_name..]
                .find(|character: char| character.is_ascii_digit())
                .ok_or("every replaced field has a value")?;
        replaced.push_str(&rest[..value_start]);
        replaced.push('{');
        replaced.push_str(field);
        replaced.push('}');
        rest = rest[value_start..].trim_start_matches(is_value);
    }
    replaced.push_str(rest);
    Ok(replaced)
}
