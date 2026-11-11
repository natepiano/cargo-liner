//! An answered incursion beside released work whose integration evidence is lost.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::ObservedIncursion;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::ReleasedWorkEvidence;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;

use super::hook_repository;
use super::hook_repository::CONFIGURED_REPOSITORY;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};
pub(super) const EVIDENCE_SESSION: &str = "evidence-session";
pub(super) const INCURSION_SESSION: &str = "incursion-session";
pub(super) const STATE: &str = ".git/hook-evidence-state.json";

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct LostEvidenceState {
    pub(super) reservation_id: String,
    pub(super) protected_tip:  String,
    pub(super) trunk:          String,
}

pub(super) const INCURSION_ANSWERED_LOST_EVIDENCE: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       EXECUTABLE,
        name:             "incursion-answered-lost-evidence",
        build:            |root| {
            let built = build_answered_incursion_with_lost_evidence(root);
            assert!(
                built.is_ok(),
                "answered incursion fixture failed: {built:?}"
            );
        },
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// Released work with lost trunk evidence before the hook response checks.
pub(super) const REWRITTEN_TRUNK_LOST_EVIDENCE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "rewritten-trunk-lost-evidence",
        build: |root| {
            let built = {
                CONFIGURED_REPOSITORY.restore_into(root);
                build_lost_evidence(root, &root.join(".git/fixture-worktrees"))
            };
            assert!(built.is_ok(), "lost evidence fixture failed: {built:?}");
        },
        ..INCURSION_ANSWERED_LOST_EVIDENCE.repository
    },
    ..INCURSION_ANSWERED_LOST_EVIDENCE
};

fn build_answered_incursion_with_lost_evidence(root: &Path) -> TestResult {
    CONFIGURED_REPOSITORY.restore_into(root);
    let incursion = prepare_incursion(root, 1)?;
    hook_repository::require_success(&incursion.output, "incursion hook")?;
    record_incursions(
        &incursion.straying_root,
        &incursion.straying_reservation_id,
        &incursion.incidents,
    )?;
    build_lost_evidence(root, &root.join(".git/fixture-worktrees"))
}

fn build_lost_evidence(root: &Path, parent: &Path) -> TestResult {
    let trunk_before = GIT.stdout(root, ["rev-parse", "main"]);
    let ReleasedWorkEvidence {
        reservation_id,
        protected_tip,
        ..
    } = settle_integration_evidence(root, parent)?;
    GIT.run(root, ["reset", "--hard", "--quiet", &trunk_before]);
    fs::write(root.join("rewritten.rs"), "// rewritten\n")?;
    GIT.run(root, ["add", "rewritten.rs"]);
    GIT.run(
        root,
        [
            "commit",
            "--quiet",
            "-m",
            "rewrite trunk past the released work",
        ],
    );
    hook_repository::require_success(
        &hook_repository::run_berth(root, &["board", "--json"])?,
        "reconcile lost evidence",
    )?;
    let trunk = GIT.stdout(root, ["rev-parse", "main"]);
    fs::write(
        root.join(STATE),
        serde_json::to_vec(&LostEvidenceState {
            reservation_id,
            protected_tip,
            trunk,
        })?,
    )?;
    Ok(())
}

/// A hook response and the incidents that it reports.
pub(super) struct IncursionObservation {
    pub(super) output:                  Output,
    pub(super) straying_reservation_id: String,
    pub(super) incidents:               Vec<ObservedIncursion>,
    pub(super) straying_root:           PathBuf,
}

pub(super) fn prepare_incursion(
    repository: &Path,
    path_count: usize,
) -> TestResult<IncursionObservation> {
    let worktrees = repository.join(".git/fixture-worktrees");
    fs::create_dir_all(&worktrees)?;
    for index in 0..path_count {
        let holder_root = worktrees.join(format!("holder-{index}"));
        hook_repository::add_named_worktree(repository, &format!("holder-{index}"), &holder_root)?;
        hook_repository::dirty_source(&holder_root, &format!("path-{index}.rs"))?;
        let scope = format!("file:path-{index}.rs");
        let holder = hook_repository::run_berth_with_session(
            &holder_root,
            &["claim", &scope, "--json"],
            &format!("incursion-holder-{index}"),
        )?;
        hook_repository::require_success(&holder, "incursion holder claim")?;
    }
    let straying_root = worktrees.join("straying");
    hook_repository::add_named_worktree(repository, "straying", &straying_root)?;
    let straying = hook_repository::run_berth_with_session(
        &straying_root,
        &["claim", "file:straying.rs", "--json"],
        INCURSION_SESSION,
    )?;
    hook_repository::require_success(&straying, "straying claim")?;
    let straying_reservation_id = hook_repository::claimed_reservation_id(&straying)?;
    for index in 0..path_count {
        fs::write(
            straying_root.join(format!("path-{index}.rs")),
            "// entered\n",
        )?;
    }

    let output = hook_repository::run_post_tool_use(
        &straying_root,
        &hook_repository::bash_payload(&straying_root, INCURSION_SESSION),
    )?;
    let incidents = observed_incursions(repository, path_count)?;
    Ok(IncursionObservation {
        output,
        straying_reservation_id,
        incidents,
        straying_root,
    })
}
pub(super) fn observed_incursions(
    repository_root: &Path,
    path_count: usize,
) -> TestResult<Vec<ObservedIncursion>> {
    let board = hook_repository::run_berth(repository_root, &["board", "--json"])?;
    hook_repository::require_success(&board, "incursion board")?;
    let envelope = hook_repository::json_output(&board)?;
    let entries = hook_repository::required_array(
        &envelope,
        "/payload/data/outstanding_incursions/entries",
        "incursion board",
    )?;
    (0..path_count)
        .map(|index| {
            let path = format!("path-{index}.rs");
            let entry = entries
                .iter()
                .find(|entry| {
                    entry["entered_paths"]
                        .as_array()
                        .is_some_and(|paths| paths.iter().any(|entered| *entered == path))
                })
                .ok_or_else(|| {
                    hook_repository::failure(format!(
                        "the board should report an incursion into {path}"
                    ))
                })?;
            Ok(ObservedIncursion {
                incident_id:            hook_repository::required_string(
                    entry,
                    "/incident_id",
                    "incursion entry",
                )?
                .to_owned(),
                foreign_reservation_id: hook_repository::required_string(
                    entry,
                    "/foreign_reservation_ids/0",
                    "incursion entry",
                )?
                .to_owned(),
            })
        })
        .collect()
}
pub(super) fn record_incursions(
    straying_root: &Path,
    straying_reservation_id: &str,
    incidents: &[ObservedIncursion],
) -> TestResult {
    for incident in incidents {
        let recorded = hook_repository::run_berth_with_session(
            straying_root,
            &[
                "resolve",
                straying_reservation_id,
                "--incursion",
                &incident.incident_id,
                "--json",
            ],
            INCURSION_SESSION,
        )?;
        hook_repository::require_success(&recorded, "incursion answer")?;
    }
    Ok(())
}
pub(super) fn settle_integration_evidence(
    repository: &Path,
    worktrees: &Path,
) -> TestResult<ReleasedWorkEvidence> {
    let integrated_root = worktrees.join("integrated");
    hook_repository::add_named_worktree(repository, "integrated", &integrated_root)?;
    let source = "src/integrated.rs";
    fs::create_dir_all(integrated_root.join("src"))?;
    fs::write(integrated_root.join(source), "// released work\n")?;
    hook_repository::run_git(&integrated_root, &["add", source])?;
    hook_repository::run_git(
        &integrated_root,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "released work",
        ],
    )?;
    let claimed = hook_repository::run_berth_with_session(
        &integrated_root,
        &["claim", &format!("file:{source}"), "--json"],
        EVIDENCE_SESSION,
    )?;
    hook_repository::require_success(&claimed, "released work claim")?;
    let reservation_id = hook_repository::claimed_reservation_id(&claimed)?;
    let released =
        hook_repository::run_berth(&integrated_root, &["release", &reservation_id, "--json"])?;
    hook_repository::require_success(&released, "released work release")?;
    let protected_tip = hook_repository::required_string(
        &hook_repository::json_output(&released)?,
        "/payload/data/protected_tip",
        "released work release",
    )?
    .to_owned();

    hook_repository::run_git(
        repository,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--quiet",
            "--no-ff",
            "-m",
            "integrate released work",
            "integrated",
        ],
    )?;
    let reconciled = hook_repository::run_berth(repository, &["board", "--json"])?;
    hook_repository::require_success(&reconciled, "settle integrated reservation")?;
    let settled = hook_repository::run_berth(repository, &["release", &reservation_id, "--json"])?;
    hook_repository::require_success(&settled, "automatic integration settlement")?;
    assert_eq!(
        hook_repository::json_output(&settled)?["payload"]["data"]["status"],
        "already_settled"
    );
    assert_eq!(
        hook_repository::json_output(&settled)?["payload"]["data"]["evidence"]["status"],
        "integrated"
    );
    Ok(ReleasedWorkEvidence {
        reservation_id,
        protected_tip,
        reporting_root: integrated_root,
    })
}
