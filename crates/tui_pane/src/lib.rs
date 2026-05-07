//! Reusable ratatui pane framework.
//!
//! `tui_pane` provides the keymap, status bar, and built-in panes
//! (Keymap, Settings, Toasts) that an application embeds via the
//! `AppContext` trait. Later phases add the public API one trait
//! family at a time.

pub mod keymap;
