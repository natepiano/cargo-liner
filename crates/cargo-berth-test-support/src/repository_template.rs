//! A repository fixture built once per test binary and copied into each test.
//!
//! nextest runs each test in a process of its own, so a fixture held in memory
//! is built again by every test: git init, configuration, a commit and
//! `cargo-berth init`, about 35 processes. A `RepositoryTemplate` keeps the
//! built repository on disk under the test crate's `CARGO_TARGET_TMPDIR`,
//! keyed by the test binary and the `cargo-berth` it runs, so the first test
//! of a build pays for it and every test copies it into a temporary directory
//! of its own. The copy gets what building in place would have given it: each
//! absolute path `cargo-berth init` wrote, as in the `reference-transaction`
//! hook, names the copy; the repository and worktree identities, when the
//! fixture ran `cargo-berth init`, are new; and git's index matches the copied
//! files.

use std::env;
use std::fs;
use std::fs::File;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;
use uuid::Uuid;

use crate::DirectorySnapshot;
use crate::fixture_tempdir;

/// Beside a template's builds, the file whose lock one builder at a time holds.
const BUILD_LOCK_FILE_NAME: &str = "build.lock";
/// The extension of the file written beside a build once the build is complete.
const COMPLETE_EXTENSION: &str = "complete";
/// The journal, relative to the repository root.
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
/// The ledger directory, relative to the repository root.
const LEDGER_DIRECTORY: &str = ".git/cargo-berth";
/// Rebuild recorded fixtures after 12 hours, well before reservations go stale at 24 hours.
const RECORDED_TEMPLATE_LIFETIME: Duration = Duration::from_hours(12);
/// The repository identity `cargo-berth init` writes, relative to the repository root.
const REPOSITORY_IDENTITY_PATH: &str = ".git/cargo-berth/repo-instance-id";
/// Under `CARGO_TARGET_TMPDIR`, the directory that holds every test binary's templates.
const TEMPLATES_DIRECTORY: &str = "cargo-berth-templates";
/// The main worktree's identity, relative to the repository root.
const WORKTREE_IDENTITY_PATH: &str = ".git/cargo-berth-worktree-id";

/// The build-owned root shared by repositories and their executable fixtures.
pub(crate) fn fixture_cache_root(executable: &Path) -> PathBuf {
    let directory = executable
        .parent()
        .expect("fixture executable should have a directory");
    let profile = if directory.file_name().is_some_and(|name| name == "deps") {
        directory
            .parent()
            .expect("test executable should have a profile directory")
    } else {
        directory
    };
    let target = profile
        .parent()
        .expect("fixture profile should have a target directory");
    let root = target.join("tmp").join(TEMPLATES_DIRECTORY);
    fs::create_dir_all(&root).expect("fixture cache root should exist");
    fs::canonicalize(root).expect("fixture cache root should resolve")
}

/// What journal state a repository template is allowed to keep.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LedgerFixture {
    /// No journal events refer to the template's owners or paths.
    Empty,
    /// Journal events and pending markers are relocated with the checkout.
    Recorded,
}

/// When a template's setup runs relative to nextest's test clocks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TemplateBuildTiming {
    /// The first test builds the template under its clock.
    InTest,
    /// A nextest setup script builds the template before tests start.
    BeforeRun,
}

/// A repository template with an explicit journal and setup lifetime.
///
/// Existing empty templates retain their literal API; recorded fixtures opt in
/// here so callers cannot silently inherit permission to copy a live ledger.
#[derive(Clone, Copy)]
pub struct LedgerTemplate {
    /// The repository history and configuration to build.
    pub repository:   RepositoryTemplate,
    /// The journal state the builder may leave behind.
    pub ledger:       LedgerFixture,
    /// Whether nextest prepares this fixture before starting tests.
    pub build_timing: TemplateBuildTiming,
}

/// One renewal map shared by identity files, journal actors and pending markers.
pub(crate) struct IdentityRenewal(Vec<(String, String)>);

impl IdentityRenewal {
    pub(crate) fn read(root: &Path) -> Self {
        let mut paths = vec![
            root.join(REPOSITORY_IDENTITY_PATH),
            root.join(WORKTREE_IDENTITY_PATH),
        ];
        if let Ok(entries) = fs::read_dir(root.join(".git/worktrees")) {
            paths.extend(entries.map(|entry| {
                entry
                    .expect("worktree entry should read")
                    .path()
                    .join("cargo-berth-worktree-id")
            }));
        }
        Self(
            paths
                .into_iter()
                .filter(|path| path.is_file())
                .map(|path| {
                    let identity = fs::read_to_string(path).expect("template identity should read");
                    (identity.trim().to_owned(), Uuid::now_v7().to_string())
                })
                .collect(),
        )
    }

    /// Renew one owner consistently across a repository and its external worktrees.
    pub(crate) fn apply_pair(
        self,
        repository: &mut DirectorySnapshot,
        worktrees: &mut DirectorySnapshot,
    ) {
        for (old, new) in self.0 {
            let main = repository.replace_text(&old, &new);
            let linked = worktrees.replace_text(&old, &new);
            assert!(
                !main.is_empty() || !linked.is_empty(),
                "template identity should be replaced"
            );
        }
    }

    pub(crate) fn apply(self, snapshot: &mut DirectorySnapshot) {
        for (old, new) in self.0 {
            assert!(
                !snapshot.replace_text(&old, &new).is_empty(),
                "template identity should be replaced"
            );
        }
    }
}

/// A repository built once per build of a test binary, then copied into each
/// test that starts from it.
#[derive(Clone, Copy)]
pub struct RepositoryTemplate {
    /// The test crate's `env!("CARGO_TARGET_TMPDIR")`, under which builds are kept.
    pub target_directory: &'static str,
    /// The test crate's `env!("CARGO_BIN_EXE_cargo-berth")`. A rebuilt binary
    /// starts a new build of every template.
    pub executable:       &'static str,
    /// Names this template among the test binary's templates.
    pub name:             &'static str,
    /// Builds the repository at the root it is given: its commits and
    /// configuration, and `cargo-berth init` when the fixture includes it. The
    /// build leaves the journal empty, so the only records that tie a ledger to
    /// this repository are its identities. It must return with no process it
    /// started still running in the root: every test copies the root without a
    /// lock once `.complete` exists. Start git through `git_command` or a
    /// `GitDriver`, and `cargo-berth` through `berth_command`.
    pub build:            fn(&Path),
}

impl RepositoryTemplate {
    /// Copy the built repository into a new temporary directory, building it
    /// first when no test of this build of the test binary has.
    ///
    /// # Panics
    ///
    /// Panics when the template cannot be built, locked, read or copied, when
    /// its journal holds a record, or when its ledger records the template's
    /// own root, which a copy cannot carry to a new root.
    #[must_use]
    pub fn instantiate(self) -> TempDir {
        LedgerTemplate {
            repository:   self,
            ledger:       LedgerFixture::Empty,
            build_timing: TemplateBuildTiming::InTest,
        }
        .instantiate()
    }

    /// Restore an empty-ledger fixture into an existing empty directory.
    ///
    /// Related fixture builders share their base without replaying its Git and
    /// initialization commands. Paths and identities are renewed for the destination.
    ///
    /// # Panics
    /// Panics under the same conditions as [`Self::instantiate`].
    pub fn restore_into(self, root: &Path) {
        LedgerTemplate {
            repository:   self,
            ledger:       LedgerFixture::Empty,
            build_timing: TemplateBuildTiming::InTest,
        }
        .restore_into(root);
    }

    /// Directory shared by every before-run template built by this executable.
    ///
    /// # Panics
    /// Panics when the running builder or berth executable cannot be inspected.
    #[must_use]
    pub fn before_run_directory(self) -> PathBuf {
        let binary = env::current_exe().expect("builder path should resolve");
        Path::new(self.target_directory)
            .join(TEMPLATES_DIRECTORY)
            .join("before-run")
            .join(self.key(&binary))
    }

    /// Names a build by the test binary and the `cargo-berth` it ran.
    fn key(self, test_binary: &Path) -> String {
        let mut hasher = DefaultHasher::new();
        for binary in [test_binary, Path::new(self.executable)] {
            let metadata = fs::metadata(binary).expect("binary metadata should read");
            metadata.len().hash(&mut hasher);
            metadata
                .modified()
                .expect("binary modification time should read")
                .hash(&mut hasher);
        }
        format!("{:016x}", hasher.finish())
    }
}

impl LedgerTemplate {
    /// Copy this fixture, renewing all owners and discarding derived replay files.
    ///
    /// # Panics
    /// Panics when the fixture cannot be built, copied or relocated.
    #[must_use]
    pub fn instantiate(self) -> TempDir {
        let repository = fixture_tempdir("repository-");
        self.restore_into(repository.path());
        repository
    }

    fn restore_into(self, root: &Path) {
        let template_root = self.built_root();
        let repository_root =
            fs::canonicalize(root).expect("destination repository should canonicalize");
        let mut snapshot = DirectorySnapshot::capture(&template_root);
        let relocated = snapshot.replace_text(
            &template_root.to_string_lossy(),
            &repository_root.to_string_lossy(),
        );
        assert!(
            self.ledger == LedgerFixture::Recorded
                || relocated
                    .iter()
                    .all(|path| !path.starts_with(LEDGER_DIRECTORY)),
            "the template ledger records its root in {relocated:?}; build this fixture in place"
        );
        IdentityRenewal::read(&template_root).apply(&mut snapshot);
        snapshot.restore(&repository_root);
        if self.ledger == LedgerFixture::Recorded {
            for name in ["reservations.json", "replay-checkpoint.json"] {
                let path = repository_root.join(LEDGER_DIRECTORY).join(name);
                if path.exists() {
                    fs::remove_file(path).expect("derived replay file should remove");
                }
            }
        }
        // The copied index keeps its staged contents. Git refreshes stale stat
        // entries when a later operation reads them; restoring needs no process.
    }

    /// The root of this template's build for the running test binary, built
    /// under the build lock when no complete build exists.
    fn built_root(self) -> PathBuf {
        if self.build_timing == TemplateBuildTiming::BeforeRun
            && let Some(directory) = env::var_os("CARGO_BERTH_PREBUILT_TEMPLATES")
        {
            let root = Path::new(&directory).join(self.repository.name);
            if self.complete(&root) {
                return root;
            }
        }
        let test_binary = env::current_exe().expect("test binary path should resolve");
        let builds = Path::new(self.repository.target_directory)
            .join(TEMPLATES_DIRECTORY)
            .join(
                test_binary
                    .file_name()
                    .expect("test binary should have a file name"),
            )
            .join(self.repository.name);
        fs::create_dir_all(&builds).expect("template directory should exist");
        let builds = fs::canonicalize(&builds).expect("template directory should canonicalize");
        let root = builds.join(self.repository.key(&test_binary));
        if self.complete(&root) {
            return root;
        }
        let lock = File::create(builds.join(BUILD_LOCK_FILE_NAME))
            .expect("template build lock should open");
        lock.lock().expect("template build lock should be taken");
        if !self.complete(&root) {
            remove_stale_builds(&builds);
            self.build_at(&root);
        }
        root
    }

    /// Copy a recorded history, placing its private linked checkouts beside the repository.
    ///
    /// # Panics
    /// Panics when a checkout cannot be relocated or restored.
    #[must_use]
    pub fn instantiate_with_external_worktrees(self, worktrees: &Path) -> TempDir {
        let repository = self.instantiate();
        let embedded = fs::canonicalize(repository.path().join(".git/fixture-worktrees"))
            .expect("embedded worktree parent should resolve");
        let destination = fs::canonicalize(worktrees).expect("worktree parent should resolve");
        let mut main = DirectorySnapshot::capture(repository.path());
        let mut linked = DirectorySnapshot::capture(&embedded);
        for snapshot in [&mut main, &mut linked] {
            let _ =
                snapshot.replace_text(&embedded.to_string_lossy(), &destination.to_string_lossy());
        }
        main.restore(repository.path());
        linked.restore(&destination);
        fs::remove_dir_all(embedded).expect("embedded checkout copies should remove");
        repository
    }

    /// Build under nextest's setup clock, once per executable pair and name.
    ///
    /// # Panics
    /// Panics when setup cannot lock or build its repository.
    pub fn prepare_before_run(self, directory: &Path) {
        fs::create_dir_all(directory).expect("before-run directory should exist");
        let lock = File::create(directory.join(format!("{}.lock", self.repository.name)))
            .expect("setup lock should open");
        lock.lock().expect("setup lock should be taken");
        let root = directory.join(self.repository.name);
        if !self.complete(&root) {
            if root.exists() {
                fs::remove_dir_all(&root).expect("expired fixture should remove");
            }
            self.build_at(&root);
        }
    }

    fn complete(self, root: &Path) -> bool {
        fs::metadata(root.with_extension(COMPLETE_EXTENSION)).is_ok_and(|metadata| {
            self.ledger == LedgerFixture::Empty
                || metadata
                    .modified()
                    .expect("completion time should read")
                    .elapsed()
                    .is_ok_and(|age| age < RECORDED_TEMPLATE_LIFETIME)
        })
    }

    fn build_at(self, root: &Path) {
        fs::create_dir(root).expect("template root should exist");
        (self.repository.build)(root);
        if self.ledger == LedgerFixture::Empty {
            assert_eq!(
                fs::metadata(root.join(JOURNAL_PATH)).map_or(0, |journal| journal.len()),
                0,
                "an empty template must leave no journal records"
            );
        }
        File::create(root.with_extension(COMPLETE_EXTENSION)).expect("completion should record");
    }
}

/// Remove every build and completion record beside the build lock: an earlier
/// binary's builds, and a build interrupted before it completed.
fn remove_stale_builds(builds: &Path) {
    fs::read_dir(builds)
        .expect("template directory should read")
        .map(|entry| entry.expect("template directory entry should read"))
        .filter(|entry| entry.file_name() != BUILD_LOCK_FILE_NAME)
        .for_each(|entry| {
            if entry
                .file_type()
                .expect("template entry type should read")
                .is_dir()
            {
                fs::remove_dir_all(entry.path())
            } else {
                fs::remove_file(entry.path())
            }
            .expect("stale template build should remove");
        });
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use tempfile::tempdir;

    use super::*;
    use crate::git_driver;

    const RESTORE_PROBE_GIT: &str = "CARGO_BERTH_RESTORE_PROBE_GIT";

    #[test]
    fn template_restore_needs_no_git_and_preserves_clean_and_dirty_indexes() {
        let templates = [
            index_template("clean-index", build_clean_index),
            index_template("dirty-index", build_dirty_index),
        ];
        if let Some(git) = env::var_os(RESTORE_PROBE_GIT) {
            for (template, expected) in templates
                .into_iter()
                .zip(["", "M  staged.txt\n M working.txt\n"])
            {
                let repository = fixture_tempdir("restore-");
                template.repository.restore_into(repository.path());
                let output = Command::new(&git)
                    .current_dir(repository.path())
                    .args(["status", "--porcelain", "--untracked-files=all"])
                    .output()
                    .expect("Git should inspect the restored index");
                assert!(output.status.success(), "{output:?}");
                assert_eq!(
                    String::from_utf8(output.stdout).expect("status should be UTF-8"),
                    expected
                );
            }
            return;
        }
        for template in templates {
            template.built_root();
        }
        let git = env::split_paths(&env::var_os("PATH").expect("PATH should exist"))
            .map(|directory| directory.join("git"))
            .find(|path| path.is_file())
            .expect("Git should be on PATH")
            .canonicalize()
            .expect("Git should canonicalize");
        let empty_path = tempdir().expect("empty PATH directory should exist");
        let output = Command::new(env::current_exe().expect("test binary should resolve"))
            .args(["--exact", "repository_template::tests::template_restore_needs_no_git_and_preserves_clean_and_dirty_indexes", "--nocapture"])
            .env("PATH", empty_path.path())
            .env(RESTORE_PROBE_GIT, git)
            .output()
            .expect("restore probe should start");
        assert!(output.status.success(), "{output:?}");
    }

    fn index_template(name: &'static str, build: fn(&Path)) -> LedgerTemplate {
        LedgerTemplate {
            repository:   RepositoryTemplate {
                target_directory: concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/tmp"),
                executable: "/bin/sh",
                name,
                build,
            },
            ledger:       LedgerFixture::Empty,
            build_timing: TemplateBuildTiming::InTest,
        }
    }

    fn build_clean_index(root: &Path) {
        run_index_git(root, &["init", "--initial-branch=main"]);
        for name in ["staged.txt", "working.txt", "unchanged.txt"] {
            fs::write(root.join(name), "original\n").expect("tracked file should write");
        }
        run_index_git(root, &["add", "."]);
        run_index_git(
            root,
            &[
                "-c",
                "user.name=Template Test",
                "-c",
                "user.email=template@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-m",
                "initial",
            ],
        );
    }

    fn build_dirty_index(root: &Path) {
        build_clean_index(root);
        fs::write(root.join("staged.txt"), "staged change\n").expect("staged file should write");
        run_index_git(root, &["add", "staged.txt"]);
        fs::write(root.join("working.txt"), "working change\n").expect("working file should write");
    }

    fn run_index_git(root: &Path, arguments: &[&str]) {
        let output = git_driver::fixture_git_command()
            .current_dir(root)
            .args(arguments)
            .output()
            .expect("fixture Git should start");
        assert!(output.status.success(), "{arguments:?}: {output:?}");
    }

    #[test]
    fn before_run_template_builds_itself_when_setup_did_not_supply_it() {
        let template = LedgerTemplate {
            repository:   RepositoryTemplate {
                target_directory: concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/tmp"),
                executable:       "/bin/sh",
                name:             "missing-before-run-build",
                build:            |root| {
                    fs::write(root.join("history"), "the required setup")
                        .expect("setup should write");
                },
            },
            ledger:       LedgerFixture::Empty,
            build_timing: TemplateBuildTiming::BeforeRun,
        };
        let test_binary = env::current_exe().expect("test binary should resolve");
        let builds = Path::new(template.repository.target_directory)
            .join(TEMPLATES_DIRECTORY)
            .join(
                test_binary
                    .file_name()
                    .expect("test binary should have a name"),
            )
            .join(template.repository.name);
        if builds.exists() {
            fs::remove_dir_all(&builds).expect("previous fallback build should remove");
        }
        let root = template.built_root();
        assert!(
            root.starts_with(fs::canonicalize(builds).expect("fallback directory should exist"))
        );
        assert_eq!(
            fs::read_to_string(root.join("history")).expect("setup should exist"),
            "the required setup"
        );
    }
}
