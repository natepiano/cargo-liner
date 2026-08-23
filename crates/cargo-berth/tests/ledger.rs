#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end ledger durability tests against disposable git repositories.

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;
use tempfile::tempdir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const INITIALIZED_MESSAGE: &str = "Initialized the cargo-berth ledger.\n";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";

#[test]
fn init_creates_the_shared_ledger_and_is_idempotent() {
    let repository = scratch_repository();

    let first_init = run_berth(repository.path(), ["init", "--json"]);
    let second_init = run_berth(repository.path(), ["init", "--json"]);

    assert!(first_init.status.success());
    assert!(second_init.status.success());
    assert!(repository.path().join(JOURNAL_PATH).is_file());
    assert!(repository.path().join(PROJECTION_PATH).is_file());
    assert!(repository.path().join(CONFIGURATION_PATH).is_file());
}

#[test]
fn init_leaves_an_edited_configuration_untouched() {
    let repository = scratch_repository();
    let first_init = run_berth(repository.path(), ["init"]);
    assert!(first_init.status.success());
    let edited_configuration = "trunk = \"trunk\"\nmaximum_reservations = 1\nmaximum_ordering_edges = 0\ngate_mode = \"enforce\"\n";
    fs::write(
        repository.path().join(CONFIGURATION_PATH),
        edited_configuration,
    )
    .expect("edited configuration should write");

    let second_init = run_berth(repository.path(), ["init"]);

    assert!(second_init.status.success());
    assert_eq!(
        fs::read_to_string(repository.path().join(CONFIGURATION_PATH))
            .expect("configuration should read"),
        edited_configuration
    );
}

#[test]
fn init_from_a_subdirectory_writes_one_configuration_at_repository_root() {
    let repository = scratch_repository();
    let subdirectory = repository.path().join("crates").join("nested");
    fs::create_dir_all(&subdirectory).expect("subdirectory should exist");

    let initialized = run_berth(&subdirectory, ["init", "--json"]);

    assert!(initialized.status.success());
    assert!(repository.path().join(CONFIGURATION_PATH).is_file());
    assert!(!subdirectory.join(CONFIGURATION_PATH).exists());
    let configuration_directory = repository.path().join(".claude").join("config");
    let mut configuration_count = 0;
    for directory_entry in
        fs::read_dir(configuration_directory).expect("configuration directory should read")
    {
        let directory_entry = directory_entry.expect("configuration entry should read");
        if directory_entry.file_name() == "berth.toml" {
            configuration_count += 1;
        }
    }
    assert_eq!(configuration_count, 1);
}

#[test]
fn deleted_projection_rebuilds_byte_for_byte_from_the_journal() {
    let repository = scratch_repository();
    let first_init = run_berth(repository.path(), ["init"]);
    assert!(first_init.status.success());
    let projection_path = repository.path().join(PROJECTION_PATH);
    let first_projection = fs::read(&projection_path).expect("projection should read");
    fs::remove_file(&projection_path).expect("projection should delete");

    let rebuild = run_berth(repository.path(), ["init"]);

    assert!(rebuild.status.success());
    assert_eq!(
        fs::read(projection_path).expect("rebuilt projection should read"),
        first_projection
    );
}

#[test]
fn init_repairs_only_a_truncated_final_journal_record() {
    let repository = scratch_repository();
    let initialized = run_berth(repository.path(), ["init"]);
    assert!(initialized.status.success());
    let journal_path = repository.path().join(JOURNAL_PATH);
    OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .expect("journal should open for tail write")
        .write_all(b"{\"op\":")
        .expect("tail should write");

    let repaired = run_berth(repository.path(), ["init"]);

    assert!(repaired.status.success());
    assert_eq!(fs::read(journal_path).expect("journal should read"), b"");
}

#[test]
fn init_rejects_a_corrupt_middle_journal_record() {
    let repository = scratch_repository();
    let initialized = run_berth(repository.path(), ["init"]);
    assert!(initialized.status.success());
    fs::write(
        repository.path().join(JOURNAL_PATH),
        b"{}\n{\"not\":\"a journal event\"}\n",
    )
    .expect("corrupt journal should write");

    let failed_init = run_berth(repository.path(), ["init", "--json"]);
    let output: serde_json::Value = serde_json::from_slice(&failed_init.stdout)
        .expect("failed init should still render its envelope");

    assert_eq!(failed_init.status.code(), Some(4));
    assert_eq!(output["status"], "ledger_unreadable");
    assert_eq!(output["exit_code"], 4);
    assert_eq!(output["payload"]["kind"], "no_facts");
    assert!(output["payload"].get("ledger").is_none());
    assert!(output["payload"].get("configuration").is_none());
}

#[test]
fn init_agrees_across_process_status_json_and_text() {
    let repository = scratch_repository();

    let json_init = run_berth(repository.path(), ["init", "--json"]);
    let text_init = run_berth(repository.path(), ["init"]);
    let json: serde_json::Value =
        serde_json::from_slice(&json_init.stdout).expect("json init should render an envelope");

    assert_eq!(json_init.status.code(), Some(0));
    assert_eq!(json["status"], "initialized");
    assert_eq!(json["exit_code"], 0);
    assert_eq!(
        String::from_utf8(text_init.stdout).expect("text output should be UTF-8"),
        INITIALIZED_MESSAGE
    );
    assert!(text_init.status.success());
}

fn scratch_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    let git_init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(repository.path())
        .output()
        .expect("git should initialize a scratch repository");
    assert!(git_init.status.success());
    repository
}

fn run_berth<const ARGUMENT_COUNT: usize>(
    repository_root: &Path,
    arguments: [&str; ARGUMENT_COUNT],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .output()
        .expect("cargo-berth should run")
}
