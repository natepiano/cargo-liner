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
| **Panes** (`tui::panes::Panes`) | `pane_manager`, `pane_data`, `visited_panes`, `layout_cache`, `worktree_summary_cache`, `hovered_pane_row`, `ci_display_modes`, `cpu_poller` | `refresh_for_selection`, `render(frame, focused)`, `handle_input(&mut self, app: &mut App, …)` (Phase 1: still takes `&mut App`; revisit in later phase per "Rollback / revisit policy"), `clear_for_tree_change`, `cpu_tick`, `set_hover`, `toggle_ci_display_mode_for(path)`, `apply_hovered_pane_row` |
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

That's roughly 12 fields instead of 60 after phases 1–8. After Phase
7 only (before Net carves), the count is ~16 — the difference is the
network state cluster, which moves out in Phase 8.

### Two axes of structure inside `Panes`

- **App → `Panes` boundary**: strict delegation, no trait. Single owner, single
  caller, concrete struct. `app.panes: Panes`.
- **`Panes` → individual pane behavior**: a `Pane` trait, landing in
  **Phase 7** (not Phase 1). Phase 1 absorbs the field cluster — that's the
  prerequisite. Phase 7 is what actually fixes the per-pane god-object
  problem: every concrete pane (`CiPane`, `CpuPane`, `GitPane`, `LintsPane`,
  `PackagePane`, `LangPane`) gets its own file with one `impl Pane` block,
  and the match-on-`PaneId` arms in `panes/spec.rs`, `panes/actions.rs`,
  `panes/support.rs`, and per-pane render files collapse into trait
  dispatch.

  By Phase 7 all the other subsystems exist as proper types, so trait
  methods can take typed subsystem references (e.g.,
  `Pane::handle_input(&mut self, &mut Selection, &mut Background, &mut
  Inflight, &Config, &mut Scan, &mut ToastManager, event)`). The
  parameter list is dependency injection, not a god-object handle —
  encapsulation by file (each pane's behavior in one place named for the
  pane) is the win.

  Phase 1 (field cluster only) and Phase 7 (per-pane trait split) are
  the two halves of the same fix. The plan does both; it does not stop
  at Phase 1.

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
   both shapes — not as speculative infrastructure. Lints / package /
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
7. **Pane catalog rewrite** — the actual fix to the per-pane god-object
   problem. Introduce a `Pane` trait, give every concrete pane its own
   file with one `impl Pane` block, and collapse the match-on-`PaneId`
   arms scattered through `panes/spec.rs`, `panes/actions.rs`,
   `panes/support.rs`, and the per-pane render files into trait dispatch.
   By this phase all the other subsystems exist as proper types, so
   trait methods take typed subsystem references (`&mut Selection,
   &mut Background, &mut Inflight, &Config, &mut Scan, &mut
   ToastManager`) — that's just dependency injection, not a god-object.
   Encapsulation by file is the win; each pane's behavior lives in one
   place named for the pane.

   **Phase 7 begins with a re-review of the phase plan against
   everything learned in Phases 1–6.** Subsystem APIs may have shifted,
   the per-pane code's actual shape may have changed under us, and the
   trait signature drafted in this doc is a starting point, not a
   contract. The first task in Phase 7 is to revisit this section and
   propose updates based on what the prior phases produced, before
   writing any pane impl code.
8. **Net subsystem** — extract the network-state cluster (`http_client`,
   `github`, `crates_io`) into its own subsystem. Today these three
   fields together carry the HTTP client and rate-limit state, the
   GitHub repo-fetch cache plus in-flight fetch tracking plus
   availability tracker, and the crates.io availability tracker. They
   share the HTTP client, are read by the Git pane, the project tree,
   and the rate-limit display, and overlap with two other subsystems
   (Inflight and Scan). After Phase 8, App stops owning any
   network-related state directly.

   **Phase 8 begins with a re-review of the phase plan against
   everything learned in Phases 1–7.** The skeleton in this doc was
   drafted before any of the prior phases existed; the actual
   `running_fetches` / `fetch_cache` overlaps with Inflight and Scan
   may have resolved themselves along the way, the `availability`
   tracking may have moved, and the public API drafted below is a
   starting point. Update this section and get user approval before
   writing carve code.

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
problem one cluster down. The plan now finishes the carve.)**

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
  - Phase 1 absorbs the field cluster only. The per-pane trait split is
    Phase 7, not deferred indefinitely — see "Two axes of structure
    inside `Panes`" above.
  - `hovered_pane_row` lives in `Panes`. Hit-testing in Phase 1 is a
    match-on-`PaneId` inside `Panes`; collapses into `Pane::hit_test`
    trait dispatch in Phase 7.
  - `Panes::handle_input` in Phase 1 keeps the `&mut App` shape that
    `panes/actions.rs` currently uses (every dispatch in
    `panes/actions.rs:32-336` reaches across ~6 of the 7 future
    subsystems). Phase 7 replaces this with `Pane::handle_input`
    taking typed subsystem references — at that point `panes/actions.rs`
    as a free-function module ceases to exist, replaced by per-pane
    `impl Pane` blocks.
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
    with the two consumers that prove it works for both shapes
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

  - **Inner shape of `TreeMutation` after the carve.**
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

- **Phase 7 (Pane catalog rewrite)**:
  - **First task: re-review the Phase 7 plan.** Before writing any
    `impl Pane` code, revisit this section, the `Panes` table row, and
    the "Two axes of structure inside `Panes`" section against the
    actual state of Phases 1–6 as committed. Subsystem APIs may have
    moved, the per-pane code's shape may have evolved, and the trait
    signature drafted in this doc is a starting point. Update the doc
    to reflect what was learned, get the user's approval on the
    revisions, then start implementing.
  - **Trait shape (starting point, subject to Phase 7 re-review).**
    ```rust
    pub trait Pane {
        fn id(&self) -> PaneId;
        fn render(&mut self, frame: &mut Frame, area: Rect, ctx: PaneRenderCtx<'_>);
        fn hit_test(&self, row: u16) -> Option<HoverTarget>;
        fn handle_input(&mut self, event: &KeyEvent, ctx: PaneInputCtx<'_>) -> InputOutcome;
        fn refresh_for_selection(&mut self, ctx: PaneRefreshCtx<'_>);
    }
    ```
    The `PaneRenderCtx`, `PaneInputCtx`, `PaneRefreshCtx` structs each
    bundle the typed subsystem references the method needs (for
    example, `PaneInputCtx { selection: &mut Selection, background:
    &mut Background, inflight: &mut Inflight, config: &Config, scan:
    &mut Scan, toasts: &mut ToastManager }`). Bundling the parameters
    in named structs keeps signatures readable and gives a single
    place to add dependencies if a future pane behavior needs more.
  - **Per-pane files.** One file per concrete pane:
    `panes/ci.rs`, `panes/cpu.rs`, `panes/git.rs`, `panes/lints.rs`,
    `panes/package.rs`, `panes/lang.rs`. Each contains the struct
    definition, the `impl Pane` block, and any private helpers. Today
    several of these files exist but only contain render code; Phase 7
    expands them to own the full behavior.
  - **What collapses.** `panes/actions.rs` (~336 lines of free
    functions taking `&mut App` plus a top-level dispatch match) and
    the match-on-`PaneId` arms in `panes/spec.rs` and `panes/support.rs`
    disappear. Their bodies move into the relevant `impl Pane`
    methods. The dispatch becomes `panes.with_focused_mut(|pane|
    pane.handle_input(event, ctx))` inside `Panes`.
  - **Hover hit-testing.** Each pane implements `hit_test` to return
    its own `HoverTarget`. Today this logic is inlined per-pane via
    `register_*_row_hitboxes` helpers writing into `pane_manager`
    during render — Phase 7 cleans this up so hit-testing is a query,
    not a render side-effect.
  - **`PaneId` enum stays.** It's still the index used by App, by
    `Panes` for focus and lookup, and by callers asking "which pane is
    selected?" Trait dispatch happens through `Panes`'s storage of
    `Vec<Box<dyn Pane>>` (or a typed array indexed by `PaneId`,
    decided at re-review time), keyed by `PaneId`.
  - **What stays in `panes/data.rs`.** The data registry
    (`PaneDataStore`) that caches per-pane data computed by builders
    is orthogonal to per-pane behavior — it stays as is. Phase 7 is
    about behavior, not data.

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

- **Phase 8 (Net subsystem)**:
  - **First task: re-review the Phase 8 plan.** Before writing carve
    code, revisit the "Net subsystem (Phase 8 skeleton)" section and
    the field-appendix entries against the actual state of Phases
    1–7. Three open questions to answer at re-review time, captured
    in that section:
    1. Do `running_fetches` / `running_fetch_toast` move into
       `Inflight`, or stay in `Net`?
    2. Does `fetch_cache` belong in `Net` (HTTP-coupled) or `Scan`
       (tree-enrichment cache)?
    3. Does `availability` collapse into `Net::availability(service)`
       or stay per-service?
    Update the section, get user approval on the answers, then
    implement.
  - **Phase 8 starts after Phase 7.** All other subsystems exist by
    then, so `Net` can take typed references to whatever it depends
    on (today: `Inflight` for the running-fetches question, `Scan`
    for the fetch-cache question) without re-introducing god-object
    parameters.
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
| `http_client` | Net (Phase 8) |
| `github` (`GitHubState`: `fetch_cache`, `repo_fetch_in_flight`, `running_fetches`, `running_fetch_toast`, `availability`) | Net (Phase 8); `running_fetches`/`running_fetch_toast` may move to Inflight at Phase 8 re-review |
| `crates_io` (`CratesIoState`) | Net (Phase 8) |
| `projects` | Scan |
| `ci_fetch_tracker` | Inflight |
| `ci_display_modes` | Panes (per-pane render preference) |
| `lint_cache_usage` | Scan (cache-stat counter, not in-flight bookkeeping) |
| `discovery_shimmers` | Scan (with future-revisit note: may move to Inflight) |
| `pending_git_first_commit` | Scan (with same future-revisit note) |
| `cpu_poller` | Panes (CPU pane is its sole reader; ticked via `panes.cpu_tick`) |
| `bg_tx`/`bg_rx` | Background (with rescan-swap caveat; see above) |
| `priority_fetch_path` | Scan |
| `expanded` | Selection |
| `pane_manager` | Panes |
| `pane_data` | Panes |
| `settings_edit_buf`/`settings_edit_cursor` | Config (combined as `SettingsEditBuffer`) |
| `focused_pane` | **App-shell** (focus stack) |
| `return_focus` | **App-shell** (focus stack) |
| `visited_panes` | Panes |
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
| `worktree_summary_cache` | Panes |
| `data_generation` | Scan |
| `mouse_pos` | **App-shell** |
| `hovered_pane_row` | Panes |
| `layout_cache` | Panes |
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

App-shell field count after the **planned 8 phases**: ~12 (focus stack
+ modal/UI shell + 8 subsystem handles). Down from ~60. Phase 7 (pane
catalog rewrite) reorganizes per-pane *behavior* but does not add or
remove App fields. Phase 8 (Net carve) removes the last residual cluster
of network state from App-shell, taking the count from ~16 (after
Phase 7) down to ~12.

### `Net` subsystem (Phase 8 skeleton — subject to re-review)

`http_client` + `github` (`GitHubState`) + `crates_io` (`CratesIoState`)
together form an eighth subsystem. This is a **skeleton only** — Phase
8 begins with a re-review of this section against everything learned in
Phases 1–7 (the `running_fetches` / `Inflight` overlap and the
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
(`fetch_cache` feeds tree enrichment). Phase 8's re-review must answer:

- Do GitHub repo fetches go through `Inflight` (uniform "in-flight
  tracker" pattern) or stay in `Net` (HTTP-coupled)? Today
  `running_fetches` + `running_fetch_toast` are GitHub-specific; if
  Inflight has matured into a generic `start_/finish_` shape by
  Phase 8, moving them is the right call. If not, leave them in `Net`.
- Does `fetch_cache` belong in `Net` (HTTP-coupled, response cache)
  or in `Scan` (tree-enrichment cache, keyed by repo path)? Today it
  lives on App; Phase 8's re-review picks a home.
- Does the `availability` tracker (currently per-service in `github`
  and `crates_io`) collapse into a single `Net::availability(service)`
  query, or stay as separate fields?

After Phase 8 there is **no residual network state on App-shell**.
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
  shape stays.
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
  documentation requirement** (same shape as the mutation-guard rule
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
- **Move into `Panes`:** `apply_hovered_pane_row`
  (`tui/app/mod.rs:278-286`), `toggle_ci_display_mode_for`
  (`mod.rs:565`), `focus_pane`, `register_*_row_hitboxes` helpers (today
  in `panes/git.rs` etc., already pane-local).
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
  subsystems). Phase 7 replaces it wholesale with `Pane::handle_input`
  taking typed subsystem references — that's the dedicated phase for
  this work, not a mid-phase revision. The intermediate phases (3–6)
  may layer in narrower facade methods, but the structural rewrite is
  Phase 7.
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
```

Phase rules:

- **Phase 1** lands the initial pattern index covering "Mutation guard
  (RAII) — fan-out flavor" referencing the existing `TreeMutation`,
  plus stub entries for the other patterns marked "lands in Phase N."
- **Each later phase** that introduces a new instance of a pattern
  updates the App-module doc comment to point at the new canonical
  example (or fills in the stub if it's the first instance).
- **Each later phase** that introduces a *new* pattern adds a new
  section to the App-module doc comment with the same shape.

The patterns themselves:

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
  later phase copy that doc shape — no comment is shorter, no
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
  that doc shape.
