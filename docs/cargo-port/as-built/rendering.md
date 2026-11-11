# Rendering and cache invalidation

cargo-port inspects a frame when input, results, animation, or status changes can affect the screen.
It renders that frame once into the real terminal.
It retains one completed screen for change classification.

Paths below are relative to `crates/cargo-port/src/`.

## Event loop

`tui/terminal/event_loop.rs` owns `event_loop`, `draw_frame`, and the frame counter.
`wait_for_event` selects input and background channels, including CI, clean, owned-run, and CPU messages.
A disconnected input channel ends the loop instead of causing repeated ready events.

1. Drain ready input and background messages.
2. Update status, CPU data, and animation state.
3. Refresh affected row, disk, width, and detail caches.
4. Render an inspected frame through `Terminal::draw`.
5. Start queued background actions after drawing.
6. Block until a channel or visual deadline requires another inspection.

`FrameState` tracks the application inputs that can change visible output.
`FrameOutput::{FirstFrame, Drawn(RenderedScreen)}` owns the retained screen.
`FrameDraw::{Drawn(Duration), Unchanged}` classifies the completed buffer.
Only `Drawn` increments the frame counter.

`draw_frame_with_renderer` calls the renderer once.
It compares the completed buffer with the retained buffer.
When content changes, `clone_from` reuses the retained cell vector.
The first frame initializes that storage.
There is no candidate render into `Terminal<TestBackend>` in the production path.
CPU-only changes use the same full render path.

The loop does not query `get_cursor_position`.
That terminal query can block or fail when the terminal supplies no response.
cargo-port does not set a cursor position, so its buffer describes the screen.

## Caches

`ProjectList` maintains a project-path index in `tui/project_list/list.rs`.
Root changes rebuild the index.
`ProjectPathLookup::{Found, UnknownPath, StaleRoute}` distinguishes lookup outcomes.
Render-time lookups do not repeatedly search all roots.

`tui/app/navigation/cache.rs` owns application cache refresh decisions.
`tui/panes/data.rs` defines `DetailCacheKey` and `DetailCacheState::{Cleared, Current(key)}`.
Selection converts to that named state at the application boundary.
`detail_is_current` and `set_detail_stamp` consume the state directly.
A generation change rebuilds selected Package, Git, and Targets data together.

Visible-row, disk, and column caches have their own validity keys.
`tui/panes/project_list/disk.rs` computes disk totals without cloning a child vector for each node.

## Timing and calibration

The idle heartbeat is one second.
Animation uses the 80 ms application tick when needed.
Toast and framework status deadlines can require an earlier inspection.
An idle heartbeat alone does not require a changed screen.
A CPU sample can change the screen and cause a full render.

`TUI_PANE_LOG=tui_pane::perf=trace` enables the `drawn_frames` trace.
The count reports changed frames, not event-loop iterations.
Controlled release measurements reached about 0.2% of one CPU core while idle.
This figure is a calibration result, not a runtime limit.

## Invariants and rationale

1. Render each inspected frame once into the real terminal.
2. Count only completed screens whose cells change.
3. Refresh derived data only when its validity key changes.
4. Preserve file-reload checks while idle.
5. Keep terminal queries out of change classification.

The retained screen detects visually unchanged results without rendering a second candidate frame.
Typed cache states keep an empty selection distinct from valid selected data.
See [background work](background-work.md) for cache ownership outside the render loop.
