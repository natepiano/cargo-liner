//! Command-boundary dispatch for working-tree drift reconciliation.

use crate::drift;
use crate::drift::DriftRequest;
use crate::output::OutputEnvelope;

/// Run one parsed drift request.
pub(crate) fn execute(request: DriftRequest) -> OutputEnvelope { drift::execute(request) }
