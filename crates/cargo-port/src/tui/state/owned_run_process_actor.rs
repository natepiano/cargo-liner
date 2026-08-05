//! The single owner of one Cargo Port-owned run's child process.
//!
//! [`OwnedRunProcessActor`] holds both the child wait and the non-cloneable
//! [`OwnedProcessGroupTerminationCapability`] for that run. Its worker handles
//! command delivery and child reaping serially, so it cannot signal a process
//! group concurrently with reaping the group leader.
//!
//! Callers retain only [`OwnedRunTerminationToken`]. A token identifies one
//! run, but carries no PID, group ID, or host handle. Termination submission and
//! outcome polling use channels, so a host signal never blocks the TUI loop.

use std::process::Child;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;

#[cfg(all(not(unix), not(test)))]
use sysinfo::Pid;
#[cfg(all(not(unix), not(test)))]
use sysinfo::ProcessRefreshKind;
#[cfg(all(not(unix), not(test)))]
use sysinfo::ProcessesToUpdate;
#[cfg(all(not(unix), not(test)))]
use sysinfo::Signal;
#[cfg(all(not(unix), not(test)))]
use sysinfo::System;

use super::constants::OWNED_RUN_CHILD_REAP_POLL_INTERVAL;
#[cfg(unix)]
use super::constants::UNACTIVATED_GROUP_EXIT_POLL_ATTEMPTS;
#[cfg(unix)]
use super::constants::UNACTIVATED_GROUP_EXIT_POLL_INTERVAL;
use super::inflight::OwnedRunId;
use crate::channel::Receiver;
use crate::channel::RecvTimeoutError;
use crate::channel::Sender;
use crate::channel::unbounded;
use crate::process_observation::identity::ProcessIdentity;
#[cfg(not(test))]
use crate::process_observation::identity::StrongProcessIdentityRevalidation;
use crate::process_observation::identity::VerifiedProcessIdentity;
#[cfg(not(test))]
use crate::process_observation::identity::revalidate_strong_process_identity;
use crate::tui::messages::OwnedRunEvent;

/// How the actor resolved one accepted owned-group termination request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedProcessGroupSignalOutcome {
    Sent,
    ProcessAlreadyReaped,
    IdentityNoLongerCurrent,
    SignalFailed,
}

/// A run-bound claim on termination authority.
///
/// It identifies a run but contains no process-control data. UI state and a
/// retained confirmation can safely store it until the actor either honors or
/// refuses it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OwnedRunTerminationToken(OwnedRunId);

/// Whether a termination token reached the actor worker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunTerminationSubmission {
    Submitted(OwnedRunId),
    RequestAlreadyPending,
    TokenRefused,
    ActorUnavailable,
}

/// The actor's result for one token it received.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedRunTerminationOutcome {
    Honored {
        owned_run_id: OwnedRunId,
        signal:       OwnedProcessGroupSignalOutcome,
    },
    Refused {
        owned_run_id: OwnedRunId,
    },
}

#[cfg(test)]
impl OwnedRunTerminationOutcome {
    pub(crate) const fn owned_run_id(self) -> OwnedRunId {
        match self {
            Self::Honored { owned_run_id, .. } | Self::Refused { owned_run_id } => owned_run_id,
        }
    }
}

/// Opaque authority to signal the isolated group created for one verified
/// Cargo Port-owned root process.
///
/// Construction consumes revalidated identity evidence. The capability is not
/// cloned, decomposed, or exposed outside this file; it exists only in the
/// actor worker that also owns the child wait.
#[derive(Debug)]
struct OwnedProcessGroupTerminationCapability {
    #[cfg(not(test))]
    process_group_id:            u32,
    #[cfg(not(test))]
    process_identity:            ProcessIdentity,
    #[cfg(test)]
    authorized_process_identity: ProcessIdentity,
    #[cfg(test)]
    test_signal_outcome:         OwnedProcessGroupSignalOutcome,
    #[cfg(test)]
    test_signal_count:           std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl OwnedProcessGroupTerminationCapability {
    #[cfg(not(test))]
    const fn from_verified_root(verified_process_identity: VerifiedProcessIdentity) -> Self {
        let process_identity = verified_process_identity.into_process_identity();
        let process_group_id = process_identity.pid();
        Self {
            process_group_id,
            process_identity,
        }
    }

    #[cfg(test)]
    fn from_verified_root(verified_process_identity: VerifiedProcessIdentity) -> Self {
        let process_identity = verified_process_identity.into_process_identity();
        Self {
            authorized_process_identity: process_identity,
            test_signal_outcome:         OwnedProcessGroupSignalOutcome::Sent,
            test_signal_count:           std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
                0,
            )),
        }
    }

    #[cfg(not(test))]
    fn signal(&self) -> OwnedProcessGroupSignalOutcome {
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
    fn signal(&self) -> OwnedProcessGroupSignalOutcome {
        self.test_signal_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.authorized_process_identity.pid() == 0 {
            OwnedProcessGroupSignalOutcome::SignalFailed
        } else {
            self.test_signal_outcome
        }
    }

    #[cfg(test)]
    fn with_test_signal_outcome(
        process_identity: ProcessIdentity,
        test_signal_outcome: OwnedProcessGroupSignalOutcome,
    ) -> Self {
        Self::with_test_signal_probe(
            process_identity,
            test_signal_outcome,
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        )
    }

    #[cfg(test)]
    const fn with_test_signal_probe(
        process_identity: ProcessIdentity,
        test_signal_outcome: OwnedProcessGroupSignalOutcome,
        test_signal_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        Self {
            #[cfg(not(test))]
            process_group_id: process_identity.pid(),
            #[cfg(not(test))]
            process_identity,
            #[cfg(test)]
            authorized_process_identity: process_identity,
            test_signal_outcome,
            test_signal_count,
        }
    }
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
    if system
        .process(pid)
        .is_some_and(|process| process.kill_with(Signal::Term).unwrap_or(false))
    {
        OwnedProcessGroupSignalOutcome::Sent
    } else {
        OwnedProcessGroupSignalOutcome::SignalFailed
    }
}

/// A command received by the actor worker.
enum OwnedRunProcessCommand {
    Terminate(OwnedRunTerminationToken),
}

/// The child process the actor worker owns and reaps.
enum OwnedRunChildProcess {
    Spawned(Child),
    #[cfg(test)]
    Fixture,
}

/// Whether the actor worker found that its child is still running.
enum OwnedRunChildStatus {
    Running,
    Reaped,
}

impl OwnedRunChildProcess {
    fn status(&mut self) -> OwnedRunChildStatus {
        match self {
            Self::Spawned(child) => match child.try_wait() {
                Ok(Some(_)) => OwnedRunChildStatus::Reaped,
                Ok(None) | Err(_) => OwnedRunChildStatus::Running,
            },
            #[cfg(test)]
            Self::Fixture => OwnedRunChildStatus::Running,
        }
    }

    /// Retain ownership and wait until the child is reaped after the command
    /// endpoint disconnects.
    fn wait_until_reaped(&mut self) {
        match self {
            Self::Spawned(child) => loop {
                if child.wait().is_ok() {
                    break;
                }
                std::thread::sleep(OWNED_RUN_CHILD_REAP_POLL_INTERVAL);
            },
            #[cfg(test)]
            Self::Fixture => {},
        }
    }

    fn discard_unactivated(&mut self) {
        match self {
            Self::Spawned(child) => discard_unverified_owned_process_group(child),
            #[cfg(test)]
            Self::Fixture => {},
        }
    }
}

/// Resources the actor owns before the `RunningOwnedRun` lifecycle accepts it.
struct PreparedOwnedRunProcessActor {
    child:                                      OwnedRunChildProcess,
    owned_process_group_termination_capability: OwnedProcessGroupTerminationCapability,
    output_readers:                             Vec<JoinHandle<()>>,
    owned_run_event_sender:                     Sender<OwnedRunEvent>,
}

/// The actor endpoint available after its worker owns the child and capability.
struct ServingOwnedRunProcessActor {
    command_tx:        Sender<OwnedRunProcessCommand>,
    command_admission: Arc<Mutex<OwnedRunCommandAdmission>>,
}

/// The worker-only command endpoint and its shared admission boundary.
struct OwnedRunProcessWorkerChannels {
    command_rx:        Receiver<OwnedRunProcessCommand>,
    command_admission: Arc<Mutex<OwnedRunCommandAdmission>>,
}

impl ServingOwnedRunProcessActor {
    fn unavailable() -> Self {
        let (command_tx, command_rx) = unbounded();
        drop(command_rx);
        Self {
            command_tx,
            command_admission: Arc::new(Mutex::new(OwnedRunCommandAdmission::ChildReaped)),
        }
    }
}

/// The actor's lifecycle across `Inflight` activation.
enum OwnedRunProcessActorState {
    Prepared(PreparedOwnedRunProcessActor),
    Serving(ServingOwnedRunProcessActor),
}

enum OwnedRunCommandEndpoint {
    Connected,
    Disconnected,
}

/// Whether `submit_termination` may enqueue work for the actor worker.
enum OwnedRunCommandAdmission {
    Accepting,
    ChildReaped,
}

/// The sole owner of one owned run's child wait and group termination
/// authority.
pub(crate) struct OwnedRunProcessActor {
    owned_run_id: OwnedRunId,
    state:        OwnedRunProcessActorState,
}

impl OwnedRunProcessActor {
    /// Prepare an actor before its `Starting` lifecycle is promoted. The child
    /// and capability already belong to the actor, but its worker remains idle
    /// until the lifecycle owns the actor and can accept its messages.
    #[cfg(not(test))]
    pub(crate) const fn prepare(
        owned_run_id: OwnedRunId,
        child: Child,
        verified_process_identity: VerifiedProcessIdentity,
        output_readers: Vec<JoinHandle<()>>,
        owned_run_event_sender: Sender<OwnedRunEvent>,
    ) -> Self {
        Self {
            owned_run_id,
            state: OwnedRunProcessActorState::Prepared(PreparedOwnedRunProcessActor {
                child: OwnedRunChildProcess::Spawned(child),
                owned_process_group_termination_capability:
                    OwnedProcessGroupTerminationCapability::from_verified_root(
                        verified_process_identity,
                    ),
                output_readers,
                owned_run_event_sender,
            }),
        }
    }

    /// Prepare the test build's probed capability before activation.
    #[cfg(test)]
    pub(crate) fn prepare(
        owned_run_id: OwnedRunId,
        child: Child,
        verified_process_identity: VerifiedProcessIdentity,
        output_readers: Vec<JoinHandle<()>>,
        owned_run_event_sender: Sender<OwnedRunEvent>,
    ) -> Self {
        Self {
            owned_run_id,
            state: OwnedRunProcessActorState::Prepared(PreparedOwnedRunProcessActor {
                child: OwnedRunChildProcess::Spawned(child),
                owned_process_group_termination_capability:
                    OwnedProcessGroupTerminationCapability::from_verified_root(
                        verified_process_identity,
                    ),
                output_readers,
                owned_run_event_sender,
            }),
        }
    }

    /// Start the serialized worker after `Inflight` owns this actor.
    pub(crate) fn start_worker(&mut self) {
        let OwnedRunProcessActorState::Prepared(prepared_owned_run_process_actor) =
            std::mem::replace(
                &mut self.state,
                OwnedRunProcessActorState::Serving(ServingOwnedRunProcessActor::unavailable()),
            )
        else {
            return;
        };
        let (command_tx, command_rx) = unbounded();
        let command_admission = Arc::new(Mutex::new(OwnedRunCommandAdmission::Accepting));
        let owned_run_id = self.owned_run_id;
        let worker_command_admission = Arc::clone(&command_admission);
        std::thread::spawn(move || {
            run_owned_run_process_worker(
                owned_run_id,
                prepared_owned_run_process_actor,
                OwnedRunProcessWorkerChannels {
                    command_rx,
                    command_admission: worker_command_admission,
                },
            );
        });
        self.state = OwnedRunProcessActorState::Serving(ServingOwnedRunProcessActor {
            command_tx,
            command_admission,
        });
    }

    /// Return an opaque authorization token for this actor's one run.
    pub(crate) const fn termination_token(&self) -> OwnedRunTerminationToken {
        OwnedRunTerminationToken(self.owned_run_id)
    }

    /// Submit one token without waiting for host work or its result.
    pub(crate) fn submit_termination(
        &self,
        owned_run_termination_token: OwnedRunTerminationToken,
    ) -> OwnedRunTerminationSubmission {
        if owned_run_termination_token.0 != self.owned_run_id {
            return OwnedRunTerminationSubmission::TokenRefused;
        }
        match &self.state {
            OwnedRunProcessActorState::Prepared(_) => {
                OwnedRunTerminationSubmission::ActorUnavailable
            },
            OwnedRunProcessActorState::Serving(serving_owned_run_process_actor) => {
                let Ok(command_admission) =
                    serving_owned_run_process_actor.command_admission.try_lock()
                else {
                    return OwnedRunTerminationSubmission::ActorUnavailable;
                };
                match *command_admission {
                    OwnedRunCommandAdmission::Accepting => serving_owned_run_process_actor
                        .command_tx
                        .send(OwnedRunProcessCommand::Terminate(
                            owned_run_termination_token,
                        ))
                        .map_or(OwnedRunTerminationSubmission::ActorUnavailable, |()| {
                            OwnedRunTerminationSubmission::Submitted(self.owned_run_id)
                        }),
                    OwnedRunCommandAdmission::ChildReaped => {
                        OwnedRunTerminationSubmission::ActorUnavailable
                    },
                }
            },
        }
    }

    /// Dispose of a process that never reached a live owned-run lifecycle.
    /// This is failed-launch cleanup, separate from user-requested termination.
    pub(crate) fn discard_unactivated(self) {
        let OwnedRunProcessActorState::Prepared(mut prepared_owned_run_process_actor) = self.state
        else {
            return;
        };
        prepared_owned_run_process_actor.child.discard_unactivated();
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        owned_run_id: OwnedRunId,
        process_identity: ProcessIdentity,
        test_signal_outcome: OwnedProcessGroupSignalOutcome,
    ) -> Self {
        let (owned_run_event_sender, _) = unbounded();
        Self {
            owned_run_id,
            state: OwnedRunProcessActorState::Prepared(PreparedOwnedRunProcessActor {
                child: OwnedRunChildProcess::Fixture,
                owned_process_group_termination_capability:
                    OwnedProcessGroupTerminationCapability::with_test_signal_outcome(
                        process_identity,
                        test_signal_outcome,
                    ),
                output_readers: Vec::new(),
                owned_run_event_sender,
            }),
        }
    }

    #[cfg(test)]
    fn prepare_with_test_signal_probe(
        owned_run_id: OwnedRunId,
        child: Child,
        verified_process_identity: VerifiedProcessIdentity,
        owned_run_event_sender: Sender<OwnedRunEvent>,
    ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let test_signal_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let process_identity = verified_process_identity.into_process_identity();
        (
            Self {
                owned_run_id,
                state: OwnedRunProcessActorState::Prepared(PreparedOwnedRunProcessActor {
                    child: OwnedRunChildProcess::Spawned(child),
                    owned_process_group_termination_capability:
                        OwnedProcessGroupTerminationCapability::with_test_signal_probe(
                            process_identity,
                            OwnedProcessGroupSignalOutcome::Sent,
                            test_signal_count.clone(),
                        ),
                    output_readers: Vec::new(),
                    owned_run_event_sender,
                }),
            },
            test_signal_count,
        )
    }
}

fn run_owned_run_process_worker(
    owned_run_id: OwnedRunId,
    mut prepared_owned_run_process_actor: PreparedOwnedRunProcessActor,
    owned_run_process_worker_channels: OwnedRunProcessWorkerChannels,
) {
    let OwnedRunProcessWorkerChannels {
        command_rx,
        command_admission,
    } = owned_run_process_worker_channels;
    let mut command_endpoint = OwnedRunCommandEndpoint::Connected;
    loop {
        if matches!(
            prepared_owned_run_process_actor.child.status(),
            OwnedRunChildStatus::Reaped
        ) {
            close_command_admission_after_reap(
                owned_run_id,
                &command_rx,
                &command_admission,
                &prepared_owned_run_process_actor.owned_run_event_sender,
            );
            break;
        }
        match command_endpoint {
            OwnedRunCommandEndpoint::Connected => {
                match command_rx.recv_timeout(OWNED_RUN_CHILD_REAP_POLL_INTERVAL) {
                    Ok(OwnedRunProcessCommand::Terminate(owned_run_termination_token)) => {
                        let owned_run_termination_outcome =
                            if owned_run_termination_token.0 == owned_run_id {
                                OwnedRunTerminationOutcome::Honored {
                                    owned_run_id,
                                    signal: prepared_owned_run_process_actor
                                        .owned_process_group_termination_capability
                                        .signal(),
                                }
                            } else {
                                OwnedRunTerminationOutcome::Refused {
                                    owned_run_id: owned_run_termination_token.0,
                                }
                            };
                        let _ = prepared_owned_run_process_actor
                            .owned_run_event_sender
                            .send(OwnedRunEvent::TerminationOutcome(
                                owned_run_termination_outcome,
                            ));
                    },
                    Err(RecvTimeoutError::Timeout) => {},
                    Err(RecvTimeoutError::Disconnected) => {
                        command_endpoint = OwnedRunCommandEndpoint::Disconnected;
                    },
                }
            },
            OwnedRunCommandEndpoint::Disconnected => {
                prepared_owned_run_process_actor.child.wait_until_reaped();
                close_command_admission_after_reap(
                    owned_run_id,
                    &command_rx,
                    &command_admission,
                    &prepared_owned_run_process_actor.owned_run_event_sender,
                );
                break;
            },
        }
    }

    for output_reader in prepared_owned_run_process_actor.output_readers {
        let _ = output_reader.join();
    }
    let _ = prepared_owned_run_process_actor
        .owned_run_event_sender
        .send(OwnedRunEvent::Finished { owned_run_id });
}

/// Stop accepting commands, then reconcile every command whose send completed
/// before the child was observed reaped. The worker holds
/// `OwnedRunCommandAdmission` until `command_rx` is empty, so no sender can
/// enqueue after draining begins.
fn close_command_admission_after_reap(
    owned_run_id: OwnedRunId,
    command_rx: &Receiver<OwnedRunProcessCommand>,
    command_admission: &Mutex<OwnedRunCommandAdmission>,
    owned_run_event_sender: &Sender<OwnedRunEvent>,
) {
    let mut command_admission = match command_admission.lock() {
        Ok(command_admission) => command_admission,
        Err(poisoned_command_admission) => poisoned_command_admission.into_inner(),
    };
    *command_admission = OwnedRunCommandAdmission::ChildReaped;
    while let Ok(OwnedRunProcessCommand::Terminate(owned_run_termination_token)) =
        command_rx.try_recv()
    {
        let owned_run_termination_outcome = if owned_run_termination_token.0 == owned_run_id {
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::ProcessAlreadyReaped,
            }
        } else {
            OwnedRunTerminationOutcome::Refused {
                owned_run_id: owned_run_termination_token.0,
            }
        };
        let _ = owned_run_event_sender.send(OwnedRunEvent::TerminationOutcome(
            owned_run_termination_outcome,
        ));
    }
    drop(command_admission);
}

/// Terminate a process group that was started but never verified into a live
/// owned run. This is failed-launch cleanup, so it is the one narrow path that
/// may escalate after a strong identity could not be established.
#[cfg(unix)]
pub(crate) fn discard_unverified_owned_process_group(child: &mut Child) {
    use std::process::Command;

    let process_group_id = child.id();
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(format!("-{process_group_id}"))
        .status();
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg(format!("-{process_group_id}"))
        .status();
    let _ = child.kill();
    let _ = child.wait();
    for _ in 0..UNACTIVATED_GROUP_EXIT_POLL_ATTEMPTS {
        if !process_group_exists(process_group_id) {
            break;
        }
        std::thread::sleep(UNACTIVATED_GROUP_EXIT_POLL_INTERVAL);
    }
}

#[cfg(unix)]
fn process_group_exists(process_group_id: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(format!("-{process_group_id}"))
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
pub(crate) fn discard_unverified_owned_process_group(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    #[cfg(unix)]
    use std::io::Write as _;
    use std::num::NonZeroU64;
    #[cfg(unix)]
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;
    #[cfg(unix)]
    use std::sync::atomic::Ordering;

    use super::*;
    #[cfg(unix)]
    use crate::process_observation::identity::CurrentProcessIdentityObservation;
    #[cfg(unix)]
    use crate::process_observation::identity::observe_current_process_identity;

    fn owned_run_id(value: u64) -> OwnedRunId {
        OwnedRunId::for_test(NonZeroU64::new(value).unwrap_or(NonZeroU64::MIN))
    }

    struct StartedActorFixture {
        owned_run_process_actor: OwnedRunProcessActor,
        owned_run_event_rx:      Receiver<OwnedRunEvent>,
    }

    fn started_actor(
        owned_run_id: OwnedRunId,
        test_signal_outcome: OwnedProcessGroupSignalOutcome,
    ) -> StartedActorFixture {
        let (owned_run_event_sender, owned_run_event_rx) = unbounded();
        let mut owned_run_process_actor = OwnedRunProcessActor {
            owned_run_id,
            state: OwnedRunProcessActorState::Prepared(PreparedOwnedRunProcessActor {
                child: OwnedRunChildProcess::Fixture,
                owned_process_group_termination_capability:
                    OwnedProcessGroupTerminationCapability::with_test_signal_outcome(
                        ProcessIdentity::for_test(4242, 7),
                        test_signal_outcome,
                    ),
                output_readers: Vec::new(),
                owned_run_event_sender,
            }),
        };
        owned_run_process_actor.start_worker();
        StartedActorFixture {
            owned_run_process_actor,
            owned_run_event_rx,
        }
    }

    fn await_outcome(owned_run_event_rx: &Receiver<OwnedRunEvent>) -> OwnedRunTerminationOutcome {
        match owned_run_event_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(OwnedRunEvent::TerminationOutcome(owned_run_termination_outcome)) => {
                owned_run_termination_outcome
            },
            Ok(_) => panic!("the actor worker returned an event before its outcome"),
            Err(error) => panic!("the actor worker returned no outcome: {error}"),
        }
    }

    #[cfg(unix)]
    struct RealActorFixture {
        owned_run_process_actor: OwnedRunProcessActor,
        child_stdin:             std::process::ChildStdin,
        owned_run_event_rx:      Receiver<OwnedRunEvent>,
        test_signal_count:       std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[cfg(unix)]
    impl RealActorFixture {
        fn spawn(owned_run_id: OwnedRunId) -> Self {
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg("IFS= read -r finished")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap_or_else(|error| panic!("the actor child fixture should spawn: {error}"));
            let child_stdin = child
                .stdin
                .take()
                .unwrap_or_else(|| panic!("the actor child fixture should have stdin"));
            let verified_process_identity = match observe_current_process_identity(child.id()) {
                CurrentProcessIdentityObservation::Verified(verified_process_identity) => {
                    verified_process_identity
                },
                observation => {
                    panic!(
                        "the actor child fixture should have a verified identity: {observation:?}"
                    )
                },
            };
            let (owned_run_event_sender, owned_run_event_rx) = unbounded();
            let (mut owned_run_process_actor, test_signal_count) =
                OwnedRunProcessActor::prepare_with_test_signal_probe(
                    owned_run_id,
                    child,
                    verified_process_identity,
                    owned_run_event_sender,
                );
            owned_run_process_actor.start_worker();
            Self {
                owned_run_process_actor,
                child_stdin,
                owned_run_event_rx,
                test_signal_count,
            }
        }

        fn release_child(mut child_stdin: std::process::ChildStdin) {
            child_stdin
                .write_all(b"finished\n")
                .unwrap_or_else(|error| panic!("the actor child fixture should exit: {error}"));
        }

        fn await_finished(owned_run_event_rx: &Receiver<OwnedRunEvent>, owned_run_id: OwnedRunId) {
            match owned_run_event_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(OwnedRunEvent::Finished {
                    owned_run_id: finished_owned_run_id,
                }) if finished_owned_run_id == owned_run_id => {},
                Ok(_) => panic!("the actor child fixture returned an unrelated message"),
                Err(error) => panic!("the actor child fixture did not finish: {error}"),
            }
        }
    }

    #[test]
    fn the_actor_honors_the_token_for_the_run_it_owns_without_blocking() {
        let owned_run_id = owned_run_id(1);
        let StartedActorFixture {
            owned_run_process_actor,
            owned_run_event_rx,
        } = started_actor(owned_run_id, OwnedProcessGroupSignalOutcome::Sent);
        assert_eq!(
            owned_run_process_actor.submit_termination(owned_run_process_actor.termination_token()),
            OwnedRunTerminationSubmission::Submitted(owned_run_id)
        );
        assert_eq!(
            await_outcome(&owned_run_event_rx),
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::Sent,
            }
        );
    }

    #[test]
    fn a_token_retained_from_an_earlier_run_is_refused_by_the_actor() {
        let StartedActorFixture {
            owned_run_process_actor,
            ..
        } = started_actor(owned_run_id(2), OwnedProcessGroupSignalOutcome::Sent);
        let retained_token_from_an_earlier_run = OwnedRunTerminationToken(owned_run_id(1));
        assert_eq!(
            owned_run_process_actor.submit_termination(retained_token_from_an_earlier_run),
            OwnedRunTerminationSubmission::TokenRefused
        );
    }

    #[test]
    fn the_actor_keeps_host_signal_outcomes_distinct() {
        for signal in [
            OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent,
            OwnedProcessGroupSignalOutcome::SignalFailed,
        ] {
            let owned_run_id = owned_run_id(3);
            let StartedActorFixture {
                owned_run_process_actor,
                owned_run_event_rx,
            } = started_actor(owned_run_id, signal);
            let _ = owned_run_process_actor
                .submit_termination(owned_run_process_actor.termination_token());
            assert_eq!(
                await_outcome(&owned_run_event_rx),
                OwnedRunTerminationOutcome::Honored {
                    owned_run_id,
                    signal
                }
            );
        }
    }

    #[test]
    fn actor_outcomes_remain_correlated_when_multiple_tokens_queue() {
        let owned_run_id = owned_run_id(4);
        let StartedActorFixture {
            owned_run_process_actor,
            owned_run_event_rx,
        } = started_actor(owned_run_id, OwnedProcessGroupSignalOutcome::Sent);
        let termination_token = owned_run_process_actor.termination_token();
        for _ in 0..4 {
            assert_eq!(
                owned_run_process_actor.submit_termination(termination_token),
                OwnedRunTerminationSubmission::Submitted(owned_run_id)
            );
        }
        for _ in 0..4 {
            assert_eq!(
                await_outcome(&owned_run_event_rx).owned_run_id(),
                owned_run_id
            );
        }
    }

    #[test]
    fn prepared_actor_refuses_submission_until_its_lifecycle_starts_the_worker() {
        let owned_run_process_actor = OwnedRunProcessActor::for_test(
            owned_run_id(5),
            ProcessIdentity::for_test(4242, 7),
            OwnedProcessGroupSignalOutcome::Sent,
        );
        assert_eq!(
            owned_run_process_actor.submit_termination(owned_run_process_actor.termination_token()),
            OwnedRunTerminationSubmission::ActorUnavailable
        );
    }

    #[cfg(unix)]
    #[test]
    fn endpoint_disconnection_keeps_ownership_until_the_real_child_is_reaped() {
        let owned_run_id = owned_run_id(6);
        let RealActorFixture {
            owned_run_process_actor,
            child_stdin,
            owned_run_event_rx,
            test_signal_count,
        } = RealActorFixture::spawn(owned_run_id);

        drop(owned_run_process_actor);
        RealActorFixture::release_child(child_stdin);
        RealActorFixture::await_finished(&owned_run_event_rx, owned_run_id);
        assert_eq!(test_signal_count.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn a_real_child_reaped_before_submission_receives_no_signal() {
        let owned_run_id = owned_run_id(7);
        let RealActorFixture {
            owned_run_process_actor,
            child_stdin,
            owned_run_event_rx,
            test_signal_count,
        } = RealActorFixture::spawn(owned_run_id);
        let termination_token = owned_run_process_actor.termination_token();

        RealActorFixture::release_child(child_stdin);
        RealActorFixture::await_finished(&owned_run_event_rx, owned_run_id);
        assert_eq!(
            owned_run_process_actor.submit_termination(termination_token),
            OwnedRunTerminationSubmission::ActorUnavailable
        );
        assert_eq!(test_signal_count.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn signaling_and_real_child_completion_are_serialized() {
        let owned_run_id = owned_run_id(8);
        let RealActorFixture {
            owned_run_process_actor,
            child_stdin,
            owned_run_event_rx,
            test_signal_count,
        } = RealActorFixture::spawn(owned_run_id);

        assert_eq!(
            owned_run_process_actor.submit_termination(owned_run_process_actor.termination_token()),
            OwnedRunTerminationSubmission::Submitted(owned_run_id)
        );
        assert_eq!(
            await_outcome(&owned_run_event_rx),
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::Sent,
            }
        );
        assert_eq!(test_signal_count.load(Ordering::SeqCst), 1);
        RealActorFixture::release_child(child_stdin);
        RealActorFixture::await_finished(&owned_run_event_rx, owned_run_id);
    }
}
