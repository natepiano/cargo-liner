//! Hook-generated pending rewrites, recorded before the gate test clocks start.
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::deferring_run_scope;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &["CARGO_BERTH_BYPASS"],
};
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const PENDING_BYPASS_PREFIX: &str = "cargo-berth-pending-bypass-";

/// An ordered checkpoint whose real rebase hook left a pending marker.
pub const PENDING_ORDERED_REWRITE: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       BERTH_EXECUTABLE,
        name:             "pending-ordered-rewrite",
        build:            build_pending_ordered_rewrite,
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// Two real hook markers, with the ordered checkpoint second in importer order.
pub const TWO_PENDING_ORDERED_REWRITES: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "two-pending-ordered-rewrites",
        build: build_two_pending_ordered_rewrites,
        ..PENDING_ORDERED_REWRITE.repository
    },
    ..PENDING_ORDERED_REWRITE
};

struct PendingPair<'root> {
    repository:   &'root Path,
    worktrees:    PathBuf,
    blocked_root: PathBuf,
    blocked_id:   String,
    holder_id:    String,
}

fn save_pair(pair: &PendingPair<'_>) {
    fs::write(
        pair.repository.join(".git/pending-ordered-pair.json"),
        serde_json::to_vec(
            &serde_json::json!({"blocked_id":pair.blocked_id,"holder_id":pair.holder_id}),
        )
        .expect("pair identities should encode"),
    )
    .expect("pair identities should write");
}

fn build_pending_ordered_rewrite(root: &Path) {
    let pair = build_pending_pair(root);
    save_pair(&pair);
}

fn build_pending_pair(root: &Path) -> PendingPair<'_> {
    let fixture = deferred_pair(root);
    let root = fixture.repository;
    let sequenced = run_berth(
        root,
        &[
            "sequence",
            &fixture.holder_id,
            &fixture.blocked_id,
            "--why",
            "predecessor must integrate before the rebased checkpoint",
            "--json",
        ],
    );
    assert!(sequenced.status.success(), "{}", json_output(&sequenced));
    let old_tip = commit_work(
        &fixture.blocked_root,
        "src/lib.rs",
        "pub fn rebased_successor() {}\n",
        "checkpoint successor before rebase",
    );
    let checkpoint = run_berth(
        &fixture.blocked_root,
        &["release", &fixture.blocked_id, "--json"],
    );
    assert!(checkpoint.status.success(), "{}", json_output(&checkpoint));
    assert_eq!(
        json_output(&checkpoint)["payload"]["data"]["status"],
        "checkpointed"
    );
    commit_work(root, "upstream.txt", "upstream advance\n", "advance trunk");
    set_gate_mode(root, "enforce");
    git(&fixture.blocked_root, &["rebase", "main"]);
    set_gate_mode(&fixture.blocked_root, "enforce");
    let rebased_tip = git_stdout(&fixture.blocked_root, &["rev-parse", "HEAD"]);
    assert_ne!(rebased_tip, old_tip);
    assert_eq!(pending_bypass_count(root), 1);
    let marker = pending_bypass_marker(root);
    assert_eq!(marker["kind"], "branch_rewrite");
    assert_eq!(
        marker["pairs"],
        serde_json::json!([{"old": old_tip, "new": rebased_tip}])
    );

    fixture
}

fn build_two_pending_ordered_rewrites(root: &Path) {
    let pair = build_pending_pair(root);
    let root = pair.repository;
    let blocked_marker = pending_bypass_marker(root);
    let blocked_old_tip = blocked_marker["pairs"][0]["old"]
        .as_str()
        .expect("blocked rewrite should identify its old tip")
        .to_owned();
    assert_rewritten_tip_enters_without_old_tip(&pair, &blocked_old_tip);
    let marker_path = fs::read_dir(root.join(".git"))
        .expect("common Git directory should read")
        .map(|entry| entry.expect("marker entry should read").path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PENDING_BYPASS_PREFIX))
        })
        .expect("blocked marker should exist");
    let held_marker = pair.worktrees.join("blocked-marker.json");
    fs::rename(&marker_path, &held_marker)
        .expect("hold the second marker while preparing the first");
    let first_root = add_worktree(root, &pair.worktrees, "budget-first");
    let claimed = claim(
        &first_root,
        "file:tests/base.rs",
        THIRD_RUN,
        "docs/budget-first.md",
        "consume the first rewrite comparison",
    );
    assert!(claimed.status.success(), "{}", json_output(&claimed));
    let first_id = reservation_id(&claimed);
    commit_work(
        &first_root,
        "tests/base.rs",
        "// first protected phase\n",
        "first checkpoint",
    );
    let checkpoint = run_berth(&first_root, &["release", &first_id, "--json"]);
    assert!(checkpoint.status.success(), "{}", json_output(&checkpoint));
    assert_eq!(json_output(&checkpoint)["status"], "outstanding");
    let journal = root.join(JOURNAL_PATH);
    let permissions = fs::metadata(&journal)
        .expect("journal metadata should read")
        .permissions();
    fs::set_permissions(
        &journal,
        fs::Permissions::from_mode(permissions.mode() & !0o222),
    )
    .expect("journal should reject amend reconciliation");
    let amended = GIT.output(
        &first_root,
        [
            "commit",
            "--amend",
            "--quiet",
            "-m",
            "rewritten first checkpoint",
        ],
    );
    fs::set_permissions(&journal, permissions).expect("journal permissions should restore");
    assert!(
        amended.status.success(),
        "{}",
        String::from_utf8_lossy(&amended.stderr)
    );
    assert_eq!(pending_bypass_count(root), 1);
    let first_marker = pending_bypass_marker(root);
    assert_eq!(first_marker["kind"], "branch_rewrite");
    let first_path = root
        .join(".git")
        .join(format!("{PENDING_BYPASS_PREFIX}000-budget-first.json"));
    // Preserve the real hook payload while arranging deterministic importer order.
    for entry in fs::read_dir(root.join(".git")).expect("common Git directory should read") {
        let entry = entry.expect("marker entry should read");
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(PENDING_BYPASS_PREFIX)
        {
            fs::rename(entry.path(), &first_path)
                .expect("first marker should sort before the blocked marker");
        }
    }
    fs::rename(held_marker, marker_path).expect("restore the untouched blocked marker");
    assert_eq!(pending_bypass_count(root), 2);
    save_pair(&pair);
    fs::write(
        root.join(".git/pending-rewrite-budget.json"),
        serde_json::to_vec(
            &serde_json::json!({"first_id":first_id,"blocked_old_tip":blocked_old_tip}),
        )
        .expect("rewrite budget should encode"),
    )
    .expect("rewrite budget should write");
}

fn deferred_pair(repository: &Path) -> PendingPair<'_> {
    super::gate_templates::build_scratch_repository(repository);
    let initialized = run_berth(repository, &["init", "--json"]);
    assert!(
        initialized.status.success(),
        "{}",
        json_output(&initialized)
    );
    git(repository, &["add", "--force", ".claude/config/berth.toml"]);
    git(
        repository,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "configure berth",
        ],
    );
    let repository_root = repository;
    let worktrees = repository.join(".git/fixture-worktrees");
    fs::create_dir_all(&worktrees).expect("fixture worktree parent should exist");
    let holder_root = add_worktree(repository_root, &worktrees, "pair-holder");
    let blocked_root = add_worktree(repository_root, &worktrees, "pair-blocked");
    dirty_source(&holder_root, "src/lib.rs");
    let holder = claim(
        &holder_root,
        "tree:src",
        FIRST_RUN,
        "docs/pair-holder.md",
        "pair-holder",
    );
    let holder_id = reservation_id(&holder);
    let blocked = defer_claim(
        &blocked_root,
        "file:src/lib.rs",
        SECOND_RUN,
        "docs/pair-blocked.md",
        "pair-blocked",
        &holder_id,
    );
    PendingPair {
        repository,
        worktrees,
        blocked_root,
        blocked_id: reservation_id(&blocked),
        holder_id,
    }
}

fn assert_rewritten_tip_enters_without_old_tip(pair: &PendingPair<'_>, old_tip: &str) {
    for target in ["main", "HEAD"] {
        let contains_old = GIT.output(
            &pair.blocked_root,
            ["merge-base", "--is-ancestor", old_tip, target],
        );
        assert_eq!(
            contains_old.status.code(),
            Some(1),
            "old protected tip must be absent from {target}: {}",
            String::from_utf8_lossy(&contains_old.stderr)
        );
    }
    let already_on_trunk = GIT.output(
        &pair.blocked_root,
        ["merge-base", "--is-ancestor", "HEAD", "main"],
    );
    assert_eq!(already_on_trunk.status.code(), Some(1));
    git(
        &pair.blocked_root,
        &["merge-base", "--is-ancestor", "main", "HEAD"],
    );
}

fn claim(repository_root: &Path, scope: &str, run: &str, plan: &str, phase: &str) -> Output {
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            plan,
            "--phase",
            phase,
            "--why",
            "protect test work",
            "--json",
        ],
    )
}

fn defer_claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    plan: &str,
    phase: &str,
    blocker: &str,
) -> Output {
    assert!(
        claim(repository_root, &deferring_run_scope(run), run, plan, phase)
            .status
            .success()
    );
    run_berth(
        repository_root,
        &[
            "claim",
            scope,
            "--run",
            run,
            "--plan",
            plan,
            "--phase",
            phase,
            "--defer",
            blocker,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "protect deferred work",
            "--json",
        ],
    )
}

fn dirty_source(root: &Path, path: &str) {
    let target = root.join(path);
    fs::create_dir_all(target.parent().expect("held path has a parent"))
        .expect("held directory should exist");
    fs::write(target, "uncommitted holder work\n").expect("held work should write");
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let worktree_root = parent.join(branch);
    GIT.add_worktree_without_hooks(repository_root, &worktree_root, branch, "HEAD");
    worktree_root
}

fn commit_work(repository_root: &Path, path: &str, contents: &str, message: &str) -> String {
    fs::write(repository_root.join(path), contents).expect("work source should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
    git_stdout(repository_root, &["rev-parse", "HEAD"])
}

fn pending_bypass_count(repository_root: &Path) -> usize {
    fs::read_dir(repository_root.join(".git"))
        .expect("common git directory should read")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(PENDING_BYPASS_PREFIX))
        .count()
}

fn pending_bypass_marker(repository_root: &Path) -> serde_json::Value {
    let marker_path = fs::read_dir(repository_root.join(".git"))
        .expect("common git directory should read")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PENDING_BYPASS_PREFIX))
        })
        .expect("pending bypass marker should exist");
    let marker = fs::read(marker_path).expect("pending bypass marker should read");
    serde_json::from_slice(&marker).expect("pending bypass marker should contain valid JSON")
}

fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should report a reservation id")
        .to_owned()
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should print JSON")
}
fn set_gate_mode(root: &Path, mode: &str) { super::gate_templates::set_gate_mode(root, mode); }
fn git(root: &Path, args: &[&str]) { GIT.run(root, args); }
fn git_stdout(root: &Path, args: &[&str]) -> String { GIT.stdout(root, args) }
fn run_berth(root: &Path, args: &[&str]) -> Output {
    cargo_berth_test_support::berth_command(BERTH_EXECUTABLE)
        .args(args)
        .current_dir(root)
        .env_remove("CARGO_BERTH_BYPASS")
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("pending rewrite setup should run")
}
