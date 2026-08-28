//! Locked conversion of a recorded deferral into one ordering edge.

use std::cell::RefCell;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::config::Enrollment;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::coordination_identity::CoordinationIdentityValidationContext;
use crate::coordination_identity::CoordinationIdentityValidationError;
use crate::coordination_identity::RecoveryCommandLine;
use crate::coordination_identity::validate_coordination_identity;
use crate::edge::EdgeDeclarationRejection;
use crate::edge::EdgeReplayError;
use crate::edge::OrderingEdge;
use crate::edge::OrderingGraph;
use crate::edge::OrderingReason;
use crate::edge::PreparedOrderingEdge;
use crate::ids::ReservationId;
use crate::ledger;
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
pub(crate) fn execute(
    sequence_request: &SequenceRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
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
        Ok(Enrollment::Enrolled(reconciliation_report)) => reconciliation_report,
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => {
            return OutputEnvelope::unconfigured(
                CommandVerb::Sequence,
                &expected_configuration_path,
            );
        },
        Err(error) => return error.into_output(CommandVerb::Sequence),
    };
    let output_envelope = match execute_sequence(sequence_request, recovery_command_line) {
        Ok(Enrollment::Enrolled(edge)) => {
            match edge.readiness(&reconciliation_report.repository_snapshot) {
                Ok(readiness) => OutputEnvelope::sequenced(edge, readiness),
                Err(error) => {
                    OutputEnvelope::ledger_unreadable(CommandVerb::Sequence, &error.to_string())
                },
            }
        },
        Ok(Enrollment::Unconfigured {
            expected_configuration_path,
        }) => OutputEnvelope::unconfigured(CommandVerb::Sequence, &expected_configuration_path),
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
        Err(SequenceError::Rejected(SequenceRejection::CoordinationIdentity(rejection))) => {
            OutputEnvelope::coordination_identity_rejected(CommandVerb::Sequence, rejection)
        },
        Err(SequenceError::Rejected(SequenceRejection::ReservationReplay(error))) => {
            OutputEnvelope::replay_failure(CommandVerb::Sequence, &error)
        },
        Err(SequenceError::Rejected(SequenceRejection::InvalidCanonicalWorktreeRoot)) => {
            OutputEnvelope::ledger_unreadable(
                CommandVerb::Sequence,
                "the current worktree root is not canonical UTF-8",
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
        ) => OutputEnvelope::ledger_error(CommandVerb::Sequence, &error),
        Err(SequenceError::Config(error)) => {
            OutputEnvelope::ledger_error(CommandVerb::Sequence, &LedgerError::Config(error))
        },
        Err(error) => OutputEnvelope::ledger_unreadable(CommandVerb::Sequence, &error.to_string()),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_sequence(
    sequence_request: &SequenceRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> Result<Enrollment<OrderingEdge>, SequenceError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let resolved_edit_authorization = ledger::resolve_identity(&worktree_context)?;
    let journal_mutation_actor = resolved_edit_authorization
        .journal_mutation_actor_for(resolved_edit_authorization.coordination_run_id);
    let identity_validation = CoordinationIdentityValidationContext::for_user_command(
        resolved_edit_authorization,
        &worktree_context,
        recovery_command_line,
    );
    let berth_config = match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let prepared_edge = RefCell::<PreparedEdgeState>::new(PreparedEdgeState::NotPrepared);
    let outcome = ledger.transact(
        journal_mutation_actor.worktree_id,
        journal_mutation_actor.coordination_run_id,
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return TransactionValidation::Reject(SequenceRejection::ReservationReplay(
                        error,
                    ));
                },
            };
            if let Err(error) = validate_coordination_identity(&reservations, &identity_validation)
            {
                let rejection = match error {
                    CoordinationIdentityValidationError::Rejected(rejection) => {
                        SequenceRejection::CoordinationIdentity(rejection)
                    },
                    CoordinationIdentityValidationError::InvalidCanonicalWorktreeRoot => {
                        SequenceRejection::InvalidCanonicalWorktreeRoot
                    },
                };
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
        LedgerTransactionOutcome::Appended { event, .. } => {
            let PreparedEdgeState::Prepared(edge) = prepared_edge.into_inner() else {
                return Err(SequenceError::MissingPreparedEdge);
            };
            Ok(Enrollment::Enrolled(edge.into_edge(event.event_id())))
        },
        LedgerTransactionOutcome::Rejected(rejection) => Err(SequenceError::Rejected(rejection)),
    }
}

enum PreparedEdgeState {
    NotPrepared,
    Prepared(PreparedOrderingEdge),
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
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
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
            Self::CoordinationIdentity(rejection) => rejection.fmt(formatter),
            Self::InvalidCanonicalWorktreeRoot => {
                formatter.write_str("the current worktree root is not canonical UTF-8")
            },
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
