//! Detailed activity is drawn and sorted in ordinary trees and unassigned pools.

#![expect(
    clippy::panic,
    reason = "an unexpected fixture tile kind must fail the test"
)]

use ratatui::buffer::Cell;

use super::*;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::RunLocation;
use crate::census::VerifyStep;
use crate::census::worker_dispatch::WorkerTask;
use crate::sort::SortDirection;

fn activity_forest() -> MachineState {
    let rows = [
        (20, "writer-root", AgentActivity::Active(Activity::Writing)),
        (30, "reader-root", AgentActivity::Active(Activity::Reading)),
        (
            40,
            "gate-root",
            AgentActivity::Active(Activity::Verify {
                step:     VerifyStep::Clippy,
                location: RunLocation::Local,
            }),
        ),
        (50, "waiting-root", AgentActivity::Active(Activity::Waiting)),
    ]
    .map(|(pid, name, activity)| AgentRow {
        activity,
        children: vec![ChildRow {
            activity: AgentActivity::Active(Activity::Verify {
                step:     VerifyStep::Nextest,
                location: RunLocation::Mac,
            }),
            ..child(0, ChildKind::Thread, None, &format!("nested-{pid}"), MINUTE)
        }],
        ..agent(pid, name, HOUR + u64::from(pid), None)
    });
    MachineState::Answered(rows.into(), HierarchyEdges::default())
}

fn activity_pool(sort: TreeSort) -> Buffer {
    let state = activity_forest();
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let sorts = HashMap::from([(
        TileKey::Unassigned {
            machine: "fixture".into(),
        },
        sort,
    )]);
    let tiles = tile_order_sorted(&placement, &sorts, NOW);
    let AgentTile::Unassigned(entry) = &tiles[0] else {
        panic!("all roots are unassigned")
    };
    let rect = area(240, unassigned_height(entry));
    let mut buffer = Buffer::empty(rect);
    draw_unassigned(
        &mut buffer,
        rect,
        entry,
        NOW,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    buffer
}

fn status_at<'a>(buffer: &'a Buffer, name: &str) -> &'a Cell {
    let x = find(buffer, TREE_STATUS_HEADER).expect("status header").0;
    let y = find(buffer, name).expect("named row").1;
    &buffer[(x, y)]
}

#[test]
fn activity_unassigned_root_and_nested_worker_draw_words_and_status_colors() {
    let buffer = activity_pool(TreeSort::default());
    let status_x = find(&buffer, TREE_STATUS_HEADER).expect("status header").0;
    for (name, word) in [
        ("writer-root", "writing"),
        ("reader-root", "reading"),
        ("gate-root", "clippy"),
        ("waiting-root", "waiting"),
        ("nested-20", "nextest (mac)"),
    ] {
        let y = find(&buffer, name).expect("row").1;
        let text: String = (status_x..status_x + u16::try_from(word.len()).expect("short word"))
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        assert_eq!(text, word, "{name}");
    }
    assert_eq!(
        status_at(&buffer, "writer-root").fg,
        Role::Busy.style().fg.expect("busy color")
    );
    assert_eq!(
        status_at(&buffer, "reader-root").fg,
        status_at(&buffer, "writer-root").fg
    );
    assert_eq!(
        status_at(&buffer, "waiting-root").fg,
        Role::Idle.style().fg.expect("idle color")
    );
    assert_eq!(
        status_at(&buffer, "gate-root").fg,
        status_at(&buffer, "nested-20").fg,
        "verify steps share an accent"
    );
    assert_ne!(
        status_at(&buffer, "gate-root").fg,
        status_at(&buffer, "waiting-root").fg
    );
    assert_ne!(
        status_at(&buffer, "gate-root").fg,
        status_at(&buffer, "writer-root").fg,
        "gates have their own accent"
    );
}

#[test]
fn activity_pool_status_sort_uses_drawn_words_and_keeps_workers_under_their_roots() {
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let buffer = activity_pool(TreeSort {
            column: TreeColumn::Status,
            direction,
        });
        let mut names = ["writer-root", "reader-root", "gate-root", "waiting-root"];
        names.sort_by_key(|name| find(&buffer, name).expect("root").1);
        let mut expected = ["gate-root", "reader-root", "waiting-root", "writer-root"];
        if direction == SortDirection::Descending {
            expected.reverse();
        }
        assert_eq!(names, expected);
        for (pid, root) in [
            (20, "writer-root"),
            (30, "reader-root"),
            (40, "gate-root"),
            (50, "waiting-root"),
        ] {
            assert_eq!(
                find(&buffer, &format!("nested-{pid}")).expect("worker").1,
                find(&buffer, root).expect("root").1 + 1
            );
        }
    }
}

#[test]
fn activity_tree_account_cells_hide_standard_tier_and_show_only_fast() {
    let account: Account =
        serde_json::from_value(serde_json::json!({"home":"/fixture/codex1", "label":"codex1"}))
            .expect("account");
    let state = MachineState::Answered(
        vec![AgentRow {
            children: vec![
                ChildRow {
                    account: account.clone(),
                    service_tier: ServiceTier::Standard,
                    ..child(0, ChildKind::Thread, None, "standard-worker", HOUR)
                },
                ChildRow {
                    account,
                    service_tier: ServiceTier::Fast,
                    ..child(0, ChildKind::Thread, None, "fast-worker", MINUTE)
                },
            ],
            ..agent(10, "manager", 2 * HOUR, None)
        }],
        HierarchyEdges::default(),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = cell_order_placed(&placement);
    let rect = area(240, 10);
    let mut buffer = Buffer::empty(rect);
    draw(
        &mut buffer,
        rect,
        &cells[0],
        &cells,
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    let account_x = find(&buffer, TREE_ACCOUNT_HEADER)
        .expect("account header")
        .0;
    for (name, expected) in [
        ("standard-worker", "codex1"),
        ("fast-worker", "codex1 fast"),
    ] {
        let y = find(&buffer, name).expect("worker row").1;
        let text: String = (account_x..account_x + 13)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        assert_eq!(text.trim(), expected);
    }
}

#[test]
fn activity_managed_tree_sorts_status_words_without_reordering_each_workers_children() {
    let mut root = agent(10, "director", 4 * HOUR, None);
    root.session_id = SessionIdentity::Claude("root-id".into());
    let mut reader = agent(20, "reader-member", 3 * HOUR, None);
    reader.session_id = SessionIdentity::Claude("reader-id".into());
    reader.activity = AgentActivity::Active(Activity::Reading);
    let mut writer = agent(30, "writer-member", 2 * HOUR, None);
    writer.session_id = SessionIdentity::Claude("writer-id".into());
    writer.activity = AgentActivity::Active(Activity::Writing);
    writer.children = vec![ChildRow {
        activity: AgentActivity::Active(Activity::Verify {
            step:     VerifyStep::Fmt,
            location: RunLocation::Local,
        }),
        ..child(0, ChildKind::Thread, None, "writer-worker", MINUTE)
    }];
    let state = MachineState::Answered(
        vec![root, writer, reader],
        HierarchyEdges(vec![
            HierarchyEdge {
                manager: "root-id".into(),
                member:  "reader-id".into(),
                role:    HierarchyRole::Test,
            },
            HierarchyEdge {
                manager: "root-id".into(),
                member:  "writer-id".into(),
                role:    HierarchyRole::Impl,
            },
        ]),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let original = cell_order_placed(&placement);
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let sorts = HashMap::from([(
            TileKey::Agent(original[0].id.clone()),
            TreeSort {
                column: TreeColumn::Status,
                direction,
            },
        )]);
        let tiles = tile_order_sorted(&placement, &sorts, NOW);
        let AgentTile::Agent(entry) = &tiles[0] else {
            panic!("the director has a production role")
        };
        let buffer = drawn_into(entry, &original, 240, 12);
        let reader_y = find(&buffer, "reader-member").expect("reader").1;
        let writer_y = find(&buffer, "writer-member").expect("writer").1;
        assert_eq!(reader_y < writer_y, direction == SortDirection::Ascending);
        assert_eq!(
            find(&buffer, "writer-worker").expect("worker").1,
            writer_y + 1
        );
        assert_eq!(status_at(&buffer, "reader-member").symbol(), "r");
        assert_eq!(
            status_at(&buffer, "writer-member").fg,
            Role::Busy.style().fg.expect("busy color")
        );
    }
}

#[test]
fn activity_worker_names_use_the_final_hierarchy_role_and_never_draw_raw_thread_ids() {
    let task_id = "01a142f0-1234-5678-9abc-0123456789ab";
    let unreadable_id = "01a1437d-1234-5678-9abc-0123456789ab";
    let mut root = agent(10, "director", 4 * HOUR, None);
    root.session_id = SessionIdentity::Claude("root-id".into());
    root.children = vec![
        ChildRow {
            thread_id: CodexThreadId::Recorded(task_id.into()),
            worker_task: WorkerTask::from_prompt(
                "Repair the widget\nDo not use this follow-up line",
            ),
            ..child(0, ChildKind::Thread, None, task_id, HOUR)
        },
        ChildRow {
            thread_id: CodexThreadId::Recorded(unreadable_id.into()),
            worker_task: WorkerTask::from_prompt(""),
            ..child(0, ChildKind::Thread, None, unreadable_id, MINUTE)
        },
    ];
    let state = MachineState::Answered(
        vec![root],
        HierarchyEdges(vec![
            HierarchyEdge {
                manager: "root-id".into(),
                member:  task_id.into(),
                role:    HierarchyRole::Fix,
            },
            HierarchyEdge {
                manager: "root-id".into(),
                member:  unreadable_id.into(),
                role:    HierarchyRole::Test,
            },
        ]),
    );
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let agents = cell_order_placed(&placement);
    let buffer = drawn_into(&agents[0], &agents, 240, 12);
    let text = lines(&buffer).join("\n");
    assert!(text.contains("fix · Repair the widget"), "{text}");
    assert!(
        text.contains("└─ test"),
        "the unreadable task still has its hierarchy role: {text}"
    );
    assert!(
        !text.contains(task_id) && !text.contains(unreadable_id),
        "{text}"
    );
    assert!(!text.contains("follow-up line"), "{text}");
    let role_x = find(&buffer, TREE_ROLE_HEADER).expect("role header").0;
    let y = find(&buffer, "fix · Repair the widget")
        .expect("task worker")
        .1;
    assert_eq!(buffer[(role_x, y)].symbol(), "f");
}
