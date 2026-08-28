//! Generated correlation rules and schemas for every JSON output consumer.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

use crate::alert::LostIntegrationEvidenceAlert;
use crate::alert::LostIntegrationEvidenceStatus;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::drift::IncursionCommitOrigin;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::JournalOperation;
use crate::ledger::TrunkObservationAtClaim;
use crate::output::CommandVerb;
use crate::output::IdentityPayload;
use crate::output::IntegrationPayload;
use crate::output::OutputEnvelope;
use crate::output::OutputStatus;
use crate::output::ReplayFailurePayload;
use crate::output::ReplayFailureReason;
use crate::output::ReplayFailureSubjectKind;
use crate::output::ReservationLifecycleQueryPayload;
use crate::output::ResolvePayload;
use crate::output::output_facts_schema;
use crate::reservation::EditBlockingStatus;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::IntegrationProof;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationLifecycleSnapshot;
use crate::reservation::ScopedPatchEquivalenceVerdict;
use crate::reservation::SuccessorScopedPatchEquivalenceVerdict;

const CONTRACT_NAME: &str = "cargo-berth-output";
const CONTRACT_VERSION: u32 = 1;
const EMITTED: &str = "emitted";
const DECODABLE_ONLY: &str = "decodable_only";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ConsumerStatusMetadata {
    status:    String,
    exit_code: u8,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct OutcomeDiscriminant {
    path:  Vec<String>,
    value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct OutcomeRule {
    verb:            String,
    status:          String,
    exit_code:       u8,
    payload_kind:    String,
    data_policy:     String,
    discriminants:   Vec<OutcomeDiscriminant>,
    required_paths:  Vec<Vec<String>>,
    forbidden_paths: Vec<Vec<String>>,
    emission:        String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ConsumerMetadata {
    verbs:                              Vec<String>,
    statuses:                           Vec<ConsumerStatusMetadata>,
    outcomes:                           Vec<OutcomeRule>,
    replay_failures:                    Vec<ConsumerReplayFailureMetadata>,
    integration_proofs:                 Vec<String>,
    trunk_at_claim_alternatives:        Vec<String>,
    lost_integration_evidence_statuses: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ConsumerReplayFailureMetadata {
    reason:       String,
    subject_kind: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ConsumerArtifacts {
    envelope_validation_jq: String,
    status_payload_tables:  String,
}

#[derive(Debug, Deserialize)]
struct CheckedContract {
    consumer_metadata:  ConsumerMetadata,
    consumer_artifacts: ConsumerArtifacts,
    schemas:            GeneratedSchemas,
}

type GeneratedSchemas = BTreeMap<String, Value>;

/// Regenerate the checked-in v1 contract entirely in memory.
pub(crate) fn generate_output_contract() -> Result<String, serde_json::Error> {
    let schemas = generated_schemas()?;
    let consumer_metadata = consumer_metadata(&schemas)?;
    let consumer_artifacts = render_consumer_artifacts(&consumer_metadata, &schemas)?;
    let fixtures = generated_fixtures();
    let contract = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "contract": CONTRACT_NAME,
        "version": CONTRACT_VERSION,
        "consumer_metadata": consumer_metadata,
        "consumer_artifacts": consumer_artifacts,
        "schemas": schemas,
        "wire_inventories": wire_inventories(),
        "fixtures": fixtures,
    });
    let mut serialized = serde_json::to_string_pretty(&contract)?;
    serialized.push('\n');
    Ok(serialized)
}

/// Reproduce both consumer files from a serialized checked-in contract.
pub(crate) fn consumer_artifacts_from_contract(
    serialized_contract: &str,
) -> Result<(String, String), serde_json::Error> {
    let checked_contract = serde_json::from_str::<CheckedContract>(serialized_contract)?;
    let regenerated = render_consumer_artifacts(
        &checked_contract.consumer_metadata,
        &checked_contract.schemas,
    )?;
    Ok((
        regenerated.status_payload_tables,
        regenerated.envelope_validation_jq,
    ))
}

/// Read the consumer bytes embedded for installation after reproducibility validation.
pub(crate) fn embedded_consumer_artifacts(
    serialized_contract: &str,
) -> Result<(String, String), serde_json::Error> {
    let checked_contract = serde_json::from_str::<CheckedContract>(serialized_contract)?;
    Ok((
        checked_contract.consumer_artifacts.status_payload_tables,
        checked_contract.consumer_artifacts.envelope_validation_jq,
    ))
}

fn generated_schemas() -> Result<GeneratedSchemas, serde_json::Error> {
    let mut schemas = BTreeMap::new();
    schemas.insert(
        "canonical_worktree_root".to_owned(),
        schema_value::<CanonicalWorktreeRoot>()?,
    );
    schemas.insert(
        "coordination_identity_rejection".to_owned(),
        schema_value::<CoordinationIdentityRejection>()?,
    );
    schemas.insert(
        "identity_payload".to_owned(),
        schema_value::<IdentityPayload>()?,
    );
    schemas.insert(
        "integration_evidence_status".to_owned(),
        schema_value::<IntegrationEvidenceStatus>()?,
    );
    schemas.insert(
        "integration_payload".to_owned(),
        schema_value::<IntegrationPayload>()?,
    );
    schemas.insert(
        "integration_proof".to_owned(),
        schema_value::<IntegrationProof>()?,
    );
    schemas.insert(
        "lost_integration_evidence_alert".to_owned(),
        schema_value::<LostIntegrationEvidenceAlert>()?,
    );
    schemas.insert(
        "output_envelope".to_owned(),
        schema_value::<OutputEnvelope>()?,
    );
    schemas.insert(
        "output_facts".to_owned(),
        serde_json::to_value(output_facts_schema())?,
    );
    schemas.insert(
        "protected_reservation_tip".to_owned(),
        schema_value::<ProtectedReservationTip>()?,
    );
    schemas.insert(
        "release_disposition".to_owned(),
        schema_value::<ReleaseDisposition>()?,
    );
    schemas.insert(
        "replay_failure".to_owned(),
        schema_value::<ReplayFailurePayload>()?,
    );
    schemas.insert(
        "reservation_lifecycle".to_owned(),
        schema_value::<ReservationLifecycleSnapshot>()?,
    );
    schemas.insert(
        "reservation_lifecycle_query".to_owned(),
        schema_value::<ReservationLifecycleQueryPayload>()?,
    );
    schemas.insert(
        "resolve_payload".to_owned(),
        schema_value::<ResolvePayload>()?,
    );
    Ok(schemas)
}

fn schema_value<SchemaType: JsonSchema>() -> Result<Value, serde_json::Error> {
    serde_json::to_value(schemars::schema_for!(SchemaType))
}

fn consumer_metadata(
    generated_schemas: &GeneratedSchemas,
) -> Result<ConsumerMetadata, serde_json::Error> {
    with_schema_requirements(
        ConsumerMetadata {
            verbs:                              CommandVerb::ALL
                .into_iter()
                .map(CommandVerb::wire_name)
                .map(str::to_owned)
                .collect(),
            statuses:                           OutputStatus::ALL
                .iter()
                .map(|metadata| ConsumerStatusMetadata {
                    status:    metadata.wire_name.to_owned(),
                    exit_code: u8::from(metadata.exit_code),
                })
                .collect(),
            outcomes:                           outcome_rules(),
            replay_failures:                    ReplayFailureReason::ALL
                .iter()
                .map(|(reason, subject)| ConsumerReplayFailureMetadata {
                    reason:       (*reason).to_owned(),
                    subject_kind: match subject {
                        ReplayFailureSubjectKind::Reservation => "reservation",
                        ReplayFailureSubjectKind::IncursionIncident => "incursion_incident",
                        ReplayFailureSubjectKind::ForcedIntegrationPermit => {
                            "forced_integration_permit"
                        },
                    }
                    .to_owned(),
                })
                .collect(),
            integration_proofs:                 IntegrationProof::ALL
                .iter()
                .copied()
                .map(IntegrationProof::wire_name)
                .map(str::to_owned)
                .collect(),
            trunk_at_claim_alternatives:        TrunkObservationAtClaim::WIRE_ALTERNATIVES
                .iter()
                .copied()
                .map(str::to_owned)
                .collect(),
            lost_integration_evidence_statuses: LostIntegrationEvidenceStatus::ALL
                .iter()
                .copied()
                .map(LostIntegrationEvidenceStatus::wire_name)
                .map(str::to_owned)
                .collect(),
        },
        generated_schemas,
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "one declaration keeps every correlated engine outcome visible and reviewable"
)]
fn outcome_rules() -> Vec<OutcomeRule> {
    let mut rules = Vec::new();
    for command_verb in CommandVerb::ALL {
        for output_status in [
            OutputStatus::Unimplemented,
            OutputStatus::LedgerUnreadable,
            OutputStatus::Unconfigured,
            OutputStatus::InvalidInput,
            OutputStatus::Contention,
        ] {
            rules.push(rule(
                command_verb,
                output_status,
                "no_facts",
                "absent",
                &[],
                &[],
                EMITTED,
            ));
        }
        rules.push(rule(
            command_verb,
            OutputStatus::LedgerUnreadable,
            "replay_failure",
            "object",
            &[],
            &[],
            EMITTED,
        ));
    }

    rules.extend([
        simple_rule(CommandVerb::Board, OutputStatus::BoardReady, "board"),
        simple_rule(CommandVerb::Init, OutputStatus::Initialized, "init"),
        simple_rule(
            CommandVerb::Init,
            OutputStatus::ProjectionRepaired,
            "projection_repair",
        ),
        simple_rule(
            CommandVerb::Init,
            OutputStatus::Reinitialized,
            "reinitialize",
        ),
        simple_rule(
            CommandVerb::Board,
            OutputStatus::TerminalViewFailed,
            "no_facts",
        ),
        payload_status_rule(CommandVerb::Check, OutputStatus::Clear, "check", "clear"),
        payload_status_rule(
            CommandVerb::Check,
            OutputStatus::BlockedByOverlap,
            "check",
            "blocked",
        ),
        payload_status_rule(
            CommandVerb::Claim,
            OutputStatus::Claimed,
            "claim",
            "claimed",
        ),
        payload_status_rule(
            CommandVerb::Claim,
            OutputStatus::ReservationLimitReached,
            "claim",
            "reservation_limit_reached",
        ),
        payload_status_rule(
            CommandVerb::Claim,
            OutputStatus::OrderingEdgeLimitReached,
            "claim",
            "ordering_edge_limit_reached",
        ),
        payload_status_rule(
            CommandVerb::Claim,
            OutputStatus::BlockedByOverlap,
            "claim",
            "blocked",
        ),
        payload_status_rule(
            CommandVerb::Claim,
            OutputStatus::NeedsUserAuthorization,
            "claim",
            "needs_user_authorization",
        ),
        simple_rule(CommandVerb::Drift, OutputStatus::Clear, "drift"),
        simple_rule(CommandVerb::Drift, OutputStatus::Widened, "drift"),
        simple_rule(CommandVerb::Drift, OutputStatus::Incursion, "drift"),
        simple_rule(CommandVerb::Drift, OutputStatus::DriftCollision, "drift"),
        simple_rule(
            CommandVerb::Drift,
            OutputStatus::DriftAttributionRequired,
            "drift",
        ),
        simple_rule(CommandVerb::Drift, OutputStatus::ObjectUnknown, "drift"),
        payload_status_rule(
            CommandVerb::Integrate,
            OutputStatus::Integrated,
            "integrate",
            "integrated",
        ),
        payload_status_rule(
            CommandVerb::Integrate,
            OutputStatus::BlockedByOrdering,
            "integrate",
            "blocked",
        ),
        simple_rule(
            CommandVerb::Integrate,
            OutputStatus::LegacyHookOutdated,
            "no_facts",
        ),
        payload_status_rule(
            CommandVerb::Sequence,
            OutputStatus::Sequenced,
            "sequence",
            "sequenced",
        ),
        payload_status_and_reason_rule(OutputStatus::DuplicateOrderingEdge, "duplicate"),
        payload_status_and_reason_rule(OutputStatus::OrderingCycle, "cycle"),
        payload_status_and_reason_rule(OutputStatus::MissingDeferral, "missing_deferral"),
        payload_status_and_reason_rule(
            OutputStatus::OrderingEdgeLimitReached,
            "ordering_edge_limit_reached",
        ),
        payload_status_and_reason_rule(OutputStatus::InvalidInput, "unknown_endpoint"),
        payload_status_and_reason_rule(OutputStatus::InvalidInput, "same_endpoint"),
        payload_status_and_reason_rule(OutputStatus::InvalidInput, "ambiguous_deferral"),
        payload_status_rule(
            CommandVerb::Release,
            OutputStatus::Outstanding,
            "release",
            "checkpointed",
        ),
        payload_status_rule(
            CommandVerb::Release,
            OutputStatus::Outstanding,
            "release",
            "resnapshotted",
        ),
        release_evidence_rule(OutputStatus::Outstanding, "not_integrated"),
        release_evidence_rule(OutputStatus::Integrated, "integrated"),
        release_evidence_rule(OutputStatus::TrunkRewritten, "trunk_rewritten"),
        release_evidence_rule(OutputStatus::ObjectUnknown, "object_unknown"),
        release_disposition_rule(CommandVerb::Release, OutputStatus::Integrated, "integrated"),
        release_disposition_rule(
            CommandVerb::Release,
            OutputStatus::Integrated,
            "rewritten_integration",
        ),
        release_disposition_rule(CommandVerb::Release, OutputStatus::Released, "abandoned"),
        release_disposition_rule(
            CommandVerb::Release,
            OutputStatus::Released,
            "retired_orphan",
        ),
        payload_status_rule(
            CommandVerb::Resolve,
            OutputStatus::IncursionResolved,
            "resolve",
            "recorded_now",
        ),
        payload_status_rule(
            CommandVerb::Resolve,
            OutputStatus::IncursionResolved,
            "resolve",
            "already_recorded_by_same_coordination_actor",
        ),
        payload_status_rule(
            CommandVerb::Resolve,
            OutputStatus::IncursionResolved,
            "resolve",
            "every_incursion_resolved",
        ),
        rule(
            CommandVerb::Resolve,
            OutputStatus::IncursionResolved,
            "resolve",
            "object",
            &[("data.status", "incursion_resolved")],
            &[],
            DECODABLE_ONLY,
        ),
        payload_status_rule(
            CommandVerb::Resolve,
            OutputStatus::InvalidInput,
            "resolve",
            "already_recorded_by_different_coordination_actor",
        ),
        payload_status_rule(
            CommandVerb::Resolve,
            OutputStatus::Recovered,
            "resolve",
            "recovered",
        ),
        release_disposition_rule(CommandVerb::Resolve, OutputStatus::Integrated, "integrated"),
        release_disposition_rule(
            CommandVerb::Resolve,
            OutputStatus::Integrated,
            "rewritten_integration",
        ),
        release_disposition_rule(CommandVerb::Resolve, OutputStatus::Released, "abandoned"),
        release_disposition_rule(
            CommandVerb::Resolve,
            OutputStatus::Released,
            "retired_orphan",
        ),
        simple_rule(CommandVerb::Renew, OutputStatus::Renewed, "renew"),
        payload_status_rule(
            CommandVerb::Identity,
            OutputStatus::SessionMappingCleared,
            "identity",
            "session_mapping_removed",
        ),
        payload_status_rule(
            CommandVerb::Identity,
            OutputStatus::SessionMappingCleared,
            "identity",
            "session_mapping_already_absent",
        ),
        payload_status_rule(
            CommandVerb::Identity,
            OutputStatus::SessionMappingUnavailable,
            "identity",
            "current_session_unavailable",
        ),
    ]);

    for lifecycle_status in [
        "active",
        "outstanding",
        "released_after_checkpoint",
        "released_without_checkpoint",
    ] {
        rules.push(rule(
            CommandVerb::Board,
            OutputStatus::BoardReady,
            "reservation",
            "object",
            &[("data.lifecycle.status", lifecycle_status)],
            &["data.status"],
            EMITTED,
        ));
    }
    rules.push(rule(
        CommandVerb::Board,
        OutputStatus::InvalidInput,
        "reservation",
        "object",
        &[("data.status", "unknown_reservation")],
        &["data.lifecycle"],
        EMITTED,
    ));

    for rejection_kind in [
        "stale_session_mapping",
        "stale_marker_run",
        "session_worktree_mismatch",
    ] {
        for command_verb in [
            CommandVerb::Check,
            CommandVerb::Claim,
            CommandVerb::Drift,
            CommandVerb::Sequence,
        ] {
            rules.push(rule(
                command_verb,
                OutputStatus::InvalidInput,
                "coordination_identity",
                "object",
                &[("data.kind", rejection_kind)],
                &[],
                EMITTED,
            ));
        }
        rules.push(rule(
            CommandVerb::Integrate,
            OutputStatus::InvalidInput,
            "integrate",
            "object",
            &[
                ("data.status", "rejected"),
                ("data.reason.kind", rejection_kind),
            ],
            &[],
            EMITTED,
        ));
    }

    rules
}

fn simple_rule(
    command_verb: CommandVerb,
    output_status: OutputStatus,
    payload_kind: &'static str,
) -> OutcomeRule {
    let data_policy = if payload_kind == "no_facts" {
        "absent"
    } else {
        "object"
    };
    rule(
        command_verb,
        output_status,
        payload_kind,
        data_policy,
        &[],
        &[],
        EMITTED,
    )
}

fn payload_status_rule(
    command_verb: CommandVerb,
    output_status: OutputStatus,
    payload_kind: &'static str,
    payload_status: &'static str,
) -> OutcomeRule {
    rule(
        command_verb,
        output_status,
        payload_kind,
        "object",
        &[("data.status", payload_status)],
        &[],
        EMITTED,
    )
}

fn payload_status_and_reason_rule(
    output_status: OutputStatus,
    reason_kind: &'static str,
) -> OutcomeRule {
    rule(
        CommandVerb::Sequence,
        output_status,
        "sequence",
        "object",
        &[
            ("data.status", "rejected"),
            ("data.reason.kind", reason_kind),
        ],
        &[],
        EMITTED,
    )
}

fn release_evidence_rule(
    output_status: OutputStatus,
    evidence_status: &'static str,
) -> OutcomeRule {
    rule(
        CommandVerb::Release,
        output_status,
        "release",
        "object",
        &[
            ("data.status", "evidence_revalidated"),
            ("data.evidence.status", evidence_status),
        ],
        &[],
        EMITTED,
    )
}

fn release_disposition_rule(
    command_verb: CommandVerb,
    output_status: OutputStatus,
    disposition_kind: &'static str,
) -> OutcomeRule {
    rule(
        command_verb,
        output_status,
        if command_verb == CommandVerb::Release {
            "release"
        } else {
            "resolve"
        },
        "object",
        &[
            ("data.status", "released"),
            ("data.disposition.kind", disposition_kind),
        ],
        &[],
        EMITTED,
    )
}

fn rule(
    command_verb: CommandVerb,
    output_status: OutputStatus,
    payload_kind: &'static str,
    data_policy: &'static str,
    discriminants: &[(&'static str, &'static str)],
    forbidden_paths: &[&'static str],
    emission: &'static str,
) -> OutcomeRule {
    OutcomeRule {
        verb:            command_verb.wire_name().to_owned(),
        status:          output_status.wire_name().to_owned(),
        exit_code:       u8::from(output_status.exit_code()),
        payload_kind:    payload_kind.to_owned(),
        data_policy:     data_policy.to_owned(),
        discriminants:   discriminants
            .iter()
            .map(|(path, value)| OutcomeDiscriminant {
                path:  path.split('.').map(str::to_owned).collect(),
                value: (*value).to_owned(),
            })
            .collect(),
        required_paths:  Vec::new(),
        forbidden_paths: forbidden_paths
            .iter()
            .map(|path| path.split('.').map(str::to_owned).collect())
            .collect(),
        emission:        emission.to_owned(),
    }
}

fn wire_inventories() -> Value {
    let replay_failures = ReplayFailureReason::ALL
        .iter()
        .map(|(reason, subject)| {
            json!({
                "reason": reason,
                "subject": match subject {
                    ReplayFailureSubjectKind::Reservation => "reservation",
                    ReplayFailureSubjectKind::IncursionIncident => "incursion_incident",
                    ReplayFailureSubjectKind::ForcedIntegrationPermit => {
                        "forced_integration_permit"
                    },
                },
                "effect": "hard_stop",
            })
        })
        .collect::<Vec<_>>();
    json!({
        "journal_operations": JournalOperation::WIRE_VARIANTS,
        "replay_failures": replay_failures,
        "reservation_edit_blocking_status": {
            "emitted": EditBlockingStatus::ALL
                .iter()
                .copied()
                .map(EditBlockingStatus::wire_name)
                .collect::<Vec<_>>(),
            "reserved": ["reblocked_active_constraint"],
        },
        "integration_proof": IntegrationProof::ALL
            .iter()
            .copied()
            .map(IntegrationProof::wire_name)
            .collect::<Vec<_>>(),
        "trunk_at_claim": TrunkObservationAtClaim::WIRE_ALTERNATIVES,
        "successor_scoped_patch_equivalence_verdict":
            SuccessorScopedPatchEquivalenceVerdict::ALL
                .iter()
                .copied()
                .map(SuccessorScopedPatchEquivalenceVerdict::wire_name)
                .collect::<Vec<_>>(),
        "scoped_patch_equivalence_verdict": ScopedPatchEquivalenceVerdict::ALL
            .iter()
            .copied()
            .map(ScopedPatchEquivalenceVerdict::wire_name)
            .collect::<Vec<_>>(),
        "incursion_commit_origin": IncursionCommitOrigin::ALL
            .iter()
            .copied()
            .map(IncursionCommitOrigin::wire_name)
            .collect::<Vec<_>>(),
        "journal_identity_inputs": {
            "field_presence": "optional",
            "wire_status": "recorded",
            "members": [
                "invocation_directory",
                "cargo_berth_session_id",
                "cargo_berth_run",
                "git_dir",
                "git_common_dir"
            ],
            "invocation_directory_statuses": ["utf8", "too_long", "non_utf8", "unavailable"],
            "environment_statuses": ["unset", "utf8", "too_long", "non_utf8"],
            "unrecorded_is_wire_value": false,
        },
        "lost_integration_evidence_status": LostIntegrationEvidenceStatus::ALL
            .iter()
            .copied()
            .map(LostIntegrationEvidenceStatus::wire_name)
            .collect::<Vec<_>>(),
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "one fixture set keeps valid and invalid correlated outcomes adjacent"
)]
fn generated_fixtures() -> Value {
    let reservation_id = "01991f4d-77d8-7f5f-9a1f-000000000001";
    let incident_id = "01991f4d-77d8-7f5f-9a1f-000000000002";
    let lifecycle_fixtures = [
        json!({"status": "active"}),
        json!({
            "status": "outstanding",
            "protected_tip": "1111111111111111111111111111111111111111",
        }),
        json!({
            "status": "released_after_checkpoint",
            "protected_tip": "1111111111111111111111111111111111111111",
            "disposition": {"kind": "abandoned", "evidence": "fixture decision"},
        }),
        json!({
            "status": "released_without_checkpoint",
            "disposition": {"kind": "retired_orphan", "evidence": "fixture decision"},
        }),
    ];
    let unknown = envelope(
        "board",
        "invalid_input",
        5,
        "reservation",
        json!({"status": "unknown_reservation", "reservation_id": reservation_id}),
    );
    let recorded_now = envelope(
        "resolve",
        "incursion_resolved",
        0,
        "resolve",
        json!({
            "status": "recorded_now",
            "reservation_id": reservation_id,
            "incident_id": incident_id,
        }),
    );
    let foreign_actor = envelope(
        "resolve",
        "invalid_input",
        5,
        "resolve",
        json!({
            "status": "already_recorded_by_different_coordination_actor",
            "reservation_id": reservation_id,
            "incident_id": incident_id,
            "resolving_worktree_id": "01991f4d-77d8-7f5f-9a1f-000000000002",
            "resolving_coordination_run_id": "01991f4d-77d8-7f5f-9a1f-000000000003",
            "resolution_event_id": "01991f4d-77d8-7f5f-9a1f-000000000004",
            "resolved_at": "2026-08-28T00:00:00Z",
        }),
    );
    let mixed_success = envelope(
        "board",
        "board_ready",
        0,
        "reservation",
        json!({
            "reservation_id": reservation_id,
            "status": "unknown_reservation",
            "lifecycle": {"status": "active"},
        }),
    );
    let success_with_foreign_actor = envelope(
        "resolve",
        "incursion_resolved",
        0,
        "resolve",
        foreign_actor["payload"]["data"].clone(),
    );
    let success_with_identity_rejection = envelope(
        "integrate",
        "integrated",
        0,
        "integrate",
        json!({
            "status": "rejected",
            "reason": identity_rejection("stale_session_mapping", reservation_id),
        }),
    );
    let empty_argv = envelope(
        "check",
        "invalid_input",
        5,
        "coordination_identity",
        json!({
            "kind": "stale_session_mapping",
            "coordination_run_id": "01991f4d-77d8-7f5f-9a1f-000000000003",
            "reservation_id": reservation_id,
            "recovery_actions": [{
                "kind": "clear_session_mapping",
                "argv": [],
                "cwd": "/tmp/repository",
            }],
        }),
    );
    let lost_evidence = json!({
        "kind": "lost_integration_evidence",
        "reservation_id": reservation_id,
        "protected_tip": "1111111111111111111111111111111111111111",
        "evidence_status": {"status": "trunk_rewritten"},
        "recovery": {
            "kind": "verify_resolved_trunk",
            "trunk_oid": "2222222222222222222222222222222222222222",
            "action": {"action": "resolve_integrated_as", "reservation_id": reservation_id},
        },
    });
    let mut envelope_lost_evidence = envelope(
        "drift",
        "clear",
        0,
        "drift",
        json!({"comparison": "cheap_delta", "widening": {"status": "not_needed"}, "results": []}),
    );
    envelope_lost_evidence["payload"]["alerts"] =
        json!([{"kind": "lost_integration_evidence", "data": lost_evidence}]);
    let board_lost_evidence = envelope(
        "board",
        "board_ready",
        0,
        "board",
        json!({"alerts": {"entries": [lost_evidence]}}),
    );

    let mut valid = lifecycle_fixtures
        .into_iter()
        .map(|lifecycle| {
            envelope(
                "board",
                "board_ready",
                0,
                "reservation",
                json!({"reservation_id": reservation_id, "lifecycle": lifecycle}),
            )
        })
        .collect::<Vec<_>>();
    valid.extend([
        unknown,
        recorded_now,
        envelope(
            "resolve",
            "incursion_resolved",
            0,
            "resolve",
            json!({
                "status": "already_recorded_by_same_coordination_actor",
                "reservation_id": reservation_id,
                "incident_id": incident_id,
            }),
        ),
        foreign_actor,
        envelope(
            "identity",
            "session_mapping_cleared",
            0,
            "identity",
            json!({"status": "session_mapping_removed"}),
        ),
        envelope(
            "identity",
            "session_mapping_cleared",
            0,
            "identity",
            json!({"status": "session_mapping_already_absent"}),
        ),
        envelope(
            "identity",
            "session_mapping_unavailable",
            5,
            "identity",
            json!({"status": "current_session_unavailable"}),
        ),
        envelope_lost_evidence,
        board_lost_evidence,
        integration_proof_fixture("protected_tip_ancestor"),
        integration_proof_fixture("scoped_patch_equivalent"),
    ]);
    for (reason, subject_kind) in ReplayFailureReason::ALL {
        valid.push(envelope(
            "board",
            "ledger_unreadable",
            4,
            "replay_failure",
            json!({
                "reason": reason,
                "subject": {"kind": replay_subject_wire_name(*subject_kind), "id": reservation_id},
                "effect": "hard_stop",
            }),
        ));
    }

    let mut invalid_integrated_alert = lost_evidence;
    invalid_integrated_alert["evidence_status"]["status"] = json!("integrated");
    let mut invalid_envelope_alert = envelope(
        "drift",
        "clear",
        0,
        "drift",
        json!({"comparison": "cheap_delta", "widening": {"status": "not_needed"}, "results": []}),
    );
    invalid_envelope_alert["payload"]["alerts"] = json!([{
        "kind": "lost_integration_evidence",
        "data": invalid_integrated_alert.clone(),
    }]);
    let invalid_board_alert = envelope(
        "board",
        "board_ready",
        0,
        "board",
        json!({"alerts": {"entries": [invalid_integrated_alert]}}),
    );
    let invalid = vec![
        mixed_success,
        envelope(
            "board",
            "invalid_input",
            5,
            "reservation",
            json!({
                "reservation_id": reservation_id,
                "status": "unknown_reservation",
                "lifecycle": {"status": "active"},
            }),
        ),
        success_with_foreign_actor,
        success_with_identity_rejection,
        empty_argv,
        invalid_envelope_alert,
        invalid_board_alert,
        integration_proof_fixture("unpublished_proof"),
        envelope(
            "resolve",
            "invalid_input",
            5,
            "resolve",
            json!({
                "status": "already_recorded_by_different_coordination_actor",
                "reservation_id": reservation_id,
                "incident_id": incident_id,
            }),
        ),
        envelope(
            "resolve",
            "incursion_resolved",
            0,
            "resolve",
            json!({
                "status": "recorded_now",
                "incident_id": incident_id,
            }),
        ),
        envelope(
            "resolve",
            "incursion_resolved",
            0,
            "resolve",
            json!({
                "status": "recorded_now",
                "reservation_id": reservation_id,
            }),
        ),
    ];
    json!({"valid": valid, "invalid": invalid})
}

fn integration_proof_fixture(proof: &'static str) -> Value {
    envelope(
        "release",
        "integrated",
        0,
        "release",
        json!({
            "status": "evidence_revalidated",
            "reservation_id": "01991f4d-77d8-7f5f-9a1f-000000000001",
            "evidence": {
                "status": "integrated",
                "trunk_oid": "2222222222222222222222222222222222222222",
                "proof": proof,
            },
            "marker": {"status": "already_absent"},
        }),
    )
}

const fn replay_subject_wire_name(subject_kind: ReplayFailureSubjectKind) -> &'static str {
    match subject_kind {
        ReplayFailureSubjectKind::Reservation => "reservation",
        ReplayFailureSubjectKind::IncursionIncident => "incursion_incident",
        ReplayFailureSubjectKind::ForcedIntegrationPermit => "forced_integration_permit",
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "fixture call sites transfer complete payload facts into an owned envelope"
)]
fn envelope(
    verb: &'static str,
    status: &'static str,
    exit_code: u8,
    payload_kind: &'static str,
    data: Value,
) -> Value {
    json!({
        "verb": verb,
        "status": status,
        "exit_code": exit_code,
        "reservations": [],
        "blocked_by": [],
        "message": "generated fixture",
        "payload": {"kind": payload_kind, "data": data, "alerts": []},
    })
}

fn identity_rejection(kind: &'static str, reservation_id: &'static str) -> Value {
    json!({
        "kind": kind,
        "coordination_run_id": "01991f4d-77d8-7f5f-9a1f-000000000003",
        "reservation_id": reservation_id,
        "recovery_actions": [{
            "kind": "clear_session_mapping",
            "argv": ["cargo-berth", "identity", "clear-session", "--json"],
            "cwd": "/tmp/repository",
        }],
    })
}

fn with_schema_requirements(
    mut consumer_metadata: ConsumerMetadata,
    generated_schemas: &GeneratedSchemas,
) -> Result<ConsumerMetadata, serde_json::Error> {
    let output_facts_schema = generated_schemas
        .get("output_facts")
        .ok_or_else(|| contract_generation_error("output_facts schema is missing"))?;
    for outcome_rule in &mut consumer_metadata.outcomes {
        outcome_rule.required_paths =
            required_paths_for_outcome(output_facts_schema, outcome_rule)?;
    }
    Ok(consumer_metadata)
}

fn required_paths_for_outcome(
    output_facts_schema: &Value,
    outcome_rule: &OutcomeRule,
) -> Result<Vec<Vec<String>>, serde_json::Error> {
    let alternatives = output_facts_schema["oneOf"]
        .as_array()
        .ok_or_else(|| contract_generation_error("output_facts schema has no alternatives"))?;
    let payload_schema = alternatives
        .iter()
        .find(|alternative| {
            alternative["properties"]["kind"]["const"].as_str()
                == Some(outcome_rule.payload_kind.as_str())
        })
        .ok_or_else(|| {
            contract_generation_error(format!(
                "output_facts schema has no {:?} payload",
                outcome_rule.payload_kind
            ))
        })?;
    let mut required_paths = payload_schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|member| vec![member.to_owned()])
        .collect::<BTreeSet<_>>();
    let Some(data_schema) = payload_schema["properties"].get("data") else {
        return Ok(required_paths.into_iter().collect());
    };
    let data_discriminants = outcome_rule
        .discriminants
        .iter()
        .filter_map(|discriminant| {
            discriminant
                .path
                .strip_prefix(&["data".to_owned()])
                .map(|path| (path, discriminant.value.as_str()))
        })
        .collect::<Vec<_>>();
    required_paths.extend(required_schema_paths(
        output_facts_schema,
        data_schema,
        &["data".to_owned()],
        &data_discriminants,
    ));
    Ok(required_paths.into_iter().collect())
}

fn required_schema_paths(
    root_schema: &Value,
    schema: &Value,
    prefix: &[String],
    discriminants: &[(&[String], &str)],
) -> BTreeSet<Vec<String>> {
    let schema = resolved_schema(root_schema, schema);
    if let Some(alternatives) = schema["oneOf"]
        .as_array()
        .or_else(|| schema["anyOf"].as_array())
    {
        let mut alternative_requirements = alternatives
            .iter()
            .filter(|alternative| {
                discriminants.iter().all(|(path, expected)| {
                    schema_accepts_discriminant(root_schema, alternative, path, expected)
                })
            })
            .map(|alternative| {
                required_schema_paths(root_schema, alternative, prefix, discriminants)
            });
        let Some(mut shared_requirements) = alternative_requirements.next() else {
            return BTreeSet::new();
        };
        for requirements in alternative_requirements {
            shared_requirements.retain(|path| requirements.contains(path));
        }
        return shared_requirements;
    }

    let mut required_paths = BTreeSet::new();
    if let Some(all_of) = schema["allOf"].as_array() {
        for member_schema in all_of {
            required_paths.extend(required_schema_paths(
                root_schema,
                member_schema,
                prefix,
                discriminants,
            ));
        }
    }
    let Some(required_members) = schema["required"].as_array() else {
        return required_paths;
    };
    for member in required_members.iter().filter_map(Value::as_str) {
        let mut member_path = prefix.to_vec();
        member_path.push(member.to_owned());
        required_paths.insert(member_path.clone());
        if let Some(member_schema) = schema["properties"].get(member) {
            let nested_discriminants = discriminants
                .iter()
                .filter_map(|(path, expected)| {
                    path.strip_prefix(&[member.to_owned()])
                        .map(|nested| (nested, *expected))
                })
                .collect::<Vec<_>>();
            required_paths.extend(required_schema_paths(
                root_schema,
                member_schema,
                &member_path,
                &nested_discriminants,
            ));
        }
    }
    required_paths
}

fn schema_accepts_discriminant(
    root_schema: &Value,
    schema: &Value,
    path: &[String],
    expected: &str,
) -> bool {
    let schema = resolved_schema(root_schema, schema);
    if path.is_empty() {
        return schema["const"]
            .as_str()
            .is_none_or(|value| value == expected);
    }
    if let Some(alternatives) = schema["oneOf"]
        .as_array()
        .or_else(|| schema["anyOf"].as_array())
    {
        return alternatives.iter().any(|alternative| {
            schema_accepts_discriminant(root_schema, alternative, path, expected)
        });
    }
    if let Some(all_of) = schema["allOf"].as_array() {
        return all_of.iter().all(|member_schema| {
            schema_accepts_discriminant(root_schema, member_schema, path, expected)
        });
    }
    schema["properties"]
        .get(&path[0])
        .is_none_or(|member_schema| {
            schema_accepts_discriminant(root_schema, member_schema, &path[1..], expected)
        })
}

fn resolved_schema<'schema>(root_schema: &'schema Value, schema: &'schema Value) -> &'schema Value {
    schema["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| root_schema.pointer(pointer))
        .unwrap_or(schema)
}

fn contract_generation_error(message: impl Into<String>) -> serde_json::Error {
    serde_json::Error::io(std::io::Error::other(message.into()))
}

fn render_consumer_artifacts(
    consumer_metadata: &ConsumerMetadata,
    generated_schemas: &GeneratedSchemas,
) -> Result<ConsumerArtifacts, serde_json::Error> {
    let consumer_metadata = with_schema_requirements(consumer_metadata.clone(), generated_schemas)?;
    Ok(ConsumerArtifacts {
        envelope_validation_jq: render_jq_validator(&consumer_metadata)?,
        status_payload_tables:  render_python_tables(&consumer_metadata),
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "the generated static Python module is clearer as one sequential template"
)]
fn render_python_tables(consumer_metadata: &ConsumerMetadata) -> String {
    let mut rendered = String::from(
        r"# Generated from docs/cargo-berth/generated/output-contract.json.
from __future__ import annotations

from collections.abc import Mapping
from typing import Final, NamedTuple, cast


class OutcomeRule(NamedTuple):
    verb: str
    status: str
    exit_code: int
    payload_kind: str
    data_policy: str
    discriminants: tuple[tuple[tuple[str, ...], str], ...]
    required_paths: tuple[tuple[str, ...], ...]
    forbidden_paths: tuple[tuple[str, ...], ...]
    emission: str


",
    );
    write_python_frozen_set(&mut rendered, "KNOWN_VERBS", &consumer_metadata.verbs);
    let payload_kinds = sorted_unique(
        consumer_metadata
            .outcomes
            .iter()
            .map(|rule| rule.payload_kind.as_str()),
    );
    write_python_frozen_set(&mut rendered, "KNOWN_PAYLOAD_KINDS", &payload_kinds);
    let statuses = consumer_metadata
        .statuses
        .iter()
        .map(|metadata| metadata.status.as_str())
        .collect::<Vec<_>>();
    write_python_frozen_set(&mut rendered, "KNOWN_STATUSES", &statuses);
    write_python_frozen_set(
        &mut rendered,
        "INTEGRATION_PROOFS",
        &consumer_metadata.integration_proofs,
    );
    write_python_frozen_set(
        &mut rendered,
        "TRUNK_AT_CLAIM_ALTERNATIVES",
        &consumer_metadata.trunk_at_claim_alternatives,
    );
    write_python_frozen_set(
        &mut rendered,
        "LOST_INTEGRATION_EVIDENCE_STATUSES",
        &consumer_metadata.lost_integration_evidence_statuses,
    );
    rendered.push_str("REPLAY_FAILURE_SUBJECT_KINDS: Final[dict[str, str]] = {\n");
    for failure in &consumer_metadata.replay_failures {
        let _ = writeln!(
            rendered,
            "    {:?}: {:?},",
            failure.reason, failure.subject_kind
        );
    }
    rendered.push_str("}\n\n");
    write_python_mapping(
        &mut rendered,
        "VERB_PAYLOAD_KINDS",
        consumer_metadata,
        |rule| rule.verb.as_str(),
    );
    write_python_mapping(
        &mut rendered,
        "STATUS_PAYLOAD_KINDS",
        consumer_metadata,
        |rule| rule.status.as_str(),
    );
    rendered.push_str("FIXED_STATUS_EXIT_CODES: Final[dict[str, int]] = {\n");
    for metadata in &consumer_metadata.statuses {
        let _ = writeln!(
            rendered,
            "    {:?}: {},",
            metadata.status, metadata.exit_code
        );
    }
    rendered.push_str("}\n\nOUTCOME_RULES: Final[tuple[OutcomeRule, ...]] = (\n");
    for rule in &consumer_metadata.outcomes {
        let discriminants = python_discriminants(&rule.discriminants);
        let required_paths = python_paths(&rule.required_paths);
        let forbidden_paths = python_paths(&rule.forbidden_paths);
        let _ = writeln!(
            rendered,
            "    OutcomeRule({:?}, {:?}, {}, {:?}, {:?}, {discriminants}, {required_paths}, {forbidden_paths}, {:?}),",
            rule.verb,
            rule.status,
            rule.exit_code,
            rule.payload_kind,
            rule.data_policy,
            rule.emission,
        );
    }
    rendered.push_str(
        r#")

def _value_at(value: object, path: tuple[str, ...]) -> tuple[bool, object]:
    current = value
    for member in path:
        if not isinstance(current, dict):
            return False, None
        mapping = cast(dict[object, object], current)
        if member not in mapping:
            return False, None
        current = mapping[member]
    return True, current


def _nonempty_string(value: object) -> bool:
    return isinstance(value, str) and bool(value)


def _valid_recovery_actions(value: object) -> bool:
    if not isinstance(value, list) or not value:
        return False
    actions = cast(list[object], value)
    for action_value in actions:
        if not isinstance(action_value, dict):
            return False
        action = cast(dict[object, object], action_value)
        argv_value = action.get("argv")
        if not isinstance(argv_value, list) or not argv_value:
            return False
        argv = cast(list[object], argv_value)
        if not all(_nonempty_string(argument) for argument in argv):
            return False
        cwd = action.get("cwd")
        if not isinstance(cwd, str) or not cwd.startswith("/"):
            return False
    return True


def _valid_identity_rejection(value: object) -> bool:
    if not isinstance(value, dict):
        return False
    rejection = cast(dict[object, object], value)
    return (
        rejection.get("kind") in {
            "stale_session_mapping",
            "stale_marker_run",
            "session_worktree_mismatch",
        }
        and _nonempty_string(rejection.get("coordination_run_id"))
        and _valid_recovery_actions(rejection.get("recovery_actions"))
    )


def _valid_lost_integration_evidence(value: object) -> bool:
    if not isinstance(value, dict):
        return False
    alert = cast(dict[object, object], value)
    wrapped_data = alert.get("data")
    if isinstance(wrapped_data, dict):
        alert = cast(dict[object, object], wrapped_data)
    evidence_status = alert.get("evidence_status")
    if not isinstance(evidence_status, dict):
        return False
    evidence = cast(dict[object, object], evidence_status)
    return evidence.get("status") in LOST_INTEGRATION_EVIDENCE_STATUSES


def _valid_nested_contract_values(value: object) -> bool:
    if isinstance(value, list):
        return all(_valid_nested_contract_values(member) for member in cast(list[object], value))
    if not isinstance(value, dict):
        return True
    mapping = cast(dict[object, object], value)
    if mapping.get("kind") == "lost_integration_evidence":
        if not _valid_lost_integration_evidence(mapping):
            return False
    if mapping.get("status") == "integrated" and "proof" in mapping:
        if mapping.get("proof") not in INTEGRATION_PROOFS:
            return False
    if "trunk_at_claim" in mapping:
        trunk_at_claim = mapping["trunk_at_claim"]
        resolved = _nonempty_string(trunk_at_claim)
        unresolved = (
            isinstance(trunk_at_claim, dict)
            and _nonempty_string(cast(dict[object, object], trunk_at_claim).get("reference"))
        )
        if not (resolved or unresolved):
            return False
    return all(_valid_nested_contract_values(member) for member in mapping.values())


def _valid_replay_failure(data: object) -> bool:
    if not isinstance(data, dict):
        return False
    failure = cast(dict[object, object], data)
    reason = failure.get("reason")
    if not isinstance(reason, str):
        return False
    expected_subject = REPLAY_FAILURE_SUBJECT_KINDS.get(reason)
    subject = failure.get("subject")
    if expected_subject is None or not isinstance(subject, dict):
        return False
    typed_subject = cast(dict[object, object], subject)
    return (
        failure.get("effect") == "hard_stop"
        and typed_subject.get("kind") == expected_subject
        and _nonempty_string(typed_subject.get("id"))
    )


def _valid_foreign_actor_resolution(data: object) -> bool:
    if not isinstance(data, dict):
        return False
    resolution = cast(dict[object, object], data)
    if resolution.get("status") != "already_recorded_by_different_coordination_actor":
        return True
    return all(
        _nonempty_string(resolution.get(member))
        for member in (
            "reservation_id",
            "incident_id",
            "resolving_worktree_id",
            "resolving_coordination_run_id",
            "resolution_event_id",
            "resolved_at",
        )
    )


def _valid_special_payload(payload_kind: str, payload: Mapping[str, object]) -> bool:
    data = payload.get("data")
    if not _valid_nested_contract_values(payload):
        return False
    if payload_kind == "replay_failure":
        return _valid_replay_failure(data)
    if payload_kind == "resolve" and not _valid_foreign_actor_resolution(data):
        return False
    if payload_kind == "coordination_identity":
        return _valid_identity_rejection(data)
    if payload_kind == "integrate" and isinstance(data, dict):
        integration = cast(dict[object, object], data)
        if integration.get("status") == "rejected":
            return _valid_identity_rejection(integration.get("reason"))
    return True


def valid_outcome_tuple(
    verb: str,
    status: str,
    exit_code: int,
    payload: Mapping[str, object],
) -> bool:
    payload_kind = payload.get("kind")
    if not isinstance(payload_kind, str):
        return False
    for rule in OUTCOME_RULES:
        if (
            rule.verb != verb
            or rule.status != status
            or rule.exit_code != exit_code
            or rule.payload_kind != payload_kind
        ):
            continue
        has_data = "data" in payload
        if rule.data_policy == "absent" and has_data:
            continue
        if rule.data_policy == "object" and not isinstance(payload.get("data"), dict):
            continue
        if not all(
            _value_at(payload, path) == (True, expected)
            for path, expected in rule.discriminants
        ):
            continue
        if not all(_value_at(payload, path)[0] for path in rule.required_paths):
            continue
        if any(_value_at(payload, path)[0] for path in rule.forbidden_paths):
            continue
        return _valid_special_payload(payload_kind, payload)
    return False
"#,
    );
    rendered
}

fn write_python_frozen_set(rendered: &mut String, name: &str, values: &[impl AsRef<str>]) {
    let _ = writeln!(rendered, "{name}: Final[frozenset[str]] = frozenset({{");
    for value in values {
        let _ = writeln!(rendered, "    {:?},", value.as_ref());
    }
    rendered.push_str("})\n\n");
}

fn write_python_mapping<Key>(
    rendered: &mut String,
    name: &str,
    consumer_metadata: &ConsumerMetadata,
    key: Key,
) where
    Key: Fn(&OutcomeRule) -> &str,
{
    let mut mapping = BTreeMap::<&str, Vec<&str>>::new();
    for rule in &consumer_metadata.outcomes {
        mapping
            .entry(key(rule))
            .or_default()
            .push(&rule.payload_kind);
    }
    let _ = writeln!(rendered, "{name}: Final[dict[str, frozenset[str]]] = {{");
    for (key, values) in mapping {
        let values = sorted_unique(values);
        let _ = writeln!(rendered, "    {key:?}: frozenset({{");
        for value in values {
            let _ = writeln!(rendered, "        {value:?},");
        }
        rendered.push_str("    }),\n");
    }
    rendered.push_str("}\n\n");
}

fn sorted_unique<'value>(values: impl IntoIterator<Item = &'value str>) -> Vec<&'value str> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn python_discriminants(discriminants: &[OutcomeDiscriminant]) -> String {
    let entries = discriminants
        .iter()
        .map(|discriminant| {
            format!(
                "({}, {:?})",
                python_string_tuple(&discriminant.path),
                discriminant.value
            )
        })
        .collect::<Vec<_>>();
    python_tuple(&entries)
}

fn python_paths(paths: &[Vec<String>]) -> String {
    let paths = paths
        .iter()
        .map(|path| python_string_tuple(path))
        .collect::<Vec<_>>();
    python_tuple(&paths)
}

fn python_string_tuple(values: &[String]) -> String {
    let values = values
        .iter()
        .map(|value| format!("{value:?}"))
        .collect::<Vec<_>>();
    python_tuple(&values)
}

fn python_tuple(values: &[String]) -> String {
    match values {
        [] => "()".to_owned(),
        [value] => format!("({value},)"),
        _ => format!("({})", values.join(", ")),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the generated static jq module is clearer as one sequential template"
)]
fn render_jq_validator(consumer_metadata: &ConsumerMetadata) -> Result<String, serde_json::Error> {
    let outcome_rules = serde_json::to_string(&consumer_metadata.outcomes)?;
    let integration_proofs = serde_json::to_string(&consumer_metadata.integration_proofs)?;
    let trunk_at_claim_alternatives =
        serde_json::to_string(&consumer_metadata.trunk_at_claim_alternatives)?;
    let lost_integration_evidence_statuses =
        serde_json::to_string(&consumer_metadata.lost_integration_evidence_statuses)?;
    let replay_failure_subject_kinds = consumer_metadata
        .replay_failures
        .iter()
        .map(|failure| (failure.reason.as_str(), failure.subject_kind.as_str()))
        .collect::<BTreeMap<_, _>>();
    let replay_failure_subject_kinds = serde_json::to_string(&replay_failure_subject_kinds)?;
    Ok(format!(
        "# Generated from docs/cargo-berth/generated/output-contract.json.\n\
def cargo_berth_outcome_rules: {outcome_rules};\n\
def cargo_berth_integration_proofs: {integration_proofs};\n\
def cargo_berth_trunk_at_claim_alternatives: {trunk_at_claim_alternatives};\n\
def cargo_berth_lost_integration_evidence_statuses: {lost_integration_evidence_statuses};\n\
def cargo_berth_replay_failure_subject_kinds: {replay_failure_subject_kinds};\n\
def cargo_berth_nonempty_string: type == \"string\" and length > 0;\n\
def cargo_berth_path_present($path): any(paths; . == $path);\n\
def cargo_berth_valid_recovery_action:\n\
    type == \"object\" and\n\
    (.kind | cargo_berth_nonempty_string) and\n\
    (.argv | type == \"array\" and length > 0) and\n\
    all(.argv[]; cargo_berth_nonempty_string) and\n\
    (.cwd | type == \"string\" and startswith(\"/\"));\n\
def cargo_berth_valid_identity_rejection:\n\
    type == \"object\" and\n\
    (.kind == \"stale_session_mapping\" or\n\
     .kind == \"stale_marker_run\" or\n\
     .kind == \"session_worktree_mismatch\") and\n\
    (.coordination_run_id | cargo_berth_nonempty_string) and\n\
    (.recovery_actions | type == \"array\" and length > 0) and\n\
    all(.recovery_actions[]; cargo_berth_valid_recovery_action);\n\
def cargo_berth_valid_lost_integration_evidence:\n\
    (if has(\"data\") then .data else . end) |\n\
    type == \"object\" and\n\
    (.evidence_status | type == \"object\") and\n\
    .evidence_status.status as $status |\n\
    (cargo_berth_lost_integration_evidence_statuses | index($status)) != null;\n\
def cargo_berth_valid_nested_contract_values:\n\
    if type == \"array\" then\n\
        all(.[]; cargo_berth_valid_nested_contract_values)\n\
    elif type == \"object\" then\n\
        (if .kind? == \"lost_integration_evidence\" then\n\
            cargo_berth_valid_lost_integration_evidence\n\
         else true end) and\n\
        (if .status? == \"integrated\" and has(\"proof\") then\n\
            .proof as $proof |\n\
            (cargo_berth_integration_proofs | index($proof)) != null\n\
         else true end) and\n\
        (if has(\"trunk_at_claim\") then\n\
            (.trunk_at_claim | cargo_berth_nonempty_string) or\n\
            (.trunk_at_claim |\n\
                type == \"object\" and\n\
                (.reference | cargo_berth_nonempty_string))\n\
         else true end) and\n\
        all(.[]; cargo_berth_valid_nested_contract_values)\n\
    else true end;\n\
def cargo_berth_valid_replay_failure:\n\
    type == \"object\" and\n\
    .reason as $reason |\n\
    (cargo_berth_replay_failure_subject_kinds[$reason] // null) as $subject_kind |\n\
    $subject_kind != null and\n\
    .effect == \"hard_stop\" and\n\
    (.subject | type == \"object\") and\n\
    .subject.kind == $subject_kind and\n\
    (.subject.id | cargo_berth_nonempty_string);\n\
def cargo_berth_valid_foreign_actor_resolution:\n\
    if .status? == \"already_recorded_by_different_coordination_actor\" then\n\
        (.reservation_id | cargo_berth_nonempty_string) and\n\
        (.incident_id | cargo_berth_nonempty_string) and\n\
        (.resolving_worktree_id | cargo_berth_nonempty_string) and\n\
        (.resolving_coordination_run_id | cargo_berth_nonempty_string) and\n\
        (.resolution_event_id | cargo_berth_nonempty_string) and\n\
        (.resolved_at | cargo_berth_nonempty_string)\n\
    else true end;\n\
def cargo_berth_valid_special_payload:\n\
    (.payload | cargo_berth_valid_nested_contract_values) and\n\
    (if .payload.kind == \"replay_failure\" then\n\
        (.payload.data | cargo_berth_valid_replay_failure)\n\
     elif .payload.kind == \"resolve\" then\n\
        (.payload.data | cargo_berth_valid_foreign_actor_resolution)\n\
     elif .payload.kind == \"coordination_identity\" then\n\
        (.payload.data | cargo_berth_valid_identity_rejection)\n\
     elif .payload.kind == \"integrate\" and .payload.data.status == \"rejected\" then\n\
        (.payload.data.reason | cargo_berth_valid_identity_rejection)\n\
     else true end);\n\
def cargo_berth_valid_outcome_tuple:\n\
    . as $envelope |\n\
    any(cargo_berth_outcome_rules[];\n\
        . as $rule |\n\
        $rule.verb == $envelope.verb and\n\
        $rule.status == $envelope.status and\n\
        $rule.exit_code == $envelope.exit_code and\n\
        $rule.payload_kind == $envelope.payload.kind and\n\
        (if $rule.data_policy == \"absent\" then\n\
            ($envelope.payload | has(\"data\") | not)\n\
         else ($envelope.payload.data | type == \"object\") end) and\n\
        all($rule.discriminants[];\n\
            . as $discriminant |\n\
            ($envelope.payload | getpath($discriminant.path)) == $discriminant.value) and\n\
        all($rule.required_paths[];\n\
            . as $path |\n\
            ($envelope.payload | cargo_berth_path_present($path))) and\n\
        all($rule.forbidden_paths[];\n\
            . as $path |\n\
            ($envelope.payload | cargo_berth_path_present($path)) | not)\n\
    ) and cargo_berth_valid_special_payload;\n\
def cargo_berth_valid_contract_envelope:\n\
    type == \"object\" and\n\
    (.verb | cargo_berth_nonempty_string) and\n\
    (.status | cargo_berth_nonempty_string) and\n\
    (.exit_code | type == \"number\") and\n\
    (.reservations | type == \"array\") and\n\
    all(.reservations[]; cargo_berth_nonempty_string) and\n\
    (.blocked_by | type == \"array\") and\n\
    all(.blocked_by[]; cargo_berth_nonempty_string) and\n\
    (.message | cargo_berth_nonempty_string) and\n\
    (.payload | type == \"object\") and\n\
    (.payload.alerts | type == \"array\") and\n\
    cargo_berth_valid_outcome_tuple;\n"
    ))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::io::Write as _;
    use std::path::Path;
    use std::process::Command;
    use std::process::Stdio;

    use serde_json::Value;

    use super::CONTRACT_NAME;
    use super::ConsumerMetadata;
    use super::consumer_artifacts_from_contract;
    use super::embedded_consumer_artifacts;
    use super::generate_output_contract;
    use super::resolved_schema;
    use super::schema_value;
    use crate::ledger::JournalOperation;
    use crate::reservation::ReservationLifecycleSnapshot;

    const CHECKED_CONTRACT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/cargo-berth/generated/output-contract.json"
    );

    #[test]
    fn generated_artifacts_are_reproducible_from_the_checked_in_contract() {
        let generated = generate_output_contract().expect("contract generation should succeed");
        if std::env::var_os("CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT").is_some() {
            if let Some(parent) = Path::new(CHECKED_CONTRACT).parent() {
                fs::create_dir_all(parent).expect("generated contract directory should exist");
            }
            fs::write(CHECKED_CONTRACT, &generated)
                .expect("generated contract should be writable when explicitly requested");
        }
        let checked = fs::read_to_string(CHECKED_CONTRACT)
            .expect("the generated output contract should be checked in");
        assert_eq!(generated, checked);

        let regenerated = consumer_artifacts_from_contract(&checked)
            .expect("consumer artifacts should derive from the checked contract");
        let embedded = embedded_consumer_artifacts(&checked)
            .expect("checked contract should embed installation bytes");
        assert_eq!(regenerated, embedded);
    }

    #[test]
    fn generated_contract_covers_required_phase_outcomes() {
        let generated = generate_output_contract().expect("contract generation should succeed");
        let contract: Value =
            serde_json::from_str(&generated).expect("generated contract should be JSON");
        assert_eq!(contract["contract"], CONTRACT_NAME);
        let serialized = generated.as_str();
        for required in [
            "recorded_now",
            "already_recorded_by_same_coordination_actor",
            "already_recorded_by_different_coordination_actor",
            "session_mapping_removed",
            "session_mapping_already_absent",
            "current_session_unavailable",
            "reblocked_active_constraint",
            "scoped_patch_equivalence_checked",
            "successor_scoped_patch_equivalence_checked",
            "protected_tip_ancestor",
            "scoped_patch_equivalent",
            "phase_authored",
            "already_on_trunk",
            "lost_integration_evidence_status",
        ] {
            assert!(serialized.contains(required), "missing {required}");
        }
    }

    #[test]
    fn checked_contract_inventory_tracks_the_journal_declaration() {
        let checked = fs::read_to_string(CHECKED_CONTRACT)
            .expect("the generated output contract should be checked in");
        let contract: Value =
            serde_json::from_str(&checked).expect("checked contract should be JSON");
        let checked_inventory = contract["wire_inventories"]["journal_operations"]
            .as_array()
            .expect("journal inventory should be an array")
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(checked_inventory, JournalOperation::WIRE_VARIANTS);
    }

    #[test]
    fn outcome_rules_cover_every_tagged_payload_variant() {
        let contract = contract_value();
        let output_facts_schema = &contract["schemas"]["output_facts"];
        let outcomes = contract["consumer_metadata"]["outcomes"]
            .as_array()
            .expect("outcomes should be an array");
        let alternatives = output_facts_schema["oneOf"]
            .as_array()
            .expect("output facts should have alternatives");
        for alternative in alternatives {
            let payload_kind = alternative["properties"]["kind"]["const"]
                .as_str()
                .expect("payload alternative should have a kind");
            let Some(data_schema) = alternative["properties"].get("data") else {
                continue;
            };
            let data_schema = resolved_schema(output_facts_schema, data_schema);
            let Some(payload_alternatives) = data_schema["oneOf"]
                .as_array()
                .or_else(|| data_schema["anyOf"].as_array())
            else {
                continue;
            };
            for tag_member in ["status", "kind"] {
                let tags = payload_alternatives
                    .iter()
                    .map(|payload_alternative| {
                        let payload_alternative =
                            resolved_schema(output_facts_schema, payload_alternative);
                        payload_alternative["properties"][tag_member]["const"].as_str()
                    })
                    .collect::<Option<Vec<_>>>();
                let Some(tags) = tags else {
                    continue;
                };
                for tag in tags {
                    assert!(
                        outcomes.iter().any(|outcome| {
                            outcome["payload_kind"] == payload_kind
                                && outcome["discriminants"].as_array().is_some_and(|values| {
                                    values.iter().any(|value| {
                                        value["path"] == serde_json::json!(["data", tag_member])
                                            && value["value"] == tag
                                    })
                                })
                        }),
                        "missing outcome rule for {payload_kind}.{tag_member}={tag}"
                    );
                }
                break;
            }
        }
    }

    #[test]
    fn generated_python_jq_and_rust_consumers_validate_every_fixture() {
        let generated = generate_output_contract().expect("contract generation should succeed");
        let contract =
            serde_json::from_str::<Value>(&generated).expect("generated contract should be JSON");
        let metadata =
            serde_json::from_value::<ConsumerMetadata>(contract["consumer_metadata"].clone())
                .expect("consumer metadata should decode");
        for fixture in contract["fixtures"]["valid"]
            .as_array()
            .expect("valid fixtures should be an array")
        {
            assert!(rust_consumer_accepts(&metadata, fixture));
        }
        for fixture in contract["fixtures"]["invalid"]
            .as_array()
            .expect("invalid fixtures should be an array")
        {
            assert!(!rust_consumer_accepts(&metadata, fixture));
        }

        run_python_fixture_oracle(&generated);
        run_jq_fixture_oracle(&generated);
    }

    #[test]
    fn generated_contract_covers_both_reservation_lifecycle_wire_shapes() {
        let contract = contract_value();
        let outcomes = contract["consumer_metadata"]["outcomes"]
            .as_array()
            .expect("outcomes should be an array");
        for lifecycle_status in [
            "active",
            "outstanding",
            "released_after_checkpoint",
            "released_without_checkpoint",
        ] {
            assert!(outcomes.iter().any(|outcome| {
                outcome["verb"] == "board"
                    && outcome["status"] == "board_ready"
                    && outcome["exit_code"] == 0
                    && outcome["payload_kind"] == "reservation"
                    && outcome["discriminants"].as_array().is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value["value"] == lifecycle_status)
                    })
            }));
        }
        assert!(outcomes.iter().any(|outcome| {
            outcome["verb"] == "board"
                && outcome["status"] == "invalid_input"
                && outcome["exit_code"] == 5
                && outcome["payload_kind"] == "reservation"
                && outcome.to_string().contains("unknown_reservation")
        }));
    }

    #[test]
    fn generated_contract_covers_identity_clear_session_outcomes() {
        let serialized = generate_output_contract().expect("contract generation should succeed");
        for required in [
            "session_mapping_removed",
            "session_mapping_already_absent",
            "current_session_unavailable",
        ] {
            assert!(serialized.contains(required));
        }
    }

    #[test]
    fn generated_contract_covers_coordination_identity_rejections() {
        let contract = contract_value();
        let outcomes = contract["consumer_metadata"]["outcomes"]
            .as_array()
            .expect("outcomes should be an array");
        for rejection in [
            "stale_session_mapping",
            "stale_marker_run",
            "session_worktree_mismatch",
        ] {
            assert!(outcomes.iter().any(|outcome| {
                outcome["payload_kind"] == "coordination_identity"
                    && outcome.to_string().contains(rejection)
            }));
            assert!(outcomes.iter().any(|outcome| {
                outcome["payload_kind"] == "integrate"
                    && outcome["status"] == "invalid_input"
                    && outcome.to_string().contains(rejection)
            }));
        }
    }

    #[test]
    fn generated_contract_covers_both_lost_evidence_wire_forms() {
        let contract = contract_value();
        let valid = contract["fixtures"]["valid"]
            .as_array()
            .expect("valid fixtures should be an array");
        let invalid = contract["fixtures"]["invalid"]
            .as_array()
            .expect("invalid fixtures should be an array");
        assert!(valid.iter().any(|fixture| {
            fixture["payload"]["alerts"]
                .to_string()
                .contains("lost_integration_evidence")
        }));
        assert!(valid.iter().any(|fixture| {
            fixture["payload"]["data"]["alerts"]
                .to_string()
                .contains("lost_integration_evidence")
        }));
        assert_eq!(
            invalid
                .iter()
                .filter(
                    |fixture| fixture.to_string().contains("\"status\":\"integrated\"")
                        && fixture.to_string().contains("lost_integration_evidence")
                )
                .count(),
            2
        );
    }

    #[test]
    fn generated_replay_inventory_is_exhaustive_and_typed() {
        let contract = contract_value();
        let inventory = contract["wire_inventories"]["replay_failures"]
            .as_array()
            .expect("replay inventory should be an array");
        let fixtures = contract["fixtures"]["valid"]
            .as_array()
            .expect("valid fixtures should be an array");
        assert_eq!(inventory.len(), super::ReplayFailureReason::ALL.len());
        for replay_failure in inventory {
            assert_eq!(replay_failure["effect"], "hard_stop");
            assert!(fixtures.iter().any(|fixture| {
                fixture["payload"]["kind"] == "replay_failure"
                    && fixture["payload"]["data"]["reason"] == replay_failure["reason"]
                    && fixture["payload"]["data"]["subject"]["kind"] == replay_failure["subject"]
            }));
        }
    }

    #[test]
    fn renaming_a_rust_type_keeps_generated_artifacts_byte_identical() {
        type RenamedReservationLifecycleDto = ReservationLifecycleSnapshot;
        assert_eq!(
            schema_value::<ReservationLifecycleSnapshot>()
                .expect("reservation lifecycle schema should generate"),
            schema_value::<RenamedReservationLifecycleDto>()
                .expect("renamed lifecycle schema should generate")
        );
        let generated = generate_output_contract().expect("contract generation should succeed");
        assert!(!generated.contains("ReservationLifecycleSnapshot"));
        assert!(generated.contains("reservation_lifecycle"));
    }

    fn contract_value() -> Value {
        serde_json::from_str(
            &generate_output_contract().expect("contract generation should succeed"),
        )
        .expect("generated contract should be JSON")
    }

    fn run_python_fixture_oracle(serialized_contract: &str) {
        let temporary = tempfile::tempdir().expect("temporary oracle directory should exist");
        let table_path = temporary.path().join("status_payload_tables.py");
        let (python_tables, _) = consumer_artifacts_from_contract(serialized_contract)
            .expect("Python tables should derive from the contract");
        fs::write(&table_path, python_tables).expect("Python tables should be writable");
        let program = r#"
import importlib.util
import json
import sys

spec = importlib.util.spec_from_file_location("generated_tables", sys.argv[1])
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
contract = json.load(sys.stdin)
for fixture in contract["fixtures"]["valid"]:
    assert module.valid_outcome_tuple(
        fixture["verb"], fixture["status"], fixture["exit_code"], fixture["payload"]
    )
for fixture in contract["fixtures"]["invalid"]:
    assert not module.valid_outcome_tuple(
        fixture["verb"], fixture["status"], fixture["exit_code"], fixture["payload"]
    )
"#;
        let mut child = Command::new("python3")
            .args(["-c", program])
            .arg(&table_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Python fixture oracle should start");
        child
            .stdin
            .as_mut()
            .expect("Python stdin should be available")
            .write_all(serialized_contract.as_bytes())
            .expect("Python fixture oracle should receive the contract");
        let output = child
            .wait_with_output()
            .expect("Python fixture oracle should finish");
        assert!(
            output.status.success(),
            "Python fixture oracle failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn run_jq_fixture_oracle(serialized_contract: &str) {
        let temporary = tempfile::tempdir().expect("temporary oracle directory should exist");
        let validator_path = temporary.path().join("envelope_validation.jq");
        let (_, mut jq_validator) = consumer_artifacts_from_contract(serialized_contract)
            .expect("jq validator should derive from the contract");
        jq_validator.push_str(
            "\n(all(.fixtures.valid[]; cargo_berth_valid_contract_envelope)) and \
             (all(.fixtures.invalid[]; cargo_berth_valid_contract_envelope | not))\n",
        );
        fs::write(&validator_path, jq_validator).expect("jq validator should be writable");
        let mut child = Command::new("jq")
            .args(["--exit-status", "-f"])
            .arg(&validator_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("jq fixture oracle should start");
        child
            .stdin
            .as_mut()
            .expect("jq stdin should be available")
            .write_all(serialized_contract.as_bytes())
            .expect("jq fixture oracle should receive the contract");
        let output = child
            .wait_with_output()
            .expect("jq fixture oracle should finish");
        assert!(
            output.status.success(),
            "jq fixture oracle failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn rust_consumer_accepts(metadata: &ConsumerMetadata, envelope: &Value) -> bool {
        let Some(payload) = envelope.get("payload") else {
            return false;
        };
        let Some(payload_kind) = payload.get("kind").and_then(Value::as_str) else {
            return false;
        };
        let tuple_matches = metadata.outcomes.iter().any(|rule| {
            envelope.get("verb").and_then(Value::as_str) == Some(rule.verb.as_str())
                && envelope.get("status").and_then(Value::as_str) == Some(rule.status.as_str())
                && envelope.get("exit_code").and_then(Value::as_u64)
                    == Some(u64::from(rule.exit_code))
                && payload_kind == rule.payload_kind
                && match rule.data_policy.as_str() {
                    "absent" => payload.get("data").is_none(),
                    "object" => payload.get("data").is_some_and(Value::is_object),
                    _ => false,
                }
                && rule.discriminants.iter().all(|discriminant| {
                    value_at(payload, &discriminant.path).and_then(Value::as_str)
                        == Some(discriminant.value.as_str())
                })
                && rule
                    .required_paths
                    .iter()
                    .all(|path| value_at(payload, path).is_some())
                && rule
                    .forbidden_paths
                    .iter()
                    .all(|path| value_at(payload, path).is_none())
        });
        tuple_matches
            && valid_nested_contract_values(metadata, payload)
            && match payload_kind {
                "replay_failure" => valid_replay_failure(metadata, &payload["data"]),
                "resolve" => valid_foreign_actor_resolution(&payload["data"]),
                "coordination_identity" => valid_identity_rejection(&payload["data"]),
                "integrate" if payload["data"]["status"] == "rejected" => {
                    valid_identity_rejection(&payload["data"]["reason"])
                },
                _ => true,
            }
    }

    fn value_at<'value>(value: &'value Value, path: &[String]) -> Option<&'value Value> {
        path.iter()
            .try_fold(value, |current, member| current.get(member))
    }

    fn valid_nested_contract_values(metadata: &ConsumerMetadata, value: &Value) -> bool {
        match value {
            Value::Array(values) => values
                .iter()
                .all(|member| valid_nested_contract_values(metadata, member)),
            Value::Object(mapping) => {
                let valid_lost_evidence = mapping.get("kind").and_then(Value::as_str)
                    != Some("lost_integration_evidence")
                    || valid_lost_integration_evidence(metadata, value);
                let valid_proof = mapping.get("status").and_then(Value::as_str)
                    != Some("integrated")
                    || !mapping.contains_key("proof")
                    || mapping
                        .get("proof")
                        .and_then(Value::as_str)
                        .is_some_and(|proof| {
                            metadata
                                .integration_proofs
                                .iter()
                                .any(|known| known == proof)
                        });
                let valid_trunk = mapping.get("trunk_at_claim").is_none_or(|trunk| {
                    trunk.as_str().is_some_and(|value| !value.is_empty())
                        || trunk
                            .get("reference")
                            .and_then(Value::as_str)
                            .is_some_and(|value| !value.is_empty())
                });
                valid_lost_evidence
                    && valid_proof
                    && valid_trunk
                    && mapping
                        .values()
                        .all(|member| valid_nested_contract_values(metadata, member))
            },
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => true,
        }
    }

    fn valid_lost_integration_evidence(metadata: &ConsumerMetadata, alert: &Value) -> bool {
        let alert = alert.get("data").unwrap_or(alert);
        alert["evidence_status"]["status"]
            .as_str()
            .is_some_and(|status| {
                metadata
                    .lost_integration_evidence_statuses
                    .iter()
                    .any(|known| known == status)
            })
    }

    fn valid_replay_failure(metadata: &ConsumerMetadata, failure: &Value) -> bool {
        let Some(reason) = failure["reason"].as_str() else {
            return false;
        };
        let Some(expected_subject) = metadata
            .replay_failures
            .iter()
            .find(|known| known.reason == reason)
            .map(|known| known.subject_kind.as_str())
        else {
            return false;
        };
        failure["effect"] == "hard_stop"
            && failure["subject"]["kind"] == expected_subject
            && failure["subject"]["id"]
                .as_str()
                .is_some_and(|identifier| !identifier.is_empty())
    }

    fn valid_foreign_actor_resolution(resolution: &Value) -> bool {
        if resolution["status"] != "already_recorded_by_different_coordination_actor" {
            return true;
        }
        [
            "reservation_id",
            "incident_id",
            "resolving_worktree_id",
            "resolving_coordination_run_id",
            "resolution_event_id",
            "resolved_at",
        ]
        .into_iter()
        .all(|member| {
            resolution[member]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        })
    }

    fn valid_identity_rejection(rejection: &Value) -> bool {
        [
            "stale_session_mapping",
            "stale_marker_run",
            "session_worktree_mismatch",
        ]
        .contains(&rejection["kind"].as_str().unwrap_or_default())
            && rejection["coordination_run_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && rejection["recovery_actions"]
                .as_array()
                .is_some_and(|actions| {
                    !actions.is_empty()
                        && actions.iter().all(|action| {
                            action["argv"].as_array().is_some_and(|arguments| {
                                !arguments.is_empty()
                                    && arguments.iter().all(|argument| {
                                        argument.as_str().is_some_and(|value| !value.is_empty())
                                    })
                            })
                        })
                })
    }
}
