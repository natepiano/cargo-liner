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

use tui_pane::RunningTracker;

use super::owned_run_process_actor::OwnedProcessGroupSignalOutcome;
use super::owned_run_process_actor::OwnedRunProcessActor;
use super::owned_run_process_actor::OwnedRunTerminationOutcome;
use super::owned_run_process_actor::OwnedRunTerminationSubmission;
use super::owned_run_process_actor::OwnedRunTerminationToken;
use crate::build_monitor::LiveOwnedRoot;
use crate::build_monitor::OwnedRootEvidence;
use crate::build_monitor::OwnedRootLifecycle;
use crate::process_observation::identity::ProcessIdentity;
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

    /// The retained output together with the run that produced it.
    ///
    /// Output that names no producer carries no lines, so a caller can never
    /// draw output it cannot attribute. The producer is the retaining run, not
    /// [`Self::identity`]: run N's output stays labelled N while run N+1 is
    /// queued or starting.
    pub(crate) fn output_state(&self) -> OwnedRunOutputStateRef<'_> {
        debug_assert!(
            self.lifecycle
                .output_identity_is_valid(self.output_identity())
        );
        match self.lifecycle.output_identity() {
            OwnedRunOutputIdentityRef::Uncorrelated => OwnedRunOutputStateRef::Absent,
            OwnedRunOutputIdentityRef::Correlated(producer) => OwnedRunOutputStateRef::Retained {
                producer: *producer,
                title:    self.lifecycle.output_title(),
                lines:    self.lifecycle.output(),
            },
        }
    }

    pub(crate) fn output_is_empty(&self) -> bool { self.output().is_empty() }

    /// How the run behind the retained output ended.
    ///
    /// The Output pane keeps that output pinned after the run is gone, so the
    /// marker has to survive the lifecycle it describes.
    pub(crate) const fn completion_marker(&self) -> OwnedRunCompletionMarker {
        self.lifecycle.completion_marker()
    }

    pub(crate) fn running_label(&self) -> OwnedRunRunningLabelRef<'_> {
        self.lifecycle.running_label()
    }

    pub(crate) const fn is_running(&self) -> bool {
        matches!(
            self.lifecycle(),
            OwnedRunLifecycle::Running(_) | OwnedRunLifecycle::TerminationRequestPending(_)
        )
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
                | OwnedRunLifecycle::TerminationRequestPending(_)
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
                    running_owned_run.root_identity.clone(),
                    running_owned_run.launch_directory.clone(),
                    OwnedRootLifecycle::Live,
                ))
            },
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run) => {
                let running_owned_run = &termination_request_pending_owned_run.running_owned_run;
                OwnedRootEvidence::Root(LiveOwnedRoot::new(
                    running_owned_run.owned_run_id,
                    running_owned_run.root_identity.clone(),
                    running_owned_run.launch_directory.clone(),
                    OwnedRootLifecycle::Live,
                ))
            },
            OwnedRunLifecycle::Stopping(stopping_owned_run) => {
                OwnedRootEvidence::Root(LiveOwnedRoot::new(
                    stopping_owned_run.owned_run_id,
                    stopping_owned_run.root_identity.clone(),
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
        owned_run_process_actor: OwnedRunProcessActor,
        root_identity: ProcessIdentity,
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
                    owned_run_process_actor,
                    root_identity,
                });
                OwnedRunActivation::Activated
            },
            lifecycle => {
                self.lifecycle = lifecycle;
                OwnedRunActivation::NoMatchingStartingRun(owned_run_process_actor)
            },
        }
    }

    fn start_process_actor(&mut self, owned_run_id: OwnedRunId) {
        if let OwnedRunLifecycle::Running(running_owned_run) = &mut self.lifecycle
            && running_owned_run.owned_run_id == owned_run_id
        {
            running_owned_run.owned_run_process_actor.start_worker();
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
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run)
                if termination_request_pending_owned_run
                    .running_owned_run
                    .owned_run_id
                    == owned_run_id =>
            {
                termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
                    .lines
                    .push(line);
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
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run)
                if termination_request_pending_owned_run
                    .running_owned_run
                    .owned_run_id
                    == owned_run_id =>
            {
                replace_progress_line(
                    &mut termination_request_pending_owned_run
                        .running_owned_run
                        .retained_output
                        .lines,
                    line,
                );
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
            OwnedRunLifecycle::TerminationRequestPending(
                mut termination_request_pending_owned_run,
            ) if termination_request_pending_owned_run
                .running_owned_run
                .owned_run_id
                == owned_run_id =>
            {
                append_marker_if_absent(
                    &mut termination_request_pending_owned_run
                        .running_owned_run
                        .retained_output
                        .lines,
                    DONE_MARKER,
                );
                let running_owned_run = termination_request_pending_owned_run.running_owned_run;
                self.lifecycle = OwnedRunLifecycle::RetainedSuccess(RetainedOwnedRun {
                    owned_run_id,
                    retained_output: running_owned_run.retained_output,
                });
                OwnedRunMessageUpdate::Applied
            },
            OwnedRunLifecycle::Stopping(mut stopping_owned_run)
                if stopping_owned_run.owned_run_id == owned_run_id =>
            {
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

    const fn termination(&self) -> OwnedRunTermination {
        match &self.lifecycle {
            OwnedRunLifecycle::Running(running_owned_run) => OwnedRunTermination::Available {
                owned_run_id:                running_owned_run.owned_run_id,
                owned_run_termination_token: running_owned_run
                    .owned_run_process_actor
                    .termination_token(),
            },
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run) => {
                OwnedRunTermination::RequestPending {
                    owned_run_id: termination_request_pending_owned_run
                        .running_owned_run
                        .owned_run_id,
                }
            },
            _ => OwnedRunTermination::NoRunningRun,
        }
    }

    fn submit_termination(
        &mut self,
        owned_run_termination_token: OwnedRunTerminationToken,
    ) -> OwnedRunTerminationSubmission {
        let owned_run_termination_submission = match &self.lifecycle {
            OwnedRunLifecycle::Running(running_owned_run) => running_owned_run
                .owned_run_process_actor
                .submit_termination(owned_run_termination_token),
            OwnedRunLifecycle::TerminationRequestPending(_) => {
                OwnedRunTerminationSubmission::RequestAlreadyPending
            },
            _ => OwnedRunTerminationSubmission::ActorUnavailable,
        };
        if let OwnedRunTerminationSubmission::Submitted(owned_run_id) =
            owned_run_termination_submission
        {
            self.begin_termination_request(owned_run_id);
        }
        owned_run_termination_submission
    }

    fn begin_termination_request(&mut self, owned_run_id: OwnedRunId) {
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        self.lifecycle = match lifecycle {
            OwnedRunLifecycle::Running(running_owned_run)
                if running_owned_run.owned_run_id == owned_run_id =>
            {
                OwnedRunLifecycle::TerminationRequestPending(TerminationRequestPendingOwnedRun {
                    running_owned_run,
                })
            },
            lifecycle => lifecycle,
        };
    }

    fn reconcile_termination_outcome(
        &mut self,
        owned_run_termination_outcome: OwnedRunTerminationOutcome,
    ) -> OwnedRunStopTransition {
        let owned_run_id = match owned_run_termination_outcome {
            OwnedRunTerminationOutcome::Honored { owned_run_id, .. }
            | OwnedRunTerminationOutcome::Refused { owned_run_id } => owned_run_id,
        };
        let lifecycle = std::mem::replace(&mut self.lifecycle, OwnedRunLifecycle::absent());
        let OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run) =
            lifecycle
        else {
            self.lifecycle = lifecycle;
            return OwnedRunStopTransition::NoMatchingTerminationRequest;
        };
        if termination_request_pending_owned_run
            .running_owned_run
            .owned_run_id
            != owned_run_id
        {
            self.lifecycle =
                OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run);
            return OwnedRunStopTransition::NoMatchingTerminationRequest;
        }

        let mut running_owned_run = termination_request_pending_owned_run.running_owned_run;
        match owned_run_termination_outcome {
            OwnedRunTerminationOutcome::Honored {
                signal: OwnedProcessGroupSignalOutcome::Sent,
                ..
            } => {
                append_marker_if_absent(
                    &mut running_owned_run.retained_output.lines,
                    KILLED_MARKER,
                );
                self.lifecycle =
                    OwnedRunLifecycle::Stopping(StoppingOwnedRun::from(running_owned_run));
                OwnedRunStopTransition::Stopping
            },
            OwnedRunTerminationOutcome::Honored {
                signal:
                    OwnedProcessGroupSignalOutcome::ProcessAlreadyReaped
                    | OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent
                    | OwnedProcessGroupSignalOutcome::SignalFailed,
                ..
            }
            | OwnedRunTerminationOutcome::Refused { .. } => {
                self.lifecycle = OwnedRunLifecycle::Running(running_owned_run);
                OwnedRunStopTransition::RetryableRunning
            },
        }
    }

    #[cfg(test)]
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
                OwnedRunStopTransition::NoMatchingTerminationRequest
            },
        }
    }

    fn clear_output(&mut self) {
        match &mut self.lifecycle {
            OwnedRunLifecycle::Queued(queued_owned_run) => {
                queued_owned_run.retained_output = OwnedRunRetainedOutput::absent();
            },
            OwnedRunLifecycle::Starting(starting_owned_run) => {
                starting_owned_run.retained_output = OwnedRunRetainedOutput::absent();
            },
            OwnedRunLifecycle::Running(running_owned_run) => {
                running_owned_run.retained_output = OwnedRunRetainedOutput::correlated_unnamed(
                    running_owned_run.owned_run_id,
                    Vec::new(),
                );
            },
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run) => {
                let running_owned_run =
                    &mut termination_request_pending_owned_run.running_owned_run;
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
    fn output_mut_for_test(&mut self) -> &mut Vec<String> {
        // Seeding lines onto a slot that never ran would leave output naming no
        // producer, which presentation reads as absent. Promote the slot to a
        // retained run first so the seeded output is attributable.
        if let OwnedRunLifecycle::Absent(retained_output) = &mut self.lifecycle {
            let lines = std::mem::take(&mut retained_output.lines);
            self.set_output_for_test(lines);
        }
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
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run) => {
                &mut termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
                    .lines
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
        let mut owned_run_process_actor = OwnedRunProcessActor::for_test(
            owned_run_id,
            process_identity.clone(),
            OwnedProcessGroupSignalOutcome::Sent,
        );
        owned_run_process_actor.start_worker();
        self.lifecycle = OwnedRunLifecycle::Running(RunningOwnedRun {
            owned_run_id,
            running_label: running_label.clone(),
            launch_directory: PathBuf::new(),
            retained_output: retained_output.with_missing_title_replaced_by(running_label),
            root_identity: process_identity,
            owned_run_process_actor,
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
            OwnedRunLifecycle::TerminationRequestPending(termination_request_pending_owned_run) => {
                let running_owned_run = termination_request_pending_owned_run.running_owned_run;
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
    TerminationRequestPending(TerminationRequestPendingOwnedRun),
    Stopping(StoppingOwnedRun),
    RetainedSuccess(RetainedOwnedRun),
    GoneAfterSignal(RetainedOwnedRun),
    Failed(FailedOwnedRun),
}

impl OwnedRunLifecycle {
    const fn absent() -> Self { Self::Absent(OwnedRunRetainedOutput::absent()) }

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
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                OwnedRunIdentityRef::Current(
                    &termination_request_pending_owned_run
                        .running_owned_run
                        .owned_run_id,
                )
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
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                &termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
                    .lines
            },
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
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
                    .identity
                    .as_ref()
            },
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
                Self::TerminationRequestPending(termination_request_pending_owned_run),
                OwnedRunOutputIdentityRef::Correlated(output_owned_run_id),
            ) => {
                output_owned_run_id
                    == &termination_request_pending_owned_run
                        .running_owned_run
                        .owned_run_id
            },
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
                | Self::TerminationRequestPending(_)
                | Self::Stopping(_)
                | Self::RetainedSuccess(_)
                | Self::GoneAfterSignal(_)
                | Self::Failed(_),
                OwnedRunOutputIdentityRef::Uncorrelated,
            )
            | (Self::Absent(_), OwnedRunOutputIdentityRef::Correlated(_)) => false,
        }
    }

    /// Which end state the retained output was captured from, if any.
    const fn completion_marker(&self) -> OwnedRunCompletionMarker {
        match self {
            Self::Absent(_)
            | Self::Queued(_)
            | Self::Starting(_)
            | Self::Running(_)
            | Self::TerminationRequestPending(_)
            | Self::Stopping(_) => OwnedRunCompletionMarker::NotCompleted,
            Self::RetainedSuccess(_) => OwnedRunCompletionMarker::Done,
            Self::GoneAfterSignal(_) => OwnedRunCompletionMarker::Killed,
            Self::Failed(_) => OwnedRunCompletionMarker::Failed,
        }
    }

    fn output_title(&self) -> OwnedRunOutputTitleRef<'_> {
        match self {
            Self::Absent(_) => OwnedRunOutputTitleRef::Unavailable,
            Self::Queued(queued_owned_run) => queued_owned_run.retained_output.title.as_ref(),
            Self::Starting(starting_owned_run) => starting_owned_run.retained_output.title.as_ref(),
            Self::Running(running_owned_run) => running_owned_run.retained_output.title.as_ref(),
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
                    .title
                    .as_ref()
            },
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
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                OwnedRunRunningLabelRef::Running(
                    &termination_request_pending_owned_run
                        .running_owned_run
                        .running_label,
                )
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
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
            },
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
            Self::TerminationRequestPending(termination_request_pending_owned_run) => {
                termination_request_pending_owned_run
                    .running_owned_run
                    .retained_output
                    .title = title;
            },
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

/// A running owned run has immutable root evidence and one actor endpoint.
pub(crate) struct RunningOwnedRun {
    owned_run_id:            OwnedRunId,
    running_label:           String,
    launch_directory:        PathBuf,
    retained_output:         OwnedRunRetainedOutput,
    root_identity:           ProcessIdentity,
    owned_run_process_actor: OwnedRunProcessActor,
}

/// A running owned run whose actor has accepted one termination request but
/// has not yet reported whether it sent a signal.
pub(crate) struct TerminationRequestPendingOwnedRun {
    running_owned_run: RunningOwnedRun,
}

/// A stop-requested run retains live-root evidence until completion. Dropping
/// the command endpoint makes the detached actor worker wait for and reap its
/// child while retaining the worker-owned termination capability.
pub(crate) struct StoppingOwnedRun {
    owned_run_id:     OwnedRunId,
    launch_directory: PathBuf,
    retained_output:  OwnedRunRetainedOutput,
    root_identity:    ProcessIdentity,
}

impl From<RunningOwnedRun> for StoppingOwnedRun {
    fn from(running_owned_run: RunningOwnedRun) -> Self {
        Self {
            owned_run_id:     running_owned_run.owned_run_id,
            launch_directory: running_owned_run.launch_directory,
            retained_output:  running_owned_run.retained_output,
            root_identity:    running_owned_run.root_identity,
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

    /// Output with no producer to attribute it to. It takes no lines: the
    /// state that would let uncorrelated output still be drawn is the one this
    /// constructor exists to make unbuildable.
    const fn absent() -> Self {
        Self {
            identity: OwnedRunOutputIdentity::Uncorrelated,
            title:    OwnedRunOutputTitle::Unavailable,
            lines:    Vec::new(),
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

/// The owned run's retained output as presentation reads it.
///
/// [`Self::Absent`] is the only state without lines, so visible output always
/// names the run that produced it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunOutputStateRef<'a> {
    /// No run has produced output that is still retained.
    Absent,
    /// This run's output is retained and drawable.
    Retained {
        producer: OwnedRunId,
        title:    OwnedRunOutputTitleRef<'a>,
        lines:    &'a [String],
    },
}

/// Borrowed title availability for retained or live owned output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunOutputTitleRef<'a> {
    Named(&'a str),
    Unavailable,
}

/// How the run that produced the retained output ended.
///
/// The Output pane pins completed output with this marker even after the
/// monitored scope moves away from the run, so the marker is a property of the
/// retained output rather than of the current lifecycle slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunCompletionMarker {
    /// Nothing has completed: the slot is empty or a run is still live.
    NotCompleted,
    /// The run finished on its own.
    Done,
    /// The run ended after Cargo Port signalled its process group.
    Killed,
    /// The launch never produced a running process.
    Failed,
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

/// Whether activation accepted the actor that owns the newly spawned child.
pub(crate) enum OwnedRunActivation {
    Activated,
    NoMatchingStartingRun(OwnedRunProcessActor),
}

/// Whether one correlated background message changed the current lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunMessageUpdate {
    Applied,
    Ignored,
}

/// The current run's opaque termination authorization, when one is live.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunTermination {
    Available {
        owned_run_id:                OwnedRunId,
        owned_run_termination_token: OwnedRunTerminationToken,
    },
    /// The actor has accepted one request, so retry authority remains withheld
    /// until that request reports whether it sent a signal.
    RequestPending {
        owned_run_id: OwnedRunId,
    },
    NoRunningRun,
}

/// How a correlated termination outcome changed the owned-run lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunStopTransition {
    Stopping,
    RetryableRunning,
    NoMatchingTerminationRequest,
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

/// Whether the owned-run fixture reached the state it drives toward.
#[cfg(test)]
pub(crate) enum OwnedRunFixture {
    /// A fresh slot refused the launch, so there is no run to retain output
    /// from.
    Unbuilt,
    /// One finished run's output, retained under the run that wrote it.
    Built {
        inflight: Box<Inflight>,
        producer: OwnedRunId,
    },
}

/// One queued run's launch request, for tests that drive the owned-run
/// lifecycle rather than assemble its result.
#[cfg(test)]
fn pending_run_for_test() -> PendingExampleRun {
    use crate::tui::panes::BuildMode;
    use crate::tui::panes::CargoPackageInvocation;
    use crate::tui::panes::RunTargetKind;

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

    /// An `Inflight` driven through one whole owned run — queued, started,
    /// activated, one line of output, finished — with the next run already
    /// queued behind it.
    ///
    /// This is the state a retained body is drawn in, and only the lifecycle
    /// can produce it: the retaining run and the current lifecycle identity
    /// differ here, and a hand-assembled state can put them in combinations
    /// the lifecycle never reaches.
    #[cfg(test)]
    pub(crate) fn with_retained_output_and_next_run_queued(line: &str) -> OwnedRunFixture {
        let mut inflight = Self::new();
        let producer = match inflight.queue_owned_run(pending_run_for_test()) {
            OwnedRunLaunchAdmission::Queued(owned_run_id) => owned_run_id,
            OwnedRunLaunchAdmission::AlreadyActive
            | OwnedRunLaunchAdmission::IdentitiesExhausted => return OwnedRunFixture::Unbuilt,
        };
        assert_eq!(
            inflight.begin_owned_run_launch(),
            OwnedRunLaunchStart::Starting(producer)
        );
        assert!(matches!(
            inflight.activate_owned_run(
                producer,
                OwnedRunProcessActor::for_test(
                    producer,
                    ProcessIdentity::for_test(4242, 7),
                    OwnedProcessGroupSignalOutcome::Sent,
                ),
                ProcessIdentity::for_test(4242, 7),
            ),
            OwnedRunActivation::Activated
        ));
        let _ = inflight.record_owned_run_output(producer, line.to_string());
        let _ = inflight.finish_owned_run(producer);
        let _ = inflight.queue_owned_run(pending_run_for_test());
        OwnedRunFixture::Built {
            inflight: Box::new(inflight),
            producer,
        }
    }

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
        owned_run_process_actor: OwnedRunProcessActor,
        root_identity: ProcessIdentity,
    ) -> OwnedRunActivation {
        self.owned_run
            .activate(owned_run_id, owned_run_process_actor, root_identity)
    }

    pub(crate) fn start_owned_run_process_actor(&mut self, owned_run_id: OwnedRunId) {
        self.owned_run.start_process_actor(owned_run_id);
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

    pub(crate) const fn owned_run_termination(&self) -> OwnedRunTermination {
        self.owned_run.termination()
    }

    pub(crate) fn submit_owned_run_termination(
        &mut self,
        owned_run_termination_token: OwnedRunTerminationToken,
    ) -> OwnedRunTerminationSubmission {
        self.owned_run
            .submit_termination(owned_run_termination_token)
    }

    pub(crate) fn reconcile_owned_run_termination(
        &mut self,
        owned_run_termination_outcome: OwnedRunTerminationOutcome,
    ) -> OwnedRunStopTransition {
        self.owned_run
            .reconcile_termination_outcome(owned_run_termination_outcome)
    }

    #[cfg(test)]
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
    pub(crate) fn example_output_mut(&mut self) -> &mut Vec<String> {
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
    use crate::tui::panes::CiFetchKind;

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
            owned_run.queue(pending_run_for_test()),
            OwnedRunLaunchAdmission::IdentitiesExhausted
        );
        assert!(matches!(
            owned_run.identity(),
            OwnedRunIdentityRef::Current(current_owned_run_id)
                if *current_owned_run_id == last_owned_run_id
        ));
    }

    fn abs(path: &str) -> AbsolutePath { AbsolutePath::from(PathBuf::from(path)) }

    fn queue_owned_run(inflight: &mut Inflight) -> OwnedRunId {
        let owned_run_launch_admission = inflight.queue_owned_run(pending_run_for_test());
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

    fn activate_owned_run(
        inflight: &mut Inflight,
        signal_outcome: OwnedProcessGroupSignalOutcome,
    ) -> (OwnedRunId, OwnedRunTerminationToken) {
        let owned_run_id = queue_owned_run(inflight);
        assert_eq!(
            inflight.begin_owned_run_launch(),
            OwnedRunLaunchStart::Starting(owned_run_id)
        );
        let process_identity = ProcessIdentity::for_test(42, 7);
        assert!(matches!(
            inflight.activate_owned_run(
                owned_run_id,
                OwnedRunProcessActor::for_test(
                    owned_run_id,
                    process_identity.clone(),
                    signal_outcome,
                ),
                process_identity,
            ),
            OwnedRunActivation::Activated
        ));
        inflight.start_owned_run_process_actor(owned_run_id);
        let OwnedRunTermination::Available {
            owned_run_termination_token,
            ..
        } = inflight.owned_run_termination()
        else {
            panic!("an activated owned run should expose termination authority");
        };
        (owned_run_id, owned_run_termination_token)
    }

    fn submit_termination_request(inflight: &mut Inflight) -> OwnedRunId {
        let (owned_run_id, owned_run_termination_token) =
            activate_owned_run(inflight, OwnedProcessGroupSignalOutcome::Sent);
        assert_eq!(
            inflight.submit_owned_run_termination(owned_run_termination_token),
            OwnedRunTerminationSubmission::Submitted(owned_run_id)
        );
        owned_run_id
    }

    fn assert_failed_termination_restores_retryable_running(
        owned_run_termination_outcome: impl FnOnce(OwnedRunId) -> OwnedRunTerminationOutcome,
    ) {
        let mut inflight = fresh();
        let (owned_run_id, owned_run_termination_token) =
            activate_owned_run(&mut inflight, OwnedProcessGroupSignalOutcome::Sent);
        let _ = inflight.record_owned_run_output(owned_run_id, "preserved output".to_string());
        assert_eq!(
            inflight.submit_owned_run_termination(owned_run_termination_token),
            OwnedRunTerminationSubmission::Submitted(owned_run_id)
        );

        assert_eq!(
            inflight.reconcile_owned_run_termination(owned_run_termination_outcome(owned_run_id)),
            OwnedRunStopTransition::RetryableRunning
        );
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Running(_)
        ));
        assert!(
            inflight
                .owned_run()
                .output()
                .iter()
                .any(|line| line == "preserved output")
        );
        let OwnedRootEvidence::Root(live_owned_root) = inflight.owned_run().owned_root_evidence()
        else {
            panic!("a failed termination request should preserve its live root");
        };
        assert_eq!(live_owned_root.owned_run_id(), owned_run_id);
        assert_eq!(live_owned_root.launch_directory(), Path::new("/tmp/demo"));
        assert_eq!(
            live_owned_root.owned_root_lifecycle(),
            OwnedRootLifecycle::Live
        );
        let OwnedRunTermination::Available {
            owned_run_id: retryable_owned_run_id,
            owned_run_termination_token,
        } = inflight.owned_run_termination()
        else {
            panic!("a failed termination request should restore retry authority");
        };
        assert_eq!(retryable_owned_run_id, owned_run_id);
        assert_eq!(
            inflight.submit_owned_run_termination(owned_run_termination_token),
            OwnedRunTerminationSubmission::Submitted(owned_run_id),
            "the same actor should accept the retry"
        );
    }

    #[test]
    fn termination_submission_enters_pending_without_claiming_a_signal_was_sent() {
        let mut inflight = fresh();
        let (owned_run_id, owned_run_termination_token) =
            activate_owned_run(&mut inflight, OwnedProcessGroupSignalOutcome::Sent);

        assert_eq!(
            inflight.submit_owned_run_termination(owned_run_termination_token),
            OwnedRunTerminationSubmission::Submitted(owned_run_id)
        );

        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::TerminationRequestPending(_)
        ));
        assert_eq!(
            inflight.owned_run_termination(),
            OwnedRunTermination::RequestPending { owned_run_id }
        );
        assert!(matches!(
            inflight.owned_run().running_label(),
            OwnedRunRunningLabelRef::Running(_)
        ));
        assert!(
            !inflight
                .owned_run()
                .output()
                .iter()
                .any(|line| line == KILLED_MARKER)
        );
        assert_eq!(
            inflight.submit_owned_run_termination(owned_run_termination_token),
            OwnedRunTerminationSubmission::RequestAlreadyPending
        );
    }

    #[test]
    fn sent_termination_outcome_enters_stopping_and_appends_the_killed_marker() {
        let mut inflight = fresh();
        let owned_run_id = submit_termination_request(&mut inflight);
        let output_before_signal = inflight.owned_run().output().to_vec();

        assert_eq!(
            inflight.reconcile_owned_run_termination(OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::Sent,
            }),
            OwnedRunStopTransition::Stopping
        );

        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Stopping(_)
        ));
        assert_eq!(
            &inflight.owned_run().output()[..output_before_signal.len()],
            output_before_signal
        );
        assert_eq!(
            inflight.owned_run().output().last().map(String::as_str),
            Some(KILLED_MARKER)
        );
    }

    #[test]
    fn identity_no_longer_current_outcome_restores_retryable_running() {
        assert_failed_termination_restores_retryable_running(|owned_run_id| {
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent,
            }
        });
    }

    #[test]
    fn process_already_reaped_outcome_restores_retryable_running() {
        assert_failed_termination_restores_retryable_running(|owned_run_id| {
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::ProcessAlreadyReaped,
            }
        });
    }

    #[test]
    fn signal_failed_outcome_restores_retryable_running() {
        assert_failed_termination_restores_retryable_running(|owned_run_id| {
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::SignalFailed,
            }
        });
    }

    #[test]
    fn refused_termination_outcome_restores_retryable_running() {
        assert_failed_termination_restores_retryable_running(|owned_run_id| {
            OwnedRunTerminationOutcome::Refused { owned_run_id }
        });
    }

    #[test]
    fn stale_termination_outcome_cannot_change_the_pending_run() {
        let mut inflight = fresh();
        let owned_run_id = submit_termination_request(&mut inflight);
        let stale_owned_run_id = OwnedRunId::for_test(
            owned_run_id
                .0
                .checked_add(1)
                .expect("the stale identity should be representable"),
        );

        assert_eq!(
            inflight.reconcile_owned_run_termination(OwnedRunTerminationOutcome::Honored {
                owned_run_id: stale_owned_run_id,
                signal:       OwnedProcessGroupSignalOutcome::Sent,
            }),
            OwnedRunStopTransition::NoMatchingTerminationRequest
        );
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::TerminationRequestPending(_)
        ));
        assert_eq!(
            inflight.owned_run_termination(),
            OwnedRunTermination::RequestPending { owned_run_id }
        );
        assert!(
            !inflight
                .owned_run()
                .output()
                .iter()
                .any(|line| line == KILLED_MARKER)
        );
    }

    #[test]
    fn child_completion_while_termination_is_pending_finishes_the_matching_run_normally() {
        let mut inflight = fresh();
        let owned_run_id = submit_termination_request(&mut inflight);

        assert_eq!(
            inflight.finish_owned_run(owned_run_id),
            OwnedRunMessageUpdate::Applied
        );
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::RetainedSuccess(_)
        ));
        assert_eq!(
            inflight.owned_run().completion_marker(),
            OwnedRunCompletionMarker::Done
        );
        assert_eq!(
            inflight.owned_run().output().last().map(String::as_str),
            Some(DONE_MARKER)
        );
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
    fn messages_from_a_previous_run_do_not_change_a_later_run() {
        let mut inflight = fresh();
        let first_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        assert!(matches!(
            inflight.activate_owned_run(
                first_run_id,
                OwnedRunProcessActor::for_test(
                    first_run_id,
                    process_identity.clone(),
                    OwnedProcessGroupSignalOutcome::Sent,
                ),
                process_identity,
            ),
            OwnedRunActivation::Activated
        ));
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
    fn running_and_stopping_lifecycles_own_actor_termination_authority() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let _ = inflight.activate_owned_run(
            owned_run_id,
            OwnedRunProcessActor::for_test(
                owned_run_id,
                process_identity.clone(),
                OwnedProcessGroupSignalOutcome::Sent,
            ),
            process_identity,
        );
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
        let _ = inflight.activate_owned_run(
            owned_run_id,
            OwnedRunProcessActor::for_test(
                owned_run_id,
                process_identity.clone(),
                OwnedProcessGroupSignalOutcome::Sent,
            ),
            process_identity.clone(),
        );

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
        let _ = inflight.activate_owned_run(
            owned_run_id,
            OwnedRunProcessActor::for_test(
                owned_run_id,
                process_identity.clone(),
                OwnedProcessGroupSignalOutcome::Sent,
            ),
            process_identity,
        );
        let _ = inflight.mark_owned_run_stopping(owned_run_id);

        inflight.clear_owned_run_output();

        assert!(inflight.owned_run().output_is_empty());
        assert!(matches!(
            inflight.owned_run().lifecycle(),
            OwnedRunLifecycle::Stopping(_)
        ));
        assert_eq!(
            inflight.queue_owned_run(pending_run_for_test()),
            OwnedRunLaunchAdmission::AlreadyActive
        );
    }

    #[test]
    fn completion_restores_the_killed_marker_after_late_output_and_progress() {
        let mut inflight = fresh();
        let owned_run_id = queue_owned_run(&mut inflight);
        let _ = inflight.begin_owned_run_launch();
        let process_identity = ProcessIdentity::for_test(42, 7);
        let _ = inflight.activate_owned_run(
            owned_run_id,
            OwnedRunProcessActor::for_test(
                owned_run_id,
                process_identity.clone(),
                OwnedProcessGroupSignalOutcome::Sent,
            ),
            process_identity,
        );
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
