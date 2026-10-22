//! Prompt boilerplate never becomes a worker's drawn job.

#![expect(
    clippy::expect_used,
    reason = "fixture paths and worker rows must exist"
)]

use std::collections::HashMap;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::agent_cell;
use crate::agent_cell::AgentTile;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::HierarchyEdge;
use crate::census::HierarchyEdges;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::tier::ThreadWorktree;
use crate::footer_colors::FooterColors;

const THREAD: &str = "01a14488-1234-5678-9abc-0123456789ab";

#[derive(Clone, Copy, Debug)]
enum NamingPath {
    RoleFallback,
    SessionRoster,
    LiveDispatch,
}

fn drawn_worker(prompt: &str, role: HierarchyRole, path: NamingPath) -> String {
    let base = Path::new("/tmp/claude/unit");
    fs::create_dir_all(base).expect("fixture session root");
    let session = tempfile::Builder::new()
        .prefix("handler-opening-")
        .tempdir_in(base)
        .expect("isolated session");
    let seat = format!("fixture-{}", role.text());
    fs::write(
        session.path().join(ROSTER_FILE),
        serde_json::json!({seat.clone():{"thread_id":THREAD,"role":role.text(),"launcher_pid":42}})
            .to_string(),
    )
    .expect("roster writes");
    let prompt = format!(
        "{prompt}\nSummary: {}/phases/5/summary.txt",
        session.path().display()
    );
    let dispatch = match path {
        NamingPath::RoleFallback => WorkerDispatch::Plain,
        NamingPath::SessionRoster => WorkerNames::default().resolve(
            THREAD,
            &prompt,
            &ThreadWorktree::Recorded(PathBuf::from("/work/fixture")),
        ),
        NamingPath::LiveDispatch => {
            let argv = [
                "bash",
                "/fixture/.claude/scripts/delegate/implement.sh",
                "--to",
                &seat,
                session.path().to_str().expect("fixture path"),
                "/work/fixture",
                "/fixture/prompt.md",
                "impl",
                &prompt,
                role.text(),
                "working",
                "0",
                role.text(),
            ]
            .map(String::from);
            ParsedDispatch::from_arguments(&argv)
                .expect("live dispatch argv")
                .resolve(&WorkerRoster::read(session.path()), 42)
        },
    };
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude","session_id":"fixture-root-id","name":"fixture-root","pid":10,"started":100,
        "directory":"/work/fixture","desktop":null,"branch":null,"launched_by":null,
        "children":[{"kind":"thread","depth":0,"pid":null,"thread_id":THREAD,
            "name":THREAD,"started":200,"timer":null}]
    }])).expect("worker rows");
    rows[0].children[0].worker_task = WorkerTask::from_prompt(&prompt);
    rows[0].children[0].dispatch = dispatch;
    let state = MachineState::Answered(
        rows,
        HierarchyEdges(vec![HierarchyEdge {
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager: "fixture-root-id".into(),
            member: THREAD.into(),
            role,
        }])
        .into(),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let agents = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &agents, &HashMap::new(), 1000);
    let area = Rect::new(0, 0, 240, 12);
    let mut buffer = Buffer::empty(area);
    match &tiles[0] {
        AgentTile::Agent(entry) => agent_cell::draw(
            &mut buffer,
            area,
            entry,
            &agents,
            1000,
            AccountColumn::Hidden,
            &FooterColors::default(),
        ),
        AgentTile::Unassigned(entry) => agent_cell::draw_unassigned(
            &mut buffer,
            area,
            entry,
            1000,
            AccountColumn::Hidden,
            &FooterColors::default(),
        ),
    }
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_opening(prompt: &str, role: HierarchyRole, expected: &str) {
    for path in [
        NamingPath::RoleFallback,
        NamingPath::SessionRoster,
        NamingPath::LiveDispatch,
    ] {
        let text = drawn_worker(prompt, role, path);
        assert!(text.contains(expected), "{path:?}: {text}");
        assert!(!text.contains("You are slot"), "{path:?}: {text}");
        assert!(!text.contains("board.sh"), "{path:?}: {text}");
        assert!(!text.contains(THREAD), "{path:?}: {text}");
    }
}

#[test]
fn prompt_opening_fix_preamble_names_the_first_board_role_job_in_every_label_path() {
    assert_opening(
        "You are slot fix, opening as fix for phase 20's repair round 14. Write the requested code directly; do not ask questions. Before your first tool call, run:\nbash ~/.claude/scripts/delegate/board.sh role /tmp/claude/delegate/fixture fix fix \"phase 20 repair round 14\"\n\nLater example: board.sh role /tmp/claude/unit/other fix fix \"wrong later job\"",
        HierarchyRole::Fix,
        "fix · phase 20 repair round 14",
    );
}

#[test]
fn prompt_opening_reviewer_preamble_uses_the_quoted_review_job() {
    assert_opening(
        "You are slot review, now working as a reviewer. Edit nothing. Read the diff\ncold as if someone else wrote it.\n\nYour first tool call: `bash ~/.claude/scripts/delegate/board.sh role /tmp/claude/unit/fixture review review \"phase 7 review, adversary lens\"`.\nNarrate each change of activity.",
        HierarchyRole::Review,
        "review · phase 7 review, adversary lens",
    );
}

#[test]
fn prompt_opening_colon_preamble_accepts_the_live_repair_prompt_shape() {
    assert_opening(
        "You are slot fix, opening as fix: a fresh seat repairing review findings in phase 17. Write the requested code directly.\n\nYour first tool call is `bash ~/.claude/scripts/delegate/board.sh role /tmp/claude/delegate/fixture fix fix \"repairing phase 17 review findings\"`.",
        HierarchyRole::Fix,
        "fix · repairing phase 17 review findings",
    );
}

#[test]
fn prompt_opening_headings_and_plain_jobs_keep_their_own_text() {
    for prompt in [
        "# Repair the portal\n\n## Role\nYou are slot fix, now working as fix.",
        "Repair the portal\nIgnore this follow-up line",
    ] {
        assert_opening(prompt, HierarchyRole::Fix, "fix · Repair the portal");
    }
}

#[test]
fn prompt_opening_preamble_without_a_job_draws_the_role_alone() {
    for path in [
        NamingPath::RoleFallback,
        NamingPath::SessionRoster,
        NamingPath::LiveDispatch,
    ] {
        let text = drawn_worker(
            "You are slot fix, now working as fix. Write code directly.\nDo not ask questions.",
            HierarchyRole::Fix,
            path,
        );
        let worker = text
            .lines()
            .find(|line| line.contains("└─"))
            .expect("worker line");
        assert!(worker.contains("fix"), "{path:?}: {text}");
        assert!(!worker.contains('·'), "no invented job: {path:?}: {text}");
        assert!(!text.contains("You are slot"), "{path:?}: {text}");
    }
}

#[test]
fn prompt_opening_quoted_backticks_and_newlines_stay_inside_the_board_job() {
    for command in [
        "board.sh role /tmp/session fix fix 'repair `WorkerTask`'",
        "board.sh role /tmp/session fix fix \"repair \\`WorkerTask\\`\"",
        "board.sh role /tmp/session fix fix $'repair `WorkerTask`'",
        "board.sh role /tmp/session fix fix 'repair\n`WorkerTask`'",
    ] {
        assert_opening(
            &format!(
                "You are slot fix, now working as fix.\nFirst call: `{command}`.\nNever draw this trailing instruction."
            ),
            HierarchyRole::Fix,
            "fix · repair `WorkerTask`",
        );
    }
}

#[test]
fn prompt_opening_missing_board_job_and_empty_heading_remain_role_only() {
    for prompt in [
        "You are slot fix, now working as fix.\n`board.sh role /tmp/session fix fix ''`",
        "You are slot fix, now working as fix.\n`board.sh role /tmp/session fix fix '   '`",
        "###   ",
    ] {
        assert_eq!(WorkerTask::from_prompt(prompt), WorkerTask::RoleOnly);
        for path in [
            NamingPath::RoleFallback,
            NamingPath::SessionRoster,
            NamingPath::LiveDispatch,
        ] {
            let text = drawn_worker(prompt, HierarchyRole::Fix, path);
            let worker = text
                .lines()
                .find(|line| line.contains("└─"))
                .expect("worker line");
            assert!(worker.contains("fix"), "{path:?}: {text}");
            assert!(!worker.contains('·'), "no invented job: {path:?}: {text}");
        }
    }
}

#[test]
fn pooled_review_titles_come_from_the_opening_and_never_from_diff_code() {
    // Exact opening of a pooled closure review prompt; its following body is a diff.
    let opening = "You are a pooled reviewer for handler-enh-unit, phase 17. Edit nothing. Your first command is git -C /home/natepiano/rust/cargo-liner-handler rev-parse HEAD; if it is not a540eb5d2fe13096e3ba380e03c2d05716fb03bd, report error. Re-read the tree; trust nothing from memory about its state. The phase's change is uncommitted: review `git diff HEAD` in the worktree.";
    let diff = "\n\n## Diff\n```diff\ndiff --git a/module.rs b/module.rs\n--- a/module.rs\n+++ b/module.rs\n@@ -1,2 +1,3 @@\n# ; /// The launcher interprets this code\n+fn launch() {}\n```";
    for prompt in [
        format!("{opening}{diff}"),
        format!("{diff}\n# misleading code heading"),
        "```rust\n# ; /// The launcher interprets this code\nfn launch() {}\n```".to_owned(),
    ] {
        let text = drawn_worker(&prompt, HierarchyRole::Review, NamingPath::RoleFallback);
        assert!(!text.contains("The launcher interprets"), "{text}");
        assert!(!text.contains("misleading code heading"), "{text}");
        let row = text
            .lines()
            .find(|line| line.contains("└─"))
            .expect("review row");
        assert!(row.contains("review"), "{text}");
    }
    let role_only = drawn_worker(
        &format!("{opening}{diff}"),
        HierarchyRole::Review,
        NamingPath::RoleFallback,
    );
    let row = role_only
        .lines()
        .find(|line| line.contains("└─"))
        .expect("review row");
    assert!(
        !row.contains('·'),
        "the pool preamble names no task: {role_only}"
    );
    assert_opening(
        &format!("# Check dispatch folding\n{opening}{diff}"),
        HierarchyRole::Review,
        "review · Check dispatch folding",
    );
    assert_opening(
        &format!("Review dispatch folding.\n{diff}"),
        HierarchyRole::Review,
        "review · Review dispatch folding.",
    );
}
