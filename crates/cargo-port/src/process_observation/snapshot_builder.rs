//! Test-only assembly of immutable process observation snapshots.
//!
//! Classification reads a [`ProcessObservationSnapshot`] and nothing else, so a
//! classification test states the processes it wants observed rather than
//! driving a host refresh. Every field a test does not state is observed with a
//! stated default, which keeps a test's own text down to the evidence it is
//! about.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use super::identity::ProcessIdentity;
use super::identity::ProcessIncarnation;
use super::snapshot::BuildCandidateRole;
use super::snapshot::ParentageValidationOutcome;
use super::snapshot::ProcessFieldObservation;
use super::snapshot::ProcessFieldUnavailable;
use super::snapshot::ProcessIncarnationEvidence;
use super::snapshot::ProcessIncarnationState;
use super::snapshot::ProcessObservationSnapshot;
use super::snapshot::ProcessSnapshotRecord;
use super::snapshot::ValidatedParentEdge;

/// Whether one observed process is a Cargo, compiler, or wrapper candidate, and
/// which role the observer's cache recorded for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ObservedCandidateRole {
    Cargo,
    Compiler,
    Wrapper,
    NotCandidate,
}

impl ObservedCandidateRole {
    const fn build_candidate_role(self) -> Option<BuildCandidateRole> {
        match self {
            Self::Cargo => Some(BuildCandidateRole::Cargo),
            Self::Compiler => Some(BuildCandidateRole::Compiler),
            Self::Wrapper => Some(BuildCandidateRole::Wrapper),
            Self::NotCandidate => None,
        }
    }
}

/// Where one observed process sits in the validated parent chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ObservedParentage {
    /// The process has no observed parent inside the snapshot.
    Root,
    /// The process's parent edge was validated and names this identity.
    ValidatedChildOf(ProcessIdentity),
    /// The platform reported no usable parentage for this process.
    Unobservable,
}

/// One process a test wants present in the assembled snapshot.
#[derive(Clone, Debug)]
pub(crate) struct ObservedProcess {
    identity:       ProcessIdentity,
    incarnation:    ProcessIncarnation,
    executable:     ProcessFieldObservation<PathBuf>,
    argv:           ProcessFieldObservation<Vec<OsString>>,
    cwd:            ProcessFieldObservation<PathBuf>,
    parentage:      ObservedParentage,
    candidate_role: ObservedCandidateRole,
}

impl ObservedProcess {
    /// Observe one process by PID, creation token, executable, and arguments.
    /// The exec fingerprint is derived from `fingerprint_source`, so two
    /// observations of the same PID with different sources are two
    /// incarnations — which is what a same-PID exec looks like to
    /// classification.
    pub(crate) fn new(
        pid: u32,
        creation_token: u64,
        fingerprint_source: &str,
        executable: &str,
        argv: &[&str],
    ) -> Self {
        let identity = ProcessIdentity::for_test(pid, creation_token);
        Self {
            incarnation: ProcessIncarnation::for_test(identity.clone(), fingerprint_source),
            identity,
            executable: ProcessFieldObservation::Observed(PathBuf::from(executable)),
            argv: ProcessFieldObservation::Observed(
                argv.iter().map(OsString::from).collect::<Vec<_>>(),
            ),
            cwd: ProcessFieldObservation::Observed(PathBuf::from("/")),
            parentage: ObservedParentage::Root,
            candidate_role: ObservedCandidateRole::NotCandidate,
        }
    }

    /// The identity this process was observed under.
    pub(crate) const fn identity(&self) -> &ProcessIdentity { &self.identity }

    /// The exec-sensitive incarnation classification keys this process by.
    pub(crate) const fn incarnation(&self) -> &ProcessIncarnation { &self.incarnation }

    /// Observe the process's working directory as this path.
    #[must_use]
    pub(crate) fn with_cwd(mut self, cwd: &Path) -> Self {
        self.cwd = ProcessFieldObservation::Observed(cwd.to_path_buf());
        self
    }

    /// Leave the process's working directory unreadable for this cycle.
    #[must_use]
    pub(crate) fn with_unreadable_cwd(mut self) -> Self {
        self.cwd =
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformLookupFailed);
        self
    }

    /// Leave the process's arguments unreadable for this cycle.
    #[must_use]
    pub(crate) fn with_unreadable_argv(mut self) -> Self {
        self.argv =
            ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformLookupFailed);
        self
    }

    /// Place the process under a validated parent edge.
    #[must_use]
    pub(crate) fn with_validated_parent(mut self, parent: &ProcessIdentity) -> Self {
        self.parentage = ObservedParentage::ValidatedChildOf(parent.clone());
        self
    }

    /// Leave the process's parentage unobservable for this cycle.
    #[must_use]
    pub(crate) fn with_unobservable_parentage(mut self) -> Self {
        self.parentage = ObservedParentage::Unobservable;
        self
    }

    /// Record the candidate role the observer's cache holds for this process.
    #[must_use]
    pub(crate) const fn with_candidate_role(mut self, role: ObservedCandidateRole) -> Self {
        self.candidate_role = role;
        self
    }

    fn parentage_observation(&self) -> ProcessFieldObservation<ParentageValidationOutcome> {
        match &self.parentage {
            ObservedParentage::Root => {
                ProcessFieldObservation::Observed(ParentageValidationOutcome::Root {
                    child: self.identity.clone(),
                })
            },
            ObservedParentage::ValidatedChildOf(parent) => {
                ProcessFieldObservation::Observed(ParentageValidationOutcome::ValidatedEdge(
                    ValidatedParentEdge::for_test(parent.clone(), self.identity.clone()),
                ))
            },
            ObservedParentage::Unobservable => {
                ProcessFieldObservation::Unavailable(ProcessFieldUnavailable::PlatformDidNotReport)
            },
        }
    }

    fn snapshot_record(&self) -> ProcessSnapshotRecord {
        ProcessSnapshotRecord::for_test(
            self.identity.clone(),
            ProcessIncarnationEvidence::Strong {
                incarnation:       self.incarnation.clone(),
                incarnation_state: ProcessIncarnationState::NewlyObserved,
            },
            self.executable.clone(),
            self.argv.clone(),
            self.cwd.clone(),
            self.parentage_observation(),
        )
    }
}

/// Assemble one immutable snapshot from stated process observations.
pub(crate) fn snapshot_of(observed_processes: &[ObservedProcess]) -> ProcessObservationSnapshot {
    snapshot_of_at(Instant::now(), observed_processes)
}

/// Assemble one immutable snapshot observed at a stated instant.
pub(crate) fn snapshot_of_at(
    observed_at: Instant,
    observed_processes: &[ObservedProcess],
) -> ProcessObservationSnapshot {
    let mut strongly_identified_processes = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    for observed_process in observed_processes {
        strongly_identified_processes.insert(
            observed_process.identity.clone(),
            observed_process.snapshot_record(),
        );
        if let Some(build_candidate_role) = observed_process.candidate_role.build_candidate_role() {
            candidates.insert(observed_process.incarnation.clone(), build_candidate_role);
        }
    }
    ProcessObservationSnapshot::from_parts_for_test(
        observed_at,
        strongly_identified_processes,
        super::snapshot::BuildCandidateIncarnations::for_test(candidates),
    )
}
