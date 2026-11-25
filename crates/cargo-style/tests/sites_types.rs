//! Compare type and trait sites with the pinned Python ledger.

use std::path::Path;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use serde::Deserialize;
use serde_json::Value;

mod support;

use support::ParityCheckout;
use support::TestResult;
use support::git;
use support::output_with_input;
use support::parity_checkout;

const ORACLE_SOURCE: &[u8] = include_bytes!("sites/types_oracle.py");

#[derive(Deserialize)]
struct TypeSitesCase {
    name:     String,
    rule:     String,
    ledger:   String,
    expected: Value,
}

fn oracle_cases(checkout: &Path, root: &Path) -> TestResult<Vec<TypeSitesCase>> {
    let repo = root.join("repo");
    std::fs::create_dir(&repo)?;
    git(
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
    let output = output_with_input(
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
        ORACLE_SOURCE,
    )?;
    assert!(
        output.status.success(),
        "Python type sites oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

impl TypeSitesCase {
    fn native_reply(&self, root: &Path) -> TestResult<Value> {
        let ledger = root.join(&self.ledger);
        let before = std::fs::read(&ledger)?;
        let output = output_with_input(
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
            std::fs::read(ledger)?,
            before,
            "{} changed the ledger",
            self.name
        );
        assert!(
            output.status.success(),
            "{}: cargo style sites: {}",
            self.name,
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stderr.as_slice(), b"", "{}", self.name);
        Ok(serde_json::from_slice(&output.stdout)?)
    }
}

#[test]
fn type_and_trait_sites_match_the_python_ledger() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let directory = fixture_tempdir("cargo-style-type-sites-");
    let root = directory.path();
    let mut mismatches = Vec::new();
    for case in oracle_cases(&checkout, root)? {
        let actual = case.native_reply(root)?;
        if actual != case.expected {
            mismatches.push((case.name, case.expected, actual));
        }
    }
    assert!(
        mismatches.is_empty(),
        "Python sites differences: {mismatches:#?}"
    );
    Ok(())
}
