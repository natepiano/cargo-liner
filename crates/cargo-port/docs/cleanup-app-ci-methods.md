# Cleanup: App CI methods → ProjectList

## Goal

Move CI lookups onto `ProjectList`, delete pass-through methods on `App`, and resolve display-mode internally so callers don't duplicate `app.ci.display_mode_for(p)` at every call site.

Bundles two clippy fixes that the original ticket flagged (`unused_imports` in `startup_phase/orchestrator.rs`; `clippy::too_many_lines` on `build_pane_data_common`).

## Background

Recent commits collapsed App pass-throughs into their natural owners:

- `6311b02` — `App::ci_for` → `ProjectList::ci_status_for(path, mode)`.
- `2158eb2` — CI multi-path aggregation → `RootItem::ci_status(closure)`.
- `58c6263` — `App::ci_is_fetching` → inlined into call sites.

Three CI methods on `App` remain. All are pass-throughs. They duplicate `app.ci.display_mode_for(p)` at every caller because `ProjectList::ci_status_for` and `ProjectList::ci_runs_for_display_inner` take a pre-resolved `display_mode`.

This plan inverts `6311b02`'s call-site-resolves-mode pattern (it's now `ProjectList`-resolves-mode via a `&Ci` parameter). Inversion is justified because every external caller passes the same `app.ci.display_mode_for(p)` boilerplate — the resolution belongs once, at the lookup site, not duplicated at every consumer.

## Current state

### App pass-throughs
- `App::ci_runs_for_display(&self, path: &Path) -> Vec<CiRun>` — resolves mode, calls `ProjectList::ci_runs_for_display_inner`.
- `App::selected_ci_runs(&self) -> Vec<CiRun>` — resolves selected path, calls `App::ci_runs_for_display`.

The only non-test caller of `App::ci_runs_for_display` is `App::selected_ci_runs` at `app/mod.rs:400`. Both methods disappear together; no other production code references either.

### ProjectList CI surface
- `pub(super) fn ci_status_for(&self, path, display_mode) -> Option<CiStatus>` — every external caller passes `app.ci.display_mode_for(p)`.
- `pub(super) fn ci_runs_for_display_inner(&self, path, display_mode) -> Vec<CiRun>` — `_inner` suffix exists only because `App::ci_runs_for_display` was the wrapper.
- `pub(super) fn latest_ci_run_for_path(&self, path, display_mode) -> Option<&CiRun>` — single caller: `ci_status_for`.

### Duplicated closure
`|p: &Path| pl.ci_status_for(p, app.ci.display_mode_for(p))` appears verbatim at:
- `panes/project_list.rs:334–337` (passed to `RootItem::ci_status` for multi-path aggregation).
- `panes/support.rs:1648–1654` (same purpose, inside `build_pane_data_common`).

### Asymmetric Rust filter (bug)
`panes/support.rs:1640–1655` filters single-path CI status to Rust projects only via `pl.is_rust_at_path(abs_path)`. The multi-path branch at `panes/project_list.rs:334` applies no such filter. Confirmed with the user: non-Rust projects can have CI. The filter is buggy, not intentional.

## Target state

### ProjectList — public methods

```rust
pub(super) fn ci_status_for(&self, path: &Path, ci: &Ci) -> Option<CiStatus>
```
Replaces `(path, display_mode)`. Mode resolved internally via `ci.display_mode_for(path)`. Unpublished-worktree suppression unchanged.

```rust
pub(super) fn ci_status_for_root_item(&self, item: &RootItem, ci: &Ci) -> Option<CiStatus>
```
New. Wraps `item.ci_status(|p| self.ci_status_for(p, ci))`. The name spells out that `RootItem` isn't always at a single path — `WorktreeGroup` aggregates across worktrees, so the path-taking method can't serve this case.

```rust
pub(super) fn ci_runs_for_ci_pane(&self, path: &Path, ci: &Ci) -> Vec<CiRun>
```
Renamed from `ci_runs_for_display_inner`. Mode resolved internally. The name names the only consumer (the CI pane via `build_ci_data`).

**Naming alternative considered:** `ci_runs_for_display(path, ci)`. The bare `ci_runs_for_display` becomes available once `App::ci_runs_for_display` is deleted, and naming by data returned (rather than by consumer) is the convention sibling methods follow (`git_info_for`, `ci_info_for`, `latest_ci_run_for_path`). Picked `ci_runs_for_ci_pane` per user direction; flag if you want to revert to `ci_runs_for_display`.

### File placement

`src/tui/project_list.rs` has five `impl ProjectList` blocks.
- Place `ci_status_for_root_item` next to `ci_status_for` in block #1 (around line 515).
- Place `ci_runs_for_ci_pane` (and the inlined-into-`ci_status_for` body) where `ci_runs_for_display_inner` currently sits in block #5 (around line 2511, alongside the now-removed `latest_ci_run_for_path` at 2494).

### ProjectList — private
- `latest_ci_run_for_path` is inlined into `ci_status_for`. Inlining is safe because `ci_status_for` is its only caller (verified via `rg "latest_ci_run_for_path\("` — single hit inside `ci_status_for`'s body). After inline, `ci_status_for` resolves `let display_mode = ci.display_mode_for(path);` once and uses it in the latest-run filter.

### App — deletions
- `App::ci_runs_for_display` — pass-through, deleted.
- `App::selected_ci_runs` — pass-through, deleted. The CI pane resolves selected path inline at the call site.
- Remove `use crate::ci::CiRun;` at `app/mod.rs:88` — its only consumers are the two deleted methods. (`CiRunDisplayMode` re-export at line 128 is unrelated and stays.)

## Behavior change

Deleting the `is_rust_at_path` filter in `build_pane_data_common` is a behavior change, not a pure refactor. Affected surface: the **detail pane's CI row** for single-path non-Rust projects with CI runs. The project-list row already routes through `RootItem::ci_status` (no Rust filter) and is unaffected. This matches the multi-path branch and matches `ProjectList::ci_status_for`'s contract. No tests assert the old (buggy) suppression — verified by reading `app/tests/state.rs` and `panes/tests.rs`.

## Bundled clippy fixes

### `orchestrator.rs:21` — `unused_imports`
`super::toast_bodies` is consumed only by `Startup::lint_toast_body_for`, which is `#[cfg(test)]`. In release builds the import is unused. Gate the import: `#[cfg(test)] use super::toast_bodies;` (matches the existing `HashSet` import gate at line 17–18).

### `panes/support.rs:1617` — `clippy::too_many_lines` (116/100)
`build_pane_data_common`. Style guide: `never-allow-clippy-too-many-lines.md` — extract helpers, don't `#[allow]`.

After collapsing the CI dispatch (~14 lines saved), extract two more helpers:

```rust
fn compute_in_project_bytes(
    pl: &ProjectList,
    abs_path: &Path,
) -> (Option<u64>, Option<u64>)
```
Owns the `pl.at_path(...).map_or(...)` tuple unpacking. ~3 lines saved.

```rust
fn compute_package_displays(
    app: &App,
    pl: &ProjectList,
    abs_path: &AbsolutePath,
    ci: Option<CiStatus>,
    is_worktree_group: bool,
) -> (LintDisplay, CiDisplay)
```
Owns the `Lint::package_display` + `app.ci.package_display` pair. Computes `is_rust` internally. ~12 lines saved.

Total reduction: ~29 lines, well under the 100-line limit.

**Fallback if `compute_package_displays` doesn't extract cleanly** (5 params + 2-tuple return is borderline): drop it and keep only `compute_in_project_bytes`. Combined with the CI-dispatch collapse, that still lands at ~98 lines, under the limit.

## Call site updates

### Single-line replacements

| Location | Before | After |
|---|---|---|
| `panes/project_list.rs:419` | `pl.ci_status_for(path, app.ci.display_mode_for(path))` | `pl.ci_status_for(path, &app.ci)` |
| `panes/project_list.rs:520` | `pl.ci_status_for(wt_abs, app.ci.display_mode_for(wt_abs))` | `pl.ci_status_for(wt_abs, &app.ci)` |
| `app/tests/state.rs:263, 320, 384, 399` | `app.project_list.ci_status_for(project.path(), app.ci.display_mode_for(project.path()))` | `app.project_list.ci_status_for(project.path(), &app.ci)` |
| `app/tests/state.rs:324, 388, 403` | `app.ci_runs_for_display(path)` | `app.project_list.ci_runs_for_ci_pane(path, &app.ci)` |
| `app/tests/panes.rs:578` | `pl.ci_status_for(p, app.ci.display_mode_for(p))` | `pl.ci_status_for(p, &app.ci)` |

### Multi-line replacements

**`panes/project_list.rs:334`** (inside the project-list row builder):

```rust
// before
let ci = item.ci_status(|p| {
    app.project_list
        .ci_status_for(p, app.ci.display_mode_for(p))
});
// after
let ci = app.project_list.ci_status_for_root_item(&item.item, &app.ci);
```
Note: existing `item.ci_status(...)` reaches `RootItem::ci_status` via `ProjectEntry`'s `Deref`. The new form passes `&item.item` explicitly, since `ProjectList::ci_status_for_root_item` takes `&RootItem` directly.

**`panes/support.rs::build_pane_data_common`** (CI dispatch + buggy Rust-filter deletion):

```rust
// before
let ci = wt_item.map_or_else(
    || {
        if app.project_list.is_rust_at_path(abs_path) {
            app.project_list
                .ci_status_for(abs_path, app.ci.display_mode_for(abs_path))
        } else {
            None
        }
    },
    |item| {
        item.ci_status(|p| {
            app.project_list
                .ci_status_for(p, app.ci.display_mode_for(p))
        })
    },
);
// after
let ci = wt_item.map_or_else(
    || pl.ci_status_for(abs_path, &app.ci),
    |item| pl.ci_status_for_root_item(&item.item, &app.ci),
);
```

**`app/tests/state.rs:63` and `:74`** (multi-line forms):

```rust
// before
app.project_list.ci_status_for(
    test_path("~/ws").as_path(),
    app.ci.display_mode_for(test_path("~/ws").as_path()),
)
// after
app.project_list.ci_status_for(test_path("~/ws").as_path(), &app.ci)
```
Each call collapses from 4 lines to 1.

**`panes/support.rs::build_ci_data`** (replaces `app.selected_ci_runs()`):

```rust
// before
let runs = app.selected_ci_runs();
// after
let runs = app
    .project_list
    .selected_project_path()
    .map_or_else(Vec::new, |path| {
        app.project_list.ci_runs_for_ci_pane(path, &app.ci)
    });
```
No new imports needed in `panes/support.rs` — both methods live on `ProjectList`, already accessible via `app.project_list`.

## Doc comment updates

Three stale references after the rename:
- `tui/ci_state.rs:169` — references `ProjectList::ci_status_for(path, display_mode)`. Update to new signature.
- `tui/ci_state.rs:234` (inside the doc-comment block at lines 230–247) — text "Mirrors the filtering in `App::ci_runs_for_display_inner`." Replace owner+name → `ProjectList::ci_runs_for_ci_pane`.
- `tui/project_list.rs:1549–1554` — five-line section comment whose final line names `latest_ci_run_for_path` and `ci_runs_for_display_inner` as App-level methods (already wrong; becomes more wrong post-rename). Rewrite or delete the cross-subsystem-methods sentence; keep the section header. Per `feedback_no_phase_in_comments.md`, also drop the `Phase 11` reference in the header — it's stale phase-numbering noise.

## Order of operations

Each step must leave the tree compiling at its commit boundary. Each step is a separate commit — established pattern (cf. `6311b02`, `2158eb2`, `58c6263`).

1. **Atomic CI-API migration** *(separate commit)*: signature change breaks every existing call site, so this entire step is one commit (tree compiles at the boundary).
   - Add `ci_status_for_root_item` and `ci_runs_for_ci_pane` on `ProjectList` (file placements per "File placement" section above).
   - Change `ProjectList::ci_status_for` signature from `(path, display_mode)` to `(path, ci)`.
   - Inline `latest_ci_run_for_path` into `ci_status_for`.
   - Update every caller (panes, App-internal, tests) to the new signatures.
   - Delete `App::ci_runs_for_display` and `App::selected_ci_runs`.
   - Remove the now-unused `use crate::ci::CiRun;` at `app/mod.rs:88`.
   - Update doc comments at `tui/ci_state.rs:169`, `:234`, and `tui/project_list.rs:1549–1554`.
2. **Delete the `is_rust_at_path` filter** in `build_pane_data_common` *(separate commit, behavior change reviewable in isolation)*.
3. **Apply line-count fix** *(separate commit)*: extract `compute_in_project_bytes`, then `compute_package_displays` (or fall back to the 1-helper plan if `compute_package_displays` doesn't extract cleanly). The orchestrator pattern from `~/rust/nate_style/rust/never-allow-clippy-too-many-lines.md` applies — `build_pane_data_common` is already mostly orchestration.
4. **Apply unused-import fix** *(separate commit)*: gate `super::toast_bodies` with `#[cfg(test)]`.
5. **Verify**: `cargo build && cargo +nightly fmt && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo nextest run`.

## Non-goals

- Don't change `App::ci_toggle_available_for` or other CI-adjacent methods that aren't pass-throughs.
- Don't change `latest_ci_run_for_path`'s consumers — there's only one and it disappears with the inline.
- Don't introduce a generalized `App::ci_status_lookup() -> impl Fn` style helper; the duplication target is on `ProjectList` per project conventions (move things off `App`, not add new closure factories).
- Don't tighten `RootItem::ci_status`'s pre-existing `pub(crate)` visibility (`src/project/root_item.rs:587`) — out of plan scope. (It pre-dates the user's `pub(crate)` policy and is unaffected by these changes.)
