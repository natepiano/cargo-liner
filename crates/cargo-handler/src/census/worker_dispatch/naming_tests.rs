//! A worker's first task locates its own roster without a surviving launcher.

#![expect(
    clippy::expect_used,
    reason = "temporary rosters and rollouts must be readable"
)]

use std::time::Duration;
use std::time::UNIX_EPOCH;

use tempfile::TempDir;

use super::*;

const THREAD: &str = "01a142f0-1234-5678-9abc-0123456789ab";
const TASK: &str = "Repair the portal without losing pending state";

fn session(root: &Path) -> TempDir {
    fs::create_dir_all(root).expect("fixture session root exists");
    tempfile::Builder::new()
        .prefix("handler-names-")
        .tempdir_in(root)
        .expect("isolated session")
}

fn write_roster(directory: &Path, thread: &str, seat: &str, modified: u64) {
    let path = directory.join(ROSTER_FILE);
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            seat:{"thread_id":thread, "role":"fix", "launcher_pid":null}
        }))
        .expect("roster encodes"),
    )
    .expect("roster writes");
    fs::File::options()
        .write(true)
        .open(&path)
        .expect("roster opens")
        .set_modified(UNIX_EPOCH + Duration::from_secs(modified))
        .expect("fixture mtime sets without sleeping");
}

#[test]
fn worker_naming_launcher_gone_still_uses_its_session_roster_and_first_task_line() {
    let root = TempDir::new().expect("isolated session roots");
    let roots = SessionRoots::new([root.path().join("unit"), root.path().join("delegate")]);
    for family in ["unit", "delegate"] {
        let directory = session(&root.path().join(family));
        write_roster(directory.path(), THREAD, "fixture-fix2", 100);
        let prompt = format!(
            "{TASK}\n\nWrite the summary to {}/phases/4/impl_summary_fix.txt\nIgnore this later task line.",
            directory.path().display()
        );
        let label = WorkerNames::default().name(
            THREAD,
            &prompt,
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            WorkerRoleSource::Unroled,
            &roots,
        );
        assert_eq!(label, format!("fix2 · {TASK}"));
        assert!(!label.contains(THREAD));
    }
}

#[test]
fn worker_naming_no_matching_roster_uses_role_and_task_or_role_alone() {
    let root = TempDir::new().expect("isolated session roots");
    let roots = SessionRoots::new([root.path().to_path_buf()]);
    let mut names = WorkerNames::default();
    let role = WorkerRoleSource::Hierarchy(HierarchyRole::Test);
    assert_eq!(
        names.name(
            THREAD,
            &format!("{TASK}\nsecond task line"),
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            role,
            &roots
        ),
        format!("test · {TASK}")
    );
    let directory = tempfile::tempdir().expect("unreadable rollout fixture");
    let unreadable =
        crate::census::tier::Rollouts::default().read(&directory.path().join("missing.jsonl"));
    assert_eq!(
        names.name(
            THREAD,
            unreadable.first_prompt.text(),
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            role,
            &roots
        ),
        "test"
    );
    assert_eq!(
        names.name(
            THREAD,
            "",
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            WorkerRoleSource::Unroled,
            &roots
        ),
        "worker"
    );
}

#[test]
fn worker_naming_chooses_a_roster_that_lists_the_thread_then_the_newest_match() {
    let root = TempDir::new().expect("isolated session roots");
    let roots = SessionRoots::new([root.path().join("unit"), root.path().join("delegate")]);
    let older = session(&root.path().join("unit"));
    let newer = session(&root.path().join("delegate"));
    write_roster(older.path(), THREAD, "fixture-fix2", 100);
    write_roster(newer.path(), "other-thread", "fixture-review", 200);
    let prompt = format!(
        "{TASK}\nBoard: {}/board.log\nSummary: {}/phases/4/summary.txt",
        older.path().display(),
        newer.path().display()
    );
    let mut names = WorkerNames::default();
    assert_eq!(
        names.name(
            THREAD,
            &prompt,
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            WorkerRoleSource::Unroled,
            &roots
        ),
        format!("fix2 · {TASK}")
    );
    write_roster(newer.path(), THREAD, "fixture-review", 300);
    assert_eq!(
        names.name(
            THREAD,
            &prompt,
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            WorkerRoleSource::Unroled,
            &roots
        ),
        format!("review · {TASK}"),
        "mtime invalidates the previously nonmatching roster"
    );
    write_roster(older.path(), THREAD, "fixture-impl", 400);
    assert_eq!(
        names.name(
            THREAD,
            &prompt,
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
            WorkerRoleSource::Unroled,
            &roots
        ),
        format!("impl · {TASK}"),
        "newest matching roster wins regardless of path order"
    );
}

#[test]
fn worker_naming_reads_the_first_task_from_rollout_after_injected_context() {
    let root = TempDir::new().expect("isolated session root");
    let roots = SessionRoots::new([root.path().join("unit")]);
    let directory = session(&root.path().join("unit"));
    write_roster(directory.path(), THREAD, "fixture-impl", 100);
    let rollout = directory.path().join("rollout.jsonl");
    let prompt = format!(
        "{TASK}\nSummary: {}/phases/4/impl_summary.txt",
        directory.path().display()
    );
    let user = |text: &str| {
        serde_json::json!({"type":"response_item","payload":{
            "type":"message", "role":"user", "content":[{"type":"input_text","text":text}]
        }})
    };
    let lines = [
        serde_json::json!({"type":"session_meta","payload":{"timestamp":"2026-10-16T13:40:00Z","cwd":"/work/fixture"}}),
        user("# AGENTS.md instructions\nDo not name the row after this context."),
        user("<environment_context>fixture</environment_context>"),
        user(&prompt),
        user("A later follow-up must not replace the first task"),
    ];
    fs::write(
        &rollout,
        format!(
            "{}\n",
            lines
                .iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .expect("rollout writes");
    let read = crate::census::tier::Rollouts::default().read(&rollout);
    assert_eq!(
        WorkerNames::default().name(
            THREAD,
            read.first_prompt.text(),
            &read.worktree,
            WorkerRoleSource::Unroled,
            &roots
        ),
        format!("impl · {TASK}")
    );
}

#[test]
fn session_discovery_uses_only_supplied_roots_and_preserves_id_boundaries_and_order() {
    let directory = TempDir::new().expect("isolated discovery roots");
    let unit = directory.path().join("unit");
    let delegate = directory.path().join("delegate");
    let roots = SessionRoots::new([unit.clone(), delegate.clone()]);
    let prompt = format!(
        "{}/z-9/summary {}/a-1_more {}/z-9/board {}/unit-other/ignored {}/unlisted/ignored {}/",
        unit.display(),
        delegate.display(),
        unit.display(),
        directory.path().display(),
        directory.path().display(),
        delegate.display()
    );
    assert_eq!(
        WorkerNames::session_dirs(&prompt, &roots),
        vec![delegate.join("a-1"), unit.join("z-9")],
        "discovery sorts and deduplicates, stops IDs before underscores, and ignores other roots"
    );
}
