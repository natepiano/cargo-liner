# Reduce App's method footprint

## Status

Not started. Successor to `app-pass-through-collapse.md`, which
finished phases 1–22 with App still carrying **308 methods**
across 28 files (137 of them in `tui/app/mod.rs` alone). The
prior plan was named for collapse but executed only the
prerequisite step (extract subsystems and route App's methods
through them); the second half (rewrite call sites, delete the
shims, and clean up state ownership) was never scheduled. This
plan does that step and the structural cleanup the prior plan
should have done in its early phases.

## Goal

Drop App's method count from **308 → ~156** (per team-agent
classification of all 308 methods + subagent review
corrections + threading `include_non_rust` through the
visibility-cache helpers + relocating Group W static helpers
to their data owners, executed across 11 phases sized by
call-site rewrite count) through four parallel cleanups:

1. **Trivial-accessor deletion (universal).** Every accessor whose body is `&self.X` /
   `&mut self.X` / a single-call delegation gets deleted; the field becomes `pub(super)`;
   callers reach the field directly. Applied at every nesting depth — App, subsystems,
   nested types.
2. **Pass-through collapse.** Every App method that exists only to forward to a subsystem
   gets deleted; callers rewrite to `app.subsystem.method()`.
3. **State-ownership corrections.** Two structural changes the prior plan should have
   made: lift `ProjectList` to App (1), and merge `Selection` into `ProjectList` so
   project-list pane state lives with the project tree it describes (2).

## Decision rule (universal)

**Trivial accessor pairs are not OOP we want — anywhere in the
crate.** If a method's body is `&self.X` or `&mut self.X` or a
single-call pass-through (`self.x.y()`), the field replaces it.
The rule applies at every level:

- `App::scan()` returning `&self.scan` — delete; field goes `pub(super)`.
- `Scan::scan_state_mut()` returning `&mut self.state` — delete; field goes `pub(super)`.
- `Github::availability_mut()` returning `&mut self.availability` — delete; field goes
  `pub(super)`.
- And so on, at every nesting depth.

Methods that do anything else stay:

- Composition, multi-field read, derived computation (`selected_row_owns_lint` reads
  Selection + Lint + Config and returns a bool — keeps).
- Wrapping logic on top of a primitive (`WatchedFile::current()` delegates into a generic
  — keeps; the primitive earns its existence via reuse across `Config` and `Keymap`).
- Cross-subsystem orchestrators (`apply_config`, `rescan`, `show_timed_toast` —
  multi-subsystem state changes — keep).

## Why the prior plan stopped short

Five drift modes documented after Phase 22's audit:

1. **Self-contained per-phase definition of done.** Each phase shipped on "state moved into
   subsystem; routed through it." That passed tests and looked clean per phase, but the
   App-method-count delta wasn't tracked, so the gap was invisible.
2. **Retrospectives measured the wrong thing.** Outcome / Producer / Consumer / Lessons —
   none tracked App's method count delta. Adding that one line per retrospective would
   have surfaced the drift by Phase 5.
3. **The "End state" paragraph in the prior plan rationalized what was actually drift.**
   "Cross-subsystem reactions are named methods on App" became cover for leaving
   pass-throughs in place that were not cross-subsystem.
4. **No feedback loop between phases.** "What's the next phase?" / "yes, keep going" did
   not include a "are we still on track for the headline goal" check.
5. **`Cross-subsystem orchestrator on App` ratcheted up.** Once the pattern was named,
   borderline cases got classified into it instead of asking whether the caller could be
   moved closer to the data.

This plan's correction: a single up-front classification of every
App method, a universal rule applied recursively, and a
script-driven count delta in every retrospective.

## Pre-flight: classifier-bug lesson

The first draft of this plan listed five "dead" methods with
zero callers. They weren't dead — `cargo clippy --workspace
--all-targets --all-features -- -D warnings` is green and dead
code is a clippy error. The classifier had a bug: it required
the caller line to contain `self.` or `app.` literally, so
rustfmt-wrapped chains like

```rust
self
    .discovery_parent_row(session_path)
```

were not counted as callers. The corrected counter looks for
`\.<name>\(` across all `.rs` files, excludes the def line, and
strips comment lines. **No method on App is dead** — clippy
already covered that.

The 3 tool (`scripts/count_app_methods.py`) ships the corrected
classifier so future audits don't repeat this bug.

## Classification framework

Every App method falls into one of five groups:

- **Group A — DELETE, publish fields `pub(super)`.** Trivial subsystem-handle accessors
  (`config()`, `lint()`, …) and any other `&self.X` / `&mut self.X` body. Make the field
  `pub(super)`; rewrite callers; delete the accessor. Applied recursively into subsystems.
- **Group B — INLINE-AT-CALLER.** Single caller; inline the body at the call site, then
  delete.
- **Group C — REWRITE TO `app.subsystem.method()`** (2–5 callers each). Rewrite each
  caller to skip the App shim, then delete.
- **Group D — REWRITE, HIGH IMPACT** (6+ callers). Same mechanic as C, more call sites.
- **Group E — KEEP (genuine orchestrators).** Compose two or more subsystems with no
  single owner. Examples: `apply_config`, `rescan`, `apply_lint_config_change`,
  `maybe_complete_startup_*`, `force_settings_if_unconfigured`, `handle_bg_msg`,
  `request_clean_confirm`, `show_timed_toast` (touches Toasts + Config + viewport).

**Caveat.** The first-pass classifier put methods in Groups B/C/D
by caller count and Group E by body length. That's coarse —
some Group D entries are genuine orchestrators
(`show_timed_toast` has 22 callers but is a real composition),
and some Group E entries are misclassified single-subsystem
methods that should relocate. A pre-Phase-8 deep-dive task
re-classifies all 308 methods by what the body actually does;
the per-group counts in this doc update from its output.

## Target metric (team-agent classification)

Six agents in parallel hand-classified all 308 App methods by
reading each body in source. Final counts:

| Category | Count | Phase | Outcome |
| --- | ---: | --- | --- |
| T — trivial accessor (`&self.X`) | 28 | 3 | delete; field `pub(super)` |
| P — pass-through (single subsystem after 1/2) | 65 | 3 | delete; callers go direct |
| S — single-subsystem orchestrator | 64 | 8 | **relocate to owning subsystem** |
| X — cross-subsystem orchestrator | 102 | — | keep on App |
| H — handler/dispatcher | 22 | — | keep on App |
| W — wrapping logic / static helpers / App-local | 27 | — | keep on App |
| **Total** | **308** | | |

**Final App method count target: ~156.** Math: phase-by-phase sum from
the summary table: 2 + 13 + 20 + 9 + 3 + 2 + 10 + 2 + 2 + 17 + 27 + 18 + 5
+ 23 = **153 removed** across Phases 1, 3–13 (App-side reductions) plus
Phase 14 (5 App-local accessors) and Phase 15 (23 static-helper relocations).
**308 − 153 ≈ 156.** Phases 2 and 14's crate-wide pass don't reduce App's
count directly; Phase 2 preps for 11/12 and Phase 14 cleans trivial
accessors crate-wide.

**Where the reduction concentrates.** The ~153 removals:
- **~88** trivial accessors / pass-throughs deleted (trivial-accessor + pass-through) across Phases 3–9.
- **64** single-subsystem orchestrators relocated.

S relocation destinations:

| Destination | Methods |
| --- | ---: |
| → `project_list` (post-Phase-2: tree + nav + selection) | 46 |
| → `startup` | 5 |
| → `toasts` | 4 |
| → `scan` | 4 |
| → `net` | 2 |
| → `background` | 1 |
| → `inflight` | 1 |
| → `ci` | 1 |

The `project_list` destination dominates: 46 of 64 relocations
move into the merged ProjectList. Methods like `path_for_row`,
`display_path_for_row`, `expand_key_for_row`,
`abs_path_for_row`, `dismiss_target_for_row_inner`,
`collapse_row`, `build_selected_pane_data`, `expand_all`,
`collapse_all`, `select_project_in_tree` — they only touch the
project tree + selection state, which after Phase 2 are the same
struct.

**The remaining ~156 are real cross-subsystem work.** Composers
like `apply_config`, `rescan`, `handle_bg_msg`, the
`maybe_complete_startup_*` family, `prune_toasts`, the toast
helpers (`show_timed_toast`, `start_task_toast`,
`finish_task_toast`), and the row-action methods
(`enter_action`, `dismiss`, `sync_selected_project`) all
genuinely compose multiple subsystems and have no single
owner.

**Hand-verified, not heuristic.** Each agent read the actual
method body in source, traced helper calls one level, and
chose the category. Spot-corrections vs. the prior heuristic:
~30 methods reclassified (mostly W → P after recognizing
post-Phase-2 destinations, and H → S where helper traces resolved
to single-subsystem).

## Phase plan

Each phase is one collapse mechanic applied to one scope, in
order of structural risk (low → high) and visibility (each phase
moves the headline number). Every phase ends with a
**count-delta line in its retrospective**, generated by
`scripts/count_app_methods.py` — the per-phase correction for
the prior plan's missing feedback loop.

### Phase 1 — Lift `ProjectList` from `Scan` to `App` ✅

`projects: ProjectList` lives on `Scan` today. But `ProjectList`
is the central per-project data store of the whole app: lint
runs, CI info, git info, language stats, package/workspace
fields, disk usage — all live inside the tree. Every subsystem
that produces per-project data writes into it. Scan is one
writer among many; its privilege is producing the initial
tree, not owning the data.

Phase 2 of the prior plan put `projects` inside `Scan` because
Scan was the subsystem being extracted. That was the wrong
owner.

**Steps.**

1. Add `projects: ProjectList` as a direct field on `App`. Initialize from
   `AppBuilder::Started.projects` in `construct.rs`.
2. Delete `Scan::projects` field, `Scan::projects()`, `Scan::projects_mut()`. Update
   `Scan::new()` to drop the projects parameter.
3. Update `App::TreeMutation` to borrow `&mut ProjectList + &mut Panes + &mut Selection`
   instead of `&mut Scan + ...`.
4. Per-project mutators across `async_tasks/*` switch from
   `self.scan.projects_mut().X` to `self.projects.X` — one layer drops.

**What stays on Scan.** The scan-cluster machinery: `state`
(ScanState), `dirty` (DirtyState), `data_generation`,
`discovery_shimmers`, `pending_git_first_commit`,
`metadata_store`, `target_dir_index`, `priority_fetch_path`,
`confirm_verifying`, `retry_spawn_mode`. Coordination state,
not project data.

#### Retrospective

**What worked:**
- All 4 listed steps applied as written. `App::projects` field added; `Scan::projects` field, `Scan::projects()`, `Scan::projects_mut()`, and the `projects` parameter on `Scan::new()` deleted; `TreeMutation` now borrows `&mut ProjectList + &mut Panes + &mut Selection`.
- `App::projects()` / `App::projects_mut()` accessors retained as one-line shims (`&self.projects` / `&mut self.projects`) so non-`scan` callers were untouched. The headline-count drop for those is Phase 10.
- 599 tests pass; clippy clean.

**What deviated from the plan:**
- The plan listed 4 caller-rewrite locations (`async_tasks/*`); the actual surface was wider: `src/tui/app/ci.rs`, `src/tui/panes/actions.rs`, `src/tui/panes/lang.rs`, plus the render plumbing. `panes/lang.rs` and the `PaneRenderCtx` / `DispatchArgs` chain held a `scan: &Scan` field whose only use was `scan.projects()`.
- Renamed that render-context field from `scan` to `projects` (`&'a ProjectList`) in `tui/pane/dispatch.rs`, `tui/panes/system.rs`, `tui/render.rs`, `tui/interaction.rs`, `tui/panes/lang.rs`, `tui/panes/package.rs`. `split_ci_for_render`, `split_lint_for_render`, `split_panes_for_render` now hand out `&ProjectList` instead of `&Scan`.

**Surprises:**
- The render path was the largest borrow-of-`projects-via-Scan` consumer, not `async_tasks/*`. The plan named the scan-cluster mutators but missed the render plumbing entirely.
- `panes/actions.rs` used `app.scan_mut().projects_mut()` — caller-side `App::scan_mut().projects_mut()` chains existed outside the `self.scan.projects_mut()` pattern and were not enumerated by the plan.

**Implications for remaining phases:**
- Phase 2 (Selection → ProjectList merge) inherits the new `App::projects` field directly; the rename to `project_list` is now a single-field rename instead of a field-plus-restructure.
- Phase 10 (delete `App::projects()` / `App::projects_mut()`) caller count was estimated at ~304; live counts post-Phase-1 are 250 + 25 = ~275. The render-context rename plus the wider-than-planned async_tasks sweep dropped roughly 30 call sites. Phase 10 numbers updated.
- The `&Scan` → `&ProjectList` rename in render contexts revealed that the only render consumer of `Scan` (lang.rs) was actually a project-list consumer. No remaining render-path code reads from `Scan` directly. This means future phases that touch `Scan` accessors don't need to reach into the render plumbing.

#### Phase 1 Review

- Phase 2 step 4 — drop the `&Selection` slot from `App::split_panes_for_render`'s return tuple at the same time `Selection` is deleted (already bound `_selection` and unused).
- Phase 2 step 5 — rename now also covers `PaneRenderCtx::projects`, `DispatchArgs::projects`, and locals named `projects` in `dispatch_via_trait` / `render_lints_pane` / `render_ci_pane` (introduced by Phase 1's render-context rename).
- Phase 2 borrow-checker note — reworded to reflect that `TreeMutation` already borrows `&mut ProjectList + &mut Panes + &mut Selection` post-Phase-1; Phase 2's actual change is dropping the `&mut Selection` slot.
- Phase 8 (Scan trivial-accessor / pass-through delete; was Phase 5.4 pre-resequence) — caller estimate trimmed to ~40–50 with explicit note that no `tui/render.rs` or `tui/panes/*` touches are needed.
- Phase 10 — caller-count updated from ~304 to ~275 (live: 250 + 25); also added explicit "depends on Phase 2 step 5" sequencing note to guard against the `selected_project_path_for_render` call path breaking if Phase 10 lands before the field rename.
- Phase 11 — added render-path note about `selected_project_path_for_render`: post-Phase-2 it's a pure `ProjectList` query, so render.rs can either keep the App shim or invert order (split-borrow first, then call on `&ProjectList`).
- Phase 15 — ordering note expanded to include Phase 2 dependency (field rename), not just Phase 10.

### Phase 2 — Merge Selection into ProjectList ✅

`Selection` is named for cross-pane state but every field is
project-list navigation: `cursor`, `expanded`,
`finder`, `cached_visible_rows`, `cached_root_sorted`,
`cached_child_sorted`, `cached_fit_widths`, `paths`, `sync`. It
only makes sense paired with the project tree. Other panes hold
their pane state inside `Panes` or on their own subsystems.
`Selection` was extracted as a separate App field because the
prior plan kept `ProjectList` as a non-TUI data type and didn't
want to couple it to TUI navigation state.

Phase 2 fixes that. `ProjectList` becomes a TUI module, absorbs
Selection's state, replaces both App fields.

**Steps.**

1. Move `src/project_list.rs` → `src/tui/project_list.rs`. Update import paths.
2. Add Selection's fields and methods to `impl ProjectList`.
3. Move `SelectionMutation` (RAII guard for visibility-changing operations) to the new
   location; its `Drop` recompute now operates on `self` directly.
4. Delete the `Selection` struct and the `selection: Selection` field on App. Drop
   the `&Selection` slot from `App::split_panes_for_render`'s return tuple at the
   same time (it's currently bound as `_selection` and unused; leaving it in place
   would not compile after `Selection` is deleted).
5. Rename App's `projects` field to `project_list` (now reflects the absorbed scope).
   **Pause and ask the user to perform this rename via the editor's global rename
   feature** (per CLAUDE.md: editor rename is faster and more accurate than mechanical
   substitution for type/field renames). The rename also covers `PaneRenderCtx::projects`
   (in `src/tui/pane/dispatch.rs`) and `DispatchArgs::projects` (in `src/tui/panes/system.rs`)
   plus the locals named `projects` in `dispatch_via_trait` / `render_lints_pane` /
   `render_ci_pane` — these were introduced by Phase 1's render-context rename and should
   move in lockstep with the App field.
6. Rewrite all callers: `app.selection.X` / `app.projects.X` → `app.project_list.X`.
   The user-driven rename in step 5 handles the field-name rewrite; this step covers the
   selection-side merge that the editor rename can't see (where today's `app.selection.X`
   becomes `app.project_list.X`).

**Borrow-checker note.** `TreeMutation` already borrows `&mut ProjectList + &mut Panes
+ &mut Selection` post-Phase-1. Phase 2's actual change is dropping the `&mut Selection`
slot once Selection's fields/methods migrate into ProjectList — one fewer borrow in the
fan-out, the rest of the TreeMutation surface is already in place.

#### Retrospective

**What worked:**
- All 6 listed steps applied. `src/project_list.rs` moved to `src/tui/project_list.rs`; `Selection` struct deleted; `selection: Selection` field deleted from `App`; the `&Selection` slot dropped from `split_panes_for_render`; `App.projects` field renamed to `App.project_list` (user-driven editor rename); `PaneRenderCtx::projects` and `DispatchArgs::projects` renamed to `project_list` in lockstep; all `app.selection.X` and `self.selection.X` rewritten to project_list.
- 599 tests pass; clippy clean; `cargo install` smoke-test green.
- `App::projects()` / `App::projects_mut()` accessor methods retained intact (slated for deletion in Phase 10) — Phase 2 made zero touches to ~330 caller call sites of those methods, keeping the diff bounded.
- `TreeMutation` now borrows `&mut ProjectList + &mut Panes` (slot dropped); `Drop` calls `self.projects.recompute_visibility(self.include_non_rust)` directly, no Selection middleman.

**What deviated from the plan:**
- The plan's step 2 spec ("Add Selection's fields and methods to `impl ProjectList`") collided with `ProjectList::visible_rows(&HashSet<ExpandKey>, bool) -> Vec<VisibleRow>` (the recomputer) vs `Selection::visible_rows() -> &[VisibleRow]` (the cached accessor). Resolved by renaming the recomputer to `compute_visible_rows`; the cached accessor kept its `visible_rows()` name to minimize caller churn (only 3 sites needed the `compute_*` rename).
- Two new methods on `ProjectList` not predicted by the plan: `iter_with_expanded_mut` (split-borrow over `roots.values()` and `&mut expanded` for bulk-expand paths in `navigation/bulk.rs` and `async_tasks/tree.rs`); `replace_roots_from(replacement)` to preserve selection-cluster state on whole-tree replacement (used by `App::set_projects` test-only and `TreeMutation::replace_all`). Without `replace_roots_from`, `set_projects` regressed `expanded_tree_rebuild_refreshes_clickable_rows` (cursor + expanded got nuked).
- One new method on `ProjectList`: `init_runtime_state(lint_enabled)` — production-only side-effecting seed for `paths` (loads last-selected from terminal-state file) and `cached_fit_widths`. Tests use `ProjectList::default()` / `ProjectList::new(items)` with `..Self::default()` FRU to skip the side effects.
- `FinderState` gained `#[derive(Default)]`; the unused `FinderState::new()` constructor was deleted.
- Per a feedback rule surfaced mid-phase (`feedback_no_pub_crate`), every method I added to `ProjectList` uses `pub(super)`, not `pub(crate)`. Pre-existing `pub(crate)` methods on `ProjectList` (`new`, `len`, `is_empty`, `iter`, `compute_visible_rows`, etc.) were left as-is — opportunistic visibility cleanup is not in Phase 2 scope.
- `bulk.rs::expand_path_in_tree` and `async_tasks::tree::migrate_legacy_root_expansions` had to be restructured to handle the new "iter and mutate same struct" collision. `expand_path_in_tree` switched to `iter_with_expanded_mut`. `migrate_legacy_root_expansions` snapshots `(idx, &RootItem)` pairs first, then iterates the snapshot while mutating `expanded` — the original chained `find` over a live iterator is no longer borrow-compatible.

**Surprises:**
- `set_projects` (test-only) and `TreeMutation::replace_all` were the two whole-tree-replacement paths whose semantics quietly relied on `Selection` being a separate field. The pre-merge code did `*self.projects = projects;` and Selection state survived because Selection was at a different `App` field address. After the merge, the same statement zeroes selection state. The plan named `TreeMutation`'s borrow tuple but did not flag this semantic regression. `replace_roots_from` is the targeted fix.
- `expand_path_in_tree`'s borrow pattern (`let Self { projects, selection, .. } = self;` followed by simultaneous `projects.iter()` + `selection.expanded_mut()`) was load-bearing on Selection-as-separate-field. Once merged, `iter_with_expanded_mut` is the equivalent split-borrow. Phase 2's plan called this out generically as the borrow-checker note but didn't show the pattern callers had built on top of.
- The plan estimated ~150 caller rewrites in `src/` plus ~50 in `tests/`. Live diff: 38 files, +536/-316. The actual rewrite happened almost entirely through the editor rename of one App field (which also resolved `PaneRenderCtx`/`DispatchArgs` and dispatch-side locals), plus targeted edits to the few sites where `selection.X` had to merge with `projects.X`. The plan over-estimated the caller-rewrite cost.
- `app.selection()` / `app.selection_mut()` accessor methods on `App` (returning `&Selection` / `&mut Selection`) had to be deleted along with the field — they don't exist as a target for the editor rename. Mechanical sed `app.selection_mut()` → `app.projects_mut()` and `app.selection()` → `app.projects()` covered every caller.

**Implications for remaining phases:**
- Phase 10 (delete `App::projects()` / `App::projects_mut()`) call sites are unchanged — Phase 2 did not touch them. The post-Phase-2 caller count is the same ~275 the plan currently records. No revision needed.
- Phase 11 (`project_list` absorption I — row-navigation): the absorption target was already proven by Phase 2's bulk migration. `App::row_count()`, `App::cursor()`, `App::set_cursor()`, `App::move_*` are now thin pass-throughs to identically-named methods on `ProjectList` — when Phase 11 deletes them, callers can switch to `app.project_list.X` mechanically without behavioral risk.
- Phase 12 (`project_list` absorption II — action methods) inherits the new `iter_with_expanded_mut` pattern. If any of the action-method moves want to walk projects while mutating expansion, the helper is already in place.
- Phase 15 (Group W static helpers → data owners): `replace_roots_from` is a candidate static-helper-on-data-owner that isn't in the Phase 15 enumeration. If Phase 15 cataloged static helpers by-eye from Group W, it likely missed this one. Worth reviewing.
- The decision to keep `pub(crate)` on pre-existing `ProjectList` methods rather than tightening to `pub(super)` is a Phase 14 concern (recursive trivial-accessor purge inside subsystems). Phase 14 should call out the visibility tightening explicitly so the cleanup is recorded, not silently dropped.

#### Phase 2 Review

- Phase 11/Phase 12 — original Phase 11 listed `expand_path_in_tree`, `select_matching_visible_row`, `clean_selection`, `select_root_row` which Phase 12 also enumerated. Reconciled: Phase 11 now scopes to row-navigation **read-side** queries only (`path_for_row`, `selected_project_path`, `selected_row`, etc., 16 methods); the mutating/expansion-affecting/`include_non_rust`-threaded methods all moved to Phase 12 (28 methods). Sub-commit framing rejected — both stay as full phases per the no-sub-commits rule.
- Phase 12 — added explicit pointer to `ProjectList::iter_with_expanded_mut` as the split-borrow pattern relocated methods should reuse. Added note to re-run `count_app_methods.py` at phase start because Phase 2's caller absorption may have lowered the live rewrite count.
- Phase 14 — added explicit "Visibility tightening on relocated types" subsection enumerating the ~38 pre-existing `pub(crate)` methods on `ProjectList` slated for `pub(super)` tightening (zero caller rewrites — narrowing only).
- Phase 15 — added "Already-resident helpers (no Phase 15 action)" subsection naming `ProjectList::replace_roots_from` so future passes don't relitigate moving it (already lives on the data owner; called by `App::set_projects` and `TreeMutation::replace_all`).
- Phase summary table — Phase 11/8 row-counts updated to reflect dedup (Phase 11: ~16/~80, Phase 12: ~28/~170; "App after" columns adjusted to 231/203 respectively).
- Group X line for `split_panes_for_render` updated to reflect the now-4-tuple return signature (selection slot dropped in Phase 2 step 4).
- No findings rejected.

### Phase 3 — Tooling + small-subsystem trivial-accessor / pass-through delete ✅

Two pieces:

1. **Ship `scripts/count_app_methods.py`** — the corrected classifier with the helper-resolution
   table. Output: total App method count, per-category breakdown, S relocation list. Single
   command, <1s. Every retrospective uses this.
2. **Delete trivial-accessor / pass-through for Config, Keymap, LayoutCache.** Publish `app.config`, `app.keymap`,
   `app.layout_cache` as `pub(super)`. Delete the trivial accessors and short pass-throughs
   (`config()`, `config_mut()`, `current_config()`, `current_config_mut()`, `config_path()`,
   `keymap()`, `keymap_mut()`, `layout_cache()`, `layout_cache_mut()`, `settings_edit_*`).
   Rewrite call sites.

**Methods removed:** ~12. **Caller rewrites:** ~100.

Picked first because Config/Keymap/LayoutCache have low fanout — smoke-tests the
mechanic (publish + rewrite + delete + validate + count delta) before higher-traffic
subsystems.

#### Retrospective

**What worked:**
- `scripts/count_app_methods.py` shipped at ~80 lines, runs <1s, gives total + per-file table. Counted 306 pre-Phase-3 → 293 post-Phase-3 (delta −13, matching predicted ~12).
- The publish-field + perl-bulk-rewrite + delete-accessor mechanic is mechanical: one perl pass rewrote 13 method calls across 16 files, then deletions surfaced as dead-code warnings, which guided where to cut.

**What deviated from the plan:**
- 13 methods deleted, not 12: `config()`, `config_mut()`, `current_config()`, `current_config_mut()`, `config_path()`, `keymap()`, `keymap_mut()`, `layout_cache()`, `layout_cache_mut()`, `settings_edit_buf`, `settings_edit_cursor`, `settings_edit_parts_mut`, `set_settings_edit_state`. Plan's `~12` was a rough estimate; `settings_edit_*` is 4 methods and the plan pluralized it as one bullet.
- One incidental rewrite outside the plan: `*app.layout_cache_mut() = LayoutCache::default();` collapsed to `app.layout_cache = LayoutCache::default();` — the dereference is no longer needed once the field is direct.

**Surprises:**
- The `.config_mut()` pattern was test-only (`#[cfg(test)]`). The plan listed it among prod accessors. With the field public, tests just write `&mut app.config` directly — no `cfg(test)` accessor needed.
- `python3 scripts/count_app_methods.py` reports 293, not ~156 yet — Phase 3 is one of thirteen reduction phases. The headline target reduction lands across Phases 4–15.

**Implications for remaining phases:**
- The publish + bulk-rewrite + delete mechanic worked clean on a low-fanout subsystem. Phases 4–5 can reuse the exact perl pattern; bigger fanout means longer regex but identical mechanic.
- Tests-only mut accessors (`config_mut`, future `scan_state_mut`, `background_mut`, etc.) get deleted entirely once their field is `pub(super)` — tests reach via field directly. No `#[cfg(test)]` shim needed.
- `count_app_methods.py` is now the single source of truth for the headline number per phase. Future retrospectives report `pre-N → post-N (delta)` from this script.

#### Phase 3 Review

- Phase 5 (was trivial-accessor / pass-through delete: Panes/Focus/Overlays/Scan/Startup as one phase with sub-commits 5.1–5.5) split into five separately-numbered phases (Panes/Focus/Overlays/Scan/Startup = Phases 5–9) per the no-sub-commits rule. Old Phases 6–11 renumbered to 10–15. Summary table and all cross-references updated.
- Phase 13 scan list reconciled with inventory: `handle_git_first_commit`, `should_verify_before_clean`, `handle_out_of_tree_target_size`, `handle_repo_meta`. `clean_metadata_dispatch` and `update_generations_for_msg` stay on App (Group X).
- Phase 13 toasts list replaced with inventory's 4 specific methods: `push_service_unavailable_toast`, `start_task_toast`, `mark_tracked_item_completed`, `focused_toast_id`. `dismiss_keymap_diagnostics` stays on App (Group X).
- Phase 13 background list replaced: `register_item_background_services` (S relocation) instead of `finish_watcher_registration_batch` (P-shim handled in trivial-accessor / pass-through sweep). Phase 13 method count 17 → 18.
- Phase 12 dropped `register_existing_projects` (Group X — touches `project_list` and `background`); count 28 → 27.
- Phase 5 (Panes trivial-accessor / pass-through) gained `poll_cpu_if_due`; count ~8 → ~9. (`apply_hovered_pane_row` was already excluded as Group X.)
- Phase 11 (project_list absorption I) gained `last_selected_path` (single-subsystem read); count ~16 → ~17.
- Phase 14 gained an "App-local trivial accessors" subsection enumerating `mouse_pos`/`set_mouse_pos`, `animation_elapsed`, `toast_timeout`, `resolved_dirs` (5 App-local removals). Final App count target adjusted: 161 → ~156.
- Summary table caller-rewrite columns updated: Phase 6 estimate 304 → 275 (live: 250 + 25); Phase 9 method count 18 → 17; Phase 3 row marked ✅ with measured delta.
- trivial-accessor + pass-through table heading rescoped from "Phase 3 deletion list" to "deletion list (Phases 3–9)"; 13 Phase-3-completed rows marked with ✅.

### Phase 4 — Medium-subsystem trivial-accessor / pass-through delete (Lint, Ci, Toasts, Net, Background, Inflight) ✅

Publish each subsystem as `pub(super)`. Delete trivial accessors and pass-throughs:
`lint()`/`_mut`, `ci()`/`_mut`, `toasts()`/`_mut`, `net()`, `inflight()`,
`background_mut()`, plus their pass-throughs (`bg_tx`, `ci_fetch_tx`, `clean_tx`,
`example_tx`, `http_client`, `rate_limit`, `github_status`, `repo_fetch_cache`,
`example_*`, `pending_cleans_mut`, set/take pending fetch helpers).

**Methods removed:** ~20. **Caller rewrites:** ~250.

#### Retrospective

**What worked:**
- Publish-field + perl-bulk-rewrite + delete-accessor mechanic transferred cleanly from Phase 3. 28 App methods removed (293 → 265, delta −28); plan predicted ~20 — undercount because pass-throughs (`set_pending_*`, `take_pending_*`, `set_ci_fetch_toast`, `pending_cleans_mut`, `example_output_mut`) were collapsed into "set/take pending fetch helpers" / "example_*" bullets in the plan.
- Diagnostic-driven iteration worked: each `cargo check` round surfaced the next batch of stale references, including two `&dyn Hittable` arms in `interaction.rs:66,76` that needed `&app.toasts` / `&app.lint` / `&app.ci` (field access loses the auto-borrow that the accessor provided).

**What deviated from the plan:**
- Pass-through bulk-replace over-applied across non-App types whose own methods share the same name. `\.rate_limit\(\)` matched HttpClient::rate_limit and Net::rate_limit bodies (`self.http_client.rate_limit()` → `self.http_client.net.rate_limit()`); `\.example_*\(\)` matched Inflight test bodies (`inflight.example_running()` → `inflight.inflight.example_running()`); `self.net.X()` and `self.inflight.X()` already-correct call sites in `app/async_tasks/*.rs` got prefixed twice. Required a follow-up perl pass to revert `\.net\.net\.` / `\.inflight\.inflight\.` / `inflight.inflight.` / `client.net.X()` / `self.http_client.net.rate_limit()`.
- Field-access loses auto-borrow at trait-object coercion sites: `app.toasts` (a `ToastManager` value) does not coerce to `&dyn Hittable` the way `app.toasts()` (returning `&ToastManager`) did. Needed manual `&app.toasts` / `&app.lint` / `&app.ci` at three sites in `interaction.rs`.
- `set_ci_fetch_toast` could not be done via mechanical regex — it wraps the arg in `Some(...)`. Single caller rewritten by hand: `app.set_ci_fetch_toast(task_id)` → `app.ci.set_fetch_toast(Some(task_id))`.
- 8 unused imports surfaced after the deletions (`VecDeque`, `GitHubRateLimit`, `RepoCache`, `PendingCiFetch`, `PendingExampleRun`, `CiFetchMsg`, `CleanMsg`, `ExampleMsg`). All removed.

**Surprises:**
- The naming overlap between App-side accessor and underlying-type method (Net::rate_limit, HttpClient::rate_limit, Inflight::example_running) is real and breaks naive bulk regex. Future phases with similar overlap (Panes::pane_data overlaps with App::pane_data?) need the regex scoped to App-side call patterns or the over-replacement reverted in a second pass.
- The `inflight` test module uses a variable literally named `inflight`, so even `\binflight\.X\(` patterns hit. Rust's lack of method-call AST for a regex tool means revert-after-the-fact is the practical approach.

**Implications for remaining phases:**
- Phase 5 (Panes) — `panes` is a common substring; `pane_data`, `panes_mut`, `set_hovered_pane_row` are App-side names that may or may not collide with method names on the `Panes` type itself. Pre-flight: enumerate Panes' own methods before running the regex; expect a revert pass.
- Phase 6 (Focus) — `focus()`, `focus_mut()`, `focused_pane()` — `Focus::focused_pane` likely exists. Same revert-pass risk.
- Phase 7 (Overlays) — `overlays()`/`_mut()` — high call-site count (~130) but the names are unlikely to collide with non-App types.
- The clean run of test + clippy + install confirmed Phase 4's net effect: −28 App methods, no behavior change. End-state 265 is 9 below the plan's 274 estimate; future targets adjust down by 9 unless re-estimated.

#### Phase 4 Review

- Phase 5 (Panes) gained a mandatory pre-flight name-collision check + revert pass: `Panes::pane_data` and `Panes::worktree_summary_or_compute` collide with App-side accessors. Trait-object-coercion grep also added.
- Phase 6 (Focus), Phase 7 (Overlays), Phase 9 (Startup) tagged as "no pre-flight collision check needed" — verified against subsystem method lists; those phases can run the bulk-replace mechanic without a revert pass.
- Phase 8 (Scan) gained a mandatory pre-flight + revert pass: `Scan::scan_state_mut`, `Scan::bump_generation`, `Scan::metadata_store`, `Scan::target_dir_index`, `Scan::priority_fetch_path`, `Scan::confirm_verifying`, `Scan::discovery_shimmers` all collide. Densest collision surface in the trivial-accessor / pass-through sweep.
- "Mechanics of a collapse step" section grew from 6 steps to 9: added step 2 (pre-flight collision grep), step 5 (revert double-prefix `\.X\.X\.` → `.X.`), step 7 (clean up orphaned imports). Also called out trait-object-coercion sites (auto-borrow lost when accessor → field) and arity-changing rewrites (`set_ci_fetch_toast(x)` → `ci.set_fetch_toast(Some(x))`).
- Phase 13 framing confirmed (no edit): with subsystems now public, S-relocation is uniformly "lift body as-is into impl OwningSubsystem" — no field-publish prereq remaining.
- Phase 14 App-local accessors confirmed live (no edit): `animation_elapsed`, `mouse_pos`, `toast_timeout`, `resolved_dirs` all still have prod callers; deletion + caller rewrite is real Phase 14 work.
- Phase 10 sequencing constraint unchanged (no edit): "must run after Phase 2" is the binding prereq, not field-publication. Phase 4's mechanic validation does help Phase 10's playbook but doesn't change ordering.
- End-state arithmetic confirmed (no edit): summary table predicts 265 → 147 across remaining App-side phases, matching the ~156 target within phase-estimate noise.

### Phase 5 — Panes trivial-accessor / pass-through delete ✅

Publish `panes` as `pub(super)`. Delete trivial accessors and pass-throughs:
`panes`/`_mut`, `pane_data`, `set_hovered_pane_row`, `worktree_summary_or_compute`,
`poll_cpu_if_due`. Rewrite call sites.

**Pre-flight name-collision check (mandatory):** `Panes::pane_data` (`src/tui/panes/system.rs:212`)
and `Panes::worktree_summary_or_compute` (line 222) collide with the App-side accessors
slated for deletion. The mechanical regex `\.pane_data\(\)` / `\.worktree_summary_or_compute\(`
will rewrite already-correct `self.panes.pane_data()` into `self.panes.panes.pane_data()`.
Run a revert pass after bulk-replace: `\.panes\.panes\.` → `.panes.`.

**Trait-object coercion sites:** Phase 4 hit `&dyn Hittable` arms in `tui/interaction.rs`
where the accessor's auto-borrow disappeared once the field went public. Grep for `&dyn`
patterns referencing `panes` before assuming the bulk-replace is complete.

**Methods removed:** ~9. **Caller rewrites:** ~120.

### Retrospective

**What worked:** Pre-flight collision check correctly identified `Panes::pane_data` and `Panes::worktree_summary_or_compute` as collision points. The 6-step mechanic (publish → bulk-replace → revert pass → delete → cleanup → validate) ran cleanly with one trait-object-coercion fix and three unrelated-method false positives.

**What deviated from the plan:** 6 methods removed (not ~9): `panes`, `panes_mut`, `pane_data`, `set_hovered_pane_row`, `worktree_summary_or_compute`, `poll_cpu_if_due`. The plan listed these correctly; the "~9" estimate was conservative.

**Surprises:**
- The bulk regex `\.panes\(\)` → `.panes` clobbered unrelated `ResolvedPaneLayout::panes()` at `render.rs:145` and `input.rs:279,345` — both were field-private method calls. Required surgical revert.
- The `\.panes\.panes\.` revert regex did NOT catch `panes.panes.pane_data()` in `panes/system.rs` tests because the leading `panes` had no `.` prefix. Needed a more specific revert regex `\bpanes\.panes\.pane_data\(\)` → `panes.pane_data()`.
- `interaction.rs:144` had `let panes = app.panes_mut();` — bulk regex turned it into `let panes = app.panes;` (a move, requiring Drop). Fix: `&mut app.panes`.
- `Panes::worktree_summary_or_compute` body called `self.git.worktree_summary_or_compute(...)`. Bulk regex turned it into `self.git.panes.worktree_summary_or_compute(...)`. Surgical revert needed because `\.git\.panes\.` is not a generic pattern.

**Implications for remaining phases:**
- Bulk-regex pattern `\.X\(\)` → `.field` is unsafe when X is a common method name across multiple types. For Phase 6 (Focus), Phase 7 (Overlays), Phase 8 (Scan), Phase 9 (Startup): grep for non-App callers of the same accessor name BEFORE running the regex. If any exist, surgically rewrite App-only sites instead of bulk-replacing.
- Revert regex `\.X\.X\.` only catches the case where the bad insertion is preceded by `.`. For tests/inner code where the variable is a bare identifier (e.g. `panes.X.Y`), use `\bvar\.X\.X` or do surgical fix.

### Phase 5 Review

- Phase 8 (Scan): dropped the unneeded find-and-replace cleanup directive; rewrote pre-flight to verify what was actually checked (no unrelated type has a `scan()`/`scan_mut()` method).
- Phase 8 → Phase 11: moved `set_projects` (test-only helper); body is project_list work, not Scan work. Phase 8 method count ~10 → ~9; Phase 11 gained one test-only delete.
- Phase 14: added `ResolvedPaneLayout::panes()` (`tui/pane/layout.rs`) and `Panes::worktree_summary_or_compute` (`panes/system.rs:222`) to the recursive purge enumeration.
- Phases 6/7/9: replaced "no pre-flight needed" with a concrete pre-flight (verify no unrelated type has a method by the same name) + grep for `let .* = .*\._mut()` binding sites.
- Phase 7: added explicit note that `&dyn Hittable` arms in `interaction.rs` are safe (underlying call still returns a reference, unlike Phase 4's `&app.toasts` case).
- Mechanics step 3: enumerated three rewrite categories — plain field access, trait-object coercion (need `&app.field`), `let`-binding from `_mut()` accessor (need `&mut app.field`).
- Mechanics step 5: added the no-leading-dot revert variant `\bsubsystem\.subsystem\.` for inner-scope sites where a local var shares the field name.

### Phase 6 — Focus trivial-accessor / pass-through delete ✅

Publish `focus` as `pub(super)`. Delete `focus`/`_mut`, `focused_pane`. Rewrite
call sites.

**Pre-flight check:** Verified `fn focus` is only defined on App; `Focus`'s
own methods (`current`, `set`) don't share names with the App-side accessors
slated for deletion. Also grep `let .* = .*\.focus_mut\(\)` to find any
`let`-binding sites that need explicit `&mut app.focus` after the bulk
rewrite (per Phase 5's `interaction.rs:144` lesson).

**Methods removed:** 3. **Caller rewrites:** ~93.

### Retrospective

**What worked:**
- Pre-flight grep for `let .* = .*\.focus_mut\(\)` returned zero hits — every
  `_mut` call site was an immediate method chain (`app.focus_mut().set(...)`),
  not a let-binding. No manual `&mut app.focus` rewrite needed.
- No name collisions on `Focus`: the type's methods (`current`, `set`,
  `pane_state`, `is`, etc.) don't overlap with the App-side accessors. Bulk
  perl rewrite landed clean — no double-prefix revert pass needed.
- Rewrite ordering mattered and worked as planned: `_mut` first, then
  `focused_pane` (pass-through, expanded inline), then `focus` last. Doing
  `focus` first would have over-replaced `focus_mut`.
- `focused_pane()` pass-through was inlined to `.focus.current()` in one bulk
  pass — no follow-up touchups needed.

**What deviated from the plan:**
- Caller rewrite count was ~93, not the estimated ~85 (42 `.focus()` + 35
  `.focus_mut()` + 16 `.focused_pane()`).

**Surprises:**
- First Phase since 3 to compile clean on the first `cargo check` with no
  manual touchups. Pre-flight discipline (collision grep + let-binding grep)
  caught everything before the bulk pass.
- `focused_pane()` is the first pass-through inlined as a chain expansion
  (`.focus.current()`) rather than a single-token replacement. The mechanics
  step 4 already covers "arity-changing rewrites"; chain-expansion fits the
  same category and worked via plain perl.

**Implications for remaining phases:**
- Phase 7 (Overlays) follows the same low-risk profile: no name collisions
  flagged, similar caller volume (~130). Pre-flight discipline is the gate;
  if collision and let-binding greps both come back empty, expect
  compile-clean on first attempt.
- The chain-expansion rewrite category should be added to mechanics step 3
  as category (d): pass-through inlining (`app.foo()` → `app.subsystem.bar()`
  when `app.foo()` body is `self.subsystem.bar()`). Phase 7's overlays don't
  have any such pass-throughs, but Phase 8/9 may.

### Phase 6 Review

- Phase 7: noted that `Overlays` itself exposes trivial-accessor methods
  (`finder_pane`, `settings_pane`, plus `_mut` variants) which are candidates
  for Phase 14's recursive purge — flagged in Phase 7's body so the Phase 7
  retrospective picks them up.
- Phase 8: corrected caller estimate from "~40–50" to "~50" based on live
  count of 51.
- Phase 8: added explicit revert pass for `scan_state_mut` collision —
  `Scan` has its own `scan_state_mut` method, so the bulk regex
  `\.scan_state_mut\(\)` will create double-prefix patterns that need the
  step-5 revert.
- Phase 8: added within-phase ordering note — pass-through chain expansions
  (`increment_data_generation` → `.scan.bump_generation()`,
  `data_generation_for_test` → `.scan.generation()`) must be rewritten
  before the underlying `scan`/`scan_mut` accessors are deleted.
- Mechanics: added rewrite category (d) — pass-through inlining /
  chain-expansion — to step 3.
- Mechanics: promoted let-binding grep from a Phase 5 lesson into step 2's
  pre-flight checklist.

### Phase 7 — Overlays trivial-accessor / pass-through delete ✅

Publish `overlays` as `pub(super)`. Delete `overlays`/`_mut`. Rewrite call sites.

**Pre-flight check:** Verified `fn overlays` is only defined on App; `Overlays`
has no method named `overlays`. Also grep `let .* = .*\.overlays_mut\(\)` for
`let`-binding rewrites.

**Trait-object arms:** `src/tui/interaction.rs:65–78` builds `&dyn Hittable`
from `app.overlays().finder_pane()` etc. After publish, the rewrite
`app.overlays.finder_pane()` is still a method call returning a reference, so
auto-borrow is preserved — no `&app.overlays` injection needed (unlike Phase 4's
`&app.toasts` case).

**Methods removed:** ~2. **Caller rewrites:** ~127 (live count).

**Note for Phase 14:** `Overlays` itself exposes six trivial-accessor
methods (`finder_pane`/`finder_pane_mut`, `settings_pane`/`settings_pane_mut`,
`keymap_pane`/`keymap_pane_mut` — each body is exactly `&self.{field}` /
`&mut self.{field}` per `src/tui/overlays/mod.rs`) which become candidates
for the recursive purge once Phase 7 publishes the field. Phase 14 must publish
each underlying field as `pub(super)` and rewrite caller chains
(`app.overlays.finder_pane().viewport()` → `app.overlays.finder_pane.viewport()`)
in the same files Phase 7 just touched (`src/tui/render.rs`,
`src/tui/interaction.rs`, `src/tui/finder.rs`, `src/tui/settings.rs`,
`src/tui/keymap_ui.rs`). The trait-object-coercion footnote (calling a
method that returns `&FinderPane` auto-borrows; reading the field
directly does not) applies to the `&dyn Hittable` arms at
`src/tui/interaction.rs:65–78` — Phase 7 noted this as a precaution but
did not need to fire; Phase 14 will.

### Retrospective

**What worked:** Pre-flight grep (`fn overlays`, `fn overlays_mut`, `let .* = .*\.overlays_mut\(\)`) all returned the App-only result with no collisions and no let-bindings, so the bulk regex pass landed clean. Two-pass ordering (`_mut` first, then read-side) avoided partial matches as expected. Compiled clean on first attempt.
**What deviated from the plan:** Nothing. Live caller count (127) matched the plan's estimate exactly.
**Surprises:** None. The "trait-object arms" note in the plan turned out to be a non-issue — there was no `&dyn Hittable` build site to special-case in the diff (the rewrite category note from prior phases was preserved precautionarily but not load-bearing here).
**Implications for remaining phases:** Reinforces that the now-standard mechanic (pre-flight grep → field publish → bulk-rewrite `_mut` then read-side → delete methods → validate) is reliable for any subsystem whose accessor name doesn't collide with a method on the subsystem type itself. Phase 8's `scan_state_mut` collision remains the only known exception in the queue.

### Phase 7 Review

- Phase 14: enumerated all six `Overlays` accessor pairs (`finder_pane`/`finder_pane_mut`, `settings_pane`/`settings_pane_mut`, `keymap_pane`/`keymap_pane_mut`) in the Phase 7 "Note for Phase 14"; previously only named four. Added trait-object-coercion caveat for `src/tui/interaction.rs:65–78` `&dyn Hittable` arms — Phase 7 noted it precautionarily but didn't fire; Phase 14 will.
- Phase 8: caller-rewrite estimate corrected from `~50 (live: 51)` to `~95 (live)`. Original count missed the test-only and pass-through accessors that Phase 8 also deletes (~30 of the 34 `scan_state_mut` callers live in `src/tui/app/tests/{panes,discovery_shimmer,state}.rs`).
- Phase 8: collision note re-categorized from category-(a) field-publish double-prefix to category-(d) chain-expansion collision. Mechanic outcome (same revert pass) unchanged; framing now matches the actual rewrite the chain expansion produces.
- Phase 8: chain-expansion ordering list grew from 2 entries (`increment_data_generation`, `data_generation_for_test`) to 5 — added `scan_state_mut`, `set_retry_spawn_mode_for_test`, `refresh_derived_state`, all of which forward to `self.scan.X()` and must be expanded before the `\.scan\(\)`/`\.scan_mut\(\)` field-publish bulk pass.
- Phase summary table: corrected baseline-drift error introduced by Phase 7 actually leaving App at 254 (not the table's prior 260). Phases 8–15 "App after" columns shifted -6 each; final post-15 floor moved from ~156 to ~151.
- Phases 9, 10, 11, 12, 13, 15: confirmed clean — no edits needed. Phase 9 pre-flight (`fn startup`/`startup_mut` only on App) verified; no collisions expected. Phases 10/11/12/13/15 unaffected by Phase 7 outcome.

### Phase 8 — Scan trivial-accessor / pass-through delete

Publish `scan` as `pub(super)`. Delete `scan`/`_mut`, `scan_state_mut` (test-only),
`data_generation_for_test`, `set_retry_spawn_mode_for_test`,
`increment_data_generation`, `refresh_derived_state`. Rewrite call sites.

(Note: `set_projects` was previously listed here. Moved to Phase 11 because its
body is `self.project_list.replace_roots_from(projects)` — project_list work,
not Scan work.)

**Phase 8 scope note (post-Phase-1):** Phase 1 dropped `Scan::projects()` /
`Scan::projects_mut()` outright and rewrote the render plumbing to take
`&ProjectList` directly. So Phase 8 is now purely an `app/*` and `async_tasks/*`
sweep — no `tui/render.rs` or `tui/panes/*` touches needed for `scan()` /
`scan_mut()` deletion. Caller estimate trimmed accordingly.

**Pre-flight check:** Verified `fn scan` is only defined on App in
`src/tui/app/mod.rs` — no unrelated type has a method named `scan` that
the bulk regex `\.scan\(\)` → `.scan` could clobber. Same for `scan_mut`.
**Known collision (chain-expansion / category (d)):** `Scan` itself has
its own `scan_state_mut` method, so when the chain-expansion pass rewrites
`app.scan_state_mut()` → `app.scan.scan_state_mut()`, today's
already-correct sites `self.scan.scan_state_mut()`
(`async_tasks/dispatch.rs:37,47,87`, `async_tasks/tree.rs:142,143,144`)
get incorrectly extended to `self.scan.scan.scan_state_mut()` if the regex
runs unscoped. The step-5 revert pass (`\.scan\.scan\.` → `.scan.` plus
the no-leading-dot variant `\bscan\.scan\.` → `scan.`) is required, not
optional.

**Within-phase ordering:** This phase has five pass-through chain
expansions queued — all delegations to `self.scan.X()`:
- `increment_data_generation` → `self.scan.bump_generation()`
- `data_generation_for_test` → `self.scan.generation()`
- `scan_state_mut` → `self.scan.scan_state_mut()`
- `set_retry_spawn_mode_for_test` → `self.scan.set_retry_spawn_mode_for_test()`
- `refresh_derived_state` → `self.scan.refresh_derived_state()`

Per mechanics step 3 category (d), rewrite all five chain expansions
**before** deleting `scan` / `scan_mut`, otherwise the bulk pass on `scan`
will see them as already-published-field uses and skip the chain expansion.

**Methods removed:** ~9. **Caller rewrites:** ~95 (live count, all five
chain-expansion methods plus `\.scan\(\)`/`\.scan_mut\(\)`). ~30 of the 34
`scan_state_mut` callers live in
`src/tui/app/tests/{panes,discovery_shimmer,state}.rs`.

### Phase 9 — Startup trivial-accessor / pass-through delete

Publish `startup` as `pub(super)`. Delete `startup`/`_mut`. Rewrite call sites.

**Pre-flight check:** Verified `fn startup` is only defined on App; `Startup`'s
own methods (`new`, `reset`, phase trackers) don't share names with App's
accessors. Also grep `let .* = .*\.startup_mut\(\)` for `let`-binding rewrites.

**Methods removed:** ~2. **Caller rewrites:** ~25.

### Phase 10 — Delete `App::projects()` / `projects_mut()` (highest-fanout rewrite)

These two pass-throughs survived 1/2 (1 lifted the field; 2 renamed it
`project_list`). After 5 publishes other subsystems' fields, the only remaining
App-level pass-throughs are these two — and they have the largest fanout in the entire
plan.

Live counts post-Phase-1 (`rg -n '\.projects\(\)' src/` and `\.projects_mut\(\)`):
- `app.projects()` / `self.projects()` → 250 occurrences
- `app.projects_mut()` / `self.projects_mut()` → 25 occurrences

Rewrite each to `app.project_list.X` (or `&mut app.project_list.X`). Delete both methods.

**Methods removed:** 2. **Caller rewrites:** ~275. **Largest single phase by
call-site count in the entire plan.**

**Ordering: Phase 10 depends on Phase 2 step 5 (field rename) being complete.**
`tui/render.rs::dispatch_via_trait`, `render_lints_pane`, and `render_ci_pane` call
`app.selected_project_path_for_render()` before split-borrowing, which routes through
`self.projects()`. If Phase 10 deletes `App::projects()` before Phase 2 has renamed the
field `projects` → `project_list`, that call path breaks. The plan's overall 1→2→…→10
order handles this implicitly; making the dependency explicit guards against
phase reordering silently introducing the bug.

Recommend the user run a global rename (`projects()` → `project_list`,
`projects_mut()` → `project_list`) via the editor's rename feature — see CLAUDE.md
note about rename support. Falls back to mechanical perl substitution otherwise
(handle the rustfmt-wrapped chain pattern).

### Phase 11 — `project_list` absorption I (row-navigation read-side)

Relocate row-navigation single-subsystem read methods to `impl ProjectList`
(post-Phase-2). These are pure queries over `ProjectList` state with no
`include_non_rust` threading — the read-side commits as one bounded phase
before the larger Phase 12 action-method sweep.

`path_for_row`, `display_path_for_row`, `abs_path_for_row`, `expand_key_for_row`,
`dismiss_target_for_row_inner`, `worktree_parent_node_index`,
`row_matches_project_path`, `selected_project_path`, `selected_row`,
`build_selected_pane_data`, `current_branch_for`, `latest_ci_run_for_path`,
`owner_repo_for_path_inner`, `ci_toggle_available_for_inner`,
`ci_runs_for_display_inner`, `try_collapse`, `last_selected_path`.

Also delete the test-only helper `set_projects` (body: one-line
`self.project_list.replace_roots_from(projects)`); test callers switch to
calling `replace_roots_from` directly on the field. Moved here from Phase 8
because the work is project_list-side, not Scan-side.

**Methods relocated:** ~17 + 1 test-only delete. **Caller rewrites:** ~80.

**Render-path note (post-Phase-1):** `tui/render.rs::dispatch_via_trait`,
`render_lints_pane`, and `render_ci_pane` currently call
`app.selected_project_path_for_render()` *before* split-borrowing. After Phase
2, `selected_project_path` is a pure `ProjectList` query (Selection's cursor
field having merged in), so Phase 11 can either keep that App-shim wrapper or
invert the order in render.rs (split-borrow first, then call
`projects.selected_project_path()` on the borrowed `&ProjectList`). Inverting
drops one App-level method.

### Phase 12 — `project_list` absorption II (action methods + `include_non_rust` threading)

Relocate the remaining `project_list` S methods (mutating, expansion-affecting,
or threaded through `include_non_rust`): `expand_all`, `collapse_all`,
`collapse_row`, `collapse_to`, `collapse`, `select_project_in_tree`,
`select_matching_visible_row`, `expand_path_in_tree`, `select_root`,
`select_root_row`, `clean_selection`, `move_up`, `move_down`, `move_to_top`,
`move_to_bottom`, `apply_finder`, `toggle_expand`,
`capture_legacy_root_expansions`, `migrate_legacy_root_expansions`,
`apply_cargo_fields_from_workspace_metadata`, `lint_runtime_root_entries`,
`handle_language_stats_batch`, `handle_crates_io_version_msg`, plus
`has_cached_non_rust_projects`, `selected_project_is_deleted`, `selected_ci_path`.
(`register_existing_projects` stays on App as Group X — it touches `project_list`
and `background` together when registering new items' watchers.)

**Methods relocated:** ~27. **Caller rewrites:** ~120 in `src/` plus ~50 in
`tests/` (the `include_non_rust` flag must be passed at every test call site of
the threaded methods — `tests/rows.rs`, `tests/panes.rs`, `tests/worktrees.rs`,
`tests/mod.rs`). Re-run `count_app_methods.py` at phase start — Phase 2's
absorbed-Selection rewrites (38 files / +536/-316) absorbed several call sites
the original estimate counted twice; the live number may be lower.

**Pattern from Phase 2:** the relocated methods that walk projects while
mutating `expanded` (e.g. `expand_path_in_tree`, `select_project_in_tree`,
`migrate_legacy_root_expansions`) should use the
`ProjectList::iter_with_expanded_mut` split-borrow helper introduced in Phase
2, not re-invent the destructuring pattern. The helper returns `(Values<'_>,
&mut HashSet<ExpandKey>)` from `&mut self`.

**`include_non_rust` parameter threading.** Per review-finding C2, eight
methods (`expand_all`, `collapse_all`, `collapse_row`, `collapse`,
`select_matching_visible_row`, `select_project_in_tree`, `expand_path_in_tree`,
`collapse_to`) plus `ensure_visible_rows_cached` currently read
`self.config().include_non_rust()` to decide whether to filter non-Rust rows.
To keep them on `ProjectList` as S relocations rather than X cross-subsystem,
change their signatures to take `include_non_rust: bool` as an argument; each
App-side caller extracts the value from config first:

```rust
let include_non_rust = app.config.current().tui.include_non_rust.includes_non_rust();
app.project_list.expand_all(include_non_rust);
```

The flag is small and stable (it changes only on config save). Threading it
explicitly is cleaner than coupling `ProjectList` to `Config`.

After Phases 7 and 8, ProjectList absorbs the navigation/data layer it
conceptually owned all along. The `impl App` block in `tui/app/navigation/*`
shrinks substantially; most of `navigation/` becomes `impl ProjectList`.

### Phase 13 — Non-`project_list` S relocations

Relocate the remaining S methods to their owning subsystems:

- → `startup` (5): `startup_disk_toast_body`, `startup_git_toast_body`,
  `startup_metadata_toast_body`, `log_startup_phase_plan`, `maybe_complete_startup_lints`.
- → `toasts` (4): `push_service_unavailable_toast`, `start_task_toast`,
  `mark_tracked_item_completed`, `focused_toast_id`. (`running_items_for_toast`
  is a static helper — moved in 15 onto `RunningTracker`. `dismiss_keymap_diagnostics`
  stays on App as Group X — touches Toasts + Keymap diagnostics state.)
- → `scan` (4): `handle_git_first_commit`, `should_verify_before_clean`,
  `handle_out_of_tree_target_size`, `handle_repo_meta`. (`clean_metadata_dispatch`
  and `update_generations_for_msg` stay on App as Group X — they touch
  net+background+scan and dispatch across every BackgroundMsg variant respectively.)
- → `net` (2): `availability_for`, `spawn_rate_limit_prime`.
- → `background` (1): `register_item_background_services`. (`finish_watcher_registration_batch`
  is a P-category one-line shim handled in the Phase 4–9 trivial-accessor / pass-through sweep, not an S relocation.)
- → `inflight` (1): `apply_example_progress`.
- → `ci` (1): `ci_display_mode_label_for_inner`.

**Methods relocated:** 18. **Caller rewrites:** ~100.

### Phase 14 — Recursive trivial-accessor purge inside subsystems

The universal decision rule applies at every nesting depth, not just on `App`.
Phase 14 sweeps the same rule through subsystem internals: every `pub(super) const fn
x(&self) -> &X { &self.x }` inside `Scan`, `Net.{Github, CratesIo}`, `Lint`,
`Inflight`, `Panes.{CpuPane, GitPane, TargetsPane, ...}`, `Config.WatchedFile`,
`Keymap.WatchedFile`, `ScanState`, `tui/pane/layout.rs::ResolvedPaneLayout`,
etc. — publish the field as `pub(super)`, delete the accessor, rewrite callers.

Specific accessors flagged by earlier phases:
- `ResolvedPaneLayout::panes()` (`src/tui/pane/layout.rs`) — body is `&self.panes`.
  Surfaced during Phase 5 as a regex false-positive collision. After Phase 14
  publishes the field and deletes the accessor, the call sites at
  `src/tui/render.rs:145` and `src/tui/input.rs:279,345` rewrite to direct
  field access.
- `Panes::worktree_summary_or_compute` (`src/tui/panes/system.rs:222`) —
  one-line pass-through to `GitPane::worktree_summary_or_compute`. Delete and
  publish `Panes::git` so callers go through `panes.git.worktree_summary_or_compute(...)`.

**Methods removed crate-wide:** ~50–80. **Caller rewrites:** ~200.

**App-local trivial accessors (no subsystem).** Several App accessors don't
belong to any owned subsystem — they wrap App's own primitive fields or
compose two subsystems with one line. Publish the field (or inline the body)
and delete the accessor:

- `mouse_pos`, `set_mouse_pos` — publish `App::mouse_pos: Option<Position>` as
  `pub(super)`, delete both accessors.
- `animation_elapsed` — publish `App::animation_started: Instant` as
  `pub(super)`; callers compute `app.animation_started.elapsed()` directly.
- `toast_timeout` — one-line wrapper over `config.tui.status_flash_secs`.
  Inline at the two call sites in `show_timed_toast` /
  `show_timed_warning_toast` and delete.
- `resolved_dirs` — one-line wrapper over `scan::resolve_include_dirs`.
  Inline at call sites.

**Visibility tightening on relocated types.** Phase 2 moved `ProjectList` from
`src/project_list.rs` (top-level) to `src/tui/project_list.rs` (nested).
Pre-existing `pub(crate)` methods on `ProjectList` (`new`, `len`, `is_empty`,
`iter`, `compute_visible_rows`, `at_path`, `entry_containing`,
`git_directories`, `for_each_leaf`, `for_each_leaf_path`, `lint_at_path`,
`lint_at_path_mut`, `entry_containing_mut`, `replace_ci_data_for_path`,
`ci_info_for`, `unpublished_ci_branch_name`, `is_deleted`, `regroup_members`,
`regroup_top_level_worktrees`, `insert_into_hierarchy`, `replace_leaf_by_path`,
`clear`, `resolved_root_labels`, `is_submodule_path`, ~14 more) all stay
`pub(crate)`. Tighten each to `pub(super)` unless a non-tui consumer is
identified (none currently). ~38 visibility tightenings; zero caller rewrites
(visibility narrowing is invisible to call sites already inside `tui/`).

Headline metric for 14: **crate-wide trivial-accessor count** (reported by
`count_app_methods.py`). App's count drops by 5 in 14 (the App-local accessors
listed above) — it lands at ~179 after Phase 14. Phase 14 also cleans the rest
of the codebase to match the same rule.

### Phase 15 — Relocate Group W static helpers to their data owners

23 of Group W's 27 entries are `Self::foo(...)` associated functions inside
`impl App` — they don't take `&self` and have nothing to do with App's state.
They're declared in `impl App` for convenience but they're really utility
functions over `RootItem` / `WorktreeGroup` / iterators. Move each to its
data owner.

**Worktree helpers** → `RootItem` / `WorktreeGroup`:
- `worktree_display_path`, `worktree_member_display_path`, `worktree_vendored_display_path`
- `worktree_abs_path`, `worktree_member_abs_path`, `worktree_vendored_abs_path`
- `worktree_path_ref`, `worktree_member_path_ref`, `worktree_vendored_path_ref`
- `unique_item_paths` (`mod.rs:527`) → `RootItem`

**Member/vendored helpers** → `RustProject` / `Workspace` / `Package`:
- `resolve_member`, `resolve_vendored`, `worktree_member_ref`, `worktree_vendored_ref`
  (`navigation/pane_data.rs`) → `RootItem` or `WorktreeGroup`
- `member_path_ref`, `vendored_path_ref` (`navigation/selection.rs`) → `RootItem`

**Toast/tracker helpers** → their respective owners:
- `running_items_for_toast` (`running_toasts.rs:41`) → `RunningTracker`
- `tracked_items_for_startup`, `startup_remaining_toast_body`,
  `startup_lint_toast_body_for` (`startup_phase/toast_bodies.rs`) → `Startup` (or
  free functions in the `startup_phase` module)

**Diagnostic helpers** — relocate or leave:
- `record_background_msg_kind`, `log_saturated_background_batch` (`async_tasks/poll.rs`):
  these are tracing/diagnostic helpers used inside `poll_background`. Either move to
  free functions in `poll.rs` (cleaner), or leave (low priority).

**Navigation cursor helper:**
- `collapse_anchor_row` (`navigation/movement.rs:5`) — `const fn` over a `VisibleRow`
  arg. Move to `impl VisibleRow`.

**Already-resident helpers (no Phase 15 action):**
- `ProjectList::replace_roots_from` (introduced in Phase 2) is a static-helper-on-data-owner
  that already lives on `ProjectList`. Called by `App::set_projects` (test-only) and
  `TreeMutation::replace_all`. Listed here so future passes don't relitigate moving it.

**Methods removed from App:** ~23. **Caller rewrites:** mostly `Self::foo(...)` →
`Type::foo(...)` plus method-call form where it makes sense (~50 sites total).

After 15, App's method count drops from 179 → **~156** (exact: 179 − 23 = 156).
Group W's instance methods that genuinely belong on App (`set_confirm`,
`confirm`, `take_confirm`, `build_worktree_detail`) stay.

**Ordering: Phase 15 must run after Phase 2 and Phase 10.** Several Phase 15 callers are
inside today's `impl App` blocks that read `self.projects()` (e.g. `member_path_ref` at
`navigation/selection.rs:79,87`). Phase 2 renames the field `projects` → `project_list`;
Phase 10 deletes the `projects()` accessor. After both, those callers use
`self.project_list` directly; relocating Phase 15 helpers before 2 or 6 would land them
referencing a still-named field or a still-live accessor and need re-rewriting. Phase 15
is otherwise independent of Phases 7 and 8.


## Mechanics of a collapse step

For each candidate App method `app.foo(args)` whose body is
`self.subsystem.bar(args)`:

1. **Find call sites.** `rg -n '\.foo\(' src/ --type rust`. The leading `\.` plus
   open-paren matches both `app.foo(` and rustfmt-wrapped `\n    .foo(` patterns. Filter
   to actual calls (not the def line, not doc comments).
2. **Pre-flight checklist.** Run all three before the bulk pass:
   - **Name-collision check:** For each accessor name being deleted, grep
     for a same-named method on the underlying type:
     `grep "fn $NAME\b" src/tui/$SUBSYSTEM*.rs`. If a collision exists, the bulk regex
     will over-replace `self.subsystem.X()` (already correct) into
     `self.subsystem.subsystem.X()`. Plan for a step-5 revert pass.
   - **Let-binding grep:** `rg -n 'let .* = .*\.${NAME}_mut\(\)' src/ --type rust`.
     Each hit becomes a manual rewrite (`let x = &mut app.field;`) after the bulk
     pass — the bulk regex turns it into a value move, which won't compile.
     (Phase 5 hit this at `interaction.rs:144`.)
   - **Chain-expansion review:** For each pass-through to be deleted (body is
     `self.subsystem.bar(...)` not `&self.subsystem`), note that the rewrite is
     a chain expansion (`.foo()` → `.subsystem.bar()`), not a single-token
     swap. These must run **before** the underlying field accessor is deleted
     so the bulk pass on `field`/`field_mut` doesn't strand them.
3. **Inspect each call site.** Four rewrite categories to watch for:
   (a) plain method call → field access — the common case;
   (b) trait-object coercion sites (`&dyn Hittable` arms, `&dyn Renderable` etc.)
       lose the auto-borrow that the accessor provided — need explicit `&app.field`;
   (c) `let` bindings from a `_mut()` accessor (e.g. `let panes = app.panes_mut();`)
       become a value move once the accessor is gone — need explicit
       `let panes = &mut app.panes;`. (Phase 5 hit this at `interaction.rs:144`.)
   (d) pass-through inlining / chain-expansion: when `app.foo()` body is
       `self.subsystem.bar()`, the rewrite is `app.foo()` → `app.subsystem.bar()`
       (e.g., Phase 6's `focused_pane()` → `.focus.current()`). Run these
       chain-expansion rewrites before deleting the underlying field accessor.
   For internal callers (`self.foo()`), the rewrite is `self.subsystem.bar()`.
4. **Apply the rewrites.** Bulk perl per the regex; Edit per file for arity-changing
   rewrites (e.g. `set_ci_fetch_toast(x)` → `ci.set_fetch_toast(Some(x))`). Use the
   multi-line `\s*` pattern when rustfmt has wrapped a call.
5. **Revert double-prefix patterns.** `\.subsystem\.subsystem\.` → `.subsystem.`,
   plus the no-leading-dot variant `\bsubsystem\.subsystem\.` → `subsystem.` for
   tests/inner code where a local var is named the same as the field (Phase 5
   hit this at `panes/system.rs` test sites). Required when step 2 flagged a
   collision; cheap to run unconditionally.
6. **Delete the App method.** No transitional `#[deprecated]` shim.
7. **Clean up unused imports.** Pass-through deletions often orphan imports
   (`GitHubRateLimit`, `RepoCache`, message types). Remove them when warnings surface.
8. **Validate.** `cargo check` → `cargo nextest run` → `cargo clippy --workspace
   --all-targets --all-features -- -D warnings` → `cargo install --path .`.
9. **Record the count delta.** `python3 scripts/count_app_methods.py` and put the
   before/after numbers in the phase retrospective.

## What stays on App (Group E preview)

To make the orchestrator vs. pass-through distinction concrete,
genuine orchestrators that should stay on App after this plan
completes (sample, will be firmed up by the deep-dive):

- `apply_config`, `rescan`, `apply_lint_config_change` — touch ≥3 subsystems each.
- `maybe_complete_startup_*` (six methods) — touch Startup + Toasts + Config.
- `force_settings_if_unconfigured` — touches Config + Focus + Overlays + Panes.
- `handle_bg_msg` — pattern-match dispatch across every subsystem.
- `request_clean_confirm` / `request_clean_group_confirm` — touch Scan + Confirm + Background.
- `prune_toasts`, `start_clean`, `start_task_toast`, `finish_task_toast`,
  `show_timed_toast`, `show_timed_warning_toast` — touch Toasts + Config + Focus.
- `tabbable_panes`, `is_pane_tabbable`, `focus_next_pane`, `focus_previous_pane`,
  `input_context` — read across Selection (post-Phase-2: ProjectList) + Scan + Panes +
  Inflight + Toasts + Overlays.
- `mutate_tree` (RAII guard constructor) — borrows multiple subsystems disjointly.
- `selected_row_owns_lint`, `lint_cell` — read multiple subsystems and return derived value.
- `discovery_*` family — read Scan + Config.

## Success criteria

- App's method count drops to **184** after Phase 13 (App-side trivial-accessor / pass-through/S phases complete), to **179** after Phase 14 (App-local accessors), and to **~156** after Phase 15 (static-helper relocation).
- `tui/app/mod.rs` drops from 1565 lines to under ~800.
- Every phase retrospective includes a `count: before → after (delta)` line generated by
  `scripts/count_app_methods.py`.
- All 599 tests still pass after each phase.
- Clippy stays green under `--all-features -- -D warnings` after each phase.
- Trivial-accessor count crate-wide drops to 0 after Phase 14 (all data fields `pub(super)` or
  carry real logic).

## Phase summary

Sized by call-site rewrite count (~100–400 per phase) rather
than method count, because that's where the actual work
sits.

| Phase | Scope | Methods removed | Caller rewrites | App after |
| --- | --- | ---: | ---: | ---: |
| 1 | Lift `ProjectList` to App (structural) | 2 | ~15 | 306 |
| 2 | Merge `Selection` into `ProjectList`; relocate to `tui/` (structural) | 0 | ~150 | 306 |
| 3 | Tooling + trivial-accessor / pass-through delete: Config, Keymap, LayoutCache | 13 | ~140 | 293 ✅ |
| 4 | trivial-accessor / pass-through delete: Lint, Ci, Toasts, Net, Background, Inflight | 28 | ~250 | 265 ✅ |
| 5 | trivial-accessor / pass-through delete: Panes | 6 | ~110 | 259 ✅ |
| 6 | trivial-accessor / pass-through delete: Focus | 3 | ~93 | 256 ✅ |
| 7 | trivial-accessor / pass-through delete: Overlays | 2 | 127 | 254 ✅ |
| 8 | trivial-accessor / pass-through delete: Scan | ~9 | ~95 | 245 |
| 9 | trivial-accessor / pass-through delete: Startup | ~2 | ~25 | 243 |
| **10** | **Delete `App::projects()` / `projects_mut()`** | **2** | **~275** | **241** |
| 11 | `project_list` absorption I — row-navigation read-side | ~17 | ~85 | 224 |
| 12 | `project_list` absorption II — action methods (with `include_non_rust` arg threading) | ~27 | ~170 | 197 |
| 13 | Non-`project_list` S relocations | 18 | ~95 | 179 |
| 14 | Recursive trivial-accessor purge (crate-wide + 5 App-local accessors) | ~50–80 (crate-wide), 5 (App) | ~200 | 174 |
| 15 | Relocate Group W static helpers to their data owners (after 10) | 23 | ~50 | **151** |

**Net: 308 → 179 on App after Phase 13, → 174 after Phase 14, → ~151 after Phase 15.**
Per review-finding C2, six methods (`expand_all`, `collapse_all`,
`select_matching_visible_row`, `select_project_in_tree`,
`expand_path_in_tree`, `collapse_to`) keep their S →
project_list classification by adding `include_non_rust: bool`
to their signatures; each App-side caller reads the flag from
config first. 11 then relocates the 23 static helpers
(`worktree_*`, `running_items_for_toast`, etc.) to their
proper data owners (`RootItem`, `WorktreeGroup`,
`RunningTracker`, etc.) — they were declared inside `impl App`
for convenience but don't belong on App at all.

Phase 10 is the largest single phase by call-site count (~275). The combined
Phase 5–9 trivial-accessor / pass-through sweep (~410 callers across 5 separately-numbered phases) is bigger
in aggregate, but each individual phase is small enough to review independently.

Phase 14 is a companion phase — App's headline count target is
satisfied at end of 9. 10 reduces trivial-accessor count
crate-wide but leaves App's number unchanged.

Numbers from team-agent hand-classification of all 308 methods,
plus subagent review correction (C1+C2). Final values land
within ±5 per phase as actual rewrites expose edge cases.

Note: this plan dropped the prior caller-count buckets (B/C/D
for 1, 2–5, 6+ callers). Hand-classification showed the
distinction was not load-bearing — the trivial-accessor / pass-through delete mechanic is
the same whether a method has 1 caller or 200. Caller fanout
matters for phase sizing (already accounted for above), not
for the structure of the work.
## Method inventory (team-agent classification)

All 308 App methods, hand-classified by reading each body. Six agents in parallel covered the codebase by file group; each read the actual source for every method in its assigned slice.

### Final counts

| Category | Count | Phase | Outcome |
| --- | ---: | --- | --- |
| **T** — trivial accessor (`&self.X`) | 28 | 3 | delete; field `pub(super)` |
| **P** — pass-through (single-subsystem after 1/2) | 65 | 3 | delete; callers go direct |
| **S** — single-subsystem orchestrator | 64 | 8 | **relocate to owning subsystem** |
| **X** — cross-subsystem orchestrator | 102 | — | keep on App |
| **H** — handler/dispatcher (BackgroundMsg / multi-subsystem fan-out) | 22 | — | keep on App |
| **W** — wrapping logic / static helpers / App-local fields | 27 | — | keep on App |
| **Total** | **308** | | |

**Phases 3–9 delete trivial-accessor + pass-through = ~88 methods.**  
**Phases 10–13 absorb/relocate ~64 S methods.**  
**App's final method count ≈ 156 after Phase 15 (down from 308, ~49% reduction).** Phase-by-phase math: 3–13 remove T + P + S to land App at 184, Phase 14 removes 5 App-local accessors (lands at 179), then Phase 15 relocates 23 static helpers from Group W (lands at ~156). Phase 14's main work is crate-wide trivial-accessor cleanup.

### Group S — relocation list (single-subsystem orchestrators)

Each method moves from App to the destination subsystem in Phase 11 (read-side),
Phase 12 (action methods), or Phase 13 (non-`project_list`).

#### → `project_list` (46 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `display_path_for_row` | `navigation/selection.rs:186` | 100L | Reads self.projects() and dispatches via worktree_* associated helpers — project_list post-Phase-1 |
| `abs_path_for_row` | `navigation/selection.rs:290` | 97L | Reads self.projects() and dispatches via worktree_* associated helpers — project_list post-Phase-1 |
| `collapse_row` | `navigation/expand.rs:108` | 95L | Match on VisibleRow and calls try_collapse/collapse_to/is_inline_group helpers — all project_list-resident post-Phase-2 |
| `expand_path_in_tree` | `navigation/bulk.rs:80` | 75L | Reads scan.projects() and writes selection.expanded; post-Phase-1/2 single subsystem |
| `build_selected_pane_data` | `navigation/pane_data.rs:15` | 69L | Reads self.projects() via selected_row + helpers; calls tui::panes::build_pane_data passing &self — main state access is project_list |
| `dismiss_target_for_row_inner` | `dismiss.rs:24` | 57L | pure projects() row resolution |
| `path_for_row` | `navigation/selection.rs:70` | 56L | Reads self.projects() and dispatches to associated helpers — project_list post-Phase-1 |
| `expand_key_for_row` | `navigation/expand.rs:17` | 47L | Reads self.projects() only (post-Phase-1 part of project_list) |
| `migrate_legacy_root_expansions` | `async_tasks/tree.rs:84` | 45L | mutates selection.expanded reading scan.projects (post-Phase-2 one subsystem) |
| `expand_all` | `navigation/bulk.rs:13` | 44L | Reads scan.projects() and writes selection.expanded/paths; post-Phase-1/2 both fold into project_list |
| `capture_legacy_root_expansions` | `async_tasks/tree.rs:46` | 36L | reads project_list and selection.expanded (post-Phase-2 one subsystem) |
| `lint_runtime_root_entries` | `async_tasks/lint_runtime.rs:81` | 28L | pure read of projects to build entries |
| `clean_selection` | `navigation/selection.rs:33` | 25L | selected_row + self.projects() + worktree_path_ref helper — project_list post-Phase-2 |
| `worktree_parent_node_index` | `dismiss.rs:120` | 20L | pure projects() iteration |
| `is_worktree_inline_group` | `navigation/worktree_paths.rs:26` | 19L | self.projects().get only — project_list post-Phase-1 |
| `collapse_all` | `navigation/bulk.rs:60` | 17L | Reads/writes selection state (expanded, cursor, paths); ensure_visible_rows_cached touches config but only for filter passed into project_list — post-Phase-2 single subsystem |
| `selected_row_owns_lint` | `mod.rs:286` | 16L | matches on selected_row variants; pure project_list classification |
| `apply_cargo_fields_from_workspace_metadata` | `async_tasks/metadata_handlers.rs:200` | 13L | only stamps cargo onto projects rust_info / vendored |
| `detail_path_is_affected` | `async_tasks/priority_fetch.rs:8` | 13L | reads selected path and project_list lint pointers |
| `clear_all_lint_state` | `mod.rs:713` | 11L | iterates projects leaves and clears LintRuns on project_list |
| `expand` | `navigation/expand.rs:67` | 11L | selection.cursor + visible_rows + expanded_mut — all project_list post-Phase-2 |
| `collapse` | `navigation/expand.rs:96` | 9L | Reads/writes selection state (cursor, expanded) only — project_list post-Phase-2 |
| `is_inline_group` | `navigation/worktree_paths.rs:13` | 9L | self.projects().get only — project_list post-Phase-1 |
| `select_matching_visible_row` | `navigation/bulk.rs:163` | 8L | Reads visible_rows + writes selection.cursor; all project_list post-Phase-2 |
| `has_cached_non_rust_projects` | `construct.rs:227` | 7L | only walks projects() leaves |
| `select_root_row` | `dismiss.rs:144` | 7L | reads visible_rows, sets selection cursor (post-Phase-2, single subsystem) |
| `discovery_shimmer_session_matches` | `mod.rs:661` | 6L | calls helpers that read projects only |
| `selected_is_expandable` | `navigation/expand.rs:8` | 6L | selection.cursor + visible_rows + expand_key_for_row (project_list) |
| `selected_item` | `navigation/selection.rs:22` | 6L | selected_row + self.projects().get — project_list post-Phase-2 |
| `collapse_to` | `navigation/expand.rs:82` | 5L | selection.expanded_mut + ensure_visible_rows_cached + selection.set_cursor — project_list post-Phase-2 (ensure_visible_rows_cached's config read is internal to project_list helper) |
| `handle_crates_io_version_msg` | `async_tasks/lint_handlers.rs:12` | 5L | only mutates projects rust_info / vendored |
| `handle_language_stats_batch` | `async_tasks/metadata_handlers.rs:32` | 5L | loops setting language_stats on projects |
| `selected_display_path` | `navigation/selection.rs:178` | 4L | visible_rows + selection.cursor + display_path_for_row — project_list post-Phase-2 |
| `startup_git_directory_for_path` | `async_tasks/repo_handlers.rs:339` | 4L | reads only project_list iter |
| `owner_repo_for_path_inner` | `ci.rs:19` | 4L | only reads projects(); pure project_list query |
| `selected_ci_path` | `mod.rs:495` | 3L | resolves selected path via projects().entry_containing |
| `discovery_scope_contains` | `mod.rs:675` | 3L | iterates projects() |
| `discovery_parent_row` | `mod.rs:681` | 3L | iterates projects() |
| `selected_row` | `navigation/selection.rs:15` | 3L | visible_rows + selection.cursor — project_list post-Phase-2 |
| `ci_is_exhausted` | `query/ci_queries.rs:7` | 3L | pure projects() ci_data query |
| `selected_project_is_deleted` | `mod.rs:687` | 2L | selected_project_path + projects().is_deleted |
| `select_project_in_tree` | `navigation/bulk.rs:174` | 2L | Two helper calls, both touch project_list only post-Phase-2 |
| `ensure_disk_cache` | `navigation/cache.rs:33` | 2L | Calls panes::compute_disk_cache(self.projects()) and writes selection — single subsystem post-Phase-2 |
| `dismiss_target_for_row` | `mod.rs:1079` | 1L | inner reads only projects() (post-Phase-1: project_list) |
| `owner_repo_for_path` | `mod.rs:1083` | 1L | inner reads projects().primary_url_for + parse |
| `ci_toggle_available_for` | `mod.rs:1091` | 1L | inner reads projects().git_info_for branch |

#### → `startup` (5 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `maybe_complete_startup_lints` | `async_tasks/startup_phase/tracker.rs:222` | 22L | reads/writes only startup.lint_phase |
| `log_startup_phase_plan` | `async_tasks/startup_phase/tracker.rs:114` | 8L | logs only startup expected lengths |
| `startup_disk_toast_body` | `async_tasks/startup_phase/toast_bodies.rs:11` | 3L | reads only startup.disk |
| `startup_git_toast_body` | `async_tasks/startup_phase/toast_bodies.rs:16` | 3L | reads only startup.git |
| `startup_metadata_toast_body` | `async_tasks/startup_phase/toast_bodies.rs:21` | 3L | reads only startup.metadata |

#### → `toasts` (4 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `push_service_unavailable_toast` | `async_tasks/service_handlers.rs:72` | 5L | only mutates toasts |
| `start_task_toast` | `mod.rs:395` | 4L | toasts.push_task + viewport len update |
| `mark_tracked_item_completed` | `mod.rs:423` | 3L | toasts.mark_item_completed + viewport len |
| `focused_toast_id` | `mod.rs:361` | 2L | reads toasts.active_now + toasts.viewport |

#### → `scan` (4 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `handle_git_first_commit` | `async_tasks/repo_handlers.rs:194` | 22L | only mutates scan.projects and pending_git_first_commit |
| `should_verify_before_clean` | `mod.rs:914` | 15L | reads scan.metadata_store + manifest fingerprint |
| `handle_out_of_tree_target_size` | `async_tasks/metadata_handlers.rs:15` | 10L | only touches scan.metadata_store |
| `handle_repo_meta` | `async_tasks/repo_handlers.rs:264` | 4L | only writes to scan.projects entry github_info |

#### → `net` (2 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `spawn_rate_limit_prime` | `async_tasks/service_handlers.rs:106` | 9L | only uses net.http_client |
| `availability_for` | `async_tasks/service_handlers.rs:63` | 4L | returns mutable net.github/crates_io availability |

#### → `background` (1 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `register_item_background_services` | `async_tasks/background_services.rs:33` | 17L | only sends WatcherMsg via self.background |

#### → `inflight` (1 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `apply_example_progress` | `async_tasks/poll.rs:166` | 5L | only mutates inflight.example_output |

#### → `ci` (1 methods)

| Method | File:line | Body | Why this destination |
| --- | --- | ---: | --- |
| `ci_display_mode_label_for_inner` | `ci.rs:161` | 4L | matches on ci display mode only |

### Group X — cross-subsystem orchestrators (KEEP)

102 methods. Touch 2+ subsystems; no single-subsystem owner.

| Method | File:line | Body | Touches |
| --- | --- | ---: | --- |
| `spawn_repo_fetch_for_git_info` | `async_tasks/repo_handlers.rs:24` | 65L | touches net, background, config to spawn worker thread |
| `register_lint_for_root_items` | `async_tasks/lint_runtime.rs:131` | 51L | lint runtime + project_list iteration |
| `sync_running_toast` | `async_tasks/running_toasts.rs:66` | 39L | mutates toasts and reads config |
| `schedule_startup_project_details` | `async_tasks/background_services.rs:52` | 36L | touches background, net, project_list, then calls schedule_member_crates_io_fetches |
| `maybe_complete_startup_ready` | `async_tasks/startup_phase/tracker.rs:246` | 36L | reads startup, touches toasts via finish_task_toast |
| `discovery_name_segments_for_path` | `mod.rs:598` | 33L | reads config + helper that touches scan.discovery_shimmers + projects |
| `register_lint_project_if_eligible` | `async_tasks/lint_runtime.rs:184` | 31L | project_list + lint runtime |
| `schedule_git_first_commit_refreshes` | `async_tasks/background_services.rs:121` | 31L | touches background sender and project_list iteration |
| `handle_checkout_info` | `async_tasks/repo_handlers.rs:97` | 28L | mutates project_list, scan, startup, toasts and triggers fetch |
| `handle_project_discovered` | `async_tasks/repo_handlers.rs:270` | 28L | touches project_list, background, config, scan, panes, selection |
| `ci_for_item` | `mod.rs:553` | 27L | iterates paths calling ci_for + latest_ci_run_for_path (ci+projects) |
| `input_context` | `mod.rs:1139` | 26L | reads overlays + focus + panes::behavior |
| `maybe_complete_startup_lint_cache` | `async_tasks/lint_handlers.rs:36` | 26L | startup + lint + toasts |
| `show_keymap_diagnostics` | `async_tasks/config.rs:86` | 23L | toasts + keymap (push, set_diagnostics_id) |
| `apply_lint_config_change` | `async_tasks/config.rs:220` | 23L | lint + scan + project_list + toasts + overlays + background |
| `maybe_priority_fetch` | `async_tasks/priority_fetch.rs:24` | 23L | touches project_list, panes, scan, then spawns terminal fetch |
| `maybe_complete_startup_repo` | `async_tasks/startup_phase/tracker.rs:174` | 23L | touches startup and toasts |
| `apply_disk_usage` | `async_tasks/disk_handlers.rs:32` | 22L | project_list + lint runtime register/unregister |
| `schedule_member_crates_io_fetches` | `async_tasks/background_services.rs:97` | 22L | touches background sender, net client, project_list iteration |
| `apply_unavailability` | `async_tasks/service_handlers.rs:39` | 22L | touches net availability, toasts, spawns retry |
| `reset_startup_phase_state` | `async_tasks/startup_phase/tracker.rs:23` | 22L | reads project_list and writes startup |
| `apply_tree_build` | `async_tasks/tree.rs:20` | 21L | touches selection, scan, lint, focus, panes via helpers |
| `focus_next_pane` | `mod.rs:1229` | 19L | mutates focus, toasts; calls tabbable_panes |
| `finish_new` | `construct.rs:237` | 19L | startup orchestrator across panes, overlays, net, config, projects |
| `dismiss` | `dismiss.rs:96` | 19L | touches toasts, projects, selection, layout_cache |
| `mutate_tree` | `mod.rs:1024` | 18L | reads config, split-borrows scan+panes+selection |
| `focus_previous_pane` | `mod.rs:1251` | 18L | mutates focus, toasts; calls tabbable_panes |
| `poll_clean_msgs` | `async_tasks/poll.rs:180` | 18L | reads background channel, mutates inflight, calls sync_running_clean_toast |
| `maybe_complete_startup_disk` | `async_tasks/startup_phase/tracker.rs:136` | 17L | touches startup and toasts via finish_task_toast/mark_tracked_item_completed |
| `maybe_complete_startup_git` | `async_tasks/startup_phase/tracker.rs:155` | 17L | touches startup and toasts |
| `maybe_complete_startup_metadata` | `async_tasks/startup_phase/tracker.rs:199` | 17L | touches startup and toasts |
| `load_initial_keymap` | `async_tasks/config.rs:26` | 16L | config + keymap + toasts (show_keymap_diagnostics, show_timed_toast) |
| `spawn_service_retry` | `async_tasks/service_handlers.rs:83` | 16L | touches scan retry mode, background, net client |
| `force_settings_if_unconfigured` | `mod.rs:1119` | 15L | touches config, focus, overlays, panes |
| `insert_ci_runs` | `ci.rs:27` | 15L | touches projects, scan, ci |
| `start_clean` | `mod.rs:433` | 14L | scan.resolve_target_dir + show_timed_toast + inflight.clean_mut + sync_running_clean_toast |
| `ci_runs_for_display_inner` | `ci.rs:190` | 14L | uses projects() ci_info plus ci display mode |
| `lint_cell` | `mod.rs:313` | 13L | reads config.lint_enabled + animation_started + status; cross-subsystem |
| `discovery_shimmer_session_for_path` | `mod.rs:640` | 13L | reads scan.discovery_shimmers + helper touching projects |
| `tabbable_panes` | `mod.rs:1213` | 13L | reads inflight + calls is_pane_tabbable + toasts |
| `maybe_reload_config_from_disk` | `async_tasks/config.rs:116` | 13L | config + toasts + apply_config call |
| `refresh_lint_runs_from_disk` | `async_tasks/lint_runtime.rs:46` | 13L | project_list + lint reads + cache refresh |
| `new` | `mod.rs:256` | 12L | constructor delegates to AppBuilder, fans out to all subsystems |
| `lint_runtime_projects` | `async_tasks/lint_runtime.rs:111` | 12L | scan.is_complete + project_list + lint_runtime_root_entries |
| `respawn_watcher` | `async_tasks/lint_runtime.rs:18` | 11L | config + background + net + lint + scan |
| `register_background_services_for_tree` | `async_tasks/background_services.rs:20` | 11L | iterates project_list and calls per-item background register touching net/background |
| `toggle_ci_display_mode_for_inner` | `ci.rs:176` | 11L | touches ci, scan, project_list (via helper) |
| `register_discovery_shimmer` | `mod.rs:585` | 10L | reads scan.is_complete + config + writes scan.discovery_shimmers |
| `prune_inactive_project_state` | `mod.rs:692` | 10L | reads projects + writes scan.pending_git_first_commit + ci.fetch_tracker |
| `clean_metadata_dispatch` | `mod.rs:935` | 10L | reads net.http_client + background + scan.metadata_store |
| `reset_project_panes` | `mod.rs:1272` | 10L | resets panes, ci, lint, toasts, focus |
| `ensure_fit_widths_cached` | `navigation/cache.rs:20` | 10L | Reads projects() and config().lint_enabled() and writes selection — config + project_list |
| `poll_example_msgs` | `async_tasks/poll.rs:154` | 10L | reads background channel, writes inflight, calls finish_example_run |
| `prune_toasts` | `mod.rs:366` | 9L | reads config + writes toasts + reads/writes focus |
| `ci_for` | `mod.rs:506` | 9L | projects().unpublished_ci_branch_name + projects().ci_info_for + latest_ci_run_for_path (ci+projects) |
| `handle_lint_cache_pruned` | `async_tasks/lint_handlers.rs:133` | 9L | toasts + lint cache refresh |
| `apply_service_signal` | `async_tasks/service_handlers.rs:13` | 9L | dispatches across handle_service_reachable and apply_unavailability |
| `latest_ci_run_for_path` | `ci.rs:207` | 9L | uses projects() ci_info plus ci display mode |
| `request_clean_confirm` | `mod.rs:875` | 8L | touches scan, net, background, confirm |
| `request_clean_group_confirm` | `mod.rs:894` | 8L | touches scan, net, background, confirm |
| `refresh_lint_cache_usage_from_disk` | `async_tasks/lint_runtime.rs:70` | 8L | config + lint (reads cache size from config) |
| `handle_repo_fetch_complete` | `async_tasks/repo_handlers.rs:254` | 8L | mutates net, startup, calls maybe_log_startup and sync_running_repo_fetch_toast |
| `finish_task_toast` | `mod.rs:406` | 7L | reads config.task_linger_secs + toasts.finish_task + prune_toasts |
| `split_panes_for_render` | `mod.rs:745` | 7L | returns refs to panes + layout_cache + config + project_list (post-Phase-2: 4-tuple, selection slot dropped) |
| `handle_disk_usage_msg` | `async_tasks/disk_handlers.rs:56` | 7L | startup + toasts + handle_disk_usage chain |
| `handle_disk_usage_batch_msg` | `async_tasks/disk_handlers.rs:65` | 7L | scan + startup + toasts + batch chain |
| `reload_lint_history` | `async_tasks/lint_runtime.rs:61` | 7L | project_list + lint disk read + scan.projects_mut |
| `maybe_trigger_repo_fetch` | `async_tasks/repo_handlers.rs:185` | 7L | reads project_list, calls spawn_repo_fetch_for_git_info |
| `focused_dismiss_target` | `dismiss.rs:85` | 7L | touches focus, toasts, project_list |
| `handle_lint_startup_status_msg` | `async_tasks/lint_handlers.rs:24` | 6L | project_list + startup + maybe_complete chain |
| `handle_service_reachable` | `async_tasks/service_handlers.rs:31` | 6L | touches net availability and toasts |
| `mark_service_recovered` | `async_tasks/service_handlers.rs:117` | 6L | touches net availability and toasts |
| `ci_is_fetching` | `mod.rs:518` | 5L | projects().entry_containing + ci.fetch_tracker; cross-subsystem |
| `set_example_output` | `mod.rs:980` | 5L | writes inflight + may set focus.set(Output) |
| `ensure_visible_rows_cached` | `navigation/cache.rs:9` | 5L | Reads config.include_non_rust() AND scan.projects() AND mutates selection — touches config + project_list |
| `record_config_reload_failure` | `async_tasks/config.rs:19` | 5L | overlays + toasts |
| `apply_disk_usage_breakdown` | `async_tasks/disk_handlers.rs:25` | 5L | project_list + delegates to apply_disk_usage |
| `update_generations_for_msg` | `async_tasks/dispatch.rs:25` | 5L | scan.bump_generation + detail_path_is_affected (project_list) |
| `finish_example_run` | `async_tasks/poll.rs:173` | 5L | touches inflight and scan |
| `sync_running_clean_toast` | `async_tasks/running_toasts.rs:13` | 5L | reads inflight, calls sync_running_toast on toasts/config |
| `sync_running_lint_toast` | `async_tasks/running_toasts.rs:20` | 5L | reads lint, calls sync_running_toast on toasts/config |
| `rebuild_visible_rows_now` | `async_tasks/tree.rs:131` | 5L | reads config and recomputes selection visibility over scan.projects |
| `show_timed_warning_toast` | `mod.rs:384` | 4L | toasts.push_timed_styled using config-derived timeout |
| `set_task_tracked_items` | `mod.rs:416` | 4L | reads config + writes toasts |
| `save_and_apply_config` | `async_tasks/config.rs:131` | 4L | config + apply_config orchestration |
| `handle_disk_usage` | `async_tasks/disk_handlers.rs:10` | 4L | inflight + apply_disk_usage (project_list + lint) |
| `sync_lint_runtime_projects` | `async_tasks/lint_runtime.rs:125` | 4L | lint runtime + lint_runtime_projects (project_list) |
| `sync_running_repo_fetch_toast` | `async_tasks/running_toasts.rs:29` | 4L | reads net.github, calls sync_running_toast on toasts/config |
| `show_timed_toast` | `mod.rs:378` | 3L | toasts.push_timed using config-derived timeout |
| `dismiss_keymap_diagnostics` | `async_tasks/config.rs:111` | 3L | keymap + toasts |
| `register_existing_projects` | `async_tasks/lint_runtime.rs:31` | 3L | project_list iter + register_item_background_services |
| `respawn_watcher_and_register_existing_projects` | `async_tasks/lint_runtime.rs:41` | 3L | composes three cross-subsystem calls |
| `register_lint_for_path` | `async_tasks/lint_runtime.rs:217` | 3L | project_list + register_lint_project_if_eligible (lint) |
| `clean_spawn_failed` | `mod.rs:450` | 2L | inflight.clean_mut.remove + sync_running_clean_toast (writes toasts) |
| `dismiss_toast` | `mod.rs:455` | 2L | toasts.dismiss + prune_toasts (touches focus too via prune) |
| `selected_ci_runs` | `mod.rs:501` | 2L | selected_project_path + ci_runs_for_display (touches ci + projects) |
| `split_ci_for_render` | `mod.rs:760` | 1L | returns refs to ci + config + scan |
| `split_lint_for_render` | `mod.rs:766` | 1L | returns refs to lint + config + scan |
| `apply_hovered_pane_row` | `mod.rs:787` | 1L | interaction helper writes toasts+ci+lint+overlays+panes viewports |
| `toggle_ci_display_mode_for` | `mod.rs:1095` | 1L | inner writes ci + scan.bump_generation, reads project_list |
| `ci_runs_for_display` | `mod.rs:1099` | 1L | inner reads project_list (ci_info, branch) + ci.display_mode |
| `reset_cpu_placeholder` | `mod.rs:1105` | 1L | reads config.current().cpu, writes panes.reset_cpu |

### Group H — handlers/dispatchers (KEEP)

22 methods. Large multi-subsystem dispatch (BackgroundMsg match, scan-result handlers, startup tracker drivers).

| Method | File:line | Body | Notes |
| --- | --- | ---: | --- |
| `handle_ci_fetch_complete` | `ci.rs:46` | 103L | large dispatch over ci, scan, net, overlays, toasts, projects |
| `handle_bg_msg` | `async_tasks/dispatch.rs:93` | 99L | giant BackgroundMsg match dispatcher |
| `handle_cargo_metadata_msg` | `async_tasks/metadata_handlers.rs:48` | 72L | scan + startup + toasts + accept_cargo_metadata |
| `handle_lint_status_msg` | `async_tasks/lint_handlers.rs:64` | 67L | project_list + lint + config + scan + startup + toasts |
| `apply_config` | `async_tasks/config.rs:137` | 62L | huge fan-out: config, scan, net, lint, project_list |
| `handle_scan_result` | `async_tasks/dispatch.rs:32` | 54L | scan + project_list + lint + startup + background dispatch |
| `accept_cargo_metadata` | `async_tasks/metadata_handlers.rs:132` | 54L | scan + project_list + net + background spawn |
| `poll_background` | `async_tasks/poll.rs:15` | 46L | top-level dispatcher draining bg_rx and calling many subsystem handlers |
| `handle_repo_info` | `async_tasks/repo_handlers.rs:133` | 44L | mutates scan/project_list, queries net, invalidates cache, triggers fetch |
| `rescan` | `async_tasks/tree.rs:138` | 44L | resets scan, ci, lint, net, startup, focus, overlays, panes, inflight, selection, background |
| `poll_ci_fetches` | `async_tasks/poll.rs:112` | 40L | drains ci_fetch_rx and dispatches across ci, project_list, toasts, config |
| `maybe_reload_keymap_from_disk` | `async_tasks/config.rs:44` | 39L | keymap + config + toasts + filesystem dispatcher |
| `sync_selected_project` | `query/post_selection.rs:10` | 39L | dispatch across selection, focus, scan, panes, layout_cache |
| `is_pane_tabbable` | `mod.rs:1171` | 38L | dispatches across project_list, panes, inflight, lint, ci, toasts |
| `enter_action` | `query/post_selection.rs:55` | 38L | dispatch across focus, panes, ci, projects |
| `handle_project_refreshed` | `async_tasks/repo_handlers.rs:300` | 36L | rebuilds tree and clears ci/lint/panes content, multi-subsystem |
| `ensure_detail_cached` | `navigation/cache.rs:43` | 34L | Multi-subsystem orchestrator: scan.generation, pane_data, ci_mut, lint_mut, panes_mut, build_ci_data/build_lints_data — fan-out to many subsystems |
| `handle_repo_fetch_queued` | `async_tasks/repo_handlers.rs:218` | 34L | mutates startup, net, toasts, config and syncs running toast |
| `start_startup_detail_toasts` | `async_tasks/startup_phase/tracker.rs:79` | 33L | reads startup, calls toast_body helpers and toasts methods |
| `start_startup_toast` | `async_tasks/startup_phase/tracker.rs:47` | 30L | builds tracked items and calls start_task_toast/set_task_tracked_items on toasts then writes startup |
| `maybe_log_startup_phase_completions` | `async_tasks/startup_phase/tracker.rs:124` | 10L | dispatches to all maybe_complete_* per-phase |
| `initialize_startup_phase_tracker` | `async_tasks/startup_phase/tracker.rs:16` | 5L | calls reset, start_startup_toast, detail_toasts, log_plan, completions |

### Group W — wrapping logic / static helpers (KEEP)

27 methods. Static helpers (`Self::foo` associated functions), App-local field reads, or pure computations that don't touch subsystems. Many of these are candidates to relocate to their data owners (e.g. `worktree_*` helpers belong on `RootItem` / `WorktreeGroup`), but that's a follow-up cleanup, not part of this plan.

| Method | File:line | Body | Notes |
| --- | --- | ---: | --- |
| `record_background_msg_kind` | `async_tasks/poll.rs:63` | 30L | static stats counter, no self |
| `build_worktree_detail` | `navigation/pane_data.rs:183` | 28L | Method takes &self only to forward to tui::panes builders; no direct subsystem field access |
| `vendored_path_ref` | `navigation/selection.rs:147` | 27L | Associated fn on RootItem arg; no &self |
| `collapse_anchor_row` | `navigation/movement.rs:5` | 25L | Pure const fn on VisibleRow argument; no &self, no subsystem touch |
| `worktree_vendored_ref` | `navigation/pane_data.rs:152` | 23L | Associated fn on RootItem arg; no &self |
| `worktree_vendored_display_path` | `navigation/worktree_paths.rs:94` | 23L | Associated fn on RootItem arg; no &self |
| `worktree_vendored_abs_path` | `navigation/worktree_paths.rs:170` | 23L | Associated fn on RootItem arg; no &self |
| `worktree_vendored_path_ref` | `navigation/worktree_paths.rs:246` | 23L | Associated fn on RootItem arg; no &self |
| `unique_item_paths` | `mod.rs:527` | 22L | associated fn on &RootItem; touches no self/subsystem |
| `worktree_display_path` | `navigation/worktree_paths.rs:48` | 21L | Associated fn on RootItem arg; no &self |
| `worktree_abs_path` | `navigation/worktree_paths.rs:124` | 21L | Associated fn on RootItem arg; no &self |
| `worktree_path_ref` | `navigation/worktree_paths.rs:200` | 21L | Associated fn on RootItem arg; no &self |
| `resolve_member` | `navigation/pane_data.rs:88` | 15L | Associated fn on RootItem arg; no &self |
| `resolve_vendored` | `navigation/pane_data.rs:111` | 15L | Associated fn on RootItem arg; no &self |
| `member_path_ref` | `navigation/selection.rs:129` | 15L | Associated fn on RootItem arg; no &self |
| `worktree_member_display_path` | `navigation/worktree_paths.rs:72` | 14L | Associated fn on RootItem arg; no &self |
| `worktree_member_abs_path` | `navigation/worktree_paths.rs:148` | 14L | Associated fn on RootItem arg; no &self |
| `worktree_member_path_ref` | `navigation/worktree_paths.rs:224` | 14L | Associated fn on RootItem arg; no &self |
| `tracked_items_for_startup` | `async_tasks/startup_phase/toast_bodies.rs:31` | 14L | static helper over expected/seen sets, no self |
| `worktree_member_ref` | `navigation/pane_data.rs:130` | 13L | Associated fn on RootItem arg; no &self |
| `log_saturated_background_batch` | `async_tasks/poll.rs:98` | 12L | static logging helper, no self |
| `running_items_for_toast` | `async_tasks/running_toasts.rs:41` | 10L | static generic helper, no self |
| `startup_remaining_toast_body` | `async_tasks/startup_phase/toast_bodies.rs:50` | 10L | static helper over expected/seen sets, no self |
| `startup_lint_toast_body_for` | `async_tasks/startup_phase/toast_bodies.rs:66` | 10L | static test helper, no self |
| `set_confirm` | `mod.rs:864` | 1L | writes self.confirm Option (App-local field) |
| `confirm` | `mod.rs:948` | 1L | returns self.confirm.as_ref() (App-local) |
| `take_confirm` | `mod.rs:1047` | 1L | takes self.confirm Option (App-local) |

### Groups T + P — deletion list (Phases 3–9)

~88 methods. Trivial accessors and pass-throughs. Phase 3 publishes
Config/Keymap/LayoutCache; Phase 4 covers Lint/Ci/Toasts/Net/Background/Inflight;
Phases 5–9 cover Panes/Focus/Overlays/Scan/Startup. Phase 3 entries are marked
✅ below.

`projects` and `projects_mut` belong to Phase 10 — not the trivial-accessor + pass-through sweep — but are
listed here for completeness since they're one-line pass-throughs.

| Method | Cat | File:line | Body | Notes |
| --- | --- | --- | ---: | --- |
| `refresh_lint_runtime_from_config` | P | `async_tasks/config.rs:248` | 1L | one-line shim to apply_lint_config_change |
| `handle_disk_usage_batch` | P | `async_tasks/disk_handlers.rs:16` | 3L | loop calling apply_disk_usage_breakdown |
| `finish_watcher_registration_batch` | P | `async_tasks/lint_runtime.rs:36` | 3L | one-line background.send_watcher |
| `refresh_derived_state` | P | `async_tasks/tree.rs:45` | 1L | single-line bump_generation on scan |
| `ci_display_mode_for` | P | `ci.rs:157` | 1L | one-line ci forward |
| `current_branch_for` | P | `ci.rs:168` | 1L | one-line projects() git_info access |
| `ci_toggle_available_for_inner` | P | `ci.rs:172` | 1L | thin wrapper over current_branch_for |
| `current_config` ✅ | P | `mod.rs:331` | 1L | self.config.current() |
| `current_config_mut` ✅ | P | `mod.rs:337` | 1L | self.config.current_mut() |
| `resolved_dirs` | P | `mod.rs:347` | 1L | resolves include_dirs from config |
| `toast_timeout` | P | `mod.rs:357` | 1L | reads config.tui.status_flash_secs only |
| `projects` | P | `mod.rs:460` | 1L | post-Phase-1: direct project_list accessor |
| `projects_mut` | P | `mod.rs:462` | 1L | post-Phase-1: direct project_list accessor |
| `repo_fetch_cache` | P | `mod.rs:464` | 1L | self.net.github().fetch_cache() |
| `github_status` | P | `mod.rs:477` | 1L | self.net.github_status() |
| `rate_limit` | P | `mod.rs:481` | 1L | self.net.rate_limit() |
| `animation_elapsed` | P | `mod.rs:583` | 1L | self.animation_started.elapsed() |
| `lint_at_path` | P | `mod.rs:705` | 1L | self.projects().lint_at_path(path) |
| `lint_at_path_mut` | P | `mod.rs:709` | 1L | self.projects_mut().lint_at_path_mut(path) |
| `pane_data` | P | `mod.rs:731` | 1L | self.panes.pane_data() |
| `selected_project_path_for_render` | P | `mod.rs:775` | 1L | delegates to selected_project_path (project_list) |
| `mouse_pos` | P | `mod.rs:779` | 1L | self.mouse_pos |
| `set_mouse_pos` | P | `mod.rs:781` | 1L | sets `self.mouse_pos |
| `set_hovered_pane_row` | P | `mod.rs:783` | 1L | self.panes.set_hover(...) |
| `cached_fit_widths` | P | `mod.rs:791` | 1L | self.selection.fit_widths()` (post-Phase-2: project_list) |
| `cached_root_sorted` | P | `mod.rs:795` | 1L | self.selection.cached_root_sorted()` (post-Phase-2) |
| `cached_child_sorted` | P | `mod.rs:797` | 1L | self.selection.cached_child_sorted()` (post-Phase-2) |
| `focused_pane` | P | `mod.rs:801` | 1L | self.focus.current() |
| `expanded` | P | `mod.rs:807` | 1L | delegates to selection.expanded() (post-Phase-2: project_list) |
| `expanded_mut` | P | `mod.rs:810` | 1L | delegates to selection.expanded_mut() |
| `finder` | P | `mod.rs:814` | 1L | delegates to selection.finder() |
| `finder_mut` | P | `mod.rs:816` | 1L | delegates to selection.finder_mut() |
| `last_selected_path` | P | `mod.rs:824` | 1L | delegates to selection.paths().last_selected |
| `set_pending_example_run` | P | `mod.rs:828` | 1L | delegates to inflight.set_pending_example_run |
| `take_pending_example_run` | P | `mod.rs:832` | 1L | delegates to inflight.take_pending_example_run |
| `set_pending_ci_fetch` | P | `mod.rs:836` | 1L | delegates to inflight.set_pending_ci_fetch |
| `set_ci_fetch_toast` | P | `mod.rs:840` | 1L | delegates to ci.set_fetch_toast |
| `take_pending_ci_fetch` | P | `mod.rs:844` | 1L | delegates to inflight.take_pending_ci_fetch |
| `pending_cleans_mut` | P | `mod.rs:848` | 1L | delegates to inflight.pending_cleans_mut |
| `settings_edit_buf` ✅ | P | `mod.rs:950` | 1L | delegates to config.edit_buffer().buf() |
| `settings_edit_cursor` ✅ | P | `mod.rs:952` | 1L | delegates to config.edit_buffer().cursor() |
| `settings_edit_parts_mut` ✅ | P | `mod.rs:954` | 1L | delegates to config.edit_buffer_mut().parts_mut() |
| `set_settings_edit_state` ✅ | P | `mod.rs:958` | 1L | delegates to config.edit_buffer_mut().set |
| `bg_tx` | P | `mod.rs:966` | 1L | delegates to background.bg_sender() |
| `http_client` | P | `mod.rs:968` | 1L | delegates to net.http_client() |
| `ci_fetch_tx` | P | `mod.rs:970` | 1L | delegates to background.ci_fetch_sender() |
| `clean_tx` | P | `mod.rs:972` | 1L | delegates to background.clean_sender() |
| `example_tx` | P | `mod.rs:974` | 1L | delegates to background.example_sender() |
| `example_child` | P | `mod.rs:976` | 1L | delegates to inflight.example_child() |
| `example_output` | P | `mod.rs:978` | 1L | delegates to inflight.example_output() |
| `example_output_mut` | P | `mod.rs:988` | 1L | delegates to inflight.example_output_mut() |
| `example_running` | P | `mod.rs:992` | 1L | delegates to inflight.example_running() |
| `set_example_running` | P | `mod.rs:994` | 1L | delegates to inflight.set_example_running |
| `increment_data_generation` | P | `mod.rs:998` | 1L | delegates to scan.bump_generation() |
| `worktree_summary_or_compute` | P | `mod.rs:1003` | 1L | delegates to panes.worktree_summary_or_compute |
| `config_path` ✅ | P | `mod.rs:1045` | 1L | delegates to config.path() |
| `set_projects` | P | `mod.rs:1050` | 1L | writes scan.projects_mut() (test-only) |
| `set_retry_spawn_mode_for_test` | P | `mod.rs:1055` | 1L | delegates to scan.set_retry_spawn_mode |
| `scan_state_mut` | P | `mod.rs:1072` | 1L | delegates to scan.scan_state_mut() |
| `data_generation_for_test` | P | `mod.rs:1075` | 1L | delegates to scan.generation() |
| `ci_display_mode_label_for` | P | `mod.rs:1087` | 1L | inner just maps ci.display_mode_for to label |
| `poll_cpu_if_due` | P | `mod.rs:1103` | 1L | delegates to panes.cpu_tick(now) |
| `row_matches_project_path` | P | `navigation/bulk.rs:158` | 2L | One-line delegates to self.path_for_row (project_list only) |
| `try_collapse` | P | `navigation/expand.rs:92` | 1L | One-line delegates to selection.expanded_mut().remove(key) |
| `selected_project_path` | P | `navigation/selection.rs:63` | 2L | One-line delegates to selected_row + path_for_row |
| `config` ✅ | T | `mod.rs:329` | 1L | &self.config |
| `config_mut` ✅ | T | `mod.rs:345` | 1L | &mut self.config |
| `keymap` ✅ | T | `mod.rs:351` | 1L | &self.keymap |
| `keymap_mut` ✅ | T | `mod.rs:353` | 1L | &mut self.keymap |
| `toasts` | T | `mod.rs:355` | 1L | &self.toasts |
| `net` | T | `mod.rs:472` | 1L | &self.net |
| `lint` | T | `mod.rs:485` | 1L | &self.lint |
| `lint_mut` | T | `mod.rs:487` | 1L | &mut self.lint |
| `ci` | T | `mod.rs:491` | 1L | &self.ci |
| `ci_mut` | T | `mod.rs:493` | 1L | &mut self.ci |
| `layout_cache` ✅ | T | `mod.rs:727` | 1L | &self.layout_cache |
| `layout_cache_mut` ✅ | T | `mod.rs:729` | 1L | &mut self.layout_cache |
| `panes_mut` | T | `mod.rs:733` | 1L | &mut self.panes |
| `panes` | T | `mod.rs:738` | 1L | &self.panes |
| `focus` | T | `mod.rs:803` | 1L | returns &self.focus |
| `focus_mut` | T | `mod.rs:805` | 1L | returns &mut self.focus |
| `selection` | T | `mod.rs:819` | 1L | returns &self.selection (project_list post-Phase-2) |
| `selection_mut` | T | `mod.rs:822` | 1L | returns &mut self.selection |
| `background_mut` | T | `mod.rs:855` | 1L | returns &mut self.background (test-only) |
| `inflight` | T | `mod.rs:861` | 1L | returns &self.inflight (test-only) |
| `overlays` | T | `mod.rs:962` | 1L | returns &self.overlays |
| `overlays_mut` | T | `mod.rs:964` | 1L | returns &mut self.overlays |
| `scan` | T | `mod.rs:1059` | 1L | returns &self.scan |
| `scan_mut` | T | `mod.rs:1061` | 1L | returns &mut self.scan |
| `startup` | T | `mod.rs:1066` | 1L | returns &self.startup (test-only) |
| `startup_mut` | T | `mod.rs:1069` | 1L | returns &mut self.startup |
| `toasts_mut` | T | `mod.rs:1077` | 1L | returns &mut self.toasts |
| `visible_rows` | T | `navigation/cache.rs:18` | 1L | Trivial accessor: returns self.selection.visible_rows() |
