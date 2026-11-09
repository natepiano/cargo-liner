# remove-running-section-handler-footer-colors-next

Approved next items for [cargo-handler](../cargo-handler/as-built/hierarchy.md).
The [as-built docs](../cargo-handler/as-built/tile-layout.md) describe the current implementation.
Each item becomes a phase when the showrunner places it.


## Items

### Worker row names drop the role word and slot text  · status: next (showrunner 2026-10-28, after Phase 36)

From C82 (worker rows show their job), confirmed live and landed. A small polish phase after Phase 36.
1. Codex row names repeat the role word ("impl · Phase 1 – Footer colors…", "fix · …", "test · …"). The role column already shows impl/fix/test, so the name drops the "<role> · " prefix.
2. Slot text shows as a job ("impl · names-unit-1 — slot impl", "impl · Style ticket types-unit-2 — slot `i…"). The name drops a trailing "— slot <x>". When nothing is left, the row shows the ticket ("ticket types-unit-2").
3. Since Phase 67, a staff name replaces the fallback name and takes precedence over job text (`census/classify.rs` ≈216, `agent_cell.rs` ≈1361). Change only task-derived fallback names; keep staff names exactly.
Tests: one red-first test for each removal, and one that a staff name stays unchanged. The checkpoint adds a Change-Review line.

### Each app shows its own CPU % before Uptime  · status: next (showrunner 2026-10-28, after Phase 36 and the CPU cost fixes; lower priority than other work)

**Source:** the user, via the showrunner, 2026-10-28: "i want all of port, tile, handler to show their own cpu % utilization on the bottom left - before the Uptime number".

**Placement (user via showrunner, 2026-10-28: "shared of course so it goes in tui_pane"):** the sampler, `CpuReadout` and the status-bar cell live in tui_pane as one cell that each app places. cargo-handler and cargo-tile put it before Uptime. cargo-port draws its bar through the shared `StatusLine` renderer (`crates/cargo-port/src/tui/render/mod.rs` ≈843, `render_status_bar`); port-unit places the same cell at its bottom left. Export only what those three uses need.

**Design (showrunner):** build it once in the tui_pane status bar where Uptime is drawn (`crates/tui_pane/src/bar/status_line.rs`, the uptime segment); wire it in cargo-handler and cargo-tile; port-unit wires cargo-port. Show the app's own process CPU as a percent of one core, sampled at most once a second from getrusage or /proc/self/stat deltas, at almost no cost. The sampler supplies its next sample deadline to each host's display-time state. A due sample refreshes the status bar without requesting a tile-body frame. Tests prove that sampling and the displayed percent advance while the grid is settled. A typed value `CpuReadout::{Sampling, Percent(..)}`, never a placeholder 0. Red-first tests under 1 s, and a Change-Review line.

### Hierarchy timing values move to cargo-handler's config  · status: next (showrunner 2026-10-31, after current work)

**Source:** the showrunner, 2026-10-31, from Phase 57c: these are configuration values in code.

Move the accounts-command timeout and reuse window into typed `[census]` settings beside `accounts_command`. Keep defaults of 10 seconds and 300 seconds. Preserve immediate staff-file revision checks; add no hierarchy polling setting. Preserve the remote probe deadline when removing the shared timeout constant. Add red-first tests under one second for configured values and defaults. The checkpoint adds a Change-Review line only if users can see the change.
