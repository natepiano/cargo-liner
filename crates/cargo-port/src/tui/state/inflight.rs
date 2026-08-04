//! The `Inflight` subsystem.
//!
//! Owns App's in-flight bookkeeping:
//! - clean: in-flight cargo clean paths plus the running-clean toast slot
//! - `pending_cleans`, `pending_ci_fetch`
//! - one identity-correlated Cargo Port-owned run and its retained output
//!
//! Lint lifecycle (`runtime`, running paths, toast) lives on
//! [`Lint`](super::Lint); CI fetch lifecycle lives on
//! [`Ci`](super::Ci).

use std::collections::VecDeque;
use std::num::NonZeroU64;
use std::path::PathBuf;

#[cfg(not(unix))]
use sysinfo::Pid;
#[cfg(not(unix))]
use sysinfo::ProcessRefreshKind;
#[cfg(not(unix))]
use sysinfo::ProcessesToUpdate;
#[cfg(not(unix))]
use sysinfo::Signal;
#[cfg(not(unix))]
use sysinfo::System;
use tui_pane::RunningTracker;

use crate::build_monitor::LiveOwnedRoot;
use crate::build_monitor::OwnedRootEvidence;
use crate::build_monitor::OwnedRootLifecycle;
use crate::process_observation::identity::ProcessIdentity;
#[cfg(not(test))]
use crate::process_observation::identity::StrongProcessIdentityRevalidation;
use crate::process_observation::identity::VerifiedProcessIdentity;
#[cfg(not(test))]
use crate::process_observation::identity::revalidate_strong_process_identity;
use crate::project::AbsolutePath;
use crate::tui::app::PendingClean;
use crate::tui::panes::PendingCiFetch;
use crate::tui::panes::PendingExampleRun;

/// A monotonically allocated identity for one Cargo Port-owned run.
///
/// The counter is [`NonZeroU64`], so a zero identity is unrepresentable and the
/// type carries a niche: no launch path can hand out a placeholder that later
/// correlates against a real run.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct OwnedRunId(NonZeroU64);

impl OwnedRunId {
    /// Name one owned-run identity directly, so classification tests can state
    /// owned-root evidence without driving a launch.
    #[cfg(test)]
    pub(crate) const fn for_test(owned_run_id: NonZeroU64) -> Self { Self(owned_run_id) }
}

/// Whether the monotonic counter still had an unused owned-run identity.
///
/// Exhaustion rejects the run rather than repeating an identity: late
/// messages, output joins, and cursors all correlate on [`OwnedRunId`], so a
/// reissued value would attach one run's output to another.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunIdAllocation {
    /// A fresh identity no earlier run has held.
    Allocated(OwnedRunId),
    /// Every identity the counter can represent has already been issued.
    Exhausted,
}

/// Opaque authority to signal the isolated group created for one verified
/// Cargo Port-owned root process.
#[derive(Debug)]
pub(crate) struct OwnedProcessGroupTerminationCapability {
    process_group_id:    u32,
    process_identity:    ProcessIdentity,
    #[cfg(test)]
    test_signal_outcome: OwnedProcessGroupSignalOutcome,
}

impl OwnedProcessGroupTerminationCapability {
    /// Consume identity evidence that was observed and revalidated before
    /// constructing group termination authority.
    pub(crate) const fn from_verified_root(
        verified_process_identity: VerifiedProcessIdentity,
    ) -> Self {
        let process_identity = verified_process_identity.into_process_identity();
        let process_group_id = process_identity.pid();
        Self {
            process_group_id,
            process_identity,
            #[cfg(test)]
            test_signal_outcome: OwnedProcessGroupSignalOutcome::Sent,
        }
    }

    /// The verified root identity, as observation. Handing this out does not
    /// hand out signaling authority: the capability itself never leaves.
    const fn root_identity(&self) -> &ProcessIdentity { &self.process_identity }

    #[cfg(not(test))]
    pub(crate) fn signal(&self) -> OwnedProcessGroupSignalOutcome {
        match revalidate_strong_process_identity(&self.process_identity) {
            StrongProcessIdentityRevalidation::Current => {
                signal_owned_process_group(self.process_group_id, &self.process_identity)
            },
            StrongProcessIdentityRevalidation::Replaced(_)
            | StrongProcessIdentityRevalidation::Unavailable(_) => {
                OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent
            },
        }
    }

    #[cfg(test)]
    pub(crate) const fn signal(&self) -> OwnedProcessGroupSignalOutcome { self.test_signal_outcome }

    const fn root_binding_is_consistent(&self) -> bool {
        self.process_group_id == self.process_identity.pid()
    }

    #[cfg(test)]
    const fn for_test(process_identity: ProcessIdentity) -> Self {
        Self {
            process_group_id: process_identity.pid(),
            process_identity,
            test_signal_outcome: OwnedProcessGroupSignalOutcome::Sent,
        }
    }

    #[cfg(test)]
    const fn with_test_signal_outcome(
        process_identity: ProcessIdentity,
        test_signal_outcome: OwnedProcessGroupSignalOutcome,
    ) -> Self {
        Self {
            process_group_id: process_identity.pid(),
            process_identity,
            test_signal_outcome,
        }
    }
}

/// What happened when identity-bound owned-group signaling was attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedProcessGroupSignalOutcome {
    Sent,
    IdentityNoLongerCurrent,
    SignalFailed,
}

#[cfg(all(unix, not(test)))]
fn signal_owned_process_group(
    process_group_id: u32,
    process_identity: &ProcessIdentity,
) -> OwnedProcessGroupSignalOutcome {
    let group_signal_sent = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{process_group_id}"))
        .status()
        .is_ok_and(|status| status.success());
    if group_signal_sent {
        return OwnedProcessGroupSignalOutcome::Sent;
    }

    if !matches!(
        revalidate_strong_process_identity(process_identity),
        StrongProcessIdentityRevalidation::Current
    ) {
        return OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent;
    }

    let root_signal_sent = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(process_identity.pid().to_string())
        .status()
        .is_ok_and(|status| status.success());

    if root_signal_sent {
        OwnedProcessGroupSignalOutcome::Sent
    } else {
        OwnedProcessGroupSignalOutcome::SignalFailed
    }
}

#[cfg(all(not(unix), not(test)))]
fn signal_owned_process_group(
    _: u32,
    process_identity: &ProcessIdentity,
) -> OwnedProcessGroupSignalOutcome {
    let mut system = System::new();
    let pid = Pid::from_u32(process_identity.pid());
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    if !matches!(
        revalidate_strong_process_identity(process_identity),
        StrongProcessIdentityRevalidation::Current
    ) {
        return OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent;
    }
    let signal_sent = system
        .process(pid)
        .is_some_and(|process| process.kill_with(Signal::Term).unwrap_or(false));
    if signal_sent {
        OwnedProcessGroupSignalOutcome::Sent
    } else {
        OwnedProcessGroupSignalOutcome::SignalFailed
    }
}

/// The sole owner of the one Cargo Port-owned run's lifecycle and output.
pub(crate) struct OwnedRun {
    next_id:   NonZeroU64,
    lifecycle: OwnedRunLifecycle,
}

impl OwnedRun {
    /// Construct the single process-lifetime owned-run slot.
    ///
    /// The counter starts at one and is never reset, so a second construction
    /// would reissue identities that messages still in the channel carry.
    /// Private visibility is what keeps [`Inflight::new`] the sole call site:
    /// no other module can construct a second slot.
    const fn new() -> Self {
        Self {
            next_id:   NonZeroU64::MIN,
            lifecycle: OwnedRunLifecycle::absent(),
        }
    }

    pub(crate) const fn lifecycle(&self) -> &OwnedRunLifecycle { &self.lifecycle }

    pub(crate) const fn identity(&self) -> OwnedRunIdentityRef<'_> { self.lifecycle.identity() }

    pub(crate) fn output(&self) -> &[String] {
        debug_assert!(
            self.lifecycle
                .output_identity_is_valid(self.output_identity())
        );
        self.lifecycle.output()
    }

    pub(crate) const fn output_identity(&self) -> OwnedRunOutputIdentityRef<'_> {
        self.lifecycle.output_identity()
    }

    pub(crate) fn output_is_empty(&self) -> bool { self.output().is_empty() }

    pub(crate) fn output_title(&self) -> OwnedRunOutputTitleRef<'_> {
        self.lifecycle.output_title()
    }

    pub(crate) fn running_label(&self) -> OwnedRunRunningLabelRef<'_> {
        self.lifecycle.running_label()
    }

    pub(crate) const fn is_running(&self) -> bool {
        matches!(self.lifecycle(), OwnedRunLifecycle::Running(_))
    }

    const fn allocate_id(&mut self) -> OwnedRunIdAllocation {
        let Some(next_id) = self.next_id.checked_add(1) else {
            return OwnedRunIdAllocation::Exhausted;
        };
        let owned_run_id = OwnedRunId(self.next_id);
        self.next_id = next_id;
        OwnedRunIdAllocation::Allocated(owned_run_id)
    }

    /// Start the counter near its ceiling so a test can reach exhaustion
    /// without issuing `u64::MAX` identities.
    #[cfg(test)]
    const fn seed_next_id_for_test(&mut self, next_id: NonZeroU64) { self.next_id = next_id; }

    fn queue(&mut self, pending_example_run: PendingExampleRun) -> OwnedRunLaunchAdmission {
        if matches!(
            self.lifecycle,
            OwnedRunLifecycle::Queued(_)
                | OwnedRunLifecycle::Starting(_)
                | OwnedRunLifecycle::Running(_)
                | OwnedRunLifecycle::Stopping(_)
        ) {
            return OwnedRunLaunchAdmission::AlreadyActive;
        }

        let OwnedRunIdAllocation::Allocated(owned_run_id) = self.allocate_id() else {
            return OwnedRunLaunchAdmission::IdentitiesExhausted;
        };
        let retained_output = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent())
            .into_retained_output();
        self.lifecycle = OwnedRunLifecycle::Queued(QueuedOwnedRun {
            owned_run_id,
            pending_example_run,
            retained_output,
        });
        OwnedRunLaunchAdmission::Queued(owned_run_id)
    }

    fn begin_launch(&mut self) -> OwnedRunLaunchStart {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        match lifecycle {
            OwnedRunLifecycle::Queued(queued_owned_run) => {
                let owned_run_id = queued_owned_run.owned_run_id;
                self.lifecycle =
                    OwnedRunLifecycle::Starting(StartingOwnedRun::from(queued_owned_run));
                OwnedRunLaunchStart::Starting(owned_run_id)
            },
            lifecycle => {
                self.lifecycle = lifecycle;
                OwnedRunLaunchStart::NoQueuedRun
            },
        }
    }

    fn starting_request(&self, owned_run_id: OwnedRunId) -> OwnedRunStartingRequest<'_> {
        match &self.lifecycle {
            OwnedRunLifecycle::Starting(starting_owned_run)
                if starting_owned_run.owned_run_id == owned_run_id =>
            {
                OwnedRunStartingRequest::Starting(&starting_owned_run.pending_example_run)
            },
            _ => OwnedRunStartingRequest::NoMatchingStartingRun,
        }
    }

    /// Immutable evidence about the one Cargo Port-owned Cargo root, for build
    /// classification. A stopping run still has live compiler descendants, so
    /// it is reported as an owned root with a stopping lifecycle rather than as
    /// no root at all.
    pub(crate) fn owned_root_evidence(&self) -> OwnedRootEvidence {
        match &self.lifecycle {
            OwnedRunLifecycle::Running(running_owned_run) => {
                OwnedRootEvidence::Root(LiveOwnedRoot::new(
                    running_owned_run.owned_run_id,
                    running_owned_run
                        .owned_process_group_termination_capability
                        .root_identity()
                        .clone(),
                    running_owned_run.launch_directory.clone(),
                    OwnedRootLifecycle::Live,
                ))
            },
            OwnedRunLifecycle::Stopping(stopping_owned_run) => {
                OwnedRootEvidence::Root(LiveOwnedRoot::new(
                    stopping_owned_run.owned_run_id,
                    stopping_owned_run
                        .owned_process_group_termination_capability
                        .root_identity()
                        .clone(),
                    stopping_owned_run.launch_directory.clone(),
                    OwnedRootLifecycle::Stopping,
                ))
            },
            OwnedRunLifecycle::Absent(_)
            | OwnedRunLifecycle::Queued(_)
            | OwnedRunLifecycle::Starting(_)
            | OwnedRunLifecycle::RetainedSuccess(_)
            | OwnedRunLifecycle::GoneAfterSignal(_)
            | OwnedRunLifecycle::Failed(_) => OwnedRootEvidence::NoLiveRoot,
        }
    }

    fn activate(
        &mut self,
        owned_run_id: OwnedRunId,
        owned_process_group_termination_capability: OwnedProcessGroupTerminationCapability,
    ) -> OwnedRunActivation {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        match lifecycle {
            OwnedRunLifecycle::Starting(starting_owned_run)
                if starting_owned_run.owned_run_id == owned_run_id =>
            {
                let mode = starting_owned_run.pending_example_run.build_mode.label();
                let launch_directory =
                    PathBuf::from(starting_owned_run.pending_example_run.abs_path);
                let display_path = starting_owned_run.pending_example_run.display_path;
                let target_name = starting_owned_run.pending_example_run.target_name;
                self.lifecycle = OwnedRunLifecycle::Running(RunningOwnedRun {
                    owned_run_id,
                    running_label: format!("{display_path}{mode}"),
                    launch_directory,
                    retained_output: OwnedRunRetainedOutput::named(
                        owned_run_id,
                        display_path,
                        vec![format!("Building {target_name}{mode}...")],
                    ),
                    owned_process_group_termination_capability,
                });
                OwnedRunActivation::Activated
            },
            lifecycle => {
                self.lifecycle = lifecycle;
                OwnedRunActivation::NoMatchingStartingRun
            },
        }
    }

    fn fail_starting(&mut self, owned_run_id: OwnedRunId, failure_message: String) {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        match lifecycle {
            OwnedRunLifecycle::Starting(starting_owned_run)
                if starting_owned_run.owned_run_id == owned_run_id =>
            {
                self.lifecycle = OwnedRunLifecycle::Failed(FailedOwnedRun {
                    owned_run_id,
                    retained_output: OwnedRunRetainedOutput::named(
                        owned_run_id,
                        starting_owned_run.pending_example_run.display_path,
                        vec![failure_message],
                    ),
                });
            },
            lifecycle => self.lifecycle = lifecycle,
        }
    }

    fn record_output(&mut self, owned_run_id: OwnedRunId, line: String) -> OwnedRunMessageUpdate {
        match &mut self.lifecycle {
            OwnedRunLifecycle::Running(running_owned_run)
                if running_owned_run.owned_run_id == owned_run_id =>
            {
                running_owned_run.retained_output.lines.push(line);
                OwnedRunMessageUpdate::Applied
            },
            OwnedRunLifecycle::Stopping(stopping_owned_run)
                if stopping_owned_run.owned_run_id == owned_run_id =>
            {
                stopping_owned_run.retained_output.lines.push(line);
                OwnedRunMessageUpdate::Applied
            },
            _ => OwnedRunMessageUpdate::Ignored,
        }
    }

    fn record_progress(&mut self, owned_run_id: OwnedRunId, line: String) -> OwnedRunMessageUpdate {
        match &mut self.lifecycle {
            OwnedRunLifecycle::Running(running_owned_run)
                if running_owned_run.owned_run_id == owned_run_id =>
            {
                replace_progress_line(&mut running_owned_run.retained_output.lines, line);
                OwnedRunMessageUpdate::Applied
            },
            OwnedRunLifecycle::Stopping(stopping_owned_run)
                if stopping_owned_run.owned_run_id == owned_run_id =>
            {
                replace_progress_line(&mut stopping_owned_run.retained_output.lines, line);
                OwnedRunMessageUpdate::Applied
            },
            _ => OwnedRunMessageUpdate::Ignored,
        }
    }

    fn acknowledge_started(&self, owned_run_id: OwnedRunId) -> OwnedRunMessageUpdate {
        match self.identity() {
            OwnedRunIdentityRef::Current(current_owned_run_id)
                if *current_owned_run_id == owned_run_id && self.is_running() =>
            {
                OwnedRunMessageUpdate::Applied
            },
            OwnedRunIdentityRef::Absent | OwnedRunIdentityRef::Current(_) => {
                OwnedRunMessageUpdate::Ignored
            },
        }
    }

    fn finish(&mut self, owned_run_id: OwnedRunId) -> OwnedRunMessageUpdate {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        match lifecycle {
            OwnedRunLifecycle::Running(mut running_owned_run)
                if running_owned_run.owned_run_id == owned_run_id =>
            {
                append_marker_if_absent(&mut running_owned_run.retained_output.lines, DONE_MARKER);
                self.lifecycle = OwnedRunLifecycle::RetainedSuccess(RetainedOwnedRun {
                    owned_run_id,
                    retained_output: running_owned_run.retained_output,
                });
                OwnedRunMessageUpdate::Applied
            },
            OwnedRunLifecycle::Stopping(mut stopping_owned_run)
                if stopping_owned_run.owned_run_id == owned_run_id =>
            {
                debug_assert!(
                    stopping_owned_run
                        .owned_process_group_termination_capability
                        .root_binding_is_consistent()
                );
                move_marker_to_end(&mut stopping_owned_run.retained_output.lines, KILLED_MARKER);
                self.lifecycle = OwnedRunLifecycle::GoneAfterSignal(RetainedOwnedRun {
                    owned_run_id,
                    retained_output: stopping_owned_run.retained_output,
                });
                OwnedRunMessageUpdate::Applied
            },
            lifecycle => {
                self.lifecycle = lifecycle;
                OwnedRunMessageUpdate::Ignored
            },
        }
    }

    const fn termination(&self) -> OwnedRunTermination<'_> {
        match &self.lifecycle {
            OwnedRunLifecycle::Running(running_owned_run) => OwnedRunTermination::Available {
                owned_run_id:                               running_owned_run.owned_run_id,
                owned_process_group_termination_capability: &running_owned_run
                    .owned_process_group_termination_capability,
            },
            _ => OwnedRunTermination::NoRunningRun,
        }
    }

    fn begin_stopping(&mut self, owned_run_id: OwnedRunId) -> OwnedRunStopTransition {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        match lifecycle {
            OwnedRunLifecycle::Running(mut running_owned_run)
                if running_owned_run.owned_run_id == owned_run_id =>
            {
                append_marker_if_absent(
                    &mut running_owned_run.retained_output.lines,
                    KILLED_MARKER,
                );
                self.lifecycle =
                    OwnedRunLifecycle::Stopping(StoppingOwnedRun::from(running_owned_run));
                OwnedRunStopTransition::Stopping
            },
            lifecycle => {
                self.lifecycle = lifecycle;
                OwnedRunStopTransition::NoMatchingRunningRun
            },
        }
    }

    fn clear_output(&mut self) {
        match &mut self.lifecycle {
            OwnedRunLifecycle::Queued(queued_owned_run) => {
                queued_owned_run.retained_output = OwnedRunRetainedOutput::uncorrelated(Vec::new());
            },
            OwnedRunLifecycle::Starting(starting_owned_run) => {
                starting_owned_run.retained_output =
                    OwnedRunRetainedOutput::uncorrelated(Vec::new());
            },
            OwnedRunLifecycle::Running(running_owned_run) => {
                running_owned_run.retained_output = OwnedRunRetainedOutput::correlated_unnamed(
                    running_owned_run.owned_run_id,
                    Vec::new(),
                );
            },
            OwnedRunLifecycle::Stopping(stopping_owned_run) => {
                stopping_owned_run.retained_output = OwnedRunRetainedOutput::correlated_unnamed(
                    stopping_owned_run.owned_run_id,
                    Vec::new(),
                );
            },
            OwnedRunLifecycle::Absent(_)
            | OwnedRunLifecycle::RetainedSuccess(_)
            | OwnedRunLifecycle::GoneAfterSignal(_)
            | OwnedRunLifecycle::Failed(_) => {
                self.lifecycle = OwnedRunLifecycle::absent();
            },
        }
    }

    #[cfg(test)]
    #[allow(
        clippy::panic,
        reason = "a test helper should fail loudly on an exhausted counter"
    )]
    fn set_output_for_test(&mut self, lines: Vec<String>) {
        let OwnedRunIdAllocation::Allocated(owned_run_id) = self.allocate_id() else {
            panic!("owned-run identities should not be exhausted in tests");
        };
        self.lifecycle = OwnedRunLifecycle::RetainedSuccess(RetainedOwnedRun {
            owned_run_id,
            retained_output: OwnedRunRetainedOutput::correlated_unnamed(owned_run_id, lines),
        });
    }

    #[cfg(test)]
    const fn output_mut_for_test(&mut self) -> &mut Vec<String> {
        match &mut self.lifecycle {
            OwnedRunLifecycle::Absent(retained_output) => &mut retained_output.lines,
            OwnedRunLifecycle::Queued(queued_owned_run) => {
                &mut queued_owned_run.retained_output.lines
            },
            OwnedRunLifecycle::Starting(starting_owned_run) => {
                &mut starting_owned_run.retained_output.lines
            },
            OwnedRunLifecycle::Running(running_owned_run) => {
                &mut running_owned_run.retained_output.lines
            },
            OwnedRunLifecycle::Stopping(stopping_owned_run) => {
                &mut stopping_owned_run.retained_output.lines
            },
            OwnedRunLifecycle::RetainedSuccess(retained_owned_run)
            | OwnedRunLifecycle::GoneAfterSignal(retained_owned_run) => {
                &mut retained_owned_run.retained_output.lines
            },
            OwnedRunLifecycle::Failed(failed_owned_run) => {
                &mut failed_owned_run.retained_output.lines
            },
        }
    }

    #[cfg(test)]
    #[allow(
        clippy::panic,
        reason = "a test helper should fail loudly on an exhausted counter"
    )]
    fn set_running_for_test(&mut self, running_label: Option<String>) {
        let Some(running_label) = running_label else {
            self.set_not_running_for_test();
            return;
        };
        let retained_output = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent())
            .into_retained_output();
        let OwnedRunIdAllocation::Allocated(owned_run_id) = self.allocate_id() else {
            panic!("owned-run identities should not be exhausted in tests");
        };
        let mut retained_output = retained_output;
        retained_output.correlate_to(owned_run_id);
        let process_identity = ProcessIdentity::for_test(1, owned_run_id.0.get());
        let owned_process_group_termination_capability =
            OwnedProcessGroupTerminationCapability::for_test(process_identity);
        self.lifecycle = OwnedRunLifecycle::Running(RunningOwnedRun {
            owned_run_id,
            running_label: running_label.clone(),
            launch_directory: PathBuf::new(),
            retained_output: retained_output.with_missing_title_replaced_by(running_label),
            owned_process_group_termination_capability,
        });
    }

    #[cfg(test)]
    fn set_not_running_for_test(&mut self) {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        self.lifecycle = match lifecycle {
            OwnedRunLifecycle::Running(running_owned_run) => {
                OwnedRunLifecycle::RetainedSuccess(RetainedOwnedRun {
                    owned_run_id:    running_owned_run.owned_run_id,
                    retained_output: running_owned_run.retained_output,
                })
            },
            lifecycle => lifecycle,
        };
    }

    #[cfg(test)]
    fn set_title_for_test(&mut self, title: Option<String>) {
        self.lifecycle.set_title_for_test(title);
    }
}

/// The state-specific lifecycle of a Cargo Port-owned run.
pub(crate) enum OwnedRunLifecycle {
    Absent(OwnedRunRetainedOutput),
    Queued(QueuedOwnedRun),
    Starting(StartingOwnedRun),
    Running(RunningOwnedRun),
    Stopping(StoppingOwnedRun),
    RetainedSuccess(RetainedOwnedRun),
    GoneAfterSignal(RetainedOwnedRun),
    Failed(FailedOwnedRun),
}

impl OwnedRunLifecycle {
    const fn absent() -> Self { Self::Absent(OwnedRunRetainedOutput::uncorrelated(Vec::new())) }

    const fn identity(&self) -> OwnedRunIdentityRef<'_> {
        match self {
            Self::Absent(_) => OwnedRunIdentityRef::Absent,
            Self::Queued(queued_owned_run) => {
                OwnedRunIdentityRef::Current(&queued_owned_run.owned_run_id)
            },
            Self::Starting(starting_owned_run) => {
                OwnedRunIdentityRef::Current(&starting_owned_run.owned_run_id)
            },
            Self::Running(running_owned_run) => {
                OwnedRunIdentityRef::Current(&running_owned_run.owned_run_id)
            },
            Self::Stopping(stopping_owned_run) => {
                OwnedRunIdentityRef::Current(&stopping_owned_run.owned_run_id)
            },
            Self::RetainedSuccess(retained_owned_run)
            | Self::GoneAfterSignal(retained_owned_run) => {
                OwnedRunIdentityRef::Current(&retained_owned_run.owned_run_id)
            },
            Self::Failed(failed_owned_run) => {
                OwnedRunIdentityRef::Current(&failed_owned_run.owned_run_id)
            },
        }
    }

    fn output(&self) -> &[String] {
        match self {
            Self::Absent(retained_output) => &retained_output.lines,
            Self::Queued(queued_owned_run) => &queued_owned_run.retained_output.lines,
            Self::Starting(starting_owned_run) => &starting_owned_run.retained_output.lines,
            Self::Running(running_owned_run) => &running_owned_run.retained_output.lines,
            Self::Stopping(stopping_owned_run) => &stopping_owned_run.retained_output.lines,
            Self::RetainedSuccess(retained_owned_run)
            | Self::GoneAfterSignal(retained_owned_run) => {
                &retained_owned_run.retained_output.lines
            },
            Self::Failed(failed_owned_run) => &failed_owned_run.retained_output.lines,
        }
    }

    const fn output_identity(&self) -> OwnedRunOutputIdentityRef<'_> {
        match self {
            Self::Absent(retained_output) => retained_output.identity.as_ref(),
            Self::Queued(queued_owned_run) => queued_owned_run.retained_output.identity.as_ref(),
            Self::Starting(starting_owned_run) => {
                starting_owned_run.retained_output.identity.as_ref()
            },
            Self::Running(running_owned_run) => running_owned_run.retained_output.identity.as_ref(),
            Self::Stopping(stopping_owned_run) => {
                stopping_owned_run.retained_output.identity.as_ref()
            },
            Self::RetainedSuccess(retained_owned_run)
            | Self::GoneAfterSignal(retained_owned_run) => {
                retained_owned_run.retained_output.identity.as_ref()
            },
            Self::Failed(failed_owned_run) => failed_owned_run.retained_output.identity.as_ref(),
        }
    }

    fn output_identity_is_valid(&self, output_identity: OwnedRunOutputIdentityRef<'_>) -> bool {
        match (self, output_identity) {
            (
                Self::Absent(_) | Self::Queued(_) | Self::Starting(_),
                OwnedRunOutputIdentityRef::Uncorrelated,
            ) => true,
            (
                Self::Queued(queued_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => output_owned_run_id < &queued_owned_run.owned_run_id,
            (
                Self::Starting(starting_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => output_owned_run_id < &starting_owned_run.owned_run_id,
            (
                Self::Running(running_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => output_owned_run_id == &running_owned_run.owned_run_id,
            (
                Self::Stopping(stopping_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => output_owned_run_id == &stopping_owned_run.owned_run_id,
            (
                Self::RetainedSuccess(retained_owned_run)
                | Self::GoneAfterSignal(retained_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => output_owned_run_id == &retained_owned_run.owned_run_id,
            (
                Self::Failed(failed_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => output_owned_run_id == &failed_owned_run.owned_run_id,
            (
                Self::Running(_)
                | Self::Stopping(_)
                | Self::RetainedSuccess(_)
                | Self::GoneAfterSignal(_)
                | Self::Failed(_),
                OwnedRunOutputIdentityRef::Uncorrelated,
            )
            | (Self::Absent(_), OwnedRunOutputIdentityRef::Correlated(_)) => false,
        }
    }

    fn output_title(&self) -> OwnedRunOutputTitleRef<'_> {
        match self {
            Self::Absent(_) => OwnedRunOutputTitleRef::Unavailable,
            Self::Queued(queued_owned_run) => queued_owned_run.retained_output.title.as_ref(),
            Self::Starting(starting_owned_run) => starting_owned_run.retained_output.title.as_ref(),
            Self::Running(running_owned_run) => running_owned_run.retained_output.title.as_ref(),
            Self::Stopping(stopping_owned_run) => stopping_owned_run.retained_output.title.as_ref(),
            Self::RetainedSuccess(retained_owned_run)
            | Self::GoneAfterSignal(retained_owned_run) => {
                retained_owned_run.retained_output.title.as_ref()
            },
            Self::Failed(failed_owned_run) => failed_owned_run.retained_output.title.as_ref(),
        }
    }

    fn running_label(&self) -> OwnedRunRunningLabelRef<'_> {
        match self {
            Self::Running(running_owned_run) => {
                OwnedRunRunningLabelRef::Running(&running_owned_run.running_label)
            },
            Self::Absent(_)
            | Self::Queued(_)
            | Self::Starting(_)
            | Self::Stopping(_)
            | Self::RetainedSuccess(_)
            | Self::GoneAfterSignal(_)
            | Self::Failed(_) => OwnedRunRunningLabelRef::NotRunning,
        }
    }

    fn into_retained_output(self) -> OwnedRunRetainedOutput {
        match self {
            Self::Absent(retained_output) => retained_output,
            Self::Queued(queued_owned_run) => queued_owned_run.retained_output,
            Self::Starting(starting_owned_run) => starting_owned_run.retained_output,
            Self::Running(running_owned_run) => running_owned_run.retained_output,
            Self::Stopping(stopping_owned_run) => stopping_owned_run.retained_output,
            Self::RetainedSuccess(retained_owned_run)
            | Self::GoneAfterSignal(retained_owned_run) => retained_owned_run.retained_output,
            Self::Failed(failed_owned_run) => failed_owned_run.retained_output,
        }
    }

    #[cfg(test)]
    fn set_title_for_test(&mut self, title: Option<String>) {
        let title = OwnedRunOutputTitle::from_optional_for_test(title);
        match self {
            Self::Absent(_) => {},
            Self::Queued(queued_owned_run) => queued_owned_run.retained_output.title = title,
            Self::Starting(starting_owned_run) => starting_owned_run.retained_output.title = title,
            Self::Running(running_owned_run) => running_owned_run.retained_output.title = title,
            Self::Stopping(stopping_owned_run) => stopping_owned_run.retained_output.title = title,
            Self::RetainedSuccess(retained_owned_run)
            | Self::GoneAfterSignal(retained_owned_run) => {
                retained_owned_run.retained_output.title = title;
            },
            Self::Failed(failed_owned_run) => failed_owned_run.retained_output.title = title,
        }
    }
}

/// A queued owned run has launch data but no observed process root.
pub(crate) struct QueuedOwnedRun {
    owned_run_id:        OwnedRunId,
    pending_example_run: PendingExampleRun,
    retained_output:     OwnedRunRetainedOutput,
}

/// A starting owned run has launch data but no observed process root.
pub(crate) struct StartingOwnedRun {
    owned_run_id:        OwnedRunId,
    pending_example_run: PendingExampleRun,
    retained_output:     OwnedRunRetainedOutput,
}

impl From<QueuedOwnedRun> for StartingOwnedRun {
    fn from(queued_owned_run: QueuedOwnedRun) -> Self {
        Self {
            owned_run_id:        queued_owned_run.owned_run_id,
            pending_example_run: queued_owned_run.pending_example_run,
            retained_output:     queued_owned_run.retained_output,
        }
    }
}

/// A running owned run has a verified root and group termination authority.
pub(crate) struct RunningOwnedRun {
    owned_run_id:                               OwnedRunId,
    running_label:                              String,
    launch_directory:                           PathBuf,
    retained_output:                            OwnedRunRetainedOutput,
    owned_process_group_termination_capability: OwnedProcessGroupTerminationCapability,
}

/// A stopped-requested run retains verified root authority until the process
/// reports completion.
pub(crate) struct StoppingOwnedRun {
    owned_run_id:                               OwnedRunId,
    launch_directory:                           PathBuf,
    retained_output:                            OwnedRunRetainedOutput,
    owned_process_group_termination_capability: OwnedProcessGroupTerminationCapability,
}

impl From<RunningOwnedRun> for StoppingOwnedRun {
    fn from(running_owned_run: RunningOwnedRun) -> Self {
        Self {
            owned_run_id:                               running_owned_run.owned_run_id,
            launch_directory:                           running_owned_run.launch_directory,
            retained_output:                            running_owned_run.retained_output,
            owned_process_group_termination_capability: running_owned_run
                .owned_process_group_termination_capability,
        }
    }
}

/// Completed output that remains available after the process lifecycle ends.
pub(crate) struct RetainedOwnedRun {
    owned_run_id:    OwnedRunId,
    retained_output: OwnedRunRetainedOutput,
}

/// A failed launch retains its diagnostic output without process authority.
pub(crate) struct FailedOwnedRun {
    owned_run_id:    OwnedRunId,
    retained_output: OwnedRunRetainedOutput,
}

pub(crate) struct OwnedRunRetainedOutput {
    identity: OwnedRunOutputIdentity,
    title:    OwnedRunOutputTitle,
    lines:    Vec<String>,
}

impl OwnedRunRetainedOutput {
    const fn named(owned_run_id: OwnedRunId, title: String, lines: Vec<String>) -> Self {
        Self {
            identity: OwnedRunOutputIdentity::Correlated(owned_run_id),
            title: OwnedRunOutputTitle::Named(title),
            lines,
        }
    }

    const fn correlated_unnamed(owned_run_id: OwnedRunId, lines: Vec<String>) -> Self {
        Self {
            identity: OwnedRunOutputIdentity::Correlated(owned_run_id),
            title: OwnedRunOutputTitle::Unavailable,
            lines,
        }
    }

    const fn uncorrelated(lines: Vec<String>) -> Self {
        Self {
            identity: OwnedRunOutputIdentity::Uncorrelated,
            title: OwnedRunOutputTitle::Unavailable,
            lines,
        }
    }

    #[cfg(test)]
    fn with_missing_title_replaced_by(mut self, title: String) -> Self {
        if matches!(self.title, OwnedRunOutputTitle::Unavailable) {
            self.title = OwnedRunOutputTitle::Named(title);
        }
        self
    }

    #[cfg(test)]
    const fn correlate_to(&mut self, owned_run_id: OwnedRunId) {
        self.identity = OwnedRunOutputIdentity::Correlated(owned_run_id);
    }
}

enum OwnedRunOutputIdentity {
    Correlated(OwnedRunId),
    Uncorrelated,
}

impl OwnedRunOutputIdentity {
    const fn as_ref(&self) -> OwnedRunOutputIdentityRef<'_> {
        match self {
            Self::Correlated(owned_run_id) => OwnedRunOutputIdentityRef::Correlated(owned_run_id),
            Self::Uncorrelated => OwnedRunOutputIdentityRef::Uncorrelated,
        }
    }
}

enum OwnedRunOutputTitle {
    Named(String),
    Unavailable,
}

impl OwnedRunOutputTitle {
    fn as_ref(&self) -> OwnedRunOutputTitleRef<'_> {
        match self {
            Self::Named(title) => OwnedRunOutputTitleRef::Named(title),
            Self::Unavailable => OwnedRunOutputTitleRef::Unavailable,
        }
    }

    #[cfg(test)]
    fn from_optional_for_test(title: Option<String>) -> Self {
        title.map_or(Self::Unavailable, Self::Named)
    }
}

/// Borrowed identity availability for the current owned-run lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunIdentityRef<'a> {
    Absent,
    Current(&'a OwnedRunId),
}

/// The run that produced the currently retained output, when one exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunOutputIdentityRef<'a> {
    Correlated(&'a OwnedRunId),
    Uncorrelated,
}

/// Borrowed title availability for retained or live owned output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunOutputTitleRef<'a> {
    Named(&'a str),
    Unavailable,
}

/// Borrowed live label availability for the current owned-run lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunRunningLabelRef<'a> {
    Running(&'a str),
    NotRunning,
}

/// Whether the sole owned-run slot accepted a launch request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(crate) enum OwnedRunLaunchAdmission {
    Queued(OwnedRunId),
    AlreadyActive,
    /// The monotonic identity counter has no unused value left, so the launch
    /// is refused rather than correlated against an identity already in use.
    IdentitiesExhausted,
}

/// Whether a queued owned run entered its synchronous launch boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunLaunchStart {
    Starting(OwnedRunId),
    NoQueuedRun,
}

/// The launch request present for a matching `Starting` lifecycle.
pub(crate) enum OwnedRunStartingRequest<'a> {
    Starting(&'a PendingExampleRun),
    NoMatchingStartingRun,
}

/// Whether activation found the starting lifecycle it was asked to promote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunActivation {
    Activated,
    NoMatchingStartingRun,
}

/// Whether one correlated background message changed the current lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunMessageUpdate {
    Applied,
    Ignored,
}

/// The current run's identity-bound group termination authority.
pub(crate) enum OwnedRunTermination<'a> {
    Available {
        owned_run_id:                               OwnedRunId,
        owned_process_group_termination_capability: &'a OwnedProcessGroupTerminationCapability,
    },
    NoRunningRun,
}

/// Whether a signaled run entered its stopping lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunStopTransition {
    Stopping,
    NoMatchingRunningRun,
}

const DONE_MARKER: &str = "── done ──";
const KILLED_MARKER: &str = "── killed ──";

fn replace_progress_line(lines: &mut Vec<String>, line: String) {
    if let Some(last) = lines.last_mut() {
        *last = line;
    } else {
        lines.push(line);
    }
}

fn append_marker_if_absent(lines: &mut Vec<String>, marker: &str) {
    if !lines
        .last()
        .is_some_and(|line| line == DONE_MARKER || line == KILLED_MARKER)
    {
        lines.push(marker.to_string());
    }
}

fn move_marker_to_end(lines: &mut Vec<String>, marker: &str) {
    lines.retain(|line| line != marker);
    lines.push(marker.to_string());
}

/// Owns App's in-flight bookkeeping. App holds a single
/// `inflight: Inflight`.
pub(crate) struct Inflight {
    /// In-flight cargo clean state — same lifecycle as
    /// `Lint::running` and `Github::running`.
    clean:            RunningTracker<AbsolutePath>,
    pending_cleans:   VecDeque<PendingClean>,
    pending_ci_fetch: Option<PendingCiFetch>,
    owned_run:        OwnedRun,
}

impl Inflight {
    pub(crate) fn new() -> Self {
        Self {
            clean:            RunningTracker::new(),
            pending_cleans:   VecDeque::new(),
            pending_ci_fetch: None,
            owned_run:        OwnedRun::new(),
        }
    }

    // ── running clean tracker ───────────────────────────────────────

    pub(crate) const fn clean(&self) -> &RunningTracker<AbsolutePath> { &self.clean }

    pub(crate) const fn clean_mut(&mut self) -> &mut RunningTracker<AbsolutePath> {
        &mut self.clean
    }

    /// Whether a cargo clean or owned run is in flight, so the render loop
    /// should keep ticking to advance their spinners.
    pub(crate) fn needs_animation(&self) -> bool {
        !self.clean().is_empty() || self.owned_run.is_running()
    }

    // ── pending queues ──────────────────────────────────────────────

    pub(crate) const fn pending_cleans_mut(&mut self) -> &mut VecDeque<PendingClean> {
        &mut self.pending_cleans
    }

    pub(crate) fn set_pending_ci_fetch(&mut self, fetch: PendingCiFetch) {
        self.pending_ci_fetch = Some(fetch);
    }

    /// Test-only inspection accessor — production paths consume
    /// the slot via [`Self::take_pending_ci_fetch`].
    #[cfg(test)]
    pub(crate) const fn pending_ci_fetch_ref(&self) -> Option<&PendingCiFetch> {
        self.pending_ci_fetch.as_ref()
    }

    pub(crate) const fn take_pending_ci_fetch(&mut self) -> Option<PendingCiFetch> {
        self.pending_ci_fetch.take()
    }

    pub(crate) fn clear_pending_ci_fetch(&mut self) { self.pending_ci_fetch = None; }

    // ── owned run ───────────────────────────────────────────────────

    pub(crate) const fn owned_run(&self) -> &OwnedRun { &self.owned_run }

    pub(crate) fn queue_owned_run(
        &mut self,
        pending_example_run: PendingExampleRun,
    ) -> OwnedRunLaunchAdmission {
        self.owned_run.queue(pending_example_run)
    }

    pub(crate) fn begin_owned_run_launch(&mut self) -> OwnedRunLaunchStart {
        self.owned_run.begin_launch()
    }

    pub(crate) fn starting_owned_run_request(
        &self,
        owned_run_id: OwnedRunId,
    ) -> OwnedRunStartingRequest<'_> {
        self.owned_run.starting_request(owned_run_id)
    }

    pub(crate) fn activate_owned_run(
        &mut self,
        owned_run_id: OwnedRunId,
        owned_process_group_termination_capability: OwnedProcessGroupTerminationCapability,
    ) -> OwnedRunActivation {
        self.owned_run
            .activate(owned_run_id, owned_process_group_termination_capability)
    }

    pub(crate) fn fail_owned_run_start(
        &mut self,
        owned_run_id: OwnedRunId,
        failure_message: String,
    ) {
        self.owned_run.fail_starting(owned_run_id, failure_message);
    }

    pub(crate) fn record_owned_run_output(
        &mut self,
        owned_run_id: OwnedRunId,
        line: String,
    ) -> OwnedRunMessageUpdate {
        self.owned_run.record_output(owned_run_id, line)
    }

    pub(crate) fn record_owned_run_progress(
        &mut self,
        owned_run_id: OwnedRunId,
        line: String,
    ) -> OwnedRunMessageUpdate {
        self.owned_run.record_progress(owned_run_id, line)
    }

    pub(crate) fn acknowledge_owned_run_started(
        &self,
        owned_run_id: OwnedRunId,
    ) -> OwnedRunMessageUpdate {
        self.owned_run.acknowledge_started(owned_run_id)
    }

    pub(crate) fn finish_owned_run(&mut self, owned_run_id: OwnedRunId) -> OwnedRunMessageUpdate {
        self.owned_run.finish(owned_run_id)
    }

    pub(crate) const fn owned_run_termination(&self) -> OwnedRunTermination<'_> {
        self.owned_run.termination()
    }

    pub(crate) fn mark_owned_run_stopping(
        &mut self,
        owned_run_id: OwnedRunId,
    ) -> OwnedRunStopTransition {
        self.owned_run.begin_stopping(owned_run_id)
    }

    pub(crate) fn clear_owned_run_output(&mut self) { self.owned_run.clear_output(); }

    #[cfg(test)]
    pub(crate) fn take_pending_example_run(&mut self) -> Option<PendingExampleRun> {
        let OwnedRunLaunchStart::Starting(owned_run_id) = self.begin_owned_run_launch() else {
            return None;
        };
        match self.starting_owned_run_request(owned_run_id) {
            OwnedRunStartingRequest::Starting(pending_example_run) => {
                Some(pending_example_run.clone())
            },
            OwnedRunStartingRequest::NoMatchingStartingRun => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn example_running(&self) -> Option<&str> {
        match self.owned_run.running_label() {
            OwnedRunRunningLabelRef::Running(running_label) => Some(running_label),
            OwnedRunRunningLabelRef::NotRunning => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_example_running(&mut self, running: Option<String>) {
        self.owned_run.set_running_for_test(running);
    }

    #[cfg(test)]
    pub(crate) fn set_example_title(&mut self, title: Option<String>) {
        self.owned_run.set_title_for_test(title);
    }

    #[cfg(test)]
    pub(crate) fn example_output(&self) -> &[String] { self.owned_run.output() }

    #[cfg(test)]
    pub(crate) const fn example_output_mut(&mut self) -> &mut Vec<String> {
        self.owned_run.output_mut_for_test()
    }

    #[cfg(test)]
    pub(crate) fn set_example_output(&mut self, output: Vec<String>) {
        self.owned_run.set_output_for_test(output);
    }

    #[cfg(test)]
    pub(crate) fn apply_example_progress(&mut self, line: String) {
        replace_progress_line(self.owned_run.output_mut_for_test(), line);
    }

    #[cfg(test)]
    pub(crate) fn append_done_marker(&mut self) {
        append_marker_if_absent(self.owned_run.output_mut_for_test(), DONE_MARKER);
    }

    #[cfg(test)]
    pub(crate) fn mark_run_killed(&mut self) {
        let OwnedRunTermination::Available { owned_run_id, .. } = self.owned_run_termination()
        else {
            return;
        };
        let _ = self.mark_owned_run_stopping(owned_run_id);
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::Instant;

    use tui_pane::ToastTaskId;

    use super::*;
    use crate::tui::panes::BuildMode;
    use crate::tui::panes::CargoPackageInvocation;
    use crate::tui::panes::CiFetchKind;
    use crate::tui::panes::RunTargetKind;

    fn fresh() -> Inflight { Inflight::new() }

    #[test]
    fn exhausted_owned_run_identities_reject_the_launch_without_reissuing_one() {
        let mut owned_run = OwnedRun::new();
        owned_run.seed_next_id_for_test(
            NonZeroU64::new(u64::MAX - 1).expect("a seeded counter should be non-zero"),
        );

        owned_run.set_running_for_test(Some("last allocatable run".to_string()));
        owned_run.set_not_running_for_test();
        let last_owned_run_id =
            OwnedRunId(NonZeroU64::new(u64::MAX - 1).expect("the issued identity is non-zero"));

        assert_eq!(
            owned_run.queue(pending_example_run()),
            OwnedRunLaunchAdmission::IdentitiesExhausted
        );
        assert!(matches!(
            owned_run.identity(),
            OwnedRunIdentityRef::Current(current_owned_run_id)
                if *current_owned_run_id == last_owned_run_id
        ));
    }

    fn abs(path: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(path)) }

    fn pending_example_run() -> PendingExampleRun {
        PendingExampleRun {
            abs_path:                 "/tmp/demo".to_string(),
            target_name:              "demo".to_string(),
            display_path:             "demo".to_string(),
            cargo_package_invocation: CargoPackageInvocation::WorkspaceDefault,
            run_target_kind:          RunTargetKind::Binary,
            build_mode:               BuildMode::Debug,
            required_features:        Vec::new(),
        }
    }

    fn queue_owned_run(inflight: &mut Inflight) -> OwnedRunId {
        let owned_run_launch_admission = inflight.queue_owned_run(pending_example_run());
        assert!(matches!(
            owned_run_launch_admission,
            OwnedRunLaunchAdmission::Queued(_)
        ));
        match owned_run_launch_admission {
            OwnedRunLaunchAdmission::Queued(owned_run_id) => owned_run_id,
            OwnedRunLaunchAdmission::AlreadyActive
            | OwnedRunLaunchAdmission::IdentitiesExhausted => {
                panic!("a fresh owned-run slot should queue a launch")
            },
        }
    }

    #[test]
    fn queued_and_starting_lifecycles_do_not_fabricate_process_authority() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Queued(_)
        ));
        assert!(matches!(
            inflight.owned_run_termination(),
            OwnedRunTermination::NoRunningRun
        ));

        assert_eq!(
            inflight.begin_owned_run_launch(),
            OwnedRunLaunchStart::Starting(owned_run_id)
        );
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Starting(_)
        ));
        assert!(matches!(
            inflight.owned_run_termination(),
            OwnedRunTermination::NoRunningRun
        ));
    }

    #[test]
    fn identity_unavailable_launch_keeps_no_live_termination_authority() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        inflight.fail_owned_run_start(
            owned_run_id,
            "Failed to establish a verified process identity".to_string(),
        );

        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Failed(_)
        ));
        assert!(matches!(
            inflight.owned_run_termination(),
            OwnedRunTermination::NoRunningRun
        ));
    }

    #[test]
    fn owned_group_signal_outcomes_remain_distinct() {
        let process_identity = ProcessIdentity::for_test(42, 7);
        let identity_changed = OwnedProcessGroupTerminationCapability::with_test_signal_outcome(
            process_identity.clone(),
            OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent,
        );
        let signal_failed = OwnedProcessGroupTerminationCapability::with_test_signal_outcome(
            process_identity,
            OwnedProcessGroupSignalOutcome::SignalFailed,
        );

        assert_eq!(identity_changed.process_group_id, 42);
        assert_eq!(identity_changed.process_identity.pid(), 42);
        assert_eq!(
            identity_changed.signal(),
            OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent
        );
        assert_eq!(
            signal_failed.signal(),
            OwnedProcessGroupSignalOutcome::SignalFailed
        );
    }

    #[test]
    fn messages_from_a_previous_run_do_not_change_a_later_run() {
        let mut inflight = fresh();
        let first_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let verified_process_identity = VerifiedProcessIdentity::for_test(process_identity);
        let termination_capability =
            OwnedProcessGroupTerminationCapability::from_verified_root(verified_process_identity);
        assert_eq!(
            inflight.activate_owned_run(first_run_id, termination_capability),
            OwnedRunActivation::Activated
        );
        let _ = inflight.finish_owned_run(first_run_id);

        let second_run_id = queue_owned_run(&mut inflight);
        assert_ne!(first_run_id, second_run_id);
        assert_eq!(
            inflight.owned_run().output_identity(),
            OwnedRunOutputIdentityRef::Correlated(&first_run_id)
        );
        assert_eq!(
            inflight.owned_run().identity(),
            OwnedRunIdentityRef::Current(&second_run_id)
        );
        assert_eq!(
            inflight.record_owned_run_output(first_run_id, "late output".to_string()),
            OwnedRunMessageUpdate::Ignored
        );
        assert_eq!(
            inflight.record_owned_run_progress(first_run_id, "late progress".to_string()),
            OwnedRunMessageUpdate::Ignored
        );
        assert_eq!(
            inflight.acknowledge_owned_run_started(first_run_id),
            OwnedRunMessageUpdate::Ignored
        );
        assert_eq!(
            inflight.finish_owned_run(first_run_id),
            OwnedRunMessageUpdate::Ignored
        );
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Queued(_)
        ));
        assert!(
            !inflight
                .owned_run()
                .output()
                .iter()
                .any(|line| line == "late output" || line == "late progress")
        );
    }

    #[test]
    fn running_and_stopping_lifecycles_own_identity_bound_authority() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let verified_process_identity = VerifiedProcessIdentity::for_test(process_identity);
        let termination_capability =
            OwnedProcessGroupTerminationCapability::from_verified_root(verified_process_identity);
        let _ = inflight.activate_owned_run(owned_run_id, termination_capability);
        assert!(matches!(
            inflight.owned_run_termination(),
            OwnedRunTermination::Available { .. }
        ));

        let _ = inflight.mark_owned_run_stopping(owned_run_id);
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Stopping(_)
        ));
    }

    #[test]
    fn owned_root_evidence_reports_live_then_stopping_then_no_live_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut inflight = fresh();
        assert_eq!(
            inflight.owned_run().owned_root_evidence(),
            OwnedRootEvidence::NoLiveRoot,
            "an absent run owns no Cargo root"
        );

        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let verified_process_identity = VerifiedProcessIdentity::for_test(process_identity.clone());
        let termination_capability =
            OwnedProcessGroupTerminationCapability::from_verified_root(verified_process_identity);
        let _ = inflight.activate_owned_run(owned_run_id, termination_capability);

        let OwnedRootEvidence::Root(live_owned_root) = inflight.owned_run().owned_root_evidence()
        else {
            return Err("an activated run owns a live Cargo root".into());
        };
        assert_eq!(live_owned_root.owned_run_id(), owned_run_id);
        assert_eq!(live_owned_root.root_identity(), &process_identity);
        assert_eq!(live_owned_root.launch_directory(), Path::new("/tmp/demo"));
        assert_eq!(
            live_owned_root.owned_root_lifecycle(),
            OwnedRootLifecycle::Live
        );

        let _ = inflight.mark_owned_run_stopping(owned_run_id);
        let OwnedRootEvidence::Root(stopping_owned_root) =
            inflight.owned_run().owned_root_evidence()
        else {
            return Err("a stopping run still owns live compiler descendants".into());
        };
        assert_eq!(
            stopping_owned_root.owned_root_lifecycle(),
            OwnedRootLifecycle::Stopping
        );
        assert_eq!(
            stopping_owned_root.launch_directory(),
            Path::new("/tmp/demo")
        );
        Ok(())
    }

    #[test]
    fn closing_stopping_output_preserves_the_active_run_slot() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let verified_process_identity = VerifiedProcessIdentity::for_test(process_identity);
        let termination_capability =
            OwnedProcessGroupTerminationCapability::from_verified_root(verified_process_identity);
        let _ = inflight.activate_owned_run(owned_run_id, termination_capability);
        let _ = inflight.mark_owned_run_stopping(owned_run_id);

        inflight.clear_owned_run_output();

        assert!(inflight.owned_run().output_is_empty());
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Stopping(_)
        ));
        assert_eq!(
            inflight.queue_owned_run(pending_example_run()),
            OwnedRunLaunchAdmission::AlreadyActive
        );
    }

    #[test]
    fn completion_restores_the_killed_marker_after_late_output_and_progress() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let verified_process_identity = VerifiedProcessIdentity::for_test(process_identity);
        let termination_capability =
            OwnedProcessGroupTerminationCapability::from_verified_root(verified_process_identity);
        let _ = inflight.activate_owned_run(owned_run_id, termination_capability);
        let _ = inflight.mark_owned_run_stopping(owned_run_id);

        let _ = inflight.record_owned_run_output(owned_run_id, "late output".to_string());
        let _ = inflight.record_owned_run_progress(owned_run_id, "late progress".to_string());
        let _ = inflight.finish_owned_run(owned_run_id);

        assert_eq!(
            inflight.owned_run().output().last().map(String::as_str),
            Some(KILLED_MARKER)
        );
        assert_eq!(
            inflight
                .owned_run()
                .output()
                .iter()
                .filter(|line| line.as_str() == KILLED_MARKER)
                .count(),
            1
        );
    }

    #[test]
    fn running_clean_paths_round_trip() {
        let mut inflight = fresh();
        let path = abs("/tmp/foo");
        inflight.clean_mut().insert(path.clone(), Instant::now());
        assert!(inflight.clean().running.contains_key(&path));
        let removed = inflight.clean_mut().remove(&path);
        assert!(removed.is_some());
        assert!(inflight.clean().is_empty());
    }

    #[test]
    fn clean_toast_round_trip() {
        let mut inflight = fresh();
        inflight.clean_mut().toast = Some(ToastTaskId(7));
        assert_eq!(inflight.clean().toast, Some(ToastTaskId(7)));
        inflight.clean_mut().toast = None;
        assert!(inflight.clean().toast.is_none());
    }

    #[test]
    fn pending_ci_fetch_set_take_clear() {
        fn fixture() -> PendingCiFetch {
            PendingCiFetch {
                project_path:      "/tmp/proj".into(),
                ci_run_count:      5,
                oldest_created_at: None,
                ci_fetch_kind:     CiFetchKind::Sync,
            }
        }

        let mut inflight = fresh();
        inflight.set_pending_ci_fetch(fixture());
        let taken = inflight.take_pending_ci_fetch();
        assert!(taken.is_some());
        assert!(inflight.take_pending_ci_fetch().is_none());

        inflight.set_pending_ci_fetch(fixture());
        inflight.clear_pending_ci_fetch();
        assert!(inflight.take_pending_ci_fetch().is_none());
    }

    #[test]
    fn killed_run_does_not_also_append_done_marker() {
        let mut inflight = fresh();
        inflight.example_output_mut().push("line".to_string());
        inflight.set_example_running(Some("demo".to_string()));

        inflight.mark_run_killed();
        assert!(inflight.example_running().is_none());
        assert_eq!(
            inflight.example_output().last().map(String::as_str),
            Some(KILLED_MARKER),
        );

        inflight.append_done_marker();
        let markers = inflight
            .example_output()
            .iter()
            .filter(|line| line.starts_with("──"))
            .count();
        assert_eq!(markers, 1, "a killed run keeps exactly one terminal marker");
    }

    #[test]
    fn normal_finish_appends_done_marker_once() {
        let mut inflight = fresh();
        inflight.example_output_mut().push("line".to_string());

        inflight.append_done_marker();
        inflight.append_done_marker();
        assert_eq!(
            inflight.example_output(),
            &["line".to_string(), DONE_MARKER.to_string()],
        );
    }

    #[test]
    fn pending_cleans_queue_is_fifo() {
        let mut inflight = fresh();
        inflight.pending_cleans_mut().push_back(PendingClean {
            abs_path: abs("/tmp/a"),
        });
        inflight.pending_cleans_mut().push_back(PendingClean {
            abs_path: abs("/tmp/b"),
        });

        let first = inflight.pending_cleans_mut().pop_front();
        assert_eq!(
            first.unwrap().abs_path.as_path(),
            crate::project::normalize_test_path(std::path::Path::new("/tmp/a")).as_path(),
            "FIFO ordering preserved"
        );
        let second = inflight.pending_cleans_mut().pop_front();
        assert_eq!(
            second.unwrap().abs_path.as_path(),
            crate::project::normalize_test_path(std::path::Path::new("/tmp/b")).as_path()
        );
        assert!(inflight.pending_cleans_mut().pop_front().is_none());
    }
}
