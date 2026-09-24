# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `AppIdentity` names an app to the framework: the directory its files live in under the OS config root, its binary name, and its `[appearance]` defaults, with `config_path`, `keymap_path` and `themes_dir` resolved from them. `LoadedConfig<C>` reads the app's own serde config struct from `config.toml`, runs on defaults and keeps the text when the file does not parse, writes back a file that does not spell out every setting, and `save`s edits, keeping the struct's key order. `AppearanceConfig<I>` is the `[appearance]` table, taking the value of any key the file leaves out from `I`. `install_theme` builds the theme registry from the app's built-ins plus `themes/*.toml`, resolves the table against it and installs it.
- The settings overlay an app shows under `s`: `SettingsRows<S>` composes its rows, with builders for the rows the framework owns (the `[appearance]` steppers, `initial rows`, the Files paths and the Notices) between the app's own sections, steppers and read-only values, and answers which `SettingTarget` the selected row edits. `step_framework_setting` steps a `FrameworkSetting` and `apply_settings` re-resolves the theme and saves, through the `AppConfig` trait the app's config struct implements; `stepped` and `SettingStep` are the wrapping step every stepper uses. `draw_settings` fits the popup to its widest row. `SettingsNavigation<A>` is the navigation scope that moves through it, for an app implementing `SettingsHost`. `InitialRows` is `tiles.initial_rows`, serialized as the bare integer.
- `BarPalette::themed` styles the status line from the installed theme, and `StatusLineNote::flag` is a note that is a label alone. `draw_keymap_overlay` and `draw_global_shortcuts_overlay` draw the keymap overlay and the `?` popup over the whole frame.
- `run_terminal` runs an app from taking the terminal to handing it back: raw mode, the alternate screen and mouse reporting, and in iTerm2 the app's profile, put back on exit and on a panic; frames paced against a fixed deadline, a burst of resize events drained before the next draw, the whole screen written over every two seconds; and a relaunch of the binary with its arguments on a restart. The app implements `TerminalApp` -- its keymap, frame and click, and hooks the loop and the key ladder call at fixed points -- folds its per-pass work in through a `PollWork` that answers with a `Repaint`, and times the loop through a `FrameProbe` (`NoProbe` times nothing), whose provided `timed`, `note` and `trace` time a `FramePhase` of the app's own frame, write a line of their own to the probe's log, and ask for every frame in full. `VisualDeadline` is the earliest moment the app needs a frame with no event behind it. `dispatch_key` is the key ladder: the app's modal, an open framework overlay, the attract hook, the framework globals, then the app's own globals.
- `TileGrid<Id>`, the animated grid of bordered cells `cargo-tile` draws: a summary cell and one cell per group the app names by its own `Id`, divided by what each cell asks for through `TileDemands` and laid out as `TilePlacement`s a frame at a time. `TileAction` and `TileGrid::apply` carry out adding and removing cells and moving focus; `TABLE_CELL` and `MIN_INITIAL_ROWS` fix the summary's cell number and the floor on the first column's rows. `draw_tile_grid` draws it, with the app's `TileCells` supplying each cell's rows and contents and the grid adding the shared borders, a rows readout along each cell's foot and the number an empty cell carries; `TileGridContents` hides the contents while leaving the frames, and `draw_tile_cell` draws one cell's interior alone. `TILE_ROWS_CONTENT_LABEL` is the readout's leading label.
- With the `backdrop` feature, what an app's frame calls around the attract screen: `Updates` says whether the display takes new work in or is held still, `fade_to_background` carries a region's text toward the colour each cell is painted on, and `attract_ground` is the colour it goes toward where a cell is painted on nothing.
- KDE Plasma: the backdrop draws the windows really standing under the terminal, over the real wallpaper, assembled from `KWin`'s stacking order and one `ScreenShot2` capture per window. Without the desktop entry `KWin` requires, the wallpaper reconstruction stands as before.
- `ResolvingPixels`, a third attract animation: a travelling wave of coarseness over the backdrop, clumping cells into blocks that share a colour. `PixelResolve` and `PixelFill` say how a block hands its cells back and what a cell is drawn with.
- `DriftingText`, a second attract animation: each line a ring of characters drifting the way a `BandDirection` names, in the backdrop's colours. `TextDrift`, `spread_wider` and `spread_narrower` steer how far the lanes' speeds run apart.
- `BandSettings`, `TextSettings` and `PixelSettings` expose each animation's steerable parameters, restored through its steering transitions or generated from a seed.
- The keymap overlay lays its sections out in as many columns as the popup allows, so a tall keymap is read rather than scrolled; clicks read both axes.
- `BackdropMonitor::identify` settles which of the emulator's windows this app is drawn in, by having the terminal briefly wear a title only this process knows.
- `BandFraying` and `cycle_fraying` step through the four fraying settings one edge at a time; `tail_faster` and `tail_slower` set how fast an edge frays.
- Steer a `TravelingBand` while it runs: `set_direction`, `widen`, `narrow`, `speed_up`, `slow_down`, each clamping in the band so a held key can be handed straight through.
- `kernel_parent` reads a process's parent where sysinfo leaves it unset -- on macOS, any process another user owns, `/usr/bin/login` among them.
- The `backdrop` feature: `BackdropMonitor::current()` answers what is behind a rectangle of the grid, one colour per cell, capturing on a worker thread while the window's position is re-read every frame so the colours follow a drag. Off by default, and `None` off macOS or where Screen Recording is refused.
- `color_distance` answers how far apart two colours look, weighted the redmean way rather than counting channels equally.
- `Pane::cycle_step`, the app-pane counterpart of `Toasts::try_consume_cycle_step`: a pane with a ring of its own gets first refusal on each Tab step.
- `PaneBorders` decides whether neighbouring panes share the cells their borders fall on -- `Shared` is what 0.7.0 did, `Separate` gives each pane a closed box.
- `blend_color` and `pane_background` draw text part of the way between two colours, which a cell needs because it holds three opaque bytes and nowhere to put a fourth.

### Changed
- `DriftingText`'s lanes merge into one another rather than meeting at an edge: `merged` pulls the smoothstep back toward a straight ramp by `TEXT_LANE_BODY_PERCENT`.
- `DEFAULT_TEXT_SPEED` drops from 12 cells a second to 9.
- The drifting field's lanes are dealt uneven thicknesses within `TEXT_LANE_SPREAD_PERCENT` of the nominal; bands all one size read as a ruled grid.
- `DriftingText` and `TravelingBand` paint the cell behind each character as well as the character, `TEXT_BEHIND_FADE` and `BAND_BEHIND_FADE` apart, so the picture lines up with itself.
- `DriftingText`'s lane speeds travel along the field instead of standing where they were dealt, read as a ring so the pattern has no edge to cross.
- `DriftingText` deals each line a ring of numbers rather than characters and `TextFill` says how they are drawn, so sub-cell travel is no longer discarded; `set_direction` carries the field over instead of re-dealing it.
- `TravelingBand` staggers where each offset begins its lap, so a shallow strip no longer leaves the same run of grid empty on every offset.
- The keymap overlay sizes its description column to the widest description and widens the popup to fit, rather than padding to a fixed 25 columns.
- A band standing as deep as the grid lights every line: both edges are read as runs on the ring it travels.
- A band turned between the sideways and the up-and-down axis keeps the depth a ruler would measure rather than the count of cells, which a cell twice as tall as it is wide doubled.
- A band is never deeper than the grid it crosses, rather than stopping at a fixed two hundred.
- Both of a band's edges enter and leave a line in proportion to how far across it they are; whole-line steps read as hesitation.
- Poll frames every eight milliseconds rather than sixteen, which is under the refresh interval of the displays these apps run on.
- Work the band's travel out in microseconds rather than milliseconds, which rounded a twenty-millisecond frame down to nineteen.
- Ask the window server where the terminal stands from a thread of its own, not the render loop, where a capture in flight ahead of it cost tens of milliseconds.
- Every cell a `TravelingBand` covers is drawn in exactly the backdrop's colour there, front to back, since a terminal cell carries no alpha to spend.
- A `TravelingBand` wraps rather than clearing the far edge and starting over, so the grid is never empty between passes.
- A band leaving fades toward whatever each cell is already painted on; `ground` stands in only where a cell is painted on nothing.
- The bar's `Tab pane` row is drawn only where the step has somewhere to land.
- **Breaking:** `render_panes` and `GridLines::render` take a `PaneBorders`, and `render_panes` asks `PaneBorders::pane_area` rather than calling `share_borders` itself.
- Restore the focused-border colour 0.7.0 removed -- `PaneChromeTheme::active_border`, `PaneChrome::active_border`, `active_border_color()` and `border_style(focused)` -- since the reasoning for dropping it holds only under `PaneBorders::Shared`.
- `PaneChromeTheme::active_border` is `Option<StyleSpec>` and defaults to the focused title's colour, so a theme written while the key was ignored still loads.
- Narrow `SECTION_HEADER_INDENT` to one space from two and `SECTION_ITEM_INDENT` to one from four, so a header and its items start in the same column.

### Fixed
- KDE backdrop: a window underneath is drawn at its `frameGeometry`, with `include-shadow: false` so the capture covers exactly that. Placed by `bufferGeometry` -- which `KWin` reports as the client area inside the title bar, or the shadowed frame on a window that decorates itself -- every window stood tens of pixels right of and below where it really was, while the wallpaper lined up exactly.
- KDE backdrop: the window stack is read on every capture and a picture kept only while its arrangement stands, so a window that moved or closed no longer lingers for the whole `COMPOSITE_HOLD`. The read costs five milliseconds against a capture per window.
- `Desktop::placement` reads the terminal's current cell count rather than the capture's, which after a resize gave a negative padding and placed a halved window 86 columns off the display. The cell *size* still comes from the capture.
- KDE backdrop: a window the user has dragged keeps working. `KWin` reports its geometry with a fraction on the end, which parsed as no whole coordinate and dropped the window from the stack, silently giving the composite back to the wallpaper.
- The backdrop captures with CoreGraphics `CGWindowListCreateImage` rather than ScreenCaptureKit, whose screenshot and shareable-content calls block one another across processes.
- `CaptureFailure::CaptureWorkerReplaced` distinguishes a worker abandoned after a second consecutive missed deadline from a single missed attempt.
- The backdrop monitor replaces a capture worker only after a second consecutive missed deadline, and a returned result restores its replacement allowance.
- `window_titles`, `window_titled` and `window_at` ask CoreGraphics rather than `SCShareableContent` -- seventy milliseconds against a few hundred microseconds, paid on the thread that draws.
- The marker title `identify` sets stays on until a pass finds it or the passes run out; taken off within its own pass, no window was ever caught wearing it.
- `named_emulator_windows` takes the `.app` extension off `TERM_PROGRAM` before folding it, so `iTerm.app` no longer folds to `itermapp` and matches nothing.
- `display_under` settles for the display nearest the window's centre rather than the first, which is the primary -- the display a window standing elsewhere is least likely to be on.
- The backdrop divides the terminal's reported text area by the display's backing scale factor, since `TIOCGWINSZ` answers in pixels where everything it is measured against is in points.
- The grid a capture is reduced to rounds up to cover the whole display, rather than leaving the bottom row of a full-height window unpainted.
- `BackdropMonitor::identify` asks the terminal outright where its window stands, with the xterm position report, before falling back to a marker title the reader may have pinned over.
- `BackdropMonitor::identify` is retried on a pace until it settles; one unpaced burst covered milliseconds where the marker needed hundreds to cross the pty.
- The backdrop finds the emulator's windows by `TERM_PROGRAM` where the parent chain does not reach it -- an emulator hosting its sessions in a server process is in nobody's parent chain.
- `BackdropMonitor` chooses and places its display by `CGDisplayBounds`, the coordinate space window frames are read in; `SCDisplay::frame` is not, so a window off the primary matched none.
- `BackdropMonitor` finds its own window by the number `identify` pinned to it, and the capture leaves out every window standing in front of this one.

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
