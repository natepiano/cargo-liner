//! What the keyboard and the mouse do to the Output pane once the monitor is
//! drawing columns: which row a motion lands on, which key the pane keeps and
//! which it lets through to the framework, and what a copy gesture reads.
//!
//! The presentations here come from the real classifier, so a column is the
//! owned one because the cycle attributed its root to the owned run — the same
//! evidence production reads.

use std::num::NonZeroU64;

use tui_pane::CopySelectionResult;
use tui_pane::NavAction;

use super::ColumnSelection;
use super::OutputMonitorHit;
use super::OutputPane;
use super::OutputPresentation;
use super::OutputTabStep;
use super::OwnedColumnSelection;
use super::SelectedBuildTerminationSelection;
use super::presentation::MonitorColumns;
use super::presentation::MonitorPresentation;
use super::presentation::MonitorVisibility;
use super::selection::OutputCursorTarget;
use crate::build_monitor::BuildSessionId;
use crate::build_monitor::BuildTerminationLifecycleRegistry;
use crate::build_monitor::ClassifiedRoot;
use crate::build_monitor::FixtureRootOwnership;
use crate::build_monitor::MonitorSessionOwnership;
use crate::build_monitor::MonitorSnapshot;
use crate::build_monitor::classified_monitor_snapshot_with_ownership;
use crate::project::AbsolutePath;
use crate::tui::OwnedRunId;
use crate::tui::compile_visibility::CompileVisibilityState;
use crate::tui::compile_visibility::MonitorScopeKey;
use crate::tui::compile_visibility::MonitorScopeResolution;
use crate::tui::state::OwnedRunCompletionMarker;
use crate::tui::state::OwnedRunOutputStateRef;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;

/// The root Cargo Port launched, and the compilers running under it.
const OWNED_ROOT_PID: u32 = 7100;
const OWNED_COMPILER_PIDS: &[u32] = &[7101, 7102, 7103];
/// A Cargo run by someone else on the same host.
const EXTERNAL_ROOT_PID: u32 = 7200;
const EXTERNAL_COMPILER_PIDS: &[u32] = &[7201, 7202];
/// How many activity rows the owned column draws.
const OWNED_ACTIVITY_ROWS: usize = OWNED_COMPILER_PIDS.len();
/// How many captured lines the retained owned output holds.
const CAPTURED_LINES: usize = 3;

fn empty_termination_lifecycle_registry() -> &'static BuildTerminationLifecycleRegistry {
    static REGISTRY: std::sync::OnceLock<BuildTerminationLifecycleRegistry> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(BuildTerminationLifecycleRegistry::default)
}

fn owned_run_id() -> OwnedRunId { OwnedRunId::for_test(NonZeroU64::MIN) }

/// A monitor switched on over an actionable scope.
fn actionable_scope() -> CompileVisibilityState {
    CompileVisibilityState::on_for_test(MonitorScopeResolution::Ready(MonitorScopeKey::for_test(
        AbsolutePath::from(std::path::Path::new("/tmp/cargo-port-interaction")),
    )))
}

/// The lines the retained owned output holds.
fn captured_lines() -> Vec<String> {
    (0..CAPTURED_LINES)
        .map(|line| format!("captured line {line}"))
        .collect()
}

/// One owned column and one external column, both classified this cycle.
fn owned_and_external_snapshot() -> MonitorSnapshot {
    owned_and_external_snapshot_for(owned_run_id())
}

fn owned_and_external_snapshot_for(owned_run_id: OwnedRunId) -> MonitorSnapshot {
    staged_snapshot_for(
        &[
            ClassifiedRoot {
                root_pid:      OWNED_ROOT_PID,
                compiler_pids: OWNED_COMPILER_PIDS,
            },
            ClassifiedRoot {
                root_pid:      EXTERNAL_ROOT_PID,
                compiler_pids: EXTERNAL_COMPILER_PIDS,
            },
        ],
        owned_run_id,
    )
}

/// Only the owned column, for the cases that turn on there being no column
/// beside it.
fn owned_only_snapshot() -> MonitorSnapshot {
    staged_snapshot(&[ClassifiedRoot {
        root_pid:      OWNED_ROOT_PID,
        compiler_pids: OWNED_COMPILER_PIDS,
    }])
}

fn staged_snapshot(classified_roots: &[ClassifiedRoot]) -> MonitorSnapshot {
    staged_snapshot_for(classified_roots, owned_run_id())
}

fn staged_snapshot_for(
    classified_roots: &[ClassifiedRoot],
    owned_run_id: OwnedRunId,
) -> MonitorSnapshot {
    match classified_monitor_snapshot_with_ownership(
        classified_roots,
        &FixtureRootOwnership::OwnedRoot {
            root_pid: OWNED_ROOT_PID,
            owned_run_id,
        },
    ) {
        Ok(monitor_snapshot) => monitor_snapshot,
        Err(error) => panic!("the classification fixture builds a snapshot: {error}"),
    }
}

/// Derive with the owned run's output retained, which is what pins Cargo Port's
/// own body beside the monitor.
fn derive_with_captured_output<'a>(
    compile_visibility_state: &'a CompileVisibilityState,
    monitor_snapshot: &'a MonitorSnapshot,
    lines: &'a [String],
) -> OutputPresentation<'a> {
    derive_with_captured_output_for(
        compile_visibility_state,
        monitor_snapshot,
        owned_run_id(),
        lines,
    )
}

fn derive_with_captured_output_for<'a>(
    compile_visibility_state: &'a CompileVisibilityState,
    monitor_snapshot: &'a MonitorSnapshot,
    producer: OwnedRunId,
    lines: &'a [String],
) -> OutputPresentation<'a> {
    OutputPresentation::derive(
        compile_visibility_state,
        monitor_snapshot,
        empty_termination_lifecycle_registry(),
        OwnedRunOutputStateRef::Retained {
            producer,
            title: OwnedRunOutputTitleRef::Named("cargo build"),
            lines,
        },
        OwnedRunRunningLabelRef::NotRunning,
        OwnedRunCompletionMarker::Done,
    )
}

/// Derive with nothing retained, so no column holds captured output.
fn derive_without_captured_output<'a>(
    compile_visibility_state: &'a CompileVisibilityState,
    monitor_snapshot: &'a MonitorSnapshot,
) -> OutputPresentation<'a> {
    OutputPresentation::derive(
        compile_visibility_state,
        monitor_snapshot,
        empty_termination_lifecycle_registry(),
        OwnedRunOutputStateRef::Absent,
        OwnedRunRunningLabelRef::NotRunning,
        OwnedRunCompletionMarker::NotCompleted,
    )
}

fn columns_of<'a>(output_presentation: &OutputPresentation<'a>) -> MonitorColumns<'a> {
    let MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) =
        output_presentation.monitor()
    else {
        panic!("this monitor is showing columns");
    };
    monitor_columns
}

/// Where the owned column sits among the drawn columns, and its identity.
fn owned_column(output_presentation: &OutputPresentation<'_>) -> (usize, BuildSessionId) {
    columns_of(output_presentation)
        .columns()
        .enumerate()
        .find_map(|(column_index, monitor_column)| {
            matches!(
                monitor_column.session_ownership(),
                MonitorSessionOwnership::Owned(_)
            )
            .then(|| (column_index, monitor_column.build_session_id().clone()))
        })
        .unwrap_or_else(|| panic!("the staged cycle attributes one root to the owned run"))
}

/// The identity of the column at `column_index`.
fn column_id(output_presentation: &OutputPresentation<'_>, column_index: usize) -> BuildSessionId {
    columns_of(output_presentation)
        .columns()
        .nth(column_index)
        .unwrap_or_else(|| panic!("column {column_index} is drawn"))
        .build_session_id()
        .clone()
}

/// Put the cursor on a column's header the way a click on it does.
fn click_header(pane: &mut OutputPane, build_session_id: BuildSessionId, column_index: usize) {
    pane.focus_hit(&OutputMonitorHit::Header {
        build_session_id,
        column_index,
    });
}

#[test]
fn selected_build_termination_targets_the_activity_columns_root_session() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let output_presentation =
        derive_without_captured_output(&compile_visibility_state, &monitor_snapshot);
    let (external_index, external_column) = columns_of(&output_presentation)
        .columns()
        .enumerate()
        .find(|(_, monitor_column)| {
            matches!(
                monitor_column.session_ownership(),
                MonitorSessionOwnership::External
            )
        })
        .unwrap_or_else(|| panic!("fixture includes an external build column"));
    let build_session_id = external_column.build_session_id().clone();
    let compile_activity = external_column.session_row().compile_activities()[0]
        .compile_activity_id()
        .clone();
    let compiler_child_count = external_column.session_row().compile_activities().len();
    let mut output_pane = OutputPane::new();
    output_pane.focus_hit(&OutputMonitorHit::Activity {
        compile_activity_id: compile_activity,
        build_session_id:    build_session_id.clone(),
        column_index:        external_index,
        row_index:           0,
    });

    let SelectedBuildTerminationSelection::SelectedBuild(selected_build_termination_display_target) =
        output_pane.selected_build_termination_selection(&output_presentation)
    else {
        panic!("an activity cursor should resolve its owning session column");
    };
    assert_eq!(
        selected_build_termination_display_target.build_session_id(),
        &build_session_id
    );
    let (_, selected_build_termination_confirmation_display) =
        selected_build_termination_display_target.into_parts();
    assert_eq!(
        selected_build_termination_confirmation_display.compiler_child_count(),
        compiler_child_count,
        "the selected activity identifies the root column, not one compiler child"
    );
}

#[test]
fn selected_build_termination_targets_captured_output_through_its_owned_column() {
    let compile_visibility_state = actionable_scope();
    let unrelated_root_pid = 7_000;
    let monitor_snapshot = staged_snapshot(&[
        ClassifiedRoot {
            root_pid:      unrelated_root_pid,
            compiler_pids: &[7_001],
        },
        ClassifiedRoot {
            root_pid:      OWNED_ROOT_PID,
            compiler_pids: OWNED_COMPILER_PIDS,
        },
    ]);
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let first_column_id = column_id(&output_presentation, 0);
    let (_, owned_column_id) = owned_column(&output_presentation);
    assert_ne!(
        first_column_id, owned_column_id,
        "the fixture puts an unrelated build ahead of the owned producer"
    );
    let mut output_pane = OutputPane::new();
    output_pane.focus_hit(&OutputMonitorHit::CapturedOutput {
        producer: owned_run_id(),
        row:      0,
    });

    let SelectedBuildTerminationSelection::SelectedBuild(selected_build_termination_display_target) =
        output_pane.selected_build_termination_selection(&output_presentation)
    else {
        panic!("captured output with a current owned column should select its producer");
    };

    assert_eq!(
        selected_build_termination_display_target.build_session_id(),
        &owned_column_id,
        "captured output targets its owned producer rather than the first unrelated column"
    );
}

#[test]
fn selected_build_termination_refuses_captured_output_after_owned_run_replacement_before_reconcile()
{
    let compile_visibility_state = actionable_scope();
    let retained_producer = owned_run_id();
    let current_producer = OwnedRunId::for_test(NonZeroU64::MAX);
    let monitor_snapshot = owned_and_external_snapshot_for(current_producer);
    let lines = captured_lines();
    let output_presentation = derive_with_captured_output_for(
        &compile_visibility_state,
        &monitor_snapshot,
        current_producer,
        &lines,
    );
    let mut output_pane = OutputPane::new();
    output_pane.focus_hit(&OutputMonitorHit::CapturedOutput {
        producer: retained_producer,
        row:      0,
    });

    assert!(matches!(
        output_pane.selected_build_termination_selection(&output_presentation),
        SelectedBuildTerminationSelection::NoBuildSelected
    ));
}

/// Apply one keyboard motion with the monitor on, where the viewport motion the
/// pane would have used off-monitor is never reached.
fn navigate(pane: &mut OutputPane, output_presentation: &OutputPresentation<'_>, nav: NavAction) {
    pane.navigate_action(output_presentation, nav, |_| {});
}

/// The separator between a session's activity rows and its captured output is
/// drawn, not occupied: one Down from the last activity row lands on the first
/// captured line, and one Up comes straight back.
#[test]
fn down_from_the_last_activity_row_crosses_the_output_separator_in_one_step() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let (owned_index, owned_id) = owned_column(&output_presentation);

    let mut pane = OutputPane::new();
    click_header(&mut pane, owned_id, owned_index);

    for row_index in 0..OWNED_ACTIVITY_ROWS {
        navigate(&mut pane, &output_presentation, NavAction::Down);
        assert!(
            matches!(pane.cursor().target(), OutputCursorTarget::Activity(_)),
            "row {row_index} of the owned column is an activity row"
        );
        assert_eq!(pane.cursor().row_index(), row_index);
    }

    navigate(&mut pane, &output_presentation, NavAction::Down);
    assert!(
        matches!(
            pane.cursor().target(),
            OutputCursorTarget::CapturedOutput(_)
        ),
        "one step past the last activity row is the first captured line"
    );
    assert_eq!(pane.viewport.pos(), 0);

    navigate(&mut pane, &output_presentation, NavAction::Up);
    assert!(matches!(
        pane.cursor().target(),
        OutputCursorTarget::Activity(_)
    ));
    assert_eq!(
        pane.cursor().row_index(),
        OWNED_ACTIVITY_ROWS - 1,
        "coming back up lands on the last activity row, not on the separator"
    );
}

/// The column body has two ends and the cursor stops at both: Home and Up hold
/// at the header, End and Down hold at the last captured line.
#[test]
fn row_traversal_stops_at_both_ends_of_the_column_body() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let (owned_index, owned_id) = owned_column(&output_presentation);

    let mut pane = OutputPane::new();
    click_header(&mut pane, owned_id.clone(), owned_index);

    navigate(&mut pane, &output_presentation, NavAction::End);
    assert!(matches!(
        pane.cursor().target(),
        OutputCursorTarget::CapturedOutput(_)
    ));
    assert_eq!(pane.viewport.pos(), CAPTURED_LINES - 1);

    navigate(&mut pane, &output_presentation, NavAction::Down);
    assert_eq!(
        pane.viewport.pos(),
        CAPTURED_LINES - 1,
        "Down at the bottom of the body stays there"
    );

    navigate(&mut pane, &output_presentation, NavAction::Home);
    assert_eq!(
        pane.cursor().target(),
        &OutputCursorTarget::Header(owned_id.clone())
    );

    navigate(&mut pane, &output_presentation, NavAction::Up);
    assert_eq!(
        pane.cursor().target(),
        &OutputCursorTarget::Header(owned_id),
        "Up at the header stays on the header"
    );
}

/// A page motion steps by the rows the column draws, so a column taller than
/// the pane is paged through rather than walked a row at a time.
#[test]
fn page_motions_step_by_the_rows_the_column_draws() {
    const VISIBLE_ROWS: usize = 2;

    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let (owned_index, owned_id) = owned_column(&output_presentation);

    let mut pane = OutputPane::new();
    click_header(&mut pane, owned_id.clone(), owned_index);
    pane.sync_column_scroll(VISIBLE_ROWS, OWNED_ACTIVITY_ROWS);

    navigate(&mut pane, &output_presentation, NavAction::PageDown);
    assert!(matches!(
        pane.cursor().target(),
        OutputCursorTarget::Activity(_)
    ));
    assert_eq!(
        pane.cursor().row_index(),
        VISIBLE_ROWS - 1,
        "a page down from the header steps a page into the activity rows"
    );

    navigate(&mut pane, &output_presentation, NavAction::PageUp);
    assert_eq!(
        pane.cursor().target(),
        &OutputCursorTarget::Header(owned_id),
        "a page up from one page in returns to the header"
    );
}

/// Left and Right select the adjacent column and hold at the ends rather than
/// wrapping around.
#[test]
fn left_and_right_walk_the_columns_and_hold_at_the_ends() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let first_id = column_id(&output_presentation, 0);
    let second_id = column_id(&output_presentation, 1);

    let mut pane = OutputPane::new();
    click_header(&mut pane, first_id.clone(), 0);

    navigate(&mut pane, &output_presentation, NavAction::Right);
    assert_eq!(pane.cursor().column_index(), 1);
    assert_eq!(
        pane.cursor().target(),
        &OutputCursorTarget::Header(second_id)
    );

    navigate(&mut pane, &output_presentation, NavAction::Right);
    assert_eq!(
        pane.cursor().column_index(),
        1,
        "there is no column past the last one"
    );

    navigate(&mut pane, &output_presentation, NavAction::Left);
    assert_eq!(pane.cursor().column_index(), 0);
    assert_eq!(
        pane.cursor().target(),
        &OutputCursorTarget::Header(first_id)
    );

    navigate(&mut pane, &output_presentation, NavAction::Left);
    assert_eq!(
        pane.cursor().column_index(),
        0,
        "there is no column before the first one"
    );
}

/// Tab and Shift-Tab step between columns while there is one to step to, and
/// report no column at the ends so the key falls through to the framework's
/// pane snaking.
#[test]
fn tab_steps_between_columns_and_falls_through_at_the_ends() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let first_id = column_id(&output_presentation, 0);

    let mut pane = OutputPane::new();
    click_header(&mut pane, first_id, 0);

    assert_eq!(
        pane.tab_to_adjacent_column(&output_presentation, OutputTabStep::PreviousColumn),
        ColumnSelection::NoSuchColumn,
        "Shift-Tab on the first column falls through"
    );
    assert_eq!(
        pane.tab_to_adjacent_column(&output_presentation, OutputTabStep::NextColumn),
        ColumnSelection::Moved
    );
    assert_eq!(pane.cursor().column_index(), 1);
    assert_eq!(
        pane.tab_to_adjacent_column(&output_presentation, OutputTabStep::NextColumn),
        ColumnSelection::NoSuchColumn,
        "Tab on the last column falls through"
    );
    assert_eq!(
        pane.tab_to_adjacent_column(&output_presentation, OutputTabStep::PreviousColumn),
        ColumnSelection::Moved
    );
    assert_eq!(pane.cursor().column_index(), 0);
}

/// With one column there is nowhere to tab to, and with the monitor off there
/// are no columns at all: Tab belongs to the framework in both.
#[test]
fn tab_falls_through_without_a_second_column_and_with_the_monitor_off() {
    let compile_visibility_state = actionable_scope();
    let lines = captured_lines();

    let single = owned_only_snapshot();
    let single_presentation =
        derive_with_captured_output(&compile_visibility_state, &single, &lines);
    let mut pane = OutputPane::new();
    for output_tab_step in [OutputTabStep::NextColumn, OutputTabStep::PreviousColumn] {
        assert_eq!(
            pane.tab_to_adjacent_column(&single_presentation, output_tab_step),
            ColumnSelection::NoSuchColumn,
            "one column is nothing to tab between"
        );
    }

    let off_state = CompileVisibilityState::Off;
    let off_presentation = derive_with_captured_output(&off_state, &MonitorSnapshot::Off, &lines);
    for output_tab_step in [OutputTabStep::NextColumn, OutputTabStep::PreviousColumn] {
        assert_eq!(
            pane.tab_to_adjacent_column(&off_presentation, output_tab_step),
            ColumnSelection::NoSuchColumn,
            "a monitor that is off draws no columns to tab between"
        );
    }
}

/// A copy gesture reads the row the cursor is on: an activity row copies its
/// own text, a header copies nothing, and captured output copies the buffer.
#[test]
fn a_copy_gesture_reads_the_row_the_cursor_is_on() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let (owned_index, owned_id) = owned_column(&output_presentation);

    let mut pane = OutputPane::new();
    click_header(&mut pane, owned_id, owned_index);
    assert_eq!(
        pane.copy_payload_for_cursor(&output_presentation),
        CopySelectionResult::Nothing,
        "a column header has no transcript behind it"
    );

    let mut activity_texts = Vec::new();
    for _ in 0..OWNED_ACTIVITY_ROWS {
        navigate(&mut pane, &output_presentation, NavAction::Down);
        let CopySelectionResult::Payload(copy_payload) =
            pane.copy_payload_for_cursor(&output_presentation)
        else {
            panic!("an activity row copies its own text");
        };
        assert!(
            copy_payload.text.starts_with("rustc "),
            "the row names the compiler it runs: {}",
            copy_payload.text
        );
        assert!(
            !activity_texts.contains(&copy_payload.text),
            "each activity row copies its own row, not the one above it"
        );
        activity_texts.push(copy_payload.text);
    }

    navigate(&mut pane, &output_presentation, NavAction::Down);
    let CopySelectionResult::Payload(copy_payload) =
        pane.copy_payload_for_cursor(&output_presentation)
    else {
        panic!("captured output copies the selected range");
    };
    assert!(
        copy_payload.text.contains("captured line 0"),
        "the captured buffer is what the owned column's output copies: {}",
        copy_payload.text
    );
}

/// A visual selection belongs to Cargo Port's own output. The owned column
/// answers that it is selected from any of its rows; an external column never
/// does, and neither does anything at all once nothing is retained.
#[test]
fn only_the_owned_column_reports_itself_selected() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let (owned_index, owned_id) = owned_column(&output_presentation);
    let external_index = 1 - owned_index;
    let external_id = column_id(&output_presentation, external_index);

    let mut pane = OutputPane::new();
    click_header(&mut pane, external_id, external_index);
    assert_eq!(
        pane.owned_column_selection(&output_presentation),
        OwnedColumnSelection::NotSelected,
        "an external column is not Cargo Port's own"
    );

    click_header(&mut pane, owned_id, owned_index);
    assert_eq!(
        pane.owned_column_selection(&output_presentation),
        OwnedColumnSelection::Selected,
        "the owned column's header is part of the owned column"
    );

    navigate(&mut pane, &output_presentation, NavAction::End);
    assert_eq!(
        pane.owned_column_selection(&output_presentation),
        OwnedColumnSelection::Selected,
        "so is its captured output"
    );

    let unretained = derive_without_captured_output(&compile_visibility_state, &monitor_snapshot);
    assert_eq!(
        pane.owned_column_selection(&unretained),
        OwnedColumnSelection::NotSelected,
        "with nothing retained there is no owned body to select"
    );
}

/// With the monitor off the pane is Cargo Port's own captured output and
/// nothing else: motions drive the viewport exactly as they did before the
/// monitor existed, and the whole pane is the owned body.
#[test]
fn a_monitor_that_is_off_drives_the_captured_output_buffer() {
    let off_state = CompileVisibilityState::Off;
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&off_state, &MonitorSnapshot::Off, &lines);

    let mut pane = OutputPane::new();
    pane.navigate_action(&output_presentation, NavAction::Down, |viewport| {
        viewport.set_pos(CAPTURED_LINES - 1);
    });
    assert_eq!(
        pane.viewport.pos(),
        CAPTURED_LINES - 1,
        "the viewport motion the caller passed is what moved the buffer"
    );

    assert_eq!(
        pane.owned_column_selection(&output_presentation),
        OwnedColumnSelection::Selected,
        "with no columns drawn the owned body is the whole pane"
    );
    assert!(
        matches!(
            pane.copy_payload_for_cursor(&output_presentation),
            CopySelectionResult::Payload(_)
        ),
        "the captured buffer is copyable with the monitor off"
    );
}

/// A click aims the cursor at the row it landed on, including the captured
/// output rows, which also scroll the buffer to the clicked line.
#[test]
fn a_click_moves_the_cursor_onto_the_row_it_landed_on() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = owned_and_external_snapshot();
    let lines = captured_lines();
    let output_presentation =
        derive_with_captured_output(&compile_visibility_state, &monitor_snapshot, &lines);
    let (owned_index, owned_id) = owned_column(&output_presentation);
    let compile_activity_id = columns_of(&output_presentation)
        .columns()
        .nth(owned_index)
        .and_then(|monitor_column| {
            monitor_column
                .session_row()
                .compile_activities()
                .first()
                .map(|compile_activity| compile_activity.compile_activity_id().clone())
        })
        .unwrap_or_else(|| panic!("the owned column draws activity rows"));

    let mut pane = OutputPane::new();
    pane.focus_hit(&OutputMonitorHit::Activity {
        compile_activity_id: compile_activity_id.clone(),
        build_session_id:    owned_id,
        column_index:        owned_index,
        row_index:           0,
    });
    assert_eq!(
        pane.cursor().target(),
        &OutputCursorTarget::Activity(compile_activity_id)
    );
    assert_eq!(pane.cursor().column_index(), owned_index);
    assert_eq!(pane.cursor().row_index(), 0);

    pane.focus_hit(&OutputMonitorHit::CapturedOutput {
        producer: owned_run_id(),
        row:      CAPTURED_LINES - 1,
    });
    assert!(matches!(
        pane.cursor().target(),
        OutputCursorTarget::CapturedOutput(_)
    ));
    assert_eq!(pane.viewport.pos(), CAPTURED_LINES - 1);
    assert_eq!(
        pane.owned_column_selection(&output_presentation),
        OwnedColumnSelection::Selected,
        "a click into the captured output selects the owned column"
    );
}
