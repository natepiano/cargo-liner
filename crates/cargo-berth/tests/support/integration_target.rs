#![allow(
    dead_code,
    reason = "each integration-test binary uses a different part of this fixture"
)]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use serde_json::Value;
use tempfile::TempDir;
use tempfile::tempdir;

const BERTH: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[
        "CARGO_BERTH_RUN",
        "CARGO_BERTH_SESSION_ID",
        "GIT_DIR",
        "GIT_COMMON_DIR",
    ],
};

pub(crate) struct IntegrationRepository {
    repository:      TempDir,
    worktrees:       TempDir,
    pub integration: PathBuf,
}

impl IntegrationRepository {
    pub(crate) fn new() -> Self {
        let repository = tempdir().expect("repository parent");
        let worktrees = tempdir().expect("worktree parent");
        let root = repository.path();
        git(root, &["init", "--quiet", "--initial-branch=main"]);
        git(root, &["config", "user.name", "Berth Test"]);
        git(root, &["config", "user.email", "berth@example.invalid"]);
        git(root, &["config", "maintenance.auto", "false"]);
        git(root, &["config", "gc.auto", "0"]);
        commit_file(root, "README.md", "initial\n", "initial");
        let initialized = run(root, &["init", "--json"]);
        assert_success(&initialized);
        git(root, &["add", ".claude/config/berth.toml"]);
        git(
            root,
            &["commit", "--quiet", "-m", "track berth configuration"],
        );

        let integration = worktrees.path().join("integration");
        git(
            root,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "integration",
                integration.to_str().expect("UTF-8 path"),
                "main",
            ],
        );
        let integration = fs::canonicalize(integration).expect("canonical integration worktree");
        commit_file(
            &integration,
            "integration.txt",
            "integration\n",
            "integration start",
        );
        Self {
            repository,
            worktrees,
            integration,
        }
    }

    pub(crate) fn root(&self) -> &Path { self.repository.path() }

    pub(crate) fn lane(&self, branch: &str, target: &str) -> PathBuf {
        let checkout = self.worktrees.path().join(branch);
        git(
            self.root(),
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                branch,
                checkout.to_str().expect("UTF-8 path"),
                "integration",
            ],
        );
        git(
            self.root(),
            &[
                "config",
                &format!("branch.{branch}.cargoBerthTarget"),
                target,
            ],
        );
        fs::canonicalize(checkout).expect("canonical lane worktree")
    }

    pub(crate) fn merge_by_commit(&self, branch: &str) {
        git(
            &self.integration,
            &["merge", "--no-ff", "--no-edit", branch],
        );
    }

    pub(crate) fn merge_fast_forward(&self, branch: &str) {
        git(&self.integration, &["merge", "--ff-only", branch]);
    }

    pub(crate) fn remove_integration_branch(&self) {
        git(
            self.root(),
            &[
                "worktree",
                "remove",
                "--force",
                self.integration.to_str().expect("UTF-8 path"),
            ],
        );
        git(self.root(), &["branch", "-D", "integration"]);
    }
}

pub(crate) fn git(root: &Path, args: &[&str]) { GIT.run(root, args); }

pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> String { GIT.stdout(root, args) }

pub(crate) fn commit_file(root: &Path, path: &str, contents: &str, message: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("file parent")).expect("file parent exists");
    fs::write(file, contents).expect("fixture file writes");
    git(root, &["add", path]);
    git(root, &["commit", "--quiet", "-m", message]);
}

pub(crate) fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(BERTH)
        .args(args)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("cargo-berth runs")
}

pub(crate) fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("JSON output")
}

pub(crate) fn journal(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join(".git/cargo-berth/journal.ndjson"))
        .expect("journal reads")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal entry decodes"))
        .collect()
}

pub(crate) fn claim(root: &Path, path: &str, run_id: &str, target: Option<&str>) -> Output {
    let mut args = vec!["claim", path, "--run", run_id, "--json"];
    if let Some(target) = target {
        args.extend(["--target", target]);
    }
    run(root, &args)
}

pub(crate) fn set_trunk(root: &Path, branch: &str) {
    let file = root.join(".claude/config/berth.toml");
    let config = fs::read_to_string(&file).expect("berth configuration reads");
    let updated = config.replacen("trunk = \"main\"", &format!("trunk = \"{branch}\""), 1);
    assert_ne!(config, updated, "default trunk line exists");
    fs::write(file, updated).expect("berth configuration writes");
}
