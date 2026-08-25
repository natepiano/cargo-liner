use std::convert::Infallible;
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;
use tempfile::tempdir;

use super::AnswerAcquisition;
use super::BoardAlert;
use super::BoardIntegrationEvidence;
use super::BoardModel;
use super::BypassAuditEntry;
use super::OrderingConsequence;
use super::OverrideConsequence;
use super::RecordedAnswer;
use super::ReservationRow;
use super::StaleReservationResolutionAction;
use super::SymmetricDeferralConsequence;
use super::WaitingAction;
use super::board_alerts;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapScopeSet;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::answer::OverlapAuthorizationReason;
use crate::answer::OverlapScopeRevision;
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
use crate::ledger::ReservationSnapshot;
use crate::ledger::ScopeKind;
use crate::ledger::TransactionValidation;
use crate::ledger::TrunkCommitAtClaim;
use crate::ledger::WorktreeAdministrativeLocator;
use crate::ledger::WorktreeContext;
use crate::reconcile;
use crate::reconcile::RecoveredBypassReporting;
use crate::reservation::AbandonmentReason;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::OrphanRetirementReason;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationFreshness;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RewrittenIntegrationTrunkCommit;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const GIT_BINARY: &str = "git";
const PENDING_BYPASS_NAME: &str =
    "cargo-berth-pending-bypass-01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a99.json";
const UNKNOWN_OBJECT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

type FixtureResult<T> = Result<T, Box<dyn Error>>;

#[derive(Clone)]
struct TestActor {
    root:                            PathBuf,
    worktree_id:                     WorktreeId,
    coordination_run_id:             CoordinationRunId,
    worktree_root:                   CanonicalWorktreeRoot,
    worktree_administrative_locator: WorktreeAdministrativeLocator,
}

struct BoardFixture {
    repository:                TempDir,
    linked_worktree_directory: TempDir,
    ledger:                    Ledger,
    main_actor:                TestActor,
}

struct ReservationClaimFixture {
    reservation_id: ReservationId,
    scopes:         ReservationScopeSet,
}

#[derive(Clone, Copy)]
enum OverlapAnswerFixture {
    Sequence,
    Defer,
    Override,
}

struct AnsweredBoardFixture {
    model:        BoardModel,
    blocker_id:   ReservationId,
    requester_id: ReservationId,
}

struct OrderedBoardFixture {
    board:             BoardFixture,
    predecessor_actor: TestActor,
    predecessor:       ReservationClaimFixture,
    successor:         ReservationClaimFixture,
}

#[test]
fn deferring_reconciliation_leaves_recovery_for_one_reporting_board() -> FixtureResult<()> {
    let fixture = BoardFixture::new()?;
    let marker_path = fixture
        .repository
        .path()
        .join(".git")
        .join(PENDING_BYPASS_NAME);
    fs::write(
        &marker_path,
        r#"{"cause":{"kind":"environment_override","bypassed_merge":"model-recovery"},"occurrence_time":{"status":"unavailable"}}
"#,
    )?;

    let deferred =
        match reconcile::reconcile(fixture.repository.path(), RecoveredBypassReporting::Defer)? {
            crate::config::Enrollment::Enrolled(deferred) => deferred,
            crate::config::Enrollment::Unconfigured { .. } => {
                return Err("initialized board fixture is not enrolled".into());
            },
        };
    assert!(deferred.recovered_bypass_markers.is_empty());
    assert!(marker_path.exists());

    let recovered = fixture.model()?;
    assert_eq!(
        serde_json::to_value(&recovered.recovered_bypasses_this_invocation)?,
        serde_json::json!([PENDING_BYPASS_NAME])
    );
    assert!(matches!(
        recovered.bypass_audit.entries.as_slice(),
        [BypassAuditEntry::EnvironmentOverride { .. }]
    ));
    assert!(!marker_path.exists());

    let later_read = fixture.model()?;
    assert_eq!(
        serde_json::to_value(&later_read.recovered_bypasses_this_invocation)?,
        serde_json::json!([])
    );
    assert!(matches!(
        later_read.bypass_audit.entries.as_slice(),
        [BypassAuditEntry::EnvironmentOverride { .. }]
    ));
    assert!(!marker_path.exists());
    Ok(())
}

#[test]
fn stale_reservation_alert_names_the_renew_resolution() -> FixtureResult<()> {
    let fixture = BoardFixture::new()?;
    let actor = fixture.main_actor();
    let reservation = fixture.claim(&actor, "stale.rs", ConflictAuthorization::NoConflict)?;
    let model = fixture.model()?;
    let fresh_row = reservation_row(&model, reservation.reservation_id)?.clone();
    assert!(board_alerts(&[], std::slice::from_ref(&fresh_row), &[])?.is_empty());

    let mut stale_row = fresh_row;
    let ReservationFreshness::Fresh { last_activity_at } = stale_row.freshness.clone() else {
        return Err(io::Error::other("new reservation should be fresh").into());
    };
    stale_row.freshness = ReservationFreshness::Stale { last_activity_at };
    let alerts = board_alerts(&[], &[stale_row], &[])?;
    assert!(matches!(
        alerts.as_slice(),
        [BoardAlert::StaleReservation {
            reservation_id,
            resolution: StaleReservationResolutionAction::Renew {
                reservation_id: action_reservation_id,
            },
            ..
        }] if *reservation_id == reservation.reservation_id
            && *action_reservation_id == reservation.reservation_id
    ));
    Ok(())
}

#[test]
fn overlap_answers_preserve_typed_authorization_variants() -> FixtureResult<()> {
    let sequence = answered_board(OverlapAnswerFixture::Sequence)?;
    let sequence_answer = recorded_answer(&sequence.model, sequence.requester_id)?;
    let RecordedAnswer::Sequence {
        reservation_id,
        blocker,
        direction,
        exact_approved_scopes,
        authorization_reason,
        acquisition,
        consequence,
    } = sequence_answer
    else {
        return Err(
            io::Error::other("sequence fixture should produce a sequence audit row").into(),
        );
    };
    assert_eq!(*reservation_id, sequence.requester_id);
    assert_eq!(*blocker, sequence.blocker_id);
    assert_eq!(*direction, OrderingDirection::HolderBeforeRequester);
    assert_authorized_overlap(exact_approved_scopes, sequence.blocker_id);
    assert_eq!(
        authorization_reason.to_string(),
        "holder must integrate first"
    );
    assert!(matches!(acquisition, AnswerAcquisition::Claim));
    assert!(matches!(
        consequence,
        OrderingConsequence::Holding {
            action: WaitingAction::PredecessorCheckpoint { .. },
        }
    ));

    let defer = answered_board(OverlapAnswerFixture::Defer)?;
    let defer_answer = recorded_answer(&defer.model, defer.requester_id)?;
    let RecordedAnswer::Defer {
        reservation_id,
        blocker,
        exact_approved_scopes,
        authorization_reason,
        acquisition,
        consequence,
    } = defer_answer
    else {
        return Err(io::Error::other("defer fixture should produce a defer audit row").into());
    };
    assert_eq!(*reservation_id, defer.requester_id);
    assert_eq!(*blocker, defer.blocker_id);
    assert_authorized_overlap(exact_approved_scopes, defer.blocker_id);
    assert_eq!(
        authorization_reason.to_string(),
        "integration order is deferred"
    );
    assert!(matches!(acquisition, AnswerAcquisition::Claim));
    assert_eq!(
        *consequence,
        SymmetricDeferralConsequence::BothIntegrationsHeldUntilSequence
    );

    let override_fixture = answered_board(OverlapAnswerFixture::Override)?;
    let override_answer = recorded_answer(&override_fixture.model, override_fixture.requester_id)?;
    let RecordedAnswer::Override {
        reservation_id,
        blocker,
        exact_approved_scopes,
        authorization_reason,
        acquisition,
        consequence,
    } = override_answer
    else {
        return Err(
            io::Error::other("override fixture should produce an override audit row").into(),
        );
    };
    assert_eq!(*reservation_id, override_fixture.requester_id);
    assert_eq!(*blocker, override_fixture.blocker_id);
    assert_authorized_overlap(exact_approved_scopes, override_fixture.blocker_id);
    assert_eq!(
        authorization_reason.to_string(),
        "overlapping edits are accepted"
    );
    assert!(matches!(acquisition, AnswerAcquisition::Claim));
    assert_eq!(
        *consequence,
        OverrideConsequence::EditingAuthorizedWithoutIntegrationOrder
    );
    Ok(())
}

#[test]
fn release_dispositions_remain_typed_in_resolved_rows() -> FixtureResult<()> {
    let fixture = BoardFixture::new()?;
    let actor = fixture.main_actor();
    let trunk = fixture.trunk()?;

    let integrated = fixture.claim(&actor, "integrated.rs", ConflictAuthorization::NoConflict)?;
    fixture.checkpoint(
        &actor,
        integrated.reservation_id,
        trunk.clone(),
        trunk.clone(),
    )?;
    fixture.record_evidence(
        &actor,
        integrated.reservation_id,
        IntegrationEvidenceStatus::Integrated {
            trunk_oid: trunk.clone(),
        },
    )?;
    fixture.release(
        &actor,
        integrated.reservation_id,
        ReleaseDisposition::Integrated,
    )?;

    let rewritten = fixture.claim(&actor, "rewritten.rs", ConflictAuthorization::NoConflict)?;
    fixture.checkpoint(
        &actor,
        rewritten.reservation_id,
        trunk.clone(),
        trunk.clone(),
    )?;
    fixture.record_evidence(
        &actor,
        rewritten.reservation_id,
        IntegrationEvidenceStatus::Integrated {
            trunk_oid: trunk.clone(),
        },
    )?;
    fixture.release(
        &actor,
        rewritten.reservation_id,
        ReleaseDisposition::RewrittenIntegration(RewrittenIntegrationTrunkCommit::from(trunk)),
    )?;

    let abandoned = fixture.claim(&actor, "abandoned.rs", ConflictAuthorization::NoConflict)?;
    fixture.release(
        &actor,
        abandoned.reservation_id,
        ReleaseDisposition::Abandoned("discarded deliberately".parse::<AbandonmentReason>()?),
    )?;

    let retired = fixture.claim(&actor, "retired.rs", ConflictAuthorization::NoConflict)?;
    fixture.release(
        &actor,
        retired.reservation_id,
        ReleaseDisposition::RetiredOrphan(
            "retired after review".parse::<OrphanRetirementReason>()?,
        ),
    )?;

    let model = fixture.model()?;
    assert!(matches!(
        &reservation_row(&model, integrated.reservation_id)?.lifecycle,
        ReservationLifecycle::Released {
            disposition: ReleaseDisposition::Integrated,
        }
    ));
    assert!(matches!(
        &reservation_row(&model, rewritten.reservation_id)?.lifecycle,
        ReservationLifecycle::Released {
            disposition: ReleaseDisposition::RewrittenIntegration(_),
        }
    ));
    assert!(matches!(
        &reservation_row(&model, abandoned.reservation_id)?.lifecycle,
        ReservationLifecycle::Released {
            disposition: ReleaseDisposition::Abandoned(_),
        }
    ));
    assert!(matches!(
        &reservation_row(&model, retired.reservation_id)?.lifecycle,
        ReservationLifecycle::Released {
            disposition: ReleaseDisposition::RetiredOrphan(_),
        }
    ));
    Ok(())
}

#[test]
fn waiting_reasons_pair_typed_evidence_with_actions() -> FixtureResult<()> {
    assert_checkpoint_not_integrated_and_incorporation_actions()?;
    assert_trunk_rewritten_action()?;
    assert_object_unknown_action()?;
    Ok(())
}

fn assert_checkpoint_not_integrated_and_incorporation_actions() -> FixtureResult<()> {
    let initial = OrderedBoardFixture::new()?;
    let initial_model = initial.model()?;
    assert_waiting_endpoints(&initial_model, &initial);
    let WaitingAction::PredecessorCheckpoint { instruction } = waiting_action(&initial_model)?
    else {
        return Err(io::Error::other("active predecessor should require a checkpoint").into());
    };
    assert!(instruction.contains("nobody can act yet"));

    let protected_tip = initial.commit_predecessor()?;
    let checkpoint_trunk = initial.board.trunk()?;
    initial.board.checkpoint(
        &initial.predecessor_actor,
        initial.predecessor.reservation_id,
        protected_tip,
        checkpoint_trunk,
    )?;
    let not_integrated = initial.model()?;
    let WaitingAction::PredecessorNotIntegrated { instruction } = waiting_action(&not_integrated)?
    else {
        return Err(io::Error::other("unmerged predecessor should be not integrated").into());
    };
    assert!(instruction.contains("reach trunk"));
    assert!(matches!(
        &reservation_row(&not_integrated, initial.predecessor.reservation_id)?.integration_evidence,
        BoardIntegrationEvidence::Current {
            status: IntegrationEvidenceStatus::NotIntegrated,
        }
    ));

    initial.merge_predecessor()?;
    let integrated = initial.model()?;
    let WaitingAction::SuccessorMustIncorporatePredecessor { instruction } =
        waiting_action(&integrated)?
    else {
        return Err(io::Error::other(
            "integrated predecessor should require the successor's own rebase",
        )
        .into());
    };
    assert!(instruction.contains("reader's own rebase"));
    assert!(matches!(
        &reservation_row(&integrated, initial.predecessor.reservation_id)?.integration_evidence,
        BoardIntegrationEvidence::Current {
            status: IntegrationEvidenceStatus::Integrated { .. },
        }
    ));
    Ok(())
}

fn assert_trunk_rewritten_action() -> FixtureResult<()> {
    let rewritten = OrderedBoardFixture::new()?;
    let rewritten_tip = rewritten.board.trunk()?;
    rewritten.board.checkpoint(
        &rewritten.predecessor_actor,
        rewritten.predecessor.reservation_id,
        rewritten_tip.clone(),
        rewritten_tip,
    )?;
    rewritten.board.amend_trunk()?;
    let rewritten_model = rewritten.model()?;
    let WaitingAction::TrunkEvidenceRewritten {
        instruction,
        resolve_flag,
    } = waiting_action(&rewritten_model)?
    else {
        return Err(io::Error::other("rewritten trunk should require new evidence").into());
    };
    assert!(instruction.contains("trunk rewrite"));
    assert_eq!(resolve_flag, "resolve --integrated-as <trunk-oid>");
    assert!(matches!(
        &reservation_row(&rewritten_model, rewritten.predecessor.reservation_id)?
            .integration_evidence,
        BoardIntegrationEvidence::Current {
            status: IntegrationEvidenceStatus::TrunkRewritten,
        }
    ));
    Ok(())
}

fn assert_object_unknown_action() -> FixtureResult<()> {
    let unknown = OrderedBoardFixture::new()?;
    let known_tip = unknown.board.trunk()?;
    unknown.board.checkpoint(
        &unknown.predecessor_actor,
        unknown.predecessor.reservation_id,
        known_tip.clone(),
        known_tip.clone(),
    )?;
    unknown.model()?;
    let unknown_tip = UNKNOWN_OBJECT_ID.parse::<GitObjectId>()?;
    unknown.board.append_as(
        &unknown.predecessor_actor,
        JournalOperation::Resnapshot {
            reservation_id: unknown.predecessor.reservation_id,
            snapshot:       ReservationSnapshot::Outstanding {
                protected_tip: ProtectedReservationTip::from(unknown_tip),
                trunk_oid:     known_tip,
            },
        },
    )?;
    let unknown_model = unknown.model()?;
    let WaitingAction::PredecessorObjectUnknown { instruction } = waiting_action(&unknown_model)?
    else {
        return Err(io::Error::other("missing predecessor object should be reported").into());
    };
    assert!(instruction.contains("does not resolve"));
    assert!(matches!(
        &reservation_row(&unknown_model, unknown.predecessor.reservation_id)?.integration_evidence,
        BoardIntegrationEvidence::Current {
            status: IntegrationEvidenceStatus::ObjectUnknown,
        }
    ));
    Ok(())
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
    fn new() -> FixtureResult<Self> {
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

    fn main_actor(&self) -> TestActor { self.main_actor.clone() }

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

    fn trunk(&self) -> FixtureResult<GitObjectId> { git_object_id(self.repository.path(), "main") }

    fn claim(
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

    fn append_as(&self, actor: &TestActor, operation: JournalOperation) -> FixtureResult<()> {
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

    fn checkpoint(
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

    fn record_evidence(
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

    fn release(
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

    fn amend_trunk(&self) -> FixtureResult<()> {
        git(
            self.repository.path(),
            &["commit", "--quiet", "--amend", "-m", "rewritten trunk"],
        )?;
        Ok(())
    }

    fn model(&self) -> FixtureResult<BoardModel> {
        let report =
            match reconcile::reconcile(self.repository.path(), RecoveredBypassReporting::Report)? {
                crate::config::Enrollment::Enrolled(report) => report,
                crate::config::Enrollment::Unconfigured { .. } => {
                    return Err("initialized board fixture is not enrolled".into());
                },
            };
        Ok(BoardModel::build(self.repository.path(), &report)?)
    }
}

impl OrderedBoardFixture {
    fn new() -> FixtureResult<Self> {
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

    fn model(&self) -> FixtureResult<BoardModel> { self.board.model() }

    fn commit_predecessor(&self) -> FixtureResult<GitObjectId> {
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

    fn merge_predecessor(&self) -> FixtureResult<()> {
        git(
            self.board.repository.path(),
            &["merge", "--quiet", "--ff-only", "predecessor"],
        )?;
        Ok(())
    }
}

fn answered_board(answer: OverlapAnswerFixture) -> FixtureResult<AnsweredBoardFixture> {
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

fn recorded_answer(
    model: &BoardModel,
    reservation_id: ReservationId,
) -> FixtureResult<&RecordedAnswer> {
    model
        .recorded_overlap_answers
        .entries
        .iter()
        .find(|answer| match answer {
            RecordedAnswer::Sequence {
                reservation_id: candidate,
                ..
            }
            | RecordedAnswer::Defer {
                reservation_id: candidate,
                ..
            }
            | RecordedAnswer::Override {
                reservation_id: candidate,
                ..
            }
            | RecordedAnswer::ExistingAnswersCoverEveryOverlap {
                reservation_id: candidate,
                ..
            }
            | RecordedAnswer::WidenWithoutForeignOverlap {
                reservation_id: candidate,
                ..
            } => *candidate == reservation_id,
            RecordedAnswer::OrderingCreatedFromDeferral { .. } => false,
        })
        .ok_or_else(|| io::Error::other("recorded answer should exist").into())
}

fn assert_authorized_overlap(overlaps: &AuthorizedOverlapSet, blocker_id: ReservationId) {
    assert_eq!(overlaps.as_slice().len(), 1);
    let overlap = &overlaps.as_slice()[0];
    assert_eq!(overlap.reservation_id, blocker_id);
    assert_eq!(overlap.scopes.as_slice().len(), 1);
    assert_eq!(overlap.scopes.as_slice()[0].path.to_string(), "shared.rs");
    assert_eq!(overlap.scopes.as_slice()[0].kind, ScopeKind::File);
}

fn reservation_row(
    model: &BoardModel,
    reservation_id: ReservationId,
) -> FixtureResult<&ReservationRow> {
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

fn waiting_action(model: &BoardModel) -> FixtureResult<&WaitingAction> {
    if model.waiting.entries.len() != 1 {
        return Err(io::Error::other("fixture should produce one waiting constraint").into());
    }
    Ok(&model.waiting.entries[0].action)
}

fn assert_waiting_endpoints(model: &BoardModel, fixture: &OrderedBoardFixture) {
    assert_eq!(model.waiting.entries.len(), 1);
    assert_eq!(
        model.waiting.entries[0].predecessor,
        fixture.predecessor.reservation_id
    );
    assert_eq!(
        model.waiting.entries[0].successor,
        fixture.successor.reservation_id
    );
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
