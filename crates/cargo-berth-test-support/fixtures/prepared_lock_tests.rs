//! Prepared histories for lock and proof tests. The tests execute their own commands.

use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const MAIN_COORDINATION_RUN_ID: &str = "01a03f63-03e7-7fb2-ae63-5b297177f59f";
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
/// The four histories that the nextest setup prepares for these tests.
pub const PREPARED_LOCK_REPOSITORIES: [LedgerTemplate; 4] = [
    HOOK_WORKTREE,
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

/// A settled reservation and the commits that test its integration proof.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PreparedProofHistory<Commits> {
    /// The reservation whose proof the test observes.
    pub reservation_id:   String,
    /// The base of the reservation's protected work.
    pub phase_start_head: String,
    /// The commit that holds the protected work.
    pub protected_tip:    String,
    /// The alternate commits for the proof under test.
    pub commits:          Commits,
}

/// A competitor whose committed scope differs from the protected scope.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AncestorProofCommits {
    /// The competitor's protected tip.
    pub competitor: String,
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
    let foreign = root.join(".git/fixture-worktrees/busy-ledger-holder");
    fs::create_dir_all(foreign.parent().ok_or("linked checkout parent")?)
        .map_err(|error| format!("linked checkout parent should exist: {error}"))?;
    GIT.add_worktree_without_hooks(root, &foreign, "busy-ledger-holder", "HEAD");
    let policy = foreign.join(CONFIGURATION_PATH);
    fs::create_dir_all(policy.parent().ok_or("policy parent")?)
        .map_err(|error| format!("policy parent should exist: {error}"))?;
    fs::copy(root.join(CONFIGURATION_PATH), policy)
        .map_err(|error| format!("linked policy should copy: {error}"))?;
    Ok(())
}

fn protected_phase(root: &Path) -> FixtureResult<PreparedProofHistory<()>> {
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
    Ok(PreparedProofHistory {
        reservation_id,
        phase_start_head,
        protected_tip,
        commits: (),
    })
}

fn prepare_ancestor(root: &Path) -> FixtureResult<()> {
    let subject = protected_phase(root)?;
    settle_protected_phase(root, &subject)?;
    let competitor =
        GIT.commit_competitor_scope(root, "src/distinct-0.rs", &subject.phase_start_head, 0);
    write_history(
        root,
        &PreparedProofHistory {
            reservation_id:   subject.reservation_id,
            phase_start_head: subject.phase_start_head,
            protected_tip:    subject.protected_tip,
            commits:          AncestorProofCommits { competitor },
        },
    )?;
    Ok(())
}

fn prepare_content(root: &Path) -> FixtureResult<()> {
    let info = protected_phase(root)?;
    settle_protected_phase(root, &info)?;
    let base = &info.phase_start_head;
    let warmed = GIT.commit_library_target_from_base(
        root,
        base,
        "pub fn protected() {}\n",
        "equivalent proof target",
    );
    let output = run(root, &["board", "--json"])?;
    assert_success(&output);
    let board = json(&output);
    let row = cargo_berth_test_support::reservation_row(&board, &info.reservation_id);
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
        &PreparedProofHistory {
            reservation_id:   info.reservation_id,
            phase_start_head: info.phase_start_head,
            protected_tip:    info.protected_tip,
            commits:          ContentProofCommits {
                warmed_target:  warmed,
                amended_target: amended,
                advances:       [first, second],
            },
        },
    )?;
    Ok(())
}

fn settle_protected_phase(root: &Path, info: &PreparedProofHistory<()>) -> FixtureResult<()> {
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
        "op":"checkpoint", "reservation_id":info.reservation_id,
        "protected_tip":info.protected_tip, "trunk_snapshot":info.protected_tip});
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
    let row = cargo_berth_test_support::reservation_row(&board, &info.reservation_id);
    assert_eq!(
        row["lifecycle"]["stage"], "released",
        "ordinary reconciliation should settle the reachable checkpoint"
    );
    assert_eq!(
        row["integration_evidence"]["status"]["proof"],
        "protected_tip_ancestor"
    );
    Ok(())
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
