# Shared pane presentation and overlay input

`tui_pane` owns common color policy, pane borders, title fitting, page marks, and framework overlay scrolling.
Applications provide content, configuration, palettes, and placement rectangles.

Paths below are relative to `crates/tui_pane/src/`.

## Color policy

`color_mode.rs` defines `ColorMode::{Colored, NoColor}`.
`ColorMode::from_environment()` reads the terminal policy at application startup.
The application stores the result in `Framework::color_mode`.
Draw code receives the value explicitly.

```rust
pub fn clear_colors(self, buffer: &mut Buffer);
```

`ColorMode::clear_colors` resets foreground and background under NoColor.
It preserves symbols and text modifiers.
Colored leaves the buffer unchanged.
Applications call this operation after drawing and before terminal output.
Every draw exit must apply the same rule, including a separate status-clock draw.

`pane/chrome.rs`, `pane/frame.rs`, and `pane/state.rs` consume the mode.
Selection helpers and pane chrome do not read `NO_COLOR` themselves.
NoColor can therefore keep bold, reverse, and thick focus borders without color resets removing the modifiers.

The theme role `text.dim_value` supplies quiet values through `dim_value_color()`.
Secondary text is its fallback.
Application contrast tests compare text with the actual focused and unfocused backgrounds.
The 4.5:1 text bound does not describe an emoji's colored image.

## Titles, popup borders, and paging

`pane/title.rs` owns `fit_title_sides`.
`pane/frame.rs` applies title overlays through `GridLines`.
A whole title can use both side spaces when needed.
The fitting rule removes both side spaces together.
A shortened title keeps a blank cell before its corner or joint.
`pane/rules.rs` preserves a divider label before shortening it with `…`.

`pane/popup.rs` defines `PopupFrame`, `PopupAreas`, and `PopupBorder`.
`PopupBorder::ColorMode` selects a plain colored border and a thick NoColor border.
`PopupBorder::Thick` selects a thick border in either mode.

```rust
pub fn render_with_border(self, frame: &mut Frame, border: PopupBorder) -> PopupAreas;
```

The method belongs to `PopupFrame`.
Its default rendering methods use `PopupBorder::ColorMode`.
A popup that needs a thick border in both modes selects Thick explicitly.

`ViewportOverflow::for_paged_content` measures drawn content.
Page labels describe the drawn page, not a stale logical offset.
The last page does not show a further-page arrow.
Consumers must measure the same wrapped or stacked rows they render.

## Status-line fitting

`bar/status_line.rs` keeps hint fitting private through `HintFit`.
The default policy preserves whole hint groups.
`StatusLine::with_individual_note_hints()` allows individual hints to drop when notes need room.
That policy keeps at least three blank cells before notes.
Applications opt in explicitly; the presence of notes alone does not select the policy.

`with_hint_notes` styles mode notes as enabled shortcut text.
`StatusLineGlobal::fixed` uses the shared hint renderer for modal keys.
The application owns the notes and their meaning.

## Framework overlay wheel handling

`framework/wheel.rs` defines:

```rust
pub fn wheel_framework_overlay<Ctx: SettingsHost>(
    ctx: &mut Ctx,
    position: Position,
    direction: ScrollDirection,
) -> OverlayWheel;
```

`OverlayWheel::{Consumed, NoOverlay}` distinguishes modal consumption from application scrolling.
An open settings, keymap, or shortcuts overlay consumes the event even outside its scrollable content.
It moves selection only when the pointer is inside the relevant area.
The application applies scroll inversion before this call.

Settings uses the same navigation operation for keys and wheel movement.
`SettingsHost::before_settings_navigation` lets the application clear its own transient state first.
The application must not duplicate the three overlay scroll arms.

## Invariants and gotchas

1. Read the environment at the application boundary.
2. Pass the selected color mode through rendering.
3. Clear colors without clearing modifiers.
4. Keep title spaces symmetric when removing them.
5. Use drawn rows for page counts and hit areas.
6. Keep framework overlays modal for wheel events.

A raw terminal capture proves output modifier behavior.
A terminal multiplexer can rewrite those bytes and conceal a color-reset defect.
Shared pane changes can alter cargo-port, cargo-handler, cargo-tile, and cargo-berth frame tests.
