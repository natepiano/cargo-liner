//! Tile title role suffixes use the same footer palette as tree role words.

#![expect(
    clippy::expect_used,
    reason = "fixture tiles and role suffixes must exist"
)]

use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;

use super::*;
use crate::census::AgentRow;
use crate::census::HierarchyEdge;
use crate::census::HierarchyEdges;
use crate::census::HierarchyRole;
use crate::census::MachineState;
use crate::census::hierarchy::ReviewerState;
use crate::footer_colors::FooterRole;

#[test]
fn tile_title_suffixes_use_showrunner_and_unit_role_colors() {
    let rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([
        {"agent":"claude","name":"production","pid":10,"started":10,"session_id":"boss","directory":"/fixture","children":[]},
        {"agent":"claude","name":"director","pid":20,"started":20,"session_id":"unit","directory":"/fixture","children":[]},
        {"agent":"claude","name":"worker","pid":30,"started":30,"session_id":"worker","directory":"/fixture","children":[]}
    ])).expect("agent rows");
    let edges = HierarchyEdges(vec![
        HierarchyEdge {
            manager:  "boss".into(),
            member:   "unit".into(),
            role:     HierarchyRole::Unit,
            reviewer: ReviewerState::NotPool,
        },
        HierarchyEdge {
            manager:  "unit".into(),
            member:   "worker".into(),
            role:     HierarchyRole::Impl,
            reviewer: ReviewerState::NotPool,
        },
    ]);
    let colors =
        FooterColors::parse("role.showrunner=bright-magenta\nrole.unit-director=bright-blue");
    for (name, word, role) in [
        ("production", "showrunner", FooterRole::Showrunner),
        ("director", "unit", FooterRole::UnitDirector),
    ] {
        let visible = rows
            .iter()
            .filter(|row| name == "production" || row.name != "production")
            .cloned()
            .collect();
        let state = MachineState::Answered(visible, edges.clone().into());
        let cells = Cells::new(
            vec![Machine {
                name:  "fixture",
                state: &state,
            }],
            100,
            &colors,
            &HashMap::new(),
            SummarySort::default(),
        );
        let tile = cells
            .tiles
            .iter()
            .find(|tile| matches!(tile,AgentTile::Agent(entry) if entry.row.name == name))
            .expect("role tile");
        let line = cells.group_title(&tile.key()).expect("tile title");
        let text = line.to_string();
        let start = summary::cell_width(&text[..text.find(word).expect("title role suffix")]);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 100, 1));
        Paragraph::new(line).render(buffer.area, &mut buffer);
        let expected = colors
            .role(role)
            .style(Style::default())
            .fg
            .expect("role ink");
        for offset in 0..summary::cell_width(word) {
            assert_eq!(buffer[(start + offset, 0)].fg, expected, "{name}: {word}");
        }
    }
}
