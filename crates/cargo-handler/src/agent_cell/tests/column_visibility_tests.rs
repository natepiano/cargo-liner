//! Column visibility is measured from drawn rows, including real grid clicks.

use std::collections::HashSet;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::CellPoint;
use tui_pane::TerminalApp;

use super::*;
use crate::app::App;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::ShellTimerObservation;
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::render;
use crate::sort;
use crate::sort::HeaderHit;
use crate::sort::SortDirection;

const METADATA_WIDTH_MIN: u16 = 55;
const METADATA_WIDTH_END: u16 = 180;
const METADATA_COLUMN_STAGES: [MetadataColumns; 3] = [
    MetadataColumns::ModelAndEffort,
    MetadataColumns::ModelOnly,
    MetadataColumns::Neither,
];

#[derive(Clone, Copy, Debug)]
enum MetadataFit {
    Local,
    SharedColumn,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum MetadataColumns {
    ModelAndEffort,
    ModelOnly,
    Neither,
}

#[test]
fn waiting_workers_keep_whole_status_and_core_columns_at_phase_three_widths() {
    let mut root = agent(10, "owner-with-a-long-session-name", HOUR, None);
    root.account =
        serde_json::from_value(serde_json::json!({"home":"~/.claude1","label":"claude1"}))
            .expect("account");
    let mut worker = child(
        0,
        ChildKind::Thread,
        None,
        "worker-with-a-long-session-name",
        MINUTE,
    );
    worker.account = root.account.clone();
    root.children.push(worker);
    let mut cell = entry(&root, None);
    cell.lines[0].subject = LineSubject::ManagedThread(
        &root.children[0],
        HierarchyRole::Impl,
        RowIdentity {
            machine: "natedev",
            pid:     root.pid,
        },
    );
    cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Waiting);
    for width in [60, 70, 90, 92] {
        let buffer = drawn_into(&cell, &[], width, 12);
        let columns = header_spans(&cell, area(width, 12), NOW, AccountColumn::Shown);
        for column in [
            TreeColumn::Account,
            TreeColumn::Role,
            TreeColumn::Activity,
            TreeColumn::Status,
        ] {
            assert!(
                columns.iter().any(|span| span.column == column),
                "width={width}: {:?}",
                lines(&buffer)
            );
        }
        let (x, y) = find(&buffer, "waiting").expect("the whole status word");
        assert_eq!(
            (x..x + 7)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>(),
            "waiting"
        );
        for word in ["claude1", "worker", "impl"] {
            assert!(
                lines(&buffer).iter().any(|line| line.contains(word)),
                "width={width} {word}: {:?}",
                lines(&buffer)
            );
        }
    }
}

fn column_rows() -> Vec<AgentRow> {
    let mut owner = agent(10, "owner", HOUR, None);
    owner.children = vec![child(
        0,
        ChildKind::Subagent(SubagentRole::Reviewer),
        None,
        "inspect-widget",
        MINUTE,
    )];
    vec![owner, agent(20, "free-session", MINUTE, None)]
}

fn column_forest() -> MachineState {
    MachineState::Answered(column_rows(), HierarchyEdges::default().into())
}

#[test]
fn long_sessions_keep_the_four_core_columns_before_their_names() {
    let mut rows = column_rows();
    rows[0].name = "natedev-showrunner-with-a-very-long-session-name-that-must-yield-space-to-the-activity-column".into();
    rows[0].branch = Some("feature/long-branch-name".into());
    rows[1].branch = rows[0].branch.clone();
    rows[0].session_id = SessionIdentity::Claude("showrunner-session".into());
    rows[1].session_id = SessionIdentity::Claude("unit-session".into());
    rows[0].model = ModelName::Opus;
    rows[0].effort = ReasoningEffort::High;
    rows[0].account = serde_json::from_value(serde_json::json!({
        "home":"~/.claude1", "label":"claude1"
    }))
    .expect("account");
    let state = MachineState::Answered(
        rows,
        HierarchyEdges(vec![crate::census::HierarchyEdge {
            manager: "showrunner-session".into(),
            member:  "unit-session".into(),
            role:    HierarchyRole::Unit,
        }])
        .into(),
    );
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells: Vec<_> = tile_order_sorted(&placement, &HashMap::new(), NOW)
        .into_iter()
        .map(|mut tile| {
            tile.apply_expansion(&HandlerExpansion::default());
            tile
        })
        .filter_map(|tile| match tile {
            AgentTile::Agent(entry) => Some(entry),
            AgentTile::Unassigned(_) => None,
        })
        .collect();
    let owner = cells.iter().find(|cell| cell.row.pid == 10).expect("owner");
    for width in [60, 70, 90, 92] {
        let buffer = drawn_into(owner, &cells, width, 12);
        let spans = header_spans(owner, area(width, 12), NOW, AccountColumn::Shown);
        let columns: Vec<_> = spans.iter().map(|span| span.column).collect();
        assert_eq!(
            columns,
            [
                TreeColumn::Session,
                TreeColumn::Account,
                TreeColumn::Role,
                TreeColumn::Activity,
                TreeColumn::Status
            ],
            "{}",
            lines(&buffer).join("\n")
        );
        assert!(
            lines(&buffer)
                .iter()
                .any(|row| row.contains("natedev-") && row.contains('…')),
            "{}",
            lines(&buffer).join("\n")
        );
    }
    assert_column_drop_sequence(owner, &cells);
}

#[test]
fn the_c320_unassigned_tile_keeps_activity_at_90_columns() {
    let mut row = agent(
        10,
        "unassigned-session-with-a-very-long-name-that-must-yield-space-to-the-activity-column",
        HOUR,
        None,
    );
    row.account =
        serde_json::from_value(serde_json::json!({"home":"~/.claude1","label":"claude1"}))
            .expect("account");
    row.branch = Some("feature/long-branch".into());
    let state = MachineState::Answered(vec![row], HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let pool = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(pool) => Some(pool),
            AgentTile::Agent(_) => None,
        })
        .expect("unassigned pool");
    let rect = area(90, 12);
    let mut buffer = Buffer::empty(rect);
    draw_unassigned(
        &mut buffer,
        rect,
        pool,
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    let columns: Vec<_> = unassigned_header_spans(pool, rect, NOW, AccountColumn::Shown)
        .into_iter()
        .map(|span| span.column)
        .collect();
    assert_eq!(
        columns,
        [
            TreeColumn::Session,
            TreeColumn::Account,
            TreeColumn::Role,
            TreeColumn::Activity,
            TreeColumn::Status
        ]
    );
    assert!(
        lines(&buffer)
            .iter()
            .any(|line| line.contains("unassigned-session") && line.contains('…'))
    );
}

fn assert_column_drop_sequence(owner: &AgentEntry<'_>, cells: &[AgentEntry<'_>]) {
    let full = vec![
        TreeColumn::Session,
        TreeColumn::Account,
        TreeColumn::Role,
        TreeColumn::Activity,
        TreeColumn::Status,
        TreeColumn::Branch,
        TreeColumn::Type,
        TreeColumn::Model,
        TreeColumn::Effort,
        TreeColumn::Directory,
    ];
    let mut previous = full.clone();
    let mut dropped = Vec::new();
    for width in (10..=260).rev() {
        let buffer = drawn_into(owner, cells, width, 12);
        let spans = header_spans(owner, area(width, 12), NOW, AccountColumn::Shown);
        let columns: Vec<_> = spans.iter().map(|span| span.column).collect();
        if width == 260 {
            assert_eq!(columns, full);
        }
        for column in &previous {
            if !columns.contains(column) {
                dropped.push(*column);
            }
        }
        assert!(
            columns.iter().all(|column| previous.contains(column)),
            "columns return at width {width}: {}",
            lines(&buffer).join("\n")
        );
        previous = columns;
    }
    assert_eq!(
        dropped,
        [
            TreeColumn::Effort,
            TreeColumn::Model,
            TreeColumn::Directory,
            TreeColumn::Type,
            TreeColumn::Branch,
            TreeColumn::Account,
            TreeColumn::Role,
            TreeColumn::Activity,
            TreeColumn::Status
        ]
    );
}

#[test]
fn unassigned_tiles_omit_headers_for_columns_without_values() {
    let state = column_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let pool = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(pool) => Some(pool),
            AgentTile::Agent(_) => None,
        })
        .expect("unassigned tile");
    for width in [65, 220] {
        let rect = area(width, 12);
        let mut buffer = Buffer::empty(rect);
        draw_unassigned(
            &mut buffer,
            rect,
            pool,
            NOW,
            AccountColumn::Hidden,
            &FooterColors::default(),
        );
        assert!(find(&buffer, "free-session").is_some());
        for absent in ["type", "age", "branch"] {
            assert!(
                find(&buffer, absent).is_none(),
                "{absent}: {}",
                lines(&buffer).join("\n")
            );
        }
        // Select the actual status header after the empty columns leave.
        let (x, y) = find(&buffer, "status").expect("status header");
        let spans = unassigned_header_spans(pool, rect, NOW, AccountColumn::Hidden);
        assert_eq!(
            sort::header_hit(
                &spans,
                CellPoint {
                    column: x,
                    row:    y,
                }
            ),
            HeaderHit::Column(TreeColumn::Status)
        );
        assert!(!spans.iter().any(|span| matches!(
            span.column,
            TreeColumn::Type | TreeColumn::Age | TreeColumn::Branch
        )));
    }
}

#[test]
fn tiles_keep_type_and_role_headers_when_their_rows_have_values() {
    let state = column_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = cell_order_placed(&placement);
    let owner = cells
        .iter()
        .find(|cell| cell.row.pid == 10)
        .expect("work tile");
    for width in [65, 220] {
        let buffer = drawn_into(owner, &cells, width, 12);
        let y = find(&buffer, "inspect-widget").expect("work row").1;
        for (header, word) in [
            ("type", "subagent"),
            ("role", "worker"),
            ("activity", "reviewer"),
        ] {
            let x = find(&buffer, header).expect("occupied column header").0;
            let actual: String = (x..x + summary::cell_width(word))
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert_eq!(actual, word, "width={width}");
        }
    }
}

fn draw_grid(app: &mut App) -> Buffer {
    let backend = TestBackend::new(180, 60);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let keymap = app.keymap();
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("first grid frame");
    app.tiles.settle_for_test();
    terminal
        .draw(|frame| render::draw(frame, app, &keymap))
        .expect("settled grid frame");
    terminal.backend().buffer().clone()
}

#[test]
fn different_tile_columns_share_core_positions_and_click_the_drawn_headers() {
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("one screen column");
    app.loaded_config.config.tiles.widen_summary = false;
    app.census.apply(CensusUpdate::Local {
        rows:      column_rows(),
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    let buffer = draw_grid(&mut app);
    let header_rows: Vec<_> = lines(&buffer)
        .iter()
        .enumerate()
        .filter_map(|(y, line)| {
            (line.contains("session") && line.contains("status") && !line.contains("pid"))
                .then_some(u16::try_from(y).expect("header row"))
        })
        .collect();
    assert_eq!(header_rows.len(), 2, "{}", lines(&buffer).join("\n"));
    assert_eq!(
        header_rows
            .iter()
            .filter(|&&y| lines(&buffer)[usize::from(y)].contains("type"))
            .count(),
        1,
        "only the work tile has a type column: {}",
        lines(&buffer).join("\n")
    );
    let mut status_positions = Vec::new();
    let mut keys: Vec<TileKey> = Vec::new();
    for y in header_rows {
        let line = &lines(&buffer)[usize::from(y)];
        let x = summary::cell_width(&line[..line.find("status").expect("status header")]);
        status_positions.push(x);
        assert!(line.contains("role"), "{line}");
        app.click(Position::new(x, y));
        let (key, selected) = app
            .tree_sorts
            .iter()
            .find(|(key, _)| !keys.contains(key))
            .expect("clicked tile sort");
        assert_eq!(selected.column, TreeColumn::Status, "{line}");
        keys.push(key.clone());
    }
    assert_eq!(
        status_positions[0], status_positions[1],
        "different column sets share the core positions"
    );
}

#[test]
fn equal_tile_column_sets_share_drawn_header_positions() {
    let mut short = agent(10, "short-owner", HOUR, None);
    short.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(11),
        "build",
        MINUTE,
    ));
    let mut long = agent(20, "long-owner", HOUR, None);
    long.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "run-the-long-project-build",
        MINUTE,
    ));
    let cells = [entry(&short, None), entry(&long, None)];
    let buffers: Vec<_> = cells
        .iter()
        .map(|cell| drawn_into(cell, &cells, 180, 12))
        .collect();
    for header in ["session", "role", "activity", "status", "type"] {
        let left = find(&buffers[0], header).expect("first header").0;
        let right = find(&buffers[1], header).expect("second header").0;
        assert_eq!(left, right, "{header}");
    }
    assert!(find(&buffers[0], "build").is_some());
    assert!(find(&buffers[1], "run-the-long-project-build").is_some());
}

#[test]
fn shared_age_slots_keep_all_headers_aligned_at_the_right_edge() {
    let mut older = agent(10, "older-owner", HOUR, None);
    older.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(11),
        "build",
        6 * 24 * HOUR + 17 * HOUR,
    ));
    let mut younger = agent(20, "younger-owner", HOUR, None);
    younger.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "run-the-long-project-build",
        24 * HOUR + 5 * HOUR,
    ));
    let mut cells = [entry(&older, None), entry(&younger, None)];
    for cell in &mut cells {
        cell.sort = TreeSort {
            column:    TreeColumn::Session,
            direction: SortDirection::Ascending,
        };
    }
    for width in [100, 180] {
        let buffers: Vec<_> = cells
            .iter()
            .map(|cell| drawn_into(cell, &cells, width, 12))
            .collect();
        for header in ["session", "role", "activity", "status", "type"] {
            assert_eq!(
                find(&buffers[0], header).expect("older header").0,
                find(&buffers[1], header).expect("younger header").0,
                "{header}: width={width}"
            );
        }
        for (index, buffer) in buffers.iter().enumerate() {
            assert!(find(buffer, "age").is_none());
            let name = if index == 0 {
                "build"
            } else {
                "run-the-long-project-build"
            };
            let expected = if index == 0 {
                "6 days, 17 hours, 0 minutes"
            } else {
                "1 day, 5 hours, 0 minutes"
            };
            assert_eq!(detail_value(&cells[index], name, "age", NOW), expected);
        }
    }
}

#[test]
fn unassigned_rows_without_paths_keep_other_rows_branch_and_directory_columns() {
    let mut first = agent(10, "first-session", HOUR, None);
    first.branch = Some("first-branch".into());
    let mut second = agent(20, "second-session", HOUR, None);
    second.branch = Some("second-branch".into());
    let mut missing = agent(30, "probe-blocked", HOUR, None);
    missing.directory.clear();
    let state = MachineState::Answered(
        vec![first, second, missing],
        HierarchyEdges::default().into(),
    );
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let pool = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(pool) => Some(pool),
            AgentTile::Agent(_) => None,
        })
        .expect("unassigned tile");
    let rect = area(220, 12);
    let mut buffer = Buffer::empty(rect);
    draw_unassigned(
        &mut buffer,
        rect,
        pool,
        NOW,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    for word in [
        "branch",
        "directory",
        "first-branch",
        "second-branch",
        "~/rust/first-session",
        "~/rust/second-session",
        "probe-blocked",
    ] {
        assert!(
            find(&buffer, word).is_some(),
            "missing {word}: {}",
            lines(&buffer).join("\n")
        );
    }
}

#[test]
fn hidden_accounts_do_not_split_shared_drawn_column_positions() {
    let mut short = agent(10, "short-owner", HOUR, None);
    short.account = serde_json::from_value(serde_json::json!({
        "home": "~/.claude1", "label": "claude1"
    }))
    .expect("recorded account");
    short.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(11),
        "build",
        MINUTE,
    ));
    let mut long = agent(20, "long-owner", HOUR, None);
    long.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "run-the-long-project-build",
        MINUTE,
    ));
    long.account = short.account.clone();
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.tiles.initial_rows =
        serde_json::from_value(serde_json::json!(8)).expect("one screen column");
    app.loaded_config.config.tiles.widen_summary = false;
    app.census.apply(CensusUpdate::Local {
        rows:      vec![short, long],
        hierarchy: HierarchyEdges::default().into(),
        stage:     ScanStage::Completed,
    });
    let buffer = draw_grid(&mut app);
    let headers: Vec<_> = lines(&buffer)
        .into_iter()
        .filter(|line| line.contains("session") && line.contains("type") && line.contains("status"))
        .collect();
    assert_eq!(headers.len(), 2, "{}", lines(&buffer).join("\n"));
    assert!(find(&buffer, "build").is_some());
    assert!(find(&buffer, "run-the-long-project-build").is_some());
    for header in &headers {
        assert!(header.contains("account"), "{header}");
    }
    for word in ["account", "session", "role", "activity", "status"] {
        let positions: Vec<_> = headers
            .iter()
            .map(|header| summary::cell_width(&header[..header.find(word).expect("drawn header")]))
            .collect();
        assert_eq!(positions[0], positions[1], "{word}: {headers:#?}");
    }
}

/// Draw exact content allocations through the same frame preparation as the grid.
fn allocated_tiles(rows: Vec<AgentRow>, allocations: &[(u32, Rect)]) -> Buffer {
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = vec![Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let allocations: Vec<_> = allocations
        .iter()
        .map(|&(pid, rect)| {
            let key = tiles
                .iter()
                .find_map(|tile| match tile {
                    AgentTile::Agent(entry) if entry.row.pid == pid => Some(tile.key()),
                    _ => None,
                })
                .expect("allocated root tile");
            (key, rect)
        })
        .collect();
    let width = allocations
        .iter()
        .map(|(_, rect)| rect.right())
        .max()
        .expect("allocated width");
    let height = allocations
        .iter()
        .map(|(_, rect)| rect.bottom())
        .max()
        .expect("allocated height");
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    render::draw_allocated_tiles_for_test(machines, NOW, &allocations, &mut buffer);
    buffer
}

/// Read a tile's allocation without changing the screen positions in the source buffer.
fn allocation_text(buffer: &Buffer, area: Rect) -> Vec<String> {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

#[test]
fn status_columns_fit_painted_words_without_an_offscreen_thinking_row() {
    let recorded_account = |label| {
        serde_json::from_value(serde_json::json!({
            "home": format!("~/.{label}"), "label": label
        }))
        .expect("recorded fixture account")
    };
    let mut owner = agent(10, "status-owner", HOUR, None);
    owner.account = recorded_account("claude1");
    owner.children = [
        ("visible-running", Activity::Running, 600),
        ("visible-waiting", Activity::Waiting, 500),
        ("another-running", Activity::Running, 400),
        ("later-waiting", Activity::Waiting, 300),
        ("last-running", Activity::Running, 200),
        ("offscreen-thinking", Activity::Thinking, 60),
    ]
    .into_iter()
    .map(|(name, activity, age)| ChildRow {
        activity: AgentActivity::Active(activity),
        ..child(
            0,
            ChildKind::Subagent(SubagentRole::Reviewer),
            None,
            name,
            age,
        )
    })
    .collect();
    let other = AgentRow {
        account: recorded_account("claude2"),
        ..agent(20, "other-account", HOUR, None)
    };
    let rect = Rect::new(0, 0, 130, 6);
    let buffer = allocated_tiles(vec![owner, other], &[(10, rect)]);
    let rows = allocation_text(&buffer, rect);
    let header = rows
        .iter()
        .find(|line| line.contains("status") && line.contains("account"))
        .expect("drawn status and account headers");
    let status_x = summary::cell_width(&header[..header.find("status").expect("status column")]);
    let type_x = summary::cell_width(&header[..header.find("type").expect("type column")]);
    assert!(rows.iter().any(|line| line.contains("running")), "{rows:?}");
    assert!(rows.iter().any(|line| line.contains("waiting")), "{rows:?}");
    assert!(
        !rows.iter().any(|line| line.contains("offscreen-thinking")),
        "{rows:?}"
    );
    assert_eq!(
        type_x - status_x,
        summary::cell_width(TREE_STATUS_HEADER) + TABLE_COLUMN_SPACING,
        "six-cell source status header plus two-cell gap: {rows:?}"
    );
}

#[test]
fn role_columns_fit_visible_roles_without_hidden_or_other_screen_column_roles() {
    let mut left = agent(10, "left-owner", HOUR, None);
    left.children = [
        ("implementation", SubagentRole::Impl, 600),
        ("inspection", SubagentRole::Reviewer, 500),
        ("another-inspection", SubagentRole::Reviewer, 400),
        ("later-inspection", SubagentRole::Reviewer, 300),
        ("last-inspection", SubagentRole::Reviewer, 200),
        ("offscreen-manager", SubagentRole::Showrunner, 60),
    ]
    .into_iter()
    .map(|(name, role, age)| child(0, ChildKind::Subagent(role), None, name, age))
    .collect();
    let mut lower = agent(20, "lower-owner", HOUR, None);
    lower.children = vec![
        child(
            0,
            ChildKind::Subagent(SubagentRole::Impl),
            None,
            "lower-implementation",
            600,
        ),
        child(
            0,
            ChildKind::Subagent(SubagentRole::Reviewer),
            None,
            "lower-inspection",
            500,
        ),
    ];
    let mut right = agent(30, "right-owner", HOUR, None);
    right.children.push(child(
        0,
        ChildKind::Subagent(SubagentRole::Showrunner),
        None,
        "visible-manager",
        600,
    ));
    let upper_area = Rect::new(0, 0, 110, 6);
    let lower_area = Rect::new(0, 8, 110, 6);
    let right_area = Rect::new(120, 0, 110, 14);
    let buffer = allocated_tiles(
        vec![left, lower, right],
        &[(10, upper_area), (20, lower_area), (30, right_area)],
    );
    for rect in [upper_area, lower_area] {
        let rows = allocation_text(&buffer, rect);
        let header = rows
            .iter()
            .find(|line| line.contains("session") && line.contains("role"))
            .expect("role header");
        let role_x = header.find("activity").expect("activity column");
        let status_x = header.find("status").expect("status column");
        assert!(rows.iter().any(|line| line.contains("impl")), "{rows:?}");
        assert!(
            rows.iter().any(|line| line.contains("reviewer")),
            "{rows:?}"
        );
        assert!(
            !rows.iter().any(|line| line.contains("offscreen-manager")),
            "{rows:?}"
        );
        assert_eq!(
            status_x - role_x,
            8 + 2,
            "eight-cell role plus two-cell gap: {rows:?}"
        );
        for (y, _) in rows
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains("reviewer"))
        {
            let y = rect.y + u16::try_from(y).expect("drawn row fits");
            let start = rect.x + u16::try_from(role_x).expect("role column fits");
            let end = rect.x + u16::try_from(status_x).expect("status column fits");
            let role: String = (start..end).map(|x| buffer[(x, y)].symbol()).collect();
            assert_eq!(role, "reviewer  ");
        }
    }
    let right = allocation_text(&buffer, right_area);
    assert!(
        right
            .iter()
            .any(|line| line.contains("visible-manager") && line.contains("showrunner")),
        "{right:?}"
    );
}

#[test]
fn converging_column_groups_share_every_drawn_header_and_age_right_edge() {
    for (width, different_columns) in [(90, false), (50, false), (90, true), (50, true)] {
        assert_converging_columns(width, different_columns);
    }
    assert_converging_account_columns();
}

fn assert_converging_account_columns() {
    let mut first = agent(10, "recorded-owner", HOUR, None);
    first.directory.clear();
    first.account = serde_json::from_value(serde_json::json!({
        "home":"~/.claude1", "label":"claude1"
    }))
    .expect("recorded account");
    let mut second = agent(20, "unrecorded-owner", HOUR, None);
    second.directory.clear();
    for (owner, pid) in [(&mut first, 11), (&mut second, 21)] {
        owner.children.push(child(
            0,
            ChildKind::Shell(ShellTimerObservation::Untimed),
            Some(pid),
            &"s".repeat(100),
            MINUTE,
        ));
    }
    let areas = [Rect::new(0, 0, 90, 12), Rect::new(0, 14, 90, 12)];
    let buffer = allocated_tiles(vec![first, second], &[(10, areas[0]), (20, areas[1])]);
    let headers: Vec<_> = areas
        .iter()
        .map(|&area| {
            allocation_text(&buffer, area)
                .into_iter()
                .find(|line| line.contains("session") && line.contains("status"))
                .expect("core headers")
        })
        .collect();
    assert!(headers[0].contains("account"), "{headers:?}");
    assert!(!headers[1].contains("account"), "{headers:?}");
    for column in ["role", "activity", "status"] {
        assert_eq!(
            headers[0].find(column),
            headers[1].find(column),
            "{headers:?}"
        );
    }
    let session_ends: Vec<_> = areas
        .iter()
        .enumerate()
        .map(|(index, &area)| {
            let rows = allocation_text(&buffer, area);
            let shell = rows
                .iter()
                .find(|line| line.contains("ssss"))
                .expect("shell row");
            assert_eq!(shell.contains("claude1"), index == 0, "{shell}");
            summary::cell_width(&shell[..shell.find('…').expect("cut shell name")])
        })
        .collect();
    assert_eq!(session_ends[0], session_ends[1], "shared session width");
    let account =
        summary::cell_width(&headers[0][..headers[0].find("account").expect("account slot")]);
    assert_eq!(
        session_ends[0] + summary::cell_width("…") + TABLE_COLUMN_SPACING,
        account
    );
    let role = summary::cell_width(&headers[1][..headers[1].find("role").expect("role column")]);
    let lower = allocation_text(&buffer, areas[1]);
    let shell_y = areas[1].y
        + u16::try_from(
            lower
                .iter()
                .position(|row| row.contains("ssss"))
                .expect("shell row"),
        )
        .expect("shell row fits");
    assert!((account..role).all(|x| buffer[(x, shell_y)].symbol() == " "));
}

fn assert_converging_columns(width: u16, different_columns: bool) {
    let mut first = agent(10, "first-owner", HOUR, None);
    first.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(11),
        "build",
        14 * 24 * HOUR,
    ));
    let mut second = agent(20, "second-owner", HOUR, None);
    second.children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(21),
        "run-the-project-build",
        2 * MINUTE + 49,
    ));
    let mut first_member = agent(12, "first-member", 8 * 24 * HOUR + 20 * HOUR, Some(10));
    first_member.branch =
        Some("branch-that-forces-the-compressed-layout-because-it-is-long".into());
    first_member.directory =
        "/a/directory/that/makes/this/populated/column/set/different/from/the/other/tile".into();
    let mut second_member = agent(22, "second-member", 5 * HOUR + 16 * MINUTE, Some(20));
    second_member.branch = first_member.branch.clone();
    second_member.directory.clear();
    if different_columns {
        first_member.branch = Some("main".into());
        second_member.branch = None;
        second.children[0].name = "run-the-project-build-with-a-name".into();
    }
    let upper = Rect::new(0, 0, width, 12);
    let lower = Rect::new(0, 14, width, 12);
    let state = MachineState::Answered(
        vec![
            first.clone(),
            second.clone(),
            first_member.clone(),
            second_member.clone(),
        ],
        HierarchyEdges::default().into(),
    );
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = cell_order_placed(&placement);
    let buffer = allocated_tiles(
        vec![first, second, first_member, second_member],
        &[(10, upper), (20, lower)],
    );
    if width == 50 {
        assert!(
            allocation_text(&buffer, lower)
                .iter()
                .any(|line| line.contains('…'))
        );
    }
    assert_shared_headers(&buffer, [upper, lower], different_columns);
    for (owner, name, expected) in [
        (10, "first-member", "8 days, 20 hours, 0 minutes"),
        (20, "second-member", "5 hours, 16 minutes"),
        (10, "build", "14 days, 0 hours, 0 minutes"),
        (
            20,
            if different_columns {
                "run-the-project-build-with-a-name"
            } else {
                "run-the-project-build"
            },
            "2 minutes",
        ),
    ] {
        let cell = cells
            .iter()
            .find(|cell| cell.row.pid == owner)
            .expect("age owner");
        assert_eq!(detail_value(cell, name, "age", NOW), expected);
    }
}

fn assert_shared_headers(buffer: &Buffer, areas: [Rect; 2], different_columns: bool) {
    let headers: Vec<_> = areas
        .into_iter()
        .map(|rect| {
            allocation_text(buffer, rect)
                .into_iter()
                .find(|line| line.contains("session") && line.contains("status"))
                .expect("tree header")
        })
        .collect();
    let words = |line: &str| {
        line.split_whitespace()
            .filter(|word| !matches!(*word, "▲" | "▼"))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    if !different_columns {
        assert_eq!(
            words(&headers[0]),
            words(&headers[1]),
            "both fitted sets: {headers:?}"
        );
    } else if areas[0].width == 90 {
        assert_eq!(
            words(&headers[0]),
            ["session", "role", "activity", "status", "branch", "type"]
        );
        assert_eq!(
            words(&headers[1]),
            ["session", "role", "activity", "status", "type"]
        );
    }
    for word in words(&headers[0]).into_iter().filter(|word| {
        !different_columns || matches!(word.as_str(), "session" | "role" | "activity" | "status")
    }) {
        assert_eq!(
            headers[0].find(&word),
            headers[1].find(&word),
            "{word}: {headers:?}"
        );
    }
    assert!(
        !headers
            .iter()
            .any(|header| header.split_whitespace().any(|word| word == "age"))
    );
}

fn narrow_core_owner() -> AgentRow {
    let mut owner = agent(1_107_206, "session-name-that-needs-an-ellipsis", HOUR, None);
    owner.children = vec![ChildRow {
        activity: AgentActivity::Active(Activity::Waiting),
        ..child(
            0,
            ChildKind::Subagent(SubagentRole::Reviewer),
            None,
            "review-name-that-needs-an-ellipsis",
            MINUTE,
        )
    }];
    owner
}

#[test]
fn narrow_column_visibility_keeps_whole_core_headers_status_and_gaps() {
    let owner = narrow_core_owner();
    let mut cell = entry(&owner, None);
    cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Busy);
    for width in [30, 32, 40, 50] {
        let buffer = drawn_into(&cell, &[], width, 10);
        let text = lines(&buffer);
        let header = text
            .iter()
            .find(|line| line.contains("session") && line.contains("status"))
            .expect("whole session and status headers at core width");
        let expected = if width < 40 {
            vec!["session", "activity", "status"]
        } else {
            vec!["session", "role", "activity", "status"]
        };
        assert_eq!(
            header.split_whitespace().collect::<Vec<_>>(),
            expected,
            "{text:?}"
        );
        let role = header
            .find(if width < 40 { "activity" } else { "role" })
            .expect("last core header");
        let status = header.find("status").expect("status header");
        assert!(
            status >= role + 5,
            "one gap between role and status: {header}"
        );
        assert!(
            text.iter().any(|line| line.contains("busy")),
            "status stays whole: width={width}: {text:?}"
        );
        assert!(
            text.iter().any(|line| line.contains('…')),
            "cut names need an ellipsis: width={width}: {text:?}"
        );
    }
}

#[test]
fn narrow_below_core_cuts_names_before_role_and_never_cuts_status() {
    let owner = narrow_core_owner();
    let mut cell = entry(&owner, None);
    cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Busy);
    let buffer = drawn_into(&cell, &[], 22, 10);
    let text = lines(&buffer);
    assert!(
        text.iter().any(|line| line.contains("busy")),
        "whole status below core: {text:?}"
    );
    assert!(
        text[0].split_whitespace().collect::<Vec<_>>() == ["session", "status"],
        "role and activity leave before status: {text:?}"
    );
    assert!(
        text.iter().any(|line| line.contains('…')),
        "shortened name is explicit: {text:?}"
    );
}

#[test]
fn narrow_wide_tile_keeps_every_populated_optional_column() {
    let owner = narrow_core_owner();
    let mut member = agent(1_107_207, "member", MINUTE, Some(owner.pid));
    member.branch = Some("topic".into());
    let buffer = allocated_tiles(
        vec![owner, member],
        &[(1_107_206, Rect::new(0, 0, 200, 10))],
    );
    let text = lines(&buffer);
    let header = text
        .iter()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("full headers");
    for word in ["role", "activity", "status", "type", "branch", "directory"] {
        assert!(
            header.contains(word),
            "no unnecessary drop at 200: missing {word}: {text:?}"
        );
    }
}

#[test]
fn narrow_review_sixteen_cell_tree_spends_role_before_status() {
    let owner = narrow_core_owner();
    let mut cell = entry(&owner, None);
    cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Busy);
    let buffer = drawn_into(&cell, &[], 18, 10);
    let text = lines(&buffer);
    assert!(
        text.iter().any(|line| line.contains("busy")),
        "status must remain whole in a sixteen-cell tree: {text:?}"
    );
    assert!(
        text.iter().any(|line| line.contains('…')),
        "cut text needs an ellipsis: {text:?}"
    );
}

#[test]
fn recorded_models_and_efforts_draw_after_roles_with_blank_shell_values() {
    let mut rows = column_rows();
    rows[0].model = ModelName::Opus;
    rows[0].effort = ReasoningEffort::High;
    let mut claude = agent(40, "owner", MINUTE, Some(10));
    claude.model = ModelName::Opus;
    claude.effort = ReasoningEffort::High;
    rows.push(claude);
    let mut codex = agent(30, "codex-worker", MINUTE, Some(10));
    codex.agent = Agent::Codex;
    codex.model = ModelName::Sol;
    codex.effort = ReasoningEffort::Max;
    rows.push(codex);
    rows[0].children.push(child(
        0,
        ChildKind::Shell(ShellTimerObservation::Untimed),
        Some(31),
        "plain-shell",
        MINUTE,
    ));
    let buffer = allocated_tiles(rows, &[(10, Rect::new(0, 0, 220, 12))]);
    let text = lines(&buffer);
    let header = text
        .iter()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("tree header");
    let role = header.find("role").expect("role header");
    let model = header.find("model").expect("model header");
    let effort = header.find("effort").expect("effort header");
    let status = header.find("status").expect("status header");
    assert!(
        role < status && status < model && model < effort,
        "{text:?}"
    );
    assert!(
        text.iter()
            .any(|line| line.contains("owner") && line.contains("opus") && line.contains("high")),
        "{text:?}"
    );
    assert!(
        text.iter().any(|line| line.contains("codex-worker")
            && line.contains("sol")
            && line.contains("max")),
        "{text:?}"
    );
    let shell = text
        .iter()
        .find(|line| line.contains("plain-shell"))
        .expect("shell row");
    let model_x = summary::cell_width(&header[..model]);
    let metadata_end = ["branch", "directory"]
        .into_iter()
        .filter_map(|name| header.find(name))
        .filter(|index| *index > model)
        .min()
        .unwrap_or(header.len());
    let metadata_end = summary::cell_width(&header[..metadata_end]);
    let metadata: String = shell
        .chars()
        .skip(usize::from(model_x))
        .take(usize::from(metadata_end - model_x))
        .collect();
    assert_eq!(metadata.trim(), "", "{text:?}");
}

#[test]
fn model_and_effort_columns_yield_to_names_in_local_and_shared_tiles() {
    for fit in [MetadataFit::Local, MetadataFit::SharedColumn] {
        let mut rows = column_rows();
        rows[0].model = ModelName::Opus;
        rows[0].effort = ReasoningEffort::XHigh;
        rows[0].children[0].model = ModelName::Opus;
        rows[0].children[0].effort = ReasoningEffort::XHigh;
        for index in 0..4 {
            rows[0].children.push(child(
                0,
                ChildKind::Subagent(SubagentRole::Reviewer),
                None,
                &format!("plain-work-{index}"),
                MINUTE,
            ));
        }
        let wide = metadata_tile(rows.clone(), 220, fit);
        assert!(find(&wide, "model").is_some(), "wide model header");
        assert!(find(&wide, "effort").is_some(), "wide effort header");
        let buffer = metadata_tile(rows, 60, fit);
        let text = allocation_text(&buffer, Rect::new(0, 0, 60, 12));
        assert!(
            text.iter().any(|line| line.contains("inspect-widget")),
            "{text:?}"
        );
        assert!(
            !text
                .iter()
                .any(|line| line.contains("model") || line.contains("effort")),
            "{text:?}"
        );
    }
}

#[test]
fn shell_tiles_omit_model_and_effort_columns() {
    let mut rows = column_rows();
    rows[0].children[0].kind = ChildKind::Shell(ShellTimerObservation::Untimed);
    let buffer = allocated_tiles(rows, &[(10, Rect::new(0, 0, 220, 12))]);
    assert!(find(&buffer, "inspect-widget").is_some());
    assert_eq!(find(&buffer, "model"), None);
    assert_eq!(find(&buffer, "effort"), None);
}

fn metadata_tile(mut rows: Vec<AgentRow>, width: u16, fit: MetadataFit) -> Buffer {
    if matches!(fit, MetadataFit::SharedColumn) {
        rows[1].children.push(child(
            0,
            ChildKind::Shell(ShellTimerObservation::Untimed),
            Some(21),
            "neighbor-work",
            MINUTE,
        ));
        return allocated_tiles(
            rows,
            &[
                (10, Rect::new(0, 0, width, 12)),
                (20, Rect::new(0, 12, width, 12)),
            ],
        );
    }
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let cells = cell_order_placed(&placement);
    let owner = cells
        .iter()
        .find(|cell| cell.row.pid == 10)
        .expect("metadata tile");
    drawn_into(owner, &cells, width, 12)
}

#[test]
fn narrow_tiles_drop_effort_then_model_before_directory_and_branch() {
    for fit in [MetadataFit::Local, MetadataFit::SharedColumn] {
        let mut rows = column_rows();
        rows[0].model = ModelName::Opus;
        rows[0].effort = ReasoningEffort::XHigh;
        rows[0].children[0].model = ModelName::Sol;
        rows[0].children[0].effort = ReasoningEffort::High;
        let mut member = agent(30, "member", MINUTE, Some(10));
        member.model = ModelName::Haiku;
        member.effort = ReasoningEffort::Low;
        member.branch = Some("topic".into());
        rows.push(member);
        let mut stages = HashSet::new();
        for width in METADATA_WIDTH_MIN..METADATA_WIDTH_END {
            let buffer = metadata_tile(rows.clone(), width, fit);
            let text = allocation_text(&buffer, Rect::new(0, 0, width, 12));
            let header = text
                .iter()
                .find(|line| line.contains("session") && line.contains("status"))
                .expect("core headers");
            if header.contains("directory") && header.contains("branch") {
                match (header.contains("model"), header.contains("effort")) {
                    (true, true) => {
                        stages.insert(MetadataColumns::ModelAndEffort);
                    },
                    (true, false) => {
                        stages.insert(MetadataColumns::ModelOnly);
                    },
                    (false, false) => {
                        stages.insert(MetadataColumns::Neither);
                    },
                    (false, true) => {
                        assert!(header.contains("model"), "effort outlived model: {header}");
                    },
                }
            }
            if stages.len() == METADATA_COLUMN_STAGES.len() {
                break;
            }
        }
        assert_eq!(stages, HashSet::from(METADATA_COLUMN_STAGES), "fit={fit:?}");
    }
}
