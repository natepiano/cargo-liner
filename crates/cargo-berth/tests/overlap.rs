#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for claim acquisition and mutation-free edit checks.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;
use tempfile::tempdir;

const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

#[test]
fn blocked_claim_names_holder_provenance_and_appends_nothing() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let first_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:crates/hana_kana",
            "--run",
            FIRST_RUN,
            "--plan",
            "docs/holder-plan.md",
            "--phase",
            "holder-phase",
            "--why",
            "holder changes kana internals",
            "--json",
        ],
    );
    assert!(first_claim.status.success());
    let sibling_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:crates/hana_kana_extra",
            "--run",
            SECOND_RUN,
            "--json",
        ],
    );
    assert!(sibling_claim.status.success());
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before rejection");

    let blocked = run_berth(
        repository.path(),
        &[
            "claim",
            "file:crates/hana_kana/src/lib.rs",
            "--run",
            THIRD_RUN,
            "--json",
        ],
    );
    let envelope = json_output(&blocked);

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(envelope["exit_code"], 1);
    assert_eq!(envelope["status"], "blocked_by_overlap");
    assert_eq!(envelope["payload"]["kind"], "claim");
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["head_snapshot"]["full_ref"],
        "refs/heads/main"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["plan"],
        "docs/holder-plan.md"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["phase"],
        "holder-phase"
    );
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("reduce the requested scopes"))
    );
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should read after rejection"),
        journal_before
    );
}

#[test]
fn file_and_tree_scopes_differ_for_descendants() {
    let file_repository = initialized_repository(PathCaseSetting::Sensitive);
    assert!(
        run_berth(
            file_repository.path(),
            &["claim", "file:generated", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    assert!(
        run_berth(
            file_repository.path(),
            &[
                "claim",
                "file:generated/child.rs",
                "--run",
                SECOND_RUN,
                "--json"
            ]
        )
        .status
        .success()
    );

    let tree_repository = initialized_repository(PathCaseSetting::Sensitive);
    assert!(
        run_berth(
            tree_repository.path(),
            &["claim", "tree:generated", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    let blocked = run_berth(
        tree_repository.path(),
        &[
            "claim",
            "file:generated/child.rs",
            "--run",
            SECOND_RUN,
            "--json",
        ],
    );
    assert_eq!(blocked.status.code(), Some(1));
}

#[test]
fn ignore_case_blocks_component_case_variants() {
    let repository = initialized_repository(PathCaseSetting::Insensitive);
    assert!(
        run_berth(
            repository.path(),
            &["claim", "tree:Crates/Hana", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );

    let blocked = run_berth(
        repository.path(),
        &[
            "claim",
            "file:crates/hana/src/lib.rs",
            "--run",
            SECOND_RUN,
            "--json",
        ],
    );

    assert_eq!(blocked.status.code(), Some(1));
}

#[test]
fn check_uses_run_identity_without_git_or_file_mutation() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let claim = run_berth(
        repository.path(),
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(
        claim.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claim.stdout)
    );
    let claim_envelope = json_output(&claim);
    assert_eq!(
        claim_envelope["payload"]["data"]["coordination_run_id"],
        FIRST_RUN
    );
    assert_eq!(
        claim_envelope["payload"]["data"]["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join(MARKER_PATH))
            .expect("coordination marker should read")
            .trim(),
        FIRST_RUN
    );
    let journal_before =
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should read before check");
    let projection_before = fs::read(repository.path().join(PROJECTION_PATH))
        .expect("projection should read before check");
    let lock_before =
        fs::read(repository.path().join(LOCK_PATH)).expect("lock should read before check");
    let empty_path = tempdir().expect("empty PATH directory should exist");

    let own_check = run_check_without_git(repository.path(), empty_path.path(), FIRST_RUN);
    let foreign_check = run_check_without_git(repository.path(), empty_path.path(), SECOND_RUN);

    assert!(own_check.status.success());
    assert_eq!(foreign_check.status.code(), Some(1));
    assert_eq!(json_output(&foreign_check)["status"], "blocked_by_overlap");
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should reread"),
        journal_before
    );
    assert_eq!(
        fs::read(repository.path().join(PROJECTION_PATH)).expect("projection should reread"),
        projection_before
    );
    assert_eq!(
        fs::read(repository.path().join(LOCK_PATH)).expect("lock should reread"),
        lock_before
    );
}

#[test]
fn claims_without_run_continue_the_worktree_coordination_run() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let first_claim = run_berth(repository.path(), &["claim", "tree:crates/a", "--json"]);
    assert!(first_claim.status.success());
    let first_run = fs::read_to_string(repository.path().join(MARKER_PATH))
        .expect("first coordination marker should read");

    let second_claim = run_berth(repository.path(), &["claim", "tree:crates/b", "--json"]);
    assert!(second_claim.status.success());
    assert_eq!(
        fs::read_to_string(repository.path().join(MARKER_PATH))
            .expect("second coordination marker should read"),
        first_run
    );

    let check = run_berth(
        repository.path(),
        &["check", "file:crates/a/x.rs", "--json"],
    );
    assert!(check.status.success());
    assert_eq!(json_output(&check)["status"], "clear");
}

#[test]
fn reconciliation_removes_a_malformed_marker_directory_before_claim() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    fs::create_dir(repository.path().join(MARKER_PATH))
        .expect("marker destination directory should exist");

    let claim = run_berth(
        repository.path(),
        &["claim", "file:src/lib.rs", "--run", FIRST_RUN, "--json"],
    );
    let envelope = json_output(&claim);

    assert!(claim.status.success());
    assert_eq!(envelope["status"], "claimed");
    assert_eq!(
        envelope["payload"]["data"]["coordination_run_id"],
        FIRST_RUN
    );
    assert_eq!(
        envelope["payload"]["data"]["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join(JOURNAL_PATH))
            .expect("journal should contain the committed claim")
            .lines()
            .count(),
        1
    );
}

#[test]
fn blocked_message_names_every_holder() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    for scope in ["tree:src", "file:src/lib.rs"] {
        assert!(
            run_berth(
                repository.path(),
                &["claim", scope, "--run", FIRST_RUN, "--json"]
            )
            .status
            .success()
        );
    }

    let blocked = run_berth(
        repository.path(),
        &["claim", "file:src/lib.rs", "--run", SECOND_RUN, "--json"],
    );

    assert_eq!(blocked.status.code(), Some(1));
    let envelope = json_output(&blocked);
    let message = envelope["message"]
        .as_str()
        .expect("blocked message should be text");
    let conflicts = envelope["payload"]["data"]["conflicts"]
        .as_array()
        .expect("blocked payload should list conflicts");
    assert_eq!(conflicts.len(), 2);
    assert!(message.contains("2 reservations hold overlapping paths"));
    for conflict in conflicts {
        let reservation_id = conflict["reservation_id"]
            .as_str()
            .expect("conflict should name its reservation");
        assert!(message.contains(reservation_id));
    }
}

#[test]
fn future_paths_succeed_invalid_paths_do_not_append_and_missing_why_is_typed() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let future_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "file:future/generated.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
    );
    assert!(future_claim.status.success());
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should read after future claim");
    let journal_event: serde_json::Value =
        serde_json::from_str(journal.lines().next().expect("claim record should exist"))
            .expect("claim record should decode");
    assert_eq!(journal_event["purpose"]["kind"], "not_provided_by_caller");
    let journal_before_rejections = journal.into_bytes();

    for invalid_path in ["file:../outside.rs", "file:/absolute.rs"] {
        let rejected = run_berth(
            repository.path(),
            &["claim", invalid_path, "--run", SECOND_RUN, "--json"],
        );
        assert_eq!(rejected.status.code(), Some(5));
        assert!(
            json_output(&rejected)["message"]
                .as_str()
                .is_some_and(|message| message.contains("repository-relative path"))
        );
    }
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should reread"),
        journal_before_rejections
    );
}

#[test]
fn unpaired_plan_flags_are_usage_errors_and_rejection_sweeps_stale_marker() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    for arguments in [
        vec!["claim", "file:a", "--plan", "docs/plan.md"],
        vec!["claim", "file:a", "--phase", "phase-a"],
    ] {
        assert_eq!(
            run_berth(repository.path(), &arguments).status.code(),
            Some(5)
        );
    }
    assert!(
        run_berth(
            repository.path(),
            &["claim", "file:Cargo.toml", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    fs::write(
        repository.path().join(MARKER_PATH),
        format!("{THIRD_RUN}\n"),
    )
    .expect("newer marker should write");

    let rejected = run_berth(
        repository.path(),
        &["claim", "file:Cargo.toml", "--run", SECOND_RUN, "--json"],
    );

    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        !repository.path().join(MARKER_PATH).exists(),
        "reconciliation should sweep a marker without a matching active reservation"
    );
}

#[derive(Clone, Copy)]
enum PathCaseSetting {
    Sensitive,
    Insensitive,
}

impl PathCaseSetting {
    const fn git_value(self) -> &'static str {
        match self {
            Self::Sensitive => "false",
            Self::Insensitive => "true",
        }
    }
}

fn initialized_repository(path_case_setting: PathCaseSetting) -> TempDir {
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
    git(
        repository.path(),
        &["config", "core.ignoreCase", path_case_setting.git_value()],
    );
    fs::write(repository.path().join("README.md"), "scratch repository\n")
        .expect("scratch file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth(repository.path(), &["init", "--json"]);
    assert!(initialized.status.success());
    repository
}

fn run_check_without_git(repository_root: &Path, path: &Path, run: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(repository_root)
        .env("PATH", path)
        .env(RUN_ENVIRONMENT, run)
        .output()
        .expect("cargo-berth check should run without git")
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
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

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}
