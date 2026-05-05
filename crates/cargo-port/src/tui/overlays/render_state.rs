//! Per-overlay-pane render state.
//!
//! Each of the three overlay panes (Keymap, Settings, Finder) carries
//! a small viewport-bearing struct here. They were absorbed from
//! `tui/panes/pane_impls.rs` in Phase 17. `Overlays` (in `mod.rs`)
//! owns one of each as fields.
//!
//! Render is a no-op (the popup body lives in `keymap_ui.rs`,
//! `settings.rs`, `finder.rs`); these structs only exist so the three
//! viewports have a typed home that impls `Pane` and `Hittable`. The
//! impls live in `pane_impls.rs`.

use crate::tui::pane::Viewport;

pub struct KeymapPane {
    pub(super) viewport: Viewport,
}

impl KeymapPane {
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Default for KeymapPane {
    fn default() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }
}

pub struct SettingsPane {
    pub(super) viewport:     Viewport,
    /// Per-rendered-line mapping from line index (relative to the
    /// settings popup's content area) to the underlying setting row
    /// index. Spacer / header lines are `None`. Recorded by
    /// `settings::render_settings_popup`. Ephemeral per-frame layout
    /// state; render writes it, hit-test reads it (sibling of
    /// `viewport`, same pattern as `CpuPane.row_rects`).
    pub(super) line_targets: Vec<Option<usize>>,
}

impl SettingsPane {
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub fn set_line_targets(&mut self, targets: Vec<Option<usize>>) { self.line_targets = targets; }
}

impl Default for SettingsPane {
    fn default() -> Self {
        Self {
            viewport:     Viewport::new(),
            line_targets: Vec::new(),
        }
    }
}

pub struct FinderPane {
    pub(super) viewport: Viewport,
}

impl FinderPane {
    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Default for FinderPane {
    fn default() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }
}
