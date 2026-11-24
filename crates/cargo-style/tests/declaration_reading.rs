//! Compare declaration readings, site evidence and approvals with the pinned Python ledger.

mod support;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use serde::Deserialize;
use serde_json::Value;
use support::ParityCheckout;
use support::TestResult;
use support::git;
use support::output_with_input;
use support::parity_checkout;

const ORACLE: &[u8] = include_bytes!("oracle/declaration.py");
const RELOCATION_ORACLE: &[u8] = include_bytes!("oracle/same_declaration.py");
const OUTSIDE_ORACLE: &[u8] = include_bytes!("oracle/outside_rename.py");
const READING_FIELDS: [&str; 3] = ["declaration", "site_evidence", "approvals"];
/// The oracle's fixtures, in the order a sorted map holds them.
const FIXTURES: [&str; 3] = ["outside", "relocation", "renames"];

#[derive(Deserialize)]
struct ReadingCase {
    name:   String,
    id:     String,
    answer: Value,
}

#[test]
fn declaration_reading_matches_python() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let temporary = fixture_tempdir("cargo-style-declaration-reading-");
    let root = temporary.path();
    let oracle = oracle_replies(&checkout, root)?;
    assert_eq!(
        oracle.keys().map(String::as_str).collect::<Vec<_>>(),
        FIXTURES
    );
    let mut mismatches = Vec::new();
    for (fixture, cases) in &oracle {
        let fixture_root = root.join(fixture);
        let ledger = fixture_root.join("ledger.sqlite");
        let saved = std::fs::read(&ledger)?;
        let repository = repository_state(&fixture_root)?;
        mismatches.extend(reading_mismatches(&fixture_root, cases)?);
        assert_eq!(
            std::fs::read(ledger)?,
            saved,
            "{fixture}: the ledger bytes changed"
        );
        assert_eq!(
            repository_state(&fixture_root)?,
            repository,
            "{fixture}: the repository changed"
        );
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    approval_questions_without_file(&root.join("renames"), &oracle["renames"])
}

/// A request without an approvals file answers each approval question as unobserved.
fn approval_questions_without_file(root: &Path, cases: &[ReadingCase]) -> TestResult<()> {
    let case = cases
        .iter()
        .find(|case| case.name == "approved")
        .ok_or("missing the approved rename case")?;
    let output = finding_with(root, &[case.id.as_str()], Approvals::NoFile)?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reply: Value = serde_json::from_slice(&output.stdout)?;
    let answer = &reply["findings"][0];
    assert_eq!(
        answer["declaration"],
        serde_json::json!({"status": "approval_unobserved", "path": "decisions/approved.md"})
    );
    assert_eq!(answer["approvals"], serde_json::json!([]));
    Ok(())
}

#[test]
fn approvals_reject_a_repeated_decision() -> TestResult<()> {
    for (decisions, repeated) in [
        (r#"{"a.md": "approved", "a.md": "declined"}"#, "a.md"),
        (r#"{"a.md": "approved", "./a.md": "approved"}"#, "a.md"),
    ] {
        let temporary = fixture_tempdir("cargo-style-approvals-repeated-");
        let root = temporary.path();
        std::fs::write(
            root.join("approvals.json"),
            format!(r#"{{"decisions": {decisions}}}"#),
        )?;
        let output = finding(root, &["0000000000000001"])?;
        assert!(!output.status.success(), "{decisions}: the request passed");
        assert_eq!(output.stdout.as_slice(), b"", "{decisions}");
        let diagnostic = String::from_utf8(output.stderr)?;
        assert!(
            diagnostic.contains(&format!("name decision {repeated} twice")),
            "{decisions}: {diagnostic}"
        );
    }
    Ok(())
}

#[test]
fn approvals_need_rules() -> TestResult<()> {
    let temporary = fixture_tempdir("cargo-style-approvals-usage-");
    let root = temporary.path();
    let output = output_with_input(
        FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"))
            .arg("finding")
            .arg("--repo")
            .arg(root)
            .arg("--ledger")
            .arg(root.join("ledger.sqlite"))
            .arg("--approvals")
            .arg(root.join("approvals.json"))
            .arg("0000000000000001"),
        &[],
    )?;
    assert!(
        !output.status.success(),
        "the request accepted --approvals without --rules"
    );
    assert_eq!(output.stdout.as_slice(), b"");
    let diagnostic = String::from_utf8(output.stderr)?;
    assert!(
        diagnostic.contains("[--rules <rules.json> [--approvals <approvals.json>]]"),
        "{diagnostic}"
    );
    Ok(())
}

fn oracle_replies(checkout: &Path, root: &Path) -> TestResult<BTreeMap<String, Vec<ReadingCase>>> {
    let home = root.join("home");
    std::fs::create_dir(&home)?;
    std::fs::write(root.join("same_declaration.py"), RELOCATION_ORACLE)?;
    std::fs::write(root.join("outside_rename.py"), OUTSIDE_ORACLE)?;
    let output = output_with_input(
        FixtureCommand::new(checkout.join("scripts/lib/py"))
            .current_dir(checkout)
            .env("HOME", home)
            .env("XDG_CACHE_HOME", root.join("python-cache"))
            .env("CONF_DIR", root.join("config"))
            .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("global-rules"))
            .env("STYLE_TEST_READER_SOURCE", "python")
            .env("PYTHONPATH", root)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-")
            .arg(root),
        ORACLE,
    )?;
    assert!(
        output.status.success(),
        "The Python declaration oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// The head commit and the working tree status of a fixture repository.
fn repository_state(root: &Path) -> TestResult<[String; 2]> {
    let repository = root.join("repo");
    Ok([
        git(&repository, &["rev-parse", "HEAD"])?,
        git(&repository, &["status", "--porcelain", "--ignored"])?,
    ])
}

/// Whether a finding request names the fixture's approvals file.
#[derive(Clone, Copy)]
enum Approvals {
    File,
    NoFile,
}

fn finding(root: &Path, ids: &[&str]) -> TestResult<Output> {
    finding_with(root, ids, Approvals::File)
}

fn finding_with(root: &Path, ids: &[&str], approvals: Approvals) -> TestResult<Output> {
    let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
    command
        .env("HOME", root.join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("finding")
        .arg("--repo")
        .arg(root.join("repo"))
        .arg("--ledger")
        .arg(root.join("ledger.sqlite"))
        .arg("--rules")
        .arg(root.join("rules.json"));
    if matches!(approvals, Approvals::File) {
        command.arg("--approvals").arg(root.join("approvals.json"));
    }
    output_with_input(command.args(ids), &[])
}

fn reading_mismatches(root: &Path, cases: &[ReadingCase]) -> TestResult<Vec<String>> {
    let ids: Vec<_> = cases.iter().map(|case| case.id.as_str()).collect();
    let output = finding(root, &ids)?;
    assert!(
        output.status.success(),
        "The finding request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr.as_slice(), b"");
    let reply: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(reply["schema"], 7);
    let findings = reply["findings"]
        .as_array()
        .ok_or("missing finding replies")?;
    assert_eq!(findings.len(), cases.len());
    let mut mismatches = Vec::new();
    for (actual, expected) in findings.iter().zip(cases) {
        assert_eq!(actual["id"], expected.id.as_str(), "{}", expected.name);
        let reading: serde_json::Map<_, _> = READING_FIELDS
            .iter()
            .map(|field| ((*field).to_owned(), actual[*field].clone()))
            .collect();
        let reading = Value::Object(reading);
        if reading != expected.answer {
            mismatches.push(format!(
                "Finding {}\n  Rust: {reading}\nPython: {}",
                expected.name, expected.answer
            ));
        }
    }
    Ok(mismatches)
}
