//! Compare coverage with the Python ledger on private Git and SQLite fixtures.

#[expect(
    dead_code,
    reason = "coverage fixtures come from the Python oracle, so the Git helpers stay unused"
)]
mod support;
#[path = "support/wal.rs"]
mod wal;

use std::fs;
use std::path::Path;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use serde::Deserialize;
use serde_json::Value;
use support::ParityCheckout;
use support::TestResult;
use support::output_with_input;
use support::parity_checkout;
use wal::read_wal;

const ORACLE: &[u8] = include_bytes!("coverage/oracle.py");

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OracleCommand {
    #[default]
    Coverage,
    Sites,
}

#[derive(Deserialize)]
struct CoverageCase {
    name:    String,
    ledger:  String,
    rules:   String,
    flags:   Vec<String>,
    #[serde(default)]
    command: OracleCommand,
    result:  ExpectedCoverage,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExpectedCoverage {
    Reply { body: Value },
    Refused { diagnostic: String },
}

#[test]
fn coverage_matches_python_cells_and_rule_counts() -> TestResult<()> { compare_group("judgments") }

#[test]
fn coverage_membership_matches_python() -> TestResult<()> { compare_group("membership") }

#[test]
fn coverage_refresh_matches_python() -> TestResult<()> { compare_group("refresh") }

fn compare_group(group: &str) -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let fixture = fixture_tempdir("cargo-style-coverage-");
    let home = fixture.path().join("home");
    fs::create_dir(&home)?;
    let cases = oracle_cases(&checkout, fixture.path(), group)?;
    for case in cases {
        compare_case(fixture.path(), &case)?;
    }
    Ok(())
}

fn oracle_cases(checkout: &Path, root: &Path, group: &str) -> TestResult<Vec<CoverageCase>> {
    let output = output_with_input(
        FixtureCommand::new(checkout.join("scripts/lib/py"))
            .current_dir(checkout)
            .env("HOME", root.join("home"))
            .env("XDG_CACHE_HOME", root.join("python-cache"))
            .env("XDG_CONFIG_HOME", root.join("xdg-config"))
            .env("CONF_DIR", root.join("config"))
            .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("global-rules"))
            .env("STYLE_TEST_READER_SOURCE", "python")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-")
            .arg(root)
            .arg(group),
        ORACLE,
    )?;
    assert!(
        output.status.success(),
        "Python coverage oracle ({group}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn compare_case(root: &Path, case: &CoverageCase) -> TestResult<()> {
    let ledger = root.join(&case.ledger);
    let before = fs::read(&ledger)?;
    let wal_before = read_wal(&ledger)?;
    let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
    command
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("global-rules"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg(match case.command {
            OracleCommand::Coverage => "coverage",
            OracleCommand::Sites => "sites",
        })
        .arg("--repo")
        .arg(root.join("repo"))
        .arg("--ledger")
        .arg(&ledger);
    if matches!(case.command, OracleCommand::Coverage) {
        command.arg("--rules").arg(root.join(&case.rules));
    }
    let output = output_with_input(command.args(&case.flags), &[])?;
    let diagnostic = String::from_utf8(output.stderr)?;
    match &case.result {
        ExpectedCoverage::Reply { body } => {
            assert!(
                output.status.success(),
                "{}: the command did not return an answer: {diagnostic}",
                case.name
            );
            assert!(diagnostic.is_empty(), "{}: {diagnostic}", case.name);
            let reply: Value = serde_json::from_slice(&output.stdout)?;
            assert_eq!(&reply, body, "{}", case.name);
        },
        ExpectedCoverage::Refused {
            diagnostic: expected,
        } => {
            assert!(!output.status.success(), "{} gave a reply", case.name);
            assert!(output.stdout.is_empty(), "{}", case.name);
            assert!(diagnostic.contains(expected), "{}: {diagnostic}", case.name);
        },
    }
    assert_eq!(
        fs::read(&ledger)?,
        before,
        "{} changed the ledger",
        case.name
    );
    assert_eq!(
        read_wal(&ledger)?,
        wal_before,
        "{} changed the WAL",
        case.name
    );
    Ok(())
}
