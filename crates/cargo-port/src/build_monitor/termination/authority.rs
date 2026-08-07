//! Snapshot-derived authority and opaque selected or scoped authorization.

use std::collections::BTreeMap;

use super::super::scope::BuildScopeKey;
use super::super::session::BuildSessionId;
use super::super::session::SessionScope;
use super::super::snapshot::ActionableMonitorData;
use super::lifecycle::BuildTerminationDisplayIdentity;
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

/// Opaque authority frozen for the exact all-actionable set in one scope.
#[derive(Debug)]
pub(crate) struct ScopeTerminationAuthorization {
    pub(super) scope_key: BuildScopeKey,
    pub(super) targets:   Vec<FrozenBuildTerminationTarget>,
}

/// Whether a retained scope authorization still covers the current roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::build_monitor) enum ScopeTerminationAuthorizationCurrency {
    /// Covered roots are equal; revision-only churn does not invalidate targets.
    Current,
    /// The current actionable scope covers a different root set.
    CoveredRootsChanged,
}

impl ScopeTerminationAuthorization {
    pub(in crate::build_monitor) fn currency_against(
        &self,
        actionable_monitor_data: ActionableMonitorData<'_>,
    ) -> ScopeTerminationAuthorizationCurrency {
        if self.scope_key.covered_scope_roots()
            == actionable_monitor_data
                .build_scope_key()
                .covered_scope_roots()
        {
            ScopeTerminationAuthorizationCurrency::Current
        } else {
            ScopeTerminationAuthorizationCurrency::CoveredRootsChanged
        }
    }
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
    /// The scope has no roots to terminate or includes an observed-only root.
    ScopeNotFullyActionable,
    /// An active transaction owns the lifecycle until it becomes terminal.
    Busy,
}
