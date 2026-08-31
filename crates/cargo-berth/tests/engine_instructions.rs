//! Real-binary acceptance guard for executable engine instructions.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;

const CARGO_BERTH_RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const CARGO_BERTH_SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const SCRATCH_ROOT: &str = "/tmp/claude";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const STALE_CLAIM_TIME: &str = "2020-01-01T00:00:00.000Z";

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[test]
fn rendered_shell_instructions_invoke_the_engine() -> TestResult {
    let (blocked_claim, board) = blocked_claim_and_board_envelopes()?;
    let ambiguous_first_touch = ambiguous_first_touch_envelope()?;
    let scenarios = [
        ("blocked claim", &blocked_claim),
        ("populated board", &board),
        ("ambiguous first touch", &ambiguous_first_touch),
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
    let output = Command::new("git")
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
