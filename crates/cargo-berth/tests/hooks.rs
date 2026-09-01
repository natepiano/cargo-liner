//! Built-binary acceptance tests for the public Claude `PreToolUse` hook verb.

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
const AMBIENT_SESSION: &str = "ambient-session";
const AMBIGUITY_SESSION: &str = "ambiguous-session";
const COORDINATION_IDENTITY_EDIT_SUMMARY: &str =
    "cargo-berth rejected this edit under the current coordination identity.";
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
    assert!(refusal.contains("stale_session_mapping"));
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-berth"));
    command
        .args(["hook", "pre-tool-use"])
        .current_dir(repository_root)
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
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| failure("pre-tool-use stdin should be piped"))?;
    serde_json::to_writer(&mut stdin, payload)?;
    drop(stdin);
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

fn run_git(repository_root: &Path, arguments: &[&str]) -> TestResult {
    let output = Command::new("git")
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
