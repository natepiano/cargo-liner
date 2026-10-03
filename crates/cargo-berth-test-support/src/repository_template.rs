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

use tempfile::TempDir;
use tempfile::tempdir;
use uuid::Uuid;

use crate::DirectorySnapshot;
use crate::git_driver::git_command;

/// Beside a template's builds, the file whose lock one builder at a time holds.
const BUILD_LOCK_FILE_NAME: &str = "build.lock";
/// The extension of the file written beside a build once the build is complete.
const COMPLETE_EXTENSION: &str = "complete";
/// The journal, relative to the repository root.
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
/// The ledger directory, relative to the repository root.
const LEDGER_DIRECTORY: &str = ".git/cargo-berth";
/// The repository identity `cargo-berth init` writes, relative to the repository root.
const REPOSITORY_IDENTITY_PATH: &str = ".git/cargo-berth/repo-instance-id";
/// Under `CARGO_TARGET_TMPDIR`, the directory that holds every test binary's templates.
const TEMPLATES_DIRECTORY: &str = "cargo-berth-templates";
/// The main worktree's identity, relative to the repository root.
const WORKTREE_IDENTITY_PATH: &str = ".git/cargo-berth-worktree-id";

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
    /// this repository are its identities.
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
        let template_root = self.built_root();
        let repository = tempdir().expect("temporary repository should exist");
        let repository_root =
            fs::canonicalize(repository.path()).expect("temporary repository should canonicalize");
        let mut snapshot = DirectorySnapshot::capture(&template_root);
        let relocated = snapshot.replace_text(
            &template_root.to_string_lossy(),
            &repository_root.to_string_lossy(),
        );
        assert!(
            relocated
                .iter()
                .all(|path| !path.starts_with(LEDGER_DIRECTORY)),
            "the template ledger records its root in {relocated:?}; build this fixture in place"
        );
        for identity_path in [REPOSITORY_IDENTITY_PATH, WORKTREE_IDENTITY_PATH]
            .into_iter()
            .filter(|identity_path| template_root.join(identity_path).exists())
        {
            let identity = fs::read_to_string(template_root.join(identity_path))
                .expect("template identity should read");
            let renewed = snapshot.replace_text(identity.trim(), &Uuid::now_v7().to_string());
            assert!(
                !renewed.is_empty(),
                "the template identity in {identity_path} should be replaced"
            );
        }
        snapshot.restore(&repository_root);
        let refreshed = git_command(self.executable)
            .args(["update-index", "-q", "--refresh"])
            .current_dir(&repository_root)
            .status()
            .expect("git should run");
        assert!(refreshed.success(), "git update-index --refresh failed");
        repository
    }

    /// The root of this template's build for the running test binary, built
    /// under the build lock when no complete build exists.
    fn built_root(self) -> PathBuf {
        let test_binary = env::current_exe().expect("test binary path should resolve");
        let builds = Path::new(self.target_directory)
            .join(TEMPLATES_DIRECTORY)
            .join(
                test_binary
                    .file_name()
                    .expect("test binary should have a file name"),
            )
            .join(self.name);
        fs::create_dir_all(&builds).expect("template directory should exist");
        let builds = fs::canonicalize(&builds).expect("template directory should canonicalize");
        let root = builds.join(self.key(&test_binary));
        let complete = root.with_extension(COMPLETE_EXTENSION);
        if complete.exists() {
            return root;
        }
        let lock = File::create(builds.join(BUILD_LOCK_FILE_NAME))
            .expect("template build lock should open");
        lock.lock().expect("template build lock should be taken");
        if !complete.exists() {
            remove_stale_builds(&builds);
            fs::create_dir(&root).expect("template root should exist");
            (self.build)(&root);
            assert_eq!(
                fs::metadata(root.join(JOURNAL_PATH)).map_or(0, |journal| journal.len()),
                0,
                "a template journal record would carry the template identities to every copy"
            );
            File::create(&complete).expect("template completion should record");
        }
        root
    }

    /// Names a build by the test binary and the `cargo-berth` it ran, so a
    /// rebuild of either starts a new build.
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
