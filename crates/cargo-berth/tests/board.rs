#![allow(
    clippy::expect_used,
    reason = "tests should stop immediately when fixtures or command output are invalid"
)]

//! Built-binary tests for the headless board and its coherent replay projection.

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
const LEAVE_ALTERNATE_SCREEN: &str = "\u{1b}[?1049l";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    command_name="$2"
    (
        shift 2
        printf '%s' "$command_name" >> "$CARGO_BERTH_TEST_GIT_TRACE"
        for argument in "$@"; do printf ' %s' "$argument" >> "$CARGO_BERTH_TEST_GIT_TRACE"; done
        printf '\n' >> "$CARGO_BERTH_TEST_GIT_TRACE"
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
    assert_eq!(envelope["status"], "board_ready");
    assert_eq!(envelope["payload"]["kind"], "board");
    assert_eq!(
        envelope["payload"]["data"]["integration_order"],
        "undeclared"
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
    assert!(!String::from_utf8_lossy(&text.stdout).contains("ReservationRow"));
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
    let predecessor = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let deferred = defer_claim(
        repository.path(),
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
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
    let deadline = Instant::now() + Duration::from_secs(5);
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
        let blocker = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
        let blocker_id = reservation_id(&blocker);
        let answered = answered_claim(
            repository.path(),
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
    let first = claim(repository.path(), "file:first.rs", FIRST_RUN);
    let first_id = reservation_id(&first);
    let second = answered_claim(
        repository.path(),
        "file:first.rs",
        SECOND_RUN,
        "--after",
        &first_id,
        "first chain order",
    );
    assert!(second.status.success());

    let third_run = uuid::Uuid::now_v7().to_string();
    let fourth_run = uuid::Uuid::now_v7().to_string();
    let third = claim(repository.path(), "file:third.rs", &third_run);
    let third_id = reservation_id(&third);
    let fourth = answered_claim(
        repository.path(),
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
        reservation_row(&not_integrated, &fixture.predecessor_id)["integration_evidence"]["status"]
            ["status"],
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
        reservation_row(&unknown, &unknown_fixture.predecessor_id)["integration_evidence"]["status"]
            ["status"],
        "object_unknown"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one scenario compares all four release dispositions before and after one rewrite"
)]
fn release_dispositions_move_to_resolved_and_reenter_when_trunk_rewrites() {
    // The CLI records git-backed terminal dispositions only after a real merge. Direct journal
    // facts let this model test cover all four durable variants without coupling it to gate I/O.
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
            serde_json::json!({
                "op": "checkpoint",
                "reservation_id": reservation_id,
                "protected_tip": trunk_oid,
                "trunk_snapshot": trunk_oid,
            }),
        );
        append_journal_operation(
            repository.path(),
            serde_json::json!({
                "op": "evidence_revalidated",
                "reservation_id": reservation_id,
                "status": {"status": "integrated", "trunk_oid": trunk_oid},
                "edit_blocking_status": "clear",
            }),
        );
    }
    for (reservation_id, disposition) in [
        (
            &reservation_ids[0],
            serde_json::json!({"kind": "integrated"}),
        ),
        (
            &reservation_ids[1],
            serde_json::json!({"kind": "rewritten_integration", "evidence": trunk_oid}),
        ),
        (
            &reservation_ids[2],
            serde_json::json!({"kind": "abandoned", "evidence": "discarded deliberately"}),
        ),
        (
            &reservation_ids[3],
            serde_json::json!({"kind": "retired_orphan", "evidence": "retired after review"}),
        ),
    ] {
        append_journal_operation(
            repository.path(),
            serde_json::json!({
                "op": "release",
                "reservation_id": reservation_id,
                "disposition": disposition,
            }),
        );
    }

    let clean = board_data(repository.path());
    let resolved = clean["resolved"]["entries"]
        .as_array()
        .expect("resolved audit should be an array");
    assert_eq!(resolved.len(), 4);
    let dispositions = reservation_ids
        .iter()
        .map(|reservation_id| {
            reservation_row(&clean, reservation_id)["lifecycle"]["disposition"]["kind"]
                .as_str()
                .expect("released row should carry a disposition")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        dispositions,
        [
            "integrated",
            "rewritten_integration",
            "abandoned",
            "retired_orphan"
        ]
    );
    assert_eq!(
        reservation_row(&clean, &reservation_ids[0])["integration_evidence"]["status"]["status"],
        "integrated"
    );

    git(
        repository.path(),
        &["commit", "--quiet", "--amend", "-m", "rewrite main"],
    );
    let rewritten = board_data(repository.path());
    for reservation_id in &reservation_ids[..2] {
        let row = reservation_row(&rewritten, reservation_id);
        assert_eq!(row["visibility"], "reblocked_active_constraint");
        assert_eq!(
            row["integration_evidence"]["status"]["status"],
            "trunk_rewritten"
        );
        assert!(
            rewritten["resolved"]["entries"]
                .as_array()
                .is_some_and(|rows| {
                    rows.iter()
                        .all(|row| row["reservation_id"].as_str() != Some(reservation_id.as_str()))
                })
        );
    }
    for reservation_id in &reservation_ids[2..] {
        assert_eq!(
            reservation_row(&rewritten, reservation_id)["visibility"],
            "resolved_audit"
        );
    }
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
        serde_json::json!({
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
        reservation_row(&data, &fresh_id)["freshness"]["status"],
        "fresh"
    );
    let stale_row = reservation_row(&data, &stale_id);
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
            reservation_row(&data, reservation_id)["holder"]["liveness"],
            expected_liveness
        );
    }
    assert_eq!(
        reservation_row(&data, &reservation_ids[3])["ahead_behind_main"]["status"],
        "unavailable"
    );
    assert_eq!(
        reservation_row(&data, &unrelated_id)["ahead_behind_main"]["status"],
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

fn reservation_row<'board>(
    data: &'board serde_json::Value,
    reservation_id: &str,
) -> &'board serde_json::Value {
    ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .find(|row| row["reservation_id"] == reservation_id)
        .expect("reservation should have a board row")
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
        repository.path(),
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
            serde_json::json!({
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
            serde_json::json!({
                "op": "forced_integration_permit",
                "permit_id": permit_id,
                "reservation_id": reservation_id,
                "reason": format!("approved {kind}"),
                "skipped_holds": skipped_holds,
            }),
        );
        append_journal_operation(
            repository.path(),
            serde_json::json!({
                "op": "consume_forced_integration_permit",
                "permit_id": permit_id,
                "reservation_id": reservation_id,
            }),
        );
        append_journal_operation(
            repository.path(),
            serde_json::json!({
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
        serde_json::json!({
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
        serde_json::json!({
            "op": "consume_forced_integration_permit",
            "permit_id": permit_id,
            "reservation_id": reservation_id,
        }),
    );
    append_journal_operation(
        repository.path(),
        serde_json::json!({
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
        serde_json::json!({
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
    let holder = claim(repository.path(), "tree:shared", FIRST_RUN);
    let holder_id = reservation_id(&holder);
    let subject = answered_claim(
        repository.path(),
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
        repository.path(),
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
        serde_json::json!({
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
        serde_json::json!({
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
            serde_json::json!({
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
            .filter(|line| line.starts_with("rev-parse refs/heads/main"))
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
            .filter(|line| line.contains("--left-right --boundary"))
            .count(),
        1
    );
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
        2
    );
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

struct TracedBoard {
    output:     Output,
    trace_path: std::path::PathBuf,
    _directory: TempDir,
}

struct OrderedFixture {
    repository:       TempDir,
    _worktrees:       TempDir,
    predecessor_root: PathBuf,
    successor_root:   PathBuf,
    predecessor_id:   String,
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
    OrderedFixture {
        repository,
        _worktrees: worktrees,
        predecessor_root,
        successor_root,
        predecessor_id,
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
    operation: serde_json::Value,
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

#[allow(
    clippy::needless_pass_by_value,
    reason = "callers construct one-use JSON operation fixtures inline"
)]
fn append_journal_operation_with_actor(
    repository_root: &Path,
    operation: serde_json::Value,
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

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
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
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
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
