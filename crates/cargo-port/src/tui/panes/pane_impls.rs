//! Per-pane unit structs and their `Pane` impls.
//!
//! Phase 8 lands per-pane state (cursor `Viewport`, content slot,
//! pane-specific extras) on the relevant struct and dispatches
//! `render` through the `Pane` trait. Phase 9 brings the remaining
//! seven panes (`ProjectList`, `Targets`, `Output`, `Toasts`,
//! `Settings`, `Finder`, `Keymap`) into trait dispatch and
//! reintroduces `handle_input` / `is_navigable` on the trait.
//! Behavior + size mappings continue to live as `PaneId`-pure
//! free functions in `panes/spec.rs`.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::dispatch::Pane;
use super::dispatch::PaneRenderCtx;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::cpu::CpuPoller;
use crate::tui::cpu::CpuSnapshot;
use crate::tui::pane::Viewport;

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
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
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
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        let styles = super::package::RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         crate::tui::pane::default_pane_chrome(),
        };
        super::git::render_git_pane_body(frame, area, self, &styles, ctx);
    }
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
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: PaneRenderCtx<'_, '_>) {
        super::ci::render_ci_pane_body(frame, area, self, ctx);
    }
}
