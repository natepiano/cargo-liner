//! Reusable ratatui pane framework.
//!
//! `tui_pane` provides the keymap, status bar, and built-in panes
//! (Keymap, Settings, Toasts) that an application embeds via the
//! `AppContext` trait. Phase 1 ships the crate as an empty shell so
//! the workspace conversion can land in isolation; later phases add
//! the public API one trait family at a time.
