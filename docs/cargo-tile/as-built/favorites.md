# Favorites and attract steering — as-built

A reference for the next engineer modifying `cargo-tile`'s favorites feature and the attract
steering it shares. cargo-tile persists **favorites**: named snapshots of the attract mode
and its steerable parameters, saved losslessly to a TOML file, listed in an overlay whose
rows can be selected, loaded, and deleted for exactly what they are — including rows a newer
build wrote that this build cannot parse. Loading a favorite is one of several **wholesale
attract replacements**: the others draw a fresh mode and its parameters (`r`), show a saved
favorite at random (`m`), and undo the last replacement (`u`). All of them route through one
apply point, `Attract::apply_settings`, which is also the sole undo-capture site. The data
model lives under `cargo-tile/src/favorites/`, the overlay under
`cargo-tile/src/favorites_overlay/`, the attract steering in `cargo-tile/src/attract/`, and
the app's randomness in `cargo-tile/src/random.rs`.

The desktop-backdrop feature this overlay's attract modes render against is a peer
subsystem in `tui_pane`; see the sibling
[desktop-backdrop.md](../../tui_pane/as-built/desktop-backdrop.md).

---

## How it works

### Data model — `cargo-tile/src/favorites/{mod,rows,recognition,file}.rs`

A directory module split by ownership; `mod.rs` re-exports a stable `pub(crate)` surface
so no outside caller names a submodule path.

- `rows.rs` — the in-memory model. `enum AttractSettings { MovingBand(BandSettings),
  MovingText(TextSettings), Pixelate(PixelSettings) }` with `mode()`; `struct Favorite {
  id: FavoriteId, saved, settings }`; `struct FavoriteId(Uuid)` — **opaque**, private
  field, cross-module construction only through `#[cfg(test)] pub(super) const fn
  from_uuid_for_test`. `struct FavoriteRows { tables: Vec<Table>, recognitions:
  Vec<FavoriteRowRecognition>, additional_fields: Table }` holds the raw TOML tables
  alongside their typed interpretations, which is what makes persistence **lossless** —
  unknown tables and keys survive a save/delete round-trip. `refresh_recognitions` sorts
  the recognitions independently of the raw `tables` (attract-mode order, then newest
  first) and **demotes a duplicate-id row** to unrecognized. `push(&Favorite) ->
  FavoriteSaveOutcome { Added, Refreshed }` — `Refreshed` rewrites only the timestamp when
  an existing row's `settings` compare equal (the derived `PartialEq` on `AttractSettings`,
  the same equality the overlay's currency mark uses). `remove_recognized(FavoriteId)` and
  `remove_unrecognized(&locator) -> UnrecognizedFavoriteRemoval { Removed, LocatorStale }`.
- `enum FavoriteRowRecognition { Recognized(Favorite), Unrecognized { diagnostic:
  UnrecognizedFavoriteValue, removal_locator: UnrecognizedFavoriteRemovalLocator } }` — a
  **struct variant**, so every match site names its fields.
- `struct UnrecognizedFavoriteRemovalLocator { raw_table_index: usize, fingerprint:
  String }` — **the row-locator that survives concurrent edits.** `fingerprint =
  table.to_string()` (serialized text). `locate(tables) ->
  UnrecognizedFavoriteTableLocation { ExactlyOne(usize), NotExactlyOne }` trusts the
  recorded index while that table's serialized text still matches, else falls back to a
  content search that resolves **only when exactly one** table matches. Matching by text
  (not by parsed `toml::Table`) is deliberate: `toml::Value::Float` wraps `f64`, so `nan`
  never equals its own snapshot and `-0.0`/`0.0` compare equal — comparing text makes
  `nan` deletable and a genuine `-0.0→0.0` edit a refusal.
- `recognition.rs` — TOML recognition and serialization: `recognize_favorite(&Table) ->
  Result<Favorite, UnrecognizedFavoriteValue>`, the per-field `recognize_*` helpers, the
  `*_name` enum-spelling helpers, `table_from_favorite`, and `struct
  UnrecognizedFavoriteValue { key, spelling }` (the first field value that could not be
  read). (This module was extracted from `rows.rs`.)
- `file.rs` — everything touching the file. `enum FavoritesFileState { LocationUnavailable,
  Missing { path }, Loaded { path, rows: FavoriteRows }, Unparseable { path, error },
  Unreadable { path, error } }` (each state carries the path it concerns);
  `load() -> FavoritesFileState`; `push(AttractSettings) ->
  Result<FavoriteSaveOutcome, FavoritesMutationError>`; `remove(FavoriteRemovalTarget) ->
  Result<(), FavoritesMutationError>` — the **single** removal entry point. `enum
  FavoriteRemovalTarget { Recognized(FavoriteId), Unrecognized(locator) }`. All writes go
  through `acquire_lock` (advisory file lock, bounded retries) → `edit_at_location`
  (read-modify-write whose closure returns `Result`, so a refusal aborts before
  `atomic_replace` and leaves the file byte-identical) → `atomic_replace` (temp file +
  rename). `FavoritesMutationError::UnrecognizedFavoriteChanged` is the named refusal when
  a locator no longer identifies exactly one raw table; `favorite_refusal_message` /
  `FavoritesRetryInstruction` / `ResolvedBinding` build the user-facing recovery text
  (`ResolvedBinding::display_short()` returns `""` for an unbound key).

### Overlay — `cargo-tile/src/favorites_overlay/{mod,content,bindings,line_plan,parameter_column,table_layout,notice,pane,constants}.rs`

A directory module whose imports run one way (`content`/`bindings` import no siblings;
`line_plan` imports both; `mod` from all). The overlay performs **no file I/O** — it
emits removal outcomes that `terminal.rs` commits.

- `pane.rs` — `struct FavoritesOverlayPane` (`impl Pane<App>`, `impl Shortcuts<App>`), the
  modal action enum `FavoritesOverlayAction`, and `dispatch`.
- `mod.rs` — `struct FavoritesOverlay` and its lifecycle. `open(&keymap,
  current_parameters)` / `open_file_state(state, current_parameters, &keymap)` call
  `favorites::load`. `advance(now) -> FavoritesOverlayFrameOutcome { Quiet, Repaint,
  CommitRemoval(FavoriteRemovalTarget) }` performs no I/O. Two **deliberately separate**
  state fields: `FavoriteDeletionConfirmationState { NoConfirmationArmed,
  AwaitingSecondPress(FavoriteRowIdentity) }` (the two-press delete question) and
  `FavoriteRemovalCommitState { NoCommitPending, Pending(FavoriteRowIdentity) }` (a commit
  already in flight) — they clear on different events. `handle_unmapped_key()` routes any
  key the overlay does not map into cancelling the confirmation. `FavoritesOverlayCloseCommit`
  carries `removal_targets: Vec<FavoriteRemovalTarget>` out on close.
  `refresh_current_parameters(current_parameters)` re-snapshots after a resize and sets
  `CachedSurfaceWidth::NeedsRebuild`.
- `line_plan.rs` — the rendered plan and **row identity**. `enum FavoriteRowIdentity {
  Recognized(FavoriteId), Unrecognized(UnrecognizedFavoriteRemovalLocator) }` (round-trips
  to/from `FavoriteRemovalTarget`). `enum CachedOverlayLine { NonRow(Line), Row { identity,
  current_parameters, tail } }` — only `Row` lines carry identity, and only they enter
  `CachedLinePlan::selectable_line_index`, so the cursor cannot land on a blank or heading.
  `enum FavoriteSelection { NoRowSelected, Row(FavoriteRowIdentity) }`. `enum
  FavoriteRowCurrentParameters { Unrecognized, Different, Matching }` — the
  "matches-running-parameters" mark, decided **once per plan rebuild** in
  `build_line_plan(content, current_parameters, bindings, width, horizontal_page)` (never
  per frame). The private `enum FavoriteRowMarker { Neither, Selected, Current,
  SelectedAndCurrent }` fuses selection and currency into one value (they cannot disagree);
  `prefix()` returns the three-cell `"   "` / `"▸  "` / `" ● "` / `"▸● "`. `rendered_line`
  draws the marker and fades an unrecognized row from `error_color()` and a recognized one
  from `text_default()` toward `attract::ground()`.
- `content.rs` — `enum FavoritesOverlayContent { Rows(FavoriteRowsView), NoneSaved,
  OnlyUnrecognized(UnrecognizedFavoritesView), LocationUnavailable, Unparseable{..},
  Unreadable{..} }` (`From<FavoritesFileState>`); `FavoriteRowsView`/`FavoriteRowView`
  (settings + timestamp only, no rendered cells); `FavoriteRowLifecycle { Active, Removing
  { since } }` drives the removal fade.
- `bindings.rs` — the **capability-gated footer**. `enum SelectedFavoriteActions {
  NoFavoriteSelected, DeleteOnly, LoadAndDelete }` and the private `FavoritesFooterRequest`
  decide the segments; `refresh_footer(...)` rebuilds them into `CachedFavoritesFooter {
  NeedsRebuild, Current{..} }` and `footer(&self) -> &str` only reads (no per-frame
  `String`). Load is offered only for a recognized selection; delete for either kind.
- `parameter_column.rs` — `struct ParameterColumnDescriptor { heading, value_renderer:
  fn(AttractSettings) -> String }` reached through `render_value`. `BAND_COLUMNS`,
  `TEXT_COLUMNS`, `PIXEL_COLUMNS` each **pair a heading with its own column's renderer**,
  so reordering a table moves the value with the heading — no index-matched parallel array
  exists to fall out of step. `column_descriptors(mode)` selects the table.
- `table_layout.rs` — `FavoriteSectionTableLayout` (`measure`/`visible_parameter_columns`/
  `last_horizontal_column_page`), `format_table_line`/`format_table_tail`. Column budgets
  take the prefix width from `FAVORITE_ROW_PREFIX_WIDTH`, never a literal.
- `notice.rs` — `enum FavoritesOverlayNotice { NoNotice, DeletionRefused{..},
  DeletionConfirmation{..}, FavoriteAdjusted{..} }`; `favorites_heading(saved_count)`
  builds the border title ` Favorites -- N saved -- ● matches the current parameters `;
  `deletion_refusal_message` matches `FavoritesMutationError::UnrecognizedFavoriteChanged`
  **by variant** (not message text) and ends with `Close and reopen favorites, then try
  again.`.
- `constants.rs` — overlay-local layout constants incl. `FAVORITE_ROW_PREFIX_WIDTH: usize
  = 3` and `FAVORITE_REMOVAL_FADE: Duration = 400ms`.

### Snapshot of running parameters

`App`'s `AppOverlay::{Closed, Favorites(OpenFavoritesOverlayState)}` pairs the overlay with
`OpenFavoritesCurrentParameters` — a copy type wrapping `AttractSettings` with
`matches(attract_settings)` and `From<AttractSettings>`, so raw settings cannot be passed
where a snapshot is meant. Both open paths in `globals.rs` snapshot
`app.attract.current_settings().into()` before opening;
`terminal.rs::refresh_open_favorites_after_resize` re-snapshots once per coalesced resize
burst (attract parameters are re-clamped on resize).

### Save messaging

`globals.rs::save_favorite` renders one confirmation toast per `FavoriteSaveOutcome`:
`Favorite added` vs `Favorite refreshed`, each naming the mode.

### Framework shortcut hiding

cargo-tile installs a `FrameworkGlobalShortcutPresentation` (a `fn(GlobalAction) ->
FrameworkGlobalShortcutVisibility`) on its `Keymap` that hides `GlobalAction::Dismiss`
(`x`) from the compact shortcut popup while keeping it bound, dispatchable, and rebindable
in the full editor (`global_shortcut_rows` filters; `keymap_help_rows` does not).

### Apply and undo core — `attract/mod.rs`

`Attract::apply_settings(&mut self, requested: AttractSettings) -> SettingsApplicationOutcome`
is the **single point** at which a wholesale parameter replacement happens, and the **single
undo-capture site**. It sizes every animation (`size_all_animations`), captures the complete
configuration it is about to displace into
`ReplacementUndoState::Available(AttractConfigurationBeforeReplacement(self.configuration()))`,
writes `self.mode = requested.mode()`, then routes the requested set through the selected
animation's `apply`, reads it back, and answers `AppliedExactly` when `effective == requested`
or `AppliedWithAdjustments { requested, effective }` when the animation clamped a value. The
row on disk is never rewritten, so a value out of range on this terminal survives to a taller
one. Its three production callers — `randomize_from_seed` (`r`), `show_random_favorite_with`
(`m`), and the overlay's `enter`-load branch — each reach it only after their own candidate
has succeeded.

`AttractConfiguration { mode, band, text, pixels, presentation }` is the whole restorable
state; `AttractPresentation { visibility_instruction, grid_presentation }` is the durable
part that survives a replacement. Sizing precedes the capture on purpose: capturing first
would store parameters for modes never shown, hence never fitted to the terminal, and those
values are wrong the moment they are restored.

### Random favorite (`m`) — `globals.rs`

`AppGlobalAction::RandomFavorite` binds the bare `m` key in every scope.
`show_random_favorite` delegates to the deterministic `show_random_favorite_with(app, load:
impl FnOnce() -> FavoritesFileState, seed: impl FnOnce() -> u64)` (tests pass fixed closures;
production passes `favorites::load` and `random::clock_seed`). It loads the favorites file
**fresh on every press** — a favorite saved by another running instance is visible at once —
draws one recognized row uniformly through `draw_recognized_settings(rows, seed)` (a
`random::bounded_index` over `rows.recognized().count()`), applies it via
`Attract::apply_settings`, and calls `Attract::request_show()`. Any state that yields no
usable favorite — missing, unreadable, unparseable, unresolvable location, loaded-but-empty,
or loaded with no recognized row — instead opens the overlay at the matching diagnostic
through `FavoritesOverlay::open_file_state`, rather than doing nothing. An adjusted
application is reported through `favorites_overlay::report_closed_overlay_adjustment(&mut App,
SettingsApplicationOutcome)`: silent on `AppliedExactly`, a scheduled lowercase warning toast
otherwise. The clamp lands in the running attract state only and never rewrites the saved
file.

### Randomize (`r`) — `attract/mod.rs`

`AppGlobalAction::RandomizeAttract` binds the bare `r` key and calls `Attract::randomize()`,
which seeds from `random::clock_seed()` and defers to `randomize_from_seed(seed)`. That method
sizes every animation **before** drawing, picks a mode with `AttractMode::draw(seed)`, draws
that mode's parameters with `draw_random_settings(&self, mode: AttractMode, seed: u64)` (each
animation's own `random_settings(seed)`), applies the drawn set through `apply_settings`
(which sets `self.mode`), and `request_show()`s. `AttractMode` owns the selection: `ALL:
[Self; 3]`, `INDEX_BOUND: NonZeroIndexBound` (a `const` whose `Err` arm is a `panic!`, so an
empty mode list is a compile error), and `draw(seed) -> Self` maps a `random::bounded_index`
draw through a total `match` mirroring `ALL` positionally. Because sizing precedes the draw,
`tui_pane`'s `random_settings` bounds each parameter by the real terminal and a narrow window
can never be handed a band wider than itself; the outcome is bound and `debug_assert_eq!`'d
against `AppliedExactly` (an adjusted draw would be a bug, and a `debug_assert` around the
call itself would compile the call out of release, so the outcome is bound first).

### Undo (`u`) — `globals.rs`, `attract/mod.rs`

`AppGlobalAction::UndoAttractReplacement` binds the bare `u` key; `globals.rs`'s free
`undo_attract_replacement` calls
`Attract::restore_configuration_before_last_replacement() -> AttractConfigurationRestoreOutcome`
and renders one toast per arm. Restore **consumes** the checkpoint
(`mem::replace(.., ReplacementUndoState::Unavailable)`), so a second `u` reports nothing to
undo; it calls neither `apply_settings` (which would capture a fresh checkpoint and make `u`
its own undo target) nor `request_show`. It sizes every animation, applies the three captured
parameter sets through their own `apply` methods, and writes `mode`, `visibility_instruction`,
and `grid_presentation` back — restoring the screen the viewer was actually looking at, not
only its parameters. `ReplacementUndoState` is `Unavailable` or
`Available(AttractConfigurationBeforeReplacement)`. The outcome is `NothingToUndo`,
`RestoredExactly { mode }`, or `RestoredWithAdjustments { mode, adjusted_parameter_sets }`,
where `adjusted_parameter_sets: AdjustedAttractParameterSets` enumerates the **seven** nonempty
combinations of the three parameter sets (with `.names()`), so "restored with adjustments,
nothing adjusted" is unrepresentable.

The presentation values undo restores are `AttractVisibilityInstruction { FollowRoster, Show,
Hide }` (the reader's standing instruction, which outranks the roster) and
`AttractGridPresentation { OverGrid, ReplacesGrid }` (whether the strip covers or replaces the
grid), grouped as `AttractPresentation`. They replaced a former `Asked` enum plus a `covering:
bool`: two enums that name every state, so "not asked for" and "asked to hide" are no longer
one value. `request_show()` — called by `m` and `r`, never by undo — sets `Show` +
`ReplacesGrid` (idempotent, reverses a fade-out); `asked_for()` and `keyed_mode()` read
`visibility_instruction`.

### Random source — `random.rs`

`cargo-tile/src/random.rs` is the crate's only source of randomness for app-owned choices:
dependency-free SplitMix64 plus **rejection sampling** (uniform, not modulo-biased). Its three
`pub(crate)` entry points are `clock_seed() -> u64` (nanoseconds since the Unix epoch),
`NonZeroIndexBound::try_from_len(len: usize) -> Result<NonZeroIndexBound, EmptyIndexDomain>`,
and `bounded_index(seed: u64, bound: NonZeroIndexBound) -> usize`. The empty-list case is
unreachable inside the draw — it is decided once, by the caller, through the fallible
`try_from_len` constructor (`AttractMode::INDEX_BOUND` resolves it at compile time; `m`
resolves it against the recognized-row count). This is **distinct** from `tui_pane`'s
`backdrop/random.rs` `Xorshift`, which is `pub(super)` inside the backdrop feature and drives
each animation's `random_settings`; the two generators never mix.

### Attract settings snapshot API — `tui_pane` backdrop

The parameters favorites persist, and the parameters randomize draws, come from `tui_pane`'s
backdrop animations. `TravelingBand`, `DriftingText`, and `ResolvingPixels` each carry
`settings(&self) -> <T>Settings` (`const`), `apply(&mut self, settings: <T>Settings)`, and
`random_settings(&self, seed: u64) -> <T>Settings`. The snapshots are `BandSettings {
direction: BandDirection, width: u32, speed: u32, tail_speed: u32, fraying: BandFraying }`,
`TextSettings { direction: BandDirection, speed: u32, spread: u32, drift: TextDrift, fill:
TextFill }`, and `PixelSettings { direction: BandDirection, speed: u32, wave_percent: u32,
block_columns: u32, resolve: PixelResolve, fill: PixelFill }` — public structs with public
fields in `backdrop/{band,text,pixels}.rs`, re-exported from `tui_pane` under the `backdrop`
feature, all deriving `Clone, Copy, Debug, Eq, PartialEq`. A snapshot holds only what a key
steers, never runtime state, and `apply` is a semantic transition routing every field through
the private absolute clamp setters, so it can **silently clamp** an out-of-range value — which
is why the load path reports whether a favorite `AppliedExactly` or `AppliedWithAdjustments`.
`random_settings` takes `&self` so a bounded draw uses the animation's live extent, which is
why the sizing pass must run first. cargo-tile's `AttractSettings { MovingBand(BandSettings),
MovingText(TextSettings), Pixelate(PixelSettings) }` (see the data model above) is the tagged
union over the three, with `AttractSettings::mode() -> AttractMode` reading the mode off the
variant.

---

## Invariants

1. **Favorites persistence is lossless.** Unknown `[[favorite]]` tables and unknown keys
   survive save and delete; `FavoriteRows` keeps the raw `Table`s beside the typed
   recognitions and a refused mutation leaves the file byte-identical.
2. **`FavoriteId` is opaque outside `favorites`.** Anything locating a row by id goes
   through `rows.rs`. An unrecognized row has no id — it is identified by
   `UnrecognizedFavoriteRemovalLocator` (text fingerprint), which is why the running-params
   mark and the row cursor key off `FavoriteRowIdentity`, not `FavoriteId`.
3. **Deletion is two-press with any-key cancel, and the overlay writes no files.** The
   confirmation state and the in-flight commit guard are separate fields; the overlay
   emits `CommitRemoval`/`removal_targets` and `terminal.rs` commits both recognized and
   unrecognized rows.
4. **The `save`-dedup equality and the "matches running parameters" equality are the same**
   derived `PartialEq` on `AttractSettings`. They must keep agreeing — no second
   comparison key, normalization, or epsilon.
5. **Sizing precedes every parameter read, draw, and capture.** `apply_settings` and
   `randomize_from_seed` size every animation before touching parameters, so
   `random_settings` is bounded by the real terminal and a captured configuration holds
   values that were fitted to it — never a band wider than its window, never a mode never
   shown and therefore never sized.
6. **`apply_settings` is the single wholesale-replacement point and the single undo-capture
   site.** `r`, `m`, and `enter` all replace through it, and it captures the displaced
   configuration into `ReplacementUndoState` exactly once, so undo has one checkpoint and an
   adjustment is reported once. `restore_configuration_before_last_replacement` deliberately
   does **not** go back through it.
7. **`random.rs` is the crate's only randomness, and it is dependency-free.** App-owned
   choices — `m`'s row, `r`'s mode and its parameters — compose `bounded_index(clock_seed(),
   bound)`; no second generator or `random` crate is added. `tui_pane`'s backdrop `Xorshift`
   is a separate generator behind the backdrop feature and never mixes with this one.
8. **`AttractVisibilityInstruction` and `AttractGridPresentation` are the non-`Option`,
   non-bool successors to the old `Asked` enum + `covering: bool`.** Every attract
   visibility/grid state is a named variant; nothing reads presentation as a boolean or an
   absent option, and undo restores both from the captured `AttractPresentation`.

---

## Calibration / gotchas

- **Locator identity is serialized text, not parsed values** (see invariants 1–2
  rationale): the ambiguity scan belongs *only* on the content-search fallback — running it
  while the recorded index still holds the expected row would make two byte-identical broken
  rows block each other forever.
- **`ResolvedBinding::display_short()` returns `""` for an unbound key**, so footer/notice
  text built from a binding must match `ResolvedBinding::Bound` first and a paired hint
  (move, page) needs both halves bound.
- **The ~57-cell favorites border title truncates below ~61 columns**, and the `● matches
  the current parameters` legend is the first thing lost.

---

## Why

- **The row-locator design.** An unrecognized row has no id, and the file can change under
  the user between listing and deletion. A recorded index gives O(1) deletion in the common
  case; a serialized-text fingerprint re-verifies that the row is still the one the user
  saw and refuses (rather than deletes wrongly) when it is not; matching by text rather than
  parsed floats is the only way `nan` rows are deletable and `-0.0→0.0` edits are caught.
- **Row identity and capability-gated footer.** Rows carry `FavoriteRowIdentity` and only
  real rows are selectable, so the cursor cannot land on a heading and delete/load offer
  only what the current selection can actually do — the UI cannot present an action that
  will fail.
- **One apply point.** Routing every wholesale replacement through `Attract::apply_settings`
  gives undo a single capture site — no per-caller checkpoint to drift — and reports an
  adjustment once, through one formatter. The alternative was three copies of the same
  checkpoint at three call sites, each able to fall out of step; and restoring through
  `apply_settings` would capture a fresh checkpoint from the undo itself, making `u` its own
  undo target.
- **A dependency-free RNG.** `m` and `r` need only a uniform bounded draw at a handful of
  call sites; a `random` crate for that is unwarranted. SplitMix64 with rejection sampling is
  small, unbiased, and deterministic under a fixed seed, so tests pin the exact draw and the
  empty domain is ruled out by construction rather than a runtime branch.
- **Named visibility/grid enums over `Asked` + `covering: bool`.** The strip comes on by
  itself over an idle grid, so "not asked for" and "asked to hide" are opposite intentions
  that a single bool or `Option` collapses into one value — which once left `a` unable to
  dismiss the strip at the moment it was being watched. Two enums that name every state let a
  restored undo return the exact screen the viewer had, not just its parameters.
