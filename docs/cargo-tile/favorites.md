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
- `r` draws fresh parameters at random across every mode and every
  parameter, and starts showing the result.
- `m` picks a saved favorite at random and shows it. With nothing
  saved, an overlay says so and `esc` dismisses it.
- `u` puts back whatever was on screen before the last replacement.

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
`GlobalShortcuts` -- and the framework's key ladder and hit test match
on it exhaustively. The draw switch does **not**: `render.rs:183` ends
in `_ => ()`, which swallows a future variant along with `None`. D3 is
unaffected, but the claimed compiler check is absent until that arm is
narrowed (F21).

**The repaint loop is demand-driven.** Nothing repaints unless something
asks. `Attract::showing` is what keeps the animation's frames coming;
an overlay animating a row out needs the same, and forgetting it is the
defect recorded in the attract-mode attempts log.

## Decisions to settle before Phase 1

### D1 — the snapshot API on tui_pane [approved]

Approved 2026-08-26. One plain-data struct per animation, plus a
reader, a writer, and a randomizer.

```rust
pub struct BandSettings   { direction, width, speed, tail_speed, fraying }
pub struct TextSettings   { direction, speed, spread, drift, fill }
pub struct PixelSettings  { direction, speed, wave_percent, block_columns, resolve, fill }

impl TravelingBand {
    pub fn settings(&self) -> BandSettings;
    pub fn apply(&mut self, settings: BandSettings);
}
// and the same pair on DriftingText / ResolvingPixels

impl TravelingBand  { pub fn random_settings(seed: u64) -> BandSettings; }
impl DriftingText   { pub fn random_settings(seed: u64) -> TextSettings; }
impl ResolvingPixels{ pub fn random_settings(seed: u64) -> PixelSettings; }
```

`random` lives here rather than in cargo-tile because the ranges live
here. A defect in a range gets fixed where the range is.

It hangs off the **animation**, not the settings struct, because the
band's real width limit is its current line count -- `MAX_BAND_WIDTH =
1000` is a pre-sizing sentinel, not a runtime bound. Generating against
the sentinel and clamping on apply would collapse most seeds onto the
same terminal-dependent maximum. Text and pixels do not need the
context, but one shape across all three keeps the caller uniform (F4).
`Xorshift` in `backdrop/random.rs` is test-only today and has to be
reachable from non-test code; that file joins Phase 1.

The settings fields are **public plain data**. cargo-tile has to build
these values from TOML, and private fields would force an unplanned
getter and constructor per field. `apply` is the boundary that clamps,
so public fields cost nothing (F19).

What a snapshot holds is only what a key steers. Everything else is
runtime state that must **not** be saved or restored, because restoring
it would put a strip halfway across a window it was never sized to:

> `glyphs`, `tails`, `heads`, `phases`, `lanes`, `ripple`, `waved`,
> `grains`, `xorshift`, `faded`, `columns`, `rows`, `cell_pixels`,
> `leading_edge`, `middle`, `rolled_through`

A snapshot therefore *holds* none of those fields. `apply` is a
different matter: it is a **semantic transition, not field assignment**,
and it updates exactly the runtime state the equivalent keypress would
(F3). The existing mutators maintain derived state deliberately --
`TravelingBand::set_direction` rescales width and resets `leading_edge`
and `rolled_through`, `DriftingText::set_direction` rebuilds `lines`,
`cycle_drift` to `Together` resets each line's accumulated drift,
`ResolvingPixels::set_direction` transforms `middle`. Assigning past
them produces states unreachable by steering.

So `apply` must:

- Run in dependency order: direction first, then the enum transitions,
  then the numeric targets. Band width after direction, text spread
  after drift.
- Route every numeric field through the same private clamp helpers the
  setters use. A struct built from hand-edited TOML can carry a zero
  speed or a spread above 100, and direct assignment would admit it.
- Reach absolute values through private absolute setters, with the
  public `cycle_*` methods delegating to them, so one path maintains
  the invariants.

Consequence for Phase 1's test: `settings()` round-trips a *valid*
value exactly; an out-of-range value normalizes rather than round-trips.
Both are tested (F3, F15).

### D2 — serde on the animation enums

`BandDirection`, `BandFraying`, `TextDrift`, `TextFill`,
`PixelResolve`, `PixelFill` have to reach TOML somehow.

- **(a)** Derive `Serialize`/`Deserialize` in tui_pane. Makes serde a
  hard dependency of the backdrop feature and puts the on-disk spelling
  under the library's control.
- **(b)** cargo-tile maps each enum to and from a string in its own
  file model.

**Recommended: (b).** Not because it keeps serde out -- `tui_pane`
already depends on serde with `derive` unconditionally
(`crates/tui_pane/Cargo.toml:25`), so that argument is void (F23). The
reason that stands is ownership: the app that writes the file owns the
file's vocabulary, and an on-disk spelling should not be pinned by a
library that has no other reason to care.

The mapping is a `match` in cargo-tile, with a shape that fails loudly
when a variant is added (F10):

- `enum -> &'static str` is **exhaustive, no wildcard arm**. A new
  tui_pane variant then breaks the build here rather than silently
  losing a spelling.
- `str -> Option<Enum>` stays tolerant, since it has to skip a stale
  file entry.
- The app-owned `AttractMode` needs the same pair for the `mode` tag;
  D2 listed only the six animation enums.
- Every variant of all seven enums gets a round-trip test.

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
would go through `InputContext::app_modal_overlay_hit()` and `ModalHit`
-- the hook tui_pane already provides for exactly this -- rather than a
second ladder. Mouse remains out of scope either way; see Non-goals.

### D4 — the two randomize keys [settled]

`ctrl-shift-r` was the original ask and cannot be delivered as things
stand. A terminal sends the same byte for `ctrl-r` and `ctrl-shift-r`
(0x12) unless the Kitty keyboard protocol is negotiated. cargo-port
pushes those flags (`crates/cargo-port/src/tui/terminal/run.rs:85`);
**cargo-tile does not**. Pushing them here would change key reporting
for every binding in the app, and would still degrade to nothing on a
terminal that will not negotiate.

Settled 2026-08-26, revised in review:

- **`r`** — draw a fresh configuration at random, across every mode and
  parameter. The bigger, less reversible action gets the bare letter,
  because it is the one pressed repeatedly while exploring.
- **`m`** — pick a saved favorite at random. `q` was proposed first and
  cannot be used: it is the framework's `Quit`, so a mis-press would
  exit the app.
- **`u`** — undo the last replacement. See P2.

Plain `r` is unbound in every scope: the framework binds capital `R` to
`Restart` (`crates/tui_pane/src/keymap/global_action.rs:64`), and no
attract scope binds either case. So `r` reaches the app globals through
the ladder untouched.

Every key bound anywhere in cargo-tile today, verified against the
three `bindings!` blocks and `GlobalAction::defaults()`:

| Scope | Keys |
|---|---|
| Framework globals | `q`, `R`, `Tab` / `shift-Tab`, `ctrl-k`, `s`, `?`, `x` |
| App globals | `+` / `=`, `-`, arrows, `f`, `a`, `p` |
| Attract (all three) | arrows, `,` / `.`, `<` / `>`, `[` / `]`, `+` / `=` / `-`, `v`, `t`, `1` / `2` / `3` |

Everything else is free, `m` and `u` included. cargo-tile sets no vim
mode, so `h` `j` `k` `l` are not taken either.

`ctrl-s` and `ctrl-o` stay as asked; both are free. Raw mode disables
`IXON`, so `ctrl-s` is not swallowed as flow control -- worth confirming
live on iTerm2 all the same.

**`x` stops being a close key in cargo-tile.** See P1, settled
2026-08-26. `x` is the framework's `Dismiss` default, which is what
makes it close every overlay in the app -- and what would make a
reflexive press over the favorites table destroy a row. Rather than
route around the collision, cargo-tile removes it: `terminal.rs:451`
lets a framework overlay pass through both the key that opened it and
anything bound to `Dismiss`; the second clause goes. `esc` already
closes those overlays through each one's own cancel binding
(`overlays/settings.rs:139`), so nothing is lost, and `s` / `ctrl-k` /
`?` still toggle their own overlay shut through the first clause.

tui_pane's defaults are **not** touched, so cargo-port keeps `x` for
its own dismiss fallback (`framework_keymap/builder.rs:64`).

With `x` free, favorites deletes on the first press -- no confirmation
step. Left as is: with no overlay open, `x` still clears a visible
toast, since that path does not run through the clause being removed.

### D5 — which scope the five keys belong to

- **(a)** All three attract scopes (`MovingBandAction`,
  `MovingTextAction`, `PixelateAction`), the way `1` / `2` / `3` are
  bound in each.
- **(b)** `AppGlobalAction`.

**Recommended: (b).** One place instead of three near-copies, one
section in the keymap overlay, and it works from the grid as well: `m`
over a working grid gives you a random favorite and turns the attract
screen on to show it. The animations hold their parameters whether or
not they are being drawn, so `ctrl-s` from the grid saves something
real too.

The ladder already suits this: attract-scope keys are offered first,
and none of `ctrl-s`, `ctrl-o`, `m`, `r` or `u` collide with anything
they bind, so all five fall through to the app globals below.

## Design

### The file

`<os config dir>/cargo-tile/favorites.toml`, alongside `config.toml`
and `keymap.toml`, reached through a new `config::favorites_path()`
next to the existing `keymap_path()`.

```toml
[[favorite]]
id            = "01a03f60-2e8b-77c2-858f-476ee413d81c"
saved         = "2026-08-26T14:31:05.412-07:00"
mode          = "pixelate"
direction     = "left"
speed         = 24
wave_percent  = 145
block_columns = 6
resolve       = "scatter"
fill          = "solid"

[[favorite]]
id         = "01a03f5e-9c14-7b41-8a02-1de4c7c9b330"
saved      = "2026-08-26T09:02:44.870-07:00"
mode       = "moving_band"
direction  = "right"
width      = 12
speed      = 40
tail_speed = 96
fraying    = "both"
```

One array of tables, mode-tagged, each holding only the keys its own
mode has. `saved` is RFC 3339 local time with fractional seconds;
chrono is already a dependency. `id` is minted once at save and never
changes -- deletion, selection and the rendered-line map all address a
row by it, never by storage index (F9), and it is what lets a mutation
re-find its row after re-reading the file (F2).

**In memory the parsed `toml` tables are the model**, with a typed
favorite derived from each table for display and loading (F1). This is
the difference between skipping a row and destroying it. A row whose
`mode` is unknown, or whose enum spelling does not parse, is **skipped
for display** -- the posture `keymap.toml` already takes toward a stale
entry -- but it is still written back out on the next save or delete.
Serializing only the recognized rows would silently delete a favorite
written by a newer version, or one hand-edited with a typo. Unknown
*keys* on an otherwise-good row survive the same way.

A file that does not exist is an empty list, not an error. The other
three outcomes are distinct states, not all folded into "empty" (F17):

| State | `ctrl-o` | `ctrl-s` | `x` |
|---|---|---|---|
| `Missing` | empty notice | writes a new file | n/a |
| `Loaded` | the table | appends | deletes |
| Whole-file parse error | shows path + parse error | refused | refused |
| Read failure | shows path + error | refused | refused |

Refusing rather than overwriting matches what `config.toml` already
does with a file that failed to parse. Reporting "nothing saved" over
a file that exists but cannot be read would be a lie, and letting
`ctrl-s` replace a damaged file with one row loses everything in it.

**Every mutation is a locked read-modify-write ending in an atomic
replace** (F2): take a sibling lock file, re-read and re-parse under it,
mutate the raw table list by `id`, write a temporary file in the same
directory, flush, and rename over `favorites.toml`. Two running
instances otherwise each hold a stale snapshot and the later writer
drops the earlier one's favorite; a direct `fs::write` interrupted
mid-way leaves a truncated file. The cost is one lock and one reparse
per keypress-driven mutation, which is not a per-frame path.

Saving is **idempotent on `(mode, settings)`** (F20): an identical
parameter set updates the existing row's `saved` rather than adding a
second row. Repeated `ctrl-s` otherwise clutters the table with
indistinguishable rows and gives that one parameter set extra weight in
`m`'s uniform draw. Within a mode, rows are ordered newest first.

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
  ▸ 26 Aug 14:31:05    left        24    145      6  scatter   solid
    25 Aug 22:07:19    up          12     60      3  blend     shade

  Attract: Moving Band
    Saved              Direction  Width  Speed  Tail   Fraying
                       ←↑↓→       -/+    </>    [/]    v
    26 Aug 09:02:44    right         12     40     96  both

  ↑↓ move   enter load   x delete   esc close
```

The key line under each header is read from the **live keymap** -- via
the scope for `AppPaneId::Attract(mode)`, resolved with
`Keymap::key_for_toml_key` and rendered with
`KeySequence::display_short()`, not `KeyBind::display_short` (F24) --
so a rebound key shows through rather than a hardcoded label going
stale.

That needs an explicit **per-column descriptor**, because the mapping
is not one-to-one (F12). A displayed parameter usually covers a *pair*
of actions with aliases on each: band speed is `SpeedFaster` and
`SpeedSlower`, bound to `>` and `<` shifted and unshifted. The
descriptor names the action or action pair per column, the policy is
primary-binding-per-action, and an unbound half renders as a blank
rather than a stale default. The band example above had Tail on `</>`
and Speed on `,/.`; the real defaults are Speed `</>` and Tail `[/]`
(`attract/moving_band.rs:95`) -- which is exactly the class of error a
descriptor plus a test against the resolved keymap prevents.

Timestamps display to the second, and carry the year when the row is
not from the current year (F22). Minute precision is not enough to tell
two saves of a similar parameter set apart.

**Narrow terminals** are their own problem (F13). `ColumnWidths` grows
columns to content and the keymap overlay's `columns_that_fit` reduces
the number of side-by-side *sections* -- neither makes a seven-column
row fit a terminal narrower than the row. `Saved` and the selection
marker pin to the left edge; the parameter columns page horizontally
with left/right, one whole column at a time; the header, key line and
cells are all built from the same visible-column slice. Chosen over
clipping or dropping columns by priority: paging is the only one of the
three where you can still see the value you are about to load.

The empty case is a non-selectable line, not an empty table: `No
favorites saved -- press <live ctrl-s label> to save one`. A list with
one mode renders that mode's section only, with no others stubbed in.

### Deleting with a fade

`x` marks the selected row `Removing { since: Instant }` rather than
dropping it. Alpha is computed from `now - since` against a fixed fade
duration, **not** incremented per draw -- otherwise an unrelated scan
or keypress adds frames and the fade runs faster (F6). The row leaves
the selection set the moment deletion starts and the cursor moves to
the next active row, but it keeps its rendered line until the fade ends
(F9). When alpha reaches `u8::MAX` the row is dropped by `id`, the
table is laid out again without it, and the file is rewritten.

The overlay must report that it owes frames while a removal is in
flight, the way `Attract::showing` does, or the fade draws one frame
and stops. This is the exact defect in the attract-mode attempts log;
treat it as a requirement, not an afterthought. Three details decide
whether that requirement is actually met (F6):

- **Where it advances.** `FavoritesOverlay::advance(now)` runs from
  `terminal::event_loop` **outside** the `Updates::Frozen` branch, on
  its own deadline. The attract screen's frame request sits inside that
  branch (`terminal.rs:290`); copying its placement would freeze the
  deletion fade, and leaning on `Attract::showing` would only work when
  the attract screen happens to be up -- a delete over a working grid
  would stop after its event-driven frame.
- **Where the commit happens.** `advance` returns whether a repaint or
  a final removal is owed. Mutation and file I/O stay out of
  `render::draw`; discovering `u8::MAX` mid-render and writing the file
  there puts a disk write inside a frame.
- **Closing mid-fade.** Deletion is committed at `x`, not at fade end.
  If the overlay closes while a row is fading, the row is removed and
  the file written immediately.

Row rendering is **cached, not rebuilt per frame** (F18). The keymap
overlay builds, formats and measures every row before applying its
scroll offset; copied here, each fade frame would do O(total favorites)
of string work to animate one row. The grouped line plan and the
formatted cells are built on open, on mutation, on keymap replacement
and on width change; a frame renders only the lines intersecting the
viewport and recolors the fading row. No count or file-size cap is
imposed -- with the cache in place the per-frame cost is O(visible
rows), and refusing a save is a worse experience than a slower open.

### Loading

`enter` sets `Attract::mode` to the row's mode, calls `apply` on that
animation with the row's settings, closes the overlay, and asks for the
attract screen **unconditionally** through a new idempotent
`Attract::request_show()` (F5). Not "if it is not already showing":
`Attract::showing()` only tests that the fade is off its maximum, so it
stays true through a fade-*out*, and a load landing in that window
would skip the request and watch the favorite it just loaded disappear.
`toggle()` is equally unsuitable, since it can ask for the opposite
state. The other two animations
keep whatever they were last steered to, which is what already makes
`1` / `2` / `3` a turn rather than a restart.

## Phases

Each phase ends green: `cargo build && cargo +nightly fmt`, clippy
clean, `cargo nextest run` passing, and the patch version bumped.

### Phase 1 — the snapshot API

`crates/tui_pane/src/backdrop/{band,text,pixels,random}.rs`, `lib.rs`,
tui_pane CHANGELOG.

Add the three settings structs with public fields, `settings()` /
`apply()` and `random_settings(seed)` on each animation, and the
private absolute setters `apply` and the `cycle_*` methods both route
through. Make `Xorshift::seeded` reachable outside tests. Export from
`lib.rs` under the `backdrop` feature.

Done when:

- A valid settings value taken from an animation and applied to a
  fresh one round-trips exactly through `settings()`.
- `apply` on an **already-running, already-sized** animation preserves
  the invariants: every direction change, every drift change and every
  fraying change, from a non-default starting state, leaves the same
  runtime state the equivalent keypress would.
- Arbitrary constructed values normalize rather than round-trip --
  `0` and `u32::MAX` on every numeric field land inside the clamps.
- `random_settings` is deterministic per seed and, over a fixed seed
  corpus, **every field varies and every enum variant is reachable**
  (F16). A generator returning one constant valid value must fail this,
  which it would pass under "only ever produces values inside the
  clamps".
- Band width drawn for a sized band lands inside that band's own axis
  extent, not the `MAX_BAND_WIDTH` sentinel.

### Phase 2 — the file

`crates/cargo-tile/src/favorites.rs` (new), `config.rs`, `constants.rs`.

The raw-table model with a typed favorite derived per row, the
exhaustive enum-to-string mapping from D2 (including `AttractMode`),
`favorites_path()`, load, save, push, remove -- each mutation locked,
re-read, addressed by `id`, and committed by atomic replace.

The typed payload is **one enum, not a string plus optional fields**
(F11):

```rust
struct Favorite { id: FavoriteId, saved: DateTime<FixedOffset>, settings: FavoriteSettings }
enum FavoriteSettings { MovingBand(BandSettings), MovingText(TextSettings), Pixelate(PixelSettings) }
```

`mode` is derived from the variant. A `mode: String` alongside optional
per-mode fields would let missing, mixed and mismatched settings past
parsing, and every later consumer -- grouping, `m`, `enter` -- would
re-derive a relationship the type already carries. The raw
`toml::Table` stays confined to parsing so unknown rows survive.

Done when:

- A list survives save and load unchanged.
- An entry with an unknown mode or a misspelled enum is skipped for
  display **and is still present in the file after a save and after a
  delete** (F1). Same for an unknown key on a recognized row.
- Truncated or otherwise unparseable TOML puts favorites in a
  read-only error state carrying the path; `ctrl-s` and `x` are
  refused, not silently applied to an empty list.
- A missing file loads as empty.
- Every variant of all seven enums round-trips; the `enum -> str`
  match has no wildcard arm.
- Saving an identical `(mode, settings)` twice leaves one row with the
  later timestamp.

### Phase 3 — `ctrl-s`

`globals.rs`, `attract/mod.rs`, `render.rs`, `terminal.rs`.

`AppGlobalAction::SaveFavorite`. `Attract` gains a method returning the
current mode's settings as a favorite row. A toast confirms the save,
and reports the path on a write failure.

`render.rs` and `terminal.rs` are here because **the toast has no path
to the screen yet** (F7). `App` owns `framework.toasts`, but
`render::draw` never renders it and `event_loop` never calls
`Toasts::prune` or schedules its entrance, expiry and exit frames --
pushing a toast today produces nothing at all. Render the stack with
`ToastsRenderCtx` beneath the modal overlays, prune it from the loop
outside the `Updates::Frozen` branch, and fold its animation deadlines
into the same visual deadline the deletion fade uses. Frames are asked
for during the entrance and exit only, with one wake at expiry.

Persistence stays synchronous on the dispatch path, matching
`config.rs`. A few KB of TOML behind a lock is not a frame hazard, and
a worker thread plus a reply channel is machinery the next reader of
this code would have to hold for no measured gain.

Done when: pressing the key with each of the three modes showing writes
a row that reads back as that mode's current parameters; the same
holds with the attract screen **fully hidden**, per D5 (F15); the
success toast and the write-failure toast both appear on screen and
expire.

### Phase 4 — the overlay

`crates/cargo-tile/src/favorites_overlay.rs` (new), `render.rs`,
`terminal.rs`, `app.rs`, `globals.rs`, `keymap.rs`, `interaction.rs`,
`attract/mod.rs`.

`AppGlobalAction::OpenFavorites`. One `FavoritesOverlay` controller
owns open state, the row list, selection, the `Viewport`, the removal
fade, the cached line plan, rendering, input and `frame_owed()`; `App`
owns one instance (F14). Spreading state across `App`, drawing across
two files, input across `terminal.rs` and fade scheduling elsewhere is
how the one-frame defect gets reintroduced. `attract/mod.rs` is on the
list because `enter` has to reach private `Attract` fields -- add
`Attract::apply_favorite()` and `Attract::request_show()` there.
Measurement uses `ColumnWidths`; scrolling uses `Viewport`.

The overlay is a **complete modal**, not just a key-order tweak (F8).
`AppOverlay::{Favorites, NoFavorites}` with a registered
`FavoritesOverlayAction` scope and `AppPaneId::Favorites`, per
`docs/cargo-port/style/adding-a-keybinding.md`, so the footer labels
follow rebinding like every other surface. While an `AppOverlay` is
open its scope is dispatched and **every** key is consumed, unmatched
ones as no-ops. Taking only the recognized keys ahead of the framework
check leaves `ctrl-r` randomizing behind the popup and `?` opening a
framework overlay on top of it. At most one app or framework modal is
open at a time. Mouse stays a non-goal; if a click path is added later
it uses the existing `InputContext::app_modal_overlay_hit()` rather
than a second hit-test ladder.

Also narrow `render.rs:183`'s `_ => ()` to `None => ()` (F21), and
drop the `|| matches!(action, GlobalAction::Dismiss)` clause from
`terminal.rs:451` so `x` no longer closes a framework overlay (P1).

Done when: the table groups by mode with per-mode headers and live key
labels matching the resolved keymap column by column; the selection
walks every row across sections; a list too tall to fit scrolls and
keeps the cursor visible; a table wider than the terminal pages its
parameter columns with `Saved` pinned; `x` fades the row and rewrites
the file **with the attract screen fully hidden, with updates frozen,
and with no other events arriving** (F15); closing mid-fade still
removes the row; `enter` loads and the animation changes; a key
bound to a global action does nothing while the overlay is open; and
`x` over an open settings, keymap or global-shortcuts overlay leaves it
open while `esc` still closes it (P1).

### Phase 5 — `m`

`globals.rs`, `favorites.rs`, `favorites_overlay.rs`.

`AppGlobalAction::RandomFavorite` on `m`. Picks uniformly from the saved list
and loads it. With an empty list, `AppOverlay::NoFavorites` -- the
empty state already defined for `ctrl-o`, reused from the same
controller rather than a second notice overlay with its own owner
(F14) -- says so and `esc` dismisses it.

Done when: selection is proven through its **bounded index draw against
a fixed seed corpus**, not by pressing the key until the row changes
(F16) -- a valid list can legitimately return the same row twice, so
"repeated presses visibly move" is a flaky condition; and an empty list
opens the notice, which renders and consumes `esc` through the app
overlay route ahead of framework handling.

### Phase 6 — randomize everything (`r`)

`globals.rs`, `attract/mod.rs`.

Draws a mode at random, draws that mode's settings at random via
Phase 1's `random_settings` on the chosen animation, applies both, and
turns the attract screen on with `request_show()` (F5).

`AppGlobalAction::UndoReplace` on `u` lands here too, since this is the
key most likely to overshoot. The checkpoint is captured by whichever
of the three replacing actions runs, so Phases 4 and 5 populate it as
well.

Done when: over a fixed seed corpus every mode and every enum variant
is reached, every value sits inside its clamps, and the animation's
`settings()` after the action equals the generated target (F16) --
which is what proves the settings were applied and not merely drawn;
and `u` after each of the three replacing actions restores the mode,
all three parameter sets and the attract screen's visibility.

## Non-goals

- No mouse support inside the favorites overlay. Keyboard first; a
  click path is separate work, and when it comes it goes through the
  existing `InputContext::app_modal_overlay_hit()` hook rather than a
  second hit-test ladder.
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
| `x` is already the framework's `Dismiss` | cargo-tile drops the dismiss clause at `terminal.rs:451`, so `x` closes nothing and `esc` keeps doing it (P1). tui_pane untouched |
| `ctrl-s` swallowed as XOFF | Raw mode disables `IXON`; confirm live on iTerm2 before Phase 3 ships |
| Delete fade draws one frame and stops | Frame-owed reporting is part of Phase 4's completion condition, not a follow-up |
| A saved favorite from a differently-sized window loads wrong | Snapshots exclude every size-derived field; `wave_percent` is already a share rather than a distance |
| The overlay is too wide for a narrow terminal | `94bd49e4`'s work reduces the number of side-by-side *sections*; it does not fit one wide row. `Saved` pins, parameter columns page with left/right (F13) |
| A save or delete deletes the rows it could not parse | The parsed `toml` tables are the persistence model; typed favorites are derived for display only (F1) |
| Two instances each rewrite the file | Locked re-read, mutate by `id`, atomic replace (F2) |
| `apply` puts an animation in a state steering cannot reach | `apply` is an ordered transition through the existing setters, not field assignment (F3) |

## Review findings (auto-recorded)

Team review, 2026-08-26 -- five expert reviewers (correctness,
architecture, type system, risk and cost, ergonomics). No reviewer
raised a premise-challenge; all five accept the approach. The findings
below had one correct outcome and are folded into the sections above.
Two judgment calls survived to **Proposed user decisions**.

| # | Finding | Where recorded | Severity |
|---|---|---|---|
| F1 | A save or delete serializes only recognized rows, deleting every skipped or newer-version row | The file | critical |
| F2 | Whole-file rewrite is neither atomic nor safe against a second instance; no stable row identity | The file | critical |
| F3 | `apply` as field assignment bypasses the invariants the setters maintain, and admits out-of-range values | D1 | important |
| F4 | `BandSettings::random(seed)` cannot know the real width limit; the sentinel would collapse most seeds onto one width | D1 | important |
| F5 | `Attract::showing()` stays true through a fade-out, so "ask if not already showing" loses the favorite just loaded | Loading | important |
| F6 | The fade's frame request, alpha source and commit point are all unspecified; the obvious placements freeze or accelerate it | Deleting with a fade | important |
| F7 | `framework.toasts` is never rendered or pruned, so Phase 3's toast cannot appear at all | Phase 3 | important |
| F8 | Taking only the recognized keys leaves globals and framework overlays live behind the popup | Phase 4 | important |
| F9 | Storage index served as identity, selection ordinal and rendered line at once | The file, Deleting with a fade | important |
| F10 | Nothing forced the `enum -> str` direction to be exhaustive, so a new variant loses its spelling silently | D2 | important |
| F11 | `mode: String` plus optional per-mode fields admits mixed and mismatched settings | Phase 2 | important |
| F12 | A column maps to an action *pair* with aliases; no rule said which binding is displayed | The overlay | important |
| F13 | The cited column machinery reduces sections, it does not fit a wide row to a narrow terminal | The overlay | important |
| F14 | Overlay state, drawing, input and fade scheduling were spread across five files with no owner | Phase 4, Phase 5 | important |
| F15 | Phases 3-5 could pass while missing the hidden-screen, frozen-updates and dismiss behavior they promise | Phases 3-5 | important |
| F16 | The random completion conditions are probabilistic and pass a constant generator | Phases 1, 5, 6 | important |
| F17 | Malformed TOML, read failure and write failure had no defined behavior | The file | important |
| F18 | Copying the keymap overlay's build-everything-then-scroll pattern makes each fade frame O(total favorites) | Deleting with a fade | important |
| F19 | Public structs with private fields cannot be built from TOML in cargo-tile | D1 | important |
| F20 | Repeated `ctrl-s` adds indistinguishable rows and skews `m`'s uniform draw | The file | minor |
| F21 | `render.rs:183` ends in `_ => ()`, so a future framework overlay compiles with no draw arm | What is already there, Phase 4 | minor |
| F22 | Minute precision cannot tell two saves of a similar parameter set apart | The overlay | minor |
| F23 | D2's stated reason is void: `tui_pane` already depends on serde unconditionally | D2 | minor |
| F24 | The band example labels Tail `</>`; the defaults are Speed `</>`, Tail `[/]`. The display API is `KeySequence::display_short()` | The overlay | minor |

Two reviewer recommendations were **declined**, with the reason
recorded where the work is:

- *A persistence worker thread and reply channel.* Declined in Phase 3:
  the write is a few KB behind a lock on a keypress path, and the
  thread is complexity a later reader has to hold for no measured gain.
  The real defect underneath it -- file I/O inside `render::draw` -- is
  fixed by F6 instead.
- *A hard favorites count and file-size cap.* Declined in Deleting with
  a fade: the caching in F18 removes the per-frame cost that motivated
  it, and refusing to save is a worse outcome than a slower open.

## Proposed user decisions

### P1 -- `x` deletes a saved row with no confirmation [settled]

Settled 2026-08-26: **remove the collision instead of confirming past
it.** `x` is the framework's `Dismiss` default, so it closes every
overlay in the app -- which is what would make a reflexive press over
the favorites table destroy a saved row.

cargo-tile drops the second clause at `terminal.rs:451`, the one that
lets any `Dismiss`-bound key through an open framework overlay. `esc`
already closes those overlays through each one's own cancel binding, so
that path is unaffected, and `s` / `ctrl-k` / `?` still toggle their own
overlay shut through the first clause. tui_pane's defaults are not
touched, so cargo-port keeps `x` for its dismiss fallback.

Favorites therefore deletes on the first press, with no confirmation
step -- the reflex that made one dangerous no longer exists.

Left as is: with no overlay open, `x` still clears a visible toast.
That path does not run through the removed clause, toasts expire on
their own, and a toast is not something you press `x` to escape.

Recorded in D4, Phase 4 and Risks.

### P2 -- a random draw replaces settings you cannot get back [settled]

Settled 2026-08-26: **one step back exists.**

`r`, `m` and `enter` in the table all replace the
current mode's parameters wholesale. The failure is the press after the
good one -- something appears that you like and your hand has already
pressed the key again. Saving first is the intended workflow, but the
moment you would need it is the moment you do not take it.

Before any of the three replacing actions, capture the current mode,
all three parameter sets, and whether the attract screen was up. `u`
restores them. It covers all three, not just the random draw: an undo
that catches one but not the others is worse than none, because you
cannot predict which press it will catch.

`u` is unbound in every scope; only `ctrl-u` is taken, by tui_pane's
vim half-page scroll, and cargo-tile sets no vim mode.

Recorded in The request, D4 and Phase 6.
