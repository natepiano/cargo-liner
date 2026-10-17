#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! A director can coordinate parallel phases without combining their reservations.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Output;
use std::process::Stdio;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;
use cargo_berth_test_support::reservation_row;
use serde_json::Value;
use tempfile::TempDir;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const DIRECTOR_SESSION: &str = "dir-sid";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &["CARGO_BERTH_RUN", "CARGO_BERTH_SESSION_ID"],
};

#[test]
fn a_director_runs_two_phases_each_with_its_own_reservation() {
    let repository = initialized_repository();
    let root = repository.path();

    let first = run_berth_with_session(root, &["check", "file:a.rs", "--json"], "dir-sid.phase-15");
    assert_success(&first);
    let first = json(&first);
    let first_acquisition = &first["payload"]["data"]["acquisition"];
    let r15 = first_acquisition["reservation_id"]
        .as_str()
        .expect("phase 15 should name its reservation");
    assert_eq!(first_acquisition["kind"], "appended");
    let run = first_acquisition["coordination_run_id"]
        .as_str()
        .expect("phase 15 should name its coordination run");

    let second =
        run_berth_with_session(root, &["check", "file:b.rs", "--json"], "dir-sid.phase-16");
    assert_success(&second);
    let second = json(&second);
    let second_acquisition = &second["payload"]["data"]["acquisition"];
    let r16 = second_acquisition["reservation_id"]
        .as_str()
        .expect("phase 16 should name its reservation");
    assert_ne!(r15, r16, "a new phase must receive its own reservation");
    assert_eq!(second_acquisition["kind"], "appended");
    assert_eq!(second_acquisition["coordination_run_id"], run);

    let board = run_berth_with_session(root, &["board", "--json"], DIRECTOR_SESSION);
    assert_success(&board);
    let board = json(&board);
    assert_eq!(
        reservation_row(&board, r15)["scopes"],
        serde_json::json!([{"kind": "file", "path": "a.rs"}]),
        "opening phase 16 must leave phase 15's declaration alone"
    );

    let mapped = run_berth_with_session(
        root,
        &["check", "--reservation", r15, "file:a.rs", "--json"],
        DIRECTOR_SESSION,
    );
    assert_success(&mapped);
    assert_eq!(
        json(&mapped)["payload"]["data"]["acquisition"]["reservation_id"],
        r15
    );
    let mappings = session_mappings(root);
    assert_eq!(
        mappings["identities"][DIRECTOR_SESSION]["reservation_id"],
        r15
    );
    assert_eq!(
        mappings["identities"]["dir-sid.phase-15"]["reservation_id"],
        r15
    );
    assert_eq!(
        mappings["identities"]["dir-sid.phase-16"]["reservation_id"],
        r16
    );

    fs::write(root.join("c.rs"), "// unheld Bash write\n")
        .expect("Bash's unheld file should write");
    let before_hook = journal_events(root);
    let hook = run_post_tool_use(root);
    assert_success(&hook);
    let after_hook = journal_events(root);
    assert!(
        !after_hook[before_hook.len()..]
            .iter()
            .any(|event| event["op"] == "widen"),
        "the director's session mapping must not attribute an unheld path to either phase"
    );
    let answer = json(&hook);
    let detail = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("ambiguous drift should explain its candidates to the director");
    for expected in ["c.rs", r15, r16] {
        assert!(
            detail.contains(expected),
            "missing {expected} in drift answer: {detail}"
        );
    }

    assert_unclaimed_work_keeps_phase_outstanding(root, r15, r16);
}

fn assert_unclaimed_work_keeps_phase_outstanding(root: &Path, r15: &str, r16: &str) {
    let checkpoint = run_berth_with_session(root, &["release", r15, "--json"], "dir-sid.phase-15");
    assert_success(&checkpoint);
    let checkpoint = json(&checkpoint);
    assert_eq!(checkpoint["status"], "outstanding", "{checkpoint}");
    assert_eq!(checkpoint["payload"]["data"]["status"], "checkpointed");

    let pending = run_berth_with_session(root, &["release", r15, "--json"], "dir-sid.phase-15");
    assert_success(&pending);
    let pending = json(&pending);
    assert_eq!(pending["status"], "outstanding", "{pending}");
    assert_eq!(pending["payload"]["data"]["status"], "outstanding");
    assert_eq!(
        pending["payload"]["data"]["pending_paths"],
        serde_json::json!(["c.rs"])
    );

    let board = phase_board(root);
    let checkpointed = reservation_row(&board, r15);
    assert_eq!(checkpointed["lifecycle"]["stage"], "outstanding");
    assert_eq!(
        checkpointed["merge_extent"]["scopes"],
        serde_json::json!([{"kind": "file", "path": "c.rs"}])
    );
    let remaining = reservation_row(&board, r16);
    assert_eq!(remaining["lifecycle"]["stage"], "active");
    assert_eq!(
        remaining["scopes"],
        serde_json::json!([{"kind": "file", "path": "b.rs"}]),
        "checkpointing phase 15 must leave phase 16 active with its original declaration"
    );
    assert!(
        !journal_events(root)
            .iter()
            .any(|event| event["op"] == "release" && event["reservation_id"] == r15),
        "unclaimed dirt must prevent phase 15 from settling"
    );
}

#[test]
fn a_fresh_session_reports_left_unmapped_when_siblings_hold_every_requested_path() {
    let repository = initialized_repository();
    let root = repository.path();
    let first = run_berth_with_session(root, &["check", "file:a.rs", "--json"], "dir-sid.phase-15");
    assert_success(&first);
    let first = json(&first);
    let reservation = &first["payload"]["data"]["acquisition"]["reservation_id"];
    let events_before = journal_events(root);

    let covered =
        run_berth_with_session(root, &["check", "file:a.rs", "--json"], "dir-sid.phase-16");
    assert_success(&covered);
    let covered = json(&covered);
    let acquisition = &covered["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "already_held");
    assert_eq!(&acquisition["reservation_id"], reservation);
    assert_eq!(
        acquisition["session_mapping_publication"],
        serde_json::json!({"status": "left_unmapped"})
    );
    let mappings = session_mappings(root);
    assert!(mappings["identities"].get("dir-sid.phase-16").is_none());
    assert_eq!(journal_events(root), events_before);

    let opened =
        run_berth_with_session(root, &["check", "file:b.rs", "--json"], "dir-sid.phase-16");
    assert_success(&opened);
    let opened = json(&opened);
    let acquisition = &opened["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "appended");
    assert_ne!(&acquisition["reservation_id"], reservation);
    assert_eq!(
        acquisition["session_mapping_publication"]["status"],
        "published"
    );
}

#[test]
fn an_explicit_check_widens_and_maps_an_unmapped_phase_session() {
    let repository = initialized_repository();
    let root = repository.path();
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let r16 = open_phase(root, "file:b.rs", "dir-sid.phase-16");
    let events_before = journal_events(root);

    let selected = run_berth_with_session(
        root,
        &["check", "--reservation", &r15, "file:explicit.rs", "--json"],
        "dir-sid.phase-17",
    );
    assert_success(&selected);
    let selected = json(&selected);
    let acquisition = &selected["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "widened");
    assert_eq!(acquisition["reservation_id"], r15);
    assert_eq!(
        acquisition["session_mapping_publication"]["status"],
        "published"
    );
    assert_eq!(
        session_mappings(root)["identities"]["dir-sid.phase-17"]["reservation_id"],
        r15
    );
    let events = journal_events(root);
    assert_eq!(events.len(), events_before.len() + 1);
    assert_eq!(events.last().expect("explicit widen")["op"], "widen");
    assert_eq!(
        events.last().expect("explicit widen")["reservation_id"],
        r15
    );
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &r16)["scopes"],
        serde_json::json!([{"kind":"file", "path":"b.rs"}])
    );
}

#[test]
fn a_second_check_in_a_phase_widens_its_own_reservation_without_changing_its_sibling() {
    let repository = initialized_repository();
    let root = repository.path();
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let r16 = open_phase(root, "file:b.rs", "dir-sid.phase-16");
    let events_before = journal_events(root);

    let checked = run_berth_with_session(
        root,
        &["check", "file:next.rs", "--json"],
        "dir-sid.phase-16",
    );
    assert_success(&checked);
    let checked = json(&checked);
    assert_eq!(checked["payload"]["data"]["acquisition"]["kind"], "widened");
    assert_eq!(
        checked["payload"]["data"]["acquisition"]["reservation_id"],
        r16
    );
    let events = journal_events(root);
    assert_eq!(events.len(), events_before.len() + 1);
    assert_eq!(events.last().expect("phase widen")["reservation_id"], r16);
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &r15)["scopes"],
        serde_json::json!([{"kind":"file", "path":"a.rs"}])
    );
    assert_eq!(
        reservation_row(&board, &r16)["scopes"],
        serde_json::json!([
            {"kind":"file", "path":"b.rs"}, {"kind":"file", "path":"next.rs"}
        ])
    );
}

#[test]
fn a_new_phase_reserves_only_free_paths_and_reports_the_sibling_holding_the_rest() {
    let repository = initialized_repository();
    let root = repository.path();
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");

    let mixed = run_berth_with_session(
        root,
        &["check", "file:a.rs", "file:b.rs", "--json"],
        "dir-sid.phase-16",
    );
    assert_success(&mixed);
    let mixed = json(&mixed);
    let acquisition = &mixed["payload"]["data"]["acquisition"];
    assert_eq!(acquisition["kind"], "appended");
    let r16 = acquisition["reservation_id"]
        .as_str()
        .expect("new phase reservation");
    assert_ne!(r15, r16);
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &r15)["scopes"],
        serde_json::json!([{"kind":"file", "path":"a.rs"}])
    );
    assert_eq!(
        reservation_row(&board, r16)["scopes"],
        serde_json::json!([{"kind":"file", "path":"b.rs"}])
    );
    assert_eq!(
        acquisition["already_held"],
        serde_json::json!([{
            "reservation_id": r15,
            "scopes": [{"kind": "file", "path": "a.rs"}]
        }]),
        "the mixed check must associate its held path with the sibling that protects it"
    );
}

#[test]
fn parallel_phase_merge_extents_contain_only_each_phases_own_paths() {
    let repository = initialized_repository();
    let root = repository.path();
    GIT.run(root, ["switch", "--quiet", "-c", "unit"]);
    let first = open_phase(root, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(root, "file:b.rs", "dir-sid.phase-2");
    fs::write(root.join("a.rs"), "// first phase work\n").expect("first phase file");
    fs::write(root.join("b.rs"), "// second phase work\n").expect("second phase file");
    GIT.run(root, ["add", "a.rs"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "first phase"]);

    let board = phase_board(root);
    for (id, path) in [(&first, "a.rs"), (&second, "b.rs")] {
        let extent = &reservation_row(&board, id)["merge_extent"];
        assert_eq!(extent["status"], "protected", "{board}");
        assert_eq!(
            extent["scopes"],
            serde_json::json!([{"kind": "file", "path": path}]),
            "a phase must protect its own work without adopting its sibling's paths: {board}"
        );
    }
}

#[test]
fn checkpointing_each_phase_names_its_dirty_or_unmerged_own_paths() {
    let repository = initialized_repository();
    let root = repository.path();
    GIT.run(root, ["switch", "--quiet", "-c", "unit"]);
    let first = open_phase(root, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(root, "file:b.rs", "dir-sid.phase-2");
    fs::write(root.join("a.rs"), "// committed phase work\n").expect("first phase file");
    fs::write(root.join("b.rs"), "// dirty phase work\n").expect("second phase file");
    GIT.run(root, ["add", "a.rs"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "first phase"]);

    for (id, session, path) in [
        (&first, "dir-sid.phase-1", "a.rs"),
        (&second, "dir-sid.phase-2", "b.rs"),
    ] {
        let checkpoint = run_berth_with_session(root, &["release", id, "--json"], session);
        assert_success(&checkpoint);
        let checkpoint = json(&checkpoint);
        assert_eq!(checkpoint["status"], "outstanding", "{checkpoint}");
        assert_eq!(checkpoint["payload"]["data"]["status"], "checkpointed");
        assert_eq!(
            checkpoint["payload"]["data"]["pending_paths"],
            serde_json::json!([path])
        );
        assert!(
            checkpoint["message"]
                .as_str()
                .expect("checkpoint message")
                .ends_with(&format!("; pending paths: {path}.")),
            "{checkpoint}"
        );
    }
}

#[test]
fn unclaimed_dirty_paths_belong_to_both_live_phase_extents() {
    let repository = initialized_repository();
    let root = repository.path();
    let first = open_phase(root, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(root, "file:b.rs", "dir-sid.phase-2");
    for path in ["a.rs", "b.rs", "c.rs"] {
        fs::write(root.join(path), "// pending work\n").expect("phase work");
    }

    let board = phase_board(root);
    for (id, own) in [(&first, "a.rs"), (&second, "b.rs")] {
        assert_eq!(
            reservation_row(&board, id)["merge_extent"]["scopes"],
            serde_json::json!([
                {"kind": "file", "path": own}, {"kind": "file", "path": "c.rs"}
            ]),
            "the unclaimed dirty path must remain protected by each phase: {board}"
        );
    }
}

#[test]
fn a_lone_phase_extent_contains_the_whole_worktree_surface() {
    let repository = initialized_repository();
    let root = repository.path();
    GIT.run(root, ["switch", "--quiet", "-c", "unit"]);
    let phase = open_phase(root, "file:a.rs", "dir-sid.phase-1");
    fs::write(root.join("a.rs"), "// committed own work\n").expect("own work");
    GIT.run(root, ["add", "a.rs"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "own work"]);
    fs::write(root.join("b.rs"), "// staged unclaimed work\n").expect("staged work");
    GIT.run(root, ["add", "b.rs"]);
    fs::write(root.join("c.rs"), "// untracked work\n").expect("untracked work");

    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &phase)["merge_extent"]["scopes"],
        serde_json::json!([
            {"kind": "file", "path": "a.rs"}, {"kind": "file", "path": "b.rs"},
            {"kind": "file", "path": "c.rs"}
        ]),
        "a lone phase must keep both its own and every unclaimed path: {board}"
    );
}

#[test]
fn reconciliation_keeps_a_checkpoint_outstanding_while_its_own_path_is_dirty() {
    assert_reconciliation_retains_pending_own_path(false);
}

#[test]
fn reconciliation_keeps_a_checkpoint_outstanding_while_its_own_path_is_unmerged() {
    assert_reconciliation_retains_pending_own_path(true);
}

fn assert_reconciliation_retains_pending_own_path(commit_pending: bool) {
    let repository = initialized_repository();
    let trunk = repository.path();
    let unit_directory = tempfile::tempdir().expect("unit directory");
    let unit = unit_directory.path();
    GIT.add_worktree_without_hooks(trunk, unit, "unit", "main");
    let first = open_phase(unit, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(unit, "file:b.rs", "dir-sid.phase-2");
    fs::write(unit.join("a.rs"), "// checkpoint work\n").expect("checkpoint work");
    GIT.run(unit, ["add", "a.rs"]);
    GIT.run_without_hooks(unit, ["commit", "--quiet", "-m", "checkpoint work"]);
    let tip = GIT.stdout(unit, ["rev-parse", "HEAD"]);
    let checkpoint =
        run_berth_with_session(unit, &["release", &first, "--json"], "dir-sid.phase-1");
    assert_success(&checkpoint);
    fs::write(unit.join("a.rs"), "// later own work\n").expect("pending own work");
    fs::write(unit.join("b.rs"), "// sibling work\n").expect("sibling work");
    if commit_pending {
        GIT.run(unit, ["add", "a.rs"]);
        GIT.run_without_hooks(unit, ["commit", "--quiet", "-m", "later own work"]);
    }
    GIT.run_without_hooks(trunk, ["merge", "--quiet", "--ff-only", tip.as_str()]);

    let board = phase_board(unit);
    let pending = reservation_row(&board, &first);
    assert_eq!(pending["lifecycle"]["stage"], "outstanding", "{board}");
    assert_eq!(
        pending["integration_evidence"]["status"]["status"],
        "not_integrated"
    );
    assert_eq!(
        pending["merge_extent"]["scopes"],
        serde_json::json!([{"kind": "file", "path": "a.rs"}])
    );
    assert_eq!(
        reservation_row(&board, &second)["lifecycle"]["stage"],
        "active"
    );
    assert!(
        journal_events(trunk)
            .iter()
            .all(|event| event["op"] != "release" || event["reservation_id"] != first)
    );
}

#[test]
fn patch_equivalent_own_paths_settle_without_adopting_unmerged_sibling_work() {
    let repository = initialized_repository();
    let trunk = repository.path();
    let unit_directory = tempfile::tempdir().expect("unit directory");
    let unit = unit_directory.path();
    GIT.add_worktree_without_hooks(trunk, unit, "unit", "main");
    let first = open_phase(unit, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(unit, "file:b.rs", "dir-sid.phase-2");
    fs::write(unit.join("a.rs"), "// integrated own work\n").expect("first phase work");
    GIT.run(unit, ["add", "a.rs"]);
    GIT.run_without_hooks(unit, ["commit", "--quiet", "-m", "first phase work"]);
    let protected_tip = GIT.stdout(unit, ["rev-parse", "HEAD"]);
    assert_success(&run_berth_with_session(
        unit,
        &["release", &first, "--json"],
        "dir-sid.phase-1",
    ));
    fs::write(unit.join("b.rs"), "// unmerged sibling work\n").expect("sibling work");
    GIT.run(unit, ["add", "b.rs"]);
    GIT.run_without_hooks(unit, ["commit", "--quiet", "-m", "sibling work"]);
    fs::write(trunk.join("trunk.rs"), "// independent trunk work\n").expect("trunk work");
    GIT.run(trunk, ["add", "trunk.rs"]);
    GIT.run_without_hooks(trunk, ["commit", "--quiet", "-m", "independent trunk work"]);
    GIT.run_without_hooks(trunk, ["cherry-pick", protected_tip.as_str()]);
    assert_ne!(GIT.stdout(trunk, ["rev-parse", "HEAD"]), protected_tip);

    let board = phase_board(unit);
    let integrated = reservation_row(&board, &first);
    assert_eq!(integrated["lifecycle"]["stage"], "released", "{board}");
    assert_eq!(
        integrated["lifecycle"]["disposition"]["kind"],
        "rewritten_integration"
    );
    assert_eq!(integrated["merge_extent"]["at_release"]["status"], "empty");
    assert_eq!(
        integrated["integration_evidence"]["status"]["proof"],
        "scoped_patch_equivalent"
    );
    assert_eq!(
        reservation_row(&board, &second)["lifecycle"]["stage"],
        "active"
    );
    assert_eq!(
        reservation_row(&board, &second)["merge_extent"]["scopes"],
        serde_json::json!([{"kind": "file", "path": "b.rs"}])
    );
}

#[test]
fn abandoning_a_sibling_restores_its_unclaimed_paths_to_the_remaining_extent() {
    assert_departed_sibling_paths_remain_protected(false);
}

#[test]
fn retargeting_a_sibling_restores_its_unclaimed_paths_to_the_remaining_extent() {
    assert_departed_sibling_paths_remain_protected(true);
}

fn assert_departed_sibling_paths_remain_protected(retarget: bool) {
    let repository = initialized_repository();
    let root = repository.path();
    GIT.run(root, ["switch", "--quiet", "-c", "unit"]);
    GIT.run(root, ["branch", "other-target", "main"]);
    let first = open_phase(root, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(root, "file:b.rs", "dir-sid.phase-2");
    fs::write(root.join("b.rs"), "// committed sibling work\n").expect("sibling work");
    GIT.run(root, ["add", "b.rs"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "sibling work"]);
    fs::write(root.join("a.rs"), "// remaining work\n").expect("remaining work");
    let before = phase_board(root);
    assert_eq!(
        reservation_row(&before, &first)["merge_extent"]["scopes"],
        serde_json::json!([{"kind":"file", "path":"a.rs"}])
    );
    assert_eq!(
        reservation_row(&before, &second)["merge_extent"]["scopes"],
        serde_json::json!([{"kind":"file", "path":"b.rs"}])
    );
    let output = if retarget {
        run_berth_with_session(
            root,
            &["retarget", &second, "--target", "other-target", "--json"],
            "dir-sid.phase-2",
        )
    } else {
        run_berth_with_session(
            root,
            &[
                "resolve",
                &second,
                "--abandon",
                "--why",
                "sibling stopped",
                "--json",
            ],
            "dir-sid.phase-2",
        )
    };
    assert_success(&output);
    let after = phase_board(root);
    assert_eq!(
        reservation_row(&after, &first)["merge_extent"]["scopes"],
        serde_json::json!([{"kind":"file", "path":"a.rs"}, {"kind":"file", "path":"b.rs"}]),
        "unchanged Git inputs cannot hide the departed sibling's now-unclaimed work: {after}"
    );
}

#[test]
fn parallel_phases_release_in_one_call_after_their_own_work_is_clean_and_merged() {
    let repository = initialized_repository();
    let trunk = repository.path();
    let unit_directory = tempfile::tempdir().expect("unit worktree directory");
    let unit = unit_directory.path();
    GIT.add_worktree_without_hooks(trunk, unit, "unit", "main");
    let first = open_phase(unit, "file:a.rs", "dir-sid.phase-1");
    let second = open_phase(unit, "file:b.rs", "dir-sid.phase-2");
    fs::write(unit.join("a.rs"), "// first phase work\n").expect("first phase file");
    fs::write(unit.join("b.rs"), "// second phase work\n").expect("second phase file");
    GIT.run(unit, ["add", "a.rs"]);
    GIT.run_without_hooks(unit, ["commit", "--quiet", "-m", "first phase"]);

    let first_checkpoint =
        run_berth_with_session(unit, &["release", &first, "--json"], "dir-sid.phase-1");
    assert_success(&first_checkpoint);
    assert_eq!(
        json(&first_checkpoint)["payload"]["data"]["status"],
        "checkpointed"
    );
    let second_checkpoint =
        run_berth_with_session(unit, &["release", &second, "--json"], "dir-sid.phase-2");
    assert_success(&second_checkpoint);
    let second_checkpoint = json(&second_checkpoint);
    assert_eq!(
        second_checkpoint["status"], "outstanding",
        "{second_checkpoint}"
    );
    assert_eq!(
        second_checkpoint["payload"]["data"]["status"], "checkpointed",
        "dirty phase 2 work must remain outstanding: {second_checkpoint}"
    );

    GIT.run_without_hooks(trunk, ["merge", "--quiet", "--ff-only", "unit"]);
    let first_release =
        run_berth_with_session(unit, &["release", &first, "--json"], "dir-sid.phase-1");
    assert_success(&first_release);
    let first_release = json(&first_release);
    assert_eq!(
        first_release["payload"]["data"]["status"], "released",
        "{first_release}"
    );
    assert_eq!(
        first_release["payload"]["data"]["disposition"]["kind"], "integrated",
        "dirty sibling work must not prevent phase 1's release: {first_release}"
    );

    let second_pending =
        run_berth_with_session(unit, &["release", &second, "--json"], "dir-sid.phase-2");
    assert_success(&second_pending);
    let second_pending = json(&second_pending);
    assert_eq!(second_pending["status"], "outstanding", "{second_pending}");
    assert_eq!(
        second_pending["payload"]["data"]["status"], "outstanding",
        "phase 2 must not settle while its own path is dirty: {second_pending}"
    );
    assert_eq!(
        second_pending["payload"]["data"]["pending_paths"],
        serde_json::json!(["b.rs"]),
        "the structured answer must identify only phase 2's pending work: {second_pending}"
    );
    assert!(
        second_pending["message"]
            .as_str()
            .expect("outstanding release explanation")
            .contains("b.rs"),
        "the release answer must name the phase's pending path: {second_pending}"
    );
    let board = phase_board(unit);
    let pending = reservation_row(&board, &second);
    assert_eq!(pending["lifecycle"]["stage"], "outstanding", "{board}");
    assert_eq!(
        pending["merge_extent"]["scopes"],
        serde_json::json!([{"kind": "file", "path": "b.rs"}]),
        "phase 2 must keep its dirty path protected after phase 1 settles: {board}"
    );

    GIT.run(unit, ["add", "b.rs"]);
    GIT.run_without_hooks(unit, ["commit", "--quiet", "-m", "second phase"]);
    GIT.run_without_hooks(trunk, ["merge", "--quiet", "--ff-only", "unit"]);
    let second_release =
        run_berth_with_session(unit, &["release", &second, "--json"], "dir-sid.phase-2");
    assert_success(&second_release);
    let second_release = json(&second_release);
    assert_eq!(
        second_release["payload"]["data"]["status"], "released",
        "{second_release}"
    );
    assert_eq!(
        second_release["payload"]["data"]["disposition"]["kind"], "integrated",
        "merging phase 2's own work must allow release in this call: {second_release}"
    );
}

#[test]
fn a_phase_checkpoint_does_not_adopt_a_released_siblings_committed_paths() {
    let repository = initialized_repository();
    let root = repository.path();
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let r16 = open_phase(root, "file:b.rs", "dir-sid.phase-16");
    fs::write(root.join("a.rs"), "// phase 15 checkpoint\n").expect("phase 15 file");
    // Keep the concurrent phase live while reconciliation settles the first checkpoint.
    fs::write(root.join("b.rs"), "// phase 16 declared work\n").expect("phase 16 file");
    GIT.run(root, ["add", "a.rs"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "phase 15 checkpoint"]);
    assert_success(&run_berth_with_session(
        root,
        &["release", &r15, "--json"],
        "dir-sid.phase-15",
    ));
    fs::write(
        root.join("phase16-extra.rs"),
        "// phase 16 unclaimed work\n",
    )
    .expect("phase 16 extra file");
    let before = journal_events(root);

    let checkpoint = run_berth_with_session(
        root,
        &["drift", "--full", "--reservation", &r16, "--json"],
        "dir-sid.phase-16",
    );
    assert_success(&checkpoint);
    let events = journal_events(root);
    let widens = events[before.len()..]
        .iter()
        .filter(|event| event["op"] == "widen")
        .collect::<Vec<_>>();
    assert_eq!(
        widens.len(),
        1,
        "checkpoint must acquire the free phase 16 file: {}",
        json(&checkpoint)
    );
    assert_eq!(widens[0]["reservation_id"], r16);
    assert_eq!(
        widens[0]["added_scopes"],
        serde_json::json!([{"kind":"file", "path":"phase16-extra.rs"}])
    );
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &r16)["scopes"],
        serde_json::json!([
            {"kind":"file", "path":"b.rs"}, {"kind":"file", "path":"phase16-extra.rs"}
        ])
    );
    assert_eq!(
        reservation_row(&board, &r15)["lifecycle"]["stage"],
        "released"
    );
}

#[test]
fn releasing_one_phase_retires_only_its_session_mapping() {
    let repository = initialized_repository();
    let root = repository.path();
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let r16 = open_phase(root, "file:b.rs", "dir-sid.phase-16");
    let before = session_mappings(root);
    assert_eq!(
        before["identities"]["dir-sid.phase-15"]["reservation_id"],
        r15
    );
    assert_eq!(
        before["identities"]["dir-sid.phase-16"]["reservation_id"],
        r16
    );

    let released = run_berth_with_session(root, &["release", &r15, "--json"], "dir-sid.phase-15");
    assert_success(&released);
    let after = session_mappings(root);
    assert!(
        after["identities"].get("dir-sid.phase-15").is_none(),
        "released phase mapping must retire: {after}"
    );
    assert_eq!(
        after["identities"]["dir-sid.phase-16"],
        before["identities"]["dir-sid.phase-16"]
    );
    let continued = run_berth_with_session(
        root,
        &["check", "file:continued.rs", "--json"],
        "dir-sid.phase-16",
    );
    assert_success(&continued);
    let continued = json(&continued);
    assert_eq!(
        continued["payload"]["data"]["acquisition"]["kind"],
        "widened"
    );
    assert_eq!(
        continued["payload"]["data"]["acquisition"]["reservation_id"],
        r16
    );
}

#[test]
fn the_reservation_limit_counts_each_parallel_phase() {
    let repository = initialized_repository();
    let root = repository.path();
    fs::write(
        root.join(CONFIGURATION_PATH),
        "trunk = \"main\"\nmaximum_reservations = 2\n",
    )
    .expect("reservation limit configuration");
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let r16 = open_phase(root, "file:b.rs", "dir-sid.phase-16");
    assert_ne!(r15, r16);
    let before = journal_events(root);
    let mappings_before = session_mappings(root);

    let refused =
        run_berth_with_session(root, &["check", "file:c.rs", "--json"], "dir-sid.phase-17");
    let exit_code = refused.status.code();
    let refused = json(&refused);
    assert_eq!(refused["verb"], "check");
    assert_eq!(refused["status"], "reservation_limit_reached");
    assert_eq!(exit_code, Some(1), "{refused}");
    assert_eq!(refused["payload"]["kind"], "check");
    assert_eq!(
        refused["payload"]["data"]["status"],
        "reservation_limit_reached"
    );
    assert_eq!(refused["payload"]["data"]["maximum"], 2);
    assert_eq!(journal_events(root), before);
    assert_eq!(session_mappings(root), mappings_before);
}

#[test]
fn named_claim_before_attaches_only_to_the_selected_phase() {
    assert_named_answer("--before", "sequence", "requester_before_holder");
}

#[test]
fn named_claim_after_attaches_only_to_the_selected_phase() {
    assert_named_answer("--after", "sequence", "holder_before_requester");
}

#[test]
fn named_claim_repeated_before_refuses_duplicate_ordering_without_mutation() {
    assert_repeated_named_ordering_is_refused("--before");
}

#[test]
fn named_claim_repeated_after_refuses_duplicate_ordering_without_mutation() {
    assert_repeated_named_ordering_is_refused("--after");
}

#[test]
fn ordinary_claim_repeated_ordering_uses_distinct_reservations() {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    let mut ids = Vec::new();
    for _ in 0..2 {
        let claimed = run_berth_with_session(
            root,
            &[
                "claim",
                "file:shared.rs",
                "--after",
                &fixture.holder,
                "--overlap-why",
                "ordinary repeated answer",
                "--json",
            ],
            DIRECTOR_SESSION,
        );
        assert_success(&claimed);
        ids.push(json(&claimed)["payload"]["data"]["reservation_id"].clone());
    }
    assert_ne!(ids[0], ids[1]);
    fs::remove_file(root.join(".git/cargo-berth/reservations.json")).expect("force replay");
    let board = phase_board(root);
    let waiting = board["payload"]["data"]["waiting"]["entries"]
        .as_array()
        .expect("live ordering holds");
    for id in ids {
        assert!(
            waiting
                .iter()
                .any(|entry| entry["predecessor"] == fixture.holder && entry["successor"] == id)
        );
    }
}

#[test]
fn named_claim_defer_records_only_the_selected_phase_without_acquiring_paths() {
    assert_named_answer("--defer", "defer", "");
}

#[test]
fn named_claim_override_attaches_only_to_the_selected_phase() {
    assert_named_answer("--override", "override", "");
}

#[test]
fn named_claim_unknown_selection_is_mutation_free() {
    let fixture = NamedPhaseOverlap::new();
    fixture.assert_refused_selection("01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0aff");
}

#[test]
fn named_claim_inactive_selection_is_mutation_free() {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    let ended = open_phase(root, "file:ended.rs", "ended-phase");
    assert_success(&run_berth_with_session(
        root,
        &["release", &ended, "--json"],
        "ended-phase",
    ));
    // Settle the ended phase first. The assertion helper's board read then refreshes
    // the remaining sibling group before measuring the refused claim.
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &ended)["lifecycle"]["stage"],
        "released"
    );
    fixture.assert_refused_selection(&ended);
}

#[test]
fn named_claim_foreign_run_selection_is_mutation_free() {
    let fixture = NamedPhaseOverlap::new();
    let mappings = session_mappings(fixture.root());
    assert_ne!(
        mappings["identities"]["holder-session"]["coordination_run_id"],
        mappings["identities"][DIRECTOR_SESSION]["coordination_run_id"]
    );
    fixture.assert_refused_selection(&fixture.holder);
}

#[test]
fn named_claim_foreign_worktree_selection_is_mutation_free() {
    let fixture = NamedPhaseOverlap::new();
    // The run matches, so only the worktree eligibility rule can reject this id.
    let run =
        session_mappings(fixture.root())["identities"][DIRECTOR_SESSION]["coordination_run_id"]
            .as_str()
            .expect("director run")
            .to_owned();
    let foreign_root = fixture.worktrees.path().join("same-run");
    GIT.run_without_hooks(
        fixture.root(),
        [
            "worktree",
            "add",
            "--quiet",
            "-b",
            "same-run",
            foreign_root.to_str().expect("foreign worktree path"),
        ],
    );
    let foreign = run_berth_with_session(
        &foreign_root,
        &["claim", "file:foreign-tree.rs", "--run", &run, "--json"],
        "same-run-other-worktree",
    );
    assert_success(&foreign);
    let foreign = json(&foreign);
    assert_eq!(
        foreign["payload"]["data"]["coordination_run_id"], run,
        "only the worktree should differ"
    );
    fs::write(
        foreign_root.join("foreign-tree.rs"),
        "// foreign worktree work\n",
    )
    .expect("foreign worktree work");
    fixture.assert_refused_selection(
        foreign["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("foreign worktree id"),
    );
}

#[test]
fn named_claim_all_held_answer_survives_replay_and_moves_the_director_mapping() {
    let fixture = NamedPhaseOverlap::with_phase16_holding_overlap();
    let root = fixture.root();
    let before = journal_events(root);
    let answered = fixture.answer(&fixture.r16, "--after", &[]);
    assert_success(&answered);
    let events = journal_events(root);
    let appended = &events[before.len()..];
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0]["op"], "answer");
    assert_eq!(appended[0]["reservation_id"], fixture.r16);
    assert_eq!(appended[0]["authorizations"][0]["kind"], "sequence");
    assert_eq!(
        appended[0]["authorizations"][0]["direction"],
        "holder_before_requester"
    );
    assert_director_mapping(root, &fixture.r16);
    fs::remove_file(root.join(".git/cargo-berth/reservations.json"))
        .expect("discard projection to require replay");
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &fixture.r16)["scopes"],
        serde_json::json!([
            {"kind":"file", "path":"b.rs"}, {"kind":"file", "path":"shared.rs"}
        ])
    );
    let waiting = board["payload"]["data"]["waiting"]["entries"]
        .as_array()
        .expect("live ordering holds");
    assert!(
        waiting.iter().any(
            |entry| entry["edge_id"] == appended[0]["authorizations"][0]["edge_id"]
                && entry["predecessor"] == fixture.holder
                && entry["successor"] == fixture.r16
        ),
        "the answer-only ordering must survive replay: {board}"
    );
    assert_eq!(
        journal_events(root)
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        3
    );
}

#[test]
fn named_claim_replace_keeps_the_siblings_ordering_edge() {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    assert_success(&fixture.answer(&fixture.r15, "--before", &[]));
    let before_edge =
        journal_events(root).last().expect("R15 ordering")["authorization"]["edge_id"].clone();
    assert_success(&fixture.answer(&fixture.r16, "--before", &[]));
    let own_edge =
        answer_authorization(journal_events(root).last().expect("R16 prior ordering"))["edge_id"]
            .clone();
    fixture.map_director_to_r15();
    assert_success(&fixture.answer(&fixture.r16, "--after", &["--replace"]));
    let events = journal_events(root);
    let event = events.last().expect("R16 answer");
    let authorization = answer_authorization(event);
    let replaced = authorization["replaced"]
        .as_array()
        .expect("R16 replacement");
    assert_eq!(
        replaced.len(),
        1,
        "only R16's prior ordering may be retired: {event}"
    );
    assert_eq!(replaced[0]["reservation_id"], fixture.r16);
    assert_eq!(replaced[0]["edge_id"], own_edge);
    fs::remove_file(root.join(".git/cargo-berth/reservations.json")).expect("force replay");
    let board = phase_board(root);
    let waiting = board["payload"]["data"]["waiting"]["entries"]
        .as_array()
        .expect("live ordering holds");
    assert!(
        waiting.iter().any(|entry| entry["edge_id"] == before_edge
            && entry["predecessor"] == fixture.r15
            && entry["successor"] == fixture.holder),
        "R15's edge must still hold its successor after replay: {board}"
    );
    assert!(
        !waiting.iter().any(|entry| entry["edge_id"] == own_edge),
        "R16's replaced edge must no longer hold integration"
    );
    assert_director_mapping(root, &fixture.r16);
}

#[test]
fn named_claim_preserves_the_selected_target_baseline_purpose_and_plan() {
    let repository = initialized_repository();
    let root = repository.path();
    let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
    let baseline = GIT.stdout(root, ["rev-parse", "HEAD"]);
    GIT.run(root, ["branch", "phase-target"]);
    let claimed = run_berth_with_session(
        root,
        &[
            "claim",
            "file:b.rs",
            "--target",
            "phase-target",
            "--head",
            &baseline,
            "--why",
            "phase 16 purpose",
            "--plan",
            "docs/plan.md",
            "--phase",
            "16",
            "--json",
        ],
        "dir-sid.phase-16",
    );
    assert_success(&claimed);
    let claimed = json(&claimed);
    let r16 = claimed["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("phase 16");
    let original = reservation_row(&phase_board(root), r16).clone();
    fs::write(root.join("a.rs"), "// keep phase 15 active\n").expect("phase 15 work");
    fs::write(root.join("b.rs"), "// keep phase 16 active\n").expect("phase 16 work");
    fs::write(root.join("later.txt"), "later head\n").expect("new commit");
    GIT.run(root, ["add", "later.txt"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "advance caller head"]);
    assert_ne!(GIT.stdout(root, ["rev-parse", "HEAD"]), baseline);
    assert_success(&run_berth_with_session(
        root,
        &["check", "--reservation", &r15, "file:a.rs", "--json"],
        DIRECTOR_SESSION,
    ));
    let selected = run_berth_with_session(
        root,
        &["claim", "file:extra.rs", "--reservation", r16, "--json"],
        DIRECTOR_SESSION,
    );
    assert_success(&selected);
    let after = reservation_row(&phase_board(root), r16).clone();
    for field in ["target", "purpose", "source"] {
        assert!(
            !original[field].is_null(),
            "fixture must expose {field}: {original}"
        );
        assert_eq!(
            after[field], original[field],
            "named claim must preserve {field}"
        );
    }
    let checked = run_berth_with_session(
        root,
        &["check", "--reservation", r16, "file:extra.rs", "--json"],
        DIRECTOR_SESSION,
    );
    assert_success(&checked);
    assert_eq!(
        json(&checked)["payload"]["data"]["acquisition"]["phase_start_head"],
        baseline
    );
    assert_director_mapping(root, r16);
}

#[test]
fn named_claim_refuses_creation_only_arguments_without_mutation() {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    let head = GIT.stdout(root, ["rev-parse", "HEAD"]);
    for creation in [
        vec!["--target", "main"],
        vec!["--head", head.as_str()],
        vec!["--why", "new purpose"],
        vec!["--plan", "docs/plan.md", "--phase", "16"],
    ] {
        let before = journal_events(root);
        let mappings = session_mappings(root);
        let mut arguments = vec![
            "claim",
            "file:new.rs",
            "--reservation",
            &fixture.r16,
            "--json",
        ];
        arguments.extend(creation);
        let refused = run_berth_with_session(root, &arguments, DIRECTOR_SESSION);
        assert!(
            !refused.status.success(),
            "creation-only arguments must be refused"
        );
        assert_eq!(journal_events(root), before);
        assert_eq!(session_mappings(root), mappings);
    }
}

#[test]
fn named_claim_reports_sibling_coverage_and_widens_at_the_reservation_limit() {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    fs::write(
        root.join(CONFIGURATION_PATH),
        "trunk = \"main\"\nmaximum_reservations = 3\n",
    )
    .expect("limit configuration");
    let refused = run_berth_with_session(
        root,
        &["check", "file:over-limit.rs", "--json"],
        "dir-sid.phase-17",
    );
    assert_eq!(refused.status.code(), Some(1));
    assert_eq!(
        json(&refused)["status"],
        "reservation_limit_reached",
        "fixture must reach the actual reservation cap"
    );
    let before = journal_events(root);
    let selected = run_berth_with_session(
        root,
        &[
            "claim",
            "file:a.rs",
            "file:shared.rs",
            "--reservation",
            &fixture.r16,
            "--after",
            &fixture.holder,
            "--overlap-why",
            "phase 16 follows the holder",
            "--json",
        ],
        DIRECTOR_SESSION,
    );
    assert_success(&selected);
    let selected = json(&selected);
    assert_eq!(
        selected["payload"]["data"]["acquisition"]["already_held"],
        serde_json::json!([
            {"reservation_id":fixture.r15, "scopes":[{"kind":"file", "path":"a.rs"}]}
        ])
    );
    let events = journal_events(root);
    assert!(
        !events[before.len()..]
            .iter()
            .any(|event| event["op"] == "claim")
    );
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &fixture.r16)["scopes"],
        serde_json::json!([
            {"kind":"file", "path":"b.rs"}, {"kind":"file", "path":"shared.rs"}
        ])
    );
    assert_eq!(
        reservation_row(&board, &fixture.r15)["scopes"],
        serde_json::json!([{"kind":"file", "path":"a.rs"}])
    );
    assert_director_mapping(root, &fixture.r16);
}

#[test]
fn ordinary_claim_still_appends_and_legacy_defer_uses_the_oldest_phase() {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    let before = journal_events(root);
    let claimed = run_berth_with_session(
        root,
        &[
            "claim",
            "file:shared.rs",
            "--after",
            &fixture.holder,
            "--overlap-why",
            "legacy acquisition",
            "--json",
        ],
        DIRECTOR_SESSION,
    );
    assert_success(&claimed);
    let claimed = json(&claimed);
    let id = claimed["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("new ordinary claim");
    assert_ne!(id, fixture.r15);
    assert_ne!(id, fixture.r16);
    let events = journal_events(root);
    assert_eq!(events[before.len()]["op"], "claim");
    assert_eq!(events[before.len()]["reservation_id"], id);
    fixture.map_director_to_r15();
    let deferred = run_berth_with_session(
        root,
        &[
            "claim",
            "file:shared.rs",
            "--defer",
            &fixture.holder,
            "--overlap-why",
            "legacy deferral",
            "--json",
        ],
        DIRECTOR_SESSION,
    );
    assert_success(&deferred);
    let events = journal_events(root);
    assert_eq!(events.last().expect("legacy answer")["op"], "answer");
    assert_eq!(
        events.last().expect("legacy answer")["reservation_id"],
        fixture.r15
    );
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &fixture.r16)["scopes"],
        serde_json::json!([{"kind":"file", "path":"b.rs"}])
    );
}

/// Two active phases and a real foreign holder, with the director selecting phase 15.
struct NamedPhaseOverlap {
    repository: TempDir,
    worktrees:  TempDir,
    r15:        String,
    r16:        String,
    holder:     String,
}

/// Whether phase 16 already owns the requested overlap before the answer under test.
#[derive(Clone, Copy)]
enum Phase16InitialCoverage {
    SeedOnly,
    SeedAndOverlap,
}

impl NamedPhaseOverlap {
    fn new() -> Self { Self::with_coverage(Phase16InitialCoverage::SeedOnly) }

    fn with_phase16_holding_overlap() -> Self {
        Self::with_coverage(Phase16InitialCoverage::SeedAndOverlap)
    }

    fn with_coverage(coverage: Phase16InitialCoverage) -> Self {
        let repository = initialized_repository();
        let root = repository.path();
        let r15 = open_phase(root, "file:a.rs", "dir-sid.phase-15");
        let worktrees = tempfile::tempdir().expect("worktree parent");
        let holder_root = worktrees.path().join("holder");
        GIT.run_without_hooks(
            root,
            [
                "worktree",
                "add",
                "--quiet",
                "-b",
                "holder",
                holder_root.to_str().expect("worktree path"),
            ],
        );
        let claimed = run_berth_with_session(
            &holder_root,
            &["claim", "file:shared.rs", "--json"],
            "holder-session",
        );
        assert_success(&claimed);
        let holder = json(&claimed)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("holder id")
            .to_owned();
        fs::write(holder_root.join("shared.rs"), "// foreign holder work\n")
            .expect("holder's real overlap");
        let _ = phase_board(root);
        let r16 = match coverage {
            Phase16InitialCoverage::SeedOnly => open_phase(root, "file:b.rs", "dir-sid.phase-16"),
            Phase16InitialCoverage::SeedAndOverlap => {
                let claimed = run_berth_with_session(
                    root,
                    &[
                        "claim",
                        "file:b.rs",
                        "file:shared.rs",
                        "--override",
                        &holder,
                        "--overlap-why",
                        "seed phase 16 overlap",
                        "--json",
                    ],
                    "dir-sid.phase-16",
                );
                assert_success(&claimed);
                json(&claimed)["payload"]["data"]["reservation_id"]
                    .as_str()
                    .expect("phase 16 id")
                    .to_owned()
            },
        };
        let fixture = Self {
            repository,
            worktrees,
            r15,
            r16,
            holder,
        };
        fixture.map_director_to_r15();
        fixture
    }

    fn root(&self) -> &Path { self.repository.path() }

    fn map_director_to_r15(&self) {
        assert_success(&run_berth_with_session(
            self.root(),
            &["check", "--reservation", &self.r15, "file:a.rs", "--json"],
            DIRECTOR_SESSION,
        ));
        assert_director_mapping(self.root(), &self.r15);
    }

    fn answer(&self, selected: &str, flag: &str, extra: &[&str]) -> Output {
        let mut arguments = vec![
            "claim",
            "file:shared.rs",
            "--reservation",
            selected,
            flag,
            &self.holder,
            "--overlap-why",
            "answer for the named phase",
            "--json",
        ];
        arguments.extend_from_slice(extra);
        run_berth_with_session(self.root(), &arguments, DIRECTOR_SESSION)
    }

    fn assert_refused_selection(&self, selected: &str) {
        let _ = phase_board(self.root());
        let events = journal_events(self.root());
        let mappings = session_mappings(self.root());
        let checked = run_berth_with_session(
            self.root(),
            &[
                "check",
                "file:selection-probe.rs",
                "--reservation",
                selected,
                "--json",
            ],
            DIRECTOR_SESSION,
        );
        let claimed = self.answer(selected, "--after", &[]);
        assert!(
            !claimed.status.success(),
            "invalid named selection must be refused"
        );
        assert_eq!(claimed.status.code(), checked.status.code());
        let claimed = json(&claimed);
        assert_eq!(claimed["verb"], "claim");
        assert_eq!(
            json(&checked)["status"],
            "invalid_input",
            "the clear path must reach check's selection validator"
        );
        assert_eq!(claimed["status"], json(&checked)["status"]);
        assert_eq!(journal_events(self.root()), events);
        assert_eq!(session_mappings(self.root()), mappings);
    }
}

fn assert_director_mapping(root: &Path, selected: &str) {
    assert_eq!(
        session_mappings(root)["identities"][DIRECTOR_SESSION]["reservation_id"],
        selected
    );
}

fn assert_repeated_named_ordering_is_refused(flag: &str) {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    let first = run_berth_with_session(
        root,
        &[
            "claim",
            "file:shared.rs",
            "--reservation",
            &fixture.r16,
            flag,
            &fixture.holder,
            "--overlap-why",
            "initial phase ordering",
            "--json",
        ],
        "dir-sid.phase-16",
    );
    assert_success(&first);
    let edge =
        answer_authorization(journal_events(root).last().expect("first ordering"))["edge_id"]
            .clone();
    assert_director_mapping(root, &fixture.r15);
    let events = journal_events(root);
    let mappings = session_mappings(root);
    // Refuse both a zero-growth repeat and a repeat that would otherwise widen.
    for extra in [vec![], vec!["file:extra.rs"]] {
        let refused = fixture.answer(&fixture.r16, flag, &extra);
        assert_eq!(refused.status.code(), Some(2));
        let refused = json(&refused);
        assert_eq!(refused["verb"], "claim");
        assert_eq!(refused["status"], "duplicate_ordering_edge");
        let (before, after) = if flag == "--before" {
            (&fixture.r16, &fixture.holder)
        } else {
            (&fixture.holder, &fixture.r16)
        };
        assert_eq!(
            refused["message"],
            format!("Ordering edge {before} before {after} already exists.")
        );
        assert_eq!(journal_events(root), events);
        assert_eq!(session_mappings(root), mappings);
    }
    fs::remove_file(root.join(".git/cargo-berth/reservations.json")).expect("force replay");
    let board = phase_board(root);
    let waiting = board["payload"]["data"]["waiting"]["entries"]
        .as_array()
        .expect("live ordering holds");
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["edge_id"], edge);
    assert_eq!(
        reservation_row(&board, &fixture.r16)["scopes"],
        serde_json::json!([
            {"kind":"file", "path":"b.rs"}, {"kind":"file", "path":"shared.rs"}
        ])
    );
}

fn answer_authorization(event: &Value) -> &Value {
    if event["op"] == "answer" {
        &event["authorizations"][0]
    } else {
        &event["authorization"]
    }
}

fn assert_named_answer(flag: &str, kind: &str, direction: &str) {
    let fixture = NamedPhaseOverlap::new();
    let root = fixture.root();
    let before = journal_events(root);
    let applied = fixture.answer(&fixture.r16, flag, &[]);
    assert_success(&applied);
    let events = journal_events(root);
    let appended = &events[before.len()..];
    assert_eq!(
        appended.len(),
        1,
        "one atomic answer transaction: {appended:?}"
    );
    assert_eq!(appended[0]["reservation_id"], fixture.r16);
    let authorization = answer_authorization(&appended[0]);
    assert_eq!(authorization["kind"], kind);
    assert_eq!(authorization["blocker"], fixture.holder);
    if !direction.is_empty() {
        assert_eq!(authorization["direction"], direction);
    }
    assert_eq!(
        appended[0]["op"],
        if flag == "--defer" { "answer" } else { "widen" }
    );
    assert_director_mapping(root, &fixture.r16);
    let board = phase_board(root);
    assert_eq!(
        reservation_row(&board, &fixture.r15)["scopes"],
        serde_json::json!([{"kind":"file", "path":"a.rs"}])
    );
    let expected = if flag == "--defer" {
        serde_json::json!([{"kind":"file", "path":"b.rs"}])
    } else {
        serde_json::json!([{"kind":"file", "path":"b.rs"}, {"kind":"file", "path":"shared.rs"}])
    };
    assert_eq!(reservation_row(&board, &fixture.r16)["scopes"], expected);
    if flag == "--defer" {
        let overlaps = board["payload"]["data"]["unresolved_overlaps"]["entries"]
            .as_array()
            .expect("live deferrals");
        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0]["deferred"], fixture.r16);
        assert_eq!(overlaps[0]["blocker"], fixture.holder);
        assert_eq!(
            overlaps[0]["consequence"],
            "both_integrations_held_until_sequence"
        );
    }
    assert_eq!(
        events.iter().filter(|event| event["op"] == "claim").count(),
        3
    );
}

fn open_phase(root: &Path, path: &str, session_id: &str) -> String {
    let opened = run_berth_with_session(root, &["check", path, "--json"], session_id);
    assert_success(&opened);
    let opened = json(&opened);
    assert_eq!(opened["payload"]["data"]["acquisition"]["kind"], "appended");
    opened["payload"]["data"]["acquisition"]["reservation_id"]
        .as_str()
        .expect("phase reservation")
        .to_owned()
}

fn phase_board(root: &Path) -> Value {
    let board = run_berth_with_session(root, &["board", "--json"], DIRECTOR_SESSION);
    assert_success(&board);
    json(&board)
}

fn session_mappings(root: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(root.join(SESSION_MAPPING_PATH)).expect("session mappings should read"),
    )
    .expect("session mappings should be JSON")
}

fn initialized_repository() -> TempDir {
    let repository = tempfile::tempdir().expect("temporary repository should exist");
    let root = repository.path();
    GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
    GIT.run(root, ["config", "user.name", "Berth Test"]);
    GIT.run(root, ["config", "user.email", "berth@example.invalid"]);
    fs::write(root.join("README.md"), "parallel phase fixture\n")
        .expect("initial file should write");
    GIT.run(root, ["add", "README.md"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth_with_session(root, &["init", "--json"], DIRECTOR_SESSION);
    assert_success(&initialized);
    GIT.run(root, ["add", CONFIGURATION_PATH]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "configure berth"]);
    repository
}

fn run_berth_with_session(root: &Path, arguments: &[&str], session_id: &str) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session_id)
        .output()
        .expect("cargo-berth should run")
}

fn run_post_tool_use(root: &Path) -> Output {
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "cwd": root,
        "tool_input": {"command": "printf '// unheld Bash write\\n' > c.rs"},
        "session_id": DIRECTOR_SESSION,
    });
    let mut child = berth_command(BERTH_EXECUTABLE)
        .args(["hook", "post-tool-use"])
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("post-tool-use hook should start");
    let mut stdin = child.stdin.take().expect("hook stdin should be piped");
    stdin
        .write_all(&serde_json::to_vec(&payload).expect("Bash payload should serialize"))
        .expect("Bash payload should reach the hook");
    drop(stdin);
    child
        .wait_with_output()
        .expect("post-tool-use hook should finish")
}

fn journal_events(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should be JSON"))
        .collect()
}
