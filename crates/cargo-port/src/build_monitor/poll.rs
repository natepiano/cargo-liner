//! Storing one classified cycle into the monitor, narrowing it to the selected
//! scope, and aging what is shown when a cycle produces nothing.
//!
//! This is the single site that narrows the host-wide classification to the
//! current [`BuildScopeKey`]. The worker deliberately classifies every host
//! build session, which is what keeps an out-of-scope Cargo Port-owned run
//! classifiable; the narrowing happens here so the set the pane renders and the
//! set a scope-wide termination acts on are one value.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;

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
use super::session::BuildSession;
use super::session::BuildSessionId;
use super::session::LiveOwnedRoot;
use super::session::OwnedRootEvidence;
use super::session::ScopeAttribution;
use super::session::SessionScope;
use super::session::TargetDirectoryEvidence;
use super::snapshot::BuildLockContention;
use super::snapshot::MonitorData;
use super::snapshot::MonitorSessionOwnership;
use super::snapshot::MonitorSessionRow;
use super::snapshot::MonitorSnapshot;
use super::termination::BuildTerminationAuthority;
use super::termination::ClassifiedExternalTerminationSupport;
use super::termination::ClassifiedExternalTerminationSupports;
use super::termination::ExternalBuildTerminationAuthority;
use super::termination::OwnedBuildTerminationAuthority;
use super::termination::OwnedTerminationSupport;
use crate::project::CanonicalCheckoutRoot;
use crate::project::CanonicalTargetDirectory;

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
        let (
            build_scope_key,
            owned_root_evidence,
            build_classification,
            mut classified_external_termination_supports,
            owned_termination_support,
        ) = completed_build_classification.into_scoped_classification();
        let monitor_data =
            scoped_monitor_data(build_scope_key, &build_classification, &owned_root_evidence);
        self.termination_lifecycle_registry
            .record_fresh_observations(monitor_data.session_rows());
        self.termination_state
            .replace_current_authorities(current_termination_authorities(
                &monitor_data,
                &mut classified_external_termination_supports,
                owned_termination_support,
            ));
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
        self.termination_state.clear_current_authorities();
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
            MonitorSnapshot::PendingWithRetained(_) | MonitorSnapshot::StaleWithRetained(_)
        ) {
            self.termination_lifecycle_registry.clear_terminal_entries();
            self.termination_state.clear_current_authorities();
        }
    }
}

/// Convert this cycle's root support into action authority only for the exact
/// rows the monitor stored. An observed-only adapter, an unresolved external
/// scope, and a stale owned token all become absent from this map rather than a
/// weaker authority type.
fn current_termination_authorities(
    monitor_data: &MonitorData,
    classified_external_termination_supports: &mut ClassifiedExternalTerminationSupports,
    owned_termination_support: OwnedTerminationSupport,
) -> BTreeMap<BuildSessionId, BuildTerminationAuthority> {
    monitor_data
        .session_rows()
        .iter()
        .filter_map(|monitor_session_row| {
            let build_session = monitor_session_row.build_session();
            let build_session_id = monitor_session_row.build_session_id().clone();
            let build_termination_authority = match monitor_session_row.session_ownership() {
                MonitorSessionOwnership::Owned(owned_run_id) => match owned_termination_support {
                    OwnedTerminationSupport::Actionable {
                        owned_run_id: supported_owned_run_id,
                        owned_run_termination_token,
                    } if supported_owned_run_id == owned_run_id => Some(
                        BuildTerminationAuthority::Owned(OwnedBuildTerminationAuthority {
                            owned_run_id,
                            owned_run_termination_token,
                        }),
                    ),
                    OwnedTerminationSupport::Actionable { .. }
                    | OwnedTerminationSupport::Unavailable => None,
                },
                MonitorSessionOwnership::External => {
                    let ClassifiedExternalTerminationSupport::Actionable(
                        external_process_termination_capability,
                    ) = classified_external_termination_supports.take(&build_session_id)
                    else {
                        return None;
                    };
                    let SessionScope::Resolved { .. } = build_session.session_scope() else {
                        return None;
                    };
                    Some(BuildTerminationAuthority::External(
                        ExternalBuildTerminationAuthority {
                            session_scope: build_session.session_scope().clone(),
                            root_identity: build_session.root_identity().clone(),
                            external_process_termination_capability,
                        },
                    ))
                },
            };
            build_termination_authority
                .map(|build_termination_authority| (build_session_id, build_termination_authority))
        })
        .collect()
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
    let build_lock_contention_by_session = build_lock_contention_by_session(build_classification);
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
                build_lock_contention_by_session
                    .get(build_session.build_session_id())
                    .copied()
                    .unwrap_or_default(),
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

/// Where every live session sits in the queue for its build-directory lock.
///
/// Sessions writing to one canonical target directory serialize behind Cargo's
/// exclusive lock on it, so a set of them is a queue and the one with compiler
/// children is the one building. A set that leaves no single such session names
/// no holder: an observation cycle can straddle a handoff, and a claim about who
/// blocks whom is worth nothing if it can name two holders at once.
///
/// Runs over the whole classification rather than the narrowed rows, so a holder
/// the current scope does not cover still accounts for the waiters it blocks. A
/// session whose target directory is
/// [`Unobservable`](super::session::TargetDirectoryEvidence::Unobservable) —
/// `CARGO_TARGET_DIR` and `build.target-dir` are set in the environment, which
/// process observation does not read — joins no set and stays
/// [`Undetermined`](BuildLockContention::Undetermined).
fn build_lock_contention_by_session(
    build_classification: &BuildClassification,
) -> HashMap<&BuildSessionId, BuildLockContention> {
    let mut sessions_by_target_directory: HashMap<&CanonicalTargetDirectory, Vec<&BuildSession>> =
        HashMap::new();
    for build_session in build_classification.build_sessions() {
        if let TargetDirectoryEvidence::Determined(canonical_target_directory) =
            build_session.session_target_directory().evidence()
        {
            sessions_by_target_directory
                .entry(canonical_target_directory)
                .or_default()
                .push(build_session);
        }
    }
    let mut build_lock_contention_by_session = HashMap::new();
    for sessions_sharing_target_directory in sessions_by_target_directory.into_values() {
        if sessions_sharing_target_directory.len() < 2 {
            continue;
        }
        let compiling_sessions: Vec<&BuildSession> = sessions_sharing_target_directory
            .iter()
            .copied()
            .filter(|build_session| {
                build_classification
                    .compile_activities()
                    .iter()
                    .any(|compile_activity| {
                        activity_names_session(compile_activity, build_session.build_session_id())
                    })
            })
            .collect();
        let [holder] = compiling_sessions.as_slice() else {
            continue;
        };
        let holder_pid = holder.root_observation().root_pid();
        for build_session in sessions_sharing_target_directory {
            let build_lock_contention =
                if build_session.build_session_id() == holder.build_session_id() {
                    BuildLockContention::Holding
                } else {
                    BuildLockContention::WaitingBehind { holder_pid }
                };
            build_lock_contention_by_session
                .insert(build_session.build_session_id(), build_lock_contention);
        }
    }
    build_lock_contention_by_session
}

/// The activities this cycle resolved to one session, in classification order.
fn attributed_activities(
    build_session_id: &BuildSessionId,
    compile_activities: &[CompileActivity],
) -> Vec<CompileActivity> {
    compile_activities
        .iter()
        .filter(|compile_activity| activity_names_session(compile_activity, build_session_id))
        .cloned()
        .collect()
}

/// Whether attribution resolved one activity to exactly this session.
fn activity_names_session(
    compile_activity: &CompileActivity,
    build_session_id: &BuildSessionId,
) -> bool {
    matches!(
        compile_activity.compiler_attribution().attributed_session(),
        AttributedSession::Session(attributed_session_id)
            if attributed_session_id == build_session_id
    )
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
