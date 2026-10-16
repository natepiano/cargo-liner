# cargo-handler startup and census — as-built

A reference for the next engineer who changes how cargo-handler starts, scans this machine, records and labels each agent's account, or keys its cells.

## What it is

cargo-handler draws its first frame before any census answer exists: the summary pane shows each machine as `<name> · scanning`, and rows fill in as answers arrive. The census scheduler sends every remote its ssh probe before it scans this machine. On the first scan the local scanner publishes twice: the classified rows (agent, name, status, pid, directory) as soon as the process table and session records are read, then the completed rows once the slow detail sources (transcripts, launchers, children, desktops, branches) are read. Every later scan publishes only completed rows. Each agent row records its account: the configuration home its process's environment names (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, else the program's default), with the label the scanning machine's `[accounts]` table gives it. Each agent cell is keyed by its process's own start time, so the cell keeps its identity when a later answer moves the session's age earlier. An optional log, `CARGO_HANDLER_FRAME_LOG`, records startup milestones and per-stage scan times.

Where the code lives (paths relative to `crates/cargo-handler/src/`):
- startup sequence and census drain: `terminal.rs` (`run`, `Ticker`)
- scheduler: `census/schedule.rs`
- local scan and its stages: `census/scan.rs`
- census state and rows: `census/mod.rs`; classification: `census/classify.rs`; wire format: `census/probe.rs`
- accounts (configuration homes, labels, `Account` on rows, `AccountColumn`): `census/account.rs`, tests in `census/account/tests.rs`; the `[accounts]` table: `config.rs` (`AccountsConfig`); variable names: `constants.rs` (`CLAUDE_CONFIG_DIR_VARIABLE`, `CODEX_HOME_VARIABLE`)
- cell identity: `tiles.rs` (`AgentCell`), `agent_cell.rs` (`cell_order_placed`)
- timing log: `startup_log.rs`; its variable name: `constants.rs` (`FRAME_LOG_VARIABLE`)
- attract-screen identification hook: `app.rs` (`before_draw`)

## How it works

### First frame

- `terminal::run` calls `startup_log::init()`, loads the config, installs the theme, reads the system appearance (`spawn_appearance_watcher`, blocking), builds the `App`, then hands `run_terminal` a closure that builds the `Ticker` around `census::schedule::spawn(app.remote_machines.clone(), app.hierarchy_command.clone(), AccountLabels::new(&app.loaded_config.config.accounts, dirs::home_dir().as_deref()))`. `spawn(remotes, hierarchy, account_labels)` moves the labels into `LocalScanner::new(account_labels)`.
- `spawn` returns its `Receiver<CensusUpdate>` at once; all scanning happens on the scheduler thread. The event loop's first pass drains whatever has arrived (normally nothing) and draws.
- `Census::new(local_name)` starts with `local: MachineState::Scanning`, and `Census::machines` reports any configured remote with no answer as `Scanning`, so the first frame draws the summary frame with every machine heading reading `scanning`.
- `Ticker::drain_census` folds each `CensusUpdate` into `Census::apply`, which returns `false` (no repaint) when the new state equals the old. An answer from a host removed from `machines.remote` since its probe left is dropped.

### Scheduler — `census/schedule.rs`

- `spawn_staged_with(remotes, hierarchy: HierarchyCommand, hierarchy_runner: Arc<dyn HierarchyRunner>, runner: Arc<dyn RemoteRunner>, scan_local: impl FnMut(&mut LocalAnswers) -> ScanSchedule, cadence: Cadence)` runs one loop. In each pass, remotes that are due are dispatched first (`dispatch` spawns one thread per host and skips a host whose last probe is still in `InFlight`), then the local scan runs on the scheduler thread if due.
- Cadence is `LOCAL_SCAN_INTERVAL` (2 s) and `REMOTE_PROBE_INTERVAL` (5 s). `remotes.snapshot()` is read every remote round, so a host added in settings is probed from the next round.
- The local source receives `LocalAnswers` (the `Sender`, the shared hierarchy setting, its runner and `HierarchyCache`) and publishes through `LocalAnswers::publish(stage, rows)`. Classified rows go out with no hierarchy edges; completed rows go out first with the cached edges, then again after hierarchy discovery (see `hierarchy.md`), so discovery never delays row detail. It returns `ScanSchedule::Finished` once a send fails (the receiver is gone), which ends the thread.
- `spawn_with_test_source` runs one real scheduler round with a test-held source; `spawn_with` (test-only) adapts a `FnMut() -> Vec<AgentRow>` source for the cadence tests.

### Staged local scan — `census/scan.rs`

- `LocalScanner::scan_staged(&mut self, publish: impl FnMut(ScanStage, Vec<AgentRow>))` reads `sessions`, `process_table` and `codex_sqlite`, then calls `scan_stages_with`.
- `scan_stages_with` runs `classify::agent_rows(processes, sessions, codex_threads, home, &self.account_labels)`, publishes those rows as `ScanStage::Classified` if `self.publication` is `LocalPublication::FirstAnswerPending`, runs `attach_details` on the same rows, publishes them as `ScanStage::Completed`, and sets `CompletedAnswerPublished`.
- `attach_details` stages, in order: `codex_service_tiers`, `transcript_discovery`, `transcript_launchers`, `transcript_session_starts`, `transcript_subagents`, `transcript_shell_calls`, `codex_app_server_threads`, `attach_children`, `attach_desktops`, `attach_branches`. Each is wrapped in `startup_log::startup_stage(name, body)`.
- `LocalScanner::scan()` keeps only the last publication; `probe::print` uses it (`LocalScanner::new(AccountLabels::new(&loaded.config.accounts, dirs::home_dir().as_deref())).scan()`), so a remote probe always prints the completed answer.

### Accounts — `census/account.rs`

- Config: `[accounts]` is `AccountsConfig(BTreeMap<String, String>)` (`serde(transparent)`) on `Config::accounts`, after `census`. Each key is an agent configuration home written with `~` (`"~/.claude1" = "claude1"`), each value the label its rows show; the default config writes an empty table. It is read at startup, so an edit applies on restart.
- Labels: `AccountLabels::new(&AccountsConfig, machine_home: Option<&Path>)` normalizes each key to the `~` form once (`classify::directory_label`). `resolve(home, machine_home) -> AccountHome` writes the home in that form and gives it its configured label, else its last path component without one leading dot (`~/.codex1` → `codex1`). `AccountHome { home, label }` has private fields, `resolve` is its only constructor, and `label()` is its only reader.
- Environment: `process_table`'s third pass (`proc_agent_environments`) reads `environ` for processes named `codex` or `claude` and for every session-record pid (version-named Claude binaries). `config_home_of(&ProcessEntry, &[OsString])` sets `ProcessEntry::config_home: ConfigHome { Unread, Default(Program), Named(PathBuf) }`. `Program { Claude, Codex }` owns its variable (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`) and default directory (`CLAUDE_DIRNAME` / `CODEX_DIRNAME`). In `config_home(&[OsString], Program)` an absent or empty variable is `Default`, only an exact `NAME=` prefix matches, and a non-UTF-8 value stays a path. A non-agent process, or one whose environment comes back empty (sysinfo could not read it), stays `Unread`. `CLAUDE_PID` is still read for Codex processes only.
- Rows: `account_of(process, machine_home, labels) -> Account` maps `Unread` to `Unrecorded`, `Default` to the machine home joined with the default directory (`Unrecorded` when the machine home is unknown) and `Named` to its path, each resolved through `AccountLabels::resolve`. `Account { Unrecorded (default), Home(AccountHome) }` sits on `AgentRow::account` and `ChildRow::account`. `classify::agent_rows(.., &AccountLabels)` passes the labels to `claude_row`, `desktop_app_row` and `codex_row`, so a row gets its account where it is built.
- Threads: `app_server_threads` sets `ThreadEntry::account` from the app server's process, and `tree.rs` carries it through `Node` and `flatten` into the thread's `ChildRow`. Every other child (shell, subagent, carrier) is `Unrecorded`.
- Per machine: `terminal::run` and `probe::print` each build `AccountLabels` from their own loaded config and `dirs::home_dir()`, so each machine labels its own homes and a remote's rows arrive labelled.
- Wire: `Account` serializes as `null` or `{"home":"~/.claude1","label":"claude1"}` (`serde(from = "Option<AccountHome>", into = "Option<AccountHome>")`). `ProbeAgentRow::account` is `#[serde(default)]`; `ChildRow::account` is `#[serde(default)]` and omitted while unrecorded (`Account::is_unrecorded`). `PROBE_SCHEMA` stays 3.
- Display: whether tiles and the summary name accounts (`AccountColumn`, `AgentRow::agent_label`) is decided once per frame; see `hierarchy.md`, Tile tree.

### Cell identity — `census/mod.rs`, `tiles.rs`, `census/probe.rs`

- `AgentRow` carries `started` (the session's start: for Claude Code, the transcript's first line once `attach_session_starts` reads it, else the process start) and `process_started` (the process's own start). `classify::agent_rows` sets both from `ProcessEntry::started`; only `started` moves later in the scan.
- `AgentCell { machine: String, pid: u32, process_started: u64 }` is the `TileGrid` key; `cell_order_placed` builds it from each placement root's `row.process_started`. A reused pid gets a new start time and so a new cell.
- `AgentRow` deserializes through `ProbeAgentRow`, whose `process_started: Option<u64>` falls back to `started` when a probe from an older build omits it. `PROBE_SCHEMA` stays `3`; the field is additive.

### Startup timing log — `startup_log.rs`

- `init()` reads the env var named by `FRAME_LOG_VARIABLE` (`CARGO_HANDLER_FRAME_LOG`) into a `OnceLock<StartupTiming>`; unset means `Disabled`, and `startup_event` and `startup_stage` return without timing anything.
- Each line is `<ms since start>ms <event>`, appended under a mutex. Milestones: `process_start`, `config_loaded`, `appearance_read_returned`, `alternate_screen_entered`, `first_frame_flushed`, `identify_start`/`identify_end`, `first_local_answer rows=N`, `first_agent_rows rows=N` (visible rows only; `Agent::CodexThreadHost` carriers are not counted), `hierarchy_failed <reason>`, `local_applied rows=N`, `remote_answer <host>`, `remote_applied <host>`, and `scan_stage_start <stage>` / `scan_stage_end <stage> elapsed=Nms`.
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
- Environment reads and account labels resolve in the scan (the scheduler thread locally, `probe::print` on a remote), never in rendering. The renderer reads only `Account` values already on the rows.
- An `AccountHome` is built only by `AccountLabels::resolve`, so a label always belongs to the home beside it.
- Probe compatibility for accounts: `PROBE_SCHEMA` stays 3, a probe printed before `account` existed parses to `Account::Unrecorded`, and an older reader ignores the field (`ProbeAgentRow` does not deny unknown fields).
- `GridLines::open` runs after every `add`; a band added later draws its line back.

## Calibration / gotchas

- The slow first scan is two cold first reads in the details stage: `transcript_launchers` (4.4 s cold, 5 ms warm) and `codex_app_server_threads` (2.4 s cold, 3 ms warm). Under load 162 they made a 36 s first answer before staging.
- Measured under load with staging: first frame ~11 ms, first agent rows ~118–228 ms, details seconds later.
- A progress timer (`Walker::timer`, `sleep_timer` in `census/tree.rs`) is read from the one `sleep` leaf under a shell wrapper, never from a script name or folder. That is why the move of `~/.claude/scripts/delegate` and `/tmp/claude/delegate` to `unit` needed no matcher change; `a_timer_under_the_unit_folders_reads_as_under_the_delegate_folders` pins both names.
- To record: `CARGO_HANDLER_FRAME_LOG=/path/to/log cargo-handler`. The file is appended to, never truncated; delete it between runs.
- The local scan runs on the scheduler thread, so while a slow scan runs no remote round starts; remotes already dispatched keep running on their own threads.
- `before_draw` calls `Attract::identify` only while the attract screen shows; `identify_start`/`identify_end` appear only then.
- `[accounts]` is read at startup: restart cargo-handler after an edit. Each machine labels its own homes, so a label change for the Mac's rows is an edit to the Mac's config, not natedev's.
- Account labels are ASCII (see `hierarchy.md`, gotchas: widths are counted in characters).
- `account.rs` imports `std::os::unix::ffi::OsStrExt` unguarded; cargo-handler is not built on Windows.
- A Mac install through `cargo install --git` does not read the checkout's `.cargo/config.toml`: pass its macOS RUSTFLAGS (the Swift `-L` and rpath link args) and `MACOSX_DEPLOYMENT_TARGET=14.0`.

## Why

- **Classified rows first.** The classification inputs (sessions, process table, Codex sqlite) are cheap; the details include the two cold reads above. Publishing the classified rows lets the summary list agents in a fraction of a second instead of tens of seconds.
- **First scan only.** After startup the detail caches are warm and a scan completes quickly; a classified publication then would only replace complete rows with bare ones.
- **Remotes first.** A remote probe costs the scheduler a thread spawn; scanning first would hold every remote behind the local scan, the slowest one included.
- **`process_started` as the key.** `attach_session_starts` moves `started` back to the transcript's first line during the details stage, so a key on `started` changed between the classified and completed answers and a resumed session lost its cell and focus.
- **Accounts resolved on each machine.** A machine's environments, home directory and `[accounts]` table are its own; resolving labels in its scan sends a remote's rows already labelled and keeps config and environment reads off the UI thread.
- **`resolve` as `AccountHome`'s only constructor.** A home and its label are computed together, so no row can carry a label taken from another home.
- **Ruled out for accounts:** `From<&AccountsConfig>` reading `dirs::home_dir()` (a `From` takes no outside context, so the machine home is a parameter); patching accounts onto classified rows through a second pid table; an `AccountHome::home()` getter with no consumer.
- **The spanning rule lives in tui_pane.** cargo-handler draws the summary at its full spanned width; the divider came from `draw_placements` adding each band's full-height frame over the summary's contents, so the fix sits where the lines are drawn.
- **Ruled out:** making the appearance read non-blocking (it costs 30 ms) and moving window identification off the UI thread (each pass ≤119 ms). Both were measured and left as they are.
