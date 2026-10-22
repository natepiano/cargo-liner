//! Summary and tile age cells share aligned unit slots.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and rendered columns must exist"
)]
#![expect(clippy::panic, reason = "missing drawn age labels fail the fixture")]

use agent_cell::TreeColumn;

use super::*;
use crate::census::HierarchyEdges;
use crate::sort::AccountsSort;
use crate::sort::SortDirection;
use crate::sort::SummaryColumn;
use crate::sort::TreeSort;

const NOW: u64 = 2_000_000;
const AGES: [(&str, u64); 4] = [
    ("eight-days", 8 * 86400 + 20 * 3600),
    ("five-hours", 5 * 3600 + 16 * 60),
    ("fourteen-days", 14 * 86400),
    ("two-minutes", 2 * 60 + 49),
];

fn row_text(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

fn locate(buffer: &Buffer, label: &str) -> (u16, u16) {
    for y in 0..buffer.area.height {
        let text = row_text(buffer, y);
        if let Some(index) = text.find(label) {
            return (cell_width(&text[..index]), y);
        }
    }
    panic!("missing {label}: {buffer:?}");
}

fn assert_unit_slots(buffer: &Buffer) -> Vec<String> {
    let (_, header_y) = locate(buffer, "status");
    let header = row_text(buffer, header_y);
    let age_header = cell_width(&header[..header.find("age").expect("age header")]);
    let age_end = AGES
        .iter()
        .map(|(name, _)| {
            let (_, y) = locate(buffer, name);
            let text = row_text(buffer, y);
            age::age_label(AGES.iter().find(|(label, _)| label == name).expect("age").1)
                .split_whitespace()
                .map(|token| {
                    cell_width(&text[..text.rfind(token).expect("age token")]) + cell_width(token)
                })
                .max()
                .expect("age")
        })
        .max()
        .expect("rows");
    let age_start = AGES
        .iter()
        .map(|(name, seconds)| {
            let (_, y) = locate(buffer, name);
            let text = row_text(buffer, y);
            age::age_label(*seconds)
                .split_whitespace()
                .map(|token| cell_width(&text[..text.rfind(token).expect("age token")]))
                .min()
                .expect("age")
        })
        .min()
        .expect("rows");
    let mut columns = std::collections::HashMap::new();
    let mut labels = Vec::new();
    for (name, seconds) in AGES {
        let (_, y) = locate(buffer, name);
        let text = row_text(buffer, y);
        let mut slots = Vec::new();
        for unit in age::age_label(seconds).split_whitespace() {
            let index = text.rfind(unit).expect("age unit draws");
            let x = cell_width(&text[..index]) + cell_width(unit) - 1;
            let suffix = unit.chars().last().expect("unit suffix");
            if let Some(previous) = columns.insert(suffix, x) {
                assert_eq!(x, previous, "{suffix}: {text}");
            }
            slots.push(unit.to_owned());
        }
        assert_eq!(slots.join(" "), age::age_label(seconds));
        labels.push(
            (age_start..age_end)
                .map(|x| buffer[(x, y)].symbol())
                .collect(),
        );
    }
    assert_eq!(
        age_header + cell_width("age"),
        age_end,
        "age header right edge aligns with age column"
    );
    labels
}

#[test]
fn summary_and_tree_ages_align_each_unit_and_right_align_the_header() {
    let rows:Vec<AgentRow>=AGES.iter().enumerate().map(|(index,(name,seconds))|serde_json::from_value(serde_json::json!({
        "agent":"claude","name":name,"pid":100+index,"started":NOW-seconds,"directory":"/fixture",
        "children":[{"depth":0,"kind":"shell","pid":999,"name":"fixture work","started":NOW-seconds}]
    })).expect("age row")).collect();
    let state = MachineState::Answered(rows.clone(), HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 160, 20);
    let mut summary = Buffer::empty(area);
    draw(
        &mut summary,
        area,
        &SummaryFrame {
            accounts:       &AccountsTable::NotConfigured,
            accounts_sort:  AccountsSort::SourceOrder,
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            NOW,
            account_column: AccountColumn::Shown,
            footer_colors:  &FooterColors::default(),
            sort:           SummarySort {
                column:    SummaryColumn::Name,
                direction: SortDirection::Ascending,
            },
        },
    );
    let summary_labels = assert_unit_slots(&summary);
    let mut root = rows[0].clone();
    root.name = "root".into();
    root.children=rows.iter().map(|row|serde_json::from_value(serde_json::json!({"depth":0,"kind":"thread","name":row.name,"started":row.started})).expect("thread row")).collect();
    let state = MachineState::Answered(vec![root], HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let original = agent_cell::cell_order_placed(&placement);
    let sorts = original
        .iter()
        .map(|entry| {
            (
                entry.id.clone(),
                TreeSort {
                    column:    TreeColumn::Session,
                    direction: SortDirection::Ascending,
                },
            )
        })
        .collect();
    let cells = agent_cell::cell_order_sorted(&placement, &sorts, NOW);
    let mut tree = Buffer::empty(area);
    agent_cell::draw(
        &mut tree,
        area,
        &cells[0],
        &cells,
        NOW,
        AccountColumn::Shown,
        &FooterColors::default(),
    );
    let tree_labels = assert_unit_slots(&tree);
    assert_eq!(
        summary_labels, tree_labels,
        "the same ages draw the same unit-slot text in both tables"
    );
}

#[test]
fn summary_pid_digits_end_under_the_right_aligned_pid_header() {
    let rows:Vec<AgentRow>=[7,12345].into_iter().map(|pid|serde_json::from_value(serde_json::json!({
        "agent":"claude","name":format!("session-{pid}"),"pid":pid,"started":NOW-60,"directory":"/fixture",
        "children":[{"depth":0,"kind":"shell","name":"fixture command","started":NOW-30}]
    })).expect("pid fixture")).collect();
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 120, 12);
    let mut buffer = Buffer::empty(area);
    draw(
        &mut buffer,
        area,
        &SummaryFrame {
            accounts:       &AccountsTable::NotConfigured,
            accounts_sort:  AccountsSort::SourceOrder,
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            NOW,
            account_column: AccountColumn::Shown,
            footer_colors:  &FooterColors::default(),
            sort:           SummarySort {
                column:    SummaryColumn::Name,
                direction: SortDirection::Ascending,
            },
        },
    );
    let (x, _) = locate(&buffer, "pid");
    let right = x + cell_width("pid");
    for pid in ["7", "12345"] {
        let (_, y) = locate(&buffer, &format!("session-{pid}"));
        let line = row_text(&buffer, y);
        let start = line.find(pid).expect("pid digits");
        assert_eq!(
            cell_width(&line[..start]) + cell_width(pid),
            right,
            "{line}"
        );
    }
}

#[test]
fn summary_age_header_ends_at_the_last_unit_used_by_its_rows() {
    let ages = [
        ("week-old", 7 * 86400 + 21 * 3600),
        ("twenty-days", 20 * 86400),
    ];
    let rows: Vec<AgentRow> =
        ages.iter()
            .enumerate()
            .map(|(index, (name, seconds))| {
                serde_json::from_value(serde_json::json!({
            "agent":"claude", "name":name, "pid":10+index,
            "started":NOW-seconds,"directory":"/fixture",
            "children":[{"depth":0,"kind":"shell","name":"fixture work","started":NOW-seconds}]
        })).expect("summary age fixture")
            })
            .collect();
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 160, 12);
    let mut buffer = Buffer::empty(area);
    draw(
        &mut buffer,
        area,
        &SummaryFrame {
            accounts:       &AccountsTable::NotConfigured,
            accounts_sort:  AccountsSort::SourceOrder,
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            NOW,
            account_column: AccountColumn::Shown,
            footer_colors:  &FooterColors::default(),
            sort:           SummarySort {
                column:    SummaryColumn::Name,
                direction: SortDirection::Ascending,
            },
        },
    );
    let (_, header_y) = locate(&buffer, "status");
    let header = row_text(&buffer, header_y);
    let header_x = cell_width(&header[..header.find("age").expect("age column header")]);
    let end = header_x + 3;
    for (name, expected) in [("week-old", " 7d 21h"), ("twenty-days", "20d    ")] {
        let (_, y) = locate(&buffer, name);
        let actual: String = (end - 7..end).map(|x| buffer[(x, y)].symbol()).collect();
        assert_eq!(actual, expected, "{name}: no unused minute or second slots");
    }
}
