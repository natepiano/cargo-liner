//! Group identities keep their sort independently of membership and summary rows.

#![expect(clippy::expect_used, reason = "fixture rows and headers must exist")]
#![expect(clippy::panic, reason = "missing drawn fixture labels fail the test")]

use ratatui::style::Color;
use tui_pane::CellClick;
use tui_pane::CellPoint;

use super::*;
use crate::agent_cell::AgentTile;
use crate::agent_cell::TreeColumn;
use crate::app::App;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdge;
use crate::census::HierarchyEdges;
use crate::census::HierarchyRole;
use crate::census::accounts::AccountsTable;
use crate::constants::TREE_AGE_HEADER;
use crate::constants::TREE_ROLE_HEADER;
use crate::constants::TREE_SESSION_HEADER;
use crate::constants::UNASSIGNED_CELL_TITLE;
use crate::render;
use crate::sort::SortDirection;
use crate::sort::TreeSort;
use crate::theme::RainbowHue;
use crate::tiles::TileContent;
use crate::tiles::TileKey;

const NOW: u64 = 1_000_000;
const AREA: Rect = Rect::new(0, 0, 220, 20);

fn session(pid: u32, name: &str, age: u64) -> AgentRow {
    serde_json::from_value(serde_json::json!({
        "agent":"claude", "name":name, "pid":pid, "started":NOW-age,
        "process_started":NOW-age, "session_id":format!("session-{pid}"),
        "directory":format!("/fixture/{name}"), "desktop":null, "branch":null,
        "status":"idle", "launched_by":null, "children":[]
    }))
    .expect("fixture session")
}

fn key(machine: &str) -> TileKey {
    TileKey::Unassigned {
        machine: machine.to_string(),
    }
}

fn label_position(buffer: &Buffer, label: &str) -> (u16, u16) {
    for y in buffer.area.y..buffer.area.bottom() {
        let text: String = (buffer.area.x..buffer.area.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        if let Some((offset, _)) = text.match_indices(label).find(|(offset, _)| {
            text[..*offset]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
                && text[*offset + label.len()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        }) {
            return (
                buffer.area.x + u16::try_from(text[..offset].chars().count()).expect("column fits"),
                y,
            );
        }
    }
    panic!("missing drawn label {label}");
}

fn text_row(buffer: &Buffer, label: &str) -> String {
    let y = label_position(buffer, label).1;
    let text: String = (buffer.area.x..buffer.area.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn apply_local(app: &mut App, rows: Vec<AgentRow>) {
    app.census.apply(CensusUpdate::LocalScanCompleted(
        rows,
        HierarchyEdges::default(),
    ));
    app.prune_tree_sorts();
}

fn app_pool_buffer(app: &App, machine: &str) -> Buffer {
    let machines = app
        .census
        .machines(&app.loaded_config.config.machines.remote);
    let placement = AgentPlacement::from_machines(&machines);
    let tiles = agent_cell::tile_order_sorted(&placement, &app.tree_sorts, NOW);
    let entry = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(entry) if entry.machine == machine => Some(entry),
            AgentTile::Agent(_) | AgentTile::Unassigned(_) => None,
        })
        .expect("machine pool exists");
    let mut buffer = Buffer::empty(AREA);
    agent_cell::draw_unassigned(
        &mut buffer,
        AREA,
        entry,
        NOW,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    buffer
}

fn click_pool_header(app: &mut App, machine: &str, column: TreeColumn) {
    let point = {
        let machines = app
            .census
            .machines(&app.loaded_config.config.machines.remote);
        let placement = AgentPlacement::from_machines(&machines);
        let tiles = agent_cell::tile_order_sorted(&placement, &app.tree_sorts, NOW);
        let entry = tiles
            .iter()
            .find_map(|tile| match tile {
                AgentTile::Unassigned(entry) if entry.machine == machine => Some(entry),
                AgentTile::Agent(_) | AgentTile::Unassigned(_) => None,
            })
            .expect("machine pool exists");
        let span = agent_cell::unassigned_header_spans(entry, AREA, NOW, AccountColumn::Hidden)
            .into_iter()
            .find(|span| span.column == column)
            .expect("requested header");
        CellPoint {
            column: span.columns.start,
            row:    span.row,
        }
    };
    render::cell_clicked(
        app,
        CellClick {
            index: 1,
            content: TileContent::Group(key(machine)),
            area: AREA,
            point,
        },
    );
}

#[test]
fn unassigned_sort_survives_members_joining_and_leaving_and_resets_after_disappearance() {
    let mut app = App::new_for_test().expect("test app");
    let older = session(20, "zeta", 7200);
    let younger = session(30, "alpha", 3600);
    apply_local(&mut app, vec![older.clone(), younger.clone()]);
    click_pool_header(&mut app, "natedev", TreeColumn::Session);
    let selected = TreeSort {
        column:    TreeColumn::Session,
        direction: SortDirection::Ascending,
    };
    assert_eq!(app.tree_sorts.get(&key("natedev")), Some(&selected));
    let before = app_pool_buffer(&app, "natedev");
    assert!(
        text_row(&before, &format!("{TREE_SESSION_HEADER} ▲"))
            .contains(&format!("{TREE_SESSION_HEADER} ▲"))
    );
    assert!(label_position(&before, "alpha").1 < label_position(&before, "zeta").1);

    let joined = session(40, "beta", 1800);
    apply_local(&mut app, vec![older.clone(), younger, joined.clone()]);
    let buffer = app_pool_buffer(&app, "natedev");
    assert!(label_position(&buffer, "alpha").1 < label_position(&buffer, "beta").1);
    assert!(label_position(&buffer, "beta").1 < label_position(&buffer, "zeta").1);
    assert_eq!(app.tree_sorts.get(&key("natedev")), Some(&selected));

    // The original oldest root leaves too: the preference belongs to the pool.
    apply_local(&mut app, vec![joined, session(45, "gamma", 900)]);
    let buffer = app_pool_buffer(&app, "natedev");
    assert!(label_position(&buffer, "beta").1 < label_position(&buffer, "gamma").1);
    assert!(
        text_row(&buffer, &format!("{TREE_SESSION_HEADER} ▲"))
            .contains(&format!("{TREE_SESSION_HEADER} ▲"))
    );
    apply_local(&mut app, Vec::new());
    assert!(!app.tree_sorts.contains_key(&key("natedev")));
    apply_local(&mut app, vec![older, session(50, "new-alpha", 600)]);
    let buffer = app_pool_buffer(&app, "natedev");
    assert!(
        text_row(&buffer, &format!("{TREE_AGE_HEADER} ▼"))
            .contains(&format!("{TREE_AGE_HEADER} ▼"))
    );
    assert!(label_position(&buffer, "zeta").1 < label_position(&buffer, "new-alpha").1);
}

#[test]
fn unassigned_machine_sorts_are_independent_and_one_machine_disappearing_preserves_the_other() {
    let mut app = App::new_for_test().expect("test app");
    app.loaded_config.config.machines.remote = vec!["mac".to_string()];
    let older = session(20, "zeta", 7200);
    let younger = session(30, "alpha", 3600);
    apply_local(&mut app, vec![older.clone(), younger.clone()]);
    app.census.apply(CensusUpdate::Remote {
        host:  "mac".into(),
        state: MachineState::Answered(vec![older, younger], HierarchyEdges::default()),
    });
    click_pool_header(&mut app, "natedev", TreeColumn::Session);
    let local = app_pool_buffer(&app, "natedev");
    let remote = app_pool_buffer(&app, "mac");
    assert!(label_position(&local, "alpha").1 < label_position(&local, "zeta").1);
    assert!(label_position(&remote, "zeta").1 < label_position(&remote, "alpha").1);
    click_pool_header(&mut app, "mac", TreeColumn::Session);
    click_pool_header(&mut app, "mac", TreeColumn::Session);
    let remote_sort = app.tree_sorts[&key("mac")];
    let local = app_pool_buffer(&app, "natedev");
    let remote = app_pool_buffer(&app, "mac");
    assert!(
        text_row(&local, &format!("{TREE_SESSION_HEADER} ▲"))
            .contains(&format!("{TREE_SESSION_HEADER} ▲"))
    );
    assert!(
        text_row(&remote, &format!("{TREE_SESSION_HEADER} ▼"))
            .contains(&format!("{TREE_SESSION_HEADER} ▼"))
    );
    assert!(label_position(&remote, "zeta").1 < label_position(&remote, "alpha").1);
    apply_local(&mut app, Vec::new());
    assert!(!app.tree_sorts.contains_key(&key("natedev")));
    assert_eq!(app.tree_sorts.get(&key("mac")), Some(&remote_sort));
    let buffer = app_pool_buffer(&app, "mac");
    assert!(
        text_row(&buffer, &format!("{TREE_SESSION_HEADER} ▼"))
            .contains(&format!("{TREE_SESSION_HEADER} ▼"))
    );
    app.census.apply(CensusUpdate::Remote {
        host:  "mac".into(),
        state: MachineState::Answered(Vec::new(), HierarchyEdges::default()),
    });
    app.prune_tree_sorts();
    assert!(app.tree_sorts.is_empty());
}

fn summary_states() -> [MachineState; 2] {
    let local = MachineState::Answered(
        vec![
            session(10, "director", 14400),
            session(11, "assigned-worker", 10800),
            session(20, "ups", 7200),
            session(30, "postmaster", 3600),
        ],
        HierarchyEdges(vec![HierarchyEdge {
            manager: "session-10".into(),
            member:  "session-11".into(),
            role:    HierarchyRole::Impl,
        }]),
    );
    let remote = MachineState::Answered(
        vec![session(20, "remote-session", 1800)],
        HierarchyEdges::default(),
    );
    [local, remote]
}

fn assert_pool_matches_summary(tiles: &[AgentTile<'_>], name: &str, color: Color) {
    let machine = if name == "remote-session" {
        "mac"
    } else {
        "natedev"
    };
    let pool = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(entry) if entry.machine == machine => Some(entry),
            AgentTile::Agent(_) | AgentTile::Unassigned(_) => None,
        })
        .expect(UNASSIGNED_CELL_TITLE);
    let mut buffer = Buffer::empty(AREA);
    agent_cell::draw_unassigned(
        &mut buffer,
        AREA,
        pool,
        NOW,
        AccountColumn::Hidden,
        &FooterColors::parse("role.unassigned=purple"),
    );
    assert_eq!(buffer[label_position(&buffer, name)].fg, color);
    let y = label_position(&buffer, name).1;
    let role_x = label_position(&buffer, TREE_ROLE_HEADER).0;
    assert_eq!(buffer[(role_x, y)].fg, Color::Indexed(141));
}

#[test]
fn unassigned_grouping_keeps_individual_summary_rows_counts_and_default_name_hues() {
    let [local, remote] = summary_states();
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
    let agents = agent_cell::cell_order_placed(&placement);
    let sorts = std::collections::HashMap::from([(
        key("natedev"),
        TreeSort {
            column:    TreeColumn::Session,
            direction: SortDirection::Ascending,
        },
    )]);
    let tiles = agent_cell::group_tiles(&placement, &agents, &sorts, NOW);
    assert_eq!(tiles.len(), 3, "one director tile and one pool per machine");
    let mut buffer = Buffer::empty(AREA);
    draw(
        &mut buffer,
        AREA,
        &SummaryFrame {
            accounts:       &AccountsTable::NotConfigured,
            machines:       &machines,
            placement:      &placement,
            cells:          &agents,
            now:            NOW,
            account_column: AccountColumn::Hidden,
            footer_colors:  &FooterColors::default(),
            sort:           SummarySort::default(),
        },
    );
    assert_eq!(
        text_row(&buffer, "natedev ·"),
        "natedev · 3 top-level agents"
    );
    assert_eq!(text_row(&buffer, "mac ·"), "mac · 1 top-level agent");
    for (name, expected, hue) in [
        (
            "director",
            "10 claude director idle 4h — /fixture/director",
            RainbowHue::Red,
        ),
        (
            "ups",
            "20 claude ups idle 2h — /fixture/ups",
            RainbowHue::Yellow,
        ),
        (
            "postmaster",
            "30 claude postmaster idle 1h — /fixture/postmaster",
            RainbowHue::Green,
        ),
        (
            "remote-session",
            "20 claude remote-session idle 30m — /fixture/remote-session",
            RainbowHue::Cyan,
        ),
    ] {
        assert_eq!(text_row(&buffer, name), expected);
        let color = buffer[label_position(&buffer, name)].fg;
        assert_eq!(color, Role::Rainbow(hue).style().fg.expect("name hue"));
        if name != "director" {
            assert_pool_matches_summary(&tiles, name, color);
        }
    }
    let mut order = ["director", "ups", "postmaster", "remote-session"];
    order.sort_by_key(|name| label_position(&buffer, name).1);
    assert_eq!(order, ["director", "ups", "postmaster", "remote-session"]);
    assert!((0..AREA.height).all(|y| {
        let row: String = (0..AREA.width).map(|x| buffer[(x, y)].symbol()).collect();
        !row.contains("assigned-worker") && !row.contains(UNASSIGNED_CELL_TITLE)
    }));
}
