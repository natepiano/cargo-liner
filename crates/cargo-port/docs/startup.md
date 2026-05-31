# Startup progress panel

## Goal

Replace the current per-phase startup toasts — each rendering a truncated
"remaining: foo, bar, baz" item list — with a single consolidated panel that
shows **one progress bar per type of background work**, each advancing to 100%.

The panel answers one question: *what background work is still running, and how
far along is each kind relative to the others.* It reflects work completion and
relative timing, not which data matters most.

## Motivation

The startup toasts track background work that must finish before the UI is fully
populated. Today each tracked phase renders its own toast whose body is the set
of not-yet-seen item paths. Two problems:

- The remaining-item list is variable-width text that churns as items drop off,
  and truncates on wide fleets. It conveys *which* items are pending but not
  *how far along* the phase is.
- Several independent toasts appearing, filling, and dismissing on their own
  schedules read as busy and jittery.

A fixed-width bar per phase conveys proportion the list cannot, and a single
container that holds every bar shows relative timing in one glance (e.g. disk at
100% while metadata sits at 30% — you learn what is slow).

## Non-goals

- Do not turn the panel into a triage surface. Bar inclusion is about "is this
  countable background work," not "does this data drive a decision."
- Do not gate first paint on the panel. Phases already run off the critical
  path; the panel only observes them.
- Do not show a bar for work that has no knowable denominator (`expected` never
  set).

## Current state

The `Startup` subsystem (`src/tui/app/async_tasks/startup_phase/`) owns the
phase trackers. Each is a `KeyedPhase<K>` (an `expected: Option<HashSet<K>>`
plus a `seen: HashSet<K>`) or a `CountedPhase`. Tracked today:

| Phase | Tracker | Key | Denominator |
|---|---|---|---|
| disk | `KeyedPhase<AbsolutePath>` | project root | fixed (project roots) |
| git | `KeyedPhase<AbsolutePath>` | project root | fixed |
| repo | `KeyedPhase<OwnerRepo>` | GitHub repo | **dynamic** (grows as remotes resolve) |
| metadata | `KeyedPhase<AbsolutePath>` | workspace root | fixed |
| lint | `KeyedPhase<AbsolutePath>` (`lint_phase`) + `CountedPhase` (`lint_count`) | project root | fixed |

Notes that affect the panel:

- `metadata` keys on workspace root; disk/git/lint key on project root. One
  `seen` per key, so the bar formula is unaffected, but the row counts differ.
- `lint` is two fields. The row tracks `lint_phase` (terminal Passed/Failed per
  project). `lint_phase` has custom completion logic (`tracker.rs:29-52`) that
  refuses to complete on an empty `expected` set; `lint_count` is internal
  cardinality and is not a row.
- The toast body today is an app-computed string: `remaining_toast_body`
  (`toast_bodies.rs`) filters `expected − seen` and renders it via
  `format_toast_items`. The Startup toast itself currently uses a tracked-items
  checklist (`set_tracked_items` + empty body, `tracker.rs:115-116`).

Two kinds of background work are **not** tracked today and stream in silently:

- **Language stats** (tokei) → secondary Languages pane.
- **Test counts** → detail-pane Tests sub-section.

Both are local, bounded reads feeding non-primary surfaces. Under this plan they
each become a bar row: the inclusion test is "is it countable background work,"
and both qualify. The panel will linger until they finish (tokei on a large
fleet can take tens of seconds after the UI is usable); that is accepted —
showing every startup element advancing together is the point.

## Design

### One panel, N rows

Render **one stable "Startup" toast containing N labeled mini-bars**, not one
toast per phase.

```
Startup
  disk       ▓▓▓▓▓▓▓▓ 100%
  metadata   ▓▓▓░░░░░  38%
  git        ▓▓▓▓▓▓░░  75%
  lint       ▓▓▓▓▓▓▓▓ 100%
  languages  ▓▓▓▓▓▓▓▓ 100%
  tests      ▓▓▓▓▓░░░  62%
```

Rows fill in place; the whole panel dismisses once every row is terminal — at
100% past its minimum-visible floor, or failed (see [Failure is
terminal](#failure-is-terminal--startup-always-finishes)). One container removes
the pop-in / pop-out churn of independent toasts, and relative timing is visible
at a glance.

### Typed progress model

Per-row state is a typed value, not scattered flags. A render step turns rows
into glyphs; the trackers never emit pre-rendered strings.

```rust
struct ProgressRow {
    label: &'static str,   // from the STARTUP_PHASE_* constants
    state: ProgressState,
}

enum ProgressState {
    /// Denominator known and stable; render a bar.
    Progress { seen: usize, expected: usize },
    /// Denominator still growing; render an indeterminate placeholder.
    Waiting,
    /// Terminal failure; render a stalled marker. Detail lives in an error toast.
    Failed,
    /// At 100% but the minimum-visible floor has not elapsed; render full.
    CompleteHeld,
}
```

Each tracked phase derives its `ProgressState` from its own fields plus the
clock — no phase holds a `String`. The denominator is itself a typed enum rather
than `Option<HashSet<K>>` plus a separate stabilized flag, so the
stabilized-but-unknown combination is unrepresentable:

```rust
enum Denominator<K: Eq + Hash> {
    Unknown,            // expected not yet known — row omitted
    Growing(HashSet<K>),// known but still resolving — render Waiting
    Stable(HashSet<K>), // final — render a Progress bar
}
```

`KeyedPhase` replaces its `expected: Option<HashSet<K>>` and the proposed
`expected_stabilized` bool with a single `Denominator<K>`; both phase trackers
also gain `first_seen`, a typed failure marker, and a `progress_state(now,
min_visible)` method. `Denominator` maps directly: `Unknown` ⇒ row omitted;
`Growing` ⇒ `Waiting`; `Stable` ⇒ `Progress`. `KeyedPhase` and `CountedPhase`
evolve independently — `CountedPhase` counts rather than keys, so it does not
share `Denominator`; `progress_state` returns `ProgressState` either way, so
callers never see the internal representation.

The bar fraction is subset-based and clamped to `[0, 1]`, carried as a
`Percentage(0..=100)` newtype computed once in `progress_state` (not raw counts
the renderer re-divides): `seen` can exceed `expected` (nested disk entries), and
an empty `Stable` set renders 100% without dividing by zero.

### Rendering is app-side

The bar is a unicode string (`▓`/`░` + percent) assembled from `ProgressRow`s.
No framework change is required: `tui_pane::Toasts::update_task_body`
(`tui_pane/src/toasts/lifecycle.rs:107`) already exists. The app gains a thin
`App::update_task_body` wrapper mirroring `set_task_tracked_items` /
`finish_task_toast` (`src/tui/app/mod.rs:542-549`). The Startup toast moves from
its tracked-items checklist to a body of bars, refreshed each phase tick via
`update_task_body`.

A richer rendering (real `Gauge` widgets, animated fill, per-row color) would be
a future `tui_pane` change. It is not part of this plan.

## Behaviors

### Minimum visible duration

A phase that completes in under ~100 ms would flash 0 → 100 % as noise. Each row
carries a **minimum-visible duration**: `first_seen` is stamped on the row's
first progress, and the row holds `CompleteHeld` until `max(complete_at,
first_seen + min_visible)`. The panel dismisses only after every row is past its
floor.

### Dynamic denominators

The repo phase's denominator grows as git remotes resolve, so a raw percentage
would jump backward. While its `Denominator` is `Growing` the row renders
`Waiting`. It flips to `Stable` once the git phase completes (all local repos
enumerated) and the remote-resolution pass that derives GitHub repos has drained
— debounced so a late `RepoFetchQueued` does not reopen `Growing`; any arriving
after `Stable` is dropped. Only then does the row switch to a `Progress` bar.

### Failure is terminal — startup always finishes

A failed phase never holds the panel open. When a phase cannot finish
(rate-limited GitHub, `cargo metadata` error, fetch error, or a per-phase
timeout), it:

1. pops a separate **warning/error toast** naming the reason, and
2. enters the `Failed` state, which the dismissal gate counts as done.

The dismissal gate is **"every row Complete (past floor) or Failed."** The
actionable detail lives in the per-failure toast; the bar row only marks the
stalled state. Nothing — a hung fetch, a silent task, an unresolved remote — can
hang the panel.

A row is exactly one of `Progress` / `Waiting` / `CompleteHeld` / `Failed` — the
states are mutually exclusive. A timeout transitions straight to `Failed`; a
phase that fails before its `first_seen` is stamped skips the min-visible floor
(immediate). Once the panel has dismissed, a late result for an already-`Failed`
phase no-ops with a debug log — it never touches a stale toast id or mutates
`seen`.

## Implementation phases

### 1. Typed panel with bars and minimum-visible duration

- Add the `Denominator<K>` enum (`Unknown` / `Stable` here; `Growing` arrives
  in phase 2 with its first constructor), the `ProgressState` enum (`Progress` /
  `CompleteHeld` here; `Waiting` / `Failed` arrive in phase 2), `ProgressRow`,
  and the `Percentage` newtype. Each enum variant — and the `FailureReason`
  marker — is introduced alongside the code that first constructs it, so the
  workspace's no-dead-code rule stays satisfied; the schema completes in phase 2.
- Replace `KeyedPhase`'s `expected: Option<HashSet<K>>` with `Denominator<K>`
  (~15 compiler-caught `.expected` sites; the incremental `ensure_expected()` +
  `.insert()` pattern becomes `Denominator::insert`). Add `first_seen:
  Option<Instant>` to `KeyedPhase` (the only tracker that renders a row),
  stamped when the row first becomes visible.
- Add `progress_state(now, min_visible) -> Option<ProgressState>` (None omits
  the row), `first_seen`, `is_omitted`, and a default `gate_satisfied` /
  `min_visible_elapsed` to the `PhaseCompletion` trait so the panel assembler
  and dismissal gate stay generic over phase type; it derives `Progress` /
  `CompleteHeld` here.
- Add `startup_panel_body(rows)` and a `progress_bar(percentage)` helper as
  `pub(super)` free fns in `toast_bodies.rs`, mirroring `remaining_toast_body`.
  Labels come from the `STARTUP_PHASE_*` constants; `STARTUP_BAR_FILLED` /
  `STARTUP_BAR_EMPTY` / `STARTUP_BAR_WIDTH` / `STARTUP_ROW_MIN_VISIBLE` are new
  constants in `constants.rs`.
- Add the `App::update_task_body` wrapper; replace the Startup toast's
  tracked-items checklist with a bar body refreshed each phase tick and each
  frame (`tick_startup_panel`, driven from `poll_background_frame`, so the
  minimum-visible floor can close the panel even with no new message). This
  removes the checklist's auto-finish-on-last-item, so the panel is closed
  explicitly when every row is terminal; drop the `mark_tracked_item_completed`
  calls on the umbrella toast (`tracker.rs`, `lint_handlers.rs`) and the GitHub
  tracked-item add in `repo_handlers.rs` (`STARTUP_PHASE_GITHUB` is removed here
  and reintroduced with the repo row in phase 2). The detail toasts' tracked
  items are unaffected.

  > **Post-ship correction (2026-05-31):** keeping the per-phase detail toasts
  > alongside the panel broke the feature. The toast stack renders newest-at-
  > bottom and drops the oldest card when it runs out of vertical room
  > (`render_bottom_up`), so the panel — created first, and competing for height
  > budget against the taller detail toasts — was crowded out of a normal-height
  > terminal entirely, and starved to its title row when it did fit. The three
  > startup detail toasts ("Calculating disk usage", "Scanning local git repos",
  > "Running cargo metadata") and all their plumbing (the `toast` field on
  > `KeyedPhase`/`CountedPhase`, `take_toast`, `tracked_items`, the
  > `*_toast_body` builders) were removed; the panel is now the sole startup
  > surface. Tradeoff: the panel shows an aggregate percent per phase, not the
  > per-item pending list the detail toasts carried.
  >
  > **Post-ship addition (2026-05-31):** a **crates.io** row was added and the
  > **GitHub** row was fixed to hold the panel until in-flight network fetches
  > finish (previously the panel closed at ~1.4s, while the GitHub and crates.io
  > fetches were still running in their own toasts). The crates.io denominator is
  > seeded upfront from `collect_publishable_crates_io_targets()` (so the row gets
  > a determinate bar; empty list omits the row); the GitHub denominator now
  > keeps growing while the panel is open so a repo fetch queued after local-git
  > stabilizes still feeds the row and reopens the gate. The standalone
  > "Retrieving GitHub repo details" / "Fetching crates.io info" toasts are
  > suppressed while the panel is open (they return for steady-state fetches).
  > The panel now stays visible for the full startup-network window (~5s on the
  > dev tree). Rows are now `[…; 8]`: disk, git, GitHub, crates.io, metadata,
  > lint, languages, tests.
  >
  > **Post-ship addition (2026-05-31):** rows now carry a **gradient color**
  > (linear white at 0% → full green at 100%) and, when a row is still in
  > progress after `STARTUP_ROW_DETAIL_DELAY` (1s), the **item it is currently
  > working on** appears after the percent. The gradient required a per-line
  > colored toast body (`ToastBody::Colored`, threaded through
  > `ToastView.body_line_colors` to the card renderer); the panel switched from
  > the string body to `start_colored_task` / `update_task_colored`. The current
  > item comes from the live in-flight fetch for the network rows and a
  > deterministic pending key (`KeyedPhase::pending_sample`) for the batch rows;
  > rows that finish under 1s never show it.
  >
  > **Post-ship addition (2026-05-31):** on completion the panel now lingers for
  > the standard `finished_task_visible` window with a **"Closing in N"
  > countdown** like every other toast, instead of vanishing at once. Because the
  > panel renders a body (no tracked items), `finish_task` would have given it a
  > zero linger; `Toasts::finish_task_lingering` sets an explicit linger and
  > `App::finish_body_toast_with_countdown` passes `finished_task_visible`. The
  > test-count row label is **"Test counts"** (not "Tests").
- Refactor `maybe_complete_startup_ready` to iterate the tracked phases
  (`[&dyn PhaseCompletion; 5]`: disk, git, repo, metadata, lint) via
  `gate_satisfied` rather than hard-coding each one, gating on the per-row floor
  — `now >= max(complete_at, first_seen + min_visible)` for every phase.
  Iterating means a later row (phase 2 repo, phase 3 languages/tests) cannot
  silently miss the gate.
- Wire the four phases with stable denominators (disk, git, metadata, lint) as
  rows — repo joins in phase 2 with its stabilization logic, so no phase ever
  ships a regressing repo bar. Lint tracks `lint_phase` and resets to `Unknown`
  (omitted) until a real lint run is queued, never a premature 100%; its first
  queued run transitions it to `Stable` and stamps `first_seen`. (`lint_count`
  stays internal cardinality and no longer gates the panel.)

### 2. Repo stabilization, failure, and per-phase timeout

- Add the `Denominator::Growing` and `ProgressState::Waiting` / `Failed`
  variants and the `FailureReason` marker (each deferred from phase 1 until its
  first constructor lands here), and reintroduce the `STARTUP_PHASE_GITHUB`
  label constant for the repo row.
- Repo's `Denominator` starts `Growing` and flips to `Stable` on the debounced
  stabilization signal above; `Growing` ⇒ `Waiting`. Once stabilized, drop any
  late `RepoFetchQueued` rather than reopening `Growing`. Wire the repo row
  here, now that `Waiting` exists — it never renders a regressing percentage.
- Add the typed failure marker (`FailureReason { RateLimited, FetchError,
  Timeout(Duration) }`, a `failure: Option<FailureReason>` field on
  `KeyedPhase`) and wire the `Failed` `ProgressState` variant; extend
  `gate_satisfied` / `progress_state` so a failed phase is terminal. (No
  `MetadataError` variant: `handle_cargo_metadata_msg` already toasts on `Err`
  and marks the workspace `seen`, so metadata self-completes and never stalls
  the row — a single workspace error must not fail the whole metadata row.)
- Mark the repo row `Failed` when GitHub is confirmed down during startup.
  Rather than a new `RepoFetchFailed` message, hook the existing
  `confirm_service_unreachable` path (after the grace window, where the
  service-unavailable toast is already pushed): `RateLimited` ⇒
  `FailureReason::RateLimited`, `Unreachable` ⇒ `FailureReason::FetchError`,
  via `App::fail_startup_repo_phase`. The service toast names the reason, so the
  row adds none of its own.
- Add a single generous timeout (`STARTUP_ROW_TIMEOUT`), applied uniformly to
  every phase and sized so legitimately slow work (tokei, test counts) finishes
  well inside it. A phase visible past it is marked `Failed(Timeout)` and pops
  one timeout toast. Driven from the frame tick (`sweep_startup_timeouts` in
  `tick_startup_panel`), not message arrival, so a silently-hung phase still
  trips it. This is the backstop that guarantees "startup always finishes" for
  every row, including a repo whose fetches hang without a service signal.
- Treat git as terminal on complete *or* failed (`is_terminal()` on the trait):
  `maybe_complete_startup_repo` gates on git terminal, then stabilizes the repo
  denominator, so a timed-out git releases repo instead of both hanging.
- Guard late results: `maybe_log_startup_phase_completions` returns early once
  `startup.complete_at` is set — the closed panel toast is never touched (the
  per-handler `seen` bookkeeping is idempotent and harmless).
- The dismissal gate (from phase 1) already counts a failed row as terminal, so
  startup always finishes.

### 3. Languages and test-count rows

- Add `languages: KeyedPhase<AbsolutePath>` and `tests: KeyedPhase<AbsolutePath>`
  to `Startup`.
- Initialize their `expected` in `reset_startup_phase_state` from the disk-root
  set (`initial_disk_roots`), before any language/test batch can arrive. The
  initial tokei / test-count scans are spawned from the same `disk_entries` as
  disk usage (`cargo_metadata.rs`) and emit one batch entry per root, so the
  disk-root set is the correct denominator: every root appears in some batch, so
  `seen` converges to a superset of `expected`.
- Add `App::mark_startup_languages_seen` / `mark_startup_tests_seen` hooks,
  called from the `LanguageStatsBatch` / `TestCountsBatch` dispatch arms
  *before* the `ProjectList` handler consumes the entries. Each marks its row's
  `seen` from the entry keys (project roots). A plain `complete_once` in
  `maybe_log_startup_phase_completions` records their completion timestamp for
  the min-visible floor (no detail toast or extra logging).
- Both rows join the panel, the iterating dismissal gate, and the timeout sweep
  automatically — the phase arrays became `[…; 7]`.

## Tests

- **Bar string:** fraction → glyph count + percent, including `expected = empty`
  (renders 100%) and overshoot (`seen > expected`, clamped to 100%).
- **Omitted row:** `expected = None` produces no row.
- **Minimum visible:** a phase that completes instantly stays displayed until
  `first_seen + min_visible`.
- **Dynamic denominator:** repo renders `Waiting` while growing, switches to a
  bar once stabilized, and never reports a regressing percentage.
- **Failure is terminal:** a failed phase (fetch error, metadata error, or
  timeout) pops a reason toast, enters `Failed`, and does not hold the panel.
- **Completion:** the panel dismisses only when every row is Complete (past its
  floor) or Failed.
- **Gate covers every row:** adding a phase (repo in 2, languages/tests in 3)
  holds the panel until that row is terminal — no row renders ungated.
- **git failure releases repo:** a timed-out/`Failed` git unblocks repo so the
  panel still finishes.
- **Late result after dismissal:** a result arriving for an already-`Failed`,
  already-dismissed phase no-ops (no panic, no stale-toast write).
- **New rows:** languages and test counts populate `expected` at scan start,
  mark `seen` from their batch handlers, and gate dismissal.
