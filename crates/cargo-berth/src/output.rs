//! The frozen JSON output contract for `cargo-berth`.
//!
//! Every JSON response retains the original six-field envelope and adds one
//! typed `payload` field. Consumers can continue reading the original fields,
//! while newer consumers use `payload` instead of scraping `message`.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;

use schemars::JsonSchema;
use schemars::Schema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

use crate::alert::Alert;
use crate::alert::RecoverabilityVerdict;
use crate::answer::OverlapEscalationPayload;
use crate::answer::PermissiveOverlapAnswer;
use crate::board::BoardModel;
use crate::board::LiveIncursionMembership;
use crate::config::InitializationState;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::drift::DriftEffect;
use crate::drift::DriftPathAttributionOutcome;
use crate::drift::DriftReport;
use crate::drift::IncursionCommit;
use crate::drift::IncursionCommitOrigin;
use crate::drift::PostWriteFreePathProtection;
use crate::drift::ReservationDriftResult;
use crate::edge::EdgeDeclarationRejection;
use crate::edge::EdgeHold;
use crate::edge::EdgeReadiness;
use crate::edge::IntegrationHold;
use crate::edge::OrderingEdge;
use crate::edge::UnintegratedPredecessorEvidence;
use crate::exit::BerthExit;
use crate::gate::IntegrationViolation;
use crate::gate::install::ActiveManagedHookInstallation;
use crate::gate::install::ManagedHookActivationOutcome;
use crate::gate::install::ManagedHookInstallation;
use crate::gate::permit::ForcedIntegrationPermitReplayError;
use crate::ids::CoordinationRunId;
use crate::ids::EventId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::WireOrderedReservationIds;
use crate::ids::WorktreeId;
use crate::ledger::ClaimSource;
use crate::ledger::CollisionPathSet;
use crate::ledger::ForeignReservationIdSet;
use crate::ledger::IncursionIncidentId;
use crate::ledger::LedgerError;
use crate::ledger::LedgerInitialization;
use crate::ledger::MUTATING_VERB_CONTENTION_TOLERANCE;
use crate::ledger::OrderingDirection;
use crate::ledger::ReservationPurpose;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::presentation::EnvelopePresentation;
use crate::presentation::actionable_board_notices_block;
use crate::presentation::ambiguous_first_touch_block;
use crate::presentation::automatic_widening_block;
use crate::presentation::blocked_edit_refusal_block;
use crate::presentation::coordination_identity_block;
use crate::presentation::degraded_session_mapping_block;
use crate::presentation::engine_message_block;
use crate::presentation::lost_integration_evidence_block;
use crate::presentation::orphaned_outstanding_block;
use crate::presentation::outstanding_incursion_block;
use crate::presentation::replay_failure_block;
use crate::presentation::unverifiable_incursion_block;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::LifecycleTransitionError;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationConflict;
use crate::reservation::ReservationLifecycleSnapshot;
use crate::reservation::ReservationReplayError;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;
use crate::session::CurrentSessionMappingRemoval;
use crate::session::SessionIdentityMappingPublication;
use crate::verb::claim::FirstTouchReservationAcquisition;
use crate::verb::claim::FirstTouchReservationAcquisitionKind;

const INITIALIZED_MESSAGE: &str = "Initialized the cargo-berth ledger.";
const PROJECTION_REPAIRED_MESSAGE: &str =
    "Rebuilt reservations.json from journal truth without changing the journal.";
const BOARD_READY_MESSAGE: &str =
    "The reservation board was read. Use `cargo-berth board --json` to inspect it.";
const AMBIGUOUS_RESERVATION_RECOVERY_COMMAND: &str =
    "cargo-berth check --reservation <reservation-id> <path>...";
/// The one sentence a fail-open edit decision states, wherever it is rendered.
pub(crate) const LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE: &str = "cargo-berth could not establish edit safety; editing is allowed because ledger loss fails open.";
const CHECK_INVALID_INPUT_SUMMARY: &str =
    "cargo-berth rejected this edit because it could not accept the request.";
const CHECK_CONTENTION_SUMMARY: &str = "cargo-berth rejected this edit because another cargo-berth operation still holds the ledger lock.";
/// The clause a hook-facing response states when the ledger could not be read at all.
const LEDGER_UNREADABLE_CONDITION: &str = "cargo-berth could not read the reservation ledger";
/// The clause a hook-facing response states when the bounded lock wait ran out.
const LEDGER_LOCK_DEADLINE_CONDITION: &str = "cargo-berth exhausted its ledger-lock deadline";
/// The summary a drift response states when it could not settle which reservation to compare.
const DRIFT_SELECTION_SUMMARY: &str =
    "cargo-berth could not select or validate the drift reservation.";
/// The command a rejected drift selection tells the reader to run by hand.
const DRIFT_SELECTION_RECOVERY: &str = "Run `cargo-berth drift --reservation <id> --json` by hand.";
/// The command an unreadable board ledger tells the reader to run once it is repaired.
const BOARD_LEDGER_RECOVERY: &str =
    "Run `cargo-berth board --json` again after repairing the ledger.";
/// The command an exhausted board lock deadline tells the reader to run once it is free.
const BOARD_CONTENTION_RECOVERY: &str = "Run `cargo-berth board --json` when the ledger is free.";
/// The summary a post-Bash response falls back to when no verb stated its condition.
const UNSTATED_CONDITION_SUMMARY: &str = "cargo-berth could not inspect this Bash call.";
#[cfg(test)]
const UNIMPLEMENTED_MESSAGE: &str = "The reservation engine is not implemented.";

/// The generated-output contract version reported by every response envelope.
pub(crate) const OUTPUT_CONTRACT_VERSION: u32 = 2;

/// The JSON Schema extension that records a failed closed-value selector transform.
pub(crate) const CLOSED_VALUE_SELECTOR_TRANSFORM_FAILURE_KEY: &str =
    "x-cargo-berth-closed-value-selector-transform-failure";

enum PropertySelectorValues<'schema> {
    Open,
    Closed(Vec<&'schema Value>),
}

impl<'schema> From<&'schema Value> for PropertySelectorValues<'schema> {
    fn from(property_schema: &'schema Value) -> Self {
        property_schema
            .get("enum")
            .and_then(Value::as_array)
            .map(|values| values.iter().collect())
            .or_else(|| {
                property_schema
                    .get("oneOf")
                    .and_then(Value::as_array)
                    .and_then(|alternatives| {
                        alternatives
                            .iter()
                            .map(|alternative| alternative.get("const"))
                            .collect()
                    })
            })
            .map_or(Self::Open, Self::Closed)
    }
}

enum ClosedValueSelectorTransformFailure {
    ObjectSchemaUnavailable,
    ObjectTypeRequired,
    ObjectPropertiesUnavailable,
    RequiredPropertiesUnavailable,
    PropertyCardinality { actual: usize },
    SelectorNotRequired { member: String },
    NoKnownValues { member: String },
    NonStringValue { member: String },
    DuplicateKnownValue { member: String, value: String },
}

impl ClosedValueSelectorTransformFailure {
    const fn kind(&self) -> &'static str {
        match self {
            Self::ObjectSchemaUnavailable => "object_schema_unavailable",
            Self::ObjectTypeRequired => "object_type_required",
            Self::ObjectPropertiesUnavailable => "object_properties_unavailable",
            Self::RequiredPropertiesUnavailable => "required_properties_unavailable",
            Self::PropertyCardinality { .. } => "property_cardinality",
            Self::SelectorNotRequired { .. } => "selector_not_required",
            Self::NoKnownValues { .. } => "no_known_values",
            Self::NonStringValue { .. } => "non_string_value",
            Self::DuplicateKnownValue { .. } => "duplicate_known_value",
        }
    }
}

impl From<ClosedValueSelectorTransformFailure> for Schema {
    fn from(failure: ClosedValueSelectorTransformFailure) -> Self {
        let mut details = Map::new();
        details.insert("kind".to_owned(), Value::String(failure.kind().to_owned()));
        match failure {
            ClosedValueSelectorTransformFailure::ObjectSchemaUnavailable
            | ClosedValueSelectorTransformFailure::ObjectTypeRequired
            | ClosedValueSelectorTransformFailure::ObjectPropertiesUnavailable
            | ClosedValueSelectorTransformFailure::RequiredPropertiesUnavailable => {},
            ClosedValueSelectorTransformFailure::PropertyCardinality { actual } => {
                details.insert("actual".to_owned(), Value::from(actual));
            },
            ClosedValueSelectorTransformFailure::SelectorNotRequired { member }
            | ClosedValueSelectorTransformFailure::NoKnownValues { member }
            | ClosedValueSelectorTransformFailure::NonStringValue { member } => {
                details.insert("member".to_owned(), Value::String(member));
            },
            ClosedValueSelectorTransformFailure::DuplicateKnownValue { member, value } => {
                details.insert("member".to_owned(), Value::String(member));
                details.insert("value".to_owned(), Value::String(value));
            },
        }

        let mut failed_schema = Map::new();
        failed_schema.insert("not".to_owned(), Value::Object(Map::new()));
        failed_schema.insert(
            CLOSED_VALUE_SELECTOR_TRANSFORM_FAILURE_KEY.to_owned(),
            Value::Object(details),
        );
        failed_schema.into()
    }
}

/// Express that one inline closed scalar selects its containing object's schema.
///
/// The alternatives come from the scalar schema values, so a producer variant added to the
/// selector enum becomes a schema branch without a separate output-contract inventory edit.
pub(crate) fn closed_value_selects_object_shape(schema: &mut Schema) {
    *schema = match closed_value_selector_object_schema(schema) {
        Ok(transformed_schema) => transformed_schema,
        Err(failure) => failure.into(),
    };
}

fn closed_value_selector_object_schema(
    schema: &Schema,
) -> Result<Schema, ClosedValueSelectorTransformFailure> {
    let schema_object = schema
        .as_object()
        .ok_or(ClosedValueSelectorTransformFailure::ObjectSchemaUnavailable)?;
    match schema_object.get("type") {
        Some(Value::String(schema_type)) if schema_type == "object" => {},
        _ => return Err(ClosedValueSelectorTransformFailure::ObjectTypeRequired),
    }
    let properties = schema_object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or(ClosedValueSelectorTransformFailure::ObjectPropertiesUnavailable)?;
    let required_properties = schema_object
        .get("required")
        .and_then(Value::as_array)
        .ok_or(ClosedValueSelectorTransformFailure::RequiredPropertiesUnavailable)?;
    let selectors = properties
        .iter()
        .filter_map(|(member, property_schema)| {
            match PropertySelectorValues::from(property_schema) {
                PropertySelectorValues::Open => None,
                PropertySelectorValues::Closed(values) => Some((member, values)),
            }
        })
        .collect::<Vec<_>>();
    let [(selector, known_values)] = selectors.as_slice() else {
        return Err(ClosedValueSelectorTransformFailure::PropertyCardinality {
            actual: selectors.len(),
        });
    };
    if !required_properties
        .iter()
        .any(|required| required.as_str() == Some(selector.as_str()))
    {
        return Err(ClosedValueSelectorTransformFailure::SelectorNotRequired {
            member: (*selector).clone(),
        });
    }
    if known_values.is_empty() {
        return Err(ClosedValueSelectorTransformFailure::NoKnownValues {
            member: (*selector).clone(),
        });
    }
    let known_values = known_values
        .iter()
        .map(|known_value| {
            known_value.as_str().ok_or_else(|| {
                ClosedValueSelectorTransformFailure::NonStringValue {
                    member: (*selector).clone(),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut unique_values = BTreeSet::new();
    for known_value in &known_values {
        if !unique_values.insert(*known_value) {
            return Err(ClosedValueSelectorTransformFailure::DuplicateKnownValue {
                member: (*selector).clone(),
                value:  (*known_value).to_owned(),
            });
        }
    }
    let alternatives = known_values
        .into_iter()
        .map(|known_value| {
            let mut alternative_properties = properties.clone();
            alternative_properties.insert(
                (*selector).clone(),
                serde_json::json!({
                    "type": "string",
                    "const": known_value,
                }),
            );
            let mut alternative = schema_object.clone();
            alternative.insert(
                "properties".to_owned(),
                Value::Object(alternative_properties),
            );
            Value::Object(alternative)
        })
        .collect();
    let mut transformed_schema = Map::new();
    transformed_schema.insert("oneOf".to_owned(), Value::Array(alternatives));
    Ok(transformed_schema.into())
}

/// One response from a `cargo-berth` verb.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "output_envelope")]
pub(crate) struct OutputEnvelope {
    /// The generated output contract generation that produced this response.
    output_contract_version: u32,
    /// The verb that produced this response.
    verb:                    CommandVerb,
    /// The response's lifecycle status.
    status:                  OutputStatus,
    /// The process exit status for this response.
    #[schemars(with = "u8")]
    pub(crate) exit_code:    BerthExit,
    /// Reservations relevant to this response.
    #[schemars(with = "Vec<String>")]
    reservations:            Vec<ReservationId>,
    /// Reservations that block this response.
    #[schemars(with = "Vec<String>")]
    blocked_by:              Vec<ReservationId>,
    /// A human-readable explanation of this response.
    message:                 String,
    /// Render-ready output supplied by the engine that decided this response.
    presentation:            EnvelopePresentation,
    /// The verb-keyed facts consumers need without parsing prose.
    payload:                 OutputPayload,
}

/// Trusted rendering produced inside the installed `PostToolUse` engine process.
pub(crate) enum PostToolUseRendering {
    /// The hook has no feedback to publish.
    NoFeedback,
    /// The hook can publish this complete typed feedback without another validator process.
    Feedback { summary: String, detail: String },
    /// What the response carries is decided by the live state of the reported incursions.
    ///
    /// A drift answer alone cannot say whether an incursion it observed still needs an
    /// answer, because the incident may already have been resolved. The engine reads the
    /// board and renders again against that live state before it states anything.
    FeedbackDecidedByLiveIncursionState,
}

/// A verb named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "command_verb")]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommandVerb {
    /// Initialize the shared ledger.
    Init,
    /// Show the reservation board.
    Board,
    /// Check a proposed path footprint.
    Check,
    /// Claim paths for a reservation.
    Claim,
    /// Compare observed changes with one or more active reservations.
    Drift,
    /// Release a reservation at a checkpoint.
    Release,
    /// Record an ordering relationship.
    Sequence,
    /// Integrate a reservation into trunk.
    Integrate,
    /// Resolve a stuck reservation after inspecting its condition.
    Resolve,
    /// Renew a reservation's explicit activity record.
    Renew,
    /// Manage the current process's disposable coordination identity.
    Identity,
}

/// Whether the post-commit hook should stay silent or print a warning.
pub(crate) enum PostCommitRendering {
    /// The full comparison found nothing the hook needs to report.
    Silent,
    /// The hook must print this diagnostic while leaving the commit standing.
    Warning(String),
}

/// The subject whose journal replay failed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "replay_failure_subject")]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
enum ReplayFailureSubject {
    /// A retained reservation or reservation mutation was invalid.
    Reservation(#[schemars(with = "String")] ReservationId),
    /// A retained incursion-incident record was invalid.
    IncursionIncident(#[schemars(with = "String")] IncursionIncidentId),
    /// A retained forced-integration permit record was invalid.
    ForcedIntegrationPermit(#[schemars(with = "String")] ForcedIntegrationPermitId),
}

/// The command-level effect of a replay failure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "replay_failure_effect")]
#[serde(rename_all = "snake_case")]
enum ReplayFailureEffect {
    /// No command can safely continue from the rejected journal sequence.
    HardStop,
}

/// A typed ledger replay failure that does not require parsing `message`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "replay_failure")]
pub(crate) struct ReplayFailurePayload {
    /// The exact replay invariant that rejected the journal.
    reason:  ReplayFailureReason,
    /// The reservation or incursion incident identified by that invariant.
    subject: ReplayFailureSubject,
    /// The command-level consequence shared by every replay failure.
    effect:  ReplayFailureEffect,
}

impl ReplayFailurePayload {
    fn rendered_reason(&self) -> String {
        serde_json::to_string(&self.reason).map_or_else(
            |_| "unknown_replay_failure".to_owned(),
            |serialized| serialized.trim_matches('"').to_owned(),
        )
    }

    fn rendered_subject(&self) -> String {
        match self.subject {
            ReplayFailureSubject::Reservation(reservation_id) => {
                format!("reservation {reservation_id}")
            },
            ReplayFailureSubject::IncursionIncident(incident_id) => {
                format!("incursion incident {incident_id}")
            },
            ReplayFailureSubject::ForcedIntegrationPermit(permit_id) => {
                format!("forced-integration permit {permit_id}")
            },
        }
    }
}

macro_rules! declare_output_contract_metadata {
    (
        statuses {
            $(
                $(#[$status_meta:meta])*
                $status_variant:ident => ($status_wire:literal, $status_exit:ident);
            )+
        }
        reservation_replay_failures {
            $(
                $reservation_failure:ident => $reservation_failure_wire:literal;
            )+
        }
        incident_replay_failures {
            $(
                $incident_failure:ident => $incident_failure_wire:literal;
            )+
        }
        lifecycle_transition_replay_failures {
            $(
                $lifecycle_transition_failure:ident => $lifecycle_transition_failure_wire:literal;
            )+
        }
        permit_replay_failures {
            $(
                $permit_failure:ident => $permit_failure_wire:literal;
            )+
        }
    ) => {
        /// The status named in a JSON response.
        #[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
        #[schemars(rename = "output_status")]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum OutputStatus {
            $($(#[$status_meta])* $status_variant,)+
        }

        /// The exact invariant that stopped reservation replay.
        #[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
        #[schemars(rename = "replay_failure_reason")]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum ReplayFailureReason {
            $($reservation_failure,)+
            $($incident_failure,)+
            $($lifecycle_transition_failure,)+
            $($permit_failure,)+
        }

        impl From<&ReservationReplayError> for ReplayFailurePayload {
            fn from(error: &ReservationReplayError) -> Self {
                let (reason, subject) = match error {
                    $(
                        ReservationReplayError::$reservation_failure(reservation_id) => (
                            ReplayFailureReason::$reservation_failure,
                            ReplayFailureSubject::Reservation(*reservation_id),
                        ),
                    )+
                    $(
                        ReservationReplayError::$incident_failure(incident_id) => (
                            ReplayFailureReason::$incident_failure,
                            ReplayFailureSubject::IncursionIncident(*incident_id),
                        ),
                    )+
                    ReservationReplayError::InvalidLifecycleTransition(
                        reservation_id,
                        transition_failure,
                    ) => {
                        let reason = match transition_failure {
                            $(
                                LifecycleTransitionError::$lifecycle_transition_failure => {
                                    ReplayFailureReason::$lifecycle_transition_failure
                                },
                            )+
                        };
                        (reason, ReplayFailureSubject::Reservation(*reservation_id))
                    },
                };
                Self {
                    reason,
                    subject,
                    effect: ReplayFailureEffect::HardStop,
                }
            }
        }

        impl From<&ForcedIntegrationPermitReplayError> for ReplayFailurePayload {
            fn from(error: &ForcedIntegrationPermitReplayError) -> Self {
                let (reason, permit_id) = match error {
                    $(
                        ForcedIntegrationPermitReplayError::$permit_failure(permit_id) => (
                            ReplayFailureReason::$permit_failure,
                            *permit_id,
                        ),
                    )+
                };
                Self {
                    reason,
                    subject: ReplayFailureSubject::ForcedIntegrationPermit(permit_id),
                    effect: ReplayFailureEffect::HardStop,
                }
            }
        }
    };
}

declare_output_contract_metadata! {
    statuses {
        /// The verb parsed, but no engine stands behind it yet.
        Unimplemented => ("unimplemented", Clear);
        /// The headless board was projected from reconciled journal and repository facts.
        BoardReady => ("board_ready", Clear);
        /// Initialization created or verified the durable coordination resources.
        Initialized => ("initialized", Clear);
        /// Explicit repair rebuilt only the disposable journal projection.
        ProjectionRepaired => ("projection_repaired", Clear);
        /// Confirmed reinitialization discarded the reviewed journal state.
        Reinitialized => ("reinitialized", Clear);
        /// The journal or its projection could not be safely read or published.
        LedgerUnreadable => ("ledger_unreadable", LedgerUnreadable);
        /// The installed reference-transaction hook predates issuing-checkout capture.
        LegacyHookOutdated => ("legacy_hook_outdated", LedgerUnreadable);
        /// This repository has no berth configuration, so it is not participating in coordination.
        Unconfigured => ("unconfigured", LedgerUnreadable);
        /// The board was handed a terminal and the terminal failed.
        TerminalViewFailed => ("terminal_view_failed", TerminalViewFailed);
        /// An overlap-free edit check may proceed.
        Clear => ("clear", Clear);
        /// A new reservation was appended and published.
        Claimed => ("claimed", Clear);
        /// Unreserved changed paths were added to a reservation.
        Widened => ("widened", Clear);
        /// A write entered a foreign edit-blocking reservation.
        Incursion => ("incursion", BlockedByOverlap);
        /// A widening gained a foreign blocker before its lock was acquired.
        DriftCollision => ("drift_collision", BlockedByOverlap);
        /// Unclaimed paths require an explicit reservation attribution.
        DriftAttributionRequired => ("drift_attribution_required", BlockedByOverlap);
        /// Several active reservations are eligible, and no session mapping selects one.
        AmbiguousActiveRunReservations => ("ambiguous_active_run_reservations", BlockedByOverlap);
        /// Repository policy permits no additional live reservations.
        ReservationLimitReached => ("reservation_limit_reached", BlockedByOverlap);
        /// Repository policy permits no additional ordering edges.
        OrderingEdgeLimitReached => ("ordering_edge_limit_reached", BlockedByOrdering);
        /// One or more foreign reservations overlap the requested paths.
        BlockedByOverlap => ("blocked_by_overlap", BlockedByOverlap);
        /// One or more ordering or deferral holds reject integration.
        BlockedByOrdering => ("blocked_by_ordering", BlockedByOrdering);
        /// A permissive overlap answer needs a matching reviewed proposal.
        NeedsUserAuthorization => ("needs_user_authorization", NeedsUserAuthorization);
        /// The caller can correct the request and retry without repairing the ledger.
        InvalidInput => ("invalid_input", UsageError);
        /// Another mutation retained the ledger lock through the retry window.
        Contention => ("contention", BlockedByContention);
        /// A deferral was converted into one durable ordering edge.
        Sequenced => ("sequenced", Clear);
        /// The requested directed edge already exists.
        DuplicateOrderingEdge => ("duplicate_ordering_edge", BlockedByOrdering);
        /// The requested directed edge would make the graph cyclic.
        OrderingCycle => ("ordering_cycle", BlockedByOrdering);
        /// The named reservations have no unresolved deferral to order.
        MissingDeferral => ("missing_deferral", BlockedByOrdering);
        /// The reservation now has a protected checkpoint awaiting integration.
        Outstanding => ("outstanding", Clear);
        /// Current trunk contains the reservation's integration evidence.
        Integrated => ("integrated", Clear);
        /// Current trunk no longer contains previously verified evidence.
        TrunkRewritten => ("trunk_rewritten", Clear);
        /// Git could not resolve an object needed to verify integration.
        ObjectUnknown => ("object_unknown", Clear);
        /// A user-confirmed non-integration disposition ended the reservation.
        Released => ("released", Clear);
        /// A replacement worktree now owns surviving reservation work.
        Recovered => ("recovered", Clear);
        /// A still-live reservation recorded recent activity.
        Renewed => ("renewed", Clear);
        /// The current harness-session mapping was removed or was already absent.
        SessionMappingCleared => ("session_mapping_cleared", Clear);
        /// No harness-session identifier selected a mapping to remove.
        SessionMappingUnavailable => ("session_mapping_unavailable", UsageError);
        /// A user disposition answered one outstanding incursion incident.
        IncursionResolved => ("incursion_resolved", Clear);
    }
    reservation_replay_failures {
        DuplicateClaim => "duplicate_claim";
        UnknownReservation => "unknown_reservation";
        EmptyScopeSet => "empty_scope_set";
        WidenRequiresUnreleased => "widen_requires_unreleased";
        RevisionExhausted => "revision_exhausted";
        IntegrationProofSubjectRevisionExhausted => "integration_proof_subject_revision_exhausted";
        SnapshotStateMismatch => "snapshot_state_mismatch";
        IntegratedReleaseWithoutEvidence => "integrated_release_without_evidence";
        ActiveEvidenceRevalidation => "active_evidence_revalidation";
        ActiveScopedPatchComparison => "active_scoped_patch_comparison";
        IntegrationProofSubjectMismatch => "integration_proof_subject_mismatch";
        DecisionHasNoGitEvidence => "decision_has_no_git_evidence";
        MissingProtectedTip => "missing_protected_tip";
        MissingTrunkSnapshot => "missing_trunk_snapshot";
        WorktreeRelocationMismatch => "worktree_relocation_mismatch";
        WorktreeRebindingMismatch => "worktree_rebinding_mismatch";
        InvalidReplacementDisposition => "invalid_replacement_disposition";
    }
    incident_replay_failures {
        DuplicateIncursionIncident => "duplicate_incursion_incident";
        UnknownIncursionIncident => "unknown_incursion_incident";
        IncursionIncidentAlreadyResolved => "incursion_incident_already_resolved";
    }
    lifecycle_transition_replay_failures {
        CheckpointRequiresActive => "checkpoint_requires_active";
        ResnapshotRequiresOutstanding => "resnapshot_requires_outstanding";
        ReleaseRequiresCheckpoint => "release_requires_checkpoint";
        AlreadyReleased => "already_released";
        SupersededDispositionMismatch => "superseded_disposition_mismatch";
        ReplacementRequiresRelease => "replacement_requires_release";
    }
    permit_replay_failures {
        DuplicatePermit => "duplicate_permit";
        UnknownPermit => "unknown_permit";
        AlreadyConsumed => "already_consumed";
        ReservationMismatch => "reservation_mismatch";
    }
}

/// Structured facts and additive alerts returned inside the typed payload field.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "output_payload")]
struct OutputPayload {
    /// The verb-keyed result whose serialized `kind` and `data` layout is stable.
    #[serde(flatten)]
    facts:  OutputFacts,
    /// Durable coordination alerts relevant to this response.
    #[serde(default)]
    alerts: Vec<Alert>,
}

/// Structured facts that correspond to the response's verb.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "output_facts")]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum OutputFacts {
    /// The operation failed before it could establish any durable facts.
    NoFacts,
    /// A typed invariant violation stopped append-only journal replay.
    ReplayFailure(ReplayFailurePayload),
    /// Facts returned by `init`.
    Init(InitializationPayload),
    /// Facts returned by `init --repair-projection`.
    ProjectionRepair(ProjectionRepairPayload),
    /// Facts returned by confirmed journal reinitialization.
    Reinitialize(ReinitializationPayload),
    /// Facts returned by the headless reservation board.
    Board(Box<BoardModel>),
    /// One reservation's lifecycle or a typed unknown-id rejection.
    Reservation(ReservationLifecycleQueryPayload),
    /// The locked first-touch reservation-selection result.
    FirstTouchReservationSelection(FirstTouchReservationSelectionPayload),
    /// Facts returned by `check`.
    Check(CheckPayload),
    /// Facts returned by `claim`.
    Claim(ClaimPayload),
    /// Facts returned by `drift`.
    Drift(DriftReport),
    /// Facts returned by `release`.
    Release(ReleasePayload),
    /// Facts returned by `sequence`.
    Sequence(SequencePayload),
    /// Facts returned by `integrate`.
    Integrate(IntegrationPayload),
    /// Facts returned by a recovery decision.
    Resolve(ResolvePayload),
    /// Facts returned by a renewal.
    Renew(RenewPayload),
    /// Facts returned by coordination identity management.
    Identity(IdentityPayload),
    /// A shared coordination identity rejection returned by any validating command.
    CoordinationIdentity(CoordinationIdentityRejection),
}

#[cfg(test)]
pub(crate) fn output_facts_schema() -> Schema { schemars::schema_for!(OutputFacts) }

/// The resources an `init` call created or left intact.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "initialization_payload")]
struct InitializationPayload {
    /// Whether initialization created the journal or found an existing one.
    ledger:        InitializationResource,
    /// Whether initialization created the config or left an existing file intact.
    configuration: InitializationResource,
    /// Whether every registered managed hook is now in force.
    hooks:         Vec<InitializedManagedHook>,
}

/// The activation result for one hook in the managed-hook registry.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "initialized_managed_hook")]
struct InitializedManagedHook {
    /// The git hook name from the managed-hook registry.
    name:       String,
    /// Whether the hook will run and how initialization reached that state.
    activation: ManagedHookActivation,
}

/// Whether one managed hook will run after initialization.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "managed_hook_activation")]
#[serde(tag = "status", rename_all = "snake_case")]
enum ManagedHookActivation {
    /// The managed hook is installed and executable.
    Active {
        /// Whether this call installed or retained the managed script.
        installation: ActiveHookInstallation,
    },
    /// The managed hook is not in force.
    Inactive {
        /// Why initialization could not activate this hook.
        reason: ManagedHookInactivity,
    },
}

/// How an active managed hook reached its current state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "active_hook_installation")]
#[serde(rename_all = "snake_case")]
enum ActiveHookInstallation {
    /// This initialization call created the hook.
    Installed,
    /// This initialization call retained or refreshed the managed hook.
    Current,
}

/// Why a managed hook is not in force after initialization.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "managed_hook_inactivity")]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ManagedHookInactivity {
    /// An unrelated hook still owns the hook name.
    PreservedUnmanaged,
    /// Filesystem or git access prevented hook installation.
    InstallationFailed {
        /// The error returned while installing this hook.
        diagnostic: String,
    },
}

/// The explicit guarantee reported after rebuilding the disposable projection.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "projection_repair_payload")]
struct ProjectionRepairPayload {
    /// The only file this operation rebuilt.
    projection: RepairedProjection,
    /// The journal mutation guarantee of explicit projection repair.
    journal:    ProjectionRepairJournalEffect,
}

/// The exact destructive effect of confirmed ledger reinitialization.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "reinitialization_payload")]
struct ReinitializationPayload {
    /// The journal bytes discarded after confirmation.
    discarded_bytes:              u64,
    /// The newline-terminated records present before truncation.
    discarded_complete_records:   u64,
    /// Environment bypass markers retained outside the unreadable journal.
    pending_environment_bypasses: u64,
}

/// The disposable projection rebuilt by explicit repair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "repaired_projection")]
#[serde(rename_all = "snake_case")]
enum RepairedProjection {
    /// `reservations.json` was derived again from complete journal facts.
    ReservationsJsonRebuilt,
}

/// Whether explicit projection repair changed journal truth.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "projection_repair_journal_effect")]
#[serde(rename_all = "snake_case")]
enum ProjectionRepairJournalEffect {
    /// `journal.ndjson` remained byte-identical.
    Unchanged,
}

/// The result of selecting one reservation independently of board placement.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "reservation_lifecycle_query")]
#[serde(untagged)]
pub(crate) enum ReservationLifecycleQueryPayload {
    /// The selected reservation and its current lifecycle.
    Snapshot {
        /// The reservation selected by the caller.
        reservation_id: ReservationId,
        /// Its point-in-time lifecycle reading.
        lifecycle:      ReservationLifecycleSnapshot,
    },
    /// A typed caller-correctable rejection.
    Rejected(ReservationLifecycleQueryRejection),
}

/// Why a named reservation lifecycle query was rejected.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "reservation_lifecycle_query_rejection")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReservationLifecycleQueryRejection {
    /// No retained reservation has this non-recyclable identity.
    UnknownReservation {
        /// The reservation identity supplied by the caller.
        reservation_id: ReservationId,
    },
}

impl ReservationLifecycleQueryRejection {
    const fn reservation_id(self) -> ReservationId {
        match self {
            Self::UnknownReservation { reservation_id } => reservation_id,
        }
    }
}

/// The initialization outcome for one durable resource.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "initialization_resource")]
#[serde(rename_all = "snake_case")]
enum InitializationResource {
    /// This initialization call created the resource.
    Created,
    /// This initialization call retained an existing resource unchanged.
    Existing,
}

/// Typed outcomes returned by the trunk integration gate.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "integration_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum IntegrationPayload {
    /// The selected reservation entered trunk after a clear decision.
    Integrated {
        /// The reservation whose protected work entered trunk.
        reservation_id: ReservationId,
        /// The main object against which the update was validated.
        #[schemars(with = "String")]
        previous:       GitObjectId,
        /// The new main object installed by the update.
        #[schemars(with = "String")]
        proposed:       GitObjectId,
        /// The journal generation validated under the decision lock.
        generation:     ProjectionGeneration,
        /// How gate policy treated the update.
        gate:           IntegratedGateOutcome,
    },
    /// Enforcing policy refused an out-of-order update.
    Blocked {
        /// The reservation the caller asked to integrate.
        reservation_id: ReservationId,
        /// The journal generation validated under the decision lock.
        generation:     ProjectionGeneration,
        /// Every exact hold that prevented integration.
        violations:     Vec<IntegrationViolation>,
    },
    /// Caller identity named a coordination run that no longer owns active work.
    Rejected {
        /// The semantic reason integration could not select active work.
        reason: CoordinationIdentityRejection,
    },
}

/// How a successful integration related to current gate policy.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "integrated_gate_outcome")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum IntegratedGateOutcome {
    /// No integration constraint held the reservation.
    Clear,
    /// Observe-only policy logged holds that enforcing mode would reject.
    Observed {
        /// The holds reported without rejecting the update.
        violations: Vec<IntegrationViolation>,
    },
    /// A one-use permit was issued and consumed by the update.
    Forced {
        /// The durable permit identity.
        permit_id:           ForcedIntegrationPermitId,
        /// The exact holds the user chose to skip.
        skipped_holds:       SkippedIntegrationHoldSet,
        /// Holds on other entering reservations reported by observe-only policy.
        observed_violations: Vec<IntegrationViolation>,
    },
}

/// Typed outcomes returned by `resolve`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "resolve_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ResolvePayload {
    /// A user disposition answered an outstanding incursion incident.
    IncursionResolved {
        /// The reservation whose drift produced the incident.
        reservation_id: ReservationId,
        /// The incident answered by the appended disposition.
        #[schemars(with = "String")]
        incident_id:    IncursionIncidentId,
    },
    /// This invocation appended the requested incursion disposition.
    RecordedNow {
        /// The reservation whose drift produced the incident.
        reservation_id: ReservationId,
        /// The incident answered by this invocation.
        #[schemars(with = "String")]
        incident_id:    IncursionIncidentId,
    },
    /// This worktree coordination run had already appended the disposition.
    AlreadyRecordedBySameCoordinationActor {
        /// The reservation whose drift produced the incident.
        reservation_id: ReservationId,
        /// The incident already answered by this coordination actor.
        #[schemars(with = "String")]
        incident_id:    IncursionIncidentId,
    },
    /// Another worktree coordination run had already appended the disposition.
    AlreadyRecordedByDifferentCoordinationActor {
        /// The reservation whose drift produced the incident.
        reservation_id:                ReservationId,
        /// The incident already answered by another coordination actor.
        #[schemars(with = "String")]
        incident_id:                   IncursionIncidentId,
        /// The worktree identity recorded on the resolution event.
        resolving_worktree_id:         WorktreeId,
        /// The coordination run recorded on the resolution event.
        resolving_coordination_run_id: CoordinationRunId,
        /// The journal append that answered the incident.
        resolution_event_id:           EventId,
        /// When the disposition was recorded.
        #[schemars(with = "String")]
        resolved_at:                   RecordedAt,
    },
    /// A user disposition answered every incident outstanding for one reservation.
    EveryIncursionResolved {
        /// The reservation whose drift produced the incidents.
        reservation_id: ReservationId,
        /// Every incident answered by the appended dispositions.
        #[schemars(with = "Vec<String>")]
        incident_ids:   Vec<IncursionIncidentId>,
    },
    /// Surviving work moved to a replacement worktree identity.
    Recovered {
        /// The reservation whose holder changed.
        reservation_id: ReservationId,
        /// The opaque identity of the replacement worktree.
        worktree_id:    WorktreeId,
    },
    /// A user-confirmed terminal disposition resolved the reservation.
    Released {
        /// The reservation that received the disposition.
        reservation_id:              ReservationId,
        /// The recorded disposition or replacement disposition.
        disposition:                 ReleaseDisposition,
        /// Whether the harness session mapping retired this reservation.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
}

/// Typed facts returned by `renew`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "renew_payload")]
struct RenewPayload {
    /// The reservation whose activity timestamp advanced.
    reservation_id: ReservationId,
}

/// Typed outcomes returned by `identity`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "identity_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum IdentityPayload {
    /// The current harness session's mapping was removed.
    SessionMappingRemoved,
    /// The current harness session had no stored mapping.
    SessionMappingAlreadyAbsent,
    /// The process supplied no usable harness session identifier.
    CurrentSessionUnavailable,
}

impl From<CurrentSessionMappingRemoval> for IdentityPayload {
    fn from(removal: CurrentSessionMappingRemoval) -> Self {
        match removal {
            CurrentSessionMappingRemoval::Removed => Self::SessionMappingRemoved,
            CurrentSessionMappingRemoval::AlreadyAbsent => Self::SessionMappingAlreadyAbsent,
            CurrentSessionMappingRemoval::CurrentSessionUnavailable => {
                Self::CurrentSessionUnavailable
            },
        }
    }
}

/// Typed outcomes returned by `claim`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "claim_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
enum ClaimPayload {
    /// A reservation was appended with this minimal antichain.
    Claimed {
        /// The newly issued reservation identity.
        reservation_id:              ReservationId,
        /// The coordination run that owns the appended reservation.
        coordination_run_id:         CoordinationRunId,
        /// The exact durable footprint.
        scopes:                      ReservationScopeSet,
        /// Whether the worktree marker records `coordination_run_id`.
        marker_publication:          CoordinationRunMarkerPublication,
        /// Whether the harness session mapping reflects this claim.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Foreign holders prevented the append.
    Blocked {
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
    /// A permissive answer was proposed but has not supplied the current exact token.
    NeedsUserAuthorization {
        /// The conflicts, proposed answer, reason, consequence, and proposal token.
        #[serde(flatten)]
        escalation: Box<OverlapEscalationPayload>,
    },
    /// Repository policy rejected another live reservation.
    ReservationLimitReached {
        /// The configured maximum number of nonterminal reservations.
        maximum: u32,
    },
    /// Repository policy rejected another claim-time ordering edge.
    OrderingEdgeLimitReached {
        /// The configured maximum number of durable ordering edges.
        maximum: u32,
    },
}

/// Typed outcomes returned by `sequence`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "sequence_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
enum SequencePayload {
    /// One durable edge was appended by resolving a prior deferral.
    Sequenced {
        /// The complete replayable edge record.
        edge:      OrderingEdge,
        /// The edge state derived from the preceding repository snapshot.
        readiness: EdgeReadiness,
    },
    /// The locked graph rejected the requested relationship.
    Rejected {
        /// The requested predecessor.
        first:  ReservationId,
        /// The requested successor.
        then:   ReservationId,
        /// The semantic reason no edge was appended.
        reason: SequenceRejectionKind,
    },
}

/// A stable semantic rejection returned by `sequence`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "sequence_rejection_kind")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SequenceRejectionKind {
    /// At least one endpoint does not name a retained reservation.
    UnknownEndpoint {
        /// The missing reservation.
        reservation_id: ReservationId,
    },
    /// One reservation was supplied as both endpoints.
    SameEndpoint,
    /// The exact directed edge already exists.
    Duplicate,
    /// The proposed edge would create a directed cycle.
    Cycle,
    /// No unresolved defer answer joins the endpoints.
    MissingDeferral,
    /// Both endpoint directions contain defer answers.
    AmbiguousDeferral,
    /// Repository policy permits no additional ordering edge.
    OrderingEdgeLimitReached {
        /// The configured durable edge maximum.
        maximum: u32,
    },
}

impl From<EdgeDeclarationRejection> for SequenceRejectionKind {
    fn from(rejection: EdgeDeclarationRejection) -> Self {
        match rejection {
            EdgeDeclarationRejection::UnknownEndpoint(reservation_id) => {
                Self::UnknownEndpoint { reservation_id }
            },
            EdgeDeclarationRejection::SameEndpoint => Self::SameEndpoint,
            EdgeDeclarationRejection::Duplicate => Self::Duplicate,
            EdgeDeclarationRejection::Cycle => Self::Cycle,
            EdgeDeclarationRejection::MissingDeferral => Self::MissingDeferral,
            EdgeDeclarationRejection::AmbiguousDeferral => Self::AmbiguousDeferral,
        }
    }
}

impl SequenceRejectionKind {
    fn blocked_by(&self, first: ReservationId, then: ReservationId) -> Vec<ReservationId> {
        match self {
            Self::Duplicate => vec![first],
            Self::Cycle => vec![then],
            Self::UnknownEndpoint { .. }
            | Self::SameEndpoint
            | Self::MissingDeferral
            | Self::AmbiguousDeferral
            | Self::OrderingEdgeLimitReached { .. } => Vec::new(),
        }
    }

    fn response(
        &self,
        first: ReservationId,
        then: ReservationId,
    ) -> (OutputStatus, BerthExit, String) {
        match self {
            Self::Duplicate => (
                OutputStatus::DuplicateOrderingEdge,
                BerthExit::BlockedByOrdering,
                format!("Ordering edge {first} before {then} already exists."),
            ),
            Self::Cycle => (
                OutputStatus::OrderingCycle,
                BerthExit::BlockedByOrdering,
                format!("Ordering edge {first} before {then} would create a cycle."),
            ),
            Self::MissingDeferral => (
                OutputStatus::MissingDeferral,
                BerthExit::BlockedByOrdering,
                format!(
                    "Reservations {first} and {then} have no unresolved defer answer to sequence."
                ),
            ),
            Self::OrderingEdgeLimitReached { maximum } => (
                OutputStatus::OrderingEdgeLimitReached,
                BerthExit::BlockedByOrdering,
                format!("The configured maximum of {maximum} ordering edges has been reached."),
            ),
            Self::UnknownEndpoint { reservation_id } => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                format!("Reservation {reservation_id} does not exist."),
            ),
            Self::SameEndpoint => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                "An ordering edge requires two different reservations.".to_owned(),
            ),
            Self::AmbiguousDeferral => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                format!("Reservations {first} and {then} recorded deferrals in both directions."),
            ),
        }
    }
}

/// Whether the successful claim also published its worktree run marker.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "coordination_run_marker_publication")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CoordinationRunMarkerPublication {
    /// The marker now identifies the run that owns the appended claim.
    Published,
    /// The claim is durable, but the marker could not be updated.
    Unavailable {
        /// The marker publication failure.
        #[schemars(length(min = 1))]
        diagnostic: String,
    },
}

/// Typed outcomes returned by `check`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "check_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
enum CheckPayload {
    /// No foreign live reservation overlaps the requested paths.
    Clear {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes:      ReservationScopeSet,
        /// The complete first-touch result that permits the edit.
        acquisition: FirstTouchReservationAcquisition,
    },
    /// Foreign holders block one or more requested paths.
    Blocked {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes:    ReservationScopeSet,
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
}

/// Typed outcomes returned when first-touch validation must select an active reservation.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "first_touch_reservation_selection_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
enum FirstTouchReservationSelectionPayload {
    /// No usable session mapping distinguishes several eligible active reservations.
    AmbiguousActiveRunReservations {
        /// Every eligible reservation id in ascending deterministic order.
        #[schemars(with = "Vec<String>")]
        candidate_reservation_ids: WireOrderedReservationIds,
    },
}

/// Typed state transitions and evidence results returned by `release`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "release_payload")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReleasePayload {
    /// An active reservation recorded its first protected checkpoint.
    Checkpointed {
        /// The reservation that changed state.
        reservation_id:              ReservationId,
        /// The fixed commit retained for integration checks.
        protected_tip:               ProtectedReservationTip,
        /// The trunk commit observed at checkpoint.
        #[schemars(with = "String")]
        trunk_oid:                   GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:                      CoordinationRunMarkerRetirement,
        /// Whether the harness session mapping retired this reservation.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// A rebased outstanding reservation replaced its protected checkpoint.
    Resnapshotted {
        /// The reservation that changed state.
        reservation_id: ReservationId,
        /// The replacement fixed commit.
        protected_tip:  ProtectedReservationTip,
        /// The trunk commit observed with the replacement.
        #[schemars(with = "String")]
        trunk_oid:      GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A point-in-time git result was appended for hook-safe replay.
    EvidenceRevalidated {
        /// The reservation whose evidence was checked.
        reservation_id: ReservationId,
        /// What current trunk proves.
        evidence:       IntegrationEvidenceStatus,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A verified or user-confirmed disposition was appended.
    Released {
        /// The reservation that received the disposition.
        reservation_id:              ReservationId,
        /// The retained terminal disposition.
        disposition:                 ReleaseDisposition,
        /// What happened to the worktree coordination-run marker.
        marker:                      CoordinationRunMarkerRetirement,
        /// Whether the harness session mapping retired this reservation.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
}

impl ReleasePayload {
    const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::Checkpointed { reservation_id, .. }
            | Self::Resnapshotted { reservation_id, .. }
            | Self::EvidenceRevalidated { reservation_id, .. }
            | Self::Released { reservation_id, .. } => *reservation_id,
        }
    }

    const fn output_status(&self) -> OutputStatus {
        match self {
            Self::Checkpointed { .. } | Self::Resnapshotted { .. } => OutputStatus::Outstanding,
            Self::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::Integrated { .. } => OutputStatus::Integrated,
                IntegrationEvidenceStatus::NotIntegrated => OutputStatus::Outstanding,
                IntegrationEvidenceStatus::TrunkRewritten => OutputStatus::TrunkRewritten,
                IntegrationEvidenceStatus::ObjectUnknown => OutputStatus::ObjectUnknown,
            },
            Self::Released { disposition, .. } => release_disposition_status(disposition),
        }
    }
}

const fn release_disposition_status(disposition: &ReleaseDisposition) -> OutputStatus {
    match disposition {
        ReleaseDisposition::Integrated | ReleaseDisposition::RewrittenIntegration(_) => {
            OutputStatus::Integrated
        },
        ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => {
            OutputStatus::Released
        },
    }
}

/// The ordinary-release decision for the worktree coordination-run marker.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[schemars(rename = "coordination_run_marker_retirement")]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CoordinationRunMarkerRetirement {
    /// The marker still named this run and was removed.
    Removed,
    /// No marker existed when release checked it.
    AlreadyAbsent,
    /// Another active reservation from this run still needs the marker.
    PreservedForActiveReservation,
    /// A newer run owns the marker.
    PreservedDifferentRun,
    /// The stateful check ran outside the reservation's holder worktree.
    PreservedDifferentWorktree,
    /// A malformed marker remains for phase-5 reconciliation.
    PreservedMalformed,
    /// The release fact is durable, but marker access failed.
    Unavailable {
        /// The marker filesystem diagnostic.
        diagnostic: String,
    },
}

impl OutputEnvelope {
    /// Return the process exit status selected by the engine.
    pub(crate) const fn exit_code(&self) -> BerthExit { self.exit_code }

    /// Borrow the render-ready output selected by the engine.
    pub(crate) const fn presentation(&self) -> &EnvelopePresentation { &self.presentation }

    /// Build the response for a verb that has no engine behind it yet.
    #[cfg(test)]
    fn unimplemented(command_verb: CommandVerb) -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    command_verb,
            status:                  OutputStatus::Unimplemented,
            exit_code:               BerthExit::Clear,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 UNIMPLEMENTED_MESSAGE.to_owned(),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::pending(command_verb),
        }
    }

    /// Build a successful headless board response without requiring a terminal.
    pub(crate) fn board(board: BoardModel) -> Self {
        let reservations = board.reservation_ids().into_vec();
        let presentation = board.envelope_presentation();
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Board,
            status: OutputStatus::BoardReady,
            exit_code: BerthExit::Clear,
            reservations,
            blocked_by: Vec::new(),
            message: BOARD_READY_MESSAGE.to_owned(),
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Board(Box::new(board))),
        }
    }

    /// Build a successful placement-independent reservation lifecycle response.
    pub(crate) fn reservation_lifecycle(
        reservation_id: ReservationId,
        reservation_lifecycle_snapshot: ReservationLifecycleSnapshot,
    ) -> Self {
        let presentation = crate::board::reservation_lifecycle_presentation(
            reservation_id,
            &reservation_lifecycle_snapshot,
        );
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Board,
            status: OutputStatus::BoardReady,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message: format!("Reservation {reservation_id} lifecycle was read."),
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Reservation(
                ReservationLifecycleQueryPayload::Snapshot {
                    reservation_id,
                    lifecycle: reservation_lifecycle_snapshot,
                },
            )),
        }
    }

    /// Build a typed rejection for an unknown reservation lifecycle query.
    pub(crate) fn reservation_lifecycle_query_rejected(
        rejection: ReservationLifecycleQueryRejection,
    ) -> Self {
        let reservation_id = rejection.reservation_id();
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    CommandVerb::Board,
            status:                  OutputStatus::InvalidInput,
            exit_code:               BerthExit::UsageError,
            reservations:            vec![reservation_id],
            blocked_by:              Vec::new(),
            message:                 format!("Reservation {reservation_id} does not exist."),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::from_facts(OutputFacts::Reservation(
                ReservationLifecycleQueryPayload::Rejected(rejection),
            )),
        }
    }

    /// Build a successful board response after the terminal view could not open.
    pub(crate) fn board_with_terminal_view_opening_failure(
        board: BoardModel,
        diagnostic: &str,
    ) -> Self {
        let mut output_envelope = Self::board(board);
        output_envelope
            .message
            .push_str("\nThe terminal view could not open: ");
        output_envelope.message.push_str(diagnostic);
        output_envelope
            .message
            .push_str(". Run `cargo-berth board --json` instead.");
        output_envelope
    }

    /// Build an internal-failure response after the terminal board was visible.
    pub(crate) fn terminal_view_failed_after_board_opened(diagnostic: &str) -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    CommandVerb::Board,
            status:                  OutputStatus::TerminalViewFailed,
            exit_code:               BerthExit::TerminalViewFailed,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 format!(
                "The terminal view failed after it opened: {diagnostic}. Run `cargo-berth board --json` instead."
            ),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build the successful response for completed initialization.
    pub(crate) fn initialized(
        initialization: LedgerInitialization,
        hook_installations: &[ManagedHookInstallation],
    ) -> Self {
        let hooks = hook_installations
            .iter()
            .map(InitializedManagedHook::from)
            .collect::<Vec<_>>();
        let message = initialization_message(&hooks);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Init,
            status: OutputStatus::Initialized,
            exit_code: BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message,
            presentation: EnvelopePresentation::NotProvided,
            payload: OutputPayload::from_facts(OutputFacts::Init(InitializationPayload {
                ledger: initialization.ledger.into(),
                configuration: initialization.configuration.into(),
                hooks,
            })),
        }
    }

    /// Build the successful response for an explicit projection-only repair.
    pub(crate) fn projection_repaired() -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    CommandVerb::Init,
            status:                  OutputStatus::ProjectionRepaired,
            exit_code:               BerthExit::Clear,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 PROJECTION_REPAIRED_MESSAGE.to_owned(),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::from_facts(OutputFacts::ProjectionRepair(
                ProjectionRepairPayload {
                    projection: RepairedProjection::ReservationsJsonRebuilt,
                    journal:    ProjectionRepairJournalEffect::Unchanged,
                },
            )),
        }
    }

    /// Build a successful trunk update after its locked gate decision.
    pub(crate) fn integrated(integration_payload: IntegrationPayload) -> Self {
        let IntegrationPayload::Integrated {
            reservation_id,
            gate,
            ..
        } = &integration_payload
        else {
            return Self::invalid_input(
                CommandVerb::Integrate,
                "an integrated response requires an integrated payload",
            );
        };
        let policy = match gate {
            IntegratedGateOutcome::Clear => "the ordering gate was clear",
            IntegratedGateOutcome::Observed { .. } => {
                "observe-only policy reported an ordering hold"
            },
            IntegratedGateOutcome::Forced { permit_id, .. } => {
                let message = format!(
                    "Integrated reservation {reservation_id} using one-use permit {permit_id}."
                );
                let summary = format!("cargo-berth integrated reservation {reservation_id}.");
                let presentation = engine_result_presentation(&summary, &message);
                return Self {
                    output_contract_version: OUTPUT_CONTRACT_VERSION,
                    verb: CommandVerb::Integrate,
                    status: OutputStatus::Integrated,
                    exit_code: BerthExit::Clear,
                    reservations: vec![*reservation_id],
                    blocked_by: Vec::new(),
                    message,
                    presentation,
                    payload: OutputPayload::from_facts(OutputFacts::Integrate(integration_payload)),
                };
            },
        };
        let message = format!("Integrated reservation {reservation_id}; {policy}.");
        let summary = format!("cargo-berth integrated reservation {reservation_id}.");
        let presentation = engine_result_presentation(&summary, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Integrate,
            status: OutputStatus::Integrated,
            exit_code: BerthExit::Clear,
            reservations: vec![*reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Integrate(integration_payload)),
        }
    }

    /// Build an enforcing gate denial with complete reservation and recovery context.
    pub(crate) fn integration_blocked(
        reservation_id: ReservationId,
        generation: ProjectionGeneration,
        violations: Vec<IntegrationViolation>,
    ) -> Self {
        let blocked_by = integration_blockers(&violations).into_vec();
        let message = integration_blocked_message(reservation_id, &violations);
        let summary = format!("cargo-berth refused integration for reservation {reservation_id}.");
        let presentation = engine_result_presentation(&summary, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Integrate,
            status: OutputStatus::BlockedByOrdering,
            exit_code: BerthExit::BlockedByOrdering,
            reservations: vec![reservation_id],
            blocked_by,
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Integrate(
                IntegrationPayload::Blocked {
                    reservation_id,
                    generation,
                    violations,
                },
            )),
        }
    }

    /// Build the result of confirmed journal reinitialization.
    pub(crate) fn reinitialized(
        discarded_bytes: u64,
        discarded_complete_records: u64,
        pending_environment_bypasses: u64,
    ) -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    CommandVerb::Init,
            status:                  OutputStatus::Reinitialized,
            exit_code:               BerthExit::Clear,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 format!(
                "Reinitialized cargo-berth after confirmed order review; discarded {discarded_bytes} journal bytes across {discarded_complete_records} complete record(s). {pending_environment_bypasses} environment bypass marker(s) remain reportable."
            ),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::from_facts(OutputFacts::Reinitialize(
                ReinitializationPayload {
                    discarded_bytes,
                    discarded_complete_records,
                    pending_environment_bypasses,
                },
            )),
        }
    }

    /// Build a ledger-unreadable response without adding a new process outcome.
    pub(crate) fn ledger_unreadable(command_verb: CommandVerb, diagnostic: &str) -> Self {
        let message = format!("The reservation ledger could not be read: {diagnostic}");
        let presentation = hook_facing_presentation(
            command_verb,
            &HookFacingCondition::LedgerUnreadable { message: &message },
        );
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: command_verb,
            status: OutputStatus::LedgerUnreadable,
            exit_code: BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a typed hard-stop response for a rejected reservation replay.
    pub(crate) fn replay_failure(
        command_verb: CommandVerb,
        error: &ReservationReplayError,
    ) -> Self {
        let mut output_envelope = Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    command_verb,
            status:                  OutputStatus::LedgerUnreadable,
            exit_code:               BerthExit::LedgerUnreadable,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 format!(
                "The reservation ledger could not be replayed: {error}"
            ),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::from_facts(OutputFacts::ReplayFailure(
                ReplayFailurePayload::from(error),
            )),
        };
        output_envelope.refresh_post_tool_use_presentation();
        output_envelope
    }

    /// Build a typed hard-stop response for invalid forced-integration permit history.
    pub(crate) fn forced_integration_permit_replay_failure(
        error: &ForcedIntegrationPermitReplayError,
    ) -> Self {
        let mut output_envelope = Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    CommandVerb::Integrate,
            status:                  OutputStatus::LedgerUnreadable,
            exit_code:               BerthExit::LedgerUnreadable,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 format!(
                "The forced-integration permit journal could not be replayed: {error}"
            ),
            presentation:            EnvelopePresentation::NotProvided,
            payload:                 OutputPayload::from_facts(OutputFacts::ReplayFailure(
                ReplayFailurePayload::from(error),
            )),
        };
        output_envelope.refresh_post_tool_use_presentation();
        output_envelope
    }

    /// Build the recovery response for a reference-transaction hook installed before v1.
    pub(crate) fn legacy_hook_outdated() -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:         CommandVerb::Integrate,
            status:       OutputStatus::LegacyHookOutdated,
            exit_code:    BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      "The installed reference-transaction hook is out of date; run `cargo-berth init` to replace it, then retry integration.".to_owned(),
            presentation: EnvelopePresentation::NotProvided,
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a response for a repository that is not participating in coordination.
    pub(crate) fn unconfigured(
        command_verb: CommandVerb,
        expected_configuration_path: &Path,
    ) -> Self {
        let presentation =
            hook_facing_presentation(command_verb, &HookFacingCondition::Unconfigured);

        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: command_verb,
            status: OutputStatus::Unconfigured,
            exit_code: BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message: format!(
                "this repository has no cargo-berth configuration at {}; run `cargo-berth init` to create it",
                expected_configuration_path.display()
            ),
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Convert a ledger failure into the requesting verb's public response.
    pub(crate) fn ledger_error(command_verb: CommandVerb, error: &LedgerError) -> Self {
        Self::ledger_unreadable(command_verb, &error.to_string())
    }

    /// Build the successful result for one appended claim.
    pub(crate) fn claimed(
        reservation_id: ReservationId,
        coordination_run_id: CoordinationRunId,
        scopes: ReservationScopeSet,
        marker_publication: CoordinationRunMarkerPublication,
        session_mapping_publication: SessionIdentityMappingPublication,
    ) -> Self {
        let scope_count = scopes.as_slice().len();
        let message = match (&marker_publication, &session_mapping_publication) {
            (
                CoordinationRunMarkerPublication::Published,
                SessionIdentityMappingPublication::Published,
            ) => {
                format!("Claimed {scope_count} reservation scope(s) as {reservation_id}.")
            },
            (
                CoordinationRunMarkerPublication::Unavailable { diagnostic },
                SessionIdentityMappingPublication::Published,
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the coordination-run marker could not be published: {diagnostic}. Restore coordination run {coordination_run_id} through the process environment before subsequent commands."
            ),
            (
                CoordinationRunMarkerPublication::Published,
                SessionIdentityMappingPublication::ExplicitSelectionAppliesOnlyToCurrentInvocation {
                    ..
                },
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}. The explicit reservation selection applies only to this invocation because no usable harness session id was supplied."
            ),
            (
                CoordinationRunMarkerPublication::Unavailable { diagnostic },
                SessionIdentityMappingPublication::ExplicitSelectionAppliesOnlyToCurrentInvocation {
                    ..
                },
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the coordination-run marker could not be published: {diagnostic}. The explicit reservation selection applies only to this invocation because no usable harness session id was supplied. Restore coordination run {coordination_run_id} through the process environment before subsequent commands."
            ),
            (
                CoordinationRunMarkerPublication::Published,
                SessionIdentityMappingPublication::Unavailable { diagnostic },
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the harness session mapping could not be published: {diagnostic}. Later session-keyed drift checks may require an explicit coordination run and reservation."
            ),
            (
                CoordinationRunMarkerPublication::Unavailable {
                    diagnostic: marker_diagnostic,
                },
                SessionIdentityMappingPublication::Unavailable {
                    diagnostic: session_diagnostic,
                },
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but neither fallback identity publication completed. Coordination-run marker: {marker_diagnostic}. Harness session mapping: {session_diagnostic}. Restore coordination run {coordination_run_id} through the process environment and name reservation {reservation_id} explicitly for later drift checks."
            ),
        };
        let presentation =
            claimed_presentation(reservation_id, &message, &session_mapping_publication);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Claim,
            status: OutputStatus::Claimed,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Claim(ClaimPayload::Claimed {
                reservation_id,
                coordination_run_id,
                scopes,
                marker_publication,
                session_mapping_publication,
            })),
        }
    }

    /// Build a complete drift result with status and process outcome in agreement.
    pub(crate) fn drift(report: DriftReport) -> Self {
        let has_incursion = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Incursion { .. })
            })
        });
        let has_collision = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Collision { .. })
            })
        });
        let has_widen = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Widened { .. })
            })
        });
        let has_unknown_phase_start = report.results.iter().any(|result| {
            matches!(
                result,
                ReservationDriftResult::PhaseStartObjectUnknown { .. }
            )
        });
        let status = if has_incursion
            || matches!(
                &report.path_attribution,
                DriftPathAttributionOutcome::IncursionDetected { .. }
            ) {
            OutputStatus::Incursion
        } else if has_collision {
            OutputStatus::DriftCollision
        } else if has_widen
            || matches!(
                &report.path_attribution,
                DriftPathAttributionOutcome::FirstTouchReserved { .. }
            )
        {
            OutputStatus::Widened
        } else if matches!(
            &report.path_attribution,
            DriftPathAttributionOutcome::Ambiguous { .. }
                | DriftPathAttributionOutcome::CoordinationRunRequired { .. }
        ) {
            OutputStatus::DriftAttributionRequired
        } else if has_unknown_phase_start {
            OutputStatus::ObjectUnknown
        } else {
            OutputStatus::Clear
        };
        let exit_code = if report.has_blocking_effect() {
            BerthExit::BlockedByOverlap
        } else {
            BerthExit::Clear
        };
        let reservations = report.reservation_ids();
        let blocked_by = report.blocking_reservation_ids().into_vec();
        let message = drift_message(&report);
        let mut output_envelope = Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Drift,
            status,
            exit_code,
            reservations,
            blocked_by,
            message,
            presentation: EnvelopePresentation::NotProvided,
            payload: OutputPayload::from_facts(OutputFacts::Drift(report)),
        };
        output_envelope.refresh_post_tool_use_presentation();
        output_envelope
    }

    /// Build a typed rejection when no additional live reservation is permitted.
    pub(crate) fn reservation_limit_reached(maximum: u32) -> Self {
        let message =
            format!("The configured maximum of {maximum} live reservations has been reached.");
        let presentation = engine_result_presentation(&message, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Claim,
            status: OutputStatus::ReservationLimitReached,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::ReservationLimitReached { maximum },
            )),
        }
    }

    /// Build a typed claim rejection when no additional ordering edge is permitted.
    pub(crate) fn claim_ordering_edge_limit_reached(maximum: u32) -> Self {
        let message =
            format!("The configured maximum of {maximum} ordering edges has been reached.");
        let presentation = engine_result_presentation(&message, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Claim,
            status: OutputStatus::OrderingEdgeLimitReached,
            exit_code: BerthExit::BlockedByOrdering,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::OrderingEdgeLimitReached { maximum },
            )),
        }
    }

    /// Build the successful response for a deferral converted into an ordering edge.
    pub(crate) fn sequenced(edge: OrderingEdge, readiness: EdgeReadiness) -> Self {
        let edge_id = edge.edge_id;
        let before = edge.before;
        let after = edge.after;
        let message = format!("Recorded ordering edge {edge_id}: {before} before {after}.");
        let summary = format!("cargo-berth recorded ordering edge {edge_id}.");
        let presentation = engine_result_presentation(&summary, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Sequence,
            status: OutputStatus::Sequenced,
            exit_code: BerthExit::Clear,
            reservations: vec![before, after],
            blocked_by: if readiness.holds_successor() {
                vec![before]
            } else {
                Vec::new()
            },
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Sequence(SequencePayload::Sequenced {
                edge,
                readiness,
            })),
        }
    }

    /// Build a locked semantic rejection for a requested deferral resolution.
    pub(crate) fn sequence_rejected(
        first: ReservationId,
        then: ReservationId,
        reason: SequenceRejectionKind,
    ) -> Self {
        let (status, exit_code, message) = reason.response(first, then);
        let blocked_by = reason.blocked_by(first, then);
        let presentation = engine_result_presentation(
            "cargo-berth did not record the requested ordering edge.",
            &message,
        );
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Sequence,
            status,
            exit_code,
            reservations: vec![first, then],
            blocked_by,
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Sequence(SequencePayload::Rejected {
                first,
                then,
                reason,
            })),
        }
    }

    /// Build an integration rejection that retains the inactive identity source.
    pub(crate) fn integration_rejected(
        reservation_id: ReservationId,
        reason: CoordinationIdentityRejection,
    ) -> Self {
        let message = reason.to_string();
        let summary = format!("cargo-berth rejected integration for reservation {reservation_id}.");
        let presentation = engine_result_presentation(&summary, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Integrate,
            status: OutputStatus::InvalidInput,
            exit_code: BerthExit::UsageError,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Integrate(
                IntegrationPayload::Rejected { reason },
            )),
        }
    }

    /// Build a shared coordination-identity rejection for a validating command.
    pub(crate) fn coordination_identity_rejected(
        command_verb: CommandVerb,
        reason: CoordinationIdentityRejection,
    ) -> Self {
        let reservations = reason.reservation_ids();
        let message = reason.to_string();
        let mut output_envelope = Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: command_verb,
            status: OutputStatus::InvalidInput,
            exit_code: BerthExit::UsageError,
            reservations,
            blocked_by: Vec::new(),
            message,
            presentation: EnvelopePresentation::NotProvided,
            payload: OutputPayload::from_facts(OutputFacts::CoordinationIdentity(reason)),
        };
        output_envelope.refresh_post_tool_use_presentation();
        output_envelope
    }

    /// Build the result of removing only the current harness-session mapping.
    pub(crate) fn current_session_mapping_removed(removal: CurrentSessionMappingRemoval) -> Self {
        let (status, exit_code, message) = match removal {
            CurrentSessionMappingRemoval::Removed => (
                OutputStatus::SessionMappingCleared,
                BerthExit::Clear,
                "Removed the current harness-session mapping. No reservation or edit decision changed.",
            ),
            CurrentSessionMappingRemoval::AlreadyAbsent => (
                OutputStatus::SessionMappingCleared,
                BerthExit::Clear,
                "The current harness session had no stored mapping. No reservation or edit decision changed.",
            ),
            CurrentSessionMappingRemoval::CurrentSessionUnavailable => (
                OutputStatus::SessionMappingUnavailable,
                BerthExit::UsageError,
                "No usable CARGO_BERTH_SESSION_ID selected a session mapping. Run this recovery command from the harness session that supplied the rejected command; no session mapping changed.",
            ),
        };
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Identity,
            status,
            exit_code,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message: message.to_owned(),
            presentation: EnvelopePresentation::NotProvided,
            payload: OutputPayload::from_facts(OutputFacts::Identity(removal.into())),
        }
    }

    /// Build a claim rejection that names every foreign holder.
    pub(crate) fn blocked_claim(conflicts: Vec<ReservationConflict>) -> Self {
        let blocked_by = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        let message = blocked_message(&conflicts);
        let refusal_detail = blocked_claim_refusal_detail(&conflicts);
        let presentation = blocked_edit_refusal_block(&refusal_detail).into();
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Claim,
            status: OutputStatus::BlockedByOverlap,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by,
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Claim(ClaimPayload::Blocked {
                conflicts,
            })),
        }
    }

    /// Build a claim response that requires a second invocation with the current token.
    pub(crate) fn claim_authorization_required(escalation: OverlapEscalationPayload) -> Self {
        let blocked_by = escalation
            .conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        let mut message = format!(
            "User authorization is required before this overlap can be recorded: {}. Review every holder, shared scope, plan, phase, direction, and reason in the payload, then rerun this claim with --proposal '{}'.",
            escalation.consequence, escalation.proposal_token
        );
        let direction = overlap_direction_description(&escalation.answer);
        let holder_material = escalation
            .conflicts
            .iter()
            .map(|conflict| {
                let shared_scopes = conflict
                    .overlapping_scopes
                    .as_slice()
                    .iter()
                    .map(|scope| {
                        let kind = match scope.kind {
                            ScopeKind::File => "file",
                            ScopeKind::Tree => "tree",
                        };
                        format!("{kind}:{}", scope.path)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Holder {}: {}; shared scopes: {}; direction: {}; reason: {}; consequence: {}.",
                    conflict.reservation_id,
                    source_description(&conflict.source),
                    shared_scopes,
                    direction,
                    escalation.authorization_reason,
                    escalation.consequence,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !holder_material.is_empty() {
            message.push('\n');
            message.push_str(&holder_material);
        }
        let presentation = claim_authorization_presentation(&escalation);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Claim,
            status: OutputStatus::NeedsUserAuthorization,
            exit_code: BerthExit::NeedsUserAuthorization,
            reservations: Vec::new(),
            blocked_by,
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::NeedsUserAuthorization {
                    escalation: Box::new(escalation),
                },
            )),
        }
    }

    /// Build a successful edit check whose locked transaction established protection.
    pub(crate) fn clear_check(
        scopes: ReservationScopeSet,
        acquisition: FirstTouchReservationAcquisition,
    ) -> Self {
        let message = match acquisition.kind {
            FirstTouchReservationAcquisitionKind::Appended => {
                "No foreign reservation overlaps the requested paths; a first-touch reservation was acquired."
            },
            FirstTouchReservationAcquisitionKind::Widened => {
                "No foreign reservation overlaps the requested paths; the acting run's first-touch reservation was widened."
            },
            FirstTouchReservationAcquisitionKind::AlreadyHeld => {
                "No foreign reservation overlaps the requested paths; the acting run already holds them."
            },
        };
        let message = message_with_session_mapping_publication(
            message.to_owned(),
            &acquisition.session_mapping_publication,
        );
        let presentation = session_mapping_publication_presentation(
            &message,
            &acquisition.session_mapping_publication,
        );
        let reservation_id = acquisition.reservation_id;
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Check,
            status: OutputStatus::Clear,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Check(CheckPayload::Clear {
                scopes,
                acquisition,
            })),
        }
    }

    /// Build a mutation-free rejection when no mapping selects one active reservation.
    pub(crate) fn ambiguous_active_run_reservations(
        command_verb: CommandVerb,
        candidate_reservation_ids: WireOrderedReservationIds,
    ) -> Self {
        let rendered_candidates = candidate_reservation_ids
            .as_slice()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let message = format!(
            "No usable harness-session mapping selects one active reservation among {rendered_candidates}. Run `{AMBIGUOUS_RESERVATION_RECOVERY_COMMAND}` with one candidate id to select it. No reservation was appended or widened, and no harness-session mapping was published."
        );
        let candidate_reservation_id_strings = candidate_reservation_ids
            .as_slice()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let presentation =
            ambiguous_first_touch_block(&message, &candidate_reservation_id_strings).into();
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: command_verb,
            status: OutputStatus::AmbiguousActiveRunReservations,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: candidate_reservation_ids.as_slice().to_vec(),
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::FirstTouchReservationSelection(
                FirstTouchReservationSelectionPayload::AmbiguousActiveRunReservations {
                    candidate_reservation_ids,
                },
            )),
        }
    }

    /// Build a blocked mutation-free edit check.
    pub(crate) fn blocked_check(
        scopes: ReservationScopeSet,
        conflicts: Vec<ReservationConflict>,
    ) -> Self {
        let blocked_by = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        let message = blocked_message(&conflicts);
        let refusal_detail = blocked_edit_refusal_detail(&scopes, &conflicts);
        let presentation = blocked_edit_refusal_block(&refusal_detail).into();
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Check,
            status: OutputStatus::BlockedByOverlap,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by,
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Check(CheckPayload::Blocked {
                scopes,
                conflicts,
            })),
        }
    }

    /// Build a caller-correctable request rejection.
    pub(crate) fn invalid_input(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    command_verb,
            status:                  OutputStatus::InvalidInput,
            exit_code:               BerthExit::UsageError,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 diagnostic.to_owned(),
            presentation:            hook_facing_presentation(
                command_verb,
                &HookFacingCondition::InvalidInput { diagnostic },
            ),
            payload:                 OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Convert a drift envelope into the commit hook's silent-or-warning behavior.
    pub(crate) fn post_commit_rendering(&self) -> PostCommitRendering {
        match self.status {
            OutputStatus::Clear | OutputStatus::Unconfigured => PostCommitRendering::Silent,
            OutputStatus::Widened | OutputStatus::Incursion | OutputStatus::DriftCollision => {
                PostCommitRendering::Warning(self.message.clone())
            },
            OutputStatus::LedgerUnreadable => PostCommitRendering::Warning(format!(
                "cargo-berth could not check this commit's drift because the ledger was unreadable. {} Run `cargo-berth drift --full` by hand; this commit remains in place.",
                self.message
            )),
            OutputStatus::Contention => PostCommitRendering::Warning(format!(
                "cargo-berth could not check this commit's drift because the ledger lock deadline was exhausted. {} Run `cargo-berth drift --full` by hand; this commit remains in place.",
                self.message
            )),
            OutputStatus::Unimplemented
            | OutputStatus::BoardReady
            | OutputStatus::Initialized
            | OutputStatus::ProjectionRepaired
            | OutputStatus::Reinitialized
            | OutputStatus::LegacyHookOutdated
            | OutputStatus::TerminalViewFailed
            | OutputStatus::Claimed
            | OutputStatus::DriftAttributionRequired
            | OutputStatus::AmbiguousActiveRunReservations
            | OutputStatus::ReservationLimitReached
            | OutputStatus::OrderingEdgeLimitReached
            | OutputStatus::BlockedByOverlap
            | OutputStatus::BlockedByOrdering
            | OutputStatus::NeedsUserAuthorization
            | OutputStatus::InvalidInput
            | OutputStatus::Sequenced
            | OutputStatus::DuplicateOrderingEdge
            | OutputStatus::OrderingCycle
            | OutputStatus::MissingDeferral
            | OutputStatus::Outstanding
            | OutputStatus::Integrated
            | OutputStatus::TrunkRewritten
            | OutputStatus::ObjectUnknown
            | OutputStatus::Released
            | OutputStatus::Recovered
            | OutputStatus::Renewed
            | OutputStatus::SessionMappingCleared
            | OutputStatus::SessionMappingUnavailable
            | OutputStatus::IncursionResolved => PostCommitRendering::Warning(format!(
                "cargo-berth could not complete the post-commit drift check. {} Run `cargo-berth drift --full` by hand; this commit remains in place.",
                self.message
            )),
        }
    }

    /// Build a bounded lock-contention result with retry guidance.
    pub(crate) fn contention(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    command_verb,
            status:                  OutputStatus::Contention,
            exit_code:               BerthExit::BlockedByContention,
            reservations:            Vec::new(),
            blocked_by:              Vec::new(),
            message:                 diagnostic.to_owned(),
            presentation:            hook_facing_presentation(
                command_verb,
                &HookFacingCondition::Contention { diagnostic },
            ),
            payload:                 OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a successful release lifecycle or evidence response.
    pub(crate) fn released(release_payload: ReleasePayload) -> Self {
        let reservation_id = release_payload.reservation_id();
        let status = release_payload.output_status();
        let message = match &release_payload {
            ReleasePayload::Checkpointed {
                protected_tip,
                session_mapping_publication,
                ..
            } => message_with_session_mapping_publication(
                format!(
                    "Reservation {reservation_id} is outstanding at protected tip {protected_tip}."
                ),
                session_mapping_publication,
            ),
            ReleasePayload::Resnapshotted { protected_tip, .. } => {
                format!("Reservation {reservation_id} now retains protected tip {protected_tip}.")
            },
            ReleasePayload::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::NotIntegrated => format!(
                    "Reservation {reservation_id} remains outstanding; its protected tip is not in trunk."
                ),
                IntegrationEvidenceStatus::Integrated { trunk_oid, .. } => format!(
                    "Reservation {reservation_id} has integration evidence in trunk commit {trunk_oid}."
                ),
                IntegrationEvidenceStatus::TrunkRewritten => format!(
                    "Reservation {reservation_id} is blocking again because trunk no longer contains its verified evidence."
                ),
                IntegrationEvidenceStatus::ObjectUnknown => format!(
                    "Reservation {reservation_id} is blocking because git could not resolve its integration evidence."
                ),
            },
            ReleasePayload::Released {
                disposition,
                session_mapping_publication,
                ..
            } => message_with_session_mapping_publication(
                format!("Reservation {reservation_id} recorded disposition {disposition:?}."),
                session_mapping_publication,
            ),
        };
        let summary = format!("cargo-berth updated reservation {reservation_id}.");
        let presentation = engine_result_presentation(&summary, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Release,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Release(release_payload)),
        }
    }

    /// Build a successful recovery response.
    pub(crate) fn resolved(resolve_payload: ResolvePayload) -> Self {
        let (reservation_id, status, message) = match &resolve_payload {
            ResolvePayload::IncursionResolved {
                reservation_id,
                incident_id,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!("Incursion incident {incident_id} is resolved."),
            ),
            ResolvePayload::RecordedNow {
                reservation_id,
                incident_id,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!("Incursion incident {incident_id} was recorded as resolved."),
            ),
            ResolvePayload::AlreadyRecordedBySameCoordinationActor {
                reservation_id,
                incident_id,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!(
                    "Incursion incident {incident_id} was already resolved by this worktree coordination run."
                ),
            ),
            ResolvePayload::AlreadyRecordedByDifferentCoordinationActor {
                reservation_id,
                incident_id,
                resolving_worktree_id,
                resolving_coordination_run_id,
                resolution_event_id,
                resolved_at,
            } => {
                return Self::incursion_resolution_recorded_by_different_actor(
                    *reservation_id,
                    *incident_id,
                    *resolving_worktree_id,
                    *resolving_coordination_run_id,
                    *resolution_event_id,
                    resolved_at.clone(),
                );
            },
            ResolvePayload::EveryIncursionResolved {
                reservation_id,
                incident_ids,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!(
                    "Every incursion incident outstanding for reservation {reservation_id} is resolved: {}.",
                    incident_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            ResolvePayload::Recovered {
                reservation_id,
                worktree_id,
            } => (
                *reservation_id,
                OutputStatus::Recovered,
                format!("Reservation {reservation_id} is recovered in worktree {worktree_id}."),
            ),
            ResolvePayload::Released {
                reservation_id,
                disposition,
                session_mapping_publication,
            } => (
                *reservation_id,
                release_disposition_status(disposition),
                message_with_session_mapping_publication(
                    format!("Reservation {reservation_id} recorded disposition {disposition:?}."),
                    session_mapping_publication,
                ),
            ),
        };
        let summary = format!("cargo-berth resolved reservation {reservation_id}.");
        let presentation = engine_result_presentation(&summary, &message);
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Resolve,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Resolve(resolve_payload)),
        }
    }

    /// Build a typed rejection for an incident resolved by another coordination actor.
    pub(crate) fn incursion_resolution_recorded_by_different_actor(
        reservation_id: ReservationId,
        incident_id: IncursionIncidentId,
        resolving_worktree_id: WorktreeId,
        resolving_coordination_run_id: CoordinationRunId,
        resolution_event_id: EventId,
        resolved_at: RecordedAt,
    ) -> Self {
        let message = format!(
            "Incursion incident {incident_id} was already resolved by worktree {resolving_worktree_id} in coordination run {resolving_coordination_run_id}, event {resolution_event_id} at {resolved_at}."
        );
        let presentation = engine_result_presentation(
            "cargo-berth rejected a duplicate incursion resolution.",
            &message,
        );
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb: CommandVerb::Resolve,
            status: OutputStatus::InvalidInput,
            exit_code: BerthExit::UsageError,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            presentation,
            payload: OutputPayload::from_facts(OutputFacts::Resolve(
                ResolvePayload::AlreadyRecordedByDifferentCoordinationActor {
                    reservation_id,
                    incident_id,
                    resolving_worktree_id,
                    resolving_coordination_run_id,
                    resolution_event_id,
                    resolved_at,
                },
            )),
        }
    }

    /// Build a successful activity-renewal response.
    pub(crate) fn renewed(reservation_id: ReservationId) -> Self {
        Self {
            output_contract_version: OUTPUT_CONTRACT_VERSION,
            verb:                    CommandVerb::Renew,
            status:                  OutputStatus::Renewed,
            exit_code:               BerthExit::Clear,
            reservations:            vec![reservation_id],
            blocked_by:              Vec::new(),
            message:                 format!("Reservation {reservation_id} activity was renewed."),
            presentation:            EnvelopePresentation::nothing_to_show(),
            payload:                 OutputPayload::from_facts(OutputFacts::Renew(RenewPayload {
                reservation_id,
            })),
        }
    }

    /// The verb this response is recorded under.
    #[cfg(test)]
    pub(crate) const fn verb(&self) -> CommandVerb { self.verb }

    /// Attach alerts derived by the reconciliation that preceded this command.
    pub(crate) fn with_alerts(mut self, alerts: Vec<Alert>) -> Self {
        self.payload.alerts = alerts;
        if matches!(self.payload.facts, OutputFacts::Drift(_)) {
            self.refresh_post_tool_use_presentation();
        }
        if matches!(self.verb, CommandVerb::Board) {
            self.presentation = presentation_from_actionable_alerts(&self.payload.alerts);
        }
        self
    }

    fn refresh_post_tool_use_presentation(&mut self) {
        match self.post_tool_use_rendering() {
            PostToolUseRendering::NoFeedback => {
                self.presentation = EnvelopePresentation::nothing_to_show();
            },
            PostToolUseRendering::Feedback { summary, detail } => {
                self.presentation
                    .replace_with(engine_message_block(&summary, &detail));
            },
            PostToolUseRendering::FeedbackDecidedByLiveIncursionState => {
                self.presentation = match &self.payload.facts {
                    OutputFacts::Drift(report) => {
                        drift_non_incursion_presentation(report, &self.payload.alerts)
                    },
                    OutputFacts::NoFacts
                    | OutputFacts::Init(_)
                    | OutputFacts::ProjectionRepair(_)
                    | OutputFacts::Reinitialize(_)
                    | OutputFacts::Board(_)
                    | OutputFacts::Reservation(_)
                    | OutputFacts::ReplayFailure(_)
                    | OutputFacts::FirstTouchReservationSelection(_)
                    | OutputFacts::Check(_)
                    | OutputFacts::Claim(_)
                    | OutputFacts::Release(_)
                    | OutputFacts::Sequence(_)
                    | OutputFacts::Integrate(_)
                    | OutputFacts::Resolve(_)
                    | OutputFacts::Renew(_)
                    | OutputFacts::CoordinationIdentity(_)
                    | OutputFacts::Identity(_) => engine_result_presentation(
                        "cargo-berth rejected an unexpected live-board presentation request.",
                        &self.message,
                    ),
                };
            },
        }
    }

    /// Render the primary result followed by every durable alert as its own line.
    pub(crate) fn render_text(&self) -> String {
        let mut rendered = self.message.clone();
        if let OutputFacts::Board(board) = &self.payload.facts {
            for marker_name in board.recovered_bypass_marker_names() {
                let _ = write!(
                    rendered,
                    "\nRecovered bypass marker {marker_name}: a bypass recorded earlier while the journal was unwritable has now been filed in the journal."
                );
            }
        }
        for alert in &self.payload.alerts {
            rendered.push('\n');
            rendered.push_str(&alert.to_string());
        }
        rendered
    }

    /// State a coordination-identity rejection in the terms of the verb that hit it.
    const fn coordination_identity_summary(&self) -> &'static str {
        match self.verb {
            CommandVerb::Check => {
                "cargo-berth rejected this edit under the current coordination identity."
            },
            CommandVerb::Board => {
                "cargo-berth rejected this SessionStart read under the current coordination identity."
            },
            CommandVerb::Drift => {
                "cargo-berth rejected drift under the current coordination identity."
            },
            CommandVerb::Init
            | CommandVerb::Claim
            | CommandVerb::Release
            | CommandVerb::Sequence
            | CommandVerb::Integrate
            | CommandVerb::Resolve
            | CommandVerb::Renew
            | CommandVerb::Identity => {
                "cargo-berth rejected this command under the current coordination identity."
            },
        }
    }

    /// Render an installed-engine `PostToolUse` result without reparsing its serialized envelope.
    pub(crate) fn post_tool_use_rendering(&self) -> PostToolUseRendering {
        match &self.payload.facts {
            OutputFacts::Drift(report) => self.post_tool_use_drift_rendering(report),
            OutputFacts::ReplayFailure(failure) => {
                let summary = match self.verb {
                    CommandVerb::Check => LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE,
                    CommandVerb::Board => {
                        "cargo-berth stopped on invalid reservation history at SessionStart."
                    },
                    CommandVerb::Drift => {
                        "cargo-berth stopped on invalid reservation history after Bash."
                    },
                    CommandVerb::Init
                    | CommandVerb::Claim
                    | CommandVerb::Release
                    | CommandVerb::Sequence
                    | CommandVerb::Integrate
                    | CommandVerb::Resolve
                    | CommandVerb::Renew
                    | CommandVerb::Identity => {
                        "cargo-berth stopped on invalid reservation history."
                    },
                };
                let block = replay_failure_block(
                    summary,
                    &failure.rendered_reason(),
                    &failure.rendered_subject(),
                );
                PostToolUseRendering::Feedback {
                    summary: block.summary,
                    detail:  block.detail,
                }
            },
            OutputFacts::CoordinationIdentity(rejection) => {
                let recovery_actions = rejection.rendered_recovery_actions();
                let block = coordination_identity_block(
                    self.coordination_identity_summary(),
                    rejection.wire_kind(),
                    &recovery_actions,
                );
                PostToolUseRendering::Feedback {
                    summary: block.summary,
                    detail:  block.detail,
                }
            },
            OutputFacts::FirstTouchReservationSelection(
                FirstTouchReservationSelectionPayload::AmbiguousActiveRunReservations { .. },
            ) => PostToolUseRendering::Feedback {
                summary: "cargo-berth could not select one active reservation for first-touch attribution."
                    .to_owned(),
                detail:  format!("DRIFT ATTRIBUTION REQUIRED: {}", self.message),
            },
            OutputFacts::NoFacts if matches!(self.status, OutputStatus::Unconfigured) => {
                PostToolUseRendering::NoFeedback
            },
            OutputFacts::NoFacts => self.stated_condition_rendering(),
            OutputFacts::Init(_)
            | OutputFacts::ProjectionRepair(_)
            | OutputFacts::Reinitialize(_)
            | OutputFacts::Board(_)
            | OutputFacts::Reservation(_)
            | OutputFacts::Check(_)
            | OutputFacts::Claim(_)
            | OutputFacts::Release(_)
            | OutputFacts::Sequence(_)
            | OutputFacts::Integrate(_)
            | OutputFacts::Resolve(_)
            | OutputFacts::Renew(_)
            | OutputFacts::Identity(_) => PostToolUseRendering::Feedback {
                summary: "cargo-berth rejected an unexpected PostToolUse response.".to_owned(),
                detail:  self.message.clone(),
            },
        }
    }

    /// State a response that carries no facts in the words the deciding verb chose.
    ///
    /// An unreadable ledger, a rejected reservation selection and an exhausted lock
    /// deadline are three different answers, and [`drift_presentation`] already renders
    /// each one for this event. Reading that presentation keeps them three answers here
    /// instead of collapsing them into a single report that no drift comparison ran.
    /// Only a response with no presentation at all reaches the generic wording, which is
    /// the working-directory answer it was written for.
    fn stated_condition_rendering(&self) -> PostToolUseRendering {
        match &self.presentation {
            EnvelopePresentation::NothingToShow => PostToolUseRendering::NoFeedback,
            EnvelopePresentation::RenderedBlocks { blocks } => {
                let stated_block = blocks.as_slice().first().map_or_else(
                    || engine_message_block(UNSTATED_CONDITION_SUMMARY, &self.message),
                    Clone::clone,
                );
                PostToolUseRendering::Feedback {
                    summary: stated_block.summary,
                    detail:  stated_block.detail,
                }
            },
            EnvelopePresentation::NotProvided => PostToolUseRendering::Feedback {
                summary: UNSTATED_CONDITION_SUMMARY.to_owned(),
                detail:  self.message.clone(),
            },
        }
    }

    /// Render drift-reported incursions against a board read taken after that drift response.
    pub(crate) fn post_tool_use_rendering_with_live_board(
        &self,
        live_board: &Self,
    ) -> PostToolUseRendering {
        let (OutputFacts::Drift(report), OutputFacts::Board(board)) =
            (&self.payload.facts, &live_board.payload.facts)
        else {
            return unverifiable_live_incursion_rendering();
        };
        let mut immediate_stop_messages = Vec::new();
        let mut notice_messages = Vec::new();
        append_live_path_attribution_rendering(
            &report.path_attribution,
            &mut immediate_stop_messages,
            &mut notice_messages,
        );
        for result in &report.results {
            let ReservationDriftResult::Changed {
                reservation_id,
                effects,
            } = result
            else {
                continue;
            };
            for effect in effects.as_slice() {
                match effect {
                    DriftEffect::Incursion {
                        incident_id,
                        foreign_reservation_ids,
                        paths,
                        commits,
                    } => match board.live_incursion_membership(*incident_id) {
                        LiveIncursionMembership::Outstanding => {
                            let entered_paths = paths
                                .as_slice()
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>();
                            let foreign_reservation_ids = foreign_reservation_ids
                                .as_slice()
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>();
                            immediate_stop_messages.push(outstanding_incursion_block(
                                &reservation_id.to_string(),
                                &entered_paths,
                                &foreign_reservation_ids,
                                &incident_id.to_string(),
                                &render_incursion_commits(commits),
                            ));
                        },
                        LiveIncursionMembership::Recorded => {},
                        LiveIncursionMembership::Unverifiable => {
                            return unverifiable_live_incursion_rendering();
                        },
                    },
                    DriftEffect::Widened { added_scopes } => {
                        let added_scopes = added_scopes
                            .as_slice()
                            .iter()
                            .map(|scope| format!("file:{}", scope.path))
                            .collect::<Vec<_>>();
                        notice_messages.push(
                            automatic_widening_block(&reservation_id.to_string(), &added_scopes)
                                .detail,
                        );
                    },
                    DriftEffect::Collision {
                        foreign_reservation_ids,
                        paths,
                    } => immediate_stop_messages.push(collision_detail(
                        *reservation_id,
                        foreign_reservation_ids,
                        paths,
                    )),
                }
            }
        }
        let lost_evidence_messages = self
            .payload
            .alerts
            .iter()
            .filter(|alert| matches!(alert, Alert::LostIntegrationEvidence(_)))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        live_board_feedback(
            immediate_stop_messages,
            notice_messages,
            lost_evidence_messages,
        )
    }

    fn post_tool_use_drift_rendering(&self, report: &DriftReport) -> PostToolUseRendering {
        if report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Incursion { .. })
            })
        }) {
            return PostToolUseRendering::FeedbackDecidedByLiveIncursionState;
        }
        let has_collision = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Collision { .. })
            })
        });
        let has_widen = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Widened { .. })
            })
        });
        let lost_evidence = self
            .payload
            .alerts
            .iter()
            .filter(|alert| matches!(alert, Alert::LostIntegrationEvidence(_)))
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let (prefix, immediate_stop) = match &report.path_attribution {
            DriftPathAttributionOutcome::FirstTouchReserved { .. } => {
                ("FIRST-TOUCH CLAIM: ", false)
            },
            DriftPathAttributionOutcome::IncursionDetected { protection, .. } => match protection {
                PostWriteFreePathProtection::NotAcquired => {
                    ("POST-WRITE INCURSION: nothing was reserved. ", true)
                },
                PostWriteFreePathProtection::Acquired { .. } => ("POST-WRITE INCURSION: ", true),
            },
            DriftPathAttributionOutcome::Ambiguous { .. }
            | DriftPathAttributionOutcome::CoordinationRunRequired { .. } => {
                ("DRIFT ATTRIBUTION REQUIRED: ", true)
            },
            DriftPathAttributionOutcome::NotNeeded
            | DriftPathAttributionOutcome::Attributed { .. }
                if has_collision =>
            {
                ("COLLISION: ", true)
            },
            DriftPathAttributionOutcome::NotNeeded
            | DriftPathAttributionOutcome::Attributed { .. }
                if has_widen =>
            {
                ("AUTO-WIDEN: ", false)
            },
            DriftPathAttributionOutcome::NotNeeded
            | DriftPathAttributionOutcome::Attributed { .. } => ("", false),
        };
        let mut messages = match prefix {
            "COLLISION: " => {
                let mut details = automatic_widening_details(report);
                details.extend(collision_details(report));
                details
            },
            "AUTO-WIDEN: " => automatic_widening_details(report),
            _ if !prefix.is_empty() || report.has_reportable_effect() => {
                vec![format!("{prefix}{}", self.message)]
            },
            _ => Vec::new(),
        };
        messages.extend(lost_evidence);
        if messages.is_empty() {
            return PostToolUseRendering::NoFeedback;
        }
        if !immediate_stop
            && !has_collision
            && self
                .payload
                .alerts
                .iter()
                .any(|alert| matches!(alert, Alert::LostIntegrationEvidence(_)))
        {
            let block = lost_integration_evidence_block(&messages.join("\n"));
            return PostToolUseRendering::Feedback {
                summary: block.summary,
                detail:  block.detail,
            };
        }
        let summary = if immediate_stop || has_collision {
            "cargo-berth detected drift that requires an immediate stop."
        } else {
            "cargo-berth widened this worktree reservation footprint."
        };
        PostToolUseRendering::Feedback {
            summary: summary.to_owned(),
            detail:  messages.join("\n"),
        }
    }
}

fn unverifiable_live_incursion_rendering() -> PostToolUseRendering {
    let block = unverifiable_incursion_block();
    PostToolUseRendering::Feedback {
        summary: block.summary,
        detail:  block.detail,
    }
}

fn presentation_from_actionable_alerts(alerts: &[Alert]) -> EnvelopePresentation {
    let details = alerts
        .iter()
        .map(|alert| match alert {
            Alert::LostIntegrationEvidence(_) => alert.to_string(),
            Alert::OrphanedOutstanding(orphan) => {
                let reservation_id = orphan.reservation_id().to_string();
                let (recoverability, recovery_commands) = match orphan.recoverability() {
                    RecoverabilityVerdict::RecoverableFromBranch => (
                        "recoverable_from_branch",
                        vec![format!("resolve {reservation_id} --recovered")],
                    ),
                    RecoverabilityVerdict::RecoverableFromProtectedTip => (
                        "recoverable_from_protected_tip",
                        vec![format!("resolve {reservation_id} --recovered")],
                    ),
                    RecoverabilityVerdict::CommitUnavailable => (
                        "commit_unavailable",
                        vec![
                            format!("resolve {reservation_id} --retire-orphan --why <reason>"),
                            format!("resolve {reservation_id} --abandon --why <reason>"),
                        ],
                    ),
                };
                orphaned_outstanding_block(
                    &reservation_id,
                    &orphan.protected_tip().to_string(),
                    recoverability,
                    &recovery_commands,
                )
            },
        })
        .collect::<Vec<_>>();
    match details.as_slice() {
        [] => EnvelopePresentation::nothing_to_show(),
        [_, ..] => actionable_board_notices_block(&details).into(),
    }
}

fn drift_non_incursion_presentation(
    report: &DriftReport,
    alerts: &[Alert],
) -> EnvelopePresentation {
    let mut widening_details = automatic_widening_details(report);
    let lost_evidence_details = alerts
        .iter()
        .filter(|alert| matches!(alert, Alert::LostIntegrationEvidence(_)))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    match (
        widening_details.as_slice(),
        lost_evidence_details.as_slice(),
    ) {
        ([], []) => EnvelopePresentation::nothing_to_show(),
        (_, [_, ..]) => {
            widening_details.extend(lost_evidence_details);
            lost_integration_evidence_block(&widening_details.join("\n")).into()
        },
        ([_, ..], []) => engine_message_block(
            "cargo-berth widened this worktree reservation footprint.",
            &widening_details.join("\n"),
        )
        .into(),
    }
}

fn automatic_widening_details(report: &DriftReport) -> Vec<String> {
    report
        .results
        .iter()
        .filter_map(|result| match result {
            ReservationDriftResult::Changed {
                reservation_id,
                effects,
            } => effects.as_slice().iter().find_map(|effect| match effect {
                DriftEffect::Widened { added_scopes } => {
                    let added_scopes = added_scopes
                        .as_slice()
                        .iter()
                        .map(|scope| format!("file:{}", scope.path))
                        .collect::<Vec<_>>();
                    Some(
                        automatic_widening_block(&reservation_id.to_string(), &added_scopes).detail,
                    )
                },
                DriftEffect::Incursion { .. } | DriftEffect::Collision { .. } => None,
            }),
            ReservationDriftResult::Unchanged { .. }
            | ReservationDriftResult::PhaseStartObjectUnknown { .. } => None,
        })
        .collect()
}

fn collision_details(report: &DriftReport) -> Vec<String> {
    report
        .results
        .iter()
        .filter_map(|result| match result {
            ReservationDriftResult::Changed {
                reservation_id,
                effects,
            } => effects.as_slice().iter().find_map(|effect| match effect {
                DriftEffect::Collision {
                    foreign_reservation_ids,
                    paths,
                } => Some(collision_detail(
                    *reservation_id,
                    foreign_reservation_ids,
                    paths,
                )),
                DriftEffect::Widened { .. } | DriftEffect::Incursion { .. } => None,
            }),
            ReservationDriftResult::Unchanged { .. }
            | ReservationDriftResult::PhaseStartObjectUnknown { .. } => None,
        })
        .collect()
}

fn collision_detail(
    reservation_id: ReservationId,
    foreign_reservation_ids: &ForeignReservationIdSet,
    paths: &CollisionPathSet,
) -> String {
    format!(
        "COLLISION: reservation {reservation_id} could not widen to {} because {} now holds the path. STOP and resolve the overlap before making more changes.",
        paths
            .as_slice()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", "),
        foreign_reservation_ids
            .as_slice()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

impl OutputPayload {
    const fn from_facts(facts: OutputFacts) -> Self {
        Self {
            facts,
            alerts: Vec::new(),
        }
    }

    #[cfg(test)]
    const fn pending(command_verb: CommandVerb) -> Self {
        let facts = match command_verb {
            CommandVerb::Board
            | CommandVerb::Init
            | CommandVerb::Check
            | CommandVerb::Claim
            | CommandVerb::Drift
            | CommandVerb::Release
            | CommandVerb::Sequence
            | CommandVerb::Resolve
            | CommandVerb::Renew
            | CommandVerb::Identity
            | CommandVerb::Integrate => OutputFacts::NoFacts,
        };
        Self::from_facts(facts)
    }
}

fn blocked_message(conflicts: &[ReservationConflict]) -> String {
    let mut message = overlap_holder_description(conflicts);
    match first_touch_disposition_description(conflicts) {
        FirstTouchHolderRecoveryDescription::NotApplicable => {},
        FirstTouchHolderRecoveryDescription::Described(disposition) => {
            message.push(' ');
            message.push_str(&disposition);
        },
    }
    message
}

fn claimed_presentation(
    reservation_id: ReservationId,
    message: &str,
    session_mapping_publication: &SessionIdentityMappingPublication,
) -> EnvelopePresentation {
    let summary = format!("cargo-berth claimed reservation {reservation_id}.");
    let claimed_detail = format!("Reservation id: `{reservation_id}`.\n\n{message}");
    let detail = match session_mapping_publication {
        SessionIdentityMappingPublication::Published
        | SessionIdentityMappingPublication::ExplicitSelectionAppliesOnlyToCurrentInvocation {
            ..
        } => claimed_detail,
        SessionIdentityMappingPublication::Unavailable { diagnostic } => format!(
            "{claimed_detail}\n\nThe journal append and reservation `{reservation_id}` are durable, but the harness session mapping is unavailable: {diagnostic}. Name reservation `{reservation_id}` explicitly on subsequent commands."
        ),
    };
    engine_message_block(&summary, &detail).into()
}

/// The occasion an engine response is produced on, for the text that names it.
///
/// A response that says *when* the engine met a condition — `after Bash`, `at
/// SessionStart` — is true only while the harness hook for that event is what invoked
/// the verb. Every verb these events drive is also a verb a person runs by hand, so
/// naming the occasion after the verb would put a harness event in front of a reader no
/// harness event ever reached. The hook process that owns the answer records the
/// occasion instead, once, before the verb runs.
#[derive(Clone, Copy)]
pub(crate) enum EngineAnswerOccasion {
    /// No harness hook drives this process, so the response names no event.
    DirectInvocation,
    /// A `PostToolUse` hook owns this answer, taken after one Bash call completed.
    CompletedBashCall,
    /// A `SessionStart` hook owns this answer, taken while one session opened.
    OpeningSession,
}

/// The occasion the current process answers on, recorded once by the hook that owns it.
static ENGINE_ANSWER_OCCASION: OnceLock<EngineAnswerOccasion> = OnceLock::new();

impl EngineAnswerOccasion {
    /// Record this occasion for every response the current process goes on to build.
    ///
    /// A process answers exactly one harness event, so the first record stands and a
    /// later one is ignored rather than allowed to re-date an answer already given.
    pub(crate) fn own_this_process(self) { ENGINE_ANSWER_OCCASION.get_or_init(|| self); }

    /// The occasion this process answers on, which is none until a hook records one.
    fn current() -> Self {
        ENGINE_ANSWER_OCCASION
            .get()
            .copied()
            .unwrap_or(Self::DirectInvocation)
    }

    /// Complete one condition clause with the occasion the engine met it on.
    fn summary_for(self, condition: &str) -> String {
        let occasion = match self {
            Self::DirectInvocation => "",
            Self::CompletedBashCall => " after Bash",
            Self::OpeningSession => " at SessionStart",
        };
        format!("{condition}{occasion}.")
    }
}

/// The engine conditions a harness hook reads, in the words the deciding verb states.
enum HookFacingCondition<'condition> {
    /// This repository is not participating in coordination.
    Unconfigured,
    /// The reservation ledger could not be read.
    LedgerUnreadable { message: &'condition str },
    /// The bounded ledger-lock deadline was exhausted before the ledger came free.
    Contention { diagnostic: &'condition str },
    /// The request itself could not be accepted.
    InvalidInput { diagnostic: &'condition str },
}

/// Carry a presentation only for the verbs a harness hook reads.
///
/// `check` reaches its user through `hook pre-tool-use`, `board` through
/// `hook session-start`, and `drift` through `hook post-tool-use`, so all three state
/// these conditions in their own words rather than leave a hook to classify a wire
/// status. Every other verb reaches its user through the engine `message`, so naming
/// them here once keeps a new verb from being forgotten in four constructors.
///
/// The verb decides which words a condition gets; it does not decide which occasion
/// those words name. Each of these verbs is shared between its hook and a person
/// running it by hand, so the occasion comes from
/// [`EngineAnswerOccasion::current`] instead.
fn hook_facing_presentation(
    command_verb: CommandVerb,
    condition: &HookFacingCondition<'_>,
) -> EnvelopePresentation {
    match command_verb {
        CommandVerb::Check => pre_tool_use_check_presentation(condition),
        CommandVerb::Board => board_presentation(condition),
        CommandVerb::Drift => drift_presentation(condition),
        CommandVerb::Init
        | CommandVerb::Claim
        | CommandVerb::Release
        | CommandVerb::Sequence
        | CommandVerb::Integrate
        | CommandVerb::Resolve
        | CommandVerb::Renew
        | CommandVerb::Identity => EnvelopePresentation::NotProvided,
    }
}

/// State one condition in the words `hook pre-tool-use` publishes about a pending edit.
fn pre_tool_use_check_presentation(condition: &HookFacingCondition<'_>) -> EnvelopePresentation {
    match condition {
        HookFacingCondition::Unconfigured => EnvelopePresentation::nothing_to_show(),
        HookFacingCondition::LedgerUnreadable { message } => {
            engine_message_block(LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE, message).into()
        },
        HookFacingCondition::Contention { diagnostic } => {
            engine_message_block(CHECK_CONTENTION_SUMMARY, diagnostic).into()
        },
        HookFacingCondition::InvalidInput { diagnostic } => {
            engine_message_block(CHECK_INVALID_INPUT_SUMMARY, diagnostic).into()
        },
    }
}

/// State one condition in the words a `board` reader is owed, hook or terminal alike.
///
/// A repository outside coordination is not a condition to raise, so an unconfigured
/// board is deliberate silence rather than a notice at every session start. An
/// unreadable ledger and an exhausted lock deadline are both reported, and they are
/// distinguished here so a session-start hook branches on presentation alone: both
/// arrive as the same exit status, and a consumer that had to tell them apart would be
/// classifying a wire status the engine already understands. A rejected request is the
/// caller's own, and a caller reading its own rejection is already reading `message`.
fn board_presentation(condition: &HookFacingCondition<'_>) -> EnvelopePresentation {
    let occasion = EngineAnswerOccasion::current();
    match condition {
        HookFacingCondition::Unconfigured => EnvelopePresentation::nothing_to_show(),
        HookFacingCondition::LedgerUnreadable { message } => engine_message_block(
            &occasion.summary_for(LEDGER_UNREADABLE_CONDITION),
            &format!("{message} {BOARD_LEDGER_RECOVERY}"),
        )
        .into(),
        HookFacingCondition::Contention { diagnostic } => engine_message_block(
            &occasion.summary_for(LEDGER_LOCK_DEADLINE_CONDITION),
            &format!(
                "{diagnostic} {} {BOARD_CONTENTION_RECOVERY}",
                spent_retry_budget_sentence(occasion, "the hook did not invoke board again")
            ),
        )
        .into(),
        HookFacingCondition::InvalidInput { .. } => EnvelopePresentation::NotProvided,
    }
}

/// State one condition in the words a `drift` reader is owed, hook or terminal alike.
///
/// The Bash call a `hook post-tool-use` process reports on has already run, so none of
/// these conditions blocks anything; what is at stake is only that the reader learns no
/// drift comparison covered it, and learns which of the three reasons applies. An
/// unconfigured repository is not one of them, so it stays silent.
fn drift_presentation(condition: &HookFacingCondition<'_>) -> EnvelopePresentation {
    let occasion = EngineAnswerOccasion::current();
    match condition {
        HookFacingCondition::Unconfigured => EnvelopePresentation::nothing_to_show(),
        HookFacingCondition::LedgerUnreadable { message } => {
            engine_message_block(&occasion.summary_for(LEDGER_UNREADABLE_CONDITION), message).into()
        },
        HookFacingCondition::Contention { diagnostic } => engine_message_block(
            &occasion.summary_for(LEDGER_LOCK_DEADLINE_CONDITION),
            &format!(
                "{diagnostic} {}",
                spent_retry_budget_sentence(occasion, "it was not invoked again")
            ),
        )
        .into(),
        HookFacingCondition::InvalidInput { diagnostic } => engine_message_block(
            DRIFT_SELECTION_SUMMARY,
            &format!("{diagnostic} {DRIFT_SELECTION_RECOVERY}"),
        )
        .into(),
    }
}

/// State that the bounded ledger-lock wait is already spent, and who did not spend it twice.
///
/// The wait itself is [`MUTATING_VERB_CONTENTION_TOLERANCE`], so the figure is taken from
/// the tolerance the engine actually applies rather than restated beside it. A caller
/// running the verb by hand is free to run it again immediately and is told nothing about
/// a hook that did not.
fn spent_retry_budget_sentence(occasion: EngineAnswerOccasion, unretried: &str) -> String {
    let budget_seconds = MUTATING_VERB_CONTENTION_TOLERANCE.as_secs();
    let spent = format!("The engine already spent its single {budget_seconds}-second retry budget");
    match occasion {
        EngineAnswerOccasion::DirectInvocation => format!("{spent}."),
        EngineAnswerOccasion::CompletedBashCall | EngineAnswerOccasion::OpeningSession => {
            format!("{spent}; {unretried}.")
        },
    }
}

fn engine_result_presentation(summary: &str, detail: &str) -> EnvelopePresentation {
    engine_message_block(summary, detail).into()
}

fn blocked_claim_refusal_detail(conflicts: &[ReservationConflict]) -> String {
    let mut sections = vec![claim_holder_facts(conflicts)];
    sections.push(blocked_edit_answer_guidance().to_owned());
    append_first_touch_holder_recovery_guidance(
        &mut sections,
        conflicts,
        FirstTouchHolderRecoveryContext::Refusal,
    );
    if conflicts.len() > 1 {
        sections.push(
            "More than one holder remains. Narrow the requested scopes before asking for a proposal, because one proposal binds exactly one blocker."
                .to_owned(),
        );
    }
    sections.join("\n\n")
}

fn claim_authorization_presentation(escalation: &OverlapEscalationPayload) -> EnvelopePresentation {
    let direction = overlap_direction_description(&escalation.answer);
    let proposal_token = escalation.proposal_token.to_string();
    let mut sections = vec![
        claim_holder_facts(&escalation.conflicts),
        format!(
            "- selected direction: {direction}.\n- authorization reason: {}.\n- consequence: {}.\n- proposal: after explicit approval, repeat the claim with the selected answer and supply the exact transient token through `--proposal`.\n- transient token:\n\n`{proposal_token}`",
            escalation.authorization_reason, escalation.consequence,
        ),
    ];
    append_first_touch_holder_recovery_guidance(
        &mut sections,
        &escalation.conflicts,
        FirstTouchHolderRecoveryContext::Proposal,
    );
    engine_message_block(
        "cargo-berth prepared an overlap proposal that awaits explicit approval.",
        &sections.join("\n\n"),
    )
    .into()
}

fn blocked_edit_refusal_detail(
    requested_scopes: &ReservationScopeSet,
    conflicts: &[ReservationConflict],
) -> String {
    let mut sections = vec![blocked_edit_holder_facts(requested_scopes, conflicts)];
    sections.push(blocked_edit_answer_guidance().to_owned());
    append_first_touch_holder_recovery_guidance(
        &mut sections,
        conflicts,
        FirstTouchHolderRecoveryContext::Refusal,
    );
    if conflicts.len() > 1 {
        sections.push(
            "More than one holder remains. Narrow the requested scopes before asking for a proposal, because one proposal binds exactly one blocker."
                .to_owned(),
        );
    }
    sections.join("\n\n")
}

fn blocked_edit_holder_facts(
    requested_scopes: &ReservationScopeSet,
    conflicts: &[ReservationConflict],
) -> String {
    format!(
        "The requested edit is blocked for these scopes: {}.\n\n{}",
        render_scopes(requested_scopes),
        claim_holder_facts(conflicts),
    )
}

fn claim_holder_facts(conflicts: &[ReservationConflict]) -> String {
    conflicts
        .iter()
        .map(|conflict| {
            format!(
                "Holder `{}`:\n- coordination run id: `{}`\n- worktree: `{}`\n- branch or detached head: `{}`\n- claimed at: `{}`\n- activity: {}\n- acquisition source: {}\n- reservation purpose: {}\n- exact shared scopes: {}",
                conflict.reservation_id,
                conflict.holder_run_id,
                conflict.holder_worktree_id(),
                conflict.holder_branch(),
                conflict.claimed_at(),
                conflict.holder_activity_description(),
                source_description(&conflict.source),
                purpose_description(&conflict.purpose),
                render_scopes(&conflict.overlapping_scopes),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) const fn blocked_edit_answer_guidance() -> &'static str {
    r#"Choose exactly one answer for one named holder. The first four are reasoned `cargo-berth claim` answers, and each requires a non-empty reason. Run the `cargo-berth` invocation shown for each answer from the repository:

1. **Land before the holder** — `cargo-berth claim <paths...> --before <holder-reservation-id> --overlap-why "<reason>"`. The requester takes the paths and integrates first; the holder remains held until the requester is on trunk. Use this when the holder will build on the requester's change.
2. **Land after the holder** — `cargo-berth claim <paths...> --after <holder-reservation-id> --overlap-why "<reason>"`. The requester takes the paths and integrates second; it remains held until the holder's protected tip is on trunk and is an ancestor of the requester's `HEAD`. Use this when the requester will build on the holder.
3. **Defer the order** — `cargo-berth claim <paths...> --defer <holder-reservation-id> --overlap-why "<reason>"`. The requester takes the paths, no ordering edge is added, and the unresolved overlap remains visible on the board until someone later sequences it.
4. **Override** — `cargo-berth claim <paths...> --override <holder-reservation-id> --overlap-why "<reason>"`. The requester takes the paths, no ordering edge is added, and the override plus its reason remains visible on the board.
5. **Leave it alone.** Run no engine command, append nothing, and work elsewhere.

Only **Land before the holder** and **Land after the holder** add an ordering edge. Defer and override add no edge; their recorded overlap remains visible on the board.

An answered claim only produces a proposal at exit 3. Show that proposal and wait for explicit approval in a later turn before submitting its exact `--proposal` token. Never produce and submit a token in the same turn.

The trunk-gate bypass is not an edit answer and cannot permit this edit."#
}

fn first_touch_holder_recovery_guidance(
    conflicts: &[ReservationConflict],
    context: FirstTouchHolderRecoveryContext,
) -> FirstTouchHolderRecoveryDescription {
    let dispositions = conflicts
        .iter()
        .filter(|conflict| matches!(conflict.source, ClaimSource::FirstTouch))
        .map(|conflict| {
            let reservation_id = conflict.reservation_id;
            format!(
                "- Reservation `{reservation_id}`: `cargo-berth release {reservation_id}` once the work is on trunk, `cargo-berth resolve {reservation_id} --integrated-as <TRUNK_OID>` after that release when git cannot prove the integration, or `cargo-berth resolve {reservation_id} --abandon --why <WHY>` when the work was discarded."
            )
        })
        .collect::<Vec<_>>();
    let clearing_relationship = match context {
        FirstTouchHolderRecoveryContext::Refusal => "An overlap answer does not clear one",
        FirstTouchHolderRecoveryContext::Proposal => "The proposal does not clear one",
    };
    match dispositions.as_slice() {
        [] => FirstTouchHolderRecoveryDescription::NotApplicable,
        [_, ..] => FirstTouchHolderRecoveryDescription::Described(format!(
            "A first-touch holder was acquired by whichever edit reached the paths first, so it may protect no work at all. {clearing_relationship}; these commands do, and they belong to the holder:\n\n{}\n\n`release` records the protected checkpoint and must run from the holder's own worktree. Both `resolve` dispositions run from anywhere but assert facts about the holder's work, so ask the holder before recording one.",
            dispositions.join("\n")
        )),
    }
}

fn append_first_touch_holder_recovery_guidance(
    sections: &mut Vec<String>,
    conflicts: &[ReservationConflict],
    context: FirstTouchHolderRecoveryContext,
) {
    match first_touch_holder_recovery_guidance(conflicts, context) {
        FirstTouchHolderRecoveryDescription::NotApplicable => {},
        FirstTouchHolderRecoveryDescription::Described(guidance) => sections.push(guidance),
    }
}

/// Which claim document explains how a first-touch holder is cleared.
#[derive(Clone, Copy)]
enum FirstTouchHolderRecoveryContext {
    /// A refusal presents overlap answers and holder recovery separately.
    Refusal,
    /// A proposal presents the selected answer without the refusal's answer menu.
    Proposal,
}

/// Whether blocked first-touch holders need a disposition description.
enum FirstTouchHolderRecoveryDescription {
    /// None of the blocking reservations came from a first-touch edit.
    NotApplicable,
    /// The description names how each first-touch holder can clear its reservation.
    Described(String),
}

/// Name the verbs that clear a first-touch holder, which no other message reaches.
fn first_touch_disposition_description(
    conflicts: &[ReservationConflict],
) -> FirstTouchHolderRecoveryDescription {
    let first_touch_holders = conflicts
        .iter()
        .filter(|conflict| matches!(conflict.source, ClaimSource::FirstTouch))
        .map(|conflict| conflict.reservation_id.to_string())
        .collect::<Vec<_>>();
    match first_touch_holders.as_slice() {
        [] => FirstTouchHolderRecoveryDescription::NotApplicable,
        [reservation_id] => FirstTouchHolderRecoveryDescription::Described(format!(
            "Reservation {reservation_id} came from a first-touch edit, so its holder clears it with cargo-berth release {reservation_id} once the work is on trunk, cargo-berth resolve {reservation_id} --integrated-as <TRUNK_OID> after that release when git cannot prove the integration, or cargo-berth resolve {reservation_id} --abandon --why <WHY> when the work is discarded."
        )),
        [_, _, ..] => FirstTouchHolderRecoveryDescription::Described(format!(
            "Reservations {} came from first-touch edits, so a holder clears one with cargo-berth release <RESERVATION_ID> once the work is on trunk, cargo-berth resolve <RESERVATION_ID> --integrated-as <TRUNK_OID> after that release when git cannot prove the integration, or cargo-berth resolve <RESERVATION_ID> --abandon --why <WHY> when the work is discarded.",
            first_touch_holders.join(", ")
        )),
    }
}

fn overlap_holder_description(conflicts: &[ReservationConflict]) -> String {
    match conflicts {
        [] => {
            "A foreign reservation overlaps the requested paths; reduce the requested scopes or coordinate with the holder, then retry."
                .to_owned()
        },
        [conflict] => {
            format!(
                "Reservation {} on {} ({}, {}) holds overlapping paths for {}; reduce the requested scopes or coordinate with the holder, then retry.",
                conflict.reservation_id,
                conflict.holder_branch(),
                source_description(&conflict.source),
                purpose_description(&conflict.purpose),
                conflict.holder_run_id,
            )
        },
        [_, _, ..] => {
            let holder_count = conflicts.len();
            let holders = conflicts
                .iter()
                .map(|conflict| {
                    format!(
                        "reservation {} on {} ({}, {}) for coordination run {}",
                        conflict.reservation_id,
                        conflict.holder_branch(),
                        source_description(&conflict.source),
                        purpose_description(&conflict.purpose),
                        conflict.holder_run_id,
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{holder_count} reservations hold overlapping paths: {holders}; reduce the requested scopes or coordinate with the holders, then retry.",
            )
        },
    }
}

fn message_with_session_mapping_publication(
    message: String,
    publication: &SessionIdentityMappingPublication,
) -> String {
    match publication {
        SessionIdentityMappingPublication::Published => message,
        SessionIdentityMappingPublication::ExplicitSelectionAppliesOnlyToCurrentInvocation {
            ..
        } => format!(
            "{message} The explicit reservation selection applies only to this invocation because no usable harness session id was supplied; name the reservation again on a later check."
        ),
        SessionIdentityMappingPublication::Unavailable { diagnostic } => format!(
            "{message} The harness session mapping could not be published: {diagnostic}. Name the coordination run and reservation explicitly for later drift checks."
        ),
    }
}

fn session_mapping_publication_presentation(
    message: &str,
    publication: &SessionIdentityMappingPublication,
) -> EnvelopePresentation {
    match publication {
        SessionIdentityMappingPublication::Published => EnvelopePresentation::nothing_to_show(),
        SessionIdentityMappingPublication::ExplicitSelectionAppliesOnlyToCurrentInvocation {
            ..
        }
        | SessionIdentityMappingPublication::Unavailable { .. } => {
            degraded_session_mapping_block(message).into()
        },
    }
}

fn result_has_effect(
    result: &ReservationDriftResult,
    matches_effect: impl Fn(&DriftEffect) -> bool,
) -> bool {
    match result {
        ReservationDriftResult::Unchanged { .. }
        | ReservationDriftResult::PhaseStartObjectUnknown { .. } => false,
        ReservationDriftResult::Changed { effects, .. } => {
            effects.as_slice().iter().any(matches_effect)
        },
    }
}

/// The abbreviated object-name length a reader can paste into a git command.
const SHORT_OBJECT_ID_CHARACTERS: usize = 8;

/// Name the commits behind an incursion's entered paths, or nothing when it has none.
///
/// A path that arrived on a commit and a path just written read identically otherwise,
/// so the reader cannot tell a false incursion from a real one without rebuilding the
/// phase range by hand.
fn render_incursion_commits(commits: &[IncursionCommit]) -> String {
    if commits.is_empty() {
        return String::new();
    }
    let rendered = commits
        .iter()
        .map(|commit| {
            format!(
                "{} \"{}\" ({}) covering {}",
                commit
                    .commit
                    .to_string()
                    .chars()
                    .take(SHORT_OBJECT_ID_CHARACTERS)
                    .collect::<String>(),
                commit.subject,
                match commit.origin {
                    IncursionCommitOrigin::PhaseAuthored => "this phase authored it",
                    IncursionCommitOrigin::AlreadyOnTrunk =>
                        "already on trunk, so this phase received it",
                    IncursionCommitOrigin::Unknown => "origin undetermined",
                },
                commit
                    .paths
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(" Committed by {rendered}.")
}

fn drift_message(report: &DriftReport) -> String {
    if !report.has_reportable_effect() {
        return if report.results.is_empty() {
            "No active reservation in this worktree required a drift check.".to_owned()
        } else {
            "No changed path fell outside the selected reservation coverage.".to_owned()
        };
    }
    let mut message = drift_path_attribution_message(&report.path_attribution);
    for result in &report.results {
        let (reservation_id, effects) = match result {
            ReservationDriftResult::Unchanged { .. } => continue,
            ReservationDriftResult::PhaseStartObjectUnknown {
                reservation_id,
                phase_start,
            } => {
                if !message.is_empty() {
                    message.push(' ');
                }
                let _ = write!(
                    message,
                    "Reservation {reservation_id} could not be compared because git could not read phase-start object {phase_start}. Restore that object before using this drift result."
                );
                continue;
            },
            ReservationDriftResult::Changed {
                reservation_id,
                effects,
            } => (reservation_id, effects),
        };
        for effect in effects.as_slice() {
            if !message.is_empty() {
                message.push(' ');
            }
            match effect {
                DriftEffect::Widened { added_scopes } => {
                    let rendered = added_scopes
                        .as_slice()
                        .iter()
                        .map(|scope| format!("file:{}", scope.path))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = write!(
                        message,
                        "Widened reservation {reservation_id} to cover {rendered}."
                    );
                },
                DriftEffect::Incursion {
                    incident_id,
                    foreign_reservation_ids,
                    paths,
                    commits,
                } => {
                    let _ = write!(
                        message,
                        "Incursion {incident_id}: reservation {reservation_id} entered {} held by foreign reservation(s) {}.{} Stop and resolve the overlap with `cargo-berth resolve {reservation_id} --incursion {incident_id}` before making more changes. If no coordination run was identified before first-touch attribution, CARGO_BERTH_RUN can select an existing run for later invocations.",
                        paths
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        foreign_reservation_ids
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        render_incursion_commits(commits)
                    );
                },
                DriftEffect::Collision {
                    foreign_reservation_ids,
                    paths,
                } => {
                    let _ = write!(
                        message,
                        "Reservation {reservation_id} could not widen to {} because foreign reservation(s) {} acquired an edit-blocking overlap. Stop and resolve the collision.",
                        paths
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        foreign_reservation_ids
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                },
            }
        }
    }
    message
}

fn append_live_path_attribution_rendering(
    attribution: &DriftPathAttributionOutcome,
    immediate_stop_messages: &mut Vec<String>,
    notice_messages: &mut Vec<String>,
) {
    let message = drift_path_attribution_message(attribution);
    match attribution {
        DriftPathAttributionOutcome::FirstTouchReserved { .. } => {
            notice_messages.push(format!("FIRST-TOUCH CLAIM: {message}"));
        },
        DriftPathAttributionOutcome::IncursionDetected { .. } => {
            immediate_stop_messages.push(format!("POST-WRITE INCURSION: {message}"));
        },
        DriftPathAttributionOutcome::Ambiguous { .. }
        | DriftPathAttributionOutcome::CoordinationRunRequired { .. } => {
            immediate_stop_messages.push(format!("DRIFT ATTRIBUTION REQUIRED: {message}"));
        },
        DriftPathAttributionOutcome::NotNeeded | DriftPathAttributionOutcome::Attributed { .. } => {
        },
    }
}

fn live_board_feedback(
    mut immediate_stop_messages: Vec<String>,
    notice_messages: Vec<String>,
    lost_evidence_messages: Vec<String>,
) -> PostToolUseRendering {
    if immediate_stop_messages.is_empty()
        && notice_messages.is_empty()
        && lost_evidence_messages.is_empty()
    {
        return PostToolUseRendering::NoFeedback;
    }
    let summary = if !immediate_stop_messages.is_empty() {
        "cargo-berth detected drift that requires an immediate stop."
    } else if !lost_evidence_messages.is_empty() {
        "cargo-berth detected lost integration evidence for released work."
    } else {
        "cargo-berth widened this worktree reservation footprint."
    };
    immediate_stop_messages.extend(notice_messages);
    immediate_stop_messages.extend(lost_evidence_messages);
    PostToolUseRendering::Feedback {
        summary: summary.to_owned(),
        detail:  immediate_stop_messages.join("\n"),
    }
}

fn drift_path_attribution_message(attribution: &DriftPathAttributionOutcome) -> String {
    match attribution {
        DriftPathAttributionOutcome::NotNeeded | DriftPathAttributionOutcome::Attributed { .. } => {
            String::new()
        },
        DriftPathAttributionOutcome::FirstTouchReserved { acquisition, .. } => {
            format!(
                "First-touch reservation {} now protects the changed paths.",
                acquisition.reservation_id
            )
        },
        DriftPathAttributionOutcome::IncursionDetected {
            paths,
            conflicts,
            protection,
        } => {
            let incursion = format!(
                "Post-write detection found changed paths {} inside foreign reservations {}. The write already happened; stop and resolve the incursion before making more changes.",
                paths
                    .as_slice()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                conflicts
                    .iter()
                    .map(|conflict| conflict.reservation_id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            match protection {
                PostWriteFreePathProtection::NotAcquired => incursion,
                PostWriteFreePathProtection::Acquired {
                    acquisition,
                    scopes,
                } => format!(
                    "{incursion} First-touch reservation {} now protects the free paths {}. If no coordination run was identified before this observation, one was started; CARGO_BERTH_RUN can select an existing run for later invocations.",
                    acquisition.reservation_id,
                    scopes
                        .as_slice()
                        .iter()
                        .map(|scope| scope.path.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        },
        DriftPathAttributionOutcome::Ambiguous { candidates, paths } => format!(
            "Changed paths {} were not widened because attribution is ambiguous among reservations {}. Run `cargo-berth drift --reservation <id>` with one listed reservation.",
            paths
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            candidates
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        DriftPathAttributionOutcome::CoordinationRunRequired { paths } => format!(
            "Changed paths {} were not widened because no coordination run was identified. Set CARGO_BERTH_RUN to the run that owns the target reservation, then run `cargo-berth drift --reservation <id>`.",
            paths
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn integration_blockers(violations: &[IntegrationViolation]) -> WireOrderedReservationIds {
    WireOrderedReservationIds::sorted_and_deduplicated(
        violations
            .iter()
            .flat_map(|violation| violation.blocking_reservations.iter())
            .map(|reservation| reservation.reservation_id)
            .collect(),
    )
}

fn integration_blocked_message(
    reservation_id: ReservationId,
    violations: &[IntegrationViolation],
) -> String {
    let mut message = format!(
        "Reservation {reservation_id} cannot enter main while its integration order is held."
    );
    for violation in violations {
        let _ = write!(
            message,
            "\nEntering reservation {}: {}; purpose: {}; protected paths: {}.",
            violation.reservation.reservation_id,
            source_description(&violation.reservation.source),
            purpose_description(&violation.reservation.purpose),
            render_scopes(&violation.reservation.scopes),
        );
        for blocker in &violation.blocking_reservations {
            let _ = write!(
                message,
                "\nBlocking reservation {}: {}; purpose: {}; protected paths: {}.",
                blocker.reservation_id,
                source_description(&blocker.source),
                purpose_description(&blocker.purpose),
                render_scopes(&blocker.scopes),
            );
        }
        for hold in &violation.holds {
            message.push('\n');
            message.push_str(&integration_hold_message(
                violation.reservation.reservation_id,
                hold,
            ));
        }
    }
    let _ = write!(
        message,
        "\nTo deliberately proceed once: cargo-berth integrate {reservation_id} --force --why \"<reason>\". Last resort: CARGO_BERTH_BYPASS=1 <git command>."
    );
    message
}

fn integration_hold_message(subject: ReservationId, hold: &IntegrationHold) -> String {
    match hold {
        IntegrationHold::OrderingEdge {
            edge_id,
            predecessor,
            scopes,
            reason,
            readiness,
            ..
        } => {
            let recovery = match readiness {
                EdgeReadiness::Holding {
                    hold: EdgeHold::AwaitingPredecessorCheckpoint,
                } => format!("run cargo-berth release {predecessor} after checkpointing it"),
                EdgeReadiness::Holding {
                    hold:
                        EdgeHold::PredecessorNotOnTrunk {
                            evidence: UnintegratedPredecessorEvidence::NotIntegrated,
                        },
                } => format!("run cargo-berth integrate {predecessor}"),
                EdgeReadiness::Holding {
                    hold:
                        EdgeHold::PredecessorNotOnTrunk {
                            evidence: UnintegratedPredecessorEvidence::TrunkRewritten,
                        },
                } => format!(
                    "re-record verified evidence with cargo-berth resolve {predecessor} --integrated-as <trunk-oid>"
                ),
                EdgeReadiness::Holding {
                    hold:
                        EdgeHold::PredecessorNotOnTrunk {
                            evidence: UnintegratedPredecessorEvidence::ObjectUnknown,
                        },
                } => "repair the unresolvable git object, then rerun the integration".to_owned(),
                EdgeReadiness::Holding {
                    hold: EdgeHold::AwaitingSuccessorIncorporation,
                } => "rebase this worktree onto current main so it incorporates the predecessor"
                    .to_owned(),
                EdgeReadiness::Cancelled | EdgeReadiness::Fulfilled => {
                    "rerun the gate because this edge is no longer holding".to_owned()
                },
            };
            format!(
                "Ordering edge {edge_id} waits on reservation {predecessor}; covered paths: {}; recorded reason: {reason}; recovery: {recovery}.",
                render_scopes(scopes),
            )
        },
        IntegrationHold::DeferredOverlap {
            deferred,
            blocker,
            scopes,
            reason,
            ..
        } => {
            let counterpart = if *deferred == subject {
                *blocker
            } else {
                *deferred
            };
            format!(
                "Unresolved deferral with reservation {counterpart}; covered paths: {}; recorded reason: {reason}; recovery: cargo-berth sequence {counterpart} {subject} --why \"{}\".",
                render_scopes(scopes),
                shell_double_quoted(&reason.to_string()),
            )
        },
    }
}

fn render_scopes(scopes: &ReservationScopeSet) -> String {
    scopes
        .as_slice()
        .iter()
        .map(|scope| {
            let kind = match scope.kind {
                ScopeKind::File => "file",
                ScopeKind::Tree => "tree",
            };
            format!("{kind}:{}", scope.path)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn shell_double_quoted(value: &str) -> String { value.replace('\\', "\\\\").replace('"', "\\\"") }

fn source_description(claim_source: &ClaimSource) -> String {
    match claim_source {
        ClaimSource::WorkPlan { plan, phase } => format!("plan {plan}, phase {phase}"),
        ClaimSource::FirstTouch => "first-touch edit".to_owned(),
        ClaimSource::Explicit => "explicit claim".to_owned(),
    }
}

fn purpose_description(reservation_purpose: &ReservationPurpose) -> String {
    match reservation_purpose {
        ReservationPurpose::Explained(explanation) => explanation.to_string(),
        ReservationPurpose::NotProvidedByCaller => "no reason provided by caller".to_owned(),
    }
}

fn overlap_direction_description(answer: &PermissiveOverlapAnswer) -> String {
    match answer {
        PermissiveOverlapAnswer::Sequence { blocker, direction } => match direction {
            OrderingDirection::RequesterBeforeHolder => {
                format!("requester before holder {blocker}")
            },
            OrderingDirection::HolderBeforeRequester => {
                format!("holder {blocker} before requester")
            },
        },
        PermissiveOverlapAnswer::Defer { blocker } => {
            format!("none declared; deferred with holder {blocker}")
        },
        PermissiveOverlapAnswer::Override { blocker } => {
            format!("none declared; overridden with holder {blocker}")
        },
    }
}

impl From<InitializationState> for InitializationResource {
    fn from(initialization_state: InitializationState) -> Self {
        match initialization_state {
            InitializationState::Created => Self::Created,
            InitializationState::Existing => Self::Existing,
        }
    }
}

impl From<&ManagedHookInstallation> for InitializedManagedHook {
    fn from(installation: &ManagedHookInstallation) -> Self {
        Self {
            name:       installation.name().to_owned(),
            activation: ManagedHookActivation::from(installation.activation()),
        }
    }
}

impl From<&ManagedHookActivationOutcome> for ManagedHookActivation {
    fn from(activation: &ManagedHookActivationOutcome) -> Self {
        match activation {
            ManagedHookActivationOutcome::Active { installation } => Self::Active {
                installation: ActiveHookInstallation::from(*installation),
            },
            ManagedHookActivationOutcome::Inactive { reason } => Self::Inactive {
                reason: ManagedHookInactivity::from(reason),
            },
        }
    }
}

impl From<ActiveManagedHookInstallation> for ActiveHookInstallation {
    fn from(installation: ActiveManagedHookInstallation) -> Self {
        match installation {
            ActiveManagedHookInstallation::Installed => Self::Installed,
            ActiveManagedHookInstallation::Current => Self::Current,
        }
    }
}

impl From<&crate::gate::install::ManagedHookInactivity> for ManagedHookInactivity {
    fn from(reason: &crate::gate::install::ManagedHookInactivity) -> Self {
        match reason {
            crate::gate::install::ManagedHookInactivity::PreservedUnmanaged => {
                Self::PreservedUnmanaged
            },
            crate::gate::install::ManagedHookInactivity::InstallationFailed { diagnostic } => {
                Self::InstallationFailed {
                    diagnostic: diagnostic.clone(),
                }
            },
        }
    }
}

fn initialization_message(hooks: &[InitializedManagedHook]) -> String {
    let mut message = INITIALIZED_MESSAGE.to_owned();
    for hook in hooks {
        match &hook.activation {
            ManagedHookActivation::Active { .. } => {},
            ManagedHookActivation::Inactive {
                reason: ManagedHookInactivity::PreservedUnmanaged,
            } => {
                let _ = write!(
                    message,
                    " Hook '{}' is occupied by an unmanaged hook, so cargo-berth protection for that hook is not active. Incorporate the existing hook in a wrapper or move it aside, then rerun cargo berth init.",
                    hook.name
                );
            },
            ManagedHookActivation::Inactive {
                reason: ManagedHookInactivity::InstallationFailed { diagnostic },
            } => {
                let _ = write!(
                    message,
                    " Hook '{}' is not active because cargo-berth could not install it: {diagnostic}. Resolve the reported hook installation error, then rerun cargo berth init.",
                    hook.name
                );
            },
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::CommandVerb;
    use super::LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE;
    use super::OutputEnvelope;
    use super::OutputStatus;
    use super::PostCommitRendering;
    use crate::config::ConfigError;
    use crate::config::InitializationState;
    use crate::ledger::LedgerError;
    use crate::ledger::LedgerInitialization;
    use crate::presentation::EnvelopePresentation;
    use crate::reservation::LifecycleTransitionError;
    use crate::reservation::ReservationReplayError;

    const REPLAY_RESERVATION_ID: &str = "01991f4d-77d8-7f5f-9a1f-000000000001";

    #[test]
    fn envelope_round_trips_with_its_additive_payload_field() {
        let output_envelope = OutputEnvelope::unimplemented(CommandVerb::Board);
        let serialized_envelope = serde_json::to_string(&output_envelope);

        assert!(
            serialized_envelope
                .as_ref()
                .is_ok_and(|serialized_envelope| serialized_envelope.contains("\"payload\""))
        );
        assert!(
            serialized_envelope
                .and_then(
                    |serialized_envelope| serde_json::from_str::<OutputEnvelope>(
                        &serialized_envelope
                    )
                )
                .is_ok_and(|round_tripped| round_tripped == output_envelope)
        );
    }

    #[test]
    fn named_replay_failures_carry_exact_reasons_subjects_and_hard_stops()
    -> Result<(), Box<dyn std::error::Error>> {
        let reservation_id = REPLAY_RESERVATION_ID.parse()?;
        for (error, reason) in [
            (
                ReservationReplayError::WidenRequiresUnreleased(reservation_id),
                "widen_requires_unreleased",
            ),
            (
                ReservationReplayError::InvalidLifecycleTransition(
                    reservation_id,
                    LifecycleTransitionError::ResnapshotRequiresOutstanding,
                ),
                "resnapshot_requires_outstanding",
            ),
            (
                ReservationReplayError::IntegrationProofSubjectRevisionExhausted(reservation_id),
                "integration_proof_subject_revision_exhausted",
            ),
            (
                ReservationReplayError::ActiveScopedPatchComparison(reservation_id),
                "active_scoped_patch_comparison",
            ),
            (
                ReservationReplayError::IntegrationProofSubjectMismatch(reservation_id),
                "integration_proof_subject_mismatch",
            ),
        ] {
            let envelope = OutputEnvelope::replay_failure(CommandVerb::Release, &error);
            let serialized = serde_json::to_value(envelope)?;
            assert_eq!(
                serialized["payload"]["data"],
                json!({
                    "reason": reason,
                    "subject": {"kind": "reservation", "id": REPLAY_RESERVATION_ID},
                    "effect": "hard_stop",
                })
            );
            assert_eq!(serialized["status"], "ledger_unreadable");
            assert_eq!(serialized["exit_code"], 4);
        }
        Ok(())
    }

    #[test]
    fn drift_verb_uses_its_frozen_serde_spelling() {
        assert!(
            serde_json::to_string(&CommandVerb::Drift)
                .is_ok_and(|serialized| serialized == "\"drift\"")
        );
        assert_eq!(
            serde_json::from_str::<CommandVerb>("\"drift\"").ok(),
            Some(CommandVerb::Drift)
        );
    }

    #[test]
    fn init_has_a_non_placeholder_status() {
        let output_envelope = OutputEnvelope::initialized(
            LedgerInitialization {
                ledger:        InitializationState::Created,
                configuration: InitializationState::Existing,
            },
            &[],
        );

        assert_eq!(output_envelope.status, OutputStatus::Initialized);
        assert_eq!(output_envelope.exit_code, crate::exit::BerthExit::Clear);
    }

    #[test]
    fn failed_init_has_no_initialization_facts() {
        let output_envelope = OutputEnvelope::ledger_unreadable(CommandVerb::Init, "bad journal");

        assert_eq!(output_envelope.payload.facts, super::OutputFacts::NoFacts);
        assert!(
            serde_json::to_string(&output_envelope.payload).is_ok_and(|payload| !payload
                .contains("ledger")
                && !payload.contains("configuration"))
        );
    }

    #[test]
    fn unconfigured_and_unreadable_checks_have_distinct_presentations_at_exit_four() {
        let expected_configuration_path = PathBuf::from(".claude/config/berth.toml");
        let malformed = LedgerError::Config(ConfigError::UnknownKey("porthole".to_owned()));

        let unconfigured =
            OutputEnvelope::unconfigured(CommandVerb::Check, &expected_configuration_path);
        let ledger_unreadable = OutputEnvelope::ledger_error(CommandVerb::Check, &malformed);

        assert_eq!(unconfigured.status, OutputStatus::Unconfigured);
        assert_eq!(ledger_unreadable.status, OutputStatus::LedgerUnreadable);
        assert_eq!(
            unconfigured.exit_code,
            crate::exit::BerthExit::LedgerUnreadable
        );
        assert_eq!(
            ledger_unreadable.exit_code,
            crate::exit::BerthExit::LedgerUnreadable
        );
        assert_eq!(
            unconfigured.presentation,
            EnvelopePresentation::NothingToShow
        );
        assert!(matches!(
            &ledger_unreadable.presentation,
            EnvelopePresentation::RenderedBlocks { blocks }
                if matches!(
                    blocks.as_slice(),
                    [block]
                        if block.summary == LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE
                            && block.detail == ledger_unreadable.message
                )
        ));
        assert!(
            unconfigured
                .message
                .contains(&expected_configuration_path.display().to_string())
        );
    }

    #[test]
    fn ledger_error_keeps_its_prefix_for_a_malformed_configuration() {
        let malformed = LedgerError::Config(ConfigError::UnknownKey("porthole".to_owned()));

        let unreadable = OutputEnvelope::ledger_error(CommandVerb::Init, &malformed);

        assert_eq!(unreadable.status, OutputStatus::LedgerUnreadable);
        assert_eq!(unreadable.presentation, EnvelopePresentation::NotProvided);
        assert!(unreadable.message.ends_with(&malformed.to_string()));
        assert!(
            unreadable
                .message
                .contains("ledger configuration failed: unknown berth configuration key: porthole")
        );
    }

    #[test]
    fn unconfigured_post_commit_rendering_is_silent() {
        let output_envelope = OutputEnvelope::unconfigured(
            CommandVerb::Drift,
            &PathBuf::from(".claude/config/berth.toml"),
        );

        assert_eq!(
            output_envelope.presentation,
            EnvelopePresentation::NothingToShow
        );
        assert!(matches!(
            output_envelope.post_commit_rendering(),
            PostCommitRendering::Silent
        ));
    }

    #[test]
    fn terminal_view_failure_has_its_own_status_and_exit_code() {
        let output_envelope =
            OutputEnvelope::terminal_view_failed_after_board_opened("terminal disconnected");

        assert_eq!(output_envelope.status, OutputStatus::TerminalViewFailed);
        assert_eq!(
            output_envelope.exit_code,
            crate::exit::BerthExit::TerminalViewFailed
        );
        assert_eq!(output_envelope.payload.facts, super::OutputFacts::NoFacts);
        assert!(output_envelope.message.contains("terminal disconnected"));
        assert!(output_envelope.message.contains("cargo-berth board --json"));
    }
}
