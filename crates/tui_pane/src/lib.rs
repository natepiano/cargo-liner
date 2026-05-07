//! Reusable ratatui pane framework.
//!
//! `tui_pane` provides the keymap, status bar, and built-in panes
//! (Keymap, Settings, Toasts) that an application embeds via the
//! `AppContext` trait. Later phases add the public API one trait
//! family at a time.

mod app_context;
mod bar;
mod framework;
mod keymap;
mod pane_id;

// Crate-root re-exports. The `action_enum!` and `bindings!` macros
// reference these types via `$crate::*` paths, which requires them at
// the crate root. `tests/macro_use.rs` locks the path stability.
pub use app_context::AppContext;
pub use bar::BarRegion;
pub use bar::BarSlot;
pub use bar::InputMode;
pub use bar::ShortcutState;
pub use framework::Framework;
pub use keymap::Action;
pub use keymap::Bindings;
pub use keymap::GlobalAction;
pub use keymap::KeyBind;
pub use keymap::KeyInput;
pub use keymap::KeyParseError;
pub use keymap::KeymapError;
pub use keymap::ScopeMap;
pub use keymap::VimMode;
pub use pane_id::FocusedPane;
pub use pane_id::FrameworkPaneId;
