//! Installed-reader checks over an unchanged journal containing merge extent evidence.
//!
//! Set `CARGO_BERTH_EXECUTABLE` to the reader under inspection. Each test restores its
//! own repository, so an incompatible reader cannot damage another test's ledger.

#[path = "support/reader_compat_hooks.rs"]
mod reader_compat_hooks;

use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use cargo_berth_test_support::EXECUTABLE_ENVIRONMENT;
use cargo_berth_test_support::git_command;
use reader_compat_hooks::AmbientHarnessSession;
use reader_compat_hooks::HookResponseEvent;
use reader_compat_hooks::expected_hook_feedback;
use reader_compat_hooks::hook_feedback;
use reader_compat_hooks::spawn_hook_verb;
use serde_json::Value;
use tempfile::TempDir;

/// Setup uses this binary only to renew activity, keeping the frozen journal intact.
const BUILT_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
/// The fixture's historical records must be present before any reader is invoked.
const JOURNAL: &str = include_str!("fixtures/reader_compat/journal.ndjson");
/// Every journal access is relative to the newly restored temporary repository.
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
/// The session payload deliberately overrides any ambient harness identity.
const SESSION: &str = "reader-compat";
/// Frozen hook reports carry this heading before their JSON board contents.
const BOARD_REPORT_PREFIX: &str = "cargo-berth read the complete reservation board report.\n\n";

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[test]
fn board_reads_merge_extent_fixture() -> TestResult {
    assert_command(
        &["board", "--json"],
        include_str!("fixtures/reader_compat/board.json"),
    )
}

#[test]
fn check_reads_merge_extent_fixture() -> TestResult {
    assert_command(
        &["check", "file:claimed.txt", "--json"],
        include_str!("fixtures/reader_compat/check.json"),
    )
}

#[test]
fn drift_reads_merge_extent_fixture() -> TestResult {
    assert_command(
        &["drift", "--full", "--json"],
        include_str!("fixtures/reader_compat/drift.json"),
    )
}

#[test]
fn session_start_reads_merge_extent_fixture() -> TestResult {
    let repository = fixture_repository()?;
    let payload = serde_json::json!({
        "hook_event_name": "SessionStart",
        "cwd": repository.path(),
        "source": "startup",
        "session_id": SESSION,
    });
    let output = spawn_hook_verb(
        &reader_executable()?,
        repository.path(),
        "session-start",
        &serde_json::to_vec(&payload)?,
        &AmbientHarnessSession::Present("ignored-ambient-session"),
    )?;
    assert_hook_response(
        &output,
        HookResponseEvent::SessionStart,
        repository.path(),
        include_str!("fixtures/reader_compat/session-start.json"),
    )
}

#[test]
fn post_tool_use_reads_merge_extent_fixture() -> TestResult {
    let repository = fixture_repository()?;
    fs::write(
        repository.path().join("additional.txt"),
        "new work after Bash\n",
    )?;
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "cwd": repository.path(),
        "tool_input": {"command": "true"},
        "session_id": SESSION,
    });
    let output = spawn_hook_verb(
        &reader_executable()?,
        repository.path(),
        "post-tool-use",
        &serde_json::to_vec(&payload)?,
        &AmbientHarnessSession::Absent,
    )?;
    assert_hook_response(
        &output,
        HookResponseEvent::PostToolUse,
        repository.path(),
        include_str!("fixtures/reader_compat/post-tool-use.json"),
    )
}

#[test]
fn init_pins_legacy_claim_target_once_and_keeps_it_after_trunk_edit() -> TestResult {
    let repository = fixture_repository()?;
    let root = repository.path();
    let before = fs::read(root.join(JOURNAL_PATH))?;
    let first = run_reader(Path::new(BUILT_EXECUTABLE), root, &["init", "--json"])?;
    require_readable(&first, "first migration init")?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    let response: Value = serde_json::from_slice(&first.stdout)?;
    let pinned = &response["payload"]["data"]["targets_pinned"];
    assert_eq!(pinned["target"], "refs/heads/main");
    assert_eq!(pinned["reservations"].as_array().map(Vec::len), Some(1));
    let after_first = fs::read(root.join(JOURNAL_PATH))?;
    assert!(
        after_first.starts_with(&before),
        "migration preserves the fixture prefix"
    );
    let pin_events = |bytes: &[u8]| -> TestResult<Vec<Value>> {
        Ok(String::from_utf8(bytes.to_vec())?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|event| event["op"] == "unrecorded_targets_pinned")
            .collect())
    };
    let pins = pin_events(&after_first)?;
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0]["target"], "refs/heads/main");

    let second = run_reader(Path::new(BUILT_EXECUTABLE), root, &["init", "--json"])?;
    require_readable(&second, "second migration init")?;
    assert!(second.status.success());
    assert!(
        serde_json::from_slice::<Value>(&second.stdout)?["payload"]["data"]["targets_pinned"]
            .is_null()
    );
    assert_eq!(pin_events(&fs::read(root.join(JOURNAL_PATH))?)?.len(), 1);

    let branch = git_command(BUILT_EXECUTABLE)
        .args(["branch", "replacement-trunk"])
        .current_dir(root)
        .output()?;
    require_readable(&branch, "create replacement trunk")?;
    assert!(branch.status.success());
    let config_path = root.join(".claude/config/berth.toml");
    let config = fs::read_to_string(&config_path)?;
    let edited = config.replacen("trunk = \"main\"", "trunk = \"replacement-trunk\"", 1);
    assert_ne!(edited, config, "fixture has a main trunk setting");
    fs::write(config_path, edited)?;
    let third = run_reader(Path::new(BUILT_EXECUTABLE), root, &["init", "--json"])?;
    require_readable(&third, "init after trunk edit")?;
    assert!(third.status.success());
    assert_eq!(pin_events(&fs::read(root.join(JOURNAL_PATH))?)?, pins);
    Ok(())
}

fn reader_executable() -> TestResult<PathBuf> {
    let executable = std::env::var_os(EXECUTABLE_ENVIRONMENT)
        .map_or_else(|| PathBuf::from(BUILT_EXECUTABLE), PathBuf::from);
    // Resolve relative overrides before a child changes into the fixture repository.
    Ok(fs::canonicalize(executable)?)
}

fn fixture_repository() -> TestResult<TempDir> {
    let repository = tempfile::tempdir()?;
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reader_compat");
    let output = git_command(BUILT_EXECUTABLE)
        .args(["-c", "core.hooksPath=/dev/null", "clone", "--quiet"])
        .arg(fixture.join("repository.bundle"))
        .arg(repository.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()?;
    require_readable(&output, "restore fixture repository")?;
    fs::create_dir(repository.path().join(".git/cargo-berth"))?;
    for (source, destination) in [
        ("journal.ndjson", JOURNAL_PATH),
        ("repo-instance-id", ".git/cargo-berth/repo-instance-id"),
        ("worktree-id", ".git/cargo-berth-worktree-id"),
        ("run-id", ".git/cargo-berth-run-id"),
    ] {
        fs::copy(fixture.join(source), repository.path().join(destination))?;
    }
    let records = JOURNAL
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        records.len(),
        2,
        "the fixture freezes a claim and its first merge observation"
    );
    assert_eq!(records[1]["op"], "merge_extent_observed");
    assert_eq!(records[1]["extent"]["status"], "protected");
    let reservation_id = records[0]["reservation_id"]
        .as_str()
        .ok_or("fixture claim needs a reservation ID")?;
    fs::write(
        repository.path().join("claimed.txt"),
        "uncommitted fixture work\n",
    )?;
    // Renew with the built reader so stale-reservation notices do not change with the date.
    // The selected reader still has to decode every original record on its first call.
    let renewed = run_reader(
        Path::new(BUILT_EXECUTABLE),
        repository.path(),
        &["renew", reservation_id, "--json"],
    )?;
    require_readable(&renewed, "renew fixture activity")?;
    assert!(fs::read(repository.path().join(JOURNAL_PATH))?.starts_with(JOURNAL.as_bytes()));
    Ok(repository)
}

fn run_reader(executable: &Path, repository: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(Command::new(executable)
        .args(arguments)
        .current_dir(repository)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .env_remove("CARGO_BERTH_BYPASS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()?)
}

fn assert_command(arguments: &[&str], expected: &str) -> TestResult {
    let repository = fixture_repository()?;
    let executable = reader_executable()?;
    let output = run_reader(&executable, repository.path(), arguments)?;
    let context = format!("{} {}", executable.display(), arguments.join(" "));
    require_readable(&output, &context)?;
    let response: Value = serde_json::from_slice(&output.stdout)?;
    let mut observed =
        serde_json::json!({"status": response["status"], "payload": response["payload"]});
    let mut expected: Value = serde_json::from_str(expected)?;
    normalize(&mut observed, repository.path())?;
    normalize(&mut expected, repository.path())?;
    assert_eq!(observed, expected, "{context}");
    Ok(())
}

fn assert_hook_response(
    output: &Output,
    event: HookResponseEvent,
    repository: &Path,
    expected: &str,
) -> TestResult {
    require_readable(output, "reader hook response")?;
    // Keep the existing hook protocol checks, including each event's continuation rule.
    let mut feedback = hook_feedback(output, event, "reader compatibility")?;
    let mut context = Value::String(feedback.additional_context);
    normalize(&mut context, repository)?;
    let Value::String(additional_context) = context else {
        return Err("hook context must remain text".into());
    };
    feedback.additional_context = additional_context;
    let mut expected: Value = serde_json::from_str(expected)?;
    normalize(&mut expected, repository)?;
    assert_eq!(
        feedback,
        expected_hook_feedback(&expected, "reader compatibility fixture")?
    );
    let mut response: Value = serde_json::from_slice(&output.stdout)?;
    normalize(&mut response, repository)?;
    assert_eq!(response, expected, "the complete hook response must match");
    Ok(())
}

/// Hooks exit zero on ledger failures, so inspect both output streams before exit status.
fn require_readable(output: &Output, context: &str) -> TestResult {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for diagnostic in [
        "ledger_unreadable",
        "journal record",
        "unknown variant",
        "could not read the reservation ledger",
        "could not establish edit safety",
        "ledger-lock deadline",
    ] {
        if stdout.contains(diagnostic) || stderr.contains(diagnostic) {
            return Err(std::io::Error::other(format!(
                "{context}: ledger error: stdout={stdout} stderr={stderr}"
            ))
            .into());
        }
    }
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(std::io::Error::other(format!(
            "{context}: status={} stdout={stdout} stderr={stderr}",
            output.status
        ))
        .into());
    }
    Ok(())
}

/// Restate only run-dependent paths, append sizes, and renewed activity timestamps.
///
/// The reader reports canonical worktree roots, while a temporary directory can be
/// reached through a symlink or a `..` component — `$TMPDIR` under macOS `/var` is
/// the common one. Resolve the fixture root once, so the replacement matches the
/// path the reader actually prints rather than leaving the resolved prefix behind.
fn normalize(value: &mut Value, repository: &Path) -> TestResult {
    let repository = fs::canonicalize(repository)?;
    normalize_under_canonical_root(value, &repository)
}

fn normalize_under_canonical_root(value: &mut Value, repository: &Path) -> TestResult {
    match value {
        Value::Object(fields) => {
            for (name, value) in fields {
                match name.as_str() {
                    "journal_byte_offset" => *value = Value::from("{JOURNAL_BYTE_OFFSET}"),
                    "last_activity_at" => *value = Value::from("{LAST_ACTIVITY_AT}"),
                    _ => normalize_under_canonical_root(value, repository)?,
                }
            }
        },
        Value::Array(values) => {
            for value in values {
                normalize_under_canonical_root(value, repository)?;
            }
        },
        Value::String(text) => {
            if let Some(report) = text.strip_prefix(BOARD_REPORT_PREFIX) {
                let mut report: Value = serde_json::from_str(report)?;
                normalize_under_canonical_root(&mut report, repository)?;
                *text = format!(
                    "{BOARD_REPORT_PREFIX}{}",
                    serde_json::to_string_pretty(&report)?
                );
            } else {
                *text = text.replace(
                    repository.to_str().ok_or("fixture path must be UTF-8")?,
                    "{WORKTREE_ROOT}",
                );
            }
        },
        Value::Null | Value::Bool(_) | Value::Number(_) => {},
    }
    Ok(())
}
