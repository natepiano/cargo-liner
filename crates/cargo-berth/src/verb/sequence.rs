//! Locked conversion of a recorded deferral into one ordering edge.

use std::cell::RefCell;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::edge::EdgeDeclarationRejection;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingEdge;
use crate::edge::OrderingGraph;
use crate::edge::OrderingReason;
use crate::edge::PreparedOrderingEdge;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::EditAuthorization;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::output::SequenceRejectionKind;
use crate::reconcile;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;

/// A parsed request to order two reservations that already share a deferral.
pub(crate) struct SequenceRequest {
    /// The reservation whose protected work must be incorporated first.
    pub(crate) first:  ReservationId,
    /// The reservation held until it incorporates `first`.
    pub(crate) then:   ReservationId,
    /// Why the user selected this order.
    pub(crate) reason: OrderingReason,
}

/// Execute one stateful deferral resolution and attach preceding reconciliation alerts.
pub(crate) fn execute(sequence_request: &SequenceRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Sequence, &error.to_string());
        },
    };
    let reconciliation_report = match reconcile::reconcile_for_sequence(
        &invocation_directory,
        sequence_request.first,
        sequence_request.then,
    ) {
        Ok(reconciliation_report) => reconciliation_report,
        Err(error) => return error.into_output(CommandVerb::Sequence),
    };
    let output_envelope = match execute_sequence(sequence_request) {
        Ok(edge) => match edge.readiness(&reconciliation_report.repository_snapshot) {
            Ok(readiness) => OutputEnvelope::sequenced(edge, readiness),
            Err(error) => {
                OutputEnvelope::ledger_unreadable(CommandVerb::Sequence, &error.to_string())
            },
        },
        Err(SequenceError::Rejected(SequenceRejection::Declaration(rejection))) => {
            let kind = SequenceRejectionKind::from(rejection);
            OutputEnvelope::sequence_rejected(sequence_request.first, sequence_request.then, kind)
        },
        Err(SequenceError::Rejected(SequenceRejection::EdgeLimitReached(maximum))) => {
            OutputEnvelope::sequence_rejected(
                sequence_request.first,
                sequence_request.then,
                SequenceRejectionKind::OrderingEdgeLimitReached { maximum },
            )
        },
        Err(SequenceError::Rejected(SequenceRejection::InactiveMarkerRun(coordination_run_id))) => {
            OutputEnvelope::sequence_rejected(
                sequence_request.first,
                sequence_request.then,
                SequenceRejectionKind::InactiveMarkerRun {
                    coordination_run_id,
                },
            )
        },
        Err(SequenceError::Transaction(LedgerTransactionError::LockContention)) => {
            OutputEnvelope::contention(
                CommandVerb::Sequence,
                &LedgerTransactionError::LockContention.to_string(),
            )
        },
        Err(SequenceError::Transaction(LedgerTransactionError::CorrectableInput(error))) => {
            OutputEnvelope::invalid_input(CommandVerb::Sequence, &error.to_string())
        },
        Err(
            SequenceError::Ledger(error)
            | SequenceError::Transaction(LedgerTransactionError::LedgerUnreadable(error)),
        ) => OutputEnvelope::ledger_unreadable(CommandVerb::Sequence, &error.to_string()),
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Sequence, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_sequence(sequence_request: &SequenceRequest) -> Result<OrderingEdge, SequenceError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let worktree_identity = ledger::worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let run_validation =
        SequenceRunValidation::resolve(worktree_context.administrative_directory());
    let berth_config = BerthConfig::read(worktree_context.repository_root())?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let prepared_edge = RefCell::<PreparedEdgeState>::new(PreparedEdgeState::NotPrepared);
    let outcome = ledger.transact(
        worktree_identity.id,
        run_validation.coordination_run_id(),
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return TransactionValidation::Reject(SequenceRejection::ReservationReplay(
                        error,
                    ));
                },
            };
            if let Err(rejection) = run_validation.validate(&reservations) {
                return TransactionValidation::Reject(rejection);
            }
            let ordering_graph = match OrderingGraph::replay(state.events()) {
                Ok(ordering_graph) => ordering_graph,
                Err(error) => {
                    return TransactionValidation::Reject(SequenceRejection::EdgeReplay(error));
                },
            };
            let edge = match ordering_graph.prepare_deferred_edge(
                sequence_request.first,
                sequence_request.then,
                sequence_request.reason.clone(),
            ) {
                Ok(edge) => edge,
                Err(error) => {
                    return TransactionValidation::Reject(SequenceRejection::Declaration(error));
                },
            };
            if count_reaches_limit(
                ordering_graph.edge_count(),
                berth_config.maximum_ordering_edges,
            ) {
                return TransactionValidation::Reject(SequenceRejection::EdgeLimitReached(
                    berth_config.maximum_ordering_edges,
                ));
            }
            let operation = edge.operation();
            prepared_edge.replace(PreparedEdgeState::Prepared(edge));
            TransactionValidation::Append(Box::new(operation))
        },
    )?;
    match outcome {
        LedgerTransactionOutcome::Appended(event) => {
            let PreparedEdgeState::Prepared(edge) = prepared_edge.into_inner() else {
                return Err(SequenceError::MissingPreparedEdge);
            };
            Ok(edge.into_edge(event.event_id()))
        },
        LedgerTransactionOutcome::Rejected(rejection) => Err(SequenceError::Rejected(rejection)),
    }
}

enum PreparedEdgeState {
    NotPrepared,
    Prepared(PreparedOrderingEdge),
}

#[derive(Clone, Copy)]
enum SequenceRunValidation {
    Independent(CoordinationRunId),
    ActiveMarkerRequired {
        coordination_run_id: CoordinationRunId,
        worktree_id:         WorktreeId,
    },
}

impl SequenceRunValidation {
    fn resolve(administrative_directory: &Path) -> Self {
        match EditAuthorization::resolve(administrative_directory) {
            EditAuthorization::Environment(coordination_run_id) => {
                Self::Independent(coordination_run_id)
            },
            EditAuthorization::Marker {
                coordination_run_id,
                worktree_id,
            } => Self::ActiveMarkerRequired {
                coordination_run_id,
                worktree_id,
            },
            EditAuthorization::Unidentified => Self::Independent(CoordinationRunId::new()),
        }
    }

    const fn coordination_run_id(self) -> CoordinationRunId {
        match self {
            Self::Independent(coordination_run_id)
            | Self::ActiveMarkerRequired {
                coordination_run_id,
                ..
            } => coordination_run_id,
        }
    }

    fn validate(self, reservations: &RetainedReservationSet) -> Result<(), SequenceRejection> {
        let Self::ActiveMarkerRequired {
            coordination_run_id,
            worktree_id,
        } = self
        else {
            return Ok(());
        };
        if reservations.iter().any(|reservation| {
            reservation.actor().run == coordination_run_id
                && reservation.actor().worktree == worktree_id
                && matches!(reservation.lifecycle(), ReservationLifecycle::Active)
        }) {
            Ok(())
        } else {
            Err(SequenceRejection::InactiveMarkerRun(coordination_run_id))
        }
    }
}

fn count_reaches_limit(count: usize, maximum: u32) -> bool {
    u64::try_from(count).map_or(true, |count| count >= u64::from(maximum))
}

#[derive(Debug)]
enum SequenceRejection {
    ReservationReplay(ReservationReplayError),
    EdgeReplay(EdgeReplayError),
    Declaration(EdgeDeclarationRejection),
    EdgeLimitReached(u32),
    InactiveMarkerRun(CoordinationRunId),
}

#[derive(Debug)]
enum SequenceError {
    Io(std::io::Error),
    Config(ConfigError),
    Ledger(LedgerError),
    Transaction(LedgerTransactionError),
    Rejected(SequenceRejection),
    MissingPreparedEdge,
}

impl Display for SequenceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "sequence I/O failed: {error}"),
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::Rejected(rejection) => rejection.fmt(formatter),
            Self::MissingPreparedEdge => {
                formatter.write_str("the appended sequence operation lost its prepared edge")
            },
        }
    }
}

impl Display for SequenceRejection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservationReplay(error) => error.fmt(formatter),
            Self::EdgeReplay(error) => error.fmt(formatter),
            Self::Declaration(error) => error.fmt(formatter),
            Self::EdgeLimitReached(maximum) => write!(
                formatter,
                "the configured maximum of {maximum} ordering edges has been reached"
            ),
            Self::InactiveMarkerRun(coordination_run_id) => write!(
                formatter,
                "coordination-run marker {coordination_run_id} no longer has an active reservation"
            ),
        }
    }
}

impl std::error::Error for SequenceError {}

impl From<std::io::Error> for SequenceError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for SequenceError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<LedgerError> for SequenceError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<LedgerTransactionError> for SequenceError {
    fn from(error: LedgerTransactionError) -> Self { Self::Transaction(error) }
}
