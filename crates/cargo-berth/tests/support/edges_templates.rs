//! Repository states prepared before the edge tests start.

use std::fs;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::CommandDeadlineExt as _;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::claim_id;
use cargo_berth_test_support::deferring_run_scope;
use cargo_berth_test_support::write_file;
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";

use cargo_berth_test_support::CrossTargetOrdering;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// Released integration predecessor and its deferred main-target successor.
pub const CROSS_TARGET_PAIR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "cross-target-pair",
        build: |root| {
            IntegrationRepository::build_cross_target_pair(
                root,
                BERTH_EXECUTABLE,
                CrossTargetOrdering::Deferred,
            );
        },
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};

/// Released integration predecessor with an explicitly ordered main successor.
pub const CROSS_TARGET_AFTER_PAIR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "cross-target-after-pair",
        build: |root| {
            IntegrationRepository::build_cross_target_pair(
                root,
                BERTH_EXECUTABLE,
                CrossTargetOrdering::After,
            );
        },
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};

/// A claimed repository whose copied projection must be rebuilt.
pub const RECORDED_PROJECTION: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       BERTH_EXECUTABLE,
        name:             "recorded-projection",
        build:            build_recorded_projection,
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

fn build_recorded_projection(root: &Path) {
    IntegrationRepository::build_main_template(root, BERTH_EXECUTABLE);
    let claim = berth_command(BERTH_EXECUTABLE)
        .args([
            "claim",
            "file:README.md",
            "--run",
            "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b",
            "--json",
        ])
        .current_dir(root)
        .bounded_output()
        .expect("fixture claim should run");
    assert_success(&claim);
}

/// Commit one fixture file while suppressing managed hooks.
///
/// # Panics
/// Panics when the file or commit cannot be written.
pub fn commit_successor_fixture_file(root: &Path, path: &str, content: &str, message: &str) {
    fs::write(root.join(path), content).expect("fixture source should write");
    git(root, &["add", path]);
    git(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
}

/// Append a legacy edge event using the previous record’s actor and generation.
///
/// # Panics
/// Panics when the existing journal cannot be decoded or updated.
pub fn append_edge_fixture_event(root: &Path, mut event: serde_json::Value) {
    let previous = last_journal_event(root);
    for field in ["schema_version", "actor", "at"] {
        event[field] = previous[field].clone();
    }
    event["event_id"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
    event["projection_generation"] = serde_json::json!(
        previous["projection_generation"]
            .as_u64()
            .expect("generation should exist")
            + 1
    );
    let mut journal = journal_text(root);
    journal.push_str(&event.to_string());
    journal.push('\n');
    fs::write(root.join(JOURNAL_PATH), journal).expect("legacy successor event should append");
}

/// Reservation identities and commit tips of a rewritten successor history.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RewrittenSuccessorState {
    /// The retained predecessor reservation.
    pub predecessor_id:   String,
    /// The deferred successor reservation.
    pub successor_id:     String,
    /// Commit before the predecessor’s protected work.
    pub phase_start_head: String,
    /// Last commit protected by the predecessor.
    pub protected_tip:    String,
    /// Last commit present on the successor branch.
    pub successor_head:   String,
}

/// Assemble a released predecessor and a successor whose patch may match it.
///
/// # Panics
/// Panics when repository setup or its claim assertions fail.
#[must_use]
pub fn build_rewritten_successor(
    root: &Path,
    worktrees: &Path,
    successor_is_equivalent: bool,
) -> RewrittenSuccessorState {
    let phase_start_head = git_stdout(root, &["rev-parse", "HEAD"]);
    let predecessor_root = add_worktree(root, worktrees, "predecessor");
    let successor_root = add_worktree(root, worktrees, "successor");
    dirty_source(&predecessor_root, "src/lib.rs");
    let predecessor = claim(&predecessor_root, "file:src/lib.rs", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);

    let rewritten_content = "pub fn rewritten_predecessor() {}\n";
    fs::write(predecessor_root.join("src/lib.rs"), rewritten_content)
        .expect("predecessor source should write");
    git(&predecessor_root, &["add", "src/lib.rs"]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "protected predecessor"],
    );
    let protected_tip = git_stdout(&predecessor_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );

    if successor_is_equivalent {
        fs::write(successor_root.join("src/lib.rs"), rewritten_content)
            .expect("equivalent successor source should write");
        git(&successor_root, &["add", "src/lib.rs"]);
    } else {
        fs::write(
            successor_root.join("tests/successor.rs"),
            "// unrelated successor work\n",
        )
        .expect("unrelated successor source should write");
        git(&successor_root, &["add", "tests/successor.rs"]);
    }
    git(
        &successor_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "successor work",
        ],
    );
    let successor_head = git_stdout(&successor_root, &["rev-parse", "HEAD"]);

    // Integrate last without hooks so the command under test owns the first settlement.
    fs::write(root.join("src/lib.rs"), rewritten_content)
        .expect("rewritten trunk source should write");
    git(root, &["add", "src/lib.rs"]);
    git(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "rewritten trunk integration",
        ],
    );

    RewrittenSuccessorState {
        predecessor_id,
        successor_id,
        phase_start_head,
        protected_tip,
        successor_head,
    }
}

/// Whether the gap between equivalent successor patches touches protected paths.
#[derive(Clone, Copy)]
pub enum SuccessorGap {
    /// The gap changes only a path outside the reservation.
    Unrelated,
    /// The gap changes a protected path.
    Protected,
}

/// Legacy negative verdict with unrelated work between matching patches.
pub const LEGACY_NEGATIVE_UNRELATED_GAP: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "legacy-negative-unrelated-gap",
        build: |root| build_legacy_negative(root, SuccessorGap::Unrelated),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
/// Legacy negative verdict with protected work between matching patches.
pub const LEGACY_NEGATIVE_PROTECTED_GAP: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "legacy-negative-protected-gap",
        build: |root| build_legacy_negative(root, SuccessorGap::Protected),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
/// Select the complete legacy verdict fixture for one gap behavior.
#[must_use]
pub const fn legacy_negative_template(gap: SuccessorGap) -> LedgerTemplate {
    match gap {
        SuccessorGap::Unrelated => LEGACY_NEGATIVE_UNRELATED_GAP,
        SuccessorGap::Protected => LEGACY_NEGATIVE_PROTECTED_GAP,
    }
}

fn build_legacy_negative(root: &Path, successor_gap: SuccessorGap) {
    build_edge_main(root);
    let worktrees = &root.join(".git/fixture-worktrees");
    let mut state = build_rewritten_successor(root, worktrees, true);
    git(root, &["config", "core.hooksPath", "/dev/null"]);
    let predecessor_root = worktrees.join("predecessor");
    let successor_root = worktrees.join("successor");
    let old_tip = state.protected_tip.clone();
    commit_successor_fixture_file(
        &predecessor_root,
        "src/second.rs",
        "pub fn second() {}\n",
        "second phase commit",
    );
    state.protected_tip = git_stdout(&predecessor_root, &["rev-parse", "HEAD"]);
    let (gap, gap_content) = match successor_gap {
        SuccessorGap::Protected => (
            "src/lib.rs",
            "pub fn rewritten_predecessor() {}\npub fn intervening() {}\n",
        ),
        SuccessorGap::Unrelated => ("tests/gap.rs", "// intervening work\n"),
    };
    commit_successor_fixture_file(&successor_root, gap, gap_content, "gap between equivalents");
    commit_successor_fixture_file(
        &successor_root,
        "src/second.rs",
        "pub fn second() {}\n",
        "second equivalent",
    );
    state.successor_head = git_stdout(&successor_root, &["rev-parse", "HEAD"]);
    commit_successor_fixture_file(
        root,
        "src/second.rs",
        "pub fn second() {}\n",
        "complete phase on trunk",
    );
    // Assemble the old checkpoint directly, before any evaluator sees the two-commit subject.
    let journal = journal_text(root)
        .lines()
        .map(|line| {
            let mut event: serde_json::Value =
                serde_json::from_str(line).expect("event should decode");
            if event["reservation_id"] == state.predecessor_id {
                if event["op"] == "claim" {
                    event["scopes"] = serde_json::json!([{"path": "src", "kind": "tree"}]);
                }
                if event["protected_tip"] == old_tip {
                    event["protected_tip"] = serde_json::json!(state.protected_tip);
                }
            }
            event.to_string() + "\n"
        })
        .collect::<String>();
    fs::write(root.join(JOURNAL_PATH), journal).expect("two-commit checkpoint should write");
    retain_release(root, &state.predecessor_id);
    append_edge_fixture_event(
        root,
        serde_json::json!({
            "op": "successor_scoped_patch_equivalence_checked",
            "predecessor_reservation_id": state.predecessor_id,
            "subject": 1, "successor_head": state.successor_head,
            "verdict": "different",
        }),
    );
    fs::write(
        root.join(".git/legacy-negative.json"),
        serde_json::to_vec(&state).expect("legacy state should encode"),
    )
    .expect("legacy state should write");
}

/// Read the reservation identities and tips saved by a legacy verdict builder.
///
/// # Panics
/// Panics when the fixture state cannot be read or decoded.
#[must_use]
pub fn legacy_negative_state(root: &Path) -> RewrittenSuccessorState {
    serde_json::from_slice(
        &fs::read(root.join(".git/legacy-negative.json")).expect("legacy state should read"),
    )
    .expect("legacy state should decode")
}

/// Preserve the predecessor’s protected tip in its legacy release record.
///
/// # Panics
/// Panics when the fixture journal cannot be read or rewritten.
pub fn retain_release(root: &Path, predecessor_id: &str) {
    assert_eq!(journal_operation_count(root, "release"), 0);
    let previous = last_journal_event(root);
    let mut generation = previous["projection_generation"]
        .as_u64()
        .expect("generation should be numeric");
    let mut journal = journal_text(root);
    for mut event in [
        serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": predecessor_id,
            "status": {"status": "integrated", "trunk_oid": git_stdout(root, &["rev-parse", "HEAD"]), "proof": "scoped_patch_equivalent"},
            "edit_blocking_status": "clear",
        }),
        serde_json::json!({
            "op": "release",
            "reservation_id": predecessor_id,
            "disposition": {"kind": "integrated"},
        }),
    ] {
        generation += 1;
        for field in ["schema_version", "actor", "at"] {
            event[field] = previous[field].clone();
        }
        event["event_id"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
        event["projection_generation"] = serde_json::json!(generation);
        journal.push_str(&event.to_string());
        journal.push('\n');
    }
    fs::write(root.join(JOURNAL_PATH), journal).expect("legacy proof and release should append");
}

fn build_edge_main(root: &Path) {
    IntegrationRepository::build_main_template(root, BERTH_EXECUTABLE);
    write_file(root, "src/lib.rs", "pub fn value() {}\n");
    write_file(root, "tests/base.rs", "// test\n");
    GIT.run_without_hooks(root, ["add", "src", "tests"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "source and test paths"]);
}
fn git(root: &Path, args: &[&str]) { GIT.run(root, args); }
fn git_stdout(root: &Path, args: &[&str]) -> String { GIT.stdout(root, args) }
fn add_worktree(root: &Path, parent: &Path, branch: &str) -> std::path::PathBuf {
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
fn dirty_source(root: &Path, path: &str) { write_file(root, path, "// uncommitted holder work\n"); }
fn claim(root: &Path, scope: &str, run: &str) -> Output {
    run_berth(
        root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--why",
            "protect test work",
            "--json",
        ],
    )
}
fn reservation_id(output: &Output) -> String { claim_id(output) }
fn defer_claim(root: &Path, scope: &str, run: &str, blocker: &str) -> Output {
    assert_success(&claim(root, &deferring_run_scope(run), run));
    run_berth(
        root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--defer",
            blocker,
            "--overlap-why",
            "the order needs a decision",
            "--json",
        ],
    )
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
fn journal_operation_count(root: &Path, op: &str) -> usize {
    journal_text(root)
        .lines()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("event should decode")["op"]
                == op
        })
        .count()
}

/// Source trees and six empty linked lanes, before any reservation exists.
/// Disjoint committed trees and all lanes needed for middle-holder recovery.
pub const LEFT_RIGHT_TREES: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "left-right-trees",
        build: build_left_right_trees,
        ..RECORDED_PROJECTION.repository
    },
    ledger:       LedgerFixture::Empty,
    build_timing: TemplateBuildTiming::BeforeRun,
};
fn build_left_right_trees(root: &Path) {
    build_edge_main(root);
    write_file(root, "left/shared.rs", "// left\n");
    for path in ["right/before.rs", "right/orphaned.rs", "right/recovered.rs"] {
        write_file(root, path, "// right\n");
    }
    GIT.run_without_hooks(root, ["add", "left", "right"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "shared paths"]);
    for branch in [
        "first",
        "middle",
        "before-orphaning",
        "while-orphaned",
        "after-recovery",
        "replacement-middle",
    ] {
        add_worktree(root, &root.join(".git/fixture-worktrees"), branch);
    }
}

/// A released predecessor and an equivalent successor before the first board evaluation.
pub const EQUIVALENT_SUCCESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "equivalent-successor-history",
        build: |root| build_successor_history(root, true),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
/// A released predecessor and unrelated successor before the first board evaluation.
pub const DIFFERENT_SUCCESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "different-successor-history",
        build: |root| build_successor_history(root, false),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
fn build_successor_history(root: &Path, equivalent: bool) {
    build_edge_main(root);
    let state = build_rewritten_successor(root, &root.join(".git/fixture-worktrees"), equivalent);
    fs::write(
        root.join(".git/successor-history.json"),
        serde_json::to_vec(&state).expect("history should encode"),
    )
    .expect("history should write");
}
/// Read the protected history prepared before the test starts.
///
/// # Panics
/// Panics when history metadata cannot be read or decoded.
#[must_use]
pub fn successor_history_state(root: &Path) -> RewrittenSuccessorState {
    serde_json::from_slice(
        &fs::read(root.join(".git/successor-history.json")).expect("history should read"),
    )
    .expect("history should decode")
}
/// One cold predecessor, already merged without reconciliation.
pub const ONE_PREDECESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "one-predecessor-history",
        build: |root| build_predecessor_history(root, 1),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
/// Two cold predecessors, already merged without reconciliation.
pub const TWO_PREDECESSORS: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "two-predecessor-history",
        build: |root| build_predecessor_history(root, 2),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};

fn build_predecessor_history(root: &Path, predecessor_count: usize) {
    build_edge_main(root);
    commit_predecessor_scopes(root, predecessor_count);
    let worktrees = root.join(".git/fixture-worktrees");
    let successor_root = add_worktree(root, &worktrees, "successor");
    let predecessor_worktrees = (0..predecessor_count)
        .map(|predecessor_index| {
            let predecessor_branch = format!("predecessor-{predecessor_index}");
            let predecessor_root = add_worktree(root, &worktrees, &predecessor_branch);
            (predecessor_branch, predecessor_root)
        })
        .collect::<Vec<_>>();
    // Every answer below attaches to this one reservation of the successor run.
    assert!(
        claim(
            &successor_root,
            &deferring_run_scope(SECOND_RUN),
            SECOND_RUN
        )
        .status
        .success()
    );

    for (predecessor_index, (_, predecessor_root)) in predecessor_worktrees.iter().enumerate() {
        let predecessor_path = format!("predecessors/predecessor-{predecessor_index}.rs");
        dirty_source(predecessor_root, &predecessor_path);
        let predecessor_run = uuid::Uuid::now_v7().to_string();
        let predecessor = claim(
            predecessor_root,
            &format!("file:{predecessor_path}"),
            &predecessor_run,
        );
        let predecessor_id = reservation_id(&predecessor);
        let successor = defer_answer(
            &successor_root,
            &format!("file:{predecessor_path}"),
            SECOND_RUN,
            &predecessor_id,
        );
        let successor_id = reservation_id(&successor);
        let sequenced = sequence(
            root,
            &predecessor_id,
            &successor_id,
            "predecessor lands before this successor",
        );
        assert!(sequenced.status.success());

        fs::write(
            predecessor_root.join(&predecessor_path),
            format!("pub fn protected_predecessor_{predecessor_index}() {{}}\n"),
        )
        .expect("predecessor source should write");
        git(predecessor_root, &["add", &predecessor_path]);
        git(
            predecessor_root,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("protected predecessor {predecessor_index}"),
            ],
        );
        assert!(
            run_berth(predecessor_root, &["release", &predecessor_id, "--json"])
                .status
                .success()
        );
    }

    // Observe every cold predecessor together, before ordinary reconciliation can settle one.
    for (predecessor_branch, _) in &predecessor_worktrees {
        git(
            root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "merge",
                "--quiet",
                "--no-ff",
                predecessor_branch.as_str(),
            ],
        );
    }
}

fn commit_predecessor_scopes(repository_root: &Path, predecessor_count: usize) {
    fs::create_dir_all(repository_root.join("predecessors"))
        .expect("predecessor scope directory should exist");
    for predecessor_index in 0..predecessor_count {
        fs::write(
            repository_root.join(format!("predecessors/predecessor-{predecessor_index}.rs")),
            format!("// reserved predecessor scope {predecessor_index}\n"),
        )
        .expect("predecessor scope source should write");
    }
    git(repository_root, &["add", "predecessors"]);
    // No reservation exists yet, so the hooks would write no berth state.
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "predecessor reservation scopes",
        ],
    );
}

fn sequence(repository_root: &Path, before: &str, after: &str, why: &str) -> Output {
    run_berth(
        repository_root,
        &["sequence", before, after, "--why", why, "--json"],
    )
}

fn defer_answer(repository_root: &Path, scope: &str, run: &str, blocker: &str) -> Output {
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--defer",
            blocker,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "protect deferred work",
            "--json",
        ],
    )
}

/// A checkpoint with an unobserved historical cherry-pick and a conflicting current trunk.
pub const HISTORICAL_PROOF: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "historical-proof",
        build: build_historical_proof,
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
fn build_historical_proof(root: &Path) {
    build_edge_main(root);
    // Historical proof tests deliberately observe commits without any managed hook.
    git(root, &["config", "core.hooksPath", "/dev/null"]);
    let phase_start_head = git_stdout(root, &["rev-parse", "HEAD"]);
    let holder = add_worktree(
        root,
        &root.join(".git/fixture-worktrees"),
        "historical-holder",
    );
    let id = reservation_id(&claim(&holder, "file:src/lib.rs", FIRST_RUN));
    commit_successor_fixture_file(
        &holder,
        "src/lib.rs",
        "pub fn protected() {}\n",
        "protected phase",
    );
    let protected_tip = git_stdout(&holder, &["rev-parse", "HEAD"]);
    append_edge_fixture_event(
        root,
        serde_json::json!({
            "op": "checkpoint", "reservation_id": id,
            "protected_tip": protected_tip, "trunk_snapshot": phase_start_head,
        }),
    );
    commit_successor_fixture_file(
        root,
        "src/lib.rs",
        "pub fn protected() {}\n",
        "uncaptured cherry-pick",
    );
    let witness = git_stdout(root, &["rev-parse", "HEAD"]);
    assert_ne!(witness, protected_tip);
    commit_successor_fixture_file(
        root,
        "src/lib.rs",
        "pub fn later_trunk_revision() {}\n",
        "later protected-hunk rewrite",
    );
    let target = git_stdout(root, &["rev-parse", "HEAD"]);
    let replay = GIT.output(
        root,
        [
            "merge-tree",
            "--write-tree",
            &format!("--merge-base={phase_start_head}"),
            &target,
            &protected_tip,
        ],
    );
    assert_eq!(
        replay.status.code(),
        Some(1),
        "current-trunk replay must conflict"
    );
    fs::write(
        root.join(".git/historical-proof.json"),
        serde_json::to_vec(&serde_json::json!({
            "reservation_id": id, "phase_start_head": phase_start_head,
            "protected_tip": protected_tip, "witness": witness, "target": target,
        }))
        .expect("historical proof should encode"),
    )
    .expect("historical proof should write");
}
/// A committed lane with its active integration cover, before either landing operation.
pub const COVER_LANDING: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "cover-landing",
        build: |root| IntegrationRepository::build_cover_landing(root, BERTH_EXECUTABLE),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};

/// Source and committed berth policy without lifecycle events or automatic reconciliation.
pub const MERGE_EXTENT_MAIN: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "merge-extent-main",
        build: |root| IntegrationRepository::build_merge_extent_main(root, BERTH_EXECUTABLE),
        ..RECORDED_PROJECTION.repository
    },
    ledger:       LedgerFixture::Empty,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// An independent holder and requester with no reservations yet.
pub const MERGE_EXTENT_PAIR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "merge-extent-pair",
        build: |root| IntegrationRepository::build_merge_extent_pair(root, BERTH_EXECUTABLE),
        ..RECORDED_PROJECTION.repository
    },
    ..MERGE_EXTENT_MAIN
};

/// The integration target's first hooked commit, ready for a lane to claim work.
pub const INITIAL_INTEGRATION_PAIR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "initial-integration-pair",
        build: |root| IntegrationRepository::build_initial_integration_pair(root, BERTH_EXECUTABLE),
        ..RECORDED_PROJECTION.repository
    },
    ..RECORDED_PROJECTION
};
