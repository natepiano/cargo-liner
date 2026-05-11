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
mod panes;
mod settings;

// Crate-root re-exports. The `action_enum!` and `bindings!` macros
// reference these types via `$crate::*` paths, which requires them at
// the crate root. `tests/macro_use.rs` locks the path stability.
pub use app_context::AppContext;
pub use bar::BarPalette;
pub use bar::BarRegion;
pub use bar::BarSlot;
pub use bar::ShortcutState;
pub use bar::StatusBar;
pub use bar::Visibility;
pub use bar::render as render_status_bar;
pub use framework::CycleDirection;
pub use framework::Framework;
pub use framework::ListNavigation;
pub use framework::TabOrder;
pub use framework::TabStop;
pub use keymap::Action;
pub use keymap::Bindings;
pub use keymap::Configuring;
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
pub use keymap::Registering;
pub use keymap::RenderedSlot;
pub use keymap::ScopeMap;
pub use keymap::Shortcuts;
pub use keymap::VimMode;
pub use pane::Mode;
pub use pane::Pane;
pub use pane_id::FocusedPane;
pub use pane_id::FrameworkFocusId;
pub use pane_id::FrameworkOverlayId;
pub use panes::KeymapPane;
pub use panes::KeymapPaneAction;
pub use panes::SettingsPane;
pub use panes::SettingsPaneAction;
pub use panes::Toast;
pub use panes::ToastId;
pub use panes::ToastStyle;
pub use panes::Toasts;
pub use panes::ToastsAction;
pub use settings::SettingEntry;
pub use settings::SettingKind;
pub use settings::SettingsRegistry;
