//! Real-binary acceptance guard for executable engine instructions.

use cargo_berth_test_support::git_command;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

use std::error::Error;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use serde_json::Value;
use tempfile::TempDir;

const CARGO_BERTH_RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const CARGO_BERTH_SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const CHECK_LEDGER_UNREADABLE_SCENARIO: &str = "check ledger-unreadable";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const CORRUPT_JOURNAL_RECORD: &[u8] = b"this journal record is not JSON\n";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const POST_TOOL_USE_INVALID_PAYLOAD_SCENARIO: &str = "post-tool-use invalid payload";
const POST_TOOL_USE_SCENARIO: &str = "post-tool-use coordination identity recovery";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const SCRATCH_ROOT: &str = "/tmp/claude";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const SESSION_START_SCENARIO: &str = "session-start board report";
const STALE_CLAIM_TIME: &str = "2020-01-01T00:00:00.000Z";
const UNKNOWN_RESERVATION_ID: &str = "01991f4d-77d8-7f5f-9a1f-000000000001";

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[test]
fn rendered_shell_instructions_invoke_the_engine() -> TestResult {
    let (blocked_claim, board) = blocked_claim_and_board_envelopes()?;
    let ambiguous_first_touch = ambiguous_first_touch_envelope()?;
    let replay_failure = replay_failure_envelope()?;
    let post_tool_use = post_tool_use_identity_recovery_envelope()?;
    let session_start = session_start_board_envelope()?;
    let scenarios = [
        ("blocked claim", &blocked_claim),
        ("populated board", &board),
        ("ambiguous first touch", &ambiguous_first_touch),
        ("pre-tool-use replay failure", &replay_failure),
        (POST_TOOL_USE_SCENARIO, &post_tool_use),
        (SESSION_START_SCENARIO, &session_start),
    ];

    for (scenario, envelope) in scenarios {
        let shell_command_count = inspect_rendered_blocks(scenario, envelope)?;
        if shell_command_count == 0 {
            return Err(failure(format!(
                "{scenario} rendered no blocks containing shell commands"
            )));
        }
    }
    Ok(())
}

#[test]
fn the_check_verb_states_its_own_ledger_unreadable_instructions() -> TestResult {
    let envelope = check_ledger_unreadable_envelope()?;
    let scenario = CHECK_LEDGER_UNREADABLE_SCENARIO;
    let presentation_kind = required_string(&envelope, "/presentation/kind", scenario)?;
    if presentation_kind != "rendered_blocks" {
        return Err(failure(format!(
            "{scenario} should return rendered blocks, found presentation kind {presentation_kind:?}"
        )));
    }
    let blocks = required_array(&envelope, "/presentation/blocks", scenario)?;
    let [block] = blocks else {
        return Err(failure(format!(
            "{scenario} should render exactly one block, rendered {}",
            blocks.len()
        )));
    };
    inspect_rendered_block(scenario, 0, block)?;
    assert_ledger_unreadable_instruction(&envelope, block)
}

/// The refusal for a payload this verb cannot read is the engine's own instruction.
///
/// This is the sentence a reader acts on when no drift check covered their Bash call, and the
/// only text the engine prints on that path, so nothing else states it for them. It must name
/// the engine, instruct only in `cargo-berth` commands, and name every part of the payload
/// it refuses on: a clause the verb rejects but the sentence omits sends the reader
/// hunting a fault the text never mentions.
#[test]
fn the_post_tool_use_verb_states_its_own_invalid_payload_instructions() -> TestResult {
    let envelope = post_tool_use_invalid_payload_envelope()?;
    let scenario = POST_TOOL_USE_INVALID_PAYLOAD_SCENARIO;

    let shell_command_count = inspect_rendered_blocks(scenario, &envelope)?;
    if shell_command_count == 0 {
        return Err(failure(format!(
            "{scenario} rendered no blocks containing shell commands"
        )));
    }

    let summary = required_string(&envelope, "/presentation/blocks/0/summary", scenario)?;
    if !summary.contains("cargo-berth") {
        return Err(failure(format!(
            "{scenario} should name the engine that refused the payload: {summary:?}"
        )));
    }

    let detail = required_string(&envelope, "/presentation/blocks/0/detail", scenario)?;
    for refused_part in ["valid JSON", "tool_name", "session_id", "cwd"] {
        if !detail.contains(refused_part) {
            return Err(failure(format!(
                "{scenario} should name {refused_part:?} among the payload parts it refuses on: {detail:?}"
            )));
        }
    }
    Ok(())
}

/// The check verb's ledger-unreadable block must state the engine's own words.
fn assert_ledger_unreadable_instruction(envelope: &Value, block: &Value) -> TestResult {
    let scenario = CHECK_LEDGER_UNREADABLE_SCENARIO;
    let summary = required_string(block, "/summary", scenario)?;
    let detail = required_string(block, "/detail", scenario)?;
    let message = required_string(envelope, "/message", scenario)?;
    if detail != message {
        return Err(failure(format!(
            "{scenario} should state the engine's own message as its detail: message={message:?} detail={detail:?}"
        )));
    }
    if summary.is_empty() || summary == detail {
        return Err(failure(format!(
            "{scenario} should head its detail with a separate summary: summary={summary:?} detail={detail:?}"
        )));
    }
    if detail.contains("The reservation ledger could not be read") {
        return Ok(());
    }
    Err(failure(format!(
        "{scenario} should tell the reader the ledger could not be read: {detail:?}"
    )))
}

fn check_ledger_unreadable_envelope() -> TestResult<Value> {
    let repository = initialized_repository()?;
    let seed = run_berth(
        repository.path(),
        &["claim", "file:seed.rs", "--run", FIRST_RUN, "--json"],
        "engine-instructions-unreadable",
    )?;
    require_success(&seed, "unreadable ledger seed")?;
    let mut journal = OpenOptions::new()
        .append(true)
        .open(repository.path().join(JOURNAL_PATH))?;
    journal.write_all(CORRUPT_JOURNAL_RECORD)?;
    let projection_path = repository.path().join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path)?;
    }

    let unreadable = run_berth(
        repository.path(),
        &["check", "file:unreadable.rs", "--json"],
        "engine-instructions-unreadable",
    )?;
    require_exit_code(&unreadable, 4, CHECK_LEDGER_UNREADABLE_SCENARIO)?;
    json_output(&unreadable, CHECK_LEDGER_UNREADABLE_SCENARIO)
}

fn blocked_claim_and_board_envelopes() -> TestResult<(Value, Value)> {
    let repository = initialized_repository()?;
    let holder = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:crates/cargo-berth",
            "--run",
            FIRST_RUN,
            "--plan",
            "docs/holder-plan.md",
            "--phase",
            "holder-phase",
            "--why",
            "protect the holder implementation",
            "--json",
        ],
        "engine-instructions-holder",
    )?;
    require_success(&holder, "holder claim")?;
    age_only_journal_event(repository.path())?;

    let (requester_directory, requester_root) = add_worktree(&repository, "requester")?;
    let blocked = run_berth(
        &requester_root,
        &["claim", "file:crates/cargo-berth/src/main.rs", "--json"],
        "engine-instructions-requester",
    )?;
    require_exit_code(&blocked, 1, "overlapping requester claim")?;
    let blocked_envelope = json_output(&blocked, "overlapping requester claim")?;

    let board = run_berth(
        repository.path(),
        &["board", "--json"],
        "engine-instructions-board",
    )?;
    require_success(&board, "populated board")?;
    let board_envelope = json_output(&board, "populated board")?;
    drop(requester_directory);
    Ok((blocked_envelope, board_envelope))
}

fn age_only_journal_event(repository: &Path) -> TestResult {
    let journal_path = repository.join(JOURNAL_PATH);
    let journal = fs::read_to_string(&journal_path)?;
    let mut events = journal
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let [event] = events.as_mut_slice() else {
        return Err(failure(format!(
            "expected one holder claim event, found {}",
            events.len()
        )));
    };
    event["at"] = Value::String(STALE_CLAIM_TIME.to_owned());
    let mut serialized = serde_json::to_vec(event)?;
    serialized.push(b'\n');
    fs::write(journal_path, serialized)?;

    let projection_path = repository.join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path)?;
    }
    Ok(())
}

fn ambiguous_first_touch_envelope() -> TestResult<Value> {
    let repository = initialized_repository()?;
    let older_claim = run_berth(
        repository.path(),
        &["claim", "tree:shared", "--run", FIRST_RUN, "--json"],
        "engine-instructions-selection",
    )?;
    require_success(&older_claim, "older first-touch candidate")?;
    let newer_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "file:shared/child.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
        "engine-instructions-selection",
    )?;
    require_success(&newer_claim, "newer first-touch candidate")?;
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))?;

    let ambiguous = run_berth(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
        "engine-instructions-selection",
    )?;
    require_exit_code(&ambiguous, 1, "ambiguous first-touch check")?;
    json_output(&ambiguous, "ambiguous first-touch check")
}

fn replay_failure_envelope() -> TestResult<Value> {
    let repository = initialized_repository()?;
    let seed = run_berth(
        repository.path(),
        &["claim", "file:seed.rs", "--run", FIRST_RUN, "--json"],
        "engine-instructions-replay",
    )?;
    require_success(&seed, "replay fixture seed")?;
    append_unknown_release(repository.path())?;
    let projection_path = repository.path().join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path)?;
    }

    let replay_failure = run_berth(
        repository.path(),
        &["check", "file:replay.rs", "--json"],
        "engine-instructions-replay",
    )?;
    require_exit_code(&replay_failure, 4, "pre-tool-use replay failure")?;
    json_output(&replay_failure, "pre-tool-use replay failure")
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

/// The refusal `hook post-tool-use` states for a payload that reports no Bash call.
///
/// The payload is well-formed JSON naming another tool, so the verb refuses it by reading
/// the payload alone and never reaches the repository. That is the same refusal a
/// malformed body reaches, and it is the one carrying the engine's instructions.
fn post_tool_use_invalid_payload_envelope() -> TestResult<Value> {
    let repository = initialized_repository()?;

    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Read",
        "cwd": working_directory_argument(repository.path())?,
        "session_id": "engine-instructions-invalid-payload",
    });
    let refused = run_hook_verb(repository.path(), "post-tool-use", &payload)?;
    require_success(&refused, POST_TOOL_USE_INVALID_PAYLOAD_SCENARIO)?;
    hook_response_envelope(&refused, POST_TOOL_USE_INVALID_PAYLOAD_SCENARIO)
}

/// The recovery text `hook post-tool-use` publishes when drift cannot attribute a session.
///
/// Two claims share one harness session and the mapping that separated them is gone, so the
/// Bash call this payload reports cannot be attributed to a single reservation and the
/// engine answers with the commands that re-establish the session's identity.
fn post_tool_use_identity_recovery_envelope() -> TestResult<Value> {
    let repository = initialized_repository()?;
    for scope in ["tree:shared", "file:other.rs"] {
        let candidate = run_berth(
            repository.path(),
            &["claim", scope, "--json"],
            "engine-instructions-post-tool-use",
        )?;
        require_success(&candidate, "post-tool-use attribution candidate")?;
    }
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))?;
    fs::create_dir_all(repository.path().join("shared"))?;
    fs::write(repository.path().join("shared/child.rs"), "// changed\n")?;

    let payload = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "cwd": working_directory_argument(repository.path())?,
        "session_id": "engine-instructions-post-tool-use",
    });
    let answered = run_hook_verb(repository.path(), "post-tool-use", &payload)?;
    require_success(&answered, POST_TOOL_USE_SCENARIO)?;
    hook_response_envelope(&answered, POST_TOOL_USE_SCENARIO)
}

/// The board report `hook session-start` publishes to a session that has one stale claim.
fn session_start_board_envelope() -> TestResult<Value> {
    let repository = initialized_repository()?;
    let claim = run_berth(
        repository.path(),
        &["claim", "file:stale.rs", "--run", FIRST_RUN, "--json"],
        "engine-instructions-session-start",
    )?;
    require_success(&claim, "session-start stale claim")?;
    age_only_journal_event(repository.path())?;

    let payload = serde_json::json!({
        "hook_event_name": "SessionStart",
        "cwd": working_directory_argument(repository.path())?,
        "session_id": "engine-instructions-session-start",
    });
    let reconciled = run_hook_verb(repository.path(), "session-start", &payload)?;
    require_success(&reconciled, SESSION_START_SCENARIO)?;
    hook_response_envelope(&reconciled, SESSION_START_SCENARIO)
}

/// Read one hook response as the rendered block whose text the harness shows its reader.
///
/// A hook publishes the text it rendered rather than the envelope it rendered from, so the
/// response's heading and context are restored to the block this suite inspects.
fn hook_response_envelope(output: &Output, scenario: &str) -> TestResult<Value> {
    let response = json_output(output, scenario)?;
    let summary = required_string(&response, "/systemMessage", scenario)?;
    let detail = required_string(&response, "/hookSpecificOutput/additionalContext", scenario)?;
    Ok(serde_json::json!({
        "presentation": {
            "kind": "rendered_blocks",
            "blocks": [{"summary": summary, "detail": detail}],
        },
    }))
}

/// Run one public hook verb the way the harness runs it: raw payload on standard input.
fn run_hook_verb(repository: &Path, hook_event: &str, payload: &Value) -> TestResult<Output> {
    let mut hook = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["hook", hook_event])
        .current_dir(repository)
        .env_remove(CARGO_BERTH_RUN_ENVIRONMENT)
        .env_remove(CARGO_BERTH_SESSION_ENVIRONMENT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut standard_input = hook
        .stdin
        .take()
        .ok_or_else(|| failure(format!("{hook_event} hook should accept standard input")))?;
    standard_input.write_all(&serde_json::to_vec(payload)?)?;
    drop(standard_input);
    Ok(hook.wait_with_output()?)
}

fn working_directory_argument(repository: &Path) -> TestResult<String> {
    repository
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| failure("scratch repository path should be valid UTF-8"))
}

fn inspect_rendered_blocks(scenario: &str, envelope: &Value) -> TestResult<usize> {
    let presentation_kind = required_string(envelope, "/presentation/kind", scenario)?;
    if presentation_kind != "rendered_blocks" {
        return Err(failure(format!(
            "{scenario} should return rendered blocks, found presentation kind {presentation_kind:?}"
        )));
    }
    let blocks = required_array(envelope, "/presentation/blocks", scenario)?;
    if blocks.is_empty() {
        return Err(failure(format!(
            "{scenario} should expose at least one rendered block"
        )));
    }

    blocks
        .iter()
        .enumerate()
        .map(|(block_index, block)| inspect_rendered_block(scenario, block_index, block))
        .collect::<TestResult<Vec<_>>>()
        .map(|shell_command_counts| shell_command_counts.into_iter().sum())
}

fn inspect_rendered_block(scenario: &str, block_index: usize, block: &Value) -> TestResult<usize> {
    let summary = required_string(block, "/summary", scenario)?;
    let detail = required_string(block, "/detail", scenario)?;
    let fields = [("summary", summary), ("detail", detail)];
    for (field, text) in fields {
        assert_no_coordinator_instruction(scenario, block_index, field, text)?;
    }

    let shell_commands = fields
        .into_iter()
        .flat_map(|(_, text)| text.lines())
        .flat_map(inline_code_spans)
        .filter(|span| is_shell_command(span))
        .collect::<Vec<_>>();
    if shell_commands.is_empty() {
        return Ok(0);
    }
    let offending_shell_commands = shell_commands
        .iter()
        .copied()
        .filter(|command| !command.starts_with("cargo-berth"))
        .collect::<Vec<_>>();
    if !offending_shell_commands.is_empty() {
        return Err(failure(format!(
            "{scenario} rendered block {block_index} contains shell commands that do not begin with cargo-berth: {offending_shell_commands:?}"
        )));
    }
    Ok(shell_commands.len())
}

fn assert_no_coordinator_instruction(
    scenario: &str,
    block_index: usize,
    field: &str,
    text: &str,
) -> TestResult {
    for (line_index, line) in text.lines().enumerate() {
        for forbidden in ["python3 -m berth", "PYTHONPATH=", "berth.claim_state"] {
            if line.contains(forbidden) {
                return Err(failure(format!(
                    "{scenario} rendered block {block_index} {field} line {} instructs the reader to run the coordinator ({forbidden:?}): {line:?}",
                    line_index + 1
                )));
            }
        }
    }
    Ok(())
}

fn inline_code_spans(line: &str) -> impl Iterator<Item = &str> {
    let mut parts = line.split('`');
    std::iter::from_fn(move || {
        parts.next()?;
        parts.next()
    })
}

fn is_shell_command(span: &str) -> bool {
    let Some(executable) = span.split_whitespace().next() else {
        return false;
    };
    executable.starts_with("PYTHONPATH=")
        || matches!(
            executable,
            "bash"
                | "board"
                | "cargo"
                | "cargo-berth"
                | "check"
                | "claim"
                | "drift"
                | "git"
                | "identity"
                | "integrate"
                | "python3"
                | "release"
                | "renew"
                | "resolve"
                | "sequence"
                | "sh"
        )
}

fn initialized_repository() -> TestResult<TempDir> {
    fs::create_dir_all(SCRATCH_ROOT)?;
    let repository = TempDir::new_in(SCRATCH_ROOT)?;
    run_git(repository.path(), &["init", "-b", "main"])?;
    run_git(
        repository.path(),
        &[
            "config",
            "user.email",
            "engine-instructions@example.invalid",
        ],
    )?;
    run_git(
        repository.path(),
        &["config", "user.name", "Engine Instructions Test"],
    )?;
    fs::write(repository.path().join("README.md"), "scratch repository\n")?;
    run_git(repository.path(), &["add", "README.md"])?;
    run_git(
        repository.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
    )?;
    let initialized = run_berth(
        repository.path(),
        &["init", "--json"],
        "engine-instructions-init",
    )?;
    require_success(&initialized, "cargo-berth init")?;
    Ok(repository)
}

fn add_worktree(repository: &TempDir, branch: &str) -> TestResult<(TempDir, PathBuf)> {
    let directory = TempDir::new_in(SCRATCH_ROOT)?;
    let root = directory.path().join(branch);
    let worktree_path = root
        .to_str()
        .ok_or_else(|| failure("scratch worktree path should be valid UTF-8"))?;
    run_git(
        repository.path(),
        &["worktree", "add", "--quiet", "-b", branch, worktree_path],
    )?;
    let configuration_path = root.join(CONFIGURATION_PATH);
    let configuration_directory = configuration_path
        .parent()
        .ok_or_else(|| failure("cargo-berth configuration should have a parent directory"))?;
    fs::create_dir_all(configuration_directory)?;
    fs::copy(
        repository.path().join(CONFIGURATION_PATH),
        configuration_path,
    )?;
    Ok((directory, root))
}

fn run_git(repository: &Path, arguments: &[&str]) -> TestResult {
    let output = git_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(repository)
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(failure(format!(
        "git {arguments:?} failed in scratch repository: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )))
}

fn run_berth(repository: &Path, arguments: &[&str], session_id: &str) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository)
        .env_remove(CARGO_BERTH_RUN_ENVIRONMENT)
        .env(CARGO_BERTH_SESSION_ENVIRONMENT, session_id)
        .output()?)
}

fn require_success(output: &Output, operation: &str) -> TestResult {
    if output.status.success() {
        return Ok(());
    }
    Err(command_failure(output, operation))
}

fn require_exit_code(output: &Output, expected: i32, operation: &str) -> TestResult {
    if output.status.code() == Some(expected) {
        return Ok(());
    }
    Err(command_failure(output, operation))
}

fn command_failure(output: &Output, operation: &str) -> Box<dyn Error> {
    failure(format!(
        "{operation} exited with {:?}: stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn json_output(output: &Output, operation: &str) -> TestResult<Value> {
    serde_json::from_slice(&output.stdout).map_err(|error| {
        failure(format!(
            "{operation} stdout should be a JSON envelope: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })
}

fn required_string<'value>(
    value: &'value Value,
    pointer: &str,
    context: &str,
) -> TestResult<&'value str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{context} {pointer} should be a string in {value}")))
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
        .ok_or_else(|| failure(format!("{context} {pointer} should be an array in {value}")))
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}
