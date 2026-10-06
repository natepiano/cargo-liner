# Red lint spinner while cargo waits on a file lock — as-built

A reference for the next engineer who changes how cargo-port shows a running lint.

While any command in a project's lint run is stopped waiting for a cargo file lock, every lint spinner for that project turns the error color (red). When cargo gets the lock and output resumes, the spinner goes back to the accent color. One run can flip several times. The glyph never changes; only its color does.

Where the code lives (paths relative to `crates/cargo-port/`, except the `tui_pane/` paths, which are relative to `crates/`):
- the phase type: `src/lint/status.rs`
- the detection marker: `src/lint/constants.rs`
- detection and publishing: `src/lint/runtime/command.rs`
- spinner color and the toast projection: `src/tui/state/lint.rs`
- spinner sites: `src/tui/panes/project_list/tree_rows.rs`, `src/tui/panes/package/render.rs`, `src/tui/panes/lints/{data,render}.rs`
- toast sync: `src/tui/app/async_tasks/running_toasts.rs`
- the framework side: `tui_pane/src/toasts/{item,running_tracker,lifecycle,toast}.rs` and `tui_pane/src/toasts/render/card.rs`

---

## How it works

### The phase — `src/lint/status.rs`

- **Type.** `LintRunPhase { Executing, Blocked }`, `Executing` by default. It derives `Ord`, so `Blocked > Executing`. Re-exported as `crate::lint::LintRunPhase`.
- **Where it lives.** Inside the live variant only: `LintStatus::Running(DateTime<FixedOffset>, LintRunPhase)` and `LintStatusKind::Running(LintRunPhase)`. `LintStatus::kind()` copies the phase across.
- **Aggregation.** The phase does not change `Running`'s severity rank. When `combine` meets two `Running` values it keeps the later timestamp and `lhs_phase.max(rhs_phase)`, so `LintStatus::aggregate` over a set that holds one blocked run is `Running(_, Blocked)`. `WorktreeGroup::lint_rollup_status` (`src/project/git/worktree_group.rs`) aggregates only the running checkouts when any are running, so one blocked checkout turns the group's rollup row red.
- **Disk.** `parse_run` reads a `Running` record from `latest.json` as `Running(ts, Executing)`. `CachedLintStatus::from_lint_status` returns `None` for any `Running`.

### Detection — `src/lint/runtime/command.rs`

- **Reading output.** `run_command` spawns each command (the direnv probe included) with stdout and stderr piped. One thread per pipe runs `scan_stream(source, OutputStream, &PhaseReporter) -> Vec<u8>`, which reads with `BufReader::read_until(b'\n')` into one growing buffer. The log file gets stdout's bytes followed by stderr's, unchanged.
- **Matching.** After each line, `scan_stream` tests `line.contains(FILE_LOCK_WAIT_MARKER)` (`"waiting for file lock"`, in `src/lint/constants.rs`). A match records `Blocked` for that pipe; any other line records `Executing` for that pipe.
- **Per-pipe state.** `SharedPhase(Arc<AtomicU8>)` holds one bit per pipe (`OutputStream::Stdout`, `OutputStream::Stderr`). The combined phase is `Blocked` while either bit is set. `record(stream, phase) -> Option<LintRunPhase>` returns the combined phase only when it changed; `clear()` zeroes both bits and returns `Executing` only if a bit was set. Every method takes and returns `LintRunPhase`.
- **Publishing.** `PhaseReporter { project_root, started_at, origin, background_tx, phase: SharedPhase }` is cloned into both reader threads. Its `record` and `clear` publish only on a change, as `BackgroundMsg::LintStatus { path, status: LintStatus::Running(started_at, phase), origin }`. It sends on `background_tx` directly instead of through `publish_status`, because the status cache never accepts `Running`.
- **End of command.** After joining both reader threads, `run_command` calls `context.reporter.clear()`, so a wait notice that was a command's last output ends with the command.
- **Per-run values.** `run_commands_for_project` takes `started_at = Local::now().fixed_offset()` once, derives the run record's RFC 3339 string from it, and builds one `PhaseReporter` for the run. `execute_commands` and `run_command` receive it inside `CommandContext<'_> { project_root, manifest_path, cache_root, output_dir, child_slot, reporter }`.
- **App side.** The message takes the existing path: `App::handle_lint_status_msg` (`src/tui/app/async_tasks/lint_handlers.rs`) updates the model, the worktree rollup and the running toast.

### Spinner sites

Every site colors a running spinner through one helper in `src/tui/state/lint.rs`, `running_spinner_color(phase) -> Color` (re-exported from `state` as `lint_running_spinner_color`): `Executing` gives `tui_pane::accent_color()`, `Blocked` gives `tui_pane::error_color()`.

| Site | Code |
| --- | --- |
| Project list root row: a project, or a worktree group's rollup | `render_root_item` in `src/tui/panes/project_list/tree_rows.rs`, via `lint_cell_for(&Lint::status_for_root(..), ..)` |
| Project list child package row | `render_child_item`, via `lint_cell_for(&Lint::status_for_path(..), ..)` |
| Worktree checkout row | `render_worktree_entry`, via `lint_cell_for(&Lint::status_for_worktree(..), ..)` |
| Package detail tab, Lint row | `lint_display_style` in `src/tui/panes/package/render.rs` |
| Lints pane run-history table | `build_lint_rows` in `src/tui/panes/lints/render.rs` |

- **`lint_cell_for(status, config, animation_elapsed) -> LintCell`** pairs the color with the icon from `icon_for` in `src/tui/integration/lint_icon.rs`, which maps `LintStatusKind::Running(_)` to `ACTIVITY_SPINNER` in both phases.
- **Lints pane.** `LintsData::phases: Vec<LintRunPhase>` is aligned by index with `runs`. `build_lints_data` and, for a worktree group, `aggregate_group_lints` (both in `src/tui/panes/lints/data.rs`) fill it from each owner checkout's live `LintStatus` through `phase_of`; `run_phase` gives `Executing` to every run whose record is not `LintRunStatus::Running`.

### Toast row — `tui_pane`

- **Activity.** `TrackedItemActivity { Progressing, Stalled }`, `Progressing` by default, in `tui_pane/src/toasts/item.rs` and exported from the crate root. `TrackedItem` and `TrackedItemView` each carry an `activity` field. `TrackedItem::new` sets `Progressing`, and the toast's view builder in `toast.rs` copies it through.
- **Tracker.** `RunningTracker<K>` holds `running: HashMap<K, RunningEntry>`, where `RunningEntry { started_at: Instant, activity: TrackedItemActivity }`. `mark_running(k, started, activity)` inserts an absent key; for a key already present it sets only the activity and keeps the original `started_at`. `items_for_toast` copies each entry's activity onto its `TrackedItem`.
- **Projection.** `Lint::apply_lint_status` in `src/tui/state/lint.rs` handles `LintStatusKind::Running(phase)` with `mark_running(path, Instant::now(), activity_for(phase))`. `activity_for` maps `Blocked` to `Stalled`.
- **Propagation.** `Toasts::add_new_tracked_items` skips keys the toast already holds, so `App::sync_running_toast` calls `Toasts::refresh_tracked_item_activity(task_id, items)` right after it. That method sets `activity` on existing items by key and leaves the toast's lifetime status alone.
- **Color.** `tracked_item_line` in `tui_pane/src/toasts/render/card.rs` styles the spinner with `spinner_style(linger_progress, activity)`: `FallbackToastPalette::error` (`Color::Red`) when `Stalled`, `accent` otherwise. A completed item that is lingering shows no spinner and takes the default style.

---

## Invariants

- The phase lives only inside `LintStatus::Running`.
- The phase is never persisted. `latest.json` carries none, and every disk read is `Executing`.
- Publish on transitions only, never once per line.
- Each pipe keeps its own blocked bit. A line on stdout must not clear a wait still in force on stderr.
- The reader threads are joined and the phase cleared before the run's terminal publish, so a stale `Running` message cannot land after `Passed` or `Failed`.
- Within `Running`, `Blocked` outranks `Executing`; the phase never changes `Running`'s severity rank against other statuses.
- A status refresh keeps a tracked item's original start instant; only its activity changes.
- `tui_pane` stays domain-neutral. It knows `Progressing` and `Stalled`, never cargo or file locks.

---

## Gotchas

- **No acquire message.** Cargo prints `Blocking waiting for file lock on build directory` (or `on package cache`) to stderr and prints nothing when it gets the lock. The next line that does not match the marker is the acquire signal.
- **ANSI escapes.** Cargo wraps the leading `Blocking` word in color escapes. Matching the tail of the line avoids them, so nothing is stripped.
- **Both streams.** Both pipes are scanned. Lint tools such as cargo-mend write their own output to stderr as well.
- **Quiet commands.** `-q` or `CARGO_TERM_QUIET` suppresses the notice. A quiet command never turns red, and never turns red falsely.
- **Repeated notices.** Consecutive wait lines with no other output between them read as one continuous wait; there is no signal to separate them.
- **Three ways a run flips.** Between commands: one command releases the build-directory lock, an outside `cargo build` takes it, and the next command waits. Within one command: cargo takes and drops the package-cache lock more than once. Against itself: cargo-port's per-project workers lint in parallel and all contend for the one `~/.cargo/.package-cache` lock.
- **Killed commands.** A command killed while cargo was waiting prints nothing more. Without the clear at the end of each command, the run would stay red.

---

## Why

- **Phase inside `Running`, not in a side set on `Lint`.** A `HashSet<AbsolutePath>` beside the status could outlive the run and would never reach `LintStatus::aggregate`, so the worktree rollup row would miss it.
- **No disk write per flip.** A run can flip many times. The Lints pane reads the live status instead of `latest.json`.
- **Per-pipe bits in an `AtomicU8`.** Two reader threads update it without a lock, and it stays right when the two pipes disagree. With a single flag, a stdout line would clear a wait still held on stderr and the spinner would go blue while cargo was still stopped.
- **One `mark_running` operation.** Insert-if-absent and set-activity are one call, so a refresh can never restart the elapsed clock.
- **A separate `refresh_tracked_item_activity`.** Folding it into `add_new_tracked_items` would make that name stop describing what it does.
- **`CommandContext`.** `execute_commands` and `run_command` would otherwise exceed clippy's argument limit.

---

## Tests

| File | Test | Covers |
| --- | --- | --- |
| `src/lint/status.rs` | `aggregate_running_prefers_blocked_phase` | one blocked checkout reddens the worktree rollup |
| `src/lint/runtime/command.rs` | `scanning_output_reports_file_lock_waits_and_passes_log_bytes_through` | detection through ANSI escapes; log bytes unchanged |
| `src/lint/runtime/command.rs` | `a_wait_on_one_stream_is_not_cleared_by_output_on_the_other` | per-pipe state |
| `src/lint/runtime/command.rs` | `ending_a_command_clears_a_wait_it_never_recovered_from` | no stuck red after a kill |
| `src/lint/runtime/command.rs` | `a_running_command_publishes_its_file_lock_wait` | a full run through `run_commands_for_project` |
| `src/tui/state/lint.rs` | `a_blocked_run_turns_the_lint_spinner_red` | `Blocked` gives `error_color()` |
| `src/tui/state/lint.rs` | `a_blocked_run_stalls_its_toast_item` | the phase reaches the tracker and flips back |
| `src/tui/integration/lint_icon.rs` | `running_kind_uses_framework_activity_spinner_in_both_phases` | same glyph in both phases |
| `src/tui/app/async_tasks/running_toasts.rs` | `sync_running_toast_pushes_activity_changes_onto_existing_items` | activity reaches items already on the toast |
| `src/tui/panes/package/render.rs` | `package_lint_row_reddens_while_blocked_on_a_file_lock` | detail-tab Lint row |
| `tui_pane/src/toasts/render/drawing.rs` | `stalled_tracked_item_spinner_takes_the_palette_error_color` | toast spinner color |
| `tui_pane/src/toasts/running_tracker.rs` | `mark_running_keeps_the_original_start_and_takes_the_new_activity` | a refresh keeps the elapsed clock |
