//! Generated JSON schemas publishing this engine's documented output contract.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use schemars::Schema;
use serde_json::Error;
use serde_json::Value;
use serde_json::json;

use crate::alert::LostIntegrationEvidenceAlert;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::ledger::CanonicalWorktreeRoot;
use crate::output;
use crate::output::CLOSED_VALUE_SELECTOR_TRANSFORM_FAILURE_KEY;
use crate::output::IdentityPayload;
use crate::output::IntegrationPayload;
use crate::output::OUTPUT_CONTRACT_VERSION;
use crate::output::OutputEnvelope;
use crate::output::ReplayFailurePayload;
use crate::output::ReservationLifecycleQueryPayload;
use crate::output::ResolvePayload;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::IntegrationProof;
use crate::reservation::IntegrationWitness;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationLifecycleSnapshot;
use crate::reservation::ScopedPatchEvaluatorVersion;

const CONTRACT_NAME: &str = "cargo-berth-output";

type GeneratedSchemas = BTreeMap<String, Value>;

#[derive(Clone, Copy)]
enum ClosedValueSelectorTransformStatus<'schema> {
    Complete,
    Failed(&'schema Value),
}

/// Regenerate the checked-in output contract entirely in memory.
fn generate_output_contract() -> Result<String, Error> {
    generate_output_contract_with_reservation_lifecycle::<ReservationLifecycleSnapshot>()
}

fn generate_output_contract_with_reservation_lifecycle<LifecycleSchema: JsonSchema>()
-> Result<String, Error> {
    let contract = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "contract": CONTRACT_NAME,
        "version": OUTPUT_CONTRACT_VERSION,
        "schemas": generated_schemas::<LifecycleSchema>()?,
    });
    let mut serialized = serde_json::to_string_pretty(&contract)?;
    serialized.push('\n');
    Ok(serialized)
}

fn generated_schemas<LifecycleSchema: JsonSchema>() -> Result<GeneratedSchemas, Error> {
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
        "scoped_patch_evaluator_version".to_owned(),
        schema_value::<ScopedPatchEvaluatorVersion>()?,
    );
    schemas.insert(
        "integration_witness".to_owned(),
        schema_value::<IntegrationWitness>()?,
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
        checked_schema_value(output::output_facts_schema())?,
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
        schema_value::<LifecycleSchema>()?,
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

fn schema_value<SchemaType: JsonSchema>() -> Result<Value, Error> {
    checked_schema_value(schemars::schema_for!(SchemaType))
}

fn checked_schema_value(schema: Schema) -> Result<Value, Error> {
    let schema_value = serde_json::to_value(schema)?;
    assert_closed_value_selector_transforms_completed(&schema_value)?;
    Ok(schema_value)
}

fn assert_closed_value_selector_transforms_completed(schema: &Value) -> Result<(), Error> {
    match closed_value_selector_transform_status(schema) {
        ClosedValueSelectorTransformStatus::Complete => Ok(()),
        ClosedValueSelectorTransformStatus::Failed(failure) => {
            Err(<Error as serde::ser::Error>::custom(format!(
                "closed-value selector transform failed: {failure}"
            )))
        },
    }
}

fn closed_value_selector_transform_status(
    schema: &Value,
) -> ClosedValueSelectorTransformStatus<'_> {
    match schema {
        Value::Array(members) => nested_transform_status(members),
        Value::Object(members) => members
            .get(CLOSED_VALUE_SELECTOR_TRANSFORM_FAILURE_KEY)
            .map_or_else(
                || nested_transform_status(members.values()),
                ClosedValueSelectorTransformStatus::Failed,
            ),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            ClosedValueSelectorTransformStatus::Complete
        },
    }
}

fn nested_transform_status<'schema>(
    members: impl IntoIterator<Item = &'schema Value>,
) -> ClosedValueSelectorTransformStatus<'schema> {
    members
        .into_iter()
        .find_map(
            |member| match closed_value_selector_transform_status(member) {
                ClosedValueSelectorTransformStatus::Complete => None,
                failed @ ClosedValueSelectorTransformStatus::Failed(_) => Some(failed),
            },
        )
        .unwrap_or(ClosedValueSelectorTransformStatus::Complete)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::path::Path;

    use schemars::Schema;
    use serde_json::Map;
    use serde_json::Value;
    use serde_json::json;

    use super::CONTRACT_NAME;
    use super::checked_schema_value;
    use super::generate_output_contract;
    use crate::output;
    use crate::output::OUTPUT_CONTRACT_VERSION;

    const CHECKED_CONTRACT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/cargo-berth/generated/output-contract.json"
    );

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const FIRST_TOUCH_ACQUISITION_SCHEMA: &str = "FirstTouchReservationAcquisition";
    const GENERATED_OUTPUT_CONTRACT: &str =
        include_str!("../../../docs/cargo-berth/generated/output-contract.json");
    const OUTPUT_ENVELOPE_SCHEMA: &str = "output_envelope";
    const RESERVATION_SCOPE_SCHEMA: &str = "ReservationScope";
    const SELECTOR_MEMBER: &str = "kind";

    type ContractTestResult<T> = Result<T, Box<dyn Error>>;

    #[test]
    fn generated_contract_retains_every_closed_selector_branch() -> ContractTestResult<()> {
        let contract: Value = serde_json::from_str(GENERATED_OUTPUT_CONTRACT)?;
        let schemas = required_object_member(&contract, "schemas")?;
        let output_envelope = required_object_map_member(schemas, OUTPUT_ENVELOPE_SCHEMA)?;
        let definitions = required_object_map_member(output_envelope, "$defs")?;

        assert_selector_branches(
            definitions,
            FIRST_TOUCH_ACQUISITION_SCHEMA,
            &["already_held", "appended", "widened"],
        )?;
        assert_selector_branches(definitions, RESERVATION_SCOPE_SCHEMA, &["file", "tree"])
    }

    fn assert_selector_branches(
        definitions: &Map<String, Value>,
        schema_name: &str,
        expected_values: &[&str],
    ) -> ContractTestResult<()> {
        let schema = definitions
            .get(schema_name)
            .ok_or_else(|| failure(format!("generated contract should define {schema_name}")))?;
        let branches = schema
            .get("oneOf")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                failure(format!(
                    "generated contract definition {schema_name} should enumerate selector branches"
                ))
            })?;
        let actual_values = branches
            .iter()
            .map(|branch| selector_value(branch, schema_name))
            .collect::<ContractTestResult<Vec<_>>>()?;
        let actual_set = actual_values.iter().copied().collect::<BTreeSet<_>>();
        let expected_set = expected_values.iter().copied().collect::<BTreeSet<_>>();

        if actual_values.len() != expected_values.len() || actual_set != expected_set {
            return Err(failure(format!(
                "generated contract definition {schema_name} has selector values {actual_values:?}; expected {expected_values:?}"
            )));
        }
        Ok(())
    }

    fn selector_value<'a>(branch: &'a Value, schema_name: &str) -> ContractTestResult<&'a str> {
        branch
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(SELECTOR_MEMBER))
            .and_then(|selector| selector.get("const"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                failure(format!(
                    "every {schema_name} branch should constrain its {SELECTOR_MEMBER} selector"
                ))
            })
    }

    fn required_object_member<'a>(
        object: &'a Value,
        member: &str,
    ) -> ContractTestResult<&'a Map<String, Value>> {
        object
            .get(member)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                failure(format!(
                    "generated contract should carry object member {member}"
                ))
            })
    }

    fn required_object_map_member<'a>(
        object: &'a Map<String, Value>,
        member: &str,
    ) -> ContractTestResult<&'a Map<String, Value>> {
        object
            .get(member)
            .and_then(Value::as_object)
            .ok_or_else(|| {
                failure(format!(
                    "generated contract should carry object member {member}"
                ))
            })
    }

    fn failure(message: impl Into<String>) -> Box<dyn Error> {
        Box::new(io::Error::other(message.into()))
    }

    #[test]
    fn generated_artifacts_are_reproducible_from_the_checked_in_contract() -> TestResult {
        let generated = generate_output_contract()?;
        if std::env::var_os("CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT").is_some() {
            if let Some(parent) = Path::new(CHECKED_CONTRACT).parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(CHECKED_CONTRACT, &generated)?;
        }
        let checked = fs::read_to_string(CHECKED_CONTRACT)?;
        assert_eq!(generated, checked);
        Ok(())
    }

    #[test]
    fn generated_contract_contains_the_current_shell_and_phase_wires() -> TestResult {
        let generated = generate_output_contract()?;
        let contract: Value = serde_json::from_str(&generated)?;
        assert_eq!(contract["contract"], CONTRACT_NAME);
        assert_eq!(contract["version"], OUTPUT_CONTRACT_VERSION);

        assert!(generated.contains("output_contract_version"));
        assert!(generated.contains("presentation"));
        assert!(generated.contains("not_provided"));
        assert!(generated.contains("rendered_blocks"));
        assert!(generated.contains("ambiguous_active_run_reservations"));
        assert!(generated.contains("first_touch_reservation_selection"));
        Ok(())
    }

    #[test]
    fn selector_transform_failures_stop_output_contract_generation() -> TestResult {
        let malformed_schemas = [
            json!({
                "type": "object",
                "properties": {"kind": {"type": "string"}},
                "required": ["kind"],
            }),
            json!({
                "type": "object",
                "properties": {"kind": {"type": "string", "enum": []}},
                "required": ["kind"],
            }),
            json!({
                "type": "object",
                "properties": {"kind": {"type": "string", "enum": ["file", "file"]}},
                "required": ["kind"],
            }),
            json!({
                "type": "string",
                "properties": {"kind": {"type": "string", "enum": ["file", "tree"]}},
                "required": ["kind"],
            }),
            json!({
                "type": "object",
                "properties": {"kind": {"type": "string", "enum": ["file", "tree"]}},
            }),
        ];

        for schema_value in malformed_schemas {
            let mut schema = Schema::try_from(schema_value)?;
            output::closed_value_serializes_as_object(&mut schema);
            assert!(checked_schema_value(schema).is_err());
        }
        Ok(())
    }
}
