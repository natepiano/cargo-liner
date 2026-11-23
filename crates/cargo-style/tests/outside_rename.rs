//! Compare outside rename replies with the pinned Python ledger.

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

const ORACLE: &[u8] = include_bytes!("oracle/outside_rename.py");
const RELOCATION_ORACLE: &[u8] = include_bytes!("oracle/same_declaration.py");
const REPAIR_CASES: &[&str] = &[
    "nested_trait_crate",
    "raw_cfg_argument",
    "field_python_whitespace",
];

#[derive(Deserialize)]
struct OutsideCase {
    name:   String,
    answer: Value,
}

#[derive(Deserialize)]
struct MissingCandidateSource {
    id:   String,
    blob: String,
    path: String,
    read: String,
}

#[derive(Deserialize)]
struct OracleReplies {
    synced_tip:     String,
    cases:          Vec<OutsideCase>,
    missing_source: MissingCandidateSource,
}

#[test]
fn outside_rename_matches_python_in_request_order() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let temporary = fixture_tempdir("cargo-style-outside-rename-");
    let root = temporary.path();
    let repo = root.join("repo");
    std::fs::create_dir(&repo)?;
    git(
        &repo,
        &[
            "init",
            "-q",
            "--object-format=sha1",
            "--initial-branch=main",
        ],
    )?;

    let oracle = oracle_replies(&checkout, root)?;
    for name in REPAIR_CASES {
        assert!(
            oracle.cases.iter().any(|case| case.name == *name),
            "Missing finding {name}"
        );
    }
    let ledger = root.join("ledger.sqlite");
    let saved = std::fs::read(&ledger)?;
    let ids = assert_outside_reply(root, &oracle)?;
    let first_id = ids.first().ok_or("missing successful finding")?;
    assert_missing_candidate_source(root, first_id, &oracle)?;
    assert_eq!(std::fs::read(ledger)?, saved, "The ledger bytes changed");
    Ok(())
}

fn oracle_replies(checkout: &Path, root: &Path) -> TestResult<OracleReplies> {
    let home = root.join("home");
    std::fs::create_dir(&home)?;
    std::fs::write(root.join("same_declaration.py"), RELOCATION_ORACLE)?;
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
        "The Python outside rename oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn finding(root: &Path, ids: &[&str]) -> TestResult<Output> {
    output_with_input(
        FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"))
            .env("HOME", root.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("finding")
            .arg("--repo")
            .arg(root.join("repo"))
            .arg("--ledger")
            .arg(root.join("ledger.sqlite"))
            .arg("--rules")
            .arg(root.join("rules.json"))
            .args(ids),
        &[],
    )
}

fn assert_outside_reply<'a>(root: &Path, oracle: &'a OracleReplies) -> TestResult<Vec<&'a str>> {
    let ids = oracle
        .cases
        .iter()
        .map(|case| {
            case.answer["id"]
                .as_str()
                .ok_or("missing oracle finding id")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let output = finding(root, &ids)?;
    assert!(
        output.status.success(),
        "The finding request failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr.as_slice(), b"");
    let mut reply: Value = serde_json::from_slice(&output.stdout)?;
    let findings = reply["findings"]
        .as_array_mut()
        .ok_or("missing finding replies")?;
    assert_eq!(findings.len(), ids.len());
    let mut mismatches = Vec::new();
    for (actual, expected) in findings.iter_mut().zip(&oracle.cases) {
        if expected.answer["status"] != "unknown_finding" {
            let fields = actual.as_object_mut().ok_or("expected a finding object")?;
            for field in ["rename_admission", "rename_applied", "footprint"] {
                let _ = fields
                    .remove(field)
                    .ok_or_else(|| format!("{}: missing {field}", expected.name))?;
            }
        }
        if *actual != expected.answer {
            mismatches.push(format!(
                "Finding {}\n  Rust: {actual}\nPython: {}",
                expected.name, expected.answer
            ));
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    assert_eq!(reply["schema"], 6);
    assert_eq!(
        reply["observed"],
        json!({"synced_tip": oracle.synced_tip, "ledger_revision": "absent"})
    );
    Ok(ids)
}

fn assert_missing_candidate_source(
    root: &Path,
    first_id: &str,
    oracle: &OracleReplies,
) -> TestResult<()> {
    let missing = &oracle.missing_source;
    let (directory, file) = missing.blob.split_at_checked(2).ok_or("invalid blob id")?;
    std::fs::remove_file(root.join("repo/.git/objects").join(directory).join(file))?;
    let output = finding(root, &[first_id, &missing.id])?;
    assert!(
        !output.status.success(),
        "The request accepted a missing candidate blob"
    );
    assert_eq!(
        output.stdout.as_slice(),
        b"",
        "The failed request wrote a reply"
    );
    let diagnostic = String::from_utf8(output.stderr)?;
    assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
    let context = format!(
        "cannot prove outside rename for {} from {} to {}",
        missing.path, missing.read, oracle.synced_tip
    );
    for expected in [
        context.as_str(),
        "read prior candidate declarations",
        missing.blob.as_str(),
    ] {
        assert!(diagnostic.contains(expected), "{diagnostic}");
    }
    Ok(())
}
