// Shared starting histories and fixture operations.
// Shared fixture operations and recorded histories for edges.

pub(super) use cargo_berth_test_support::FIRST_RUN;
pub(super) use cargo_berth_test_support::GitDriver;
pub(super) use cargo_berth_test_support::IntegrationRepository;
pub(super) use cargo_berth_test_support::OptionalLocks;
pub(super) use cargo_berth_test_support::RepositoryTemplate;
pub(super) use cargo_berth_test_support::SECOND_RUN;
pub(super) use cargo_berth_test_support::assert_success;
pub(super) use cargo_berth_test_support::berth_command;
pub(super) use cargo_berth_test_support::claim_id;
pub(super) use cargo_berth_test_support::deferring_run_scope;
pub(super) use cargo_berth_test_support::write_file;

use crate::edges_templates::INITIAL_INTEGRATION_PAIR;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: no optional locks, with nothing held back from a hook.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};

/// The repository most tests here start from: a source and a test file committed on `main`,
/// then `cargo-berth init`.
pub(super) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "edges-initialized",
    build:            build_initialized_repository,
};

/// The source checkout with berth policy committed before linked lanes branch.
pub(super) const CONFIGURED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    name: "configured",
    build: |root| {
        build_initialized_repository(root);
        commit_configuration(root);
    },
    ..INITIALIZED_REPOSITORY
};

pub(super) use std::fs;
pub(super) use std::path::Path;
pub(super) use std::path::PathBuf;
pub(super) use std::process::Output;

use cargo_berth_test_support::json;
pub(super) use tempfile::TempDir;
pub(super) use tempfile::tempdir;

use crate::recorded_history;

pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const FIFTH_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";

pub(super) const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";

pub(super) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

pub(super) const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

pub(super) struct EdgeReadinessFixture {
    pub(super) repository:       TempDir,
    pub(super) worktrees:        TempDir,
    pub(super) base_head:        String,
    pub(super) baseline_journal: Vec<u8>,
}

impl EdgeReadinessFixture {
    pub(super) fn new() -> Self {
        let repository = CONFIGURED_REPOSITORY.instantiate();
        let base_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
        let baseline_journal =
            fs::read(repository.path().join(JOURNAL_PATH)).expect("initial journal should read");
        Self {
            repository,
            worktrees: tempdir().expect("worktree parent should exist"),
            base_head,
            baseline_journal,
        }
    }

    pub(super) fn start_case(&self, case_index: usize) -> (PathBuf, PathBuf) {
        git(
            self.repository.path(),
            &["reset", "--hard", &self.base_head],
        );
        fs::write(
            self.repository.path().join(JOURNAL_PATH),
            &self.baseline_journal,
        )
        .expect("journal should restore before independent readiness case");
        let projection = self
            .repository
            .path()
            .join(".git/cargo-berth/reservations.json");
        if projection.exists() {
            fs::remove_file(projection).expect("prior projection should remove");
        }
        let predecessor_root = add_worktree(
            self.repository.path(),
            self.worktrees.path(),
            &format!("predecessor-{case_index}"),
        );
        let successor_root = add_worktree(
            self.repository.path(),
            self.worktrees.path(),
            &format!("successor-{case_index}"),
        );
        (predecessor_root, successor_root)
    }
}

pub(super) struct CheckpointedPair {
    pub(super) predecessor_root: PathBuf,
    pub(super) successor_root:   PathBuf,
    pub(super) predecessor_id:   String,
    pub(super) successor_id:     String,
    pub(super) journal:          Vec<u8>,
}

impl CheckpointedPair {
    pub(super) fn new(fixture: &EdgeReadinessFixture) -> Self {
        let (predecessor_root, successor_root) = fixture.start_case(0);
        fs::write(
            predecessor_root.join("src/lib.rs"),
            "pub(super) fn predecessor() {}\n",
        )
        .expect("predecessor source should write");
        git(&predecessor_root, &["add", "."]);
        git(
            &predecessor_root,
            &["commit", "--quiet", "-m", "predecessor work"],
        );
        let predecessor_id = reservation_id(&claim(&predecessor_root, "tree:src", FIRST_RUN));
        let successor_id = reservation_id(&defer_claim(
            &successor_root,
            "file:src/lib.rs",
            SECOND_RUN,
            &predecessor_id,
        ));
        assert!(
            run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
                .status
                .success()
        );
        let journal = fs::read(fixture.repository.path().join(JOURNAL_PATH))
            .expect("checkpoint journal should read");
        Self {
            predecessor_root,
            successor_root,
            predecessor_id,
            successor_id,
            journal,
        }
    }
}

pub(super) fn deferred_pair(holder_root: &Path, requester_root: &Path) -> (String, String) {
    dirty_source(holder_root, "src/lib.rs");
    let holder = claim(holder_root, "tree:src", FIRST_RUN);
    let holder_id = reservation_id(&holder);
    let requester = defer_claim(requester_root, "file:src/lib.rs", SECOND_RUN, &holder_id);
    assert!(requester.status.success());
    (holder_id, reservation_id(&requester))
}

pub(super) fn sequence(repository_root: &Path, before: &str, after: &str, why: &str) -> Output {
    run_berth(
        repository_root,
        &["sequence", before, after, "--why", why, "--json"],
    )
}

/// Claim [`deferring_run_scope`] for `run`, then answer `blocker` on `scope` with `--defer`.
///
/// A deferral acquires no reservation, so the answer attaches to the one claimed first, and the
/// ordering graph holds that reservation against `blocker`.
pub(super) fn defer_claim(repository_root: &Path, scope: &str, run: &str, blocker: &str) -> Output {
    assert!(
        claim(repository_root, &deferring_run_scope(run), run)
            .status
            .success()
    );
    defer_answer(repository_root, scope, run, blocker)
}

pub(super) fn defer_answer(
    repository_root: &Path,
    scope: &str,
    run: &str,
    blocker: &str,
) -> Output {
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

/// Commit the policy without hooks: before any reservation exists, the trunk gate and the
/// post-commit drift check write no berth state.
pub(super) fn commit_configuration(repository_root: &Path) {
    git(
        repository_root,
        &["add", "--force", ".claude/config/berth.toml"],
    );
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "configure berth",
        ],
    );
}

pub(super) fn add_worktree(
    repository_root: &Path,
    parent: &Path,
    branch: &str,
) -> std::path::PathBuf {
    let worktree_root = parent.join(branch);
    GIT.add_worktree_without_hooks(repository_root, &worktree_root, branch, "HEAD");
    worktree_root
}

pub(super) fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

/// Add a real worktree beside the repository, the second party berth has always refused.
///
/// A distinct `--run` inside one worktree now names a second party too, but only a real
/// worktree can hold a reservation of its own alongside another run's. The returned
/// directory owns the worktree and must outlive its use.
pub(super) fn foreign_worktree(repository: &TempDir, name: &str) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("foreign worktree parent should exist");
    let root = directory.path().join(name);
    GIT.add_worktree_without_hooks(repository.path(), &root, name, "HEAD");
    let configuration = root.join(CONFIGURATION_PATH);
    if let Some(parent) = configuration.parent() {
        fs::create_dir_all(parent).expect("foreign worktree configuration should have a directory");
    }
    fs::copy(repository.path().join(CONFIGURATION_PATH), configuration)
        .expect("foreign worktree should share the repository configuration");
    (directory, root)
}

pub(super) fn build_initialized_repository(repository_root: &Path) {
    git(
        repository_root,
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository_root,
        &["config", "user.email", "test@example.com"],
    );
    git(repository_root, &["config", "user.name", "Test User"]);
    git(repository_root, &["config", "maintenance.auto", "false"]);
    fs::create_dir_all(repository_root.join("src")).expect("source directory should exist");
    fs::write(
        repository_root.join("src/lib.rs"),
        "pub(super) fn value() {}\n",
    )
    .expect("source should write");
    fs::create_dir_all(repository_root.join("tests")).expect("tests directory should exist");
    fs::write(repository_root.join("tests/base.rs"), "// test\n")
        .expect("test source should write");
    git(repository_root, &["add", "."]);
    git(repository_root, &["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth(repository_root, &["init", "--json"]);
    assert!(initialized.status.success());
}

/// A declared overlap needs actual uncommitted work in the holder's branch.
pub(super) fn dirty_source(root: &Path, path: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("dirty source has a parent"))
        .expect("dirty source parent should exist");
    fs::write(file, "// uncommitted holder work\n").expect("dirty source should write");
}

pub(super) fn claim(repository_root: &Path, scope: &str, run: &str) -> Output {
    claim_scopes(repository_root, &[scope], run)
}

pub(super) fn claim_scopes(repository_root: &Path, scopes: &[&str], run: &str) -> Output {
    let mut arguments = vec!["claim"];
    arguments.extend_from_slice(scopes);
    arguments.extend_from_slice(&["--run", run, "--why", "protect test work", "--json"]);
    run_berth(repository_root, &arguments)
}

pub(super) fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should report a reservation id")
        .to_owned()
}

pub(super) fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should print JSON")
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn git(repository_root: &Path, arguments: &[&str]) {
    GIT.run(repository_root, arguments);
}

/// Starting history before
/// `released_predecessor_ref_retires_only_after_every_successor_is_terminal`.
pub(super) const BEFORE_RELEASED_PREDECESSOR_REF_RETIRES_ONLY_AFTER_EVERY_SUCCESSOR_IS_TERMINAL:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-released-predecessor-ref-retires-only-after-every-successor-is-terminal",
    build_before_released_predecessor_ref_retires_only_after_every_successor_is_terminal,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ReleasedPredecessorRefRetiresOnlyAfterEverySuccessorIsTerminalState {
    pub(super) predecessor_id: String,
    pub(super) successor_id:   String,
}

fn build_before_released_predecessor_ref_retires_only_after_every_successor_is_terminal(
    destination: &Path,
) {
    let repository = CONFIGURED_REPOSITORY.instantiate();
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub(super) fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "predecessor first",
    );
    assert!(sequence.status.success());
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    git(repository.path(), &["merge", "--quiet", "predecessor"]);
    assert!(
        run_berth(repository.path(), &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    assert!(
        run_berth(repository.path(), &["release", &predecessor_id, "--json"])
            .status
            .success()
    );

    recorded_history::save(
        destination,
        repository.path(),
        &ReleasedPredecessorRefRetiresOnlyAfterEverySuccessorIsTerminalState {
            predecessor_id,
            successor_id,
        },
    );
}

/// Starting history before `successor_head_and_current_trunk_both_control_readiness`.
pub(super) const BEFORE_SUCCESSOR_HEAD_AND_CURRENT_TRUNK_BOTH_CONTROL_READINESS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-successor-head-and-current-trunk-both-control-readiness",
    build_before_successor_head_and_current_trunk_both_control_readiness,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SuccessorHeadAndCurrentTrunkBothControlReadinessState {
    pub(super) base:                String,
    pub(super) predecessor_id:      String,
    pub(super) first_successor_id:  String,
    pub(super) second_successor_id: String,
}

fn build_before_successor_head_and_current_trunk_both_control_readiness(destination: &Path) {
    let repository = CONFIGURED_REPOSITORY.instantiate();
    fs::create_dir_all(repository.path().join("left")).expect("left directory should exist");
    fs::create_dir_all(repository.path().join("right")).expect("right directory should exist");
    fs::write(repository.path().join("left/shared.rs"), "// left\n")
        .expect("left source should write");
    fs::write(repository.path().join("right/shared.rs"), "// right\n")
        .expect("right source should write");
    git(repository.path(), &["add", "."]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "shared paths"],
    );
    let base = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let first_successor_root = add_worktree(repository.path(), worktrees.path(), "successor-one");
    let second_successor_root = add_worktree(repository.path(), worktrees.path(), "successor-two");
    fs::write(
        predecessor_root.join("left/shared.rs"),
        "// predecessor left\n",
    )
    .expect("predecessor left source should write");
    fs::write(
        predecessor_root.join("right/shared.rs"),
        "// predecessor right\n",
    )
    .expect("predecessor right source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim_scopes(&predecessor_root, &["tree:left", "tree:right"], FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let first_successor = defer_claim(
        &first_successor_root,
        "file:left/shared.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let first_successor_id = reservation_id(&first_successor);
    let second_successor = defer_claim(
        &second_successor_root,
        "file:right/shared.rs",
        THIRD_RUN,
        &predecessor_id,
    );
    let second_successor_id = reservation_id(&second_successor);
    let checkpoint = run_berth(&predecessor_root, &["release", &predecessor_id, "--json"]);
    assert!(checkpoint.status.success());
    git(repository.path(), &["merge", "--quiet", "predecessor"]);

    recorded_history::save(
        destination,
        repository.path(),
        &SuccessorHeadAndCurrentTrunkBothControlReadinessState {
            base,
            predecessor_id,
            first_successor_id,
            second_successor_id,
        },
    );
}

/// Starting history before `same_target_edge_waits_for_predecessor_to_reach_integration`.
pub(super) const BEFORE_SAME_TARGET_EDGE_WAITS_FOR_PREDECESSOR_TO_REACH_INTEGRATION:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-same-target-edge-waits-for-predecessor-to-reach-integration",
    build_before_same_target_edge_waits_for_predecessor_to_reach_integration,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SameTargetEdgeWaitsForPredecessorToReachIntegrationState {
    pub(super) successor:      PathBuf,
    pub(super) predecessor_id: String,
    pub(super) successor_id:   String,
}

fn build_before_same_target_edge_waits_for_predecessor_to_reach_integration(destination: &Path) {
    let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let predecessor = repo.lane("same-target-a", "integration");
    let successor = repo.lane("same-target-b", "integration");
    write_file(&predecessor, "shared.txt", "A\n");
    let claimed = repo.claim(&predecessor, "file:shared.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let predecessor_id = claim_id(&claimed);
    let deferred = repo.defer_claim(&successor, "file:shared.txt", SECOND_RUN, &predecessor_id);
    assert_success(&deferred);
    let successor_id = claim_id(&deferred);
    repo.commit_file(&predecessor, "shared.txt", "A\n", "A work");
    assert_success(&repo.run(&predecessor, &["release", &predecessor_id, "--json"]));

    recorded_history::save(
        destination,
        repo.root(),
        &SameTargetEdgeWaitsForPredecessorToReachIntegrationState {
            successor,
            predecessor_id,
            successor_id,
        },
    );
}

/// Starting history before `sequence_rejects_each_invalid_identity_and_carries_alerts`.
pub(super) const BEFORE_SEQUENCE_REJECTS_EACH_INVALID_IDENTITY_AND_CARRIES_ALERTS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-sequence-rejects-each-invalid-identity-and-carries-alerts",
    build_before_sequence_rejects_each_invalid_identity_and_carries_alerts,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SequenceRejectsEachInvalidIdentityAndCarriesAlertsState {
    pub(super) holder_id:             String,
    pub(super) requester_id:          String,
    pub(super) orphan_id:             String,
    pub(super) third_root:            PathBuf,
    pub(super) mapped_reservation_id: String,
}

fn build_before_sequence_rejects_each_invalid_identity_and_carries_alerts(destination: &Path) {
    let repository = CONFIGURED_REPOSITORY.instantiate();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (holder_id, requester_id) = deferred_pair(repository.path(), &second_root);
    let worktrees = tempdir().expect("worktree parent should exist");
    let orphan_root = add_worktree(repository.path(), worktrees.path(), "orphan-alert");
    fs::write(orphan_root.join("orphan.txt"), "orphan work\n").expect("orphan source should write");
    git(&orphan_root, &["add", "."]);
    git(&orphan_root, &["commit", "--quiet", "-m", "orphan work"]);
    let orphan = claim(&orphan_root, "file:orphan.txt", FIFTH_RUN);
    let orphan_id = reservation_id(&orphan);
    assert!(
        run_berth(&orphan_root, &["release", &orphan_id, "--json"])
            .status
            .success()
    );
    fs::remove_dir_all(&orphan_root).expect("orphan worktree should be removable");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);

    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let session_id = "foreign-sequence-session";
    let mapped_claim = run_berth_with_session(
        &third_root,
        &[
            "claim",
            "file:session-sequence",
            "--run",
            THIRD_RUN,
            "--why",
            "hold the mapped reservation in the third checkout",
            "--json",
        ],
        session_id,
    );
    assert!(mapped_claim.status.success());

    let mapped_reservation_id = reservation_id(&mapped_claim);

    recorded_history::save(
        destination,
        repository.path(),
        &SequenceRejectsEachInvalidIdentityAndCarriesAlertsState {
            holder_id,
            requester_id,
            orphan_id,
            third_root,
            mapped_reservation_id,
        },
    );
}

pub(super) const HISTORY_CHECKPOINT_PAIR: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "edges-checkpoint-readiness-pair",
        build_history_checkpoint_pair,
    );

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CheckpointPairHistory {
    pub(super) journal:          String,
    pub(super) base_head:        String,
    pub(super) predecessor_root: PathBuf,
    pub(super) successor_root:   PathBuf,
    pub(super) predecessor_id:   String,
    pub(super) successor_id:     String,
}

fn build_history_checkpoint_pair(destination: &Path) {
    let fixture = EdgeReadinessFixture::new();
    let pair = CheckpointedPair::new(&fixture);
    recorded_history::save(
        destination,
        fixture.repository.path(),
        &CheckpointPairHistory {
            journal:          String::from_utf8(pair.journal)
                .expect("checkpoint journal should be UTF-8"),
            base_head:        fixture.base_head,
            predecessor_root: pair.predecessor_root,
            successor_root:   pair.successor_root,
            predecessor_id:   pair.predecessor_id,
            successor_id:     pair.successor_id,
        },
    );
}

#[derive(Clone, Copy)]
pub(super) enum CheckpointReadiness {
    NotIntegrated,
    Incorporated,
    MissingObject,
}
pub(super) fn recorded_checkpoint_pair() -> (EdgeReadinessFixture, CheckpointedPair) {
    let worktrees = tempdir().expect("worktree parent should exist");
    let repository = HISTORY_CHECKPOINT_PAIR.instantiate_with_external_worktrees(worktrees.path());
    let CheckpointPairHistory {
        journal,
        base_head,
        predecessor_root,
        successor_root,
        predecessor_id,
        successor_id,
    } = recorded_history::read(repository.path());
    let journal = journal.into_bytes();
    let fixture = EdgeReadinessFixture {
        repository,
        worktrees,
        base_head,
        baseline_journal: journal.clone(),
    };
    let pair = CheckpointedPair {
        predecessor_root,
        successor_root,
        predecessor_id,
        successor_id,
        journal,
    };
    (fixture, pair)
}

pub(super) const HISTORY_READINESS_NOT_INTEGRATED: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("edges-readiness-not_integrated", |root| {
        build_history_checkpoint_readiness(root, CheckpointReadiness::NotIntegrated);
    });

pub(super) const HISTORY_READINESS_INCORPORATED: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("edges-readiness-incorporated", |root| {
        build_history_checkpoint_readiness(root, CheckpointReadiness::Incorporated);
    });

pub(super) const HISTORY_READINESS_MISSING_OBJECT: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("edges-readiness-missing_object", |root| {
        build_history_checkpoint_readiness(root, CheckpointReadiness::MissingObject);
    });

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CheckpointReadinessHistory {
    pub(super) predecessor_id: String,
    pub(super) successor_id:   String,
    pub(super) expected:       serde_json::Value,
}
fn build_history_checkpoint_readiness(destination: &Path, readiness: CheckpointReadiness) {
    let (fixture, pair) = recorded_checkpoint_pair();

    let expected = match readiness {
        CheckpointReadiness::NotIntegrated => {
            serde_json::json!({"state": "holding", "hold": {"reason": "predecessor_not_on_trunk", "evidence": "not_integrated"}})
        },
        CheckpointReadiness::Incorporated => {
            git(
                fixture.repository.path(),
                &["merge", "--quiet", "predecessor-0"],
            );
            git(&pair.successor_root, &["merge", "--quiet", "main"]);
            serde_json::json!({"state": "fulfilled"})
        },
        CheckpointReadiness::MissingObject => {
            let protected_tip =
                git_stdout(fixture.repository.path(), &["rev-parse", "predecessor-0"]);
            git(
                fixture.repository.path(),
                &["reset", "--hard", &fixture.base_head],
            );
            git(
                &pair.successor_root,
                &["reset", "--hard", &fixture.base_head],
            );
            git(
                fixture.repository.path(),
                &[
                    "worktree",
                    "remove",
                    "--force",
                    pair.predecessor_root
                        .to_str()
                        .expect("worktree path should be UTF-8"),
                ],
            );
            git(
                fixture.repository.path(),
                &["branch", "-D", "predecessor-0"],
            );
            git(
                fixture.repository.path(),
                &["update-ref", "-d", &reservation_ref(&pair.predecessor_id)],
            );
            git(
                fixture.repository.path(),
                &["update-ref", "-d", &reservation_ref(&pair.successor_id)],
            );
            git(
                fixture.repository.path(),
                &["update-ref", "-d", "ORIG_HEAD"],
            );
            git(&pair.successor_root, &["update-ref", "-d", "ORIG_HEAD"]);
            git(
                fixture.repository.path(),
                &["reflog", "expire", "--expire=now", "--all"],
            );
            // These fixture commits are loose objects. Prune them after expiring reflogs;
            // repacking reachable history cannot affect the missing-object assertion.
            git(fixture.repository.path(), &["prune", "--expire=now"]);
            assert!(
                !git_status(
                    fixture.repository.path(),
                    &["cat-file", "-e", &protected_tip]
                ),
                "predecessor object must be absent after pruning"
            );
            serde_json::json!({"state": "holding", "hold": {"reason": "predecessor_not_on_trunk", "evidence": "object_unknown"}})
        },
    };
    fs::write(fixture.repository.path().join(JOURNAL_PATH), &pair.journal)
        .expect("checkpoint journal should restore before each readiness observation");
    let projection = fixture
        .repository
        .path()
        .join(".git/cargo-berth/reservations.json");
    if projection.exists() {
        fs::remove_file(projection).expect("prior readiness projection should remove");
    }

    recorded_history::save(
        destination,
        fixture.repository.path(),
        &CheckpointReadinessHistory {
            predecessor_id: pair.predecessor_id,
            successor_id: pair.successor_id,
            expected,
        },
    );
}

pub(super) fn reservation_ref(reservation_id: &str) -> String {
    format!("refs/cargo-berth/reservations/{reservation_id}")
}

pub(super) fn git_status(repository_root: &Path, arguments: &[&str]) -> bool {
    GIT.succeeds(repository_root, arguments)
}

pub(super) const HISTORY_SAME_TARGET_EDGE_REFUSES_INTEGRATION_BEFORE_THE_PREDECESSOR:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-same-target-edge-refuses-integration-before-the-predecessor",
    |destination| {
        let repo = IntegrationRepository::from_recorded_template(
            BEFORE_SAME_TARGET_EDGE_WAITS_FOR_PREDECESSOR_TO_REACH_INTEGRATION,
        );
        let SameTargetEdgeWaitsForPredecessorToReachIntegrationState {
            successor,
            predecessor_id,
            successor_id,
        } = recorded_history::read(repo.root());
        let ordered = sequence(
            repo.root(),
            &predecessor_id,
            &successor_id,
            "A reaches I first",
        );
        assert_success(&ordered);
        assert_eq!(
            json_output(&ordered)["payload"]["data"]["readiness"],
            serde_json::json!({
                "state": "holding",
                "hold": {"reason": "predecessor_not_on_trunk", "evidence": "not_integrated"}
            })
        );
        let waiting = repo.board();
        let row = waiting["payload"]["data"]["waiting"]["entries"]
            .as_array()
            .expect("waiting entries")
            .iter()
            .find(|row| row["successor"] == successor_id)
            .expect("successor waiting row");
        assert_eq!(
            row["action"]["instruction"],
            "wait for the predecessor to reach integration"
        );
        for root in [repo.root(), successor.as_path()] {
            let configuration = root.join(CONFIGURATION_PATH);
            let contents = fs::read_to_string(&configuration).expect("gate configuration reads");
            assert!(contents.contains("gate_mode = \"observe\""));
            fs::write(
                configuration,
                contents.replace("gate_mode = \"observe\"", "gate_mode = \"enforce\""),
            )
            .expect("gate configuration writes");
        }

        recorded_history::save(
            destination,
            repo.root(),
            &SameTargetEdgeRefusesIntegrationBeforeThePredecessorHistory {
                successor,
                predecessor_id,
                successor_id,
            },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SameTargetEdgeRefusesIntegrationBeforeThePredecessorHistory {
    pub(super) successor:      PathBuf,
    pub(super) predecessor_id: String,
    pub(super) successor_id:   String,
}

pub(super) const HISTORY_SAME_TARGET_EDGE_REQUIRES_INCORPORATION_AFTER_THE_PREDECESSOR_LANDS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-same-target-edge-requires-incorporation-after-the-predecessor-lands",
    |destination| {
        let repo = IntegrationRepository::from_recorded_template(
            BEFORE_SAME_TARGET_EDGE_WAITS_FOR_PREDECESSOR_TO_REACH_INTEGRATION,
        );
        let SameTargetEdgeWaitsForPredecessorToReachIntegrationState {
            successor,
            predecessor_id,
            successor_id,
        } = recorded_history::read(repo.root());
        let ordered = sequence(
            repo.root(),
            &predecessor_id,
            &successor_id,
            "A reaches I first",
        );
        assert_success(&ordered);
        assert_eq!(
            json_output(&ordered)["payload"]["data"]["readiness"],
            serde_json::json!({
                "state": "holding",
                "hold": {"reason": "predecessor_not_on_trunk", "evidence": "not_integrated"}
            })
        );
        let waiting = repo.board();
        let row = waiting["payload"]["data"]["waiting"]["entries"]
            .as_array()
            .expect("waiting entries")
            .iter()
            .find(|row| row["successor"] == successor_id)
            .expect("successor waiting row");
        assert_eq!(
            row["action"]["instruction"],
            "wait for the predecessor to reach integration"
        );
        for root in [repo.root(), successor.as_path()] {
            let configuration = root.join(CONFIGURATION_PATH);
            let contents = fs::read_to_string(&configuration).expect("gate configuration reads");
            assert!(contents.contains("gate_mode = \"observe\""));
            fs::write(
                configuration,
                contents.replace("gate_mode = \"observe\"", "gate_mode = \"enforce\""),
            )
            .expect("gate configuration writes");
        }
        let denied = repo.run(&successor, &["integrate", &successor_id, "--json"]);
        let denied_json = json(&denied);
        assert_eq!(
            denied_json["status"], "blocked_by_ordering",
            "{denied_json}"
        );
        assert!(
            denied_json["message"]
                .as_str()
                .expect("denial message")
                .contains(&format!(
                    "integrate reservation {predecessor_id} into integration"
                ))
        );

        recorded_history::save(
            destination,
            repo.root(),
            &SameTargetEdgeRequiresIncorporationAfterThePredecessorLandsHistory {
                successor,
                predecessor_id,
                successor_id,
            },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SameTargetEdgeRequiresIncorporationAfterThePredecessorLandsHistory {
    pub(super) successor:      PathBuf,
    pub(super) predecessor_id: String,
    pub(super) successor_id:   String,
}

pub(super) const HISTORY_SAME_TARGET_EDGE_SETTLES_AFTER_SUCCESSOR_INCORPORATION:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "edges-same-target-edge-settles-after-successor-incorporation",
    |destination| {
        let repo = IntegrationRepository::from_recorded_template(
            BEFORE_SAME_TARGET_EDGE_WAITS_FOR_PREDECESSOR_TO_REACH_INTEGRATION,
        );
        let SameTargetEdgeWaitsForPredecessorToReachIntegrationState {
            successor,
            predecessor_id,
            successor_id,
        } = recorded_history::read(repo.root());
        let ordered = sequence(
            repo.root(),
            &predecessor_id,
            &successor_id,
            "A reaches I first",
        );
        assert_success(&ordered);
        assert_eq!(
            json_output(&ordered)["payload"]["data"]["readiness"],
            serde_json::json!({
                "state": "holding",
                "hold": {"reason": "predecessor_not_on_trunk", "evidence": "not_integrated"}
            })
        );
        let waiting = repo.board();
        let row = waiting["payload"]["data"]["waiting"]["entries"]
            .as_array()
            .expect("waiting entries")
            .iter()
            .find(|row| row["successor"] == successor_id)
            .expect("successor waiting row");
        assert_eq!(
            row["action"]["instruction"],
            "wait for the predecessor to reach integration"
        );
        for root in [repo.root(), successor.as_path()] {
            let configuration = root.join(CONFIGURATION_PATH);
            let contents = fs::read_to_string(&configuration).expect("gate configuration reads");
            assert!(contents.contains("gate_mode = \"observe\""));
            fs::write(
                configuration,
                contents.replace("gate_mode = \"observe\"", "gate_mode = \"enforce\""),
            )
            .expect("gate configuration writes");
        }
        let denied = repo.run(&successor, &["integrate", &successor_id, "--json"]);
        let denied_json = json(&denied);
        assert_eq!(
            denied_json["status"], "blocked_by_ordering",
            "{denied_json}"
        );
        assert!(
            denied_json["message"]
                .as_str()
                .expect("denial message")
                .contains(&format!(
                    "integrate reservation {predecessor_id} into integration"
                ))
        );

        repo.merge_by_commit("same-target-a");
        let board = repo.board();
        assert_eq!(
            waiting_action_for(&board, &successor_id),
            "successor_must_incorporate_predecessor"
        );
        let row = board["payload"]["data"]["waiting"]["entries"]
            .as_array()
            .expect("waiting entries")
            .iter()
            .find(|row| row["successor"] == successor_id)
            .expect("successor waiting row");
        assert!(
            row["action"]["instruction"]
                .as_str()
                .expect("instruction")
                .contains("current integration")
        );
        assert_ne!(
            repo.git_stdout(repo.root(), &["rev-parse", "main"]),
            repo.git_stdout(repo.root(), &["rev-parse", "integration"])
        );

        recorded_history::save(
            destination,
            repo.root(),
            &SameTargetEdgeSettlesAfterSuccessorIncorporationHistory {
                successor,
                predecessor_id,
                successor_id,
            },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SameTargetEdgeSettlesAfterSuccessorIncorporationHistory {
    pub(super) successor:      PathBuf,
    pub(super) predecessor_id: String,
    pub(super) successor_id:   String,
}

pub(super) fn waiting_action_for<'board>(
    board: &'board serde_json::Value,
    successor_id: &str,
) -> &'board str {
    board["payload"]["data"]["waiting"]["entries"]
        .as_array()
        .expect("waiting entries")
        .iter()
        .find(|row| row["successor"] == successor_id)
        .expect("successor has a waiting edge")["action"]["reason"]
        .as_str()
        .expect("waiting action reason")
}

/// The initial edges and ordinary readiness precede deletion of the middle checkout.
pub(super) const ORPHANED_MIDDLE_START: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("edges-orphaned-middle-start", build_orphaned_middle_start);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct OrphanedMiddleStart {
    pub(super) first_id:           String,
    pub(super) middle_id:          String,
    pub(super) ordinary_readiness: serde_json::Value,
}

fn build_orphaned_middle_start(destination: &Path) {
    let repository = crate::edges_templates::LEFT_RIGHT_TREES.instantiate();
    let worktrees = repository.path().join(".git/fixture-worktrees");
    let first_root = worktrees.join("first");
    let middle_root = worktrees.join("middle");
    let before_root = worktrees.join("before-orphaning");
    dirty_source(&first_root, "left/shared.rs");
    let first = claim(&first_root, "tree:left", FIRST_RUN);
    let first_id = reservation_id(&first);
    let middle_id = reservation_id(&claim(&middle_root, "tree:right", SECOND_RUN));
    let middle = defer_answer(&middle_root, "file:left/shared.rs", SECOND_RUN, &first_id);
    assert_eq!(reservation_id(&middle), middle_id);
    let first_edge = sequence(
        repository.path(),
        &first_id,
        &middle_id,
        "first before middle",
    );
    assert!(first_edge.status.success());
    commit_middle_work(&middle_root);
    let before = defer_claim(&before_root, "file:right/before.rs", THIRD_RUN, &middle_id);
    let before_id = reservation_id(&before);
    let before_edge = sequence(
        repository.path(),
        &middle_id,
        &before_id,
        "middle before the comparison successor",
    );
    assert!(before_edge.status.success());
    let ordinary_readiness = json_output(&before_edge)["payload"]["data"]["readiness"].clone();
    recorded_history::save(
        destination,
        repository.path(),
        &OrphanedMiddleStart {
            first_id,
            middle_id,
            ordinary_readiness,
        },
    );
}

/// The middle holder retains its real outgoing work while its checkout is absent.
fn commit_middle_work(middle_root: &Path) {
    for path in ["right/before.rs", "right/orphaned.rs", "right/recovered.rs"] {
        dirty_source(middle_root, path);
    }
    git(middle_root, &["add", "right"]);
    git(
        middle_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "middle work",
        ],
    );
}
