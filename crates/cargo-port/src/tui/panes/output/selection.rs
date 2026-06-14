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

/// Linewise selection state for the output pane.
///
/// There is always a selection — at minimum the single row under the
/// cursor — so the pane has no separate select/deselect mode. `anchor`
/// is the fixed end; the moving end is `OutputPane::viewport`'s `pos`,
/// and the selected range runs between them. `mode` is the
/// [`SelectionMode`] that decides how plain motions read.
///
/// `snapshot` freezes the buffer once the selection stops tracking the
/// live tail, so a streaming child process can't drift a pinned range.
/// While the selection follows the tail it stays `None` and render/yank
/// read the live buffer.
pub struct OutputSelection {
    pub(super) anchor:         usize,
    pub(super) selection_mode: SelectionMode,
    pub(super) snapshot:       Option<Rc<[String]>>,
}

impl OutputSelection {
    pub(super) const fn new() -> Self {
        Self {
            anchor:         0,
            selection_mode: SelectionMode::Normal,
            snapshot:       None,
        }
    }

    /// Whether the vim visual-line sub-mode is active.
    pub const fn is_visual(&self) -> bool { matches!(self.selection_mode, SelectionMode::Visual) }

    /// The frozen buffer snapshot, present once the selection has stopped
    /// following the live tail.
    pub const fn snapshot(&self) -> Option<&Rc<[String]>> { self.snapshot.as_ref() }
}
