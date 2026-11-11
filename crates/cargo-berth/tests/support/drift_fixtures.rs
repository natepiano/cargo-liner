// Shared starting histories and fixture operations.

pub(super) use cargo_berth_test_support::FIRST_RUN;
pub(super) use cargo_berth_test_support::GitDriver;
pub(super) use cargo_berth_test_support::OptionalLocks;
pub(super) use cargo_berth_test_support::RepositoryTemplate;
pub(super) use cargo_berth_test_support::SECOND_RUN;
pub(super) use cargo_berth_test_support::berth_command;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// Committed berth policy and an empty ledger, copied with fresh checkout owners.
pub(super) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "drift-initialized",
    build:            build_initialized_repository,
};

/// How this file drives git: no optional locks, clearing what its fixtures set for themselves.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[BYPASS_ENVIRONMENT, RUN_ENVIRONMENT],
};

pub(super) use std::fs;
pub(super) use std::path::Path;
pub(super) use std::path::PathBuf;
pub(super) use std::process::Output;

use cargo_berth_test_support::assert_success;
pub(super) use tempfile::TempDir;
pub(super) use tempfile::tempdir;

use crate::recorded_history;

pub(super) const BYPASS_ENVIRONMENT: &str = "CARGO_BERTH_BYPASS";

pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const POST_COMMIT_ENVIRONMENT: &str = "CARGO_BERTH_POST_COMMIT";

pub(super) const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";

pub(super) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

/// Where the holder standing over a widening subject's scopes is, and in what state.
///
/// A widening asks the one foreignness question the pre-edit hook asks, so a same-worktree
/// holder is foreign only while another *presented* run holds it `Active`. These are the
/// two same-worktree standings that question declines --- released and awaiting
/// integration, and claimed under an identity the engine issued for itself --- and the one
/// standing it accepts.
#[derive(Clone, Copy, Debug)]
pub(super) enum OverlappedHolderStanding {
    /// This worktree, another run, released and awaiting integration.
    HereAwaitingIntegration,
    /// This worktree, claimed by post-commit drift under an identity nobody presented.
    HereUnderAnEngineIssuedIdentity,
    /// Another worktree, held `Active` by a run that presented its identity.
    InAnotherWorktreeUnderAPresentedRun,
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

/// Give a foreign holder actual work that can conflict when its branch merges.
pub(super) fn dirty_source(repository_root: &Path, path: &str) {
    let target = repository_root.join(path);
    fs::create_dir_all(target.parent().expect("held path has a parent"))
        .expect("held directory should exist");
    fs::write(target, "uncommitted holder work\n").expect("held path should write");
}

pub(super) fn initialized_repository() -> TempDir { INITIALIZED_REPOSITORY.instantiate() }

pub(super) fn build_initialized_repository(repository_root: &Path) {
    build_scratch_repository(repository_root);
    assert!(
        run_berth(repository_root, &["init", "--json"])
            .status
            .success()
    );
    git(repository_root, &["add", "--force", CONFIGURATION_PATH]);
    // This unreserved trunk commit has no hook effect; gate.rs proves that contract.
    GIT.run_without_hooks(
        repository_root,
        ["commit", "--quiet", "-m", "configure berth"],
    );
}

pub(super) fn build_scratch_repository(repository_root: &Path) {
    cargo_berth_test_support::build_scratch_repository(repository_root, GIT);
}

pub(super) fn claim(repository_root: &Path, scope: &str, run: &str) -> String {
    let claimed = run_berth(
        repository_root,
        &["claim", scope, "--run", run, "--why", "test work", "--json"],
    );
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    json_output(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

pub(super) fn claim_with_override(
    repository_root: &Path,
    scope: &str,
    run: &str,
    holder_id: &str,
) -> String {
    let arguments = [
        "claim",
        scope,
        "--run",
        run,
        "--why",
        "test overlapping work",
        "--override",
        holder_id,
        "--overlap-why",
        "the shared edit was reviewed",
        "--json",
    ];
    let applied = run_berth(repository_root, &arguments);
    assert!(
        applied.status.success(),
        "the override claim failed: {}",
        String::from_utf8_lossy(&applied.stdout)
    );
    json_output(&applied)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("authorized claim should return a reservation id")
        .to_owned()
}

pub(super) fn post_commit_drift(repository_root: &Path, arguments: &[&str]) -> Output {
    let mut command_arguments = vec!["drift", "--full"];
    command_arguments.extend_from_slice(arguments);
    command_arguments.push("--json");
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(command_arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .output()
        .expect("post-commit drift should run")
}

pub(super) fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render JSON")
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(POST_COMMIT_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn git(repository_root: &Path, arguments: &[&str]) {
    GIT.run(repository_root, arguments);
}

pub(super) const HISTORY_WIDENING_OVER_AN_OVERLAPPED_HOLDER_CHECKPOINT:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "drift-widening-over-an-overlapped-holder-checkpoint",
    |destination| {
        build_history_widening_over_an_overlapped_holder(
            destination,
            OverlappedHolderStanding::HereAwaitingIntegration,
        );
    },
);

pub(super) const HISTORY_WIDENING_OVER_AN_OVERLAPPED_HOLDER_ENGINE:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "drift-widening-over-an-overlapped-holder-engine",
    |destination| {
        build_history_widening_over_an_overlapped_holder(
            destination,
            OverlappedHolderStanding::HereUnderAnEngineIssuedIdentity,
        );
    },
);

pub(super) const HISTORY_WIDENING_OVER_AN_OVERLAPPED_HOLDER_FOREIGN:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "drift-widening-over-an-overlapped-holder-foreign",
    |destination| {
        build_history_widening_over_an_overlapped_holder(
            destination,
            OverlappedHolderStanding::InAnotherWorktreeUnderAPresentedRun,
        );
    },
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct WideningOverAnOverlappedHolderHistory {
    pub(super) holder_id:  String,
    pub(super) subject_id: String,
    pub(super) probe_root: PathBuf,
}

fn build_history_widening_over_an_overlapped_holder(
    destination: &Path,
    standing: OverlappedHolderStanding,
) {
    let repository = initialized_repository();
    let (_probe_directory, probe_root) = foreign_worktree(&repository, "probe");
    let (_holder_directory, holder_root) = foreign_worktree(&repository, "holder");
    let (holder_id, subject_id) = match standing {
        OverlappedHolderStanding::HereAwaitingIntegration => {
            // The holder must carry unintegrated work, or releasing it leaves nothing to
            // integrate, its evidence reads as clear, and it stops blocking edits at all.
            git(
                repository.path(),
                &["switch", "--quiet", "-c", "holder-phase"],
            );
            fs::create_dir_all(repository.path().join("shared"))
                .expect("held scope directory should exist");
            fs::write(repository.path().join("shared/held.txt"), "holder work\n")
                .expect("held path should write");
            git(repository.path(), &["add", "shared/held.txt"]);
            git(
                repository.path(),
                &["commit", "--quiet", "-m", "holder work"],
            );
            let holder_id = claim(repository.path(), "file:shared/held.txt", FIRST_RUN);
            let checkpointed = run_berth(repository.path(), &["release", &holder_id, "--json"]);
            assert!(checkpointed.status.success());
            assert_eq!(
                json_output(&checkpointed)["payload"]["data"]["status"],
                "checkpointed"
            );
            (
                holder_id,
                claim(repository.path(), "tree:shared", SECOND_RUN),
            )
        },
        OverlappedHolderStanding::HereUnderAnEngineIssuedIdentity => {
            fs::create_dir_all(repository.path().join("shared"))
                .expect("held scope directory should exist");
            fs::write(repository.path().join("shared/held.txt"), "engine-issued\n")
                .expect("held path should write");
            let first_touched = post_commit_drift(repository.path(), &[]);
            assert!(first_touched.status.success());
            let holder_id = json_output(&first_touched)["payload"]["data"]["widening"]
                ["acquisition"]["reservation_id"]
                .as_str()
                .expect("the post-write first touch should return a reservation id")
                .to_owned();
            (
                holder_id,
                claim(repository.path(), "tree:shared", SECOND_RUN),
            )
        },
        OverlappedHolderStanding::InAnotherWorktreeUnderAPresentedRun => {
            dirty_source(&holder_root, "shared/held.txt");
            let holder_id = claim(&holder_root, "file:shared/held.txt", FIRST_RUN);
            let subject_id =
                claim_with_override(repository.path(), "tree:shared", SECOND_RUN, &holder_id);
            (holder_id, subject_id)
        },
    };

    recorded_history::save(
        destination,
        repository.path(),
        &WideningOverAnOverlappedHolderHistory {
            holder_id,
            subject_id,
            probe_root,
        },
    );
}

pub(super) fn commit_entering_path(root: &Path, path: &str, contents: &str) -> String {
    commit_path_without_hooks(root, path, contents);
    git_stdout(root, &["rev-parse", "HEAD"]).trim().to_owned()
}

/// Disable hooks and set a commit time after the claims.
pub(super) fn commit_path_without_hooks(root: &Path, path: &str, contents: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("entered path parent"))
        .expect("entered directory writes");
    fs::write(file, contents).expect("entered path writes");
    git(root, &["add", path]);
    let committed = git_output_with_environment(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "enter foreign scope",
        ],
        "GIT_COMMITTER_DATE",
        "2030-01-01T00:00:00Z",
    );
    assert_success(&committed);
}

pub(super) fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

pub(super) fn git_output_with_environment(
    repository_root: &Path,
    arguments: &[&str],
    name: &str,
    value: &str,
) -> Output {
    GIT.output_with_environment(repository_root, arguments, name, value)
}

pub(super) const COMMITTED_INCURSION: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("drift-committed-incursion", |destination| {
        let repository = initialized_repository();
        let (subject_directory, subject_root) =
            foreign_worktree(&repository, "commit-entry-subject");
        dirty_source(repository.path(), "shared/entered.txt");
        dirty_source(repository.path(), "shared/local.txt");
        dirty_source(repository.path(), "shared/authored.txt");
        let holder_id = claim(repository.path(), "tree:shared", FIRST_RUN);
        let subject_id = claim(&subject_root, "file:own.txt", SECOND_RUN);
        let entering_commit =
            commit_entering_path(&subject_root, "shared/entered.txt", "entered\n");

        recorded_history::save(
            destination,
            repository.path(),
            &CommittedIncursionHistory {
                subject_root,
                subject_id,
                holder_id,
                entering_commit,
            },
        );
        drop(subject_directory);
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CommittedIncursionHistory {
    pub(super) subject_root:    PathBuf,
    pub(super) subject_id:      String,
    pub(super) holder_id:       String,
    pub(super) entering_commit: String,
}
