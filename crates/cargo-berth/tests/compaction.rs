//! End-to-end checks that `init --compact-journal` removes superseded merge extent
//! observations without changing anything a reader reports.
//!
//! One fixture holds a reservation whose holder committed between board runs, so several
//! holder observations accumulate, then an incursion, its resolution, and a release.

use std::io::Write as _;
use std::path::Path;
use std::process::Output;
use std::process::Stdio;

use cargo_berth_test_support::DirectorySnapshot;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;
use serde_json::Value;
use tempfile::TempDir;

#[path = "support/compaction_fixture.rs"]
mod compaction_fixture;
use compaction_fixture::EXECUTABLE;
use compaction_fixture::HELD_PATH;
use compaction_fixture::TestResult;
use compaction_fixture::run;

const SESSION: &str = "compaction";
const JOURNAL_BYTE_OFFSET: &str = "journal_byte_offset";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LAST_ACTIVITY_AT: &str = "last_activity_at";

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
        let worktrees = tempfile::tempdir()?;
        let root = compaction_fixture::PREPARED_COMPACTION
            .instantiate_with_external_worktrees(worktrees.path());
        Ok(Self { root, worktrees })
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
