use super::types::Pane;
use super::types::PaneId;

// ── PaneManager ────────────────────────────────────────────────────

/// Owns all pane navigation state. Extracted from `App` so render
/// functions can borrow `PaneManager` mutably while borrowing `App`
/// immutably for project data.
pub(in super::super) struct PaneManager {
    pub project_list: Pane,
    pub package:      Pane,
    pub git:          Pane,
    pub targets:      Pane,
    pub ci:           Pane,
    pub toasts:       Pane,
    pub lints:        Pane,
    pub settings:     Pane,
    pub keymap:       Pane,
    pub output:       Pane,
}

impl PaneManager {
    pub const fn new() -> Self {
        Self {
            project_list: Pane::new(),
            package:      Pane::new(),
            git:          Pane::new(),
            targets:      Pane::new(),
            ci:           Pane::new(),
            toasts:       Pane::new(),
            lints:        Pane::new(),
            settings:     Pane::new(),
            keymap:       Pane::new(),
            output:       Pane::new(),
        }
    }

    pub const fn by_id(&self, id: PaneId) -> &Pane {
        match id {
            PaneId::ProjectList | PaneId::Search | PaneId::Finder => &self.project_list,
            PaneId::Package => &self.package,
            PaneId::Git => &self.git,
            PaneId::Targets => &self.targets,
            PaneId::CiRuns => &self.ci,
            PaneId::Toasts => &self.toasts,
            PaneId::Lints => &self.lints,
            PaneId::Settings => &self.settings,
            PaneId::Keymap => &self.keymap,
            PaneId::Output => &self.output,
        }
    }

    pub const fn by_id_mut(&mut self, id: PaneId) -> &mut Pane {
        match id {
            PaneId::ProjectList | PaneId::Search | PaneId::Finder => &mut self.project_list,
            PaneId::Package => &mut self.package,
            PaneId::Git => &mut self.git,
            PaneId::Targets => &mut self.targets,
            PaneId::CiRuns => &mut self.ci,
            PaneId::Toasts => &mut self.toasts,
            PaneId::Lints => &mut self.lints,
            PaneId::Settings => &mut self.settings,
            PaneId::Keymap => &mut self.keymap,
            PaneId::Output => &mut self.output,
        }
    }
}
