# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0] - 2026-08-19

### Added
- Add `StatusLineNote` and `status_line_note_spans`: right-side status-line segments that carry no key binding, render before the global shortcut slots, and stay visible while the focused pane is in `Mode::TextInput`.
- Add `AltModifierLabel` and `KeyBind::platform_label`, so an Alt binding displays as `Option-K` on macOS and `Alt-K` elsewhere.
- Add `CoreCluster` (macOS), reporting whether a core belongs to the Apple Silicon performance or efficiency cluster.

### Changed
- **Breaking:** `StatusLine::new` takes a `notes: &[StatusLineNote]` argument before `globals`, and `StatusLine` gains the matching public field.
- **Breaking (macOS):** `CpuCoreUsage` gains a `cluster: Option<CoreCluster>` field, so struct-literal construction must supply it.

## [0.5.0] - 2026-07-30

### Added
- Add `TrackedItemActivity` to `TrackedItem`/`TrackedItemView` so a caller can report a tracked item as stalled and have its toast spinner render in the palette's error color, plus `Toasts::refresh_tracked_item_activity` to push activity changes onto items a toast already holds.

## [0.4.3] - 2026-07-27

### Changed
- Version bump to 0.4.3 to maintain workspace version synchronization.

## [0.4.2] - 2026-07-27

### Fixed
- Gate the `bounded_percent_u8` and `GpuUsage` re-exports in the CPU diagnostics module to the platforms whose readers use them, clearing the remaining unused-import warnings in a Windows build.

## [0.4.1] - 2026-07-27

### Fixed
- Gate the CPU/GPU platform imports that only the macOS and Linux readers use, so a Windows build compiles without unused-import warnings.

## [0.4.0] - 2026-07-27

### Changed
- Version bump to 0.4.0 to maintain workspace version synchronization.

## [0.3.0] - 2026-07-10

### Changed
- Change `Modifiers` from a public bool-field struct to a `ratatui::style::Modifier` bitflags alias; theme TOML still accepts `bold`, `italic`, `dim`, and `underline`.
- Make `GlobalShortcutsPane` selectable and add stable scope/action identifiers to `GlobalShortcutRow` for remapping integrations.

### Fixed
- Fit the default Global Shortcuts list while retaining navigation and scrolling on smaller terminals.

## [0.2.1] - 2026-06-23

### Changed
- Version bump to 0.2.1 to maintain workspace version synchronization.

## [0.2.0] - 2026-06-23

### Added
- Add `ToastStyle::Success` and fallback success-toast palette/rendering support.

## [0.1.5] - 2026-06-22

### Changed
- Change key bindings to use `From<KeyEvent>` for key-event normalization.
- Change framework render-state APIs to use named state enums for keymap rows, settings focus, toast focus, and pane focus.
- Change toast settings callers to use `toasts_enabled()` and `set_toasts_enabled()`.
- Split status bar rendering, toast management, theme state, settings-store errors, and layout grid code into focused modules.

## [0.1.4] - 2026-06-14

### Changed
- Rename `StatusLineGlobal.state` and `RenderedSlot.state` to `shortcut_state`, and `RenderFocus.state` to `pane_focus_state`.

### Fixed
- Normalize framework keymap parsing so `+` and `=` can resolve the same bound action key
