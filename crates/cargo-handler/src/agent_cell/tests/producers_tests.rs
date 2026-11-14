//! Staff producers draw as flat rows of one `producers` tile, apart from the
//! unassigned tiles.

#![expect(clippy::panic, reason = "a missing tile fails the fixture")]

use super::*;
use crate::census::RoleFallback;
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::constants::PRODUCERS_CELL_TITLE;
use crate::tiles::TileKey;

/// A root that staff names a producer.
fn producer(pid: u32, name: &str) -> AgentRow {
    AgentRow {
        role_fallback: RoleFallback::Producer,
        ..session(pid, name)
    }
}

/// A root that staff does not name.
fn session(pid: u32, name: &str) -> AgentRow {
    AgentRow {
        session_id: SessionIdentity::Claude(format!("{name}-{pid}")),
        ..agent(pid, name, HOUR, None)
    }
}

/// A root with its own tile, because it has work.
fn owner(pid: u32, name: &str) -> AgentRow {
    AgentRow {
        children: vec![child(
            0,
            ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
            Some(pid + 1),
            "owner-shell",
            HOUR,
        )],
        ..session(pid, name)
    }
}

fn answered(rows: Vec<AgentRow>) -> MachineState {
    MachineState::Answered(rows, HierarchyEdges::default().into())
}

/// The drawn names of one pool, or `None` when the pool has no tile.
fn pool_names(tiles: &[AgentTile<'_>], pool: TilePool<'_>) -> Option<Vec<String>> {
    tiles.iter().find_map(|tile| match tile {
        AgentTile::Pooled(entry) if entry.pool == pool => Some(
            tile.named_row_identities()
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
        ),
        AgentTile::Agent(_) | AgentTile::Pooled(_) => None,
    })
}

fn tile_index(tiles: &[AgentTile<'_>], key: &TileKey) -> usize {
    tiles
        .iter()
        .position(|tile| &tile.key() == key)
        .unwrap_or_else(|| panic!("missing tile {key:?}"))
}

#[test]
fn a_staff_producer_draws_in_the_producers_tile_before_the_unassigned_tiles() {
    let local = answered(vec![owner(10, "director"), session(20, "loner")]);
    let remote = answered(vec![producer(30, "morty")]);
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
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    assert_eq!(
        pool_names(&tiles, TilePool::Producers),
        Some(vec!["morty".to_owned()])
    );
    assert_eq!(
        pool_names(&tiles, TilePool::Unassigned { machine: "natedev" }),
        Some(vec!["loner".to_owned()])
    );
    assert_eq!(
        pool_names(&tiles, TilePool::Unassigned { machine: "mac" }),
        None
    );
    let unassigned = TileKey::Unassigned {
        machine: "natedev".to_owned(),
    };
    assert!(tile_index(&tiles, &TileKey::Producers) < tile_index(&tiles, &unassigned));
    assert!(tile_keys(&placement).contains(&TileKey::Producers));
}

#[test]
fn a_session_that_staff_does_not_name_a_producer_stays_unassigned() {
    let state = answered(vec![session(20, "producer")]);
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    assert_eq!(pool_names(&tiles, TilePool::Producers), None);
    assert_eq!(
        pool_names(&tiles, TilePool::Unassigned { machine: "natedev" }),
        Some(vec!["producer".to_owned()])
    );
    assert!(!tile_keys(&placement).contains(&TileKey::Producers));
}

#[test]
fn the_producers_tile_leaves_with_its_last_producer() {
    for (rows, shown) in [
        (vec![producer(30, "morty"), session(20, "loner")], true),
        (vec![session(20, "loner")], false),
    ] {
        let state = answered(rows);
        let machines = [Machine {
            name:  "natedev",
            state: &state,
        }];
        let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
        let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
        assert_eq!(pool_names(&tiles, TilePool::Producers).is_some(), shown);
        assert_eq!(tile_keys(&placement).contains(&TileKey::Producers), shown);
    }
}

#[test]
fn the_producers_tile_title_reads_producers() {
    assert_eq!(PRODUCERS_CELL_TITLE, "producers");
    assert_eq!(TilePool::Producers.title(), PRODUCERS_CELL_TITLE);
}

#[test]
fn producers_of_two_machines_share_one_tile_as_distinct_rows() {
    let local = answered(vec![producer(30, "morty")]);
    let remote = answered(vec![producer(30, "morty")]);
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
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let pools: Vec<_> = tiles
        .iter()
        .filter(|tile| tile.key() == TileKey::Producers)
        .collect();
    let [tile] = pools.as_slice() else {
        panic!("one producers tile, not {}", pools.len());
    };
    let rows = tile.named_row_identities();
    let mut names: Vec<_> = rows.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            format!("morty{HEADING_SEPARATOR}mac"),
            format!("morty{HEADING_SEPARATOR}natedev")
        ]
    );
    assert_ne!(
        rows[0].1, rows[1].1,
        "each machine keeps its own row identity"
    );
}

#[test]
fn a_producer_without_a_status_entry_still_draws() {
    let state = answered(vec![producer(30, "morty")]);
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
    let tiles = tile_order_sorted(&placement, &HashMap::new(), NOW);
    let Some(AgentTile::Pooled(entry)) = tiles.first() else {
        panic!("the producers tile");
    };
    let rect = area(220, pooled_height(entry));
    let mut buffer = Buffer::empty(rect);
    draw_pooled(
        &mut buffer,
        rect,
        entry,
        NOW,
        AccountColumn::Shown,
        &FooterColors::parse(""),
    );
    assert!(find(&buffer, "morty").is_some());
}
