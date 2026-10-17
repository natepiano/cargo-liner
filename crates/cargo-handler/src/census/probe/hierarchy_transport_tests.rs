//! Remote probe diagnostics retain their reason and the receiver's placement links.

#![expect(clippy::expect_used, reason = "fixture probes and rows must decode")]

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::agent_cell;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::HierarchyEdge;
use crate::census::HierarchyRole;
use crate::census::Machine;
use crate::census::RemoteHierarchyAnswers;
use crate::census::accounts::AccountsTable;
use crate::footer_colors::FooterColors;
use crate::sort::SummarySort;
use crate::summary;
use crate::summary::SummaryFrame;

fn rows() -> Vec<AgentRow> {
    [
        (10, "boss", "remote-showrunner"),
        (20, "unit", "remote-unit"),
    ]
    .map(|(pid, session_id, name)| {
        serde_json::from_value(serde_json::json!({
            "agent":"claude", "session_id":session_id, "name":name, "pid":pid,
            "started":100,"directory":"/fixture","children":[]
        }))
        .expect("remote session fixture")
    })
    .into()
}

#[test]
fn hierarchy_probe_failure_reason_reaches_the_rendered_remote_note_and_keeps_links() {
    let links = HierarchyEdges(vec![HierarchyEdge {
        manager: "boss".into(),
        member:  "unit".into(),
        role:    HierarchyRole::Unit,
    }]);
    let mut retained = RemoteHierarchyAnswers::default();
    for (notice, wire_edges, expected) in [
        (HierarchyNotice::Clear, links.clone(), ""),
        (
            HierarchyNotice::Failed(HierarchyError::TimedOut),
            HierarchyEdges::default(),
            "hierarchy: timed out",
        ),
        (HierarchyNotice::Clear, links, ""),
    ] {
        let mut report = ProbeReport::from_answer("remote".into(), rows(), wire_edges);
        match notice {
            HierarchyNotice::Failed(reason) => report.record_failure(reason),
            _ => report.hierarchy_notice = notice,
        }
        let encoded = serde_json::to_vec(&report).expect("outgoing report encodes");
        let state = retained.apply("remote", parse_answer(&encoded));
        assert_eq!(state.hierarchy_notice(), notice);
        assert_eq!(
            state.edges().len(),
            1,
            "a failed probe retains its last successful links"
        );
        let machines = [Machine {
            name:  "remote",
            state: &state,
        }];
        let placement = AgentPlacement::from_machines(&machines);
        assert!(
            !placement.is_root("remote", 20),
            "the remote unit stays managed"
        );
        let cells = agent_cell::cell_order_placed(&placement);
        let accounts = AccountsTable::NotConfigured;
        let colors = FooterColors::default();
        let frame = SummaryFrame {
            machines:       &machines,
            placement:      &placement,
            cells:          &cells,
            now:            200,
            account_column: AccountColumn::Hidden,
            footer_colors:  &colors,
            sort:           SummarySort::default(),
            accounts:       &accounts,
        };
        let area = Rect::new(
            0,
            0,
            summary::width(
                &machines,
                &placement,
                200,
                AccountColumn::Hidden,
                SummarySort::default(),
                &accounts,
            ),
            u16::try_from(summary::height(&machines, &placement, &accounts))
                .expect("small remote summary"),
        );
        let mut buffer = Buffer::empty(area);
        summary::draw(&mut buffer, area, &frame);
        let output = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(output.contains("remote-showrunner"), "{output}");
        if expected.is_empty() {
            assert!(!output.contains("hierarchy:"), "{output}");
        } else {
            assert!(output.contains(expected), "{output}");
        }
    }
}

#[test]
fn failed_probe_preserves_links_in_a_receiver_that_only_knows_the_failure_flag() {
    #[derive(Deserialize)]
    struct PreviousReport {
        rows:             Vec<AgentRow>,
        #[serde(default)]
        edges:            HierarchyEdges,
        #[serde(default)]
        hierarchy_failed: bool,
    }

    let links = HierarchyEdges(vec![HierarchyEdge {
        manager: "boss".into(),
        member:  "unit".into(),
        role:    HierarchyRole::Unit,
    }]);
    let successful = ProbeReport::from_answer("remote".into(), rows(), links.clone());
    let mut failed = ProbeReport::from_answer("remote".into(), rows(), HierarchyEdges::default());
    failed.rows[0].name = "fresh remote showrunner".into();
    failed.record_failure(HierarchyError::TimedOut);
    let mut retained = HierarchyEdges::default();
    for report in [successful, failed] {
        let wire = serde_json::to_vec(&report).expect("new probe output");
        let previous: PreviousReport =
            serde_json::from_slice(&wire).expect("previous schema-3 reader");
        if !previous.hierarchy_failed {
            retained = previous.edges;
        }
        assert_eq!(
            retained, links,
            "a failed discovery cannot clear previous placement"
        );
        assert_eq!(previous.rows.len(), 2, "fresh rows still arrive");
        if previous.hierarchy_failed {
            assert_eq!(previous.rows[0].name, "fresh remote showrunner");
            assert_eq!(
                parse_answer(&wire).expect("new receiver").1,
                HierarchyAnswer::Failed(HierarchyError::TimedOut)
            );
        }
    }
}

#[test]
fn hierarchy_probe_transports_unmatched_and_distinct_failures_and_reads_old_reports() {
    for notice in [
        HierarchyNotice::NoSessionMatched,
        HierarchyNotice::Failed(HierarchyError::CouldNotStart),
        HierarchyNotice::Failed(HierarchyError::OutputTooLarge),
        HierarchyNotice::Failed(HierarchyError::UnknownSchema(42)),
    ] {
        let mut report =
            ProbeReport::from_answer("remote".into(), rows(), HierarchyEdges::default());
        report.hierarchy_notice = notice;
        let encoded = serde_json::to_vec(&report).expect("typed report encodes");
        let state = RemoteHierarchyAnswers::default().apply("remote", parse_answer(&encoded));
        assert_eq!(state.hierarchy_notice(), notice);
    }
    for (legacy_failed, expected) in [
        (false, HierarchyNotice::Clear),
        (
            true,
            HierarchyNotice::Failed(HierarchyError::ExitedUnsuccessfully),
        ),
    ] {
        let mut wire = serde_json::json!({"schema":3,"machine":"old-mac","rows":rows(),"edges":[]});
        if legacy_failed {
            wire["hierarchy_failed"] = serde_json::json!(true);
        }
        let state = RemoteHierarchyAnswers::default().apply(
            "old-mac",
            parse_answer(&serde_json::to_vec(&wire).expect("old report encodes")),
        );
        assert_eq!(state.hierarchy_notice(), expected);
        assert_eq!(state.rows().len(), 2, "compatibility retains visible rows");
    }
}
