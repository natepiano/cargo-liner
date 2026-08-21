//! Cargo-port's app-side half of the keymap overlay.
//!
//! The overlay's rendering, its edit flow, and its TOML writer all
//! live in the framework — [`tui_pane::KeymapPane`] and
//! [`tui_pane::KeymapEditContext`]. What stays here is what only
//! cargo-port knows: which binds vim mode generates rather than the
//! user configuring them, and which bindings vim mode would collide
//! with if it were turned on.
mod controller;

pub(super) use controller::*;
