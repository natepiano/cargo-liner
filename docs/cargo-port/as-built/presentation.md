# Pane layout, selection, and overlays

cargo-port allocates panes from their measured needs and draws them through `tui_pane`.
It preserves useful names, whole labels, and explicit cuts at narrow widths.

Paths below are relative to `crates/cargo-port/src/`.

## Allocation and focus

`tui/panes/layout.rs` defines `PaneVisibility`, `MiddlePaneNeeds`, `resolve_layout`, and `top_pane_widths`.
`ProjectColumnLayout` supplies preferred, minimum, and already-shown Projects widths.
Languages and Targets can take unused Projects width.
They do not take cells from a name already visible on the screen.
Targets supplies preferred width from its names, Source, and Kind.

Package and Git remain side by side when both fit.
When only one fits, the existing top-pane rule hides Package and keeps Git.
A hidden pane has no mouse areas and leaves the Tab order.
Focus moves once to a drawn pane.
Restoring width does not move focus again.

`tui/render/mod.rs` supplies selected content and row needs to layout.
`TargetsData::inner_rows` limits target height to the header and real entries.
Detail panes receive the remaining rows in their column before paging.
The CPU pane places its page count in the bottom border.
Languages totals count parent rows without adding their child subtotals again.

## Color and selection

The application reads `ColorMode` at startup and keeps it in `Framework::color_mode`.
Drawing functions receive that value instead of reading `NO_COLOR`.
`tui/theme_roles.rs` owns the final `paint_selection` pass.
Selected rows receive one fill and one text color across the inner row.
Text contrast has a 4.5:1 floor against the real focused and unfocused backgrounds.
An emoji is a colored picture and does not use that text-contrast bound.

Under NoColor, active rows use reverse and remembered rows use bold.
Focused pane borders use the thick form.
`prepare_terminal_buffer` calls `ColorMode::clear_colors` after drawing and keeps its VS16 handling.
Color removal preserves text modifiers.
CPU bars therefore retain block glyphs without relying on cell background colors.

`text.dim_value` supplies quiet values such as No CI, unpublished branch, and empty lint values.
Its fallback is secondary text.
Built-in theme constructors and shipped theme files must agree.
Tests restore temporary themes through the `ThemeOverride` Drop fixture.

## Finder and sccache

`tui/finder/dispatch.rs` cuts the widest finder column first.
Dir values use `render::cut_path_from_left` and a leading `…`.
Selected Type cells use the selection foreground.
Every shortened value has a visible cut mark.

`tui/sccache/render.rs` measures the widest popup row and gives it equal side padding.
Its selection reaches the right border and its scroll readout names the actual position.
Reason rows use plain explanatory text.
The popup chooses `PopupBorder::Thick` in both color modes.
Other popups keep `PopupFrame`'s default border behavior.

Settings paths wrap at slash boundaries inside the popup margin.
Selected Settings rows use one readable foreground.
Framework overlay wheel handling owns settings, keymap, and shortcut scrolling.
The application applies scroll inversion before calling the framework helper.

## Invariants and calibration

1. Keep labels whole except the documented cut Projects header with Lint enabled.
2. Mark shortened text with `…`.
3. Preserve selection styling across the whole inner row.
4. Exclude hidden panes from navigation and hit testing.
5. Measure pagination from drawn rows.
6. Preserve terminal modifiers when clearing colors.

Cut values remove a trailing space or middle-dot separator before the mark.
Remote URLs and Finder paths intentionally cut from the left.
A shared pane or toast change can affect cargo-handler and cargo-tile frame tests.
See [framework presentation](../../tui_pane/as-built/pane-presentation.md) for shared contracts.
