//! Immutable host process observation without process-control operations.
//!
//! Nothing here signals, terminates, or otherwise controls a process. It
//! produces immutable evidence only, so a reader of host state can never
//! become a writer of it.

mod executor;
pub(crate) mod identity;
pub(crate) mod snapshot;

use std::collections::BTreeMap;
use std::collections::BTreeSet;

pub(crate) use executor::ProcessRefreshDeadline;
pub(crate) use executor::ProcessRefreshDispatchOutcome;
pub(crate) use executor::ProcessRefreshExecution;
pub(crate) use executor::ProcessRefreshExecutionBackendSelection;
pub(crate) use executor::ProcessRefreshExecutor;
pub(crate) use executor::ProcessRefreshResultPoll;
pub(crate) use executor::ProcessRefreshResultReceiver;
pub(crate) use executor::RunningTargetsRefreshSchedule;
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
pub(crate) use snapshot::ProcessRefreshExecutionOutcome;
use snapshot::ProcessRefreshObservations;
use snapshot::ProcessSamplingOutcome;
use snapshot::ReportedParent;
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
        #[cfg(unix)]
        use std::process::Child;

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
            child: Child,
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
            let pid = Pid::from_u32(u32::MAX);
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
struct ProcessObserver {
    running_metrics_system: RunningMetricsSystem,
    incarnation_cache:      ProcessIncarnationCache,
}

impl ProcessObserver {
    /// Execute one Running Targets cycle over the observer's private host state.
    fn refresh_running_targets_cycle(&mut self) -> ProcessObservationSnapshot {
        let FullSystemSnapshotCycle {
            process_observation_snapshot,
            running_metrics_refresh_targets,
        } = self.refresh_full_system_snapshot(
            process_field_refresh_kind(),
            &SysinfoProcessRefreshHostSource,
        );
        let running_process_metrics = self
            .running_metrics_system
            .observe_process_metrics_for_cycle(&running_metrics_refresh_targets.pids, |pid| {
                PlatformProcessObservation::observe_lifetime(pid)
                    .identity()
                    .clone()
            });
        process_observation_snapshot
            .bind_running_process_metrics(std::time::Instant::now(), running_process_metrics)
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
                            full_process_refresh_evidence,
                        },
                        pids.into(),
                    )
                },
            };
        let process_observation_snapshot = self
            .incarnation_cache
            .snapshot_from(std::time::Instant::now(), process_refresh_observations);
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::Instant;

    use sysinfo::Pid;
    use sysinfo::UpdateKind;

    use super::FullRefreshDirectlySampledPids;
    use super::PlatformProcessObservation;
    use super::ProcessObserver;
    use super::ProcessRefreshSamplingEvidence;
    use super::identity::InsufficientProcessIdentity;
    use super::identity::ObservedProcessIdentity;
    use super::identity::ProcessCreationOrderEvidence;
    use super::identity::ProcessIdentity;
    use super::snapshot::FullProcessRefreshEvidence;
    use super::snapshot::ProcessFieldLifetimeBinding;
    use super::snapshot::ProcessFieldObservation;
    use super::snapshot::ProcessFieldSample;
    use super::snapshot::ProcessFieldSourceObservation;
    use super::snapshot::ProcessRefreshObservations;
    use super::snapshot::ProcessSamplingOutcome;
    use super::snapshot::ReportedParent;

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
        pids: &[Pid],
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
            ProcessRefreshObservations {
                process_sampling_outcomes:     vec![process_sampling_outcome],
                full_process_refresh_evidence: FullProcessRefreshEvidence::NoProcessesUpdated,
            },
        );
    }

    fn apply_full_refresh_identity_observations(
        process_observer: &mut ProcessObserver,
        latest_identity_observations: BTreeMap<u32, ObservedProcessIdentity>,
    ) {
        process_observer.incarnation_cache.snapshot_from(
            Instant::now(),
            ProcessRefreshObservations {
                process_sampling_outcomes:     Vec::new(),
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
    fn each_running_cycle_executes_its_metrics_observation_once() {
        let mut process_observer = ProcessObserver::default();

        let running_snapshot = process_observer.refresh_running_targets_cycle();
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

        process_observer.refresh_running_targets_cycle();
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

        process_observer.refresh_running_targets_cycle();

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
            .with_exe(UpdateKind::Always)
            .with_cmd(UpdateKind::Always)
            .with_cwd(UpdateKind::Always);
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
}
