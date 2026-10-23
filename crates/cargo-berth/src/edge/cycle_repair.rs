//! The exact live ordering edge an explicit cycle repair may retire.

use serde::Deserialize;
use serde::Serialize;

use crate::ids::EdgeId;
use crate::ids::EventId;
use crate::ids::ReservationId;

/// An edge whose addition closes a cycle among the journal's surviving edges.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CycleClosingEdge {
    /// The one relationship retired by the repair.
    pub(crate) edge_id:              EdgeId,
    /// The journal record that introduced this relationship.
    pub(crate) declaration_event_id: EventId,
    /// The reservation ordered first by this edge.
    pub(crate) predecessor:          ReservationId,
    /// The reservation held behind that predecessor.
    pub(crate) successor:            ReservationId,
}

/// Whether replay proves an ordering edge needs explicit retirement.
pub(crate) enum OrderingCycleRepairPlan {
    /// Every surviving relationship is acyclic; no journal change is needed.
    Acyclic,
    /// The first surviving edge in append order that closes a cycle.
    Retire(CycleClosingEdge),
}
