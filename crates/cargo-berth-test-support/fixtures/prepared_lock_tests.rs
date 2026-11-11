//! Prepared histories for lock and proof tests. The tests execute their own commands.

use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::MAIN_COORDINATION_RUN_ID;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::SECOND_RUN;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &["CARGO_BERTH_RUN", "CARGO_BERTH_SESSION_ID"],
};

type FixtureResult<T> = Result<T, Box<dyn Error>>;

/// An initialized repository with a private foreign checkout and no claims.
pub const HOOK_WORKTREE: LedgerTemplate = template("lock-hook-worktree", |root| {
    let built = prepare_hook(root);
    assert!(built.is_ok(), "prepared hook history failed: {built:?}");
});
/// A private foreign checkout whose live claim holds work, before the held-path checks.
pub const LIVE_FOREIGN_HOLDER: LedgerTemplate = template("lock-live-foreign-holder", |root| {
    let built = prepare_live_holder(root);
    assert!(built.is_ok(), "prepared live holder failed: {built:?}");
});
/// The foreign checkout inside a `LIVE_FOREIGN_HOLDER` repository.
pub const LIVE_FOREIGN_HOLDER_CHECKOUT: &str = ".git/fixture-worktrees/live-holder";
/// A packed released predecessor and its active successor, before live abandonment.
pub const PACKED_SUCCESSOR: LedgerTemplate = template("lock-packed-successor", |root| {
    let built = prepare_retention(root);
    assert!(
        built.is_ok(),
        "prepared retention history failed: {built:?}"
    );
});
/// An ancestry proof and a distinct competitor, before the live trunk rewrite.
pub const ANCESTOR_CHECKPOINT: LedgerTemplate = template("lock-ancestor-checkpoint", |root| {
    let built = prepare_ancestor(root);
    assert!(built.is_ok(), "prepared ancestor history failed: {built:?}");
});
/// A scoped proof and alternate Git targets, before the live revalidation commands.
pub const CONTENT_CHECKPOINT: LedgerTemplate = template("lock-content-checkpoint", |root| {
    let built = prepare_content(root);
    assert!(built.is_ok(), "prepared content history failed: {built:?}");
});
/// The five histories that the nextest setup prepares for these tests.
pub const PREPARED_LOCK_REPOSITORIES: [LedgerTemplate; 5] = [
    HOOK_WORKTREE,
    LIVE_FOREIGN_HOLDER,
    PACKED_SUCCESSOR,
    ANCESTOR_CHECKPOINT,
    CONTENT_CHECKPOINT,
];

const fn template(name: &'static str, build: fn(&Path)) -> LedgerTemplate {
    LedgerTemplate {
        repository:   RepositoryTemplate {
            target_directory: env!("CARGO_TARGET_TMPDIR"),
            executable: BERTH_EXECUTABLE,
            name,
            build,
        },
        ledger:       LedgerFixture::Recorded,
        build_timing: TemplateBuildTiming::BeforeRun,
    }
}

/// A claimed reservation whose protected work is committed and not yet settled.
struct ActiveProofHistory {
    reservation_id:   String,
    phase_start_head: String,
    protected_tip:    String,
}

/// A released reservation and the commits that test its integration proof.
/// Only settlement of an active history produces this type.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ReleasedProofHistory<Commits> {
    /// The reservation whose proof the test observes.
    pub reservation_id:   String,
    /// The base of the reservation's protected work.
    pub phase_start_head: String,
    /// The commit that holds the protected work.
    pub protected_tip:    String,
    /// The alternate commits for the proof under test.
    pub commits:          Commits,
}

impl ReleasedProofHistory<()> {
    /// Attach the commits that the proof under test reads.
    fn with_commits<Commits>(self, commits: Commits) -> ReleasedProofHistory<Commits> {
        ReleasedProofHistory {
            reservation_id: self.reservation_id,
            phase_start_head: self.phase_start_head,
            protected_tip: self.protected_tip,
            commits,
        }
    }
}

/// Two competitors whose committed scope differs from the protected scope.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AncestorProofCommits {
    /// The protected tip of the competitor that the first deferred pass reads.
    pub competitor:       String,
    /// The protected tip of the competitor that the second deferred pass adds.
    pub later_competitor: String,
}

/// Equivalent content targets and two later trunk commits.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct ContentProofCommits {
    /// The target whose scoped content proof is already recorded.
    pub warmed_target:  String,
    /// An alternate target with the same protected content.
    pub amended_target: String,
    /// Two successive commits after the amended target.
    pub advances:       [String; 2],
}

/// A released predecessor and the active successor that retains it.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RetainedPredecessorHistory {
    /// The reservation whose retention reference is packed.
    pub predecessor_id: String,
    /// The active reservation that the test abandons.
    pub successor_id:   String,
}

/// Read the typed history for a prepared repository.
///
/// # Errors
/// Returns a file error or a JSON decoding error.
pub fn read_history<History: serde::de::DeserializeOwned>(root: &Path) -> FixtureResult<History> {
    Ok(serde_json::from_slice(
        &fs::read(root.join(".git/prepared-lock-history.json"))
            .map_err(|error| format!("prepared history should read: {error}"))?,
    )
    .map_err(|error| format!("prepared history should decode: {error}"))?)
}

fn write_history(root: &Path, history: &impl serde::Serialize) -> FixtureResult<()> {
    fs::write(
        root.join(".git/prepared-lock-history.json"),
        serde_json::to_vec(history)
            .map_err(|error| format!("prepared history should encode: {error}"))?,
    )
    .map_err(|error| format!("prepared history should write: {error}"))?;
    Ok(())
}

fn initialize(root: &Path, file: &str, contents: &str) -> FixtureResult<()> {
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    let configuration = root.join(".git/config");
    let mut text = fs::read_to_string(&configuration)
        .map_err(|error| format!("Git configuration should read: {error}"))?;
    text.push_str("\n[user]\n name = Lock Test\n email = lock@example.invalid\n");
    fs::write(configuration, text)
        .map_err(|error| format!("Git identity should write: {error}"))?;
    if let Some(parent) = root.join(file).parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("source parent should exist: {error}"))?;
    }
    fs::write(root.join(file), contents)
        .map_err(|error| format!("initial source should write: {error}"))?;
    git(root, &["add", file]);
    git(root, &["commit", "--quiet", "-m", "initial"]);
    assert_success(&run(root, &["init", "--json"])?);
    Ok(())
}

fn prepare_hook(root: &Path) -> FixtureResult<()> {
    initialize(root, "README.md", "hook fixture\n")?;
    add_foreign_checkout(root, "busy-ledger-holder")?;
    Ok(())
}

fn prepare_live_holder(root: &Path) -> FixtureResult<()> {
    initialize(root, "README.md", "held path fixture\n")?;
    let foreign = add_foreign_checkout(root, "live-holder")?;
    assert_eq!(foreign, root.join(LIVE_FOREIGN_HOLDER_CHECKOUT));
    assert_success(&run(
        &foreign,
        &["claim", "file:foreign.rs", "--run", SECOND_RUN, "--json"],
    )?);
    fs::write(foreign.join("foreign.rs"), "live foreign work\n")
        .map_err(|error| format!("foreign work should write: {error}"))?;
    Ok(())
}

/// Add a private linked checkout that shares the repository's policy.
fn add_foreign_checkout(root: &Path, name: &str) -> FixtureResult<std::path::PathBuf> {
    let foreign = root.join(".git/fixture-worktrees").join(name);
    fs::create_dir_all(foreign.parent().ok_or("linked checkout parent")?)
        .map_err(|error| format!("linked checkout parent should exist: {error}"))?;
    GIT.add_worktree_without_hooks(root, &foreign, name, "HEAD");
    let policy = foreign.join(CONFIGURATION_PATH);
    fs::create_dir_all(policy.parent().ok_or("policy parent")?)
        .map_err(|error| format!("policy parent should exist: {error}"))?;
    fs::copy(root.join(CONFIGURATION_PATH), policy)
        .map_err(|error| format!("linked policy should copy: {error}"))?;
    Ok(foreign)
}

fn protected_phase(root: &Path) -> FixtureResult<ActiveProofHistory> {
    initialize(root, "src/lib.rs", "pub fn value() {}\n")?;
    let phase_start_head = git_stdout(root, &["rev-parse", "HEAD"]);
    let claimed = run(
        root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "protect board test work",
            "--json",
        ],
    )?;
    assert_success(&claimed);
    let reservation_id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .ok_or("reservation id")?
        .to_owned();
    fs::write(root.join("src/lib.rs"), "pub fn protected() {}\n")
        .map_err(|error| format!("protected source should write: {error}"))?;
    git(root, &["add", "src/lib.rs"]);
    git(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected ancestor",
        ],
    );
    let protected_tip = git_stdout(root, &["rev-parse", "HEAD"]);
    Ok(ActiveProofHistory {
        reservation_id,
        phase_start_head,
        protected_tip,
    })
}

fn prepare_ancestor(root: &Path) -> FixtureResult<()> {
    let released = settle_protected_phase(root, protected_phase(root)?)?;
    let competitor =
        GIT.commit_competitor_scope(root, "src/distinct-0.rs", &released.phase_start_head, 0);
    let later_competitor =
        GIT.commit_competitor_scope(root, "src/distinct-0.rs", &released.phase_start_head, 1);
    write_history(
        root,
        &released.with_commits(AncestorProofCommits {
            competitor,
            later_competitor,
        }),
    )?;
    Ok(())
}

fn prepare_content(root: &Path) -> FixtureResult<()> {
    let released = settle_protected_phase(root, protected_phase(root)?)?;
    let base = &released.phase_start_head;
    let warmed = GIT.commit_library_target_from_base(
        root,
        base,
        "pub fn protected() {}\n",
        "equivalent proof target",
    );
    let output = run(root, &["board", "--json"])?;
    assert_success(&output);
    let board = json(&output);
    let row = cargo_berth_test_support::reservation_row(&board, &released.reservation_id);
    let status = &row["integration_evidence"]["status"];
    assert_eq!(status["status"], "integrated");
    assert_eq!(status["proof"], "scoped_patch_equivalent");
    assert_eq!(status["trunk_oid"], warmed);
    let amended = GIT.commit_library_target_from_base(
        root,
        base,
        "pub fn protected() {}\n",
        "amended equivalent target",
    );
    let first = GIT.advance_trunk(root, "first");
    let second = GIT.advance_trunk(root, "second");
    for (name, head) in [
        ("warmed", &warmed),
        ("amended", &amended),
        ("first", &first),
        ("second", &second),
    ] {
        git(
            root,
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "update-ref",
                &format!("refs/prepared-lock-history/{name}"),
                head,
            ],
        );
    }
    git(
        root,
        &["-c", "core.hooksPath=/dev/null", "reset", "--hard", &warmed],
    );
    write_history(
        root,
        &released.with_commits(ContentProofCommits {
            warmed_target:  warmed,
            amended_target: amended,
            advances:       [first, second],
        }),
    )?;
    Ok(())
}

fn settle_protected_phase(
    root: &Path,
    active: ActiveProofHistory,
) -> FixtureResult<ReleasedProofHistory<()>> {
    let journal_path = root.join(".git/cargo-berth/journal.ndjson");
    let text = fs::read_to_string(&journal_path)
        .map_err(|error| format!("journal should read: {error}"))?;
    let previous: serde_json::Value = serde_json::from_str(
        text.lines()
            .last()
            .ok_or("fixture should contain a claim")?,
    )
    .map_err(|error| format!("claim should decode: {error}"))?;
    let checkpoint = serde_json::json!({
        "schema_version":previous["schema_version"],
        "event_id":uuid::Uuid::now_v7().to_string(),
        "actor":previous["actor"], "at":previous["at"],
        "projection_generation":previous["projection_generation"].as_u64()
            .ok_or("generation should be numeric")? + 1,
        "op":"checkpoint", "reservation_id":active.reservation_id,
        "protected_tip":active.protected_tip, "trunk_snapshot":active.protected_tip});
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(journal_path)
        .map_err(|error| format!("journal should open: {error}"))?;
    writeln!(journal, "{checkpoint}")
        .map_err(|error| format!("checkpoint should append: {error}"))?;
    drop(journal);
    let output = run(root, &["board", "--json"])?;
    assert_success(&output);
    let board = json(&output);
    let row = cargo_berth_test_support::reservation_row(&board, &active.reservation_id);
    assert_eq!(
        row["lifecycle"]["stage"], "released",
        "ordinary reconciliation should settle the reachable checkpoint"
    );
    assert_eq!(
        row["integration_evidence"]["status"]["proof"],
        "protected_tip_ancestor"
    );
    Ok(ReleasedProofHistory {
        reservation_id:   active.reservation_id,
        phase_start_head: active.phase_start_head,
        protected_tip:    active.protected_tip,
        commits:          (),
    })
}

fn prepare_retention(root: &Path) -> FixtureResult<()> {
    let repo = IntegrationRepository::new(BERTH_EXECUTABLE);
    repo.git_without_hooks(repo.root(), &["switch", "--quiet", "-c", "retention-lock"]);
    let predecessor = repo.claim(
        repo.root(),
        "file:retained.txt",
        MAIN_COORDINATION_RUN_ID,
        None,
    );
    assert_success(&predecessor);
    let predecessor_id = json(&predecessor)["reservations"][0]
        .as_str()
        .ok_or("predecessor id")?
        .to_owned();
    fs::write(repo.root().join("retained.txt"), "outstanding work\n")
        .map_err(|error| format!("work should write: {error}"))?;
    repo.git_without_hooks(repo.root(), &["add", "retained.txt"]);
    repo.git_without_hooks(
        repo.root(),
        &["commit", "--quiet", "-m", "outstanding work"],
    );
    assert_success(&repo.run(repo.root(), &["release", &predecessor_id, "--json"]));
    assert_success(&repo.run(repo.root(), &["board", "--json"]));
    repo.git_without_hooks(repo.root(), &["pack-refs", "--all"]);
    let successor = repo.claim(
        repo.root(),
        "file:successor.txt",
        MAIN_COORDINATION_RUN_ID,
        None,
    );
    assert_success(&successor);
    let successor_id = json(&successor)["reservations"][0]
        .as_str()
        .ok_or("successor id")?
        .to_owned();
    assert_success(&repo.run(
        repo.root(),
        &[
            "sequence",
            &predecessor_id,
            &successor_id,
            "--why",
            "retain predecessor until successor ends",
            "--json",
        ],
    ));
    let tip = repo.git_stdout(repo.root(), &["rev-parse", "HEAD"]);
    repo.git_without_hooks(repo.root(), &["branch", "--force", "main", &tip]);
    repo.git_without_hooks(repo.root(), &["config", "core.packedRefsTimeout", "20"]);
    repo.git_without_hooks(repo.root(), &["config", "core.filesRefLockTimeout", "20"]);
    assert_success(&repo.run(repo.root(), &["board", "--json"]));
    let retained = format!("refs/cargo-berth/reservations/{predecessor_id}");
    assert_eq!(repo.git_stdout(repo.root(), &["rev-parse", &retained]), tip);
    repo.git_without_hooks(repo.root(), &["pack-refs", "--all"]);
    write_history(
        repo.root(),
        &RetainedPredecessorHistory {
            predecessor_id,
            successor_id,
        },
    )?;
    repo.write_recorded_template(root);
    Ok(())
}

fn run(root: &Path, arguments: &[&str]) -> FixtureResult<Output> {
    Ok(berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .map_err(|error| format!("fixture command should run: {error}"))?)
}

fn git(root: &Path, arguments: &[&str]) { GIT.run(root, arguments); }
fn git_stdout(root: &Path, arguments: &[&str]) -> String { GIT.stdout(root, arguments) }
