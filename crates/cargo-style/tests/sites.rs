//! Compare the sites command with the Python ledger and ripgrep.

use std::path::Path;
use std::process::Output;

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

const ORACLE_SOURCE: &[u8] = include_bytes!("sites/oracle.py");
const RIPGREP_CONFIG: &str = "RIPGREP_CONFIG_PATH";

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RegexSettings {
    Default,
    Configured { config: String, ignore: String },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ExpectedSites {
    Reply { value: Value },
    Failure { fragments: Vec<String> },
}

#[derive(Deserialize)]
struct SitesCase {
    name:     String,
    rule:     String,
    ledger:   String,
    checkout: String,
    settings: RegexSettings,
    expected: ExpectedSites,
}

fn oracle_cases(checkout: &Path, root: &Path) -> TestResult<Vec<SitesCase>> {
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
    let output = output_with_input(
        FixtureCommand::new(checkout.join("scripts/lib/py"))
            .current_dir(checkout)
            .env("HOME", home)
            .env("XDG_CACHE_HOME", root.join("python-cache"))
            .env("CONF_DIR", root.join("config"))
            .env("STYLE_TEST_READER_SOURCE", "python")
            .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("style"))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-")
            .arg(root),
        ORACLE_SOURCE,
    )?;
    assert!(
        output.status.success(),
        "Python sites oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

impl SitesCase {
    fn run(&self, root: &Path) -> TestResult<Output> {
        let ledger = root.join(&self.ledger);
        let before = std::fs::read(&ledger)?;
        let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
        command
            .env("HOME", root.join("home"))
            .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("style"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["sites", "--repo"])
            .arg(root.join("repo"))
            .arg("--ledger")
            .arg(&ledger);
        if self.checkout != "repo" {
            command.arg("--checkout").arg(root.join(&self.checkout));
        }
        command.arg(&self.rule);
        match &self.settings {
            RegexSettings::Default => {
                command.env_remove(RIPGREP_CONFIG);
            },
            RegexSettings::Configured { config, ignore } => {
                std::fs::write(root.join("rg.conf"), config)?;
                std::fs::write(root.join("rg.ignore"), ignore)?;
                command.env(RIPGREP_CONFIG, root.join("rg.conf"));
            },
        }
        let output = output_with_input(&mut command, b"")?;
        assert_eq!(
            std::fs::read(ledger)?,
            before,
            "{} changed the ledger",
            self.name
        );
        Ok(output)
    }

    fn compare(&self, output: &Output) -> TestResult<()> {
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        match &self.expected {
            ExpectedSites::Reply { value } => {
                assert!(
                    output.status.success(),
                    "{}: cargo style sites: {diagnostic}",
                    self.name
                );
                assert_eq!(output.stderr.as_slice(), b"", "{}", self.name);
                let reply: Value = serde_json::from_slice(&output.stdout)?;
                assert_eq!(&reply, value, "{}", self.name);
            },
            ExpectedSites::Failure { fragments } => {
                assert!(!output.status.success(), "{}", self.name);
                assert_eq!(output.stdout.as_slice(), b"", "{}", self.name);
                assert_eq!(diagnostic.lines().count(), 1, "{}: {diagnostic}", self.name);
                for fragment in fragments {
                    assert!(diagnostic.contains(fragment), "{}: {diagnostic}", self.name);
                }
            },
        }
        Ok(())
    }
}

#[test]
fn regex_sites_match_the_python_ledger() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let directory = fixture_tempdir("cargo-style-sites-");
    let root = directory.path();
    for case in oracle_cases(&checkout, root)? {
        case.compare(&case.run(root)?)?;
    }
    Ok(())
}
