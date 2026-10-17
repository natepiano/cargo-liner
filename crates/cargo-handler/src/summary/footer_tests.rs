//! Machine headings and account colors as they appear in the summary buffer.

#![expect(
    clippy::expect_used,
    reason = "test fixtures and rendered labels must exist"
)]

use ratatui::style::Color;

use super::*;
use crate::census::Account;
use crate::census::HierarchyEdges;
use crate::census::ServiceTier;
use crate::census::SessionIdentity;
use crate::footer_colors::FooterColors;

/// Fixed census time for deterministic age labels.
const NOW: u64 = 1_000_000;
/// Seconds in an hour.
const HOUR: u64 = 3_600;
/// Seconds in a minute.
const MINUTE: u64 = 60;

/// A listed process with no hierarchy, account, or retained work.
fn row(
    pid: u32,
    agent: Agent,
    name: &str,
    status: Option<&str>,
    age: u64,
    directory: &str,
) -> AgentRow {
    AgentRow {
        session_id: SessionIdentity::Unrecorded,
        agent,
        service_tier: ServiceTier::Unrecorded,
        account: Account::Unrecorded,
        name: name.to_string(),
        activity: crate::census::AgentActivity::from_legacy(status),
        started: NOW - age,
        process_started: NOW - age,
        pid,
        desktop: None,
        directory: directory.to_string(),
        branch: None,
        launched_by: None,
        children: Vec::new(),
    }
}

/// A Claude session with a stable identity for hierarchy placement.
fn session(pid: u32, id: &str) -> AgentRow {
    AgentRow {
        session_id: SessionIdentity::Claude(id.to_string()),
        ..row(pid, Agent::Claude, id, None, HOUR, "~")
    }
}

/// Read each buffer row, omitting only unused trailing cells.
fn lines(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// A probe account fixture, independent of the user's account configuration.
fn account(label: &str) -> Account {
    serde_json::from_value(serde_json::json!({
        "home": format!("~/.{label}"), "label": label,
    }))
    .expect("the probe account fixture parses")
}

/// Render the summary at its requested width and height with explicit colors.
fn rendered_with_colors(
    state: &MachineState,
    account_column: AccountColumn,
    colors: &FooterColors,
) -> Buffer {
    let machines = [Machine {
        name: "natedev",
        state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let area = Rect::new(
        0,
        0,
        width(
            &machines,
            &placement,
            NOW,
            account_column,
            SummarySort::default(),
        ),
        u16::try_from(height(&machines, &placement)).expect("fixture height"),
    );
    let mut buffer = Buffer::empty(area);
    draw(
        &mut buffer,
        area,
        &SummaryFrame {
            machines: &machines,
            placement: &placement,
            cells: &agent_cell::cell_order_placed(&placement),
            now: NOW,
            account_column,
            footer_colors: colors,
            sort: SummarySort::default(),
        },
    );
    buffer
}

/// Assert the foreground of each character of a label in the drawn table.
fn assert_color(buffer: &Buffer, text: &str, expected: Color) {
    let drawn = lines(buffer);
    let (y, line) = drawn
        .iter()
        .enumerate()
        .find(|(_, line)| line.contains(text))
        .expect("the label is drawn");
    let start = line.find(text).expect("the label has a column");
    let x = u16::try_from(line[..start].chars().count()).expect("fixture column");
    let y = u16::try_from(y).expect("fixture row");
    for offset in 0..cell_width(text) {
        assert_eq!(buffer[(x + offset, y)].fg, expected, "{text} at {offset}");
    }
}

#[test]
fn machine_headings_count_top_level_agents_instead_of_all_sessions() {
    for (roots, expected) in [
        (0, " natedev · no agents"),
        (1, " natedev · 1 top-level agent"),
        (3, " natedev · 3 top-level agents"),
    ] {
        let mut rows: Vec<_> = (0..roots)
            .map(|index| session(10 + index, &format!("root-{index}")))
            .collect();
        if roots > 0 {
            rows.push(AgentRow {
                launched_by: Some(10),
                ..session(20, "nested-session")
            });
        }
        let state = MachineState::Answered(rows, HierarchyEdges::default());
        let buffer = rendered_with_colors(&state, AccountColumn::Hidden, &FooterColors::default());
        assert_eq!(lines(&buffer)[0], expected);
    }
}

#[test]
fn summary_account_labels_take_footer_colors_but_tiers_keep_the_program_color() {
    let state = MachineState::Answered(
        vec![
            AgentRow {
                account: account("claude1"),
                ..row(10, Agent::Claude, "manager", Some("busy"), HOUR, "~")
            },
            AgentRow {
                account: account("codex1"),
                service_tier: ServiceTier::Fast,
                ..row(20, Agent::Codex, "worker", None, MINUTE, "~")
            },
        ],
        HierarchyEdges::default(),
    );
    let colors = FooterColors::parse(
        "account.claude1=orange\naccount.codex1=bright-cyan\n\
         account.claude=red\naccount.codex=green",
    );
    let buffer = rendered_with_colors(&state, AccountColumn::Shown, &colors);
    assert_color(&buffer, "claude1", Color::Indexed(208));
    assert_color(&buffer, "codex1", Color::LightCyan);
    assert_color(
        &buffer,
        " fast",
        Role::Codex.style().fg.expect("program color"),
    );
    let unrecorded = MachineState::Answered(
        vec![row(30, Agent::Claude, "unrecorded", None, MINUTE, "~")],
        HierarchyEdges::default(),
    );
    let program = rendered_with_colors(&unrecorded, AccountColumn::Shown, &colors);
    assert_color(
        &program,
        "claude",
        Role::Claude.style().fg.expect("program color"),
    );

    let hidden = rendered_with_colors(&state, AccountColumn::Hidden, &colors);
    assert_color(
        &hidden,
        "claude",
        Role::Claude.style().fg.expect("program color"),
    );
    assert_color(
        &hidden,
        "codex fast",
        Role::Codex.style().fg.expect("program color"),
    );
}

#[test]
fn summary_accounts_without_a_footer_entry_keep_the_program_style() {
    let state = MachineState::Answered(
        vec![AgentRow {
            account: account("claude1"),
            ..row(10, Agent::Claude, "manager", Some("busy"), HOUR, "~")
        }],
        HierarchyEdges::default(),
    );
    let buffer = rendered_with_colors(&state, AccountColumn::Shown, &FooterColors::default());
    assert_color(
        &buffer,
        "claude1",
        Role::Claude.style().fg.expect("program color"),
    );
}
