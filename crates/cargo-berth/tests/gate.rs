#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end tests for installation, enforcement, release valves, and gate cost.

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tempfile::tempdir;

const BYPASS_ENVIRONMENT: &str = "CARGO_BERTH_BYPASS";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GIT_BINARY: &str = "git";
const HOOK_PATH: &str = ".git/hooks/reference-transaction";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const PENDING_BYPASS_PREFIX: &str = "cargo-berth-pending-bypass-";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    case "$2" in
        cat-file) printf '%s %s\n' "$2" "$3" >> "$CARGO_BERTH_TEST_GIT_TRACE" ;;
        rev-list) printf '%s\n' "$2" >> "$CARGO_BERTH_TEST_GIT_TRACE" ;;
    esac
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;

#[test]
fn init_manages_the_common_hook_without_overwriting_an_unmanaged_owner() {
    let repository = scratch_repository();

    let initialized = run_berth(repository.path(), &["init", "--json"]);
    assert!(initialized.status.success());
    let payload = json_output(&initialized);
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["name"],
        "reference-transaction"
    );
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["status"],
        "active"
    );
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["installation"],
        "installed"
    );
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read(&hook_path).expect("managed hook should read");
    assert!(installed.starts_with(b"#!/bin/sh\n"));
    assert!(
        installed
            .windows("__reference-transaction".len())
            .any(|window| window == b"__reference-transaction")
    );
    assert!(!String::from_utf8_lossy(&installed).contains("cargo berth drift"));
    assert_ne!(
        fs::metadata(&hook_path)
            .expect("managed hook metadata should read")
            .permissions()
            .mode()
            & 0o111,
        0
    );

    assert!(run_berth(repository.path(), &["init"]).status.success());
    assert_eq!(
        fs::read(&hook_path).expect("idempotent hook should read"),
        installed
    );
    let unmanaged = b"\xff#!/bin/sh\nexit 0\n";
    fs::write(&hook_path, unmanaged).expect("unmanaged hook should write");

    let preserved = run_berth(repository.path(), &["init"]);
    assert!(preserved.status.success());
    let diagnostic = String::from_utf8_lossy(&preserved.stdout);
    assert!(diagnostic.contains("reference-transaction"));
    assert!(diagnostic.contains("protection for that hook is not active"));
    assert!(diagnostic.contains("wrapper or move it aside"));
    assert_eq!(
        fs::read(&hook_path).expect("unmanaged hook should remain readable"),
        unmanaged
    );
    let preserved_json = run_berth(repository.path(), &["init", "--json"]);
    let payload = json_output(&preserved_json);
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["status"],
        "inactive"
    );
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["reason"]["kind"],
        "preserved_unmanaged"
    );

    let help = run_berth(repository.path(), &["--help"]);
    assert!(help.status.success());
    assert!(!String::from_utf8_lossy(&help.stdout).contains("__reference-transaction"));
}

#[test]
fn init_reports_hook_installation_failure_without_claiming_the_ledger_is_unreadable() {
    let repository = scratch_repository();
    let occupied_hooks_path = repository.path().join("occupied-hooks-path");
    fs::write(&occupied_hooks_path, "not a directory\n").expect("occupied hooks path should write");
    git(
        repository.path(),
        &[
            "config",
            "core.hooksPath",
            occupied_hooks_path
                .to_str()
                .expect("hooks path should be UTF-8"),
        ],
    );

    let initialized = run_berth(repository.path(), &["init", "--json"]);
    assert!(initialized.status.success());
    let payload = json_output(&initialized);
    assert_eq!(payload["status"], "initialized");
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["status"],
        "inactive"
    );
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["reason"]["kind"],
        "installation_failed"
    );
    assert!(
        payload["payload"]["data"]["hooks"][0]["activation"]["reason"]["diagnostic"]
            .as_str()
            .is_some_and(|diagnostic| diagnostic.contains("managed hook installation failed"))
    );
}

#[test]
fn init_installs_into_the_effective_core_hooks_path() {
    let repository = scratch_repository();
    git(
        repository.path(),
        &["config", "core.hooksPath", "custom-hooks"],
    );

    assert!(run_berth(repository.path(), &["init"]).status.success());
    let configured_hook = repository.path().join("custom-hooks/reference-transaction");
    assert!(configured_hook.is_file());
    assert!(!repository.path().join(HOOK_PATH).is_file());
    assert!(
        fs::read(configured_hook)
            .expect("configured hook should read")
            .windows("__reference-transaction".len())
            .any(|window| window == b"__reference-transaction")
    );
}

#[test]
fn managed_hook_fails_open_and_marks_a_bypass_for_an_unavailable_binary() {
    let repository = initialized_repository();
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read_to_string(&hook_path).expect("managed hook should read");
    let executable = shell_single_quoted(Path::new(env!("CARGO_BIN_EXE_cargo-berth")));
    let unavailable = shell_single_quoted(Path::new("/missing/cargo-berth"));
    let broken = installed.replace(&executable, &unavailable);
    assert_ne!(broken, installed);
    fs::write(&hook_path, broken).expect("broken hook fixture should write");
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base} {base} refs/heads/main\n");

    let permitted = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Unset);
    assert!(permitted.status.success());
    let diagnostic = String::from_utf8_lossy(&permitted.stderr);
    assert!(diagnostic.contains("executable is unavailable"));
    assert!(diagnostic.contains("permitting this ref transaction"));
    assert!(diagnostic.contains("Rerun cargo berth init"));

    let bypassed = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Set);
    assert!(bypassed.status.success());
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("executable is unavailable"));
    assert!(diagnostic.contains("Rerun cargo berth init"));
    assert_eq!(pending_bypass_count(repository.path()), 1);
}

#[test]
fn managed_hook_journals_an_environment_bypass_when_the_journal_is_writable() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base} {base} refs/heads/main\n");

    let bypassed = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Set);
    assert!(
        bypassed.status.success(),
        "bypass should succeed: {}",
        String::from_utf8_lossy(&bypassed.stderr)
    );
    let bypass_record = journal_text(repository.path())
        .lines()
        .map(|record| {
            serde_json::from_str::<serde_json::Value>(record)
                .expect("journal record should deserialize")
        })
        .find(|record| record["op"] == "bypass")
        .expect("environment bypass should be journalled");
    assert_eq!(bypass_record["action"], "integration");
    assert_eq!(bypass_record["cause"]["kind"], "environment_override");
    assert_eq!(pending_bypass_count(repository.path()), 0);
}

#[test]
fn a_bypass_the_binary_cannot_record_still_leaves_a_marker() {
    let repository = initialized_repository();
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read_to_string(&hook_path).expect("managed hook should read");
    let rejecting = installed.replace(
        "__reference-transaction \"$@\"",
        "__reference-transaction --not-a-flag \"$@\"",
    );
    assert_ne!(rejecting, installed);
    fs::write(&hook_path, rejecting).expect("rejecting hook fixture should write");
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base} {base} refs/heads/main\n");

    let bypassed = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Set);
    assert!(
        bypassed.status.success(),
        "bypass must never fail a ref transaction: {}",
        String::from_utf8_lossy(&bypassed.stderr)
    );
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("could not record this bypass"));
    assert!(diagnostic.contains("CARGO_BERTH_BYPASS=1"));
    assert_eq!(pending_bypass_count(repository.path()), 1);
    assert!(
        !journal_text(repository.path())
            .lines()
            .filter_map(|record| serde_json::from_str::<serde_json::Value>(record).ok())
            .any(|record| record["op"] == "bypass"),
        "a bypass the binary never recorded must not appear in the journal"
    );
}

#[test]
fn an_unrecorded_binary_bypass_warns_without_blocking_the_ref_update() {
    let non_repository = tempdir().expect("non-repository directory should exist");

    let bypassed = run_berth_with_environment(
        non_repository.path(),
        &["__reference-transaction", "prepared", "refs/heads/main"],
        BYPASS_ENVIRONMENT,
        "1",
    );

    assert!(bypassed.status.success());
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("took the CARGO_BERTH_BYPASS=1 override"));
    assert!(diagnostic.contains("neither the journal nor a pending marker"));
    assert!(diagnostic.contains("ref transaction remains permitted"));
    assert!(diagnostic.contains("rerun cargo berth init"));
}

#[test]
fn a_bypass_without_an_invocation_directory_warns_without_blocking_the_ref_update() {
    let directory = tempdir().expect("temporary parent should exist");
    let removed_directory = directory.path().join("removed");
    fs::create_dir(&removed_directory).expect("removable directory should exist");
    let executable = Path::new(env!("CARGO_BIN_EXE_cargo-berth"));
    let command = format!(
        "cd {} && rmdir {} && exec {} __reference-transaction prepared refs/heads/main",
        shell_single_quoted(&removed_directory),
        shell_single_quoted(&removed_directory),
        shell_single_quoted(executable),
    );

    let bypassed = Command::new("sh")
        .args(["-c", &command])
        .env(BYPASS_ENVIRONMENT, "1")
        .output()
        .expect("private gate should run from a removed directory");

    assert!(bypassed.status.success());
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("took the CARGO_BERTH_BYPASS=1 override"));
    assert!(diagnostic.contains("could not resolve its invocation directory"));
    assert!(diagnostic.contains("No audit fact was retained"));
    assert!(diagnostic.contains("ref transaction remains permitted"));
    assert!(diagnostic.contains("rerun cargo berth init"));
}

#[test]
fn non_trunk_updates_do_not_read_an_absent_configuration() {
    let repository = initialized_repository();
    let configuration_path = repository.path().join(CONFIGURATION_PATH);
    let saved_configuration = repository.path().join("berth.toml.saved");
    fs::rename(&configuration_path, &saved_configuration).expect("configuration should move aside");

    let checkout = git_output(repository.path(), &["checkout", "-b", "x"]);
    assert!(
        checkout.status.success(),
        "branch creation failed: {}",
        String::from_utf8_lossy(&checkout.stderr)
    );
    fs::write(repository.path().join("branch.txt"), "branch\n")
        .expect("branch source should write");
    git(repository.path(), &["add", "branch.txt"]);
    let commit = git_output(
        repository.path(),
        &["commit", "--quiet", "-m", "branch work"],
    );
    assert!(
        commit.status.success(),
        "branch commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );

    let branch_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let main = git_stdout(repository.path(), &["rev-parse", "main"]);
    let possible_trunk = propose_trunk(repository.path(), &main, &branch_head);
    assert!(possible_trunk.status.success());
    let warning = String::from_utf8_lossy(&possible_trunk.stderr);
    assert!(warning.contains("could not read its configuration"));
    assert!(warning.contains("Restore the configuration"));
    assert!(!warning.contains("cargo berth drift"));
    assert!(warning.contains("CARGO_BERTH_BYPASS=1"));
}

#[test]
fn ungoverned_ref_lines_are_typed_ignored_entries_and_parse_denials_name_bypass() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    fs::write(repository.path().join(JOURNAL_PATH), b"{}\n").expect("corrupt journal should write");

    let ignored = run_private_hook(
        repository.path(),
        "prepared",
        &format!("invalid invalid worktrees/foo/HEAD\n{base} {base} refs/heads/topic\n"),
    );
    assert!(ignored.status.success());

    let malformed = run_private_hook(repository.path(), "prepared", "one-field\n");
    assert_eq!(malformed.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&malformed.stderr).contains("CARGO_BERTH_BYPASS=1"));
}

#[test]
fn observe_enforce_and_one_use_force_apply_to_both_deferred_endpoints() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let holder_root = add_worktree(repository.path(), worktrees.path(), "holder");
    let requester_root = add_worktree(repository.path(), worktrees.path(), "requester");
    let holder = claim(
        &holder_root,
        "tree:src",
        FIRST_RUN,
        "docs/holder-plan.md",
        "holder-phase",
    );
    let holder_id = reservation_id(&holder);
    let requester = defer_claim(
        &requester_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "docs/requester-plan.md",
        "phase-8",
        &holder_id,
    );
    let requester_id = reservation_id(&requester);
    let holder_head = commit_work(
        &holder_root,
        "src/holder.rs",
        "pub fn holder_work() {}\n",
        "holder work",
    );
    let holder_observed = propose_trunk(repository.path(), &base, &holder_head);
    assert!(holder_observed.status.success());
    let holder_observation = String::from_utf8_lossy(&holder_observed.stderr);
    assert!(holder_observation.contains("Observe-only"));
    assert!(holder_observation.contains(&requester_id));
    restore_trunk(repository.path(), &holder_head, &base);

    let requester_head = commit_work(
        &requester_root,
        "src/lib.rs",
        "pub fn requester_work() {}\n",
        "requester work",
    );

    let observed = propose_trunk(repository.path(), &base, &requester_head);
    assert!(observed.status.success());
    assert!(String::from_utf8_lossy(&observed.stderr).contains("Observe-only"));
    restore_trunk(repository.path(), &requester_head, &base);
    set_gate_mode(repository.path(), "enforce");

    let blocked = propose_trunk(repository.path(), &base, &requester_head);
    let denial = String::from_utf8_lossy(&blocked.stderr);
    assert!(!blocked.status.success());
    assert_denial_context(&denial, &requester_id, &holder_id);
    assert_eq!(git_stdout(repository.path(), &["rev-parse", "main"]), base);

    let forced = run_berth(
        &requester_root,
        &[
            "integrate",
            &requester_id,
            "--force",
            "--why",
            "accept the reviewed ordering exception",
            "--json",
        ],
    );
    assert!(
        forced.status.success(),
        "forced integration failed: {}",
        String::from_utf8_lossy(&forced.stdout)
    );
    assert_eq!(
        git_stdout(repository.path(), &["rev-parse", "main"]),
        requester_head
    );
    assert_forced_permit_consumed(repository.path());

    restore_trunk(repository.path(), &requester_head, &base);
    let reused = propose_trunk(repository.path(), &base, &requester_head);
    assert!(
        !reused.status.success(),
        "a consumed permit must not replay"
    );
}

#[test]
fn an_observed_violation_with_closed_stderr_still_permits_the_ref_update() {
    let repository = initialized_repository();
    let deferred_pair = deferred_pair(repository.path());
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let blocked_head = commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn blocked_work() {}\n",
        "blocked work",
    );
    let executable = Path::new(env!("CARGO_BIN_EXE_cargo-berth"));
    let command = format!(
        "exec {} __reference-transaction prepared refs/heads/main 2>&-",
        shell_single_quoted(executable),
    );
    let mut child = Command::new("sh")
        .args(["-c", &command])
        .current_dir(repository.path())
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove("CARGO_BERTH_RUN")
        .stdin(Stdio::piped())
        .spawn()
        .expect("private gate should start with closed stderr");
    child
        .stdin
        .take()
        .expect("private gate stdin should be piped")
        .write_all(format!("{base} {blocked_head} refs/heads/main\n").as_bytes())
        .expect("reference transaction should write");

    let status = child.wait().expect("private gate should exit");

    assert!(status.success());
    assert_eq!(status.code(), Some(0), "private gate must not abort");
}

#[test]
fn integrate_rejects_a_stale_worktree_non_fast_forward() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let stale_root = add_worktree(repository.path(), worktrees.path(), "stale");
    let reservation = claim(&stale_root, "tree:src", FIRST_RUN, "docs/stale.md", "stale");
    let reservation_id = reservation_id(&reservation);
    let stale_head = commit_work(
        &stale_root,
        "src/stale.rs",
        "pub fn stale() {}\n",
        "stale work",
    );
    let newer_main = commit_work(
        repository.path(),
        "newer-main.txt",
        "newer main\n",
        "newer main",
    );

    let integration = run_berth(&stale_root, &["integrate", &reservation_id, "--json"]);
    assert_eq!(integration.status.code(), Some(4));
    let output = String::from_utf8_lossy(&integration.stdout);
    assert!(output.contains("non-fast-forward"));
    assert!(output.contains(&newer_main));
    assert!(output.contains(&stale_head));
    assert_eq!(
        git_stdout(repository.path(), &["rev-parse", "main"]),
        newer_main
    );
}

#[test]
fn integrate_enforces_holds_for_every_entering_reservation() {
    let repository = initialized_repository();
    let deferred_pair = deferred_pair(repository.path());
    let blocked_head = commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn blocked_work() {}\n",
        "blocked work",
    );
    let clear_root = add_worktree(
        repository.path(),
        deferred_pair.worktrees.path(),
        "clear-request",
    );
    let clear = claim(
        &clear_root,
        "tree:tests",
        THIRD_RUN,
        "docs/clear.md",
        "clear",
    );
    let clear_id = reservation_id(&clear);
    commit_work(
        &clear_root,
        "tests/clear.rs",
        "// clear reservation\n",
        "clear work",
    );
    git(
        &clear_root,
        &["merge", "--no-ff", "--no-edit", &blocked_head],
    );
    let previous = git_stdout(repository.path(), &["rev-parse", "main"]);
    set_gate_mode(repository.path(), "enforce");
    set_gate_mode(&clear_root, "enforce");

    let integration = run_berth(&clear_root, &["integrate", &clear_id, "--json"]);
    assert_eq!(integration.status.code(), Some(2));
    let denial = String::from_utf8_lossy(&integration.stdout);
    assert!(denial.contains(&deferred_pair.blocked_id));
    assert!(denial.contains(&deferred_pair.holder_id));
    assert_eq!(
        git_stdout(repository.path(), &["rev-parse", "main"]),
        previous
    );
}

#[test]
fn unavailable_worktree_heads_surface_as_enforced_violations() {
    let repository = initialized_repository();
    let deferred_pair = deferred_pair(repository.path());
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let blocked_head = commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn unavailable_work() {}\n",
        "unavailable work",
    );
    set_gate_mode(repository.path(), "enforce");
    git(
        repository.path(),
        &[
            "worktree",
            "remove",
            "--force",
            deferred_pair
                .blocked_root
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );

    let blocked = propose_trunk(repository.path(), &base, &blocked_head);
    assert!(!blocked.status.success());
    let denial = String::from_utf8_lossy(&blocked.stderr);
    assert!(denial.contains(&deferred_pair.blocked_id));
    assert!(denial.contains(&deferred_pair.holder_id));
    assert_eq!(git_stdout(repository.path(), &["rev-parse", "main"]), base);
}

#[test]
fn permit_consumption_waits_for_committed_and_aborted_does_not_spend_it() {
    const ABORT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_ABORT_REFERENCE_TRANSACTION";

    let repository = initialized_repository();
    let deferred_pair = deferred_pair(repository.path());
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let blocked_head = commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn phase_work() {}\n",
        "phase work",
    );
    set_gate_mode(repository.path(), "enforce");
    let hook_path = repository.path().join(HOOK_PATH);
    let original_hook_path = repository
        .path()
        .join(".git/hooks/reference-transaction.original");
    fs::copy(&hook_path, &original_hook_path).expect("original hook should copy");
    let phase_log = repository.path().join("reference-transaction-phases.log");
    let wrapper = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\n{} \"$@\"\nstatus=$?\nif [ \"$1\" = \"prepared\" ] && [ \"${{{ABORT_ENVIRONMENT}:-}}\" = \"1\" ] && [ \"$status\" -eq 0 ]; then\n    exit 1\nfi\nexit \"$status\"\n",
        shell_single_quoted(&phase_log),
        shell_single_quoted(&original_hook_path),
    );
    fs::write(&hook_path, wrapper).expect("phase wrapper should write");
    let mut permissions = fs::metadata(&hook_path)
        .expect("phase wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook_path, permissions).expect("phase wrapper should be executable");

    let aborted = run_berth_with_environment(
        &deferred_pair.blocked_root,
        &[
            "integrate",
            &deferred_pair.blocked_id,
            "--force",
            "--why",
            "exercise aborted phase",
            "--json",
        ],
        ABORT_ENVIRONMENT,
        "1",
    );
    assert_eq!(aborted.status.code(), Some(4));
    assert_eq!(git_stdout(repository.path(), &["rev-parse", "main"]), base);
    let journal_after_abort = journal_text(repository.path());
    assert_eq!(
        journal_after_abort
            .matches("\"op\":\"forced_integration_permit\"")
            .count(),
        1
    );
    assert!(!journal_after_abort.contains("\"op\":\"consume_forced_integration_permit\""));
    assert!(!journal_after_abort.contains("\"kind\":\"forced_integration\""));

    let abort_phase = run_hook_at_path(
        &hook_path,
        repository.path(),
        "aborted",
        &format!("{base} {blocked_head} refs/heads/main\n"),
        ReleaseValve::Unset,
    );
    assert!(abort_phase.status.success());
    assert!(
        !journal_text(repository.path()).contains("\"op\":\"consume_forced_integration_permit\"")
    );

    let committed = propose_trunk(repository.path(), &base, &blocked_head);
    assert!(
        committed.status.success(),
        "committed update failed: {}",
        String::from_utf8_lossy(&committed.stderr)
    );
    assert_forced_permit_consumed(repository.path());
    let phases = fs::read_to_string(phase_log).expect("phase log should read");
    assert!(phases.lines().any(|phase| phase == "prepared"));
    assert!(phases.lines().any(|phase| phase == "committed"));
    assert!(phases.lines().any(|phase| phase == "aborted"));
}

#[test]
fn environment_bypass_precedes_corruption_and_confirmed_reinit_recovers() {
    let repository = initialized_repository();
    let configuration_before =
        fs::read(repository.path().join(CONFIGURATION_PATH)).expect("configuration should read");
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let feature_root = add_worktree(repository.path(), worktrees.path(), "feature");
    fs::write(feature_root.join("feature.txt"), "feature\n").expect("feature source should write");
    git(&feature_root, &["add", "feature.txt"]);
    git(&feature_root, &["commit", "--quiet", "-m", "feature"]);
    let feature_head = git_stdout(&feature_root, &["rev-parse", "HEAD"]);
    fs::write(repository.path().join(JOURNAL_PATH), b"{}\n").expect("corrupt journal should write");

    let closed = propose_trunk(repository.path(), &base, &feature_head);
    assert!(!closed.status.success());
    let closed_error = String::from_utf8_lossy(&closed.stderr);
    assert!(closed_error.contains("could not prove this integration safe"));
    assert!(closed_error.contains("CARGO_BERTH_BYPASS=1"));

    let bypassed = update_main(repository.path(), &base, &feature_head, ReleaseValve::Set);
    assert!(
        bypassed.status.success(),
        "bypass should permit corrupt-ledger update: {}",
        String::from_utf8_lossy(&bypassed.stderr)
    );
    assert_eq!(pending_bypass_count(repository.path()), 1);

    let contradictory = run_berth(
        repository.path(),
        &["init", "--repair-projection", "--reinitialize-after-review"],
    );
    assert_eq!(contradictory.status.code(), Some(5));

    let reinitialized = run_berth(
        repository.path(),
        &["init", "--reinitialize-after-review", "--json"],
    );
    let payload = json_output(&reinitialized);
    assert!(reinitialized.status.success());
    assert_eq!(payload["status"], "reinitialized");
    assert_eq!(
        payload["payload"]["data"]["pending_environment_bypasses"],
        1
    );
    assert_eq!(journal_text(repository.path()), "");
    assert_eq!(
        fs::read(repository.path().join(CONFIGURATION_PATH))
            .expect("configuration should remain readable"),
        configuration_before
    );

    fs::remove_file(repository.path().join(JOURNAL_PATH))
        .expect("journal should be removable for absent-ledger recovery");
    assert!(
        run_berth(repository.path(), &["init", "--reinitialize-after-review"])
            .status
            .success()
    );
    assert_eq!(journal_text(repository.path()), "");
}

#[test]
fn multi_ref_transactions_evaluate_only_the_trunk_update() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let feature_root = add_worktree(repository.path(), worktrees.path(), "multi-ref");
    let feature_head = commit_work(
        &feature_root,
        "multi-ref.txt",
        "multi ref\n",
        "multi ref work",
    );
    let input = format!(
        "invalid invalid refs/tags/release\n{base} {feature_head} refs/heads/multi-ref\n{base} {feature_head} refs/heads/main\n"
    );

    let transaction = run_private_hook(repository.path(), "prepared", &input);
    assert!(
        transaction.status.success(),
        "multi-ref transaction failed: {}",
        String::from_utf8_lossy(&transaction.stderr)
    );
}

#[test]
fn hook_boundary_reports_missing_and_corrupt_ledgers_with_exit_four() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let feature_root = add_worktree(repository.path(), worktrees.path(), "ledger-boundary");
    let feature_head = commit_work(
        &feature_root,
        "ledger-boundary.txt",
        "ledger boundary\n",
        "ledger boundary work",
    );
    let input = format!("{base} {feature_head} refs/heads/main\n");
    fs::remove_file(repository.path().join(JOURNAL_PATH)).expect("journal should remove");

    let missing = run_private_hook(repository.path(), "prepared", &input);
    assert_eq!(missing.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("CARGO_BERTH_BYPASS=1"));

    fs::write(repository.path().join(JOURNAL_PATH), b"{}\n").expect("corrupt journal should write");
    let corrupt = run_private_hook(repository.path(), "prepared", &input);
    assert_eq!(corrupt.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&corrupt.stderr).contains("CARGO_BERTH_BYPASS=1"));
}

#[test]
fn hook_lock_contention_uses_one_ten_second_deadline() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let feature_root = add_worktree(repository.path(), worktrees.path(), "lock-deadline");
    let feature_head = commit_work(
        &feature_root,
        "lock-deadline.txt",
        "lock deadline\n",
        "lock deadline work",
    );
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(repository.path().join(LOCK_PATH))
        .expect("mutation lock should open");
    lock_file
        .try_lock()
        .expect("test should hold mutation lock");
    let started_at = Instant::now();

    let blocked = run_private_hook(
        repository.path(),
        "prepared",
        &format!("{base} {feature_head} refs/heads/main\n"),
    );
    let elapsed = started_at.elapsed();
    assert_eq!(blocked.status.code(), Some(6));
    assert!(elapsed >= Duration::from_secs(9));
    assert!(elapsed < Duration::from_secs(15));
    let diagnostic = String::from_utf8_lossy(&blocked.stderr);
    assert!(diagnostic.contains("10-second"));
    assert!(diagnostic.contains("CARGO_BERTH_BYPASS=1"));
    std::mem::drop(lock_file);
}

#[test]
fn hook_git_cost_scales_with_protected_graph_predecessors() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let unrelated_root = add_worktree(repository.path(), worktrees.path(), "unrelated");

    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn predecessor_work() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "src/lib.rs"]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim(
        &predecessor_root,
        "tree:src",
        FIRST_RUN,
        "docs/predecessor.md",
        "predecessor",
    );
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "docs/successor.md",
        "successor",
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    assert!(
        run_berth(
            repository.path(),
            &[
                "sequence",
                &predecessor_id,
                &successor_id,
                "--why",
                "predecessor first",
                "--json",
            ],
        )
        .status
        .success()
    );
    assert!(
        claim(
            &unrelated_root,
            "tree:tests",
            THIRD_RUN,
            "docs/unrelated.md",
            "unrelated",
        )
        .status
        .success()
    );

    fs::write(
        successor_root.join("src/lib.rs"),
        "pub fn successor_work() {}\n",
    )
    .expect("successor source should write");
    git(&successor_root, &["add", "src/lib.rs"]);
    git(
        &successor_root,
        &["commit", "--quiet", "-m", "successor work"],
    );
    let successor_head = git_stdout(&successor_root, &["rev-parse", "HEAD"]);

    let traced = run_private_hook_with_git_trace(
        repository.path(),
        &format!("{base} {successor_head} refs/heads/main\n"),
    );
    assert!(
        traced.output.status.success(),
        "observe-only hook failed: {}",
        String::from_utf8_lossy(&traced.output.stderr)
    );
    let trace = fs::read_to_string(&traced.trace_path).expect("git trace should read");
    assert_eq!(
        trace
            .lines()
            .filter(|command| *command == "cat-file --batch-check")
            .count(),
        1
    );
    assert_eq!(
        trace
            .lines()
            .filter(|command| *command == "rev-list")
            .count(),
        2
    );
}

#[test]
fn deleting_the_retention_ref_leaves_no_hook_owned_reference() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "prunable");
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn prunable() {}\n",
    )
    .expect("prunable source should write");
    git(&predecessor_root, &["add", "src/lib.rs"]);
    git(&predecessor_root, &["commit", "--quiet", "-m", "prunable"]);
    let protected_tip = git_stdout(&predecessor_root, &["rev-parse", "HEAD"]);
    let predecessor = claim(
        &predecessor_root,
        "tree:src",
        FIRST_RUN,
        "docs/prunable.md",
        "prunable",
    );
    let predecessor_id = reservation_id(&predecessor);
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    git(
        repository.path(),
        &[
            "worktree",
            "remove",
            "--force",
            predecessor_root
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );
    git(repository.path(), &["branch", "-D", "prunable"]);
    let retention_ref = format!("refs/cargo-berth/reservations/{predecessor_id}");
    git(repository.path(), &["update-ref", "-d", &retention_ref]);
    assert!(!reference_exists(repository.path(), &retention_ref));
    git(
        repository.path(),
        &["reflog", "expire", "--expire=now", "--all"],
    );
    assert!(!reference_exists(repository.path(), &retention_ref));
    git(repository.path(), &["gc", "--prune=now"]);
    assert!(!reference_exists(repository.path(), &retention_ref));

    let object_status = Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(["cat-file", "-e", &format!("{protected_tip}^{{commit}}")])
        .current_dir(repository.path())
        .status()
        .expect("git cat-file should run");
    assert!(!object_status.success());
}

struct TracedHook {
    output:     Output,
    trace_path: PathBuf,
    _directory: TempDir,
}

struct DeferredPair {
    worktrees:    TempDir,
    blocked_root: PathBuf,
    blocked_id:   String,
    holder_id:    String,
}

fn deferred_pair(repository_root: &Path) -> DeferredPair {
    let worktrees = tempdir().expect("worktree parent should exist");
    let holder_root = add_worktree(repository_root, worktrees.path(), "pair-holder");
    let blocked_root = add_worktree(repository_root, worktrees.path(), "pair-blocked");
    let holder = claim(
        &holder_root,
        "tree:src",
        FIRST_RUN,
        "docs/pair-holder.md",
        "pair-holder",
    );
    let holder_id = reservation_id(&holder);
    let blocked = defer_claim(
        &blocked_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "docs/pair-blocked.md",
        "pair-blocked",
        &holder_id,
    );
    DeferredPair {
        worktrees,
        blocked_root,
        blocked_id: reservation_id(&blocked),
        holder_id,
    }
}

fn initialized_repository() -> TempDir {
    let repository = scratch_repository();
    assert!(
        run_berth(repository.path(), &["init", "--json"])
            .status
            .success()
    );
    git(repository.path(), &["add", CONFIGURATION_PATH]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    repository
}

fn scratch_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(repository.path(), &["config", "user.name", "Test User"]);
    fs::create_dir_all(repository.path().join("src")).expect("source directory should exist");
    fs::create_dir_all(repository.path().join("tests")).expect("test directory should exist");
    fs::write(repository.path().join("src/lib.rs"), "pub fn base() {}\n")
        .expect("base source should write");
    fs::write(repository.path().join("tests/base.rs"), "// base\n")
        .expect("base test should write");
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    repository
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let worktree_root = parent.join(branch);
    git(
        repository_root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            worktree_root
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );
    worktree_root
}

fn commit_work(repository_root: &Path, path: &str, contents: &str, message: &str) -> String {
    fs::write(repository_root.join(path), contents).expect("work source should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn assert_denial_context(denial: &str, requester_id: &str, holder_id: &str) {
    for required in [
        requester_id,
        holder_id,
        "docs/requester-plan.md",
        "phase-8",
        "file:src/lib.rs",
        "cargo-berth sequence",
        "cargo-berth integrate",
        "CARGO_BERTH_BYPASS=1",
    ] {
        assert!(
            denial.contains(required),
            "denial omitted {required}: {denial}"
        );
    }
}

fn assert_forced_permit_consumed(repository_root: &Path) {
    let journal = journal_text(repository_root);
    for operation in [
        "\"op\":\"forced_integration_permit\"",
        "\"op\":\"consume_forced_integration_permit\"",
    ] {
        assert_eq!(journal.matches(operation).count(), 1);
    }
    assert!(journal.contains("\"kind\":\"forced_integration\""));
}

fn claim(repository_root: &Path, scope: &str, run: &str, plan: &str, phase: &str) -> Output {
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            plan,
            "--phase",
            phase,
            "--why",
            "protect test work",
            "--json",
        ],
    )
}

fn defer_claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    plan: &str,
    phase: &str,
    blocker: &str,
) -> Output {
    let proposal = run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            plan,
            "--phase",
            phase,
            "--defer",
            blocker,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "protect deferred work",
            "--json",
        ],
    );
    let proposal_token = json_output(&proposal)["payload"]["data"]["proposal_token"]
        .as_str()
        .expect("proposal should contain a token")
        .to_owned();
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            plan,
            "--phase",
            phase,
            "--defer",
            blocker,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "protect deferred work",
            "--proposal",
            &proposal_token,
            "--json",
        ],
    )
}

fn set_gate_mode(repository_root: &Path, mode: &str) {
    let configuration_path = repository_root.join(CONFIGURATION_PATH);
    let configuration = fs::read_to_string(&configuration_path).expect("configuration should read");
    let updated = configuration
        .lines()
        .map(|line| {
            if line.starts_with("gate_mode") {
                format!("gate_mode = \"{mode}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(configuration_path, format!("{updated}\n")).expect("configuration should write");
}

/// Propose `proposed` as the new trunk tip with the release valve unset.
fn propose_trunk(repository_root: &Path, previous: &str, proposed: &str) -> Output {
    update_main(repository_root, previous, proposed, ReleaseValve::Unset)
}

/// Put trunk back to `previous` after a permitted observation.
fn restore_trunk(repository_root: &Path, current: &str, previous: &str) {
    assert!(
        propose_trunk(repository_root, current, previous)
            .status
            .success()
    );
}

/// Whether the release valve is set for one invocation.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ReleaseValve {
    Unset,
    Set,
}

fn update_main(
    repository_root: &Path,
    previous: &str,
    proposed: &str,
    release_valve: ReleaseValve,
) -> Output {
    let mut command = Command::new(GIT_BINARY);
    command
        .arg("--no-optional-locks")
        .args(["update-ref", "refs/heads/main", proposed, previous])
        .current_dir(repository_root);
    if release_valve == ReleaseValve::Set {
        command.env(BYPASS_ENVIRONMENT, "1");
    } else {
        command.env_remove(BYPASS_ENVIRONMENT);
    }
    command.output().expect("git update-ref should run")
}

fn pending_bypass_count(repository_root: &Path) -> usize {
    fs::read_dir(repository_root.join(".git"))
        .expect("common git directory should read")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(PENDING_BYPASS_PREFIX))
        .count()
}

fn reference_exists(repository_root: &Path, reference: &str) -> bool {
    Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(["show-ref", "--verify", "--quiet", reference])
        .current_dir(repository_root)
        .status()
        .expect("git show-ref should run")
        .success()
}

fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should report a reservation id")
        .to_owned()
}

fn journal_text(repository_root: &Path) -> String {
    fs::read_to_string(repository_root.join(JOURNAL_PATH)).expect("journal should read")
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should print JSON")
}

fn run_private_hook(repository_root: &Path, phase: &str, input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["__reference-transaction", phase, "refs/heads/main"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove("CARGO_BERTH_RUN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("private gate should start");
    child
        .stdin
        .take()
        .expect("private gate stdin should exist")
        .write_all(input.as_bytes())
        .expect("private gate stdin should write");
    child
        .wait_with_output()
        .expect("private gate should finish")
}

fn run_hook_script(
    repository_root: &Path,
    phase: &str,
    input: &str,
    release_valve: ReleaseValve,
) -> Output {
    run_hook_at_path(
        &repository_root.join(HOOK_PATH),
        repository_root,
        phase,
        input,
        release_valve,
    )
}

fn run_hook_at_path(
    hook_path: &Path,
    repository_root: &Path,
    phase: &str,
    input: &str,
    release_valve: ReleaseValve,
) -> Output {
    let mut command = Command::new(hook_path);
    command
        .arg(phase)
        .current_dir(repository_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if release_valve == ReleaseValve::Set {
        command.env(BYPASS_ENVIRONMENT, "1");
    } else {
        command.env_remove(BYPASS_ENVIRONMENT);
    }
    let mut child = command.spawn().expect("managed hook should start");
    child
        .stdin
        .take()
        .expect("managed hook stdin should exist")
        .write_all(input.as_bytes())
        .expect("managed hook stdin should write");
    child
        .wait_with_output()
        .expect("managed hook should finish")
}

fn run_private_hook_with_git_trace(repository_root: &Path, input: &str) -> TracedHook {
    let directory = tempdir().expect("wrapper directory should exist");
    let wrapper_path = directory.path().join(GIT_BINARY);
    let trace_path = directory.path().join("trace");
    fs::write(&wrapper_path, TRACING_GIT_WRAPPER).expect("git wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let original_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["__reference-transaction", "prepared", "refs/heads/main"])
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(TRACE_ENVIRONMENT, &trace_path)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove("CARGO_BERTH_RUN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("private gate should start");
    child
        .stdin
        .take()
        .expect("private gate stdin should exist")
        .write_all(input.as_bytes())
        .expect("private gate stdin should write");
    let output = child
        .wait_with_output()
        .expect("private gate should finish");
    TracedHook {
        output,
        trace_path,
        _directory: directory,
    }
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove("CARGO_BERTH_RUN")
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_environment(
    repository_root: &Path,
    arguments: &[&str],
    name: &str,
    value: &str,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove("CARGO_BERTH_RUN")
        .env(name, value)
        .output()
        .expect("cargo-berth should run")
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    let output = Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output should be UTF-8")
        .trim()
        .to_owned()
}

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = git_output(repository_root, arguments);
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .output()
        .expect("git should run")
}

fn git_binary() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("test PATH should exist"))
        .map(|directory| directory.join(GIT_BINARY))
        .find(|candidate| candidate.is_file())
        .expect("git should exist on PATH")
}

fn shell_single_quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}
