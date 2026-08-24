//! The frozen JSON output contract for `cargo-berth`.
//!
//! Every JSON response retains the original six-field envelope and adds one
//! typed `payload` field. Consumers can continue reading the original fields,
//! while newer consumers use `payload` instead of scraping `message`.

use serde::Deserialize;
use serde::Serialize;

use crate::alert::Alert;
use crate::answer::OverlapEscalationPayload;
use crate::answer::PermissiveOverlapAnswer;
use crate::config::InitializationState;
use crate::exit::BerthExit;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::ClaimSource;
use crate::ledger::LedgerInitialization;
use crate::ledger::OrderingDirection;
use crate::ledger::ReservationPurpose;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationConflict;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

const INITIALIZED_MESSAGE: &str = "Initialized the cargo-berth ledger.";
const PROJECTION_REPAIRED_MESSAGE: &str =
    "Rebuilt reservations.json from journal truth without changing the journal.";
const UNIMPLEMENTED_MESSAGE: &str = "The reservation engine is not implemented.";

/// One response from a `cargo-berth` verb.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OutputEnvelope {
    /// The verb that produced this response.
    pub(crate) verb:         CommandVerb,
    /// The response's lifecycle status.
    pub(crate) status:       OutputStatus,
    /// The process exit status for this response.
    pub(crate) exit_code:    BerthExit,
    /// Reservations relevant to this response.
    pub(crate) reservations: Vec<ReservationId>,
    /// Reservations that block this response.
    pub(crate) blocked_by:   Vec<ReservationId>,
    /// A human-readable explanation of this response.
    pub(crate) message:      String,
    /// The verb-keyed facts consumers need without parsing prose.
    pub(crate) payload:      OutputPayload,
}

/// A verb named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommandVerb {
    /// Initialize the shared ledger.
    Init,
    /// Show the reservation board.
    Board,
    /// Check a proposed path footprint.
    Check,
    /// Claim paths for a reservation.
    Claim,
    /// Release a reservation at a checkpoint.
    Release,
    /// Record an ordering relationship.
    Sequence,
    /// Integrate a reservation into trunk.
    Integrate,
    /// Resolve a stuck reservation after inspecting its condition.
    Resolve,
    /// Renew a reservation's explicit activity record.
    Renew,
}

/// The status named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputStatus {
    /// The verb parsed, but no engine stands behind it yet.
    Unimplemented,
    /// Initialization created or verified the durable coordination resources.
    Initialized,
    /// Explicit repair rebuilt only the disposable journal projection.
    ProjectionRepaired,
    /// The journal or its projection could not be safely read or published.
    LedgerUnreadable,
    /// An overlap-free edit check may proceed.
    Clear,
    /// A new reservation was appended and published.
    Claimed,
    /// One or more foreign reservations overlap the requested paths.
    BlockedByOverlap,
    /// A permissive overlap answer needs a matching reviewed proposal.
    NeedsUserAuthorization,
    /// The caller can correct the request and retry without repairing the ledger.
    InvalidInput,
    /// Another mutation retained the ledger lock through the retry window.
    Contention,
    /// The reservation now has a protected checkpoint awaiting integration.
    Outstanding,
    /// Current trunk contains the reservation's integration evidence.
    Integrated,
    /// Current trunk no longer contains previously verified evidence.
    TrunkRewritten,
    /// Git could not resolve an object needed to verify integration.
    ObjectUnknown,
    /// A user-confirmed non-integration disposition ended the reservation.
    Released,
    /// A replacement worktree now owns surviving reservation work.
    Recovered,
    /// A still-live reservation recorded recent activity.
    Renewed,
}

/// Structured facts and additive alerts returned inside the typed payload field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OutputPayload {
    /// The verb-keyed result whose serialized `kind` and `data` layout is stable.
    #[serde(flatten)]
    facts:  OutputFacts,
    /// Durable coordination alerts relevant to this response.
    #[serde(default)]
    alerts: Vec<Alert>,
}

/// Structured facts that correspond to the response's verb.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum OutputFacts {
    /// The operation failed before it could establish any durable facts.
    NoFacts,
    /// Facts returned by `init`.
    Init(InitializationPayload),
    /// Facts returned by `init --repair-projection`.
    ProjectionRepair(ProjectionRepairPayload),
    /// Placeholder facts for an unimplemented board query.
    Board(PendingPayload),
    /// Facts returned by `check`.
    Check(CheckPayload),
    /// Facts returned by `claim`.
    Claim(ClaimPayload),
    /// Facts returned by `release`.
    Release(ReleasePayload),
    /// Placeholder facts for an unimplemented sequencing operation.
    Sequence(PendingPayload),
    /// Placeholder facts for an unimplemented integration.
    Integrate(PendingPayload),
    /// Facts returned by a recovery decision.
    Resolve(ResolvePayload),
    /// Facts returned by a renewal.
    Renew(RenewPayload),
}

/// The resources an `init` call created or left intact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct InitializationPayload {
    /// Whether initialization created the journal or found an existing one.
    pub(crate) ledger:        InitializationResource,
    /// Whether initialization created the config or left an existing file intact.
    pub(crate) configuration: InitializationResource,
}

/// The explicit guarantee reported after rebuilding the disposable projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectionRepairPayload {
    /// The only file this operation rebuilt.
    pub(crate) projection: RepairedProjection,
    /// The journal mutation guarantee of explicit projection repair.
    pub(crate) journal:    ProjectionRepairJournalEffect,
}

/// The disposable projection rebuilt by explicit repair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepairedProjection {
    /// `reservations.json` was derived again from complete journal facts.
    ReservationsJsonRebuilt,
}

/// Whether explicit projection repair changed journal truth.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectionRepairJournalEffect {
    /// `journal.ndjson` remained byte-identical.
    Unchanged,
}

/// The initialization outcome for one durable resource.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InitializationResource {
    /// This initialization call created the resource.
    Created,
    /// This initialization call retained an existing resource unchanged.
    Existing,
}

/// A deliberately empty typed placeholder for a verb whose engine arrives later.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingPayload {}

/// Typed outcomes returned by `resolve`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ResolvePayload {
    /// Surviving work moved to a replacement worktree identity.
    Recovered {
        /// The reservation whose holder changed.
        reservation_id: ReservationId,
        /// The opaque identity of the replacement worktree.
        worktree_id:    WorktreeId,
    },
    /// A user-confirmed terminal disposition resolved the reservation.
    Released {
        /// The reservation that received the disposition.
        reservation_id: ReservationId,
        /// The recorded disposition or replacement disposition.
        disposition:    ReleaseDisposition,
    },
}

/// Typed facts returned by `renew`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct RenewPayload {
    /// The reservation whose activity timestamp advanced.
    pub(crate) reservation_id: ReservationId,
}

/// Typed outcomes returned by `claim`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ClaimPayload {
    /// A reservation was appended with this minimal antichain.
    Claimed {
        /// The newly minted reservation identity.
        reservation_id:      ReservationId,
        /// The coordination run that owns the appended reservation.
        coordination_run_id: CoordinationRunId,
        /// The exact durable footprint.
        scopes:              ReservationScopeSet,
        /// Whether the worktree marker records `coordination_run_id`.
        marker_publication:  CoordinationRunMarkerPublication,
    },
    /// Foreign holders prevented the append.
    Blocked {
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
    /// A permissive answer was proposed but has not supplied the current exact token.
    NeedsUserAuthorization {
        /// The conflicts, proposed answer, reason, consequence, and proposal token.
        #[serde(flatten)]
        escalation: Box<OverlapEscalationPayload>,
    },
}

/// Whether the successful claim also published its worktree run marker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CoordinationRunMarkerPublication {
    /// The marker now identifies the run that owns the appended claim.
    Published,
    /// The claim is durable, but the marker could not be updated.
    Unavailable {
        /// The marker publication failure.
        diagnostic: String,
    },
}

/// Typed outcomes returned by `check`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CheckPayload {
    /// No foreign live reservation overlaps the requested paths.
    Clear {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes: ReservationScopeSet,
    },
    /// Foreign holders block one or more requested paths.
    Blocked {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes:    ReservationScopeSet,
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
}

/// Typed state transitions and evidence results returned by `release`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReleasePayload {
    /// An active reservation recorded its first protected checkpoint.
    Checkpointed {
        /// The reservation that changed state.
        reservation_id: ReservationId,
        /// The fixed commit retained for integration checks.
        protected_tip:  ProtectedReservationTip,
        /// The trunk commit observed at checkpoint.
        trunk_oid:      GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A rebased outstanding reservation replaced its protected checkpoint.
    Resnapshotted {
        /// The reservation that changed state.
        reservation_id: ReservationId,
        /// The replacement fixed commit.
        protected_tip:  ProtectedReservationTip,
        /// The trunk commit observed with the replacement.
        trunk_oid:      GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A point-in-time git result was appended for hook-safe replay.
    EvidenceRevalidated {
        /// The reservation whose evidence was checked.
        reservation_id: ReservationId,
        /// What current trunk proves.
        evidence:       IntegrationEvidenceStatus,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A verified or user-confirmed disposition was appended.
    Released {
        /// The reservation that received the disposition.
        reservation_id: ReservationId,
        /// The retained terminal disposition.
        disposition:    ReleaseDisposition,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
}

impl ReleasePayload {
    const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::Checkpointed { reservation_id, .. }
            | Self::Resnapshotted { reservation_id, .. }
            | Self::EvidenceRevalidated { reservation_id, .. }
            | Self::Released { reservation_id, .. } => *reservation_id,
        }
    }

    const fn output_status(&self) -> OutputStatus {
        match self {
            Self::Checkpointed { .. } | Self::Resnapshotted { .. } => OutputStatus::Outstanding,
            Self::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::Integrated { .. } => OutputStatus::Integrated,
                IntegrationEvidenceStatus::NotIntegrated => OutputStatus::Outstanding,
                IntegrationEvidenceStatus::TrunkRewritten => OutputStatus::TrunkRewritten,
                IntegrationEvidenceStatus::ObjectUnknown => OutputStatus::ObjectUnknown,
            },
            Self::Released { disposition, .. } => match disposition {
                ReleaseDisposition::Integrated | ReleaseDisposition::RewrittenIntegration(_) => {
                    OutputStatus::Integrated
                },
                ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => {
                    OutputStatus::Released
                },
            },
        }
    }
}

/// The ordinary-release decision for the worktree coordination-run marker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CoordinationRunMarkerRetirement {
    /// The marker still named this run and was removed.
    Removed,
    /// No marker existed when release checked it.
    AlreadyAbsent,
    /// Another active reservation from this run still needs the marker.
    PreservedForActiveReservation,
    /// A newer run owns the marker.
    PreservedDifferentRun,
    /// The stateful check ran outside the reservation's holder worktree.
    PreservedDifferentWorktree,
    /// A malformed marker remains for phase-5 reconciliation.
    PreservedMalformed,
    /// The release fact is durable, but marker access failed.
    Unavailable {
        /// The marker filesystem diagnostic.
        diagnostic: String,
    },
}

impl OutputEnvelope {
    /// Build the response for a verb that has no engine behind it yet.
    pub(crate) fn unimplemented(command_verb: CommandVerb) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::Unimplemented,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      UNIMPLEMENTED_MESSAGE.to_owned(),
            payload:      OutputPayload::pending(command_verb),
        }
    }

    /// Build the successful response for completed initialization.
    pub(crate) fn initialized(initialization: LedgerInitialization) -> Self {
        Self {
            verb:         CommandVerb::Init,
            status:       OutputStatus::Initialized,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      INITIALIZED_MESSAGE.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::Init(InitializationPayload {
                ledger:        initialization.ledger.into(),
                configuration: initialization.configuration.into(),
            })),
        }
    }

    /// Build the successful response for an explicit projection-only repair.
    pub(crate) fn projection_repaired() -> Self {
        Self {
            verb:         CommandVerb::Init,
            status:       OutputStatus::ProjectionRepaired,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      PROJECTION_REPAIRED_MESSAGE.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::ProjectionRepair(
                ProjectionRepairPayload {
                    projection: RepairedProjection::ReservationsJsonRebuilt,
                    journal:    ProjectionRepairJournalEffect::Unchanged,
                },
            )),
        }
    }

    /// Build a ledger-unreadable response without adding a new process outcome.
    pub(crate) fn ledger_unreadable(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::LedgerUnreadable,
            exit_code:    BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!("The reservation ledger could not be read: {diagnostic}"),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build the successful result for one appended claim.
    pub(crate) fn claimed(
        reservation_id: ReservationId,
        coordination_run_id: CoordinationRunId,
        scopes: ReservationScopeSet,
        marker_publication: CoordinationRunMarkerPublication,
    ) -> Self {
        let scope_count = scopes.as_slice().len();
        let message = match &marker_publication {
            CoordinationRunMarkerPublication::Published => {
                format!("Claimed {scope_count} reservation scope(s) as {reservation_id}.")
            },
            CoordinationRunMarkerPublication::Unavailable { diagnostic } => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the coordination-run marker could not be published: {diagnostic}. Restore coordination run {coordination_run_id} through the process environment before subsequent commands."
            ),
        };
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::Claimed,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Claim(ClaimPayload::Claimed {
                reservation_id,
                coordination_run_id,
                scopes,
                marker_publication,
            })),
        }
    }

    /// Build a claim rejection that names every foreign holder.
    pub(crate) fn blocked_claim(conflicts: Vec<ReservationConflict>) -> Self {
        let blocked_by = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::BlockedByOverlap,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by,
            message: blocked_message(&conflicts),
            payload: OutputPayload::from_facts(OutputFacts::Claim(ClaimPayload::Blocked {
                conflicts,
            })),
        }
    }

    /// Build a claim response that requires a second invocation with the current token.
    pub(crate) fn claim_authorization_required(escalation: OverlapEscalationPayload) -> Self {
        let blocked_by = escalation
            .conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        let mut message = format!(
            "User authorization is required before this overlap can be recorded: {}. Review every holder, shared scope, plan, phase, direction, and reason in the payload, then rerun this claim with --proposal '{}'.",
            escalation.consequence, escalation.proposal_token
        );
        let direction = overlap_direction_description(&escalation.answer);
        let holder_material = escalation
            .conflicts
            .iter()
            .map(|conflict| {
                let shared_scopes = conflict
                    .overlapping_scopes
                    .as_slice()
                    .iter()
                    .map(|scope| {
                        let kind = match scope.kind {
                            ScopeKind::File => "file",
                            ScopeKind::Tree => "tree",
                        };
                        format!("{kind}:{}", scope.path)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Holder {}: {}; shared scopes: {}; direction: {}; reason: {}; consequence: {}.",
                    conflict.reservation_id,
                    source_description(&conflict.source),
                    shared_scopes,
                    direction,
                    escalation.authorization_reason,
                    escalation.consequence,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !holder_material.is_empty() {
            message.push('\n');
            message.push_str(&holder_material);
        }
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::NeedsUserAuthorization,
            exit_code: BerthExit::NeedsUserAuthorization,
            reservations: Vec::new(),
            blocked_by,
            message,
            payload: OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::NeedsUserAuthorization {
                    escalation: Box::new(escalation),
                },
            )),
        }
    }

    /// Build a successful mutation-free edit check.
    pub(crate) fn clear_check(scopes: ReservationScopeSet) -> Self {
        Self {
            verb:         CommandVerb::Check,
            status:       OutputStatus::Clear,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      "No foreign reservation overlaps the requested paths.".to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::Check(CheckPayload::Clear {
                scopes,
            })),
        }
    }

    /// Build a blocked mutation-free edit check.
    pub(crate) fn blocked_check(
        scopes: ReservationScopeSet,
        conflicts: Vec<ReservationConflict>,
    ) -> Self {
        let blocked_by = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        Self {
            verb: CommandVerb::Check,
            status: OutputStatus::BlockedByOverlap,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by,
            message: blocked_message(&conflicts),
            payload: OutputPayload::from_facts(OutputFacts::Check(CheckPayload::Blocked {
                scopes,
                conflicts,
            })),
        }
    }

    /// Build a caller-correctable request rejection.
    pub(crate) fn invalid_input(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::InvalidInput,
            exit_code:    BerthExit::UsageError,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      diagnostic.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a bounded lock-contention result with retry guidance.
    pub(crate) fn contention(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::Contention,
            exit_code:    BerthExit::BlockedByContention,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      diagnostic.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a successful release lifecycle or evidence response.
    pub(crate) fn released(release_payload: ReleasePayload) -> Self {
        let reservation_id = release_payload.reservation_id();
        let status = release_payload.output_status();
        let message = match &release_payload {
            ReleasePayload::Checkpointed { protected_tip, .. } => format!(
                "Reservation {reservation_id} is outstanding at protected tip {protected_tip}."
            ),
            ReleasePayload::Resnapshotted { protected_tip, .. } => {
                format!("Reservation {reservation_id} now retains protected tip {protected_tip}.")
            },
            ReleasePayload::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::NotIntegrated => format!(
                    "Reservation {reservation_id} remains outstanding; its protected tip is not in trunk."
                ),
                IntegrationEvidenceStatus::Integrated { trunk_oid } => format!(
                    "Reservation {reservation_id} has integration evidence in trunk commit {trunk_oid}."
                ),
                IntegrationEvidenceStatus::TrunkRewritten => format!(
                    "Reservation {reservation_id} is blocking again because trunk no longer contains its verified evidence."
                ),
                IntegrationEvidenceStatus::ObjectUnknown => format!(
                    "Reservation {reservation_id} is blocking because git could not resolve its integration evidence."
                ),
            },
            ReleasePayload::Released { disposition, .. } => {
                format!("Reservation {reservation_id} recorded disposition {disposition:?}.")
            },
        };
        Self {
            verb: CommandVerb::Release,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Release(release_payload)),
        }
    }

    /// Build a successful recovery response.
    pub(crate) fn resolved(resolve_payload: ResolvePayload) -> Self {
        let (reservation_id, status, message) = match &resolve_payload {
            ResolvePayload::Recovered {
                reservation_id,
                worktree_id,
            } => (
                *reservation_id,
                OutputStatus::Recovered,
                format!("Reservation {reservation_id} is recovered in worktree {worktree_id}."),
            ),
            ResolvePayload::Released {
                reservation_id,
                disposition,
            } => (
                *reservation_id,
                match disposition {
                    ReleaseDisposition::Integrated
                    | ReleaseDisposition::RewrittenIntegration(_) => OutputStatus::Integrated,
                    ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => {
                        OutputStatus::Released
                    },
                },
                format!("Reservation {reservation_id} recorded disposition {disposition:?}."),
            ),
        };
        Self {
            verb: CommandVerb::Resolve,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Resolve(resolve_payload)),
        }
    }

    /// Build a successful activity-renewal response.
    pub(crate) fn renewed(reservation_id: ReservationId) -> Self {
        Self {
            verb:         CommandVerb::Renew,
            status:       OutputStatus::Renewed,
            exit_code:    BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by:   Vec::new(),
            message:      format!("Reservation {reservation_id} activity was renewed."),
            payload:      OutputPayload::from_facts(OutputFacts::Renew(RenewPayload {
                reservation_id,
            })),
        }
    }

    /// Attach alerts derived by the reconciliation that preceded this command.
    pub(crate) fn with_alerts(mut self, alerts: Vec<Alert>) -> Self {
        self.payload.alerts = alerts;
        self
    }

    /// Render the primary result followed by every durable alert as its own line.
    pub(crate) fn render_text(&self) -> String {
        let mut rendered = self.message.clone();
        for alert in &self.payload.alerts {
            rendered.push('\n');
            rendered.push_str(&alert.to_string());
        }
        rendered
    }
}

impl OutputPayload {
    const fn from_facts(facts: OutputFacts) -> Self {
        Self {
            facts,
            alerts: Vec::new(),
        }
    }

    const fn pending(command_verb: CommandVerb) -> Self {
        let pending = PendingPayload {};
        let facts = match command_verb {
            CommandVerb::Board => OutputFacts::Board(pending),
            CommandVerb::Init | CommandVerb::Check | CommandVerb::Claim | CommandVerb::Release => {
                OutputFacts::NoFacts
            },
            CommandVerb::Sequence => OutputFacts::Sequence(pending),
            CommandVerb::Integrate => OutputFacts::Integrate(pending),
            CommandVerb::Resolve | CommandVerb::Renew => OutputFacts::NoFacts,
        };
        Self::from_facts(facts)
    }
}

fn blocked_message(conflicts: &[ReservationConflict]) -> String {
    match conflicts {
        [] => {
            "A foreign reservation overlaps the requested paths; reduce the requested scopes or coordinate with the holder, then retry."
                .to_owned()
        },
        [conflict] => {
            format!(
                "Reservation {} on {} ({}, {}) holds overlapping paths for {}; reduce the requested scopes or coordinate with the holder, then retry.",
                conflict.reservation_id,
                conflict.holder_branch(),
                source_description(&conflict.source),
                purpose_description(&conflict.purpose),
                conflict.holder_run_id,
            )
        },
        [_, _, ..] => {
            let holder_count = conflicts.len();
            let holders = conflicts
                .iter()
                .map(|conflict| {
                    format!(
                        "reservation {} on {} ({}, {}) for coordination run {}",
                        conflict.reservation_id,
                        conflict.holder_branch(),
                        source_description(&conflict.source),
                        purpose_description(&conflict.purpose),
                        conflict.holder_run_id,
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{holder_count} reservations hold overlapping paths: {holders}; reduce the requested scopes or coordinate with the holders, then retry.",
            )
        },
    }
}

fn source_description(claim_source: &ClaimSource) -> String {
    match claim_source {
        ClaimSource::WorkPlan { plan, phase } => format!("plan {plan}, phase {phase}"),
        ClaimSource::Explicit => "explicit claim".to_owned(),
    }
}

fn purpose_description(reservation_purpose: &ReservationPurpose) -> String {
    match reservation_purpose {
        ReservationPurpose::Explained(explanation) => explanation.to_string(),
        ReservationPurpose::NotProvidedByCaller => "no reason provided by caller".to_owned(),
    }
}

fn overlap_direction_description(answer: &PermissiveOverlapAnswer) -> String {
    match answer {
        PermissiveOverlapAnswer::Sequence { blocker, direction } => match direction {
            OrderingDirection::RequesterBeforeHolder => {
                format!("requester before holder {blocker}")
            },
            OrderingDirection::HolderBeforeRequester => {
                format!("holder {blocker} before requester")
            },
        },
        PermissiveOverlapAnswer::Defer { blocker } => {
            format!("none declared; deferred with holder {blocker}")
        },
        PermissiveOverlapAnswer::Override { blocker } => {
            format!("none declared; overridden with holder {blocker}")
        },
    }
}

impl From<InitializationState> for InitializationResource {
    fn from(initialization_state: InitializationState) -> Self {
        match initialization_state {
            InitializationState::Created => Self::Created,
            InitializationState::Existing => Self::Existing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandVerb;
    use super::OutputEnvelope;
    use super::OutputStatus;

    #[test]
    fn envelope_round_trips_with_its_additive_payload_field() {
        let output_envelope = OutputEnvelope::unimplemented(CommandVerb::Board);
        let serialized_envelope = serde_json::to_string(&output_envelope);

        assert!(
            serialized_envelope
                .as_ref()
                .is_ok_and(|serialized_envelope| serialized_envelope.contains("\"payload\""))
        );
        assert!(
            serialized_envelope
                .and_then(
                    |serialized_envelope| serde_json::from_str::<OutputEnvelope>(
                        &serialized_envelope
                    )
                )
                .is_ok_and(|round_tripped| round_tripped == output_envelope)
        );
    }

    #[test]
    fn init_has_a_non_placeholder_status() {
        let output_envelope = OutputEnvelope::initialized(crate::ledger::LedgerInitialization {
            ledger:        crate::config::InitializationState::Created,
            configuration: crate::config::InitializationState::Existing,
        });

        assert_eq!(output_envelope.status, OutputStatus::Initialized);
        assert_eq!(output_envelope.exit_code, crate::exit::BerthExit::Clear);
    }

    #[test]
    fn failed_init_has_no_initialization_facts() {
        let output_envelope = OutputEnvelope::ledger_unreadable(CommandVerb::Init, "bad journal");

        assert_eq!(output_envelope.payload.facts, super::OutputFacts::NoFacts);
        assert!(
            serde_json::to_string(&output_envelope.payload).is_ok_and(|payload| !payload
                .contains("ledger")
                && !payload.contains("configuration"))
        );
    }
}
