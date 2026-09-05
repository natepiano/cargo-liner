use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::Signal;
use sysinfo::System;

use crate::process_observation::identity;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::identity::StrongProcessIdentityRevalidation;

/// Opaque authority to signal one strongly identified Running Targets process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::tui) struct RunningTargetTerminationCapability {
    process_identity: ProcessIdentity,
}

impl RunningTargetTerminationCapability {
    pub(in crate::tui) const fn from_observed_identity(process_identity: ProcessIdentity) -> Self {
        Self { process_identity }
    }

    pub(in crate::tui) const fn process_identity(&self) -> &ProcessIdentity {
        &self.process_identity
    }

    /// Revalidate the observed identity and preserve the legacy `SIGTERM` action.
    pub(in crate::tui) fn terminate(self) -> RunningTargetTerminationOutcome {
        let pid = Pid::from_u32(self.process_identity.pid());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().without_tasks(),
        );
        let Some(process) = system.process(pid) else {
            return RunningTargetTerminationOutcome::SignalNotDelivered;
        };
        match identity::revalidate_strong_process_identity(&self.process_identity) {
            StrongProcessIdentityRevalidation::Current => {},
            StrongProcessIdentityRevalidation::Replaced(_)
            | StrongProcessIdentityRevalidation::Unavailable(_) => {
                return RunningTargetTerminationOutcome::IdentityNoLongerCurrent;
            },
        }
        match process.kill_with(Signal::Term) {
            Some(true) => RunningTargetTerminationOutcome::SignalDelivered,
            Some(false) | None => RunningTargetTerminationOutcome::SignalNotDelivered,
        }
    }

    #[cfg(test)]
    pub(in crate::tui) const fn for_test(pid: u32, creation_token: u64) -> Self {
        Self::from_observed_identity(ProcessIdentity::for_test(pid, creation_token))
    }
}

/// The result of the preserved Running Targets signaling path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::tui) enum RunningTargetTerminationOutcome {
    SignalDelivered,
    SignalNotDelivered,
    IdentityNoLongerCurrent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_pid_is_rejected_before_running_target_signaling() {
        let stale_capability = RunningTargetTerminationCapability::for_test(std::process::id(), 0);

        assert_eq!(
            stale_capability.terminate(),
            RunningTargetTerminationOutcome::IdentityNoLongerCurrent
        );
    }
}
