//! Column visibility is measured from drawn rows, including real grid clicks.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use tui_pane::CellPoint;
use tui_pane::TerminalApp;

use super::*;
use crate::app::App;
use crate::render;
use crate::sort;
use crate::sort::HeaderHit;
use crate::sort::SortDirection;

fn column_rows() -> Vec<AgentRow> {
    let mut owner = agent(10, "owner", HOUR, None);
    owner.children = vec![child(
        0,
        ChildKind::Subagent(SubagentRole::from_record("review", "")),
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
fn unassigned_tiles_omit_headers_for_columns_without_values() {
    let state = column_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
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
        for absent in ["type", "role", "branch"] {
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
            TreeColumn::Type | TreeColumn::Role | TreeColumn::Branch
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
    let placement = AgentPlacement::from_machines(&machines);
    let cells = cell_order_placed(&placement);
    let owner = cells
        .iter()
        .find(|cell| cell.row.pid == 10)
        .expect("work tile");
    for width in [65, 220] {
        let buffer = drawn_into(owner, &cells, width, 12);
        let y = find(&buffer, "inspect-widget").expect("work row").1;
        for (header, word) in [("type", "subagent"), ("role", "review")] {
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
fn different_tile_columns_fit_independently_and_click_the_drawn_headers() {
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
        let occupied = line.contains("type");
        assert_eq!(line.contains("role"), occupied, "{line}");
        app.click(Position::new(x, y));
        let (key, selected) = app
            .tree_sorts
            .iter()
            .find(|(key, _)| !keys.contains(key))
            .expect("clicked tile sort");
        assert_eq!(selected.column, TreeColumn::Status, "{line}");
        keys.push(key.clone());
    }
    assert_ne!(
        status_positions[0], status_positions[1],
        "different column sets must fit independently"
    );
}

#[test]
fn equal_tile_column_sets_share_drawn_header_positions() {
    let mut short = agent(10, "short-owner", HOUR, None);
    short
        .children
        .push(child(0, ChildKind::Shell, Some(11), "build", MINUTE));
    let mut long = agent(20, "long-owner", HOUR, None);
    long.children.push(child(
        0,
        ChildKind::Shell,
        Some(21),
        "run-the-long-project-build",
        MINUTE,
    ));
    let cells = [entry(&short, None), entry(&long, None)];
    let buffers: Vec<_> = cells
        .iter()
        .map(|cell| drawn_into(cell, &cells, 180, 12))
        .collect();
    for header in ["session", "type", "status", "age"] {
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
        ChildKind::Shell,
        Some(11),
        "build",
        6 * 24 * HOUR + 17 * HOUR,
    ));
    let mut younger = agent(20, "younger-owner", HOUR, None);
    younger.children.push(child(
        0,
        ChildKind::Shell,
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
        for header in ["session", "type", "status", "age"] {
            assert_eq!(
                find(&buffers[0], header).expect("older header").0,
                find(&buffers[1], header).expect("younger header").0,
                "{header}: width={width}"
            );
        }
        for (buffer, age) in buffers.iter().zip(["6d 17h", "1d  5h"]) {
            let edge = header_indented(buffer.area).right();
            assert_eq!(find(buffer, "age").expect("age header").0 + 3, edge);
            assert_eq!(
                find(buffer, age).expect("age with shared unit slots").0 + summary::cell_width(age),
                edge
            );
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
    let placement = AgentPlacement::from_machines(&machines);
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
    short
        .children
        .push(child(0, ChildKind::Shell, Some(11), "build", MINUTE));
    let mut long = agent(20, "long-owner", HOUR, None);
    long.children.push(child(
        0,
        ChildKind::Shell,
        Some(21),
        "run-the-long-project-build",
        MINUTE,
    ));
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
        assert!(!header.contains("account"), "{header}");
    }
    for word in ["session", "status", "age"] {
        let positions: Vec<_> = headers
            .iter()
            .map(|header| summary::cell_width(&header[..header.find(word).expect("drawn header")]))
            .collect();
        assert_eq!(positions[0], positions[1], "{word}: {headers:#?}");
    }
}
