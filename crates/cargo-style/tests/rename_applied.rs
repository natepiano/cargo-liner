//! Compare rename results at the synced tip with the Python ledger.

mod support;

use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use support::ParityCheckout;
use support::TestResult;
use support::git;
use support::output_with_input;
use support::parity_checkout;

const ORACLE_SOURCE: &[u8] = include_bytes!("oracle/rename_applied.py");

#[derive(Deserialize)]
struct AppliedCase {
    name:   String,
    id:     String,
    result: Value,
}

#[derive(Deserialize)]
struct FailureCase {
    name:        String,
    id:          String,
    diagnostics: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RenamePolicy {
    Supplied,
    Omitted,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum LedgerFixture {
    Main,
    MalformedRules,
    MalformedFindingIds,
}

impl LedgerFixture {
    const fn filename(&self) -> &'static str {
        match self {
            Self::Main => "ledger.sqlite",
            Self::MalformedRules => "malformed_rules.sqlite",
            Self::MalformedFindingIds => "malformed_finding_ids.sqlite",
        }
    }
}

#[derive(Deserialize)]
struct AppliedRequest {
    ledger: LedgerFixture,
    policy: RenamePolicy,
    cases:  Vec<AppliedCase>,
}

#[derive(Deserialize)]
struct OracleAnswers {
    synced_tip: String,
    requests:   Vec<AppliedRequest>,
    failures:   Vec<FailureCase>,
}

#[test]
fn rename_applied_matches_python_in_request_order() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let directory = fixture_tempdir("cargo-style-rename-applied-");
    let root = directory.path();
    let repository = root.join("repo");
    std::fs::create_dir(&repository)?;
    git(
        &repository,
        &[
            "init",
            "-q",
            "--object-format=sha1",
            "--initial-branch=main",
        ],
    )?;
    let oracle = read_oracle(&checkout, root)?;
    for request in &oracle.requests {
        assert_answers(root, request, &oracle.synced_tip)?;
    }
    let first = oracle
        .requests
        .iter()
        .flat_map(|request| &request.cases)
        .next()
        .ok_or("Python did not provide a finding")?;
    for failure in &oracle.failures {
        assert_failed_request(root, &first.id, failure)?;
    }
    Ok(())
}

fn read_oracle(checkout: &Path, root: &Path) -> TestResult<OracleAnswers> {
    let home = root.join("home");
    std::fs::create_dir(&home)?;
    let output = output_with_input(
        FixtureCommand::new(checkout.join("scripts/lib/py"))
            .current_dir(checkout)
            .env("HOME", home)
            .env("XDG_CACHE_HOME", root.join("python-cache"))
            .env("CONF_DIR", root.join("config"))
            .env("STYLE_TEST_READER_SOURCE", "python")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-")
            .arg(root),
        ORACLE_SOURCE,
    )?;
    assert!(
        output.status.success(),
        "Python rename applied oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn finding(
    root: &Path,
    fixture: &LedgerFixture,
    policy: &RenamePolicy,
    ids: &[&str],
) -> TestResult<Output> {
    let ledger = root.join(fixture.filename());
    let before = std::fs::read(&ledger)?;
    let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
    command
        .env("HOME", root.join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("finding")
        .arg("--repo")
        .arg(root.join("repo"))
        .arg("--ledger")
        .arg(&ledger);
    if matches!(policy, RenamePolicy::Supplied) {
        command
            .arg("--rename-policy")
            .arg(root.join("rename_policy.toml"));
    }
    let output = output_with_input(command.args(ids), &[])?;
    assert_eq!(std::fs::read(ledger)?, before, "the ledger bytes changed");
    Ok(output)
}

fn assert_answers(root: &Path, request: &AppliedRequest, tip: &str) -> TestResult<()> {
    let ids = request
        .cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<Vec<_>>();
    let output = finding(root, &request.ledger, &request.policy, &ids)?;
    assert!(
        output.status.success(),
        "cargo style finding: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr.as_slice(), b"");
    let reply: Value = serde_json::from_slice(&output.stdout)?;
    let findings = reply["findings"]
        .as_array()
        .ok_or("missing finding replies")?;
    assert_eq!(findings.len(), request.cases.len());
    for (actual, expected) in findings.iter().zip(&request.cases) {
        assert_eq!(actual["id"], expected.id, "{}", expected.name);
        if expected.result["status"] == "unknown_finding" {
            assert_eq!(
                actual,
                &json!({"id": expected.id, "status": "unknown_finding"})
            );
        } else {
            let applied = actual
                .get("rename_applied")
                .ok_or_else(|| format!("{}: missing rename_applied", expected.name))?;
            assert_eq!(applied, &expected.result, "{}", expected.name);
        }
    }
    assert_eq!(reply["schema"], 5);
    assert_eq!(
        reply["observed"],
        json!({"synced_tip": tip, "ledger_revision": "absent"})
    );
    Ok(())
}

fn assert_failed_request(root: &Path, first: &str, failure: &FailureCase) -> TestResult<()> {
    let output = finding(
        root,
        &LedgerFixture::Main,
        &RenamePolicy::Supplied,
        &[first, &failure.id],
    )?;
    assert!(!output.status.success(), "{}", failure.name);
    assert_eq!(output.stdout.as_slice(), b"", "{}", failure.name);
    let diagnostic = String::from_utf8(output.stderr)?;
    assert_eq!(
        diagnostic.lines().count(),
        1,
        "{}: {diagnostic}",
        failure.name
    );
    for expected in &failure.diagnostics {
        assert!(
            diagnostic.contains(expected.as_str()),
            "{}: expected {expected:?} in {diagnostic}",
            failure.name
        );
    }
    Ok(())
}
