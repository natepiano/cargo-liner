#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for proposal-bound overlap answers.

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
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
const GIT_BINARY: &str = "git";
const GIT_HEAD_REVISION: &str = "HEAD";
const GIT_LOOKUP_COMMAND: &str = "command -v git";
const GIT_NO_OPTIONAL_LOCKS_ARG: &str = "--no-optional-locks";
const GIT_REV_PARSE_COMMAND: &str = "rev-parse";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const MANUAL_EVENT_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e";
const PAUSED_GIT_WRAPPER_TIMEOUT: Duration = Duration::from_secs(60);
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SHELL_BINARY: &str = "sh";
const SHELL_COMMAND_ARG: &str = "-c";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

#[derive(Clone, Copy)]
struct AnswerReasons<'text> {
    purpose:       &'text str,
    authorization: &'text str,
}

impl<'text> AnswerReasons<'text> {
    const fn new(purpose: &'text str, authorization: &'text str) -> Self {
        Self {
            purpose,
            authorization,
        }
    }
}

struct PausedBerthProcess {
    child:             Child,
    continue_path:     PathBuf,
    ready_path:        PathBuf,
    wrapper_directory: TempDir,
}

impl PausedBerthProcess {
    fn spawn(repository_root: &Path, arguments: &[&str]) -> Self {
        let wrapper_directory = tempdir().expect("git wrapper directory should exist");
        let wrapper_path = wrapper_directory.path().join(GIT_BINARY);
        fs::write(&wrapper_path, paused_git_wrapper()).expect("git wrapper should write");
        let mut permissions = fs::metadata(&wrapper_path)
            .expect("git wrapper metadata should read")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
        let ready_path = wrapper_directory.path().join("ready");
        let continue_path = wrapper_directory.path().join("continue");
        let original_path = std::env::var_os("PATH").expect("test PATH should exist");
        let wrapped_path = std::env::join_paths(
            std::iter::once(wrapper_directory.path().to_path_buf())
                .chain(std::env::split_paths(&original_path)),
        )
        .expect("wrapped PATH should join");
        let child = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
            .args(arguments)
            .current_dir(repository_root)
            .env("PATH", wrapped_path)
            .env("CARGO_BERTH_TEST_GIT_READY", &ready_path)
            .env("CARGO_BERTH_TEST_GIT_CONTINUE", &continue_path)
            .env("CARGO_BERTH_TEST_REAL_GIT", git_binary())
            .env_remove(RUN_ENVIRONMENT)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("cargo-berth should start with paused git");
        Self {
            child,
            continue_path,
            ready_path,
            wrapper_directory,
        }
    }

    fn wait_until_paused(&mut self) { wait_for_path(&self.ready_path, &mut self.child); }

    fn continue_and_wait(self) -> Output {
        let Self {
            child,
            continue_path,
            ready_path: _,
            wrapper_directory,
        } = self;
        fs::write(continue_path, b"continue\n").expect("paused git should continue");
        let output = child
            .wait_with_output()
            .expect("cargo-berth should finish after git continues");
        std::mem::drop(wrapper_directory);
        output
    }
}

#[test]
fn proposal_round_trip_records_separate_reasons_and_exact_file_scope() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(
        repository.path(),
        "tree:crates",
        FIRST_RUN,
        "docs/holder.md",
        "holder-phase",
        "protect the holder tree",
    );
    let holder_id = reservation_id(&holder);
    let journal_before = journal_bytes(repository.path());

    let proposed = propose_answer(
        &second_root,
        "file:crates/a/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new(
            "protect only the requested implementation file",
            "the plans intentionally edit this file together",
        ),
    );
    let proposal_envelope = json_output(&proposed);

    assert_eq!(proposed.status.code(), Some(3));
    assert_complete_escalation(&proposal_envelope);
    assert_eq!(journal_bytes(repository.path()), journal_before);

    let changed_reason = apply_proposal(
        &second_root,
        "file:crates/a/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new(
            "protect only the requested implementation file",
            "a different authorization reason",
        ),
        proposal_token(&proposal_envelope),
    );
    assert_eq!(changed_reason.status.code(), Some(3));
    assert_eq!(journal_bytes(repository.path()), journal_before);

    let applied = apply_proposal(
        &second_root,
        "file:crates/a/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new(
            "protect only the requested implementation file",
            "the plans intentionally edit this file together",
        ),
        proposal_token(&proposal_envelope),
    );
    assert!(applied.status.success());
    let recorded = last_journal_event(repository.path());
    assert_eq!(recorded["purpose"]["kind"], "explained");
    assert_eq!(
        recorded["purpose"]["explanation"],
        "protect only the requested implementation file"
    );
    assert_eq!(recorded["authorization"]["kind"], "override");
    assert_eq!(recorded["authorization"]["blocker"], holder_id);
    assert_eq!(
        recorded["authorization"]["reason"],
        "the plans intentionally edit this file together"
    );
    assert_eq!(
        recorded["authorization"]["overlaps"][0]["scopes"][0]["kind"],
        "file"
    );

    assert!(
        check(repository.path(), &["file:crates/a/lib.rs"], FIRST_RUN)
            .status
            .success()
    );
    assert!(
        check(&second_root, &["file:crates/a/lib.rs"], SECOND_RUN)
            .status
            .success()
    );
    assert_eq!(
        check(&second_root, &["file:crates/a/sibling.rs"], SECOND_RUN)
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        check(&second_root, &["tree:crates/a"], SECOND_RUN)
            .status
            .code(),
        Some(1)
    );
}

#[test]
fn multi_path_first_touch_does_not_duplicate_answered_scopes() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(
        repository.path(),
        "file:src/lib.rs",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect the holder file",
    );
    let holder_id = reservation_id(&holder);
    let proposed = propose_answer(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--defer",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the integration order is not known yet",
        ),
    );
    let proposal_envelope = json_output(&proposed);
    assert_eq!(proposed.status.code(), Some(3));
    let applied = apply_proposal(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--defer",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the integration order is not known yet",
        ),
        proposal_token(&proposal_envelope),
    );
    assert!(applied.status.success());
    let answered_reservation_id = reservation_id(&applied);
    let answered_claim = last_journal_event(repository.path());
    assert_eq!(answered_claim["authorization"]["kind"], "defer");
    assert_eq!(
        answered_claim["scopes"],
        serde_json::json!([{"kind": "file", "path": "src/lib.rs"}])
    );

    assert!(
        check(
            &second_root,
            &["file:src/lib.rs", "file:src/free.rs"],
            SECOND_RUN,
        )
        .status
        .success()
    );
    let first_touch_reservation_id = assert_first_touch_append(repository.path());
    assert!(
        check(repository.path(), &["file:src/lib.rs"], FIRST_RUN)
            .status
            .success()
    );

    assert!(
        check(
            &second_root,
            &["file:src/lib.rs", "file:src/other.rs"],
            SECOND_RUN,
        )
        .status
        .success()
    );
    assert_first_touch_widen(repository.path(), &first_touch_reservation_id);
    assert!(
        check(repository.path(), &["file:src/lib.rs"], FIRST_RUN)
            .status
            .success()
    );

    assert_only_answer_reservation_acquires_scope(repository.path(), &answered_reservation_id);
    retire_answer_carrying_reservation(&second_root, &answered_reservation_id);
    let blocked = check(&second_root, &["file:src/lib.rs"], SECOND_RUN);
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(
        json_output(&blocked)["blocked_by"],
        serde_json::json!([holder_id])
    );
}

#[test]
fn checkpointed_first_touch_reservation_is_neither_reused_nor_widened() {
    let repository = initialized_repository();

    assert!(
        check(repository.path(), &["file:src/lib.rs"], SECOND_RUN)
            .status
            .success()
    );
    let checkpointed_reservation_id = reservation_id_from_last_journal_event(repository.path());
    assert!(
        run_berth(
            repository.path(),
            ["release", &checkpointed_reservation_id, "--json"],
        )
        .status
        .success()
    );

    assert!(
        check(repository.path(), &["file:src/lib.rs"], SECOND_RUN)
            .status
            .success()
    );
    let replacement = last_journal_event(repository.path());
    assert_eq!(replacement["op"], "claim");
    assert_eq!(replacement["source"]["kind"], "first_touch");
    assert_ne!(replacement["reservation_id"], checkpointed_reservation_id);
    let replacement_reservation_id = replacement["reservation_id"]
        .as_str()
        .expect("replacement reservation identifier should be a string")
        .to_owned();
    assert!(
        run_berth(
            repository.path(),
            ["release", &replacement_reservation_id, "--json"],
        )
        .status
        .success()
    );

    assert!(
        check(repository.path(), &["file:src/other.rs"], SECOND_RUN)
            .status
            .success()
    );
    let separate = last_journal_event(repository.path());
    assert_eq!(separate["op"], "claim");
    assert_eq!(separate["source"]["kind"], "first_touch");
    assert_ne!(separate["reservation_id"], checkpointed_reservation_id);
    assert_ne!(separate["reservation_id"], replacement_reservation_id);
    assert_eq!(
        separate["scopes"],
        serde_json::json!([{"kind": "file", "path": "src/other.rs"}])
    );
}

#[test]
fn unidentified_caller_can_mint_and_spend_its_proposal() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);

    let proposed = propose_answer_without_run(
        &second_root,
        "file:src/lib.rs",
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
    );
    let proposal_envelope = json_output(&proposed);
    assert_eq!(proposed.status.code(), Some(3));

    let applied = apply_proposal_without_run(
        &second_root,
        "file:src/lib.rs",
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        proposal_token(&proposal_envelope),
    );

    assert!(
        applied.status.success(),
        "unidentified proposal application failed: {}",
        String::from_utf8_lossy(&applied.stdout)
    );
}

#[test]
fn text_escalation_renders_explicit_holder_material() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim_explicit(repository.path(), "tree:src", FIRST_RUN, "protect source");
    let holder_id = reservation_id(&holder);

    let escalation = run_berth(
        &second_root,
        [
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            "--why",
            "protect the requester file",
            "--after",
            &holder_id,
            "--overlap-why",
            "the holder must integrate first",
        ],
    );
    let text = String::from_utf8(escalation.stdout).expect("text output should be UTF-8");

    assert_eq!(escalation.status.code(), Some(3));
    assert!(text.contains(&format!("Holder {holder_id}: explicit claim")));
    assert!(text.contains("shared scopes: file:src/lib.rs"));
    assert!(text.contains(&format!("direction: holder {holder_id} before requester")));
    assert!(text.contains("reason: the holder must integrate first"));
    assert!(text.contains("consequence: editing proceeds on the shown scopes"));
}

#[test]
fn every_permissive_answer_requires_a_reason_and_a_proposal() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect the holder source",
    );
    let holder_id = reservation_id(&holder);
    let journal_before = journal_bytes(repository.path());

    for answer in ["--before", "--after", "--defer", "--override"] {
        let missing_reason = run_berth(
            &second_root,
            [
                "claim",
                "file:src/lib.rs",
                "--run",
                SECOND_RUN,
                answer,
                &holder_id,
                "--why",
                "protect the requester file",
                "--json",
            ],
        );
        assert_eq!(missing_reason.status.code(), Some(5));

        let proposed = propose_answer(
            &second_root,
            "file:src/lib.rs",
            SECOND_RUN,
            answer,
            &holder_id,
            AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        );
        let envelope = json_output(&proposed);
        assert_eq!(proposed.status.code(), Some(3));
        assert_eq!(envelope["exit_code"], 3);
        assert_eq!(envelope["status"], "needs_user_authorization");
        assert!(
            envelope["message"]
                .as_str()
                .is_some_and(|message| message.contains("--proposal"))
        );
        assert!(
            envelope["payload"]["data"]["proposal_token"]
                .as_str()
                .is_some_and(|token| !token.is_empty())
        );
        assert_eq!(journal_bytes(repository.path()), journal_before);
    }
}

#[test]
fn renewal_preserves_a_proposal_while_scope_widening_invalidates_it() {
    let renewed_repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&renewed_repository, "second");
    let holder = claim(
        renewed_repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let proposed = propose_answer(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--after",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the holder must integrate first",
        ),
    );
    let proposal_envelope = json_output(&proposed);
    assert!(
        run_berth(renewed_repository.path(), ["renew", &holder_id, "--json"])
            .status
            .success()
    );
    let applied = apply_proposal(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--after",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the holder must integrate first",
        ),
        proposal_token(&proposal_envelope),
    );
    assert!(applied.status.success());

    let widened_repository = initialized_repository();
    let (_widened_second_directory, widened_second_root) =
        foreign_worktree(&widened_repository, "second");
    let holder = claim(
        widened_repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let proposed = propose_answer(
        &widened_second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
    );
    let proposal_envelope = json_output(&proposed);
    let old_token = proposal_token(&proposal_envelope).to_owned();
    append_widen(widened_repository.path(), &holder_id, "docs/new.rs");
    let journal_after_widen = journal_bytes(widened_repository.path());

    let stale = apply_proposal(
        &widened_second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        &old_token,
    );
    let stale_envelope = json_output(&stale);
    assert_eq!(stale.status.code(), Some(3));
    assert_ne!(proposal_token(&stale_envelope), old_token);
    assert_eq!(
        journal_bytes(widened_repository.path()),
        journal_after_widen
    );
}

#[test]
fn authorization_survives_holder_lifecycle_changes_but_not_scope_widening() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    git(repository.path(), ["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let proposed = propose_answer(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--after",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the holder must integrate first",
        ),
    );
    let proposal_envelope = json_output(&proposed);
    let applied = apply_proposal(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--after",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the holder must integrate first",
        ),
        proposal_token(&proposal_envelope),
    );
    assert!(applied.status.success());
    assert_eq!(
        last_journal_event(repository.path())["authorization"]["blocker"],
        holder_id
    );

    assert!(
        run_berth(repository.path(), ["renew", &holder_id, "--json"])
            .status
            .success()
    );
    assert!(
        check(&second_root, &["file:src/lib.rs"], SECOND_RUN)
            .status
            .success()
    );

    let checkpoint = run_berth(repository.path(), ["release", &holder_id, "--json"]);
    assert!(checkpoint.status.success());
    assert_eq!(
        json_output(&checkpoint)["payload"]["data"]["status"],
        "checkpointed"
    );
    assert!(
        check(&second_root, &["file:src/lib.rs"], SECOND_RUN)
            .status
            .success()
    );

    let revalidated = run_berth(repository.path(), ["release", &holder_id, "--json"]);
    assert!(revalidated.status.success());
    assert_eq!(
        json_output(&revalidated)["payload"]["data"]["status"],
        "evidence_revalidated"
    );
    assert!(
        check(&second_root, &["file:src/lib.rs"], SECOND_RUN)
            .status
            .success()
    );

    append_widen(repository.path(), &holder_id, "docs/new.rs");
    let reblocked = check(&second_root, &["file:src/lib.rs"], SECOND_RUN);
    assert_eq!(reblocked.status.code(), Some(1));
    assert_eq!(
        json_output(&reblocked)["blocked_by"],
        serde_json::json!([holder_id])
    );
}

#[test]
fn proposal_tokens_are_bound_to_the_holder_and_requester() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let first_holder = claim(
        repository.path(),
        "file:src/first.rs",
        FIRST_RUN,
        "docs/first-holder.md",
        "phase-a",
        "protect first",
    );
    let first_holder_id = reservation_id(&first_holder);
    let second_holder = claim(
        &third_root,
        "file:src/second.rs",
        THIRD_RUN,
        "docs/second-holder.md",
        "phase-c",
        "protect second",
    );
    let second_holder_id = reservation_id(&second_holder);
    let proposed = propose_answer(
        &second_root,
        "file:src/first.rs",
        SECOND_RUN,
        "--override",
        &first_holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
    );
    let proposal_envelope = json_output(&proposed);
    let token = proposal_token(&proposal_envelope);

    let different_holder = apply_proposal(
        &second_root,
        "file:src/second.rs",
        SECOND_RUN,
        "--override",
        &second_holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        token,
    );
    assert_eq!(different_holder.status.code(), Some(3));

    let third_requester = apply_proposal(
        &third_root,
        "file:src/first.rs",
        THIRD_RUN,
        "--override",
        &first_holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        token,
    );
    assert_eq!(third_requester.status.code(), Some(3));

    let same_holder_and_requester = apply_proposal(
        &second_root,
        "file:src/first.rs",
        SECOND_RUN,
        "--override",
        &first_holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        token,
    );
    assert!(same_holder_and_requester.status.success());
}

#[test]
fn permissive_answer_is_blocked_by_multiple_holders_without_minting() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let first_holder = claim(
        repository.path(),
        "file:src/first.rs",
        FIRST_RUN,
        "docs/first-holder.md",
        "phase-a",
        "protect first",
    );
    let first_holder_id = reservation_id(&first_holder);
    let second_holder = claim(
        &third_root,
        "file:src/second.rs",
        THIRD_RUN,
        "docs/second-holder.md",
        "phase-c",
        "protect second",
    );
    let second_holder_id = reservation_id(&second_holder);
    let journal_before = journal_bytes(repository.path());

    let blocked = propose_answer(
        &second_root,
        "tree:src",
        SECOND_RUN,
        "--override",
        &first_holder_id,
        AnswerReasons::new(
            "protect the requester tree",
            "the first overlap was reviewed",
        ),
    );
    let blocked_envelope = json_output(&blocked);

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(
        blocked_envelope["blocked_by"],
        serde_json::json!([first_holder_id, second_holder_id])
    );
    assert!(
        blocked_envelope
            .pointer("/payload/data/proposal_token")
            .is_none()
    );
    assert_eq!(journal_bytes(repository.path()), journal_before);
}

#[test]
fn permissive_answer_without_a_conflict_is_blocked_without_minting() {
    let repository = initialized_repository();
    let holder = claim(
        repository.path(),
        "file:src/held.rs",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect the held file",
    );
    let holder_id = reservation_id(&holder);
    let journal_before = journal_bytes(repository.path());

    let blocked = propose_answer(
        repository.path(),
        "file:src/unrelated.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new(
            "protect an unrelated requester file",
            "the named holder does not overlap",
        ),
    );
    let blocked_envelope = json_output(&blocked);

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(blocked_envelope["blocked_by"], serde_json::json!([]));
    assert!(
        blocked_envelope
            .pointer("/payload/data/proposal_token")
            .is_none()
    );
    assert_eq!(journal_bytes(repository.path()), journal_before);
}

#[test]
fn proposal_is_blocked_when_its_sole_holder_is_released() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let proposed = propose_answer(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
    );
    let proposal_envelope = json_output(&proposed);
    assert_eq!(proposed.status.code(), Some(3));
    let token = proposal_token(&proposal_envelope).to_owned();
    assert!(
        run_berth(repository.path(), ["release", &holder_id, "--json"])
            .status
            .success()
    );
    assert!(
        run_berth(repository.path(), ["release", &holder_id, "--json"])
            .status
            .success()
    );
    let journal_before_apply = journal_bytes(repository.path());

    let blocked = apply_proposal(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        &token,
    );
    let blocked_envelope = json_output(&blocked);

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(blocked_envelope["blocked_by"], serde_json::json!([]));
    assert!(
        blocked_envelope
            .pointer("/payload/data/proposal_token")
            .is_none()
    );
    assert_eq!(journal_bytes(repository.path()), journal_before_apply);
}

#[test]
fn single_holder_proposal_is_blocked_when_a_second_holder_appears() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let first_holder = claim(
        repository.path(),
        "file:src/first.rs",
        FIRST_RUN,
        "docs/first-holder.md",
        "phase-a",
        "protect first",
    );
    let first_holder_id = reservation_id(&first_holder);
    let proposed = propose_answer(
        &second_root,
        "tree:src",
        SECOND_RUN,
        "--override",
        &first_holder_id,
        AnswerReasons::new(
            "protect the requester tree",
            "the first overlap was reviewed",
        ),
    );
    let proposal_envelope = json_output(&proposed);
    assert_eq!(proposed.status.code(), Some(3));
    let token = proposal_token(&proposal_envelope).to_owned();

    let second_holder = claim(
        &third_root,
        "file:src/second.rs",
        THIRD_RUN,
        "docs/second-holder.md",
        "phase-c",
        "protect second",
    );
    let second_holder_id = reservation_id(&second_holder);
    let journal_before_apply = journal_bytes(repository.path());
    let blocked = apply_proposal(
        &second_root,
        "tree:src",
        SECOND_RUN,
        "--override",
        &first_holder_id,
        AnswerReasons::new(
            "protect the requester tree",
            "the first overlap was reviewed",
        ),
        &token,
    );
    let blocked_envelope = json_output(&blocked);

    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(
        blocked_envelope["blocked_by"],
        serde_json::json!([first_holder_id, second_holder_id])
    );
    assert!(
        blocked_envelope
            .pointer("/payload/data/proposal_token")
            .is_none()
    );
    assert_eq!(journal_bytes(repository.path()), journal_before_apply);
}

#[test]
fn authorization_claim_reports_reconciliation_alerts_on_its_own_envelope() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    git(repository.path(), ["add", ".claude/config/berth.toml"]);
    git(
        repository.path(),
        ["commit", "--quiet", "-m", "track berth config"],
    );
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let orphan_worktree = worktree_parent.path().join("orphan");
    let orphan_worktree_text = orphan_worktree
        .to_str()
        .expect("orphan worktree path should be UTF-8");
    git(
        repository.path(),
        [
            "worktree",
            "add",
            "--quiet",
            "-b",
            "orphan",
            orphan_worktree_text,
        ],
    );
    commit_file(
        &orphan_worktree,
        "orphan.txt",
        "orphan work\n",
        "orphan work",
    );
    let orphan_claim = claim(
        &orphan_worktree,
        "file:orphan.txt",
        THIRD_RUN,
        "docs/orphan.md",
        "phase-c",
        "protect orphan work",
    );
    let orphan_id = reservation_id(&orphan_claim);
    assert!(
        run_berth(&orphan_worktree, ["release", &orphan_id, "--json"])
            .status
            .success()
    );
    fs::remove_dir_all(&orphan_worktree).expect("orphan worktree should be removed");
    git(repository.path(), ["worktree", "prune", "--expire", "now"]);

    let proposed = propose_answer(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
    );
    let proposal_envelope = json_output(&proposed);
    assert_eq!(proposed.status.code(), Some(3));
    assert_eq!(
        proposal_envelope["payload"]["alerts"][0]["kind"],
        "orphaned_outstanding"
    );
    assert_eq!(
        proposal_envelope["payload"]["alerts"][0]["data"]["reservation_id"],
        orphan_id
    );

    let applied = apply_proposal(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
        proposal_token(&proposal_envelope),
    );
    let applied_envelope = json_output(&applied);
    assert!(applied.status.success());
    assert_eq!(
        applied_envelope["payload"]["alerts"][0]["kind"],
        "orphaned_outstanding"
    );
}

#[test]
fn authorization_claim_refuses_marker_identity_that_becomes_stale() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let marker_seed = claim(
        &second_root,
        "file:seed.txt",
        SECOND_RUN,
        "docs/requester.md",
        "seed-phase",
        "keep the requester marker active",
    );
    let marker_seed_id = reservation_id(&marker_seed);
    let proposed = propose_answer_without_run(
        &second_root,
        "file:src/lib.rs",
        "--override",
        &holder_id,
        AnswerReasons::new("protect the requester file", "the shared edit was reviewed"),
    );
    let proposal_envelope = json_output(&proposed);
    assert_eq!(proposed.status.code(), Some(3));
    let token = proposal_token(&proposal_envelope).to_owned();
    let arguments = [
        "claim",
        "file:src/lib.rs",
        "--plan",
        "docs/requester.md",
        "--phase",
        "requester-phase",
        "--why",
        "protect the requester file",
        "--override",
        holder_id.as_str(),
        "--overlap-why",
        "the shared edit was reviewed",
        "--proposal",
        token.as_str(),
        "--json",
    ];
    let mut applying_claim = PausedBerthProcess::spawn(&second_root, &arguments);
    applying_claim.wait_until_paused();
    assert!(
        run_berth(&second_root, ["release", &marker_seed_id, "--json"])
            .status
            .success()
    );

    let rejected = applying_claim.continue_and_wait();
    let rejected_envelope = json_output(&rejected);
    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(rejected_envelope["status"], "invalid_input");
    assert!(
        rejected_envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("no longer has an active reservation"))
    );
    assert_eq!(
        journal_events(repository.path())
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        2
    );
}

#[test]
fn answers_are_not_transitive_to_a_third_reservation() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let holder = claim(
        repository.path(),
        "tree:crates",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect crates",
    );
    let holder_id = reservation_id(&holder);
    let third = claim(
        &third_root,
        "file:docs/other.md",
        THIRD_RUN,
        "docs/third.md",
        "phase-c",
        "protect unrelated documentation",
    );
    let third_id = reservation_id(&third);
    let proposed = propose_answer(
        &second_root,
        "file:crates/a/lib.rs",
        SECOND_RUN,
        "--override",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the A and B overlap was reviewed",
        ),
    );
    let envelope = json_output(&proposed);
    assert!(
        apply_proposal(
            &second_root,
            "file:crates/a/lib.rs",
            SECOND_RUN,
            "--override",
            &holder_id,
            AnswerReasons::new(
                "protect the requester file",
                "the A and B overlap was reviewed",
            ),
            proposal_token(&envelope),
        )
        .status
        .success()
    );
    append_widen(repository.path(), &third_id, "crates/a/lib.rs");

    let blocked_by_third = check(&second_root, &["file:crates/a/lib.rs"], SECOND_RUN);
    let blocked_envelope = json_output(&blocked_by_third);
    assert_eq!(blocked_by_third.status.code(), Some(1));
    assert_eq!(
        blocked_envelope["blocked_by"],
        serde_json::json!([third_id])
    );
}

#[test]
fn defer_records_both_integration_holds_and_permits_both_editors() {
    let defer_repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&defer_repository, "second");
    let holder = claim(
        defer_repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    let proposed = propose_answer(
        &second_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "--defer",
        &holder_id,
        AnswerReasons::new(
            "protect the requester file",
            "the integration order is not known yet",
        ),
    );
    let envelope = json_output(&proposed);
    assert_eq!(
        envelope["payload"]["data"]["consequence"],
        "both_integrations_held"
    );
    assert!(
        apply_proposal(
            &second_root,
            "file:src/lib.rs",
            SECOND_RUN,
            "--defer",
            &holder_id,
            AnswerReasons::new(
                "protect the requester file",
                "the integration order is not known yet",
            ),
            proposal_token(&envelope),
        )
        .status
        .success()
    );
    assert_eq!(
        last_journal_event(defer_repository.path())["authorization"]["kind"],
        "defer"
    );
    assert_eq!(
        last_journal_event(defer_repository.path())["authorization"]["blocker"],
        holder_id
    );
    assert!(
        check(defer_repository.path(), &["file:src/lib.rs"], FIRST_RUN)
            .status
            .success()
    );
    assert!(
        check(&second_root, &["file:src/lib.rs"], SECOND_RUN)
            .status
            .success()
    );
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
        [
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
        ["init", "--quiet", "--initial-branch=main"],
    );
    git(repository.path(), ["config", "user.name", "Berth Test"]);
    git(
        repository.path(),
        ["config", "user.email", "berth@example.invalid"],
    );
    git(repository.path(), ["config", "core.ignoreCase", "false"]);
    fs::write(repository.path().join("README.md"), "scratch repository\n")
        .expect("scratch file should write");
    git(repository.path(), ["add", "README.md"]);
    git(repository.path(), ["commit", "--quiet", "-m", "initial"]);
    assert!(
        run_berth(repository.path(), ["init", "--json"])
            .status
            .success()
    );
    repository
}

fn claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    plan: &str,
    phase: &str,
    purpose: &str,
) -> Output {
    let output = run_berth(
        repository_root,
        [
            "claim", "--run", run, "--plan", plan, "--phase", phase, "--why", purpose, scope,
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

fn claim_explicit(repository_root: &Path, scope: &str, run: &str, purpose: &str) -> Output {
    let output = run_berth(
        repository_root,
        ["claim", scope, "--run", run, "--why", purpose, "--json"],
    );
    assert!(
        output.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

fn propose_answer(
    repository_root: &Path,
    scope: &str,
    run: &str,
    answer: &str,
    holder_id: &str,
    reasons: AnswerReasons<'_>,
) -> Output {
    run_berth(
        repository_root,
        [
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            "docs/requester.md",
            "--phase",
            "requester-phase",
            "--why",
            reasons.purpose,
            answer,
            holder_id,
            "--overlap-why",
            reasons.authorization,
            "--json",
        ],
    )
}

fn propose_answer_without_run(
    repository_root: &Path,
    scope: &str,
    answer: &str,
    holder_id: &str,
    reasons: AnswerReasons<'_>,
) -> Output {
    run_berth(
        repository_root,
        [
            "claim",
            scope,
            "--plan",
            "docs/requester.md",
            "--phase",
            "requester-phase",
            "--why",
            reasons.purpose,
            answer,
            holder_id,
            "--overlap-why",
            reasons.authorization,
            "--json",
        ],
    )
}

fn apply_proposal(
    repository_root: &Path,
    scope: &str,
    run: &str,
    answer: &str,
    holder_id: &str,
    reasons: AnswerReasons<'_>,
    proposal_token: &str,
) -> Output {
    run_berth(
        repository_root,
        [
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            "docs/requester.md",
            "--phase",
            "requester-phase",
            "--why",
            reasons.purpose,
            answer,
            holder_id,
            "--overlap-why",
            reasons.authorization,
            "--proposal",
            proposal_token,
            "--json",
        ],
    )
}

fn apply_proposal_without_run(
    repository_root: &Path,
    scope: &str,
    answer: &str,
    holder_id: &str,
    reasons: AnswerReasons<'_>,
    proposal_token: &str,
) -> Output {
    run_berth(
        repository_root,
        [
            "claim",
            scope,
            "--plan",
            "docs/requester.md",
            "--phase",
            "requester-phase",
            "--why",
            reasons.purpose,
            answer,
            holder_id,
            "--overlap-why",
            reasons.authorization,
            "--proposal",
            proposal_token,
            "--json",
        ],
    )
}

fn assert_first_touch_append(repository_root: &Path) -> String {
    let appended = last_journal_event(repository_root);
    assert_eq!(appended["op"], "claim");
    assert_eq!(appended["source"]["kind"], "first_touch");
    assert_eq!(appended["authorization"]["kind"], "no_conflict");
    assert_eq!(
        appended["scopes"],
        serde_json::json!([{"kind": "file", "path": "src/free.rs"}])
    );
    appended["reservation_id"]
        .as_str()
        .expect("first-touch reservation identifier should be a string")
        .to_owned()
}

fn assert_first_touch_widen(repository_root: &Path, first_touch_reservation_id: &str) {
    let widened = last_journal_event(repository_root);
    assert_eq!(widened["op"], "widen");
    assert_eq!(widened["reservation_id"], first_touch_reservation_id);
    assert_eq!(widened["authorization"]["kind"], "no_conflict");
    assert_eq!(
        widened["added_scopes"],
        serde_json::json!([{"kind": "file", "path": "src/other.rs"}])
    );
}

fn reservation_id_from_last_journal_event(repository_root: &Path) -> String {
    last_journal_event(repository_root)["reservation_id"]
        .as_str()
        .expect("reservation identifier should be a string")
        .to_owned()
}

fn assert_only_answer_reservation_acquires_scope(
    repository_root: &Path,
    answered_reservation_id: &str,
) {
    let answered_scope_acquisitions = journal_events(repository_root)
        .into_iter()
        .filter(|event| event["actor"]["run"] == SECOND_RUN)
        .filter(|event| {
            event["scopes"]
                .as_array()
                .or_else(|| event["added_scopes"].as_array())
                .is_some_and(|scopes| scopes.iter().any(|scope| scope["path"] == "src/lib.rs"))
        })
        .collect::<Vec<_>>();
    assert_eq!(answered_scope_acquisitions.len(), 1);
    assert_eq!(
        answered_scope_acquisitions[0]["reservation_id"],
        answered_reservation_id
    );
    assert_eq!(
        answered_scope_acquisitions[0]["authorization"]["kind"],
        "defer"
    );
}

fn retire_answer_carrying_reservation(repository_root: &Path, reservation_id: &str) {
    assert!(
        run_berth(repository_root, ["release", reservation_id, "--json"])
            .status
            .success()
    );
    let abandoned = run_berth(
        repository_root,
        [
            "resolve",
            reservation_id,
            "--abandon",
            "--why",
            "retire the answer-carrying reservation",
            "--json",
        ],
    );
    assert!(abandoned.status.success());
    assert_eq!(
        json_output(&abandoned)["payload"]["data"]["disposition"]["kind"],
        "abandoned"
    );
}

fn check(repository_root: &Path, scopes: &[&str], run: &str) -> Output {
    let arguments = std::iter::once("check")
        .chain(scopes.iter().copied())
        .chain(std::iter::once("--json"));
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env(RUN_ENVIRONMENT, run)
        .output()
        .expect("cargo-berth check should run")
}

fn commit_file(repository_root: &Path, path: &str, contents: &str, message: &str) {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("file parent should exist");
    }
    fs::write(file_path, contents).expect("committed file should write");
    git(repository_root, ["add", path]);
    git(repository_root, ["commit", "--quiet", "-m", message]);
}

fn append_widen(repository_root: &Path, reservation_id: &str, added_scope: &str) {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let events = journal_events(repository_root);
    let claim = events
        .iter()
        .find(|event| event["reservation_id"] == reservation_id)
        .expect("reservation claim should exist");
    let next_generation = events
        .iter()
        .map(|event| {
            event["projection_generation"]
                .as_u64()
                .expect("projection generation should be numeric")
        })
        .max()
        .expect("journal should contain a claim")
        + 1;
    let event = serde_json::json!({
        "schema_version": 1,
        "event_id": MANUAL_EVENT_ID,
        "actor": claim["actor"].clone(),
        "at": "2026-08-23T17:34:54.123Z",
        "projection_generation": next_generation,
        "op": "widen",
        "reservation_id": reservation_id,
        "added_scopes": [{"path": added_scope, "kind": "file"}],
        "cause": {"kind": "explicit", "reason": "test widens the reservation"},
        "authorization": {"kind": "no_conflict"},
        "edit_blocking_status": "blocking",
    });
    let mut journal = fs::OpenOptions::new()
        .append(true)
        .open(journal_path)
        .expect("journal should open for the synthetic widen");
    serde_json::to_writer(&mut journal, &event).expect("widen event should serialize");
    journal
        .write_all(b"\n")
        .expect("widen record terminator should write");
}

fn journal_events(repository_root: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .collect()
}

fn last_journal_event(repository_root: &Path) -> serde_json::Value {
    journal_events(repository_root)
        .into_iter()
        .last()
        .expect("journal should contain an event")
}

fn journal_bytes(repository_root: &Path) -> Vec<u8> {
    fs::read(repository_root.join(JOURNAL_PATH)).expect("journal should read")
}

fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

fn proposal_token(envelope: &serde_json::Value) -> &str {
    envelope["payload"]["data"]["proposal_token"]
        .as_str()
        .expect("authorization escalation should return a proposal token")
}

fn assert_complete_escalation(envelope: &serde_json::Value) {
    assert_eq!(envelope["exit_code"], 3);
    assert_eq!(envelope["status"], "needs_user_authorization");
    assert_eq!(envelope["payload"]["kind"], "claim");
    assert_eq!(
        envelope["payload"]["data"]["status"],
        "needs_user_authorization"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["plan"],
        "docs/holder.md"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["source"]["phase"],
        "holder-phase"
    );
    assert_eq!(
        envelope["payload"]["data"]["conflicts"][0]["overlapping_scopes"][0]["kind"],
        "file"
    );
    assert_eq!(
        envelope["payload"]["data"]["authorization_reason"],
        "the plans intentionally edit this file together"
    );
    assert_eq!(
        envelope["payload"]["data"]["proposal"]["requester"]["source"]["plan"],
        "docs/requester.md"
    );
    assert_eq!(
        envelope["payload"]["data"]["proposal"]["requester"]["source"]["phase"],
        "requester-phase"
    );
    assert_eq!(
        envelope["payload"]["data"]["proposal"]["requester"]["purpose"]["explanation"],
        "protect only the requested implementation file"
    );
}

fn run_berth<Arguments, Argument>(repository_root: &Path, arguments: Arguments) -> Output
where
    Arguments: IntoIterator<Item = Argument>,
    Argument: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn git<Arguments, Argument>(repository_root: &Path, arguments: Arguments)
where
    Arguments: IntoIterator<Item = Argument>,
    Argument: AsRef<OsStr>,
{
    let output = Command::new(GIT_BINARY)
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

fn paused_git_wrapper() -> String {
    format!(
        r#"#!/bin/sh
if [ "$1" = "{GIT_NO_OPTIONAL_LOCKS_ARG}" ] && [ "$2" = "{GIT_REV_PARSE_COMMAND}" ] && [ "$3" = "{GIT_HEAD_REVISION}" ]; then
    : > "$CARGO_BERTH_TEST_GIT_READY"
    while [ ! -e "$CARGO_BERTH_TEST_GIT_CONTINUE" ]; do
        sleep 0.01
    done
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#
    )
}

fn git_binary() -> String {
    let output = Command::new(SHELL_BINARY)
        .args([SHELL_COMMAND_ARG, GIT_LOOKUP_COMMAND])
        .output()
        .expect("git lookup should run");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git path should be UTF-8")
        .trim()
        .to_owned()
}

fn wait_for_path(path: &Path, child: &mut Child) {
    let deadline = Instant::now() + PAUSED_GIT_WRAPPER_TIMEOUT;
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !path.exists() {
        assert!(child.kill().is_ok(), "timed-out command should stop");
        assert!(child.wait().is_ok(), "stopped command should be reaped");
    }
    assert!(path.exists(), "git wrapper did not reach its pause point");
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}
