//! What one refresh cycle owes the compile monitor, and what classifying that
//! cycle produced.
//!
//! The demand travels from `App` to the dedicated process-refresh worker and
//! the execution travels back, so both are immutable and carry their own
//! monitor generation: a result that arrives after the scope moved on names the
//! generation it belonged to instead of being applied to the current one.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use super::classify::BuildClassification;
use super::scope::BuildScopeKey;
use super::session::OwnedRootEvidence;
use crate::project::CargoWorkspaceIndex;

/// Monotonic identity for one compile-monitor scope lifetime.
///
/// Enabling the monitor and every scope invalidation advance it, so a
/// generation names one (scope, enablement) pair and nothing else.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct CompileMonitorGeneration(u64);

impl CompileMonitorGeneration {
    pub(crate) const fn advance(&mut self) { self.0 = self.0.saturating_add(1); }
}

/// Whether an in-flight classification may still run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompileClassificationProgress {
    Proceed,
    Cancelled,
}

/// What asking a [`CompileClassificationCancellation`] to cancel achieved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompileClassificationCancellationOutcome {
    Cancelled,
    /// The capability belongs to a different monitor generation, so the
    /// in-flight cycle it guards was left running.
    BoundToAnotherGeneration,
}

/// The capability to stop one monitor generation's in-flight classification.
///
/// The generation is fixed when the capability is created and every
/// cancellation is checked against it, so holding one can never stop a cycle
/// that belongs to a later scope or generation.
#[derive(Clone, Debug)]
pub(crate) struct CompileClassificationCancellation {
    compile_monitor_generation: CompileMonitorGeneration,
    cancelled:                  Arc<AtomicBool>,
}

impl CompileClassificationCancellation {
    pub(crate) fn for_generation(compile_monitor_generation: CompileMonitorGeneration) -> Self {
        Self {
            compile_monitor_generation,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel the guarded cycle, but only when the caller names the generation
    /// this capability was created for.
    pub(crate) fn cancel(
        &self,
        compile_monitor_generation: CompileMonitorGeneration,
    ) -> CompileClassificationCancellationOutcome {
        if compile_monitor_generation == self.compile_monitor_generation {
            self.cancelled.store(true, Ordering::Release);
            CompileClassificationCancellationOutcome::Cancelled
        } else {
            CompileClassificationCancellationOutcome::BoundToAnotherGeneration
        }
    }

    pub(crate) fn progress(&self) -> CompileClassificationProgress {
        if self.cancelled.load(Ordering::Acquire) {
            CompileClassificationProgress::Cancelled
        } else {
            CompileClassificationProgress::Proceed
        }
    }
}

/// Whether one refresh cycle owes the compile monitor a classification, and the
/// immutable evidence a requested classification runs against.
///
/// The workspace index is an `Arc` clone taken when the request is built, so a
/// rebuild landing mid-flight leaves this cycle on the index its scope
/// revisions already name.
#[derive(Clone, Debug)]
pub(crate) enum CompileClassificationDemand {
    NotRequested,
    Requested {
        compile_monitor_generation: CompileMonitorGeneration,
        build_scope_key:            BuildScopeKey,
        cargo_workspace_index:      Arc<CargoWorkspaceIndex>,
        owned_root_evidence:        OwnedRootEvidence,
        cancellation:               CompileClassificationCancellation,
    },
}

/// Why a requested compile classification produced no classification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuildClassificationExecutionFailure {
    /// The workspace index the request captured names no workspace at all, so
    /// nothing observed could be attributed to a scope root. The index is
    /// rebuilt by replacement, so a request dispatched against an actionable
    /// scope can still reach the worker holding an emptied index.
    NoIndexedWorkspace,
}

/// One completed classification, together with the monitor generation, the
/// exact scope, and the owned-root evidence it was computed under.
///
/// A reader learns all three from the result itself instead of having to consult
/// the request that produced it; the sibling [`CompileClassificationExecution::
/// Cancelled`] already names its generation the same way. The owned-root
/// evidence travels with the result because re-reading the live evidence at
/// reconcile time would label this cycle's sessions with whichever owned run is
/// current then, not the one they were classified against.
#[derive(Debug, PartialEq)]
pub(crate) struct CompletedBuildClassification {
    compile_monitor_generation: CompileMonitorGeneration,
    build_scope_key:            BuildScopeKey,
    owned_root_evidence:        OwnedRootEvidence,
    classification:             Box<BuildClassification>,
}

impl CompletedBuildClassification {
    /// Stamp one classification with the generation, scope, and owned-root
    /// evidence it ran under.
    pub(crate) const fn new(
        compile_monitor_generation: CompileMonitorGeneration,
        build_scope_key: BuildScopeKey,
        owned_root_evidence: OwnedRootEvidence,
        classification: Box<BuildClassification>,
    ) -> Self {
        Self {
            compile_monitor_generation,
            build_scope_key,
            owned_root_evidence,
            classification,
        }
    }

    /// A completion carrying no classified session, for reconcile tests that
    /// exercise the generation gate rather than what the cycle observed.
    #[cfg(test)]
    pub(crate) fn empty_for_test(
        compile_monitor_generation: CompileMonitorGeneration,
        build_scope_key: BuildScopeKey,
        cycle_instant: std::time::Instant,
    ) -> Self {
        Self::new(
            compile_monitor_generation,
            build_scope_key,
            OwnedRootEvidence::NoLiveRoot,
            Box::new(BuildClassification::empty_for_test(cycle_instant)),
        )
    }

    /// The monitor generation this classification belongs to.
    pub(crate) const fn compile_monitor_generation(&self) -> CompileMonitorGeneration {
        self.compile_monitor_generation
    }

    /// The classification itself, for reporting what one cycle observed.
    pub(crate) const fn classification(&self) -> &BuildClassification { &self.classification }

    /// The scope, the owned-root evidence the cycle was classified against, and
    /// the classification itself, for the one consumer that narrows the
    /// host-wide result to that scope.
    pub(crate) fn into_scoped_classification(
        self,
    ) -> (BuildScopeKey, OwnedRootEvidence, Box<BuildClassification>) {
        (
            self.build_scope_key,
            self.owned_root_evidence,
            self.classification,
        )
    }
}

/// What classifying one refresh cycle produced for the compile monitor.
///
/// Every variant is independent of the Running Targets outcome in the same
/// cycle: a failed or cancelled classification still leaves the completed
/// observation available to Running Targets. The classification is boxed
/// because it dwarfs the other variants and every cycle moves one of these
/// through the worker result channel.
#[derive(Debug, PartialEq)]
pub(crate) enum CompileClassificationExecution {
    NotRequested,
    Completed(CompletedBuildClassification),
    /// The cycle ran and produced no classification. The generation is carried
    /// so a failure arriving after the scope moved on is dropped instead of
    /// aging the data the new generation is showing.
    Failed {
        compile_monitor_generation:             CompileMonitorGeneration,
        build_classification_execution_failure: BuildClassificationExecutionFailure,
    },
    Cancelled(CompileMonitorGeneration),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_refuses_a_generation_it_was_not_created_for() {
        let mut compile_monitor_generation = CompileMonitorGeneration::default();
        compile_monitor_generation.advance();
        let cancellation =
            CompileClassificationCancellation::for_generation(compile_monitor_generation);
        let mut later_generation = compile_monitor_generation;
        later_generation.advance();

        assert_eq!(
            cancellation.cancel(later_generation),
            CompileClassificationCancellationOutcome::BoundToAnotherGeneration
        );
        assert_eq!(
            cancellation.progress(),
            CompileClassificationProgress::Proceed
        );

        assert_eq!(
            cancellation.cancel(compile_monitor_generation),
            CompileClassificationCancellationOutcome::Cancelled
        );
        assert_eq!(
            cancellation.progress(),
            CompileClassificationProgress::Cancelled
        );
    }

    #[test]
    fn a_cloned_cancellation_stops_the_same_generation() {
        let compile_monitor_generation = CompileMonitorGeneration::default();
        let cancellation =
            CompileClassificationCancellation::for_generation(compile_monitor_generation);
        let worker_side_cancellation = cancellation.clone();

        assert_eq!(
            cancellation.cancel(compile_monitor_generation),
            CompileClassificationCancellationOutcome::Cancelled
        );
        assert_eq!(
            worker_side_cancellation.progress(),
            CompileClassificationProgress::Cancelled
        );
    }
}
