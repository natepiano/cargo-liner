//! Run a sites oracle and compare its results with cargo-style.

#[path = "../support/wal.rs"]
mod wal;

use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use serde::Deserialize;
use serde_json::Value;
use wal::read_wal;

use crate::support;
use crate::support::ParityCheckout;
use crate::support::TestResult;

#[derive(Deserialize)]
struct SitesParityCase {
    name:     String,
    rule:     String,
    ledger:   String,
    expected: ExpectedSites,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExpectedSites {
    Refusal { refusal: Vec<String> },
    Reply(Value),
}

impl SitesParityCase {
    fn run(&self, root: &Path) -> TestResult<Output> {
        let ledger = root.join(&self.ledger);
        let before = std::fs::read(&ledger)?;
        let wal_before = read_wal(&ledger)?;
        let output = support::output_with_input(
            FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"))
                .env("HOME", root.join("home"))
                .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("style"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .args(["sites", "--repo"])
                .arg(root.join("repo"))
                .arg("--ledger")
                .arg(&ledger)
                .arg(&self.rule),
            b"",
        )?;
        assert_eq!(
            std::fs::read(&ledger)?,
            before,
            "{} changed the ledger",
            self.name
        );
        assert_eq!(
            read_wal(&ledger)?,
            wal_before,
            "{} changed the WAL",
            self.name
        );
        Ok(output)
    }
}

fn oracle_cases(checkout: &Path, root: &Path, oracle: &[u8]) -> TestResult<Vec<SitesParityCase>> {
    let repo = root.join("repo");
    std::fs::create_dir(&repo)?;
    let _ = support::git(
        &repo,
        &[
            "init",
            "-q",
            "--initial-branch=main",
            "--object-format=sha1",
        ],
    )?;
    let home = root.join("home");
    std::fs::create_dir(&home)?;
    let helpers = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sites");
    let output = support::output_with_input(
        FixtureCommand::new(checkout.join("scripts/lib/py"))
            .current_dir(checkout)
            .env("HOME", home)
            .env("XDG_CACHE_HOME", root.join("python-cache"))
            .env("CONF_DIR", root.join("config"))
            .env("STYLE_TEST_READER_SOURCE", "python")
            .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("style"))
            .env("PYTHONPATH", helpers)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-")
            .arg(root),
        oracle,
    )?;
    assert!(
        output.status.success(),
        "Python sites oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub(super) fn run_sites_parity(oracle: &[u8], directory_prefix: &str) -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = support::parity_checkout() else {
        return Ok(());
    };
    let directory = fixture_tempdir(directory_prefix);
    let root = directory.path();
    let mut mismatches = Vec::new();
    for case in oracle_cases(&checkout, root, oracle)? {
        let output = case.run(root)?;
        let name = case.name;
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        match case.expected {
            ExpectedSites::Reply(expected) => {
                assert!(
                    output.status.success(),
                    "{name}: cargo style sites: {diagnostic}"
                );
                assert_eq!(output.stderr.as_slice(), b"", "{name}");
                let actual: Value = serde_json::from_slice(&output.stdout)?;
                if actual != expected {
                    mismatches.push((name, expected, actual));
                }
            },
            ExpectedSites::Refusal { refusal } => {
                assert!(!output.status.success(), "{name}");
                assert_eq!(output.stdout.as_slice(), b"", "{name}");
                assert_eq!(diagnostic.lines().count(), 1, "{name}: {diagnostic}");
                for fragment in refusal {
                    assert!(diagnostic.contains(&fragment), "{name}: {diagnostic}");
                }
            },
        }
    }
    assert!(
        mismatches.is_empty(),
        "Python sites differences: {mismatches:#?}"
    );
    Ok(())
}
