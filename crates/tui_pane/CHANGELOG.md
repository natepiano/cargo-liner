# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Narrow `SECTION_HEADER_INDENT` to one space from two, and `SECTION_ITEM_INDENT` to one space from four. A section header and the items under it now start in the same column, which gives an overlay or a table three columns back at the left margin and leaves the nesting to read from colour rather than position.

## [0.7.0] - 2026-08-21

### Added
- Add the shared border grid: `GridLines`, `PaneFrame`, `PaneFrameLabel`, `PaneFrameChrome`, `share_borders`, `draw_clipped`, `frame_inner`, `rule_title_label`, and `overflow_affordance_label`. `GridLines` collects every pane's four edges into a per-cell side bitset and derives the box-drawing glyph from it, so a boundary two panes share is drawn once and the crossing where four meet resolves without any caller naming a junction character. A pane body now returns `PaneFrameChrome` rather than drawing its own `Block`, and a rule that crossed a pane border moves into `chrome.rules` with its title becoming a label.
- Add `KeymapEditContext` and the keymap-editor controller, so the framework now owns the whole keymap overlay rather than only its rendering and state machine: selection movement, Enter-to-edit, capture validation against every binding in force, conflict detection across scopes, and the `keymap.toml` write and reload. An embedding app supplies where the file lives, the TOML header, inline-error get/set, how to rebuild its keymap, and its globals type. Previously each app had to write this itself.
- Make `Keymap::scope_toml_name_for` public: the pane-id to TOML-scope-name mapping the keymap already holds, which apps were re-deriving by hand.

### Changed
- **Breaking:** theme *content* now belongs to the embedding app, not to this crate. The four compiled-in palettes (`default_dark`, `default_light`, `high_contrast_dark`, `high_contrast_light`), the `BUILTIN_*_NAME` id constants, and the `themes/*.toml` templates are gone; an app defines its own variants and passes them in. `ThemeRegistry::new_with_builtins` takes a `Vec<ThemeVariant>` and `ThemeRegistry::from_dir_with_builtins` takes one after `dir`. Two apps built on the framework can now be retuned independently. `fallback_theme(appearance)` replaces the old built-ins wherever the crate itself needs a palette: an empty registry, or a `ThemeState` installed before startup ran.
- **Breaking:** a `resolve_active` miss now falls back to the first registered variant of the resolved appearance — the app's own default — rather than to a framework palette. `fallback_theme` stands in only when the registry holds nothing for that appearance.
- **Breaking:** remove the focused-border colour. `PaneChromeTheme::active_border`, `PaneChrome::active_border`, and `active_border_color()` are gone, and `GridLines` draws every line in the inactive shade. A border is a cell two panes share, so lighting it for the focused one took the boundary away from its neighbour and left the focused box's corners fighting the junctions they really sit on -- a `T` or a crossing could be closed into a corner or left leaking a lit arm, but not both. Focus is now carried by the background tint alone. Themes may keep an `active_border` key; it is ignored.
- Every pane now paints its own background, unfocused ones included, rather than only the focused one. A cell with no background of its own is the terminal's *default* background, and a transparent terminal window composites that cell differently from a painted one, so leaving unfocused panes bare made focus read as a difference in opacity: under iTerm2 with "Only the default background color uses transparency" ticked, the focused pane went solid while its neighbours showed the desktop. Painting both puts them on the same footing, and the window's own transparency then applies to the grid evenly, with focus carried by how far each pane's tint is pushed. Untick that iTerm2 option to see it; leaving it ticked makes the whole grid opaque instead. `focused_pane_tint_enabled()` still switches the tint off entirely, which restores unpainted panes.
- **Breaking:** remove `PaneChrome::with_inactive_border`, which had no remaining consumer once cargo-tile stopped forcing its grid to the focused shade.
- **Breaking:** remove `PaneRule`, `render_rules`, and `render_horizontal_rule`. Every rule that crossed a pane border moved into `PaneFrameChrome::rules`, which the shared border grid draws, and the three lost their last consumer in that move.
- Move development into the `natepiano/cargo-liner` workspace, where `tui_pane` now lives at `crates/tui_pane` as a peer of the tools built on it rather than as a subdirectory of cargo-port. The published crate is unchanged.

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
