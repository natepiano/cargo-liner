//! Command-boundary dispatch for working-tree drift reconciliation.

use crate::coordination_identity::RecoveryCommandLine;
use crate::drift;
use crate::drift::DriftRequest;
use crate::output::OutputEnvelope;

/// Run one parsed drift request.
pub(crate) fn execute(
    request: DriftRequest,
    recovery_command_line: &RecoveryCommandLine,
) -> OutputEnvelope {
    drift::execute(request, recovery_command_line)
}
