#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for worktree liveness, recovery, marker sweeping, and cache repair.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tempfile::tempdir;

const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const PAUSE_MODE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_PAUSE";
const PAUSE_MODE_HEAD: &str = "head";
const PAUSE_MODE_MERGE_BASE: &str = "merge-base";
const PAUSED_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$CARGO_BERTH_TEST_GIT_PAUSE" = "merge-base" ] && [ "$1" = "--no-optional-locks" ] && [ "$2" = "merge-base" ] && [ "$3" = "--is-ancestor" ]; then
    : > "$CARGO_BERTH_TEST_GIT_READY"
    while [ ! -e "$CARGO_BERTH_TEST_GIT_CONTINUE" ]; do
        sleep 0.01
    done
fi
if [ "$CARGO_BERTH_TEST_GIT_PAUSE" = "head" ] && [ "$1" = "--no-optional-locks" ] && [ "$2" = "rev-parse" ] && [ "$3" = "HEAD" ]; then
    : > "$CARGO_BERTH_TEST_GIT_READY"
    while [ ! -e "$CARGO_BERTH_TEST_GIT_CONTINUE" ]; do
        sleep 0.01
    done
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const PAUSED_GIT_WRAPPER_TIMEOUT: Duration = Duration::from_secs(60);
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";

struct PausedBerthProcess {
    child:             Child,
    continue_path:     PathBuf,
    ready_path:        PathBuf,
    wrapper_directory: TempDir,
}

impl PausedBerthProcess {
    fn spawn(repository_root: &Path, arguments: &[&str], pause_mode: &str) -> Self {
        let wrapper_directory = tempdir().expect("git wrapper directory should exist");
        let wrapper_path = wrapper_directory.path().join("git");
        fs::write(&wrapper_path, PAUSED_GIT_WRAPPER).expect("git wrapper should write");
        let mut permissions = fs::metadata(&wrapper_path)
            .expect("git wrapper metadata should read")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
        let ready_path = wrapper_directory.path().join("ready");
        let continue_path = wrapper_directory.path().join("continue");
        let original_path = std::env::var_os("PATH").expect("test PATH should exist");
        let wrapped_path = std::env::join_paths(
            std::iter::once(wrapper_directory.path().to_path_buf())
                .chain(std::env::split_paths(&original_path)),
        )
        .expect("wrapped PATH should join");
        let child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
            .args(arguments)
            .current_dir(repository_root)
            .env("PATH", wrapped_path)
            .env(PAUSE_MODE_ENVIRONMENT, pause_mode)
            .env("CARGO_BERTH_TEST_GIT_READY", &ready_path)
            .env("CARGO_BERTH_TEST_GIT_CONTINUE", &continue_path)
            .env("CARGO_BERTH_TEST_REAL_GIT", git_binary())
            .env_remove(RUN_ENVIRONMENT)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("cargo-berth should start with paused git");
        Self {
            child,
            continue_path,
            ready_path,
            wrapper_directory,
        }
    }

    fn wait_until_paused(&mut self) { wait_for_path(&self.ready_path, &mut self.child); }

    fn continue_and_wait(self) -> Output {
        let Self {
            child,
            continue_path,
            ready_path: _,
            wrapper_directory,
        } = self;
        fs::write(continue_path, b"continue\n").expect("paused git should continue");
        let output = child
            .wait_with_output()
            .expect("cargo-berth should finish after git continues");
        std::mem::drop(wrapper_directory);
        output
    }
}

#[test]
fn prunable_and_pruned_worktrees_keep_blocking_and_pruned_work_reports_recovery() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let phase_worktree = worktree_parent.path().join("phase");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "phase",
            phase_worktree
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );
    commit_file(&phase_worktree, "src/lib.rs", "phase work\n", "phase work");
    let claim = run_berth(
        &phase_worktree,
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(
        claim.status.success(),
        "claim failed: stdout={} stderr={}",
        String::from_utf8_lossy(&claim.stdout),
        String::from_utf8_lossy(&claim.stderr)
    );
    let reservation_id = reservation_id(&claim);
    let checkpoint = run_berth(&phase_worktree, &["release", &reservation_id, "--json"]);
    assert!(checkpoint.status.success());
    let protected_tip = json_output(&checkpoint)["payload"]["data"]["protected_tip"]
        .as_str()
        .expect("checkpoint should report a protected tip")
        .to_owned();

    fs::remove_dir_all(&phase_worktree).expect("worktree directory should be removed");
    let orphan_candidate = run_berth(repository.path(), &["check", "file:src/lib.rs", "--json"]);
    assert_eq!(orphan_candidate.status.code(), Some(1));
    assert!(
        json_output(&orphan_candidate)["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );

    git(repository.path(), &["worktree", "prune", "--expire", "now"]);
    assert_orphan_recovery_evidence(repository.path(), &reservation_id, &protected_tip);

    let replacement_worktree = worktree_parent.path().join("replacement");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "recovered",
            replacement_worktree
                .to_str()
                .expect("replacement path should be UTF-8"),
        ],
    );
    let recovered = run_berth(
        &replacement_worktree,
        &["resolve", &reservation_id, "--recovered", "--json"],
    );
    let recovered_json = json_output(&recovered);
    assert!(recovered.status.success());
    assert_eq!(recovered_json["status"], "recovered");
    assert!(
        recovered_json["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    let renewed = run_berth(&replacement_worktree, &["renew", &reservation_id, "--json"]);
    assert!(renewed.status.success());
    assert!(
        json_output(&renewed)["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
}

fn assert_orphan_recovery_evidence(repository: &Path, reservation_id: &str, protected_tip: &str) {
    let orphaned = run_berth(repository, &["check", "file:src/lib.rs", "--json"]);
    let orphaned_json = json_output(&orphaned);
    assert_eq!(orphaned.status.code(), Some(1));
    assert_eq!(
        orphaned_json["payload"]["alerts"][0]["kind"],
        "orphaned_outstanding"
    );
    assert_eq!(
        orphaned_json["payload"]["alerts"][0]["data"]["recoverability"],
        "recoverable_from_branch"
    );

    git(
        repository,
        &["update-ref", "refs/heads/phase", "refs/heads/main"],
    );
    let protected = run_berth(repository, &["check", "file:src/lib.rs", "--json"]);
    assert_eq!(
        json_output(&protected)["payload"]["alerts"][0]["data"]["branch_ref_status"]["status"],
        "present"
    );
    assert_eq!(
        json_output(&protected)["payload"]["alerts"][0]["data"]["recoverability"],
        "recoverable_from_protected_tip"
    );

    remove_protected_tip_recovery_sources(repository, reservation_id, protected_tip);
    let unavailable = run_berth(repository, &["check", "file:src/lib.rs", "--json"]);
    assert_eq!(
        json_output(&unavailable)["payload"]["alerts"][0]["data"]["branch_ref_status"]["status"],
        "present"
    );
    assert_eq!(
        json_output(&unavailable)["payload"]["alerts"][0]["data"]["recoverability"],
        "commit_unavailable"
    );
}

fn remove_protected_tip_recovery_sources(
    repository: &Path,
    reservation_id: &str,
    protected_tip: &str,
) {
    git(
        repository,
        &[
            "update-ref",
            "-d",
            &format!("refs/cargo-berth/reservations/{reservation_id}"),
        ],
    );
    let (object_directory, object_file) = protected_tip.split_at(2);
    fs::remove_file(
        repository
            .join(".git/objects")
            .join(object_directory)
            .join(object_file),
    )
    .expect("protected commit object should delete");
}

#[test]
fn moved_worktree_keeps_its_identity_and_updates_its_recorded_root() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let original_worktree = worktree_parent.path().join("original");
    let moved_worktree = worktree_parent.path().join("moved");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "moved-phase",
            original_worktree
                .to_str()
                .expect("original path should be UTF-8"),
        ],
    );
    let claim = run_berth(
        &original_worktree,
        &["claim", "file:moved", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&claim);
    git(
        repository.path(),
        &[
            "worktree",
            "move",
            original_worktree
                .to_str()
                .expect("original path should be UTF-8"),
            moved_worktree.to_str().expect("moved path should be UTF-8"),
        ],
    );

    assert!(
        run_berth(&moved_worktree, &["renew", &reservation_id, "--json"])
            .status
            .success()
    );
    assert_eq!(
        run_berth(&moved_worktree, &["check", "file:moved", "--json"])
            .status
            .code(),
        Some(0)
    );
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should read after relocation");
    assert!(journal.contains("\"op\":\"relocate_worktree\""));
    assert!(journal.contains(moved_worktree.to_str().expect("moved path should be UTF-8")));
}

#[test]
fn nul_porcelain_preserves_a_worktree_root_that_git_would_quote() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let unusual_worktree = worktree_parent.path().join("phase\n\"backslash\\café");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "unusual-path-phase",
            unusual_worktree
                .to_str()
                .expect("unusual worktree path should be UTF-8"),
        ],
    );
    commit_file(
        &unusual_worktree,
        "unusual",
        "unusual path work\n",
        "unusual path work",
    );
    let claim = run_berth(
        &unusual_worktree,
        &["claim", "file:unusual", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(&unusual_worktree, &["release", &reservation_id, "--json"])
            .status
            .success()
    );

    let blocked = run_berth(repository.path(), &["check", "file:unusual", "--json"]);

    assert_eq!(blocked.status.code(), Some(1));
    assert!(
        json_output(&blocked)["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
}

#[test]
fn locked_missing_worktree_keeps_blocking_without_an_orphan_alert() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let locked_worktree = worktree_parent.path().join("locked");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "locked-phase",
            locked_worktree
                .to_str()
                .expect("locked path should be UTF-8"),
        ],
    );
    assert!(
        run_berth(
            &locked_worktree,
            &["claim", "tree:locked", "--run", FIRST_RUN, "--json"],
        )
        .status
        .success()
    );
    git(
        repository.path(),
        &[
            "worktree",
            "lock",
            locked_worktree
                .to_str()
                .expect("locked path should be UTF-8"),
        ],
    );
    fs::remove_dir_all(&locked_worktree).expect("locked worktree should be removable");

    let blocked = run_berth(repository.path(), &["check", "file:locked/file", "--json"]);
    assert_eq!(blocked.status.code(), Some(1));
    assert!(
        json_output(&blocked)["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
}

#[test]
fn locked_accessible_worktree_has_its_stale_marker_swept() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let locked_worktree = worktree_parent.path().join("locked-accessible");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "locked-accessible-phase",
            locked_worktree
                .to_str()
                .expect("locked path should be UTF-8"),
        ],
    );
    commit_file(&locked_worktree, "locked", "locked work\n", "locked work");
    let claim = run_berth(
        &locked_worktree,
        &["claim", "file:locked", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(&locked_worktree, &["release", &reservation_id, "--json"])
            .status
            .success()
    );
    let marker_path =
        linked_worktree_administrative_directory(&locked_worktree).join("cargo-berth-run-id");
    fs::write(&marker_path, format!("{FIRST_RUN}\n"))
        .expect("stale locked-worktree marker should write");
    git(
        repository.path(),
        &[
            "worktree",
            "lock",
            locked_worktree
                .to_str()
                .expect("locked path should be UTF-8"),
        ],
    );

    let blocked = run_berth(repository.path(), &["check", "file:locked", "--json"]);

    assert_eq!(blocked.status.code(), Some(1));
    assert!(!marker_path.exists());
    assert!(
        json_output(&blocked)["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
}

#[test]
fn journal_from_another_repository_is_rejected_before_projection_repair() {
    let source = initialized_repository();
    assert!(
        run_berth(
            source.path(),
            &["claim", "file:foreign", "--run", FIRST_RUN, "--json"],
        )
        .status
        .success()
    );
    let destination = initialized_repository();
    fs::copy(
        source.path().join(JOURNAL_PATH),
        destination.path().join(JOURNAL_PATH),
    )
    .expect("foreign journal should copy");
    fs::remove_file(destination.path().join(PROJECTION_PATH))
        .expect("destination projection should delete");

    let refused = run_berth(destination.path(), &["check", "file:foreign", "--json"]);
    assert_eq!(refused.status.code(), Some(4));
    assert!(
        json_output(&refused)["message"]
            .as_str()
            .is_some_and(|message| message.contains("different repository instance"))
    );
    assert_eq!(
        run_berth(
            destination.path(),
            &["init", "--repair-projection", "--json"],
        )
        .status
        .code(),
        Some(4)
    );
}

#[test]
fn foreign_administrative_directory_is_refused_without_sweeping_its_marker() {
    let repository = initialized_repository();
    let foreign_repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let worktree = worktree_parent.path().join("local");
    let foreign_worktree = worktree_parent.path().join("foreign");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "local-phase",
            worktree.to_str().expect("local path should be UTF-8"),
        ],
    );
    git(
        foreign_repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "foreign-phase",
            foreign_worktree
                .to_str()
                .expect("foreign path should be UTF-8"),
        ],
    );
    assert!(
        run_berth(
            &worktree,
            &["claim", "file:local", "--run", FIRST_RUN, "--json"],
        )
        .status
        .success()
    );
    assert!(
        run_berth(
            &foreign_worktree,
            &["claim", "file:foreign", "--run", SECOND_RUN, "--json"],
        )
        .status
        .success()
    );
    let foreign_dot_git = fs::read_to_string(foreign_worktree.join(".git"))
        .expect("foreign administrative pointer should read");
    let foreign_admin = Path::new(
        foreign_dot_git
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree should name its git directory"),
    );
    let foreign_marker = foreign_admin.join("cargo-berth-run-id");
    let marker_before = fs::read(&foreign_marker).expect("foreign marker should read");
    fs::write(worktree.join(".git"), foreign_dot_git)
        .expect("local administrative pointer should be replaceable");

    let blocked = run_berth(repository.path(), &["check", "file:local", "--json"]);
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(
        fs::read(foreign_marker).expect("foreign marker should remain"),
        marker_before
    );
    assert!(
        !fs::read_to_string(repository.path().join(JOURNAL_PATH))
            .expect("local journal should read")
            .contains("\"op\":\"relocate_worktree\"")
    );
}

#[test]
fn reconciliation_repairs_a_retention_ref_after_the_checkpoint_append_survives_failure() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "retention"]);
    commit_file(repository.path(), "retained", "work\n", "retained work");
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let claim = run_berth(
        repository.path(),
        &["claim", "file:retained", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&claim);
    let ref_namespace_blocker = repository.path().join(".git/refs/cargo-berth");
    fs::write(&ref_namespace_blocker, "blocks ref directory\n")
        .expect("ref namespace blocker should write");

    let failed_checkpoint = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(failed_checkpoint.status.code(), Some(4));
    assert!(
        fs::read_to_string(repository.path().join(JOURNAL_PATH))
            .expect("journal should retain the checkpoint")
            .contains("\"op\":\"checkpoint\"")
    );
    fs::remove_file(ref_namespace_blocker).expect("ref namespace blocker should delete");
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("failed checkpoint marker should remove for retention repair check");

    let blocked = run_berth(repository.path(), &["check", "file:retained", "--json"]);
    assert_eq!(blocked.status.code(), Some(1));
    let retention_ref = format!("refs/cargo-berth/reservations/{reservation_id}");
    assert_eq!(
        git_stdout(repository.path(), &["rev-parse", &retention_ref]),
        protected_tip
    );
    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    assert!(run_berth(repository.path(), &["init"]).status.success());
    assert_eq!(
        git_stdout(repository.path(), &["rev-parse", &retention_ref]),
        protected_tip
    );
}

#[test]
fn terminal_release_omits_the_orphan_alert_it_resolves() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let phase_worktree = worktree_parent.path().join("release-alert");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "release-alert-phase",
            phase_worktree
                .to_str()
                .expect("phase worktree path should be UTF-8"),
        ],
    );
    commit_file(
        &phase_worktree,
        "release-alert",
        "integrated work\n",
        "release alert work",
    );
    let claim = run_berth(
        &phase_worktree,
        &["claim", "file:release-alert", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(&phase_worktree, &["release", &reservation_id, "--json"])
            .status
            .success()
    );
    fs::remove_dir_all(&phase_worktree).expect("phase worktree should be removed");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);
    git(
        repository.path(),
        &["merge", "--quiet", "--ff-only", "release-alert-phase"],
    );

    let evidence = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(json_output(&evidence)["status"], "integrated");
    assert_eq!(
        json_output(&evidence)["payload"]["alerts"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let released = run_berth(repository.path(), &["release", &reservation_id, "--json"]);

    assert!(released.status.success());
    assert_eq!(
        json_output(&released)["payload"]["data"]["status"],
        "released"
    );
    assert!(
        json_output(&released)["payload"]["alerts"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
}

#[test]
fn stale_markers_cannot_authorize_checks_or_seed_new_claims() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(repository.path(), "a", "phase\n", "phase work");
    let first_claim = run_berth(
        repository.path(),
        &["claim", "file:a", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&first_claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    fs::write(
        repository.path().join(MARKER_PATH),
        format!("{FIRST_RUN}\n"),
    )
    .expect("stale marker should write");

    let blocked = run_berth(repository.path(), &["check", "file:a", "--json"]);
    assert_eq!(blocked.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&blocked),
        "stale_marker_run",
        &["reconcile_and_sweep_marker"],
    );

    fs::write(
        repository.path().join(MARKER_PATH),
        format!("{FIRST_RUN}\n"),
    )
    .expect("stale marker should rewrite");
    let second_claim = run_berth(repository.path(), &["claim", "file:b", "--json"]);
    assert!(second_claim.status.success());
    assert_ne!(
        json_output(&second_claim)["payload"]["data"]["coordination_run_id"],
        FIRST_RUN
    );
}

#[test]
fn check_rejects_stale_and_foreign_session_mappings() {
    let stale_repository = initialized_repository();
    let stale_session = "stale-check-session";
    let mapped_claim = run_berth_with_session(
        stale_repository.path(),
        &["claim", "file:stale-mapped", "--run", FIRST_RUN, "--json"],
        stale_session,
    );
    assert!(mapped_claim.status.success());
    let mapped_reservation_id = reservation_id(&mapped_claim);
    let mapping_path = stale_repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(
            stale_repository.path(),
            &["release", &mapped_reservation_id, "--json"],
        )
        .status
        .success()
    );
    fs::write(&mapping_path, stale_mapping).expect("stale mapping should write");
    let stale_rejection = run_berth_with_session(
        stale_repository.path(),
        &["check", "file:stale-mapped", "--json"],
        stale_session,
    );
    assert_eq!(stale_rejection.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&stale_rejection),
        "stale_session_mapping",
        &["clear_session_mapping"],
    );

    let mismatch_repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let second_root = worktree_parent.path().join("check-second");
    git(
        mismatch_repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "check-second",
            second_root
                .to_str()
                .expect("second worktree path should be UTF-8"),
        ],
    );
    let mismatch_session = "foreign-check-session";
    let live_claim = run_berth_with_session(
        mismatch_repository.path(),
        &["claim", "file:live-mapped", "--run", SECOND_RUN, "--json"],
        mismatch_session,
    );
    assert!(live_claim.status.success());
    let mismatch_rejection = run_berth_with_session(
        &second_root,
        &["check", "file:live-mapped", "--json"],
        mismatch_session,
    );
    assert_eq!(mismatch_rejection.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&mismatch_rejection),
        "session_worktree_mismatch",
        &["rerun_from_holding_worktree", "claim_separately_here"],
    );
}

#[test]
fn marker_derived_claim_revalidates_its_run_after_reconciliation() {
    let repository = initialized_repository();
    let first_claim = run_berth(
        repository.path(),
        &["claim", "file:first", "--run", FIRST_RUN, "--json"],
    );
    let reservation_id = reservation_id(&first_claim);
    let mut pending_claim = PausedBerthProcess::spawn(
        repository.path(),
        &["claim", "file:second", "--json"],
        PAUSE_MODE_HEAD,
    );
    pending_claim.wait_until_paused();
    assert!(
        run_berth(repository.path(), &["release", &reservation_id, "--json"])
            .status
            .success()
    );

    let rejected_claim = pending_claim.continue_and_wait();

    assert_eq!(rejected_claim.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&rejected_claim),
        "stale_marker_run",
        &["reconcile_and_sweep_marker"],
    );
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should read after rejected claim");
    assert_eq!(journal.matches("\"op\":\"claim\"").count(), 1);
}

#[test]
fn rewritten_integration_reachability_runs_under_the_mutation_lock() {
    let repository = initialized_repository();
    let claim = run_berth(
        repository.path(),
        &[
            "claim",
            "file:locked-validation",
            "--run",
            FIRST_RUN,
            "--json",
        ],
    );
    let reservation_id = reservation_id(&claim);
    let trunk_oid = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let mut resolution = PausedBerthProcess::spawn(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--integrated-as",
            &trunk_oid,
            "--json",
        ],
        PAUSE_MODE_MERGE_BASE,
    );
    resolution.wait_until_paused();

    let contender = run_berth(
        repository.path(),
        &["claim", "file:contender", "--run", SECOND_RUN, "--json"],
    );

    assert_eq!(contender.status.code(), Some(6));
    assert_eq!(json_output(&contender)["status"], "contention");
    let resolution = resolution.continue_and_wait();
    assert_eq!(resolution.status.code(), Some(5));
    assert!(
        json_output(&resolution)["message"]
            .as_str()
            .is_some_and(|message| {
                message.contains("protected checkpoint") && message.contains("cargo-berth release")
            })
    );
}

#[test]
fn identity_clear_session_reports_mutation_lock_contention() {
    let repository = initialized_repository();
    let session_id = "contended-clear-session";
    let claim = run_berth_with_session(
        repository.path(),
        &[
            "claim",
            "file:locked-session-clear",
            "--run",
            FIRST_RUN,
            "--json",
        ],
        session_id,
    );
    let reservation_id = reservation_id(&claim);
    let trunk_oid = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let mut resolution = PausedBerthProcess::spawn(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--integrated-as",
            &trunk_oid,
            "--json",
        ],
        PAUSE_MODE_MERGE_BASE,
    );
    resolution.wait_until_paused();

    let clear_session = run_berth_with_session(
        repository.path(),
        &["identity", "clear-session", "--json"],
        session_id,
    );

    assert_eq!(clear_session.status.code(), Some(6));
    assert_eq!(json_output(&clear_session)["status"], "contention");
    drop(resolution.continue_and_wait());
}

#[test]
fn marker_sweep_restores_the_marker_name_after_metadata_failure() {
    let repository = initialized_repository();
    assert!(
        run_berth(
            repository.path(),
            &["claim", "file:any", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    let marker_path = repository.path().join(MARKER_PATH);
    fs::remove_file(&marker_path).expect("published coordination marker should remove");
    symlink("missing-marker-target", &marker_path)
        .expect("broken coordination marker symlink should be created");

    let failed = run_berth(repository.path(), &["check", "file:any", "--json"]);

    assert_eq!(failed.status.code(), Some(1));
    assert!(
        fs::symlink_metadata(marker_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    );
}

#[test]
fn explicit_projection_repair_recovers_ahead_caches_without_changing_the_journal() {
    let repository = initialized_repository();
    let journal_before =
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should read");
    for field in ["generation", "journal_end_offset"] {
        let projection_path = repository.path().join(PROJECTION_PATH);
        let mut projection: serde_json::Value =
            serde_json::from_slice(&fs::read(&projection_path).expect("projection should read"))
                .expect("projection should parse");
        projection[field] = serde_json::Value::from(9);
        fs::write(
            &projection_path,
            serde_json::to_vec_pretty(&projection).expect("projection should serialize"),
        )
        .expect("ahead projection should write");

        let refused = run_berth(repository.path(), &["init", "--json"]);
        assert_eq!(refused.status.code(), Some(4));
        assert!(
            json_output(&refused)["message"]
                .as_str()
                .is_some_and(|message| message.contains("--repair-projection"))
        );
        let repaired = run_berth(
            repository.path(),
            &["init", "--repair-projection", "--json"],
        );
        assert!(repaired.status.success());
        assert_eq!(
            fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should reread"),
            journal_before
        );
    }
}

#[test]
fn retire_orphan_requires_a_reason_and_excludes_other_dispositions() {
    let repository = initialized_repository();
    let reservation_id = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
    let missing_reason = run_berth(
        repository.path(),
        &["resolve", reservation_id, "--retire-orphan", "--json"],
    );
    let combined = run_berth(
        repository.path(),
        &[
            "resolve",
            reservation_id,
            "--retire-orphan",
            "--why",
            "confirmed",
            "--recovered",
            "--json",
        ],
    );
    assert_eq!(missing_reason.status.code(), Some(5));
    assert_eq!(combined.status.code(), Some(5));
}

#[test]
fn active_orphans_accept_confirmed_terminal_dispositions_without_a_checkpoint() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let abandoned_id = create_active_orphan(
        repository.path(),
        worktree_parent.path(),
        "active-abandoned",
        "file:active-abandoned",
        FIRST_RUN,
    );
    let retired_id = create_active_orphan(
        repository.path(),
        worktree_parent.path(),
        "active-retired",
        "file:active-retired",
        SECOND_RUN,
    );
    assert_eq!(
        run_berth(repository.path(), &["release", &abandoned_id, "--json"])
            .status
            .code(),
        Some(5)
    );

    let abandoned = run_berth(
        repository.path(),
        &[
            "resolve",
            &abandoned_id,
            "--abandon",
            "--why",
            "confirmed active orphan abandonment",
            "--json",
        ],
    );
    let retired = run_berth(
        repository.path(),
        &[
            "resolve",
            &retired_id,
            "--retire-orphan",
            "--why",
            "confirmed active orphan retirement",
            "--json",
        ],
    );

    assert!(abandoned.status.success());
    assert!(retired.status.success());
    assert_eq!(
        json_output(&abandoned)["payload"]["data"]["disposition"]["kind"],
        "abandoned"
    );
    assert_eq!(
        json_output(&retired)["payload"]["data"]["disposition"]["kind"],
        "retired_orphan"
    );
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should read after active orphan retirement");
    assert!(!journal.contains("\"op\":\"checkpoint\""));
    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    assert!(
        run_berth(
            repository.path(),
            &["init", "--repair-projection", "--json"]
        )
        .status
        .success()
    );
    let projection = fs::read_to_string(repository.path().join(PROJECTION_PATH))
        .expect("repaired projection should read");
    assert!(projection.contains("confirmed active orphan abandonment"));
    assert!(projection.contains("confirmed active orphan retirement"));
    assert_eq!(
        run_berth(
            repository.path(),
            &["check", "file:active-abandoned", "--json"]
        )
        .status
        .code(),
        Some(0)
    );
    assert_eq!(
        run_berth(
            repository.path(),
            &["check", "file:active-retired", "--json"]
        )
        .status
        .code(),
        Some(0)
    );
}

#[test]
fn recovery_dispositions_validate_evidence_and_remain_distinct_after_replay() {
    let repository = initialized_repository();
    let (abandoned_id, retired_id) = record_terminal_recovery_dispositions(repository.path());
    validate_rewritten_integration_replacement(repository.path());

    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    assert!(run_berth(repository.path(), &["init"]).status.success());
    let replayed_projection = fs::read_to_string(repository.path().join(PROJECTION_PATH))
        .expect("replayed projection should read");
    assert!(replayed_projection.contains("user confirmed discard"));
    assert!(replayed_projection.contains("\"kind\": \"abandoned\""));
    assert!(replayed_projection.contains("user confirmed retirement"));
    assert!(replayed_projection.contains("\"kind\": \"retired_orphan\""));
    assert_eq!(
        run_berth(repository.path(), &["renew", &abandoned_id, "--json"])
            .status
            .code(),
        Some(5)
    );
    assert_eq!(
        run_berth(repository.path(), &["renew", &retired_id, "--json"])
            .status
            .code(),
        Some(5)
    );
    let trunk_commit = git_stdout(repository.path(), &["rev-parse", "main"]);
    for reservation_id in [&abandoned_id, &retired_id] {
        let rejected = run_berth(
            repository.path(),
            &[
                "resolve",
                reservation_id,
                "--integrated-as",
                &trunk_commit,
                "--json",
            ],
        );
        let rejected_json = json_output(&rejected);

        assert_eq!(rejected.status.code(), Some(5));
        assert_eq!(rejected_json["status"], "invalid_input");
        assert_eq!(
            rejected_json["message"],
            "the reservation is already resolved"
        );
    }
}

#[test]
fn unresolved_trunk_alert_survives_and_defers_integrated_as_recovery() {
    let repository = initialized_repository();
    let claim = run_berth(
        repository.path(),
        &["claim", "file:unknown-trunk", "--run", FIRST_RUN, "--json"],
    );
    assert!(claim.status.success());
    let reservation_id = reservation_id(&claim);
    commit_file(
        repository.path(),
        "unknown-trunk",
        "released work\n",
        "unknown trunk work",
    );
    for (expected_status, expected_fact_status) in [
        ("outstanding", "checkpointed"),
        ("integrated", "evidence_revalidated"),
        ("integrated", "released"),
    ] {
        let release = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
        assert!(release.status.success());
        assert_eq!(json_output(&release)["status"], expected_status);
        assert_eq!(
            json_output(&release)["payload"]["data"]["status"],
            expected_fact_status
        );
    }
    let integrated_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "branch",
            "-m",
            "trunk-unavailable",
        ],
    );

    for _ in 0..2 {
        let board = run_berth(repository.path(), &["board", "--json"]);
        assert!(board.status.success());
        let alert = json_output(&board)["payload"]["data"]["alerts"]["entries"]
            .as_array()
            .and_then(|alerts| {
                alerts.iter().find(|alert| {
                    alert["kind"] == "lost_integration_evidence"
                        && alert["reservation_id"] == reservation_id
                })
            })
            .cloned()
            .expect("the unresolved trunk alert should remain on each board");
        assert_eq!(alert["evidence_status"]["status"], "object_unknown");
        assert_eq!(alert["recovery"]["kind"], "resolve_trunk_first");
    }

    let unavailable = run_berth(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--integrated-as",
            &integrated_tip,
            "--json",
        ],
    );
    assert_eq!(unavailable.status.code(), Some(4));
    assert_eq!(json_output(&unavailable)["status"], "ledger_unreadable");
}

fn record_terminal_recovery_dispositions(repository: &Path) -> (String, String) {
    git(repository, &["switch", "--quiet", "-c", "abandoned"]);
    commit_file(repository, "abandoned", "work\n", "abandoned work");
    let abandoned_claim = run_berth(
        repository,
        &["claim", "file:abandoned", "--run", FIRST_RUN, "--json"],
    );
    let abandoned_id = reservation_id(&abandoned_claim);
    assert!(
        run_berth(repository, &["release", &abandoned_id])
            .status
            .success()
    );
    let abandoned = run_berth(
        repository,
        &[
            "resolve",
            &abandoned_id,
            "--abandon",
            "--why",
            "user confirmed discard",
            "--json",
        ],
    );
    assert_eq!(
        json_output(&abandoned)["payload"]["data"]["disposition"]["kind"],
        "abandoned"
    );

    git(repository, &["switch", "--quiet", "main"]);
    git(repository, &["switch", "--quiet", "-c", "retired"]);
    commit_file(repository, "retired", "work\n", "retired work");
    let retired_claim = run_berth(
        repository,
        &["claim", "file:retired", "--run", FIRST_RUN, "--json"],
    );
    let retired_id = reservation_id(&retired_claim);
    assert!(
        run_berth(repository, &["release", &retired_id])
            .status
            .success()
    );
    let retired = run_berth(
        repository,
        &[
            "resolve",
            &retired_id,
            "--retire-orphan",
            "--why",
            "user confirmed retirement",
            "--json",
        ],
    );
    assert_eq!(
        json_output(&retired)["payload"]["data"]["disposition"]["kind"],
        "retired_orphan"
    );
    (abandoned_id, retired_id)
}

fn validate_rewritten_integration_replacement(repository: &Path) {
    git(repository, &["switch", "--quiet", "main"]);
    git(repository, &["switch", "--quiet", "-c", "rewritten"]);
    commit_file(repository, "rewritten", "work\n", "rewritten source");
    let unreachable_tip = git_stdout(repository, &["rev-parse", "HEAD"]);
    let rewritten_claim = run_berth(
        repository,
        &["claim", "file:rewritten", "--run", FIRST_RUN, "--json"],
    );
    let rewritten_id = reservation_id(&rewritten_claim);
    assert!(
        run_berth(repository, &["release", &rewritten_id])
            .status
            .success()
    );
    git(repository, &["switch", "--quiet", "main"]);
    let rejected = run_berth(
        repository,
        &[
            "resolve",
            &rewritten_id,
            "--integrated-as",
            &unreachable_tip,
            "--json",
        ],
    );
    assert_eq!(rejected.status.code(), Some(5));

    commit_file(
        repository,
        "integrated",
        "rewritten result\n",
        "rewritten integration",
    );
    let first_evidence = git_stdout(repository, &["rev-parse", "HEAD"]);
    let integrated = run_berth(
        repository,
        &[
            "resolve",
            &rewritten_id,
            "--integrated-as",
            &first_evidence,
            "--json",
        ],
    );
    assert!(integrated.status.success());
    assert_eq!(json_output(&integrated)["status"], "integrated");

    git(repository, &["reset", "--hard", "--quiet", "HEAD^"]);
    let reblocked = run_berth(repository, &["release", &rewritten_id, "--json"]);
    assert_eq!(json_output(&reblocked)["status"], "trunk_rewritten");
    assert_eq!(
        json_output(&reblocked)["payload"]["alerts"][0]["kind"],
        "lost_integration_evidence"
    );
    let persisted = run_berth(repository, &["release", &rewritten_id, "--json"]);
    assert_eq!(json_output(&persisted)["status"], "trunk_rewritten");
    assert_eq!(
        json_output(&persisted)["payload"]["alerts"][0]["kind"],
        "lost_integration_evidence"
    );
    commit_file(
        repository,
        "integrated-again",
        "replacement evidence\n",
        "replacement integration evidence",
    );
    let replacement_evidence = git_stdout(repository, &["rev-parse", "HEAD"]);
    let replaced = run_berth(
        repository,
        &[
            "resolve",
            &rewritten_id,
            "--integrated-as",
            &replacement_evidence,
            "--json",
        ],
    );
    assert!(replaced.status.success());
    assert!(
        fs::read_to_string(repository.join(JOURNAL_PATH))
            .expect("journal should read")
            .contains("\"op\":\"replace_release_disposition\"")
    );
    assert_eq!(
        run_berth(repository, &["check", "file:rewritten", "--json"])
            .status
            .code(),
        Some(0)
    );
}

fn create_active_orphan(
    repository: &Path,
    worktree_parent: &Path,
    branch: &str,
    scope: &str,
    run: &str,
) -> String {
    let worktree = worktree_parent.join(branch);
    git(
        repository,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            worktree
                .to_str()
                .expect("active orphan worktree path should be UTF-8"),
        ],
    );
    let claim = run_berth(&worktree, &["claim", scope, "--run", run, "--json"]);
    assert!(claim.status.success());
    let reservation_id = reservation_id(&claim);
    fs::remove_dir_all(&worktree).expect("active orphan worktree should be removed");
    git(repository, &["worktree", "prune", "--expire", "now"]);
    reservation_id
}

fn initialized_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    git(
        repository.path(),
        &["config", "user.email", "berth@example.invalid"],
    );
    fs::write(repository.path().join("README.md"), "scratch repository\n")
        .expect("scratch file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    assert!(run_berth(repository.path(), &["init"]).status.success());
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "track berth config"],
    );
    repository
}

fn reservation_id(claim: &Output) -> String {
    json_output(claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

fn commit_file(repository_root: &Path, path: &str, contents: &str, message: &str) {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("file parent should exist");
    }
    fs::write(file_path, contents).expect("committed file should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
}

fn linked_worktree_administrative_directory(worktree_root: &Path) -> PathBuf {
    let dot_git = fs::read_to_string(worktree_root.join(".git"))
        .expect("linked-worktree administrative pointer should read");
    let administrative_path = PathBuf::from(
        dot_git
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree should name its administrative directory"),
    );
    if administrative_path.is_absolute() {
        administrative_path
    } else {
        worktree_root.join(administrative_path)
    }
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn assert_coordination_identity_rejection(
    envelope: &serde_json::Value,
    expected_kind: &str,
    expected_action_kinds: &[&str],
) {
    assert_eq!(envelope["payload"]["kind"], "coordination_identity");
    assert_eq!(envelope["payload"]["data"]["kind"], expected_kind);
    let actions = envelope["payload"]["data"]["recovery_actions"]
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

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
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

fn git_binary() -> String {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("git lookup should run");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git path should be UTF-8")
        .trim()
        .to_owned()
}

fn wait_for_path(path: &Path, child: &mut Child) {
    let deadline = Instant::now() + PAUSED_GIT_WRAPPER_TIMEOUT;
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !path.exists() {
        assert!(child.kill().is_ok(), "timed-out command should stop");
        assert!(child.wait().is_ok(), "stopped command should be reaped");
    }
    assert!(path.exists(), "git wrapper did not reach its pause point");
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}
