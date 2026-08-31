//! Built-binary acceptance tests for render-ready reservation responses.

use std::error::Error;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;
use tempfile::tempdir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const STALE_CLAIM_TIME: &str = "2020-01-01T00:00:00.000Z";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct RenderedBlock<'envelope> {
    summary: &'envelope str,
    detail:  &'envelope str,
}

#[derive(Clone, Copy)]
enum DeferredClaimApproval<'proposal> {
    AwaitingApproval,
    Approved(&'proposal str),
}

#[test]
fn blocked_claim_renders_every_holder_fact_and_first_touch_dispositions() -> TestResult {
    let repository = initialized_repository()?;
    let blocked = blocked_claim_with_three_source_kinds(&repository)?;
    let envelope = json_output(&blocked)?;
    assert_eq!(blocked.status.code(), Some(1));
    assert_eq!(envelope["status"], "blocked_by_overlap");
    assert_eq!(envelope["payload"]["data"]["status"], "blocked");
    let rendered_block = only_rendered_block(&envelope)?;
    assert!(!rendered_block.summary.contains('\n'));

    let conflicts = required_array(&envelope, "/payload/data/conflicts")?;
    assert_eq!(conflicts.len(), 3);
    assert_eq!(
        conflicts
            .iter()
            .filter(|conflict| conflict["activity"]["status"] == "active")
            .count(),
        2
    );
    assert_eq!(
        conflicts
            .iter()
            .filter(|conflict| conflict["activity"]["status"] == "quiet")
            .count(),
        1
    );
    assert_eq!(
        conflicts
            .iter()
            .filter(|conflict| conflict["head_snapshot"]["kind"] == "branch")
            .count(),
        2
    );
    assert_eq!(
        conflicts
            .iter()
            .filter(|conflict| conflict["head_snapshot"]["kind"] == "detached")
            .count(),
        1
    );
    for conflict in conflicts {
        assert_conflict_facts_are_rendered(conflict, rendered_block.detail)?;
    }
    assert_first_touch_holder_is_rendered(conflicts, rendered_block.detail)?;
    assert_explicit_holder_is_rendered(conflicts, rendered_block.detail)?;
    assert_work_plan_holder_is_rendered(conflicts, rendered_block.detail)?;
    assert!(
        rendered_block
            .detail
            .contains("1. **Land before the holder**")
    );
    assert!(rendered_block.detail.contains("5. **Leave it alone.**"));
    Ok(())
}

#[test]
fn claim_proposal_renders_approval_material_without_the_answer_menu() -> TestResult {
    let repository = initialized_repository()?;
    let holder = run_berth_with_run(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        FIRST_RUN,
    )?;
    require_success(&holder, "first-touch proposal holder")?;
    let holder_envelope = json_output(&holder)?;
    let holder_id = required_string(&holder_envelope, "/payload/data/acquisition/reservation_id")?;
    let (_requester_directory, requester_root) = add_worktree(&repository, "proposal-requester")?;

    let proposal = run_berth(
        &requester_root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            "--why",
            holder_id,
            "--after",
            holder_id,
            "--overlap-why",
            holder_id,
            "--json",
        ],
    )?;
    let envelope = json_output(&proposal)?;
    assert_eq!(proposal.status.code(), Some(3));
    assert_eq!(envelope["status"], "needs_user_authorization");
    assert_eq!(
        envelope["payload"]["data"]["status"],
        "needs_user_authorization"
    );
    let rendered_block = only_rendered_block(&envelope)?;
    let conflicts = required_array(&envelope, "/payload/data/conflicts")?;
    assert_eq!(conflicts.len(), 1);
    let conflict = conflicts
        .first()
        .ok_or_else(|| failure("proposal should carry its holder conflict"))?;
    assert_eq!(required_string(conflict, "/source/kind")?, "first_touch");
    assert_conflict_facts_are_rendered(conflict, rendered_block.detail)?;

    let proposal_token = required_string(&envelope, "/payload/data/proposal_token")?;
    let authorization_reason = required_string(&envelope, "/payload/data/authorization_reason")?;
    assert_eq!(authorization_reason, holder_id);
    assert!(rendered_block.detail.contains("file:src/lib.rs"));
    assert!(rendered_block.detail.contains(&format!(
        "- selected direction: holder {holder_id} before requester"
    )));
    assert!(
        rendered_block
            .detail
            .contains(&format!("- authorization reason: {authorization_reason}"))
    );
    assert!(
        rendered_block
            .detail
            .contains("- consequence: editing proceeds on the shown scopes")
    );
    assert!(rendered_block.detail.contains("- proposal:"));
    assert!(rendered_block.detail.contains("- transient token:"));
    assert!(rendered_block.detail.contains(proposal_token));
    assert!(rendered_block.detail.contains("--proposal"));
    assert!(
        rendered_block
            .detail
            .contains(&format!("cargo-berth release {holder_id}"))
    );
    assert!(
        rendered_block
            .detail
            .contains(&format!("cargo-berth resolve {holder_id} --integrated-as"))
    );
    assert!(
        rendered_block
            .detail
            .contains(&format!("cargo-berth resolve {holder_id} --abandon --why"))
    );
    assert!(
        !rendered_block
            .detail
            .to_ascii_lowercase()
            .contains("answers above")
    );
    for answer_title in [
        "1. **Land before the holder**",
        "2. **Land after the holder**",
        "3. **Defer the order**",
        "4. **Override**",
        "5. **Leave it alone.**",
    ] {
        assert!(!rendered_block.detail.contains(answer_title));
    }
    Ok(())
}

#[test]
fn claimed_presentation_names_its_published_reservation() -> TestResult {
    let repository = initialized_repository()?;
    let claimed = run_berth_with_session(
        repository.path(),
        &["claim", "file:published.rs", "--run", FIRST_RUN, "--json"],
        "published-claim-session",
    )?;
    require_success(&claimed, "published claim")?;
    let envelope = json_output(&claimed)?;
    assert_eq!(envelope["status"], "claimed");
    assert_eq!(
        envelope["payload"]["data"]["marker_publication"]["status"],
        "published"
    );
    assert_eq!(
        envelope["payload"]["data"]["session_mapping_publication"]["status"],
        "published"
    );
    let reservation_id = required_string(&envelope, "/payload/data/reservation_id")?;
    let rendered_block = only_rendered_block(&envelope)?;
    assert!(rendered_block.summary.contains(reservation_id));
    assert!(rendered_block.detail.contains(reservation_id));
    Ok(())
}

#[test]
fn degraded_claim_renders_durable_reservation_guidance() -> TestResult {
    let repository = initialized_repository()?;
    fs::create_dir(repository.path().join(SESSION_MAPPING_PATH))?;
    let claimed = run_berth_with_session(
        repository.path(),
        &["claim", "file:degraded.rs", "--run", FIRST_RUN, "--json"],
        "degraded-claim-session",
    )?;
    let envelope = json_output(&claimed)?;
    require_success(&claimed, "degraded claim")?;
    assert_eq!(envelope["status"], "claimed");
    assert_eq!(
        envelope["payload"]["data"]["session_mapping_publication"]["status"],
        "unavailable"
    );
    let reservation_id = required_string(&envelope, "/payload/data/reservation_id")?;
    let diagnostic = required_string(
        &envelope,
        "/payload/data/session_mapping_publication/diagnostic",
    )?;
    let rendered_block = only_rendered_block(&envelope)?;
    assert!(rendered_block.summary.contains(reservation_id));
    assert!(rendered_block.detail.contains(reservation_id));
    assert!(rendered_block.detail.contains(diagnostic));
    assert!(rendered_block.detail.contains(&format!(
        "The journal append and reservation `{reservation_id}` are durable"
    )));
    assert!(rendered_block.detail.contains(&format!(
        "Name reservation `{reservation_id}` explicitly on subsequent commands"
    )));
    Ok(())
}

#[test]
fn claimed_marker_failure_renders_its_diagnostic_and_restore_action() -> TestResult {
    let repository = initialized_repository()?;
    let (_worktree_directory, worktree_root) = add_worktree(&repository, "marker-failure")?;
    let seed = claim(&worktree_root, "file:marker-seed.rs", SECOND_RUN)?;
    let seed_envelope = json_output(&seed)?;
    let seed_id = required_string(&seed_envelope, "/payload/data/reservation_id")?;
    let released = run_berth(&worktree_root, &["release", seed_id, "--json"])?;
    require_success(&released, "marker fixture seed release")?;

    let git_directory = linked_worktree_git_directory(&worktree_root)?;
    let original_permissions = fs::metadata(&git_directory)?.permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(0o555);
    fs::set_permissions(&git_directory, read_only_permissions)?;
    let claimed = run_berth_with_session(
        &worktree_root,
        &[
            "claim",
            "file:marker-failure.rs",
            "--run",
            FIRST_RUN,
            "--json",
        ],
        "marker-failure-session",
    );
    fs::set_permissions(&git_directory, original_permissions)?;
    let claimed = claimed?;
    require_success(&claimed, "claim with unavailable marker")?;
    let envelope = json_output(&claimed)?;
    assert_eq!(
        envelope["payload"]["data"]["marker_publication"]["status"],
        "unavailable"
    );
    assert_eq!(
        envelope["payload"]["data"]["session_mapping_publication"]["status"],
        "published"
    );
    let reservation_id = required_string(&envelope, "/payload/data/reservation_id")?;
    let coordination_run_id = required_string(&envelope, "/payload/data/coordination_run_id")?;
    let diagnostic = required_string(&envelope, "/payload/data/marker_publication/diagnostic")?;
    let rendered_block = only_rendered_block(&envelope)?;
    assert!(rendered_block.summary.contains(reservation_id));
    assert!(rendered_block.detail.contains(reservation_id));
    assert!(rendered_block.detail.contains(diagnostic));
    assert_eq!(coordination_run_id, FIRST_RUN);
    assert!(rendered_block.detail.contains(&format!(
        "Restore coordination run {coordination_run_id} through the process environment before subsequent commands"
    )));
    Ok(())
}

#[test]
fn release_resolve_and_renew_presentations_are_engine_considered() -> TestResult {
    let repository = initialized_repository()?;
    let renewable = claim(repository.path(), "file:renewable.rs", FIRST_RUN)?;
    let renewable_envelope = json_output(&renewable)?;
    let renewable_id = required_string(&renewable_envelope, "/payload/data/reservation_id")?;

    let renewed = run_berth(repository.path(), &["renew", renewable_id, "--json"])?;
    require_success(&renewed, "renew")?;
    assert_renew_nothing_to_show(&json_output(&renewed)?)?;

    let resolved = run_berth(
        repository.path(),
        &[
            "resolve",
            renewable_id,
            "--abandon",
            "--why",
            renewable_id,
            "--json",
        ],
    )?;
    require_success(&resolved, "resolve")?;
    assert_nonempty_rendered_blocks(&json_output(&resolved)?, "resolve")?;

    let releasable = claim(repository.path(), "file:releasable.rs", SECOND_RUN)?;
    let releasable_envelope = json_output(&releasable)?;
    let releasable_id = required_string(&releasable_envelope, "/payload/data/reservation_id")?;
    let released = run_berth(repository.path(), &["release", releasable_id, "--json"])?;
    require_success(&released, "release")?;
    assert_nonempty_rendered_blocks(&json_output(&released)?, "release")
}

#[test]
fn sequence_presentation_is_engine_considered() -> TestResult {
    let repository = initialized_repository()?;
    let first = claim(repository.path(), "file:first.rs", FIRST_RUN)?;
    let first_envelope = json_output(&first)?;
    let first_id = required_string(&first_envelope, "/payload/data/reservation_id")?;
    let second = claim(repository.path(), "file:second.rs", SECOND_RUN)?;
    let second_envelope = json_output(&second)?;
    let second_id = required_string(&second_envelope, "/payload/data/reservation_id")?;

    let sequence = run_berth(
        repository.path(),
        &["sequence", first_id, second_id, "--why", first_id, "--json"],
    )?;
    assert_eq!(sequence.status.code(), Some(2));
    let envelope = json_output(&sequence)?;
    assert_eq!(envelope["payload"]["kind"], "sequence");
    assert_eq!(envelope["payload"]["data"]["status"], "rejected");
    assert_nonempty_rendered_blocks(&envelope, "sequence")
}

#[test]
fn integration_denial_has_a_one_line_summary_and_complete_detail() -> TestResult {
    let repository = initialized_repository()?;
    let (_holder_directory, holder_root) = add_worktree(&repository, "integration-holder")?;
    let holder = claim(&holder_root, "tree:src", FIRST_RUN)?;
    let holder_envelope = json_output(&holder)?;
    let holder_id = required_string(&holder_envelope, "/payload/data/reservation_id")?;

    let (_blocked_directory, blocked_root) = add_worktree(&repository, "integration-blocked")?;
    let proposal = deferred_claim(
        &blocked_root,
        holder_id,
        DeferredClaimApproval::AwaitingApproval,
    )?;
    assert_eq!(proposal.status.code(), Some(3));
    let proposal_envelope = json_output(&proposal)?;
    let proposal_token = required_string(&proposal_envelope, "/payload/data/proposal_token")?;
    let blocked = deferred_claim(
        &blocked_root,
        holder_id,
        DeferredClaimApproval::Approved(proposal_token),
    )?;
    require_success(&blocked, "deferred claim")?;
    let blocked_envelope = json_output(&blocked)?;
    let blocked_id = required_string(&blocked_envelope, "/payload/data/reservation_id")?;
    commit_file(&blocked_root, "src/lib.rs", "pub fn blocked_work() {}\n")?;
    enable_enforcing_gate(repository.path())?;
    enable_enforcing_gate(&blocked_root)?;

    let denied = run_berth(&blocked_root, &["integrate", blocked_id, "--json"])?;
    assert_eq!(denied.status.code(), Some(2));
    let envelope = json_output(&denied)?;
    assert_eq!(envelope["status"], "blocked_by_ordering");
    let rendered_block = only_rendered_block(&envelope)?;
    assert_eq!(
        rendered_block.summary,
        format!("cargo-berth refused integration for reservation {blocked_id}.")
    );
    assert!(!rendered_block.summary.contains('\n'));
    assert!(rendered_block.detail.contains(&format!(
        "Reservation {blocked_id} cannot enter main while its integration order is held."
    )));
    let entering_reservation_line = required_detail_line(
        rendered_block.detail,
        &format!("Entering reservation {blocked_id}:"),
    )?;
    assert!(entering_reservation_line.contains("; purpose: "));
    assert!(entering_reservation_line.contains("; protected paths: "));
    let blocking_reservation_line = required_detail_line(
        rendered_block.detail,
        &format!("Blocking reservation {holder_id}:"),
    )?;
    assert!(blocking_reservation_line.contains("; purpose: "));
    assert!(blocking_reservation_line.contains("; protected paths: "));
    let integration_hold_line = required_detail_line(
        rendered_block.detail,
        &format!("Unresolved deferral with reservation {holder_id};"),
    )?;
    assert!(integration_hold_line.contains("covered paths: file:src/lib.rs;"));
    assert!(integration_hold_line.contains(&format!("recorded reason: {holder_id};")));
    assert!(integration_hold_line.ends_with(&format!(
        "recovery: cargo-berth sequence {holder_id} {blocked_id} --why \"{holder_id}\"."
    )));
    let emergency_recovery_line = required_detail_line(
        rendered_block.detail,
        &format!("To deliberately proceed once: cargo-berth integrate {blocked_id}"),
    )?;
    assert_eq!(
        emergency_recovery_line,
        format!(
            "To deliberately proceed once: cargo-berth integrate {blocked_id} --force --why \"<reason>\". Last resort: CARGO_BERTH_BYPASS=1 <git command>."
        )
    );
    Ok(())
}

#[test]
fn successful_integrate_presentation_is_engine_considered() -> TestResult {
    let repository = initialized_repository()?;
    let (_phase_directory, phase_root) = add_worktree(&repository, "integration-phase")?;
    let claimed = claim(&phase_root, "file:integrated.rs", FIRST_RUN)?;
    let claimed_envelope = json_output(&claimed)?;
    let reservation_id = required_string(&claimed_envelope, "/payload/data/reservation_id")?;
    commit_file(&phase_root, "integrated.rs", "integrated work\n")?;

    let checkpoint = run_berth(&phase_root, &["release", reservation_id, "--json"])?;
    require_success(&checkpoint, "integration checkpoint")?;
    let integrated = run_berth(&phase_root, &["integrate", reservation_id, "--json"])?;
    require_success(&integrated, "integrate")?;
    let envelope = json_output(&integrated)?;
    assert_eq!(envelope["status"], "integrated");
    assert_eq!(envelope["payload"]["data"]["status"], "integrated");
    assert_nonempty_rendered_blocks(&envelope, "integrate")
}

fn assert_first_touch_holder_is_rendered(conflicts: &[Value], detail: &str) -> TestResult {
    let conflict = conflict_with_source(conflicts, "first_touch")?;
    let reservation_id = required_string(conflict, "/reservation_id")?;
    let section = holder_section(detail, reservation_id)?;
    let last_activity = required_string(conflict, "/activity/last_activity_at")?;
    assert_eq!(required_string(conflict, "/activity/status")?, "quiet");
    assert_eq!(
        required_string(conflict, "/head_snapshot/kind")?,
        "detached"
    );
    assert!(section.contains(&format!(
        "- activity: gone quiet; last activity at {last_activity}"
    )));
    assert!(section.contains("first-touch edit"));
    assert!(conflict["source"].get("plan").is_none());
    assert!(conflict["source"].get("phase").is_none());
    assert!(!section.contains("explicit claim"));
    assert!(!section.contains("- acquisition source: plan "));
    assert!(!section.contains(", phase "));
    assert!(detail.contains(&format!("cargo-berth release {reservation_id}")));
    assert!(detail.contains(&format!(
        "cargo-berth resolve {reservation_id} --integrated-as"
    )));
    assert!(detail.contains(&format!(
        "cargo-berth resolve {reservation_id} --abandon --why"
    )));
    Ok(())
}

fn assert_explicit_holder_is_rendered(conflicts: &[Value], detail: &str) -> TestResult {
    let conflict = conflict_with_source(conflicts, "explicit")?;
    let reservation_id = required_string(conflict, "/reservation_id")?;
    let section = holder_section(detail, reservation_id)?;
    let last_activity = required_string(conflict, "/activity/last_activity_at")?;
    assert!(conflict["source"].get("plan").is_none());
    assert!(conflict["source"].get("phase").is_none());
    assert_eq!(required_string(conflict, "/head_snapshot/kind")?, "branch");
    assert!(section.contains(&format!(
        "- activity: active; last activity at {last_activity}"
    )));
    assert!(section.contains("explicit claim"));
    assert!(section.contains("explicit holder purpose"));
    assert!(!section.contains("- acquisition source: plan "));
    assert!(!section.contains(", phase "));
    Ok(())
}

fn assert_work_plan_holder_is_rendered(conflicts: &[Value], detail: &str) -> TestResult {
    let conflict = conflict_with_source(conflicts, "work_plan")?;
    let reservation_id = required_string(conflict, "/reservation_id")?;
    let section = holder_section(detail, reservation_id)?;
    let last_activity = required_string(conflict, "/activity/last_activity_at")?;
    assert_eq!(conflict["source"]["plan"], "docs/holder-plan.md");
    assert_eq!(conflict["source"]["phase"], "rendering-phase");
    assert!(section.contains(&format!(
        "- activity: active; last activity at {last_activity}"
    )));
    assert!(section.contains("docs/holder-plan.md"));
    assert!(section.contains("rendering-phase"));
    assert!(section.contains("planned holder purpose"));
    Ok(())
}

fn assert_conflict_facts_are_rendered(conflict: &Value, detail: &str) -> TestResult {
    let reservation_id = required_string(conflict, "/reservation_id")?;
    let section = holder_section(detail, reservation_id)?;
    for pointer in ["/holder_worktree_id", "/holder_run_id", "/claimed_at"] {
        let fact = required_string(conflict, pointer)?;
        assert!(
            section.contains(fact),
            "rendered holder section omitted {pointer}: {section}"
        );
    }
    assert_head_snapshot_is_rendered(conflict, section)?;
    assert_activity_is_rendered(conflict, section)?;
    assert_source_is_rendered(conflict, section)?;
    assert_purpose_is_rendered(conflict, section)?;
    let exact_scopes = exact_scope_text(conflict)?;
    assert!(
        section.contains(&format!("- exact shared scopes: {exact_scopes}")),
        "rendered holder section changed exact shared scopes: {section}"
    );
    Ok(())
}

fn assert_head_snapshot_is_rendered(conflict: &Value, section: &str) -> TestResult {
    let branch_or_head = match required_string(conflict, "/head_snapshot/kind")? {
        "branch" => required_string(conflict, "/head_snapshot/full_ref")?.to_owned(),
        "detached" => format!(
            "detached at {}",
            required_string(conflict, "/head_snapshot/head")?
        ),
        head_kind => {
            return Err(failure(format!(
                "unexpected head snapshot kind {head_kind}"
            )));
        },
    };
    assert!(section.contains(&format!("- branch or detached head: `{branch_or_head}`")));
    Ok(())
}

fn assert_activity_is_rendered(conflict: &Value, section: &str) -> TestResult {
    let activity = match required_string(conflict, "/activity/status")? {
        "active" => "active",
        "quiet" => "gone quiet",
        activity_status => {
            return Err(failure(format!(
                "unexpected holder activity status {activity_status}"
            )));
        },
    };
    let last_activity_at = required_string(conflict, "/activity/last_activity_at")?;
    assert!(section.contains(&format!(
        "- activity: {activity}; last activity at {last_activity_at}"
    )));
    Ok(())
}

fn assert_source_is_rendered(conflict: &Value, section: &str) -> TestResult {
    match required_string(conflict, "/source/kind")? {
        "work_plan" => {
            let plan = required_string(conflict, "/source/plan")?;
            let phase = required_string(conflict, "/source/phase")?;
            assert!(section.contains(&format!("- acquisition source: plan {plan}, phase {phase}")));
        },
        "explicit" => {
            assert!(section.contains("- acquisition source: explicit claim"));
            assert!(!section.contains("- acquisition source: plan "));
            assert!(!section.contains(", phase "));
        },
        "first_touch" => {
            assert!(section.contains("- acquisition source: first-touch edit"));
            assert!(!section.contains("explicit claim"));
            assert!(!section.contains("- acquisition source: plan "));
            assert!(!section.contains(", phase "));
        },
        source_kind => {
            return Err(failure(format!(
                "unexpected reservation source kind {source_kind}"
            )));
        },
    }
    Ok(())
}

fn assert_purpose_is_rendered(conflict: &Value, section: &str) -> TestResult {
    match required_string(conflict, "/purpose/kind")? {
        "explained" => {
            assert!(section.contains(required_string(conflict, "/purpose/explanation")?));
        },
        "not_provided_by_caller" => {
            assert!(section.contains("no reason provided by caller"));
        },
        purpose_kind => {
            return Err(failure(format!(
                "unexpected reservation purpose kind {purpose_kind}"
            )));
        },
    }
    Ok(())
}

fn exact_scope_text(conflict: &Value) -> TestResult<String> {
    required_array(conflict, "/overlapping_scopes")?
        .iter()
        .map(|scope| {
            let kind = required_string(scope, "/kind")?;
            let path = required_string(scope, "/path")?;
            Ok(format!("{kind}:{path}"))
        })
        .collect::<TestResult<Vec<_>>>()
        .map(|scopes| scopes.join(", "))
}

fn assert_nonempty_rendered_blocks(envelope: &Value, expected_payload_kind: &str) -> TestResult {
    assert_eq!(
        required_string(envelope, "/payload/kind")?,
        expected_payload_kind
    );
    let blocks = rendered_blocks(envelope)?;
    if blocks.is_empty() {
        return Err(failure(format!(
            "{expected_payload_kind} must publish at least one rendered block: {envelope}"
        )));
    }
    for block in blocks {
        let summary = required_string(block, "/summary")?;
        let detail = required_string(block, "/detail")?;
        assert!(!summary.is_empty());
        assert!(!summary.contains('\n'));
        assert!(!detail.is_empty());
    }
    Ok(())
}

fn assert_renew_nothing_to_show(envelope: &Value) -> TestResult {
    assert_eq!(required_string(envelope, "/payload/kind")?, "renew");
    let blocks = rendered_blocks(envelope)?;
    if blocks.is_empty() {
        Ok(())
    } else {
        Err(failure(format!(
            "renew should have no user action, found {} rendered blocks",
            blocks.len()
        )))
    }
}

fn rendered_blocks(envelope: &Value) -> TestResult<&[Value]> {
    let presentation_kind = required_string(envelope, "/presentation/kind")?;
    assert_ne!(
        presentation_kind, "not_provided",
        "contracted reservation fact serialized an unprovided presentation: {envelope}"
    );
    if presentation_kind != "rendered_blocks" {
        return Err(failure(format!(
            "engine left presentation unprovided: {}",
            envelope["presentation"]
        )));
    }
    required_array(envelope, "/presentation/blocks")
}

fn only_rendered_block(envelope: &Value) -> TestResult<RenderedBlock<'_>> {
    let blocks = rendered_blocks(envelope)?;
    let [block] = blocks else {
        return Err(failure(format!(
            "expected one rendered presentation block, found {}",
            blocks.len()
        )));
    };
    Ok(RenderedBlock {
        summary: required_string(block, "/summary")?,
        detail:  required_string(block, "/detail")?,
    })
}

fn conflict_with_source<'conflicts>(
    conflicts: &'conflicts [Value],
    source_kind: &str,
) -> TestResult<&'conflicts Value> {
    conflicts
        .iter()
        .find(|conflict| conflict["source"]["kind"] == source_kind)
        .ok_or_else(|| failure(format!("missing {source_kind} conflict")))
}

fn holder_section<'detail>(detail: &'detail str, reservation_id: &str) -> TestResult<&'detail str> {
    let start = detail
        .find(&format!("Holder `{reservation_id}`:"))
        .ok_or_else(|| failure(format!("missing holder section for {reservation_id}")))?;
    let remaining = &detail[start..];
    let end = remaining[1..]
        .find("\n\nHolder `")
        .map_or(remaining.len(), |offset| offset + 1);
    Ok(&remaining[..end])
}

fn age_only_journal_event(repository_root: &Path) -> TestResult {
    let journal_path = repository_root.join(JOURNAL_PATH);
    let journal = fs::read_to_string(&journal_path)?;
    let mut events = journal
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    let [event] = events.as_mut_slice() else {
        return Err(failure(format!(
            "expected one first-touch journal event, found {}",
            events.len()
        )));
    };
    event["at"] = Value::String(STALE_CLAIM_TIME.to_owned());
    let mut serialized = serde_json::to_vec(event)?;
    serialized.push(b'\n');
    fs::write(journal_path, serialized)?;
    let projection_path = repository_root.join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path)?;
    }
    Ok(())
}

fn blocked_claim_with_three_source_kinds(repository: &TempDir) -> TestResult<Output> {
    run_git(
        repository.path(),
        &["checkout", "--quiet", "--detach", "HEAD"],
    )?;
    let first_touch = run_berth_with_run(
        repository.path(),
        &["check", "file:first-touch.rs", "--json"],
        FIRST_RUN,
    )?;
    require_success(&first_touch, "first-touch claim")?;
    age_only_journal_event(repository.path())?;

    let (_explicit_directory, explicit_root) = add_worktree(repository, "explicit-holder")?;
    let explicit = run_berth(
        &explicit_root,
        &[
            "claim",
            "file:explicit.rs",
            "--run",
            SECOND_RUN,
            "--why",
            "explicit holder purpose",
            "--json",
        ],
    )?;
    require_success(&explicit, "explicit claim")?;

    let (_planned_directory, planned_root) = add_worktree(repository, "planned-holder")?;
    let planned = run_berth(
        &planned_root,
        &[
            "claim",
            "file:planned.rs",
            "--run",
            THIRD_RUN,
            "--plan",
            "docs/holder-plan.md",
            "--phase",
            "rendering-phase",
            "--why",
            "planned holder purpose",
            "--json",
        ],
    )?;
    require_success(&planned, "work-plan claim")?;

    let (_requester_directory, requester_root) = add_worktree(repository, "requester")?;
    run_berth(
        &requester_root,
        &[
            "claim",
            "file:first-touch.rs",
            "file:explicit.rs",
            "file:planned.rs",
            "--json",
        ],
    )
}

fn claim(repository_root: &Path, scope: &str, run: &str) -> TestResult<Output> {
    let output = run_berth(repository_root, &["claim", scope, "--run", run, "--json"])?;
    require_success(&output, "claim")?;
    let envelope = json_output(&output)?;
    let reservation_id = required_string(&envelope, "/payload/data/reservation_id")?;
    let rendered_block = only_rendered_block(&envelope)?;
    assert!(rendered_block.summary.contains(reservation_id));
    assert!(rendered_block.detail.contains(reservation_id));
    Ok(output)
}

fn deferred_claim(
    repository_root: &Path,
    holder_id: &str,
    approval: DeferredClaimApproval<'_>,
) -> TestResult<Output> {
    let mut arguments = vec![
        "claim",
        "file:src/lib.rs",
        "--run",
        SECOND_RUN,
        "--defer",
        holder_id,
        "--overlap-why",
        holder_id,
        "--why",
        holder_id,
    ];
    match approval {
        DeferredClaimApproval::AwaitingApproval => {},
        DeferredClaimApproval::Approved(proposal_token) => {
            arguments.extend(["--proposal", proposal_token]);
        },
    }
    arguments.push("--json");
    run_berth(repository_root, &arguments)
}

fn commit_file(repository_root: &Path, path: &str, contents: &str) -> TestResult {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file_path, contents)?;
    run_git(repository_root, &["add", path])?;
    run_git(repository_root, &["commit", "--quiet", "-m", path])
}

fn initialized_repository() -> TestResult<TempDir> {
    let repository = tempdir()?;
    run_git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    )?;
    run_git(
        repository.path(),
        &["config", "user.name", "Cargo Berth Test"],
    )?;
    run_git(
        repository.path(),
        &["config", "user.email", "cargo-berth@example.com"],
    )?;
    fs::write(repository.path().join("README.md"), "initial\n")?;
    run_git(repository.path(), &["add", "README.md"])?;
    run_git(repository.path(), &["commit", "--quiet", "-m", "initial"])?;
    let initialized = run_berth(repository.path(), &["init", "--json"])?;
    require_success(&initialized, "cargo-berth init")?;
    Ok(repository)
}

fn add_worktree(repository: &TempDir, branch: &str) -> TestResult<(TempDir, PathBuf)> {
    let directory = tempdir()?;
    let worktree_root = directory.path().join(branch);
    let worktree_path = worktree_root
        .to_str()
        .ok_or_else(|| failure("worktree path should be valid UTF-8"))?;
    run_git(
        repository.path(),
        &["worktree", "add", "--quiet", "-b", branch, worktree_path],
    )?;
    let configuration_path = worktree_root.join(CONFIGURATION_PATH);
    let configuration_directory = configuration_path
        .parent()
        .ok_or_else(|| failure("berth configuration should have a parent directory"))?;
    fs::create_dir_all(configuration_directory)?;
    fs::copy(
        repository.path().join(CONFIGURATION_PATH),
        configuration_path,
    )?;
    Ok((directory, worktree_root))
}

fn linked_worktree_git_directory(worktree_root: &Path) -> TestResult<PathBuf> {
    let git_file = fs::read_to_string(worktree_root.join(".git"))?;
    let path = git_file
        .strip_prefix("gitdir: ")
        .map(str::trim)
        .ok_or_else(|| failure("linked worktree .git file should name its git directory"))?;
    let git_directory = PathBuf::from(path);
    if git_directory.is_absolute() {
        Ok(git_directory)
    } else {
        Ok(worktree_root.join(git_directory))
    }
}

fn enable_enforcing_gate(repository_root: &Path) -> TestResult {
    let configuration_path = repository_root.join(CONFIGURATION_PATH);
    let configuration = fs::read_to_string(&configuration_path)?;
    let updated = configuration
        .lines()
        .map(|line| {
            if line.starts_with("gate_mode") {
                "gate_mode = \"enforce\"".to_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(configuration_path, format!("{updated}\n"))?;
    Ok(())
}

fn run_git(repository_root: &Path, arguments: &[&str]) -> TestResult {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove(SESSION_ENVIRONMENT)
        .output()?)
}

fn run_berth_with_run(repository_root: &Path, arguments: &[&str], run: &str) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env("CARGO_BERTH_RUN", run)
        .env_remove(SESSION_ENVIRONMENT)
        .output()?)
}

fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()?)
}

fn require_success(output: &Output, operation: &str) -> TestResult {
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "{operation} failed with {:?}: stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn json_output(output: &Output) -> TestResult<Value> {
    serde_json::from_slice(&output.stdout).map_err(|error| {
        failure(format!(
            "cargo-berth stdout should be JSON: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })
}

fn required_string<'value>(value: &'value Value, pointer: &str) -> TestResult<&'value str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{pointer} should be a string in {value}")))
}

fn required_detail_line<'detail>(detail: &'detail str, prefix: &str) -> TestResult<&'detail str> {
    detail
        .lines()
        .find(|line| line.starts_with(prefix))
        .ok_or_else(|| {
            failure(format!(
                "missing rendered detail line starting with {prefix}"
            ))
        })
}

fn required_array<'value>(value: &'value Value, pointer: &str) -> TestResult<&'value [Value]> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| failure(format!("{pointer} should be an array in {value}")))
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}
