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
- **Project started:** 2026-08-26T12:59:46-04:00
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
  - `crates/cargo-tile/src/attract/mod.rs` — `AttractMode` (192, `pub(crate)`),
    `Attract` (236), `new` (294), `toggle` (323), `asked_for` (337), `keyed_mode`
    (358), `favorite_settings` (371), `size_current_animation` (382),
    `record_terminal_resize` (401), `showing` (519), `due_back` (530), `identify`
    (542), `advance` (628), `render` (739), `ground` (761).
    Fields `mode`/`band`/`text`/`pixels` are private with no accessors.
    Phase 3 added the sizing state beside them: `LaidOutArea::{NeverLaidOut,
    LaidOut}`, `PendingTerminalResize::{NotReported, Reported}`,
    `LastSizedArea::{NeverSized, Sized}` and `AnimationSizing`, which records the
    last area applied to each of the three animations separately.
    `size_current_animation` sizes **only the animation the current mode selects**
    — it is not an all-mode boundary.
  - `crates/cargo-tile/src/attract/moving_band.rs` — `defaults()` (95): `>`/`.`
    Faster, `<`/`,` Slower, `[` TailSlower, `]` TailFaster, `+`/`=` Wider, `-`
    Thinner, `v` CycleFraying, `1`/`2`/`3` mode switch. `moving_text.rs` and
    `pixelate.rs` hold the other two `bindings!` blocks.
  - `crates/cargo-tile/src/globals.rs` — `action_enum!` block (**39–52**),
    `SaveFavorite` (51), `defaults()` (62), `dispatch` (81), `mode_label` (123),
    `favorite_refusal_message` (131). The last two are **private to this file**
    today; `favorite_refusal_message` hardcodes the word "save" and the literal
    `ctrl-s`, so any later phase that reports a different mutation has to
    generalise it rather than call it as it stands.
  - `crates/cargo-tile/src/keymap.rs` — `build_keymap` (64), scope registrations (75–83).
  - `crates/cargo-tile/src/terminal.rs` — `handle_key` (**705**), which now
    dispatches an open favorites overlay **first**, at **708–710**, and returns
    before the framework is consulted; the framework branch
    `if let Some(overlay) = app.framework.overlay()` (**712**), whose
    open-overlay toggle runs at **714–719** and carries **no**
    `GlobalAction::Dismiss` fallback — Phase 4 removed that clause;
    `dispatch_overlay_key` (**754**); `if app.updates == Updates::Frozen {`
    (**496**), its `else` (**498**), the attract frame request **inside** that
    else (**543**). Phase 3 put the toast prune and the shared visual frame
    request **after** the recv match and outside the `Frozen` branch, and the
    toast deadline shortens the loop's wait through `VisualDeadline::limit_wait`.
  - `crates/cargo-tile/src/render.rs` — `draw` (**137**); the toast stack drawn
    through `ToastsRenderCtx` at **183–190**, immediately **before** the overlay
    match, so toasts render *beneath* every overlay; the favorites overlay drawn
    at **191**; `match app.framework.overlay()` (**193**), which Phase 4 narrowed
    to an exhaustive match ending in `None => ()` (**197**) — there is no
    wildcard arm left to widen.
  - `crates/cargo-tile/src/app.rs` — `APP_PANE_DISPLAY_ORDER` (35), `AppPaneId`
    (47), `Updates` (65), `App` (116), `App::new` (157), `AppContext` impl (214),
    `type ToastAction = NoToastAction` (216). Phase 3 added the private
    `toast_visual_schedule` field (120) and three methods: `schedule_timed_toast`
    (181), `toast_visual_deadline` (200), `toast_visual_frame_request` (209).

**Timed toasts do not animate unless they are registered.** `cargo-tile`'s event
loop is demand-driven — it blocks until an event or the frame deadline and
repaints only when something marks the frame dirty — while `tui_pane` animates a
toast purely as a function of elapsed time and never asks for a frame. Pushing a
timed toast is therefore **not** sufficient: every `Toasts::push_timed` /
`push_timed_styled` call must be paired with

```rust
app.schedule_timed_toast(toast_id, pushed_at, visible_duration, body_text, min_interior_lines);
```

passing the **same** body text, visible duration and minimum interior lines the
toast itself was pushed with, and `pushed_at` sampled immediately beside the push.
An unregistered toast renders only on whatever unrelated event happens to wake the
loop next. `globals.rs`'s `save_favorite` is the worked example.
  - `crates/cargo-tile/src/config.rs` — `load` (**146**), `restate` (181, private),
    `save` (**194**), `config_path` (215), `keymap_path` (220), `themes_dir` (225),
    `config_root` (**229**, private), `LoadedConfig { config, error }`.
  - `crates/cargo-tile/src/constants.rs` — `CONFIG_DIRNAME` (85), `CONFIG_FILENAME`
    (87), `KEYMAP_FILENAME` (107), `THEMES_DIRNAME` (110), `APP_GLOBALS_SECTION`
    (176), `KEYMAP_TOML_HEADER` (184).
  - `crates/cargo-tile/src/interaction.rs` — `Picked` (28), `handle_click` (45),
    `overlay_row` (54, exhaustive on `FrameworkOverlayId`), `HitTestRegistry` (67),
    `InputContext` (87), `app_modal_overlay_hit` (**94–100**), which Phase 4 made
    return `ModalHit::MissedRow` while the favorites overlay is open and
    `ModalHit::Closed` otherwise.
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
- **Backdrop feature gate — required for any phase touching
  `crates/tui_pane/src/backdrop/`.** `backdrop` is **not** a default feature of
  `tui_pane` (`default = ["clipboard"]`), and `verify.sh` passes target flags
  only, never `--features`. So the scoped `tui_pane` lines above compile and test
  the crate *with none of that code in it* — they pass whether the backdrop
  module builds or not. Phase 1 shipped a `const fn` calling `Ord::min` and a
  green `verify.sh test tui_pane`; `cargo-tile` would not build. Run these as
  well, unsandboxed, and treat them as the real gate:
  `cargo nextest run -p tui_pane --features backdrop --no-fail-fast` and
  `cargo clippy -p tui_pane --features backdrop --all-targets`.
  Phases 2–8 are scoped to `cargo-tile`, which enables `tui_pane/backdrop`
  (`crates/cargo-tile/Cargo.toml:29`), so their listed lines already compile it —
  but they still do not *run* tui_pane's own backdrop tests.
- **Style:** `project-end /clippy style-only auto-proceed`. **No per-phase style
  review — user instruction.** The whole run gets one style pass, and it runs at
  the final gate: after the last phase, once workspace verification is green.
  Phases neither run it nor wait on it.
  - **It reviews the branch, not the working tree.** Every phase checkpoints a
    commit, so by the last phase `git diff` is empty and a working-tree review
    would report clean having read nothing. The reviewed range is
    `<project base>..` — every checkpoint this plan landed on
    `feat/cargo-tile-favorites` plus whatever is still uncommitted. The base is
    resolved once per run (the parent of the plan's first checkpoint, or HEAD
    before any) and held in the delegate session.
- **Invariants:**
  - **tui_pane's keymap defaults are not touched.** User constraint, stated
    verbatim: *"not a tui-pane change - definitely not - i'm only talking about
    behavior in cargo-tile - we need to not change anything that can affect
    cargo-port"*. `'x' => Dismiss` at `keymap/global_action.rs:70` stays exactly
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
    unexpected values")]` (see `config.rs:241-245`, `keymap.rs:86-90`).
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

### Phase 1 — the snapshot API  · status: done

#### As-built

Each backdrop animation reports its steerable parameters as a plain-data struct, restores one, and draws a fresh one at random. `BandSettings`, `TextSettings` and `PixelSettings` are public structs with public fields, each defined in its animation's module and re-exported from `tui_pane` under the `backdrop` feature; all three derive `Clone, Copy, Debug, Eq, PartialEq` and carry per-field doc comments.

```rust
pub struct BandSettings  { pub direction: BandDirection, pub width: u32, pub speed: u32, pub tail_speed: u32, pub fraying: BandFraying }
pub struct TextSettings  { pub direction: BandDirection, pub speed: u32, pub spread: u32, pub drift: TextDrift, pub fill: TextFill }
pub struct PixelSettings { pub direction: BandDirection, pub speed: u32, pub wave_percent: u32, pub block_columns: u32, pub resolve: PixelResolve, pub fill: PixelFill }
```

`TravelingBand`, `DriftingText` and `ResolvingPixels` each carry `pub const fn settings(&self) -> <T>Settings`, `pub fn apply(&mut self, settings: <T>Settings)` and `pub fn random_settings(&self, seed: u64) -> <T>Settings`. A snapshot holds only what a key steers — never runtime state (`glyphs`, `tails`, `heads`, `phases`, `lanes`, `ripple`, `waved`, `grains`, `xorshift`, `faded`, `columns`, `rows`, `cell_pixels`, `leading_edge`, `middle`, `rolled_through`), because restoring that would put a strip halfway across a window it was never sized to. `apply` is a semantic transition, not field assignment: it runs in dependency order (direction, then the enum transitions, then the numeric targets) and routes every field through the private absolute clamp setters, which the public `cycle_*` methods also delegate to, so one path maintains the invariants — and it can silently clamp an out-of-range value. `random_settings` takes `&self` so bounded draws use the animation's live extent instead of the pre-sizing sentinels. `TravelingBand::widest_permitted_width` is the single band-width ceiling shared by `set_width` and `random_settings`.

`Xorshift` remains `pub(super)` inside `tui_pane::backdrop`; `Xorshift::seeded` is no longer test-gated, and `Xorshift::u32_inclusive` exists for bounded draws. `random_settings` consumes a seed but produces none.

**Files:**
- `crates/tui_pane/src/backdrop/band.rs` — `BandSettings`; `settings`/`apply`/`random_settings`; private `set_speed`, `set_tail_speed`, `set_fraying`, `widest_permitted_width`
- `crates/tui_pane/src/backdrop/text.rs` — `TextSettings` and the same trio; private `set_spread` (const), `set_drift`, `set_fill`; `spread_wider` is `pub const fn`
- `crates/tui_pane/src/backdrop/pixels.rs` — `PixelSettings` and the same trio; private `set_resolve`, `set_fill`
- `crates/tui_pane/src/backdrop/random.rs` — ungated `Xorshift::seeded`, `Xorshift::u32_inclusive`
- `crates/tui_pane/src/backdrop/mod.rs`, `crates/tui_pane/src/lib.rs` — the three re-exports, cfg-attributed in `lib.rs`
- `crates/tui_pane/CHANGELOG.md` — one `## [Unreleased]` → `### Added` line

**Binds later work:** the three settings structs are the shape favorites are serialized from and deserialized into. Because `apply` clamps silently, any diagnostic for a hand-edited out-of-range value belongs to the load path, which reports whether a favorite applied exactly or with adjustments. `Xorshift` is not exported, so any consumer needing a seed owns its own seed source and bounded draw. An animation that has never been drawn is unsized, so save and randomize paths need a sizing boundary before the values they read or write match the next drawn frame.

**Gotchas:**
- `backdrop` is not a default feature of `tui_pane`, and `verify.sh` emits no `--features` flag, so a scoped `tui_pane` gate compiles and tests the crate without any of this code in it. The real gate is `cargo nextest run -p tui_pane --features backdrop --no-fail-fast` and `cargo clippy -p tui_pane --features backdrop --all-targets`, unsandboxed.
- `set_spread` must stay `const` because `pub const fn spread_narrower` calls it; that rules out `Ord::min`, which is not const-stable, so the clamp is written longhand.
- The band's width ceiling is a share of the lines on screen (`TravelingBand::widest_permitted_width`), and at zero lines it falls back to the whole-range maximum — an animation that has never been drawn is unsized, and values read or written before the first sizing are not the values the next drawn frame uses.

**Ruled out:**
- Exposing `Xorshift` from `tui_pane` — tui_pane surface changes beyond this phase are barred, and a consumer can own its own generator.
- A `MIN_TEXT_SPREAD` constant — `spread_narrower` is a bare `saturating_sub`, so 0 is already reachable by steering and the clamp must not exclude it.
- Rewriting a hand-edited row to its clamped value — destroys a value that becomes valid again on a taller terminal.
- Adding a random crate for two call sites — tui_pane is dependency-free here.

### Phase 2 — the favorites file  · status: done

#### As-built

`crates/cargo-tile/src/favorites.rs` persists attract-screen parameter sets to
`<os config dir>/cargo-tile/favorites.toml`. The model is the raw `toml::Table`
row list with typed values derived beside it, so an unknown mode, a misspelled
enum or an unknown key survives a save and a delete untouched — losslessness
falls out of the representation rather than out of special cases. `FavoriteRows`
exposes `iter()` over `FavoriteRowRecognition::{Recognized(Favorite),
Unrecognized(UnrecognizedFavoriteValue { key, spelling })}` and `recognized()`
over `&Favorite` alone, already grouped by mode and newest first within a mode.

Three `pub(crate)` entry points: `load() -> FavoritesFileState`,
`push(FavoriteSettings) -> Result<Favorite, FavoritesMutationError>`, and
`remove(FavoriteId) -> Result<(), FavoritesMutationError>`. Every mutation is a
locked read-modify-write ending in an atomic rename. `FavoritesFileState` has five
variants — `LocationUnavailable`, `Missing`, `Loaded`, `Unparseable`, `Unreadable`
— and so does `FavoritesMutationError`: `LocationUnavailable`, `Unparseable`,
`Unreadable`, `LockUnavailable`, `WriteFailed`. The error implements `Display` and
`Error`.

Mutual exclusion is a kernel advisory lock: `std::fs::File::{try_lock, unlock}`,
retried a bounded number of times before returning `LockUnavailable`, with a
`FavoritesLock` guard unlocking on drop. Each row carries a UUIDv7 `FavoriteId`
minted at save and a `saved` timestamp written with `SecondsFormat::Millis`.
`push` is idempotent on `(mode, settings)`: an identical row has its timestamp
updated rather than being duplicated.

`FavoritesLocation::from(Option<PathBuf>)` converts `config::favorites_path()`'s
external optional once at the module boundary, so no `Option<PathBuf>` reaches
any entry point.

**Files:**
- `crates/cargo-tile/src/favorites.rs` — the whole favorites file API: row model, recognition, load state, mutation errors, the lock, and the three entry points
- `crates/cargo-tile/src/config.rs` — `favorites_path()`, beside `config_path()` and `keymap_path()`
- `crates/cargo-tile/src/constants.rs` — the favorites filename, array key, lock and temp suffixes, and the lock retry count and delay
- `crates/cargo-tile/src/main.rs` — `mod favorites`, carrying a `dead_code` expectation until the module's last entry point has a caller

**Binds later work:** The three entry points above are the only way to reach the
file. `push` and `remove` do their own locking and atomic replace, so a caller
handles only the `Result`. Rows are addressed by `FavoriteId`, never by storage
index. `recognized()` already orders rows the way the overlay table renders them,
so nothing downstream sorts again; `iter()` is the one that also carries
unrecognized rows. `FavoriteSettings::mode() -> AttractMode` reads the mode off
the variant. `LockUnavailable` is an externally observable failure — a second
cargo-tile instance mid-save — with no counterpart in the plan's original state
table, so the save toast and the delete path each report it by name. The
`#[expect(dead_code)]` on `config::favorites_path()` must be deleted by the phase
that makes `push` reachable, and the one on `mod favorites` by the phase that
makes `remove` reachable; an expectation whose lint has stopped firing is itself
an error under `-D warnings`.

**Gotchas:** `favorites.toml.lock` is created on first mutation and **never
unlinked** — it persists between runs. Unlinking a locked file is what
reintroduces the race, because POSIX has no "unlink this path only if it is still
this inode", so a stat-then-`remove_file` sequence can never be made exclusive.
`#[expect(dead_code)]` on a `mod` item seeds rustc's dead-code root worklist,
which is why one module-level attribute keeps every item inside it live; placing
it above the whole `mod` block instead would suppress genuine dead code across
every unrelated module. Timestamps need millisecond precision: a `Favorite` built
in memory carries sub-second time, so second-granularity spelling makes a saved
entry unequal to the one reloaded.

**Ruled out:** A hand-rolled lock using `OpenOptions::create_new(true)` plus an
mtime staleness rule that removes an apparently abandoned lock — unfixably racy,
and the reason given for it (that `unsafe_code = deny` ruled out `flock`) was out
of date, since `File::try_lock` is safe `std` stable from Rust 1.89. Adding
dev/inode identity to that scheme — still unlinks by path. A `save()` entry point
that re-read and rewrote the file unchanged — a canonicalizing no-op no consumer
could reach; the rewrite survives only as a test helper. Rewriting a row on disk
to record a clamped value — the file keeps what the reader wrote.

### Phase 3 — `ctrl-s` and the toast path  · status: done

#### As-built

- `AppGlobalAction::SaveFavorite`, bound to `ctrl-s` in every scope, writes the current attract animation's parameters to the favorites file and reports the outcome as a toast. Persistence is synchronous on the dispatch path, matching `config.rs`.
- `favorite_refusal_message` is an exhaustive match over all five `FavoritesMutationError` variants yielding five distinct messages, `LockUnavailable` naming its own retry. Exhaustiveness is the enforcing mechanism: a new variant is a compile error, never a silent generic message. Every path that mutates favorites reports refusals through a shared formatter, or the causes stop being distinguishable.
- `Attract::size_current_animation` runs a sizing pass before any parameter read, tracked by `LaidOutArea`, `PendingTerminalResize`, `LastSizedArea` and `AnimationSizing`, so a save taken in the same input burst as a mode switch or a resize records the geometry the next frame draws.
- The event loop is demand-driven and `tui_pane` animates toasts purely on elapsed time without ever requesting a frame, so `App` owns a wake schedule: `ToastVisualSchedule` on `App`, `ToastVisualTimeline` / `ToastVisualPhase` / `ToastTimelineUpdate` in `terminal.rs`, surfaced to the loop as `VisualDeadline` (shortens the wait) and `VisualFrameRequest` (marks the frame dirty).
- `AncestryFoot` replaces an `Option<Color>` plus a bool in `draw_ancestry`; behavior-preserving.

**Files:**
- `crates/cargo-tile/src/globals.rs` — the action, the `ctrl-s` binding, `save_favorite`, `mode_label`, `favorite_refusal_message`
- `crates/cargo-tile/src/terminal.rs` — the toast timeline types, `toast_target_height`, the loop's deadline and frame-request wiring
- `crates/cargo-tile/src/app.rs` — the schedule field and its three methods
- `crates/cargo-tile/src/attract/mod.rs` — the sizing states and `size_current_animation`
- `crates/cargo-tile/src/render.rs` — the toast draw call and `AncestryFoot`

**Binds later work:**
- A timed toast pushed through `Toasts::push_timed` / `push_timed_styled` animates only if the push is paired with `App::schedule_timed_toast(toast_id, pushed_at, visible_duration, body_text, min_interior_lines)` carrying the same body, duration and minimum interior lines — the event loop is demand-driven and the framework never requests a frame.
- The toast stack is drawn immediately **before** the framework overlay match, so toasts render **beneath** every overlay; a message raised while an app modal is open is hidden by it.
- `Attract::size_current_animation` (`attract/mod.rs:382`) sizes **only** the animation the current mode selects — it is not an all-mode boundary.
- `App::toast_visual_deadline` shortens the event loop's wait and `App::toast_visual_frame_request` marks the frame dirty; both sit outside the `Updates::Frozen` branch and are the mechanism any later time-driven visual reuses.
- `favorite_refusal_message` (`globals.rs:131`) is private to that file and hardcodes the word "save" and a literal `ctrl-s`.

**Gotchas:**
- `tui_pane`'s entrance height is clamped at both ends — `current_visible_lines` is `(elapsed / entrance_line_ms + 1)`, clamped up to `min_height` and down to `target_height` — so the rendered height stops changing at `(target_height - 1)` line-steps, not `(target_height - min_height)`. A deadline derived from the naive difference ends early and a two-line message loses its second line. Exit mirrors it: `hidden = elapsed / exit_line_ms`, gone at `target_height` steps.
- `Toasts` cannot be asked whether a toast id is still live, so deadlines are computed arithmetically; `toast_target_height` in `terminal.rs` reproduces `wrapped_line_count`'s plain character wrap (not a word wrap) and will drift if the framework's wrap changes.
- `push_timed_styled` samples its own `Instant::now()`, so a caller-sampled `pushed_at` is fractionally early; the timeline carries a one-line-step slack constant instead of assuming the two clocks agree.
- Pushing a timed toast without registering it produces a toast that never animates.
- The entrance leg requests 8ms frames across the leading interval where the clamp still holds the height at `min_height`; for a single-line toast (`target_height == min_height == 3`) the whole 450ms leg is redundant.

**Ruled out:**
- A persistence worker thread and reply channel — a few-KB locked write on a keypress path does not pay for the concurrency a later reader must hold.
- Adding a next-transition accessor to `tui_pane` — it is shared with cargo-port and stays unchanged.
- Special-casing `target_height == min_height` for the redundant entrance leg — the general fix is an entrance *start* at `pushed_at + (min_height - 1) * entrance_line_ms`, which also serves multi-line toasts.
- Restoring mid-fade progress on undo — a fade is a transient, and replaying a partial one reproduces a glitch rather than a state.
- Raising deletion refusals as toasts — toasts render beneath overlays and the favorites modal stays open on a refused delete.

### Phase 4 — the favorites overlay: modal shell and table  · status: done

#### As-built

- `ctrl-o` opens `FavoritesOverlay` (`favorites_overlay.rs`), the sole controller
  for the modal: open state, content, selection, viewport, cached line plan,
  rendering, and key handling. `App` holds exactly one instance; the surface has
  no time-driven state and requests no frames.
- `AppOverlay::{Closed, Favorites(FavoritesOverlayContent)}` makes closed a named
  variant, not an absent `Option`. `FavoritesOverlayContent` has six variants —
  `Rows`, `NoneSaved`, `OnlyUnrecognized`, `LocationUnavailable`, `Unparseable`,
  `Unreadable` — built by `from_file_state` as an exhaustive match with no
  wildcard arm.
- App-modal dispatch runs ahead of the framework overlay check in
  `terminal.rs::handle_key` (708–710); the framework's global `x => Dismiss`
  fallback is removed, so a framework overlay now closes only through its own
  toggle or cancel binding. `render.rs`'s overlay match is exhaustive, ending
  `None => ()`. `interaction.rs::app_modal_overlay_hit` reports the modal as open
  so clicks never reach the grid underneath.
- `FavoritesOverlayAction` scope: `SelectPrevious`/`SelectNext` (`up`/`k`,
  `down`/`j`), `PageColumnsLeft`/`PageColumnsRight` (`left`/`h`, `right`/`l`),
  `Close` (`esc`). Every other key is consumed as a no-op while the modal is
  open.
- Each mode's column key line, the footer labels, and the empty-notice's
  close/save labels resolve through one conversion, `ResolvedBinding::{
  Bound(KeySequence), Unbound}`, fed by a column-descriptor table enumerating
  every column of all three attract modes explicitly (see Gotchas).
- Horizontal paging is bounded by `last_horizontal_column_page`, a measured value
  cached on the line plan (max across present mode sections), not a
  column-count constant; the footer advertises paging only when it is nonzero.
  `visible_parameter_columns` budgets one `COLUMN_GAP` per rendered column and
  always keeps the first parameter column visible, even in a window too narrow
  for it.
- Column widths and padding are measured in terminal display cells via
  `unicode-width` (`measured_saved_width`, `push_display_padded`); no
  `chars().count()` or `{:<}`/`{:>}` width formatting remains on this surface.
- Timestamps render to the second, with the year appended when the row predates
  the current year.

**Files:**
- `crates/cargo-tile/src/favorites_overlay.rs` — the whole modal: content
  states, the two-index line plan, viewport scrolling, column budgeting/paging,
  display-cell measurement, rendering, key handling, and its tests.
- `crates/cargo-tile/src/favorites.rs` — unchanged parsing/recognition;
  `FavoriteRows::iter()` is what the overlay reads.
- `crates/cargo-tile/src/terminal.rs` — app-modal dispatch ahead of the
  framework check; the `Dismiss` fallback removed.
- `crates/cargo-tile/src/render.rs` — draws the overlay above the toast stack;
  exhaustive framework-overlay match.
- `crates/cargo-tile/src/interaction.rs` — click absorption while the modal is
  open.
- `crates/cargo-tile/src/globals.rs`, `keymap.rs`, `app.rs`, `main.rs` — the
  `ctrl-o` binding, the `Favorites` app pane, and the module declaration.
- `crates/cargo-tile/Cargo.toml`, `Cargo.lock` — `unicode-width` added.

**Binds later work:** The line plan carries two indices: `selectable_line_index`
holds recognized favorite rows only, `navigation_line_index` holds those plus
every line after the last recognized row so the cursor can scroll into the
unrecognized-diagnostics block. `SelectedFavorite::{NoFavoriteSelected,
Selected(FavoriteId)}` is derived from `navigation_line_index`;
`NoFavoriteSelected` is a reachable on-screen cursor position, not an error.
`CachedOverlayLine::{Static(Line), Favorite { id, tail }}` applies selection
styling at draw time from the cached `tail`, so a per-frame restyle needs no
plan rebuild; `rebuild_line_plan` itself runs only on open, on surface-width
change, and on horizontal-page change. `close()` resets the cached plan to
default and calls `Viewport::clear_surface`, so anything that must be committed
on close has to run before that reset — "`enter` loads, `x` deletes with a
fade" depends on this ordering. `FavoritesOverlayContent::from_file_state` is
the exhaustive load mapping: a loaded-but-empty file reaches `NoneSaved`, a
loaded file with no recognized rows reaches `OnlyUnrecognized`;
`open_file_state` is the single entry point that applies it — "`m`, a random
saved favorite" reuses it. `unicode-width` is now a cargo-tile dependency, and
every label added to this surface must be measured with it. The footer is
built from live bindings (`FavoritesSurfaceBindings::footer`), not hardcoded
labels.

**Gotchas:**
- The three attract modes do not share column action names: Pixelate binds
  `sweep_left/up/down/right` while Moving Band and Moving Text bind `travel_*`;
  Speed is `<`/`>` and Tail is `[`/`]`. A descriptor keyed on the wrong names
  renders a table that looks right but shows the wrong keys.
- `FavoriteRows::iter()` is already grouped by attract mode then newest-first
  (`compare_recognitions`: mode order, then `saved` descending, then id, every
  `Unrecognized` after every `Recognized`) — no second sort belongs in the
  overlay.
- `Viewport::set_len` clamps `pos` and `clear_surface` resets it to 0, so
  closing and reopening with fewer rows needs no extra bookkeeping.

**Ruled out:**
- One selection index for both selection and scrolling — the diagnostics block
  must be reachable by the cursor but can never be selected.
- Bounding horizontal paging by a column-count constant — the real limit depends
  on measured widths and differs per mode section.
- Measuring column text with `chars().count()` — a wide character then
  misaligns every column to its right.
- Dropping the first parameter column in a window too narrow for it — a clipped
  first column beats showing the date alone.
- Rewriting the removed global `Dismiss` clause into an exception rather than
  deleting it — the exception would have re-entered on the next overlay.

### Phase 5 — `enter` loads, `x` deletes with a fade  · status: todo

#### Work Order

**Goal:** The two mutating keys in the favorites table.

**Spec:**

**`enter` loads.** Set `Attract::mode` to the row's mode, call `apply` on that
animation with the row's settings, close the overlay, and ask for the attract
screen **unconditionally** through a new idempotent `Attract::request_show()`.
Not "if it is not already showing": `Attract::showing()` (`attract/mod.rs:519`)
only tests that the fade is off its maximum, so it stays true through a
fade-*out*, and a load landing in that window would skip the request and watch
the favorite it just loaded disappear. `toggle()` (323) is equally unsuitable,
since it can ask for the opposite state. The other two animations keep whatever
they were last steered to — that is what already makes `1` / `2` / `3` a turn
rather than a restart.

`Attract`'s `mode`/`band`/`text`/`pixels` fields are private with no accessors,
so both `Attract::apply_favorite()` and `Attract::request_show()` go in
`attract/mod.rs`.

**A loaded favorite can be quietly corrected, and it must not be.** `apply` runs
the private clamp setters, so a hand-edited row holding an out-of-range value —
or one saved on a much taller terminal — lands as a different value than the file
states. Nothing currently tells the reader that happened: they typed a number and
the tool shows another one. So `apply_favorite` sizes the animation through Phase
3's zero-duration boundary first, then reports what it actually did:

```rust
enum FavoriteApplicationOutcome {
    AppliedExactly,
    AppliedWithAdjustments { requested: FavoriteSettings, effective: FavoriteSettings },
}
```

On `AppliedWithAdjustments` a warning names the fields that moved and what they
became. **Which surface carries it depends on whether the overlay is still open** —
see *Where a message goes* below; `enter` closes the overlay first, so its warning
is a scheduled toast. **The row on disk is not rewritten.** The file keeps what the reader wrote, and the correction is
reported rather than committed — rewriting it would destroy a value that becomes
valid again on a taller terminal.

Phase 6's `m` and Phase 8's `u` load through this same call and report the same
way.

**`x` deletes with a fade.** `x` marks the selected row
`FavoriteRowLifecycle::Removing { since: Instant }` rather than dropping it; the
other variant is `Active`, and the enum is the row's state rather than a flag
beside it. Alpha is computed from
`now - since` against a fixed fade duration, **not** incremented per draw —
otherwise an unrelated scan or keypress adds frames and the fade runs faster.
Use `blend_color(color, ground, alpha)` (`theme/blend.rs:35`, re-exported at
`lib.rs:264`); alpha 0 leaves the row at full strength and `u8::MAX` yields the
ground, the same scale the animations' `fade(faded: u8)` uses.

The row leaves the selection set the moment deletion starts and the cursor moves
to the next active row, but it keeps its rendered line until the fade ends. When
alpha reaches `u8::MAX` the file is rewritten by calling `remove` **with the row's
`FavoriteId`**, and only a successful `remove` drops the row and lays the table
out again without it.

**Dropping a row can change which content variant the overlay is in.**
`FavoritesOverlayContent::Rows` guarantees at least one recognized row, so
deleting the last one must not leave `Rows` holding an empty table. After every
successful removal, normalize the content: to `OnlyUnrecognized` when
unrecognized diagnostic rows remain, and to `NoneSaved` when none do. Several
rows may be `Removing` at once, because the cursor advances off each one as its
fade starts and the next `x` can land before the first fade ends; every matured
removal is committed exactly once, and `close()` commits every in-flight row
before it resets the controller.

**A refused deletion puts the row back.** `remove` returns
`Result<(), FavoritesMutationError>` and can fail five different ways — another
instance holds the lock, the file became unparseable or unreadable, the config
directory stopped resolving, or the write failed. The row must not disappear on
any of them, because an overlay that drops a row the file still holds tells the
reader a deletion happened that did not, and the next open contradicts it. So the
order is: fade, then `remove`, then drop. On `Err`, return the row to
`FavoriteRowLifecycle::Active`, restore it to the selection set and to the cached
line plan, and raise an error toast naming the cause from the value's `Display`.
`LockUnavailable` says favorites are in use and the key can be pressed again.

**Where a message goes, and why it is not always a toast.** Phase 3 draws the
toast stack *beneath* every overlay. A deletion refusal happens while the
favorites overlay is still open — that is the whole point, the row has to stay
visible — so a refusal toast would render underneath the modal and the reader
would see the row reappear with no explanation. The rule for this phase:

- **Overlay open → the overlay says it.** Add a notice line owned by
  `FavoritesOverlay`, rendered inside the modal, naming the cause. That covers
  every `x` refusal and any adjustment warning raised before the overlay closes.
  The notice is a state on the controller, not a string that may or may not be
  there:

  ```rust
  enum FavoritesOverlayNotice {
      NoNotice,
      DeletionRefused { message: String },
      FavoriteAdjusted { message: String },
  }
  ```

  It is cleared on open, on close, and at the start of every new mutation
  attempt, and it is removed once that attempt succeeds — so a refusal stays
  readable until the reader does something about it, and a successful retry is
  what makes it disappear.
- **Overlay already closed → a scheduled toast.** The two cases are `enter`'s
  adjustment warning (the overlay closes as part of the load) and a
  close-mid-fade commit that fails after the modal is gone. Both use Phase 3's
  toast path, and **both must pair the push with `App::schedule_timed_toast`** —
  see the Delegation Context; a pushed-but-unregistered toast does not animate on
  this loop.

**The refusal formatter has to be generalised before it can be reused.**
`favorite_refusal_message` (`globals.rs:138`) is private to that file and its text
hardcodes the word "save" and a literal `ctrl-s`, so it cannot report a deletion
as it stands. Move it somewhere both call sites reach — `favorites.rs` alongside
`FavoritesMutationError` is the natural home — and parameterise it by two things:
a named mutation operation (`FavoritesMutation::{Save, Delete}`, so the message
says which one was refused) and a retry instruction.

**The retry instruction is not always a single key, so it cannot be a single
binding.** A refused save is retried by pressing `ctrl-s` again, and a refused
delete is retried by pressing `x` again — but only while the overlay is still
open. A close-mid-fade commit that fails reports after the modal is gone, and at
that point `x` is not routed to `Delete` at all: the reader has to reopen with
`ctrl-o`, select the row, and then press `x`. Naming a live binding there would
name a key that does nothing. So the formatter takes

```rust
enum FavoritesRetryInstruction {
    Press(ResolvedBinding),
    ReopenThenPress { open: ResolvedBinding, retry: ResolvedBinding },
}
```

and `LockUnavailable` describes whichever of the two the reader actually has. Keep the match exhaustive over all five `FavoritesMutationError`
variants with no wildcard arm — that exhaustiveness is what makes a new variant a
compile error instead of a generic message — and keep the existing
"every refusal names a distinct cause" test passing across both operations.

**The overlay's frame request is introduced here**, because this is the first
thing in it that moves without an event behind it. Phase 4 deliberately added
none. The overlay must report that it owes frames while a removal is in flight, or
the fade draws one frame and stops — the exact defect recorded in the attract-mode
attempts log. It is a requirement, not an afterthought.

`advance(now)` returns a named outcome rather than a bool, because there are three
distinguishable answers and the third one has a side effect the caller must run:

```rust
enum FavoritesOverlayFrameOutcome {
    Quiet,                              // nothing is fading; no frame owed
    Repaint,                            // a fade is mid-flight; draw again
    CommitRemoval(FavoriteId),          // the fade finished; the caller writes the file
}
```

`CommitRemoval` is what keeps the disk write out of the render path. The event
loop composes this with Phase 3's toast request — both live outside the
`Updates::Frozen` branch, and either one asking for a frame marks it dirty — and
the overlay's deadline joins `VisualDeadline` the same way the toast schedule's
does, so a fade shortens the loop's wait.

Three details decide whether it is actually met:

- **Where it advances.** `FavoritesOverlay::advance(now)` runs from
  `terminal::event_loop` **outside** the `Updates::Frozen` branch
  (`terminal.rs:496`), on the shared visual deadline Phase 3 established. The
  attract screen's frame request sits *inside* that branch's `else` (543);
  copying its placement would freeze the deletion fade, and leaning on
  `Attract::showing` would only work when the attract screen happens to be up —
  a delete over a working grid would stop after its event-driven frame.
- **Where the commit happens.** `advance` returns whether a repaint or a final
  removal is owed. Mutation and file I/O stay out of `render::draw` (136);
  discovering `u8::MAX` mid-render and writing the file there puts a disk write
  inside a frame. `render::draw` is at `render.rs:137`.
- **Closing mid-fade.** Deletion is committed at `x`, not at fade end. If the
  overlay closes while a row is fading, the row is removed and the file written
  immediately.

**Files:**
- `crates/cargo-tile/src/favorites_overlay.rs` — `Load` and `Delete` added to `FavoritesOverlayAction` with their footer labels, `enter` and `x` handling, the in-overlay refusal notice, `FavoriteRowLifecycle::{Active, Removing { since }}`, `advance(now) -> FavoritesOverlayFrameOutcome`, elapsed-time alpha, close-mid-fade commit
- `crates/cargo-tile/src/favorites.rs` — generalise the refusal formatter: move it here, parameterise by `FavoritesMutation::{Save, Delete}` and `FavoritesRetryInstruction`, keep the match exhaustive; delete the three now-obsolete `#[expect(dead_code, reason = "deleting a favorite starts in the next phase")]` attributes — on `FavoriteRows::remove` (238), the public `remove` (386), and `remove_from_location` (469)
- `crates/cargo-tile/src/globals.rs` — `save_favorite` calls the moved formatter instead of the local one
- `crates/cargo-tile/src/attract/mod.rs` — `Attract::apply_favorite()` returning `FavoriteApplicationOutcome`, `Attract::request_show()`
- `crates/cargo-tile/src/terminal.rs` — call `advance` outside the `Frozen` branch, composing its outcome with Phase 3's toast frame request; fold the overlay's fade deadline into `VisualDeadline`
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 4 owns `FavoritesOverlay` with its
selection, `Viewport`, cached line plan and `FavoritesOverlayContent`; this phase
extends that controller and adds no second owner, and adds the overlay's first
frame request. Phase 4's `FavoritesOverlayAction` already carries movement, paging
and close; `Load` and `Delete` join that same enum here, with TOML names `load`
and `delete` and defaults `enter` and `x`. The cached line plan is rebuilt on
mutation, so a removal invalidates it. Phase 4 already dropped the `Dismiss`
clause at `terminal.rs:716`, so `x` closes nothing and is free to delete here.
Phase 2's `remove` addresses the row by `FavoriteId` and does its own locked
read-modify-write with an atomic replace; it returns
`Result<(), FavoritesMutationError>` and every one of that enum's five variants is
a refusal this phase has to survive.

**What Phase 4 actually shipped, that this phase builds on.** The overlay keeps
**two** line indices, not one. `selectable_line_index` holds recognized favorite
rows only; `navigation_line_index` holds those plus every line after the last
recognized row, so the cursor can scroll into the unrecognized-diagnostics block.
The cursor's identity is
`SelectedFavorite::{NoFavoriteSelected, Selected(FavoriteId)}`, derived from
`navigation_line_index`, so **`NoFavoriteSelected` is a reachable position with a
live cursor on screen** — `enter` and `x` must both no-op there rather than
assuming a row.

**The typed settings `enter` needs are not on the row yet.** `FavoriteRowView`
(`favorites_overlay.rs:220`) keeps only `id`, a formatted `saved` timestamp and
formatted display `cells`; the `FavoriteSettings` they were rendered from is
dropped in `from_favorite`. So this phase gives `FavoriteRowView` a
`settings: FavoriteSettings` field and widens the cursor to
`SelectedFavorite::{NoFavoriteSelected, Selected { id, settings }}`. `enter` and
`x` match that one semantic state; `enter` applies the retained typed settings
and never reconstructs a value by parsing a display string, and no `Option`
appears in either the row or the cursor.

A line is `CachedOverlayLine::{Static(Line), Favorite { id, tail }}`
and `rendered_line` applies selection styling at draw time from the cached `tail`
string, which is what lets a per-frame fade restyle a row **without** rebuilding
the plan. `rebuild_line_plan` runs only on open, on surface-width change, and on
horizontal page change; `render` takes `&mut self` and rebuilds when the width
moved, so `advance(now)` stays a separate call. `close()` resets the cached plan
to default and calls `Viewport::clear_surface`, so a close-mid-fade commit has to
run before that reset. `crates/cargo-tile/Cargo.toml` already depends on
`unicode-width`; measure any new label for this surface in display cells, never
`chars().count()`. `FavoritesSurfaceBindings::footer` builds the footer from live
bindings and already varies on whether paging is available, so `Load` and
`Delete` labels join it there.

**`mod favorites` in `main.rs` already carries no `#[expect(dead_code, …)]`** —
Phase 4 removed it when `load` went live, so this phase touches `main.rs` not at
all. Phase 1's `apply` is an ordered
semantic transition through the private clamp setters. Phase 3 established the
shared visual deadline outside the `Updates::Frozen` branch, cargo-tile's
terminal-area state updated ahead of `Attract::advance`'s early return, the
zero-duration sizing boundary this phase applies before `apply`, and the toast
render and prune path. Two Phase 3 facts bind this phase specifically: toasts
render **beneath** overlays, so a message raised while the modal is open must be
an overlay notice rather than a toast; and a timed toast animates only if its push
is paired with `App::schedule_timed_toast` carrying the same body, duration and
minimum interior lines.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- `enter` loads: the mode changes, the animation's `settings()` equals the row's
  settings, and the attract screen comes up **even when the load lands during a
  fade-out**.
- `enter` applies the settings the row retained, proven by loading a favorite
  whose display string is lossy for at least one field and asserting the applied
  value matches the file rather than the rendered cell.
- With the cursor parked on the unrecognized-diagnostics block, both `enter` and
  `x` do nothing: no mode change, no file write, no notice.
- Deleting the last recognized row leaves the overlay in `NoneSaved` when no
  unrecognized rows remain, and in `OnlyUnrecognized` when they do — never in
  `Rows` with an empty table.
- Two `x` presses in quick succession, the second landing before the first fade
  ends, remove both rows and write each exactly once.
- `x` fades the row and rewrites the file **with the attract screen fully hidden,
  with updates frozen, and with no other events arriving**.
- The fade's duration is driven by elapsed time: extra draws from unrelated
  events do not shorten it.
- Closing the overlay mid-fade still removes the row and writes the file.
- A deletion the file refuses leaves the row on screen: with a second process
  holding the favorites lock, `x` fades the row, `remove` returns
  `LockUnavailable`, the row returns to `Active` and to the selection set, the
  cause is named **inside the open overlay** rather than in a toast, and reopening
  the overlay still shows the row.
- The same holds for a `WriteFailed` refusal.
- The refusal text names the operation: a refused deletion does not say "save",
  and `LockUnavailable` names a usable retry rather than a hardcoded `ctrl-s`.
  All five error variants still produce distinct messages, for both operations.
- The three refusal surfaces are tested separately, because they do not name the
  same keys: a refused save says press the save key again; a refused delete with
  the overlay open says press the delete key again; and a refused close-mid-fade
  commit, reported after the modal is gone, says to reopen and then press it.
- A refusal notice stays readable until the reader acts on it, and a successful
  retry clears it.
- Every timed toast this phase raises is registered with the visual schedule: with
  updates frozen and no input arriving, `enter`'s adjustment warning opens, holds
  and expires on its own.
- `advance` returns `CommitRemoval` exactly once per completed fade, and the
  overlay's fade deadline shortens the event loop's wait.
- No file write happens inside `render::draw`.
- Loading a hand-edited row whose value is out of range applies the clamped
  value, reports `AppliedWithAdjustments`, renders the warning toast naming the
  adjusted fields, and leaves the row's raw TOML on disk untouched.
- Loading a row that needs no correction reports `AppliedExactly` and shows no
  warning.

### Phase 6 — `m`, a random saved favorite  · status: todo

#### Work Order

**Goal:** `m` picks a saved favorite at random and shows it.

**Spec:**

`AppGlobalAction::RandomFavorite` on `m`. `m` is free in every scope. `q` was
proposed first and cannot be used: it is the framework's `Quit`
(`keymap/global_action.rs:63`), so a mis-press would exit the app.

**cargo-tile has no source of randomness yet.** The workspace declares no random
crate at all, and Phase 1's `Xorshift` is `pub(super)` inside
`tui_pane::backdrop` — not reachable from here, and not to be exposed, since
widening tui_pane's surface is outside this plan. `random_settings(seed)`
consumes a seed; nothing yet produces one. This phase establishes cargo-tile's
own, in a new `random` module:

These are **two separate functions**, not one that both reads the clock and
accepts a seed — a single signature cannot do both, and describing it as if it
could is what leaves an implementer guessing:

- **A seed source**, `fn clock_seed() -> u64`, drawing from the clock (nanos since
  `SystemTime::UNIX_EPOCH`). It takes nothing and is the only part that is
  non-deterministic, so it is the only part tests never call. This matches
  tui_pane's dependency-free posture rather than adding a crate for two call sites.
- **An unbiased bounded index draw**, `fn bounded_index(seed: u64, bound:
  NonZeroIndexBound) -> usize`, which is pure: same seed and bound, same index.
  Callers compose the two — `bounded_index(clock_seed(), bound)` in the app,
  `bounded_index(fixed, bound)` in tests — and that composition is what makes the
  fixed-seed corpus in the gate possible.

Plain modulo skews toward the low indices whenever the bound does not divide the
generator's range; reject and redraw the short tail instead.

**The bound is a nonempty type, not a `usize` checked at the top and not an
`Option<usize>` returned from the bottom.** "Pick one of none" has no answer, so
the draw must not be reachable with an empty list at all. Introduce a small
semantic bound with a fallible constructor and no domain-owned `Option` — the
contract is `NonZeroIndexBound::try_from_len(usize) -> Result<NonZeroIndexBound,
EmptyIndexDomain>`, and `NonZeroUsize::new`'s external `Option` is converted
inside that constructor and never leaves it. The empty case is then decided
once, by the caller, at the point where it already has a different job to do
(open the empty notice). Do not store a `NonZeroIndexBound` in an `Option`
anywhere; the absence of a bound is `EmptyIndexDomain`, which says what the
absence means. A draw
that returns `Option<usize>` pushes that decision to every call site and invites
an `unwrap`; a draw that takes a bare `usize` and guards internally has to invent
an answer for a case that has none.

Phase 7 reuses both halves. Neither exposes nor changes anything in tui_pane.

Picks uniformly from the saved list and loads it through the same path `enter`
uses.

**Every way `m` can fail to load opens Phase 4's overlay in the matching state.**
`m` calls `favorites::load()`, so it faces exactly the positions Phase 4 already
named as `FavoritesOverlayContent` variants, and it reuses them rather than
inventing a second notice surface with its own owner:

| `load()` result | What `m` opens |
| --- | --- |
| `Loaded`, at least one recognized row | nothing — draws the favorite |
| `Missing` | `FavoritesOverlayContent::NoneSaved` |
| `Loaded`, no rows at all | `FavoritesOverlayContent::NoneSaved` |
| `Loaded`, rows present but none recognized | `FavoritesOverlayContent::OnlyUnrecognized` |
| `LocationUnavailable` | `FavoritesOverlayContent::LocationUnavailable` |
| `Unparseable` | `FavoritesOverlayContent::Unparseable` |
| `Unreadable` | `FavoritesOverlayContent::Unreadable` |

**This mapping already exists — reuse it, do not restate it.**
`FavoritesOverlayContent::from_file_state` (`favorites_overlay.rs:141`) is the
exhaustive match Phase 4 shipped, and it is what draws the loaded/empty
distinction above: a `Loaded` file with no rows at all falls to `NoneSaved`,
while `Loaded` with rows that this build understood none of falls to
`OnlyUnrecognized`. Writing that mapping a second time in `globals.rs` would put
the two copies one refactor apart from disagreeing. So `m` hands its
`FavoritesFileState` straight to `FavoritesOverlay::open_file_state` (569), which
already runs `from_file_state`, resolves the surface bindings and rebuilds the
line plan. Six source positions therefore reach five content variants, and a
seventh `FavoritesFileState` variant is a compile error inside `from_file_state`
rather than a silent fall-through to "no favorites".

`esc` dismisses each of them through the same app-modal route.

Because the overlay opens on failure and stays closed on success, `m`'s own
adjustment warning is a **toast** — nothing is covering it — and it must be
registered with `App::schedule_timed_toast` beside the push, per the Delegation
Context.

The key lives on `AppGlobalAction`, not on the three attract scopes. One place
instead of three near-copies, one section in the keymap overlay, and it works
from the grid as well: `m` over a working grid gives you a random favorite and
turns the attract screen on to show it. The ladder already suits this — attract
scope keys are offered first, and `m` collides with nothing they bind, so it
falls through to the app globals below.

**Files:**
- `crates/cargo-tile/src/globals.rs` — `RandomFavorite` variant, default binding, dispatch arm
- `crates/cargo-tile/src/random.rs` — new file: `clock_seed()`, the nonempty bound type, and the pure unbiased `bounded_index(seed, bound)`
- `crates/cargo-tile/src/main.rs` — declare `mod random;` in the `mod` block at 4–27
- `crates/cargo-tile/src/favorites_overlay.rs` — widen Phase 4's existing `FavoritesOverlay::open_file_state(FavoritesFileState, &Keymap<App>)` (569) to `pub(crate)` so `m` can hand it the very `FavoritesFileState` it already loaded; add no second constructor
- `crates/cargo-tile/src/favorites.rs` — delete the now-obsolete `#[expect(dead_code, reason = "loading a favorite starts in the next phase")]` attribute on `recognized()` (153)
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** Phase 5 provides `Attract::apply_favorite()`
and `Attract::request_show()`; this phase calls them rather than reaching into
`Attract`'s private fields. Phase 4 provides `FavoritesOverlayContent` with one
variant per load position and its non-selectable notices, dispatched through the
app-modal route ahead of the framework check; this phase opens them and adds none
of its own. Phase 3 requires every timed toast to be registered with
`App::schedule_timed_toast` beside its push, or it will not animate.

**The three Phase 2 APIs this phase consumes, exactly:** `favorites::load() ->
FavoritesFileState`; `FavoriteRows::recognized() -> impl Iterator<Item =
&Favorite>`, which is the **only** one of the two iterators this phase uses,
because a row this build cannot understand cannot be loaded and must never enter
the draw; and `FavoriteSettings::mode() -> AttractMode`, which supplies the mode
to switch to. Do not reach for `FavoriteRows::iter()` here — that one carries the
unrecognized rows and belongs to Phase 4's table.

Call `load()` on **every** `m` press, not once at startup: another instance can
save a favorite between presses, and a cached list would hide it.

Phase 2's load returns `FavoritesFileState` — a parse error, a read failure, or an
unresolvable config directory is reported, never treated as an empty list.
**Neither is a file whose rows exist but none are recognized.** `Loaded` with
`recognized()` empty is a distinct position from `Missing`: there is something in
the file and this build understood none of it, so `m` opens
`FavoritesOverlayContent::OnlyUnrecognized`, where the diagnostic lines name which
keys and spellings failed, rather than claiming no favorites are saved. Phase 5's `apply_favorite` returns `FavoriteApplicationOutcome`, so
`m` reports an adjusted favorite the same way `enter` does.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- Selection is proven through its **bounded index draw against a fixed seed
  corpus**, not by pressing the key until the row changes — a valid list can
  legitimately return the same row twice, so "repeated presses visibly move" is a
  flaky condition.
- All six non-loadable source positions reach their content variant through
  Phase 4's own `open_file_state`, and each renders and consumes `esc` through
  the app overlay route ahead of framework handling. Six positions map into five
  variants, because a missing file and a loaded-but-empty file both reach
  `NoneSaved`.
- A file holding only unrecognized rows does **not** report "no favorites": it
  opens `OnlyUnrecognized`, distinctly from both the `Missing` case and the
  loaded-but-empty case, which are asserted separately.
- Pressing `m` after another process saves a favorite can draw that favorite.
- `m`'s adjustment warning toast animates and expires with updates frozen and no
  input arriving, which proves it was registered with the visual schedule.
- The bounded draw's rejection path is exercised **directly**, not inferred from
  coverage: a fixed corpus that reaches every index proves nothing about bias.
  Either drive the reject-and-redraw threshold with seeds chosen to land in the
  short tail and assert the redraw happens, or exhaust a reduced-width generator
  model over every possible state and assert each index is produced an equal
  number of times.
- `NonZeroIndexBound::try_from_len(0)` returns `EmptyIndexDomain`, so the empty
  case is unreachable inside the draw rather than guarded there, and no
  `Option<NonZeroIndexBound>` appears in any signature or field.

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
framework binds capital `R` to `Restart` (`keymap/global_action.rs:64`), and no
attract scope binds either case, so `r` reaches the app globals through the
ladder untouched.

`ctrl-shift-r` was the original ask and cannot be delivered. A terminal sends the
same byte for `ctrl-r` and `ctrl-shift-r` (0x12) unless the Kitty keyboard
protocol is negotiated. cargo-port pushes those flags
(`crates/cargo-port/src/tui/terminal/run.rs:86–87`); **cargo-tile does not**.
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
correct during a fade-out, and `apply_favorite` returning
`FavoriteApplicationOutcome`. Phase 6 provides cargo-tile's `random` module — `clock_seed()`, the nonempty
bound type, and the pure `bounded_index(seed, bound)`; `r` composes the same two
(`bounded_index(clock_seed(), bound)`) to draw its mode rather than adding a
second helper, and its gate uses the pure half with fixed seeds. Phase 3 provides the terminal-area state and the zero-duration
sizing boundary, which must run before `random_settings` so the band draws its
width against the real screen rather than the unsized whole-range sentinel.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- Over a fixed seed corpus every mode and every enum variant is reached, and
  every value sits inside its clamps.
- A draw made while the band has never been shown lands inside the ceiling the
  current screen allows, not the unsized sentinel range.
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

Capture the current mode, **all three parameter sets**, and how the attract screen
was being presented. `AppGlobalAction::UndoReplace` on `u` restores them.

**Capture after the replacement is certain, not before the key is handled.** `m`
can find the file `Missing`, in an error state, or holding rows none of which this
build recognizes, and in each of those it replaces nothing. Taking the checkpoint
on the keypress would overwrite a good undo point with the current parameters and
leave `u` restoring what is already on screen — the one press where undo matters
is the press after a good one, and a failed `m` in between would have destroyed
it. So each of the three call sites captures only once its own replacement is
committed: after a favorite has been selected successfully, not on entry. One step only, and the
one-step limit is the type's job, not a flag beside it:

```rust
enum ReplacementUndoState {
    Unavailable,
    Available(PreviousAttractConfiguration),
}
```

Taking the checkpoint returns it to `Unavailable`, so a second `u` has nothing to
restore because there is nothing there — not because a boolean said so.

**Capture sits between "the replacement will happen" and "the replacement has
happened".** Each of the three call sites knows it has a real candidate before it
applies anything: `m` has selected a recognized row, `enter` has one under the
cursor, `r` has drawn a mode and settings. The checkpoint is taken at that point —
after the candidate is certain, **before** `apply` runs — so what it stores is the
configuration being replaced rather than the one replacing it.

**Phase 3's sizing boundary is single-mode and this phase needs an all-mode one.**
`Attract::size_current_animation` (`attract/mod.rs:382`) sizes only the animation
the current mode selects, which is all Phase 3's save needed. Undo captures and
restores **all three** parameter sets, so a mode that has never been shown — or
one that was last sized against a taller terminal — would be captured or restored
against a stale area. Add an all-mode zero-duration sizing operation beside it in
`attract/mod.rs`, running the same `advance(area, Duration::ZERO)` pass over each
of the three animations and recording each in `AnimationSizing`, and run it before
both the capture and the restore.

**"Whether the screen was up" is not a boolean and `showing()` cannot answer
it.** `showing()` (`attract/mod.rs:519`) is only `faded != u8::MAX`, so it stays
true through an entire fade-*out*, and it cannot tell a screen that came up
because the grid went idle from one the reader explicitly asked for, nor either
from one the reader dismissed. Restoring from a bool would put the screen into a
position the reader never had.

**But the presentation is richer than four coarse variants, and undo restores the
settled position rather than the instant.** `Attract` actually carries four
separate things: `Asked::{Nothing, For, Against}` (`attract/mod.rs:117`) — the
reader's own standing instruction; `Standing::{Showing, Leaving, Working,
Settling(Instant)}` (141) — where the screen stands with the roster, including two
mid-transition positions; `covering` (278) — whether the screen replaces the grid
or lies over it; and `faded` — numeric fade progress. A four-variant enum cannot
describe that, and the earlier draft of this phase promised to restore a
mid-fade-out position it had no way to represent.

The rule for this phase is therefore: **capture what is durable, restore the
settled destination, and do not replay a transition.**

- **Capture verbatim:** `Asked`, because it is the reader's own instruction and is
  meaningful at any later moment; and `covering`, because it decides whether the
  restored screen replaces the grid or lies over it.
- **Capture as a destination, not a position:** where the screen was *heading* —
  on screen or off it. `Standing::Leaving` and `Standing::Settling` are both
  in-flight, and a `Settling` deadline captured a minute ago is meaningless when
  `u` is pressed.
- **Do not capture `faded` and do not restore it.** A fade is a transient. Putting
  back a half-finished one reproduces a glitch rather than a state, and the reader
  cannot have meant "and leave it 40% faded". The restore sets the destination and
  lets the ordinary fade run to it, exactly as the equivalent keypress would.

Name the captured shape accordingly, and name each of its three parts for what
it means rather than for the field it came from. `Asked` does not say what is
being asked for, and "covering" is a boolean carrying two domain positions, so
neither survives into the captured type as it stands:

```rust
enum AttractVisibilityInstruction { FollowRoster, Show, Hide }
enum AttractVisibilityDestination { Shown, Hidden }
enum AttractGridPresentation { OverGrid, ReplacesGrid }

struct PreviousAttractPresentation {
    instruction: AttractVisibilityInstruction,
    destination: AttractVisibilityDestination,
    presentation: AttractGridPresentation,
}
```

`AttractVisibilityInstruction` is the reader's own standing instruction captured
verbatim from `Asked`; `AttractVisibilityDestination` is where the screen was
heading, not where it was; and `AttractGridPresentation` replaces the `covering`
flag with its two meanings written out. No four-variant approximation, and no
bool.

Restoring a hidden destination needs a hide transition to match Phase 5's
`request_show()` — idempotent in the same way, so restoring twice or restoring a
screen that is already down does nothing rather than toggling it back up.

**What `u` tells the reader, in all three of its outcomes.** An exact restore can
legitimately put back a *hidden* screen, so "it worked" is not always visible and
a silent `u` would read as a dead key. Every outcome gets a surface:

- **Restored exactly** — a brief confirmation naming the mode that came back. It
  is the one case where the screen may show nothing, so it is the case that most
  needs saying.
- **Restored with adjustments** — the terminal shrank since the checkpoint, so the
  restore was clamped. Report through `AttractConfigurationRestoreOutcome`, naming
  which of the three parameter sets moved.
- **Nothing to undo** — `ReplacementUndoState::Unavailable`. Currently this is an
  unexplained no-op, which is indistinguishable from a broken key. Say so.

All three are toasts: `u` is a global action with no modal open, so nothing covers
them. **Each must be registered with `App::schedule_timed_toast` beside its push**
per the Delegation Context, or it will not animate on this loop.

It covers all three replacing actions, not just the random draw: an undo that
catches one but not the others is worse than none, because you cannot predict
which press it will catch. The checkpoint is captured by whichever of the three replacing actions
runs, so this phase adds the capture at all three existing call sites.

`u` is unbound in every scope. Only `ctrl-u` is taken, by tui_pane's vim
half-page scroll, and cargo-tile sets no vim mode — so `h` `j` `k` `l` are free
too.

**Files:**
- `crates/cargo-tile/src/globals.rs` — `UndoReplace` variant, default binding, dispatch arm
- `crates/cargo-tile/src/attract/mod.rs` — `ReplacementUndoState`, `PreviousAttractConfiguration`, the captured presentation (standing instruction, destination, covering), `AttractConfigurationRestoreOutcome`, the all-mode zero-duration sizing operation beside `size_current_animation` (382), the idempotent hide transition, the capture, and the restore
- `crates/cargo-tile/src/favorites_overlay.rs` — capture once `enter`'s row is certain, before `apply_favorite` runs
- `crates/cargo-tile/Cargo.toml` — patch version bump
- `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` → `### Added`

**Constraints from prior phases:** The three replacing call sites are Phase 5's
`enter` (in `favorites_overlay.rs`, via `Attract::apply_favorite()`), Phase 6's
`m` (in `globals.rs`), and Phase 7's `r` (in `attract/mod.rs`). Phase 3's
`size_current_animation` (`attract/mod.rs:382`) sizes **only the animation the
current mode selects**; this phase adds the all-mode counterpart it needs, and
Phase 3's `App::schedule_timed_toast` must be paired with every toast push here or
the notice will not animate. Phase 1's
`settings()` on each animation is what the checkpoint stores, and `apply` is what
restores it — an ordered semantic transition, so restoring a checkpoint leaves
the same runtime state the equivalent keypress would. Phase 5's
`Attract::request_show()` restores visibility, and this phase adds the matching
hide transition for a checkpoint that was taken while the screen was down.
Phase 5's `apply_favorite` returns `FavoriteApplicationOutcome`, whose
`AppliedWithAdjustments { requested, effective }` carries **one**
`FavoriteSettings` — the single mode's parameters that one favorite held. That
type cannot describe this phase's restore and must not be reused for it: an undo
puts back the mode, *all three* parameter sets, and the presentation state, so a
report shaped around one settings variant would be a name that is not true of its
payload.

This phase therefore owns two of its own types: a full semantic configuration
snapshot — the same shape `PreviousAttractConfiguration` stores — and
`AttractConfigurationRestoreOutcome`, which compares the requested and effective
**full** configurations and names which of the three parameter sets moved. A
restore that has to be clamped, because the terminal shrank since the checkpoint,
reports through that outcome rather than applying it silently. Phase 3's
terminal-area state and zero-duration sizing boundary run before the restore for
the same reason.

**Acceptance gate:** `verify.sh check/test/lint cargo-tile` all green, plus:

- After each of the three replacing actions, `u` restores the mode, all three
  parameter sets, the standing instruction and the covering flag — proven
  separately for a screen that came up automatically, one the reader asked for,
  one the reader dismissed, and one fully hidden.
- A checkpoint taken while the screen was mid-transition restores the settled
  destination it was heading to, and does **not** put back a partial fade: after
  the restore the fade runs normally to that destination.
- All three restore outcomes reach the reader: an exact restore onto a hidden
  screen still shows a confirmation, an adjusted restore names which parameter
  sets moved, and `u` with nothing to undo says so instead of doing nothing
  visible. Each of those toasts animates and expires with updates frozen and no
  input arriving.
- A mode that has never been shown is sized before it is captured and before it is
  restored: capturing on one terminal size, shrinking, then restoring reports the
  adjustment rather than storing or applying an unsized sentinel.
- A second `u` does not step back twice: the state is `Unavailable` after the
  first, and the restore is a no-op rather than a toggle.
- Restoring onto a terminal smaller than the one the checkpoint was taken on
  reports the adjustment through `AttractConfigurationRestoreOutcome`, naming
  which of the three parameter sets moved, instead of correcting silently.
- A press of `m` that replaces nothing — the file missing, in an error state, or
  holding no recognized rows — leaves an existing
  `ReplacementUndoState::Available` exactly as it was, and the following `u`
  still restores the configuration from before the last real replacement.
