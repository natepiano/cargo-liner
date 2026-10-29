# remove-running-section-handler-footer-colors-next

Approved next items from `remove-running-section-handler-footer-colors.md`. Each item becomes a phase when the showrunner places it.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Items

### Worker row names drop the role word and slot text  · status: next (showrunner 2026-10-28, after Phase 36)

From C82 (worker rows show their job), confirmed live and landed. A small polish phase after Phase 36.
1. Codex row names repeat the role word ("impl · Phase 1 – Footer colors…", "fix · …", "test · …"). The role column already shows impl/fix/test, so the name drops the "<role> · " prefix.
2. Slot text shows as a job ("impl · names-unit-1 — slot impl", "impl · Style ticket types-unit-2 — slot `i…"). The name drops a trailing "— slot <x>". When nothing is left, the row shows the ticket ("ticket types-unit-2").
Tests: one red-first test for each. The checkpoint adds a Change-Review line.

### Each app shows its own CPU % before Uptime  · status: next (showrunner 2026-10-28, after Phase 36 and the CPU cost fixes; lower priority than other work)

**Source:** the user, via the showrunner, 2026-10-28: "i want all of port, tile, handler to show their own cpu % utilization on the bottom left - before the Uptime number".

**Placement (user via showrunner, 2026-10-28: "shared of course so it goes in tui_pane"):** the sampler, `CpuReadout` and the status-bar cell live in tui_pane as one cell that each app places. cargo-handler and cargo-tile put it before Uptime. cargo-port draws its own bar (`crates/cargo-port/src/tui/render.rs:700`, no Uptime); port-unit places the same cell at its bottom left. Export only what those three uses need.

**Design (showrunner):** build it once in the tui_pane status bar where Uptime is drawn (`crates/tui_pane/src/bar/status_line.rs`, the uptime segment); wire it in cargo-handler and cargo-tile; port-unit wires cargo-port. Show the app's own process CPU as a percent of one core, sampled at most once a second from getrusage or /proc/self/stat deltas, at almost no cost. A typed value `CpuReadout::{Sampling, Percent(..)}`, never a placeholder 0. Red-first tests under 1 s, and a Change-Review line.
