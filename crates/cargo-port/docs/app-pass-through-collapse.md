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
Phase 9 (Overlays).

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
| 2 (merged 1b+2+4b) | **Done** | Trivial subsystems collapsed in one commit. App methods removed: `sync_keymap_stamp`, `current_keymap[_mut]`, `keymap_path`, `active_toasts`, `toasts_is_alive_for_test`, `confirm_verifying`, `clear_confirm_verifying_for`, `metadata_store_handle`, `target_dir_index_ref`, `resolve_metadata`, `resolve_target_dir`, `complete_ci_fetch_for`, `start_ci_fetch_for`, `replace_ci_data_for_path` (15 total). Accessors added on App (all `pub(super)` per Phase 1 lesson 1): `keymap()`, `keymap_mut()`, `toasts()`, `scan()`, `scan_mut()`, `ci_mut()`. Mend: **133 → 131** (-2 actual vs ~10–15 predicted; see Phase 2 lessons below). 597/597 tests pass. |
| 3 | **Done** | Git/Repo reads relocated to `ProjectList`. 9 methods moved (`git_info_for`, `repo_info_for`, `primary_url_for`, `primary_ahead_behind_for`, `fetch_url_for`, `git_status_for`, `git_status_for_item`, `git_sync`, `git_main`) plus `worst_git_status` helper. `tui/app/query/git_repo_queries.rs` deleted. ~44 call sites rewritten. Mend: **131 → 123** (-8, within predicted ~5–9). 597/597 tests pass. Path (a) chosen (kept formatted strings on ProjectList; typed-enum split deferred). |
| 4 | **Ready** | Ci pass-throughs. Mechanical visibility narrowing. |
| 5 | **Ready** | Discovery shimmer + project predicates. Mechanical visibility narrowing. |
| 6 | **Ready** | Overlays subsystem extraction (incl. Exit + inline_error). |
| 7 | **Ready** | Path-resolution NavRead (rewritten by Phase 11 if 11 lands first). |
| 8 | **Ready** | Movement/selection mutators stay on App; subsumed by Phase 11. |
| 9 | **Ready** | Focus subsystem. Mechanical visibility narrowing. |
| **10** *(added after Phase 1 review; expanded after Phase 2)* | **Ready** | Two parts: (A) tighten ~30 internal-helper `pub`s in `app/focus.rs` and `app/async_tasks/*` (no API change), and (B) relocate `CiFetchTracker` from `tui/app/types.rs` to `tui/ci_state.rs` and re-narrow its widened methods plus `Ci::fetch_tracker[_mut]` accessors. Predicted ~22–25 reduction, zero design risk. Should run **before** the architectural phases so 11/12/13 build on a tightened App surface. |
| 11 (move `Viewport.pos` → `Selection.cursor`) | **Ready** | Design depth filled in below: post-move structs, `&Scan`-arg method signatures, scroll-follows-cursor location, end-to-end flow. |
| 12 (subsystems implement `Pane`) | **Ready** | Design depth filled in below: `Pane` trait signature, post-Phase-12 `PaneRenderCtx` fields, `RenderSplit<'a>` borrow-helper (post-Phase-6: `&mut Inflight` dropped — output state precomputes), dispatch loop, detail-cache lives at `crate::tui::detail_cache::DetailCache`, `*Layout` rename deferred to follow-up. One execution-time choice flagged: option (a) vs (b) for `Selection`'s `Pane` impl, with (b) recommended. |
| 13 (`Bus<Event>`) | **Ready** | Design depth filled in below: full `Event` enum (~14 variants), full `Command` enum, `EventHandler` trait signature with `&mut self + &Scan`, `EventBus` (~10 lines), drain loop with verified borrows, command pattern eliminating cross-subsystem `&mut`, re-entrancy semantics, `StartupOrchestrator` API, end-to-end traced flow for `apply_config`. |

The earlier rounds of review pointed out gaps that were real; this
pass produced concrete trait signatures, struct definitions,
borrow-composition sketches, and traced flows. All three
architectural phases (9, 10, 11) now have enough design to sit down
and implement.

## Lessons from Phases 1 + 2 + 3 + 4 + 5 (applied to remaining phases)

**See also:** Phase 2's section below has 5 lessons on subsystem-helper
widening, accessor placement, and prediction calibration. Phase 3's
section adds 5 more on prediction calibration confirmation, empty-file
deletion, multi-line caller rewrites, the path-(a)-vs-(b) default, and
the `pub(crate)`-is-free finding (for `ProjectList` only). Phase 4's
section adds 5 more on the `pub(crate)` mend policy inside `tui/app/`,
involuntary -1 from hosting orchestrators in `mod.rs`, helper migration
when methods move up, the empty-file refinement, and the pull-forward
sub-task pattern. Phase 5's section adds 5 more on the viewport-len
sync coupling that broke the "ToastManager-only" prediction, mass-move
as the right pattern for mono-coupled files, third confirmation of the
range methodology, the `mend --fix` post-fmt sequence for inline paths,
and the renumbering payoff.

1. **`pub(super)` from `tui/<subsystem>.rs` reaches the entire `tui/`
   subtree.** No subsystem under `tui/` needs `pub` for its
   methods — `pub(super)` is sufficient. Drop "side-channel widening"
   discussion from Phases 1b, 2, 3, 4, 4b, 5.

2. **The multiplier effect is structural.** Removing a `pub` wrapper
   clears its own warning *and* secondary "only used inside subtree"
   warnings on adjacent self-callers (Phase 1: -14 vs -10 predicted,
   ~40% overshoot). All downstream phase predictions should be read
   as conservative; the visibility-narrowing total (~89-92) is more
   likely 120+, the architectural total (~46) more likely 60+.

3. **Drop the "relocate-but-no-mend-change" items from per-phase
   scopes.** `pub(super)` items aren't visibility hazards; relocating
   them is pure churn. Phase 1 correctly skipped them.

4. **Small phases combine.** Phases 1b, 2, and 4b are each predicted
   1-3 reductions. They combined into a single "trivial subsystems"
   phase (now `1b+2+4b` in the table) — one commit, all of them at
   once.

5. **App's internal helpers are their own cluster.** ~30 of the 133
   remaining warnings are in `app/focus.rs` and `app/async_tasks/*`
   internal helpers — not pass-throughs to subsystems. They're
   helpers that were `pub` set defensively. Adding **Phase 10** (new)
   to handle these as a pure tightening pass before the architectural
   phases.

6. **Open question — wrapper accessors vs. exposed `current()`.**
   Phase 1 added 10 one-line wrappers on `Config` (e.g.
   `lint_enabled()` returning `self.current().lint.enabled`). The
   alternative — drop the wrappers, let callers write
   `app.config().current().lint.enabled` — is cleaner per the doc's
   own "expose subsystems, don't re-export their methods" principle.
   Phase 1 violated that principle; the wrappers shipped because they
   read marginally better at call sites. **Decision for Phases
   1b/2/3/4 onward: prefer exposing `current()` / accessors to
   subsystem-owned data, rather than wrapping each field with a
   one-liner.** Don't retroactively undo Phase 1 (working code,
   smoke-tested), but don't repeat the pattern.

## Phase order

Phases run smallest blast radius first. Each phase is one commit, gated by
`cargo check` + `cargo nextest run --workspace` + `cargo +nightly fmt --all`.
Each phase must independently leave the tree green.

Phases 1–10 are visibility narrowings. Phases 11–13 are architectural
moves derived from the post-Phase-9 review (see end of doc).

**Sequence after merges and additions:**

| # of 16 | Phase | Status |
| ------- | ----- | ------ |
| 1 of 16 | Phase 1 — Config | **Done** (`7160e04`) |
| 2 of 16 | Phase 2 — Trivial subsystems (Keymap + Toasts + Scan/metadata) | **Done** |
| 3 of 16 | Phase 3 — Git/Repo reads → ProjectList | **Done** |
| 4 of 16 | Phase 4 — Ci pass-throughs | **Done** |
| 5 of 16 | Phase 5 — Toast orchestrator relocation | **Done** |
| 6 of 16 | Phase 6 — Discovery shimmer + project predicates | **Done** |
| 7 of 16 | Phase 7 — Mechanical `pub(super)` sweep | Ready |
| 8 of 16 | Phase 8 — Focus subsystem (lives at `tui/focus.rs`) | Ready |
| 9 of 16 | Phase 9 — Overlays subsystem (lives at `tui/overlays.rs`) | Ready |
| 10 of 16 | Phase 10 — Tighten internal-helper visibility (residue after Phases 7 + 8) | Ready |
| 11 of 16 | Phase 11 — Move `Viewport.pos` → `Selection.cursor` (absorbs movement mutators + path resolution from earlier drafts) | Ready |
| 12 of 16 | Phase 12 — Subsystems implement `Pane` | Ready |
| 13 of 16 | Phase 13 — `Bus<Event>` skeleton + `apply_service_signal` (gate) | Ready |
| 14 of 16 | Phase 14 — `apply_lint_config_change` over bus | Ready |
| 15 of 16 | Phase 15 — `apply_config` over bus (introduces `ConfigDiff`) | Ready |
| 16 of 16 | Phase 16 — Startup-phase tracker + `StartupOrchestrator` | Ready |

History: earlier drafts used `1b`, `4b`, `7a`, `7b`, `8b` letter suffixes;
those were resequenced into a 1–13 numeric scheme. Post-Phase-4 review
(a) inserted a Toasts phase as new Phase 5 (orphan from Phase 2 review
identified 11 unclaimed mend warnings in `query/toasts.rs`) and
(b) absorbed an earlier-draft "movement / selection mutators" phase
into Phase 11. Post-Phase-5 review split the original `Bus<Event>` phase
across four phases (13–16) so each ship lands green; Phase 13 is the
gate (skeleton + smallest case), Phases 14–16 widen subscriber surface
incrementally.

Post-Phase-6 review made three structural changes that triggered the
final 1–16 renumbering: (i) inserted a mechanical `pub(super)` sweep
phase (now Phase 7); (ii) put Focus before Overlays (Phase 8 then
Phase 9) — `open_overlay`/`close_overlay` are pure Focus mutations,
so running Focus first lets Overlays be a clean state-only extraction;
(iii) absorbed the earlier-draft "path resolution" phase into Phase 11,
since path-resolution methods don't need `&Panes` after Phase 11's
cursor relocation. Execution order now matches numeric order:

```
1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12 → 13 → 14 → 15 → 16
```

Phase 7 is a mechanical sweep — ~25–30 warnings cleared as a single
commit before architectural work resumes. Phases 13–16 run sequentially
at the end; Phase 13 is the architectural gate — if its bus skeleton
doesn't compile cleanly, Phases 14–16 need rethinking.

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

## Phase 2 — merged trivial subsystems (Keymap + Toasts + Scan/metadata) — **DONE**

Phase 2 combined the original Phases 1b, 2, and 4b into one commit per
the Phase-1 retrospective decision (small phases combine).

**Results:**
- Mend warnings: **133 → 131** (-2 actual vs ~10–15 predicted).
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

**Type widenings made (each adds 1 mend warning):**
- `CiFetchTracker::start` and `::complete` widened from `pub(super)` → `pub`. Required because callers (`tui/panes/actions.rs`, `tui/terminal.rs`) live outside `tui/app/`. Net cost: +2 warnings, but `pub fn complete_ci_fetch_for/start_ci_fetch_for` on App were the same warnings under different names — no regression vs baseline.

**Why the undershoot (-2 vs ~10–15):**
1. **Phase 1's multiplier effect did not repeat.** Phase 1's removed App methods had many self-callers inside `app/*` that became dead-code-eligible once their public form was gone. Phase 2's removed methods were mostly called from outside `app/*` (`tui/render.rs`, `tui/panes/*`, tests), so the secondary clearing was small.
2. **Each accessor we added is itself a `pub(super)` item that mend doesn't flag — but each helper we added on a subsystem is a `pub`/`pub(super)` method that mend does flag if its callers are too narrow.** When `Scan::resolve_target_dir` (`pub(super)`) absorbs callers, mend treats it as fine. When `ToastManager::active_now` (`pub`) absorbs callers from outside `tui/toasts/`, mend flags it. We traded ~11 App pub methods for ~3–4 surviving subsystem-level pub methods.
3. **Some moves were 1:1 not 1:0.** `metadata_store_handle` removed from App, added to Scan. `replace_ci_data_for_path` removed from App, added to ProjectList. The warning count tracks `pub` items, not where they live.

**Lessons (apply to Phases 3, 4, 5, 6, 7, 8, 9):**
1. **Predict reductions by counting only the `pub` removed *minus* the `pub` added.** Don't assume all moves are 1:0 — many are 1:1.
2. **`pub(super)` accessors on App in `mod.rs` cost zero mend warnings.** Always put subsystem accessors directly in `mod.rs`, not in `query/<file>.rs` (where `pub(super)` only reaches `query/`, forcing `pub`). **Caveat:** Phase 1's `app.config()` was placed in `src/tui/app/query/config_accessors.rs` as `pub fn` and is currently a flagged warning (1 of the 131). The Phase 10 cleanup pass should move it to `mod.rs` and re-narrow to `pub(super)`. Don't repeat the pattern in Phases 3–7.
3. **Helper methods on subsystems should default to `pub(super)`** — only widen to `pub` if callers genuinely live outside the subsystem's parent module. Verify per-call-site, not per-pattern.
4. **Don't add wrapper convenience methods unless they save a meaningful boilerplate per caller.** `ToastManager::active_now` saves an `Instant::now()` arg at ~15 call sites; that's worth +1 mend warning. A wrapper that saves no boilerplate just adds a flagged item.
5. **Subsystem-internal types' methods may need widening when callers cross module boundaries.** `CiFetchTracker::start/complete` had to widen because the type lives in `tui/app/types.rs` but callers are in `tui/panes/`. This is structural — the type is in the wrong place. **Phase 10 candidate:** move `CiFetchTracker` from `tui/app/types.rs` to `tui/ci_state.rs` (where it belongs), then re-narrow `start`/`complete` to `pub(super)`. **Also re-narrow `Ci::fetch_tracker` and `Ci::fetch_tracker_mut`** (currently `pub` at `tui/ci_state.rs:84/86`) to `pub(super)` — their callers are all inside `tui/`, so widening past `pub(super)` was never needed. Without that follow-up, you save 2 mend warnings on `start/complete` but lose them again on the accessors.

   **Companion audit:** while moving `CiFetchTracker`, scan the rest of `tui/app/types.rs` for other types whose data-owner module already exists (likely candidates: `DiscoveryShimmer`, `ScanState`, `DirtyState` — all scan-cluster). Relocate alongside if it saves additional widening; otherwise leave them in place to avoid churn.

## Phase 1b — `Keymap` *(superseded by Phase 2 merge — kept for historical reference)*

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
file I/O. They stay on `App` (or move to Overlays in Phase 9) — not
this phase.

**Expected mend reduction:** ~1.

**Risk:** none.

## Phase 2 (original) — `Toasts` *(superseded by Phase 2 merge — kept for historical reference)*

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

## Phase 3 — Git/Repo reads (extract into `ProjectList`) — **DONE**

**Results:**
- Mend warnings: **131 → 123** (-8, within predicted ~5–9 from Phase 2 lesson 1).
- 11 files changed, 203 insertions / 202 deletions.
- 597/597 tests pass; clippy clean; smoke-tested via `cargo install --path .`.
- 9 read methods moved from `App` to `ProjectList`: `git_info_for`, `repo_info_for`, `primary_url_for`, `primary_ahead_behind_for`, `fetch_url_for`, `git_status_for`, `git_status_for_item`, `git_sync`, `git_main`.
- `worst_git_status` helper relocated alongside as a free function in `project_list.rs`.
- `tui/app/query/git_repo_queries.rs` deleted entirely; `mod git_repo_queries` removed from `query/mod.rs`.
- ~44 call sites rewritten as `app.projects().<method>(path)` across 11 files.
- New `ProjectList` methods all `pub(crate)`. No new `pub` items flagged by mend.
- Path chosen: **(a)** — kept formatted strings (`git_sync`, `git_main`) as `pub(crate) fn` on `ProjectList` with the existing format logic. Path (b) (typed `SyncDisplay` enum + render-side format) deferred; revisit only if cross-cluster format reuse appears.

**Lessons (apply to remaining phases):**

1. **Phase 2 lesson 1's range methodology held.** Predicted ~5–9, hit -8 — first phase with calibrated predictions matching reality. Continue applying `pub removed minus pub added` to all forward phases.

2. **Empty-file deletion is one extra step worth doing inside the same phase.** When a file's entire contents move out, delete the file and unregister it from `query/mod.rs` (or wherever) in the same commit. Phase 3's `git_repo_queries.rs` deletion saved the follow-up "remove empty module" commit the original retrospective scaffolding had reserved. **Apply to:** Phase 4 (Ci queries may collapse `query/ci_queries.rs` similarly), Phase 6 (`query/discovery_shimmer.rs` and/or `query/project_predicates.rs`), Phase 8 (`app/focus.rs`).

3. **Multi-line `app\n.method(` callers are not caught by simple `sed`.** Phase 3 needed a fallback `perl -0pe` pass for the chained-call sites in `tui/app/async_tasks/repo_handlers.rs`, `tui/app/ci.rs`, etc. Future phases should run the perl pass eagerly, not as a fallback. Pattern:
   ```bash
   perl -i -0pe 's/(\bself|\bapp)\n(\s+)\.(<method1>|<method2>|...)\(/$1\n$2.<accessor>()\n$2.$3(/g' <files>
   ```

4. **Path-(a)-vs-(b) tradeoff: default to (a) when no caller benefits from the typed return.** Phase 3's `git_sync`/`git_main` produce strings consumed only by the project-list pane render. Path (b) (typed enum + render-side format) introduces a new pub type and a format helper for *no caller* that needs the typed form. Path (a) keeps the model layer with one extra string-formatting method but spends zero new public surface. The "model layer stays formatting-free" argument is real but only pays off when ≥2 callers branch on the typed state. **Default for future phases: (a) unless you can name two callers that need the structural information.**

5. **The `pub(crate)` items on `ProjectList` were free.** No mend warnings added — `pub(crate)` is the existing convention on `ProjectList` and not flagged by `suspicious_pub`. Confirms Phase 2 lesson 2's framing: subsystem helpers default to whatever the subsystem's existing visibility convention is, not `pub`.



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

**Expected mend reduction (post-Phase-2 lesson 1 — `pub` removed minus `pub` added):**
- removed: ~9 App methods
- added: ~3 likely flagged (any `ProjectList` git/repo helper used from `tui/panes/*` may need `pub`/`pub(crate)`)
- net: ~5–9 (range)

**Risk:** medium — touches render path. Land **before** Phase 4 (Ci) so
`Ci::for_path` can call `ProjectList::primary_ahead_behind_for` rather
than re-implementing it.

## Phase 4 — `Ci` (depends on Phase 3) — **DONE**

**Results:**
- Mend warnings: **123 → 114** (-9, beat predicted ~6–7).
- Tests: 597/597 pass; clippy clean; smoke-tested via `cargo install --path .`.
- 3 read methods moved from `App` to `ProjectList`: `ci_data_for`, `ci_info_for`, `unpublished_ci_branch_name`. All `pub(crate)` (free per Phase 3 lesson 5).
- 5 orchestrator/glue methods moved from `tui/app/query/ci_queries.rs` to `tui/app/mod.rs` so `pub(super)` would reach `tui/`: `selected_ci_path`, `selected_ci_runs`, `ci_for`, `ci_is_fetching`, `ci_for_item`. All previously `pub` on App; each move counted as -1 mend warning.
- Helper `unique_item_paths` moved with `ci_for_item` from `query/project_predicates.rs` into `mod.rs` (private `fn`, no `impl App` needed).
- `query/ci_queries.rs` slimmed to 1 method (`ci_is_exhausted`, kept at `pub(super)`); not deleted because `ci_is_exhausted` belongs adjacent to its only caller in the same `query/` parent.
- `app.config()` pulled forward: moved from `query/config_accessors.rs` (`pub`) to `mod.rs` (`pub(super) const fn`), file deleted, `mod config_accessors` removed from `query/mod.rs`.
- Path (a) chosen for `ci_is_fetching`: stayed on `App` as 4-line Ci+ProjectList glue; (b) ("`Ci::is_fetching_for_entry(&self, projects: &ProjectList, path: &Path)`") would cross the boundary the original plan tried to avoid.

**Lessons (apply to remaining phases):**

1. **`pub(crate)` is forbidden by mend policy inside `tui/app/`** — only outside `crate::tui::app` (e.g., `crate::project_list`) is `pub(crate)` accepted. Inside the App subtree, `pub(crate)` becomes a hard mend `error`, not a warning. Methods on `App` that need broad visibility must live in `tui/app/mod.rs` and use `pub(super)` (which reaches `tui/`). Phase 3 lesson 5 ("`pub(crate)` is free") applies only to `ProjectList`, not to `App`. **Apply to:** every remaining App-method narrowing — if the method is called from `tui/panes/`, `tui/render.rs`, etc., plan to host it in `mod.rs`, not in a `query/*.rs` submodule.

2. **Hosting an App method in `mod.rs` is involuntary -1.** When a previously-`pub` orchestrator moves up to `mod.rs` to satisfy `pub(super)` rules, the move *is* the visibility narrowing — even when the move wasn't the primary goal. Phase 4's overshoot (-9 vs -6/-7 predicted) came entirely from the 5 orchestrator methods that had to move up. **Calibration:** when listing methods staying on App, count any that are still `pub` (not `pub(super)`) as -1 each in the prediction.

3. **Helpers travel with their callers when moving up.** When `ci_for_item` moved to `mod.rs`, `unique_item_paths` (a `pub(super)` static helper in `query/project_predicates.rs`) had to move with it, because `pub(super)` from the deeper module doesn't reach `mod.rs`. Audit dependencies before moving methods up; private static helpers without `&self` migrate cleanly as plain `fn` in the destination module.

4. **`query/ci_queries.rs` did not delete cleanly.** Phase 3 lesson 2 (empty-file deletion) didn't apply: `ci_is_exhausted` belongs in `query/` because its only caller (`post_selection.rs`) is also in `query/`, and moving it to `mod.rs` would widen needlessly. Refinement to lesson: delete only when ≥1 method in the file has callers that force broader visibility AND no remaining method has narrow-enough callers to justify staying. Otherwise, slim the file in place.

5. **Pull-forward sub-tasks pay off.** `app.config()` cleanup (originally a Phase 1 leftover) was a 5-minute change that bundled cleanly with Phase 4's diff. Pattern confirmed: each phase should sweep one or two adjacent micro-debts the original plan pre-classified, not gate them behind a separate phase.

## Phase 4b — Scan / metadata pass-throughs *(superseded by Phase 2 merge — kept for historical reference)*

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

## Phase 5 — Toast orchestrator relocation — **DONE**

**Results:**
- Mend warnings: **114 → 103** (-11; predicted ~8–11 — within range, hit upper bound).
- 13 files changed (estimated; from `git diff --stat` at execution).
- 597/597 tests pass; clippy clean; smoke-tested via `cargo install --path .`.
- All 11 `pub` methods moved from `query/toasts.rs` to `tui/app/mod.rs` as `pub(super)`: `focused_toast_id`, `prune_toasts`, `show_timed_toast`, `show_timed_warning_toast`, `start_task_toast`, `finish_task_toast`, `set_task_tracked_items`, `mark_tracked_item_completed`, `start_clean`, `clean_spawn_failed`, `dismiss_toast`. Plus the private `toast_timeout` helper.
- `query/toasts.rs` deleted; `mod toasts;` removed from `query/mod.rs`.
- Imports hoisted into `mod.rs`: `Duration`, `Warning`, `ToastView`, `TrackedItem`. Mend `--fix` auto-hoisted `crate::project::home_relative_path` after fmt (1 import-fix applied).
- All 11 methods touched **at least two subsystems** (Toasts + Panes for `viewport.set_len(toast_len)` sync after every mutation). Pre-execution review predicted "most touch only ToastManager"; that turned out wrong — every mutator path syncs the pane viewport-len, so every method is a Toasts+Panes orchestrator. No methods moved into `ToastManager` (path (a) of the original strategy was vacant).

**Lessons (apply to remaining phases):**

1. **The "ToastManager-only" prediction was wrong because of the viewport-len sync.** Every toast-mutation method ends with `self.panes_mut().toasts_mut().viewport_mut().set_len(toast_len)` — a Panes write. That coupling is invisible from a method-list scan; only reading the bodies surfaces it. **Apply to Phase 9+:** for any "move method into subsystem X" prediction, audit body for cross-subsystem writes (especially viewport/cache invalidation) before locking in the (a)-into-subsystem path. **Caveat (post-Phase-5 architecture review):** this pattern is narrower than it first appears. Most pane `viewport_mut().set_len(rows.len())` calls happen inside *render bodies* (`src/tui/panes/package.rs:250`, `git.rs:661`, `lints.rs:160,173`) — those are not cross-subsystem writes from mutators; the render fn already holds `&mut Pane`. The genuine recurrences are: (i) `focus_next_pane`/`focus_previous_pane` in `focus.rs:228-272` (collapses cleanly after Phase 12 moves the toast viewport into `ToastManager`), and (ii) `reset_project_panes` in `focus.rs:274-285` (six pane viewport mutations from one orchestrator). Phase 6 (shimmer) does NOT have this pattern — shimmer mutations are `Scan`-only. Phase 8 has a transitive instance: `focus_pane → self.panes.mark_visited`, which the new "migrate `mark_visited` into `Focus`" sub-task in Phase 8 eliminates.

2. **Mechanical mass-move was the right call.** All 11 methods sat in one file with the same coupling, and all 11 had external callers. Moving them as a block to `mod.rs` was one perl pass + one delete. Time-to-green: ~5 minutes of editing + one mend `--fix` for the inline-path import. **Pattern:** when an entire `query/*.rs` file's methods share the same "external callers + multi-subsystem touch" property, move the block, don't triage method-by-method.

3. **Range methodology held a third time.** Phase 2 lesson 1 (count `pub` removed minus `pub` added) predicted 8–11 with 11 being the upper-bound when zero new `pub` items needed adding. Hit -11 exactly because `pub(super)` accessors don't add mend warnings. Forward phases should treat the upper end of `pub`-removed minus `pub`-added as the realistic target when no new subsystem types are introduced.

4. **`mend --fix` after `fmt` is the canonical sequence for inline-path cleanups.** Phase 5 used `crate::project::home_relative_path` inline (because `home_relative_path` wasn't already imported). Mend's `prefer-module-import` lint flagged it as auto-fixable; one `cargo mend --fix` resolved it. **Apply:** after any phase that introduces a method body referencing crate paths, run `cargo mend --fix` once before final validation.

5. **Phase 4.5 → Phase 5 renumbering paid off immediately.** The retrospective sits cleanly at slot 5 in the canonical sequence with no "see also Phase 4.5" footnotes. Worth the 51-reference sed pass that landed in commit `e97f7c3`.

## Phase 6 — Discovery shimmer + project predicates — **DONE**

**Results:**
- Mend warnings: **103 → 92** (-11; predicted ~9–13 — within range, near upper bound).
- 597/597 tests pass; clippy clean; smoke-tested via `cargo install --path .`.
- 4 read methods moved to `ProjectList` as `pub(crate)` (free per Phase 3 lesson 5): `is_deleted`, `is_rust_at_path`, `is_vendored_path`, `is_workspace_member_path`.
- 1 method moved to `Scan` as `pub(super)` (free, outside `tui/app/`): `prune_shimmers` (renamed from `prune_discovery_shimmers`).
- 5 orchestrators relocated to `tui/app/mod.rs` as `pub(super)`: `animation_elapsed`, `register_discovery_shimmer`, `discovery_name_segments_for_path`, `selected_project_is_deleted`, `prune_inactive_project_state`. The first three were involuntary `mod.rs` rehosts per Phase 4 lesson 3; `prune_inactive_project_state` was an unanticipated rehost (its only caller is in `tui/app/construct.rs`, which `pub(super)` from `query/` doesn't reach).
- 2 view-formatting helpers moved to `panes/project_list.rs` as free fns (re-exported from `panes/mod.rs` as `pub(super)`): `formatted_disk`, `formatted_disk_for_item`. Path (a) chosen — pass `&App` to the helper rather than introduce a `DiscoveryShimmerView<'a>` type.
- ~12 private helper fns travelled with `discovery_name_segments_for_path` into `mod.rs` (Phase 4 lesson 4): `discovery_shimmer_session_for_path`, `discovery_shimmer_session_matches`, `discovery_scope_contains`, `discovery_parent_row`, `discovery_shimmer_window_len`, `discovery_shimmer_step_millis`, `discovery_shimmer_phase_offset`, `DiscoveryParentRow` struct, `package_contains_path`, `workspace_contains_path`, `root_item_scope_contains`, `workspace_scope_contains`, `package_scope_contains`, `root_item_parent_row`, `workspace_parent_row`, `package_parent_row`. The two methods on `DiscoveryRowKind` (`allows_parent_kind`, `discriminant`) moved to `tui/app/types.rs` next to the enum, as `pub(super)`.
- 3 files deleted: `query/discovery_shimmer.rs`, `query/project_predicates.rs`, `query/disk.rs`. `query/mod.rs` now lists only `ci_queries` and `post_selection`.
- ~25 call-site rewrites across `panes/project_list.rs`, `panes/support.rs`, `terminal.rs`, `dismiss.rs`, `tests/rows.rs`, `tests/panes.rs`, `tests/state.rs`, `tests/worktrees.rs`, `tests/discovery_shimmer.rs`, `tests/mod.rs`, `async_tasks/lint_handlers.rs`, `async_tasks/lint_runtime.rs`.
- Mend `--fix` auto-applied 8 of 11 inline-path-import fixes after the move; 3 fixes failed mid-run (mend's transform produced non-compiling code for `crate::tui::panes::formatted_disk` — Phase 4 lesson 4's "imports auto-fixed" pattern doesn't extend to module-qualified paths) and were applied by hand.

**Lessons (apply to remaining phases):**

1. **`mend --fix` is not a drop-in for inline-module-path imports across module boundaries.** When a moved function lives behind a `pub(super) use` re-export (here, `panes/project_list.rs::formatted_disk` re-exported via `panes/mod.rs`), mend's auto-fix transforms `crate::tui::panes::project_list::formatted_disk` into a `use` import that doesn't compile (the source module is private). It rolled back cleanly. **Apply:** when moving methods that callers reach via re-exports, expect 2–3 inline-path fixes will need manual application. Run `cargo mend --fix` first, capture any rollback, fix the residue by hand. The `super::formatted_disk` form (when the caller is also under `panes/`) is the cleanest replacement.

2. **`pub(crate)` is forbidden in `tui/panes/` too, not just `tui/app/`.** Phase 4 lesson 1 said "outside `tui/app/`, `pub(crate)` is fine." Phase 6 found a counter-example: `pub(crate) fn formatted_disk` in `tui/panes/project_list.rs` triggered the same "use of `pub(crate)` is forbidden by policy" mend error. The rule is broader than originally captured — `pub(crate)` is forbidden across all of `tui/`, not just `tui/app/`. Outside `tui/` (e.g., `crate::project_list`, `crate::scan_state` if it lived at crate root) is where `pub(crate)` is free. The corrected pattern: `pub fn` from a private module + `pub(super) use` re-export from the parent. **Apply to Phase 8 (Focus at `tui/focus.rs`) and Phase 9 (Overlays at `tui/overlays.rs`):** subsystem types directly at `crate::tui::*` get `pub(crate)` free, but their *internal helpers* in private submodules need the `pub fn + pub(super) use` re-export pattern.

3. **`prune_inactive_project_state` was a missed prediction.** The plan said this method "stays in `query/project_predicates.rs` narrowed to `pub(super)`." But its caller is `tui/app/construct.rs`, and `pub(super)` from `query/project_predicates.rs` reaches only `query/` — not `construct.rs` which is a sibling of `query/` under `tui/app/`. So this method had to move to `mod.rs` too. **Apply:** for any method "narrowed in place" decision, audit the caller's module path against the file's `super`. If the nearest common ancestor is wider than the file's `super`, it's a `mod.rs` rehost.

4. **3-file deletion in one phase is the upper bound seen so far.** Phases 3 and 4 each deleted 1 file; Phase 5 deleted 1; Phase 6 deleted 3 (`discovery_shimmer.rs`, `project_predicates.rs`, `disk.rs`). The pattern: when `query/*.rs` files have only `pub fn` items with external callers, all three fall together once the orchestrators move to `mod.rs`. **Apply to Phase 10 Part C:** the `query/*` empty-file sweep is real but already mostly done by this point — only `query/ci_queries.rs` (one method left) and `query/post_selection.rs` (one method) remain after Phase 6. Part C's prediction of ~3-5 sweep wins shrinks to ~1-2.

5. **Phase-5-pattern audit (viewport-len sync) held: shimmer mutations are clean.** `Scan::prune_shimmers(&mut self, now)` is single-borrow `&mut self.discovery_shimmers`. No Panes coupling, no transitive viewport-len sync. The (a)-into-subsystem path was open exactly as the audit predicted. Phase 5's coupling pattern is genuinely narrow; don't read it as universal.

## Phase 7 — Mechanical `pub(super)` sweep (post-Phase-6 review surfaced this)

**Context:** post-Phase-6 mend audit found 92 remaining warnings, **all
hinting `pub(super)`**. ~50 of those sit in files that no architectural
phase will touch: `src/tui/app/async_tasks/*` (~25), `tui/app/navigation/*`
(~14), `tui/app/dismiss.rs` (2), `tui/app/types.rs` (3),
`tui/app/query/post_selection.rs` (2), `tui/panes/*` (3). They are
internal helpers that were `pub` set defensively and are pinpointed by
mend with no design risk.

**Strategy:** for each warning, accept mend's `pub(super)` hint and
verify compilation. Skip anything that doesn't compile cleanly under
`pub(super)` from its current location — those go to Phase 10.

**Files in scope:**
- `tui/app/async_tasks/config.rs`, `lint_runtime.rs`, `repo_handlers.rs`,
  `metadata_handlers.rs`, `disk_handlers.rs`, `service_handlers.rs`,
  `startup_phase/tracker.rs`, `tree.rs`, `poll.rs`
- `tui/app/navigation/cache.rs`, `selection.rs`, `movement.rs`,
  `expand.rs`, `bulk.rs`
- `tui/app/dismiss.rs`, `tui/app/types.rs`, `tui/app/query/post_selection.rs`
- `tui/panes/support.rs`, `tui/panes/project_list.rs`

**Why ship this before Phase 9:** with ~25 internal-helper warnings
cleared, Phase 9s/9/11/12 reductions become measurable rather than
buried in the mend signal. Also forecloses the Phase 10 Part A "pile
of disconnected tightenings" risk — the sweep is mechanical and ships
green in one commit, not a multi-phase trickle.

**Expected mend reduction:** ~25–30 (target: 92 → ~62–67).

**Risk:** very low — every fix is mend-pinpointed and verified by
`cargo check`. Reverts are per-line.

**Sequencing:** runs as a single mid-stream commit, before Phase 9.
Phase 10 Part A then covers only the residue (~5–10 stragglers, methods
that don't accept `pub(super)` from their current file because callers
sit outside the file's `super` — Phase 6 lesson 3 recurrences).

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

**Migrate `Panes::mark_visited`/`unvisit` state into `Focus` (post-Phase-5
review):** `focus.rs:147,150,281-284` calls `self.panes.mark_visited(...)`
and `self.panes.unvisit(...)` from focus-cluster code. The visited-pane
set is Focus state living in Panes. Phase 8 should pull it across:
`Focus` owns a `visited_panes: HashSet<PaneId>` field; `Panes` drops the
duplicate. This is the same anti-pattern Phase 5 found with the toast
viewport-len sync — state living on the wrong subsystem because the
mutation pathway goes through it. Captured here so it lands with Phase 8
rather than as a Phase 14 follow-up.

**Phase-5-pattern audit for Focus:** `focus_pane` (`focus.rs:147-153`)
calls `self.panes.mark_visited(pane)` — a Focus mutation that writes to
`Panes`. After the `mark_visited` migration above, this becomes
`self.visited.insert(pane)` (single-borrow `&mut Focus`). Without the
migration, `focus_pane` is a Focus+Panes orchestrator that must stay
on App as `pub(super)` glue.

**Subsystem location decision (post-Phase-4 lesson 1):**
Two options for where `Focus` lives in the module tree:
- **(a) `tui/app/focus.rs`** (today's location) — `pub(super)` from Focus
  methods reaches only `tui/app/`, NOT `tui/panes/`, `tui/render.rs`, etc.
  Every Focus method called externally is forced to also live in
  `tui/app/mod.rs` as a `pub(super)` orchestrator (Phase 4 lesson 3).
- **(b) `tui/focus.rs`** — mirrors `tui/selection.rs`, `tui/ci_state.rs`,
  `tui/toasts.rs`. `Focus` becomes a `tui`-level subsystem; methods can be
  `pub(crate)` (free per Phase 4 lesson 1, since `crate::tui::focus` is
  outside `tui/app/`). External callers reach methods directly via
  `app.focus().method()`.

**Pick (b).** Phase 4's confirmation that subsystem types living outside
`tui/app/` get free `pub(crate)` makes (b) save ~6 mend warnings vs (a)
that would otherwise become involuntary `mod.rs` rehosts. The relocation
itself is mechanical — same as Phase 4's `app.config()` cleanup pattern.

**Tradeoff:** this is the deepest entanglement of the bunch — focus depends
on overlays which depend on selection which depends on projects. Doing this
last lets the prior phases land first so dependencies are exposed as proper
subsystem reads, not as field reaches. Doing it first would force premature
abstractions.

**Expected mend reduction (post-Phase-4 calibration):**
- removed: ~16 App methods
- added: ~3 (Focus accessor + helpers — `pub(crate)` on `tui/focus.rs` is free per Phase 4 lesson 1)
- saved: ~6 from picking location (b) over (a)
- net: **~12–18** (revised up from ~10–16)

**Risk:** highest — touches every keyboard handler.

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

**Methods to relocate:**
- `open_settings`, `close_settings`, `is_settings_open`, `begin_settings_editing`, `end_settings_editing`, `is_settings_editing`
- `is_finder_open`, `open_finder`, `close_finder`
- `open_keymap`, `close_keymap`, `is_keymap_open`, `is_awaiting_key`, `keymap_begin_awaiting`, `keymap_end_awaiting`
- `request_quit`, `should_quit`, `request_restart`, `should_restart`
- `open_overlay`, `close_overlay`

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

**Refactor:** create `crate::tui::overlays::Overlays` owning the
`UiModes` (rename to `OverlayState` once moved). Methods become `Overlays`
methods. `App` exposes:
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

**Expected mend reduction (post-Phase-4 calibration):**
- removed: ~18 App methods (with `inline_error` path (a))
- added: ~2 (`overlays()` / `overlays_mut()` accessors on App, `pub(super)`)
- saved: ~6 from picking location (b) over (a) — avoids `mod.rs` rehosts for external callers
- net: **~14–20** (revised up from ~12–18)

**Risk:** low-medium — `inline_error` callers from outside Overlays need
re-routing to `app.overlays().inline_error()`. Audit before commit.

## Path resolution — earlier-draft phase, absorbed into Phase 11 by post-Phase-6 review *(retained as historical pointer)*

**Status:** the post-Phase-6 architecture review traced every path-resolution
method through the code. Each one reads `panes` exactly once —
`self.panes().project_list().viewport().pos()` in `selected_row` and
`selected_display_path`. After Phase 11 moves the cursor to `Selection`,
both reads become `self.selection.cursor()` and there's nothing left
that needs `&Panes`. The remaining bodies are pure `RootItem` /
`WorktreeGroup` walks. Each method becomes either a `Selection` method
taking `&Scan` as an arg, or a free fn on `RootItem`. The `NavRead<'a>`
wrapper struct the original Phase 8 design proposed is unneeded — Phase 11
already provides the structural answer.

**Distributed across:**

*Move to `Selection` (under Phase 11):*
- `selected_row` → `Selection::selected_visible_row(&self) -> Option<VisibleRow>`
- `selected_item`, `selected_project_path`, `selected_display_path`,
  `clean_selection`, `selected_is_expandable`

*Move to free fns on `RootItem` / `WorktreeGroup` in `crate::project`:*
- `path_for_row`, `display_path_for_row`, `abs_path_for_row`,
  `expand_key_for_row`, `row_matches_project_path`
- The worktree path helpers (`worktree_path_ref`, `worktree_member_abs_path`,
  `worktree_display_path`, etc.) — they don't need any `App` field

*Stays as App orchestrator (cross-frame coordination):*
- `row_count`, `visible_rows` — both read `cached_visible_rows` from
  `Selection`; collapse to `app.selection().cached_visible_rows().len()` /
  call sites and remove the wrappers

**Net mend reduction: 0 as a standalone phase** — accounted for in
Phase 11's column. Original ~8–12 estimate stays, just attributed to
Phase 11 instead.

## Movement / selection mutators — earlier-draft phase, superseded by Phase 11 in post-Phase-4 review *(kept for historical reference)*

**Status:** the original Phase 8 had net mend reduction of **0** by design —
it documented methods that "stay on App." Post-Phase-4 review concluded
that since Phase 11 moves the project-list cursor onto `Selection`, every
method in the original Phase 8 list either becomes a `Selection` method
(collapse/expand/movement mutators) or stays on App as an orchestrator.
There is no independent work in this earlier-draft phase. The list below
moves into Phase 11's prose; this section remains only as a navigation
aid for readers of older drafts. **In the post-Phase-6 sequence, slot 8
is now the Focus subsystem; movement mutators and path resolution are
both inside Phase 11.**

**Original Phase 8 list (now distributed across Phase 11 and "stays as orchestrator"):**

*Move to `Selection` (under Phase 11):*
- `move_up`, `move_down`, `move_to_top`, `move_to_bottom`, `collapse_anchor_row`
- `expand`, `collapse`, `expand_all`, `collapse_all`, `try_collapse`, `collapse_to`, `collapse_row`
- `select_project_in_tree`, `select_matching_visible_row`, `expand_path_in_tree`
- `sync_selected_project`

*Stays on `App` as orchestrator (multi-subsystem reads):*
- `ensure_visible_rows_cached`, `ensure_fit_widths_cached`, `ensure_disk_cache`
- `ensure_detail_cached` — reads `Ci`, `Lint`, `Scan.generation()` to decide whether to rebuild

**Why the original "stays on App" framing was wrong:** Phase 8's argument was
that mutators touch both `Selection.expanded` and `Panes::project_list().viewport().pos()`,
so a separate `Navigator` type can't own them. Phase 11 invalidates this by
moving the viewport cursor onto `Selection` itself — the mutators become
single-borrow `&mut Selection` methods, no `Navigator` type needed.

**Net mend reduction: 0** — accounted for in Phase 11's column.

## Phase 10 — Tighten internal-helper visibility + relocate `CiFetchTracker`

A purely structural cleanup phase identified after Phase 1 and refined
after Phase 2. No new APIs, no behavior change — just narrowing
visibility on items that were `pub` set defensively.

**Two parts:**

### Part A — Internal-helper tightening (post-Phase-6 review: substantially reduced)

Post-Phase-6 review re-scoped Part A. The original ~70-warning estimate
assumed all internal helpers would be tightened in one phase. Two
intervening shifts shrink the residue:

1. **Phase 7 (mechanical sweep) ships ~25–30 of these as a single
   pre-Phase-7 commit.** Files in scope: `async_tasks/*`,
   `navigation/*`, `dismiss.rs`, `types.rs`, `query/post_selection.rs`,
   `panes/*`.
2. **Phase 8 (Focus subsystem extraction) absorbs `focus.rs`'s 33
   warnings incidentally** when methods relocate to `tui/focus.rs`.

After Phase 7 + 8 + 9 land, Part A's residue is the methods whose mend
hint won't compile under `pub(super)` because the caller's nearest
common ancestor is wider than the file's `super` — Phase 6 lesson 3
recurrences. Estimated **~5–10 stragglers**.

**Method:** for each remaining `pub fn`, accept mend's hint or, if
the caller is outside the file's `super`, host in `tui/app/mod.rs`
instead. Never use `pub(in path)` (project convention; mend forbids
`pub(crate)` for items under `tui/`).

**Predicted reduction (recalibrated):** ~5–10. Most of the original
Part A surface is absorbed by Phase 7 + 8 + 9.

### Part B — Relocate `CiFetchTracker` to `tui/ci_state.rs`

`CiFetchTracker` currently lives in `src/tui/app/types.rs` (line 278).
Its data home is the CI subsystem — it's the in-flight set of CI fetch
paths. Phase 2 had to widen `CiFetchTracker::start` and `::complete`
from `pub(super)` to `pub` because callers (`tui/panes/actions.rs`,
`tui/terminal.rs`) live outside `tui/app/`. Each widening is a +1 mend
warning.

After relocation to `tui/ci_state.rs`:
- `pub(super)` from `ci_state.rs` reaches all of `tui/`, so callers in `tui/panes/` and `tui/terminal.rs` work without widening.
- `CiFetchTracker::start` and `::complete` re-narrow from `pub` → `pub(super)`. -2 warnings.
- The accessors `Ci::fetch_tracker(&self)` and `Ci::fetch_tracker_mut(&mut self)` in `ci_state.rs` (lines 84/86) are currently `pub` but their callers are all inside `tui/`. Re-narrow to `pub(super)`. -2 warnings.

**Net from Part B:** -4 warnings.

**Sub-task:** update the `pub(super) use types::CiFetchTracker;`
re-export at `src/tui/app/mod.rs:118` — either delete it (if the type
is now reached as `crate::tui::ci_state::CiFetchTracker`) or replace
with `pub(super) use super::ci_state::CiFetchTracker;`.

**Audit candidate:** while moving `CiFetchTracker`, scan the rest of
`tui/app/types.rs` for other types whose data-owner module already
exists. Any that fit the same pattern should move alongside in this
phase. (`DiscoveryShimmer`, `ScanState`, `DirtyState` are likely
candidates — they're scan-cluster types currently in App's bag.)
Move them only if it saves additional widening; relocation that
doesn't change the mend count is churn and stays out of scope.

### Part C — `query/*` empty-file sweep (Phase 3 lesson 2 applied)

After Phases 3, 4, 5 run, several `tui/app/query/<file>.rs` modules
will be drained of all `pub` items. Phase 3 set the precedent
(deleted `git_repo_queries.rs`); Phase 10 Part C is the catch-all
sweep for the rest:

- `query/disk.rs` — formatting helpers; move to a render-side helper or `ProjectList`. **Likely deletes** after Phase 6.
- `query/ci_queries.rs` — most contents move to `ProjectList` or `Ci` in Phase 4. **Likely deletes** if `selected_ci_*` orchestrators relocate to a `selection`-related module.
- `query/discovery_shimmer.rs` — only private session-helpers remain after Phase 6. **Likely deletes.**
- `query/project_predicates.rs` — `prune_inactive_project_state` is a Scan+Ci orchestrator and stays on App. File survives but shrinks.
- `query/post_selection.rs` — `sync_selected_project` and `enter_action` are real orchestrators (touch ≥3 subsystems each). **Stays.** Do not drain.
- `query/toasts.rs` — orchestrator methods (`prune_toasts`, `show_timed_toast`, etc.) stay per Phase 2 plan. **Stays.**
- `query/config_accessors.rs` — emptied in Phase 4 by the pull-forward sub-task. **Deletes** during Phase 4, not Phase 10.

**Net from Part C:** -3 to -5 (each empty-file deletion clears 0–2 stragglers; main savings is structural, not warning count).

**Predicted reduction (Parts A + B + C):** **~32–48** (was ~22–25; widened by Part A recalibration and added Part C).

**Risk:** zero design risk — every change is local. Some risk that a
narrower visibility doesn't compile and forces a partial widening; in
that case mend's hint is wrong and the wider visibility stays.

## Phase 11 — Move `Viewport.pos` to `Selection.cursor` (Cluster C)

Source of truth: see "Item 4" in the post-Phase-9 review section
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

**Ordering constraint:** Phase 11 supersedes Phase 8's `NavRead { panes }`
design. Land Phase 11 before Phase 8, or skip Phase 8 and let Phase 11
absorb its work.

**No `include_non_rust` cache field on `Selection` (post-Phase-5 review
correction):** earlier drafts proposed caching `include_non_rust` on
`Selection` to avoid re-reading `Config` per frame, with a sync
obligation routed through `apply_config`. That design has been dropped.
Reasons:
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
the bool into `Selection`'s state. Phase 13's `ConfigChanged` event
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

**Expected mend reduction:** ~12 + ~8–12 (path resolution) = **~20–24**.

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
visibility recompute self-contained. The Phase 13 `ConfigDiff`
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

**Status: ready to execute.**

## Phase 12 — Subsystems own pane state; drop wrapper types (Cluster B for panes)

Source of truth: see "Item 5" in the post-Phase-9 review section
below. Summary:

- Widen `Pane` trait visibility from `pub(super)` to `pub(crate)` in
  `tui/panes/dispatch.rs`.
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
- Detail-pane data cache (`DetailCacheKey`-keyed) home (post-Phase-5
  loose-ends decision): introduce **`DetailCache`** at
  `src/tui/detail_cache.rs` (outside `tui/app/`), `pub(crate)` on its
  methods. Path (b) — "keep in existing wrappers as the reason they
  remain" — contradicts Phase 12's drop-wrappers thesis, so it's out.
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

**Expected mend reduction:** ~9 (the Toast+Panes orchestrators that
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

**Post-Phase-12 `Pane` trait** (visibility widened to `pub(crate)`):
```rust
pub(crate) trait Pane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>);
}
```

Same signature; only the visibility changes.

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
project-list render keeps a thin wrapper that borrows. Worth ~0
mend-warning difference; saves one design contortion.

**Thesis exception (full enumeration):** Phase 12's headline says
"drop the wrapper types," but the actual outcome is mixed.

| Original wrapper | Fate | Reason |
|---|---|---|
| `ToastsPane` | **collapse** — viewport into ToastManager | Pure pass-through wrapper |
| `CiPane` | **collapse** — viewport into Ci, content moves to Ci | Pure pass-through wrapper |
| `LintsPane` | **collapse** — viewport into Lint, content moves to Lint | Pure pass-through wrapper |
| `KeymapPane` | **collapse** — into Overlays (Phase 9) | Phase 9 dependency |
| `SettingsPane` | **collapse** — into Overlays | Phase 9 dependency |
| `FinderPane` | **collapse** — into Overlays | Phase 9 dependency |
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
  spanning Selection, Net, Ci, Scan, and git state. After Phase 12,
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

## Phase 13 — `Bus<Event>` skeleton + `apply_service_signal` (Cluster A, gate phase)

Source of truth: see "Item 6" in the post-Phase-9 review section
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

**Phase 13 is the bus skeleton + smallest case (post-Phase-5 review):**
the original "all-at-once" framing put every cross-cutting orchestrator
on the line at once. The architectural risk is that `EventBus` shape
doesn't fit Rust's borrow checker against a real subscriber set;
discovering that after `apply_config` is converted is expensive. Cluster
A is now spread across Phases 13–16 so each ship lands green:

- **Phase 13 — Bus skeleton + smallest case.** Introduce `EventBus`,
  `Event`, `Command`, `EventHandler`, `HandlerCtx`. Convert
  `apply_service_signal` only (single event variant, 1–2 subscribers).
  Validate the drain-loop pattern compiles and behaves identically.
  ~3 mend reduction. **This is the architectural gate.** If this
  phase doesn't compile cleanly, Phases 14–16 need rethinking.
- **Phase 14 — `apply_lint_config_change`.** Three subsystems (Inflight,
  Scan, Selection), all already orchestrated through one method.
  Clean conversion to one `Event::LintConfigChanged` with three
  subscribers. ~2 mend reduction.
- **Phase 15 — `apply_config`.** Highest fan-out (6 branches) and
  requires `ConfigDiff`. Wait until Phases 13–14 are stable. ~6 mend
  reduction.
- **Phase 16 — Startup-phase tracker + `StartupOrchestrator`.** Largest
  body but most isolated — it's already a state-machine adapter.
  ~4 mend reduction plus the file relocation.

Each phase ships green and is independently revertible. The
all-or-nothing framing was the real risk.

**Realistic mend reduction across Phases 13–16 (revised down from ~25):**
body counting from `apply_config` (`async_tasks/config.rs:138-197` —
6 fan-out branches) plus `apply_service_signal`,
`mark_service_recovered`, `apply_lint_config_change`, and the 6
`maybe_complete_startup_*` family yields ~15 methods that genuinely
collapse, not 25. The other ~10 in Cluster A are sub-helpers
(`sync_running_lint_toast`, `clear_all_lint_state`, etc.) that become
Commands — they don't disappear from the codebase, they migrate from
`impl App` to `apply_command` arms. Mend count drops ~10–15 across the
four phases, not 25.

**`apply_command` is itself a god-table.** A 21-arm match on App
mirrors the orchestrator-method count we removed. Phases 13–16's "App
becomes thin" claim is half-true: bodies of orchestrator methods move
out, but a comparably-sized dispatch table moves in. The mend benefit
is real (each method drops a `pub`); the line-count of `mod.rs` may
not fall as much as advertised.

**Ordering:** Phases 13–16 run last among the architectural phases.
Subsystems need their pane state already moved (Phase 12) so their
`handle()` bodies can mutate self plus update their own pane.

**Why Phase 12 must be done correctly first, not just "land first."**
`apply_service_signal`'s reaction body today calls
`push_service_unavailable_toast`, which writes both
`self.toasts.push_persistent(...)` and
`self.panes_mut().toasts_mut().viewport_mut().set_len(...)`. That
second write is what Phase 12 eliminates by moving the toasts
viewport into `ToastManager`. If Phase 12 lands without that
relocation, `HandlerCtx` for `ServiceSignal` subscribers must carry
both `&mut Toasts` AND `&mut Panes`, re-enacting through the ctx
struct the bus was supposed to remove. The bus only works if
Phase 12 has actually moved each pane's state into its data subsystem.

**Phase 13 mend reduction:** ~3 (skeleton + `apply_service_signal`).

**Risk:** highest of all phases — Phase 13 introduces the bus
skeleton; Phases 14–16 widen its subscriber surface incrementally.

## Phase 14 — `apply_lint_config_change` over `Bus<Event>`

Convert `apply_lint_config_change` (`async_tasks/config.rs:217-240`)
into one `Event::LintConfigChanged` with three subscribers (Inflight,
Scan, Selection). All three are already orchestrated through the
existing single method, so the conversion is mechanical: each
subscriber's `handle()` runs the body fragment that today lives
inline in `apply_lint_config_change`.

**Pre-requisite:** Phase 13's bus skeleton must compile cleanly.

**Expected mend reduction:** ~2.

**Risk:** low — mechanical conversion of an already-orchestrated method.

## Phase 15 — `apply_config` over `Bus<Event>`

Convert `apply_config` (`async_tasks/config.rs:138-197`) into
`Event::ConfigChanged(ConfigDiff)`. Six fan-out branches become six
subscribers (or fewer if related branches share a handler). Requires
designing `ConfigDiff` — what fields the diff carries, what fields
subscribers read.

**Pre-requisite:** Phases 13–14 stable (bus skeleton plus one widened
subscriber set).

**Special case — `force_settings_if_unconfigured` (`config.rs:183`):**
this is a *pane-mutating* side-effect inside the rescan branch. Under
Phase 15 it becomes `Command::ForceSettingsIfUnconfigured` — but this
command writes to `Overlays` AND `Panes` (settings viewport pos).
After Phase 12 it's clean (Overlays owns the settings viewport pos);
before Phase 12 it's the same multi-borrow it always was, which is
why Phase 12 → Phase 15 ordering is hard.

**Expected mend reduction:** ~6.

**Risk:** medium — `ConfigDiff` design is the load-bearing decision;
the rest is mechanical fan-out.

## Phase 16 — Startup-phase tracker + `StartupOrchestrator`

Add `StartupOrchestrator` at
`src/tui/app/async_tasks/startup_phase/orchestrator.rs` to publish
`StartupPhaseAdvanced(...)` events in the dependency order today
encoded in `maybe_log_startup_phase_completions`. Convert the 6
`maybe_complete_startup_*` family methods into subscribers that
publish their advancement events to the bus.

**Pre-requisite:** Phases 13–15 stable.

**Expected mend reduction:** ~4 plus the file relocation.

**Risk:** low — startup-phase logic is already a state-machine adapter;
the orchestrator extraction is structural rather than semantic.

### Phase 13 design depth

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
  step in Phase 13. Bus events carry `Arc<CargoPortConfig>` cheaply.
  Adds one sub-step to Phase 13.
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
  reactions in 6 different subsystems. After Phase 13, App's
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
`tick_once` or equivalent). After Phase 13, that call becomes:
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

## Stable intermediate state — between Phase 12 and Phase 13

After Phase 12 lands and before Phase 13 starts, the codebase is in a
defensible shipping state: clean subsystem ownership, every method
single-borrow against subsystems, no event bus. App still has its ~21
orchestrator surface — but every orchestrator is now thin glue, not a
body of work. Mend count lands around ~50–60.

This is a valid endpoint if Phase 13's bus introduction turns out to be
disruptive in ways the design depth doesn't model (subscriber-borrow
shape, drain-loop interaction with the existing `mutate_tree` guard,
`HandlerCtx` ergonomics). If Phase 13 (the architectural gate) reveals the
bus pattern doesn't fit, parking at this point ships ~85% of the
intended work without the architectural risk.

Recommend a deliberate pause after Phase 12 to evaluate before
committing to Phase 13.

## Loose ends — items not slated for any phase

Items the post-Phase-5 review surfaced that the existing 13 phases
don't cover. Each has been triaged into the closest phase or marked
as a Phase 14 candidate.

- **`Inflight` API audit.** `inflight: Inflight` already lives at
  `src/tui/inflight.rs` (outside `tui/app/`), so its accessors
  *should* be `pub(crate)`-free per Phase 4 lesson 1. No phase
  audits this. Check during Phase 10 Part C: count the
  `pub fn` items currently in `inflight.rs`; if any are `pub` rather
  than `pub(crate)`, narrow them in the same commit. ~2–4 likely.
- **`Background` and `LayoutCache` accessors.** Both are App fields.
  No phase touches them. Likely 1–3 mend warnings each; fold into
  Phase 10 Part A.
- **`status_flash` field** (App field set from
  `apply_lint_config_change`). Conceptually belongs with Toasts or
  Overlays. Defer until after Phase 13 — its move depends on whether
  Toasts becomes a bus subscriber or stays orchestrated.
- **`ensure_detail_cached` cache home.** Punted to "execution time"
  in Phase 12. Pick now: option (a) add a `DetailCache` type owned
  by App, alongside the other subsystems. The "per-pane wrappers as
  cache homes" path keeps 4 wrappers alive purely as cache containers,
  contradicting Phase 12's "drop wrapper types" thesis. **Decision:
  Phase 12 introduces `DetailCache`** at `src/tui/detail_cache.rs`
  (outside `tui/app/`), `pub(crate)` on its methods.

These together account for ~5–10 additional mend reductions
distributed across Phases 10 + 12. They do not change the canonical
sequence.

## Total expected impact

Updated post-Phase-2 with actuals and revised forward predictions. Phase
2's undershoot (-2 vs ~10–15 predicted) means future predictions for
visibility-narrowing phases should be read as *upper bounds*, not
expected values.

| Phase | Cluster | Reduction |
| ----- | ------- | --------- |
| 1   | Config (pub items only) | **-14 actual** (predicted -10) |
| 2   | Trivial subsystems (Keymap + Toasts + Scan/metadata) | **-2 actual** (predicted ~10–15) |
| 3   | Git/Repo extract → ProjectList | **-8 actual** (predicted ~5–9 — within range) |
| 4   | Ci (3 methods → ProjectList; 5 orchestrators relocated to `mod.rs`) + pulled-forward `app.config()` cleanup | **-9 actual** (predicted ~6–7 — overshoot from involuntary -1 on `mod.rs` rehosts) |
| 5   | Toast orchestrator relocation (orphan from Phase 2 surfaced by post-Phase-4 review) | **-11 actual** (predicted ~8–11 — hit upper bound; range methodology held) |
| 6   | Discovery shimmer + project predicates | **-11 actual** (predicted ~9–13 — within range, near upper bound) |
| Phase 7 | Mechanical `pub(super)` sweep on `async_tasks/*`, `navigation/*`, `dismiss.rs`, `types.rs`, `query/post_selection.rs`, `panes/*` | ~25–30 |
| 7   | Overlays subsystem at `tui/overlays.rs` (incl. Exit + inline_error) | ~14–20 |
| 8   | *Absorbed into Phase 11* (path resolution; was ~12–16) | n/a |
| 9   | Focus subsystem at `tui/focus.rs` (runs **before** Phase 9 per post-Phase-6 review) | ~12–18 |
| 10  | Internal-helper tightening (Part A residue ~5–10 after Phases 7 + 8 + 9) + relocate `CiFetchTracker` (Part B ~4) + `query/*` empty-file sweep (Part C ~1–2 since most files already deleted) | ~10–16 |
| **Visibility subtotal (Phases Phase 7–10)** | | **~61–84** (forward only; 1+2+3+4+5+6 already in done row) |
| 11  | Move `Viewport.pos` to `Selection.cursor` (absorbs original Phase 8 movement mutators **and** new Phase 8 path resolution) | ~20–24 |
| 12  | Subsystems own pane state; drop wrapper types (Cluster B for Toasts+) | ~9 |
| 13  | `Bus<Event>` skeleton + `apply_service_signal` (Cluster A gate phase) | ~3 |
| 14  | `apply_lint_config_change` over bus | ~2 |
| 15  | `apply_config` over bus (introduces `ConfigDiff`) | ~6 |
| 16  | Startup-phase tracker + `StartupOrchestrator` | ~4 |
| **Architectural subtotal (Phases 11–16)** | | **~39–48** (Phase 11 widened to absorb path resolution) |
| **Forward grand total (Phases Phase 7–16)** | | **~100–132** |
| **Done (Phases 1+2+3+4+5+6)** | | **-55 (147 → 92)** |
| **Stable intermediate (after Phase 12, before Phase 13)** | | residual ~50–60 mend warnings |

> **Caveat on the upper bound:** the upper of ~142 exceeds the 131
> remaining warnings. Treat the upper bound as theoretical. The lower
> bound (~112) is the realistic projection when Phase 2 lesson 1 is
> applied (count `pub` removed *minus* `pub` added). Phase 1's
> multiplier effect (where removing one `pub` cleared multiple secondary
> warnings) was specific to Phase 1's call-graph topology and did not
> repeat in Phase 2 — so don't budget for it.

After Phase 13, App is down to ~10–12 methods: `new`, `run`, top-level
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
the 12 selection mutators absorbed into Phase 11, the apply_service_signal cluster, and
`tabbable_panes`).

These ~55–58 are App's actual contract surface — every one of them touches
≥2 subsystems, which is the correct location for an orchestrator method.

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
