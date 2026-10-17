//! The summary uses the same activity words, sort keys and palette as tiles.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and buffer labels must exist"
)]
#![expect(clippy::panic, reason = "missing drawn labels fail a fixture")]

use super::*;
use crate::census::Activity;
use crate::census::AgentActivity;
use crate::census::RunLocation;
use crate::census::ServiceTier;
use crate::census::VerifyStep;
use crate::census::accounts::AccountsTable;
use crate::sort::SortDirection;

fn row(pid: u32, name: &str, activity: AgentActivity) -> AgentRow {
    let mut row: AgentRow = serde_json::from_value(serde_json::json!({
        "agent":"codex", "name":name, "status":null,
        "pid":pid, "started":100 + pid, "directory":"/fixture", "desktop":null,
        "branch":null, "launched_by":null, "children":[]
    }))
    .expect("probe-compatible row");
    row.activity = activity;
    row
}

fn position(buffer: &Buffer, label: &str) -> (u16, u16) {
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
    panic!("missing label {label}");
}

fn rendered(rows: Vec<AgentRow>, sort: SummarySort) -> Buffer {
    let state = MachineState::Answered(rows, crate::census::HierarchyEdges::default());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 220, 12);
    let mut buffer = Buffer::empty(area);
    draw(
        &mut buffer,
        area,
        &SummaryFrame {
            accounts: &AccountsTable::NotConfigured,
            machines: &machines,
            placement: &placement,
            cells: &cells,
            now: 1000,
            account_column: AccountColumn::Hidden,
            footer_colors: &FooterColors::default(),
            sort,
        },
    );
    buffer
}

#[test]
fn activity_summary_sorts_drawn_words_and_preserves_active_gate_and_waiting_colors() {
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let rows = vec![
            row(
                10,
                "writer-session",
                AgentActivity::Active(Activity::Writing),
            ),
            row(
                20,
                "reader-session",
                AgentActivity::Active(Activity::Reading),
            ),
            row(
                30,
                "local-gate",
                AgentActivity::Active(Activity::Verify {
                    step:     VerifyStep::Clippy,
                    location: RunLocation::Local,
                }),
            ),
            row(
                40,
                "remote-gate",
                AgentActivity::Active(Activity::Verify {
                    step:     VerifyStep::Nextest,
                    location: RunLocation::Mac,
                }),
            ),
            row(
                50,
                "waiting-session",
                AgentActivity::Active(Activity::Waiting),
            ),
        ];
        let buffer = rendered(
            rows,
            SummarySort {
                column: SummaryColumn::Status,
                direction,
            },
        );
        let mut actual = [
            "writer-session",
            "reader-session",
            "local-gate",
            "remote-gate",
            "waiting-session",
        ];
        actual.sort_by_key(|name| position(&buffer, name).1);
        let mut expected = [
            "local-gate",
            "remote-gate",
            "reader-session",
            "waiting-session",
            "writer-session",
        ];
        if direction == SortDirection::Descending {
            expected.reverse();
        }
        assert_eq!(actual, expected);
        let status_x = position(&buffer, SummaryColumn::Status.header()).0;
        let cell = |name| &buffer[(status_x, position(&buffer, name).1)];
        for (name, word) in [
            ("writer-session", "writing"),
            ("reader-session", "reading"),
            ("local-gate", "clippy"),
            ("remote-gate", "nextest (mac)"),
            ("waiting-session", "waiting"),
        ] {
            let y = position(&buffer, name).1;
            let text: String = (status_x
                ..status_x + u16::try_from(word.len()).expect("short status"))
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            assert_eq!(text, word);
        }
        assert_eq!(
            cell("writer-session").fg,
            Role::Busy.style().fg.expect("busy color")
        );
        assert_eq!(cell("reader-session").fg, cell("writer-session").fg);
        assert_eq!(
            cell("waiting-session").fg,
            Role::Idle.style().fg.expect("idle color")
        );
        assert_eq!(cell("local-gate").fg, cell("remote-gate").fg);
        assert_ne!(cell("local-gate").fg, cell("waiting-session").fg);
        assert_ne!(
            cell("local-gate").fg,
            cell("writer-session").fg,
            "gates have their own accent"
        );
    }
}

#[test]
fn activity_summary_agent_cells_show_no_standard_mark_and_keep_fast() {
    let mut standard = row(10, "standard-session", AgentActivity::Idle);
    standard.service_tier = ServiceTier::Standard;
    let mut fast = row(20, "fast-session", AgentActivity::Idle);
    fast.service_tier = ServiceTier::Fast;
    let buffer = rendered(vec![standard, fast], SummarySort::default());
    let agent_x = position(&buffer, SummaryColumn::Agent.header()).0;
    let name_x = position(&buffer, SummaryColumn::Name.header()).0;
    for (name, expected) in [
        ("standard-session", "codex"),
        ("fast-session", "codex fast"),
    ] {
        let y = position(&buffer, name).1;
        let text: String = (agent_x..name_x).map(|x| buffer[(x, y)].symbol()).collect();
        assert_eq!(text.trim(), expected);
    }
}
