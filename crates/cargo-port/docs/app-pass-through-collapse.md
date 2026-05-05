# Collapsing App's pass-through accessors

## Status

Phases 1–19 done. Phases 20–22 remain.

**Phase 20 — Rip out the event bus.** Revert Phase 18. The
architectural review found the bus pattern doesn't fit this
codebase; the cross-subsystem orchestrators on `App` are
themselves the fan-out point, and a queue-and-dispatch layer
adds dispatch indirection without architectural value. After
Phase 20, no bus types exist; `apply_service_signal` is a
direct method call.

**Phase 21 — `TreeReaction` enum cleanup of `ReloadActions`.**
Replace mutually-exclusive `rescan` / `rebuild_tree` booleans
with a `TreeReaction` enum. Type system refuses combinations
that today rely on runtime branching to stay correct.

**Phase 22 — Extract `StartupOrchestrator` (state-owning).**
Move the startup-phase state machine into its own subsystem.
Phase-tracking state migrates off `scan_state` and `lint` (it
isn't scan data; it isn't lint data) into the orchestrator.

All three remaining phases are reorderable — none has a source
dependency on the others.

**End state.** The codebase has one architectural pattern: `App`
owns named subsystems by reference; cross-subsystem reactions
are named methods on `App` that call into subsystems directly;
no event-bus indirection. This matches Rust's ownership model
directly — `App` holds `&mut self` to all its subsystems and is
the only place those references compose.

**Why this plan exists.** `App` was a god struct with `impl App { ... }`
blocks scattered across `tui/app/focus.rs`, `tui/app/dismiss.rs`,
`tui/app/query/*.rs`, `tui/app/async_tasks/*.rs`,
`tui/app/navigation/*.rs`. Too many methods on `App`, owned across too
many files, with no subsystem-level encapsulation. The work
extracts subsystems (Config, Keymap, Toasts, Scan, ProjectList git/repo
reads, Ci, Discovery shimmer, Focus, Overlays, plus Phase 22's
StartupOrchestrator) and routes call sites through them, so
each piece of state lives with the subsystem that owns it.

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
Phase 9 (Overlays).

These stay on `App` because each one touches at least two subsystems and
encapsulates a cross-cutting decision.

## Lessons from earlier phases (applied to remaining work)

1. **`pub(super)` from `tui/<subsystem>.rs` reaches the entire `tui/`
   subtree.** No subsystem under `tui/` needs `pub` for its methods —
   `pub(super)` is sufficient.

2. **Hosting matters.** A method on `App` defined inside
   `tui/app/<sub>.rs` only reaches `tui/app/` from its `pub(super)`.
   Methods that need to reach `tui/` (callers in `tui/render.rs`,
   `tui/input.rs`, `tui/panes/*`, etc.) must live in `tui/app/mod.rs`.
   Phase 9's six Group 3 orchestrators (`force_settings_if_unconfigured`,
   `input_context`, `is_pane_tabbable`, `tabbable_panes`,
   `focus_next_pane`/`focus_previous_pane`, `reset_project_panes`)
   were placed in `mod.rs` for this reason.

3. **`pub(crate)` is forbidden inside `tui/`.** Subsystem types that
   need crate-wide visibility (`Focus`, `Overlays`) live at
   `crate::tui::<subsystem>` (outside `tui/app/` and outside any
   private submodule). Subsystem types that only need `tui/`-wide
   visibility (`Selection`, `Scan`, `Lint`, `Net`, `Inflight`,
   `ToastManager`) stay `pub(super)`.

4. **Group 1 / 2 / 3 framework (Phase 9).** When extracting a
   subsystem:
   - **Group 1**: methods absorbed into the new subsystem (move bodies).
   - **Group 2**: callers redirect through existing accessors (zero new
     state, just call-site rewrites). Prefer this when the subsystem
     already exposes what callers need.
   - **Group 3**: cross-subsystem orchestrators stay on App in `mod.rs`.
   Group 2 is cheapest; choose it whenever feasible.

5. **Let post-move tooling tidy imports.** When moving impl blocks
   between files, the auto-generated `crate::tui::*` paths often have
   shorter forms once the destination's `use` set is considered.
   Don't hand-edit imports during a move; run the project's
   import-tidy step afterwards.

6. **Open question — wrapper accessors vs. exposed `current()`.**
   Phase 1 added 10 one-line wrappers on `Config` (e.g.
   `lint_enabled()` returning `self.current().lint.enabled`). The
   alternative — drop the wrappers, let callers write
   `app.config().current().lint.enabled` — is cleaner per the doc's
   own "expose subsystems, don't re-export their methods" principle.
   **Decision: prefer exposing `current()` / accessors to subsystem-owned
   data over wrapping each field with a one-liner.** Don't retroactively
   undo Phase 1; don't repeat the pattern.

## Phase order

Phases run smallest blast radius first. Each phase is one commit, gated by
`cargo check` + `cargo nextest run --workspace` + `cargo +nightly fmt --all`.
Each phase must independently leave the tree green.

Phases 11–16 are the remaining architectural moves.

**Sequence after merges and additions:**

| # of 22 | Phase | Status |
| ------- | ----- | ------ |
| 1 of 22 | Phase 1 — Config | **Done** |
| 2 of 22 | Phase 2 — Trivial subsystems (Keymap + Toasts + Scan/metadata) | **Done** |
| 3 of 22 | Phase 3 — Git/Repo reads → ProjectList | **Done** |
| 4 of 22 | Phase 4 — Ci pass-throughs | **Done** |
| 5 of 22 | Phase 5 — Toast orchestrator relocation | **Done** |
| 6 of 22 | Phase 6 — Discovery shimmer + project predicates | **Done** |
| 7 of 22 | Phase 7 — Internal-helper visibility narrowing pass | **Done** |
| 8 of 22 | Phase 8 — Focus subsystem (lives at `tui/focus.rs`) | **Done** |
| 9 of 22 | Phase 9 — Overlays subsystem (lives at `tui/overlays/`) | **Done** |
| 10 of 22 | Phase 10 — `CiFetchTracker` relocation prep for Phase 11 | **Done** |
| 11 of 22 | Phase 11 — Move `Viewport.pos` → `Selection.cursor` | **Done** |
| 12 of 22 | Phase 12 — Pane trait foundations (cursor-mirror cleanup + `Pane` trait relocation) | **Done** |
| 13 of 22 | Phase 13 — Relocate Panes' dispatch methods to App-level | **Done** |
| 14 of 22 | Phase 14 — `ToastsPane` → `ToastManager` absorption | **Done** |
| 15 of 22 | Phase 15 — `CiPane` → `Ci` absorption | **Done** |
| 16 of 22 | Phase 16 — `LintsPane` → `Lint` absorption | **Done** |
| 17 of 22 | Phase 17 — `KeymapPane` + `SettingsPane` + `FinderPane` → `Overlays` absorption | **Done** |
| 18 of 22 | Phase 18 — Event-bus skeleton + `apply_service_signal` smoke test | **Done** *(reverted in Phase 20)* |
| 19 of 22 | Phase 19 — Move `row_count` + 4 cursor-movement methods onto `Selection` | **Done** |
| 20 of 22 | Phase 20 — Rip out the event bus (revert Phase 18) | Ready |
| 21 of 22 | Phase 21 — `TreeReaction` enum cleanup of `ReloadActions` | Ready (reorderable) |
| 22 of 22 | Phase 22 — Extract `StartupOrchestrator` (state-owning subsystem) | Ready (reorderable) |

Execution order: numeric, except Phase 21 may land at any point
— it has no source dependency on the bus conversion or its
preambles.

Per-phase steps:
1. Add the subsystem accessor on `App`.
2. Rewrite call sites: `app.foo_method(args)` → `app.subsystem().foo_method(args)`.
3. Move method definitions from `App` impl blocks into the subsystem `impl`.
4. Remove the now-unused `App::foo_method` pass-throughs.
5. `cargo build && cargo nextest run --workspace` — confirm tree is green.

## Phase 1 — `Config` (smallest) — **DONE** (`7160e04`)

**Results:**
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

**Pass-throughs to remove from `App`:**
- `editor`, `terminal_command`, `terminal_command_configured`
- `lint_enabled`, `invert_scroll`, `include_non_rust`, `ci_run_count`, `navigation_keys`
- `discovery_shimmer_enabled`, `discovery_shimmer_duration`

**Already private `fn` (does not need to move):** `toast_timeout`

**Out of Phase 1 scope** (despite earlier drafts listing them):
`current_config`, `current_config_mut`, `config_path`,
`settings_edit_buf`, `settings_edit_cursor`,
`settings_edit_parts_mut`, `set_settings_edit_state` are already
`pub(super)`. Relocating them would require touching ~10 callers for
zero structural gain. They stay on App unless a future cleanup phase
rolls them into a focused relocation. Phase 1 does not move them.

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

## Phase 2 — merged trivial subsystems (Keymap + Toasts + Scan/metadata) — **DONE**

Phase 2 combined the original Phases 1b, 2, and 4b into one commit per
the Phase-1 retrospective decision (small phases combine).

**Results:**
- 24 files changed, 178 insertions / 179 deletions (net neutral).
- 597/597 tests pass.

**Methods removed from App (15 total):**
- Keymap: `sync_keymap_stamp`, `current_keymap`, `current_keymap_mut`, `keymap_path`
- Toasts: `active_toasts`, `toasts_is_alive_for_test`
- Scan/metadata: `confirm_verifying`, `clear_confirm_verifying_for`, `metadata_store_handle`, `target_dir_index_ref`, `resolve_metadata`, `resolve_target_dir`, `complete_ci_fetch_for`, `start_ci_fetch_for`, `replace_ci_data_for_path`

**Accessors added on App** (all `pub(super)` in `mod.rs` per Phase 1 lesson 1):
- `keymap()`, `keymap_mut()`, `toasts()`, `scan()`, `scan_mut()`, `ci_mut()`

**Helper methods added on subsystems:**
- `Scan::metadata_store_handle`, `Scan::resolve_metadata`, `Scan::resolve_target_dir`, `Scan::clear_confirm_verifying_for` (`pub(super)`)
- `ProjectList::replace_ci_data_for_path` (`pub(crate)`)
- `ToastManager::active_now` (`pub` — wraps `active(Instant::now())`)

**Lessons (apply to remaining phases):**
1. **`pub(super)` accessors on App belong in `mod.rs`.** Subsystem
   accessors in `mod.rs` reach `tui/` via `pub(super)`. Accessors in
   `query/<file>.rs` only reach `query/`, forcing `pub`. **Caveat:**
   Phase 1's `app.config()` was placed in
   `src/tui/app/query/config_accessors.rs` as `pub fn`. Don't repeat
   the pattern.
2. **Helper methods on subsystems should default to `pub(super)`** —
   only widen to `pub` if callers genuinely live outside the
   subsystem's parent module. Verify per-call-site, not per-pattern.
3. **Don't add wrapper convenience methods unless they save meaningful
   boilerplate per caller.** A wrapper that saves no boilerplate just
   adds noise.
4. **Subsystem-internal types' methods may need widening when callers
   cross module boundaries.** `CiFetchTracker::start/complete` had to
   widen because the type lives in `tui/app/types.rs` but callers are
   in `tui/panes/`. This is structural — the type is in the wrong
   place. **Phase 10 candidate:** move `CiFetchTracker` from
   `tui/app/types.rs` to `tui/ci_state.rs` and re-narrow `start`/
   `complete` to `pub(super)`.

## Phase 3 — Git/Repo reads (extract into `ProjectList`) — **DONE**

**Results:**
- 11 files changed, 203 insertions / 202 deletions.
- 597/597 tests pass; clippy clean; smoke-tested via `cargo install --path .`.
- 9 read methods moved from `App` to `ProjectList`: `git_info_for`, `repo_info_for`, `primary_url_for`, `primary_ahead_behind_for`, `fetch_url_for`, `git_status_for`, `git_status_for_item`, `git_sync`, `git_main`.
- `worst_git_status` helper relocated alongside as a free function in `project_list.rs`.
- `tui/app/query/git_repo_queries.rs` deleted entirely; `mod git_repo_queries` removed from `query/mod.rs`.
- ~44 call sites rewritten as `app.projects().<method>(path)` across 11 files.
- Path chosen: **(a)** — kept formatted strings (`git_sync`, `git_main`) as `pub(crate) fn` on `ProjectList` with the existing format logic. Path (b) (typed `SyncDisplay` enum + render-side format) deferred; revisit only if cross-cluster format reuse appears.

**Lessons (apply to remaining phases):**

1. **Empty-file deletion is one extra step worth doing inside the same phase.** When a file's entire contents move out, delete the file and unregister it from `query/mod.rs` (or wherever) in the same commit. Phase 3's `git_repo_queries.rs` deletion saved a follow-up "remove empty module" commit.

2. **Multi-line `app\n.method(` callers are not caught by simple `sed`.** Phase 3 needed a fallback `perl -0pe` pass for the chained-call sites in `tui/app/async_tasks/repo_handlers.rs`, `tui/app/ci.rs`, etc. Future phases should run the perl pass eagerly, not as a fallback. Pattern:
   ```bash
   perl -i -0pe 's/(\bself|\bapp)\n(\s+)\.(<method1>|<method2>|...)\(/$1\n$2.<accessor>()\n$2.$3(/g' <files>
   ```

3. **Path-(a)-vs-(b) tradeoff: default to (a) when no caller benefits from the typed return.** Phase 3's `git_sync`/`git_main` produce strings consumed only by the project-list pane render. Path (b) (typed enum + render-side format) introduces a new pub type and a format helper for *no caller* that needs the typed form. The "model layer stays formatting-free" argument is real but only pays off when ≥2 callers branch on the typed state. **Default for future phases: (a) unless you can name two callers that need the structural information.**

4. **Subsystem helpers default to the subsystem's existing visibility convention.** `ProjectList` uses `pub(crate)` throughout; new helpers added to it stayed `pub(crate)`. Don't escalate to `pub` unless callers genuinely need it.



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

- removed: ~9 App methods
- added: ~3 likely flagged (any `ProjectList` git/repo helper used from `tui/panes/*` may need `pub`/`pub(crate)`)
- net: ~5–9 (range)

**Risk:** medium — touches render path. Land **before** Phase 4 (Ci) so
`Ci::for_path` can call `ProjectList::primary_ahead_behind_for` rather
than re-implementing it.

## Phase 4 — `Ci` (depends on Phase 3) — **DONE**

**Results:**
- Tests: 597/597 pass; clippy clean; smoke-tested via `cargo install --path .`.
- 3 read methods moved from `App` to `ProjectList`: `ci_data_for`, `ci_info_for`, `unpublished_ci_branch_name`. All `pub(crate)` (free convention on `ProjectList`).
- Helper `unique_item_paths` moved with `ci_for_item` from `query/project_predicates.rs` into `mod.rs` (private `fn`, no `impl App` needed).
- `query/ci_queries.rs` slimmed to 1 method (`ci_is_exhausted`, kept at `pub(super)`); not deleted because `ci_is_exhausted` belongs adjacent to its only caller.
- `app.config()` pulled forward: moved from `query/config_accessors.rs` (`pub`) to `mod.rs` (`pub(super) const fn`), file deleted, `mod config_accessors` removed from `query/mod.rs`.
- Path (a) chosen for `ci_is_fetching`: stayed on `App` as 4-line Ci+ProjectList glue.

**Lessons (apply to remaining phases):**

1. **`pub(crate)` is forbidden inside `tui/`** — only outside the `tui/` subtree (e.g., `crate::project_list`) is `pub(crate)` free. Inside `tui/`, methods that need broader-than-`pub(super)` reach must live in a module whose `pub(super)` already reaches the right scope. Methods on `App` that need to reach `tui/` must live in `tui/app/mod.rs`.

2. **Hosting an App method in `mod.rs` is involuntary.** When a previously-`pub` orchestrator moves up to `mod.rs` to satisfy `pub(super)` rules, the move is itself the visibility narrowing — even when the move wasn't the primary goal.

3. **Helpers travel with their callers when moving up.** When `ci_for_item` moved to `mod.rs`, `unique_item_paths` (a `pub(super)` static helper in `query/project_predicates.rs`) had to move with it, because `pub(super)` from the deeper module doesn't reach `mod.rs`. Audit dependencies before moving methods up; private static helpers without `&self` migrate cleanly as plain `fn` in the destination module.

4. **`query/ci_queries.rs` did not delete cleanly.** Phase 3 lesson 2 (empty-file deletion) didn't apply: `ci_is_exhausted` belongs in `query/` because its only caller (`post_selection.rs`) is also in `query/`, and moving it to `mod.rs` would widen needlessly. Refinement to lesson: delete only when ≥1 method in the file has callers that force broader visibility AND no remaining method has narrow-enough callers to justify staying. Otherwise, slim the file in place.

5. **Pull-forward sub-tasks pay off.** `app.config()` cleanup (originally a Phase 1 leftover) was a 5-minute change that bundled cleanly with Phase 4's diff. Pattern confirmed: each phase should sweep one or two adjacent micro-debts the original plan pre-classified, not gate them behind a separate phase.

## Phase 5 — Toast orchestrator relocation — **DONE**

**Results:**
- 13 files changed (estimated; from `git diff --stat` at execution).
- 597/597 tests pass; clippy clean; smoke-tested via `cargo install --path .`.
- All 11 `pub` methods moved from `query/toasts.rs` to `tui/app/mod.rs` as `pub(super)`: `focused_toast_id`, `prune_toasts`, `show_timed_toast`, `show_timed_warning_toast`, `start_task_toast`, `finish_task_toast`, `set_task_tracked_items`, `mark_tracked_item_completed`, `start_clean`, `clean_spawn_failed`, `dismiss_toast`. Plus the private `toast_timeout` helper.
- `query/toasts.rs` deleted; `mod toasts;` removed from `query/mod.rs`.
- All 11 methods touched **at least two subsystems** (Toasts + Panes for `viewport.set_len(toast_len)` sync after every mutation). Pre-execution review predicted "most touch only ToastManager"; that turned out wrong — every mutator path syncs the pane viewport-len, so every method is a Toasts+Panes orchestrator. No methods moved into `ToastManager` (path (a) of the original strategy was vacant).

**Lessons (apply to remaining phases):**

1. **The "ToastManager-only" prediction was wrong because of the viewport-len sync.** Every toast-mutation method ends with `self.panes_mut().toasts_mut().viewport_mut().set_len(toast_len)` — a Panes write. That coupling is invisible from a method-list scan; only reading the bodies surfaces it. **Apply to future subsystem extractions:** for any "move method into subsystem X" decision, audit body for cross-subsystem writes (especially viewport/cache invalidation) before locking in the absorb-into-subsystem path. **Caveat (post-Phase-5 architecture review):** this pattern is narrower than it first appears. Most pane `viewport_mut().set_len(rows.len())` calls happen inside *render bodies* — those are not cross-subsystem writes from mutators; the render fn already holds `&mut Pane`. The genuine recurrences are: `focus_next_pane`/`focus_previous_pane` and `reset_project_panes` (multiple pane viewport mutations from one orchestrator).

2. **Mechanical mass-move was the right call.** All 11 methods sat in one file with the same coupling, and all 11 had external callers. Moving them as a block to `mod.rs` was one perl pass + one delete. **Pattern:** when an entire `query/*.rs` file's methods share the same "external callers + multi-subsystem touch" property, move the block, don't triage method-by-method.

3. **Phase 4.5 → Phase 5 renumbering paid off immediately.** The retrospective sits cleanly at slot 5 in the canonical sequence with no "see also Phase 4.5" footnotes.

## Phase 6 — Discovery shimmer + project predicates — **DONE**

**Results:**
- 597/597 tests pass; clippy clean; smoke-tested via `cargo install --path .`.
- 4 read methods moved to `ProjectList` as `pub(crate)` (free per Phase 3 lesson 5): `is_deleted`, `is_rust_at_path`, `is_vendored_path`, `is_workspace_member_path`.
- 1 method moved to `Scan` as `pub(super)` (free, outside `tui/app/`): `prune_shimmers` (renamed from `prune_discovery_shimmers`).
- 5 orchestrators relocated to `tui/app/mod.rs` as `pub(super)`: `animation_elapsed`, `register_discovery_shimmer`, `discovery_name_segments_for_path`, `selected_project_is_deleted`, `prune_inactive_project_state`. The first three were involuntary `mod.rs` rehosts per Phase 4 lesson 3; `prune_inactive_project_state` was an unanticipated rehost (its only caller is in `tui/app/construct.rs`, which `pub(super)` from `query/` doesn't reach).
- 2 view-formatting helpers moved to `panes/project_list.rs` as free fns (re-exported from `panes/mod.rs` as `pub(super)`): `formatted_disk`, `formatted_disk_for_item`. Path (a) chosen — pass `&App` to the helper rather than introduce a `DiscoveryShimmerView<'a>` type.
- ~12 private helper fns travelled with `discovery_name_segments_for_path` into `mod.rs` (Phase 4 lesson 4): `discovery_shimmer_session_for_path`, `discovery_shimmer_session_matches`, `discovery_scope_contains`, `discovery_parent_row`, `discovery_shimmer_window_len`, `discovery_shimmer_step_millis`, `discovery_shimmer_phase_offset`, `DiscoveryParentRow` struct, `package_contains_path`, `workspace_contains_path`, `root_item_scope_contains`, `workspace_scope_contains`, `package_scope_contains`, `root_item_parent_row`, `workspace_parent_row`, `package_parent_row`. The two methods on `DiscoveryRowKind` (`allows_parent_kind`, `discriminant`) moved to `tui/app/types.rs` next to the enum, as `pub(super)`.
- 3 files deleted: `query/discovery_shimmer.rs`, `query/project_predicates.rs`, `query/disk.rs`. `query/mod.rs` now lists only `ci_queries` and `post_selection`.
- ~25 call-site rewrites across `panes/project_list.rs`, `panes/support.rs`, `terminal.rs`, `dismiss.rs`, `tests/rows.rs`, `tests/panes.rs`, `tests/state.rs`, `tests/worktrees.rs`, `tests/discovery_shimmer.rs`, `tests/mod.rs`, `async_tasks/lint_handlers.rs`, `async_tasks/lint_runtime.rs`.

**Lessons (apply to remaining phases):**

1. **`pub(crate)` is forbidden across all of `tui/`, not just `tui/app/`.** `pub(crate) fn formatted_disk` in `tui/panes/project_list.rs` was rejected. Outside `tui/` (e.g., `crate::project_list`) is where `pub(crate)` is free. The pattern for `tui/` private-submodule helpers: `pub fn` from a private module + `pub(super) use` re-export from the parent. **For subsystem types at `crate::tui::*` (e.g., `Focus`, `Overlays`):** `pub(crate)` is fine; but their *internal helpers* in private submodules need the `pub fn + pub(super) use` re-export pattern.

2. **Hosting-against-caller-scope audit.** `prune_inactive_project_state` was originally planned to stay in `query/project_predicates.rs` narrowed to `pub(super)`. But its caller is `tui/app/construct.rs`, and `pub(super)` from `query/project_predicates.rs` reaches only `query/` — not `construct.rs` which is a sibling of `query/` under `tui/app/`. So this method had to move to `mod.rs`. **Apply:** for any "narrowed in place" decision, audit the caller's module path against the file's `super`. If the nearest common ancestor is wider than the file's `super`, it's a `mod.rs` rehost.

3. **Phase-5-pattern audit (viewport-len sync) held: shimmer mutations are clean.** `Scan::prune_shimmers(&mut self, now)` is single-borrow `&mut self.discovery_shimmers`. No Panes coupling, no transitive viewport-len sync. Phase 5's coupling pattern is genuinely narrow; don't read it as universal.

## Phase 7 — Internal-helper visibility narrowing pass — **DONE**

**Results:**
- 597/597 tests pass; clippy clean.
- 2 narrowings landed: `ExitMode::should_quit`, `ExitMode::should_restart` (pure-leaf enum methods on `tui/app/types.rs`).
- 57 of 59 hand-audited `pub` items couldn't narrow to `pub(super)`: callers reach them from `tui/render.rs`, `tui/input.rs`, `tui/terminal.rs`, etc. via the `App` impl block, so `pub(super)` from inside `tui/app/<sub>.rs` doesn't reach them. Reverted in place.

**Lesson — `pub(super)`-from-submodule reach is narrower than it looks.** A `pub(super)` method on `App` defined inside `tui/app/<sub>.rs` only reaches `tui/app/`. Methods called from elsewhere in `tui/` must be hosted in `tui/app/mod.rs` to get `pub(super)` reaching `crate::tui`. The 57 reverted candidates are not mechanically narrow-able; they need the structural moves Phases 8/9 already perform (extract subsystem; relocate orchestrator to `mod.rs`).

## Phase 8 — `Focus` subsystem — **DONE**

**Results:**
- 597/597 tests pass; clippy clean.
- New `Focus` subsystem at `src/tui/focus.rs` (outside `tui/app/`, `pub(crate)` methods are free).
- `App` struct fields `focused_pane: PaneId` and `return_focus: Option<PaneId>` removed; replaced with `focus: Focus`. Field `visited: HashSet<PaneId>` removed from `Panes`; ownership moves to `Focus`.
- `Focus` owns: `focused_pane`, `overlay_return` (renamed from `return_focus` to satisfy clippy's `field-name-ends-with-struct-name` lint), `visited`. Methods: `new`, `current`, `is`, `base`, `set`, `open_overlay`, `close_overlay`, `overlay_return`, `retarget_overlay_return`, `overlay_return_is_in`, `unvisit`, `remembers_visited`.
- Wrapper methods deleted from `tui/app/focus.rs` impl App: `is_focused`, `base_focus`, `focus_pane`, `open_overlay`, `close_overlay`, `remembers_selection` (6 deletions).
- `mark_visited`/`unvisit`/`remembers_visited` removed from `Panes` (3 deletions).
- ~30 caller rewrites: `app.is_focused(p)` → `app.focus().is(p)`, `app.focus_pane(p)` → `app.focus_mut().set(p)`, etc., across `render.rs`, `interaction.rs`, `input.rs`, `finder.rs`, `settings.rs`, `panes/*`, `tests/*`.

**Lessons (apply to remaining phases):**

1. **Clippy's `field-name-ends-with-struct-name` matters for new subsystem types.** Clippy rejected `Focus.return_focus`; `Focus.focused_pane` would have triggered too. Renamed to `overlay_return`. **Apply to future subsystem extractions:** when designing field names for new subsystem structs, avoid the struct-name suffix or prefix (use semantic names — `overlay_return` instead of `return_focus`, `current` instead of `focused_pane`).

2. **State migration vs method migration are different scopes.** Phase 8 migrated state (`focused_pane`, `return_focus`, `visited` off App; ownership consolidated in `Focus`). It did NOT migrate the entire `tui/app/focus.rs` file — that requires Phase 9 (overlays) to land first, since most of `focus.rs` is overlay-state methods. **Apply:** future subsystem extractions should explicitly list which state moves (the data) and which methods move (the surface). The two scopes can decouple cleanly.

3. **Clippy's `const fn` hint requires explicit attention.** Three of the new methods needed `const fn` for clippy: `current`, `overlay_return`, `retarget_overlay_return`. Run clippy after every state-method addition; clippy doesn't run as part of the basic cargo check loop.

4. **App struct documentation drifts.** Doc comments on `App.<field>` referencing sub-fields go stale when subsystems extract state. Future subsystem extractions should grep the App-struct doc comments for stale field references.

## Phase 9 — `Overlays` subsystem extraction

**Source:** `App.ui_modes: UiModes` (struct in `tui/app/types.rs`) and the
`KeymapMode`, `FinderMode`, `SettingsMode`, `ExitMode` enums.

**Ordering: Phase 8 before Phase 9 (post-Phase-6 architecture review).**
`open_overlay` (`focus.rs:155-162`) does two writes: `return_focus =
Some(self.base_focus())` and `focused_pane = pane`. Both are pure Focus
state; **neither touches `ui_modes`**. `close_overlay` (`focus.rs:164-167`)
is symmetric. `tabbable_panes` and `is_pane_tabbable` (`focus.rs:213-285`)
also don't read `ui_modes`. Plus `query/post_selection.rs:37-38` —
`sync_selected_project` writes `self.return_focus` directly, also a
Focus field.

Run **Phase 8 first**. After Focus is extracted, `open_overlay` /
`close_overlay` live on `Focus` as pure focus mutations, and Phase 9
(Overlays) becomes a clean state-only extraction with zero focus
entanglement. Doing Phase 9 first forces those two methods to either
stay on App as orchestrators (involuntary `mod.rs` rehost — exactly
the anti-pattern Phase 6 found, where `prune_inactive_project_state`
became an unanticipated rehost) or move to Overlays *and* take
`&mut Focus` callbacks, which is more design surface than just landing
Focus first.

**Three-group split of `tui/app/focus.rs` (post-Phase-8 architecture review):**
the file remains misnamed — most of it is overlay/scan/selection accessors,
not focus. Phase 9 distributes its contents across three destinations:

**Group 1 — pure `UiModes` writes → move to `Overlays` (~14 methods):**
- `is_finder_open`, `open_finder`, `close_finder`
- `is_settings_open`, `is_settings_editing`, `open_settings`, `close_settings`
- `begin_settings_editing`, `end_settings_editing`
- `is_keymap_open`, `open_keymap`, `close_keymap`,
  `keymap_begin_awaiting`, `keymap_end_awaiting`
- `should_quit`, `should_restart`, `request_quit`, `request_restart`

`open_overlay` / `close_overlay` are **already on `Focus`** after Phase 8
— remove from Phase 9 scope.

**Group 2 — misplaced methods → redirect to their actual owners:**
- `is_scan_complete`, `terminal_is_dirty`, `mark_terminal_dirty`,
  `clear_terminal_dirty` → move to `Scan` (or delete and have callers
  use `app.scan().scan_state().phase.is_complete()` etc.)
- `selection_changed`, `mark_selection_changed`, `clear_selection_changed`
  → move to `Selection`

**Group 3 — multi-subsystem orchestrators → stay in `tui/app/mod.rs`:**
- `force_settings_if_unconfigured` (Config + Focus + Overlays + Panes + inline_error)
- `input_context` — could land as a free fn in `tui/shortcuts.rs` taking
  `(&Overlays, &Focus, has_inline_error: bool)`; recommend that path
- `is_pane_tabbable`, `tabbable_panes` — read 5+ subsystems
- `pane_focus_state` — pure Focus read; **move onto `Focus`**
- `focus_next_pane`, `focus_previous_pane`, `reset_project_panes` —
  touch Focus + multiple pane viewports; stay on App, will benefit from
  Phase 12's pane-state migration into subsystems

**Subsystem location decision (post-Phase-4 lesson 1, mirrors Phase 8):**
Two options:
- **(a) `crate::tui::app::overlays::Overlays`** — `pub(super)` reaches only
  `tui/app/`. Every Overlays method called from `tui/settings.rs`,
  `tui/keymap_ui.rs`, `tui/finder.rs`, `tui/interaction.rs`, `tui/render.rs`,
  `tui/terminal.rs` is forced to also live in `tui/app/mod.rs` as a
  `pub(super)` orchestrator (Phase 4 lesson 3). Estimated ~6 involuntary
  rehosts.
- **(b) `crate::tui::overlays::Overlays`** — mirrors `tui/selection.rs`,
  `tui/ci_state.rs`, `tui/toasts.rs`, `tui/focus.rs` (Phase 8 decision).
  Methods are `pub(crate)` (free per Phase 4 lesson 1). External callers
  reach `Overlays` directly via `app.overlays().method()`.

**Pick (b).** Same reasoning as Phase 8's location decision: subsystem
types living outside `tui/app/` get free `pub(crate)` and avoid the
`mod.rs` rehost cascade. Bonus: Phase 12's `RenderSplit` borrow becomes
cleaner because all split fields are then `tui/`-level subsystems with
uniform visibility rules.

**Bundled cleanup (post-Phase-8 review):**
- **`status_flash` field** (`mod.rs:220`, written from `app/ci.rs:96, 117`,
  `async_tasks/config.rs:18, 237`, `construct.rs:197`) — same conceptual
  class as `inline_error` (transient UI feedback). Move into `Overlays`
  alongside `inline_error`, or into `ToastManager` as a dedicated
  status-flash channel. **Pick `Overlays`** for symmetry with `inline_error`.
- **App-struct `panes:` doc comment drift** (`mod.rs:188`) — still mentions
  `visited_panes` which moved to `Focus` in Phase 8. One-line fix; bundle here.

**Refactor:** create `crate::tui::overlays::Overlays` owning the
`UiModes` (post-Phase-8 review: **skip the rename** to `OverlayState` —
`UiModes` is a defensible name and the rename doubles diff noise for
zero architectural payoff, same rationale as Phase 12 deferring `*Layout`).
Methods become `Overlays` methods. `App` exposes:
```rust
impl App {
    pub(super) const fn overlays(&self) -> &Overlays { &self.overlays }
    pub(super) const fn overlays_mut(&mut self) -> &mut Overlays { &mut self.overlays }
}
```

**Tradeoff 1 — focus coupling:** *resolved by running Phase 8 first* (see
"Ordering" above). `open_overlay`/`close_overlay` move to `Focus` as pure
focus mutations; they don't appear in Overlays at all. `request_quit` and
`request_restart` are simple state writes on `OverlayState.exit` —
trivially relocate.

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

- removed: ~18 App methods (with `inline_error` path (a))
- added: ~2 (`overlays()` / `overlays_mut()` accessors on App, `pub(super)`)
- saved: ~6 from picking location (b) over (a) — avoids `mod.rs` rehosts for external callers
- net: **~14–20** (revised up from ~12–18)

**Risk:** low-medium — `inline_error` callers from outside Overlays need
re-routing to `app.overlays().inline_error()`. Audit before commit.

### Phase 9 retrospective

**Outcome:** architectural extraction landed cleanly. New module
`crate::tui::overlays` owns the four overlay-mode enums (`FinderMode`,
`SettingsMode`, `KeymapMode`, `ExitMode`), the `inline_error` slot, and
the `status_flash` slot. App lost three fields (`ui_modes`, `inline_error`,
`status_flash`) and gained one (`overlays: Overlays`).

**Numbers:**
- Tests: 597 / 597 pass.
- Net +151 / -276 lines across 30+ files (mostly mechanical caller
  rewrites; `app/focus.rs` deleted entirely).

**Lessons:**
1. **Group 2 redirects are cheaper than Group 1 absorptions.** Group 1
   moved ~14 methods into Overlays (each is a real method; each adds a
   visibility surface). Group 2 redirected three small selection-sync
   methods through existing `Selection` methods (`sync().is_changed()`,
   `mark_sync_changed`, `mark_sync_stable`) — no new method bodies on
   Selection, just call-site rewrites. Group 3 (six Group 3
   orchestrators) stayed on App but moved from `app/focus.rs` to
   `app/mod.rs` to widen their `pub(super)` reach to `crate::tui` per
   Phase 4 lesson 3 (involuntary mod.rs hosting). **Apply:** when a
   subsystem exposes most of its surface via existing methods, prefer
   redirect (Group 2) over absorption (Group 1) — it adds zero
   visibility surface.
2. **`#[cfg(test)] pub(super) selection_mut()` had to widen.**
   `terminal.rs`'s `clear_selection_changed` and `mark_selection_changed`
   need a mutable Selection handle from outside `tui/app/`. The plan
   assumed Selection's existing methods were enough and routed through
   `app.selection_mut().mark_sync_changed()`. That required dropping
   the `#[cfg(test)]` gate on `selection_mut`. The wide handle is fine
   here because Selection's mutable surface is already gated by the
   `SelectionMutation` guard for visibility-changing ops. **Apply to
   Phase 11:** when moving cursor to Selection, callers reach mutators
   via `selection_mut()` — same pattern, already in place.
3. **Hosting Group 3 in `app/mod.rs` brings inline-path imports.**
   Moving `force_settings_if_unconfigured`, `input_context`,
   `is_pane_tabbable`, etc. into `app/mod.rs` lands `crate::tui::*`
   path-qualified references in a module that already has `use`
   imports for the same paths. **Apply:** after moving impl blocks
   into `mod.rs`, run the project's import-tidy step to shorten
   them rather than hand-editing during the move.

**File-level changes:**
- *Created:* `src/tui/overlays.rs` (~145 lines).
- *Deleted:* `src/tui/app/focus.rs` (Group 1 → Overlays, Group 2 →
  Scan/Selection, Group 3 → `app/mod.rs`).
- *Modified:* `src/tui/app/mod.rs` (added Group 3 orchestrators,
  added `overlays()`/`overlays_mut()`, dropped `ui_modes()`,
  `inline_error()`, `set_inline_error`, `clear_inline_error`,
  widened `selection_mut` to non-test, fixed `panes:` doc drift),
  `src/tui/app/types.rs` (deleted `UiModes`, `FinderMode`, `SettingsMode`,
  `KeymapMode`, `ExitMode`), `src/tui/scan_state.rs` (added
  `is_complete`, `terminal_is_dirty`, `mark_terminal_dirty`,
  `clear_terminal_dirty`), `src/tui/focus.rs` (added `pane_state`),
  ~25 caller files across `tui/` (mechanical `app.X()` →
  `app.overlays().X()` / `app.scan().X()` rewrites).

## Phase 10 — `CiFetchTracker` relocation prep for Phase 11

Move one type to a better file.

**Motivation:** `CiFetchTracker` sits in `src/tui/app/types.rs` but
every caller goes through `Ci::fetch_tracker_mut()` in
`tui/ci_state.rs`. Move the type alongside its accessor.

**Mechanical change:**
- Move `CiFetchTracker` from `tui/app/types.rs` to `tui/ci_state.rs`.
- Delete the `pub(super) use types::CiFetchTracker;` re-export at
  `src/tui/app/mod.rs:133`. `Ci`'s `use super::app::CiFetchTracker;`
  becomes a same-module reference and drops the `use`.
- All four direct callers (`tui/terminal.rs:367`,
  `tui/panes/actions.rs:317`, `tui/app/ci.rs:39,146`,
  `tui/app/async_tasks/tree.rs:145`) reach the type through
  `Ci::fetch_tracker_mut()` and don't need rewriting.

**~30 lines mechanical. Recommend folding into Phase 11's prep step
rather than running as a separate phase commit** — there's no per-phase
commit-boundary justification.

**`query/*` empty-file sweep already complete.** Earlier-draft Part C
is obsolete: Phase 4 deleted `query/config_accessors.rs`; Phase 5
deleted `query/toasts.rs`; Phase 6 deleted `query/discovery_shimmer.rs`,
`query/disk.rs`, `query/project_predicates.rs`; Phase 9 deleted
`app/focus.rs`. The live `query/` files are `ci_queries.rs` (12 lines),
`post_selection.rs` (96 lines), and `mod.rs`. `ci_queries.rs` could
fold into `app/ci.rs` during Phase 10; `post_selection.rs` stays.

**Risk:** zero. The relocation is local; build verifies compilation.

## Phase 11 — Move `Viewport.pos` to `Selection.cursor` (Cluster C)

Source of truth: see "Item 4" in the post-Phase-9 review section
below. Summary:

### Group 1 / 2 / 3 taxonomy (post-Phase-9 framework)

Phase 9's lesson 1 — *redirects (Group 2) are cheaper than absorptions
(Group 1)* — applies cleanly to Phase 11. The work splits three ways:

- **Group 1 — move method bodies into `Selection`:** the 12 Cluster-C
  methods (movement + expand/collapse + select-matching). Each becomes
  a `Selection` method; the App-side wrapper deletes entirely (not
  thinned to a one-liner). Phase 9 widened `selection_mut()` to
  non-test, so external callers reach these via
  `app.selection_mut().move_up()`, etc.
- **Group 2 — call-site rewrites through existing accessors:**
  `selected_row`, `selected_project_path`, `selected_display_path` and
  related path-resolution methods (currently in `app/query/`). After
  the cursor lives on `Selection`, these become reads on
  `app.selection().cursor()` plus pure `RootItem`/`WorktreeGroup`
  walks. Method bodies relocate to `Selection` or to free fns on
  `RootItem`; no new state.
- **Group 3 — stays on App in `mod.rs`:** the four `ensure_*` cache
  methods (`ensure_visible_rows_cached`, `ensure_fit_widths_cached`,
  `ensure_disk_cache`, `ensure_detail_cached`) genuinely fan out
  across ≥2 subsystems (Selection + Scan + Ci/Lint). They stay on App
  per Phase 9 lesson 2 (mod.rs hosting reaches `crate::tui`). The
  `selected_project_path_for_render` wrapper at `mod.rs:769-771`
  becomes dead code the moment `selected_project_path` becomes a
  Selection method — delete during Phase 11.

This taxonomy makes the cost estimate sharper: ~12 Group 1 method
moves, ~6 Group 2 call-site rewrites, 4 Group 3 stays. The "stays as
App orchestrator" framing in earlier drafts is Group 3 by another name.

### Mechanics

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

**Ordering constraint:** Phase 11 supersedes Phase 8's `NavRead { panes }`
design. Land Phase 11 before Phase 8, or skip Phase 8 and let Phase 11
absorb its work.

**No `include_non_rust` cache field on `Selection` (post-Phase-5
correction, reaffirmed post-Phase-9):** earlier drafts proposed caching
`include_non_rust` on `Selection` to avoid re-reading `Config` per
frame, with a sync obligation routed through `apply_config`. That
design has been dropped. Reasons:
- **Phase 9 confirmed redirects beat absorption (lesson 1).** A cache
  field on `Selection` is a Group 1 absorption (new state, new sync
  invariant). Threading the bool through method args is a Group 2
  redirect (zero new state). Phase 9's experience favored Group 2
  every time it was viable. Same applies here.
- The cache field creates a synchronization invariant maintained by
  code review, not by types — exactly the failure mode that already
  exists for `cached_fit_widths` (kept current only by
  `apply_lint_config_change` writing it explicitly,
  `async_tasks/config.rs:233-234`). Adding a second mirror doubles the
  surface.
- The 3 methods that need cross-subsystem reads already take a `&Scan`
  arg; adding `include_non_rust: bool` (or `&Config`) to those
  signatures has the same call-site cost as today's
  `recompute_visibility(scan.projects(), include_non_rust)`.
- `ProjectList::visible_rows(&expanded, include_non_rust)` already
  takes the bool. Threading it through the new `Selection` methods
  matches the existing pattern.

**Replacement design:** `Selection::expand`, `collapse_all`, etc. take
`(&mut self, scan: &Scan, include_non_rust: bool)`. `apply_config`
recomputes visibility *via* `Selection`'s methods rather than mirroring
the bool into `Selection`'s state. Phase 20's `ConfigChanged` event
carries the bool in its payload; subscribers read it from the event,
not from a cached field.

**Phase 8 absorption:** the original Phase 8 method list (~16 movement /
selection mutators) **plus** the renamed Phase 8 (path resolution,
~12 methods) both land inside Phase 11. Path-resolution methods become
`Selection` methods or free fns on `RootItem`; movement mutators become
`&mut Selection` methods. No separate phase; counted in Phase 11's
reduction.

**Acceptance criteria (post-Phase-6 architecture review):**
- **Cursor clamp lives inside `recompute_visibility`, not at every
  caller.** Today's `try_collapse` / `try_expand` paths in
  `tui/app/navigation/expand.rs` skip the `SelectionMutation` guard and
  call `ensure_visible_rows_cached()` separately. After Phase 11, those
  paths must not be allowed to leave cursor pointing past
  `cached_visible_rows.len()` — fold the clamp into
  `recompute_visibility` itself so every recompute path is consistent.
  Single point of truth.
- **`SelectionMutation::mutate` signature change is a call-site sweep.**
  Today's signature is `mutate(projects, include_non_rust)`; Phase 11
  drops `include_non_rust` and threads the bool via method args
  instead. Update every `mutate(...)` caller in
  `tui/app/navigation/expand.rs`, `bulk.rs`. Listed here so it's not
  forgotten.
- **`selected_project_path_for_render` (`mod.rs:763-765`) deletes.**
  It's a one-line wrapper over `selected_project_path()` that exists
  only so render can name a `pub(super)` accessor. After Phase 8
  (Focus extraction) and Phase 11 (Selection cursor),
  `selected_project_path` becomes `pub(crate)` on `Selection` and the
  wrapper is dead.


**Risk:** medium — touches navigation peer code and render scroll
logic.

### Phase 11 design depth

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

**Post-Phase-11 `Viewport`:**
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

**Post-Phase-11 `Selection`** (current at `src/tui/selection.rs:36-45`):
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

**New methods on `Selection`** (Phase 11 surface):
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
Phase 11's new methods that mutate `self.expanded` need both
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
visibility recompute self-contained. Phase 20's extended
`ReloadActions` carries the `include_non_rust` decision, so
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
(`toggle_expand`, `apply_finder`). The Phase 11 bulk-mutation methods
call `recompute_visibility` explicitly at the tail.

**Scroll-follows-cursor mechanism** (corrected): today's
`render_project_list` (`src/tui/panes/project_list.rs:111-210`) does
not contain explicit scroll-follow code — ratatui's `ListState`
computes the scroll internally based on its `selected` index and
the prior `offset`. Today the code reads `viewport.scroll_offset()`,
hands it + the cursor `pos` to `ListState`, lets ratatui decide the
new offset, and writes the result back via `*list_state.offset_mut()`.

After Phase 11: same flow, just two reads. Render reads cursor from
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

### Phase 11 retrospective (foundational field move)

**Outcome:** the foundational field move landed cleanly. `Selection`
now owns `cursor: usize`. The project-list pane's `Viewport.pos`
field is no longer the source of truth — render reads
`selection().cursor()` and writes back ratatui's adjustments via
`selection_mut().set_cursor(...)`. Scroll offset stays on
`Panes::project_list().viewport().scroll_offset()` per the plan
("read cursor from `Selection`, scroll from `Panes`").

**Numbers:**
- Tests: 597 / 597 pass.
- ~58 call-site rewrites across `src/tui/app/navigation/*`,
  `src/tui/app/dismiss.rs`, `src/tui/app/async_tasks/tree.rs`,
  `src/tui/panes/project_list.rs`, `src/tui/interaction.rs`, and
  ~6 test files. Two perl passes — one single-line, one multi-line
  — caught everything except one chained `viewport_mut().home()`
  on `tree.rs` that was hand-edited.

**Lessons:**
1. **`set_pane_pos(PaneId, row)` was a hidden coupling.** The generic
   click handler in `interaction.rs:handle_click` called
   `app.panes_mut().set_pane_pos(pane, row)` — which routed
   `PaneId::ProjectList` to `project_list.viewport_mut().set_pos(row)`.
   That bypassed `Selection.cursor` and silently broke a click test
   (`expanded_tree_rebuild_refreshes_clickable_rows`). **Fix:**
   special-case `PaneId::ProjectList` in `handle_click` to route
   through `app.selection_mut().set_cursor(row)`. Left the
   `ProjectList` arm in `set_pane_pos` as a no-op with a doc comment
   so future callers can't bypass `Selection`.
2. **Cursor clamping in `recompute_visibility` is the right place.**
   The plan's design (see doc:1202-1214) called for clamping cursor
   inside `Selection::recompute_visibility` so external tree
   mutations and config changes that shrink the visible row count
   can't leave cursor pointing past the end. Confirmed: every test
   passes with a 4-line clamp.
3. **Group 1/2 method absorption deferred.** The plan called for
   moving the ~12 movement/expand methods into `Selection` (Group 1)
   and ~6 path-resolution methods (Group 2). This commit moves the
   field only; the methods stay on `App` for now (their bodies
   already read `self.selection.cursor()` directly, so they're
   single-borrow `&mut self` against multiple subsystems and don't
   block any further phase). Group 1/2 absorption can land as a
   follow-up commit when the call-site benefit is concrete.
4. **Per-row styling reads `viewport.pos()` separately from the
   ListState path.** The first build looked correct (tests passed,
   detail panes updated), but the *visible* highlighted row in the
   project list stayed on row 0 while detail panes followed the
   real cursor. Cause: `panes/project_list.rs:build_styled_items`
   calls `pane.selection_state(row_index, focus)`, which compares
   each row against `viewport.pos()` to choose the highlight overlay
   style. With cursor now on `Selection`, `viewport.pos` for the
   project-list pane was never updated and stuck at 0. **Fix:**
   sync `viewport.pos` from `selection.cursor()` at the top of
   `render_project_list` — a writeback to a now-derived field. The
   real cleanup (drop the `viewport.pos` mirror entirely; rewrite
   `selection_state` or `build_styled_items` to take a cursor arg)
   belongs in Group 1/2 absorption. **Apply:** when relocating a
   field that other code reads directly (not via the moved owner),
   audit *all* readers of the original location, not just the
   write-side.

**File-level changes:**
- *Modified:* `src/tui/selection.rs` (added `cursor` field +
  `cursor()`/`set_cursor()` accessors + cursor clamp in
  `recompute_visibility`), `src/tui/panes/system.rs`
  (`set_pane_pos`'s `ProjectList` arm now no-op + doc),
  `src/tui/interaction.rs` (click handler routes ProjectList
  through `selection_mut().set_cursor`), and ~58 call sites across
  `src/tui/`.

**Group 1/2 absorption folds into Phase 14**, not Phase 12. The
`viewport.pos` writeback was already eliminated in Phase 12 Stage 1
(via `Viewport::selection_state_for(cursor, row, focus)`), so the
load-bearing reason to bundle Group 1/2 into Phase 12 is gone.
Phase 14 (`apply_lint_config_change`) already touches Selection's
`recompute_visibility` borrow surface; folding the ~12 movement /
expand methods + ~6 path-resolution methods into Phase 14 keeps all
Selection-mutation work in one phase. Phase 12 stays focused on
pane-state ownership.

**Status: done.**

## Phase 12 — Subsystems own pane state; drop wrapper types (Cluster B for panes)

Source of truth: see "Item 5" in the post-Phase-9 review section
below. Summary:

- Relocate the `Pane` trait from `tui/panes/dispatch.rs` to
  `tui/pane/` (top-level subsystem module under `tui/`). At that
  location `pub(super)` reaches `crate::tui` — sufficient for any
  subsystem (`tui/toasts/`, `tui/ci_state.rs`, etc.) to impl it.
  No `pub(crate)` widening — `pub(crate)` is forbidden inside
  nested `tui/` modules, so the visibility comes from relocation,
  not from widening.
- Move `viewport: Viewport` (and absorbed payload, e.g. `CiData`,
  `LintsData`) into the data subsystems: `ToastManager`, `Ci`, `Lint`,
  `Selection`, `Overlays` (per Phase 9).
- Each of those subsystems implements `Pane` directly.
- Slim render-side wrappers retain only per-frame layout (`hits`,
  `row_rects`, `dismiss_actions`, `worktree_summary_cache`,
  `line_targets`). The `*Layout` rename **defers to a follow-up pass**
  (post-Phase-6 review): doing the rename in the same commit as the
  wrapper-collapse doubles diff noise without speeding the collapse.
  Land the structural change first, rename later.
- Detail-pane data cache (`DetailCacheKey`-keyed) home: **keep on `Panes`**
  (Phase 12 design depth's option (b)). The earlier post-Phase-5 decision
  to extract a `DetailCache` at `crate::tui::detail_cache` was reversed
  in the post-Phase-8 review — pulling render-machinery state into a
  top-level module couples it to nothing and is indirection without
  payoff. The cache is per-frame derived state from cross-subsystem
  reads, and its natural home is the existing `Panes` field that already
  owns per-pane viewport caches.
- `PaneRenderCtx` grows to carry the broader subsystem references
  some implementations need (e.g. `Selection`'s render reads broader
  app state today).
- `Panes` shrinks to a render-dispatch registry of `&dyn Pane` + the
  cross-pane state (focus, hover dispatch, layout cache).

**Ordering:** Phase 12 lands after **both** Phase 9 and Phase 11.
Phase 9 introduces the `Overlays` subsystem that absorbs `KeymapPane`,
`SettingsPane`, `FinderPane`'s domain state — Phase 12 needs
`Overlays` to exist before it can have `Overlays` implement `Pane`.
Phase 11 moves the project-list cursor onto `Selection`, which
Phase 12 needs in place before Selection's `Pane` impl owns the
project-list viewport.

become `ToastManager` methods naturally; symmetric wins for `Ci`,
`Lint`, etc. when they grow methods that touch their viewport).

**Risk:** highest of the architectural phases — touches the render
path, the `Pane` trait, every subsystem with a screen presence.

### Phase 12 design depth

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

This is the lever Phase 12 uses. The codebase already splits App
into disjoint borrows for render. Phase 12 widens the split.

**Post-Phase-12 `Pane` trait** (relocated from `tui/panes/dispatch.rs`
to `tui/pane/` — same signature, visibility stays `pub(super)`,
reach widens via location):
```rust
pub(super) trait Pane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>);
}
```

`pub(super)` from `tui/pane/` reaches `crate::tui` — every subsystem
under `tui/` (`tui/toasts/`, `tui/ci_state.rs`, `tui/lint_state.rs`,
`tui/overlays.rs`) can impl it without widening. **Do not** widen
to `pub(crate)`: `pub(crate)` is forbidden inside nested `tui/`
modules per the project rule, and the relocation makes widening
unnecessary.

**Visibility-tier note (post-Phase-9 clarification, post-Phase-11
correction):** subsystem types that `impl Pane` keep their current
visibility. `Selection`, `Scan`, `Lint`, `Net`, `Inflight`,
`ToastManager` stay at `pub(super)` or `pub` (whatever they are
today). `Focus` and `Overlays` are `pub(crate)` because they live
at `crate::tui::*` (top-level under `tui/`) where the project rule
allows it. Phase 12 leaves these visibility tiers untouched —
relocating the trait is the only structural change.

**Post-Phase-12 `PaneRenderCtx`** (carries the references that
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

**Post-Phase-12 split-borrow** widens to all subsystems that
implement `Pane`:
```rust
pub(super) fn split_for_render(&mut self) -> RenderSplit<'_> {
    // Post-Phase-6 review: precompute Inflight reads here (output_lines /
    // output_visible) and stash on PaneRenderCtx — same pattern as today's
    // selected_project_path precompute. Don't carry &mut Inflight in the
    // split — it's over-broad for the actual render need.
    RenderSplit {
        toasts:    &mut self.toasts,
        ci:        &mut self.ci,
        lint:      &mut self.lint,
        overlays:  &mut self.overlays,    // from Phase 9
        selection: &self.selection,       // shared borrow — render reads only
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
    pub selection: &'a Selection,           // shared — render reads cursor and visible rows
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
project-list render keeps a thin wrapper that borrows. Saves one
design contortion.

**Thesis exception (full enumeration):** Phase 12's headline says
"drop the wrapper types," but the actual outcome is mixed.

| Original wrapper | Fate | Absorbs into | Reason |
|---|---|---|---|
| `ToastsPane` | **collapse** | `ToastManager` (`crate::tui::toasts`) | Viewport + `hits` Vec; pure pass-through wrapper |
| `CiPane` | **collapse** | `Ci` (`crate::tui::ci_state`) | Viewport + content; pure pass-through wrapper |
| `LintsPane` | **collapse** | `Lint` (`crate::tui::lint_state`) | Viewport + content; pure pass-through wrapper |
| `KeymapPane` | **collapse** | `Overlays` (`crate::tui::overlays`) | Phase 9 dependency |
| `SettingsPane` | **collapse** | `Overlays` (`crate::tui::overlays`) | Phase 9 dependency |
| `FinderPane` | **collapse** | `Overlays` (`crate::tui::overlays`) | Phase 9 dependency |
| `ProjectListPane` | **survives (slim)** | — | Wraps `&mut Selection`; borrow-cell pattern (option b) |
| `PackagePane` | **survives** | — | Holds `DetailPaneData` cache |
| `LangPane` | **survives** | — | Viewport-only; no `Lang` subsystem exists to absorb it |
| `GitPane` | **survives** | — | `worktree_summary_cache` + cached detail |
| `TargetsPane` | **survives** | — | Cached targets data |
| `CpuPane` | **survives** | — | Owns `CpuPoller` (background thread state) |
| `OutputPane` | **survives (or absorbed into Inflight)** | `Inflight`? | Open question: `Inflight` already owns example state; could collapse if `Inflight` gains a viewport. Decision deferred. |

Net: **6 wrappers collapse, 7 survive (or 8 if OutputPane stays).**
The collapsed 6 are the toast/Ci/Lint/Overlays group whose
orchestrator methods on App go away.

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
  spanning Selection, Net, Ci, Scan, and git state. After Phase 12,
  these signatures change to `build_ci_data(split: &RenderSplit<'_>)`
  (or similar disjoint-borrow tuple). All ~3 callers update. This
  is the largest signature change in the call-site set. **Sanity
  check before commit:** confirm `RenderSplit<'_>` exposes every
  field these functions read; the current design-depth example
  (~5 fields) doesn't yet enumerate the full set the builders need.
- **`build_styled_items`** (`src/tui/panes/project_list.rs`, called
  from `render_tree_items`) calls
  `pane.selection_state(row_index, focus)`, which compares each row
  against `viewport.pos()` to choose the highlight overlay. After
  Phase 12, **rewrite `selection_state` to take `cursor: usize` as
  an arg** (or have `build_styled_items` accept the cursor and pass
  it down) and **delete the `viewport.pos` writeback at
  `render_project_list`'s top** (Phase 11 left it as a temporary
  mirror). This is the planned removal of the Phase 11 styling-pass
  workaround — Phase 12 owns this cleanup, not a separate phase.
  ~5 LOC of changes, but load-bearing for not letting the mirror
  rot.

**Status: done.** Phase 12 covered cursor-mirror cleanup +
`Pane` trait relocation. Wrapper absorption and the dispatch-fix
prerequisite are split into integer Phases 13–19; see those
sections below.

### Phase 12 stages 1-2 retrospective

**Outcome:** the foundational changes for Phase 12 landed cleanly.
The Phase 11 cursor mirror is gone (no more `viewport.pos` writeback
in the project-list render path). The `Pane` trait now lives at a
top-level location under `tui/`, where `pub(super)` reaches every
subsystem that will need to impl it. Stage 3 (wrapper absorption per
subsystem) hit a structural blocker that the original Phase 12 plan
didn't anticipate, captured below.

**Numbers:**
- Tests: 597 / 597 pass.
- 15 files changed, +199 / -74 lines (mostly mechanical import
  rewrites; one file rename `panes/dispatch.rs` → `pane/dispatch.rs`).
- Zero new visibility surface added — the relocation moves the trait
  to a location where existing visibility rules are sufficient.

**Lessons:**

1. **Plan said "widen `Pane` trait to `pub(crate)`"; project rule
   forbids that.** `pub(crate)` is forbidden inside nested `tui/`
   modules per the project's `feedback_no_pub_in_path.md` rule.
   `tui/panes/dispatch.rs` is nested. Caught at compile time when the
   user pointed it out — the change actually compiled, but the
   project rule supersedes it. **Apply:** plan instructions that say
   "widen to `pub(crate)`" inside `tui/` need a relocation step
   instead. The `crate::tui::<top-level>::*` location pattern (which
   Phase 8/9 used for `Focus`/`Overlays`) is the established way to
   gain `pub(super)` reach across `crate::tui` without widening.

2. **Wrapper absorption requires central pane dispatch to relocate
   first.** Attempted to absorb `ToastsPane.viewport` into
   `ToastManager`. Compiled fine for the storage move, but
   immediately broke `Panes::set_pane_pos`, `Panes::viewport_mut_for`,
   `Panes::apply_hovered_pane_row`, and `Panes::hit_test_at` — all of
   which dispatch by `PaneId`, but Panes can only reach viewports it
   owns. Once a viewport moves to a subsystem App owns directly, Panes
   can't dispatch to it from `&mut self`. The two paths are: (1) move
   all Panes dispatch up to App-level free functions taking `&mut App`
   so they can reach subsystems by name, or (2) mirror viewport on
   both wrapper and subsystem — which is exactly the Phase 11 mirror
   anti-pattern stage 1 just deleted. **Apply:** stage 3 must start
   with relocating Panes' dispatch methods *before* any wrapper
   absorption. That's its own commit, separate from any single
   absorption.

3. **Re-exports cannot widen visibility past the source's own
   declaration.** Initial attempt to expose `Pane` via
   `pub(super) use dispatch::Pane;` from `tui/panes/mod.rs` failed
   with `E0365: Pane is private, and cannot be re-exported`. The
   trait was `pub(super)` at `dispatch.rs`, so its visibility was
   capped at `tui/panes/`. Re-exports compose visibility — they
   don't widen it. **Apply:** if a type needs to be visible past
   where it's defined, the type itself must declare that visibility,
   not a re-export elsewhere. The relocation approach gives it the
   right visibility *at the source*, which is why it works.

**File-level changes:**
- *Created:* `src/tui/pane/dispatch.rs` (relocated trait + supporting
  items).
- *Deleted:* `src/tui/panes/dispatch.rs`.
- *Modified:* `src/tui/pane/mod.rs` (added `mod dispatch` + 6
  `pub(super) use` re-exports), `src/tui/panes/mod.rs` (removed
  `mod dispatch` + `HoverTarget` re-export),
  `src/tui/pane/state.rs` (added `selection_state_for` method),
  `src/tui/panes/project_list.rs` (rewrote `render_tree_items` to
  pass cursor; deleted `viewport.pos` writeback),
  `src/tui/interaction.rs` (import path + test-mod imports),
  `src/tui/panes/{ci,cpu,git,lang,lints,package,pane_impls,system}.rs`
  (import path updates).

---

**Earlier review notes (superseded by the design depth above):**
- The `Pane` trait signature (today `pub(super) trait Pane { fn
  render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx:
  &PaneRenderCtx<'_>); }` — the post-Phase-12 version may need
  different args).
- `PaneRenderCtx`'s post-Phase-12 field list (which subsystem
  references it carries beyond the current `Config`, `Scan`,
  `selected_project_path`).
- Render-loop pseudo-Rust showing how `Panes` dispatches through
  trait-objects whose owning data lives in App fields. Specifically:
  who holds the `&dyn Pane` references, when, and how they compose
  with `&mut App` at the call site.
- Post-Phase-12 `Selection` struct definition (with both Phase 11's
  `cursor` and Phase 12's project-list viewport).
- Detail-pane cache home: pick option (a) `DetailCache` type owned
  by App, or (b) keep wrappers as the documented reason.
- Module locations for the renamed `*Layout` types.
- Enumeration of wrapper-deletion call-site changes.

## Phase 13 — Relocate Panes' dispatch methods to App-level

Wrapper absorption (Phases 14–17) requires central pane dispatch
(`Panes::set_pane_pos`, `Panes::viewport_mut_for`,
`Panes::apply_hovered_pane_row`, `Panes::hit_test_at`) to reach
viewports owned by App-level subsystems, not just by `Panes`. Panes
can't reach those subsystems from `&mut self` — the borrow checker
has no path between sibling App fields. The fix: move dispatch up
to App-level free functions in `tui/interaction.rs`, each taking
`&mut App` (or `&App`) and matching `PaneId` to whichever owner
holds the target viewport. Phase 13 ships green *before* any
absorption because every viewport is still on `Panes`; only the
*location* of dispatch changes.

### Concrete moves

Move four methods out of `impl Panes` into `tui/interaction.rs`
(the only callers of `hit_test_at` already live there). Each free
fn takes `&mut App` (or `&App`) and matches `PaneId`. Per-arm RHS
swaps from `app.panes_mut().<x>_mut().viewport_mut()` (survivors)
to `app.<subsystem>_mut().viewport_mut()` (absorbed subsystems) one
absorption at a time.

1. `Panes::set_pane_pos(&mut self, id: PaneId, row)` →
   `pub(super) fn set_pane_pos(app: &mut App, id: PaneId, row: usize)`.
   Caller: `interaction.rs:29` (click handler).
2. `Panes::viewport_mut_for(&mut self, id: PaneId) -> &mut Viewport` →
   `pub(super) fn viewport_mut_for(app: &mut App, id: PaneId) -> &mut Viewport`.
   Caller: `input.rs:280` (hover dispatch).
3. `Panes::apply_hovered_pane_row(&mut self)` → split into
   `clear_all_hover(app: &mut App)` (the 13-line fan-out) and
   `set_pane_hover(app: &mut App, id: PaneId, row: usize)` reusing
   `viewport_mut_for`. Caller: `render.rs:428`.
4. `Panes::hit_test_at(&self, pos) -> Option<HoverTarget>` →
   `pub(super) fn hit_test_at(app: &App, pos: Position) -> Option<HoverTarget>`.
   The walk over `HITTABLE_Z_ORDER` stays; only the per-arm
   `&dyn Hittable` borrow site moves. Callers: `interaction.rs:20,49`.

### Untouched (stay on `Panes`)

- `Panes::set_hover` (just stores `hovered_row` — no dispatch).
- `Panes::set_detail_data` / `clear_detail_data` (the five detail
  panes are all survivors; orchestrator stays valid). After
  Phases 15-16 (Ci/Lint absorption), the signature gains `&mut Ci` /
  `&mut Lint` alongside `&mut Panes`.
- Render-side dispatch (`dispatch_*_render` in `system.rs:235-304`):
  render already routes through `split_panes_for_render` with
  disjoint borrows. After Phases 14–17, each absorbed pane's render
  moves to its subsystem's `Pane` impl; the dispatch on `Panes`
  becomes survivors-only.

**Estimate:** mechanical, ~4 method moves, ~6 call-site rewrites,
~50 LOC. Tests stay green throughout because every viewport is
still on `Panes` at this point.

**Risk:** low — pure relocation, no semantic change.

### Phase 13 retrospective

**Outcome:** clean relocation. The four dispatch methods now live
in `tui/interaction.rs` as free functions taking `&mut App` (or
`&App`). Each per-arm RHS still reads `app.panes_mut().<x>_mut()`
because every viewport is still on `Panes` — Phases 14–17 will
swap arms one at a time as each absorption lands.

**Numbers:**
- Tests: 597 / 597 pass.
- 4 functions added in `tui/interaction.rs`: `hit_test_at`,
  `set_pane_pos`, `viewport_mut_for`, `apply_hovered_pane_row`
  (plus internal `clear_all_hover` helper).
- 4 methods deleted from `impl Panes` in `tui/panes/system.rs`.
- 3 accessors added to `Panes` (`lang()`, `output_mut()`,
  `hovered_row()`) so the free fns can reach the panes they need.
- 5 call-site rewrites: `interaction.rs` (3 internal), `input.rs:280`,
  `app/mod.rs:783` (the `apply_hovered_pane_row` shim now delegates
  to the free fn).

**Lessons:**
1. **The "obvious" first move pattern works.** All four dispatch
   methods relocated cleanly because every per-arm RHS still
   compiles — the relocation is pure code-motion. No semantic
   change, no behavior change, no borrow-checker friction.
2. **Clippy's `missing_const_for_fn` fires on simple free fns.**
   Three of the new free fns needed `const fn` to satisfy clippy
   (the same lint that hit Phase 8 lessons). Run clippy before
   declaring a relocation done; the warnings are easy fixes but
   easy to miss.
3. **`apply_hovered_pane_row`'s split into clear-then-set is the
   right shape for absorption.** The existing single method's
   `clear_all_hover` half iterates every viewport — when subsystems
   start owning their viewports, the clear loop's per-arm RHS
   swaps subsystem-by-subsystem. Splitting now means each Phase
   14–17 commit only touches the relevant arm.

**File-level changes:**
- *Modified:* `src/tui/interaction.rs` (added 4 free fns + 1
  helper, ~75 LOC), `src/tui/panes/system.rs` (deleted 4 methods,
  added 3 accessors, ~80 LOC removed net), `src/tui/input.rs`
  (rewrote 1 caller), `src/tui/app/mod.rs` (rewrote
  `apply_hovered_pane_row` shim).

**Status: done.**

## Phase 14 — `ToastsPane` → `ToastManager` absorption

First wrapper absorption. Smallest blast radius:

- Move `viewport: Viewport` and `hits: Vec<ToastHitbox>` from
  `ToastsPane` (`tui/panes/pane_impls.rs`) into `ToastManager`
  (`crate::tui::toasts`).
- Add `ToastManager::viewport()`, `viewport_mut()`, `set_hits()`
  accessors.
- Implement `Pane` and `Hittable` for `ToastManager`.
- Delete `ToastsPane` from `pane_impls.rs`; remove the field from
  `Panes`.
- Update Phase 13's relocated `set_pane_pos` etc. to route
  `PaneId::Toasts` arms through `app.toasts_mut().viewport_mut()`.
- Make `App::toasts_mut()` non-test (currently `#[cfg(test)]`).
- ~26 call-site rewrites: `app.panes_mut().toasts_mut().viewport_mut()`
  → `app.toasts_mut().viewport_mut()`.

**Risk:** medium — touches the click/hover/render paths for toasts.

**Phase 18 had a hard dependency on this specific absorption** —
`apply_service_signal`'s `set_len(toast_len)` write only
disappeared once the toasts viewport lived on `ToastManager`.
(Verified satisfied at the time Phase 18 landed.)

### Phase 14 retrospective

**Outcome:** clean absorption. `ToastsPane` is gone. `ToastManager`
now owns its own `viewport: Viewport` and `hits: Vec<ToastHitbox>`,
and impls `Pane` and `Hittable` directly. The Phase 13 dispatch
relocation paid off — only one arm in each free fn (`set_pane_pos`,
`viewport_mut_for`, `clear_all_hover`, `hit_test_at`) needed to
swap from `app.panes_mut().toasts_mut()` to `app.toasts_mut()`.

**Numbers:**
- Tests: 597 / 597 pass.
- 11 mend import-tidy fixes (auto-applied).
- ~30 call-site rewrites: `app.panes_mut().toasts_mut().viewport_mut()`
  → `app.toasts_mut().viewport_mut()`.
- Removed: `ToastsPane` struct + impls (`pane_impls.rs`),
  `Panes::toasts` field + accessors (`system.rs`).
- `App::toasts_mut()` widened from `#[cfg(test)]` to
  `pub(super)` (production callers now reach the toasts viewport
  through it).

**Lessons:**
1. **Borrow-checker friction at the dispatch arms.** Building
   `viewport_mut_for` originally tied `let panes = app.panes_mut();`
   for ten arms and then needed `app.toasts_mut()` for the Toasts
   arm — second mutable borrow conflict. Fix: drop the local
   binding and have each arm take its own `app.panes_mut()` or
   `app.toasts_mut()` directly. The compiler proves the borrows
   don't overlap (each arm is a single expression). **Apply to
   Phases 15-17:** the same shape works — every match arm reaches
   App freshly, no shared local binding.
2. **Items-after-test-module.** Initial impls landed *after*
   `mod tests` at the bottom of `manager.rs`, which clippy
   rejected (`items_after_test_module`). Fix: move the impls
   before the test module. **Apply:** when adding trait impls to
   subsystem files that already have a test module, place the
   new impls above the test mod.
3. **Multi-line perl substitution still required.** The bulk
   rewrite needed both single-line and `-0pe` multi-line patterns
   to catch chained `\n.panes_mut()\n.toasts_mut()\n.viewport_mut()`
   call sites. The Phase 3 lesson recurred verbatim.

**File-level changes:**
- *Modified:* `src/tui/toasts/manager.rs` (added `viewport`/`hits`
  fields, accessors, `Pane`/`Hittable` impls), `src/tui/app/mod.rs`
  (widened `toasts_mut`), `src/tui/interaction.rs` (Toasts arms in
  4 dispatch fns now route through `app.toasts*()`),
  `src/tui/panes/pane_impls.rs` (removed `ToastsPane`),
  `src/tui/panes/system.rs` (removed `toasts` field +
  accessors), ~30 call-site rewrites across `tui/`.

**Status: done.**

## Phase 15 — `CiPane` → `Ci` absorption

**Land Phase 15 + Phase 16 as one commit.** Both modify the same
`Panes::set_detail_data` orchestrator's signature; splitting leaves
an awkward intermediate where `set_detail_data` takes `&mut Ci` but
not `&mut Lint`.

- Move `viewport: Viewport` + content from `CiPane` (`pane_impls.rs:350-396`)
  into `Ci` (`crate::tui::ci_state`).
- Add `Ci::viewport()`, `viewport_mut()`, `set_content()` accessors.
- Implement `Pane` and `Hittable` for `Ci`. Render reads only
  `pane.content()` and `pane.viewport()` (`panes/ci.rs:267-272`) —
  same as Phase 14's pattern; `Ci`'s impl reads its own content,
  no `PaneRenderCtx` extension required.
- Delete `CiPane` from `pane_impls.rs`; remove the field from `Panes`.
- Update Phase 13's `set_pane_pos` / `viewport_mut_for` /
  `clear_all_hover` / `hit_test_at` `PaneId::CiRuns` /
  `HittableId::CiRuns` arms to route through `app.ci_mut()` /
  `app.ci()` (matching the Phase 14 toasts swap).

**`Panes::set_detail_data` design (post-Phase-14 review correction):**
the earlier draft said the signature would gain `&mut Ci`. Better
approach: **drop the `ci` and `lints` parameters from
`set_detail_data` entirely** and have the only production caller
(`navigation/cache.rs:55-80`) write to subsystems directly:

```rust
let ci = build_ci_data(self);
let lints = build_lints_data(self);
// ... gather other detail data ...
self.ci_mut().set_content(ci);
self.lint_mut().set_content(lints);
self.panes_mut().set_detail_data(key, package, git, targets);
```

Each statement freshly borrows `app` so no borrow-checker conflict
(Phase 14 lesson 1). This re-uses the absorption pattern instead of
re-enacting the orchestrator through `&mut` arguments.

**Risk:** medium — `set_detail_data` signature is the load-bearing
piece. Tests at `system.rs:399,428` need updating.

## Phase 16 — `LintsPane` → `Lint` absorption

Same pattern as Phase 15. **Land in the same commit as Phase 15**
so `set_detail_data` loses both `ci` and `lints` parameters
together.

- Move `viewport: Viewport` + content from `LintsPane`
  (`pane_impls.rs:305-343`) into `Lint`.
- Add `Lint::viewport()`, `viewport_mut()`, `set_content()` accessors.
- Implement `Pane` and `Hittable` for `Lint`. Render reads only
  `pane.content()` and `pane.viewport()` (`panes/lints.rs:135-156`).
- Delete `LintsPane`; remove the field from `Panes`.
- Update Phase 13 dispatch arms for `PaneId::Lints` /
  `HittableId::Lints`.

**Risk:** medium — mirrors Phase 15; same `set_detail_data` change.

### Phase 15 + 16 retrospective (bundled)

**Outcome:** `CiPane` and `LintsPane` are gone. `Ci` (`crate::tui::ci_state`)
and `Lint` (`crate::tui::lint_state`) each own their own
`viewport: Viewport` and `Option<CiData>` / `Option<LintsData>`
content slot, and impl `Pane` + `Hittable` directly. Phase 13's
dispatch free fns route `PaneId::Lints` / `PaneId::CiRuns` /
`HittableId::Lints` / `HittableId::CiRuns` through
`app.ci()` / `app.ci_mut()` / `app.lint()` / `app.lint_mut()`.
`Panes::set_detail_data` and `Panes::clear_detail_data` shed the
`ci` and `lints` parameters; the only production caller
(`navigation/cache.rs`) now writes to the subsystems directly with
fresh `app` borrows.

**Numbers:**
- Tests: 597 / 597 pass.
- 41 mend import-tidy fixes (auto-applied).
- Removed: `CiPane` + `LintsPane` structs and impls
  (`pane_impls.rs`), `Panes::ci_runs` and `Panes::lints` fields +
  accessors (`system.rs`), `Panes::dispatch_ci_render` and
  `dispatch_lints_render` methods, `Panes::override_lints_for_test`
  and `override_ci_runs_for_test`, the dead
  `render_lints_pane_for_test` test helper.
- Render path: a new `App::split_ci_for_render` and
  `App::split_lint_for_render` give the render dispatcher its own
  split-borrow tuple `(&mut Ci/Lint, &Config, &Scan)` since CI and
  Lint content no longer live on `Panes`. `render.rs` calls
  `panes::render_ci_pane_body` and `render_lints_pane_body`
  directly (not through `dispatch_via_trait`).
- `set_detail_data` signature went from 6 args to 4
  (`(stamp, package, git, targets)`), and its tests in
  `panes/system.rs` lost the now-irrelevant CI/Lints assertions.

**Lessons:**
1. **Subsystems that ship their own render/hit dispatch need their
   own split-borrow.** Phase 14's toasts absorption fit through
   `app.toasts_mut()` calls inside the Phase-13 free fns and the
   existing `dispatch_via_trait` plumbing. CI/Lints don't: their
   render bodies (`render_ci_pane_body`, `render_lints_pane_body`)
   want `&mut Ci` / `&mut Lint` plus `&Config` + `&Scan` for
   `PaneRenderCtx`, and `Ci` / `Lint` are not on `Panes`. Solution:
   add `split_ci_for_render` / `split_lint_for_render` accessors
   on `App` and call the render bodies directly from
   `render.rs`'s tile match arm. **Apply to Phase 17:** Keymap +
   Settings + Finder absorption into `Overlays` will need its own
   split-borrow if the overlay rendering touches `Config` / `Scan`.
2. **Drop orchestrator parameters instead of re-threading them.**
   The earlier draft of Phase 15 said `set_detail_data` would gain
   `&mut Ci` / `&mut Lint` parameters. The post-Phase-14 review
   caught that this re-creates the orchestrator with new types. The
   correct pattern (per Phase 14 lesson 1): drop the `ci`/`lints`
   parameters entirely; the lone caller writes to each subsystem
   with a fresh `app.ci_mut()` / `app.lint_mut()` borrow before the
   `app.panes_mut().set_detail_data(...)` call. The compiler proves
   the borrows don't overlap because each statement is independent.
3. **Bundling phases is correct when they share a load-bearing
   signature.** Splitting Phase 15 from Phase 16 would have left
   `set_detail_data` taking `(stamp, package, git, targets, lints)`
   for one commit — an awkward intermediate that exists only to be
   deleted. One commit, both absorptions, signature shrinks once.
4. **Test helpers absorb too.** `render_lints_pane_for_test` was a
   leftover scaffold from Phase 12; once `Lint` impls `Pane`
   directly, the helper had no callers and was deleted rather than
   re-typed. **Apply:** when a phase removes the wrapper a test
   helper was scaffolded around, check whether the helper is still
   reachable rather than mechanically updating its signature.
5. **`override_runs_for_test` belongs on the subsystem, not on
   `Panes`.** Test-only writes that previously routed through
   `Panes::override_ci_runs_for_test` now sit on `Ci` directly
   (`app.ci_mut().override_runs_for_test(...)`). The `Panes`
   wrapper version is gone. Tests update accordingly. **Apply:**
   when an absorption removes a wrapper, check `#[cfg(test)]`
   helpers on the wrapper too — they belong on the new owner.

**File-level changes:**
- *Modified:* `src/tui/ci_state.rs` (added `viewport`/`content`
  fields, accessors, `Pane`/`Hittable` impls,
  `override_runs_for_test`), `src/tui/lint_state.rs` (same),
  `src/tui/app/mod.rs` (added `lint_mut`, `split_ci_for_render`,
  `split_lint_for_render`; updated `is_satisfied` to read CI/Lints
  through subsystems), `src/tui/render.rs` (new
  `render_lints_pane`/`render_ci_pane` helpers replace
  `dispatch_via_trait` for these two panes),
  `src/tui/interaction.rs` (Lints/CiRuns arms in dispatch fns +
  test render helpers route through subsystems),
  `src/tui/app/navigation/cache.rs` (caller writes Ci/Lint
  directly, narrows `set_detail_data` call),
  `src/tui/app/async_tasks/repo_handlers.rs` (clears Ci/Lint
  alongside `clear_detail_data`),
  `src/tui/panes/system.rs` (removed `ci_runs`/`lints` fields +
  accessors + dispatch + override fns; `set_detail_data` /
  `clear_detail_data` narrowed; tests updated),
  `src/tui/panes/pane_impls.rs` (removed `CiPane`/`LintsPane`),
  `src/tui/panes/ci.rs`, `src/tui/panes/lints.rs` (render bodies
  take `&mut Ci` / `&mut Lint`; dead test helper removed),
  `src/tui/panes/actions.rs`, `src/tui/panes/mod.rs` (re-exports).

**Status: done.**

## Phase 17 — Keymap + Settings + Finder → `Overlays` absorption

Three viewports land in `Overlays` (`crate::tui::overlays`) together
— single commit since they all move to the same destination.

**File-layout precondition (post-Phase-15+16 review correction).**
Today `Overlays` is a single flat struct in one file
(`src/tui/overlays.rs`) holding mode-state (`FinderMode`,
`SettingsMode`, `KeymapMode`). Adding three `Viewport` fields plus
`line_targets: Vec<Option<usize>>` plus six accessors plus three
`Pane`/`Hittable` impl pairs to that file mixes mode-state with
render-side viewport plumbing and roughly doubles its size.
Phase 14's `ToastManager` precedent the absorption pattern keeps
citing lives in a separate `tui/toasts/` directory. Before Phase 17
starts, **promote `overlays.rs` to `overlays/mod.rs`** with
sibling files:
- `overlays/mod.rs` — `Overlays` struct + mode-state methods
  (current contents).
- `overlays/render_state.rs` — the three `Viewport` fields,
  `line_targets`, and the viewport accessors.
- `overlays/pane_impls.rs` — the three `Pane` + `Hittable` impl
  pairs.

This mirrors `tui/toasts/{mod.rs, manager.rs, ...}`. Don't bolt the
new state onto `overlays.rs` directly.

- Move `viewport: Viewport` from each of `KeymapPane`,
  `SettingsPane`, `FinderPane` into `Overlays` as **three
  independent fields** (`keymap_viewport`, `settings_viewport`,
  `finder_viewport`). A single shared overlay viewport would
  conflate cursors when overlays are reopened — they have
  independent cursor state.
- **`SettingsPane.line_targets: Vec<Option<usize>>`** must move
  alongside (`pane_impls.rs:442`). This is **ephemeral per-frame
  layout state** — render writes it, hit-test reads it, sibling of
  `viewport`. Same pattern as `CpuPane.row_rects` and
  `GitPane.row_layout`. Put it on `Overlays` next to
  `settings_viewport` (in `overlays/render_state.rs`); call it out
  as ephemeral layout state, not "settings state."
- Add `Overlays::keymap_viewport()`, `keymap_viewport_mut()`, and
  parallel pairs for settings + finder. Stay `pub(crate)` —
  `Overlays` lives at `crate::tui::overlays` (top-level under
  `tui/`), so the `pub(crate)`-allowed-at-top-level rule applies.
- Implement `Pane` and `Hittable` three times for `Overlays` — no
  way around the per-`PaneId` dispatch needing per-pane handlers.
  Render impls are no-ops (overlay path in `keymap_ui.rs`,
  `settings.rs`, `finder.rs` does the work) — same shape as
  ToastManager's Phase 14 impl.
- Delete `KeymapPane`, `SettingsPane`, `FinderPane`; remove the
  three fields from `Panes`.
- Update Phase 13 dispatch arms for the three.

**Split-borrow accessor (per Phase 15+16 lesson 1).** Settings
render touches `Config` (`src/tui/settings.rs:787` reads
`app.panes().settings().viewport().pos()`, and the popup body reads
config fields per row). Once `Overlays` owns the viewport, the
render path needs `(&mut Overlays, &Config, &Scan)`. **Add
`App::split_overlays_for_render(&mut self) -> (&mut Overlays, &Config, &Scan)`**
parallel to `split_ci_for_render` / `split_lint_for_render`, and
call the settings/keymap/finder render bodies through it directly
rather than through `dispatch_via_trait`. Same recipe Phase 15+16
followed; do not skip this step.

**Risk:** medium — three panes in one commit means larger blast
radius, but identical pattern to Phase 14 × 3.

**Tests-still-green gate** (absorbed from former Phase 18): verify
that all `build_*_data` callers compile and the `cache.rs` rebuild
path uses the fresh-borrow pattern (`app.ci_mut().set_content(ci);
app.lint_mut().set_content(lints); app.panes_mut().set_detail_data(...)`).
Phase 15/16 already shipped this destination-borrow change; this
gate just confirms Phase 17's overlay absorption hasn't regressed
it.

### Phase 17 retrospective

**Outcome:** clean three-wrapper absorption. `KeymapPane`,
`SettingsPane`, `FinderPane` are gone from
`tui/panes/pane_impls.rs`; `Overlays` (now at
`tui/overlays/{mod,render_state,pane_impls}.rs`) owns the three
viewports + `SettingsPane.line_targets`, exposes them via
`keymap_pane()` / `keymap_pane_mut()` (and parallels for settings
+ finder), and the new sibling files carry the pane-render-state
types and their `Pane`/`Hittable` impls. `Panes` lost its three
fields and six accessors. The dispatch arms in `interaction.rs`
(`hit_test_at`, `viewport_mut_for`, `clear_all_hover`) route
`PaneId::Keymap` / `PaneId::Settings` / `PaneId::Finder` through
`app.overlays()` / `app.overlays_mut()`. `App::split_overlays_for_render`
turned out not to be necessary — the keymap/settings/finder
renderers all take `&mut App` (or `&App` for keymap) directly,
not the trait-dispatch path Phase 15+16 had to thread through, so
no borrow split was forced.

**Numbers:**
- Tests: 597 / 597 pass.
- 15 mend import-tidy fixes (auto-applied).
- ~70 call-site rewrites across 8 files
  (`interaction.rs`, `panes/system.rs`, `panes/pane_impls.rs`,
  `finder.rs`, `settings.rs`, `keymap_ui.rs`, `input.rs`,
  `app/mod.rs`).
- File-layout precondition done: `overlays.rs` (146 lines)
  promoted to `overlays/{mod.rs, render_state.rs, pane_impls.rs}`
  (3 files, ~280 lines total) — mode state, render state, and
  trait impls cleanly separated.
- Removed: `KeymapPane` / `SettingsPane` / `FinderPane` structs +
  impls from `panes/pane_impls.rs`, `Panes::keymap` / `settings`
  / `finder` fields + accessors from `panes/system.rs`.

**Lessons:**
1. **Top-level `pub(crate)` only when the module IS top-level.**
   `tui/overlays/render_state.rs` is a sibling under
   `tui/overlays/`, not a top-level module under `tui/`.
   Declaring `pub(crate)` items there fails mend's policy. Fix:
   types and methods inside nested submodules use plain `pub`,
   and the top-level `mod.rs` widens via `pub(crate) use` if
   crate-wide visibility is needed. **Apply to Phase 20+:** any
   bus types that end up in `tui/app/bus.rs` (deeper than
   `tui/`) face the same constraint — declare `pub(super)` and
   re-export upward, never `pub(crate)` from inside `tui/app/`.
2. **Three Hittable impls require three concrete types.** "Impl
   `Pane` and `Hittable` three times for `Overlays`" can't be
   read literally — Rust forbids three `impl Hittable for X`
   blocks for the same type. The right shape is three small
   types (`KeymapPane`, `SettingsPane`, `FinderPane`) that each
   impl `Hittable` once, owned as fields by `Overlays`. The
   `HITTABLE_Z_ORDER` dispatch then routes each `HittableId` to
   the matching field via `app.overlays().keymap_pane()` /
   `settings_pane()` / `finder_pane()`. Type names can be reused
   from the deleted wrappers — what changed is who owns them and
   where they live.
3. **`pub use` re-exports flow visibility upward.** Three small
   types declared `pub` in `tui/overlays/render_state.rs` are
   visible only inside `tui/overlays/`. The
   `pub(crate) use render_state::{FinderPane, KeymapPane, SettingsPane};`
   in `mod.rs` widens them to crate-wide reach without putting
   `pub(crate)` on the type definitions themselves. Same trick
   for the methods: their callable visibility at external sites
   is determined by the type's re-export path, not the method's
   declared modifier.
4. **The split-borrow accessor isn't always needed.** Plan
   called for `App::split_overlays_for_render(&mut self) -> (&mut Overlays, &Config, &Scan)`
   parallel to Phase 15+16's `split_ci_for_render`. Reality: the
   keymap/settings/finder renderers (`render_keymap_popup`,
   `render_settings_popup`, `render_finder_popup`) take
   `&mut App` directly from `render::ui` and don't go through
   the `Pane` trait's dispatch path that forced Phase 15+16's
   split. After absorption, callers reach
   `app.overlays_mut().settings_pane_mut().viewport_mut()` with
   no borrow conflict — `&mut App` already gives access to all
   subsystems in turn. **Apply:** the split-borrow accessor is
   needed only when the render body takes typed parameters
   (which forces a split). For renderers already taking `&mut
   App`, the absorption is a pure call-site rewrite.
5. **Multi-line perl substitutions catch the rest.** The bulk
   rewrite needed both single-line patterns
   (`s/app\.panes_mut\(\)\.keymap_mut\(\)/app.overlays_mut().keymap_pane_mut()/g`)
   AND `-0pe` multi-line patterns to catch chained
   `app\n.panes_mut()\n.keymap_mut()\n.viewport_mut()` calls.
   The Phase 14 / Phase 15+16 lesson recurred verbatim.

**File-level changes:**
- *New:* `src/tui/overlays/mod.rs` (existing `overlays.rs` content
  + 3 new fields + 6 accessor methods), `src/tui/overlays/render_state.rs`
  (`KeymapPane` / `SettingsPane` / `FinderPane` definitions + their
  inherent methods), `src/tui/overlays/pane_impls.rs` (`Pane` +
  `Hittable` impls for the three).
- *Deleted:* `src/tui/overlays.rs`, `KeymapPane` / `SettingsPane` /
  `FinderPane` from `src/tui/panes/pane_impls.rs`, `keymap` /
  `settings` / `finder` fields and their six accessors from
  `src/tui/panes/system.rs`.
- *Modified:* `src/tui/interaction.rs` (3 dispatch arms in
  `hit_test_at`, 3 in `viewport_mut_for`, 3 in `clear_all_hover`
  — the last using fresh `app.overlays_mut()` borrows per the
  Phase 14 lesson), `src/tui/keymap_ui.rs`, `src/tui/settings.rs`,
  `src/tui/finder.rs`, `src/tui/input.rs`, `src/tui/app/mod.rs`
  (~70 call-site rewrites total).

**Status: done.**

## Phase 18 — Bus skeleton + `apply_service_signal` smoke test — **DONE** *(reverted in Phase 20)*

Smoke-test phase: introduced the bus types and routed a single
event end-to-end to confirm the drain-loop pattern compiled and
behaved identically to the prior direct-call version.

**Note: superseded by Phase 20.** The architectural review
post-Phase-19 found the bus pattern doesn't fit this codebase
(see Phase 20 for rationale). This retrospective documents what
was built; Phase 20 reverts it. Lessons captured below remain
useful for direct-method orchestrators that take their place.

**What landed (since reverted):**
- `src/tui/app/bus.rs` (new) defined `Event`, `Command`,
  `EventBus`, `EventHandler`, `HandlerCtx`. The trait and ctx
  types shipped as `#[allow(dead_code)]` scaffolding for a
  future expansion that the architectural review deemed
  unjustified.
- `App` gets a `bus: EventBus` field.
- `apply_service_signal(signal)` becomes
  `self.bus.publish(Event::ServiceSignal(signal)); self.drain_bus();`.
- The retry path goes through `Command::SpawnServiceRetry(ServiceKind)`
  so the handler doesn't need `&mut Background`.
- `drain_bus`, `deliver_event`, `execute_command` live next to
  the methods they dispatch to (in `service_handlers.rs`), not
  in `bus.rs` — bus types stay alone in their file.

**Stay-on-App items NOT migrating to the bus:** `input_context`,
`is_pane_tabbable`, `tabbable_panes`, `focus_next_pane`,
`focus_previous_pane`, `reset_project_panes`,
`force_settings_if_unconfigured`. All are synchronous reads or
single-shot direct writes triggered by a known caller. The bus
exists to fan out an event across a subscriber set the emitter
doesn't know — that pattern doesn't apply to keystroke-driven
read paths or post-bootstrap checks.

### Phase 18 retrospective

**Outcome:** clean smoke test. `EventBus`, `Event`, `Command`,
`HandlerCtx`, `EventHandler` live in `src/tui/app/bus.rs` (new);
the `bus: EventBus` field sits on `App` next to the other
subsystems; `apply_service_signal` now publishes
`Event::ServiceSignal(signal)` and immediately calls
`drain_bus`, which alternates events→commands until both queues
are empty. The original signal-routing body moved to
`handle_service_signal_event` (still on `App`, called via
`deliver_event`'s match arm). The retry path now goes through
`Command::SpawnServiceRetry(service)` instead of calling
`spawn_service_retry` inline, which means the retry dispatch
runs in the command-drain phase after the toast push — a
trivial reorder relative to the prior sequence (toast push
first, then OS thread spawn) with no observable effect.

**Numbers:**
- Tests: 597 / 597 pass.
- 1 mend import-tidy fix (auto-applied: `use bus::EventBus;`).
- 1 new file (`src/tui/app/bus.rs`, ~80 lines).
- 3 modified files (`src/tui/app/mod.rs` — `mod bus;` +
  field + import; `src/tui/app/construct.rs` — field init;
  `src/tui/app/async_tasks/service_handlers.rs` —
  publish/drain/deliver/execute, plus the renamed
  `handle_service_signal_event` body).

**Lessons:**
1. **`pub(super)` on App methods doesn't reach sibling submodules
   of `app`.** The first attempt put `drain_bus`, `deliver_event`,
   and `execute_command` as an `impl App` block inside
   `app/bus.rs`. They needed to call
   `handle_service_signal_event` and `spawn_service_retry`,
   which are `pub(super)` from inside
   `app/async_tasks/service_handlers.rs` — so visible to
   `async_tasks` only, not to `app/bus.rs`. Two paths fixed this:
   widen those methods to `pub(crate)`, or move the dispatch impl
   into `service_handlers.rs` next to its callees. The second was
   cleaner (no widened visibility on a private method) and is the
   pattern to repeat in Phase 20 — keep the bus *types* in
   `bus.rs`, but put `impl App` dispatch arms in the same file as
   the handler bodies they call. **Apply to Phase 20:** the bus
   types stay alone in `bus.rs`; the per-event reactor lives next
   to the methods it calls (e.g. `apply_lint_config_change`'s
   reactor in `async_tasks/lint_config.rs` or wherever the
   handler body sits).
2. **Clippy forces `Copy` derives + by-value passing for small
   enums.** `Event` and `Command` started without `derive(Copy,
   Clone, Debug)`. Clippy's `needless_pass_by_value` and
   `trivially_copy_pass_by_ref` then complained about both
   `cmd: Command` (pass-by-value but matched, not consumed) and
   `ev: &Event` (borrow of a 2-byte type below the 8-byte limit).
   Adding the derives plus passing both by value (Event in
   `deliver_event` and `EventHandler::handle`) cleared both
   lints. **Apply to Phase 20:** when adding new `Event` or
   `Command` variants, derive `Copy`/`Clone`/`Debug` on the enums
   from the start; Phase 20's variants will likely carry owned
   data (`CargoPortConfig` in `ConfigChanged`) and won't be `Copy`, at which point pass
   by reference is correct again — the derive is the right
   default, the by-value/ref decision follows the variant data.
3. **`&mut self` on commands that don't need it loses to clippy.**
   `execute_command` initially took `&mut self` for symmetry with
   `deliver_event`, but the only Phase-18 variant
   (`SpawnServiceRetry`) calls `spawn_service_retry` which is
   `&self`. Clippy's `needless_pass_by_ref_mut` rejected the
   unused mut. Demoted to `&self`; Phase 20 will promote back
   when the first command needs `&mut self`. **Apply to Phase
   21:** start each new dispatch fn at the minimum mut-level the
   current variants require, not the level a future variant
   might need.
4. **Drain alternates events ↔ commands until both queues are
   empty.** A single forward pass would miss events that commands
   publish (and vice versa). The drain loop is therefore:
   ```
   loop {
       if let Some(ev) = self.bus.pop_event() {
           self.deliver_event(ev);
           continue;
       }
       if let Some(cmd) = self.bus.pop_command() {
           self.execute_command(cmd);
           continue;
       }
       break;
   }
   ```
   In Phase 18 no command publishes an event and no event handler
   publishes a follow-up event, so the loop terminates after a
   single pass each — but the alternation is what protects Phase
   21+ where a `Command::SpawnRescan` may publish a
   `RescanStarted` event downstream.
5. **`HandlerCtx` and `EventHandler` weren't needed for the smoke
   test.** Subscribers in Phase 18 take `&mut self` (App), reach
   the bus via `self.bus.dispatch(...)` directly, and don't need
   the typed-borrow split that `HandlerCtx` provides. The plan
   called this out (Phase 17 lesson 4) and the result confirms it:
   `HandlerCtx` and `EventHandler` are scaffolding marked
   `#[allow(dead_code, reason = "Phase 20 scaffolding")]` —
   they stay until Phase 20 wires them up. Phase 20's
   six-subscriber fan-out is the upgrade trigger. **Apply to
   Phase 20:** if `apply_lint_config_change`'s reactor compiles
   while taking `&mut App`, the typed `HandlerCtx` stays unused;
   it's only needed when Rust's borrow checker rejects the naive
   `&mut App` shape (e.g. concurrent `&mut Lint` + `&mut Scan` +
   `&mut Selection` + `&mut Overlays` + `&mut ToastManager`
   borrows on a single event).

**File-level changes:**
- *New:* `src/tui/app/bus.rs` (`EventBus`, `Event`, `Command`,
  `HandlerCtx`, `EventHandler` — types only, no `impl App`
  block).
- *Modified:* `src/tui/app/mod.rs` (added `mod bus;` and
  `bus: EventBus` field with import via `use bus::EventBus;`),
  `src/tui/app/construct.rs` (`bus: EventBus::new()` in the
  builder), `src/tui/app/async_tasks/service_handlers.rs`
  (`apply_service_signal` body replaced with publish+drain;
  added `drain_bus`, `deliver_event`, `execute_command`;
  renamed signal-routing body to `handle_service_signal_event`;
  routed retry through `Command::SpawnServiceRetry`).

**Status: done.**

## Phase 19 — Move `row_count` + cursor-movement methods onto `Selection` — **DONE**

Five methods relocated from `App` onto `Selection`:
- `row_count` — pure read of the cached visible-rows length.
- `move_up`, `move_down`, `move_to_top`, `move_to_bottom` —
  cursor-only mutations with no scan dependency.

Call sites at `dismiss.rs:108`, `interaction.rs:733`, and
`input.rs:265-609` updated to `app.selection_mut().move_down()`
etc. (chained form compiled cleanly; no split-borrow accessor
needed).

### Phase 19 retrospective

**Outcome:** clean relocation of the cursor-navigation surface.
`Selection` now owns `row_count` and the four directional
movement methods; `App::row_count` is gone, `movement.rs` keeps
only the `collapse_anchor_row` helper (used by `App::collapse_all`
which stayed on App). `selection.rs` gained ~30 lines (one
`row_count` + four `const fn move_*` definitions); App lost ~40
lines across `expand.rs` and `movement.rs`. All 599 tests pass,
clippy clean (after `const fn` upgrades), mend clean.

**Plan-vs-reality scope correction.** The plan listed 12 methods
under "movement methods": `move_up/down/top/bottom`, `expand`,
`collapse`, `expand_all`, `collapse_all`,
`select_project_in_tree`, `select_matching_visible_row`,
`expand_path_in_tree`, `try_collapse`. Reading the actual
bodies, only the four directional movers and `row_count` are
single-subsystem (cursor + visible_rows length, both already on
`Selection`). The other eight methods orchestrate Selection
mutations together with `ProjectList` reads, calls to
`ensure_visible_rows_cached` (which itself reads
`config.include_non_rust` and `scan.projects()`), and
inline-group helpers. Moving them to `Selection` would push
`projects: &ProjectList` and `include_non_rust: bool` parameters
through every method signature — pollution of `Selection`'s API
for no architectural win, because these methods *are* the
cross-subsystem orchestration the plan's "Cross-subsystem
orchestrator on App" pattern (see `mod.rs` § "Recurring
patterns") explicitly assigns to `App`. They stay on `App`.

**Lessons:**
1. **"Movement methods" was an over-broad category.** The plan
   conflated cursor navigation (single-subsystem,
   `Selection`-owned) with tree-shape mutations (cross-subsystem,
   App-orchestrator-owned). The former relocate cleanly; the
   latter don't, and shouldn't. Future phase planning: when
   listing methods to relocate, classify each by data ownership
   first (does the body touch one subsystem or many?), not by
   verb similarity (`move_up` and `expand` are both "navigation"
   but live at different architectural layers).
2. **`#[allow(dead_code)]` warnings during partial relocation.**
   Adding the four movement methods to `Selection` triggered
   "method is never used" warnings until the call-site rewrite
   landed. Order: add to destination, run `perl -i` to rewrite
   call sites, then delete from source — kept the build green
   the whole time. Clippy also forced `const fn` on the
   directional movers (they don't actually mutate anything that
   isn't a primitive). Standard Phase-15+16 lesson recurred.
3. **Call-site rewrite chain compiled without split-borrow
   accessor.** `app.selection_mut().move_down()` works because
   `selection_mut` returns `&mut Selection` and `move_down`
   needs only that — no other subsystem read involved. The plan
   reserved a fallback (`split_for_movement` accessor), unused.

**File-level changes:**
- *Modified:* `src/tui/selection.rs` (added `row_count` +
  `move_up` / `move_down` / `move_to_top` / `move_to_bottom`),
  `src/tui/app/navigation/expand.rs` (removed `App::row_count`),
  `src/tui/app/navigation/movement.rs` (removed four
  movement methods; `collapse_anchor_row` helper kept),
  `src/tui/app/dismiss.rs` (one `self.row_count()` →
  `self.selection.row_count()` rewrite),
  `src/tui/interaction.rs` (one call-site rewrite),
  `src/tui/input.rs` (eight call-site rewrites).

**Status: done.**

## Phase 20 — Rip out the event bus

**Goal.** Revert Phase 18's bus introduction. After Phase 20,
the codebase has no `EventBus`, `Event`, `Command`,
`EventHandler`, or `HandlerCtx` types. `apply_service_signal`
becomes a direct method call on `App` again; the
`service_handlers.rs` body returns to its pre-Phase-18 shape.

**Why this phase exists.** The architectural review after
Phase 19 found the bus pattern doesn't fit this codebase. The
cross-subsystem orchestrators on `App` (`apply_config`,
`apply_lint_config_change`, `apply_service_signal`, `rescan`,
etc.) are themselves the fan-out point. Replacing direct
method dispatch with publish-event + drain +
return-`Vec<Command>` + execute is more dispatch layers, same
logic. Two of the bus's five types (`HandlerCtx`,
`EventHandler`) shipped as dead-code scaffolding waiting for an
expansion that wasn't going to deliver architectural value
proportional to its cost. Removing the bus makes the codebase
self-consistent: one orchestrator pattern (named methods on
`App`) used everywhere.

This is also why Phase 18's "smoke test" wasn't a real
borrow-checker stress test. The `apply_service_signal` reactor
took `&mut self` on App rather than typed `&mut Subsystem`
parameters, so `HandlerCtx`'s typed-borrow surface never got
exercised. That made the bus skeleton load-bearing only for the
`Command::SpawnServiceRetry` deferral — and that deferral can
be replaced with a direct call inside `apply_unavailability`
since the pre-Phase-18 code already did exactly that.

**Files affected.**
- *Delete:* `src/tui/app/bus.rs`.
- *Modify `src/tui/app/mod.rs`:* remove `mod bus;`, remove the
  `bus: EventBus` field, remove `use bus::EventBus;`.
- *Modify `src/tui/app/construct.rs`:* remove
  `bus: EventBus::new()` from the App struct initializer.
- *Modify `src/tui/app/async_tasks/service_handlers.rs`:*
  - `apply_service_signal` body returns to a direct match on
    the `ServiceSignal` variants, calling
    `handle_service_reachable` / `apply_unavailability`
    directly (the pre-Phase-18 shape).
  - Delete `drain_bus`, `deliver_event`, `execute_command`, and
    `handle_service_signal_event` (the Phase 18 routing
    methods — `handle_service_signal_event`'s body folds back
    into `apply_service_signal`).
  - `apply_unavailability`'s retry-spawn path goes back to a
    direct `self.spawn_service_retry(service)` call, dropping
    the `Command::SpawnServiceRetry` indirection.
- *Remove imports of `Command` and `Event`* from
  `service_handlers.rs`.

**Pre-requisites:** none. Phase 20 has no source dependency on
Phase 21 or 22.

**Risk:** low. Phase 18's retrospective documented that no
command publishes an event and no event handler publishes a
follow-up event in the smoke-test surface — the drain loop
never exercised any non-trivial behavior. Reverting it returns
the code to a known-good prior shape (the test suite from
pre-Phase-18 verifies the revert; today's 599-test count drops
back to 597 if the two Phase-18-added bus-related tests
existed, otherwise stays at 599).

**Apply the cross-phase lessons in reverse:**
- The bulk-rewrite tooling (multi-line perl, etc.) used to
  introduce bus call sites in Phase 18 applies to removing
  them.

## Phase 21 — `TreeReaction` enum cleanup of `ReloadActions`

**Goal.** Replace the mutually-exclusive `rescan` /
`rebuild_tree` boolean fields in `ReloadActions`
(`src/tui/config_reload.rs`) with a single `TreeReaction` enum.
The type system then refuses combinations that today rely on
runtime branching to stay correct.

```rust
pub enum TreeReaction {
    None,            // no tree change
    RegroupMembers,  // in-place regroup_members + refresh_derived_state
    FullRescan,      // wholesale rescan + force_settings_if_unconfigured
}

pub struct ReloadActions {
    pub tree:                 TreeReaction,
    pub refresh_lint_runtime: bool,
    pub refresh_cpu:          bool,
    pub force_rate_limit:     Option<bool>,
}
```

**Producer.** `collect_reload_actions`
(`src/tui/config_reload.rs:130-184`) currently calls
`mark_rescan(...)` and `mark_rebuild_tree(...)` on independent
methods that each set one boolean. The enum collapse means
`collect_reload_actions` returns
`tree: TreeReaction::FullRescan` or `RegroupMembers` directly;
`mark_*` helpers either return the variant or are inlined.

**Consumer.** `apply_config`'s existing
`if actions.rescan.should_apply() { ... } else if
actions.rebuild_tree.should_apply() { ... }` collapses into
`match actions.tree { TreeReaction::FullRescan => ...,
TreeReaction::RegroupMembers => ..., TreeReaction::None => ... }`.

**Cross-field interactions to preserve.** `refresh_lint_runtime`
and `refresh_cpu` stay independent booleans. The non-rescan
branch in `apply_config:182-194` runs
`respawn_watcher_and_register_existing_projects` (gated on
`refresh_lint_runtime.should_apply()`) and `regroup_members`
(gated on `rebuild_tree.should_apply()`). The match arm for
`TreeReaction::RegroupMembers` must coexist with
`refresh_lint_runtime: true` — they are orthogonal in the new
type as they are in the old.

**`force_settings_if_unconfigured` stays a direct method call.**
Called from startup (`construct.rs:246`) and from
`apply_config`'s rescan branch (`config.rs:181`); both ask "if
no roots, open settings." Already correctly placed as an
orchestrator on App; the `TreeReaction::FullRescan` match arm
calls it directly.

**Pre-requisites:** none. Reorderable with Phase 20 and Phase 22.

**Risk:** low. Mechanical type tightening with an
exhaustively-checked match. Tests verify no behavior change.

## Phase 22 — Extract `StartupOrchestrator` (state-owning subsystem)

**Goal.** Move the startup-phase state machine into its own
subsystem at
`src/tui/app/async_tasks/startup_phase/orchestrator.rs`. The
phase-tracking state migrates with the methods — this is a real
subsystem extraction, not just a method relocation.

**Why state-owning, not method-only.** The architectural review
post-Phase-19 found that the existing `tracker.rs` does **not**
own the startup-phase state today. The state lives on
`self.scan.scan_state_mut().startup_phases` (the disk / git /
repo / metadata phase counters) and on `self.lint.phase` /
`self.lint.startup_phase` (the lint phase). Each
`maybe_complete_startup_*` method on App reads `self.scan` to
check the phase and writes `self.toasts` (via
`finish_task_toast`, `mark_tracked_item_completed`);
`maybe_complete_startup_lints` reads `self.lint.phase` while
siblings write `self.scan`. The phase-tracking fields aren't
scan data and aren't lint data — they're startup-coordination
data living in the wrong subsystems. Method-only relocation
would leave the same architectural debt the plan was designed
to address. State-owning extraction moves the data to its
correct owner.

**Steps.**

1. **Create `StartupOrchestrator`.** Define the struct at
   `tui/app/async_tasks/startup_phase/orchestrator.rs`. Move
   the phase-tracking fields off `scan_state.startup_phases`
   and `lint.phase` / `lint.startup_phase` into the
   orchestrator. Delete those fields from `scan_state.rs` and
   `lint_state.rs`.
2. **Add `App::startup` field + accessors.** `app.startup()` /
   `app.startup_mut()` follow the pattern established in
   Phases 8–10 for subsystem accessors.
3. **Move the six `maybe_complete_startup_*` methods.** Each
   becomes `&mut self` on the orchestrator, taking the
   cross-subsystem references it needs as typed parameters
   (`&Scan`, `&Lint`, `&mut ToastManager`). Where the method
   today reads `self.scan.scan_state().startup_phases.foo`, it
   now reads `self.<phase>` directly on the orchestrator.
4. **Update the per-tick caller.** App's per-tick poll loop
   calls `self.startup.advance(...)` once per tick (or
   continues calling `maybe_log_startup_phase_completions`
   which now delegates to the orchestrator).
5. **Update test access.** `tests/state.rs:1899-1931` calls
   `maybe_complete_startup_*` directly on `App` to drive
   startup phases. Rewrite to
   `app.startup_mut().maybe_complete_*(...)`.
6. **Update read sites of the migrated state.** Any place
   reading `scan_state.startup_phases` or `lint.phase` /
   `lint.startup_phase` now reads through `app.startup()`.

**Pre-requisites:** none. Reorderable with Phases 20 and 21.

**Per Phase 17 lesson 5:** the six `maybe_complete_startup_*`
methods will have chained call sites; bulk rewrites need both
single-line and multi-line perl substitutions to catch every
site.

**Visibility (Phase 12 lesson 1).** The orchestrator's location
is deeply nested under `tui/`. The project rule forbids
`pub(crate)` in nested `tui/` modules. Declare types
`pub(super)` and re-export upward at `tui/app/mod.rs` only as
needed. If broader visibility turns out to be required,
relocate the type to a top-level `crate::tui::*` module rather
than widening in place.

**Risk:** medium. State migration off `scan_state` and `lint`
is the load-bearing piece — those modules need their fields
removed and any read sites of the migrated state need to switch
to `app.startup()` calls. The methods themselves are mechanical
to relocate once the state is in place.

### Bus design (Phases 18 + 21)

The design is a **command pattern layered on top of pub/sub**.
Subscribers don't get cross-subsystem `&mut` references during
event handling; they return a list of `Command`s describing the
side-effects they want, and App applies the commands sequentially
after gathering them. This sidesteps all borrow-checker conflicts
that a traditional bus would hit.

**`Event` enum** (Phase 20's full surface):

```rust
#[derive(Clone, Debug)]
pub(super) enum Event {
    /// Service-availability signal — fan-out to Net + Toasts (Phase 18).
    ServiceSignal(ServiceSignal),                       // Copy

    /// Service-recovered signal — fired when force-rate-limit flips off
    /// or the retry probe succeeds.
    ServiceRecovered(ServiceKind),                      // Copy

    /// Lint config changed — six-subscriber fan-out (Phase 20).
    LintConfigChanged,                                  // Copy

    /// Full config changed. Carries the resolved `ReloadActions`
    /// (with `TreeReaction` enum) plus raw `prev`/`next` for
    /// fine-grained subscriber checks.
    ConfigChanged {
        prev:    CargoPortConfig,
        next:    CargoPortConfig,
        actions: ReloadActions,
    },                                                  // not Copy
}
```

`Event::ConfigChanged` carries owned `CargoPortConfig` values
(cloned once per save), so it isn't `Copy`. The other three are
`Copy`. Subscribers that need fine-grained config-field diffs read
`prev`/`next` directly; the high-level reactions are already
resolved into `actions`.

**`ReloadActions` (extended in Phase 20 step 1, lives in
`src/tui/config_reload.rs`).** The existing helper gets the
`TreeReaction` enum replacing the mutually-exclusive `rescan` /
`rebuild_tree` booleans:

```rust
pub enum TreeReaction {
    None,
    RegroupMembers,  // in-place regroup + refresh_derived_state
    FullRescan,      // wholesale rescan + force_settings_if_unconfigured
}

pub struct ReloadActions {
    pub tree:                 TreeReaction,
    pub refresh_lint_runtime: bool,
    pub refresh_cpu:          bool,
    pub force_rate_limit:     Option<bool>,
}
```

This makes the "rescan vs regroup" exclusivity a type-system
invariant rather than a runtime convention. Subscribers
pattern-match on the variant; the compiler refuses
combinations that today rely on `if/else` to stay correct.

There is no separate `ConfigDiff` helper — `ReloadActions`
already carries the high-level decisions, and subscribers reach
raw `prev`/`next` for the rare field-level read. One diff helper,
not two.

**`Command` enum** — derived from the actual side-effects needed
by Phase 18 + Phase 20's subscriber set. Final granularity decided
at implementation time.

```rust
#[derive(Clone, Debug)]
pub(super) enum Command {
    // Phase 18 (already implemented)
    SpawnServiceRetry(ServiceKind),

    // Phase 20 — service / network
    DismissToast(ToastTaskId),
    MarkServiceRecovered(ServiceKind),

    // Phase 20 — lint runtime
    RestartLintRuntime,
    RefreshLintFromDisk,
    BumpScanGeneration,

    // Phase 20 — selection / panes
    ResetFitWidths(bool),
    ResetCpuPlaceholder,

    // Phase 20 — tree mutation (App-owned because Scan can't subscribe)
    ApplyTreeReaction(TreeReaction),

    // Phase 20 — toasts (cross-subsystem effect from any subscriber)
    PushToast { title: String, body: String, style: ToastStyle },

    // Phase 20 — re-entrant publish
    PublishEvent(Event),
}
```

Phase 18 ships only `SpawnServiceRetry`. Phase 20 adds the rest.

**`EventHandler` trait** (concrete signature):

```rust
pub(super) trait EventHandler {
    /// Examine `event`. Mutate the subsystem's own private state
    /// in place via `&mut self`. Return commands for any
    /// cross-subsystem effects.
    ///
    /// Two-channel rule:
    /// - In-place `&mut self` mutation is permitted only for the
    ///   subsystem's own private state.
    /// - Cross-subsystem effects MUST return as `Command` variants.
    fn handle(&mut self, event: &Event, scan: &Scan) -> Vec<Command>;
}
```

`Event` is passed by reference because the `ConfigChanged` payload
isn't `Copy`. `Scan` is the only cross-cutting read passed in;
subscribers needing other shared reads emit a `Command` whose
`apply_command` arm holds the right borrows.

**`Scan` is not a subscriber.** Rust would reject
`self.scan.handle(&event, &self.scan)` as same-field aliasing.
Tree-mutation reactions go through `Command::ApplyTreeReaction`,
which App's `apply_command` arm handles directly with `&mut self.scan`
+ `&mut self.background` in scope.

**Module locations.** `Event`, `Command`, `EventBus`,
`EventHandler`, `HandlerCtx` live in `src/tui/app/bus.rs` (already
the case from Phase 18). `EventHandler` impls live next to each
subscriber's struct definition: `Lint` in `src/tui/lint_state.rs`,
`Net` in `src/tui/net_state.rs`, `Selection` in
`src/tui/selection.rs`, `ToastManager` in `src/tui/toasts/`,
`Overlays` in `src/tui/overlays/`.

**Visibility on bus types.** `bus.rs` lives under `tui/app/`, so
the nested-module `pub(crate)` ban applies. Declare types
`pub(super)` and re-export upward as needed. Phase 18 already
follows this — Phase 20 extends without changing the visibility
discipline.

**Drain loop on App** — Phase 18 already implemented this:

```rust
impl App {
    fn drain_bus(&mut self) {
        loop {
            if let Some(ev) = self.bus.pop_event() {
                self.deliver_event(ev);
                continue;
            }
            if let Some(cmd) = self.bus.pop_command() {
                self.execute_command(cmd);
                continue;
            }
            break;
        }
    }

    fn deliver_event(&mut self, ev: Event) {
        // Phase 20: explicit per-subscriber dispatch lives here.
        let cmds = match &ev {
            Event::ServiceSignal(s)    => self.handle_service_signal_event(*s),
            Event::ServiceRecovered(k) => self.handle_service_recovered_event(*k),
            Event::LintConfigChanged   => self.handle_lint_config_changed(),
            Event::ConfigChanged { .. } => self.handle_config_changed(&ev),
        };
        for cmd in cmds { self.bus.dispatch(cmd); }
    }

    fn execute_command(&mut self, cmd: Command) { /* match on Command */ }
}
```

Per Phase 18 lesson 1, `deliver_event` and `execute_command` live
next to the methods they dispatch to (in
`async_tasks/service_handlers.rs` for the Phase 18 surface, in
`async_tasks/config.rs` for the Phase 20 surface). Bus *types*
stay alone in `bus.rs`.

**Subscriber dispatch under App's borrow.** Each
`handle_*_event(&mut self, ...)` method on App takes a fresh
borrow of the subsystem field it mutates, calls subsystem-level
methods, and returns `Vec<Command>`. The App-level method *is*
the dispatch (per Phase 18 lesson 1) — there is no separate
"App walks subscribers" loop, because App's `&mut self` already
gives access to every subsystem in turn.

**Re-entrancy semantics.** A handler returning
`Command::PublishEvent(Foo)` enqueues `Foo` for a later iteration
of `drain_bus`. Because `drain_bus` alternates events↔commands,
the newly-published event is delivered after the current event's
commands all run.

**Termination invariant** (must hold per-subscriber, enforced by
review):

1. No subscriber publishes the event it is currently handling.
2. `Event::ConfigChanged` is published only from
   `App::apply_config`. Subscribers may not synthesize it from
   any other event.
3. Subscribers that emit `Command::PublishEvent(...)` must emit a
   downstream event in the dependency DAG, never an upstream one.
   The DAG: `ConfigChanged → {LintConfigChanged, ServiceSignal,
   ServiceRecovered}`. No back-edges.

**Debug-build cycle detection** (cheap insurance): the drain loop
maintains a counter, hard-asserts at e.g. 1000 iterations per
drain. Catches infinite loops in dev without slowing production.

**Borrow-checker check:** `self.bus.pop_event()` and
`self.bus.pop_command()` borrow `self.bus` temporarily; the borrow
is released before each subscriber dispatch. `deliver_event` and
`execute_command` take `&mut self` and re-borrow specific
subsystem fields disjointly. No two `&mut` to the same field at
the same time. **Compiles.**

**End-to-end traced flow — user saves config (Phase 20):**

1. Settings overlay closes; calls `app.apply_config(new_cfg)`.
2. `apply_config` (now thin):
   ```rust
   let prev = self.config.current().clone();
   let actions = collect_reload_actions(&prev, &new_cfg, ctx);
   *self.config.current_mut() = new_cfg.clone();
   self.bus.publish(Event::ConfigChanged { prev, next: new_cfg, actions });
   self.drain_bus();
   ```
3. `drain_bus` pops `ConfigChanged`. `deliver_event` calls
   `handle_config_changed(&ev)`, which dispatches to each
   subscriber and collects their `Vec<Command>`:
   - `Lint::handle(ConfigChanged)` → if `lint_enabled` flipped or
     `cargo_args` changed: `[RestartLintRuntime, RefreshLintFromDisk,
     BumpScanGeneration]`.
   - `Net::handle(ConfigChanged)` → if `force_rate_limit` flipped:
     `[PublishEvent(Event::ServiceSignal(...))]` or
     `[PublishEvent(Event::ServiceRecovered(...))]`.
   - `Selection::handle(ConfigChanged)` → if `lint_enabled`
     flipped: `[ResetFitWidths(next.lint.enabled)]`.
   - `Overlays::handle(LintConfigChanged)` → on warning:
     `[PushToast { ... }]`.
   - App-owned tree dispatch: `[ApplyTreeReaction(actions.tree)]`.
4. `execute_command` runs each command. `ApplyTreeReaction(FullRescan)`
   calls `self.rescan(); self.force_settings_if_unconfigured();`
   directly. `PublishEvent(ServiceSignal(...))` enqueues a follow-up
   event.
5. Drain alternates, picks up `ServiceSignal(...)`, runs the Phase
   18 handler chain.
6. Queue empties; `drain_bus` returns; `apply_config` returns.

`apply_config`'s body shrinks from ~50 lines to ~5 lines.
Per-subsystem fan-out logic moves into each subscriber's `handle()`
body.

**Status: ready to execute.** Phase 18 already proved the
skeleton. Open items at Phase 20 execution time:
- Confirm `clear_all_lint_state` and `refresh_lint_runs_from_disk`
  borrow patterns by inlining them.
- Pick whether `BackgroundMsg` dispatch (handle_bg_msg) becomes a
  bus event or stays direct (recommended: stays direct — it's
  pattern-match dispatch, not fan-out).

## Loose ends — items not slated for any phase

Tracking sheet for items the running architecture reviews flagged.
Updated post-Phase-8.

**Resolved in Phase 8:**
- ~~`mark_visited` migration~~ — done (now in `Focus`).

**Bundled into Phase 9 (post-Phase-8 review):**
- **`status_flash` field** — moves into `Overlays` alongside `inline_error`.
- **App-struct `panes:` doc comment drift** — one-line cleanup.

**Resolved by Phase 12 design depth:**
- **`ensure_detail_cached` cache home** — keep on `Panes`. Earlier
  proposal to extract `crate::tui::detail_cache` was reversed in
  post-Phase-8 review (render-machinery state, no payoff in moving).

**Still loose:**
- **`Inflight` API audit.** `inflight: Inflight` lives at
  `src/tui/inflight.rs` (outside `tui/app/`), so its accessors
  *should* be `pub(crate)`-free. Defer to a post-Phase-12 audit since
  Phase 12 will rewrite the inflight render-time read pattern anyway.
- **`Background` and `LayoutCache` accessors.** Both are App fields,
  not touched by any planned phase. Defer indefinitely or fold into a
  pre-13 audit pass if scope warrants.

## Phase summary

| Phase | Cluster | Status |
| ----- | ------- | ------ |
| 1   | Config (pub items only) | Done |
| 2   | Trivial subsystems (Keymap + Toasts + Scan/metadata) | Done |
| 3   | Git/Repo extract → ProjectList | Done |
| 4   | Ci (3 methods → ProjectList; 5 orchestrators relocated to `mod.rs`) + pulled-forward `app.config()` cleanup | Done |
| 5   | Toast orchestrator relocation | Done |
| 6   | Discovery shimmer + project predicates | Done |
| 7   | Internal-helper visibility narrowing pass | Done |
| 8   | Focus subsystem at `tui/focus.rs` | Done |
| 9   | Overlays subsystem at `tui/overlays/` (Phase 17 promoted from `overlays.rs` to a directory) | Done |
| 10  | `CiFetchTracker` relocation | Done |
| 11  | `Viewport.pos` → `Selection.cursor` (cursor field) | Done |
| 12  | Pane trait foundations (cursor-mirror cleanup + trait relocation) | Done |
| 13  | Relocate Panes' dispatch methods to App-level | Done |
| 14  | `ToastsPane` → `ToastManager` absorption | Done |
| 15  | `CiPane` → `Ci` absorption | Done |
| 16  | `LintsPane` → `Lint` absorption | Done |
| 17  | Keymap + Settings + Finder → `Overlays` absorption | Done |
| 18  | Event-bus skeleton + `apply_service_signal` smoke test | Done |
| 19  | Move `row_count` + 4 cursor-movement methods onto `Selection` | Done |
| 20  | Rip out the event bus (revert Phase 18) | Ready |
| 21  | `TreeReaction` enum cleanup of `ReloadActions` | Ready (reorderable) |
| 22  | Extract `StartupOrchestrator` (state-owning subsystem) | Ready (reorderable) |

After Phases 19–21, App's struct surface drops to roughly:
`new`, `run`, the bus-publishing entry points (`apply_config`,
`rescan`, `handle_bg_msg`, `apply_service_signal`,
`mark_service_recovered`), the keystroke-driven group flagged
in Phase 18 (`input_context`, `tabbable_panes`,
`focus_next_pane`, `force_settings_if_unconfigured`, etc.),
and a small set of multi-subsystem helpers
(`ensure_detail_cached`, `ci_for`, `sync_selected_project`).

The coupling direction shifts: subscribers pull the events they
care about; App pushes commands subscribers ask for.
`apply_command` becomes a flat dispatch table — the count of arms
tracks the side-effect surface, not the orchestrator surface.

## Post-Phase-9 architecture review

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
sum to ~57, which roughly matches the residual orchestrator surface
after the 9-phase extraction. This correspondence is bookkeeping —
the clusters were sized by sorting the residual into A/B/C — not
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

**Supersedes Phase 8's `NavRead { panes }` field.** Phase 8
proposed a `NavRead { selection, scan, panes }` borrow type because
`selected_row` had to read `panes().project_list().viewport().pos()`.
After Item 4, the cursor is on `Selection`, so `NavRead` no longer
needs `panes`. Rewrite Phase 8's signature when this item lands
before it (or reorder execution: Item 4 before Phase 8).

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
- Overlays (Phase 9 subsystem) absorb `KeymapPane`, `SettingsPane`,
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
    ConfigChanged { prev, next, actions: ReloadActions },
    LintConfigChanged,
}

trait EventHandler {
    fn handle(&mut self, ev: &Event, scan: &Scan) -> Vec<Command>;
}
```

(Phase 18 + 21 final design — see "Bus design" section above for
the full detail. The startup-phase tracker does not publish bus
events; see Phase 20.)

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
git-phase complete; ready-phase gates on all four prior.

**Where the sequencing layer lives** — see Phase 20 above.
A `StartupOrchestrator` at
`src/tui/app/async_tasks/startup_phase/orchestrator.rs` owns the
sequencing rules. It does **not** publish bus events — there are
no cross-subsystem subscribers, so the orchestrator advances its
own state machine directly via `advance(&mut self, ...)` and
calls into other subsystems (e.g. toasts) by direct method when
needed. App's per-tick poll loop calls
`self.startup_orchestrator.advance(...)`. This is a structural
relocation, not a bus consumer.

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

Phase 9 (Overlays) and Phase 8 (Focus) interact. If Phase 9 lands first with
clean boundaries, Phase 8's `tabbable_panes` migration takes a clean
`&Overlays` arg. If Phase 8 lands first, Focus has to read `App.ui_modes`
directly and Phase 9 retraces. Order is fixed: Phase 9 before Phase 8.

The Navigator change (originally one phase) is now split across Phase 8
(read accessors: `selected_*`, `path_for_row`, `display_path_for_row`,
`abs_path_for_row`, `row_count`, `visible_rows`, `expand_key_for_row`,
`selected_is_expandable`) and Phase 11 (mutators: `move_*`, `expand`,
`collapse_*`, `select_*`, `expand_path_in_tree`, `ensure_*_cached` — these
were the original Phase 8, absorbed into Phase 11 in the post-Phase-4
review because Phase 11's viewport-cursor relocation eliminates the
borrow problem that justified keeping them separate).
