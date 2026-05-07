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
mod pane;
mod pane_id;

// Crate-root re-exports. The `action_enum!` and `bindings!` macros
// reference these types via `$crate::*` paths, which requires them at
// the crate root. `tests/macro_use.rs` locks the path stability.
pub use app_context::AppContext;
pub use bar::BarRegion;
pub use bar::BarSlot;
pub use bar::ShortcutState;
pub use bar::Visibility;
pub use framework::Framework;
pub use keymap::Action;
pub use keymap::Bindings;
pub use keymap::ErasedScope;
pub use keymap::GlobalAction;
pub use keymap::Globals;
pub use keymap::KeyBind;
pub use keymap::KeyInput;
pub use keymap::KeyOutcome;
pub use keymap::KeyParseError;
pub use keymap::Keymap;
pub use keymap::KeymapBuilder;
pub use keymap::KeymapError;
pub use keymap::Navigation;
pub use keymap::RenderedSlot;
pub use keymap::ScopeMap;
pub use keymap::Shortcuts;
pub use keymap::VimMode;
pub use pane::Mode;
pub use pane::Pane;
pub use pane_id::FocusedPane;
pub use pane_id::FrameworkPaneId;
