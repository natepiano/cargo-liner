//! Identity-revalidated termination of processes Cargo Port did not start.
//!
//! Observation and termination stay separate subsystems.
//! [`crate::process_observation`] produces immutable evidence and platform
//! capabilities and never signals anything; [`ProcessTerminator`] owns every
//! revalidation, signal delivery, and confirmation wait and runs them on its
//! own worker thread, so the TUI event loop never blocks on a host call.
//!
//! Requests and results are correlated by [`TerminationRequestId`]: a plan is
//! immutable once built, and the summary the worker returns names the request
//! it answers.
//!
//! No layer here escalates to `SIGKILL`. A target that outlives its
//! confirmation deadline is reported as a survivor.

mod constants;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "build-monitor authorization has no production capability consumer yet"
    )
)]
mod platform;

use std::num::NonZeroU64;

#[cfg(any(target_os = "linux", test))]
use platform::BoundProcessObjectPresence;
use platform::BoundSignalDelivery;
pub(crate) use platform::ExternalProcessTerminationCapability;
use platform::TerminationSignalAdmission;

#[cfg(any(target_os = "linux", test))]
use self::constants::TERMINATION_CONFIRMATION_TIMEOUT;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::channel::TryRecvError;
use crate::channel::unbounded;

/// A monotonically allocated identity for one submitted termination request.
///
/// The counter is [`NonZeroU64`], so no caller can name a placeholder request
/// that later correlates against a real one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TerminationRequestId(NonZeroU64);

/// Whether the monotonic counter still had an unused request identity.
///
/// Exhaustion rejects the request rather than repeating an identity: a reissued
/// value would attach one request's outcome summary to another.
#[derive(Debug)]
pub(crate) enum TerminationPlanCreation {
    Planned(TerminationExecutionPlan),
    RequestIdsExhausted,
}

/// One request's frozen set of targets.
///
/// The plan is immutable once built, so a build that starts after the request
/// was authorized cannot join it. Target order is the order the plan was built
/// in; the worker signals targets in that order and never reorders them.
#[derive(Debug)]
pub(crate) struct TerminationExecutionPlan {
    termination_request_id: TerminationRequestId,
    targets:                Vec<ExternalProcessTerminationCapability>,
}

impl TerminationExecutionPlan {
    pub(crate) const fn termination_request_id(&self) -> TerminationRequestId {
        self.termination_request_id
    }

    #[cfg(test)]
    pub(crate) const fn target_count(&self) -> usize { self.targets.len() }
}

/// Why one planned target was not signaled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminationError {
    /// The PID still names the authorized process object, but the host binds
    /// no signal to it, so the target stays observed-only.
    HostHasNoIdentityBoundAdapter { pid: u32 },
    /// The PID names the authorized process lifetime, but that lifetime
    /// replaced its executable image after the plan authorized it.
    ProcessImageReplaced { pid: u32 },
    /// The host could not establish that the authorized process object is
    /// still present. This is not evidence that it has exited.
    ProcessRevalidationUnavailable { pid: u32 },
    /// The identity-bound host adapter refused the graceful signal.
    HostRejectedSignal { pid: u32 },
}

/// What one planned target's termination attempt established.
///
/// Each variant states only what the worker observed. Nothing here claims that
/// a signal caused an exit it cannot attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminationTargetOutcome {
    /// The authorized process object was already gone before any signal.
    AlreadyGone { pid: u32 },
    /// The signal was delivered and the identity-bound handle then reported
    /// the process object gone.
    #[cfg(any(target_os = "linux", test))]
    GoneAfterSignaling { pid: u32 },
    /// The signal was delivered and the identity-bound handle still reported
    /// the process object present when its confirmation deadline expired.
    #[cfg(any(target_os = "linux", test))]
    Survived { pid: u32 },
    /// The signal was delivered, but the bound handle could not establish
    /// whether the process object remained present.
    #[cfg(any(target_os = "linux", test))]
    SignaledButUnconfirmed { pid: u32 },
    /// No signal was delivered.
    Refused(TerminationError),
}

/// One request's correlated, immutable result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminationOutcomeSummary {
    termination_request_id: TerminationRequestId,
    target_outcomes:        Vec<TerminationTargetOutcome>,
}

impl TerminationOutcomeSummary {
    pub(crate) const fn termination_request_id(&self) -> TerminationRequestId {
        self.termination_request_id
    }

    #[cfg(test)]
    pub(crate) fn target_outcomes(&self) -> &[TerminationTargetOutcome] { &self.target_outcomes }
}

/// Whether a plan reached the termination worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminationDispatchOutcome {
    Dispatched(TerminationRequestId),
    WorkerUnavailable,
}

/// Whether a completed request's summary was waiting on the result channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminationResultPoll {
    Completed(TerminationOutcomeSummary),
    NoCompletedRequest,
    WorkerUnavailable,
}

/// The sole owner of external termination work.
///
/// It holds the request and result channel ends; the worker thread it starts
/// owns every host call. Dropping the terminator closes the request channel,
/// which is what ends the worker.
#[derive(Debug)]
pub(crate) struct ProcessTerminator {
    next_request_id: NonZeroU64,
    plan_tx:         Sender<TerminationExecutionPlan>,
    outcome_rx:      Receiver<TerminationOutcomeSummary>,
}

impl ProcessTerminator {
    /// Start the dedicated termination worker.
    pub(crate) fn start() -> Self {
        let (plan_tx, plan_rx) = unbounded::<TerminationExecutionPlan>();
        let (outcome_tx, outcome_rx) = unbounded::<TerminationOutcomeSummary>();
        std::thread::spawn(move || run_termination_worker(&plan_rx, &outcome_tx));
        Self {
            next_request_id: NonZeroU64::MIN,
            plan_tx,
            outcome_rx,
        }
    }

    /// Freeze one request's targets under a fresh request identity.
    pub(crate) fn plan_termination(
        &mut self,
        targets: Vec<ExternalProcessTerminationCapability>,
    ) -> TerminationPlanCreation {
        let termination_request_id = TerminationRequestId(self.next_request_id);
        let Some(next_request_id) = self.next_request_id.checked_add(1) else {
            return TerminationPlanCreation::RequestIdsExhausted;
        };
        self.next_request_id = next_request_id;
        TerminationPlanCreation::Planned(TerminationExecutionPlan {
            termination_request_id,
            targets,
        })
    }

    /// Hand one frozen plan to the worker without waiting for its result.
    pub(crate) fn request_termination(
        &self,
        termination_execution_plan: TerminationExecutionPlan,
    ) -> TerminationDispatchOutcome {
        let termination_request_id = termination_execution_plan.termination_request_id();
        self.plan_tx
            .send(termination_execution_plan)
            .map_or(TerminationDispatchOutcome::WorkerUnavailable, |()| {
                TerminationDispatchOutcome::Dispatched(termination_request_id)
            })
    }

    /// Take one completed request's summary if the worker has returned it.
    pub(crate) fn poll_outcome(&self) -> TerminationResultPoll {
        match self.outcome_rx.try_recv() {
            Ok(termination_outcome_summary) => {
                TerminationResultPoll::Completed(termination_outcome_summary)
            },
            Err(TryRecvError::Empty) => TerminationResultPoll::NoCompletedRequest,
            Err(TryRecvError::Disconnected) => TerminationResultPoll::WorkerUnavailable,
        }
    }
}

fn run_termination_worker(
    plan_rx: &Receiver<TerminationExecutionPlan>,
    outcome_tx: &Sender<TerminationOutcomeSummary>,
) {
    while let Ok(termination_execution_plan) = plan_rx.recv() {
        if outcome_tx
            .send(execute_termination_plan(&termination_execution_plan))
            .is_err()
        {
            break;
        }
    }
}

fn execute_termination_plan(
    termination_execution_plan: &TerminationExecutionPlan,
) -> TerminationOutcomeSummary {
    TerminationOutcomeSummary {
        termination_request_id: termination_execution_plan.termination_request_id,
        target_outcomes:        termination_execution_plan
            .targets
            .iter()
            .map(terminate_one_target)
            .collect(),
    }
}

/// Observe the host once per target, then act on what it reported.
///
/// The observation and the decision stay separate so the acting half can be
/// driven from fixed admission evidence instead of a live process race.
fn terminate_one_target(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
) -> TerminationTargetOutcome {
    apply_termination_admission(
        external_process_termination_capability,
        external_process_termination_capability.observe_admission(),
    )
}

#[cfg(any(target_os = "linux", test))]
fn apply_termination_admission(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    termination_signal_admission: TerminationSignalAdmission,
) -> TerminationTargetOutcome {
    let pid = external_process_termination_capability.pid();
    match termination_signal_admission {
        TerminationSignalAdmission::PidReused | TerminationSignalAdmission::ProcessGone => {
            TerminationTargetOutcome::AlreadyGone { pid }
        },
        TerminationSignalAdmission::RevalidationUnavailable => {
            TerminationTargetOutcome::Refused(TerminationError::ProcessRevalidationUnavailable {
                pid,
            })
        },
        TerminationSignalAdmission::ProcessImageReplaced => {
            TerminationTargetOutcome::Refused(TerminationError::ProcessImageReplaced { pid })
        },
        TerminationSignalAdmission::SameProcessObject => {
            if external_process_termination_capability.has_identity_bound_adapter() {
                deliver_one_bound_signal(external_process_termination_capability, pid)
            } else {
                TerminationTargetOutcome::Refused(TerminationError::HostHasNoIdentityBoundAdapter {
                    pid,
                })
            }
        },
    }
}

#[cfg(all(not(target_os = "linux"), not(test)))]
const fn apply_termination_admission(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    termination_signal_admission: TerminationSignalAdmission,
) -> TerminationTargetOutcome {
    let pid = external_process_termination_capability.pid();
    match termination_signal_admission {
        TerminationSignalAdmission::PidReused | TerminationSignalAdmission::ProcessGone => {
            TerminationTargetOutcome::AlreadyGone { pid }
        },
        TerminationSignalAdmission::RevalidationUnavailable => {
            TerminationTargetOutcome::Refused(TerminationError::ProcessRevalidationUnavailable {
                pid,
            })
        },
        TerminationSignalAdmission::ProcessImageReplaced => {
            TerminationTargetOutcome::Refused(TerminationError::ProcessImageReplaced { pid })
        },
        TerminationSignalAdmission::SameProcessObject => {
            if external_process_termination_capability.has_identity_bound_adapter() {
                deliver_one_bound_signal(external_process_termination_capability, pid)
            } else {
                TerminationTargetOutcome::Refused(TerminationError::HostHasNoIdentityBoundAdapter {
                    pid,
                })
            }
        },
    }
}

/// Deliver exactly one graceful signal and report what followed it. A survivor
/// is reported as a survivor; nothing here sends a second, stronger signal.
#[cfg(any(target_os = "linux", test))]
fn deliver_one_bound_signal(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    pid: u32,
) -> TerminationTargetOutcome {
    match external_process_termination_capability.deliver_termination_request() {
        BoundSignalDelivery::Rejected => {
            TerminationTargetOutcome::Refused(TerminationError::HostRejectedSignal { pid })
        },
        #[cfg(any(target_os = "linux", test))]
        BoundSignalDelivery::ProcessGone => TerminationTargetOutcome::AlreadyGone { pid },
        #[cfg(any(target_os = "linux", test))]
        BoundSignalDelivery::Accepted => {
            match external_process_termination_capability
                .confirm_process_object_gone(TERMINATION_CONFIRMATION_TIMEOUT)
            {
                #[cfg(any(target_os = "linux", test))]
                BoundProcessObjectPresence::Gone => {
                    TerminationTargetOutcome::GoneAfterSignaling { pid }
                },
                #[cfg(any(target_os = "linux", test))]
                BoundProcessObjectPresence::Present => TerminationTargetOutcome::Survived { pid },
                BoundProcessObjectPresence::Unavailable => {
                    TerminationTargetOutcome::SignaledButUnconfirmed { pid }
                },
            }
        },
    }
}

#[cfg(all(not(target_os = "linux"), not(test)))]
const fn deliver_one_bound_signal(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    pid: u32,
) -> TerminationTargetOutcome {
    match external_process_termination_capability.deliver_termination_request() {
        BoundSignalDelivery::Rejected => {
            TerminationTargetOutcome::Refused(TerminationError::HostRejectedSignal { pid })
        },
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::process_observation::PlatformTerminationCapabilityObservation;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::ProcessIncarnation;
    use crate::process_observation::observe_platform_termination_capability_for_test;

    const FIXTURE_PID: u32 = 4242;
    const TERMINATION_POLL_ATTEMPTS: usize = 200;

    fn test_capability(pid: u32) -> ExternalProcessTerminationCapability {
        ExternalProcessTerminationCapability::for_test(
            ProcessIncarnation::for_test(ProcessIdentity::for_test(pid, 7), "/usr/bin/cargo"),
            BoundSignalDelivery::Accepted,
            &[BoundProcessObjectPresence::Gone],
        )
    }

    fn planned(
        process_terminator: &mut ProcessTerminator,
        targets: Vec<ExternalProcessTerminationCapability>,
    ) -> TerminationExecutionPlan {
        match process_terminator.plan_termination(targets) {
            TerminationPlanCreation::Planned(termination_execution_plan) => {
                termination_execution_plan
            },
            TerminationPlanCreation::RequestIdsExhausted => {
                panic!("a fresh terminator has unused request identities")
            },
        }
    }

    fn await_outcome(process_terminator: &ProcessTerminator) -> TerminationOutcomeSummary {
        for _ in 0..TERMINATION_POLL_ATTEMPTS {
            match process_terminator.poll_outcome() {
                TerminationResultPoll::Completed(termination_outcome_summary) => {
                    return termination_outcome_summary;
                },
                TerminationResultPoll::NoCompletedRequest => {
                    std::thread::sleep(constants::TERMINATION_CONFIRMATION_POLL_INTERVAL);
                },
                TerminationResultPoll::WorkerUnavailable => {
                    panic!("the termination worker ended before returning a summary")
                },
            }
        }
        panic!("the termination worker returned no summary")
    }

    #[test]
    fn request_identities_are_monotonic_and_never_reissued() {
        let mut process_terminator = ProcessTerminator::start();
        let first = planned(&mut process_terminator, Vec::new()).termination_request_id();
        let second = planned(&mut process_terminator, Vec::new()).termination_request_id();
        assert_ne!(first, second);
        assert!(first < second);
    }

    #[test]
    fn a_plan_freezes_its_targets_at_creation() {
        let mut process_terminator = ProcessTerminator::start();
        let termination_execution_plan =
            planned(&mut process_terminator, vec![test_capability(FIXTURE_PID)]);
        assert_eq!(termination_execution_plan.target_count(), 1);
    }

    #[test]
    fn each_summary_names_the_request_it_answers() {
        let mut process_terminator = ProcessTerminator::start();
        let first = planned(&mut process_terminator, vec![test_capability(FIXTURE_PID)]);
        let second = planned(
            &mut process_terminator,
            vec![
                test_capability(FIXTURE_PID),
                test_capability(FIXTURE_PID + 1),
            ],
        );
        let first_request_id = first.termination_request_id();
        let second_request_id = second.termination_request_id();

        assert_eq!(
            process_terminator.request_termination(first),
            TerminationDispatchOutcome::Dispatched(first_request_id)
        );
        let first_summary = await_outcome(&process_terminator);
        assert_eq!(first_summary.termination_request_id(), first_request_id);
        assert_eq!(first_summary.target_outcomes().len(), 1);

        assert_eq!(
            process_terminator.request_termination(second),
            TerminationDispatchOutcome::Dispatched(second_request_id)
        );
        let second_summary = await_outcome(&process_terminator);
        assert_eq!(second_summary.termination_request_id(), second_request_id);
        assert_eq!(second_summary.target_outcomes().len(), 2);
    }

    #[test]
    fn a_bound_target_that_exits_after_one_signal_is_reported_as_gone() {
        let capability = ExternalProcessTerminationCapability::for_test(
            ProcessIncarnation::for_test(
                ProcessIdentity::for_test(FIXTURE_PID, 7),
                "/usr/bin/cargo",
            ),
            BoundSignalDelivery::Accepted,
            &[BoundProcessObjectPresence::Gone],
        );
        assert_eq!(
            apply_termination_admission(&capability, TerminationSignalAdmission::SameProcessObject),
            TerminationTargetOutcome::GoneAfterSignaling { pid: FIXTURE_PID }
        );
    }

    #[test]
    fn a_bound_target_the_host_refuses_is_reported_without_a_second_signal() {
        let capability = ExternalProcessTerminationCapability::for_test(
            ProcessIncarnation::for_test(
                ProcessIdentity::for_test(FIXTURE_PID, 7),
                "/usr/bin/cargo",
            ),
            BoundSignalDelivery::Rejected,
            &[BoundProcessObjectPresence::Present],
        );
        assert_eq!(
            apply_termination_admission(&capability, TerminationSignalAdmission::SameProcessObject),
            TerminationTargetOutcome::Refused(TerminationError::HostRejectedSignal {
                pid: FIXTURE_PID,
            })
        );
    }

    #[test]
    fn disappearance_between_admission_and_bound_delivery_is_already_gone() {
        let capability = ExternalProcessTerminationCapability::for_test(
            ProcessIncarnation::for_test(
                ProcessIdentity::for_test(FIXTURE_PID, 7),
                "/usr/bin/cargo",
            ),
            BoundSignalDelivery::ProcessGone,
            &[],
        );
        assert_eq!(
            apply_termination_admission(&capability, TerminationSignalAdmission::SameProcessObject),
            TerminationTargetOutcome::AlreadyGone { pid: FIXTURE_PID }
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_disappearance_reaches_already_gone_through_the_full_admission_path() {
        let mut child = Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap_or_else(|error| panic!("the disappearance fixture should spawn: {error}"));
        let capability = match observe_platform_termination_capability_for_test(child.id()) {
            PlatformTerminationCapabilityObservation::Available(capability) => capability,
            PlatformTerminationCapabilityObservation::InsufficientIncarnationEvidence => {
                panic!("the live disappearance fixture should produce authority")
            },
        };
        child
            .kill()
            .unwrap_or_else(|error| panic!("the disappearance fixture should stop: {error}"));
        child
            .wait()
            .unwrap_or_else(|error| panic!("the disappearance fixture should be reaped: {error}"));

        assert_eq!(
            terminate_one_target(&capability),
            TerminationTargetOutcome::AlreadyGone { pid: child.id() }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_worker_signals_a_real_process_through_pidfd() {
        let mut child = Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap_or_else(|error| panic!("the pidfd fixture should spawn: {error}"));
        let capability = match observe_platform_termination_capability_for_test(child.id()) {
            PlatformTerminationCapabilityObservation::Available(capability) => capability,
            PlatformTerminationCapabilityObservation::InsufficientIncarnationEvidence => {
                panic!("the live pidfd fixture should produce authority")
            },
        };
        assert!(capability.has_identity_bound_adapter());
        let mut process_terminator = ProcessTerminator::start();
        let plan = planned(&mut process_terminator, vec![capability]);
        let request_id = plan.termination_request_id();
        assert_eq!(
            process_terminator.request_termination(plan),
            TerminationDispatchOutcome::Dispatched(request_id)
        );
        assert_eq!(
            await_outcome(&process_terminator).target_outcomes(),
            &[TerminationTargetOutcome::GoneAfterSignaling { pid: child.id() }]
        );
        child
            .wait()
            .unwrap_or_else(|error| panic!("the pidfd fixture should be reaped: {error}"));
    }

    #[test]
    fn dropping_the_terminator_ends_the_worker() {
        let process_terminator = ProcessTerminator::start();
        let outcome_rx = process_terminator.outcome_rx.clone();
        drop(process_terminator);
        for _ in 0..TERMINATION_POLL_ATTEMPTS {
            if outcome_rx.try_recv() == Err(TryRecvError::Disconnected) {
                return;
            }
            std::thread::sleep(constants::TERMINATION_CONFIRMATION_POLL_INTERVAL);
        }
        panic!("the worker outlived the terminator that owns it");
    }
}
