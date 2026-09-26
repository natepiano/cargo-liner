//! Real-hook fixtures for checkpoint interval rewrites and durable marker recovery.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use cargo_berth_test_support::berth_command;
use serde_json::Value;
use tempfile::TempDir;
use tempfile::tempdir;

use super::BERTH_EXECUTABLE;
use super::FIRST_RUN;
use super::GIT;
use super::JOURNAL_PATH;
use super::RETENTION_REF_PREFIX;
use super::claim;
use super::commit_file;
use super::git_command;
use super::initialized_repository;
use super::journal_events;
use super::json_output;
use super::reservation_id;
use super::run_berth;
use super::split_rebase;
use super::split_rebase::write_first_rebase_action;

/// The marker namespace is shared by legacy bypasses and branch rewrites.
const PENDING_PREFIX: &str = "cargo-berth-pending-bypass-";
/// Space between independently editable hunks makes the later conflict unambiguous.
const HUNK_SEPARATOR: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";

/// A linked holder whose managed hooks consult a different policy checkout.
struct RewriteFixture {
    repository: TempDir,
    worktrees:  TempDir,
    holder:     PathBuf,
}

impl RewriteFixture {
    fn new() -> Self {
        let repository = initialized_repository();
        for (key, value) in [("maintenance.auto", "false"), ("gc.auto", "0")] {
            GIT.run(repository.path(), ["config", key, value]);
        }
        commit_file(
            repository.path(),
            "phase.txt",
            &contents("base", "base"),
            "phase base",
        );
        GIT.run(repository.path(), ["add", ".claude/config/berth.toml"]);
        GIT.run(
            repository.path(),
            ["commit", "--quiet", "-m", "policy configuration"],
        );
        let worktrees = tempdir().expect("linked checkout parent should exist");
        let holder = worktrees.path().join("holder");
        GIT.run(
            repository.path(),
            [
                "worktree",
                "add",
                "--quiet",
                "-b",
                "holder",
                holder.to_str().expect("holder path should be UTF-8"),
            ],
        );
        assert!(
            repository
                .path()
                .join(".git/hooks/reference-transaction")
                .is_file()
        );
        let administrative_directory = GIT.stdout(&holder, ["rev-parse", "--absolute-git-dir"]);
        assert_ne!(
            Path::new(&administrative_directory),
            repository.path().join(".git")
        );
        Self {
            repository,
            worktrees,
            holder,
        }
    }

    fn root(&self) -> &Path { self.repository.path() }

    fn head(&self) -> String { GIT.stdout(&self.holder, ["rev-parse", "HEAD"]) }

    fn start_phase(&self) -> String {
        let claimed = claim(&self.holder, "file:phase.txt", FIRST_RUN);
        assert!(
            claimed.status.success(),
            "{}",
            String::from_utf8_lossy(&claimed.stdout)
        );
        reservation_id(&claimed)
    }

    fn commit(&self, first: &str, later: &str) {
        commit_file(
            &self.holder,
            "phase.txt",
            &contents(first, later),
            "holder phase",
        );
    }

    fn checkpoint(&self, id: &str) {
        let checkpoint = run_berth(&self.holder, &["release", id, "--json"]);
        assert!(
            checkpoint.status.success(),
            "{}",
            String::from_utf8_lossy(&checkpoint.stdout)
        );
        assert_eq!(json_output(&checkpoint)["status"], "outstanding");
    }

    fn upstream_conflict(&self) {
        commit_file(
            self.root(),
            "phase.txt",
            &contents("base", "upstream"),
            "upstream changes later hunk",
        );
    }

    fn stop_rebase(&self) {
        let rebased = GIT.output(&self.holder, ["rebase", "main"]);
        assert!(
            !rebased.status.success(),
            "fixture should conflict in its later hunk"
        );
        assert!(
            GIT.stdout(&self.holder, ["diff", "--name-only", "--diff-filter=U"])
                .contains("phase.txt")
        );
    }

    fn resolve_rebase(&self) {
        // Keep the protected hunk exact. An adjacent addition conflicts during scoped
        // replay; this separated resolution edit still exercises additional content.
        fs::write(
            self.holder.join("phase.txt"),
            contents("one", "two").replace("\ne\n", "\ne\nupstream resolution retained\n"),
        )
        .expect("conflict resolution should keep the protected hunk");
        GIT.run(&self.holder, ["add", "phase.txt"]);
        let continued = git_command(BERTH_EXECUTABLE)
            .args(["rebase", "--continue"])
            .env("GIT_EDITOR", "true")
            .current_dir(&self.holder)
            .output()
            .expect("rebase continuation should run");
        assert!(
            continued.status.success(),
            "{}",
            String::from_utf8_lossy(&continued.stderr)
        );
    }

    /// Keep the real committed hook's marker by making journal writes fail during amend.
    fn deferred_amend(&self, first: &str, later: &str) -> PathBuf {
        let old_tip = self.head();
        fs::write(self.holder.join("phase.txt"), contents(first, later))
            .expect("amend source should write");
        GIT.run(&self.holder, ["add", "phase.txt"]);
        let journal = self.root().join(JOURNAL_PATH);
        let permissions = make_read_only(&journal);
        let amended = GIT.output(
            &self.holder,
            [
                "commit",
                "--amend",
                "--quiet",
                "-m",
                "deferred rewritten checkpoint",
            ],
        );
        fs::set_permissions(&journal, permissions).expect("journal permissions should restore");
        assert!(
            amended.status.success(),
            "committed hook must not undo the amend: {}",
            String::from_utf8_lossy(&amended.stderr)
        );
        let markers = pending_markers(self.root());
        assert_eq!(
            markers.len(),
            1,
            "the committed hook must durably capture the rewrite: {markers:?}"
        );
        let marker: Value =
            serde_json::from_slice(&fs::read(&markers[0]).expect("captured marker should read"))
                .expect("captured marker should decode");
        assert_eq!(marker["kind"], "branch_rewrite");
        assert_eq!(
            marker["worktree_administrative_directory"],
            GIT.stdout(&self.holder, ["rev-parse", "--absolute-git-dir"])
        );
        assert_eq!(
            marker["pairs"],
            serde_json::json!([{"old": old_tip, "new": self.head()}])
        );
        markers[0].clone()
    }
}

/// Ordinary reconciliation can finish before integration or be overtaken by its hooks.
#[derive(Clone, Copy)]
enum ReconciliationTiming {
    BeforeFastForward,
    DuringIntegration,
}

pub(super) fn checkpoint_ranges_survive_two_rebases() {
    for timing in [
        ReconciliationTiming::BeforeFastForward,
        ReconciliationTiming::DuringIntegration,
    ] {
        checkpoint_ranges_with_reconciliation(timing);
    }
}

fn checkpoint_ranges_with_reconciliation(timing: ReconciliationTiming) {
    let fixture = RewriteFixture::new();
    let first = fixture.start_phase();
    fixture.commit("one", "base");
    let first_old_tip = fixture.head();
    fixture.checkpoint(&first);
    let second = fixture.start_phase();
    fixture.commit("one", "two");
    fixture.upstream_conflict();
    fixture.stop_rebase();
    fixture.resolve_rebase();
    drain_markers(&fixture, &[&first], CheckpointPlacement::BesideTrunk);
    let first_rebased_tip = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]);
    assert_ne!(first_old_tip, first_rebased_tip);
    assert_latest_anchors(&fixture, &first, &first_rebased_tip);
    let events = journal_events(fixture.root());
    let second_anchor = events.iter().rev().find(|event| {
        event["op"] == "resnapshot"
            && event["reservation_id"] == second
            && event["snapshot"]["stage"] == "active"
    });
    assert_eq!(
        second_anchor.map(|event| &event["snapshot"]["claim_snapshot"]),
        Some(&serde_json::json!(first_rebased_tip)),
        "the conflict-resolved active phase must start after the earlier checkpoint: {events:?}"
    );
    fixture.checkpoint(&second);
    GIT.run(
        &fixture.holder,
        [
            "commit",
            "--amend",
            "--quiet",
            "-m",
            "amended second checkpoint",
        ],
    );
    drain_markers(
        &fixture,
        &[&first, &second],
        CheckpointPlacement::BesideTrunk,
    );
    assert_latest_anchors(&fixture, &second, &fixture.head());

    commit_file(
        fixture.root(),
        "upstream.txt",
        "second upstream change\n",
        "second upstream base",
    );
    GIT.run(&fixture.holder, ["rebase", "main"]);
    let final_tip = fixture.head();
    let earlier_tip = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]);
    assert_ne!(earlier_tip, final_tip);
    assert_ne!(first_rebased_tip, earlier_tip);
    if matches!(timing, ReconciliationTiming::BeforeFastForward) {
        drain_markers(
            &fixture,
            &[&first, &second],
            CheckpointPlacement::BesideTrunk,
        );
        assert_latest_anchors(&fixture, &first, &earlier_tip);
        assert_latest_anchors(&fixture, &second, &final_tip);
    }
    // The prepared gate and drift hooks may already consume some markers or settle
    // subjects. Every board pass must settle each accepted subject already on trunk.
    GIT.run(fixture.root(), ["merge", "--quiet", "--ff-only", "holder"]);
    drain_markers(&fixture, &[&first, &second], CheckpointPlacement::OnTrunk);
    assert_final_checkpoint(&fixture, &first, &earlier_tip, &final_tip);
    assert_final_checkpoint(&fixture, &second, &final_tip, &final_tip);
    GIT.run(&fixture.holder, ["switch", "--quiet", "--detach"]);
    GIT.run(fixture.root(), ["branch", "-D", "holder"]);
    let settled = board(fixture.root());
    for id in [&first, &second] {
        assert_eq!(
            snapshot(&settled, id)["lifecycle"]["stage"],
            "released",
            "{settled}"
        );
        assert_eq!(
            journal_events(fixture.root())
                .iter()
                .filter(|event| event["op"] == "release" && event["reservation_id"] == *id)
                .count(),
            1
        );
    }
    assert_eq!(
        settled["payload"]["data"]["alerts"]["entries"],
        serde_json::json!([])
    );
    let journal = fs::read(fixture.root().join(JOURNAL_PATH)).expect("settled journal should read");
    board(fixture.root());
    assert_eq!(
        fs::read(fixture.root().join(JOURNAL_PATH)).expect("repeated journal should read"),
        journal
    );
}

/// Missing apply onto still captures and reanchors exactly the outstanding phase.
pub(super) fn uninterrupted_apply_rebase_reanchors_only_the_checkpoint_commit() {
    apply_rebase_reanchors_checkpoint(ApplyRebaseProgress::Uninterrupted);
}

/// A conflict writes apply onto and preserves the same outstanding phase interval.
pub(super) fn conflicted_apply_rebase_reanchors_only_the_checkpoint_commit() {
    apply_rebase_reanchors_checkpoint(ApplyRebaseProgress::Conflicted);
}

/// Creating another branch during stopped apply must leave the holder's map usable.
pub(super) fn side_branch_creation_during_stopped_apply_rebase_preserves_checkpoint_capture() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    let first_tip = fixture.head();
    // Replay this first commit before the conflict so Git itself writes the map.
    fs::write(fixture.holder.join("phase.txt"), contents("one", "two"))
        .expect("second protected hunk should write");
    fs::write(fixture.holder.join("unreserved.txt"), "holder addition\n")
        .expect("unreserved conflict should write");
    GIT.run(&fixture.holder, ["add", "phase.txt", "unreserved.txt"]);
    let tree = GIT.stdout(&fixture.holder, ["write-tree"]);
    // Avoid post-commit drift widening the reservation into the conflict path.
    let old_tip = GIT.stdout(
        &fixture.holder,
        [
            "commit-tree",
            &tree,
            "-p",
            &first_tip,
            "-m",
            "second phase hunk",
        ],
    );
    GIT.run(
        &fixture.holder,
        ["update-ref", "refs/heads/holder", &old_tip, &first_tip],
    );
    fixture.checkpoint(&id);
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| { event["op"] == "widen" && event["reservation_id"] == id })
    );
    commit_file(
        fixture.root(),
        "unreserved.txt",
        "trunk addition\n",
        "advance trunk",
    );
    let trunk = GIT.stdout(fixture.root(), ["rev-parse", "main"]);
    let observation = tempdir().expect("apply observation should exist");
    create_side_branch_during_stopped_apply(&fixture, observation.path(), &trunk, &old_tip);
    fs::write(fixture.holder.join("unreserved.txt"), "trunk addition\n")
        .expect("resolution should preserve trunk content");
    GIT.run(&fixture.holder, ["add", "unreserved.txt"]);
    let continued = observed_apply_command(&fixture, observation.path())
        .args(["rebase", "--continue"])
        .env("GIT_EDITOR", "true")
        .output()
        .expect("apply rebase should continue");
    assert!(
        continued.status.success(),
        "{}",
        String::from_utf8_lossy(&continued.stderr)
    );
    let hooks = fs::read_to_string(observation.path().join("hooks"))
        .expect("continued hook observation should read");
    assert!(hooks.contains("apply-map onto=present result=0"), "{hooks}");
    assert!(
        hooks
            .lines()
            .filter_map(|line| line.split_once("result="))
            .all(|(_, result)| result == "0"),
        "{hooks}"
    );
    let rewritten_tip = fixture.head();
    let rewritten_first = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]);
    assert_ne!(rewritten_tip, old_tip);
    assert_eq!(GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^^"]), trunk);
    let markers = pending_markers(fixture.root());
    assert_eq!(
        markers.len(),
        1,
        "only the rebase branch should be captured: {hooks}"
    );
    let marker: Value = serde_json::from_slice(&fs::read(&markers[0]).expect("marker should read"))
        .expect("marker should decode");
    assert_eq!(marker["kind"], "branch_rewrite");
    assert_eq!(marker["new_tips"], serde_json::json!([rewritten_tip]));
    assert_eq!(
        marker["pairs"],
        serde_json::json!([
            {"old": first_tip, "new": rewritten_first},
            {"old": old_tip, "new": rewritten_tip}
        ])
    );
    drain_markers(&fixture, &[&id], CheckpointPlacement::BesideTrunk);
    let events = journal_events(fixture.root());
    let accepted = events
        .iter()
        .rev()
        .find(|event| event["op"] == "resnapshot" && event["reservation_id"] == id)
        .expect("the whole checkpoint must reanchor");
    assert_eq!(accepted["snapshot"]["stage"], "outstanding");
    assert_eq!(accepted["snapshot"]["phase_start_head"], trunk);
    assert_eq!(accepted["snapshot"]["protected_tip"], rewritten_tip);
    assert_eq!(retained_tip(&fixture, &id), rewritten_tip);
    assert_eq!(
        snapshot(&board(fixture.root()), &id)["lifecycle"]["stage"],
        "outstanding"
    );
}

/// Observe the branch command separately: Git ignores a committed hook's failure.
fn create_side_branch_during_stopped_apply(
    fixture: &RewriteFixture,
    observation: &Path,
    trunk: &str,
    old_tip: &str,
) {
    let rebased = observed_apply_command(fixture, observation)
        .args(["rebase", "--apply", "main"])
        .output()
        .expect("apply rebase should run");
    assert!(!rebased.status.success(), "the second commit must conflict");
    assert_eq!(
        GIT.stdout(&fixture.holder, ["diff", "--name-only", "--diff-filter=U"]),
        "unreserved.txt"
    );
    let administrative_directory = GIT.stdout(&fixture.holder, ["rev-parse", "--absolute-git-dir"]);
    let apply_directory = Path::new(&administrative_directory).join("rebase-apply");
    for (name, expected) in [
        ("onto", trunk),
        ("orig-head", old_tip),
        ("head-name", "refs/heads/holder"),
    ] {
        assert_eq!(
            fs::read_to_string(apply_directory.join(name))
                .expect("stopped state should read")
                .trim(),
            expected
        );
    }
    assert!(
        !fs::read_to_string(apply_directory.join("rewritten"))
            .expect("the first replay must write the real apply map")
            .trim()
            .is_empty()
    );
    assert!(pending_markers(fixture.root()).is_empty());
    fs::write(observation.join("hooks"), "").expect("branch observation should start empty");
    let created = observed_apply_command(fixture, observation)
        .args(["branch", "side"])
        .output()
        .expect("side branch creation should run");
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let hooks = fs::read_to_string(observation.join("hooks")).expect("branch hooks should read");
    assert!(
        hooks.contains(&format!(
            "{} {} refs/heads/side",
            "0".repeat(40),
            fixture.head()
        )),
        "{hooks}"
    );
    assert!(
        hooks.contains("__reference-transaction committed"),
        "{hooks}"
    );
    assert!(hooks.contains("apply-map onto=present result=0"), "{hooks}");
    assert!(
        hooks
            .lines()
            .filter_map(|line| line.split_once("result="))
            .all(|(_, result)| result == "0"),
        "side creation must not error a managed hook: {hooks}"
    );
    assert!(
        pending_markers(fixture.root()).is_empty(),
        "side creation must not capture a rewrite"
    );
}

/// Whether Git writes the apply backend's basic state by stopping for a conflict.
#[derive(Clone, Copy)]
enum ApplyRebaseProgress {
    Uninterrupted,
    Conflicted,
}

/// Exercise both apply state lifetimes with the same protected checkpoint.
fn apply_rebase_reanchors_checkpoint(progress: ApplyRebaseProgress) {
    let fixture = RewriteFixture::new();
    let old_start = fixture.head();
    let id = fixture.start_phase();
    if matches!(progress, ApplyRebaseProgress::Conflicted) {
        fs::write(fixture.holder.join("unreserved.txt"), "holder addition\n")
            .expect("unreserved conflicting change should write");
        fs::write(fixture.holder.join("phase.txt"), contents("one", "base"))
            .expect("protected hunk should write");
        GIT.run(&fixture.holder, ["add", "phase.txt", "unreserved.txt"]);
        // The conflict must stay outside the reservation: ordinary post-commit
        // drift would widen its scope to include unreserved.txt. Construct A with
        // plumbing while retaining the managed hook on the actual branch update.
        let tree = GIT.stdout(&fixture.holder, ["write-tree"]);
        let tip = GIT.stdout(
            &fixture.holder,
            ["commit-tree", &tree, "-p", &old_start, "-m", "holder phase"],
        );
        GIT.run(
            &fixture.holder,
            ["update-ref", "refs/heads/holder", &tip, &old_start],
        );
    } else {
        fixture.commit("one", "base");
    }
    let old_tip = fixture.head();
    fixture.checkpoint(&id);
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| { event["op"] == "widen" && event["reservation_id"] == id }),
        "the conflicting path must remain outside the protected scope"
    );
    commit_file(
        fixture.root(),
        "unreserved.txt",
        "trunk addition\n",
        "advance trunk outside the reserved scope",
    );
    let trunk = GIT.stdout(fixture.root(), ["rev-parse", "main"]);
    assert_ne!(trunk, old_start);
    assert!(pending_markers(fixture.root()).is_empty());

    let observation = tempdir().expect("apply hook observation directory should exist");
    complete_observed_apply_rebase(&fixture, observation.path(), progress, &trunk);
    let hooks = fs::read_to_string(observation.path().join("hooks"))
        .expect("the real apply rebase must invoke the managed executable");
    let expected_state = match progress {
        ApplyRebaseProgress::Uninterrupted => "apply-map onto=absent result=0",
        ApplyRebaseProgress::Conflicted => "apply-map onto=present result=0",
    };
    assert!(hooks.contains(expected_state), "{hooks}");
    assert!(
        hooks
            .lines()
            .filter_map(|line| line.split_once("result="))
            .all(|(_, result)| result == "0"),
        "every managed hook invocation must succeed: {hooks}"
    );
    let rewritten_tip = fixture.head();
    assert_ne!(rewritten_tip, old_tip);
    assert_eq!(GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]), trunk);
    let markers = pending_markers(fixture.root());
    assert_eq!(
        markers.len(),
        1,
        "apply rebase must capture its rewrite map: {hooks}"
    );
    let marker: Value = serde_json::from_slice(&fs::read(&markers[0]).expect("marker should read"))
        .expect("marker should decode");
    assert_eq!(marker["kind"], "branch_rewrite");
    assert_eq!(
        marker["pairs"],
        serde_json::json!([{"old": old_tip, "new": rewritten_tip}])
    );
    assert_eq!(
        marker["created_commits"],
        serde_json::json!([rewritten_tip]),
        "the unrelated trunk commit must not widen the protected interval"
    );
    drain_markers(&fixture, &[&id], CheckpointPlacement::BesideTrunk);
    assert_latest_anchors(&fixture, &id, &rewritten_tip);
    assert_eq!(retained_tip(&fixture, &id), rewritten_tip);
    assert_eq!(
        GIT.stdout(
            fixture.root(),
            ["rev-list", &format!("{trunk}..{rewritten_tip}")]
        ),
        rewritten_tip
    );
    let observed = board(fixture.root());
    assert_eq!(
        snapshot(&observed, &id)["lifecycle"]["stage"],
        "outstanding"
    );
}

/// Resolve only an unreserved add/add conflict, leaving the protected hunk exact.
fn complete_observed_apply_rebase(
    fixture: &RewriteFixture,
    observation: &Path,
    progress: ApplyRebaseProgress,
    trunk: &str,
) {
    let rebased = observed_apply_command(fixture, observation)
        .args(["rebase", "--apply", "main"])
        .output()
        .expect("apply rebase should run");
    match progress {
        ApplyRebaseProgress::Uninterrupted => assert!(
            rebased.status.success(),
            "{}",
            String::from_utf8_lossy(&rebased.stderr)
        ),
        ApplyRebaseProgress::Conflicted => {
            assert!(!rebased.status.success(), "apply rebase must stop");
            assert_eq!(
                GIT.stdout(&fixture.holder, ["diff", "--name-only", "--diff-filter=U"]),
                "unreserved.txt"
            );
            let administrative_directory =
                GIT.stdout(&fixture.holder, ["rev-parse", "--absolute-git-dir"]);
            assert_eq!(
                fs::read_to_string(Path::new(&administrative_directory).join("rebase-apply/onto"))
                    .expect("stopped apply rebase must record onto")
                    .trim(),
                trunk
            );
            assert!(pending_markers(fixture.root()).is_empty());
            fs::write(fixture.holder.join("unreserved.txt"), "trunk addition\n")
                .expect("resolution should preserve the unrelated trunk change");
            GIT.run(&fixture.holder, ["add", "unreserved.txt"]);
            let continued = observed_apply_command(fixture, observation)
                .args(["rebase", "--continue"])
                .env("GIT_EDITOR", "true")
                .output()
                .expect("apply rebase should continue");
            assert!(
                continued.status.success(),
                "{}",
                String::from_utf8_lossy(&continued.stderr)
            );
        },
    }
}

/// Observe the real managed executable without changing its input or return status.
fn observed_apply_command(fixture: &RewriteFixture, observation: &Path) -> Command {
    let wrapper = observation.join("cargo-berth");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
input=$(mktemp "$CARGO_BERTH_TEST_APPLY_OBSERVATION/input.XXXXXX") || exit 97
cat > "$input"
printf '%s\n' "$*" >> "$CARGO_BERTH_TEST_APPLY_OBSERVATION/hooks"
cat "$input" >> "$CARGO_BERTH_TEST_APPLY_OBSERVATION/hooks"
apply_state=no-apply-map
if [ -f "$CARGO_BERTH_TEST_APPLY_DIRECTORY/rewritten" ]; then
    apply_state='apply-map onto=absent'
    if [ -f "$CARGO_BERTH_TEST_APPLY_DIRECTORY/onto" ]; then
        apply_state='apply-map onto=present'
    fi
fi
"$CARGO_BERTH_TEST_REAL_BERTH" "$@" < "$input"
result=$?
printf '%s result=%s\n' "$apply_state" "$result" >> "$CARGO_BERTH_TEST_APPLY_OBSERVATION/hooks"
rm -f "$input"
exit "$result"
"#,
    )
    .expect("apply observer should write");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("apply observer should execute");
    let mut command = git_command(wrapper.to_str().expect("observer path should be UTF-8"));
    command
        .current_dir(&fixture.holder)
        .env("CARGO_BERTH_TEST_REAL_BERTH", BERTH_EXECUTABLE)
        .env(
            "CARGO_BERTH_TEST_APPLY_DIRECTORY",
            Path::new(&GIT.stdout(&fixture.holder, ["rev-parse", "--absolute-git-dir"]))
                .join("rebase-apply"),
        )
        .env("CARGO_BERTH_TEST_APPLY_OBSERVATION", observation);
    command
}

pub(super) fn aborted_rebase_does_not_leave_a_rewrite_marker() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    fixture.checkpoint(&id);
    fixture.commit("one", "two");
    let old_head = fixture.head();
    fixture.upstream_conflict();
    fixture.stop_rebase();
    assert!(pending_markers(fixture.root()).is_empty());
    GIT.run(&fixture.holder, ["rebase", "--abort"]);
    assert_eq!(fixture.head(), old_head);
    assert!(pending_markers(fixture.root()).is_empty());
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| event["op"] == "resnapshot" && event["reservation_id"] == id)
    );
}

pub(super) fn rebase_onto_another_reservation_keeps_its_work_outside_the_active_phase() {
    rebase_onto_another_reservation(SplitReservationStage::Active);
}

pub(super) fn rebase_onto_another_reservation_keeps_its_work_outside_the_checkpoint() {
    rebase_onto_another_reservation(SplitReservationStage::Outstanding);
}

fn rebase_onto_another_reservation(stage: SplitReservationStage) {
    let fixture = RewriteFixture::new();
    let trunk = GIT.stdout(fixture.root(), ["rev-parse", "main"]);
    let other = fixture.worktrees.path().join("other-reservation");
    GIT.run(
        fixture.root(),
        [
            "worktree",
            "add",
            "--quiet",
            "-b",
            "other-reservation",
            other.to_str().expect("other holder path should be UTF-8"),
        ],
    );
    let other_claim = claim(&other, "file:other.txt", super::SECOND_RUN);
    assert!(
        other_claim.status.success(),
        "{}",
        json_output(&other_claim)
    );
    let other_id = reservation_id(&other_claim);
    commit_file(
        &other,
        "other.txt",
        "other reservation's work\n",
        "other phase",
    );
    let other_tip = GIT.stdout(&other, ["rev-parse", "HEAD"]);
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    let old_tip = fixture.head();
    if matches!(stage, SplitReservationStage::Outstanding) {
        fixture.checkpoint(&id);
    }

    GIT.run(&fixture.holder, ["rebase", "other-reservation"]);
    let rewritten_tip = fixture.head();
    assert_ne!(rewritten_tip, old_tip);
    assert_eq!(
        GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]),
        other_tip
    );
    assert_eq!(GIT.stdout(fixture.root(), ["rev-parse", "main"]), trunk);
    drain_markers(&fixture, &[&id], CheckpointPlacement::BesideTrunk);
    assert_split_anchors(&fixture, &id, &other_tip, &rewritten_tip, &trunk, stage);
    let protected_commits = GIT.stdout(
        fixture.root(),
        ["rev-list", &format!("{other_tip}..{rewritten_tip}")],
    );
    assert_eq!(protected_commits, rewritten_tip);
    let protected_paths = GIT.stdout(
        fixture.root(),
        ["diff", "--name-only", &other_tip, &rewritten_tip],
    );
    assert_eq!(protected_paths, "phase.txt");
    assert!(
        !journal_events(fixture.root()).iter().any(|event| {
            event["reservation_id"] == other_id
                && matches!(event["op"].as_str(), Some("resnapshot" | "release"))
        }),
        "rebasing the second reservation must preserve the first reservation's ownership"
    );
}

pub(super) fn pruned_unneeded_rewrite_pair_does_not_prevent_marker_retirement() {
    let fixture = RewriteFixture::new();
    fixture.commit("unreserved", "base");
    let old_tip = fixture.head();
    commit_file(
        fixture.root(),
        "upstream.txt",
        "upstream\n",
        "advance trunk",
    );
    GIT.run(&fixture.holder, ["rebase", "main"]);
    let markers = pending_markers(fixture.root());
    assert_eq!(markers.len(), 1, "real rebase must capture its map");
    let marker: Value = serde_json::from_slice(&fs::read(&markers[0]).expect("marker should read"))
        .expect("marker should decode");
    assert_eq!(
        marker["pairs"],
        serde_json::json!([{"old": old_tip, "new": fixture.head()}])
    );
    GIT.run(
        fixture.root(),
        ["reflog", "expire", "--expire=now", "--all"],
    );
    GIT.run(fixture.root(), ["gc", "--prune=now"]);
    assert!(
        !GIT.output(fixture.root(), ["cat-file", "-e", &old_tip])
            .status
            .success(),
        "fixture must actually prune the old mapped commit"
    );
    let observed = board(fixture.root());
    assert!(pending_markers(fixture.root()).is_empty(), "{observed}");
    assert_eq!(
        observed["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
}

pub(super) fn stopped_rebase_defers_a_pending_reanchor() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "two");
    fixture.checkpoint(&id);
    fixture.upstream_conflict();
    let old_tip = retained_tip(&fixture, &id);
    let marker = fixture.deferred_amend("one", "two");
    let amended_tip = fixture.head();
    fixture.stop_rebase();
    let stopped = board(fixture.root());
    assert!(
        marker.exists(),
        "a stopped rebase must defer its checkout's marker"
    );
    assert_eq!(
        stopped["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
    assert_eq!(
        stopped["payload"]["data"]["bypass_audit"]["entries"],
        serde_json::json!([])
    );
    assert!(
        !stopped["payload"]["data"]["alerts"]["entries"]
            .to_string()
            .contains("bypass"),
        "a pending rewrite must not produce an unrecorded-bypass notice: {stopped}"
    );
    assert_eq!(retained_tip(&fixture, &id), old_tip);
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| event["op"] == "resnapshot" && event["reservation_id"] == id)
    );
    GIT.run(&fixture.holder, ["rebase", "--abort"]);
    board(fixture.root());
    assert!(!marker.exists());
    assert_eq!(retained_tip(&fixture, &id), amended_tip);
    assert_latest_anchors(&fixture, &id, &amended_tip);
}

pub(super) fn failed_append_preserves_rewrite_marker_and_retention() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    fixture.checkpoint(&id);
    let old_tip = retained_tip(&fixture, &id);
    let marker = fixture.deferred_amend("one", "base");
    let marker_bytes = fs::read(&marker).expect("rewrite marker should read");
    let journal_path = fixture.root().join(JOURNAL_PATH);
    let journal = fs::read(&journal_path).expect("journal should read");
    let permissions = make_read_only(&journal_path);
    let failed = run_berth(fixture.root(), &["board", "--json"]);
    fs::set_permissions(&journal_path, permissions).expect("journal permissions should restore");
    assert_eq!(
        failed.status.code(),
        Some(4),
        "{}",
        String::from_utf8_lossy(&failed.stdout)
    );
    assert_eq!(
        fs::read(&marker).expect("failed append must retain marker"),
        marker_bytes
    );
    assert_eq!(
        fs::read(&journal_path).expect("failed journal should read"),
        journal
    );
    assert_eq!(retained_tip(&fixture, &id), old_tip);
    board(fixture.root());
    assert!(!marker.exists(), "successful retry must remove the marker");
    assert_eq!(retained_tip(&fixture, &id), fixture.head());
    assert_latest_anchors(&fixture, &id, &fixture.head());
}

pub(super) fn refused_reanchor_preserves_live_and_orphaned_checkpoint_anchors() {
    for orphaned in [false, true] {
        let fixture = RewriteFixture::new();
        let id = fixture.start_phase();
        fixture.commit("one", "base");
        fixture.checkpoint(&id);
        let before = board(fixture.root());
        let old_tip = retained_tip(&fixture, &id);
        let marker = fixture.deferred_amend("discarded", "base");
        if orphaned {
            fs::rename(
                &fixture.holder,
                fixture.worktrees.path().join("missing-holder"),
            )
            .expect("holder should become unavailable");
            GIT.run(fixture.root(), ["worktree", "prune", "--expire", "now"]);
        }
        let observed = board(fixture.root());
        assert!(
            !marker.exists(),
            "a definitive refusal must complete even when its administrative directory is missing"
        );
        let retained = snapshot(&observed, &id);
        assert_eq!(retained["lifecycle"]["stage"], "outstanding", "{observed}");
        assert_eq!(retained["lifecycle"], snapshot(&before, &id)["lifecycle"]);
        assert_eq!(retained_tip(&fixture, &id), old_tip);
        assert!(
            !journal_events(fixture.root())
                .iter()
                .any(|event| event["op"] == "resnapshot" && event["reservation_id"] == id)
        );
        assert_eq!(retained["edit_blocking_status"], "blocking");
        let checked = run_berth(fixture.root(), &["check", "file:phase.txt", "--json"]);
        assert_eq!(
            checked.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&checked.stdout)
        );
        assert!(
            json_output(&checked)["blocked_by"]
                .as_array()
                .expect("check should name blockers")
                .contains(&serde_json::json!(id))
        );
        if orphaned {
            let alert = observed["payload"]["data"]["alerts"]["entries"]
                .as_array()
                .expect("alerts should be listed")
                .iter()
                .find(|alert| {
                    alert["kind"] == "orphaned_outstanding" && alert["reservation_id"] == id
                })
                .expect("refused orphan must retain its existing recovery action");
            assert_eq!(alert["resolution"]["action"], "recover_with_trunk");
            assert_eq!(
                alert["resolution"]["recovery"]["action"]["reservation_id"],
                id
            );
            assert_eq!(
                alert["resolution"]["recovery"]["trunk_oid"],
                GIT.stdout(fixture.root(), ["rev-parse", "main"])
            );
        }
    }
}

pub(super) fn mixed_rewrite_and_bypass_markers_report_only_the_bypass() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    fixture.checkpoint(&id);
    let rewrite_marker = fixture.deferred_amend("one", "base");
    let bypass_name = format!("{PENDING_PREFIX}mixed-bypass.json");
    let bypass_marker = fixture.root().join(".git").join(&bypass_name);
    fs::write(&bypass_marker, r#"{"cause":{"kind":"environment_override","bypassed_merge":"mixed-marker-test"},"occurrence_time":{"status":"unavailable"}}"#).expect("legacy bypass marker should write");
    let observed = board(fixture.root());
    assert!(!rewrite_marker.exists());
    assert!(!bypass_marker.exists());
    assert_eq!(
        observed["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([bypass_name])
    );
    assert_eq!(
        observed["payload"]["data"]["bypass_audit"]["entries"]
            .as_array()
            .expect("audit should list bypasses")
            .len(),
        1
    );
    assert_eq!(
        journal_events(fixture.root())
            .iter()
            .filter(|event| event["op"] == "bypass")
            .count(),
        1
    );
    assert_latest_anchors(&fixture, &id, &fixture.head());
    let repeated = board(fixture.root());
    assert_eq!(
        repeated["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
}

pub(super) fn drift_cleans_up_rewrite_markers_without_reporting_bypasses() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    fixture.checkpoint(&id);
    let active_id = fixture.start_phase();
    let marker = fixture.deferred_amend("one", "base");
    let drift = run_berth(
        &fixture.holder,
        &["drift", "--full", "--reservation", &active_id, "--json"],
    );
    assert!(
        drift.status.success(),
        "{}",
        String::from_utf8_lossy(&drift.stdout)
    );
    assert!(
        !marker.exists(),
        "rewrite cleanup must not depend on bypass reporting"
    );
    assert_latest_anchors(&fixture, &id, &fixture.head());
    assert_eq!(
        journal_events(fixture.root())
            .iter()
            .filter(|event| event["op"] == "bypass")
            .count(),
        0
    );
}

fn contents(first: &str, later: &str) -> String {
    format!("first={first}\n{HUNK_SEPARATOR}later={later}\n")
}

/// Both Git workflows nominate only the last commit of the two-commit split.
#[derive(Clone, Copy)]
enum SplitWorkflow {
    InteractiveRebase,
    SoftReset,
}

impl SplitWorkflow {
    fn stop_before_split(self, fixture: &RewriteFixture, editor: &Path) {
        match self {
            Self::InteractiveRebase => {
                split_rebase::stop_with_editor(
                    &fixture.holder,
                    editor,
                    &["--onto", "main", "HEAD^"],
                );
            },
            Self::SoftReset => {
                // Keep the old branch tip until the replacement is complete, so its
                // real ref transaction captures the old-to-new fallback map.
                GIT.run(&fixture.holder, ["switch", "--quiet", "--detach"]);
                GIT.run(&fixture.holder, ["reset", "--soft", "HEAD~1"]);
            },
        }
    }

    fn complete_split(self, fixture: &RewriteFixture, old_tip: &str, split_tip: &str) {
        match self {
            Self::InteractiveRebase => {
                split_rebase::continue_rebase(&fixture.holder);
            },
            Self::SoftReset => {
                GIT.run(
                    &fixture.holder,
                    ["update-ref", "refs/heads/holder", split_tip, old_tip],
                );
                GIT.run(&fixture.holder, ["switch", "--quiet", "holder"]);
            },
        }
    }
}

pub(super) fn interactive_split_keeps_both_protected_hunks_in_the_checkpoint() {
    assert_split_keeps_both_hunks(
        SplitWorkflow::InteractiveRebase,
        SplitReservationStage::Outstanding,
    );
}

pub(super) fn soft_reset_split_keeps_both_protected_hunks_in_the_checkpoint() {
    assert_split_keeps_both_hunks(SplitWorkflow::SoftReset, SplitReservationStage::Outstanding);
}

pub(super) fn interactive_split_keeps_both_protected_hunks_in_the_active_phase() {
    assert_split_keeps_both_hunks(
        SplitWorkflow::InteractiveRebase,
        SplitReservationStage::Active,
    );
}

pub(super) fn soft_reset_split_keeps_both_protected_hunks_in_the_active_phase() {
    assert_split_keeps_both_hunks(SplitWorkflow::SoftReset, SplitReservationStage::Active);
}

/// The reservation state whose anchors the real rewrite must preserve.
#[derive(Clone, Copy)]
enum SplitReservationStage {
    Active,
    Outstanding,
}

fn assert_split_keeps_both_hunks(workflow: SplitWorkflow, stage: SplitReservationStage) {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "two");
    let old_tip = fixture.head();
    if matches!(stage, SplitReservationStage::Outstanding) {
        fixture.checkpoint(&id);
    }
    if matches!(
        (stage, workflow),
        (
            SplitReservationStage::Active,
            SplitWorkflow::InteractiveRebase
        )
    ) {
        commit_file(
            fixture.root(),
            "upstream.txt",
            "new trunk base\n",
            "advance the active rebase base",
        );
    }
    let editor = fixture.worktrees.path().join("sequence-editor");
    write_first_rebase_action(&editor, "edit");
    workflow.stop_before_split(&fixture, &editor);
    fixture.commit("one", "base");
    let first_split_tip = fixture.head();
    let expected_start = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]);
    fixture.commit("one", "two");
    let split_tip = fixture.head();
    workflow.complete_split(&fixture, &old_tip, &split_tip);
    let markers = pending_markers(fixture.root());
    assert_eq!(
        markers.len(),
        1,
        "the real rewrite must leave a marker: {markers:?}"
    );
    let marker: Value = serde_json::from_slice(&fs::read(&markers[0]).expect("marker should read"))
        .expect("marker should decode");
    assert_eq!(
        marker["pairs"],
        serde_json::json!([{"old": old_tip, "new": split_tip}])
    );
    drain_markers(&fixture, &[&id], CheckpointPlacement::BesideTrunk);
    assert_split_anchors(
        &fixture,
        &id,
        &expected_start,
        &split_tip,
        &first_split_tip,
        stage,
    );
    if matches!(stage, SplitReservationStage::Active) {
        fixture.checkpoint(&id);
    }

    write_first_rebase_action(&editor, "drop");
    let dropped = git_command(BERTH_EXECUTABLE)
        .args(["rebase", "-i", &expected_start])
        .env("GIT_SEQUENCE_EDITOR", &editor)
        .current_dir(&fixture.holder)
        .output()
        .expect("second rebase should run");
    assert!(
        dropped.status.success(),
        "{}",
        String::from_utf8_lossy(&dropped.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.holder.join("phase.txt")).expect("split source should read"),
        contents("base", "two")
    );
    GIT.run(fixture.root(), ["merge", "--quiet", "--ff-only", "holder"]);
    let observed = board(fixture.root());
    assert_eq!(
        snapshot(&observed, &id)["lifecycle"]["stage"],
        "outstanding",
        "lost first hunk must block settlement: {observed}"
    );
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| event["op"] == "release" && event["reservation_id"] == id)
    );
}

/// Ref changes that must not change the commits attributed to a finished rewrite.
#[derive(Clone, Copy)]
enum SplitRefChange {
    ActiveRetention,
    UpdatedSibling,
    SwitchedCheckout,
}

pub(super) fn split_checkpoint_survives_an_active_phases_rewritten_retention_ref() {
    assert_split_survives_ref_change(SplitRefChange::ActiveRetention);
}

pub(super) fn split_checkpoint_survives_stacked_branches_rewritten_with_update_refs() {
    assert_split_survives_ref_change(SplitRefChange::UpdatedSibling);
}

/// Branch writes within one rebase must observe the same operation boundary.
#[derive(Clone, Copy)]
enum StackedBranchWriteOrder {
    UpperFirst,
    LowerFirst,
}

pub(super) fn upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_upper_first() {
    assert_upper_checkpoint_survives_stacked_split(StackedBranchWriteOrder::UpperFirst);
}

pub(super) fn upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_lower_first() {
    assert_upper_checkpoint_survives_stacked_split(StackedBranchWriteOrder::LowerFirst);
}

fn assert_upper_checkpoint_survives_stacked_split(order: StackedBranchWriteOrder) {
    let fixture = RewriteFixture::new();
    let trunk = fixture.head();
    let upper = fixture.start_phase();
    fixture.commit("one", "two");
    let old_lower = fixture.head();
    GIT.run(&fixture.holder, ["branch", "lower"]);
    fixture.commit("one", "three");
    let old_upper = fixture.head();
    fixture.checkpoint(&upper);
    let editor = fixture.worktrees.path().join("stacked-order-editor");
    write_stacked_rebase_editor(&editor, order, &old_lower);
    stop_stacked_split_rebase(&fixture, SplitRefChange::UpdatedSibling, &editor, &trunk);
    fixture.commit("one", "base");
    let first_split = fixture.head();
    fixture.commit("one", "two");
    let split_tip = fixture.head();
    SplitWorkflow::InteractiveRebase.complete_split(&fixture, &old_lower, &split_tip);
    let new_upper = fixture.head();
    assert_ne!(old_upper, new_upper);
    assert_eq!(
        GIT.stdout(&fixture.holder, ["rev-parse", "lower"]),
        split_tip
    );
    let mut markers = pending_markers(fixture.root());
    markers.sort();
    assert_eq!(markers.len(), 2, "each branch must capture its own event");
    let first: Value = serde_json::from_slice(&fs::read(&markers[0]).expect("marker should read"))
        .expect("marker should decode");
    let first_tip = match order {
        StackedBranchWriteOrder::UpperFirst => &new_upper,
        StackedBranchWriteOrder::LowerFirst => &split_tip,
    };
    assert_eq!(first["new_tips"], serde_json::json!([first_tip]));
    assert_split_capture(
        &markers,
        SplitRefChange::UpdatedSibling,
        &trunk,
        &first_split,
        &split_tip,
        &new_upper,
    );
    drain_markers(&fixture, &[&upper], CheckpointPlacement::BesideTrunk);
    assert_split_anchors(
        &fixture,
        &upper,
        &trunk,
        &new_upper,
        &first_split,
        SplitReservationStage::Outstanding,
    );
    assert_dropping_split_preserves_checkpoint(
        &fixture,
        &upper,
        &trunk,
        &new_upper,
        &first_split,
        &editor,
    );
}

/// Move lower after the full map is flushed by exec, before Git writes upper.
fn write_stacked_rebase_editor(editor: &Path, order: StackedBranchWriteOrder, old_lower: &str) {
    if matches!(order, StackedBranchWriteOrder::UpperFirst) {
        write_first_rebase_action(editor, "edit");
        return;
    }
    fs::write(editor, format!(
        "#!/bin/sh\nsed -e '1s/^pick /edit /' -e '/^update-ref refs\\/heads\\/lower$/d' \"$1\" > \"$1.edited\"\nprintf '%s\\n' 'exec git update-ref refs/heads/lower HEAD^ {old_lower}' >> \"$1.edited\"\nmv \"$1.edited\" \"$1\"\n"
    )).expect("lower-first sequence editor should write");
    fs::set_permissions(editor, fs::Permissions::from_mode(0o755))
        .expect("lower-first sequence editor should execute");
}

pub(super) fn split_checkpoint_survives_switching_the_issuing_checkout_before_board() {
    assert_split_survives_ref_change(SplitRefChange::SwitchedCheckout);
}

fn assert_split_survives_ref_change(change: SplitRefChange) {
    let fixture = RewriteFixture::new();
    let trunk = fixture.head();
    let lower = fixture.start_phase();
    fixture.commit("one", "two");
    let old_lower = fixture.head();
    fixture.checkpoint(&lower);
    if matches!(change, SplitRefChange::UpdatedSibling) {
        GIT.run(&fixture.holder, ["branch", "lower"]);
    }
    let mut active_phases = Vec::new();
    if !matches!(change, SplitRefChange::SwitchedCheckout) {
        active_phases.push(fixture.start_phase());
        fixture.commit("one", "three");
    }
    let old_upper = fixture.head();
    let editor = fixture.worktrees.path().join("split-editor");
    if matches!(change, SplitRefChange::UpdatedSibling) {
        split_rebase::stop_for_split(&fixture.holder, &editor, &trunk);
    } else {
        write_first_rebase_action(&editor, "edit");
        stop_stacked_split_rebase(&fixture, change, &editor, &trunk);
    }
    fixture.commit("one", "base");
    let first_split = fixture.head();
    fixture.commit("one", "two");
    let split_tip = fixture.head();
    SplitWorkflow::InteractiveRebase.complete_split(&fixture, &old_lower, &split_tip);
    let new_upper = fixture.head();
    assert_ne!(old_upper, new_upper);
    let markers = pending_markers(fixture.root());
    assert!(
        !markers.is_empty(),
        "the real committed hook must retain its rewrite map"
    );
    if matches!(change, SplitRefChange::UpdatedSibling) {
        assert_eq!(
            GIT.stdout(&fixture.holder, ["rev-parse", "lower"]),
            split_tip
        );
        assert_ne!(new_upper, split_tip);
    }
    assert_split_capture(
        &markers,
        change,
        &trunk,
        &first_split,
        &split_tip,
        &new_upper,
    );
    if matches!(change, SplitRefChange::SwitchedCheckout) {
        switch_before_split_import(&fixture, &markers, &trunk);
    }
    for upper in &active_phases {
        assert_split_anchors(
            &fixture,
            upper,
            &split_tip,
            &new_upper,
            &first_split,
            SplitReservationStage::Active,
        );
    }
    if matches!(change, SplitRefChange::ActiveRetention) {
        checkpoint_upper_before_lower_import(
            &fixture,
            &markers,
            std::mem::take(&mut active_phases),
            &new_upper,
        );
        assert_eq!(retained_tip(&fixture, &lower), old_lower);
    }
    drain_markers(&fixture, &[&lower], CheckpointPlacement::BesideTrunk);
    assert_split_anchors(
        &fixture,
        &lower,
        &trunk,
        &split_tip,
        &first_split,
        SplitReservationStage::Outstanding,
    );
    if matches!(change, SplitRefChange::SwitchedCheckout) {
        GIT.run(&fixture.holder, ["switch", "--quiet", "holder"]);
    }
    for upper in active_phases {
        fixture.checkpoint(&upper);
    }
    assert_dropping_split_preserves_checkpoint(
        &fixture,
        &lower,
        &trunk,
        &split_tip,
        &first_split,
        &editor,
    );
}

/// Stop the real rebase at its lower commit, preserving Git's update-ref todo entries.
fn stop_stacked_split_rebase(
    fixture: &RewriteFixture,
    change: SplitRefChange,
    editor: &Path,
    trunk: &str,
) {
    let mut revisions = vec![trunk];
    if matches!(change, SplitRefChange::UpdatedSibling) {
        revisions.push("--update-refs");
    }
    split_rebase::stop_with_editor(&fixture.holder, editor, &revisions);
}

/// Move HEAD before the pending event is consumed, including through checkout hooks.
fn switch_before_split_import(fixture: &RewriteFixture, markers: &[PathBuf], trunk: &str) {
    // Keep reconciliation pending through post-checkout, too: the marker must
    // be consumed only after the issuing checkout names an unrelated branch.
    let journal = fixture.root().join(JOURNAL_PATH);
    let permissions = make_read_only(&journal);
    let switched = GIT.output(
        &fixture.holder,
        ["switch", "--quiet", "-c", "unrelated", trunk],
    );
    fs::set_permissions(&journal, permissions).expect("journal permissions should restore");
    assert!(
        switched.status.success(),
        "{}",
        String::from_utf8_lossy(&switched.stderr)
    );
    assert_eq!(fixture.head(), trunk);
    assert!(markers.iter().all(|marker| marker.exists()));
}

/// Verify branch-local pairs and both protected split commits in the issuing event.
fn assert_split_capture(
    markers: &[PathBuf],
    change: SplitRefChange,
    trunk: &str,
    first_split: &str,
    split_tip: &str,
    new_upper: &str,
) {
    let captured: Vec<Value> = markers
        .iter()
        .map(|marker| {
            serde_json::from_slice(&fs::read(marker).expect("captured rewrite should read"))
                .expect("captured rewrite should decode")
        })
        .collect();
    for marker in &captured {
        let created = marker["created_commits"]
            .as_array()
            .expect("every rewrite marker must require its event's created commits");
        assert!(
            !created.contains(&serde_json::json!(trunk)),
            "the old base is not newly created"
        );
        assert!(
            marker.get("rewritten_branches").is_none(),
            "markers must not rely on later ref inference"
        );
        for pair in marker["pairs"]
            .as_array()
            .expect("rewrite pairs should be a list")
        {
            assert!(
                created.contains(&pair["new"]),
                "branch pairs must belong to its created set"
            );
        }
    }
    let upper_marker = captured
        .iter()
        .find(|marker| marker["new_tips"] == serde_json::json!([new_upper]))
        .expect("the issuing branch must have its own marker");
    let created = upper_marker["created_commits"]
        .as_array()
        .expect("created commits should be a list");
    assert!(
        created.contains(&serde_json::json!(first_split)),
        "S1 must be captured: {upper_marker}"
    );
    assert!(
        created.contains(&serde_json::json!(split_tip)),
        "S2 must be captured: {upper_marker}"
    );
    if matches!(change, SplitRefChange::UpdatedSibling) {
        let lower_marker = captured
            .iter()
            .find(|marker| marker["new_tips"] == serde_json::json!([split_tip]))
            .expect("the lower branch must have its own marker");
        let lower_created = lower_marker["created_commits"]
            .as_array()
            .expect("lower created commits should be a list");
        assert_eq!(lower_created.len(), 2, "{lower_marker}");
        assert!(lower_created.contains(&serde_json::json!(first_split)));
        assert!(lower_created.contains(&serde_json::json!(split_tip)));
        assert!(
            lower_marker["pairs"]
                .as_array()
                .expect("lower pairs should be a list")
                .iter()
                .any(|pair| pair["new"] == split_tip),
            "an earlier upper update must not hide the lower pair: {lower_marker}"
        );
        assert_eq!(
            upper_marker["pairs"]
                .as_array()
                .expect("upper pairs should be a list")
                .len(),
            2
        );
    }
}

/// Create a real rewritten retention ref before the lower marker becomes readable.
fn checkpoint_upper_before_lower_import(
    fixture: &RewriteFixture,
    markers: &[PathBuf],
    active_phases: Vec<String>,
    new_upper: &str,
) {
    // An unavailable marker lets the already re-anchored upper phase checkpoint
    // first. Its real retention ref now reaches both split commits before the
    // lower checkpoint consumes the original event in a later process.
    let permissions: Vec<_> = markers
        .iter()
        .map(|marker| {
            let permissions = fs::metadata(marker)
                .expect("marker metadata should read")
                .permissions();
            fs::set_permissions(marker, fs::Permissions::from_mode(0o000))
                .expect("marker should become unreadable");
            permissions
        })
        .collect();
    for upper in active_phases {
        fixture.checkpoint(&upper);
        assert_eq!(retained_tip(fixture, &upper), new_upper);
    }
    for (marker, permissions) in markers.iter().zip(permissions) {
        fs::set_permissions(marker, permissions).expect("marker permissions should restore");
    }
}

/// Losing S1 must preserve the accepted interval and block actual-trunk settlement.
fn assert_dropping_split_preserves_checkpoint(
    fixture: &RewriteFixture,
    id: &str,
    trunk: &str,
    protected_tip: &str,
    first_split: &str,
    editor: &Path,
) {
    write_first_rebase_action(editor, "drop");
    let dropped = git_command(BERTH_EXECUTABLE)
        .args(["rebase", "-i", trunk])
        .env("GIT_SEQUENCE_EDITOR", editor)
        .current_dir(&fixture.holder)
        .output()
        .expect("later rebase should drop the first split commit");
    assert!(
        dropped.status.success(),
        "{}",
        String::from_utf8_lossy(&dropped.stderr)
    );
    assert!(
        fs::read_to_string(fixture.holder.join("phase.txt"))
            .expect("rewritten phase should read")
            .starts_with("first=base\n")
    );
    board(fixture.root());
    assert_split_anchors(
        fixture,
        id,
        trunk,
        protected_tip,
        first_split,
        SplitReservationStage::Outstanding,
    );
    assert_eq!(retained_tip(fixture, id), protected_tip);
    GIT.run(fixture.root(), ["merge", "--quiet", "--ff-only", "holder"]);
    let observed = board(fixture.root());
    assert_eq!(snapshot(&observed, id)["lifecycle"]["stage"], "outstanding");
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| { event["op"] == "release" && event["reservation_id"] == id }),
        "dropping the first protected hunk must never release the checkpoint"
    );
}

fn assert_split_anchors(
    fixture: &RewriteFixture,
    id: &str,
    expected_start: &str,
    split_tip: &str,
    first_split_tip: &str,
    stage: SplitReservationStage,
) {
    let events = journal_events(fixture.root());
    let accepted = events
        .iter()
        .rev()
        .find(|event| {
            event["reservation_id"] == id
                && (event["op"] == "resnapshot"
                    || (matches!(stage, SplitReservationStage::Active) && event["op"] == "claim"))
        })
        .expect("split phase should retain its accepted anchors");
    let anchor = match stage {
        SplitReservationStage::Active if accepted["op"] == "claim" => {
            // Splitting on the same base can correctly preserve the original
            // active anchor without appending a redundant resnapshot.
            &accepted["phase_start_head"]
        },
        SplitReservationStage::Active => {
            assert_eq!(accepted["snapshot"]["stage"], "active");
            &accepted["snapshot"]["claim_snapshot"]
        },
        SplitReservationStage::Outstanding => {
            assert_eq!(accepted["snapshot"]["stage"], "outstanding");
            assert_eq!(accepted["snapshot"]["protected_tip"], split_tip);
            &accepted["snapshot"]["phase_start_head"]
        },
    };
    assert_eq!(
        anchor, expected_start,
        "the first split commit's hunk must remain inside the protected interval: {accepted}"
    );
    assert_ne!(anchor, first_split_tip);
}

pub(super) fn unreadable_bypass_marker_does_not_block_rewrite_reconciliation() {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    fixture.checkpoint(&id);
    let rewrite_marker = fixture.deferred_amend("one", "base");
    let bypass_marker = fixture
        .root()
        .join(".git")
        .join(format!("{PENDING_PREFIX}unreadable.json"));
    fs::write(&bypass_marker, r#"{"cause":{"kind":"environment_override","bypassed_merge":"unreadable-marker-test"},"occurrence_time":{"status":"unavailable"}}"#)
        .expect("legacy bypass marker should write");
    fs::set_permissions(&bypass_marker, fs::Permissions::from_mode(0o000))
        .expect("legacy marker should become unreadable");
    let unreadable = fs::read(&bypass_marker);
    let output = run_berth(fixture.root(), &["board", "--json"]);
    fs::set_permissions(&bypass_marker, fs::Permissions::from_mode(0o600))
        .expect("legacy marker permissions should restore");
    assert_eq!(
        unreadable.expect_err("fixture must deny reads").kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let observed = json_output(&output);
    assert!(!rewrite_marker.exists());
    assert!(bypass_marker.exists());
    let notice = observed["payload"]["data"]["alerts"]["entries"]
        .as_array()
        .expect("board should list alerts")
        .iter()
        .find(|alert| alert["kind"] == "unrecorded_bypasses")
        .expect("unreadable bypass must retain its recovery notice");
    assert_eq!(notice["count"], 1, "{observed}");
    assert_eq!(
        observed["payload"]["data"]["recovered_bypasses_this_invocation"],
        serde_json::json!([])
    );
    assert_eq!(
        observed["payload"]["data"]["bypass_audit"]["entries"],
        serde_json::json!([])
    );
    assert_latest_anchors(&fixture, &id, &fixture.head());
}

pub(super) fn transient_mapped_comparison_failure_preserves_the_rewrite_for_retry() {
    assert_transient_rewrite_failure_is_retryable(RewriteReadFailure::MappedComparison);
}

pub(super) fn transient_old_phase_read_failure_preserves_the_rewrite_for_retry() {
    assert_transient_rewrite_failure_is_retryable(RewriteReadFailure::OldPhaseRange);
}

pub(super) fn created_commit_read_failure_errors_capture_without_writing_a_marker() {
    assert_capture_read_failure_writes_no_marker(CaptureReadFailure::CreatedCommits);
}

pub(super) fn unreadable_rebase_onto_errors_capture_without_writing_a_marker() {
    assert_capture_read_failure_writes_no_marker(CaptureReadFailure::RebaseOnto);
}

/// The operation boundary read that must fail without persisting partial capture.
#[derive(Clone, Copy)]
enum CaptureReadFailure {
    CreatedCommits,
    RebaseOnto,
}

fn assert_capture_read_failure_writes_no_marker(failure: CaptureReadFailure) {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "two");
    let old_tip = fixture.head();
    fixture.checkpoint(&id);
    let editor = fixture.worktrees.path().join("capture-failure-editor");
    write_first_rebase_action(&editor, "edit");
    SplitWorkflow::InteractiveRebase.stop_before_split(&fixture, &editor);
    fixture.commit("one", "base");
    fixture.commit("one", "two");
    let mapped_tip = fixture.head();
    let wrapper_directory = tempdir().expect("Git wrapper directory should exist");
    let failure_log = wrapper_directory.path().join("failed-capture-read");
    write_capture_git_wrapper(wrapper_directory.path());
    let original_path = std::env::var_os("PATH").expect("PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(wrapper_directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let berth_wrapper = write_capture_berth_wrapper(wrapper_directory.path());
    let hook_failures = wrapper_directory.path().join("hook-failures");
    let administrative_directory = GIT.stdout(&fixture.holder, ["rev-parse", "--absolute-git-dir"]);
    let onto = Path::new(&administrative_directory).join("rebase-merge/onto");
    let failing_read = match failure {
        CaptureReadFailure::CreatedCommits => "created",
        CaptureReadFailure::RebaseOnto => "onto",
    };
    let continued = git_command(
        berth_wrapper
            .to_str()
            .expect("wrapper path should be UTF-8"),
    )
    .args(["rebase", "--continue"])
    .current_dir(&fixture.holder)
    .env("GIT_EDITOR", "true")
    .env("CARGO_BERTH_TEST_CAPTURE_PATH", wrapped_path)
    .env("CARGO_BERTH_TEST_REAL_BERTH", BERTH_EXECUTABLE)
    .env("CARGO_BERTH_TEST_HOOK_FAILURES", &hook_failures)
    .env("CARGO_BERTH_TEST_REAL_GIT", super::git_binary())
    .env("CARGO_BERTH_TEST_MAPPED_TIP", &mapped_tip)
    .env("CARGO_BERTH_TEST_REWRITE_FAILURE_LOG", &failure_log)
    .env("CARGO_BERTH_TEST_CAPTURE_FAILURE", failing_read)
    .env("CARGO_BERTH_TEST_REBASE_ONTO", &onto)
    .output()
    .expect("rebase continuation with failing capture should run");
    assert!(
        failure_log.is_file(),
        "the committed hook must attempt the failed read: {}",
        String::from_utf8_lossy(&continued.stderr)
    );
    assert!(
        fs::read_to_string(&hook_failures)
            .expect("capture must fail the hook invocation")
            .contains("committed")
    );
    assert!(
        continued.status.success(),
        "a committed hook cannot undo Git's completed rewrite"
    );
    let diagnostic = match failure {
        CaptureReadFailure::CreatedCommits => "injected created-commit capture read failure",
        CaptureReadFailure::RebaseOnto => "Permission denied",
    };
    assert!(
        String::from_utf8_lossy(&continued.stderr).contains(diagnostic),
        "capture must report the read error: {}",
        String::from_utf8_lossy(&continued.stderr)
    );
    assert!(
        pending_markers(fixture.root()).is_empty(),
        "failed capture must not store an empty created set"
    );
    assert_eq!(retained_tip(&fixture, &id), old_tip);
    assert!(
        !journal_events(fixture.root())
            .iter()
            .any(|event| { event["op"] == "resnapshot" && event["reservation_id"] == id })
    );
}

/// Fail only the operation-defined created range, including an explicit git directory.
fn write_capture_git_wrapper(directory: &Path) {
    let wrapper = directory.join("git");
    fs::write(
        &wrapper,
        r#"#!/bin/sh
case "$CARGO_BERTH_TEST_CAPTURE_FAILURE $* " in
    created*" rev-list --ignore-missing $CARGO_BERTH_TEST_MAPPED_TIP --not "*)
        printf '%s\n' "$*" >> "$CARGO_BERTH_TEST_REWRITE_FAILURE_LOG"
        printf '%s\n' 'injected created-commit capture read failure' >&2
        exit 128
        ;;
esac
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#,
    )
    .expect("capture Git wrapper should write");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("capture Git wrapper should execute");
}

/// Make the onto unreadable only to the hook, then restore it for Git's own reads.
fn write_capture_berth_wrapper(directory: &Path) -> PathBuf {
    // Git prepends its own exec directory to PATH before invoking hooks. Restore
    // the injector at the executable boundary while retaining the managed script.
    let berth_wrapper = directory.join("cargo-berth");
    fs::write(
        &berth_wrapper,
        r#"#!/bin/sh
PATH="$CARGO_BERTH_TEST_CAPTURE_PATH"
export PATH
deny_onto=false
case "$CARGO_BERTH_TEST_CAPTURE_FAILURE $*" in
    onto*committed*)
        chmod 000 "$CARGO_BERTH_TEST_REBASE_ONTO" || exit 98
        if [ -r "$CARGO_BERTH_TEST_REBASE_ONTO" ]; then exit 99; fi
        printf '%s\n' 'onto is unreadable' >> "$CARGO_BERTH_TEST_REWRITE_FAILURE_LOG"
        deny_onto=true
        ;;
esac
"$CARGO_BERTH_TEST_REAL_BERTH" "$@"
result=$?
if "$deny_onto"; then chmod 644 "$CARGO_BERTH_TEST_REBASE_ONTO"; fi
if [ "$result" -ne 0 ]; then
    printf '%s %s\n' "$result" "$*" >> "$CARGO_BERTH_TEST_HOOK_FAILURES"
fi
exit "$result"
"#,
    )
    .expect("managed executable wrapper should write");
    fs::set_permissions(&berth_wrapper, fs::Permissions::from_mode(0o755))
        .expect("managed executable wrapper should execute");
    berth_wrapper
}

/// The Git observation withheld during exactly one reconciliation invocation.
#[derive(Clone, Copy)]
enum RewriteReadFailure {
    MappedComparison,
    OldPhaseRange,
}

fn assert_transient_rewrite_failure_is_retryable(failure: RewriteReadFailure) {
    let fixture = RewriteFixture::new();
    let id = fixture.start_phase();
    fixture.commit("one", "base");
    fixture.checkpoint(&id);
    let old_tip = retained_tip(&fixture, &id);
    let marker = fixture.deferred_amend("one", "base");
    let wrapper_directory = tempdir().expect("Git wrapper directory should exist");
    let wrapper = wrapper_directory.path().join("git");
    let failure_log = wrapper_directory.path().join("failed-rewrite-read");
    fs::write(&wrapper, r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    fail_read=false
    case "$CARGO_BERTH_TEST_REWRITE_READ_FAILURE" in
        comparison)
            if [ "$2" = "merge-base" ] && [ "$3" != "--is-ancestor" ] && [ "$4" = "$CARGO_BERTH_TEST_MAPPED_TIP" ]; then
                fail_read=true
            fi
            ;;
        old_phase)
            if [ "$2" = "rev-list" ] && [ "$3" = "--reverse" ] && [ "$4" = "--topo-order" ] && [ "$5" = "$CARGO_BERTH_TEST_OLD_PHASE_RANGE" ]; then
                fail_read=true
            fi
            ;;
    esac
    if [ "$fail_read" = true ]; then
        printf '%s\n' "$*" >> "$CARGO_BERTH_TEST_REWRITE_FAILURE_LOG"
        printf '%s\n' 'injected transient rewrite read failure' >&2
        exit 128
    fi
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#).expect("Git wrapper should write");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("Git wrapper should execute");
    let original_path = std::env::var_os("PATH").expect("PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(wrapper_directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let old_start = GIT.stdout(fixture.root(), ["rev-parse", &format!("{old_tip}^")]);
    let failing_read = match failure {
        RewriteReadFailure::MappedComparison => "comparison",
        RewriteReadFailure::OldPhaseRange => "old_phase",
    };
    let failed = berth_command(BERTH_EXECUTABLE)
        .args(["board", "--json"])
        .current_dir(fixture.root())
        .env("PATH", wrapped_path)
        .env("CARGO_BERTH_TEST_REAL_GIT", super::git_binary())
        .env("CARGO_BERTH_TEST_MAPPED_TIP", fixture.head())
        .env(
            "CARGO_BERTH_TEST_OLD_PHASE_RANGE",
            format!("{old_start}..{old_tip}"),
        )
        .env("CARGO_BERTH_TEST_REWRITE_READ_FAILURE", failing_read)
        .env("CARGO_BERTH_TEST_REWRITE_FAILURE_LOG", &failure_log)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .env_remove("CARGO_BERTH_BYPASS")
        .output()
        .expect("board with failed rewrite read should run");
    assert!(
        failure_log.is_file(),
        "fixture must fail the selected rewrite read: {failing_read}"
    );
    assert!(
        failed.status.success(),
        "{} {}",
        String::from_utf8_lossy(&failed.stdout),
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(
        marker.exists(),
        "an unavailable comparison must remain retryable"
    );
    assert_eq!(retained_tip(&fixture, &id), old_tip);
    let pending: Value =
        serde_json::from_slice(&fs::read(&marker).expect("deferred marker should read"))
            .expect("deferred marker should decode");
    assert!(
        pending.get("completed_subjects").is_none()
            || pending["completed_subjects"] == serde_json::json!([]),
        "a failed comparison must not complete its subject: {pending}"
    );
    board(fixture.root());
    assert!(
        !marker.exists(),
        "successful retry must complete the marker"
    );
    assert_latest_anchors(&fixture, &id, &fixture.head());
}

fn board(root: &Path) -> Value {
    let output = run_berth(root, &["board", "--json"]);
    assert!(
        output.status.success(),
        "board failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    json_output(&output)
}

/// Count mapped comparison destinations independently of ordinary trunk evidence.
fn board_with_rewrite_trace(root: &Path) -> Value {
    let traces = tempdir().expect("git trace directory should exist");
    let trace_path = traces.path().join("git.trace");
    let output = berth_command(BERTH_EXECUTABLE)
        .args(["board", "--json"])
        .current_dir(root)
        .env("GIT_TRACE", &trace_path)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .env_remove("CARGO_BERTH_BYPASS")
        .output()
        .expect("traced board should run");
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let trunk = GIT.stdout(root, ["rev-parse", "main"]);
    let trace = fs::read_to_string(trace_path).expect("git trace should read");
    let mapped_comparisons = trace
        .lines()
        .filter_map(|line| {
            line.split_once("built-in: git merge-base ")
                .map(|(_, arguments)| arguments)
        })
        .filter(|arguments| {
            let arguments: Vec<_> = arguments.split_whitespace().collect();
            arguments.len() == 2 && arguments[0] != "--is-ancestor" && arguments[1] != trunk
        })
        .count();
    assert!(
        mapped_comparisons <= 1,
        "mapped tips must not admit additional cold subjects: {trace}"
    );
    json_output(&output)
}

fn snapshot<'board>(board: &'board Value, id: &str) -> &'board Value {
    let data = &board["payload"]["data"];
    ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .find(|entry| entry["reservation_id"] == id)
        .expect("checkpoint should remain visible on the board")
}

fn pending_markers(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root.join(".git"))
        .expect("common git directory should read")
        .map(|entry| {
            entry
                .expect("common git directory entry should read")
                .path()
        })
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PENDING_PREFIX))
        })
        .collect()
}

fn retained_tip(fixture: &RewriteFixture, id: &str) -> String {
    GIT.stdout(
        fixture.root(),
        ["rev-parse", &format!("{RETENTION_REF_PREFIX}{id}")],
    )
}

fn make_read_only(path: &Path) -> fs::Permissions {
    let permissions = fs::metadata(path)
        .expect("journal metadata should read")
        .permissions();
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(permissions.mode() & !0o222),
    )
    .expect("journal should become read-only");
    permissions
}

fn assert_latest_anchors(fixture: &RewriteFixture, id: &str, tip: &str) {
    let events = journal_events(fixture.root());
    let accepted = events.iter().rev().find(|event| {
        event["op"] == "resnapshot"
            && event["reservation_id"] == id
            && event["snapshot"]["stage"] == "outstanding"
    });
    let lifecycle_events: Vec<_> = events
        .iter()
        .filter(|event| {
            event["reservation_id"] == id
                && matches!(
                    event["op"].as_str(),
                    Some("claim" | "checkpoint" | "resnapshot" | "release")
                )
        })
        .collect();
    assert!(
        accepted.is_some(),
        "checkpoint {id} must have an accepted interval ending at {tip}; lifecycle records: {lifecycle_events:?}"
    );
    let accepted = accepted.expect("accepted interval should exist");
    assert_eq!(
        accepted["snapshot"]["protected_tip"], tip,
        "{lifecycle_events:?}"
    );
    let expected_start = GIT.stdout(fixture.root(), ["rev-parse", &format!("{tip}^")]);
    assert_eq!(
        accepted["snapshot"]["phase_start_head"], expected_start,
        "each checkpoint must retain its own interval: {accepted}"
    );
}

/// Released subjects may stop consuming maps once a full actual-trunk proof exists.
fn assert_final_checkpoint(fixture: &RewriteFixture, id: &str, tip: &str, trunk: &str) {
    let events = journal_events(fixture.root());
    let accepted = events
        .iter()
        .rev()
        .find(|event| {
            event["op"] == "resnapshot"
                && event["reservation_id"] == id
                && event["snapshot"]["stage"] == "outstanding"
        })
        .expect("each checkpoint should have accepted anchors before integration");
    if accepted["snapshot"]["protected_tip"] == tip {
        assert_latest_anchors(fixture, id, tip);
    } else {
        let release = events
            .iter()
            .find(|event| event["op"] == "release" && event["reservation_id"] == id)
            .expect("a checkpoint that stops mapping must already be settled");
        assert_eq!(
            release["disposition"],
            serde_json::json!({"kind": "rewritten_integration", "evidence": trunk})
        );
    }
}

/// Whether the drained checkpoints already sit on trunk and must settle as they are accepted.
#[derive(Clone, Copy, Eq, PartialEq)]
enum CheckpointPlacement {
    BesideTrunk,
    OnTrunk,
}

/// A cold pass may accept one distinct subject; deferred markers must make progress.
fn drain_markers(fixture: &RewriteFixture, ids: &[&str], placement: CheckpointPlacement) {
    for _ in 0..=ids.len() {
        let previous_events = journal_events(fixture.root()).len();
        let observed = board_with_rewrite_trace(fixture.root());
        let events = journal_events(fixture.root());
        let accepted_count = events[previous_events..]
            .iter()
            .filter(|event| {
                event["op"] == "resnapshot" && event["snapshot"]["stage"] == "outstanding"
            })
            .count();
        assert!(
            accepted_count <= 1,
            "distinct checkpoints must share the actual trunk acceptance budget: {events:?}"
        );
        if placement == CheckpointPlacement::OnTrunk {
            for id in ids {
                let events = journal_events(fixture.root());
                let accepted = events.iter().rev().find(|event| {
                    event["op"] == "resnapshot"
                        && event["reservation_id"] == *id
                        && event["snapshot"]["stage"] == "outstanding"
                });
                if let Some(accepted) = accepted {
                    let tip = accepted["snapshot"]["protected_tip"]
                        .as_str()
                        .expect("mapped tip should be text");
                    if GIT
                        .output(fixture.root(), ["merge-base", "--is-ancestor", tip, "main"])
                        .status
                        .success()
                    {
                        assert_eq!(
                            snapshot(&observed, id)["lifecycle"]["stage"],
                            "released",
                            "an accepted checkpoint already on trunk must settle in its accepting pass: {observed}"
                        );
                    }
                }
            }
        }
        if pending_markers(fixture.root()).is_empty() {
            if placement == CheckpointPlacement::OnTrunk {
                for id in ids {
                    assert_eq!(
                        snapshot(&observed, id)["lifecycle"]["stage"],
                        "released",
                        "{observed}"
                    );
                }
                let settled_journal = fs::read(fixture.root().join(JOURNAL_PATH))
                    .expect("settling journal should read");
                board(fixture.root());
                assert_eq!(
                    fs::read(fixture.root().join(JOURNAL_PATH)).expect("next journal should read"),
                    settled_journal,
                    "the pass immediately after settlement must append nothing"
                );
            }
            return;
        }
    }
    assert!(
        pending_markers(fixture.root()).is_empty(),
        "rewrite marker did not drain within one pass per subject"
    );
}
