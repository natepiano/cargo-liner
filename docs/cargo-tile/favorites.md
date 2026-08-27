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
  - `crates/cargo-tile/src/attract/mod.rs` — `AttractMode` (194, `pub(crate)`),
    `Attract` (269), `new` (327), `toggle` (356), `request_show` (367),
    `randomize` (373), `asked_for` (404), `keyed_mode` (425),
    `current_settings` (438), `apply_settings` (448),
    `size_current_animation` (480), `record_terminal_resize` (499), `showing`
    (617), `due_back` (628), `identify` (640), `advance` (726), `render` (837),
    `ground` (859).
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
  - `crates/cargo-tile/src/globals.rs` — `action_enum!` block (**52–69**),
    `RandomizeAttract` (63), `SaveFavorite` (65), `OpenFavorites` (66),
    `RandomFavorite` (67), `defaults()` (78), `r` (88), `ctrl-s` (90), `ctrl-o`
    (91), `m` (92), `dispatcher()` (96), `dispatch` (100), its
    `RandomizeAttract` arm (111), `SaveFavorite` arm (113), `RandomFavorite` arm
    (118), `show_random_favorite` (123), `show_random_favorite_with` (127),
    `mode_label` (197). `mode_label` is **private to this file**. The module doc
    (**12–22**) **enumerates** the non-grid globals and never states a total: a
    phase adding one names it in the group it belongs to and leaves every count
    alone. Phase 6's only review finding was a count there that had been wrong
    since Phase 3.
  - `crates/cargo-tile/src/favorites.rs` — `favorite_refusal_message(mutation:
    FavoritesMutation, retry: &FavoritesRetryInstruction, error:
    &FavoritesMutationError) -> String` (**426**). Phase 5 moved it here out of
    `globals.rs` and generalised it: the mutation word and the retry sentence
    both come from arguments, so `FavoritesMutation::{Save, Delete}` (381) and
    `FavoritesRetryInstruction` (398) carry what used to be hardcoded. Any phase
    reporting a refused favorites mutation calls it as it stands.
  - `crates/cargo-tile/src/keymap.rs` — `build_keymap` (64), scope registrations (75–83).
  - `crates/cargo-tile/src/terminal.rs` — `handle_key` (**717**), which now
    dispatches an open favorites overlay **first**, at **720–723**, and returns
    before the framework is consulted; the framework branch
    `if let Some(overlay) = app.framework.overlay()` (**724**), whose
    open-overlay toggle runs at **726–731** and carries **no**
    `GlobalAction::Dismiss` fallback — Phase 4 removed that clause;
    `dispatch_overlay_key` (**766**); `if app.updates == Updates::Frozen {`
    (**508**), its `else` (**510**), the attract frame request **inside** that
    else (**555**). Phase 5 added the fading-row advance at **493–499**, which
    commits one pending deletion per loop iteration outside the `Frozen` branch. Phase 3 put the toast prune and the shared visual frame
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
`push(AttractSettings) -> Result<Favorite, FavoritesMutationError>`, and
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
unrecognized rows. `AttractSettings::mode() -> AttractMode` reads the mode off
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

### Phase 5 — `enter` loads, `x` deletes with a fade  · status: done

#### As-built

`enter` on a recognized row loads it. `FavoritesOverlay::handle_action` returns
`FavoritesOverlayActionOutcome::Load(settings)`; the **module-level `dispatch`
function** in `favorites_overlay.rs` — not `handle_action`, which holds no
`&mut App` — applies it through `Attract::apply_settings`, closes the modal,
calls `Attract::request_show()`, and reports the outcome as a scheduled toast
paired with `App::schedule_timed_toast`.
`apply_settings(&mut self, requested: AttractSettings) -> SettingsApplicationOutcome`
sizes the requested mode's own animation, applies, reads the settings back, and
answers `AppliedExactly` or
`AppliedWithAdjustments { requested, effective }`; the row on disk is never
rewritten, so a value that is out of range on this terminal survives to a taller
one. `request_show()` is a const setter writing `asked = Asked::For` **and**
`covering = true`, so it is idempotent and reverses a fade-out.

`x` marks the row `FavoriteRowLifecycle::Removing { since }` and commits the disk
removal at the keypress, in the event loop, never in `render::draw` — the fade is
presentation only. `FavoritesOverlay::advance(now)` runs outside the
`Updates::Frozen` branch and answers `Quiet`, `Repaint`, or
`CommitRemoval(FavoriteId)`; alpha comes from elapsed time through
`blend_color(text_default(), ground(), alpha)`, so extra draws never shorten it.
`finish_removal` is idempotent per id, and closing mid-fade commits through
`begin_close` before `finish_close` resets the cached plan.

A refused mutation raises `FavoritesOverlayNotice::DeletionRefused` inside the
open modal, because toasts render beneath overlays. The notice wraps and is
variable-height, bounded by the popup's existing 80%-of-area cap with
`CONTENT_MIN_HEIGHT` reserving one favorite row. `favorite_refusal_message` lives
in `favorites.rs` and takes `FavoritesMutation` and `FavoritesRetryInstruction`,
so the mutation word and the retry sentence are arguments rather than a hardcoded
"save" / `ctrl-s`; the match stays exhaustive over all five
`FavoritesMutationError` variants. `ResolvedBinding::{Bound, Unbound}` carries
`action_name: &'static str` separately from the caller's instruction phrase, so
an unbound retry names a real keymap action.

`FavoriteRows::refresh_recognitions` demotes the second and every later table
carrying an already-seen `FavoriteId` to `FavoriteRowRecognition::Unrecognized`,
spelled `<id> (duplicate)`, so no two rows share one id and `remove` can never
delete a row other than the one on screen.

**Files:**
- `crates/cargo-tile/src/favorites_overlay.rs` — `Load`/`Delete` handling, the
  module-level `dispatch` branch that reaches `Attract`, `FavoriteRowLifecycle`,
  `advance` / `finish_removal` / `begin_close` / `finish_close`, the wrapped
  notice and its height allowance, and the private reporting path
  (`report_application_outcome`, `favorite_adjustment_message`,
  `push_scheduled_toast`).
- `crates/cargo-tile/src/attract/mod.rs` — `request_show`, `apply_settings`,
  `SettingsApplicationOutcome`.
- `crates/cargo-tile/src/favorites.rs` — `favorite_refusal_message`,
  `FavoritesMutation`, `FavoritesRetryInstruction`, `ResolvedBinding`,
  `refresh_recognitions`.
- `crates/cargo-tile/src/globals.rs`, `crates/cargo-tile/src/terminal.rs` — call
  sites for the moved refusal message and the fading-row advance.

**Binds later work:**
- `Attract::apply_settings(&mut self, requested: AttractSettings) -> SettingsApplicationOutcome`
  with `AppliedExactly` / `AppliedWithAdjustments { requested, effective }` is the
  only load path; it already sizes the requested mode's own animation. *`m`, a
  random saved favorite* and *`u`, undo the last replacement* both load through it
  with the overlay closed, so their adjustment report is a scheduled toast paired
  with `App::schedule_timed_toast`, never an overlay notice.
- `Attract::request_show()` overwrites `asked` as well as `covering`. It is a
  transition, not a restore, and nothing captured survives a call to it — an
  `Attract`-owned restore starts the destination transition first, then writes both
  captured values back.
- The `enter` capture site is the module-level `dispatch` function's
  `FavoritesOverlayActionOutcome::Load(settings)` branch.
- `FavoritesOverlayNotice::{NoNotice, DeletionRefused, FavoriteAdjusted}` is the
  in-modal message surface; a new in-modal message is a new variant, not a second
  string field.

**Gotchas:**
- Clearing `FavoritesOverlayNotice` ahead of the `SelectedFavorite` check, or in
  `finish_removal`'s `Ok` arm without checking the id, silently erases a refusal
  that is still true. Both clears happen only for a real favorite, and a success
  clears only the notice its own row raised.
- Enum values printed beside the favorites table go through the lowercase name
  helpers (`direction_name`, `fraying_name`, `drift_name`, `text_fill_name`,
  `pixel_resolve_name`, `pixel_fill_name`), never `{:?}` — the file and the table
  both spell them lowercase.
- The popup already carries two independent height bounds (`popup_height_cap` at
  80% of the area, then `.min(area.height)`); a third fixed cap on a sub-region
  only clips content.

**Ruled out:**
- A one-line unwrapped `Paragraph` for the notice — at 80 columns the prefix alone
  exceeds the interior width, so the cause never reached the screen.
- A fixed three-row cap on the refusal notice — redundant against the two existing
  bounds, and it clipped the cause.
- Reusing the caller's instruction phrase as the keymap action name in
  `ResolvedBinding::Unbound` — it named an action that does not exist.
- Committing the disk removal at fade end rather than at the key.
- Gating the load on `Attract::showing()` or `toggle()` — `showing()` stays true
  through a fade-out and `toggle()` can ask for the opposite state.

### Phase 6 — `m`, a random saved favorite  · status: done

#### As-built

`AppGlobalAction::RandomFavorite` binds the bare `m` key, free in every scope
(framework globals, all three attract panes, vim nav extras — nothing shadows
it). On every press it loads the favorites file fresh (never cached, so a
favorite saved by another running instance is visible immediately), draws one
recognized row uniformly at random through a bounded index draw, applies it to
the attract screen through `Attract::apply_settings`, and calls
`Attract::request_show()`.

Every state that yields no usable favorite — missing file, unreadable,
unparseable, unresolvable config location, loaded-but-empty, or loaded with
rows present but none recognized — opens the favorites overlay at the matching
diagnostic position instead of doing nothing, reusing
`FavoritesOverlayContent::from_file_state` through `FavoritesOverlay::open_file_state`
(now `pub(crate)`) rather than restating the mapping.

New `random.rs` is the crate's only source of randomness: dependency-free
SplitMix64 plus rejection sampling (uniform, not biased by modulo), exposed as
`clock_seed() -> u64`, `NonZeroIndexBound::try_from_len(usize) ->
Result<NonZeroIndexBound, EmptyIndexDomain>`, and `bounded_index(seed: u64,
bound: NonZeroIndexBound) -> usize`, all `pub(crate)`. The empty-list case is
unreachable inside the draw — it is decided once, by the caller, via the
fallible constructor.

An adjusted favorite applied from outside the overlay reports through a new
shared `pub(crate) fn report_closed_overlay_adjustment(&mut App,
SettingsApplicationOutcome)` in `favorites_overlay.rs`: silent on an exact
application, otherwise a scheduled lowercase warning toast via the existing
`push_scheduled_toast`/`report_application_outcome` path. `enter`'s closed
branch now routes through the same function, leaving one formatter and one
scheduled-toast site. The adjustment is clamped in the running attract state
only and never rewrites the saved favorites file.

`globals.rs::show_random_favorite_with(app, load, seed)` is the deterministic
test seam; `show_random_favorite` is the zero-argument production entry.
`favorites.rs::recognized()` is now production code — its dead-code exemption
is gone.

**Files:**
- `crates/cargo-tile/src/random.rs` — seed source, nonempty index bound, unbiased bounded draw
- `crates/cargo-tile/src/globals.rs` — `RandomFavorite` action, dispatch arm, `show_random_favorite`/`show_random_favorite_with`
- `crates/cargo-tile/src/favorites_overlay.rs` — `report_closed_overlay_adjustment`; `open_file_state` widened to `pub(crate)`
- `crates/cargo-tile/src/favorites.rs` — `recognized()` no longer dead code

**Binds later work:** `random.rs`'s three `pub(crate)` functions are the
crate's only RNG; the phase that adds `r` (randomize everything) composes
`bounded_index(clock_seed(), bound)` from this module rather than adding a
second one. `m` applies a favorite from outside the overlay via
`show_random_favorite_with` in `globals.rs`, a second call site (alongside
`favorites_overlay.rs`'s dispatch `Load` branch) for anything that must observe
a wholesale parameter replacement — the undo phase (`u`) needs both.

**Gotchas:** `Attract::request_show()` only sets fields (`asked`, `covering`);
`advance()` re-derives the fade direction from `asked` every frame, so nothing
a caller writes before `advance()` runs survives as a destination. `globals.rs`'s
module doc enumerates the non-grid globals rather than stating a total — an
earlier stale count is why; keep it as an enumeration when adding globals.

**Ruled out:** a second random helper module for later phases — compose from
`random.rs` instead; restoring a captured visibility destination for undo —
`advance()` recomputes direction from `asked` alone, so a stored destination is
unreachable.

### Phase 7 — `r`, randomize everything  · status: done

#### As-built

`r` draws a fresh attract configuration — a mode and that mode's parameters — and
shows it. `AppGlobalAction::RandomizeAttract` (`globals.rs:63`, key at **88**,
dispatch arm at **111**) calls `Attract::randomize()`.

`AttractMode` owns the selection: `ALL: [Self; 3]` and `INDEX_BOUND:
NonZeroIndexBound`, a `const` whose `Err` arm is a `panic!` so an empty mode list
is a compile error rather than a runtime fallback. `draw(seed)` calls
`random::bounded_index` and maps the index with a total `match`; no runtime branch
in it can be skipped.

`Attract::randomize()` seeds from `random::clock_seed()`. `randomize_from_seed`
sets the mode, calls `size_current_animation()`, draws through
`draw_random_settings(&self)`, binds the `apply_settings` outcome and
`debug_assert_eq!`s it against `AppliedExactly`, then `request_show()`. **Sizing
precedes the draw**, so `tui_pane`'s `random_settings(seed)` bounds each parameter
by the real terminal and a narrow window can never be handed a band wider than
itself.

Four names were corrected because `r` routes a never-saved configuration through
them: `FavoriteSettings` → `AttractSettings`, `Attract::apply_favorite` →
`apply_settings`, `FavoriteApplicationOutcome` → `SettingsApplicationOutcome`
(`AppliedExactly` / `AppliedWithAdjustments { requested, effective }`),
`Attract::favorite_settings` → `current_settings`. The on-disk favorites format is
unchanged — schema constants, mode spellings, serializer and format fixtures all
byte-equivalent.

**Files:**
- `crates/cargo-tile/src/attract/mod.rs` — `AttractMode::{ALL, INDEX_BOUND, draw}`,
  `Attract::{randomize, randomize_from_seed, draw_random_settings,
  current_settings, apply_settings}`, `AttractSettings`,
  `SettingsApplicationOutcome`.
- `crates/cargo-tile/src/globals.rs` — the `RandomizeAttract` action, its `r`
  binding and dispatch arm, and the module-doc line naming `r` among the non-grid
  globals.
- `crates/cargo-tile/src/favorites.rs`, `favorites_overlay.rs` — renamed API only.

**Binds later work:** `Attract::apply_settings` is the crate's single point at
which a wholesale replacement is certain and about to happen; its three production
callers are `randomize_from_seed`, the favorites overlay's `Load` branch, and
`show_random_favorite_with`, and each reaches it only after its own candidate has
succeeded. `randomize_from_seed` assigns `self.mode` **before** calling
`apply_settings`, so anything that reads the outgoing mode from inside that method
sees the new one. `random.rs` remains the crate's only randomness for app-owned
choices.

**Gotchas:** a `debug_assert` around a call compiles the call out of release
builds — the outcome must be bound first and the binding asserted. `AttractMode::ALL`
order is load-bearing: `draw`'s index-to-mode `match` mirrors it positionally.
`globals.rs`'s module doc enumerates the non-grid globals and states no total.

**Ruled out:** a second random generator or bounded-selection helper beside
`random.rs`; a hard `assert!` on the keypress path, which would kill the whole TUI
for an invariant the reader could not act on; renaming `Favorite`, `FavoriteId`, or
any save/load/delete path, which genuinely are about saved favorites.

### Phase 8 — `u`, undo the last replacement  · status: done

#### As-built

`u` restores the complete attract configuration displaced by the most recent
wholesale replacement — mode, all three parameter sets, and both presentation
values — and reports the result as one toast: nothing to undo, an exact restore
naming the mode, or a restore naming which parameter sets the current terminal
moved.

`Attract::apply_settings` is the single capture site. It sizes every animation,
captures the configuration it is about to displace into `ReplacementUndoState`,
then applies. The three replacement paths — `r`, `m`, and `enter` — get undo
without knowing the checkpoint exists. `ReplacementUndoState` is `Unavailable`
or `Available(AttractConfigurationBeforeReplacement)`, and restore consumes it,
so a second `u` says there is nothing to undo.

`restore_configuration_before_last_replacement` calls neither `apply_settings`
nor `request_show`. It clears the checkpoint first, sizes every animation,
applies the three parameter sets through their own `apply` methods, and writes
mode and both presentation values into the private fields. It returns
`AttractConfigurationRestoreOutcome`, whose adjusted arm carries
`AdjustedAttractParameterSets` — the seven nonempty combinations, so "restored
with adjustments, nothing adjusted" is unrepresentable.

`AttractVisibilityInstruction` (`FollowRoster` / `Show` / `Hide`) and
`AttractGridPresentation` (`OverGrid` / `ReplacesGrid`) replace the former
`Asked` enum and `covering: bool`. They drive fade direction, keyboard
ownership, the status-line attract note, and whether the grid is drawn — which
is what makes an undo return the screen the viewer was actually looking at.

**Files:**
- `crates/cargo-tile/src/attract/mod.rs` — the capture point, the undo state,
  the restore path, the outcome types, and the two presentation enums.
- `crates/cargo-tile/src/globals.rs` — the `u` binding and the three toasts.
- `crates/cargo-tile/src/favorites_overlay.rs` — the `enter` load path's undo
  coverage.

**Gotchas:**
- `apply_settings` sizes every animation before it captures. Capturing first
  stores parameters for modes never shown and therefore never fitted to the
  terminal, and those values are wrong the moment they are restored.
- A test that sizes the animations itself cannot see that ordering defect; the
  capture-order tests must let `apply_settings` perform the first sizing.
- An undo test asserting only the settings cannot see a presentation value
  captured in the wrong order; it has to build a non-default hidden-but-
  grid-replacing state and compare the whole configuration.
- `faded` is neither captured nor restored: fade progress is transient, and a
  restore that reset it would visibly jump.

**Ruled out:**
- Capturing at each call site rather than inside `apply_settings` — three copies
  of the same checkpoint, each able to drift.
- Restoring through `apply_settings` — it would capture a fresh checkpoint from
  the undo itself, making `u` its own undo target.
- Renaming `AttractPresentation` to state it holds only the restorable subset —
  the enclosing `AttractConfiguration` already carries that meaning.
- Capturing or restoring `faded`.
