//! Check the persistent cache through the installed reader protocol.

#[path = "../src/cache_constants.rs"]
mod cache_constants;
#[path = "support/process.rs"]
mod process;
#[path = "../src/state_directory.rs"]
mod state_directory;

use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::slice;

use cache_constants::FACT_DATABASE_NAME;
use cache_constants::PARSE_COUNT_ENV;
use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
use process::TestResult;
use process::git_input;
use process::start;
use rusqlite::Connection;
use rusqlite::OpenFlags;
use serde_json::Value;
use serde_json::json;
use state_directory::STATE_DIRECTORY;
use tempfile::TempDir;

const REPOSITORY_NAME: &str = "repository";
const ALL_FACTS: [&str; 7] = [
    "tokens",
    "lines",
    "names",
    "module_scan",
    "declarations",
    "name_sites",
    "rust_code",
];
const SAMPLE_SOURCE: &[u8] = b"fn sample() { sample(); }\n";

struct RepositoryFixture {
    directory: TempDir,
}

impl RepositoryFixture {
    fn new() -> TestResult<Self> {
        let directory = fixture_tempdir("cargo-style-cache-");
        let fixture = Self { directory };
        let repository = fixture.repository();
        std::fs::create_dir(&repository)?;
        git_input(&repository, &["init", "-q", "--object-format=sha1"], &[])?;
        git_input(&repository, &["config", "user.name", "Reader Test"], &[])?;
        git_input(
            &repository,
            &["config", "user.email", "reader@example.invalid"],
            &[],
        )?;
        Ok(fixture)
    }

    fn repository(&self) -> PathBuf { self.directory.path().join(REPOSITORY_NAME) }

    fn state(&self) -> PathBuf { self.directory.path().join("state") }

    fn database(&self) -> PathBuf { self.state().join(REPOSITORY_NAME).join(FACT_DATABASE_NAME) }

    fn unavailable_state(&self) -> TestResult<PathBuf> {
        let state = self.directory.path().join("unavailable-state");
        std::fs::create_dir(&state)?;
        // A file at the repository state path prevents cache creation for every user.
        std::fs::write(state.join(REPOSITORY_NAME), [])?;
        Ok(state)
    }

    fn write_blob(&self, bytes: &[u8]) -> TestResult<String> {
        git_input(&self.repository(), &["hash-object", "-w", "--stdin"], bytes)
    }

    fn commit_files(&self, files: &[(&str, &[u8])]) -> TestResult<String> {
        let repository = self.repository();
        for &(name, bytes) in files {
            std::fs::write(repository.join(name), bytes)?;
        }
        git_input(&repository, &["add", "."], &[])?;
        git_input(&repository, &["commit", "-qm", "Add reader fixtures"], &[])?;
        git_input(&repository, &["rev-parse", "HEAD"], &[])
    }

    fn explicit_request(&self, blobs: &[String], facts: &[&str]) -> Value {
        json!({"repository":self.repository(), "commits":[], "blobs":blobs, "facts":facts})
    }

    fn read(&self, request: &Value, parses: usize) -> TestResult<Output> {
        let output = answer(reader(&self.state()).env(PARSE_COUNT_ENV, "1"), request)?;
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("cargo-style: parsed {parses} blobs\n"),
        );
        Ok(output)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct StoredBlobFacts {
    blob_id:        String,
    reader_version: String,
    reading:        String,
}

struct HomeExpansionPaths {
    named_state:  PathBuf,
    private_home: PathBuf,
}

fn rows(database: &Path) -> TestResult<Vec<StoredBlobFacts>> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare(
        "SELECT blob_id, reader_version, CAST(reading AS TEXT)
         FROM blob_facts ORDER BY blob_id, reader_version",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(StoredBlobFacts {
            blob_id:        row.get(0)?,
            reader_version: row.get(1)?,
            reading:        row.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn reader(state: &Path) -> FixtureCommand {
    let mut command = FixtureCommand::new(env!("CARGO_BIN_EXE_cargo-style"));
    command
        .env(STATE_DIRECTORY, state)
        .env_remove(PARSE_COUNT_ENV)
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C")
        .env("RAYON_NUM_THREADS", "2");
    command
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn answer(command: &mut FixtureCommand, request: &Value) -> TestResult<Output> {
    let output = start(command)?.output_with_input(&serde_json::to_vec(request)?)?;
    assert_success(&output);
    Ok(output)
}

fn reply_lines(output: &Output) -> TestResult<Vec<Value>> {
    Ok(std::str::from_utf8(&output.stdout)?
        .lines()
        .map(serde_json::from_str)
        .collect::<serde_json::Result<_>>()?)
}

fn command_text(command: &mut FixtureCommand) -> TestResult<String> {
    let output = start(command)?.wait_with_output()?;
    assert_success(&output);
    Ok(String::from_utf8(output.stdout)?.trim_end().to_owned())
}

fn home_expansion_paths(root: &Path) -> TestResult<HomeExpansionPaths> {
    let account = command_text(FixtureCommand::new("id").arg("-un"))?;
    assert_ne!(account, "");
    assert!(
        account
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    );
    let home = command_text(
        FixtureCommand::new("/bin/sh")
            .arg("-c")
            .arg(format!("printf '%s\\n' ~{account}")),
    )?;
    let home = Path::new(&home).canonicalize()?;
    let root = root.canonicalize()?;
    let mut named_state = PathBuf::from(format!("~{account}"));
    let mut private_home = root.join("private-home");
    // Keep the state path inside root even if the reader does not expand the user name.
    for _ in home.ancestors().skip(1) {
        named_state.push("..");
        private_home.push("level");
    }
    named_state.push(root.strip_prefix("/")?);
    named_state.push("named-state");
    Ok(HomeExpansionPaths {
        named_state,
        private_home,
    })
}

#[test]
fn cold_and_warm_replies_match_the_uncached_reply_byte_for_byte() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blob = fixture.write_blob(SAMPLE_SOURCE)?;
    let request = fixture.explicit_request(&[blob], &ALL_FACTS);
    let uncached = answer(&mut reader(&fixture.unavailable_state()?), &request)?;
    let cold = fixture.read(&request, 1)?;
    let warm = fixture.read(&request, 0)?;
    assert_eq!(reply_lines(&uncached)?.len(), ALL_FACTS.len());
    assert_eq!(cold.stdout, uncached.stdout);
    assert_eq!(warm.stdout, uncached.stdout);
    Ok(())
}

#[test]
fn a_warm_request_parses_zero_blobs_when_it_asks_for_more_fact_kinds() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blobs = [fixture.write_blob(SAMPLE_SOURCE)?];
    fixture.read(&fixture.explicit_request(&blobs, &["lines"]), 1)?;
    let request = fixture.explicit_request(&blobs, &ALL_FACTS);
    let warm = fixture.read(&request, 0)?;
    let uncached = answer(&mut reader(&fixture.unavailable_state()?), &request)?;
    assert_eq!(warm.stdout, uncached.stdout);
    let connection =
        Connection::open_with_flags(fixture.database(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    assert_eq!(journal_mode, "wal");
    Ok(())
}

#[test]
fn two_paths_with_the_same_blob_id_parse_once_and_share_the_worktree_cache() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let commit =
        fixture.commit_files(&[("first.rs", SAMPLE_SOURCE), ("second.rs", SAMPLE_SOURCE)])?;
    let repository = fixture.repository();
    let mut request = json!({"repository":repository, "commits":[commit], "facts":["lines"]});
    let cold = fixture.read(&request, 1)?;
    assert_eq!(reply_lines(&cold)?.len(), 1);
    assert_eq!(rows(&fixture.database())?.len(), 1);

    let checkout = fixture.directory.path().join("linked-checkout");
    git_input(
        &repository,
        &[
            "worktree",
            "add",
            "--detach",
            checkout.to_str().ok_or("invalid checkout path")?,
            &commit,
        ],
        &[],
    )?;
    request["repository"] = json!(checkout);
    let warm = fixture.read(&request, 0)?;
    assert_eq!(warm.stdout, cold.stdout);
    assert!(!fixture.state().join("linked-checkout").exists());
    Ok(())
}

#[test]
fn changing_the_stored_reader_version_causes_a_cache_miss() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blobs = [fixture.write_blob(SAMPLE_SOURCE)?];
    let request = fixture.explicit_request(&blobs, &ALL_FACTS);
    let cold = fixture.read(&request, 1)?;
    let current = rows(&fixture.database())?;
    assert_eq!(current.len(), 1);
    let previous_version = "previous-reader-version";
    {
        let connection = Connection::open(fixture.database())?;
        assert_eq!(
            connection.execute(
                "UPDATE blob_facts SET reader_version = ?1",
                [previous_version]
            )?,
            1,
        );
    }
    let reparsed = fixture.read(&request, 1)?;
    assert_eq!(reparsed.stdout, cold.stdout);
    let after = rows(&fixture.database())?;
    assert_eq!(after.len(), 2);
    assert!(after.contains(&current[0]));
    assert!(
        after
            .iter()
            .any(|row| row.reader_version == previous_version)
    );
    Ok(())
}

#[test]
fn malformed_utf8_has_a_stored_reading_and_the_same_warm_reply() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blob = fixture.write_blob(b"fn sample() {} // \xff\n")?;
    let request = fixture.explicit_request(slice::from_ref(&blob), &["rust_code", "tokens"]);
    let cold = fixture.read(&request, 1)?;
    let lines = reply_lines(&cold)?;
    assert!(lines.contains(&json!({"blob":blob, "fact":"rust_code", "unreadable":"not_utf8"})));
    assert!(
        lines
            .iter()
            .any(|line| line["fact"] == "tokens" && line["value"].is_array())
    );
    let stored = rows(&fixture.database())?;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].blob_id, blob);
    let reading: Value = serde_json::from_str(&stored[0].reading)?;
    assert!(reading.get("malformed_utf8").is_some(), "{reading}");
    let warm = fixture.read(&request, 0)?;
    assert_eq!(warm.stdout, cold.stdout);
    Ok(())
}

#[test]
fn a_missing_object_is_read_after_it_appears() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blob = git_input(
        &fixture.repository(),
        &["hash-object", "--stdin"],
        SAMPLE_SOURCE,
    )?;
    let request = fixture.explicit_request(slice::from_ref(&blob), &["lines"]);
    let missing = fixture.read(&request, 0)?;
    assert_eq!(
        reply_lines(&missing)?,
        [json!({"blob":blob, "fact":"lines", "unreadable":"missing_blob"})],
    );
    if fixture.database().exists() {
        assert_eq!(rows(&fixture.database())?, [] as [StoredBlobFacts; 0]);
    }
    assert_eq!(fixture.write_blob(SAMPLE_SOURCE)?, blob);
    let found = fixture.read(&request, 1)?;
    assert_eq!(
        reply_lines(&found)?,
        [json!({"blob":blob, "fact":"lines", "value":1})]
    );
    assert_eq!(rows(&fixture.database())?.len(), 1);
    Ok(())
}

#[test]
fn a_warm_request_reports_a_cached_object_that_git_no_longer_has() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blob = fixture.write_blob(SAMPLE_SOURCE)?;
    let request = fixture.explicit_request(slice::from_ref(&blob), &["lines"]);
    let cold = fixture.read(&request, 1)?;
    assert_eq!(
        reply_lines(&cold)?,
        [json!({"blob":blob, "fact":"lines", "value":1})],
    );
    assert_eq!(rows(&fixture.database())?.len(), 1);

    let (prefix, suffix) = blob.split_at(2);
    std::fs::remove_file(
        fixture
            .repository()
            .join(".git/objects")
            .join(prefix)
            .join(suffix),
    )?;
    let missing = fixture.read(&request, 0)?;
    assert_eq!(
        reply_lines(&missing)?,
        [json!({"blob":blob, "fact":"lines", "unreadable":"missing_blob"})],
    );
    let uncached = answer(&mut reader(&fixture.unavailable_state()?), &request)?;
    assert_eq!(missing.stdout, uncached.stdout);
    Ok(())
}

#[test]
fn cold_and_warm_requests_ignore_blob_replacements() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let repository = fixture.repository();
    let blob = fixture.write_blob(SAMPLE_SOURCE)?;
    let replacement = fixture.write_blob(b"fn sample() {}\nfn extra() {}\n")?;
    let request = fixture.explicit_request(slice::from_ref(&blob), &["lines"]);
    let expected = [json!({"blob":blob, "fact":"lines", "value":1})];
    assert_eq!(reply_lines(&fixture.read(&request, 1)?)?, expected);

    // gix 0.89 loads replacement refs when core.useReplaceRefs is false.
    git_input(
        &repository,
        &["config", "core.useReplaceRefs", "false"],
        &[],
    )?;
    git_input(&repository, &["replace", &blob, &replacement], &[])?;
    assert_eq!(reply_lines(&fixture.read(&request, 0)?)?, expected);

    let cold_state = fixture.directory.path().join("cold-state");
    let cold = answer(&mut reader(&cold_state), &request)?;
    let warm = answer(&mut reader(&cold_state), &request)?;
    assert_eq!(reply_lines(&cold)?, expected);
    assert_eq!(reply_lines(&warm)?, expected);
    Ok(())
}

#[test]
fn a_warm_request_propagates_a_pack_index_error() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let repository = fixture.repository();
    let blob = fixture.write_blob(SAMPLE_SOURCE)?;
    let pack = git_input(
        &repository,
        &["pack-objects", ".git/objects/pack/pack"],
        format!("{blob}\n").as_bytes(),
    )?;
    let objects = repository.join(".git/objects");
    let (prefix, suffix) = blob.split_at(2);
    std::fs::remove_file(objects.join(prefix).join(suffix))?;

    let request = fixture.explicit_request(slice::from_ref(&blob), &["lines"]);
    fixture.read(&request, 1)?;
    fixture.read(&request, 0)?;
    let index = objects.join("pack").join(format!("pack-{pack}.idx"));
    std::fs::remove_file(&index)?;
    std::fs::write(index, b"invalid pack index")?;

    for state in [fixture.unavailable_state()?, fixture.state()] {
        let output =
            start(&mut reader(&state))?.output_with_input(&serde_json::to_vec(&request)?)?;
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        let reply = std::str::from_utf8(&output.stdout)?;
        assert!(
            !output.status.success(),
            "{} returned {reply}",
            state.display()
        );
        assert_eq!(reply, "");
        assert!(
            diagnostic.contains(&format!("cannot read blob {blob}")),
            "{diagnostic}"
        );
    }
    Ok(())
}

#[test]
fn the_state_directory_expands_direct_and_dot_prefixed_home_forms() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let root = fixture.directory.path();
    let HomeExpansionPaths {
        named_state,
        private_home,
    } = home_expansion_paths(root)?;
    std::fs::create_dir_all(&private_home)?;
    let blob = fixture.write_blob(SAMPLE_SOURCE)?;
    let request = fixture.explicit_request(slice::from_ref(&blob), &["lines"]);
    for (configured_state, expanded_state) in [
        (
            PathBuf::from("~/unnamed-state"),
            private_home.join("unnamed-state"),
        ),
        (
            PathBuf::from("./~/dot-unnamed-state"),
            private_home.join("dot-unnamed-state"),
        ),
        (
            Path::new(".").join(named_state.with_file_name("dot-named-state")),
            root.join("dot-named-state"),
        ),
        (named_state, root.join("named-state")),
    ] {
        let output = answer(
            reader(&configured_state)
                .env("HOME", &private_home)
                .current_dir(&private_home),
            &request,
        )?;
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let database = expanded_state
            .join(REPOSITORY_NAME)
            .join(FACT_DATABASE_NAME);
        assert!(
            database.is_file(),
            "expected cache at {}",
            database.display()
        );
        let stored = rows(&database)?;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].blob_id, blob);
    }
    Ok(())
}

#[test]
fn an_explicit_blob_request_parses_only_the_object_named_by_one_path() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    fixture.commit_files(&[
        ("requested.rs", SAMPLE_SOURCE),
        ("other.rs", b"fn other() {}\n"),
    ])?;
    let blob = git_input(
        &fixture.repository(),
        &["rev-parse", "HEAD:requested.rs"],
        &[],
    )?;
    let request = fixture.explicit_request(slice::from_ref(&blob), &["lines"]);
    let output = fixture.read(&request, 1)?;
    assert_eq!(
        reply_lines(&output)?,
        [json!({"blob":blob, "fact":"lines", "value":1})]
    );
    let stored = rows(&fixture.database())?;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].blob_id, blob);
    Ok(())
}

#[test]
fn an_unavailable_cache_reports_one_warning_and_preserves_the_reply() -> TestResult<()> {
    let fixture = RepositoryFixture::new()?;
    let blob = fixture.write_blob(SAMPLE_SOURCE)?;
    let request = fixture.explicit_request(&[blob], &ALL_FACTS);
    let cached = fixture.read(&request, 1)?;
    let uncached = answer(&mut reader(&fixture.unavailable_state()?), &request)?;
    assert_eq!(uncached.stdout, cached.stdout);
    let diagnostic = std::str::from_utf8(&uncached.stderr)?;
    assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
    assert!(
        diagnostic.starts_with("cargo-style: fact cache unavailable:"),
        "{diagnostic}"
    );
    Ok(())
}
