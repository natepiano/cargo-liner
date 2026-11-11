# Lint history and file-lock waits

cargo-port shows a running lint's spinner in the error color while Cargo reports a file-lock wait.
The spinner returns to the accent color when output resumes.
Its glyph and elapsed start time remain unchanged.

Paths below are relative to `crates/cargo-port/src/` unless they begin with `crates/`.

## Status and detection

`lint/status.rs` defines `LintRunPhase::{Executing, Blocked}`.
Only `LintStatus::Running(timestamp, phase)` carries this state.
`LintStatusKind::Running(phase)` exposes it to drawing code.
Within running values, Blocked outranks Executing during aggregation.
The phase does not change Running's severity relative to completed statuses.
A worktree-group rollup uses running checkouts when any are running.

`lint/runtime/command.rs` reads both output pipes through `scan_stream`.
A line containing `FILE_LOCK_WAIT_MARKER`, which is `waiting for file lock`, marks that pipe Blocked.
Any other line marks that pipe Executing.
`SharedPhase(Arc<AtomicU8>)` stores a separate bit for each pipe.
One pipe cannot clear the other's wait.

`PhaseReporter` sends a `BackgroundMsg::LintStatus` only when the combined phase changes.
Each run uses one original start timestamp.
After both reader threads join, command completion clears the blocked state before publishing completion.
The stored log retains stdout bytes followed by stderr bytes.
The phase is not persisted in the run record.
Disk reads of a running record start in Executing.

## Application and toast presentation

`tui/state/lint.rs::running_spinner_color(phase)` supplies the spinner color.
Project rows, Package, and Lint Runs call the same helper.
`tui/panes/lints/data.rs` aligns its phase vector with the displayed run vector.
The live status supplies the phase rather than a disk rewrite for each transition.

Blocked maps to `TrackedItemActivity::Stalled` in the shared toast tracker.
Executing maps to `Progressing`.
`RunningTracker::mark_running` changes activity without resetting an existing item's start instant.
`tui/app/async_tasks/running_toasts.rs` refreshes activities on existing toast items.

`crates/tui_pane/src/toasts/render/card.rs` renders tracked items.
`spinner_style` receives `TrackedItemLifecycle` and `TrackedItemActivity`.
A stalled running item uses the palette's error color.
A completed fading item has no activity spinner.
The framework knows nothing about Cargo locks.

## Lint Runs layout

`tui/panes/lints/render.rs` builds rows and their drawn geometry.
Headers, dates, and values keep whole text where their columns fit.
Narrow layouts stack each complete run instead of clipping records.
The height, page count, and hit areas use those drawn lines.
A click on a stacked continuation line selects the same run.
Each run keeps one blank margin cell at each border.
The Cache size row shows its cap once.

## Invariants and gotchas

1. Persist no transient file-lock phase.
2. Publish phase transitions rather than every output line.
3. Combine both pipes before choosing the phase.
4. Clear the phase before the terminal status publication.
5. Keep a tracked item's original start time after a status refresh.
6. Match selection and hit areas to the drawn run rows.

Cargo prints no explicit lock-acquired message.
The next nonmatching line is the observable recovery signal.
ANSI escapes around Blocking do not affect matching the marker's trailing text.
Quiet commands can omit the marker and therefore show no blocked phase.
A killed quiet command still receives the completion clear.
Consecutive wait lines represent one continuous observable wait.

Per-pipe bits avoid a false recovery when stdout resumes while stderr remains blocked.
Live status avoids repeated disk writes during a command with many lock waits.
