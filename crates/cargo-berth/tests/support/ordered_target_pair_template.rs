//! Ordered integration lanes recorded before the gate test clocks start.
use std::fs;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::deferring_run_scope;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[
        "CARGO_BERTH_RUN",
        "CARGO_BERTH_SESSION_ID",
        "GIT_DIR",
        "GIT_COMMON_DIR",
    ],
};
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const WITNESSES: &str = ".git/ordered-target-pair.json";

/// Two independent tips with a real deferred claim and sequence edge.
pub(crate) const SEPARATE_TARGET_PAIR: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       BERTH_EXECUTABLE,
        name:             "separate-ordered-target-pair",
        build:            |root| build_pair(root, SuccessorAncestry::Separate),
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// The ordered successor contains its predecessor, which has not reached the target.
pub(crate) const MERGED_TARGET_PAIR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "merged-ordered-target-pair",
        build: |root| build_pair(root, SuccessorAncestry::ContainsPredecessor),
        ..SEPARATE_TARGET_PAIR.repository
    },
    ..SEPARATE_TARGET_PAIR
};

#[derive(Clone, Copy)]
enum SuccessorAncestry {
    Separate,
    ContainsPredecessor,
}

/// Recorded identities and commit witnesses used by the unchanged gate subjects.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct TargetPairWitnesses {
    pub(crate) predecessor_id:     String,
    pub(crate) successor_id:       String,
    pub(crate) integration_before: String,
    pub(crate) predecessor_tip:    String,
    pub(crate) successor_tip:      String,
}

fn build_pair(root: &Path, ancestry: SuccessorAncestry) {
    IntegrationRepository::build_initial_integration_pair(root, BERTH_EXECUTABLE);
    let worktrees = root.join(".git/fixture-worktrees");
    let predecessor = worktrees.join("target-a");
    let successor = worktrees.join("target-b");
    for (checkout, branch) in [(&predecessor, "target-a"), (&successor, "target-b")] {
        GIT.add_worktree_without_hooks(root, checkout, branch, "integration");
        GIT.run(
            root,
            [
                "config",
                &format!("branch.{branch}.cargoBerthTarget"),
                "integration",
            ],
        );
    }
    let integration_before = GIT.stdout(root, ["rev-parse", "integration"]);
    fs::write(predecessor.join("shared.txt"), "A\n").expect("predecessor work should write");
    let (predecessor_id, successor_id) = order_claims(root, &predecessor, &successor);
    for (checkout, contents, message) in [
        (&predecessor, "A\n", "A work"),
        (&successor, "B\n", "B work"),
    ] {
        fs::write(checkout.join("shared.txt"), contents).expect("lane work should write");
        GIT.run(checkout, ["add", "shared.txt"]);
        GIT.run(checkout, ["commit", "--quiet", "-m", message]);
    }
    if matches!(ancestry, SuccessorAncestry::ContainsPredecessor) {
        GIT.run(&successor, ["merge", "-s", "ours", "--no-edit", "target-a"]);
    }
    let witnesses = TargetPairWitnesses {
        predecessor_id,
        successor_id,
        integration_before,
        predecessor_tip: GIT.stdout(&predecessor, ["rev-parse", "HEAD"]),
        successor_tip: GIT.stdout(&successor, ["rev-parse", "HEAD"]),
    };
    fs::write(
        root.join(WITNESSES),
        serde_json::to_vec(&witnesses).expect("target pair witnesses should encode"),
    )
    .expect("target pair witnesses should write");
}

fn order_claims(root: &Path, predecessor: &Path, successor: &Path) -> (String, String) {
    let claimed = run_berth(
        predecessor,
        &[
            "claim",
            "file:shared.txt",
            "--run",
            FIRST_RUN,
            "--json",
            "--target",
            "integration",
        ],
    );
    let predecessor_id = claim_id(&claimed);
    let own_scope = deferring_run_scope(SECOND_RUN);
    drop(run_berth(
        successor,
        &[
            "claim",
            &own_scope,
            "--run",
            SECOND_RUN,
            "--why",
            "protect deferred work",
            "--target",
            "integration",
            "--json",
        ],
    ));
    let deferred = run_berth(
        successor,
        &[
            "claim",
            "file:shared.txt",
            "--run",
            SECOND_RUN,
            "--defer",
            &predecessor_id,
            "--overlap-why",
            "the order needs a decision",
            "--why",
            "protect deferred work",
            "--json",
        ],
    );
    let successor_id = claim_id(&deferred);
    drop(run_berth(
        root,
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "A lands before B",
            "--json",
        ],
    ));
    (predecessor_id, successor_id)
}

fn run_berth(root: &Path, args: &[&str]) -> Output {
    let output = cargo_berth_test_support::berth_command(BERTH_EXECUTABLE)
        .args(args)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("target pair setup command should run");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn claim_id(output: &Output) -> String {
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("claim should print JSON");
    envelope["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should identify its reservation")
        .to_owned()
}
