//! Recorded rewrite histories and the repository operations used to build and exercise them.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::CommandDeadlineExt as _;
use cargo_berth_test_support::DirectorySnapshot;
use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::SECOND_RUN;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::git_command;
use cargo_berth_test_support::json as json_output;
use cargo_berth_test_support::reservation_row;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use split_rebase::write_first_rebase_action;
use tempfile::tempdir;

use super::split_rebase;
use crate::git_configuration;
use crate::linked_checkout;
use crate::loose_refs;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};
pub(super) const PENDING_PREFIX: &str = "cargo-berth-pending-bypass-";
const HUNK_SEPARATOR: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
pub(super) const STATE_PATH: &str = ".git/rewrite-fixture.json";

/// The empty-ledger main and holder checkout before any phase starts.
pub(super) const REWRITE_MAIN: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "rewrite-main",
    build:            build_rewrite_main,
};

/// An active phase containing both protected hunks, before either split workflow.
pub(super) const SPLIT_ACTIVE_PHASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "split-active-phase",
        build: |root| build_split_phase(root, SplitReservationStage::Active),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// The same protected interval, checkpointed before either split workflow.
pub(super) const SPLIT_OUTSTANDING_PHASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "split-outstanding-phase",
        build: |root| build_split_phase(root, SplitReservationStage::Outstanding),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// The reservation and original tip shared by both split workflows.
#[derive(Serialize, Deserialize)]
pub(super) struct SplitPhaseState {
    pub(super) id:      String,
    pub(super) old_tip: String,
}

fn build_split_phase(root: &Path, stage: SplitReservationStage) {
    REWRITE_MAIN.restore_into(root);
    let fixture = RewriteRepository::at_root(root);
    let id = fixture.start_phase();
    fixture.commit("one", "two");
    let old_tip = fixture.head();
    if matches!(stage, SplitReservationStage::Outstanding) {
        fixture.checkpoint(&id);
    }
    write_state(root, &SplitPhaseState { id, old_tip });
}

/// One protected hunk, checkpointed before a later rewrite starts.
pub(super) const ONE_HUNK_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "one-hunk-checkpoint",
        build: |root| {
            REWRITE_MAIN.restore_into(root);
            let fixture = RewriteRepository::at_root(root);
            let id = fixture.start_checkpoint("one", "base");
            write_state(
                root,
                &SplitPhaseState {
                    id,
                    old_tip: fixture.head(),
                },
            );
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// Two holder commits and conflicting trunk work before a live rebase starts.
pub(super) const ABORTABLE_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "abortable-checkpoint",
        build: |root| {
            ONE_HUNK_CHECKPOINT.restore_into(root);
            let fixture = RewriteRepository::at_root(root);
            let mut state: SplitPhaseState = serde_json::from_slice(
                &fs::read(root.join(STATE_PATH)).expect("checkpoint state should read"),
            )
            .expect("checkpoint state should decode");
            fixture.commit("one", "two");
            state.old_tip = fixture.head();
            fixture.upstream_conflict();
            write_state(root, &state);
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A two-hunk checkpoint before a deferred amend and conflicting rebase.
pub(super) const CONFLICTING_TWO_HUNK_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "conflicting-two-hunk-checkpoint",
        build: |root| {
            SPLIT_OUTSTANDING_PHASE.restore_into(root);
            RewriteRepository::at_root(root).upstream_conflict();
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

#[derive(Serialize, Deserialize)]
pub(super) struct ApplyCheckpointState {
    pub(super) id:        String,
    pub(super) old_start: String,
    pub(super) first_tip: String,
    pub(super) old_tip:   String,
    pub(super) trunk:     String,
}

/// An apply checkpoint before its uninterrupted rebase starts.
pub(super) const APPLY_UNINTERRUPTED_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "apply-uninterrupted-checkpoint",
        build: |root| build_apply_checkpoint(root, ApplyPreparation::Uninterrupted),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// An apply checkpoint with an unreserved conflict path.
pub(super) const APPLY_CONFLICTED_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "apply-conflicted-checkpoint",
        build: |root| build_apply_checkpoint(root, ApplyPreparation::Conflicted),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// Two protected commits before a side branch changes during a stopped apply rebase.
pub(super) const APPLY_SIDE_BRANCH_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "apply-side-branch-checkpoint",
        build: |root| build_apply_checkpoint(root, ApplyPreparation::SideBranch),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

#[derive(Clone, Copy)]
enum ApplyPreparation {
    Uninterrupted,
    Conflicted,
    SideBranch,
}

#[derive(Serialize, Deserialize)]
pub(super) struct OtherReservationState {
    pub(super) id:        String,
    pub(super) other_id:  String,
    pub(super) other_tip: String,
    pub(super) old_tip:   String,
    pub(super) trunk:     String,
}

/// Active holder work before a rebase onto another reservation.
pub(super) const ACTIVE_BEFORE_OTHER_REBASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-before-other-rebase",
        build: |root| build_before_other_rebase(root, SplitReservationStage::Active),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// An outstanding checkpoint before a rebase onto another reservation.
pub(super) const CHECKPOINT_BEFORE_OTHER_REBASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-before-other-rebase",
        build: |root| build_before_other_rebase(root, SplitReservationStage::Outstanding),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

fn build_before_other_rebase(root: &Path, stage: SplitReservationStage) {
    REWRITE_MAIN.restore_into(root);
    let fixture = RewriteRepository::at_root(root);
    let trunk = GIT.stdout(root, ["rev-parse", "main"]);
    let parent = root.join(".git/fixture-worktrees");
    fs::create_dir_all(&parent).expect("worktree parent should exist");
    let other = parent.join("other-reservation");
    GIT.add_worktree_without_hooks(root, &other, "other-reservation", "HEAD");
    let other_claim = claim(&other, "file:other.txt", SECOND_RUN);
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
    write_state(
        root,
        &OtherReservationState {
            id,
            other_id,
            other_tip,
            old_tip,
            trunk,
        },
    );
}

fn build_apply_checkpoint(root: &Path, preparation: ApplyPreparation) {
    REWRITE_MAIN.restore_into(root);
    let fixture = RewriteRepository::at_root(root);
    let old_start = fixture.head();
    let id = fixture.start_phase();
    if !matches!(preparation, ApplyPreparation::Conflicted) {
        fixture.commit("one", "base");
    }
    let first_tip = fixture.head();
    if !matches!(preparation, ApplyPreparation::Uninterrupted) {
        let later = if matches!(preparation, ApplyPreparation::SideBranch) {
            "two"
        } else {
            "base"
        };
        fs::write(fixture.holder.join("phase.txt"), contents("one", later))
            .expect("protected source should write");
        fs::write(fixture.holder.join("unreserved.txt"), "holder addition\n")
            .expect("unreserved conflict should write");
        GIT.run(&fixture.holder, ["add", "phase.txt", "unreserved.txt"]);
        let tree = GIT.stdout(&fixture.holder, ["write-tree"]);
        let tip = GIT.stdout(
            &fixture.holder,
            ["commit-tree", &tree, "-p", &first_tip, "-m", "holder phase"],
        );
        GIT.run(
            &fixture.holder,
            ["update-ref", "refs/heads/holder", &tip, &first_tip],
        );
    }
    let old_tip = fixture.head();
    fixture.checkpoint(&id);
    assert!(
        !journal_events(root)
            .iter()
            .any(|event| event["op"] == "widen" && event["reservation_id"] == id)
    );
    commit_file(
        root,
        "unreserved.txt",
        "trunk addition\n",
        "advance trunk outside the reserved scope",
    );
    let trunk = GIT.stdout(root, ["rev-parse", "main"]);
    assert_ne!(trunk, old_start);
    assert_eq!(pending_markers(root), [] as [PathBuf; 0]);
    write_state(
        root,
        &ApplyCheckpointState {
            id,
            old_start,
            first_tip,
            old_tip,
            trunk,
        },
    );
}

/// Unreserved holder work before a live rebase and pruning remove its old commit.
pub(super) const UNRESERVED_REBASE_PREFIX: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "unreserved-rebase-prefix",
        build: |root| {
            REWRITE_MAIN.restore_into(root);
            let fixture = RewriteRepository::at_root(root);
            fixture.commit("unreserved", "base");
            let old_tip = fixture.head();
            commit_file(root, "upstream.txt", "upstream\n", "advance trunk");
            write_state(root, &old_tip);
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// Two outstanding checkpoints after their second real rebase, with pending markers.
pub(super) const TWO_REBASE_HISTORY: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "two-rebase-history",
        build: build_two_rebase_history,
        ..REWRITE_MAIN
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// One upper checkpoint spanning both commits, with its lower sibling ref.
pub(super) const STACKED_UPPER_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "stacked-upper-checkpoint",
        build: build_stacked_upper_checkpoint,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A lower checkpoint and the active upper phase before a split changes retention.
pub(super) const SPLIT_LOWER_CHECKPOINT_ACTIVE_RETENTION: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "split-lower-checkpoint-active-retention",
        build: build_active_retention,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A lower checkpoint and active upper phase with a sibling ref updated by rebase.
pub(super) const SPLIT_LOWER_CHECKPOINT_UPDATED_SIBLING: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "split-lower-checkpoint-updated-sibling",
        build: build_updated_sibling,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A lower checkpoint before the issuing checkout changes branches during the split.
pub(super) const SPLIT_LOWER_CHECKPOINT_SWITCHED_CHECKOUT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "split-lower-checkpoint-switched-checkout",
        build: build_switched_checkout,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

fn build_switched_checkout(root: &Path) {
    build_split_phase(root, SplitReservationStage::Outstanding);
    let state: SplitPhaseState = serde_json::from_slice(
        &fs::read(root.join(STATE_PATH)).expect("split phase state should read"),
    )
    .expect("split phase state should decode");
    write_state(
        root,
        &SplitLowerCheckpointState {
            trunk:         GIT.stdout(root, ["rev-parse", "main"]),
            lower:         state.id,
            old_upper:     state.old_tip.clone(),
            old_lower:     state.old_tip,
            active_phases: Vec::new(),
        },
    );
}

/// Reservation identities and final interval tips left by the two-rebase prefix.
#[derive(Serialize, Deserialize)]
pub(super) struct TwoRebaseHistoryState {
    pub(super) first:       String,
    pub(super) second:      String,
    pub(super) earlier_tip: String,
    pub(super) final_tip:   String,
}

/// The upper checkpoint and both original branch tips before an interactive split.
#[derive(Serialize, Deserialize)]
pub(super) struct StackedUpperCheckpointState {
    pub(super) trunk:     String,
    pub(super) upper:     String,
    pub(super) old_lower: String,
    pub(super) old_upper: String,
}

/// The lower checkpoint, active phases and original tips before the split rebase.
#[derive(Serialize, Deserialize)]
pub(super) struct SplitLowerCheckpointState {
    pub(super) trunk:         String,
    pub(super) lower:         String,
    pub(super) old_lower:     String,
    pub(super) old_upper:     String,
    pub(super) active_phases: Vec<String>,
}

fn write_state(root: &Path, state: &impl Serialize) {
    fs::write(
        root.join(STATE_PATH),
        serde_json::to_vec(state).expect("rewrite state should encode"),
    )
    .expect("rewrite state should write");
}

fn build_rewrite_main(root: &Path) {
    GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
    git_configuration::append_git_configuration(root, "\n[user]\n name = Berth Test\n email = berth@example.invalid\n[maintenance]\n auto = false\n[gc]\n auto = 0\n").expect("git configuration should write");
    fs::write(root.join("README.md"), "scratch repository\n").expect("scratch file should write");
    GIT.run(root, ["add", "README.md"]);
    GIT.run(root, ["commit", "--quiet", "-m", "initial"]);
    GIT.run(root, ["tag", "initial-state"]);
    assert!(run_berth(root, &["init"]).status.success());
    fs::write(root.join("phase.txt"), contents("base", "base")).expect("phase base should write");
    GIT.run(
        root,
        ["add", "--force", "phase.txt", ".claude/config/berth.toml"],
    );
    // The unreserved-trunk gate regression covers this source and policy baseline.
    // Rewrites after a phase starts keep every capture and drift hook.
    GIT.run_without_hooks(
        root,
        [
            "commit",
            "--quiet",
            "-m",
            "phase base and policy configuration",
        ],
    );
    GIT.add_worktree_without_hooks(root, &root.join(".git/fixture-holder"), "holder", "HEAD");
}

/// An initialized main and linked holder, independent of who owns their directory.
pub(super) struct RewriteRepository {
    root:              PathBuf,
    pub(super) holder: PathBuf,
}

impl RewriteRepository {
    pub(super) fn at_root(root: &Path) -> Self {
        let holder = root.join(".git/fixture-holder");
        assert!(root.join(".git/hooks/reference-transaction").is_file());
        assert_ne!(
            linked_checkout::administrative_directory(&holder),
            root.join(".git")
        );
        Self {
            root: root.to_owned(),
            holder,
        }
    }

    pub(super) fn root(&self) -> &Path { &self.root }
    pub(super) fn head(&self) -> String {
        // This fixture creates loose refs. Reading its own HEAD avoids spawning
        // Git merely to remember the tip before the next real rewrite command.
        let administrative_directory = linked_checkout::administrative_directory(&self.holder);
        let head = fs::read_to_string(administrative_directory.join("HEAD"))
            .expect("holder HEAD should read");
        head.trim().strip_prefix("ref: ").map_or_else(
            || head.trim().to_owned(),
            |reference| loose_refs::read_loose_ref(self.root(), reference),
        )
    }

    pub(super) fn start_phase(&self) -> String {
        let claimed = claim(&self.holder, "file:phase.txt", FIRST_RUN);
        assert!(
            claimed.status.success(),
            "{}",
            String::from_utf8_lossy(&claimed.stdout)
        );
        reservation_id(&claimed)
    }

    pub(super) fn start_checkpoint(&self, first: &str, later: &str) -> String {
        let id = self.start_phase();
        self.commit(first, later);
        self.checkpoint(&id);
        id
    }

    pub(super) fn commit(&self, first: &str, later: &str) {
        fs::write(self.holder.join("phase.txt"), contents(first, later))
            .expect("tracked phase should write");
        GIT.run(
            &self.holder,
            [
                "commit",
                "--quiet",
                "--only",
                "-m",
                "holder phase",
                "--",
                "phase.txt",
            ],
        );
    }

    pub(super) fn checkpoint(&self, id: &str) {
        let checkpoint = run_berth(&self.holder, &["release", id, "--json"]);
        assert!(
            checkpoint.status.success(),
            "{}",
            String::from_utf8_lossy(&checkpoint.stdout)
        );
        assert_eq!(json_output(&checkpoint)["status"], "outstanding");
    }

    pub(super) fn upstream_conflict(&self) {
        fs::write(self.root().join("phase.txt"), contents("base", "upstream"))
            .expect("tracked upstream phase should write");
        GIT.run(
            self.root(),
            [
                "commit",
                "--quiet",
                "--only",
                "-m",
                "upstream changes later hunk",
                "--",
                "phase.txt",
            ],
        );
    }

    pub(super) fn stop_rebase(&self) {
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

    pub(super) fn resolve_rebase(&self) {
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
            .bounded_output()
            .expect("rebase continuation should run");
        assert!(
            continued.status.success(),
            "{}",
            String::from_utf8_lossy(&continued.stderr)
        );
    }
}

fn build_two_rebase_history(root: &Path) {
    REWRITE_MAIN.restore_into(root);
    let fixture = RewriteRepository::at_root(root);
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
    write_state(
        root,
        &TwoRebaseHistoryState {
            first,
            second,
            earlier_tip,
            final_tip,
        },
    );
}

/// Whether the drained checkpoints already sit on trunk and must settle as they are accepted.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CheckpointPlacement {
    BesideTrunk,
    OnTrunk,
}

pub(super) fn contents(first: &str, later: &str) -> String {
    format!("first={first}\n{HUNK_SEPARATOR}later={later}\n")
}

pub(super) fn board(root: &Path) -> Value {
    let output = run_berth(root, &["board", "--json"]);
    assert!(
        output.status.success(),
        "board failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    json_output(&output)
}

pub(super) fn board_with_rewrite_trace(root: &Path) -> Value {
    let traces = tempdir().expect("git trace directory should exist");
    let trace_path = traces.path().join("git.trace");
    let output = berth_command(BERTH_EXECUTABLE)
        .args(["board", "--json"])
        .current_dir(root)
        .env("GIT_TRACE", &trace_path)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .env_remove("CARGO_BERTH_BYPASS")
        .bounded_output()
        .expect("traced board should run");
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let trunk = loose_refs::read_loose_ref(root, "refs/heads/main");
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

pub(super) fn snapshot<'board>(board: &'board Value, id: &str) -> &'board Value {
    reservation_row(board, id)
}

pub(super) fn pending_markers(root: &Path) -> Vec<PathBuf> {
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

pub(super) fn assert_latest_anchors(fixture: &RewriteRepository, id: &str, tip: &str) {
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

pub(super) fn drain_markers(
    fixture: &RewriteRepository,
    ids: &[&str],
    placement: CheckpointPlacement,
) {
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

fn run_berth(root: &Path, arguments: &[&str]) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .bounded_output()
        .expect("cargo-berth should run")
}

fn claim(root: &Path, scope: &str, run: &str) -> Output {
    run_berth(root, &["claim", scope, "--run", run, "--json"])
}

fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should name its reservation")
        .to_owned()
}

fn journal_events(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .collect()
}

fn commit_file(root: &Path, path: &str, content: &str, message: &str) {
    fs::write(root.join(path), content).expect("committed file should write");
    GIT.run(root, ["add", path]);
    GIT.run(root, ["commit", "--quiet", "-m", message]);
}

fn build_stacked_upper_checkpoint(root: &Path) {
    REWRITE_MAIN.restore_into(root);
    let fixture = RewriteRepository::at_root(root);
    let trunk = fixture.head();
    let upper = fixture.start_phase();
    fixture.commit("one", "two");
    let old_lower = fixture.head();
    GIT.run(&fixture.holder, ["branch", "lower"]);
    fixture.commit("one", "three");
    let old_upper = fixture.head();
    fixture.checkpoint(&upper);
    write_state(
        root,
        &StackedUpperCheckpointState {
            trunk,
            upper,
            old_lower,
            old_upper,
        },
    );
}

fn build_active_retention(root: &Path) {
    let (fixture, mut state) = build_lower_checkpoint(root);
    state.active_phases.push(fixture.start_phase());
    fixture.commit("one", "three");
    state.old_upper = fixture.head();
    write_state(root, &state);
}

fn build_updated_sibling(root: &Path) {
    let (fixture, mut state) = build_lower_checkpoint(root);
    GIT.run(&fixture.holder, ["branch", "lower"]);
    state.active_phases.push(fixture.start_phase());
    fixture.commit("one", "three");
    state.old_upper = fixture.head();
    write_state(root, &state);
}

fn build_lower_checkpoint(root: &Path) -> (RewriteRepository, SplitLowerCheckpointState) {
    REWRITE_MAIN.restore_into(root);
    let fixture = RewriteRepository::at_root(root);
    let trunk = fixture.head();
    let lower = fixture.start_phase();
    fixture.commit("one", "two");
    let old_lower = fixture.head();
    fixture.checkpoint(&lower);
    let state = SplitLowerCheckpointState {
        trunk,
        lower,
        old_upper: old_lower.clone(),
        old_lower,
        active_phases: Vec::new(),
    };
    (fixture, state)
}

/// The reservation state whose anchors the real rewrite must preserve.
#[derive(Clone, Copy)]
pub(super) enum SplitReservationStage {
    Active,
    Outstanding,
}

/// Ref changes that must not change the commits attributed to a finished rewrite.
#[derive(Clone, Copy)]
pub enum SplitRefChange {
    /// Rewrite the retention ref of a still-active upper phase.
    ActiveRetention,
    /// Rewrite the lower sibling branch through Git update-refs.
    UpdatedSibling,
    /// Switch the issuing checkout before reconciliation imports the split.
    SwitchedCheckout,
}

/// Stop the real rebase at its lower commit, preserving Git's update-ref todo entries.
pub(super) fn stop_stacked_split_rebase(
    fixture: &RewriteRepository,
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
fn switch_before_split_import(fixture: &RewriteRepository, markers: &[PathBuf], trunk: &str) {
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
pub(super) fn assert_split_capture(
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
    fixture: &RewriteRepository,
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

pub(super) fn assert_split_anchors(
    fixture: &RewriteRepository,
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

pub(super) fn make_read_only(path: &Path) -> fs::Permissions {
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

/// Exercise the real split, capture and reconciliation before a later dropped hunk.
pub(super) fn prepare_split_checkpoint(
    fixture: &RewriteRepository,
    state: SplitLowerCheckpointState,
    editor: &Path,
    change: SplitRefChange,
) -> DrainedSplitCheckpointState {
    let SplitLowerCheckpointState {
        trunk,
        lower,
        old_lower,
        old_upper,
        mut active_phases,
    } = state;
    if matches!(change, SplitRefChange::UpdatedSibling) {
        split_rebase::stop_for_split(&fixture.holder, editor, &trunk);
    } else {
        write_first_rebase_action(editor, "edit");
        stop_stacked_split_rebase(fixture, change, editor, &trunk);
    }
    fixture.commit("one", "base");
    let first_split = fixture.head();
    fixture.commit("one", "two");
    let split_tip = fixture.head();
    split_rebase::continue_rebase(&fixture.holder);
    let new_upper = fixture.head();
    assert_ne!(old_upper, new_upper);
    let markers = pending_markers(fixture.root());
    assert!(
        !markers.is_empty(),
        "the real committed hook must retain its rewrite map"
    );
    if matches!(change, SplitRefChange::UpdatedSibling) {
        assert_eq!(
            loose_refs::read_loose_ref(fixture.root(), "refs/heads/lower"),
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
        switch_before_split_import(fixture, &markers, &trunk);
    }
    for upper in &active_phases {
        assert_split_anchors(
            fixture,
            upper,
            &split_tip,
            &new_upper,
            &first_split,
            SplitReservationStage::Active,
        );
    }
    if matches!(change, SplitRefChange::ActiveRetention) {
        checkpoint_upper_before_lower_import(
            fixture,
            &markers,
            std::mem::take(&mut active_phases),
            &new_upper,
        );
        assert_eq!(retained_tip(fixture, &lower), old_lower);
    }
    drain_markers(fixture, &[&lower], CheckpointPlacement::BesideTrunk);
    assert_split_anchors(
        fixture,
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
    DrainedSplitCheckpointState {
        trunk,
        checkpoint: lower,
        protected_tip: split_tip,
        first_split,
    }
}

/// A reconciled split checkpoint whose protected first hunk has not yet been dropped.
#[derive(Serialize, Deserialize)]
pub(super) struct DrainedSplitCheckpointState {
    pub(super) trunk:         String,
    pub(super) checkpoint:    String,
    pub(super) protected_tip: String,
    pub(super) first_split:   String,
}

/// A reconciled lower checkpoint after the upper retention ref was rewritten.
pub(super) const DRAINED_SPLIT_RETENTION_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "drained-split-retention-checkpoint",
        build: build_drained_retention_checkpoint,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A reconciled lower checkpoint after rebase updated its sibling branch.
pub(super) const DRAINED_SPLIT_SIBLING_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "drained-split-sibling-checkpoint",
        build: build_drained_sibling_checkpoint,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

fn build_drained_retention_checkpoint(root: &Path) {
    build_active_retention(root);
    build_drained_checkpoint(root, SplitRefChange::ActiveRetention);
}

fn build_drained_sibling_checkpoint(root: &Path) {
    build_updated_sibling(root);
    build_drained_checkpoint(root, SplitRefChange::UpdatedSibling);
}

/// A reconciled checkpoint after its issuing checkout switched branches.
pub(super) const DRAINED_SPLIT_SWITCHED_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "drained-split-switched-checkpoint",
        build: |root| {
            build_switched_checkout(root);
            build_drained_checkpoint(root, SplitRefChange::SwitchedCheckout);
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

fn build_drained_checkpoint(root: &Path, change: SplitRefChange) {
    let fixture = RewriteRepository::at_root(root);
    let state =
        serde_json::from_slice(&fs::read(root.join(STATE_PATH)).expect("split state should read"))
            .expect("split state should decode");
    let editor_directory = cargo_berth_test_support::fixture_tempdir("split-editor-");
    let state = prepare_split_checkpoint(
        &fixture,
        state,
        &editor_directory.path().join("split-editor"),
        change,
    );
    write_state(root, &state);
}

fn retained_tip(fixture: &RewriteRepository, id: &str) -> String {
    loose_refs::read_loose_ref(
        fixture.root(),
        &format!("refs/cargo-berth/reservations/{id}"),
    )
}

/// The interval witnesses after the first real split.
#[derive(Serialize, Deserialize)]
pub(super) struct CompletedSplitState {
    pub(super) id:              String,
    pub(super) old_tip:         String,
    pub(super) first_split_tip: String,
    pub(super) expected_start:  String,
    pub(super) split_tip:       String,
}

/// The upper reservation and branch witnesses after the first stacked split.
#[derive(Serialize, Deserialize)]
pub(super) struct CompletedStackedSplitState {
    pub(super) trunk:       String,
    pub(super) upper:       String,
    pub(super) old_upper:   String,
    pub(super) first_split: String,
    pub(super) split_tip:   String,
    pub(super) new_upper:   String,
}

/// An active reservation after its first real interactive split.
pub(super) const ACTIVE_FIRST_SPLIT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-first-split",
        build: build_active_first_split,
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};
/// An upper checkpoint after Git writes the upper branch first.
pub(super) const UPPER_FIRST_SPLIT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "upper-first-split",
        build: |root| build_first_stacked_split(root, StackedBranchWriteOrder::UpperFirst),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};
/// An upper checkpoint after Git writes the lower branch first.
pub(super) const LOWER_FIRST_SPLIT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "lower-first-split",
        build: |root| build_first_stacked_split(root, StackedBranchWriteOrder::LowerFirst),
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

fn build_active_first_split(root: &Path) {
    SPLIT_ACTIVE_PHASE.restore_into(root);
    let state = prepare_first_split(
        root,
        SplitWorkflow::InteractiveRebase,
        SplitReservationStage::Active,
    );
    write_state(root, &state);
}

/// A reconciled interactive split before a second rebase drops a protected hunk.
pub(super) const RECONCILED_INTERACTIVE_CHECKPOINT_SPLIT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "reconciled-interactive-checkpoint-split",
        build: |root| {
            build_reconciled_split(
                root,
                SplitWorkflow::InteractiveRebase,
                SplitReservationStage::Outstanding,
            );
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A reconciled soft reset split from an active reservation.
pub(super) const RECONCILED_SOFT_ACTIVE_SPLIT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "reconciled-soft-active-split",
        build: |root| {
            build_reconciled_split(
                root,
                SplitWorkflow::SoftReset,
                SplitReservationStage::Active,
            );
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

/// A reconciled soft reset split from an outstanding checkpoint.
pub(super) const RECONCILED_SOFT_CHECKPOINT_SPLIT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "reconciled-soft-checkpoint-split",
        build: |root| {
            build_reconciled_split(
                root,
                SplitWorkflow::SoftReset,
                SplitReservationStage::Outstanding,
            );
        },
        ..REWRITE_MAIN
    },
    ..TWO_REBASE_HISTORY
};

fn build_reconciled_split(root: &Path, workflow: SplitWorkflow, stage: SplitReservationStage) {
    match stage {
        SplitReservationStage::Active => SPLIT_ACTIVE_PHASE.restore_into(root),
        SplitReservationStage::Outstanding => SPLIT_OUTSTANDING_PHASE.restore_into(root),
    }
    let interval = prepare_first_split(root, workflow, stage);
    reconcile_first_split(root, &interval, stage);
    write_state(root, &interval);
}

fn build_first_stacked_split(root: &Path, order: StackedBranchWriteOrder) {
    STACKED_UPPER_CHECKPOINT.restore_into(root);
    let state = prepare_first_stacked_split(root, order);
    write_state(root, &state);
}

/// Both Git workflows nominate only the last commit of the two-commit split.
#[derive(Clone, Copy)]
pub enum SplitWorkflow {
    /// Split through the real interactive rebase hooks.
    InteractiveRebase,
    /// Split through a soft reset and a final branch update.
    SoftReset,
}

impl SplitWorkflow {
    pub(super) fn stop_before_split(self, fixture: &RewriteRepository, editor: &Path) {
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

    pub(super) fn complete_split(
        self,
        fixture: &RewriteRepository,
        old_tip: &str,
        split_tip: &str,
    ) {
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

/// Branch writes within one rebase must observe the same operation boundary.
#[derive(Clone, Copy)]
pub(super) enum StackedBranchWriteOrder {
    UpperFirst,
    LowerFirst,
}

/// Move lower after the full map is flushed by exec, before Git writes upper.
fn write_stacked_rebase_editor(editor: &Path, order: StackedBranchWriteOrder, old_lower: &str) {
    if matches!(order, StackedBranchWriteOrder::UpperFirst) {
        split_rebase::write_first_rebase_action(editor, "edit");
        return;
    }
    fs::write(editor.with_extension("lower"), old_lower)
        .expect("lower-first editor's private reference should write");
    DirectorySnapshot::write_shared_shell_script(
        editor,
        r#"#!/bin/sh
IFS= read -r old_lower < "$0.lower" || [ -n "$old_lower" ] || exit 1
sed -e '1s/^pick /edit /' -e '/^update-ref refs\/heads\/lower$/d' "$1" > "$1.edited"
printf '%s\n' "exec git update-ref refs/heads/lower HEAD^ $old_lower" >> "$1.edited"
mv "$1.edited" "$1"
"#,
    );
}

/// Run the first real split and retain its interval witnesses.
pub(super) fn prepare_first_split(
    root: &Path,
    workflow: SplitWorkflow,
    stage: SplitReservationStage,
) -> CompletedSplitState {
    let fixture = RewriteRepository::at_root(root);
    let editor_directory = cargo_berth_test_support::fixture_tempdir("first-split-editor-");
    let SplitPhaseState { id, old_tip } = serde_json::from_slice(
        &fs::read(fixture.root().join(STATE_PATH)).expect("split state should read"),
    )
    .expect("split state should decode");
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
    let editor = editor_directory.path().join("sequence-editor");
    split_rebase::write_first_rebase_action(&editor, "edit");
    workflow.stop_before_split(&fixture, &editor);
    fixture.commit("one", "base");
    let first_split_tip = fixture.head();
    let expected_start = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD^"]);
    fixture.commit("one", "two");
    let split_tip = fixture.head();
    workflow.complete_split(&fixture, &old_tip, &split_tip);
    CompletedSplitState {
        id,
        old_tip,
        first_split_tip,
        expected_start,
        split_tip,
    }
}

fn prepare_first_stacked_split(
    root: &Path,
    order: StackedBranchWriteOrder,
) -> CompletedStackedSplitState {
    let fixture = RewriteRepository::at_root(root);
    let editor_directory = cargo_berth_test_support::fixture_tempdir("stacked-split-editor-");
    let StackedUpperCheckpointState {
        trunk,
        upper,
        old_lower,
        old_upper,
    } = serde_json::from_slice(
        &fs::read(fixture.root().join(STATE_PATH)).expect("stacked state should read"),
    )
    .expect("stacked state should decode");
    let editor = editor_directory.path().join("stacked-order-editor");
    write_stacked_rebase_editor(&editor, order, &old_lower);
    stop_stacked_split_rebase(&fixture, SplitRefChange::UpdatedSibling, &editor, &trunk);
    fixture.commit("one", "base");
    let first_split = fixture.head();
    fixture.commit("one", "two");
    let split_tip = fixture.head();
    SplitWorkflow::InteractiveRebase.complete_split(&fixture, &old_lower, &split_tip);
    let new_upper = fixture.head();
    CompletedStackedSplitState {
        trunk,
        upper,
        old_upper,
        first_split,
        split_tip,
        new_upper,
    }
}

/// Reconcile the split and check its anchors before a later rewrite.
pub(super) fn reconcile_first_split(
    root: &Path,
    interval: &CompletedSplitState,
    stage: SplitReservationStage,
) {
    let fixture = RewriteRepository::at_root(root);
    drain_markers(&fixture, &[&interval.id], CheckpointPlacement::BesideTrunk);
    assert_split_anchors(
        &fixture,
        &interval.id,
        &interval.expected_start,
        &interval.split_tip,
        &interval.first_split_tip,
        stage,
    );
    if matches!(stage, SplitReservationStage::Active) {
        fixture.checkpoint(&interval.id);
    }
}
