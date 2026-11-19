//! Compare finding footprints with the Python ledger on private Git fixtures.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

mod support;

use support::ParityCheckout;
use support::TestResult;
use support::git;
use support::output_with_input;
use support::parity_checkout;

const ORACLE_SOURCE: &[u8] = include_bytes!("oracle/footprint.py");
const LEDGER: &str = "ledger #1.sqlite";

#[derive(Deserialize)]
struct FootprintRow {
    case:      String,
    id:        String,
    footprint: Value,
}

#[derive(Deserialize)]
struct MissingOwner {
    base:       String,
    finding:    String,
    diagnostic: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum FifoManifest {
    Unsupported,
    Available(Vec<FootprintRow>),
}

#[derive(Deserialize)]
struct OracleFootprints {
    tip:             String,
    base:            String,
    synced:          Vec<FootprintRow>,
    with_base:       Vec<FootprintRow>,
    changed_library: Vec<FootprintRow>,
    no_root:         Vec<FootprintRow>,
    plain:           String,
    broken_base:     String,
    missing_blob:    String,
    missing_finding: String,
    missing_owner:   MissingOwner,
    warning_match:   Vec<FootprintRow>,
    fifo_manifest:   FifoManifest,
}

#[derive(Clone, Copy)]
enum FootprintBase<'a> {
    SyncedOnly,
    WithBase(&'a str),
}

struct FootprintFixture {
    directory: TempDir,
    oracle:    OracleFootprints,
}

impl FootprintFixture {
    fn create(checkout: &Path) -> TestResult<Self> {
        let directory = fixture_tempdir("cargo-style-footprint-");
        let root = directory.path();
        let repository = root.join("repo");
        fs::create_dir(&repository)?;
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
        fs::create_dir(&home)?;
        let output = output_with_input(
            FixtureCommand::new(checkout.join("scripts/lib/py"))
                .current_dir(checkout)
                .env("HOME", home)
                .env("XDG_CACHE_HOME", root.join("python-cache"))
                .env("CONF_DIR", root.join("config"))
                .env("STYLE_HISTORY_NATE_STYLE_DIR", root.join("rules"))
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
            "Python footprint oracle: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let oracle = serde_json::from_slice(&output.stdout)?;
        Ok(Self { directory, oracle })
    }

    fn repository(&self) -> PathBuf { self.directory.path().join("repo") }

    fn finding(&self, base: FootprintBase<'_>, ids: &[&str]) -> TestResult<Output> {
        let root = self.directory.path();
        let ledger = root.join(LEDGER);
        let before = fs::read(&ledger)?;
        let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
        command
            .env("HOME", root.join("home"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("finding")
            .arg("--repo")
            .arg(self.repository())
            .arg("--ledger")
            .arg(&ledger);
        if let FootprintBase::WithBase(revision) = base {
            command.arg("--base").arg(revision);
        }
        let output = output_with_input(command.args(ids), &[])?;
        assert_eq!(fs::read(ledger)?, before, "the ledger bytes changed");
        Ok(output)
    }

    fn assert_rows(&self, base: FootprintBase<'_>, expected: &[FootprintRow]) -> TestResult<()> {
        let ids: Vec<_> = expected.iter().map(|row| row.id.as_str()).collect();
        let output = self.finding(base, &ids)?;
        assert!(
            output.status.success(),
            "cargo style finding: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "the request emitted a diagnostic");
        let reply: Value = serde_json::from_slice(&output.stdout)?;
        let findings = reply["findings"]
            .as_array()
            .ok_or("the reply has no findings array")?;
        assert_eq!(findings.len(), ids.len());
        for (actual, row) in findings.iter().zip(expected) {
            if row.footprint.is_null() {
                assert_eq!(actual, &json!({"id":row.id, "status":"unknown_finding"}));
            } else {
                assert_eq!(actual["id"], row.id, "{}", row.case);
                assert_eq!(
                    actual["footprint"], row.footprint,
                    "footprint for {}",
                    row.case
                );
            }
        }
        assert_eq!(reply["schema"], 2);
        assert_eq!(
            reply["observed"],
            json!({"synced_tip":self.oracle.tip, "ledger_revision":"absent"})
        );
        Ok(())
    }

    fn assert_request_error(&self, revision: &str, ids: &[&str], subject: &str) -> TestResult<()> {
        let output = self.finding(FootprintBase::WithBase(revision), ids)?;
        assert!(!output.status.success(), "the request accepted {revision}");
        assert!(
            output.stdout.is_empty(),
            "the request emitted a partial reply"
        );
        let diagnostic = String::from_utf8(output.stderr)?;
        assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
        assert!(diagnostic.contains(subject), "{diagnostic}");
        Ok(())
    }
}

#[test]
fn finding_footprints_match_python_in_request_order() -> TestResult<()> {
    let ParityCheckout::Enabled(checkout) = parity_checkout() else {
        return Ok(());
    };
    let fixture = FootprintFixture::create(&checkout)?;
    let oracle = &fixture.oracle;
    fixture.assert_rows(FootprintBase::SyncedOnly, &oracle.synced)?;
    fixture.assert_rows(FootprintBase::WithBase(&oracle.base), &oracle.with_base)?;
    fixture.assert_rows(FootprintBase::WithBase(&oracle.tip), &oracle.synced)?;
    fixture.assert_rows(
        FootprintBase::WithBase(&oracle.broken_base),
        &oracle.warning_match,
    )?;

    let repository = fixture.repository();
    let manifest = repository.join("app/Cargo.toml");
    let committed_manifest = fs::read_to_string(&manifest)?;
    fs::write(
        &manifest,
        committed_manifest.replace("committed-lib", "local-lib"),
    )?;
    fixture.assert_rows(FootprintBase::SyncedOnly, &oracle.changed_library)?;
    fs::remove_file(repository.join("app/src/lib.rs"))?;
    fixture.assert_rows(FootprintBase::SyncedOnly, &oracle.no_root)?;

    let unresolved = "missing-footprint-base";
    fixture.assert_request_error(unresolved, &[&oracle.plain], unresolved)?;
    fixture.assert_request_error(
        &oracle.broken_base,
        &[&oracle.plain, &oracle.missing_finding],
        &oracle.missing_blob,
    )?;
    let owner = &oracle.missing_owner;
    fixture.assert_request_error(
        &owner.base,
        &[&oracle.plain, &owner.finding],
        &owner.diagnostic,
    )?;
    assert_fifo_manifest(&fixture)
}

fn assert_fifo_manifest(fixture: &FootprintFixture) -> TestResult<()> {
    let FifoManifest::Available(rows) = &fixture.oracle.fifo_manifest else {
        eprintln!("Skipped the manifest FIFO row because the platform has no mkfifo.");
        return Ok(());
    };
    let repository = fixture.repository();
    git(
        &repository,
        &[
            "checkout",
            "-q",
            &fixture.oracle.tip,
            "--",
            "app/src/lib.rs",
        ],
    )?;
    fs::rename(
        fixture.directory.path().join("manifest.fifo"),
        repository.join("app/Cargo.toml"),
    )?;
    fixture.assert_rows(FootprintBase::SyncedOnly, rows)
}
