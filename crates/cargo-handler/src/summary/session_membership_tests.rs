//! Summary roots and unassigned sessions use one placement decision.

#![expect(clippy::expect_used, reason = "rendered fixture rows must exist")]

use std::collections::HashMap;

use super::*;
use crate::agent_cell::AgentTile;
use crate::census::AgentActivity;
use crate::census::HierarchyEdges;
use crate::census::MachineState;
use crate::census::hierarchy::ReviewerState;
use crate::sort::AccountsSort;

const NOW: u64 = 1_792_166_400;
const AREA: Rect = Rect::new(0, 0, 200, 25);

fn sessions() -> Vec<AgentRow> {
    serde_json::from_value(serde_json::json!([
        {"agent":"claude","name":"production","pid":10,"started":NOW-100,"session_id":"boss","directory":"/fixture","children":[]},
        {"agent":"claude","name":"director","pid":20,"started":NOW-90,"session_id":"unit","directory":"/fixture","children":[]},
        {"agent":"claude","name":"ups","pid":30,"started":NOW-80,"session_id":"ups","directory":"~","children":[]},
        {"agent":"claude","name":"postmaster","pid":40,"started":NOW-70,"session_id":"postmaster","directory":"~","children":[]}
    ])).expect("session fixtures")
}

fn text(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn draw_summary(machines: &[Machine<'_>], placement: &AgentPlacement<'_>) -> Buffer {
    let cells = agent_cell::cell_order_placed(placement);
    let mut buffer = Buffer::empty(AREA);
    draw(
        &mut buffer,
        AREA,
        &SummaryFrame {
            accounts: &AccountsTable::NotConfigured,
            accounts_sort: AccountsSort::SourceOrder,
            machines,
            placement,
            cells: &cells,
            now: NOW,
            account_column: AccountColumn::Shown,
            footer_colors: &FooterColors::default(),
            sort: SummarySort::default(),
        },
    );
    buffer
}

fn state() -> MachineState {
    let edges = HierarchyEdges(vec![crate::census::HierarchyEdge {
        manager:  "boss".into(),
        member:   "unit".into(),
        role:     crate::census::HierarchyRole::Unit,
        reviewer: ReviewerState::NotPool,
    }]);
    MachineState::Answered(sessions(), edges.into())
}

#[test]
fn summary_headers_name_the_account_and_session_columns() {
    let mut state = state();
    if let MachineState::Answered(rows, _) = &mut state {
        rows[0].account = serde_json::from_value(serde_json::json!({
            "home": "~/.claude1", "label": "claude1"
        }))
        .expect("recorded account");
        rows[0].desktop = Some("review-desk".to_string());
        rows[0].activity = AgentActivity::Idle;
    }
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let lines = text(&draw_summary(&machines, &placement));
    let header = lines
        .iter()
        .find(|line| line.starts_with("pid "))
        .expect("summary header");
    let words: Vec<_> = header
        .split_whitespace()
        .filter(|word| !matches!(*word, "▲" | "▼"))
        .collect();
    assert_eq!(
        words,
        [
            "pid",
            "account",
            "session",
            "status",
            "age",
            "desktop",
            "directory"
        ]
    );
}

#[test]
fn summary_omits_the_desktop_header_when_no_drawn_row_has_a_desktop() {
    let mut state = state();
    if let MachineState::Answered(rows, _) = &mut state {
        // This member is in the census, but only the top-level production is drawn here.
        rows[1].desktop = Some("member-desk".to_string());
    }
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let buffer = draw_summary(&machines, &placement);
    let rows = text(&buffer);
    let header = rows
        .iter()
        .find(|line| line.starts_with("pid "))
        .expect("summary header");
    assert!(
        !header.split_whitespace().any(|word| word == "desktop"),
        "{header}"
    );
    assert!(
        header.split_whitespace().any(|word| word == "directory"),
        "{header}"
    );
    assert!(
        rows.iter()
            .any(|line| line.contains("production") && line.contains("/fixture")),
        "{rows:?}"
    );
    let cells = agent_cell::cell_order_placed(&placement);
    let spans = header_spans(
        AREA,
        &SummaryFrame {
            accounts:       &AccountsTable::NotConfigured,
            accounts_sort:  AccountsSort::SourceOrder,
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            NOW,
            account_column: AccountColumn::Shown,
            footer_colors:  &FooterColors::default(),
            sort:           SummarySort::default(),
        },
    );
    assert!(
        !spans
            .iter()
            .any(|span| span.column == SummaryColumn::Desktop)
    );
    let drawn_x = (0..AREA.width - 9)
        .find(|&x| {
            (0..9)
                .map(|offset| buffer[(x + offset, spans[0].row)].symbol())
                .collect::<String>()
                == "directory"
        })
        .expect("drawn directory header");
    assert_eq!(
        crate::sort::header_hit(
            &spans,
            tui_pane::CellPoint {
                column: drawn_x,
                row:    spans[0].row,
            }
        ),
        crate::sort::HeaderHit::Column(SummaryColumn::Directory)
    );
}

#[test]
fn summary_keeps_the_desktop_header_and_value_when_a_drawn_row_has_one() {
    let mut state = state();
    if let MachineState::Answered(rows, _) = &mut state {
        rows[0].desktop = Some("review-desk".to_string());
    }
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let rows = text(&draw_summary(&machines, &placement));
    let header = rows
        .iter()
        .find(|line| line.starts_with("pid "))
        .expect("summary header");
    assert!(
        header.split_whitespace().any(|word| word == "desktop"),
        "{header}"
    );
    assert!(
        rows.iter()
            .any(|line| line.contains("production") && line.contains("review-desk")),
        "{rows:?}"
    );
}

#[test]
fn sessions_without_hierarchy_draw_only_in_unassigned_and_do_not_inflate_summary_count() {
    let state = state();
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let summary = text(&draw_summary(&machines, &placement));
    assert!(
        summary
            .iter()
            .any(|line| line == "fixture · 1 top-level agent"),
        "{summary:?}"
    );
    assert!(
        summary.iter().any(|line| line.contains("production")),
        "{summary:?}"
    );
    for name in ["ups", "postmaster", "director"] {
        assert!(
            !summary
                .iter()
                .any(|line| line.split_whitespace().any(|word| word == name)),
            "{name}: {summary:?}"
        );
    }
    let agents = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &agents, &HashMap::new(), NOW);
    let pool = tiles
        .iter()
        .find_map(|tile| match tile {
            AgentTile::Unassigned(entry) => Some(entry),
            AgentTile::Agent(_) => None,
        })
        .expect("unassigned tile");
    let mut buffer = Buffer::empty(AREA);
    agent_cell::draw_unassigned(
        &mut buffer,
        AREA,
        pool,
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    let pool = text(&buffer);
    for name in ["ups", "postmaster"] {
        assert!(
            pool.iter()
                .any(|line| line.split_whitespace().any(|word| word == name)),
            "{name}: {pool:?}"
        );
    }
    assert!(
        !pool.iter().any(|line| line.contains("production")),
        "{pool:?}"
    );
}

#[test]
fn a_session_with_work_has_one_summary_entry_and_one_tile_instead_of_an_unassigned_copy() {
    let mut rows = sessions();
    rows.truncate(1);
    rows[0].children.push(
        serde_json::from_value(serde_json::json!({
            "depth":0,"kind":"shell","pid":11,"name":"cargo check","started":NOW-50
        }))
        .expect("work fixture"),
    );
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let summary = text(&draw_summary(&machines, &placement));
    assert!(
        summary
            .iter()
            .any(|line| line == "fixture · 1 top-level agent"),
        "{summary:?}"
    );
    assert_eq!(
        summary
            .iter()
            .filter(|line| line.contains("production"))
            .count(),
        1,
        "{summary:?}"
    );
    let cells = agent_cell::cell_order_placed(&placement);
    let tiles = agent_cell::group_tiles(&placement, &cells, &HashMap::new(), NOW);
    assert_eq!(tiles.len(), 1);
    let entry = match &tiles[0] {
        AgentTile::Agent(entry) => Some(entry),
        AgentTile::Unassigned(_) => None,
    }
    .expect("a session with work has its own tile");
    let mut buffer = Buffer::empty(AREA);
    agent_cell::draw(
        &mut buffer,
        AREA,
        entry,
        &cells,
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    assert!(
        text(&buffer)
            .iter()
            .any(|line| line.contains("cargo check"))
    );
}
