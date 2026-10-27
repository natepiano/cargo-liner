#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for claim acquisition, edit checks and refusal auditing.

#[path = "support/overlap_refusal.rs"]
mod overlap_refusal;

use cargo_berth_test_support::CLAUDE_CODE_SESSION_ENVIRONMENT;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::board_reservation_ids;
use cargo_berth_test_support::is_merge_extent_observation;
use cargo_berth_test_support::json;
use cargo_berth_test_support::reservation_row;
use overlap_refusal::assert_one_refusal_appended;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: an ordinary checkout, with nothing held back from a hook.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

/// An initialized main checkout with untracked policy and no reservations.
const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "initialized",
    build:            build_initialized_repository,
};

/// Initialized main with committed policy for integration-target scenarios.
const INTEGRATION_MAIN: RepositoryTemplate = RepositoryTemplate {
    name: "integration-main",
    build: |root| IntegrationRepository::build_main_template(root, BERTH_EXECUTABLE),
    ..INITIALIZED_REPOSITORY
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
const TREE_CLAIM_HOLDER_COUNT: usize = 124;
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

#[test]
fn a_tree_claim_records_many_holder_answers_without_a_size_refusal() {
    let repository = initialized_repository();
    let root = repository.path();
    let own = run_berth(
        root,
        &["claim", "file:own.rs", "--run", FIRST_RUN, "--json"],
    );
    assert_success(&own);
    let own_id = json(&own)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("own reservation")
        .to_owned();
    let (_directory, foreign) = foreign_worktree(&repository, "many-holders");
    make_dirty(&foreign, "crates/hana/tests/holder_0.rs");
    let holder = run_berth(
        &foreign,
        &[
            "claim",
            "file:crates/hana/tests/holder_0.rs",
            "--run",
            SECOND_RUN,
            "--json",
        ],
    );
    assert_success(&holder);
    let holder_id = json(&holder)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("holder reservation")
        .to_owned();
    let journal = seed_unavailable_tree_holders(root, &holder_id);
    let configuration = root.join(CONFIGURATION_PATH);
    let configured = fs::read_to_string(&configuration).expect("configuration");
    fs::write(
        &configuration,
        format!("default_answer = \"first_ready\"\n{configured}"),
    )
    .expect("default answers should configure");

    let claimed = run_berth(
        root,
        &[
            "claim",
            "--reservation",
            &own_id,
            "tree:crates/hana/tests",
            "--run",
            FIRST_RUN,
            "--json",
        ],
    );
    assert_success(&claimed);
    let after = fs::read_to_string(root.join(JOURNAL_PATH)).expect("answers should read");
    let answer: serde_json::Value = after
        .strip_prefix(&journal)
        .expect("existing records remain unchanged")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("appended fact"))
        .find(|event| event["op"] == "answer")
        .expect("one answer record");
    assert_eq!(
        answer["scopes"],
        serde_json::json!([{"kind":"tree", "path":"crates/hana/tests"}])
    );
    assert_eq!(
        answer["authorizations"]
            .as_array()
            .expect("holder answers")
            .len(),
        TREE_CLAIM_HOLDER_COUNT,
        "every holder must receive its answer: {answer}"
    );
    assert!(serde_json::to_vec(&answer).expect("encoded answers").len() > 16 * 1024);
}

#[test]
fn check_uses_the_current_runs_target_when_an_older_run_is_still_outstanding() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    let lane = repo.lane("two-run-lane", "integration");
    repo.git(&lane, &["reset", "--hard", "main"]);
    let older = repo.claim(&lane, "file:older.txt", FIRST_RUN, Some("main"));
    assert_success(&older);
    let older_id = json(&older)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("older reservation ID")
        .to_owned();
    repo.commit_file(&lane, "older.txt", "older\n", "older work");
    repo.commit_file(
        &lane,
        "integration.txt",
        "lane version\n",
        "lane changes target path",
    );
    let checkpoint = repo.run(&lane, &["release", &older_id, "--json"]);
    assert_success(&checkpoint);
    let current = repo.claim(&lane, "file:current.txt", SECOND_RUN, None);
    assert_success(&current);
    let _ = repo.board();
    let cover_id = repo.cover_claims()[0]["reservation_id"]
        .as_str()
        .expect("cover reservation ID")
        .to_owned();

    let check = repo.run(&lane, &["check", "file:integration.txt", "--json"]);
    assert_success(&check);
    assert_eq!(json(&check)["status"], "clear");
    assert!(
        !String::from_utf8_lossy(&check.stdout).contains(&cover_id),
        "current run already contains the cover's committed work"
    );
}

#[test]
fn sibling_lane_can_edit_landed_work_while_main_meets_the_cover() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    let a = repo.lane("contained-a", "integration");
    let b = repo.lane("contained-b", "integration");
    let a_claim = repo.claim(&a, "file:shared.txt", FIRST_RUN, None);
    assert_success(&a_claim);
    let b_claim = repo.claim(&b, "file:b.txt", SECOND_RUN, None);
    assert_success(&b_claim);
    let _ = repo.board();
    repo.commit_file(&a, "shared.txt", "landed\n", "lane A work");
    repo.merge_by_commit("contained-a");
    let observed = repo.board();
    let a_id = json(&a_claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane A ID")
        .to_owned();
    assert_eq!(
        reservation_row(&observed, &a_id)["lifecycle"]["stage"],
        "released",
        "{observed}"
    );

    let sibling = repo.run(&b, &["check", "file:shared.txt", "--json"]);
    assert_success(&sibling);
    assert_eq!(json(&sibling)["status"], "clear");

    let main = repo.run(repo.root(), &["check", "file:shared.txt", "--json"]);
    assert_eq!(json(&main)["status"], "blocked_by_overlap");
    let cover_id = repo.cover_claims()[0]["reservation_id"]
        .as_str()
        .expect("cover ID")
        .to_owned();
    assert!(
        String::from_utf8_lossy(&main.stdout).contains(&cover_id),
        "main meets the integration cover"
    );
}

#[test]
fn lane_editing_its_target_diff_records_no_cover_incursion() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    repo.commit_file(
        &repo.integration,
        "Cargo.toml",
        "[package]\nname = \"integration\"\nversion = \"0.1.0\"\n",
        "integration manifest",
    );
    let lane = repo.lane("target-diff-lane", "integration");
    let claimed = repo.claim(&lane, "file:lane.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let _ = repo.board();
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane ID")
        .to_owned();
    let before = repo
        .journal()
        .into_iter()
        .filter(|event| event["op"] == "incursion")
        .count();
    fs::write(
        lane.join("Cargo.toml"),
        "[package]\nname = \"lane\"\nversion = \"0.1.0\"\n",
    )
    .expect("lane manifest edits");
    let drift = repo.run(&lane, &["drift", "--full", "--reservation", &id, "--json"]);
    assert_success(&drift);
    let after = repo
        .journal()
        .into_iter()
        .filter(|event| event["op"] == "incursion")
        .count();
    assert_eq!(
        after, before,
        "target cover does not protect against its own lane"
    );
}

#[test]
fn claim_target_precedence_and_source_are_recorded() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    let explicit_lane = repo.lane("explicit-target-lane", "integration");
    let explicit = repo.claim(
        &explicit_lane,
        "file:explicit.txt",
        FIRST_RUN,
        Some("refs/heads/main"),
    );
    assert_success(&explicit);
    let explicit_payload = json(&explicit);
    assert_eq!(
        explicit_payload["payload"]["data"]["target"]["ref"],
        "refs/heads/main"
    );
    assert_eq!(
        explicit_payload["payload"]["data"]["target"]["source"],
        "claim_argument"
    );

    let configured_lane = repo.lane("configured-target-lane", "integration");
    let configured = repo.claim(&configured_lane, "file:configured.txt", SECOND_RUN, None);
    assert_success(&configured);
    let configured_payload = json(&configured);
    assert_eq!(
        configured_payload["payload"]["data"]["target"]["ref"],
        "refs/heads/integration"
    );
    assert_eq!(
        configured_payload["payload"]["data"]["target"]["source"],
        "branch_configuration"
    );
    assert_eq!(
        configured_payload["payload"]["data"]["target"]["commit"],
        repo.git_stdout(repo.root(), &["rev-parse", "integration"])
    );

    repo.git(&repo.integration, &["switch", "--detach", "--quiet"]);
    let detached = repo.claim(&repo.integration, "file:detached.txt", THIRD_RUN, None);
    assert_success(&detached);
    let detached_payload = json(&detached);
    assert_eq!(
        detached_payload["payload"]["data"]["target"]["ref"],
        "refs/heads/main"
    );
    assert_eq!(
        detached_payload["payload"]["data"]["target"]["source"],
        "repository_trunk"
    );

    let claims: Vec<_> = repo
        .journal()
        .into_iter()
        .filter(|event| event["op"] == "claim")
        .collect();
    assert_eq!(claims.len(), 3);
    for (claim, target, source) in [
        (&claims[0], "refs/heads/main", "claim_argument"),
        (&claims[1], "refs/heads/integration", "branch_configuration"),
        (&claims[2], "refs/heads/main", "repository_trunk"),
    ] {
        assert_eq!(claim["target"]["target"], target);
        assert_eq!(claim["target"]["source"], source);
    }
}

#[test]
fn explicit_claim_refuses_own_and_unresolved_targets() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    let lane = repo.lane("refused-target-lane", "integration");
    for (target, reason) in [
        ("refused-target-lane", "own branch"),
        ("no-such-target", "resolve"),
    ] {
        let output = repo.claim(&lane, "file:refused.txt", FIRST_RUN, Some(target));
        assert!(!output.status.success());
        let response = json(&output);
        assert_eq!(response["status"], "invalid_input");
        assert!(String::from_utf8_lossy(&output.stdout).contains(target));
        assert!(String::from_utf8_lossy(&output.stdout).contains(reason));
    }
    assert!(repo.journal().iter().all(|event| event["op"] != "claim"));
}

#[test]
fn explicit_claim_distinguishes_a_nonbranch_target_from_an_unresolved_branch() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    let lane = repo.lane("malformed-claim-lane", "integration");
    for (target, message) in [
        (
            "refs/tags/v1",
            "target `refs/tags/v1` is not a local branch",
        ),
        (
            "bad..branch",
            "target `bad..branch` is not a valid local branch",
        ),
        ("nope", "target `nope` does not resolve to a local branch"),
    ] {
        let output = repo.claim(&lane, "file:claim-target.txt", FIRST_RUN, Some(target));
        assert!(!output.status.success());
        let response = json(&output);
        assert_eq!(response["status"], "invalid_input");
        assert_eq!(response["message"], message, "target argument: {target}");
    }
}

#[test]
fn first_touch_invalid_branch_settings_fall_back_to_repository_trunk() {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    for (branch, configured_target, reason) in [
        ("own-target-lane", "own-target-lane", "own_branch"),
        ("missing-target-lane", "no-such-target", "unresolved"),
    ] {
        let lane = repo.lane(branch, configured_target);
        let output = repo.run(&lane, &["check", &format!("file:{branch}.txt"), "--json"]);
        assert_success(&output);
        let payload = json(&output);
        let acquired_target = &payload["payload"]["data"]["acquisition"]["target"];
        assert_eq!(acquired_target["ref"], "refs/heads/main");
        assert_eq!(acquired_target["source"], "repository_trunk");
        assert_eq!(acquired_target["fallback"]["requested"], configured_target);
        assert_eq!(acquired_target["fallback"]["reason"], reason);
        let claims: Vec<_> = repo
            .journal()
            .into_iter()
            .filter(|event| {
                event["op"] == "claim"
                    && event["head_snapshot"]["full_ref"] == format!("refs/heads/{branch}")
            })
            .collect();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0]["source"]["kind"], "first_touch");
        assert_eq!(claims[0]["target"]["target"], "refs/heads/main");
        assert_eq!(claims[0]["target"]["source"], "repository_trunk");
        assert_eq!(
            claims[0]["target"]["fallback"]["requested"],
            configured_target
        );
        assert_eq!(claims[0]["target"]["fallback"]["reason"], reason);
    }
}

#[test]
fn a_refused_claim_journals_its_requester_paths_and_holders() {
    let repository = initialized_repository();
    let holder = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:crates/hana_kana",
            "--run",
            FIRST_RUN,
            "--json",
        ],
    );
    assert_success(&holder);
    let holder_id = json_output(&holder)["payload"]["data"]["reservation_id"].clone();
    let holder_record = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "claim" && event["reservation_id"] == holder_id)
        .expect("holder claim record");
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    make_dirty(repository.path(), "crates/hana_kana/src/lib.rs");
    let before_board_output = run_berth(repository.path(), &["board", "--json"]);
    assert_success(&before_board_output);
    let before_board = json_output(&before_board_output);
    let journal_before =
        fs::read(repository.path().join(JOURNAL_PATH)).expect("journal before refusal");

    let blocked = run_berth(
        &second_root,
        &[
            "claim",
            "file:crates/hana_kana/src/lib.rs",
            "--run",
            SECOND_RUN,
            "--json",
        ],
    );
    let envelope = json_output(&blocked);
    assert_eq!(blocked.status.code(), Some(1), "{envelope}");
    assert_eq!(envelope["status"], "blocked_by_overlap");
    let record = assert_one_refusal_appended(
        &journal_before,
        &fs::read(repository.path().join(JOURNAL_PATH)).expect("journal after refusal"),
        "claim",
        &envelope,
    );
    assert_eq!(record["actor"]["run"], SECOND_RUN);
    assert_ne!(
        record["actor"]["worktree"],
        holder_record["actor"]["worktree"]
    );
    assert_eq!(record["identity_inputs"]["status"], "recorded");
    assert_eq!(
        record["scopes"],
        serde_json::json!([{"path": "crates/hana_kana/src/lib.rs", "kind": "file"}])
    );
    assert_eq!(record["holders"][0]["reservation_id"], holder_id);
    assert_eq!(
        record["holders"][0]["scopes"],
        envelope["payload"]["data"]["conflicts"][0]["overlapping_scopes"]
    );

    let after_board_output = run_berth(repository.path(), &["board", "--json"]);
    assert_success(&after_board_output);
    let after_board = json_output(&after_board_output);
    assert_eq!(
        board_reservation_ids(&before_board),
        board_reservation_ids(&after_board)
    );
    assert_eq!(
        reservation_row(&after_board, holder_id.as_str().expect("holder id"))["reservation_id"],
        holder_id
    );
}

#[test]
fn blocked_claim_names_holder_provenance_and_journals_one_refusal() {
    let repository = initialized_repository();
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
    make_dirty(repository.path(), "crates/hana_kana/src/lib.rs");
    reconcile_fixture(repository.path());
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
    assert_one_refusal_appended(
        &journal_before,
        &fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should read after rejection"),
        "claim",
        &envelope,
    );
}

#[test]
fn check_reuses_its_runs_reservation_without_git_or_a_duplicate_append() {
    let repository = initialized_repository();
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
    assert_eq!(foreign_check.status.code(), Some(4));
    assert_eq!(json_output(&foreign_check)["status"], "ledger_unreadable");
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
fn clear_check_without_session_creates_then_widens_one_exact_file_reservation() {
    let repository = initialized_repository();

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
    assert_eq!(second_acquisition["target"], first_acquisition["target"]);
    assert_eq!(second_acquisition["reservation_id"], reservation_id);
    assert_eq!(
        second_acquisition["coordination_run_id"],
        coordination_run_id
    );
    assert_eq!(second_acquisition["phase_start_head"], phase_start_head);
    assert_publication_status(&second_acquisition, "marker_publication");
    assert_eq!(
        second_acquisition["session_mapping_publication"]["status"],
        "left_unmapped"
    );
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());
    assert_eq!(journal_events(repository.path()), first_events);

    let widened = run_berth(repository.path(), &["check", "file:later.rs", "--json"]);
    let widened_envelope = json_output(&widened);
    let widened_acquisition = &widened_envelope["payload"]["data"]["acquisition"];
    assert!(widened.status.success());
    assert_eq!(widened_acquisition["kind"], "widened");
    assert_eq!(widened_acquisition["target"], first_acquisition["target"]);
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
    assert!(!repository.path().join(SESSION_MAPPING_PATH).exists());

    assert_board_contains_every_claim_source(repository.path());
}

fn assert_publication_status(acquisition: &serde_json::Value, field: &str) {
    assert_eq!(acquisition[field]["status"], "published");
}

#[test]
fn session_mapped_reservation_survives_first_touch_and_receives_widen() {
    let repository = initialized_repository();
    let session_id = "overlapping-claim-session";
    let reservations = claim_overlapping_reservations(repository.path(), session_id);
    reconcile_fixture(repository.path());

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

/// An edit a same-run, same-worktree sibling already declares is covered, so first touch
/// widens no reservation onto it.
///
/// A session mapped to its newest claim edited a path inside an older sibling's scope, and
/// first touch widened the newest claim onto it, so two siblings declared the same path.
#[test]
fn first_touch_does_not_widen_onto_a_path_a_sibling_declares() {
    let repository = initialized_repository();
    let session_id = "sibling-declared-session";
    let reservations = claim_overlapping_reservations(repository.path(), session_id);
    reconcile_fixture(repository.path());
    let events_before = journal_events(repository.path());

    let covered = run_berth_with_session(
        repository.path(),
        &["check", "file:shared/other.rs", "--json"],
        session_id,
    );
    let envelope = json_output(&covered);
    let acquisition = &envelope["payload"]["data"]["acquisition"];

    assert!(covered.status.success());
    assert_eq!(envelope["status"], "clear");
    assert_eq!(
        acquisition["kind"], "already_held",
        "the older sibling already declares the path: {acquisition}"
    );
    assert_eq!(acquisition["reservation_id"], reservations.newer);
    assert_eq!(
        journal_events(repository.path()),
        events_before,
        "no reservation widens onto a path its run already declares"
    );
    assert_session_mapping(
        repository.path(),
        session_id,
        FIRST_RUN,
        &reservations.newer,
    );
}

#[test]
fn no_session_with_two_eligible_reservations_reports_ambiguity_without_mutation() {
    let repository = initialized_repository();
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

    let ambiguous = run_berth(
        repository.path(),
        &["check", "file:shared/child.rs", "--json"],
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
    let candidates = expected_candidate_ids
        .iter()
        .map(|id| format!("`{id}` (no session)"))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(
        envelope["message"],
        format!(
            "No session names one phase's reservation among {candidates}. Run `cargo-berth check --reservation <reservation-id> <path>...` with one candidate id to select it. No reservation was appended or widened, and no session was mapped to one."
        )
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

/// The director can choose a phase directly from the refusal, without looking up sessions.
#[test]
fn ambiguous_phase_refusal_names_each_candidates_session() {
    let repository = initialized_repository();
    let root = repository.path();
    let mut phases = Vec::new();
    for (session, path) in [
        ("director-sid.phase-A", "file:phase-a.rs"),
        ("director-sid.phase-B", "file:phase-b.rs"),
    ] {
        let opened = run_berth_with_session(root, &["check", path, "--json"], session);
        let opened = json_output(&opened);
        assert_eq!(opened["status"], "clear", "{opened:#}");
        let acquisition = &opened["payload"]["data"]["acquisition"];
        assert_eq!(acquisition["kind"], "appended");
        phases.push((
            acquisition["reservation_id"]
                .as_str()
                .expect("phase reservation")
                .to_owned(),
            session,
            acquisition["coordination_run_id"].clone(),
        ));
    }
    assert_ne!(phases[0].0, phases[1].0);
    assert_eq!(phases[0].2, phases[1].2, "both phases share one run");
    phases.sort_by(|left, right| left.0.cmp(&right.0));
    let journal_before = fs::read(root.join(JOURNAL_PATH)).expect("journal before refusal");
    let mappings_before = fs::read(root.join(SESSION_MAPPING_PATH)).expect("phase mappings");

    // A usable unmapped session opens a new phase; ambiguity is the sessionless check.
    let refused = run_berth(root, &["check", "file:director-edit.rs", "--json"]);
    let envelope = json_output(&refused);
    let candidates = phases
        .iter()
        .map(|(id, session, _)| format!("`{id}` (session {session})"))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(refused.status.code(), Some(1), "{envelope:#}");
    assert_eq!(envelope["status"], "ambiguous_active_run_reservations");
    assert_eq!(
        envelope["message"],
        format!(
            "No session names one phase's reservation among {candidates}. Run `cargo-berth check --reservation <reservation-id> <path>...` with one candidate id to select it. No reservation was appended or widened, and no session was mapped to one."
        )
    );
    assert!(
        !envelope["message"]
            .as_str()
            .expect("refusal message")
            .contains("harness-session")
    );

    // The director's edit hook selects existing work even for an unmapped usable session.
    let mut hook = berth_command(BERTH_EXECUTABLE)
        .args(["hook", "pre-tool-use"])
        .current_dir(root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("director edit hook should start");
    let input = hook.stdin.take().expect("hook stdin");
    serde_json::to_writer(
        input,
        &serde_json::json!({
            "cwd": root,
            "session_id": "director-sid",
            "tool_name": "Edit",
            "tool_input": { "file_path": root.join("director-edit.rs") },
        }),
    )
    .expect("director edit payload should write");
    let hook = hook
        .wait_with_output()
        .expect("director edit hook should finish");
    assert_eq!(hook.status.code(), Some(2), "{hook:?}");
    assert!(hook.stdout.is_empty(), "{hook:?}");
    let hook_text = String::from_utf8(hook.stderr).expect("hook refusal text");
    let (_, hook_detail) = hook_text
        .split_once("\n\n")
        .expect("hook summary before detail");
    assert!(
        hook_detail.starts_with(envelope["message"].as_str().expect("refusal message")),
        "{hook_detail}"
    );
    for (id, session, _) in &phases {
        assert!(
            hook_detail.contains(&format!("- `{id}` (session {session})")),
            "{hook_detail}"
        );
    }
    assert_eq!(
        envelope["payload"]["data"]["candidate_reservation_ids"],
        serde_json::json!(phases.iter().map(|(id, _, _)| id).collect::<Vec<_>>())
    );
    assert_eq!(
        fs::read(root.join(JOURNAL_PATH)).expect("journal after refusal"),
        journal_before
    );
    assert_eq!(
        fs::read(root.join(SESSION_MAPPING_PATH)).expect("mappings after refusal"),
        mappings_before
    );
}

/// An unreadable disposable session index cannot establish that a candidate has no session.
#[test]
fn ambiguous_refusal_does_not_claim_no_sessions_when_the_index_is_unreadable() {
    let repository = initialized_repository();
    let root = repository.path();
    let reservations = claim_overlapping_reservations(root, "mapped-phase");
    let mut candidates = vec![reservations.older, reservations.newer];
    candidates.sort();
    let labels = candidates
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let journal_before = fs::read(root.join(JOURNAL_PATH)).expect("journal before refusal");
    let mapping_path = root.join(SESSION_MAPPING_PATH);
    fs::write(&mapping_path, b"not a session index").expect("corrupted session index");
    for directory_index in [false, true] {
        if directory_index {
            fs::remove_file(&mapping_path).expect("remove corrupted index");
            fs::create_dir(&mapping_path).expect("directory cannot be read as a session index");
        }
        let refused = run_berth(root, &["check", "file:shared/child.rs", "--json"]);
        let envelope = json_output(&refused);
        assert_eq!(refused.status.code(), Some(1), "{envelope:#}");
        assert_eq!(envelope["status"], "ambiguous_active_run_reservations");
        assert_eq!(
            envelope["payload"]["data"]["candidate_reservation_ids"],
            serde_json::json!(candidates)
        );
        assert_eq!(
            envelope["message"],
            format!(
                "No session names one phase's reservation among {labels}. Run `cargo-berth check --reservation <reservation-id> <path>...` with one candidate id to select it. No reservation was appended or widened, and no session was mapped to one."
            )
        );
        let detail = envelope["presentation"]["blocks"][0]["detail"]
            .as_str()
            .expect("ambiguity detail");
        assert!(!detail.contains("(no session)"), "{detail}");
        for candidate in &candidates {
            assert!(
                detail
                    .lines()
                    .any(|line| line == format!("- `{candidate}`")),
                "{detail}"
            );
        }
        assert_eq!(
            fs::read(root.join(JOURNAL_PATH)).expect("journal after refusal"),
            journal_before
        );
        if directory_index {
            assert!(mapping_path.is_dir());
        } else {
            assert_eq!(
                fs::read(&mapping_path).expect("corrupted index remains"),
                b"not a session index"
            );
        }
    }
}

#[test]
fn explicit_check_selection_republishes_mapping_for_subsequent_checks() {
    let repository = initialized_repository();
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
fn a_new_cargo_berth_session_opens_a_reservation_with_several_active_siblings() {
    assert_unmapped_session_opens_its_own_reservation(HarnessSession::CargoBerthEnvironment(
        "new phase's session",
    ));
}

#[test]
fn a_new_claude_code_session_opens_a_reservation_with_several_active_siblings() {
    assert_unmapped_session_opens_its_own_reservation(HarnessSession::ClaudeCode(
        "claude code phase session",
    ));
}

#[test]
fn explicit_check_without_harness_session_reports_invocation_only_selection() {
    let repository = initialized_repository();
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
    let repository = initialized_repository();
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
    reconcile_fixture(repository.path());
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
    let repository = initialized_repository();
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
fn blocked_check_returns_holder_decision_facts_and_journals_one_refusal() {
    let repository = initialized_repository();
    let holder = berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:shared.rs", "--json"])
        .current_dir(repository.path())
        .env(RUN_ENVIRONMENT, FIRST_RUN)
        .output()
        .expect("first-touch holder check should run");
    assert!(holder.status.success());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    make_dirty(repository.path(), "shared.rs");
    reconcile_fixture(repository.path());
    let journal_before = fs::read(repository.path().join(JOURNAL_PATH))
        .expect("journal should read before blocked check");

    let blocked = berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
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
    let record = assert_one_refusal_appended(
        &journal_before,
        &fs::read(repository.path().join(JOURNAL_PATH))
            .expect("journal should reread after blocked check"),
        "check",
        &envelope,
    );
    assert_eq!(
        record["scopes"],
        serde_json::json!([{"path": "shared.rs", "kind": "file"}])
    );
}

#[test]
fn concurrent_first_touch_checks_admit_both_clean_worktrees_under_the_mutation_lock() {
    let repository = initialized_repository();
    let mutation_lock =
        File::open(repository.path().join(LOCK_PATH)).expect("mutation lock should open");
    mutation_lock
        .lock()
        .expect("test should hold mutation lock");
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let first_ready_path = repository.path().join("first-lock-ready");
    let second_ready_path = repository.path().join("second-lock-ready");
    let first = berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:raced.rs", "--json"])
        .current_dir(repository.path())
        .env(RUN_ENVIRONMENT, FIRST_RUN)
        .env(MUTATION_LOCK_READY_PATH_ENVIRONMENT, &first_ready_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("first concurrent check should start");
    let second = berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
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
        2,
        "separate clean branches have no conflicting merge surface: {status_codes:?}"
    );
    let events = journal_events(repository.path());
    let claims: Vec<_> = events
        .iter()
        .filter(|event| event["op"] == "claim")
        .collect();
    assert_eq!(claims.len(), 2);
    for event in claims {
        assert_eq!(event["source"]["kind"], "first_touch");
        assert_eq!(event["scopes"][0]["path"], "raced.rs");
    }
}

#[test]
fn check_reports_unconfigured_when_an_initialized_repository_loses_its_configuration() {
    let repository = initialized_repository();
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

/// A linked worktree added after `init` has no configuration file of its own, because the
/// file is untracked and `git worktree add` does not carry it. It reads the main
/// worktree's file instead, so it coordinates from its first edit.
#[test]
fn check_in_a_linked_worktree_without_its_own_configuration_reads_the_main_worktree() {
    let repository = initialized_repository();
    let claim = run_berth(
        repository.path(),
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(claim.status.success());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    make_dirty(repository.path(), "src/lib.rs");
    reconcile_fixture(repository.path());
    fs::remove_file(second_root.join(CONFIGURATION_PATH)).expect("configuration should be removed");

    let check = berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(&second_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("cargo-berth check should run");
    let envelope = json_output(&check);

    assert_eq!(envelope["status"], "blocked_by_overlap");
}

/// With no configuration in the main worktree either, a linked worktree is unconfigured,
/// says so, and names the main worktree's file as the one `init` should create.
#[test]
fn check_does_not_replay_a_foreign_conflict_after_configuration_is_removed() {
    let repository = initialized_repository();
    let claim = run_berth(
        repository.path(),
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(claim.status.success());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    fs::remove_file(second_root.join(CONFIGURATION_PATH)).expect("configuration should be removed");
    let main_configuration_path = repository.path().join(CONFIGURATION_PATH);
    fs::remove_file(&main_configuration_path).expect("main configuration should be removed");

    let check = berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
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
    assert!(envelope["message"].as_str().is_some_and(|message| {
        message.contains(&main_configuration_path.display().to_string())
    }));
}

#[test]
fn check_reports_malformed_present_configuration_as_ledger_unreadable() {
    let repository = initialized_repository();
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
    let repository = initialized_repository();
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
    let repository = initialized_repository();
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
fn blocked_check_message_names_only_the_foreign_holder_run() {
    let repository = initialized_repository();
    let holder = run_berth(
        repository.path(),
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "update the holder library",
            "--json",
        ],
    );
    assert_success(&holder);
    let holder = json_output(&holder);
    let holder_id = holder["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("holder reservation id");
    let holder_run = holder["payload"]["data"]["coordination_run_id"]
        .as_str()
        .expect("holder coordination run");
    let (_requester_directory, requester_root) = foreign_worktree(&repository, "requester");
    make_dirty(repository.path(), "src/lib.rs");

    let blocked = berth_command(BERTH_EXECUTABLE)
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(&requester_root)
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("requester's check should run");

    assert_eq!(blocked.status.code(), Some(1));
    let envelope = json_output(&blocked);
    assert_eq!(envelope["status"], "blocked_by_overlap");
    let message = envelope["message"].as_str().expect("blocked check message");
    assert_eq!(
        message,
        format!(
            "Reservation {holder_id} (coordination run {holder_run}) on refs/heads/main (explicit claim, update the holder library) holds paths that overlap the requested paths; reduce the requested scopes or coordinate with the holder, then retry."
        )
    );
    assert!(
        !message.contains(SECOND_RUN),
        "the overlap message must identify the holder without naming the requester run"
    );
}

#[test]
fn blocked_message_names_every_holder() {
    let repository = initialized_repository();
    let (_holder_directory, holder_root) = foreign_worktree(&repository, "other-holder");
    let mut holders = Vec::new();
    for (root, scope, run) in [
        (repository.path(), "tree:src", FIRST_RUN),
        (holder_root.as_path(), "file:src/lib.rs", SECOND_RUN),
    ] {
        let claimed = run_berth(root, &["claim", scope, "--run", run, "--json"]);
        assert_success(&claimed);
        let claimed = json_output(&claimed);
        holders.push((
            claimed["payload"]["data"]["reservation_id"]
                .as_str()
                .expect("holder reservation id")
                .to_owned(),
            run,
        ));
    }

    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    make_dirty(repository.path(), "src/lib.rs");
    make_dirty(&holder_root, "src/lib.rs");
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
    assert!(message.starts_with("2 reservations hold paths that overlap the requested paths: "));
    for (reservation_id, run) in holders {
        let description = format!("reservation {reservation_id} (coordination run {run}) on ");
        assert!(
            message.contains(&description),
            "the holder's run must appear beside its reservation: {message}"
        );
        assert_eq!(message.matches(&description).count(), 1);
    }
}

#[test]
fn a_first_touch_holder_block_names_the_verbs_that_clear_it() {
    let repository = initialized_repository();
    let first_touch = run_berth(repository.path(), &["check", "file:touched.rs", "--json"]);
    assert!(first_touch.status.success());
    let reservation_id =
        json_output(&first_touch)["payload"]["data"]["acquisition"]["reservation_id"]
            .as_str()
            .expect("a clear check should create a first-touch reservation")
            .to_owned();

    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    make_dirty(repository.path(), "touched.rs");
    reconcile_fixture(repository.path());
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
    let repository = initialized_repository();
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
    let repository = initialized_repository();
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
    make_dirty(repository.path(), "Cargo.toml");
    reconcile_fixture(repository.path());
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

/// Seed retained declarations for missing checkouts without starting one process per holder.
fn seed_unavailable_tree_holders(root: &Path, holder_id: &str) -> String {
    let mut events = journal_events(root);
    let prototype = events
        .iter()
        .find(|event| event["op"] == "claim" && event["reservation_id"] == holder_id)
        .expect("holder claim")
        .clone();
    let mut generation = events
        .iter()
        .map(|event| event["projection_generation"].as_u64().expect("generation"))
        .max()
        .expect("journal generation");
    // An unavailable checkout retains its declared protection. Replay those claims instead of
    // spawning 123 worktrees and commands; the live claim still binds every retained holder.
    for index in 1..TREE_CLAIM_HOLDER_COUNT {
        let mut event = prototype.clone();
        generation += 1;
        event["projection_generation"] = generation.into();
        event["event_id"] = uuid::Uuid::now_v7().to_string().into();
        event["reservation_id"] = uuid::Uuid::now_v7().to_string().into();
        event["actor"]["worktree"] = uuid::Uuid::now_v7().to_string().into();
        event["worktree_root"] = root
            .join(".git")
            .join(format!("missing-holder-{index}"))
            .to_str()
            .expect("fixture root")
            .into();
        event["worktree_administrative_locator"] =
            format!("worktrees/missing-holder-{index}").into();
        event["scopes"] = serde_json::json!([{
            "kind": "file", "path": format!("crates/hana/tests/holder_{index}.rs"),
        }]);
        events.push(event);
    }
    let journal = events
        .iter()
        .map(|event| serde_json::to_string(event).expect("fixture record") + "\n")
        .collect::<String>();
    fs::write(root.join(JOURNAL_PATH), &journal).expect("holder claims should write");
    fs::remove_file(root.join(PROJECTION_PATH)).expect("cached projection should be removed");
    let checkpoint = root.join(".git/cargo-berth/replay-checkpoint.json");
    if checkpoint.exists() {
        fs::remove_file(checkpoint).expect("cached replay checkpoint should be removed");
    }
    journal
}

fn initialized_repository() -> TempDir { INITIALIZED_REPOSITORY.instantiate() }

fn build_initialized_repository(root: &Path) {
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Berth Test"]);
    git(root, &["config", "user.email", "berth@example.invalid"]);
    git(root, &["config", "core.ignoreCase", "false"]);
    fs::write(root.join("README.md"), "scratch repository\n").expect("scratch file should write");
    git(root, &["add", "README.md"]);
    git(root, &["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth(root, &["init", "--json"]);
    assert!(initialized.status.success());
}

/// Add a real worktree beside the repository, the second party berth has always refused.
///
/// A distinct `--run` inside one worktree now names a second party too, but only a real
/// worktree can hold a reservation of its own alongside another run's. The returned
/// directory owns the worktree and must outlive its use.
fn foreign_worktree(repository: &TempDir, name: &str) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("foreign worktree parent should exist");
    let root = directory.path().join(name);
    GIT.add_worktree_without_hooks(repository.path(), &root, name, "HEAD");
    let configuration = root.join(CONFIGURATION_PATH);
    if let Some(parent) = configuration.parent() {
        fs::create_dir_all(parent).expect("foreign worktree configuration should have a directory");
    }
    fs::copy(repository.path().join(CONFIGURATION_PATH), configuration)
        .expect("foreign worktree should share the repository configuration");
    (directory, root)
}

/// Give a foreign reservation a real uncommitted merge surface.
fn make_dirty(root: &Path, path: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("dirty fixture has a parent"))
        .expect("dirty fixture parent should exist");
    fs::write(file, "uncommitted holder work\n").expect("dirty fixture should write");
}

/// Separate reconciliation's durable observation from the command mutation under test.
fn reconcile_fixture(root: &Path) {
    assert!(run_berth(root, &["board", "--json"]).status.success());
}

fn run_check_without_git(worktree_root: &Path, path: &Path, run: Option<&str>) -> Output {
    let mut command = berth_command(env!("CARGO_BIN_EXE_cargo-berth"));
    command
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(worktree_root)
        .env("PATH", path);
    match run {
        Some(run) => command.env(RUN_ENVIRONMENT, run),
        // Keep the foreign checkout's unavailable-Git case on the explicit config route.
        None => {
            cargo_berth_test_support::use_git_read_subprocess(command.env_remove(RUN_ENVIRONMENT))
        },
    };
    command
        .output()
        .expect("cargo-berth check should run without git")
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run with a harness session")
}

/// The two supported sources of a usable harness session id.
#[derive(Clone, Copy)]
enum HarnessSession {
    CargoBerthEnvironment(&'static str),
    ClaudeCode(&'static str),
}

impl HarnessSession {
    const fn session_id(self) -> &'static str {
        match self {
            Self::CargoBerthEnvironment(session_id) | Self::ClaudeCode(session_id) => session_id,
        }
    }

    fn bind(self, command: &mut Command) -> &mut Command {
        match self {
            Self::CargoBerthEnvironment(session_id) => command
                .env(SESSION_ENVIRONMENT, session_id)
                .env_remove(CLAUDE_CODE_SESSION_ENVIRONMENT),
            Self::ClaudeCode(session_id) => command
                .env_remove(SESSION_ENVIRONMENT)
                .env(CLAUDE_CODE_SESSION_ENVIRONMENT, session_id),
        }
    }
}

fn assert_unmapped_session_opens_its_own_reservation(session: HarnessSession) {
    let repository = initialized_repository();
    let siblings = claim_overlapping_reservations(repository.path(), "existing-phase-session");
    let mapping_before = fs::read(repository.path().join(SESSION_MAPPING_PATH))
        .expect("existing session mapping should read");
    let events_before = journal_events(repository.path());
    let covered = session
        .bind(
            berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
                .args(["check", "file:shared/child.rs", "--json"])
                .current_dir(repository.path())
                .env_remove(RUN_ENVIRONMENT),
        )
        .output()
        .expect("covered-only check should run");
    assert!(covered.status.success());
    let covered = json_output(&covered);
    assert_eq!(
        covered["payload"]["data"]["acquisition"]["kind"],
        "already_held"
    );
    assert_eq!(
        covered["payload"]["data"]["acquisition"]["reservation_id"],
        siblings.older
    );
    assert_eq!(journal_events(repository.path()), events_before);
    assert_eq!(
        fs::read(repository.path().join(SESSION_MAPPING_PATH)).expect("mapping should reread"),
        mapping_before
    );

    let opened = session
        .bind(
            berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
                .args(["check", "file:shared/child.rs", "file:phase.rs", "--json"])
                .current_dir(repository.path())
                .env_remove(RUN_ENVIRONMENT),
        )
        .output()
        .expect("new phase check should run");
    assert!(opened.status.success());
    let opened = json_output(&opened);
    let acquisition = &opened["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "appended");
    let id = acquisition["reservation_id"]
        .as_str()
        .expect("new phase id");
    assert_ne!(id, siblings.older);
    assert_ne!(id, siblings.newer);
    assert_eq!(acquisition["coordination_run_id"], FIRST_RUN);
    assert_session_mapping(repository.path(), session.session_id(), FIRST_RUN, id);
    let events = journal_events(repository.path());
    let claim = events.last().expect("new phase claim");
    assert_eq!(claim["op"], "claim");
    assert_eq!(claim["actor"]["run"], FIRST_RUN);
    assert_eq!(claim["source"]["kind"], "first_touch");
    assert_eq!(
        claim["scopes"],
        serde_json::json!([{"kind": "file", "path": "phase.rs"}])
    );

    let widened = session
        .bind(
            berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
                .args(["check", "file:phase-next.rs", "--json"])
                .current_dir(repository.path())
                .env_remove(RUN_ENVIRONMENT),
        )
        .output()
        .expect("mapped phase check should run");
    assert!(widened.status.success());
    let widened = json_output(&widened);
    assert_eq!(widened["payload"]["data"]["acquisition"]["kind"], "widened");
    assert_eq!(
        widened["payload"]["data"]["acquisition"]["reservation_id"],
        id
    );
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

/// Compare claim and authorization mutations independently of automatic extent observations.
/// The `lifecycle::merge_extent` tests assert transitions against the full journal.
fn journal_events(repository_root: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .filter(|event: &serde_json::Value| !is_merge_extent_observation(event))
        .collect()
}

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}
