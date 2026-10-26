//! Recorded populations for successor board cost and coverage tests.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::CommandDeadlineExt as _;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::claim_id;
use cargo_berth_test_support::write_file;

use super::edges_templates::RECORDED_PROJECTION;
use super::edges_templates::commit_successor_fixture_file;
use super::linked_checkout;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};

/// A released predecessor merged on main, with one unobserved equivalent successor head.
pub(crate) const ONE_SUCCESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "successor-population-1",
        build: |root| build_successor_population(root, 1),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
/// A released predecessor merged on main, with two unobserved equivalent successor heads.
pub(crate) const TWO_SUCCESSORS: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "successor-population-2",
        build: |root| build_successor_population(root, 2),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
/// A released predecessor merged on main, with four unobserved equivalent successor heads.
pub(crate) const FOUR_SUCCESSORS: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "successor-population-4",
        build: |root| build_successor_population(root, 4),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};

fn write_successor_scopes(root: &Path, successor_count: usize) {
    fs::create_dir_all(root.join("successors")).expect("successor scope directory should exist");
    for successor_index in 0..successor_count {
        fs::write(
            root.join(format!("successors/successor-{successor_index}.rs")),
            format!("// reserved successor scope {successor_index}\n"),
        )
        .expect("successor scope source should write");
    }
    GIT.run(root, ["add", "successors"]);
    GIT.run(
        root,
        [
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "successor reservation scopes",
        ],
    );
}

fn build_successor_population(root: &Path, successor_count: usize) {
    IntegrationRepository::build_main_template(root, BERTH_EXECUTABLE);
    write_file(root, "src/lib.rs", "pub fn value() {}\n");
    write_file(root, "tests/base.rs", "// test\n");
    GIT.run_without_hooks(root, ["add", "src", "tests"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "source and test paths"]);
    write_successor_scopes(root, successor_count);
    let worktrees = root.join(".git/fixture-worktrees");
    let predecessor_root = add_worktree(root, &worktrees, "predecessor");
    for index in 0..successor_count {
        write_file(
            &predecessor_root,
            &format!("successors/successor-{index}.rs"),
            "// uncommitted holder work\n",
        );
    }
    let predecessor = run_berth(
        &predecessor_root,
        &[
            "claim",
            "tree:successors",
            "--run",
            FIRST_RUN,
            "--why",
            "protect test work",
            "--json",
        ],
    );
    let predecessor_id = claim_id(&predecessor);
    let predecessor_claim = last_journal_event(root);
    let protected_content = "pub fn protected_predecessor() {}\n";
    // The placeholder dirt establishes overlap during claims, then must not block integration.
    GIT.run(
        &predecessor_root,
        ["restore", "--worktree", "--", "successors"],
    );
    fs::create_dir_all(predecessor_root.join("successors"))
        .expect("predecessor source directory should exist");
    fs::write(
        predecessor_root.join("successors/protected.rs"),
        protected_content,
    )
    .expect("predecessor source should write");
    GIT.run(&predecessor_root, ["add", "successors/protected.rs"]);
    GIT.run(
        &predecessor_root,
        ["commit", "--quiet", "-m", "protected predecessor"],
    );
    assert_success(&run_berth(
        &predecessor_root,
        &["release", &predecessor_id, "--json"],
    ));
    // Finish the predecessor before adding the scale population, so its commit hook and
    // release do not repeatedly observe every successor during setup.
    let mut successor_heads = Vec::new();
    for successor_index in 0..successor_count {
        let branch = format!("successor-{successor_index}");
        let successor_root = add_worktree(root, &worktrees, &branch);
        commit_successor_fixture_file(
            &successor_root,
            "successors/protected.rs",
            protected_content,
            &format!("successor {successor_index}"),
        );
        let head = GIT.stdout(&successor_root, ["rev-parse", "HEAD"]);
        record_scale_successor(root, &successor_root, &branch, &head, &predecessor_claim);
        successor_heads.push(head);
    }

    GIT.run(
        root,
        [
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--quiet",
            "predecessor",
        ],
    );

    fs::write(
        root.join(".git/successor-heads.json"),
        serde_json::to_vec(&successor_heads).expect("successor heads should encode"),
    )
    .expect("successor heads should write");
}

/// Replay setup for the scale tests: each live successor owns its file and one edge to the
/// predecessor. Claim acquisition has separate command tests; these tests measure board reads.
fn record_scale_successor(
    root: &Path,
    successor_root: &Path,
    branch: &str,
    head: &str,
    predecessor: &serde_json::Value,
) {
    let previous = last_journal_event(root);
    let mut event = predecessor.clone();
    let worktree_id = uuid::Uuid::now_v7().to_string();
    let administrative_directory = linked_checkout::administrative_directory(successor_root);
    fs::write(
        administrative_directory.join("cargo-berth-worktree-id"),
        &worktree_id,
    )
    .expect("successor identity should write");
    let scopes = serde_json::json!([{"path": format!("successors/{branch}.rs"), "kind": "file"}]);
    event["actor"]["worktree"] = serde_json::json!(worktree_id);
    event["actor"]["run"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
    event["event_id"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
    event["reservation_id"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
    event["projection_generation"] = serde_json::json!(
        previous["projection_generation"]
            .as_u64()
            .expect("generation should exist")
            + 1
    );
    event["scopes"] = scopes.clone();
    event["head_snapshot"] = serde_json::json!({
        "kind": "branch", "full_ref": format!("refs/heads/{branch}"), "head": head,
    });
    event["phase_start_head"] = serde_json::json!(head);
    event["worktree_root"] = serde_json::json!(successor_root);
    event["worktree_administrative_locator"] = serde_json::json!(
        linked_checkout::ResolvedLinkedAdministration::read(successor_root).locator()
    );
    event["authorization"] = serde_json::json!({
        "kind": "sequence",
        "overlaps": [{"reservation_id": predecessor["reservation_id"],
                      "scope_revision": predecessor["scopes"], "scopes": scopes}],
        "blocker": predecessor["reservation_id"],
        "direction": "holder_before_requester",
        "edge_id": uuid::Uuid::now_v7().to_string(),
        "reason": "predecessor lands before this successor",
    });
    let mut journal = journal_text(root);
    journal.push_str(&event.to_string());
    journal.push('\n');
    fs::write(root.join(JOURNAL_PATH), journal).expect("successor scale claim should append");
}

fn add_worktree(root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let checkout = parent.join(branch);
    GIT.add_worktree_without_hooks(root, &checkout, branch, "HEAD");
    checkout
}

fn run_berth(root: &Path, args: &[&str]) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .args(args)
        .current_dir(root)
        .bounded_output()
        .expect("fixture command should run")
}

fn journal_text(root: &Path) -> String {
    fs::read_to_string(root.join(JOURNAL_PATH)).expect("journal should read")
}

fn last_journal_event(root: &Path) -> serde_json::Value {
    serde_json::from_str(
        journal_text(root)
            .lines()
            .last()
            .expect("journal should have an event"),
    )
    .expect("event should decode")
}
