//! Cargo-port app-side keymap-overlay orchestration: capture flow
//! command routing + TOML save.
//!
//! Rendering and row-building live in the framework's
//! [`tui_pane::KeymapPane::render_overlay`]; this module retains only
//! the cargo-port-specific orchestration: dispatching overlay
//! actions, navigation keys inside the popup, capture-command
//! routing, conflict detection against currently-bound rows, and the
//! TOML save / reload path.
mod controller;

pub(super) use controller::*;
