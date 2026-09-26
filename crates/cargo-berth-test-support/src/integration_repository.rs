//! A repository whose lanes target an `integration` branch instead of `main`.
//!
//! `IntegrationRepository` initializes berth on `main`, checks `integration`
//! out in a linked worktree, and adds lanes that target either branch. Its
//! methods run git and `cargo-berth` with the executable the test crate passed
//! to `IntegrationRepository::new`; the free functions here only read what those
//! runs produced.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;
use tempfile::Builder;
use tempfile::TempDir;
use tempfile::tempdir;

use crate::berth_command;
use crate::git_driver::GitDriver;
use crate::git_driver::OptionalLocks;

/// Variables cleared before git runs, so a hook cannot inherit this process's
/// coordination run, session, or repository location.
const CLEARED_GIT_ENVIRONMENT: &[&str] = &[
    "CARGO_BERTH_RUN",
    "CARGO_BERTH_SESSION_ID",
    "GIT_DIR",
    "GIT_COMMON_DIR",
];

/// A temporary repository with berth initialized on `main` and an `integration`
/// branch checked out in a linked worktree.
pub struct IntegrationRepository {
    executable:      &'static str,
    repository:      TempDir,
    worktrees:       TempDir,
    /// The canonical path of the linked worktree that has `integration` checked
    /// out.
    pub integration: PathBuf,
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
        let repository = Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("repository parent");
        let worktrees = tempdir().expect("worktree parent");
        let integration = worktrees.path().join("integration");
        let mut integration_repository = Self {
            executable,
            repository,
            worktrees,
            integration,
        };
        let root = integration_repository.root();
        integration_repository.git(root, &["init", "--quiet", "--initial-branch=main"]);
        integration_repository.git(root, &["config", "user.name", "Berth Test"]);
        integration_repository.git(root, &["config", "user.email", "berth@example.invalid"]);
        integration_repository.git(root, &["config", "maintenance.auto", "false"]);
        integration_repository.git(root, &["config", "gc.auto", "0"]);
        integration_repository.commit_file(root, "README.md", "initial\n", "initial");
        let initialized = integration_repository.run(root, &["init", "--json"]);
        assert_success(&initialized);
        integration_repository.git(root, &["add", ".claude/config/berth.toml"]);
        integration_repository.git(
            root,
            &["commit", "--quiet", "-m", "track berth configuration"],
        );
        integration_repository.git(
            root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "integration",
                integration_repository
                    .integration
                    .to_str()
                    .expect("UTF-8 path"),
                "main",
            ],
        );
        integration_repository.integration = fs::canonicalize(&integration_repository.integration)
            .expect("canonical integration worktree");
        integration_repository.commit_file(
            &integration_repository.integration,
            "integration.txt",
            "integration\n",
            "integration start",
        );
        integration_repository
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

    /// Claim `path` from `checkout` deferred behind reservation `blocker`,
    /// authorizing the proposal the first claim returns.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth` cannot be started or the first claim does not
    /// ask for authorization with a proposal token.
    #[must_use]
    pub fn defer_claim(&self, checkout: &Path, path: &str, run_id: &str, blocker: &str) -> Output {
        self.defer_claim_to(checkout, path, run_id, blocker, None)
    }

    /// Claim `path` from `checkout` deferred behind reservation `blocker`,
    /// naming `target` when one is given, and authorize the proposal the first
    /// claim returns.
    ///
    /// # Panics
    ///
    /// Panics when `cargo-berth` cannot be started or the first claim does not
    /// ask for authorization with a proposal token.
    #[must_use]
    pub fn defer_claim_to(
        &self,
        checkout: &Path,
        path: &str,
        run_id: &str,
        blocker: &str,
        target: Option<&str>,
    ) -> Output {
        let mut arguments = vec![
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
        ];
        if let Some(target) = target {
            arguments.extend(["--target", target]);
        }
        arguments.push("--json");
        let proposal = self.run(checkout, &arguments);
        let proposal_json = json(&proposal);
        assert_eq!(
            proposal_json["status"], "needs_user_authorization",
            "{proposal_json}"
        );
        let token = proposal_json["payload"]["data"]["proposal_token"]
            .as_str()
            .expect("deferred claim proposal token")
            .to_owned();
        arguments.splice(
            arguments.len() - 1..arguments.len() - 1,
            ["--proposal", &token],
        );
        self.run(checkout, &arguments)
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
        self.git(
            self.root(),
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                checkout.to_str().expect("UTF-8 path"),
                base,
            ],
        );
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

/// The row for reservation `id` in whichever `board --json` section lists it.
///
/// # Panics
///
/// Panics when no section lists the reservation.
#[must_use]
pub fn reservation_row<'board>(board: &'board Value, id: &str) -> &'board Value {
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
