//! Shared board fixtures and accessors for the split module's test modules.

use std::convert::Infallible;
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;
use tempfile::tempdir;

use super::BoardModel;
use super::rows::BoardReservationSnapshot;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapScopeSet;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationReason;
use crate::answer::OverlapScopeRevision;
use crate::config::Enrollment;
use crate::ids::CoordinationRunId;
use crate::ids::EdgeId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadCommit;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::FullRefName;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::NonEmptyReservationPurpose;
use crate::ledger::OrderingDirection;
use crate::ledger::ProtectedPhaseStartHead;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ScopeKind;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkCommitAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const GIT_BINARY: &str = "git";

pub(super) type FixtureResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone)]
pub(super) struct TestActor {
    root:                            PathBuf,
    worktree_id:                     WorktreeId,
    coordination_run_id:             CoordinationRunId,
    worktree_root:                   CanonicalWorktreeRoot,
    worktree_administrative_locator: WorktreeAdministrativeLocator,
}

pub(super) struct BoardFixture {
    pub(super) repository:     TempDir,
    linked_worktree_directory: TempDir,
    ledger:                    Ledger,
    main_actor:                TestActor,
}

pub(super) struct ReservationClaimFixture {
    pub(super) reservation_id: ReservationId,
    scopes:                    ReservationScopeSet,
}

#[derive(Clone, Copy)]
pub(super) enum OverlapAnswerFixture {
    Sequence,
    Defer,
    Override,
}

pub(super) struct AnsweredBoardFixture {
    pub(super) model:        BoardModel,
    pub(super) blocker_id:   ReservationId,
    pub(super) requester_id: ReservationId,
}

pub(super) struct OrderedBoardFixture {
    pub(super) board:             BoardFixture,
    pub(super) predecessor_actor: TestActor,
    pub(super) predecessor:       ReservationClaimFixture,
    pub(super) successor:         ReservationClaimFixture,
}

impl TestActor {
    fn discover(root: &Path) -> FixtureResult<Self> {
        let context = WorktreeContext::discover(root)?;
        let worktree_id =
            ledger::worktree_identity(context.administrative_directory(), context.worktree_kind())?
                .id;
        let canonical_root = fs::canonicalize(context.repository_root())?;
        let worktree_root = canonical_root
            .to_str()
            .ok_or_else(|| io::Error::other("test worktree root should be UTF-8"))?
            .parse::<CanonicalWorktreeRoot>()?;
        Ok(Self {
            root: context.repository_root().to_path_buf(),
            worktree_id,
            coordination_run_id: CoordinationRunId::new(),
            worktree_root,
            worktree_administrative_locator: context.administrative_locator().clone(),
        })
    }

    fn head_snapshot(&self) -> FixtureResult<(ClaimHeadSnapshot, GitObjectId)> {
        let head = git_object_id(&self.root, "HEAD")?;
        let full_ref = git(&self.root, &["symbolic-ref", "HEAD"])?.parse::<FullRefName>()?;
        Ok((
            ClaimHeadSnapshot::Branch {
                full_ref,
                head: ClaimHeadCommit::from(head.clone()),
            },
            head,
        ))
    }
}

impl BoardFixture {
    pub(super) fn new() -> FixtureResult<Self> {
        let repository = tempdir()?;
        git(
            repository.path(),
            &["init", "--quiet", "--initial-branch", "main"],
        )?;
        git(
            repository.path(),
            &["config", "user.email", "board-model@example.com"],
        )?;
        git(
            repository.path(),
            &["config", "user.name", "Board Model Test"],
        )?;
        fs::create_dir_all(repository.path().join("src"))?;
        fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn fixture() {}\n",
        )?;
        git(repository.path(), &["add", "."])?;
        git(repository.path(), &["commit", "--quiet", "-m", "initial"])?;
        Ledger::initialize(repository.path())?;
        git(repository.path(), &["add", CONFIGURATION_PATH])?;
        git(
            repository.path(),
            &["commit", "--quiet", "-m", "configure berth"],
        )?;
        let linked_worktree_directory = tempdir()?;
        let main_actor = TestActor::discover(repository.path())?;
        let ledger = Ledger::open(repository.path())?;
        Ok(Self {
            repository,
            linked_worktree_directory,
            ledger,
            main_actor,
        })
    }

    pub(super) fn main_actor(&self) -> TestActor { self.main_actor.clone() }

    fn linked_actor(&self, branch: &str) -> FixtureResult<TestActor> {
        let root = self.linked_worktree_directory.path().join(branch);
        let root_text = root
            .to_str()
            .ok_or_else(|| io::Error::other("linked worktree root should be UTF-8"))?;
        git(
            self.repository.path(),
            &[
                "worktree", "add", "--quiet", "-b", branch, root_text, "main",
            ],
        )?;
        TestActor::discover(&root)
    }

    pub(super) fn trunk(&self) -> FixtureResult<GitObjectId> {
        git_object_id(self.repository.path(), "main")
    }

    pub(super) fn claim(
        &self,
        actor: &TestActor,
        path: &str,
        authorization: ConflictAuthorization,
    ) -> FixtureResult<ReservationClaimFixture> {
        let scopes = file_scope_set(path)?;
        let reservation_id = ReservationId::new();
        let trunk = self.trunk()?;
        let (head_snapshot, head) = actor.head_snapshot()?;
        self.append_as(
            actor,
            JournalOperation::Claim {
                reservation_id,
                scopes: scopes.clone(),
                source: ClaimSource::Explicit,
                purpose: ReservationPurpose::Explained(
                    "exercise typed board assembly".parse::<NonEmptyReservationPurpose>()?,
                ),
                trunk_at_claim: TrunkCommitAtClaim::from(trunk),
                head_snapshot,
                phase_start_head: ProtectedPhaseStartHead::from(head),
                worktree_root: actor.worktree_root.clone(),
                worktree_administrative_locator: actor.worktree_administrative_locator.clone(),
                authorization,
            },
        )?;
        Ok(ReservationClaimFixture {
            reservation_id,
            scopes,
        })
    }

    pub(super) fn append_as(
        &self,
        actor: &TestActor,
        operation: JournalOperation,
    ) -> FixtureResult<()> {
        let outcome: LedgerTransactionOutcome<Infallible> =
            self.ledger
                .transact(actor.worktree_id, actor.coordination_run_id, |_| {
                    TransactionValidation::Append(Box::new(operation))
                })?;
        match outcome {
            LedgerTransactionOutcome::Appended { .. } => Ok(()),
            LedgerTransactionOutcome::Rejected(infallible) => match infallible {},
        }
    }

    pub(super) fn checkpoint(
        &self,
        actor: &TestActor,
        reservation_id: ReservationId,
        protected_tip: GitObjectId,
        trunk_snapshot: GitObjectId,
    ) -> FixtureResult<()> {
        self.append_as(
            actor,
            JournalOperation::Checkpoint {
                reservation_id,
                protected_tip: ProtectedReservationTip::from(protected_tip),
                trunk_snapshot,
            },
        )
    }

    pub(super) fn record_evidence(
        &self,
        actor: &TestActor,
        reservation_id: ReservationId,
        status: IntegrationEvidenceStatus,
    ) -> FixtureResult<()> {
        let edit_blocking_status = status.edit_blocking_status();
        self.append_as(
            actor,
            JournalOperation::EvidenceRevalidated {
                reservation_id,
                status,
                edit_blocking_status,
            },
        )
    }

    pub(super) fn release(
        &self,
        actor: &TestActor,
        reservation_id: ReservationId,
        disposition: ReleaseDisposition,
    ) -> FixtureResult<()> {
        self.append_as(
            actor,
            JournalOperation::Release {
                reservation_id,
                disposition,
            },
        )
    }

    pub(super) fn amend_trunk(&self) -> FixtureResult<()> {
        git(
            self.repository.path(),
            &["commit", "--quiet", "--amend", "-m", "rewritten trunk"],
        )?;
        Ok(())
    }

    pub(super) fn model(&self) -> FixtureResult<BoardModel> {
        let report =
            match reconcile::reconcile(self.repository.path(), RecoveredBypassReporting::Report)? {
                Enrollment::Enrolled(report) => report,
                Enrollment::Unconfigured { .. } => {
                    return Err("initialized board fixture is not enrolled".into());
                },
            };
        Ok(BoardModel::build(self.repository.path(), &report)?)
    }
}

impl OrderedBoardFixture {
    pub(super) fn new() -> FixtureResult<Self> {
        let board = BoardFixture::new()?;
        let predecessor_actor = board.linked_actor("predecessor")?;
        let successor_actor = board.linked_actor("successor")?;
        let predecessor = board.claim(
            &predecessor_actor,
            "src/lib.rs",
            ConflictAuthorization::NoConflict,
        )?;
        let successor = board.claim(
            &successor_actor,
            "src/lib.rs",
            conflict_authorization(OverlapAnswerFixture::Sequence, &predecessor)?,
        )?;
        Ok(Self {
            board,
            predecessor_actor,
            predecessor,
            successor,
        })
    }

    pub(super) fn model(&self) -> FixtureResult<BoardModel> { self.board.model() }

    pub(super) fn commit_predecessor(&self) -> FixtureResult<GitObjectId> {
        fs::write(
            self.predecessor_actor.root.join("src/lib.rs"),
            "pub fn predecessor() {}\n",
        )?;
        git(&self.predecessor_actor.root, &["add", "src/lib.rs"])?;
        git(
            &self.predecessor_actor.root,
            &["commit", "--quiet", "-m", "predecessor work"],
        )?;
        git_object_id(&self.predecessor_actor.root, "HEAD")
    }

    pub(super) fn merge_predecessor(&self) -> FixtureResult<()> {
        git(
            self.board.repository.path(),
            &["merge", "--quiet", "--ff-only", "predecessor"],
        )?;
        Ok(())
    }
}

pub(super) fn answered_board(answer: OverlapAnswerFixture) -> FixtureResult<AnsweredBoardFixture> {
    let fixture = BoardFixture::new()?;
    let actor = fixture.main_actor();
    let blocker = fixture.claim(&actor, "shared.rs", ConflictAuthorization::NoConflict)?;
    let requester = fixture.claim(
        &actor,
        "shared.rs",
        conflict_authorization(answer, &blocker)?,
    )?;
    Ok(AnsweredBoardFixture {
        model:        fixture.model()?,
        blocker_id:   blocker.reservation_id,
        requester_id: requester.reservation_id,
    })
}

fn conflict_authorization(
    answer: OverlapAnswerFixture,
    blocker: &ReservationClaimFixture,
) -> FixtureResult<ConflictAuthorization> {
    let overlap = AuthorizedOverlap {
        reservation_id: blocker.reservation_id,
        scope_revision: OverlapScopeRevision::from(&blocker.scopes),
        scopes:         AuthorizedOverlapScopeSet::from(blocker.scopes.clone()),
    };
    let overlaps = AuthorizedOverlapSet::from(overlap);
    Ok(match answer {
        OverlapAnswerFixture::Sequence => ConflictAuthorization::Sequence {
            overlaps,
            blocker: blocker.reservation_id,
            direction: OrderingDirection::HolderBeforeRequester,
            edge_id: EdgeId::new(),
            reason: "holder must integrate first".parse::<OverlapAuthorizationReason>()?,
        },
        OverlapAnswerFixture::Defer => ConflictAuthorization::Defer {
            overlaps,
            blocker: blocker.reservation_id,
            reason: "integration order is deferred".parse::<OverlapAuthorizationReason>()?,
        },
        OverlapAnswerFixture::Override => ConflictAuthorization::Override {
            overlaps,
            blocker: blocker.reservation_id,
            reason: "overlapping edits are accepted".parse::<OverlapAuthorizationReason>()?,
        },
    })
}

fn file_scope_set(path: &str) -> FixtureResult<ReservationScopeSet> {
    Ok(ReservationScopeSet::try_from(vec![ReservationScope {
        path: path.parse::<ReservationScopePath>()?,
        kind: ScopeKind::File,
    }])?)
}

pub(super) fn board_reservation_snapshot(
    model: &BoardModel,
    reservation_id: ReservationId,
) -> FixtureResult<&BoardReservationSnapshot> {
    model
        .ready_now
        .entries
        .iter()
        .map(|entry| &entry.reservation)
        .chain(model.unconstrained_reservations.entries.iter())
        .chain(model.resolved.entries.iter())
        .find(|row| row.reservation_id == reservation_id)
        .ok_or_else(|| io::Error::other("reservation row should exist").into())
}

fn git_object_id(repository_root: &Path, revision: &str) -> FixtureResult<GitObjectId> {
    Ok(git(repository_root, &["rev-parse", revision])?.parse::<GitObjectId>()?)
}

fn git(repository_root: &Path, arguments: &[&str]) -> FixtureResult<String> {
    let output = Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ))
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
