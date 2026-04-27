# Disk-toast lag diagnosis — handoff

## Symptom

The startup toast titled **"Calculating disk usage"** keeps spinning on a
tracked entry (e.g. `~/rust/cargo-port :: 8809ms`) for several seconds
**after** the corresponding row in the project list has finished
populating its `Disk` column.

User-confirmed concrete observation (screenshot evidence):

- Project list row `cargo-port :2` displays **7.2 GiB**.
- When that row is expanded, the children show **`cargo-port` → 2.1
  GiB** and **`cargo-port-api-fix` → 5.1 GiB**, summing to 7.2 GiB.
- Toast still shows `~/rust/cargo-port :: 8809ms` with the running
  spinner glyph (not the finished/strikethrough state).

So **both worktree-checkout batches have demonstrably arrived and
populated their bytes** by the time the screenshot was taken, yet the
tracked item for the primary path (`~/rust/cargo-port`) in the
"Calculating disk usage" toast is still rendering as in-progress.

The user is **not** counting the toast's finish-linger countdown ("Closing
in N"). The complaint is specifically about the per-item **spinner**
remaining active on items whose row has already completed.

## What I established

### Layout of the data flow

1. **Phase-1 discovery** (`src/scan.rs:1660-1745`) builds
   `disk_entries: Vec<(String, AbsolutePath)>`. One entry per
   Cargo-toml-bearing dir or non-Rust git project encountered. For a
   user with both `~/rust/cargo-port` and `~/rust/cargo-port-api-fix`
   on disk, BOTH are in `disk_entries` as separate sibling paths.

2. **Project-list construction** (`AppInit::new` in
   `src/tui/app/construct.rs`, then
   `ProjectList::regroup_top_level_worktrees` in
   `src/project_list.rs:457-488`) collapses sibling worktree
   directories into a single `RootItem::Worktrees(WorktreeGroup)`
   entry whose `path()` returns `primary_path()`
   (`src/project/root_item.rs:45-51`,
   `src/project/worktree_group.rs:37`). The "primary" is the first
   matching root encountered (`find_matching_worktree_container` in
   `src/project_list.rs:627-638`).

3. **Disk-toast tracked-item set** is built in
   `App::start_startup_detail_toasts` (`src/tui/app/async_tasks.rs:797-836`)
   via `tracked_items_for_startup` (`async_tasks.rs:1078-1096`). The
   items come from
   `self.scan.startup_phases.disk.expected`, which was seeded by
   `App::reset_startup_phase_state` (`async_tasks.rs:715-761`) using
   `snapshots::initial_disk_roots(&self.projects)`
   (`src/tui/app/snapshots.rs:596-615`).

   `initial_disk_roots` walks `entry.item.path()` over the
   **post-grouping** project list and applies a nested-elimination
   (`starts_with`). For a worktree group it returns only the primary's
   path. So the toast's expected set typically contains
   `~/rust/cargo-port` and **not** `~/rust/cargo-port-api-fix`.

4. **Disk-tree dispatcher**
   (`scan::spawn_initial_disk_usage` →
   `group_disk_usage_trees` → `spawn_disk_usage_tree`,
   `src/scan.rs:1852-1928`) groups `disk_entries` (the **pre-grouping**
   list — both worktree dirs are in it) into `DiskUsageTree`s by
   `starts_with`. Sibling worktree dirs are NOT nested under each
   other, so each becomes its own tree and gets its own batch.

5. **Batch arrival**
   (`App::handle_disk_usage_batch_msg`,
   `src/tui/app/async_tasks.rs:2124-2135`):

   ```rust
   self.scan.startup_phases.disk.seen.insert(root_path.clone());
   if let Some(disk_toast) = self.scan.startup_phases.disk.toast {
       self.mark_tracked_item_completed(disk_toast, &root_path.to_string());
   }
   self.handle_disk_usage_batch(entries);
   self.maybe_log_startup_phase_completions();
   ```

   - Inserts `root_path` into the `seen` set (counter for phase-level
     completion).
   - String-matches `root_path.to_string()` against tracked-item keys
     to flip per-item `completed_at` (drives the spinner).
   - Then applies the per-entry bytes via
     `handle_disk_usage_batch` → `apply_disk_usage_breakdown` →
     `apply_disk_usage` (writes
     `project.disk_usage_bytes = Some(bytes)` for whichever
     `at_path_mut(path)` resolves to —
     `src/tui/app/async_tasks.rs:1622-1626`).

6. **Row aggregation**
   (`RootItem::disk_usage_bytes`,
   `src/project/root_item.rs:155-174`): a `WorktreeGroup` row's
   displayed total is `sum_disk(primary, linked)`
   (`root_item.rs:539-547`). Crucially, `sum_disk` uses `.flatten()`,
   so it sums whichever worktree dirs have `Some(bytes)` and
   ignores those still at `None` — `None` is treated as missing,
   not zero.

7. **Per-item rendering**
   (`tui::toasts::render::tracked_item_line`,
   `src/tui/toasts/render.rs:383-434`): `is_running` is
   `item.linger_progress.is_none()`, where `linger_progress` is
   produced in `ToastManager::active`
   (`src/tui/toasts/manager.rs:594-619`) only when
   `item.completed_at = Some(...)` AND `toast.item_linger = Some(...)`.
   So a still-spinning row means **`completed_at` is `None`** at that
   moment.

### Theories I ran and what survived

#### Theory A — *toast tracks all roots (including hidden non-Rust); list is filtered*

Speculation: `initial_disk_roots` includes hidden non-Rust roots, so
the toast waits on roots the visible list has filtered out.

**Status: not the root cause for this symptom.** The user explicitly
showed the spinner on a row that *is* in the visible list, with
finished bytes — not on a hidden root.

#### Theory B — *first sibling wins for the row, while toast waits on the primary's specific batch*

Speculation: `apply_disk_usage_breakdown` populates a worktree-grouped
row from any sibling's batch. `mark_tracked_item_completed` only
matches the exact primary key. So the row could read "done" via the
linked worktree's batch while the toast still waits on the
primary's batch, which might be the slower one.

**Status: defensible until the user expanded the row.** Once the
expanded view showed both child rows populated, the primary's batch
demonstrably **had** arrived (the `cargo-port` row alone is at 2.1
GiB). The primary's `apply_disk_usage_breakdown` ran. The
`mark_tracked_item_completed("~/rust/cargo-port")` call ran in the
same `handle_disk_usage_batch_msg` invocation. Both took effect on
the row side. The toast item should have flipped. **It didn't.**

#### Theory C — *partial total ⇒ partial population*

Speculation: 7.2 GiB might be just *one* worktree's bytes; the user is
reading the total they expect, not the total that's actually shown.

**Status: ruled out by the user's expanded-row screenshot** (2.1 + 5.1
= 7.2). Both child entries have `Some(bytes)`.

### What that leaves us with

**There is a real bug**, not just a UX/aggregation mismatch. Both of
these are demonstrably true at the moment of the screenshot:

1. The primary worktree directory (`~/rust/cargo-port`) has had its
   batch arrive — the row's child is populated.
2. The toast tracked item keyed on `~/rust/cargo-port` is still
   running.

The candidate explanations narrow to:

- **C1.** The `mark_item_completed` string match is silently failing
  to find the primary's tracked item. Possible reasons: path
  normalization difference between the key used at toast-seed time
  (`AbsolutePath::From<&AbsolutePath>` →
  `value.to_string()` →
  `Path::display()`) versus the key used at completion time
  (`root_path.to_string()` on a different `AbsolutePath` value).
  Both *should* produce identical strings, but if the two
  `AbsolutePath` values were constructed via different paths
  (canonicalize vs. raw, trailing slash, symlink resolution), they
  could disagree byte-for-byte.

- **C2.** Some later call is overwriting the tracked-items vec and
  losing the `completed_at` field. The only writer that fully
  replaces the vec is `ToastManager::set_tracked_items`
  (`src/tui/toasts/manager.rs:402-423`), which is called from
  `App::set_task_tracked_items` (`src/tui/app/query.rs:157-168`).
  Direct callers in `tui::app::async_tasks` are at lines 793, 806,
  817, 829, 1197, 1252. Lines 1197 and 1252 are inside
  `sync_tracked_path_toast` (lint/clean toasts only — different
  task ids). Lines 793/806/817/829 are startup-only. So in steady
  state, `completed_at` shouldn't be wiped — but worth checking
  whether some path runs `start_startup_detail_toasts` again, e.g.
  on rescan.

- **C3.** The tracked item's key for `~/rust/cargo-port` is being
  derived from a *different* path than I think — e.g. the worktree
  primary is actually attached such that `entry.item.path()`
  returns the linked-worktree path rather than the bare
  `~/rust/cargo-port` path, depending on `linked_worktree_identity`
  resolution. This would still print the same display label
  (`home_relative_path` is computed independently) but break the
  key match.

The single most direct way to distinguish C1/C2/C3 is **instrumentation**.

## Concrete instrumentation plan

Add `tracing::info!` at three sites and capture one startup run.

### Site 1 — toast-seed key

`src/tui/app/async_tasks.rs:1078-1096`
(`tracked_items_for_startup`): log the keys being inserted into the
toast at seed time, **before** any conversion away from the
`AbsolutePath`.

```rust
tracing::info!(
    target: "disk_diagnose",
    key = %path,                      // raw AbsolutePath display
    key_string = %path.to_string(),   // the string the key will store
    label = %label,
    "disk_toast_seed_item"
);
```

### Site 2 — batch arrival keys

`src/tui/app/async_tasks.rs:2124-2135`
(`handle_disk_usage_batch_msg`): log the `root_path` being matched and
the result of the match.

```rust
let key = root_path.to_string();
let matched = self
    .toasts
    .tracked_items_keys_for(disk_toast)        // helper to add
    .iter()
    .any(|k| k == &key);
tracing::info!(
    target: "disk_diagnose",
    root_path = %root_path,
    key_string = %key,
    matched_a_tracked_item = matched,
    "disk_batch_arrival"
);
```

(Add a small read-only helper on `ToastManager` to list the current
tracked-item keys for a task id, so we don't have to pierce
encapsulation at the call site.)

### Site 3 — `mark_item_completed` outcome

`src/tui/toasts/manager.rs:522-534`: log whether the key was found.

```rust
let mut hit = false;
for toast in &mut self.toasts {
    if toast.task_id == Some(task_id) {
        for item in &mut toast.tracked_items {
            if item.key.as_str() == key && item.completed_at.is_none() {
                item.completed_at = Some(now);
                hit = true;
                break;
            }
        }
    }
}
tracing::info!(
    target: "disk_diagnose",
    task_id = ?task_id,
    key,
    hit,
    "mark_item_completed_outcome"
);
```

### Reading the output

Filter logs for `disk_diagnose`. Compare:

- The set of `disk_toast_seed_item.key_string` values.
- Each `disk_batch_arrival.key_string` and its `matched_a_tracked_item`
  flag.
- Each `mark_item_completed_outcome.key` and its `hit` flag.

Three diagnostic verdicts fall out:

1. **`hit = false` for the primary's batch** → C1 (string mismatch).
   Compare the seed string against the arrival string for the same
   project and look for a normalization difference.

2. **`hit = true` for the primary, but spinner still shown** → C2
   (something is later replacing the tracked-items vec) or a
   rendering bug in the linger logic. Check whether
   `set_tracked_items` runs after the hit.

3. **No `disk_batch_arrival` event for `~/rust/cargo-port` at all** →
   the primary's batch never went through this code path; some
   other handler updated the row. Search who else writes
   `project.disk_usage_bytes`.

## Pointers (file:line)

- App-side disk toast lifecycle: `src/tui/app/async_tasks.rs:715-836`,
  `:862-884`, `:1078-1096`, `:2114-2136`.
- Phase-state completion logic: `src/tui/app/phase_state.rs:50-108`.
- Toast manager (set / mark / render):
  `src/tui/toasts/manager.rs:55-82` (key conversions),
  `:402-423` (`set_tracked_items`),
  `:454-465` (`complete_missing_items`),
  `:522-534` (`mark_item_completed`),
  `:594-619` (`active` view → `linger_progress`).
- Toast rendering: `src/tui/toasts/render.rs:383-434`
  (`tracked_item_line` — `is_running` derivation).
- Scan-side disk dispatcher: `src/scan.rs:1652-1745` (phase-1
  discovery), `:1852-1928` (tree grouping + spawn),
  `:1966-2001` (`dir_sizes_for_tree`).
- Project-list grouping & primary resolution:
  `src/project_list.rs:457-488`, `:627-638`;
  `src/project/root_item.rs:45-51`, `:155-174`, `:539-547`;
  `src/project/worktree_group.rs:37`.

---

## Directive for the next session

**Before writing any code, ask the user to confirm they want to
proceed.** Then:

1. Add the three instrumentation sites above (and the small
   `ToastManager` helper to enumerate tracked keys for a task id).
2. Use the project's `tracing` subscriber to emit the new lines.
   Don't introduce a new logging crate. Don't change visibility of
   types beyond what the helper requires.
3. Have the user run the tool through one cold startup that
   reproduces the symptom (cargo-port + cargo-port-api-fix on disk,
   spinner visibly hanging on the row that is otherwise populated).
4. Read the resulting log output and decide between C1 / C2 / C3 per
   "Reading the output" above.
5. Once the cause is identified, propose the fix in chat **before**
   implementing — particularly if the fix touches the toast key
   convention, since that's a small public surface that other
   subsystems consume.

Do not commit any of the instrumentation. Once the diagnosis lands
and the fix ships, revert the `tracing::info!` lines in the same
commit as the fix or in a follow-up `chore(...)`.
