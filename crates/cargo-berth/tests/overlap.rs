#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for claim acquisition and mutation-free edit checks.

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: an ordinary checkout, with nothing held back from a hook.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

use std::fs;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tempfile::tempdir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const MUTATION_LOCK_READY_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const READY_WAIT_INTERVAL: Duration = Duration::from_millis(10);
const READY_WAIT_TIMEOUT: Duration = Duration::from_secs(60);
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

#[test]
fn blocked_claim_names_holder_provenance_and_appends_nothing() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let first_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:crates/hana_kana",
            "--run",
            FIRST_RUN,
            "--plan",
            "docs/holder-plan.md",
            "--phase",
            "holder-phase",
            "--why",
            "holder changes kana internals",
            "--json",
        ],
    );
    assert!(first_claim.status.success());
    let sibling_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:crates/hana_kana_extra",
            "--run",
            FIRST_RUN,
            "--json",
        ],
    );
    assert!(sibling_claim.status.success());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before rejection");

    let blocked = run_berth(
        &second_root,
        &["claim", "file:crates/hana_kana/src/lib.rs", "--json"],
    );
    let envelope = json_output(&blocked);

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(envelope["exit_code"], 1);
    assert_eq!(envelope["status"], "blocked_by_overlap");
    assert_eq!(envelope["payload"]["kind"], "claim");
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["head_snapshot"]["full_ref"],
        "refs/heads/main"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["plan"],
        "docs/holder-plan.md"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["phase"],
        "holder-phase"
    );
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("reduce the requested scopes"))
    );
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should read after rejection"),
        journal_before
    );
}

#[test]
fn file_and_tree_scopes_differ_for_descendants() {
    let file_repository = initialized_repository(PathCaseSetting::Sensitive);
    assert!(
        run_berth(
            file_repository.path(),
            &["claim", "file:generated", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    let (_file_second_directory, file_second_root) = foreign_worktree(&file_repository, "second");
    assert!(
        run_berth(
            &file_second_root,
            &["claim", "file:generated/child.rs", "--json"]
        )
        .status
        .success()
    );

    let tree_repository = initialized_repository(PathCaseSetting::Sensitive);
    assert!(
        run_berth(
            tree_repository.path(),
            &["claim", "tree:generated", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    let (_tree_second_directory, tree_second_root) = foreign_worktree(&tree_repository, "second");
    let blocked = run_berth(
        &tree_second_root,
        &["claim", "file:generated/child.rs", "--json"],
    );
    assert_eq!(blocked.status.code(), Some(1));
}

#[test]
fn ignore_case_blocks_component_case_variants() {
    let repository = initialized_repository(PathCaseSetting::Insensitive);
    assert!(
        run_berth(
            repository.path(),
            &["claim", "tree:Crates/Hana", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );

    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let blocked = run_berth(
        &second_root,
        &["claim", "file:crates/hana/src/lib.rs", "--json"],
    );

    assert_eq!(blocked.status.code(), Some(1));
}

#[test]
fn check_reuses_its_runs_reservation_without_git_or_a_duplicate_append() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let claim = run_berth(
        repository.path(),
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(
        claim.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claim.stdout)
    );
    let claim_envelope = json_output(&claim);
    assert_eq!(
        claim_envelope["payload"]["data"]["coordination_run_id"],
        FIRST_RUN
    );
    assert_eq!(
        claim_envelope["payload"]["data"]["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join(MARKER_PATH))
            .expect("coordination marker should read")
            .trim(),
        FIRST_RUN
    );
    let journal_before =
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should read before check");
    let projection_before = fs::read(repository.path().join(PROJECTION_PATH))
        .expect("projection should read before check");
    let lock_before =
        fs::read(repository.path().join(LOCK_PATH)).expect("lock should read before check");
    let empty_path = tempdir().expect("empty PATH directory should exist");
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");

    let own_check = run_check_without_git(repository.path(), empty_path.path(), Some(FIRST_RUN));
    let foreign_check = run_check_without_git(&second_root, empty_path.path(), None);

    assert!(
        own_check.status.success(),
        "own check failed: stdout={} stderr={}",
        String::from_utf8_lossy(&own_check.stdout),
        String::from_utf8_lossy(&own_check.stderr)
    );
    assert_eq!(foreign_check.status.code(), Some(1));
    assert_eq!(json_output(&foreign_check)["status"], "blocked_by_overlap");
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should reread"),
        journal_before
    );
    assert_eq!(
        fs::read(repository.path().join(PROJECTION_PATH)).expect("projection should reread"),
        projection_before
    );
    assert_eq!(
        fs::read(repository.path().join(LOCK_PATH)).expect("lock should reread"),
        lock_before
    );
}

#[test]
fn clear_check_creates_then_widens_one_exact_file_reservation() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);

    let first = run_berth(
        repository.path(),
        &["check", "tree:shared", "file:shared/child.rs", "--json"],
    );
    let first_envelope = json_output(&first);

    assert!(first.status.success());
    assert_eq!(first_envelope["status"], "clear");
    assert_eq!(
        first_envelope["payload"]["data"]["acquisition"]["kind"],
        "appended"
    );
    let first_acquisition = &first_envelope["payload"]["data"]["acquisition"];
    let coordination_run_id = first_acquisition["coordination_run_id"]
        .as_str()
        .expect("clear check should return its created coordination run");
    let reservation_id = first_acquisition["reservation_id"]
        .as_str()
        .expect("clear check should return its reservation");
    let phase_start_head = first_acquisition["phase_start_head"].clone();
    assert_eq!(
        first_acquisition["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        first_acquisition["session_mapping_publication"]["status"],
        "published"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join(MARKER_PATH))
            .expect("first-touch marker should read")
            .trim(),
        coordination_run_id
    );
    let first_events = journal_events(repository.path());
    assert_eq!(first_events.len(), 1);
    assert_first_touch_claim_event(&first_events[0], coordination_run_id);

    let second = run_berth_with_session(
        repository.path(),
        &["check", "tree:shared", "file:shared/child.rs", "--json"],
        "already-held-first-touch",
    );
    assert!(second.status.success());
    assert_eq!(
        json_output(&second)["payload"]["data"]["acquisition"]["kind"],
        "already_held"
    );
    let second_acquisition = json_output(&second)["payload"]["data"]["acquisition"].clone();
    assert_eq!(second_acquisition["reservation_id"], reservation_id);
    assert_eq!(
        second_acquisition["coordination_run_id"],
        coordination_run_id
    );
    assert_eq!(second_acquisition["phase_start_head"], phase_start_head);
    assert_eq!(
        second_acquisition["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        second_acquisition["session_mapping_publication"]["status"],
        "published"
    );
    assert_session_mapping(
        repository.path(),
        "already-held-first-touch",
        coordination_run_id,
        reservation_id,
    );
    assert_eq!(journal_events(repository.path()), first_events);

    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))
        .expect("reuse session mapping should remove");
    let widened = run_berth_with_session(
        repository.path(),
        &["check", "file:later.rs", "--json"],
        "widened-first-touch",
    );
    let widened_envelope = json_output(&widened);
    let widened_acquisition = &widened_envelope["payload"]["data"]["acquisition"];
    assert!(widened.status.success());
    assert_eq!(widened_acquisition["kind"], "widened");
    assert_eq!(widened_acquisition["reservation_id"], reservation_id);
    assert_eq!(
        widened_acquisition["coordination_run_id"],
        coordination_run_id
    );
    assert_eq!(widened_acquisition["phase_start_head"], phase_start_head);
    assert_eq!(
        widened_acquisition["session_mapping_publication"]["status"],
        "published"
    );
    let widened_events = journal_events(repository.path());
    assert_eq!(widened_events.len(), 2);
    assert_first_touch_widen_event(&widened_events[1], reservation_id);
    assert_session_mapping(
        repository.path(),
        "widened-first-touch",
        coordination_run_id,
        reservation_id,
    );

    assert_board_contains_every_claim_source(repository.path());
}

#[test]
fn session_mapped_reservation_survives_first_touch_and_receives_widen() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let session_id = "overlapping-claim-session";
    let reservations = claim_overlapping_reservations(repository.path(), session_id);

    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.newer,
    );
    let events_after_claims = journal_events(repository.path());
    assert_eq!(events_after_claims.len(), 2);
    assert_eq!(events_after_claims[0]["reservation_id"], reservations.older);
    assert_eq!(events_after_claims[1]["reservation_id"], reservations.newer);

    let already_held = run_berth_with_session(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
        session_id,
    );
    let already_held_envelope = json_output(&already_held);
    let already_held_acquisition = &already_held_envelope["payload"]["data"]["acquisition"];
    assert!(already_held.status.success());
    assert_eq!(already_held_envelope["status"], "clear");
    assert_eq!(already_held_acquisition["kind"], "already_held");
    assert_eq!(
        already_held_acquisition["reservation_id"],
        reservations.newer
    );
    assert_eq!(journal_events(repository.path()), events_after_claims);
    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.newer,
    );

    let widened = run_berth_with_session(
        repository.path(),
        &["check", "file:later.rs", "--json"],
        session_id,
    );
    let widened_envelope = json_output(&widened);
    let widened_acquisition = &widened_envelope["payload"]["data"]["acquisition"];
    assert!(widened.status.success());
    assert_eq!(widened_acquisition["kind"], "widened");
    assert_eq!(widened_acquisition["reservation_id"], reservations.newer);
    let widened_events = journal_events(repository.path());
    assert_eq!(widened_events.len(), 3);
    assert_first_touch_widen_event(&widened_events[2], &reservations.newer);
    assert!(
        !widened_events.iter().any(|event| {
            event["op"] == "widen" && event["reservation_id"] == reservations.older
        })
    );
    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.newer,
    );
}

#[test]
fn missing_mapping_with_two_eligible_reservations_reports_ambiguity_without_mutation() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let session_id = "ambiguous-overlapping-claim-session";
    let reservations = claim_overlapping_reservations(repository.path(), session_id);
    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.newer,
    );
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))
        .expect("session mapping should be removed before ambiguous first touch");
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before ambiguous first touch");
    let projection_before = fs::read(repository.path().join(PROJECTION_PATH))
        .expect("projection should read before ambiguous first touch");

    let ambiguous = run_berth_with_session(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
        session_id,
    );
    let envelope = json_output(&ambiguous);
    let mut expected_candidate_ids = vec![reservations.older, reservations.newer];
    expected_candidate_ids.sort();
    let expected_candidates = serde_json::json!(expected_candidate_ids);

    assert_eq!(ambiguous.status.code(), Some(1));
    assert_eq!(envelope["exit_code"], 1);
    assert_eq!(envelope["status"], "ambiguous_active_run_reservations");
    assert_eq!(
        envelope["payload"]["kind"],
        "first_touch_reservation_selection"
    );
    assert_eq!(
        envelope["payload"]["data"]["status"],
        "ambiguous_active_run_reservations"
    );
    assert_eq!(
        envelope["payload"]["data"]["candidate_reservation_ids"],
        expected_candidates
    );
    assert_eq!(envelope["reservations"], expected_candidates);
    assert_eq!(envelope["blocked_by"], serde_json::json!([]));
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should reread after ambiguous first touch"),
        journal_before
    );
    assert_eq!(
        fs::read(repository.path().join(PROJECTION_PATH))
            .expect("projection should reread after ambiguous first touch"),
        projection_before
    );
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());
}

#[test]
fn explicit_check_selection_republishes_mapping_for_subsequent_checks() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let session_id = "explicit-overlapping-selection-session";
    let reservations = claim_overlapping_reservations(repository.path(), session_id);
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))
        .expect("session mapping should be removed before explicit selection");
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before explicit selection");
    let projection_before = fs::read(repository.path().join(PROJECTION_PATH))
        .expect("projection should read before explicit selection");

    let selected = run_berth_with_session(
        repository.path(),
        &[
            "check",
            "file:shared/child.rs",
            "--reservation",
            reservations.older.as_str(),
            "--json",
        ],
        session_id,
    );
    let selected_envelope = json_output(&selected);
    let selected_acquisition = &selected_envelope["payload"]["data"]["acquisition"];

    assert!(selected.status.success());
    assert_eq!(selected_envelope["status"], "clear");
    assert_eq!(selected_acquisition["kind"], "already_held");
    assert_eq!(selected_acquisition["reservation_id"], reservations.older);
    assert_eq!(
        selected_acquisition["session_mapping_publication"]["status"],
        "published"
    );
    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.older,
    );

    let ordinary = run_berth_with_session(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
        session_id,
    );
    let ordinary_envelope = json_output(&ordinary);
    let ordinary_acquisition = &ordinary_envelope["payload"]["data"]["acquisition"];

    assert!(ordinary.status.success());
    assert_eq!(ordinary_envelope["status"], "clear");
    assert_eq!(ordinary_acquisition["kind"], "already_held");
    assert_eq!(ordinary_acquisition["reservation_id"], reservations.older);
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should reread after selected and ordinary checks"),
        journal_before
    );
    assert_eq!(
        fs::read(repository.path().join(PROJECTION_PATH))
            .expect("projection should reread after selected and ordinary checks"),
        projection_before
    );
    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.older,
    );
}

#[test]
fn explicit_check_without_harness_session_reports_invocation_only_selection() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let reservations =
        claim_overlapping_reservations(repository.path(), "explicit-selection-setup-session");
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))
        .expect("setup session mapping should be removed before explicit selection");

    let selected = run_berth(
        repository.path(),
        &[
            "check",
            "file:shared/child.rs",
            "--reservation",
            reservations.older.as_str(),
            "--json",
        ],
    );
    let selected_envelope = json_output(&selected);
    let selected_acquisition = &selected_envelope["payload"]["data"]["acquisition"];
    let expected_session_mapping_publication = serde_json::json!({
        "status": "explicit_selection_applies_only_to_current_invocation",
        "reason": "harness_session_unavailable"
    });

    assert!(selected.status.success());
    assert_eq!(selected_envelope["status"], "clear");
    assert_eq!(selected_acquisition["kind"], "already_held");
    assert_eq!(selected_acquisition["reservation_id"], reservations.older);
    assert_eq!(
        selected_acquisition["session_mapping_publication"],
        expected_session_mapping_publication
    );
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());

    let widened = run_berth(
        repository.path(),
        &[
            "check",
            "file:later.rs",
            "--reservation",
            reservations.older.as_str(),
            "--json",
        ],
    );
    let widened_envelope = json_output(&widened);
    let widened_acquisition = &widened_envelope["payload"]["data"]["acquisition"];
    assert!(widened.status.success());
    assert_eq!(widened_acquisition["kind"], "widened");
    assert_eq!(widened_acquisition["reservation_id"], reservations.older);
    assert_eq!(
        widened_acquisition["session_mapping_publication"],
        expected_session_mapping_publication
    );
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());

    let ordinary = run_berth(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
    );
    let ordinary_envelope = json_output(&ordinary);
    assert_eq!(ordinary.status.code(), Some(1));
    assert_eq!(
        ordinary_envelope["status"],
        "ambiguous_active_run_reservations"
    );
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());
}

#[test]
fn explicit_check_selection_rejects_a_foreign_reservation_without_mutation() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let session_id = "explicit-foreign-selection-session";
    let own_claim = run_berth_with_session(
        repository.path(),
        &["claim", "tree:owned", "--run", FIRST_RUN, "--json"],
        session_id,
    );
    assert!(own_claim.status.success());

    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "foreign-selection");
    let foreign_claim = run_berth_with_session(
        &foreign_root,
        &["claim", "tree:foreign", "--run", SECOND_RUN, "--json"],
        "foreign-selection-session",
    );
    assert!(foreign_claim.status.success());
    let foreign_reservation_id = json_output(&foreign_claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("foreign claim should report its reservation")
        .to_owned();
    fs::remove_file(repository.path().join(SESSION_MAPPING_PATH))
        .expect("session mappings should be removed before foreign selection");
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before foreign selection");
    let projection_before = fs::read(repository.path().join(PROJECTION_PATH))
        .expect("projection should read before foreign selection");

    let rejected = run_berth_with_session(
        repository.path(),
        &[
            "check",
            "file:unclaimed.rs",
            "--reservation",
            foreign_reservation_id.as_str(),
            "--json",
        ],
        session_id,
    );
    let rejected_envelope = json_output(&rejected);

    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(rejected_envelope["exit_code"], 5);
    assert_eq!(rejected_envelope["status"], "invalid_input");
    assert_eq!(rejected_envelope["payload"]["kind"], "no_facts");
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should reread after foreign selection"),
        journal_before
    );
    assert_eq!(
        fs::read(repository.path().join(PROJECTION_PATH))
            .expect("projection should reread after foreign selection"),
        projection_before
    );
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());
}

#[test]
fn engine_envelope_carries_output_contract_version() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let check = run_berth(
        repository.path(),
        &["check", "file:engine-version.rs", "--json"],
    );
    let envelope = json_output(&check);
    let contract = checked_output_contract();

    assert!(check.status.success());
    assert_eq!(
        envelope["output_contract_version"], contract["version"],
        "an engine-produced envelope should report OUTPUT_CONTRACT_VERSION"
    );
}

fn checked_output_contract() -> serde_json::Value {
    let serialized_contract =
        include_str!("../../../docs/cargo-berth/generated/output-contract.json");
    serde_json::from_str(serialized_contract).expect("checked-in output contract should decode")
}
fn assert_first_touch_claim_event(event: &serde_json::Value, coordination_run_id: &str) {
    assert_eq!(event["schema_version"], 2);
    assert_eq!(event["op"], "claim");
    assert_eq!(event["actor"]["run"], coordination_run_id);
    assert_eq!(event["source"]["kind"], "first_touch");
    assert_eq!(event["scopes"][0]["kind"], "file");
    assert_eq!(event["scopes"][0]["path"], "shared");
    assert_eq!(event["scopes"][1]["kind"], "file");
    assert_eq!(event["scopes"][1]["path"], "shared/child.rs");
}

fn assert_first_touch_widen_event(event: &serde_json::Value, reservation_id: &str) {
    assert_eq!(event["op"], "widen");
    assert_eq!(event["reservation_id"], reservation_id);
    assert_eq!(event["added_scopes"][0]["kind"], "file");
    assert_eq!(event["added_scopes"][0]["path"], "later.rs");
}

fn assert_board_contains_every_claim_source(repository_root: &Path) {
    assert!(
        run_berth(repository_root, &["claim", "file:explicit.rs", "--json"])
            .status
            .success()
    );
    assert!(
        run_berth(
            repository_root,
            &[
                "claim",
                "file:planned.rs",
                "--plan",
                "docs/plan.md",
                "--phase",
                "planned-phase",
                "--json",
            ]
        )
        .status
        .success()
    );
    let board = json_output(&run_berth(repository_root, &["board", "--json"]));
    let source_kinds = ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| {
            board["payload"]["data"][section]["entries"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .map(|reservation| reservation["source"]["kind"].clone())
        .collect::<Vec<_>>();
    assert!(source_kinds.contains(&serde_json::json!("first_touch")));
    assert!(source_kinds.contains(&serde_json::json!("explicit")));
    assert!(source_kinds.contains(&serde_json::json!("work_plan")));
}

#[test]
fn blocked_check_returns_holder_decision_facts_without_appending() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let holder = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:shared.rs", "--json"])
        .current_dir(repository.path())
        .env(RUN_ENVIRONMENT, FIRST_RUN)
        .output()
        .expect("first-touch holder check should run");
    assert!(holder.status.success());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before blocked check");

    let blocked = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:shared.rs", "--json"])
        .current_dir(&second_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("foreign check should run");
    let envelope = json_output(&blocked);
    let conflict = &envelope["payload"]["data"]["conflicts"][0];

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(envelope["status"], "blocked_by_overlap");
    assert_eq!(conflict["holder_run_id"], FIRST_RUN);
    assert_eq!(conflict["head_snapshot"]["full_ref"], "refs/heads/main");
    assert_eq!(
        conflict["source"],
        serde_json::json!({ "kind": "first_touch" })
    );
    assert!(conflict["source"].get("phase").is_none());
    assert!(conflict["claimed_at"].is_string());
    assert_eq!(conflict["activity"]["status"], "active");
    assert!(conflict["activity"]["last_activity_at"].is_string());
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should reread after blocked check"),
        journal_before
    );
}

#[test]
fn concurrent_first_touch_checks_choose_one_holder_under_the_mutation_lock() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let mutation_lock =
        File::open(repository.path().join(LOCK_PATH)).expect("mutation lock should open");
    mutation_lock
        .lock()
        .expect("test should hold mutation lock");
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let first_ready_path = repository.path().join("first-lock-ready");
    let second_ready_path = repository.path().join("second-lock-ready");
    let first = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:raced.rs", "--json"])
        .current_dir(repository.path())
        .env(RUN_ENVIRONMENT, FIRST_RUN)
        .env(MUTATION_LOCK_READY_PATH_ENVIRONMENT, &first_ready_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first concurrent check should start");
    let second = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:raced.rs", "--json"])
        .current_dir(&second_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(MUTATION_LOCK_READY_PATH_ENVIRONMENT, &second_ready_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("second concurrent check should start");

    wait_for_lock_contenders(&[&first_ready_path, &second_ready_path]);
    mutation_lock
        .unlock()
        .expect("test should release both waiting checks");
    let first = first
        .wait_with_output()
        .expect("first concurrent check should finish");
    let second = second
        .wait_with_output()
        .expect("second concurrent check should finish");
    let outcomes = [first, second];
    let status_codes = outcomes
        .iter()
        .map(|output| output.status.code())
        .collect::<Vec<_>>();

    assert_eq!(
        outcomes
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1,
        "concurrent check status codes: {status_codes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|output| output.status.code() == Some(1))
            .count(),
        1,
        "concurrent check status codes: {status_codes:?}"
    );
    let events = journal_events(repository.path());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["op"], "claim");
    assert_eq!(events[0]["source"]["kind"], "first_touch");
    assert_eq!(events[0]["scopes"][0]["path"], "raced.rs");
    let refused = outcomes
        .iter()
        .find(|output| !output.status.success())
        .expect("one concurrent check should be refused");
    assert_eq!(json_output(refused)["status"], "blocked_by_overlap");
}

#[test]
fn check_reports_unconfigured_when_an_initialized_repository_loses_its_configuration() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let expected_configuration_path = repository.path().join(CONFIGURATION_PATH);
    fs::remove_file(&expected_configuration_path).expect("configuration should be removed");

    let check = run_berth(
        repository.path(),
        &["check", "file:unreserved.rs", "--json"],
    );
    let envelope = json_output(&check);

    assert_eq!(check.status.code(), Some(4));
    assert_eq!(envelope["exit_code"], 4);
    assert_eq!(envelope["status"], "unconfigured");
    assert_eq!(envelope["payload"]["kind"], "no_facts");
    assert!(envelope["message"].as_str().is_some_and(|message| {
        message.contains(&expected_configuration_path.display().to_string())
    }));
}

#[test]
fn check_does_not_replay_a_foreign_conflict_after_configuration_is_removed() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let claim = run_berth(
        repository.path(),
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(claim.status.success());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    fs::remove_file(second_root.join(CONFIGURATION_PATH)).expect("configuration should be removed");

    let check = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(&second_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("cargo-berth check should run");
    let envelope = json_output(&check);

    assert_eq!(check.status.code(), Some(4));
    assert_eq!(envelope["status"], "unconfigured");
    assert_eq!(envelope["payload"]["kind"], "no_facts");
    assert_ne!(envelope["status"], "blocked_by_overlap");
}

#[test]
fn check_reports_malformed_present_configuration_as_ledger_unreadable() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    fs::write(
        repository.path().join(CONFIGURATION_PATH),
        "porthole = true\n",
    )
    .expect("malformed configuration should write");

    let check = run_berth(
        repository.path(),
        &["check", "file:unreserved.rs", "--json"],
    );
    let envelope = json_output(&check);

    assert_eq!(check.status.code(), Some(4));
    assert_eq!(envelope["status"], "ledger_unreadable");
    assert_eq!(envelope["payload"]["kind"], "no_facts");
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("ledger configuration failed: "))
    );
}

#[test]
fn claims_without_run_continue_the_worktree_coordination_run() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let first_claim = run_berth(repository.path(), &["claim", "tree:crates/a", "--json"]);
    assert!(first_claim.status.success());
    let first_reservation_id = json_output(&first_claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("first claim should report its reservation")
        .to_owned();
    let first_run = fs::read_to_string(repository.path().join(MARKER_PATH))
        .expect("first coordination marker should read");

    let second_claim = run_berth(repository.path(), &["claim", "tree:crates/b", "--json"]);
    assert!(second_claim.status.success());
    let second_reservation_id = json_output(&second_claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("second claim should report its reservation")
        .to_owned();
    assert_eq!(
        fs::read_to_string(repository.path().join(MARKER_PATH))
            .expect("second coordination marker should read"),
        first_run
    );

    let check = run_berth(
        repository.path(),
        &["check", "file:crates/a/x.rs", "--json"],
    );
    let envelope = json_output(&check);
    let mut expected_candidate_ids = vec![first_reservation_id, second_reservation_id];
    expected_candidate_ids.sort();
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(envelope["status"], "ambiguous_active_run_reservations");
    assert_eq!(
        envelope["payload"]["data"]["candidate_reservation_ids"],
        serde_json::json!(expected_candidate_ids)
    );
}

#[test]
fn reconciliation_removes_a_malformed_marker_directory_before_claim() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    fs::create_dir(repository.path().join(MARKER_PATH))
        .expect("marker destination directory should exist");

    let claim = run_berth(
        repository.path(),
        &["claim", "file:src/lib.rs", "--run", FIRST_RUN, "--json"],
    );
    let envelope = json_output(&claim);

    assert!(claim.status.success());
    assert_eq!(envelope["status"], "claimed");
    assert_eq!(
        envelope["payload"]["data"]["coordination_run_id"],
        FIRST_RUN
    );
    assert_eq!(
        envelope["payload"]["data"]["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join(JOURNAL_PATH))
            .expect("journal should contain the committed claim")
            .lines()
            .count(),
        1
    );
}

#[test]
fn blocked_message_names_every_holder() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    for scope in ["tree:src", "file:src/lib.rs"] {
        assert!(
            run_berth(
                repository.path(),
                &["claim", scope, "--run", FIRST_RUN, "--json"]
            )
            .status
            .success()
        );
    }

    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let blocked = run_berth(&second_root, &["claim", "file:src/lib.rs", "--json"]);

    assert_eq!(blocked.status.code(), Some(1));
    let envelope = json_output(&blocked);
    let message = envelope["message"]
        .as_str()
        .expect("blocked message should be text");
    let conflicts = envelope["payload"]["data"]["conflicts"]
        .as_array()
        .expect("blocked payload should list conflicts");
    assert_eq!(conflicts.len(), 2);
    assert!(message.contains("2 reservations hold overlapping paths"));
    for conflict in conflicts {
        let reservation_id = conflict["reservation_id"]
            .as_str()
            .expect("conflict should name its reservation");
        assert!(message.contains(reservation_id));
    }
}

#[test]
fn a_first_touch_holder_block_names_the_verbs_that_clear_it() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let first_touch = run_berth(repository.path(), &["check", "file:touched.rs", "--json"]);
    assert!(first_touch.status.success());
    let reservation_id =
        json_output(&first_touch)["payload"]["data"]["acquisition"]["reservation_id"]
            .as_str()
            .expect("a clear check should create a first-touch reservation")
            .to_owned();

    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let blocked = run_berth(&second_root, &["claim", "file:touched.rs", "--json"]);

    assert_eq!(blocked.status.code(), Some(1));
    let envelope = json_output(&blocked);
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["kind"],
        "first_touch"
    );
    let message = envelope["message"]
        .as_str()
        .expect("blocked message should be text");
    assert!(message.contains(&format!("cargo-berth release {reservation_id}")));
    assert!(message.contains(&format!(
        "cargo-berth resolve {reservation_id} --integrated-as"
    )));
    assert!(message.contains(&format!(
        "cargo-berth resolve {reservation_id} --abandon --why"
    )));
}

#[test]
fn future_paths_succeed_invalid_paths_do_not_append_and_missing_why_is_typed() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    let future_claim = run_berth(
        repository.path(),
        &[
            "claim",
            "file:future/generated.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
    );
    assert!(future_claim.status.success());
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should read after future claim");
    let journal_event: serde_json::Value =
        serde_json::from_str(journal.lines().next().expect("claim record should exist"))
            .expect("claim record should decode");
    assert_eq!(journal_event["purpose"]["kind"], "not_provided_by_caller");
    let journal_before_rejections = journal.into_bytes();

    for invalid_path in ["file:../outside.rs", "file:/absolute.rs"] {
        let rejected = run_berth(
            repository.path(),
            &["claim", invalid_path, "--run", SECOND_RUN, "--json"],
        );
        assert_eq!(rejected.status.code(), Some(5));
        assert!(
            json_output(&rejected)["message"]
                .as_str()
                .is_some_and(|message| message.contains("repository-relative path"))
        );
    }
    assert_eq!(
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal should reread"),
        journal_before_rejections
    );
}

#[test]
fn unpaired_plan_flags_are_usage_errors_and_rejection_sweeps_stale_marker() {
    let repository = initialized_repository(PathCaseSetting::Sensitive);
    for arguments in [
        vec!["claim", "file:a", "--plan", "docs/plan.md"],
        vec!["claim", "file:a", "--phase", "phase-a"],
    ] {
        assert_eq!(
            run_berth(repository.path(), &arguments).status.code(),
            Some(5)
        );
    }
    assert!(
        run_berth(
            repository.path(),
            &["claim", "file:Cargo.toml", "--run", FIRST_RUN, "--json"]
        )
        .status
        .success()
    );
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    fs::write(
        repository.path().join(MARKER_PATH),
        format!("{THIRD_RUN}\n"),
    )
    .expect("newer marker should write");

    let rejected = run_berth(&second_root, &["claim", "file:Cargo.toml", "--json"]);

    assert_eq!(rejected.status.code(), Some(1));
    assert!(
        !repository.path().join(MARKER_PATH).exists(),
        "reconciliation should sweep a marker without a matching active reservation"
    );
}

struct OverlappingReservationIds {
    newer: String,
    older: String,
}

fn claim_overlapping_reservations(
    repository_root: &Path,
    session_id: &str,
) -> OverlappingReservationIds {
    let older_claim = run_berth_with_session(
        repository_root,
        &["claim", "tree:shared", "--run", FIRST_RUN, "--json"],
        session_id,
    );
    assert!(
        older_claim.status.success(),
        "older claim failed: {}",
        String::from_utf8_lossy(&older_claim.stdout)
    );
    let older = json_output(&older_claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("older claim should report its reservation")
        .to_owned();

    let newer_claim = run_berth_with_session(
        repository_root,
        &[
            "claim",
            "file:shared/child.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
        session_id,
    );
    assert!(
        newer_claim.status.success(),
        "newer claim failed: {}",
        String::from_utf8_lossy(&newer_claim.stdout)
    );
    let newer = json_output(&newer_claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("newer claim should report its reservation")
        .to_owned();
    assert_ne!(newer, older);

    OverlappingReservationIds { newer, older }
}

#[derive(Clone, Copy)]
enum PathCaseSetting {
    Sensitive,
    Insensitive,
}

impl PathCaseSetting {
    const fn git_value(self) -> &'static str {
        match self {
            Self::Sensitive => "false",
            Self::Insensitive => "true",
        }
    }
}

fn initialized_repository(path_case_setting: PathCaseSetting) -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    git(
        repository.path(),
        &["config", "user.email", "berth@example.invalid"],
    );
    git(
        repository.path(),
        &["config", "core.ignoreCase", path_case_setting.git_value()],
    );
    fs::write(repository.path().join("README.md"), "scratch repository\n")
        .expect("scratch file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth(repository.path(), &["init", "--json"]);
    assert!(initialized.status.success());
    repository
}

/// Add a real worktree beside the repository, the second party berth has always refused.
///
/// A distinct `--run` inside one worktree now names a second party too, but only a real
/// worktree can hold a reservation of its own alongside another run's. The returned
/// directory owns the worktree and must outlive its use.
fn foreign_worktree(repository: &TempDir, name: &str) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("foreign worktree parent should exist");
    let root = directory.path().join(name);
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            name,
            root.to_str()
                .expect("foreign worktree path should be UTF-8"),
        ],
    );
    let configuration = root.join(CONFIGURATION_PATH);
    if let Some(parent) = configuration.parent() {
        fs::create_dir_all(parent).expect("foreign worktree configuration should have a directory");
    }
    fs::copy(repository.path().join(CONFIGURATION_PATH), configuration)
        .expect("foreign worktree should share the repository configuration");
    (directory, root)
}

fn run_check_without_git(worktree_root: &Path, path: &Path, run: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-berth"));
    command
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(worktree_root)
        .env("PATH", path);
    match run {
        Some(run) => command.env(RUN_ENVIRONMENT, run),
        None => command.env_remove(RUN_ENVIRONMENT),
    };
    command
        .output()
        .expect("cargo-berth check should run without git")
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run with a harness session")
}

fn assert_session_mapping(
    repository_root: &Path,
    session_id: &str,
    coordination_run_id: &str,
    reservation_id: &str,
) {
    let mapping: serde_json::Value = serde_json::from_slice(
        &fs::read(repository_root.join(SESSION_MAPPING_PATH)).expect("session mapping should read"),
    )
    .expect("session mapping should decode");
    let identity = &mapping["identities"][session_id];
    assert_eq!(identity["coordination_run_id"], coordination_run_id);
    assert_eq!(identity["reservation_id"], reservation_id);
}

fn wait_for_lock_contenders(ready_paths: &[&Path]) {
    let deadline = Instant::now() + READY_WAIT_TIMEOUT;
    while ready_paths.iter().any(|path| !path.is_file()) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for both checks to contend in MutationLock::acquire"
        );
        thread::sleep(READY_WAIT_INTERVAL);
    }
}

fn journal_events(repository_root: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .collect()
}

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}
