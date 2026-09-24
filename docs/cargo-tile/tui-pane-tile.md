# Move tile management into tui_pane and start cargo-handler

## Goal

`cargo-tile`'s tile grid, its attract screen, and the application scaffolding
around them are general machinery living inside one application. Move them into
`tui_pane` so a second tool, `cargo-handler`, shares them, then build
`cargo-handler` on top.

`cargo-handler`'s first version: the summary cell, `+`/`-` to add and remove
tiles, focus movement, the attract screen with favorites, the settings overlay,
the keymap and `?` overlays, and the status bar.

## Decisions already made

- **The summary cell stays, always.** Cell one is a titled summary cell in every
  grid; `cargo-handler`'s is empty until it has something to show. There is no
  anchorless grid, so `TABLE_CELL` stays the constant 1 and the earlier plan's
  "optional summary" phase is dropped with its hazard.
- **Attract moves now, and `cargo-handler` uses it** from its first version.
- **Also moving in this pass:** the grid drawing loop, grid keys and clicks, the
  terminal runner, the settings/config/status-bar scaffolding, and favorites.
- **Themes stay per-app.** `tui_pane` ships no colours; `cargo-handler` gets its
  own copy of the built-in palettes and theme TOMLs.

## Constraints

- **`cargo-tile` behaves exactly as it does today at every phase boundary** —
  on screen, at the keyboard, and in every file it reads or writes. The only
  visible difference is a deeper dependency on `tui_pane`.
- **`keymap.toml` compatibility is behavior.** Action names, TOML table names
  (`APP_GLOBALS_SECTION` = "App Shortcuts", the three attract scopes, favorites),
  default keys, and the keymap overlay's row order must come out identical.
  Moving grid actions into a `tui_pane` enum must not move them into a new
  table in `cargo-tile`'s file.
- `config.toml` and `favorites.toml` keep their exact keys and layout.
- Widen `tui_pane`'s public API only as far as the two apps need. Going `pub`
  re-arms `must_use_candidate`, `missing_panics_doc` and `missing_docs`; every
  newly exported item needs a pass.
- Each phase ends with the workspace building, `cargo nextest run` green,
  `cargo +nightly fmt`, and the lint pipeline clean. Changelog entries for every
  crate touched.

## What is already true

- `tiles.rs` is ~3,000 lines, half tests. The rect-splitting half already lives
  in `tui_pane`; what is stranded is grid policy.
- `tiles.rs` couples to `cargo-tile` through ten tuning constants, `TABLE_CELL`,
  and `census::InvocationId`, used only as an opaque identity (`Clone + Eq +
  Debug`). `MIN_INITIAL_ROWS` is also used by `config.rs` and `settings.rs`.
  `TABLE_CELL` is also used by `census/scan.rs` (`focus_cell(TABLE_CELL + 1)`).
- The test `reused_pids_keep_distinct_tiles_and_focus` asserts on private
  `Focus`/`Slot`; a `(pid, lifetime)` tuple id lets it move.
- `attract/mod.rs` couples to: `app::Updates` (Live/Frozen), `probe` (frame
  log: `note`, `timed`, `trace`), `random` (`clock_seed`, `bounded_index`,
  `NonZeroIndexBound`), `favorites::AttractSettings`, eleven step/fade
  constants, and `AppPaneId::Attract(mode)` inside the three `Pane` impls.
- `render.rs::draw_panes` (~100 lines) plus the rows readout and
  `draw_number` are the only place the `GridLines` + `draw_clipped` ordering
  lives. Only `draw_contents`, `tile_demands` and the sccache labels are
  cargo-specific.
- `terminal.rs` is ~700 lines, of which scan and sccache draining are the
  cargo-specific part.
- `docs/tui_pane/as-built/app-template.md` cites a tag `app-template-v1` that
  does not exist; the template is commit `89682872`. Repoint its four citations.
- `ThemeRegistry::from_dir_with_builtins` is still the name `cargo-tile` calls.

## Phase 1 — the grid

- `TileGrid<Id>` in `tui_pane`, `Id: Clone + Eq + Debug`, with type aliases in
  `cargo-tile` bound to `InvocationId` so call sites do not change. Ten tuning
  constants become a private settings struct whose `Default` is today's values;
  export `MIN_INITIAL_ROWS` and `TABLE_CELL`.
- The drawing loop becomes a `tui_pane` function taking the anchor title, the
  per-cell content closure, and anchor border labels (cargo-tile's sccache
  label). The rows readout and the empty-cell number move with it.
- Grid actions (add, remove, four focus arrows) get a `tui_pane` dispatch
  helper; each app keeps the variants in its own globals enum so the TOML table
  stays where it is. `Hittable` for the grid and click-to-focus move too.
- Move the tests with the code; move module prose as it stands.

## Phase 2 — runner, settings, config, status bar

- Runner: terminal setup/restore, input thread, the paced demand-driven loop,
  resize-burst draining, `force_repaint`, the key ladder (app modal → framework
  overlay → attract scope → framework globals → app globals), overlay key
  dispatch, and `restart_self`. The app supplies per-poll work (cargo-tile's
  scan and sccache draining, roster fade) through a trait.
- Settings: `Step`, `stepped`, row widths, appearance steppers and `apply`,
  the Files and Notices sections, popup sizing. Apps append their own rows.
- Config: load / restate / save generic over the app's serde config, paths
  keyed by the app's directory name, `AppearanceConfig`, and the `[tiles]`
  `initial_rows` field. `theme::install` moves; palettes do not.
- Status bar: `bar_palette`, keymap and `?` overlay drawing, the frozen and
  attract notes.
- `iterm2.rs` moves with the runner, since setup/restore owns it.

## Phase 3 — attract and favorites

- Move the attract controller, `held_key`, `backdrop_notice`, the three action
  enums with their default bindings, `Updates`, `fade_to_background`, the
  backdrop-notice line, and `random` (fold it into `tui_pane`'s existing
  backdrop randomness if the two agree). Step constants become defaults.
- The three `Pane` hosts need an app-side pane id per mode: a small trait on
  the app (`&mut Attract`, pane id for a mode) with generic hosts in `tui_pane`.
- `probe` becomes a note/timing sink the app supplies, so `tui_pane` does not
  own a frame-log env var named for `cargo-tile`.
- Favorites and the favorites overlay follow: the file path comes from the app,
  and the TOML keys stay byte-identical.

## Phase 4 — cargo-handler

- New member `crates/cargo-handler` (the `crates/*` glob picks it up), binary
  `cargo-handler`, reachable as `cargo handler`. Copy `without_subcommand_name`
  from `cargo-tile/src/cli.rs` for argv; no subcommands.
- Built on the phase 1–3 pieces: `TileGrid<()>`-style empty cells plus the
  summary cell, `+`/`-`, arrows, Tab, click focus, attract with favorites,
  settings (appearance, `initial_rows`, Files, Notices), keymap and `?`
  overlays, status bar. Its own theme built-ins and `themes/*.toml`, config dir
  `cargo-handler`.
- Nothing ever sends demands, so every cell is at the floor and the grid is
  always idle: attract comes on after the quiet period, the same as `cargo-tile`
  with nothing running.
- `Pane::mode()` returns `Mode::Static`; Tab goes through `Pane::cycle_step`;
  register a navigation scope (the settings overlay moves on it).
- `cargo_common_metadata`: description, keywords, categories, license, readme,
  plus a real `README.md` and `CHANGELOG.md`.

## Already ruled out

- No dependency cycle: `tiles.rs` imports only `std`, `ratatui` and `tui_pane`.
- The orphan rule does not block `impl Hittable<Picked> for TileGrid` through an
  alias (checked by compiling a repro).
- Zero-cell geometry already works (`columns(0, _)`, `shares(&[], ..)`).
