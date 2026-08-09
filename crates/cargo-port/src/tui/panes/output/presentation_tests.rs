//! What the Output pane shows, derived from the two inputs the renderer takes:
//! the compile-visibility state and the monitor snapshot.

use std::num::NonZeroU64;
use std::sync::OnceLock;

use super::constants::MINIMUM_READABLE_COLUMN_WIDTH;
use super::presentation::MonitorColumns;
use super::presentation::MonitorEmptyState;
use super::presentation::MonitorEmptyStateIndexNote;
use super::presentation::MonitorPresentation;
use super::presentation::MonitorVisibility;
use super::presentation::OutputCopyAvailability;
use super::presentation::OutputPaneVisibility;
use super::presentation::OutputPresentation;
use super::presentation::OwnedOutputVisibility;
use super::selection::OutputCursor;
use super::selection::OutputCursorTarget;
use crate::build_monitor;
use crate::build_monitor::BuildSessionId;
use crate::build_monitor::BuildTerminationLifecycle;
use crate::build_monitor::BuildTerminationLifecycleRegistry;
use crate::build_monitor::ClassifiedRoot;
use crate::build_monitor::CompileActivityId;
use crate::build_monitor::CompilerAttribution;
use crate::build_monitor::MonitorSnapshot;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_observation::identity::ProcessIncarnation;
use crate::process_termination::TerminationTargetResult;
use crate::project::AbsolutePath;
use crate::tui::OwnedRunId;
use crate::tui::compile_visibility::CompileVisibilityState;
use crate::tui::compile_visibility::MonitorScopeKey;
use crate::tui::compile_visibility::MonitorScopeResolution;
use crate::tui::compile_visibility::MonitorScopeResolutionRevision;
use crate::tui::compile_visibility::MonitorWorkspaceIndexReadiness;
use crate::tui::state::Inflight;
use crate::tui::state::OwnedRunCompletionMarker;
use crate::tui::state::OwnedRunOutputStateRef;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;
use crate::tui::state::inflight::OwnedRunFixture;
use crate::tui::state::inflight::OwnedRunIdentityRef;

/// A monitor switched on over a scope the index has not resolved.
fn unresolved_scope(monitor_scope_resolution: MonitorScopeResolution) -> CompileVisibilityState {
    CompileVisibilityState::on_for_test(monitor_scope_resolution)
}

/// A monitor switched on over an actionable scope.
fn actionable_scope() -> CompileVisibilityState {
    CompileVisibilityState::on_for_test(MonitorScopeResolution::Ready(MonitorScopeKey::for_test(
        AbsolutePath::from(std::path::Path::new("/tmp/cargo-port-presentation")),
    )))
}

/// A revision naming no index at all.
fn no_index_yet() -> MonitorScopeResolutionRevision {
    MonitorScopeResolutionRevision::for_test(MonitorWorkspaceIndexReadiness::Uninitialized)
}

fn empty_termination_lifecycle_registry() -> &'static BuildTerminationLifecycleRegistry {
    static REGISTRY: OnceLock<BuildTerminationLifecycleRegistry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(BuildTerminationLifecycleRegistry::default)
}

/// Derive with no owned output retained.
fn derive_without_owned_output<'a>(
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

/// The monitor half of a presentation that has one.
fn monitor_of<'a>(output_presentation: &OutputPresentation<'a>) -> MonitorPresentation<'a> {
    let MonitorVisibility::On(monitor_presentation) = output_presentation.monitor() else {
        panic!("this presentation has a monitor half");
    };
    monitor_presentation
}

/// The columns a monitor with sessions is showing.
fn columns_of<'a>(output_presentation: &OutputPresentation<'a>) -> MonitorColumns<'a> {
    let MonitorPresentation::Columns(monitor_columns) = monitor_of(output_presentation) else {
        panic!("this monitor is showing columns");
    };
    monitor_columns
}

/// The empty state a monitor with no columns is showing.
fn empty_state_of(output_presentation: &OutputPresentation<'_>) -> MonitorEmptyState {
    let MonitorPresentation::Empty(monitor_empty_presentation) = monitor_of(output_presentation)
    else {
        panic!("this monitor is showing no columns");
    };
    monitor_empty_presentation.monitor_empty_state()
}

/// Each of the four scope resolutions the index has not settled draws its own
/// message and names the index it was resolved against, so a pending
/// resolution over no index reads differently from one over a stale index.
#[test]
fn every_unresolved_scope_names_itself_and_the_index_behind_it() {
    let monitor_snapshot = MonitorSnapshot::Pending;
    let cases = [
        (
            MonitorScopeResolution::PendingIndex(no_index_yet()),
            MonitorEmptyState::PendingIndex(MonitorWorkspaceIndexReadiness::Uninitialized),
        ),
        (
            MonitorScopeResolution::EmptyNonRust(no_index_yet()),
            MonitorEmptyState::EmptyNonRust(MonitorWorkspaceIndexReadiness::Uninitialized),
        ),
        (
            MonitorScopeResolution::AmbiguousOwnership(no_index_yet()),
            MonitorEmptyState::AmbiguousOwnership(MonitorWorkspaceIndexReadiness::Uninitialized),
        ),
        (
            MonitorScopeResolution::UnresolvedPath(no_index_yet()),
            MonitorEmptyState::UnresolvedPath(MonitorWorkspaceIndexReadiness::Uninitialized),
        ),
    ];

    let mut messages = Vec::new();
    for (monitor_scope_resolution, expected_empty_state) in cases {
        let compile_visibility_state = unresolved_scope(monitor_scope_resolution);
        let output_presentation =
            derive_without_owned_output(&compile_visibility_state, &monitor_snapshot);

        assert_eq!(empty_state_of(&output_presentation), expected_empty_state);
        let message = expected_empty_state.message();
        assert_eq!(
            message.index_note(),
            MonitorEmptyStateIndexNote::Index("no index yet")
        );
        let headline = message.headline();
        assert!(!messages.contains(&headline), "{headline} is not distinct");
        messages.push(headline);

        // The pane stays on the bottom row and in the tab order, so the user can
        // reach the message that explains why there is nothing to see.
        assert_eq!(
            output_presentation.pane_visibility(),
            OutputPaneVisibility::Visible
        );
        assert_eq!(
            output_presentation.copy_availability(),
            OutputCopyAvailability::Unavailable
        );
    }
}

/// An actionable scope leaves the snapshot to explain the emptiness: a first
/// cycle that has not landed and an exhausted observation read differently.
#[test]
fn an_actionable_scope_lets_the_snapshot_explain_the_emptiness() {
    let compile_visibility_state = actionable_scope();

    let awaiting =
        derive_without_owned_output(&compile_visibility_state, &MonitorSnapshot::Pending);
    assert_eq!(
        empty_state_of(&awaiting),
        MonitorEmptyState::AwaitingFirstCycle
    );

    let unavailable =
        derive_without_owned_output(&compile_visibility_state, &MonitorSnapshot::Unavailable);
    assert_eq!(empty_state_of(&unavailable), MonitorEmptyState::Unavailable);
}

/// Visibility switched off and a snapshot that is off are the same fact reached
/// from two directions: either one draws no monitor at all.
#[test]
fn a_monitor_that_is_off_draws_nothing() {
    let off_state = CompileVisibilityState::Off;
    assert_eq!(
        derive_without_owned_output(&off_state, &MonitorSnapshot::Pending).monitor(),
        MonitorVisibility::Off
    );

    let on_state = actionable_scope();
    assert_eq!(
        derive_without_owned_output(&on_state, &MonitorSnapshot::Off).monitor(),
        MonitorVisibility::Off
    );
    assert_eq!(
        derive_without_owned_output(&on_state, &MonitorSnapshot::Off).pane_visibility(),
        OutputPaneVisibility::Hidden
    );
}

/// Output that names no producer, and output whose producer has emitted
/// nothing, are both nothing to draw: the pane stays off the bottom row.
#[test]
fn output_with_no_producer_or_no_lines_is_not_drawn() {
    let off_state = CompileVisibilityState::Off;
    let monitor_snapshot = MonitorSnapshot::Off;

    let absent = derive_without_owned_output(&off_state, &monitor_snapshot);
    assert_eq!(absent, OutputPresentation::Hidden);

    let lineless = OutputPresentation::derive(
        &off_state,
        &monitor_snapshot,
        empty_termination_lifecycle_registry(),
        OwnedRunOutputStateRef::Retained {
            producer: OwnedRunId::for_test(NonZeroU64::MIN),
            title:    OwnedRunOutputTitleRef::Unavailable,
            lines:    &[],
        },
        OwnedRunRunningLabelRef::NotRunning,
        OwnedRunCompletionMarker::NotCompleted,
    );
    assert_eq!(lineless, OutputPresentation::Hidden);
    assert_eq!(
        lineless.copy_availability(),
        OutputCopyAvailability::Unavailable
    );
}

/// Retained output keeps naming the run that produced it while the next run is
/// already queued, so the body on screen is never relabelled with an identity
/// that produced none of it.
#[test]
fn retained_output_stays_attributed_to_the_run_that_produced_it() {
    let off_state = CompileVisibilityState::Off;
    let monitor_snapshot = MonitorSnapshot::Off;
    let line = "first run said this";
    // Driven through the lifecycle, so the retaining run and the current
    // identity differ the way they do on screen: run N's output is retained
    // while run N+1 is already queued.
    let OwnedRunFixture::Built { inflight, producer } =
        Inflight::with_retained_output_and_next_run_queued(line)
    else {
        panic!("a fresh owned-run slot queues a launch");
    };

    let output_presentation = OutputPresentation::derive(
        &off_state,
        &monitor_snapshot,
        empty_termination_lifecycle_registry(),
        inflight.owned_run().output_state(),
        inflight.owned_run().running_label(),
        inflight.owned_run().completion_marker(),
    );

    let OwnedOutputVisibility::OnScreen(owned_output_presentation) =
        output_presentation.owned_output()
    else {
        panic!("retained output with lines is drawn");
    };
    assert_eq!(owned_output_presentation.producer(), producer);
    // The lifecycle frames the run's own output with its launch and done
    // lines, and every one of them belongs to the producer.
    assert!(
        output_presentation
            .captured_lines()
            .iter()
            .any(|captured| captured == line),
        "the producer's output is what is drawn"
    );
    // The lifecycle has already moved on to the queued run, and the body on
    // screen is still labelled with the run that wrote it.
    assert!(
        !matches!(
            inflight.owned_run().identity(),
            OwnedRunIdentityRef::Current(current) if *current == producer
        ),
        "the current identity is the queued run, not the producer"
    );
    assert_eq!(
        output_presentation.pane_visibility(),
        OutputPaneVisibility::Visible
    );
    assert_eq!(
        output_presentation.copy_availability(),
        OutputCopyAvailability::CapturedOutput
    );
}

/// An enabled monitor beside pinned owned output keeps both halves: the monitor
/// is drawn and the captured output stays copyable.
#[test]
fn owned_output_pinned_beside_the_monitor_keeps_both_halves() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = MonitorSnapshot::Pending;
    let lines = vec!["still pinned".to_string()];

    let output_presentation = OutputPresentation::derive(
        &compile_visibility_state,
        &monitor_snapshot,
        empty_termination_lifecycle_registry(),
        OwnedRunOutputStateRef::Retained {
            producer: OwnedRunId::for_test(NonZeroU64::MIN),
            title:    OwnedRunOutputTitleRef::Named("cargo build"),
            lines:    &lines,
        },
        OwnedRunRunningLabelRef::NotRunning,
        OwnedRunCompletionMarker::Done,
    );

    assert!(matches!(
        output_presentation.monitor(),
        MonitorVisibility::On(_)
    ));
    assert!(matches!(
        output_presentation.owned_output(),
        OwnedOutputVisibility::OnScreen(_)
    ));
    assert_eq!(
        output_presentation.copy_availability(),
        OutputCopyAvailability::CapturedOutput
    );
}

/// A monitor with no owned output has nothing a copy gesture may read: an
/// external column is a live sample of observed processes, not a transcript.
#[test]
fn an_external_only_monitor_has_nothing_to_copy() {
    let compile_visibility_state = actionable_scope();
    let output_presentation =
        derive_without_owned_output(&compile_visibility_state, &MonitorSnapshot::Pending);

    assert_eq!(
        output_presentation.copy_availability(),
        OutputCopyAvailability::Unavailable
    );
    assert!(output_presentation.captured_lines().is_empty());
}

/// A monitor snapshot holding one column per root, classified by the real
/// classifier so the rows are the ones the pane draws.
fn classified_columns(classified_roots: &[ClassifiedRoot]) -> MonitorSnapshot {
    match build_monitor::classified_monitor_snapshot(classified_roots) {
        Ok(monitor_snapshot) => monitor_snapshot,
        Err(error) => panic!("the classification fixture builds a snapshot: {error}"),
    }
}

/// A snapshot of Cargo roots with no compiler under any of them.
fn columns_without_compilers(root_pids: &[u32]) -> MonitorSnapshot {
    let classified_roots: Vec<ClassifiedRoot> = root_pids
        .iter()
        .map(|root_pid| ClassifiedRoot {
            root_pid:      *root_pid,
            compiler_pids: &[],
        })
        .collect();
    classified_columns(&classified_roots)
}

/// Transaction lifecycle belongs to the monitor, not the replaceable rows;
/// presentation joins it by immutable session identity when it constructs a
/// column for rendering.
#[test]
fn monitor_columns_join_the_termination_lifecycle_registry() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = columns_without_compilers(&[4_242]);
    let MonitorSnapshot::Fresh(monitor_data) = &monitor_snapshot else {
        panic!("the classification fixture creates fresh monitor rows");
    };
    let build_session_id = monitor_data.session_rows()[0].build_session_id();
    let mut build_termination_lifecycle_registry = BuildTerminationLifecycleRegistry::default();
    build_termination_lifecycle_registry.mark_terminating_for_test(build_session_id);

    let output_presentation = OutputPresentation::derive(
        &compile_visibility_state,
        &monitor_snapshot,
        &build_termination_lifecycle_registry,
        OwnedRunOutputStateRef::Absent,
        OwnedRunRunningLabelRef::NotRunning,
        OwnedRunCompletionMarker::NotCompleted,
    );
    let monitor_columns = columns_of(&output_presentation);
    assert_eq!(monitor_columns.terminal_records().count(), 0);
    let Some(monitor_column) = monitor_columns.columns().next() else {
        panic!("the fresh monitor draws its session column");
    };

    assert_eq!(
        monitor_column.build_termination_lifecycle(),
        BuildTerminationLifecycle::Terminating
    );
}

#[test]
fn empty_monitor_projects_terminal_records_without_a_current_row() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = columns_without_compilers(&[4_243]);
    let MonitorSnapshot::Fresh(monitor_data) = &monitor_snapshot else {
        panic!("the classification fixture creates fresh monitor rows");
    };
    let monitor_session_row = &monitor_data.session_rows()[0];
    let build_session_id = monitor_session_row.build_session_id().clone();
    let mut build_termination_lifecycle_registry = BuildTerminationLifecycleRegistry::default();
    build_termination_lifecycle_registry.record_external_terminal_for_test(
        monitor_session_row,
        TerminationTargetResult::SignaledButUnconfirmed { pid: 4_243 },
    );

    let output_presentation = OutputPresentation::derive(
        &compile_visibility_state,
        &MonitorSnapshot::Unavailable,
        &build_termination_lifecycle_registry,
        OwnedRunOutputStateRef::Absent,
        OwnedRunRunningLabelRef::NotRunning,
        OwnedRunCompletionMarker::NotCompleted,
    );
    let MonitorPresentation::Empty(monitor_empty_presentation) = monitor_of(&output_presentation)
    else {
        panic!("a rowless monitor should use the empty presentation");
    };
    let records: Vec<_> = monitor_empty_presentation.terminal_records().collect();

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].display_identity().build_session_id(),
        &build_session_id
    );
}

/// Classification hands each session the activities this cycle attributed to
/// it, so one session's compilers never appear under the session beside it and
/// a session with no compiler under it renders no activity rows at all.
#[test]
fn a_session_row_carries_only_the_activities_attributed_to_it() {
    let monitor_snapshot = classified_columns(&[
        ClassifiedRoot {
            root_pid:      201,
            compiler_pids: &[2011, 2012],
        },
        ClassifiedRoot {
            root_pid:      202,
            compiler_pids: &[2021],
        },
        ClassifiedRoot {
            root_pid:      203,
            compiler_pids: &[],
        },
    ]);
    let compile_visibility_state = actionable_scope();
    let output_presentation =
        derive_without_owned_output(&compile_visibility_state, &monitor_snapshot);
    let monitor_columns = columns_of(&output_presentation);

    assert_eq!(monitor_columns.len(), 3);
    let expected_activity_counts = [2, 1, 0];
    for (monitor_column, expected_activities) in
        monitor_columns.columns().zip(expected_activity_counts)
    {
        let build_session_id = monitor_column.build_session_id();
        let compile_activities = monitor_column.session_row().compile_activities();
        assert_eq!(
            compile_activities.len(),
            expected_activities,
            "a session holds its own compilers and no others"
        );
        for compile_activity in compile_activities {
            assert_eq!(
                compile_activity.compiler_attribution(),
                &CompilerAttribution::Confirmed(build_session_id.clone()),
                "every activity in this row was attributed to this row's session"
            );
        }
    }
}

/// One column takes the whole width; several split down to the readable
/// minimum and the rest are held off screen in the model, and the window
/// scrolls only as far as keeping the selected column on screen requires.
#[test]
fn the_column_window_stays_readable_and_keeps_the_selection_on_screen() {
    let compile_visibility_state = actionable_scope();

    let single = columns_without_compilers(&[301]);
    let single_presentation = derive_without_owned_output(&compile_visibility_state, &single);
    let single_columns = columns_of(&single_presentation);
    let single_window = single_columns.window(MINIMUM_READABLE_COLUMN_WIDTH * 4, 0);
    assert_eq!(
        (single_window.first(), single_window.count()),
        (0, 1),
        "one column takes the whole width"
    );

    let several = columns_without_compilers(&[401, 402, 403, 404]);
    let several_presentation = derive_without_owned_output(&compile_visibility_state, &several);
    let several_columns = columns_of(&several_presentation);
    assert_eq!(several_columns.len(), 4);

    // A width that fits every column windows none of them away, wherever the
    // selection sits.
    let fits_all = several_columns.window(MINIMUM_READABLE_COLUMN_WIDTH * 4, 3);
    assert_eq!(
        (fits_all.first(), fits_all.count()),
        (0, 4),
        "every column fits, so every column is drawn"
    );

    // A width that fits three of four draws exactly three: the first three
    // selections need no scroll, and only the last one moves the window right.
    let fits_three = MINIMUM_READABLE_COLUMN_WIDTH * 3;
    for (selected_index, expected_first) in [(0, 0), (1, 0), (2, 0), (3, 1)] {
        let monitor_column_window = several_columns.window(fits_three, selected_index);
        assert_eq!(
            (monitor_column_window.first(), monitor_column_window.count()),
            (expected_first, 3),
            "column {selected_index} at a width that fits three of four"
        );
        assert!(
            monitor_column_window.contains(selected_index),
            "column {selected_index} stays on screen"
        );
    }

    // A width below one readable column still draws one, and that one is
    // whichever column the cursor is on.
    for selected_index in 0..several_columns.len() {
        let narrow = several_columns.window(1, selected_index);
        assert_eq!(
            (narrow.first(), narrow.count()),
            (selected_index, 1),
            "the one column a sub-minimum width draws is the selected one"
        );
    }
}

/// A cursor on an activity that exited falls back to its session's header, and
/// a cursor on a session that exited falls back to the column now at its index.
/// A same-PID exec makes both identities new, so neither is found and the
/// cursor falls back rather than pointing at a process it never named.
#[test]
fn a_cursor_falls_back_when_what_it_named_is_gone() {
    let compile_visibility_state = actionable_scope();
    let monitor_snapshot = columns_without_compilers(&[501, 502]);
    let output_presentation =
        derive_without_owned_output(&compile_visibility_state, &monitor_snapshot);
    let monitor_columns = columns_of(&output_presentation);
    let build_session_ids: Vec<_> = monitor_columns
        .columns()
        .map(|monitor_column| monitor_column.build_session_id().clone())
        .collect();

    // An activity that is not in this cycle's rows: the unit exited.
    let mut after_unit_exit = OutputCursor::empty();
    after_unit_exit.focus_activity(
        CompileActivityId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(9001, 1),
            "exited unit",
        )),
        build_session_ids[1].clone(),
        1,
        3,
    );
    after_unit_exit.reconcile(&output_presentation);
    assert_eq!(
        after_unit_exit.target(),
        &OutputCursorTarget::Header(build_session_ids[1].clone()),
        "the cursor falls back to the session it was under"
    );
    assert_eq!(after_unit_exit.column_index(), 1);

    // A session that is not in this cycle's rows: the Cargo root exited.
    let mut after_session_exit = OutputCursor::empty();
    after_session_exit.focus_header(
        BuildSessionId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(9002, 1),
            "exited session",
        )),
        1,
    );
    after_session_exit.reconcile(&output_presentation);
    assert_eq!(
        after_session_exit.target(),
        &OutputCursorTarget::Header(build_session_ids[1].clone()),
        "the cursor takes the session now at its index"
    );

    // A same-PID exec: the pid matches a live session, the exec token does not.
    let live_pid = monitor_columns
        .columns()
        .next()
        .map_or(0, |monitor_column| {
            monitor_column
                .session_row()
                .build_session()
                .root_observation()
                .root_pid()
        });
    let mut after_exec = OutputCursor::empty();
    after_exec.focus_header(
        BuildSessionId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(live_pid, u64::MAX),
            "re-execed root",
        )),
        0,
    );
    after_exec.reconcile(&output_presentation);
    assert_eq!(
        after_exec.target(),
        &OutputCursorTarget::Header(build_session_ids[0].clone()),
        "a re-execed pid is a different session, so the cursor falls back"
    );
}
