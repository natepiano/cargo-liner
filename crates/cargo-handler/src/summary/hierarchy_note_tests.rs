//! Discovery diagnostics are measured, drawn, and visible without any agents.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and terminal drawing must succeed"
)]

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tui_pane::TerminalApp;

use super::*;
use crate::app::App;
use crate::census::Census;
use crate::census::CensusUpdate;
use crate::census::HierarchyEdges;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsSource;
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::census::hierarchy::HierarchyError;
use crate::census::hierarchy::HierarchyNotice;
use crate::census::hierarchy::HierarchySnapshot;
use crate::census::scan::ScanStage;

fn rows() -> Vec<AgentRow> {
    vec![
        serde_json::from_value(serde_json::json!({
            "agent":"claude","name":"visible-production-session","pid":10,
            "started":100,"directory":"/fixture","children":[{"depth":0,"kind":"shell","pid":9999,"name":"fixture command","started":100}]
        }))
        .expect("visible session"),
    ]
}

fn lines(buffer: &Buffer) -> Vec<String> {
    (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            (buffer.area.x..buffer.area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// Draw the summary for `machines` in exactly the area it measures.
fn draw_at_measured_demand(machines: &[Machine<'_>]) -> Buffer {
    let placement = AgentPlacement::from_machines(machines, &EMPTY_SESSION_STATES);
    let cells = agent_cell::cell_order_placed(&placement);
    let colors = FooterColors::default();
    let frame = SummaryFrame {
        machines,
        placement: &placement,
        cells: &cells,
        now: 200,
        account_column: AccountColumn::Hidden,
        footer_colors: &colors,
        sort: SummarySort::default(),
        session_states: &EMPTY_SESSION_STATES,
    };
    let area = Rect::new(
        4,
        2,
        width(
            machines,
            &placement,
            200,
            AccountColumn::Hidden,
            SummarySort::default(),
        ),
        u16::try_from(height(machines, &placement)).expect("small measured height"),
    );
    let mut buffer = Buffer::empty(area);
    draw(&mut buffer, area, &frame);
    buffer
}

#[test]
fn hierarchy_notes_failed_unmatched_and_disabled_draw_below_agents_at_measured_demand() {
    for (notice, expected) in [
        (
            HierarchyNotice::Failed(HierarchyError::TimedOut),
            "hierarchy: timed out",
        ),
        (
            HierarchyNotice::Failed(HierarchyError::ExitedUnsuccessfully),
            "hierarchy: command failed",
        ),
        (
            HierarchyNotice::NoSessionMatched,
            "hierarchy: no session matched",
        ),
        (HierarchyNotice::Clear, ""),
    ] {
        let state = MachineState::Answered(
            rows(),
            HierarchySnapshot {
                notice,
                ..HierarchySnapshot::default()
            },
        );
        let machines = [Machine {
            name:  "fixture",
            state: &state,
        }];
        let buffer = draw_at_measured_demand(&machines);
        let area = buffer.area;
        let output = lines(&buffer);
        let agent_y = output
            .iter()
            .position(|line| line.contains("visible-production-session"))
            .expect("agents remain visible");
        if expected.is_empty() {
            assert!(
                output.iter().all(|line| !line.contains("hierarchy:")),
                "{output:?}"
            );
        } else {
            assert_eq!(output[agent_y + 1].trim(), expected, "{output:?}");
            assert_eq!(
                output
                    .iter()
                    .filter(|line| line.contains("hierarchy:"))
                    .count(),
                1
            );
            let y = area.y + u16::try_from(agent_y + 1).expect("note row");
            for x in area.x..area.right() {
                if !buffer[(x, y)].symbol().trim().is_empty() {
                    assert!(
                        buffer[(x, y)].modifier.contains(Modifier::DIM),
                        "diagnostic cell at {x}"
                    );
                }
            }
        }
    }
}

#[test]
fn hierarchy_failure_without_agents_or_accounts_draws_the_summary_in_the_real_frame() {
    let mut app = App::new_for_test().expect("test app");
    app.census = Census::new("fixture".into());
    app.census.apply(CensusUpdate::Local {
        rows:      Vec::new(),
        hierarchy: HierarchySnapshot {
            facts:  crate::census::CensusFacts::default().into(),
            edges:  HierarchyEdges::default(),
            notice: HierarchyNotice::Failed(HierarchyError::TimedOut),
        },
        stage:     ScanStage::Completed,
    });
    app.before_draw();
    let keymap = app.keymap();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("frame draws");
    let output = lines(terminal.backend().buffer()).join("\n");
    assert!(output.contains("summary"), "{output}");
    assert!(output.contains("hierarchy: timed out"), "{output}");
    assert!(
        !output.contains("accounts"),
        "the failure needs no account source: {output}"
    );
}

#[test]
fn an_accounts_read_failure_draws_one_note_under_the_local_agents_only_while_it_fails() {
    let failure = || AccountsFailure::Command(HierarchyError::ExitedUnsuccessfully);
    for (source, expected) in [
        (
            AccountsSource::Failed(failure()),
            "accounts: command failed",
        ),
        (
            AccountsSource::FailedKeepingLast {
                last:    EMPTY_SESSION_STATES.clone(),
                failure: failure(),
            },
            "accounts: command failed",
        ),
        (AccountsSource::Ready(EMPTY_SESSION_STATES.clone()), ""),
        (AccountsSource::AwaitingFirstReading, ""),
        (AccountsSource::NotConfigured, ""),
    ] {
        let mut census = Census::new("fixture".into());
        census.apply(CensusUpdate::Local {
            rows:      rows(),
            hierarchy: HierarchyEdges::default().into(),
            stage:     ScanStage::Completed,
        });
        census.record_local_accounts(&source);
        let buffer = draw_at_measured_demand(&census.machines(&[]));
        let output = lines(&buffer);
        let agent_y = output
            .iter()
            .position(|line| line.contains("visible-production-session"))
            .expect("agents remain visible");
        let notes: Vec<&str> = output
            .iter()
            .map(|line| line.trim())
            .filter(|line| line.starts_with("accounts:"))
            .collect();
        if expected.is_empty() {
            assert!(notes.is_empty(), "{source:?}: {output:?}");
        } else {
            assert_eq!(notes, [expected], "{source:?}: {output:?}");
            assert_eq!(output[agent_y + 1].trim(), expected, "{output:?}");
        }
    }
}
