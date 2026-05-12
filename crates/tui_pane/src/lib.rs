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
mod toasts;
mod viewport;

// Crate-root re-exports. The `action_enum!` and `bindings!` macros
// reference these types via `$crate::*` paths, which requires them at
// the crate root. `tests/macro_use.rs` locks the path stability.
pub use app_context::AppContext;
pub use app_context::NoToastAction;
pub use bar::BarPalette;
pub use bar::BarRegion;
pub use bar::BarSlot;
pub use bar::ShortcutState;
pub use bar::StatusBar;
pub use bar::StatusLine;
pub use bar::StatusLineGlobal;
pub use bar::StatusLineGlobalAction;
pub use bar::Visibility;
pub use bar::render as render_status_bar;
pub use bar::render_status_line;
pub use bar::status_line_global_spans;
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
pub use panes::SettingsCommand;
pub use panes::SettingsPane;
pub use panes::SettingsPaneAction;
pub use panes::SettingsRender;
pub use panes::SettingsRenderOptions;
pub use panes::ToastsAction;
pub use settings::AdjustDirection;
pub use settings::LoadedSettings;
pub use settings::MaxVisibleToasts;
pub use settings::ReloadedSettings;
pub use settings::SettingAdjuster;
pub use settings::SettingCodecs;
pub use settings::SettingEntry;
pub use settings::SettingKind;
pub use settings::SettingValue;
pub use settings::SettingsError;
pub use settings::SettingsFileSpec;
pub use settings::SettingsRegistry;
pub use settings::SettingsRow;
pub use settings::SettingsRowKind;
pub use settings::SettingsSection;
pub use settings::SettingsStore;
pub use settings::ToastAnimationSettings;
pub use settings::ToastDuration;
pub use settings::ToastGap;
pub use settings::ToastPlacement;
pub use settings::ToastSettings;
pub use settings::ToastWidth;
pub use toasts::Toast;
pub use toasts::ToastBody;
pub use toasts::ToastCommand;
pub use toasts::ToastHitbox;
pub use toasts::ToastId;
pub use toasts::ToastLifetime;
pub use toasts::ToastPhase;
pub use toasts::ToastRenderResult;
pub use toasts::ToastStyle;
pub use toasts::ToastTaskId;
pub use toasts::ToastTaskStatus;
pub use toasts::ToastView;
pub use toasts::Toasts;
pub use toasts::TrackedItem;
pub use toasts::TrackedItemKey;
pub use toasts::TrackedItemView;
pub use toasts::format_toast_items;
pub use toasts::render_toasts;
pub use toasts::toast_body_width;
pub use viewport::Viewport;
