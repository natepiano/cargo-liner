# Toast scheduling, wrapping, and card rendering

The toast manager supplies visual deadlines and the shared renderer supplies measured cards.
Applications merge those deadlines with their own wake schedule.
They pass placement rectangles instead of discovering pane borders from painted cells.

Paths below are relative to `crates/tui_pane/src/`.

## Scheduling

`toasts/manager.rs` defines `ToastVisualDeadline::{NoVisualChangeScheduled, At(Instant)}`.
`Toasts::next_visual_change_deadline(now)` chooses the earliest visual change among active toasts.
Changes include entrance and exit rows, expiry, countdown text, spinner frames, elapsed text, fades, and tracked-item removal.
An At deadline is strictly in the future and at least `FRAME_POLL_MILLIS`, which is 8 ms, away.
NoVisualChangeScheduled requires no toast wake.

`toasts/toast.rs` owns `ToastPhase` and `ToastEntranceSchedule`.
`ToastPhase` distinguishes Entering, Static, and Exiting.
`ToastEntranceSchedule` distinguishes Absent and Scheduled.
Named states prevent half-present animation schedules.

cargo-port takes the deadline as a minimum against its application tick.
cargo-tile consumes the same deadline instead of reproducing toast timing arithmetic.
A toast's presence alone does not mean it continuously animates.

## Card placement and pages

`toasts/render/drawing.rs` allocates cards in the caller's allowed area.
The corner pane determines the width budget when an application constrains placement to that pane.
The renderer does not scan the screen for border glyphs.
It preserves pane borders in gap rows and clears only the card and permitted gaps.
The right-border check covers occupied card rows rather than every row of the screen.

A card keeps a blank row beside a pane edge when space permits it.
`MIN_PANE_ROWS_WITH_EDGE_GAP` is six rows.
Page capacity uses allocated height, rather than the changing entrance height.
`ToastExitState` keeps an exiting toast's allocated height steady.

Plain and tracked bodies report overflow through `ViewportOverflow`.
The bottom border carries the page mark without covering the countdown or action hint.
Tracked overflow retains its own final `(+N more)` row and exact count.
The startup body is built once for each frame after layout.

## Titles and the first sentence

`toasts/render/card.rs::fitted_toast_title` fits the title beside the close control.
A visible title must retain its first readable word, with an optional continuation mark.
The check uses the fitted text, not only the original word's measured width.
Leading spaces therefore cannot produce a title containing only `…`.
An original ellipsis in the title remains valid text.
The close control keeps its required blank cell.

`first_sentence_height` in `toasts/render/drawing.rs` measures the wrapped first sentence for ordinary bodies.
It matches the original non-space characters against the drawn characters in order.
Inserted continuation marks do not count as original sentence characters.
The allocation can therefore include every row through the first sentence's final punctuation.
Tracked bodies keep their own minimum-height rule.

## Display-cell wrapping

`wrap.rs` owns `BoundaryWrap`, `WordCuts`, and `split_at_cells`.
`BoundaryWrap::new(width, word_cuts)` selects the width and word-break policy together.
`cell_count` measures terminal display cells.
`split_at_cells` walks complete graphemes and tracks their source byte offsets.
Skipped control characters must not shift those offsets into the middle of a UTF-8 sequence.

The public `wrapped(spans, width) -> Text` uses Unmarked word cuts.
Toast spans use `wrapped_toast`; styled runs use `wrapped_toast_runs`.
Both toast wrappers use Marked cuts and preserve whitespace.
The public wrapper collapses inter-word whitespace.
A zero width disables wrapping and returns the original spans.

A word that fits a fresh row moves there whole.
A longer word can break at a supported punctuation boundary or a complete grapheme boundary.
A marked hard break reserves a cell for `…` when possible.
The renderer and its measurement use `body::card_body_width`.

If indentation leaves no room, a grapheme that fits the full row remains unconsumed until the next row.
At a five-cell body width, `     metadata` produces an empty row, `meta…`, then `data`.
A grapheme wider than the whole row must be consumed to guarantee progress.
Unmarked keeps that grapheme; Marked can represent it with `…`.
Thus public wrapping of `東a` at width one retains `東` and then `a` on separate rows.

## Tracked work

`TrackedItemActivity::{Progressing, Stalled}` controls a running item's spinner color.
`RunningTracker::mark_running` retains its original start instant when activity changes.
`Toasts::refresh_tracked_item_activity` updates existing items without restarting their lifetime.
Completed fading items have no running spinner.
The framework does not know why application work stalled.

## Invariants and calibration

1. Merge framework deadlines instead of copying schedule arithmetic.
2. Measure the rows that the card actually draws.
3. Keep full graphemes and source byte boundaries.
4. Preserve a fitting grapheme when moving to a fresh row.
5. Consume oversized graphemes so wrapping always terminates.
6. Keep close controls, page marks, and action hints separate.

`toasts/render/sentence_tests.rs` checks measured and drawn rows in both color modes.
Its cases include CJK text, combining marks, skipped controls, leading indentation, and narrow titles.
`wrap.rs` also checks styled boundaries and public wrapping.
cargo-tile calls the public wrapper, so a shared wrapping change also affects its command and ancestry text.
