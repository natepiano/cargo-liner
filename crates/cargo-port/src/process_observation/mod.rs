//! Immutable host process observation without process-control operations.

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "the timing harness should fail on invalid fixture configuration"
)]
mod benchmarks;
mod executor;
pub(crate) mod identity;
pub(crate) mod snapshot;
#[cfg(test)]
pub(crate) mod snapshot_builder;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

pub(crate) use executor::CompileMonitorRefreshSchedule;
pub(crate) use executor::ProcessRefreshDeadline;
pub(crate) use executor::ProcessRefreshDispatchOutcome;
pub(crate) use executor::ProcessRefreshExecution;
pub(crate) use executor::ProcessRefreshExecutionBackendSelection;
pub(crate) use executor::ProcessRefreshExecutor;
pub(crate) use executor::ProcessRefreshResultPoll;
pub(crate) use executor::ProcessRefreshResultReceiver;
pub(crate) use executor::RefreshCycleClassifier;
pub(crate) use executor::RunningTargetsRefreshSchedule;
use identity::ObservedProcessIdentity;
use identity::PlatformProcessObservation;
use identity::ProcessIdentity;
pub(crate) use snapshot::BuildCandidateRole;
use snapshot::FullProcessRefreshEvidence;
use snapshot::ProcessFieldObservation;
use snapshot::ProcessFieldSample;
use snapshot::ProcessFieldSourceObservation;
use snapshot::ProcessFieldUnavailable;
use snapshot::ProcessIncarnationCache;
use snapshot::ProcessObservationSnapshot;
pub(crate) use snapshot::ProcessRefreshConsumerDemand;
pub(crate) use snapshot::ProcessRefreshExecutionOutcome;
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
    identity_before_sampling: PlatformProcessObservation,
    field_observation:        PidProcessFieldObservation,
    identity_after_sampling:  PlatformProcessObservation,
}

impl PidSamplingObservation {
    fn full_sampling_outcome(self) -> ProcessSamplingOutcome {
        let process_field_source_observation = match self.field_observation {
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
            self.identity_before_sampling,
            process_field_source_observation,
            self.identity_after_sampling,
        )
    }

    fn targeted_process_presence(&self) -> TargetedProcessPresence {
        match &self.field_observation {
            PidProcessFieldObservation::Sampled(process_field_source_observation) => {
                TargetedProcessPresence::Sampled(ProcessSamplingOutcome::bind_fields_to_identity(
                    self.identity_before_sampling.clone(),
                    process_field_source_observation.clone(),
                    self.identity_after_sampling.clone(),
                ))
            },
            PidProcessFieldObservation::Unavailable(process_field_unavailable) => {
                TargetedProcessPresence::FieldsUnavailable {
                    process_sampling_outcome:  ProcessSamplingOutcome::bind_fields_to_identity(
                        self.identity_before_sampling.clone(),
                        ProcessFieldSourceObservation::repeated_unavailable_fresh_system_samples(
                            process_field_unavailable.clone(),
                        ),
                        self.identity_after_sampling.clone(),
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

mod running_metrics_system {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    use sysinfo::Pid;

    use super::ObservedProcessIdentity;
    use super::ProcessIdentity;
    use super::snapshot::ProcessCpuPercent;
    use super::snapshot::RunningProcessMetricsRecord;

    #[derive(Debug, Eq, PartialEq)]
    enum IdentityBoundMetricsRecordObservation {
        Observed(RunningProcessMetricsRecord),
        RequestedPidAbsentFromRefreshedCache { pid: Pid },
    }

    mod raw_system {
        use std::collections::BTreeSet;

        use sysinfo::Pid;
        use sysinfo::ProcessRefreshKind;

        use super::IdentityBoundMetricsRecordObservation;
        use super::ProcessIdentity;
        use super::RunningMetricsCycleRefreshSet;

        mod process_table {
            use sysinfo::Pid;
            use sysinfo::ProcessRefreshKind;
            use sysinfo::ProcessesToUpdate;
            use sysinfo::System;

            use super::CachedRunningMetricsPids;
            use super::IdentityBoundMetricsRecordObservation;
            use super::ProcessIdentity;
            use super::RunningMetricsCycleRefreshSet;
            use crate::process_observation::snapshot::ProcessCpuPercent;
            use crate::process_observation::snapshot::RunningProcessMetricsRecord;

            /// Long-lived process table for Running Targets CPU and memory metrics.
            #[derive(Default)]
            pub(super) struct RunningMetricsProcessTable {
                system:                System,
                #[cfg(test)]
                raw_process_refreshes: u64,
                #[cfg(test)]
                record_replacements:   u64,
                #[cfg(test)]
                refresh_targets:       Vec<Vec<Pid>>,
            }

            impl RunningMetricsProcessTable {
                #[cfg(test)]
                pub(super) fn contains_process(&self, pid: Pid) -> bool {
                    self.system.process(pid).is_some()
                }

                pub(super) fn cached_pids(&self) -> CachedRunningMetricsPids {
                    CachedRunningMetricsPids {
                        pids: self.system.processes().keys().copied().collect(),
                    }
                }

                pub(super) fn metrics_record(
                    &self,
                    pid: Pid,
                    process_identity: &ProcessIdentity,
                ) -> IdentityBoundMetricsRecordObservation {
                    self.system.process(pid).map_or(
                        IdentityBoundMetricsRecordObservation::RequestedPidAbsentFromRefreshedCache {
                            pid,
                        },
                        |process| {
                            IdentityBoundMetricsRecordObservation::Observed(
                                RunningProcessMetricsRecord::new(
                                    process_identity.clone(),
                                    process.name().to_string_lossy().into_owned(),
                                    ProcessCpuPercent::from_sysinfo(process.cpu_usage()),
                                    process.memory(),
                                    process.start_time(),
                                ),
                            )
                        },
                    )
                }

                pub(super) fn replace_all_records(&mut self) {
                    self.system = System::new();
                    #[cfg(test)]
                    {
                        self.record_replacements += 1;
                    }
                }

                /// Performs and observes exactly one raw process-table refresh.
                pub(super) fn refresh_processes_specifics(
                    &mut self,
                    running_metrics_cycle_refresh_set: &RunningMetricsCycleRefreshSet,
                    process_refresh_kind: ProcessRefreshKind,
                ) {
                    #[cfg(test)]
                    {
                        self.raw_process_refreshes += 1;
                        self.refresh_targets
                            .push(running_metrics_cycle_refresh_set.pids().to_vec());
                    }
                    self.system.refresh_processes_specifics(
                        ProcessesToUpdate::Some(running_metrics_cycle_refresh_set.pids()),
                        true,
                        process_refresh_kind,
                    );
                }

                #[cfg(test)]
                pub(super) const fn raw_process_refresh_count(&self) -> u64 {
                    self.raw_process_refreshes
                }

                #[cfg(test)]
                pub(super) const fn record_replacement_count(&self) -> u64 {
                    self.record_replacements
                }

                #[cfg(test)]
                pub(super) fn refresh_targets(&self) -> &[Vec<Pid>] { &self.refresh_targets }
            }

            #[cfg(test)]
            mod tests {
                use sysinfo::ProcessRefreshKind;

                use super::RunningMetricsCycleRefreshSet;
                use super::RunningMetricsProcessTable;

                #[test]
                fn each_process_table_refresh_increments_actual_raw_refresh_count() {
                    let mut process_table = RunningMetricsProcessTable::default();
                    let refresh_set = RunningMetricsCycleRefreshSet { pids: Vec::new() };

                    process_table.refresh_processes_specifics(
                        &refresh_set,
                        ProcessRefreshKind::nothing().with_cpu().with_memory(),
                    );
                    process_table.refresh_processes_specifics(
                        &refresh_set,
                        ProcessRefreshKind::nothing().with_cpu().with_memory(),
                    );

                    assert_eq!(process_table.raw_process_refresh_count(), 2);
                }
            }
        }

        use process_table::RunningMetricsProcessTable;

        /// Raw process access for long-lived Running Targets metrics.
        #[derive(Default)]
        pub(super) struct RawRunningMetricsSystem {
            process_table: RunningMetricsProcessTable,
        }

        impl RawRunningMetricsSystem {
            #[cfg(test)]
            pub(super) fn contains_process(&self, pid: Pid) -> bool {
                self.process_table.contains_process(pid)
            }

            pub(super) fn cached_pids(&self) -> CachedRunningMetricsPids {
                self.process_table.cached_pids()
            }

            pub(super) fn metrics_record(
                &self,
                pid: Pid,
                process_identity: &ProcessIdentity,
            ) -> IdentityBoundMetricsRecordObservation {
                self.process_table.metrics_record(pid, process_identity)
            }

            pub(super) fn replace_all_records(&mut self) {
                self.process_table.replace_all_records();
            }

            /// The only raw process-refresh operation exposed by this boundary.
            pub(super) fn refresh_and_remove_exited_processes(
                &mut self,
                running_metrics_cycle_refresh_set: &RunningMetricsCycleRefreshSet,
            ) {
                self.process_table.refresh_processes_specifics(
                    running_metrics_cycle_refresh_set,
                    ProcessRefreshKind::nothing().with_cpu().with_memory(),
                );
            }

            #[cfg(test)]
            pub(super) const fn raw_process_refresh_count(&self) -> u64 {
                self.process_table.raw_process_refresh_count()
            }

            #[cfg(test)]
            pub(super) const fn record_replacement_count(&self) -> u64 {
                self.process_table.record_replacement_count()
            }

            #[cfg(test)]
            pub(super) fn refresh_targets(&self) -> &[Vec<Pid>] {
                self.process_table.refresh_targets()
            }
        }

        #[derive(Debug, Default, Eq, PartialEq)]
        pub(super) struct CachedRunningMetricsPids {
            pub(super) pids: BTreeSet<Pid>,
        }

        impl CachedRunningMetricsPids {
            pub(super) fn contains(&self, pid: Pid) -> bool { self.pids.contains(&pid) }

            pub(super) fn iter(&self) -> impl Iterator<Item = &Pid> { self.pids.iter() }
        }
    }

    use raw_system::CachedRunningMetricsPids;
    use raw_system::RawRunningMetricsSystem;

    /// Long-lived sysinfo records that preserve CPU baselines between Running
    /// Targets refreshes without exposing the raw `System`.
    #[derive(Default)]
    struct RunningProcessMetricsCache {
        raw_running_metrics_system: RawRunningMetricsSystem,
    }

    impl RunningProcessMetricsCache {
        #[cfg(test)]
        fn contains_process(&self, pid: Pid) -> bool {
            self.raw_running_metrics_system.contains_process(pid)
        }

        fn cached_pids(&self) -> CachedRunningMetricsPids {
            self.raw_running_metrics_system.cached_pids()
        }

        fn metrics_record(
            &self,
            pid: Pid,
            process_identity: &ProcessIdentity,
        ) -> IdentityBoundMetricsRecordObservation {
            self.raw_running_metrics_system
                .metrics_record(pid, process_identity)
        }

        fn replace_all_records(&mut self) { self.raw_running_metrics_system.replace_all_records(); }

        fn binding_authority(
            &self,
            identity_bindings: &RunningMetricsIdentityBindings,
        ) -> RunningMetricsCacheBindingAuthority {
            if self
                .cached_pids()
                .iter()
                .all(|pid| identity_bindings.by_pid.contains_key(pid))
            {
                RunningMetricsCacheBindingAuthority::EveryRecordStronglyBound
            } else {
                RunningMetricsCacheBindingAuthority::UnboundRecordsPresent
            }
        }

        fn refresh_and_remove_exited_processes(
            &mut self,
            running_metrics_cycle_refresh_set: &RunningMetricsCycleRefreshSet,
        ) {
            self.raw_running_metrics_system
                .refresh_and_remove_exited_processes(running_metrics_cycle_refresh_set);
        }

        #[cfg(test)]
        const fn raw_process_refresh_count(&self) -> u64 {
            self.raw_running_metrics_system.raw_process_refresh_count()
        }

        #[cfg(test)]
        const fn record_replacement_count(&self) -> u64 {
            self.raw_running_metrics_system.record_replacement_count()
        }

        #[cfg(test)]
        fn refresh_targets(&self) -> &[Vec<Pid>] {
            self.raw_running_metrics_system.refresh_targets()
        }
    }

    /// Strong identities proven stable across the refresh that populated
    /// `RunningMetricsSystem::metrics_cache`.
    #[derive(Debug, Default, Eq, PartialEq)]
    struct RunningMetricsIdentityBindings {
        by_pid: BTreeMap<Pid, ProcessIdentity>,
    }

    impl RunningMetricsIdentityBindings {
        fn retain_only_safe_before_refresh(
            &mut self,
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
        ) {
            self.by_pid.retain(
                |pid, bound_identity| match identities_before_refresh.get(pid) {
                    Some(ObservedProcessIdentity::Strong(identity_before_refresh)) => {
                        bound_identity == identity_before_refresh
                    },
                    Some(ObservedProcessIdentity::Insufficient(_)) => false,
                    None => true,
                },
            );
        }

        fn retain_only_identities_stable_across_refresh(
            &mut self,
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
            identities_after_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
        ) {
            let previously_bound_absent_identities: BTreeMap<Pid, ProcessIdentity> = self
                .by_pid
                .iter()
                .filter(|(pid, _)| !identities_before_refresh.contains_key(pid))
                .map(|(pid, process_identity)| (*pid, process_identity.clone()))
                .collect();
            self.by_pid = identities_before_refresh
                .iter()
                .filter_map(|(pid, observed_identity_before_refresh)| {
                    match (
                        observed_identity_before_refresh,
                        identities_after_refresh.get(pid),
                    ) {
                        (
                            ObservedProcessIdentity::Strong(identity_before_refresh),
                            Some(ObservedProcessIdentity::Strong(identity_after_refresh)),
                        ) if identity_before_refresh == identity_after_refresh => {
                            Some((*pid, identity_after_refresh.clone()))
                        },
                        _ => None,
                    }
                })
                .chain(previously_bound_absent_identities)
                .collect();
        }

        fn retain_only_cached_processes(
            &mut self,
            cached_running_metrics_pids: &CachedRunningMetricsPids,
        ) {
            self.by_pid
                .retain(|pid, _| cached_running_metrics_pids.contains(*pid));
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RunningMetricsCachePreparation {
        PreserveBaselines,
        PurgeUnsafeRecords,
    }

    impl RunningMetricsCachePreparation {
        fn classify(
            cached_running_metrics_pids: &CachedRunningMetricsPids,
            identity_bindings: &RunningMetricsIdentityBindings,
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
        ) -> Self {
            if cached_running_metrics_pids.iter().any(|pid| {
                identity_bindings
                    .by_pid
                    .get(pid)
                    .is_none_or(|bound_identity| {
                        identities_before_refresh
                            .get(pid)
                            .is_some_and(|identity_before_refresh| match identity_before_refresh {
                                ObservedProcessIdentity::Strong(identity_before_refresh) => {
                                    bound_identity != identity_before_refresh
                                },
                                ObservedProcessIdentity::Insufficient(_) => true,
                            })
                    })
            }) {
                Self::PurgeUnsafeRecords
            } else {
                Self::PreserveBaselines
            }
        }
    }

    #[derive(Debug, Default, Eq, PartialEq)]
    struct IdentityBoundCpuSamples {
        by_identity: BTreeMap<ProcessIdentity, ProcessCpuPercent>,
    }

    impl IdentityBoundCpuSamples {
        fn retain_only_stable_identities(
            &self,
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
            identity_bindings: &RunningMetricsIdentityBindings,
        ) -> Self {
            Self {
                by_identity: identities_before_refresh
                    .iter()
                    .filter_map(|(pid, observed_process_identity)| {
                        let ObservedProcessIdentity::Strong(process_identity) =
                            observed_process_identity
                        else {
                            return None;
                        };
                        if identity_bindings.by_pid.get(pid) != Some(process_identity) {
                            return None;
                        }
                        self.by_identity
                            .get(process_identity)
                            .map(|cpu_percent| (process_identity.clone(), *cpu_percent))
                    })
                    .collect(),
            }
        }

        fn from_cycle_output(
            running_process_metrics: &BTreeMap<ProcessIdentity, RunningProcessMetricsRecord>,
        ) -> Self {
            Self {
                by_identity: running_process_metrics
                    .iter()
                    .map(|(process_identity, running_process_metrics_record)| {
                        (
                            process_identity.clone(),
                            running_process_metrics_record.cpu_percent(),
                        )
                    })
                    .collect(),
            }
        }

        fn continuity_sample(
            &self,
            process_identity: &ProcessIdentity,
            refreshed_cpu_percent: ProcessCpuPercent,
        ) -> ProcessCpuPercent {
            self.by_identity
                .get(process_identity)
                .copied()
                .unwrap_or(refreshed_cpu_percent)
        }
    }

    /// CPU history availability relative to a rebuilt raw metrics `System`.
    #[derive(Debug, Eq, PartialEq)]
    enum RunningMetricsCpuContinuity {
        Established(IdentityBoundCpuSamples),
        RebuiltAwaitingBaseline(IdentityBoundCpuSamples),
        RebuiltBaselineReady(IdentityBoundCpuSamples),
    }

    impl Default for RunningMetricsCpuContinuity {
        fn default() -> Self { Self::Established(IdentityBoundCpuSamples::default()) }
    }

    impl RunningMetricsCpuContinuity {
        const fn samples(&self) -> &IdentityBoundCpuSamples {
            match self {
                Self::Established(identity_bound_cpu_samples)
                | Self::RebuiltAwaitingBaseline(identity_bound_cpu_samples)
                | Self::RebuiltBaselineReady(identity_bound_cpu_samples) => {
                    identity_bound_cpu_samples
                },
            }
        }

        fn begin_rebuild_before_refresh(
            &mut self,
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
            identity_bindings: &RunningMetricsIdentityBindings,
        ) {
            *self = Self::RebuiltAwaitingBaseline(
                self.samples()
                    .retain_only_stable_identities(identities_before_refresh, identity_bindings),
            );
        }

        fn begin_rebuild_after_refresh(
            &mut self,
            running_process_metrics: &BTreeMap<ProcessIdentity, RunningProcessMetricsRecord>,
        ) {
            *self = Self::RebuiltAwaitingBaseline(IdentityBoundCpuSamples::from_cycle_output(
                running_process_metrics,
            ));
        }

        fn record_raw_refresh(&mut self) {
            let prior = std::mem::take(self);
            *self = match prior {
                Self::Established(identity_bound_cpu_samples)
                | Self::RebuiltBaselineReady(identity_bound_cpu_samples) => {
                    Self::Established(identity_bound_cpu_samples)
                },
                Self::RebuiltAwaitingBaseline(identity_bound_cpu_samples) => {
                    Self::RebuiltBaselineReady(identity_bound_cpu_samples)
                },
            };
        }

        fn cpu_sample(
            &self,
            process_identity: &ProcessIdentity,
            refreshed_cpu_percent: ProcessCpuPercent,
        ) -> ProcessCpuPercent {
            match self {
                Self::RebuiltBaselineReady(identity_bound_cpu_samples) => {
                    identity_bound_cpu_samples
                        .continuity_sample(process_identity, refreshed_cpu_percent)
                },
                Self::Established(_) | Self::RebuiltAwaitingBaseline(_) => refreshed_cpu_percent,
            }
        }

        fn record_cycle_output(
            &mut self,
            running_process_metrics: &BTreeMap<ProcessIdentity, RunningProcessMetricsRecord>,
        ) {
            let identity_bound_cpu_samples =
                IdentityBoundCpuSamples::from_cycle_output(running_process_metrics);
            *self = match self {
                Self::Established(_) => Self::Established(identity_bound_cpu_samples),
                Self::RebuiltBaselineReady(_) => {
                    Self::RebuiltBaselineReady(identity_bound_cpu_samples)
                },
                Self::RebuiltAwaitingBaseline(_) => {
                    Self::RebuiltAwaitingBaseline(identity_bound_cpu_samples)
                },
            };
        }

        #[cfg(test)]
        fn replace_history_for_test(
            &mut self,
            by_identity: BTreeMap<ProcessIdentity, ProcessCpuPercent>,
        ) {
            *self = Self::Established(IdentityBoundCpuSamples { by_identity });
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RunningMetricsCacheBindingAuthority {
        EveryRecordStronglyBound,
        UnboundRecordsPresent,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct RunningMetricsCycleRefreshSet {
        pids: Vec<Pid>,
    }

    impl RunningMetricsCycleRefreshSet {
        fn for_cycle(
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
            prior_identity_bindings: &RunningMetricsIdentityBindings,
        ) -> Self {
            Self {
                pids: identities_before_refresh
                    .iter()
                    .filter_map(|(pid, observed_process_identity)| {
                        matches!(
                            observed_process_identity,
                            ObservedProcessIdentity::Strong(_)
                        )
                        .then_some(*pid)
                    })
                    .chain(
                        prior_identity_bindings
                            .by_pid
                            .keys()
                            .filter(|pid| !identities_before_refresh.contains_key(pid))
                            .copied(),
                    )
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            }
        }

        fn pids(&self) -> &[Pid] { &self.pids }
    }

    #[derive(Default)]
    pub(super) struct RunningMetricsSystem {
        metrics_cache:     RunningProcessMetricsCache,
        identity_bindings: RunningMetricsIdentityBindings,
        cpu_continuity:    RunningMetricsCpuContinuity,
    }

    impl RunningMetricsSystem {
        pub(super) fn observe_process_metrics_for_cycle(
            &mut self,
            pids: &[Pid],
            mut observe_identity: impl FnMut(u32) -> ObservedProcessIdentity,
        ) -> BTreeMap<ProcessIdentity, RunningProcessMetricsRecord> {
            let identities_before_refresh: BTreeMap<Pid, ObservedProcessIdentity> = pids
                .iter()
                .map(|pid| (*pid, observe_identity(pid.as_u32())))
                .collect();
            self.purge_unsafe_records_before_refresh(&identities_before_refresh);
            self.identity_bindings
                .retain_only_safe_before_refresh(&identities_before_refresh);
            let running_metrics_cycle_refresh_set = RunningMetricsCycleRefreshSet::for_cycle(
                &identities_before_refresh,
                &self.identity_bindings,
            );
            self.metrics_cache
                .refresh_and_remove_exited_processes(&running_metrics_cycle_refresh_set);
            self.cpu_continuity.record_raw_refresh();
            let identities_after_refresh: BTreeMap<Pid, ObservedProcessIdentity> = pids
                .iter()
                .map(|pid| (*pid, observe_identity(pid.as_u32())))
                .collect();
            self.identity_bindings
                .retain_only_identities_stable_across_refresh(
                    &identities_before_refresh,
                    &identities_after_refresh,
                );
            self.identity_bindings
                .retain_only_cached_processes(&self.metrics_cache.cached_pids());

            let running_process_metrics: BTreeMap<ProcessIdentity, RunningProcessMetricsRecord> =
                pids.iter()
                .copied()
                .filter_map(|pid| {
                    let ObservedProcessIdentity::Strong(identity_before_refresh) =
                        &identities_before_refresh[&pid]
                    else {
                        return None;
                    };
                    let ObservedProcessIdentity::Strong(identity_after_refresh) =
                        &identities_after_refresh[&pid]
                    else {
                        return None;
                    };
                    if identity_before_refresh != identity_after_refresh {
                        return None;
                    }
                    match self.metrics_cache.metrics_record(pid, identity_after_refresh) {
                        IdentityBoundMetricsRecordObservation::Observed(
                            mut running_process_metrics_record,
                        ) => Some((
                            identity_after_refresh.clone(),
                            {
                                let cpu_percent = self.cpu_continuity.cpu_sample(
                                    identity_after_refresh,
                                    running_process_metrics_record.cpu_percent(),
                                );
                                running_process_metrics_record
                                    .replace_cpu_percent_for_continuity(cpu_percent);
                                running_process_metrics_record
                            },
                        )),
                        IdentityBoundMetricsRecordObservation::RequestedPidAbsentFromRefreshedCache {
                            ..
                        } => None,
                    }
                })
                .collect();
            self.cpu_continuity
                .record_cycle_output(&running_process_metrics);
            if self
                .metrics_cache
                .binding_authority(&self.identity_bindings)
                == RunningMetricsCacheBindingAuthority::UnboundRecordsPresent
            {
                self.cpu_continuity
                    .begin_rebuild_after_refresh(&running_process_metrics);
                self.metrics_cache.replace_all_records();
            }
            running_process_metrics
        }

        fn purge_unsafe_records_before_refresh(
            &mut self,
            identities_before_refresh: &BTreeMap<Pid, ObservedProcessIdentity>,
        ) {
            match RunningMetricsCachePreparation::classify(
                &self.metrics_cache.cached_pids(),
                &self.identity_bindings,
                identities_before_refresh,
            ) {
                RunningMetricsCachePreparation::PreserveBaselines => {},
                RunningMetricsCachePreparation::PurgeUnsafeRecords => {
                    self.cpu_continuity.begin_rebuild_before_refresh(
                        identities_before_refresh,
                        &self.identity_bindings,
                    );
                    self.metrics_cache.replace_all_records();
                },
            }
        }

        #[cfg(test)]
        pub(super) const fn raw_process_refresh_count(&self) -> u64 {
            self.metrics_cache.raw_process_refresh_count()
        }

        #[cfg(test)]
        fn replace_cpu_history_for_test(
            &mut self,
            by_identity: BTreeMap<ProcessIdentity, ProcessCpuPercent>,
        ) {
            self.cpu_continuity.replace_history_for_test(by_identity);
        }
    }

    #[cfg(test)]
    mod tests {
        use std::collections::BTreeMap;
        use std::collections::BTreeSet;

        use sysinfo::Pid;

        use super::CachedRunningMetricsPids;
        use super::IdentityBoundMetricsRecordObservation;
        use super::RunningMetricsCachePreparation;
        use super::RunningMetricsCycleRefreshSet;
        use super::RunningMetricsIdentityBindings;
        use super::RunningMetricsSystem;
        use super::RunningProcessMetricsCache;
        use crate::process_observation::identity::InsufficientProcessIdentity;
        use crate::process_observation::identity::ObservedProcessIdentity;
        use crate::process_observation::identity::ProcessIdentity;
        use crate::process_observation::snapshot::ProcessCpuPercent;

        #[cfg(unix)]
        struct OwnedMetricsTestChild {
            child: std::process::Child,
        }

        #[cfg(unix)]
        impl OwnedMetricsTestChild {
            fn spawn() -> std::io::Result<Self> {
                std::process::Command::new("sleep")
                    .arg("30")
                    .spawn()
                    .map(|child| Self { child })
            }

            fn pid(&self) -> Pid { Pid::from_u32(self.child.id()) }
        }

        #[cfg(unix)]
        impl Drop for OwnedMetricsTestChild {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }

        #[test]
        fn metrics_record_observes_record_bound_to_requested_identity() {
            let pid = Pid::from_u32(std::process::id());
            let process_identity = ProcessIdentity::for_test(pid.as_u32(), 510);
            let mut running_process_metrics_cache = RunningProcessMetricsCache::default();
            running_process_metrics_cache.refresh_and_remove_exited_processes(
                &RunningMetricsCycleRefreshSet { pids: vec![pid] },
            );

            let observation = running_process_metrics_cache.metrics_record(pid, &process_identity);

            assert!(matches!(
                observation,
                IdentityBoundMetricsRecordObservation::Observed(
                    running_process_metrics_record
                ) if running_process_metrics_record.identity() == &process_identity
            ));
        }

        #[test]
        fn metrics_record_names_requested_pid_absent_from_refreshed_cache() {
            let pid = Pid::from_u32(52);
            let process_identity = ProcessIdentity::for_test(pid.as_u32(), 520);
            let mut running_process_metrics_cache = RunningProcessMetricsCache::default();
            running_process_metrics_cache.refresh_and_remove_exited_processes(
                &RunningMetricsCycleRefreshSet { pids: Vec::new() },
            );

            assert_eq!(
                running_process_metrics_cache.metrics_record(pid, &process_identity),
                IdentityBoundMetricsRecordObservation::RequestedPidAbsentFromRefreshedCache { pid }
            );
        }

        #[test]
        fn absent_metrics_cache_record_is_omitted_from_cycle_output() {
            let pid = Pid::from_u32(53);
            let process_identity = ProcessIdentity::for_test(pid.as_u32(), 530);
            let observed_process_identity = ObservedProcessIdentity::Strong(process_identity);
            let mut running_metrics_system = RunningMetricsSystem::default();

            let running_process_metrics = running_metrics_system
                .observe_process_metrics_for_cycle(&[pid], |_| observed_process_identity.clone());

            assert!(running_process_metrics.is_empty());
            assert!(running_metrics_system.identity_bindings.by_pid.is_empty());
            assert_eq!(running_metrics_system.raw_process_refresh_count(), 1);
        }

        #[test]
        fn same_pid_identity_replacement_invalidates_metrics_cache_before_refresh() {
            let pid = Pid::from_u32(61);
            let prior_identity = ProcessIdentity::for_test(pid.as_u32(), 700);
            let replacement_identity = ProcessIdentity::for_test(pid.as_u32(), 701);
            let replacement_identity_observations =
                BTreeMap::from([(pid, ObservedProcessIdentity::Strong(replacement_identity))]);
            let cached_running_metrics_pids = CachedRunningMetricsPids {
                pids: BTreeSet::from([pid]),
            };
            let identity_bindings = RunningMetricsIdentityBindings {
                by_pid: BTreeMap::from([(pid, prior_identity)]),
            };

            assert_eq!(
                RunningMetricsCachePreparation::classify(
                    &cached_running_metrics_pids,
                    &identity_bindings,
                    &replacement_identity_observations,
                ),
                RunningMetricsCachePreparation::PurgeUnsafeRecords
            );
        }

        #[test]
        fn unbound_cached_record_requires_purge() {
            let pid = Pid::from_u32(62);
            let cached_running_metrics_pids = CachedRunningMetricsPids {
                pids: BTreeSet::from([pid]),
            };

            assert_eq!(
                RunningMetricsCachePreparation::classify(
                    &cached_running_metrics_pids,
                    &RunningMetricsIdentityBindings::default(),
                    &BTreeMap::new(),
                ),
                RunningMetricsCachePreparation::PurgeUnsafeRecords
            );
        }

        #[test]
        fn metrics_bindings_retain_only_stable_strong_identities() {
            let stable_pid = Pid::from_u32(71);
            let exited_pid = Pid::from_u32(72);
            let changed_pid = Pid::from_u32(73);
            let insufficient_before_pid = Pid::from_u32(74);
            let stable_identity = ProcessIdentity::for_test(stable_pid.as_u32(), 710);
            let exited_identity = ProcessIdentity::for_test(exited_pid.as_u32(), 720);
            let changed_identity = ProcessIdentity::for_test(changed_pid.as_u32(), 730);
            let replacement_identity = ProcessIdentity::for_test(changed_pid.as_u32(), 731);
            let late_identity = ProcessIdentity::for_test(insufficient_before_pid.as_u32(), 740);
            let identities_before_refresh = BTreeMap::from([
                (
                    stable_pid,
                    ObservedProcessIdentity::Strong(stable_identity.clone()),
                ),
                (exited_pid, ObservedProcessIdentity::Strong(exited_identity)),
                (
                    changed_pid,
                    ObservedProcessIdentity::Strong(changed_identity),
                ),
                (
                    insufficient_before_pid,
                    ObservedProcessIdentity::Insufficient(
                        InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                            pid: insufficient_before_pid.as_u32(),
                        },
                    ),
                ),
            ]);
            let identities_after_refresh = BTreeMap::from([
                (
                    stable_pid,
                    ObservedProcessIdentity::Strong(stable_identity.clone()),
                ),
                (
                    exited_pid,
                    ObservedProcessIdentity::Insufficient(
                        InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup {
                            pid: exited_pid.as_u32(),
                        },
                    ),
                ),
                (
                    changed_pid,
                    ObservedProcessIdentity::Strong(replacement_identity),
                ),
                (
                    insufficient_before_pid,
                    ObservedProcessIdentity::Strong(late_identity),
                ),
            ]);
            let mut running_metrics_identity_bindings = RunningMetricsIdentityBindings::default();

            running_metrics_identity_bindings.retain_only_identities_stable_across_refresh(
                &identities_before_refresh,
                &identities_after_refresh,
            );

            assert_eq!(
                running_metrics_identity_bindings,
                RunningMetricsIdentityBindings {
                    by_pid: BTreeMap::from([(stable_pid, stable_identity)]),
                }
            );
        }

        #[test]
        fn unrelated_pid_cannot_enter_metrics_cycle_refresh_set() {
            let discovered_pid = Pid::from_u32(81);
            let previously_bound_pid = Pid::from_u32(82);
            let unrelated_pid = Pid::from_u32(83);
            let identities_before_refresh = BTreeMap::from([(
                discovered_pid,
                ObservedProcessIdentity::Strong(ProcessIdentity::for_test(
                    discovered_pid.as_u32(),
                    810,
                )),
            )]);
            let prior_identity_bindings = RunningMetricsIdentityBindings {
                by_pid: BTreeMap::from([(
                    previously_bound_pid,
                    ProcessIdentity::for_test(previously_bound_pid.as_u32(), 820),
                )]),
            };

            let running_metrics_cycle_refresh_set = RunningMetricsCycleRefreshSet::for_cycle(
                &identities_before_refresh,
                &prior_identity_bindings,
            );

            assert_eq!(
                running_metrics_cycle_refresh_set.pids(),
                &[discovered_pid, previously_bound_pid]
            );
            assert!(
                !running_metrics_cycle_refresh_set
                    .pids()
                    .contains(&unrelated_pid)
            );
        }

        #[test]
        fn previously_bound_pid_is_refreshed_when_current_discovery_omits_it() {
            let previously_bound_pid = Pid::from_u32(92);
            let prior_identity_bindings = RunningMetricsIdentityBindings {
                by_pid: BTreeMap::from([(
                    previously_bound_pid,
                    ProcessIdentity::for_test(previously_bound_pid.as_u32(), 920),
                )]),
            };

            let running_metrics_cycle_refresh_set = RunningMetricsCycleRefreshSet::for_cycle(
                &BTreeMap::new(),
                &prior_identity_bindings,
            );

            assert!(
                running_metrics_cycle_refresh_set
                    .pids()
                    .contains(&previously_bound_pid)
            );
        }

        #[test]
        fn strong_current_pids_enter_metrics_cycle_refresh_set_once() {
            let discovered_pid = Pid::from_u32(101);
            let identities_before_refresh = BTreeMap::from([(
                discovered_pid,
                ObservedProcessIdentity::Strong(ProcessIdentity::for_test(
                    discovered_pid.as_u32(),
                    1_010,
                )),
            )]);
            let prior_identity_bindings = RunningMetricsIdentityBindings {
                by_pid: BTreeMap::from([(
                    discovered_pid,
                    ProcessIdentity::for_test(discovered_pid.as_u32(), 1_010),
                )]),
            };

            let running_metrics_cycle_refresh_set = RunningMetricsCycleRefreshSet::for_cycle(
                &identities_before_refresh,
                &prior_identity_bindings,
            );

            assert_eq!(running_metrics_cycle_refresh_set.pids(), &[discovered_pid]);
        }

        #[test]
        fn insufficient_current_identity_does_not_enter_metrics_refresh_set() {
            let pid = Pid::from_u32(111);
            let identities_before_refresh = BTreeMap::from([(
                pid,
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid: pid.as_u32() },
                ),
            )]);

            let running_metrics_cycle_refresh_set = RunningMetricsCycleRefreshSet::for_cycle(
                &identities_before_refresh,
                &RunningMetricsIdentityBindings::default(),
            );

            assert!(running_metrics_cycle_refresh_set.pids().is_empty());
        }

        #[test]
        fn repeated_insufficient_identity_does_not_cache_pid_or_replace_baselines() {
            let pid = Pid::from_u32(std::process::id());
            let insufficient_identity = ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformIdentityLookupFailed { pid: pid.as_u32() },
            );
            let mut running_metrics_system = RunningMetricsSystem::default();

            for _ in 0..2 {
                let running_process_metrics = running_metrics_system
                    .observe_process_metrics_for_cycle(&[pid], |_| insufficient_identity.clone());
                assert!(running_process_metrics.is_empty());
            }

            assert!(!running_metrics_system.metrics_cache.contains_process(pid));
            assert_eq!(
                running_metrics_system
                    .metrics_cache
                    .record_replacement_count(),
                0
            );
            assert_eq!(
                running_metrics_system.metrics_cache.refresh_targets(),
                &[Vec::new(), Vec::new()]
            );
        }

        #[test]
        fn unrelated_process_churn_preserves_stable_binding_and_cpu_baseline() {
            let stable_pid = Pid::from_u32(121);
            let churn_pid = Pid::from_u32(122);
            let stable_identity = ProcessIdentity::for_test(stable_pid.as_u32(), 1_210);
            let churn_identity = ProcessIdentity::for_test(churn_pid.as_u32(), 1_220);
            let cached_running_metrics_pids = CachedRunningMetricsPids {
                pids: BTreeSet::from([stable_pid]),
            };
            let identities_before_refresh = BTreeMap::from([
                (
                    stable_pid,
                    ObservedProcessIdentity::Strong(stable_identity.clone()),
                ),
                (churn_pid, ObservedProcessIdentity::Strong(churn_identity)),
            ]);
            let identities_after_refresh = identities_before_refresh.clone();
            let mut identity_bindings = RunningMetricsIdentityBindings {
                by_pid: BTreeMap::from([(stable_pid, stable_identity.clone())]),
            };

            assert_eq!(
                RunningMetricsCachePreparation::classify(
                    &cached_running_metrics_pids,
                    &identity_bindings,
                    &identities_before_refresh,
                ),
                RunningMetricsCachePreparation::PreserveBaselines
            );
            identity_bindings.retain_only_identities_stable_across_refresh(
                &identities_before_refresh,
                &identities_after_refresh,
            );
            identity_bindings.retain_only_cached_processes(&cached_running_metrics_pids);
            assert_eq!(
                identity_bindings.by_pid,
                BTreeMap::from([(stable_pid, stable_identity)])
            );
        }

        #[cfg(unix)]
        #[test]
        fn replaced_pid_is_fresh_and_stable_pid_keeps_cpu_history() -> std::io::Result<()> {
            let replaced_process = OwnedMetricsTestChild::spawn()?;
            let stable_pid = Pid::from_u32(std::process::id());
            let replaced_pid = replaced_process.pid();
            let stable_identity = ProcessIdentity::for_test(stable_pid.as_u32(), 1_310);
            let prior_identity = ProcessIdentity::for_test(replaced_pid.as_u32(), 1_320);
            let replacement_identity = ProcessIdentity::for_test(replaced_pid.as_u32(), 1_321);
            let mut running_metrics_system = RunningMetricsSystem::default();

            let first_cycle = running_metrics_system.observe_process_metrics_for_cycle(
                &[stable_pid, replaced_pid],
                |pid| {
                    if pid == stable_pid.as_u32() {
                        ObservedProcessIdentity::Strong(stable_identity.clone())
                    } else {
                        ObservedProcessIdentity::Strong(prior_identity.clone())
                    }
                },
            );
            assert!(first_cycle.contains_key(&stable_identity));
            assert!(first_cycle.contains_key(&prior_identity));

            let stable_history_sample = ProcessCpuPercent::from_sysinfo(37.5);
            let prior_history_sample = ProcessCpuPercent::from_sysinfo(91.0);
            running_metrics_system.replace_cpu_history_for_test(BTreeMap::from([
                (stable_identity.clone(), stable_history_sample),
                (prior_identity.clone(), prior_history_sample),
            ]));
            let raw_refreshes_before_replacement =
                running_metrics_system.raw_process_refresh_count();

            let replacement_cycle = running_metrics_system.observe_process_metrics_for_cycle(
                &[stable_pid, replaced_pid],
                |pid| {
                    if pid == stable_pid.as_u32() {
                        ObservedProcessIdentity::Strong(stable_identity.clone())
                    } else {
                        ObservedProcessIdentity::Strong(replacement_identity.clone())
                    }
                },
            );

            assert_eq!(
                running_metrics_system.raw_process_refresh_count(),
                raw_refreshes_before_replacement + 1
            );
            assert_eq!(
                running_metrics_system
                    .metrics_cache
                    .record_replacement_count(),
                1
            );
            assert_eq!(
                running_metrics_system
                    .metrics_cache
                    .binding_authority(&running_metrics_system.identity_bindings),
                super::RunningMetricsCacheBindingAuthority::EveryRecordStronglyBound
            );
            assert!(!replacement_cycle.contains_key(&prior_identity));
            assert_eq!(
                replacement_cycle[&stable_identity].cpu_percent(),
                stable_history_sample
            );
            let replacement_record = &replacement_cycle[&replacement_identity];
            let IdentityBoundMetricsRecordObservation::Observed(raw_replacement_record) =
                running_metrics_system
                    .metrics_cache
                    .metrics_record(replaced_pid, &replacement_identity)
            else {
                return Err(std::io::Error::other(
                    "replacement PID should have a refreshed raw metrics record",
                ));
            };
            assert_eq!(
                replacement_record.cpu_percent(),
                raw_replacement_record.cpu_percent()
            );
            assert_eq!(replacement_record.name(), raw_replacement_record.name());
            assert_eq!(
                replacement_record.start_time(),
                raw_replacement_record.start_time()
            );
            assert!(
                !running_metrics_system
                    .cpu_continuity
                    .samples()
                    .by_identity
                    .contains_key(&prior_identity)
            );
            Ok(())
        }

        #[cfg(unix)]
        #[test]
        fn insufficient_pid_is_purged_without_stable_cpu_regression() -> std::io::Result<()> {
            let insufficient_process = OwnedMetricsTestChild::spawn()?;
            let stable_pid = Pid::from_u32(std::process::id());
            let insufficient_pid = insufficient_process.pid();
            let stable_identity = ProcessIdentity::for_test(stable_pid.as_u32(), 1_410);
            let prior_identity = ProcessIdentity::for_test(insufficient_pid.as_u32(), 1_420);
            let mut running_metrics_system = RunningMetricsSystem::default();

            let _ = running_metrics_system.observe_process_metrics_for_cycle(
                &[stable_pid, insufficient_pid],
                |pid| {
                    if pid == stable_pid.as_u32() {
                        ObservedProcessIdentity::Strong(stable_identity.clone())
                    } else {
                        ObservedProcessIdentity::Strong(prior_identity.clone())
                    }
                },
            );
            let stable_history_sample = ProcessCpuPercent::from_sysinfo(42.5);
            running_metrics_system.replace_cpu_history_for_test(BTreeMap::from([(
                stable_identity.clone(),
                stable_history_sample,
            )]));
            let raw_refreshes_before_insufficient_identity =
                running_metrics_system.raw_process_refresh_count();
            let insufficient_identity = ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                    pid: insufficient_pid.as_u32(),
                },
            );

            let insufficient_cycle = running_metrics_system.observe_process_metrics_for_cycle(
                &[stable_pid, insufficient_pid],
                |pid| {
                    if pid == stable_pid.as_u32() {
                        ObservedProcessIdentity::Strong(stable_identity.clone())
                    } else {
                        insufficient_identity.clone()
                    }
                },
            );

            assert_eq!(
                running_metrics_system.raw_process_refresh_count(),
                raw_refreshes_before_insufficient_identity + 1
            );
            assert_eq!(
                running_metrics_system
                    .metrics_cache
                    .record_replacement_count(),
                1
            );
            assert_eq!(
                running_metrics_system
                    .metrics_cache
                    .refresh_targets()
                    .last(),
                Some(&vec![stable_pid])
            );
            assert!(
                !running_metrics_system
                    .metrics_cache
                    .contains_process(insufficient_pid)
            );
            assert!(
                !running_metrics_system
                    .identity_bindings
                    .by_pid
                    .contains_key(&insufficient_pid)
            );
            assert_eq!(
                running_metrics_system
                    .metrics_cache
                    .binding_authority(&running_metrics_system.identity_bindings),
                super::RunningMetricsCacheBindingAuthority::EveryRecordStronglyBound
            );
            assert_eq!(insufficient_cycle.len(), 1);
            assert_eq!(
                insufficient_cycle[&stable_identity].cpu_percent(),
                stable_history_sample
            );
            Ok(())
        }
    }
}

use running_metrics_system::RunningMetricsSystem;

enum FullProcessDiscoveryOutcome {
    NoProcessesUpdated,
    Updated(Vec<Pid>),
}

trait ProcessRefreshHostSource {
    fn full_process_discovery(&self) -> FullProcessDiscoveryOutcome;

    fn process_identity_observation(&self, pid: u32) -> PlatformProcessObservation;

    fn repeated_process_field_observations(
        &self,
        pids: &[Pid],
        refresh_kind: ProcessRefreshKind,
    ) -> BTreeMap<Pid, ProcessFieldSourceObservation>;
}

struct SysinfoProcessRefreshHostSource;

impl ProcessRefreshHostSource for SysinfoProcessRefreshHostSource {
    fn full_process_discovery(&self) -> FullProcessDiscoveryOutcome {
        let mut process_discovery_system = System::new();
        let updated_processes = process_discovery_system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        if updated_processes == 0 {
            FullProcessDiscoveryOutcome::NoProcessesUpdated
        } else {
            FullProcessDiscoveryOutcome::Updated(
                process_discovery_system
                    .processes()
                    .keys()
                    .copied()
                    .collect(),
            )
        }
    }

    fn process_identity_observation(&self, pid: u32) -> PlatformProcessObservation {
        PlatformProcessObservation::observe(pid)
    }

    fn repeated_process_field_observations(
        &self,
        pids: &[Pid],
        refresh_kind: ProcessRefreshKind,
    ) -> BTreeMap<Pid, ProcessFieldSourceObservation> {
        ProcessObserver::refresh_process_field_sources(pids, refresh_kind)
    }
}

struct RunningMetricsRefreshTargets {
    pids: Vec<Pid>,
}

impl From<Vec<Pid>> for RunningMetricsRefreshTargets {
    fn from(pids: Vec<Pid>) -> Self { Self { pids } }
}

struct FullSystemSnapshotCycle {
    process_observation_snapshot:    ProcessObservationSnapshot,
    running_metrics_refresh_targets: RunningMetricsRefreshTargets,
}

/// Host-only process observation with one private long-lived metrics `System`.
#[derive(Default)]
pub(crate) struct ProcessObserver {
    running_metrics_system: RunningMetricsSystem,
    incarnation_cache:      ProcessIncarnationCache,
}

impl ProcessObserver {
    /// Execute one coalesced consumer cycle over the observer's private host state.
    fn refresh_for_consumer_demand(
        &mut self,
        process_refresh_consumer_demand: ProcessRefreshConsumerDemand,
    ) -> ProcessObservationSnapshot {
        let process_refresh_host_source = SysinfoProcessRefreshHostSource;
        let process_refresh_input = match process_refresh_consumer_demand {
            ProcessRefreshConsumerDemand::CompileMonitor => {
                ProcessRefreshInput::TargetedIdentities(
                    self.incarnation_cache.cached_process_identities(),
                )
            },
            ProcessRefreshConsumerDemand::RunningTargets
            | ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor => {
                ProcessRefreshInput::FullSystemSnapshot
            },
        };
        match process_refresh_input {
            ProcessRefreshInput::FullSystemSnapshot => {
                let FullSystemSnapshotCycle {
                    process_observation_snapshot,
                    running_metrics_refresh_targets,
                } = self.refresh_full_system_snapshot(
                    process_field_refresh_kind(),
                    &process_refresh_host_source,
                );
                let running_process_metrics = self
                    .running_metrics_system
                    .observe_process_metrics_for_cycle(
                        &running_metrics_refresh_targets.pids,
                        |pid| {
                            PlatformProcessObservation::observe_lifetime(pid)
                                .identity()
                                .clone()
                        },
                    );
                process_observation_snapshot.bind_running_process_metrics(
                    std::time::Instant::now(),
                    running_process_metrics,
                )
            },
            ProcessRefreshInput::TargetedIdentities(_) => {
                self.refresh_with_host_source(&process_refresh_input, &process_refresh_host_source)
            },
        }
    }

    /// Refresh a full or selected process set and return only immutable evidence.
    #[cfg(test)]
    pub(crate) fn refresh(
        &mut self,
        process_refresh_input: &ProcessRefreshInput,
    ) -> ProcessObservationSnapshot {
        self.refresh_with_host_source(process_refresh_input, &SysinfoProcessRefreshHostSource)
    }

    fn refresh_with_host_source(
        &mut self,
        process_refresh_input: &ProcessRefreshInput,
        process_refresh_host_source: &impl ProcessRefreshHostSource,
    ) -> ProcessObservationSnapshot {
        let refresh_kind = process_field_refresh_kind();
        match process_refresh_input {
            ProcessRefreshInput::FullSystemSnapshot => {
                self.refresh_full_system_snapshot(refresh_kind, process_refresh_host_source)
                    .process_observation_snapshot
            },
            ProcessRefreshInput::TargetedIdentities(process_identities) => {
                let process_refresh_observations = Self::refresh_targeted_observations(
                    process_identities,
                    refresh_kind,
                    process_refresh_host_source,
                );
                let scope = snapshot_scope(process_refresh_input);
                self.incarnation_cache.snapshot_from(
                    std::time::Instant::now(),
                    scope,
                    process_refresh_observations,
                )
            },
        }
    }

    fn refresh_full_system_snapshot(
        &mut self,
        refresh_kind: ProcessRefreshKind,
        process_refresh_host_source: &impl ProcessRefreshHostSource,
    ) -> FullSystemSnapshotCycle {
        let cached_process_identities = self.incarnation_cache.cached_process_identities();
        let (process_refresh_observations, running_metrics_refresh_targets) =
            match process_refresh_host_source.full_process_discovery() {
                FullProcessDiscoveryOutcome::NoProcessesUpdated => (
                    ProcessRefreshObservations {
                        process_sampling_outcomes:     Vec::new(),
                        targeted_process_observations: TargetedProcessObservations::NotRequested,
                        full_process_refresh_evidence:
                            FullProcessRefreshEvidence::NoProcessesUpdated,
                    },
                    Vec::new().into(),
                ),
                FullProcessDiscoveryOutcome::Updated(pids) => {
                    let process_refresh_sampling_evidence = Self::observe_pids_with(
                        &pids,
                        |pid| process_refresh_host_source.process_identity_observation(pid),
                        |pids| {
                            process_refresh_host_source
                                .repeated_process_field_observations(pids, refresh_kind)
                        },
                    );
                    let directly_sampled_pids =
                        FullRefreshDirectlySampledPids::from(&process_refresh_sampling_evidence);
                    let post_sampling_identities =
                        process_refresh_sampling_evidence.latest_post_sampling_identities();
                    let latest_identity_observations =
                        Self::finalize_full_refresh_identity_observations(
                            &cached_process_identities,
                            &directly_sampled_pids,
                            post_sampling_identities,
                            |pid| {
                                process_refresh_host_source
                                    .process_identity_observation(pid)
                                    .lifetime
                                    .identity()
                                    .clone()
                            },
                        );
                    let process_sampling_outcomes = process_refresh_sampling_evidence
                        .into_reconciled_sampling_outcomes(&latest_identity_observations);
                    let full_process_refresh_evidence =
                        FullProcessRefreshEvidence::UpdatedProcesses {
                            latest_identity_observations,
                        };
                    (
                        ProcessRefreshObservations {
                            process_sampling_outcomes,
                            targeted_process_observations:
                                TargetedProcessObservations::NotRequested,
                            full_process_refresh_evidence,
                        },
                        pids.into(),
                    )
                },
            };
        let process_observation_snapshot = self.incarnation_cache.snapshot_from(
            std::time::Instant::now(),
            ProcessSnapshotScope::FullSystem,
            process_refresh_observations,
        );
        FullSystemSnapshotCycle {
            process_observation_snapshot,
            running_metrics_refresh_targets,
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
        process_identities: &BTreeSet<ProcessIdentity>,
        refresh_kind: ProcessRefreshKind,
        process_refresh_host_source: &impl ProcessRefreshHostSource,
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
        let process_refresh_sampling_evidence = Self::observe_pids_with(
            &pids,
            |pid| process_refresh_host_source.process_identity_observation(pid),
            |pids| {
                process_refresh_host_source.repeated_process_field_observations(pids, refresh_kind)
            },
        );
        let post_sampling_identities =
            process_refresh_sampling_evidence.latest_post_sampling_identities();

        Self::targeted_process_refresh_observations(
            requested_identities_by_pid,
            &process_refresh_sampling_evidence,
            &post_sampling_identities,
        )
    }

    fn targeted_process_refresh_observations(
        requested_identities_by_pid: BTreeMap<u32, Vec<ProcessIdentity>>,
        process_refresh_sampling_evidence: &ProcessRefreshSamplingEvidence,
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
            let identity_before_sampling = observe_identity(pid.as_u32());
            Self::record_identity_observation_events(
                IdentityObservationSamplingPhase::BeforeFields,
                &identity_before_sampling,
                &mut identity_timeline,
            );
            identity_observations_before_fields.insert(*pid, identity_before_sampling);
        }
        let process_field_sources = observe_fields(pids);
        let mut pid_observations = BTreeMap::new();
        for pid in pids {
            let field_observation = process_field_sources.get(pid).map_or_else(
                || {
                    PidProcessFieldObservation::Unavailable(
                        ProcessFieldUnavailable::PlatformLookupFailed,
                    )
                },
                |process_field_source_observation| {
                    PidProcessFieldObservation::Sampled(process_field_source_observation.clone())
                },
            );
            let identity_after_sampling = observe_identity(pid.as_u32());
            Self::record_identity_observation_events(
                IdentityObservationSamplingPhase::AfterFields,
                &identity_after_sampling,
                &mut identity_timeline,
            );
            pid_observations.insert(
                *pid,
                PidSamplingObservation {
                    identity_before_sampling: identity_observations_before_fields[pid].clone(),
                    field_observation,
                    identity_after_sampling,
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

    /// Temporary `System` instances provide coherent process-field sampling on every
    /// platform. `running_metrics_system` is the sole long-lived CPU and memory state;
    /// it is refreshed once per due Running Targets cycle.
    fn refresh_process_field_sources(
        pids: &[Pid],
        refresh_kind: ProcessRefreshKind,
    ) -> BTreeMap<Pid, ProcessFieldSourceObservation> {
        let mut initial_field_system = System::new();
        initial_field_system.refresh_processes_specifics(
            ProcessesToUpdate::Some(pids),
            true,
            refresh_kind,
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

fn process_field_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_cwd(UpdateKind::Always)
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
    use super::ProcessRefreshConsumerDemand;
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
        identity_observations: &[PlatformProcessObservation],
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
        process_refresh_sampling_evidence: &ProcessRefreshSamplingEvidence,
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
    fn running_metrics_execute_once_only_for_cycles_that_request_them() {
        let mut process_observer = ProcessObserver::default();

        let compile_snapshot = process_observer
            .refresh_for_consumer_demand(ProcessRefreshConsumerDemand::CompileMonitor);
        assert!(matches!(
            compile_snapshot.running_process_metrics(),
            crate::process_observation::snapshot::RunningProcessMetricsObservation::NotRequested
        ));
        assert_eq!(
            process_observer
                .running_metrics_system
                .raw_process_refresh_count(),
            0
        );

        let running_snapshot = process_observer
            .refresh_for_consumer_demand(ProcessRefreshConsumerDemand::RunningTargets);
        assert!(matches!(
            running_snapshot.running_process_metrics(),
            crate::process_observation::snapshot::RunningProcessMetricsObservation::Observed(_)
        ));
        assert_eq!(
            process_observer
                .running_metrics_system
                .raw_process_refresh_count(),
            1
        );

        process_observer.refresh_for_consumer_demand(
            ProcessRefreshConsumerDemand::RunningTargetsAndCompileMonitor,
        );
        assert_eq!(
            process_observer
                .running_metrics_system
                .raw_process_refresh_count(),
            2
        );
    }

    #[test]
    fn due_running_cycle_performs_one_actual_raw_cpu_and_memory_refresh() {
        let mut process_observer = ProcessObserver::default();

        process_observer.refresh_for_consumer_demand(ProcessRefreshConsumerDemand::RunningTargets);

        assert_eq!(
            process_observer
                .running_metrics_system
                .raw_process_refresh_count(),
            1
        );
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
            &[
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
            &[
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
            identity_before_sampling: platform_observation(&old_identity),
            field_observation:        PidProcessFieldObservation::Sampled(cargo_field_source()),
            identity_after_sampling:  PlatformProcessObservation::for_test(
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
                .identity_after_sampling
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
            identity_before_sampling: platform_observation(&old_identity),
            field_observation:        PidProcessFieldObservation::Sampled(cargo_field_source()),
            identity_after_sampling:  platform_observation(&replacement_identity),
        };
        let post_sampling_identities = BTreeMap::from([(
            pid.as_u32(),
            pid_sampling_observation
                .identity_after_sampling
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
            synthetic_sampling_evidence(&[parent_pid, child_pid], &identity_observations),
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
            &synthetic_sampling_evidence(&[parent_pid, child_pid], &identity_observations),
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
            synthetic_sampling_evidence(&[child_pid, parent_pid], &identity_observations),
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
            &synthetic_sampling_evidence(&[child_pid, parent_pid], &identity_observations),
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
            synthetic_sampling_evidence(&[parent_pid, child_pid], &identity_observations),
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
            &synthetic_sampling_evidence(&[parent_pid, child_pid], &identity_observations),
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
            synthetic_sampling_evidence(&[parent_pid, child_pid], &identity_observations),
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
            &synthetic_sampling_evidence(&[parent_pid, child_pid], &identity_observations),
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
            synthetic_sampling_evidence(&[child_pid, parent_pid], &identity_observations),
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
            &synthetic_sampling_evidence(&[child_pid, parent_pid], &identity_observations),
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
            process_observer.refresh(&ProcessRefreshInput::TargetedIdentities(BTreeSet::from([
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

        let snapshot = process_observer.refresh(&ProcessRefreshInput::TargetedIdentities(
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
            let snapshot = process_observer.refresh(&ProcessRefreshInput::TargetedIdentities(
                BTreeSet::from([process_identity.clone()]),
            ));
            if snapshot
                .strongly_identified_processes()
                .get(process_identity)
                .is_some_and(&matches_record)
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
    #[derive(Clone, Copy)]
    enum ObserverIncarnationMilestone {
        NewlyObserved,
        Unchanged,
        ExecutableOrArgumentsChanged,
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl ObserverIncarnationMilestone {
        fn matches(self, incarnation_evidence: &ProcessIncarnationEvidence) -> bool {
            match self {
                Self::NewlyObserved => matches!(
                    incarnation_evidence,
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::NewlyObserved,
                        ..
                    }
                ),
                Self::Unchanged => matches!(
                    incarnation_evidence,
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::Unchanged,
                        ..
                    }
                ),
                Self::ExecutableOrArgumentsChanged => matches!(
                    incarnation_evidence,
                    ProcessIncarnationEvidence::Strong {
                        incarnation_state: ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                        ..
                    }
                ),
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn refresh_until_incarnation_milestone(
        process_observer: &mut ProcessObserver,
        process_identity: &ProcessIdentity,
        milestone: ObserverIncarnationMilestone,
    ) -> std::io::Result<ProcessObservationSnapshot> {
        refresh_target_until(process_observer, process_identity, |process_record| {
            milestone.matches(process_record.incarnation_evidence())
        })
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn observed_process_cwd(
        process_observation_snapshot: &ProcessObservationSnapshot,
        process_identity: &ProcessIdentity,
    ) -> std::io::Result<PathBuf> {
        match process_observation_snapshot.strongly_identified_processes()[process_identity].cwd() {
            ProcessFieldObservation::Observed(cwd) => Ok(cwd.clone()),
            _ => Err(std::io::Error::other(
                "stable snapshot did not contain an observed cwd",
            )),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn same_pid_exec_invalidates_then_recovers_observer_evidence() -> std::io::Result<()> {
        let mut child = SamePidExecTestChild::spawn()?;
        let process_identity = strong_identity(child.pid())?;
        let mut process_observer = ProcessObserver::default();

        let initial_snapshot = refresh_until_incarnation_milestone(
            &mut process_observer,
            &process_identity,
            ObserverIncarnationMilestone::NewlyObserved,
        )?;
        assert!(
            initial_snapshot
                .strongly_identified_processes()
                .contains_key(&process_identity)
        );
        let stable_before_exec = refresh_until_incarnation_milestone(
            &mut process_observer,
            &process_identity,
            ObserverIncarnationMilestone::Unchanged,
        )?;
        let cwd_before_exec = observed_process_cwd(&stable_before_exec, &process_identity)?;

        child.trigger_exec()?;
        child.wait_for_native_executable("sleep")?;
        let transition_snapshot = refresh_until_incarnation_milestone(
            &mut process_observer,
            &process_identity,
            ObserverIncarnationMilestone::ExecutableOrArgumentsChanged,
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

        let recovered_snapshot = refresh_until_incarnation_milestone(
            &mut process_observer,
            &process_identity,
            ObserverIncarnationMilestone::Unchanged,
        )?;
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
        let recovered_cwd = observed_process_cwd(&recovered_snapshot, &process_identity)?;
        assert_ne!(recovered_cwd, cwd_before_exec);
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

        let snapshot = process_observer.refresh(&ProcessRefreshInput::TargetedIdentities(
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

        let present_snapshot = process_observer.refresh(&ProcessRefreshInput::FullSystemSnapshot);
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
        let absent_snapshot = process_observer.refresh(&ProcessRefreshInput::FullSystemSnapshot);

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
