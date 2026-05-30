# Startup progress panel

## Goal

Replace the current per-phase startup toasts — each rendering a truncated
"remaining: foo, bar, baz" item list — with a single consolidated panel that
shows **one progress bar per type of background work**, each advancing to 100%.

The panel answers one question: *what background work is still running, and how
far along is each kind relative to the others.* It is not a triage surface — it
reflects work completion and relative timing, not which data matters most.

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
- Do not require framework changes for the first version (see
  [App vs. framework](#app-vs-framework)).
- Do not show a bar for work that has no knowable denominator.

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
| lint | `KeyedPhase<AbsolutePath>` + `CountedPhase` | project root | fixed |

The toast body is already an **app-computed string**: `remaining_toast_body`
(`toast_bodies.rs`) filters `expected − seen` and renders it via
`format_toast_items`. The framework supplies the toast container
(`ToastTaskId`, `ToastSettings`, `toast_body_width`); the app owns the body
text.

Two kinds of background work are **not** tracked and stream in silently:

- **Language stats** (tokei) → secondary Languages pane.
- **Test counts** (this feature) → detail-pane Tests sub-section.

Both are local, bounded reads feeding non-primary surfaces, which is why they
were never toast-tracked. Under this proposal they each become a bar row — the
reframe above makes "is it countable background work" the inclusion test, and
both qualify.

## Design decision: one panel, not N toasts

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

Rows fill in place; the whole panel dismisses once every bar reaches 100% (and
clears the min-visible floor below). Rationale:

- One container removes the pop-in / pop-out churn of independent toasts.
- Relative timing is visible at a glance — the original ask.
- Adding languages and test counts is one row each, not a new toast lifecycle.

## App vs. framework

**Version 1 is entirely app-side.** The bar is a unicode string
(`▓`/`░` + percent) computed from `seen.len() / expected.len()`, assembled into
the existing Startup toast's multi-line body — exactly where
`remaining_toast_body` already writes today. No `tui_pane` change.

**Framework support is only needed for richer rendering** — real `Gauge`
widgets, animated fill, per-row color beyond a styled text body. Defer until the
text version proves too plain; if pursued, it is a new structured-progress toast
variant in `tui_pane` that renders rows instead of a body string.

## Behaviors

### Dynamic denominators

A phase whose `expected` set grows mid-flight (repo discovers GitHub repos as
git remotes resolve; submodules appear during scan) would make a percentage bar
jump backward. For any phase whose `expected` is still growing, render
**`waiting for count`** (or a marquee/indeterminate state) in place of a
percentage, and switch to a real bar only once the denominator stabilizes.

### Minimum visible duration

A phase that completes in under ~100 ms would flash 0 → 100 % as noise. Each row
carries a **minimum-visible duration**: hold its displayed state until
`max(actual_complete, first_seen + min_duration)`. The panel dismisses as a unit
only after every row has both reached 100 % and cleared its floor.

### Error / stall visibility

A pure bar that stalls at 80 % does not say *why* — the current item list at
least named the stuck item. When a phase cannot finish (rate-limited GitHub,
`cargo metadata` error), **pop a separate toast naming the reason**. The bar row
may also annotate the stalled state (color/marker), but the actionable detail
lives in the error toast so the cause is never silent.

## Rows in scope

disk, git, repo, metadata, lint (already tracked) plus **languages** and
**test counts** (need new `expected`/`seen` wiring — both key on project root,
denominator fixed at the project-root set, same as disk).

## Implementation phases

1. **Bar rendering (app-side).** Add a `progress_bar_row(label, seen, expected,
   width)` body builder and a panel assembler that stacks every tracked phase's
   row into the Startup toast body. Replace `remaining_toast_body` usage at the
   panel level; keep the per-item list available for the error path.
2. **Min-visible duration.** Stamp `first_seen` per phase; gate panel completion
   on the per-row floor. Pass the duration in (configurable, not hard-coded at
   the call site).
3. **Dynamic-denominator state.** Add an "expected still growing" flag per
   `KeyedPhase`; render `waiting for count` until it settles.
4. **Error toasts.** On a phase's terminal failure, pop a reason toast; mark the
   row stalled.
5. **Wire languages + test counts as rows.** Add `KeyedPhase<AbsolutePath>`
   trackers for both, populate `expected` from the project-root set at scan
   start, mark `seen` in their batch handlers
   (`handle_language_stats_batch`, `handle_test_counts_batch`).

## Tests

- Bar string: fraction → glyph count + percent, including 0/0 (empty) and full.
- Min-visible: a phase that completes instantly stays displayed until the floor.
- Dynamic denominator: a phase whose `expected` grows renders the placeholder,
  then a bar once stable, and never reports a regressing percentage.
- Error path: a failed phase pops a reason toast and the panel does not silently
  hang at <100%.
- Completion: panel dismisses only when every row is at 100% and past its floor.

## Open decisions

- **Glyph and width** of the bar (block elements vs. `=`/`-`; fixed column
  width vs. proportional to panel width).
- **Min-visible duration** value, and whether it is per-phase or global.
- **Ordering** of rows (fixed by phase, or by start order, or slowest-first).
- **Indeterminate rendering** for dynamic phases: static `waiting for count`
  text vs. an animated marquee.
- Whether **languages + test counts** appear in the panel at all, or stay
  untracked — folding them in is the consistent choice under the reframe, but
  it does make the panel linger on cosmetic work.
- Whether to ever pursue the **framework `Gauge`** variant or stay text-only.

## Relationship to the test-counts feature

The test-counts data and its detail-pane Tests sub-section shipped independently
(`feat: show unit/integration test counts in detail pane`) and do not depend on
this panel. Wiring test counts as a bar row here only needs the `expected`/
`seen` tracker that was deliberately skipped — additive, no rework.
