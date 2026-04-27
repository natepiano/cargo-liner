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
| **Inflight** (`tui::inflight::Inflight`) | `running_clean_paths`, `running_lint_paths`, `clean_toast`, `lint_toast`, `ci_fetch_toast`, `ci_fetch_tracker`, `pending_cleans`, `pending_ci_fetch`, `pending_example_run`, `example_running`, `example_child`, `example_output`, **`lint_runtime`** (relocated here from Background — `start_lint` is the only consumer; co-locating runtime with start avoids cross-subsystem reach) | `start_clean(path, ctx)`, `finish_clean`, `start_lint(path, ctx)`, `finish_lint`, `start_ci_fetch`, `finish_ci_fetch`, `start_example`, `kill_example`, `is_clean_running`, `is_lint_running`, `queue_clean`, `drain_next_pending_clean`, `refresh_lint_runtime_from_config(&CargoPortConfig)`. `ctx` here is a small struct `StartContext<'a> { toasts: &mut ToastManager, config: &CargoPortConfig, background: &mut Background, scan: &Scan }` — the actual dependency surface, named once instead of per-method. |
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
- **handles** to the seven subsystems above (`panes`, `selection`,
  `background`, `inflight`, `config`, `keymap`, `scan`)
- the **network state cluster** (`http_client`, `github`, `crates_io`) —
  not carved in phases 1–6; see "Net subsystem (deferred)" below

That's roughly 16 fields instead of 60 after phases 1–6; ~12 if/when
the deferred `Net` carve lands.

### Two axes of structure inside `Panes`

- **App → `Panes` boundary**: strict delegation, no trait. Single owner, single
  caller, concrete struct. `app.panes: Panes`.
- **`Panes` → individual pane behavior**: a `Pane` trait, landing in
  **Phase 6** (not Phase 1). Phase 1 absorbs the field cluster — that's the
  prerequisite. Phase 6 is what actually fixes the per-pane god-object
  problem: every concrete pane (`CiPane`, `CpuPane`, `GitPane`, `LintsPane`,
  `PackagePane`, `LangPane`) gets its own file with one `impl Pane` block,
  and the match-on-`PaneId` arms in `panes/spec.rs`, `panes/actions.rs`,
  `panes/support.rs`, and per-pane render files collapse into trait
  dispatch.

  By Phase 6 all the other subsystems exist as proper types, so trait
  methods can take typed subsystem references (e.g.,
  `Pane::handle_input(&mut self, &mut Selection, &mut Background, &mut
  Inflight, &Config, &mut Scan, &mut ToastManager, event)`). The
  parameter list is dependency injection, not a god-object handle —
  encapsulation by file (each pane's behavior in one place named for the
  pane) is the win.

  Phase 1 (field cluster only) and Phase 6 (per-pane trait split) are
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
2. **Selection** — second-biggest field cluster (`cached_*`, `selection*`).
   `cached_fit_widths` is renamed to `ProjectListWidths` (a one-step rename
   of `ResolvedWidths`); the generic `ColumnWidths` extraction is
   **deferred** until a second pane (lints / ci / git) actually wants to
   consume it. Doing the generic split now is pure churn — the existing
   `ResolvedWidths` is already a single typed struct used only by
   project-list code, and the reusable primitive is explicitly speculative
   until adoption. A rename is enough.
3. **Background + Inflight** together — entangled (a "start" hits both).
4. **Config + Keymap** (one phase) — extract shared `WatchedFile<T>`
   primitive, then carve `Config` and `Keymap` as two separate subsystems
   composing it. Tightly coupled by the primitive; extracting one without
   the other leaves the duplication in place.
5. **Scan** — last because `mutate_tree` already gates it; mostly relocation.
6. **Pane catalog rewrite** — the actual fix to the per-pane god-object
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

   **Phase 6 begins with a re-review of the phase plan against
   everything learned in Phases 1–5.** Subsystem APIs may have shifted,
   the per-pane code's actual shape may have changed under us, and the
   trait signature drafted in this doc is a starting point, not a
   contract. The first task in Phase 6 is to revisit this section and
   propose updates based on what the prior phases produced, before
   writing any pane impl code.

**(Originally there was a separate "ColumnWidths primitive" phase between
Panes and Selection. Outside review pointed out that it was pure churn
without a concrete second consumer, so it was collapsed into Selection as a
rename of `ResolvedWidths` → `ProjectListWidths`. Generic `ColumnWidths` is
deferred until a second pane wants it.)**

**(Phase 6 was added after a directive that the per-pane god-object
problem must be solved, not deferred. Phase 1 absorbs the field cluster;
Phase 6 finishes the job by giving each pane its own implementation
block.)**

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
    Phase 6, not deferred indefinitely — see "Two axes of structure
    inside `Panes`" above.
  - `hovered_pane_row` lives in `Panes`. Hit-testing in Phase 1 is a
    match-on-`PaneId` inside `Panes`; collapses into `Pane::hit_test`
    trait dispatch in Phase 6.
  - `Panes::handle_input` in Phase 1 keeps the `&mut App` shape that
    `panes/actions.rs` currently uses (every dispatch in
    `panes/actions.rs:32-336` reaches across ~6 of the 7 future
    subsystems). Phase 6 replaces this with `Pane::handle_input`
    taking typed subsystem references — at that point `panes/actions.rs`
    as a free-function module ceases to exist, replaced by per-pane
    `impl Pane` blocks.
  - `apply_hovered_pane_row` (`tui/app/mod.rs:278-286`) moves wholesale
    into `Panes` — it reads `hovered_pane_row` and writes `pane_manager`,
    both of which become Panes-internal. Canonical example of "method
    that disappears from App."
  - **Phase-1 → Phase-5 staging.** `Panes::clear_for_tree_change()` lands
    in Phase 1 and is **called from App** (from the existing
    `TreeMutation::drop`) until Phase 5 wires it into the new fan-out
    `TreeMutation::drop`. Without this temporary call, Phase 1 would
    orphan `worktree_summary_cache` invalidation.

- **Phase 2 (Selection)**:
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
  - **Call-site cost across phases.** Phase 2 will write
    `selection.mutate(&self.projects)`. After Phase 5 (Scan) moves
    `projects` to `self.scan.projects()`, those call sites mass-rewrite
    to `selection.mutate(self.scan.projects())`. Acceptable per-phase
    sweep, not a redesign — but worth knowing 3 of 5 phases will touch
    the same selection-mutation call sites.

- **Phase 5 (Scan)**:
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
    `tui/app/async_tasks.rs:~1895-1920`). Today it reads
    `self.current_config.tui.inline_dirs.clone()`, then opens
    `self.mutate_tree()` and calls `tree.insert_into_hierarchy(item);
    tree.regroup_members(&inline_dirs);`, then on drop calls
    `self.register_discovery_shimmer(...)` and
    `self.rebuild_visible_rows_now()`. After Phase 5:
    `register_discovery_shimmer` lives on `Scan`,
    `rebuild_visible_rows_now` lives on `Selection`. Two acceptable
    shapes:
    - **(a) keep follow-ups outside the guard**: drop the guard, then
      call `self.scan.register_discovery_shimmer(...)` and
      `self.selection.recompute_visibility(self.scan.projects())`
      explicitly. Mirrors today's structure, but reintroduces the
      "must remember" foot-gun for the shimmer call.
    - **(b) fold them into `TreeMutation::drop`**: the guard's drop
      registers shimmers for any newly-inserted paths it observed.
      Type-enforced. Requires `TreeMutation` to track inserts, which
      it already implicitly does via the `insert_into_hierarchy` /
      `replace_leaf_by_path` etc. methods.
    
    The plan picks (b) — the whole point of the guard pattern is that
    follow-ups can't be skipped.
  - **Inner shape of `TreeMutation` after the carve.**
    ```text
    struct TreeMutation<'a> {
        scan:      &'a mut Scan,
        panes:     &'a mut Panes,
        selection: &'a mut Selection,
        inserted:  Vec<AbsolutePath>,  // for shimmer registration on drop
    }
    impl Drop for TreeMutation<'_> {
        fn drop(&mut self) {
            self.panes.clear_for_tree_change();
            for p in &self.inserted { self.scan.register_shimmer(p); }
            self.selection.recompute_visibility(self.scan.projects());
        }
    }
    ```
    Borrow-OK because `scan` and `selection` are disjoint `&mut` fields
    the guard owns separately, not borrowed from a shared `&mut App`.

- **Phase 4 (Config + Keymap, sharing a `WatchedFile<T>` primitive)**:
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

- **Phase 6 (Pane catalog rewrite)**:
  - **First task: re-review the Phase 6 plan.** Before writing any
    `impl Pane` code, revisit this section, the `Panes` table row, and
    the "Two axes of structure inside `Panes`" section against the
    actual state of Phases 1–5 as committed. Subsystem APIs may have
    moved, the per-pane code's shape may have evolved, and the trait
    signature drafted in this doc is a starting point. Update the doc
    to reflect what was learned, get the user's approval on the
    revisions, then start implementing.
  - **Trait shape (starting point, subject to Phase 6 re-review).**
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
    several of these files exist but only contain render code; Phase 6
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
    during render — Phase 6 cleans this up so hit-testing is a query,
    not a render side-effect.
  - **`PaneId` enum stays.** It's still the index used by App, by
    `Panes` for focus and lookup, and by callers asking "which pane is
    selected?" Trait dispatch happens through `Panes`'s storage of
    `Vec<Box<dyn Pane>>` (or a typed array indexed by `PaneId`,
    decided at re-review time), keyed by `PaneId`.
  - **What stays in `panes/data.rs`.** The data registry
    (`PaneDataStore`) that caches per-pane data computed by builders
    is orthogonal to per-pane behavior — it stays as is. Phase 6 is
    about behavior, not data.

- **Phase 3 (Background + Inflight)**:
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

## Field assignment appendix (every App field accounted for)

The subsystem table covers the headline carves. This appendix is the
exhaustive list — every field of `App` (`tui/app/mod.rs:88-180`) and where
it lands. Items marked **App-shell** stay on `App`.

| Field | Destination |
|---|---|
| `current_config` | Config |
| `http_client` | **Net subsystem (post-Phase-5 carve)** — see "Net subsystem (deferred)" below |
| `github` (`GitHubState`: `fetch_cache`, `repo_fetch_in_flight`, `running_fetches`, `running_fetch_toast`, `availability`) | **Net subsystem (post-Phase-5 carve)** |
| `crates_io` (`CratesIoState`) | **Net subsystem (post-Phase-5 carve)** |
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

App-shell field count after the **planned 6 phases**: ~16 (focus stack
+ modal/UI shell + `http_client` + `github` + `crates_io` + 7 subsystem
handles). Down from ~60. Phase 6 (pane catalog rewrite) reorganizes per-pane
*behavior* but does not add or remove App fields. The full ~12 number quoted
earlier in the doc assumes the deferred Net carve completes; without it, the
network state remains a residual god-object cluster on App-shell.

### Net subsystem (deferred — sketch only)

`http_client` + `github` (`GitHubState`) + `crates_io` (`CratesIoState`)
together form a sixth subsystem that this plan does **not** carve in
phases 1–6, but should carve afterward. Sketch:

| Field | Why it groups here |
|---|---|
| `http_client: HttpClient` | Shared rate-limit state and connection pool |
| `github: GitHubState` (`fetch_cache`, `repo_fetch_in_flight`, `running_fetches`, `running_fetch_toast`, `availability`) | All keyed by repo, all fed by HTTP |
| `crates_io: CratesIoState` (`availability`) | Same lifecycle as `github` (availability tracker) |

Public API roughly: `net.http_client()`, `net.github_status()`,
`net.crates_io_status()`, `net.fetch_cache()`, `net.start_repo_fetch(...)`,
`net.complete_repo_fetch(...)`, `net.poll_rate_limit()`. Read by
panes (Git pane reads availability + rate limit) and by Scan
(`fetch_cache` feeds tree enrichment).

Why deferred:
- `running_fetches` + `running_fetch_toast` overlap with `Inflight`'s
  domain — needs a decision about whether repo fetches go through
  `Inflight` (uniform "in-flight tracker") or `Net` (HTTP-coupled).
- `fetch_cache` overlaps with `Scan`'s domain (tree-enrichment cache).
- The carve adds a 6th subsystem, raising App-shell complexity
  before reducing it. Better done after the existing 5 phases settle
  the patterns.

This plan does not block on `Net`; phases 1–6 are valuable on their
own. App still owns these three fields after Phase 6.

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
  subsystems plus the deferred `Net`. After phases 1–6, its body
  becomes a sequence of subsystem calls (`self.background.swap_bg_channel(...)`,
  `self.inflight.refresh_lint_runtime_from_config(...)`,
  etc.) but the orchestration shape stays.
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
  `running_clean_paths`/`running_lint_paths` accessors,
  `refresh_lint_runtime_from_config` (since `lint_runtime` lives in
  Inflight per the table revision above).
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
  subsystems). Phase 3 (Background+Inflight) or Phase 4 (Config+Keymap)
  is the realistic point to change it to
  `(&mut self, &mut Inflight, &mut Background, &mut Selection, ctx) ->
  …` once those subsystems exist. Expect to update every dispatch
  function in `panes/actions.rs` (~12 functions, ~336 lines) when this
  flip happens.
- **`Inflight::StartContext` field set.** Phase 3 lands it with the
  fields named above. If a future phase introduces a new
  cross-subsystem dependency for a start path (e.g., a tree-aware lint
  start), the cleanest fix is to add the field to `StartContext` rather
  than thread a new parameter through every `start_*`. The struct
  exists precisely to absorb that growth.

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
