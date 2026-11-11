//! Recorded preparation for projection growth and worktree enrollment checks.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::MAIN_COORDINATION_RUN_ID;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
pub(super) const PROJECTION_SIZE_INITIAL_RENEWALS: usize = 18;
const GIT: GitDriver = GitDriver {
    executable:          EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[
        "CARGO_BERTH_RUN",
        "CARGO_BERTH_SESSION_ID",
        "GIT_DIR",
        "GIT_COMMON_DIR",
    ],
};
pub(super) const PROJECTION_HISTORY_STATE: &str = ".git/projection-history-reservation";

pub(super) const PROJECTION_HISTORY: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       EXECUTABLE,
        name:             "projection-history-seventeen-renewals",
        build:            build_projection_history,
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

fn build_projection_history(root: &Path) {
    build_git_repository(root);
    assert_success(&run(root, &["init", "--json"]));
    GIT.run(root, ["add", "--force", ".claude/config/berth.toml"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "configure berth"]);
    let id = claim(
        root,
        "file:projection-size.rs",
        MAIN_COORDINATION_RUN_ID,
        "projection-size",
    );
    for _ in 1..PROJECTION_SIZE_INITIAL_RENEWALS {
        let renewed = run(root, &["renew", &id, "--json"]);
        assert!(renewed.status.success());
    }
    fs::write(root.join(PROJECTION_HISTORY_STATE), id).expect("reservation id should write");
}

fn run(root: &Path, arguments: &[&str]) -> std::process::Output {
    run_berth_with_session(root, arguments, "projection-size")
}

pub(super) fn build_git_repository(root: &Path) {
    cargo_berth_test_support::build_scratch_repository(root, GIT);
}

/// Three dirty checkouts before init enrolls their shared path.
pub(super) const SHARED_ENROLLMENT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "shared-enrollment-before-init",
        build: |root| build_enrollment(root, EnrollmentPreparation::Shared),
        ..PROJECTION_HISTORY.repository
    },
    ledger: LedgerFixture::Empty,
    ..PROJECTION_HISTORY
};

/// Parent and child branches before init finds their distinct footprints.
pub(super) const STACKED_ENROLLMENT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "stacked-enrollment-before-init",
        build: |root| build_enrollment(root, EnrollmentPreparation::Stacked),
        ..PROJECTION_HISTORY.repository
    },
    ..SHARED_ENROLLMENT
};

/// Committed branch work before init enrolls and release checkpoints it.
pub(super) const RELEASE_ENROLLMENT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "release-enrollment-before-init",
        build: |root| build_enrollment(root, EnrollmentPreparation::Released),
        ..PROJECTION_HISTORY.repository
    },
    ..SHARED_ENROLLMENT
};

#[derive(Clone, Copy)]
enum EnrollmentPreparation {
    Shared,
    Stacked,
    Released,
}

fn build_enrollment(root: &Path, preparation: EnrollmentPreparation) {
    build_git_repository(root);
    let parent = root.join(".git/fixture-worktrees");
    fs::create_dir_all(&parent).expect("worktree parent should exist");
    match preparation {
        EnrollmentPreparation::Shared => {
            for (branch, contents) in [
                ("first", "first committed\n"),
                ("second", "second committed\n"),
            ] {
                let checkout = parent.join(branch);
                GIT.add_worktree_without_hooks(root, &checkout, branch, "main");
                commit_enrollment_work(&checkout, "README.md", contents);
                fs::write(checkout.join("README.md"), format!("{contents}dirty\n"))
                    .expect("shared dirty work should write");
            }
            fs::write(root.join("README.md"), "trunk dirty\n").expect("trunk work should write");
        },
        EnrollmentPreparation::Stacked => {
            let checkout = parent.join("parent");
            GIT.add_worktree_without_hooks(root, &checkout, "parent", "main");
            commit_enrollment_work(&checkout, "parent.txt", "parent work\n");
            let child = parent.join("child");
            GIT.add_worktree_without_hooks(root, &child, "child", "parent");
            commit_enrollment_work(&child, "child.txt", "child work\n");
        },
        EnrollmentPreparation::Released => {
            let checkout = parent.join("released");
            GIT.add_worktree_without_hooks(root, &checkout, "released", "main");
            commit_enrollment_work(&checkout, "README.md", "committed work\n");
        },
    }
}

pub(super) fn commit_enrollment_work(root: &Path, path: &str, contents: &str) {
    fs::write(root.join(path), contents).expect("work should write");
    GIT.run(root, ["add", path]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "work before enrollment"]);
}

pub(super) fn claim(repository_root: &Path, scope: &str, run: &str, session_id: &str) -> String {
    let claimed = run_berth_with_session(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--why",
            "identity fixture",
            "--json",
        ],
        session_id,
    );
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

pub(super) fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> std::process::Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session_id)
        .env_remove("GIT_DIR")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .expect("cargo-berth should run")
}
