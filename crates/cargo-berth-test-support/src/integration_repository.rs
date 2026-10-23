//! A repository whose lanes target an `integration` branch instead of `main`.
//!
//! `IntegrationRepository` initializes berth on `main`, checks `integration`
//! out in a linked worktree, and adds lanes that target either branch. Its
//! methods run git and `cargo-berth` with the executable the test crate passed
//! to `IntegrationRepository::new`; the free functions here only read what those
//! runs produced.

use std::env;
use std::fs;
use std::fs::File;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;
use tempfile::Builder;
use tempfile::TempDir;
use tempfile::tempdir;

use crate::DirectorySnapshot;
use crate::LedgerTemplate;
use crate::RepositoryTemplate;
use crate::berth_command;
use crate::git_driver::GitDriver;
use crate::git_driver::OptionalLocks;
use crate::repository_template;
use crate::repository_template::IdentityRenewal;

/// The lock that gives one process ownership of a fixture build.
const BUILD_LOCK_FILE_NAME: &str = "build.lock";
/// The marker published only after every fixture directory is complete.
const COMPLETE_FILE_NAME: &str = "complete";
/// The main checkout within a completed fixture build.
const REPOSITORY_DIRECTORY_NAME: &str = "repository";
/// The linked checkouts within an integration pair build.
const WORKTREES_DIRECTORY_NAME: &str = "worktrees";

/// Variables cleared before git runs, so a hook cannot inherit this process's
/// coordination run, session, or repository location.
const CLEARED_GIT_ENVIRONMENT: &[&str] = &[
    "CARGO_BERTH_RUN",
    "CARGO_BERTH_SESSION_ID",
    "GIT_DIR",
    "GIT_COMMON_DIR",
];

/// What [`IntegrationRepository::capture`] took from one repository, for
/// [`IntegrationRepository::restore`] to write back.
pub struct IntegrationSnapshot {
    /// The main worktree, with the `.git` directory every linked worktree shares.
    repository: DirectorySnapshot,
    /// The parent of every linked worktree, `integration` and each lane.
    worktrees:  DirectorySnapshot,
}

/// A temporary repository with berth initialized on `main` and an `integration`
/// branch checked out in a linked worktree.
pub struct IntegrationRepository {
    executable:      &'static str,
    repository:      TempDir,
    worktrees:       WorktreeDirectory,
    /// The canonical path of the linked worktree that has `integration` checked
    /// out.
    pub integration: PathBuf,
}

/// Keeps ordinary external worktrees alive, or locates linked checkouts copied with a ledger.
enum WorktreeDirectory {
    Temporary(TempDir),
    InRepository(PathBuf),
}

impl WorktreeDirectory {
    fn path(&self) -> &Path {
        match self {
            Self::Temporary(directory) => directory.path(),
            Self::InRepository(path) => path,
        }
    }
}

/// The reservation ids attached to the two lanes of a recorded cross-target pair.
pub struct CrossTargetPair {
    /// The initialized checkout and its integration target.
    pub repository:     IntegrationRepository,
    /// Lane A's released work targeting integration.
    pub predecessor_id: String,
    /// Lane X's work targeting main.
    pub successor_id:   String,
}

/// Whether X waits for an explicit sequence decision or already records A before X.
#[derive(Clone, Copy)]
pub enum CrossTargetOrdering {
    /// X has answered the overlap but no ordering edge has been chosen yet.
    Deferred,
    /// X's claim already records the edge from A to X.
    After,
}

impl IntegrationRepository {
    /// Build the repository in a temporary directory with the default name prefix.
    ///
    /// `executable` is the test crate's own `env!("CARGO_BIN_EXE_cargo-berth")`,
    /// which only that crate can expand. Every `cargo-berth` run and every git
    /// command this repository issues uses it.
    ///
    /// # Panics
    ///
    /// Panics when a temporary directory cannot be created, or when git or
    /// `cargo-berth init` fails.
    #[must_use]
    pub fn new(executable: &'static str) -> Self {
        Self::with_repository_prefix(executable, ".tmp")
    }

    /// Build the repository in a temporary directory whose name starts with
    /// `prefix`.
    ///
    /// `executable` is as for [`Self::new`].
    ///
    /// # Panics
    ///
    /// Panics when a temporary directory cannot be created, or when git or
    /// `cargo-berth init` fails.
    #[must_use]
    pub fn with_repository_prefix(executable: &'static str, prefix: &str) -> Self {
        PreparedIntegrationPair::instantiate(executable, prefix, "default", |root| {
            Self::build_main_template(root, executable);
        })
    }

    /// Build the initialized `main` checkout shared by integration-target fixtures.
    ///
    /// Its configuration is committed and its ledger is empty, so a
    /// [`RepositoryTemplate`] can copy it without replaying setup in each test.
    /// Linked worktrees and their hooked commits are added by [`Self::from_template`].
    ///
    /// # Panics
    ///
    /// Panics when a fixture file cannot be written or git or berth fails.
    pub fn build_main_template(root: &Path, executable: &'static str) {
        let seed = InitializedMainSeed::prepare(executable);
        let source = &seed.root;
        let destination = fs::canonicalize(root).expect("main template should canonicalize");
        let mut snapshot = DirectorySnapshot::capture(source);
        let _ = snapshot.replace_text(&source.to_string_lossy(), &destination.to_string_lossy());
        IdentityRenewal::read(source).apply(&mut snapshot);
        snapshot.restore(&destination);
    }

    fn build_main_seed(root: &Path, executable: &'static str) {
        let git = GitDriver {
            executable,
            optional_locks: OptionalLocks::Taken,
            cleared_environment: CLEARED_GIT_ENVIRONMENT,
        };
        git.run(root, ["init", "--quiet", "--initial-branch=main"]);
        // Write fixture-only identity and maintenance policy together, instead of
        // starting four git processes before the first commit.
        let configuration = root.join(".git/config");
        let mut contents = fs::read_to_string(&configuration).expect("git config should read");
        contents.push_str("\n[user]\n name = Berth Test\n email = berth@example.invalid\n[maintenance]\n auto = false\n[gc]\n auto = 0\n");
        fs::write(configuration, contents).expect("git config should write");
        write_file(root, "README.md", "initial\n");
        git.run(root, ["add", "README.md"]);
        git.run(root, ["commit", "--quiet", "-m", "initial"]);
        let initialized = berth_command::berth_command(executable)
            .args(["init", "--json"])
            .current_dir(root)
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_SESSION_ID")
            .output()
            .expect("cargo-berth init should run");
        assert_success(&initialized);
        git.run(root, ["add", "--force", ".claude/config/berth.toml"]);
        // The gate's unreserved-trunk test proves these hooks write no state.
        git.run_without_hooks(
            root,
            ["commit", "--quiet", "-m", "track berth configuration"],
        );
    }

    /// Build source and policy for merge-extent observations with explicit reconciliation.
    ///
    /// # Panics
    /// Panics when source, configuration, or repository initialization fails.
    pub fn build_merge_extent_main(root: &Path, executable: &'static str) {
        let git = GitDriver {
            executable,
            optional_locks: OptionalLocks::Refused,
            cleared_environment: &[
                "CARGO_BERTH_RUN",
                "CARGO_BERTH_SESSION_ID",
                "CARGO_BERTH_BYPASS",
            ],
        };
        git.run(root, ["init", "--quiet", "--initial-branch=main"]);
        let configuration = root.join(".git/config");
        let mut contents = fs::read_to_string(&configuration).expect("git config should read");
        contents.push_str("\n[user]\n name = Berth Test\n email = berth@example.invalid\n[maintenance]\n auto = false\n[gc]\n auto = 0\n");
        fs::write(&configuration, &contents).expect("fixture git config should write");
        write_file(root, "tracked.rs", "base\n");
        write_file(root, "staged.rs", "base\n");
        git.run(root, ["add", "."]);
        git.run(root, ["commit", "--quiet", "-m", "base"]);
        let initialized = berth_command::berth_command(executable)
            .args(["init", "--json"])
            .current_dir(root)
            .env("CARGO_BERTH_RUN", "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b")
            .env_remove("CARGO_BERTH_SESSION_ID")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("fixture init should run");
        assert_success(&initialized);
        let mut contents =
            fs::read_to_string(&configuration).expect("initialized git config should read");
        contents.push_str("\n[core]\n hooksPath = /dev/null\n");
        fs::write(configuration, contents).expect("fixture hook policy should write");
        git.run(root, ["add", "--force", ".claude/config/berth.toml"]);
        git.run(root, ["commit", "--quiet", "-m", "configure berth"]);
    }

    /// Prepare the independent holder and requester before merge-extent test clocks start.
    ///
    /// # Panics
    /// Panics when repository initialization or linked checkout creation fails.
    pub fn build_merge_extent_pair(root: &Path, executable: &'static str) {
        Self::build_merge_extent_main(root, executable);
        let git = GitDriver {
            executable,
            optional_locks: OptionalLocks::Refused,
            cleared_environment: &[
                "CARGO_BERTH_RUN",
                "CARGO_BERTH_SESSION_ID",
                "CARGO_BERTH_BYPASS",
            ],
        };
        for branch in ["holder", "outsider"] {
            let checkout = root.join(".git/fixture-worktrees").join(branch);
            git.add_worktree_without_hooks(root, &checkout, branch, "main");
        }
    }

    /// Prepare the initial hooked integration commit under the nextest setup clock.
    ///
    /// # Panics
    /// Panics when initialization, linked checkout creation, or its commit fails.
    pub fn build_initial_integration_pair(root: &Path, executable: &'static str) {
        Self::build_main_template(root, executable);
        let git = GitDriver {
            executable,
            optional_locks: OptionalLocks::Taken,
            cleared_environment: CLEARED_GIT_ENVIRONMENT,
        };
        let integration = root.join(".git/fixture-worktrees/integration");
        git.add_worktree_without_hooks(root, &integration, "integration", "main");
        write_file(&integration, "integration.txt", "integration\n");
        git.run(&integration, ["add", "integration.txt"]);
        git.run(
            &integration,
            ["commit", "--quiet", "-m", "integration start"],
        );
    }

    /// Copy an initialized `main` template with its berth policy already committed.
    ///
    /// The template must have no reservations or linked worktrees. Its integration checkout
    /// and first hooked commit are prepared once; each copy renews both checkout identities.
    ///
    /// # Panics
    ///
    /// Panics as [`RepositoryTemplate::instantiate`] does, or when git fails.
    #[must_use]
    pub fn from_template(template: RepositoryTemplate) -> Self {
        PreparedIntegrationPair::instantiate(template.executable, ".tmp", template.name, |root| {
            let main = template.instantiate();
            let main_root = fs::canonicalize(main.path()).expect("template main should resolve");
            let mut snapshot = DirectorySnapshot::capture(&main_root);
            drop(snapshot.replace_text(&main_root.to_string_lossy(), &root.to_string_lossy()));
            IdentityRenewal::read(&main_root).apply(&mut snapshot);
            snapshot.restore(root);
        })
    }

    /// Build a released A / waiting X pair with all linked directories under the root.
    ///
    /// # Panics
    /// Panics when any setup command or assertion fails.
    pub fn build_cross_target_pair(
        root: &Path,
        executable: &'static str,
        ordering: CrossTargetOrdering,
    ) {
        Self::build_main_template(root, executable);
        let git = GitDriver {
            executable,
            optional_locks: OptionalLocks::Taken,
            cleared_environment: CLEARED_GIT_ENVIRONMENT,
        };
        let worktrees = root.join(".git/fixture-worktrees");
        let integration = worktrees.join("integration");
        git.add_worktree_without_hooks(root, &integration, "integration", "main");
        write_file(&integration, "integration.txt", "integration\n");
        git.run(&integration, ["add", "integration.txt"]);
        git.run(
            &integration,
            ["commit", "--quiet", "-m", "integration start"],
        );
        let predecessor = worktrees.join("cross-a");
        let successor = worktrees.join("cross-x");
        git.add_worktree_without_hooks(root, &predecessor, "cross-a", "integration");
        git.add_worktree_without_hooks(root, &successor, "cross-x", "main");
        git.run(
            root,
            ["config", "branch.cross-a.cargoBerthTarget", "integration"],
        );
        git.run(root, ["config", "branch.cross-x.cargoBerthTarget", "main"]);
        let run = |checkout: &Path, arguments: &[&str]| {
            let output = berth_command::berth_command(executable)
                .args(arguments)
                .current_dir(checkout)
                .env_remove("CARGO_BERTH_RUN")
                .env_remove("CARGO_BERTH_SESSION_ID")
                .output()
                .expect("pair command should run");
            assert_success(&output);
            output
        };
        let first_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
        let second_run = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
        write_file(&predecessor, "shared.txt", "A\n");
        let predecessor_id = claim_id(&run(
            &predecessor,
            &[
                "claim",
                "file:shared.txt",
                "--run",
                first_run,
                "--target",
                "integration",
                "--json",
            ],
        ));
        let successor_id = match ordering {
            CrossTargetOrdering::Deferred => {
                let scope = deferring_run_scope(second_run);
                run(
                    &successor,
                    &["claim", &scope, "--run", second_run, "--json"],
                );
                claim_id(&run(
                    &successor,
                    &[
                        "claim",
                        "file:shared.txt",
                        "--run",
                        second_run,
                        "--defer",
                        &predecessor_id,
                        "--overlap-why",
                        "the order needs a decision",
                        "--json",
                    ],
                ))
            },
            CrossTargetOrdering::After => claim_id(&run(
                &successor,
                &[
                    "claim",
                    "file:shared.txt",
                    "--run",
                    second_run,
                    "--target",
                    "main",
                    "--after",
                    &predecessor_id,
                    "--overlap-why",
                    "A reaches main before X",
                    "--json",
                ],
            )),
        };
        git.run(&predecessor, ["add", "shared.txt"]);
        git.run(&predecessor, ["commit", "--quiet", "-m", "A work"]);
        run(&predecessor, &["release", &predecessor_id, "--json"]);
        fs::write(
            root.join(".git/cross-target.json"),
            serde_json::json!({"predecessor_id": predecessor_id, "successor_id": successor_id})
                .to_string(),
        )
        .expect("pair metadata should write");
    }

    /// Copy a cross-target pair, including the journal and each linked checkout.
    ///
    /// # Panics
    /// Panics when the recorded template or its reservation ids cannot be read.
    #[must_use]
    pub fn cross_target_pair(template: LedgerTemplate) -> CrossTargetPair {
        let repository = template.instantiate();
        let worktrees = repository.path().join(".git/fixture-worktrees");
        let metadata: Value = serde_json::from_slice(
            &fs::read(repository.path().join(".git/cross-target.json"))
                .expect("pair metadata should read"),
        )
        .expect("pair metadata should decode");
        CrossTargetPair {
            predecessor_id: metadata["predecessor_id"]
                .as_str()
                .expect("predecessor id should exist")
                .to_owned(),
            successor_id:   metadata["successor_id"]
                .as_str()
                .expect("successor id should exist")
                .to_owned(),
            repository:     Self {
                executable: template.repository.executable,
                repository,
                integration: worktrees.join("integration"),
                worktrees: WorktreeDirectory::InRepository(worktrees),
            },
        }
    }

    /// Copy a recorded integration history with each linked checkout kept private.
    ///
    /// # Panics
    /// Panics when the recorded template cannot be copied.
    #[must_use]
    pub fn from_recorded_template(template: LedgerTemplate) -> Self {
        let repository = template.instantiate();
        let worktrees = repository.path().join(".git/fixture-worktrees");
        Self {
            executable: template.repository.executable,
            repository,
            integration: worktrees.join("integration"),
            worktrees: WorktreeDirectory::InRepository(worktrees),
        }
    }

    /// Prepare the active cover and committed lane before testing either landing mode.
    ///
    /// # Panics
    /// Panics when a claim, commit, or fixture copy fails.
    pub fn build_cover_landing(root: &Path, executable: &'static str) {
        let repository = Self::new(executable);
        let lane = repository.lane("landing-lane", "integration");
        let claimed = repository.claim(
            &lane,
            "file:lane.txt",
            "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b",
            None,
        );
        assert_success(&claimed);
        let _ = repository.board();
        let lane_id = json(&claimed)["payload"]["data"]["reservation_id"].clone();
        let cover_id = repository.cover_claims()[0]["reservation_id"].clone();
        repository.commit_file(&lane, "lane.txt", "landed\n", "lane work");
        let source = fs::canonicalize(repository.root()).expect("main root should resolve");
        let linked_source = fs::canonicalize(repository.worktrees.path())
            .expect("linked checkout source should resolve");
        let destination = root.join(".git/fixture-worktrees");
        let mut main = DirectorySnapshot::capture(repository.root());
        let mut linked = DirectorySnapshot::capture(repository.worktrees.path());
        for snapshot in [&mut main, &mut linked] {
            let _ = snapshot.replace_text(
                &linked_source.to_string_lossy(),
                &destination.to_string_lossy(),
            );
            let _ = snapshot.replace_text(&source.to_string_lossy(), &root.to_string_lossy());
        }
        main.restore(root);
        fs::create_dir_all(&destination).expect("linked checkout destination should exist");
        linked.restore(&destination);
        fs::write(
            root.join(".git/cover-landing.json"),
            serde_json::to_vec(&serde_json::json!({"lane_id": lane_id, "cover_id": cover_id}))
                .expect("landing state should encode"),
        )
        .expect("landing state should write");
    }

    /// Prepare an active cover before a hook merges its dated contribution into main.
    ///
    /// # Panics
    /// Panics when the claim, contribution, or private checkout copy fails.
    pub fn build_covered_branch_hook(root: &Path, executable: &'static str) {
        let repository = Self::new(executable);
        let lane = repository.lane("cover-hook-lane", "integration");
        assert_success(&repository.claim(
            &lane,
            "file:lane.txt",
            "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c",
            None,
        ));
        // Observe the cover before the merge: a later board could settle it.
        drop(repository.board());
        let cover = repository.cover_claims()[0]["reservation_id"].clone();
        fs::write(
            repository.integration.join("integration.txt"),
            "covered contribution\n",
        )
        .expect("covered contribution should write");
        repository
            .git_driver()
            .run(&repository.integration, ["add", "integration.txt"]);
        // Preserve the fixture's ordering without waiting for Git's timestamp resolution.
        assert_success(&repository.git_driver().output_with_environment(
            &repository.integration,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "--quiet",
                "-m",
                "covered contribution",
            ],
            "GIT_COMMITTER_DATE",
            "2030-01-01T00:00:00Z",
        ));
        fs::write(
            repository.integration.join("integration.txt"),
            "ongoing covered work\n",
        )
        .expect("ongoing covered work should write");
        let source = fs::canonicalize(repository.root()).expect("main root should resolve");
        let linked_source = fs::canonicalize(repository.worktrees.path())
            .expect("linked checkout source should resolve");
        let destination = root.join(".git/fixture-worktrees");
        let mut main = DirectorySnapshot::capture(repository.root());
        let mut linked = DirectorySnapshot::capture(repository.worktrees.path());
        for snapshot in [&mut main, &mut linked] {
            drop(snapshot.replace_text(
                &linked_source.to_string_lossy(),
                &destination.to_string_lossy(),
            ));
            drop(snapshot.replace_text(&source.to_string_lossy(), &root.to_string_lossy()));
        }
        main.restore(root);
        fs::create_dir_all(&destination).expect("linked checkout destination should exist");
        linked.restore(&destination);
        fs::write(
            root.join(".git/covered-branch-hook.json"),
            serde_json::to_vec(&serde_json::json!({"cover":cover}))
                .expect("covered hook state should encode"),
        )
        .expect("covered hook state should write");
    }

    /// Locate one of the lanes already present in a recorded pair.
    #[must_use]
    pub fn recorded_lane(&self, branch: &str) -> PathBuf { self.worktrees.path().join(branch) }

    /// Run a fixture landing without running managed hooks.
    ///
    /// # Panics
    /// Panics when git fails.
    pub fn git_without_hooks(&self, checkout: &Path, arguments: &[&str]) {
        self.git_driver().run_without_hooks(checkout, arguments);
    }

    /// Capture the main worktree and every linked worktree, for [`Self::restore`].
    ///
    /// # Panics
    ///
    /// Panics as [`DirectorySnapshot::capture`] does.
    #[must_use]
    pub fn capture(&self) -> IntegrationSnapshot {
        IntegrationSnapshot {
            repository: DirectorySnapshot::capture(self.repository.path()),
            worktrees:  DirectorySnapshot::capture(self.worktrees.path()),
        }
    }

    /// Write back what [`Self::capture`] took, discarding everything written since.
    ///
    /// Git records a linked worktree and its repository by absolute path, so a
    /// capture restores only into the repository it was taken from.
    ///
    /// # Panics
    ///
    /// Panics as [`DirectorySnapshot::restore`] does.
    pub fn restore(&self, integration_snapshot: &IntegrationSnapshot) {
        integration_snapshot
            .repository
            .restore(self.repository.path());
        integration_snapshot
            .worktrees
            .restore(self.worktrees.path());
    }

    /// The main worktree, which has `main` checked out and holds the berth
    /// ledger and configuration.
    #[must_use]
    pub fn root(&self) -> &Path { self.repository.path() }

    /// Add a linked worktree on a new `branch` started from `integration`, with
    /// `target` as the branch's configured berth target.
    ///
    /// # Panics
    ///
    /// Panics when git fails or the worktree path is not UTF-8.
    #[must_use]
    pub fn lane(&self, branch: &str, target: &str) -> PathBuf {
        self.lane_from(branch, "integration", target)
    }

    /// Add a linked worktree on a new `branch` started from `main`, with `main`
    /// as the branch's configured berth target.
    ///
    /// # Panics
    ///
    /// Panics when git fails or the worktree path is not UTF-8.
    #[must_use]
    pub fn main_lane(&self, branch: &str) -> PathBuf { self.lane_from(branch, "main", "main") }

    /// Merge `branch` into `integration` with a merge commit.
    ///
    /// # Panics
    ///
    /// Panics when the merge fails.
    pub fn merge_by_commit(&self, branch: &str) {
        self.git(
            &self.integration,
            &["merge", "--no-ff", "--no-edit", branch],
        );
    }

    /// Fast-forward `integration` to `branch`.
    ///
    /// # Panics
    ///
    /// Panics when `integration` cannot fast-forward to `branch`.
    pub fn merge_fast_forward(&self, branch: &str) {
        self.git(&self.integration, &["merge", "--ff-only", branch]);
    }

    /// Remove the `integration` worktree and delete the `integration` branch.
    ///
    /// # Panics
    ///
    /// Panics when git fails or the worktree path is not UTF-8.
    pub fn remove_integration_branch(&self) {
        self.git(
            self.root(),
            &[
                "worktree",
                "remove",
                "--force",
                self.integration.to_str().expect("UTF-8 path"),
            ],
        );
        self.git(self.root(), &["branch", "-D", "integration"]);
    }

    /// Run git in `checkout` and assert it succeeded.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started or reports failure.
    pub fn git(&self, checkout: &Path, arguments: &[&str]) {
        self.git_driver().run(checkout, arguments);
    }

    /// Run git in `checkout`, assert it succeeded, and return its trimmed
    /// standard output.
    ///
    /// # Panics
    ///
    /// Panics when git cannot be started, reports failure, or writes output
    /// that is not UTF-8.
    #[must_use]
    pub fn git_stdout(&self, checkout: &Path, arguments: &[&str]) -> String {
        self.git_driver().stdout(checkout, arguments)
    }

    /// Write `contents` to `path` in `checkout`, then stage and commit it with
    /// `message`.
    ///
    /// # Panics
    ///
    /// Panics when the file cannot be written or git fails.
    pub fn commit_file(&self, checkout: &Path, path: &str, contents: &str, message: &str) {
        write_file(checkout, path, contents);
        self.git(checkout, &["add", path]);
        self.git(checkout, &["commit", "--quiet", "-m", message]);
    }

    /// Run `cargo-berth` in `checkout` without this process's coordination run
    /// or session, and return its captured result.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth` cannot be started.
    #[must_use]
    pub fn run(&self, checkout: &Path, arguments: &[&str]) -> Output {
        berth_command::berth_command(self.executable)
            .args(arguments)
            .current_dir(checkout)
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_SESSION_ID")
            .output()
            .expect("cargo-berth runs")
    }

    /// The `board --json` envelope, read from the main worktree.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth board` fails or writes output that is not JSON.
    #[must_use]
    pub fn board(&self) -> Value {
        let output = self.run(self.root(), &["board", "--json"]);
        assert_success(&output);
        json(&output)
    }

    /// Claim `path` from `checkout` as coordination run `run_id`, naming
    /// `target` when one is given.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth` cannot be started.
    #[must_use]
    pub fn claim(&self, checkout: &Path, path: &str, run_id: &str, target: Option<&str>) -> Output {
        let mut arguments = vec!["claim", path, "--run", run_id, "--json"];
        if let Some(target) = target {
            arguments.extend(["--target", target]);
        }
        self.run(checkout, &arguments)
    }

    /// Claim `path` from `checkout` deferred behind reservation `blocker`.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth` cannot be started.
    #[must_use]
    pub fn defer_claim(&self, checkout: &Path, path: &str, run_id: &str, blocker: &str) -> Output {
        self.defer_claim_to(checkout, path, run_id, blocker, None)
    }

    /// Claim `path` from `checkout` deferred behind reservation `blocker`,
    /// naming `target` when one is given.
    ///
    /// A `--defer` answer acquires no reservation, so run `run_id` first claims
    /// [`deferring_run_scope`] under `target`, and the answer attaches to that
    /// reservation, which the returned answer output names.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth` cannot be started or the first claim fails.
    #[must_use]
    pub fn defer_claim_to(
        &self,
        checkout: &Path,
        path: &str,
        run_id: &str,
        blocker: &str,
        target: Option<&str>,
    ) -> Output {
        let own_scope = deferring_run_scope(run_id);
        let mut own_claim = vec![
            "claim",
            &own_scope,
            "--run",
            run_id,
            "--why",
            "protect deferred work",
        ];
        if let Some(target) = target {
            own_claim.extend(["--target", target]);
        }
        own_claim.push("--json");
        assert_success(&self.run(checkout, &own_claim));
        self.run(
            checkout,
            &[
                "claim",
                path,
                "--run",
                run_id,
                "--defer",
                blocker,
                "--overlap-why",
                "the order needs a decision",
                "--why",
                "protect deferred work",
                "--json",
            ],
        )
    }

    /// Every event in the berth journal, in recorded order.
    ///
    /// # Panics
    ///
    /// Panics when the journal cannot be read or an entry is not JSON.
    #[must_use]
    pub fn journal(&self) -> Vec<Value> {
        fs::read_to_string(self.root().join(".git/cargo-berth/journal.ndjson"))
            .expect("journal reads")
            .lines()
            .map(|line| serde_json::from_str(line).expect("journal entry decodes"))
            .collect()
    }

    /// The journal's claim events whose source is a cover.
    ///
    /// # Panics
    ///
    /// Panics when the journal cannot be read or an entry is not JSON.
    #[must_use]
    pub fn cover_claims(&self) -> Vec<Value> {
        self.journal()
            .into_iter()
            .filter(|event| event["op"] == "claim" && event["source"]["kind"] == "cover")
            .collect()
    }

    /// Change the main worktree's configured trunk from `main` to `branch`.
    ///
    /// # Panics
    ///
    /// Panics when the configuration cannot be read or written, or does not
    /// name `main` as its trunk.
    pub fn set_trunk(&self, branch: &str) {
        let file = self.root().join(".claude/config/berth.toml");
        let config = fs::read_to_string(&file).expect("berth configuration reads");
        let updated = config.replacen("trunk = \"main\"", &format!("trunk = \"{branch}\""), 1);
        assert_ne!(config, updated, "default trunk line exists");
        fs::write(file, updated).expect("berth configuration writes");
    }

    fn lane_from(&self, branch: &str, base: &str, target: &str) -> PathBuf {
        let checkout = self.worktrees.path().join(branch);
        self.git_driver()
            .add_worktree_without_hooks(self.root(), &checkout, branch, base);
        self.git(
            self.root(),
            &[
                "config",
                &format!("branch.{branch}.cargoBerthTarget"),
                target,
            ],
        );
        fs::canonicalize(checkout).expect("canonical lane worktree")
    }

    const fn git_driver(&self) -> GitDriver {
        GitDriver {
            executable:          self.executable,
            optional_locks:      OptionalLocks::Taken,
            cleared_environment: CLEARED_GIT_ENVIRONMENT,
        }
    }
}

/// Assert `output` reports success, showing both of its streams when it does
/// not.
///
/// # Panics
///
/// Panics when `output` reports failure.
pub fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Parse `output`'s standard output as JSON.
///
/// # Panics
///
/// Panics when standard output is not JSON.
#[must_use]
pub fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

/// The file run `run_id` claims for itself before a `--defer` answer, one per run so two
/// deferring runs never overlap.
///
/// A `--defer` answer acquires no reservation: it attaches to the oldest active reservation
/// of its run, which the ordering graph then holds against each answered holder. A deferring
/// run claims this file first so that reservation exists.
#[must_use]
pub fn deferring_run_scope(run_id: &str) -> String {
    format!("file:{}", deferring_run_path(run_id))
}

/// The repository path [`deferring_run_scope`] names for run `run_id`.
#[must_use]
pub fn deferring_run_path(run_id: &str) -> String { format!("deferring/{run_id}.txt") }

/// The reservation ID a `claim --json` response reports.
///
/// # Panics
///
/// Panics when the response is not JSON or names no reservation ID.
#[must_use]
pub fn claim_id(output: &Output) -> String {
    json(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim reservation ID")
        .to_owned()
}

/// The reservation entries across every `board --json` reservation section.
pub fn board_reservation_entries(board: &Value) -> impl Iterator<Item = &Value> {
    let data = &board["payload"]["data"];
    [
        "ready_now",
        "waiting",
        "unconstrained_reservations",
        "resolved",
    ]
    .into_iter()
    .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
    .map(|entry| entry.get("reservation").unwrap_or(entry))
}

/// The sorted reservation IDs across every `board --json` reservation section.
///
/// # Panics
///
/// Panics when a reservation entry names no string reservation ID.
#[must_use]
pub fn board_reservation_ids(board: &Value) -> Vec<String> {
    let mut ids = board_reservation_entries(board)
        .map(|entry| {
            entry["reservation_id"]
                .as_str()
                .expect("reservation id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// The row for reservation `id` in whichever `board --json` section lists it.
///
/// # Panics
///
/// Panics when no section lists the reservation.
#[must_use]
pub fn reservation_row<'board>(board: &'board Value, id: &str) -> &'board Value {
    board_reservation_entries(board)
        .find(|entry| entry["reservation_id"] == id)
        .expect("reservation appears on board")
}

/// The worktree identity and coordination run marker berth recorded in
/// `checkout`'s git administrative directory.
///
/// # Panics
///
/// Panics when either file cannot be read, or when `checkout` is a linked
/// worktree whose `.git` pointer cannot be read.
#[must_use]
pub fn worktree_identity_and_marker_run(checkout: &Path) -> (String, String) {
    let git_path = checkout.join(".git");
    let administrative_directory = if git_path.is_dir() {
        git_path
    } else {
        let pointer = fs::read_to_string(&git_path).expect("linked worktree git pointer reads");
        let directory = pointer
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree git pointer names its administrative directory");
        checkout.join(directory)
    };
    let worktree = fs::read_to_string(administrative_directory.join("cargo-berth-worktree-id"))
        .expect("worktree identity reads")
        .trim()
        .to_owned();
    let run = fs::read_to_string(administrative_directory.join("cargo-berth-run-id"))
        .expect("worktree coordination marker reads")
        .trim()
        .to_owned();
    (worktree, run)
}

/// Write `contents` to `path` in `checkout`, creating missing parent
/// directories.
///
/// # Panics
///
/// Panics when a directory or the file cannot be written.
pub fn write_file(checkout: &Path, path: &str, contents: &str) {
    let file = checkout.join(path);
    fs::create_dir_all(file.parent().expect("file parent")).expect("file parent exists");
    fs::write(file, contents).expect("fixture file writes");
}

/// An immutable initialized checkout owned by the build cache, never by `/tmp`.
struct InitializedMainSeed {
    root: PathBuf,
}

impl InitializedMainSeed {
    fn prepare(executable: &'static str) -> Self {
        let builds = fixture_build_directory(executable, "initialized-main", "main");
        let root = builds.join(REPOSITORY_DIRECTORY_NAME);
        prepare_fixture_build(&builds, &[&root], || {
            IntegrationRepository::build_main_seed(&root, executable);
        });
        Self { root }
    }
}

/// Locate immutable fixtures under target, keyed by both executables that built them.
fn fixture_build_directory(executable: &str, category: &str, name: &str) -> PathBuf {
    let mut identity = DefaultHasher::new();
    name.hash(&mut identity);
    for binary in [
        env::current_exe().expect("test binary should resolve"),
        PathBuf::from(executable),
    ] {
        let metadata = fs::metadata(binary).expect("fixture executable should exist");
        metadata.len().hash(&mut identity);
        metadata
            .modified()
            .expect("executable modification should read")
            .hash(&mut identity);
    }
    let builds = repository_template::fixture_cache_root(Path::new(executable))
        .join(category)
        .join(format!("{:016x}", identity.finish()));
    fs::create_dir_all(&builds).expect("fixture build directory should exist");
    fs::canonicalize(builds).expect("fixture build directory should resolve")
}

/// Publish one complete fixture, rebuilding any directories an interrupted build left.
fn prepare_fixture_build(builds: &Path, directories: &[&Path], build: impl FnOnce()) {
    let lock =
        File::create(builds.join(BUILD_LOCK_FILE_NAME)).expect("fixture build lock should open");
    lock.lock().expect("one fixture builder should run");
    let ready = builds.join(COMPLETE_FILE_NAME);
    if !ready.is_file() {
        for directory in directories {
            if directory.exists() {
                fs::remove_dir_all(directory).expect("incomplete fixture should remove");
            }
            fs::create_dir(directory).expect("fixture directory should exist");
        }
        build();
        File::create(ready).expect("fixture completion should record");
    }
}

/// Copy a plain Git seed without repeating its setup commands in each test process.
///
/// The build owner publishes the immutable seed under the running binary's fixture
/// cache. Each copy gets private working files, index and refs. Seeds with berth
/// identities or journal records use `RepositoryTemplate` instead.
///
/// # Panics
/// Panics when building, locking, reading or copying the seed fails.
pub fn copy_git_repository_seed<E: std::fmt::Debug>(
    name: &str,
    build: impl FnOnce(&Path) -> Result<(), E>,
) -> TempDir {
    let binary = env::current_exe().expect("test executable should resolve");
    let executable = binary.to_str().expect("test executable should be UTF-8");
    let builds = fixture_build_directory(executable, "plain-git", name);
    let source = builds.join(REPOSITORY_DIRECTORY_NAME);
    prepare_fixture_build(&builds, &[&source], || {
        build(&source).expect("plain Git seed should build");
    });
    let repository = tempdir().expect("seed destination should exist");
    let destination = fs::canonicalize(repository.path()).expect("seed destination should resolve");
    let mut snapshot = DirectorySnapshot::capture(&source);
    drop(snapshot.replace_text(&source.to_string_lossy(), &destination.to_string_lossy()));
    snapshot.restore(&destination);
    repository
}

/// A main checkout and integration target whose hooked setup runs once per fixture.
/// Every test copies both trees with one owner-renewal map and private mutable files.
struct PreparedIntegrationPair {
    root:      PathBuf,
    worktrees: PathBuf,
}

impl PreparedIntegrationPair {
    fn instantiate(
        executable: &'static str,
        prefix: &str,
        name: &str,
        build_main: impl FnOnce(&Path),
    ) -> IntegrationRepository {
        let prepared = Self::prepare(executable, name, build_main);
        let repository = Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("repository parent");
        let worktrees = tempdir().expect("worktree parent");
        let repository_root =
            fs::canonicalize(repository.path()).expect("repository should resolve");
        let worktree_root = fs::canonicalize(worktrees.path()).expect("worktrees should resolve");
        let mut main = DirectorySnapshot::capture(&prepared.root);
        let mut linked = DirectorySnapshot::capture(&prepared.worktrees);
        for snapshot in [&mut main, &mut linked] {
            drop(snapshot.replace_text(
                &prepared.root.to_string_lossy(),
                &repository_root.to_string_lossy(),
            ));
            drop(snapshot.replace_text(
                &prepared.worktrees.to_string_lossy(),
                &worktree_root.to_string_lossy(),
            ));
        }
        IdentityRenewal::read(&prepared.root).apply_pair(&mut main, &mut linked);
        main.restore(&repository_root);
        linked.restore(&worktree_root);
        for name in ["reservations.json", "replay-checkpoint.json"] {
            let derived = repository_root.join(".git/cargo-berth").join(name);
            if derived.is_file() {
                fs::remove_file(derived)
                    .expect("copied projection should rebuild from renewed owners");
            }
        }
        IntegrationRepository {
            executable,
            repository,
            integration: worktree_root.join("integration"),
            worktrees: WorktreeDirectory::Temporary(worktrees),
        }
    }

    fn prepare(executable: &'static str, name: &str, build_main: impl FnOnce(&Path)) -> Self {
        let builds = fixture_build_directory(executable, "integration-pair", name);
        let root = builds.join(REPOSITORY_DIRECTORY_NAME);
        let worktrees = builds.join(WORKTREES_DIRECTORY_NAME);
        prepare_fixture_build(&builds, &[&root, &worktrees], || {
            build_main(&root);
            let integration = worktrees.join("integration");
            let git = GitDriver {
                executable,
                optional_locks: OptionalLocks::Taken,
                cleared_environment: CLEARED_GIT_ENVIRONMENT,
            };
            git.add_worktree_without_hooks(&root, &integration, "integration", "main");
            write_file(&integration, "integration.txt", "integration\n");
            git.run(&integration, ["add", "integration.txt"]);
            git.run(
                &integration,
                ["commit", "--quiet", "-m", "integration start"],
            );
        });
        Self { root, worktrees }
    }
}
