#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end ledger durability tests against disposable git repositories.

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use tempfile::TempDir;
use tempfile::tempdir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const GIT_COMMON_DIRECTORY_ENVIRONMENT: &str = "GIT_COMMON_DIR";
const GIT_DIRECTORY_ENVIRONMENT: &str = "GIT_DIR";
const INITIALIZED_MESSAGE: &str = "Initialized the cargo-berth ledger.\n";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const MAIN_COORDINATION_RUN_ID: &str = "01a03f63-03e7-7fb2-ae63-5b297177f59f";
const MAIN_WORKTREE_ID: &str = "01a03f08-e197-7a83-9b7c-bc7c555d0c00";
const OVERSIZED_IDENTITY_INPUT_BYTES: usize = 32 * 1_024;
const PREVIOUS_PROJECTION_SCHEMA_VERSION: u64 = 2;
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const PROJECTION_REPLAY_METADATA_FIELDS: [&str; 3] =
    ["generation", "journal_end_offset", "journal_fingerprint"];
const PROJECTION_SIZE_ADDITIONAL_RENEWALS: usize = 9;
const PROJECTION_SIZE_INITIAL_RENEWALS: usize = 18;
const RECORDED_INCIDENT_COORDINATION_RUN_ID: &str = "01a03f60-2e87-7b93-b933-e3dc5e9211d9";
const RECORDED_INCIDENT_WORKTREE_ID: &str = "01a03f1f-6d9c-7383-8389-a6fd541e79d5";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const RUN_MARKER_FILE_NAME: &str = "cargo-berth-run-id";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const UNKNOWN_RESERVATION_ID: &str = "01a03f08-e197-7a83-9b7c-bc7c555d0c01";
const WORKTREE_ID_FILE_NAME: &str = "cargo-berth-worktree-id";

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
fn claim_resolves_reftable_head_and_trunk_references() {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &[
            "init",
            "--quiet",
            "--initial-branch",
            "main",
            "--ref-format=reftable",
        ],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    fs::write(repository.path().join("README.md"), "reftable\n").expect("base file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    assert!(
        run_berth(repository.path(), ["init", "--json"])
            .status
            .success()
    );
    git(repository.path(), &["add", CONFIGURATION_PATH]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "configure berth",
        ],
    );

    let claimed = run_berth(
        repository.path(),
        [
            "claim",
            "file:README.md",
            "--run",
            MAIN_COORDINATION_RUN_ID,
            "--json",
        ],
    );

    assert!(
        claimed.status.success(),
        "reftable claim failed: stdout={} stderr={}",
        String::from_utf8_lossy(&claimed.stdout),
        String::from_utf8_lossy(&claimed.stderr),
    );
    assert_eq!(json_output(&claimed)["status"], "claimed");
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
fn projection_size_does_not_grow_with_journal_event_count() -> Result<(), Box<dyn std::error::Error>>
{
    let repository = initialized_repository();
    let reservation_id = claim(
        repository.path(),
        "file:projection-size.rs",
        MAIN_COORDINATION_RUN_ID,
        "projection-size",
    );

    for _ in 0..PROJECTION_SIZE_INITIAL_RENEWALS {
        let renewed = run_berth_with_session(
            repository.path(),
            &["renew", &reservation_id, "--json"],
            "projection-size",
        );
        assert!(renewed.status.success());
    }
    let short_journal_events = journal_events(repository.path());
    let short_projection_size = projection_size_with_replay_metadata_normalized(&fs::read(
        repository.path().join(PROJECTION_PATH),
    )?)?;

    for _ in 0..PROJECTION_SIZE_ADDITIONAL_RENEWALS {
        let renewed = run_berth_with_session(
            repository.path(),
            &["renew", &reservation_id, "--json"],
            "projection-size",
        );
        assert!(renewed.status.success());
    }
    let long_journal_events = journal_events(repository.path());
    let long_projection_size = projection_size_with_replay_metadata_normalized(&fs::read(
        repository.path().join(PROJECTION_PATH),
    )?)?;

    assert!(long_journal_events.len() > short_journal_events.len());
    assert_eq!(
        short_journal_events
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        1
    );
    assert_eq!(
        long_journal_events
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        1
    );
    assert!(
        short_journal_events
            .iter()
            .all(|event| event["op"] != "release")
    );
    assert!(
        long_journal_events
            .iter()
            .all(|event| event["op"] != "release")
    );
    assert_eq!(short_projection_size, long_projection_size);
    Ok(())
}

#[test]
fn previous_projection_schema_rebuilds_without_changing_the_journal()
-> Result<(), Box<dyn std::error::Error>> {
    let repository = initialized_repository();
    let reservation_id = claim(
        repository.path(),
        "file:old-projection.rs",
        MAIN_COORDINATION_RUN_ID,
        "old-projection",
    );
    let projection_path = repository.path().join(PROJECTION_PATH);
    let journal_path = repository.path().join(JOURNAL_PATH);
    let journal_before = fs::read(&journal_path)?;
    let events_before = journal_events(repository.path());
    assert!(
        events_before
            .iter()
            .any(|event| { event["op"] == "claim" && event["reservation_id"] == reservation_id })
    );
    let mut previous_projection: serde_json::Value =
        serde_json::from_slice(&fs::read(&projection_path)?)?;
    assert_eq!(previous_projection["schema_version"], 3);
    previous_projection["schema_version"] = serde_json::json!(PREVIOUS_PROJECTION_SCHEMA_VERSION);
    previous_projection["events"] = serde_json::Value::Array(events_before.clone());
    let mut serialized_previous_projection = serde_json::to_vec_pretty(&previous_projection)?;
    serialized_previous_projection.push(b'\n');
    fs::write(&projection_path, serialized_previous_projection)?;

    let rebuilt = run_berth_with_session(
        repository.path(),
        &["renew", UNKNOWN_RESERVATION_ID, "--json"],
        "old-projection",
    );

    assert_eq!(rebuilt.status.code(), Some(5));
    let rebuilt_projection: serde_json::Value =
        serde_json::from_slice(&fs::read(projection_path)?)?;
    assert_eq!(rebuilt_projection["schema_version"], 3);
    assert!(rebuilt_projection.get("events").is_none());
    assert_eq!(
        rebuilt_projection
            .as_object()
            .ok_or("projection should be an object")?
            .len(),
        5
    );
    assert_eq!(fs::read(journal_path)?, journal_before);
    assert_eq!(journal_events(repository.path()), events_before);
    Ok(())
}

#[test]
fn unsupported_projection_schema_rebuilds_without_changing_the_journal()
-> Result<(), Box<dyn std::error::Error>> {
    let repository = initialized_repository();
    let projection_path = repository.path().join(PROJECTION_PATH);
    let journal_path = repository.path().join(JOURNAL_PATH);
    let projection_before = fs::read(&projection_path)?;
    let journal_before = fs::read(&journal_path)?;
    let mut unsupported_projection: serde_json::Value = serde_json::from_slice(&projection_before)?;
    let current_schema_version = unsupported_projection["schema_version"]
        .as_u64()
        .ok_or("projection schema version should be an unsigned integer")?;
    unsupported_projection["schema_version"] = serde_json::json!(current_schema_version + 1);
    let mut serialized_unsupported_projection = serde_json::to_vec_pretty(&unsupported_projection)?;
    serialized_unsupported_projection.push(b'\n');
    fs::write(&projection_path, serialized_unsupported_projection)?;

    let rebuilt = run_berth_with_session(
        repository.path(),
        &["renew", UNKNOWN_RESERVATION_ID, "--json"],
        "unsupported-projection",
    );

    assert_eq!(rebuilt.status.code(), Some(5));
    assert_eq!(fs::read(projection_path)?, projection_before);
    assert_eq!(fs::read(journal_path)?, journal_before);
    Ok(())
}

#[test]
fn explicit_projection_repair_changes_neither_journal_nor_configuration() {
    let repository = scratch_repository();
    assert!(run_berth(repository.path(), ["init"]).status.success());
    let journal_path = repository.path().join(JOURNAL_PATH);
    OpenOptions::new()
        .append(true)
        .open(&journal_path)
        .expect("journal should open for incomplete tail write")
        .write_all(b"{\"op\":")
        .expect("incomplete journal tail should write");
    let journal_before = fs::read(&journal_path).expect("journal should read before repair");
    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    fs::remove_file(repository.path().join(CONFIGURATION_PATH))
        .expect("configuration should delete");

    let repaired = run_berth(repository.path(), ["init", "--repair-projection", "--json"]);
    let output: serde_json::Value =
        serde_json::from_slice(&repaired.stdout).expect("repair should render JSON");

    assert!(repaired.status.success());
    assert_eq!(output["status"], "projection_repaired");
    assert_eq!(output["payload"]["kind"], "projection_repair");
    assert_eq!(
        output["payload"]["data"]["projection"],
        "reservations_json_rebuilt"
    );
    assert_eq!(output["payload"]["data"]["journal"], "unchanged");
    assert_eq!(
        fs::read(journal_path).expect("journal should read after repair"),
        journal_before
    );
    assert!(!repository.path().join(CONFIGURATION_PATH).exists());
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

#[test]
fn recorded_linked_worktree_resolve_incident_uses_the_invoking_actor() {
    let repository = initialized_repository();
    write_identity_markers(
        &repository.path().join(".git"),
        MAIN_WORKTREE_ID,
        MAIN_COORDINATION_RUN_ID,
    );
    let worktrees = tempdir().expect("worktree parent should exist");
    let linked_root = add_worktree(repository.path(), worktrees.path(), "cargo-tile-favorites");
    let linked_administrative_directory = worktree_administrative_directory(&linked_root);
    write_identity_markers(
        &linked_administrative_directory,
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );
    let holder_id = claim(
        repository.path(),
        "tree:shared",
        MAIN_COORDINATION_RUN_ID,
        "main-holder-session",
    );
    let subject_id = claim(
        &linked_root,
        "file:owned.txt",
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
        "favorites-phase-1",
    );
    fs::create_dir_all(linked_root.join("shared")).expect("shared directory should exist");
    fs::write(linked_root.join("shared/entered.txt"), "incursion\n")
        .expect("incursion path should write");

    let incursion = run_berth_with_session(
        &linked_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        "favorites-phase-1",
    );
    assert_eq!(incursion.status.code(), Some(1));
    let incursion_event = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "incursion")
        .expect("drift should append an incursion");
    assert_eq!(
        incursion_event["actor"]["worktree"],
        RECORDED_INCIDENT_WORKTREE_ID
    );
    assert_eq!(
        incursion_event["actor"]["run"],
        RECORDED_INCIDENT_COORDINATION_RUN_ID
    );
    assert_eq!(
        incursion_event["foreign_reservation_ids"],
        serde_json::json!([holder_id])
    );
    let incident_id = incursion_event["incident_id"]
        .as_str()
        .expect("incursion should carry an incident id")
        .to_owned();

    let resolved = run_berth_with_session(
        &linked_root,
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
        "favorites-phase-1",
    );
    assert!(
        resolved.status.success(),
        "resolve failed: {}",
        String::from_utf8_lossy(&resolved.stdout)
    );
    let resolution_event = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "resolve_incursion")
        .expect("resolve should append its decision");
    assert_eq!(
        resolution_event["actor"]["worktree"],
        RECORDED_INCIDENT_WORKTREE_ID
    );
    assert_eq!(
        resolution_event["actor"]["run"],
        RECORDED_INCIDENT_COORDINATION_RUN_ID
    );
    assert_recorded_identity_inputs(
        &resolution_event,
        &linked_root,
        &serde_json::json!({"status": "utf8", "value": "favorites-phase-1"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
    );
}

#[test]
fn main_checkout_claim_records_the_invoking_actor_with_unset_environment() {
    let repository = initialized_repository();
    write_identity_markers(
        &repository.path().join(".git"),
        MAIN_WORKTREE_ID,
        MAIN_COORDINATION_RUN_ID,
    );

    let claimed = run_berth(
        repository.path(),
        [
            "claim",
            "file:main-owned.rs",
            "--run",
            MAIN_COORDINATION_RUN_ID,
            "--why",
            "main identity fixture",
            "--json",
        ],
    );
    assert!(claimed.status.success());
    let claim_event = last_journal_operation(repository.path(), "claim");

    assert_journalled_actor(&claim_event, MAIN_WORKTREE_ID, MAIN_COORDINATION_RUN_ID);
    assert_recorded_identity_inputs(
        &claim_event,
        repository.path(),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
    );
}

#[test]
fn oversized_session_identity_input_still_appends_the_claim() {
    let repository = initialized_repository();
    let oversized_session_id = "s".repeat(OVERSIZED_IDENTITY_INPUT_BYTES);

    let claimed = run_berth_with_session(
        repository.path(),
        &[
            "claim",
            "file:oversized-session.rs",
            "--run",
            MAIN_COORDINATION_RUN_ID,
            "--why",
            "oversized identity input fixture",
            "--json",
        ],
        &oversized_session_id,
    );

    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    let claim_event = last_journal_operation(repository.path(), "claim");
    assert_eq!(claim_event["actor"]["run"], MAIN_COORDINATION_RUN_ID);
    assert_eq!(
        claim_event["identity_inputs"]["cargo_berth_session_id"],
        serde_json::json!({
            "status": "too_long",
            "observed_bytes": OVERSIZED_IDENTITY_INPUT_BYTES,
        })
    );
}

#[test]
fn relative_git_environment_does_not_replace_invocation_filesystem_identity() {
    let repository = initialized_repository();
    write_identity_markers(
        &repository.path().join(".git"),
        MAIN_WORKTREE_ID,
        MAIN_COORDINATION_RUN_ID,
    );
    let claimed = run_berth_with_git_environment(
        repository.path(),
        &[
            "claim",
            "file:relative-environment.rs",
            "--run",
            MAIN_COORDINATION_RUN_ID,
            "--why",
            "relative Git environment fixture",
            "--json",
        ],
        ".git",
        ".git",
    );
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    let claim_event = last_journal_operation(repository.path(), "claim");

    assert_journalled_actor(&claim_event, MAIN_WORKTREE_ID, MAIN_COORDINATION_RUN_ID);
    assert_recorded_identity_inputs(
        &claim_event,
        repository.path(),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "utf8", "value": ".git"}),
        &serde_json::json!({"status": "utf8", "value": ".git"}),
    );
}

#[test]
fn separate_git_directory_claim_records_the_invoking_actor() {
    let fixture = separate_git_directory_repository();
    let worktree_root = fixture.temporary_directory.path().join("worktree");
    write_identity_markers(
        &fixture.administrative_directory,
        MAIN_WORKTREE_ID,
        MAIN_COORDINATION_RUN_ID,
    );

    let claimed = claim(
        &worktree_root,
        "file:separate-git-dir.rs",
        MAIN_COORDINATION_RUN_ID,
        "separate-git-directory",
    );
    let claim_event = journal_events_at(&fixture.journal_path)
        .into_iter()
        .find(|event| event["op"] == "claim" && event["reservation_id"] == claimed)
        .expect("claim should append to the separate Git directory");

    assert_journalled_actor(&claim_event, MAIN_WORKTREE_ID, MAIN_COORDINATION_RUN_ID);
    assert_recorded_identity_inputs(
        &claim_event,
        &worktree_root,
        &serde_json::json!({"status": "utf8", "value": "separate-git-directory"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
    );
}

#[test]
fn submodule_claim_records_the_submodule_actor() {
    let parent = initialized_repository();
    let source = scratch_repository();
    let submodule_root = parent.path().join("component");
    git(
        parent.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            source.path().to_str().expect("source path should be UTF-8"),
            "component",
        ],
    );
    git(
        parent.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "add component submodule",
        ],
    );
    assert!(
        run_berth(&submodule_root, ["init", "--json"])
            .status
            .success()
    );
    let administrative_directory = worktree_administrative_directory(&submodule_root);
    write_identity_markers(
        &administrative_directory,
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );

    let claimed = claim(
        &submodule_root,
        "file:submodule-owned.rs",
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
        "submodule-session",
    );
    let journal_path = administrative_directory
        .join("cargo-berth")
        .join("journal.ndjson");
    let claim_event = journal_events_at(&journal_path)
        .into_iter()
        .find(|event| event["op"] == "claim" && event["reservation_id"] == claimed)
        .expect("submodule claim should append to its administrative directory");

    assert_journalled_actor(
        &claim_event,
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );
    assert_recorded_identity_inputs(
        &claim_event,
        &submodule_root,
        &serde_json::json!({"status": "utf8", "value": "submodule-session"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
    );
}

#[test]
fn integrated_as_replacement_uses_invoking_worktree_actor() {
    let repository = initialized_repository();
    write_identity_markers(
        &repository.path().join(".git"),
        MAIN_WORKTREE_ID,
        MAIN_COORDINATION_RUN_ID,
    );
    let worktrees = tempdir().expect("worktree parent should exist");
    let linked_root = add_worktree(repository.path(), worktrees.path(), "replacement-operator");
    write_identity_markers(
        &worktree_administrative_directory(&linked_root),
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );
    assert_linked_replacement_operator(&linked_root, repository.path());
    let reservation_id = release_rewritten_reservation(repository.path());

    git(repository.path(), &["switch", "--quiet", "main"]);
    let first_evidence = commit_integration_evidence(
        repository.path(),
        "integrated.rs",
        "first evidence\n",
        "first integration evidence",
    );
    let first_resolution = resolve_integrated_as_from_linked_worktree(
        &linked_root,
        repository.path(),
        &reservation_id,
        &first_evidence,
        "release",
    );
    assert_journalled_actor(
        &first_resolution,
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );

    invalidate_release_disposition(repository.path(), &reservation_id);
    let replacement_evidence = commit_integration_evidence(
        repository.path(),
        "integrated-again.rs",
        "replacement evidence\n",
        "replacement integration evidence",
    );
    let replacement_event = resolve_integrated_as_from_linked_worktree(
        &linked_root,
        repository.path(),
        &reservation_id,
        &replacement_evidence,
        "replace_release_disposition",
    );
    assert_journalled_actor(
        &replacement_event,
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );
    assert_recorded_identity_inputs(
        &replacement_event,
        &linked_root,
        &serde_json::json!({"status": "utf8", "value": "replacement-operator"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
        &serde_json::json!({"status": "unset"}),
    );
}

fn assert_linked_replacement_operator(linked_root: &Path, repository_root: &Path) {
    let reservation_id = claim(
        linked_root,
        "file:operator-owned.rs",
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
        "replacement-operator",
    );
    let operator_claim = journal_events(repository_root)
        .into_iter()
        .find(|event| event["op"] == "claim" && event["reservation_id"] == reservation_id)
        .expect("operator claim should append");
    assert_journalled_actor(
        &operator_claim,
        RECORDED_INCIDENT_WORKTREE_ID,
        RECORDED_INCIDENT_COORDINATION_RUN_ID,
    );
}

fn release_rewritten_reservation(repository_root: &Path) -> String {
    git(repository_root, &["switch", "--quiet", "-c", "rewritten"]);
    fs::write(repository_root.join("rewritten.rs"), "original result\n")
        .expect("rewritten source should write");
    git(repository_root, &["add", "rewritten.rs"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "rewritten source",
        ],
    );
    let reservation_id = claim(
        repository_root,
        "file:rewritten.rs",
        MAIN_COORDINATION_RUN_ID,
        "rewritten-holder",
    );
    let checkpoint = run_berth_with_session(
        repository_root,
        &["release", &reservation_id, "--json"],
        "rewritten-holder",
    );
    assert!(checkpoint.status.success());
    assert_journalled_actor(
        &last_journal_operation(repository_root, "checkpoint"),
        MAIN_WORKTREE_ID,
        MAIN_COORDINATION_RUN_ID,
    );
    reservation_id
}

fn commit_integration_evidence(
    repository_root: &Path,
    file_name: &str,
    contents: &str,
    message: &str,
) -> String {
    fs::write(repository_root.join(file_name), contents)
        .expect("integration evidence should write");
    git(repository_root, &["add", file_name]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn resolve_integrated_as_from_linked_worktree(
    linked_root: &Path,
    repository_root: &Path,
    reservation_id: &str,
    evidence_commit: &str,
    expected_operation: &str,
) -> serde_json::Value {
    let resolution = run_berth_with_session(
        linked_root,
        &[
            "resolve",
            reservation_id,
            "--integrated-as",
            evidence_commit,
            "--json",
        ],
        "replacement-operator",
    );
    assert!(
        resolution.status.success(),
        "integrated-as resolution failed: {}",
        String::from_utf8_lossy(&resolution.stdout)
    );
    last_journal_operation(repository_root, expected_operation)
}

fn invalidate_release_disposition(repository_root: &Path, reservation_id: &str) {
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            "--quiet",
            "HEAD^",
        ],
    );
    let revalidated = run_berth_with_session(
        repository_root,
        &["release", reservation_id, "--json"],
        "rewritten-holder",
    );
    assert!(revalidated.status.success());
    assert_eq!(json_output(&revalidated)["status"], "trunk_rewritten");
}

#[test]
fn bare_repository_retains_repository_not_found_rejection() {
    let bare_repository = tempdir().expect("bare repository should exist");
    git(bare_repository.path(), &["init", "--bare", "--quiet"]);

    let rejected = run_berth(
        bare_repository.path(),
        [
            "claim",
            "file:bare.rs",
            "--run",
            MAIN_COORDINATION_RUN_ID,
            "--why",
            "bare repository rejection fixture",
            "--json",
        ],
    );
    let envelope = json_output(&rejected);

    assert_eq!(rejected.status.code(), Some(4));
    assert_eq!(envelope["status"], "ledger_unreadable");
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("no containing git worktree"))
    );
}

fn initialized_repository() -> TempDir {
    let repository = scratch_repository();
    assert!(
        run_berth(repository.path(), ["init", "--json"])
            .status
            .success()
    );
    git(repository.path(), &["add", CONFIGURATION_PATH]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "configure berth",
        ],
    );
    repository
}

struct SeparateGitDirectoryRepository {
    temporary_directory:      TempDir,
    administrative_directory: PathBuf,
    journal_path:             PathBuf,
}

fn separate_git_directory_repository() -> SeparateGitDirectoryRepository {
    let temporary_directory = tempdir().expect("temporary repository parent should exist");
    let worktree_root = temporary_directory.path().join("worktree");
    let administrative_directory = temporary_directory.path().join("administrative.git");
    git(
        temporary_directory.path(),
        &[
            "init",
            "--quiet",
            "--initial-branch",
            "main",
            "--separate-git-dir",
            administrative_directory
                .to_str()
                .expect("administrative path should be UTF-8"),
            worktree_root
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );
    git(
        &worktree_root,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&worktree_root, &["config", "user.name", "Berth Test"]);
    fs::write(worktree_root.join("README.md"), "separate git directory\n")
        .expect("base file should write");
    git(&worktree_root, &["add", "README.md"]);
    git(&worktree_root, &["commit", "--quiet", "-m", "initial"]);
    assert!(
        run_berth(&worktree_root, ["init", "--json"])
            .status
            .success()
    );
    git(&worktree_root, &["add", CONFIGURATION_PATH]);
    git(
        &worktree_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "configure berth",
        ],
    );
    let journal_path = administrative_directory
        .join("cargo-berth")
        .join("journal.ndjson");
    SeparateGitDirectoryRepository {
        temporary_directory,
        administrative_directory,
        journal_path,
    }
}

fn scratch_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    fs::write(repository.path().join("README.md"), "scratch\n").expect("base file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    repository
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let root = parent.join(branch);
    git(
        repository_root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            root.to_str().expect("worktree path should be UTF-8"),
            "main",
        ],
    );
    root
}

fn worktree_administrative_directory(worktree_root: &Path) -> PathBuf {
    let git_file = fs::read_to_string(worktree_root.join(".git"))
        .expect("linked worktree git file should read");
    let locator = PathBuf::from(
        git_file
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree should name its administrative directory"),
    );
    if locator.is_absolute() {
        locator
    } else {
        worktree_root.join(locator)
    }
}

fn write_identity_markers(
    administrative_directory: &Path,
    worktree_id: &str,
    coordination_run_id: &str,
) {
    fs::write(
        administrative_directory.join(WORKTREE_ID_FILE_NAME),
        format!("{worktree_id}\n"),
    )
    .expect("worktree identity marker should write");
    fs::write(
        administrative_directory.join(RUN_MARKER_FILE_NAME),
        format!("{coordination_run_id}\n"),
    )
    .expect("coordination run marker should write");
}

fn claim(repository_root: &Path, scope: &str, run: &str, session_id: &str) -> String {
    let claimed = run_berth_with_session(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--why",
            "identity fixture",
            "--json",
        ],
        session_id,
    );
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    json_output(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

fn journal_events(repository_root: &Path) -> Vec<serde_json::Value> {
    journal_events_at(&repository_root.join(JOURNAL_PATH))
}

fn journal_events_at(journal_path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(journal_path)
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .collect()
}

fn projection_size_with_replay_metadata_normalized(
    serialized_projection: &[u8],
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut projection: serde_json::Value = serde_json::from_slice(serialized_projection)?;
    assert!(projection.get("events").is_none());
    let projection_fields = projection
        .as_object_mut()
        .ok_or("projection should be an object")?;
    for field in PROJECTION_REPLAY_METADATA_FIELDS {
        let field_value = projection_fields
            .get_mut(field)
            .ok_or("projection should contain all replay metadata fields")?;
        *field_value = serde_json::Value::from(0);
    }
    Ok(serde_json::to_vec(&projection)?.len())
}

fn last_journal_operation(repository_root: &Path, operation: &str) -> serde_json::Value {
    journal_events(repository_root)
        .into_iter()
        .rfind(|event| event["op"] == operation)
        .expect("journal operation should exist")
}

fn assert_journalled_actor(
    event: &serde_json::Value,
    expected_worktree_id: &str,
    expected_coordination_run_id: &str,
) {
    assert_eq!(event["actor"]["worktree"], expected_worktree_id);
    assert_eq!(event["actor"]["run"], expected_coordination_run_id);
}

fn assert_recorded_identity_inputs(
    event: &serde_json::Value,
    invocation_directory: &Path,
    cargo_berth_session_id: &serde_json::Value,
    cargo_berth_run: &serde_json::Value,
    git_dir: &serde_json::Value,
    git_common_dir: &serde_json::Value,
) {
    let invocation_directory = fs::canonicalize(invocation_directory)
        .expect("invocation directory should canonicalize")
        .to_str()
        .expect("invocation directory should be UTF-8")
        .to_owned();
    assert_eq!(event["identity_inputs"]["status"], "recorded");
    assert_eq!(
        event["identity_inputs"]["invocation_directory"],
        serde_json::json!({"status": "utf8", "path": invocation_directory})
    );
    assert_eq!(
        &event["identity_inputs"]["cargo_berth_session_id"],
        cargo_berth_session_id
    );
    assert_eq!(
        &event["identity_inputs"]["cargo_berth_run"],
        cargo_berth_run
    );
    assert_eq!(&event["identity_inputs"]["git_dir"], git_dir);
    assert_eq!(&event["identity_inputs"]["git_common_dir"], git_common_dir);
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render JSON")
}

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env_remove(GIT_DIRECTORY_ENVIRONMENT)
        .env_remove(GIT_COMMON_DIRECTORY_ENVIRONMENT)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env_remove(GIT_DIRECTORY_ENVIRONMENT)
        .env_remove(GIT_COMMON_DIRECTORY_ENVIRONMENT)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output should be UTF-8")
        .trim()
        .to_owned()
}

fn run_berth<const ARGUMENT_COUNT: usize>(
    repository_root: &Path,
    arguments: [&str; ARGUMENT_COUNT],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env_remove(GIT_DIRECTORY_ENVIRONMENT)
        .env_remove(GIT_COMMON_DIRECTORY_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .env_remove(GIT_DIRECTORY_ENVIRONMENT)
        .env_remove(GIT_COMMON_DIRECTORY_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_git_environment(
    repository_root: &Path,
    arguments: &[&str],
    git_directory: &str,
    git_common_directory: &str,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(GIT_DIRECTORY_ENVIRONMENT, git_directory)
        .env(GIT_COMMON_DIRECTORY_ENVIRONMENT, git_common_directory)
        .output()
        .expect("cargo-berth should run")
}
