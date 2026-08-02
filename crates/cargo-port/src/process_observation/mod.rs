//! Immutable host process observation without process-control operations.

pub(crate) mod identity;
pub(crate) mod snapshot;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use identity::ObservedProcessIdentity;
use identity::PlatformProcessObservation;
use identity::ProcessIdentity;
use snapshot::FullProcessRefreshEvidence;
use snapshot::ProcessFieldObservation;
use snapshot::ProcessFieldSample;
use snapshot::ProcessFieldSourceObservation;
use snapshot::ProcessFieldUnavailable;
use snapshot::ProcessIncarnationCache;
use snapshot::ProcessObservationSnapshot;
use snapshot::ProcessRefreshInput;
use snapshot::ProcessRefreshObservations;
use snapshot::ProcessSamplingOutcome;
use snapshot::ProcessSnapshotScope;
use snapshot::ReportedParent;
use snapshot::TargetedProcessObservations;
use snapshot::TargetedProcessPresence;
use snapshot::TargetedProcessSamplingResult;
use snapshot::TargetedSampleAdmission;
use sysinfo::Pid;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

#[derive(Clone, Debug, Eq, PartialEq)]
enum PidProcessFieldObservation {
    Sampled(ProcessFieldSourceObservation),
    Unavailable(ProcessFieldUnavailable),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PidSamplingObservation {
    identity_before_fields: PlatformProcessObservation,
    process_fields:         PidProcessFieldObservation,
    identity_after_fields:  PlatformProcessObservation,
}

impl PidSamplingObservation {
    fn full_sampling_outcome(self) -> ProcessSamplingOutcome {
        let process_field_source_observation = match self.process_fields {
            PidProcessFieldObservation::Sampled(process_field_source_observation) => {
                process_field_source_observation
            },
            PidProcessFieldObservation::Unavailable(process_field_unavailable) => {
                ProcessFieldSourceObservation::repeated_unavailable_fresh_system_samples(
                    process_field_unavailable,
                )
            },
        };
        ProcessSamplingOutcome::bind_fields_to_identity(
            self.identity_before_fields,
            process_field_source_observation,
            self.identity_after_fields,
        )
    }

    fn targeted_process_presence(&self) -> TargetedProcessPresence {
        match &self.process_fields {
            PidProcessFieldObservation::Sampled(process_field_source_observation) => {
                TargetedProcessPresence::Sampled(ProcessSamplingOutcome::bind_fields_to_identity(
                    self.identity_before_fields.clone(),
                    process_field_source_observation.clone(),
                    self.identity_after_fields.clone(),
                ))
            },
            PidProcessFieldObservation::Unavailable(process_field_unavailable) => {
                TargetedProcessPresence::FieldsUnavailable {
                    process_sampling_outcome:  ProcessSamplingOutcome::bind_fields_to_identity(
                        self.identity_before_fields.clone(),
                        ProcessFieldSourceObservation::repeated_unavailable_fresh_system_samples(
                            process_field_unavailable.clone(),
                        ),
                        self.identity_after_fields.clone(),
                    ),
                    process_field_unavailable: process_field_unavailable.clone(),
                }
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdentityObservationSamplingPhase {
    BeforeFields,
    AfterFields,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ProcessIdentityObservationEvidence {
    Direct(ObservedProcessIdentity),
    ReportedParent(ObservedProcessIdentity),
}

impl ProcessIdentityObservationEvidence {
    fn reconcile_post_sampling_identity(
        &self,
        post_sampling_identities: &mut BTreeMap<u32, ObservedProcessIdentity>,
    ) {
        match self {
            Self::Direct(observed_identity) | Self::ReportedParent(observed_identity) => {
                post_sampling_identities.insert(observed_identity.pid(), observed_identity.clone());
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProcessIdentityObservationEvent {
    sampling_phase: IdentityObservationSamplingPhase,
    evidence:       ProcessIdentityObservationEvidence,
}

struct ProcessRefreshSamplingEvidence {
    pid_observations:  BTreeMap<Pid, PidSamplingObservation>,
    identity_timeline: Vec<ProcessIdentityObservationEvent>,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct FullRefreshDirectlySampledPids {
    pids: BTreeSet<u32>,
}

impl FullRefreshDirectlySampledPids {
    fn contains(&self, pid: u32) -> bool { self.pids.contains(&pid) }
}

impl From<&ProcessRefreshSamplingEvidence> for FullRefreshDirectlySampledPids {
    fn from(process_refresh_sampling_evidence: &ProcessRefreshSamplingEvidence) -> Self {
        Self {
            pids: process_refresh_sampling_evidence
                .pid_observations
                .keys()
                .map(|pid| pid.as_u32())
                .collect(),
        }
    }
}

impl ProcessRefreshSamplingEvidence {
    fn latest_post_sampling_identities(&self) -> BTreeMap<u32, ObservedProcessIdentity> {
        let mut post_sampling_identities = BTreeMap::new();
        for observation_event in &self.identity_timeline {
            match observation_event.sampling_phase {
                IdentityObservationSamplingPhase::BeforeFields => {},
                IdentityObservationSamplingPhase::AfterFields => {
                    observation_event
                        .evidence
                        .reconcile_post_sampling_identity(&mut post_sampling_identities);
                },
            }
        }
        post_sampling_identities
    }

    fn into_reconciled_sampling_outcomes(
        self,
        post_sampling_identities: &BTreeMap<u32, ObservedProcessIdentity>,
    ) -> Vec<ProcessSamplingOutcome> {
        self.pid_observations
            .into_iter()
            .map(|(pid, pid_sampling_observation)| {
                pid_sampling_observation
                    .full_sampling_outcome()
                    .reconcile_later_identity_observation(&post_sampling_identities[&pid.as_u32()])
            })
            .collect()
    }
}

struct TargetedPidClassification {
    observations: BTreeMap<ProcessIdentity, snapshot::TargetedProcessObservation>,
    admission:    TargetedSampleAdmission,
}

/// Host-only process observation backed by one private `sysinfo::System`.
#[derive(Default)]
pub(crate) struct ProcessObserver {
    system:            System,
    incarnation_cache: ProcessIncarnationCache,
}

impl ProcessObserver {
    /// Refresh a full or selected process set and return only immutable evidence.
    pub(crate) fn refresh(
        &mut self,
        process_refresh_input: ProcessRefreshInput,
    ) -> ProcessObservationSnapshot {
        let refresh_kind = ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always);
        let process_refresh_observations = match &process_refresh_input {
            ProcessRefreshInput::FullSystemSnapshot => {
                let cached_process_identities = self.incarnation_cache.cached_process_identities();
                self.refresh_full_system_observations(refresh_kind, &cached_process_identities)
            },
            ProcessRefreshInput::TargetedIdentities(process_identities) => {
                self.refresh_targeted_observations(process_identities, refresh_kind)
            },
        };

        let scope = snapshot_scope(&process_refresh_input);
        self.incarnation_cache.snapshot_from(
            std::time::Instant::now(),
            scope,
            process_refresh_observations,
        )
    }

    fn refresh_full_system_observations(
        &mut self,
        refresh_kind: ProcessRefreshKind,
        cached_process_identities: &BTreeSet<ProcessIdentity>,
    ) -> ProcessRefreshObservations {
        let updated_processes = self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        if updated_processes == 0 {
            return ProcessRefreshObservations {
                process_sampling_outcomes:     Vec::new(),
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence: FullProcessRefreshEvidence::NoProcessesUpdated,
            };
        }
        let pids: Vec<Pid> = self.system.processes().keys().copied().collect();
        let process_refresh_sampling_evidence = Self::observe_pids_with(
            &pids,
            |pid| PlatformProcessObservation::observe(pid),
            |pids| Self::refresh_process_field_sources(pids, refresh_kind),
        );
        let directly_sampled_pids =
            FullRefreshDirectlySampledPids::from(&process_refresh_sampling_evidence);
        let post_sampling_identities =
            process_refresh_sampling_evidence.latest_post_sampling_identities();
        let latest_identity_observations = Self::finalize_full_refresh_identity_observations(
            cached_process_identities,
            &directly_sampled_pids,
            post_sampling_identities,
            |pid| {
                PlatformProcessObservation::observe_lifetime(pid)
                    .identity()
                    .clone()
            },
        );
        let process_sampling_outcomes = process_refresh_sampling_evidence
            .into_reconciled_sampling_outcomes(&latest_identity_observations);
        let full_process_refresh_evidence = FullProcessRefreshEvidence::UpdatedProcesses {
            latest_identity_observations,
        };
        ProcessRefreshObservations {
            process_sampling_outcomes,
            targeted_process_observations: TargetedProcessObservations::NotRequested,
            full_process_refresh_evidence,
        }
    }

    fn finalize_full_refresh_identity_observations(
        cached_process_identities: &BTreeSet<ProcessIdentity>,
        directly_sampled_pids: &FullRefreshDirectlySampledPids,
        mut latest_identity_observations: BTreeMap<u32, ObservedProcessIdentity>,
        mut observe_omitted_pid: impl FnMut(u32) -> ObservedProcessIdentity,
    ) -> BTreeMap<u32, ObservedProcessIdentity> {
        let omitted_cached_pids: BTreeSet<u32> = cached_process_identities
            .iter()
            .map(ProcessIdentity::pid)
            .filter(|pid| !directly_sampled_pids.contains(*pid))
            .collect();
        for pid in omitted_cached_pids {
            latest_identity_observations.insert(pid, observe_omitted_pid(pid));
        }
        latest_identity_observations
    }

    fn refresh_targeted_observations(
        &mut self,
        process_identities: &BTreeSet<ProcessIdentity>,
        refresh_kind: ProcessRefreshKind,
    ) -> ProcessRefreshObservations {
        let mut requested_identities_by_pid: BTreeMap<u32, Vec<ProcessIdentity>> = BTreeMap::new();
        for process_identity in process_identities {
            requested_identities_by_pid
                .entry(process_identity.pid())
                .or_default()
                .push(process_identity.clone());
        }
        let pids: Vec<Pid> = requested_identities_by_pid
            .keys()
            .copied()
            .map(Pid::from_u32)
            .collect();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&pids),
            true,
            ProcessRefreshKind::nothing(),
        );
        let process_refresh_sampling_evidence = Self::observe_pids_with(
            &pids,
            |pid| PlatformProcessObservation::observe(pid),
            |pids| Self::refresh_process_field_sources(pids, refresh_kind),
        );
        let post_sampling_identities =
            process_refresh_sampling_evidence.latest_post_sampling_identities();

        Self::targeted_process_refresh_observations(
            requested_identities_by_pid,
            process_refresh_sampling_evidence,
            &post_sampling_identities,
        )
    }

    fn targeted_process_refresh_observations(
        requested_identities_by_pid: BTreeMap<u32, Vec<ProcessIdentity>>,
        process_refresh_sampling_evidence: ProcessRefreshSamplingEvidence,
        post_sampling_identities: &BTreeMap<u32, ObservedProcessIdentity>,
    ) -> ProcessRefreshObservations {
        let mut process_sampling_outcomes = Vec::new();
        let mut targeted_process_observations = BTreeMap::new();
        for (pid, requested_identities) in requested_identities_by_pid {
            let TargetedPidClassification {
                observations,
                admission,
            } = Self::classify_targeted_pid(
                &requested_identities,
                &process_refresh_sampling_evidence.pid_observations[&Pid::from_u32(pid)],
                &post_sampling_identities[&pid],
            );
            targeted_process_observations.extend(observations);
            match admission {
                TargetedSampleAdmission::Admitted(process_sampling_outcome) => {
                    process_sampling_outcomes.push(process_sampling_outcome);
                },
                TargetedSampleAdmission::Excluded => {},
            }
        }
        ProcessRefreshObservations {
            process_sampling_outcomes,
            targeted_process_observations: TargetedProcessObservations::Outcomes(
                targeted_process_observations,
            ),
            full_process_refresh_evidence: FullProcessRefreshEvidence::NotRequested,
        }
    }

    fn observe_pids_with(
        pids: &[Pid],
        mut observe_identity: impl FnMut(u32) -> PlatformProcessObservation,
        observe_fields: impl FnOnce(&[Pid]) -> BTreeMap<Pid, ProcessFieldSourceObservation>,
    ) -> ProcessRefreshSamplingEvidence {
        let mut identity_timeline = Vec::new();
        let mut identity_observations_before_fields = BTreeMap::new();
        for pid in pids {
            let identity_before_fields = observe_identity(pid.as_u32());
            Self::record_identity_observation_events(
                IdentityObservationSamplingPhase::BeforeFields,
                &identity_before_fields,
                &mut identity_timeline,
            );
            identity_observations_before_fields.insert(*pid, identity_before_fields);
        }
        let process_field_sources = observe_fields(pids);
        let mut pid_observations = BTreeMap::new();
        for pid in pids {
            let process_fields = process_field_sources.get(pid).map_or_else(
                || {
                    PidProcessFieldObservation::Unavailable(
                        ProcessFieldUnavailable::PlatformLookupFailed,
                    )
                },
                |process_field_source_observation| {
                    PidProcessFieldObservation::Sampled(process_field_source_observation.clone())
                },
            );
            let identity_after_fields = observe_identity(pid.as_u32());
            Self::record_identity_observation_events(
                IdentityObservationSamplingPhase::AfterFields,
                &identity_after_fields,
                &mut identity_timeline,
            );
            pid_observations.insert(
                *pid,
                PidSamplingObservation {
                    identity_before_fields: identity_observations_before_fields[pid].clone(),
                    process_fields,
                    identity_after_fields,
                },
            );
        }
        ProcessRefreshSamplingEvidence {
            pid_observations,
            identity_timeline,
        }
    }

    fn record_identity_observation_events(
        sampling_phase: IdentityObservationSamplingPhase,
        platform_observation: &PlatformProcessObservation,
        identity_timeline: &mut Vec<ProcessIdentityObservationEvent>,
    ) {
        identity_timeline.push(ProcessIdentityObservationEvent {
            sampling_phase,
            evidence: ProcessIdentityObservationEvidence::Direct(
                platform_observation.lifetime.identity().clone(),
            ),
        });
        match &platform_observation.parent {
            ProcessFieldObservation::Observed(ReportedParent::Identified(parent_identity)) => {
                identity_timeline.push(ProcessIdentityObservationEvent {
                    sampling_phase,
                    evidence: ProcessIdentityObservationEvidence::ReportedParent(
                        ObservedProcessIdentity::Strong(parent_identity.clone()),
                    ),
                });
            },
            ProcessFieldObservation::Observed(ReportedParent::IdentityUnavailable(
                insufficient_identity,
            )) => {
                identity_timeline.push(ProcessIdentityObservationEvent {
                    sampling_phase,
                    evidence: ProcessIdentityObservationEvidence::ReportedParent(
                        ObservedProcessIdentity::Insufficient(insufficient_identity.clone()),
                    ),
                });
            },
            ProcessFieldObservation::Observed(ReportedParent::Root)
            | ProcessFieldObservation::Unavailable(_)
            | ProcessFieldObservation::Invalidated(_) => {},
        }
    }

    fn classify_targeted_pid(
        requested_identities: &[ProcessIdentity],
        pid_sampling_observation: &PidSamplingObservation,
        later_identity: &ObservedProcessIdentity,
    ) -> TargetedPidClassification {
        let mut observations = BTreeMap::new();
        let mut admission = TargetedSampleAdmission::Excluded;
        let targeted_process_presence = pid_sampling_observation
            .targeted_process_presence()
            .reconcile_later_identity_observation(later_identity);
        for process_identity in requested_identities {
            let TargetedProcessSamplingResult {
                observation,
                admission: current_admission,
            } = TargetedProcessSamplingResult::classify(
                process_identity,
                targeted_process_presence.clone(),
            );
            observations.insert(process_identity.clone(), observation);
            if let TargetedSampleAdmission::Admitted(process_sampling_outcome) = current_admission {
                admission = TargetedSampleAdmission::Admitted(process_sampling_outcome);
            }
        }
        TargetedPidClassification {
            observations,
            admission,
        }
    }

    /// Samples fields through two newly created `System` instances on every platform.
    /// `ProcessObserver::system` remains available for future CPU history, but
    /// its command fields never enter `ProcessFieldSourceObservation`.
    fn refresh_process_field_sources(
        pids: &[Pid],
        refresh_kind: ProcessRefreshKind,
    ) -> BTreeMap<Pid, ProcessFieldSourceObservation> {
        let mut initial_field_system = System::new();
        initial_field_system.refresh_processes_specifics(
            ProcessesToUpdate::Some(pids),
            true,
            refresh_kind.clone(),
        );
        let mut repeated_field_system = System::new();
        repeated_field_system.refresh_processes_specifics(
            ProcessesToUpdate::Some(pids),
            true,
            refresh_kind,
        );
        pids.iter()
            .filter_map(|pid| {
                match (
                    initial_field_system.process(*pid),
                    repeated_field_system.process(*pid),
                ) {
                    (Some(initial), Some(repeated)) => Some((
                        *pid,
                        ProcessFieldSourceObservation::repeated_fresh_system_samples(
                            ProcessFieldSample::observe(initial),
                            ProcessFieldSample::observe(repeated),
                        ),
                    )),
                    (Some(_), None) | (None, Some(_)) => Some((
                        *pid,
                        ProcessFieldSourceObservation::fresh_system_stability_unproven(),
                    )),
                    (None, None) => None,
                }
            })
            .collect()
    }
}

fn snapshot_scope(process_refresh_input: &ProcessRefreshInput) -> ProcessSnapshotScope {
    match process_refresh_input {
        ProcessRefreshInput::FullSystemSnapshot => ProcessSnapshotScope::FullSystem,
        ProcessRefreshInput::TargetedIdentities(process_identities) => {
            ProcessSnapshotScope::TargetedIdentities(process_identities.clone())
        },
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::io::Write as _;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::time::Duration;
    use std::time::Instant;

    use super::FullRefreshDirectlySampledPids;
    use super::PidProcessFieldObservation;
    use super::PidSamplingObservation;
    use super::PlatformProcessObservation;
    use super::ProcessObserver;
    use super::ProcessRefreshSamplingEvidence;
    use crate::process_observation::identity::InsufficientProcessIdentity;
    use crate::process_observation::identity::ObservedProcessIdentity;
    use crate::process_observation::identity::ProcessCreationOrderEvidence;
    use crate::process_observation::identity::ProcessIdentity;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use crate::process_observation::snapshot::AncestryLookup;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use crate::process_observation::snapshot::AncestryTerminal;
    use crate::process_observation::snapshot::FullProcessRefreshEvidence;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use crate::process_observation::snapshot::ParentWalkDepth;
    use crate::process_observation::snapshot::ParentageValidationOutcome;
    use crate::process_observation::snapshot::ProcessFieldInvalidation;
    use crate::process_observation::snapshot::ProcessFieldLifetimeBinding;
    use crate::process_observation::snapshot::ProcessFieldObservation;
    use crate::process_observation::snapshot::ProcessFieldSample;
    use crate::process_observation::snapshot::ProcessFieldSourceObservation;
    use crate::process_observation::snapshot::ProcessFieldUnavailable;
    use crate::process_observation::snapshot::ProcessIdentityBindingInvalidation;
    use crate::process_observation::snapshot::ProcessIncarnationEvidence;
    use crate::process_observation::snapshot::ProcessIncarnationState;
    use crate::process_observation::snapshot::ProcessObservationSnapshot;
    use crate::process_observation::snapshot::ProcessRefreshInput;
    use crate::process_observation::snapshot::ProcessRefreshObservations;
    use crate::process_observation::snapshot::ProcessSamplingOutcome;
    use crate::process_observation::snapshot::ProcessSnapshotScope;
    use crate::process_observation::snapshot::ReportedParent;
    use crate::process_observation::snapshot::TargetedProcessObservation;
    use crate::process_observation::snapshot::TargetedProcessObservations;
    use crate::process_observation::snapshot::TargetedSampleAdmission;

    fn strong_identity(pid: u32) -> std::io::Result<ProcessIdentity> {
        match PlatformProcessObservation::observe_lifetime(pid)
            .identity()
            .clone()
        {
            ObservedProcessIdentity::Strong(process_identity) => Ok(process_identity),
            ObservedProcessIdentity::Insufficient(insufficient_identity) => {
                Err(std::io::Error::other(format!(
                    "process identity was insufficient: {insufficient_identity:?}"
                )))
            },
        }
    }

    fn platform_observation(process_identity: &ProcessIdentity) -> PlatformProcessObservation {
        PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test_identity(process_identity),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        )
    }

    fn platform_observation_with_parent(
        process_identity: &ProcessIdentity,
        parent_identity: &ProcessIdentity,
    ) -> PlatformProcessObservation {
        PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test_identity(process_identity),
            ProcessFieldObservation::Observed(ReportedParent::Identified(parent_identity.clone())),
        )
    }

    fn platform_observation_with_insufficient_parent(
        process_identity: &ProcessIdentity,
        insufficient_parent_identity: &InsufficientProcessIdentity,
    ) -> PlatformProcessObservation {
        PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test_identity(process_identity),
            ProcessFieldObservation::Observed(ReportedParent::IdentityUnavailable(
                insufficient_parent_identity.clone(),
            )),
        )
    }

    fn cargo_field_source() -> ProcessFieldSourceObservation {
        let process_field_sample = ProcessFieldSample::for_test(
            PathBuf::from("/usr/bin/cargo"),
            vec!["cargo".into()],
            PathBuf::from("/workspace"),
        );
        ProcessFieldSourceObservation::repeated_fresh_system_samples(
            process_field_sample.clone(),
            process_field_sample,
        )
    }

    fn synthetic_sampling_evidence(
        pids: &[sysinfo::Pid],
        identity_observations: Vec<PlatformProcessObservation>,
    ) -> ProcessRefreshSamplingEvidence {
        let next_observation = Cell::new(0);
        ProcessObserver::observe_pids_with(
            pids,
            |_| {
                let observation_index = next_observation.get();
                next_observation.set(observation_index + 1);
                identity_observations[observation_index].clone()
            },
            |pids| {
                pids.iter()
                    .map(|pid| (*pid, cargo_field_source()))
                    .collect()
            },
        )
    }

    fn full_snapshot_from_sampling_evidence(
        process_observer: &mut ProcessObserver,
        process_refresh_sampling_evidence: ProcessRefreshSamplingEvidence,
    ) -> ProcessObservationSnapshot {
        let post_sampling_identities =
            process_refresh_sampling_evidence.latest_post_sampling_identities();
        let process_sampling_outcomes = process_refresh_sampling_evidence
            .into_reconciled_sampling_outcomes(&post_sampling_identities);
        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::FullSystem,
            ProcessRefreshObservations {
                process_sampling_outcomes,
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence: FullProcessRefreshEvidence::UpdatedProcesses {
                    latest_identity_observations: post_sampling_identities,
                },
            },
        )
    }

    fn targeted_snapshot_from_sampling_evidence(
        process_observer: &mut ProcessObserver,
        requested_identities: BTreeSet<ProcessIdentity>,
        process_refresh_sampling_evidence: ProcessRefreshSamplingEvidence,
    ) -> ProcessObservationSnapshot {
        let mut requested_identities_by_pid: BTreeMap<u32, Vec<ProcessIdentity>> = BTreeMap::new();
        for process_identity in &requested_identities {
            requested_identities_by_pid
                .entry(process_identity.pid())
                .or_default()
                .push(process_identity.clone());
        }
        let post_sampling_identities =
            process_refresh_sampling_evidence.latest_post_sampling_identities();
        let process_refresh_observations = ProcessObserver::targeted_process_refresh_observations(
            requested_identities_by_pid,
            process_refresh_sampling_evidence,
            &post_sampling_identities,
        );
        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::TargetedIdentities(requested_identities),
            process_refresh_observations,
        )
    }

    fn prime_incarnation_cache(
        process_observer: &mut ProcessObserver,
        process_identity: &ProcessIdentity,
    ) {
        let platform_observation = platform_observation(process_identity);
        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            platform_observation.clone(),
            cargo_field_source(),
            platform_observation,
        );
        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::FullSystem,
            ProcessRefreshObservations {
                process_sampling_outcomes:     vec![process_sampling_outcome],
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence: FullProcessRefreshEvidence::NotRequested,
            },
        );
    }

    fn apply_full_refresh_identity_observations(
        process_observer: &mut ProcessObserver,
        latest_identity_observations: BTreeMap<u32, ObservedProcessIdentity>,
    ) {
        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::FullSystem,
            ProcessRefreshObservations {
                process_sampling_outcomes:     Vec::new(),
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence: FullProcessRefreshEvidence::UpdatedProcesses {
                    latest_identity_observations,
                },
            },
        );
    }

    #[test]
    fn platform_boundary_observes_current_and_missing_processes() {
        assert!(matches!(
            PlatformProcessObservation::observe_lifetime(std::process::id()).identity(),
            ObservedProcessIdentity::Strong(_)
        ));
        assert!(matches!(
            PlatformProcessObservation::observe_lifetime(u32::MAX).identity(),
            ObservedProcessIdentity::Insufficient(_)
        ));
    }

    #[test]
    fn production_field_adapter_uses_repeated_fresh_system_samples() -> std::io::Result<()> {
        let pid = sysinfo::Pid::from_u32(std::process::id());
        let refresh_kind = sysinfo::ProcessRefreshKind::nothing()
            .with_exe(sysinfo::UpdateKind::Always)
            .with_cmd(sysinfo::UpdateKind::Always)
            .with_cwd(sysinfo::UpdateKind::Always);
        let process_field_sources =
            ProcessObserver::refresh_process_field_sources(&[pid], refresh_kind);
        let Some(process_field_source) = process_field_sources.get(&pid) else {
            return Err(std::io::Error::other(
                "fresh process field system did not return the current process",
            ));
        };

        assert!(matches!(
            process_field_source.lifetime_binding(),
            ProcessFieldLifetimeBinding::FreshSystemSamplingInterval
        ));
        Ok(())
    }

    #[test]
    fn same_pid_targets_share_one_identity_pair_and_one_field_observation() {
        let pid = sysinfo::Pid::from_u32(50);
        let stale_identity = ProcessIdentity::for_test(pid.as_u32(), 499);
        let current_identity = ProcessIdentity::for_test(pid.as_u32(), 500);
        let events = RefCell::new(Vec::new());
        let identity_observations = Cell::new(0);

        let process_refresh_sampling_evidence = ProcessObserver::observe_pids_with(
            &[pid],
            |_| {
                identity_observations.set(identity_observations.get() + 1);
                events.borrow_mut().push("identity");
                PlatformProcessObservation::for_test(
                    ObservedProcessIdentity::Strong(current_identity.clone()),
                    ProcessCreationOrderEvidence::for_test(500),
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                )
            },
            |_| {
                events.borrow_mut().push("fields");
                BTreeMap::from([(
                    pid,
                    ProcessFieldSourceObservation::repeated_unavailable_fresh_system_samples(
                        ProcessFieldUnavailable::PlatformDidNotReport,
                    ),
                )])
            },
        );
        let post_sampling_identities =
            process_refresh_sampling_evidence.latest_post_sampling_identities();
        let classification = ProcessObserver::classify_targeted_pid(
            &[stale_identity.clone(), current_identity.clone()],
            &process_refresh_sampling_evidence.pid_observations[&pid],
            &post_sampling_identities[&pid.as_u32()],
        );

        assert_eq!(identity_observations.get(), 2);
        assert_eq!(&*events.borrow(), &["identity", "fields", "identity"]);
        assert_eq!(
            classification.observations.get(&stale_identity),
            Some(&TargetedProcessObservation::Replaced {
                replacement: current_identity.clone(),
            })
        );
        assert_eq!(
            classification.observations.get(&current_identity),
            Some(&TargetedProcessObservation::Observed)
        );
        assert!(matches!(
            classification.admission,
            TargetedSampleAdmission::Admitted(ProcessSamplingOutcome::IdentityBound(_))
        ));
    }

    #[test]
    fn omitted_same_pid_lifetimes_share_one_strong_lookup() {
        let historical_identity = ProcessIdentity::for_test(70, 700);
        let current_identity = ProcessIdentity::for_test(70, 701);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &historical_identity);
        prime_incarnation_cache(&mut process_observer, &current_identity);
        let cached_process_identities = process_observer
            .incarnation_cache
            .cached_process_identities();
        let lookup_count = Cell::new(0);

        let latest_identity_observations =
            ProcessObserver::finalize_full_refresh_identity_observations(
                &cached_process_identities,
                &FullRefreshDirectlySampledPids::default(),
                BTreeMap::new(),
                |pid| {
                    lookup_count.set(lookup_count.get() + 1);
                    assert_eq!(pid, current_identity.pid());
                    ObservedProcessIdentity::Strong(current_identity.clone())
                },
            );
        apply_full_refresh_identity_observations(
            &mut process_observer,
            latest_identity_observations,
        );

        assert_eq!(lookup_count.get(), 1);
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&historical_identity)
        );
        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&current_identity)
        );
    }

    #[test]
    fn omitted_pid_direct_identity_replaces_reported_parent_identity() {
        let historical_identity = ProcessIdentity::for_test(74, 740);
        let current_identity = ProcessIdentity::for_test(74, 741);
        let child_pid = sysinfo::Pid::from_u32(75);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 750);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &historical_identity);
        prime_incarnation_cache(&mut process_observer, &current_identity);
        let cached_process_identities = process_observer
            .incarnation_cache
            .cached_process_identities();
        let process_refresh_sampling_evidence = synthetic_sampling_evidence(
            &[child_pid],
            vec![
                platform_observation_with_parent(&child_identity, &historical_identity),
                platform_observation_with_parent(&child_identity, &historical_identity),
            ],
        );
        let directly_sampled_pids =
            FullRefreshDirectlySampledPids::from(&process_refresh_sampling_evidence);
        let parent_only_identity_observations =
            process_refresh_sampling_evidence.latest_post_sampling_identities();
        let lookup_count = Cell::new(0);

        assert!(!directly_sampled_pids.contains(current_identity.pid()));
        assert_eq!(
            parent_only_identity_observations.get(&current_identity.pid()),
            Some(&ObservedProcessIdentity::Strong(
                historical_identity.clone()
            ))
        );

        let latest_identity_observations =
            ProcessObserver::finalize_full_refresh_identity_observations(
                &cached_process_identities,
                &directly_sampled_pids,
                parent_only_identity_observations,
                |pid| {
                    lookup_count.set(lookup_count.get() + 1);
                    assert_eq!(pid, current_identity.pid());
                    ObservedProcessIdentity::Strong(current_identity.clone())
                },
            );

        assert_eq!(lookup_count.get(), 1);
        assert_eq!(
            latest_identity_observations.get(&current_identity.pid()),
            Some(&ObservedProcessIdentity::Strong(current_identity.clone()))
        );
        apply_full_refresh_identity_observations(
            &mut process_observer,
            latest_identity_observations,
        );

        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&historical_identity)
        );
        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&current_identity)
        );
    }

    #[test]
    fn omitted_pid_direct_exit_replaces_reported_parent_identity() {
        let historical_identity = ProcessIdentity::for_test(76, 760);
        let current_identity = ProcessIdentity::for_test(76, 761);
        let child_pid = sysinfo::Pid::from_u32(77);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 770);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &historical_identity);
        prime_incarnation_cache(&mut process_observer, &current_identity);
        let cached_process_identities = process_observer
            .incarnation_cache
            .cached_process_identities();
        let process_refresh_sampling_evidence = synthetic_sampling_evidence(
            &[child_pid],
            vec![
                platform_observation_with_parent(&child_identity, &current_identity),
                platform_observation_with_parent(&child_identity, &current_identity),
            ],
        );
        let directly_sampled_pids =
            FullRefreshDirectlySampledPids::from(&process_refresh_sampling_evidence);
        let parent_only_identity_observations =
            process_refresh_sampling_evidence.latest_post_sampling_identities();
        let lookup_count = Cell::new(0);
        let process_exit = ObservedProcessIdentity::Insufficient(
            InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup {
                pid: current_identity.pid(),
            },
        );

        assert_eq!(
            parent_only_identity_observations.get(&current_identity.pid()),
            Some(&ObservedProcessIdentity::Strong(current_identity.clone()))
        );

        let latest_identity_observations =
            ProcessObserver::finalize_full_refresh_identity_observations(
                &cached_process_identities,
                &directly_sampled_pids,
                parent_only_identity_observations,
                |pid| {
                    lookup_count.set(lookup_count.get() + 1);
                    assert_eq!(pid, current_identity.pid());
                    process_exit.clone()
                },
            );

        assert_eq!(lookup_count.get(), 1);
        assert_eq!(
            latest_identity_observations.get(&current_identity.pid()),
            Some(&process_exit)
        );
        apply_full_refresh_identity_observations(
            &mut process_observer,
            latest_identity_observations,
        );

        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&historical_identity)
        );
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&current_identity)
        );
    }

    #[test]
    fn omitted_same_pid_lifetimes_share_one_exit_lookup() {
        let historical_identity = ProcessIdentity::for_test(71, 710);
        let current_identity = ProcessIdentity::for_test(71, 711);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &historical_identity);
        prime_incarnation_cache(&mut process_observer, &current_identity);
        let cached_process_identities = process_observer
            .incarnation_cache
            .cached_process_identities();

        let latest_identity_observations =
            ProcessObserver::finalize_full_refresh_identity_observations(
                &cached_process_identities,
                &FullRefreshDirectlySampledPids::default(),
                BTreeMap::new(),
                |pid| {
                    ObservedProcessIdentity::Insufficient(
                        InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid },
                    )
                },
            );
        apply_full_refresh_identity_observations(
            &mut process_observer,
            latest_identity_observations,
        );

        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&historical_identity)
        );
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&current_identity)
        );
    }

    #[test]
    fn omitted_same_pid_lifetimes_share_one_insufficient_lookup() {
        let historical_identity = ProcessIdentity::for_test(72, 720);
        let current_identity = ProcessIdentity::for_test(72, 721);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &historical_identity);
        prime_incarnation_cache(&mut process_observer, &current_identity);
        let cached_process_identities = process_observer
            .incarnation_cache
            .cached_process_identities();

        let latest_identity_observations =
            ProcessObserver::finalize_full_refresh_identity_observations(
                &cached_process_identities,
                &FullRefreshDirectlySampledPids::default(),
                BTreeMap::new(),
                |pid| {
                    ObservedProcessIdentity::Insufficient(
                        InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid },
                    )
                },
            );
        apply_full_refresh_identity_observations(
            &mut process_observer,
            latest_identity_observations,
        );

        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&historical_identity)
        );
        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&current_identity)
        );
    }

    #[test]
    fn omitted_pid_boundary_cannot_observe_insufficient_then_strong() {
        let historical_identity = ProcessIdentity::for_test(73, 730);
        let current_identity = ProcessIdentity::for_test(73, 731);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &historical_identity);
        prime_incarnation_cache(&mut process_observer, &current_identity);
        let cached_process_identities = process_observer
            .incarnation_cache
            .cached_process_identities();
        let identity_observations = [
            ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                    pid: current_identity.pid(),
                },
            ),
            ObservedProcessIdentity::Strong(current_identity.clone()),
        ];
        let next_identity_observation = Cell::new(0);

        let latest_identity_observations =
            ProcessObserver::finalize_full_refresh_identity_observations(
                &cached_process_identities,
                &FullRefreshDirectlySampledPids::default(),
                BTreeMap::new(),
                |_| {
                    let observation_index = next_identity_observation.get();
                    next_identity_observation.set(observation_index + 1);
                    identity_observations[observation_index].clone()
                },
            );
        assert_eq!(next_identity_observation.get(), 1);
        assert_eq!(
            latest_identity_observations.get(&current_identity.pid()),
            Some(&identity_observations[0])
        );
        apply_full_refresh_identity_observations(
            &mut process_observer,
            latest_identity_observations,
        );

        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&historical_identity)
        );
        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&current_identity)
        );
    }

    #[test]
    fn post_sampling_disappearance_evicts_the_before_fields_incarnation() {
        let pid = sysinfo::Pid::from_u32(51);
        let old_identity = ProcessIdentity::for_test(pid.as_u32(), 510);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &old_identity);
        let pid_sampling_observation = PidSamplingObservation {
            identity_before_fields: platform_observation(&old_identity),
            process_fields:         PidProcessFieldObservation::Sampled(cargo_field_source()),
            identity_after_fields:  PlatformProcessObservation::for_test(
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup {
                        pid: pid.as_u32(),
                    },
                ),
                ProcessCreationOrderEvidence::unavailable_for_test(),
                ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::ProcessExited),
            ),
        };
        let post_sampling_identities = BTreeMap::from([(
            pid.as_u32(),
            pid_sampling_observation
                .identity_after_fields
                .lifetime
                .identity()
                .clone(),
        )]);

        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::FullSystem,
            ProcessRefreshObservations {
                process_sampling_outcomes:     vec![
                    pid_sampling_observation.full_sampling_outcome(),
                ],
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence: FullProcessRefreshEvidence::UpdatedProcesses {
                    latest_identity_observations: post_sampling_identities,
                },
            },
        );

        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&old_identity)
        );
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_unclassified_candidate(&old_identity)
        );
    }

    #[test]
    fn post_sampling_replacement_evicts_the_before_fields_incarnation() {
        let pid = sysinfo::Pid::from_u32(52);
        let old_identity = ProcessIdentity::for_test(pid.as_u32(), 520);
        let replacement_identity = ProcessIdentity::for_test(pid.as_u32(), 521);
        let mut process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut process_observer, &old_identity);
        let pid_sampling_observation = PidSamplingObservation {
            identity_before_fields: platform_observation(&old_identity),
            process_fields:         PidProcessFieldObservation::Sampled(cargo_field_source()),
            identity_after_fields:  platform_observation(&replacement_identity),
        };
        let post_sampling_identities = BTreeMap::from([(
            pid.as_u32(),
            pid_sampling_observation
                .identity_after_fields
                .lifetime
                .identity()
                .clone(),
        )]);

        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::FullSystem,
            ProcessRefreshObservations {
                process_sampling_outcomes:     vec![
                    pid_sampling_observation.full_sampling_outcome(),
                ],
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence: FullProcessRefreshEvidence::UpdatedProcesses {
                    latest_identity_observations: post_sampling_identities,
                },
            },
        );

        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&old_identity)
        );
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_unclassified_candidate(&old_identity)
        );
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&replacement_identity)
        );
    }

    #[test]
    fn later_reported_parent_identity_supersedes_an_earlier_direct_identity() {
        let parent_pid = sysinfo::Pid::from_u32(60);
        let child_pid = sysinfo::Pid::from_u32(61);
        let sampled_parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 600);
        let later_parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 601);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 610);
        let identity_observations = vec![
            platform_observation(&sampled_parent_identity),
            platform_observation_with_parent(&child_identity, &later_parent_identity),
            platform_observation(&sampled_parent_identity),
            platform_observation_with_parent(&child_identity, &later_parent_identity),
        ];

        let mut full_process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut full_process_observer, &sampled_parent_identity);
        let full_snapshot = full_snapshot_from_sampling_evidence(
            &mut full_process_observer,
            synthetic_sampling_evidence(&[parent_pid, child_pid], identity_observations.clone()),
        );

        assert!(
            !full_snapshot
                .strongly_identified_processes()
                .contains_key(&sampled_parent_identity)
        );
        assert!(
            !full_snapshot
                .strongly_identified_processes()
                .contains_key(&later_parent_identity)
        );
        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            full_snapshot.identity_binding_invalidations(),
            [ProcessIdentityBindingInvalidation::LaterIdentityObservationChangedCurrentEvidence {
                prior: ObservedProcessIdentity::Strong(prior),
                later: ObservedProcessIdentity::Strong(later),
            }] if prior == &sampled_parent_identity && later == &later_parent_identity
        ));
        assert!(
            !full_process_observer
                .incarnation_cache
                .remembers_incarnation(&sampled_parent_identity)
        );
        assert!(
            !full_process_observer
                .incarnation_cache
                .remembers_incarnation(&later_parent_identity)
        );

        let requested_identities =
            BTreeSet::from([sampled_parent_identity.clone(), child_identity.clone()]);
        let mut targeted_process_observer = ProcessObserver::default();
        let targeted_snapshot = targeted_snapshot_from_sampling_evidence(
            &mut targeted_process_observer,
            requested_identities,
            synthetic_sampling_evidence(&[parent_pid, child_pid], identity_observations),
        );

        assert!(
            !targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&sampled_parent_identity)
        );
        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            targeted_snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if outcomes.get(&sampled_parent_identity)
                    == Some(&TargetedProcessObservation::Replaced {
                        replacement: later_parent_identity,
                    })
                    && outcomes.get(&child_identity)
                        == Some(&TargetedProcessObservation::Observed)
        ));
    }

    #[test]
    fn later_direct_identity_supersedes_an_earlier_reported_parent_identity() {
        let child_pid = sysinfo::Pid::from_u32(62);
        let parent_pid = sysinfo::Pid::from_u32(63);
        let earlier_parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 630);
        let later_parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 631);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 620);
        let identity_observations = vec![
            platform_observation_with_parent(&child_identity, &earlier_parent_identity),
            platform_observation(&later_parent_identity),
            platform_observation_with_parent(&child_identity, &earlier_parent_identity),
            platform_observation(&later_parent_identity),
        ];

        let mut full_process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut full_process_observer, &earlier_parent_identity);
        let full_snapshot = full_snapshot_from_sampling_evidence(
            &mut full_process_observer,
            synthetic_sampling_evidence(&[child_pid, parent_pid], identity_observations.clone()),
        );

        assert!(
            !full_snapshot
                .strongly_identified_processes()
                .contains_key(&earlier_parent_identity)
        );
        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&later_parent_identity)
        );
        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(full_snapshot.identity_binding_invalidations().is_empty());
        assert!(
            !full_process_observer
                .incarnation_cache
                .remembers_incarnation(&earlier_parent_identity)
        );
        assert!(
            full_process_observer
                .incarnation_cache
                .remembers_incarnation(&later_parent_identity)
        );

        let requested_identities =
            BTreeSet::from([later_parent_identity.clone(), child_identity.clone()]);
        let mut targeted_process_observer = ProcessObserver::default();
        let targeted_snapshot = targeted_snapshot_from_sampling_evidence(
            &mut targeted_process_observer,
            requested_identities,
            synthetic_sampling_evidence(&[child_pid, parent_pid], identity_observations),
        );

        assert!(
            !targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&earlier_parent_identity)
        );
        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&later_parent_identity)
        );
        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            targeted_snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if outcomes.get(&later_parent_identity)
                    == Some(&TargetedProcessObservation::Observed)
                    && outcomes.get(&child_identity)
                        == Some(&TargetedProcessObservation::Observed)
        ));
    }

    #[test]
    fn later_reported_parent_exit_excludes_an_earlier_strong_parent() {
        let parent_pid = sysinfo::Pid::from_u32(64);
        let child_pid = sysinfo::Pid::from_u32(65);
        let parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 640);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 650);
        let parent_exit = InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup {
            pid: parent_pid.as_u32(),
        };
        let identity_observations = vec![
            platform_observation(&parent_identity),
            platform_observation_with_insufficient_parent(&child_identity, &parent_exit),
            platform_observation(&parent_identity),
            platform_observation_with_insufficient_parent(&child_identity, &parent_exit),
        ];

        let mut full_process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut full_process_observer, &parent_identity);
        let full_snapshot = full_snapshot_from_sampling_evidence(
            &mut full_process_observer,
            synthetic_sampling_evidence(&[parent_pid, child_pid], identity_observations.clone()),
        );

        assert!(
            !full_snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            full_snapshot.identity_binding_invalidations(),
            [ProcessIdentityBindingInvalidation::LaterIdentityObservationChangedCurrentEvidence {
                prior: ObservedProcessIdentity::Strong(prior),
                later: ObservedProcessIdentity::Insufficient(later),
            }] if prior == &parent_identity && later == &parent_exit
        ));
        assert!(
            !full_process_observer
                .incarnation_cache
                .remembers_incarnation(&parent_identity)
        );

        let requested_identities =
            BTreeSet::from([parent_identity.clone(), child_identity.clone()]);
        let mut targeted_process_observer = ProcessObserver::default();
        let targeted_snapshot = targeted_snapshot_from_sampling_evidence(
            &mut targeted_process_observer,
            requested_identities,
            synthetic_sampling_evidence(&[parent_pid, child_pid], identity_observations),
        );

        assert!(
            !targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            targeted_snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if outcomes.get(&parent_identity) == Some(&TargetedProcessObservation::Gone)
                    && outcomes.get(&child_identity)
                        == Some(&TargetedProcessObservation::Observed)
        ));
    }

    #[test]
    fn later_reported_parent_unavailability_excludes_an_earlier_strong_parent() {
        let parent_pid = sysinfo::Pid::from_u32(66);
        let child_pid = sysinfo::Pid::from_u32(67);
        let parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 660);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 670);
        let parent_identity_unavailability =
            InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                pid: parent_pid.as_u32(),
            };
        let identity_observations = vec![
            platform_observation(&parent_identity),
            platform_observation_with_insufficient_parent(
                &child_identity,
                &parent_identity_unavailability,
            ),
            platform_observation(&parent_identity),
            platform_observation_with_insufficient_parent(
                &child_identity,
                &parent_identity_unavailability,
            ),
        ];

        let mut full_process_observer = ProcessObserver::default();
        prime_incarnation_cache(&mut full_process_observer, &parent_identity);
        let full_snapshot = full_snapshot_from_sampling_evidence(
            &mut full_process_observer,
            synthetic_sampling_evidence(&[parent_pid, child_pid], identity_observations.clone()),
        );

        assert!(
            !full_snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            full_snapshot.identity_binding_invalidations(),
            [ProcessIdentityBindingInvalidation::LaterIdentityObservationChangedCurrentEvidence {
                prior: ObservedProcessIdentity::Strong(prior),
                later: ObservedProcessIdentity::Insufficient(later),
            }] if prior == &parent_identity && later == &parent_identity_unavailability
        ));
        assert!(
            full_process_observer
                .incarnation_cache
                .remembers_incarnation(&parent_identity)
        );

        let requested_identities =
            BTreeSet::from([parent_identity.clone(), child_identity.clone()]);
        let mut targeted_process_observer = ProcessObserver::default();
        let targeted_snapshot = targeted_snapshot_from_sampling_evidence(
            &mut targeted_process_observer,
            requested_identities,
            synthetic_sampling_evidence(&[parent_pid, child_pid], identity_observations),
        );

        assert!(
            !targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            targeted_snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if outcomes.get(&parent_identity)
                    == Some(&TargetedProcessObservation::IdentityUnavailable(
                        parent_identity_unavailability,
                    ))
                    && outcomes.get(&child_identity)
                        == Some(&TargetedProcessObservation::Observed)
        ));
    }

    #[test]
    fn later_direct_strong_identity_supersedes_reported_parent_unavailability() {
        let child_pid = sysinfo::Pid::from_u32(68);
        let parent_pid = sysinfo::Pid::from_u32(69);
        let child_identity = ProcessIdentity::for_test(child_pid.as_u32(), 680);
        let parent_identity = ProcessIdentity::for_test(parent_pid.as_u32(), 690);
        let parent_identity_unavailability =
            InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                pid: parent_pid.as_u32(),
            };
        let identity_observations = vec![
            platform_observation_with_insufficient_parent(
                &child_identity,
                &parent_identity_unavailability,
            ),
            platform_observation(&parent_identity),
            platform_observation_with_insufficient_parent(
                &child_identity,
                &parent_identity_unavailability,
            ),
            platform_observation(&parent_identity),
        ];

        let mut full_process_observer = ProcessObserver::default();
        let full_snapshot = full_snapshot_from_sampling_evidence(
            &mut full_process_observer,
            synthetic_sampling_evidence(&[child_pid, parent_pid], identity_observations.clone()),
        );

        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(
            full_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(full_snapshot.identity_binding_invalidations().is_empty());
        assert!(
            full_process_observer
                .incarnation_cache
                .remembers_incarnation(&parent_identity)
        );

        let requested_identities =
            BTreeSet::from([parent_identity.clone(), child_identity.clone()]);
        let mut targeted_process_observer = ProcessObserver::default();
        let targeted_snapshot = targeted_snapshot_from_sampling_evidence(
            &mut targeted_process_observer,
            requested_identities,
            synthetic_sampling_evidence(&[child_pid, parent_pid], identity_observations),
        );

        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(
            targeted_snapshot
                .strongly_identified_processes()
                .contains_key(&child_identity)
        );
        assert!(matches!(
            targeted_snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if outcomes.get(&parent_identity)
                    == Some(&TargetedProcessObservation::Observed)
                    && outcomes.get(&child_identity)
                        == Some(&TargetedProcessObservation::Observed)
        ));
    }

    #[test]
    fn exact_target_refresh_samples_only_the_requested_current_process() -> std::io::Result<()> {
        let process_identity = strong_identity(std::process::id())?;
        let mut process_observer = ProcessObserver::default();
        let snapshot =
            process_observer.refresh(ProcessRefreshInput::TargetedIdentities(BTreeSet::from([
                process_identity.clone(),
            ])));

        assert_eq!(
            snapshot.scope(),
            &ProcessSnapshotScope::TargetedIdentities(BTreeSet::from([process_identity.clone()]))
        );
        assert_eq!(snapshot.strongly_identified_processes().len(), 1);
        assert!(matches!(
            snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if matches!(
                    outcomes.get(&process_identity),
                    Some(TargetedProcessObservation::Observed)
                )
        ));
        assert!(
            snapshot
                .strongly_identified_processes()
                .get(&process_identity)
                .is_some_and(|process_record| {
                    matches!(
                        process_record.incarnation_evidence(),
                        ProcessIncarnationEvidence::Strong { .. }
                    ) && matches!(
                        process_record.executable(),
                        ProcessFieldObservation::Observed(_)
                    ) && matches!(process_record.argv(), ProcessFieldObservation::Observed(_))
                        && matches!(process_record.cwd(), ProcessFieldObservation::Observed(_))
                })
        );
        Ok(())
    }

    #[test]
    fn exact_target_refresh_classifies_each_requested_identity_for_one_pid() -> std::io::Result<()>
    {
        let current_identity = strong_identity(std::process::id())?;
        let stale_identity = ProcessIdentity::for_test(current_identity.pid(), 0);
        let requested_identities =
            BTreeSet::from([stale_identity.clone(), current_identity.clone()]);
        let mut process_observer = ProcessObserver::default();

        let snapshot = process_observer.refresh(ProcessRefreshInput::TargetedIdentities(
            requested_identities,
        ));

        let TargetedProcessObservations::Outcomes(outcomes) =
            snapshot.targeted_process_observations()
        else {
            return Err(std::io::Error::other(
                "targeted refresh did not report requested identity outcomes",
            ));
        };
        assert_eq!(outcomes.len(), 2);
        assert_eq!(
            outcomes.get(&stale_identity),
            Some(&TargetedProcessObservation::Replaced {
                replacement: current_identity.clone(),
            })
        );
        assert_eq!(
            outcomes.get(&current_identity),
            Some(&TargetedProcessObservation::Observed)
        );
        assert_eq!(
            outcomes
                .values()
                .filter(|observation| {
                    matches!(observation, TargetedProcessObservation::Observed)
                })
                .count(),
            1
        );
        assert_eq!(snapshot.strongly_identified_processes().len(), 1);
        assert!(
            snapshot
                .strongly_identified_processes()
                .contains_key(&current_identity)
        );
        assert!(
            !snapshot
                .strongly_identified_processes()
                .contains_key(&stale_identity)
        );
        Ok(())
    }

    #[cfg(unix)]
    struct OwnedTestChild {
        child: std::process::Child,
    }

    #[cfg(unix)]
    impl OwnedTestChild {
        fn spawn() -> std::io::Result<Self> {
            std::process::Command::new("sleep")
                .arg("30")
                .spawn()
                .map(|child| Self { child })
        }

        fn pid(&self) -> u32 { self.child.id() }

        fn terminate(&mut self) -> std::io::Result<()> {
            self.child.kill()?;
            self.child.wait().map(|_| ())
        }
    }

    #[cfg(unix)]
    impl Drop for OwnedTestChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct SamePidExecTestChild {
        child:    std::process::Child,
        trigger:  std::process::ChildStdin,
        exec_cwd: tempfile::TempDir,
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl SamePidExecTestChild {
        fn spawn() -> std::io::Result<Self> {
            let exec_cwd = tempfile::tempdir()?;
            let mut child = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg("read trigger; cd \"$1\" && exec /bin/sleep 30")
                .arg("same-pid-exec")
                .arg(exec_cwd.path())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            let trigger = child.stdin.take().ok_or_else(|| {
                std::io::Error::other("same-PID exec child did not expose piped stdin")
            })?;
            Ok(Self {
                child,
                trigger,
                exec_cwd,
            })
        }

        fn pid(&self) -> u32 { self.child.id() }

        fn exec_cwd(&self) -> &Path { self.exec_cwd.path() }

        fn trigger_exec(&mut self) -> std::io::Result<()> {
            self.trigger.write_all(b"\n")?;
            self.trigger.flush()
        }

        fn wait_for_native_executable(&self, executable_name: &str) -> std::io::Result<()> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let process_info = processkit::process_info(self.pid())
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
                match process_info {
                    Some(process_info) if process_info.exe_name() == Some(executable_name) => {
                        return Ok(());
                    },
                    Some(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    },
                    Some(process_info) => {
                        return Err(std::io::Error::other(format!(
                            "same-PID exec did not expose {executable_name}; last executable was {:?}",
                            process_info.exe_name()
                        )));
                    },
                    None => {
                        return Err(std::io::Error::other(
                            "same-PID exec child exited before observation",
                        ));
                    },
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Drop for SamePidExecTestChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn refresh_target_until(
        process_observer: &mut ProcessObserver,
        process_identity: &ProcessIdentity,
        matches_record: impl Fn(&crate::process_observation::snapshot::ProcessSnapshotRecord) -> bool,
    ) -> std::io::Result<ProcessObservationSnapshot> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = process_observer.refresh(ProcessRefreshInput::TargetedIdentities(
                BTreeSet::from([process_identity.clone()]),
            ));
            if snapshot
                .strongly_identified_processes()
                .get(process_identity)
                .is_some_and(|process_record| matches_record(process_record))
            {
                return Ok(snapshot);
            }
            if Instant::now() >= deadline {
                return Err(std::io::Error::other(format!(
                    "observer did not produce the requested stable state: {snapshot:?}"
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn same_pid_exec_invalidates_then_recovers_observer_evidence() -> std::io::Result<()> {
        let mut child = SamePidExecTestChild::spawn()?;
        let process_identity = strong_identity(child.pid())?;
        let mut process_observer = ProcessObserver::default();

        let initial_snapshot =
            refresh_target_until(&mut process_observer, &process_identity, |process_record| {
                matches!(
                    process_record.incarnation_evidence(),
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::NewlyObserved,
                        ..
                    }
                )
            })?;
        assert!(
            initial_snapshot
                .strongly_identified_processes()
                .contains_key(&process_identity)
        );
        let stable_before_exec =
            refresh_target_until(&mut process_observer, &process_identity, |process_record| {
                matches!(
                    process_record.incarnation_evidence(),
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::Unchanged,
                        ..
                    }
                )
            })?;
        let ProcessFieldObservation::Observed(cwd_before_exec) =
            stable_before_exec.strongly_identified_processes()[&process_identity].cwd()
        else {
            return Err(std::io::Error::other(
                "stable pre-exec snapshot did not contain cwd",
            ));
        };
        let cwd_before_exec = cwd_before_exec.clone();

        child.trigger_exec()?;
        child.wait_for_native_executable("sleep")?;
        let transition_snapshot = refresh_target_until(
            &mut process_observer,
            &process_identity,
            |process_record| {
                matches!(
                    process_record.incarnation_evidence(),
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                        ..
                    }
                )
            },
        )?;
        assert_eq!(
            transition_snapshot.scope(),
            &ProcessSnapshotScope::TargetedIdentities(BTreeSet::from([process_identity.clone()]))
        );
        let transition_record =
            &transition_snapshot.strongly_identified_processes()[&process_identity];
        assert_eq!(transition_record.identity(), &process_identity);
        assert!(matches!(
            transition_record.executable(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            transition_record.argv(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            transition_record.cwd(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            transition_record.parentage_validation_outcome(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            transition_snapshot.validated_ancestry(&process_identity, ParentWalkDepth::new(1)),
            AncestryLookup::Observed(ancestry)
                if matches!(
                    ancestry.terminal(),
                    AncestryTerminal::ParentEvidenceUnavailable { .. }
                )
        ));
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_unclassified_candidate(&process_identity)
        );

        let recovered_snapshot =
            refresh_target_until(&mut process_observer, &process_identity, |process_record| {
                matches!(
                    process_record.incarnation_evidence(),
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::Unchanged,
                        ..
                    }
                )
            })?;
        let recovered_record =
            &recovered_snapshot.strongly_identified_processes()[&process_identity];
        assert!(matches!(
            recovered_record.executable(),
            ProcessFieldObservation::Observed(executable)
                if executable.file_name().is_some_and(|name| name == "sleep")
        ));
        assert!(matches!(
            recovered_record.argv(),
            ProcessFieldObservation::Observed(argv)
                if argv.iter().any(|argument| argument == "30")
        ));
        let ProcessFieldObservation::Observed(recovered_cwd) = recovered_record.cwd() else {
            return Err(std::io::Error::other(
                "stable post-exec snapshot did not contain cwd",
            ));
        };
        assert_ne!(recovered_cwd, &cwd_before_exec);
        assert_eq!(
            recovered_cwd.canonicalize()?,
            child.exec_cwd().canonicalize()?
        );
        assert!(matches!(
            recovered_record.parentage_validation_outcome(),
            ProcessFieldObservation::Observed(_)
        ));
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_live_child_forms_a_validated_parent_edge() -> std::io::Result<()> {
        let child = OwnedTestChild::spawn()?;
        let parent_identity = strong_identity(std::process::id())?;
        let child_identity = strong_identity(child.pid())?;
        let requested_identities =
            BTreeSet::from([parent_identity.clone(), child_identity.clone()]);
        let mut process_observer = ProcessObserver::default();

        let snapshot = process_observer.refresh(ProcessRefreshInput::TargetedIdentities(
            requested_identities,
        ));

        assert!(
            snapshot
                .strongly_identified_processes()
                .contains_key(&parent_identity)
        );
        assert!(matches!(
            snapshot.strongly_identified_processes()[&child_identity]
                .parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::ValidatedEdge(edge))
                if edge.parent() == &parent_identity && edge.child() == &child_identity
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn successful_full_snapshots_evict_an_exited_real_process() -> std::io::Result<()> {
        let mut child = OwnedTestChild::spawn()?;
        let process_identity = strong_identity(child.pid())?;
        let mut process_observer = ProcessObserver::default();

        let present_snapshot = process_observer.refresh(ProcessRefreshInput::FullSystemSnapshot);
        assert!(
            present_snapshot
                .strongly_identified_processes()
                .contains_key(&process_identity)
        );
        assert!(
            process_observer
                .incarnation_cache
                .remembers_incarnation(&process_identity)
        );

        child.terminate()?;
        let absent_snapshot = process_observer.refresh(ProcessRefreshInput::FullSystemSnapshot);

        assert!(
            !absent_snapshot
                .strongly_identified_processes()
                .contains_key(&process_identity)
        );
        assert!(
            !process_observer
                .incarnation_cache
                .remembers_incarnation(&process_identity)
        );
        Ok(())
    }
}
