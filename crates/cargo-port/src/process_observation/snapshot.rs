use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use super::identity::InsufficientProcessIdentity;
use super::identity::ObservedProcessIdentity;
use super::identity::ParentCreationOrder;
use super::identity::PlatformProcessObservation;
use super::identity::ProcessCreationOrderEvidence;
use super::identity::ProcessCreationOrderUnavailable;
use super::identity::ProcessFingerprint;
use super::identity::ProcessIdentity;
use super::identity::ProcessIncarnation;

/// The observed, unavailable, or invalidated state of one process field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessFieldObservation<T> {
    Observed(T),
    Unavailable(ProcessFieldUnavailable),
    Invalidated(ProcessFieldInvalidation),
}

/// Why a process field could not be observed for the current process lifetime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessFieldUnavailable {
    PlatformDidNotReport,
    PlatformLookupFailed,
    ProcessExited,
}

/// Why sampled or retained field evidence cannot be used in the current snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessFieldInvalidation {
    ExecIncarnationEvidenceInsufficient,
    ExecutableOrArgumentsChanged,
    ProcessFieldsDifferedDuringSampling,
    ProcessFieldStabilityUnproven,
    ProcessIdentityNotStableDuringSampling,
    ParentIdentityChangedDuringSampling,
}

/// A direct parent report before its identity is validated against the snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReportedParent {
    Root,
    Identified(ProcessIdentity),
    IdentityUnavailable(InsufficientProcessIdentity),
}

/// The validation outcome for one process's direct parent relation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ParentageValidationOutcome {
    Root {
        child: ProcessIdentity,
    },
    ValidatedEdge(ValidatedParentEdge),
    UnavailableParent {
        child:           ProcessIdentity,
        parent_identity: InsufficientProcessIdentity,
    },
    UnavailableIdentifiedParent {
        edge: StrongParentEdge,
    },
    CreationOrderUnavailable {
        edge:        StrongParentEdge,
        unavailable: ProcessCreationOrderUnavailable,
    },
    RejectedEdge {
        edge:      StrongParentEdge,
        rejection: ParentEdgeRejection,
    },
}

/// Strong endpoint identities for a reported direct-parent relation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StrongParentEdge {
    parent: ProcessIdentity,
    child:  ProcessIdentity,
}

impl StrongParentEdge {
    pub(crate) const fn parent(&self) -> &ProcessIdentity { &self.parent }

    #[cfg(test)]
    pub(crate) const fn child(&self) -> &ProcessIdentity { &self.child }

    #[cfg(test)]
    pub(crate) const fn for_test(parent: ProcessIdentity, child: ProcessIdentity) -> Self {
        Self { parent, child }
    }
}

/// One direct parent relation accepted after endpoint and lifetime validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedParentEdge {
    endpoints: StrongParentEdge,
}

impl ValidatedParentEdge {
    pub(crate) const fn parent(&self) -> &ProcessIdentity { self.endpoints.parent() }

    #[cfg(test)]
    pub(crate) const fn child(&self) -> &ProcessIdentity { self.endpoints.child() }

    #[cfg(test)]
    pub(crate) const fn for_test(parent: ProcessIdentity, child: ProcessIdentity) -> Self {
        Self {
            endpoints: StrongParentEdge::for_test(parent, child),
        }
    }

    const fn endpoints(&self) -> &StrongParentEdge { &self.endpoints }
}

/// Why a direct or walked parent relation cannot be accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ParentEdgeRejection {
    SelfParent,
    CreatedAfterChild,
    IdentityReplaced { current: ProcessIdentity },
    Cycle,
}

/// The current incarnation's relationship to cached executable evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessIncarnationState {
    NewlyObserved,
    Unchanged,
    ExecutableOrArgumentsChanged { previous: ProcessIncarnation },
}

/// Whether executable and argument evidence identifies an exec incarnation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessIncarnationEvidence {
    Strong {
        incarnation:       ProcessIncarnation,
        incarnation_state: ProcessIncarnationState,
    },
    Insufficient(InsufficientProcessIncarnationEvidence),
}

/// Executable and argument states that cannot identify an exec incarnation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InsufficientProcessIncarnationEvidence {
    executable: ProcessFieldObservation<PathBuf>,
    argv:       ProcessFieldObservation<Vec<OsString>>,
}

#[cfg(test)]
impl InsufficientProcessIncarnationEvidence {
    pub(crate) const fn executable(&self) -> &ProcessFieldObservation<PathBuf> { &self.executable }

    pub(crate) const fn argv(&self) -> &ProcessFieldObservation<Vec<OsString>> { &self.argv }
}

/// One immutable record for a strongly identified process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessSnapshotRecord {
    identity:                     ProcessIdentity,
    incarnation_evidence:         ProcessIncarnationEvidence,
    executable:                   ProcessFieldObservation<PathBuf>,
    argv:                         ProcessFieldObservation<Vec<OsString>>,
    cwd:                          ProcessFieldObservation<PathBuf>,
    parentage_validation_outcome: ProcessFieldObservation<ParentageValidationOutcome>,
}

impl ProcessSnapshotRecord {
    pub(crate) const fn identity(&self) -> &ProcessIdentity { &self.identity }

    pub(crate) const fn incarnation_evidence(&self) -> &ProcessIncarnationEvidence {
        &self.incarnation_evidence
    }

    pub(crate) const fn executable(&self) -> &ProcessFieldObservation<PathBuf> { &self.executable }

    pub(crate) const fn argv(&self) -> &ProcessFieldObservation<Vec<OsString>> { &self.argv }

    pub(crate) const fn cwd(&self) -> &ProcessFieldObservation<PathBuf> { &self.cwd }

    pub(crate) const fn parentage_validation_outcome(
        &self,
    ) -> &ProcessFieldObservation<ParentageValidationOutcome> {
        &self.parentage_validation_outcome
    }
}

/// A diagnostic-only process record without a strong identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InsufficientIdentityProcessRecord {
    identity:   InsufficientProcessIdentity,
    executable: ProcessFieldObservation<PathBuf>,
    argv:       ProcessFieldObservation<Vec<OsString>>,
    cwd:        ProcessFieldObservation<PathBuf>,
    parent:     ProcessFieldObservation<ReportedParent>,
}

/// The exact process set an observer refreshes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessRefreshInput {
    FullSystemSnapshot,
    TargetedIdentities(BTreeSet<ProcessIdentity>),
}

/// The scope that produced an immutable snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessSnapshotScope {
    FullSystem,
    TargetedIdentities(BTreeSet<ProcessIdentity>),
}

/// The process consumers whose due work is served by one observer cycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessRefreshConsumerDemand {
    RunningTargets,
    CompileMonitor,
    RunningTargetsAndCompileMonitor,
}

impl ProcessRefreshConsumerDemand {
    pub(crate) const fn includes_running_targets(self) -> bool {
        matches!(
            self,
            Self::RunningTargets | Self::RunningTargetsAndCompileMonitor
        )
    }

    pub(crate) const fn coalesce(self, other: Self) -> Self {
        if matches!((self, other), (Self::RunningTargets, Self::RunningTargets)) {
            Self::RunningTargets
        } else if matches!((self, other), (Self::CompileMonitor, Self::CompileMonitor)) {
            Self::CompileMonitor
        } else {
            Self::RunningTargetsAndCompileMonitor
        }
    }
}

/// Why a requested observer execution could not produce a snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessRefreshExecutionFailure {
    RequestChannelDisconnected,
    ResultChannelDisconnected,
}

/// The immutable result of one requested observer execution.
#[derive(Debug, PartialEq)]
pub(crate) enum ProcessRefreshExecutionOutcome {
    Completed(CompletedProcessRefreshExecution),
    Failed(ProcessRefreshExecutionFailure),
}

/// Snapshot and elapsed observer time from one successfully completed refresh.
#[derive(Debug, PartialEq)]
pub(crate) struct CompletedProcessRefreshExecution {
    process_observation_snapshot: ProcessObservationSnapshot,
    elapsed:                      Duration,
}

impl CompletedProcessRefreshExecution {
    pub(super) const fn new(
        process_observation_snapshot: ProcessObservationSnapshot,
        elapsed: Duration,
    ) -> Self {
        Self {
            process_observation_snapshot,
            elapsed,
        }
    }

    pub(crate) const fn elapsed(&self) -> Duration { self.elapsed }

    #[cfg(test)]
    pub(super) const fn snapshot(&self) -> &ProcessObservationSnapshot {
        &self.process_observation_snapshot
    }

    pub(crate) fn into_snapshot(self) -> ProcessObservationSnapshot {
        self.process_observation_snapshot
    }
}

/// CPU usage encoded as IEEE-754 bits so process snapshots retain exact
/// equality for tests and correlated worker results.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessCpuPercent(u32);

impl ProcessCpuPercent {
    pub(super) const fn from_sysinfo(cpu_percent: f32) -> Self { Self(cpu_percent.to_bits()) }

    pub(crate) const fn get(self) -> f32 { f32::from_bits(self.0) }
}

/// Name and resource metrics proven to belong to one strong process identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunningProcessMetricsRecord {
    identity:     ProcessIdentity,
    name:         String,
    cpu_percent:  ProcessCpuPercent,
    memory_bytes: u64,
    start_time:   u64,
}

impl RunningProcessMetricsRecord {
    pub(super) const fn new(
        identity: ProcessIdentity,
        name: String,
        cpu_percent: ProcessCpuPercent,
        memory_bytes: u64,
        start_time: u64,
    ) -> Self {
        Self {
            identity,
            name,
            cpu_percent,
            memory_bytes,
            start_time,
        }
    }

    pub(crate) const fn identity(&self) -> &ProcessIdentity { &self.identity }

    pub(crate) fn name(&self) -> &str { &self.name }

    pub(crate) const fn cpu_percent(&self) -> ProcessCpuPercent { self.cpu_percent }

    pub(super) const fn replace_cpu_percent_for_continuity(
        &mut self,
        cpu_percent: ProcessCpuPercent,
    ) {
        self.cpu_percent = cpu_percent;
    }

    pub(crate) const fn memory_bytes(&self) -> u64 { self.memory_bytes }

    pub(crate) const fn start_time(&self) -> u64 { self.start_time }
}

/// Whether one refresh requested Running Targets resource metrics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RunningProcessMetricsObservation {
    NotRequested,
    Observed(BTreeMap<ProcessIdentity, RunningProcessMetricsRecord>),
}

/// An immutable process observation result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessObservationSnapshot {
    observed_at:                     Instant,
    scope:                           ProcessSnapshotScope,
    strongly_identified_processes:   BTreeMap<ProcessIdentity, ProcessSnapshotRecord>,
    insufficient_identity_processes: Vec<InsufficientIdentityProcessRecord>,
    identity_binding_invalidations:  Vec<ProcessIdentityBindingInvalidation>,
    targeted_process_observations:   TargetedProcessObservations,
    running_process_metrics:         RunningProcessMetricsObservation,
}

impl ProcessObservationSnapshot {
    #[cfg(test)]
    pub(super) fn empty_for_test() -> Self {
        Self {
            observed_at:                     Instant::now(),
            scope:                           ProcessSnapshotScope::FullSystem,
            strongly_identified_processes:   BTreeMap::new(),
            insufficient_identity_processes: Vec::new(),
            identity_binding_invalidations:  Vec::new(),
            targeted_process_observations:   TargetedProcessObservations::NotRequested,
            running_process_metrics:         RunningProcessMetricsObservation::Observed(
                BTreeMap::new(),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) const fn scope(&self) -> &ProcessSnapshotScope { &self.scope }

    pub(crate) const fn strongly_identified_processes(
        &self,
    ) -> &BTreeMap<ProcessIdentity, ProcessSnapshotRecord> {
        &self.strongly_identified_processes
    }

    #[cfg(test)]
    pub(crate) fn insufficient_identity_processes(&self) -> &[InsufficientIdentityProcessRecord] {
        &self.insufficient_identity_processes
    }

    #[cfg(test)]
    pub(crate) fn identity_binding_invalidations(&self) -> &[ProcessIdentityBindingInvalidation] {
        &self.identity_binding_invalidations
    }

    #[cfg(test)]
    pub(crate) const fn targeted_process_observations(&self) -> &TargetedProcessObservations {
        &self.targeted_process_observations
    }

    pub(crate) const fn running_process_metrics(&self) -> &RunningProcessMetricsObservation {
        &self.running_process_metrics
    }

    pub(super) fn bind_running_process_metrics(
        mut self,
        observed_at: Instant,
        mut running_process_metrics: BTreeMap<ProcessIdentity, RunningProcessMetricsRecord>,
    ) -> Self {
        running_process_metrics.retain(|process_identity, _| {
            self.strongly_identified_processes
                .contains_key(process_identity)
        });
        self.observed_at = observed_at;
        self.running_process_metrics =
            RunningProcessMetricsObservation::Observed(running_process_metrics);
        self
    }

    pub(crate) fn validated_ancestry(
        &self,
        process_identity: &ProcessIdentity,
        parent_walk_depth: ParentWalkDepth,
    ) -> AncestryLookup {
        let Some(process_record) = self.strongly_identified_processes.get(process_identity) else {
            return AncestryLookup::IdentityNotInSnapshot(process_identity.clone());
        };

        let mut current_identity = process_record.identity().clone();
        let mut visited_identities = BTreeSet::from([current_identity.clone()]);
        let mut edges = Vec::new();
        loop {
            if edges.len() == parent_walk_depth.0 {
                return AncestryLookup::observed(
                    edges,
                    AncestryTerminal::DepthCapped { current_identity },
                );
            }

            let Some(current_record) = self.strongly_identified_processes.get(&current_identity)
            else {
                return AncestryLookup::observed(
                    edges,
                    AncestryTerminal::SnapshotRecordUnavailable { current_identity },
                );
            };
            match current_record.parentage_validation_outcome() {
                ProcessFieldObservation::Observed(ParentageValidationOutcome::Root { child }) => {
                    return AncestryLookup::observed(edges, AncestryTerminal::root(child));
                },
                ProcessFieldObservation::Observed(ParentageValidationOutcome::ValidatedEdge(
                    edge,
                )) => {
                    if visited_identities.contains(edge.parent()) {
                        return AncestryLookup::observed(
                            edges,
                            AncestryTerminal::RejectedEdge {
                                edge:      edge.endpoints().clone(),
                                rejection: ParentEdgeRejection::Cycle,
                            },
                        );
                    }
                    visited_identities.insert(edge.parent().clone());
                    current_identity = edge.parent().clone();
                    edges.push(edge.clone());
                },
                ProcessFieldObservation::Observed(
                    ParentageValidationOutcome::UnavailableParent {
                        child,
                        parent_identity,
                    },
                ) => {
                    return AncestryLookup::observed(
                        edges,
                        AncestryTerminal::UnavailableParent {
                            child:           child.clone(),
                            parent_identity: parent_identity.clone(),
                        },
                    );
                },
                ProcessFieldObservation::Observed(
                    ParentageValidationOutcome::UnavailableIdentifiedParent { edge },
                ) => {
                    return AncestryLookup::observed(
                        edges,
                        AncestryTerminal::UnavailableIdentifiedParent { edge: edge.clone() },
                    );
                },
                ProcessFieldObservation::Observed(
                    ParentageValidationOutcome::CreationOrderUnavailable { edge, unavailable },
                ) => {
                    return AncestryLookup::observed(
                        edges,
                        AncestryTerminal::CreationOrderUnavailable {
                            edge:        edge.clone(),
                            unavailable: *unavailable,
                        },
                    );
                },
                ProcessFieldObservation::Observed(ParentageValidationOutcome::RejectedEdge {
                    edge,
                    rejection,
                }) => {
                    return AncestryLookup::observed(
                        edges,
                        AncestryTerminal::RejectedEdge {
                            edge:      edge.clone(),
                            rejection: rejection.clone(),
                        },
                    );
                },
                ProcessFieldObservation::Unavailable(_)
                | ProcessFieldObservation::Invalidated(_) => {
                    return AncestryLookup::observed(
                        edges,
                        AncestryTerminal::ParentEvidenceUnavailable {
                            child: current_identity,
                        },
                    );
                },
            }
        }
    }
}

/// A cap on validated parent edges followed from one process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParentWalkDepth(usize);

impl ParentWalkDepth {
    pub(crate) const fn new(value: usize) -> Self { Self(value) }
}

/// Result of looking up one identity's validated ancestry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AncestryLookup {
    Observed(ValidatedAncestry),
    IdentityNotInSnapshot(ProcessIdentity),
}

impl AncestryLookup {
    const fn observed(edges: Vec<ValidatedParentEdge>, terminal: AncestryTerminal) -> Self {
        Self::Observed(ValidatedAncestry { edges, terminal })
    }

    #[cfg(test)]
    pub(crate) const fn observed_for_test(
        edges: Vec<ValidatedParentEdge>,
        terminal: AncestryTerminal,
    ) -> Self {
        Self::observed(edges, terminal)
    }
}

/// A depth-capped chain of validated parent edges.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ValidatedAncestry {
    edges:    Vec<ValidatedParentEdge>,
    terminal: AncestryTerminal,
}

impl ValidatedAncestry {
    pub(crate) fn edges(&self) -> &[ValidatedParentEdge] { &self.edges }

    pub(crate) const fn terminal(&self) -> &AncestryTerminal { &self.terminal }
}

/// The fact that ended a validated ancestry walk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AncestryTerminal {
    Root {
        root: ProcessIdentity,
    },
    DepthCapped {
        current_identity: ProcessIdentity,
    },
    UnavailableParent {
        child:           ProcessIdentity,
        parent_identity: InsufficientProcessIdentity,
    },
    UnavailableIdentifiedParent {
        edge: StrongParentEdge,
    },
    CreationOrderUnavailable {
        edge:        StrongParentEdge,
        unavailable: ProcessCreationOrderUnavailable,
    },
    ParentEvidenceUnavailable {
        child: ProcessIdentity,
    },
    RejectedEdge {
        edge:      StrongParentEdge,
        rejection: ParentEdgeRejection,
    },
    SnapshotRecordUnavailable {
        current_identity: ProcessIdentity,
    },
}

impl AncestryTerminal {
    fn root(process_identity: &ProcessIdentity) -> Self {
        Self::Root {
            root: process_identity.clone(),
        }
    }
}

/// Why fields sampled from a PID cannot be bound to one strong identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessIdentityBindingInvalidation {
    PlatformIdentityChanged {
        before: ObservedProcessIdentity,
        after:  ObservedProcessIdentity,
    },
    LaterIdentityObservationChangedCurrentEvidence {
        prior: ObservedProcessIdentity,
        later: ObservedProcessIdentity,
    },
    #[cfg(test)]
    ProcessFieldSourceIdentityMismatch {
        current: ProcessIdentity,
        source:  ProcessIdentity,
    },
    #[cfg(test)]
    ProcessFieldSourceLifetimeUnproven { current: ProcessIdentity },
}

/// Exact results for a requested strong-identity refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetedProcessObservations {
    NotRequested,
    Outcomes(BTreeMap<ProcessIdentity, TargetedProcessObservation>),
}

/// The semantic outcome for one requested strong process identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TargetedProcessObservation {
    Observed,
    Gone,
    Replaced { replacement: ProcessIdentity },
    FieldsUnavailable(ProcessFieldUnavailable),
    IdentityUnavailable(InsufficientProcessIdentity),
    IdentityBindingInvalidated(ProcessIdentityBindingInvalidation),
}

/// Process fields proven to belong to one strong process lifetime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct IdentityBoundProcessObservation {
    identity:                ProcessIdentity,
    creation_order_evidence: ProcessCreationOrderEvidence,
    executable:              ProcessFieldObservation<PathBuf>,
    argv:                    ProcessFieldObservation<Vec<OsString>>,
    cwd:                     ProcessFieldObservation<PathBuf>,
    parent:                  ProcessFieldObservation<ReportedParent>,
}

/// Fields copied from one `sysinfo` process record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProcessFieldSample {
    executable: ProcessFieldObservation<PathBuf>,
    argv:       ProcessFieldObservation<Vec<OsString>>,
    cwd:        ProcessFieldObservation<PathBuf>,
}

impl ProcessFieldSample {
    pub(super) fn observe(process: &sysinfo::Process) -> Self {
        let executable = process.exe().map_or(
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformDidNotReport),
            |executable| ProcessFieldObservation::Observed(executable.to_path_buf()),
        );
        let argv = if process.cmd().is_empty() {
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformDidNotReport)
        } else {
            ProcessFieldObservation::Observed(process.cmd().to_vec())
        };
        let cwd = process.cwd().map_or(
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformDidNotReport),
            |cwd| ProcessFieldObservation::Observed(cwd.to_path_buf()),
        );
        Self {
            executable,
            argv,
            cwd,
        }
    }

    pub(super) fn unavailable(process_field_unavailable: ProcessFieldUnavailable) -> Self {
        Self {
            executable: ProcessFieldObservation::Unavailable(process_field_unavailable.clone()),
            argv:       ProcessFieldObservation::Unavailable(process_field_unavailable.clone()),
            cwd:        ProcessFieldObservation::Unavailable(process_field_unavailable),
        }
    }

    #[cfg(test)]
    pub(super) const fn for_test(executable: PathBuf, argv: Vec<OsString>, cwd: PathBuf) -> Self {
        Self {
            executable: ProcessFieldObservation::Observed(executable),
            argv:       ProcessFieldObservation::Observed(argv),
            cwd:        ProcessFieldObservation::Observed(cwd),
        }
    }
}

/// Lifetime binding carried by a process-field sampling boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProcessFieldLifetimeBinding {
    FreshSystemSamplingInterval,
    #[cfg(test)]
    Strong(ProcessIdentity),
    #[cfg(test)]
    UnprovenLongLivedSystemSample,
}

/// Whether repeated fresh-system observations agree across every sampled field.
#[derive(Clone, Debug, Eq, PartialEq)]
enum ProcessFieldSamplingEvidence {
    StableAcrossFreshSamples(ProcessFieldSample),
    ObservationsDiffered,
    StabilityUnproven,
}

/// Process-field evidence paired with its process-lifetime source evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProcessFieldSourceObservation {
    lifetime_binding:        ProcessFieldLifetimeBinding,
    field_sampling_evidence: ProcessFieldSamplingEvidence,
}

impl ProcessFieldSourceObservation {
    pub(super) fn repeated_fresh_system_samples(
        initial: ProcessFieldSample,
        repeated: ProcessFieldSample,
    ) -> Self {
        let ProcessFieldSample {
            executable: initial_executable,
            argv: initial_argv,
            cwd: initial_cwd,
        } = initial;
        let field_sampling_evidence = if initial_executable == repeated.executable
            && initial_argv == repeated.argv
            && initial_cwd == repeated.cwd
        {
            ProcessFieldSamplingEvidence::StableAcrossFreshSamples(repeated)
        } else {
            ProcessFieldSamplingEvidence::ObservationsDiffered
        };
        Self {
            lifetime_binding: ProcessFieldLifetimeBinding::FreshSystemSamplingInterval,
            field_sampling_evidence,
        }
    }

    pub(super) const fn fresh_system_stability_unproven() -> Self {
        Self {
            lifetime_binding:        ProcessFieldLifetimeBinding::FreshSystemSamplingInterval,
            field_sampling_evidence: ProcessFieldSamplingEvidence::StabilityUnproven,
        }
    }

    pub(super) fn repeated_unavailable_fresh_system_samples(
        process_field_unavailable: ProcessFieldUnavailable,
    ) -> Self {
        Self {
            lifetime_binding:        ProcessFieldLifetimeBinding::FreshSystemSamplingInterval,
            field_sampling_evidence: ProcessFieldSamplingEvidence::StableAcrossFreshSamples(
                ProcessFieldSample::unavailable(process_field_unavailable),
            ),
        }
    }

    #[cfg(test)]
    const fn strong_identity(
        source_identity: ProcessIdentity,
        process_field_sample: ProcessFieldSample,
    ) -> Self {
        Self {
            lifetime_binding:        ProcessFieldLifetimeBinding::Strong(source_identity),
            field_sampling_evidence: ProcessFieldSamplingEvidence::StableAcrossFreshSamples(
                process_field_sample,
            ),
        }
    }

    #[cfg(test)]
    const fn unproven_long_lived_system_sample(process_field_sample: ProcessFieldSample) -> Self {
        Self {
            lifetime_binding:        ProcessFieldLifetimeBinding::UnprovenLongLivedSystemSample,
            field_sampling_evidence: ProcessFieldSamplingEvidence::StableAcrossFreshSamples(
                process_field_sample,
            ),
        }
    }

    #[cfg(test)]
    pub(super) const fn lifetime_binding(&self) -> &ProcessFieldLifetimeBinding {
        &self.lifetime_binding
    }
}

/// Whether one field sample has a strong lifetime binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ProcessSamplingOutcome {
    IdentityBound(IdentityBoundProcessObservation),
    InsufficientIdentity(InsufficientIdentityProcessRecord),
    IdentityBindingInvalidated(ProcessIdentityBindingInvalidation),
}

impl ProcessSamplingOutcome {
    pub(super) fn bind_fields_to_identity(
        identity_before_fields: PlatformProcessObservation,
        process_field_source_observation: ProcessFieldSourceObservation,
        identity_after_fields: PlatformProcessObservation,
    ) -> Self {
        let PlatformProcessObservation {
            lifetime: identity_before_lifetime,
            parent: identity_before_parent,
        } = identity_before_fields;
        let identity_before = identity_before_lifetime.identity().clone();
        let identity_after = identity_after_fields.lifetime.identity().clone();
        match (identity_before, identity_after) {
            (ObservedProcessIdentity::Strong(before), ObservedProcessIdentity::Strong(after))
                if before == after =>
            {
                let ProcessFieldSourceObservation {
                    lifetime_binding,
                    field_sampling_evidence,
                } = process_field_source_observation;
                match lifetime_binding {
                    #[cfg(test)]
                    ProcessFieldLifetimeBinding::Strong(source) if source != before => {
                        return Self::IdentityBindingInvalidated(
                            ProcessIdentityBindingInvalidation::ProcessFieldSourceIdentityMismatch {
                                current: before,
                                source,
                            },
                        );
                    },
                    #[cfg(test)]
                    ProcessFieldLifetimeBinding::UnprovenLongLivedSystemSample => {
                        return Self::IdentityBindingInvalidated(
                            ProcessIdentityBindingInvalidation::ProcessFieldSourceLifetimeUnproven {
                                current: before,
                            },
                        );
                    },
                    ProcessFieldLifetimeBinding::FreshSystemSamplingInterval => {},
                    #[cfg(test)]
                    ProcessFieldLifetimeBinding::Strong(_) => {},
                }
                let parent = if identity_before_parent == identity_after_fields.parent {
                    identity_after_fields.parent
                } else {
                    ProcessFieldObservation::Invalidated(
                        ProcessFieldInvalidation::ParentIdentityChangedDuringSampling,
                    )
                };
                let process_field_sample = match field_sampling_evidence {
                    ProcessFieldSamplingEvidence::StableAcrossFreshSamples(
                        process_field_sample,
                    ) => process_field_sample,
                    ProcessFieldSamplingEvidence::ObservationsDiffered => {
                        let invalidation =
                            ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling;
                        return Self::identity_bound_invalidated_fields(
                            before,
                            identity_after_fields.lifetime.creation_order_evidence(),
                            invalidation,
                            parent,
                        );
                    },
                    ProcessFieldSamplingEvidence::StabilityUnproven => {
                        let invalidation = ProcessFieldInvalidation::ProcessFieldStabilityUnproven;
                        return Self::identity_bound_invalidated_fields(
                            before,
                            identity_after_fields.lifetime.creation_order_evidence(),
                            invalidation,
                            parent,
                        );
                    },
                };
                Self::IdentityBound(IdentityBoundProcessObservation {
                    identity: before,
                    creation_order_evidence: identity_after_fields
                        .lifetime
                        .creation_order_evidence()
                        .clone(),
                    executable: process_field_sample.executable,
                    argv: process_field_sample.argv,
                    cwd: process_field_sample.cwd,
                    parent,
                })
            },
            (
                ObservedProcessIdentity::Insufficient(before),
                ObservedProcessIdentity::Insufficient(after),
            ) if before == after => {
                let invalidation = ProcessFieldInvalidation::ProcessIdentityNotStableDuringSampling;
                Self::InsufficientIdentity(InsufficientIdentityProcessRecord {
                    identity:   after,
                    executable: ProcessFieldObservation::Invalidated(invalidation.clone()),
                    argv:       ProcessFieldObservation::Invalidated(invalidation.clone()),
                    cwd:        ProcessFieldObservation::Invalidated(invalidation.clone()),
                    parent:     ProcessFieldObservation::Invalidated(invalidation),
                })
            },
            (before, after) => Self::IdentityBindingInvalidated(
                ProcessIdentityBindingInvalidation::PlatformIdentityChanged { before, after },
            ),
        }
    }

    fn identity_bound_invalidated_fields(
        identity: ProcessIdentity,
        creation_order_evidence: &ProcessCreationOrderEvidence,
        invalidation: ProcessFieldInvalidation,
        parent: ProcessFieldObservation<ReportedParent>,
    ) -> Self {
        Self::IdentityBound(IdentityBoundProcessObservation {
            identity,
            creation_order_evidence: creation_order_evidence.clone(),
            executable: ProcessFieldObservation::Invalidated(invalidation.clone()),
            argv: ProcessFieldObservation::Invalidated(invalidation.clone()),
            cwd: ProcessFieldObservation::Invalidated(invalidation),
            parent,
        })
    }

    pub(super) fn reconcile_later_identity_observation(
        self,
        later_identity: &ObservedProcessIdentity,
    ) -> Self {
        let prior_identity = match &self {
            Self::IdentityBound(process_observation) => {
                ObservedProcessIdentity::Strong(process_observation.identity.clone())
            },
            Self::InsufficientIdentity(process_observation) => {
                ObservedProcessIdentity::Insufficient(process_observation.identity.clone())
            },
            Self::IdentityBindingInvalidated(
                ProcessIdentityBindingInvalidation::PlatformIdentityChanged { after, .. },
            ) => after.clone(),
            Self::IdentityBindingInvalidated(
                ProcessIdentityBindingInvalidation::LaterIdentityObservationChangedCurrentEvidence {
                    later,
                    ..
                },
            ) => later.clone(),
            #[cfg(test)]
            Self::IdentityBindingInvalidated(
                ProcessIdentityBindingInvalidation::ProcessFieldSourceIdentityMismatch {
                    current,
                    ..
                }
                | ProcessIdentityBindingInvalidation::ProcessFieldSourceLifetimeUnproven {
                    current,
                },
            ) => ObservedProcessIdentity::Strong(current.clone()),
        };
        if prior_identity == *later_identity {
            self
        } else {
            Self::IdentityBindingInvalidated(
                ProcessIdentityBindingInvalidation::LaterIdentityObservationChangedCurrentEvidence {
                    prior: prior_identity,
                    later: later_identity.clone(),
                },
            )
        }
    }
}

/// One refresh's sampled processes and exact-target results.
pub(super) struct ProcessRefreshObservations {
    pub(super) process_sampling_outcomes:     Vec<ProcessSamplingOutcome>,
    pub(super) targeted_process_observations: TargetedProcessObservations,
    pub(super) full_process_refresh_evidence: FullProcessRefreshEvidence,
}

/// Evidence returned by a requested full process refresh.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FullProcessRefreshEvidence {
    NotRequested,
    NoProcessesUpdated,
    UpdatedProcesses {
        latest_identity_observations: BTreeMap<u32, ObservedProcessIdentity>,
    },
}

/// Whether a requested PID still has a cached `sysinfo` record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TargetedProcessPresence {
    FieldsUnavailable {
        process_sampling_outcome:  ProcessSamplingOutcome,
        process_field_unavailable: ProcessFieldUnavailable,
    },
    Sampled(ProcessSamplingOutcome),
}

impl TargetedProcessPresence {
    pub(super) fn reconcile_later_identity_observation(
        self,
        later_identity: &ObservedProcessIdentity,
    ) -> Self {
        match self {
            Self::FieldsUnavailable {
                process_sampling_outcome,
                process_field_unavailable,
            } => Self::FieldsUnavailable {
                process_sampling_outcome: process_sampling_outcome
                    .reconcile_later_identity_observation(later_identity),
                process_field_unavailable,
            },
            Self::Sampled(process_sampling_outcome) => Self::Sampled(
                process_sampling_outcome.reconcile_later_identity_observation(later_identity),
            ),
        }
    }
}

/// Whether a sampled PID can enter an exact targeted snapshot.
pub(super) enum TargetedSampleAdmission {
    Admitted(ProcessSamplingOutcome),
    Excluded,
}

/// Exact target outcome paired with its strong-snapshot admission decision.
pub(super) struct TargetedProcessSamplingResult {
    pub(super) observation: TargetedProcessObservation,
    pub(super) admission:   TargetedSampleAdmission,
}

impl TargetedProcessSamplingResult {
    pub(super) fn classify(
        requested_identity: &ProcessIdentity,
        targeted_process_presence: TargetedProcessPresence,
    ) -> Self {
        match targeted_process_presence {
            TargetedProcessPresence::FieldsUnavailable {
                process_sampling_outcome,
                process_field_unavailable,
            } => {
                let observation = match process_sampling_outcome {
                    ProcessSamplingOutcome::IdentityBound(process_observation)
                        if process_observation.identity == *requested_identity =>
                    {
                        TargetedProcessObservation::FieldsUnavailable(process_field_unavailable)
                    },
                    ProcessSamplingOutcome::IdentityBound(process_observation) => {
                        TargetedProcessObservation::Replaced {
                            replacement: process_observation.identity,
                        }
                    },
                    ProcessSamplingOutcome::InsufficientIdentity(process_observation) => {
                        Self::from_insufficient_identity(process_observation.identity)
                    },
                    ProcessSamplingOutcome::IdentityBindingInvalidated(invalidation) => {
                        Self::from_identity_binding_invalidation(requested_identity, invalidation)
                    },
                };
                Self {
                    observation,
                    admission: TargetedSampleAdmission::Excluded,
                }
            },
            TargetedProcessPresence::Sampled(ProcessSamplingOutcome::IdentityBound(
                process_observation,
            )) if process_observation.identity == *requested_identity => Self {
                observation: TargetedProcessObservation::Observed,
                admission:   TargetedSampleAdmission::Admitted(
                    ProcessSamplingOutcome::IdentityBound(process_observation),
                ),
            },
            TargetedProcessPresence::Sampled(ProcessSamplingOutcome::IdentityBound(
                process_observation,
            )) => Self {
                observation: TargetedProcessObservation::Replaced {
                    replacement: process_observation.identity,
                },
                admission:   TargetedSampleAdmission::Excluded,
            },
            TargetedProcessPresence::Sampled(ProcessSamplingOutcome::InsufficientIdentity(
                process_observation,
            )) => Self {
                observation: Self::from_insufficient_identity(process_observation.identity),
                admission:   TargetedSampleAdmission::Excluded,
            },
            TargetedProcessPresence::Sampled(
                ProcessSamplingOutcome::IdentityBindingInvalidated(invalidation),
            ) => Self {
                observation: Self::from_identity_binding_invalidation(
                    requested_identity,
                    invalidation,
                ),
                admission:   TargetedSampleAdmission::Excluded,
            },
        }
    }

    const fn from_insufficient_identity(
        insufficient_identity: InsufficientProcessIdentity,
    ) -> TargetedProcessObservation {
        match insufficient_identity {
            InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { .. } => {
                TargetedProcessObservation::Gone
            },
            insufficient_identity => {
                TargetedProcessObservation::IdentityUnavailable(insufficient_identity)
            },
        }
    }

    fn from_identity_binding_invalidation(
        requested_identity: &ProcessIdentity,
        invalidation: ProcessIdentityBindingInvalidation,
    ) -> TargetedProcessObservation {
        let later_identity = match &invalidation {
            ProcessIdentityBindingInvalidation::PlatformIdentityChanged { after, .. } => after,
            ProcessIdentityBindingInvalidation::LaterIdentityObservationChangedCurrentEvidence {
                later,
                ..
            } => later,
            #[cfg(test)]
            ProcessIdentityBindingInvalidation::ProcessFieldSourceIdentityMismatch { .. }
            | ProcessIdentityBindingInvalidation::ProcessFieldSourceLifetimeUnproven { .. } => {
                return TargetedProcessObservation::IdentityBindingInvalidated(invalidation);
            },
        };
        match later_identity {
            ObservedProcessIdentity::Strong(replacement) if replacement != requested_identity => {
                TargetedProcessObservation::Replaced {
                    replacement: replacement.clone(),
                }
            },
            ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { .. },
            ) => TargetedProcessObservation::Gone,
            ObservedProcessIdentity::Insufficient(insufficient_identity) => {
                TargetedProcessObservation::IdentityUnavailable(insufficient_identity.clone())
            },
            ObservedProcessIdentity::Strong(_) => {
                TargetedProcessObservation::IdentityBindingInvalidated(invalidation)
            },
        }
    }
}

/// Cached process incarnations and Cargo candidate parsing.
#[derive(Debug, Default)]
pub(super) struct ProcessIncarnationCache {
    previous_incarnations:         BTreeMap<ProcessIdentity, ProcessIncarnation>,
    unclassified_cargo_candidates: BTreeMap<ProcessIdentity, UnclassifiedCargoCandidateIncarnation>,
}

impl ProcessIncarnationCache {
    pub(super) fn snapshot_from(
        &mut self,
        observed_at: Instant,
        scope: ProcessSnapshotScope,
        process_refresh_observations: ProcessRefreshObservations,
    ) -> ProcessObservationSnapshot {
        let ProcessRefreshObservations {
            process_sampling_outcomes,
            targeted_process_observations,
            full_process_refresh_evidence,
        } = process_refresh_observations;
        let mut identity_bound_processes = BTreeMap::new();
        let mut insufficient_identity_processes = Vec::new();
        let mut identity_binding_invalidations = Vec::new();
        for process_sampling_outcome in process_sampling_outcomes {
            match process_sampling_outcome {
                ProcessSamplingOutcome::IdentityBound(process_observation) => {
                    identity_bound_processes
                        .insert(process_observation.identity.clone(), process_observation);
                },
                ProcessSamplingOutcome::InsufficientIdentity(process_observation) => {
                    insufficient_identity_processes.push(process_observation);
                },
                ProcessSamplingOutcome::IdentityBindingInvalidated(invalidation) => {
                    identity_binding_invalidations.push(invalidation);
                },
            }
        }

        self.evict_processes_proven_absent_or_replaced(&full_process_refresh_evidence);

        let mut strongly_identified_processes = BTreeMap::new();
        for (process_identity, process_observation) in &identity_bound_processes {
            let process_snapshot_record = self.process_snapshot_record(
                process_identity,
                process_observation,
                &identity_bound_processes,
            );
            strongly_identified_processes.insert(process_identity.clone(), process_snapshot_record);
        }

        for process_snapshot in strongly_identified_processes.values() {
            match process_snapshot.incarnation_evidence() {
                ProcessIncarnationEvidence::Strong {
                    incarnation,
                    incarnation_state: ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                } => {
                    let process_observation =
                        &identity_bound_processes[process_snapshot.identity()];
                    self.cache_cargo_candidate(
                        process_snapshot.identity(),
                        incarnation,
                        &process_observation.executable,
                        &process_observation.argv,
                    );
                    self.previous_incarnations
                        .insert(process_snapshot.identity().clone(), incarnation.clone());
                },
                ProcessIncarnationEvidence::Strong {
                    incarnation,
                    incarnation_state:
                        ProcessIncarnationState::NewlyObserved | ProcessIncarnationState::Unchanged,
                } => {
                    self.cache_cargo_candidate(
                        process_snapshot.identity(),
                        incarnation,
                        process_snapshot.executable(),
                        process_snapshot.argv(),
                    );
                    self.previous_incarnations
                        .insert(process_snapshot.identity().clone(), incarnation.clone());
                },
                ProcessIncarnationEvidence::Insufficient(_) => {
                    self.unclassified_cargo_candidates
                        .remove(process_snapshot.identity());
                },
            }
        }

        ProcessObservationSnapshot {
            observed_at,
            scope,
            strongly_identified_processes,
            insufficient_identity_processes,
            identity_binding_invalidations,
            targeted_process_observations,
            running_process_metrics: RunningProcessMetricsObservation::NotRequested,
        }
    }

    fn process_snapshot_record(
        &self,
        process_identity: &ProcessIdentity,
        process_observation: &IdentityBoundProcessObservation,
        identity_bound_processes: &BTreeMap<ProcessIdentity, IdentityBoundProcessObservation>,
    ) -> ProcessSnapshotRecord {
        let (
            ProcessFieldObservation::Observed(executable),
            ProcessFieldObservation::Observed(argv),
        ) = (&process_observation.executable, &process_observation.argv)
        else {
            return Self::insufficient_process_snapshot_record(
                process_identity,
                process_observation,
                identity_bound_processes,
            );
        };
        let executable_argv_fingerprint =
            ProcessFingerprint::from_observed_fields(executable, argv);
        let incarnation =
            ProcessIncarnation::new(process_identity.clone(), executable_argv_fingerprint);
        let incarnation_state = self.incarnation_state(&incarnation);
        match incarnation_state {
            ProcessIncarnationState::NewlyObserved | ProcessIncarnationState::Unchanged => {
                ProcessSnapshotRecord {
                    identity:                     process_identity.clone(),
                    incarnation_evidence:         ProcessIncarnationEvidence::Strong {
                        incarnation,
                        incarnation_state,
                    },
                    executable:                   process_observation.executable.clone(),
                    argv:                         process_observation.argv.clone(),
                    cwd:                          process_observation.cwd.clone(),
                    parentage_validation_outcome: validate_parentage(
                        process_identity,
                        &process_observation.creation_order_evidence,
                        process_observation.parent.clone(),
                        identity_bound_processes,
                    ),
                }
            },
            ProcessIncarnationState::ExecutableOrArgumentsChanged { .. } => {
                let invalidation = ProcessFieldInvalidation::ExecutableOrArgumentsChanged;
                ProcessSnapshotRecord {
                    identity:                     process_identity.clone(),
                    incarnation_evidence:         ProcessIncarnationEvidence::Strong {
                        incarnation,
                        incarnation_state,
                    },
                    executable:                   ProcessFieldObservation::Invalidated(
                        invalidation.clone(),
                    ),
                    argv:                         ProcessFieldObservation::Invalidated(
                        invalidation.clone(),
                    ),
                    cwd:                          ProcessFieldObservation::Invalidated(
                        invalidation.clone(),
                    ),
                    parentage_validation_outcome: ProcessFieldObservation::Invalidated(
                        invalidation,
                    ),
                }
            },
        }
    }

    fn insufficient_process_snapshot_record(
        process_identity: &ProcessIdentity,
        process_observation: &IdentityBoundProcessObservation,
        identity_bound_processes: &BTreeMap<ProcessIdentity, IdentityBoundProcessObservation>,
    ) -> ProcessSnapshotRecord {
        let invalidation = match (&process_observation.executable, &process_observation.argv) {
            (ProcessFieldObservation::Invalidated(invalidation), _)
            | (_, ProcessFieldObservation::Invalidated(invalidation)) => invalidation.clone(),
            _ => ProcessFieldInvalidation::ExecIncarnationEvidenceInsufficient,
        };
        let parentage_validation_outcome = match &invalidation {
            ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
            | ProcessFieldInvalidation::ProcessFieldStabilityUnproven => validate_parentage(
                process_identity,
                &process_observation.creation_order_evidence,
                process_observation.parent.clone(),
                identity_bound_processes,
            ),
            _ => ProcessFieldObservation::Invalidated(invalidation.clone()),
        };
        ProcessSnapshotRecord {
            identity: process_identity.clone(),
            incarnation_evidence: ProcessIncarnationEvidence::Insufficient(
                InsufficientProcessIncarnationEvidence {
                    executable: process_observation.executable.clone(),
                    argv:       process_observation.argv.clone(),
                },
            ),
            executable: process_observation.executable.clone(),
            argv: process_observation.argv.clone(),
            cwd: ProcessFieldObservation::Invalidated(invalidation),
            parentage_validation_outcome,
        }
    }

    fn evict_processes_proven_absent_or_replaced(
        &mut self,
        full_process_refresh_evidence: &FullProcessRefreshEvidence,
    ) {
        let FullProcessRefreshEvidence::UpdatedProcesses {
            latest_identity_observations,
        } = full_process_refresh_evidence
        else {
            return;
        };
        let retain_process_identity =
            |process_identity: &ProcessIdentity| match latest_identity_observations
                .get(&process_identity.pid())
            {
                Some(ObservedProcessIdentity::Strong(current_identity)) => {
                    current_identity == process_identity
                },
                Some(ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { .. },
                )) => false,
                Some(ObservedProcessIdentity::Insufficient(_)) | None => true,
            };
        self.previous_incarnations
            .retain(|process_identity, _| retain_process_identity(process_identity));
        self.unclassified_cargo_candidates
            .retain(|process_identity, _| retain_process_identity(process_identity));
    }

    fn incarnation_state(&self, incarnation: &ProcessIncarnation) -> ProcessIncarnationState {
        match self.previous_incarnations.get(incarnation.identity()) {
            Some(previous_incarnation) if previous_incarnation == incarnation => {
                ProcessIncarnationState::Unchanged
            },
            Some(previous_incarnation) => ProcessIncarnationState::ExecutableOrArgumentsChanged {
                previous: previous_incarnation.clone(),
            },
            None => ProcessIncarnationState::NewlyObserved,
        }
    }

    fn cache_cargo_candidate(
        &mut self,
        process_identity: &ProcessIdentity,
        incarnation: &ProcessIncarnation,
        executable: &ProcessFieldObservation<PathBuf>,
        argv: &ProcessFieldObservation<Vec<OsString>>,
    ) {
        let process_candidate = CargoProcessCandidate::parse(executable, argv);
        if process_candidate == CargoProcessCandidate::NotCandidate {
            self.unclassified_cargo_candidates.remove(process_identity);
            return;
        }
        let must_cache = self
            .unclassified_cargo_candidates
            .get(process_identity)
            .is_none_or(|cached_incarnation| {
                cached_incarnation.incarnation != *incarnation
                    || cached_incarnation.candidate != process_candidate
            });
        if must_cache {
            self.unclassified_cargo_candidates.insert(
                process_identity.clone(),
                UnclassifiedCargoCandidateIncarnation {
                    incarnation: incarnation.clone(),
                    candidate:   process_candidate,
                },
            );
        }
    }

    #[cfg(test)]
    pub(super) fn remembers_unclassified_candidate(
        &self,
        process_identity: &ProcessIdentity,
    ) -> bool {
        self.unclassified_cargo_candidates
            .contains_key(process_identity)
    }

    #[cfg(test)]
    pub(super) fn remembers_incarnation(&self, process_identity: &ProcessIdentity) -> bool {
        self.previous_incarnations.contains_key(process_identity)
    }

    pub(super) fn cached_process_identities(&self) -> BTreeSet<ProcessIdentity> {
        self.previous_incarnations
            .keys()
            .chain(self.unclassified_cargo_candidates.keys())
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnclassifiedCargoCandidateIncarnation {
    incarnation: ProcessIncarnation,
    candidate:   CargoProcessCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoProcessCandidate {
    NotCandidate,
    Cargo,
    Compiler,
    Wrapper,
}

impl CargoProcessCandidate {
    const CARGO_EXECUTABLE: &'static str = "cargo";
    const CARGO_WINDOWS_EXECUTABLE: &'static str = "cargo.exe";
    const COMPILER_EXECUTABLE: &'static str = "rustc";
    const COMPILER_WINDOWS_EXECUTABLE: &'static str = "rustc.exe";
    const SCCACHE_EXECUTABLE: &'static str = "sccache";
    const SCCACHE_WINDOWS_EXECUTABLE: &'static str = "sccache.exe";
    const CLIPPY_DRIVER_EXECUTABLE: &'static str = "clippy-driver";
    const CLIPPY_DRIVER_WINDOWS_EXECUTABLE: &'static str = "clippy-driver.exe";

    fn parse(
        executable: &ProcessFieldObservation<PathBuf>,
        argv: &ProcessFieldObservation<Vec<OsString>>,
    ) -> Self {
        let ProcessFieldObservation::Observed(executable) = executable else {
            return Self::NotCandidate;
        };
        match executable.file_name().and_then(std::ffi::OsStr::to_str) {
            Some(Self::CARGO_EXECUTABLE | Self::CARGO_WINDOWS_EXECUTABLE) => Self::Cargo,
            Some(Self::COMPILER_EXECUTABLE | Self::COMPILER_WINDOWS_EXECUTABLE) => Self::Compiler,
            Some(
                Self::SCCACHE_EXECUTABLE
                | Self::SCCACHE_WINDOWS_EXECUTABLE
                | Self::CLIPPY_DRIVER_EXECUTABLE
                | Self::CLIPPY_DRIVER_WINDOWS_EXECUTABLE,
            ) => Self::Wrapper,
            _ => Self::wrapper_from_arguments(argv),
        }
    }

    fn wrapper_from_arguments(argv: &ProcessFieldObservation<Vec<OsString>>) -> Self {
        let ProcessFieldObservation::Observed(argv) = argv else {
            return Self::NotCandidate;
        };
        if argv
            .iter()
            .filter_map(|argument| std::path::Path::new(argument).file_name())
            .filter_map(std::ffi::OsStr::to_str)
            .any(|argument| {
                matches!(
                    argument,
                    Self::COMPILER_EXECUTABLE | Self::COMPILER_WINDOWS_EXECUTABLE
                )
            })
        {
            Self::Wrapper
        } else {
            Self::NotCandidate
        }
    }
}

fn validate_parentage(
    child: &ProcessIdentity,
    child_creation_order_evidence: &ProcessCreationOrderEvidence,
    parent_source: ProcessFieldObservation<ReportedParent>,
    identity_bound_processes: &BTreeMap<ProcessIdentity, IdentityBoundProcessObservation>,
) -> ProcessFieldObservation<ParentageValidationOutcome> {
    match parent_source {
        ProcessFieldObservation::Observed(ReportedParent::Root) => {
            ProcessFieldObservation::Observed(ParentageValidationOutcome::Root {
                child: child.clone(),
            })
        },
        ProcessFieldObservation::Observed(ReportedParent::Identified(parent_identity)) => {
            let edge = StrongParentEdge {
                parent: parent_identity.clone(),
                child:  child.clone(),
            };
            if parent_identity == *child {
                return ProcessFieldObservation::Observed(
                    ParentageValidationOutcome::RejectedEdge {
                        edge,
                        rejection: ParentEdgeRejection::SelfParent,
                    },
                );
            }
            if let Some(parent_process_observation) = identity_bound_processes.get(&parent_identity)
            {
                return match parent_process_observation
                    .creation_order_evidence
                    .parent_relative_to_child(child_creation_order_evidence)
                {
                    ParentCreationOrder::CreatedAfterChild => ProcessFieldObservation::Observed(
                        ParentageValidationOutcome::RejectedEdge {
                            edge,
                            rejection: ParentEdgeRejection::CreatedAfterChild,
                        },
                    ),
                    ParentCreationOrder::NotCreatedAfterChild => ProcessFieldObservation::Observed(
                        ParentageValidationOutcome::ValidatedEdge(ValidatedParentEdge {
                            endpoints: edge,
                        }),
                    ),
                    ParentCreationOrder::Unavailable(unavailable) => {
                        ProcessFieldObservation::Observed(
                            ParentageValidationOutcome::CreationOrderUnavailable {
                                edge,
                                unavailable,
                            },
                        )
                    },
                };
            }

            identity_bound_processes
                .keys()
                .find(|current_identity| current_identity.pid() == parent_identity.pid())
                .map_or_else(
                    || {
                        ProcessFieldObservation::Observed(
                            ParentageValidationOutcome::UnavailableIdentifiedParent {
                                edge: edge.clone(),
                            },
                        )
                    },
                    |current_identity| {
                        ProcessFieldObservation::Observed(
                            ParentageValidationOutcome::RejectedEdge {
                                edge:      edge.clone(),
                                rejection: ParentEdgeRejection::IdentityReplaced {
                                    current: current_identity.clone(),
                                },
                            },
                        )
                    },
                )
        },
        ProcessFieldObservation::Observed(ReportedParent::IdentityUnavailable(parent_identity)) => {
            ProcessFieldObservation::Observed(ParentageValidationOutcome::UnavailableParent {
                child: child.clone(),
                parent_identity,
            })
        },
        ProcessFieldObservation::Unavailable(unavailable) => {
            ProcessFieldObservation::Unavailable(unavailable)
        },
        ProcessFieldObservation::Invalidated(invalidation) => {
            ProcessFieldObservation::Invalidated(invalidation)
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Instant;

    use super::AncestryLookup;
    use super::AncestryTerminal;
    use super::CargoProcessCandidate;
    use super::FullProcessRefreshEvidence;
    use super::IdentityBoundProcessObservation;
    use super::InsufficientIdentityProcessRecord;
    use super::ParentEdgeRejection;
    use super::ParentWalkDepth;
    use super::ParentageValidationOutcome;
    use super::ProcessFieldInvalidation;
    use super::ProcessFieldObservation;
    use super::ProcessFieldSample;
    use super::ProcessFieldSourceObservation;
    use super::ProcessFieldUnavailable;
    use super::ProcessIdentityBindingInvalidation;
    use super::ProcessIncarnationCache;
    use super::ProcessIncarnationEvidence;
    use super::ProcessIncarnationState;
    use super::ProcessRefreshObservations;
    use super::ProcessSamplingOutcome;
    use super::ProcessSnapshotScope;
    use super::ReportedParent;
    use super::TargetedProcessObservation;
    use super::TargetedProcessObservations;
    use super::TargetedProcessPresence;
    use super::TargetedProcessSamplingResult;
    use super::TargetedSampleAdmission;
    use super::ValidatedAncestry;
    use crate::process_observation::identity::InsufficientProcessIdentity;
    use crate::process_observation::identity::ObservedProcessIdentity;
    use crate::process_observation::identity::PlatformProcessObservation;
    use crate::process_observation::identity::ProcessCreationOrderEvidence;
    use crate::process_observation::identity::ProcessCreationOrderUnavailable;
    use crate::process_observation::identity::ProcessIdentity;

    fn identity(pid: u32, creation_token: u64) -> ProcessIdentity {
        ProcessIdentity::for_test(pid, creation_token)
    }

    fn observed_process(
        process_identity: ProcessIdentity,
        parent: ProcessFieldObservation<ReportedParent>,
    ) -> IdentityBoundProcessObservation {
        let creation_order_evidence =
            ProcessCreationOrderEvidence::for_test_identity(&process_identity);
        observed_process_with_creation_order(process_identity, creation_order_evidence, parent)
    }

    fn observed_process_with_creation_order(
        process_identity: ProcessIdentity,
        creation_order_evidence: ProcessCreationOrderEvidence,
        parent: ProcessFieldObservation<ReportedParent>,
    ) -> IdentityBoundProcessObservation {
        IdentityBoundProcessObservation {
            identity: process_identity,
            creation_order_evidence,
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv: ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd: ProcessFieldObservation::Observed(PathBuf::from("/workspace")),
            parent,
        }
    }

    fn snapshot(
        cache: &mut ProcessIncarnationCache,
        process_sampling_outcomes: Vec<ProcessSamplingOutcome>,
    ) -> super::ProcessObservationSnapshot {
        let post_sampling_identities = process_sampling_outcomes
            .iter()
            .filter_map(|process_sampling_outcome| match process_sampling_outcome {
                ProcessSamplingOutcome::IdentityBound(process_observation) => Some((
                    process_observation.identity.pid(),
                    ObservedProcessIdentity::Strong(process_observation.identity.clone()),
                )),
                ProcessSamplingOutcome::InsufficientIdentity(_)
                | ProcessSamplingOutcome::IdentityBindingInvalidated(_) => None,
            })
            .collect();
        snapshot_with_refresh_evidence(
            cache,
            process_sampling_outcomes,
            FullProcessRefreshEvidence::UpdatedProcesses {
                latest_identity_observations: post_sampling_identities,
            },
        )
    }

    fn snapshot_with_refresh_evidence(
        cache: &mut ProcessIncarnationCache,
        process_sampling_outcomes: Vec<ProcessSamplingOutcome>,
        full_process_refresh_evidence: FullProcessRefreshEvidence,
    ) -> super::ProcessObservationSnapshot {
        cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::FullSystem,
            ProcessRefreshObservations {
                process_sampling_outcomes,
                targeted_process_observations: TargetedProcessObservations::NotRequested,
                full_process_refresh_evidence,
            },
        )
    }

    fn updated_process_evidence(
        latest_identity_observations: BTreeMap<u32, ObservedProcessIdentity>,
    ) -> FullProcessRefreshEvidence {
        FullProcessRefreshEvidence::UpdatedProcesses {
            latest_identity_observations,
        }
    }

    fn strong_process(
        process_identity: ProcessIdentity,
        parent: ProcessFieldObservation<ReportedParent>,
    ) -> ProcessSamplingOutcome {
        ProcessSamplingOutcome::IdentityBound(observed_process(process_identity, parent))
    }

    fn platform_observation(process_identity: &ProcessIdentity) -> PlatformProcessObservation {
        PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test_identity(process_identity),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        )
    }

    fn unavailable_target_presence(
        identity_before_fields: PlatformProcessObservation,
        identity_after_fields: PlatformProcessObservation,
        process_field_unavailable: ProcessFieldUnavailable,
    ) -> TargetedProcessPresence {
        TargetedProcessPresence::FieldsUnavailable {
            process_sampling_outcome: ProcessSamplingOutcome::bind_fields_to_identity(
                identity_before_fields,
                ProcessFieldSourceObservation::repeated_unavailable_fresh_system_samples(
                    process_field_unavailable.clone(),
                ),
                identity_after_fields,
            ),
            process_field_unavailable,
        }
    }

    fn targeted_snapshot(
        cache: &mut ProcessIncarnationCache,
        requested_identity: ProcessIdentity,
        targeted_process_sampling_result: TargetedProcessSamplingResult,
    ) -> super::ProcessObservationSnapshot {
        let TargetedProcessSamplingResult {
            observation,
            admission,
        } = targeted_process_sampling_result;
        let process_sampling_outcomes = match admission {
            TargetedSampleAdmission::Admitted(process_sampling_outcome) => {
                vec![process_sampling_outcome]
            },
            TargetedSampleAdmission::Excluded => Vec::new(),
        };
        cache.snapshot_from(
            Instant::now(),
            ProcessSnapshotScope::TargetedIdentities(BTreeSet::from([requested_identity.clone()])),
            ProcessRefreshObservations {
                process_sampling_outcomes,
                targeted_process_observations: TargetedProcessObservations::Outcomes(
                    BTreeMap::from([(requested_identity, observation)]),
                ),
                full_process_refresh_evidence: FullProcessRefreshEvidence::NotRequested,
            },
        )
    }

    fn assert_exec_transition_invalidates_phase_two_evidence(
        process_record: &super::ProcessSnapshotRecord,
    ) {
        assert!(matches!(
            process_record.executable(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            process_record.argv(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            process_record.cwd(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
        assert!(matches!(
            process_record.parentage_validation_outcome(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ExecutableOrArgumentsChanged
            )
        ));
    }

    #[test]
    fn unavailable_exec_fields_make_incarnation_evidence_insufficient() {
        let process_identity = identity(7, 70);
        let process_observation = IdentityBoundProcessObservation {
            identity:                process_identity.clone(),
            creation_order_evidence: ProcessCreationOrderEvidence::for_test(0),
            executable:              ProcessFieldObservation::Unavailable(
                ProcessFieldUnavailable::PlatformDidNotReport,
            ),
            argv:                    ProcessFieldObservation::Unavailable(
                ProcessFieldUnavailable::PlatformLookupFailed,
            ),
            cwd:                     ProcessFieldObservation::Unavailable(
                ProcessFieldUnavailable::ProcessExited,
            ),
            parent:                  ProcessFieldObservation::Unavailable(
                ProcessFieldUnavailable::PlatformDidNotReport,
            ),
        };
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(process_observation)],
        );

        assert!(
            snapshot
                .strongly_identified_processes()
                .get(&process_identity)
                .is_some_and(|process| {
                    matches!(
                        process.incarnation_evidence(),
                        ProcessIncarnationEvidence::Insufficient(evidence)
                            if matches!(
                                evidence.executable(),
                                ProcessFieldObservation::Unavailable(
                                    ProcessFieldUnavailable::PlatformDidNotReport
                                )
                            ) && matches!(
                                evidence.argv(),
                                ProcessFieldObservation::Unavailable(
                                    ProcessFieldUnavailable::PlatformLookupFailed
                                )
                            )
                    ) && matches!(
                        process.cwd(),
                        ProcessFieldObservation::Invalidated(
                            ProcessFieldInvalidation::ExecIncarnationEvidenceInsufficient
                        )
                    ) && matches!(
                        process.parentage_validation_outcome(),
                        ProcessFieldObservation::Invalidated(
                            ProcessFieldInvalidation::ExecIncarnationEvidenceInsufficient
                        )
                    )
                })
        );
        assert!(!cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn unavailable_cwd_survives_with_strong_exec_and_parentage_evidence() {
        let process_identity = identity(38, 380);
        let mut process_observation = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        process_observation.cwd =
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::ProcessExited);
        let mut cache = ProcessIncarnationCache::default();

        let snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(process_observation)],
        );
        let process_record = &snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            process_record.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong { .. }
        ));
        assert!(matches!(
            process_record.executable(),
            ProcessFieldObservation::Observed(executable)
                if executable == &PathBuf::from("/usr/bin/cargo")
        ));
        assert!(matches!(
            process_record.argv(),
            ProcessFieldObservation::Observed(argv)
                if argv == &vec![OsString::from("cargo")]
        ));
        assert!(matches!(
            process_record.cwd(),
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::ProcessExited)
        ));
        assert!(matches!(
            process_record.parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::Root { .. })
        ));
    }

    #[test]
    fn unavailable_direct_parent_survives_with_strong_exec_and_cwd_evidence() {
        let process_identity = identity(39, 390);
        let process_observation = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformLookupFailed),
        );
        let mut cache = ProcessIncarnationCache::default();

        let snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(process_observation)],
        );
        let process_record = &snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            process_record.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong { .. }
        ));
        assert!(matches!(
            process_record.executable(),
            ProcessFieldObservation::Observed(executable)
                if executable == &PathBuf::from("/usr/bin/cargo")
        ));
        assert!(matches!(
            process_record.argv(),
            ProcessFieldObservation::Observed(argv)
                if argv == &vec![OsString::from("cargo")]
        ));
        assert!(matches!(
            process_record.cwd(),
            ProcessFieldObservation::Observed(cwd) if cwd == &PathBuf::from("/workspace")
        ));
        assert!(matches!(
            process_record.parentage_validation_outcome(),
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformLookupFailed)
        ));
    }

    #[test]
    fn argv_only_transition_invalidates_all_derived_evidence_until_stable_recovery() {
        let process_identity = identity(8, 80);
        let mut cache = ProcessIncarnationCache::default();
        let first_snapshot = snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );
        let mut changed_process = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        changed_process.argv = ProcessFieldObservation::Observed(vec![
            OsString::from("cargo"),
            OsString::from("check"),
        ]);
        let second_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(
                changed_process.clone(),
            )],
        );
        let first_process = &first_snapshot.strongly_identified_processes()[&process_identity];
        let transition_process =
            &second_snapshot.strongly_identified_processes()[&process_identity];
        assert!(matches!(
            (
                first_process.incarnation_evidence(),
                transition_process.incarnation_evidence()
            ),
            (
                ProcessIncarnationEvidence::Strong {
                    incarnation: first_incarnation,
                    ..
                },
                ProcessIncarnationEvidence::Strong {
                    incarnation: transition_incarnation,
                    incarnation_state:
                        ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                }
            ) if first_incarnation.executable_argv_fingerprint()
                != transition_incarnation.executable_argv_fingerprint()
        ));
        assert_exec_transition_invalidates_phase_two_evidence(transition_process);
        assert!(matches!(
            second_snapshot.validated_ancestry(&process_identity, ParentWalkDepth::new(1)),
            AncestryLookup::Observed(ValidatedAncestry {
                terminal: AncestryTerminal::ParentEvidenceUnavailable { .. },
                ..
            })
        ));
        assert!(cache.remembers_unclassified_candidate(&process_identity));
        assert_eq!(
            cache.unclassified_cargo_candidates[&process_identity].candidate,
            CargoProcessCandidate::Cargo
        );

        let recovered_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(changed_process)],
        );
        let recovered_process =
            &recovered_snapshot.strongly_identified_processes()[&process_identity];
        assert!(matches!(
            recovered_process.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::Unchanged,
                ..
            }
        ));
        assert!(matches!(
            recovered_process.executable(),
            ProcessFieldObservation::Observed(executable)
                if executable == &PathBuf::from("/usr/bin/cargo")
        ));
        assert!(matches!(
            recovered_process.argv(),
            ProcessFieldObservation::Observed(argv)
                if argv == &vec![OsString::from("cargo"), OsString::from("check")]
        ));
        assert!(matches!(
            recovered_process.cwd(),
            ProcessFieldObservation::Observed(cwd) if cwd == &PathBuf::from("/workspace")
        ));
        assert!(matches!(
            recovered_process.parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::Root { .. })
        ));
        assert!(cache.remembers_unclassified_candidate(&process_identity));
    }

    #[test]
    fn changed_candidate_replaces_cached_candidate_as_unclassified() {
        let process_identity = identity(18, 180);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );
        assert!(cache.remembers_unclassified_candidate(&process_identity));

        let mut changed_process = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        changed_process.executable =
            ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/rustc"));
        changed_process.argv = ProcessFieldObservation::Observed(vec![OsString::from("rustc")]);
        let transition_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(changed_process)],
        );
        let transition_process =
            &transition_snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            transition_process.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                ..
            }
        ));
        assert_exec_transition_invalidates_phase_two_evidence(transition_process);
        assert_eq!(
            cache.unclassified_cargo_candidates[&process_identity].candidate,
            CargoProcessCandidate::Compiler
        );
        assert!(matches!(
            (
                &cache.unclassified_cargo_candidates[&process_identity].incarnation,
                transition_process.incarnation_evidence(),
            ),
            (
                cached_incarnation,
                ProcessIncarnationEvidence::Strong {
                    incarnation: transition_incarnation,
                    ..
                }
            ) if cached_incarnation == transition_incarnation
        ));
    }

    #[test]
    fn changed_non_candidate_removes_the_prior_candidate() {
        let process_identity = identity(40, 400);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );
        assert!(cache.remembers_unclassified_candidate(&process_identity));

        let mut changed_process = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        changed_process.executable =
            ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/python3"));
        changed_process.argv = ProcessFieldObservation::Observed(vec![OsString::from("python3")]);
        let transition_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(changed_process)],
        );
        let transition_process =
            &transition_snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            transition_process.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                ..
            }
        ));
        assert_exec_transition_invalidates_phase_two_evidence(transition_process);
        assert!(!cache.remembers_unclassified_candidate(&process_identity));
    }

    #[test]
    fn repeated_exec_field_unavailability_never_reports_unchanged() {
        let process_identity = identity(29, 290);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );
        let unavailable_process = IdentityBoundProcessObservation {
            identity:                process_identity.clone(),
            creation_order_evidence: ProcessCreationOrderEvidence::for_test(0),
            executable:              ProcessFieldObservation::Unavailable(
                ProcessFieldUnavailable::PlatformDidNotReport,
            ),
            argv:                    ProcessFieldObservation::Unavailable(
                ProcessFieldUnavailable::PlatformLookupFailed,
            ),
            cwd:                     ProcessFieldObservation::Observed(PathBuf::from(
                "/wrong-if-admitted",
            )),
            parent:                  ProcessFieldObservation::Observed(ReportedParent::Root),
        };

        for _ in 0..2 {
            let unavailable_snapshot = snapshot(
                &mut cache,
                vec![ProcessSamplingOutcome::IdentityBound(
                    unavailable_process.clone(),
                )],
            );
            let process_record =
                &unavailable_snapshot.strongly_identified_processes()[&process_identity];
            assert!(matches!(
                process_record.incarnation_evidence(),
                ProcessIncarnationEvidence::Insufficient(_)
            ));
            assert!(matches!(
                process_record.cwd(),
                ProcessFieldObservation::Invalidated(
                    ProcessFieldInvalidation::ExecIncarnationEvidenceInsufficient
                )
            ));
            assert!(!cache.remembers_unclassified_candidate(&process_identity));
            assert!(cache.remembers_incarnation(&process_identity));
        }
    }

    #[test]
    fn observed_exec_fields_recover_after_unavailability() {
        let process_identity = identity(30, 300);
        let mut cache = ProcessIncarnationCache::default();
        let stable_process = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(
                stable_process.clone(),
            )],
        );
        let mut unavailable_process = stable_process.clone();
        unavailable_process.argv =
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformDidNotReport);
        snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(unavailable_process)],
        );

        let recovered_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(stable_process)],
        );
        let recovered_process =
            &recovered_snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            recovered_process.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::Unchanged,
                ..
            }
        ));
        assert!(matches!(
            recovered_process.cwd(),
            ProcessFieldObservation::Observed(cwd) if cwd == &PathBuf::from("/workspace")
        ));
    }

    #[test]
    fn exec_observed_after_unavailable_fields_is_a_change() {
        let process_identity = identity(31, 310);
        let mut cache = ProcessIncarnationCache::default();
        let first_process = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(first_process.clone())],
        );
        let mut unavailable_process = first_process;
        unavailable_process.executable =
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformDidNotReport);
        snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(unavailable_process)],
        );
        let mut replacement_exec = observed_process(
            process_identity.clone(),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        replacement_exec.executable =
            ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/rustc"));
        replacement_exec.argv = ProcessFieldObservation::Observed(vec![OsString::from("rustc")]);

        let transition_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(
                replacement_exec.clone(),
            )],
        );
        let transition_process =
            &transition_snapshot.strongly_identified_processes()[&process_identity];
        assert!(matches!(
            transition_process.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::ExecutableOrArgumentsChanged { .. },
                ..
            }
        ));
        assert_exec_transition_invalidates_phase_two_evidence(transition_process);

        let recovered_snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::IdentityBound(replacement_exec)],
        );
        assert!(matches!(
            recovered_snapshot.strongly_identified_processes()[&process_identity]
                .incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::Unchanged,
                ..
            }
        ));
    }

    #[test]
    fn depth_cap_stops_before_unvalidated_ancestor() {
        let child = identity(9, 110);
        let parent = identity(10, 100);
        let root = identity(11, 90);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![
                strong_process(
                    child.clone(),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(parent.clone())),
                ),
                strong_process(
                    parent.clone(),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(root.clone())),
                ),
                strong_process(
                    root,
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                ),
            ],
        );
        let ancestry = snapshot.validated_ancestry(&child, ParentWalkDepth::new(1));

        assert!(matches!(
            &ancestry,
            AncestryLookup::Observed(ancestry)
                if ancestry.edges().len() == 1
                    && ancestry.edges()[0].parent() == &parent
                    && ancestry.edges()[0].child() == &child
                    && matches!(
                        ancestry.terminal(),
                        AncestryTerminal::DepthCapped { current_identity } if current_identity == &parent
                    )
        ));
    }

    #[test]
    fn equal_creation_values_leave_parent_order_unproven() {
        let child = identity(12, 120);
        let parent = identity(13, 120);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![
                strong_process(
                    child.clone(),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(parent.clone())),
                ),
                strong_process(
                    parent.clone(),
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                ),
            ],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(
                ParentageValidationOutcome::CreationOrderUnavailable {
                    edge,
                    unavailable: ProcessCreationOrderUnavailable::EqualMonotonicCreationValue,
                }
            ) if edge.parent() == &parent && edge.child() == &child
        ));
        assert!(matches!(
            snapshot.validated_ancestry(&child, ParentWalkDepth::new(1)),
            AncestryLookup::Observed(ValidatedAncestry {
                terminal: AncestryTerminal::CreationOrderUnavailable {
                    edge,
                    unavailable: ProcessCreationOrderUnavailable::EqualMonotonicCreationValue,
                },
                ..
            }) if edge.parent() == &parent && edge.child() == &child
        ));
    }

    #[test]
    fn self_parent_edge_is_rejected() {
        let process_identity = identity(22, 220);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Identified(
                    process_identity.clone(),
                )),
            )],
        );

        assert!(
            snapshot
                .strongly_identified_processes()
                .get(&process_identity)
                .is_some_and(|process| {
                    matches!(
                        process.parentage_validation_outcome(),
                        ProcessFieldObservation::Observed(
                            ParentageValidationOutcome::RejectedEdge {
                                rejection: ParentEdgeRejection::SelfParent,
                                ..
                            }
                        )
                    )
                })
        );
    }

    #[test]
    fn omitted_but_still_current_process_retains_cached_incarnation() {
        let process_identity = identity(14, 140);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );
        assert!(cache.remembers_unclassified_candidate(&process_identity));

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            updated_process_evidence(BTreeMap::from([(
                process_identity.pid(),
                ObservedProcessIdentity::Strong(process_identity.clone()),
            )])),
        );

        assert!(cache.remembers_unclassified_candidate(&process_identity));
        assert!(cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn omitted_and_gone_process_evicts_cached_incarnation() {
        let process_identity = identity(41, 410);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            updated_process_evidence(BTreeMap::from([(
                process_identity.pid(),
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup {
                        pid: process_identity.pid(),
                    },
                ),
            )])),
        );

        assert!(!cache.remembers_unclassified_candidate(&process_identity));
        assert!(!cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn omitted_and_replaced_process_evicts_cached_incarnation() {
        let process_identity = identity(42, 420);
        let replacement_identity = identity(42, 421);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            updated_process_evidence(BTreeMap::from([(
                process_identity.pid(),
                ObservedProcessIdentity::Strong(replacement_identity),
            )])),
        );

        assert!(!cache.remembers_unclassified_candidate(&process_identity));
        assert!(!cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn omitted_with_insufficient_lookup_retains_cached_incarnation() {
        let process_identity = identity(43, 430);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            updated_process_evidence(BTreeMap::from([(
                process_identity.pid(),
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                        pid: process_identity.pid(),
                    },
                ),
            )])),
        );

        assert!(cache.remembers_unclassified_candidate(&process_identity));
        assert!(cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn same_pid_replacement_evicts_the_prior_lifetime_cache() {
        let prior_identity = identity(32, 320);
        let replacement_identity = identity(32, 321);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                prior_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            updated_process_evidence(BTreeMap::from([(
                replacement_identity.pid(),
                ObservedProcessIdentity::Strong(replacement_identity),
            )])),
        );

        assert!(!cache.remembers_unclassified_candidate(&prior_identity));
        assert!(!cache.remembers_incarnation(&prior_identity));
    }

    #[test]
    fn refresh_with_no_updated_processes_retains_the_prior_lifetime_cache() {
        let process_identity = identity(33, 330);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            FullProcessRefreshEvidence::NoProcessesUpdated,
        );

        assert!(cache.remembers_unclassified_candidate(&process_identity));
        assert!(cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn transient_insufficient_identity_retains_the_live_pid_cache() {
        let process_identity = identity(34, 340);
        let mut cache = ProcessIncarnationCache::default();
        snapshot(
            &mut cache,
            vec![strong_process(
                process_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )],
        );

        snapshot_with_refresh_evidence(
            &mut cache,
            Vec::new(),
            updated_process_evidence(BTreeMap::from([(
                process_identity.pid(),
                ObservedProcessIdentity::Insufficient(
                    InsufficientProcessIdentity::PlatformIdentityLookupFailed {
                        pid: process_identity.pid(),
                    },
                ),
            )])),
        );

        assert!(cache.remembers_unclassified_candidate(&process_identity));
        assert!(cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn insufficient_identities_remain_diagnostic_only() {
        let process_observation = InsufficientIdentityProcessRecord {
            identity:   InsufficientProcessIdentity::PlatformCreationTokenUnavailable { pid: 15 },
            executable: ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessIdentityNotStableDuringSampling,
            ),
            argv:       ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessIdentityNotStableDuringSampling,
            ),
            cwd:        ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessIdentityNotStableDuringSampling,
            ),
            parent:     ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessIdentityNotStableDuringSampling,
            ),
        };
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![ProcessSamplingOutcome::InsufficientIdentity(
                process_observation,
            )],
        );

        assert!(snapshot.strongly_identified_processes().is_empty());
        assert_eq!(snapshot.insufficient_identity_processes().len(), 1);
    }

    #[test]
    fn identity_change_around_field_sample_never_creates_a_strong_record() {
        let prior_identity = identity(16, 160);
        let replacement_identity = identity(16, 161);
        let identity_before_fields = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(prior_identity.clone()),
            ProcessCreationOrderEvidence::for_test(160),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let process_field_sample = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/workspace")),
        };
        let identity_after_fields = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(replacement_identity.clone()),
            ProcessCreationOrderEvidence::for_test(161),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            identity_before_fields,
            ProcessFieldSourceObservation::repeated_fresh_system_samples(
                process_field_sample.clone(),
                process_field_sample,
            ),
            identity_after_fields,
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(&mut cache, vec![process_sampling_outcome]);

        assert!(snapshot.strongly_identified_processes().is_empty());
        assert!(matches!(
            snapshot.identity_binding_invalidations(),
            [ProcessIdentityBindingInvalidation::PlatformIdentityChanged {
                before: ObservedProcessIdentity::Strong(before),
                after: ObservedProcessIdentity::Strong(after),
            }] if before == &prior_identity && after == &replacement_identity
        ));
    }

    #[test]
    fn field_transition_between_observations_invalidates_then_stable_fields_recover() {
        let process_identity = identity(45, 450);
        let platform_observation = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test(450),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let initial = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/workspace")),
        };
        let repeated = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/rustc")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("rustc")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/workspace")),
        };
        let transition_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            platform_observation.clone(),
            ProcessFieldSourceObservation::repeated_fresh_system_samples(initial, repeated.clone()),
            platform_observation.clone(),
        );
        let mut cache = ProcessIncarnationCache::default();
        let transition_snapshot = snapshot(&mut cache, vec![transition_outcome]);
        let transition_record =
            &transition_snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            transition_record.executable(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
            )
        ));
        assert!(matches!(
            transition_record.argv(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
            )
        ));
        assert!(matches!(
            transition_record.cwd(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
            )
        ));
        assert!(matches!(
            transition_record.parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::Root { child })
                if child == &process_identity
        ));
        assert!(!cache.remembers_incarnation(&process_identity));

        let recovered_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            platform_observation.clone(),
            ProcessFieldSourceObservation::repeated_fresh_system_samples(
                repeated.clone(),
                repeated,
            ),
            platform_observation,
        );
        let recovered_snapshot = snapshot(&mut cache, vec![recovered_outcome]);
        let recovered_record =
            &recovered_snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            recovered_record.incarnation_evidence(),
            ProcessIncarnationEvidence::Strong {
                incarnation_state: ProcessIncarnationState::NewlyObserved,
                ..
            }
        ));
        assert!(matches!(
            recovered_record.executable(),
            ProcessFieldObservation::Observed(executable)
                if executable == &PathBuf::from("/usr/bin/rustc")
        ));
        assert!(matches!(
            recovered_record.argv(),
            ProcessFieldObservation::Observed(argv)
                if argv == &vec![OsString::from("rustc")]
        ));
    }

    #[test]
    fn cwd_transition_invalidates_process_fields_but_validates_stable_parent() {
        let process_identity = identity(49, 490);
        let platform_observation = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test(490),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let initial = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/workspace")),
        };
        let repeated = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/moved-workspace")),
        };
        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            platform_observation.clone(),
            ProcessFieldSourceObservation::repeated_fresh_system_samples(initial, repeated),
            platform_observation,
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(&mut cache, vec![process_sampling_outcome]);
        let process_record = &snapshot.strongly_identified_processes()[&process_identity];

        for field in [process_record.executable(), process_record.cwd()] {
            assert!(matches!(
                field,
                ProcessFieldObservation::Invalidated(
                    ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
                )
            ));
        }
        assert!(matches!(
            process_record.argv(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
            )
        ));
        assert!(matches!(
            process_record.parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::Root { child })
                if child == &process_identity
        ));
        assert!(!cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn changing_parent_is_invalidated_independently_of_unstable_process_fields() {
        let process_identity = identity(54, 540);
        let parent_before = identity(55, 550);
        let parent_after = identity(56, 560);
        let identity_before_fields = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test(540),
            ProcessFieldObservation::Observed(ReportedParent::Identified(parent_before)),
        );
        let identity_after_fields = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test(540),
            ProcessFieldObservation::Observed(ReportedParent::Identified(parent_after)),
        );
        let initial = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/workspace")),
        };
        let repeated = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/moved-workspace")),
        };
        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            identity_before_fields,
            ProcessFieldSourceObservation::repeated_fresh_system_samples(initial, repeated),
            identity_after_fields,
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(&mut cache, vec![process_sampling_outcome]);
        let process_record = &snapshot.strongly_identified_processes()[&process_identity];

        for field in [process_record.executable(), process_record.cwd()] {
            assert!(matches!(
                field,
                ProcessFieldObservation::Invalidated(
                    ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
                )
            ));
        }
        assert!(matches!(
            process_record.argv(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessFieldsDifferedDuringSampling
            )
        ));
        assert!(matches!(
            process_record.parentage_validation_outcome(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ParentIdentityChangedDuringSampling
            )
        ));
        assert!(!cache.remembers_incarnation(&process_identity));
    }

    #[test]
    fn unproven_field_stability_invalidates_process_fields_but_validates_stable_parent() {
        let process_identity = identity(46, 460);
        let platform_observation = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(process_identity.clone()),
            ProcessCreationOrderEvidence::for_test(460),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            platform_observation.clone(),
            ProcessFieldSourceObservation::fresh_system_stability_unproven(),
            platform_observation,
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(&mut cache, vec![process_sampling_outcome]);
        let process_record = &snapshot.strongly_identified_processes()[&process_identity];

        assert!(matches!(
            process_record.executable(),
            ProcessFieldObservation::Invalidated(
                ProcessFieldInvalidation::ProcessFieldStabilityUnproven
            )
        ));
        assert!(matches!(
            process_record.parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::Root { child })
                if child == &process_identity
        ));
    }

    #[test]
    fn unproven_long_lived_field_source_is_rejected() {
        let current_identity = identity(44, 440);
        let current_platform_observation = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(current_identity.clone()),
            ProcessCreationOrderEvidence::for_test(440),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let stale_process_field_sample = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/stale/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("stale-cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/stale/workspace")),
        };

        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            current_platform_observation.clone(),
            ProcessFieldSourceObservation::unproven_long_lived_system_sample(
                stale_process_field_sample,
            ),
            current_platform_observation,
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot_with_refresh_evidence(
            &mut cache,
            vec![process_sampling_outcome],
            updated_process_evidence(BTreeMap::from([(
                current_identity.pid(),
                ObservedProcessIdentity::Strong(current_identity.clone()),
            )])),
        );

        assert!(snapshot.strongly_identified_processes().is_empty());
        assert!(matches!(
            snapshot.identity_binding_invalidations(),
            [ProcessIdentityBindingInvalidation::ProcessFieldSourceLifetimeUnproven {
                current,
            }] if current == &current_identity
        ));
    }

    #[test]
    fn cached_source_record_from_prior_pid_occupant_is_rejected() {
        let source_identity = identity(35, 350);
        let current_identity = identity(35, 351);
        let current_platform_observation = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Strong(current_identity.clone()),
            ProcessCreationOrderEvidence::for_test(351),
            ProcessFieldObservation::Observed(ReportedParent::Root),
        );
        let old_process_field_sample = ProcessFieldSample {
            executable: ProcessFieldObservation::Observed(PathBuf::from("/old/cargo")),
            argv:       ProcessFieldObservation::Observed(vec![OsString::from("old-cargo")]),
            cwd:        ProcessFieldObservation::Observed(PathBuf::from("/old/workspace")),
        };

        let process_sampling_outcome = ProcessSamplingOutcome::bind_fields_to_identity(
            current_platform_observation.clone(),
            ProcessFieldSourceObservation::strong_identity(
                source_identity.clone(),
                old_process_field_sample,
            ),
            current_platform_observation,
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot_with_refresh_evidence(
            &mut cache,
            vec![process_sampling_outcome],
            updated_process_evidence(BTreeMap::from([(
                current_identity.pid(),
                ObservedProcessIdentity::Strong(current_identity.clone()),
            )])),
        );

        assert!(snapshot.strongly_identified_processes().is_empty());
        assert!(matches!(
            snapshot.identity_binding_invalidations(),
            [ProcessIdentityBindingInvalidation::ProcessFieldSourceIdentityMismatch {
                current,
                source,
            }] if current == &current_identity && source == &source_identity
        ));
    }

    #[test]
    fn exact_target_disappearance_is_reported_and_not_admitted() {
        let requested_identity = identity(19, 190);
        let missing_process_observation = PlatformProcessObservation::for_test(
            ObservedProcessIdentity::Insufficient(
                InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup {
                    pid: requested_identity.pid(),
                },
            ),
            ProcessCreationOrderEvidence::unavailable_for_test(),
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::ProcessExited),
        );
        let targeted_process_sampling_result = TargetedProcessSamplingResult::classify(
            &requested_identity,
            unavailable_target_presence(
                missing_process_observation.clone(),
                missing_process_observation,
                ProcessFieldUnavailable::ProcessExited,
            ),
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = targeted_snapshot(
            &mut cache,
            requested_identity.clone(),
            targeted_process_sampling_result,
        );

        assert!(snapshot.strongly_identified_processes().is_empty());
        assert!(matches!(
            snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if matches!(outcomes.get(&requested_identity), Some(TargetedProcessObservation::Gone))
        ));
    }

    #[test]
    fn unavailable_fields_report_replacement_for_the_before_identity() {
        let before_identity = identity(50, 500);
        let after_identity = identity(50, 501);

        let result = TargetedProcessSamplingResult::classify(
            &before_identity,
            unavailable_target_presence(
                platform_observation(&before_identity),
                platform_observation(&after_identity),
                ProcessFieldUnavailable::PlatformLookupFailed,
            ),
        );

        assert_eq!(
            result.observation,
            TargetedProcessObservation::Replaced {
                replacement: after_identity,
            }
        );
        assert!(matches!(
            result.admission,
            TargetedSampleAdmission::Excluded
        ));
    }

    #[test]
    fn unavailable_fields_invalidate_binding_for_the_after_identity() {
        let before_identity = identity(51, 510);
        let after_identity = identity(51, 511);

        let result = TargetedProcessSamplingResult::classify(
            &after_identity,
            unavailable_target_presence(
                platform_observation(&before_identity),
                platform_observation(&after_identity),
                ProcessFieldUnavailable::PlatformLookupFailed,
            ),
        );

        assert!(matches!(
            result.observation,
            TargetedProcessObservation::IdentityBindingInvalidated(
                ProcessIdentityBindingInvalidation::PlatformIdentityChanged {
                    before: ObservedProcessIdentity::Strong(before),
                    after: ObservedProcessIdentity::Strong(after),
                }
            ) if before == before_identity && after == after_identity
        ));
        assert!(matches!(
            result.admission,
            TargetedSampleAdmission::Excluded
        ));
    }

    #[test]
    fn unavailable_fields_report_after_identity_for_an_unrelated_request() {
        let before_identity = identity(52, 520);
        let after_identity = identity(52, 521);
        let unrelated_identity = identity(52, 522);

        let result = TargetedProcessSamplingResult::classify(
            &unrelated_identity,
            unavailable_target_presence(
                platform_observation(&before_identity),
                platform_observation(&after_identity),
                ProcessFieldUnavailable::PlatformLookupFailed,
            ),
        );

        assert_eq!(
            result.observation,
            TargetedProcessObservation::Replaced {
                replacement: after_identity,
            }
        );
        assert!(matches!(
            result.admission,
            TargetedSampleAdmission::Excluded
        ));
    }

    #[test]
    fn stable_identity_with_unavailable_fields_reports_field_failure() {
        let requested_identity = identity(53, 530);

        let result = TargetedProcessSamplingResult::classify(
            &requested_identity,
            unavailable_target_presence(
                platform_observation(&requested_identity),
                platform_observation(&requested_identity),
                ProcessFieldUnavailable::PlatformLookupFailed,
            ),
        );

        assert_eq!(
            result.observation,
            TargetedProcessObservation::FieldsUnavailable(
                ProcessFieldUnavailable::PlatformLookupFailed
            )
        );
        assert!(matches!(
            result.admission,
            TargetedSampleAdmission::Excluded
        ));
    }

    #[test]
    fn exact_target_replacement_is_reported_and_not_admitted() {
        let requested_identity = identity(20, 200);
        let replacement_identity = identity(20, 201);
        let targeted_process_sampling_result = TargetedProcessSamplingResult::classify(
            &requested_identity,
            TargetedProcessPresence::Sampled(strong_process(
                replacement_identity.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Root),
            )),
        );
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = targeted_snapshot(
            &mut cache,
            requested_identity.clone(),
            targeted_process_sampling_result,
        );

        assert!(snapshot.strongly_identified_processes().is_empty());
        assert!(matches!(
            snapshot.targeted_process_observations(),
            TargetedProcessObservations::Outcomes(outcomes)
                if matches!(
                    outcomes.get(&requested_identity),
                    Some(TargetedProcessObservation::Replaced { replacement })
                        if replacement == &replacement_identity
                )
        ));
    }

    #[test]
    fn current_captured_parent_identity_is_accepted() {
        let parent = identity(23, 230);
        let child = identity(24, 240);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![
                ProcessSamplingOutcome::IdentityBound(observed_process_with_creation_order(
                    child.clone(),
                    ProcessCreationOrderEvidence::for_test(240),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(parent.clone())),
                )),
                ProcessSamplingOutcome::IdentityBound(observed_process_with_creation_order(
                    parent.clone(),
                    ProcessCreationOrderEvidence::for_test(230),
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                )),
            ],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::ValidatedEdge(edge))
                if edge.parent() == &parent && edge.child() == &child
        ));
    }

    #[test]
    fn captured_parent_replaced_at_same_pid_is_rejected() {
        let captured_parent = identity(25, 250);
        let child = identity(26, 260);
        let replacement_parent = identity(25, 270);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![
                strong_process(
                    child.clone(),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(
                        captured_parent.clone(),
                    )),
                ),
                strong_process(
                    replacement_parent.clone(),
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                ),
            ],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::RejectedEdge {
                edge,
                rejection: ParentEdgeRejection::IdentityReplaced { current },
            }) if edge.parent() == &captured_parent
                && edge.child() == &child
                && current == &replacement_parent
        ));
    }

    #[test]
    fn unavailable_identified_parent_retains_both_strong_endpoints() {
        let parent = identity(36, 360);
        let child = identity(37, 370);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![strong_process(
                child.clone(),
                ProcessFieldObservation::Observed(ReportedParent::Identified(parent.clone())),
            )],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(
                ParentageValidationOutcome::UnavailableIdentifiedParent { edge }
            ) if edge.parent() == &parent && edge.child() == &child
        ));
        assert!(matches!(
            snapshot.validated_ancestry(&child, ParentWalkDepth::new(1)),
            AncestryLookup::Observed(ValidatedAncestry {
                terminal: AncestryTerminal::UnavailableIdentifiedParent { edge },
                ..
            }) if edge.parent() == &parent && edge.child() == &child
        ));
    }

    #[test]
    fn parent_created_after_child_is_rejected_as_recycled_pid_evidence() {
        let child = identity(27, 270);
        let later_parent = identity(28, 280);
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![
                ProcessSamplingOutcome::IdentityBound(observed_process_with_creation_order(
                    child.clone(),
                    ProcessCreationOrderEvidence::for_test(270),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(
                        later_parent.clone(),
                    )),
                )),
                ProcessSamplingOutcome::IdentityBound(observed_process_with_creation_order(
                    later_parent,
                    ProcessCreationOrderEvidence::for_test(280),
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                )),
            ],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::RejectedEdge {
                rejection: ParentEdgeRejection::CreatedAfterChild,
                ..
            })
        ));
    }

    #[test]
    fn unavailable_creation_order_does_not_validate_or_reject_parent_edge() {
        let child = identity(47, 470);
        let parent = identity(48, 480);
        let unavailable =
            ProcessCreationOrderUnavailable::PlatformDoesNotExposeMonotonicCreationOrder;
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![
                ProcessSamplingOutcome::IdentityBound(observed_process_with_creation_order(
                    child.clone(),
                    ProcessCreationOrderEvidence::unavailable_for_test(),
                    ProcessFieldObservation::Observed(ReportedParent::Identified(parent.clone())),
                )),
                ProcessSamplingOutcome::IdentityBound(observed_process_with_creation_order(
                    parent.clone(),
                    ProcessCreationOrderEvidence::unavailable_for_test(),
                    ProcessFieldObservation::Observed(ReportedParent::Root),
                )),
            ],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(
                ParentageValidationOutcome::CreationOrderUnavailable {
                    edge,
                    unavailable: observed,
                }
            ) if edge.parent() == &parent && edge.child() == &child && observed == &unavailable
        ));
        assert!(matches!(
            snapshot.validated_ancestry(&child, ParentWalkDepth::new(1)),
            AncestryLookup::Observed(ValidatedAncestry {
                terminal: AncestryTerminal::CreationOrderUnavailable {
                    edge,
                    unavailable: observed,
                },
                ..
            }) if edge.parent() == &parent && edge.child() == &child && observed == unavailable
        ));
    }

    #[test]
    fn unavailable_parent_identity_is_not_promoted_to_root() {
        let child = identity(17, 170);
        let parent_identity =
            InsufficientProcessIdentity::ProcessExitedBeforeIdentityLookup { pid: 999 };
        let mut cache = ProcessIncarnationCache::default();
        let snapshot = snapshot(
            &mut cache,
            vec![strong_process(
                child.clone(),
                ProcessFieldObservation::Observed(ReportedParent::IdentityUnavailable(
                    parent_identity.clone(),
                )),
            )],
        );

        assert!(matches!(
            snapshot.strongly_identified_processes()[&child].parentage_validation_outcome(),
            ProcessFieldObservation::Observed(ParentageValidationOutcome::UnavailableParent {
                parent_identity: unavailable,
                ..
            }) if unavailable == &parent_identity
        ));
    }

    #[test]
    fn cargo_candidate_parser_classifies_compiler_wrappers() {
        let candidate = CargoProcessCandidate::parse(
            &ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/sccache")),
            &ProcessFieldObservation::Observed(vec![OsString::from("rustc")]),
        );

        assert_eq!(candidate, CargoProcessCandidate::Wrapper);
    }

    #[test]
    fn cargo_candidate_parser_accepts_unix_and_windows_executable_names() {
        let cases = [
            ("/usr/bin/cargo", CargoProcessCandidate::Cargo),
            ("cargo.exe", CargoProcessCandidate::Cargo),
            ("/usr/bin/rustc", CargoProcessCandidate::Compiler),
            ("rustc.exe", CargoProcessCandidate::Compiler),
            ("/usr/bin/sccache", CargoProcessCandidate::Wrapper),
            ("sccache.exe", CargoProcessCandidate::Wrapper),
            ("/usr/bin/clippy-driver", CargoProcessCandidate::Wrapper),
            ("clippy-driver.exe", CargoProcessCandidate::Wrapper),
        ];

        for (executable, expected) in cases {
            assert_eq!(
                CargoProcessCandidate::parse(
                    &ProcessFieldObservation::Observed(PathBuf::from(executable)),
                    &ProcessFieldObservation::Observed(Vec::new()),
                ),
                expected
            );
        }
    }

    #[test]
    fn wrapper_arguments_accept_only_exact_rustc_spellings() {
        for argument in ["/usr/bin/rustc", "rustc.exe"] {
            assert_eq!(
                CargoProcessCandidate::parse(
                    &ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/wrapper")),
                    &ProcessFieldObservation::Observed(vec![OsString::from(argument)]),
                ),
                CargoProcessCandidate::Wrapper
            );
        }
        for unrelated in ["rustc.com", "rustc.exe.bak", "cargo.exe"] {
            assert_eq!(
                CargoProcessCandidate::parse(
                    &ProcessFieldObservation::Observed(PathBuf::from("/usr/bin/wrapper")),
                    &ProcessFieldObservation::Observed(vec![OsString::from(unrelated)]),
                ),
                CargoProcessCandidate::NotCandidate
            );
        }
    }
}
