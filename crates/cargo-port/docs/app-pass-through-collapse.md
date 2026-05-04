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
- `request_quit`, `request_restart`, `should_quit`, `should_restart`
- `initialize_startup_phase_tracker`, `maybe_log_startup_phase_completions`, every `maybe_complete_startup_*`
- `sync_selected_project`, `enter_action`
- `start_clean` (inflight + toast + projects coordination)
- `apply_service_signal`, `mark_service_recovered`, `spawn_service_retry`, `spawn_rate_limit_prime`

These stay on `App` because each one touches at least two subsystems and
encapsulates a cross-cutting decision.

## Phase order

Phases run smallest blast radius first. Each phase is one commit, gated by
`cargo check` + `cargo nextest run --workspace` + `cargo +nightly fmt --all`.
Each phase must independently leave the tree green.

Per-phase steps:
1. Add the subsystem accessor on `App`.
2. Rewrite call sites: `app.foo_method(args)` → `app.subsystem().foo_method(args)`.
3. Move method definitions from `App` impl blocks into the subsystem `impl`
   (where they likely already exist as `fn` and are simply being unwrapped).
4. Remove the now-unused `App::foo_method` pass-throughs.
5. `cargo mend` after each phase — confirm the warning count drops.

## Phase 1 — `Config` (smallest)

**Subsystem:** `crate::config::ConfigState` (the `App.config: ConfigState` field).

**Pass-throughs to remove from `App`:**
- `editor`, `terminal_command`, `terminal_command_configured`
- `lint_enabled`, `invert_scroll`, `include_non_rust`, `ci_run_count`, `navigation_keys`
- `discovery_shimmer_enabled`, `discovery_shimmer_duration`
- `toast_timeout`

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

**Expected mend reduction:** ~11.

**Risk:** none — these are 1-line read methods.

## Phase 2 — `Toasts`

**Subsystem:** `crate::tui::toasts::ToastManager` (the `App.toasts` field).

**Pass-throughs to remove from `App`:**
- `show_timed_toast`, `show_timed_warning_toast`
- `start_task_toast`, `finish_task_toast`
- `set_task_tracked_items`, `mark_tracked_item_completed`
- `dismiss_toast`, `focused_toast_id`, `active_toasts`, `prune_toasts`
- `toasts_is_alive_for_test` *(`#[cfg(test)]`)*

**Accessor to add:**
```rust
impl App {
    pub fn toasts(&self) -> &ToastManager { &self.toasts }
    pub fn toasts_mut(&mut self) -> &mut ToastManager { &mut self.toasts }
}
```

**Tradeoff:** `prune_toasts` and `start_task_toast` currently also touch
`App.panes` (to update the toasts viewport length). Either keep those two as
orchestrators on `App` (they are not strictly pure pass-throughs), or move
the panes update inside the toast manager via a callback / observer. Prefer
keeping them on `App`: the orchestration is real.

**Expected mend reduction:** ~10.

**Risk:** low — most callers already write `app.toasts.X` directly via field
access in places where it's still `pub(super)` accessible.

## Phase 3 — `Ci`

**Subsystem:** `crate::ci::Ci` (the `App.ci` field).

**Pass-throughs to remove from `App`:**
- `ci_for`, `ci_data_for`, `ci_info_for`
- `ci_is_fetching`, `ci_is_exhausted`
- `selected_ci_path`, `selected_ci_runs`
- `unpublished_ci_branch_name`
- `ci_for_item`

**Accessor to add:**
```rust
impl App {
    pub fn ci(&self) -> &Ci { &self.ci }
}
```

**Tradeoff:** several CI methods (`ci_for`, `ci_for_item`, `unpublished_ci_branch_name`)
need both `Ci` state and `ProjectList`. Two paths:

- **(a) Move them to `Ci` and pass `ProjectList` as an arg:**
  `app.ci().for_path(app.projects(), path)`. Most explicit.
- **(b) Move them to a new `CiQueries<'a> { ci: &'a Ci, projects: &'a ProjectList }`
  borrow type returned by `app.ci_queries()`:**
  `app.ci_queries().for_path(path)`. Hides the cross-borrow.

Pick (a). It's one extra arg per call site (~30 lines changed) and avoids a
new lifetime-parameterized helper type. The arg makes the dependency visible.

**Expected mend reduction:** ~9.

**Risk:** medium — call sites span render code. Update everything in one commit.

## Phase 4 — Git/Repo reads

**Source:** `App.scan.projects()` (a `ProjectList`) — git state lives inside
`ProjectInfo.local_git_state` and `Entry.git_repo.repo_info`.

**Pass-throughs to remove from `App`:**
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

**Risk:** medium — touches render path.

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
- Shimmer methods become methods on a `DiscoveryShimmers` accessor on `Scan`:
  `app.scan().discovery_shimmers_mut().register(path)`.
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

**Tradeoff:** `open_overlay`/`close_overlay` orchestrate focus changes too
(setting `return_focus`). Keep those two on `App` (they touch focus state),
move the rest. `request_quit` and `request_restart` are simple state writes
on `OverlayState.exit` — trivially relocate.

**Expected mend reduction:** ~18.

**Risk:** low — overlay state is well-contained.

## Phase 7 — Selection / Navigation API consolidation

**Source:** `App.selection: Selection` and `App.scan: Scan`. The methods in
`navigation/` (post-split) currently live on `impl App` because they need
both fields.

**Methods to relocate:**
- `selected_row`, `selected_item`, `selected_project_path`, `selected_display_path`
- `clean_selection`, `path_for_row`, `display_path_for_row`, `abs_path_for_row`
- `move_up`, `move_down`, `move_to_top`, `move_to_bottom`, `collapse_anchor_row`
- `expand`, `collapse`, `expand_all`, `collapse_all`, `try_collapse`, `collapse_to`, `collapse_row`
- `expand_key_for_row`, `selected_is_expandable`, `row_count`, `visible_rows`
- `select_project_in_tree`, `select_matching_visible_row`, `expand_path_in_tree`, `row_matches_project_path`
- `ensure_visible_rows_cached`, `ensure_fit_widths_cached`, `ensure_disk_cache`, `ensure_detail_cached`

**Refactor:** introduce `Navigator<'a> { selection: &'a mut Selection, projects: &'a ProjectList }` returned by `app.navigator()` / `app.navigator_mut()`. Methods land on `Navigator`. Worktree path resolution helpers (`worktree_path_ref`, `worktree_member_abs_path`, etc.) move to free fns on `RootItem` / `WorktreeGroup` — they don't need `App` at all.

**Tradeoff:** `Navigator` introduces a borrow-checker constraint: while it's
held, `app.scan` and `app.selection` are borrowed. Most callers either
read OR write, not both, so this rarely bites. Where it does (rare combined
flow), drop the navigator before reaching for another subsystem.

**Expected mend reduction:** ~30 (the navigation peer is the biggest
pass-through cluster after the split).

**Risk:** medium-high — touches the most call sites of any phase.

## Phase 8 — `Focus` subsystem

**Source:** scattered `App` fields (`base_focus`, `return_focus`, etc.) plus
overlay-aware focus computations.

**Methods to relocate:**
- `base_focus`, `focus_pane`, `focus_next_pane`, `focus_previous_pane`
- `is_focused`, `pane_focus_state`, `focused_dismiss_target`
- `tabbable_panes`
- `selection_changed`, `clear_selection_changed`, `mark_selection_changed`
- `mark_terminal_dirty`, `clear_terminal_dirty`, `terminal_is_dirty`
- `input_context`

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

**Expected mend reduction:** ~20.

**Risk:** highest — touches every keyboard handler.

## Total expected impact

Cumulative mend reduction across all phases: **~117 of the current 147
pubs**. The residual ~30 are genuinely cross-cutting orchestrators that
belong on `App` and should stay `pub`. After Phase 8, mend's report
roughly matches `App`'s actual contract surface.

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
