//! `draw_unassigned` renders `UnassignedEntry` values with nested `ChildRow`
//! lines and each session's `RainbowHue` preserved.

#![expect(
    clippy::panic,
    reason = "an unexpected tile or missing header fails the fixture"
)]

use tui_pane::CellPoint;

use super::*;
use crate::census::RoleFallback;
use crate::constants::UNASSIGNED_CELL_TITLE;
use crate::sort;
use crate::sort::HeaderHit;
use crate::tiles::TileKey;

fn unassigned_forest() -> MachineState {
    let boss = AgentRow {
        role_fallback: RoleFallback::None,
        session_id: SessionIdentity::Claude("boss-id".into()),
        ..agent(10, "director", 10 * HOUR, None)
    };
    let worker = AgentRow {
        role_fallback: RoleFallback::None,
        session_id: SessionIdentity::Claude("impl-id".into()),
        ..agent(11, "assigned-worker", 9 * HOUR, None)
    };
    let ups = AgentRow {
        role_fallback: RoleFallback::None,
        children: vec![
            child(0, ChildKind::Shell, Some(21), "ups-shell", 2 * HOUR),
            child(1, ChildKind::Thread, None, "ups-codex", HOUR),
        ],
        ..agent(20, "ups", 8 * HOUR, None)
    };
    let nested = agent(22, "opened-session", 6 * HOUR, Some(20));
    let postmaster = AgentRow {
        role_fallback: RoleFallback::None,
        children: vec![child(
            0,
            ChildKind::Subagent(crate::census::transcript::SubagentRole::Unrecorded),
            None,
            "post-task",
            HOUR,
        )],
        ..agent(30, "postmaster", 4 * HOUR, None)
    };
    MachineState::Answered(
        vec![boss, worker, ups, nested, postmaster],
        HierarchyEdges(vec![HierarchyEdge {
            reviewer: crate::census::hierarchy::ReviewerState::NotPool,
            manager:  "boss-id".into(),
            member:   "impl-id".into(),
            role:     HierarchyRole::Impl,
        }])
        .into(),
    )
}

fn standalone_forest() -> MachineState {
    let mut state = unassigned_forest();
    if let MachineState::Answered(rows, _) = &mut state {
        for row in rows.iter_mut().filter(|row| row.pid >= 20) {
            row.children.clear();
            row.launched_by = None;
        }
    }
    state
}

fn work_buffer(name: &str) -> Buffer {
    let state = unassigned_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = cell_order_placed(&placement);
    let owner = cells
        .iter()
        .find(|entry| entry.row.name == name)
        .expect("work owner");
    drawn_into(owner, &cells, 220, 12)
}

fn pool<'t, 'a>(tiles: &'t [AgentTile<'a>], machine: &str) -> &'t UnassignedEntry<'a> {
    tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(entry) if entry.machine == machine => Some(entry),
            AgentTile::Agent(_) | AgentTile::Unassigned(_) => None,
        })
        .expect("the machine has an unassigned tile")
}

fn draw_pool(entry: &UnassignedEntry<'_>, width: u16) -> Buffer {
    let rect = area(width, unassigned_height(entry));
    let mut buffer = Buffer::empty(rect);
    draw_unassigned(
        &mut buffer,
        rect,
        entry,
        NOW,
        AccountColumn::Shown,
        &FooterColors::parse("role.unassigned=purple\nrole.worker=bright-cyan"),
    );
    buffer
}

fn row_text(buffer: &Buffer, name: &str) -> String {
    let row = find(buffer, name).expect("fixture row drawn").1;
    lines(buffer)[usize::from(row)]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn unassigned_roots_share_one_tile_per_machine_and_have_no_individual_tiles() {
    let local = unassigned_forest();
    // `remote` repeats a `local` pid on another machine, so it is a different session and tile.
    let remote = MachineState::Answered(
        vec![agent(20, "remote-session", 3 * HOUR, None)],
        HierarchyEdges::default().into(),
    );
    let machines = [
        Machine {
            name:  "natedev",
            state: &local,
        },
        Machine {
            name:  "mac",
            state: &remote,
        },
    ];
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    assert_eq!(tiles.len(), 4, "three hierarchy owners and one remote pool");
    for tile in &tiles {
        if let AgentTile::Agent(entry) = tile {
            let buffer = drawn_into(entry, &cell_order_placed(&placement), 220, 12);
            match entry.row.name.as_str() {
                "director" => assert!(find(&buffer, "assigned-worker").is_some()),
                "ups" => assert!(find(&buffer, "ups-codex").is_some()),
                "postmaster" => assert!(find(&buffer, "post-task").is_some()),
                _ => panic!("unexpected hierarchy owner"),
            }
        }
    }
    assert!(
        !tiles
            .iter()
            .any(|tile| matches!(tile, AgentTile::Unassigned(entry) if entry.machine == "natedev"))
    );
    let buffer = draw_pool(pool(&tiles, "mac"), 220);
    assert!(find(&buffer, "remote-session").is_some());
    assert!(find(&buffer, "postmaster").is_none());
}

#[test]
fn unassigned_rows_are_flat_with_blank_roles_and_nested_work_rails() {
    let state = standalone_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let buffer = draw_pool(pool(&tiles, "natedev"), 220);
    for (name, prefix) in [
        ("ups", "ups thinking"),
        ("postmaster", "postmaster thinking"),
        ("opened-session", "opened-session thinking"),
    ] {
        assert!(
            row_text(&buffer, name).starts_with(prefix),
            "{name}: {}",
            row_text(&buffer, name)
        );
    }
    assert!(find(&buffer, TREE_ROLE_HEADER).is_none());
    assert!(find(&buffer, TREE_TYPE_HEADER).is_none());
    for name in ["ups", "postmaster"] {
        let (name_x, y) = find(&buffer, name).expect("root row");
        assert!((buffer.area.x..name_x).all(|x| buffer[(x, y)].symbol() == " "));
    }
    let order = ["opened-session", "postmaster", "ups"]
        .map(|name| find(&buffer, name).expect("row drawn").1);
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    let work = work_buffer("ups");
    for (name, prefix) in [
        ("opened-session", "├─ opened-session session thinking"),
        ("ups-shell", "└─ ups-shell shell running"),
        ("ups-codex", "└─ ups-codex worker idle"),
    ] {
        assert!(row_text(&work, name).starts_with(prefix));
    }
    assert!(
        row_text(&work_buffer("postmaster"), "post-task").starts_with("└─ post-task subagent idle")
    );
}

#[test]
fn unassigned_session_header_aligns_with_flat_roots_and_keeps_nested_branch_indent() {
    let state = standalone_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    for width in [65, 220] {
        let buffer = draw_pool(pool(&tiles, "natedev"), width);
        let header_x = find(&buffer, TREE_SESSION_HEADER)
            .expect("session header")
            .0;
        for name in ["ups", "postmaster"] {
            let name_x = find(&buffer, name).expect("flat root name").0;
            assert_eq!(
                header_x, name_x,
                "header aligns with {name} at width {width}"
            );
        }
        let work = work_buffer("ups");
        let (name_x, row) = find(&work, "ups-shell").expect("nested shell name");
        let glyph_x = (buffer.area.x..name_x)
            .find(|&x| work[(x, row)].symbol() == "└")
            .expect("nested shell keeps its branch glyph");
        assert!(name_x > glyph_x);
        assert!(name_x > header_x);
    }
}

#[test]
fn unassigned_grouping_and_sorting_keep_each_sessions_default_hue() {
    let state = standalone_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let sorts = HashMap::from([(
            TileKey::Unassigned {
                machine: "natedev".into(),
            },
            TreeSort {
                column: TreeColumn::Session,
                direction,
            },
        )]);
        let tiles = tile_order_sorted(&placement, &sorts, NOW);
        let buffer = draw_pool(pool(&tiles, "natedev"), 220);
        for (name, hue) in [
            ("ups", RainbowHue::Green),
            ("opened-session", RainbowHue::Cyan),
            ("postmaster", RainbowHue::Blue),
        ] {
            assert_eq!(
                buffer[find(&buffer, name).expect("name drawn")].fg,
                Role::Rainbow(hue).style().fg.expect("rainbow color"),
                "{name}"
            );
        }
        let ups_y = find(&buffer, "ups").expect("ups").1;
        let post_y = find(&buffer, "postmaster").expect("postmaster").1;
        assert_eq!(post_y < ups_y, direction == SortDirection::Ascending);
        let work = work_buffer("ups");
        assert_eq!(
            work[find(&work, "ups-codex").expect("ups worker")].fg,
            Role::Rainbow(RainbowHue::Green)
                .style()
                .fg
                .expect("owner hue")
        );
        assert!(
            find(&work, "ups-codex").expect("worker").1
                > find(&work, TREE_SESSION_HEADER).expect("header").1
        );
    }
}

#[test]
fn unassigned_header_clicks_sort_full_and_compressed_rows_and_ignore_gaps() {
    let state = standalone_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let key = TileKey::Unassigned {
        machine: "natedev".into(),
    };
    for width in [45, 220] {
        let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
        let entry = pool(&tiles, "natedev");
        let before = draw_pool(entry, width);
        let spans = unassigned_header_spans(entry, before.area, NOW, AccountColumn::Shown);
        let header = &lines(&before)[0];
        assert!(
            header.contains(&format!("{TREE_SESSION_HEADER} ▲"))
                && header.contains(TREE_AGE_HEADER)
        );
        assert_eq!(header.contains(TREE_DIRECTORY_HEADER), width == 220);
        for span in &spans {
            let x = span.columns.start;
            assert_eq!(
                sort::header_hit(
                    &spans,
                    CellPoint {
                        column: x,
                        row:    span.row,
                    }
                ),
                HeaderHit::Column(span.column)
            );
            assert_eq!(
                sort::header_hit(
                    &spans,
                    CellPoint {
                        column: x,
                        row:    span.row + 1,
                    }
                ),
                HeaderHit::Miss
            );
        }
        for pair in spans.windows(2) {
            if pair[0].columns.end < pair[1].columns.start {
                assert_eq!(
                    sort::header_hit(
                        &spans,
                        CellPoint {
                            column: pair[0].columns.end,
                            row:    pair[0].row,
                        }
                    ),
                    HeaderHit::Miss
                );
            }
        }
        let (x, y) = find(&before, TREE_SESSION_HEADER).expect("session header");
        let HeaderHit::Column(column) = sort::header_hit(
            &spans,
            CellPoint {
                column: x,
                row:    y,
            },
        ) else {
            panic!("drawn session header must select a column");
        };
        let mut sort = TreeSort::default();
        for expected_arrow in [
            format!("{TREE_SESSION_HEADER} ▲"),
            format!("{TREE_SESSION_HEADER} ▼"),
        ] {
            sort.select(column);
            let tiles = tile_order_sorted(&placement, &HashMap::from([(key.clone(), sort)]), NOW);
            let buffer = draw_pool(pool(&tiles, "natedev"), width);
            let header = &lines(&buffer)[0];
            assert!(header.contains(&expected_arrow));
            assert_eq!(header.matches('▲').count() + header.matches('▼').count(), 1);
            assert_eq!(
                find(&buffer, "postmaster").expect("postmaster").1
                    < find(&buffer, "ups").expect("ups").1,
                expected_arrow.ends_with('▲')
            );
        }
    }
}

#[test]
fn assigned_or_empty_machines_do_not_leave_an_empty_unassigned_tile() {
    let mut state = unassigned_forest();
    if let MachineState::Answered(rows, _) = &mut state {
        rows.retain(|row| row.pid == 10 || row.pid == 11);
    }
    let empty = MachineState::Answered(Vec::new(), HierarchyEdges::default().into());
    let machines = [
        Machine {
            name:  "natedev",
            state: &state,
        },
        Machine {
            name:  "mac",
            state: &empty,
        },
    ];
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    assert_eq!(tiles.len(), 1);
    let AgentTile::Agent(entry) = &tiles[0] else {
        panic!("only the assigned director remains")
    };
    let buffer = drawn_into(entry, &cell_order_placed(&placement), 220, 12);
    assert!(find(&buffer, "assigned-worker").is_some());
    assert!(find(&buffer, UNASSIGNED_CELL_TITLE).is_none());
}

#[test]
fn unassigned_age_column_reaches_the_right_content_edge_at_each_width() {
    let state = MachineState::Answered(
        vec![agent(20, "free-session", 21 * HOUR, None)],
        HierarchyEdges::default().into(),
    );
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let entry = pool(&tiles, "natedev");
    for width in [65, 220] {
        let buffer = draw_pool(entry, width);
        let content = header_indented(buffer.area);
        let (header_x, header_y) = find(&buffer, "age").expect("age header");
        let (value_x, value_y) = find(&buffer, "21h").expect("age value");
        assert_eq!(
            header_x + 3,
            content.right(),
            "width={width}: {}",
            lines(&buffer).join("\n")
        );
        assert_eq!(value_x + 3, content.right(), "width={width}");
        assert!(value_y > header_y);
        for absent in ["type", "role"] {
            assert!(find(&buffer, absent).is_none(), "{absent}: width={width}");
        }
        let spans = unassigned_header_spans(entry, buffer.area, NOW, AccountColumn::Shown);
        assert_eq!(
            sort::header_hit(
                &spans,
                CellPoint {
                    column: header_x,
                    row:    header_y,
                }
            ),
            HeaderHit::Column(TreeColumn::Age)
        );
    }
}
