use super::detail::GitData;
use super::detail::PackageData;
use super::detail::TargetsData;
use super::types::Pane;
use super::types::PaneId;

// ── PaneManager ────────────────────────────────────────────────────

/// Owns all pane navigation state and per-pane data models.
///
/// Extracted from `App` so render functions can borrow `PaneManager`
/// mutably while borrowing `App` immutably for project data. Each pane
/// owns its display data — no shared monolithic struct.
pub(in super::super) struct PaneManager {
    pub package:      Pane,
    pub lang:         Pane,
    pub git:          Pane,
    pub targets:      Pane,
    pub ci:           Pane,
    pub toasts:       Pane,
    pub lints:        Pane,
    pub settings:     Pane,
    pub keymap:       Pane,
    // Per-pane data models — populated when the selected project changes.
    pub package_data: Option<PackageData>,
    pub git_data:     Option<GitData>,
    pub targets_data: Option<TargetsData>,
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
            package:      Pane::new(),
            lang:         Pane::new(),
            git:          Pane::new(),
            targets:      Pane::new(),
            ci:           Pane::new(),
            toasts:       Pane::new(),
            lints:        Pane::new(),
            settings:     Pane::new(),
            keymap:       Pane::new(),
            package_data: None,
            git_data:     None,
            targets_data: None,
        }
    }

    /// Populate per-pane data from a `DetailInfo`. Called when the
    /// selected project changes or detail cache is rebuilt.
    pub fn set_detail_data(&mut self, info: &super::detail::DetailInfo) {
        self.package_data = Some(info.package_data());
        self.git_data = Some(info.git_data());
        self.targets_data = Some(info.targets_data());
    }

    /// Clear per-pane data (e.g., when no project is selected).
    pub fn clear_detail_data(&mut self) {
        self.package_data = None;
        self.git_data = None;
        self.targets_data = None;
    }
}
