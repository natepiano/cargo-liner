#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end tests for installation, enforcement, release valves, and gate cost.

use std::ffi::OsStr;
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
const BYPASSED_MERGE_IDENTITY_ENVIRONMENT: &str = "CARGO_BERTH_BYPASSED_MERGE_ID";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GIT_BINARY: &str = "git";
const HOOK_PATH: &str = ".git/hooks/reference-transaction";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const PENDING_BYPASS_PREFIX: &str = "cargo-berth-pending-bypass-";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    case "$2" in
        cat-file) printf '%s %s\n' "$2" "$3" >> "$CARGO_BERTH_TEST_GIT_TRACE" ;;
        merge-base)
            (
                command_name="$2"
                shift 2
                printf '%s' "$command_name" >> "$CARGO_BERTH_TEST_GIT_TRACE"
                for argument in "$@"; do printf ' %s' "$argument" >> "$CARGO_BERTH_TEST_GIT_TRACE"; done
                printf '\n' >> "$CARGO_BERTH_TEST_GIT_TRACE"
            )
            ;;
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
fn failed_managed_hook_refresh_leaves_the_previous_hook_unchanged() {
    let repository = initialized_repository();
    let hook_path = repository.path().join(HOOK_PATH);
    let previous_hook = fs::read(&hook_path).expect("managed hook should read");
    let configuration_path = repository.path().join(CONFIGURATION_PATH);
    let configuration = fs::read_to_string(&configuration_path).expect("configuration should read");
    let renamed_configuration = configuration.replace("trunk = \"main\"", "trunk = \"renamed\"");
    assert_ne!(renamed_configuration, configuration);
    fs::write(&configuration_path, renamed_configuration).expect("configuration should update");
    let hooks_directory = hook_path.parent().expect("hook should have a parent");
    let original_permissions = fs::metadata(hooks_directory)
        .expect("hooks directory metadata should read")
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o555);
    fs::set_permissions(hooks_directory, read_only_permissions)
        .expect("hooks directory should become read-only");

    let refreshed = run_berth(repository.path(), &["init", "--json"]);

    fs::set_permissions(hooks_directory, original_permissions)
        .expect("hooks directory permissions should restore");
    assert!(refreshed.status.success());
    let payload = json_output(&refreshed);
    assert_eq!(
        payload["payload"]["data"]["hooks"][0]["activation"]["status"],
        "inactive"
    );
    assert_eq!(
        fs::read(&hook_path).expect("managed hook should remain readable"),
        previous_hook
    );
}

#[test]
fn session_mapping_authorizes_only_its_live_claim_and_retires_at_checkpoint() {
    let repository = initialized_repository();
    let session_id = "session-live-claim";
    let claimed = run_berth_with_session(
        repository.path(),
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "protect mapped work",
            "--json",
        ],
        session_id,
    );
    assert!(claimed.status.success());
    assert_eq!(
        json_output(&claimed)["payload"]["data"]["session_mapping_publication"]["status"],
        "published"
    );
    let reservation_id = reservation_id(&claimed);
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("coordination marker should remove");

    let mapped = run_berth_with_session(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        session_id,
    );
    assert!(mapped.status.success());
    assert_eq!(json_output(&mapped)["status"], "clear");

    let mapping_before_checkpoint =
        fs::read(repository.path().join(SESSION_MAPPING_PATH)).expect("mapping should read");
    let checkpointed = run_berth_with_session(
        repository.path(),
        &["release", &reservation_id, "--json"],
        session_id,
    );
    assert!(checkpointed.status.success());
    assert_eq!(
        json_output(&checkpointed)["payload"]["data"]["session_mapping_publication"]["status"],
        "published"
    );
    assert!(
        !fs::read_to_string(repository.path().join(SESSION_MAPPING_PATH))
            .expect("retired mapping should read")
            .contains(session_id)
    );
    let next_claim = claim(
        repository.path(),
        "file:tests/base.rs",
        FIRST_RUN,
        "docs/session.md",
        "phase-after-checkpoint",
    );
    assert!(next_claim.status.success());
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("coordination marker should remove");
    let retired = run_berth_with_session(
        repository.path(),
        &["check", "file:tests/base.rs", "--json"],
        session_id,
    );
    assert_eq!(retired.status.code(), Some(1));
    assert_eq!(json_output(&retired)["status"], "blocked_by_overlap");

    fs::write(
        repository.path().join(SESSION_MAPPING_PATH),
        mapping_before_checkpoint,
    )
    .expect("stale mapping should write");
    let stale = run_berth_with_session(
        repository.path(),
        &["check", "file:tests/base.rs", "--json"],
        session_id,
    );
    assert_eq!(stale.status.code(), Some(1));
    assert_eq!(json_output(&stale)["status"], "blocked_by_overlap");
}

#[test]
fn session_mapping_authorizes_every_reservation_owned_by_its_run() {
    let repository = initialized_repository();
    let session_id = "same-run-reservations";
    let first = run_berth_with_session(
        repository.path(),
        &[
            "claim",
            "file:first.txt",
            "--run",
            FIRST_RUN,
            "--why",
            "protect first phase",
            "--json",
        ],
        session_id,
    );
    assert!(first.status.success());
    let second = claim(
        repository.path(),
        "file:second.txt",
        FIRST_RUN,
        "docs/session.md",
        "second-phase",
    );
    assert!(second.status.success());

    let session_check = run_berth_with_session(
        repository.path(),
        &["check", "file:second.txt", "--json"],
        session_id,
    );
    let marker_check = run_berth(repository.path(), &["check", "file:second.txt", "--json"]);

    assert!(session_check.status.success());
    assert!(marker_check.status.success());
    assert_eq!(json_output(&session_check)["status"], "clear");
    assert_eq!(json_output(&marker_check)["status"], "clear");
}

#[test]
fn mapping_publication_failures_are_reported_by_claim_and_checkpoint() {
    let claim_repository = initialized_repository();
    fs::create_dir(claim_repository.path().join(SESSION_MAPPING_PATH))
        .expect("mapping destination directory should exist");
    let claimed = run_berth_with_session(
        claim_repository.path(),
        &[
            "claim",
            "file:claim.txt",
            "--run",
            FIRST_RUN,
            "--why",
            "exercise mapping failure",
            "--json",
        ],
        "claim-publication-failure",
    );
    assert!(claimed.status.success());
    assert_eq!(
        json_output(&claimed)["payload"]["data"]["session_mapping_publication"]["status"],
        "unavailable"
    );

    let checkpoint_repository = initialized_repository();
    let checkpoint_claim = run_berth_with_session(
        checkpoint_repository.path(),
        &[
            "claim",
            "file:checkpoint.txt",
            "--run",
            FIRST_RUN,
            "--why",
            "exercise checkpoint mapping failure",
            "--json",
        ],
        "checkpoint-publication-failure",
    );
    assert!(checkpoint_claim.status.success());
    let reservation_id = reservation_id(&checkpoint_claim);
    fs::remove_file(checkpoint_repository.path().join(SESSION_MAPPING_PATH))
        .expect("mapping file should remove");
    fs::create_dir(checkpoint_repository.path().join(SESSION_MAPPING_PATH))
        .expect("mapping destination directory should exist");
    let checkpointed = run_berth_with_session(
        checkpoint_repository.path(),
        &["release", &reservation_id, "--json"],
        "checkpoint-publication-failure",
    );
    assert!(checkpointed.status.success());
    assert_eq!(
        json_output(&checkpointed)["payload"]["data"]["session_mapping_publication"]["status"],
        "unavailable"
    );
}

#[test]
fn integrate_reports_an_inactive_session_mapping_without_a_marker_diagnostic() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let integration_root = add_worktree(
        repository.path(),
        worktrees.path(),
        "inactive-session-integration",
    );
    let session_id = "stale-integration-session";
    let mapped_claim = run_berth_with_session(
        &integration_root,
        &[
            "claim",
            "file:mapped-session",
            "--run",
            FIRST_RUN,
            "--why",
            "establish integration session mapping",
            "--json",
        ],
        session_id,
    );
    assert!(mapped_claim.status.success());
    let mapped_reservation_id = reservation_id(&mapped_claim);
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(
            &integration_root,
            &["release", &mapped_reservation_id, "--json"],
        )
        .status
        .success()
    );
    let integrating_claim = claim(
        &integration_root,
        "file:integrating-session",
        SECOND_RUN,
        "docs/integrating-session.md",
        "integration",
    );
    assert!(integrating_claim.status.success());
    let integrating_reservation_id = reservation_id(&integrating_claim);
    commit_work(
        &integration_root,
        "integrating-session",
        "integration work\n",
        "integration work",
    );
    fs::write(&mapping_path, stale_mapping).expect("stale session mapping should write");

    let rejected = run_berth_with_session(
        &integration_root,
        &["integrate", &integrating_reservation_id, "--json"],
        session_id,
    );
    let rejected_json = json_output(&rejected);
    let diagnostic = rejected_json["message"]
        .as_str()
        .expect("integration rejection should have a message");

    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(rejected_json["status"], "invalid_input");
    assert!(diagnostic.contains("harness session mapping"));
    assert!(!diagnostic.contains("coordination-run marker"));
}

#[test]
fn unavailable_session_mapping_falls_through_to_marker_and_environment() {
    let repository = initialized_repository();
    let claimed = claim(
        repository.path(),
        "file:src/lib.rs",
        FIRST_RUN,
        "docs/session.md",
        "phase-session",
    );
    assert!(claimed.status.success());
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);

    fs::write(&mapping_path, "not json\n").expect("corrupt mapping should write");
    let marker_fallback = run_berth_with_session(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        "session-corrupt-marker",
    );
    assert!(marker_fallback.status.success());
    assert_eq!(json_output(&marker_fallback)["status"], "clear");

    fs::remove_file(&mapping_path).expect("mapping should remove");
    let absent_marker_fallback = run_berth_with_session(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        "session-absent-marker",
    );
    assert!(absent_marker_fallback.status.success());

    fs::write(&mapping_path, "not json\n").expect("corrupt mapping should write");
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("coordination marker should remove");
    let environment_fallback = run_berth_with_session_and_run(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        "session-corrupt-environment",
        FIRST_RUN,
    );
    assert!(environment_fallback.status.success());
    assert_eq!(json_output(&environment_fallback)["status"], "clear");

    fs::remove_file(&mapping_path).expect("mapping should remove");
    let absent_environment_fallback = run_berth_with_session_and_run(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        "session-absent-environment",
        FIRST_RUN,
    );
    assert!(absent_environment_fallback.status.success());
}

#[test]
fn session_mapping_survives_unavailable_marker_publication() {
    let repository = initialized_repository();
    let marker_path = repository.path().join(MARKER_PATH);
    let git_directory = repository.path().join(".git");
    let original_permissions = fs::metadata(&git_directory)
        .expect("git directory metadata should read")
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o555);
    fs::set_permissions(&git_directory, read_only_permissions)
        .expect("git directory should become read-only");
    let claimed = run_berth_with_session(
        repository.path(),
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "protect mapped work",
            "--json",
        ],
        "session-without-marker",
    );
    fs::set_permissions(&git_directory, original_permissions)
        .expect("git directory permissions should restore");
    assert!(claimed.status.success());
    assert_eq!(
        json_output(&claimed)["payload"]["data"]["marker_publication"]["status"],
        "unavailable"
    );
    assert!(!marker_path.exists());

    let mapped = run_berth_with_session(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        "session-without-marker",
    );
    assert!(mapped.status.success());
    assert_eq!(json_output(&mapped)["status"], "clear");
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
fn managed_hook_dispatches_only_actionable_phase_and_reference_pairs() {
    let repository = initialized_repository();
    let spy = replace_managed_hook_executable_with_spy(repository.path());
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let trunk = format!("{base} {base} refs/heads/main\n");
    let prefix = format!("{base} {base} refs/heads/main-old\n");
    let detached = format!("{base} {base} HEAD\n");
    let remote = format!("{base} {base} refs/remotes/origin/main\n");
    let local_feature = format!("{base} {base} refs/heads/feature\n");

    for (phase, input) in [
        ("preparing", trunk.as_str()),
        ("aborted", trunk.as_str()),
        ("future-phase", trunk.as_str()),
        ("prepared", prefix.as_str()),
        ("prepared", detached.as_str()),
        ("prepared", remote.as_str()),
        ("committed", detached.as_str()),
        ("committed", remote.as_str()),
    ] {
        assert!(
            run_hook_script(repository.path(), phase, input, ReleaseValve::Unset)
                .status
                .success()
        );
    }
    assert!(!spy.phase_log.exists());

    assert!(
        run_hook_script(repository.path(), "prepared", &trunk, ReleaseValve::Unset,)
            .status
            .success()
    );
    assert!(
        run_hook_script(
            repository.path(),
            "committed",
            &local_feature,
            ReleaseValve::Set,
        )
        .status
        .success()
    );
    assert!(
        run_hook_script(
            repository.path(),
            "prepared",
            "one-field\n",
            ReleaseValve::Unset,
        )
        .status
        .success()
    );
    assert!(
        run_hook_script(
            repository.path(),
            "prepared",
            "invalid invalid refs/heads/feature\n",
            ReleaseValve::Unset,
        )
        .status
        .success()
    );

    assert_eq!(
        fs::read_to_string(&spy.phase_log)
            .expect("spy phase log should read")
            .lines()
            .collect::<Vec<_>>(),
        ["prepared", "committed", "prepared", "prepared"]
    );
}

#[test]
fn managed_hook_reports_scenario_specific_binary_invocation_counts() {
    let prepared_trunk_update = prepared_trunk_update_hook_phases();
    let committed_feature_rebase = committed_feature_rebase_hook_phases();
    let committed_forced_trunk_integration = committed_forced_trunk_integration_hook_phases();

    eprintln!(
        "reference-transaction binary invocations: prepared trunk update={}, committed feature rebase={}, committed forced trunk integration={}",
        prepared_trunk_update.len(),
        committed_feature_rebase.len(),
        committed_forced_trunk_integration.len(),
    );
    assert_eq!(prepared_trunk_update, ["prepared", "committed"]);
    assert_eq!(committed_feature_rebase, ["committed"]);
    assert_eq!(
        committed_forced_trunk_integration,
        ["prepared", "committed"]
    );
}

#[test]
fn managed_hook_replays_unchanged_transaction_bytes_into_the_binary() {
    let repository = initialized_repository();
    let spy = replace_managed_hook_executable_with_spy(repository.path());
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base}\t{base}  refs/heads/main");

    let dispatched = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Unset);

    assert!(dispatched.status.success());
    assert_eq!(
        fs::read(&spy.stdin_log).expect("spy stdin log should read"),
        input.as_bytes()
    );
}

#[test]
fn managed_hook_sends_non_ascii_and_control_bytes_to_the_binary() {
    for reference_suffix in [
        b"control\x01byte".as_slice(),
        b"delete\x7fbyte".as_slice(),
        b"non-utf8\xff".as_slice(),
        b"feature\0refs/heads/main".as_slice(),
    ] {
        let repository = initialized_repository();
        let spy = replace_managed_hook_executable_with_spy(repository.path());
        let base = git_stdout(repository.path(), &["rev-parse", "main"]);
        let mut input = format!("{base} {base} refs/heads/").into_bytes();
        input.extend_from_slice(reference_suffix);
        input.push(b'\n');

        let dispatched = run_hook_script_bytes(repository.path(), "prepared", &input);

        assert!(dispatched.status.success());
        assert_eq!(
            fs::read_to_string(&spy.phase_log)
                .expect("spy phase log should read")
                .trim(),
            "prepared"
        );
        assert_eq!(
            fs::read(&spy.stdin_log).expect("spy stdin log should read"),
            input
        );
    }
}

#[test]
fn managed_hook_never_replays_a_partial_transaction_after_buffering_fails() {
    let repository = initialized_repository();
    let spy = replace_managed_hook_executable_with_spy(repository.path());
    let command_directory = tempdir().expect("command directory should exist");
    let failing_cat = command_directory.path().join("cat");
    fs::write(
        &failing_cat,
        "#!/bin/sh\nIFS= read -r ignored || :\nprintf '%s' partial\nexit 1\n",
    )
    .expect("failing cat should write");
    let mut permissions = fs::metadata(&failing_cat)
        .expect("failing cat metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&failing_cat, permissions).expect("failing cat should be executable");
    let inherited_path = std::env::var_os("PATH").expect("test PATH should exist");
    let command_search_path = std::env::join_paths(
        std::iter::once(command_directory.path().to_path_buf())
            .chain(std::env::split_paths(&inherited_path)),
    )
    .expect("command search path should join");
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base} {base} refs/heads/main\n");

    let rejected = run_hook_script_bytes_with_command_search_path(
        repository.path(),
        "prepared",
        input.as_bytes(),
        &command_search_path,
    );

    assert!(!rejected.status.success());
    assert!(!spy.phase_log.exists());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("partial input"));
}

#[test]
fn renamed_trunk_refreshes_dispatch_before_next_prepared_update() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);

    let renamed = git_output(repository.path(), &["branch", "-m", "main", "renamed"]);

    assert!(
        renamed.status.success(),
        "trunk rename failed: {}",
        String::from_utf8_lossy(&renamed.stderr)
    );
    let hook_path = repository.path().join(HOOK_PATH);
    let refreshed = fs::read_to_string(&hook_path).expect("refreshed hook should read");
    assert!(
        refreshed.contains("cargo_berth_trunk_reference='refs/heads/renamed'"),
        "hook did not refresh after branch rename: {}",
        String::from_utf8_lossy(&renamed.stderr)
    );
    assert!(!refreshed.contains("cargo_berth_trunk_reference='refs/heads/main'"));

    let spy = replace_managed_hook_executable_with_spy(repository.path());
    let prepared = run_hook_script(
        repository.path(),
        "prepared",
        &format!("{base} {base} refs/heads/renamed\n"),
        ReleaseValve::Unset,
    );
    assert!(prepared.status.success());
    assert_eq!(
        fs::read_to_string(spy.phase_log)
            .expect("spy phase log should read")
            .trim(),
        "prepared"
    );
}

#[test]
fn deleting_trunk_with_one_unrelated_same_tip_branch_leaves_dispatch_unchanged() {
    let repository = initialized_repository();
    git(
        repository.path(),
        &["checkout", "--quiet", "-b", "unrelated", "main"],
    );
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read(&hook_path).expect("managed hook should read");

    let deleted = git_output(repository.path(), &["branch", "-D", "main"]);

    assert!(
        deleted.status.success(),
        "trunk deletion failed: {}",
        String::from_utf8_lossy(&deleted.stderr)
    );
    let retained = fs::read(&hook_path).expect("retained hook should read");
    assert_eq!(retained, installed);
    assert!(
        String::from_utf8_lossy(&retained)
            .contains("cargo_berth_trunk_reference='refs/heads/main'")
    );
}

#[test]
fn deleting_trunk_with_two_proven_same_tip_renames_leaves_dispatch_unchanged() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "branch",
            "-m",
            "main",
            "candidate-a",
        ],
    );
    for candidate in ["main", "candidate-b", "candidate-c"] {
        git(
            repository.path(),
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "branch",
                candidate,
                "candidate-a",
            ],
        );
    }
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read(&hook_path).expect("managed hook should read");
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "branch",
            "-m",
            "main",
            "candidate-d",
        ],
    );
    for candidate in ["candidate-a", "candidate-d"] {
        let candidate_reference = format!("refs/heads/{candidate}");
        assert_eq!(
            git_stdout(
                repository.path(),
                &[
                    "reflog",
                    "show",
                    "--max-count=1",
                    "--format=%gs",
                    &candidate_reference,
                ],
            ),
            format!("Branch: renamed refs/heads/main to {candidate_reference}")
        );
    }
    let deleted_object_id = "0".repeat(base.len());

    let refreshed = run_hook_script(
        repository.path(),
        "committed",
        &format!("{base} {deleted_object_id} refs/heads/main\n"),
        ReleaseValve::Unset,
    );

    assert!(
        refreshed.status.success(),
        "committed trunk deletion hook failed: {}",
        String::from_utf8_lossy(&refreshed.stderr)
    );
    let retained = fs::read(&hook_path).expect("retained hook should read");
    assert_eq!(retained, installed);
    assert!(
        String::from_utf8_lossy(&retained)
            .contains("cargo_berth_trunk_reference='refs/heads/main'")
    );
}

#[test]
fn stale_trunk_reference_invokes_for_a_prepared_local_update() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let renamed = git_output(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "branch",
            "-m",
            "main",
            "renamed",
        ],
    );
    assert!(renamed.status.success());
    let spy = replace_managed_hook_executable_with_spy(repository.path());

    let prepared = run_hook_script(
        repository.path(),
        "prepared",
        &format!("{base} {base} refs/heads/renamed\n"),
        ReleaseValve::Unset,
    );

    assert!(prepared.status.success());
    assert_eq!(
        fs::read_to_string(spy.phase_log)
            .expect("spy phase log should read")
            .trim(),
        "prepared"
    );
}

#[test]
fn filtered_three_commit_rebases_report_median_and_maximum_wall_time() {
    const SAMPLE_COUNT: usize = 10;

    let repository = initialized_repository();
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "-b",
            "rebase-source",
        ],
    );
    for commit_index in 0..3 {
        let path = format!("rebase-{commit_index}.txt");
        fs::write(repository.path().join(&path), format!("{commit_index}\n"))
            .expect("rebase source should write");
        git(repository.path(), &["add", &path]);
        git(
            repository.path(),
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                &format!("rebase source {commit_index}"),
            ],
        );
    }
    let source_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "main",
        ],
    );
    fs::write(repository.path().join("upstream-rebase.txt"), "upstream\n")
        .expect("upstream source should write");
    git(repository.path(), &["add", "upstream-rebase.txt"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "upstream rebase base",
        ],
    );

    let mut no_hook_samples = Vec::with_capacity(SAMPLE_COUNT);
    let mut filtered_bypass_samples = Vec::with_capacity(SAMPLE_COUNT);
    let mut filtered_live_samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        no_hook_samples.push(run_three_commit_rebase_sample(
            repository.path(),
            &source_tip,
            RebaseHookMode::Disabled,
        ));
        filtered_bypass_samples.push(run_three_commit_rebase_sample(
            repository.path(),
            &source_tip,
            RebaseHookMode::FilteredBypass,
        ));
        filtered_live_samples.push(run_three_commit_rebase_sample(
            repository.path(),
            &source_tip,
            RebaseHookMode::FilteredLive,
        ));
    }

    for (label, samples) in [
        ("no-hook", no_hook_samples),
        ("filtered-bypass", filtered_bypass_samples),
        ("filtered-live", filtered_live_samples),
    ] {
        let summary = RebaseTimingSummary::from_samples(samples);
        eprintln!(
            "three-commit rebase {label}: median={:?}, maximum={:?}",
            summary.median, summary.maximum
        );
    }
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

    let bypassed = run_hook_script_with_bypassed_merge_identity(
        repository.path(),
        "prepared",
        &input,
        ReleaseValve::Set,
        "inherited-\"quoted\\identity",
    );
    assert!(bypassed.status.success());
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("executable is unavailable"));
    assert!(diagnostic.contains("Rerun cargo berth init"));
    assert_eq!(pending_bypass_count(repository.path()), 1);
    let marker = pending_bypass_marker(repository.path());
    let bypassed_merge = marker["cause"]["bypassed_merge"]
        .as_str()
        .expect("pending marker should carry a bypassed merge identity");
    assert!(bypassed_merge.starts_with("git-process-"));
    assert!(
        bypassed_merge
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    );
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
fn bypassed_non_trunk_transactions_leave_no_audit_fact() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base} {base} ORIG_HEAD\n{base} {base} refs/heads/topic\n");

    let bypassed = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Set);

    assert!(bypassed.status.success());
    assert_eq!(environment_bypass_record_count(repository.path()), 0);
    assert_eq!(pending_bypass_count(repository.path()), 0);
    assert!(bypassed.stderr.is_empty());
}

#[test]
fn one_trunk_transaction_among_merge_hook_invocations_records_one_audit_fact() {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let trunk_input = format!("{base} {base} refs/heads/main\n");

    let trunk = run_hook_script(
        repository.path(),
        "prepared",
        &trunk_input,
        ReleaseValve::Set,
    );

    assert!(trunk.status.success());
    assert_eq!(environment_bypass_record_count(repository.path()), 1);
    let non_trunk_input = format!("{base} {base} ORIG_HEAD\n{base} {base} AUTO_MERGE\n");

    let non_trunk = run_hook_script(
        repository.path(),
        "prepared",
        &non_trunk_input,
        ReleaseValve::Set,
    );

    assert!(non_trunk.status.success());
    assert_eq!(environment_bypass_record_count(repository.path()), 1);
    assert_eq!(pending_bypass_count(repository.path()), 0);
    assert!(non_trunk.stderr.is_empty());
}

#[test]
fn an_unconfigured_worktree_bypasses_without_writing_shared_audit_state() {
    let repository = initialized_repository();
    let configuration_path = repository.path().join(CONFIGURATION_PATH);
    let saved_configuration = repository.path().join("berth.toml.saved");
    fs::rename(&configuration_path, &saved_configuration).expect("configuration should move aside");
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let input = format!("{base} {base} refs/heads/main\n");

    let bypassed = run_hook_script(repository.path(), "prepared", &input, ReleaseValve::Set);

    assert!(
        bypassed.status.success(),
        "bypass must never fail a ref transaction: {}",
        String::from_utf8_lossy(&bypassed.stderr)
    );
    assert!(
        !journal_text(repository.path())
            .lines()
            .filter_map(|record| serde_json::from_str::<serde_json::Value>(record).ok())
            .any(|record| record["op"] == "bypass"),
        "an unenrolled worktree must not append a bypass record"
    );
    assert_eq!(pending_bypass_count(repository.path()), 0);
    assert!(bypassed.stderr.is_empty());
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
fn a_trunk_bypass_without_an_invocation_directory_leaves_a_fallback_marker() {
    let repository = initialized_repository();
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read_to_string(&hook_path).expect("managed hook should read");
    let policy_worktree_path =
        fs::canonicalize(repository.path()).expect("policy worktree should resolve");
    let policy_worktree = shell_single_quoted(&policy_worktree_path);
    let unavailable_policy_worktree =
        shell_single_quoted(&repository.path().join("unavailable-policy-worktree"));
    let detached = installed.replace(&policy_worktree, &unavailable_policy_worktree);
    assert_ne!(detached, installed);
    fs::write(&hook_path, detached).expect("detached hook fixture should write");

    let directory = tempdir().expect("temporary parent should exist");
    let removed_directory = directory.path().join("removed");
    fs::create_dir(&removed_directory).expect("removable directory should exist");
    let command = format!(
        "cd {} && rmdir {} && exec {} prepared",
        shell_single_quoted(&removed_directory),
        shell_single_quoted(&removed_directory),
        shell_single_quoted(&hook_path),
    );
    let mut child = Command::new("sh")
        .args(["-c", &command])
        .env(BYPASS_ENVIRONMENT, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("managed hook should run from a removed directory");
    child
        .stdin
        .take()
        .expect("managed hook stdin should exist")
        .write_all(
            b"0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 refs/heads/main\n",
        )
        .expect("reference transaction should write");

    let bypassed = child
        .wait_with_output()
        .expect("managed hook should finish from a removed directory");

    assert!(
        bypassed.status.success(),
        "bypass must never fail a ref transaction: {}",
        String::from_utf8_lossy(&bypassed.stderr)
    );
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("could not resolve its invocation directory"));
    assert!(diagnostic.contains("could not record this bypass"));
    assert_eq!(pending_bypass_count(repository.path()), 1);
}

#[test]
fn an_unrecorded_binary_bypass_warns_without_blocking_the_ref_update() {
    let non_repository = tempdir().expect("non-repository directory should exist");

    let bypassed = run_berth_with_input_and_environment(
        non_repository.path(),
        &["__reference-transaction", "prepared", "refs/heads/main"],
        "one-field\n",
        BYPASS_ENVIRONMENT,
        "1",
    );

    assert_eq!(bypassed.status.code(), Some(4));
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

    let mut child = Command::new("sh")
        .args(["-c", &command])
        .env(BYPASS_ENVIRONMENT, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("private gate should run from a removed directory");
    child
        .stdin
        .take()
        .expect("private gate stdin should exist")
        .write_all(
            b"0000000000000000000000000000000000000000 0000000000000000000000000000000000000000 refs/heads/main\n",
        )
        .expect("reference transaction should write");
    let bypassed = child
        .wait_with_output()
        .expect("private gate should finish from a removed directory");

    assert_eq!(bypassed.status.code(), Some(4));
    let diagnostic = String::from_utf8_lossy(&bypassed.stderr);
    assert!(diagnostic.contains("took the CARGO_BERTH_BYPASS=1 override"));
    assert!(diagnostic.contains("could not resolve its invocation directory"));
    assert!(diagnostic.contains("override could not be recorded here"));
    assert!(diagnostic.contains("marker is being left to report it later"));
    assert!(diagnostic.contains("ref transaction remains permitted"));
    assert!(diagnostic.contains("rerun cargo berth init"));
}

#[test]
fn non_trunk_updates_and_an_unconfigured_trunk_gate_are_silent() {
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
    assert!(possible_trunk.stderr.is_empty());
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
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
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
        format!("{base} {blocked_head} refs/heads/main\n").as_bytes(),
        ReleaseValve::Unset,
        BypassedMergeIdentityEnvironment::Unset,
        HookCommandSearchPath::Inherited,
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
fn committed_hook_persists_one_scoped_patch_evaluation() {
    let repository = initialized_repository();
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let claimed = claim(
        repository.path(),
        "file:src/lib.rs",
        FIRST_RUN,
        "docs/scoped-cache.md",
        "scoped-cache",
    );
    let reservation_id = reservation_id(&claimed);
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn protected() {}\n",
    )
    .expect("protected source should write");
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected identity",
        ],
    );
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id, "--json"])
            .status
            .success()
    );
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "rewritten target",
        ],
    );
    let target = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    assert!(!journal_text(repository.path()).contains("scoped_patch_equivalence_checked"));

    let traced = run_private_hook_with_git_trace(
        repository.path(),
        HookPhase::Committed,
        &format!("{protected_tip} {target} refs/heads/main\n"),
    );
    assert!(
        traced.output.status.success(),
        "committed hook failed: {}",
        String::from_utf8_lossy(&traced.output.stderr)
    );
    let trace = fs::read_to_string(&traced.trace_path).expect("git trace should read");
    let scoped_evaluation = format!("merge-base {phase_start_head} {target}");
    assert_eq!(
        trace
            .lines()
            .filter(|command| *command == scoped_evaluation)
            .count(),
        1
    );
    assert_eq!(
        journal_text(repository.path())
            .matches("\"op\":\"scoped_patch_equivalence_checked\"")
            .count(),
        1
    );
}

#[test]
fn prepared_gate_advances_each_subject_at_actual_and_proposed_targets() {
    let fixture = prepared_gate_scoped_comparison_fixture();
    let input = format!("{} {} refs/heads/main\n", fixture.actual, fixture.proposed);

    for _ in 0..=fixture.reservation_ids.len() {
        let prepared = run_private_hook(fixture.repository.path(), "prepared", &input);
        assert!(
            prepared.status.success(),
            "prepared hook failed: {}",
            String::from_utf8_lossy(&prepared.stderr)
        );
        assert_eq!(
            git_stdout(fixture.repository.path(), &["rev-parse", "HEAD"]),
            fixture.actual
        );
    }
    for target in [&fixture.actual, &fixture.proposed] {
        assert_target_compared_each_reservation(&fixture, target);
    }

    let board = run_berth(fixture.repository.path(), &["board", "--json"]);
    assert!(board.status.success());
    let data = &json_output(&board)["payload"]["data"];
    for reservation_id in &fixture.reservation_ids {
        let row = reservation_row(data, reservation_id);
        assert_eq!(
            row["integration_evidence"]["status"]["status"],
            "integrated"
        );
        assert_eq!(
            row["integration_evidence"]["status"]["proof"],
            "scoped_patch_equivalent"
        );
    }
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
        HookPhase::Prepared,
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
        3
    );
    assert_eq!(
        trace
            .lines()
            .filter(|command| *command == "rev-list")
            .count(),
        5
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

struct ManagedHookSpy {
    phase_log: PathBuf,
    stdin_log: PathBuf,
}

#[derive(Clone, Copy)]
enum RebaseHookMode {
    Disabled,
    FilteredBypass,
    FilteredLive,
}

struct RebaseTimingSummary {
    median:  Duration,
    maximum: Duration,
}

impl RebaseTimingSummary {
    fn from_samples(mut samples: Vec<Duration>) -> Self {
        samples.sort_unstable();
        Self {
            median:  samples[samples.len() / 2],
            maximum: samples[samples.len() - 1],
        }
    }
}

struct DeferredPair {
    worktrees:    TempDir,
    blocked_root: PathBuf,
    blocked_id:   String,
    holder_id:    String,
}

struct PreparedGateScopedComparisonFixture {
    repository:      TempDir,
    _worktrees:      TempDir,
    reservation_ids: Vec<String>,
    actual:          String,
    proposed:        String,
}

fn prepared_gate_scoped_comparison_fixture() -> PreparedGateScopedComparisonFixture {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let scopes = (0..4)
        .map(|index| format!("src/proposed-{index}.rs"))
        .collect::<Vec<_>>();
    let reservation_ids = scopes
        .iter()
        .map(|scope| {
            reservation_id(&claim(
                repository.path(),
                &format!("file:{scope}"),
                FIRST_RUN,
                "docs/proposed-round-robin.md",
                "proposed round robin",
            ))
        })
        .collect::<Vec<_>>();
    let protected_tip =
        commit_scoped_target(repository.path(), &scopes, "protected proposal subjects");
    for reservation_id in &reservation_ids {
        let released = run_berth(repository.path(), &["release", reservation_id, "--json"]);
        assert!(
            released.status.success(),
            "release failed: {}",
            String::from_utf8_lossy(&released.stderr)
        );
    }
    git(
        repository.path(),
        &[
            "update-ref",
            "refs/heads/protected-round-robin",
            &protected_tip,
        ],
    );
    git(
        repository.path(),
        &["-c", "core.hooksPath=/dev/null", "reset", "--hard", &base],
    );
    let actual = commit_scoped_target(repository.path(), &scopes, "actual equivalent target");
    let worktrees = tempdir().expect("worktree parent should exist");
    let proposed_root = worktrees.path().join("uncommitted-proposal");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--detach",
            proposed_root
                .to_str()
                .expect("proposal worktree path should be UTF-8"),
            &base,
        ],
    );
    let proposed = commit_scoped_target(&proposed_root, &scopes, "proposed equivalent target");
    PreparedGateScopedComparisonFixture {
        repository,
        _worktrees: worktrees,
        reservation_ids,
        actual,
        proposed,
    }
}

fn commit_scoped_target(repository_root: &Path, scopes: &[String], message: &str) -> String {
    for (index, scope) in scopes.iter().enumerate() {
        fs::write(
            repository_root.join(scope),
            format!("pub fn proposed_{index}() {{}}\n"),
        )
        .expect("scoped target source should write");
    }
    git(repository_root, &["add", "src"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn assert_target_compared_each_reservation(
    fixture: &PreparedGateScopedComparisonFixture,
    target: &str,
) {
    let records = journal_text(fixture.repository.path())
        .lines()
        .map(|record| {
            serde_json::from_str::<serde_json::Value>(record).expect("journal record should parse")
        })
        .filter(|record| {
            record["op"] == "scoped_patch_equivalence_checked" && record["target"] == target
        })
        .collect::<Vec<_>>();
    let mut compared_reservations = records
        .iter()
        .map(|record| {
            record["reservation_id"]
                .as_str()
                .expect("scoped comparison should identify its reservation")
                .to_owned()
        })
        .collect::<Vec<_>>();
    compared_reservations.sort();
    let mut expected_reservations = fixture.reservation_ids.clone();
    expected_reservations.sort();
    assert_eq!(compared_reservations, expected_reservations);
    assert!(
        records
            .iter()
            .all(|record| record["verdict"] == "integrated")
    );
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

#[derive(Clone, Copy)]
enum BypassedMergeIdentityEnvironment<'environment> {
    Inherited(&'environment str),
    Unset,
}

#[derive(Clone, Copy)]
enum HookCommandSearchPath<'environment> {
    Inherited,
    Explicit(&'environment OsStr),
}

#[derive(Clone, Copy)]
enum HookPhase {
    Prepared,
    Committed,
}

impl HookPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Committed => "committed",
        }
    }
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

fn environment_bypass_record_count(repository_root: &Path) -> usize {
    journal_text(repository_root)
        .lines()
        .filter_map(|record| serde_json::from_str::<serde_json::Value>(record).ok())
        .filter(|record| record["op"] == "bypass")
        .count()
}

fn pending_bypass_marker(repository_root: &Path) -> serde_json::Value {
    let marker_path = fs::read_dir(repository_root.join(".git"))
        .expect("common git directory should read")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PENDING_BYPASS_PREFIX))
        })
        .expect("pending bypass marker should exist");
    let marker = fs::read(marker_path).expect("pending bypass marker should read");
    serde_json::from_slice(&marker).expect("pending bypass marker should contain valid JSON")
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

fn reservation_row<'board>(
    data: &'board serde_json::Value,
    reservation_id: &str,
) -> &'board serde_json::Value {
    ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .find(|row| row["reservation_id"] == reservation_id)
        .expect("reservation should have a board row")
}

fn run_private_hook(repository_root: &Path, phase: &str, input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["__reference-transaction", phase, "refs/heads/main"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
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
        input.as_bytes(),
        release_valve,
        BypassedMergeIdentityEnvironment::Unset,
        HookCommandSearchPath::Inherited,
    )
}

fn run_hook_script_bytes(repository_root: &Path, phase: &str, input: &[u8]) -> Output {
    run_hook_at_path(
        &repository_root.join(HOOK_PATH),
        repository_root,
        phase,
        input,
        ReleaseValve::Unset,
        BypassedMergeIdentityEnvironment::Unset,
        HookCommandSearchPath::Inherited,
    )
}

fn run_hook_script_bytes_with_command_search_path(
    repository_root: &Path,
    phase: &str,
    input: &[u8],
    command_search_path: &OsStr,
) -> Output {
    run_hook_at_path(
        &repository_root.join(HOOK_PATH),
        repository_root,
        phase,
        input,
        ReleaseValve::Unset,
        BypassedMergeIdentityEnvironment::Unset,
        HookCommandSearchPath::Explicit(command_search_path),
    )
}

fn run_hook_script_with_bypassed_merge_identity(
    repository_root: &Path,
    phase: &str,
    input: &str,
    release_valve: ReleaseValve,
    bypassed_merge_identity: &str,
) -> Output {
    run_hook_at_path(
        &repository_root.join(HOOK_PATH),
        repository_root,
        phase,
        input.as_bytes(),
        release_valve,
        BypassedMergeIdentityEnvironment::Inherited(bypassed_merge_identity),
        HookCommandSearchPath::Inherited,
    )
}

fn run_hook_at_path(
    hook_path: &Path,
    repository_root: &Path,
    phase: &str,
    input: &[u8],
    release_valve: ReleaseValve,
    bypassed_merge_identity_environment: BypassedMergeIdentityEnvironment<'_>,
    command_search_path: HookCommandSearchPath<'_>,
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
    match bypassed_merge_identity_environment {
        BypassedMergeIdentityEnvironment::Inherited(bypassed_merge_identity) => {
            command.env(BYPASSED_MERGE_IDENTITY_ENVIRONMENT, bypassed_merge_identity);
        },
        BypassedMergeIdentityEnvironment::Unset => {
            command.env_remove(BYPASSED_MERGE_IDENTITY_ENVIRONMENT);
        },
    }
    match command_search_path {
        HookCommandSearchPath::Inherited => {},
        HookCommandSearchPath::Explicit(path) => {
            command.env("PATH", path);
        },
    }
    let mut child = command.spawn().expect("managed hook should start");
    child
        .stdin
        .take()
        .expect("managed hook stdin should exist")
        .write_all(input)
        .expect("managed hook stdin should write");
    child
        .wait_with_output()
        .expect("managed hook should finish")
}

fn run_private_hook_with_git_trace(
    repository_root: &Path,
    hook_phase: HookPhase,
    input: &str,
) -> TracedHook {
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
        .args([
            "__reference-transaction",
            hook_phase.as_str(),
            "refs/heads/main",
        ])
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(TRACE_ENVIRONMENT, &trace_path)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
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
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
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
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(name, value)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_input_and_environment(
    repository_root: &Path,
    arguments: &[&str],
    input: &str,
    name: &str,
    value: &str,
) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(name, value)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("cargo-berth should start");
    child
        .stdin
        .take()
        .expect("cargo-berth stdin should exist")
        .write_all(input.as_bytes())
        .expect("cargo-berth stdin should write");
    child.wait_with_output().expect("cargo-berth should finish")
}

fn replace_managed_hook_executable_with_spy(repository_root: &Path) -> ManagedHookSpy {
    let spy_path = repository_root.join(".git/cargo-berth-hook-spy");
    let phase_log = repository_root.join(".git/cargo-berth-hook-spy-phases");
    let stdin_log = repository_root.join(".git/cargo-berth-hook-spy-stdin");
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$2\" >> {}\ncat >> {}\n",
        shell_single_quoted(&phase_log),
        shell_single_quoted(&stdin_log),
    );
    fs::write(&spy_path, script).expect("hook spy should write");
    let mut permissions = fs::metadata(&spy_path)
        .expect("hook spy metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&spy_path, permissions).expect("hook spy should be executable");

    let hook_path = repository_root.join(HOOK_PATH);
    let installed = fs::read_to_string(&hook_path).expect("managed hook should read");
    let executable = shell_single_quoted(Path::new(env!("CARGO_BIN_EXE_cargo-berth")));
    let spy_executable = shell_single_quoted(&spy_path);
    let instrumented = installed.replace(&executable, &spy_executable);
    assert_ne!(instrumented, installed);
    fs::write(hook_path, instrumented).expect("instrumented hook should write");

    ManagedHookSpy {
        phase_log,
        stdin_log,
    }
}

impl ManagedHookSpy {
    fn invoked_phases(&self) -> Vec<String> {
        fs::read_to_string(&self.phase_log)
            .expect("spy phase log should read")
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn prepared_trunk_update_hook_phases() -> Vec<String> {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "-b",
            "prepared-target",
        ],
    );
    let target = commit_work_without_hooks(
        repository.path(),
        "prepared-target.txt",
        "prepared target\n",
        "prepared target",
    );
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "main",
        ],
    );
    let spy = replace_managed_hook_executable_with_spy(repository.path());

    let updated = update_main(repository.path(), &base, &target, ReleaseValve::Unset);

    assert!(
        updated.status.success(),
        "prepared trunk update failed: {}",
        String::from_utf8_lossy(&updated.stderr)
    );
    spy.invoked_phases()
}

fn committed_feature_rebase_hook_phases() -> Vec<String> {
    let repository = initialized_repository();
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "-b",
            "rebase-source",
        ],
    );
    for commit_index in 0..3 {
        commit_work_without_hooks(
            repository.path(),
            &format!("rebase-{commit_index}.txt"),
            &format!("{commit_index}\n"),
            &format!("rebase source {commit_index}"),
        );
    }
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "main",
        ],
    );
    commit_work_without_hooks(
        repository.path(),
        "upstream-rebase.txt",
        "upstream\n",
        "upstream rebase base",
    );
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "rebase-source",
        ],
    );
    let spy = replace_managed_hook_executable_with_spy(repository.path());

    let rebased = git_output(repository.path(), &["rebase", "main"]);

    assert!(
        rebased.status.success(),
        "feature rebase failed: {}",
        String::from_utf8_lossy(&rebased.stderr)
    );
    spy.invoked_phases()
}

fn committed_forced_trunk_integration_hook_phases() -> Vec<String> {
    let repository = initialized_repository();
    let deferred_pair = deferred_pair(repository.path());
    commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn forced_work() {}\n",
        "forced integration work",
    );
    set_gate_mode(repository.path(), "enforce");
    let spy = replace_managed_hook_executable_with_spy(repository.path());

    let integrated = run_berth(
        &deferred_pair.blocked_root,
        &[
            "integrate",
            &deferred_pair.blocked_id,
            "--force",
            "--why",
            "exercise forced integration dispatch",
            "--json",
        ],
    );

    assert!(
        integrated.status.success(),
        "forced integration failed: {}",
        String::from_utf8_lossy(&integrated.stdout)
    );
    spy.invoked_phases()
}

fn commit_work_without_hooks(
    repository_root: &Path,
    path: &str,
    contents: &str,
    message: &str,
) -> String {
    fs::write(repository_root.join(path), contents).expect("work source should write");
    git(repository_root, &["add", path]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn run_three_commit_rebase_sample(
    repository_root: &Path,
    source_tip: &str,
    hook_mode: RebaseHookMode,
) -> Duration {
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "branch",
            "--force",
            "rebase-trial",
            source_tip,
        ],
    );
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "rebase-trial",
        ],
    );
    let started_at = Instant::now();
    let mut command = Command::new(GIT_BINARY);
    command.arg("--no-optional-locks");
    if matches!(hook_mode, RebaseHookMode::Disabled) {
        command.args(["-c", "core.hooksPath=/dev/null"]);
    }
    command
        .args(["rebase", "main"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT);
    if matches!(hook_mode, RebaseHookMode::FilteredBypass) {
        command.env(BYPASS_ENVIRONMENT, "1");
    }
    let rebased = command.output().expect("timed rebase should run");
    let elapsed = started_at.elapsed();
    assert!(
        rebased.status.success(),
        "timed rebase failed: {}",
        String::from_utf8_lossy(&rebased.stderr)
    );
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "checkout",
            "--quiet",
            "main",
        ],
    );
    elapsed
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session_and_run(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
    coordination_run_id: &str,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env(RUN_ENVIRONMENT, coordination_run_id)
        .env(SESSION_ENVIRONMENT, session_id)
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
