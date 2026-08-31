//! Regression coverage for selector branches in the generated output contract.

use std::collections::BTreeSet;
use std::error::Error;
use std::io;

use serde_json::Map;
use serde_json::Value;

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
