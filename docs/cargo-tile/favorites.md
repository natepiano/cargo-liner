# cargo-tile — attract favorites

Save the attract screen's current parameters, list what has been saved,
load one back, and pick one at random.

## The request

- `ctrl-s` writes the running attract mode's parameters to a file
  beside `config.toml` and `keymap.toml`.
- `ctrl-o` opens a table of saved favorites: one row each, with the
  datetime, the mode, and a column per parameter. Each column header
  also names the key that steers that parameter. The list scrolls.
  `x` deletes a row -- fading it out the way other text fades, then
  repainting the table without it. `enter` loads the row.
- `ctrl-r` picks a saved favorite at random and shows it. With nothing
  saved, an overlay says so and `esc` dismisses it.
- `ctrl-shift-r` draws fresh parameters at random across every mode and
  every parameter, and starts showing the result.

## What is already there

Read before planning against it:

| Thing | Where |
|---|---|
| Config directory (`config_root`) | `crates/cargo-tile/src/config.rs:229` |
| `config.toml` load / save | `crates/cargo-tile/src/config.rs:143`, `:189` |
| Attract state, mode switching | `crates/cargo-tile/src/attract/mod.rs:176` |
| The three animations | `crates/tui_pane/src/backdrop/{band,text,pixels}.rs` |
| App global actions | `crates/cargo-tile/src/globals.rs:31` |
| Key dispatch ladder | `crates/cargo-tile/src/terminal.rs:447` |
| Overlay draw switch | `crates/cargo-tile/src/render.rs:183` |
| Scrolling viewport | `crates/tui_pane/src/layout/viewport.rs` |
| Colour fading | `tui_pane::blend_color` |

Four facts drive most of the design below.

**The animations have no readers.** `TravelingBand`, `DriftingText` and
`ResolvingPixels` expose only mutators -- `widen`, `speed_up`,
`cycle_fill`. Nothing can ask any of them what it is currently set to,
so nothing can be saved without new API on all three.

**Their clamps are private.** `MIN_BAND_SPEED`, `MAX_BLOCK_COLUMNS`,
`MAX_PIXEL_WAVE_PERCENT` and the rest are `pub(super)` inside
`backdrop/constants.rs`. A randomizer written in cargo-tile would have
to restate every range, and would go stale the first time one moved.

**`FrameworkOverlayId` is a closed enum.** `Settings`, `Keymap`,
`GlobalShortcuts` -- and the framework's key ladder, hit test and draw
switch all match on it exhaustively.

**The repaint loop is demand-driven.** Nothing repaints unless something
asks. `Attract::showing` is what keeps the animation's frames coming;
an overlay animating a row out needs the same, and forgetting it is the
defect recorded in the attract-mode attempts log.

## Decisions to settle before Phase 1

### D1 — the snapshot API on tui_pane [needs approval]

This is a public API addition to `tui_pane` and cannot proceed without
an explicit yes.

Proposed: one plain-data struct per animation, plus a reader, a writer,
and a randomizer.

```rust
pub struct BandSettings   { direction, width, speed, tail_speed, fraying }
pub struct TextSettings   { direction, speed, spread, drift, fill }
pub struct PixelSettings  { direction, speed, wave_percent, block_columns, resolve, fill }

impl TravelingBand {
    pub fn settings(&self) -> BandSettings;
    pub fn apply(&mut self, settings: BandSettings);
}
// and the same pair on DriftingText / ResolvingPixels

impl BandSettings  { pub fn random(seed: u64) -> Self; }
impl TextSettings  { pub fn random(seed: u64) -> Self; }
impl PixelSettings { pub fn random(seed: u64) -> Self; }
```

`random` lives here rather than in cargo-tile because the ranges live
here. A defect in a range gets fixed where the range is.

What a snapshot holds is only what a key steers. Everything else is
runtime state that must **not** be saved or restored, because restoring
it would put a strip halfway across a window it was never sized to:

> `glyphs`, `tails`, `heads`, `phases`, `lanes`, `ripple`, `waved`,
> `grains`, `xorshift`, `faded`, `columns`, `rows`, `cell_pixels`,
> `leading_edge`, `middle`, `rolled_through`

`apply` therefore sets the steerable parameters and leaves the rest
where the running animation already has them, which is what makes
loading a favorite a change of settings rather than a restart.

### D2 — serde on the animation enums

`BandDirection`, `BandFraying`, `TextDrift`, `TextFill`,
`PixelResolve`, `PixelFill` have to reach TOML somehow.

- **(a)** Derive `Serialize`/`Deserialize` in tui_pane. Makes serde a
  hard dependency of the backdrop feature and puts the on-disk spelling
  under the library's control.
- **(b)** cargo-tile maps each enum to and from a string in its own
  file model.

**Recommended: (b).** tui_pane stays free of serde, and the app that
writes the file owns the file's vocabulary. The enums are public with
public variants, so the mapping is a `match` in cargo-tile and nothing
more.

### D3 — where the overlay lives

- **(a)** Add `FrameworkOverlayId::Favorites`. Public API change to
  tui_pane, and it puts a cargo-tile concept inside the framework.
- **(b)** App-local: cargo-tile holds the overlay state, draws it from
  `render::draw_overlay`, and takes its keys at the top of `handle_key`
  ahead of the framework overlay check.

**Recommended: (b).** Favorites are an attract-screen idea, not a
framework one -- the same reasoning that gives each app its own theme
files. It also keeps this feature off tui_pane's public surface
entirely apart from D1.

Consequence: the framework's hit-test ladder is keyed on
`FrameworkOverlayId`, so mouse selection inside the favorites overlay
needs an app-local path of its own. See Non-goals.

### D4 — `ctrl-shift-r` cannot be delivered as asked, as things stand

A terminal sends the same byte for `ctrl-r` and `ctrl-shift-r` (0x12)
unless the Kitty keyboard protocol is negotiated. cargo-port pushes
those flags (`crates/cargo-port/src/tui/terminal/run.rs:85`);
**cargo-tile does not**. So today crossterm reports both presses
identically and the two actions cannot be told apart.

`KeyBind` itself is ready either way -- `normalized` folds
`ctrl-shift-r` to `Char('R') + CONTROL`, distinct from `Char('r') +
CONTROL`.

- **(a)** Push `DISAMBIGUATE_ESCAPE_CODES |
  REPORT_ALL_KEYS_AS_ESCAPE_CODES` in cargo-tile's terminal setup, as
  cargo-port does. Works on iTerm2 3.5+ and kitty; degrades silently to
  nothing elsewhere, so `ctrl-shift-r` would do nothing on a terminal
  that will not negotiate. It also changes key reporting for *every*
  binding in the app, which is a wider change than this feature needs.
- **(b)** Bind the randomize-everything action to a key that needs no
  negotiation -- `ctrl-g` is free, as are `ctrl-e`, `ctrl-n`, `ctrl-t`.

**Recommended: (b), with the key being the user's pick.** (a) risks
existing bindings for one shortcut. This one genuinely needs an answer
rather than a default -- it depends on which terminal is being used and
whether a global change in key reporting is acceptable.

Free control keys, for reference. Taken: `ctrl-k` (OpenKeymap),
`ctrl-b` / `ctrl-f` / `ctrl-u` / `ctrl-d` (navigation paging).
`ctrl-s`, `ctrl-o`, `ctrl-r` are all free. Raw mode disables `IXON`, so
`ctrl-s` is not swallowed as flow control -- worth confirming live on
iTerm2 all the same.

### D5 — which scope the four keys belong to

- **(a)** All three attract scopes (`MovingBandAction`,
  `MovingTextAction`, `PixelateAction`), the way `1` / `2` / `3` are
  bound in each.
- **(b)** `AppGlobalAction`.

**Recommended: (b).** One place instead of three near-copies, one
section in the keymap overlay, and it works from the grid as well:
`ctrl-r` over a working grid gives you a random favorite and turns the
attract screen on to show it. The animations hold their parameters
whether or not they are being drawn, so `ctrl-s` from the grid saves
something real too.

The ladder already suits this: attract-scope keys are offered first, and
none of them will bind a control chord, so these fall through to the
app globals below.

## Design

### The file

`<os config dir>/cargo-tile/favorites.toml`, alongside `config.toml`
and `keymap.toml`, reached through a new `config::favorites_path()`
next to the existing `keymap_path()`.

```toml
[[favorite]]
saved         = "2026-08-26T14:31:05-07:00"
mode          = "pixelate"
direction     = "left"
speed         = 24
wave_percent  = 145
block_columns = 6
resolve       = "scatter"
fill          = "solid"

[[favorite]]
saved      = "2026-08-26T09:02:44-07:00"
mode       = "moving_band"
direction  = "right"
width      = 12
speed      = 40
tail_speed = 96
fraying    = "both"
```

One array of tables, mode-tagged, each holding only the keys its own
mode has. `saved` is RFC 3339 local time; chrono is already a
dependency.

A row whose `mode` is unknown, or whose enum spelling does not parse, is
**skipped rather than failing the load** -- the posture `keymap.toml`
already takes toward a stale entry. A file that does not exist is an
empty list, not an error.

### The overlay

Modes hold disjoint parameters, so one flat table would be mostly
blanks. Group by mode instead: a heading per mode that has favorites,
its own column header, then its rows. Selection walks every row across
every section as one list, so scrolling, `x` and `enter` behave as a
single list regardless of the grouping.

```
  Favorites                                                    3 saved

  Attract: Pixelate
    Saved              Direction  Speed  Wave  Block  Resolve  Fill
                       ←↑↓→       ,/.    [/]   -/+    v        t
  ▸ 26 Aug 14:31       left        24    145      6  scatter   solid
    25 Aug 22:07       up          12     60      3  blend     shade

  Attract: Moving Band
    Saved              Direction  Width  Speed  Tail   Fraying
                       ←↑↓→       -/+    ,/.    </>    v
    26 Aug 09:02       right         12     40     96  both

  ↑↓ move   enter load   x delete   esc close
```

The key line under each header is read from the **live keymap** -- via
the scope for `AppPaneId::Attract(mode)` and `KeyBind::display_short`
-- so a rebound key shows through rather than a hardcoded label going
stale.

### Deleting with a fade

`x` marks the selected row `Removing { since: Instant }` rather than
dropping it. Each frame carries the row's cells toward the popup's
ground with `blend_color`, on the same alpha scale the attract screen
fades on. When it reaches `u8::MAX` the row is dropped, the table is
laid out again without it, and the file is rewritten.

The overlay must report that it owes frames while a removal is in
flight, the way `Attract::showing` does, or the fade draws one frame
and stops. This is the exact defect in the attract-mode attempts log;
treat it as a requirement, not an afterthought.

### Loading

`enter` sets `Attract::mode` to the row's mode, calls `apply` on that
animation with the row's settings, closes the overlay, and asks for the
attract screen if it is not already showing. The other two animations
keep whatever they were last steered to, which is what already makes
`1` / `2` / `3` a turn rather than a restart.

## Phases

Each phase ends green: `cargo build && cargo +nightly fmt`, clippy
clean, `cargo nextest run` passing, and the patch version bumped.

### Phase 1 — the snapshot API

`crates/tui_pane/src/backdrop/{band,text,pixels}.rs`, `lib.rs`,
tui_pane CHANGELOG.

Add the three settings structs, `settings()` / `apply()` on each
animation, and `random(seed)` on each struct. Export from `lib.rs`
under the `backdrop` feature.

Done when: a settings value taken from an animation, applied to a fresh
one, produces an animation that answers `settings()` with the value it
was given; and `random` over many seeds only ever produces values
already inside the clamps the setters enforce.

### Phase 2 — the file

`crates/cargo-tile/src/favorites.rs` (new), `config.rs`, `constants.rs`.

The TOML model, the enum-to-string mapping from D2, `favorites_path()`,
load, save, push, remove.

Done when: a list survives save and load unchanged; an entry with an
unknown mode or a misspelled enum is skipped while its neighbours load;
a missing file loads as empty.

### Phase 3 — `ctrl-s`

`globals.rs`, `attract/mod.rs`.

`AppGlobalAction::SaveFavorite`. `Attract` gains a method returning the
current mode's settings as a favorite row. A toast confirms the save,
and reports the path on a write failure.

Done when: pressing the key with each of the three modes showing writes
a row that reads back as that mode's current parameters.

### Phase 4 — the overlay

`crates/cargo-tile/src/favorites_ui.rs` (new), `render.rs`,
`terminal.rs`, `app.rs`, `globals.rs`.

`AppGlobalAction::OpenFavorites`. Overlay state on `App`, the grouped
table, the scrolling viewport, `enter` to load, `x` to delete with the
fade, `esc` to close, and the frame-owed reporting the fade needs.

Done when: the table groups by mode with per-mode headers and live key
labels; the selection walks every row across sections; a list too tall
to fit scrolls; `x` fades the row and rewrites the file; `enter` loads
and the animation changes.

### Phase 5 — `ctrl-r`

`globals.rs`, `favorites.rs`, plus a small notice overlay.

`AppGlobalAction::RandomFavorite`. Picks uniformly from the saved list
and loads it. With an empty list, the notice overlay says so and `esc`
dismisses it.

Done when: repeated presses over a list of several visibly move between
them, and an empty list produces the notice rather than doing nothing.

### Phase 6 — randomize everything

`globals.rs`, `attract/mod.rs`, and the key settled in D4.

Draws a mode at random, draws that mode's settings at random via
Phase 1's `random`, applies both, and turns the attract screen on.

Done when: repeated presses land on all three modes over enough tries,
and every drawn value sits inside its clamps.

## Non-goals

- No mouse support inside the favorites overlay. The framework's
  hit-test ladder is keyed on `FrameworkOverlayId`, which D3 declines
  to extend. Keyboard first; a click path is separate work.
- No naming or editing of favorites. `saved` and the parameters are the
  whole row.
- No favorites in `config.toml`. A list that grows by keypress does not
  belong in a file the app rewrites to restate its defaults.
- No migration. The file does not exist yet, so there is no old format.
- No change to what any animation draws. Phases 1–6 add reading,
  writing and applying of parameters that the keys already set.

## Risks

| Risk | Mitigation |
|---|---|
| `ctrl-shift-r` is undeliverable without a protocol change | D4 -- settle the key before Phase 6 |
| `ctrl-s` swallowed as XOFF | Raw mode disables `IXON`; confirm live on iTerm2 before Phase 3 ships |
| Delete fade draws one frame and stops | Frame-owed reporting is part of Phase 4's completion condition, not a follow-up |
| A saved favorite from a differently-sized window loads wrong | Snapshots exclude every size-derived field; `wave_percent` is already a share rather than a distance |
| The overlay is too wide for a narrow terminal | The keymap overlay's column work landed in `94bd49e4`; reuse its width handling rather than writing a second one |
