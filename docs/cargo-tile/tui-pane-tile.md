# Move the tile grid into tui_pane

## Goal

`cargo-tile`'s tile grid is a general algorithm that happens to live inside one
application. Another tool needs it. Move it into `tui_pane`, make the
always-present summary cell optional, and show the anchorless grid working in a
small app whose only feature is the grid.

Three phases. Each one stands alone and ends with the workspace building,
`cargo nextest run` green, and the lint pipeline clean.

## What is already true

Facts worth having up front, so they do not each get rediscovered:

- `crates/cargo-tile/src/tiles.rs` is about 3,000 lines, roughly half of them
  tests.
- **The framework half is already in `tui_pane`** — the rect-splitting types
  and functions `tiles.rs` imports are all `pub use` at its crate root.
  `tiles.rs` says so itself: "splitting a rect is the framework's job, not this
  module's." What is stranded in `cargo-tile` is grid *policy* — how many
  columns there are, how tall each is, and how a cell travels when that
  changes.
- **The coupling to `cargo-tile` is eleven imports** (`tiles.rs:63-74`): ten
  tuning constants, plus `TABLE_CELL`, plus `census::InvocationId`. The impl
  half uses `InvocationId` only as an opaque identity — no method calls, no map
  key, no sort — so `Clone + Eq + Debug` is the whole requirement.
- **Ten of those constants appear nowhere outside `tiles.rs` and
  `constants.rs`.** `MIN_INITIAL_ROWS` is the exception: `config.rs:126` and
  `settings.rs:304,360` use it too.
- **`TABLE_CELL` escapes the module.** It is `pub(crate)` in
  `constants.rs:393`, and `census/scan.rs:2791` imports it for
  `grid.focus_cell(TABLE_CELL + 1)` at :3764.
- `birth_stamp::ProcessLifetime` and `census::process_identity` are test-only,
  as are the constants `TEST_INVOCATION_PID` and `TEST_REPLACEMENT_LIFETIME`.
  `roster::TrackedGroup` is **not** — it is a doc link in the impl half at :79.
- `docs/tui_pane/as-built/app-template.md` points at a tag `app-template-v1`
  that **does not exist**, locally or on `origin` (cited at :3, :14, :25, :26).
  The template itself is intact at commit **`89682872`**.

## Phase 1 — move the grid, generic over the cell identity

Move the algorithm and its tests into `tui_pane`, parameterised on the identity
a cell carries.

- `TileGrid<Id>`, where `Id` needs only `Clone + Eq + Debug` — identity, not
  behavior. Use a plain generic parameter, not a trait: nothing yet needs an
  implementor to *do* anything.
- **Absorb the generic in `cargo-tile` with type aliases, written before the
  first edit** — `TileGrid`, `TileContent`, `TileDemands`, `TileDemand`, each
  bound to `InvocationId`. Variant construction and `match` arms work through a
  generic alias, so what looks like a ten-file cascade becomes nothing.
  `Direction` carries no `Id` and only changes its import path; `Placement<Id>`
  is never named outside `tiles.rs`.
- **The ten tuning constants** become fields on a settings struct whose
  `Default` carries today's values — private, no public setters. `TABLE_CELL`
  is not one of them; phase 2 owns it. `MIN_INITIAL_ROWS` must stay reachable
  from `cargo-tile`, so export that one value rather than duplicating the
  clamp.
- The tests move with the code and need a test id type. One test,
  `reused_pids_keep_distinct_tiles_and_focus` (:1624), asserts on the private
  `Focus` and `Slot`; a `(pid, lifetime)` tuple id preserves what it is testing
  and lets it move rather than forcing those types public.
- Fix the broken intra-doc links as you pass them — `rustdoc` names them.
- **Move the module's prose as it stands.** `tiles.rs` carries ~200 lines of
  essay written in `cargo-tile`'s vocabulary — "the summary table", "one
  command's tile". Rewriting it domain-neutral is several hundred changed lines
  and stops this being a relocation. Neutralise only what phase 2 makes wrong,
  when phase 2 makes it wrong.

`cargo-tile` must look and behave exactly as it does now when this phase ends.
This is a relocation, not a redesign; resist improving the algorithm on the way
past.

## Phase 2 — the summary cell becomes optional

**Read this section carefully before editing — the obvious reading of it is
wrong, and the tests will not catch the mistake.**

`TABLE_CELL` is one constant serving two unrelated jobs:

1. **The anchor's count** — how many cells exist before the slots. This is what
   becomes a runtime 0 or 1.
2. **The 1-based cell-number base** — cell numbers start at 1, and always will,
   anchor or no anchor. `cell.checked_sub(TABLE_CELL)` at :1090 is `cell - 1`
   indexing `held.rows`; `sum + row + TABLE_CELL` at :812-817 rebuilds a 1-based
   number; the Tab wrap at :837,839 means "the first cell number". These keep
   the literal 1.

Both meanings appear throughout, and at :864 a single expression carries both —
`checked_sub(TABLE_CELL + 1)` is `cell - 1 - anchor_count`. **Classify every
site before changing any.** Substituting an anchor count everywhere gives an
anchorless grid where Tab silently does nothing and every column's focused row
is off by one — and because `cargo-tile` always has the anchor, the whole moved
test suite still passes. The defect surfaces in phase 3, well downstream of the
commit that caused it. Add tests covering an anchorless grid in this phase, not
the next.

Three sites are not arithmetic at all and need restructuring: `cells()` (:987)
opens with a hard-coded summary, `cell_wants()` (:1319) pushes the summary's
demand as row zero, and `placements()` (:942-952) emits a summary cell before
the slot loop. `TileDemands.summary` (:95-100) is a named field, so the input
type changes shape too.

**Focus needs a third state.** `Focus` is `Summary | Cell(Slot)` today and
`settle_focus` (:757) falls back to `Summary` unconditionally. An anchorless
grid opens with zero cells and nothing to focus, so add a "nothing focused"
variant. The first cell added takes focus.

`cargo-tile` constructs its grid with the anchor present and sees no change.

## Phase 3 — a starter application

A new workspace member — `crates/*` is a glob with no `exclude`, so a new
directory is already a member. **Name it `cargo-handler`**, so that an installed
binary of that name runs as `cargo handler`: cargo runs any `cargo-`-prefixed
binary on the path as a subcommand of its own. A package named `cargo-handler`
with a `src/main.rs` produces that binary with no `[[bin]]` section.

Cargo hands the subcommand name back as the first argument, so `cargo handler`
arrives as `["cargo-handler", "handler"]`. The template's `main.rs` ignores argv
entirely and would not notice, but anything that later reads arguments breaks on
it. Take `cargo-tile`'s `without_subcommand_name`
(`crates/cargo-tile/src/cli.rs:148-156`), which drops that word only when it
sits directly after the binary. There are no subcommands otherwise.

**Start from commit `89682872`** — ten modules, 1,245 lines, pure framework
wiring with no application in it. Not from `cargo-port`, which is also a live
`tui_pane` app but is 108k lines with a 16,730-line `tui/app/mod.rs`. Drift
since the sha resolves by compiler error, with one exception: theming inverted,
and `ThemeRegistry::from_dir_with_builtins` is now `new_with_builtins`, the app
supplying the whole palette. Copy `crates/cargo-tile/src/theme/` from `main`
over the sha's 33-line `theme.rs`.

The template has no grid in it, so the one thing it cannot give you is how a
cell is drawn. The `GridLines` + `draw_clipped` ordering that produces shared
borders and mid-transition clipping exists only at
`crates/cargo-tile/src/render.rs:381-440` on `main`. Read it there.

Its entire feature set:

- A grid with no anchor cell, opening empty.
- `+` adds a tile, `-` removes one.
- Arrows move the focus ring; Tab cycles it.
- **A minimal keymap.** The framework globals — `s` settings, ctrl-k keymap,
  `?` shortcuts, `q` quit, `R` restart, `x` dismiss — come from
  `GlobalAction`'s defaults and need no registration
  (`crates/cargo-tile/src/keymap.rs:4-7` says so). The app registers only its
  own bindings, so the keymap overlay fills itself in and lists nothing the app
  cannot do.
- **A settings overlay with starter values.** The template's 259-line
  `settings.rs` already carries the right set and should be kept as it stands:
  three `[appearance]` stepper rows that write `config.toml` and swap the theme
  in place, a `Files` section reporting the config, themes and keymap paths,
  and a `Notices` section that appears only when a startup or config error
  needs reporting. Add none of `cargo-tile`'s domain rows — the same module
  there has grown to 1,314 lines. The grid's tuning values are not among these
  rows, so phase 1's private settings struct stays private.
- The status bar along the bottom.

**The starter never calls `sync`. Its grid is `TileGrid<()>`, every cell is
empty, and `+` and `-` are the only things that change it.** `add()` queues
directly and default demands give every cell the floor, so the grid animates
without a demand source. Building one is out of scope.

Take the key wiring from `cargo-tile`, which is the known-good shape — the
framework does not route the nav scope to the focused pane by itself, so
copying `navigation.rs` alone yields arrows that do nothing on the grid, with
no compile error to say so:

- Four app-global actions for the arrows, with their own labels, rather than
  the framework's `NavAction` — that is a closed ten-variant set, and routing
  arrows through it also binds Home/End/PageUp/PageDown and lists them in the
  keymap overlay with no grid behavior behind them.
- Tab through `Pane::cycle_step`, whose `bool` tells the framework the key was
  spent on the grid rather than the pane cycle (precedent:
  `cargo-tile/src/keymap.rs:58-60`).
- `Pane::mode()` returns `Mode::Static`. The default is `Navigable`, which
  turns on the status line's nav region and advertises page/home/end for a grid
  that is not a list.
- Register a navigation scope regardless — the framework requires one once any
  pane registers shortcuts, and the settings overlay moves on it.

Also repoint `docs/tui_pane/as-built/app-template.md` at the sha, at its four
citations. The document is accurate; only the tag it names is gone.

## Constraints

- **`cargo-tile` behaves exactly as it does today, at every phase boundary.**
  Not merely after phase 1: phase 2 changes the shape of the type it builds its
  grid from, and phase 3 adds a crate beside it. Neither may change what it
  does on screen or at the keyboard. A visible change in `cargo-tile` is a
  defect in the move, not an improvement made along the way.
- Widen `tui_pane`'s public API only as far as the new app actually needs.
- **Going `pub` re-arms lints that `pub(crate)` suppressed.** The root
  `Cargo.toml` denies `pedantic`, `nursery`, `cargo`, `all` and `missing_docs`;
  `must_use_candidate` and `missing_panics_doc` fire only on exported items, so
  every newly-public method and field needs a pass. The new crate needs
  `cargo_common_metadata` satisfied — description, keywords, categories,
  license, readme — plus a real `README.md` and `CHANGELOG.md`.
- Changelog entries for both crates, nightly `rustfmt`, clean lint pipeline.

## Already ruled out

Do not spend time on these; they were checked:

- **No new dependency and no cycle.** `tiles.rs` imports only `std`, `ratatui`
  and `tui_pane`. No `Cargo.toml` change.
- **The orphan rule is not a problem** for `impl Hittable<Picked> for TileGrid`,
  including through a type alias. Checked by compiling a repro, not by
  reasoning about it.
- **Zero-cell geometry already works.** `columns(0, _)` and `shares(&[], ..)`
  already return empty; the empty-grid risk is focus, not geometry.
