#![allow(
    clippy::expect_used,
    reason = "tests should stop immediately when fixtures or command output are invalid"
)]

//! Built-binary tests for the headless board and its coherent replay projection.

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: no optional locks, with nothing held back from a hook.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};

use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
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
const BOARD_LOCKED_READ_TIMEOUT: Duration = Duration::from_secs(60);
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GIT_BINARY: &str = "git";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const PENDING_BYPASS_NAME: &str =
    "cargo-berth-pending-bypass-01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a99.json";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
const BOARD_READY_MESSAGE: &str =
    "The reservation board was read. Use `cargo-berth board --json` to inspect it.";
const ENTER_ALTERNATE_SCREEN: &str = "\u{1b}[?1049h";
#[cfg(target_os = "macos")]
const LEAVE_ALTERNATE_SCREEN: &str = "\u{1b}[?1049l";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    command_name="$2"
    (
        shift 2
        command_line="$command_name"
        for argument in "$@"; do command_line="$command_line $argument"; done
        printf '%s\n' "$command_line" >> "$CARGO_BERTH_TEST_GIT_TRACE"
    )
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const BLOCKING_GIT_WRAPPER: &str = r#"#!/bin/sh

if [ "$1" = "--no-optional-locks" ] && [ "$2" = "worktree" ] && [ "$3" = "list" ]; then
    : > "$CARGO_BERTH_TEST_BOARD_SIGNAL"
    while [ ! -e "$CARGO_BERTH_TEST_BOARD_RELEASE" ]; do sleep 0.01; done
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;

#[test]
fn empty_board_is_headless_and_declares_no_integration_order() {
    let repository = initialized_repository();
    let json = run_berth(repository.path(), &["board", "--json"]);
    assert!(json.status.success());
    let envelope = json_output(&json);
    assert_preserved_board_envelope_fields(&envelope, &[], "board", BOARD_READY_MESSAGE);
    assert_complete_board_payload_sections(&envelope["payload"]["data"]);
    assert_eq!(envelope["status"], "board_ready");
    assert_eq!(envelope["payload"]["kind"], "board");
    assert_eq!(
        envelope["payload"]["data"]["integration_order"],
        "undeclared"
    );
    assert_eq!(
        envelope["presentation"],
        serde_json::json!({
            "kind": "rendered_blocks",
            "blocks": [],
        })
    );
    assert_eq!(envelope["message"], BOARD_READY_MESSAGE);
    assert!(!envelope.to_string().contains("unimplemented"));
    assert!(!String::from_utf8_lossy(&json.stdout).contains(ENTER_ALTERNATE_SCREEN));

    let text = run_berth(repository.path(), &["board"]);
    assert!(text.status.success());
    assert_eq!(
        String::from_utf8_lossy(&text.stdout).trim(),
        BOARD_READY_MESSAGE
    );
    assert!(String::from_utf8_lossy(&text.stdout).contains("--json"));
    assert!(!String::from_utf8_lossy(&text.stdout).contains(ENTER_ALTERNATE_SCREEN));
    assert!(!String::from_utf8_lossy(&text.stdout).contains("BoardReservationSnapshot"));
}

#[test]
fn populated_board_presentation_carries_the_complete_board_report() {
    let fixture = ordered_fixture();
    let output = run_berth(fixture.repository.path(), &["board", "--json"]);
    assert!(
        output.status.success(),
        "board failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope = json_output(&output);
    assert_preserved_board_envelope_fields(
        &envelope,
        &[&fixture.predecessor_id, &fixture.successor_id],
        "board",
        BOARD_READY_MESSAGE,
    );
    assert_complete_board_payload_sections(&envelope["payload"]["data"]);

    let report = rendered_board_report(&envelope, "complete board");
    let report_object = report
        .as_object()
        .expect("complete board report should be a JSON object");
    assert_eq!(report_object.len(), 16);
    let board_data = &envelope["payload"]["data"];
    for (report_property, payload_field) in [
        ("Journal position", "journal_position"),
        (
            "Recovered bypasses this invocation",
            "recovered_bypasses_this_invocation",
        ),
        ("Integration order", "integration_order"),
        ("Ready now", "ready_now"),
        ("Waiting", "waiting"),
        (
            "Settled ordering constraints",
            "settled_ordering_constraints",
        ),
        ("Unresolved overlaps", "unresolved_overlaps"),
        ("Recorded overlap answers", "recorded_overlap_answers"),
        ("Unconstrained reservations", "unconstrained_reservations"),
        ("Resolved reservations", "resolved"),
        ("Available forced permits", "available_forced_permits"),
        ("Bypass audit", "bypass_audit"),
        ("Outstanding incursions", "outstanding_incursions"),
        ("Recorded incursion answers", "recorded_incursion_answers"),
        ("Alerts", "alerts"),
        ("Git cost", "git_cost"),
    ] {
        assert_eq!(
            report[report_property], board_data[payload_field],
            "complete board report property {report_property:?} diverged from payload field {payload_field:?}"
        );
    }
}

#[test]
fn waiting_successor_lifecycle_is_queryable_while_omitted_from_board_rows() {
    let fixture = ordered_fixture();
    fs::write(
        fixture.successor_root.join("src/lib.rs"),
        "pub fn successor() {}\n",
    )
    .expect("successor source should write");
    git(&fixture.successor_root, &["add", "src/lib.rs"]);
    git(
        &fixture.successor_root,
        &["commit", "--quiet", "-m", "successor work"],
    );
    let protected_tip = git_stdout(&fixture.successor_root, &["rev-parse", "HEAD"]);
    let released = run_berth_with_run(
        &fixture.successor_root,
        &["release", &fixture.successor_id, "--json"],
        SECOND_RUN,
    );
    assert!(released.status.success());

    let complete_board = board_data(fixture.repository.path());
    assert_eq!(
        complete_board["waiting"]["entries"][0]["successor"],
        fixture.successor_id
    );
    assert!(!has_reservation_snapshot(
        &complete_board,
        &fixture.successor_id
    ));
    assert_reservation_lifecycle(
        fixture.repository.path(),
        &fixture.successor_id,
        "outstanding",
        &protected_tip,
    );

    let text_selector = run_berth(
        fixture.repository.path(),
        &["board", "--reservation", &fixture.successor_id],
    );
    assert_eq!(text_selector.status.code(), Some(5));
    assert!(String::from_utf8_lossy(&text_selector.stderr).contains("--json"));
}

#[test]
fn both_unresolved_overlap_endpoints_are_queryable_while_omitted_from_board_rows() {
    let repository = initialized_repository();
    let (_blocker_directory, blocker_root) = foreign_worktree(&repository, "blocker");
    let (_deferred_directory, deferred_root) = foreign_worktree(&repository, "deferred");
    let blocker = claim(&blocker_root, "file:src/lib.rs", FIRST_RUN);
    let blocker_id = reservation_id(&blocker);
    let deferred = defer_claim(&deferred_root, "file:src/lib.rs", SECOND_RUN, &blocker_id);
    let deferred_id = reservation_id(&deferred);

    fs::write(blocker_root.join("src/lib.rs"), "pub fn blocker() {}\n")
        .expect("blocker source should write");
    git(&blocker_root, &["add", "src/lib.rs"]);
    git(&blocker_root, &["commit", "--quiet", "-m", "blocker work"]);
    let blocker_tip = git_stdout(&blocker_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth_with_run(
            &blocker_root,
            &["release", &blocker_id, "--json"],
            FIRST_RUN,
        )
        .status
        .success()
    );

    fs::write(deferred_root.join("src/lib.rs"), "pub fn deferred() {}\n")
        .expect("deferred source should write");
    git(&deferred_root, &["add", "src/lib.rs"]);
    git(
        &deferred_root,
        &["commit", "--quiet", "-m", "deferred work"],
    );
    let deferred_tip = git_stdout(&deferred_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth_with_run(
            &deferred_root,
            &["release", &deferred_id, "--json"],
            SECOND_RUN,
        )
        .status
        .success()
    );

    let complete_board = board_data(repository.path());
    let overlap = &complete_board["unresolved_overlaps"]["entries"][0];
    assert_eq!(overlap["blocker"], blocker_id);
    assert_eq!(overlap["deferred"], deferred_id);
    for reservation_id in [&blocker_id, &deferred_id] {
        assert!(!has_reservation_snapshot(&complete_board, reservation_id));
    }
    assert_reservation_lifecycle(repository.path(), &blocker_id, "outstanding", &blocker_tip);
    assert_reservation_lifecycle(
        repository.path(),
        &deferred_id,
        "outstanding",
        &deferred_tip,
    );
}

#[test]
fn unknown_reservation_lifecycle_is_a_typed_invalid_input() {
    let repository = initialized_repository();
    let reservation_id = uuid::Uuid::now_v7().to_string();
    let output = run_berth(
        repository.path(),
        &["board", "--reservation", &reservation_id, "--json"],
    );
    assert_eq!(output.status.code(), Some(5));
    let envelope = json_output(&output);
    assert_eq!(envelope["verb"], "board");
    assert_eq!(envelope["status"], "invalid_input");
    assert_eq!(envelope["exit_code"], 5);
    assert_eq!(
        envelope["reservations"],
        serde_json::json!([reservation_id])
    );
    assert_eq!(envelope["payload"]["kind"], "reservation");
    assert_eq!(
        envelope["payload"]["data"],
        serde_json::json!({
            "status": "unknown_reservation",
            "reservation_id": reservation_id,
        })
    );
}

#[test]
fn reservation_lifecycle_query_distinguishes_all_four_states() {
    let repository = initialized_repository();
    let (_checkpoint_directory, checkpoint_root) =
        foreign_worktree(&repository, "checkpoint-lifecycle");
    let checkpointed = claim(&checkpoint_root, "file:src/lib.rs", FIRST_RUN);
    let checkpointed_id = reservation_id(&checkpointed);
    assert_eq!(
        reservation_lifecycle(repository.path(), &checkpointed_id),
        serde_json::json!({"status": "active"})
    );

    fs::write(
        checkpoint_root.join("src/lib.rs"),
        "pub fn checkpointed() {}\n",
    )
    .expect("checkpointed source should write");
    git(&checkpoint_root, &["add", "src/lib.rs"]);
    git(
        &checkpoint_root,
        &["commit", "--quiet", "-m", "checkpointed work"],
    );
    let protected_tip = git_stdout(&checkpoint_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth_with_run(
            &checkpoint_root,
            &["release", &checkpointed_id, "--json"],
            FIRST_RUN,
        )
        .status
        .success()
    );
    assert_eq!(
        reservation_lifecycle(repository.path(), &checkpointed_id),
        serde_json::json!({
            "status": "outstanding",
            "protected_tip": protected_tip,
        })
    );
    assert!(
        run_berth_with_run(
            &checkpoint_root,
            &[
                "resolve",
                &checkpointed_id,
                "--abandon",
                "--why",
                "checkpointed work is deliberately discarded",
                "--json",
            ],
            FIRST_RUN,
        )
        .status
        .success()
    );
    assert_eq!(
        reservation_lifecycle(repository.path(), &checkpointed_id),
        serde_json::json!({
            "status": "released_after_checkpoint",
            "protected_tip": protected_tip,
            "disposition": {
                "kind": "abandoned",
                "evidence": "checkpointed work is deliberately discarded",
            },
        })
    );

    let (_uncheckpointed_directory, uncheckpointed_root) =
        foreign_worktree(&repository, "uncheckpointed-lifecycle");
    let uncheckpointed = claim(
        &uncheckpointed_root,
        "file:src/uncheckpointed.rs",
        SECOND_RUN,
    );
    let uncheckpointed_id = reservation_id(&uncheckpointed);
    assert!(
        run_berth_with_run(
            &uncheckpointed_root,
            &[
                "resolve",
                &uncheckpointed_id,
                "--abandon",
                "--why",
                "uncheckpointed work is deliberately discarded",
                "--json",
            ],
            SECOND_RUN,
        )
        .status
        .success()
    );
    assert_eq!(
        reservation_lifecycle(repository.path(), &uncheckpointed_id),
        serde_json::json!({
            "status": "released_without_checkpoint",
            "disposition": {
                "kind": "abandoned",
                "evidence": "uncheckpointed work is deliberately discarded",
            },
        })
    );
}

#[cfg(target_os = "macos")]
#[test]
fn human_board_uses_and_restores_an_attached_terminal() {
    let repository = initialized_repository();
    let transcript = repository.path().join("terminal-transcript");
    let mut child = Command::new("/usr/bin/script")
        .args([
            "-q",
            transcript
                .to_str()
                .expect("terminal transcript path should be UTF-8"),
            env!("CARGO_BIN_EXE_cargo-berth"),
            "board",
        ])
        .current_dir(repository.path())
        .env("COLUMNS", "100")
        .env("LINES", "30")
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("terminal board should start under a pseudo-terminal");
    child
        .stdin
        .take()
        .expect("script input should be piped")
        .write_all(b"q")
        .expect("quit key should reach the pseudo-terminal");
    let output = child
        .wait_with_output()
        .expect("terminal board should finish after quit");
    assert!(
        output.status.success(),
        "terminal board failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let rendered = fs::read_to_string(transcript).expect("terminal transcript should read");
    let entered = rendered
        .find(ENTER_ALTERNATE_SCREEN)
        .expect("terminal view should enter the alternate screen");
    let restored = rendered
        .rfind(LEAVE_ALTERNATE_SCREEN)
        .expect("terminal view should leave the alternate screen");
    assert!(restored > entered);
    assert!(!rendered.contains(BOARD_READY_MESSAGE));
}

#[test]
fn board_outside_a_git_worktree_reports_the_same_unreadable_facts_in_both_modes() {
    let outside_repository = tempdir().expect("temporary directory should exist");
    let json = run_berth(outside_repository.path(), &["board", "--json"]);
    assert_eq!(json.status.code(), Some(4));
    let envelope = json_output(&json);
    assert_eq!(envelope["status"], "ledger_unreadable");
    assert_eq!(envelope["exit_code"], 4);
    assert_eq!(envelope["payload"]["kind"], "no_facts");
    assert_eq!(envelope["reservations"], serde_json::json!([]));
    assert_eq!(envelope["blocked_by"], serde_json::json!([]));

    let human = run_berth(outside_repository.path(), &["board"]);
    assert_eq!(human.status.code(), Some(4));
    assert_eq!(
        String::from_utf8_lossy(&human.stdout).trim(),
        envelope["message"]
            .as_str()
            .expect("unreadable envelope should carry a message")
    );
    assert!(!String::from_utf8_lossy(&human.stdout).contains(ENTER_ALTERNATE_SCREEN));
}

#[test]
fn resolved_deferral_moves_to_answer_audit_with_both_reasons() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let predecessor = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let deferred = defer_claim(&second_root, "file:src/lib.rs", SECOND_RUN, &predecessor_id);
    let deferred_id = reservation_id(&deferred);
    let sequenced = run_berth(
        repository.path(),
        &[
            "sequence",
            &predecessor_id,
            &deferred_id,
            "--why",
            "the predecessor API is now chosen first",
            "--json",
        ],
    );
    assert!(sequenced.status.success());

    let board = json_output(&run_berth(repository.path(), &["board", "--json"]));
    let data = &board["payload"]["data"];
    assert_eq!(
        data["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
    assert_eq!(
        data["unresolved_overlaps"]["entries"],
        serde_json::json!([])
    );
    let answers = data["recorded_overlap_answers"]["entries"]
        .as_array()
        .expect("answer audit should be an array");
    let resolved = answers
        .iter()
        .find(|answer| answer["answer"] == "ordering_created_from_deferral")
        .expect("resolved deferral should become an ordering answer");
    assert_eq!(resolved["direction"], "holder_before_requester");
    assert_eq!(
        resolved["deferral_reasons"][0],
        "the order is not known yet"
    );
    assert_eq!(
        resolved["ordering_reason"],
        "the predecessor API is now chosen first"
    );
    assert!(!answers.iter().any(|answer| answer["answer"] == "defer"));

    let position = &data["journal_position"];
    for section in [
        "ready_now",
        "waiting",
        "settled_ordering_constraints",
        "unresolved_overlaps",
        "recorded_overlap_answers",
        "alerts",
    ] {
        assert_eq!(&data[section]["journal_position"], position);
    }
}

#[test]
fn board_sections_share_one_locked_generation_when_a_claim_arrives_mid_read() {
    let repository = initialized_repository();
    let wrapper_directory = tempdir().expect("git wrapper directory should exist");
    let wrapper_path = wrapper_directory.path().join(GIT_BINARY);
    let signal_path = wrapper_directory.path().join("board-has-lock");
    let release_path = wrapper_directory.path().join("release-board");
    fs::write(&wrapper_path, BLOCKING_GIT_WRAPPER).expect("blocking wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("blocking wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("blocking wrapper should execute");
    let original_path = std::env::var_os("PATH").expect("PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(wrapper_directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");

    let board = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["board", "--json"])
        .current_dir(repository.path())
        .env("PATH", wrapped_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env("CARGO_BERTH_TEST_BOARD_SIGNAL", &signal_path)
        .env("CARGO_BERTH_TEST_BOARD_RELEASE", &release_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("board should spawn");
    let deadline = Instant::now() + BOARD_LOCKED_READ_TIMEOUT;
    while !signal_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        signal_path.exists(),
        "board should reach its locked repository read"
    );

    let claim = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args([
            "claim",
            "file:arrived-mid-read.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "arrive during board read",
            "--json",
        ])
        .current_dir(repository.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("concurrent claim should spawn");
    fs::write(&release_path, b"release\n").expect("board release signal should write");
    let board_output = board
        .wait_with_output()
        .expect("board should complete after release");
    let claim_output = claim
        .wait_with_output()
        .expect("claim should complete after board releases the lock");
    assert!(board_output.status.success());
    assert!(claim_output.status.success());

    let board = json_output(&board_output);
    let data = &board["payload"]["data"];
    let position = &data["journal_position"];
    for section in [
        "ready_now",
        "waiting",
        "settled_ordering_constraints",
        "unresolved_overlaps",
        "recorded_overlap_answers",
        "alerts",
    ] {
        assert_eq!(&data[section]["journal_position"], position);
    }
    assert_eq!(
        data["unconstrained_reservations"]["entries"],
        serde_json::json!([])
    );
    let concurrent_id = reservation_id(&claim_output);
    assert!(!data.to_string().contains(&concurrent_id));
    assert!(
        fs::metadata(repository.path().join(JOURNAL_PATH))
            .expect("journal metadata should read")
            .len()
            > position["journal_byte_offset"]
                .as_u64()
                .expect("board offset should be numeric")
    );
}

#[test]
fn overlap_answers_keep_exact_scopes_direction_reason_and_consequence() {
    enum ExpectedOverlapAnswerConsequence {
        SequenceHolding {
            state:         &'static str,
            action_reason: &'static str,
        },
        Deferral(&'static str),
        Override(&'static str),
    }

    for (answer_flag, expected_answer, expected_direction, expected_consequence) in [
        (
            "--after",
            "sequence",
            Some("holder_before_requester"),
            ExpectedOverlapAnswerConsequence::SequenceHolding {
                state:         "holding",
                action_reason: "predecessor_checkpoint",
            },
        ),
        (
            "--defer",
            "defer",
            None,
            ExpectedOverlapAnswerConsequence::Deferral("both_integrations_held_until_sequence"),
        ),
        (
            "--override",
            "override",
            None,
            ExpectedOverlapAnswerConsequence::Override(
                "editing_authorized_without_integration_order",
            ),
        ),
    ] {
        let repository = initialized_repository();
        let (_second_directory, second_root) = foreign_worktree(&repository, "second");
        let blocker = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
        let blocker_id = reservation_id(&blocker);
        let answered = answered_claim(
            &second_root,
            "file:src/lib.rs",
            SECOND_RUN,
            answer_flag,
            &blocker_id,
            "the exact overlap answer is approved",
        );
        let answered_id = reservation_id(&answered);
        let data = board_data(repository.path());
        let answers = data["recorded_overlap_answers"]["entries"]
            .as_array()
            .expect("recorded answers should be an array");
        let answer = answers
            .iter()
            .find(|answer| answer["answer"] == expected_answer)
            .expect("approved answer should render");
        assert_eq!(answer["reservation_id"], answered_id);
        assert_eq!(answer["blocker"], blocker_id);
        assert_eq!(
            answer["exact_approved_scopes"][0]["reservation_id"],
            blocker_id
        );
        assert_eq!(
            answer["exact_approved_scopes"][0]["scopes"][0],
            serde_json::json!({"path": "src/lib.rs", "kind": "file"})
        );
        assert_eq!(
            answer["authorization_reason"],
            "the exact overlap answer is approved"
        );
        match expected_direction {
            Some(direction) => assert_eq!(answer["direction"], direction),
            None => assert!(
                answer.get("direction").is_none(),
                "an answer that orders nothing must have no direction cell"
            ),
        }
        match expected_consequence {
            ExpectedOverlapAnswerConsequence::SequenceHolding {
                state,
                action_reason,
            } => {
                assert_eq!(answer["consequence"]["state"], state);
                assert_eq!(answer["consequence"]["action"]["reason"], action_reason);
                let instruction = answer["consequence"]["action"]["instruction"]
                    .as_str()
                    .expect("a held sequence consequence should carry an instruction");
                assert!(!instruction.is_empty());
            },
            ExpectedOverlapAnswerConsequence::Deferral(expected)
            | ExpectedOverlapAnswerConsequence::Override(expected) => {
                assert_eq!(answer["consequence"], expected);
            },
        }
        assert!(!data.to_string().contains("proposal_token"));
    }
}

#[test]
fn independent_ready_reservations_are_an_unnumbered_tie() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let first = claim(repository.path(), "file:first.rs", FIRST_RUN);
    let first_id = reservation_id(&first);
    let second = answered_claim(
        &second_root,
        "file:first.rs",
        SECOND_RUN,
        "--after",
        &first_id,
        "first chain order",
    );
    assert!(second.status.success());

    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let third_run = uuid::Uuid::now_v7().to_string();
    let fourth_run = uuid::Uuid::now_v7().to_string();
    let third = claim(repository.path(), "file:third.rs", &third_run);
    let third_id = reservation_id(&third);
    let fourth = answered_claim(
        &third_root,
        "file:third.rs",
        &fourth_run,
        "--after",
        &third_id,
        "second chain order",
    );
    assert!(fourth.status.success());

    let ready = board_data(repository.path())["ready_now"]["entries"]
        .as_array()
        .expect("ready set should be an array")
        .clone();
    let ready_ids = ready
        .iter()
        .map(|entry| {
            assert_eq!(entry["relation"], "unordered");
            entry["reservation"]["reservation_id"]
                .as_str()
                .expect("ready row should carry a reservation id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(ready_ids.contains(&first_id));
    assert!(ready_ids.contains(&third_id));
    assert!(
        !serde_json::Value::Array(ready)
            .to_string()
            .contains("number")
    );
}

#[test]
fn every_reachable_edge_readiness_has_its_own_action_or_settlement() {
    let fixture = ordered_fixture();
    assert_waiting_reason(
        &board_data(fixture.repository.path()),
        "predecessor_checkpoint",
        "nobody can act yet",
    );

    fs::write(
        fixture.predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&fixture.predecessor_root, &["add", "src/lib.rs"]);
    git(
        &fixture.predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let released = run_berth_with_run(
        &fixture.predecessor_root,
        &["release", &fixture.predecessor_id, "--json"],
        FIRST_RUN,
    );
    assert!(released.status.success());
    let not_integrated = board_data(fixture.repository.path());
    assert_waiting_reason(
        &not_integrated,
        "predecessor_not_integrated",
        "wait for the predecessor to reach trunk",
    );
    assert_eq!(
        board_reservation_snapshot(&not_integrated, &fixture.predecessor_id)["integration_evidence"]
            ["status"]["status"],
        "not_integrated"
    );

    git(
        fixture.repository.path(),
        &["merge", "--quiet", "--ff-only", "predecessor"],
    );
    assert_waiting_reason(
        &board_data(fixture.repository.path()),
        "successor_must_incorporate_predecessor",
        "reader's own rebase",
    );
    git(
        &fixture.successor_root,
        &["merge", "--quiet", "--ff-only", "main"],
    );
    let fulfilled = board_data(fixture.repository.path());
    assert_eq!(fulfilled["waiting"]["entries"], serde_json::json!([]));
    assert_eq!(
        fulfilled["settled_ordering_constraints"]["entries"][0]["settlement"],
        "fulfilled_successor_contains_predecessor"
    );

    let cancelled_fixture = ordered_fixture();
    let abandoned = run_berth_with_run(
        &cancelled_fixture.predecessor_root,
        &[
            "resolve",
            &cancelled_fixture.predecessor_id,
            "--abandon",
            "--why",
            "the predecessor is deliberately discarded",
            "--json",
        ],
        FIRST_RUN,
    );
    assert!(abandoned.status.success());
    let cancelled = board_data(cancelled_fixture.repository.path());
    assert_eq!(cancelled["waiting"]["entries"], serde_json::json!([]));
    assert_eq!(
        cancelled["settled_ordering_constraints"]["entries"][0]["settlement"],
        "cancelled_constraint_ended"
    );
}

#[test]
fn rewritten_and_unknown_predecessor_evidence_have_distinct_recoveries() {
    let rewritten_fixture = ordered_fixture();
    checkpoint_predecessor(&rewritten_fixture);
    git(
        rewritten_fixture.repository.path(),
        &["commit", "--quiet", "--amend", "-m", "rewritten trunk"],
    );
    let rewritten = board_data(rewritten_fixture.repository.path());
    let rewritten_wait = &rewritten["waiting"]["entries"][0]["action"];
    assert_eq!(rewritten_wait["reason"], "trunk_evidence_rewritten");
    assert_eq!(
        rewritten_wait["resolve_flag"],
        "resolve --integrated-as <trunk-oid>"
    );

    let unknown_fixture = ordered_fixture();
    checkpoint_predecessor(&unknown_fixture);
    // `resolve --integrated-as` correctly rejects an unavailable object, so the protected-tip
    // snapshot is made unavailable directly to exercise the model's `ObjectUnknown` row.
    rewrite_checkpoint_protected_tip(
        unknown_fixture.repository.path(),
        &unknown_fixture.predecessor_id,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let unknown = board_data(unknown_fixture.repository.path());
    assert_waiting_reason(&unknown, "predecessor_object_unknown", "does not resolve");
    assert_eq!(
        board_reservation_snapshot(&unknown, &unknown_fixture.predecessor_id)["integration_evidence"]
            ["status"]["status"],
        "object_unknown"
    );
}

#[test]
fn release_dispositions_remain_resolved_when_trunk_rewrites() {
    let released = released_disposition_fixture();

    let clean = board_data(released.repository.path());
    assert_eq!(
        clean["resolved"]["entries"].as_array().map(Vec::len),
        Some(4)
    );
    assert_release_disposition_recorded(&clean, &released.integrated);
    assert_release_disposition_recorded(&clean, &released.rewritten_integration);
    assert_release_disposition_recorded(&clean, &released.abandoned);
    assert_release_disposition_recorded(&clean, &released.retired_orphan);

    git(
        released.repository.path(),
        &["commit", "--quiet", "--amend", "-m", "rewrite main"],
    );

    let rewritten = board_data(released.repository.path());
    assert_eq!(
        rewritten["resolved"]["entries"].as_array().map(Vec::len),
        Some(4)
    );
    assert_release_disposition_survives_rewrite(&rewritten, &released.integrated);
    assert_release_disposition_survives_rewrite(&rewritten, &released.rewritten_integration);
    assert_release_disposition_survives_rewrite(&rewritten, &released.abandoned);
    assert_release_disposition_survives_rewrite(&rewritten, &released.retired_orphan);
}

#[test]
fn board_json_renders_a_retained_scoped_patch_equivalence_proof() {
    let repository = initialized_repository();
    let reservation_id =
        reservation_id(&claim(repository.path(), "file:scoped-proof.rs", FIRST_RUN));
    let trunk_oid = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "checkpoint",
            "reservation_id": reservation_id,
            "protected_tip": trunk_oid,
            "trunk_snapshot": trunk_oid,
        }),
    );
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "scoped_patch_equivalence_checked",
            "reservation_id": reservation_id,
            "subject": 1,
            "target": trunk_oid,
            "verdict": "integrated",
        }),
    );

    let board = board_data(repository.path());
    let status =
        &board_reservation_snapshot(&board, &reservation_id)["integration_evidence"]["status"];
    assert_eq!(status["status"], "integrated");
    assert_eq!(status["trunk_oid"], trunk_oid);
    assert_eq!(status["proof"], "scoped_patch_equivalent");
}

#[test]
fn renew_is_activity_but_unrelated_events_and_head_movement_are_not() {
    let repository = initialized_repository();
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = tempdir().expect("worktree directory should exist");
    let fresh_root = add_worktree(repository.path(), worktrees.path(), "fresh-holder");
    let stale_root = add_worktree(repository.path(), worktrees.path(), "stale-holder");
    let fresh_id = reservation_id(&claim(&fresh_root, "file:fresh.rs", FIRST_RUN));
    let stale_id = reservation_id(&claim(&stale_root, "file:stale.rs", SECOND_RUN));
    assert_ne!(fresh_id, stale_id);
    let renewed = run_berth_with_run(&fresh_root, &["renew", &fresh_id, "--json"], FIRST_RUN);
    assert!(
        renewed.status.success(),
        "renew failed: {}",
        String::from_utf8_lossy(&renewed.stdout)
    );
    rewrite_operation_times(repository.path(), "claim", "2020-01-01T00:00:00.000Z");
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("freshness journal should read");
    let stale_claim = journal
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should parse"))
        .find(|event| event["op"] == "claim" && event["reservation_id"] == stale_id)
        .expect("stale claim should remain in the journal");
    assert_eq!(stale_claim["at"], "2020-01-01T00:00:00.000Z");
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "bypass",
            "action": "integration",
            "cause": {
                "kind": "environment_override",
                "bypassed_merge": "unrelated-freshness-event",
            },
            "occurrence_time": {"status": "event_recorded_at"},
            "recording": {"kind": "direct"},
        }),
    );
    fs::write(
        stale_root.join("stale.rs"),
        "head movement within the reserved scope\n",
    )
    .expect("reserved file should write");
    git(&stale_root, &["add", "stale.rs"]);
    git(
        &stale_root,
        &["commit", "--quiet", "-m", "unrelated head movement"],
    );

    let data = board_data(repository.path());
    assert_eq!(
        board_reservation_snapshot(&data, &fresh_id)["freshness"]["status"],
        "fresh"
    );
    let stale_row = board_reservation_snapshot(&data, &stale_id);
    assert_eq!(
        stale_row["freshness"]["status"], "stale",
        "stale row: {stale_row}"
    );
    assert!(
        data["alerts"]["entries"]
            .as_array()
            .is_some_and(|alerts| alerts.iter().any(|alert| {
                alert["kind"] == "stale_reservation"
                    && alert["reservation_id"] == stale_id
                    && alert["resolution"]["action"] == "renew"
                    && alert["resolution"]["reservation_id"] == stale_id
            }))
    );
}

#[test]
fn unconstrained_holders_show_all_liveness_states_and_distinct_history_failures() {
    let repository = initialized_repository();
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = tempdir().expect("worktree directory should exist");
    let live_root = add_worktree(repository.path(), worktrees.path(), "live-holder");
    let unavailable_root = add_worktree(repository.path(), worktrees.path(), "unavailable-holder");
    let candidate_root = add_worktree(repository.path(), worktrees.path(), "candidate-holder");
    let orphaned_root = add_worktree(repository.path(), worktrees.path(), "orphaned-holder");
    let unknown_root = add_worktree(repository.path(), worktrees.path(), "unknown-holder");
    let unrelated_root = add_worktree(repository.path(), worktrees.path(), "unrelated-holder");

    let roots = [
        &live_root,
        &unavailable_root,
        &candidate_root,
        &orphaned_root,
        &unknown_root,
    ];
    let reservation_ids = roots
        .iter()
        .enumerate()
        .map(|(index, root)| {
            reservation_id(&claim(
                root,
                &format!("file:liveness-{index}.rs"),
                &uuid::Uuid::now_v7().to_string(),
            ))
        })
        .collect::<Vec<_>>();
    let orphaned_administrative_directory = worktree_administrative_directory(&orphaned_root);
    let unknown_administrative_directory = worktree_administrative_directory(&unknown_root);

    git(
        repository.path(),
        &[
            "worktree",
            "lock",
            unavailable_root
                .to_str()
                .expect("unavailable root should be UTF-8"),
        ],
    );
    fs::remove_dir_all(&candidate_root).expect("candidate root should remove");
    fs::remove_dir_all(&orphaned_root).expect("orphaned root should remove");
    fs::remove_dir_all(orphaned_administrative_directory)
        .expect("orphaned registration should remove");
    fs::remove_file(unknown_administrative_directory.join("cargo-berth-worktree-id"))
        .expect("unknown holder identity should remove");

    git(
        &unrelated_root,
        &["checkout", "--quiet", "--orphan", "unrelated-root"],
    );
    git(&unrelated_root, &["add", "."]);
    git(
        &unrelated_root,
        &["commit", "--quiet", "-m", "unrelated root"],
    );
    let unrelated_id = reservation_id(&claim(
        &unrelated_root,
        "file:unrelated.rs",
        &uuid::Uuid::now_v7().to_string(),
    ));

    let data = board_data(repository.path());
    for (reservation_id, expected_liveness) in reservation_ids.iter().zip([
        "live",
        "unavailable",
        "orphan_candidate",
        "orphaned",
        "unknown",
    ]) {
        assert_eq!(
            board_reservation_snapshot(&data, reservation_id)["holder"]["liveness"],
            expected_liveness
        );
    }
    assert_eq!(
        board_reservation_snapshot(&data, &reservation_ids[3])["ahead_behind_main"]["status"],
        "unavailable"
    );
    assert_eq!(
        board_reservation_snapshot(&data, &unrelated_id)["ahead_behind_main"]["status"],
        "unrelated"
    );
}

fn assert_waiting_reason(data: &serde_json::Value, reason: &str, instruction_fragment: &str) {
    let waiting = data["waiting"]["entries"]
        .as_array()
        .expect("waiting section should be an array");
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["action"]["reason"], reason);
    assert!(
        waiting[0]["action"]["instruction"]
            .as_str()
            .is_some_and(|instruction| instruction.contains(instruction_fragment))
    );
    assert_ne!(waiting[0]["action"], "waiting");
}

#[track_caller]
fn board_reservation_snapshot<'board>(
    data: &'board serde_json::Value,
    reservation_id: &str,
) -> &'board serde_json::Value {
    ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .find(|snapshot| snapshot["reservation_id"] == reservation_id)
        .expect("reservation should have a board snapshot")
}

fn has_reservation_snapshot(data: &serde_json::Value, reservation_id: &str) -> bool {
    ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .any(|snapshot| snapshot["reservation_id"] == reservation_id)
}

fn assert_reservation_lifecycle(
    repository_root: &Path,
    reservation_id: &str,
    expected_status: &str,
    protected_tip: &str,
) {
    let lifecycle = reservation_lifecycle(repository_root, reservation_id);
    assert_eq!(lifecycle["status"], expected_status);
    assert_eq!(lifecycle["protected_tip"], protected_tip);
}

fn reservation_lifecycle(repository_root: &Path, reservation_id: &str) -> serde_json::Value {
    let output = run_berth(
        repository_root,
        &["board", "--reservation", reservation_id, "--json"],
    );
    assert!(
        output.status.success(),
        "reservation lifecycle query failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope = json_output(&output);
    let expected_message = format!("Reservation {reservation_id} lifecycle was read.");
    assert_preserved_board_envelope_fields(
        &envelope,
        &[reservation_id],
        "reservation",
        &expected_message,
    );
    assert_eq!(envelope["verb"], "board");
    assert_eq!(envelope["status"], "board_ready");
    assert_eq!(envelope["exit_code"], 0);
    assert_eq!(
        envelope["reservations"],
        serde_json::json!([reservation_id])
    );
    assert_eq!(envelope["payload"]["kind"], "reservation");
    assert_eq!(
        envelope["payload"]["data"]["reservation_id"],
        reservation_id
    );
    let lifecycle = &envelope["payload"]["data"]["lifecycle"];
    let report = rendered_board_report(&envelope, "reservation lifecycle");
    assert_eq!(
        report,
        serde_json::json!({
            "Reservation": reservation_id,
            "Lifecycle": lifecycle,
        })
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains(ENTER_ALTERNATE_SCREEN));
    lifecycle.clone()
}

fn assert_preserved_board_envelope_fields(
    envelope: &serde_json::Value,
    expected_reservations: &[&str],
    expected_payload_kind: &str,
    expected_message: &str,
) {
    let object = envelope
        .as_object()
        .expect("board response should be a JSON object");
    let mut field_names = object.keys().map(String::as_str).collect::<Vec<_>>();
    field_names.sort_unstable();
    assert_eq!(
        field_names,
        [
            "blocked_by",
            "exit_code",
            "message",
            "output_contract_version",
            "payload",
            "presentation",
            "reservations",
            "status",
            "verb",
        ]
    );
    assert_eq!(envelope["output_contract_version"], 2);
    assert_eq!(envelope["verb"], "board");
    assert_eq!(envelope["status"], "board_ready");
    assert_eq!(envelope["exit_code"], 0);
    let mut sorted_expected_reservations = expected_reservations.to_vec();
    sorted_expected_reservations.sort_unstable();
    assert_eq!(
        envelope["reservations"],
        serde_json::json!(sorted_expected_reservations)
    );
    assert_eq!(envelope["blocked_by"], serde_json::json!([]));
    assert_eq!(envelope["message"], expected_message);
    let payload = envelope["payload"]
        .as_object()
        .expect("board payload should remain an object");
    let mut payload_field_names = payload.keys().map(String::as_str).collect::<Vec<_>>();
    payload_field_names.sort_unstable();
    assert_eq!(payload_field_names, ["alerts", "data", "kind"]);
    assert_eq!(envelope["payload"]["kind"], expected_payload_kind);
    assert_eq!(envelope["payload"]["alerts"], serde_json::json!([]));
}

fn assert_complete_board_payload_sections(data: &serde_json::Value) {
    let object = data
        .as_object()
        .expect("complete board payload should be a JSON object");
    let mut field_names = object.keys().map(String::as_str).collect::<Vec<_>>();
    field_names.sort_unstable();
    assert_eq!(
        field_names,
        [
            "alerts",
            "available_forced_permits",
            "bypass_audit",
            "git_cost",
            "integration_order",
            "journal_position",
            "outstanding_incursions",
            "ready_now",
            "recorded_incursion_answers",
            "recorded_overlap_answers",
            "recovered_bypasses_this_invocation",
            "resolved",
            "settled_ordering_constraints",
            "unconstrained_reservations",
            "unresolved_overlaps",
            "waiting",
        ]
    );
    let journal_position = &data["journal_position"];
    for section_name in [
        "ready_now",
        "waiting",
        "settled_ordering_constraints",
        "unresolved_overlaps",
        "recorded_overlap_answers",
        "unconstrained_reservations",
        "resolved",
        "available_forced_permits",
        "bypass_audit",
        "outstanding_incursions",
        "recorded_incursion_answers",
        "alerts",
    ] {
        assert_eq!(
            data[section_name]["journal_position"], *journal_position,
            "{section_name} moved off the complete board's locked journal position"
        );
        assert!(
            data[section_name]["entries"].is_array(),
            "{section_name} entries are no longer a JSON array"
        );
    }
    let git_cost = data["git_cost"]
        .as_object()
        .expect("board git cost should remain an object");
    let mut git_cost_fields = git_cost.keys().map(String::as_str).collect::<Vec<_>>();
    git_cost_fields.sort_unstable();
    assert_eq!(
        git_cost_fields,
        [
            "orphan_recovery_evidence_queries",
            "protected_predecessor_ancestry_queries",
            "reservation_evidence_revalidations",
            "trunk_resolution_calls",
            "worktree_ahead_behind_computations",
            "worktree_list_calls",
        ]
    );
}

fn rendered_board_report(envelope: &serde_json::Value, report_kind: &str) -> serde_json::Value {
    assert_eq!(
        envelope["presentation"]["kind"], "rendered_blocks",
        "{report_kind} presentation was not provided"
    );
    let blocks = envelope["presentation"]["blocks"]
        .as_array()
        .expect("rendered board presentation should carry blocks");
    assert_eq!(
        blocks.len(),
        1,
        "{report_kind} presentation should carry exactly one block"
    );
    let block = blocks
        .first()
        .expect("one rendered board presentation block should exist");
    let summary = block["summary"]
        .as_str()
        .expect("rendered board summary should be text");
    assert!(!summary.is_empty());
    assert!(!summary.contains('\n'));
    let detail = block["detail"]
        .as_str()
        .expect("rendered board detail should be text");
    assert!(!detail.is_empty());
    serde_json::from_str(detail).expect("rendered board detail should be valid JSON")
}

fn assert_integration_statuses(
    data: &serde_json::Value,
    reservation_ids: &[String],
    expected_status: &str,
) {
    for reservation_id in reservation_ids {
        assert_eq!(
            board_reservation_snapshot(data, reservation_id)["integration_evidence"]["status"]["status"],
            expected_status
        );
    }
}

fn assert_not_integrated_and_blocking(data: &serde_json::Value, reservation_id: &str) {
    let reservation = board_reservation_snapshot(data, reservation_id);
    assert_eq!(
        reservation["integration_evidence"]["status"]["status"],
        "not_integrated"
    );
    assert_eq!(reservation["edit_blocking_status"], "blocking");
}

#[test]
fn pending_environment_bypass_is_imported_once_and_unknown_time_stays_unknown() {
    let repository = initialized_repository();
    let marker_path = repository.path().join(".git").join(PENDING_BYPASS_NAME);
    fs::write(
        &marker_path,
        r#"{"cause":{"kind":"environment_override","bypassed_merge":"marker-test-merge"},"occurrence_time":{"status":"unavailable"}}
"#,
    )
    .expect("pending bypass marker should write");

    let first = json_output(&run_berth(repository.path(), &["board", "--json"]));
    assert!(!marker_path.exists());
    assert_eq!(
        first["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([PENDING_BYPASS_NAME])
    );
    let bypasses = first["payload"]["data"]["bypass_audit"]["entries"]
        .as_array()
        .expect("bypass audit should be an array");
    assert_eq!(bypasses.len(), 1);
    assert_eq!(bypasses[0]["kind"], "environment_override");
    assert_eq!(bypasses[0]["occurrences"][0]["status"], "unknown");
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);

    let second = json_output(&run_berth(repository.path(), &["board", "--json"]));
    assert_eq!(
        second["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
    assert_eq!(
        second["payload"]["data"]["bypass_audit"]["entries"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);
}

#[test]
fn human_board_names_a_recovered_bypass_once() {
    let repository = initialized_repository();
    let marker_path = repository.path().join(".git").join(PENDING_BYPASS_NAME);
    fs::write(
        &marker_path,
        r#"{"cause":{"kind":"environment_override","bypassed_merge":"human-recovery"},"occurrence_time":{"status":"unavailable"}}
"#,
    )
    .expect("pending bypass marker should write");

    let recovered = run_berth(repository.path(), &["board"]);
    assert!(recovered.status.success());
    assert_eq!(
        String::from_utf8_lossy(&recovered.stdout).trim(),
        format!(
            "{BOARD_READY_MESSAGE}\nRecovered bypass marker {PENDING_BYPASS_NAME}: a bypass recorded earlier while the journal was unwritable has now been filed in the journal."
        )
    );
    assert!(!marker_path.exists());

    let later = run_berth(repository.path(), &["board"]);
    assert!(later.status.success());
    assert_eq!(
        String::from_utf8_lossy(&later.stdout).trim(),
        BOARD_READY_MESSAGE
    );
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);
}

#[test]
fn non_board_reconciliation_preserves_recovered_bypass_for_board() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let claimed = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    assert!(claimed.status.success());
    let marker_path = repository.path().join(".git").join(PENDING_BYPASS_NAME);
    fs::write(
        &marker_path,
        r#"{"cause":{"kind":"environment_override","bypassed_merge":"non-board-recovery"},"occurrence_time":{"status":"unavailable"}}
"#,
    )
    .expect("pending bypass marker should write");

    let check = run_berth_with_run(
        &second_root,
        &["check", "file:src/lib.rs", "--json"],
        SECOND_RUN,
    );
    assert_eq!(check.status.code(), Some(1));
    assert!(marker_path.exists());
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);

    let reporting_board = board_data(repository.path());
    assert_eq!(
        reporting_board["recovered_bypasses_this_invocation"],
        serde_json::json!([PENDING_BYPASS_NAME])
    );
    assert!(!marker_path.exists());
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);

    let later_board = board_data(repository.path());
    assert_eq!(
        later_board["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
    assert!(!marker_path.exists());
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);
}

#[test]
fn environment_bypasses_group_by_merge_identity_not_coordination_run() {
    let repository = initialized_repository();
    let claim_output = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    assert!(claim_output.status.success());
    for bypassed_merge in ["first-merge", "first-merge", "first-merge", "second-merge"] {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "bypass",
                "action": "integration",
                "cause": {
                    "kind": "environment_override",
                    "bypassed_merge": bypassed_merge,
                },
                "occurrence_time": {"status": "event_recorded_at"},
                "recording": {"kind": "direct"},
            }),
        );
    }

    let bypasses = board_data(repository.path())["bypass_audit"]["entries"]
        .as_array()
        .expect("bypass audit should be an array")
        .clone();
    assert_eq!(bypasses.len(), 2);
    let mut grouped_counts = bypasses
        .iter()
        .map(|entry| {
            assert_eq!(entry["kind"], "environment_override");
            assert_eq!(entry["override_name"], "CARGO_BERTH_BYPASS=1");
            assert!(entry.get("reason").is_none());
            assert_eq!(entry["skipped_holds"], "override_preceded_ledger_read");
            entry["grouped_reference_transactions"]
                .as_u64()
                .expect("group count should be numeric")
        })
        .collect::<Vec<_>>();
    grouped_counts.sort_unstable();
    assert_eq!(grouped_counts, [1, 3]);
}

#[test]
fn forced_bypass_rows_distinguish_edges_deferrals_and_both() {
    let repository = initialized_repository();
    let reservation = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&reservation);
    let predecessor_id = reservation_id.clone();
    let edge_id = uuid::Uuid::now_v7().to_string();
    let deferral_event_id = uuid::Uuid::now_v7().to_string();

    for (kind, skipped_holds) in [
        (
            "forced_ordering_edges",
            serde_json::json!({
                "kind": "ordering_edges",
                "edges": [{"edge_id": edge_id, "predecessor": predecessor_id}],
            }),
        ),
        (
            "forced_unresolved_deferrals",
            serde_json::json!({
                "kind": "deferrals",
                "deferrals": [{
                    "declaration_event_id": deferral_event_id,
                    "deferred": reservation_id,
                    "blocker": predecessor_id,
                }],
            }),
        ),
        (
            "forced_edges_and_deferrals",
            serde_json::json!({
                "kind": "ordering_edges_and_deferrals",
                "edges": [{"edge_id": edge_id, "predecessor": predecessor_id}],
                "deferrals": [{
                    "declaration_event_id": deferral_event_id,
                    "deferred": reservation_id,
                    "blocker": predecessor_id,
                }],
            }),
        ),
    ] {
        let permit_id = uuid::Uuid::now_v7().to_string();
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "forced_integration_permit",
                "permit_id": permit_id,
                "reservation_id": reservation_id,
                "reason": format!("approved {kind}"),
                "skipped_holds": skipped_holds,
            }),
        );
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "consume_forced_integration_permit",
                "permit_id": permit_id,
                "reservation_id": reservation_id,
            }),
        );
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "bypass",
                "action": "integration",
                "cause": {
                    "kind": "forced_integration",
                    "permit_id": permit_id,
                    "reason": format!("approved {kind}"),
                },
                "occurrence_time": {"status": "event_recorded_at"},
                "recording": {"kind": "direct"},
            }),
        );
    }

    let entries = board_data(repository.path())["bypass_audit"]["entries"]
        .as_array()
        .expect("bypass rows should be an array")
        .clone();
    for expected_kind in [
        "forced_ordering_edges",
        "forced_unresolved_deferrals",
        "forced_edges_and_deferrals",
    ] {
        let row = entries
            .iter()
            .find(|entry| entry["kind"] == expected_kind)
            .expect("each forced bypass kind should render");
        assert!(
            row["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
        );
        if expected_kind != "forced_unresolved_deferrals" {
            assert_eq!(row["skipped_edges"][0]["predecessor"], predecessor_id);
        }
        if expected_kind != "forced_ordering_edges" {
            assert_eq!(row["skipped_deferrals"][0]["blocker"], predecessor_id);
        }
    }
}

#[test]
fn available_forced_permit_becomes_consumed_bypass_history() {
    // Git-abort behavior is covered in `tests/gate.rs`; this board fixture starts at the
    // resulting durable permit because the board cannot observe an aborted git process itself.
    let repository = initialized_repository();
    let reservation = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&reservation);
    let permit_id = uuid::Uuid::now_v7().to_string();
    let predecessor = reservation_id.clone();
    let skipped_holds = serde_json::json!({
        "kind": "ordering_edges",
        "edges": [{
            "edge_id": uuid::Uuid::now_v7().to_string(),
            "predecessor": predecessor,
        }],
    });
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "forced_integration_permit",
            "permit_id": permit_id,
            "reservation_id": reservation_id,
            "reason": "retry remains authorized",
            "skipped_holds": skipped_holds,
        }),
    );

    let available = board_data(repository.path());
    let permit = &available["available_forced_permits"]["entries"][0];
    assert_eq!(permit["permit_id"], permit_id);
    assert!(permit["instruction"].as_str().is_some_and(|instruction| {
        instruction.contains("retrying") && instruction.contains("consume")
    }));
    assert_eq!(available["bypass_audit"]["entries"], serde_json::json!([]));

    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "consume_forced_integration_permit",
            "permit_id": permit_id,
            "reservation_id": reservation_id,
        }),
    );
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "bypass",
            "action": "integration",
            "cause": {
                "kind": "forced_integration",
                "permit_id": permit_id,
                "reason": "retry remains authorized",
            },
            "occurrence_time": {"status": "event_recorded_at"},
            "recording": {"kind": "direct"},
        }),
    );
    let consumed = board_data(repository.path());
    assert_eq!(
        consumed["available_forced_permits"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(
        consumed["bypass_audit"]["entries"][0]["kind"],
        "forced_ordering_edges"
    );
}

#[test]
fn interrupted_marker_import_is_deduplicated_before_deletion() {
    let repository = initialized_repository();
    let reservation = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    assert!(reservation.status.success());
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "bypass",
            "action": "integration",
            "cause": {
                "kind": "environment_override",
                "bypassed_merge": "interrupted-import",
            },
            "occurrence_time": {"status": "unavailable"},
            "recording": {
                "kind": "pending_marker",
                "marker_id": PENDING_BYPASS_NAME,
            },
        }),
    );
    let marker_path = repository.path().join(".git").join(PENDING_BYPASS_NAME);
    fs::write(
        &marker_path,
        r#"{"cause":{"kind":"environment_override","bypassed_merge":"interrupted-import"},"occurrence_time":{"status":"unavailable"}}
"#,
    )
    .expect("pending marker should write");

    let board = board_data(repository.path());
    assert!(!marker_path.exists());
    assert_eq!(
        board["recovered_bypasses_this_invocation"],
        serde_json::json!([PENDING_BYPASS_NAME])
    );
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 1);
    assert_eq!(
        board["bypass_audit"]["entries"].as_array().map(Vec::len),
        Some(1)
    );
}

#[test]
fn unappendable_decoded_marker_stays_visible_as_a_timed_alert() {
    let repository = initialized_repository();
    let reservation = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    assert!(reservation.status.success());
    let marker_path = repository.path().join(".git").join(PENDING_BYPASS_NAME);
    let bypassed_merge = "x".repeat(17_000);
    fs::write(
        &marker_path,
        serde_json::to_vec(&serde_json::json!({
            "cause": {
                "kind": "environment_override",
                "bypassed_merge": bypassed_merge,
            },
            "occurrence_time": {
                "status": "known",
                "at": "2026-08-24T12:00:00.000Z",
            },
        }))
        .expect("oversized marker should serialize"),
    )
    .expect("oversized marker should write");

    let board = board_data(repository.path());
    assert!(marker_path.exists());
    assert_eq!(
        board["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
    assert_eq!(journal_operation_count(repository.path(), "bypass"), 0);
    let alert = board["alerts"]["entries"]
        .as_array()
        .and_then(|alerts| {
            alerts
                .iter()
                .find(|alert| alert["kind"] == "unrecorded_bypasses")
        })
        .expect("failed marker import should render an alert");
    assert_eq!(alert["count"], 1);
    assert_eq!(
        alert["occurrence_times"][0],
        serde_json::json!({"status": "known", "at": "2026-08-24T12:00:00.000Z"})
    );
}

#[test]
fn incursion_is_one_shared_incident_then_moves_to_answer_audit() {
    let repository = initialized_repository();
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = tempdir().expect("worktree directory should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "foreign");
    let subject = claim(repository.path(), "file:owned.txt", FIRST_RUN);
    let subject_id = reservation_id(&subject);
    let foreign = claim(&foreign_root, "tree:shared", SECOND_RUN);
    let foreign_id = reservation_id(&foreign);
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(repository.path().join("shared/entered.txt"), "incursion\n")
        .expect("incursion path should write");

    let first = run_berth_with_run(
        repository.path(),
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        FIRST_RUN,
    );
    assert_eq!(first.status.code(), Some(1));
    let repeated = run_berth_with_run(
        repository.path(),
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        FIRST_RUN,
    );
    assert_eq!(repeated.status.code(), Some(1));

    let from_straying = board_data(repository.path());
    let from_entered = board_data(&foreign_root);
    assert_eq!(
        from_straying["outstanding_incursions"],
        from_entered["outstanding_incursions"]
    );
    let incidents = from_straying["outstanding_incursions"]["entries"]
        .as_array()
        .expect("incursion section should be an array");
    assert_eq!(incidents.len(), 1);
    let incident = &incidents[0];
    assert_eq!(incident["straying_reservation_id"], subject_id);
    assert_eq!(
        incident["foreign_reservation_ids"],
        serde_json::json!([foreign_id])
    );
    assert_eq!(
        incident["entered_paths"],
        serde_json::json!(["shared/entered.txt"])
    );
    let incident_id = incident["incident_id"]
        .as_str()
        .expect("incident should carry an id")
        .to_owned();
    assert_eq!(
        incident["resolution"]["flag"],
        format!("resolve {subject_id} --incursion {incident_id}")
    );

    let resolved = run_berth_with_run(
        repository.path(),
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
        FIRST_RUN,
    );
    assert!(resolved.status.success());
    let answered = board_data(repository.path());
    assert_eq!(
        answered["outstanding_incursions"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(
        answered["recorded_incursion_answers"]["entries"][0]["incident_id"],
        incident_id
    );
}

#[test]
fn drift_widen_audit_names_existing_coverage_without_new_ordering() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(repository.path(), "tree:shared", FIRST_RUN);
    let holder_id = reservation_id(&holder);
    let subject = answered_claim(
        &second_root,
        "file:shared/approved.txt",
        SECOND_RUN,
        "--override",
        &holder_id,
        "the approved file can overlap",
    );
    let subject_id = reservation_id(&subject);
    fs::write(repository.path().join("outside.txt"), "new scope\n")
        .expect("new scope should write");
    let widened = run_berth_with_run(
        &second_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );
    assert!(widened.status.success());

    let data = board_data(repository.path());
    let answers = data["recorded_overlap_answers"]["entries"]
        .as_array()
        .expect("answer audit should be an array");
    let widen = answers
        .iter()
        .find(|answer| answer["answer"] == "existing_answers_cover_every_overlap")
        .expect("drift widen should have its own audit row");
    assert_eq!(widen["reservation_id"], subject_id);
    assert_eq!(widen["cause"]["kind"], "drift");
    assert_eq!(widen["edit_blocking_status"], "blocking");
    assert_eq!(
        widen["exact_existing_bindings"][0]["reservation_id"],
        holder_id
    );
    assert_eq!(
        widen["exact_existing_bindings"][0]["scopes"][0]["path"],
        "shared/approved.txt"
    );
    assert_eq!(
        data["settled_ordering_constraints"]["entries"],
        serde_json::json!([])
    );
    assert_eq!(data["waiting"]["entries"], serde_json::json!([]));
    assert_eq!(
        answers
            .iter()
            .filter(|answer| answer["answer"] == "existing_answers_cover_every_overlap")
            .count(),
        1
    );
    let journal =
        fs::read_to_string(repository.path().join(JOURNAL_PATH)).expect("journal should read");
    let widen_event = journal
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should parse"))
        .find(|event| event["op"] == "widen" && event["reservation_id"] == subject_id)
        .expect("drift widen should be journalled");
    assert_eq!(
        widen_event["authorization"]["kind"],
        "existing_answers_cover_every_overlap"
    );
}

#[test]
fn transaction_only_post_commit_actor_never_becomes_a_holder_or_orphan() {
    // A markerless post-commit actor is transaction-only and has no public CLI identity.
    let repository = initialized_repository();
    let subject_id = reservation_id(&claim(repository.path(), "file:subject.rs", FIRST_RUN));
    let foreign_id = reservation_id(&claim(repository.path(), "file:foreign.rs", SECOND_RUN));
    let transaction_worktree = uuid::Uuid::now_v7().to_string();
    let transaction_run = uuid::Uuid::now_v7().to_string();
    append_journal_operation_with_actor(
        repository.path(),
        &serde_json::json!({
            "op": "incursion",
            "incident_id": uuid::Uuid::now_v7().to_string(),
            "reservation_id": subject_id,
            "foreign_reservation_ids": [foreign_id],
            "paths": ["foreign.rs"],
        }),
        ActorFixture::TransactionOnly {
            worktree_id: transaction_worktree.clone(),
            run_id:      transaction_run.clone(),
        },
    );
    append_journal_operation_with_actor(
        repository.path(),
        &serde_json::json!({
            "op": "widen",
            "reservation_id": subject_id,
            "added_scopes": [{"path": "added.rs", "kind": "file"}],
            "cause": {"kind": "drift"},
            "authorization": {"kind": "no_conflict"},
            "edit_blocking_status": "blocking",
        }),
        ActorFixture::TransactionOnly {
            worktree_id: transaction_worktree.clone(),
            run_id:      transaction_run.clone(),
        },
    );

    let data = board_data(repository.path());
    assert!(!data.to_string().contains(&transaction_worktree));
    assert!(!data.to_string().contains(&transaction_run));
    assert!(data["alerts"]["entries"].as_array().is_some_and(|alerts| {
        alerts
            .iter()
            .all(|alert| alert["kind"] != "orphaned_outstanding")
    }));
    assert_eq!(
        data["recorded_overlap_answers"]["entries"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| entry["answer"] == "widen_without_foreign_overlap")
                    .count()
            }),
        Some(1)
    );
}

#[test]
fn incursion_only_post_commit_runs_add_no_invented_widening_row() {
    // Replay records incidents but no refused-or-unneeded widening outcome, so the answer rows
    // before and after these two incident reports must be identical.
    let repository = initialized_repository();
    let subject_id = reservation_id(&claim(repository.path(), "file:subject.rs", FIRST_RUN));
    let first_foreign = reservation_id(&claim(
        repository.path(),
        "file:first-foreign.rs",
        SECOND_RUN,
    ));
    let second_foreign = reservation_id(&claim(
        repository.path(),
        "file:second-foreign.rs",
        &uuid::Uuid::now_v7().to_string(),
    ));
    let before = board_data(repository.path())["recorded_overlap_answers"]["entries"].clone();
    for (foreign_id, path) in [
        (&first_foreign, "first-foreign.rs"),
        (&second_foreign, "second-foreign.rs"),
    ] {
        append_journal_operation_with_actor(
            repository.path(),
            &serde_json::json!({
                "op": "incursion",
                "incident_id": uuid::Uuid::now_v7().to_string(),
                "reservation_id": subject_id,
                "foreign_reservation_ids": [foreign_id],
                "paths": [path],
            }),
            ActorFixture::TransactionOnly {
                worktree_id: uuid::Uuid::now_v7().to_string(),
                run_id:      uuid::Uuid::now_v7().to_string(),
            },
        );
    }
    let data = board_data(repository.path());
    assert_eq!(
        data["outstanding_incursions"]["entries"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(data["recorded_overlap_answers"]["entries"], before);
    assert!(!data.to_string().contains("ambiguous"));
    assert!(!data.to_string().contains("coordination_run_required"));
}

#[test]
fn board_git_cost_separates_each_scaling_dimension() {
    let repository = initialized_repository();
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = tempdir().expect("worktree directory should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let predecessor = claim(&predecessor_root, "file:src/lib.rs", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    assert!(
        run_berth(
            repository.path(),
            &[
                "sequence",
                &predecessor_id,
                &successor_id,
                "--why",
                "predecessor first",
                "--json",
            ],
        )
        .status
        .success()
    );
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "src/lib.rs"]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );

    let traced = run_board_with_git_trace(repository.path());
    assert!(traced.output.status.success());
    let board = json_output(&traced.output);
    let cost = &board["payload"]["data"]["git_cost"];
    assert_eq!(cost["worktree_list_calls"], 1);
    assert_eq!(cost["reservation_evidence_revalidations"], 1);
    assert_eq!(cost["protected_predecessor_ancestry_queries"], 1);
    assert_eq!(cost["trunk_resolution_calls"], 1);
    assert_eq!(cost["worktree_ahead_behind_computations"], 1);
    assert_eq!(cost["orphan_recovery_evidence_queries"], 0);

    let trace = fs::read_to_string(traced.trace_path).expect("git trace should read");
    assert_eq!(
        trace
            .lines()
            .filter(|line| *line == "cat-file --batch-check=%(objectname) %(objecttype)")
            .count(),
        1
    );
    assert_eq!(
        trace
            .lines()
            .filter(|line| line.starts_with("worktree list"))
            .count(),
        1
    );
    assert_batched_evidence_and_retention_queries(&trace);
    assert_eq!(
        trace
            .lines()
            .filter(|line| *line == "rev-list --ignore-missing --parents --stdin")
            .count(),
        1
    );
    assert_eq!(
        trace
            .lines()
            .filter(|line| *line == "rev-list --parents --ignore-missing --stdin")
            .count(),
        1
    );
}

#[test]
fn retained_scoped_patch_verdicts_reuse_positive_results_after_process_restart() {
    let positive =
        rewritten_reservation_fixture(TargetRewrite::Equivalent, ReservationCompletion::Released);
    let first_positive = run_board_with_git_trace(positive.repository.path());
    assert!(first_positive.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &first_positive,
            &positive.phase_start_head,
            &positive.target,
        ),
        1
    );
    let positive_board = json_output(&first_positive.output);
    let positive_status = &board_reservation_snapshot(
        &positive_board["payload"]["data"],
        &positive.reservation_id,
    )["integration_evidence"]["status"];
    assert_eq!(positive_status["status"], "integrated");
    assert_eq!(positive_status["proof"], "scoped_patch_equivalent");
    assert_eq!(
        journal_operation_count(
            positive.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        1
    );

    for _ in 0..20 {
        let restarted_positive = run_board_with_git_trace(positive.repository.path());
        assert!(restarted_positive.output.status.success());
        assert_eq!(
            scoped_patch_comparison_attempts(
                &restarted_positive,
                &positive.phase_start_head,
                &positive.target,
            ),
            0
        );
    }

    git(
        positive.repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "second target",
        ],
    );
    let later_target = git_stdout(positive.repository.path(), &["rev-parse", "HEAD"]);
    let moved_positive = run_board_with_git_trace(positive.repository.path());
    assert!(moved_positive.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &moved_positive,
            &positive.phase_start_head,
            &later_target,
        ),
        1
    );
    assert_eq!(
        journal_operation_count(
            positive.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        2
    );
}

#[test]
fn retained_scoped_patch_verdicts_reuse_negative_results_after_process_restart() {
    let negative =
        rewritten_reservation_fixture(TargetRewrite::Different, ReservationCompletion::Released);
    let first_negative = run_board_with_git_trace(negative.repository.path());
    assert!(first_negative.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &first_negative,
            &negative.phase_start_head,
            &negative.target,
        ),
        1
    );
    let negative_board = json_output(&first_negative.output);
    assert_eq!(
        board_reservation_snapshot(&negative_board["payload"]["data"], &negative.reservation_id,)["integration_evidence"]
            ["status"]["status"],
        "trunk_rewritten"
    );
    assert_eq!(
        journal_operation_count(
            negative.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        1
    );

    let restarted_negative = run_board_with_git_trace(negative.repository.path());
    assert!(restarted_negative.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &restarted_negative,
            &negative.phase_start_head,
            &negative.target,
        ),
        0
    );

    git(
        negative.repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "second negative target",
        ],
    );
    let later_negative_target = git_stdout(negative.repository.path(), &["rev-parse", "HEAD"]);
    let moved_negative = run_board_with_git_trace(negative.repository.path());
    assert!(moved_negative.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &moved_negative,
            &negative.phase_start_head,
            &later_negative_target,
        ),
        1
    );
    assert_eq!(
        journal_operation_count(
            negative.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        2
    );
    let restarted_moved_negative = run_board_with_git_trace(negative.repository.path());
    assert!(restarted_moved_negative.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &restarted_moved_negative,
            &negative.phase_start_head,
            &later_negative_target,
        ),
        0
    );
}

#[test]
fn scoped_patch_scheduling_record_follows_its_materialized_evidence() {
    let fixture =
        rewritten_reservation_fixture(TargetRewrite::Different, ReservationCompletion::Released);
    let before = journal_record_count(fixture.repository.path());
    let reconciled = run_board_with_git_trace(fixture.repository.path());
    assert!(reconciled.output.status.success());
    let appended_operations = fs::read_to_string(fixture.repository.path().join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .skip(before)
        .map(|record| {
            serde_json::from_str::<serde_json::Value>(record).expect("journal record should parse")
        })
        .map(|record| {
            record["op"]
                .as_str()
                .expect("operation should be named")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let evidence_index = appended_operations
        .iter()
        .position(|operation| operation == "evidence_revalidated")
        .expect("changed evidence should be journaled");
    let scheduling_index = appended_operations
        .iter()
        .position(|operation| operation == "scoped_patch_equivalence_checked")
        .expect("scoped verdict should be journaled");
    assert!(evidence_index < scheduling_index);
}

#[test]
fn persistent_object_unknown_records_one_attempt_without_starving_other_subjects() {
    let fixture = persistent_unavailable_comparison_fixture();
    let first = run_board_with_git_trace(fixture.repository.path());
    assert!(first.output.status.success());
    assert_eq!(merge_base_ancestor_invocations(&first), 0);
    assert_eq!(
        board_reservation_snapshot(
            &json_output(&first.output)["payload"]["data"],
            &fixture.reservation_ids[0],
        )["integration_evidence"]["status"]["status"],
        "object_unknown"
    );
    let mut unavailable_comparisons =
        scoped_patch_comparison_attempts(&first, fixture.unavailable_phase_start, &fixture.target);

    for _ in &fixture.reservation_ids[1..] {
        let traced = run_board_with_git_trace(fixture.repository.path());
        assert!(traced.output.status.success());
        assert_eq!(merge_base_ancestor_invocations(&traced), 0);
        unavailable_comparisons += scoped_patch_comparison_attempts(
            &traced,
            fixture.unavailable_phase_start,
            &fixture.target,
        );
        assert_eq!(
            board_reservation_snapshot(
                &json_output(&traced.output)["payload"]["data"],
                &fixture.reservation_ids[0],
            )["integration_evidence"]["status"]["status"],
            "object_unknown"
        );
    }
    for _ in 0..8 {
        let traced = run_board_with_git_trace(fixture.repository.path());
        assert!(traced.output.status.success());
        assert_eq!(merge_base_ancestor_invocations(&traced), 0);
        unavailable_comparisons += scoped_patch_comparison_attempts(
            &traced,
            fixture.unavailable_phase_start,
            &fixture.target,
        );
        assert_eq!(
            board_reservation_snapshot(
                &json_output(&traced.output)["payload"]["data"],
                &fixture.reservation_ids[0],
            )["integration_evidence"]["status"]["status"],
            "object_unknown"
        );
    }
    assert!(unavailable_comparisons > 1);
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "scoped_patch_comparison_attempted"
        ),
        1
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        fixture.reservation_ids.len() - 1
    );
    assert_available_subjects_compared(&fixture);

    let board = board_data(fixture.repository.path());
    assert_integration_statuses(&board, &fixture.reservation_ids[..1], "object_unknown");
    assert_integration_statuses(&board, &fixture.reservation_ids[1..], "not_integrated");
}

#[test]
fn proof_subject_changes_force_rechecks_at_an_unchanged_target() {
    for target_rewrite in [TargetRewrite::Equivalent, TargetRewrite::Different] {
        for proof_subject_change in [
            ProofSubjectChange::Widen,
            ProofSubjectChange::Resnapshot,
            ProofSubjectChange::ReleaseDispositionReplacement,
        ] {
            assert_proof_subject_change_rechecks(target_rewrite, proof_subject_change);
        }
    }
}

#[test]
fn reachability_integrates_every_outstanding_subject_without_scoped_comparisons() {
    let one = reachable_outstanding_reservations_fixture(1);
    let one_trace = run_board_with_git_trace(one.repository.path());
    assert!(one_trace.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(&one_trace, &one.phase_start_head, &one.target),
        0
    );
    assert_integration_statuses(
        &json_output(&one_trace.output)["payload"]["data"],
        &one.reservation_ids,
        "integrated",
    );

    let several = reachable_outstanding_reservations_fixture(4);
    let several_trace = run_board_with_git_trace(several.repository.path());
    assert!(several_trace.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &several_trace,
            &several.phase_start_head,
            &several.target,
        ),
        0
    );
    let several_data = &json_output(&several_trace.output)["payload"]["data"];
    assert_integration_statuses(several_data, &several.reservation_ids, "integrated");
    for reservation_id in &several.reservation_ids {
        assert_eq!(
            board_reservation_snapshot(several_data, reservation_id)["integration_evidence"]["status"]
                ["proof"],
            "protected_tip_ancestor"
        );
    }
}

#[test]
fn deferred_comparison_rejects_a_refuted_ancestor_proof() {
    let fixture = warmed_ancestor_proof_after_trunk_rewrite();
    let competing_reservations =
        append_released_reservations(&fixture, 1, ProofSubjectSimilarity::Distinct);
    append_scoped_patch_attempt(
        fixture.repository.path(),
        &fixture.reservation_id,
        &fixture.target,
    );

    let traced = run_board_with_git_trace(fixture.repository.path());
    assert!(traced.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(&traced, &fixture.phase_start_head, &fixture.target),
        0
    );
    let board = json_output(&traced.output);
    let data = &board["payload"]["data"];
    assert_integration_statuses(
        data,
        std::slice::from_ref(&fixture.reservation_id),
        "not_integrated",
    );
    let reservation_snapshot = board_reservation_snapshot(data, &fixture.reservation_id);
    assert_eq!(reservation_snapshot["edit_blocking_status"], "clear");
    let alert = data["alerts"]["entries"]
        .as_array()
        .and_then(|alerts| {
            alerts.iter().find(|alert| {
                alert["kind"] == "lost_integration_evidence"
                    && alert["reservation_id"] == fixture.reservation_id
            })
        })
        .expect("the first reconciled board should report lost integration evidence");
    assert_eq!(alert["evidence_status"]["status"], "not_integrated");
    assert_eq!(alert["recovery"]["kind"], "verify_resolved_trunk");
    assert_eq!(alert["recovery"]["trunk_oid"], fixture.target);
    assert_eq!(
        alert["recovery"]["action"]["action"],
        "resolve_integrated_as"
    );
    assert_eq!(
        alert["recovery"]["action"]["reservation_id"],
        fixture.reservation_id
    );
    assert_integration_statuses(data, &competing_reservations, "trunk_rewritten");

    append_released_reservations(&fixture, 1, ProofSubjectSimilarity::Distinct);
    append_scoped_patch_attempt(
        fixture.repository.path(),
        &fixture.reservation_id,
        &fixture.target,
    );
    let recovered = run_berth(
        fixture.repository.path(),
        &[
            "resolve",
            &fixture.reservation_id,
            "--integrated-as",
            &fixture.target,
            "--json",
        ],
    );
    assert!(
        recovered.status.success(),
        "recovery failed: stdout={} stderr={}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr)
    );
    let recovered_envelope = json_output(&recovered);
    assert_eq!(recovered_envelope["status"], "integrated");
    let latest_evidence = fs::read_to_string(fixture.repository.path().join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .rfind(|event| {
            event["op"] == "evidence_revalidated"
                && event["reservation_id"] == fixture.reservation_id
        })
        .expect("the recovered reservation should retain materialized evidence");
    assert_eq!(latest_evidence["status"]["status"], "not_integrated");
    assert_eq!(
        journal_operation_count_for_reservation(
            fixture.repository.path(),
            "replace_release_disposition",
            &fixture.reservation_id,
        ),
        1
    );
}

#[test]
fn lost_evidence_alert_covers_an_unknown_protected_tip() {
    let unknown_tip_repository = initialized_repository();
    let unknown_tip_id = reservation_id(&claim(
        unknown_tip_repository.path(),
        "file:unknown-tip.rs",
        FIRST_RUN,
    ));
    let trunk_oid = git_stdout(unknown_tip_repository.path(), &["rev-parse", "HEAD"]);
    let unavailable_tip = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    append_journal_operation(
        unknown_tip_repository.path(),
        &serde_json::json!({
            "op": "checkpoint",
            "reservation_id": unknown_tip_id,
            "protected_tip": unavailable_tip,
            "trunk_snapshot": trunk_oid,
        }),
    );
    append_journal_operation(
        unknown_tip_repository.path(),
        &serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": unknown_tip_id,
            "status": {"status": "integrated", "trunk_oid": trunk_oid},
            "edit_blocking_status": "clear",
        }),
    );
    append_journal_operation(
        unknown_tip_repository.path(),
        &serde_json::json!({
            "op": "release",
            "reservation_id": unknown_tip_id,
            "disposition": {"kind": "integrated"},
        }),
    );

    let unknown_tip_board = board_data(unknown_tip_repository.path());
    let unknown_tip_row = board_reservation_snapshot(&unknown_tip_board, &unknown_tip_id);
    assert_eq!(unknown_tip_row["edit_blocking_status"], "clear");
    assert_eq!(
        unknown_tip_row["integration_evidence"]["status"]["status"],
        "object_unknown"
    );
    let unknown_tip_alert = unknown_tip_board["alerts"]["entries"]
        .as_array()
        .and_then(|alerts| {
            alerts.iter().find(|alert| {
                alert["kind"] == "lost_integration_evidence"
                    && alert["reservation_id"] == unknown_tip_id
            })
        })
        .expect("an unavailable protected tip should raise an alert");
    assert_eq!(
        unknown_tip_alert["recovery"]["kind"],
        "verify_resolved_trunk"
    );
    assert_eq!(unknown_tip_alert["recovery"]["trunk_oid"], trunk_oid);
}

#[test]
fn legacy_release_then_resnapshot_replays_to_a_lost_evidence_alert() {
    let legacy_repository = initialized_repository();
    let legacy_id = reservation_id(&claim(
        legacy_repository.path(),
        "file:legacy-resnapshot.rs",
        FIRST_RUN,
    ));
    fs::write(
        legacy_repository.path().join("legacy-resnapshot.rs"),
        "legacy work\n",
    )
    .expect("legacy protected work should write");
    git(legacy_repository.path(), &["add", "legacy-resnapshot.rs"]);
    git(
        legacy_repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "legacy protected work",
        ],
    );
    let protected_tip = git_stdout(legacy_repository.path(), &["rev-parse", "HEAD"]);
    for operation in [
        serde_json::json!({
            "op": "checkpoint",
            "reservation_id": legacy_id,
            "protected_tip": protected_tip,
            "trunk_snapshot": protected_tip,
        }),
        serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": legacy_id,
            "status": {"status": "integrated", "trunk_oid": protected_tip},
            "edit_blocking_status": "clear",
        }),
        serde_json::json!({
            "op": "release",
            "reservation_id": legacy_id,
            "disposition": {"kind": "integrated"},
        }),
        serde_json::json!({
            "op": "resnapshot",
            "reservation_id": legacy_id,
            "snapshot": {
                "stage": "outstanding",
                "protected_tip": protected_tip,
                "trunk_oid": protected_tip,
            },
        }),
    ] {
        append_journal_operation(legacy_repository.path(), &operation);
    }
    git(
        legacy_repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            "--quiet",
            "HEAD^",
        ],
    );

    let legacy_board = board_data(legacy_repository.path());
    let legacy_row = board_reservation_snapshot(&legacy_board, &legacy_id);
    assert_eq!(legacy_row["lifecycle"]["stage"], "released");
    assert_eq!(legacy_row["edit_blocking_status"], "clear");
    assert!(
        legacy_board["alerts"]["entries"]
            .as_array()
            .is_some_and(|alerts| alerts.iter().any(|alert| {
                alert["kind"] == "lost_integration_evidence" && alert["reservation_id"] == legacy_id
            }))
    );
}

#[test]
fn deferred_comparison_preserves_a_scoped_patch_equivalence_proof() {
    let fixture =
        rewritten_reservation_fixture(TargetRewrite::Equivalent, ReservationCompletion::Released);
    append_scoped_patch_equivalence_evidence(
        fixture.repository.path(),
        &fixture.reservation_id,
        &fixture.target,
    );
    let competing_reservations =
        append_released_reservations(&fixture, 1, ProofSubjectSimilarity::Distinct);
    append_scoped_patch_attempt(
        fixture.repository.path(),
        &fixture.reservation_id,
        &fixture.target,
    );

    let deferred = run_board_with_git_trace(fixture.repository.path());
    assert!(deferred.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(&deferred, &fixture.phase_start_head, &fixture.target,),
        0
    );
    let deferred_board = json_output(&deferred.output);
    let data = &deferred_board["payload"]["data"];
    assert_integration_statuses(
        data,
        std::slice::from_ref(&fixture.reservation_id),
        "integrated",
    );
    assert_eq!(
        board_reservation_snapshot(data, &fixture.reservation_id)["integration_evidence"]["status"]
            ["proof"],
        "scoped_patch_equivalent"
    );
    assert_eq!(
        board_reservation_snapshot(data, &fixture.reservation_id)["integration_evidence"]["status"]
            ["trunk_oid"],
        fixture.target
    );
    assert_integration_statuses(data, &competing_reservations, "trunk_rewritten");
}

#[test]
fn deferred_comparison_rejects_a_scoped_patch_proof_from_an_earlier_target() {
    let fixture = reverted_scoped_patch_proof_fixture();
    let reservation = &fixture.reservation;
    let competing_reservations =
        append_released_reservations(reservation, 1, ProofSubjectSimilarity::Distinct);
    append_scoped_patch_attempt(
        reservation.repository.path(),
        &reservation.reservation_id,
        &reservation.target,
    );
    let evidence_before = journal_operation_count_for_reservation(
        reservation.repository.path(),
        "evidence_revalidated",
        &reservation.reservation_id,
    );
    let attempts_before = journal_operation_count_for_reservation(
        reservation.repository.path(),
        "scoped_patch_comparison_attempted",
        &reservation.reservation_id,
    );
    let verdicts_before = journal_operation_count_for_reservation(
        reservation.repository.path(),
        "scoped_patch_equivalence_checked",
        &reservation.reservation_id,
    );

    let deferred = run_board_with_git_trace(reservation.repository.path());
    assert!(deferred.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &deferred,
            &reservation.phase_start_head,
            &reservation.target,
        ),
        0
    );
    assert_ne!(fixture.earlier_proof_target, reservation.target);
    let deferred_board = json_output(&deferred.output);
    let data = &deferred_board["payload"]["data"];
    assert_not_integrated_and_blocking(data, &reservation.reservation_id);
    assert_integration_statuses(data, &competing_reservations, "trunk_rewritten");
    assert_eq!(
        journal_operation_count_for_reservation(
            reservation.repository.path(),
            "evidence_revalidated",
            &reservation.reservation_id,
        ),
        evidence_before + 1
    );
    assert_eq!(
        journal_operation_count_for_reservation(
            reservation.repository.path(),
            "scoped_patch_comparison_attempted",
            &reservation.reservation_id,
        ),
        attempts_before
    );
    assert_eq!(
        journal_operation_count_for_reservation(
            reservation.repository.path(),
            "scoped_patch_equivalence_checked",
            &reservation.reservation_id,
        ),
        verdicts_before
    );

    let replay_competitor =
        append_released_reservations(reservation, 1, ProofSubjectSimilarity::Distinct);
    let replayed = run_board_with_git_trace(reservation.repository.path());
    assert!(replayed.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(
            &replayed,
            &reservation.phase_start_head,
            &reservation.target,
        ),
        0
    );
    let replayed_board = json_output(&replayed.output);
    let replayed_data = &replayed_board["payload"]["data"];
    assert_not_integrated_and_blocking(replayed_data, &reservation.reservation_id);
    assert_integration_statuses(replayed_data, &replay_competitor, "trunk_rewritten");
    assert_eq!(
        journal_operation_count_for_reservation(
            reservation.repository.path(),
            "scoped_patch_comparison_attempted",
            &reservation.reservation_id,
        ),
        attempts_before
    );
    assert_eq!(
        journal_operation_count_for_reservation(
            reservation.repository.path(),
            "scoped_patch_equivalence_checked",
            &reservation.reservation_id,
        ),
        verdicts_before
    );
}

#[test]
fn comparisons_without_retained_verdicts_advance_through_every_distinct_subject() {
    let fixture = comparison_reservations_without_retained_verdicts_fixture(4);
    let mut compared_scopes = Vec::new();

    for _ in 0..fixture.reservation_ids.len() {
        let traced = run_board_with_git_trace(fixture.repository.path());
        assert!(traced.output.status.success());
        assert_eq!(
            scoped_patch_comparison_attempts(&traced, &fixture.phase_start_head, &fixture.target),
            1
        );
        assert_integration_statuses(
            &json_output(&traced.output)["payload"]["data"],
            &fixture.reservation_ids,
            "not_integrated",
        );
        let trace = fs::read_to_string(&traced.trace_path).expect("git trace should read");
        compared_scopes.extend(
            fixture
                .scopes
                .iter()
                .filter(|scope| trace.contains(scope.as_str()))
                .cloned(),
        );
    }

    compared_scopes.sort();
    compared_scopes.dedup();
    assert_eq!(compared_scopes, fixture.scopes);
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "scoped_patch_comparison_attempted"
        ),
        0
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        fixture.reservation_ids.len()
    );
    let stable_journal_size = journal_record_count(fixture.repository.path());
    for _ in 0..20 {
        let traced = run_board_with_git_trace(fixture.repository.path());
        assert!(traced.output.status.success());
        assert_eq!(
            scoped_patch_comparison_attempts(&traced, &fixture.phase_start_head, &fixture.target),
            0
        );
    }
    assert_eq!(
        journal_record_count(fixture.repository.path()),
        stable_journal_size
    );
}

#[test]
fn distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target() {
    for target_rewrite in [TargetRewrite::Equivalent, TargetRewrite::Different] {
        let one = rewritten_reservation_fixture(target_rewrite, ReservationCompletion::Released);
        let one_trace = run_board_with_git_trace(one.repository.path());
        assert!(one_trace.output.status.success());
        let one_argv = scoped_patch_git_argv(
            &one_trace,
            &one.phase_start_head,
            &one.protected_tip,
            &one.target,
        );

        let twenty = rewritten_reservation_fixture(target_rewrite, ReservationCompletion::Released);
        let additional_reservation_ids =
            append_released_reservations(&twenty, 19, ProofSubjectSimilarity::Distinct);
        let twenty_trace = run_board_with_git_trace(twenty.repository.path());
        assert!(twenty_trace.output.status.success());
        let twenty_argv = scoped_patch_git_argv(
            &twenty_trace,
            &twenty.phase_start_head,
            &twenty.protected_tip,
            &twenty.target,
        );

        let expected_argv_total = match target_rewrite {
            TargetRewrite::Equivalent => 6,
            TargetRewrite::Different => 5,
        };
        assert!(!one_argv.is_empty());
        assert_eq!(one_argv.len(), expected_argv_total, "{one_argv:#?}");
        assert_eq!(twenty_argv.len(), expected_argv_total, "{twenty_argv:#?}");
        assert_eq!(twenty_argv.len(), one_argv.len());
        assert_eq!(merge_base_ancestor_invocations(&one_trace), 0);
        assert_eq!(
            merge_base_ancestor_invocations(&twenty_trace),
            merge_base_ancestor_invocations(&one_trace)
        );
        assert_eq!(
            canonical_git_command_sequence(&twenty_argv),
            canonical_git_command_sequence(&one_argv)
        );
        let twenty_data = &json_output(&twenty_trace.output)["payload"]["data"];
        let expected_first_status = match target_rewrite {
            TargetRewrite::Equivalent => "integrated",
            TargetRewrite::Different => "trunk_rewritten",
        };
        assert_integration_statuses(
            twenty_data,
            std::slice::from_ref(&twenty.reservation_id),
            expected_first_status,
        );
        assert_integration_statuses(twenty_data, &additional_reservation_ids, "not_integrated");
    }
}

#[test]
fn duplicate_cold_proof_subjects_share_one_git_evaluation() {
    for target_rewrite in [TargetRewrite::Equivalent, TargetRewrite::Different] {
        let one = rewritten_reservation_fixture(target_rewrite, ReservationCompletion::Released);
        let one_trace = run_board_with_git_trace(one.repository.path());
        assert!(one_trace.output.status.success());
        let one_argv = scoped_patch_git_argv(
            &one_trace,
            &one.phase_start_head,
            &one.protected_tip,
            &one.target,
        );

        let twenty = rewritten_reservation_fixture(target_rewrite, ReservationCompletion::Released);
        let additional_reservation_ids =
            append_released_reservations(&twenty, 19, ProofSubjectSimilarity::Duplicate);
        let twenty_trace = run_board_with_git_trace(twenty.repository.path());
        assert!(twenty_trace.output.status.success());
        let twenty_argv = scoped_patch_git_argv(
            &twenty_trace,
            &twenty.phase_start_head,
            &twenty.protected_tip,
            &twenty.target,
        );

        assert!(!one_argv.is_empty());
        assert_eq!(twenty_argv.len(), one_argv.len());
        assert_eq!(
            canonical_git_command_sequence(&twenty_argv),
            canonical_git_command_sequence(&one_argv)
        );
        let expected_status = match target_rewrite {
            TargetRewrite::Equivalent => "integrated",
            TargetRewrite::Different => "trunk_rewritten",
        };
        let twenty_data = &json_output(&twenty_trace.output)["payload"]["data"];
        assert_integration_statuses(
            twenty_data,
            std::slice::from_ref(&twenty.reservation_id),
            expected_status,
        );
        assert_integration_statuses(twenty_data, &additional_reservation_ids, expected_status);
    }
}

#[test]
fn orphan_recoverability_and_each_observed_git_query_are_explicit() {
    let repository = initialized_repository();
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = tempdir().expect("worktree directory should exist");
    let orphan_root = add_worktree(repository.path(), worktrees.path(), "orphan-recovery");
    fs::write(orphan_root.join("src/lib.rs"), "pub fn orphaned() {}\n")
        .expect("orphan work should write");
    git(&orphan_root, &["add", "src/lib.rs"]);
    git(&orphan_root, &["commit", "--quiet", "-m", "orphan work"]);
    let reservation = claim(&orphan_root, "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&reservation);
    let released = run_berth_with_run(
        &orphan_root,
        &["release", &reservation_id, "--json"],
        FIRST_RUN,
    );
    assert!(released.status.success());
    let protected_tip = json_output(&released)["payload"]["data"]["protected_tip"]
        .as_str()
        .expect("checkpoint should report its protected tip")
        .to_owned();
    fs::remove_dir_all(&orphan_root).expect("orphan worktree should remove");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);

    let traced = run_board_with_git_trace(repository.path());
    assert!(traced.output.status.success());
    let branch = "rev-parse refs/heads/orphan-recovery".to_owned();
    let retention = format!("rev-parse refs/cargo-berth/reservations/{reservation_id}");
    let branch_ancestry = format!("merge-base --is-ancestor {protected_tip} {protected_tip}");
    let trace = fs::read_to_string(&traced.trace_path).expect("git trace should read");
    assert_eq!(trace.lines().filter(|line| *line == branch).count(), 1);
    let protected_object_query = format!("cat-file -e {protected_tip}^{{commit}}");
    assert_eq!(
        trace
            .lines()
            .filter(|line| *line == protected_object_query)
            .count(),
        1
    );
    assert_batched_evidence_and_retention_queries(&trace);
    assert_eq!(trace.lines().filter(|line| *line == retention).count(), 1);
    assert_eq!(
        trace
            .lines()
            .filter(|line| *line == branch_ancestry)
            .count(),
        1
    );
    let traced_board = json_output(&traced.output);
    assert_eq!(
        traced_board["payload"]["data"]["git_cost"]["trunk_resolution_calls"],
        1
    );
    assert_eq!(
        traced_board["payload"]["data"]["git_cost"]["orphan_recovery_evidence_queries"],
        4
    );
    assert_orphan_verdict(
        &traced_board["payload"]["data"],
        "recoverable_from_branch",
        "work_recoverable",
        "recover",
    );

    git(
        repository.path(),
        &[
            "update-ref",
            "refs/heads/orphan-recovery",
            "refs/heads/main",
        ],
    );
    assert_orphan_verdict(
        &board_data(repository.path()),
        "recoverable_from_protected_tip",
        "work_recoverable",
        "recover",
    );

    git(
        repository.path(),
        &[
            "update-ref",
            "-d",
            &format!("refs/cargo-berth/reservations/{reservation_id}"),
        ],
    );
    git(
        repository.path(),
        &["reflog", "expire", "--expire=now", "--all"],
    );
    git(repository.path(), &["gc", "--prune=now"]);
    assert_orphan_verdict(
        &board_data(repository.path()),
        "commit_unavailable",
        "commits_lost",
        "retire_or_abandon",
    );
}

fn assert_orphan_verdict(
    data: &serde_json::Value,
    recoverability: &str,
    consequence: &str,
    action: &str,
) {
    let alert = data["alerts"]["entries"]
        .as_array()
        .and_then(|alerts| {
            alerts
                .iter()
                .find(|alert| alert["kind"] == "orphaned_outstanding")
        })
        .expect("orphan alert should render");
    assert_eq!(alert["recoverability"], recoverability);
    assert_eq!(alert["recovery_consequence"], consequence);
    assert_eq!(alert["resolution"]["action"], action);
    let encoded = alert.to_string();
    if consequence == "commits_lost" {
        assert!(encoded.contains("commits_lost"));
    } else {
        assert!(!encoded.contains("commits_lost"));
    }
}

fn assert_batched_evidence_and_retention_queries(trace: &str) {
    assert_eq!(
        trace
            .lines()
            .filter(|line| line.starts_with("cat-file --batch-check"))
            .count(),
        1
    );
    assert_eq!(
        trace
            .lines()
            .filter(|line| *line == "update-ref --stdin")
            .count(),
        1
    );
}

struct TracedBoard {
    output:     Output,
    trace_path: std::path::PathBuf,
    _directory: TempDir,
}

struct RewrittenReservationFixture {
    repository:       TempDir,
    reservation_id:   String,
    phase_start_head: String,
    protected_tip:    String,
    target:           String,
}

struct RevertedScopedPatchProofFixture {
    reservation:          RewrittenReservationFixture,
    earlier_proof_target: String,
}

struct OutstandingReservationFixture {
    repository:       TempDir,
    reservation_ids:  Vec<String>,
    scopes:           Vec<String>,
    phase_start_head: String,
    target:           String,
}

struct PersistentUnavailableComparisonFixture {
    repository:              TempDir,
    reservation_ids:         Vec<String>,
    unavailable_phase_start: &'static str,
    target:                  String,
}

#[derive(Clone, Copy)]
enum TargetRewrite {
    Equivalent,
    Different,
}

#[derive(Clone, Copy)]
enum ReservationCompletion {
    Outstanding,
    Released,
}

#[derive(Clone, Copy)]
enum ProofSubjectChange {
    Widen,
    Resnapshot,
    ReleaseDispositionReplacement,
}

#[derive(Clone, Copy)]
enum ProofSubjectSimilarity {
    Distinct,
    Duplicate,
}

struct OrderedFixture {
    repository:       TempDir,
    _worktrees:       TempDir,
    predecessor_root: PathBuf,
    successor_root:   PathBuf,
    predecessor_id:   String,
    successor_id:     String,
}

/// A board holding one released reservation for each of the four durable release dispositions.
struct ReleasedDispositionFixture {
    repository:            TempDir,
    integrated:            ReleasedDisposition,
    rewritten_integration: ReleasedDisposition,
    abandoned:             ReleasedDisposition,
    retired_orphan:        ReleasedDisposition,
}

/// One released reservation together with the disposition it was released under.
struct ReleasedDisposition {
    reservation_id: String,
    kind:           &'static str,
    evidence:       ReleaseEvidence,
}

/// What a release disposition leaves behind for a later trunk rewrite to reinterpret.
enum ReleaseEvidence {
    /// The release rests on recorded integration evidence, which a trunk rewrite restates.
    RecordedIntegration,
    /// The release rests on a stated reason, which no trunk rewrite can reach.
    StatedReason,
}

/// Release four reservations over one trunk commit, one under each durable disposition.
///
/// The CLI records git-backed terminal dispositions only after a real merge. Direct journal facts
/// let the model test cover all four durable variants without coupling it to gate I/O.
fn released_disposition_fixture() -> ReleasedDispositionFixture {
    let repository = initialized_repository();
    let runs = [
        FIRST_RUN.to_owned(),
        SECOND_RUN.to_owned(),
        uuid::Uuid::now_v7().to_string(),
        uuid::Uuid::now_v7().to_string(),
    ];
    let reservation_ids = runs
        .iter()
        .enumerate()
        .map(|(index, run)| {
            reservation_id(&claim(
                repository.path(),
                &format!("file:release-{index}.rs"),
                run,
            ))
        })
        .collect::<Vec<_>>();
    let trunk_oid = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    for reservation_id in &reservation_ids[..2] {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "checkpoint",
                "reservation_id": reservation_id,
                "protected_tip": trunk_oid,
                "trunk_snapshot": trunk_oid,
            }),
        );
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "evidence_revalidated",
                "reservation_id": reservation_id,
                "status": {"status": "integrated", "trunk_oid": trunk_oid},
                "edit_blocking_status": "clear",
            }),
        );
    }
    let releases = [
        (
            ReleasedDisposition {
                reservation_id: reservation_ids[0].clone(),
                kind:           "integrated",
                evidence:       ReleaseEvidence::RecordedIntegration,
            },
            serde_json::json!({"kind": "integrated"}),
        ),
        (
            ReleasedDisposition {
                reservation_id: reservation_ids[1].clone(),
                kind:           "rewritten_integration",
                evidence:       ReleaseEvidence::RecordedIntegration,
            },
            serde_json::json!({"kind": "rewritten_integration", "evidence": trunk_oid}),
        ),
        (
            ReleasedDisposition {
                reservation_id: reservation_ids[2].clone(),
                kind:           "abandoned",
                evidence:       ReleaseEvidence::StatedReason,
            },
            serde_json::json!({"kind": "abandoned", "evidence": "discarded deliberately"}),
        ),
        (
            ReleasedDisposition {
                reservation_id: reservation_ids[3].clone(),
                kind:           "retired_orphan",
                evidence:       ReleaseEvidence::StatedReason,
            },
            serde_json::json!({"kind": "retired_orphan", "evidence": "retired after review"}),
        ),
    ];
    for (released, disposition) in &releases {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "release",
                "reservation_id": released.reservation_id,
                "disposition": disposition,
            }),
        );
    }
    let [integrated, rewritten_integration, abandoned, retired_orphan] =
        releases.map(|(released, _)| released);
    ReleasedDispositionFixture {
        repository,
        integrated,
        rewritten_integration,
        abandoned,
        retired_orphan,
    }
}

/// Prove one released reservation reached the resolved audit under its own disposition.
#[track_caller]
fn assert_release_disposition_recorded(data: &serde_json::Value, released: &ReleasedDisposition) {
    let snapshot = board_reservation_snapshot(data, &released.reservation_id);
    assert_eq!(
        snapshot["lifecycle"]["disposition"]["kind"], released.kind,
        "{} should be recorded under its own disposition",
        released.kind
    );
    if matches!(released.evidence, ReleaseEvidence::RecordedIntegration) {
        assert_eq!(
            snapshot["integration_evidence"]["status"]["status"], "integrated",
            "{} should rest on recorded integration evidence",
            released.kind
        );
    }
}

/// Prove one released reservation stays resolved and unblocking once trunk is rewritten.
#[track_caller]
fn assert_release_disposition_survives_rewrite(
    data: &serde_json::Value,
    released: &ReleasedDisposition,
) {
    let snapshot = board_reservation_snapshot(data, &released.reservation_id);
    assert_eq!(
        snapshot["lifecycle"]["disposition"]["kind"], released.kind,
        "{} should keep its disposition across a trunk rewrite",
        released.kind
    );
    assert_eq!(
        snapshot["visibility"], "resolved_audit",
        "{} should stay in the resolved audit",
        released.kind
    );
    assert_eq!(
        snapshot["edit_blocking_status"], "clear",
        "{} should block no edit",
        released.kind
    );
    if matches!(released.evidence, ReleaseEvidence::RecordedIntegration) {
        assert_eq!(
            snapshot["integration_evidence"]["status"]["status"], "trunk_rewritten",
            "{} should restate its integration evidence as rewritten",
            released.kind
        );
    }
}

fn persistent_unavailable_comparison_fixture() -> PersistentUnavailableComparisonFixture {
    let repository = initialized_repository();
    let base = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let scopes = [
        "src/unavailable.rs".to_owned(),
        "src/available-1.rs".to_owned(),
        "src/available-2.rs".to_owned(),
    ];
    let reservation_ids = scopes
        .iter()
        .map(|scope| {
            reservation_id(&claim(
                repository.path(),
                &format!("file:{scope}"),
                FIRST_RUN,
            ))
        })
        .collect::<Vec<_>>();
    let unavailable_phase_start = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "resnapshot",
            "reservation_id": reservation_ids[0],
            "snapshot": {
                "stage": "active",
                "claim_snapshot": unavailable_phase_start,
            },
        }),
    );
    for scope in &scopes {
        fs::write(
            repository.path().join(scope),
            format!("pub fn {}() {{}}\n", fixture_function_name(scope)),
        )
        .expect("protected source should write");
    }
    git(repository.path(), &["add", "src"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected identity",
        ],
    );
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    git(
        repository.path(),
        &[
            "update-ref",
            "refs/heads/unavailable-fixture",
            &protected_tip,
        ],
    );
    for reservation_id in &reservation_ids {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "checkpoint",
                "reservation_id": reservation_id,
                "protected_tip": protected_tip,
                "trunk_snapshot": base,
            }),
        );
    }
    git(
        repository.path(),
        &["-c", "core.hooksPath=/dev/null", "reset", "--hard", &base],
    );
    let target = commit_unprotected_target(repository.path());
    PersistentUnavailableComparisonFixture {
        repository,
        reservation_ids,
        unavailable_phase_start,
        target,
    }
}

fn commit_unprotected_target(repository_root: &Path) -> String {
    fs::write(
        repository_root.join("src/target.rs"),
        "pub fn target() {}\n",
    )
    .expect("target source should write");
    git(repository_root, &["add", "src/target.rs"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "target without protected content",
        ],
    );
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn assert_available_subjects_compared(fixture: &PersistentUnavailableComparisonFixture) {
    let compared_reservations = fs::read_to_string(fixture.repository.path().join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|record| {
            serde_json::from_str::<serde_json::Value>(record).expect("journal record should parse")
        })
        .filter(|record| {
            record["op"] == "scoped_patch_equivalence_checked" && record["target"] == fixture.target
        })
        .map(|record| {
            record["reservation_id"]
                .as_str()
                .expect("comparison should identify its reservation")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        compared_reservations.len(),
        fixture.reservation_ids.len() - 1
    );
    for reservation_id in &fixture.reservation_ids[1..] {
        assert!(compared_reservations.contains(reservation_id));
    }
}

fn assert_proof_subject_change_rechecks(
    target_rewrite: TargetRewrite,
    proof_subject_change: ProofSubjectChange,
) {
    let reservation_completion = match proof_subject_change {
        ProofSubjectChange::Widen | ProofSubjectChange::Resnapshot => {
            ReservationCompletion::Outstanding
        },
        ProofSubjectChange::ReleaseDispositionReplacement => ReservationCompletion::Released,
    };
    let fixture = rewritten_reservation_fixture(target_rewrite, reservation_completion);
    let warm = run_board_with_git_trace(fixture.repository.path());
    assert!(warm.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(&warm, &fixture.phase_start_head, &fixture.target),
        1
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        1
    );

    match proof_subject_change {
        ProofSubjectChange::Widen => append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "widen",
                "reservation_id": fixture.reservation_id,
                "added_scopes": [{"path": "src/extra.rs", "kind": "file"}],
                "cause": {"kind": "explicit", "reason": "include the companion source"},
                "authorization": {"kind": "no_conflict"},
                "edit_blocking_status": "clear",
            }),
        ),
        ProofSubjectChange::Resnapshot => append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "resnapshot",
                "reservation_id": fixture.reservation_id,
                "snapshot": {
                    "stage": "outstanding",
                    "protected_tip": fixture.protected_tip,
                    "trunk_oid": fixture.protected_tip,
                },
            }),
        ),
        ProofSubjectChange::ReleaseDispositionReplacement => append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "replace_release_disposition",
                "reservation_id": fixture.reservation_id,
                "superseded": {"kind": "integrated"},
                "replacement": {
                    "kind": "rewritten_integration",
                    "evidence": fixture.protected_tip,
                },
            }),
        ),
    };

    let rechecked = run_board_with_git_trace(fixture.repository.path());
    assert!(rechecked.output.status.success());
    assert_eq!(
        scoped_patch_comparison_attempts(&rechecked, &fixture.phase_start_head, &fixture.target,),
        1
    );
    let expected_status = match target_rewrite {
        TargetRewrite::Equivalent => "integrated",
        TargetRewrite::Different => "trunk_rewritten",
    };
    assert_eq!(
        board_reservation_snapshot(
            &json_output(&rechecked.output)["payload"]["data"],
            &fixture.reservation_id,
        )["integration_evidence"]["status"]["status"],
        expected_status
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "scoped_patch_equivalence_checked"
        ),
        2
    );
}

fn rewritten_reservation_fixture(
    target_rewrite: TargetRewrite,
    reservation_completion: ReservationCompletion,
) -> RewrittenReservationFixture {
    let repository = initialized_repository();
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let reservation_id = reservation_id(&claim(repository.path(), "file:src/lib.rs", FIRST_RUN));
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn protected() {}\n",
    )
    .expect("protected source should write");
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected identity",
        ],
    );
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "checkpoint",
            "reservation_id": reservation_id,
            "protected_tip": protected_tip,
            "trunk_snapshot": protected_tip,
        }),
    );
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": reservation_id,
            "status": {
                "status": "integrated",
                "trunk_oid": protected_tip,
                "proof": "protected_tip_ancestor",
            },
            "edit_blocking_status": "clear",
        }),
    );
    if matches!(reservation_completion, ReservationCompletion::Released) {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "release",
                "reservation_id": reservation_id,
                "disposition": {"kind": "integrated"},
            }),
        );
    }
    if matches!(target_rewrite, TargetRewrite::Different) {
        fs::write(
            repository.path().join("src/lib.rs"),
            "pub fn replacement() {}\n",
        )
        .expect("replacement source should write");
        git(repository.path(), &["add", "src/lib.rs"]);
    }
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "rewritten target",
        ],
    );
    let target = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    RewrittenReservationFixture {
        repository,
        reservation_id,
        phase_start_head,
        protected_tip,
        target,
    }
}

fn reverted_scoped_patch_proof_fixture() -> RevertedScopedPatchProofFixture {
    let repository = initialized_repository();
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let reservation_id = reservation_id(&claim(repository.path(), "file:src/lib.rs", FIRST_RUN));
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn protected() {}\n",
    )
    .expect("protected source should write");
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected side-branch tip",
        ],
    );
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    append_outstanding_ancestor_evidence(repository.path(), &reservation_id, &protected_tip);
    git(
        repository.path(),
        &[
            "update-ref",
            "refs/heads/protected-scoped-patch-proof",
            &protected_tip,
        ],
    );
    let earlier_proof_target = commit_library_target_from_base(
        repository.path(),
        &phase_start_head,
        "pub fn protected() {}\n",
        "equivalent proof target",
    );
    let warmed = run_board_with_git_trace(repository.path());
    assert!(warmed.output.status.success());
    let warmed_board = json_output(&warmed.output);
    let warmed_status = &board_reservation_snapshot(
        &warmed_board["payload"]["data"],
        &reservation_id,
    )["integration_evidence"]["status"];
    assert_eq!(warmed_status["status"], "integrated");
    assert_eq!(warmed_status["proof"], "scoped_patch_equivalent");
    assert_eq!(warmed_status["trunk_oid"], earlier_proof_target);

    let target = commit_library_target_from_base(
        repository.path(),
        &phase_start_head,
        "pub fn replacement() {}\n",
        "reverted scoped content target",
    );
    let current_target_source = git_stdout(
        repository.path(),
        &["show", &format!("{target}:src/lib.rs")],
    );
    assert_eq!(current_target_source, "pub fn replacement() {}");

    RevertedScopedPatchProofFixture {
        reservation: RewrittenReservationFixture {
            repository,
            reservation_id,
            phase_start_head,
            protected_tip,
            target,
        },
        earlier_proof_target,
    }
}

fn append_outstanding_ancestor_evidence(
    repository_root: &Path,
    reservation_id: &str,
    protected_tip: &str,
) {
    append_journal_operation(
        repository_root,
        &serde_json::json!({
            "op": "checkpoint",
            "reservation_id": reservation_id,
            "protected_tip": protected_tip,
            "trunk_snapshot": protected_tip,
        }),
    );
    append_journal_operation(
        repository_root,
        &serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": reservation_id,
            "status": {
                "status": "integrated",
                "trunk_oid": protected_tip,
                "proof": "protected_tip_ancestor",
            },
            "edit_blocking_status": "clear",
        }),
    );
}

fn commit_library_target_from_base(
    repository_root: &Path,
    phase_start_head: &str,
    source: &str,
    message: &str,
) -> String {
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            phase_start_head,
        ],
    );
    fs::write(repository_root.join("src/lib.rs"), source).expect("target source should write");
    git(repository_root, &["add", "src/lib.rs"]);
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

fn warmed_ancestor_proof_after_trunk_rewrite() -> RewrittenReservationFixture {
    let repository = initialized_repository();
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let reservation_id = reservation_id(&claim(repository.path(), "file:src/lib.rs", FIRST_RUN));
    fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn protected() {}\n",
    )
    .expect("protected source should write");
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected ancestor",
        ],
    );
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "checkpoint",
            "reservation_id": reservation_id,
            "protected_tip": protected_tip,
            "trunk_snapshot": protected_tip,
        }),
    );
    let warmed = run_board_with_git_trace(repository.path());
    assert!(warmed.output.status.success());
    let warmed_board = json_output(&warmed.output);
    assert_eq!(
        board_reservation_snapshot(&warmed_board["payload"]["data"], &reservation_id)["integration_evidence"]
            ["status"]["proof"],
        "protected_tip_ancestor"
    );
    append_journal_operation(
        repository.path(),
        &serde_json::json!({
            "op": "release",
            "reservation_id": reservation_id,
            "disposition": {"kind": "integrated"},
        }),
    );
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "--amend",
            "-m",
            "rewritten ancestor target",
        ],
    );
    let target = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    RewrittenReservationFixture {
        repository,
        reservation_id,
        phase_start_head,
        protected_tip,
        target,
    }
}

fn reachable_outstanding_reservations_fixture(
    reservation_count: usize,
) -> OutstandingReservationFixture {
    let repository = initialized_repository();
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let scopes = (0..reservation_count)
        .map(|index| format!("src/reachable-{index}.rs"))
        .collect::<Vec<_>>();
    let reservation_ids = scopes
        .iter()
        .map(|scope| {
            reservation_id(&claim(
                repository.path(),
                &format!("file:{scope}"),
                FIRST_RUN,
            ))
        })
        .collect::<Vec<_>>();
    for scope in &scopes {
        fs::write(
            repository.path().join(scope),
            format!("pub fn {}() {{}}\n", fixture_function_name(scope)),
        )
        .expect("reachable source should write");
    }
    git(repository.path(), &["add", "src"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "reachable protected tips",
        ],
    );
    let target = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    for reservation_id in &reservation_ids {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "checkpoint",
                "reservation_id": reservation_id,
                "protected_tip": target,
                "trunk_snapshot": phase_start_head,
            }),
        );
    }
    OutstandingReservationFixture {
        repository,
        reservation_ids,
        scopes,
        phase_start_head,
        target,
    }
}

fn comparison_reservations_without_retained_verdicts_fixture(
    reservation_count: usize,
) -> OutstandingReservationFixture {
    let repository = initialized_repository();
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let scopes = (0..reservation_count)
        .map(|index| format!("src/without-retained-verdict-{index}.rs"))
        .collect::<Vec<_>>();
    let reservation_ids = scopes
        .iter()
        .map(|scope| {
            reservation_id(&claim(
                repository.path(),
                &format!("file:{scope}"),
                FIRST_RUN,
            ))
        })
        .collect::<Vec<_>>();
    for scope in &scopes {
        fs::write(
            repository.path().join(scope),
            format!("pub fn {}() {{}}\n", fixture_function_name(scope)),
        )
        .expect("protected source without a retained verdict should write");
    }
    git(repository.path(), &["add", "src"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "protected content without a retained verdict",
        ],
    );
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    git(
        repository.path(),
        &["update-ref", "refs/heads/protected-fixture", &protected_tip],
    );
    for reservation_id in &reservation_ids {
        append_journal_operation(
            repository.path(),
            &serde_json::json!({
                "op": "checkpoint",
                "reservation_id": reservation_id,
                "protected_tip": protected_tip,
                "trunk_snapshot": phase_start_head,
            }),
        );
    }
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            &phase_start_head,
        ],
    );
    fs::write(
        repository.path().join("src/target.rs"),
        "pub fn target() {}\n",
    )
    .expect("target source should write");
    git(repository.path(), &["add", "src/target.rs"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "target without protected content",
        ],
    );
    let target = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    OutstandingReservationFixture {
        repository,
        reservation_ids,
        scopes,
        phase_start_head,
        target,
    }
}

fn fixture_function_name(scope: &str) -> String { scope.replace(['/', '-', '.'], "_") }

fn merge_base_ancestor_invocations(traced_board: &TracedBoard) -> usize {
    fs::read_to_string(&traced_board.trace_path)
        .expect("git trace should read")
        .lines()
        .filter(|line| line.starts_with("merge-base --is-ancestor "))
        .count()
}

fn scoped_patch_comparison_attempts(
    traced_board: &TracedBoard,
    phase_start_head: &str,
    target: &str,
) -> usize {
    let merge_base_query = format!("merge-base {phase_start_head} {target}");
    let excluded_phase_start = format!("^{phase_start_head}");
    fs::read_to_string(&traced_board.trace_path)
        .expect("git trace should read")
        .lines()
        .filter(|line| {
            *line == merge_base_query
                || (line.starts_with("rev-list --cherry-mark --left-right ")
                    && line
                        .split_whitespace()
                        .any(|argument| argument == excluded_phase_start)
                    && line.contains(target))
        })
        .count()
}

fn append_released_reservations(
    fixture: &RewrittenReservationFixture,
    additional_reservations: usize,
    proof_subject_similarity: ProofSubjectSimilarity,
) -> Vec<String> {
    let journal = fs::read_to_string(fixture.repository.path().join(JOURNAL_PATH))
        .expect("journal should read");
    let claim = journal
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should parse"))
        .find(|event| event["op"] == "claim")
        .expect("fixture should contain a claim");
    let mut reservation_ids = Vec::new();
    for reservation_index in 0..additional_reservations {
        let reservation_id = uuid::Uuid::now_v7().to_string();
        reservation_ids.push(reservation_id.clone());
        let scopes = match proof_subject_similarity {
            ProofSubjectSimilarity::Distinct => serde_json::json!([{
                "path": format!("src/distinct-{reservation_index}.rs"),
                "kind": "file",
            }]),
            ProofSubjectSimilarity::Duplicate => claim["scopes"].clone(),
        };
        append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "claim",
                "reservation_id": reservation_id,
                "scopes": scopes,
                "source": claim["source"],
                "purpose": claim["purpose"],
                "trunk_at_claim": claim["trunk_at_claim"],
                "head_snapshot": claim["head_snapshot"],
                "phase_start_head": claim["phase_start_head"],
                "worktree_root": claim["worktree_root"],
                "worktree_administrative_locator": claim["worktree_administrative_locator"],
                "authorization": {"kind": "no_conflict"},
            }),
        );
        append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "checkpoint",
                "reservation_id": reservation_id,
                "protected_tip": fixture.protected_tip,
                "trunk_snapshot": fixture.protected_tip,
            }),
        );
        append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "evidence_revalidated",
                "reservation_id": reservation_id,
                "status": {
                    "status": "integrated",
                    "trunk_oid": fixture.protected_tip,
                    "proof": "protected_tip_ancestor",
                },
                "edit_blocking_status": "clear",
            }),
        );
        append_journal_operation(
            fixture.repository.path(),
            &serde_json::json!({
                "op": "release",
                "reservation_id": reservation_id,
                "disposition": {"kind": "integrated"},
            }),
        );
    }
    reservation_ids
}

fn append_scoped_patch_attempt(repository_root: &Path, reservation_id: &str, target: &str) {
    append_journal_operation(
        repository_root,
        &serde_json::json!({
            "op": "scoped_patch_comparison_attempted",
            "reservation_id": reservation_id,
            "subject": 1,
            "target": target,
        }),
    );
}

fn append_scoped_patch_equivalence_evidence(
    repository_root: &Path,
    reservation_id: &str,
    target: &str,
) {
    append_journal_operation(
        repository_root,
        &serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": reservation_id,
            "status": {
                "status": "integrated",
                "trunk_oid": target,
                "proof": "scoped_patch_equivalent",
            },
            "edit_blocking_status": "clear",
        }),
    );
}

fn scoped_patch_git_argv(
    traced_board: &TracedBoard,
    phase_start_head: &str,
    protected_tip: &str,
    target: &str,
) -> Vec<String> {
    fs::read_to_string(&traced_board.trace_path)
        .expect("git trace should read")
        .lines()
        .filter(|line| {
            matches!(
                line.split_whitespace().next(),
                Some(
                    "cat-file"
                        | "merge-base"
                        | "diff"
                        | "log"
                        | "rev-list"
                        | "read-tree"
                        | "update-index"
                        | "write-tree"
                        | "merge-tree"
                )
            ) && (line.starts_with("cat-file --batch-check=")
                || line.contains(phase_start_head)
                || line.contains(protected_tip)
                || line.contains(target)
                || matches!(
                    line.split_whitespace().next(),
                    Some("update-index" | "write-tree")
                ))
        })
        .map(str::to_owned)
        .collect()
}

fn canonical_git_command_sequence(invocations: &[String]) -> Vec<&str> {
    let mut commands = invocations
        .iter()
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    commands.sort_unstable();
    commands
}

fn ordered_fixture() -> OrderedFixture {
    let repository = initialized_repository();
    git(repository.path(), &["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = tempdir().expect("worktree directory should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let predecessor = claim(&predecessor_root, "file:src/lib.rs", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = answered_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--after",
        &predecessor_id,
        "predecessor must land first",
    );
    assert!(successor.status.success());
    let successor_id = reservation_id(&successor);
    OrderedFixture {
        repository,
        _worktrees: worktrees,
        predecessor_root,
        successor_root,
        predecessor_id,
        successor_id,
    }
}

fn checkpoint_predecessor(fixture: &OrderedFixture) {
    fs::write(
        fixture.predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&fixture.predecessor_root, &["add", "src/lib.rs"]);
    git(
        &fixture.predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let released = run_berth_with_run(
        &fixture.predecessor_root,
        &["release", &fixture.predecessor_id, "--json"],
        FIRST_RUN,
    );
    assert!(released.status.success());
}

fn run_board_with_git_trace(repository_root: &Path) -> TracedBoard {
    let directory = tempdir().expect("git wrapper directory should exist");
    let wrapper_path = directory.path().join(GIT_BINARY);
    let trace_path = directory.path().join("trace");
    fs::write(&wrapper_path, TRACING_GIT_WRAPPER).expect("git wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let original_path = std::env::var_os("PATH").expect("PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["board", "--json"])
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(TRACE_ENVIRONMENT, &trace_path)
        .output()
        .expect("board should run");
    TracedBoard {
        output,
        trace_path,
        _directory: directory,
    }
}

/// Add a real worktree beside the repository, the only actor berth treats as foreign.
///
/// Two coordination runs inside one worktree are one actor, so a distinct `--run`
/// no longer names a second party. The returned directory owns the worktree and
/// must outlive its use.
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

fn initialized_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(repository.path(), &["config", "user.name", "Test User"]);
    fs::create_dir_all(repository.path().join("src")).expect("source directory should exist");
    fs::write(repository.path().join("src/lib.rs"), "pub fn value() {}\n")
        .expect("source should write");
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    assert!(
        run_berth(repository.path(), &["init", "--json"])
            .status
            .success()
    );
    repository
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> std::path::PathBuf {
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
    let git_file = fs::read_to_string(worktree_root.join(".git")).expect("git file should read");
    PathBuf::from(
        git_file
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree git file should name its directory"),
    )
}

fn claim(repository_root: &Path, scope: &str, run: &str) -> Output {
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--why",
            "protect board test work",
            "--json",
        ],
    )
}

fn defer_claim(repository_root: &Path, scope: &str, run: &str, blocker: &str) -> Output {
    answered_claim(
        repository_root,
        scope,
        run,
        "--defer",
        blocker,
        "the order is not known yet",
    )
}

fn answered_claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    answer_flag: &str,
    blocker: &str,
    answer_reason: &str,
) -> Output {
    let proposal = run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            answer_flag,
            blocker,
            "--overlap-why",
            answer_reason,
            "--why",
            "protect answered board work",
            "--json",
        ],
    );
    let token = json_output(&proposal)["payload"]["data"]["proposal_token"]
        .as_str()
        .expect("proposal should carry a token")
        .to_owned();
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            answer_flag,
            blocker,
            "--overlap-why",
            answer_reason,
            "--why",
            "protect answered board work",
            "--proposal",
            &token,
            "--json",
        ],
    )
}

fn board_data(repository_root: &Path) -> serde_json::Value {
    let output = run_berth(repository_root, &["board", "--json"]);
    assert!(
        output.status.success(),
        "board failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    json_output(&output)["payload"]["data"].clone()
}

/// Append an otherwise unreachable journal fact while retaining the real event envelope.
fn append_journal_operation(
    repository_root: &Path,
    operation: &serde_json::Value,
) -> serde_json::Value {
    append_journal_operation_with_actor(repository_root, operation, ActorFixture::Retained)
}

enum ActorFixture {
    Retained,
    TransactionOnly {
        worktree_id: String,
        run_id:      String,
    },
}

fn append_journal_operation_with_actor(
    repository_root: &Path,
    operation: &serde_json::Value,
    actor_fixture: ActorFixture,
) -> serde_json::Value {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let text = fs::read_to_string(&journal_path).expect("journal should read");
    let previous = text
        .lines()
        .last()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should parse"))
        .expect("fixture requires one existing event");
    let mut event = operation
        .as_object()
        .expect("operation should be an object")
        .clone();
    event.insert("schema_version".to_owned(), serde_json::json!(1));
    event.insert(
        "event_id".to_owned(),
        serde_json::json!(uuid::Uuid::now_v7().to_string()),
    );
    let actor = match actor_fixture {
        ActorFixture::Retained => previous["actor"].clone(),
        ActorFixture::TransactionOnly {
            worktree_id,
            run_id,
        } => serde_json::json!({
            "repository": previous["actor"]["repository"],
            "worktree": worktree_id,
            "run": run_id,
        }),
    };
    event.insert("actor".to_owned(), actor);
    event.insert("at".to_owned(), previous["at"].clone());
    event.insert(
        "projection_generation".to_owned(),
        serde_json::json!(
            previous["projection_generation"]
                .as_u64()
                .expect("generation should be numeric")
                + 1
        ),
    );
    let event = serde_json::Value::Object(event);
    let mut journal = OpenOptions::new()
        .append(true)
        .open(journal_path)
        .expect("journal should open for append");
    serde_json::to_writer(&mut journal, &event).expect("event should serialize");
    journal
        .write_all(b"\n")
        .expect("event newline should write");
    journal.sync_all().expect("journal event should sync");
    event
}

fn rewrite_operation_times(repository_root: &Path, operation: &str, at: &str) {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let rewritten = fs::read_to_string(&journal_path)
        .expect("journal should read")
        .lines()
        .map(|line| {
            let mut event =
                serde_json::from_str::<serde_json::Value>(line).expect("event should parse");
            if event["op"] == operation {
                event["at"] = serde_json::json!(at);
            }
            serde_json::to_string(&event).expect("event should serialize")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(journal_path, format!("{rewritten}\n")).expect("journal fixture should rewrite");
    invalidate_projection(repository_root);
}

fn rewrite_checkpoint_protected_tip(
    repository_root: &Path,
    reservation_id: &str,
    protected_tip: &str,
) {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let rewritten = fs::read_to_string(&journal_path)
        .expect("journal should read")
        .lines()
        .map(|line| {
            let mut event =
                serde_json::from_str::<serde_json::Value>(line).expect("event should parse");
            if event["op"] == "checkpoint" && event["reservation_id"] == reservation_id {
                event["protected_tip"] = serde_json::json!(protected_tip);
            }
            serde_json::to_string(&event).expect("event should serialize")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(journal_path, format!("{rewritten}\n")).expect("journal fixture should rewrite");
    invalidate_projection(repository_root);
}

fn invalidate_projection(repository_root: &Path) {
    let projection_path = repository_root.join(".git/cargo-berth/reservations.json");
    match fs::remove_file(projection_path) {
        Ok(()) => {},
        Err(error) => assert_eq!(
            error.kind(),
            std::io::ErrorKind::NotFound,
            "projection fixture should remove: {error}"
        ),
    }
}

fn reservation_id(output: &Output) -> String {
    assert!(output.status.success());
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should report a reservation id")
        .to_owned()
}

fn journal_operation_count(repository_root: &Path, operation: &str) -> usize {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["op"] == operation)
        .count()
}

fn journal_operation_count_for_reservation(
    repository_root: &Path,
    operation: &str,
    reservation_id: &str,
) -> usize {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["op"] == operation && event["reservation_id"] == reservation_id)
        .count()
}

fn journal_record_count(repository_root: &Path) -> usize {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .count()
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should print JSON")
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_run(repository_root: &Path, arguments: &[&str], run: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env("CARGO_BERTH_RUN", run)
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("identified cargo-berth command should run")
}

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

fn git_binary() -> String {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("git lookup should run");
    String::from_utf8(output.stdout)
        .expect("git path should be UTF-8")
        .trim()
        .to_owned()
}
