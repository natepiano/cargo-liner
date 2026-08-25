# Changelog

All notable changes to this crate will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this crate adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Add `DriftingText`, a second attract-mode animation beside `TravelingBand`: every cell of the area drawn, each line of characters a ring drifting one of the four ways a `BandDirection` names, in the colours the `Backdrop` has for the cells on screen rather than for the characters crossing them. `TextDrift` says whether the lines travel as one or at speeds of their own, and `spread_wider` and `spread_narrower` set how far apart those speeds run -- stretching the range each line's speed is read off rather than re-drawing where it sits in it, so the same lines walk apart and back together instead of being dealt a fresh hand each press. Alternate lines are dealt from the slow and the fast third of that range, since neighbouring lines are the only ones the eye can compare and two drawn independently land on much the same speed as often as not. Sending the lines apart also opens the spread to at least its default, so the key never appears to do nothing.
- Add `BackdropMonitor::identify`, which settles which of the emulator's windows the app is drawn in by having the terminal briefly wear a title only this process knows. Every window of an emulator answers to the same application and two opened side by side are commonly the same size, so neither ownership nor size can tell them apart -- and which one the size heuristic picked changed from one capture to the next.
- Either of a `TravelingBand`'s edges can fray: `BandFraying` names the four settings and `cycle_fraying` steps through them, each step changing exactly one edge. The leading edge's excursion is held under the trailing edge's floor by a compile-time assertion, so the two can never meet and the strip keeps a core at every offset. A band starts standing across the whole grid with both edges fraying, and how much grid is left empty behind it changes while it travels, since each offset across it ends where its own two edges say. `tail_faster` and `tail_slower` change how fast they fray, one number governing both the walk to a fresh depth and the stand at it.
- Steer a `TravelingBand` while it runs: `set_direction` sends it any of the four ways a `BandDirection` names, `widen` and `narrow` change how deep it stands, and `speed_up` and `slow_down` change how fast it travels. Each clamps in the band rather than at the call site, so an app can hand a held key straight through. Travelling up or down the strip is a row crossing the area rather than a column, which is the same animation the other way up.
- Add `kernel_parent`, which reads a process's parent from the kernel where sysinfo leaves it unset -- on macOS, any process another user owns. `/usr/bin/login` is one of them and stands between a terminal emulator and the shell in it, so a walk up the chain without this never reaches the emulator at all.
- Add the `backdrop` feature: `BackdropMonitor::current()` answers what is behind a rectangle of the terminal grid, one colour per character cell, with every window the terminal owns left out. It keeps two clocks -- a capture of the whole display on a worker thread, and the window's own position re-read every frame -- so the colours follow a window that is dragged instead of trailing a capture behind it. A window that is moving asks for no captures at all -- where it stands is not something a capture holds -- so a drag does not compete with the window server for the frames it needs. `TravelingBand` draws a strip of characters across the grid in those colours. Off by default -- it pulls in `ScreenCaptureKit` -- and answers `None` off macOS or where Screen Recording is refused.
- Add `color_distance`, which answers how far apart two colours look on a 0 to 764 scale. Weighted the way the redmean approximation weights the channels rather than counting them equally, so it answers the question an app actually asks -- whether two colours would be read as the same one. Named and indexed colours are resolved the same way `blend_color` resolves them, and a colour with no channels to read gives `None`.
- Add `Pane::cycle_step`, the app-pane counterpart of `Toasts::try_consume_cycle_step`: a pane that draws a grid or ring of its own gets first refusal on each Tab step and returns whether it took it. `CycleDirection` is public for it.
- Add `PaneBorders`, which decides whether neighbouring panes share the cells their borders fall on. `PaneBorders::Shared` is what 0.7.0 did unconditionally: one lattice, every line in the inactive shade. `PaneBorders::Separate` gives each pane its own closed box, with its neighbour's line beside rather than under it. The two apps built on this framework want opposite answers, and the answer was never really the framework's to make.
- Add `blend_color` and `pane_background`, which together let an app draw text part of the way between two colours. `blend_color(color, toward, alpha)` reads both ends as red, green and blue and writes the mixture, which a terminal cell needs because it holds three opaque bytes and has nowhere to put a fourth. Named colours are read against the xterm palette for ANSI 0-15, and `Color::Indexed` against the colour cube and grayscale ramp beyond it, so a theme written in `Color::Cyan` blends the same way as one written in `Color::Rgb`; `Color::Reset` names no colour and is handed back untouched. `pane_background(focused)` answers what `pane_fill` lays down under a pane's contents -- the tint when it is switched on and the theme's own `text.bg_focus` when it is off -- which is what a caller carrying text toward the ground it stands on has to name.

### Changed
- The keymap overlay sizes its description column to the widest description it has to show and widens the popup to fit, rather than padding every description to a fixed 25 columns inside a fixed 52-column popup. A description longer than that pushed its own key right and nothing else's, so the keys stopped lining up and the longest ones ran into the description beside them.
- A band standing as deep as the grid has lines now lights every line. Both edges are read as runs on the ring the band travels, so the line the leading edge is part way across is the same one the tail is part way off, and owning only the leading share of it left one column short of full at any width.
- A band turned between the sideways and the up-and-down axis now keeps the depth a ruler would measure rather than the count of cells. A character cell is about twice as tall as it is wide, so carrying the count across a turn made the band twice as deep going vertical.
- A band is now never deeper than the grid it crosses -- the columns travelling sideways, the rows travelling up or down -- rather than stopping at a fixed two hundred. At the grid's own extent the tail meets the leading edge and everything is lit, and past that there is nothing further to show.
- The band's trailing edge now leaves the line it is on in proportion to how much of it the strip still stands on, as the leading edge already entered one. A strip on a character grid can only stand on whole cells, so with one edge shaded and the other dropping a line at a time, half its travel was still stepping.
- Poll frames every eight milliseconds rather than every sixteen, which is under the refresh interval of the displays these apps run on. A loop slower than the display holds some frames for one refresh and some for two, which at animation speeds reads as hesitation.
- The band's leading edge now lights the line it is entering in proportion to how far in it has come, rather than switching each line on whole. The edge crosses a little over half a cell per frame, so a line that could only be lit or unlit held still for a frame or two and then jumped, which read as stepping rather than travel. The rest of the strip still wears the desktop's colour exactly.
- Work the band's travel out in microseconds rather than milliseconds. A frame is a little under twenty milliseconds and rounding it down to nineteen lost a twentieth of the distance, and lost a different fraction whenever a frame arrived early.
- Ask the window server where the terminal window stands from a thread of its own rather than from the render loop. A process has one connection to it and it is served in order, so the question -- a few hundred microseconds on its own -- took tens of milliseconds whenever a capture was in flight ahead of it, dropping frames once a second. The loop now reads the newest answer without waiting for it.
- Every cell a `TravelingBand` covers is drawn in exactly the colour the backdrop has there, front to back: no lift at the leading edge and no ramp along the tail. A terminal cell carries no alpha, so anything mixed into that colour is spent on the one thing the band is for.
- A `TravelingBand` wraps rather than clearing the far edge and starting over, so its tail is still leaving one side while its leading edge is back at the other and the grid is never empty between passes.
- A band leaving now fades toward whatever each cell is already painted on, rather than toward one colour named for the whole grid. The `ground` argument stands in only where a cell is painted on nothing.
- Draw the bar's `Tab pane` row only when the step has somewhere to land -- more than one live tab stop, or a focused pane with a ring of its own. A single-pane app used to advertise a key that did nothing.
- **Breaking:** `render_panes` and `GridLines::render` take a `PaneBorders`. `render_panes` also stops calling `share_borders` itself and asks `PaneBorders::pane_area` instead, so the layout choice is made in one place rather than assumed.
- Restore the focused-border colour, which 0.7.0 removed. `PaneChromeTheme::active_border`, `PaneChrome::active_border`, and `active_border_color()` are back, along with `PaneChrome::border_style(focused)`. The reasoning for dropping it holds only where a border cell has two owners, and that is now `PaneBorders::Shared`'s answer rather than everybody's: under `Separate` a cell belongs to exactly one pane, so lighting it takes nothing from anybody and the focused pane reads as one lit box.
- `PaneChromeTheme::active_border` is `Option<StyleSpec>` and defaults when absent, so a theme file written while the key was ignored still loads. `None` takes the focused title's colour, which marks focus rather than losing it; an app whose panes share their borders never reads the field at all.
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
