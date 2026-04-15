use super::types::Pane;
use super::types::PaneId;

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
    /// Look up a pane by ID. Exhaustive match — adding a `PaneId` variant
    /// forces you to decide which pane it maps to.
    pub const fn by_id(&self, id: PaneId) -> &Pane {
        match id {
            PaneId::Package
            | PaneId::ProjectList
            | PaneId::Output
            | PaneId::Search
            | PaneId::Finder => &self.package,
            PaneId::Lang => &self.lang,
            PaneId::Git => &self.git,
            PaneId::Targets => &self.targets,
            PaneId::CiRuns => &self.ci,
            PaneId::Toasts => &self.toasts,
            PaneId::Lints => &self.lints,
            PaneId::Settings => &self.settings,
            PaneId::Keymap => &self.keymap,
        }
    }

    pub const fn by_id_mut(&mut self, id: PaneId) -> &mut Pane {
        match id {
            PaneId::Package
            | PaneId::ProjectList
            | PaneId::Output
            | PaneId::Search
            | PaneId::Finder => &mut self.package,
            PaneId::Lang => &mut self.lang,
            PaneId::Git => &mut self.git,
            PaneId::Targets => &mut self.targets,
            PaneId::CiRuns => &mut self.ci,
            PaneId::Toasts => &mut self.toasts,
            PaneId::Lints => &mut self.lints,
            PaneId::Settings => &mut self.settings,
            PaneId::Keymap => &mut self.keymap,
        }
    }

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
