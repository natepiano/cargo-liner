//! Built-binary acceptance tests for the public Claude hook verbs.
//!
//! Each test drives a raw harness payload through `cargo-berth hook <event>` and reads
//! only what the harness reads back: the process exit status, and the response object
//! the verb writes. The frozen front-end corpus supplies the text these comparisons are
//! held to, so a change to what a user is told fails here rather than reaching a user.

mod support;

use std::error::Error;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use serde_json::Value;
use tempfile::TempDir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const EVIDENCE_SESSION: &str = "evidence-session";
const AMBIENT_SESSION: &str = "ambient-session";
const AMBIENT_STALE_SESSION: &str = "ambient-stale-session";
const BOARD_SESSION: &str = "board-session";
const INCURSION_SESSION: &str = "incursion-session";
const ORPHAN_SESSION_START_ENTRY: &str = "test_session_start_renders_real_orphan_recovery_actions";
const POST_TOOL_USE_EVENT: &str = "PostToolUse";
const POST_TOOL_USE_LOST_EVIDENCE_REWRITTEN_ENTRY: &str =
    "test_hooks_render_both_lost_evidence_recoveries";
const POST_TOOL_USE_LOST_EVIDENCE_UNRESOLVABLE_ENTRY: &str =
    "test_hooks_render_both_lost_evidence_recoveries#3";
const POST_TOOL_USE_RECORDED_INCURSION_LOST_EVIDENCE_ENTRY: &str =
    "test_recorded_incursion_preserves_lost_evidence_feedback";
const POST_TOOL_USE_RECORDED_INCURSION_SILENT_ENTRY: &str =
    "test_recorded_incursion_emits_no_stop_text";
const POST_TOOL_USE_RECORDED_INCURSION_WIDENED_ENTRY: &str =
    "test_recorded_incursion_preserves_concurrent_widening_feedback";
const POST_TOOL_USE_EVERY_INCURSION_ENTRY: &str = "test_incursion_board_read_cost_is_constant#2";
const POST_TOOL_USE_INCURSION_ENTRY: &str = "test_incursion_board_read_cost_is_constant";
const POST_TOOL_USE_OUTSTANDING_INCURSION_ENTRY: &str =
    "test_outstanding_incursion_emits_stop_text";
const POST_TOOL_USE_REPLAY_ENTRY: &str =
    "test_typed_replay_failure_routes_without_message_in_every_consumer#2";
const POST_TOOL_USE_SILENT_CLEAR_ENTRY: &str =
    "test_a_named_widening_with_nothing_to_report_still_says_nothing";
const POST_TOOL_USE_SILENT_INCURSION_FREE_ENTRY: &str =
    "test_incursion_board_read_cost_is_constant#3";
const POST_TOOL_USE_STALE_SESSION_ENTRY: &str =
    "test_hooks_render_coordination_identity_recovery_actions_without_message#2";
const POST_TOOL_USE_WIDENED_ENTRY: &str = "test_incursion_board_read_cost_is_constant#4";
const QUIET_DRIFT_SESSION: &str = "quiet-drift-session";
const REPLAY_DRIFT_SESSION: &str = "replay-drift-session";
const SESSION_START_EVENT: &str = "SessionStart";
const SESSION_START_CONTENTION_DETAIL: &str = "The engine already spent its single 10-second retry budget; the hook did not invoke board \
     again. Run `cargo-berth board --json` when the ledger is free.";
const SESSION_START_CONTENTION_SUMMARY: &str =
    "cargo-berth exhausted its ledger-lock deadline at SessionStart.";
const SESSION_START_LEDGER_UNREADABLE_DETAIL: &str =
    "Run `cargo-berth board --json` again after repairing the ledger.";
const SESSION_START_LEDGER_UNREADABLE_SUMMARY: &str =
    "cargo-berth could not read the reservation ledger at SessionStart.";
const SESSION_START_LOST_EVIDENCE_REWRITTEN_ENTRY: &str =
    "test_hooks_render_both_lost_evidence_recoveries#2";
const SESSION_START_LOST_EVIDENCE_UNRESOLVABLE_ENTRY: &str =
    "test_hooks_render_both_lost_evidence_recoveries#4";
const SESSION_START_UNAVAILABLE_ORPHAN_ENTRY: &str =
    "test_session_start_renders_real_orphan_recovery_actions#2";
const SESSION_START_REPLAY_ENTRY: &str =
    "test_typed_replay_failure_routes_without_message_in_every_consumer#3";
const STALE_DRIFT_SESSION: &str = "stale-drift-session";
const UNCONFIGURED_DRIFT_SESSION: &str = "unconfigured-drift-session";
const UNMAPPED_DRIFT_SESSION: &str = "unmapped-drift-session";
const UNREADABLE_DRIFT_SESSION: &str = "unreadable-drift-session";
const WIDENING_SESSION: &str = "widening-session";
const AMBIGUITY_SESSION: &str = "ambiguous-session";
const COORDINATION_IDENTITY_EDIT_SUMMARY: &str =
    "cargo-berth rejected this edit under the current coordination identity.";
/// The rejection kind a reservation whose session mapping outlived it is refused under.
const STALE_SESSION_REJECTION_KIND: &str = "stale_session_mapping";
const CORRUPT_JOURNAL_RECORD: &[u8] = b"this journal record is not JSON\n";
const FAIL_OPEN_SENTENCE: &str = "cargo-berth could not establish edit safety; editing is allowed because ledger loss fails open.";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const FRONT_END_CORPUS_JSON: &str = include_str!("fixtures/front_end_corpus.json");
const GIT_BINARY: &str = "git";
const HOLDING_WORKTREE_FIXTURE_ROOT: &str = "{FIXTURE_ROOT}/holding worktree";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_CONTENTION_SUMMARY: &str = "cargo-berth rejected this edit because another cargo-berth operation still holds the ledger lock.";
const MUTATION_LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const PAUSED_GIT_TIMEOUT: Duration = Duration::from_secs(30);
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
/// The summary `hook post-tool-use` states for a payload it cannot read.
const POST_TOOL_USE_INVALID_PAYLOAD_SUMMARY: &str =
    "cargo-berth rejected an invalid PostToolUse payload.";
/// The detail `hook post-tool-use` states for a payload it cannot read.
const POST_TOOL_USE_INVALID_PAYLOAD_DETAIL: &str = "STOP: `cargo-berth hook post-tool-use` requires valid JSON, tool_name Bash, a session_id of 1 to 256 characters with no control characters, and a cwd that is a string when it is present. Run `cargo-berth drift --reservation <id> --json` by hand.";
/// The summary `hook post-tool-use` states for a working directory it cannot enter.
const POST_TOOL_USE_UNAVAILABLE_WORKING_DIRECTORY_SUMMARY: &str =
    "cargo-berth could not inspect this Bash call.";
/// The detail `hook post-tool-use` states for a working directory it cannot enter.
const POST_TOOL_USE_UNAVAILABLE_WORKING_DIRECTORY_DETAIL: &str =
    "STOP: the hook working directory does not exist or is unavailable.";
const REPOSITORY_FIXTURE_ROOT: &str = "{FIXTURE_ROOT}/repository";
const RESERVATION_LIMIT_SUMMARY: &str =
    "cargo-berth rejected this edit because it could not accept the request.";
const RETIRED_MISSING_PRESENTATION_DIAGNOSTIC: &str =
    "the engine returned a blocking check answer without a presentation";
const SCRATCH_ROOT: &str = "/tmp/claude";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const STALE_SESSION_RECOVERY_ENTRY: &str =
    "test_hooks_render_coordination_identity_recovery_actions_without_message";
const STALE_MARKER_RECOVERY_ENTRY: &str =
    "test_hooks_render_coordination_identity_recovery_actions_without_message#3";
const TWO_ACTION_RECOVERY_ENTRY: &str =
    "test_hooks_render_coordination_identity_recovery_actions_without_message#5";
const UNKNOWN_RESERVATION_ID: &str = "01991f4d-77d8-7f5f-9a1f-000000000001";
/// The unclaimed source a straying worktree widens onto after its incursion is answered.
const WIDENED_AFTER_INCURSION_SOURCE: &str = "widened.rs";
/// The reservation identifier the corpus froze for every lost-evidence entry.
const FROZEN_LOST_EVIDENCE_RESERVATION: &str = "reservation-lost-evidence";
/// The protected tip the corpus froze for every lost-evidence entry.
const FROZEN_LOST_EVIDENCE_PROTECTED_TIP: &str = "1111111111111111111111111111111111111111";
/// The rewritten trunk commit the corpus froze for the resolved-trunk recovery.
const FROZEN_LOST_EVIDENCE_TRUNK: &str = "2222222222222222222222222222222222222222";
/// A well-formed object identifier no repository in this suite ever writes an object for.
const ABSENT_TRUNK_OBJECT_ID: &str = "3333333333333333333333333333333333333333";
/// The trunk branch every fixture repository configures.
const TRUNK_BRANCH: &str = "main";
/// The loose reference file the trunk branch is published at.
const TRUNK_REFERENCE_PATH: &str = ".git/refs/heads/main";

/// The harness event one hook response answers, and the continuation field it states.
///
/// The two events state different response objects, and the difference is a contract:
/// `berth_post_bash.sh` reports `continue`, and `berth_session_start.sh` deliberately
/// reports none, because a session-start response cannot stop anything the harness is
/// already going to do. Naming the event rather than passing its bare string keeps that
/// difference on one type instead of at every comparison.
#[derive(Clone, Copy)]
enum HookResponseEvent {
    /// A `PostToolUse` response, which states that the session continues.
    PostToolUse,
    /// A `SessionStart` response, which states no continuation field at all.
    SessionStart,
}

/// How completely one produced response has to account for its frozen corpus text.
enum FrozenTextCoverage {
    /// The produced lines are exactly the frozen lines and carry nothing else.
    ExactlyTheFrozenLines,
    /// The produced lines carry the frozen lines inside the engine's wider report.
    TheFrozenLinesInsideTheReport,
}

impl HookResponseEvent {
    const fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => POST_TOOL_USE_EVENT,
            Self::SessionStart => SESSION_START_EVENT,
        }
    }
}

/// Whether the hook process inherits a harness session identity from its environment.
enum AmbientHarnessSession<'session> {
    /// The environment names a harness session the hook must not adopt.
    Present(&'session str),
    /// The environment names no harness session.
    Absent,
}

/// One token of a recovery argv frozen by the front-end corpus.
enum FrozenArgvToken<'token> {
    /// Shell syntax the corpus left unquoted, such as `cd` and `&&`.
    Literal(&'token str),
    /// One argv word the corpus quoted.
    Quoted(&'token str),
}

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[test]
fn silent_allow_emits_nothing() -> TestResult {
    let repository = initialized_repository()?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(repository.path(), "silent.rs", Some("silent-session")),
    )?;

    assert_hook_output(&output, 0, b"", b"")
}

#[test]
fn allow_with_notice_emits_the_engine_presentation_object() -> TestResult {
    let repository = initialized_repository()?;
    fs::create_dir(repository.path().join(SESSION_MAPPING_PATH))?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(repository.path(), "degraded.rs", Some("degraded-session")),
    )?;

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let notice: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        notice["systemMessage"],
        "cargo-berth authorized this edit and stated the detail below itself."
    );
    assert_eq!(notice["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(notice["hookSpecificOutput"]["permissionDecision"], "allow");
    let reason = required_string(
        &notice,
        "/hookSpecificOutput/permissionDecisionReason",
        "degraded allow notice",
    )?;
    assert!(reason.contains("The harness session mapping could not be published"));
    assert!(reason.contains("Name the coordination run and reservation explicitly"));
    Ok(())
}

#[test]
fn blocked_edit_emits_the_engine_refusal() -> TestResult {
    let repository = initialized_repository()?;
    let holder = run_berth(
        repository.path(),
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&holder, "holder claim")?;
    let (_requester_directory, requester_root) = add_worktree(&repository, "requester")?;
    let output = run_pre_tool_use(
        &requester_root,
        &edit_payload(&requester_root, "src/lib.rs", Some("blocked-session")),
    )?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let refusal = String::from_utf8(output.stderr)?;
    let refusal_detail = refusal
        .strip_prefix(
            "cargo-berth refused this edit because another reservation holds the requested paths.\n\n",
        )
        .ok_or_else(|| failure("blocking presentation should separate its summary and detail"))?;
    assert!(refusal_detail.ends_with('\n'));
    assert!(refusal_detail.contains("Choose exactly one answer for one named holder."));
    assert!(refusal_detail.contains("cargo-berth claim <paths...> --before"));
    Ok(())
}

#[test]
fn a_rejected_request_states_the_engine_message() -> TestResult {
    let repository = initialized_repository()?;
    fs::write(
        repository.path().join(CONFIGURATION_PATH),
        "maximum_reservations = 0\n",
    )?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(
            repository.path(),
            "unpresented.rs",
            Some("unpresented-session"),
        ),
    )?;

    assert_engine_authored_refusal(
        &output,
        RESERVATION_LIMIT_SUMMARY,
        "permits at most 0 live reservations",
        "reservation limit refusal",
    )
}

#[test]
fn a_contended_ledger_states_the_engine_message() -> TestResult {
    let repository = initialized_repository()?;
    let competing_lock = File::options()
        .read(true)
        .write(true)
        .open(repository.path().join(MUTATION_LOCK_PATH))?;
    competing_lock
        .try_lock()
        .map_err(|_| failure("the competing mutation lock should start free"))?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(repository.path(), "contended.rs", Some("contended-session")),
    )?;
    drop(competing_lock);

    assert_engine_authored_refusal(
        &output,
        LOCK_CONTENTION_SUMMARY,
        "another cargo-berth operation is still running",
        "lock contention refusal",
    )
}

#[test]
fn fail_open_notices_state_their_sentence_once() -> TestResult {
    let replay_repository = typed_replay_failure_repository()?;
    let replay = run_pre_tool_use(
        replay_repository.path(),
        &edit_payload(
            replay_repository.path(),
            "replay.rs",
            Some("replay-sentence-session"),
        ),
    )?;
    assert_fail_open_sentence_stated_once(&replay, "replay failure fail-open notice")?;

    let unreadable_repository = unreadable_ledger_repository()?;
    let unreadable = run_pre_tool_use(
        unreadable_repository.path(),
        &edit_payload(
            unreadable_repository.path(),
            "unreadable.rs",
            Some("unreadable-sentence-session"),
        ),
    )?;
    assert_fail_open_sentence_stated_once(&unreadable, "unreadable ledger fail-open notice")
}

#[test]
fn a_payload_without_session_identity_ignores_the_ambient_variable() -> TestResult {
    let repository = initialized_repository()?;
    let older = run_berth_with_session(
        repository.path(),
        &["claim", "tree:shared", "--json"],
        AMBIENT_SESSION,
    )?;
    require_success(&older, "older ambient-session candidate")?;
    let newer = run_berth_with_session(
        repository.path(),
        &["claim", "file:shared/child.rs", "--json"],
        AMBIENT_SESSION,
    )?;
    require_success(&newer, "newer ambient-session candidate")?;

    let output = spawn_pre_tool_use(
        repository.path(),
        &edit_payload(repository.path(), "shared/child.rs", None),
        &AmbientHarnessSession::Present(AMBIENT_SESSION),
    )?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let refusal = String::from_utf8(output.stderr)?;
    assert!(
        refusal.contains("could not select one active reservation for this edit"),
        "a payload naming no harness session adopted the ambient one: {refusal}"
    );
    Ok(())
}

#[test]
fn an_edit_through_a_symlinked_directory_is_still_checked() -> TestResult {
    let repository = initialized_repository()?;
    fs::create_dir(repository.path().join("real"))?;
    let holder = run_berth(
        repository.path(),
        &["claim", "file:real/held.rs", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&holder, "symlinked directory holder")?;
    let (_requester_directory, requester_root) = add_worktree(&repository, "symlink-requester")?;
    fs::create_dir(requester_root.join("real"))?;
    std::os::unix::fs::symlink("real", requester_root.join("alias"))?;

    let output = run_pre_tool_use(
        &requester_root,
        &edit_payload(&requester_root, "alias/held.rs", Some("symlink-session")),
    )?;

    assert_refused_for_scope(&output, "file:real/held.rs", "a symlinked directory edit")
}

#[test]
fn an_edit_through_a_symlink_leaving_the_worktree_keeps_its_worktree_name() -> TestResult {
    let repository = initialized_repository()?;
    let holder = run_berth(
        repository.path(),
        &["claim", "file:linked/held.rs", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&holder, "escaping symlink holder")?;
    let (_requester_directory, requester_root) = add_worktree(&repository, "escape-requester")?;
    let escape_directory = TempDir::new_in(SCRATCH_ROOT)?;
    std::os::unix::fs::symlink(escape_directory.path(), requester_root.join("linked"))?;

    let output = run_pre_tool_use(
        &requester_root,
        &edit_payload(&requester_root, "linked/held.rs", Some("escape-session")),
    )?;

    assert_refused_for_scope(
        &output,
        "file:linked/held.rs",
        "an edit through a symlink leaving the worktree",
    )
}

#[test]
fn a_parent_component_resolves_against_the_filesystem_not_the_path_string() -> TestResult {
    let repository = initialized_repository()?;
    let holder = run_berth(
        repository.path(),
        &["claim", "file:held.rs", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&holder, "parent component holder")?;
    let (_requester_directory, requester_root) = add_worktree(&repository, "parent-requester")?;
    fs::create_dir(requester_root.join("real"))?;
    let escape_directory = TempDir::new_in(SCRATCH_ROOT)?;
    let escaped_child = escape_directory.path().join("nested");
    fs::create_dir(&escaped_child)?;
    std::os::unix::fs::symlink(&escaped_child, requester_root.join("alias"))?;

    let behind_a_real_directory = run_pre_tool_use(
        &requester_root,
        &edit_payload(
            &requester_root,
            "real/../held.rs",
            Some("real-parent-session"),
        ),
    )?;
    assert_refused_for_scope(
        &behind_a_real_directory,
        "file:held.rs",
        "a parent component behind a real directory",
    )?;
    assert_no_parent_component_was_claimed(&behind_a_real_directory)?;

    let behind_an_escaping_symlink = run_pre_tool_use(
        &requester_root,
        &edit_payload(
            &requester_root,
            "alias/../held.rs",
            Some("escaping-parent-session"),
        ),
    )?;
    assert_no_parent_component_was_claimed(&behind_an_escaping_symlink)?;
    assert_no_scope_was_claimed(
        &behind_an_escaping_symlink,
        "file:held.rs",
        "a parent component behind a symlink leaving the worktree",
    )?;
    assert_hook_output(&behind_an_escaping_symlink, 0, b"", b"")?;

    let behind_an_absent_directory = run_pre_tool_use(
        &requester_root,
        &edit_payload(
            &requester_root,
            "absent/../held.rs",
            Some("absent-parent-session"),
        ),
    )?;
    assert_refusal_states(
        &behind_an_absent_directory,
        "no existing ancestor of the edit target could be resolved",
        "a parent component behind a directory that does not exist",
    )?;
    assert_no_parent_component_was_claimed(&behind_an_absent_directory)
}

#[test]
fn an_edit_named_from_a_nested_working_directory_keeps_its_repository_relative_name() -> TestResult
{
    let repository = initialized_repository()?;
    for scope in ["file:sub/held.rs", "file:sub/alias/held.rs"] {
        let holder = run_berth(
            repository.path(),
            &["claim", scope, "--run", FIRST_RUN, "--json"],
        )?;
        require_success(&holder, "nested working directory holder")?;
    }
    let (_requester_directory, requester_root) = add_worktree(&repository, "nested-requester")?;
    let nested_directory = requester_root.join("sub");
    fs::create_dir(&nested_directory)?;
    let escape_directory = TempDir::new_in(SCRATCH_ROOT)?;
    let escaped_child = escape_directory.path().join("nested");
    fs::create_dir(&escaped_child)?;
    std::os::unix::fs::symlink(&escaped_child, nested_directory.join("alias"))?;

    let plain = run_pre_tool_use(
        &nested_directory,
        &edit_payload(&nested_directory, "held.rs", Some("nested-plain-session")),
    )?;
    assert_refused_for_scope(
        &plain,
        "file:sub/held.rs",
        "an edit named from a nested directory",
    )?;

    let through_escaping_symlink = run_pre_tool_use(
        &nested_directory,
        &edit_payload(
            &nested_directory,
            "alias/held.rs",
            Some("nested-symlink-session"),
        ),
    )?;
    assert_refused_for_scope(
        &through_escaping_symlink,
        "file:sub/alias/held.rs",
        "a nested edit through a symlink leaving the worktree",
    )?;

    let behind_an_escaping_parent = run_pre_tool_use(
        &nested_directory,
        &edit_payload(
            &nested_directory,
            "alias/../held.rs",
            Some("nested-parent-session"),
        ),
    )?;
    assert_no_parent_component_was_claimed(&behind_an_escaping_parent)?;
    assert_hook_output(&behind_an_escaping_parent, 0, b"", b"")
}

#[test]
fn a_relative_edit_target_is_refused_for_not_being_absolute() -> TestResult {
    let repository = initialized_repository()?;
    let output = run_pre_tool_use(
        repository.path(),
        &serde_json::json!({
            "tool_name": "Edit",
            "cwd": repository.path(),
            "tool_input": {"file_path": "relative/target.rs"},
            "session_id": "relative-session",
        }),
    )?;

    assert_refusal_states(
        &output,
        "the edit target must be an absolute path",
        "a relative edit target",
    )
}

#[test]
fn path_normalization_answers_the_same_after_its_move() -> TestResult {
    let repository = initialized_repository()?;
    fs::create_dir(repository.path().join("src"))?;
    for scope in ["file:src/held.rs", "file:absent/deep/held.rs"] {
        let holder = run_berth(
            repository.path(),
            &["claim", scope, "--run", FIRST_RUN, "--json"],
        )?;
        require_success(&holder, "path normalization holder")?;
    }
    let (_requester_directory, requester_root) =
        add_worktree(&repository, "normalization-requester")?;
    fs::create_dir(requester_root.join("src"))?;

    let traversed = run_pre_tool_use(
        &requester_root,
        &edit_payload(
            &requester_root,
            "src/./../src/held.rs",
            Some("traversed-session"),
        ),
    )?;
    assert_refused_for_scope(&traversed, "file:src/held.rs", "a traversed edit path")?;

    let partly_existing = run_pre_tool_use(
        &requester_root,
        &edit_payload(
            &requester_root,
            "absent/deep/held.rs",
            Some("partly-existing-session"),
        ),
    )?;
    assert_refused_for_scope(
        &partly_existing,
        "file:absent/deep/held.rs",
        "an edit under directories that do not exist yet",
    )?;

    let outside = TempDir::new_in(SCRATCH_ROOT)?;
    let outside_edit = run_pre_tool_use(
        &requester_root,
        &serde_json::json!({
            "tool_name": "Edit",
            "cwd": requester_root,
            "tool_input": {"file_path": outside.path().join("outside.rs")},
            "session_id": "outside-session",
        }),
    )?;
    assert_hook_output(&outside_edit, 0, b"", b"")
}

#[test]
fn replay_failure_emits_a_fail_open_object() -> TestResult {
    let repository = typed_replay_failure_repository()?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(repository.path(), "replay.rs", Some("replay-session")),
    )?;

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let notice: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        notice["systemMessage"],
        "cargo-berth could not establish edit safety and stated the detail below itself."
    );
    assert_eq!(notice["hookSpecificOutput"]["permissionDecision"], "allow");
    let reason = required_string(
        &notice,
        "/hookSpecificOutput/permissionDecisionReason",
        "replay failure notice",
    )?;
    let corpus_detail = corpus_expected_permission_reason(
        "test_typed_replay_failure_routes_without_message_in_every_consumer",
    )?;
    assert!(reason.contains(&corpus_detail));
    Ok(())
}

#[test]
fn coordination_identity_rejection_emits_its_recovery() -> TestResult {
    let repository = initialized_repository()?;
    let session_id = "stale-hook-session";
    create_stale_session_mapping(&repository, session_id)?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(repository.path(), "new.rs", Some(session_id)),
    )?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let refusal = String::from_utf8(output.stderr)?;
    assert!(refusal.contains(STALE_SESSION_REJECTION_KIND));
    assert!(refusal.contains("identity"));
    assert!(refusal.contains("clear-session"));
    assert!(
        refusal.starts_with(COORDINATION_IDENTITY_EDIT_SUMMARY),
        "a refused edit should announce itself as a refused edit: {refusal}"
    );
    assert!(
        !refusal.contains("drift"),
        "a refused edit should not reach the user as a drift rejection: {refusal}"
    );
    Ok(())
}

#[test]
fn unconfigured_no_facts_allows_silently() -> TestResult {
    let repository = git_repository()?;
    let output = run_pre_tool_use(
        repository.path(),
        &edit_payload(
            repository.path(),
            "unconfigured.rs",
            Some("no-facts-session"),
        ),
    )?;

    assert_hook_output(&output, 0, b"", b"")
}

#[test]
fn overlap_refusal_from_raw_payload_lists_every_answer_command() -> TestResult {
    let repository = initialized_repository()?;
    let holder = run_berth(
        repository.path(),
        &["claim", "file:shared.rs", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&holder, "overlap answer holder")?;
    let (_requester_directory, requester_root) = add_worktree(&repository, "answer-requester")?;
    let output = run_pre_tool_use(
        &requester_root,
        &edit_payload(&requester_root, "shared.rs", Some("answer-session")),
    )?;

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let refusal = String::from_utf8(output.stderr)?;
    for expected in [
        "cargo-berth claim <paths...> --before <holder-reservation-id>",
        "cargo-berth claim <paths...> --after <holder-reservation-id>",
        "cargo-berth claim <paths...> --defer <holder-reservation-id>",
        "cargo-berth claim <paths...> --override <holder-reservation-id>",
        "Leave it alone.",
    ] {
        assert!(
            refusal.contains(expected),
            "missing answer {expected:?}: {refusal}"
        );
    }
    Ok(())
}

#[test]
fn ambiguity_and_replay_preserve_the_frozen_corpus_text() -> TestResult {
    let ambiguity = ambiguous_first_touch_hook()?;
    assert_eq!(ambiguity.output.status.code(), Some(2));
    assert!(ambiguity.output.stdout.is_empty());
    let mut ambiguity_stderr = String::from_utf8(ambiguity.output.stderr)?;
    for (actual, expected) in ambiguity
        .actual_reservation_ids
        .iter()
        .zip(ambiguity.corpus_reservation_ids.iter())
    {
        ambiguity_stderr = ambiguity_stderr.replace(actual, expected);
    }
    let ambiguity_expected = corpus_expected_stderr(
        "test_pre_edit_renders_an_ambiguous_first_touch_from_the_engine_message",
    )?;
    assert!(
        ambiguity_stderr.contains(ambiguity_expected.trim_end()),
        "ambiguity output changed frozen engine text: {ambiguity_stderr}"
    );

    let replay_repository = typed_replay_failure_repository()?;
    let replay = run_pre_tool_use(
        replay_repository.path(),
        &corpus_edit_payload(
            "test_typed_replay_failure_routes_without_message_in_every_consumer",
            replay_repository.path(),
            "corpus-replay.rs",
            Some("corpus-replay"),
        )?,
    )?;
    let replay_notice: Value = serde_json::from_slice(&replay.stdout)?;
    let replay_reason = required_string(
        &replay_notice,
        "/hookSpecificOutput/permissionDecisionReason",
        "corpus replay notice",
    )?;
    let replay_expected = corpus_expected_permission_reason(
        "test_typed_replay_failure_routes_without_message_in_every_consumer",
    )?;
    assert!(replay_reason.contains(&replay_expected));
    Ok(())
}

#[test]
fn session_identity_recoveries_preserve_the_frozen_corpus_text() -> TestResult {
    let stale_repository = initialized_repository()?;
    let stale_session = "corpus-stale-session";
    create_stale_session_mapping(&stale_repository, stale_session)?;
    let stale = run_pre_tool_use(
        stale_repository.path(),
        &corpus_edit_payload(
            STALE_SESSION_RECOVERY_ENTRY,
            stale_repository.path(),
            "new.rs",
            Some(stale_session),
        )?,
    )?;
    let stale_root = fs::canonicalize(stale_repository.path())?;
    assert_pre_edit_corpus_recovery(
        &stale,
        STALE_SESSION_RECOVERY_ENTRY,
        &[(REPOSITORY_FIXTURE_ROOT, &stale_root)],
    )?;

    let (two_actions, holding_root, issuing_root) = session_worktree_mismatch()?;
    assert_pre_edit_corpus_recovery(
        &two_actions,
        TWO_ACTION_RECOVERY_ENTRY,
        &[
            (HOLDING_WORKTREE_FIXTURE_ROOT, &holding_root),
            (REPOSITORY_FIXTURE_ROOT, &issuing_root),
        ],
    )
}

#[test]
fn stale_marker_recovery_preserves_the_frozen_corpus_text() -> TestResult {
    let repository = initialized_repository()?;
    let marker_seed = run_berth(
        repository.path(),
        &["claim", "file:marker-seed.rs", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&marker_seed, "marker seed claim")?;
    let marker_seed_envelope = json_output(&marker_seed)?;
    let marker_seed_id = required_string(
        &marker_seed_envelope,
        "/payload/data/reservation_id",
        "marker seed claim",
    )?;
    let payload = corpus_edit_payload(
        STALE_MARKER_RECOVERY_ENTRY,
        repository.path(),
        "marker-race.rs",
        None,
    )?;
    let mut paused = PausedPreToolUse::spawn(repository.path(), &payload)?;
    paused.wait_until_paused()?;
    let released = run_berth(repository.path(), &["release", marker_seed_id, "--json"])?;
    require_success(&released, "marker seed release")?;
    let output = paused.continue_and_wait()?;

    let repository_root = fs::canonicalize(repository.path())?;
    assert_pre_edit_corpus_recovery(
        &output,
        STALE_MARKER_RECOVERY_ENTRY,
        &[(REPOSITORY_FIXTURE_ROOT, &repository_root)],
    )
}

struct PausedPreToolUse {
    child:             Child,
    continue_path:     PathBuf,
    ready_path:        PathBuf,
    wrapper_directory: TempDir,
}

impl PausedPreToolUse {
    fn spawn(repository_root: &Path, payload: &Value) -> TestResult<Self> {
        let wrapper_directory = TempDir::new_in(SCRATCH_ROOT)?;
        let wrapper_path = wrapper_directory.path().join(GIT_BINARY);
        fs::write(&wrapper_path, paused_git_wrapper())?;
        let mut permissions = fs::metadata(&wrapper_path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&wrapper_path, permissions)?;
        let ready_path = wrapper_directory.path().join("ready");
        let continue_path = wrapper_directory.path().join("continue");
        let original_path =
            std::env::var_os("PATH").ok_or_else(|| failure("test process should supply PATH"))?;
        let wrapped_path = std::env::join_paths(
            std::iter::once(wrapper_directory.path().to_path_buf())
                .chain(std::env::split_paths(&original_path)),
        )?;
        let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
            .args(["hook", "pre-tool-use"])
            .current_dir(repository_root)
            .env("PATH", wrapped_path)
            .env("CARGO_BERTH_TEST_GIT_READY", &ready_path)
            .env("CARGO_BERTH_TEST_GIT_CONTINUE", &continue_path)
            .env("CARGO_BERTH_TEST_REAL_GIT", git_binary()?)
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_SESSION_ID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| failure("paused pre-tool-use stdin should be piped"))?;
        serde_json::to_writer(&mut stdin, payload)?;
        drop(stdin);
        Ok(Self {
            child,
            continue_path,
            ready_path,
            wrapper_directory,
        })
    }

    fn wait_until_paused(&mut self) -> TestResult {
        let deadline = Instant::now() + PAUSED_GIT_TIMEOUT;
        while !self.ready_path.exists() && Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                return Err(failure(format!(
                    "pre-tool-use exited with {status} before the git pause"
                )));
            }
            thread::sleep(Duration::from_millis(10));
        }
        if self.ready_path.exists() {
            return Ok(());
        }
        self.child.kill()?;
        self.child.wait()?;
        Err(failure("pre-tool-use did not reach the git pause"))
    }

    fn continue_and_wait(self) -> TestResult<Output> {
        let Self {
            child,
            continue_path,
            ready_path: _,
            wrapper_directory,
        } = self;
        fs::write(continue_path, b"continue\n")?;
        let output = child.wait_with_output()?;
        drop(wrapper_directory);
        Ok(output)
    }
}

const fn paused_git_wrapper() -> &'static str {
    r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "rev-parse" ] && [ "$3" = "HEAD" ]; then
    : > "$CARGO_BERTH_TEST_GIT_READY"
    while [ ! -e "$CARGO_BERTH_TEST_GIT_CONTINUE" ]; do
        sleep 0.01
    done
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#
}

fn git_binary() -> TestResult<String> {
    let output = Command::new("sh").args(["-c", "command -v git"]).output()?;
    require_success(&output, "git path lookup")?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

struct AmbiguityHookOutput {
    output:                 Output,
    actual_reservation_ids: Vec<String>,
    corpus_reservation_ids: Vec<String>,
}

fn ambiguous_first_touch_hook() -> TestResult<AmbiguityHookOutput> {
    let repository = initialized_repository()?;
    let older = run_berth_with_session(
        repository.path(),
        &["claim", "tree:shared", "--json"],
        AMBIGUITY_SESSION,
    )?;
    require_success(&older, "older first-touch candidate")?;
    let newer = run_berth_with_session(
        repository.path(),
        &["claim", "file:shared/child.rs", "--json"],
        AMBIGUITY_SESSION,
    )?;
    require_success(&newer, "newer first-touch candidate")?;
    if repository.path().join(SESSION_MAPPING_PATH).exists() {
        fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))?;
    }
    let output = run_pre_tool_use(
        repository.path(),
        &corpus_edit_payload(
            "test_pre_edit_renders_an_ambiguous_first_touch_from_the_engine_message",
            repository.path(),
            "shared/child.rs",
            Some(AMBIGUITY_SESSION),
        )?,
    )?;
    let actual_reservation_ids = [older, newer]
        .iter()
        .map(json_output)
        .collect::<TestResult<Vec<_>>>()?
        .iter()
        .map(|envelope| required_string(envelope, "/reservations/0", "first-touch candidate"))
        .map(|reservation_id| reservation_id.map(str::to_owned))
        .collect::<TestResult<Vec<_>>>()?;
    let corpus =
        corpus_entry("test_pre_edit_renders_an_ambiguous_first_touch_from_the_engine_message")?;
    let corpus_reservation_ids = required_array(
        corpus,
        "/engine_responses/check/body/reservations",
        "ambiguity corpus entry",
    )?
    .iter()
    .map(|reservation_id| {
        reservation_id
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| failure("corpus reservation id should be text"))
    })
    .collect::<TestResult<Vec<_>>>()?;
    Ok(AmbiguityHookOutput {
        output,
        actual_reservation_ids,
        corpus_reservation_ids,
    })
}

/// Drive one session/worktree mismatch, keeping both roots readable after the run.
fn session_worktree_mismatch() -> TestResult<(Output, PathBuf, PathBuf)> {
    let repository = initialized_repository()?;
    let session_id = "covered-mismatch-session";
    let holder = run_berth_with_session(
        repository.path(),
        &["claim", "file:source file.rs", "--run", FIRST_RUN, "--json"],
        session_id,
    )?;
    require_success(&holder, "session mismatch holder")?;
    let (_issuing_directory, issuing_root) = add_worktree(&repository, "issuing")?;
    let output = run_pre_tool_use(
        &issuing_root,
        &corpus_edit_payload(
            TWO_ACTION_RECOVERY_ENTRY,
            &issuing_root,
            "source file.rs",
            Some(session_id),
        )?,
    )?;
    let holding_root = fs::canonicalize(repository.path())?;
    let issuing_root = fs::canonicalize(issuing_root)?;
    Ok((output, holding_root, issuing_root))
}

fn create_stale_session_mapping(repository: &TempDir, session_id: &str) -> TestResult {
    let mapped = run_berth_with_session(
        repository.path(),
        &["claim", "file:mapped.rs", "--run", FIRST_RUN, "--json"],
        session_id,
    )?;
    require_success(&mapped, "mapped claim")?;
    let envelope = json_output(&mapped)?;
    let reservation_id =
        required_string(&envelope, "/payload/data/reservation_id", "mapped claim")?;
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path)?;
    let released = run_berth(repository.path(), &["release", reservation_id, "--json"])?;
    require_success(&released, "mapped reservation release")?;
    fs::write(mapping_path, stale_mapping)?;
    Ok(())
}

fn typed_replay_failure_repository() -> TestResult<TempDir> {
    let repository = initialized_repository()?;
    let seed = run_berth(
        repository.path(),
        &["claim", "file:seed.rs", "--run", FIRST_RUN, "--json"],
    )?;
    require_success(&seed, "replay fixture seed")?;
    append_unknown_release(repository.path())?;
    let projection_path = repository.path().join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path)?;
    }
    Ok(repository)
}

fn unreadable_ledger_repository() -> TestResult<TempDir> {
    let repository = initialized_repository()?;
    let mut journal = OpenOptions::new()
        .append(true)
        .open(repository.path().join(JOURNAL_PATH))?;
    journal.write_all(CORRUPT_JOURNAL_RECORD)?;
    let projection_path = repository.path().join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path)?;
    }
    Ok(repository)
}

fn append_unknown_release(repository_root: &Path) -> TestResult {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let journal = fs::read_to_string(&journal_path)?;
    let previous: Value = serde_json::from_str(
        journal
            .lines()
            .last()
            .ok_or_else(|| failure("replay fixture should have one journal event"))?,
    )?;
    let generation = previous["projection_generation"]
        .as_u64()
        .ok_or_else(|| failure("journal generation should be numeric"))?
        + 1;
    let event = serde_json::json!({
        "schema_version": 1,
        "event_id": uuid::Uuid::now_v7().to_string(),
        "actor": previous["actor"],
        "at": previous["at"],
        "projection_generation": generation,
        "op": "release",
        "reservation_id": UNKNOWN_RESERVATION_ID,
        "disposition": {"kind": "integrated"},
    });
    let mut journal = OpenOptions::new().append(true).open(journal_path)?;
    serde_json::to_writer(&mut journal, &event)?;
    journal.write_all(b"\n")?;
    Ok(())
}

/// Compare one pre-edit corpus entry's frozen recovery against real hook output.
///
/// Only the rejection kind and the recovery actions are a contract the corpus and
/// this engine share. The corpus also froze the retired shell front end's heading
/// sentence, which the engine now writes for itself, so the headings are
/// deliberately not compared here; `coordination_identity_rejection_emits_its_recovery`
/// covers the sentence the engine states instead.
fn assert_pre_edit_corpus_recovery(
    output: &Output,
    corpus_entry_name: &str,
    fixture_roots: &[(&str, &Path)],
) -> TestResult {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let actual = String::from_utf8(output.stderr.clone())?;
    let frozen = corpus_expected_stderr(corpus_entry_name)?;
    let rejection_kind = frozen_rejection_kind(&frozen, corpus_entry_name)?;
    if !actual.contains(rejection_kind) {
        return Err(failure(format!(
            "{corpus_entry_name} should name rejection kind {rejection_kind:?}: {actual:?}"
        )));
    }
    let expected_actions = frozen_recovery_actions(&frozen, fixture_roots);
    if expected_actions.is_empty() {
        return Err(failure(format!(
            "{corpus_entry_name} should freeze at least one recovery action"
        )));
    }
    let expected_action_list = expected_actions
        .iter()
        .map(|action| format!("`{action}`"))
        .collect::<Vec<_>>()
        .join(" or ");
    if actual.contains(&expected_action_list) {
        return Ok(());
    }
    Err(failure(format!(
        "{corpus_entry_name} changed its frozen recovery actions:\nexpected={expected_action_list:?}\nactual={actual:?}"
    )))
}

fn frozen_rejection_kind<'frozen>(
    frozen: &'frozen str,
    corpus_entry_name: &str,
) -> TestResult<&'frozen str> {
    frozen
        .split_once("coordination identity ")
        .and_then(|(_, remainder)| remainder.split_whitespace().next())
        .ok_or_else(|| {
            failure(format!(
                "{corpus_entry_name} should freeze a coordination identity rejection kind"
            ))
        })
}

/// The commands one frozen refusal offers, restated in this crate's quoting.
fn frozen_recovery_actions(frozen: &str, fixture_roots: &[(&str, &Path)]) -> Vec<String> {
    frozen
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|action| requote_frozen_action(action, fixture_roots))
        .collect()
}

/// Restate one frozen recovery argv in this crate's quoting convention.
///
/// The corpus quotes every argv token, because the retired shell front end rendered
/// them that way. This crate quotes a token only when it needs one, so the two agree
/// on the command and differ only on its spelling. Re-quoting the frozen tokens here
/// states that difference instead of discarding it: a token whose spelling the crate
/// would change is still compared, so a real quoting regression fails.
fn requote_frozen_action(action: &str, fixture_roots: &[(&str, &Path)]) -> String {
    frozen_argv_tokens(action)
        .into_iter()
        .map(|token| match token {
            FrozenArgvToken::Literal(literal) => literal.to_owned(),
            FrozenArgvToken::Quoted(quoted) => {
                shell_quote(&resolve_fixture_root(quoted, fixture_roots))
            },
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn frozen_argv_tokens(action: &str) -> Vec<FrozenArgvToken<'_>> {
    let mut tokens = Vec::new();
    let mut remainder = action;
    loop {
        remainder = remainder.trim_start();
        if remainder.is_empty() {
            return tokens;
        }
        if let Some(quoted) = remainder.strip_prefix('\'') {
            let Some((token, tail)) = quoted.split_once('\'') else {
                tokens.push(FrozenArgvToken::Literal(remainder));
                return tokens;
            };
            tokens.push(FrozenArgvToken::Quoted(token));
            remainder = tail;
        } else {
            let (token, tail) = remainder.split_once(' ').unwrap_or((remainder, ""));
            tokens.push(FrozenArgvToken::Literal(token));
            remainder = tail;
        }
    }
}

/// Replace one frozen fixture-root alias with the scratch path it stands for.
fn resolve_fixture_root(token: &str, fixture_roots: &[(&str, &Path)]) -> String {
    fixture_roots
        .iter()
        .find(|(alias, _)| *alias == token)
        .map_or_else(
            || token.to_owned(),
            |(_, root)| root.to_string_lossy().into_owned(),
        )
}

/// This crate's argv quoting rule, restated as an independent oracle.
///
/// `coordination_identity.rs::shell_quote` leaves a non-empty token bare when every
/// character is ASCII alphanumeric or one of `-_/.:=@%+`, and single-quotes it
/// otherwise, escaping any embedded quote.
fn shell_quote(argument: &str) -> String {
    if !argument.is_empty()
        && argument.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '/' | '.' | ':' | '=' | '@' | '%' | '+'
                )
        })
    {
        argument.to_owned()
    } else {
        format!("'{}'", argument.replace('\'', "'\\''"))
    }
}

fn assert_engine_authored_refusal(
    output: &Output,
    expected_summary: &str,
    expected_detail: &str,
    context: &str,
) -> TestResult {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let refusal = String::from_utf8(output.stderr.clone())?;
    if refusal.contains(RETIRED_MISSING_PRESENTATION_DIAGNOSTIC) {
        return Err(failure(format!(
            "{context} reached the user as the hook's own diagnostic instead of the engine's message: {refusal:?}"
        )));
    }
    if !refusal.starts_with(expected_summary) {
        return Err(failure(format!(
            "{context} should open with the engine's own summary {expected_summary:?}: {refusal:?}"
        )));
    }
    if refusal.contains(expected_detail) {
        return Ok(());
    }
    Err(failure(format!(
        "{context} should carry the engine's own message {expected_detail:?}: {refusal:?}"
    )))
}

fn assert_fail_open_sentence_stated_once(output: &Output, context: &str) -> TestResult {
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let notice: Value = serde_json::from_slice(&output.stdout)?;
    let system_message = required_string(&notice, "/systemMessage", context)?;
    let reason = required_string(
        &notice,
        "/hookSpecificOutput/permissionDecisionReason",
        context,
    )?;
    let occurrences = system_message.matches(FAIL_OPEN_SENTENCE).count()
        + reason.matches(FAIL_OPEN_SENTENCE).count();
    if occurrences == 1 {
        return Ok(());
    }
    Err(failure(format!(
        "{context} should state the fail-open sentence once, stated it {occurrences} time(s): systemMessage={system_message:?} permissionDecisionReason={reason:?}"
    )))
}

fn assert_refused_for_scope(output: &Output, scope: &str, context: &str) -> TestResult {
    assert!(output.stdout.is_empty());
    let refusal = String::from_utf8(output.stderr.clone())?;
    if output.status.code() != Some(2) {
        return Err(failure(format!(
            "{context} should still be checked, exited with {:?}: {refusal:?}",
            output.status.code()
        )));
    }
    if refusal.contains(scope) {
        return Ok(());
    }
    Err(failure(format!(
        "{context} should be refused for {scope:?}: {refusal:?}"
    )))
}

fn assert_refusal_states(output: &Output, reason: &str, context: &str) -> TestResult {
    let refusal = String::from_utf8(output.stderr.clone())?;
    if output.status.code() == Some(2) && refusal.contains(reason) {
        return Ok(());
    }
    Err(failure(format!(
        "{context} should refuse stating {reason:?}, exited with {:?}: {refusal:?}",
        output.status.code()
    )))
}

/// The hook never coordinated one named scope, whatever the payload spelled.
fn assert_no_scope_was_claimed(output: &Output, scope: &str, context: &str) -> TestResult {
    let answer = String::from_utf8(output.stderr.clone())?;
    if answer.contains(scope) {
        return Err(failure(format!(
            "{context} was answered as {scope:?}, a path the payload never named: {answer:?}"
        )));
    }
    Ok(())
}

/// No scope the hook forms may carry a parent component, whatever the payload named.
///
/// The refusal prose quotes `<paths...>` in its answer commands, so this reads only the
/// scope tokens the engine printed and never the sentences around them.
fn assert_no_parent_component_was_claimed(output: &Output) -> TestResult {
    let refusal = String::from_utf8(output.stderr.clone())?;
    let claimed_scope = refusal
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|token| token.starts_with("file:") || token.starts_with("tree:"))
        .find(|token| token.split('/').any(|component| component == ".."));
    claimed_scope.map_or_else(
        || Ok(()),
        |scope| {
            Err(failure(format!(
                "a hook answer named the scope {scope:?}, which carries a parent component"
            )))
        },
    )
}

fn corpus_expected_stderr(entry_name: &str) -> TestResult<String> {
    let entry = corpus_entry(entry_name)?;
    Ok(required_string(entry, "/expected/stderr", entry_name)?.to_owned())
}

fn corpus_expected_permission_reason(entry_name: &str) -> TestResult<String> {
    let entry = corpus_entry(entry_name)?;
    let stdout = required_string(entry, "/expected/stdout", entry_name)?;
    let notice: Value = serde_json::from_str(stdout)?;
    Ok(required_string(
        &notice,
        "/hookSpecificOutput/permissionDecisionReason",
        entry_name,
    )?
    .to_owned())
}

fn corpus_entry(entry_name: &str) -> TestResult<&'static Value> {
    let corpus = parsed_corpus()?;
    required_array(corpus, "/entries", "front-end corpus")?
        .iter()
        .find(|entry| entry["name"] == entry_name)
        .ok_or_else(|| failure(format!("missing corpus entry {entry_name}")))
}

fn parsed_corpus() -> TestResult<&'static Value> {
    static CORPUS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let corpus =
        CORPUS.get_or_init(|| serde_json::from_str(FRONT_END_CORPUS_JSON).unwrap_or(Value::Null));
    if corpus.is_object() {
        Ok(corpus)
    } else {
        Err(failure("front-end corpus should parse as an object"))
    }
}

fn edit_payload(repository_root: &Path, relative_path: &str, session_id: Option<&str>) -> Value {
    let file_path = repository_root.join(relative_path);
    let mut payload = serde_json::json!({
        "tool_name": "Edit",
        "cwd": repository_root,
        "tool_input": {"file_path": file_path},
    });
    if let Some(session_id) = session_id {
        payload["session_id"] = Value::String(session_id.to_owned());
    }
    payload
}

fn corpus_edit_payload(
    entry_name: &str,
    repository_root: &Path,
    relative_path: &str,
    session_id: Option<&str>,
) -> TestResult<Value> {
    let mut payload = corpus_entry(entry_name)?["harness_payload"].clone();
    let payload_object = payload
        .as_object_mut()
        .ok_or_else(|| failure(format!("{entry_name} harness payload should be an object")))?;
    payload_object.insert(
        "cwd".to_owned(),
        Value::String(repository_root.to_string_lossy().into_owned()),
    );
    match session_id {
        Some(session_id) => {
            payload_object.insert(
                "session_id".to_owned(),
                Value::String(session_id.to_owned()),
            );
        },
        None => {
            payload_object.remove("session_id");
        },
    }
    payload["tool_input"]["file_path"] = Value::String(
        repository_root
            .join(relative_path)
            .to_string_lossy()
            .into_owned(),
    );
    Ok(payload)
}

fn run_pre_tool_use(repository_root: &Path, payload: &Value) -> TestResult<Output> {
    spawn_pre_tool_use(repository_root, payload, &AmbientHarnessSession::Absent)
}

fn spawn_pre_tool_use(
    repository_root: &Path,
    payload: &Value,
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    spawn_hook_verb(
        repository_root,
        "pre-tool-use",
        &serde_json::to_vec(payload)?,
        ambient_session,
    )
}

/// Run one public hook verb the way the harness runs it: raw payload on standard input.
fn spawn_hook_verb(
    working_directory: &Path,
    hook_event: &str,
    stdin: &[u8],
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-berth"));
    command
        .args(["hook", hook_event])
        .current_dir(working_directory)
        .env_remove("CARGO_BERTH_RUN");
    match *ambient_session {
        AmbientHarnessSession::Present(session_id) => {
            command.env("CARGO_BERTH_SESSION_ID", session_id);
        },
        AmbientHarnessSession::Absent => {
            command.env_remove("CARGO_BERTH_SESSION_ID");
        },
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut piped_stdin = child
        .stdin
        .take()
        .ok_or_else(|| failure("hook stdin should be piped"))?;
    piped_stdin.write_all(stdin)?;
    drop(piped_stdin);
    Ok(child.wait_with_output()?)
}

fn initialized_repository() -> TestResult<TempDir> {
    let repository = git_repository()?;
    let initialized = run_berth(repository.path(), &["init", "--json"])?;
    require_success(&initialized, "cargo-berth init")?;
    Ok(repository)
}

fn git_repository() -> TestResult<TempDir> {
    fs::create_dir_all(SCRATCH_ROOT)?;
    let repository = TempDir::new_in(SCRATCH_ROOT)?;
    run_git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    )?;
    run_git(
        repository.path(),
        &["config", "user.name", "Hook Acceptance Test"],
    )?;
    run_git(
        repository.path(),
        &["config", "user.email", "hooks@example.invalid"],
    )?;
    fs::write(repository.path().join("README.md"), "hook fixture\n")?;
    run_git(repository.path(), &["add", "README.md"])?;
    run_git(
        repository.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ],
    )?;
    Ok(repository)
}

fn add_worktree(repository: &TempDir, name: &str) -> TestResult<(TempDir, PathBuf)> {
    let directory = TempDir::new_in(SCRATCH_ROOT)?;
    let root = directory.path().join(name);
    let root_text = root
        .to_str()
        .ok_or_else(|| failure("scratch worktree path should be UTF-8"))?;
    run_git(
        repository.path(),
        &["worktree", "add", "--quiet", "-b", name, root_text],
    )?;
    let configuration_path = root.join(CONFIGURATION_PATH);
    let configuration_directory = configuration_path
        .parent()
        .ok_or_else(|| failure("worktree configuration should have a parent"))?;
    fs::create_dir_all(configuration_directory)?;
    fs::copy(
        repository.path().join(CONFIGURATION_PATH),
        configuration_path,
    )?;
    Ok((directory, root))
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()?)
}

fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session_id)
        .output()?)
}

/// Read one reference's current commit, the way the corpus identifiers are restated from it.
fn git_revision(repository_root: &Path, reference: &str) -> TestResult<String> {
    let output = support::git_command()
        .args(["rev-parse", reference])
        .current_dir(repository_root)
        .output()?;
    require_success(&output, "git rev-parse")?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn run_git(repository_root: &Path, arguments: &[&str]) -> TestResult {
    let output = support::git_command()
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "git {arguments:?} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn assert_hook_output(output: &Output, exit_code: i32, stdout: &[u8], stderr: &[u8]) -> TestResult {
    if output.status.code() == Some(exit_code) && output.stdout == stdout && output.stderr == stderr
    {
        Ok(())
    } else {
        Err(failure(format!(
            "unexpected hook output: code={:?} stdout={:?} stderr={:?}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn require_success(output: &Output, operation: &str) -> TestResult {
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "{operation} failed with {:?}: stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn json_output(output: &Output) -> TestResult<Value> {
    serde_json::from_slice(&output.stdout).map_err(Into::into)
}

fn required_string<'value>(
    value: &'value Value,
    pointer: &str,
    context: &str,
) -> TestResult<&'value str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{context} should carry string {pointer}")))
}

fn required_array<'value>(
    value: &'value Value,
    pointer: &str,
    context: &str,
) -> TestResult<&'value [Value]> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| failure(format!("{context} should carry array {pointer}")))
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}

/// One identifier a real run produced, beside the identifier the corpus froze for it.
struct CorpusIdentifier {
    observed: String,
    frozen:   String,
}

/// The two sentences one hook response puts in front of the reader.
#[derive(Debug)]
struct HookFeedback {
    system_message:     String,
    additional_context: String,
}

/// One incursion a `PostToolUse` drift reported, named as the corpus names it.
struct ObservedIncursion {
    incident_id:            String,
    foreign_reservation_id: String,
}

/// One `PostToolUse` hook run that entered foreign reservations, kept readable after it.
struct IncursionAfterBash {
    output:                  Output,
    straying_reservation_id: String,
    incidents:               Vec<ObservedIncursion>,
    straying_root:           PathBuf,
    repository:              TempDir,
    worktrees:               TempDir,
}

impl IncursionAfterBash {
    /// Answer every reported incident, so the board records it instead of holding it open.
    ///
    /// The corpus froze what each hook says once an incursion has been answered, which is a
    /// different state from never having entered a foreign reservation at all: the incident
    /// stays on the board under its recorded answers, and the hook stops repeating the stop
    /// instruction for it.
    fn record_every_incursion(&self) -> TestResult {
        for incident in &self.incidents {
            let recorded = run_berth_with_session(
                &self.straying_root,
                &[
                    "resolve",
                    &self.straying_reservation_id,
                    "--incursion",
                    &incident.incident_id,
                    "--json",
                ],
                INCURSION_SESSION,
            )?;
            require_success(&recorded, "incursion answer")?;
        }
        Ok(())
    }

    /// Report the next Bash call from the same worktree and session.
    fn report_another_bash_call(&self) -> TestResult<Output> {
        run_post_tool_use(
            &self.straying_root,
            &bash_payload(&self.straying_root, INCURSION_SESSION),
        )
    }

    /// The corpus names the straying reservation, each foreign holder and each incident.
    fn corpus_identifiers(&self) -> Vec<CorpusIdentifier> {
        let mut identifiers = vec![CorpusIdentifier {
            observed: self.straying_reservation_id.clone(),
            frozen:   "reservation-straying".to_owned(),
        }];
        for (index, incident) in self.incidents.iter().enumerate() {
            identifiers.push(CorpusIdentifier {
                observed: incident.foreign_reservation_id.clone(),
                frozen:   format!("foreign-{index}"),
            });
            identifiers.push(CorpusIdentifier {
                observed: incident.incident_id.clone(),
                frozen:   format!("incident-{index}"),
            });
        }
        identifiers
    }
}

/// One released reservation whose worktree is gone, so the board reports it outstanding.
struct ObservedOrphan {
    reservation_id: String,
    protected_tip:  String,
}

/// Whether an orphaned reservation's commits can still be reached at all.
///
/// The board offers a different answer for each: work whose commit survives is recovered
/// into a replacement worktree, and work whose commit is gone can only be retired or
/// abandoned. The corpus froze one session-start entry for each.
#[derive(Clone, Copy)]
enum OrphanedCommitAvailability {
    /// The branch and retention reference still reach the protected tip.
    Available,
    /// Nothing in the repository reaches the protected tip any more.
    Unavailable,
}

/// A repository whose board reports orphaned outstanding reservations.
struct OrphanedReservations {
    repository: TempDir,
    orphans:    Vec<ObservedOrphan>,
    _worktrees: TempDir,
}

impl OrphanedReservations {
    /// The corpus names each orphaned reservation and the tip it protects.
    fn corpus_identifiers(&self) -> TestResult<Vec<CorpusIdentifier>> {
        self.corpus_identifiers_for(ORPHAN_SESSION_START_ENTRY)
    }

    fn corpus_identifiers_for(&self, entry_name: &str) -> TestResult<Vec<CorpusIdentifier>> {
        let entry = corpus_entry(entry_name)?;
        let frozen = required_array(
            entry,
            "/engine_responses/board/body/payload/data/alerts/entries",
            entry_name,
        )?;
        if frozen.len() != self.orphans.len() {
            return Err(failure(format!(
                "{entry_name} froze {} orphans, the fixture produced {}",
                frozen.len(),
                self.orphans.len()
            )));
        }
        frozen
            .iter()
            .zip(self.orphans.iter())
            .map(|(frozen, observed)| {
                Ok([
                    CorpusIdentifier {
                        observed: observed.reservation_id.clone(),
                        frozen:   required_string(frozen, "/reservation_id", "frozen orphan")?
                            .to_owned(),
                    },
                    CorpusIdentifier {
                        observed: observed.protected_tip.clone(),
                        frozen:   required_string(frozen, "/protected_tip", "frozen orphan")?
                            .to_owned(),
                    },
                ])
            })
            .collect::<TestResult<Vec<_>>>()
            .map(|identifiers| identifiers.into_iter().flatten().collect())
    }
}

/// A repository whose berth configuration is committed, so it never itself drifts.
///
/// Every corpus comparison below depends on this: an uncommitted `berth.toml` is a
/// changed path like any other, and the auto-widen line it produces is text the corpus
/// never froze.
fn committed_configuration_repository() -> TestResult<TempDir> {
    let repository = initialized_repository()?;
    run_git(repository.path(), &["add", CONFIGURATION_PATH])?;
    run_git(
        repository.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "configure cargo-berth",
        ],
    )?;
    Ok(repository)
}

fn claimed_reservation_id(claimed: &Output) -> TestResult<String> {
    let envelope = json_output(claimed)?;
    Ok(required_string(&envelope, "/payload/data/reservation_id", "claim")?.to_owned())
}

/// Drive one Bash call that enters `path_count` foreign reservations.
fn incursion_after_bash(path_count: usize) -> TestResult<IncursionAfterBash> {
    let repository = committed_configuration_repository()?;
    for index in 0..path_count {
        let scope = format!("file:path-{index}.rs");
        let holder = run_berth_with_session(
            repository.path(),
            &["claim", &scope, "--json"],
            &format!("incursion-holder-{index}"),
        )?;
        require_success(&holder, "incursion holder claim")?;
    }
    let worktrees = TempDir::new_in(SCRATCH_ROOT)?;
    let straying_root = worktrees.path().join("straying");
    add_named_worktree(&repository, "straying", &straying_root)?;
    let straying = run_berth_with_session(
        &straying_root,
        &["claim", "file:straying.rs", "--json"],
        INCURSION_SESSION,
    )?;
    require_success(&straying, "straying claim")?;
    let straying_reservation_id = claimed_reservation_id(&straying)?;
    for index in 0..path_count {
        fs::write(
            straying_root.join(format!("path-{index}.rs")),
            "// entered\n",
        )?;
    }

    let output = run_post_tool_use(
        &straying_root,
        &bash_payload(&straying_root, INCURSION_SESSION),
    )?;
    let incidents = observed_incursions(repository.path(), path_count)?;
    Ok(IncursionAfterBash {
        output,
        straying_reservation_id,
        incidents,
        straying_root,
        repository,
        worktrees,
    })
}

/// Read each recorded incident back from the board, ordered by the path it entered.
fn observed_incursions(
    repository_root: &Path,
    path_count: usize,
) -> TestResult<Vec<ObservedIncursion>> {
    let board = run_berth(repository_root, &["board", "--json"])?;
    require_success(&board, "incursion board")?;
    let envelope = json_output(&board)?;
    let entries = required_array(
        &envelope,
        "/payload/data/outstanding_incursions/entries",
        "incursion board",
    )?;
    (0..path_count)
        .map(|index| {
            let path = format!("path-{index}.rs");
            let entry = entries
                .iter()
                .find(|entry| {
                    entry["entered_paths"]
                        .as_array()
                        .is_some_and(|paths| paths.iter().any(|entered| *entered == path))
                })
                .ok_or_else(|| {
                    failure(format!("the board should report an incursion into {path}"))
                })?;
            Ok(ObservedIncursion {
                incident_id:            required_string(entry, "/incident_id", "incursion entry")?
                    .to_owned(),
                foreign_reservation_id: required_string(
                    entry,
                    "/foreign_reservation_ids/0",
                    "incursion entry",
                )?
                .to_owned(),
            })
        })
        .collect()
}

/// Release `count` reservations whose worktrees then disappear, orphaning each.
fn orphaned_reservations(count: usize) -> TestResult<OrphanedReservations> {
    orphaned_reservations_with(count, OrphanedCommitAvailability::Available)
}

/// Release `count` reservations whose worktrees disappear, keeping or losing their commits.
fn orphaned_reservations_with(
    count: usize,
    availability: OrphanedCommitAvailability,
) -> TestResult<OrphanedReservations> {
    let repository = committed_configuration_repository()?;
    let worktrees = TempDir::new_in(SCRATCH_ROOT)?;
    let mut orphans = Vec::with_capacity(count);
    for index in 0..count {
        let branch = format!("orphan-{index}");
        let orphan_root = worktrees.path().join(&branch);
        add_named_worktree(&repository, &branch, &orphan_root)?;
        let source = format!("src/orphan-{index}.rs");
        fs::create_dir_all(orphan_root.join("src"))?;
        fs::write(orphan_root.join(&source), "// orphaned work\n")?;
        run_git(&orphan_root, &["add", &source])?;
        run_git(
            &orphan_root,
            &[
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                "orphaned work",
            ],
        )?;
        let claimed = run_berth_with_session(
            &orphan_root,
            &["claim", &format!("file:{source}"), "--json"],
            &format!("orphan-session-{index}"),
        )?;
        require_success(&claimed, "orphan claim")?;
        let reservation_id = claimed_reservation_id(&claimed)?;
        let released = run_berth(&orphan_root, &["release", &reservation_id, "--json"])?;
        require_success(&released, "orphan release")?;
        let released_envelope = json_output(&released)?;
        let protected_tip = required_string(
            &released_envelope,
            "/payload/data/protected_tip",
            "orphan release",
        )?
        .to_owned();
        fs::remove_dir_all(&orphan_root)?;
        orphans.push(ObservedOrphan {
            reservation_id,
            protected_tip,
        });
    }
    run_git(repository.path(), &["worktree", "prune", "--expire", "now"])?;
    if matches!(availability, OrphanedCommitAvailability::Unavailable) {
        for (index, orphan) in orphans.iter().enumerate() {
            run_git(
                repository.path(),
                &["branch", "-D", &format!("orphan-{index}")],
            )?;
            run_git(
                repository.path(),
                &[
                    "update-ref",
                    "-d",
                    &format!("refs/cargo-berth/reservations/{}", orphan.reservation_id),
                ],
            )?;
        }
        run_git(
            repository.path(),
            &["reflog", "expire", "--expire=now", "--all"],
        )?;
        run_git(repository.path(), &["gc", "--prune=now", "--quiet"])?;
    }
    Ok(OrphanedReservations {
        repository,
        orphans,
        _worktrees: worktrees,
    })
}

/// One released reservation whose recorded integration proof no longer holds.
struct LostIntegrationEvidence {
    reservation_id: String,
    protected_tip:  String,
    /// The worktree a hook run reports from, whose own branch still resolves.
    ///
    /// Trunk is left unprovable in the repository's main worktree, and one of the two ways of
    /// doing that leaves its checked-out branch naming an object the repository does not hold.
    /// A hook run reports from the released work's own worktree instead, so what the response
    /// states is the alert under test rather than a broken checkout.
    reporting_root: PathBuf,
}

impl LostIntegrationEvidence {
    /// The corpus names the released reservation and the tip it can no longer prove.
    ///
    /// The trunk commit is restated only where the corpus froze one: a trunk that resolves
    /// names itself in the recovery command, and a trunk that does not resolve names nothing,
    /// so restating an absent identifier there would rewrite text the corpus never carried.
    fn corpus_identifiers(&self, trunk: &ObservedTrunk) -> Vec<CorpusIdentifier> {
        let mut identifiers = vec![
            CorpusIdentifier {
                observed: self.reservation_id.clone(),
                frozen:   FROZEN_LOST_EVIDENCE_RESERVATION.to_owned(),
            },
            CorpusIdentifier {
                observed: self.protected_tip.clone(),
                frozen:   FROZEN_LOST_EVIDENCE_PROTECTED_TIP.to_owned(),
            },
        ];
        if let ObservedTrunk::Resolved(trunk_oid) = trunk {
            identifiers.push(CorpusIdentifier {
                observed: trunk_oid.clone(),
                frozen:   FROZEN_LOST_EVIDENCE_TRUNK.to_owned(),
            });
        }
        identifiers
    }
}

/// How one recorded integration proof is taken away again.
enum IntegrationProofLoss {
    /// Trunk moves past the protected tip, so it resolves but proves nothing.
    TrunkRewrittenPastTheTip,
    /// Trunk names an object this repository does not hold, so no proof is possible.
    TrunkNamesAnAbsentObject,
}

/// Whether the trunk this repository configures still names a readable commit.
enum ObservedTrunk {
    /// Trunk resolves, and the engine can name it in the recovery command it offers.
    Resolved(String),
    /// Trunk names an object this repository does not hold, so no proof is possible.
    Unresolvable,
}

/// Record one reservation's work as integrated, then take that proof away again.
///
/// Both lost-evidence corpus entries start from the same released-and-proven reservation and
/// differ only in how the proof is lost: `Resolved` rewrites trunk past the protected tip, and
/// `Unresolvable` leaves trunk naming a commit the repository does not hold.
fn lose_integration_evidence(
    repository: &TempDir,
    worktrees: &Path,
    loss: &IntegrationProofLoss,
) -> TestResult<(LostIntegrationEvidence, ObservedTrunk)> {
    let integrated_root = worktrees.join("integrated");
    add_named_worktree(repository, "integrated", &integrated_root)?;
    let source = "src/integrated.rs";
    fs::create_dir_all(integrated_root.join("src"))?;
    fs::write(integrated_root.join(source), "// released work\n")?;
    run_git(&integrated_root, &["add", source])?;
    run_git(
        &integrated_root,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "released work",
        ],
    )?;
    let claimed = run_berth_with_session(
        &integrated_root,
        &["claim", &format!("file:{source}"), "--json"],
        EVIDENCE_SESSION,
    )?;
    require_success(&claimed, "released work claim")?;
    let reservation_id = claimed_reservation_id(&claimed)?;
    let released = run_berth(&integrated_root, &["release", &reservation_id, "--json"])?;
    require_success(&released, "released work release")?;
    let protected_tip = required_string(
        &json_output(&released)?,
        "/payload/data/protected_tip",
        "released work release",
    )?
    .to_owned();

    let trunk_before_integration = git_revision(repository.path(), TRUNK_BRANCH)?;
    run_git(
        repository.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--quiet",
            "--no-ff",
            "-m",
            "integrate released work",
            "integrated",
        ],
    )?;
    let integrated_trunk = git_revision(repository.path(), TRUNK_BRANCH)?;
    let resolved = run_berth(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--integrated-as",
            &integrated_trunk,
            "--json",
        ],
    )?;
    require_success(&resolved, "integration record")?;
    take_the_integration_proof_away(repository, &trunk_before_integration, loss)?;
    let trunk = match loss {
        IntegrationProofLoss::TrunkRewrittenPastTheTip => {
            ObservedTrunk::Resolved(git_revision(repository.path(), TRUNK_BRANCH)?)
        },
        IntegrationProofLoss::TrunkNamesAnAbsentObject => ObservedTrunk::Unresolvable,
    };
    Ok((
        LostIntegrationEvidence {
            reservation_id,
            protected_tip,
            reporting_root: integrated_root,
        },
        trunk,
    ))
}

/// Leave trunk unable to prove the protected tip, in the way the caller asked for.
fn take_the_integration_proof_away(
    repository: &TempDir,
    trunk_before_integration: &str,
    loss: &IntegrationProofLoss,
) -> TestResult {
    match loss {
        IntegrationProofLoss::TrunkRewrittenPastTheTip => {
            run_git(
                repository.path(),
                &["reset", "--hard", "--quiet", trunk_before_integration],
            )?;
            fs::write(repository.path().join("rewritten.rs"), "// rewritten\n")?;
            run_git(repository.path(), &["add", "rewritten.rs"])?;
            run_git(
                repository.path(),
                &[
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "--quiet",
                    "-m",
                    "rewrite trunk past the released work",
                ],
            )
        },
        IntegrationProofLoss::TrunkNamesAnAbsentObject => Ok(fs::write(
            repository.path().join(TRUNK_REFERENCE_PATH),
            format!("{ABSENT_TRUNK_OBJECT_ID}\n"),
        )?),
    }
}

fn add_named_worktree(repository: &TempDir, branch: &str, root: &Path) -> TestResult {
    let root_text = root
        .to_str()
        .ok_or_else(|| failure("scratch worktree path should be UTF-8"))?;
    run_git(
        repository.path(),
        &["worktree", "add", "--quiet", "-b", branch, root_text],
    )?;
    let configuration_path = root.join(CONFIGURATION_PATH);
    let configuration_directory = configuration_path
        .parent()
        .ok_or_else(|| failure("worktree configuration should have a parent"))?;
    fs::create_dir_all(configuration_directory)?;
    fs::copy(
        repository.path().join(CONFIGURATION_PATH),
        configuration_path,
    )?;
    Ok(())
}

fn bash_payload(working_directory: &Path, session_id: &str) -> Value {
    serde_json::json!({
        "tool_name": "Bash",
        "cwd": working_directory,
        "tool_input": {"command": "true"},
        "session_id": session_id,
    })
}

fn session_start_payload(working_directory: &Path, session_id: Option<&str>) -> Value {
    let mut payload = serde_json::json!({
        "hook_event_name": "SessionStart",
        "cwd": working_directory,
        "source": "startup",
    });
    if let Some(session_id) = session_id {
        payload["session_id"] = Value::String(session_id.to_owned());
    }
    payload
}

fn run_post_tool_use(working_directory: &Path, payload: &Value) -> TestResult<Output> {
    run_post_tool_use_with_ambient_session(
        working_directory,
        payload,
        &AmbientHarnessSession::Absent,
    )
}

/// Run `hook post-tool-use` under an environment that names a harness session of its own.
fn run_post_tool_use_with_ambient_session(
    working_directory: &Path,
    payload: &Value,
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    spawn_hook_verb(
        working_directory,
        "post-tool-use",
        &serde_json::to_vec(payload)?,
        ambient_session,
    )
}

fn run_post_tool_use_stdin(working_directory: &Path, stdin: &[u8]) -> TestResult<Output> {
    spawn_hook_verb(
        working_directory,
        "post-tool-use",
        stdin,
        &AmbientHarnessSession::Absent,
    )
}

fn run_session_start(
    working_directory: &Path,
    payload: &Value,
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    spawn_hook_verb(
        working_directory,
        "session-start",
        &serde_json::to_vec(payload)?,
        ambient_session,
    )
}

fn run_session_start_stdin(working_directory: &Path, stdin: &[u8]) -> TestResult<Output> {
    spawn_hook_verb(
        working_directory,
        "session-start",
        stdin,
        &AmbientHarnessSession::Absent,
    )
}

/// One hook response, read the way a harness reads it.
fn hook_feedback(
    output: &Output,
    event: HookResponseEvent,
    context: &str,
) -> TestResult<HookFeedback> {
    let event_name = event.name();
    if output.status.code() != Some(0) {
        return Err(failure(format!(
            "{context} should exit 0 like the hook it replaces, exited with {:?}: stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if !output.stderr.is_empty() {
        return Err(failure(format!(
            "{context} should keep its response on stdout: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let response: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        failure(format!(
            "{context} stdout should be one hook response object: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })?;
    let observed_event = required_string(&response, "/hookSpecificOutput/hookEventName", context)?;
    if observed_event != event_name {
        return Err(failure(format!(
            "{context} should name the {event_name} event, named {observed_event:?}"
        )));
    }
    assert_stated_continuation(&response, event, context)?;
    Ok(HookFeedback {
        system_message:     required_string(&response, "/systemMessage", context)?.to_owned(),
        additional_context: required_string(
            &response,
            "/hookSpecificOutput/additionalContext",
            context,
        )?
        .to_owned(),
    })
}

/// Each event states its own continuation field, and `SessionStart` states none.
///
/// `berth_post_bash.sh` reports `continue`, so a `PostToolUse` response reports it too. The
/// installed `berth_session_start.sh` deliberately reports no continuation field, because a
/// session-start response cannot stop anything the harness is already going to do; moving
/// the two events onto one shared writer must not add the field the installed hook omits.
fn assert_stated_continuation(
    response: &Value,
    event: HookResponseEvent,
    context: &str,
) -> TestResult {
    let stated = response.get("continue");
    match event {
        HookResponseEvent::PostToolUse => {
            if stated == Some(&Value::Bool(true)) {
                return Ok(());
            }
            Err(failure(format!(
                "{context} should report that the session continues: {response}"
            )))
        },
        HookResponseEvent::SessionStart => stated.map_or_else(
            || Ok(()),
            |stated| {
                Err(failure(format!(
                    "{context} states a continuation field the installed hook omits: {stated}"
                )))
            },
        ),
    }
}

fn assert_post_tool_use_feedback_matches_corpus(
    output: &Output,
    corpus_entry_name: &str,
    identifiers: &[CorpusIdentifier],
) -> TestResult {
    assert_hook_feedback_matches_corpus(
        output,
        HookResponseEvent::PostToolUse,
        corpus_entry_name,
        identifiers,
        &FrozenTextCoverage::ExactlyTheFrozenLines,
    )
}

fn assert_session_start_feedback_matches_corpus(
    output: &Output,
    corpus_entry_name: &str,
    identifiers: &[CorpusIdentifier],
) -> TestResult {
    assert_hook_feedback_matches_corpus(
        output,
        HookResponseEvent::SessionStart,
        corpus_entry_name,
        identifiers,
        &FrozenTextCoverage::TheFrozenLinesInsideTheReport,
    )
}

/// Compare one produced hook response against the text the corpus froze for it.
///
/// The identifiers differ every run, so each observed identifier is restated as the one
/// the corpus froze before the comparison. `PostToolUse` states exactly the notice lines,
/// so its produced lines and its frozen lines have to be the same set and a line the corpus
/// never froze fails here. `SessionStart` publishes every rendered block, so its response
/// carries the engine's complete board report around the frozen notices and only the frozen
/// lines are required to appear.
fn assert_hook_feedback_matches_corpus(
    output: &Output,
    event: HookResponseEvent,
    corpus_entry_name: &str,
    identifiers: &[CorpusIdentifier],
    coverage: &FrozenTextCoverage,
) -> TestResult {
    let feedback = hook_feedback(output, event, corpus_entry_name)?;
    let frozen = corpus_expected_hook_feedback(corpus_entry_name)?;
    let restated = identifiers.iter().fold(
        feedback.additional_context.clone(),
        |context, identifier| context.replace(&identifier.observed, &identifier.frozen),
    );
    let produced_lines = restated.lines().collect::<Vec<_>>();
    let frozen_lines = frozen.additional_context.lines().collect::<Vec<_>>();
    let unstated = frozen_lines
        .iter()
        .filter(|frozen_line| !produced_lines.contains(*frozen_line))
        .collect::<Vec<_>>();
    if !unstated.is_empty() {
        return Err(failure(format!(
            "{corpus_entry_name} no longer states its frozen text:\nmissing={unstated:#?}\nproduced={restated:?}"
        )));
    }
    if matches!(*coverage, FrozenTextCoverage::ExactlyTheFrozenLines) {
        let unfrozen = produced_lines
            .iter()
            .filter(|produced_line| !frozen_lines.contains(*produced_line))
            .collect::<Vec<_>>();
        if !unfrozen.is_empty() {
            return Err(failure(format!(
                "{corpus_entry_name} states text the corpus never froze:\nextra={unfrozen:#?}\nproduced={restated:?}"
            )));
        }
    }
    if feedback.system_message == frozen.system_message {
        return Ok(());
    }
    Err(failure(format!(
        "{corpus_entry_name} no longer states its frozen summary:\nfrozen={:?}\nproduced={:?}",
        frozen.system_message, feedback.system_message
    )))
}

/// The corpus froze a silent response for this case, and the verb must stay silent too.
fn assert_hook_stayed_silent(output: &Output, corpus_entry_name: &str) -> TestResult {
    let entry = corpus_entry(corpus_entry_name)?;
    let frozen_stdout = required_string(entry, "/expected/stdout", corpus_entry_name)?;
    if !frozen_stdout.trim().is_empty() {
        return Err(failure(format!(
            "{corpus_entry_name} is not a silent corpus entry: {frozen_stdout:?}"
        )));
    }
    assert_hook_output(output, 0, b"", b"")
}

fn corpus_expected_hook_feedback(corpus_entry_name: &str) -> TestResult<HookFeedback> {
    let entry = corpus_entry(corpus_entry_name)?;
    let stdout = required_string(entry, "/expected/stdout", corpus_entry_name)?;
    let response: Value = serde_json::from_str(stdout)?;
    Ok(HookFeedback {
        system_message:     required_string(&response, "/systemMessage", corpus_entry_name)?
            .to_owned(),
        additional_context: required_string(
            &response,
            "/hookSpecificOutput/additionalContext",
            corpus_entry_name,
        )?
        .to_owned(),
    })
}

/// Compare one produced coordination-identity refusal against its frozen recovery.
///
/// Only the rejection kind and the recovery commands are a contract the corpus and this
/// engine share; the corpus also froze the retired shell front end's heading, which the
/// engine now writes for itself.
fn assert_post_tool_use_corpus_recovery(
    output: &Output,
    corpus_entry_name: &str,
    fixture_roots: &[(&str, &Path)],
) -> TestResult {
    let feedback = hook_feedback(output, HookResponseEvent::PostToolUse, corpus_entry_name)?;
    let frozen = corpus_expected_hook_feedback(corpus_entry_name)?;
    let rejection_kind = frozen
        .additional_context
        .split_once("COORDINATION IDENTITY: ")
        .and_then(|(_, remainder)| remainder.split_whitespace().next())
        .ok_or_else(|| {
            failure(format!(
                "{corpus_entry_name} should freeze a coordination identity rejection kind"
            ))
        })?;
    if !feedback.additional_context.contains(rejection_kind) {
        return Err(failure(format!(
            "{corpus_entry_name} should name rejection kind {rejection_kind:?}: {feedback:?}"
        )));
    }
    let expected_action_list = frozen_recovery_actions(&frozen.additional_context, fixture_roots)
        .iter()
        .map(|action| format!("`{action}`"))
        .collect::<Vec<_>>()
        .join(" or ");
    if expected_action_list.is_empty() {
        return Err(failure(format!(
            "{corpus_entry_name} should freeze at least one recovery action"
        )));
    }
    if feedback.additional_context.contains(&expected_action_list) {
        return Ok(());
    }
    Err(failure(format!(
        "{corpus_entry_name} changed its frozen recovery actions:\nexpected={expected_action_list:?}\nproduced={feedback:?}"
    )))
}

// ---------------------------------------------------------------------------
// `hook post-tool-use` and `hook session-start`
// ---------------------------------------------------------------------------

#[test]
fn post_tool_use_with_nothing_to_report_emits_nothing() -> TestResult {
    let repository = committed_configuration_repository()?;
    let claimed = run_berth_with_session(
        repository.path(),
        &["claim", "file:seed.rs", "--json"],
        QUIET_DRIFT_SESSION,
    )?;
    require_success(&claimed, "quiet drift claim")?;

    let output = run_post_tool_use(
        repository.path(),
        &bash_payload(repository.path(), QUIET_DRIFT_SESSION),
    )?;

    assert_hook_stayed_silent(&output, POST_TOOL_USE_SILENT_CLEAR_ENTRY)?;
    assert_hook_stayed_silent(&output, POST_TOOL_USE_SILENT_INCURSION_FREE_ENTRY)
}

#[test]
fn post_tool_use_states_the_auto_widen_notice() -> TestResult {
    let repository = committed_configuration_repository()?;
    let claimed = run_berth_with_session(
        repository.path(),
        &["claim", "file:seed.rs", "--json"],
        WIDENING_SESSION,
    )?;
    require_success(&claimed, "widening claim")?;
    let reservation_id = claimed_reservation_id(&claimed)?;
    fs::write(repository.path().join("widened.rs"), "// widened\n")?;

    let output = run_post_tool_use(
        repository.path(),
        &bash_payload(repository.path(), WIDENING_SESSION),
    )?;

    assert_post_tool_use_feedback_matches_corpus(
        &output,
        POST_TOOL_USE_WIDENED_ENTRY,
        &[CorpusIdentifier {
            observed: reservation_id,
            frozen:   "reservation-widened".to_owned(),
        }],
    )
}

#[test]
fn post_tool_use_states_the_incursion_resolve_instruction() -> TestResult {
    let incursion = incursion_after_bash(1)?;

    assert_post_tool_use_feedback_matches_corpus(
        &incursion.output,
        POST_TOOL_USE_INCURSION_ENTRY,
        &incursion.corpus_identifiers(),
    )?;
    assert_post_tool_use_feedback_matches_corpus(
        &incursion.output,
        POST_TOOL_USE_OUTSTANDING_INCURSION_ENTRY,
        &incursion.corpus_identifiers(),
    )
}

#[test]
fn post_tool_use_states_every_incursion_of_one_bash_call() -> TestResult {
    let incursion = incursion_after_bash(4)?;

    assert_post_tool_use_feedback_matches_corpus(
        &incursion.output,
        POST_TOOL_USE_EVERY_INCURSION_ENTRY,
        &incursion.corpus_identifiers(),
    )
}

#[test]
fn post_tool_use_states_the_replay_failure_route() -> TestResult {
    let repository = typed_replay_failure_repository()?;

    let output = run_post_tool_use(
        repository.path(),
        &bash_payload(repository.path(), REPLAY_DRIFT_SESSION),
    )?;

    assert_post_tool_use_feedback_matches_corpus(&output, POST_TOOL_USE_REPLAY_ENTRY, &[])
}

#[test]
fn post_tool_use_states_its_coordination_identity_recovery() -> TestResult {
    let repository = committed_configuration_repository()?;
    create_stale_session_mapping(&repository, STALE_DRIFT_SESSION)?;

    let output = run_post_tool_use(
        repository.path(),
        &bash_payload(repository.path(), STALE_DRIFT_SESSION),
    )?;

    let repository_root = fs::canonicalize(repository.path())?;
    assert_post_tool_use_corpus_recovery(
        &output,
        POST_TOOL_USE_STALE_SESSION_ENTRY,
        &[(REPOSITORY_FIXTURE_ROOT, &repository_root)],
    )
}

/// Where the fail-closed identity binding is pinned, because this is where it has a consumer.
///
/// `hook post-tool-use` runs drift, and drift resolves an `EditAuthorization` and validates
/// the coordination identity it selected, so the session this process is bound to reaches the
/// reader. The environment names a session whose mapping outlived its reservation, and the
/// payload names a session with no mapping at all. Deleting `select_for_current_process` from
/// `ObservedBashCall::enter_current_process` leaves nothing published, so
/// `HarnessSessionId::from_current_process` falls back to `CARGO_BERTH_SESSION_ID`, the stale
/// mapping is adopted, and the recovery this asserts is absent appears instead.
///
/// Three arms, each answering one way the assertion could pass for the wrong reason: the
/// control proves the ambient value is live in this repository, the adopted arm proves the
/// stale mapping does produce the recovery when it is the session in force, and the arm under
/// test proves the payload session wins over the ambient one.
#[test]
fn post_tool_use_runs_drift_under_the_payload_session_not_the_ambient_one() -> TestResult {
    let repository = committed_configuration_repository()?;
    create_stale_session_mapping(&repository, AMBIENT_STALE_SESSION)?;

    let control = run_berth_with_session(
        repository.path(),
        &["check", "file:control.rs", "--json"],
        AMBIENT_STALE_SESSION,
    )?;
    let control_answer = String::from_utf8(control.stdout)?;
    assert!(
        control_answer.contains(STALE_SESSION_REJECTION_KIND),
        "the ambient session should be live enough to be refused when it is adopted: {control_answer}"
    );

    let adopted = run_post_tool_use_with_ambient_session(
        repository.path(),
        &bash_payload(repository.path(), AMBIENT_STALE_SESSION),
        &AmbientHarnessSession::Present(AMBIENT_STALE_SESSION),
    )?;
    let adopted_answer = String::from_utf8(adopted.stdout)?;
    assert!(
        adopted_answer.contains(STALE_SESSION_REJECTION_KIND),
        "a payload naming the stale session should reach its recovery: {adopted_answer}"
    );

    let unmapped = run_post_tool_use_with_ambient_session(
        repository.path(),
        &bash_payload(repository.path(), UNMAPPED_DRIFT_SESSION),
        &AmbientHarnessSession::Present(AMBIENT_STALE_SESSION),
    )?;
    let unmapped_answer = String::from_utf8(unmapped.stdout)?;
    assert!(
        !unmapped_answer.contains(STALE_SESSION_REJECTION_KIND),
        "post-tool-use drift adopted the ambient session identity: {unmapped_answer}"
    );
    Ok(())
}

#[test]
fn post_tool_use_states_the_unreadable_ledger_message() -> TestResult {
    let repository = unreadable_ledger_repository()?;

    let output = run_post_tool_use(
        repository.path(),
        &bash_payload(repository.path(), UNREADABLE_DRIFT_SESSION),
    )?;

    let feedback = hook_feedback(
        &output,
        HookResponseEvent::PostToolUse,
        "unreadable ledger drift",
    )?;
    assert!(
        feedback
            .additional_context
            .contains("The reservation ledger could not be read"),
        "an unreadable ledger should reach the user as the engine's own message: {feedback:?}"
    );
    Ok(())
}

#[test]
fn post_tool_use_on_an_unconfigured_repository_says_nothing() -> TestResult {
    let repository = git_repository()?;

    let output = run_post_tool_use(
        repository.path(),
        &bash_payload(repository.path(), UNCONFIGURED_DRIFT_SESSION),
    )?;

    assert_hook_output(&output, 0, b"", b"")
}

/// The two refusals this verb can state are different refusals, and the reader is told which.
///
/// Both conditions stop the reader, so the word `STOP` cannot tell them apart, and the
/// working-directory summary is byte-identical to the unstated-condition summary a
/// post-Bash response falls back to, so a summary alone cannot either. Each refusal is
/// therefore asserted against the exact summary and detail its own condition states, which
/// pins which condition produced the response rather than only that the two responses
/// differ.
#[test]
fn post_tool_use_rejects_a_payload_it_cannot_read() -> TestResult {
    let repository = committed_configuration_repository()?;

    let malformed =
        run_post_tool_use_stdin(repository.path(), b"this is not a PostToolUse payload")?;
    let malformed_feedback = hook_feedback(
        &malformed,
        HookResponseEvent::PostToolUse,
        "malformed payload",
    )?;
    assert_eq!(
        malformed_feedback.system_message, POST_TOOL_USE_INVALID_PAYLOAD_SUMMARY,
        "a payload cargo-berth cannot read should state the invalid-payload summary"
    );
    assert_eq!(
        malformed_feedback.additional_context, POST_TOOL_USE_INVALID_PAYLOAD_DETAIL,
        "a payload cargo-berth cannot read should state the invalid-payload detail, which \
         stops the reader and names the drift command to run by hand"
    );

    let absent_working_directory = run_post_tool_use(
        repository.path(),
        &serde_json::json!({
            "tool_name": "Bash",
            "cwd": repository.path().join("no-such-working-directory"),
            "tool_input": {"command": "true"},
            "session_id": UNREADABLE_DRIFT_SESSION,
        }),
    )?;
    let absent_feedback = hook_feedback(
        &absent_working_directory,
        HookResponseEvent::PostToolUse,
        "absent working directory",
    )?;
    assert_eq!(
        absent_feedback.system_message, POST_TOOL_USE_UNAVAILABLE_WORKING_DIRECTORY_SUMMARY,
        "a working directory the hook cannot enter should state the unavailable-directory \
         summary"
    );
    assert_eq!(
        absent_feedback.additional_context, POST_TOOL_USE_UNAVAILABLE_WORKING_DIRECTORY_DETAIL,
        "a working directory the hook cannot enter should state the unavailable-directory \
         detail, which stops the reader and names the working directory"
    );

    assert_ne!(
        malformed_feedback.system_message, absent_feedback.system_message,
        "a payload the verb cannot read and a working directory it cannot enter are different \
         conditions and must not share one summary"
    );
    assert_ne!(
        malformed_feedback.additional_context, absent_feedback.additional_context,
        "a payload the verb cannot read and a working directory it cannot enter are different \
         conditions and must not share one detail"
    );
    Ok(())
}

/// The user-actionable acceptance test: a raw payload, and the commands it prints.
#[test]
fn a_raw_post_tool_use_payload_states_its_drift_and_resolve_instructions() -> TestResult {
    let incursion = incursion_after_bash(1)?;
    let feedback = hook_feedback(
        &incursion.output,
        HookResponseEvent::PostToolUse,
        "raw post-tool-use payload",
    )?;
    assert!(
        feedback.additional_context.contains(&format!(
            "`cargo-berth resolve {} --incursion {}`",
            incursion.straying_reservation_id, incursion.incidents[0].incident_id
        )),
        "an incursion should tell the reader the resolve command to run: {feedback:?}"
    );

    let repository = committed_configuration_repository()?;
    let unreadable = run_post_tool_use_stdin(repository.path(), b"{\"tool_name\": \"Bash\"")?;
    let unreadable_feedback = hook_feedback(
        &unreadable,
        HookResponseEvent::PostToolUse,
        "raw unreadable post-tool-use payload",
    )?;
    assert!(
        unreadable_feedback
            .additional_context
            .contains("`cargo-berth drift --reservation <id> --json`"),
        "a payload the verb cannot read should tell the reader the drift command to run by hand: \
         {unreadable_feedback:?}"
    );
    Ok(())
}

#[test]
fn session_start_publishes_the_engine_board_report() -> TestResult {
    let orphans = orphaned_reservations(2)?;

    let output = run_session_start(
        orphans.repository.path(),
        &session_start_payload(orphans.repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_session_start_feedback_matches_corpus(
        &output,
        ORPHAN_SESSION_START_ENTRY,
        &orphans.corpus_identifiers()?,
    )
}

#[test]
fn session_start_on_a_quiet_board_emits_nothing() -> TestResult {
    let repository = committed_configuration_repository()?;

    let output = run_session_start(
        repository.path(),
        &session_start_payload(repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_hook_output(&output, 0, b"", b"")
}

#[test]
fn session_start_states_the_replay_failure_route() -> TestResult {
    let repository = typed_replay_failure_repository()?;

    let output = run_session_start(
        repository.path(),
        &session_start_payload(repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_session_start_feedback_matches_corpus(&output, SESSION_START_REPLAY_ENTRY, &[])
}

/// The board verb states this condition itself, rather than session-start restating it.
///
/// The engine message alone is not the contract here: it opens with the same sentence
/// whether the board supplied a presentation or supplied none, so a comparison against it
/// passes under either. The summary and the instruction appended after the message are what
/// only the board presentation produces, so both are compared.
#[test]
fn session_start_states_the_unreadable_ledger_presentation() -> TestResult {
    let repository = unreadable_ledger_repository()?;

    let output = run_session_start(
        repository.path(),
        &session_start_payload(repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    let feedback = hook_feedback(
        &output,
        HookResponseEvent::SessionStart,
        "unreadable ledger board",
    )?;
    assert_eq!(
        feedback.system_message, SESSION_START_LEDGER_UNREADABLE_SUMMARY,
        "an unreadable ledger should reach the user under the board's own heading: {feedback:?}"
    );
    assert!(
        feedback
            .additional_context
            .contains("The reservation ledger could not be read"),
        "an unreadable ledger should reach the user as the engine's own message: {feedback:?}"
    );
    assert!(
        feedback
            .additional_context
            .contains(SESSION_START_LEDGER_UNREADABLE_DETAIL),
        "an unreadable ledger should tell the reader what to do next: {feedback:?}"
    );
    Ok(())
}

/// A ledger another operation still holds is a second board condition with its own words.
#[test]
fn session_start_states_the_ledger_contention_presentation() -> TestResult {
    let repository = initialized_repository()?;
    let competing_lock = File::options()
        .read(true)
        .write(true)
        .open(repository.path().join(MUTATION_LOCK_PATH))?;
    competing_lock
        .try_lock()
        .map_err(|_| failure("the competing mutation lock should start free"))?;

    let output = run_session_start(
        repository.path(),
        &session_start_payload(repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;
    drop(competing_lock);

    let feedback = hook_feedback(&output, HookResponseEvent::SessionStart, "contended board")?;
    assert_eq!(
        feedback.system_message, SESSION_START_CONTENTION_SUMMARY,
        "a contended ledger should reach the user under the board's own heading: {feedback:?}"
    );
    assert!(
        feedback
            .additional_context
            .contains(SESSION_START_CONTENTION_DETAIL),
        "a contended ledger should state what the engine already spent and what to run: \
         {feedback:?}"
    );
    Ok(())
}

#[test]
fn session_start_on_an_unconfigured_repository_says_nothing() -> TestResult {
    let repository = git_repository()?;

    let output = run_session_start(
        repository.path(),
        &session_start_payload(repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_hook_output(&output, 0, b"", b"")
}

#[test]
fn session_start_reads_the_working_directory_its_payload_names() -> TestResult {
    let orphans = orphaned_reservations(2)?;
    let elsewhere = TempDir::new_in(SCRATCH_ROOT)?;

    let output = run_session_start(
        elsewhere.path(),
        &session_start_payload(orphans.repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_session_start_feedback_matches_corpus(
        &output,
        ORPHAN_SESSION_START_ENTRY,
        &orphans.corpus_identifiers()?,
    )
}

#[test]
fn session_start_rejects_a_payload_it_cannot_read() -> TestResult {
    let repository = committed_configuration_repository()?;

    let malformed =
        run_session_start_stdin(repository.path(), b"this is not a SessionStart payload")?;
    let malformed_feedback = hook_feedback(
        &malformed,
        HookResponseEvent::SessionStart,
        "malformed payload",
    )?;
    assert!(
        !malformed_feedback.additional_context.is_empty(),
        "a payload cargo-berth cannot read should still tell the reader something: {malformed_feedback:?}"
    );

    let absent = run_session_start(
        repository.path(),
        &serde_json::json!({
            "cwd": repository.path().join("no-such-working-directory"),
            "session_id": BOARD_SESSION,
        }),
        &AmbientHarnessSession::Absent,
    )?;
    let absent_feedback = hook_feedback(
        &absent,
        HookResponseEvent::SessionStart,
        "absent working directory",
    )?;
    assert!(
        !absent_feedback.additional_context.is_empty(),
        "a working directory that does not exist should still tell the reader something: {absent_feedback:?}"
    );
    Ok(())
}

/// A deliberate divergence from `berth_session_start.sh`, not parity with it.
///
/// The installed shell hook reads no `session_id` and sets no `CARGO_BERTH_SESSION_ID`, so
/// whatever the surrounding process exported stayed visible to the `board` it ran. The verb
/// publishes a no-session selection when the payload names no usable session, so an ambient
/// value cannot reach identity selection. The control below proves the ambient value is live
/// in this repository: a verb that does consult it refuses under the same environment.
///
/// What this cannot reach, stated rather than implied. The board verb does resolve a harness
/// session identity on every run — `board::execute` reconciles, and `reconcile_with_open_ledger`
/// reaches `EditAuthorization::resolve_for_worktree`, which resolves the session mapping — and
/// then throws the result away: it asks that authorization for
/// `journal_mutation_actor_for(CoordinationRunId::new())`, and that method discards the
/// authorization's own coordination run in favour of the caller's fresh one, keeping only the
/// worktree, which never depends on the session. So session-start's response is invariant under
/// the selection by construction, and no comparison of that response can fail when the binding
/// is removed. The guarantee is pinned where it has a consumer that reads the resolved identity
/// instead of discarding it, by
/// `post_tool_use_runs_drift_under_the_payload_session_not_the_ambient_one`; both verbs bind
/// through the same `select_for_current_process` helper. What this test does prove is the half
/// it can: the ambient value is live in this repository, and this response does not carry it.
#[test]
fn session_start_answers_the_same_with_and_without_an_ambient_session_identity() -> TestResult {
    let repository = committed_configuration_repository()?;
    create_stale_session_mapping(&repository, AMBIENT_STALE_SESSION)?;

    let control = run_berth_with_session(
        repository.path(),
        &["check", "file:control.rs", "--json"],
        AMBIENT_STALE_SESSION,
    )?;
    let control_message = String::from_utf8(control.stdout)?;
    assert!(
        control_message.contains(STALE_SESSION_REJECTION_KIND),
        "the ambient session should be live enough to be refused when it is adopted: {control_message}"
    );

    let payload_without_a_session = session_start_payload(repository.path(), None);
    let with_ambient = run_session_start(
        repository.path(),
        &payload_without_a_session,
        &AmbientHarnessSession::Present(AMBIENT_STALE_SESSION),
    )?;
    let without_ambient = run_session_start(
        repository.path(),
        &payload_without_a_session,
        &AmbientHarnessSession::Absent,
    )?;

    assert_eq!(with_ambient.status.code(), without_ambient.status.code());
    assert_eq!(with_ambient.stdout, without_ambient.stdout);
    assert_eq!(with_ambient.stderr, without_ambient.stderr);
    let answer = String::from_utf8(with_ambient.stdout)?;
    assert!(
        !answer.contains(STALE_SESSION_REJECTION_KIND),
        "session start adopted the ambient session identity: {answer}"
    );

    let unusable_session_payload = session_start_payload(repository.path(), Some(""));
    let with_unusable_session = run_session_start(
        repository.path(),
        &unusable_session_payload,
        &AmbientHarnessSession::Present(AMBIENT_STALE_SESSION),
    )?;
    let unusable_answer = String::from_utf8(with_unusable_session.stdout)?;
    assert!(
        !unusable_answer.contains(STALE_SESSION_REJECTION_KIND),
        "an unusable payload session fell through to the ambient one: {unusable_answer}"
    );
    Ok(())
}

/// The user-actionable acceptance test: a raw payload, and the commands it prints.
#[test]
fn a_raw_session_start_payload_states_its_resolve_instructions() -> TestResult {
    let orphans = orphaned_reservations(2)?;

    let output = run_session_start(
        orphans.repository.path(),
        &session_start_payload(orphans.repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    let feedback = hook_feedback(
        &output,
        HookResponseEvent::SessionStart,
        "raw session-start payload",
    )?;
    for orphan in &orphans.orphans {
        assert!(
            feedback.additional_context.contains(&format!(
                "`cargo-berth resolve {} --recovered`",
                orphan.reservation_id
            )),
            "an orphaned reservation should tell the reader the resolve command to run: {feedback:?}"
        );
    }
    Ok(())
}

/// A populated board with nothing actionable keeps the report block's own summary.
///
/// `berth_session_start.sh` counted rendered blocks, so a board carrying only its complete
/// report announced one actionable coordination notice when there were none. The verb
/// states the leading block's own summary instead, which is a deliberate divergence from
/// the installed hook rather than parity with it.
#[test]
fn session_start_never_counts_the_board_report_as_an_actionable_notice() -> TestResult {
    let repository = committed_configuration_repository()?;
    let claimed = run_berth_with_session(
        repository.path(),
        &["claim", "file:settled.rs", "--json"],
        BOARD_SESSION,
    )?;
    require_success(&claimed, "settled claim")?;
    let released = run_berth(
        repository.path(),
        &["release", &claimed_reservation_id(&claimed)?, "--json"],
    )?;
    require_success(&released, "settled release")?;

    let output = run_session_start(
        repository.path(),
        &session_start_payload(repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    let feedback = hook_feedback(&output, HookResponseEvent::SessionStart, "settled board")?;
    assert!(
        !feedback
            .system_message
            .contains("actionable coordination notice"),
        "a board with nothing actionable announced an actionable notice: {feedback:?}"
    );
    assert!(
        feedback
            .additional_context
            .contains("cargo-berth read the complete reservation board report."),
        "a populated board should still publish the engine's complete report: {feedback:?}"
    );
    Ok(())
}

/// A recorded incursion stops repeating itself, and the corpus froze that silence.
///
/// Answering an incident is not the same state as never having entered a foreign reservation:
/// the incident stays on the board under its recorded answers. What must not survive the answer
/// is the stop instruction, so the next Bash call from the same worktree prints nothing at all.
#[test]
fn post_tool_use_stops_repeating_an_answered_incursion() -> TestResult {
    let incursion = incursion_after_bash(1)?;
    incursion.record_every_incursion()?;

    let output = incursion.report_another_bash_call()?;

    assert_hook_stayed_silent(&output, POST_TOOL_USE_RECORDED_INCURSION_SILENT_ENTRY)
}

/// The silence is specific to the answered incident, not to the whole response.
#[test]
fn post_tool_use_states_a_widening_after_the_incursion_was_answered() -> TestResult {
    let incursion = incursion_after_bash(1)?;
    incursion.record_every_incursion()?;
    fs::write(
        incursion.straying_root.join(WIDENED_AFTER_INCURSION_SOURCE),
        "// widened after the answer\n",
    )?;

    let output = incursion.report_another_bash_call()?;

    assert_post_tool_use_feedback_matches_corpus(
        &output,
        POST_TOOL_USE_RECORDED_INCURSION_WIDENED_ENTRY,
        &incursion.corpus_identifiers(),
    )
}

/// An alert the recorded incursion never touched still reaches the reader after the answer.
#[test]
fn post_tool_use_states_lost_evidence_after_the_incursion_was_answered() -> TestResult {
    let incursion = incursion_after_bash(1)?;
    incursion.record_every_incursion()?;
    let (evidence, trunk) = lose_integration_evidence(
        &incursion.repository,
        incursion.worktrees.path(),
        &IntegrationProofLoss::TrunkRewrittenPastTheTip,
    )?;

    let output = incursion.report_another_bash_call()?;

    assert_post_tool_use_feedback_matches_corpus(
        &output,
        POST_TOOL_USE_RECORDED_INCURSION_LOST_EVIDENCE_ENTRY,
        &evidence.corpus_identifiers(&trunk),
    )
}

/// A trunk that resolves but has moved past the tip gets the recovery that names it.
#[test]
fn post_tool_use_states_the_rewritten_trunk_evidence_recovery() -> TestResult {
    let (evidence, trunk, _repository, _worktrees) =
        released_work_trunk_cannot_prove(&IntegrationProofLoss::TrunkRewrittenPastTheTip)?;

    let output = run_post_tool_use(
        &evidence.reporting_root,
        &bash_payload(&evidence.reporting_root, EVIDENCE_SESSION),
    )?;

    assert_post_tool_use_feedback_matches_corpus(
        &output,
        POST_TOOL_USE_LOST_EVIDENCE_REWRITTEN_ENTRY,
        &evidence.corpus_identifiers(&trunk),
    )
}

/// A trunk that resolves to nothing cannot be named, so the recovery asks for trunk first.
#[test]
fn post_tool_use_states_the_unresolvable_trunk_evidence_recovery() -> TestResult {
    let (evidence, trunk, _repository, _worktrees) =
        released_work_trunk_cannot_prove(&IntegrationProofLoss::TrunkNamesAnAbsentObject)?;

    let output = run_post_tool_use(
        &evidence.reporting_root,
        &bash_payload(&evidence.reporting_root, EVIDENCE_SESSION),
    )?;

    assert_post_tool_use_feedback_matches_corpus(
        &output,
        POST_TOOL_USE_LOST_EVIDENCE_UNRESOLVABLE_ENTRY,
        &evidence.corpus_identifiers(&trunk),
    )
}

/// Both events state the same recovery, so session-start carries it inside its board report.
#[test]
fn session_start_states_the_rewritten_trunk_evidence_recovery() -> TestResult {
    let (evidence, trunk, _repository, _worktrees) =
        released_work_trunk_cannot_prove(&IntegrationProofLoss::TrunkRewrittenPastTheTip)?;

    let output = run_session_start(
        &evidence.reporting_root,
        &session_start_payload(&evidence.reporting_root, Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_session_start_feedback_matches_corpus(
        &output,
        SESSION_START_LOST_EVIDENCE_REWRITTEN_ENTRY,
        &evidence.corpus_identifiers(&trunk),
    )
}

/// The unresolvable recovery reaches a starting session under the same words.
#[test]
fn session_start_states_the_unresolvable_trunk_evidence_recovery() -> TestResult {
    let (evidence, trunk, _repository, _worktrees) =
        released_work_trunk_cannot_prove(&IntegrationProofLoss::TrunkNamesAnAbsentObject)?;

    let output = run_session_start(
        &evidence.reporting_root,
        &session_start_payload(&evidence.reporting_root, Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_session_start_feedback_matches_corpus(
        &output,
        SESSION_START_LOST_EVIDENCE_UNRESOLVABLE_ENTRY,
        &evidence.corpus_identifiers(&trunk),
    )
}

/// An orphan whose commit is gone can only be retired or abandoned, never recovered.
#[test]
fn session_start_states_the_unavailable_orphan_recovery_actions() -> TestResult {
    let orphans = orphaned_reservations_with(2, OrphanedCommitAvailability::Unavailable)?;

    let output = run_session_start(
        orphans.repository.path(),
        &session_start_payload(orphans.repository.path(), Some(BOARD_SESSION)),
        &AmbientHarnessSession::Absent,
    )?;

    assert_session_start_feedback_matches_corpus(
        &output,
        SESSION_START_UNAVAILABLE_ORPHAN_ENTRY,
        &orphans.corpus_identifiers_for(SESSION_START_UNAVAILABLE_ORPHAN_ENTRY)?,
    )
}

/// Build the released-and-proven reservation whose proof was then taken away.
///
/// The repository and its worktrees are returned alongside so the caller keeps them alive for
/// the duration of the hook run; dropping either deletes the scratch tree out from under it.
fn released_work_trunk_cannot_prove(
    loss: &IntegrationProofLoss,
) -> TestResult<(LostIntegrationEvidence, ObservedTrunk, TempDir, TempDir)> {
    let repository = committed_configuration_repository()?;
    let worktrees = TempDir::new_in(SCRATCH_ROOT)?;
    let (evidence, trunk) = lose_integration_evidence(&repository, worktrees.path(), loss)?;
    Ok((evidence, trunk, repository, worktrees))
}

/// The retired two-step route is gone from the command line, not merely unused by the hook.
///
/// `hook post-tool-use` reads its payload from standard input and completes its answer in one
/// process. The hidden `--post-tool-use-payload` flag on `drift` and `board` was the other half
/// of the two-step route the front end drove before it became a pass-through, and a flag the
/// parser still accepts is a route a later change can put back into service without deciding to.
#[test]
fn the_retired_post_tool_use_payload_flag_is_no_longer_a_command_line_argument() -> TestResult {
    let repository = committed_configuration_repository()?;

    for verb in ["drift", "board"] {
        let refused = run_berth(
            repository.path(),
            &[verb, "--json", "--post-tool-use-payload"],
        )?;

        assert_ne!(
            refused.status.code(),
            Some(0),
            "`cargo-berth {verb} --json --post-tool-use-payload` should no longer be accepted"
        );
        let refusal = String::from_utf8_lossy(&refused.stderr);
        assert!(
            refusal.contains("--post-tool-use-payload"),
            "the parser should name the argument it does not recognise for {verb}: {refusal}"
        );
    }
    Ok(())
}
