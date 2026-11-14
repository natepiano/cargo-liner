//! Recorded starting histories for named parallel phases.
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::CommandDeadlineExt as _;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;
use cargo_berth_test_support::reservation_row;

use crate::recorded_history;
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const DIRECTOR_SESSION: &str = "dir-sid";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
pub(super) const STATE: &str = ".git/named-phase-state.json";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &["CARGO_BERTH_RUN", "CARGO_BERTH_SESSION_ID"],
};
const INITIALIZED: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "parallel-initialized",
    build:            build_initialized,
};
/// The parallel tests' starting repository, built by the setup script before they start.
pub(crate) const INITIALIZED_BEFORE_RUN: LedgerTemplate = INITIALIZED.before_run();
pub(crate) const NAMED_SEED: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "parallel-named-seed",
        build: |root| build_named(root, Phase16InitialCoverage::SeedOnly),
        ..INITIALIZED
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};
pub(crate) const NAMED_OVERLAP: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "parallel-named-overlap",
        build: |root| build_named(root, Phase16InitialCoverage::SeedAndOverlap),
        ..INITIALIZED
    },
    ..NAMED_SEED
};
#[derive(Clone, Copy)]
pub(super) enum Phase16InitialCoverage {
    SeedOnly,
    SeedAndOverlap,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct NamedPhaseState {
    pub r15:    String,
    pub r16:    String,
    pub holder: String,
}

fn build_initialized(root: &Path) {
    GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
    GIT.run(root, ["config", "user.name", "Berth Test"]);
    GIT.run(root, ["config", "user.email", "berth@example.invalid"]);
    fs::write(root.join("README.md"), "parallel phase fixture\n")
        .expect("initial file should write");
    GIT.run(root, ["add", "README.md"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth_with_session(root, &["init", "--json"], DIRECTOR_SESSION);
    assert_success(&initialized);
    GIT.run(root, ["add", "--force", CONFIGURATION_PATH]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "configure berth"]);
}
fn build_named(root: &Path, coverage: Phase16InitialCoverage) {
    INITIALIZED.restore_into(root);
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let worktrees = root.join(".git/fixture-worktrees");
    fs::create_dir_all(&worktrees).expect("worktree parent");
    let holder_root = worktrees.join("holder");
    GIT.run_without_hooks(
        root,
        [
            "worktree",
            "add",
            "--quiet",
            "-b",
            "holder",
            holder_root.to_str().expect("worktree path"),
        ],
    );
    let claimed = run_berth_with_session(
        &holder_root,
        &["claim", "file:shared.rs", "--json"],
        "holder-session",
    );
    assert_success(&claimed);
    let holder = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("holder id")
        .to_owned();
    fs::write(holder_root.join("shared.rs"), "// foreign holder work\n")
        .expect("holder's real overlap");
    let _ = phase_board(root);
    let r16 = match coverage {
        Phase16InitialCoverage::SeedOnly => open_phase(root, "file:b.rs", "dir-sid.phase-16"),
        Phase16InitialCoverage::SeedAndOverlap => {
            let claimed = run_berth_with_session(
                root,
                &[
                    "claim",
                    "file:b.rs",
                    "file:shared.rs",
                    "--override",
                    &holder,
                    "--overlap-why",
                    "seed phase 16 overlap",
                    "--json",
                ],
                "dir-sid.phase-16",
            );
            assert_success(&claimed);
            json(&claimed)["payload"]["data"]["reservation_id"]
                .as_str()
                .expect("phase 16 id")
                .to_owned()
        },
    };
    assert_success(&run_berth_with_session(
        root,
        &["check", "--reservation", &r15, "file:a.rs", "--json"],
        DIRECTOR_SESSION,
    ));
    let mapping = run_berth_with_session(root, &["board", "--json"], DIRECTOR_SESSION);
    assert_success(&mapping);
    fs::write(
        root.join(STATE),
        serde_json::to_vec(&NamedPhaseState { r15, r16, holder })
            .expect("named phase state should encode"),
    )
    .expect("named phase state should write");
}
fn run_berth_with_session(root: &Path, arguments: &[&str], session: &str) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session)
        .bounded_output()
        .expect("phase command should run")
}
fn open_phase(root: &Path, path: &str, session_id: &str) -> String {
    let opened = run_berth_with_session(root, &["check", path, "--json"], session_id);
    assert_success(&opened);
    let opened = json(&opened);
    assert_eq!(opened["payload"]["data"]["acquisition"]["kind"], "appended");
    opened["payload"]["data"]["acquisition"]["reservation_id"]
        .as_str()
        .expect("phase reservation")
        .to_owned()
}

fn phase_board(root: &Path) -> serde_json::Value {
    let output = run_berth_with_session(root, &["board", "--json"], DIRECTOR_SESSION);
    assert_success(&output);
    json(&output)
}

pub(super) const NAMED_METADATA: LedgerTemplate =
    recorded_history::template("parallel-named-metadata", |destination| {
        let repository = INITIALIZED.instantiate();
        let root = repository.path();
        let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
        let baseline = GIT.stdout(root, ["rev-parse", "HEAD"]);
        GIT.run(root, ["branch", "phase-target"]);
        let claimed = run_berth_with_session(
            root,
            &[
                "claim",
                "file:b.rs",
                "--target",
                "phase-target",
                "--head",
                &baseline,
                "--why",
                "phase 16 purpose",
                "--plan",
                "docs/plan.md",
                "--phase",
                "16",
                "--json",
            ],
            "dir-sid.phase-16",
        );
        assert_success(&claimed);
        let claimed = json(&claimed);
        let r16 = claimed["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("phase 16");
        let original = reservation_row(&phase_board(root), r16).clone();
        fs::write(root.join("a.rs"), "// keep phase 15 active\n").expect("phase 15 work");
        fs::write(root.join("b.rs"), "// keep phase 16 active\n").expect("phase 16 work");
        fs::write(root.join("later.txt"), "later head\n").expect("new commit");
        GIT.run(root, ["add", "later.txt"]);
        GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "advance caller head"]);
        assert_ne!(GIT.stdout(root, ["rev-parse", "HEAD"]), baseline);
        assert_success(&run_berth_with_session(
            root,
            &["check", "--reservation", &r15, "file:a.rs", "--json"],
            DIRECTOR_SESSION,
        ));

        recorded_history::save(
            destination,
            root,
            &NamedPhaseMetadata {
                r16: r16.to_owned(),
                baseline,
                original,
            },
        );
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NamedPhaseMetadata {
    pub(super) r16:      String,
    pub(super) baseline: String,
    pub(super) original: serde_json::Value,
}

/// A unit worktree with phases A and B open, after phase A committed `a.rs`.
pub(crate) const SIBLING_PHASES: LedgerTemplate =
    recorded_history::template("parallel-sibling-phases", |destination| {
        let repository = INITIALIZED.instantiate();
        let worktree_parent = tempfile::tempdir().expect("unit worktree directory");
        let phases = open_sibling_phases(repository.path(), worktree_parent.path());
        recorded_history::save(destination, repository.path(), &phases);
    });
/// [`SIBLING_PHASES`] after phase A's checkpoint and phase B's drift over `b-extra.rs`.
pub(crate) const SIBLING_PHASES_AFTER_DRIFT: LedgerTemplate =
    recorded_history::template("parallel-sibling-phases-after-drift", |destination| {
        let repository = INITIALIZED.instantiate();
        let worktree_parent = tempfile::tempdir().expect("unit worktree directory");
        let phases = open_sibling_phases(repository.path(), worktree_parent.path());
        assert_success(&run_berth_with_session(
            &phases.unit,
            &["release", &phases.first, "--json"],
            "dir-sid.phase-1",
        ));
        fs::write(phases.unit.join("b-extra.rs"), "// phase B extra work\n")
            .expect("extra phase B file");
        assert_success(&run_berth_with_session(
            &phases.unit,
            &["drift", "--full", "--reservation", &phases.second, "--json"],
            "dir-sid.phase-2",
        ));
        recorded_history::save(destination, repository.path(), &phases);
    });
/// The unit worktree and the reservations of its two phases.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SiblingPhases {
    pub(crate) unit:   PathBuf,
    pub(crate) first:  String,
    pub(crate) second: String,
}

fn open_sibling_phases(trunk: &Path, worktree_parent: &Path) -> SiblingPhases {
    let unit = worktree_parent.join("unit");
    GIT.add_worktree_without_hooks(trunk, &unit, "unit", "main");
    let first = open_phase(&unit, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(&unit, "file:b.rs", "dir-sid.phase-2");
    fs::write(unit.join("a.rs"), "// phase A checkpoint\n").expect("phase A file");
    fs::write(unit.join("b.rs"), "// phase B work\n").expect("phase B file");
    GIT.run(&unit, ["add", "a.rs"]);
    GIT.run_without_hooks(&unit, ["commit", "--quiet", "-m", "phase A checkpoint"]);
    SiblingPhases {
        unit,
        first,
        second,
    }
}
