//! Cold reads preserve old metadata while steady reads fetch only appended bytes.

#![expect(
    clippy::expect_used,
    reason = "isolated JSONL fixtures must be readable"
)]

use std::fs;
use std::io::Write;
use std::path::Path;

use super::Activity;
use super::AgentActivity;
use super::ServiceTier;
use super::activity::ActivityRead;
use super::activity::TranscriptRead;
use super::tier;
use super::tier::PacerHistory;
use super::tier::RolloutRead;
use super::tier::Rollouts;
use super::tier::ThreadWorktree;
use crate::constants::TRANSCRIPT_TAIL_START;

const CLAUDE_HEAD: &str = include_str!("fixtures/incremental_claude_head.jsonl");
const CLAUDE_TAIL: &str = include_str!("fixtures/incremental_claude_tail.jsonl");
const CODEX_HEAD: &str = include_str!("fixtures/incremental_codex_head.jsonl");
const CODEX_TAIL: &str = include_str!("fixtures/incremental_codex_tail.jsonl");
const MEBIBYTE: usize = 1024 * 1024;
const LARGE_FILE_BYTES: usize = MEBIBYTE + MEBIBYTE / 16;
const CLAUDE_COMPLETE: &str =
    "{\"type\":\"assistant\",\"message\":{\"content\":[],\"stop_reason\":\"end_turn\"}}\n";
const CODEX_COMPLETE: &str = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n";
const CODEX_START: &str = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n";

fn large_conversation(head: &str, tail: &str) -> String {
    // A neutral record cannot reset the last tool or supply bootstrap metadata.
    format!("{head}{}{tail}", neutral_records(LARGE_FILE_BYTES))
}

fn append(path: &Path, text: &str) {
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("fixture opens for append")
        .write_all(text.as_bytes())
        .expect("fixture appends");
}

#[derive(Clone, Copy)]
enum Family {
    Claude,
    Codex,
}

fn whole_activity(text: &str, family: Family) -> AgentActivity {
    let mut read = ActivityRead::default();
    for line in text.lines() {
        let record = serde_json::from_str(line).expect("fixture record parses");
        match family {
            Family::Claude => read.take_claude(&record),
            Family::Codex => read.take_codex(&record),
        }
    }
    read.activity()
}

fn rollout_tier(read: RolloutRead) -> ServiceTier {
    tier::requested(Some(read), &[], &PacerHistory::default(), 0)
}

#[test]
fn incremental_claude_cold_tail_matches_whole_activity_without_reading_the_whole_file() {
    let directory = tempfile::tempdir().expect("isolated transcript directory");
    let path = directory.path().join("claude.jsonl");
    let text = large_conversation(CLAUDE_HEAD, CLAUDE_TAIL);
    fs::write(&path, &text).expect("large transcript writes");
    let mut read = TranscriptRead::new(AgentActivity::Idle);
    read.advance(&path, AgentActivity::Idle);
    assert_eq!(
        read.activity(),
        whole_activity(&format!("{CLAUDE_HEAD}{CLAUDE_TAIL}"), Family::Claude)
    );
    assert_eq!(read.activity().text(), "reading");
    assert!(read.bytes_read() <= TRANSCRIPT_TAIL_START);
    assert!(read.bytes_read() < u64::try_from(text.len()).expect("file length fits"));
    read.advance(&path, AgentActivity::Idle);
    assert_eq!(
        read.bytes_read(),
        0,
        "an unchanged scan fetches no file bytes"
    );
    assert_eq!(read.activity().text(), "reading");
}

#[test]
fn incremental_claude_append_finishes_a_pending_record_without_fetching_it_twice() {
    let directory = tempfile::tempdir().expect("isolated transcript directory");
    let path = directory.path().join("claude.jsonl");
    fs::write(&path, format!("{CLAUDE_HEAD}{CLAUDE_TAIL}")).expect("transcript writes");
    let mut read = TranscriptRead::new(AgentActivity::Idle);
    read.advance(&path, AgentActivity::Idle);
    let partial = CLAUDE_COMPLETE.trim_end_matches('\n');
    append(&path, partial);
    read.advance(&path, AgentActivity::Idle);
    assert_eq!(
        read.bytes_read(),
        u64::try_from(partial.len()).expect("short append")
    );
    assert_eq!(
        read.activity().text(),
        "reading",
        "a partial record cannot close the turn"
    );
    read.advance(&path, AgentActivity::Idle);
    assert_eq!(read.bytes_read(), 0);
    append(&path, "\n");
    read.advance(&path, AgentActivity::Idle);
    assert_eq!(
        read.bytes_read(),
        1,
        "the buffered prefix is not read again"
    );
    assert_eq!(read.activity(), AgentActivity::Idle);
}

#[test]
fn incremental_codex_cold_tail_keeps_first_task_worktree_start_and_historical_tier() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let large_path = directory.path().join("large.jsonl");
    let small_path = directory.path().join("small.jsonl");
    let text = large_conversation(CODEX_HEAD, CODEX_TAIL);
    fs::write(&large_path, &text).expect("large rollout writes");
    fs::write(&small_path, format!("{CODEX_HEAD}{CODEX_TAIL}")).expect("small rollout writes");
    let mut rollouts = Rollouts::default();
    let large = rollouts.read(&large_path);
    let small = rollouts.read(&small_path);
    assert_eq!(
        large.activity(),
        whole_activity(&format!("{CODEX_HEAD}{CODEX_TAIL}"), Family::Codex)
    );
    assert_eq!(large.activity(), small.activity());
    assert_eq!(large.first_prompt, small.first_prompt);
    assert!(large.first_prompt.text().starts_with("Repair the portal"));
    assert!(!large.first_prompt.text().contains("later follow-up"));
    assert_eq!(
        large.worktree,
        ThreadWorktree::Recorded("/work/fixture".into())
    );
    assert_eq!(large.began, small.began);
    assert!(
        matches!(large.began, tier::RolloutStart::Recorded(_)),
        "session start must survive tail bootstrapping"
    );
    assert_eq!(rollout_tier(large.clone()), ServiceTier::Fast);
    assert_eq!(rollout_tier(large.clone()), rollout_tier(small));
    assert!(
        large.bytes_read() <= u64::try_from(text.len()).expect("file length fits"),
        "the historical tier search must not fetch any byte twice"
    );
    assert_eq!(rollouts.read(&large_path).bytes_read(), 0);
}

#[test]
fn incremental_codex_append_updates_activity_and_tier_without_bootstrapping_again() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let path = directory.path().join("codex.jsonl");
    fs::write(&path, large_conversation(CODEX_HEAD, CODEX_TAIL)).expect("rollout writes");
    let mut rollouts = Rollouts::default();
    let first = rollouts.read(&path);
    let appended = format!(
        "{CODEX_COMPLETE}{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"thread_settings_applied\",\"thread_settings\":{{\"service_tier\":\"default\"}}}}}}\n"
    );
    append(&path, &appended);
    let second = rollouts.read(&path);
    assert_eq!(
        second.bytes_read(),
        u64::try_from(appended.len()).expect("short append")
    );
    assert_eq!(second.activity(), AgentActivity::Idle);
    assert_eq!(second.first_prompt, first.first_prompt);
    assert_eq!(second.worktree, first.worktree);
    assert_eq!(second.began, first.began);
    assert_eq!(rollout_tier(second), ServiceTier::Standard);
    append(&path, CODEX_START.trim_end_matches('\n'));
    let partial = rollouts.read(&path);
    assert_eq!(partial.activity(), AgentActivity::Idle);
    append(&path, "\n");
    let restarted = rollouts.read(&path);
    assert_eq!(restarted.bytes_read(), 1);
    assert_eq!(restarted.activity().text(), "thinking");
    assert_eq!(rollouts.read(&path).bytes_read(), 0);
}

#[test]
fn incremental_codex_paths_and_truncated_files_do_not_reuse_another_conversations_metadata() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let first_path = directory.path().join("first.jsonl");
    let second_path = directory.path().join("second.jsonl");
    fs::write(&first_path, large_conversation(CODEX_HEAD, CODEX_TAIL))
        .expect("first rollout writes");
    let other = CODEX_HEAD
        .replace("Repair the portal", "Review the widget")
        .replace("/work/fixture", "/work/widget");
    fs::write(&second_path, format!("{other}{CODEX_COMPLETE}")).expect("second rollout writes");
    let mut rollouts = Rollouts::default();
    assert_eq!(rollouts.read(&first_path).activity().text(), "reading");
    let second = rollouts.read(&second_path);
    assert_eq!(second.activity(), AgentActivity::Idle);
    assert!(second.first_prompt.text().starts_with("Review the widget"));
    assert_eq!(
        second.worktree,
        ThreadWorktree::Recorded("/work/widget".into())
    );
    fs::write(&first_path, format!("{other}{CODEX_COMPLETE}")).expect("replacement rollout writes");
    let replaced = rollouts.read(&first_path);
    assert_eq!(replaced.first_prompt, second.first_prompt);
    assert_eq!(replaced.worktree, second.worktree);
    assert_eq!(replaced.activity(), AgentActivity::Idle);
}

#[test]
fn incremental_sparse_tail_finds_the_last_tool_before_neutral_output() {
    let directory = tempfile::tempdir().expect("isolated conversation directory");
    let neutral = "{\"type\":\"progress\",\"padding\":\"tool output without an activity event\"}\n";
    let suffix_bytes = usize::try_from(2 * TRANSCRIPT_TAIL_START).expect("small suffix");
    let suffix = neutral.repeat(suffix_bytes / neutral.len() + 1);
    let claude = format!("{}{suffix}", large_conversation(CLAUDE_HEAD, CLAUDE_TAIL));
    let claude_path = directory.path().join("claude.jsonl");
    fs::write(&claude_path, &claude).expect("sparse Claude transcript writes");
    let mut transcript = TranscriptRead::new(AgentActivity::Idle);
    transcript.advance(&claude_path, AgentActivity::Idle);
    assert_eq!(
        transcript.activity(),
        whole_activity(&format!("{CLAUDE_HEAD}{CLAUDE_TAIL}"), Family::Claude)
    );
    assert_eq!(transcript.activity().text(), "reading");
    assert!(transcript.bytes_read() <= 4 * TRANSCRIPT_TAIL_START);
    assert!(transcript.bytes_read() < u64::try_from(claude.len()).expect("file length fits"));

    let recent = format!("{DEFAULT_TIER}{CODEX_TAIL}");
    let codex = format!("{}{suffix}", large_conversation(CODEX_HEAD, &recent));
    let codex_path = directory.path().join("codex.jsonl");
    fs::write(&codex_path, &codex).expect("sparse Codex rollout writes");
    let read = Rollouts::default().read(&codex_path);
    assert_eq!(
        read.activity(),
        whole_activity(&format!("{CODEX_HEAD}{CODEX_TAIL}"), Family::Codex)
    );
    assert_eq!(read.activity().text(), "reading");
    assert!(read.bytes_read() <= 5 * TRANSCRIPT_TAIL_START);
    assert!(read.bytes_read() < u64::try_from(codex.len()).expect("file length fits"));
}

const DEFAULT_TIER: &str = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"service_tier\":\"default\"}}}\n";

fn neutral_records(minimum_bytes: usize) -> String {
    let record = format!(
        "{{\"type\":\"progress\",\"padding\":\"{}\"}}\n",
        "x".repeat(32 * 1024)
    );
    record.repeat(minimum_bytes.div_ceil(record.len()))
}

#[test]
fn incremental_codex_last_tier_after_the_old_head_limit_replaces_priority() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let path = directory.path().join("late-tier.jsonl");
    let text = format!(
        "{CODEX_HEAD}{}{DEFAULT_TIER}{}{CODEX_TAIL}",
        neutral_records(5 * MEBIBYTE),
        neutral_records(MEBIBYTE)
    );
    fs::write(&path, &text).expect("late tier rollout writes");
    let read = Rollouts::default().read(&path);
    assert_eq!(rollout_tier(read), ServiceTier::Standard);
}

#[test]
fn incremental_codex_first_task_after_large_injected_context_precedes_tail_followup() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let path = directory.path().join("late-task.jsonl");
    let metadata = CODEX_HEAD.lines().next().expect("session metadata");
    let prompt = "Repair the task after the injected context";
    // One record puts the task beyond the old head limit without JSON setup work.
    let text = format!(
        "{metadata}\n{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions\\n{}\"}}]}}}}\n\
         {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{prompt}\"}}]}}}}\n\
         {}{DEFAULT_TIER}{CODEX_TAIL}",
        "x".repeat(4 * MEBIBYTE + 1),
        neutral_records(LARGE_FILE_BYTES)
    );
    fs::write(&path, text).expect("late task rollout writes");
    let read = Rollouts::default().read(&path);
    assert_eq!(read.first_prompt.text(), prompt);
    assert_eq!(read.activity().text(), "reading");
}

#[test]
fn incremental_claude_last_tool_survives_multiple_backward_growth_steps() {
    let directory = tempfile::tempdir().expect("isolated transcript directory");
    let path = directory.path().join("long-neutral-claude.jsonl");
    let text = format!(
        "{CLAUDE_HEAD}{CLAUDE_TAIL}{}",
        neutral_records(LARGE_FILE_BYTES)
    );
    fs::write(&path, &text).expect("long neutral transcript writes");
    let thinking = AgentActivity::Active(Activity::Thinking);
    let mut read = TranscriptRead::new(thinking);
    read.advance(&path, thinking);
    assert_eq!(
        read.activity(),
        whole_activity(&format!("{CLAUDE_HEAD}{CLAUDE_TAIL}"), Family::Claude)
    );
    assert_eq!(read.activity().text(), "reading");
}

#[test]
fn incremental_codex_last_tool_survives_multiple_backward_growth_steps() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let path = directory.path().join("long-neutral-codex.jsonl");
    let text = format!(
        "{CODEX_HEAD}{CODEX_TAIL}{}",
        neutral_records(LARGE_FILE_BYTES)
    );
    fs::write(&path, &text).expect("long neutral rollout writes");
    let read = Rollouts::default().read(&path);
    assert_eq!(
        read.activity(),
        whole_activity(&format!("{CODEX_HEAD}{CODEX_TAIL}"), Family::Codex)
    );
    assert_eq!(read.activity().text(), "reading");
}

#[test]
fn incremental_codex_early_metadata_does_not_fetch_a_four_mebibyte_head() {
    let directory = tempfile::tempdir().expect("isolated rollout directory");
    let path = directory.path().join("early-metadata.jsonl");
    let text = format!(
        "{CODEX_HEAD}{}{DEFAULT_TIER}{CODEX_TAIL}",
        neutral_records(LARGE_FILE_BYTES)
    );
    fs::write(&path, text).expect("early metadata rollout writes");
    let read = Rollouts::default().read(&path);
    assert!(read.first_prompt.text().starts_with("Repair the portal"));
    assert_eq!(
        read.worktree,
        ThreadWorktree::Recorded("/work/fixture".into())
    );
    assert_eq!(read.activity().text(), "reading");
    assert!(
        read.bytes_read() < 4 * TRANSCRIPT_TAIL_START,
        "early metadata and recent activity need far less than the old head budget: {} bytes",
        read.bytes_read()
    );
}
