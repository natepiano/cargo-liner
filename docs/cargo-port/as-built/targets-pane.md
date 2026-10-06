# Targets pane — as-built

A reference for the next engineer who modifies cargo-port's Targets pane or the owned-run code it launches.

The Targets pane lists the selected project's runnable targets (binaries, examples and benches) in one table that fills the pane body. `Enter` launches the selected target in debug mode and `r` in release mode. Either becomes cargo-port's single **owned run**: its output goes to the Output pane, which takes over the bottom row, and `Esc` stops it. Before anything may signal the run's process, the process is bound to a `ProcessIdentity` (its PID plus an OS creation token). That identity is checked again right before every signal.

Where the code lives (paths relative to `crates/cargo-port/`):
- the pane: `src/tui/panes/targets/`
- its actions: `src/tui/panes/actions.rs`
- its keymap host: `src/tui/integration/framework_keymap/targets_pane.rs`
- the owned run: `src/tui/state/{inflight,owned_run_process_actor}.rs` and `src/tui/terminal/processes.rs`
- process identity: `src/process_observation/identity.rs`
- the render loop: `src/tui/terminal/event_loop.rs`

Watching or stopping targets that cargo-port did not launch is cargo-tile's job; see [Why](#why).

---

## How it works

### Layout — `src/tui/panes/targets/{render,constants,pane,mod}.rs`

- **Entry point.** `render_targets_pane_body(frame, area, &mut TargetsPane) -> PaneFrameChrome` draws the table when `pane.content().is_some_and(TargetsData::has_targets)`. Otherwise `render_empty_targets` clears the viewport surface and the row rects, and returns chrome titled ` No Targets ` that is never drawn focused.
- **Region.** `targets_region(table_rows) -> Region` is `Region::stack(vec![Region::rows(table_rows, Size::Fill).header()])`: one box that fills `tui_pane::frame_inner(area)`, with one chrome row for the ratatui `Table` header. `region.place(content_inner, cursor, &[scroll_offset])` returns the placed boxes, indexed by `TABLE_BOX` (`0`, defined in `constants.rs`).
- **Table rendering.** `render_targets_table`:
  - sets the viewport's content area and visible row count from `placed[TABLE_BOX].content`;
  - renders a stateful `Table`, whose `TableState` selects the cursor row and whose offset stays in step with `viewport.scroll_offset()` in both directions;
  - records one `(Rect, row_index)` per visible data row in `TargetsPane::row_rects`;
  - pushes `tui_pane::overflow_affordance_label` (the pager) onto the bottom border.
- **Columns.**
  - Target: one leading space, then the name, truncated with `…` at `name_max`.
  - Source: drawn in the label color.
  - Kind: right-aligned, colored by `RunTargetKind::color()`.
- **Column widths.** `compute_layout` sizes them in this order:
  - Kind is fixed at `RunTargetKind::padded_label_width()`.
  - Two 1-column gaps are reserved (`TARGET_TABLE_COLUMN_SPACING` × `TARGET_TABLE_GAP_COUNT`).
  - Source gets the widest source label (never narrower than the `Source` header), capped at what is left.
  - Target gets the rest. So Target shrinks before Source does, and Source and Kind stay against the right edge.
- **Title.** `build_targets_title(focus: PaneFocusState, cursor: usize, data: &TargetsData) -> String` returns `tui_pane::prefixed_pane_title("Targets", &PaneTitleCount::Grouped(groups))`. There is one group for each nonempty kind: `Binary`, `Examples`, `Benches`. While the pane is `Active`, the group that contains the cursor also shows the cursor's index within that group.
- **The pane struct.** `TargetsPane { viewport, focus, content: Option<TargetsData>, row_rects }`. `hit_test_at` maps a screen position through `row_rects` to `HoverTarget::PaneRow { pane: PaneId::Targets, row }`. One `Viewport` covers the whole table, and its position is an index into `build_target_list_from_data(data)`.

### Data — `src/tui/panes/targets/data.rs`

- **`TargetsData { binaries, examples, benches: Vec<TargetEntry> }`.** `has_targets()` is true when any of the three lists is nonempty. `build_target_list_from_data` flattens them as binaries, then examples, then benches. The table, the title groups, launching and copying all index into that order.
- **`TargetEntry`** has the fields `name`, `display_name`, `run_target_kind`, `source: TargetSource`, `project_path`, `package_name`, `src_path` and `required_features`.
- **`from_workspace_metadata(&WorkspaceMetadata, selected_path)`** builds the lists:
  - Which packages: selecting the checkout root includes every package; selecting a member includes only that member.
  - What each package contributes: the bin named after the package becomes `Binary`, and every bench becomes `Bench`.
  - Every example becomes `Example`. Its category comes from `examples/<category>/<file>.rs`, except that a directory named after the target itself (`examples/<name>/main.rs`) is not a category.
  - Source labels: `TargetSource::workspace_root` is used only for the root package of a multi-package workspace. Every other package gets `member(<package>)`.
  - Sort order: by source (root, then members, then worktrees), then by name. Within examples, root-level examples come before categorized ones.
- **`lookup_targets_data(app, abs_path, worktree_item)`**:
  - For a worktree group that renders as a group, it merges the targets of each visible checkout and relabels them `TargetSource::worktree("<checkout>/<package>")`.
  - For any other path, it reads the metadata store, through `containing_checkout_root` and then `get`.
  - When there is no metadata, it returns `TargetsData::default()`. There is no fallback that parses `Cargo.toml` by hand.
- **When the data is rebuilt.** `App::ensure_detail_cached` (`src/tui/app/navigation/cache.rs`) writes the data through `Panes::set_detail_data(stamp, package, git, targets)`, or clears it with `clear_detail_data`. Both are keyed on `DetailCacheKey { visible_row, generation }`, so a bump of the scan generation rebuilds the data.

### Reachability and navigation — `src/tui/panes/{spec,actions}.rs`, `src/tui/app/mod.rs`, `framework_keymap/targets_pane.rs`

- **Tab order.** `panes::behavior(PaneId::Targets)` is `PaneBehavior::DetailTargets`. For that behavior, `App::is_pane_tabbable` returns `self.panes.targets.content().is_some_and(panes::TargetsData::has_targets)`. The keymap host's `tab_stop()` is `TabStop::ordered(TARGETS_TAB_ORDER, targets_is_tabbable)`.
- **Keymap scope `targets`.**
  - `TargetsAction::Activate`: TOML key `activate`, default `Enter`.
  - `TargetsAction::ReleaseBuild`: TOML key `release_build`, default `r`.
  - Their `visibility` is `targets_run_visibility(ctx)`, which applies the same `has_targets` test.
  - `CopySelection` copies the selected entry's `src_path`.
- **Moving the cursor.** `dispatch_navigation_action` sends `AppPaneId::Targets`, together with Lang, Cpu and Git, to `navigate_detail`. That function moves the viewport chosen by `active_detail_pane(app)` (picked from `base_focus()`), with Left acting as Up and Right as Down.
- **Key folding.** `normalize_nav` (`src/tui/input/dispatch.rs`) turns horizontal keys, vim `h`/`l` included, into vertical moves for `DetailTargets`. Edge-scroll focus advance reads the cursor through `active_detail_viewport`.

### Launch, Output pane, stop

1. **Dispatch.** `dispatch_targets_action` sends `Activate` through `handle_detail_enter` to `handle_target_action(app, BuildMode::Debug)`, and `ReleaseBuild` to `handle_target_action(app, BuildMode::Release)`.
2. **Queue.** `handle_target_action` takes the entry at `viewport.pos()` and builds a `PendingExampleRun` (`src/tui/panes/pane_data/pending.rs`) with these fields: `abs_path`, `target_name`, `display_path`, `cargo_package_invocation: CargoPackageInvocation::Package(package_name)`, `run_target_kind`, `build_mode` and `required_features`. It then calls `app.inflight.queue_owned_run`.
   - If the result is `OwnedRunLaunchAdmission::AlreadyActive` or `IdentitiesExhausted`, a `Run refused` toast appears.
   - `target_display_path` builds the run's label from the selected project-list row: root label, then checkout label, then source segments, then target name. A binary named after its package adds no target-name segment.
3. **Spawn.** After the frame is drawn, `spawn_pending_background_tasks` (`event_loop.rs`) calls `begin_owned_run_launch()`. On `OwnedRunLaunchStart::Starting(id)` it calls `processes::spawn_owned_run_process(app, id)`.
4. **The command.** `cargo_command_for_owned_run` builds one of three commands, then adds options:
   - Command: `cargo run`, `cargo run --example <name>` or `cargo bench --bench <name>`.
   - Options: `--release`, `--package <name>`, `--features <required-features>` and `--color always`.
   - It runs in the project directory, with stdin set to null and stdout and stderr piped.
   - On Unix, `isolate_owned_process` sets `process_group(0)`, so the cargo child's PID is also its process-group ID.
5. **Identity gate.** `observe_current_process_identity(child.id())` must return `Verified`. On any other result, `discard_unverified_owned_process_group` cleans up the child, and the start fails with `Failed to establish a verified process identity`.
6. **Output readers and actor.**
   - Two reader threads read stdout and stderr. A line ending in `\n` is sent as `OwnedRunEvent::Output`. A line ending in `\r` is sent as `Progress`, which replaces the previous last line. Both go through `app.background.example_sender()`.
   - `OwnedRunProcessActor::prepare` takes ownership of the child, the `VerifiedProcessIdentity`, the readers and the sender.
   - `activate_owned_run` moves the lifecycle to `Running`, the actor worker starts, and `Started` is sent.
7. **Event handling.** `App::poll_example_msgs` (`src/tui/app/async_tasks/poll.rs`) applies each event to `Inflight`. Every event carries an `OwnedRunId`, and an event for any other run is ignored. `Finished` calls `finish_owned_run`, which appends `── done ──` (or keeps `── killed ──` as the last line), and then calls `panes.output.on_process_exit()`.
8. **Output pane.** `OutputPresentation::derive(owned_run.output_state(), owned_run.running_label())` is `Owned` while the run has retained lines. That makes the Output pane visible and switches the layout to `BottomRow::Output`: Output spans the whole bottom row in place of Lints and CI Runs, which drop out of the tab order. `owned_run_output_replaced` moves focus to Output when the buffer goes from empty to nonempty.
9. **`Esc`.** `classify_output_cancel_preflight` decides what `Esc` does. It runs after the confirmation modal and before the finder and sccache overlays, the globals, and pane dispatch. The first matching case wins:
   - If an Output visual selection is active, it is exited.
   - Otherwise, a raw `Esc` while `owned_run().is_running()` calls `terminal::signal_owned_run`.
   - Otherwise, the key bound to `output.cancel`, with captured output on screen, calls `clear_owned_run_output`. If the Output pane had focus, focus moves to Targets.

   The Output pane's own `OutputAction::Cancel` (`cancel_owned_output`) behaves the same way, and its bar label reads `done`, `stop` or `close` to match.
10. **Stopping.** `signal_owned_run` submits the `OwnedRunTerminationToken` from `OwnedRunTermination::Available` and returns `Submitted` or `NotSubmitted`. The lifecycle then enters `TerminationRequestPending`. The actor replies with a `TerminationOutcome`, and `reconcile_owned_run_termination` handles it:
    - `Sent` moves the run to `Stopping` and appends `── killed ──`.
    - `ProcessAlreadyReaped`, `IdentityNoLongerCurrent`, `SignalFailed` and `Refused` return the run to `Running`, so the user can try to stop it again.

**The owned-run slot.** `OwnedRun` is a single slot, and only `Inflight::new` creates it. Its `OwnedRunLifecycle` states are `Absent`, `Queued`, `Starting`, `Running`, `TerminationRequestPending`, `Stopping`, `RetainedSuccess`, `GoneAfterSignal` and `Failed`. Each `OwnedRunId` comes from a `NonZeroU64` counter that is never reset; once the counter is exhausted, every launch is refused.

### Process identity — `src/process_observation/identity.rs`

`src/process_observation/mod.rs` exposes only the `identity` module.

- **`ProcessIdentity { pid: u32, creation_token }`.** The token is the time the OS started the process: start ticks from `/proc/<pid>/stat` on Linux, `FILETIME` on Windows, and `proc_pid_rusage`'s `ri_proc_start_abstime` on macOS. A reused PID therefore compares unequal to the old identity.
- **How it is read.** Reads go through `processkit::process_info(pid)`. On macOS the code also queries `libc::proc_pid_rusage`, and requires the processkit start time to be the same before and after that query.
- **`observe_current_process_identity(pid) -> CurrentProcessIdentityObservation`** reads the identity and immediately checks it again. It returns `Verified(VerifiedProcessIdentity)`, `InitialIdentityUnavailable`, `ReplacedDuringRevalidation` or `RevalidationUnavailable`.
- **`revalidate_strong_process_identity(&ProcessIdentity)`** reads the PID again and returns `Current`, `Replaced(ProcessIdentity)` or `Unavailable(..)`.

Production callers (one each):
- **`src/tui/terminal/processes.rs`** calls `observe_current_process_identity`: `spawn_owned_run_process` checks the new child with it (step 5 above).
- **`src/tui/state/owned_run_process_actor.rs`** calls `revalidate_strong_process_identity`:
  - `OwnedProcessGroupTerminationCapability::from_verified_root` consumes the `VerifiedProcessIdentity`.
  - Its `signal()` checks the identity again, then calls `support::terminate_process_group`, which sends SIGTERM to the whole group through rustix.
  - On Unix, if the group signal fails, it checks the identity once more and then runs `kill -TERM <root pid>`.
  - On other platforms, it refreshes that one PID in `sysinfo`, checks the identity again, and sends `Signal::Term`.

`src/tui/state/inflight.rs` imports `ProcessIdentity` only under `#[cfg(test)]`, for `ProcessIdentity::for_test` fixtures. In production it holds the actor and hands out an `OwnedRunTerminationToken`, which carries no PID.

**The actor's worker thread.** One worker thread owns the `Child` and the termination capability, which cannot be cloned. It alternates between `try_wait()` on the child and `recv_timeout(OWNED_RUN_CHILD_REAP_POLL_INTERVAL)` (25 ms) on its command channel, so a signal never runs at the same time as reaping. After the child is reaped, the worker stops accepting commands, answers any token still queued with `ProcessAlreadyReaped`, waits for the reader threads to finish, and sends `Finished`.

### Event loop timing — `src/tui/terminal/{event_loop,frame_metrics}.rs`

- **What wakes the loop.** `wait_for_event` puts these receivers in a `Select`: input, background, CI fetch, clean, and owned-run (`example_rx`), plus CPU samples while the monitor is sampling. It then blocks in `ready_timeout(app.animation_timeout())`.
- **What each wake does.** It drains every source, ticks the CPU pane, refreshes caches, draws the frame, and then runs `spawn_pending_background_tasks`.
- **The timeout.** `App::animation_timeout()` (`src/tui/app/mod.rs`) returns one of two values, shortened if the next toast visual change is sooner. Nothing else sets the timeout.
  - `ANIMATION_TICK` (80 ms) while something is animating: scan shimmers, running lints, `Inflight::needs_animation()` (a clean in progress or an owned run running), or PR check polls.
  - `IDLE_HEARTBEAT` (1 s) otherwise, which keeps the mtime-polled config, keymap and theme reloads running while idle.
- **Frame speed.** `FrameMetrics::speed()` returns a `FrameSpeed`, either `BelowSlowThreshold` or `Slow`, measured against `tui_pane::SLOW_FRAME_MS`. `log_performance` emits the `slow_frame` trace only for `Slow` frames. Time spent waiting is not counted in `frame_elapsed`.

---

## Invariants

1. **Nothing in cargo-port polls the process table.** Every process read looks up one PID: `processkit::process_info` at launch and before each signal, plus, off Unix, a one-PID `sysinfo` refresh when stopping. The CPU pane samples total CPU only (`refresh_cpu_all`). The actor worker waits only on its own child. `tui_pane`'s macOS backdrop does scan every process, but it sits behind the `backdrop` feature, which cargo-port does not enable.
2. **Neighboring subsystems keep their own data.**
   - The CPU pane (`tui_pane::CpuMonitor`, in `src/tui/panes/cpu/`).
   - The Output pane, which gets everything it shows from the owned run.
   - Inflight and owned-run termination (`inflight.rs`, `owned_run_process_actor.rs`, `terminal/processes.rs`).
   - The lint runtime and `src/tui/app/async_tasks/running_toasts.rs`.
   - The project list's running indicator, which covers lints only (`project_lint_is_running` in `src/tui/project_list/list.rs`).
3. **Reachability is one test.** `is_pane_tabbable` and `targets_run_visibility` must both use `TargetsData::has_targets`, so a change to one means changing the other.
4. **One owned run at a time, and its ids are never reused.** `queue` refuses a launch while any live lifecycle exists. `OwnedRunId` is never reissued; once ids run out, launches are refused.
5. **No signal without a current identity.**
   - A run reaches `Running` only after its identity is `Verified`.
   - Every signal path checks the identity again first.
   - The termination capability never leaves the actor worker, and the token that stands for it carries no PID.
   - The one exception is `discard_unverified_owned_process_group`, which cleans up a child whose launch failed before it became a run.
6. **Every Cargo dependency has at least one named user.**
   - `sysinfo`: `src/scan/disk_usage.rs` and the non-Unix owned-run stop.
   - `libc`: the macOS `query_macos_monotonic_process_start` in `identity.rs`.
   - `processkit`: `identity.rs`.
   - `sha2`: `src/project/cargo/metadata_store.rs` and `src/lint/paths.rs`.
7. **No dead code.** There is no `#[allow(dead_code)]`, and no item whose only caller was removed. Any new `#[allow]` goes to the user for review.

---

## Calibration / gotchas

- **`DOT_CARGO_DIR` is shared.** `DOT_CARGO_DIR` in `src/constants.rs` is read by `lint/trigger.rs`, `watcher/roots.rs` and `project/cargo/metadata_store.rs`.
- **Short panes happen.** Pane heights of 2 rows or fewer occur, so the Targets layout has no minimum-height debug assertion.
- **The screenshot callouts are hand-drawn.** Callouts 1–3 in `assets/pane-targets-numbered.png` were drawn by hand, and no script regenerates them. A visible change to the pane needs a new screenshot with the callouts redrawn by hand.
- **Some layout helpers exist only in tests.** `TABLE_CHROME` (`1`, the header row) lives only in the `render.rs` test module, next to `target_col_width_from`, which is also test-only.
- **The loop still wakes every second when idle.** That wake comes from `IDLE_HEARTBEAT` in `animation_timeout()`. Each wake runs `poll_background_frame`, which reloads the config, keymap and themes when their files' modification times change. A longer heartbeat delays those reloads while the app is idle.
- **`Esc` stops a run from anywhere on the app surface.** The stop check looks for the raw `KeyCode::Esc`, not whatever key is bound to `output.cancel`, and it runs before the finder and sccache overlays see the key. So `Esc` stops a running target from any pane, even while either overlay is open. Closing retained output, by contrast, follows the binding.
- **Closing Output can focus an empty Targets pane.** Closing the Output pane moves focus to Targets without checking `has_targets`. If the project has no targets, `targets_run_visibility` hides the run actions and `handle_target_action` finds no entry, so nothing runs.
- **Launching takes one frame.** Input dispatch only queues the run; the process is spawned after the next draw, in `spawn_pending_background_tasks`.

---

## Why

- **Watching and stopping arbitrary running targets belongs to cargo-tile.** cargo-port controls only the run it launched. The README and the CHANGELOG send users who want that to cargo-tile, installed with `cargo install --git https://github.com/natepiano/cargo-liner cargo-tile`. It is not linked on crates.io, because the published 0.1.0 is behind the workspace version.
- **A `kill` entry in a user's keymap gets a warning, not a migration.**
  - The framework keymap is built with `ignore_unknown_entries()` (`src/tui/integration/framework_keymap/builder.rs`), so a leftover `[targets] kill` entry is skipped and recorded in `Keymap::unknown_warnings()`.
  - `load_initial_keymap` (`src/tui/app/async_tasks/config.rs`) shows that warning, and drops the duplicate `UnknownAction` report from `keymap/load.rs`.
  - Because the warning covers it, `migrate_removed_action_keys` has no `kill` step.
- **The loop has no process deadline.** There is no process scan, so `animation_timeout()` alone sets how long the loop waits. The 1 s heartbeat that remains is there for the file-based reloads.
- **Stopping is tied to the process identity, not just the PID.** A PID can be reused by a new process. The creation token makes a reused PID compare unequal, and checking it again right before each signal keeps a stop from reaching the new process.
- **One actor owns both the child and the right to signal it.** Because signaling and reaping happen on one thread, a group signal never races the reaping of the group leader. Requests and results travel over channels, so sending a signal never blocks the TUI loop.
- **`OwnedRunId` is never reissued.** Late output, progress and termination messages are matched to a run by this id, so a reused id would attach one run's output to a different run.
