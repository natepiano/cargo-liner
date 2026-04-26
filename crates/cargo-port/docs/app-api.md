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
| **Panes** (`tui::panes::Panes`) | `pane_manager`, `pane_data`, `visited_panes`, `layout_cache`, `worktree_summary_cache`, `hovered_pane_row` | `refresh_for_selection`, `render(frame, focused)`, `handle_input`, `clear_for_tree_change`, `cpu_tick`, `set_hover` |
| **Selection** (`tui::selection::Selection`) | `cached_visible_rows`, `cached_root_sorted`, `cached_child_sorted`, `cached_fit_widths` (typed as `ProjectListWidths` after step 2), `selection_paths`, `selection`, `expanded`, `finder` | Direct: `visible_rows`, `cursor_row`, `move_cursor`, `select_path`, `fit_widths`, `selected_paths`, `mutate(&projects) -> SelectionMutation<'_>`. On `SelectionMutation`: `toggle_expand`, `apply_finder` (recompute fires on drop). |
| **Background** (`tui::background::Background`) | All four mpsc tx/rx pairs (`bg_*`, `ci_fetch_*`, `clean_*`, `example_*`), `lint_runtime`, `watch_tx` | `poll_all -> PendingMessages`, `send_watcher`, `spawn_lint`, `spawn_clean`, `spawn_ci_fetch`, `spawn_example`, `bg_sender` |
| **Inflight** (`tui::inflight::Inflight`) | `running_clean_paths`, `running_lint_paths`, `clean_toast`, `lint_toast`, `ci_fetch_toast`, `ci_fetch_tracker`, `pending_cleans`, `pending_ci_fetch`, `pending_example_run`, `example_running`, `example_child`, `example_output` | `start_clean(path, &mut toasts) -> StartOutcome`, `finish_clean`, `start_lint`, `finish_lint`, `start_ci_fetch`, `finish_ci_fetch`, `start_example`, `kill_example`, `is_clean_running`, `is_lint_running`, `queue_clean`, `drain_next_pending_clean` |
| **Config** (`tui::config_state::Config`) | `current_config`, `config_path`, `config_last_seen`, `settings_edit_buf`, `settings_edit_cursor` (last two combined as `SettingsEditBuffer`) | `current`, `try_reload(&mut toasts) -> ReloadOutcome`, `begin_settings_edit -> &mut SettingsEditBuffer`, `commit_settings_edit(&mut toasts) -> CommitOutcome`, `discard_settings_edit` |
| **Keymap** (`tui::keymap_state::Keymap`) | `current_keymap`, `keymap_path`, `keymap_last_seen`, `keymap_diagnostics_id` | `current`, `try_reload(&mut toasts) -> ReloadOutcome` |
| **`WatchedFile<T>`** (`tui::watched_file`, generic primitive) | `path`, `stamp`, `current: T` | `current`, `path`, `try_reload(parse_fn) -> ReloadOutcome`. Composed by both `Config` and `Keymap`; not held by App directly. |
| **Scan** (`tui::scan_state::Scan`) | `projects`, `scan`, `dirty`, `data_generation`, `discovery_shimmers`, `pending_git_first_commit`, `metadata_store`, `target_dir_index`, `priority_fetch_path`, `confirm_verifying` | Direct: `projects`, `generation`, `metadata_store`, `target_dir_index`, `set_priority_fetch`, `shimmer_for`, `mark_dirty`, `apply_metadata`, `record_first_commit`, `set_confirm_verifying`, `bump_generation` (explicit, called from message-relevance code), `mutate_tree(&mut panes, &mut selection) -> TreeMutation<'_>`. On `TreeMutation`: structural ops; `Drop` fans out invalidation to Scan's caches + `panes.clear_for_tree_change()` + `selection.recompute_visibility()`. |

App keeps only:

- the **event loop** (tick, draw, input dispatch)
- the **focus stack** (`focused_pane`, `return_focus`) — popups, toasts, and
  panes all push/pop focus, so this is an App-shell concern. `Panes` is told
  who to highlight via `render(frame, focused)`.
- the **modal/UI shell** (`confirm`, `toasts`, `inline_error`, `status_flash`,
  `ui_modes`, `mouse_pos`, `animation_started`)
- **handles** to the seven subsystems above (`panes`, `selection`,
  `background`, `inflight`, `config`, `keymap`, `scan`)

That's roughly 11 fields instead of 60.

### Two axes of structure inside `Panes`

- **App → `Panes` boundary**: strict delegation, no trait. Single owner, single
  caller, concrete struct. `app.panes: Panes`.
- **`Panes` → individual pane behavior**: a `Pane` trait, one impl per concrete
  pane (`CiPane`, `CpuPane`, `GitPane`, `LintsPane`, `PackagePane`,
  `LangPane`). Methods: `render`, `hit_test(row) -> Option<HoverTarget>`,
  `handle_input`, `refresh_for_selection`. The match-on-`PaneId` arms scattered
  through today's `spec.rs` / `actions.rs` / `support.rs` collapse into trait
  dispatch.

Hover handling, for example, becomes: `Panes` holds `hovered_pane_row` as
state but resolves the hit by calling `pane.hit_test(row)` on the trait — each
pane's bespoke row-targeting logic stays inside that pane.

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

Estimated: **385 → ~30** remaining `pub(in super::super)` sites, and those will
be genuine cross-module reaches worth a second look.

## Order of execution (one PR per subsystem)

1. **Panes** first — biggest visibility win in `panes/`, and the fix the
   prior conversation has been pointing at.
2. **ColumnFit primitive** (small, standalone) — split `ResolvedWidths` into
   a generic `ColumnFit` mechanism in `tui::columns` plus a
   `ProjectListWidths` newtype. No App fields move; this prepares the ground
   so step 3 (Selection) can absorb `cached_fit_widths` as the typed
   `ProjectListWidths`, and lints/ci/git panes can reuse the fitting
   mechanism in a later sweep.
3. **Selection** — second-biggest field cluster, cleanly factored already
   (`cached_*`, `selection*`). Uses the new `ProjectListWidths` from step 2.
4. **Background + Inflight** together — entangled (a "start" hits both).
5. **Config + Keymap** (one phase) — extract shared `WatchedFile<T>`
   primitive, then carve `Config` and `Keymap` as two separate subsystems
   composing it. Tightly coupled by the primitive; extracting one without
   the other leaves the duplication in place.
6. **Scan** — last because `mutate_tree` already gates it; mostly relocation.

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

- **Step 1 (Panes)**:
  - Two-axis design: App↔Panes is concrete delegation; Panes↔individual panes
    is a `Pane` trait. Per-pane bespoke logic (hit-test, render, handle_input,
    refresh_for_selection) lives in trait impls.
  - `hovered_pane_row` lives in `Panes`, but hit-testing dispatches through
    `Pane::hit_test` so each pane's bespoke row-targeting stays local.

- **Step 2 (`ColumnWidths` primitive)**:
  - Split `ResolvedWidths` (in `tui::columns`) into:
    - generic `ColumnWidths` (in new submodule `tui::columns::widths`) —
      "given cells with min/max constraints, compute fitted column widths";
      reusable by any pane with columns.
    - `ProjectListWidths` newtype (in `tui::columns`) — wraps `ColumnWidths`
      with the project-list column schema (name, status, disk, etc.).
  - `summary_label_col` stays as a free function in `tui::columns` (not on
    `ProjectListWidths`) — likely reused by other summary rows.
  - No App fields move in this step.
  - Out of scope here: actually adopting `ColumnWidths` in lints/ci/git
    panes. That's a follow-up sweep once the primitive exists.

- **Step 3 (Selection)**:
  - `cached_fit_widths` is absorbed as `ProjectListWidths` (from step 2), not
    the bare `ResolvedWidths`.
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

- **Step 6 (Scan)**:
  - `TreeMutation::drop` fans out to all three affected subsystems (Scan's
    own caches, `panes.clear_for_tree_change()`,
    `selection.recompute_visibility()`). `mutate_tree` takes `&mut panes,
    &mut selection` so the guard holds the references it needs to fan out
    on drop. Same RAII reasoning as `SelectionMutation` — the type system
    makes it impossible to mutate the tree without all three invalidations
    firing.
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

- **Step 5 (Config + Keymap, sharing a `WatchedFile<T>` primitive)**:
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

- **Step 4 (Background + Inflight)**:
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

## Recurring patterns

- **Mutation guard (RAII)**: when a subsystem has derived/cached state that
  must be recomputed after a cluster of mutations, gate the mutating methods
  through a `&mut Self`-borrowing guard whose `Drop` runs the recompute.
  Two flavors:
  - **Self-only**: `SelectionMutation` invalidates only Selection's own
    derived state on drop.
  - **Fan-out**: `TreeMutation` borrows references to other subsystems
    (`&mut panes, &mut selection`) and invalidates each on drop. Use this
    shape when one subsystem's mutation forces invalidation across
    siblings — the borrow declares the dependency at the type level.
  Apply this pattern to any future subsystem with the same invariant.
