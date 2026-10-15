# cargo-handler startup and census — as-built

A reference for the next engineer who changes how cargo-handler starts, scans this machine, or keys its cells.

## What it is

cargo-handler draws its first frame before any census answer exists: the summary pane shows each machine as `<name> · scanning`, and rows fill in as answers arrive. The census scheduler sends every remote its ssh probe before it scans this machine. On the first scan the local scanner publishes twice: the classified rows (agent, name, status, pid, directory) as soon as the process table and session records are read, then the completed rows once the slow detail sources (transcripts, launchers, children, desktops, branches) are read. Every later scan publishes only completed rows. Each agent cell is keyed by its process's own start time, so the cell keeps its identity when a later answer moves the session's age earlier. An optional log, `CARGO_HANDLER_FRAME_LOG`, records startup milestones and per-stage scan times.

Where the code lives (paths relative to `crates/cargo-handler/src/`):
- startup sequence and census drain: `terminal.rs` (`run`, `Ticker`)
- scheduler: `census/schedule.rs`
- local scan and its stages: `census/scan.rs`
- census state and rows: `census/mod.rs`; classification: `census/classify.rs`; wire format: `census/probe.rs`
- cell identity: `tiles.rs` (`AgentCell`), `agent_cell.rs` (`cell_order`)
- timing log: `startup_log.rs`; its variable name: `constants.rs` (`FRAME_LOG_VARIABLE`)
- attract-screen identification hook: `app.rs` (`before_draw`)

## How it works

### First frame

- `terminal::run` calls `startup_log::init()`, loads the config, installs the theme, reads the system appearance (`spawn_appearance_watcher`, blocking), builds the `App`, then hands `run_terminal` a closure that builds the `Ticker` around `census::schedule::spawn(app.remote_machines.clone())`.
- `spawn` returns its `Receiver<CensusUpdate>` at once; all scanning happens on the scheduler thread. The event loop's first pass drains whatever has arrived (normally nothing) and draws.
- `Census::new(local_name)` starts with `local: MachineState::Scanning`, and `Census::machines` reports any configured remote with no answer as `Scanning`, so the first frame draws the summary frame with every machine heading reading `scanning`.
- `Ticker::drain_census` folds each `CensusUpdate` into `Census::apply`, which returns `false` (no repaint) when the new state equals the old. An answer from a host removed from `machines.remote` since its probe left is dropped.

### Scheduler — `census/schedule.rs`

- `spawn_staged_with(remotes, runner: Arc<dyn RemoteRunner>, scan_local: impl FnMut(&Sender<CensusUpdate>) -> ScanSchedule, cadence: Cadence)` runs one loop. In each pass, remotes that are due are dispatched first (`dispatch` spawns one thread per host and skips a host whose last probe is still in `InFlight`), then the local scan runs on the scheduler thread if due.
- Cadence is `LOCAL_SCAN_INTERVAL` (2 s) and `REMOTE_PROBE_INTERVAL` (5 s). `remotes.snapshot()` is read every remote round, so a host added in settings is probed from the next round.
- The local source receives the `Sender` itself and may send more than one `CensusUpdate::Local` per scan. It returns `ScanSchedule::Finished` once a send fails (the receiver is gone), which ends the thread.
- `spawn_with_test_source` runs one real scheduler round with a test-held source; `spawn_with` (test-only) adapts a `FnMut() -> Vec<AgentRow>` source for the cadence tests.

### Staged local scan — `census/scan.rs`

- `LocalScanner::scan_staged(&mut self, publish: impl FnMut(Vec<AgentRow>))` reads `sessions`, `process_table` and `codex_sqlite`, then calls `scan_stages`.
- `scan_stages` runs `classify::agent_rows(processes, sessions, codex_threads, home)`, publishes those rows if `self.publication` is `LocalPublication::FirstAnswerPending`, runs `attach_details` on the same rows, publishes the completed rows, and sets `CompletedAnswerPublished`.
- `attach_details` stages, in order: `codex_service_tiers`, `transcript_discovery`, `transcript_launchers`, `transcript_session_starts`, `transcript_subagents`, `transcript_shell_calls`, `codex_app_server_threads`, `attach_children`, `attach_desktops`, `attach_branches`. Each is wrapped in `startup_log::startup_stage(name, body)`.
- `LocalScanner::scan()` keeps only the last publication; `probe::print` uses it, so a remote probe always prints the completed answer.

### Cell identity — `census/mod.rs`, `tiles.rs`, `census/probe.rs`

- `AgentRow` carries `started` (the session's start: for Claude Code, the transcript's first line once `attach_session_starts` reads it, else the process start) and `process_started` (the process's own start). `classify::agent_rows` sets both from `ProcessEntry::started`; only `started` moves later in the scan.
- `AgentCell { machine: String, pid: u32, process_started: u64 }` is the `TileGrid` key; `cell_order` builds it from `row.process_started`. A reused pid gets a new start time and so a new cell.
- `AgentRow` deserializes through `ProbeAgentRow`, whose `process_started: Option<u64>` falls back to `started` when a probe from an older build omits it. `PROBE_SCHEMA` stays `3`; the field is additive.

### Startup timing log — `startup_log.rs`

- `init()` reads the env var named by `FRAME_LOG_VARIABLE` (`CARGO_HANDLER_FRAME_LOG`) into a `OnceLock<StartupTiming>`; unset means `Disabled`, and `startup_event` and `startup_stage` return without timing anything.
- Each line is `<ms since start>ms <event>`, appended under a mutex. Milestones: `process_start`, `config_loaded`, `appearance_read_returned`, `alternate_screen_entered`, `first_frame_flushed`, `identify_start`/`identify_end`, `first_local_answer rows=N`, `first_agent_rows rows=N`, `local_applied rows=N`, `remote_answer <host>`, `remote_applied <host>`, and `scan_stage_start <stage>` / `scan_stage_end <stage> elapsed=Nms`.
- `StartupProbe` is the `FrameProbe` for both the app and its `Attract` (`tui_pane::Attract<StartupProbe>`); its `timed` marks the first `FramePhase::Draw` and its `note` forwards tui_pane's notes.
- `startup_local_answer` logs when the scanner publishes; `local_applied` logs when the UI drains it, so the gap between them is event-loop latency.

### Spanning summary — `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/pane/frame.rs`

With `tiles.widen_summary` on, the summary can span several column bands. `draw_placements` collects each summary piece's `line_frame.rect()` and, after every band and piece frame is added, calls `GridLines::open(rect)` on each before `render`: cells inside the rect lose all sides, edge cells lose their inward side (`Sides::without`), corners keep their joins. No band divider crosses summary text, and the bottom border keeps `┬` where a column line continues below.

## Invariants

- The first frame never waits on a census answer; nothing on the UI thread scans.
- Remotes due in a round are dispatched before that round's local scan.
- Only the first local scan publishes classified rows. A later scan publishing them would show agents without branches, launchers and children on every refresh until the details arrived.
- Cells are keyed by `process_started`, never by `started`.
- A probe prints only the completed answer.
- `GridLines::open` runs after every `add`; a band added later draws its line back.

## Calibration / gotchas

- The slow first scan is two cold first reads in the details stage: `transcript_launchers` (4.4 s cold, 5 ms warm) and `codex_app_server_threads` (2.4 s cold, 3 ms warm). Under load 162 they made a 36 s first answer before staging.
- Measured under load with staging: first frame ~11 ms, first agent rows ~118–228 ms, details seconds later.
- A progress timer (`Walker::timer`, `sleep_timer` in `census/tree.rs`) is read from the one `sleep` leaf under a shell wrapper, never from a script name or folder. That is why the move of `~/.claude/scripts/delegate` and `/tmp/claude/delegate` to `unit` needed no matcher change; `a_timer_under_the_unit_folders_reads_as_under_the_delegate_folders` pins both names.
- To record: `CARGO_HANDLER_FRAME_LOG=/path/to/log cargo-handler`. The file is appended to, never truncated; delete it between runs.
- The local scan runs on the scheduler thread, so while a slow scan runs no remote round starts; remotes already dispatched keep running on their own threads.
- `before_draw` calls `Attract::identify` only while the attract screen shows; `identify_start`/`identify_end` appear only then.

## Why

- **Classified rows first.** The classification inputs (sessions, process table, Codex sqlite) are cheap; the details include the two cold reads above. Publishing the classified rows lets the summary list agents in a fraction of a second instead of tens of seconds.
- **First scan only.** After startup the detail caches are warm and a scan completes quickly; a classified publication then would only replace complete rows with bare ones.
- **Remotes first.** A remote probe costs the scheduler a thread spawn; scanning first would hold every remote behind the local scan, the slowest one included.
- **`process_started` as the key.** `attach_session_starts` moves `started` back to the transcript's first line during the details stage, so a key on `started` changed between the classified and completed answers and a resumed session lost its cell and focus.
- **The spanning rule lives in tui_pane.** cargo-handler draws the summary at its full spanned width; the divider came from `draw_placements` adding each band's full-height frame over the summary's contents, so the fix sits where the lines are drawn.
- **Ruled out:** making the appearance read non-blocking (it costs 30 ms) and moving window identification off the UI thread (each pass ≤119 ms). Both were measured and left as they are.
