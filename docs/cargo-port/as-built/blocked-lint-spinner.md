# Red lint spinner while cargo waits on a file lock

Status: **implemented**. The plan below is kept as the record of the
investigation; the "As built" section at the end notes where the shipped code
differs from the plan.

Goal: when any command in a lint run is stopped waiting for a cargo file lock,
every lint spinner for that project turns red. When the lock is acquired the
spinner returns to `accent_color()`. A single run can flip colors several times.

The user approved red (`error_color()` / `palette.error`), not yellow.

---

## Investigation findings

### Execution model

- `src/lint/runtime/supervisor.rs:611-647` — `WorkerContext::run`. One worker
  thread per watched project. Projects lint concurrently with each other.
- `src/lint/runtime/command.rs:292-353` — `execute_commands` runs the configured
  commands **strictly sequentially**. Each is a separate `/bin/sh -c` → separate
  `cargo` process → each acquires and releases cargo's locks independently.
- `src/lint/runtime/command.rs:430-522` — `run_command`. Output is fully
  buffered: two threads `read_to_end` stdout/stderr, are joined, then the bytes
  are written to the log (stdout bytes followed by stderr bytes). Nothing
  inspects output live today.
- Status is published exactly twice per run: run start (`command.rs:172`) and
  the terminal write (`command.rs:234`). `execute_commands` rewrites
  `latest.json` after each command but publishes no message.

### The user's hypothesis is correct, and understates it

Three independent ways a run flips blue → red → blue:

1. **Between commands.** `mend` releases the build-directory lock, an external
   `cargo build` queued behind it takes the lock, then `clippy` waits.
2. **Within one command.**
   `~/Library/Caches/cargo-port/lint-runs/bevy_catenary-*/mend-latest.log` holds
   **four** `Blocking waiting for file lock on package cache` lines inside a
   single command — cargo takes and drops the package-cache lock repeatedly.
3. **cargo-port against itself.** Per-project workers run in parallel and all
   contend for the one global `~/.cargo/.package-cache` lock.

### Detection signal

Cargo writes `Blocking waiting for file lock on {build directory|package cache}`
to stderr and prints **nothing** on acquisition. The next output line is the
acquire signal. Verified in `nateroids-*/mend-latest.log`:

```
    Blocking waiting for file lock on build directory
    Checking nateroids v0.18.0 (/Users/natemccoy/rust/nateroids)
```

Caveats:

- Log lines carry ANSI escapes around the `Blocking` word
  (`\x1b[1m\x1b[92m    Blocking\x1b[0m waiting for ...`). Matching must tolerate
  or strip them. Matching on the substring `waiting for file lock` sidesteps the
  escapes entirely, since the escapes wrap the status word only.
- Cargo suppresses the line under `-q` / `CARGO_TERM_QUIET`. The user's lint
  shim (`~/.claude/scripts/lint/lint`, invoked as `lint mend|clippy|doc`)
  uses neither, so all three of their commands emit it. A user-authored quiet
  command degrades to never-red — no false red.
- cargo-mend writes its own summary to stderr too; scan **both** streams.

### Spinner sites — five, not four

| # | Where | Code | Color source |
|---|---|---|---|
| 1 | Project row Lint column | `src/tui/state/lint.rs:366-381` `lint_cell_for`, via `tui/panes/project_list/tree_rows.rs:405` | `accent_color()` |
| 2 | Worktree child row | same fn, via `tree_rows.rs:464` | same |
| 3 | Worktree group rollup row | same fn, via `tree_rows.rs:571`, status from `src/project/git/worktree_group.rs:86-102` | same |
| 4 | Package detail tab Lint row | `src/tui/panes/package/render.rs:1152-1162` `lint_display_style` | `accent_color()` |
| 5 | Lints pane run-history table | `src/tui/panes/lints/render.rs:96-103` | `accent_color()`, driven by `LintRunStatus::Running` read from `latest.json` |
| — | Toast per-project row | `tui_pane/src/toasts/render/card.rs:282-326` `tracked_item_line` | `palette.accent` (framework-owned) |

Sites 1–3 share one function, so they are one edit. The user listed four sites
(1/2+3 counted as two, 4, and the toast); site 5 was proposed by the assistant
and the user did not object — **include it**.

---

## Decisions taken

- Phase lives **inside** `LintStatus::Running`, not in a side set on `Lint`. A
  side `HashSet<AbsolutePath>` can outlive the run and never reaches
  `LintStatus::aggregate`, so the worktree rollup row would not work.
- Reuse the existing publish path (`publish_status` → `BackgroundMsg::LintStatus`
  → `handle_lint_status_msg`). No new channel.
- Do **not** persist the phase to `latest.json` — that would mean a disk write
  per flip. Site 5 reads the live `LintStatus` instead.
- Framework-generic naming in `tui_pane`: the toast layer must not learn about
  cargo locks.

---

## Implementation plan

### 1. `src/lint/status.rs` — the domain type

Add above `LintStatusKind` (vocabulary before anchor type, per
`module-structure`):

```rust
/// Whether a live lint command is making progress or is stopped waiting for
/// a cargo file lock (`Blocking waiting for file lock on ...` on the command's
/// stderr). Detected by `crate::lint::runtime::command`; consumed by the
/// spinner color in `crate::tui::state::lint_cell_for`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum LintRunPhase {
    #[default]
    Executing,
    Blocked,
}
```

Then:

- `LintStatusKind::Running(LintRunPhase)`
- `LintStatus::Running(DateTime<FixedOffset>, LintRunPhase)`
- `kind()` — `Self::Running(_, phase) => LintStatusKind::Running(*phase)`
- `severity_rank()` — leave `Running` at 3 (Failed stays 4 and still wins).
- `combine()` `Ordering::Equal` arm — `(Running(lhs_ts, lhs_phase), Running(rhs_ts, rhs_phase)) => Running(lhs_ts.max(rhs_ts), lhs_phase.max(rhs_phase))`.
  This is what makes one blocked checkout turn the whole worktree rollup red
  (`Ord` on `LintRunPhase` puts `Blocked` above `Executing`).
- `CachedLintStatus::from_lint_status` — `LintStatus::Running(..)` still `None`.
- `parse_run` (`status.rs:181`) — `LintStatus::Running(ts, LintRunPhase::Executing)`.
  A disk read never knows about blocking; that is correct, since the only disk
  read during a live run happens at run start.
- Tests: add a case asserting `aggregate` of `[Running(ts, Executing), Running(ts, Blocked)]` is `Running(_, Blocked)`.

`src/lint/mod.rs:48` — add `pub use status::LintRunPhase;`.

### 2. `src/lint/constants.rs` — the detection marker

```rust
// src lint runtime command
/// Substring cargo prints on stderr while waiting for a file lock, e.g.
/// `Blocking waiting for file lock on build directory`. Cargo prints nothing
/// when it acquires the lock, so the next non-matching output line is the
/// acquire signal.
pub(super) const FILE_LOCK_WAIT_MARKER: &str = "waiting for file lock";
```

Keep the file's existing section-comment layout and alphabetical order within
the section.

### 3. `src/lint/runtime/command.rs` — live detection

- `run_commands_for_project` parses `run.started_at` once
  (`status::parse_timestamp`) and threads the resulting
  `DateTime<FixedOffset>` into `execute_commands`.
- `execute_commands`: before each command, if the shared phase flag is
  `Blocked`, reset it to `Executing` and publish.
- `run_command` gains a reporter value (clone into both reader threads) holding
  `project_root: AbsolutePath`, `background_tx: Sender<BackgroundMsg>`,
  `origin: LintRunOrigin`, `started_at: DateTime<FixedOffset>`, and an
  `Arc<AtomicBool>` (or `Arc<Mutex<LintRunPhase>>`) shared by both streams so
  only transitions publish.
- Reader threads switch from `read_to_end` to `BufReader::read_until(b'\n')`,
  appending each line into the same accumulating `Vec<u8>` so the written log
  bytes stay byte-identical to today. After each line, test it against
  `FILE_LOCK_WAIT_MARKER`; a match sets `Blocked`, any other line clears to
  `Executing`. Publish only on change, as
  `BackgroundMsg::LintStatus { path, status: LintStatus::Running(started_at, phase), origin }`.
  `publish_status` already skips the status cache for `Running`
  (`CachedLintStatus::from_lint_status` returns `None`), so sending direct from
  the reader thread is safe.
- **Ordering is safe**: the reader threads are joined before the terminal
  publish, so a stale "executing" message cannot land after "passed".
- Style: prefer an enum over a bare `bool` for the shared flag if it does not
  force a `Mutex` where an `AtomicBool` would do — an `AtomicBool` plus
  `LintRunPhase::from(bool)` keeps both the style rule and the lock-free read.

### 4. `tui_pane` — generic per-item activity

Framework must stay domain-neutral. Name it after progress, not cargo.

- New enum (in `tui_pane/src/toasts/item.rs`, next to `TrackedItem`):

  ```rust
  /// Whether a tracked item is progressing or stalled waiting on something
  /// outside its control. `Stalled` colors the item's spinner
  /// `FallbackToastPalette::error` in `tracked_item_line`.
  pub enum TrackedItemActivity { #[default] Progressing, Stalled }
  ```

- `TrackedItem` gains `pub activity: TrackedItemActivity`; `TrackedItem::new`
  defaults it to `Progressing`.
- `TrackedItemView` gains the same field; `Toast::view` copies it through
  (`tui_pane/src/toasts/view.rs` / `toast.rs`).
- `tracked_item_line` (`card.rs:313-325`) — the spinner span picks
  `palette.error` when `Stalled`, `palette.accent` otherwise.
  `FallbackToastPalette` already has `error: Color::Red`
  (`tui_pane/src/toasts/render/drawing.rs:40-48`).
- `RunningTracker<K>` (`tui_pane/src/toasts/running_tracker.rs`) currently stores
  `HashMap<K, Instant>`. Change the value to a small struct holding
  `started_at: Instant` and `activity: TrackedItemActivity`, and add a setter for
  the activity. `items_for_toast` fills `TrackedItem::activity` from it.
  Callers that touch `.running` directly: `src/tui/state/lint.rs:177,228,233,238,243`
  (`entry().or_insert_with(Instant::now)`, `len`, `contains_key`),
  `src/tui/state/inflight.rs` (test `contains_key`), `src/tui/app/mod.rs` tests
  (`contains_key`). `insert`/`remove` signatures need one pass.
- **The propagation gap**: `Toasts::add_new_tracked_items`
  (`tui_pane/src/toasts/lifecycle.rs:221-240`) skips keys that already exist, so
  an activity flip on a live item would never reach the toast. Add an additive
  method — `refresh_tracked_item_activity(task_id, items: &[TrackedItem])` —
  that updates `activity` on existing keys, and call it from
  `App::sync_running_toast` (`src/tui/app/async_tasks/running_toasts.rs:90-130`)
  right after `add_new_tracked_items`. Do not overload `add_new_tracked_items`;
  the name would stop describing what it does.

### 5. App-side plumbing

- `src/tui/state/lint.rs`
  - `apply_lint_status` (`:167-184`) — the `Running` arm takes the phase and
    writes it into the tracker entry (insert with `Progressing`, then set the
    activity so a re-published `Running` updates it).
  - `lint_cell_for` (`:366-381`) — `LintStatus::Running(_, LintRunPhase::Blocked)`
    → `error_color()`; `Running(_, Executing)` → `accent_color()`.
  - `package_display` (`:355`) — `matches!(status, LintStatus::Running(..))`.
- `src/tui/integration/lint_icon.rs:16` — `LintStatusKind::Running(_)` arm
  (same `ACTIVITY_SPINNER` glyph for both phases; only the color differs).
- `src/tui/panes/package/render.rs:1158` — split the `Running` arm so
  `Blocked` uses `error_color()`; `Stale` keeps `accent_color()`.
- `src/tui/app/mod.rs:384` — the `#[cfg(test)]` `lint_cell` delegator mirrors
  `lint_cell_for`; it and the ~30 test constructors in that file need the new
  payload.
- `src/project/git/worktree_group.rs:95` — `matches!(s, LintStatus::Running(..))`.
- `src/tui/project_list/list.rs` — one `Some(crate::lint::LintStatus::Running(_))`
  pattern.
- `src/tui/app/async_tasks/lint_handlers.rs:22,66,75,98` — patterns become
  `Running(..)`; `handle_lint_status_msg` already routes model + rollup + toast,
  so no structural change.

### 6. Site 5 — Lints pane run table

`src/tui/panes/lints/data.rs` `build_lints_data` produces `LintsData { runs,
sizes, owner_paths, owner_of, project_kind }`. Add a `phases: Vec<LintRunPhase>`
parallel to `runs` (matching the existing parallel-vec convention), filled from
the live status of each row's owner path
(`Lint::status_for_path(&app.project_list, path)`); non-running rows get
`Executing`. `build_lint_rows` (`src/tui/panes/lints/render.rs:96-103`) then
picks `error_color()` for a `Running` row whose phase is `Blocked`.
`aggregate_group_lints` in the same file needs the same fill.

### 7. Verification

- `cargo build && cargo +nightly fmt`
- `cargo nextest run`
- `cargo mend` (per the per-phase convention)
- Manual: run two lints against the same project while holding the build-dir
  lock from another terminal (`cargo build` in a loop), confirm red on all five
  sites plus the toast row, and confirm the flip back to accent.
- `cargo install --path .` after the change is confirmed working.

---

## Open items

None blocking. Site 5 was proposed and not objected to; it is in the plan.

---

## As built

Where the shipped code differs from the plan above:

- **Per-pipe blocked state, not one flag.** `SharedPhase` in
  `src/lint/runtime/command.rs` is an `Arc<AtomicU8>` bitmask with one bit per
  pipe (`OutputStream::Stdout` / `Stderr`), not a single `AtomicBool`. With one
  flag, a line arriving on stdout would clear a wait still held on stderr and
  the spinner would go blue while cargo was still stopped. The combined phase is
  `Blocked` while *either* bit is set. Every method on the type takes and
  returns `LintRunPhase`; the bitmask never escapes.
- **`PhaseReporter::clear()` at end of command**, rather than a reset before
  each command. Clearing when both pipes hit EOF covers the same case plus the
  one the plan missed: a command killed while cargo was still waiting would
  otherwise leave the run stuck red.
- **`started_at` is not parsed back.** `run_commands_for_project` now builds
  `Local::now().fixed_offset()` first and derives the `started_at` string from
  it, so no `parse_timestamp` round-trip is needed.
- **Parameter bundling.** `execute_commands` and `run_command` would have gone
  past clippy's argument limit, so the per-run values they share moved into a
  `CommandContext<'_>` struct.
- **`RunningTracker::mark_running`** replaced the planned pair of
  `insert` + `set_activity`. Inserting when absent and setting the activity
  either way is one operation, and it keeps an already-running key's original
  start instant so a status refresh never restarts its elapsed clock. The map's
  value type is now a public `RunningEntry { started_at, activity }`.
- **`App::lint_cell` now delegates** to `state::lint_cell_for` instead of
  duplicating its body. Its doc comment already claimed it was a thin
  delegator; the phase color would otherwise have had to be written twice.

Tests added:

| File | Test | Covers |
|---|---|---|
| `src/lint/status.rs` | `aggregate_running_prefers_blocked_phase` | one blocked checkout reddens the worktree rollup |
| `src/lint/runtime/command.rs` | `scanning_output_reports_file_lock_waits_and_passes_log_bytes_through` | detection through ANSI escapes; log bytes unchanged |
| `src/lint/runtime/command.rs` | `a_wait_on_one_stream_is_not_cleared_by_output_on_the_other` | per-pipe state |
| `src/lint/runtime/command.rs` | `ending_a_command_clears_a_wait_it_never_recovered_from` | no stuck-red after a kill |
| `src/lint/runtime/command.rs` | `a_running_command_publishes_its_file_lock_wait` | full run through `run_commands_for_project` |
| `src/tui/state/lint.rs` | `a_blocked_run_turns_the_lint_spinner_red` | `Blocked` ⇒ `error_color()` |
| `src/tui/state/lint.rs` | `a_blocked_run_stalls_its_toast_item` | phase reaches the tracker and flips back |
| `src/tui/app/async_tasks/running_toasts.rs` | `sync_running_toast_pushes_activity_changes_onto_existing_items` | the propagation gap past `add_new_tracked_items` |
| `src/tui/panes/package/render.rs` | `package_lint_row_reddens_while_blocked_on_a_file_lock` | detail-tab Lint row |
| `tui_pane` | `stalled_tracked_item_spinner_takes_the_palette_error_color` | toast spinner color |
| `tui_pane` | `mark_running_keeps_the_original_start_and_takes_the_new_activity` | elapsed clock is not restarted by a refresh |

The `lint_icon` running test now covers both phases.

The detection rule was also replayed offline against the 66 `*-latest.log` files
in `~/Library/Caches/cargo-port/lint-runs/`: 23 of them contain a real cargo
file-lock wait, and each produces exactly one `Blocked` and one `Executing`
transition. Consecutive `Blocking` lines with no output between them read as one
continuous wait — cargo prints nothing on acquisition, so there is no signal
that would separate them.
