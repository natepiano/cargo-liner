# Collapsing App's pass-through accessors

## Status

After splitting `async_tasks`, `navigation`, and `query` into per-subsystem
directories, mend reports **147 `pub` items on `App` that mend wants narrowed
but cannot be**, because non-`tui/app/` callers (`tui/terminal.rs`,
`tui/interaction.rs`, `tui/render.rs`, `tui/panes/*`) reach them. Narrowing
to `pub(super)` breaks compilation; `pub(in path::*)` is banned.

The 147 are not a visibility problem. They are a **structural** problem:
most of them are pass-through accessors that duplicate a method already on
the subsystem field they delegate to. Each pass-through is a place where
`App` has been used as a public namespace for subsystem internals.

## Principle

**Expose subsystems by reference, do not re-export their methods on `App`.**

```rust
// before — App has a method per subsystem method:
impl App {
    pub fn ci_for(&self, p: &Path) -> Option<Conclusion> { self.ci.for_path(p) }
    pub fn ci_data_for(&self, p: &Path) -> Option<&ProjectCiData> { self.ci.data_for(p) }
    pub fn ci_is_fetching(&self, p: &Path) -> bool { self.ci.is_fetching(p) }
    // ...
}

// after — App exposes the subsystem; callers go through it:
impl App {
    pub fn ci(&self) -> &Ci { &self.ci }
}
// caller:
//   before: app.ci_for(path)
//   after:  app.ci().for_path(path)
```

The pass-through methods on `App` are deleted; the methods on the underlying
subsystem absorb their callers and stay `pub` on the subsystem (or
`pub(super)` if no caller is outside).

This is the same pattern the existing `Lint`, `Net`, `Ci`, `Inflight`
subsystems already follow — the half that was completed. The other half
(removing the App-side wrappers) is what this plan does.

## Out of `App`'s contract

Methods that **should not** become pass-throughs because they coordinate
multiple subsystems stay on `App` as orchestrators (and remain `pub` —
they are App's actual contract surface):

- `apply_config`, `apply_lint_config_change`, `refresh_lint_runtime_from_config`
- `rescan`, `handle_bg_msg`, `poll_background`, `poll_ci_fetches`, `poll_example_msgs`, `poll_clean_msgs`
- `initialize_startup_phase_tracker`, `maybe_log_startup_phase_completions`, every `maybe_complete_startup_*`
- `sync_selected_project`, `enter_action`
- `start_clean` (inflight + toast + projects coordination)
- `apply_service_signal`, `mark_service_recovered`, `spawn_service_retry`, `spawn_rate_limit_prime`
- `ensure_detail_cached` (Panes + Selection + Scan + Ci + Lint)
- `ci_for`, `ci_for_item`, `unpublished_ci_branch_name` (Ci + ProjectList + git state — these are orchestrators despite the name)
- The toast manager methods that *also* update the Toasts pane viewport length
  (`prune_toasts`, `show_timed_toast`, `show_timed_warning_toast`,
  `start_task_toast`, `set_task_tracked_items`, `mark_tracked_item_completed`,
  `dismiss_toast`, `focused_toast_id`, `finish_task_toast`) — they couple
  `ToastManager` and `Panes`. See Phase 2.

`request_quit`, `should_quit`, `request_restart`, `should_restart` are *not*
in this list — they are trivial state writes on `ExitMode` and move in
Phase 6 (Overlays).

These stay on `App` because each one touches at least two subsystems and
encapsulates a cross-cutting decision.

## Plan readiness (architecture review)

A depth-focused architecture review evaluated whether each phase has
enough concrete design to implement, vs. enough framing to discuss.
After incorporating the review's feedback by writing the missing
design depth in-line per phase:

| Phases | Readiness | Notes |
| ------ | --------- | ----- |
| 1 | **Done** (commit `7160e04`) | Config pass-through accessors collapsed. 10 flag methods moved to `Config`, `app.config()` accessor added, ~50 call sites updated. Mend warnings: **147 → 133** (-14, predicted -10 — overshoot due to secondary warnings clearing when self-callers inside App impls also went away; treat per-phase predictions as conservative). `Config` stayed `pub(super)`; no widening needed. The `pub(super)` "relocate but no mend change" items (`current_config*`, `settings_edit_*`) were dropped from scope. 597/597 tests pass. |
| 1b, 2, 3, 4, 4b, 5, 6, 7a, 7b, 8 | **Ready** | Mechanical visibility narrowings; one new accessor + call-site updates per phase. Risk is schedule, not design. |
| 9 (move `Viewport.pos` → `Selection.cursor`) | **Ready** | Design depth filled in below: post-move structs, `&Scan`-arg method signatures, scroll-follows-cursor location, end-to-end flow. |
| 10 (subsystems implement `Pane`) | **Ready** | Design depth filled in below: `Pane` trait signature, post-Phase-10 `PaneRenderCtx` fields, `RenderSplit<'a>` borrow-helper, dispatch loop, detail-cache decision (option b — keeps cache on `Panes`), `*Layout` type module locations. One execution-time choice flagged: option (a) vs (b) for `Selection`'s `Pane` impl, with (b) recommended. |
| 11 (`Bus<Event>`) | **Ready** | Design depth filled in below: full `Event` enum (~14 variants), full `Command` enum, `EventHandler` trait signature with `&mut self + &Scan`, `EventBus` (~10 lines), drain loop with verified borrows, command pattern eliminating cross-subsystem `&mut`, re-entrancy semantics, `StartupOrchestrator` API, end-to-end traced flow for `apply_config`. |

The earlier rounds of review pointed out gaps that were real; this
pass produced concrete trait signatures, struct definitions,
borrow-composition sketches, and traced flows. All three
architectural phases (9, 10, 11) now have enough design to sit down
and implement.

## Phase order

Phases run smallest blast radius first. Each phase is one commit, gated by
`cargo check` + `cargo nextest run --workspace` + `cargo +nightly fmt --all`.
Each phase must independently leave the tree green.

Phases 1–8 are visibility narrowings. Phases 9–11 are architectural
moves derived from the post-Phase-8 review (see end of doc).

**Effective execution order** (resolves Phase 9 vs Phase 7a
contradiction): the canonical numbered list reads
`1, 1b, 2, 3, 4, 4b, 5, 6, 7a, 7b, 8, 9, 10, 11` for narrative
clarity, but Phase 9 must precede Phase 7a because Phase 9
supersedes Phase 7a's `NavRead { panes }` design. Recommended
**actual** execution sequence:

```
1, 1b, 2, 3, 4, 4b, 5, 6, 9, 7a, 7b, 8, 10, 11
                       ^^^^^^^ swap — Phase 9 before 7a
```

Why the doc keeps the canonical list as-numbered: Phase 7a/7b are
"navigator" group and Phase 9 is "borrow-cell mutator fix" —
narratively grouping them differently helps the reader understand
the cluster framing. The execution order is what matters at
implementation time.

Per-phase steps:
1. Add the subsystem accessor on `App`.
2. Rewrite call sites: `app.foo_method(args)` → `app.subsystem().foo_method(args)`.
3. Move method definitions from `App` impl blocks into the subsystem `impl`
   (where they likely already exist as `fn` and are simply being unwrapped).
4. Remove the now-unused `App::foo_method` pass-throughs.
5. `cargo mend` after each phase — confirm the warning count drops.

## Phase 1 — `Config` (smallest) — **DONE** (`7160e04`)

**Results:**
- Mend warnings: 147 → 133 (-14 actual vs -10 predicted; overshoot
  came from secondary warnings clearing when self-callers inside
  App impls also went away).
- 17 files changed, 90 insertions / 72 deletions.
- 597/597 tests pass.
- `Config` and all 10 new flag methods stayed `pub(super)`. No
  widening required — all callers live inside `tui/`.
- The `pub(super)` items (`current_config*`, `settings_edit_*`)
  were correctly NOT relocated; the doc's earlier draft listing
  them was overscope.
- Smoke-tested locally after `cargo install --path .`.



**Subsystem:** `crate::config::ConfigState` (the `App.config: ConfigState` field).

**Pass-throughs to remove from `App` (currently `pub`, contributes to mend count):**
- `editor`, `terminal_command`, `terminal_command_configured`
- `lint_enabled`, `invert_scroll`, `include_non_rust`, `ci_run_count`, `navigation_keys`
- `discovery_shimmer_enabled`, `discovery_shimmer_duration`

**Already private `fn` (does not need to move):** `toast_timeout`

**Out of Phase 1 scope** (despite earlier drafts listing them):
`current_config`, `current_config_mut`, `config_path`,
`settings_edit_buf`, `settings_edit_cursor`,
`settings_edit_parts_mut`, `set_settings_edit_state` are already
`pub(super)`. Relocating them would not change the mend count and
would require touching ~10 callers for zero gain. They stay on App
unless a future cleanup phase rolls them into a focused
relocation. Phase 1 does not move them.

**Accessor to add:**
```rust
impl App {
    pub fn config(&self) -> &ConfigState { &self.config }
}
```

**Call site rewrites:**
```rust
// before: app.editor()                  → app.config().current().tui.editor.as_str()
// before: app.lint_enabled()            → app.config().current().lint.enabled
// before: app.terminal_command()        → app.config().current().tui.terminal_command.as_str()
// before: app.invert_scroll()           → app.config().current().mouse.invert_scroll
// before: app.toast_timeout()           → Duration::from_secs_f64(app.config().current().tui.status_flash_secs)
```

**Tradeoff:** call sites get longer (`.current().tui.editor` chain). Mitigation: where the accessor reads more than one config field per caller, leave it on `App` as a real method — but those are rare. Most are one-field reads.

**Expected mend reduction:** ~10 (the `pub` ones; `pub(super)` ones are
clean-up only).

**No widening required (verified at execution time):** `Config` lives at
`crate::tui::config_state::Config`. Its callers — `tui/render.rs`,
`tui/panes/*`, `tui/settings.rs`, `tui/input.rs`, `tui/keymap_ui.rs`,
plus the new `tui/app/query/config_accessors.rs` — all live inside
`tui/`. `pub(super)` from `config_state.rs` reaches the entire
`tui/` subtree, so the new flag-accessor methods stay `pub(super)`.
The earlier draft's "side-channel widening" framing was wrong: it
conflated "non-`tui/app/`" with "non-`tui/`". For Phases 1b, 2, 3,
4, 4b, 5 the same reasoning likely applies — the new accessor
methods on `KeymapState`, `ToastManager`, `ProjectList`, `Ci`,
`Scan` should default to `pub(super)`, not `pub`. Verify per-phase.

**Risk:** none — 1-line read methods.

## Phase 1b — `Keymap`

**Subsystem:** `crate::keymap::KeymapState` (the `App.keymap` field).

**Pass-throughs to remove from `App`:**
- `sync_keymap_stamp` *(currently `pub`; contributes to mend count)*
- `current_keymap`, `current_keymap_mut`, `keymap_path` *(already `pub(super)`; relocate but no mend count change; same widening note as Phase 1)*

**Accessor to add:**
```rust
impl App {
    pub fn keymap(&self) -> &KeymapState { &self.keymap }
    pub fn keymap_mut(&mut self) -> &mut KeymapState { &mut self.keymap }
}
```

**Tradeoff:** `load_initial_keymap`, `maybe_reload_keymap_from_disk`,
`show_keymap_diagnostics`, `dismiss_keymap_diagnostics`,
`keymap_begin_awaiting`, `keymap_end_awaiting`, `is_awaiting_key`,
`is_keymap_open`, `open_keymap`, `close_keymap` are not pass-throughs —
they touch toasts (diagnostics), overlays (open/close/awaiting), or do
file I/O. They stay on `App` (or move to Overlays in Phase 6) — not
this phase.

**Expected mend reduction:** ~1.

**Risk:** none.

## Phase 2 — `Toasts` (smaller than first draft)

**Subsystem:** `crate::tui::toasts::ToastManager` (the `App.toasts` field).

**Pure pass-throughs to remove from `App`:**
- `active_toasts`
- `toasts_is_alive_for_test` *(`#[cfg(test)]`)*

**Accessor to add:**
```rust
impl App {
    pub fn toasts(&self) -> &ToastManager { &self.toasts }
    pub fn toasts_mut(&mut self) -> &mut ToastManager { &mut self.toasts }
}
```

**Stays on `App` as orchestrator** (each updates `panes().toasts().viewport()` length after touching `ToastManager`):
- `show_timed_toast`, `show_timed_warning_toast`
- `start_task_toast`, `finish_task_toast`
- `set_task_tracked_items`, `mark_tracked_item_completed`
- `dismiss_toast`, `focused_toast_id`, `prune_toasts`

**Tradeoff (deferred):** the viewport-len recompute is real cross-cutting state
between `Toasts` and `Panes`. Two paths if this is revisited later:
- **(a) Keep these on `App`** — accepts ~9 stuck-pub orchestrators.
- **(b) Add an observer/callback** inside `ToastManager` that pushes the new
  active count to a `&mut Viewport` borrow at end of every mutation.
  Eliminates the cross-cutting at the cost of an indirection most callers
  don't need.

This phase chooses (a). Path (b) is a follow-up if the orchestrator count
becomes a problem.

**Expected mend reduction:** ~2.

**Risk:** none — only two methods touched.

## Phase 3 — Git/Repo reads (extract into `ProjectList`)

**Source:** `App.scan.projects()` (a `ProjectList`) — git state lives inside
`ProjectInfo.local_git_state` and `Entry.git_repo.repo_info`.

**Note: these are extracts, not pass-through deletions.** None of these
methods exists on `ProjectList` today. Phase 3 *creates* them on
`ProjectList`, then removes the `App` versions.

**Methods to extract from `App` into `ProjectList`:**
- `git_info_for`, `repo_info_for`
- `primary_url_for`, `primary_ahead_behind_for`, `fetch_url_for`
- `git_status_for`, `git_status_for_item`
- `git_sync`, `git_main`

**Accessor to add:** none initially — these become `ProjectList` methods,
reached via the existing `app.projects()` (which already exists).

**Call site rewrites:**
```rust
// before: app.git_info_for(path)        → app.projects().git_info_for(path)
// before: app.git_sync(path)            → app.projects().git_sync(path)
```

**Tradeoff:** `git_sync` and `git_main` build formatted strings using
constants from `crate::constants` — that's view logic. Two paths:

- **(a) Keep formatted-string methods on `ProjectList`:** simpler, but mixes view
  formatting into a model type. Most direct migration.
- **(b) Return typed sync state (`AheadBehind`, `NoRemote`, `Empty`) from
  `ProjectList` and format at the render site:** cleaner separation, more
  changes (3 callers each), introduces a new enum.

Pick (b) for `git_sync`/`git_main` only — they're the only string-returning
ones; the rest return typed state already. Cost: ~6 call-site changes plus
one small enum. Buy: model layer stays formatting-free.

**Expected mend reduction:** ~9.

**Risk:** medium — touches render path. Land **before** Phase 4 (Ci) so
`Ci::for_path` can call `ProjectList::primary_ahead_behind_for` rather
than re-implementing it.

## Phase 4 — `Ci` (depends on Phase 3)

**Subsystem:** `crate::ci::Ci` (the `App.ci` field).

**Pass-throughs to remove from `App`:**
- `ci_data_for`, `ci_info_for`
- `ci_is_fetching`, `ci_is_exhausted`
- `selected_ci_path`, `selected_ci_runs`

**Stays on `App` as orchestrator** (each crosses Ci + ProjectList + git state):
- `ci_for` (calls `unpublished_ci_branch_name` which needs `git_info_for` + `repo_info_for`)
- `ci_for_item`
- `unpublished_ci_branch_name`

**Accessor to add:**
```rust
impl App {
    pub fn ci(&self) -> &Ci { &self.ci }
}
```

**Tradeoff:** the original draft proposed pushing `ProjectList` as an arg
into `Ci::for_path`. That makes `Ci` depend on `ProjectList` in its
signature, which is a worse boundary than keeping the orchestrator on
`App`. Path chosen: keep cross-subsystem methods on `App`.

**Expected mend reduction:** ~6.

**Risk:** low — pure pass-throughs.

## Phase 4b — Scan / metadata pass-throughs

**Source:** `App.scan: Scan` and `App.metadata_store: WorkspaceMetadataStore`.

**Pass-throughs to remove from `App`:**
- `metadata_store_handle` — clone `Arc<Mutex<WorkspaceMetadataStore>>`
- `target_dir_index_ref` — borrow on metadata
- `resolve_metadata`, `resolve_target_dir` — read on metadata store
- `confirm_verifying`, `clear_confirm_verifying_for` — write on Inflight verifying set
- `complete_ci_fetch_for`, `replace_ci_data_for_path`, `start_ci_fetch_for` — Ci tracker mutators

**Accessor strategy:**
- `metadata_store_handle`, `target_dir_index_ref`, `resolve_metadata`,
  `resolve_target_dir` move to a `metadata()` accessor returning
  `&WorkspaceMetadataStore` (or its handle).
- `confirm_verifying`, `clear_confirm_verifying_for` move to `Inflight`
  (they're already inflight-tracked state).
- `complete_ci_fetch_for`, `replace_ci_data_for_path`, `start_ci_fetch_for`
  move to `Ci` (tracker mutators on the existing `Ci` subsystem).

**Tradeoff:** `start_ci_fetch_for` updates a tracker AND emits a toast (or
similar UI side-effect) — verify before moving. If yes, it stays on `App`
as a Ci+Toasts orchestrator; only the pure `Ci`-only mutators move.

**Expected mend reduction:** ~5–8 depending on side-effect verification.

**Risk:** low.

## Phase 5 — Discovery shimmer + project predicates

**Subsystem:** `crate::tui::app::Scan` (the `App.scan` field), specifically the
discovery shimmer map already on it.

**Pass-throughs to remove from `App`:**
- `register_discovery_shimmer`, `prune_discovery_shimmers`
- `discovery_name_segments_for_path` *(this one builds styled segments — view logic)*
- `is_deleted`, `selected_project_is_deleted`
- `is_rust_at_path`, `is_vendored_path`, `is_workspace_member_path`
- `formatted_disk`, `formatted_disk_for_item`

**Accessor strategy:**
- Project predicates (`is_deleted`, `is_rust_at_path`, etc.) become methods on
  `ProjectList` and are reached via `app.projects().is_deleted(path)`.
- `selected_project_is_deleted` stays on `App` (combines selection + projects).
- Shimmer methods become methods on a `DiscoveryShimmers` accessor on `Scan`.
  `register_discovery_shimmer` reads both `is_scan_complete()` (Scan) *and*
  `discovery_shimmer_enabled()` (Config) before mutating `Scan`. Hoist the
  config check to the caller; `Scan::register_shimmer` becomes
  `if scan_complete { shimmers.insert(path) }` and the caller checks the
  config flag first. ~3 caller updates.
- `formatted_disk` is view formatting — move to a render helper.
- `discovery_name_segments_for_path` is view formatting — move to render.

**Tradeoff:** `discovery_name_segments_for_path` reads from both `Config`
(for shimmer enable + duration) and `Scan` (for the shimmer state). Building
it inside render means render needs both reads. Either:

- **(a) Pass `&App` to the render helper** — keeps `App` available, doesn't
  collapse the `pub fn` count materially.
- **(b) Build a `DiscoveryShimmerView<'a>` once per frame** and pass that to
  render — explicit dependencies, but a new type.

Pick (b) only if Phase 6/7 (frame-prep view types) lands first; otherwise
keep this method on `App` for now and revisit.

**Expected mend reduction:** ~10.

**Risk:** medium-high — touches render path and a multi-borrow read.

## Phase 6 — `Overlays` subsystem extraction

**Source:** `App.ui_modes: UiModes` (struct in `tui/app/types.rs`) and the
`KeymapMode`, `FinderMode`, `SettingsMode`, `ExitMode` enums.

**Methods to relocate:**
- `open_settings`, `close_settings`, `is_settings_open`, `begin_settings_editing`, `end_settings_editing`, `is_settings_editing`
- `is_finder_open`, `open_finder`, `close_finder`
- `open_keymap`, `close_keymap`, `is_keymap_open`, `is_awaiting_key`, `keymap_begin_awaiting`, `keymap_end_awaiting`
- `request_quit`, `should_quit`, `request_restart`, `should_restart`
- `open_overlay`, `close_overlay`

**Refactor:** create `crate::tui::app::overlays::Overlays` owning the
`UiModes` (rename to `OverlayState` once moved). Methods become `Overlays`
methods. `App` exposes:
```rust
impl App {
    pub fn overlays(&self) -> &Overlays { &self.overlays }
    pub fn overlays_mut(&mut self) -> &mut Overlays { &mut self.overlays }
}
```

**Tradeoff 1 — focus coupling:** `open_overlay`/`close_overlay` orchestrate
focus changes too (setting `return_focus`). Keep those two on `App`, move
the rest. `request_quit` and `request_restart` are simple state writes
on `OverlayState.exit` — trivially relocate.

**Tradeoff 2 — `inline_error`:** `close_settings`, `close_keymap`,
`begin_settings_editing`, `end_settings_editing`, `keymap_begin_awaiting`,
`keymap_end_awaiting` each clear `App.inline_error` (an App field, not on
`UiModes`). Two paths:
- **(a) Move `inline_error` into `OverlayState`.** Clean; methods relocate
  cleanly. Cost: every read of `inline_error` outside Overlays now goes
  through `app.overlays().inline_error()`.
- **(b) Keep `inline_error` on `App`.** These 6 methods stay on `App` as
  Overlays+InlineError orchestrators; only the simpler open/state-query
  methods move (~12 instead of ~18).

Pick (a). `inline_error` is a UI-mode-level state already, conceptually
belonging with overlays.

**Expected mend reduction:** ~18 (with path (a)).

**Risk:** low-medium — `inline_error` callers from outside Overlays need
re-routing to `app.overlays().inline_error()`. Audit before commit.

## Phase 7a — Path resolution (clean)

**Source:** `App.scan: Scan` (read) + `App.selection: Selection` (read).

**Methods to relocate** (pure path/display resolution, no viewport mutation):
- `selected_row`, `selected_item`, `selected_project_path`, `selected_display_path`
- `clean_selection`, `path_for_row`, `display_path_for_row`, `abs_path_for_row`
- `expand_key_for_row`, `selected_is_expandable`, `row_count`, `visible_rows`
- `row_matches_project_path`
- The worktree path helpers (`worktree_path_ref`, `worktree_member_abs_path`,
  `worktree_display_path`, etc.) — currently `Self::` static methods on
  `App`. These move to **free fns on `RootItem` / `WorktreeGroup`** in
  `crate::project` — they don't need any `App` field.

**Refactor (revised after Phase 9 lands):** introduce
`NavRead<'a> { selection: &'a Selection, scan: &'a Scan }` returned
by `app.nav_read()`. **The `panes` field is dropped** because Phase 9
moves the project-list cursor onto `Selection` — `selected_row` /
`selected_is_expandable` read it from `Selection` directly. If
Phase 9 has not landed when Phase 7a runs, fall back to
`NavRead { selection, scan, panes }` and rewrite at Phase 9; or
reorder to land Phase 9 first.

**Expected mend reduction:** ~12.

**Risk:** medium — touches many call sites but all reads.

## Phase 7b — Movement / selection mutators (stays on `App`)

**Methods that **stay** on `App`:**
- `move_up`, `move_down`, `move_to_top`, `move_to_bottom`, `collapse_anchor_row`
- `expand`, `collapse`, `expand_all`, `collapse_all`, `try_collapse`, `collapse_to`, `collapse_row`
- `select_project_in_tree`, `select_matching_visible_row`, `expand_path_in_tree`
- `ensure_visible_rows_cached`, `ensure_fit_widths_cached`, `ensure_disk_cache`, `ensure_detail_cached`
- `sync_selected_project`

**Why they stay:** every one of these mutates *both* `Selection.expanded` (or
`Selection.paths`) **and** `Panes::project_list().viewport().pos()` — the
viewport position lives on `Panes`, not on `Selection`. A `Navigator` type
that holds `&mut Selection` + `&mut Panes` simultaneously re-introduces the
three-borrow cell that `mutate_tree` already needs and adds a lifetime
parameter to most call sites for no real benefit. Keep these on `App`.

`ensure_detail_cached` additionally reads `Ci`, `Lint`, `Scan.generation()`
to decide whether to rebuild — it's an orchestrator, not a navigator
method.

**Expected mend reduction:** 0 (these stay `pub`).

**Risk:** none — no change.

## Phase 8 — `Focus` subsystem

**Source:** scattered `App` fields (`base_focus`, `return_focus`, etc.) plus
overlay-aware focus computations.

**Methods to relocate (currently `pub`):**
- `base_focus`, `focus_pane`, `focus_next_pane`, `focus_previous_pane`
- `is_focused`, `pane_focus_state`, `focused_dismiss_target`
- `selection_changed`, `clear_selection_changed`
- `mark_terminal_dirty`, `clear_terminal_dirty`, `terminal_is_dirty`
- `input_context`

**Methods staying on `App` after re-evaluation:**
- `tabbable_panes` — does **not** read `ui_modes`; depends on `Inflight`,
  `Panes` (across multiple panes), `ProjectList`, `Selection`, `ToastManager`.
  This is a 5+ subsystem orchestrator, not a focus-state method. Already
  `pub(super)` (no mend count contribution), stays as-is.
- `mark_selection_changed` — already `pub(super)`, no mend count
  contribution. Stays where it is.

**Refactor:** the focus state already partially lives on `App`. Extract a
`Focus` subsystem owning `base_focus`, `return_focus`, `selection_dirty`,
`terminal_dirty`, plus the methods listed. Tabbable pane computation needs
`Overlays` (some panes hide while overlays are open) — the method takes
`overlays: &Overlays` as an arg.

**Tradeoff:** this is the deepest entanglement of the bunch — focus depends
on overlays which depend on selection which depends on projects. Doing this
last lets the prior phases land first so dependencies are exposed as proper
subsystem reads, not as field reaches. Doing it first would force premature
abstractions.

**Expected mend reduction:** ~16.

**Risk:** highest — touches every keyboard handler.

## Phase 9 — Move `Viewport.pos` to `Selection.cursor` (Cluster C)

Source of truth: see "Item 4" in the post-Phase-8 review section
below. Summary:

- Relocate one field: `Viewport.pos: usize` from `Panes::project_list().viewport()`
  into `Selection.cursor: usize`.
- The 12 Cluster-C methods split into two groups by signature:
  - **`&mut Selection`-only (~10 methods):** `move_up`, `move_down`,
    `move_to_top`, `move_to_bottom`, `expand`, `collapse`,
    `collapse_all`, `try_collapse`, `select_matching_visible_row`,
    `row_count`-equivalents. Become plain methods on `Selection`.
  - **`&mut Selection` + `&Scan` (~2 methods):** `expand_path_in_tree`
    and `select_project_in_tree` (which calls `expand_path_in_tree`).
    These iterate `scan.projects()` while mutating
    `selection.expanded_mut()`. Become methods on `Selection` with
    `scan: &Scan` as an arg: `Selection::expand_path_in_tree(&mut self, scan: &Scan, target_path)`.
  - **`expand_all`** also iterates `scan.projects()` — same
    `&mut self, scan: &Scan` signature.
- ~30 call-site updates in `tui/app/navigation/*` and the render path
  (read cursor from `Selection`, scroll from `Panes`). The render
  scroll-follows-cursor logic continues to live in render code; it
  reads cursor from `Selection` and updates scroll on `Panes` —
  same logic as today, just split across two reads.

**Ordering constraint:** Phase 9 supersedes Phase 7a's `NavRead { panes }`
design. Land Phase 9 before Phase 7a, or skip Phase 7a and let Phase 9
absorb its work.

**Expected mend reduction:** ~12.

**Risk:** medium — touches navigation peer code and render scroll
logic.

### Phase 9 design depth

**Today's `Viewport` struct** (`src/tui/pane/state.rs:55-63`):
```rust
pub struct Viewport {
    cursor:        ScrollState,      // <-- moves to Selection
    hovered:       Option<usize>,    // stays — render-only hover state
    len:           usize,            // stays — derived per-frame from data length
    content_area:  Rect,             // stays — recorded each frame by render
    scroll_offset: usize,            // stays — render-only scroll state
    visible_rows:  usize,            // stays — visible row count for overflow indicator
}
```

`ScrollState` is a one-field wrapper (`pos: usize`) with bounds-checked
mutators (`up`, `down`, `set`, `clamp`, `jump_home`, `jump_end`).

**Post-Phase-9 `Viewport`:**
```rust
pub struct Viewport {
    hovered:       Option<usize>,
    len:           usize,
    content_area:  Rect,
    scroll_offset: usize,
    visible_rows:  usize,
}
```

`ScrollState` deletes (or moves to Selection if reused).

**Post-Phase-9 `Selection`** (current at `src/tui/selection.rs:36-45`):
```rust
pub(super) struct Selection {
    paths:               SelectionPaths,
    sync:                SelectionSync,
    expanded:            HashSet<ExpandKey>,
    finder:              FinderState,
    cached_visible_rows: Vec<VisibleRow>,
    cached_root_sorted:  Vec<u64>,
    cached_child_sorted: HashMap<usize, Vec<u64>>,
    cached_fit_widths:   ProjectListWidths,
    cursor:              usize,        // <-- new: project-list row cursor
}
```

Just one new field. The cursor lives next to `cached_visible_rows` —
the same `Vec<VisibleRow>` it indexes into.

**New methods on `Selection`** (Phase 9 surface):
```rust
impl Selection {
    pub fn cursor(&self) -> usize { self.cursor }
    pub fn move_up(&mut self) {
        if self.cursor > 0 { self.cursor -= 1; }
    }
    pub fn move_down(&mut self) {
        let len = self.cached_visible_rows.len();
        if len > 0 && self.cursor < len - 1 { self.cursor += 1; }
    }
    pub fn move_to_top(&mut self) { self.cursor = 0; }
    pub fn move_to_bottom(&mut self) {
        self.cursor = self.cached_visible_rows.len().saturating_sub(1);
    }
    pub fn try_collapse(&mut self, key: &ExpandKey) -> bool { self.expanded.remove(key) }
    pub fn collapse(&mut self) -> bool { /* uses self.cursor + self.cached_visible_rows */ }
    pub fn collapse_all(&mut self) { /* clears self.expanded, recomputes */ }
    pub fn select_matching_visible_row(&mut self, target_path: &Path) {
        if let Some(i) = self.cached_visible_rows.iter().position(|r| /* ... */) {
            self.cursor = i;
        }
    }
}
```

`row_count` is `self.cached_visible_rows.len()` — `Selection` already
owns this data; no `&Scan` needed.

**Methods that need `&Scan`** (the 3 exceptions):
```rust
impl Selection {
    pub fn expand(&mut self, scan: &Scan) -> bool { /* reads scan.projects() */ }
    pub fn expand_all(&mut self, scan: &Scan) {
        for (ni, entry) in scan.projects().iter().enumerate() {
            if entry.item.has_children() {
                self.expanded.insert(ExpandKey::Node(ni));
            }
            // ... per-RootItem traversal
        }
    }
    pub fn expand_path_in_tree(&mut self, scan: &Scan, target: &Path) {
        // reads scan.projects(), writes self.expanded
    }
    pub fn select_project_in_tree(&mut self, scan: &Scan, target: &Path) {
        self.expand_path_in_tree(scan, target);
        self.select_matching_visible_row(target);
    }
}
```

**Visibility-cache recompute design.** Today's
`recompute_visibility(&mut self, projects: &ProjectList, include_non_rust: bool)`
takes `&ProjectList` (not `&Scan`) and `include_non_rust: bool`.
Phase 9's new methods that mutate `self.expanded` need both
arguments. Two choices:

- **(a) Methods take `&Scan, include_non_rust`** — pass through:
  `pub fn expand_all(&mut self, scan: &Scan, include_non_rust: bool)`.
  Caller threads `include_non_rust` from `app.config().current().tui.include_non_rust.includes_non_rust()`.
- **(b) Selection holds `include_non_rust` as a cached field**, set
  at config-change time. Methods take `&Scan` only. Adds one field
  to Selection.

**Pick (b).** `Selection` already caches config-derived state
(`cached_fit_widths` is computed from `lint_enabled`); a
`include_non_rust: bool` field updated at the same time keeps the
visibility recompute self-contained. The Phase 11 `ConfigDiff`
helper already inspects `include_non_rust`-equivalent fields, so
keeping Selection's flag in sync is a one-line `Command::SetIncludeNonRust(bool)`.

After (b):
- The `&mut self`-only methods (`move_*`, `select_matching_visible_row`)
  only mutate `cursor`. No recompute needed — cursor doesn't change
  visibility.
- `try_collapse`, `collapse_to`, `collapse_row`, `collapse`,
  `collapse_all` mutate `expanded`. They call
  `self.recompute_visibility(projects)` (taking `&ProjectList`,
  with `include_non_rust` read from `self.include_non_rust`). The
  caller passes `&ProjectList`. Since `Selection` doesn't own
  `ProjectList`, these methods take `projects: &ProjectList` as
  arg, not `&Scan` (caller does `app.scan.projects()` to get it).
- `expand`, `expand_all`, `expand_path_in_tree`,
  `select_project_in_tree` mutate `expanded` and need `scan` for
  iteration. They take `scan: &Scan` and call
  `self.recompute_visibility(scan.projects())` at the tail.

**Cursor clamp on shrink.** `recompute_visibility` may shrink
`cached_visible_rows`. After recompute, `Selection` clamps
`self.cursor` to the new length:
```rust
fn recompute_visibility(&mut self, projects: &ProjectList) {
    // ... existing body that rebuilds cached_visible_rows
    let len = self.cached_visible_rows.len();
    if len == 0 {
        self.cursor = 0;
    } else if self.cursor >= len {
        self.cursor = len - 1;
    }
}
```
This replaces today's `Viewport::set_len`-driven clamp.

The existing `SelectionMutation` guard pattern is **not extended**
to the new methods — it exists for single-key toggle paths
(`toggle_expand`, `apply_finder`). The Phase 9 bulk-mutation methods
call `recompute_visibility` explicitly at the tail.

**Scroll-follows-cursor mechanism** (corrected): today's
`render_project_list` (`src/tui/panes/project_list.rs:111-210`) does
not contain explicit scroll-follow code — ratatui's `ListState`
computes the scroll internally based on its `selected` index and
the prior `offset`. Today the code reads `viewport.scroll_offset()`,
hands it + the cursor `pos` to `ListState`, lets ratatui decide the
new offset, and writes the result back via `*list_state.offset_mut()`.

After Phase 9: same flow, just two reads. Render reads cursor from
`selection.cursor()` and offset from `panes.project_list().viewport().scroll_offset()`,
hands both to `ListState`, writes the new offset back to
`panes.project_list_mut().viewport_mut().set_scroll_offset(...)`.
ratatui still does the math; the change is purely about where the
two inputs come from.

**End-to-end flow (user presses Down arrow):**
1. Event handler in `tui/interaction.rs` (or wherever key dispatch
   lives) calls `app.selection_mut().move_down()`.
2. `Selection::move_down` increments `self.cursor` if not at end.
3. App's main loop ticks; render runs.
4. Render reads `app.selection().cursor()` and
   `app.panes().project_list().viewport().scroll_offset()`. Computes
   whether cursor is visible; if not, updates scroll offset.
5. Render draws rows from `app.selection().visible_rows()` using
   cursor and scroll offset for highlight + scroll.

No subsystem holds a borrow on the other during the flow. App
provides `&mut Selection` for step 2 and `&Selection` + `&mut Panes`
for step 4–5; both are sequential.

**Call-site updates (~30):**
- `src/tui/app/navigation/movement.rs`: 5 methods become thin
  delegations to `Selection` (or move outright). Body changes from
  `self.panes_mut().project_list_mut().viewport_mut().up()` to
  `self.selection.move_up()`.
- `src/tui/app/navigation/expand.rs`: similar — 8 methods.
- `src/tui/app/navigation/bulk.rs`: 4 methods, two of which take
  `&Scan` arg.
- `src/tui/app/navigation/selection.rs`: `selected_row` reads
  `self.selection.cursor()` instead of `self.panes().project_list().viewport().pos()`.
- `src/tui/render.rs`: scroll-follows-cursor logic relocates from
  inline `Viewport` self-mutation to an explicit scroll-update step
  reading `selection.cursor()`.

**Status: ready to execute.**

## Phase 10 — Subsystems own pane state; drop wrapper types (Cluster B for panes)

Source of truth: see "Item 5" in the post-Phase-8 review section
below. Summary:

- Widen `Pane` trait visibility from `pub(super)` to `pub(crate)` in
  `tui/panes/dispatch.rs`.
- Move `viewport: Viewport` (and absorbed payload, e.g. `CiData`,
  `LintsData`) into the data subsystems: `ToastManager`, `Ci`, `Lint`,
  `Selection`, `Overlays` (per Phase 6).
- Each of those subsystems implements `Pane` directly.
- Slim render-side wrappers retain only per-frame layout (`hits`,
  `row_rects`, `dismiss_actions`, `worktree_summary_cache`,
  `line_targets`). Rename to `*Layout` to make the layout/domain
  separation visible.
- Detail-pane data cache (`DetailCacheKey`-keyed) gets its own home
  — pick option (a) `DetailCache` type owned by App, or (b) keep in
  the existing wrappers as the documented reason they remain.
  Decision deferred to execution time.
- `PaneRenderCtx` grows to carry the broader subsystem references
  some implementations need (e.g. `Selection`'s render reads broader
  app state today).
- `Panes` shrinks to a render-dispatch registry of `&dyn Pane` + the
  cross-pane state (focus, hover dispatch, layout cache).

**Ordering:** Phase 10 lands after **both** Phase 6 and Phase 9.
Phase 6 introduces the `Overlays` subsystem that absorbs `KeymapPane`,
`SettingsPane`, `FinderPane`'s domain state — Phase 10 needs
`Overlays` to exist before it can have `Overlays` implement `Pane`.
Phase 9 moves the project-list cursor onto `Selection`, which
Phase 10 needs in place before Selection's `Pane` impl owns the
project-list viewport.

**Expected mend reduction:** ~9 (the Toast+Panes orchestrators that
become `ToastManager` methods naturally; symmetric wins for `Ci`,
`Lint`, etc. when they grow methods that touch their viewport).

**Risk:** highest of the architectural phases — touches the render
path, the `Pane` trait, every subsystem with a screen presence.

### Phase 10 design depth

**Today's `Pane` trait** (`src/tui/panes/dispatch.rs:31`):
```rust
pub(super) trait Pane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>);
}
```

**Today's `PaneRenderCtx`** (`src/tui/panes/dispatch.rs:20-27`):
```rust
pub struct PaneRenderCtx<'a> {
    pub focus_state:           PaneFocusState,
    pub is_focused:            bool,
    pub animation_elapsed:     std::time::Duration,
    pub config:                &'a Config,
    pub scan:                  &'a Scan,
    pub selected_project_path: Option<&'a std::path::Path>,
}
```

**Today's split-borrow** (`src/tui/app/mod.rs:418-428`):
```rust
pub(super) fn split_panes_for_render(&mut self)
    -> (&mut Panes, &mut LayoutCache, &Config, &Selection, &Scan);
```

This is the lever Phase 10 uses. The codebase already splits App
into disjoint borrows for render. Phase 10 widens the split.

**Post-Phase-10 `Pane` trait** (visibility widened to `pub(crate)`):
```rust
pub(crate) trait Pane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>);
}
```

Same signature; only the visibility changes.

**Post-Phase-10 `PaneRenderCtx`** (carries the references that
`Pane` impls reach today via `app.<x>()` accessors):
```rust
pub struct PaneRenderCtx<'a> {
    pub focus_state:           PaneFocusState,
    pub is_focused:            bool,
    pub animation_elapsed:     std::time::Duration,
    pub config:                &'a Config,
    pub scan:                  &'a Scan,
    pub selection:             &'a Selection,    // NEW (replaces app.selection() reach)
    pub selected_project_path: Option<&'a std::path::Path>,
}
```

Three new fields. `selection` replaces today's
`selected_row` / `cursor` reaches into App; the others stay for
back-compat with detail-pane render bodies.

**Post-Phase-10 split-borrow** widens to all subsystems that
implement `Pane`:
```rust
pub(super) fn split_for_render(&mut self) -> RenderSplit<'_> {
    RenderSplit {
        toasts:    &mut self.toasts,
        ci:        &mut self.ci,
        lint:      &mut self.lint,
        overlays:  &mut self.overlays,    // from Phase 6
        selection: &mut self.selection,   // owns project-list pane post-Phase-9
        panes:     &mut self.panes,       // shrinks to layout cache + detail-pane wrappers
        layout_cache: &mut self.layout_cache,
        config:    &self.config,
        scan:      &self.scan,
    }
}

pub(super) struct RenderSplit<'a> {
    pub toasts:    &'a mut ToastManager,
    pub ci:        &'a mut Ci,
    pub lint:      &'a mut Lint,
    pub overlays:  &'a mut Overlays,
    pub selection: &'a mut Selection,
    pub inflight:  &'a mut Inflight,        // OutputPane reads example output
    pub panes:     &'a mut Panes,
    pub layout_cache: &'a mut LayoutCache,
    pub config:    &'a Config,
    pub scan:      &'a Scan,
}
```

All disjoint App fields → split-borrow is sound.

**Render dispatch loop** in `src/tui/render.rs`:
```rust
fn render_frame(frame: &mut Frame, app: &mut App) {
    let split = app.split_for_render();
    // Build ctx once; reborrow individual fields for each pane render.
    let ctx_template = |selection: &Selection| PaneRenderCtx {
        focus_state: /* ... */,
        is_focused:  /* ... */,
        animation_elapsed: /* ... */,
        config:      split.config,
        scan:        split.scan,
        selection,
        selected_project_path: /* ... */,
    };

    // Each call uses `split.<x>` mutably, with `split.selection` borrowed
    // immutably alongside (sound — disjoint vs. shared).
    split.toasts.render(frame, area, &ctx_template(split.selection));
    split.ci.render    (frame, area, &ctx_template(split.selection));
    split.lint.render  (frame, area, &ctx_template(split.selection));

    // Selection renders itself for project list. Inside its render it
    // reads &mut self; ctx omits selection (would alias with &mut self).
    let ctx_for_self = PaneRenderCtxNoSelection { /* same minus `selection` */ };
    split.selection.render(frame, area, &ctx_for_self);
}
```

There's the one wrinkle: when `Selection` renders itself, `ctx.selection`
would alias with `&mut self`. Two paths:

- **(a) Two `PaneRenderCtx` variants**: one with `selection`, one
  without. `Selection::render` takes the without-variant.
- **(b) `Selection` does not implement `Pane`** — keep the
  project-list render path on a thin `ProjectListPane` wrapper that
  borrows `&mut Selection` from the split. Same destination, less
  trait-alignment pressure.

Pick **(b)**. The pattern "subsystem implements Pane" works for the
clean cases (ToastManager, Ci, Lint, Overlays); for `Selection` the
project-list render keeps a thin wrapper that borrows. Worth ~0
mend-warning difference; saves one design contortion.

**Thesis exception (full enumeration):** Phase 10's headline says
"drop the wrapper types," but the actual outcome is mixed.

| Original wrapper | Fate | Reason |
|---|---|---|
| `ToastsPane` | **collapse** — viewport into ToastManager | Pure pass-through wrapper |
| `CiPane` | **collapse** — viewport into Ci, content moves to Ci | Pure pass-through wrapper |
| `LintsPane` | **collapse** — viewport into Lint, content moves to Lint | Pure pass-through wrapper |
| `KeymapPane` | **collapse** — into Overlays (Phase 6) | Phase 6 dependency |
| `SettingsPane` | **collapse** — into Overlays | Phase 6 dependency |
| `FinderPane` | **collapse** — into Overlays | Phase 6 dependency |
| `ProjectListPane` | **survives (slim)** — wraps `&mut Selection` | Borrow-cell pattern (option b) |
| `PackagePane` | **survives** — holds `DetailPaneData` cache | Detail-pane cache home |
| `LangPane` | **survives** — same as Package (cached detail data) | Detail-pane cache home |
| `GitPane` | **survives** — holds `worktree_summary_cache` + cached detail | Detail-pane cache + per-frame layout |
| `TargetsPane` | **survives** — cached targets data | Detail-pane cache home |
| `CpuPane` | **survives** — owns `CpuPoller` (background thread state) | Real subsystem state, not a wrapper |
| `OutputPane` | **survives (or absorbed into Inflight)** — example output buffer | Open question: Inflight already owns example state; OutputPane could collapse if Inflight gets a viewport. Decision deferred to execution. |

Net: **6 wrappers collapse, 7 survive (or 8 if OutputPane stays).**
Mend reduction stays ~9 because the collapsed 6 are the
toast/Ci/Lint/Overlays group whose orchestrator methods on App
go away.

**Detail-pane cache decision:** `ensure_detail_cached` builds and
caches `Package` / `Git` / `Targets` / `Lints` / `Ci` data per
`DetailCacheKey { row, generation }`. Today the cache is stored on
`Panes::set_detail_data`. **Decision: keep this cache on `Panes`
(option b).** Reasoning: the cache's owner is the render machinery
(detail panes are built per-frame from cross-subsystem reads, not
per-subsystem); moving it to any one data subsystem would couple
that subsystem to the rendering layer. The wrappers that today hold
the cached data (`PackagePane`, `GitPane`, `TargetsPane`, etc.) stay
as **`*DetailPane`** wrappers, with the documented reason: per-row
cross-subsystem render-input cache.

**`*Layout` types** for the per-frame hit-test state move to
`src/tui/panes/layout.rs` (new file, or extend the existing
`tui/panes/layout.rs` if it's render layout). Names:
- `ToastsLayout` (was `ToastsPane.hits`)
- `CpuLayout` (was `CpuPane.row_rects`)
- `GitLayout` (was `GitPane.worktree_summary_cache` + `GitRowLayout`)
- `SettingsLayout` (was `SettingsPane.line_targets`)
- `ProjectListLayout` (was `ProjectListPane.dismiss_actions`)

These live alongside `Panes` (or on a smaller layout-only registry)
because they're written each render frame and read by the next
hit-test cycle — they belong with the render dispatch, not with
domain data.

**Wrapper-deletion call-site changes:**
- `tui/render.rs` switches from `app.panes_mut()` reach into pane
  state to `split.<subsystem>` reach.
- `tui/panes/system.rs` `dispatch_*_render` helpers either inline
  into the new render loop, or take the `RenderSplit` directly
  instead of `&mut Panes`.
- ~50 call sites that read `panes.toasts()` / `panes.ci()` /
  `panes.lints()` etc. become `app.toasts()` / `app.ci()` / etc.
- ~14 wrapper struct definitions (in `pane_impls.rs`) collapse to
  ~7 (the 4 detail panes + CpuPane + ProjectListPane + the
  layout-only types after rename).
- **`build_ci_data`** (`src/tui/panes/support.rs:1773`) and
  **`build_lints_data`** today take `&App` and read ~12 accessors
  spanning Selection, Net, Ci, Scan, and git state. After Phase 10,
  these signatures change to `build_ci_data(split: &RenderSplit<'_>)`
  (or similar disjoint-borrow tuple). All ~3 callers update. This
  is the largest signature change in the call-site set.

**Status: ready to execute** — all load-bearing artifacts are now
defined. One execution-time choice remains: option (a) vs (b) for
`Selection`'s `Pane` impl, with (b) recommended.

---

**Earlier review notes (superseded by the design depth above):**
- The `Pane` trait signature (today `pub(super) trait Pane { fn
  render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx:
  &PaneRenderCtx<'_>); }` — the post-Phase-10 version may need
  different args).
- `PaneRenderCtx`'s post-Phase-10 field list (which subsystem
  references it carries beyond the current `Config`, `Scan`,
  `selected_project_path`).
- Render-loop pseudo-Rust showing how `Panes` dispatches through
  trait-objects whose owning data lives in App fields. Specifically:
  who holds the `&dyn Pane` references, when, and how they compose
  with `&mut App` at the call site.
- Post-Phase-10 `Selection` struct definition (with both Phase 9's
  `cursor` and Phase 10's project-list viewport).
- Detail-pane cache home: pick option (a) `DetailCache` type owned
  by App, or (b) keep wrappers as the documented reason.
- Module locations for the renamed `*Layout` types.
- Enumeration of wrapper-deletion call-site changes.

## Phase 11 — `Bus<Event>` for cross-cutting events (Cluster A)

Source of truth: see "Item 6" in the post-Phase-8 review section
below. Summary:

- Introduce `enum Event` with ~10–15 variants (`ConfigChanged`,
  `ServiceSignal`, `ServiceRecovered`, `StartupPhaseAdvanced`, etc.).
- Introduce `trait EventHandler { fn handle(&mut self, ev: &Event,
  ctx: &mut HandlerCtx); }`.
- Subscribers (subsystems) register at App construction.
- Two-phase delivery: events queue, then drain to subscribers.
- App's role: receive raw triggers, wrap into `Event`, publish.
- Add `StartupOrchestrator` at
  `src/tui/app/async_tasks/startup_phase/orchestrator.rs` to publish
  `StartupPhaseAdvanced(...)` events in the dependency order today
  encoded in `maybe_log_startup_phase_completions`.

**Ordering:** Phase 11 lands last among the architectural phases.
Subsystems need their pane state already moved (Phase 10) so their
`handle()` bodies can mutate self plus update their own pane.

**Why Phase 10 must be done correctly first, not just "land first."**
`apply_service_signal`'s reaction body today calls
`push_service_unavailable_toast`, which writes both
`self.toasts.push_persistent(...)` and
`self.panes_mut().toasts_mut().viewport_mut().set_len(...)`. That
second write is what Phase 10 eliminates by moving the toasts
viewport into `ToastManager`. If Phase 10 lands without that
relocation, `HandlerCtx` for `ServiceSignal` subscribers must carry
both `&mut Toasts` AND `&mut Panes`, re-enacting through the ctx
struct the multi-borrow chain Phase 11 was supposed to remove. The
bus only works if Phase 10 has actually moved each pane's state
into its data subsystem.

**Expected mend reduction:** ~25 — the largest single drop.

**Risk:** highest of all phases — touches startup-phase ordering,
service-signal handling, and the existing `apply_config` / `rescan`
fan-out logic.

### Phase 11 design depth

The design uses a **command pattern layered on top of pub/sub**.
Subscribers don't get cross-subsystem `&mut` references during
event handling; they return a list of `Command`s describing the
side-effects they want, and App applies the commands sequentially
after gathering them. This sidesteps all borrow-checker conflicts
that a traditional bus would hit.

**`StartupPhase` enum** (must be defined; not present in source today):
```rust
pub enum StartupPhase {
    Disk,      // disk-usage scan complete for all expected roots
    Git,       // local git state populated for all expected dirs
    Repo,      // GitHub repo info fetched for all queued repos
    Metadata,  // cargo metadata complete for all expected workspaces
    Lints,     // lint-cache check complete
    Ready,     // overall startup complete
}
```
Variants mirror today's `maybe_complete_startup_*` family
(`disk`/`git`/`repo`/`metadata`/`lints`/`ready`). Lives at
`src/tui/app/async_tasks/startup_phase/orchestrator.rs`.

**`ConfigChanged` payload — Arc decision.** Today, neither `Config`
nor `CargoPortConfig` is wrapped in `Arc`. Three options for the
event payload:
- **(a) `prev: CargoPortConfig, next: CargoPortConfig`** — clones the
  whole config per event (~hundreds of bytes). Simplest. Acceptable
  given ConfigChanged is rare (user-driven save).
- **(b) Add `Arc<CargoPortConfig>` to `ConfigState`** as a precursor
  step in Phase 11. Bus events carry `Arc<CargoPortConfig>` cheaply.
  Adds one sub-step to Phase 11.
- **(c) Reference variant `ConfigChanged<'a> { prev: &'a Config, next: &'a Config }`**
  forces a lifetime onto `Event`, which prevents the bus owning
  `VecDeque<Event>`. Rejected.

Pick **(a)** — clone per ConfigChanged event. The frequency is low
(~once per save action), and adding `Arc` everywhere `Config` is
read (option b) is a separate refactor with broader fallout. Revisit
if ConfigChanged ever becomes frequent.

**Full `Event` enum** (~14 variants):
```rust
pub enum Event {
    // Config flow — payload cloned per (a) above
    ConfigChanged { prev: CargoPortConfig, next: CargoPortConfig },
    LintConfigChanged,

    // Service signals (one variant per service state)
    ServiceReachable(ServiceKind),
    ServiceUnreachable(ServiceKind),
    ServiceRateLimited(ServiceKind),
    ServiceRecovered(ServiceKind),

    // Startup phases
    StartupPhaseAdvanced(StartupPhase),
    StartupReady,

    // Tree mutation
    RescanRequested,
    ScanRestarted,
    TreeRebuilt,
    ProjectDiscovered(AbsolutePath),
    ProjectRefreshed(AbsolutePath),

    // Selection
    SelectionChanged(Option<AbsolutePath>),
}
```

**`Command` enum** — derived mechanically from today's orchestrator
bodies (`apply_config`, `apply_service_signal`,
`maybe_complete_startup_*`):
```rust
pub enum Command {
    // Toast side-effects
    PushToast { title: String, body: String, style: ToastStyle },
    DismissToast(u64),
    SyncRunningLintToast,
    SyncRunningCleanToast,
    MarkStartupPhaseCompleted(StartupPhase),

    // Service / network
    ScheduleServiceRetry(ServiceKind, Duration),
    SetServiceAvailability(ServiceKind, ServiceAvailability),
    MarkServiceRecovered(ServiceKind),

    // Inflight tracking
    MarkInflightClean(AbsolutePath),

    // Lint runtime
    RestartLintRuntime,        // spawn new runtime + swap in
    ClearAllLintState,         // clears in-memory lint state on projects
    RefreshLintRunsFromDisk,   // re-read lint history per project
    SyncLintRuntimeProjects,   // sync registered set with live tree

    // Selection / panes
    ResetFitWidths { lint_enabled: bool },
    ResetCpuPlaceholder,
    RegroupWorkspaceMembers,   // tree-level regroup based on inline_dirs

    // Scan / discovery
    ClearShimmers,
    BumpScanGeneration,
    RefreshDerivedState,       // marks caches dirty without full rebuild
    RespawnWatcherAndRegisterExisting,

    // Tree
    ForceSettingsIfUnconfigured,

    // Re-entrant publish
    PublishEvent(Event),
}
```

This enumerates ~21 commands derived from `apply_config`'s body
(`src/tui/app/async_tasks/config.rs`), the service-signal handlers,
and the startup-phase tracker. Final count may shift by a few during
execution as exact granularity is decided (e.g. `RestartLintRuntime`
vs splitting into `SpawnLintRuntime` + `SwapLintRuntime`), but the
side-effect surface is now fully accounted for. The `apply_config`
body has nothing left that isn't covered by some command.

**`EventHandler` trait** (concrete signature):
```rust
pub(crate) trait EventHandler {
    /// Examine `event`. Read whatever's needed via `&self` plus the
    /// shared `&Scan` reference for cross-cutting reads. Return the
    /// commands you want App to apply on your behalf.
    ///
    /// **Two-channel rule:**
    /// - In-place `&mut self` mutation is permitted ONLY for the
    ///   subsystem's own private state (e.g. `Lint::runtime` swap,
    ///   `Inflight::clean_mut().insert`). Anything cross-subsystem
    ///   goes through `Command`.
    /// - Cross-subsystem effects MUST return as `Command` variants.
    fn handle(&mut self, event: &Event, scan: &Scan) -> Vec<Command>;
}
```

**`ConfigDiff` helper** (prevents per-subscriber field-by-field
re-derivation of `prev` vs `next`):
```rust
pub struct ConfigDiff<'a> {
    pub prev: &'a CargoPortConfig,
    pub next: &'a CargoPortConfig,
}

impl<'a> ConfigDiff<'a> {
    pub fn lint_enabled_flipped(&self) -> bool {
        self.prev.lint.enabled != self.next.lint.enabled
    }
    pub fn force_github_rate_limit_flipped(&self) -> Option<bool> {
        if self.prev.debug.force_github_rate_limit != self.next.debug.force_github_rate_limit {
            Some(self.next.debug.force_github_rate_limit)
        } else {
            None
        }
    }
    pub fn include_dirs_changed(&self) -> bool { /* ... */ }
    pub fn inline_dirs_changed(&self) -> bool { /* ... */ }
    pub fn navigation_keys_changed(&self) -> bool { /* ... */ }
    pub fn discovery_shimmer_enabled_flipped(&self) -> Option<bool> { /* ... */ }
    pub fn lint_cargo_args_changed(&self) -> bool { /* ... */ }
    // ... one accessor per field a subscriber asks about
}
```

`Event::ConfigChanged` carries `prev: CargoPortConfig, next: CargoPortConfig`
(per the Arc decision above); each subscriber's `handle()` constructs
`ConfigDiff { prev, next }` once and queries the fields it cares
about. This prevents the field-diff logic from being duplicated
across subscribers and keeps `handle()` bodies readable rather than
becoming god-methods that re-derive the diff inline.

Lives in `src/tui/app/bus.rs` next to `Event` and `Command`.

No `HandlerCtx` struct needed. The trait carries `&mut self` (the
subsystem's own state, mutable) and `&Scan` (cross-cutting read).
That's it.

**`Scan` itself is not a subscriber** — Rust would reject
`self.scan.handle(&event, &self.scan)` as same-field aliasing.
Reactions that today fire from `Scan`-related config changes are
dispatched directly by App in `apply_command` (specifically via the
`RescanRequested` event re-publish path: a non-Scan subscriber
inspects the new config and emits `Command::PublishEvent(Event::RescanRequested)`).
The `RescanRequested` event then drives App's own `apply_command`
arm to clear scan state and respawn the watcher — App is the one
holding `&mut self.scan` and `&mut self.background` for those
mutations, no subsystem-level handler needed. No separate
`RescanWatcher` type — App owns the dispatch.

**Module locations** for the bus types: define `Event`, `Command`,
`EventBus`, `EventHandler` in a new `src/tui/app/bus.rs` module.
`StartupOrchestrator` lives at
`src/tui/app/async_tasks/startup_phase/orchestrator.rs` (already
specified earlier). `EventHandler` impls live next to each
subscriber's existing struct definition: `Lint` in
`src/tui/lint_state.rs`, `Ci` in `src/tui/ci_state.rs`, `Net` in
`src/tui/net_state.rs`, `ToastManager` in `src/tui/toasts/`,
`Selection` in `src/tui/selection.rs`, `Inflight` in
`src/tui/inflight.rs` (or wherever it lives).

**`EventBus` (~10 lines):**
```rust
pub struct EventBus {
    queue: VecDeque<Event>,
}

impl EventBus {
    pub fn new() -> Self { Self { queue: VecDeque::new() } }
    pub fn publish(&mut self, event: Event) { self.queue.push_back(event); }
    pub fn pop(&mut self) -> Option<Event> { self.queue.pop_front() }
    pub fn is_empty(&self) -> bool { self.queue.is_empty() }
}
```

**Drain loop on App** — explicitly names each subscriber. This is
not a missed abstraction; it's the natural fit for Rust's ownership
model. App owns every subsystem; only App can hand out `&mut` to
each one in turn.

```rust
impl App {
    fn drain_events(&mut self) {
        while let Some(event) = self.bus.pop() {
            let mut cmds = Vec::new();
            // Each subscriber: &mut its own subsystem field, &Scan as shared.
            cmds.extend(self.lint.handle(&event, &self.scan));
            cmds.extend(self.net.handle(&event, &self.scan));
            cmds.extend(self.ci.handle(&event, &self.scan));
            cmds.extend(self.toasts.handle(&event, &self.scan));
            cmds.extend(self.selection.handle(&event, &self.scan));
            cmds.extend(self.inflight.handle(&event, &self.scan));
            cmds.extend(self.startup_orchestrator.handle(&event, &self.scan));
            // 7 subscribers above. `Scan` is NOT a subscriber
            // (self-aliasing); `Background`'s only reaction
            // (watcher channel rebuild on tui.include_dirs change)
            // is dispatched directly by App from the
            // `RescanRequested` arm of `apply_command`, not via
            // `EventHandler`.

            for cmd in cmds { self.apply_command(cmd); }
        }
    }

    fn apply_command(&mut self, cmd: Command) {
        match cmd {
            Command::PushToast { title, body, style } => {
                self.toasts.push_styled(title, body, style);
            }
            Command::PublishEvent(ev) => self.bus.publish(ev),
            Command::SetServiceAvailability(kind, avail) => {
                self.net.set_service_availability(kind, avail);
            }
            Command::RestartLintRuntime => { /* spawn + swap */ }
            // ... arm per Command variant (21 total)
        }
    }
}
```

**Where `apply_command` lives.** On App. Each arm is a 1-3 line
mutation against the target subsystem (`self.toasts.push_styled(...)`,
`self.net.set_service_availability(...)`, etc.). 21 arms total.
Yes, this technically creates a "god-of-commands" — but it is a
deliberate tradeoff:

- **What App still owns:** the dispatch table (which command goes
  where). That's mechanical routing, not orchestration logic. Each
  arm is a function call into the target subsystem. The arm body
  contains no business logic — that lives in the subsystem method
  the arm calls.
- **What App no longer owns:** the *fan-out decisions*. Today,
  `apply_config` knows that a config change should fire 6
  reactions in 6 different subsystems. After Phase 11, App's
  `apply_config` doesn't know which subsystems react — it only
  knows `bus.publish(Event::ConfigChanged); drain_events()`. The
  fan-out lives in each subscriber's `handle()` body.

`apply_command` could be split across subsystems via a `Reducer`
trait (`trait Reducer { fn reduce(&mut self, cmd: Command); }`,
each subsystem implements it for its own command variants), but
that adds indirection without removing the god-table — App still
has to dispatch each command to the right Reducer. Not worth the
complexity. **Decision: keep `apply_command` on App as a flat
21-arm match.** The match is mechanical; the orchestration is in
the handlers.

**Re-entrancy semantics:** subscriber A handles `ConfigChanged`,
returns `Command::PublishEvent(RescanRequested)`. App's
`apply_command` calls `self.bus.publish(RescanRequested)`, which
pushes onto the back of `bus.queue`. The outer `while let Some(event)
= self.bus.pop()` loop sees it on the next iteration.

**Command ordering within one event's drain:** if a handler
returns `[Cmd::PushToast, Cmd::PublishEvent(Foo), Cmd::ScheduleServiceRetry(...)]`,
App's loop is `for cmd in cmds { self.apply_command(cmd); }` — so
`PushToast` runs, then `PublishEvent(Foo)` (which appends to the
queue), then `ScheduleServiceRetry`. **All commands from the
current event finish applying before any newly-published event is
drained.** The newly-published `Foo` is processed only after every
subscriber has finished `handle(current_event)` AND every command
from those handlers has been applied. Subscribers writing handlers
can rely on this: ordering within their own returned `Vec<Command>`
is preserved; ordering between their commands and downstream
subscribers' reactions to a re-published event is "subscribers
first, downstream later."

**Termination invariant** (must hold per-subscriber, enforced by
review):

1. **No subscriber publishes the event it is currently handling.**
   `Lint::handle(ConfigChanged)` may not return
   `Command::PublishEvent(Event::ConfigChanged { ... })`.
2. **`Event::ConfigChanged` is published only from `App::apply_config`.**
   Subscribers may not synthesize it from any other event.
3. **`Event::RescanRequested` only triggers `Event::ScanRestarted` (via
   App's `apply_command` arm), never `ConfigChanged`.**
4. **Subscribers that emit `Command::PublishEvent(...)` MUST emit a
   downstream event in the dependency DAG**, never an upstream one.
   The DAG (verifiable in `bus.rs`):
   `ConfigChanged → {LintConfigChanged, ServiceRateLimited,
   ServiceRecovered, RescanRequested}`,
   `RescanRequested → ScanRestarted → TreeRebuilt → SelectionChanged`,
   etc. No back-edges.

**Debug-build cycle detection** (cheap insurance): drain loop
maintains a counter, hard-asserts at e.g. 1000 events per drain.
Catches infinite loops in dev without slowing production.

```rust
fn drain_events(&mut self) {
    let mut iter = 0;
    while let Some(event) = self.bus.pop() {
        iter += 1;
        debug_assert!(iter < 1000, "event bus runaway: {:?}", event);
        // ... rest of drain
    }
}
```

**Borrow-checker check:** in `drain_events`, the loop iteration
borrows `self.bus` *temporarily* via `pop()` (returns `Option<Event>`,
borrow released). Then each `self.<subsystem>.handle(...)` reborrows
a single subsystem field mutably alongside `&self.scan` (shared) —
disjoint, sound. Each `apply_command` reborrows whichever subsystem
the command targets. No two `&mut` at the same time. **Compiles.**

**`StartupOrchestrator`** (`src/tui/app/async_tasks/startup_phase/orchestrator.rs`):
```rust
pub(crate) struct StartupOrchestrator {
    phase: StartupPhaseTracker,  // existing tracker state moves here
}

impl StartupOrchestrator {
    pub fn new() -> Self { /* ... */ }

    /// Called once per tick. Examines tracker state, publishes
    /// StartupPhaseAdvanced events in dependency order (disk → git
    /// → repo → metadata → lints → ready).
    pub fn advance(&mut self, bus: &mut EventBus, now: Instant, scan: &Scan, lint: &Lint) {
        if self.phase.disk_complete_at.is_none() && self.disk_done(scan) {
            self.phase.disk_complete_at = Some(now);
            bus.publish(Event::StartupPhaseAdvanced(StartupPhase::Disk));
        }
        if self.phase.disk_complete_at.is_some()
            && self.phase.git_complete_at.is_none()
            && self.git_done(scan)
        {
            self.phase.git_complete_at = Some(now);
            bus.publish(Event::StartupPhaseAdvanced(StartupPhase::Git));
        }
        // ... per-phase rules in dependency order; ready_phase last
    }
}

impl EventHandler for StartupOrchestrator {
    fn handle(&mut self, event: &Event, _scan: &Scan) -> Vec<Command> {
        match event {
            Event::StartupPhaseAdvanced(phase) => {
                // Update tracked-item completion in toasts via Command.
                vec![Command::MarkStartupPhaseCompleted(*phase)]
            }
            _ => vec![],
        }
    }
}
```

The orchestrator does **not** orchestrate via App method calls; it
publishes events. Phase ordering rules live in `advance()`'s body.
Subscribers (Toasts, etc.) react via `handle()` + commands.

**Where `advance` is invoked.** App's main poll loop already calls
`maybe_log_startup_phase_completions` every tick (today, in
`tick_once` or equivalent). After Phase 11, that call becomes:
```rust
fn tick_once(&mut self) {
    // ... existing per-tick work
    self.startup_orchestrator.advance(
        &mut self.bus, Instant::now(), &self.scan, &self.lint
    );
    self.drain_events();
}
```
`advance` examines tracker state and publishes `StartupPhaseAdvanced`
events; `drain_events` delivers them to subscribers.

**End-to-end traced flow — user saves config:**

1. Settings overlay closes; calls `app.apply_config(new_cfg)`.
2. `apply_config` (now thin):
   ```rust
   let prev = self.config.current().clone();
   self.config.replace(new_cfg);
   let next = self.config.current().clone();
   self.bus.publish(Event::ConfigChanged { prev, next });
   self.drain_events();
   ```
3. `drain_events` pops `ConfigChanged`. Calls each subscriber:
   - `Lint::handle(ConfigChanged)` → if `lint.enabled` flipped or
     `lint.cargo_args` changed, returns `[Command::RestartLintRuntime,
     Command::PushToast { title: "Lint runtime", body: warning, style: Warning }]`
   - `Net::handle(ConfigChanged)` → if `force_github_rate_limit`
     toggled, returns `[Command::PublishEvent(Event::ServiceRateLimited(ServiceKind::GitHub))]`
     (or `Recovered`).
   - `Selection::handle(ConfigChanged)` → if `lint.enabled` flipped,
     returns `[Command::ResetFitWidths { lint_enabled: next.lint.enabled }]`.
   - **Scan does NOT subscribe directly.** The `tui.include_dirs`-
     changed reaction comes from a non-Scan subscriber (e.g. Lint
     reads the new config and notices the dirs differ from the
     prior scan), which emits
     `Command::PublishEvent(Event::RescanRequested)`. App's
     `apply_command` arm for `RescanRequested` does the actual scan
     state mutation directly (`self.scan.projects_mut().clear()`,
     `self.background.swap_bg_channel(...)`, etc.) — App holds
     `&mut self.scan` and `&mut self.background` together with no
     subscriber-level borrow conflict.
4. App applies each command:
   - `RestartLintRuntime` → spawns new runtime, swaps in.
   - `PushToast` → `self.toasts.push_styled(...)`.
   - `PublishEvent(ServiceRateLimited(GitHub))` → adds to queue.
   - `ResetFitWidths { lint_enabled }` → `self.selection.reset_fit_widths(lint_enabled)`.
   - `PublishEvent(RescanRequested)` → adds to queue.
5. Loop iterates. Pops `ServiceRateLimited(GitHub)`. Subscribers
   handle (Net updates availability, Toasts pushes "GitHub rate-limited"
   toast). Drains.
6. Loop iterates. Pops `RescanRequested`. **No subscriber handles it
   via the `EventHandler` trait** (Scan can't, per the self-aliasing
   constraint). Instead, App's `apply_command` arm for the implicit
   "rescan request" handles it directly with `&mut self.scan` and
   `&mut self.background` in scope: clears scan state, swaps the bg
   channel, respawns the watcher, registers projects, fires
   `Event::ScanRestarted` via `bus.publish` for downstream
   subscribers. App is the one subsystem with all the borrows it
   needs to do this in one place.
7. Queue empties; `drain_events` returns. `apply_config` returns.

App's `apply_config` body shrinks from ~50 lines (today's per-subsystem
fan-out) to ~5 lines (publish + drain). The fan-out logic moves into
each subsystem's `handle()` body. Ordering rules that today are
encoded by call sequence become explicit through event chaining.

**`apply_service_signal`** similarly shrinks to:
```rust
pub fn apply_service_signal(&mut self, signal: ServiceSignal) {
    self.bus.publish(match signal {
        ServiceSignal::Reachable(k)    => Event::ServiceReachable(k),
        ServiceSignal::Unreachable(k)  => Event::ServiceUnreachable(k),
        ServiceSignal::RateLimited(k)  => Event::ServiceRateLimited(k),
    });
    self.drain_events();
}
```

**`mark_service_recovered`** today is a separate App method that
fires when the user clears the force-rate-limit flag (called
directly from `apply_config`'s force-flag branch). Under the bus
design it becomes:
```rust
pub fn mark_service_recovered(&mut self, kind: ServiceKind) {
    self.bus.publish(Event::ServiceRecovered(kind));
    self.drain_events();
}
```
Net's `handle(ServiceRecovered)` returns
`[Command::MarkServiceRecovered(kind), Command::DismissToast(prev_toast_id)]`.

`apply_config`'s force-flag branch (today calling
`self.apply_service_signal(...)` and `self.mark_service_recovered(...)`
directly) instead returns from Net's `handle(ConfigChanged)` body:
- if force flipped to true:  `[Command::PublishEvent(Event::ServiceRateLimited(GitHub))]`
- if force flipped to false: `[Command::PublishEvent(Event::ServiceRecovered(GitHub))]`

No special-case App method needed.

**Status: ready to execute.** All load-bearing artifacts defined:
trait signature, 21-variant command set, drain loop with verified
borrows, re-entrancy semantics with termination invariant, orchestrator
API, end-to-end flow. Open items at execution time:
- Whether `BackgroundMsg` dispatch (handle_bg_msg) becomes a single
  `Event::BackgroundMsgReceived` or stays as direct dispatch
  (recommended: stays direct — it's pattern-match dispatch, not
  fan-out).
- `Inflight` module path — verify before writing the impl.

---

**Earlier review notes (superseded by the design depth above):**
- Full `enum Event` variant list (~10–15 named, only ~4 in doc
  today).
- `HandlerCtx` definition — likely per-event-family rather than
  monolithic, but the design isn't decided. A monolithic ctx that
  carries `&mut` to every subsystem re-creates the original
  multi-borrow problem; a per-family ctx requires deciding the
  families.
- `trait EventHandler` complete signature including ctx parameter.
- Drain-loop pseudo-Rust. Specifically: re-entrancy semantics — when
  subscriber A handles `ConfigChanged` and during handling publishes
  `RescanRequested`, does that get drained in the same drain pass,
  queued for next tick, or rejected? This is the central question
  of any event bus and is currently unaddressed.
- Borrow-checker demonstration (not just claim). The bus owning a
  `VecDeque<Event>` while App walks subscribers per event with
  `&mut self.<subsystem>` is the likely design but not in the doc.
- `StartupOrchestrator`'s struct definition: what state does it
  carry, what's its API surface beyond `advance(&mut bus, now)`?
- One end-to-end traced flow: user saves config →
  `publish(Event::ConfigChanged(...))` → drain → each subscriber's
  `handle()` body sketched → final state. Without this, the
  existing `apply_config` body's ordering needs (lint runtime
  refresh after config write, keymap reload trigger, etc.) cannot
  be verified to map onto the new design.

## Total expected impact

After review, the per-phase numbers are revised down to reflect that the
toast cluster is mostly Toast+Panes orchestrators and that selection
mutators stay on `App`:

| Phase | Cluster | Reduction |
| ----- | ------- | --------- |
| 1   | Config (pub items only) | ~10 |
| 1b  | Keymap (`sync_keymap_stamp` only) | ~1  |
| 2   | Toasts (pure pass-throughs only) | ~2  |
| 3   | Git/Repo extract → ProjectList | ~9 |
| 4   | Ci (pure only; cross-subsystem stays on App) | ~6 |
| 4b  | Scan / metadata pass-throughs | ~5–8 |
| 5   | Discovery shimmer + project predicates | ~10 |
| 6   | Overlays subsystem (incl. Exit + inline_error) | ~18 |
| 7a  | Path-resolution NavRead (rewritten by Phase 9) | ~12 |
| 7b  | Movement/selection mutators stay on App (replaced by Phase 9) | 0 |
| 8   | Focus subsystem | ~16 |
| **Visibility subtotal (Phases 1–8)** | | **~89–92** |
| 9   | Move `Viewport.pos` to `Selection.cursor` (Cluster C) | ~12 |
| 10  | Subsystems own pane state; drop wrapper types (Cluster B for Toasts+) | ~9 |
| 11  | `Bus<Event>` for cross-cutting events (Cluster A) | ~25 |
| **Architectural subtotal (Phases 9–11)** | | **~46** |
| **Grand total** | | **~135–138 of 147** |

After Phase 11, App is down to ~10–12 methods: `new`, `run`, top-level
event entry points (`apply_config`, `rescan`, `handle_bg_msg`) that
publish to the bus, plus a few items that genuinely have no other
home. That's the destination — App as a thin coordinator, not a god.

Earlier drafts overstated reductions because they counted methods that are
already `pub(super)` (and therefore not in mend's 147 `pub` count).
Relocating those methods is still cleanup work, but doesn't change the
warning total.

**Residual ~55–58** are genuine cross-cutting orchestrators (apply_config,
rescan, handle_bg_msg, ensure_detail_cached, ci_for, ci_for_item,
unpublished_ci_branch_name, the 9 Toast+Panes orchestrators, sync_selected_project,
the 12 selection mutators in 7b, the apply_service_signal cluster, and
`tabbable_panes`).

These ~49 are App's actual contract surface — every one of them touches
≥2 subsystems, which is the correct location for an orchestrator method.

## Post-Phase-8 architecture review

### Guiding principle [agreed]

**Work should happen where it is most coupled.** A method belongs with the
data it operates on, not with the UI subsystem that triggered it or the
orchestrator that happens to own multiple subsystems.

When a method's body has multiple parts, decompose by data ownership: each
part moves to its rightful owner (a model type, a subsystem). What remains
on App is *thin glue* — query, dispatch, side-effect — not a body of work.

This principle takes precedence over "find a single owner among existing
subsystems." If the natural owner is a model type (e.g. `RustProject`),
that's where the data-query method goes, even if App or a UI subsystem
was the historical caller.

**Note on cluster sizes.** The per-cluster counts (~25 / ~20 / ~12)
sum to ~57, which roughly matches the residual ~55-58 stuck-pub count
after the 8-phase visibility plan. This correspondence is bookkeeping
— the clusters were sized by sorting the residual into A/B/C — not
external validation that the framings are correct.

### Cluster A — cross-cutting events (~25 methods) [agreed]

A class of App methods where a single event needs to fan out to 5+
subsystems. Examples: `apply_config`, `rescan`, `handle_bg_msg`,
`apply_service_signal`, `mark_service_recovered`, all
`maybe_complete_startup_*`. The fan-out is in the event itself —
whoever writes the method has to know the full subscriber list.

Recognizing this as a single class is the prerequisite for picking a
single fix (Item 6 below proposes `Bus<Event>`). Without the framing,
each method gets a one-off treatment.

### Cluster B — multi-part orchestration (~20 methods) [agreed]

A class of App methods whose body has multiple parts, each with a
different rightful data owner. Example: `start_clean` has 4 parts —
target-dir resolution (belongs on `RustProject`), filesystem check
(no clear owner, just an `fs::exists`), "already clean" toast
(belongs on `Toasts`), running-clean tracking (belongs on `Inflight`).

Per the guiding principle, each part moves to its data owner. What
remains on App is the thin glue (~5 lines for `start_clean`: ask
project, check fs, dispatch).

The method usually doesn't disappear from App entirely — it gets
thinner. Net result is the same: the body of work moves to where the
data lives, App becomes a coordination point rather than an
implementation site.

### Cluster C — borrow-cell mutators (~12 methods) [agreed]

A class of App methods stuck on App because the data they operate on
is split across two structs. Example: `move_down` increments the
selected-row index. "What row is selected" lives in
`Panes::project_list().viewport().pos()`; "which rows exist / which
are expanded" lives in `Selection`. Method needs `&mut Panes` AND
`&mut Selection`.

Methods in this cluster: `move_up`, `move_down`, `move_to_top`,
`move_to_bottom`, `expand`, `collapse`, `expand_all`, `collapse_all`,
`select_project_in_tree`, `select_matching_visible_row`,
`expand_path_in_tree`, `try_collapse`.

Per the guiding principle, the row-index data belongs with the rest
of selection state (`Selection`), not with viewport scroll state
(`Panes`). The data is in the wrong place. Fix the data layout, and
the method moves cleanly onto `Selection`. Item 4 below is the
concrete instance of this fix.

### Item 4 — Move `Viewport.pos` to `Selection.cursor` [agreed]

One field moves: the cursor index. Today it lives in
`Panes::project_list().viewport().pos: usize`. After this change it
lives at `Selection.cursor: usize`.

Everything else stays where it is — scroll offset, viewport
dimensions, all of `Panes` — unchanged.

Rationale: the cursor is "what row is selected," which is exactly
what `Selection` already owns (expanded rows, selected paths, etc.).
The viewport scroll offset is rendering state and stays on `Panes`.

After the move, the 12 Cluster-C methods (`move_up`, `move_down`,
`move_to_top`, `move_to_bottom`, `expand`, `collapse`, `expand_all`,
`collapse_all`, `select_project_in_tree`,
`select_matching_visible_row`, `expand_path_in_tree`, `try_collapse`)
each only need `&mut Selection`. They become methods on `Selection`,
not on App.

**Supersedes Phase 7a's `NavRead { panes }` field.** Phase 7a
proposed a `NavRead { selection, scan, panes }` borrow type because
`selected_row` had to read `panes().project_list().viewport().pos()`.
After Item 4, the cursor is on `Selection`, so `NavRead` no longer
needs `panes`. Rewrite Phase 7a's signature when this item lands
before it (or reorder execution: Item 4 before Phase 7a).

Render code that scrolls to keep the cursor visible reads the cursor
from `Selection` and updates scroll on `Panes` — same logic as
today, just two reads instead of one struct-internal read.

Cost: ~30 call-site updates across `tui/app/navigation/*` and the
render path. Mechanical.

### Item 5 — Subsystems implement `Pane` directly; drop wrapper types [agreed]

Today, `Panes` owns 14 named pane wrapper structs (`PackagePane`,
`LangPane`, `CpuPane`, `GitPane`, `LintsPane`, `CiPane`, `ToastsPane`,
`KeymapPane`, `SettingsPane`, `FinderPane`, `TargetsPane`,
`ProjectListPane`, `OutputPane`, plus per-pane payload types in
`Option<...>` inside them). Most are trivial — just
`viewport: Viewport + content: Option<T>`. The `Pane` trait
(in `tui/panes/dispatch.rs`) is for render dispatch.

Per the guiding principle, the per-pane state belongs where the data
lives:
- `ToastManager` adds `viewport: Viewport`, implements `Pane`
  directly. Absorbs the `ToastsPane` wrapper.
- `Ci` adds `viewport: Viewport`, absorbs `CiData`, implements `Pane`.
- `Lint` adds `viewport: Viewport`, absorbs `LintsData`, implements
  `Pane`.
- `Selection` (with the cursor from Item 4) absorbs the
  `ProjectListPane` state, implements `Pane`.
- Overlays (Phase 6 subsystem) absorb `KeymapPane`, `SettingsPane`,
  `FinderPane`.
- Detail panes (`Package`, `Lang`, `Git`, `Targets`) — built per-frame
  from existing data; if there's no clear data owner, keep the
  wrapper *with a documented reason*, not as default god-bag growth.

After: `Panes` shrinks to a render-dispatch registry of `&dyn Pane`
references plus genuinely cross-pane state (focus, hover dispatch,
layout cache). Several wrapper types do not collapse — see "What
stays on the render side" below.

**What stays on the render side, not in subsystems:**

Per-frame hit-test layout is not domain state. It's recorded each
frame for the next click/hover dispatch. It belongs in a render-side
container, not in a domain subsystem. Specifically:
- `ToastsPane.hits: Vec<ToastHitbox>`
- `CpuPane.row_rects` (and `CpuPoller`)
- `GitPane.worktree_summary_cache` (RefCell), `GitRowLayout`
- `SettingsPane.line_targets`
- `ProjectListPane.dismiss_actions: Vec<(Rect, DismissTarget)>`

These do *not* move into `ToastManager` / `Ci` / `Lint` / `Selection`.
They stay in render-side wrapper types (slimmed-down versions of the
existing structs, owning only the per-frame layout). Naming them
something like `ToastsLayout`, `CpuLayout`, etc. (renaming is part of
the work) keeps the layout/domain separation visible.

**`Pane` trait visibility.** Today `Pane` is `pub(super)` in
`tui/panes/dispatch.rs`. If subsystems outside `tui::panes`
implement it (`ToastManager`, `Ci`, `Lint`, `Selection`), the trait
needs widening to `pub(crate)`. Add to the migration steps for
this item.

**`PaneRenderCtx` reach.** Today `PaneRenderCtx` carries `Config`,
`Scan`, and `selected_project_path`. Some subsystems' render bodies
need more — e.g. the project-list render reads broader `App` state
(lint enabled, config flags). When `Selection` implements `Pane`
directly, either `PaneRenderCtx` grows to carry the broader subsystem
references, or render-time accessors fan out via `&App`. This is
real work, not a one-line trait impl. Acknowledge before execution.

**Detail-pane data cache.** `ensure_detail_cached` builds and caches
`DetailPaneData` keyed by `DetailCacheKey { row, generation }` —
the plan's "built per-frame" claim is wrong; there's a real cache.
The cache state has to live somewhere. Options:
- (a) Cache moves to a dedicated `DetailCache` type owned by App,
  outside the per-pane wrapper world entirely.
- (b) Cache stays inside the existing `PackagePane`/`GitPane`/
  `TargetsPane` wrappers (which then keep their wrapper status, with
  the cache as the documented reason).
Option (a) is cleaner per the principle. Pick at execution time.

This makes Items 5-equivalent (remove the 9 Toast+Panes orchestrators
from App) a side-effect of the broader fix — methods like
`prune_toasts`, `show_timed_toast`, `start_task_toast` become
`ToastManager` methods because `ToastManager` now owns the viewport
they need to update.

Cost: relocating 14 wrapper types and updating call sites that
read `panes.<x>()` to `app.<x>()` directly (since each subsystem is
now its own pane). Mechanical but broad.

### Item 6 — `Bus<Event>` for cross-cutting events [agreed]

Today, ~25 App methods fan a single event out to N subsystems by
direct method calls in sequence. Examples: `apply_config`,
`apply_service_signal`, `mark_service_recovered`, the
`maybe_complete_startup_*` family.

The change: introduce a small in-process event bus.

```rust
enum Event {
    ServiceSignal(ServiceSignal),
    ServiceRecovered(ServiceKind),
    ConfigChanged(...),
    StartupPhaseAdvanced(Phase, Instant),
    // ... ~10-15 variants
}

trait EventHandler {
    fn handle(&mut self, ev: &Event, ctx: &mut HandlerCtx);
}
```

Each subsystem subscribes to the events it cares about. App
publishes events when raw triggers fire (config save, bg msg
arrival, scan complete). The bus delivers to subscribers.

Decisions:
- **Two-phase delivery.** Bus collects events into a queue, then
  drains to each subscriber serially. Avoids the borrow-checker
  conflict from subscribers wanting `&mut self` while the bus
  iterates its own list. Channels were considered and rejected
  (introduces async-style scheduling for a synchronous flow).
- **Handler ctx.** Each subscriber receives `&mut HandlerCtx`
  carrying the cross-subsystem references it needs to mutate. The
  ctx assembly happens at bus creation, once.

After: ~25 App methods collapse into a handful (`publish` calls + a
small dispatch root). Subsystems own their own reactions.

**Risk — startup-phase ordering.** Today
`maybe_log_startup_phase_completions` calls
`maybe_complete_startup_disk` → `_git` → `_repo` → `_metadata` →
`_lints` → `_ready` in that exact order. Repo-phase gates on
git-phase complete; ready-phase gates on all four prior. Moving to
a bus means this ordering is expressed as event chaining
(`StartupPhaseAdvanced(Disk)` published before `StartupPhaseAdvanced(Git)`)
or via a small sequencing layer that publishes events in the
correct order. Sequencing layer is preferred — keeps the ordering
rules explicit, doesn't smear them across subscribers.

**Where the sequencing layer lives.** Add a `StartupOrchestrator`
type at `src/tui/app/async_tasks/startup_phase/orchestrator.rs`
(or merged into the existing `tracker.rs`). It owns the sequencing
rules and publishes `StartupPhaseAdvanced(...)` events in dependency
order. App's role narrows to: when a tick fires, call
`startup_orchestrator.advance(&mut bus, now)`. The orchestrator
**replaces** the App methods, not just renames them — its body
encodes the rules; subscribers don't see ordering, they see events.
This is *not* re-introducing the App orchestrator under a different
name; it's a small focused type with one job.

**Tradeoff — call graph readability.** Following an event flow now
means reading the subscriber list, not a single method body.
Acceptable in exchange for App no longer being the directory of
"who reacts to what."

Cost: largest of any item — touches 25 methods plus all subscribers.

### Item 7 — Decline classical DI [agreed]

"Classical DI" here means: pass each subsystem as `&mut impl Trait`
into orchestrator functions, instead of holding subsystems as fields
on App and calling methods on them.

Rejected for this codebase. Reasons:

- **Cluster A methods touch 5-8 subsystems each.** Translating
  `rescan` to `fn rescan(scan: &mut impl Scan, ci: &mut impl Ci,
  lint: &mut impl Lint, net: &mut impl Net, background: &mut impl
  Background, panes: &mut impl Panes, selection: &mut impl Selection,
  inflight: &mut impl Inflight)` produces the same fan-out with worse
  ergonomics, plus trait-per-subsystem boilerplate. Item 6 (`Bus<Event>`)
  is the right answer for this cluster, not DI.
- **Cluster B is already addressed by the data-ownership principle.**
  Moving each part of a multi-part orchestrator to its rightful owner
  produces the same end state DI would (clean signatures, explicit
  dependencies) without trait boilerplate or trait-objects-of-fields.
- **Cluster C is a data-layout problem.** DI doesn't change where
  data lives; only Item 4-style relocations do.

DI is therefore the wrong tool for the actual problem in this
codebase. The right tools are: (Item 4) move misplaced data, (Item 5)
let subsystems own their pane, (Item 6) replace fan-out call chains
with an event bus.

## Not in this plan

- New tests are not added or modified by these phases. If a test exists today
  it stays; if a phase needs new test coverage, that's a follow-up commit.
- The public API visible outside `crate::tui` is unchanged. Callers in `bin/`
  still see `App::run`, `App::new`, etc.
- Subsystem-internal reorganization beyond what a given phase requires is a
  separate follow-up. Each phase's body lists exactly what it touches.

## Sequencing risk

Phase 6 (Overlays) and Phase 8 (Focus) interact. If Phase 6 lands first with
clean boundaries, Phase 8's `tabbable_panes` migration takes a clean
`&Overlays` arg. If Phase 8 lands first, Focus has to read `App.ui_modes`
directly and Phase 6 retraces. Order is fixed: 6 before 8.

Phase 7 (Navigator) is the largest single change. It can be split
in two if the diff exceeds ~600 lines: 7a (read accessors:
`selected_*`, `path_for_row`, `display_path_for_row`, `abs_path_for_row`,
`row_count`, `visible_rows`, `expand_key_for_row`, `selected_is_expandable`)
+ 7b (mutators: `move_*`, `expand`, `collapse_*`, `select_*`,
`expand_path_in_tree`, `ensure_*_cached`). Decide at execution time based on
diff size after 7a.
