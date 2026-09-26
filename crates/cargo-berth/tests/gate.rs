#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end tests for installation, enforcement, release valves, and gate cost.

#[path = "support/integration_target.rs"]
mod integration_target;

#[path = "support/timing.rs"]
mod timing;

#[path = "support/split_rebase.rs"]
mod split_rebase;

use cargo_berth_test_support::EXECUTABLE_ENVIRONMENT;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::git_command;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: no optional locks, clearing what its fixtures set for themselves.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[BYPASS_ENVIRONMENT],
};

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tempfile::tempdir;
use timing::LOCK_CONTENTION_TOLERANCE;
use timing::LOCK_CONTENTION_TOLERANCE_ENVIRONMENT;
use timing::POLL_INTERVAL;
use timing::SCHEDULING_ALLOWANCE;

const BYPASS_ENVIRONMENT: &str = "CARGO_BERTH_BYPASS";
const BYPASSED_MERGE_IDENTITY_ENVIRONMENT: &str = "CARGO_BERTH_BYPASSED_MERGE_ID";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const MUTATION_LOCK_READY_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH";

const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GATE_DEADLINE: Duration = Duration::from_secs(1);
const GATE_DEADLINE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GATE_DEADLINE_MS";
const GIT_BINARY: &str = "git";
const HOOK_PATH: &str = ".git/hooks/reference-transaction";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const GATE_TARGETS_PATH: &str = ".git/cargo-berth/gate-targets";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const PENDING_BYPASS_PREFIX: &str = "cargo-berth-pending-bypass-";
/// Attribution and merge observations remain fixed for these two holder checkouts, including
/// the acting HEAD read and the merge-tree and diff that narrow the foreign holder to work HEAD
/// lacks. Every branch path read is those two invocations.
const POST_COMMIT_ENGINE_GIT_PROCESS_CEILING: usize = 16;
const RAW_GIT_BEHAVIOR_ENVIRONMENT: &str = "CARGO_BERTH_TEST_RAW_GIT_BEHAVIOR";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT: &str =
    "CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
/// The search path the wrapper uses for its own utilities.
///
/// [`RawGitBehavior::RemoveAfterTargetHistory`] narrows the traced process's `PATH` to the
/// wrapper directory alone, which is what makes git unavailable once the wrapper deletes
/// itself. That narrowing also reaches the wrapper's own `mkdir`, `rmdir`, `rm` and `sleep`,
/// so the wrapper restores this path for itself before running any of them. Resolving them
/// under `/bin` instead only works where `/bin` holds more than `sh`.
const UTILITY_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_UTILITY_PATH";
const RAW_TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh

separator=$(printf '\037')
record=git
for argument in "$@"; do
    record="${record}${separator}${argument}"
done
PATH="$CARGO_BERTH_TEST_UTILITY_PATH"
export PATH
lock_attempt=0
while ! mkdir "$CARGO_BERTH_TEST_GIT_TRACE.lock" 2>/dev/null; do
    lock_attempt=$((lock_attempt + 1))
    if [ "$lock_attempt" -ge 200 ]; then
        printf '%s\n' 'timed out acquiring raw git trace lock' >&2
        exit 24
    fi
    sleep 0.01
done
printf '%s\036' "$record" >> "$CARGO_BERTH_TEST_GIT_TRACE"
rmdir "$CARGO_BERTH_TEST_GIT_TRACE.lock"
if [ "${CARGO_BERTH_TEST_RAW_GIT_BEHAVIOR:-pass_through}" = "fail_phase_diff" ] \
    && [ "$1" = "--no-optional-locks" ] \
    && [ "$2" = "diff-tree" ] \
    && [ "$3" = "--stdin" ]; then
    printf '%s\n' 'injected completed git failure' >&2
    exit 23
fi
if [ "${CARGO_BERTH_TEST_RAW_GIT_BEHAVIOR:-pass_through}" = "fail_origin_classification" ] \
    && [ "$1" = "--no-optional-locks" ] \
    && [ "$2" = "rev-list" ] \
    && [ -n "${3:-}" ] \
    && [ -z "${4:-}" ]; then
    case "$3" in
        *..*)
            printf '%s\n' 'injected origin classification failure' >&2
            exit 23
            ;;
    esac
fi
if [ "${CARGO_BERTH_TEST_RAW_GIT_BEHAVIOR:-pass_through}" = "remove_after_target_history" ] \
    && [ "$1" = "--no-optional-locks" ] \
    && [ "$2" = "rev-list" ] \
    && [ "$3" = "--parents" ] \
    && [ -n "${4:-}" ] \
    && [ -z "${5:-}" ]; then
    "$CARGO_BERTH_TEST_REAL_GIT" "$@"
    status=$?
    rm -f "$0"
    exit "$status"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;

struct TargetPair {
    repository:         integration_target::IntegrationRepository,
    predecessor:        PathBuf,
    successor:          PathBuf,
    predecessor_id:     String,
    successor_id:       String,
    integration_before: String,
    predecessor_tip:    String,
    successor_tip:      String,
}

fn target_pair(merge_predecessor: bool) -> TargetPair {
    let repository = integration_target::IntegrationRepository::new();
    let predecessor = repository.lane("target-a", "integration");
    let successor = repository.lane("target-b", "integration");
    let integration_before =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed = integration_target::claim(
        &predecessor,
        "file:shared.txt",
        FIRST_RUN,
        Some("integration"),
    );
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred = integration_target::defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("integration"),
    );
    integration_target::assert_success(&deferred);
    let successor_id = integration_target::claim_id(&deferred);
    let sequenced = integration_target::run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A lands before B",
            "--json",
        ],
    );
    integration_target::assert_success(&sequenced);
    integration_target::commit_file(&predecessor, "shared.txt", "A\n", "A work");
    integration_target::commit_file(&successor, "shared.txt", "B\n", "B work");
    if merge_predecessor {
        integration_target::git(
            &successor,
            &["merge", "-s", "ours", "--no-edit", "target-a"],
        );
    }
    let predecessor_tip = integration_target::git_stdout(&predecessor, &["rev-parse", "HEAD"]);
    let successor_tip = integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]);
    TargetPair {
        repository,
        predecessor,
        successor,
        predecessor_id,
        successor_id,
        integration_before,
        predecessor_tip,
        successor_tip,
    }
}

fn gate_targets(root: &Path) -> Vec<String> {
    match fs::read_to_string(root.join(GATE_TARGETS_PATH)) {
        Ok(contents) => contents.lines().map(str::to_owned).collect(),
        Err(error) => {
            assert_eq!(
                error.kind(),
                std::io::ErrorKind::NotFound,
                "gate targets should read: {error}"
            );
            Vec::new()
        },
    }
}

struct MainPredecessorIntegrationSuccessor {
    repository:         integration_target::IntegrationRepository,
    successor:          PathBuf,
    predecessor_id:     String,
    successor_id:       String,
    main_before:        String,
    integration_before: String,
    predecessor_tip:    String,
    successor_tip:      String,
}

fn main_predecessor_integration_successor() -> MainPredecessorIntegrationSuccessor {
    let repository = integration_target::IntegrationRepository::new();
    let predecessor = repository.main_lane("main-predecessor");
    let successor = repository.lane("integration-successor", "integration");
    let main_before = integration_target::git_stdout(repository.root(), &["rev-parse", "main"]);
    let integration_before =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed = integration_target::claim(&predecessor, "file:shared.txt", FIRST_RUN, None);
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred = integration_target::defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("integration"),
    );
    integration_target::assert_success(&deferred);
    let successor_id = integration_target::claim_id(&deferred);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A before B",
            "--json",
        ],
    ));
    integration_target::commit_file(&predecessor, "shared.txt", "A\n", "A work");
    integration_target::assert_success(&integration_target::run(
        &predecessor,
        &["release", &predecessor_id, "--json"],
    ));
    integration_target::commit_file(&successor, "shared.txt", "B\n", "B work");
    let predecessor_tip = integration_target::git_stdout(&predecessor, &["rev-parse", "HEAD"]);
    let successor_tip = integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]);
    MainPredecessorIntegrationSuccessor {
        repository,
        successor,
        predecessor_id,
        successor_id,
        main_before,
        integration_before,
        predecessor_tip,
        successor_tip,
    }
}

#[cfg(unix)]
#[test]
fn listed_target_in_repository_path_with_backslash_is_gated() {
    let repository = integration_target::IntegrationRepository::with_repository_prefix("repo\\tx");
    let predecessor = repository.lane("slash-a", "integration");
    let successor = repository.lane("slash-b", "integration");
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed = integration_target::claim(
        &predecessor,
        "file:shared.txt",
        FIRST_RUN,
        Some("integration"),
    );
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred = integration_target::defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("integration"),
    );
    integration_target::assert_success(&deferred);
    let successor_id = integration_target::claim_id(&deferred);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A before B",
            "--json",
        ],
    ));
    integration_target::commit_file(&predecessor, "shared.txt", "A\n", "A work");
    integration_target::commit_file(&successor, "shared.txt", "B\n", "B work");
    set_gate_mode(repository.root(), "enforce");
    let before = integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    let after = integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]);
    let denied = propose_branch(repository.root(), "integration", &before, &after);
    assert!(
        !denied.status.success(),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    assert!(String::from_utf8_lossy(&denied.stderr).contains(&successor_id));
}

#[test]
fn trunk_gate_checks_successor_recorded_against_another_target() {
    let pair = main_predecessor_integration_successor();
    set_gate_mode(pair.repository.root(), "enforce");
    let denied = propose_trunk(
        pair.repository.root(),
        &pair.main_before,
        &pair.successor_tip,
    );
    let denial = String::from_utf8_lossy(&denied.stderr);
    assert!(!denied.status.success(), "{denial}");
    assert!(denial.contains(&pair.successor_id), "{denial}");
    let landed = propose_trunk(
        pair.repository.root(),
        &pair.main_before,
        &pair.predecessor_tip,
    );
    assert!(
        landed.status.success(),
        "{}",
        String::from_utf8_lossy(&landed.stderr)
    );
    integration_target::git(
        &pair.successor,
        &["merge", "-s", "ours", "--no-edit", "main"],
    );
    let incorporated = integration_target::git_stdout(&pair.successor, &["rev-parse", "HEAD"]);
    let allowed = propose_trunk(pair.repository.root(), &pair.predecessor_tip, &incorporated);
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
}

#[test]
fn cross_target_main_predecessor_recovery_prescribes_integrate() {
    let pair = main_predecessor_integration_successor();
    set_gate_mode(pair.repository.root(), "enforce");
    let denied = propose_branch(
        pair.repository.root(),
        "integration",
        &pair.integration_before,
        &pair.successor_tip,
    );
    let denial = String::from_utf8_lossy(&denied.stderr);
    assert!(!denied.status.success(), "{denial}");
    assert!(
        denial.contains(&format!(
            "run cargo-berth integrate {}",
            pair.predecessor_id
        )),
        "{denial}"
    );
    assert!(!denial.contains("land main on main"), "{denial}");
}

#[test]
fn renamed_trunk_refusal_names_its_branch() {
    let repository = integration_target::IntegrationRepository::new();
    integration_target::git(repository.root(), &["branch", "develop", "main"]);
    integration_target::set_trunk(repository.root(), "develop");
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &["init", "--json"],
    ));
    let predecessor = repository.lane("develop-a", "develop");
    let successor = repository.lane("develop-b", "develop");
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed =
        integration_target::claim(&predecessor, "file:shared.txt", FIRST_RUN, Some("develop"));
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred = integration_target::defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("develop"),
    );
    integration_target::assert_success(&deferred);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &integration_target::claim_id(&deferred),
            "--why",
            "A before B",
            "--json",
        ],
    ));
    integration_target::commit_file(&predecessor, "shared.txt", "A\n", "A work");
    integration_target::commit_file(&successor, "shared.txt", "B\n", "B work");
    set_gate_mode(repository.root(), "enforce");
    let before = integration_target::git_stdout(repository.root(), &["rev-parse", "develop"]);
    let after = integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]);
    let denied = propose_branch(repository.root(), "develop", &before, &after);
    assert!(
        !denied.status.success(),
        "{}",
        String::from_utf8_lossy(&denied.stderr)
    );
    assert!(String::from_utf8_lossy(&denied.stderr).contains("cannot enter develop"));
}

#[test]
fn reinitialization_clears_gate_targets() {
    let repository = integration_target::IntegrationRepository::new();
    let lane = repository.lane("reset-target", "integration");
    integration_target::write_file(&lane, "held.txt", "held\n");
    integration_target::assert_success(&integration_target::claim(
        &lane,
        "file:held.txt",
        FIRST_RUN,
        Some("integration"),
    ));
    assert_eq!(gate_targets(repository.root()), ["refs/heads/integration"]);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &["init", "--reinitialize-after-review", "--json"],
    ));
    assert!(gate_targets(repository.root()).is_empty());
}

#[test]
fn committed_hook_skips_unlisted_fast_forward_before_journal_replay() {
    let repository = integration_target::IntegrationRepository::new();
    let listed_lane = repository.lane("listed-source", "integration");
    integration_target::write_file(&listed_lane, "listed.txt", "listed\n");
    integration_target::assert_success(&integration_target::claim(
        &listed_lane,
        "file:listed.txt",
        FIRST_RUN,
        Some("integration"),
    ));
    let lane = repository.lane("ordinary-lane", "integration");
    let before = integration_target::git_stdout(&lane, &["rev-parse", "HEAD"]);
    integration_target::commit_file(&lane, "ordinary.txt", "ordinary\n", "ordinary work");
    let after = integration_target::git_stdout(&lane, &["rev-parse", "HEAD"]);
    let integration_before =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    integration_target::git(
        &repository.integration,
        &["commit", "--allow-empty", "--quiet", "-m", "listed work"],
    );
    let integration_after =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    let mut journal = OpenOptions::new()
        .append(true)
        .open(repository.root().join(JOURNAL_PATH))
        .expect("journal opens for corruption");
    journal.write_all(b"{}\n").expect("corrupt record writes");
    let ordinary = run_private_hook(
        repository.root(),
        "committed",
        &format!("{before} {after} refs/heads/ordinary-lane\n"),
    );
    assert!(
        ordinary.status.success(),
        "{}",
        String::from_utf8_lossy(&ordinary.stderr)
    );
    assert!(
        ordinary.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&ordinary.stderr)
    );
    let listed = run_private_hook(
        repository.root(),
        "committed",
        &format!("{integration_before} {integration_after} refs/heads/integration\n"),
    );
    assert!(
        !listed.stderr.is_empty(),
        "listed target should report journal corruption"
    );
    assert!(
        String::from_utf8_lossy(&listed.stderr).contains("journal"),
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
}

#[test]
fn gate_targets_tracks_live_recorded_targets_and_default_is_empty() {
    let default = initialized_repository();
    assert!(gate_targets(default.path()).is_empty());
    let repository = integration_target::IntegrationRepository::new();
    let lane = repository.lane("target-list-lane", "integration");
    integration_target::write_file(&lane, "listed.txt", "listed\n");
    let claimed =
        integration_target::claim(&lane, "file:listed.txt", FIRST_RUN, Some("integration"));
    integration_target::assert_success(&claimed);
    assert_eq!(gate_targets(repository.root()), ["refs/heads/integration"]);
    fs::write(
        repository.root().join(GATE_TARGETS_PATH),
        "refs/heads/stale\n",
    )
    .expect("stale filter writes");
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &["board", "--json"],
    ));
    assert_eq!(gate_targets(repository.root()), ["refs/heads/integration"]);
    let id = integration_target::claim_id(&claimed);
    integration_target::commit_file(&lane, "listed.txt", "listed\n", "listed work");
    integration_target::assert_success(&integration_target::run(
        &lane,
        &["release", &id, "--json"],
    ));
    repository.merge_fast_forward("target-list-lane");
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &["board", "--json"],
    ));
    assert!(gate_targets(repository.root()).is_empty());
}

#[test]
fn target_gate_rejects_successor_until_predecessor_lands_on_target() {
    let pair = target_pair(true);
    set_gate_mode(pair.repository.root(), "enforce");
    let denied = propose_branch(
        pair.repository.root(),
        "integration",
        &pair.integration_before,
        &pair.successor_tip,
    );
    assert!(
        !denied.status.success(),
        "successor entered integration before A"
    );
    let text = String::from_utf8_lossy(&denied.stderr);
    assert!(
        text.contains(&format!(
            "Reservation {} cannot enter integration while its integration order is held",
            pair.successor_id
        )),
        "{text}"
    );
    assert!(text.contains("integration"), "{text}");
    assert!(text.contains(&pair.predecessor_id), "{text}");
    assert!(text.contains(&pair.successor_id), "{text}");
    assert_eq!(
        integration_target::git_stdout(pair.repository.root(), &["rev-parse", "integration"]),
        pair.integration_before
    );
    integration_target::assert_success(&integration_target::run(
        &pair.predecessor,
        &["release", &pair.predecessor_id, "--json"],
    ));
    pair.repository.merge_fast_forward("target-a");
    let allowed = propose_branch(
        pair.repository.root(),
        "integration",
        &pair.predecessor_tip,
        &pair.successor_tip,
    );
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
}

#[test]
fn listed_target_recreation_is_gated_at_its_proposed_tip() {
    let pair = target_pair(false);
    pair.repository.remove_integration_branch();
    assert_eq!(
        gate_targets(pair.repository.root()),
        ["refs/heads/integration"]
    );
    set_gate_mode(pair.repository.root(), "enforce");
    let absent = "0000000000000000000000000000000000000000";
    let denied = propose_branch(
        pair.repository.root(),
        "integration",
        absent,
        &pair.successor_tip,
    );
    assert!(
        !denied.status.success(),
        "recreated target entered B before A"
    );
    let text = String::from_utf8_lossy(&denied.stderr);
    assert!(text.contains(&pair.predecessor_id), "{text}");
    assert!(text.contains(&pair.successor_id), "{text}");
    let admitted = propose_branch(
        pair.repository.root(),
        "integration",
        absent,
        &pair.predecessor_tip,
    );
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
}

#[test]
fn target_gate_observe_and_enforce_messages_name_the_target() {
    for mode in ["observe", "enforce"] {
        let pair = target_pair(true);
        set_gate_mode(pair.repository.root(), mode);
        let result = propose_branch(
            pair.repository.root(),
            "integration",
            &pair.integration_before,
            &pair.successor_tip,
        );
        let text = String::from_utf8_lossy(&result.stderr);
        assert!(text.contains("integration"), "{text}");
        assert!(
            text.contains(&format!(
                "Reservation {} cannot enter integration while its integration order is held",
                pair.successor_id
            )),
            "{text}"
        );
        if mode == "observe" {
            assert_eq!(result.status.code(), Some(0), "{text}");
            assert!(
                text.contains("Observe-only cargo-berth integration gate:"),
                "{text}"
            );
        } else {
            assert!(!result.status.success(), "{text}");
            let input = format!(
                "{} {} refs/heads/integration\n",
                pair.integration_before, pair.successor_tip
            );
            let hook = run_installed_prepared_hook(pair.repository.root(), &input);
            assert_eq!(hook.status.code(), Some(2));
        }
    }
}

#[test]
fn unlisted_prepared_branch_update_does_not_start_cargo_berth() {
    let repository = integration_target::IntegrationRepository::new();
    let lane = repository.lane("filter-lane", "integration");
    let claimed =
        integration_target::claim(&lane, "file:filter.txt", FIRST_RUN, Some("integration"));
    integration_target::assert_success(&claimed);
    assert_eq!(gate_targets(repository.root()), ["refs/heads/integration"]);
    integration_target::commit_file(&lane, "filter.txt", "filter\n", "filter work");
    let before = integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    let after = integration_target::git_stdout(&lane, &["rev-parse", "HEAD"]);

    let shim_directory = tempdir().expect("shim directory");
    let marker = shim_directory.path().join("invoked");
    let shim = shim_directory.path().join("cargo-berth");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\nprintf '%s\\n' invoked >> {}\nexit 9\n",
            shell_single_quoted(&marker)
        ),
    )
    .expect("recording shim writes");
    let mut permissions = fs::metadata(&shim).expect("shim metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shim, permissions).expect("shim executes");
    let mut search_path = vec![shim_directory.path().to_path_buf()];
    search_path.extend(std::env::split_paths(
        &std::env::var_os("PATH").expect("PATH exists"),
    ));
    let search_path = std::env::join_paths(search_path).expect("PATH joins");
    let mut child = Command::new(repository.root().join(HOOK_PATH))
        .arg("prepared")
        .current_dir(repository.root())
        .env_remove(EXECUTABLE_ENVIRONMENT)
        .env("PATH", &search_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("prepared hook starts");
    child
        .stdin
        .take()
        .expect("prepared stdin")
        .write_all(format!("{before} {after} refs/heads/unlisted\n").as_bytes())
        .expect("prepared input writes");
    let updated = child.wait_with_output().expect("prepared hook finishes");
    assert!(
        updated.status.success(),
        "{}",
        String::from_utf8_lossy(&updated.stderr)
    );
    assert!(
        !marker.exists(),
        "unlisted prepared update started cargo-berth"
    );

    let mut child = Command::new(repository.root().join(HOOK_PATH))
        .arg("prepared")
        .current_dir(repository.root())
        .env_remove(EXECUTABLE_ENVIRONMENT)
        .env("PATH", &search_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("listed prepared hook starts");
    child
        .stdin
        .take()
        .expect("listed prepared stdin")
        .write_all(format!("{before} {after} refs/heads/integration\n").as_bytes())
        .expect("listed prepared input writes");
    let _listed = child
        .wait_with_output()
        .expect("listed prepared hook finishes");
    assert!(
        marker.exists(),
        "listed control did not start recording shim"
    );
}

#[test]
fn two_gated_refs_in_one_transaction_refuse_before_journal_changes() {
    for integration_first in [false, true] {
        let pair = target_pair(true);
        set_gate_mode(pair.repository.root(), "enforce");
        let main = integration_target::git_stdout(pair.repository.root(), &["rev-parse", "main"]);
        let entries = [
            format!("update refs/heads/main {} {main}\n", pair.predecessor_tip),
            format!(
                "update refs/heads/integration {} {}\n",
                pair.successor_tip, pair.integration_before
            ),
        ];
        let order = if integration_first { [1, 0] } else { [0, 1] };
        let input = format!(
            "start\n{}{}prepare\ncommit\n",
            entries[order[0]], entries[order[1]]
        );
        let before = fs::read(pair.repository.root().join(JOURNAL_PATH))
            .expect("journal before transaction");
        let mut child = git_command(BERTH_EXECUTABLE)
            .arg("update-ref")
            .arg("--stdin")
            .current_dir(pair.repository.root())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("transaction starts");
        child
            .stdin
            .take()
            .expect("transaction stdin")
            .write_all(input.as_bytes())
            .expect("transaction writes");
        let result = child.wait_with_output().expect("transaction finishes");
        assert!(!result.status.success(), "transaction accepted both refs");
        let text = String::from_utf8_lossy(&result.stderr);
        assert!(
            text.contains("one transaction would move multiple gated refs"),
            "{text}"
        );
        assert!(text.contains("refs/heads/main"), "{text}");
        assert!(text.contains("refs/heads/integration"), "{text}");
        assert!(text.contains("move them one at a time"), "{text}");
        let hook_input = format!(
            "{main} {} refs/heads/main\n{} {} refs/heads/integration\n",
            pair.predecessor_tip, pair.integration_before, pair.successor_tip
        );
        let hook = run_installed_prepared_hook(pair.repository.root(), &hook_input);
        assert_eq!(hook.status.code(), Some(5));
        assert_eq!(
            fs::read(pair.repository.root().join(JOURNAL_PATH)).expect("journal after transaction"),
            before
        );
    }
}

#[test]
fn integrate_moves_the_present_target_and_missing_target_falls_back_to_main() {
    for remove_target in [false, true] {
        let repository = integration_target::IntegrationRepository::new();
        let lane = repository.lane(
            if remove_target {
                "missing-lane"
            } else {
                "present-lane"
            },
            "integration",
        );
        integration_target::write_file(&lane, "work.txt", "work\n");
        let claimed =
            integration_target::claim(&lane, "file:work.txt", FIRST_RUN, Some("integration"));
        integration_target::assert_success(&claimed);
        let id = integration_target::claim_id(&claimed);
        integration_target::commit_file(&lane, "work.txt", "work\n", "lane work");
        let tip = integration_target::git_stdout(&lane, &["rev-parse", "HEAD"]);
        let main_before = integration_target::git_stdout(repository.root(), &["rev-parse", "main"]);
        let integration_before =
            integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
        if remove_target {
            repository.remove_integration_branch();
        }
        let integrated = integration_target::run(&lane, &["integrate", &id, "--json"]);
        integration_target::assert_success(&integrated);
        let moved = if remove_target { "main" } else { "integration" };
        let envelope = integration_target::json(&integrated);
        let data = &envelope["payload"]["data"];
        assert_eq!(data["status"], "integrated");
        assert_eq!(data["target"], format!("refs/heads/{moved}"));
        let previous = if remove_target {
            main_before.as_str()
        } else {
            integration_before.as_str()
        };
        assert_eq!(data["previous"], previous);
        assert_eq!(data["proposed"], tip);
        assert_eq!(
            integration_target::git_stdout(repository.root(), &["rev-parse", moved]),
            tip
        );
        if !remove_target {
            assert_eq!(
                integration_target::git_stdout(repository.root(), &["rev-parse", "main"]),
                main_before
            );
        }
    }
}

#[test]
fn blocked_integrate_json_names_the_target() {
    let pair = target_pair(false);
    set_gate_mode(pair.repository.root(), "enforce");
    set_gate_mode(&pair.successor, "enforce");
    let blocked = integration_target::run(
        &pair.successor,
        &["integrate", &pair.successor_id, "--json"],
    );
    assert_eq!(
        blocked.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&blocked.stdout)
    );
    let envelope = integration_target::json(&blocked);
    let data = &envelope["payload"]["data"];
    assert_eq!(data["status"], "blocked");
    assert_eq!(data["target"], "refs/heads/integration");
}

#[test]
fn integrate_blocks_when_successor_tip_contains_held_predecessor() {
    let pair = target_pair(true);
    set_gate_mode(pair.repository.root(), "enforce");
    set_gate_mode(&pair.successor, "enforce");

    let blocked = integration_target::run(
        &pair.successor,
        &["integrate", &pair.successor_id, "--json"],
    );
    assert_eq!(
        blocked.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&blocked.stdout),
        String::from_utf8_lossy(&blocked.stderr)
    );
}

#[test]
fn held_integrate_is_blocked_even_when_target_tip_is_unchanged() {
    let repository = integration_target::IntegrationRepository::new();
    let predecessor = repository.lane("noop-a", "integration");
    let successor = repository.lane("noop-b", "integration");
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed = integration_target::claim(
        &predecessor,
        "file:shared.txt",
        FIRST_RUN,
        Some("integration"),
    );
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred = integration_target::defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("integration"),
    );
    integration_target::assert_success(&deferred);
    let successor_id = integration_target::claim_id(&deferred);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A lands before B",
            "--json",
        ],
    ));
    let target_tip =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    assert_eq!(
        integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]),
        target_tip
    );
    set_gate_mode(repository.root(), "enforce");
    set_gate_mode(&successor, "enforce");

    let blocked = integration_target::run(&successor, &["integrate", &successor_id, "--json"]);
    assert_eq!(
        blocked.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&blocked.stdout)
    );
    let envelope = integration_target::json(&blocked);
    assert_eq!(envelope["payload"]["data"]["status"], "blocked");
    assert_eq!(
        envelope["payload"]["data"]["target"],
        "refs/heads/integration"
    );
    assert_eq!(
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]),
        target_tip
    );
}

#[test]
fn trunk_gate_remembers_first_target_landing_after_target_revalidation() {
    let repository = integration_target::IntegrationRepository::new();
    let predecessor = repository.lane("proof-a", "integration");
    let successor = repository.main_lane("proof-x");
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed = integration_target::claim(
        &predecessor,
        "file:shared.txt",
        FIRST_RUN,
        Some("integration"),
    );
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred = integration_target::defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("main"),
    );
    integration_target::assert_success(&deferred);
    let successor_id = integration_target::claim_id(&deferred);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A reaches main before X",
            "--json",
        ],
    ));
    integration_target::commit_file(&predecessor, "shared.txt", "A\n", "first A landing");
    integration_target::assert_success(&integration_target::run(
        &predecessor,
        &["release", &predecessor_id, "--json"],
    ));
    repository.merge_fast_forward("proof-a");
    let first_landing =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    integration_target::git(repository.root(), &["merge", "--ff-only", "integration"]);
    assert_eq!(
        integration_target::git_stdout(repository.root(), &["rev-parse", "main"]),
        first_landing
    );

    integration_target::commit_file(
        &repository.integration,
        "integration-later.txt",
        "later\n",
        "I revalidates A",
    );
    let second_landing =
        integration_target::git_stdout(repository.root(), &["rev-parse", "integration"]);
    assert_ne!(first_landing, second_landing);
    integration_target::assert_success(&integration_target::run(
        repository.root(),
        &["board", "--json"],
    ));
    assert!(
        integration_target::journal(repository.root())
            .iter()
            .any(|event| {
                event["op"] == "evidence_revalidated"
                    && event["reservation_id"] == predecessor_id
                    && event["status"]["trunk_oid"] == second_landing
            }),
        "A must revalidate at I2 after I1 reaches main"
    );

    integration_target::commit_file(&successor, "shared.txt", "X\n", "X work");
    integration_target::git(&successor, &["merge", "-s", "ours", "--no-edit", "main"]);
    let successor_tip = integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]);
    assert!(!GIT.succeeds(
        repository.root(),
        &[
            "merge-base",
            "--is-ancestor",
            &second_landing,
            &successor_tip
        ]
    ));
    set_gate_mode(repository.root(), "enforce");
    let admitted = propose_trunk(repository.root(), &first_landing, &successor_tip);
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
}

#[test]
fn enforce_gate_rejects_cross_target_successor_before_integration_lands_on_main() {
    let repo = integration_target::IntegrationRepository::new();
    let predecessor = repo.lane("gate-a", "integration");
    let successor = repo.main_lane("gate-x");
    integration_target::write_file(&predecessor, "shared.txt", "A\n");
    let claimed = integration_target::claim(&predecessor, "file:shared.txt", FIRST_RUN, None);
    integration_target::assert_success(&claimed);
    let predecessor_id = integration_target::claim_id(&claimed);
    let deferred =
        integration_target::defer_claim(&successor, "file:shared.txt", SECOND_RUN, &predecessor_id);
    integration_target::assert_success(&deferred);
    let successor_id = integration_target::claim_id(&deferred);
    integration_target::commit_file(&predecessor, "shared.txt", "A\n", "A work");
    integration_target::assert_success(&integration_target::run(
        &predecessor,
        &["release", &predecessor_id, "--json"],
    ));
    repo.merge_by_commit("gate-a");
    let sequenced = integration_target::run(
        repo.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A lands before X",
            "--json",
        ],
    );
    integration_target::assert_success(&sequenced);
    assert_eq!(
        integration_target::json(&sequenced)["payload"]["data"]["readiness"]["hold"]["reason"],
        "predecessor_not_on_trunk"
    );

    integration_target::commit_file(&successor, "shared.txt", "X\n", "X work");
    set_gate_mode(repo.root(), "enforce");
    let main = integration_target::git_stdout(repo.root(), &["rev-parse", "main"]);
    let successor_tip = integration_target::git_stdout(&successor, &["rev-parse", "HEAD"]);
    let rejected = propose_trunk(repo.root(), &main, &successor_tip);
    assert!(
        !rejected.status.success(),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    let denial = String::from_utf8_lossy(&rejected.stderr);
    assert!(denial.contains("Ordering edge"), "{denial}");
    assert!(denial.contains(&predecessor_id), "{denial}");
    assert!(denial.contains(&successor_id), "{denial}");
    assert!(
        denial.contains("integration") && denial.contains("main"),
        "{denial}"
    );
    assert!(
        denial.contains("land integration on main before integrating this reservation"),
        "{denial}"
    );
    assert!(
        !denial.contains(&format!("integrate {predecessor_id}")),
        "{denial}"
    );
    assert_eq!(
        integration_target::git_stdout(repo.root(), &["rev-parse", "main"]),
        main
    );
    assert_ne!(
        integration_target::git_stdout(repo.root(), &["rev-parse", "integration"]),
        main
    );
}

#[test]
fn enrollment_authorizes_both_edits_but_holds_integration_under_each_gate_policy() {
    for (mode, acting_index) in [
        ("observe", 0),
        ("observe", 1),
        ("enforce", 0),
        ("enforce", 1),
    ] {
        let repository = scratch_repository();
        let root = repository.path();
        let base = git_stdout(root, &["rev-parse", "main"]);
        let worktrees = tempdir().expect("worktree parent should exist");
        let first = add_worktree(root, worktrees.path(), "enrolled-first");
        let second = add_worktree(root, worktrees.path(), "enrolled-second");
        for (checkout, contents) in [
            (&first, "pub fn first_committed() {}\n"),
            (&second, "pub fn second_committed() {}\n"),
        ] {
            commit_work_without_hooks(checkout, "src/lib.rs", contents, "work before enrollment");
            fs::write(
                checkout.join("src/lib.rs"),
                format!("{contents}// dirty before init\n"),
            )
            .expect("dirty shared work should write");
        }
        let initialized = run_berth(root, &["init", "--json"]);
        let initialized_envelope = json_output(&initialized);
        assert!(initialized.status.success(), "{initialized_envelope}");
        let enrollment = &initialized_envelope["payload"]["data"]["enrollment"];
        assert_eq!(
            enrollment["enrolled"]
                .as_array()
                .expect("enrolled rows")
                .len(),
            2
        );
        assert_eq!(
            enrollment["overlaps"]
                .as_array()
                .expect("overlap rows")
                .len(),
            1
        );
        set_gate_mode(root, mode);
        let mut reservation_ids = Vec::new();
        for (checkout, session) in [
            (&first, "enrolled-first-session"),
            (&second, "enrolled-second-session"),
        ] {
            let checked =
                run_berth_with_session(checkout, &["check", "file:src/lib.rs", "--json"], session);
            let envelope = json_output(&checked);
            assert!(checked.status.success(), "{envelope}");
            assert_eq!(envelope["status"], "clear");
            assert_eq!(
                envelope["payload"]["data"]["acquisition"]["kind"],
                "already_held"
            );
            reservation_ids.push(
                envelope["payload"]["data"]["acquisition"]["reservation_id"]
                    .as_str()
                    .expect("reused enrolled reservation")
                    .to_owned(),
            );
            commit_work(
                checkout,
                "src/lib.rs",
                &format!("// edited by {session}\n"),
                "authorized shared edit",
            );
            let drift = run_berth_with_session(checkout, &["drift", "--full", "--json"], session);
            assert!(drift.status.success(), "{}", json_output(&drift));
        }
        let board = run_berth(root, &["board", "--json"]);
        let board_envelope = json_output(&board);
        assert!(board.status.success(), "{board_envelope}");
        assert_eq!(
            board_envelope["payload"]["data"]["outstanding_incursions"]["entries"],
            serde_json::json!([])
        );
        assert_eq!(
            board_envelope["payload"]["data"]["unresolved_overlaps"]["entries"][0]["origin"],
            "enrollment"
        );

        let acting_root = [&first, &second][acting_index];
        let integrated = run_berth(
            acting_root,
            &["integrate", &reservation_ids[acting_index], "--json"],
        );
        assert_enrollment_gate_decision(
            root,
            mode,
            &base,
            &integrated,
            &reservation_ids[1 - acting_index],
        );
    }
}

#[test]
fn uninterrupted_apply_rebase_reanchors_active_reservation_above_unrelated_trunk_work() {
    let repository = initialized_repository();
    let root = repository.path();
    let phase_start = git_stdout(root, &["rev-parse", "HEAD"]);
    let worktrees = tempdir().expect("linked checkout parent should exist");
    let holder = add_worktree(root, worktrees.path(), "holder");
    assert!(root.join(HOOK_PATH).is_file());
    let claimed = claim(
        &holder,
        "file:src/lib.rs",
        FIRST_RUN,
        "docs/apply-rebase-plan.md",
        "active-apply-rebase",
    );
    assert!(claimed.status.success(), "{}", json_output(&claimed));
    let id = reservation_id(&claimed);
    let original_tip = commit_work(
        &holder,
        "src/lib.rs",
        "pub fn holder_work() {}\n",
        "active holder work",
    );
    let trunk = commit_work(root, "unrelated.txt", "upstream\n", "unrelated trunk work");
    assert_ne!(trunk, phase_start);

    git(&holder, &["rebase", "--apply", "main"]);
    let rewritten_tip = git_stdout(&holder, &["rev-parse", "HEAD"]);
    assert_ne!(rewritten_tip, original_tip);
    assert_ne!(rewritten_tip, trunk);

    // Read before any berth command can reconcile: the managed hook must already
    // anchor this still-active phase after the unrelated upstream commit.
    let events = journal_text(root)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("journal should decode"))
        .collect::<Vec<_>>();
    let reanchor = events
        .iter()
        .rev()
        .find(|event| event["op"] == "resnapshot" && event["reservation_id"] == id)
        .expect("uninterrupted apply rebase should reanchor the active reservation in its hook");
    assert_eq!(reanchor["snapshot"]["stage"], "active", "{reanchor}");
    assert_eq!(reanchor["snapshot"]["claim_snapshot"], trunk, "{reanchor}");
    assert!(!events.iter().any(|event| {
        event["reservation_id"] == id
            && matches!(event["op"].as_str(), Some("checkpoint" | "release"))
    }));

    let checkpointed = run_berth(&holder, &["release", &id, "--json"]);
    assert!(
        checkpointed.status.success(),
        "{}",
        json_output(&checkpointed)
    );
    assert_eq!(json_output(&checkpointed)["status"], "outstanding");
    let checkpoint = journal_text(root)
        .lines()
        .rev()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("journal should decode"))
        .find(|event| event["op"] == "checkpoint" && event["reservation_id"] == id)
        .expect("rebased active work should have an outstanding checkpoint");
    assert_eq!(checkpoint["protected_tip"], rewritten_tip, "{checkpoint}");
    let protected_range = git_stdout(
        root,
        &[
            "rev-list",
            "--reverse",
            &format!("{trunk}..{rewritten_tip}"),
        ],
    );
    assert_eq!(protected_range, rewritten_tip);
    assert_eq!(
        git_stdout(
            root,
            &["rev-parse", &format!("refs/cargo-berth/reservations/{id}")]
        ),
        rewritten_tip
    );
}

#[test]
fn active_lower_reservation_keeps_both_split_commits_after_update_refs() {
    let repository = initialized_repository();
    let root = repository.path();
    let base_contents = "base\na\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nbase\n";
    let first_contents = base_contents.replacen("base", "one", 1);
    let full_contents = first_contents.replace("base", "two");
    let trunk = commit_work(root, "phase.txt", base_contents, "phase base");
    let worktrees = tempdir().expect("linked checkout parent should exist");
    let holder = add_worktree(root, worktrees.path(), "lower");
    let claimed = claim(
        &holder,
        "file:phase.txt",
        FIRST_RUN,
        "docs/split-plan.md",
        "lower",
    );
    assert!(claimed.status.success(), "{}", json_output(&claimed));
    let id = reservation_id(&claimed);
    let old_lower = commit_work(&holder, "phase.txt", &full_contents, "both lower hunks");
    git(&holder, &["switch", "--quiet", "-c", "upper"]);
    let old_upper = commit_work(&holder, "upper.txt", "upper\n", "stacked upper work");

    let editor = worktrees.path().join("split-editor");
    split_rebase::stop_for_split(&holder, &editor, &trunk);
    let first_split = commit_work(&holder, "phase.txt", &first_contents, "first split hunk");
    let second_split = commit_work(&holder, "phase.txt", &full_contents, "second split hunk");
    split_rebase::continue_rebase(&holder);
    assert_ne!(git_stdout(&holder, &["rev-parse", "upper"]), old_upper);
    assert_eq!(git_stdout(&holder, &["rev-parse", "lower"]), second_split);
    assert_ne!(second_split, old_lower);

    // Git updates checked-out upper first, then lower in a separate transaction.
    // Lower's active phase must still include the split hidden by upper's new ref.
    assert_lower_phase_start(root, &id, &trunk, &second_split);

    git(&holder, &["switch", "--quiet", "lower"]);
    let checkpoint = run_berth(&holder, &["release", &id, "--json"]);
    assert!(checkpoint.status.success(), "{}", json_output(&checkpoint));
    assert_eq!(json_output(&checkpoint)["status"], "outstanding");
    assert_split_checkpoint_interval(root, &id, &trunk, &first_split, &second_split);
}

/// Reconstruct the phase start retained by claim or its latest active reanchor.
fn assert_lower_phase_start(root: &Path, id: &str, trunk: &str, second_split: &str) {
    let events = journal_text(root)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("journal should decode"))
        .collect::<Vec<_>>();
    let latest = events
        .iter()
        .rev()
        .find(|event| {
            event["reservation_id"] == id
                && matches!(event["op"].as_str(), Some("claim" | "resnapshot"))
        })
        .expect("active reservation should retain its anchor");
    let anchor = if latest["op"] == "claim" {
        &latest["phase_start_head"]
    } else {
        assert_eq!(latest["snapshot"]["stage"], "active", "{latest}");
        &latest["snapshot"]["claim_snapshot"]
    };
    assert_eq!(
        anchor, trunk,
        "both split commits must remain protected: {latest}"
    );
    assert_ne!(anchor, second_split);
}

/// The checkpoint must cover the whole split and retain its final commit.
fn assert_split_checkpoint_interval(
    root: &Path,
    id: &str,
    trunk: &str,
    first_split: &str,
    second_split: &str,
) {
    let events = journal_text(root)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("journal should decode"))
        .collect::<Vec<_>>();
    let checkpoint = events
        .iter()
        .rev()
        .find(|event| event["op"] == "checkpoint" && event["reservation_id"] == id)
        .expect("lower should have an outstanding checkpoint");
    assert_lower_phase_start(root, id, trunk, second_split);
    assert_eq!(checkpoint["protected_tip"], second_split, "{checkpoint}");
    let protected_range = git_stdout(
        root,
        &["rev-list", "--reverse", &format!("{trunk}..{second_split}")],
    );
    assert_eq!(
        protected_range.lines().collect::<Vec<_>>(),
        [first_split, second_split]
    );
    assert_eq!(
        git_stdout(
            root,
            &["rev-parse", &format!("refs/cargo-berth/reservations/{id}")]
        ),
        second_split
    );
}

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
    let installed_text = String::from_utf8_lossy(&installed);
    let issuing_directory_assignment = installed_text
        .find("CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY=$PWD")
        .expect("managed hook should capture its issuing directory");
    let policy_worktree_change = installed_text
        .find("if [ -d ")
        .expect("managed hook should retain its policy worktree change");
    assert!(issuing_directory_assignment < policy_worktree_change);
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
    assert_eq!(stale.status.code(), Some(5));
    let stale_json = json_output(&stale);
    assert_eq!(stale_json["payload"]["kind"], "coordination_identity");
    assert_eq!(
        stale_json["payload"]["data"]["kind"],
        "stale_session_mapping"
    );
    assert_eq!(
        stale_json["payload"]["data"]["recovery_actions"][0]["kind"],
        "clear_session_mapping"
    );
}

#[test]
fn session_mapping_selects_its_reservation_while_marker_only_check_is_ambiguous() {
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
    let first_reservation_id = reservation_id(&first);
    let second = claim(
        repository.path(),
        "file:second.txt",
        FIRST_RUN,
        "docs/session.md",
        "second-phase",
    );
    assert!(second.status.success());
    let second_reservation_id = reservation_id(&second);

    let session_check = run_berth_with_session(
        repository.path(),
        &["check", "file:second.txt", "--json"],
        session_id,
    );
    let marker_check = run_berth(repository.path(), &["check", "file:second.txt", "--json"]);
    let session_envelope = json_output(&session_check);
    let marker_envelope = json_output(&marker_check);
    let mut expected_candidate_ids = vec![first_reservation_id.clone(), second_reservation_id];
    expected_candidate_ids.sort();

    assert!(session_check.status.success());
    assert_eq!(session_envelope["status"], "clear");
    assert_eq!(
        session_envelope["payload"]["data"]["acquisition"]["kind"],
        "widened"
    );
    assert_eq!(
        session_envelope["payload"]["data"]["acquisition"]["reservation_id"],
        first_reservation_id
    );
    assert_eq!(marker_check.status.code(), Some(1));
    assert_eq!(
        marker_envelope["status"],
        "ambiguous_active_run_reservations"
    );
    assert_eq!(
        marker_envelope["payload"]["data"]["candidate_reservation_ids"],
        serde_json::json!(expected_candidate_ids)
    );
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
fn integrate_rejects_every_invalid_coordination_identity() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let integration_root =
        add_worktree(repository.path(), worktrees.path(), "identity-integration");
    let session_id = "integration-session";
    let mapped_claim = run_berth_with_session(
        repository.path(),
        &["claim", "file:mapped-holder", "--run", FIRST_RUN, "--json"],
        session_id,
    );
    assert!(mapped_claim.status.success());
    let mapped_id = reservation_id(&mapped_claim);
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    let integrating_claim = claim(
        &integration_root,
        "file:integrating",
        SECOND_RUN,
        "docs/integrating.md",
        "integration",
    );
    assert!(integrating_claim.status.success());
    let integrating_id = reservation_id(&integrating_claim);
    commit_work(
        &integration_root,
        "integrating",
        "integration work\n",
        "integration work",
    );

    let rejected = run_berth_with_session(
        &integration_root,
        &["integrate", &integrating_id, "--json"],
        session_id,
    );
    assert_eq!(rejected.status.code(), Some(5));
    assert_integration_identity_rejection(
        &json_output(&rejected),
        "session_worktree_mismatch",
        &["rerun_from_holding_worktree", "claim_separately_here"],
    );

    assert!(
        run_berth(repository.path(), &["release", &mapped_id, "--json"])
            .status
            .success()
    );
    fs::write(&mapping_path, stale_mapping).expect("stale session mapping should write");
    let rejected = run_berth_with_session(
        &integration_root,
        &["integrate", &integrating_id, "--json"],
        session_id,
    );
    let rejected_json = json_output(&rejected);
    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(rejected_json["status"], "invalid_input");
    assert_integration_identity_rejection(
        &rejected_json,
        "stale_session_mapping",
        &["clear_session_mapping"],
    );
    let diagnostic = rejected_json["message"]
        .as_str()
        .expect("rejection should have a message");
    assert!(diagnostic.contains("Harness session mapping"));
    assert!(!diagnostic.contains("coordination-run marker"));

    let administrative_root = git_stdout(&integration_root, &["rev-parse", "--absolute-git-dir"]);
    fs::write(
        Path::new(&administrative_root).join("cargo-berth-run-id"),
        format!("{FIRST_RUN}\n"),
    )
    .expect("stale marker should write");
    let rejected = run_berth(&integration_root, &["integrate", &integrating_id, "--json"]);
    assert_eq!(rejected.status.code(), Some(5));
    assert_integration_identity_rejection(
        &json_output(&rejected),
        "stale_marker_run",
        &["reconcile_and_sweep_marker"],
    );
}

#[test]
fn reference_transaction_gate_rejects_every_invalid_coordination_identity() {
    let repository = initialized_repository();
    assert_reference_transaction_rejects_stale_marker(&repository);
    fs::remove_file(repository.path().join(MARKER_PATH)).expect("stale marker should remove");
    assert_reference_transaction_rejects_stale_session_mapping(&repository);
    assert!(!repository.path().join(MARKER_PATH).exists());
    assert_reference_transaction_rejects_session_worktree_mismatch(&repository);
}

#[test]
fn managed_gate_accepts_a_linked_worktree_session_owned_by_that_worktree() {
    let repository = initialized_repository();
    let previous = git_stdout(repository.path(), &["rev-parse", "refs/heads/main"]);
    let proposed = commit_work(
        repository.path(),
        "tests/linked-session.rs",
        "// linked session gate\n",
        "linked session gate",
    );
    let worktrees = tempdir().expect("worktree parent should exist");
    let linked_root = add_worktree(repository.path(), worktrees.path(), "linked-session-gate");
    let session_id = "linked-reference-transaction-session";
    let linked_claim = run_berth_with_session(
        &linked_root,
        &[
            "claim",
            "file:linked-session-holder",
            "--run",
            FIRST_RUN,
            "--why",
            "hold the linked checkout session",
            "--json",
        ],
        session_id,
    );
    assert!(linked_claim.status.success());
    let input = format!("{previous} {proposed} refs/heads/main\n");

    let permitted = run_managed_hook_with_session(
        repository.path(),
        &linked_root,
        "prepared",
        &input,
        session_id,
    );

    assert!(
        permitted.status.success(),
        "linked-worktree gate rejected its owning session: {}",
        String::from_utf8_lossy(&permitted.stderr)
    );
}

#[test]
fn a_managed_hook_that_does_not_report_its_issuing_directory_requires_reinitialization() {
    let repository = initialized_repository();
    let previous = git_stdout(repository.path(), &["rev-parse", "refs/heads/main"]);
    let proposed = commit_work(
        repository.path(),
        "tests/uncaptured-hook.rs",
        "// uncaptured hook gate\n",
        "uncaptured hook gate",
    );
    let worktrees = tempdir().expect("worktree parent should exist");
    let linked_root = add_worktree(repository.path(), worktrees.path(), "uncaptured-hook-gate");
    let session_id = "uncaptured-reference-transaction-session";
    let linked_claim = run_berth_with_session(
        &linked_root,
        &[
            "claim",
            "file:uncaptured-hook-holder",
            "--run",
            FIRST_RUN,
            "--why",
            "hold the uncaptured hook issuing session",
            "--json",
        ],
        session_id,
    );
    assert!(linked_claim.status.success());
    let hook_path = repository.path().join(HOOK_PATH);
    let managed_hook = fs::read_to_string(&hook_path).expect("managed hook should read");
    let issuing_directory_capture = format!(
        "{REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT}=$PWD\nexport {REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT}\n"
    );
    let uncaptured_hook = managed_hook.replace(&issuing_directory_capture, "");
    assert_ne!(uncaptured_hook, managed_hook);
    fs::write(&hook_path, uncaptured_hook).expect("uncaptured hook fixture should write");
    let input = format!("{previous} {proposed} refs/heads/main\n");

    let rejected = run_managed_hook_with_session(
        repository.path(),
        &linked_root,
        "prepared",
        &input,
        session_id,
    );
    let diagnostic = String::from_utf8_lossy(&rejected.stderr);

    assert_eq!(rejected.status.code(), Some(5));
    assert!(diagnostic.contains("cargo-berth init"));
    assert!(diagnostic.contains("CARGO_BERTH_BYPASS=1"));
    assert!(!diagnostic.contains("is active in"));
    assert!(!diagnostic.contains("but this command ran in"));
}

fn assert_reference_transaction_rejects_stale_marker(marker_repository: &TempDir) {
    let marker_previous = git_stdout(marker_repository.path(), &["rev-parse", "HEAD"]);
    let marker_proposed = commit_work(
        marker_repository.path(),
        "tests/base.rs",
        "// marker target\n",
        "marker target",
    );
    let marker_seed = claim(
        marker_repository.path(),
        "file:marker-seed",
        FIRST_RUN,
        "docs/marker-gate.md",
        "marker-gate",
    );
    let marker_seed_id = reservation_id(&marker_seed);
    assert!(
        run_berth(
            marker_repository.path(),
            &["release", &marker_seed_id, "--json"],
        )
        .status
        .success()
    );
    fs::write(
        marker_repository.path().join(MARKER_PATH),
        format!("{FIRST_RUN}\n"),
    )
    .expect("stale marker should write");
    let marker_input = format!("{marker_previous} {marker_proposed} refs/heads/main\n");
    let marker_rejection = run_private_hook(marker_repository.path(), "prepared", &marker_input);
    let marker_diagnostic = String::from_utf8_lossy(&marker_rejection.stderr);
    assert_eq!(marker_rejection.status.code(), Some(5));
    assert!(marker_diagnostic.contains("inactive marker"));
    assert!(marker_diagnostic.contains("cargo-berth board --json"));
    assert!(!marker_diagnostic.contains("__reference-transaction"));
}

fn assert_reference_transaction_rejects_stale_session_mapping(session_repository: &TempDir) {
    let session_previous = git_stdout(session_repository.path(), &["rev-parse", "HEAD"]);
    let session_proposed = commit_work(
        session_repository.path(),
        "tests/base.rs",
        "// session target\n",
        "session target",
    );
    let session_id = "stale-reference-transaction-session";
    let mapped_claim = run_berth_with_session(
        session_repository.path(),
        &["claim", "file:session-seed", "--run", SECOND_RUN, "--json"],
        session_id,
    );
    let mapped_reservation_id = reservation_id(&mapped_claim);
    let mapping_path = session_repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(
            session_repository.path(),
            &["release", &mapped_reservation_id, "--json"],
        )
        .status
        .success()
    );
    fs::write(&mapping_path, stale_mapping).expect("stale mapping should write");
    let session_input = format!("{session_previous} {session_proposed} refs/heads/main\n");
    let session_rejection = run_private_hook_with_session(
        session_repository.path(),
        "prepared",
        &session_input,
        session_id,
    );
    let session_diagnostic = String::from_utf8_lossy(&session_rejection.stderr);
    assert_eq!(session_rejection.status.code(), Some(5));
    assert!(session_diagnostic.contains("inactive reservation"));
    assert!(session_diagnostic.contains("cargo-berth identity clear-session --json"));
    assert!(!session_diagnostic.contains("__reference-transaction"));
}

fn assert_reference_transaction_rejects_session_worktree_mismatch(mismatch_repository: &TempDir) {
    let mismatch_previous = git_stdout(mismatch_repository.path(), &["rev-parse", "HEAD"]);
    let mismatch_proposed = commit_work(
        mismatch_repository.path(),
        "tests/base.rs",
        "// mismatch target\n",
        "mismatch target",
    );
    let worktrees = tempdir().expect("worktree parent should exist");
    let second_root = add_worktree(
        mismatch_repository.path(),
        worktrees.path(),
        "mismatch-gate",
    );
    let mismatch_session = "foreign-reference-transaction-session";
    let live_claim = run_berth_with_session(
        mismatch_repository.path(),
        &[
            "claim",
            "file:mismatch-holder",
            "--run",
            THIRD_RUN,
            "--json",
        ],
        mismatch_session,
    );
    assert!(live_claim.status.success());
    let mismatch_input = format!("{mismatch_previous} {mismatch_proposed} refs/heads/main\n");
    let mismatch_rejection =
        run_private_hook_with_session(&second_root, "prepared", &mismatch_input, mismatch_session);
    let mismatch_diagnostic = String::from_utf8_lossy(&mismatch_rejection.stderr);
    assert_eq!(mismatch_rejection.status.code(), Some(5));
    assert!(mismatch_diagnostic.contains("active in"));
    assert!(mismatch_diagnostic.contains("cd "));
    assert!(mismatch_diagnostic.contains("cargo-berth identity clear-session --json"));
    assert!(mismatch_diagnostic.contains("retry the original git command"));
    assert!(!mismatch_diagnostic.contains("__reference-transaction"));
    assert!(
        mismatch_diagnostic.contains(
            mismatch_repository
                .path()
                .canonicalize()
                .expect("repository root should canonicalize")
                .to_str()
                .expect("repository root should be UTF-8")
        )
    );
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
fn retention_ref_writes_and_deletions_suppress_the_repository_root_hook() {
    let repository = initialized_repository();
    // Hook instrumentation is fixture metadata, not uncommitted branch work.
    fs::write(
        repository.path().join(".git/info/exclude"),
        "/repository-root-hooks/\n/reference-transaction-sentinel.log\n",
    )
    .expect("sentinel artifacts should be excluded from merge protection");
    let sentinel_log = install_repository_root_reference_transaction_sentinel(repository.path());
    let control_ref = "refs/hook-sentinel/control";
    git(repository.path(), &["update-ref", control_ref, "HEAD"]);
    let control_entries =
        fs::read_to_string(&sentinel_log).expect("reference-transaction sentinel log should read");
    assert!(
        control_entries
            .lines()
            .any(|entry| entry.ends_with(control_ref)),
        "reference-transaction sentinel did not observe the control ref: {control_entries}"
    );

    let claimed = claim(
        repository.path(),
        "file:src/lib.rs",
        FIRST_RUN,
        "docs/retention-suppression.md",
        "retention suppression",
    );
    assert!(claimed.status.success());
    let reservation_id = reservation_id(&claimed);
    let retention_ref = format!("refs/cargo-berth/reservations/{reservation_id}");

    let checkpointed = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert!(checkpointed.status.success());
    assert_eq!(
        json_output(&checkpointed)["payload"]["data"]["status"],
        "checkpointed"
    );
    assert!(reference_exists(repository.path(), &retention_ref));

    let evidence = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert!(evidence.status.success());
    assert_eq!(
        json_output(&evidence)["payload"]["data"]["status"],
        "released"
    );
    let released = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert!(released.status.success());
    assert_eq!(
        json_output(&released)["payload"]["data"]["status"],
        "already_settled"
    );
    assert!(reference_exists(repository.path(), &retention_ref));

    let reconciled = run_berth(repository.path(), &["board", "--json"]);
    assert!(reconciled.status.success());
    assert!(!reference_exists(repository.path(), &retention_ref));
    let sentinel_entries = fs::read_to_string(sentinel_log)
        .expect("reference-transaction sentinel log should read after retention operations");
    let retention_entries = sentinel_entries
        .lines()
        .filter(|entry| entry.contains(" refs/cargo-berth/"))
        .collect::<Vec<_>>();
    assert!(
        retention_entries.is_empty(),
        "sentinel observed retention refs: {retention_entries:#?}"
    );
}

#[test]
fn retention_ref_transactions_have_constant_git_invocations_across_cardinalities() {
    let one_repair = trace_retention_ref_reconciliation(1, RetentionRefPass::RepairOnly);
    let three_repairs = trace_retention_ref_reconciliation(3, RetentionRefPass::RepairOnly);
    let one_deletion = trace_retention_ref_reconciliation(1, RetentionRefPass::DeletionOnly);
    let three_deletions = trace_retention_ref_reconciliation(3, RetentionRefPass::DeletionOnly);

    assert_same_git_invocation_sequence(&one_repair, &three_repairs);
    assert_same_git_invocation_sequence(&one_deletion, &three_deletions);
    for trace in [&one_repair, &three_repairs, &one_deletion, &three_deletions] {
        assert_one_suppressed_ref_transaction(trace);
    }
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
fn filtered_three_commit_rebases_succeed_with_live_and_bypassed_hooks() {
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

    for hook_mode in [RebaseHookMode::FilteredBypass, RebaseHookMode::FilteredLive] {
        run_three_commit_rebase_sample(repository.path(), &source_tip, hook_mode);
    }
}

#[test]
fn managed_hook_fails_open_and_marks_a_bypass_for_an_unavailable_binary() {
    let repository = initialized_repository();
    let hook_path = repository.path().join(HOOK_PATH);
    let installed = fs::read_to_string(&hook_path).expect("managed hook should read");
    let broken = pin_managed_hook_executable(&installed, Path::new("/missing/cargo-berth"));
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
fn pending_markers_import_the_action_they_name_and_default_to_integration() {
    let repository = initialized_repository();
    let common_git_directory = repository.path().join(".git");
    fs::write(
        common_git_directory.join("cargo-berth-pending-bypass-edit-test.json"),
        concat!(
            r#"{"action":"editing","cause":{"kind":"environment_override","#,
            r#""bypassed_merge":"edit-hook-1"},"occurrence_time":{"status":"unavailable"}}"#,
            "\n"
        ),
    )
    .expect("editing marker should write");
    fs::write(
        common_git_directory.join("cargo-berth-pending-bypass-git-test.json"),
        concat!(
            r#"{"cause":{"kind":"environment_override","bypassed_merge":"git-process-1"},"#,
            r#""occurrence_time":{"status":"unavailable"}}"#,
            "\n"
        ),
    )
    .expect("integration marker should write");
    assert_eq!(pending_bypass_count(repository.path()), 2);

    let board = run_berth(repository.path(), &["board", "--json"]);
    assert!(
        board.status.success(),
        "board should import the pending markers: {}",
        String::from_utf8_lossy(&board.stderr)
    );

    let mut actions: Vec<String> = journal_text(repository.path())
        .lines()
        .filter_map(|record| serde_json::from_str::<serde_json::Value>(record).ok())
        .filter(|record| record["op"] == "bypass")
        .map(|record| {
            assert_eq!(record["cause"]["kind"], "environment_override");
            record["action"]
                .as_str()
                .expect("bypass record should name its action")
                .to_owned()
        })
        .collect();
    actions.sort();
    assert_eq!(actions, ["editing", "integration"]);
    assert_eq!(pending_bypass_count(repository.path()), 0);
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
    let non_trunk_input = format!(
        "{base} {base} ORIG_HEAD\n{base} {base} AUTO_MERGE\n{base} {base} refs/heads/topic\n"
    );

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
        .env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE)
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
    dirty_source(&holder_root, "src/lib.rs");
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
fn deferred_gate_observes_closed_stderr_then_enforces_every_entering_reservation() {
    let deferred_pair = deferred_pair(initialized_repository());
    let repository = &deferred_pair.repository;
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let blocked_head = commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn blocked_work() {}\n",
        "blocked work",
    );
    for case in [
        DeferredGateCase::ObserveClosedStderr,
        DeferredGateCase::EnforceEnteringReservations,
    ] {
        match case {
            DeferredGateCase::ObserveClosedStderr => {
                let executable = Path::new(env!("CARGO_BIN_EXE_cargo-berth"));
                let command = format!(
                    "exec {} __reference-transaction prepared refs/heads/main 2>&-",
                    shell_single_quoted(executable),
                );
                let mut child = Command::new("sh")
                    .args(["-c", &command])
                    .current_dir(repository.path())
                    .env(
                        REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT,
                        repository.path(),
                    )
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
            },
            DeferredGateCase::EnforceEnteringReservations => {
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
            },
        }
    }
}

enum DeferredGateCase {
    ObserveClosedStderr,
    EnforceEnteringReservations,
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
fn unavailable_worktree_heads_surface_as_enforced_violations() {
    let deferred_pair = deferred_pair(initialized_repository());
    let repository = &deferred_pair.repository;
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

    let deferred_pair = deferred_pair(initialized_repository());
    let repository = &deferred_pair.repository;
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
fn pending_rebase_checkpoint_obeys_ordering_in_direct_integration() {
    let fixture = pending_rebase_checkpoint_with_ordering_hold();
    let root = fixture.repository.path();
    let base = git_stdout(root, &["rev-parse", "main"]);
    let rebased_tip = git_stdout(&fixture.blocked_root, &["rev-parse", "HEAD"]);
    let direct = propose_trunk(root, &base, &rebased_tip);
    assert!(
        !direct.status.success(),
        "the ordering hold must reject the fast-forward"
    );
    let denial = String::from_utf8_lossy(&direct.stderr);
    assert!(denial.contains(&fixture.holder_id), "{denial}");
    assert!(denial.contains(&fixture.blocked_id), "{denial}");
    assert!(denial.contains("Ordering edge"), "{denial}");
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), base);
}

#[test]
fn pending_rebase_checkpoint_obeys_ordering_in_explicit_integration() {
    let fixture = pending_rebase_checkpoint_with_ordering_hold();
    let root = fixture.repository.path();
    let base = git_stdout(root, &["rev-parse", "main"]);
    let rebased_tip = git_stdout(&fixture.blocked_root, &["rev-parse", "HEAD"]);
    let explicit = run_berth(
        &fixture.blocked_root,
        &["integrate", &fixture.blocked_id, "--json"],
    );
    let decision = json_output(&explicit);
    assert_eq!(explicit.status.code(), Some(2), "{decision}");
    assert_eq!(decision["status"], "blocked_by_ordering", "{decision}");
    assert_eq!(
        decision["blocked_by"],
        serde_json::json!([fixture.holder_id])
    );
    let violation = &decision["payload"]["data"]["violations"][0];
    assert_eq!(
        violation["reservation"]["reservation_id"],
        fixture.blocked_id
    );
    assert_eq!(
        violation["reservation"]["lifecycle"]["protected_tip"],
        rebased_tip
    );
    assert_eq!(violation["holds"][0]["kind"], "ordering_edge");
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), base);
}

/// Leave the real rebase marker untouched until the caller enters one gate path.
fn pending_rebase_checkpoint_with_ordering_hold() -> DeferredPair {
    let fixture = deferred_pair(initialized_repository());
    let root = fixture.repository.path();
    let sequenced = run_berth(
        root,
        &[
            "sequence",
            &fixture.holder_id,
            &fixture.blocked_id,
            "--why",
            "predecessor must integrate before the rebased checkpoint",
            "--json",
        ],
    );
    assert!(sequenced.status.success(), "{}", json_output(&sequenced));
    let old_tip = commit_work(
        &fixture.blocked_root,
        "src/lib.rs",
        "pub fn rebased_successor() {}\n",
        "checkpoint successor before rebase",
    );
    let checkpoint = run_berth(
        &fixture.blocked_root,
        &["release", &fixture.blocked_id, "--json"],
    );
    assert!(checkpoint.status.success(), "{}", json_output(&checkpoint));
    assert_eq!(
        json_output(&checkpoint)["payload"]["data"]["status"],
        "checkpointed"
    );
    commit_work(root, "upstream.txt", "upstream advance\n", "advance trunk");
    set_gate_mode(root, "enforce");
    git(&fixture.blocked_root, &["rebase", "main"]);
    set_gate_mode(&fixture.blocked_root, "enforce");
    let rebased_tip = git_stdout(&fixture.blocked_root, &["rev-parse", "HEAD"]);
    assert_ne!(rebased_tip, old_tip);
    assert_eq!(pending_bypass_count(root), 1);
    let marker = pending_bypass_marker(root);
    assert_eq!(marker["kind"], "branch_rewrite");
    assert_eq!(
        marker["pairs"],
        serde_json::json!([{"old": old_tip, "new": rebased_tip}])
    );

    fixture
}

#[test]
fn budget_deferred_rewrite_obeys_ordering_in_direct_integration() {
    let fixture = two_pending_rewrites_with_ordering_hold();
    let root = fixture.pair.repository.path();
    let base = git_stdout(root, &["rev-parse", "main"]);
    let rewritten_tip = git_stdout(&fixture.pair.blocked_root, &["rev-parse", "HEAD"]);
    git(
        root,
        &["merge-base", "--is-ancestor", &base, &rewritten_tip],
    );
    let direct = propose_trunk(root, &base, &rewritten_tip);
    assert!(
        !direct.status.success(),
        "a deferred rewrite must retain its ordering hold"
    );
    let denial = String::from_utf8_lossy(&direct.stderr);
    assert!(denial.contains("Ordering edge"), "{denial}");
    assert!(denial.contains(&fixture.pair.holder_id), "{denial}");
    assert!(denial.contains(&fixture.pair.blocked_id), "{denial}");
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), base);
    assert_second_rewrite_remains_deferred(&fixture);
}

#[test]
fn budget_deferred_rewrite_obeys_ordering_in_explicit_integration() {
    let fixture = two_pending_rewrites_with_ordering_hold();
    let root = fixture.pair.repository.path();
    let base = git_stdout(root, &["rev-parse", "main"]);
    let explicit = run_berth(
        &fixture.pair.blocked_root,
        &["integrate", &fixture.pair.blocked_id, "--json"],
    );
    let decision = json_output(&explicit);
    assert_eq!(explicit.status.code(), Some(2), "{decision}");
    assert_eq!(decision["status"], "blocked_by_ordering", "{decision}");
    assert_eq!(
        decision["blocked_by"],
        serde_json::json!([fixture.pair.holder_id])
    );
    let violation = &decision["payload"]["data"]["violations"][0];
    assert_eq!(
        violation["reservation"]["reservation_id"],
        fixture.pair.blocked_id
    );
    assert_eq!(violation["holds"][0]["kind"], "ordering_edge");
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), base);
    assert_second_rewrite_remains_deferred(&fixture);
}

/// Two distinct cold subjects share one trunk target, with the ordered subject second.
struct DeferredRewriteGateFixture {
    pair:            DeferredPair,
    first_id:        String,
    blocked_old_tip: String,
}

fn two_pending_rewrites_with_ordering_hold() -> DeferredRewriteGateFixture {
    let pair = pending_rebase_checkpoint_with_ordering_hold();
    let root = pair.repository.path();
    let blocked_marker = pending_bypass_marker(root);
    let blocked_old_tip = blocked_marker["pairs"][0]["old"]
        .as_str()
        .expect("blocked rewrite should identify its old tip")
        .to_owned();
    assert_rewritten_tip_enters_without_old_tip(&pair, &blocked_old_tip);
    let marker_path = fs::read_dir(root.join(".git"))
        .expect("common Git directory should read")
        .map(|entry| entry.expect("marker entry should read").path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PENDING_BYPASS_PREFIX))
        })
        .expect("blocked marker should exist");
    let held_marker = pair.worktrees.path().join("blocked-marker.json");
    fs::rename(&marker_path, &held_marker)
        .expect("hold the second marker while preparing the first");
    let first_root = add_worktree(root, pair.worktrees.path(), "budget-first");
    let claimed = claim(
        &first_root,
        "file:tests/base.rs",
        THIRD_RUN,
        "docs/budget-first.md",
        "consume the first rewrite comparison",
    );
    assert!(claimed.status.success(), "{}", json_output(&claimed));
    let first_id = reservation_id(&claimed);
    commit_work(
        &first_root,
        "tests/base.rs",
        "// first protected phase\n",
        "first checkpoint",
    );
    let checkpoint = run_berth(&first_root, &["release", &first_id, "--json"]);
    assert!(checkpoint.status.success(), "{}", json_output(&checkpoint));
    assert_eq!(json_output(&checkpoint)["status"], "outstanding");
    let journal = root.join(JOURNAL_PATH);
    let permissions = fs::metadata(&journal)
        .expect("journal metadata should read")
        .permissions();
    fs::set_permissions(
        &journal,
        fs::Permissions::from_mode(permissions.mode() & !0o222),
    )
    .expect("journal should reject amend reconciliation");
    let amended = GIT.output(
        &first_root,
        [
            "commit",
            "--amend",
            "--quiet",
            "-m",
            "rewritten first checkpoint",
        ],
    );
    fs::set_permissions(&journal, permissions).expect("journal permissions should restore");
    assert!(
        amended.status.success(),
        "{}",
        String::from_utf8_lossy(&amended.stderr)
    );
    assert_eq!(pending_bypass_count(root), 1);
    let first_marker = pending_bypass_marker(root);
    assert_eq!(first_marker["kind"], "branch_rewrite");
    let first_path = root
        .join(".git")
        .join(format!("{PENDING_BYPASS_PREFIX}000-budget-first.json"));
    // Preserve the real hook payload while arranging deterministic importer order.
    for entry in fs::read_dir(root.join(".git")).expect("common Git directory should read") {
        let entry = entry.expect("marker entry should read");
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(PENDING_BYPASS_PREFIX)
        {
            fs::rename(entry.path(), &first_path)
                .expect("first marker should sort before the blocked marker");
        }
    }
    fs::rename(held_marker, marker_path).expect("restore the untouched blocked marker");
    assert_eq!(pending_bypass_count(root), 2);
    DeferredRewriteGateFixture {
        pair,
        first_id,
        blocked_old_tip,
    }
}

/// Only the rewritten destination can identify this reservation in the proposed update.
fn assert_rewritten_tip_enters_without_old_tip(pair: &DeferredPair, old_tip: &str) {
    for target in ["main", "HEAD"] {
        let contains_old = GIT.output(
            &pair.blocked_root,
            ["merge-base", "--is-ancestor", old_tip, target],
        );
        assert_eq!(
            contains_old.status.code(),
            Some(1),
            "old protected tip must be absent from {target}: {}",
            String::from_utf8_lossy(&contains_old.stderr)
        );
    }
    let already_on_trunk = GIT.output(
        &pair.blocked_root,
        ["merge-base", "--is-ancestor", "HEAD", "main"],
    );
    assert_eq!(already_on_trunk.status.code(), Some(1));
    git(
        &pair.blocked_root,
        &["merge-base", "--is-ancestor", "main", "HEAD"],
    );
}

fn assert_second_rewrite_remains_deferred(fixture: &DeferredRewriteGateFixture) {
    let operations = journal_text(fixture.pair.repository.path())
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("journal event should decode")
        })
        .collect::<Vec<_>>();
    assert!(operations.iter().any(|event| event["op"] == "resnapshot" && event["reservation_id"] == fixture.first_id),
        "first distinct subject must consume the acceptance budget: {operations:?}");
    assert!(
        !operations.iter().any(|event| event["op"] == "resnapshot"
            && event["reservation_id"] == fixture.pair.blocked_id),
        "second subject must remain deferred so this tests partial projection: {operations:?}"
    );
    assert_eq!(
        git_stdout(
            fixture.pair.repository.path(),
            &[
                "rev-parse",
                &format!("refs/cargo-berth/reservations/{}", fixture.pair.blocked_id)
            ]
        ),
        fixture.blocked_old_tip
    );
    assert_eq!(pending_bypass_count(fixture.pair.repository.path()), 1);
}

#[test]
fn forced_checkpoint_consumes_its_permit_before_ordinary_reconciliation_settles_it() {
    let fixture = deferred_pair(initialized_repository());
    let root = fixture.repository.path();
    let base = git_stdout(root, &["rev-parse", "main"]);
    let sequenced = run_berth(
        root,
        &[
            "sequence",
            &fixture.holder_id,
            &fixture.blocked_id,
            "--why",
            "unintegrated predecessor must land first",
            "--json",
        ],
    );
    assert!(sequenced.status.success(), "{}", json_output(&sequenced));
    let checkpoint = commit_work(
        &fixture.blocked_root,
        "src/lib.rs",
        "pub fn checkpointed_successor() {}\n",
        "checkpoint successor work",
    );
    let released = run_berth(
        &fixture.blocked_root,
        &["release", &fixture.blocked_id, "--json"],
    );
    assert!(released.status.success());
    assert_eq!(
        json_output(&released)["payload"]["data"]["status"],
        "checkpointed"
    );
    assert_eq!(
        git_stdout(&fixture.blocked_root, &["rev-parse", "HEAD"]),
        checkpoint
    );
    assert!(git_stdout(&fixture.blocked_root, &["status", "--porcelain"]).is_empty());
    set_gate_mode(root, "enforce");

    let blocked = propose_trunk(root, &base, &checkpoint);
    assert!(!blocked.status.success());
    let denial = String::from_utf8_lossy(&blocked.stderr);
    assert!(denial.contains(&fixture.holder_id), "{denial}");
    assert!(denial.contains(&fixture.blocked_id), "{denial}");
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), base);
    let before = journal_text(root).lines().count();

    let forced = run_berth(
        &fixture.blocked_root,
        &[
            "integrate",
            &fixture.blocked_id,
            "--force",
            "--why",
            "accept checkpoint ordering exception",
            "--json",
        ],
    );
    assert!(forced.status.success(), "{}", json_output(&forced));
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), checkpoint);
    assert_checkpoint_permit_audit(root, &fixture.blocked_id);

    let repeated = run_private_hook(
        root,
        "committed",
        &format!("{base} {checkpoint} refs/heads/main\n"),
    );
    assert!(repeated.status.success());
    assert_checkpoint_permit_audit(root, &fixture.blocked_id);
    let journal = journal_text(root);
    assert!(
        journal.lines().skip(before).all(|line| {
            let event: serde_json::Value = serde_json::from_str(line).expect("event should decode");
            event["op"] != "release" || event["reservation_id"] != fixture.blocked_id
        }),
        "the committed hook must leave the checkpoint outstanding"
    );
    let board = run_berth(root, &["board", "--json"]);
    assert!(board.status.success(), "{}", json_output(&board));
    let settlements = journal_text(root)
        .lines()
        .skip(before)
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should decode"))
        .filter(|event| event["op"] == "release" && event["reservation_id"] == fixture.blocked_id)
        .count();
    assert_eq!(
        settlements, 1,
        "board must settle the integrated checkpoint"
    );
    assert_checkpoint_permit_audit(root, &fixture.blocked_id);
}

/// Tie the one-use consumption and bypass audit to the checkpoint's issued permit.
fn assert_checkpoint_permit_audit(repository_root: &Path, reservation_id: &str) {
    assert_forced_permit_consumed(repository_root);
    let records = journal_text(repository_root)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should decode"))
        .collect::<Vec<_>>();
    let issued = records
        .iter()
        .find(|event| event["op"] == "forced_integration_permit")
        .expect("forced integration should issue a permit");
    let consumed = records
        .iter()
        .find(|event| event["op"] == "consume_forced_integration_permit")
        .expect("committed integration should consume its permit");
    assert_eq!(issued["reservation_id"], reservation_id);
    assert_eq!(consumed["reservation_id"], reservation_id);
    assert_eq!(consumed["permit_id"], issued["permit_id"]);
    let audits = records
        .iter()
        .filter(|event| event["op"] == "bypass" && event["cause"]["kind"] == "forced_integration")
        .collect::<Vec<_>>();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0]["cause"]["permit_id"], issued["permit_id"]);
}

#[test]
fn committed_hook_persists_one_scoped_patch_evaluation_record() {
    let repository = initialized_repository();
    let previous_trunk = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
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
    assert_ne!(protected_tip, target);
    assert!(!journal_text(repository.path()).contains("scoped_patch_equivalence_checked"));

    // A fast-forward to equivalent work exercises scoped evidence; an amend
    // transaction now accepts the rewrite map before ordinary gate evidence.
    let committed = run_private_hook(
        repository.path(),
        "committed",
        &format!("{previous_trunk} {target} refs/heads/main\n"),
    );
    assert!(
        committed.status.success(),
        "committed hook failed: {}",
        String::from_utf8_lossy(&committed.stderr)
    );
    assert_eq!(
        journal_text(repository.path())
            .matches("\"op\":\"scoped_patch_equivalence_checked\"")
            .count(),
        1
    );
}

#[test]
fn prepared_gate_settles_actual_trunk_evidence_but_never_a_proposed_witness() {
    for witness_is_actual in [false, true] {
        let repository = initialized_repository();
        let worktrees = tempdir().expect("worktree parent should exist");
        let holder = add_worktree(repository.path(), worktrees.path(), "settlement-holder");
        let base = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
        let id = reservation_id(&claim(
            &holder,
            "file:src/lib.rs",
            FIRST_RUN,
            "docs/settlement.md",
            "settlement",
        ));
        let checkpoint = commit_work_without_hooks(
            &holder,
            "src/lib.rs",
            "pub fn integrated() {}\n",
            "protected checkpoint",
        );
        let released = run_berth(&holder, &["release", &id, "--json"]);
        assert!(released.status.success());
        assert_eq!(
            json_output(&released)["payload"]["data"]["status"],
            "checkpointed"
        );
        let witness = commit_work_without_hooks(
            repository.path(),
            "src/lib.rs",
            "pub fn integrated() {}\n",
            "equivalent trunk witness",
        );
        assert_ne!(checkpoint, witness);
        let (actual, proposed) = if witness_is_actual {
            (witness.clone(), checkpoint.clone())
        } else {
            git(
                repository.path(),
                &["-c", "core.hooksPath=/dev/null", "reset", "--hard", &base],
            );
            (base, witness.clone())
        };
        let before = journal_text(repository.path()).lines().count();
        let input = format!("{actual} {proposed} refs/heads/main\n");

        let prepared = run_private_hook(repository.path(), "prepared", &input);
        assert!(
            prepared.status.success(),
            "{}",
            String::from_utf8_lossy(&prepared.stderr)
        );
        assert_eq!(
            git_stdout(repository.path(), &["rev-parse", "main"]),
            actual
        );
        let journal = journal_text(repository.path());
        let operations = journal
            .lines()
            .skip(before)
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("event should decode")
            })
            .collect::<Vec<_>>();
        let settlements = operations
            .iter()
            .filter(|event| event["op"] == "release" && event["reservation_id"] == id)
            .collect::<Vec<_>>();
        assert_eq!(
            settlements.len(),
            usize::from(witness_is_actual),
            "{journal}"
        );
        if witness_is_actual {
            assert_eq!(
                settlements[0]["disposition"]["kind"],
                "rewritten_integration"
            );
            assert_eq!(settlements[0]["disposition"]["evidence"], witness);
            let release_position = operations
                .iter()
                .position(|event| event["op"] == "release" && event["reservation_id"] == id)
                .expect("actual settlement should exist");
            assert_eq!(
                operations[release_position - 1]["op"],
                "evidence_revalidated"
            );
            assert_eq!(operations[release_position - 1]["reservation_id"], id);
        }
    }
    assert_prepared_gate_requires_the_settled_predecessor_witness();
}

/// A successor carrying the old checkpoint still needs the rewritten trunk witness.
fn assert_prepared_gate_requires_the_settled_predecessor_witness() {
    let fixture = deferred_pair(initialized_repository());
    let root = fixture.repository.path();
    let holder = fixture.worktrees.path().join("pair-holder");
    let sequenced = run_berth(
        root,
        &[
            "sequence",
            &fixture.holder_id,
            &fixture.blocked_id,
            "--why",
            "predecessor lands first",
            "--json",
        ],
    );
    assert!(sequenced.status.success(), "{}", json_output(&sequenced));
    let checkpoint = commit_work_without_hooks(
        &holder,
        "src/lib.rs",
        "pub fn integrated() {}\n",
        "protected predecessor",
    );
    let released = run_berth(&holder, &["release", &fixture.holder_id, "--json"]);
    assert!(released.status.success());
    git(
        &fixture.blocked_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            &checkpoint,
        ],
    );
    let successor = commit_work_without_hooks(
        &fixture.blocked_root,
        "src/successor.rs",
        "pub fn successor() {}\n",
        "successor work",
    );
    let witness = commit_work_without_hooks(
        root,
        "src/lib.rs",
        "pub fn integrated() {}\n",
        "rewritten predecessor witness",
    );
    assert_ne!(checkpoint, witness);
    set_gate_mode(root, "enforce");
    let input = format!("{witness} {successor} refs/heads/main\n");

    let first = run_private_hook(root, "prepared", &input);
    assert!(
        !first.status.success(),
        "gate admitted successor without the trunk witness"
    );
    let denial = String::from_utf8_lossy(&first.stderr);
    assert!(denial.contains(&fixture.holder_id), "{denial}");
    assert!(denial.contains(&fixture.blocked_id), "{denial}");
    let journal = journal_text(root);
    assert!(
        journal.lines().any(|line| {
            let event: serde_json::Value = serde_json::from_str(line).expect("event should decode");
            event["op"] == "release" && event["reservation_id"] == fixture.holder_id
        }),
        "actual-trunk settlement must commit even when the proposal is denied"
    );
    let board = run_berth(root, &["board", "--json"]);
    assert!(board.status.success());
    let repeated = run_private_hook(root, "prepared", &input);
    assert_eq!(repeated.status.code(), first.status.code());
    let repeated_denial = String::from_utf8_lossy(&repeated.stderr);
    assert!(
        repeated_denial.contains(&fixture.holder_id),
        "{repeated_denial}"
    );
    assert!(
        repeated_denial.contains(&fixture.blocked_id),
        "{repeated_denial}"
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
        let row = board_reservation_snapshot(data, reservation_id);
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
fn hook_lock_contention_uses_one_shortened_lock_deadline() {
    let blocked = run_gate_with_blocked_mutation_lock(
        LOCK_CONTENTION_TOLERANCE_ENVIRONMENT,
        LOCK_CONTENTION_TOLERANCE,
    );
    let diagnostic = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        diagnostic.contains("lock deadline; the ledger was busy"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("total deadline"), "{diagnostic}");
    assert!(diagnostic.contains("10-second"));
    assert!(diagnostic.contains("CARGO_BERTH_BYPASS=1"));
}

#[test]
fn hook_outer_gate_deadline_expires_while_worker_waits_on_the_mutation_lock() {
    let blocked = run_gate_with_blocked_mutation_lock(GATE_DEADLINE_ENVIRONMENT, GATE_DEADLINE);
    let diagnostic = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        diagnostic
            .contains("exhausted its 10-second total deadline; no integration decision was made"),
        "{diagnostic}"
    );
    assert!(diagnostic.contains("CARGO_BERTH_BYPASS=1"));
}

#[test]
fn git_hook_post_commit_path_and_commit_cardinality_matrix_is_fixed() {
    // This invokes berth's installed git post-commit hook. The external Claude
    // PostToolUse shim is outside this repository and is not exercised here.
    let one_reservation = post_commit_reservation_cardinality_trace(1);
    let three_reservations = post_commit_reservation_cardinality_trace(3);

    assert_same_git_process_multiset(&one_reservation, &three_reservations);
    assert_eq!(
        one_reservation.len(),
        POST_COMMIT_ENGINE_GIT_PROCESS_CEILING
    );
    assert!(three_reservations.len() <= POST_COMMIT_ENGINE_GIT_PROCESS_CEILING);
    assert_eq!(
        one_reservation
            .iter()
            .filter(|invocation| raw_git_command(invocation) == Some("log"))
            .count(),
        1
    );
    assert_eq!(
        three_reservations
            .iter()
            .filter(|invocation| raw_git_command(invocation) == Some("log"))
            .count(),
        1
    );

    let attributed_baseline = post_commit_path_commit_cardinality_trace(1, 1);
    assert_eq!(
        attributed_baseline.len(),
        POST_COMMIT_ENGINE_GIT_PROCESS_CEILING
    );
    assert_eq!(git_command_count(&attributed_baseline, "log"), 1);
    for path_count in [0, 1, 2] {
        for commit_count in [0, 1, 2] {
            let observed = post_commit_path_commit_cardinality_trace(path_count, commit_count);
            assert!(observed.len() <= POST_COMMIT_ENGINE_GIT_PROCESS_CEILING);
            if path_count > 0 && commit_count > 0 {
                assert_same_git_process_multiset(&attributed_baseline, &observed);
                assert_eq!(git_command_count(&observed, "log"), 1);
            } else {
                assert_eq!(git_command_count(&observed, "log"), 0);
            }
        }
    }

    for (path_count, commit_count) in [(0, 0), (0, 1), (0, 14), (0, 100), (1, 0), (4, 0), (33, 0)] {
        let observed = post_commit_path_commit_cardinality_trace(path_count, commit_count);
        assert!(observed.len() <= POST_COMMIT_ENGINE_GIT_PROCESS_CEILING);
        assert_eq!(git_command_count(&observed, "log"), 0);
    }
}

#[test]
fn batched_git_path_distinguishes_spawn_failure_from_completed_failure() {
    let unavailable_repository = initialized_repository();
    let unavailable_reservation = reservation_id(&claim(
        unavailable_repository.path(),
        "file:owned.txt",
        FIRST_RUN,
        "docs/unavailable.md",
        "unavailable git boundary",
    ));
    let (unavailable, unavailable_trace) = run_berth_with_raw_git_behavior(
        unavailable_repository.path(),
        &[
            "drift",
            "--full",
            "--reservation",
            &unavailable_reservation,
            "--json",
        ],
        RawGitBehavior::RemoveAfterTargetHistory,
    );
    let unavailable_envelope = json_output(&unavailable);
    assert_eq!(unavailable_envelope["status"], "ledger_unreadable");
    // Assert the spawn failure, not which git call happened to hit the removed wrapper first.
    // Drift issues its fingerprint commands concurrently, so under a loaded suite the first
    // call to find the wrapper gone is whichever the scheduler let through --- observed as
    // `could not run drift fingerprint` alone and as `git cat-file failed while computing
    // drift` under load. Both are the same event: the engine could not spawn git at all, which
    // is what this test distinguishes from a git that ran and exited non-zero. Pinning one
    // subcommand pinned the scheduler.
    assert!(
        unavailable_envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("No such file or directory")),
        "spawn failure did not retain its typed diagnostic: {unavailable_envelope}"
    );
    assert!(
        unavailable_trace
            .iter()
            .any(|invocation| raw_git_command(invocation) == Some("rev-list")),
        "the failed batched history read was absent: {unavailable_trace:#?}"
    );
    assert_eq!(git_command_count(&unavailable_trace, "diff-tree"), 0);

    let failed_repository = initialized_repository();
    let failed_reservation = reservation_id(&claim(
        failed_repository.path(),
        "file:owned.txt",
        FIRST_RUN,
        "docs/failed.md",
        "failed git boundary",
    ));
    let (failed, failed_trace) = run_berth_with_raw_git_behavior(
        failed_repository.path(),
        &[
            "drift",
            "--full",
            "--reservation",
            &failed_reservation,
            "--json",
        ],
        RawGitBehavior::FailPhaseDiff,
    );
    let failed_envelope = json_output(&failed);
    assert_eq!(failed_envelope["status"], "ledger_unreadable");
    assert!(
        failed_envelope["message"].as_str().is_some_and(|message| {
            message.contains("failed while computing drift")
                && message.contains("injected completed git failure")
        }),
        "completed failure did not retain its typed diagnostic: {failed_envelope}"
    );
    assert_eq!(git_command_count(&failed_trace, "diff-tree"), 1);
    assert!(
        !failed_envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("No such file or directory")),
        "a git that ran and exited non-zero must not read as a spawn failure: {failed_envelope}"
    );
    assert_ne!(unavailable_envelope["message"], failed_envelope["message"]);
}

#[test]
fn incursion_report_survives_origin_classification_failure() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let subject_root = add_worktree(
        repository.path(),
        worktrees.path(),
        "origin-classification-failure",
    );
    dirty_source(repository.path(), "held.txt");
    assert!(
        claim(
            repository.path(),
            "file:held.txt",
            FIRST_RUN,
            "docs/origin-holder.md",
            "origin holder",
        )
        .status
        .success()
    );
    let subject_id = reservation_id(&claim(
        &subject_root,
        "file:own.txt",
        SECOND_RUN,
        "docs/origin-subject.md",
        "origin subject",
    ));
    fs::write(subject_root.join("held.txt"), "entered holder scope\n")
        .expect("held path should write");
    git(&subject_root, &["add", "held.txt"]);
    git(
        &subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "enter holder scope",
        ],
    );
    let entering_commit = git_stdout(&subject_root, &["rev-parse", "HEAD"]);
    let origin_basis = git_stdout(&subject_root, &["rev-parse", "refs/heads/main"]);
    let origin_range = format!("{origin_basis}..{entering_commit}");
    let (reported, trace) = run_berth_with_raw_git_behavior(
        &subject_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        RawGitBehavior::FailOriginClassification,
    );
    let envelope = json_output(&reported);

    assert_eq!(reported.status.code(), Some(1));
    assert_eq!(envelope["status"], "incursion");
    let commit = envelope["payload"]["data"]["results"]
        .as_array()
        .and_then(|results| {
            results.iter().find_map(|result| {
                result["effects"]
                    .as_array()?
                    .iter()
                    .find(|effect| effect["kind"] == "incursion")
                    .map(|effect| &effect["commits"][0])
            })
        })
        .expect("the incursion should retain commit attribution");
    assert_eq!(commit["commit"], entering_commit);
    assert_eq!(commit["origin"], "unknown");

    let origin_invocation_count = trace
        .iter()
        .filter(|invocation| {
            invocation.arguments == ["git", "--no-optional-locks", "rev-list", &origin_range]
        })
        .count();
    assert_eq!(origin_invocation_count, 1, "raw git trace: {trace:#?}");
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

    let object_status = git_command(BERTH_EXECUTABLE)
        .arg("--no-optional-locks")
        .args(["cat-file", "-e", &format!("{protected_tip}^{{commit}}")])
        .current_dir(repository.path())
        .status()
        .expect("git cat-file should run");
    assert!(!object_status.success());
}

#[derive(Debug, Eq, PartialEq)]
struct RawGitInvocation {
    arguments: Vec<String>,
}

#[derive(Clone, Copy)]
enum RawGitBehavior {
    PassThrough,
    RemoveAfterTargetHistory,
    FailPhaseDiff,
    FailOriginClassification,
}

impl RawGitBehavior {
    const fn as_str(self) -> &'static str {
        match self {
            Self::PassThrough => "pass_through",
            Self::RemoveAfterTargetHistory => "remove_after_target_history",
            Self::FailPhaseDiff => "fail_phase_diff",
            Self::FailOriginClassification => "fail_origin_classification",
        }
    }
}

#[derive(Clone, Copy)]
enum RetentionRefPass {
    RepairOnly,
    DeletionOnly,
}

struct ManagedHookSpy {
    phase_log: PathBuf,
    stdin_log: PathBuf,
}

#[derive(Clone, Copy)]
enum RebaseHookMode {
    FilteredBypass,
    FilteredLive,
}

struct DeferredPair {
    repository:   TempDir,
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
    // Keep every proof subject outstanding while both target schedules are exercised.
    for scope in &scopes {
        fs::write(
            repository.path().join(scope),
            "// reserved work remains dirty\n",
        )
        .expect("reserved dirt should prevent lifecycle settlement");
    }

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

fn deferred_pair(repository: TempDir) -> DeferredPair {
    let repository_root = repository.path();
    let worktrees = tempdir().expect("worktree parent should exist");
    let holder_root = add_worktree(repository_root, worktrees.path(), "pair-holder");
    let blocked_root = add_worktree(repository_root, worktrees.path(), "pair-blocked");
    dirty_source(&holder_root, "src/lib.rs");
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
        repository,
        worktrees,
        blocked_root,
        blocked_id: reservation_id(&blocked),
        holder_id,
    }
}

/// Hold actual uncommitted work on the branch whose integration conflicts.
fn dirty_source(root: &Path, path: &str) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().expect("held path has a parent"))
        .expect("held directory should exist");
    fs::write(target, "uncommitted holder work\n").expect("held work should write");
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
    // Git runs `maintenance run --auto --detach` after a commit, and this machine leaves that
    // default on. Several detached runs then repack one repository at once, and the geometric
    // repack deletes a pack a commit still in flight is reading, which surfaces as
    // `invalid object <oid> for '<path>'` from a commit that did nothing wrong. A fixture
    // repository is short-lived and never needs maintenance, so it opts out of both schedulers.
    git(repository.path(), &["config", "maintenance.auto", "false"]);
    git(repository.path(), &["config", "gc.auto", "0"]);
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
    propose_branch(repository_root, "main", previous, proposed)
}

fn propose_branch(repository_root: &Path, branch: &str, previous: &str, proposed: &str) -> Output {
    update_branch(
        repository_root,
        branch,
        previous,
        proposed,
        ReleaseValve::Unset,
    )
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

fn update_main(
    repository_root: &Path,
    previous: &str,
    proposed: &str,
    release_valve: ReleaseValve,
) -> Output {
    update_branch(repository_root, "main", previous, proposed, release_valve)
}

fn update_branch(
    repository_root: &Path,
    branch: &str,
    previous: &str,
    proposed: &str,
    release_valve: ReleaseValve,
) -> Output {
    let mut command = git_command(BERTH_EXECUTABLE);
    command
        .arg("--no-optional-locks")
        .args([
            "update-ref",
            &format!("refs/heads/{branch}"),
            proposed,
            previous,
        ])
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
    GIT.succeeds(
        repository_root,
        &["show-ref", "--verify", "--quiet", reference],
    )
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

fn board_reservation_snapshot<'board>(
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

fn run_gate_with_blocked_mutation_lock(deadline_environment: &str, deadline: Duration) -> Output {
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
    let ready_directory = tempdir().expect("lock readiness directory should exist");
    let ready_path = ready_directory.path().join("ready");
    // Measure from launch; the worker starts its deadline before reporting readiness.
    let started_at = Instant::now();
    let mut child = berth_command()
        .args(["__reference-transaction", "prepared", "refs/heads/main"])
        .current_dir(repository.path())
        .env(
            REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT,
            repository.path(),
        )
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env_remove(LOCK_CONTENTION_TOLERANCE_ENVIRONMENT)
        .env_remove(GATE_DEADLINE_ENVIRONMENT)
        .env(deadline_environment, deadline.as_millis().to_string())
        .env(MUTATION_LOCK_READY_ENVIRONMENT, &ready_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("contended private gate should start");
    child
        .stdin
        .take()
        .expect("private gate stdin should exist")
        .write_all(format!("{base} {feature_head} refs/heads/main\n").as_bytes())
        .expect("private gate stdin should write");
    while !ready_path.exists() && started_at.elapsed() < SCHEDULING_ALLOWANCE {
        thread::sleep(POLL_INTERVAL);
    }
    if !ready_path.exists() {
        child
            .kill()
            .expect("gate without a ready signal should stop");
        child.wait().expect("stopped gate should be reaped");
    }
    assert!(
        ready_path.exists(),
        "gate worker should report mutation lock contention"
    );
    let blocked = child
        .wait_with_output()
        .expect("private gate should finish");
    let elapsed = started_at.elapsed();
    assert_eq!(blocked.status.code(), Some(6));
    assert!(elapsed >= deadline, "gate returned too early: {elapsed:?}");
    assert!(
        elapsed < deadline + SCHEDULING_ALLOWANCE,
        "gate took too long: {elapsed:?}"
    );
    drop(lock_file);
    blocked
}

fn run_installed_prepared_hook(repository_root: &Path, input: &str) -> Output {
    let mut child = Command::new(repository_root.join(HOOK_PATH))
        .arg("prepared")
        .current_dir(repository_root)
        .env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("installed hook should start");
    child
        .stdin
        .take()
        .expect("installed hook stdin")
        .write_all(input.as_bytes())
        .expect("installed hook input writes");
    child.wait_with_output().expect("installed hook finishes")
}

fn run_private_hook(repository_root: &Path, phase: &str, input: &str) -> Output {
    let mut child = berth_command()
        .args(["__reference-transaction", phase, "refs/heads/main"])
        .current_dir(repository_root)
        .env(
            REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT,
            repository_root,
        )
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

fn run_private_hook_with_session(
    repository_root: &Path,
    phase: &str,
    input: &str,
    session_id: &str,
) -> Output {
    let mut child = berth_command()
        .args(["__reference-transaction", phase, "refs/heads/main"])
        .current_dir(repository_root)
        .env(
            REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT,
            repository_root,
        )
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
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

fn run_managed_hook_with_session(
    policy_repository_root: &Path,
    issuing_root: &Path,
    phase: &str,
    input: &str,
    session_id: &str,
) -> Output {
    let mut child = Command::new(policy_repository_root.join(HOOK_PATH))
        .arg(phase)
        .current_dir(issuing_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("managed gate should start");
    child
        .stdin
        .take()
        .expect("managed gate stdin should exist")
        .write_all(input.as_bytes())
        .expect("managed gate stdin should write");
    child
        .wait_with_output()
        .expect("managed gate should finish")
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
        .stderr(Stdio::piped())
        // A hook run straight from its path skips the git invocation that would
        // otherwise name the binary under test, so it names it here. Left to its
        // own resolution the hook finds whatever `cargo install` last left on the
        // machine, and a test then passes or fails on that instead of on the code
        // it was compiled from. A fixture that pins the executable in the script
        // text still wins, because pinning replaces this variable's reader.
        .env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE);
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
    let input_result = child
        .stdin
        .take()
        .expect("managed hook stdin should exist")
        .write_all(input);
    if let Err(error) = input_result {
        // Ignored phases exit before reading stdin. The caller still checks
        // the hook's exit status and whether its worker was dispatched.
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
    child
        .wait_with_output()
        .expect("managed hook should finish")
}

fn post_commit_reservation_cardinality_trace(reservation_count: usize) -> Vec<RawGitInvocation> {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let subject_root = add_worktree(
        repository.path(),
        worktrees.path(),
        "hook-incursion-subject",
    );
    dirty_source(repository.path(), "hook-incursion/entered.txt");
    assert!(
        claim(
            repository.path(),
            "tree:hook-incursion",
            FIRST_RUN,
            "docs/hook-incursion-holder.md",
            "hook incursion holder",
        )
        .status
        .success()
    );
    for index in 0..reservation_count {
        let subject_path = format!("subject-{index}.txt");
        assert!(
            claim(
                &subject_root,
                &format!("file:{subject_path}"),
                SECOND_RUN,
                "docs/hook-incursion-subject.md",
                &format!("hook incursion subject {index}"),
            )
            .status
            .success()
        );
        fs::write(
            subject_root.join(&subject_path),
            format!("subject {index}\n"),
        )
        .expect("hook subject path should write");
        git(&subject_root, &["add", &subject_path]);
        git(
            &subject_root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                &format!("hook subject {index}"),
            ],
        );
    }
    fs::create_dir_all(subject_root.join("hook-incursion"))
        .expect("hook incursion directory should exist");
    fs::write(subject_root.join("hook-incursion/entered.txt"), "entered\n")
        .expect("hook incursion path should write");
    git(&subject_root, &["add", "hook-incursion/entered.txt"]);
    git(
        &subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "hook incursion work",
        ],
    );
    let post_commit_hook = repository.path().join(".git/hooks/post-commit");
    let (output, invocations) =
        run_command_with_raw_git_trace(&subject_root, post_commit_hook.as_os_str(), &[]);
    assert!(
        output.status.success(),
        "traced post-commit hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    invocations
}

fn post_commit_path_commit_cardinality_trace(
    path_count: usize,
    commit_count: usize,
) -> Vec<RawGitInvocation> {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let subject_root = add_worktree(
        repository.path(),
        worktrees.path(),
        "hook-cardinality-subject",
    );
    for index in 0..path_count {
        dirty_source(
            repository.path(),
            &format!("hook-cardinality/path-{index}.txt"),
        );
    }
    assert!(
        claim(
            repository.path(),
            "tree:hook-cardinality",
            FIRST_RUN,
            "docs/hook-cardinality-holder.md",
            "hook cardinality holder",
        )
        .status
        .success()
    );
    assert!(
        claim(
            &subject_root,
            "file:subject.txt",
            SECOND_RUN,
            "docs/hook-cardinality-subject.md",
            "hook cardinality subject",
        )
        .status
        .success()
    );
    fs::create_dir_all(subject_root.join("hook-cardinality"))
        .expect("hook cardinality directory should exist");
    for commit_index in 0..commit_count {
        if path_count == 0 {
            fs::write(
                subject_root.join("subject.txt"),
                format!("subject version {commit_index}\n"),
            )
            .expect("covered subject path should write");
        } else if commit_index == 0 {
            for path_index in 0..path_count {
                fs::write(
                    subject_root.join(format!("hook-cardinality/path-{path_index}.txt")),
                    format!("path {path_index}, version {commit_index}\n"),
                )
                .expect("entered cardinality path should write");
            }
        } else {
            let path_index = commit_index % path_count;
            fs::write(
                subject_root.join(format!("hook-cardinality/path-{path_index}.txt")),
                format!("path {path_index}, version {commit_index}\n"),
            )
            .expect("entered cardinality path should update");
        }
        git(&subject_root, &["add", "-A"]);
        git(
            &subject_root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                &format!("hook cardinality commit {commit_index}"),
            ],
        );
    }
    if commit_count == 0 {
        for path_index in 0..path_count {
            fs::write(
                subject_root.join(format!("hook-cardinality/path-{path_index}.txt")),
                format!("uncommitted path {path_index}\n"),
            )
            .expect("uncommitted cardinality path should write");
        }
    }
    let post_commit_hook = repository.path().join(".git/hooks/post-commit");
    let (output, invocations) =
        run_command_with_raw_git_trace(&subject_root, post_commit_hook.as_os_str(), &[]);
    assert!(
        output.status.success(),
        "traced post-commit hook failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    invocations
}

fn run_berth_with_raw_git_trace(
    repository_root: &Path,
    arguments: &[&str],
) -> (Output, Vec<RawGitInvocation>) {
    run_command_with_raw_git_behavior(
        repository_root,
        OsStr::new(env!("CARGO_BIN_EXE_cargo-berth")),
        arguments,
        RawGitBehavior::PassThrough,
    )
}

fn run_berth_with_raw_git_behavior(
    repository_root: &Path,
    arguments: &[&str],
    raw_git_behavior: RawGitBehavior,
) -> (Output, Vec<RawGitInvocation>) {
    run_command_with_raw_git_behavior(
        repository_root,
        OsStr::new(env!("CARGO_BIN_EXE_cargo-berth")),
        arguments,
        raw_git_behavior,
    )
}

fn run_command_with_raw_git_trace(
    repository_root: &Path,
    executable: &OsStr,
    arguments: &[&str],
) -> (Output, Vec<RawGitInvocation>) {
    run_command_with_raw_git_behavior(
        repository_root,
        executable,
        arguments,
        RawGitBehavior::PassThrough,
    )
}

fn run_command_with_raw_git_behavior(
    repository_root: &Path,
    executable: &OsStr,
    arguments: &[&str],
    raw_git_behavior: RawGitBehavior,
) -> (Output, Vec<RawGitInvocation>) {
    let directory = tempdir().expect("wrapper directory should exist");
    let wrapper_path = directory.path().join(GIT_BINARY);
    let trace_path = directory.path().join("raw-trace");
    fs::write(&wrapper_path, RAW_TRACING_GIT_WRAPPER).expect("git wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let original_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = match raw_git_behavior {
        RawGitBehavior::RemoveAfterTargetHistory => directory.path().as_os_str().to_owned(),
        RawGitBehavior::PassThrough
        | RawGitBehavior::FailPhaseDiff
        | RawGitBehavior::FailOriginClassification => std::env::join_paths(
            std::iter::once(directory.path().to_path_buf())
                .chain(std::env::split_paths(&original_path)),
        )
        .expect("wrapped PATH should join"),
    };
    let output = Command::new(executable)
        .args(arguments)
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(RAW_GIT_BEHAVIOR_ENVIRONMENT, raw_git_behavior.as_str())
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(TRACE_ENVIRONMENT, &trace_path)
        .env(UTILITY_PATH_ENVIRONMENT, &original_path)
        // The traced command is a managed hook, or reaches one through the
        // wrapped git, and a hook resolves its own cargo-berth. The wrapped
        // search path still carries the machine's own, so without this the
        // trace counts whatever `cargo install` last left there.
        .env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run");
    let trace = fs::read_to_string(&trace_path).expect("raw git trace should read");
    let invocations = trace
        .split('\u{1e}')
        .filter(|record| !record.is_empty())
        .map(|record| RawGitInvocation {
            arguments: record.split('\u{1f}').map(str::to_owned).collect(),
        })
        .collect();
    (output, invocations)
}

/// Start the `cargo-berth` under test, named for any hook it goes on to fire.
///
/// The binary a test spawns is settled by its own path, but a managed hook this
/// binary triggers resolves `cargo-berth` for itself and answers with whatever
/// `cargo install` last left on the machine. Naming it here keeps a hook on the
/// code the test was compiled from, so a developer machine with an installed
/// copy and a continuous integration machine without one agree.
fn berth_command() -> Command {
    let mut command = Command::new(BERTH_EXECUTABLE);
    command.env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE);
    command
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command()
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
    berth_command()
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
    let mut child = berth_command()
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

/// Pin a managed hook to one binary in place of the resolution it performs when it runs.
///
/// The installed hook names no binary: it resolves `cargo-berth` at run time so a
/// hook outlives the build that wrote it. A test that needs a stub, a spy, or a
/// path that is deliberately absent replaces that resolution with a fixed answer.
fn pin_managed_hook_executable(installed: &str, executable: &Path) -> String {
    let resolution = format!("cargo_berth_executable=\"${{{EXECUTABLE_ENVIRONMENT}:-}}\"");
    let pinned = installed.replace(
        resolution.as_str(),
        &format!("cargo_berth_executable={}", shell_single_quoted(executable)),
    );
    assert_ne!(
        pinned, installed,
        "managed hook should carry its executable resolution"
    );
    pinned
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
    let instrumented = pin_managed_hook_executable(&installed, &spy_path);
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
    let deferred_pair = deferred_pair(initialized_repository());
    let repository = &deferred_pair.repository;
    commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub fn forced_work() {}\n",
        "forced integration work",
    );
    set_gate_mode(repository.path(), "enforce");
    let phase_log = wrap_managed_hook_with_phase_log(repository.path());

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
    assert_forced_permit_consumed(repository.path());
    let lifecycle = fs::read_to_string(phase_log)
        .expect("forced-integration phase log should read")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(lifecycle, ["preparing", "prepared", "committed"]);
    lifecycle
        .into_iter()
        .filter(|phase| phase != "preparing")
        .collect()
}

fn wrap_managed_hook_with_phase_log(repository_root: &Path) -> PathBuf {
    let hook_path = repository_root.join(HOOK_PATH);
    let original_hook_path = repository_root.join(".git/reference-transaction.original");
    fs::copy(&hook_path, &original_hook_path).expect("managed hook should copy");
    let phase_log = repository_root.join(".git/reference-transaction-phases.log");
    let wrapper = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\nexec {} \"$@\"\n",
        shell_single_quoted(&phase_log),
        shell_single_quoted(&original_hook_path),
    );
    fs::write(&hook_path, wrapper).expect("managed hook wrapper should write");
    let mut permissions = fs::metadata(&hook_path)
        .expect("managed hook wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook_path, permissions)
        .expect("managed hook wrapper should be executable");
    phase_log
}

fn install_repository_root_reference_transaction_sentinel(repository_root: &Path) -> PathBuf {
    let hooks_directory = repository_root.join("repository-root-hooks");
    fs::create_dir(&hooks_directory).expect("repository-root hooks directory should exist");
    git(
        repository_root,
        &[
            "config",
            "core.hooksPath",
            hooks_directory
                .to_str()
                .expect("repository-root hooks path should be UTF-8"),
        ],
    );
    let sentinel_path = hooks_directory.join("reference-transaction");
    let sentinel_log = repository_root.join("reference-transaction-sentinel.log");
    let script = format!(
        "#!/bin/sh\nwhile read -r old_object new_object reference; do\n    printf '%s %s\\n' \"$1\" \"$reference\" >> {}\ndone\n",
        shell_single_quoted(&sentinel_log),
    );
    fs::write(&sentinel_path, script).expect("repository-root sentinel should write");
    let mut permissions = fs::metadata(&sentinel_path)
        .expect("repository-root sentinel metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&sentinel_path, permissions)
        .expect("repository-root sentinel should be executable");
    sentinel_log
}

fn trace_retention_ref_reconciliation(
    reservation_count: usize,
    pass: RetentionRefPass,
) -> Vec<RawGitInvocation> {
    let repository = initialized_repository();
    let reservation_ids = checkpointed_reservations(repository.path(), reservation_count);
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    match pass {
        RetentionRefPass::RepairOnly => {
            let transaction =
                reservation_ids
                    .iter()
                    .fold(String::new(), |mut transaction, reservation_id| {
                        let _ = writeln!(
                            transaction,
                            "delete refs/cargo-berth/reservations/{reservation_id}"
                        );
                        transaction
                    });
            apply_test_ref_transaction(repository.path(), &transaction);
        },
        RetentionRefPass::DeletionOnly => {
            for index in 0..reservation_count {
                fs::remove_file(repository.path().join(format!("retention-trace-{index}")))
                    .expect("discard setup dirt before automatic settlement");
            }
            let observed = run_berth(repository.path(), &["board", "--json"]);
            assert!(observed.status.success());
            let transaction =
                reservation_ids
                    .iter()
                    .fold(String::new(), |mut transaction, reservation_id| {
                        let _ = writeln!(
                            transaction,
                            "update refs/cargo-berth/reservations/{reservation_id} {protected_tip}"
                        );
                        transaction
                    });
            apply_test_ref_transaction(repository.path(), &transaction);
        },
    }
    let sentinel_log = install_repository_root_reference_transaction_sentinel(repository.path());
    let (output, invocations) =
        run_berth_with_raw_git_trace(repository.path(), &["board", "--json"]);
    assert!(
        output.status.success(),
        "traced reconciliation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    for reservation_id in &reservation_ids {
        assert_eq!(
            reference_exists(
                repository.path(),
                &format!("refs/cargo-berth/reservations/{reservation_id}")
            ),
            matches!(pass, RetentionRefPass::RepairOnly),
        );
    }
    assert!(!sentinel_log.exists());
    invocations
}

fn checkpointed_reservations(repository_root: &Path, count: usize) -> Vec<String> {
    let reservation_ids = (0..count)
        .map(|index| {
            let claimed = claim(
                repository_root,
                &format!("file:retention-trace-{index}"),
                FIRST_RUN,
                "docs/retention-trace.md",
                "retention trace",
            );
            assert!(claimed.status.success());
            reservation_id(&claimed)
        })
        .collect::<Vec<_>>();
    // Keep every checkpoint outstanding until the repair/deletion case chooses its lifecycle.
    for index in 0..count {
        fs::write(
            repository_root.join(format!("retention-trace-{index}")),
            "unmerged work\n",
        )
        .expect("reserved dirty work should write");
    }
    for reservation_id in &reservation_ids {
        let checkpointed = run_berth(repository_root, &["release", reservation_id, "--json"]);
        assert!(checkpointed.status.success());
        assert_eq!(
            json_output(&checkpointed)["payload"]["data"]["status"],
            "checkpointed"
        );
    }
    reservation_ids
}

fn apply_test_ref_transaction(repository_root: &Path, input: &str) {
    let mut child = git_command(BERTH_EXECUTABLE)
        .arg("--no-optional-locks")
        .args(["-c", "core.hooksPath=/dev/null", "update-ref", "--stdin"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("test ref transaction should start");
    child
        .stdin
        .take()
        .expect("test ref transaction stdin should exist")
        .write_all(input.as_bytes())
        .expect("test ref transaction stdin should write");
    let output = child
        .wait_with_output()
        .expect("test ref transaction should finish");
    assert!(
        output.status.success(),
        "test ref transaction failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_one_suppressed_ref_transaction(invocations: &[RawGitInvocation]) {
    let transactions = invocations
        .iter()
        .filter(|invocation| {
            invocation
                .arguments
                .iter()
                .any(|argument| argument == "update-ref")
        })
        .collect::<Vec<_>>();
    assert_eq!(transactions.len(), 1, "raw git trace: {invocations:#?}");
    assert_eq!(
        transactions[0].arguments,
        ["git", "--no-optional-locks", "update-ref", "--stdin"]
    );
}

fn assert_same_git_invocation_sequence(left: &[RawGitInvocation], right: &[RawGitInvocation]) {
    assert_eq!(left.len(), right.len(), "left={left:#?}\nright={right:#?}");
    for (left_invocation, right_invocation) in normalized_git_invocations(left)
        .into_iter()
        .zip(normalized_git_invocations(right))
    {
        assert_eq!(
            left_invocation.arguments.len(),
            right_invocation.arguments.len(),
            "left={left:#?}\nright={right:#?}"
        );
        for (left_argument, right_argument) in left_invocation
            .arguments
            .iter()
            .zip(&right_invocation.arguments)
        {
            assert!(
                left_argument == right_argument
                    || (is_full_git_object_id(left_argument)
                        && is_full_git_object_id(right_argument)),
                "left={left:#?}\nright={right:#?}"
            );
        }
    }
}

fn normalized_git_invocations(invocations: &[RawGitInvocation]) -> Vec<&RawGitInvocation> {
    let mut invocations = invocations.iter().collect::<Vec<_>>();
    let concurrent_startup_reads = invocations
        .iter()
        .take(3)
        .take_while(|invocation| {
            matches!(
                raw_git_command(invocation),
                Some("cat-file" | "rev-list" | "status" | "worktree")
            )
        })
        .count();
    if concurrent_startup_reads >= 2 {
        invocations[..concurrent_startup_reads]
            .sort_by(|left, right| left.arguments.cmp(&right.arguments));
    }
    invocations
}

fn assert_same_git_process_multiset(left: &[RawGitInvocation], right: &[RawGitInvocation]) {
    assert_eq!(left.len(), right.len(), "left={left:#?}\nright={right:#?}");
    assert_eq!(
        canonical_git_command_multiset(left),
        canonical_git_command_multiset(right),
        "left={left:#?}\nright={right:#?}"
    );
}

fn canonical_git_command_multiset(invocations: &[RawGitInvocation]) -> Vec<String> {
    let mut sequence = raw_git_command_sequence(invocations);
    sequence.sort();
    sequence
}

fn git_command_count(invocations: &[RawGitInvocation], command: &str) -> usize {
    invocations
        .iter()
        .filter(|invocation| raw_git_command(invocation) == Some(command))
        .count()
}

fn raw_git_command_sequence(invocations: &[RawGitInvocation]) -> Vec<String> {
    invocations
        .iter()
        .filter_map(raw_git_command)
        .map(str::to_owned)
        .collect()
}

fn raw_git_command(invocation: &RawGitInvocation) -> Option<&str> {
    let mut arguments = invocation.arguments.iter().skip(1);
    loop {
        match arguments.next().map(String::as_str) {
            Some("--no-optional-locks") => {},
            Some("-c") => {
                let _ = arguments.next();
            },
            command => return command,
        }
    }
}

fn is_full_git_object_id(argument: &str) -> bool {
    matches!(argument.len(), 40 | 64) && argument.bytes().all(|byte| byte.is_ascii_hexdigit())
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
) {
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
    let mut command = git_command(BERTH_EXECUTABLE);
    command.arg("--no-optional-locks");
    command
        .args(["rebase", "main"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT);
    if matches!(hook_mode, RebaseHookMode::FilteredBypass) {
        command.env(BYPASS_ENVIRONMENT, "1");
    }
    let rebased = command.output().expect("three-commit rebase should run");
    assert!(
        rebased.status.success(),
        "three-commit rebase failed: {}",
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
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    berth_command()
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn assert_integration_identity_rejection(
    envelope: &serde_json::Value,
    expected_kind: &str,
    expected_action_kinds: &[&str],
) {
    assert_eq!(envelope["payload"]["kind"], "integrate");
    assert_eq!(envelope["payload"]["data"]["status"], "rejected");
    assert_eq!(envelope["payload"]["data"]["reason"]["kind"], expected_kind);
    let actions = envelope["payload"]["data"]["reason"]["recovery_actions"]
        .as_array()
        .expect("identity rejection should carry recovery actions");
    assert!(!actions.is_empty());
    assert_eq!(
        actions
            .iter()
            .map(|action| action["kind"].as_str().expect("action kind should be text"))
            .collect::<Vec<_>>(),
        expected_action_kinds
    );
    for action in actions {
        assert!(
            action["argv"]
                .as_array()
                .is_some_and(|argv| !argv.is_empty())
        );
        assert!(
            action["cwd"]
                .as_str()
                .is_some_and(|cwd| Path::new(cwd).is_absolute())
        );
    }
}

fn run_berth_with_session_and_run(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
    coordination_run_id: &str,
) -> Output {
    berth_command()
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env(RUN_ENVIRONMENT, coordination_run_id)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn git_output(repository_root: &Path, arguments: &[&str]) -> Output {
    GIT.output(repository_root, arguments)
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

fn assert_enrollment_gate_decision(
    root: &Path,
    mode: &str,
    base: &str,
    integrated: &Output,
    other_reservation: &str,
) {
    let envelope = json_output(integrated);
    let data = &envelope["payload"]["data"];
    if mode == "observe" {
        assert!(integrated.status.success(), "{envelope}");
        assert_eq!(data["gate"]["kind"], "observed");
        assert_eq!(
            data["gate"]["violations"][0]["holds"][0]["kind"],
            "deferred_overlap"
        );
        assert_ne!(git_stdout(root, &["rev-parse", "main"]), base);
    } else {
        assert_eq!(integrated.status.code(), Some(2), "{envelope}");
        assert_eq!(envelope["status"], "blocked_by_ordering");
        assert_eq!(
            data["violations"][0]["holds"][0]["kind"],
            "deferred_overlap"
        );
        assert_eq!(
            envelope["blocked_by"],
            serde_json::json!([other_reservation])
        );
        assert_eq!(git_stdout(root, &["rev-parse", "main"]), base);
    }
}
