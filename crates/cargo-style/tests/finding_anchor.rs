//! Compare finding anchors with the Python ledger on a private Git repository.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use rusqlite::Connection;
use rusqlite::params;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

#[path = "../src/finding/constants.rs"]
mod constants;
mod support;

use constants::FINDING_LEDGER_FLAG;
use constants::FINDING_REPO_FLAG;
use constants::FINDING_SUBCOMMAND;
use support::ParityCheckout;
use support::TestResult;
use support::git;
use support::output_with_input;
use support::parity_checkout;

const ORACLE_SOURCE: &[u8] = include_bytes!("oracle/finding_anchor.py");
const LEDGER_FILENAME: &str = "ledger #1.sqlite";

#[derive(Deserialize)]
struct AnchorCase {
    case:   String,
    answer: Value,
}

#[derive(Deserialize)]
struct OracleAnswers {
    synced_tip: String,
    revision:   u64,
    invalid_id: String,
    cases:      Vec<AnchorCase>,
}

struct AnchorFixture {
    directory: TempDir,
    oracle:    OracleAnswers,
}

impl AnchorFixture {
    fn new(checkout: &Path) -> TestResult<Self> {
        let directory = fixture_tempdir("cargo-style-finding-");
        let root = directory.path();
        let repository = root.join("repo");
        std::fs::create_dir(&repository)?;
        git(
            &repository,
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
                .env("HOME", &home)
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
            "Python anchor oracle: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let oracle = serde_json::from_slice(&output.stdout)?;
        Ok(Self { directory, oracle })
    }

    fn ledger(&self, filename: &str) -> PathBuf { self.directory.path().join(filename) }

    fn finding_command(&self, ledger: &Path, ids: &[&str]) -> FixtureCommand {
        let root = self.directory.path();
        let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
        command
            .env("HOME", root.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg(FINDING_SUBCOMMAND)
            .arg(FINDING_REPO_FLAG)
            .arg(root.join("repo"))
            .arg(FINDING_LEDGER_FLAG)
            .arg(ledger)
            .args(ids);
        command
    }

    fn run(&self, mut command: FixtureCommand, input: &[u8]) -> TestResult<Output> {
        Ok(support::process::start(
            command
                .env("HOME", self.ledger("home"))
                .env_remove(support::STATE_DIRECTORY),
        )?
        .output_with_input(input)?)
    }

    fn populate_cache(&self, repository: &Path, blob: &str) -> TestResult<()> {
        let input = serde_json::to_vec(
            &json!({"repository":repository, "commits":[], "blobs":[blob], "facts":["lines"]}),
        )?;
        let output = self.run(
            FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style")),
            &input,
        )?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    }

    fn assert_reply(&self, filename: &str, ledger_revision: &Value) -> TestResult<()> {
        let ledger = self.ledger(filename);
        let before = std::fs::read(&ledger)?;
        let ids = self
            .oracle
            .cases
            .iter()
            .map(|case| {
                case.answer["id"]
                    .as_str()
                    .ok_or("Python did not give a finding id")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let output = output_with_input(&mut self.finding_command(&ledger, &ids), &[])?;
        assert!(
            output.status.success(),
            "cargo style finding: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stderr.as_slice(), b"");
        let reply: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(reply["schema"], 1);
        assert_eq!(
            reply["observed"],
            json!({"synced_tip":self.oracle.synced_tip, "ledger_revision":ledger_revision})
        );
        let findings = reply["findings"]
            .as_array()
            .ok_or("the reply has no findings array")?;
        assert_eq!(findings.len(), ids.len());
        for (actual, expected) in findings.iter().zip(&self.oracle.cases) {
            assert_eq!(actual, &expected.answer, "{}", expected.case);
        }
        assert_eq!(std::fs::read(&ledger)?, before, "the ledger bytes changed");
        Ok(())
    }

    fn case(&self, name: &str) -> TestResult<&AnchorCase> {
        self.oracle
            .cases
            .iter()
            .find(|case| case.case == name)
            .ok_or_else(|| format!("Python did not give the case {name}").into())
    }

    fn failure(&self, filename: &str) -> TestResult<String> {
        let ledger = self.ledger(filename);
        let mut command = self.finding_command(&ledger, &[&self.oracle.invalid_id]);
        let output = output_with_input(&mut command, &[])?;
        assert!(
            !output.status.success(),
            "{filename} unexpectedly succeeded"
        );
        assert!(output.stdout.is_empty(), "{filename} gave a reply");
        let diagnostic = String::from_utf8(output.stderr)?;
        assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
        Ok(diagnostic)
    }
}

impl AnchorCase {
    fn id(&self) -> TestResult<&str> {
        self.answer["id"]
            .as_str()
            .ok_or_else(|| "Python did not give a finding id".into())
    }

    fn assert_answer(&self, output: &Output) -> TestResult<Value> {
        assert!(
            output.status.success(),
            "{}: {}",
            self.case,
            String::from_utf8_lossy(&output.stderr)
        );
        let reply: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(reply["findings"], json!([self.answer]), "{}", self.case);
        Ok(reply)
    }
}

#[test]
fn finding_rejects_a_path_outside_the_repository() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let fixture = AnchorFixture::new(&checkout)?;
    let diagnostic = fixture.failure("invalid-path.sqlite")?;
    assert!(
        diagnostic.contains("invalid row in held_findings")
            && diagnostic.contains(&fixture.oracle.invalid_id)
            && diagnostic.contains("path:"),
        "{diagnostic}"
    );
    Ok(())
}

fn directory_bytes(directory: &Path) -> TestResult<BTreeMap<OsString, Vec<u8>>> {
    std::fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            Ok((entry.file_name(), std::fs::read(entry.path())?))
        })
        .collect()
}

#[test]
fn finding_leaves_both_wal_ledger_directories_unchanged() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let fixture = AnchorFixture::new(&checkout)?;
    assert_wal_ledger_directories(&fixture)?;
    assert_cached_finding(&fixture)
}

fn assert_wal_ledger_directories(fixture: &AnchorFixture) -> TestResult<()> {
    let orphan = fixture.ledger("orphan");
    let case = fixture.case("recorded")?;
    let before = directory_bytes(&orphan)?;
    assert_eq!(
        before.len(),
        2,
        "the fixture must start with a database and WAL"
    );
    assert!(before.contains_key(&OsString::from("wal.sqlite-wal")));
    let command = fixture.finding_command(&orphan.join("wal.sqlite"), &[case.id()?]);
    let reply = case.assert_answer(&fixture.run(command, &[])?)?;
    assert_eq!(
        reply["observed"]["ledger_revision"],
        json!({"revision":fixture.oracle.revision})
    );
    assert_directory_bytes(&orphan, &before)?;

    let closed = fixture.ledger("wal");
    let before = directory_bytes(&closed)?;
    assert_eq!(before.len(), 1, "the fixture must start without sidecars");
    let command = fixture.finding_command(&closed.join("wal.sqlite"), &[case.id()?]);
    let _ = case.assert_answer(&fixture.run(command, &[])?)?;
    assert_directory_bytes(&closed, &before)
}

fn assert_directory_bytes(
    directory: &Path,
    before: &BTreeMap<OsString, Vec<u8>>,
) -> TestResult<()> {
    let after = directory_bytes(directory)?;
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>()
    );
    assert!(&after == before, "the ledger directory bytes changed");
    Ok(())
}

fn assert_cached_finding(fixture: &AnchorFixture) -> TestResult<()> {
    let path = env::var_os("PATH").ok_or("PATH is absent")?;
    let bin = fixture.ledger("bin");
    std::fs::create_dir(&bin)?;
    let git_script = bin.join("git");
    std::fs::write(
        &git_script,
        b"#!/bin/sh\nif [ \"$3\" = cat-file ]; then touch \"$ANCHOR_CAT_FILE\"; exit 90; fi\nexport PATH=\"$ANCHOR_GIT_PATH\"\nexec git \"$@\"\n",
    )?;
    std::fs::set_permissions(&git_script, std::fs::Permissions::from_mode(0o700))?;
    let marker = bin.join("cat-file-called");
    let search_path = env::join_paths(std::iter::once(bin).chain(env::split_paths(&path)))?;
    let case = fixture.case("base only")?;
    let state = fixture.ledger("home/.local/state/style/repo");
    std::fs::create_dir_all(&state)?;
    let ledger = state.join("ledger.sqlite");
    let _ = std::fs::copy(fixture.ledger("wal/wal.sqlite"), &ledger)?;
    let read = || {
        let mut command = fixture.finding_command(&ledger, &[case.id()?]);
        command
            .env("PATH", &search_path)
            .env("ANCHOR_GIT_PATH", &path)
            .env("ANCHOR_CAT_FILE", &marker);
        fixture.run(command, &[])
    };
    let cold = read()?;
    let _ = case.assert_answer(&cold)?;

    let repository = fixture.ledger("repo");
    let blob = git(&repository, &["rev-parse", "HEAD^:src/base.rs"])?;
    fixture.populate_cache(&repository, &blob)?;
    let cache = state.join("selection.sqlite");
    let before = directory_bytes(&state)?;
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        ["ledger.sqlite", "selection.sqlite"]
    );
    let warm = read()?;
    let _ = case.assert_answer(&warm)?;
    assert_eq!(warm.stdout, cold.stdout);
    assert_directory_bytes(&state, &before)?;

    assert_cached_text_is_used(&cache, &blob, read)?;
    assert!(!marker.exists(), "finding invoked git cat-file");
    Ok(())
}

fn assert_cached_text_is_used(
    cache: &Path,
    blob: &str,
    read: impl FnOnce() -> TestResult<Output>,
) -> TestResult<()> {
    // Change only the cached text to prove that the warm reader uses it.
    {
        let database = Connection::open(cache)?;
        let mut reading: Value = serde_json::from_str(&database.query_row(
            "SELECT reading FROM blob_facts WHERE blob_id = ?1",
            [blob],
            |row| row.get::<_, String>(0),
        )?)?;
        assert!(reading["read"]["bundle"]["text"].is_string());
        reading["read"]["bundle"]["text"] = json!("text absent from both commits");
        assert_eq!(
            database.execute(
                "UPDATE blob_facts SET reading = ?1 WHERE blob_id = ?2",
                params![serde_json::to_string(&reading)?, blob],
            )?,
            1
        );
    }
    let altered = read()?;
    assert!(altered.status.success());
    let reply: Value = serde_json::from_slice(&altered.stdout)?;
    assert_eq!(
        reply["findings"][0]["reason"],
        "no source anchor matches the stored coordinate"
    );
    Ok(())
}

#[test]
fn finding_replies_match_the_python_anchors_in_request_order() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let fixture = AnchorFixture::new(&checkout)?;
    fixture.assert_reply(LEDGER_FILENAME, &json!("absent"))?;
    fixture.assert_reply(
        "revision.sqlite",
        &json!({"revision":fixture.oracle.revision}),
    )
}

#[test]
fn ledger_failures_report_one_diagnostic_and_no_reply() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let fixture = AnchorFixture::new(&checkout)?;
    let missing = fixture.failure("missing.sqlite")?;
    assert!(
        missing.contains("missing ledger at") && missing.contains("missing.sqlite"),
        "{missing}"
    );
    assert!(!fixture.ledger("missing.sqlite").exists());

    let unsupported = fixture.failure("unsupported.sqlite")?;
    assert!(
        unsupported.contains("unsupported ledger schema"),
        "{unsupported}"
    );

    let invalid = fixture.failure("invalid.sqlite")?;
    assert!(
        invalid.contains("invalid row in held_findings")
            && invalid.contains(&fixture.oracle.invalid_id),
        "{invalid}"
    );
    Ok(())
}
