//! Recorded protected histories before the proof subject changes.

use std::fs;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::claim_id;

use super::edges_templates;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
/// The saved Git and reservation identities.
pub(crate) const STATE: &str = ".git/proof-subject.json";
const PROTECTED_SOURCE: &str = "pub fn protected() {}\n";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};

/// The initial source and berth policy shared by board histories.
pub(crate) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "initialized",
    build:            build_initialized_repository,
};

/// A protected reservation whose amended trunk keeps equivalent source.
pub(crate) const EQUIVALENT_TARGET: LedgerTemplate =
    template("proof-subject-equivalent-target", |root| {
        build_rewritten_target(root, PROTECTED_SOURCE);
    });
/// A protected reservation whose amended trunk has different source.
pub(crate) const DIFFERENT_TARGET: LedgerTemplate =
    template("proof-subject-different-target", |root| {
        build_rewritten_target(root, "pub fn replacement() {}\n");
    });

const fn template(name: &'static str, build: fn(&Path)) -> LedgerTemplate {
    LedgerTemplate {
        repository:   RepositoryTemplate {
            name,
            build,
            ..INITIALIZED_REPOSITORY
        },
        ledger:       LedgerFixture::Recorded,
        build_timing: TemplateBuildTiming::BeforeRun,
    }
}

/// The Git and reservation identities that the live proof checks use.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct RewrittenTargetState {
    pub(crate) reservation_id:   String,
    pub(crate) phase_start_head: String,
    pub(crate) protected_tip:    String,
    pub(crate) target:           String,
}

fn build_rewritten_target(root: &Path, source: &str) {
    INITIALIZED_REPOSITORY.restore_into(root);
    let phase_start_head = GIT.stdout(root, ["rev-parse", "HEAD"]);
    let reservation_id = claim_id(&run_berth(
        root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "protect board test work",
            "--json",
        ],
    ));
    fs::write(root.join("src/lib.rs"), PROTECTED_SOURCE).expect("protected source should write");
    GIT.run(root, ["add", "src/lib.rs"]);
    GIT.run(
        root,
        [
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected identity",
        ],
    );
    let protected_tip = GIT.stdout(root, ["rev-parse", "HEAD"]);
    edges_templates::append_edge_fixture_event(
        root,
        serde_json::json!({
            "op": "checkpoint", "reservation_id": reservation_id,
            "protected_tip": protected_tip, "trunk_snapshot": protected_tip,
        }),
    );
    edges_templates::append_edge_fixture_event(
        root,
        serde_json::json!({
            "op": "evidence_revalidated", "reservation_id": reservation_id,
            "status": {"status": "integrated", "trunk_oid": protected_tip, "proof": "protected_tip_ancestor"},
            "edit_blocking_status": "clear",
        }),
    );
    fs::write(root.join("src/lib.rs"), source).expect("replacement source should write");
    GIT.run(root, ["add", "src/lib.rs"]);
    GIT.run(
        root,
        [
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "rewritten target",
        ],
    );
    let target = GIT.stdout(root, ["rev-parse", "HEAD"]);
    fs::write(
        root.join(STATE),
        serde_json::to_vec(&RewrittenTargetState {
            reservation_id,
            phase_start_head,
            protected_tip,
            target,
        })
        .expect("proof state should encode"),
    )
    .expect("proof state should write");
}

fn build_initialized_repository(repository_root: &Path) {
    GIT.run(
        repository_root,
        ["init", "--quiet", "--initial-branch", "main"],
    );
    GIT.run(
        repository_root,
        ["config", "user.email", "test@example.com"],
    );
    GIT.run(repository_root, ["config", "user.name", "Test User"]);
    fs::create_dir_all(repository_root.join("src")).expect("source directory should exist");
    fs::write(repository_root.join("src/lib.rs"), "pub fn value() {}\n")
        .expect("source should write");
    GIT.run(repository_root, ["add", "."]);
    GIT.run(repository_root, ["commit", "--quiet", "-m", "initial"]);
    assert!(
        run_berth(repository_root, &["init", "--json"])
            .status
            .success()
    );
}

fn run_berth(root: &Path, arguments: &[&str]) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("fixture command should run")
}
