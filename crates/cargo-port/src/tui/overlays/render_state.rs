//! Finder overlay render state.
//!
//! Finder is app-owned, so its viewport remains in `Overlays`.
//! Settings and Keymap are framework-owned and carry their viewport
//! state on `tui_pane::SettingsPane` / `tui_pane::KeymapPane`.
//!
//! Render is a no-op (the popup body lives in `finder.rs`); this
//! struct exists so the Finder viewport has a typed home that impls
//! `Pane` and `Hittable`. The impls live in `pane_impls.rs`.

use tui_pane::Viewport;

pub struct FinderPane {
    pub viewport: Viewport,
}

impl FinderPane {}

impl Default for FinderPane {
    fn default() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }
}
