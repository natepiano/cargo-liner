# cargo-tile — attract favorites

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Save the attract screen's current parameters, list what has been saved, load one back, pick one at random, and step back once.

## Delegation Context
<!-- Shared across all phases. /plan:delegate prepends this to every dispatch. -->

- **Project:** `tui_pane` (workspace lib, `crates/tui_pane`, v0.8.0-dev) — reusable
  ratatui pane framework: keymap, status bar, overlays, toasts, backdrop/attract
  animations. `cargo-tile` (bin, `crates/cargo-tile`, v0.2.53-dev) — terminal UI
  cargo tool; owns the attract screen, config/keymap files, grid. Phase 1 is
  tui_pane; Phases 2–8 are cargo-tile. `cargo-berth`, `cargo-mend` and
  `cargo-port` are untouched.
- **Stack:** Rust edition 2024, resolver 3. Workspace deps these phases use:
  `ratatui 0.30.2`, `crossterm 0.29.0`, `chrono 0.4.45` (cargo-tile only),
  `toml 1.1.4`, `serde 1` (`derive`, in both crates), `dirs 6.0.0`,
  `tempfile 3.27.0` (cargo-tile dev-dep), `uuid 1` (`serde`, `v7` — declared at
  workspace level, **not** yet a cargo-tile dependency; Phase 2 adds it).
  The workspace declares **no** file-locking crate.
- **Layout:**
  - `crates/tui_pane/src/backdrop/` — `mod.rs` (private submodules + re-exports),
    `band.rs`, `text.rs`, `pixels.rs`, `constants.rs`, `random.rs`; plus
    `desktop.rs` / `monitor.rs` / `query.rs` (screen capture, untouched).
  - `crates/tui_pane/src/` — `lib.rs`, `layout/{viewport,column_widths}.rs`,
    `overlays/{keymap_ui,settings,keymap}.rs`, `keymap/{mod,global_action,key_sequence,key_bind}.rs`,
    `theme/blend.rs`, `toasts/`, `framework/mod.rs`, `pane/id.rs`.
  - `crates/cargo-tile/src/` — `app.rs`, `attract/{mod,moving_band,moving_text,pixelate,held_key}.rs`,
    `config.rs`, `constants.rs`, `globals.rs`, `interaction.rs`, `keymap.rs`,
    `render.rs`, `terminal.rs`. New files: `favorites.rs` (Phase 2),
    `favorites_overlay.rs` (Phase 4).
  - `crates/cargo-tile/tests/` **does not exist**. Every test in these phases is
    an inline `#[cfg(test)] mod tests` in the source file.
  - `docs/cargo-port/style/adding-a-keybinding.md` — the keybinding recipe Phase 4 follows.
- **Key files:**
  - `crates/tui_pane/src/backdrop/band.rs` — `BandDirection` (76), `BandFraying`
    (102), `TravelingBand` (243), `Default` (327), impl (353), public mutators
    396–520, private `set_width` (804).
  - `crates/tui_pane/src/backdrop/text.rs` — `TextDrift` (91), `TextFill` (191),
    `DriftingText` (226), impl (312), mutators 357–459, private `set_speed` (673).
    **No** private `set_spread`.
  - `crates/tui_pane/src/backdrop/pixels.rs` — `PixelResolve` (99), `PixelFill`
    (143), `ResolvingPixels` (294), impl (369), mutators 403–485, private
    `set_speed` (773), `set_block_columns` (779), `set_wave` (785).
  - `crates/tui_pane/src/backdrop/constants.rs` — every clamp, all `pub(super)`.
    There is **no `MIN_TEXT_SPREAD`**; text spread's floor is 0 by `saturating_sub`.
  - `crates/tui_pane/src/backdrop/random.rs` — `Xorshift` (23), `seeded` (44,
    `#[cfg(test)]`-gated), `random_glyph` (80). All `pub(super)`.
  - `crates/tui_pane/src/backdrop/mod.rs` — private `mod` decls (31–38), the nine
    `pub use` re-exports (40–54).
  - `crates/tui_pane/src/lib.rs` — `#[cfg(feature = "backdrop")] mod backdrop;`
    (9–10); each backdrop re-export separately cfg-attributed (35–56);
    `pub use theme::blend_color;` (258); `pub use toasts::ToastsRenderCtx;` (300).
  - `crates/tui_pane/src/layout/viewport.rs` — `ViewportOverflow` (14), `Viewport`
    (62), `keep_visible_scroll_offset` (301), `render_overflow_affordance` (315).
  - `crates/tui_pane/src/layout/column_widths.rs` — `ColumnSpec` (7), `ColumnWidths` (36).
  - `crates/tui_pane/src/overlays/keymap_ui.rs` — `prepare_overlay_inputs` (126),
    `ordered_help_rows` (161), `render_overlay` (173), private `columns_that_fit` (512).
  - `crates/tui_pane/src/overlays/settings.rs` — `defaults()` (136), `Esc => OverlayAction::Cancel` (139).
  - `crates/tui_pane/src/keymap/global_action.rs` — `defaults()` (61),
    `'q' => Quit` (64), `'R' => Restart` (**65**), `'x' => Dismiss` (69).
  - `crates/tui_pane/src/keymap/mod.rs` — `key_for_toml_key` (331), `keys_for_toml_key` (344).
  - `crates/tui_pane/src/keymap/key_sequence.rs` — `display_short` (70).
  - `crates/tui_pane/src/pane/id.rs` — `FrameworkOverlayId` (23–30), three variants, not `#[non_exhaustive]`.
  - `crates/tui_pane/src/framework/mod.rs` — `pub toasts: Toasts<Ctx>` (106).
  - `crates/tui_pane/src/toasts/lifecycle.rs` — `prune` (334), `prune_tracked_items` (320).
  - `crates/tui_pane/src/theme/blend.rs` — `blend_color(color, toward, alpha)` (35);
    alpha 0 leaves `color`, `u8::MAX` yields `toward` — the same scale as `fade(faded: u8)`.
  - `crates/cargo-tile/src/attract/mod.rs` — `AttractMode` (162, `pub(crate)`),
    `Attract` (176), `new` (227), `toggle` (253), `asked_for` (267), `keyed_mode`
    (288), mode switching (317–319, 346–348, 374–376), `showing` (410), `due_back`
    (421), `identify` (433), `advance` (519), `render` (625), `ground` (647).
    Fields `mode`/`band`/`text`/`pixels` are private with no accessors.
  - `crates/cargo-tile/src/attract/moving_band.rs` — `defaults()` (95): `>`/`.`
    Faster, `<`/`,` Slower, `[` TailSlower, `]` TailFaster, `+`/`=` Wider, `-`
    Thinner, `v` CycleFraying, `1`/`2`/`3` mode switch. `moving_text.rs` and
    `pixelate.rs` hold the other two `bindings!` blocks.
  - `crates/cargo-tile/src/globals.rs` — `action_enum!` block (**29–42**),
    `defaults()` (51), `dispatch` (69).
  - `crates/cargo-tile/src/keymap.rs` — `build_keymap` (64), scope registrations (75–83).
  - `crates/cargo-tile/src/terminal.rs` — `handle_key` (**448**);
    `if let Some(overlay) = app.framework.overlay()` (**451**);
    `|| matches!(action, GlobalAction::Dismiss))` (**456**);
    `dispatch_overlay_key` (505); `if app.updates == Updates::Frozen {` (**243**),
    its `else` (244), the attract frame request **inside** that else (290).
  - `crates/cargo-tile/src/render.rs` — `draw` (136);
    `match app.framework.overlay()` (**183**); its `_ => ()` arm (**187**).
  - `crates/cargo-tile/src/app.rs` — `APP_PANE_DISPLAY_ORDER` (30), `AppPaneId`
    (42), `Updates` (60), `App` (111), `pub(crate) framework: Framework<Self>`
    (113), `App::new` (150), `AppContext` impl (173), `type ToastAction = NoToastAction` (175).
  - `crates/cargo-tile/src/config.rs` — `load` (**146**), `restate` (181, private),
    `save` (**194**), `config_path` (215), `keymap_path` (220), `themes_dir` (225),
    `config_root` (**229**, private), `LoadedConfig { config, error }`.
  - `crates/cargo-tile/src/constants.rs` — `CONFIG_DIRNAME` (85), `CONFIG_FILENAME`
    (87), `KEYMAP_FILENAME` (107), `THEMES_DIRNAME` (110), `APP_GLOBALS_SECTION`
    (176), `KEYMAP_TOML_HEADER` (184).
  - `crates/cargo-tile/src/interaction.rs` — `Picked` (28), `handle_click` (45),
    `overlay_row` (54, exhaustive on `FrameworkOverlayId`), `HitTestRegistry` (67),
    `InputContext` (87), `app_modal_overlay_hit` returning `ModalHit::Closed` (94).
  - `crates/tui_pane/Cargo.toml` — version (12), `[features] backdrop` (21),
    `serde` (**31**), macOS-only deps (42).
  - `crates/cargo-tile/Cargo.toml` — version (12), `chrono` (18), `toml` (25),
    `tui_pane` with `features = ["backdrop"]` (29), `[dev-dependencies] tempfile` (31–32).
  - `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Keep a
    Changelog, `## [Unreleased]` → `### Added` / `### Changed` / `### Fixed`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check tui_pane` (Phase 1);
  `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile` (Phases 2–8).
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` (Phase 1);
  `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` (Phases 2–8).
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` (Phase 1);
  `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` (Phases 2–8).
- **Style:** `phase-end /clippy style-only auto-proceed`
- **Invariants:**
  - **tui_pane's keymap defaults are not touched.** User constraint, stated
    verbatim: *"not a tui-pane change - definitely not - i'm only talking about
    behavior in cargo-tile - we need to not change anything that can affect
    cargo-port"*. `'x' => Dismiss` at `keymap/global_action.rs:69` stays exactly
    as it is; cargo-port keeps `x` for its own dismiss fallback. Phase 4's
    `x`-no-longer-closes change is a cargo-tile dispatch-ladder edit only.
    Phase 1 is the one tui_pane phase and it adds API without changing behavior.
  - **Backdrop feature gating is two-hop.** `lib.rs:9-10` declares
    `mod backdrop` behind `#[cfg(feature = "backdrop")]`, and each of the nine
    re-exports at 35–56 carries **its own** `#[cfg(feature = "backdrop")]`
    attribute, alphabetically ordered. A new settings struct follows the same
    shape: `pub use band::BandSettings;` in `backdrop/mod.rs`, then a separately
    cfg-attributed `pub use backdrop::BandSettings;` in `lib.rs`, in alphabetical
    position. `backdrop` is not in `default`; cargo-tile opts in.
  - **Clamp constants stay `pub(super)`.** Everything in `backdrop/constants.rs`
    is visible only inside `backdrop/*`, which is why the settings structs and the
    randomizers live there and not in cargo-tile.
  - **macOS gating covers the capture path only.** `backdrop/constants.rs`
    (417, 426, 440, 448, 452, 464, 474), `desktop.rs:348`, `query.rs:84,87` and
    `lib.rs:87` carry `#[cfg(target_os = "macos")]`. `band.rs`, `text.rs`,
    `pixels.rs` and `random.rs` are platform-neutral and must stay so; check any
    new constant for dead-code warnings on the Linux target before adding a cfg.
  - **Lints.** `missing_docs = deny` — every new public item *and every public
    struct field* needs a doc comment. `unwrap_used` / `expect_used` / `panic` /
    `unreachable` / `unsafe_code` are denied outside tests; test modules opt back
    in with `#[expect(clippy::expect_used, reason = "tests should panic on
    unexpected values")]` (see `config.rs:231-235`, `keymap.rs:86-90`).
    `self_named_module_files = deny`.
  - **Every phase ends green:** `cargo build && cargo +nightly fmt`, clippy clean,
    tests passing, CHANGELOG entry added under `## [Unreleased]` (a sentence or
    two, not a paragraph). cargo-tile bumps its patch version once per phase;
    tui_pane is mid-cycle at `0.8.0-dev` and takes a CHANGELOG entry with no
    version edit.
  - **Scope boundaries the plan sets** (author's, not the user's): no mouse
    support inside the favorites overlay — keyboard first; if a click path is
    added later it goes through the existing
    `InputContext::app_modal_overlay_hit()` hook rather than a second hit-test
    ladder. No naming or editing of favorites — `saved` and the parameters are
    the whole row. No favorites in `config.toml` — a list that grows by keypress
    does not belong in a file the app rewrites to restate its defaults. No
    migration — the file does not exist yet. No change to what any animation
    draws: these phases add reading, writing and applying of parameters the keys
    already set.

## Phases

### Phase 1 — the snapshot API  · status: todo

#### Work Order

**Goal:** Each backdrop animation can report its steerable parameters, restore a
set of them as a semantic transition, and generate a random valid set.

**Spec:**

Three plain-data structs, one per animation, defined in that animation's own
module. Fields are **public** — cargo-tile builds these values from TOML in
Phase 2, and private fields would force an unplanned getter and constructor per
field. `apply` is the boundary that clamps, so public fields cost nothing. Every
field carries a doc comment (`missing_docs = deny`).

```rust
pub struct BandSettings  { pub direction: BandDirection, pub width: u32, pub speed: u32, pub tail_speed: u32, pub fraying: BandFraying }
pub struct TextSettings  { pub direction: BandDirection, pub speed: u32, pub spread: u32, pub drift: TextDrift, pub fill: TextFill }
pub struct PixelSettings { pub direction: BandDirection, pub speed: u32, pub wave_percent: u32, pub block_columns: u32, pub resolve: PixelResolve, pub fill: PixelFill }
```

All three animations already hold `direction: BandDirection`; that is the shared
direction enum, not a band-only one. Derive `Clone, Copy, Debug, Eq, PartialEq`
to match the animations.

Three methods per animation:

```rust
impl TravelingBand {
    pub fn settings(&self) -> BandSettings;
    pub fn apply(&mut self, settings: BandSettings);
    pub fn random_settings(&self, seed: u64) -> BandSettings;
}
// and the same three on DriftingText / ResolvingPixels
```

`random_settings` takes `&self`, not a bare seed. The band's real width limit is
its current line count — `MAX_BAND_WIDTH = 1000` is a pre-sizing sentinel, not a
runtime bound, and generating against the sentinel then clamping on apply would
collapse most seeds onto the same terminal-dependent maximum. Text and pixels do
not need the context, but one shape across all three keeps the caller uniform.

Randomization lives here rather than in cargo-tile because the ranges live here
(`backdrop/constants.rs`, all `pub(super)`), and a defect in a range gets fixed
where the range is. Reuse `Xorshift`: drop the `#[cfg(test)]` gate from
`Xorshift::seeded` (`backdrop/random.rs:44`) so non-test code can seed it; it
stays `pub(super)`.

**What a snapshot holds is only what a key steers.** Everything else is runtime
state that must not be saved or restored, because restoring it would put a strip
halfway across a window it was never sized to:

> `glyphs`, `tails`, `heads`, `phases`, `lanes`, `ripple`, `waved`, `grains`,
> `xorshift`, `faded`, `columns`, `rows`, `cell_pixels`, `leading_edge`,
> `middle`, `rolled_through`

A snapshot therefore *holds* none of those fields. `apply` is a different
matter: it is a **semantic transition, not field assignment**, and it updates
exactly the runtime state the equivalent keypress would. The existing mutators
maintain derived state deliberately — `TravelingBand::set_direction` rescales
width and resets `leading_edge` and `rolled_through`, `DriftingText::set_direction`
rebuilds `lines`, `cycle_drift` to `Together` resets each line's accumulated
drift, `ResolvingPixels::set_direction` transforms `middle`. Assigning past them
produces states unreachable by steering.

So `apply` must:

- Run in **dependency order**: direction first, then the enum transitions, then
  the numeric targets. Band width after direction; text spread after drift.
- Route **every numeric field through a private absolute setter** that clamps.
  A struct built from hand-edited TOML can carry a zero speed or a spread above
  100, and direct assignment would admit it.
- Reach absolute values only through those private setters, with the public
  `cycle_*` methods delegating to them, so one path maintains the invariants.

Private absolute setters that already exist: `TravelingBand::set_width` (804),
`DriftingText::set_speed` (673), `ResolvingPixels::{set_speed, set_block_columns,
set_wave}` (773, 779, 785). Ones to add:

- `TravelingBand::set_speed` / `set_tail_speed` — the clamps are currently
  inline in `speed_up` / `slow_down` / `tail_faster` / `tail_slower`; lift them.
- `TravelingBand::set_fraying` — `cycle_fraying` delegates to it.
- `DriftingText::set_spread` — clamp `.min(MAX_TEXT_SPREAD)` with a floor of 0.
  There is **no `MIN_TEXT_SPREAD` constant and none is added**: `spread_narrower`
  is a bare `saturating_sub` today, so 0 is already reachable by steering and the
  clamp must not exclude it.
- `DriftingText::set_drift` / `set_fill` — `cycle_drift` / `cycle_fill` delegate.
- `ResolvingPixels::set_resolve` / `set_fill` — `cycle_resolve` / `cycle_fill` delegate.

Direction setters are already public on all three and already early-return when
the direction is unchanged; `apply` uses them as they are.

Band width drawn by `random_settings` comes from the band's own axis extent —
the same `lines()` count `set_width` derives its maximum from — not from
`MAX_BAND_WIDTH`.

Export: add `pub use band::BandSettings;`, `pub use pixels::PixelSettings;`,
`pub use text::TextSettings;` to `backdrop/mod.rs` in alphabetical position among
the existing re-exports (40–54), then three separately cfg-attributed
`pub use backdrop::…;` lines in `lib.rs` among 35–56. Clamp constants stay
`pub(super)`.

tui_pane is mid-cycle at `0.8.0-dev`: add a CHANGELOG entry under
`## [Unreleased]` → `### Added`, no version edit.

**Files:**
- `crates/tui_pane/src/backdrop/band.rs` — `BandSettings`, `settings`, `apply`, `random_settings`, `set_speed`, `set_tail_speed`, `set_fraying`
- `crates/tui_pane/src/backdrop/text.rs` — `TextSettings`, `settings`, `apply`, `random_settings`, `set_spread`, `set_drift`, `set_fill`
- `crates/tui_pane/src/backdrop/pixels.rs` — `PixelSettings`, `settings`, `apply`, `random_settings`, `set_resolve`, `set_fill`
- `crates/tui_pane/src/backdrop/random.rs` — ungate `Xorshift::seeded`
- `crates/tui_pane/src/backdrop/mod.rs` — three re-exports
- `crates/tui_pane/src/lib.rs` — three cfg-attributed re-exports
- `crates/tui_pane/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** None — this is Phase 1.

**Acceptance gate:** `verify.sh check/test/lint tui_pane` all green, plus inline
`#[cfg(test)] mod tests` proving:

- A valid settings value taken from an animation and applied to a fresh one
  round-trips exactly through `settings()`.
- `apply` on an **already-running, already-sized** animation preserves the
  invariants: every direction change, every drift change and every fraying
  change, from a non-default starting state, leaves the same runtime state the
  equivalent keypress would.
- Arbitrary constructed values normalize rather than round-trip — `0` and
  `u32::MAX` on every numeric field land inside the clamps.
- `random_settings` is deterministic per seed and, over a fixed seed corpus,
  **every field varies and every enum variant is reachable**. A generator
  returning one constant valid value must fail this.
- Band width drawn for a sized band lands inside that band's own axis extent,
  not the `MAX_BAND_WIDTH` sentinel.

### Phase 2 — the favorites file  · status: todo

#### Work Order

**Goal:** cargo-tile can read and write `favorites.toml` without ever losing a
row it did not understand, and without two instances clobbering each other.

**Spec:**

`<os config dir>/cargo-tile/favorites.toml`, alongside `config.toml` and
`keymap.toml`, reached through a new `config::favorites_path()` next to the
existing `keymap_path()` (`config.rs:220`). It must live in `config.rs` because
`config_root()` (229) is private. `FAVORITES_FILENAME` joins `constants.rs`
beside `KEYMAP_FILENAME` (107).

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

One array of tables, mode-tagged, each holding only the keys its own mode has.
`saved` is RFC 3339 local time with fractional seconds; `chrono` is already a
cargo-tile dependency (`Cargo.toml:18`). `id` is a UUIDv7 minted once at save
and never changed — deletion, selection and the rendered-line map all address a
row by it, never by storage index, and it is what lets a mutation re-find its
row after re-reading the file. Add `uuid = { workspace = true }` to
`crates/cargo-tile/Cargo.toml`; the workspace already declares it with the
`serde` and `v7` features, but cargo-tile does not depend on it yet. v7 is
time-ordered, so id order matches save order.

**In memory the parsed `toml` tables are the model**, with a typed favorite
derived from each table for display and loading. This is the difference between
skipping a row and destroying it. A row whose `mode` is unknown, or whose enum
spelling does not parse, is **skipped for display** — the posture `keymap.toml`
already takes toward a stale entry — but it is still written back out on the next
save or delete. Serializing only the recognized rows would silently delete a
favorite written by a newer version, or one hand-edited with a typo. Unknown
*keys* on an otherwise-good row survive the same way.

The typed payload is **one enum, not a string plus optional fields**:

```rust
struct Favorite { id: FavoriteId, saved: DateTime<FixedOffset>, settings: FavoriteSettings }
enum FavoriteSettings { MovingBand(BandSettings), MovingText(TextSettings), Pixelate(PixelSettings) }
```

`FavoriteId` is a newtype over `uuid::Uuid`. `mode` is derived from the variant.
A `mode: String` alongside optional per-mode fields would let missing, mixed and
mismatched settings past parsing, and every later consumer — grouping, `m`,
`enter` — would re-derive a relationship the type already carries. The raw
`toml::Table` stays confined to parsing so unknown rows survive.

A file that does not exist is an empty list, not an error. The four outcomes are
**distinct states in one enum**, not all folded into "empty":

| State | `ctrl-o` | `ctrl-s` | `x` |
|---|---|---|---|
| `Missing` | empty notice | writes a new file | n/a |
| `Loaded` | the table | appends | deletes |
| Whole-file parse error | shows path + parse error | refused | refused |
| Read failure | shows path + error | refused | refused |

Refusing rather than overwriting matches what `config.toml` already does with a
file that failed to parse: `restate` (`config.rs:181`) is reached only on the
`Ok` arm of `toml::from_str`, so an unparseable config is never overwritten.
Reporting "nothing saved" over a file that exists but cannot be read would be a
lie, and letting `ctrl-s` replace a damaged file with one row loses everything in it.

**Every mutation is a locked read-modify-write ending in an atomic replace.**
Take a sibling lock file, re-read and re-parse under it, mutate the raw table
list by `id`, write a temporary file in the same directory, `sync_all`, and
rename over `favorites.toml`. Two running instances otherwise each hold a stale
snapshot and the later writer drops the earlier one's favorite; a direct
`fs::write` (what `config::save` at 194 does today) interrupted mid-way leaves a
truncated file. The cost is one lock and one reparse per keypress-driven
mutation, which is not a per-frame path.

The lock is `favorites.toml.lock`, acquired with
`OpenOptions::new().write(true).create_new(true)`; on `AlreadyExists`, retry
briefly, and treat a lock file whose mtime is older than five seconds as stale —
remove it and retry once. Release on both the success and the error path. This
is dependency-free on purpose: the workspace declares no file-locking crate, and
`unsafe_code = deny` rules out calling `flock` directly. *Author's call for a
two-instance edge case on a config file; swapping in a locking crate later is a
contained change to this one helper.*

Saving is **idempotent on `(mode, settings)`**: an identical parameter set
updates the existing row's `saved` rather than adding a second row. Repeated
`ctrl-s` otherwise clutters the table with indistinguishable rows and gives that
one parameter set extra weight in `m`'s uniform draw. Within a mode, rows are
ordered newest first.

The enum-to-string mapping is a `match` in cargo-tile, not a serde derive in
tui_pane. The app that writes the file owns the file's vocabulary, and an on-disk
spelling should not be pinned by a library that has no other reason to care.
(Note: "it keeps serde out of tui_pane" is **not** the reason — `tui_pane`
already depends on serde with `derive` unconditionally, `Cargo.toml:31`.) The
mapping's shape must fail loudly when a variant is added:

- `enum -> &'static str` is **exhaustive, with no wildcard arm**. A new tui_pane
  variant then breaks the build here rather than silently losing a spelling.
- `str -> Option<Enum>` stays tolerant, since it has to skip a stale file entry.
- **Seven** enums need the pair, not six: the six animation enums
  (`BandDirection`, `BandFraying`, `TextDrift`, `TextFill`, `PixelResolve`,
  `PixelFill`) **and** the app-owned `AttractMode` (`attract/mod.rs:162`,
  `pub(crate)`) for the `mode` tag.

Public surface for the module: `load`, `save`, `push`, `remove`. Keep everything
`pub(crate)`; nothing here has an outside consumer.

**Files:**
- `crates/cargo-tile/src/favorites.rs` — new file: raw-table model, typed `Favorite` / `FavoriteSettings` / `FavoriteId`, the load state enum, the seven enum mappings, `load` / `save` / `push` / `remove`, the lock + atomic replace helper
- `crates/cargo-tile/src/config.rs` — `favorites_path()` beside `keymap_path()` (220)
- `crates/cargo-tile/src/constants.rs` — `FAVORITES_FILENAME` beside `KEYMAP_FILENAME` (107), plus the lock and temp suffixes
- `crates/cargo-tile/src/main.rs` — declare `mod favorites;` in the `mod` block at 4–13
- `crates/cargo-tile/Cargo.toml` — add `uuid = { workspace = true }`; patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 1 exports `BandSettings`,
`TextSettings` and `PixelSettings` from `tui_pane` under the `backdrop` feature,
which `crates/cargo-tile/Cargo.toml:29` already enables. Their fields are public
plain data, so cargo-tile constructs them with full struct literals — use no
`..Default::default()`, so a new field is a compile error at this boundary. The
six animation enums were already re-exported from `tui_pane` before Phase 1.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus
inline tests (there is no `crates/cargo-tile/tests/` directory; `tempfile` is
already a dev-dependency) proving:

- A list survives save and load unchanged.
- An entry with an unknown mode or a misspelled enum is skipped for display
  **and is still present in the file after a save and after a delete**. Same for
  an unknown key on a recognized row.
- Truncated or otherwise unparseable TOML puts favorites in a read-only error
  state carrying the path; save and delete are refused, not silently applied to
  an empty list.
- A missing file loads as empty.
- Every variant of all seven enums round-trips; the `enum -> str` match has no
  wildcard arm.
- Saving an identical `(mode, settings)` twice leaves one row with the later
  timestamp.

### Phase 3 — `ctrl-s` and the toast path  · status: todo

#### Work Order

**Goal:** `ctrl-s` saves the running mode's parameters and says so on screen.

**Spec:**

`AppGlobalAction::SaveFavorite` bound to `ctrl-s`, added to the `action_enum!`
block at `globals.rs:29-42`, its default binding at `defaults()` (51), and its
arm in `dispatch` (69). The `AppGlobalAction` scope is already registered in
`keymap.rs:75-83`, so `keymap.rs` does not change.

`ctrl-s` is free in every scope and is not swallowed as XOFF: raw mode's
`cfmakeraw` clears `IXON`. Confirm live on iTerm2 before this phase ships.

`Attract` gains a method returning the current mode's settings as a
`FavoriteSettings` — the fields `mode`/`band`/`text`/`pixels` are private with no
accessors (`attract/mod.rs:176`), so this must live in that module. The
animations hold their parameters whether or not they are being drawn, so this
works with the attract screen fully hidden too, which is what makes `ctrl-s`
from a working grid save something real.

**The toast has no path to the screen yet.** `App` owns `framework.toasts`
transitively (`app.rs:113` → `framework/mod.rs:106`), but a full grep of
`crates/cargo-tile/src/` finds no `ToastsRenderCtx` use, no `Toasts::prune`
call, and no toast rendering at all — `render::draw` never renders the stack and
`event_loop` never prunes it. Pushing a toast today produces nothing. So:

- Render the toast stack in `render::draw` (136) with `ToastsRenderCtx`
  (`tui_pane::lib.rs:300`), beneath the modal overlays.
- Call `Toasts::prune` (`toasts/lifecycle.rs:334`) from `terminal::event_loop`
  **outside** the `Updates::Frozen` branch (`terminal.rs:243`), not inside its
  `else` where the attract frame request sits (290).
- Fold the toast entrance/expiry/exit deadlines into a **single visual deadline**
  the loop uses to ask for frames. Ask for frames during the entrance and exit
  only, with one wake at expiry — not a continuous repaint through the static
  timeout. Phase 5's deletion fade reuses this same deadline.

`App::ToastAction` is `NoToastAction` (`app.rs:175`) and stays that way; these
toasts are not interactive.

A toast confirms the save and reports the path on a write failure.

Persistence stays **synchronous on the dispatch path**, matching `config.rs`. A
reviewer proposed a persistence worker thread and reply channel; declined — the
write is a few KB behind a lock on a keypress path, and the thread is complexity
a later reader has to hold for no measured gain. The real defect underneath that
proposal, file I/O inside `render::draw`, is fixed by Phase 5's commit-point rule
instead.

**Files:**
- `crates/cargo-tile/src/globals.rs` — `SaveFavorite` variant, default binding, dispatch arm
- `crates/cargo-tile/src/attract/mod.rs` — a method returning the current mode's `FavoriteSettings`
- `crates/cargo-tile/src/render.rs` — render the toast stack with `ToastsRenderCtx`
- `crates/cargo-tile/src/terminal.rs` — prune toasts outside the `Frozen` branch; the shared visual deadline
- `crates/cargo-tile/src/favorites.rs` — call site for `push`
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 2 provides
`favorites::{load, save, push, remove}`, the `Favorite` / `FavoriteSettings` /
`FavoriteId` types, the four-state load enum, and `config::favorites_path()`.
`push` is idempotent on `(mode, settings)` and does its own locked
read-modify-write with an atomic replace, so the dispatch path calls it and
handles only its `Result`. Phase 1 provides `settings()` on each animation.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- Pressing the key with each of the three modes showing writes a row that reads
  back as that mode's current parameters.
- The same holds with the attract screen **fully hidden**.
- The success toast and the write-failure toast both appear on screen and expire.

### Phase 4 — the favorites overlay: modal shell and table  · status: todo

#### Work Order

**Goal:** `ctrl-o` opens a scrolling, mode-grouped table of saved favorites that
consumes every key while it is open.

**Spec:**

`AppGlobalAction::OpenFavorites` on `ctrl-o` (free in every scope), added to
`globals.rs` the same way Phase 3 added `SaveFavorite`.

**One controller owns everything.** `favorites_overlay.rs` holds a
`FavoritesOverlay` owning open state, the row list, selection, the `Viewport`,
the cached line plan, rendering, input handling, and `frame_owed()`. `App` owns
exactly one instance. Spreading state across `App`, drawing across two files,
input across `terminal.rs` and fade scheduling elsewhere is how the one-frame
repaint defect gets reintroduced — the demand-driven loop repaints nothing unless
something asks.

**The overlay is a complete modal, not a key-order tweak.**
`AppOverlay::{Favorites, NoFavorites}` with a registered `FavoritesOverlayAction`
scope and `AppPaneId::Favorites`, following
`docs/cargo-port/style/adding-a-keybinding.md`, so the footer labels follow
rebinding like every other surface. While an `AppOverlay` is open its scope is
dispatched and **every** key is consumed, unmatched ones as no-ops, ahead of the
framework overlay check at `terminal.rs:451`. Taking only the recognized keys
would leave `r` randomizing behind the popup and `?` opening a framework overlay
on top of it. At most one app or framework modal is open at a time.

`InputContext::app_modal_overlay_hit` (`interaction.rs:94`) returns
`ModalHit::Closed` unconditionally today; it must report the app modal as open so
a click does not fall through to the grid. Mouse selection inside the overlay
stays out of scope.

**Layout.** Modes hold disjoint parameters, so one flat table would be mostly
blanks. Group by mode: a heading per mode that has favorites, its own column
header, then its rows. Selection walks every row across every section as one
list, so scrolling behaves as a single list regardless of the grouping.

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

The key line under each header is read from the **live keymap** — via the scope
for `AppPaneId::Attract(mode)`, resolved with `Keymap::key_for_toml_key`
(`keymap/mod.rs:331`) and rendered with `KeySequence::display_short()`
(`key_sequence.rs:70`), **not** `KeyBind::display_short` — so a rebound key shows
through rather than a hardcoded label going stale.

That needs an explicit **per-column descriptor**, because the mapping is not
one-to-one. A displayed parameter usually covers a *pair* of actions with aliases
on each: band speed is `SpeedFaster` and `SpeedSlower`, bound to `>`/`.` and
`<`/`,`. The descriptor names the action or action pair per column, the policy is
primary-binding-per-action, and an unbound half renders as a blank rather than a
stale default. The sketch above is deliberately wrong as a warning: it puts Tail
on `</>` and Speed on `,/.`, while the real defaults (`attract/moving_band.rs:95`)
are Speed `</>` and Tail `[/]`. A descriptor plus a test against the resolved
keymap is what prevents exactly that class of error.

Timestamps display to the second, and carry the year when the row is not from the
current year. Minute precision cannot tell two saves of a similar parameter set
apart.

**Narrow terminals are their own problem.** `ColumnWidths`
(`layout/column_widths.rs:36`) only grows columns to observed content — it has no
notion of a total width budget — and the keymap overlay's private
`columns_that_fit` (`keymap_ui.rs:512`) reduces the number of side-by-side
*sections*. Neither makes a seven-column row fit a terminal narrower than the
row. So: `Saved` and the selection marker pin to the left edge; the parameter
columns page horizontally with left/right, one whole column at a time; the
header, key line and cells are all built from the same visible-column slice.
Chosen over clipping or dropping columns by priority — paging is the only one of
the three where you can still see the value you are about to load.

The empty case is a non-selectable line, not an empty table: `No favorites
saved -- press <live ctrl-s label> to save one`. A list with one mode renders
that mode's section only, with no others stubbed in.

**Row rendering is cached, not rebuilt per frame.** `keymap_ui.rs`'s
`prepare_overlay_inputs` (126) / `render_overlay` (173) build, format and measure
every row before applying the scroll offset; copied here, each of Phase 5's fade
frames would do O(total favorites) of string work to animate one row. Build the
grouped line plan and the formatted cells on open, on mutation, on keymap
replacement and on width change; a frame renders only the lines intersecting the
viewport. Measurement uses `ColumnWidths`; scrolling uses `Viewport`
(`layout/viewport.rs:62`). No count or file-size cap is imposed — a reviewer
proposed one and it is declined: with the cache in place the per-frame cost is
O(visible rows), and refusing a save is a worse experience than a slower open.

**Two ladder corrections land here.**

- Narrow `render.rs:187`'s `_ => ()` to `None => ()`. The match at 183 currently
  swallows a future `FrameworkOverlayId` variant along with `None`, so a new
  framework overlay would compile with no draw arm. `terminal.rs`'s
  `dispatch_overlay_key` (505) already matches every variant.
- Drop the `|| matches!(action, GlobalAction::Dismiss))` clause at
  **`terminal.rs:456`** (inside the `if let` beginning at 454). That clause lets
  any `Dismiss`-bound key through an open framework overlay, which is what makes
  `x` close every overlay in the app — and what would make a reflexive `x` over
  the favorites table destroy a saved row in Phase 5. Removing it leaves the
  condition as `if let Some(action) = … && !in_text_mode &&
  matches_open_overlay_toggle(action, overlay)`. `esc` already closes those
  overlays through each one's own cancel binding (`overlays/settings.rs:139`), so
  nothing is lost, and `s` / `ctrl-k` / `?` still toggle their own overlay shut
  through the surviving clause. **tui_pane's defaults are not touched** — see the
  Invariants; cargo-port keeps `x` for its own dismiss fallback. Left as is: with
  no overlay open, `x` still clears a visible toast, since that path does not run
  through the removed clause.

**Files:**
- `crates/cargo-tile/src/favorites_overlay.rs` — new file holding `FavoritesOverlay`: open state, rows, selection, `Viewport`, cached line plan, column descriptors, rendering, input, `frame_owed()`
- `crates/cargo-tile/src/app.rs` — `App` owns one `FavoritesOverlay`; `AppOverlay::{Favorites, NoFavorites}`; `AppPaneId::Favorites` (42) and `APP_PANE_DISPLAY_ORDER` (30)
- `crates/cargo-tile/src/globals.rs` — `OpenFavorites` variant, default binding, dispatch arm
- `crates/cargo-tile/src/keymap.rs` — register the `FavoritesOverlayAction` scope (75–83)
- `crates/cargo-tile/src/render.rs` — draw the overlay; narrow `_ => ()` (187) to `None => ()`
- `crates/cargo-tile/src/terminal.rs` — app-modal dispatch ahead of the framework check (451); drop the `Dismiss` clause (456)
- `crates/cargo-tile/src/interaction.rs` — `app_modal_overlay_hit` (94) reports the app modal as open
- `crates/cargo-tile/src/main.rs` — declare `mod favorites_overlay;` in the `mod` block at 4–13
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added` / `### Changed`

**Constraints from prior phases:** Phase 2 provides `favorites::load()` and its
four-state enum — the overlay renders the parse-error and read-failure states as
the path plus the error, not as an empty list. Rows are addressed by
`FavoriteId`, never by storage index, and arrive ordered newest first within a
mode. Phase 3 established the shared visual deadline in `terminal::event_loop`
outside the `Updates::Frozen` branch; `frame_owed()` folds into it. Phase 3 also
added the toast render path, so this phase can report a load failure.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- The table groups by mode with per-mode headers, and the live key labels match
  the resolved keymap column by column — including after a rebinding.
- Selection walks every row across sections as one list.
- A list too tall to fit scrolls and keeps the cursor visible.
- A table wider than the terminal pages its parameter columns with `Saved` pinned,
  and the header, key line and cells stay aligned across a page.
- An empty list shows the notice with the live `ctrl-s` label, and `esc` dismisses it.
- A key bound to an app or framework global action does nothing while the overlay
  is open.
- `x` over an open settings, keymap or global-shortcuts overlay leaves it open,
  while `esc` still closes it.

### Phase 5 — `enter` loads, `x` deletes with a fade  · status: todo

#### Work Order

**Goal:** The two mutating keys in the favorites table.

**Spec:**

**`enter` loads.** Set `Attract::mode` to the row's mode, call `apply` on that
animation with the row's settings, close the overlay, and ask for the attract
screen **unconditionally** through a new idempotent `Attract::request_show()`.
Not "if it is not already showing": `Attract::showing()` (`attract/mod.rs:410`)
only tests that the fade is off its maximum, so it stays true through a
fade-*out*, and a load landing in that window would skip the request and watch
the favorite it just loaded disappear. `toggle()` (253) is equally unsuitable,
since it can ask for the opposite state. The other two animations keep whatever
they were last steered to — that is what already makes `1` / `2` / `3` a turn
rather than a restart.

`Attract`'s `mode`/`band`/`text`/`pixels` fields are private with no accessors,
so both `Attract::apply_favorite()` and `Attract::request_show()` go in
`attract/mod.rs`.

**`x` deletes with a fade.** `x` marks the selected row
`Removing { since: Instant }` rather than dropping it. Alpha is computed from
`now - since` against a fixed fade duration, **not** incremented per draw —
otherwise an unrelated scan or keypress adds frames and the fade runs faster.
Use `blend_color(color, ground, alpha)` (`theme/blend.rs:35`, re-exported at
`lib.rs:258`); alpha 0 leaves the row at full strength and `u8::MAX` yields the
ground, the same scale the animations' `fade(faded: u8)` uses.

The row leaves the selection set the moment deletion starts and the cursor moves
to the next active row, but it keeps its rendered line until the fade ends. When
alpha reaches `u8::MAX` the row is dropped **by `id`**, the table is laid out
again without it, and the file is rewritten.

The overlay must report that it owes frames while a removal is in flight, the way
`Attract::showing` does, or the fade draws one frame and stops. This is the exact
defect recorded in the attract-mode attempts log; it is a requirement, not an
afterthought. Three details decide whether it is actually met:

- **Where it advances.** `FavoritesOverlay::advance(now)` runs from
  `terminal::event_loop` **outside** the `Updates::Frozen` branch
  (`terminal.rs:243`), on the shared visual deadline Phase 3 established. The
  attract screen's frame request sits *inside* that branch's `else` (290);
  copying its placement would freeze the deletion fade, and leaning on
  `Attract::showing` would only work when the attract screen happens to be up —
  a delete over a working grid would stop after its event-driven frame.
- **Where the commit happens.** `advance` returns whether a repaint or a final
  removal is owed. Mutation and file I/O stay out of `render::draw` (136);
  discovering `u8::MAX` mid-render and writing the file there puts a disk write
  inside a frame.
- **Closing mid-fade.** Deletion is committed at `x`, not at fade end. If the
  overlay closes while a row is fading, the row is removed and the file written
  immediately.

**Files:**
- `crates/cargo-tile/src/favorites_overlay.rs` — `enter` and `x` handling, `Removing { since }`, `advance(now)`, elapsed-time alpha, close-mid-fade commit
- `crates/cargo-tile/src/attract/mod.rs` — `Attract::apply_favorite()`, `Attract::request_show()`
- `crates/cargo-tile/src/terminal.rs` — call `advance` outside the `Frozen` branch on the shared deadline
- `crates/cargo-tile/src/favorites.rs` — `remove` call site
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 4 owns `FavoritesOverlay` with its
selection, `Viewport`, cached line plan and `frame_owed()`; this phase extends
that controller and adds no second owner. The cached line plan is rebuilt on
mutation, so a removal invalidates it. Phase 4 already dropped the `Dismiss`
clause at `terminal.rs:456`, so `x` closes nothing and is free to delete here.
Phase 2's `remove` addresses the row by `FavoriteId` and does its own locked
read-modify-write with an atomic replace. Phase 1's `apply` is an ordered
semantic transition through the private clamp setters. Phase 3 established the
shared visual deadline outside the `Updates::Frozen` branch.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- `enter` loads: the mode changes, the animation's `settings()` equals the row's
  settings, and the attract screen comes up **even when the load lands during a
  fade-out**.
- `x` fades the row and rewrites the file **with the attract screen fully hidden,
  with updates frozen, and with no other events arriving**.
- The fade's duration is driven by elapsed time: extra draws from unrelated
  events do not shorten it.
- Closing the overlay mid-fade still removes the row and writes the file.
- No file write happens inside `render::draw`.

### Phase 6 — `m`, a random saved favorite  · status: todo

#### Work Order

**Goal:** `m` picks a saved favorite at random and shows it.

**Spec:**

`AppGlobalAction::RandomFavorite` on `m`. `m` is free in every scope. `q` was
proposed first and cannot be used: it is the framework's `Quit`
(`keymap/global_action.rs:64`), so a mis-press would exit the app.

Picks uniformly from the saved list and loads it through the same path `enter`
uses. With an empty list, `AppOverlay::NoFavorites` — the empty state already
defined for `ctrl-o` in Phase 4, reused from the same controller rather than a
second notice overlay with its own owner — says so and `esc` dismisses it.

The key lives on `AppGlobalAction`, not on the three attract scopes. One place
instead of three near-copies, one section in the keymap overlay, and it works
from the grid as well: `m` over a working grid gives you a random favorite and
turns the attract screen on to show it. The ladder already suits this — attract
scope keys are offered first, and `m` collides with nothing they bind, so it
falls through to the app globals below.

**Files:**
- `crates/cargo-tile/src/globals.rs` — `RandomFavorite` variant, default binding, dispatch arm
- `crates/cargo-tile/src/favorites.rs` — the bounded uniform index draw
- `crates/cargo-tile/src/favorites_overlay.rs` — open `NoFavorites` on an empty list
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 5 provides `Attract::apply_favorite()`
and `Attract::request_show()`; this phase calls them rather than reaching into
`Attract`'s private fields. Phase 4 provides `AppOverlay::NoFavorites` and its
non-selectable empty notice, dispatched through the app-modal route ahead of the
framework check. Phase 2's load returns the four-state enum — a parse error or
read failure is reported, not treated as an empty list.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- Selection is proven through its **bounded index draw against a fixed seed
  corpus**, not by pressing the key until the row changes — a valid list can
  legitimately return the same row twice, so "repeated presses visibly move" is a
  flaky condition.
- An empty list opens the notice, which renders and consumes `esc` through the
  app overlay route ahead of framework handling.

### Phase 7 — `r`, randomize everything  · status: todo

#### Work Order

**Goal:** `r` draws a fresh configuration at random across every mode and
parameter, and starts showing the result.

**Spec:**

`AppGlobalAction::RandomizeAll` on `r`. Draws a mode at random, draws that mode's
settings at random via Phase 1's `random_settings` on the chosen animation,
applies both, and turns the attract screen on with `request_show()`.

`r` gets the bare letter because it is the bigger, less reversible action and the
one pressed repeatedly while exploring. It is unbound in every scope: the
framework binds capital `R` to `Restart` (`keymap/global_action.rs:65`), and no
attract scope binds either case, so `r` reaches the app globals through the
ladder untouched.

`ctrl-shift-r` was the original ask and cannot be delivered. A terminal sends the
same byte for `ctrl-r` and `ctrl-shift-r` (0x12) unless the Kitty keyboard
protocol is negotiated. cargo-port pushes those flags
(`crates/cargo-port/src/tui/terminal/run.rs:85`); **cargo-tile does not**.
Pushing them here would change key reporting for every binding in the app, and
would still degrade to nothing on a terminal that will not negotiate.

**Files:**
- `crates/cargo-tile/src/globals.rs` — `RandomizeAll` variant, default binding, dispatch arm
- `crates/cargo-tile/src/attract/mod.rs` — draw a mode, call `random_settings` on that animation, apply, `request_show()`
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 1 provides
`random_settings(&self, seed: u64)` on each of the three animations, seeded from
the now-ungated `Xorshift::seeded`, with band width drawn from the band's own
axis extent. Phase 5 provides `Attract::request_show()`, which is idempotent and
correct during a fade-out.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- Over a fixed seed corpus every mode and every enum variant is reached, and
  every value sits inside its clamps.
- The animation's `settings()` after the action equals the generated target —
  which is what proves the settings were applied and not merely drawn.

### Phase 8 — `u`, undo the last replacement  · status: todo

#### Work Order

**Goal:** One step back from any of the three actions that replace the current
parameters wholesale.

**Spec:**

`r`, `m` and `enter` in the table all replace the current mode's parameters
wholesale. The failure is the press *after* the good one — something appears that
you like and your hand has already pressed the key again. Saving first is the
intended workflow, but the moment you would need it is the moment you do not take
it.

Before any of the three replacing actions runs, capture the current mode, **all
three parameter sets**, and whether the attract screen was up.
`AppGlobalAction::UndoReplace` on `u` restores them. One step only.

It covers all three, not just the random draw: an undo that catches one but not
the others is worse than none, because you cannot predict which press it will
catch. The checkpoint is captured by whichever of the three replacing actions
runs, so this phase adds the capture at all three existing call sites.

`u` is unbound in every scope. Only `ctrl-u` is taken, by tui_pane's vim
half-page scroll, and cargo-tile sets no vim mode — so `h` `j` `k` `l` are free
too.

**Files:**
- `crates/cargo-tile/src/globals.rs` — `UndoReplace` variant, default binding, dispatch arm
- `crates/cargo-tile/src/attract/mod.rs` — the checkpoint type, its capture, and the restore
- `crates/cargo-tile/src/favorites_overlay.rs` — capture before `enter` loads
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** The three replacing call sites are Phase 5's
`enter` (in `favorites_overlay.rs`, via `Attract::apply_favorite()`), Phase 6's
`m` (in `globals.rs`), and Phase 7's `r` (in `attract/mod.rs`). Phase 1's
`settings()` on each animation is what the checkpoint stores, and `apply` is what
restores it — an ordered semantic transition, so restoring a checkpoint leaves
the same runtime state the equivalent keypress would. Phase 5's
`Attract::request_show()` restores visibility; there must be a matching way to
put the screen back down when the checkpoint says it was hidden.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- After each of the three replacing actions, `u` restores the mode, all three
  parameter sets, and the attract screen's visibility.
- A second `u` does not step back twice — one step only.
