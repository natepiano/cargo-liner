//! Reusable ratatui pane framework.
//!
//! `tui_pane` provides the keymap, status bar, and built-in panes
//! (Keymap, Settings, Toasts) that an application embeds via the
//! `AppContext` trait. Later phases add the public API one trait
//! family at a time.

mod bar;
mod keymap;

// Crate-root re-exports. The `action_enum!` and `bindings!` macros
// reference these types via `$crate::*` paths, which requires them at
// the crate root. `tests/macro_use.rs` locks the path stability.
pub use bar::BarRegion;
pub use bar::BarSlot;
pub use bar::InputMode;
pub use bar::ShortcutState;
pub use keymap::Action;
pub use keymap::Bindings;
pub use keymap::GlobalAction;
pub use keymap::KeyBind;
pub use keymap::KeyInput;
pub use keymap::KeyParseError;
pub use keymap::KeymapError;
pub use keymap::ScopeMap;
pub use keymap::VimMode;
