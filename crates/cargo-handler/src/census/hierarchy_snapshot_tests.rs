//! Hierarchy discovery keeps production sessions placed through transient failures.

#![expect(
    clippy::expect_used,
    reason = "fixture rows and scripted answers must exist"
)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tui_pane::TerminalApp;

use super::*;
use crate::agent_cell;
use crate::app::App;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::Census;
use crate::census::CensusUpdate;
use crate::census::HierarchyCommand;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::scan::ScanStage;
use crate::census::schedule;
use crate::footer_colors::FooterColors;

const SHOWRUNNER: &str = "4dbdb78e-a83b-4248-b8f9-ff278dcbc60c";
const UNIT: &str = "fc0ab8f8-54e4-4c8e-9eed-89b6be9766d0";

fn production_rows() -> Vec<AgentRow> {
    [
        (3_208_779, SHOWRUNNER, "production-showrunner"),
        (2_778_913, UNIT, "cargo-handler-unit"),
    ]
    .map(|(pid, session, name)| {
        serde_json::from_value(serde_json::json!({
            "agent":"claude", "session_id":session, "name":name,
            "started":100,"pid":pid,"directory":"/fixture", "children":[]
        }))
        .expect("session ids from the live natedev scan")
    })
    .into()
}

fn assert_unit_draws_under_showrunner(rows: Vec<AgentRow>, edges: HierarchyEdges) {
    let state = MachineState::Answered(rows, edges.into());
    let machines = [Machine {
        name:  "natedev",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    assert_eq!(
        placement.roots().len(),
        1,
        "the unit is not an unassigned root"
    );
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 180, 10);
    let mut buffer = Buffer::empty(area);
    agent_cell::draw(
        &mut buffer,
        area,
        &cells[0],
        &cells,
        200,
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    let lines: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let member = lines
        .iter()
        .find(|line| line.contains("cargo-handler-unit"))
        .expect("the managed unit remains visible in its showrunner tile");
    assert!(
        member
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair == ["cargo-handler-unit", "unit"]),
        "{member}"
    );
    assert!(
        member.contains('└'),
        "the unit has a managed tree rail: {member}"
    );
}

#[test]
fn hierarchy_live_session_ids_keep_the_unit_nested_across_failure_and_recovery() {
    struct Attempts(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);
    impl HierarchyRunner for Attempts {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            self.0
                .lock()
                .expect("attempt queue")
                .pop_front()
                .expect("next attempt")
        }
    }
    let edges = parse(
        &serde_json::to_vec(&serde_json::json!({
            "schema":1,"edges":[{"manager":SHOWRUNNER,"member":UNIT,"role":"unit"}]
        }))
        .expect("production edge encodes"),
    )
    .expect("production edge parses");
    let runner = Attempts(Mutex::new(VecDeque::from([
        Ok(edges.clone()),
        Err(HierarchyError::TimedOut),
        Ok(edges),
    ])));
    let mut cache = HierarchyCache::default();
    let source = HierarchySource::Command("fixture production command".into());
    let now = Instant::now();
    for (moment, expected) in [
        (now, HierarchyNotice::Clear),
        (
            now + HIERARCHY_REFRESH_INTERVAL,
            HierarchyNotice::Failed(HierarchyError::TimedOut),
        ),
        (
            now + HIERARCHY_REFRESH_INTERVAL + Duration::from_millis(1),
            HierarchyNotice::Clear,
        ),
    ] {
        let rows = production_rows();
        let discovered = cache.snapshot(source.clone(), &rows, &runner, moment);
        assert_eq!(discovered.notice, expected);
        assert_unit_draws_under_showrunner(rows, discovered.edges);
    }
    assert!(runner.0.lock().expect("attempt queue").is_empty());
}

#[test]
fn hierarchy_scheduler_publishes_failure_and_recovery_without_losing_managed_rows() {
    struct PublicationAttempts(Mutex<VecDeque<Result<HierarchyEdges, HierarchyError>>>);
    impl HierarchyRunner for PublicationAttempts {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            self.0
                .lock()
                .expect("publication queue")
                .pop_front()
                .expect("next publication")
        }
    }
    let links = HierarchyEdges(vec![HierarchyEdge {
        reviewer: crate::census::hierarchy::ReviewerState::NotPool,
        manager:  SHOWRUNNER.into(),
        member:   UNIT.into(),
        role:     HierarchyRole::Unit,
    }]);
    let runner = Arc::new(PublicationAttempts(Mutex::new(VecDeque::from([
        Ok(links.clone()),
        Err(HierarchyError::TimedOut),
        Ok(links),
    ]))));
    let updates = schedule::spawn_with_test_source(
        HierarchyCommand::new(HierarchySource::Command("fixture hierarchy".into())),
        runner,
        |sender| {
            let now = Instant::now();
            sender
                .publish_at(ScanStage::Completed, production_rows(), now)
                .expect("successful publication");
            let mut changed = production_rows();
            let mut additional = changed[0].clone();
            additional.pid = 99;
            additional.session_id =
                SessionIdentity::Claude("new-session-triggers-discovery".into());
            changed.push(additional);
            sender
                .publish_at(
                    ScanStage::Completed,
                    changed.clone(),
                    now + Duration::from_secs(2),
                )
                .expect("failed discovery publication");
            sender
                .publish_at(ScanStage::Completed, changed, now + Duration::from_secs(4))
                .expect("recovered publication");
        },
    );
    let mut completions = updates.into_iter().filter(|update| {
        matches!(
            update,
            CensusUpdate::Local {
                stage: ScanStage::Completed,
                ..
            }
        )
    });
    let mut census = Census::new("natedev".into());
    for expected in [
        HierarchyNotice::Clear,
        HierarchyNotice::Failed(HierarchyError::TimedOut),
        HierarchyNotice::Clear,
    ] {
        let update = completions
            .next()
            .expect("background scheduler completes each scan");
        census.apply(update);
        let machines = census.machines(&[]);
        assert_eq!(machines[0].state.hierarchy_notice(), expected);
        let placement = AgentPlacement::from_machines(&machines);
        assert!(
            !placement.is_root("natedev", 2_778_913),
            "failure must not unassign the unit"
        );
        assert_eq!(machines[0].state.edges().len(), 1);
    }
}

#[test]
fn hierarchy_configured_unmatched_and_disabled_sources_publish_distinct_outcomes() {
    struct ForeignSessions;
    impl HierarchyRunner for ForeignSessions {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            Ok(HierarchyEdges(vec![HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  "another-production".into(),
                member:   "another-session".into(),
                role:     HierarchyRole::Unit,
            }]))
        }
    }
    let mut cache = HierarchyCache::default();
    let rows = production_rows();
    let now = Instant::now();
    let unmatched = cache.snapshot(
        HierarchySource::Command("fixture command".into()),
        &rows,
        &ForeignSessions,
        now,
    );
    assert_eq!(unmatched.notice, HierarchyNotice::NoSessionMatched);
    assert_eq!(
        unmatched.edges.0.len(),
        1,
        "unmatched does not discard a valid command answer"
    );
    let disabled = cache.snapshot(HierarchySource::Off, &rows, &ForeignSessions, now);
    assert_eq!(disabled.notice, HierarchyNotice::Clear);
    assert!(
        disabled.edges.0.is_empty(),
        "disabling discovery clears prior links"
    );
}

#[test]
fn hierarchy_classified_sessions_draw_roles_before_slow_details_complete() {
    struct ProductionTree;
    impl HierarchyRunner for ProductionTree {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            parse(include_bytes!("fixtures/hierarchy_startup.json"))
        }
    }
    let (release_details, details_released) = mpsc::channel();
    let (completed, completion) = mpsc::channel();
    let updates = schedule::spawn_with_test_source(
        HierarchyCommand::new(HierarchySource::Command("fixture production tree".into())),
        Arc::new(ProductionTree),
        move |sender| {
            let rows = production_rows();
            sender
                .publish(ScanStage::Classified, rows.clone())
                .expect("publish classified sessions");
            details_released
                .recv()
                .expect("hold slow detail reads until roles arrive");
            sender
                .publish(ScanStage::Completed, rows)
                .expect("publish completed details");
            completed.send(()).expect("record detail completion");
        },
    );
    let classified = updates.recv_timeout(Duration::from_millis(200));
    // `release_details.send(())` unblocks `details_released.recv()` before
    // assertions, even when the update receive operations timed out.
    release_details.send(()).expect("release detail reads");
    completion
        .recv_timeout(Duration::from_millis(200))
        .expect("the source finishes promptly");
    let mut census = Census::new("natedev".into());
    census.apply(classified.expect("classified roles reach the frame before details"));
    let machines = census.machines(&[]);
    assert_eq!(machines[0].state.rows().len(), 2);
    let machines = census.machines(&[]);
    assert_eq!(machines[0].state.hierarchy_notice(), HierarchyNotice::Clear);
    assert_unit_draws_under_showrunner(
        machines[0].state.rows().to_vec(),
        HierarchyEdges(machines[0].state.edges().to_vec()),
    );
}

#[test]
fn unmatched_hierarchy_note_stays_drawn_while_expired_links_await_refresh() {
    struct ForeignHierarchy;
    impl HierarchyRunner for ForeignHierarchy {
        fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
            Ok(HierarchyEdges(vec![HierarchyEdge {
                reviewer: crate::census::hierarchy::ReviewerState::NotPool,
                manager:  "foreign-manager".into(),
                member:   "foreign-unit".into(),
                role:     HierarchyRole::Unit,
            }]))
        }
    }
    let mut cache = HierarchyCache::default();
    let source = HierarchySource::Command("fixture foreign hierarchy".into());
    cache.snapshot(
        source.clone(),
        &production_rows(),
        &ForeignHierarchy,
        Instant::now()
            .checked_sub(HIERARCHY_REFRESH_INTERVAL)
            .expect("the fixture clock supports an expired hierarchy"),
    );
    // Simulate the classified publication before a slow refresh by reading
    // `HierarchyCache::retained` with its expired links and notice.
    let retained = cache.retained(source);
    let mut app = App::new_for_test().expect("test app");
    app.census = Census::new("natedev".into());
    app.census.apply(CensusUpdate::Local {
        rows:      production_rows(),
        hierarchy: retained,
        stage:     ScanStage::Classified,
    });
    app.before_draw();
    let keymap = app.keymap();
    let mut terminal = Terminal::new(TestBackend::new(180, 40)).expect("test terminal");
    terminal
        .draw(|frame| app.draw(frame, &keymap))
        .expect("draw retained notice");
    let buffer = terminal.backend().buffer();
    let output = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(output.contains("hierarchy: no session matched"), "{output}");
    assert_eq!(
        cache.retained(HierarchySource::Off).notice,
        HierarchyNotice::Clear,
        "a replacement source clears the old outcome"
    );
}
