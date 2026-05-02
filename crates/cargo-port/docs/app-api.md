# Splitting App's API into subsystems

## Problem

`App` is a god object: ~60 fields, owns everything, every sibling impl-file
under `src/tui/app/` reaches into its private guts directly. That's what forces
the **385 `pub(in super::super)`** sites in the codebase — 269 in `app/`, 43 in
`panes/`, the rest scattered.

The smell isn't a file-location problem. The smell is the **API surface between
`App` and `panes/` (and between `App` and its own impl-files)**. Narrow that
surface and the visibility annotations collapse.

## Strategy

Carve App's fields into a handful of **owned subsystems**, each with a small
public method set. App's impl-files and `panes/` then talk to the subsystem
instead of poking App's fields. Internals of each subsystem become `pub(super)`
within that subsystem's module — not `pub(in super::super)`.

## Subsystems (proposed)

| Subsystem | Owns (App fields it absorbs) | Public API surface (~5–8 methods) |
|---|---|---|
| **Panes** (`tui::panes::Panes`) | Phase 1 absorbed: `pane_manager`, `pane_data`, `visited_panes`, `layout_cache`, `worktree_summary_cache`, `hovered_pane_row`, `ci_display_modes`, `cpu_poller`. Phases 7–10 dissolve the grab bag: per-pane state moves onto each pane struct (cursor/scroll/hover/visited via embedded `Viewport`; `cpu_poller` to `CpuPane`; `ci_display_modes` to `CiPane`); `layout_cache` moves to App-shell at Phase 10; `Panes` ends up as the registry of per-pane structs. | Phase 1: facade methods. Phase 7+: `pane(id) -> &dyn Pane`, `pane_mut(id) -> &mut dyn Pane`, plus typed accessors per pane. |
| **Selection** (`tui::selection::Selection`) | `cached_visible_rows`, `cached_root_sorted`, `cached_child_sorted`, `cached_fit_widths` (renamed `ProjectListWidths` in Phase 2), `selection_paths`, `selection`, `expanded`, `finder` | Direct: `visible_rows`, `cursor_row`, `move_cursor`, `select_path`, `fit_widths`, `selected_paths`, `mutate(&projects) -> SelectionMutation<'_>`. On `SelectionMutation`: `toggle_expand`, `apply_finder` (recompute fires on drop). |
| **Background** (`tui::background::Background`) | All four mpsc tx/rx pairs (`bg_*`, `ci_fetch_*`, `clean_*`, `example_*`), `watch_tx` | `poll_all -> PendingMessages`, `send_watcher`, `spawn_clean`, `spawn_ci_fetch`, `spawn_example`, `bg_sender`, `swap_bg_channel(new_tx, new_rx)` (called by rescan) |
| **Inflight** (`tui::inflight::Inflight`) | `running_clean_paths`, `running_lint_paths`, `clean_toast`, `lint_toast`, `ci_fetch_toast`, `ci_fetch_tracker`, `pending_cleans`, `pending_ci_fetch`, `pending_example_run`, `example_running`, `example_child`, `example_output`, **`lint_runtime`** (relocated here from Background — `start_lint` is the only consumer; co-locating runtime with start avoids cross-subsystem reach) | `start_clean(path, ctx)`, `finish_clean`, `start_lint(path, ctx)`, `finish_lint`, `start_ci_fetch`, `finish_ci_fetch`, `start_example`, `kill_example`, `is_clean_running`, `is_lint_running`, `queue_clean`, `drain_next_pending_clean`, `respawn_lint_runtime(&LintConfig)` (just the runtime respawn — full lint-config-change handling is orchestrated by App, see below). `ctx` here is a small struct `StartContext<'a> { toasts: &mut ToastManager, config: &CargoPortConfig, background: &mut Background, scan: &Scan }` — the actual dependency surface, named once instead of per-method. |
| **Config** (`tui::config_state::Config`) | `current_config`, `config_path`, `config_last_seen`, `settings_edit_buf`, `settings_edit_cursor` (last two combined as `SettingsEditBuffer`) | `current`, `try_reload(&mut toasts) -> ReloadOutcome`, `begin_settings_edit -> &mut SettingsEditBuffer`, `commit_settings_edit(&mut toasts) -> CommitOutcome`, `discard_settings_edit` |
| **Keymap** (`tui::keymap_state::Keymap`) | `current_keymap`, `keymap_path`, `keymap_last_seen`, `keymap_diagnostics_id` | `current`, `try_reload(&mut toasts) -> ReloadOutcome` |
| **`WatchedFile<T>`** (`tui::watched_file`, generic primitive) | `path`, `stamp`, `current: T` | `current`, `path`, `try_reload(parse_fn) -> ReloadOutcome` where `parse_fn: impl FnOnce(&[u8]) -> Result<T, String>` and `ReloadOutcome = enum { Unchanged, Reloaded, Failed(String) }` — **non-generic**, error stringified at the parse boundary. App's tick polls Config and Keymap side-by-side and surfaces `Failed(msg)` as a toast uniformly. Composed by both `Config` and `Keymap`; not held by App directly. |
| **Scan** (`tui::scan_state::Scan`) | `projects`, `scan`, `dirty`, `data_generation`, `discovery_shimmers`, `pending_git_first_commit`, `metadata_store`, `target_dir_index`, `priority_fetch_path`, `confirm_verifying`, `lint_cache_usage` | Direct: `projects`, `generation`, `metadata_store`, `target_dir_index`, `set_priority_fetch`, `shimmer_for`, `register_shimmer(path)`, `mark_dirty`, `apply_metadata`, `record_first_commit`, `set_confirm_verifying`, `bump_generation` (explicit, called from message-relevance code), `lint_cache_usage`. **`mutate_tree` stays on `App`**, not on `Scan`, so it can split-borrow App's disjoint fields (`let App { scan, panes, selection, .. } = self;`). The guard's `Drop` fans out to Scan's caches + `panes.clear_for_tree_change()` + `selection.recompute_visibility()`. See "Worked example" in design notes. |

### `Background` channel-rescan caveat

The `bg_tx`/`bg_rx` channel pair is **replaced wholesale on every rescan**
(today via `App::rescan` — `tui/app/async_tasks.rs:~1391`). The other three
channel pairs are not. Bundling all four into `Background` requires either:

- a `swap_bg_channel(new_tx, new_rx)` method on `Background` that the
  rescan path calls, or
- a sub-struct `ScanChannel { tx, rx }` inside `Background` that
  `swap_bg_channel` mutates.

The rename does not eliminate this asymmetry — `ci_fetch_*`, `clean_*`,
`example_*` outlive any single rescan; `bg_*` does not. Plan to keep the
swap method explicit so the lifecycle difference stays visible in the
type, rather than getting smoothed over.

### `Inflight::StartContext` parameter cluster

`Inflight::start_*` methods take a `StartContext<'_>` struct rather than a
list of bare references. Reason: the existing flows in
`tui/app/async_tasks.rs` need `&mut ToastManager` (toast composition),
`&CargoPortConfig` (clean linger seconds), `&mut Background` (push to
spawn channel), and `&Scan` (project lookup for toast labels). Naming the
cluster once keeps the method signatures readable and gives
implementation a single place to add a dependency if a future start path
needs more.

App keeps only:

- the **event loop** (tick, draw, input dispatch)
- the **focus stack** (`focused_pane`, `return_focus`) — popups, toasts, and
  panes all push/pop focus, so this is an App-shell concern. `Panes` is told
  who to highlight via `render(frame, focused)`.
- the **modal/UI shell** (`confirm`, `toasts`, `inline_error`, `status_flash`,
  `ui_modes`, `mouse_pos`, `animation_started`)
- **handles** to the eight subsystems above (`panes`, `selection`,
  `background`, `inflight`, `config`, `keymap`, `scan`, `net`)

That's roughly 12 fields instead of 60 after phases 1–11. After
Phase 10 (before Net carves), the count is ~16 — the difference is
the network state cluster, which moves out in Phase 11.

### Two axes of structure inside `Panes`

- **App → `Panes` boundary**: strict delegation, no trait. Single owner, single
  caller, concrete struct. `app.panes: Panes`.
- **`Panes` → individual pane behavior**: a `Pane` trait, with each
  pane owning its own state. Phase 1 absorbed the field cluster as a
  grab-bag struct. Phases 7–10 dissolve the grab bag and rebuild
  `Panes` as a registry of per-pane structs that each own their own
  state (cursor/scroll/hover/visited/content/extras). Common behavior
  is the `Pane` trait with default methods backed by an embedded
  `Viewport`. The original Phase 1 → single-Phase-7 plan was
  reframed during the Phase 7 re-review; see the Phase 7 design
  notes for the architectural model and the eight invariants that
  govern Phases 7–10.

  By Phase 7 all the other subsystems exist as proper types, so trait
  methods can take typed subsystem references via the
  `PaneRenderCtx`/`PaneInputCtx`/`PaneNavCtx` bundles. The bundles
  are dependency injection, not a god-object handle — encapsulation
  by file (each pane's behavior in one place named for the pane) is
  the win.

  The Phase 1 grab-bag absorption, the Phase 7 foundation, and the
  Phase 8/9 per-pane migrations are the four halves of the same fix.
  The plan does all of them.

## Visibility math after this

- **App's impl-files** call `app.panes.foo()`, `app.selection.bar()`, etc. They
  no longer need access to App's private fields, so most `pub(in super::super)
  fn` on App collapses to `pub(super)` (or moves into the subsystem and
  disappears).
- **`panes/` internals** stop being reached into by App directly. Everything
  inside `panes/` becomes `pub(super)` because the only outside caller is
  `PaneSystem`'s public facade.
- **Cross-cutters in `panes/`** (`data.rs`, `support.rs`, `layout.rs`,
  `spec.rs`, `actions.rs`) become private siblings of `PaneSystem` —
  `pub(super)`, not `pub(in super::super)`.

Rough estimate: **385 → ~80–120** remaining `pub(in super::super)` sites
(not 385 → 30). Calibration:

- Some annotations exist because they're called from `tui::input`,
  `tui::render`, etc. — *siblings* of `app/`, not children of it. Those
  don't collapse to `pub(super)` after the carve; they stay `pub(super)`
  on the new subsystem (which is the same scope as `pub(in super::super)`
  was on App, just one level shallower).
- Real wins are concentrated where impl-files inside `app/` itself
  reach into App's fields — that's where the bulk of 269 lives. The
  remainder will still need to expose facade methods cross-module.

The 385 number is a directionally-useful pressure metric, not a target
to hit.

## Order of execution (one PR per subsystem)

1. **Panes** first — biggest visibility win in `panes/`, and the fix the
   prior conversation has been pointing at.
2. **`ColumnWidths` primitive + two adopters** — extract a generic
   "fit columns to content with min-width-per-column" helper into a new
   submodule `tui::columns::widths`, and adopt it in **two** existing
   places that currently open-code the same pattern:
   - the project list (`ResolvedWidths` becomes `ProjectListWidths`,
     a wrapper around the new `ColumnWidths`),
   - the CI pane (`build_ci_widths` in `panes/ci.rs:120` collapses
     into a few calls into `ColumnWidths`).

   The primitive ships paired with adoption that proves it works for
   both consumers — not as speculative infrastructure. Lints / package /
   git panes can adopt later if their column logic grows; today they
   don't fit content-aware widths, so they're not on the hook for this
   phase.
3. **Selection** — second-biggest field cluster (`cached_*`,
   `selection*`). `cached_fit_widths` is absorbed as the
   `ProjectListWidths` introduced in Phase 2.
4. **Background + Inflight** together — entangled (a "start" hits both).
5. **Config + Keymap** (one phase) — extract shared `WatchedFile<T>`
   primitive, then carve `Config` and `Keymap` as two separate subsystems
   composing it. Tightly coupled by the primitive; extracting one without
   the other leaves the duplication in place.
6. **Scan** — `mutate_tree` already gates it; mostly relocation.
7. **Pane trait + foundation** — introduce the `Pane` trait, the
   `Viewport` shared-state primitive, and the registry. Replace
   `match PaneId` dispatch in `render.rs` / `input.rs` with trait
   dispatch through the registry. Skeleton `impl Pane` blocks for all
   13 panes call into the existing free functions for now — per-pane
   bodies don't move yet. Delete the `PaneBehavior` enum (its dispatch
   uses are subsumed by the trait). Repurpose the existing `Panes`
   struct as the registry: same name, gutted of its grab-bag fields
   (those move in Phases 8–9), holds only the per-pane structs as
   named fields plus `pane(id) -> &dyn Pane` / `pane_mut` dispatch
   methods.

   **The architectural model** that drives Phases 7–10 is captured in
   the Phase 7 design-notes section as eight invariants. Read those
   first. The short version: each pane is a struct that owns its own
   state (cursor, scroll, hover, visited, content, pane-specific
   extras); common behavior is a trait with default methods backed by
   an embedded `Viewport`; outsiders read pane state through methods,
   never through field access; there is no central pane-state grab
   bag.

   **Phase 7 begins with a re-review of the phase plan against
   everything learned in Phases 1–6.** That re-review has run; the
   plan below is its output.
8. **Migrate the 6 detail/data panes to own their state and bodies** —
   `CiPane`, `CpuPane`, `GitPane`, `LintsPane`, `PackagePane`,
   `LangPane`. Each pane gains a `Viewport` field, a content slot
   (replacing its slot in the central `PaneDataStore`), and any
   pane-specific extras (`CpuPane` absorbs `cpu_poller`; `CiPane`
   absorbs `ci_display_modes`). Render and input bodies move from
   free functions in `panes/*.rs` and `panes/actions.rs` into trait
   methods. Shared input-handling bodies (Up/Down/Home/End nav,
   detail-pane keymap dispatch) ride as default helper methods on
   the `Pane` trait itself — no sub-traits. Each pane's
   `handle_input` body opts in to whichever helpers it needs.
9. **Migrate the remaining 7 panes** — `ProjectListPane`,
   `TargetsPane`, `OutputPane`, `ToastsPane`, `SettingsPane`,
   `FinderPane`, `KeymapPane`. `ProjectList`'s ~250-line render body
   moves out of `render.rs` into `panes/project_list.rs`. The overlay
   panes (Settings/Finder/Keymap) get the same trait treatment; their
   special handling for App-shell modal mode stays where it is. After
   Phase 9 the central `PaneDataStore` and the free-function dispatch
   in `panes/actions.rs` are entirely gone.
10. **Hit-test promotion + final cleanup** — promote hit-testing from
    a render side-effect to a `Pane::hit_test` trait method. Pull the
    `register_*_row_hitboxes` helpers out of render bodies; render
    becomes pure(r). Mouse handling switches from looking up a side
    dictionary in `pane_manager` to calling `pane.hit_test(row)`.
    Any residual fields on `Panes` that the per-pane migrations left
    behind get their final home (App-shell or pane-local).
11. **Lint subsystem** — extract the lint-state cluster off App into
    its own subsystem. Today the lint runtime, run history, status
    cache, icon helpers, trigger plumbing, and `is_rust_at_path` /
    `lint_icon` / `selected_lint_icon` accessors are all directly on
    App. After Phase 11, App stops owning any lint-related state
    directly; readers go through `App::lint() -> &Lint`.

    Lint depends on Background+Inflight (Phase 4 — done; lint runs
    spawn through it), Config (Phase 5 — done; lint config is a
    `WatchedFile`), and Scan (Phase 6 — done; project/path lookups).
    No Net dependency — lint runs are local cargo invocations.

    **Phase 11 begins with a re-review of the phase plan against
    everything learned in Phases 1–10.** Output of the design phase:
    field-cluster list, public API, and the `LintDisplay` typed enum
    (see "Typed display values, not pre-rendered strings" in
    Recurring patterns). The carve sub-phases land the fields and
    the API together, including `Lint::package_display(path) ->
    LintDisplay` for the Package pane row. The typed enum is a
    non-negotiable carve deliverable, not a follow-up. Get user
    approval on the design before writing carve code.

12. **Net subsystem** — extract the network-state cluster (`http_client`,
    `github`, `crates_io`) into its own subsystem. Today these three
    fields together carry the HTTP client and rate-limit state, the
    GitHub repo-fetch cache plus in-flight fetch tracking plus
    availability tracker, and the crates.io availability tracker. They
    share the HTTP client, are read by the Git pane, the project tree,
    and the rate-limit display, and overlap with two other subsystems
    (Inflight and Scan). After Phase 12, App stops owning any
    network-related state directly.

    **Phase 12 begins with a re-review of the phase plan against
    everything learned in Phases 1–11.** The skeleton in this doc was
    drafted before any of the prior phases existed; the actual
    `running_fetches` / `fetch_cache` overlaps with Inflight and Scan
    may have resolved themselves along the way, the `availability`
    tracking may have moved, and the public API drafted below is a
    starting point. Update this section and get user approval before
    writing carve code.

13. **Ci subsystem** — extract the CI-state cluster off App into its
    own subsystem. Today the CI run cache, workflow detection, branch
    publication state, and `ci_for` / `ci_for_item` accessors are
    directly on App; the CI display strings on `PackageData` funnel
    through ~6 of these. After Phase 13, App stops owning any
    CI-related state directly; readers go through `App::ci() -> &Ci`.

    Ci depends on Background+Inflight (Phase 4 — done), Scan (Phase 6
    — done), **and Net (Phase 12)**. Ci must come after Net so it
    can compose `Net`'s HTTP client + GitHub fetch cache cleanly,
    rather than reaching into App for HTTP state and re-plumbing
    after Net carves.

    **Phase 13 begins with a re-review of the phase plan against
    everything learned in Phases 1–12.** Output of the design phase:
    field-cluster list, public API, and the `CiDisplay` typed enum.
    The carve sub-phases land the fields and the API together,
    including `Ci::package_display(path) -> CiDisplay`. As with
    Phase 11, the typed enum is a non-negotiable carve deliverable.

    **Phase 13.last (capstone, lands inside Phase 13)** — once both
    `Lint::package_display` and `Ci::package_display` exist, flip
    `PackageData.lint_display` and `PackageData.ci_display` from
    `String` to the typed `LintDisplay` / `CiDisplay` enums. Update
    the Package renderer in `panes/package.rs` to match on enum
    variants instead of string-comparing constants like
    `NO_LINT_RUNS`, `NO_CI_WORKFLOW`. Delete `resolve_lint_display`
    and `resolve_ci_display` from `panes/support.rs`. Pure
    consumer-side cleanup gated by both subsystems being done.

**(History: an earlier draft collapsed the `ColumnWidths` primitive
into Selection, on the argument that the project list was the only
consumer. That was wrong — `panes/ci.rs:120 build_ci_widths` already
open-codes the same fitting pattern. Phase 2 is restored as a real
phase that ships the primitive and adopts it in both places.)**

**(History: Phase 7 was added after a directive that the per-pane
god-object problem must be solved, not deferred. Phase 1 absorbs the
field cluster; Phase 7 finishes the job by giving each pane its own
implementation block.)**

**(History: Phase 8 was added after a directive that the network
state cluster must be properly separated, not left as residual
App-shell hand-waving. Earlier drafts called it a "deferred sketch"
that this plan would not address; that punted the same god-object
problem one cluster down. The plan now finishes the carve. After
the Phase 7 redesign that split the original Phase 7 into four,
the Net subsystem moved from Phase 8 to Phase 11.)**

**(History: Phase 7 was originally a single phase introducing a
`Pane` trait and migrating six per-pane bodies into trait impls.
The Phase 7 re-review reframed the work around per-pane ownership:
each pane is a struct that owns all of its own state (cursor,
scroll, hover, visited, content, pane-specific extras), accessed
by outsiders only through methods, with common behavior on a
trait. The original Phase 1 `Panes` struct turned out to be a
grab bag of unrelated state and is dissolved over Phases 7–9.
The reframe split Phase 7 into four phases: Phase 7 (foundation:
trait + Viewport + registry + skeleton impls), Phase 8 (migrate
six detail/data panes), Phase 9 (migrate the remaining seven
panes), Phase 10 (hit-test promotion + final cleanup). The
network carve was renumbered from Phase 8 to Phase 11.)**

**(History: Phases 11 and 13 (Lint and Ci subsystems) were added
after a Phase 8.14 review of the Package pane's `lint_display` /
`ci_display` resolution surfaced a smell — producing those two
display strings funneled ~6 App methods through two free helpers
in `panes/support.rs`, with `String` as the carrier type. The
right fix was to move the resolution behind subsystems that own
the lint and CI domain state, and to replace the strings with
typed enums (`LintDisplay`, `CiDisplay`). Net was renumbered
from 11 to 12. Ci sits after Net because it composes `Net`'s
HTTP client and fetch cache. The typed enums and the
`PackageData` capstone (Phase 13.last) are contracted into the
carve phases, not deferred as follow-ups, so the typed-display
intent cannot drift back to `String` fields.)**

### Per-phase workflow (applies to every step above)

Each step is its own implementation phase, executed and shipped before the
next one starts:

1. **Write tests first** — characterization tests for the App fields/methods
   being moved (so we can prove behavior is preserved), plus new unit tests
   for the subsystem's facade.
2. **Develop** — carve the subsystem, route App's impl-files through the new
   facade, collapse visibility annotations.
3. **Validate** — `cargo nextest run`, `/clippy` (mend + style review +
   clippy), manual TUI smoke check for the user-visible behavior touched by
   that subsystem.
4. **Commit** — single conventional commit per phase (or a small series if
   the carve is genuinely separable). Push and confirm CI green before
   starting the next phase.

No phase begins until the prior phase is committed and green.

## What this is *not*

- Not a rewrite. Each subsystem is a `mv` of fields + methods into a new
  module, plus a facade method per call site.
- Not a behavior change. No new caches, no new threads, no user-visible API
  changes.
- Not a `panes/` reorg. Files stay where they are; only the **boundary
  between App and `panes/`** moves.

## Per-step design notes

Notes captured during walkthroughs that affect implementation but aren't
captured in the tables above. Items that were merely "decided" (naming,
sequencing, scope) are not retained — once decided, they live in the tables
and the step list, not here.

- **Phase 1 (Panes)**:
  - Phase 1 absorbs the field cluster only. The per-pane trait split
    happens in Phases 7–10 — Phase 7 lays the foundation (trait,
    `Viewport`, registry); Phases 8–9 migrate per-pane bodies and
    state; Phase 10 promotes hit-testing and finishes cleanup. See
    "Two axes of structure inside `Panes`" above and the Phase 7
    design-notes section below.
  - `hovered_pane_row` lives in `Panes` after Phase 1. Hit-testing in
    Phase 1 is a match-on-`PaneId` inside `Panes`; promotes to
    `Pane::hit_test` trait dispatch in Phase 10. The hovered-row
    field itself is absorbed into each pane's `Viewport.hovered_row`
    during Phases 8–9.
  - `Panes::handle_input` in Phase 1 keeps the `&mut App` signature that
    `panes/actions.rs` currently uses (every dispatch in
    `panes/actions.rs:32-336` reaches across ~6 of the 7 future
    subsystems). Phase 7 introduces `Pane::handle_input` on the
    trait with skeleton impls calling the existing free functions;
    Phases 8–9 move the actual bodies into trait impls, at which
    point `panes/actions.rs` as a free-function module ceases to
    exist.
  - `apply_hovered_pane_row` (`tui/app/mod.rs:278-286`) moves wholesale
    into `Panes` — it reads `hovered_pane_row` and writes `pane_manager`,
    both of which become Panes-internal. Canonical example of "method
    that disappears from App."
  - **Phase-1 → Phase-5 staging.** `Panes::clear_for_tree_change()` lands
    in Phase 1 and is **called from App** (from the existing
    `TreeMutation::drop`) until Phase 6 wires it into the new fan-out
    `TreeMutation::drop`. Without this temporary call, Phase 1 would
    orphan `worktree_summary_cache` invalidation.

- **Phase 2 (`ColumnWidths` primitive + two adopters)**:
  - Generic `ColumnWidths` lands in a new submodule `tui::columns::widths`.
    Shape: `pub struct ColumnWidths { specs: Vec<ColumnSpec>, widths:
    Vec<u16> }` where `ColumnSpec` carries a header-label minimum and
    optional max. API: `new(specs)`, `observe_cell(col, width)` (grow
    column to fit content), `widths() -> &[u16]`, `into_constraints() ->
    Vec<Constraint>` (for ratatui).
  - **Adopt in the project list.** `ResolvedWidths` becomes
    `ProjectListWidths` — a thin wrapper holding `ColumnWidths` plus the
    project-list helpers (`row_to_line`, `header_line`,
    `build_summary_cells`). `summary_label_col` stays a free function in
    `tui::columns` (likely reused by future summary rows).
  - **Adopt in the CI pane.** `panes/ci.rs:120 build_ci_widths` is
    open-coded today — `ci_runs.iter().map(|r| r.branch.len()).max()`,
    `commit_title.len()` max, header-label minimums. Replace with
    `ColumnWidths` calls; delete the manual fitting.
  - **Out of scope this phase.** Lints pane (uses fixed-length
    constraints today), package pane (`kind_col_width` is a static
    label-width lookup, not content-aware fitting), git pane. They can
    adopt later if their column logic grows.
  - **Why this is a real phase.** The reusable primitive ships paired
    with the two consumers that prove it works for both forms
    (project-list rows + CI runs). Not speculative infrastructure.

- **Phase 3 (Selection)**:
  - `cached_fit_widths` is absorbed under the new name `ProjectListWidths`
    (a rename of `ResolvedWidths` — no generic split this phase, see
    phase ordering).
  - `expanded` and `finder` *state* live in Selection; the finder *modal mode*
    (whether finder owns input) stays in App's `ui_modes`.
  - `SelectionSync` stays internal to Selection — only `cursor_row()` is
    observable.
  - **Mutation guard for visibility-changing ops.** Same RAII pattern as
    `TreeMutation`. Visibility-changing methods (`toggle_expand`,
    `apply_finder`) are not callable on `Selection` directly — only via
    `selection.mutate(&projects) -> SelectionMutation<'_>`. The guard's
    `Drop` calls `recompute_visibility`. Cursor moves (`move_cursor`,
    `select_path`) stay direct since they don't change visibility. Result:
    the type system makes it impossible to mutate visibility-affecting
    state without triggering recompute.
  - **TreeMutation interaction.** When the project tree changes,
    `TreeMutation::drop` (in App) also has to trigger Selection's recompute.
    It does so by calling `selection.recompute_visibility(&projects)`
    directly — a `pub(super)` method, not part of the user-facing mutation
    API. The `SelectionMutation` guard exists to prevent *forgetting*
    recompute when calling Selection's own visibility-changing methods;
    `TreeMutation` is the orthogonal case (tree changed externally), so it
    invokes recompute explicitly.
  - `recompute_visibility` takes `&projects: &ProjectList` as an arg
    (Selection does not own or hold a reference to the project tree).
  - Finder split: `FinderState` (input buffer, match index, filtered set)
    lives in Selection; the boolean "finder is the active input mode" lives
    in `app.ui_modes`. App routes `/` → enter finder mode; while in finder
    mode, keystrokes go to `selection.mutate(...).apply_finder(input)`.
  - **Call-site cost across phases.** Phase 3 will write
    `selection.mutate(&self.projects)`. After Phase 6 (Scan) moves
    `projects` to `self.scan.projects()`, those call sites mass-rewrite
    to `selection.mutate(self.scan.projects())`. Acceptable per-phase
    sweep, not a redesign — but worth knowing 3 phases will touch the
    same selection-mutation call sites.

- **Phase 6 (Scan)**:
  - `mutate_tree` stays on `App` (not on `Scan`) for borrow-checker
    reasons. Each call site uses `let App { scan, panes, selection, .. } =
    self;` to split-borrow disjoint fields and pass them to the guard's
    constructor. Putting `mutate_tree` on `Scan` would force callers to
    hold `&mut app.scan` while also passing `&mut app.panes, &mut
    app.selection` — the borrow checker rejects this even though the
    fields are disjoint, because method-call syntax reborrows the receiver.
  - `TreeMutation::drop` fans out to all three affected subsystems (Scan's
    own caches, `panes.clear_for_tree_change()`,
    `selection.recompute_visibility()`). The guard owns the three `&mut`
    references it needs to fan out. Same RAII reasoning as
    `SelectionMutation` — the type system makes it impossible to mutate
    the tree without all three invalidations firing.
  - `data_generation` bumps stay explicit (called from message-relevance
    code via `BackgroundMsg::detail_relevance`), not auto-bumped by
    `TreeMutation::drop`. Reason: not every structural change is
    detail-relevant; auto-bumping would invalidate the detail cache too
    aggressively.
  - `metadata_store: Arc<Mutex<...>>` is exposed as
    `scan.metadata_store() -> &Arc<...>`; spawned tasks `.clone()` the Arc
    they need.
  - `pending_git_first_commit` and `discovery_shimmers` live in Scan (keyed
    on tree paths, consumed by tree-render code). Flagged as a future
    revisit: they may move to `Inflight` later if their "in-flight
    enrichment" framing wins out — out of scope for this phase to keep the
    refactor bounded.
  - **Worked example: `handle_project_discovered`** (currently
    `tui/app/async_tasks.rs:~1895-1920`).

    **Today:**
    ```rust
    let inline_dirs = self.current_config.tui.inline_dirs.clone();
    {
        let mut tree = self.mutate_tree();
        tree.insert_into_hierarchy(item);
        tree.regroup_members(&inline_dirs);
    } // <- TreeMutation::drop runs here, clearing worktree_summary_cache
    self.register_discovery_shimmer(item.path());     // manual follow-up
    self.rebuild_visible_rows_now();                  // manual follow-up
    ```
    The two manual follow-ups can be forgotten — that's the bug class
    we want to design out.

    **After Phase 6:**
    ```rust
    let inline_dirs = self.config.current().tui.inline_dirs.clone();
    {
        // Destructuring gives us three disjoint &mut borrows the
        // compiler accepts simultaneously.
        let App { scan, panes, selection, .. } = self;
        let mut tree = TreeMutation::new(scan, panes, selection);
        tree.insert_into_hierarchy(item);
        tree.regroup_members(&inline_dirs);
    } // <- TreeMutation::drop runs here. The Drop impl now does:
      //     1. panes.clear_for_tree_change()
      //     2. scan.register_shimmer(p) for each inserted path
      //     3. selection.recompute_visibility(scan.projects())
      //    No manual follow-ups after the block — they can't be
      //    forgotten because they live in Drop.
    ```

    The behavioral difference: the two manual follow-ups disappear
    from the call site and become part of `TreeMutation`'s automatic
    cleanup. Same effect; impossible to skip.

  - **Inner structure of `TreeMutation` after the carve.**
    ```rust
    struct TreeMutation<'a> {
        scan:      &'a mut Scan,
        panes:     &'a mut Panes,
        selection: &'a mut Selection,
        inserted:  Vec<AbsolutePath>,  // remembered by insert_into_hierarchy
    }
    impl Drop for TreeMutation<'_> {
        fn drop(&mut self) {
            self.panes.clear_for_tree_change();
            for p in &self.inserted {
                self.scan.register_shimmer(p);
            }
            self.selection.recompute_visibility(self.scan.projects());
        }
    }
    ```
    Borrow-OK because `scan`, `panes`, `selection` are stored as three
    independent `&mut` references on the guard struct itself — not
    re-borrowed from a shared `&mut App` at drop time.

- **Phase 5 (Config + Keymap, sharing a `WatchedFile<T>` primitive)**:
  - The shared "load from disk + watch stamp + try-reload" lifecycle is
    extracted as a generic struct `WatchedFile<T>` in a new submodule
    `tui::watched_file`. Not a trait — App calls each watched thing
    explicitly, no polymorphic dispatch needed.
  - `Config` and `Keymap` become two separate subsystems, each composing a
    `WatchedFile<T>` plus its bespoke state (edit buffer for `Config`,
    diagnostics-toast id for `Keymap`).
  - Splitting is justified by genuinely different bespoke state and
    different downstream wiring (config reload triggers rescan; keymap
    reload rebuilds dispatch table). The shared part is captured *once* in
    `WatchedFile<T>`, not duplicated.
  - `SettingsEditBuffer` is a typed pair (`buf: String, cursor: usize`),
    not two raw fields — prevents cursor drift past buffer bounds.
  - `ReloadOutcome` = `enum { Unchanged, Reloaded, Failed(reason) }`. App
    rescans defensively on `Reloaded`; no diff payload yet (optimization
    for later if needed).
  - Settings-modal mode (whether settings editor owns input) stays in
    `app.ui_modes` — same split as finder.

- **Phase 7 (Pane trait + foundation)**:

  **Architectural model — the eight invariants.** Phases 7–10 are
  governed by these. Anything proposed during implementation that
  breaks one of them needs to revisit the design rather than
  bypassing the invariant.

  1. **Every pane is a struct.** All 13 `PaneId` variants —
     `ProjectListPane`, `PackagePane`, `LangPane`, `CpuPane`,
     `GitPane`, `TargetsPane`, `LintsPane`, `CiPane`,
     `OutputPane`, `ToastsPane`, `SettingsPane`, `FinderPane`,
     `KeymapPane` — exist as their own struct.
  2. **A pane owns its own state.** Cursor, scroll, viewport,
     content data, and any pane-specific extras (poller, display
     modes, etc.) live on that pane's struct.
  3. **State of the same kind across all panes still lives
     per-pane, not in a shared array indexed by `PaneId`.**
     Example: every pane has a "current cursor row" — each pane
     holds its own copy. No `[Pane; N]` array of cursors.
  4. **There is no central pane-state grab bag.** The current
     `Panes` struct is repurposed as a registry that owns the
     per-pane structs as named fields. It carries no pane-keyed
     state of its own.
  5. **Common behavior is expressed via a trait.** A single `Pane`
     trait defines the shared interface, with default helper
     methods on the trait itself for shared input-handling patterns
     (nav keys, detail-pane keymap dispatch). No sub-traits — the
     base trait's default methods cover the code-reuse cases.
     Each pane's behavior is its `impl Pane`.
  6. **Outsiders read pane state through methods.** No reaching
     into pane fields from App or other subsystems. If App needs
     CI's display mode, it calls `panes.ci().display_mode_for(path)`;
     it does not read a field directly.
  7. **App holds panes through the registry.** Trait dispatch
     happens through `panes.pane(id) -> &dyn Pane` /
     `panes.pane_mut(id) -> &mut dyn Pane`. The registry's only
     job is to own the per-pane structs.
  8. **The data assembly path writes into per-pane state.** Today
     `panes/support.rs` produces all pane data centrally and stuffs
     it into one `PaneDataStore`. After Phase 8/9 that assembly
     path runs the same computations but routes each result into
     the matching pane's content slot. The central `PaneDataStore`
     is gone after Phase 9.

  **Pane trait declaration.**

  ```rust
  pub trait Pane {
      // Required hooks every pane must provide.
      fn id(&self) -> PaneId;
      fn viewport(&self) -> &Viewport;
      fn viewport_mut(&mut self) -> &mut Viewport;
      fn input_context(&self) -> InputContext;
      fn has_row_hitboxes(&self) -> bool;
      fn size_spec(&self) -> PaneSizeSpec;
      fn render(&mut self, frame: &mut Frame, area: Rect, ctx: PaneRenderCtx<'_>);
      fn handle_input(&mut self, event: &KeyEvent, ctx: PaneInputCtx<'_>);

      // Default methods (with reasonable defaults; panes override as needed).
      fn is_navigable(&self, _ctx: PaneNavCtx<'_>) -> bool { false }
      fn cursor_row(&self) -> usize { self.viewport().cursor_row }
      fn scroll_offset(&self) -> usize { self.viewport().scroll_offset }
      fn hovered_row(&self) -> Option<usize> { self.viewport().hovered_row }
      fn is_visited(&self) -> bool { self.viewport().is_visited }
      fn mark_visited(&mut self) { self.viewport_mut().is_visited = true; }
      fn clear_visited(&mut self) { self.viewport_mut().is_visited = false; }
      fn set_hover(&mut self, row: Option<usize>) { self.viewport_mut().hovered_row = row; }
      fn clear_hover(&mut self) { self.viewport_mut().hovered_row = None; }
      // Plus up/down/home/end nav, scroll mutators, content-area set/get,
      // viewport rows set/get — all default-impl forwarders to the
      // embedded Viewport. Net effect: per-pane impls write only the
      // genuinely-different methods (render, handle_input, etc.).

      // Added to the trait in Phase 10 (not present in Phases 7–9):
      //   fn hit_test(&self, row: u16) -> Option<HoverTarget>;
  }
  ```

  Notes on the trait:
  - **`size_spec` takes no parameter.** `CpuPane`'s width depends on
    its CPU snapshot, which `CpuPane` already owns after Phase 8.
    Earlier drafts threaded `cpu_width` through every pane's
    `size_spec`; the universal trait stays universal.
  - **`is_navigable` defaults to `false`.** The 6 non-navigable panes
    (Output, Toasts, Settings, Finder, Keymap, plus ProjectList's
    overlay forms) inherit the default. The 7 navigable panes
    override.
  - **`hit_test` lands on the trait in Phase 10.** During Phases 7–9
    hitbox registration continues to happen as a render side-effect
    via `PaneRenderCtx::hit_sink` (see "Hitbox-registration transition"
    below). Phase 10 adds `hit_test` to the trait, removes
    `hit_sink` from the ctx, and pulls the registration helpers out
    of render bodies.

  **`Viewport` — the embedded shared-state struct.**

  ```rust
  pub struct Viewport {
      cursor_row:    usize,
      scroll_offset: usize,
      viewport_rows: usize,
      content_area:  Rect,
      hovered_row:   Option<usize>,
      is_visited:    bool,
  }
  ```

  Every per-pane struct embeds one. The trait's two required hooks
  (`viewport`/`viewport_mut`) are one-liners on each pane. All the
  generic UI mechanics ride on default-impl methods that delegate
  to the embedded `Viewport`.

  Today's `Pane` cursor struct (in `tui/pane/state.rs`) is renamed
  to `Viewport` so the name `Pane` is free for the trait. The
  fields it holds today (`pos`, `scroll_offset`, `viewport_rows`,
  `content_area`, `hovered`) become `Viewport`'s fields, plus
  `is_visited` (newly absorbed from `Panes::visited`).

  **Shared input-handling bodies as default helper methods on
  `Pane` itself — no sub-traits.** The detail panes (Package, Lang,
  Git, Cpu, Targets) and the table panes (Lints, CiRuns) share
  enough input-handling pattern (Up/Down/Home/End nav block, then
  keymap-action dispatch) that we want one shared body, not seven
  copies. Earlier drafts proposed sub-traits (`NavigablePane`,
  `DetailFieldsPane`) for this. Dropped: nothing in the codebase
  does `dyn NavigablePane` dispatch; the sub-traits existed
  purely to make a default method body inheritable, which the
  base `Pane` trait can do directly.

  ```rust
  trait Pane {
      fn handle_input(&mut self, event: &KeyEvent, ctx: PaneInputCtx<'_>);

      // Default helper available to any pane. Non-using panes ignore.
      fn handle_nav_keys(&mut self, event: &KeyEvent) -> bool {
          match event.code {
              KeyCode::Up   => { self.viewport_mut().up();   true }
              KeyCode::Down => { self.viewport_mut().down(); true }
              KeyCode::Home => { self.viewport_mut().home(); true }
              KeyCode::End  => { self.viewport_mut().end();  true }
              _ => false,
          }
      }
  }

  impl Pane for PackagePane {
      fn handle_input(&mut self, event, ctx) {
          if self.handle_nav_keys(event) { return; }
          // package-specific keymap-action dispatch follows
      }
  }
  ```

  Each pane's `handle_input` body opts in to whichever helpers it
  needs. The "nav block" helper is on the `Pane` trait. The
  "detail-pane keymap-action dispatch" helper, if it earns its
  weight in shared lines, lands the same way: a default method
  on `Pane`. Sub-traits stay out of the design.

  Helpers land in **Phase 8** (when the first pane that needs them
  migrates), not in Phase 7 — Phase 7 ships the foundation trait
  only.

  **`Panes` is repurposed, not deleted.** The struct survives at
  the same name; it becomes a registry whose only contents are
  the per-pane structs.

  ```rust
  pub struct Panes {
      project_list: ProjectListPane,
      package:      PackagePane,
      lang:         LangPane,
      cpu:          CpuPane,
      git:          GitPane,
      targets:      TargetsPane,
      lints:        LintsPane,
      ci_runs:      CiPane,
      output:       OutputPane,
      toasts:       ToastsPane,
      settings:     SettingsPane,
      finder:       FinderPane,
      keymap:       KeymapPane,
  }
  impl Panes {
      pub fn pane(&self, id: PaneId) -> &dyn Pane { /* match */ }
      pub fn pane_mut(&mut self, id: PaneId) -> &mut dyn Pane { /* match */ }
      // Typed accessors for callers that want a specific pane:
      pub fn ci(&self) -> &CiPane { &self.ci_runs }
      pub fn cpu_mut(&mut self) -> &mut CpuPane { &mut self.cpu }
      // ... one pair per pane as needed.
  }
  ```

  **Borrow-checker note.** Typed accessors borrow `&mut self` of the
  whole `Panes`, so two consecutive accessor calls cannot be held
  simultaneously. If a future call site needs disjoint borrows of
  two specific panes at once, use field destructure:
  `let Panes { ci_runs, git, .. } = &mut self.panes;`. The Phase 7
  audit shows no current call site needs this — single-pane dispatch
  is the only access pattern — but the option exists.

  **Pane construction.** Each per-pane struct is built once at App
  startup with whatever dependencies it needs (e.g., `CpuPane`
  needs the `CpuConfig` to build its `CpuPoller`; `KeymapPane`
  needs the initial keymap reference). Constructors stay
  per-pane: `CpuPane::new(cfg)`, `CiPane::new()`, etc.
  `Panes::new(deps: PaneDeps)` is the single entry point that
  builds all 13 panes from a small `PaneDeps` struct (carrying
  `&CpuConfig` and any other startup-time dependencies). All
  construction logic lives in one place; no sprawl into App.

  **Project-selection invalidation.** Per-pane `content` slots
  become stale when the user selects a different project. The
  invalidation flow is centralized in the data-assembly path
  (`panes/support.rs`): on a selection change, the assembly path
  re-runs and overwrites all six (Phase 8) / all twelve content
  slots (Phase 9+). There is no per-pane invalidation hook on the
  `Pane` trait — content is always overwritten by the assembly
  pass, never selectively cleared. This is consistent with
  invariant 8 (assembly writes per-pane state) and avoids ad-hoc
  `clear_*` methods on individual panes.

  **Testability.** Each ctx struct (`PaneRenderCtx`,
  `PaneInputCtx`, `PaneNavCtx`) ships with a `for_test(...)` or
  `default_for_test()` constructor that builds the bundle from
  test stubs of each subsystem. Without this, isolated pane
  tests are uneconomical and "panes own behavior" is only
  theoretically testable. The test constructors land in Phase 7
  alongside the ctx struct definitions.

  **Phase 7 scope (foundation only).** This phase introduces the
  trait, `Viewport`, the registry repurpose, and **skeleton**
  `impl Pane` blocks for all 13 panes that implement only the
  `PaneId`-pure metadata methods (`id`, `has_row_hitboxes`,
  `size_spec`, `input_context`). Render, input, viewport
  accessors, and `is_navigable` are declared on the trait but
  not yet implemented per-pane — their default bodies on the
  trait panic with `unimplemented!()` and no caller dispatches
  through them yet. (The borrow-checker constraint of the eventual
  trait dispatch — see Phase 7 design notes — forces this
  split: per-pane bodies cannot accept `&mut App` while `panes`
  is mutably borrowed out of App, and the typed ctx bundles
  cannot be assembled until each pane's per-pane state moves
  onto its own struct. Phase 8 migrates state + bodies
  pane-by-pane and flips the dispatch sites then.)

  The `PaneBehavior` enum and the existing free-function
  dispatch (`panes::has_row_hitboxes`, `panes::size_spec`,
  `panes::behavior`) **survive Phase 7 unchanged**. They remain
  the canonical callers throughout Phase 7. Phase 8/9 flip
  dispatch sites pane-by-pane as bodies migrate; Phase 9 deletes
  `PaneBehavior` once both render and input dispatch flow
  through the trait.

  The current grab-bag fields on `Panes` (`manager`, `data`,
  `visited`, `layout_cache`, `worktree_summary_cache`,
  `hovered_row`, `ci_display_modes`, `cpu_poller`) stay where
  they are during Phase 7 — their migration happens during
  Phases 8–9 as each pane fully takes ownership of its share.

  **Borrow-checker constraint.** `PaneRenderCtx`, `PaneInputCtx`, and
  `PaneNavCtx` carry pre-extracted typed references to the
  subsystems each method needs (Selection, Inflight, Background,
  Config, Scan, ToastManager, Keymap, animation_elapsed) — they
  do **not** carry `&Panes` or `&mut Panes`, because the active
  pane is borrowed *out* of `Panes` at dispatch time. Anything a
  pane needs from outside its own state goes through the ctx
  bundle. The Phase 7 audit (run during the re-review) confirmed
  no pane reads or writes another pane's state, so this design is
  sufficient.

  **What `PaneRenderCtx` carries (per-pane needs from the audit).**
  Lean by design — each pane uses a subset.

  ```rust
  pub struct PaneRenderCtx<'a> {
      pub selection:        &'a Selection,        // Lang, Lints, CiRuns, Finder
      pub scan:             &'a Scan,             // CiRuns (cache dir lookups)
      pub config:           &'a Config,           // ProjectList, Cpu, Settings
      pub keymap:           &'a Keymap,           // help-text rendering
      pub inflight:         &'a Inflight,         // CiRuns (in-flight check)
      pub toasts:           &'a ToastManager,     // ToastsPane reads the toast list
      pub focused_pane:     PaneId,               // for is_focused() answers
      pub focus_state:      PaneFocusState,
      pub animation_elapsed: Duration,            // Lints, CiRuns
      pub hit_sink:         &'a mut HitboxSink,   // see hitbox transition note below; removed in Phase 10
  }
  ```

  ```rust
  pub struct PaneInputCtx<'a> {
      pub selection:  &'a mut Selection,          // Finder; ProjectList row-cursor moves
      pub scan:       &'a mut Scan,               // CiRuns clear-cache, Lints clear
      pub background: &'a mut Background,
      pub inflight:   &'a mut Inflight,           // CiRuns/Lints start/spawn
      pub config:     &'a mut Config,             // SettingsPane writes settings_edit_buffer
      pub keymap:     &'a Keymap,
      pub toasts:     &'a mut ToastManager,
  }
  ```

  ```rust
  pub struct PaneNavCtx<'a> {
      // Read by is_navigable() to decide if a pane has displayable
      // content right now (so focus skip behaves correctly).
      pub selection:  &'a Selection,
      pub scan:       &'a Scan,
      pub inflight:   &'a Inflight,
  }
  ```

  Both ctx structs are constructed at dispatch sites via the
  destructure pattern: `let App { selection, scan, panes, .. } =
  self;` then `panes.pane_mut(id).render(frame, area, ctx)`.

  **Hitbox-registration transition.** Today `register_ci_row_hitboxes`
  and `register_git_row_hitboxes` write hitbox rectangles into
  `pane_manager` *as a side-effect of render*. After per-pane
  cursors absorb into each pane's `Viewport` in Phases 8–9,
  `pane_manager` no longer owns those rectangles. During Phases 7–9
  the rectangles flow into `PaneRenderCtx::hit_sink` (a
  `&mut HitboxSink` carrying the current hover-lookup table), and
  the registration helpers stay where they are today, just
  re-pointed from `pane_manager` to the sink.

  **Where `HitboxSink` physically lives.** During Phases 7–9 it is
  a thin wrapper around the existing `pane_manager` row-hitbox map
  — the same data structure callers read today, just behind a typed
  handle that the ctx can carry. Mouse-position handling continues
  to look up the map exactly as it does today; only the *write
  path* (render → ctx → sink → map) is re-routed. The wrapper
  exists so the trait method's signature can name "the place
  hitboxes go" without exposing `&mut PaneManager` itself. In
  Phase 10 both the wrapper type and the underlying map are
  deleted in the same change.

  **Test stubbability.** `HitboxSink` carries a no-op variant
  (e.g., `HitboxSink::null()`) usable from `PaneRenderCtx::for_test`
  so isolated render tests don't have to construct a real hover-
  lookup table. The wrapper is small and trivially stubbable.

  Phase 10 adds `Pane::hit_test(row) -> Option<HoverTarget>` to
  the trait, removes `hit_sink` from `PaneRenderCtx`, deletes
  `HitboxSink` and the registration helpers — hover lookup becomes
  a query pass instead of a render side-effect.

- **Phase 7 cleanup obligations carried into Phase 8.**
  Phase 7 ships several `#[allow(dead_code, reason = "Phase 7
  foundation; wired up in Phase 8")]` markers on items that exist
  for the foundation but have no caller yet. Phase 8 **must
  remove every Phase-7 in-flight allow it activates**:
  - `Panes::pane(id)` / `Panes::pane_mut(id)` — first dispatch
    site (the `match PaneId` flip in `render.rs::render_tiled_pane`
    or any pane's `handle_input` dispatch) deletes the allow.
  - `Pane` trait — first non-trivial caller of any trait method
    deletes the allow.
  - `InputContextKind` — first user (the `app/focus.rs::input_context`
    flip) deletes the allow.
  - `Viewport::hovered()` — first default-impl method on `Pane`
    that calls it deletes the allow.
  - `PaneRenderCtx` / `PaneInputCtx` / `PaneNavCtx` — when each
    ctx is first populated with real subsystem refs (i.e., when
    the first pane's render/input body migrates), drop the
    `#[allow(dead_code, reason = "Phase 7 placeholder ctx; ...")]`
    on the affected struct.
  - `HitboxSink::null()` — first non-test caller (a real Phase-8
    render dispatcher constructing a sink) deletes the allow.

  Phase 9 / Phase 10 inherit any allow that hasn't lapsed by end
  of Phase 8. Phase 10 is the last phase that may carry an
  in-flight allow from this set; if any survive past Phase 10's
  validation, the design has a gap that must be addressed before
  the carve is considered shipped.

- **Phase 8 (Migrate the 6 detail/data panes)**:
  - **Phase 8 ships as up to 6 commits**, one per pane (or
    grouped if a pair migrates trivially together). Order is
    simplest-first to grow the trait-dispatch surface
    incrementally with validation between commits. Both/all
    commits land before Phase 8 is considered shipped; Phase 9
    starts after every detail/data pane is fully on the trait.
  - **Each per-pane commit may itself ship in two stages** if
    the migration is large: (a) **state relocation** — move
    pane-specific extras (`cpu_poller`, `ci_display_modes`),
    content slot, and viewport onto the per-pane struct;
    update typed accessors and assembly path; old free-function
    render body still runs (now reading from per-pane state via
    typed accessors). (b) **body migration** — move the render
    and input bodies into trait methods, flip the dispatch
    sites, retire dead-code allows. The split keeps each commit
    reviewable; both stages must land before that pane is
    considered shipped.
  - Migrate `CiPane`, `CpuPane`, `GitPane`, `LintsPane`,
    `PackagePane`, `LangPane`. Each pane gains:
    - `viewport: Viewport` field (the per-pane cursor/scroll/hover/
      visited state, replacing its slot in the old `PaneManager`).
    - `content: Option<XData>` field. For panes that have a
      `PaneDataStore` slot today (Package, Cpu, Git, CI, Lints —
      five of the six migrating in Phase 8), this replaces that
      slot. `LangPane` does not have a `PaneDataStore` slot today
      (`render_lang_panel_standalone` reads language stats
      directly off the project tree); after Phase 8 it gains a
      content slot the same way the others do.
    - Pane-specific extras: `CpuPane` absorbs `Panes::cpu_poller`;
      `CiPane` absorbs `Panes::ci_display_modes`.
  - Render and input bodies move from the free functions in
    `panes/*.rs` and `panes/actions.rs` into the trait method
    bodies. The skeleton impls from Phase 7 get filled in.
  - Shared input-handling default methods on `Pane` land here.
    The Up/Down/Home/End nav helper (`handle_nav_keys`) and the
    detail-pane keymap-dispatch helper land alongside the first
    pane that uses them. Detail panes (`PackagePane`, `LangPane`,
    `GitPane`, `CpuPane`) call both helpers from their
    `handle_input` body; table panes (`LintsPane`, `CiPane`)
    call the nav helper plus their own action dispatch.
    `TargetsPane` adopts the same pattern in Phase 9 when it
    migrates.
  - **`CpuPane::tick(now: Instant)`** is a concrete method (not on
    the `Pane` trait) that owns the per-tick CPU poll. App's per-tick
    handler calls `self.panes.cpu_mut().tick(now)` — replaces today's
    `Panes::cpu_tick`.
  - The data-assembly path (`panes/support.rs`) gets retargeted
    to write each result into the matching per-pane `content`
    slot via the registry: `panes.ci_mut().set_content(...)`,
    `panes.git_mut().set_content(...)`, etc. **`set_content` is a
    concrete method on each per-pane struct, not on the `Pane`
    trait** — each pane has its own `Content` type
    (`CiData`/`GitData`/`LintsData`/…), so a single trait method
    would have to be generic on associated type for no real benefit.
    The assembly path knows the concrete types it writes to. The
    `PaneDataStore` slots for the five Phase-8 migrators that have
    one (Package, Cpu, Git, CI, Lints) are removed; the `targets`
    slot remains in `PaneDataStore` until Phase 9 takes Targets.
    `LangPane` did not have a slot today, so it adds a content
    field rather than replacing one.
  - **`worktree_summary_cache` half-state.** GitPane migrates in
    Phase 8, but `Panes::worktree_summary_cache` stays on the
    registry through Phase 9. GitPane reads/writes it via a
    registry accessor (`panes.worktree_summary_cache()` /
    `panes.worktree_summary_cache_mut()`) during Phases 8–9. Phase
    10 picks the cache's final home (likely `GitPane` itself, or a
    data-assembly service if the assembly path needs it before any
    pane has been told).
  - **Grab-bag ledger after Phase 8.** Of the 8 fields the Phase 1
    `Panes` carried, Phase 8 removes: `cpu_poller` (→ `CpuPane`),
    `ci_display_modes` (→ `CiPane`), six slots out of `pane_data`
    and six slots out of `pane_manager` (→ each migrated pane's
    `Viewport` and `content`), and the corresponding entries in
    `visited` and `hovered_row` (also → `Viewport`). What remains
    on the registry after Phase 8: the seven non-migrated panes'
    cursor/data slots, `layout_cache`, `worktree_summary_cache`.

- **Phase 9 (Migrate the remaining 7 panes)**:
  - Migrate `ProjectListPane`, `TargetsPane`, `OutputPane`,
    `ToastsPane`, `SettingsPane`, `FinderPane`, `KeymapPane`.
  - `ProjectListPane` is the largest move by far: closer to ~600
    lines once the named tree-row helpers are counted
    (`render_project_list`, `render_root_item`, `render_child_item`,
    `render_member_item`, `render_worktree_entry`,
    `render_wt_group_header`, `render_wt_member`,
    `render_vendored_item`, `render_submodule_item`,
    `render_path_only_entry`, `render_wt_vendored_item`,
    `render_tree_items`, `render_tree_item`). All move from
    `render.rs` into `panes/project_list.rs`. **Phase 9 ships as
    two commits**: `ProjectListPane` alone in the first commit, the
    remaining six panes in the second. Both land before the phase
    is considered shipped.
  - **`TargetsPane` adopts the shared input-handler helpers.** The
    nav helper and detail-pane dispatch helper landed in Phase 8 on
    the `Pane` trait; `TargetsPane`'s migration in Phase 9 calls
    them from its own `handle_input` body, same pattern as the four
    detail panes that adopted in Phase 8 (Package/Lang/Git/Cpu).
  - The overlay panes (`SettingsPane`, `FinderPane`,
    `KeymapPane`) get the same trait treatment. Their special
    handling for the App-shell modal mode (`ui_modes.settings`,
    `ui_modes.finder`, `ui_modes.keymap`) stays on App-shell —
    the panes implement `Pane` for their render/input bodies but
    the modal-mode flag stays where it is.
  - **`Panes::apply_hovered_pane_row` collapses.** Today's
    multi-step "clear all hovers, find the hovered pane, set hover
    on it" flow becomes a single per-pane `set_hover` call at the
    render call site (or a one-liner App-shell helper if the
    `mouse_pos`-to-row conversion stays grouped). The dispatch
    target is `panes.pane_mut(hovered.pane).set_hover(Some(row))`,
    using the `set_hover` default-impl method on the trait.
  - After Phase 9, the central `PaneDataStore` and the free
    functions in `panes/actions.rs` are entirely gone.
    `panes/support.rs` is now strictly a data-assembly module
    that writes results into per-pane slots.
  - **Grab-bag ledger after Phase 9.** All `pane_data` slots
    (the central `PaneDataStore`) are gone — each pane owns its
    `content` field. All `pane_manager` per-pane cursor slots are
    gone — each pane owns its `Viewport`. `visited` and
    `hovered_row` are fully absorbed into per-pane `Viewport`s.
    What remains on the registry after Phase 9: `layout_cache`
    (coordination state, not pane state) and
    `worktree_summary_cache` (assembly cache without a clean
    home yet). Both move out in Phase 10.

- **Phase 10 (Hit-test promotion + final cleanup)**:
  - **Hit-test promotion.** `Pane::hit_test(row: u16) ->
    Option<HoverTarget>` is added to the trait as a required
    method (it was absent in Phases 7–9). Each pane implements
    it. `PaneRenderCtx::hit_sink` is removed from the ctx struct.
    The `register_ci_row_hitboxes` / `register_git_row_hitboxes`
    helpers — which during Phases 7–9 wrote into `hit_sink` — are
    deleted. Mouse-position handling switches from looking up the
    side dictionary to calling `panes.pane(id).hit_test(row)`.
  - **`Panes::worktree_summary_cache` moves to `GitPane`.** The
    cache feeds the Git pane's worktree section; it is git-pane
    content. The "data-assembly service" alternative discussed in
    earlier drafts was the assembly path absorbing what is
    properly pane state to avoid putting it on the obvious owner.
    Pre-committed here, not "decided at implementation time."
  - **`Panes::layout_cache`** moves to App-shell (it's coordination
    state, not pane state — what rect each pane occupies on
    screen, computed once per draw and shared).
  - **`PaneManager`-the-struct disappears.** Today `PaneManager`
    is the container holding the 13 per-pane cursor structs plus
    the row-hitbox map. Phases 8–9 absorb the cursors into per-pane
    `Viewport`s; Phase 10 removes the row-hitbox map (replaced by
    `Pane::hit_test`). At that point `PaneManager` has nothing to
    hold and is deleted.
  - **End state.** The `Panes` struct holds only the 13 per-pane
    structs. All 8 grab-bag fields it carried after Phase 1 have
    found their final homes: `pane_manager` and `pane_data` are
    disassembled into per-pane `Viewport`s and `content` slots;
    `visited` and `hovered_row` are absorbed into `Viewport`;
    `cpu_poller` lives on `CpuPane`; `ci_display_modes` lives on
    `CiPane`; `layout_cache` is on App-shell;
    `worktree_summary_cache` has its final home decided here.

- **Phase 4 (Background + Inflight)**:
  - One phase, not two. Every "start" call site touches both subsystems
    (push to channel + mark in-flight + update toast); splitting would
    touch each site twice.
  - `watch_tx` lives in `Background` for uniformity (all I/O channels in
    one place). Watcher replies come back through `bg_rx` anyway.
  - `StartOutcome` is an `enum { Started, AlreadyRunning, Queued }`, not a
    `bool` — type-driven so a duplicate-start can't be silently misread as
    a fresh start.
  - `ToastManager` stays on App (broader than inflight: confirm popups,
    errors, manual toasts). `Inflight` methods that update toasts take
    `&mut ToastManager` as a parameter.

- **Phase 11 (Net subsystem)**:
  - **First task: re-review the Phase 11 plan.** Before writing carve
    code, revisit the "Net subsystem (Phase 11 skeleton)" section and
    the field-appendix entries against the actual state of Phases
    1–10. Three open questions to answer at re-review time, captured
    in that section:
    1. Do `running_fetches` / `running_fetch_toast` move into
       `Inflight`, or stay in `Net`?
    2. Does `fetch_cache` belong in `Net` (HTTP-coupled) or `Scan`
       (tree-enrichment cache)?
    3. Does `availability` collapse into `Net::availability(service)`
       or stay per-service?
    Update the section, get user approval on the answers, then
    implement.
  - **Phase 11 starts after Phase 10.** All other subsystems exist
    by then, so `Net` can take typed references to whatever it
    depends on (today: `Inflight` for the running-fetches question,
    `Scan` for the fetch-cache question) without re-introducing
    god-object parameters.
  - **What disappears from App.** The three fields (`http_client`,
    `github`, `crates_io`) plus every accessor today exposing them
    (e.g., `App::repo_fetch_cache`, `App::github_status`,
    `App::rate_limit`, `App::start_repo_fetch_for`,
    `App::complete_repo_fetch_for`). All move onto `Net` or the
    chosen home from re-review.
  - **End state.** App owns no network state directly. Eight
    subsystem handles, ~12 App-shell fields total.

## Field assignment appendix (every App field accounted for)

The subsystem table covers the headline carves. This appendix is the
exhaustive list — every field of `App` (`tui/app/mod.rs:88-180`) and where
it lands. Items marked **App-shell** stay on `App`.

| Field | Destination |
|---|---|
| `current_config` | Config |
| `http_client` | Net (Phase 11) |
| `github` (`GitHubState`: `fetch_cache`, `repo_fetch_in_flight`, `running_fetches`, `running_fetch_toast`, `availability`) | Net (Phase 11); `running_fetches`/`running_fetch_toast` may move to Inflight at Phase 11 re-review |
| `crates_io` (`CratesIoState`) | Net (Phase 11) |
| `projects` | Scan |
| `ci_fetch_tracker` | Inflight |
| `ci_display_modes` | Panes in Phase 1; moves to `CiPane` in Phase 8 |
| `lint_cache_usage` | Scan (cache-stat counter, not in-flight bookkeeping) |
| `discovery_shimmers` | Scan (with future-revisit note: may move to Inflight) |
| `pending_git_first_commit` | Scan (with same future-revisit note) |
| `cpu_poller` | Panes in Phase 1; moves to `CpuPane` in Phase 8 |
| `bg_tx`/`bg_rx` | Background (with rescan-swap caveat; see above) |
| `priority_fetch_path` | Scan |
| `expanded` | Selection |
| `pane_manager` | Panes in Phase 1; per-pane cursors absorb into each pane's `Viewport` field in Phases 8–9; the `PaneManager` struct itself (today owning the row-hitbox map) is deleted in Phase 10 when `Pane::hit_test` replaces side-effect registration |
| `pane_data` | Panes in Phase 1; per-pane content slots absorb into each pane's `content` field in Phases 8–9 |
| `settings_edit_buf`/`settings_edit_cursor` | Config (combined as `SettingsEditBuffer`) |
| `focused_pane` | **App-shell** (focus stack) |
| `return_focus` | **App-shell** (focus stack) |
| `visited_panes` | Panes in Phase 1; absorbed into each pane's `Viewport.is_visited` in Phases 8–9 |
| `pending_example_run` | Inflight |
| `pending_ci_fetch` | Inflight |
| `pending_cleans` | Inflight |
| `confirm` | **App-shell** (modal/UI shell) |
| `animation_started` | **App-shell** (UI shell) |
| `ci_fetch_tx`/`rx` | Background |
| `clean_tx`/`rx` | Background |
| `example_running` | Inflight |
| `example_child` | Inflight |
| `example_output` | Inflight |
| `example_tx`/`rx` | Background |
| `running_clean_paths` | Inflight |
| `clean_toast` | Inflight |
| `running_lint_paths` | Inflight |
| `lint_toast` | Inflight |
| `ci_fetch_toast` | Inflight |
| `watch_tx` | Background |
| `lint_runtime` | Background |
| `selection_paths` | Selection |
| `finder` | Selection |
| `cached_visible_rows` | Selection |
| `cached_root_sorted` | Selection |
| `cached_child_sorted` | Selection |
| `cached_fit_widths` | Selection (typed as `ProjectListWidths` after rename) |
| `worktree_summary_cache` | Panes in Phase 1; final home (likely `GitPane` or assembly service) decided in Phase 10 |
| `data_generation` | Scan |
| `mouse_pos` | **App-shell** |
| `hovered_pane_row` | Panes in Phase 1; absorbed into each pane's `Viewport.hovered_row` in Phases 8–9 |
| `layout_cache` | Panes in Phase 1; moves to App-shell in Phase 10 (coordination state, not pane state) |
| `status_flash` | **App-shell** |
| `toasts` | **App-shell** (broader than Inflight) |
| `config_path` / `config_last_seen` | Config (inside `WatchedFile<CargoPortConfig>`) |
| `current_keymap` / `keymap_path` / `keymap_last_seen` | Keymap (inside `WatchedFile<ResolvedKeymap>`) |
| `keymap_diagnostics_id` | Keymap |
| `inline_error` | **App-shell** |
| `ui_modes` | **App-shell** (modal-mode flags including finder/settings) |
| `dirty: DirtyState` | Scan (drives next-tick work; consumed by Scan's apply path) |
| `scan: ScanState` | Scan (state machine) |
| `selection: SelectionSync` | Selection (private internal) |
| `metadata_store` | Scan |
| `target_dir_index` | Scan |
| `confirm_verifying` | Scan |
| `retry_spawn_mode` (test-only) | Scan |

App-shell field count after the **planned 11 phases**: ~12 (focus
stack + modal/UI shell + 8 subsystem handles). Down from ~60.
Phases 7–10 (the pane subsystem rewrite) reorganize per-pane state
and behavior but do not add App fields; Phase 10 moves
`Panes::layout_cache` to App-shell as part of final cleanup, which
nets out at the same total. Phase 11 (Net carve) removes the last
residual cluster of network state from App-shell, taking the count
from ~16 (after Phase 10) down to ~12.

### `Net` subsystem (Phase 11 skeleton — subject to re-review)

`http_client` + `github` (`GitHubState`) + `crates_io` (`CratesIoState`)
together form an eighth subsystem. This is a **skeleton only** — Phase
11 begins with a re-review of this section against everything learned
in Phases 1–10 (the `running_fetches` / `Inflight` overlap and the
`fetch_cache` / `Scan` overlap may have resolved themselves along the
way), and the public API drafted here is a starting point.

Fields absorbed:

| Field | Why it groups here |
|---|---|
| `http_client: HttpClient` | Shared rate-limit state and connection pool |
| `github: GitHubState` (`fetch_cache`, `repo_fetch_in_flight`, `running_fetches`, `running_fetch_toast`, `availability`) | All keyed by repo, all fed by HTTP |
| `crates_io: CratesIoState` (`availability`) | Same lifecycle as `github` (availability tracker) |

Sketch of public API (re-review will refine):
```text
net.http_client() -> &HttpClient
net.github_status() -> AvailabilityStatus
net.crates_io_status() -> AvailabilityStatus
net.fetch_cache() -> &RepoCache
net.start_repo_fetch(repo, ctx) -> StartOutcome
net.complete_repo_fetch(repo, result)
net.poll_rate_limit() -> GitHubRateLimit
```

Read by Panes (Git pane reads availability + rate limit) and by Scan
(`fetch_cache` feeds tree enrichment). Phase 11's re-review must
answer:

- Do GitHub repo fetches go through `Inflight` (uniform "in-flight
  tracker" pattern) or stay in `Net` (HTTP-coupled)? Today
  `running_fetches` + `running_fetch_toast` are GitHub-specific; if
  Inflight has matured into a generic `start_/finish_` pattern by
  Phase 11, moving them is the right call. If not, leave them in
  `Net`.
- Does `fetch_cache` belong in `Net` (HTTP-coupled, response cache)
  or in `Scan` (tree-enrichment cache, keyed by repo path)? Today it
  lives on App; Phase 11's re-review picks a home.
- Does the `availability` tracker (currently per-service in `github`
  and `crates_io`) collapse into a single `Net::availability(service)`
  query, or stay as separate fields?

After Phase 11 there is **no residual network state on App-shell**.
App's eight subsystem handles are: `panes`, `selection`, `background`,
`inflight`, `config`, `keymap`, `scan`, `net`.

## Methods that stay on App

Most of the plan focuses on field placement. The matching method-level
question — "which `App::*` methods stay, which move to a subsystem, which
disappear?" — is captured here. Methods marked **App-shell** stay on `App`
because they orchestrate across subsystems.

- **Event-loop dispatchers (App-shell):** `tick`, `draw`,
  `handle_bg_msg` (`tui/app/async_tasks.rs:~2303`), `handle_*` per-message
  dispatchers, `rescan` (`async_tasks.rs:~1353`). These read inputs and
  fan out to subsystems; staying on App is correct.
- **`rescan` is the canonical orchestrator example.** Today it
  mutates `github.fetch_cache`, swaps `bg_tx`/`bg_rx`, resets `scan`
  state, and reconfigures `lint_runtime` — crossing 4 of the 7
  subsystems plus `Net`. After phases 1–8, its body becomes a sequence
  of subsystem calls (`self.background.swap_bg_channel(...)`,
  `self.apply_lint_config_change(...)` (see below),
  `self.net.invalidate_fetch_cache(...)`, etc.) but the orchestration
  pattern stays.
- **`apply_lint_config_change` is App-shell, not Inflight.** Today's
  `App::refresh_lint_runtime_from_config` (`tui/app/async_tasks.rs:343-357`)
  is doing more than its name suggests: it respawns the lint runtime
  *and* clears in-memory lint state, clears `running_lint_paths`,
  syncs the running-lint toast, syncs the lint runtime's project
  list, refreshes lint runs from disk, recomputes column widths
  (because the project pane shows different columns when lints are
  on vs off), and bumps `data_generation`. After the carve those
  side-effects span three subsystems:
  - **Inflight**: `respawn_lint_runtime(&config.lint)`, clear
    `running_lint_paths`, sync toast.
  - **Scan**: clear in-memory lint state, refresh lint runs from
    disk, bump `data_generation`.
  - **Selection**: recompute `cached_fit_widths` (lint-enabled
    column schema differs from lint-disabled).
  The orchestration lives on App as `apply_lint_config_change`,
  called from the per-tick config-reload handler. **In-code
  documentation requirement** (same pattern as the mutation-guard rule
  above): the function must land with a doc comment that names
  every subsystem it touches and what it does in each, so a future
  maintainer adding a new lint-config side-effect knows where to put
  it. Reference template (rough draft, refine at implementation):
  ```rust
  /// Apply a lint configuration change. Cross-subsystem orchestration —
  /// not a method on any single subsystem because lint config changes
  /// fan out across three areas:
  ///
  /// - **Inflight**: respawns the lint runtime, clears in-flight lint
  ///   paths, refreshes the lint toast.
  /// - **Scan**: clears in-memory lint state, refreshes lint runs from
  ///   disk, bumps `data_generation` so detail panes redraw.
  /// - **Selection**: recomputes `cached_fit_widths` because the
  ///   project pane's column schema depends on whether lints are
  ///   enabled.
  ///
  /// Called from the per-tick config-reload handler. New side-effects
  /// of a lint-config change MUST be added here (or in the relevant
  /// subsystem method this function calls), not in random callers.
  fn apply_lint_config_change(&mut self, lint_cfg: &LintConfig);
  ```
- **Move under the new pane plan (Phases 7–10):**
  - `apply_hovered_pane_row` (`tui/app/mod.rs:278-286`) — Phase 1
    moved this onto `Panes` as a temporary home; under the new
    plan it collapses in Phase 9 to a single per-pane `set_hover`
    call (the trait default-impl method on `Viewport`). The
    surviving call site is either inlined at the renderer or kept
    as a one-line App-shell helper that converts `mouse_pos` into
    a `(PaneId, row)` pair and dispatches to the per-pane
    `set_hover`.
  - `toggle_ci_display_mode_for` (`mod.rs:565`) — under Phase 1
    this lives on `Panes`; under Phase 8 it lands on `CiPane`
    along with `ci_display_modes`. Outside callers reach it via
    `panes.ci_mut().toggle_display_mode_for(path)`.
  - `focus_pane` — App-shell. `focused_pane` and `return_focus`
    stay on App per the field appendix; `focus_pane` reads pane
    state via the registry (calling `is_navigable(...)` to decide
    skip behavior) but writes App's focus stack.
  - `register_*_row_hitboxes` helpers — *not* relocated. During
    Phases 7–9 they continue to run (re-pointed from
    `pane_manager` to `PaneRenderCtx::hit_sink`). Phase 10
    deletes them entirely when `Pane::hit_test` becomes a query
    method.
- **Move into `Scan`:** `register_discovery_shimmer` (today on App at
  `query.rs:480`), `increment_data_generation`, `complete_ci_fetch_for`
  (currently on App; ci-fetch tracker is in Inflight, but the
  *completion* affects scan state), `replace_ci_data_for_path`,
  `start_ci_fetch_for`, `should_verify_before_clean` (currently
  `mod.rs:394`), `clean_metadata_dispatch` (`mod.rs:415`),
  `resolve_target_dir` (`mod.rs:622`).
- **Move into `Selection`:** `rebuild_visible_rows_now`,
  `selected_project_path`, `lint_at_path`/`lint_at_path_mut` (callers
  pass through, real owner is `projects`).
- **Move into `Inflight`:** `start_task_toast`-style helpers,
  `running_clean_paths`/`running_lint_paths` accessors. Just the
  *runtime respawn* portion of `refresh_lint_runtime_from_config`
  becomes `Inflight::respawn_lint_runtime`; the broader
  cross-subsystem orchestration becomes `App::apply_lint_config_change`
  per the entry above.
- **Disappear (collapse into facade):** the ~270 `pub(in super::super)
  fn` accessors that exist solely to expose a single field through App.

## Rollback / revisit policy

"No phase begins until the prior is committed and green" is necessary but
not sufficient. Add one more rule:

- **If a later phase reveals a flaw in an earlier phase's facade, fix the
  earlier facade rather than working around it.** Working around a wrong
  facade in subsequent phases bakes the mistake in and produces facade
  layers on top of facade layers. The git history makes earlier facades
  cheap to revisit; the conventional-commit prefixes (`refactor:`) signal
  it's safe to do so.

### Facades most likely to need revision

Specific predictions, so reviewers expect these revisits as part of the
plan rather than treating them as scope creep:

- **`Panes::handle_input` signature.** Phase 1 lands it taking `&mut App`
  (because `panes/actions.rs:32-336` reaches into ~6 of the 7 future
  subsystems). Phase 7 introduces `Pane::handle_input` on the trait
  with skeleton impls calling the existing free functions; Phases 8–9
  move the actual bodies into trait impls and replace the `&mut App`
  parameter with typed `PaneInputCtx<'_>` bundles — at that point
  `panes/actions.rs` as a free-function module ceases to exist. The
  intermediate phases (3–6) may layer in narrower facade methods, but
  the structural rewrite is Phases 7–9.
- **`Inflight::StartContext` field set.** Phase 4 lands it with the
  fields named above. If a future phase introduces a new
  cross-subsystem dependency for a start path (e.g., a tree-aware lint
  start), the cleanest fix is to add the field to `StartContext` rather
  than thread a new parameter through every `start_*`. The struct
  exists precisely to absorb that growth.

## Recurring patterns

**Where the patterns live in code.** Phase 1 lands a module-level doc
comment at the top of `src/tui/app/mod.rs` that names every recurring
pattern this plan introduces, briefly describes each, and points at
the canonical example for each. Subsequent phases extend that comment
when they introduce new patterns. The plan document is the design
source of truth; the App-module doc comment is the in-code index a
maintainer hits when reading the code.

Every individual use of a pattern (a mutation guard, a cross-subsystem
orchestrator, a generic-primitive composition) lands with its own doc
comment that references the App-module index by pattern name — e.g.,
`/// Mutation guard (RAII). See "Recurring patterns" in
src/tui/app/mod.rs for the pattern; this is the fan-out variant
covering Scan + Panes + Selection invalidation.` That keeps the index
authoritative and the per-use comments short.

**Reference template for the App-module doc comment** (Phase 1
delivers this; refine across phases):

```rust
//! # Recurring patterns
//!
//! This module's structure (and the subsystems it owns) follows a few
//! patterns that recur across the codebase. New code that fits one of
//! these patterns MUST follow the named pattern, not invent a variant.
//!
//! ## Mutation guard (RAII)
//! Gate mutating methods through a temporary handle whose `Drop` runs
//! the recompute that derived caches need. The only way to call the
//! mutating methods is via the handle; the only way to drop the handle
//! is to let the recompute fire. Type-enforced; no convention to
//! remember.
//!
//! - **Self-only flavor** — see `SelectionMutation` (carved in Phase 3).
//! - **Fan-out flavor** — see `TreeMutation` (this module). The guard
//!   borrows the sibling subsystems it must invalidate; `Drop` fans
//!   out across them.
//!
//! ## Cross-subsystem orchestrator on App
//! When an operation has to touch multiple subsystems and there is no
//! single subsystem where it naturally lives, it stays as a named
//! method on `App`. These are the legitimate App-shell methods after
//! the carve. Their doc comments name every subsystem they touch and
//! instruct future maintainers that new side-effects of the same
//! event MUST be added there, not scattered.
//!
//! - See `App::apply_lint_config_change` (Phase 4) — touches Inflight
//!   + Scan + Selection.
//! - See `App::rescan` — touches Background + Scan + Inflight + Net.
//!
//! ## Generic primitive plus bespoke state
//! When two subsystems need the same lifecycle (e.g., load-watch-reload)
//! but carry different bespoke state, write the lifecycle as a generic
//! struct and have each subsystem compose it.
//!
//! - See `tui::watched_file::WatchedFile<T>` (Phase 5) — composed by
//!   `Config` (with edit buffer) and `Keymap` (with diagnostics-toast
//!   id).
//!
//! ## Typed display values, not pre-rendered strings
//! When a pane renders a value derived from a subsystem, the subsystem
//! returns a typed enum naming the *state*, not a `String` pre-formatted
//! for display. The renderer matches on variants and formats at render
//! time. Stops cross-subsystem free helpers from accreting in
//! `panes/support.rs` and stops stringly-typed dispatch in renderers.
//!
//! - See `LintDisplay` (Phase 11) — lint state for the Package pane row.
//! - See `CiDisplay` (Phase 13) — CI state for the Package pane row.
```

Phase rules:

- **Phase 1** lands the initial pattern index covering "Mutation guard
  (RAII) — fan-out flavor" referencing the existing `TreeMutation`,
  plus stub entries for the other patterns marked "lands in Phase N."
- **Each later phase** that introduces a new instance of a pattern
  updates the App-module doc comment to point at the new canonical
  example (or fills in the stub if it's the first instance).
- **Each later phase** that introduces a *new* pattern adds a new
  section to the App-module doc comment with the same pattern.

The patterns themselves:

- **Mutation guard (RAII)**: when a subsystem has derived/cached state that
  must be recomputed after a cluster of mutations, gate the mutating methods
  through a `&mut Self`-borrowing guard whose `Drop` runs the recompute.
  Two flavors:
  - **Self-only**: `SelectionMutation` invalidates only Selection's own
    derived state on drop.
  - **Fan-out**: `TreeMutation` borrows references to other subsystems
    (`&mut panes, &mut selection`) and invalidates each on drop. Use this
    pattern when one subsystem's mutation forces invalidation across
    siblings — the borrow declares the dependency at the type level.
  Apply this pattern to any future subsystem with the same invariant.

  **In-code documentation requirement.** Every mutation guard *must* land
  with a doc comment on the guard struct that:
  1. Names the type-level invariant the guard enforces (what makes
     bypass impossible).
  2. Names what runs in `Drop` (which caches clear / which recomputes
     fire).
  3. Tells a future maintainer where to add new mutation paths — i.e.,
     "add new methods *here*, not on the underlying subsystem, so the
     drop-clear fires."

  The existing `TreeMutation` (`src/tui/app/mod.rs:630-639`) is the
  reference template. New guards introduced by Phases 3, 6, or any
  later phase copy that doc pattern — no comment is shorter, no
  comment is fluffier. The plan's "Recurring patterns" section names
  the pattern; the guard's doc comment is what a future maintainer
  hits first when reading the code.

- **Cross-subsystem orchestrator on App**: when an operation has to
  touch multiple subsystems and there's no single subsystem where it
  naturally lives, it stays on `App` as a named method (e.g.,
  `App::apply_lint_config_change`, `App::rescan`). These are the
  legitimate App-shell methods after the carve.

  **In-code documentation requirement.** Every cross-subsystem
  orchestrator on App *must* land with a doc comment that:
  1. States that it's a cross-subsystem orchestration (and is therefore
     intentionally on `App`, not on one of the subsystems).
  2. Lists every subsystem it touches and what it does in each.
  3. Tells a future maintainer that new side-effects of the same
     event MUST be added here (or in the subsystem methods this
     function calls), not in random other callers — so the
     orchestration stays the single source of truth for the event.

  The plan's "Methods that stay on App" section gives a concrete
  template for `apply_lint_config_change`. New orchestrators copy
  that doc pattern.

- **Typed display values, not pre-rendered strings**: when a UI
  consumer (typically a pane) needs to render a value derived from
  a subsystem's state, the subsystem returns a typed value
  describing the *state* — not a `String` pre-formatted for
  display. The renderer matches on the typed value's variants and
  formats them at render time.

  Why: pre-rendered strings collapse semantically distinct states
  (e.g., "no CI workflow", "no CI runs", "runs present with
  conclusion X + count N") into flat text the renderer disambiguates
  by string equality (`if value == NO_LINT_RUNS`). That's
  stringly-typed dispatch — fragile, hides the state machine, and
  forces every consumer to re-derive the same string-comparison
  logic. It also tends to drag in a free helper that funnels
  multiple App methods to assemble the string, which is the smell
  Phases 11 and 13 exist to fix (see "Phase 8 follow-ups").

  Apply this pattern when a subsystem owns the underlying state
  and a pane is currently building a `*_display: String` field
  out of cross-subsystem reads. The subsystem grows a typed enum
  and a `<Subsystem>::<consumer>_display(...)` method that returns
  it; `PaneData` carries the enum, not the string.

  **In-code documentation requirement.** Every typed-display enum
  *must* land with a doc comment that:
  1. Names the consumer (e.g., "Display value for the Lint row in
     the Package detail pane").
  2. Documents each variant and the underlying state it
     represents (not how it renders — rendering is the
     consumer's job).
  3. References this pattern by name so future readers find the
     index.

  - See `LintDisplay` (Phase 11) — Lint state for the Package
    pane row.
  - See `CiDisplay` (Phase 13) — CI state for the Package pane row.

## Phase 8 follow-ups (resolved)

Items surfaced during Phase 8 that were tracked here for
follow-up. Both are now resolved or contracted into named phases:

- **`PackageData` Lint/Ci display resolution belongs behind a
  typed boundary.** *Resolved by being contracted into Phases 11
  and 13.* Phase 8.14 left `lint_display: String` and
  `ci_display: String` on `PackageData`, fed by two free helpers
  in `panes/support.rs` (`resolve_lint_display`,
  `resolve_ci_display`) that funnelled ~6 App methods each. The
  permanent fix lives in the new carve phases:

  - Phase 11.0 (Lint design) defines the `LintDisplay` typed enum
    and the `Lint::package_display(path) -> LintDisplay` API as
    non-negotiable carve deliverables.
  - Phase 13.0 (Ci design) does the same for `CiDisplay` /
    `Ci::package_display(path) -> CiDisplay`.
  - Phase 13.last (capstone, inside Phase 13) flips the
    `PackageData` field types from `String` to the typed enums,
    updates the renderer to match on variants, and deletes the
    free helpers from `support.rs`.

  See "Typed display values, not pre-rendered strings" in
  Recurring patterns for the principle that drives this work.

## Phase 10.3 trait-redesign decisions (implemented)

Six open design questions surfaced when starting Phase 10.3
(`Pane::hit_test` + remove push-during-render hitboxes). Decisions
recorded here as we walk them; implementation begins after all six
are settled.

- **Push → pull inversion scope.** _Decided: full replacement, no
  parallel migration._ `HitboxSink` and `PaneRenderCtx::hit_sink`
  are deleted in the same phase that introduces `Pane::hit_test`.
  No cutover trigger needed.
- **Action sub-regions (Body vs Action `[x]`).** _Decided: option (b)
  — `hit_test(row) -> Option<RowHit>` returns a richer enum that
  carries the row plus an optional list of action `Rect`s the
  click handler resolves against `mouse_x`._

  ```rust
  fn hit_test(&self, row: u16) -> Option<RowHit>;

  enum RowHit {
      Row,
      RowWithActions { actions: Vec<(Rect, ActionId)> },
  }
  ```

  Tradeoffs accepted: per-call allocation for the action `Vec`
  (small; switch to `SmallVec` or fixed-size array if it shows
  up in profiling); two-step caller logic (ask the pane, then
  resolve action against returned rects); pane has to either
  retain action rects between renders or recompute them inside
  `hit_test`.

  Tradeoffs rejected: option (a) — adding `col: u16` to the
  signature — burdens 12 panes that ignore it for one pane's
  benefit. Option (c) — keeping action regions on a separate
  push-during-render path — preserves a push-model exception
  inside the inversion, which is exactly what 10.3 is removing.
- **Z-order across panes.** _Decided: option (a) — explicit z-list
  constant owned by the click dispatcher._

  ```rust
  const PANE_Z_ORDER: [PaneId; 13] = [
      PaneId::Toasts,    // top
      PaneId::Finder,
      PaneId::Settings,
      PaneId::Keymap,
      // ... content panes below ...
      PaneId::ProjectList,
      // ...
  ];

  fn dispatch_click(panes: &Panes, mouse: Pos) -> Option<HoverTarget> {
      for id in PANE_Z_ORDER {
          if let Some(hit) = panes.pane(id).hit_test_at(mouse) {
              return Some(hit);
          }
      }
      None
  }
  ```

  Picked because it's one-to-one with the concept (a list of
  `PaneId` in stacking order), single source of truth, and the
  dispatcher loop reads top-to-bottom matching the constant. The
  cost — keeping `PANE_Z_ORDER` in sync with `PaneId` — is bounded
  by a unit test that asserts each `PaneId` variant appears exactly
  once in the constant.

  Tradeoffs rejected: (b) two ordered lists invents an
  "overlays vs content" category nothing else uses; (c) per-pane
  `z_order()` distributes stacking across 13 impls and lets two
  panes silently claim the same z; (d) tying z-order to
  `LayoutCache` couples stacking to layout state that moves
  independently.
- **Overlay panes (Toasts/Settings/Finder/Keymap).** _Decided:
  option (c) — single trait method takes raw position; panes
  internally convert to whatever native unit they use._

  ```rust
  trait Pane {
      fn render(&mut self, ...);
      fn hit_test_at(&self, pos: Pos) -> Option<HoverTarget>;
  }
  ```

  Row panes use a default helper on `Pane` to convert `Pos` →
  local row (using their stored area + scroll offset), then run
  row logic. Overlays walk their own per-widget rects. The trait
  stays at one abstraction level — "where in your area did this
  click land?" — and each pane answers in its own terms.

  This decision pre-resolves Q6: `pos → row` lives on the pane via
  a default helper, not in the dispatcher. The dispatcher walks
  `PANE_Z_ORDER` and passes raw `Pos` to each pane's
  `hit_test_at` until one returns `Some`.

  Tradeoffs accepted: 12 row panes call the default helper for
  `Pos → row` conversion (one line each, deduplicated by the
  helper). Each pane needs access to its own area + scroll
  offset at hit-test time — already true today via the
  `Viewport` field plus the tiled `Rect` stored on the pane after
  render.

  Tradeoffs rejected: (a) forcing every overlay through
  `hit_test(row)` requires fake row indices for Settings / Finder
  / Keymap. (b) two trait methods (`hit_test_row` and
  `hit_test_at`) with default `None` lets a pane silently forget
  to implement either and become unclickable. (d) leaving
  overlays outside the trait keeps a hand-rolled path in the
  dispatcher, partially defeating the carve.
- **Trait surface impact (default impl vs `Hittable` sub-trait).**
  _Decided: option (c) — split into `Pane` + `Hittable: Pane`
  sub-trait. Only clickable panes implement `Hittable`._

  ```rust
  trait Hittable: Pane {
      fn hit_test_at(&self, pos: Pos) -> Option<HoverTarget>;
  }

  #[derive(strum::EnumIter, Clone, Copy, PartialEq, Eq, Hash, Debug)]
  enum HittableId {
      Toasts, Finder, Settings, Keymap, ProjectList,
      Package, Lang, Git, Targets, Lints, CiRuns,
  }

  const HITTABLE_Z_ORDER: [HittableId; 11] = [
      HittableId::Toasts, HittableId::Finder,
      HittableId::Settings, HittableId::Keymap,
      HittableId::ProjectList, HittableId::Package,
      HittableId::Lang, HittableId::Git, HittableId::Targets,
      HittableId::Lints, HittableId::CiRuns,
  ];

  impl Panes {
      fn hit_test_at(&self, pos: Pos) -> Option<HoverTarget> {
          for id in HITTABLE_Z_ORDER {
              let pane: &dyn Hittable = match id {
                  HittableId::Toasts      => &self.toasts,
                  HittableId::Finder      => &self.finder,
                  HittableId::Settings    => &self.settings,
                  HittableId::Keymap      => &self.keymap,
                  HittableId::ProjectList => &self.project_list,
                  HittableId::Package     => &self.package,
                  HittableId::Lang        => &self.lang,
                  HittableId::Git         => &self.git,
                  HittableId::Targets     => &self.targets,
                  HittableId::Lints       => &self.lints,
                  HittableId::CiRuns      => &self.ci_runs,
              };
              if let Some(hit) = pane.hit_test_at(pos) { return Some(hit); }
          }
          None
      }
  }

  #[test]
  fn z_order_covers_every_hittable_id() {
      use strum::IntoEnumIterator;
      let in_order: HashSet<HittableId> = HITTABLE_Z_ORDER.iter().copied().collect();
      let all: HashSet<HittableId> = HittableId::iter().collect();
      assert_eq!(in_order, all);
  }
  ```

  Three drift guards, two compile-time, one CI-time:
  1. Match on `HittableId` is exhaustive — adding a variant forces
     a match arm update at compile time.
  2. The `&dyn Hittable` cast in each arm rejects non-Hittable
     types at compile time — you cannot put a non-clickable pane
     in the dispatch match.
  3. The strum-backed unit test catches "added a `HittableId`
     variant but forgot to put it in `HITTABLE_Z_ORDER`" at CI
     time. Strum is already a project dependency.

  Tradeoffs accepted: a sub-enum (`HittableId`) parallel to the
  Hittable trait impls; one residual drift direction not closed by
  the type system (impl-without-HittableId-variant) — pragmatically
  unlikely because you only write `impl Hittable for X` when you
  want X in dispatch.

  Tradeoffs rejected: (a) default-`None` `hit_test_at` on `Pane`
  trades a real safety property (compile-time exhaustiveness) for
  a few keystrokes saved. (b)/(d) costs ~6 lines of explicit
  `None` stubs but hides "is this pane clickable?" inside each
  pane's body instead of in a single named trait. The
  type-system-pulling-weight option costs a sub-enum and earns
  a place to grow `Hittable` into a richer family later
  (`tab_target`, `default_action`) without forcing every pane
  to write stubs. Heavier alternatives (linkme/inventory crate
  registries, custom proc macros) close the impl-without-variant
  direction at compile time but add machinery disproportionate to
  this codebase's size.
- **mouse_y → row conversion ownership.** _Decided: pane owns the
  conversion via a default helper on `Viewport`; dispatcher does
  not precompute._

  Q4's decision (raw `Pos` into `hit_test_at`) already implies
  this. Recording explicitly:

  ```rust
  impl Viewport {
      /// Convert a screen-space position to a local row within
      /// this viewport's content area, accounting for scroll
      /// offset. Returns `None` if `pos` is outside the content
      /// area.
      pub fn pos_to_local_row(&self, pos: Pos) -> Option<usize> {
          if !self.content_area.contains(pos) { return None; }
          let visual_row = pos.y.saturating_sub(self.content_area.y);
          Some(self.scroll_offset + usize::from(visual_row))
      }
  }
  ```

  Row-grid Hittable panes call
  `self.viewport.pos_to_local_row(pos)?` at the top of their
  `hit_test_at` and run row logic. Overlays
  (Toasts/Settings/Finder/Keymap) don't call the helper; they walk
  their own widget rects directly.

  Tradeoffs accepted: `hit_test_at` is implicitly coupled to
  "render has run at least once" — `Viewport::content_area` and
  `scroll_offset` are written by the render pass each frame. A
  pane queried before its first render has `Viewport::default()`
  and returns `None`, which is the correct silent-no-op
  behavior.

  Tradeoffs rejected: dispatcher precomputing
  `(pane_id, local_row)` would require a `Pane::viewport()` trait
  method and split the conversion logic between dispatcher and
  pane — two places to change when the conversion rule changes.

## End of Phase 10.3 design — implementation can begin

All six design questions are settled. The implementation that
follows is mechanical:

1. Add `Hittable: Pane` sub-trait, `HittableId` enum (with
   `strum::EnumIter`), `HITTABLE_Z_ORDER` constant.
2. Add `Viewport::pos_to_local_row` helper.
3. Add `Panes::hit_test_at(pos) -> Option<HoverTarget>` walking the
   z-order list with the exhaustive `HittableId` match.
4. Implement `Hittable` on the 11 clickable panes (row panes
   convert `Pos` → row via the helper; overlays walk widget
   rects).
5. Rewrite `interaction.rs::handle_click` and
   `hovered_pane_row_at` to call `Panes::hit_test_at` instead of
   reading the `pane_manager` row-hitbox map.
6. Delete `HitboxSink`, `PaneRenderCtx::hit_sink`, the four
   `register_*_row_hitboxes` helpers, and the `pane_manager`
   row-hitbox map.
7. Land the strum-backed `z_order_covers_every_hittable_id`
   unit test.

- **Autonomously-added `#[allow]` markers from Phase 7 need user
  review.** *Resolved in Phase 8.15.* The Phase-7 `#[allow(dead_code)]`
  markers covered placeholder trait surface (`Pane::id`,
  `input_context`, `has_row_hitboxes`, `size_spec`, `handle_input`,
  `is_navigable`, plus `PaneInputCtx`, `PaneNavCtx`,
  `InputContextKind`, the `Panes::pane`/`pane_mut` registry, and
  `PaneRenderCtx::focused_pane`/`selection`). Phase 8.15 trimmed
  the trait + ctx surface to its live shape: only `fn render` on
  the trait, only the `PaneRenderCtx` fields render bodies actually
  read. Phase 9 reintroduces what it needs when each remaining
  pane absorbs state and a render body — additions that are
  driven by working code, not by speculative scaffolding.

## Phase 11.0 Lint subsystem design (in progress)

Re-review of Phase 11 (Lint) carve plan against everything
learned in Phases 1–10. Focus surfaced by user: the current
`resolve_lint_display` machinery (six App accessors funneling
into a 17-line stringifier with a duplicated worktree fan-out)
is overly complex; pre-computation is fine, but seek
simplifications. Decisions are recorded below as each is
settled.

- **Q1: Lint icon API surface.** _Decided: option (a) — three
  typed functions on `Lint`, each returning unframed
  `LintStatus`._

  ```rust
  impl Lint {
      pub fn status_for_path(&self, projects: &ProjectList, path: &Path) -> LintStatus;
      pub fn status_for_root(&self, item: &RootItem) -> LintStatus;
      pub fn status_for_worktree(&self, item: &RootItem, wi: usize) -> LintStatus;
  }
  ```

  Animation framing leaves the API entirely; callers already
  have `animation_elapsed` via `PaneRenderCtx` after Phase 8.
  The 30-line `VisibleRow` match in
  `App::selected_lint_icon` deletes — call sites know which
  row variant they're rendering and pick the matching function
  directly.

  Tradeoffs accepted: three functions instead of one, but each
  function body is 1–2 lines and call sites read directly
  (e.g., `lint.status_for_root(item)`).

  Tradeoffs rejected: option (b) — one
  `Lint::status(LintTarget)` function — forces every caller to
  construct a wrapper enum with no producer-side gain (the body
  is still an exhaustive match on three disjoint inputs).

- **Q2: `LintDisplay` enum design.** _Decided: typed enum lands
  in Phase 11 (option (i)) and carries `count: usize`._

  ```rust
  /// Display value for the Lint row in the Package detail pane.
  /// Pattern: typed display values, not pre-rendered strings.
  pub enum LintDisplay {
      NotRust,
      NoRuns,
      Runs { count: usize, status: LintStatus },
  }

  impl Lint {
      pub fn package_display(
          &self,
          projects: &ProjectList,
          abs: &AbsolutePath,
          is_worktree_group: bool,
      ) -> LintDisplay;
  }
  ```

  `PackageData.lint_display` flips from `String` to `LintDisplay`
  inside Phase 11. The Package renderer matches on variants,
  applying `animation_elapsed` to `status.icon()` at render
  time. The `NO_LINT_RUNS` / `NO_LINT_RUNS_NOT_RUST` constants
  and the `format\!("{icon} {n}")` move into the renderer. The
  string stringifier `resolve_lint_display` is deleted.

  Tradeoffs accepted: Package renderer changes in Phase 11
  (small — one match on three variants). The `Ci` half of the
  capstone (Phase 13.last) still has to flip
  `PackageData.ci_display` separately, so 13.last loses the
  "both at once" framing — but Lint's stale-spinner accident
  goes away two phases earlier. `count: usize` keeps
  `LintDisplay` Copy-cheap and decoupled from `LintRuns`; the
  renderer has no use for the runs themselves.

  Tradeoffs rejected: option (ii) — keep `lint_display: String`
  on `PackageData` until 13.last — pays the cost of building
  the typed enum then immediately throwing it away, and leaves
  the load-bearing animation accident in place across two more
  phases.

- **Q3: Worktree-group fan-out primitive.** _Decided:
  iterator-returning helpers on `WorktreeGroup` plus a single
  `Lint::run_count_at` collapse._

  ```rust
  impl<Kind: ProjectFields> WorktreeGroup<Kind> {
      /// Iterate primary + linked checkouts in canonical order.
      pub fn iter_entries(&self) -> impl Iterator<Item = &Kind>;
      pub fn iter_paths(&self) -> impl Iterator<Item = &Path>;
  }

  impl Lint {
      /// Run count at `path`, or 0 when no lint history exists.
      pub fn run_count_at(&self, path: &Path) -> usize;
  }
  ```

  Lint call site:

  ```rust
  let count: usize = group.iter_paths().map(|p| lint.run_count_at(p)).sum();
  ```

  Single-project case: `lint.run_count_at(path)` directly.
  `lint_run_count_for` and the open-coded `WorktreeGroup` match
  in `panes/support.rs` are deleted.

  Driving evidence: the `primary, linked, ..` destructure
  shows up 16 times in `worktree_group.rs` alone and 30+ more
  across `project_list.rs`, `snapshots.rs`, `root_item.rs`,
  `scan.rs`, `finder.rs`. The iterator primitive is a single
  source of truth for "the order in which a group's checkouts
  are visited" and replaces existing duplication on top of
  serving the new Lint call site. As a follow-up dividend,
  several of the 16 internal duplications can collapse to
  `iter_entries().any/find/...` (the index-based
  `lint_status_for_worktree` is the exception — it stays).

  Tradeoffs accepted: `impl Iterator<Item = …>` is one line of
  API instead of a concrete `Vec`. One normal Rust idiom, no
  allocation per call.

  Tradeoffs rejected: option (a) `fold_checkouts<T>(closure)`
  ties iteration to a single generic reduction; doesn't
  generalize across the varied existing reductions. Option (b)
  `checkout_paths() -> Vec<&Path>` allocates per call and
  doesn't address the broader existing duplication. Option (c)
  leave it inline preserves the duplication that already
  burdens the codebase.

- **Q4: `lint_runtime` ownership.** _Decided: option (a) —
  relocate from `Inflight` into `Lint`._

  ```rust
  pub struct Lint {
      runtime: Option<RuntimeHandle>,
      // …
  }
  impl Lint {
      pub fn respawn_runtime(&mut self, cfg: &LintConfig);
      pub fn runtime(&self) -> Option<&RuntimeHandle>;
      pub fn start_lint(&mut self, path: AbsolutePath, ctx: StartContext<'_>);
  }
  ```

  `Inflight::start_lint` migrates to `Lint::start_lint`; the
  `lint_runtime` / `lint_runtime_clone` / `set_lint_runtime`
  trio on `Inflight` is deleted. `App::apply_lint_config_change`
  and `refresh_lint_runtime_from_config` collapse into
  `lint.respawn_runtime(cfg)`. `Inflight` retains only the
  inflight-tracking fields (running paths, toasts, pending
  queues), so its name matches its contents.

  Tradeoffs accepted: this reverses the Phase 4 placement
  decision. Justified because Phase 4 placed runtime next to
  start (`Inflight::start_lint`) before `Lint` existed; now
  that `Lint` is the carve target, "runtime co-located with
  start" leads to the same conclusion with `Lint` as the home.

  Tradeoffs rejected: option (b) leave on `Inflight` —
  `Lint::start_lint` would have to reach into `Inflight` for
  the runtime, splitting a coherent unit (spawn machinery)
  across two subsystems for no reason except history. Option
  (c) App-shell ownership — most plumbing, no clear winner.

- **Q5: Module placement.** _Decided: option (a) — `LintStatus`
  in `crate::lint`, `LintDisplay` in `tui::lint_state`._

  | Type | Module | Reason |
  |---|---|---|
  | `LintStatus` (rollup state — `NoLog`/`Running`/`Passed`/`Failed`/`Mixed` etc.) | `crate::lint` | Domain concept; consumed by both project tree (`root_item::lint_rollup_status`) and `tui::lint_state`. |
  | `LintDisplay` (typed display value for the Package row) | `tui::lint_state` | Consumer-specific; only the Package detail pane uses it. Lives next to `Lint::package_display`. |

  Layering rule established: **domain types in `crate::lint`,
  consumer-specific display types in `tui::lint_state`.** Phase
  13 mirrors with `CiStatus` (in `crate::ci`) vs. `CiDisplay`
  (in `tui::ci_state`).

  Tradeoffs accepted: two homes for two types instead of one.
  Aligns with the existing import direction (TUI depends on
  domain, not the other way around).

  Tradeoffs rejected: option (b) put both in `crate::lint` —
  drags TUI display vocabulary into the domain crate. Option
  (c) put both in `tui::lint_state` — would force
  `crate::project::root_item` to depend on `tui::lint_state`,
  inverting the layering.

- **Q6: Per-row caller routing.** _Decided: option (a) — delete
  `App::selected_lint_icon`. The `Lint::package_display` body
  branches on `is_worktree_group` directly._

  ```rust
  impl Lint {
      pub fn package_display(
          &self,
          projects: &ProjectList,
          abs: &AbsolutePath,
          item: &RootItem,
          is_worktree_group: bool,
      ) -> LintDisplay {
          if \!self.is_rust_at(item) { return LintDisplay::NotRust; }
          let status = if is_worktree_group {
              self.status_for_root(item)
          } else {
              self.status_for_path(projects, abs.as_path())
          };
          let count = if is_worktree_group {
              item.as_worktree_group()
                  .map(|g| g.iter_paths().map(|p| self.run_count_at(p)).sum())
                  .unwrap_or_else(|| self.run_count_at(abs.as_path()))
          } else {
              self.run_count_at(abs.as_path())
          };
          match (count, status) {
              (0, _) => LintDisplay::NoRuns,
              (n, s) => LintDisplay::Runs { count: n, status: s },
          }
      }
  }
  ```

  `App::selected_lint_icon` is deleted. Its only consumer
  (`resolve_lint_display`) is also deleted. ProjectList row
  rendering already calls the per-row functions directly today
  — this preserves that direct pattern and extends it to the
  Package detail.

  Tradeoffs accepted: branching on `is_worktree_group` lives
  inside `Lint::package_display`. The caller still passes the
  bool, but doesn't construct a routing enum.

  Tradeoffs rejected: option (b) `Lint::selected_status(VisibleRow)`
  pulls TUI-shell types (`VisibleRow`) into the lint subsystem,
  inverting the layering for one consumer. Option (c) free
  function in `panes/support.rs` adds a multi-hop layer with
  no caller benefit.

- **Q7: Lint field cluster.** _Decided: full cluster moves to
  `Lint`._

  ```rust
  pub struct Lint {
      runtime:       Option<RuntimeHandle>,
      running_paths: HashMap<AbsolutePath, Instant>,
      running_toast: Option<ToastTaskId>,
      cache_usage:   CacheUsage,
      startup_phase: CountedPhase,
      active_phases: KeyedPhase<AbsolutePath>,
  }
  ```

  | Field | Source | Reason |
  |---|---|---|
  | `runtime` | `Inflight` | Q4 decision. |
  | `running_paths` | `Inflight` | Co-locates with `start_lint` / `finish_lint`. |
  | `running_toast` | `Inflight` | Same; the toast is the "N lints running" indicator. |
  | `cache_usage` | `Scan` | Field is literally `lint_cache_usage`; `refresh_lint_cache_usage_from_disk` is a lint-spawn job. |
  | `startup_phase`, `active_phases` | App's `Phases` struct | Both fields are literally `lint*`. App's `Phases` keeps the non-lint counters. |

  **Stays where it is:**
  - Per-project `LintRuns` on `ProjectList` — tree-owned data;
    Lint queries through `projects.lint_at_path`. Moving it
    would invert the project-tree → lint dependency.
  - `is_rust_at_path` — project-tree query.
  - `selected_row_owns_lint` — selection-row query.
  - `lint_enabled()` — config flag, already on `Config`.

  Driving principle: anything literally named `lint_*` belongs
  on `Lint`. The exception is per-project run history, which
  the project tree owns and Lint queries.

  Tradeoffs accepted: this phase relocates more fields than
  the minimum. Justified because the alternative — leaving
  some lint-specific fields on `Inflight`/`Scan`/`App` — is
  just deferred work, not a design boundary, and would require
  cross-subsystem reaches that a follow-up would un-do.

## End of Phase 11.0 design — implementation can begin

All seven design questions are settled. Implementation outline:

1. New module `tui::lint_state` with `Lint` struct + the field
   cluster from Q7.
2. New types: `LintStatus` in `crate::lint` (Q5);
   `LintDisplay` in `tui::lint_state` (Q5).
3. New iterator helpers on `WorktreeGroup`: `iter_entries`,
   `iter_paths` (Q3). Use the helpers to absorb internal
   duplication in `worktree_group.rs` opportunistically.
4. New functions on `Lint` (Q1, Q2, Q3, Q4, Q6):
   `status_for_path` / `status_for_root` / `status_for_worktree`
   (returning unframed `LintStatus`), `run_count_at`,
   `package_display`, `respawn_runtime`, `start_lint`,
   `finish_lint`, plus the relocated-from-App handlers
   (`apply_config_change`, `refresh_runs_from_disk`,
   `reload_history`, `refresh_cache_usage_from_disk`,
   `lint_runtime_projects_snapshot`, `sync_runtime_projects`,
   `register_lint_for_*`, `maybe_complete_startup_lints`,
   `startup_toast_body_for`, `sync_running_toast`,
   `handle_*_msg`).
5. Delete: `App::lint_icon`, `App::lint_icon_for_root`,
   `App::lint_icon_for_worktree`, `App::selected_lint_icon`,
   `panes::support::resolve_lint_display`,
   `panes::support::lint_run_count_for`,
   `Inflight::lint_runtime` / `_clone` / `set_*`,
   `Scan::lint_cache_usage` / `set_lint_cache_usage` (relocated).
6. Flip `PackageData.lint_display` from `String` to
   `LintDisplay`. Update `panes::package` renderer to match on
   variants and apply `animation_elapsed` to `status.icon()`
   at render time. Delete the `NO_LINT_RUNS` /
   `NO_LINT_RUNS_NOT_RUST` string constants from the package
   pane's render path (they may still be used elsewhere — keep
   only if other consumers exist).
7. Tests update: lint-related characterization tests in
   `tui::app::tests::state` move alongside the new module or
   stay where they are with paths updated.

Per-phase workflow applies (write tests first, develop,
validate with `cargo build` + `cargo nextest run` +
`cargo clippy --all-targets` + `cargo +nightly fmt` +
`cargo install --path .`, stop for review before committing).

