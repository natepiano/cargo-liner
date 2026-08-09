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
    all(not(test), not(target_os = "linux")),
    expect(
        dead_code,
        reason = "build-monitor authorization has no production capability consumer yet"
    )
)]
mod platform;
mod transaction;

use std::num::NonZeroU64;
use std::thread::JoinHandle;
use std::time::Instant;

#[cfg(any(target_os = "linux", test))]
use platform::BoundProcessObjectPresence;
use platform::BoundSignalDelivery;
pub(crate) use platform::ExternalProcessTerminationCapability;
use platform::TerminationSignalAdmission;
pub(crate) use transaction::AdmittedTerminationDescendantObservation;
pub(crate) use transaction::AdmittedTerminationDescendantPresence;
pub(crate) use transaction::FrozenTerminationRootObservation;
pub(crate) use transaction::FrozenTerminationRootPresence;
pub(crate) use transaction::TerminationDescendantObservationPass;
pub(crate) use transaction::observe_termination_descendants;

use crate::channel;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::channel::TryRecvError;

/// A monotonically allocated identity for one submitted termination request.
///
/// The counter is [`NonZeroU64`], so no caller can name a placeholder request
/// that later correlates against a real one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TerminationRequestId(NonZeroU64);

/// Private semantic identity for one target inside a termination transaction.
///
/// The identity is allocated by the transaction owner and travels with both
/// the frozen capability and the worker outcome. It deliberately is not a PID
/// or a vector offset, so a reordered worker plan cannot reconcile one
/// process's outcome to another build session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TerminationTargetId(NonZeroU64);

impl TerminationTargetId {
    /// Allocate an identity from a transaction-owned counter.
    pub(crate) const fn from_non_zero(value: NonZeroU64) -> Self { Self(value) }

    #[cfg(test)]
    pub(crate) const fn for_test(value: NonZeroU64) -> Self { Self(value) }
}

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
/// was authorized cannot join it. The worker orders admitted descendants from
/// deepest to shallowest and frozen roots last.
#[derive(Debug)]
pub(crate) struct TerminationExecutionPlan {
    termination_request_id: TerminationRequestId,
    deadline:               TerminationExecutionDeadline,
    targets:                Vec<TerminationExecutionTarget>,
}

impl TerminationExecutionPlan {
    pub(crate) const fn termination_request_id(&self) -> TerminationRequestId {
        self.termination_request_id
    }

    #[cfg(test)]
    const fn target_count(&self) -> usize { self.targets.len() }
}

/// The time boundary applied by the external worker.
#[derive(Clone, Copy, Debug)]
enum TerminationExecutionDeadline {
    At(Instant),
    StartupHandshake,
}

impl TerminationExecutionDeadline {
    fn expired(self, now: Instant) -> bool {
        match self {
            Self::At(deadline) => now >= deadline,
            Self::StartupHandshake => false,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn confirmation_timeout(self, now: Instant) -> std::time::Duration {
        match self {
            Self::At(deadline) => self::constants::TERMINATION_CONFIRMATION_TIMEOUT
                .min(deadline.saturating_duration_since(now)),
            Self::StartupHandshake => self::constants::TERMINATION_CONFIRMATION_TIMEOUT,
        }
    }
}

/// Where one target sits in its frozen process tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TerminationExecutionTargetRole {
    FrozenRoot,
    AdmittedDescendant { depth_from_root: usize },
}

impl TerminationExecutionTargetRole {
    const fn leaf_order(self) -> usize {
        match self {
            Self::FrozenRoot => 0,
            Self::AdmittedDescendant { depth_from_root } => depth_from_root,
        }
    }
}

/// One frozen external target together with the transaction identity that owns
/// its eventual result.
#[derive(Debug)]
pub(crate) struct TerminationExecutionTarget {
    semantic_target_id:                      TerminationTargetId,
    role:                                    TerminationExecutionTargetRole,
    external_process_termination_capability: ExternalProcessTerminationCapability,
}

impl TerminationExecutionTarget {
    /// Pair one opaque, move-only capability with its transaction-local
    /// semantic identity.
    pub(crate) const fn new(
        semantic_target_id: TerminationTargetId,
        external_process_termination_capability: ExternalProcessTerminationCapability,
    ) -> Self {
        Self {
            semantic_target_id,
            role: TerminationExecutionTargetRole::FrozenRoot,
            external_process_termination_capability,
        }
    }

    /// Pair a newly admitted descendant with its validated root depth.
    pub(crate) const fn admitted_descendant(
        semantic_target_id: TerminationTargetId,
        depth_from_root: usize,
        external_process_termination_capability: ExternalProcessTerminationCapability,
    ) -> Self {
        Self {
            semantic_target_id,
            role: TerminationExecutionTargetRole::AdmittedDescendant { depth_from_root },
            external_process_termination_capability,
        }
    }
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
    /// The root still existed, but no longer satisfied its frozen scope condition.
    FrozenScopeDiverged { pid: u32 },
    /// The transaction deadline arrived before this target's pass began.
    DeadlineExpired { pid: u32 },
    /// The worker's monotonic request identity space was exhausted before the
    /// target could be dispatched.
    RequestIdentitiesExhausted { pid: u32 },
    /// The dedicated termination worker was unavailable when the target was
    /// dispatched.
    TerminationWorkerUnavailable { pid: u32 },
    /// The worker accepted the plan under a request identity other than the
    /// one allocated for this transaction pass.
    TerminationRequestCorrelationMismatch { pid: u32 },
}

/// What one planned target's termination attempt established.
///
/// Each variant states only what the worker observed. Nothing here claims that
/// a signal caused an exit it cannot attribute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TerminationTargetResult {
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

/// One target's correlated result from the external worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminationTargetOutcome {
    semantic_target_id: TerminationTargetId,
    role:               TerminationExecutionTargetRole,
    result:             TerminationTargetResult,
}

impl TerminationTargetOutcome {
    pub(crate) const fn semantic_target_id(&self) -> TerminationTargetId { self.semantic_target_id }

    pub(crate) const fn result(&self) -> &TerminationTargetResult { &self.result }

    pub(crate) const fn role(&self) -> TerminationExecutionTargetRole { self.role }
}

/// Whether the worker completed its ordered pass before the plan deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminationOutcomeDeadline {
    CompletedWithinDeadline,
    Expired,
}

/// One request's correlated, immutable result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminationOutcomeSummary {
    termination_request_id: TerminationRequestId,
    deadline:               TerminationOutcomeDeadline,
    target_outcomes:        Vec<TerminationTargetOutcome>,
}

impl TerminationOutcomeSummary {
    pub(crate) const fn termination_request_id(&self) -> TerminationRequestId {
        self.termination_request_id
    }

    pub(crate) fn target_outcomes(&self) -> &[TerminationTargetOutcome] { &self.target_outcomes }

    #[cfg(test)]
    const fn deadline(&self) -> TerminationOutcomeDeadline { self.deadline }
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
/// It holds the request and result channel ends plus the worker join handle.
/// Dropping the terminator sends `ProcessTerminationWorkerCommand::Shutdown`
/// and joins the thread after its bounded in-flight work completes.
#[derive(Debug)]
pub(crate) struct ProcessTerminator {
    next_request_id:  NonZeroU64,
    command_sender:   Sender<ProcessTerminationWorkerCommand>,
    outcome_receiver: Receiver<TerminationOutcomeSummary>,
    thread_state:     ProcessTerminationWorkerThreadState,
}

#[derive(Debug)]
enum ProcessTerminationWorkerCommand {
    Execute(TerminationExecutionPlan),
    Shutdown,
}

#[derive(Debug)]
enum ProcessTerminationWorkerThreadState {
    Running(JoinHandle<()>),
    Joined,
    #[cfg(test)]
    Disconnected,
}

impl ProcessTerminator {
    /// Start the dedicated termination worker.
    pub(crate) fn start() -> Self {
        let (command_sender, command_receiver) = channel::unbounded();
        let (outcome_sender, outcome_receiver) = channel::unbounded();
        let join_handle = std::thread::spawn(move || {
            run_termination_worker(&command_receiver, &outcome_sender);
        });
        Self {
            next_request_id: NonZeroU64::MIN,
            command_sender,
            outcome_receiver,
            thread_state: ProcessTerminationWorkerThreadState::Running(join_handle),
        }
    }

    #[cfg(test)]
    pub(crate) fn disconnected_for_test() -> Self {
        let (command_sender, command_receiver) = channel::unbounded();
        let (outcome_sender, outcome_receiver) = channel::unbounded();
        drop(command_receiver);
        drop(outcome_sender);
        Self {
            next_request_id: NonZeroU64::MIN,
            command_sender,
            outcome_receiver,
            thread_state: ProcessTerminationWorkerThreadState::Disconnected,
        }
    }

    /// Freeze one request's targets under a fresh request identity.
    pub(crate) fn plan_termination(
        &mut self,
        targets: Vec<TerminationExecutionTarget>,
    ) -> TerminationPlanCreation {
        let termination_request_id = TerminationRequestId(self.next_request_id);
        let Some(next_request_id) = self.next_request_id.checked_add(1) else {
            return TerminationPlanCreation::RequestIdsExhausted;
        };
        self.next_request_id = next_request_id;
        TerminationPlanCreation::Planned(TerminationExecutionPlan {
            termination_request_id,
            deadline: TerminationExecutionDeadline::StartupHandshake,
            targets,
        })
    }

    /// Freeze one deadline-bounded transaction pass under a fresh request identity.
    pub(crate) fn plan_bounded_termination(
        &mut self,
        targets: Vec<TerminationExecutionTarget>,
        deadline: Instant,
    ) -> TerminationPlanCreation {
        let termination_request_id = TerminationRequestId(self.next_request_id);
        let Some(next_request_id) = self.next_request_id.checked_add(1) else {
            return TerminationPlanCreation::RequestIdsExhausted;
        };
        self.next_request_id = next_request_id;
        TerminationPlanCreation::Planned(TerminationExecutionPlan {
            termination_request_id,
            deadline: TerminationExecutionDeadline::At(deadline),
            targets,
        })
    }

    /// Hand one frozen plan to the worker without waiting for its result.
    pub(crate) fn request_termination(
        &self,
        termination_execution_plan: TerminationExecutionPlan,
    ) -> TerminationDispatchOutcome {
        let termination_request_id = termination_execution_plan.termination_request_id();
        self.command_sender
            .send(ProcessTerminationWorkerCommand::Execute(
                termination_execution_plan,
            ))
            .map_or(TerminationDispatchOutcome::WorkerUnavailable, |()| {
                TerminationDispatchOutcome::Dispatched(termination_request_id)
            })
    }

    /// Take one completed request's summary if the worker has returned it.
    pub(crate) fn poll_outcome(&self) -> TerminationResultPoll {
        match self.outcome_receiver.try_recv() {
            Ok(termination_outcome_summary) => {
                TerminationResultPoll::Completed(termination_outcome_summary)
            },
            Err(TryRecvError::Empty) => TerminationResultPoll::NoCompletedRequest,
            Err(TryRecvError::Disconnected) => TerminationResultPoll::WorkerUnavailable,
        }
    }

    /// Borrow the worker result channel only to register an event-loop wakeup.
    pub(crate) const fn outcome_receiver(&self) -> &Receiver<TerminationOutcomeSummary> {
        &self.outcome_receiver
    }
}

impl Drop for ProcessTerminator {
    fn drop(&mut self) {
        let _ = self
            .command_sender
            .send(ProcessTerminationWorkerCommand::Shutdown);
        let thread_state = std::mem::replace(
            &mut self.thread_state,
            ProcessTerminationWorkerThreadState::Joined,
        );
        if let ProcessTerminationWorkerThreadState::Running(join_handle) = thread_state {
            let _ = join_handle.join();
        }
    }
}

fn run_termination_worker(
    command_receiver: &Receiver<ProcessTerminationWorkerCommand>,
    outcome_sender: &Sender<TerminationOutcomeSummary>,
) {
    while let Ok(command) = command_receiver.recv() {
        match command {
            ProcessTerminationWorkerCommand::Execute(termination_execution_plan) => {
                if outcome_sender
                    .send(execute_termination_plan(&termination_execution_plan))
                    .is_err()
                {
                    break;
                }
            },
            ProcessTerminationWorkerCommand::Shutdown => break,
        }
    }
}

fn execute_termination_plan(
    termination_execution_plan: &TerminationExecutionPlan,
) -> TerminationOutcomeSummary {
    let mut ordered_targets: Vec<&TerminationExecutionTarget> =
        termination_execution_plan.targets.iter().collect();
    ordered_targets.sort_by(|left, right| {
        right
            .role
            .leaf_order()
            .cmp(&left.role.leaf_order())
            .then_with(|| left.semantic_target_id.cmp(&right.semantic_target_id))
    });
    let mut deadline = TerminationOutcomeDeadline::CompletedWithinDeadline;
    let target_outcomes = ordered_targets
        .into_iter()
        .map(|termination_execution_target| {
            let result = if termination_execution_plan.deadline.expired(Instant::now()) {
                deadline = TerminationOutcomeDeadline::Expired;
                TerminationTargetResult::Refused(TerminationError::DeadlineExpired {
                    pid: termination_execution_target
                        .external_process_termination_capability
                        .pid(),
                })
            } else {
                terminate_one_target(
                    &termination_execution_target.external_process_termination_capability,
                    termination_execution_plan.deadline,
                )
            };
            if termination_execution_plan.deadline.expired(Instant::now()) {
                deadline = TerminationOutcomeDeadline::Expired;
            }
            TerminationTargetOutcome {
                semantic_target_id: termination_execution_target.semantic_target_id,
                role: termination_execution_target.role,
                result,
            }
        })
        .collect();
    TerminationOutcomeSummary {
        termination_request_id: termination_execution_plan.termination_request_id,
        deadline,
        target_outcomes,
    }
}

/// Observe the host once per target, then act on what it reported.
///
/// The observation and the decision stay separate so the acting half can be
/// driven from fixed admission evidence instead of a live process race.
fn terminate_one_target(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    termination_execution_deadline: TerminationExecutionDeadline,
) -> TerminationTargetResult {
    apply_termination_admission(
        external_process_termination_capability,
        external_process_termination_capability.observe_admission(),
        termination_execution_deadline,
    )
}

#[cfg(any(target_os = "linux", test))]
fn apply_termination_admission(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    termination_signal_admission: TerminationSignalAdmission,
    termination_execution_deadline: TerminationExecutionDeadline,
) -> TerminationTargetResult {
    let pid = external_process_termination_capability.pid();
    match termination_signal_admission {
        TerminationSignalAdmission::PidReused | TerminationSignalAdmission::ProcessGone => {
            TerminationTargetResult::AlreadyGone { pid }
        },
        TerminationSignalAdmission::RevalidationUnavailable => {
            TerminationTargetResult::Refused(TerminationError::ProcessRevalidationUnavailable {
                pid,
            })
        },
        TerminationSignalAdmission::ProcessImageReplaced => {
            TerminationTargetResult::Refused(TerminationError::ProcessImageReplaced { pid })
        },
        TerminationSignalAdmission::SameProcessObject => {
            if external_process_termination_capability.is_actionable() {
                deliver_one_bound_signal(
                    external_process_termination_capability,
                    pid,
                    termination_execution_deadline.confirmation_timeout(Instant::now()),
                )
            } else {
                TerminationTargetResult::Refused(TerminationError::HostHasNoIdentityBoundAdapter {
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
    _: TerminationExecutionDeadline,
) -> TerminationTargetResult {
    let pid = external_process_termination_capability.pid();
    match termination_signal_admission {
        TerminationSignalAdmission::PidReused | TerminationSignalAdmission::ProcessGone => {
            TerminationTargetResult::AlreadyGone { pid }
        },
        TerminationSignalAdmission::RevalidationUnavailable => {
            TerminationTargetResult::Refused(TerminationError::ProcessRevalidationUnavailable {
                pid,
            })
        },
        TerminationSignalAdmission::ProcessImageReplaced => {
            TerminationTargetResult::Refused(TerminationError::ProcessImageReplaced { pid })
        },
        TerminationSignalAdmission::SameProcessObject => {
            if external_process_termination_capability.is_actionable() {
                deliver_one_bound_signal(
                    external_process_termination_capability,
                    pid,
                    std::time::Duration::ZERO,
                )
            } else {
                TerminationTargetResult::Refused(TerminationError::HostHasNoIdentityBoundAdapter {
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
    confirmation_timeout: std::time::Duration,
) -> TerminationTargetResult {
    match external_process_termination_capability.deliver_termination_request() {
        BoundSignalDelivery::Rejected => {
            TerminationTargetResult::Refused(TerminationError::HostRejectedSignal { pid })
        },
        #[cfg(any(target_os = "linux", test))]
        BoundSignalDelivery::ProcessGone => TerminationTargetResult::AlreadyGone { pid },
        #[cfg(any(target_os = "linux", test))]
        BoundSignalDelivery::Accepted => {
            match external_process_termination_capability
                .confirm_process_object_gone(confirmation_timeout)
            {
                #[cfg(any(target_os = "linux", test))]
                BoundProcessObjectPresence::Gone => {
                    TerminationTargetResult::GoneAfterSignaling { pid }
                },
                #[cfg(any(target_os = "linux", test))]
                BoundProcessObjectPresence::Present => TerminationTargetResult::Survived { pid },
                BoundProcessObjectPresence::Unavailable => {
                    TerminationTargetResult::SignaledButUnconfirmed { pid }
                },
            }
        },
    }
}

#[cfg(all(not(target_os = "linux"), not(test)))]
const fn deliver_one_bound_signal(
    external_process_termination_capability: &ExternalProcessTerminationCapability,
    pid: u32,
    _: std::time::Duration,
) -> TerminationTargetResult {
    match external_process_termination_capability.deliver_termination_request() {
        BoundSignalDelivery::Rejected => {
            TerminationTargetResult::Refused(TerminationError::HostRejectedSignal { pid })
        },
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;
    use crate::process_observation;
    use crate::process_observation::PlatformTerminationCapabilityObservation;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::ProcessIncarnation;

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
        let targets = targets
            .into_iter()
            .enumerate()
            .map(|(index, external_process_termination_capability)| {
                let Ok(target_number) = u64::try_from(index + 1) else {
                    panic!("test target identities should fit in u64");
                };
                let Some(target_number) = NonZeroU64::new(target_number) else {
                    panic!("test target identities start at one");
                };
                let termination_target_id = TerminationTargetId::for_test(target_number);
                TerminationExecutionTarget::new(
                    termination_target_id,
                    external_process_termination_capability,
                )
            })
            .collect();
        match process_terminator.plan_termination(targets) {
            TerminationPlanCreation::Planned(termination_execution_plan) => {
                termination_execution_plan
            },
            TerminationPlanCreation::RequestIdsExhausted => {
                panic!("a fresh terminator has unused request identities")
            },
        }
    }

    fn bounded_plan(
        process_terminator: &mut ProcessTerminator,
        targets: Vec<TerminationExecutionTarget>,
        deadline: Instant,
    ) -> TerminationExecutionPlan {
        match process_terminator.plan_bounded_termination(targets, deadline) {
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
    fn bounded_pass_orders_deepest_descendant_before_parent_and_root() {
        let mut process_terminator = ProcessTerminator::start();
        let root_target_id = TerminationTargetId::for_test(NonZeroU64::MIN);
        let parent_target_id = TerminationTargetId::for_test(NonZeroU64::MIN.saturating_add(1));
        let leaf_target_id = TerminationTargetId::for_test(NonZeroU64::MIN.saturating_add(2));
        let plan = bounded_plan(
            &mut process_terminator,
            vec![
                TerminationExecutionTarget::new(root_target_id, test_capability(FIXTURE_PID)),
                TerminationExecutionTarget::admitted_descendant(
                    parent_target_id,
                    1,
                    test_capability(FIXTURE_PID + 1),
                ),
                TerminationExecutionTarget::admitted_descendant(
                    leaf_target_id,
                    2,
                    test_capability(FIXTURE_PID + 2),
                ),
            ],
            Instant::now() + Duration::from_secs(5),
        );
        let request_id = plan.termination_request_id();
        assert_eq!(
            process_terminator.request_termination(plan),
            TerminationDispatchOutcome::Dispatched(request_id)
        );
        let summary = await_outcome(&process_terminator);

        assert_eq!(
            summary
                .target_outcomes()
                .iter()
                .map(TerminationTargetOutcome::semantic_target_id)
                .collect::<Vec<_>>(),
            vec![leaf_target_id, parent_target_id, root_target_id]
        );
        assert!(matches!(
            summary.target_outcomes()[0].role(),
            TerminationExecutionTargetRole::AdmittedDescendant { depth_from_root: 2 }
        ));
        assert_eq!(
            summary.target_outcomes()[2].role(),
            TerminationExecutionTargetRole::FrozenRoot
        );
    }

    #[test]
    fn expired_pass_reports_deadline_without_claiming_a_signal() {
        let mut process_terminator = ProcessTerminator::start();
        let target_id = TerminationTargetId::for_test(NonZeroU64::MIN);
        let plan = bounded_plan(
            &mut process_terminator,
            vec![TerminationExecutionTarget::new(
                target_id,
                test_capability(FIXTURE_PID),
            )],
            Instant::now(),
        );
        let request_id = plan.termination_request_id();
        assert_eq!(
            process_terminator.request_termination(plan),
            TerminationDispatchOutcome::Dispatched(request_id)
        );
        let summary = await_outcome(&process_terminator);

        assert_eq!(summary.deadline(), TerminationOutcomeDeadline::Expired);
        assert!(matches!(
            summary.target_outcomes(),
            [TerminationTargetOutcome {
                semantic_target_id,
                result: TerminationTargetResult::Refused(TerminationError::DeadlineExpired {
                    pid: FIXTURE_PID,
                }),
                ..
            }] if *semantic_target_id == target_id
        ));
    }

    #[test]
    fn bounded_confirmation_reports_both_survivor_and_expired_deadline() {
        let mut process_terminator = ProcessTerminator::start();
        let target_id = TerminationTargetId::for_test(NonZeroU64::MIN);
        let capability = ExternalProcessTerminationCapability::for_test(
            ProcessIncarnation::for_test(
                ProcessIdentity::for_test(FIXTURE_PID, 7),
                "/usr/bin/cargo",
            ),
            BoundSignalDelivery::Accepted,
            &[BoundProcessObjectPresence::Present; 8],
        );
        let plan = bounded_plan(
            &mut process_terminator,
            vec![TerminationExecutionTarget::new(target_id, capability)],
            Instant::now() + Duration::from_millis(100),
        );
        let summary = execute_termination_plan(&plan);

        assert_eq!(summary.deadline(), TerminationOutcomeDeadline::Expired);
        assert!(matches!(
            summary.target_outcomes(),
            [TerminationTargetOutcome {
                semantic_target_id,
                result: TerminationTargetResult::Survived { pid: FIXTURE_PID },
                ..
            }] if *semantic_target_id == target_id
        ));
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
            apply_termination_admission(
                &capability,
                TerminationSignalAdmission::SameProcessObject,
                TerminationExecutionDeadline::StartupHandshake,
            ),
            TerminationTargetResult::GoneAfterSignaling { pid: FIXTURE_PID }
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
            apply_termination_admission(
                &capability,
                TerminationSignalAdmission::SameProcessObject,
                TerminationExecutionDeadline::StartupHandshake,
            ),
            TerminationTargetResult::Refused(TerminationError::HostRejectedSignal {
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
            apply_termination_admission(
                &capability,
                TerminationSignalAdmission::SameProcessObject,
                TerminationExecutionDeadline::StartupHandshake,
            ),
            TerminationTargetResult::AlreadyGone { pid: FIXTURE_PID }
        );
    }

    #[cfg(unix)]
    #[test]
    fn real_disappearance_reaches_already_gone_through_the_full_admission_path() {
        let mut child = Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap_or_else(|error| panic!("the disappearance fixture should spawn: {error}"));
        let capability =
            match process_observation::observe_platform_termination_capability_for_test(child.id())
            {
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
            terminate_one_target(&capability, TerminationExecutionDeadline::StartupHandshake),
            TerminationTargetResult::AlreadyGone { pid: child.id() }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_worker_signals_a_real_process_through_pidfd() {
        let mut child = Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap_or_else(|error| panic!("the pidfd fixture should spawn: {error}"));
        let capability =
            match process_observation::observe_platform_termination_capability_for_test(child.id())
            {
                PlatformTerminationCapabilityObservation::Available(capability) => capability,
                PlatformTerminationCapabilityObservation::InsufficientIncarnationEvidence => {
                    panic!("the live pidfd fixture should produce authority")
                },
            };
        assert!(capability.is_actionable());
        let mut process_terminator = ProcessTerminator::start();
        let plan = planned(&mut process_terminator, vec![capability]);
        let request_id = plan.termination_request_id();
        assert_eq!(
            process_terminator.request_termination(plan),
            TerminationDispatchOutcome::Dispatched(request_id)
        );
        let termination_outcome_summary = await_outcome(&process_terminator);
        assert!(matches!(
            termination_outcome_summary.target_outcomes(),
            [TerminationTargetOutcome {
                result: TerminationTargetResult::GoneAfterSignaling { pid },
                ..
            }] if *pid == child.id()
        ));
        child
            .wait()
            .unwrap_or_else(|error| panic!("the pidfd fixture should be reaped: {error}"));
    }

    #[test]
    fn dropping_the_terminator_ends_the_worker() {
        let process_terminator = ProcessTerminator::start();
        let outcome_rx = process_terminator.outcome_receiver().clone();
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
