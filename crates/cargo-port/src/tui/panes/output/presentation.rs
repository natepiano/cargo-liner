//! The one value the Output pane is drawn from.
//!
//! Layout, visibility, focus reconciliation, tabbability, the bottom-row action
//! labels, copy availability, and rendering all read [`OutputPresentation`], so
//! none of them can disagree about what the pane is currently showing.

use crate::tui::state::OwnedRunOutputStateRef;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;

/// Whether the Output pane occupies the bottom row this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputPaneVisibility {
    /// The diagnostics panes own the bottom row.
    Hidden,
    /// The Output pane is drawn, focusable, and in the tab order.
    Visible,
}

/// Whether the pane currently has anything a copy gesture may read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputCopyAvailability {
    /// Nothing on screen is captured output.
    Unavailable,
    /// The owned run's captured output is on screen and may be copied.
    CapturedOutput,
}

/// The Cargo Port-owned run's own body: its retained output and lifecycle
/// label, keyed by the run that produced them.
///
/// The producer is the retaining run, never the current lifecycle identity, so
/// run N's output stays attributed to N while run N+1 is queued or starting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedOutputPresentation<'a> {
    title:         OwnedRunOutputTitleRef<'a>,
    running_label: OwnedRunRunningLabelRef<'a>,
    lines:         &'a [String],
}

impl<'a> OwnedOutputPresentation<'a> {
    /// The title the retained output was captured under.
    pub(super) const fn title(&self) -> OwnedRunOutputTitleRef<'a> { self.title }

    /// The current run's running label, when one is running.
    pub(super) const fn running_label(&self) -> OwnedRunRunningLabelRef<'a> { self.running_label }

    /// The captured lines, borrowed rather than copied.
    pub(super) const fn lines(&self) -> &'a [String] { self.lines }
}

/// Everything the Output pane shows this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputPresentation<'a> {
    /// Nothing to show: the diagnostics panes own the bottom row.
    Hidden,
    /// The owned run's captured output is on screen.
    Owned(OwnedOutputPresentation<'a>),
}

impl<'a> OutputPresentation<'a> {
    /// Derive what the pane shows from the owned run's retained output.
    pub const fn derive(
        owned_run_output_state: OwnedRunOutputStateRef<'a>,
        owned_run_running_label: OwnedRunRunningLabelRef<'a>,
    ) -> Self {
        match owned_run_output_state {
            OwnedRunOutputStateRef::Retained { title, lines, .. } if !lines.is_empty() => {
                Self::Owned(OwnedOutputPresentation {
                    title,
                    running_label: owned_run_running_label,
                    lines,
                })
            },
            // No producer, or a producer that has emitted nothing yet: either
            // way there is nothing to draw, and the pane stays off the bottom
            // row until there is.
            OwnedRunOutputStateRef::Absent | OwnedRunOutputStateRef::Retained { .. } => {
                Self::Hidden
            },
        }
    }

    /// Whether the pane is drawn, focusable, and in the tab order.
    pub const fn pane_visibility(&self) -> OutputPaneVisibility {
        match self {
            Self::Hidden => OutputPaneVisibility::Hidden,
            Self::Owned(_) => OutputPaneVisibility::Visible,
        }
    }

    /// Whether a copy gesture has captured output to read.
    pub const fn copy_availability(&self) -> OutputCopyAvailability {
        match self {
            Self::Hidden => OutputCopyAvailability::Unavailable,
            Self::Owned(_) => OutputCopyAvailability::CapturedOutput,
        }
    }

    /// The owned body on screen this frame, when there is one.
    pub(super) const fn owned_output(&self) -> Option<OwnedOutputPresentation<'a>> {
        match self {
            Self::Hidden => None,
            Self::Owned(owned) => Some(*owned),
        }
    }

    /// The captured lines a copy or visual selection reads.
    pub const fn captured_lines(&self) -> &'a [String] {
        match self {
            Self::Hidden => &[],
            Self::Owned(owned) => owned.lines(),
        }
    }
}
