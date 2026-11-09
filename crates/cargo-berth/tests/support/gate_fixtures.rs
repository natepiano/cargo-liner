pub(super) use std::os::unix::fs::PermissionsExt;
const FIXTURE_USER_NAME: &str = "Test User";

const FIXTURE_USER_EMAIL: &str = "test@example.com";

// Shared starting histories and fixture operations.

pub(super) use cargo_berth_test_support::EXECUTABLE_ENVIRONMENT;
pub(super) use cargo_berth_test_support::GitDriver;
pub(super) use cargo_berth_test_support::IntegrationRepository;
pub(super) use cargo_berth_test_support::OptionalLocks;
pub(super) use cargo_berth_test_support::RepositoryTemplate;
pub(super) use cargo_berth_test_support::assert_success;
pub(super) use cargo_berth_test_support::claim_id;
pub(super) use cargo_berth_test_support::deferring_run_scope;
pub(super) use cargo_berth_test_support::git_command;
pub(super) use cargo_berth_test_support::json;
pub(super) use cargo_berth_test_support::write_file;

use crate::edges_templates::INITIAL_INTEGRATION_PAIR;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: no optional locks, clearing what its fixtures set for themselves.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[BYPASS_ENVIRONMENT],
};

/// The repository most tests here start from; see `build_initialized_repository`.
pub(super) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "gate-initialized",
    build:            build_initialized_repository,
};

/// A repository after `cargo-berth init`; see `build_staged_configuration_repository`.
pub(super) const STAGED_CONFIGURATION_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    name: "staged-configuration",
    build: build_staged_configuration_repository,
    ..INITIALIZED_REPOSITORY
};

use std::fmt::Write as _;
pub(super) use std::fs;
pub(super) use std::fs::OpenOptions;
pub(super) use std::io::Write;
pub(super) use std::path::Path;
pub(super) use std::path::PathBuf;
pub(super) use std::process::Output;
pub(super) use std::process::Stdio;

pub(super) use cargo_berth_test_support::FixtureCommand as Command;
pub(super) use tempfile::TempDir;
pub(super) use tempfile::tempdir;

use crate::gate_templates;
use crate::recorded_history;
use crate::split_rebase;

pub(super) const BYPASS_ENVIRONMENT: &str = "CARGO_BERTH_BYPASS";

pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";

pub(super) const HOOK_PATH: &str = ".git/hooks/reference-transaction";

pub(super) const MARKER_PATH: &str = ".git/cargo-berth-run-id";

pub(super) const POST_COMMIT_SUBJECT_BRANCH: &str = "hook-subject";

pub(super) const RETENTION_TRACE_BRANCH: &str = "retention-trace";

pub(super) const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";

pub(super) const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";

pub(super) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

pub(super) const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";

pub(super) const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

#[derive(Clone, Copy)]
pub(super) enum RetentionRefPass {
    RepairOnly,
    DeletionOnly,
}

pub(super) struct DeferredPair {
    pub(super) repository:   TempDir,
    pub(super) worktrees:    TempDir,
    pub(super) blocked_root: PathBuf,
    pub(super) blocked_id:   String,
    pub(super) holder_id:    String,
}

pub(super) struct PreparedGateScopedComparisonFixture {
    pub(super) repository:      TempDir,
    pub(super) _worktrees:      TempDir,
    pub(super) reservation_ids: Vec<String>,
    pub(super) actual:          String,
    pub(super) proposed:        String,
}

pub(super) fn commit_scoped_target(
    repository_root: &Path,
    scopes: &[String],
    message: &str,
) -> String {
    for (index, scope) in scopes.iter().enumerate() {
        fs::write(
            repository_root.join(scope),
            format!("pub(super) fn proposed_{index}() {{}}\n"),
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

pub(super) fn deferred_pair(repository: TempDir) -> DeferredPair {
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
pub(super) fn dirty_source(root: &Path, path: &str) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().expect("held path has a parent"))
        .expect("held directory should exist");
    fs::write(target, "uncommitted holder work\n").expect("held work should write");
}

pub(super) fn initialized_repository() -> TempDir { INITIALIZED_REPOSITORY.instantiate() }

/// A scratch repository with berth initialized and its configuration committed on `main`.
///
/// The configuration commit skips the hooks: with no reservation yet they write nothing, which
/// `hooked_configuration_commit_on_an_unreserved_trunk_leaves_berth_state_unchanged` proves.
pub(super) fn build_initialized_repository(repository_root: &Path) {
    STAGED_CONFIGURATION_REPOSITORY.restore_into(repository_root);
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

/// A scratch repository after `cargo-berth init`, with the configuration staged to commit.
pub(super) fn build_staged_configuration_repository(repository_root: &Path) {
    build_scratch_repository(repository_root);
    assert!(
        run_berth(repository_root, &["init", "--json"])
            .status
            .success()
    );
    git(repository_root, &["add", "--force", CONFIGURATION_PATH]);
}

/// A source and a test file committed on `main`, with git maintenance off.
pub(super) fn build_scratch_repository(repository_root: &Path) {
    gate_templates::build_scratch_repository(repository_root);
}

pub(super) fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let worktree_root = parent.join(branch);
    GIT.add_worktree_without_hooks(repository_root, &worktree_root, branch, "HEAD");
    worktree_root
}

pub(super) fn commit_work(
    repository_root: &Path,
    path: &str,
    contents: &str,
    message: &str,
) -> String {
    fs::write(repository_root.join(path), contents).expect("work source should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

pub(super) fn claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    plan: &str,
    phase: &str,
) -> Output {
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

/// Claim [`deferring_run_scope`] for `run`, then answer `blocker` on `scope` with `--defer`.
///
/// A deferral acquires no reservation, so the answer attaches to the one claimed first, and the
/// ordering graph holds that reservation against `blocker`.
pub(super) fn defer_claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    plan: &str,
    phase: &str,
    blocker: &str,
) -> Output {
    assert!(
        claim(repository_root, &deferring_run_scope(run), run, plan, phase)
            .status
            .success()
    );
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
            "--json",
        ],
    )
}

pub(super) fn set_gate_mode(repository_root: &Path, mode: &str) {
    gate_templates::set_gate_mode(repository_root, mode);
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

/// The configured repository and linked worktree for post-commit setup.
pub(super) struct PostCommitCellFixture {
    pub(super) repository:   TempDir,
    pub(super) worktrees:    TempDir,
    pub(super) subject_root: PathBuf,
}

impl PostCommitCellFixture {
    pub(super) fn new() -> Self {
        let worktrees = tempdir().expect("worktree parent should exist");
        let repository =
            HISTORY_POST_COMMIT_CELL.instantiate_with_external_worktrees(worktrees.path());
        let PostCommitCellHistory { subject_root } = recorded_history::read(repository.path());
        Self {
            repository,
            worktrees,
            subject_root,
        }
    }
}

/// One commit `commit_files_without_hooks` makes: its message and the files it writes over its
/// parent's tree, each as `(path, contents)`.
pub(super) struct FixtureCommit {
    pub(super) message:       String,
    pub(super) written_files: Vec<(String, String)>,
}

/// Make each of `commits` in order on `branch`, checked out at `worktree_root`, then check the
/// last one out there.
///
/// One `git fast-import` makes every commit and one `git read-tree` updates the index and
/// worktree, so a cell's history costs two processes whatever its commit count. The hooks stay
/// disabled, as for every fixture commit that is not under test.
pub(super) fn commit_files_without_hooks(
    worktree_root: &Path,
    branch: &str,
    commits: &[FixtureCommit],
) {
    if commits.is_empty() {
        return;
    }
    let mut stream = String::new();
    for (index, fixture_commit) in commits.iter().enumerate() {
        let _ = writeln!(stream, "commit refs/heads/{branch}");
        let _ = writeln!(
            stream,
            "committer {FIXTURE_USER_NAME} <{FIXTURE_USER_EMAIL}> now"
        );
        let _ = writeln!(
            stream,
            "data {}\n{}",
            fixture_commit.message.len(),
            fixture_commit.message
        );
        if index == 0 {
            let _ = writeln!(stream, "from refs/heads/{branch}^0");
        }
        for (path, contents) in &fixture_commit.written_files {
            let _ = writeln!(
                stream,
                "M 100644 inline {path}\ndata {}\n{contents}",
                contents.len()
            );
        }
    }
    let mut child = git_command(BERTH_EXECUTABLE)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "fast-import",
            "--quiet",
            "--date-format=now",
        ])
        .current_dir(worktree_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("fast-import should start");
    child
        .stdin
        .take()
        .expect("fast-import stdin")
        .write_all(stream.as_bytes())
        .expect("fast-import stream should write");
    let imported = child.wait_with_output().expect("fast-import should finish");
    assert!(
        imported.status.success(),
        "fast-import failed: {}",
        String::from_utf8_lossy(&imported.stderr)
    );
    git(worktree_root, &["read-tree", "--reset", "-u", "HEAD"]);
}

/// Start the `cargo-berth` under test, named for any hook it goes on to fire.
///
/// The binary a test spawns is settled by its own path, but a managed hook this
/// binary triggers resolves `cargo-berth` for itself and answers with whatever
/// `cargo install` last left on the machine. Naming it here keeps a hook on the
/// code the test was compiled from, so a developer machine with an installed
/// copy and a continuous integration machine without one agree.
pub(super) fn berth_command() -> Command {
    let mut command = cargo_berth_test_support::berth_command(BERTH_EXECUTABLE);
    command.env(EXECUTABLE_ENVIRONMENT, BERTH_EXECUTABLE);
    command
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command()
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn write_private_hook(path: &Path, contents: impl AsRef<[u8]>) {
    let permissions = fs::metadata(path)
        .expect("managed hook metadata should read")
        .permissions();
    fs::remove_file(path).expect("shared managed hook should detach");
    fs::write(path, contents).expect("private managed hook should write");
    fs::set_permissions(path, permissions)
        .expect("private managed hook permissions should restore");
}

pub(super) fn install_repository_root_reference_transaction_sentinel(
    repository_root: &Path,
) -> PathBuf {
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

pub(super) fn checkpointed_reservations(repository_root: &Path, count: usize) -> Vec<String> {
    // Keep every checkpoint outstanding until the repair/deletion case chooses its lifecycle.
    // The reserved work is committed off trunk: reconciliation ends an integrated checkpoint
    // while a later active reservation of the same checkout still protects its work.
    git(
        repository_root,
        &["checkout", "--quiet", "-b", RETENTION_TRACE_BRANCH],
    );
    for index in 0..count {
        let path = format!("retention-trace-{index}");
        fs::write(repository_root.join(&path), "unmerged work\n")
            .expect("reserved branch work should write");
        git(repository_root, &["add", &path]);
    }
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "retention trace branch work",
        ],
    );
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
    for reservation_id in &reservation_ids {
        let checkpointed = run_berth(repository_root, &["release", reservation_id, "--json"]);
        assert!(checkpointed.status.success());
        assert_eq!(
            json_output(&checkpointed)["payload"]["data"]["status"],
            "checkpointed"
        );
    }
    // Record each checkpoint's unintegrated evidence before the traced pass.
    assert!(
        run_berth(repository_root, &["board", "--json"])
            .status
            .success()
    );
    reservation_ids
}

pub(super) fn apply_test_ref_transaction(repository_root: &Path, input: &str) {
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

pub(super) fn commit_work_without_hooks(
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

pub(super) fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> Output {
    berth_command()
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

pub(super) fn git(repository_root: &Path, arguments: &[&str]) {
    GIT.run(repository_root, arguments);
}

/// Runs git with an invocation-recording wrapper in place of the hooks' `cargo-berth`.
///
/// The wrapper appends each invocation's subcommand and phase to a file, then runs the
/// `cargo-berth` under test.
pub(super) struct HookedGit {
    pub(super) directory: TempDir,
}

impl HookedGit {
    pub(super) fn new() -> Self {
        let directory = tempdir().expect("wrapper directory");
        let hooked_git = Self { directory };
        let wrapper = hooked_git.wrapper();
        fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" >> {}\nexec {} \"$@\"\n",
                shell_single_quoted(&hooked_git.invocations()),
                shell_single_quoted(Path::new(BERTH_EXECUTABLE))
            ),
        )
        .expect("invocation-recording wrapper writes");
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).expect("wrapper executes");
        hooked_git
    }

    pub(super) fn wrapper(&self) -> PathBuf { self.directory.path().join("cargo-berth") }

    pub(super) fn invocations(&self) -> PathBuf { self.directory.path().join("invocations") }

    /// Run git with the wrapper as the hooks' `cargo-berth`, and assert it succeeded.
    pub(super) fn run(&self, repository_root: &Path, arguments: &[&str]) {
        let wrapper = self.wrapper();
        let output = GIT.output_with_environment(
            repository_root,
            arguments,
            EXECUTABLE_ENVIRONMENT,
            wrapper.to_str().expect("wrapper path should be UTF-8"),
        );
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    pub(super) fn commit_file(
        &self,
        repository_root: &Path,
        path: &str,
        contents: &str,
        message: &str,
    ) {
        write_file(repository_root, path, contents);
        git(repository_root, &["add", path]);
        self.run(repository_root, &["commit", "--quiet", "-m", message]);
    }
}

/// Claim the file named `branch` on the checked-out `branch`, release it, and resolve it with
/// `disposition`.
pub(super) fn resolve_branch_work(repository_root: &Path, branch: &str, disposition: &str) {
    let scope = format!("file:{branch}");
    let claimed = run_berth(
        repository_root,
        &["claim", &scope, "--run", FIRST_RUN, "--json"],
    );
    assert_success(&claimed);
    let reservation_id = reservation_id(&claimed);
    assert_success(&run_berth(repository_root, &["release", &reservation_id]));
    assert_success(&run_berth(
        repository_root,
        &[
            "resolve",
            &reservation_id,
            disposition,
            "--why",
            "the fixture discards this work",
        ],
    ));
}

pub(super) fn shell_single_quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

/// Starting history before `observe_enforce_and_one_use_force_apply_to_both_deferred_endpoints`.
pub(super) const BEFORE_OBSERVE_ENFORCE_AND_ONE_USE_FORCE_APPLY_TO_BOTH_DEFERRED_ENDPOINTS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-observe-enforce-and-one-use-force-apply-to-both-deferred-endpoints",
    build_before_observe_enforce_and_one_use_force_apply_to_both_deferred_endpoints,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ObserveEnforceAndOneUseForceApplyToBothDeferredEndpointsState {
    pub(super) base:           String,
    pub(super) holder_id:      String,
    pub(super) requester_id:   String,
    pub(super) holder_head:    String,
    pub(super) requester_root: PathBuf,
}

fn build_before_observe_enforce_and_one_use_force_apply_to_both_deferred_endpoints(
    destination: &Path,
) {
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
        "pub(super) fn holder_work() {}\n",
        "holder work",
    );

    recorded_history::save(
        destination,
        repository.path(),
        &ObserveEnforceAndOneUseForceApplyToBothDeferredEndpointsState {
            base,
            holder_id,
            requester_id,
            holder_head,
            requester_root,
        },
    );
}

/// Starting history before `active_lower_reservation_keeps_both_split_commits_after_update_refs`.
pub(super) const BEFORE_ACTIVE_LOWER_RESERVATION_KEEPS_BOTH_SPLIT_COMMITS_AFTER_UPDATE_REFS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-active-lower-reservation-keeps-both-split-commits-after-update-refs",
    build_before_active_lower_reservation_keeps_both_split_commits_after_update_refs,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ActiveLowerReservationKeepsBothSplitCommitsAfterUpdateRefsState {
    pub(super) trunk:          String,
    pub(super) first_contents: String,
    pub(super) full_contents:  String,
    pub(super) holder:         PathBuf,
    pub(super) id:             String,
    pub(super) old_lower:      String,
    pub(super) old_upper:      String,
}

fn build_before_active_lower_reservation_keeps_both_split_commits_after_update_refs(
    destination: &Path,
) {
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

    recorded_history::save(
        destination,
        repository.path(),
        &ActiveLowerReservationKeepsBothSplitCommitsAfterUpdateRefsState {
            trunk,
            first_contents,
            full_contents,
            holder,
            id,
            old_lower,
            old_upper,
        },
    );
}

/// Starting history before
/// `prepared_gate_settles_actual_trunk_evidence_but_never_a_proposed_witness`.
pub(super) const BEFORE_PREPARED_GATE_SETTLES_ACTUAL_TRUNK_EVIDENCE_BUT_NEVER_A_PROPOSED_WITNESS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-prepared-gate-settles-actual-trunk-evidence-but-never-a-proposed-witness",
    build_before_prepared_gate_settles_actual_trunk_evidence_but_never_a_proposed_witness,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedGateSettlesActualTrunkEvidenceButNeverAProposedWitnessState {
    pub(super) holder:     PathBuf,
    pub(super) base:       String,
    pub(super) id:         String,
    pub(super) checkpoint: String,
    pub(super) witness:    String,
}

fn build_before_prepared_gate_settles_actual_trunk_evidence_but_never_a_proposed_witness(
    destination: &Path,
) {
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
        "pub(super) fn integrated() {}\n",
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
        "pub(super) fn integrated() {}\n",
        "equivalent trunk witness",
    );
    assert_ne!(checkpoint, witness);

    recorded_history::save(
        destination,
        repository.path(),
        &PreparedGateSettlesActualTrunkEvidenceButNeverAProposedWitnessState {
            holder,
            base,
            id,
            checkpoint,
            witness,
        },
    );
}

/// Starting history before `listed_target_in_repository_path_with_backslash_is_gated`.
pub(super) const BEFORE_LISTED_TARGET_IN_REPOSITORY_PATH_WITH_BACKSLASH_IS_GATED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-listed-target-in-repository-path-with-backslash-is-gated",
    build_before_listed_target_in_repository_path_with_backslash_is_gated,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ListedTargetInRepositoryPathWithBackslashIsGatedState {
    pub(super) successor_id: String,
    pub(super) before:       String,
    pub(super) after:        String,
}

fn build_before_listed_target_in_repository_path_with_backslash_is_gated(destination: &Path) {
    let repository = IntegrationRepository::with_repository_prefix(BERTH_EXECUTABLE, "repo\\tx");
    let predecessor = repository.lane("slash-a", "integration");
    let successor = repository.lane("slash-b", "integration");
    write_file(&predecessor, "shared.txt", "A\n");
    let claimed = repository.claim(
        &predecessor,
        "file:shared.txt",
        FIRST_RUN,
        Some("integration"),
    );
    assert_success(&claimed);
    let predecessor_id = claim_id(&claimed);
    let deferred = repository.defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("integration"),
    );
    assert_success(&deferred);
    let successor_id = claim_id(&deferred);
    assert_success(&repository.run(
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
    repository.commit_file(&predecessor, "shared.txt", "A\n", "A work");
    repository.commit_file(&successor, "shared.txt", "B\n", "B work");
    set_gate_mode(repository.root(), "enforce");
    let before = repository.git_stdout(repository.root(), &["rev-parse", "integration"]);
    let after = repository.git_stdout(&successor, &["rev-parse", "HEAD"]);

    recorded_history::save(
        destination,
        repository.root(),
        &ListedTargetInRepositoryPathWithBackslashIsGatedState {
            successor_id,
            before,
            after,
        },
    );
}

/// Starting history before `target_gate_holds_neither_lane_after_a_default_answered_overlap`.
pub(super) const BEFORE_TARGET_GATE_HOLDS_NEITHER_LANE_AFTER_A_DEFAULT_ANSWERED_OVERLAP:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-target-gate-holds-neither-lane-after-a-default-answered-overlap",
    build_before_target_gate_holds_neither_lane_after_a_default_answered_overlap,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TargetGateHoldsNeitherLaneAfterADefaultAnsweredOverlapState {
    pub(super) holder:             PathBuf,
    pub(super) requester:          PathBuf,
    pub(super) integration_before: String,
    pub(super) requester_tip:      String,
}

fn build_before_target_gate_holds_neither_lane_after_a_default_answered_overlap(
    destination: &Path,
) {
    let repository = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let holder = repository.lane("target-a", "integration");
    let requester = repository.lane("target-b", "integration");
    let integration_before =
        repository.git_stdout(repository.root(), &["rev-parse", "integration"]);
    write_file(&holder, "shared.txt", "A\n");
    assert_success(&repository.claim(&holder, "file:shared.txt", FIRST_RUN, Some("integration")));
    let mut configuration = OpenOptions::new()
        .append(true)
        .open(repository.root().join(CONFIGURATION_PATH))
        .expect("configuration should open");
    writeln!(configuration, "default_answer = \"first_ready\"")
        .expect("default answer should write");
    assert_success(&repository.claim(
        &requester,
        &deferring_run_scope(SECOND_RUN),
        SECOND_RUN,
        Some("integration"),
    ));
    let answered = repository.claim(
        &requester,
        "file:shared.txt",
        SECOND_RUN,
        Some("integration"),
    );
    assert_success(&answered);
    assert_eq!(json(&answered)["status"], "answered");
    repository.commit_file(&holder, "shared.txt", "A\n", "A work");
    repository.commit_file(&requester, "shared.txt", "B\n", "B work");
    let requester_tip = repository.git_stdout(&requester, &["rev-parse", "HEAD"]);
    set_gate_mode(repository.root(), "enforce");

    recorded_history::save(
        destination,
        repository.root(),
        &TargetGateHoldsNeitherLaneAfterADefaultAnsweredOverlapState {
            holder,
            requester,
            integration_before,
            requester_tip,
        },
    );
}

/// Starting history before `renamed_trunk_refusal_names_its_branch`.
pub(super) const BEFORE_RENAMED_TRUNK_REFUSAL_NAMES_ITS_BRANCH:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-renamed-trunk-refusal-names-its-branch",
    build_before_renamed_trunk_refusal_names_its_branch,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RenamedTrunkRefusalNamesItsBranchState {
    pub(super) before: String,
    pub(super) after:  String,
}

fn build_before_renamed_trunk_refusal_names_its_branch(destination: &Path) {
    let repository = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    repository.git(repository.root(), &["branch", "develop", "main"]);
    repository.set_trunk("develop");
    assert_success(&repository.run(repository.root(), &["init", "--json"]));
    let predecessor = repository.lane("develop-a", "develop");
    let successor = repository.lane("develop-b", "develop");
    write_file(&predecessor, "shared.txt", "A\n");
    let claimed = repository.claim(&predecessor, "file:shared.txt", FIRST_RUN, Some("develop"));
    assert_success(&claimed);
    let predecessor_id = claim_id(&claimed);
    let deferred = repository.defer_claim_to(
        &successor,
        "file:shared.txt",
        SECOND_RUN,
        &predecessor_id,
        Some("develop"),
    );
    assert_success(&deferred);
    assert_success(&repository.run(
        repository.root(),
        &[
            "sequence",
            &predecessor_id,
            &claim_id(&deferred),
            "--why",
            "A before B",
            "--json",
        ],
    ));
    repository.commit_file(&predecessor, "shared.txt", "A\n", "A work");
    repository.commit_file(&successor, "shared.txt", "B\n", "B work");
    set_gate_mode(repository.root(), "enforce");
    let before = repository.git_stdout(repository.root(), &["rev-parse", "develop"]);
    let after = repository.git_stdout(&successor, &["rev-parse", "HEAD"]);

    recorded_history::save(
        destination,
        repository.root(),
        &RenamedTrunkRefusalNamesItsBranchState { before, after },
    );
}

/// Starting history before
/// `uninterrupted_apply_rebase_reanchors_active_reservation_above_unrelated_trunk_work`.
pub(super) const BEFORE_UNINTERRUPTED_APPLY_REBASE_REANCHORS_ACTIVE_RESERVATION_ABOVE_UNRELATED_TRUNK_WORK: cargo_berth_test_support::LedgerTemplate = recorded_history::template("gate-uninterrupted-apply-rebase-reanchors-active-reservation-above-unrelated-trunk-work", build_before_uninterrupted_apply_rebase_reanchors_active_reservation_above_unrelated_trunk_work);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct UninterruptedApplyRebaseReanchorsActiveReservationAboveUnrelatedTrunkWorkState {
    pub(super) holder:       PathBuf,
    pub(super) id:           String,
    pub(super) original_tip: String,
    pub(super) trunk:        String,
}

fn build_before_uninterrupted_apply_rebase_reanchors_active_reservation_above_unrelated_trunk_work(
    destination: &Path,
) {
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
        "pub(super) fn holder_work() {}\n",
        "active holder work",
    );
    let trunk = commit_work(root, "unrelated.txt", "upstream\n", "unrelated trunk work");
    assert_ne!(trunk, phase_start);

    recorded_history::save(
        destination,
        repository.path(),
        &UninterruptedApplyRebaseReanchorsActiveReservationAboveUnrelatedTrunkWorkState {
            holder,
            id,
            original_tip,
            trunk,
        },
    );
}

pub(super) const BEFORE_FORCED_CHECKPOINT_CONSUMES_ITS_PERMIT_BEFORE_ORDINARY_RECONCILIATION_SETTLES_IT:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-forced-checkpoint-consumes-its-permit-before-ordinary-reconciliation-settles-it",
    build_before_forced_checkpoint_consumes_its_permit_before_ordinary_reconciliation_settles_it,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ForcedCheckpointConsumesItsPermitBeforeOrdinaryReconciliationSettlesItState {
    pub(super) blocked_root: PathBuf,
    pub(super) blocked_id:   String,
    pub(super) holder_id:    String,
    pub(super) base:         String,
    pub(super) checkpoint:   String,
}

fn build_before_forced_checkpoint_consumes_its_permit_before_ordinary_reconciliation_settles_it(
    destination: &Path,
) {
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
        "pub(super) fn checkpointed_successor() {}\n",
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
    assert_eq!(
        git_stdout(&fixture.blocked_root, &["status", "--porcelain"]),
        ""
    );
    set_gate_mode(root, "enforce");

    recorded_history::save(
        destination,
        fixture.repository.path(),
        &ForcedCheckpointConsumesItsPermitBeforeOrdinaryReconciliationSettlesItState {
            blocked_root: fixture.blocked_root,
            blocked_id: fixture.blocked_id,
            holder_id: fixture.holder_id,
            base,
            checkpoint,
        },
    );
    drop(fixture.worktrees);
}

pub(super) const BEFORE_PERMIT_CONSUMPTION_WAITS_FOR_COMMITTED_AND_ABORTED_DOES_NOT_SPEND_IT:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-permit-consumption-waits-for-committed-and-aborted-does-not-spend-it",
    build_before_permit_consumption_waits_for_committed_and_aborted_does_not_spend_it,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PermitConsumptionWaitsForCommittedAndAbortedDoesNotSpendItState {
    pub(super) blocked_root: PathBuf,
    pub(super) blocked_id:   String,
    pub(super) holder_id:    String,
    pub(super) base:         String,
    pub(super) blocked_head: String,
    pub(super) phase_log:    PathBuf,
    pub(super) hook_path:    PathBuf,
}

fn build_before_permit_consumption_waits_for_committed_and_aborted_does_not_spend_it(
    destination: &Path,
) {
    const ABORT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_ABORT_REFERENCE_TRANSACTION";

    let deferred_pair = deferred_pair(initialized_repository());
    let repository = &deferred_pair.repository;
    let base = git_stdout(repository.path(), &["rev-parse", "main"]);
    let blocked_head = commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub(super) fn phase_work() {}\n",
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
    write_private_hook(&hook_path, wrapper);
    let mut permissions = fs::metadata(&hook_path)
        .expect("phase wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&hook_path, permissions).expect("phase wrapper should be executable");

    recorded_history::save(
        destination,
        deferred_pair.repository.path(),
        &PermitConsumptionWaitsForCommittedAndAbortedDoesNotSpendItState {
            blocked_root: deferred_pair.blocked_root,
            blocked_id: deferred_pair.blocked_id,
            holder_id: deferred_pair.holder_id,
            base,
            blocked_head,
            phase_log,
            hook_path,
        },
    );
    drop(deferred_pair.worktrees);
}

pub(super) const BEFORE_PREPARED_GATE_SCOPED_COMPARISON: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "gate-prepared-scoped-comparison",
        build_before_prepared_gate_comparison,
    );

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedGateComparisonState {
    pub(super) reservation_ids: Vec<String>,
    pub(super) actual:          String,
    pub(super) proposed:        String,
}

fn build_before_prepared_gate_comparison(destination: &Path) {
    let fixture = {
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
    };
    recorded_history::save(
        destination,
        fixture.repository.path(),
        &PreparedGateComparisonState {
            reservation_ids: fixture.reservation_ids,
            actual:          fixture.actual,
            proposed:        fixture.proposed,
        },
    );
}

pub(super) const HISTORY_POST_COMMIT_RESERVATION_CARDINALITY_TRACE_1:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-reservation-cardinality-trace-1",
    |destination| build_history_post_commit_reservation_cardinality_trace(destination, 1),
);

pub(super) const HISTORY_POST_COMMIT_RESERVATION_CARDINALITY_TRACE_3:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-reservation-cardinality-trace-3",
    |destination| build_history_post_commit_reservation_cardinality_trace(destination, 3),
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PostCommitReservationCardinalityTraceHistory {
    pub(super) subject_root: PathBuf,
}

fn build_history_post_commit_reservation_cardinality_trace(
    destination: &Path,
    reservation_count: usize,
) {
    let fixture = PostCommitCellFixture::new();

    let repository_root = fixture.repository.path();
    let subject_root = fixture.subject_root.as_path();
    dirty_source(repository_root, "hook-incursion/entered.txt");
    assert!(
        claim(
            repository_root,
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
                subject_root,
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
        git(subject_root, &["add", &subject_path]);
        git(
            subject_root,
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
    git(subject_root, &["add", "hook-incursion/entered.txt"]);
    git(
        subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "hook incursion work",
        ],
    );

    recorded_history::save(
        destination,
        repository_root,
        &PostCommitReservationCardinalityTraceHistory {
            subject_root: subject_root.to_path_buf(),
        },
    );
    drop(fixture.worktrees);
}

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P1C1:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p1c1",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 1, 1),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P1C2:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p1c2",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 1, 2),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P2C1:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p2c1",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 2, 1),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P2C2:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p2c2",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 2, 2),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C0:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p0c0",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 0, 0),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C1:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p0c1",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 0, 1),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C2:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p0c2",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 0, 2),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C14:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p0c14",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 0, 14),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C100:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p0c100",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 0, 100),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P1C0:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p1c0",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 1, 0),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P2C0:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p2c0",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 2, 0),
);

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P4C0:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p4c0",
    |destination| build_history_post_commit_path_commit_cardinality_trace(destination, 4, 0),
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PostCommitPathCommitCardinalityTraceHistory {
    pub(super) subject_root: PathBuf,
}

fn build_history_post_commit_path_commit_cardinality_trace(
    destination: &Path,
    path_count: usize,
    commit_count: usize,
) {
    let fixture = PostCommitCellFixture::new();

    let repository_root = fixture.repository.path();
    let subject_root = fixture.subject_root.as_path();
    for index in 0..path_count {
        dirty_source(
            repository_root,
            &format!("hook-cardinality/path-{index}.txt"),
        );
    }
    assert!(
        claim(
            repository_root,
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
            subject_root,
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
    let commits = (0..commit_count)
        .map(|commit_index| {
            let written_files = if path_count == 0 {
                vec![(
                    "subject.txt".to_owned(),
                    format!("subject version {commit_index}\n"),
                )]
            } else if commit_index == 0 {
                (0..path_count)
                    .map(|path_index| {
                        (
                            format!("hook-cardinality/path-{path_index}.txt"),
                            format!("path {path_index}, version {commit_index}\n"),
                        )
                    })
                    .collect()
            } else {
                let path_index = commit_index % path_count;
                vec![(
                    format!("hook-cardinality/path-{path_index}.txt"),
                    format!("path {path_index}, version {commit_index}\n"),
                )]
            };
            FixtureCommit {
                message: format!("hook cardinality commit {commit_index}"),
                written_files,
            }
        })
        .collect::<Vec<_>>();
    commit_files_without_hooks(subject_root, POST_COMMIT_SUBJECT_BRANCH, &commits);
    if commit_count == 0 {
        for path_index in 0..path_count {
            fs::write(
                subject_root.join(format!("hook-cardinality/path-{path_index}.txt")),
                format!("uncommitted path {path_index}\n"),
            )
            .expect("uncommitted cardinality path should write");
        }
    }

    recorded_history::save(
        destination,
        repository_root,
        &PostCommitPathCommitCardinalityTraceHistory {
            subject_root: subject_root.to_path_buf(),
        },
    );
    drop(fixture.worktrees);
}

pub(super) const HISTORY_TRACE_RETENTION_REF_RECONCILIATION_REPAIR_1:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-trace-retention-ref-reconciliation-repair-1",
    |destination| {
        build_history_trace_retention_ref_reconciliation(
            destination,
            1,
            RetentionRefPass::RepairOnly,
        );
    },
);

pub(super) const HISTORY_TRACE_RETENTION_REF_RECONCILIATION_REPAIR_3:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-trace-retention-ref-reconciliation-repair-3",
    |destination| {
        build_history_trace_retention_ref_reconciliation(
            destination,
            3,
            RetentionRefPass::RepairOnly,
        );
    },
);

pub(super) const HISTORY_TRACE_RETENTION_REF_RECONCILIATION_DELETE_1:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-trace-retention-ref-reconciliation-delete-1",
    |destination| {
        build_history_trace_retention_ref_reconciliation(
            destination,
            1,
            RetentionRefPass::DeletionOnly,
        );
    },
);

pub(super) const HISTORY_TRACE_RETENTION_REF_RECONCILIATION_DELETE_3:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-trace-retention-ref-reconciliation-delete-3",
    |destination| {
        build_history_trace_retention_ref_reconciliation(
            destination,
            3,
            RetentionRefPass::DeletionOnly,
        );
    },
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TraceRetentionRefReconciliationHistory {
    pub(super) reservation_ids: Vec<String>,
    pub(super) sentinel_log:    PathBuf,
}

fn build_history_trace_retention_ref_reconciliation(
    destination: &Path,
    reservation_count: usize,
    pass: RetentionRefPass,
) {
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
            git(
                repository.path(),
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "branch",
                    "--force",
                    "main",
                    RETENTION_TRACE_BRANCH,
                ],
            );
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

    recorded_history::save(
        destination,
        repository.path(),
        &TraceRetentionRefReconciliationHistory {
            reservation_ids,
            sentinel_log,
        },
    );
}

pub(super) const HISTORY_PREPARED_TRUNK_UPDATE_HOOK_PHASES_READY:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-prepared-trunk-update-hook-phases-ready",
    build_history_prepared_trunk_update_hook_phases,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PreparedTrunkUpdateHookPhasesHistory {
    pub(super) base:   String,
    pub(super) target: String,
}

fn build_history_prepared_trunk_update_hook_phases(destination: &Path) {
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

    recorded_history::save(
        destination,
        repository.path(),
        &PreparedTrunkUpdateHookPhasesHistory { base, target },
    );
}

pub(super) const HISTORY_COMMITTED_FEATURE_REBASE_HOOK_PHASES_READY:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-committed-feature-rebase-hook-phases-ready",
    build_history_committed_feature_rebase_hook_phases,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CommittedFeatureRebaseHookPhasesHistory {}

fn build_history_committed_feature_rebase_hook_phases(destination: &Path) {
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

    recorded_history::save(
        destination,
        repository.path(),
        &CommittedFeatureRebaseHookPhasesHistory {},
    );
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct IntegrateMovesThePresentTargetAndMissingTargetFallsBackToMainHistory {
    pub(super) lane:               PathBuf,
    pub(super) id:                 String,
    pub(super) tip:                String,
    pub(super) main_before:        String,
    pub(super) integration_before: String,
}

pub(super) const HISTORY_INTEGRATE_MOVES_THE_PRESENT_TARGET_AND_MISSING_TARGET_FALLS_BACK_TO_MAIN_PRESENT: cargo_berth_test_support::LedgerTemplate = recorded_history::template("gate-integrate-moves-the-present-target-and-missing-target-falls-back-to-main-present", |destination| build_history_integrate_moves_the_present_target_and_missing_target_falls_back_to_main(destination, TargetPresence::Present));

pub(super) const HISTORY_INTEGRATE_MOVES_THE_PRESENT_TARGET_AND_MISSING_TARGET_FALLS_BACK_TO_MAIN_MISSING: cargo_berth_test_support::LedgerTemplate = recorded_history::template("gate-integrate-moves-the-present-target-and-missing-target-falls-back-to-main-missing", |destination| build_history_integrate_moves_the_present_target_and_missing_target_falls_back_to_main(destination, TargetPresence::Missing));

#[derive(Clone, Copy)]
pub(super) enum TargetPresence {
    Present,
    Missing,
}

fn build_history_integrate_moves_the_present_target_and_missing_target_falls_back_to_main(
    destination: &Path,
    target_presence: TargetPresence,
) {
    let repository = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let lane = repository.lane(
        match target_presence {
            TargetPresence::Present => "present-lane",
            TargetPresence::Missing => "missing-lane",
        },
        "integration",
    );
    write_file(&lane, "work.txt", "work\n");
    let claimed = repository.claim(&lane, "file:work.txt", FIRST_RUN, Some("integration"));
    assert_success(&claimed);
    let id = claim_id(&claimed);
    repository.commit_file(&lane, "work.txt", "work\n", "lane work");
    let tip = repository.git_stdout(&lane, &["rev-parse", "HEAD"]);
    let main_before = repository.git_stdout(repository.root(), &["rev-parse", "main"]);
    let integration_before =
        repository.git_stdout(repository.root(), &["rev-parse", "integration"]);
    if matches!(target_presence, TargetPresence::Missing) {
        repository.remove_integration_branch();
    }

    recorded_history::save(
        destination,
        repository.root(),
        &IntegrateMovesThePresentTargetAndMissingTargetFallsBackToMainHistory {
            lane,
            id,
            tip,
            main_before,
            integration_before,
        },
    );
}

pub(super) const HISTORY_ASSERT_REFERENCE_TRANSACTION_REJECTS_STALE_MARKER_PREPARED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-assert-reference-transaction-rejects-stale-marker-prepared",
    build_history_assert_reference_transaction_rejects_stale_marker,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AssertReferenceTransactionRejectsStaleMarkerHistory {
    pub(super) marker_input: String,
}

fn build_history_assert_reference_transaction_rejects_stale_marker(destination: &Path) {
    let marker_repository = initialized_repository();

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

    recorded_history::save(
        destination,
        marker_repository.path(),
        &AssertReferenceTransactionRejectsStaleMarkerHistory { marker_input },
    );
}

pub(super) const HISTORY_ASSERT_REFERENCE_TRANSACTION_REJECTS_STALE_SESSION_MAPPING_PREPARED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-assert-reference-transaction-rejects-stale-session-mapping-prepared",
    |destination| {
        build_history_assert_reference_transaction_rejects_stale_session_mapping(destination);
    },
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AssertReferenceTransactionRejectsStaleSessionMappingHistory {
    pub(super) session_input: String,
}

fn build_history_assert_reference_transaction_rejects_stale_session_mapping(destination: &Path) {
    let session_repository = initialized_repository();

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

    recorded_history::save(
        destination,
        session_repository.path(),
        &AssertReferenceTransactionRejectsStaleSessionMappingHistory { session_input },
    );
}

pub(super) const HISTORY_ASSERT_REFERENCE_TRANSACTION_REJECTS_SESSION_WORKTREE_MISMATCH_PREPARED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-assert-reference-transaction-rejects-session-worktree-mismatch-prepared",
    |destination| {
        build_history_assert_reference_transaction_rejects_session_worktree_mismatch(destination);
    },
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AssertReferenceTransactionRejectsSessionWorktreeMismatchHistory {
    pub(super) second_root:    PathBuf,
    pub(super) mismatch_input: String,
}

fn build_history_assert_reference_transaction_rejects_session_worktree_mismatch(
    destination: &Path,
) {
    let mismatch_repository = initialized_repository();

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

    recorded_history::save(
        destination,
        mismatch_repository.path(),
        &AssertReferenceTransactionRejectsSessionWorktreeMismatchHistory {
            second_root,
            mismatch_input,
        },
    );
}

pub(super) const HISTORY_FORCED_INTEGRATION_PHASES: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "gate-forced-integration-phases",
        build_history_forced_integration_phases,
    );

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ForcedIntegrationPhasesHistory {
    pub(super) blocked_root: PathBuf,
    pub(super) blocked_id:   String,
    pub(super) holder_id:    String,
}

fn build_history_forced_integration_phases(destination: &Path) {
    let deferred_pair = deferred_pair(initialized_repository());
    let repository = &deferred_pair.repository;
    commit_work(
        &deferred_pair.blocked_root,
        "src/lib.rs",
        "pub(super) fn forced_work() {}\n",
        "forced integration work",
    );
    set_gate_mode(repository.path(), "enforce");

    recorded_history::save(
        destination,
        repository.path(),
        &ForcedIntegrationPhasesHistory {
            blocked_root: deferred_pair.blocked_root,
            blocked_id:   deferred_pair.blocked_id,
            holder_id:    deferred_pair.holder_id,
        },
    );
    drop(deferred_pair.worktrees);
}

pub(super) const RECOVERY_BRANCH_HISTORIES: [(&str, cargo_berth_test_support::LedgerTemplate); 3] = [
    (
        "abandoned",
        recorded_history::template("gate-recovery-branch-abandoned", |root| {
            build_recovery_branch_history(root, "abandoned", RecoveryBranchDisposition::Abandon);
        }),
    ),
    (
        "retired",
        recorded_history::template("gate-recovery-branch-retired", |root| {
            build_recovery_branch_history(root, "retired", RecoveryBranchDisposition::Retire);
        }),
    ),
    (
        "rewritten",
        recorded_history::template("gate-recovery-branch-rewritten", |root| {
            build_recovery_branch_history(root, "rewritten", RecoveryBranchDisposition::Rewrite);
        }),
    ),
];

#[derive(Clone, Copy)]
pub(super) enum RecoveryBranchDisposition {
    Abandon,
    Retire,
    Rewrite,
}

fn build_recovery_branch_history(
    destination: &Path,
    branch: &str,
    disposition: RecoveryBranchDisposition,
) {
    let repository = initialized_repository();
    let root = repository.path();
    let hooked_git = HookedGit::new();
    hooked_git.run(root, &["switch", "--quiet", "-c", branch]);
    hooked_git.commit_file(root, branch, "work\n", &format!("{branch} work"));
    match disposition {
        RecoveryBranchDisposition::Abandon => resolve_branch_work(root, branch, "--abandon"),
        RecoveryBranchDisposition::Retire => resolve_branch_work(root, branch, "--retire-orphan"),
        RecoveryBranchDisposition::Rewrite => {},
    }
    recorded_history::save(destination, root, &());
}

pub(super) const HISTORY_LISTED_TARGET: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-listed-target", build_listed_target_history);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ListedTargetHistory {
    pub(super) lane: PathBuf,
    pub(super) id:   String,
}

fn build_listed_target_history(destination: &Path) {
    let repository = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let lane = repository.lane("target-list-lane", "integration");
    write_file(&lane, "listed.txt", "listed\n");
    let claimed = repository.claim(&lane, "file:listed.txt", FIRST_RUN, Some("integration"));
    assert_success(&claimed);

    let id = claim_id(&claimed);
    recorded_history::save(
        destination,
        repository.root(),
        &ListedTargetHistory { lane, id },
    );
}

pub(super) const HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P33C0:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "gate-post-commit-path-commit-cardinality-trace-p33c0",
    |root| build_history_post_commit_path_commit_cardinality_trace(root, 33, 0),
);

pub(super) const HISTORY_SETTLED_PREDECESSOR_GATE: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "gate-settled-predecessor",
        build_history_settled_predecessor_gate,
    );
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SettledPredecessorGateHistory {
    pub(super) holder_id:  String,
    pub(super) blocked_id: String,
    pub(super) input:      String,
}
fn build_history_settled_predecessor_gate(destination: &Path) {
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
        "fn integrated() {}\n",
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
        "fn successor() {}\n",
        "successor work",
    );
    let witness = commit_work_without_hooks(
        root,
        "src/lib.rs",
        "fn integrated() {}\n",
        "rewritten predecessor witness",
    );
    assert_ne!(checkpoint, witness);
    set_gate_mode(root, "enforce");
    let input = format!("{witness} {successor} refs/heads/main\n");

    recorded_history::save(
        destination,
        root,
        &SettledPredecessorGateHistory {
            holder_id: fixture.holder_id,
            blocked_id: fixture.blocked_id,
            input,
        },
    );
    drop(fixture.worktrees);
}

pub(super) const HISTORY_DEFERRED_ENDPOINT_REQUESTER: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "gate-deferred-endpoint-requester",
        build_deferred_endpoint_requester,
    );
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct DeferredEndpointsHistory {
    pub(super) base:           String,
    pub(super) requester_head: String,
    pub(super) requester_root: PathBuf,
    pub(super) requester_id:   String,
    pub(super) holder_id:      String,
}
fn build_deferred_endpoint_requester(destination: &Path) {
    let repository =
        BEFORE_OBSERVE_ENFORCE_AND_ONE_USE_FORCE_APPLY_TO_BOTH_DEFERRED_ENDPOINTS.instantiate();
    let ObserveEnforceAndOneUseForceApplyToBothDeferredEndpointsState {
        base,
        holder_id,
        requester_id,
        holder_head,
        requester_root,
    } = recorded_history::read(repository.path());
    let holder_observed = propose_trunk(repository.path(), &base, &holder_head);
    assert!(holder_observed.status.success());
    let holder_observation = String::from_utf8_lossy(&holder_observed.stderr);
    assert!(
        holder_observation.contains(OBSERVED_TRUNK_UPDATE_OF_ONE_HELD_RESERVATION),
        "{holder_observation}"
    );
    restore_trunk(repository.path(), &holder_head, &base);

    let requester_head = commit_work(
        &requester_root,
        "src/lib.rs",
        "pub fn requester_work() {}\n",
        "requester work",
    );

    recorded_history::save(
        destination,
        repository.path(),
        &DeferredEndpointsHistory {
            base,
            requester_head,
            requester_root,
            requester_id,
            holder_id,
        },
    );
}

pub(super) const HISTORY_DEFERRED_ENDPOINT_FORCE: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "gate-deferred-endpoint-force",
        build_deferred_endpoint_force,
    );

fn build_deferred_endpoint_force(destination: &Path) {
    let repository = HISTORY_DEFERRED_ENDPOINT_REQUESTER.instantiate();
    let DeferredEndpointsHistory {
        base,
        requester_head,
        requester_root,
        requester_id,
        holder_id,
    } = recorded_history::read(repository.path());
    let observed = propose_trunk(repository.path(), &base, &requester_head);
    assert!(observed.status.success());
    assert!(
        String::from_utf8_lossy(&observed.stderr)
            .contains(OBSERVED_TRUNK_UPDATE_OF_ONE_HELD_RESERVATION)
    );
    restore_trunk(repository.path(), &requester_head, &base);

    recorded_history::save(
        destination,
        repository.path(),
        &DeferredEndpointsHistory {
            base,
            requester_head,
            requester_root,
            requester_id,
            holder_id,
        },
    );
}

pub(super) const HISTORY_DEFERRED_ENDPOINT_REPLAY: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template(
        "gate-deferred-endpoint-replay",
        build_deferred_endpoint_replay,
    );

fn build_deferred_endpoint_replay(destination: &Path) {
    let repository = HISTORY_DEFERRED_ENDPOINT_FORCE.instantiate();
    let DeferredEndpointsHistory {
        base,
        requester_head,
        requester_root,
        requester_id,
        holder_id,
    } = recorded_history::read(repository.path());
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

    recorded_history::save(
        destination,
        repository.path(),
        &DeferredEndpointsHistory {
            base,
            requester_head,
            requester_root,
            requester_id,
            holder_id,
        },
    );
}

pub(super) const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";

/// The observe-only gate's line for a trunk update that brings in one held reservation.
pub(super) const OBSERVED_TRUNK_UPDATE_OF_ONE_HELD_RESERVATION: &str = "cargo-berth (observe-only): enforcing mode would refuse this update of refs/heads/main, where 1 entering reservation is held; see `cargo-berth board`.";

pub(super) fn assert_denial_context(denial: &str, requester_id: &str, holder_id: &str) {
    for required in [
        requester_id,
        holder_id,
        "docs/requester-plan.md",
        "phase-8",
        "protected paths: 1 scope.",
        "covered paths: 1 scope;",
        "cargo-berth sequence",
        "cargo-berth integrate",
        "CARGO_BERTH_BYPASS=1",
    ] {
        assert!(
            denial.contains(required),
            "denial omitted {required}: {denial}"
        );
    }
    // The gate counts scopes; the board and the integrate payload list them.
    assert!(!denial.contains("file:src/lib.rs"), "{denial}");
}

pub(super) fn assert_forced_permit_consumed(repository_root: &Path) {
    let journal = journal_text(repository_root);
    for operation in [
        "\"op\":\"forced_integration_permit\"",
        "\"op\":\"consume_forced_integration_permit\"",
    ] {
        assert_eq!(journal.matches(operation).count(), 1);
    }
    assert!(journal.contains("\"kind\":\"forced_integration\""));
}

/// Propose `proposed` as the new trunk tip with the release valve unset.
pub(super) fn propose_trunk(repository_root: &Path, previous: &str, proposed: &str) -> Output {
    propose_branch(repository_root, "main", previous, proposed)
}

pub(super) fn propose_branch(
    repository_root: &Path,
    branch: &str,
    previous: &str,
    proposed: &str,
) -> Output {
    update_branch(
        repository_root,
        branch,
        previous,
        proposed,
        ReleaseValve::Unset,
    )
}

/// Put trunk back to `previous` after a permitted observation.
pub(super) fn restore_trunk(repository_root: &Path, current: &str, previous: &str) {
    assert!(
        propose_trunk(repository_root, current, previous)
            .status
            .success()
    );
}

/// Whether the release valve is set for one invocation.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ReleaseValve {
    Unset,
    Set,
}

pub(super) fn update_branch(
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

pub(super) fn journal_text(repository_root: &Path) -> String {
    fs::read_to_string(repository_root.join(JOURNAL_PATH)).expect("journal should read")
}

pub(super) const HISTORY_POST_COMMIT_CELL: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-post-commit-cell", build_history_post_commit_cell);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PostCommitCellHistory {
    pub(super) subject_root: PathBuf,
}
fn build_history_post_commit_cell(destination: &Path) {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let subject_root = add_worktree(
        repository.path(),
        worktrees.path(),
        POST_COMMIT_SUBJECT_BRANCH,
    );

    recorded_history::save(
        destination,
        repository.path(),
        &PostCommitCellHistory { subject_root },
    );
    drop(worktrees);
}

/// The committed feature history before the gate waits on a held ledger lock.
pub(super) const HISTORY_LOCK_DEADLINE: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-lock-deadline", build_history_lock_deadline);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct LockDeadlineHistory {
    pub(super) base:         String,
    pub(super) feature_head: String,
}
fn build_history_lock_deadline(destination: &Path) {
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

    recorded_history::save(
        destination,
        repository.path(),
        &LockDeadlineHistory { base, feature_head },
    );
    drop(worktrees);
}

pub(super) const HISTORY_LOWER_BEFORE_UPDATE_REFS: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-lower-before-update-refs", |destination| {
        let repository = BEFORE_ACTIVE_LOWER_RESERVATION_KEEPS_BOTH_SPLIT_COMMITS_AFTER_UPDATE_REFS
            .instantiate();
        let ActiveLowerReservationKeepsBothSplitCommitsAfterUpdateRefsState {
            trunk,
            first_contents,
            full_contents,
            holder,
            id,
            old_lower,
            old_upper,
        } = recorded_history::read(repository.path());
        let root = repository.path();
        let worktrees = root.join(".git/fixture-worktrees");
        let editor = worktrees.as_path().join("split-editor");
        split_rebase::stop_for_split(&holder, &editor, &trunk);
        let first_split = commit_work(&holder, "phase.txt", &first_contents, "first split hunk");
        let second_split = commit_work(&holder, "phase.txt", &full_contents, "second split hunk");

        recorded_history::save(
            destination,
            root,
            &LowerBeforeUpdateRefsHistory {
                trunk,
                holder,
                id,
                old_lower,
                old_upper,
                first_split,
                second_split,
            },
        );
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct LowerBeforeUpdateRefsHistory {
    pub(super) trunk:        String,
    pub(super) holder:       PathBuf,
    pub(super) id:           String,
    pub(super) old_lower:    String,
    pub(super) old_upper:    String,
    pub(super) first_split:  String,
    pub(super) second_split: String,
}

pub(super) fn run_private_hook(repository_root: &Path, phase: &str, input: &str) -> Output {
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

pub(super) const REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT: &str =
    "CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY";

pub(super) fn assert_prepared_subject_pass(fixture: &PreparedGateScopedComparisonFixture) {
    let input = format!("{} {} refs/heads/main\n", fixture.actual, fixture.proposed);
    let before = journal_text(fixture.repository.path());
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

    let after = journal_text(fixture.repository.path());
    let added = after
        .strip_prefix(&before)
        .expect("a prepared call must preserve prior records");
    let previous = before
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("prior record should decode")
        })
        .filter(|record| record["op"] == "scoped_patch_equivalence_checked")
        .collect::<Vec<_>>();
    let comparisons = added
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("new record should decode")
        })
        .filter(|record| record["op"] == "scoped_patch_equivalence_checked")
        .collect::<Vec<_>>();
    let mut expected_comparisons = 0;
    for target in [&fixture.actual, &fixture.proposed] {
        let already_compared = previous
            .iter()
            .filter(|record| record["target"] == *target)
            .map(|record| {
                record["reservation_id"]
                    .as_str()
                    .expect("comparison subject should decode")
            })
            .collect::<Vec<_>>();
        let has_uncompared_subject = fixture
            .reservation_ids
            .iter()
            .any(|subject| !already_compared.contains(&subject.as_str()));
        let expected_for_target = usize::from(has_uncompared_subject);
        expected_comparisons += expected_for_target;
        let written = comparisons
            .iter()
            .filter(|record| record["target"] == *target)
            .collect::<Vec<_>>();
        assert_eq!(
            written.len(),
            expected_for_target,
            "this live call must advance target {target}: {added}"
        );
        for record in written {
            let subject = record["reservation_id"]
                .as_str()
                .expect("new comparison needs its subject");
            assert!(
                fixture.reservation_ids.iter().any(|id| id == subject),
                "{record}"
            );
            assert!(
                !already_compared.contains(&subject),
                "this live call repeated an earlier subject: {record}"
            );
            assert_eq!(record["verdict"], "integrated", "{record}");
        }
    }
    assert_eq!(
        comparisons.len(),
        expected_comparisons,
        "the live call must compare only the two requested targets: {added}"
    );
}

fn build_prepared_subject_pass(destination: &Path, prior_passes: usize) {
    let worktrees = tempdir().expect("worktree parent should exist");
    let repository = BEFORE_PREPARED_GATE_SCOPED_COMPARISON
        .instantiate_with_external_worktrees(worktrees.path());
    let PreparedGateComparisonState {
        reservation_ids,
        actual,
        proposed,
    } = recorded_history::read(repository.path());
    let fixture = PreparedGateScopedComparisonFixture {
        repository,
        _worktrees: worktrees,
        reservation_ids,
        actual,
        proposed,
    };
    for _ in 0..prior_passes {
        assert_prepared_subject_pass(&fixture);
    }
    recorded_history::save(
        destination,
        fixture.repository.path(),
        &PreparedGateComparisonState {
            reservation_ids: fixture.reservation_ids,
            actual:          fixture.actual,
            proposed:        fixture.proposed,
        },
    );
}

pub(super) const HISTORY_PREPARED_SUBJECT_PASS_1: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-prepared-subject-pass-1", |destination| {
        build_prepared_subject_pass(destination, 1);
    });

pub(super) const HISTORY_PREPARED_SUBJECT_PASS_2: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-prepared-subject-pass-2", |destination| {
        build_prepared_subject_pass(destination, 2);
    });

pub(super) const HISTORY_PREPARED_SUBJECT_PASS_3: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-prepared-subject-pass-3", |destination| {
        build_prepared_subject_pass(destination, 3);
    });

pub(super) const HISTORY_PREPARED_SUBJECT_PASS_4: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("gate-prepared-subject-pass-4", |destination| {
        build_prepared_subject_pass(destination, 4);
    });
