//! Enrolled overlapping lanes, prepared before the gate tests start.
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::CrossTargetOrdering;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &["CARGO_BERTH_BYPASS"],
};
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIXTURE_USER_EMAIL: &str = "test@example.com";
const FIXTURE_USER_NAME: &str = "Test User";
/// Two committed lanes, before dirty work causes berth to enroll either checkout.
pub const UNENROLLED_PAIR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "unenrolled-pair",
        build: build_unenrolled_pair,
        ..ENROLLED_PAIR.repository
    },
    ledger: LedgerFixture::Empty,
    ..ENROLLED_PAIR
};

/// A predecessor landed on main, then revalidated against its advanced integration target.
pub const REVALIDATED_CROSS_TARGET_LANDING: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "revalidated-cross-target-landing",
        build: build_revalidated_cross_target_landing,
        ..ENROLLED_PAIR.repository
    },
    ..ENROLLED_PAIR
};

/// Commit witnesses needed to ask the trunk gate about the earlier landing.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CrossTargetLandingWitnesses {
    /// The predecessor's first integration tip, already landed on main.
    pub first_landing:  String,
    /// The later integration tip used to revalidate the predecessor.
    pub second_landing: String,
    /// Successor work based on main's first landing, without the later tip.
    pub successor_tip:  String,
}

fn build_revalidated_cross_target_landing(root: &Path) {
    IntegrationRepository::build_cross_target_pair(
        root,
        BERTH_EXECUTABLE,
        CrossTargetOrdering::After,
    );
    let worktrees = root.join(".git/fixture-worktrees");
    let integration = worktrees.join("integration");
    let successor = worktrees.join("cross-x");
    // These landings and commits deliberately run the managed hooks: the gate must remember
    // the first landing even after the predecessor's target advances and revalidates it.
    git(&integration, &["merge", "--ff-only", "cross-a"]);
    let first_landing = git_stdout(root, &["rev-parse", "integration"]);
    git(root, &["merge", "--ff-only", "integration"]);
    assert_eq!(git_stdout(root, &["rev-parse", "main"]), first_landing);
    commit_work(
        &integration,
        "integration-later.txt",
        "later\n",
        "I revalidates A",
    );
    let second_landing = git_stdout(root, &["rev-parse", "integration"]);
    assert_ne!(first_landing, second_landing);
    let board = run_berth(root, &["board", "--json"]);
    assert!(board.status.success(), "{}", json_output(&board));
    commit_work(&successor, "shared.txt", "X\n", "X work");
    git(&successor, &["merge", "-s", "ours", "--no-edit", "main"]);
    let state = CrossTargetLandingWitnesses {
        first_landing,
        second_landing,
        successor_tip: git_stdout(&successor, &["rev-parse", "HEAD"]),
    };
    fs::write(
        root.join(".git/cross-target-landing.json"),
        serde_json::to_vec(&state).expect("landing witnesses should encode"),
    )
    .expect("landing witnesses should write");
}

/// Read the commit witnesses saved with the hooked cross-target history.
///
/// # Panics
/// Panics when the saved witnesses cannot be read or decoded.
#[must_use]
pub fn read_cross_target_landing(root: &Path) -> CrossTargetLandingWitnesses {
    serde_json::from_slice(
        &fs::read(root.join(".git/cross-target-landing.json"))
            .expect("landing witnesses should read"),
    )
    .expect("landing witnesses should decode")
}

/// Two overlapping enrolled lanes with authorized commits and drift reconciled.
pub const ENROLLED_PAIR: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       BERTH_EXECUTABLE,
        name:             "enrolled-pair",
        build:            build_enrolled_pair,
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// The shared base and both reservation identities of an enrolled pair.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct EnrolledPair {
    /// Main’s commit before either enrolled lane changed it.
    pub base:            String,
    /// First and second lane reservations, in enrollment order.
    pub reservation_ids: [String; 2],
}

fn build_enrolled_pair(root: &Path) {
    build_unenrolled_pair(root);
    let state = enroll_pair(root);
    fs::write(
        root.join(".git/enrolled-pair.json"),
        serde_json::to_vec(&state).expect("enrollment state should encode"),
    )
    .expect("enrollment state should write");
}
/// Read the state saved by the enrolled-pair builder.
///
/// # Panics
/// Panics when the fixture state cannot be read or decoded.
#[must_use]
pub fn read_enrolled_pair(root: &Path) -> EnrolledPair {
    serde_json::from_slice(
        &fs::read(root.join(".git/enrolled-pair.json")).expect("enrollment state should read"),
    )
    .expect("enrollment state should decode")
}
/// Enroll both dirty lanes, authorize their edits, and reconcile their commits.
///
/// # Panics
/// Panics when any setup command or enrollment assertion fails.
#[must_use]
pub fn enroll_pair(root: &Path) -> EnrolledPair {
    let base = git_stdout(root, &["rev-parse", "main"]);
    let worktrees = root.join(".git/fixture-worktrees");
    let first = worktrees.join("enrolled-first");
    let second = worktrees.join("enrolled-second");
    for checkout in [&first, &second] {
        let path = checkout.join("src/lib.rs");
        let contents = fs::read_to_string(&path).expect("committed lane work should read");
        fs::write(path, format!("{contents}// dirty before init\n"))
            .expect("dirty shared work should write");
    }
    let initialized = run_berth(root, &["init", "--json"]);
    let initialized_envelope = json_output(&initialized);
    assert!(initialized.status.success(), "{initialized_envelope}");
    let enrollment = &initialized_envelope["payload"]["data"]["enrollment"];
    assert_eq!(
        enrollment["enrolled"]
            .as_array()
            .expect("enrolled rows")
            .len(),
        2
    );
    assert_eq!(
        enrollment["overlaps"]
            .as_array()
            .expect("overlap rows")
            .len(),
        1
    );
    set_gate_mode(root, "observe");
    let mut reservation_ids = Vec::new();
    for (checkout, session) in [
        (&first, "enrolled-first-session"),
        (&second, "enrolled-second-session"),
    ] {
        let checked =
            run_berth_with_session(checkout, &["check", "file:src/lib.rs", "--json"], session);
        let envelope = json_output(&checked);
        assert!(checked.status.success(), "{envelope}");
        assert_eq!(envelope["status"], "clear");
        assert_eq!(
            envelope["payload"]["data"]["acquisition"]["kind"],
            "already_held"
        );
        reservation_ids.push(
            envelope["payload"]["data"]["acquisition"]["reservation_id"]
                .as_str()
                .expect("reused enrolled reservation")
                .to_owned(),
        );
        commit_work(
            checkout,
            "src/lib.rs",
            &format!("// edited by {session}\n"),
            "authorized shared edit",
        );
        let drift = run_berth_with_session(checkout, &["drift", "--full", "--json"], session);
        assert!(drift.status.success(), "{}", json_output(&drift));
    }
    let board = run_berth(root, &["board", "--json"]);
    let board_envelope = json_output(&board);
    assert!(board.status.success(), "{board_envelope}");
    assert_eq!(
        board_envelope["payload"]["data"]["outstanding_incursions"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(
        board_envelope["payload"]["data"]["unresolved_overlaps"]["entries"][0]["origin"],
        "enrollment"
    );

    EnrolledPair {
        base,
        reservation_ids: reservation_ids
            .try_into()
            .expect("two enrolled reservations should exist"),
    }
}

fn build_unenrolled_pair(root: &Path) {
    build_scratch_repository(root);
    let worktrees = root.join(".git/fixture-worktrees");
    for (branch, contents) in [
        ("enrolled-first", "pub fn first_committed() {}\n"),
        ("enrolled-second", "pub fn second_committed() {}\n"),
    ] {
        let checkout = add_worktree(root, &worktrees, branch);
        commit_work_without_hooks(&checkout, "src/lib.rs", contents, "work before enrollment");
    }
}

/// Create the uninitialized main checkout used by enrollment scenarios.
///
/// # Panics
/// Panics when repository setup cannot complete.
pub fn build_scratch_repository(repository_root: &Path) {
    git(
        repository_root,
        &["init", "--quiet", "--initial-branch", "main"],
    );
    // Git runs `maintenance run --auto --detach` after a commit, and this machine leaves that
    // default on. Several detached runs then repack one repository at once, and the geometric
    // repack deletes a pack a commit still in flight is reading, which surfaces as
    // `invalid object <oid> for '<path>'` from a commit that did nothing wrong. A fixture
    // repository is short-lived and never needs maintenance, so it opts out of both schedulers.
    git(repository_root, &["config", "maintenance.auto", "false"]);
    git(repository_root, &["config", "gc.auto", "0"]);
    git(
        repository_root,
        &["config", "user.email", FIXTURE_USER_EMAIL],
    );
    git(repository_root, &["config", "user.name", FIXTURE_USER_NAME]);
    fs::create_dir_all(repository_root.join("src")).expect("source directory should exist");
    fs::create_dir_all(repository_root.join("tests")).expect("test directory should exist");
    fs::write(repository_root.join("src/lib.rs"), "pub fn base() {}\n")
        .expect("base source should write");
    fs::write(repository_root.join("tests/base.rs"), "// base\n").expect("base test should write");
    git(repository_root, &["add", "."]);
    git(repository_root, &["commit", "--quiet", "-m", "initial"]);
}

/// Rewrite the fixture’s configured gate mode.
///
/// # Panics
/// Panics when the fixture configuration cannot be read or written.
pub fn set_gate_mode(repository_root: &Path, mode: &str) {
    let configuration_path = repository_root.join(CONFIGURATION_PATH);
    let configuration = fs::read_to_string(&configuration_path).expect("configuration should read");
    let updated = configuration
        .lines()
        .map(|line| {
            if line.starts_with("gate_mode") {
                format!("gate_mode = \"{mode}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(configuration_path, format!("{updated}\n")).expect("configuration should write");
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let worktree_root = parent.join(branch);
    GIT.add_worktree_without_hooks(repository_root, &worktree_root, branch, "HEAD");
    worktree_root
}

fn commit_work_without_hooks(
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

fn commit_work(repository_root: &Path, path: &str, contents: &str, message: &str) -> String {
    fs::write(repository_root.join(path), contents).expect("work source should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn git(root: &Path, args: &[&str]) { GIT.run(root, args); }
fn git_stdout(root: &Path, args: &[&str]) -> String { GIT.stdout(root, args) }
fn json_output(output: &Output) -> serde_json::Value { cargo_berth_test_support::json(output) }
fn run_berth(root: &Path, args: &[&str]) -> Output {
    cargo_berth_test_support::berth_command(BERTH_EXECUTABLE)
        .args(args)
        .current_dir(root)
        .env_remove("CARGO_BERTH_BYPASS")
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("enrollment command should run")
}
fn run_berth_with_session(root: &Path, args: &[&str], session: &str) -> Output {
    cargo_berth_test_support::berth_command(BERTH_EXECUTABLE)
        .args(args)
        .current_dir(root)
        .env_remove("CARGO_BERTH_BYPASS")
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session)
        .output()
        .expect("enrollment session command should run")
}
