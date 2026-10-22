//! Subagent work uses activity words and the owning session's account.

use super::*;
use crate::census::Activity;
use crate::footer_colors::FooterTint;
use crate::sort::SortDirection;

fn account(label: &str) -> Account {
    serde_json::from_value(serde_json::json!({"home":format!("~/.{label}"),"label":label}))
        .expect("account fixture")
}

fn rendered(root: &AgentRow) -> Buffer {
    let cell = entry(root, None);
    let rect = area(220, height(&cell, 220, NOW, AccountColumn::Shown));
    let mut buffer = Buffer::empty(rect);
    draw(
        &mut buffer,
        rect,
        &cell,
        std::slice::from_ref(&cell),
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    buffer
}

fn column(buffer: &Buffer, header: &str, next: &str, name: &str) -> String {
    let header_row = (0..buffer.area.height)
        .find(|&y| {
            let text: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            text.split_whitespace().next() == Some("session") && text.contains("status")
        })
        .expect("tree header");
    let header_text: String = (0..buffer.area.width)
        .map(|x| buffer[(x, header_row)].symbol())
        .collect();
    let start =
        summary::cell_width(&header_text[..header_text.find(header).expect("column header")]);
    let end =
        summary::cell_width(&header_text[..header_text.find(next).expect("next column header")]);
    let row = find(buffer, name).expect("work row").1;
    (start..end)
        .map(|x| buffer[(x, row)].symbol())
        .collect::<String>()
        .trim()
        .to_owned()
}

#[test]
fn subagent_rows_show_reading_and_idle_in_the_status_column() {
    for (activity, expected) in [
        (AgentActivity::Active(Activity::Reading), "reading"),
        (AgentActivity::Idle, "idle"),
    ] {
        let root = AgentRow {
            account: account("claude1"),
            children: vec![ChildRow {
                activity,
                ..child(
                    0,
                    ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                    None,
                    "Inspect source files",
                    MINUTE,
                )
            }],
            ..agent(10, "director", HOUR, None)
        };
        assert_eq!(
            column(
                &rendered(&root),
                "status",
                "account",
                "Inspect source files"
            ),
            expected
        );
    }
}

#[test]
fn subagent_and_shell_rows_inherit_the_session_account_and_threads_keep_their_own() {
    let root = AgentRow {
        account: account("claude1"),
        children: vec![
            child(
                0,
                ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                None,
                "Inspect source files",
                MINUTE,
            ),
            child(1, ChildKind::Shell, Some(21), "subagent command", MINUTE),
            child(0, ChildKind::Shell, Some(22), "session command", MINUTE),
            ChildRow {
                account: account("codex2"),
                ..child(0, ChildKind::Thread, None, "Codex task", MINUTE)
            },
        ],
        ..agent(10, "director", HOUR, None)
    };
    let buffer = rendered(&root);
    for name in [
        "Inspect source files",
        "subagent command",
        "session command",
    ] {
        assert_eq!(
            column(&buffer, "account", "branch", name),
            "claude1",
            "{name}"
        );
    }
    assert_eq!(column(&buffer, "account", "branch", "Codex task"), "codex2");
}

#[test]
fn subagent_account_width_and_sort_use_the_shown_session_account() {
    let label = "z-inherited-session-account";
    let root = AgentRow {
        account: account(label),
        children: vec![
            child(
                0,
                ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                None,
                "Inspect source files",
                MINUTE,
            ),
            ChildRow {
                account: account("a-codex"),
                ..child(0, ChildKind::Thread, None, "Codex task", MINUTE)
            },
        ],
        ..agent(10, "director", HOUR, None)
    };
    let state = MachineState::Answered(vec![root], HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let defaults = cell_order_placed(&placement);
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let sorts = defaults
            .iter()
            .map(|cell| {
                (
                    cell.id.clone(),
                    TreeSort {
                        column: TreeColumn::Account,
                        direction,
                    },
                )
            })
            .collect();
        let cells = cell_order_sorted(&placement, &sorts, NOW);
        let rect = area(220, 20);
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
        assert_eq!(
            column(&buffer, "account", "branch", "Inspect source files"),
            label
        );
        assert_eq!(
            column(&buffer, "account", "branch", "Codex task"),
            "a-codex"
        );
        let subagent_row = find(&buffer, "Inspect source files")
            .expect("subagent row")
            .1;
        let codex_row = find(&buffer, "Codex task").expect("thread row").1;
        assert_eq!(
            codex_row < subagent_row,
            direction == SortDirection::Ascending
        );
    }
}

#[test]
fn codex_thread_without_an_account_keeps_the_missing_value_under_a_claude_session() {
    let root = AgentRow {
        account: account("claude1"),
        children: vec![child(
            0,
            ChildKind::Thread,
            None,
            "Unrecorded Codex task",
            MINUTE,
        )],
        ..agent(10, "director", HOUR, None)
    };
    assert_eq!(
        column(
            &rendered(&root),
            "account",
            "branch",
            "Unrecorded Codex task"
        ),
        crate::constants::MISSING_VALUE
    );
}

#[test]
fn inherited_claude_work_accounts_preserve_the_frame_account_column_decision() {
    let mut rows = vec![
        AgentRow {
            account: account("claude1"),
            ..agent(10, "director", HOUR, None)
        },
        AgentRow {
            agent: Agent::Codex,
            account: account("codex2"),
            ..agent(20, "Codex session", HOUR, None)
        },
    ];
    let before = MachineState::Answered(rows.clone(), HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &before,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let baseline = account_column(&cell_order_placed(&placement));
    assert_eq!(baseline, AccountColumn::Hidden);

    rows[0].children = vec![
        child(0, ChildKind::Shell, Some(21), "session command", MINUTE),
        child(
            0,
            ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
            None,
            "Inspect source files",
            MINUTE,
        ),
        child(1, ChildKind::Shell, Some(22), "subagent command", MINUTE),
    ];
    let after = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &after,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = cell_order_placed(&placement);
    let frame_column = account_column(&cells);
    assert_eq!(
        frame_column, baseline,
        "inherited accounts introduce no new account family"
    );

    let director = cells
        .iter()
        .find(|cell| cell.row.pid == 10)
        .expect("director tile");
    let rect = area(220, 20);
    let mut buffer = Buffer::empty(rect);
    draw(
        &mut buffer,
        rect,
        director,
        &cells,
        NOW,
        frame_column,
        &FooterColors::default(),
    );
    let drawn = lines(&buffer);
    let header = drawn
        .iter()
        .find(|line| line.split_whitespace().next() == Some("session"))
        .expect("tree header");
    assert!(
        !header.split_whitespace().any(|word| word == "account"),
        "{header}"
    );
    for name in [
        "session command",
        "Inspect source files",
        "subagent command",
    ] {
        assert!(
            find(&buffer, name).is_some(),
            "the work row still draws: {name}"
        );
    }
}

#[test]
fn unassigned_and_unlabeled_role_cells_ignore_the_retired_footer_color() {
    let root = agent(10, "director", HOUR, None);
    let rows = [
        agent(20, "standalone session", MINUTE, None),
        agent(30, "unlabeled session", MINUTE, None),
        agent(40, "hierarchy worker", MINUTE, None),
        agent(50, "fallback reviewer", MINUTE, None),
    ];
    let mut cell = entry(&root, None);
    for (row, subject) in rows.iter().zip([
        LineSubject::UnassignedSession,
        LineSubject::Agent(RoleLabel::Unlabeled),
        LineSubject::Agent(RoleLabel::Stated(HierarchyRole::Impl)),
        LineSubject::Agent(RoleLabel::ReviewFallback),
    ]) {
        cell.lines.push(TreeLine {
            dispatch_label: DispatchLabel::Original,
            machine: "natedev",
            location: SessionLocation::SameMachine,
            row,
            hue: RainbowHue::Red,
            rails: Vec::new(),
            branch: Branch::Last,
            subject,
        });
    }
    let thread = child(0, ChildKind::Thread, None, "managed worker", MINUTE);
    cell.lines.push(TreeLine {
        dispatch_label: DispatchLabel::Original,
        machine:        "natedev",
        location:       SessionLocation::SameMachine,
        row:            &root,
        hue:            RainbowHue::Red,
        rails:          Vec::new(),
        branch:         Branch::Last,
        subject:        LineSubject::ManagedThread(&thread, HierarchyRole::Test),
    });
    let rect = area(104, 12);
    let colors = FooterColors::parse("role.unassigned=purple\nrole.worker=bright-cyan");
    let mut buffer = Buffer::empty(rect);
    draw(
        &mut buffer,
        rect,
        &cell,
        std::slice::from_ref(&cell),
        NOW,
        AccountColumn::Hidden,
        &colors,
    );
    let role_x = find(&buffer, "role").expect("role header").0;
    let standalone_y = find(&buffer, "standalone session")
        .expect("standalone row")
        .1;
    for offset in 0..summary::cell_width(TREE_ROLE_HEADER) {
        let cell = &buffer[(role_x + offset, standalone_y)];
        assert_eq!(cell.symbol(), " ", "standalone role stays blank");
        assert_eq!(cell.fg, Color::Reset, "blank role has no footer ink");
    }
    let plain = FooterTint::Plain.style(Style::default().fg(text_default()));
    for (name, role) in [
        ("unlabeled session", "session"),
        ("hierarchy worker", "impl"),
        ("fallback reviewer", "review"),
        ("managed worker", "test"),
    ] {
        let y = find(&buffer, name).expect("session or worker row").1;
        let drawn: String = (0..summary::cell_width(role))
            .map(|offset| buffer[(role_x + offset, y)].symbol())
            .collect();
        assert_eq!(drawn, role, "{name}");
        for offset in 0..summary::cell_width(role) {
            let drawn = &buffer[(role_x + offset, y)];
            assert_eq!(Some(drawn.fg), plain.fg, "{name}");
            assert_eq!(drawn.modifier, plain.add_modifier, "{name}");
        }
    }
}

#[test]
fn work_type_has_its_own_column_and_subagent_roles_name_the_work() {
    let root = AgentRow {
        children: vec![
            child(0, ChildKind::Shell, Some(21), "cargo test", MINUTE),
            ChildRow {
                kind: ChildKind::Subagent(crate::census::transcript::SubagentRole::from_record(
                    "general-purpose",
                    "Review source changes",
                )),
                ..child(
                    0,
                    ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                    None,
                    "Review source changes",
                    MINUTE,
                )
            },
            child(
                0,
                ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                None,
                "Do the requested work",
                MINUTE,
            ),
            child(0, ChildKind::Thread, None, "seat task", MINUTE),
        ],
        ..agent(10, "director", HOUR, None)
    };
    let buffer = rendered(&root);
    let header = (0..buffer.area.height)
        .find(|&y| {
            let line: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            line.split_whitespace().next() == Some("session") && line.contains("status")
        })
        .expect("tree header");
    let headers: String = (0..buffer.area.width)
        .map(|x| buffer[(x, header)].symbol())
        .collect();
    let words: Vec<_> = headers
        .split_whitespace()
        .filter(|word| !matches!(*word, "▲" | "▼"))
        .collect();
    assert_eq!(&words[..3], &["session", "type", "role"], "{headers}");
    for (name, expected_type, expected_role) in [
        ("cargo test", "shell", ""),
        ("Review source changes", "subagent", "review"),
        ("Do the requested work", "subagent", ""),
        ("seat task", "", "worker"),
        ("director", "", ""),
    ] {
        assert_eq!(
            column(&buffer, "type", "role", name),
            expected_type,
            "{name}"
        );
        assert_eq!(
            column(&buffer, "role", "status", name),
            expected_role,
            "{name}"
        );
    }
}

#[test]
fn equal_width_tiles_share_short_column_positions_and_two_space_gaps() {
    let roots = [
        AgentRow {
            children: vec![child(0, ChildKind::Shell, Some(11), "short", MINUTE)],
            ..agent(10, "first", HOUR, None)
        },
        AgentRow {
            children: vec![child(
                0,
                ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
                None,
                "Review a substantially longer session name",
                MINUTE,
            )],
            ..agent(20, "second", HOUR, None)
        },
    ];
    let state = MachineState::Answered(roots.into(), HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let cells = cell_order(&machines);
    let buffers: Vec<_> = cells
        .iter()
        .map(|cell| {
            let rect = area(160, 12);
            let mut buffer = Buffer::empty(rect);
            draw(
                &mut buffer,
                rect,
                cell,
                &cells,
                NOW,
                AccountColumn::Shown,
                &FooterColors::default(),
            );
            buffer
        })
        .collect();
    for header in [
        "type",
        "role",
        "status",
        "account",
        "branch",
        "directory",
        "age",
    ] {
        let positions: Vec<_> = buffers
            .iter()
            .map(|buffer| find(buffer, header).expect("drawn column").0)
            .collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] == pair[1]),
            "{header}: {positions:?}"
        );
    }
    let widest = buffers
        .iter()
        .map(|buffer| {
            let (type_x, y) = find(buffer, "type").expect("type header");
            let role_x = find(buffer, "role").expect("role header").0;
            (y + 1..buffer.area.height)
                .map(|row| {
                    (type_x..role_x)
                        .map(|x| buffer[(x, row)].symbol())
                        .collect::<String>()
                        .trim_end()
                        .chars()
                        .count()
                })
                .max()
                .expect("data rows")
                .max("type".len())
        })
        .max()
        .expect("tile fixtures");
    for buffer in &buffers {
        let type_x = find(buffer, "type").expect("type header").0;
        let role_x = find(buffer, "role").expect("role header").0;
        assert_eq!(
            usize::from(role_x - type_x),
            widest + 2,
            "two-space gap after the widest type in the shared screen column"
        );
    }
}
