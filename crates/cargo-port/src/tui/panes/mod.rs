use super::types::Pane;

// ── PaneManager ────────────────────────────────────────────────────

/// Owns all pane navigation state. Extracted from `App` so render
/// functions can borrow `PaneManager` mutably while borrowing `App`
/// immutably for project data.
pub(in super::super) struct PaneManager {
    pub package:  Pane,
    pub lang:     Pane,
    pub git:      Pane,
    pub targets:  Pane,
    pub ci:       Pane,
    pub toasts:   Pane,
    pub lints:    Pane,
    pub settings: Pane,
    pub keymap:   Pane,
}

impl PaneManager {
    pub const fn new() -> Self {
        Self {
            package:  Pane::new(),
            lang:     Pane::new(),
            git:      Pane::new(),
            targets:  Pane::new(),
            ci:       Pane::new(),
            toasts:   Pane::new(),
            lints:    Pane::new(),
            settings: Pane::new(),
            keymap:   Pane::new(),
        }
    }
}
