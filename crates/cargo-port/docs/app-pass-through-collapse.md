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

## Phase order

Phases run smallest blast radius first. Each phase is one commit, gated by
`cargo check` + `cargo nextest run --workspace` + `cargo +nightly fmt --all`.
Each phase must independently leave the tree green.

Phases 1–8 are visibility narrowings. Phases 9–11 are architectural
moves derived from the post-Phase-8 review (see end of doc). Phase 9
must precede Phase 7a (Item 4 supersedes Phase 7a's `NavRead { panes }`
design).

Per-phase steps:
1. Add the subsystem accessor on `App`.
2. Rewrite call sites: `app.foo_method(args)` → `app.subsystem().foo_method(args)`.
3. Move method definitions from `App` impl blocks into the subsystem `impl`
   (where they likely already exist as `fn` and are simply being unwrapped).
4. Remove the now-unused `App::foo_method` pass-throughs.
5. `cargo mend` after each phase — confirm the warning count drops.

## Phase 1 — `Config` (smallest)

**Subsystem:** `crate::config::ConfigState` (the `App.config: ConfigState` field).

**Pass-throughs to remove from `App` (currently `pub`, contributes to mend count):**
- `editor`, `terminal_command`, `terminal_command_configured`
- `lint_enabled`, `invert_scroll`, `include_non_rust`, `ci_run_count`, `navigation_keys`
- `discovery_shimmer_enabled`, `discovery_shimmer_duration`

**Already `pub(super)` — relocate but no mend count change:**
- `current_config`, `current_config_mut`, `config_path`
- `settings_edit_buf`, `settings_edit_cursor`, `settings_edit_parts_mut`, `set_settings_edit_state`

**Already private `fn` (does not need to move):** `toast_timeout`

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

**Side-channel widening:** the `pub(super)` methods listed today are reachable
from inside `tui/app/`. After relocating into `ConfigState`, the underlying
`ConfigState` methods need to be `pub` (because callers cross modules). This
is a real visibility widening; `ConfigState` becomes part of `App`'s public
contract via `app.config()`. Acceptable because `ConfigState` is the public
config subsystem.

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

**Refactor:** introduce `NavRead<'a> { selection: &'a Selection, scan: &'a Scan, panes: &'a Panes }` returned by `app.nav_read()`. `&panes` is needed because `selected_row` / `selected_is_expandable` read `panes().project_list().viewport().pos()`. All three are `&` borrows, so cohabitation is fine.

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
- 12 Cluster-C methods (`move_up`, `move_down`, `move_to_top`,
  `move_to_bottom`, `expand`, `collapse`, `expand_all`, `collapse_all`,
  `select_project_in_tree`, `select_matching_visible_row`,
  `expand_path_in_tree`, `try_collapse`) become methods on `Selection`.
- ~30 call-site updates in `tui/app/navigation/*` and the render path
  (read cursor from `Selection`, scroll from `Panes`).

**Ordering constraint:** Phase 9 supersedes Phase 7a's `NavRead { panes }`
design. Land Phase 9 before Phase 7a, or skip Phase 7a and let Phase 9
absorb its work.

**Expected mend reduction:** ~12.

**Risk:** medium — touches navigation peer code and render scroll
logic.

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

**Ordering:** Phase 10 lands after Phase 9 (because Phase 9 affects
where the project-list cursor lives, which Phase 10 places into
`Selection`'s `Pane` impl).

**Expected mend reduction:** ~9 (the Toast+Panes orchestrators that
become `ToastManager` methods naturally; symmetric wins for `Ci`,
`Lint`, etc. when they grow methods that touch their viewport).

**Risk:** highest of the architectural phases — touches the render
path, the `Pane` trait, every subsystem with a screen presence.

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

**Expected mend reduction:** ~25 — the largest single drop.

**Risk:** highest of all phases — touches startup-phase ordering,
service-signal handling, and the existing `apply_config` / `rescan`
fan-out logic.

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

## Out-of-scope

- No new tests added or modified.
- No public API changes visible outside `crate::tui` (callers in `bin/` etc.
  still see `App::run`, `App::new`, etc.).
- No subsystem internal reorganization beyond what each phase requires.

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
