//! The failure modes of deriving one coherent board from retained facts.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::edge::EdgeReplayError;
use crate::edge::MissingReadinessFact;
use crate::gate::permit::ForcedIntegrationPermitReplayError;
use crate::ids::EdgeId;
use crate::ids::ProjectionGeneration;
use crate::reservation::ReservationReplayError;

/// A coherent board could not be derived from retained journal and repository facts.
#[derive(Debug)]
pub(crate) enum BoardError {
    /// Reservation replay failed.
    Reservation(ReservationReplayError),
    /// Ordering graph replay failed.
    Edge(EdgeReplayError),
    /// A repository observation omitted a required edge fact.
    MissingReadiness(MissingReadinessFact),
    /// A recorded answer named an edge absent from the replayed graph.
    MissingOrderingEdge(EdgeId),
    /// Forced-permit replay found inconsistent issue or consumption records.
    ForcedPermitReplay(ForcedIntegrationPermitReplayError),
    /// An orphan alert retained a branch reference that no longer satisfies its type.
    InvalidBranchReference(String),
    /// The projection and event replay did not describe the same committed generation.
    MismatchedProjectionGeneration {
        /// The generation carried by the retained event replay.
        replay:      ProjectionGeneration,
        /// The generation carried by the shared constraint projection.
        constraints: ProjectionGeneration,
    },
}

impl Display for BoardError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reservation(error) => error.fmt(formatter),
            Self::Edge(error) => error.fmt(formatter),
            Self::MissingReadiness(error) => error.fmt(formatter),
            Self::MissingOrderingEdge(edge_id) => {
                write!(
                    formatter,
                    "recorded overlap answer names missing edge {edge_id}"
                )
            },
            Self::ForcedPermitReplay(error) => error.fmt(formatter),
            Self::InvalidBranchReference(reference) => {
                write!(
                    formatter,
                    "orphan alert retained invalid branch reference {reference}"
                )
            },
            Self::MismatchedProjectionGeneration {
                replay,
                constraints,
            } => write!(
                formatter,
                "board replay generation {replay} does not match constraint generation {constraints}"
            ),
        }
    }
}

impl Error for BoardError {}

impl From<ReservationReplayError> for BoardError {
    fn from(error: ReservationReplayError) -> Self { Self::Reservation(error) }
}

impl From<EdgeReplayError> for BoardError {
    fn from(error: EdgeReplayError) -> Self { Self::Edge(error) }
}

impl From<MissingReadinessFact> for BoardError {
    fn from(error: MissingReadinessFact) -> Self { Self::MissingReadiness(error) }
}
