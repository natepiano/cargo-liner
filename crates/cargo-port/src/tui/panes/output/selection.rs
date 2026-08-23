use std::rc::Rc;

/// The output pane's selection sub-mode.
///
/// In `Normal` the selection is the single row under the cursor and plain
/// motions move it whole (the anchor follows the cursor). In `Visual` —
/// the vim visual-line sub-mode (`V`) — plain motions grow the range from
/// the fixed anchor.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionMode {
    Normal,
    Visual,
}

/// Which buffer the visual selection reads.
///
/// A selection that has stopped tracking the streaming tail reads the buffer it
/// was frozen against, so a child process still writing cannot move a range the
/// user already picked.
#[derive(Clone)]
pub(super) enum VisualSelectionSource {
    /// The selection follows the tail and reads whatever output is live.
    LiveOutput,
    /// The selection is pinned against this frozen buffer.
    Frozen(Rc<[String]>),
}

impl VisualSelectionSource {
    /// The lines the selection and the copy payload read.
    pub(super) fn lines<'a>(&'a self, live: &'a [String]) -> &'a [String] {
        match self {
            Self::LiveOutput => live,
            Self::Frozen(frozen) => frozen,
        }
    }
}

/// The rows the selection currently covers.
///
/// An empty buffer has no rows to name, which is a different fact from a
/// one-row selection at index zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputSelectionRange {
    /// There is nothing to select.
    Empty,
    /// The inclusive row range the selection covers.
    Rows { first: usize, last: usize },
}

impl OutputSelectionRange {
    /// How many rows the range covers.
    pub const fn line_count(self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Rows { first, last } => last - first + 1,
        }
    }

    /// Whether `row` is inside the range.
    pub const fn contains(self, row: usize) -> bool {
        match self {
            Self::Empty => false,
            Self::Rows { first, last } => row >= first && row <= last,
        }
    }
}

/// Linewise selection state for the output pane.
///
/// There is always a selection — at minimum the single row under the
/// cursor — so the pane has no separate select/deselect mode. `anchor`
/// is the fixed end; the moving end is `OutputPane::viewport`'s `pos`,
/// and the selected range runs between them. `mode` is the
/// [`SelectionMode`] that decides how plain motions read.
///
/// `visual_selection_source` names which buffer the range reads, so a streaming
/// child process can't drift a pinned range.
pub struct OutputSelection {
    pub(super) anchor:                  usize,
    pub(super) selection_mode:          SelectionMode,
    pub(super) visual_selection_source: VisualSelectionSource,
}

impl OutputSelection {
    pub(super) const fn new() -> Self {
        Self {
            anchor:                  0,
            selection_mode:          SelectionMode::Normal,
            visual_selection_source: VisualSelectionSource::LiveOutput,
        }
    }

    /// Whether the vim visual-line sub-mode is active.
    pub const fn is_visual(&self) -> bool { matches!(self.selection_mode, SelectionMode::Visual) }

    /// Which buffer the selection reads.
    pub(super) const fn visual_selection_source(&self) -> &VisualSelectionSource {
        &self.visual_selection_source
    }
}
