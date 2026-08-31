# Toast visual-change deadline — as-built

A reference for the next engineer modifying `tui_pane`'s toast scheduling. This is a
small framework surface in `tui_pane/src/toasts/manager.rs`: it answers, over all active
toasts, the exact instant any of them can next look different, so a consumer's render loop
repaints on demand rather than polling. `cargo-tile` and `cargo-port` both consume it;
neither holds toast-scheduling arithmetic of its own.

---

## How it works

`Toasts::next_visual_change_deadline(now) -> ToastVisualDeadline {
NoVisualChangeScheduled, At(Instant) }` (exported at the crate root) answers, over all
active toasts, the earliest instant any of them can next look different (entrance/exit
line-height boundaries, expiry, per-second countdown, spinner frames, elapsed readout,
linger fades, tracked-item removal). The aggregate is floored at `FRAME_POLL_MILLIS` (8ms)
and is always strictly in the future. A toast's phase is
`ToastPhase::{Entering{starts_at,ends_at}, Static, Exiting{started_at}}` and its entrance
`ToastEntranceSchedule::{Absent, Scheduled{..}}` — neither is a bare `Option<Instant>`.
cargo-tile holds no scheduling arithmetic of its own; cargo-port's `animation_timeout`
takes the deadline as a minimum against its 80ms `ANIMATION_TICK` and no longer reports
`is_animating` merely because a toast exists.

---

## Invariants

1. **The deadline is always usable.** `next_visual_change_deadline` returns an instant that
   is always strictly in the future and floored at `FRAME_POLL_MILLIS` (8ms), so a caller
   can schedule its next repaint against it without ever computing a zero or negative wait.
2. **`ToastPhase` and `ToastEntranceSchedule` hold their own schedule.** Each is a
   non-`Option` enum — `ToastPhase::{Entering{starts_at,ends_at}, Static, Exiting{started_at}}`
   and `ToastEntranceSchedule::{Absent, Scheduled{..}}` — so the schedule cannot be
   half-present the way a bare `Option<Instant>` pair could.
3. **Consumers own no scheduling arithmetic; they merge the deadline as a minimum.**
   cargo-tile holds no toast-scheduling math, and cargo-port's `animation_timeout` takes
   the deadline as a minimum against its own 80ms `ANIMATION_TICK` rather than recomputing
   when a toast next changes.

---

## Calibration / gotchas

- `FRAME_POLL_MILLIS = 8` — the toast-deadline floor. The aggregate deadline is never
  reported closer than this into the future.

---

## Why

- **Framework-owned toast deadline.** A toast is the thing that knows when it next changes;
  computing that in each consumer duplicated fragile arithmetic and drifted. The framework
  advertises the exact next-change instant (floored at a render-usable 8ms) so repaints are
  demand-driven, and consumers merge it with their own tick as a minimum.
