//! Focused tests for what the Output pane shows.

use std::num::NonZeroU64;

use super::presentation::OutputCopyAvailability;
use super::presentation::OutputPaneVisibility;
use super::presentation::OutputPresentation;
use crate::tui::OwnedRunId;
use crate::tui::state::OwnedRunOutputStateRef;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;

/// Output that names no producer, and output whose producer has emitted
/// nothing, are both nothing to draw: the pane stays off the bottom row.
#[test]
fn output_with_no_producer_or_no_lines_is_not_drawn() {
    let absent = OutputPresentation::derive(
        OwnedRunOutputStateRef::Absent,
        OwnedRunRunningLabelRef::NotRunning,
    );
    assert_eq!(absent, OutputPresentation::Hidden);
    assert_eq!(absent.pane_visibility(), OutputPaneVisibility::Hidden);

    let lineless = OutputPresentation::derive(
        OwnedRunOutputStateRef::Retained {
            producer: OwnedRunId::for_test(NonZeroU64::MIN),
            title:    OwnedRunOutputTitleRef::Unavailable,
            lines:    &[],
        },
        OwnedRunRunningLabelRef::NotRunning,
    );
    assert_eq!(lineless, OutputPresentation::Hidden);
    assert_eq!(
        lineless.copy_availability(),
        OutputCopyAvailability::Unavailable
    );
}
