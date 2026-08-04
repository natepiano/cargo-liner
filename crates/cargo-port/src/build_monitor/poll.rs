//! Storing one classified cycle into the monitor, narrowing it to the selected
//! scope, and aging what is shown when a cycle produces nothing.
//!
//! This is the single site that narrows the host-wide classification to the
//! current [`BuildScopeKey`]. The worker deliberately classifies every host
//! build session, which is what keeps an out-of-scope Cargo Port-owned run
//! classifiable; the narrowing happens here so the set the pane renders and the
//! set a scope-wide termination acts on are one value.

use std::collections::BTreeSet;

use super::BuildMonitor;
use super::activity::AttributedSession;
use super::activity::CompileActivity;
use super::activity::CompilerAttribution;
use super::activity::UnattributedCompileActivity;
use super::activity::UnattributedScopeEvidence;
use super::classify::BuildClassification;
use super::execution::CompletedBuildClassification;
use super::scope::BuildScopeActionability;
use super::scope::BuildScopeKey;
use super::session::BuildSessionId;
use super::session::LiveOwnedRoot;
use super::session::OwnedRootEvidence;
use super::session::ScopeAttribution;
use super::session::SessionScope;
use super::snapshot::MonitorData;
use super::snapshot::MonitorSessionOwnership;
use super::snapshot::MonitorSessionRow;
use super::snapshot::MonitorSnapshot;
use crate::project::CanonicalCheckoutRoot;

impl BuildMonitor {
    /// Store one completed classification as the latest presentation snapshot.
    ///
    /// The caller has already checked that the classification's generation is
    /// the one the monitor still accepts; a result from a superseded generation
    /// never reaches here. Ownership is joined against the owned-root evidence
    /// the completion carries, not the live evidence at this instant: a run that
    /// turned over mid-cycle would otherwise relabel this cycle's sessions with
    /// its successor's identity, or drop an out-of-scope owned session entirely.
    pub(crate) fn record_classification(
        &mut self,
        completed_build_classification: CompletedBuildClassification,
    ) {
        let (build_scope_key, owned_root_evidence, build_classification) =
            completed_build_classification.into_scoped_classification();
        let monitor_data =
            scoped_monitor_data(build_scope_key, &build_classification, &owned_root_evidence);
        self.live_session_ids = monitor_data
            .session_rows()
            .iter()
            .map(|monitor_session_row| monitor_session_row.build_session_id().clone())
            .collect();
        self.monitor_snapshot = MonitorSnapshot::Fresh(monitor_data);
    }

    /// Age what is shown by one step because a cycle produced no
    /// classification at all. Data that matched the current generation becomes
    /// visibly stale and non-actionable for one interval, then unavailable.
    ///
    /// The caller has already checked that the failing cycle belongs to the
    /// generation the monitor is still waiting on.
    pub(crate) fn record_classification_failure(&mut self) {
        self.monitor_snapshot =
            std::mem::replace(&mut self.monitor_snapshot, MonitorSnapshot::Unavailable).aged();
        if matches!(self.monitor_snapshot, MonitorSnapshot::Unavailable) {
            self.live_session_ids.clear();
        }
    }

    /// Move to the new scope, keeping the prior rows on screen when the new
    /// scope covers the same canonical roots.
    ///
    /// Ordinary cursor movement between two rows of one workspace advances the
    /// generation with both root sets unchanged, so retention is the normal
    /// display state during a scan rather than an edge one.
    pub(crate) fn replace_scope(&mut self, build_scope_actionability: &BuildScopeActionability) {
        self.monitor_snapshot = match build_scope_actionability {
            BuildScopeActionability::Actionable(build_scope_key) => {
                std::mem::replace(&mut self.monitor_snapshot, MonitorSnapshot::Pending)
                    .superseded_by_scope(build_scope_key)
            },
            BuildScopeActionability::NotActionable => MonitorSnapshot::Pending,
        };
        if !matches!(
            self.monitor_snapshot,
            MonitorSnapshot::PendingWithRetained(_)
        ) {
            self.live_session_ids.clear();
        }
    }
}

/// Keep the sessions this scope covers, plus the Cargo Port-owned session
/// wherever it is building, together with each surviving session's attributed
/// activities and the unattributed activities this scope still explains.
///
/// This is the one narrowing site. The unattributed set is filtered here from
/// the same surviving-session identities the rows were built from, so the pane
/// never re-derives it: an ambiguous activity survives when at least one of its
/// candidate sessions did, and an activity that named no candidate at all
/// survives because nothing observed places it anywhere else.
///
/// Scope containment is one-sided: a session carries a canonical checkout root
/// and nothing else, so this is not the two-sided root comparison that decides
/// scope-key equality.
fn scoped_monitor_data(
    build_scope_key: BuildScopeKey,
    build_classification: &BuildClassification,
    owned_root_evidence: &OwnedRootEvidence,
) -> MonitorData {
    let session_rows: Vec<MonitorSessionRow> = build_classification
        .build_sessions()
        .iter()
        .filter_map(|build_session| {
            let session_ownership =
                session_ownership(build_session.session_scope(), owned_root_evidence);
            let within_scope = build_session
                .session_scope()
                .is_within(build_scope_key.canonical_checkout_roots());
            if !within_scope && session_ownership == MonitorSessionOwnership::External {
                return None;
            }
            let compile_activities = attributed_activities(
                build_session.build_session_id(),
                build_classification.compile_activities(),
            );
            Some(MonitorData::session_row(
                build_session.clone(),
                compile_activities,
                session_ownership,
            ))
        })
        .collect();
    let scoped_session_ids: BTreeSet<&BuildSessionId> = session_rows
        .iter()
        .map(MonitorSessionRow::build_session_id)
        .collect();
    let unattributed_activities = build_classification
        .unattributed_compile_activities()
        .iter()
        .filter(|unattributed_compile_activity| {
            scope_explains_unattributed(
                unattributed_compile_activity,
                &build_scope_key,
                &scoped_session_ids,
            )
        })
        .cloned()
        .collect();
    MonitorData::new(
        build_scope_key,
        session_rows,
        unattributed_activities,
        build_classification.cycle_instant(),
    )
}

/// The activities this cycle resolved to one session, in classification order.
fn attributed_activities(
    build_session_id: &BuildSessionId,
    compile_activities: &[CompileActivity],
) -> Vec<CompileActivity> {
    compile_activities
        .iter()
        .filter(|compile_activity| {
            matches!(
                compile_activity.compiler_attribution().attributed_session(),
                AttributedSession::Session(attributed_session_id)
                    if attributed_session_id == build_session_id
            )
        })
        .cloned()
        .collect()
}

/// Whether the narrowed scope still has to explain one unattributed activity.
///
/// An ambiguous activity is placed by its candidate sessions, which is the
/// stronger evidence. One that named no candidate at all is placed by where it
/// was observed working, and stays visible when that directory could not be
/// read at all.
fn scope_explains_unattributed(
    unattributed_compile_activity: &UnattributedCompileActivity,
    build_scope_key: &BuildScopeKey,
    scoped_session_ids: &BTreeSet<&BuildSessionId>,
) -> bool {
    match unattributed_compile_activity.compiler_attribution() {
        CompilerAttribution::Ambiguous { candidates } => candidates
            .candidates()
            .iter()
            .any(|build_session_id| scoped_session_ids.contains(build_session_id)),
        CompilerAttribution::Unattributed => scope_covers_working_directory(
            unattributed_compile_activity.scope_evidence(),
            build_scope_key.canonical_checkout_roots(),
        ),
        CompilerAttribution::Confirmed(_) | CompilerAttribution::UniqueOutputMatch(_) => false,
    }
}

/// Whether one covered checkout root contains the directory an unattributed
/// compiler was working in.
///
/// Containment, not equality: Cargo runs a compiler in the workspace root it is
/// building, which for a nested workspace sits under the checkout root the
/// scope names rather than at it.
fn scope_covers_working_directory(
    unattributed_scope_evidence: &UnattributedScopeEvidence,
    canonical_checkout_roots: &[CanonicalCheckoutRoot],
) -> bool {
    match unattributed_scope_evidence {
        UnattributedScopeEvidence::WorkingDirectory(working_directory) => canonical_checkout_roots
            .iter()
            .any(|canonical_checkout_root| {
                working_directory
                    .as_path()
                    .starts_with(canonical_checkout_root.path().as_path())
            }),
        UnattributedScopeEvidence::Unplaceable => true,
    }
}

/// Associate the one Cargo Port-owned run with the one session classification
/// resolved through its own Cargo root, retaining only the lifecycle identity.
const fn session_ownership(
    session_scope: &SessionScope,
    owned_root_evidence: &OwnedRootEvidence,
) -> MonitorSessionOwnership {
    let OwnedRootEvidence::Root(live_owned_root) = owned_root_evidence else {
        return MonitorSessionOwnership::External;
    };
    match session_scope {
        SessionScope::Resolved {
            method: ScopeAttribution::OwnedRoot,
            ..
        } => MonitorSessionOwnership::Owned(LiveOwnedRoot::owned_run_id(live_owned_root)),
        SessionScope::Resolved { .. } | SessionScope::Unresolved => {
            MonitorSessionOwnership::External
        },
    }
}
