//! Compare the original-site reply with strict proof from the Python ledger.

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

const ORACLE_SOURCE: &[u8] = include_bytes!("oracle/original_site.py");
const BUILD_CASE: &str = "unchanged crate inputs with build history";

#[derive(Deserialize)]
struct SiteCase {
    name:   String,
    answer: Value,
}

#[derive(Deserialize)]
struct FailedSiteRequest {
    name:       String,
    ledger:     String,
    finding:    String,
    diagnostic: String,
}

#[derive(Deserialize)]
struct FindingAtAlternateTip {
    ledger:     String,
    synced_tip: String,
    site:       SiteCase,
}

#[derive(Deserialize)]
struct OracleAnswers {
    synced_tip:          String,
    missing_pin:         String,
    missing_rule:        String,
    missing_pin_finding: String,
    cases:               Vec<SiteCase>,
    failures:            Vec<FailedSiteRequest>,
    non_utf8_ignore:     FindingAtAlternateTip,
}

#[test]
fn original_site_matches_python_in_request_order() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let directory = fixture_tempdir("cargo-style-original-site-");
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
    let oracle = original_site_answers(&checkout, root)?;
    let ledger = root.join("ledger.sqlite");
    let before = std::fs::read(&ledger)?;

    assert_site_reply(root, &oracle)?;
    assert_site_reply_with_policy(root, &oracle)?;
    assert_finding_at_alternate_tip(root, &oracle.non_utf8_ignore)?;
    assert_missing_pin_error(root, &oracle)?;
    for failure in &oracle.failures {
        assert_request_error(root, failure)?;
    }
    assert_eq!(std::fs::read(ledger)?, before, "the ledger bytes changed");
    Ok(())
}

fn original_site_answers(checkout: &Path, root: &Path) -> TestResult<OracleAnswers> {
    let home = root.join("home");
    std::fs::create_dir(&home)?;
    let output = output_with_input(
        FixtureCommand::new(checkout.join("scripts/lib/py"))
            .current_dir(checkout)
            .env("HOME", home)
            .env("XDG_CACHE_HOME", root.join("python-cache"))
            .env("CONF_DIR", root.join("config"))
            .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("global-rules"))
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
        "Python site oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn finding(root: &Path, rules: &str, ids: &[&str]) -> TestResult<Output> {
    finding_in_ledger(root, "ledger.sqlite", rules, ids)
}

fn finding_in_ledger(root: &Path, ledger: &str, rules: &str, ids: &[&str]) -> TestResult<Output> {
    output_with_input(finding_command(root, ledger, rules).args(ids), &[])
}

fn finding_command(root: &Path, ledger: &str, rules: &str) -> FixtureCommand {
    let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
    command
        .env("HOME", root.join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("finding")
        .arg("--repo")
        .arg(root.join("repo"))
        .arg("--ledger")
        .arg(root.join(ledger))
        .arg("--rules")
        .arg(root.join(rules));
    command
}

fn assert_site_reply(root: &Path, oracle: &OracleAnswers) -> TestResult<()> {
    let ids = oracle
        .cases
        .iter()
        .map(|case| {
            case.answer["id"]
                .as_str()
                .ok_or("missing oracle finding id")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut reply = site_reply(&finding(root, "rules.json", &ids)?, oracle)?;
    let findings = reply["findings"]
        .as_array_mut()
        .ok_or("missing finding replies")?;
    assert_eq!(findings.len(), ids.len());
    for (actual, expected) in findings.iter_mut().zip(&oracle.cases) {
        if matches!(
            expected.name.as_str(),
            "unchanged crate inputs" | BUILD_CASE
        ) {
            let refusal = json!({
                "status": "unproved",
                "reason": "rename admission requires --rename-policy <rename_policy.toml>",
                "line": 0,
                "text": ""
            });
            assert_eq!(actual["rename_admission"], refusal, "{}", expected.name);
            if expected.name == BUILD_CASE {
                assert_eq!(actual["rename_applied"], refusal, "{}", expected.name);
            }
        }
        assert_site_fields(actual, expected)?;
    }
    Ok(())
}

fn assert_site_reply_with_policy(root: &Path, oracle: &OracleAnswers) -> TestResult<()> {
    let expected = oracle
        .cases
        .iter()
        .find(|case| case.name == BUILD_CASE)
        .ok_or("missing build case")?;
    let id = expected.answer["id"]
        .as_str()
        .ok_or("missing build finding id")?;
    let policy = root.join("rename_policy.toml");
    std::fs::write(&policy, b"inert_attributes = []\nstd_macros = []\n")?;
    let output = output_with_input(
        finding_command(root, "ledger.sqlite", "rules.json")
            .arg("--rename-policy")
            .arg(policy)
            .arg(id),
        &[],
    )?;
    let mut reply = site_reply(&output, oracle)?;
    let actual = &mut reply["findings"][0];
    assert_eq!(actual["rename_admission"]["status"], "admitted");
    assert_eq!(actual["rename_applied"]["status"], "not_applied");
    assert_site_fields(actual, expected)
}

fn assert_finding_at_alternate_tip(
    root: &Path,
    expected: &FindingAtAlternateTip,
) -> TestResult<()> {
    let id = expected.site.answer["id"]
        .as_str()
        .ok_or("missing oracle finding id")?;
    let ledger = root.join(&expected.ledger);
    let before = std::fs::read(&ledger)?;
    let output = finding_in_ledger(root, &expected.ledger, "rules.json", &[id])?;
    let mut reply = site_reply_at(&output, &expected.synced_tip)?;
    assert_site_fields(&mut reply["findings"][0], &expected.site)?;
    assert_eq!(std::fs::read(ledger)?, before, "the ledger bytes changed");
    Ok(())
}

fn site_reply(output: &Output, oracle: &OracleAnswers) -> TestResult<Value> {
    site_reply_at(output, &oracle.synced_tip)
}

fn site_reply_at(output: &Output, synced_tip: &str) -> TestResult<Value> {
    assert!(
        output.status.success(),
        "The finding request did not return original_site: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr.as_slice(), b"");
    let reply: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(reply["schema"], 7);
    assert_eq!(
        reply["observed"],
        json!({"synced_tip":synced_tip, "ledger_revision":"absent"})
    );
    Ok(reply)
}

fn assert_site_fields(actual: &mut Value, expected: &SiteCase) -> TestResult<()> {
    if expected.answer["status"] != "unknown_finding" {
        let fields = actual.as_object_mut().ok_or("expected a finding object")?;
        for field in [
            "same_declaration",
            "outside_rename",
            "rename_admission",
            "rename_applied",
            "footprint",
            "declaration",
            "site_evidence",
            "approvals",
        ] {
            let _ = fields
                .remove(field)
                .ok_or_else(|| format!("{}: missing {field}", expected.name))?;
        }
    }
    assert_eq!(&*actual, &expected.answer, "{}", expected.name);
    Ok(())
}

fn assert_missing_pin_error(root: &Path, oracle: &OracleAnswers) -> TestResult<()> {
    let output = finding(
        root,
        "missing-rule-blob.json",
        &[&oracle.missing_pin_finding],
    )?;
    assert!(!output.status.success(), "the missing pin gave a reply");
    assert_eq!(output.stdout.as_slice(), b"");
    let diagnostic = String::from_utf8(output.stderr)?;
    assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
    assert!(diagnostic.contains(&oracle.missing_rule), "{diagnostic}");
    assert!(diagnostic.contains(&oracle.missing_pin), "{diagnostic}");
    Ok(())
}

fn assert_request_error(root: &Path, failure: &FailedSiteRequest) -> TestResult<()> {
    let ledger = root.join(&failure.ledger);
    let before = std::fs::read(&ledger)?;
    let output = finding_in_ledger(root, &failure.ledger, "rules.json", &[&failure.finding])?;
    assert!(!output.status.success(), "{} gave a reply", failure.name);
    assert_eq!(output.stdout.as_slice(), b"", "{}", failure.name);
    let diagnostic = String::from_utf8(output.stderr)?;
    assert_eq!(
        diagnostic.lines().count(),
        1,
        "{}: {diagnostic}",
        failure.name
    );
    assert!(
        diagnostic.contains(&failure.diagnostic),
        "{}: {diagnostic}",
        failure.name
    );
    assert_eq!(
        std::fs::read(ledger)?,
        before,
        "{} changed the ledger",
        failure.name
    );
    Ok(())
}
