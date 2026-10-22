//! One tree per tile, with shared columns and the frame's account decision.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use summary::SummaryFrame;

use super::*;
use crate::census::AccountColumn;
use crate::census::AgentActivity;
use crate::census::RoleFallback;
use crate::census::accounts::AccountsTable;
use crate::census::hierarchy::ReviewerState;
use crate::census::worker_dispatch::WorkerTask;
use crate::constants::TREE_ACCOUNT_HEADER;
use crate::constants::TREE_AGE_HEADER;
use crate::constants::TREE_BRANCH_HEADER;
use crate::constants::TREE_DIRECTORY_HEADER;
use crate::constants::TREE_ROLE_HEADER;
use crate::constants::TREE_SESSION_HEADER;
use crate::constants::TREE_STATUS_HEADER;
use crate::sort::AccountsSort;
use crate::sort::SummarySort;

/// A remote probe's account value, independent of the runner's home directory.
fn home(label: &str) -> Account {
    serde_json::from_value(serde_json::json!({
        "home": format!("~/.{label}"), "label": label,
    }))
    .expect("the probe account fixture parses")
}

/// The mockup: a showrunner, two units and a thread held by the first unit.
fn mockup(claude_accounts: [&str; 3]) -> MachineState {
    let mut root = agent(10, "showrunner", 24 * HOUR, None);
    root.session_id = SessionIdentity::Claude("production".to_string());
    root.account = home(claude_accounts[0]);
    root.branch = Some("main".to_string());
    let mut dashboard = agent(20, "dashboard", 6 * HOUR, None);
    dashboard.session_id = SessionIdentity::Claude("dashboard-id".to_string());
    dashboard.account = home(claude_accounts[1]);
    dashboard.branch = Some("dashboard".to_string());
    let mut diagrams = agent(30, "diagrams", 4 * HOUR, None);
    diagrams.session_id = SessionIdentity::Claude("diagrams-id".to_string());
    diagrams.account = home(claude_accounts[2]);
    diagrams.activity = AgentActivity::Idle;
    let thread = ChildRow {
        worker_task: WorkerTask::Unrecorded,
        thread_id: CodexThreadId::Recorded("dashboard-impl-id".to_string()),
        account: home("codex1"),
        ..child(
            0,
            ChildKind::Thread,
            None,
            "hanadocs-dashboard-impl",
            5 * HOUR,
        )
    };
    let carrier = AgentRow {
        role_fallback: RoleFallback::None,
        agent: Agent::CodexThreadHost,
        account: home("codex1"),
        directory: "~/rust/dashboard".to_string(),
        branch: Some("dashboard".to_string()),
        children: vec![thread],
        ..agent(40, "app-server", 12 * HOUR, None)
    };
    let edges = [
        ("production", "dashboard-id", HierarchyRole::Unit),
        ("production", "diagrams-id", HierarchyRole::Unit),
        ("dashboard-id", "dashboard-impl-id", HierarchyRole::Impl),
    ]
    .into_iter()
    .map(|(manager, member, role)| HierarchyEdge {
        reviewer: ReviewerState::NotPool,
        manager: manager.to_string(),
        member: member.to_string(),
        role,
    })
    .collect();
    MachineState::Answered(
        vec![root, dashboard, diagrams, carrier],
        HierarchyEdges(edges).into(),
    )
}

/// Draw a cell at its requested height with the explicit frame decision.
fn rendered(
    cell: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    width: u16,
    account_column: AccountColumn,
) -> Buffer {
    let rect = area(width, height(cell, width, NOW, account_column));
    let mut buffer = Buffer::empty(rect);
    draw(
        &mut buffer,
        rect,
        cell,
        cells,
        NOW,
        account_column,
        &FooterColors::default(),
    );
    buffer
}

/// Read a value between the column starts in the header, rather than by spacing.
fn column_value(buffer: &Buffer, header: &str, row: u16, headers: &[&str]) -> String {
    // Read the last age from drawn cells: right alignment puts its values before
    // the header start, so that start cannot delimit the preceding directory.
    let mut age_parts = Vec::new();
    let mut cursor = buffer.area.right();
    while cursor > buffer.area.x {
        while cursor > buffer.area.x && buffer[(cursor - 1, row)].symbol() == " " {
            cursor -= 1;
        }
        let end = cursor;
        while cursor > buffer.area.x && buffer[(cursor - 1, row)].symbol() != " " {
            cursor -= 1;
        }
        let part: String = (cursor..end).map(|x| buffer[(x, row)].symbol()).collect();
        if part.len() <= 1
            || !part.ends_with(['d', 'h', 'm', 's'])
            || !part.as_bytes()[..part.len() - 1]
                .iter()
                .all(u8::is_ascii_digit)
        {
            break;
        }
        age_parts.push((cursor, part));
    }
    age_parts.reverse();
    if header == TREE_AGE_HEADER {
        return age_parts
            .iter()
            .map(|(_, part)| part.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    let header_at = |label| {
        find_all(buffer, label)
            .into_iter()
            .find(|&at| buffer[at].fg == label_color())
    };
    let (start, header_row) = header_at(header).expect("the column has a header");
    let end = headers
        .iter()
        .filter_map(|label| header_at(label))
        .filter(|&(x, y)| y == header_row && x > start)
        .map(|(x, _)| x)
        .min()
        .unwrap_or(buffer.area.width);
    let end = if header == TREE_DIRECTORY_HEADER {
        age_parts.first().map_or(end, |(x, _)| end.min(*x))
    } else {
        end
    };
    (start..end)
        .map(|x| buffer[(x, row)].symbol())
        .collect::<String>()
        .trim()
        .to_string()
}

/// All tree header names, including the columns absent from a compressed view.
const HEADERS: [&str; 8] = [
    TREE_SESSION_HEADER,
    TREE_TYPE_HEADER,
    TREE_ROLE_HEADER,
    TREE_STATUS_HEADER,
    TREE_ACCOUNT_HEADER,
    TREE_BRANCH_HEADER,
    TREE_DIRECTORY_HEADER,
    TREE_AGE_HEADER,
];

/// The row containing a named tree line.
fn row_of(buffer: &Buffer, name: &str) -> u16 {
    find(buffer, name).expect("the tree line is drawn").1
}

#[test]
fn a_compressed_showrunner_starts_with_aligned_tree_columns() {
    let state = mockup(["claude2", "claude1", "claude3"]);
    let cells = natedev_cells(&state);
    assert_eq!(cells.len(), 1, "the thread host titles no separate cell");
    let frame_account_column = account_column(&cells);
    let buffer = rendered(&cells[0], &cells, 80, frame_account_column);
    let text = lines(&buffer);

    assert_eq!(text.len(), 4);
    assert_eq!(
        text[0].split_whitespace().collect::<Vec<_>>(),
        ["session", "type", "role", "status", "account", "age", "▼"]
    );
    assert_eq!(
        text[1].split_whitespace().collect::<Vec<_>>(),
        ["├─", "dashboard", "unit", "thinking", "claude1", "6h"]
    );
    assert_eq!(
        text[2].split_whitespace().collect::<Vec<_>>(),
        [
            "│",
            "└─",
            "hanadocs-dashboard-impl",
            "impl",
            "idle",
            "codex1",
            "5h"
        ]
    );
    assert_eq!(
        text[3].split_whitespace().collect::<Vec<_>>(),
        ["└─", "diagrams", "unit", "idle", "claude3", "4h"]
    );
    assert_eq!(
        find(&buffer, "session").expect("header").0,
        find(&buffer, "dashboard").expect("name").0
    );
    for (name, role, status, account, age) in [
        ("dashboard", "unit", "thinking", "claude1", "6h"),
        ("hanadocs-dashboard-impl", "impl", "idle", "codex1", "5h"),
        ("diagrams", "unit", "idle", "claude3", "4h"),
    ] {
        let row = row_of(&buffer, name);
        for (header, expected) in [
            ("role", role),
            ("status", status),
            ("account", account),
            ("age", age),
        ] {
            assert_eq!(
                column_value(&buffer, header, row, &HEADERS),
                expected,
                "{name} {header}"
            );
        }
    }
}

#[test]
fn a_full_showrunner_puts_the_tree_header_directly_after_its_place() {
    let state = mockup(["claude2", "claude1", "claude3"]);
    let cells = natedev_cells(&state);
    let buffer = rendered(&cells[0], &cells, 220, account_column(&cells));
    let text = lines(&buffer);

    assert!(text[0].contains(" · claude2 · thinking · "));
    assert_eq!(text[1], " main · ~/rust/showrunner");
    assert_eq!(
        text[2].split_whitespace().collect::<Vec<_>>(),
        [
            "session",
            "type",
            "role",
            "status",
            "account",
            "branch",
            "directory",
            "age",
            "▼"
        ]
    );
    assert_eq!(text.len(), 6);
    assert_eq!(row_of(&buffer, "dashboard"), 3);
    assert_eq!(column_value(&buffer, "branch", 3, &HEADERS), "dashboard");
    assert_eq!(
        column_value(&buffer, "directory", 3, &HEADERS),
        "~/rust/dashboard"
    );
    assert_eq!(column_value(&buffer, "branch", 4, &HEADERS), "dashboard");
    assert_eq!(
        column_value(&buffer, "directory", 4, &HEADERS),
        "~/rust/dashboard"
    );
    assert_eq!(column_value(&buffer, "branch", 5, &HEADERS), "—");
}

#[test]
fn tree_headers_names_roles_accounts_and_statuses_keep_their_colors() {
    let state = mockup(["claude2", "claude1", "claude3"]);
    let cells = natedev_cells(&state);
    for width in [80, 220] {
        let buffer = rendered(&cells[0], &cells, width, account_column(&cells));
        for header in HEADERS {
            if let Some(at) = find(&buffer, header) {
                for offset in 0..u16::try_from(header.len()).expect("header width") {
                    assert_eq!(
                        buffer[(at.0 + offset, at.1)].fg,
                        label_color(),
                        "{header} at {width}"
                    );
                }
            }
        }
        for (name, pid, status_role) in [
            ("dashboard", 20, Role::Busy),
            ("hanadocs-dashboard-impl", 20, Role::Idle),
            ("diagrams", 30, Role::Idle),
        ] {
            let at = find(&buffer, name).expect("the name is drawn");
            assert_eq!(Some(buffer[at].fg), name_style(&cells, "natedev", pid).fg);
            for header in ["role", "account"] {
                let x = find(&buffer, header).expect("the column is drawn").0;
                assert_eq!(buffer[(x, at.1)].fg, text_default(), "{name} {header}");
            }
            let status_x = find(&buffer, "status").expect("status header").0;
            assert_eq!(Some(buffer[(status_x, at.1)].fg), status_role.style().fg);
        }
    }
}

#[test]
fn one_account_per_program_hides_accounts_and_keeps_the_program_heading() {
    let single = mockup(["claude2", "claude2", "claude2"]);
    let multiple = mockup(["claude2", "claude1", "claude3"]);
    for (state, expected) in [
        (&single, AccountColumn::Hidden),
        (&multiple, AccountColumn::Shown),
    ] {
        let cells = natedev_cells(state);
        let frame_account_column = account_column(&cells);
        assert_eq!(frame_account_column, expected);
        for width in [80, 220] {
            let buffer = rendered(&cells[0], &cells, width, frame_account_column);
            let text = lines(&buffer).join("\n");
            match expected {
                AccountColumn::Hidden => {
                    assert!(find(&buffer, "account").is_none());
                    assert!(!text.contains("claude2"));
                    assert!(!text.contains("codex1"));
                },
                AccountColumn::Shown => assert!(find(&buffer, "account").is_some()),
            }
            if width == 220 {
                let heading = &lines(&buffer)[0];
                let program = match expected {
                    AccountColumn::Hidden => " · claude · ",
                    AccountColumn::Shown => " · claude2 · ",
                };
                assert!(heading.contains(program), "{heading}");
            }
        }
    }
}

#[test]
fn a_thread_account_and_another_cells_root_share_the_frame_decision() {
    let local = mockup(["claude2"; 3]);
    let remote = MachineState::Answered(
        vec![AgentRow {
            role_fallback: crate::census::RoleFallback::None,
            agent: Agent::Codex,
            account: home("codex2"),
            ..agent(50, "isolated", HOUR, None)
        }],
        HierarchyEdges::default().into(),
    );
    let cells = cell_order(&[
        Machine {
            name:  "natedev",
            state: &local,
        },
        Machine {
            name:  "mac",
            state: &remote,
        },
    ]);
    assert_eq!(cells.len(), 2);
    let frame_account_column = account_column(&cells);
    assert_eq!(frame_account_column, AccountColumn::Shown);
    // Call `rendered` only for `cells[0]`, as a grid with no room for `cells[1]` would.
    let buffer = rendered(&cells[0], &cells, 220, frame_account_column);
    assert!(find(&buffer, "isolated").is_none());
    assert!(find(&buffer, "account").is_some());
    assert!(lines(&buffer)[0].contains(" · claude2 · "));
}

/// Work below a member, including dropped process nodes and a session subtree.
fn working_group() -> MachineState {
    let mut worker = agent(20, "unit-member", 2 * HOUR, Some(10));
    worker.account = home("claude1");
    worker.children = vec![
        child(
            0,
            ChildKind::Direct(Agent::Codex),
            Some(100),
            "mcp-hidden",
            HOUR,
        ),
        ChildRow {
            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
            timer: Some(Timer {
                started:  NOW - MINUTE,
                deadline: NOW + MINUTE,
            }),
            ..child(0, ChildKind::Shell, Some(101), "clock job", 10 * MINUTE)
        },
        child(
            1,
            ChildKind::UnderShell(Agent::Codex),
            Some(102),
            "app-server-hidden",
            10 * MINUTE,
        ),
        ChildRow {
            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
            account: home("codex1"),
            ..child(2, ChildKind::Thread, None, "unheld worker", 9 * MINUTE)
        },
        child(
            0,
            ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
            None,
            "review worker",
            8 * MINUTE,
        ),
        child(1, ChildKind::Shell, Some(103), "nested command", 7 * MINUTE),
        child(
            0,
            ChildKind::Session(Agent::Claude),
            Some(104),
            "session-hidden",
            6 * MINUTE,
        ),
        child(
            1,
            ChildKind::Shell,
            Some(105),
            "session-work-hidden",
            5 * MINUTE,
        ),
        child(
            0,
            ChildKind::Detached(Agent::Codex),
            Some(106),
            "detached-hidden",
            4 * MINUTE,
        ),
    ];
    let root = AgentRow {
        role_fallback: RoleFallback::None,
        account: home("claude2"),
        children: vec![child(0, ChildKind::Shell, Some(107), "root command", HOUR)],
        ..agent(10, "production", 3 * HOUR, None)
    };
    MachineState::Answered(vec![root, worker], HierarchyEdges::default().into())
}

#[test]
fn work_lines_nest_below_their_member_and_skip_process_and_session_nodes() {
    let state = working_group();
    let cells = natedev_cells(&state);
    let buffer = rendered(&cells[0], &cells, 220, account_column(&cells));
    let text = lines(&buffer).join("\n");
    for hidden in [
        "mcp-hidden",
        "app-server-hidden",
        "detached-hidden",
        "session-hidden",
        "session-work-hidden",
    ] {
        assert!(!text.contains(hidden), "{hidden} is not a tree line");
    }
    for (name, role, status, account, age) in [
        ("unit-member", "session", "thinking", "claude1", "2h"),
        ("clock job", "", "◑ 1:00", "claude1", "10m"),
        ("unheld worker", "worker", "idle", "codex1", "9m"),
        ("review worker", "", "idle", "claude1", "8m"),
        ("nested command", "", "running", "claude1", "7m"),
        ("root command", "", "running", "claude2", "1h"),
    ] {
        let y = row_of(&buffer, name);
        assert_eq!(find_all(&buffer, name).len(), 1, "{name} is drawn once");
        for (header, expected) in [
            ("role", role),
            ("status", status),
            ("account", account),
            ("age", age),
        ] {
            assert_eq!(
                column_value(&buffer, header, y, &HEADERS),
                expected,
                "{name} {header}"
            );
        }
        if role != "session" {
            for header in ["branch", "directory"] {
                assert_eq!(column_value(&buffer, header, y, &HEADERS), "—");
            }
            let at = find(&buffer, name).expect("work name");
            let expected_color = match role {
                "worker" => buffer[find(&buffer, "unit-member").expect("parent session")].fg,
                _ => text_default(),
            };
            assert_eq!(
                buffer[at].fg, expected_color,
                "{name} uses its row kind's color"
            );
        }
    }
    let rows = lines(&buffer);
    assert!(rows[usize::from(row_of(&buffer, "unit-member"))].contains("├─ unit-member"));
    assert!(rows[usize::from(row_of(&buffer, "clock job"))].contains("│  ├─ clock job"));
    assert!(rows[usize::from(row_of(&buffer, "unheld worker"))].contains("│  │  └─ unheld worker"));
    assert!(
        rows[usize::from(row_of(&buffer, "nested command"))].contains("│     └─ nested command")
    );
    assert!(
        rows.last()
            .expect("last work line")
            .contains("└─ root command")
    );
    let status_x = find(&buffer, "status").expect("status column").0;
    for (name, role) in [
        ("clock job", Role::Shell),
        ("unheld worker", Role::Idle),
        ("review worker", Role::Idle),
    ] {
        assert_eq!(
            Some(buffer[(status_x, row_of(&buffer, name))].fg),
            role.style().fg
        );
    }
}

#[test]
fn no_tile_restores_the_removed_children_table_headers() {
    for state in [mockup(["claude2", "claude1", "claude3"]), working_group()] {
        let cells = natedev_cells(&state);
        for cell in &cells {
            for width in [80, 220] {
                let buffer = rendered(cell, &cells, width, account_column(&cells));
                for label in runs_in(&buffer, label_color()) {
                    assert!(
                        !["pid", "via", "runs", "name"].contains(&label.as_str()),
                        "old table label {label}"
                    );
                }
                assert!(
                    !lines(&buffer)
                        .iter()
                        .any(|line| line.contains(" via ") || line.contains(" runs "))
                );
            }
        }
    }
}

#[test]
fn tree_headers_and_lines_keep_blank_margins_before_tile_borders() {
    let row = AgentRow {
        role_fallback: RoleFallback::None,
        children: vec![
            child(
                0,
                ChildKind::Shell,
                Some(11),
                &"long command ".repeat(7),
                10 * MINUTE,
            ),
            child(0, ChildKind::Shell, Some(12), "second command", 10 * MINUTE),
        ],
        ..agent(10, "worker", HOUR, None)
    };
    let cell = entry(&row, None);
    let indent = summary::cell_width(SECTION_HEADER_INDENT);
    for account_column in [AccountColumn::Hidden, AccountColumn::Shown] {
        let full_width = (70..240)
            .find(|&width| {
                matches!(
                    CellView::at_width(&cell, width, NOW, account_column),
                    CellView::Full(_)
                )
            })
            .expect("the tree fits at a wide width");
        assert_eq!(
            CellView::at_width(&cell, full_width - 1, NOW, account_column),
            CellView::Compressed,
            "the full tree uses all available columns"
        );
        for width in [70, full_width] {
            let rows = u16::try_from(height(&cell, width, NOW, account_column))
                .expect("the tree height fits");
            let outer = Rect::new(0, 0, width + 2, rows + 2);
            let inner = Rect::new(1, 1, width, rows);
            let mut buffer = Buffer::empty(outer);
            ratatui::widgets::Block::bordered().render(outer, &mut buffer);
            draw(
                &mut buffer,
                inner,
                &cell,
                &[],
                NOW,
                account_column,
                &FooterColors::default(),
            );
            let header_y = row_of(&buffer, "session");
            assert_eq!(
                header_y,
                if width == full_width { 3 } else { 1 },
                "exercise both the full and compressed views"
            );
            for y in header_y..inner.bottom() {
                assert_eq!(buffer[(outer.right() - 1, y)].symbol(), "│");
                for offset in 0..indent {
                    assert_eq!(buffer[(inner.x + offset, y)].symbol(), " ");
                    assert_eq!(
                        buffer[(inner.right() - 1 - offset, y)].symbol(),
                        " ",
                        "tree row {y} at width {width} keeps its right margin"
                    );
                }
                let age_x = find(&buffer, "age").expect("age header").0;
                assert!(
                    (age_x..inner.right() - indent).any(|x| buffer[(x, y)].symbol() != " "),
                    "the header and each age remain visible before the margin"
                );
            }
        }
    }
}

#[test]
fn an_empty_tree_has_only_the_nothing_running_note_in_either_view() {
    let row = agent(10, "empty", HOUR, None);
    let cell = entry(&row, None);
    for frame_account_column in [AccountColumn::Hidden, AccountColumn::Shown] {
        for width in [20, 220] {
            let buffer = rendered(&cell, &[], width, frame_account_column);
            assert_eq!(find_all(&buffer, "nothing running").len(), 1);
            assert!(find(&buffer, "session").is_none());
            assert!(find(&buffer, "role").is_none());
            assert!(find(&buffer, "account").is_none());
            if width == 20 {
                assert_eq!(lines(&buffer), [" nothing running"]);
            } else {
                assert_eq!(lines(&buffer).last().expect("note"), " nothing running");
                assert_eq!(buffer.area.height, 3);
            }
        }
    }
}

/// The tile widths the height tests cover.
const TESTED_WIDTHS: RangeInclusive<usize> = 10..=140;

/// Layout transitions and their neighbors, including the tested range's ends.
fn layout_widths(cell: &AgentEntry<'_>, account_column: AccountColumn) -> BTreeSet<u16> {
    let indent = usize::from(summary::cell_width(SECTION_HEADER_INDENT));
    let mut full = header_line(cell.row, cell.machine, NOW, account_column)
        .width()
        .max(place_line(cell.row).width());
    if !cell.lines.is_empty() {
        full = full.max(
            table_width(&tree_widths(
                &cell.lines,
                TreeLayout::Full.columns(account_column),
                NOW,
                TreeSort::default(),
                SessionHeaderLead::OverBranch,
                &TreeSizing::Local,
            )) + 2 * indent,
        );
    }
    if TESTED_WIDTHS.contains(&full) {
        let full = u16::try_from(full).expect("the full boundary fits");
        assert!(
            matches!(
                CellView::at_width(cell, full, NOW, account_column),
                CellView::Full(_)
            ),
            "{} fits the full view at width {full}",
            cell.row.name
        );
        assert_eq!(
            CellView::at_width(cell, full - 1, NOW, account_column),
            CellView::Compressed,
            "{} needs the full boundary width {full}",
            cell.row.name
        );
    }

    let columns = TreeLayout::Compressed.columns(account_column);
    let gaps = columns.len().saturating_sub(1);
    let spacing = usize::from(TABLE_COLUMN_SPACING);
    let widths = tree_widths(
        &cell.lines,
        columns,
        NOW,
        TreeSort::default(),
        SessionHeaderLead::OverBranch,
        &TreeSizing::Local,
    );
    let fixed = widths[1..]
        .iter()
        .map(|&width| usize::from(width))
        .sum::<usize>()
        + spacing * gaps;
    let lead = cell
        .lines
        .iter()
        .map(|line| usize::from(summary::cell_width(&line.lead())))
        .max()
        .unwrap_or(0);
    let boundaries = [full, fixed + 2 * indent, fixed + lead + 2 * indent];
    boundaries
        .into_iter()
        .chain((1..=spacing).map(|step| step * gaps + 1 + 2 * indent))
        .flat_map(|boundary| [boundary.saturating_sub(1), boundary, boundary + 1])
        .chain([*TESTED_WIDTHS.start(), *TESTED_WIDTHS.end()])
        .map(|width| {
            u16::try_from(width.clamp(*TESTED_WIDTHS.start(), *TESTED_WIDTHS.end()))
                .expect("the test width fits")
        })
        .collect()
}

/// Check each cell's layout boundaries for one fixture.
fn assert_heights_at_widths(state: &MachineState) {
    let cells = natedev_cells(state);
    let frame_account_column = account_column(&cells);
    for cell in &cells {
        for width in layout_widths(cell, frame_account_column) {
            let rows = height(cell, width, NOW, frame_account_column);
            let rect = area(width, rows + 3);
            let mut buffer = Buffer::empty(rect);
            draw(
                &mut buffer,
                rect,
                cell,
                &cells,
                NOW,
                frame_account_column,
                &FooterColors::default(),
            );
            let text = lines(&buffer);
            let last = text.iter().rposition(|line| !line.is_empty());
            assert_eq!(
                last.map(|index| index + 1),
                Some(rows),
                "{} at width {width}: {text:#?}",
                cell.row.name
            );
        }
    }
}

#[test]
fn tree_heights_match_drawn_rows_at_layout_widths() {
    assert_heights_at_widths(&mockup(["claude2", "claude1", "claude3"]));
}

#[test]
fn tree_heights_without_accounts_match_drawn_rows_at_layout_widths() {
    assert_heights_at_widths(&mockup(["claude2"; 3]));
}

#[test]
fn tree_heights_with_nested_work_match_drawn_rows_at_layout_widths() {
    assert_heights_at_widths(&boss_group());
}

fn summary_account_state(frame_account_column: AccountColumn) -> MachineState {
    let second_account = match frame_account_column {
        AccountColumn::Shown => "claude3",
        AccountColumn::Hidden => "claude2",
    };
    MachineState::Answered(
        vec![
            AgentRow {
                role_fallback: crate::census::RoleFallback::None,
                account: home("claude2"),
                ..agent(10, "alpha", 3 * HOUR, None)
            },
            AgentRow {
                role_fallback: crate::census::RoleFallback::None,
                account: home(second_account),
                ..agent(20, "beta", 2 * HOUR, None)
            },
            AgentRow {
                role_fallback: crate::census::RoleFallback::None,
                agent: Agent::Codex,
                account: home("codex1"),
                service_tier: ServiceTier::Fast,
                ..agent(30, "gamma", HOUR, None)
            },
            AgentRow {
                account: home("claude2"),
                ..agent(11, "alpha child", MINUTE, Some(10))
            },
            AgentRow {
                account: home(second_account),
                ..agent(21, "beta child", MINUTE, Some(20))
            },
            AgentRow {
                agent: Agent::Codex,
                account: home("codex1"),
                ..agent(31, "gamma child", MINUTE, Some(30))
            },
        ],
        HierarchyEdges::default().into(),
    )
}

#[test]
fn summary_agent_values_follow_the_same_frame_account_decision() {
    for frame_account_column in [AccountColumn::Shown, AccountColumn::Hidden] {
        let state = summary_account_state(frame_account_column);
        let machines = [Machine {
            name:  "natedev",
            state: &state,
        }];
        let placement = AgentPlacement::from_machines(&machines);
        let cells = cell_order_placed(&placement);
        assert_eq!(account_column(&cells), frame_account_column);
        let width = summary::width(
            &machines,
            &placement,
            NOW,
            frame_account_column,
            SummarySort::default(),
            &AccountsTable::NotConfigured,
            AccountsSort::SourceOrder,
        );
        let rect = area(
            width,
            summary::height(&machines, &placement, &AccountsTable::NotConfigured),
        );
        let mut buffer = Buffer::empty(rect);
        summary::draw(
            &mut buffer,
            rect,
            &SummaryFrame {
                accounts_sort:  AccountsSort::SourceOrder,
                accounts:       &AccountsTable::NotConfigured,
                machines:       &machines,
                placement:      &placement,
                cells:          &cells,
                now:            NOW,
                account_column: frame_account_column,
                footer_colors:  &FooterColors::default(),
                sort:           SummarySort::default(),
            },
        );
        for (name, shown, hidden) in [
            ("alpha", "claude2", "claude"),
            ("beta", "claude3", "claude"),
            ("gamma", "codex1 fast", "codex fast"),
        ] {
            let expected = match frame_account_column {
                AccountColumn::Shown => shown,
                AccountColumn::Hidden => hidden,
            };
            assert_eq!(
                column_value(
                    &buffer,
                    "account",
                    row_of(&buffer, name),
                    &crate::constants::SUMMARY_HEADERS
                ),
                expected
            );
        }
        let widest = lines(&buffer)
            .iter()
            .map(|line| line.chars().count())
            .max()
            .expect("summary rows");
        assert_eq!(
            usize::from(width),
            widest,
            "summary width includes account labels"
        );
    }
}
