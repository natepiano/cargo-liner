//! Independent acceptance oracle for the front-end envelope shell.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use serde::Serialize;
use serde_json::Value;
use tempfile::TempDir;

const CARGO_BERTH_SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const EXPECTED_CORPUS_ENTRIES: usize = 50;
const EXPECTED_UNCOVERED_CORPUS_ENTRIES: usize = 45;
/// Corpus entries whose frozen text `tests/hooks.rs` compares against the real
/// binary, each named beside the test that drives it.
const HOOK_ACCEPTANCE_TEXT_COMPARED_ENTRIES: [(&str, &str); 4] = [
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message",
        "session_identity_recoveries_preserve_the_frozen_corpus_text",
    ),
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message#3",
        "stale_marker_recovery_preserves_the_frozen_corpus_text",
    ),
    (
        "test_hooks_render_coordination_identity_recovery_actions_without_message#5",
        "session_identity_recoveries_preserve_the_frozen_corpus_text",
    ),
    (
        "test_typed_replay_failure_routes_without_message_in_every_consumer",
        "replay_failure_emits_a_fail_open_object",
    ),
];
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const FRONT_END_CORPUS_JSON: &str = include_str!("fixtures/front_end_corpus.json");
const GENERATED_CONTRACT_JSON: &str =
    include_str!("../../../docs/cargo-berth/generated/output-contract.json");
const SCRATCH_ROOT: &str = "/tmp/claude";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";

type ShellOracleResult<T> = Result<T, Box<dyn Error>>;

#[derive(Debug)]
struct RenderedOutputBlockEvidence<'a> {
    summary: &'a str,
    detail:  &'a str,
}

enum CorpusEntryCoverage {
    /// This suite compares the entry's frozen text against real engine output.
    TextCompared,
    /// `tests/hooks.rs` compares the entry's frozen text against the real binary.
    TextComparedByHookAcceptance,
    /// No suite compares this entry's frozen text against anything the engine emits.
    Uncovered(UncoveredCorpusEntry),
}

struct UncoveredCorpusEntry {
    name:       String,
    diagnostic: String,
}

#[derive(Serialize)]
struct GeneratedShellCases {
    accepted: Vec<NamedShellEnvelope>,
    rejected: Vec<NamedShellEnvelope>,
}

#[derive(Serialize)]
struct NamedShellEnvelope {
    name:     String,
    envelope: Value,
}

#[test]
fn real_shell_rejects_malformation_and_preserves_compatibility() -> ShellOracleResult<()> {
    let real_envelope = ambiguous_first_touch_envelope()?;
    let rendered_output_blocks = rendered_output_blocks(&real_envelope)?;
    if rendered_output_blocks.is_empty() {
        return Err(failure(
            "real ambiguous first-touch envelope should carry a rendered presentation block",
        ));
    }
    if !rendered_output_blocks.iter().any(|block| {
        !block.summary.is_empty() && block.detail.contains("cargo-berth check --reservation")
    }) {
        return Err(failure(format!(
            "real ambiguous first-touch presentation should retain its selection command: {rendered_output_blocks:#?}"
        )));
    }

    let generated_contract: Value = serde_json::from_str(GENERATED_CONTRACT_JSON)?;
    let generated_shell_cases = generated_shell_cases(real_envelope)?;
    run_python_shell_consumer(&generated_contract, &generated_shell_cases)?;
    run_jq_shell_consumer(&generated_contract, &generated_shell_cases)
}

#[test]
fn every_corpus_entry_is_text_compared_or_reported_uncovered() -> ShellOracleResult<()> {
    let real_envelope = ambiguous_first_touch_envelope()?;
    let rendered_output_blocks = rendered_output_blocks(&real_envelope)?;
    let corpus: Value = serde_json::from_str(FRONT_END_CORPUS_JSON)?;
    let entries = corpus
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("front-end corpus should carry an entries array"))?;
    if entries.len() != EXPECTED_CORPUS_ENTRIES {
        return Err(failure(format!(
            "front-end corpus entry count changed: expected {EXPECTED_CORPUS_ENTRIES}, found {}",
            entries.len()
        )));
    }

    let coverage = entries
        .iter()
        .map(|entry| classify_corpus_entry(entry, &real_envelope, &rendered_output_blocks))
        .collect::<ShellOracleResult<Vec<_>>>()?;
    let compared_by_hook_acceptance = coverage
        .iter()
        .filter(|coverage| matches!(coverage, CorpusEntryCoverage::TextComparedByHookAcceptance))
        .count();
    if compared_by_hook_acceptance != HOOK_ACCEPTANCE_TEXT_COMPARED_ENTRIES.len() {
        return Err(failure(format!(
            "the hook acceptance suite names {} corpus entries but {compared_by_hook_acceptance} of them exist",
            HOOK_ACCEPTANCE_TEXT_COMPARED_ENTRIES.len()
        )));
    }
    let uncovered = coverage
        .into_iter()
        .filter_map(|coverage| match coverage {
            CorpusEntryCoverage::TextCompared
            | CorpusEntryCoverage::TextComparedByHookAcceptance => None,
            CorpusEntryCoverage::Uncovered(entry) => Some(entry),
        })
        .collect::<Vec<_>>();
    for entry in &uncovered {
        eprintln!("UNCOVERED {}: {}", entry.name, entry.diagnostic);
    }
    if uncovered.len() != EXPECTED_UNCOVERED_CORPUS_ENTRIES {
        return Err(failure(format!(
            "front-end corpus uncovered count changed: expected {EXPECTED_UNCOVERED_CORPUS_ENTRIES}, found {}",
            uncovered.len()
        )));
    }
    Ok(())
}

fn classify_corpus_entry(
    entry: &Value,
    real_envelope: &Value,
    rendered_output_blocks: &[RenderedOutputBlockEvidence<'_>],
) -> ShellOracleResult<CorpusEntryCoverage> {
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| failure("every front-end corpus entry should carry a name"))?;
    if name == "test_pre_edit_renders_an_ambiguous_first_touch_from_the_engine_message" {
        compare_ambiguous_first_touch_text(entry, real_envelope, rendered_output_blocks)?;
        return Ok(CorpusEntryCoverage::TextCompared);
    }
    if HOOK_ACCEPTANCE_TEXT_COMPARED_ENTRIES
        .iter()
        .any(|(compared_name, _)| *compared_name == name)
    {
        return Ok(CorpusEntryCoverage::TextComparedByHookAcceptance);
    }
    Ok(CorpusEntryCoverage::Uncovered(UncoveredCorpusEntry {
        name:       name.to_owned(),
        diagnostic: uncovered_corpus_diagnostic(entry)?,
    }))
}

fn compare_ambiguous_first_touch_text(
    entry: &Value,
    real_envelope: &Value,
    rendered_output_blocks: &[RenderedOutputBlockEvidence<'_>],
) -> ShellOracleResult<()> {
    let [block] = rendered_output_blocks else {
        return Err(failure(format!(
            "real ambiguous first-touch response should render exactly one block, found {}",
            rendered_output_blocks.len()
        )));
    };
    let expected = entry
        .get("expected")
        .and_then(|expected| expected.get("stderr"))
        .and_then(Value::as_str)
        .ok_or_else(|| failure("ambiguous first-touch corpus entry should carry stderr"))?
        .trim_end_matches('\n');
    let normalized_detail = normalize_reservation_ids(entry, real_envelope, block.detail)?;
    if normalized_detail != expected {
        return Err(failure(format!(
            "ambiguous first-touch production presentation differs from the corpus:\nexpected={expected:?}\nactual={normalized_detail:?}"
        )));
    }
    Ok(())
}

fn normalize_reservation_ids(
    entry: &Value,
    real_envelope: &Value,
    rendered_detail: &str,
) -> ShellOracleResult<String> {
    let actual = real_envelope
        .get("reservations")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("real ambiguous response should carry reservations"))?;
    let expected = entry
        .get("engine_responses")
        .and_then(|responses| responses.get("check"))
        .and_then(|response| response.get("body"))
        .and_then(|body| body.get("reservations"))
        .and_then(Value::as_array)
        .ok_or_else(|| failure("ambiguous corpus response should carry reservations"))?;
    if actual.len() != expected.len() {
        return Err(failure(format!(
            "ambiguous reservation count differs: real={}, corpus={}",
            actual.len(),
            expected.len()
        )));
    }
    actual.iter().zip(expected).try_fold(
        rendered_detail.to_owned(),
        |detail, (actual, expected)| {
            let actual = actual
                .as_str()
                .ok_or_else(|| failure("real ambiguous reservation id should be a string"))?;
            let expected = expected
                .as_str()
                .ok_or_else(|| failure("corpus ambiguous reservation id should be a string"))?;
            Ok(detail.replace(actual, expected))
        },
    )
}

fn uncovered_corpus_diagnostic(entry: &Value) -> ShellOracleResult<String> {
    let engine_responses = entry
        .get("engine_responses")
        .and_then(Value::as_object)
        .ok_or_else(|| failure("front-end corpus entry should carry engine_responses"))?;
    if engine_responses.is_empty() {
        return Ok(
            "the case models a front-end installation failure before cargo-berth runs".to_owned(),
        );
    }
    let routes = engine_responses
        .iter()
        .map(|(invocation, response)| {
            let body = response.get("body").and_then(Value::as_object);
            let verb = body
                .and_then(|body| body.get("verb"))
                .and_then(Value::as_str)
                .unwrap_or("missing-verb");
            let status = body
                .and_then(|body| body.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("missing-status");
            format!("{invocation}:{verb}/{status}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    let synthetic_future_envelope = engine_responses.values().any(|response| {
        response
            .get("body")
            .and_then(Value::as_object)
            .is_some_and(|body| {
                ["status", "payload"]
                    .iter()
                    .filter_map(|member| body.get(*member))
                    .any(|member| member.to_string().contains("no_installed_table_names"))
            })
    });
    if synthetic_future_envelope {
        Ok(format!(
            "the corpus supplies a synthetic future envelope that this binary cannot emit ({routes})"
        ))
    } else {
        Ok(format!(
            "front_end_corpus.rs has no real-binary setup that produces {routes}"
        ))
    }
}

fn ambiguous_first_touch_envelope() -> ShellOracleResult<Value> {
    fs::create_dir_all(SCRATCH_ROOT)?;
    let repository = TempDir::new_in(SCRATCH_ROOT)?;
    run_git(repository.path(), &["init", "-b", "main"])?;
    run_git(
        repository.path(),
        &["config", "user.email", "oracle@example.invalid"],
    )?;
    run_git(
        repository.path(),
        &["config", "user.name", "Front End Oracle"],
    )?;
    fs::write(
        repository.path().join("README.md"),
        "front-end shell oracle\n",
    )?;
    run_git(repository.path(), &["add", "README.md"])?;
    run_git(
        repository.path(),
        &["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
    )?;

    let initialized = run_berth(repository.path(), &["init", "--json"], "shell-init")?;
    require_success(&initialized, "scratch cargo-berth init")?;
    let older_claim = run_berth(
        repository.path(),
        &["claim", "tree:shared", "--run", FIRST_RUN, "--json"],
        "shell-selection",
    )?;
    require_success(&older_claim, "older overlapping claim")?;
    let newer_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "file:shared/child.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
        "shell-selection",
    )?;
    require_success(&newer_claim, "newer overlapping claim")?;
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))?;

    let ambiguous = run_berth(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
        "shell-selection",
    )?;
    if ambiguous.status.code() != Some(1) {
        return Err(failure(format!(
            "ambiguous first touch should exit 1: stdout={} stderr={}",
            String::from_utf8_lossy(&ambiguous.stdout),
            String::from_utf8_lossy(&ambiguous.stderr)
        )));
    }
    Ok(serde_json::from_slice(&ambiguous.stdout)?)
}

fn run_git(repository: &Path, arguments: &[&str]) -> ShellOracleResult<()> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()?;
    if !output.status.success() {
        return Err(failure(format!(
            "git {} failed in scratch repository: stdout={} stderr={}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn run_berth(repository: &Path, arguments: &[&str], session_id: &str) -> ShellOracleResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository)
        .env(CARGO_BERTH_SESSION_ENVIRONMENT, session_id)
        .output()?)
}

fn require_success(output: &Output, operation: &str) -> ShellOracleResult<()> {
    if !output.status.success() {
        return Err(failure(format!(
            "{operation} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn rendered_output_blocks(
    envelope: &Value,
) -> ShellOracleResult<Vec<RenderedOutputBlockEvidence<'_>>> {
    let envelope = required_object(envelope, "real cargo-berth envelope")?;
    let presentation = envelope
        .get("presentation")
        .ok_or_else(|| failure("real cargo-berth envelope should carry presentation"))?;
    let presentation = required_object(presentation, "real cargo-berth presentation")?;
    let kind = required_string_member(presentation, "kind", "real cargo-berth presentation")?;
    if kind != "rendered_blocks" {
        return Err(failure(format!(
            "real ambiguous first-touch presentation should use rendered_blocks, found {kind:?}"
        )));
    }
    let blocks = presentation
        .get("blocks")
        .and_then(Value::as_array)
        .ok_or_else(|| failure("real rendered_blocks presentation should carry an array"))?;
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| rendered_output_block(block, index))
        .collect()
}

fn rendered_output_block(
    block: &Value,
    index: usize,
) -> ShellOracleResult<RenderedOutputBlockEvidence<'_>> {
    let context = format!("real cargo-berth presentation block {index}");
    let block = required_object(block, context.as_str())?;
    let summary = required_string_member(block, "summary", context.as_str())?;
    let detail = required_string_member(block, "detail", context.as_str())?;
    Ok(RenderedOutputBlockEvidence { summary, detail })
}

fn generated_shell_cases(real_envelope: Value) -> ShellOracleResult<GeneratedShellCases> {
    let base_envelope = serde_json::json!({
        "verb": "check",
        "status": "clear",
        "exit_code": 0,
        "message": "",
        "reservations": [],
        "blocked_by": [],
        "presentation": {
            "kind": "rendered_blocks",
            "blocks": [{
                "summary": "engine presentation summary",
                "detail": "engine presentation detail"
            }]
        }
    });
    let mut accepted = compatible_outer_shell_envelopes(&base_envelope)?;
    accepted.extend(compatible_presentation_envelopes(&base_envelope)?);
    accepted.push(named_envelope(
        "real cargo-berth envelope",
        real_envelope.clone(),
    ));

    let mut payload_ignored = real_envelope;
    insert_member(&mut payload_ignored, "payload", serde_json::json!(17))?;
    accepted.push(named_envelope(
        "real shell with payload ignored",
        payload_ignored,
    ));

    let mut rejected = malformed_outer_shell_envelopes(&base_envelope)?;
    rejected.extend(malformed_presentation_envelopes(&base_envelope)?);
    Ok(GeneratedShellCases { accepted, rejected })
}

fn compatible_outer_shell_envelopes(base: &Value) -> ShellOracleResult<Vec<NamedShellEnvelope>> {
    let mut extra_field = base.clone();
    insert_member(
        &mut extra_field,
        "member_this_version_never_named",
        serde_json::json!({"future": [1, 2, 3]}),
    )?;
    Ok(vec![
        named_envelope("minimal familiar shell", base.clone()),
        named_envelope(
            "unfamiliar verb value",
            replaced_member(base, "verb", serde_json::json!("future_verb"))?,
        ),
        named_envelope(
            "unfamiliar status value",
            replaced_member(base, "status", serde_json::json!("future_status"))?,
        ),
        named_envelope(
            "unfamiliar exit value",
            replaced_member(base, "exit_code", serde_json::json!(91))?,
        ),
        named_envelope("extra top-level member", extra_field),
    ])
}

fn compatible_presentation_envelopes(base: &Value) -> ShellOracleResult<Vec<NamedShellEnvelope>> {
    Ok(vec![
        named_envelope(
            "unfamiliar presentation kind",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({
                    "kind": "future_presentation",
                    "future_member": {"nested": true}
                }),
            )?,
        ),
        named_envelope(
            "extra rendered-blocks members",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({
                    "kind": "rendered_blocks",
                    "blocks": [{
                        "summary": "summary",
                        "detail": "detail",
                        "future_block_member": true
                    }],
                    "future_presentation_member": true
                }),
            )?,
        ),
        named_envelope(
            "not-provided presentation with extra member",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({"kind": "not_provided", "future_member": true}),
            )?,
        ),
    ])
}

fn malformed_outer_shell_envelopes(base: &Value) -> ShellOracleResult<Vec<NamedShellEnvelope>> {
    Ok(vec![
        named_envelope(
            "verb has the wrong type",
            replaced_member(base, "verb", serde_json::json!(3))?,
        ),
        named_envelope(
            "status has the wrong type",
            replaced_member(base, "status", serde_json::json!(["clear"]))?,
        ),
        named_envelope(
            "exit_code has the wrong type",
            replaced_member(base, "exit_code", serde_json::json!(true))?,
        ),
        named_envelope(
            "exit_code has a fractional representation",
            replaced_member(base, "exit_code", serde_json::json!(1.5))?,
        ),
        named_envelope(
            "message has the wrong type",
            replaced_member(base, "message", serde_json::json!({}))?,
        ),
        named_envelope(
            "reservations is missing",
            removed_member(base, "reservations")?,
        ),
        named_envelope(
            "reservations has the wrong type",
            replaced_member(base, "reservations", serde_json::json!({}))?,
        ),
        named_envelope(
            "reservations member has the wrong type",
            replaced_member(base, "reservations", serde_json::json!([5]))?,
        ),
        named_envelope(
            "blocked_by has the wrong type",
            replaced_member(base, "blocked_by", serde_json::json!("reservation-a"))?,
        ),
        named_envelope(
            "blocked_by member has the wrong type",
            replaced_member(base, "blocked_by", serde_json::json!([5]))?,
        ),
    ])
}

fn malformed_presentation_envelopes(base: &Value) -> ShellOracleResult<Vec<NamedShellEnvelope>> {
    Ok(vec![
        named_envelope(
            "presentation has the wrong type",
            replaced_member(base, "presentation", serde_json::json!([]))?,
        ),
        named_envelope(
            "presentation kind has the wrong type",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({"kind": 4, "blocks": []}),
            )?,
        ),
        named_envelope(
            "rendered blocks has the wrong type",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({"kind": "rendered_blocks", "blocks": "presented"}),
            )?,
        ),
        named_envelope(
            "rendered block has the wrong type",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({"kind": "rendered_blocks", "blocks": [7]}),
            )?,
        ),
        named_envelope(
            "rendered block summary has the wrong type",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({
                    "kind": "rendered_blocks",
                    "blocks": [{"summary": 7, "detail": "detail"}]
                }),
            )?,
        ),
        named_envelope(
            "rendered block detail has the wrong type",
            replaced_member(
                base,
                "presentation",
                serde_json::json!({
                    "kind": "rendered_blocks",
                    "blocks": [{"summary": "summary", "detail": 7}]
                }),
            )?,
        ),
    ])
}

fn named_envelope(name: &str, envelope: Value) -> NamedShellEnvelope {
    NamedShellEnvelope {
        name: name.to_owned(),
        envelope,
    }
}

fn replaced_member(base: &Value, member: &str, value: Value) -> ShellOracleResult<Value> {
    let mut envelope = base.clone();
    insert_member(&mut envelope, member, value)?;
    Ok(envelope)
}

fn removed_member(base: &Value, member: &str) -> ShellOracleResult<Value> {
    let mut envelope = base.clone();
    envelope
        .as_object_mut()
        .ok_or_else(|| failure("synthetic shell envelope should be an object"))?
        .remove(member)
        .ok_or_else(|| failure(format!("synthetic shell envelope should carry {member}")))?;
    Ok(envelope)
}

fn insert_member(envelope: &mut Value, member: &str, value: Value) -> ShellOracleResult<()> {
    let envelope = envelope
        .as_object_mut()
        .ok_or_else(|| failure("synthetic shell envelope should be an object"))?;
    envelope.insert(member.to_owned(), value);
    Ok(())
}

fn run_python_shell_consumer(
    generated_contract: &Value,
    cases: &GeneratedShellCases,
) -> ShellOracleResult<()> {
    let temporary = TempDir::new_in(SCRATCH_ROOT)?;
    let consumer_path = temporary.path().join("status_payload_tables.py");
    let cases_path = temporary.path().join("shell_cases.json");
    let consumer = required_contract_artifact(generated_contract, "status_payload_tables")?;
    fs::write(&consumer_path, consumer)?;
    fs::write(&cases_path, serde_json::to_vec(cases)?)?;
    let program = r#"import importlib.util
import json
import sys

spec = importlib.util.spec_from_file_location("generated_tables", sys.argv[1])
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
with open(sys.argv[2], encoding="utf-8") as cases_file:
    cases = json.load(cases_file)

failures = []
for case in cases["accepted"]:
    if not module.valid_contract_envelope(case["envelope"]):
        failures.append(f"rejected accepted case: {case['name']}")
for case in cases["rejected"]:
    if module.valid_contract_envelope(case["envelope"]):
        failures.append(f"accepted rejected case: {case['name']}")
if failures:
    print("\n".join(failures))
    raise SystemExit(1)
"#;
    let output = Command::new("python3")
        .args(["-c", program])
        .arg(consumer_path)
        .arg(cases_path)
        .output()?;
    require_consumer_success(&output, "Python")
}

fn run_jq_shell_consumer(
    generated_contract: &Value,
    cases: &GeneratedShellCases,
) -> ShellOracleResult<()> {
    let temporary = TempDir::new_in(SCRATCH_ROOT)?;
    let consumer_path = temporary.path().join("envelope_validation.jq");
    let cases_path = temporary.path().join("shell_cases.json");
    let mut consumer =
        required_contract_artifact(generated_contract, "envelope_validation_jq")?.to_owned();
    consumer.push_str(
        r"
. as $cases |
(all($cases.accepted[]; (.envelope | cargo_berth_valid_contract_envelope))) and
(all($cases.rejected[]; ((.envelope | cargo_berth_valid_contract_envelope) | not)))
",
    );
    fs::write(&consumer_path, consumer)?;
    fs::write(&cases_path, serde_json::to_vec(cases)?)?;
    let output = Command::new("jq")
        .args(["--exit-status", "-f"])
        .arg(consumer_path)
        .arg(cases_path)
        .output()?;
    require_consumer_success(&output, "jq")
}

fn require_consumer_success(output: &Output, consumer: &str) -> ShellOracleResult<()> {
    if !output.status.success() {
        return Err(failure(format!(
            "generated {consumer} shell consumer disagreed with the independent cases:\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

fn required_contract_artifact<'a>(
    generated_contract: &'a Value,
    artifact: &str,
) -> ShellOracleResult<&'a str> {
    generated_contract
        .get("consumer_artifacts")
        .and_then(|artifacts| artifacts.get(artifact))
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("generated contract should embed {artifact}")))
}

fn required_object<'a>(
    value: &'a Value,
    context: &str,
) -> ShellOracleResult<&'a serde_json::Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| failure(format!("{context} should be an object")))
}

fn required_string_member<'a>(
    object: &'a serde_json::Map<String, Value>,
    member: &str,
    context: &str,
) -> ShellOracleResult<&'a str> {
    object
        .get(member)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{context} should carry string member {member}")))
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}
