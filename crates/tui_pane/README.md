# tui_pane

Reusable `ratatui` pane framework embedded by the `cargo-port` binary in this
workspace.

`tui_pane` owns the generic terminal-UI mechanics that should be reusable across
apps:

- configurable keymaps and TOML overlay loading
- framework and app-global shortcut dispatch
- tab traversal and framework focus state
- status bar and status line rendering
- built-in Settings and Keymap panes
- toast storage, focus, actions, rendering, and tracked task rows
- viewport cursor, hover, scroll, visible-row, and overflow state

The embedding app supplies domain data and side effects:

- app pane identifiers and pane bodies
- app-specific shortcut enums and dispatchers
- app-specific settings registry entries and apply callbacks
- app facts used by the status line
- palette/theme values
- domain-to-framework adapters such as tracked toast item keys

Public API is exported from the crate root. Prefer `tui_pane::Keymap`,
`tui_pane::SettingsStore`, `tui_pane::Toasts`, and similar root paths rather
than importing through internal modules.

This crate is not separately published today.
