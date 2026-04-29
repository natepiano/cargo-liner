//! Per-pane unit structs and their `Pane` impls (Phase 7
//! foundation).
//!
//! Phase 7 lands one unit struct per `PaneId` variant with the
//! `PaneId`-pure trait methods (`id`, `has_row_hitboxes`,
//! `size_spec`, `input_context`). The render and input bodies
//! land in Phase 8 (six detail/data panes) and Phase 9 (seven
//! remaining). The structs are zero-sized today; Phase 8 absorbs
//! per-pane state (cursor `Viewport`, content slot,
//! pane-specific extras) onto the relevant struct.
//!
//! These impls are the future trait-dispatch path. During Phase 7
//! the existing `panes::has_row_hitboxes(id)` /
//! `panes::size_spec(id, cpu_width)` / `panes::behavior(id)`
//! free functions remain the primary callers; the trait impls
//! produce the same answers and are pinned by the
//! characterization tests in `panes/spec.rs`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::dispatch::InputContextKind;
use super::dispatch::Pane;
use super::dispatch::PaneRenderCtx;
use super::spec::PaneId;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::cpu::CpuPoller;
use crate::tui::cpu::CpuSnapshot;
use crate::tui::pane::PaneAxisSize;
use crate::tui::pane::PaneSizeSpec;
use crate::tui::pane::Viewport;

// ── ProjectList ─────────────────────────────────────────────────
pub struct ProjectListPane;
impl Pane for ProjectListPane {
    fn id(&self) -> PaneId { PaneId::ProjectList }
    fn input_context(&self) -> InputContextKind { InputContextKind::ProjectList }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

// ── Package ─────────────────────────────────────────────────────
//
// Phase 8.5: cursor `Viewport` absorbed onto PackagePane.
// Phase 8.8: `content: Option<PackageData>` absorbed (was the
// `package` slot in `PaneDataStore`'s detail set).
pub struct PackagePane {
    viewport: Viewport,
    content:  Option<super::PackageData>,
}

impl PackagePane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::PackageData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::PackageData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Pane for PackagePane {
    fn id(&self) -> PaneId { PaneId::Package }
    fn input_context(&self) -> InputContextKind { InputContextKind::DetailFields }
    fn has_row_hitboxes(&self) -> bool { true }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        let styles = super::package::RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         crate::tui::pane::default_pane_chrome(),
        };
        super::package::render_package_pane_body(frame, area, self, &styles, ctx);
    }
}

// ── Lang ────────────────────────────────────────────────────────
//
// Phase 8.2: cursor `Viewport` migrates onto LangPane. Lang has
// no `PaneDataStore` slot today (renderer reads `language_stats`
// directly off the project tree on every render), so there is no
// content-slot relocation to do here. PaneManager keeps its
// vestigial Lang slot.
pub struct LangPane {
    viewport: Viewport,
}

impl LangPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Pane for LangPane {
    fn id(&self) -> PaneId { PaneId::Lang }
    fn input_context(&self) -> InputContextKind { InputContextKind::DetailFields }
    fn has_row_hitboxes(&self) -> bool { true }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        let styles = super::package::RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         crate::tui::pane::default_pane_chrome(),
        };
        super::lang::render_lang_pane_body(frame, area, self, &styles, ctx);
    }
}

// ── Cpu ─────────────────────────────────────────────────────────
//
// Phase 8.1a absorbed `cpu_poller` and `content` onto CpuPane.
// Phase 8.1b absorbs the cursor `Viewport` (was the `Cpu` slot in
// `PaneManager`'s array). The slot in PaneManager stays vestigial
// until Phase 9 migrates the remaining panes; CpuPane is the only
// reader/writer of its cursor state now.
//
// Render body remains in `panes/cpu.rs::render_cpu_panel` as a
// free function. Body migration into the trait method lands in a
// later sub-phase.
pub struct CpuPane {
    viewport: Viewport,
    content:  Option<CpuSnapshot>,
    poller:   CpuPoller,
}

impl CpuPane {
    pub fn new(cfg: &CpuConfig) -> Self {
        let mut pane = Self {
            viewport: Viewport::new(),
            content:  None,
            poller:   CpuPoller::new(cfg),
        };
        pane.install_placeholder();
        pane
    }

    /// Tick the CPU poller. If a fresh snapshot is produced, store
    /// it as the pane's content. Called once per app tick by App.
    pub fn tick(&mut self, now: Instant) {
        if let Some(snapshot) = self.poller.poll_if_due(now) {
            self.content = Some(snapshot);
        }
    }

    /// Recreate the poller for `cfg` and seed `content` with a
    /// placeholder snapshot. Used after a config reload changes
    /// CPU poll behavior.
    pub fn reset(&mut self, cfg: &CpuConfig) {
        self.poller = CpuPoller::new(cfg);
        self.install_placeholder();
    }

    /// Seed `content` with the current poller's placeholder
    /// snapshot without recreating the poller. Used at startup.
    pub fn install_placeholder(&mut self) {
        self.content = Some(self.poller.placeholder_snapshot());
    }

    pub const fn content(&self) -> Option<&CpuSnapshot> { self.content.as_ref() }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Pane for CpuPane {
    fn id(&self) -> PaneId { PaneId::Cpu }
    fn input_context(&self) -> InputContextKind {
        // Today: PaneBehavior::Cpu folds into InputContext::DetailTargets.
        InputContextKind::DetailTargets
    }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, cpu_width: u16) -> PaneSizeSpec {
        PaneSizeSpec {
            width:  PaneAxisSize::Fixed(cpu_width),
            height: PaneAxisSize::Fill(1),
        }
    }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        // Body lives in `panes/cpu.rs` next to its helpers.
        let styles = super::package::RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         crate::tui::pane::default_pane_chrome(),
        };
        super::cpu::render_cpu_pane_body(frame, area, self, &styles, ctx);
    }
}

// ── Git ─────────────────────────────────────────────────────────
//
// Phase 8.6: cursor `Viewport` absorbed onto GitPane.
// Phase 8.8: `content: Option<GitData>` absorbed (was the `git`
// slot in `PaneDataStore`'s detail set).
// `worktree_summary_cache` stays on `Panes` for now per Phase 10
// design (final home is `GitPane` per the doc).
pub struct GitPane {
    viewport: Viewport,
    content:  Option<super::GitData>,
}

impl GitPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::GitData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::GitData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Pane for GitPane {
    fn id(&self) -> PaneId { PaneId::Git }
    fn input_context(&self) -> InputContextKind { InputContextKind::DetailFields }
    fn has_row_hitboxes(&self) -> bool {
        // Git registers its own hitboxes from `render_git_pane_body` because
        // rows don't map 1:1 to screen lines (section rules, headers,
        // spacers). Matches `spec::has_row_hitboxes(PaneId::Git)`.
        false
    }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        let styles = super::package::RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         crate::tui::pane::default_pane_chrome(),
        };
        super::git::render_git_pane_body(frame, area, self, &styles, ctx);
    }
}

// ── Targets ─────────────────────────────────────────────────────
pub struct TargetsPane;
impl Pane for TargetsPane {
    fn id(&self) -> PaneId { PaneId::Targets }
    fn input_context(&self) -> InputContextKind { InputContextKind::DetailTargets }
    fn has_row_hitboxes(&self) -> bool { true }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

// ── Lints ───────────────────────────────────────────────────────
//
// Phase 8.3: cursor `Viewport` absorbed onto LintsPane.
// Phase 8.8: `content: Option<LintsData>` absorbed (was the
// `lints` slot in `PaneDataStore`'s detail set).
pub struct LintsPane {
    viewport: Viewport,
    content:  Option<super::LintsData>,
}

impl LintsPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::LintsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::LintsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Pane for LintsPane {
    fn id(&self) -> PaneId { PaneId::Lints }
    fn input_context(&self) -> InputContextKind { InputContextKind::Lints }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        super::lints::render_lints_pane_body(frame, area, self, ctx);
    }
}

// ── CiRuns ──────────────────────────────────────────────────────
//
// Phase 8.4: cursor `Viewport` absorbed onto CiPane.
// Phase 8.7: per-project `display_modes` absorbed.
// Phase 8.8: `content: Option<CiData>` absorbed (was the `ci`
// slot in `PaneDataStore`'s detail set).
pub struct CiPane {
    viewport:      Viewport,
    content:       Option<super::CiData>,
    display_modes: HashMap<AbsolutePath, CiRunDisplayMode>,
}

impl CiPane {
    pub fn new() -> Self {
        Self {
            viewport:      Viewport::new(),
            content:       None,
            display_modes: HashMap::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::CiData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::CiData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    /// Test-only: replace `content.runs` on an already-populated
    /// detail set and drop the mode label. Mirrors what a production
    /// rebuild would produce for fixture CI data.
    #[cfg(test)]
    pub fn override_runs_for_test(&mut self, runs: Vec<crate::ci::CiRun>) {
        if let Some(ci) = self.content.as_mut() {
            ci.runs = runs;
            ci.mode_label = None;
        }
    }

    pub fn display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.display_modes.get(path).copied().unwrap_or_default()
    }

    pub fn set_display_mode(&mut self, path: AbsolutePath, mode: CiRunDisplayMode) {
        self.display_modes.insert(path, mode);
    }

    pub fn remove_display_mode(&mut self, path: &Path) { self.display_modes.remove(path); }

    pub fn clear_display_modes(&mut self) { self.display_modes.clear(); }
}

impl Pane for CiPane {
    fn id(&self) -> PaneId { PaneId::CiRuns }
    fn input_context(&self) -> InputContextKind { InputContextKind::CiRuns }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }

    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        super::ci::render_ci_pane_body(frame, area, self, ctx);
    }
}

// ── Output ──────────────────────────────────────────────────────
pub struct OutputPane;
impl Pane for OutputPane {
    fn id(&self) -> PaneId { PaneId::Output }
    fn input_context(&self) -> InputContextKind { InputContextKind::Output }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

// ── Toasts ──────────────────────────────────────────────────────
pub struct ToastsPane;
impl Pane for ToastsPane {
    fn id(&self) -> PaneId { PaneId::Toasts }
    fn input_context(&self) -> InputContextKind { InputContextKind::Toasts }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

// ── Settings ────────────────────────────────────────────────────
pub struct SettingsPane;
impl Pane for SettingsPane {
    fn id(&self) -> PaneId { PaneId::Settings }
    fn input_context(&self) -> InputContextKind { InputContextKind::Overlay }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

// ── Finder ──────────────────────────────────────────────────────
pub struct FinderPane;
impl Pane for FinderPane {
    fn id(&self) -> PaneId { PaneId::Finder }
    fn input_context(&self) -> InputContextKind { InputContextKind::Overlay }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

// ── Keymap ──────────────────────────────────────────────────────
pub struct KeymapPane;
impl Pane for KeymapPane {
    fn id(&self) -> PaneId { PaneId::Keymap }
    fn input_context(&self) -> InputContextKind { InputContextKind::Overlay }
    fn has_row_hitboxes(&self) -> bool { false }
    fn size_spec(&self, _cpu_width: u16) -> PaneSizeSpec { PaneSizeSpec::fill() }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    //! Verify each pane's trait impl matches today's free-function
    //! answers — the Phase 7 trait must produce identical results
    //! to the existing `spec::*` dispatch so Phase 8/9 can swap
    //! callers without behavior change.
    use super::super::spec::behavior;
    use super::super::spec::has_row_hitboxes as spec_has_row_hitboxes;
    use super::super::spec::size_spec as spec_size_spec;
    use super::*;

    fn pane_for(id: PaneId) -> Box<dyn Pane> {
        match id {
            PaneId::ProjectList => Box::new(ProjectListPane),
            PaneId::Package => Box::new(PackagePane::new()),
            PaneId::Lang => Box::new(LangPane::new()),
            PaneId::Cpu => Box::new(CpuPane::new(&CpuConfig::default())),
            PaneId::Git => Box::new(GitPane::new()),
            PaneId::Targets => Box::new(TargetsPane),
            PaneId::Lints => Box::new(LintsPane::new()),
            PaneId::CiRuns => Box::new(CiPane::new()),
            PaneId::Output => Box::new(OutputPane),
            PaneId::Toasts => Box::new(ToastsPane),
            PaneId::Settings => Box::new(SettingsPane),
            PaneId::Finder => Box::new(FinderPane),
            PaneId::Keymap => Box::new(KeymapPane),
        }
    }

    fn all_ids() -> [PaneId; 13] {
        [
            PaneId::ProjectList,
            PaneId::Package,
            PaneId::Lang,
            PaneId::Cpu,
            PaneId::Git,
            PaneId::Targets,
            PaneId::Lints,
            PaneId::CiRuns,
            PaneId::Output,
            PaneId::Toasts,
            PaneId::Settings,
            PaneId::Finder,
            PaneId::Keymap,
        ]
    }

    #[test]
    fn id_matches_construction() {
        for id in all_ids() {
            let pane = pane_for(id);
            assert_eq!(pane.id(), id, "{id:?}");
        }
    }

    #[test]
    fn has_row_hitboxes_matches_spec_function() {
        for id in all_ids() {
            let pane = pane_for(id);
            assert_eq!(pane.has_row_hitboxes(), spec_has_row_hitboxes(id), "{id:?}");
        }
    }

    #[test]
    fn size_spec_matches_spec_function() {
        for id in all_ids() {
            for cpu_width in [4, 12, 32] {
                let pane = pane_for(id);
                assert_eq!(
                    pane.size_spec(cpu_width),
                    spec_size_spec(id, cpu_width),
                    "{id:?} cpu_width={cpu_width}"
                );
            }
        }
    }

    #[test]
    fn input_context_matches_today_dispatch() {
        // Mirrors `app/focus.rs::input_context`'s match arms:
        // PaneBehavior::ProjectList | Overlay → InputContext::ProjectList,
        // PaneBehavior::DetailFields → InputContext::DetailFields, etc.
        // The `Overlay` panes intentionally report their own
        // `InputContextKind::Overlay` here; App's input router treats
        // overlays specially (via ui_modes flags) before falling
        // through to the per-pane kind, which today maps Overlay
        // back to ProjectList. Phase 8 wires that through.
        for id in all_ids() {
            let pane = pane_for(id);
            let kind = pane.input_context();
            let expected = match behavior(id) {
                super::super::spec::PaneBehavior::ProjectList => InputContextKind::ProjectList,
                super::super::spec::PaneBehavior::DetailFields => InputContextKind::DetailFields,
                super::super::spec::PaneBehavior::DetailTargets
                | super::super::spec::PaneBehavior::Cpu => InputContextKind::DetailTargets,
                super::super::spec::PaneBehavior::Lints => InputContextKind::Lints,
                super::super::spec::PaneBehavior::CiRuns => InputContextKind::CiRuns,
                super::super::spec::PaneBehavior::Output => InputContextKind::Output,
                super::super::spec::PaneBehavior::Toasts => InputContextKind::Toasts,
                super::super::spec::PaneBehavior::Overlay => InputContextKind::Overlay,
            };
            assert_eq!(kind, expected, "{id:?}");
        }
    }
}
