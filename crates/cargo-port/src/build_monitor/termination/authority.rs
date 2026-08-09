//! Snapshot-derived authority and opaque selected or output-build-set authorization.

use std::collections::BTreeMap;

use super::lifecycle::BuildTerminationDisplayIdentity;
use super::transaction::AdditionalBuildExclusion;
use crate::build_monitor::scope::BuildScopeKey;
use crate::build_monitor::session::BuildSessionId;
use crate::build_monitor::session::SessionScope;
use crate::build_monitor::snapshot::ActionableMonitorData;
use crate::process_observation::ExternalTerminationSupport;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_termination::ExternalProcessTerminationCapability;
use crate::tui::OwnedRunId;
use crate::tui::OwnedRunTerminationToken;

/// Raw owned-run support carried across one classification boundary.
///
/// This is not build authority: only `BuildMonitor` may join the token to the
/// owned session row and the snapshot's visible actionability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedTerminationSupport {
    /// The owned-run actor issued a token for this live run.
    Actionable {
        owned_run_id:                OwnedRunId,
        owned_run_termination_token: OwnedRunTerminationToken,
    },
    /// No actor-issued token was available when the cycle was dispatched.
    Unavailable,
}

/// Move-only external support classified under one session identity.
#[derive(Debug)]
pub(crate) enum ClassifiedExternalTerminationSupport {
    /// The observer found a safe external signal adapter for this root.
    Actionable(ExternalProcessTerminationCapability),
    /// The root remains visible but no identity-bound signal adapter exists.
    ObservedOnly,
}

impl From<ExternalTerminationSupport> for ClassifiedExternalTerminationSupport {
    fn from(external_termination_support: ExternalTerminationSupport) -> Self {
        match external_termination_support {
            ExternalTerminationSupport::Actionable(external_process_termination_capability) => {
                Self::Actionable(external_process_termination_capability)
            },
            ExternalTerminationSupport::ObservedOnly => Self::ObservedOnly,
        }
    }
}

/// Move-only root support indexed by the session classification created from
/// the same observer cycle.
#[derive(Debug, Default)]
pub(crate) struct ClassifiedExternalTerminationSupports {
    by_session_id: BTreeMap<BuildSessionId, ClassifiedExternalTerminationSupport>,
}

impl ClassifiedExternalTerminationSupports {
    pub(crate) fn insert(
        &mut self,
        build_session_id: BuildSessionId,
        classified_external_termination_support: ClassifiedExternalTerminationSupport,
    ) {
        self.by_session_id
            .insert(build_session_id, classified_external_termination_support);
    }

    pub(in crate::build_monitor) fn take(
        &mut self,
        build_session_id: &BuildSessionId,
    ) -> ClassifiedExternalTerminationSupport {
        self.by_session_id
            .remove(build_session_id)
            .unwrap_or(ClassifiedExternalTerminationSupport::ObservedOnly)
    }
}

/// Authority for an owned build root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OwnedBuildTerminationAuthority {
    pub(in crate::build_monitor) owned_run_id:                OwnedRunId,
    pub(in crate::build_monitor) owned_run_termination_token: OwnedRunTerminationToken,
}

/// Authority for an external build root.
#[derive(Debug)]
pub(crate) struct ExternalBuildTerminationAuthority {
    pub(in crate::build_monitor) session_scope:                           SessionScope,
    pub(in crate::build_monitor) root_identity:                           ProcessIdentity,
    pub(in crate::build_monitor) external_process_termination_capability:
        ExternalProcessTerminationCapability,
}

/// The only action-bearing representation of a current build session.
#[derive(Debug)]
pub(crate) enum BuildTerminationAuthority {
    /// Cargo Port owns the root and its isolated actor issued this token.
    Owned(OwnedBuildTerminationAuthority),
    /// A strong external root identity has scope evidence and a safe adapter.
    External(ExternalBuildTerminationAuthority),
}

/// Opaque authority frozen for exactly one selected build.
#[derive(Debug)]
pub(crate) struct SelectedBuildTerminationAuthorization {
    pub(super) session_id:       BuildSessionId,
    pub(super) scope_key:        BuildScopeKey,
    pub(super) authority:        BuildTerminationAuthority,
    pub(super) display_identity: BuildTerminationDisplayIdentity,
}

/// Whether one displayed build session can currently freeze a selected-build
/// termination authorization.
///
/// This is deliberately read-only. The Output shortcut bar needs to describe
/// the same authority map that confirmation will consume without taking the
/// move-only capability out of that map.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectedBuildTerminationAvailability {
    /// The displayed session has a current authority-bearing handle.
    Available,
    /// The monitor has no current snapshot on which termination is allowed.
    SnapshotNotActionable,
    /// This displayed session has no current authority-bearing handle.
    SessionNotActionable,
    /// An earlier transaction owns the lifecycle until it reaches a terminal state.
    Busy,
}

/// Whether a retained selected-build authorization still names current data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::build_monitor) enum SelectedBuildTerminationAuthorizationCurrency {
    /// The exact frozen scope key and session identity remain current.
    Current,
    /// A revision or covered-root change replaced the frozen scope key.
    ScopeChanged,
    /// The frozen session is no longer a row in the exact current scope.
    SessionChanged,
}

impl SelectedBuildTerminationAuthorization {
    pub(in crate::build_monitor) fn currency_against(
        &self,
        actionable_monitor_data: ActionableMonitorData<'_>,
    ) -> SelectedBuildTerminationAuthorizationCurrency {
        if self.scope_key != *actionable_monitor_data.build_scope_key() {
            return SelectedBuildTerminationAuthorizationCurrency::ScopeChanged;
        }
        if actionable_monitor_data
            .session_rows()
            .iter()
            .any(|monitor_session_row| monitor_session_row.build_session_id() == &self.session_id)
        {
            SelectedBuildTerminationAuthorizationCurrency::Current
        } else {
            SelectedBuildTerminationAuthorizationCurrency::SessionChanged
        }
    }
}

/// Opaque authority frozen for the exact actionable root rows in Output.
#[derive(Debug)]
pub(crate) struct OutputBuildSetTerminationAuthorization {
    pub(super) scope_key: BuildScopeKey,
    pub(super) targets:   Vec<FrozenBuildTerminationTarget>,
}

/// Whether a retained output-build-set authorization still covers the current
/// roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::build_monitor) enum OutputBuildSetTerminationAuthorizationCurrency {
    /// Covered roots are equal; revision-only churn does not invalidate targets.
    Current,
    /// The current actionable scope covers a different root set.
    CoveredRootsChanged,
}

impl OutputBuildSetTerminationAuthorization {
    pub(in crate::build_monitor) fn currency_against(
        &self,
        actionable_monitor_data: ActionableMonitorData<'_>,
    ) -> OutputBuildSetTerminationAuthorizationCurrency {
        if self.scope_key.covered_scope_roots()
            == actionable_monitor_data
                .build_scope_key()
                .covered_scope_roots()
        {
            OutputBuildSetTerminationAuthorizationCurrency::Current
        } else {
            OutputBuildSetTerminationAuthorizationCurrency::CoveredRootsChanged
        }
    }

    /// Count current Output rows that appeared after this authorization froze
    /// its exact root identities. They remain outside destructive authority.
    pub(in crate::build_monitor) fn additional_build_exclusion_against(
        &self,
        actionable_monitor_data: ActionableMonitorData<'_>,
    ) -> AdditionalBuildExclusion {
        let count = actionable_monitor_data
            .session_rows()
            .iter()
            .filter(|monitor_session_row| {
                !self.targets.iter().any(|frozen_target| {
                    &frozen_target.session_id == monitor_session_row.build_session_id()
                })
            })
            .count();
        if count == 0 {
            AdditionalBuildExclusion::NoAdditionalBuilds
        } else {
            AdditionalBuildExclusion::Excluded { count }
        }
    }
}

/// Whether the exact live root rows currently displayed in Output can freeze
/// one all-or-refuse termination authorization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputBuildSetTerminationAvailability {
    /// Every displayed root row has current termination authority.
    Available,
    /// The monitor has no current actionable snapshot.
    SnapshotNotActionable,
    /// The row set is empty or includes at least one observed-only root.
    BuildSetNotFullyActionable,
    /// An earlier transaction owns the lifecycle until it reaches a terminal state.
    Busy,
}

#[derive(Debug)]
pub(super) struct FrozenBuildTerminationTarget {
    pub(super) session_id:       BuildSessionId,
    pub(super) authority:        BuildTerminationAuthority,
    pub(super) display_identity: BuildTerminationDisplayIdentity,
}

/// Why the monitor did or did not construct a frozen authorization aggregate.
#[derive(Debug)]
pub(crate) enum BuildTerminationAuthorizationConstruction<A> {
    /// An aggregate was constructed from current visible action authority.
    Authorized(A),
    /// The snapshot's displayed state does not permit any destructive action.
    SnapshotNotActionable,
    /// The selected session is missing, observed-only, already terminating, or
    /// otherwise lacks an authority-bearing handle.
    SessionNotActionable,
    /// The Output row set has no roots to terminate or includes an observed-only root.
    BuildSetNotFullyActionable,
    /// An active transaction owns the lifecycle until it becomes terminal.
    Busy,
}
