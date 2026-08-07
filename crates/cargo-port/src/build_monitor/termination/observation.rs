//! Sole-observer root revalidation and descendant admission policy.

use std::collections::BTreeMap;

use super::super::session::SessionScope;
use super::transaction::BuildTerminationTransactionId;
use crate::process_observation::ExternalTerminationSupport;
use crate::process_observation::ProcessObserver;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::snapshot::BuildCandidateRole;
use crate::process_observation::snapshot::ProcessFieldObservation;
use crate::process_observation::snapshot::ProcessObservationSnapshot;
use crate::process_observation::snapshot::ProcessSnapshotRecord;
use crate::process_termination::AdmittedTerminationDescendantObservation;
use crate::process_termination::AdmittedTerminationDescendantPresence;
use crate::process_termination::ExternalProcessTerminationCapability;
use crate::process_termination::FrozenTerminationRootObservation;
use crate::process_termination::FrozenTerminationRootPresence;
use crate::process_termination::TerminationDescendantObservationPass;
use crate::process_termination::TerminationTargetId;
use crate::process_termination::observe_termination_descendants;

/// Whether the shared refresh worker owes an observation pass to a transaction.
#[derive(Clone, Debug)]
pub(crate) enum BuildTerminationObservationDemand {
    NotRequested,
    Requested(BuildTerminationObservationRequest),
}

/// Frozen identities and retained descendants for one observer pass.
#[derive(Clone, Debug)]
pub(crate) struct BuildTerminationObservationRequest {
    pub(super) transaction_id:       BuildTerminationTransactionId,
    pub(super) frozen_roots:         Vec<BuildTerminationRootObservationRequest>,
    pub(super) admitted_descendants: Vec<AdmittedTerminationDescendantObservation>,
}

/// One frozen root's scope condition for an observer pass.
#[derive(Clone, Debug)]
pub(super) struct BuildTerminationRootObservationRequest {
    pub(super) semantic_target_id: TerminationTargetId,
    pub(super) session_scope:      SessionScope,
    pub(super) root_identity:      ProcessIdentity,
}

/// What the shared worker produced for one transaction pass.
#[derive(Debug)]
pub(crate) enum BuildTerminationObservationExecution {
    NotRequested,
    Completed(CompletedBuildTerminationObservation),
}

/// Root revalidation, retained target presence, and new move-only capabilities.
#[derive(Debug)]
pub(crate) struct CompletedBuildTerminationObservation {
    pub(super) transaction_id:       BuildTerminationTransactionId,
    pub(super) root_presence:        BTreeMap<TerminationTargetId, BuildTerminationRootPresence>,
    pub(super) descendant_presence:
        BTreeMap<TerminationTargetId, AdmittedTerminationDescendantPresence>,
    pub(super) admitted_descendants: Vec<NewActionableTerminationDescendant>,
    exclusions:                      Vec<BuildTerminationDescendantExclusion>,
}

/// Whether a frozen root remains live under its exact frozen scope condition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BuildTerminationRootPresence {
    LiveInFrozenScope,
    Gone,
    ScopeDiverged,
}

/// A newly admitted descendant with authority minted by the shared observer.
#[derive(Debug)]
pub(super) struct NewActionableTerminationDescendant {
    pub(super) root_target_id:   TerminationTargetId,
    pub(super) process_identity: ProcessIdentity,
    pub(super) parent_identity:  ProcessIdentity,
    pub(super) depth_from_root:  usize,
    pub(super) capability:       ExternalProcessTerminationCapability,
}

/// Why a process below a root remained observed-only for this transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildTerminationDescendantExclusionReason {
    CargoPort,
    ShellOrLlmAncestor,
    PersistentCompilerCache,
    SeparateNestedSession,
    ScopeDivergent,
    TargetDirectoryHeuristicOnly,
    HostObservedOnly,
}

/// One excluded descendant and the exact policy that excluded it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct BuildTerminationDescendantExclusion {
    process_identity: ProcessIdentity,
    reason:           BuildTerminationDescendantExclusionReason,
}

/// How build evidence associated a descendant with the frozen root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TerminationDescendantAssociation {
    ValidatedParentChain,
    TargetDirectoryHeuristicOnly,
}

/// Whether one descendant executable is a nested Cargo invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NestedCargoProcess {
    Cargo,
    Other,
}

/// Classify one active transaction pass on the worker that owns the sole observer.
pub(crate) fn observe_build_termination_demand(
    process_observer: &ProcessObserver,
    process_observation_snapshot: &ProcessObservationSnapshot,
    build_termination_observation_demand: BuildTerminationObservationDemand,
) -> BuildTerminationObservationExecution {
    let BuildTerminationObservationDemand::Requested(request) =
        build_termination_observation_demand
    else {
        return BuildTerminationObservationExecution::NotRequested;
    };
    let frozen_roots: Vec<_> = request
        .frozen_roots
        .iter()
        .map(|root| {
            FrozenTerminationRootObservation::new(
                root.semantic_target_id,
                root.root_identity.clone(),
            )
        })
        .collect();
    let descendant_observation_pass = observe_termination_descendants(
        process_observation_snapshot,
        &frozen_roots,
        &request.admitted_descendants,
    );
    let root_presence = observe_root_presence(
        process_observation_snapshot,
        &request,
        &descendant_observation_pass,
    );
    let (admitted_descendants, exclusions) = classify_new_descendants(
        process_observer,
        process_observation_snapshot,
        &request,
        &descendant_observation_pass,
        &root_presence,
    );
    BuildTerminationObservationExecution::Completed(CompletedBuildTerminationObservation {
        transaction_id: request.transaction_id,
        root_presence,
        descendant_presence: descendant_observation_pass.descendant_presence().clone(),
        admitted_descendants,
        exclusions,
    })
}

fn observe_root_presence(
    process_observation_snapshot: &ProcessObservationSnapshot,
    request: &BuildTerminationObservationRequest,
    descendant_observation_pass: &TerminationDescendantObservationPass,
) -> BTreeMap<TerminationTargetId, BuildTerminationRootPresence> {
    request
        .frozen_roots
        .iter()
        .map(|root| {
            let presence = match descendant_observation_pass
                .root_presence()
                .get(&root.semantic_target_id)
            {
                Some(FrozenTerminationRootPresence::Live) => process_observation_snapshot
                    .strongly_identified_processes()
                    .get(&root.root_identity)
                    .map_or(
                        BuildTerminationRootPresence::Gone,
                        |process_snapshot_record| {
                            if process_record_is_within_scope(
                                process_snapshot_record,
                                &root.session_scope,
                            ) {
                                BuildTerminationRootPresence::LiveInFrozenScope
                            } else {
                                BuildTerminationRootPresence::ScopeDiverged
                            }
                        },
                    ),
                Some(FrozenTerminationRootPresence::Gone) | None => {
                    BuildTerminationRootPresence::Gone
                },
            };
            (root.semantic_target_id, presence)
        })
        .collect()
}

fn classify_new_descendants(
    process_observer: &ProcessObserver,
    process_observation_snapshot: &ProcessObservationSnapshot,
    request: &BuildTerminationObservationRequest,
    descendant_observation_pass: &TerminationDescendantObservationPass,
    root_presence: &BTreeMap<TerminationTargetId, BuildTerminationRootPresence>,
) -> (
    Vec<NewActionableTerminationDescendant>,
    Vec<BuildTerminationDescendantExclusion>,
) {
    let mut admitted_descendants = Vec::new();
    let mut exclusions = Vec::new();
    for candidate in descendant_observation_pass.new_candidates() {
        let Some(root) = request
            .frozen_roots
            .iter()
            .find(|root| root.semantic_target_id == candidate.root_target_id())
        else {
            continue;
        };
        if root_presence.get(&root.semantic_target_id)
            != Some(&BuildTerminationRootPresence::LiveInFrozenScope)
        {
            continue;
        }
        let Some(process_snapshot_record) = process_observation_snapshot
            .strongly_identified_processes()
            .get(candidate.process_identity())
        else {
            continue;
        };
        let nested_cargo_process = process_observation_snapshot
            .build_candidate_incarnations()
            .iter()
            .find(|(incarnation, _)| incarnation.identity() == candidate.process_identity())
            .map_or(NestedCargoProcess::Other, |(_, build_candidate_role)| {
                match build_candidate_role {
                    BuildCandidateRole::Cargo => NestedCargoProcess::Cargo,
                    BuildCandidateRole::Compiler | BuildCandidateRole::Wrapper => {
                        NestedCargoProcess::Other
                    },
                }
            });
        if let Err(reason) = descendant_admission_policy(
            process_snapshot_record,
            &root.session_scope,
            nested_cargo_process,
            TerminationDescendantAssociation::ValidatedParentChain,
        ) {
            exclusions.push(BuildTerminationDescendantExclusion {
                process_identity: candidate.process_identity().clone(),
                reason,
            });
            continue;
        }
        match process_observer.external_termination_support(process_snapshot_record) {
            ExternalTerminationSupport::Actionable(capability) => {
                admitted_descendants.push(NewActionableTerminationDescendant {
                    root_target_id: candidate.root_target_id(),
                    process_identity: candidate.process_identity().clone(),
                    parent_identity: candidate.parent_identity().clone(),
                    depth_from_root: candidate.depth_from_root(),
                    capability,
                });
            },
            ExternalTerminationSupport::ObservedOnly => {
                exclusions.push(BuildTerminationDescendantExclusion {
                    process_identity: candidate.process_identity().clone(),
                    reason:           BuildTerminationDescendantExclusionReason::HostObservedOnly,
                });
            },
        }
    }
    (admitted_descendants, exclusions)
}

fn descendant_admission_policy(
    process_snapshot_record: &ProcessSnapshotRecord,
    session_scope: &SessionScope,
    nested_cargo_process: NestedCargoProcess,
    termination_descendant_association: TerminationDescendantAssociation,
) -> Result<(), BuildTerminationDescendantExclusionReason> {
    if termination_descendant_association
        == TerminationDescendantAssociation::TargetDirectoryHeuristicOnly
    {
        return Err(BuildTerminationDescendantExclusionReason::TargetDirectoryHeuristicOnly);
    }
    let executable_name = observed_executable_name(process_snapshot_record);
    if process_snapshot_record.identity().pid() == std::process::id()
        || executable_name == "cargo-port"
    {
        return Err(BuildTerminationDescendantExclusionReason::CargoPort);
    }
    if matches!(
        executable_name,
        "sh" | "bash"
            | "dash"
            | "fish"
            | "zsh"
            | "nu"
            | "pwsh"
            | "powershell"
            | "claude"
            | "codex"
            | "gemini"
    ) {
        return Err(BuildTerminationDescendantExclusionReason::ShellOrLlmAncestor);
    }
    if matches!(executable_name, "sccache" | "rust-cache") {
        return Err(BuildTerminationDescendantExclusionReason::PersistentCompilerCache);
    }
    if !process_record_is_within_scope(process_snapshot_record, session_scope) {
        return match nested_cargo_process {
            NestedCargoProcess::Cargo => {
                Err(BuildTerminationDescendantExclusionReason::SeparateNestedSession)
            },
            NestedCargoProcess::Other => {
                Err(BuildTerminationDescendantExclusionReason::ScopeDivergent)
            },
        };
    }
    Ok(())
}

fn observed_executable_name(process_snapshot_record: &ProcessSnapshotRecord) -> &str {
    let ProcessFieldObservation::Observed(executable) = process_snapshot_record.executable() else {
        return "";
    };
    executable
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default()
}

fn process_record_is_within_scope(
    process_snapshot_record: &ProcessSnapshotRecord,
    session_scope: &SessionScope,
) -> bool {
    let SessionScope::Resolved { root, .. } = session_scope else {
        return false;
    };
    matches!(
        process_snapshot_record.cwd(),
        ProcessFieldObservation::Observed(working_directory)
            if working_directory.starts_with(root.path().as_path())
    )
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::num::NonZeroU64;
    use std::path::Path;
    use std::path::PathBuf;

    use super::*;
    use crate::build_monitor::session::ScopeAttribution;
    use crate::process_observation::snapshot_builder::ObservedProcess;
    use crate::process_observation::snapshot_builder::snapshot_of;
    use crate::project::AbsolutePath;
    use crate::project::CanonicalCheckoutRoot;

    fn transaction_id() -> BuildTerminationTransactionId {
        BuildTerminationTransactionId(NonZeroU64::MIN)
    }

    fn target_id(value: u64) -> TerminationTargetId {
        let Some(value) = NonZeroU64::new(value) else {
            panic!("test target identity should be nonzero");
        };
        TerminationTargetId::for_test(value)
    }

    fn resolved_scope(root: &str) -> SessionScope {
        SessionScope::Resolved {
            method: ScopeAttribution::WorkingDirectoryManifest,
            root:   CanonicalCheckoutRoot::for_test(AbsolutePath::from(PathBuf::from(root))),
        }
    }

    fn completed_observation(
        execution: BuildTerminationObservationExecution,
    ) -> CompletedBuildTerminationObservation {
        match execution {
            BuildTerminationObservationExecution::Completed(completed) => completed,
            BuildTerminationObservationExecution::NotRequested => {
                panic!("test requested a transaction observation")
            },
        }
    }

    #[test]
    fn sole_observer_transports_descendant_capabilities_across_full_chain_passes() {
        let root_target_id = target_id(1);
        let child_target_id = target_id(2);
        let root = ObservedProcess::new(4_000_001, 1, "root", "/usr/bin/cargo", &["cargo"])
            .with_cwd(Path::new("/workspace/project"));
        let child = ObservedProcess::new(4_000_002, 2, "child", "/usr/bin/rustc", &["rustc"])
            .with_cwd(Path::new("/workspace/project"))
            .with_validated_parent(root.identity());
        let first_snapshot = snapshot_of(&[root.clone(), child.clone()]);
        let first_request = BuildTerminationObservationRequest {
            transaction_id:       transaction_id(),
            frozen_roots:         vec![BuildTerminationRootObservationRequest {
                semantic_target_id: root_target_id,
                session_scope:      resolved_scope("/workspace"),
                root_identity:      root.identity().clone(),
            }],
            admitted_descendants: Vec::new(),
        };

        let mut unrelated_observer = ProcessObserver::default();
        unrelated_observer.enable_actionable_capability_fixture();
        let unrelated_observation = completed_observation(observe_build_termination_demand(
            &unrelated_observer,
            &first_snapshot,
            BuildTerminationObservationDemand::Requested(first_request.clone()),
        ));
        assert!(unrelated_observation.admitted_descendants.is_empty());
        assert!(matches!(
            unrelated_observation.exclusions.as_slice(),
            [BuildTerminationDescendantExclusion {
                reason: BuildTerminationDescendantExclusionReason::HostObservedOnly,
                ..
            }]
        ));

        let mut sole_observer = ProcessObserver::default();
        sole_observer.enable_actionable_capability_fixture();
        sole_observer.remember_snapshot_incarnations_for_test(&first_snapshot);
        let first_observation = completed_observation(observe_build_termination_demand(
            &sole_observer,
            &first_snapshot,
            BuildTerminationObservationDemand::Requested(first_request),
        ));
        assert_eq!(first_observation.admitted_descendants.len(), 1);
        assert_eq!(
            first_observation.admitted_descendants[0].process_identity,
            *child.identity()
        );
        assert!(
            first_observation.admitted_descendants[0]
                .capability
                .is_actionable()
        );

        let grandchild =
            ObservedProcess::new(4_000_003, 3, "grandchild", "/usr/bin/rustc", &["rustc"])
                .with_cwd(Path::new("/workspace/project"))
                .with_validated_parent(child.identity());
        let second_snapshot = snapshot_of(&[root.clone(), child.clone(), grandchild.clone()]);
        sole_observer.remember_snapshot_incarnations_for_test(&second_snapshot);
        let second_observation = completed_observation(observe_build_termination_demand(
            &sole_observer,
            &second_snapshot,
            BuildTerminationObservationDemand::Requested(BuildTerminationObservationRequest {
                transaction_id:       transaction_id(),
                frozen_roots:         vec![BuildTerminationRootObservationRequest {
                    semantic_target_id: root_target_id,
                    session_scope:      resolved_scope("/workspace"),
                    root_identity:      root.identity().clone(),
                }],
                admitted_descendants: vec![AdmittedTerminationDescendantObservation::new(
                    child_target_id,
                    child.identity().clone(),
                )],
            }),
        ));
        assert_eq!(second_observation.admitted_descendants.len(), 1);
        assert_eq!(
            second_observation.admitted_descendants[0].process_identity,
            *grandchild.identity()
        );
        assert_eq!(
            second_observation.admitted_descendants[0].parent_identity,
            *child.identity()
        );
        assert_eq!(
            second_observation.admitted_descendants[0].depth_from_root,
            2
        );
        assert!(
            second_observation.admitted_descendants[0]
                .capability
                .is_actionable()
        );
    }

    #[test]
    fn admission_policy_names_every_transaction_exclusion() {
        let session_scope = resolved_scope("/workspace");
        let cases = [
            (
                "/workspace/project",
                "/usr/bin/cargo-port",
                NestedCargoProcess::Other,
                TerminationDescendantAssociation::ValidatedParentChain,
                BuildTerminationDescendantExclusionReason::CargoPort,
            ),
            (
                "/workspace/project",
                "/usr/bin/codex",
                NestedCargoProcess::Other,
                TerminationDescendantAssociation::ValidatedParentChain,
                BuildTerminationDescendantExclusionReason::ShellOrLlmAncestor,
            ),
            (
                "/workspace/project",
                "/usr/bin/sccache",
                NestedCargoProcess::Other,
                TerminationDescendantAssociation::ValidatedParentChain,
                BuildTerminationDescendantExclusionReason::PersistentCompilerCache,
            ),
            (
                "/other/project",
                "/usr/bin/cargo",
                NestedCargoProcess::Cargo,
                TerminationDescendantAssociation::ValidatedParentChain,
                BuildTerminationDescendantExclusionReason::SeparateNestedSession,
            ),
            (
                "/other/project",
                "/usr/bin/rustc",
                NestedCargoProcess::Other,
                TerminationDescendantAssociation::ValidatedParentChain,
                BuildTerminationDescendantExclusionReason::ScopeDivergent,
            ),
            (
                "/workspace/project",
                "/usr/bin/rustc",
                NestedCargoProcess::Other,
                TerminationDescendantAssociation::TargetDirectoryHeuristicOnly,
                BuildTerminationDescendantExclusionReason::TargetDirectoryHeuristicOnly,
            ),
        ];

        for (index, (cwd, executable, nested_cargo, association, expected)) in
            cases.into_iter().enumerate()
        {
            let Ok(index) = u32::try_from(index) else {
                panic!("exclusion case index should fit in u32");
            };
            let process =
                ObservedProcess::new(4_100_000 + index, 1, executable, executable, &[executable])
                    .with_cwd(Path::new(cwd));
            let snapshot = snapshot_of(std::slice::from_ref(&process));
            let record = &snapshot.strongly_identified_processes()[process.identity()];
            assert_eq!(
                descendant_admission_policy(record, &session_scope, nested_cargo, association),
                Err(expected)
            );
        }

        let admitted = ObservedProcess::new(4_100_100, 1, "admitted", "/usr/bin/rustc", &["rustc"])
            .with_cwd(Path::new("/workspace/project"));
        let admitted_snapshot = snapshot_of(std::slice::from_ref(&admitted));
        assert_eq!(
            descendant_admission_policy(
                &admitted_snapshot.strongly_identified_processes()[admitted.identity()],
                &session_scope,
                NestedCargoProcess::Other,
                TerminationDescendantAssociation::ValidatedParentChain,
            ),
            Ok(())
        );
    }
}
