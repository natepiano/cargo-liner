//! Sorting preserves the displayed hierarchy and its preassigned colors.

use tui_pane::CellPoint;

use super::*;
use crate::census::Activity;
use crate::census::worker_dispatch::WorkerTask;
use crate::sort;
use crate::sort::HeaderHit;
use crate::sort::SortDirection;
use crate::sort::TreeSort;

const COLUMNS: [TreeColumn; 7] = [
    TreeColumn::Session,
    TreeColumn::Role,
    TreeColumn::Status,
    TreeColumn::Account,
    TreeColumn::Branch,
    TreeColumn::Directory,
    TreeColumn::Age,
];

fn sort_account(label: &str) -> Account {
    serde_json::from_value(serde_json::json!({"home":format!("~/.{label}"),"label":label}))
        .expect("fixture account")
}

/// The oldest sibling sorts after the younger by name, status, account and path.
fn sorting_forest() -> MachineState {
    let root = AgentRow {
        session_id: SessionIdentity::Claude("root-id".into()),
        ..agent(10, "root", 10 * HOUR, None)
    };
    let older = AgentRow {
        session_id: SessionIdentity::Claude("older-id".into()),
        account: sort_account("z-account"),
        activity: AgentActivity::from(Some("idle")),
        branch: Some("z-branch".into()),
        directory: "/z-directory".into(),
        ..agent(20, "zeta", 8 * HOUR, Some(10))
    };
    let younger = AgentRow {
        session_id: SessionIdentity::Claude("younger-id".into()),
        account: sort_account("a-account"),
        activity: AgentActivity::from(Some("thinking")),
        branch: Some("a-branch".into()),
        directory: "/a-directory".into(),
        children: vec![child(0, ChildKind::Shell, Some(41), "a-shell", 2 * HOUR)],
        ..agent(30, "alpha", 4 * HOUR, Some(10))
    };
    let nested = AgentRow {
        children: vec![child(0, ChildKind::Thread, None, "z-thread", 3 * HOUR)],
        ..agent(40, "nested", 5 * HOUR, Some(20))
    };
    MachineState::Answered(
        vec![root, older, younger, nested],
        HierarchyEdges(vec![
            HierarchyEdge {
                manager: "root-id".into(),
                member:  "older-id".into(),
                role:    HierarchyRole::Fix,
            },
            HierarchyEdge {
                manager: "root-id".into(),
                member:  "younger-id".into(),
                role:    HierarchyRole::Impl,
            },
        ])
        .into(),
    )
}

/// The adapter is the only test helper coupled to the frame API.
fn sorted_buffer(machines: &[Machine<'_>], width: u16, sort: TreeSort) -> Buffer {
    let placement = AgentPlacement::from_machines(machines);
    let defaults = cell_order_placed(&placement);
    let sorts = defaults
        .iter()
        .map(|cell| (cell.id.clone(), sort))
        .collect();
    let cells = cell_order_sorted(&placement, &sorts, NOW);
    let entry = &cells[0];
    let area = area(width, 20);
    let mut buffer = Buffer::empty(area);
    draw(
        &mut buffer,
        area,
        entry,
        &cells,
        NOW,
        AccountColumn::Shown,
        &FooterColors::parse(
            "role.worker=bright-cyan\naccount.a-account=orange\naccount.z-account=purple",
        ),
    );
    buffer
}

fn visible_order(buffer: &Buffer, names: &[&str]) -> Vec<String> {
    let mut positions: Vec<_> = names
        .iter()
        .map(|name| {
            (
                find(buffer, name).expect("fixture row visible").1,
                (*name).to_string(),
            )
        })
        .collect();
    positions.sort_by_key(|item| item.0);
    positions.into_iter().map(|(_, name)| name).collect()
}

fn name_color(buffer: &Buffer, name: &str) -> Color {
    buffer[find(buffer, name).expect("name drawn")].fg
}

#[test]
fn default_tree_sort_preserves_phase_one_rows_rails_and_hues() {
    let state = sorting_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let buffer = sorted_buffer(&machines, 220, TreeSort::default());
    assert_eq!(
        visible_order(&buffer, &["zeta", "nested", "z-thread", "alpha", "a-shell"]),
        ["zeta", "nested", "z-thread", "alpha", "a-shell"]
    );
    let rendered = lines(&buffer);
    for (name, expected) in [
        (
            "zeta",
            "├─ zeta fix idle z-account z-branch /z-directory 8h",
        ),
        (
            "nested",
            "│ └─ nested session thinking — — ~/rust/nested 5h",
        ),
        ("z-thread", "│ └─ z-thread worker idle — — — 3h"),
        (
            "alpha",
            "└─ alpha impl thinking a-account a-branch /a-directory 4h",
        ),
        ("a-shell", "└─ a-shell — command — — — 2h"),
    ] {
        let y = usize::from(find(&buffer, name).expect("default row").1);
        assert_eq!(
            rendered[y].split_whitespace().collect::<Vec<_>>().join(" "),
            expected
        );
    }

    for (name, hue) in [
        ("zeta", RainbowHue::Orange),
        ("nested", RainbowHue::Yellow),
        ("alpha", RainbowHue::Green),
    ] {
        assert_eq!(
            name_color(&buffer, name),
            Role::Rainbow(hue).style().fg.expect("rainbow color"),
            "{name}"
        );
    }
    assert_eq!(
        name_color(&buffer, "root"),
        text_default(),
        "the root header keeps its phase-one text style"
    );
    assert_eq!(
        name_color(&buffer, "z-thread"),
        name_color(&buffer, "nested")
    );
    assert_eq!(name_color(&buffer, "a-shell"), text_default());
    let drawn = lines(&buffer);
    assert!(drawn[usize::from(find(&buffer, "zeta").expect("zeta").1)].contains("├─ zeta"));
    assert!(drawn[usize::from(find(&buffer, "alpha").expect("alpha").1)].contains("└─ alpha"));
}

/// Check one column independently so each regression stays below one second.
fn assert_column_keeps_subtrees_and_hues(column: TreeColumn) {
    let state = sorting_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let baseline = sorted_buffer(&machines, 220, TreeSort::default());
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let buffer = sorted_buffer(&machines, 220, TreeSort { column, direction });
        let alpha_first = matches!(direction, SortDirection::Ascending)
            != matches!(column, TreeColumn::Role | TreeColumn::Status);
        let expected = if alpha_first {
            vec!["alpha", "a-shell", "zeta", "nested", "z-thread"]
        } else {
            vec!["zeta", "nested", "z-thread", "alpha", "a-shell"]
        };
        assert_eq!(
            visible_order(&buffer, &["zeta", "nested", "z-thread", "alpha", "a-shell"]),
            expected,
            "{column:?} {direction:?}"
        );
        for name in ["root", "zeta", "nested", "z-thread", "alpha", "a-shell"] {
            assert_eq!(
                name_color(&buffer, name),
                name_color(&baseline, name),
                "{column:?} {direction:?} {name}"
            );
        }
        let first = if alpha_first { "alpha" } else { "zeta" };
        let last = if alpha_first { "zeta" } else { "alpha" };
        let drawn = lines(&buffer);
        assert!(
            drawn[usize::from(find(&buffer, first).expect("first sibling").1)]
                .contains(&format!("├─ {first}"))
        );
        assert!(
            drawn[usize::from(find(&buffer, last).expect("last sibling").1)]
                .contains(&format!("└─ {last}"))
        );
    }
}

#[test]
fn session_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Session);
}

#[test]
fn role_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Role);
}

#[test]
fn status_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Status);
}

#[test]
fn account_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Account);
}

#[test]
fn branch_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Branch);
}

#[test]
fn directory_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Directory);
}

#[test]
fn age_sort_keeps_siblings_subtrees_and_hues_in_both_directions() {
    assert_column_keeps_subtrees_and_hues(TreeColumn::Age);
}

#[test]
fn tree_header_clicks_match_drawn_columns_and_ignore_gaps_and_data_rows() {
    let state = sorting_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let cells = cell_order(&machines);
    for width in [60, 220] {
        let buffer = sorted_buffer(&machines, width, TreeSort::default());
        assert_eq!(
            matches!(
                CellView::fitted(&cells[0], buffer.area, NOW, AccountColumn::Shown),
                CellView::Compressed
            ),
            width == 60
        );
        for column in COLUMNS {
            let Some((x, y)) = find(&buffer, column.header()) else {
                continue;
            };
            let spans = header_spans(&cells[0], buffer.area, NOW, AccountColumn::Shown);
            assert_eq!(
                sort::header_hit(
                    &spans,
                    CellPoint {
                        column: x,
                        row:    y,
                    }
                ),
                HeaderHit::Column(column)
            );
            assert_eq!(
                sort::header_hit(
                    &spans,
                    CellPoint {
                        column: x,
                        row:    y + 1,
                    }
                ),
                HeaderHit::Miss
            );
        }
        let header_y = find(&buffer, "session").expect("header").1;
        let spans = header_spans(&cells[0], buffer.area, NOW, AccountColumn::Shown);
        for pair in spans.windows(2) {
            if pair[0].columns.end < pair[1].columns.start {
                assert_eq!(
                    sort::header_hit(
                        &spans,
                        CellPoint {
                            column: pair[0].columns.end,
                            row:    header_y,
                        }
                    ),
                    HeaderHit::Miss
                );
            }
        }
    }
}

#[test]
fn only_the_selected_tree_header_has_a_direction_arrow() {
    let state = sorting_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    for column in COLUMNS {
        for (direction, arrow) in [
            (SortDirection::Ascending, "▲"),
            (SortDirection::Descending, "▼"),
        ] {
            let buffer = sorted_buffer(&machines, 220, TreeSort { column, direction });
            let header = &lines(&buffer)[usize::from(find(&buffer, "session").expect("header").1)];
            assert_eq!(header.matches('▲').count() + header.matches('▼').count(), 1);
            assert!(
                header.contains(&format!("{} {arrow}", column.header())),
                "{header}"
            );
        }
    }
}

#[test]
fn choosing_a_tree_column_uses_its_natural_direction_and_repeated_clicks_reverse_it() {
    let mut sort = TreeSort::default();
    sort.select(TreeColumn::Age);
    assert_eq!(sort.direction, SortDirection::Ascending);
    for column in COLUMNS
        .into_iter()
        .filter(|column| *column != TreeColumn::Age)
    {
        sort.select(column);
        assert_eq!(
            sort,
            TreeSort {
                column,
                direction: SortDirection::Ascending
            }
        );
        sort.select(column);
        assert_eq!(
            sort,
            TreeSort {
                column,
                direction: SortDirection::Descending
            }
        );
    }
    sort.select(TreeColumn::Age);
    assert_eq!(
        sort,
        TreeSort {
            column:    TreeColumn::Age,
            direction: SortDirection::Descending,
        }
    );
}

#[test]
fn nested_work_sorts_its_own_siblings_and_keeps_each_subtree_together() {
    let older = ChildRow {
        worker_task: WorkerTask::Unrecorded,
        account: sort_account("z-account"),
        activity: AgentActivity::Idle,
        ..child(1, ChildKind::Thread, None, "z-work", HOUR)
    };
    let younger = ChildRow {
        worker_task: WorkerTask::Unrecorded,
        account: sort_account("a-account"),
        activity: AgentActivity::Active(Activity::Thinking),
        ..child(1, ChildKind::Thread, None, "a-work", 30 * MINUTE)
    };
    let root = AgentRow {
        children: vec![
            child(0, ChildKind::Subagent, None, "work-parent", 2 * HOUR),
            older,
            child(2, ChildKind::Shell, Some(51), "z-descendant", 45 * MINUTE),
            younger,
            child(2, ChildKind::Shell, Some(52), "a-descendant", 20 * MINUTE),
        ],
        ..agent(10, "root", 10 * HOUR, None)
    };
    let state = MachineState::Answered(vec![root], HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    for column in COLUMNS {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let buffer = sorted_buffer(&machines, 220, TreeSort { column, direction });
            let alpha_first = match column {
                TreeColumn::Status => direction == SortDirection::Descending,
                TreeColumn::Session | TreeColumn::Account | TreeColumn::Age => {
                    direction == SortDirection::Ascending
                },
                TreeColumn::Role | TreeColumn::Branch | TreeColumn::Directory => false,
            };
            let expected = if alpha_first {
                vec![
                    "work-parent",
                    "a-work",
                    "a-descendant",
                    "z-work",
                    "z-descendant",
                ]
            } else {
                vec![
                    "work-parent",
                    "z-work",
                    "z-descendant",
                    "a-work",
                    "a-descendant",
                ]
            };
            assert_eq!(
                visible_order(
                    &buffer,
                    &[
                        "work-parent",
                        "z-work",
                        "z-descendant",
                        "a-work",
                        "a-descendant"
                    ]
                ),
                expected,
                "{column:?} {direction:?}"
            );
        }
    }
}

#[test]
fn equal_tree_keys_keep_the_default_start_and_pid_tie_break_in_both_directions() {
    let mut state = sorting_forest();
    if let MachineState::Answered(rows, _) = &mut state {
        rows[1].activity = crate::census::AgentActivity::Active(Activity::Thinking);
        rows[2].activity = crate::census::AgentActivity::Active(Activity::Thinking);
        rows[2].started = rows[1].started;
    }
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let buffer = sorted_buffer(
            &machines,
            220,
            TreeSort {
                column: TreeColumn::Status,
                direction,
            },
        );
        assert_eq!(
            visible_order(&buffer, &["zeta", "alpha"]),
            ["zeta", "alpha"]
        );
    }
}

#[test]
fn sorting_preserves_the_footer_role_and_account_foregrounds() {
    let state = sorting_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    for column in COLUMNS {
        let buffer = sorted_buffer(
            &machines,
            220,
            TreeSort {
                column,
                direction: SortDirection::Ascending,
            },
        );
        for (label, color) in [
            ("a-account", Color::Indexed(208)),
            ("z-account", Color::Indexed(141)),
        ] {
            assert_eq!(
                buffer[find(&buffer, label).expect("account label")].fg,
                color
            );
        }
        let role_x = find(&buffer, "role").expect("role header").0;
        for name in ["zeta", "alpha"] {
            let row = find(&buffer, name).expect("worker row").1;
            assert_eq!(buffer[(role_x, row)].fg, Color::LightCyan);
        }
    }
}

fn independent_sort_forest() -> MachineState {
    let mut root = agent(10, "root", 12 * HOUR, None);
    root.session_id = SessionIdentity::Claude("root-id".into());
    let mut rows = vec![root];
    let mut edges = Vec::new();
    for (pid, name, role, status, account, branch, directory, age) in [
        (
            20,
            "bee",
            HierarchyRole::Fix,
            "idle",
            "c-account",
            "d-branch",
            "/d",
            8 * HOUR,
        ),
        (
            30,
            "ant",
            HierarchyRole::Impl,
            "waiting",
            "d-account",
            "c-branch",
            "/b",
            6 * HOUR,
        ),
        (
            40,
            "dog",
            HierarchyRole::Review,
            "git",
            "a-account",
            "a-branch",
            "/c",
            4 * HOUR,
        ),
        (
            50,
            "cat",
            HierarchyRole::Test,
            "reading",
            "b-account",
            "b-branch",
            "/a",
            2 * HOUR,
        ),
    ] {
        let id = format!("id-{pid}");
        rows.push(AgentRow {
            session_id: SessionIdentity::Claude(id.clone()),
            activity: AgentActivity::from(Some(status)),
            account: sort_account(account),
            branch: Some(branch.into()),
            directory: directory.into(),
            children: vec![child(
                0,
                ChildKind::Shell,
                Some(pid + 1),
                &format!("under-{name}"),
                MINUTE,
            )],
            ..agent(pid, name, age, Some(10))
        });
        edges.push(HierarchyEdge {
            manager: "root-id".into(),
            member: id,
            role,
        });
    }
    MachineState::Answered(rows, HierarchyEdges(edges).into())
}

#[test]
fn each_tree_sort_uses_its_own_column_instead_of_reusing_name_or_age_order() {
    let state = independent_sort_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    for (column, ascending) in [
        (TreeColumn::Session, ["ant", "bee", "cat", "dog"]),
        (TreeColumn::Role, ["bee", "ant", "dog", "cat"]),
        (TreeColumn::Status, ["dog", "bee", "cat", "ant"]),
        (TreeColumn::Account, ["dog", "cat", "bee", "ant"]),
        (TreeColumn::Branch, ["dog", "cat", "ant", "bee"]),
        (TreeColumn::Directory, ["cat", "ant", "dog", "bee"]),
        (TreeColumn::Age, ["cat", "dog", "ant", "bee"]),
    ] {
        for direction in [SortDirection::Ascending, SortDirection::Descending] {
            let buffer = sorted_buffer(&machines, 220, TreeSort { column, direction });
            let mut expected = ascending;
            if direction == SortDirection::Descending {
                expected.reverse();
            }
            assert_eq!(
                visible_order(&buffer, &["bee", "ant", "dog", "cat"]),
                expected,
                "{column:?} {direction:?}"
            );
            for name in ascending {
                assert_eq!(
                    find(&buffer, &format!("under-{name}"))
                        .expect("work child")
                        .1,
                    find(&buffer, name).expect("parent").1 + 1
                );
            }
        }
    }
}
