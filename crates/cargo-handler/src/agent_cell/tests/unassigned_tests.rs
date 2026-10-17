//! Unassigned roots share a flat tile without losing their own work or colors.

#![expect(
    clippy::panic,
    reason = "an unexpected tile or missing header fails the fixture"
)]

use tui_pane::CellPoint;

use super::*;
use crate::constants::UNASSIGNED_CELL_TITLE;
use crate::sort;
use crate::sort::HeaderHit;
use crate::tiles::TileKey;

fn unassigned_forest() -> MachineState {
    let boss = AgentRow {
        session_id: SessionIdentity::Claude("boss-id".into()),
        ..agent(10, "director", 10 * HOUR, None)
    };
    let worker = AgentRow {
        session_id: SessionIdentity::Claude("impl-id".into()),
        ..agent(11, "assigned-worker", 9 * HOUR, None)
    };
    let ups = AgentRow {
        children: vec![
            child(0, ChildKind::Shell, Some(21), "ups-shell", 2 * HOUR),
            child(1, ChildKind::Thread, None, "ups-codex", HOUR),
        ],
        ..agent(20, "ups", 8 * HOUR, None)
    };
    let nested = agent(22, "opened-session", 6 * HOUR, Some(20));
    let postmaster = AgentRow {
        children: vec![child(0, ChildKind::Subagent, None, "post-task", HOUR)],
        ..agent(30, "postmaster", 4 * HOUR, None)
    };
    MachineState::Answered(
        vec![boss, worker, ups, nested, postmaster],
        HierarchyEdges(vec![HierarchyEdge {
            manager: "boss-id".into(),
            member:  "impl-id".into(),
            role:    HierarchyRole::Impl,
        }]),
    )
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
    // A repeated pid on another machine is a different session and tile.
    let remote = MachineState::Answered(
        vec![agent(20, "remote-session", 3 * HOUR, None)],
        HierarchyEdges::default(),
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
    assert_eq!(tiles.len(), 3, "one assigned root and two machine pools");
    for tile in &tiles {
        if let AgentTile::Agent(entry) = tile {
            assert_eq!(entry.row.name, "director");
            let buffer = drawn_into(entry, &cell_order_placed(&placement), 220, 12);
            assert!(find(&buffer, "assigned-worker").is_some());
            assert!(find(&buffer, "postmaster").is_none());
        }
    }
    let buffer = draw_pool(pool(&tiles, "natedev"), 220);
    assert!(find(&buffer, "ups").is_some());
    assert!(find(&buffer, "postmaster").is_some());
    assert!(find(&buffer, "assigned-worker").is_none());
    let buffer = draw_pool(pool(&tiles, "mac"), 220);
    assert!(find(&buffer, "remote-session").is_some());
    assert!(find(&buffer, "postmaster").is_none());
}

#[test]
fn unassigned_rows_are_flat_with_purple_missing_roles_and_nested_work_rails() {
    let state = unassigned_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let buffer = draw_pool(pool(&tiles, "natedev"), 220);
    for (name, prefix) in [
        ("ups", "ups — thinking"),
        ("postmaster", "postmaster — thinking"),
        ("opened-session", "├─ opened-session session thinking"),
        ("ups-shell", "└─ ups-shell — command"),
        ("ups-codex", "└─ ups-codex worker idle"),
        ("post-task", "└─ post-task subagent claude"),
    ] {
        assert!(
            row_text(&buffer, name).starts_with(prefix),
            "{name}: {}",
            row_text(&buffer, name)
        );
    }
    let role_x = find(&buffer, TREE_ROLE_HEADER).expect("role header").0;
    for name in ["ups", "postmaster"] {
        let (name_x, y) = find(&buffer, name).expect("root row");
        assert_eq!(buffer[(role_x, y)].symbol(), MISSING_VALUE);
        assert_eq!(buffer[(role_x, y)].fg, Color::Indexed(141));
        assert!((buffer.area.x..name_x).all(|x| buffer[(x, y)].symbol() == " "));
    }
    let order = [
        "ups",
        "opened-session",
        "ups-shell",
        "ups-codex",
        "postmaster",
        "post-task",
    ]
    .map(|name| find(&buffer, name).expect("row drawn").1);
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn unassigned_session_header_aligns_with_flat_roots_and_keeps_nested_branch_indent() {
    let state = unassigned_forest();
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
        let (name_x, row) = find(&buffer, "ups-shell").expect("nested shell name");
        let glyph_x = (buffer.area.x..name_x)
            .find(|&x| buffer[(x, row)].symbol() == "└")
            .expect("nested shell keeps its branch glyph");
        assert!(name_x > glyph_x);
        assert!(name_x > header_x);
    }
}

#[test]
fn unassigned_grouping_and_sorting_keep_each_sessions_default_hue() {
    let state = unassigned_forest();
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
            ("ups", RainbowHue::Yellow),
            ("opened-session", RainbowHue::Green),
            ("postmaster", RainbowHue::Cyan),
            ("ups-codex", RainbowHue::Yellow),
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
        let work_y = find(&buffer, "ups-codex").expect("ups worker").1;
        assert!(work_y > ups_y);
        assert_eq!(work_y < post_y, direction == SortDirection::Descending);
    }
}

#[test]
fn unassigned_header_clicks_sort_full_and_compressed_rows_and_ignore_gaps() {
    let state = unassigned_forest();
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let key = TileKey::Unassigned {
        machine: "natedev".into(),
    };
    for width in [65, 220] {
        let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
        let entry = pool(&tiles, "natedev");
        let before = draw_pool(entry, width);
        let spans = unassigned_header_spans(entry, before.area, NOW, AccountColumn::Shown);
        let header = &lines(&before)[0];
        assert!(
            header.contains(TREE_SESSION_HEADER)
                && header.contains(&format!("{TREE_AGE_HEADER} ▼"))
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
    let empty = MachineState::Answered(Vec::new(), HierarchyEdges::default());
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
