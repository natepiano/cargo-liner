//! Compare rename admission with the Python ledger for the same frozen inputs.

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

const ORACLE_SOURCE: &[u8] = include_bytes!("oracle/rename_admission.py");
const POLICY_FLAG: &str = "--rename-policy";

#[derive(Deserialize)]
struct AdmissionCase {
    name:   String,
    answer: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RenamePolicyFixture {
    Omitted,
    Valid,
    Malformed,
    Absent,
    HexEscape,
    InlineTableTrailingComma,
    DuplicateTable,
    AbsentZeroWidthSpace,
}

impl RenamePolicyFixture {
    fn apply(&self, command: &mut FixtureCommand, root: &Path) {
        let directory = match self {
            Self::Omitted => return,
            Self::Valid => "valid",
            Self::Malformed => "malformed",
            Self::Absent => "absent",
            Self::HexEscape => "hex_escape",
            Self::InlineTableTrailingComma => "inline_table_trailing_comma",
            Self::DuplicateTable => "duplicate_table",
            Self::AbsentZeroWidthSpace => "absent-\u{200b}",
        };
        command
            .arg(POLICY_FLAG)
            .arg(root.join(directory).join("rename_policy.toml"));
    }
}

#[derive(Deserialize)]
struct AdmissionRequest {
    policy: RenamePolicyFixture,
    cases:  Vec<AdmissionCase>,
}

#[derive(Deserialize)]
struct OracleAnswers {
    synced_tip:      String,
    policy_required: String,
    requests:        Vec<AdmissionRequest>,
}

#[test]
fn rename_admission_matches_python_in_request_order() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let directory = fixture_tempdir("cargo-style-rename-admission-");
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
        assert_reply(root, request, &oracle.synced_tip)?;
    }
    assert_policy_required(root, &oracle.policy_required)
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
        "Python rename admission oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn finding(root: &Path, policy: &RenamePolicyFixture, ids: &[&str]) -> TestResult<Output> {
    let ledger = root.join("ledger.sqlite");
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
    policy.apply(&mut command, root);
    let output = output_with_input(command.args(ids), &[])?;
    assert_eq!(std::fs::read(ledger)?, before, "the ledger bytes changed");
    Ok(output)
}

fn assert_reply(root: &Path, request: &AdmissionRequest, tip: &str) -> TestResult<()> {
    let ids = request
        .cases
        .iter()
        .map(|case| {
            case.answer["id"]
                .as_str()
                .ok_or("Python did not give a finding id")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let output = finding(root, &request.policy, &ids)?;
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
        assert_eq!(actual, &expected.answer, "{}", expected.name);
    }
    assert_eq!(reply["schema"], 3);
    assert_eq!(
        reply["observed"],
        json!({"synced_tip":tip, "ledger_revision":"absent"})
    );
    Ok(())
}

fn assert_policy_required(root: &Path, id: &str) -> TestResult<()> {
    let output = finding(root, &RenamePolicyFixture::Omitted, &[id])?;
    assert!(!output.status.success(), "the local rename needs a policy");
    assert_eq!(output.stdout.as_slice(), b"");
    let diagnostic = String::from_utf8(output.stderr)?;
    assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
    assert!(
        diagnostic.contains("rename admission requires --rename-policy <rename_policy.toml>"),
        "{diagnostic}"
    );
    Ok(())
}
