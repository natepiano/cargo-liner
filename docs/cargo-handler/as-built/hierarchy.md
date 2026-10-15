# cargo-handler agent hierarchy — as-built

A reference for the next engineer who changes how cargo-handler learns which agent manages which, nests agents in tiles, or labels their roles.

## What it is

cargo-handler nests each agent under the agent that manages it, as a command configured on each machine reports. A managed Claude session or Codex worker thread appears as a tree line inside its manager's tile, possibly across machines, and never titles a tile or appears as a summary entry of its own: only top-level agents (placement roots) are summary entries. Each tile title reads `<name> · <role>` and each nested tree line shows its role after its name, with roles `showrunner`, `unit`, `impl`, `test`, `review` and `fix`. With no command configured, or before the command first answers, nesting falls back to the launch match (`AgentRow::launched_by`).

Where the code lives (paths relative to `crates/cargo-handler/src/` unless noted):
- command runner, contract parse, cache, last-good retention: `census/hierarchy.rs`
- domain types (`SessionIdentity`, `HierarchyRole`, `HierarchyEdge`, `HierarchyEdges`, `HierarchySource`, `HierarchyCommand`, `CodexThreadId`, `Agent::CodexThreadHost`, `MachineState`, `RemoteHierarchyAnswers`): `census/mod.rs`
- placement forest and role labels: `census/placement.rs`
- scheduler wiring (`LocalAnswers`, remote per-host answers): `census/schedule.rs`
- wire fields (`session_id`, `edges`, `hierarchy_failed`, `thread_hosts`): `census/probe.rs`
- session ids on rows: `census/classify.rs` (`claude_row`)
- setting: `config.rs` (`CensusConfig`), `settings.rs` (`AppSetting::HierarchyCommand`), `app.rs` (`App::hierarchy_command`), `terminal.rs` (spawn call)
- constants: `constants.rs`
- tile order and tree lines: `agent_cell.rs`; tile titles: `render.rs` (`Cells::group_title`); summary roots: `summary/mod.rs` (`root_rows`)
- styled titles and bounded blit: `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/tiles/draw.rs`

## How it works

### The `hierarchy_command` contract

- `[census] hierarchy_command` in `config.toml` (`CensusConfig::hierarchy_command: String`, default empty) is one line of shell. `HierarchySource::from(String)` maps an empty or whitespace-only line to `HierarchySource::Off`, anything else to `HierarchySource::Command(String)`.
- stdout on exit 0 is `{"schema":1,"edges":[{"manager":"<sessionId>","member":"<id>","role":"<role>"}]}` (`HIERARCHY_SCHEMA` = 1). `manager` is a Claude Code `sessionId` from the session records. `member` is a Claude `sessionId`, or for a Codex worker its Codex thread id (the id in its rollout filename). `role` is the member's role.
- Each machine emits only the edges whose member is local. The manager may run on another machine; placement joins by session id across every machine shown.
- Parse (`hierarchy::parse`): a foreign schema fails the whole answer (`HierarchyError::UnknownSchema`); an unknown role (`HierarchyWireRole::Unknown`, `#[serde(other)]`) drops only that edge; a missing field fails the answer (`MalformedEdge`), as does an empty `manager` or `member` (`EmptySessionId`). Valid edges become `HierarchyEdges(Vec<HierarchyEdge { manager, member, role: HierarchyRole }>)`.

### Setting and Settings row

- The Settings overlay lists the command under the `Census` section with label `hierarchy command` (`CENSUS_SETTINGS_SECTION`, `HIERARCHY_COMMAND_LABEL`), edited as one text line. `settings::set_hierarchy_command` writes the text into the loaded config and calls `HierarchyCommand::replace(HierarchySource::from(text))`; it never runs the command. An empty edit turns discovery off.
- `HierarchyCommand(Arc<RwLock<HierarchySource>>)` with `new`, `replace` and `snapshot` is shared between `App::hierarchy_command` and the scheduler: `terminal::run` calls `census::schedule::spawn(app.remote_machines.clone(), app.hierarchy_command.clone())`, and the scheduler takes a `snapshot()` for each publication, so an edit applies from the next local scan.

### The runner — `census/hierarchy.rs`

- `trait HierarchyRunner: Send + Sync { fn run(&self, command: &str, timeout: Duration) -> Result<HierarchyEdges, HierarchyError>; }` lets scheduler tests substitute fixtures; `CommandRunner` is the production runner.
- `CommandRunner::run` spawns `sh -c <command>` with stdin and stderr null and stdout piped, in its own process group (`process_group(0)` on unix). A reader thread reads at most `HIERARCHY_OUTPUT_LIMIT + 1` bytes, so oversized output fails (`OutputTooLarge`) without waiting for EOF. The main thread waits on that read until the deadline, then polls `try_wait` every `PROBE_EXIT_CHECK` (20 ms) until the deadline. On timeout or any read failure `end` runs `kill -KILL -- -<pgid>` on the whole group, then kills and reaps the shell.
- `HierarchyError` names each failure: `CouldNotStart`, `TimedOut`, `ExitedUnsuccessfully`, `OutputUnreadable`, `OutputTooLarge`, `ExitUnreadable`, `UnknownSchema(u32)`, `MalformedEdge`, `EmptySessionId`.
- `hierarchy::read(source, runner) -> HierarchyAnswer` runs one discovery with `HIERARCHY_TIMEOUT`. `HierarchySource::Off` answers `HierarchyAnswer::Answered(HierarchyEdges::default())`. A failure writes one `hierarchy_failed <reason>` line to the startup log (when `CARGO_HANDLER_FRAME_LOG` is set) and answers `HierarchyAnswer::Failed`, which is distinct from an authoritative empty answer.

### Cache, refresh and last-good retention

- `HierarchyCache { source, sessions: BTreeSet<String>, success: LastSuccessfulHierarchy, retry: HierarchyRetry }` lives in the scheduler's `LocalAnswers`.
- `LastSuccessfulHierarchy::{Unanswered, Answered { refreshed: Instant, edges }}` holds the last successful answer for the current source; `edges()` returns it, or no edges when unanswered.
- `HierarchyCache::edges(source, rows, runner, now)` reuses the success while all hold: the retry state is `HierarchyRetry::OnExpiration`, the set of local `SessionIdentity::Claude` ids equals the cached set, and the success is younger than `HIERARCHY_REFRESH_INTERVAL`. Otherwise it runs the command. A success replaces the retained edges; a failure keeps them and sets `HierarchyRetry::NextTick`, so the next local scan retries regardless of age.
- `HierarchyCache::cached(source)` clears the retained success when the source changes or is `Off`, so a new or disabled command never shows the old command's edges.
- Retained edges are not pruned; a stale edge whose member or manager is no longer listed resolves to nothing in placement.

### Scheduler — `census/schedule.rs`

- `spawn(remotes, hierarchy: HierarchyCommand) -> Receiver<CensusUpdate>`; `spawn_staged_with(remotes, hierarchy, hierarchy_runner: Arc<dyn HierarchyRunner>, runner: Arc<dyn RemoteRunner>, scan_local: impl FnMut(&mut LocalAnswers) -> ScanSchedule, cadence)`.
- `LocalAnswers::publish(stage, rows)`: `ScanStage::Classified` rows go out as `CensusUpdate::Local(rows, HierarchyEdges::default())`. `ScanStage::Completed` rows go out first with the cache's current edges (`cached(..).edges()`), then `LocalAnswers::send` runs `HierarchyCache::edges` (which may run the command) and publishes the same rows with the fresh edges. Completed row detail is never held behind a command that takes its whole deadline. `Census::apply` skips a repaint when the second publication equals the first.
- Remote answers: each probe thread passes `probe::parse_answer` output to `RemoteHierarchyAnswers::apply(host, answer)`. `HierarchyAnswer::Answered(edges)` stores and applies them; `HierarchyAnswer::Failed` applies the host's last stored edges (or none); an unreadable answer becomes `MachineState::Failed(reason)`. `RemoteHierarchyAnswers::retain(configured)` runs at every remote round and before each answer applies, and an answer for a host no longer configured is discarded, so a removed host cannot restore its old edges.
- Each machine's edges sit on `MachineState::Answered(Vec<AgentRow>, HierarchyEdges)`, read through `MachineState::edges()`; `CensusUpdate::Local(Vec<AgentRow>, HierarchyEdges)`.

### Probe — `census/probe.rs`

- `PROBE_SCHEMA` stays 3. New fields are all optional: `AgentRow.session_id` (via `ProbeAgentRow`, `#[serde(default)] Option<String>`, mapped to `SessionIdentity`), `ProbeReport.edges` (`#[serde(default)]`), `ProbeReport.hierarchy_failed` (`#[serde(default)]`, omitted when false, so absent means answered), `ProbeReport.thread_hosts` (`#[serde(default)]`, omitted when empty), and `ChildRow.thread_id` (omitted when `CodexThreadId::Unrecorded`).
- `probe::print` loads the probed machine's own config and runs its own `hierarchy_command` once per probe through `hierarchy::read` (no cache on the probe side); a failure sets `hierarchy_failed`.
- `ProbeReport::from_answer` partitions rows into visible `rows` and transport-only `thread_hosts` (`AgentRow::is_visible`), so an older reader never counts a carrier as an agent; `into_answer` restores `Agent::CodexThreadHost` on the carriers and maps `hierarchy_failed` to `HierarchyAnswer::Failed`.
- `parse_answer(&[u8]) -> Result<(Vec<AgentRow>, HierarchyAnswer), String>` keeps failure distinct; `parse_report` collapses it to `(rows, edges)`.

### Identities on rows

- `AgentRow.session_id: SessionIdentity::{Unrecorded, Claude(String)}`, set in `classify::claude_row` from its `SessionRecord`; Codex and desktop-app rows are `Unrecorded`. `SessionIdentity::from(String)` maps an empty id to `Unrecorded`.
- Codex worker threads are children (`ChildKind::Thread`) of an app-server row, not rows; they carry `CodexThreadId::{Unrecorded, Recorded(String)}` on `ChildRow.thread_id`.
- `Agent::CodexThreadHost` (name `app-server`) is an app server not under the Codex desktop app or beneath an agent: a transport-only carrier for its threads. `AgentRow::is_visible` is false for it; it never becomes a placement row, a root, a summary entry or a startup `first_agent_rows` count.

### Placement — `census/placement.rs`

`AgentPlacement::from_machines(&[Machine])` builds one cross-machine forest per render from the current `MachineState`s; it borrows them and is never stored. `render::Cells::new` builds it once and hands it to `agent_cell::cell_order_placed` and the summary's `height`, `width` and `draw`.

- Types: `RowIdentity { machine: &str, pid: u32 }`; `Placement::{Root, Managed { manager: RowIdentity, role: HierarchyRole }, Launched(RowIdentity)}` with `is_child_of(parent)`; `PlacedAgent { identity, row, placement }`; `ManagedThread { owner, owner_row, child_index, manager, role }`; `AgentPlacement { rows: Vec<PlacedAgent>, threads: Vec<ManagedThread>, rescued_roots }`. All re-exported from `census/mod.rs`.
- Rows: every visible row starts as `Root`, then becomes `Launched(launcher)` when its `launched_by` pid is a listed row on its own machine.
- Managers: edges from every machine are sorted by manager id. The manager map holds each listed `Agent::Claude` row by its `SessionIdentity::Claude` id, on any machine. A row whose `SessionIdentity::Claude` id is an edge member becomes `Managed { manager, role }` from the first edge whose manager resolves and is not the row itself, so the lowest resolving manager id wins.
- Cycles: rows are checked in session-id order, repeating until stable; a `Managed` link that closes a cycle reverts that row to its launcher placement. Launcher-only cycles are then rescued: rows unreached from ordinary roots become `Root` and are listed after them (`rescued_roots`).
- Threads: a `ChildKind::Thread` child with `CodexThreadId::Recorded(id)` that is an edge member with a resolving manager becomes a `ManagedThread`, once per id, preferring a listed owner over a `CodexThreadHost` carrier. It leaves its owner's running work and nests as a tree line under its manager, showing its program and service tier.
- `roots()` lists `Root` rows per machine in census order, groups (roots with members or managed threads) before lone agents, rescued roots last. `is_root(machine, pid)` answers summary membership.

### Role labels

- `RoleLabel::{Unlabeled, Stated(HierarchyRole), Inferred(HierarchyRole)}`, from `AgentPlacement::role_label(identity)`.
- A row placed `Managed { role, .. }` is `Stated(role)`. Otherwise the label is inferred from what the row manages: any managed member (row or thread) with role `Unit` or `Showrunner` makes it `Inferred(Showrunner)`; any other managed member makes it `Inferred(Unit)`; no managed members leaves it `Unlabeled`. A managed thread's label is always `Stated(thread.role)`.
- `RoleLabel::suffix()` returns `" · <role>"` (`HEADING_SEPARATOR` then `HierarchyRole::text()`), or an empty string for `Unlabeled`.

### Rendering

- Tile title: `Cells::group_title` returns `Line` of two spans: `AGENT_CELL_TITLE_LEAD` + name in `Role::Rainbow(entry.hue)`, then `entry.role_label.suffix()` in `label_color()`. A launch-only group has no suffix.
- Tree lines: `SessionEntry::name_spans` draws the name in its hue followed by the suffix in the label color. A member on another machine reads `<name> · <machine>` before its role suffix.
- `cell_order_placed` nests sessions depth first, oldest first; `SessionEntry` and `CellChild` carry their machine, and hue, name and child lookups match on machine and pid. `nest` reconciles `ChildKind::Session` markers with the forest so each member's work appears once, adding a marker for a member known only from the hierarchy.
- Summary: `summary::root_rows` keeps only rows where `placement.is_root(machine, pid)`.

### Styled titles in tui_pane

- `TileCells::group_title(&self, id) -> Option<Line<'static>>` (default `None`) and `GridLines::add_titled(&mut self, frame: PaneFrame, title: impl Into<Line<'a>>)`. A `&str` or `String` still converts.
- `GridLines::write_overlay` patches the line's and each span's style over `chrome.title_style(focused)`, so a span that sets only a color keeps the focus weight. A title wider than its row is cut to `width - 1` and ends in `…` (`ELISION`) styled like the last visible grapheme, so segment styles survive clipping and motion.
- `draw_clipped` binds each pane to `ClippedPaneDestination { buffer, frame, visible }`, where `visible` is the motion clip intersected with the buffer's area and the pane's columns. A still pane draws directly only when its whole rect fits `visible`; any other pane draws into a scratch buffer and `blit` copies only cells inside `visible`, so a pane overhanging the buffer cannot write outside it.

## Invariants

- Nothing on the UI thread runs a command or reads a file: the command runs on the scheduler thread (local) or inside the remote's `cargo-handler probe`; a Settings edit only replaces the shared source.
- Startup never waits on the command: classified rows carry no edges, and completed rows publish before discovery runs.
- `PROBE_SCHEMA` stays 3; every new wire field is optional, so older peers parse newer probes and newer readers parse older ones.
- `HierarchyAnswer::Failed` never erases edges: locally the cache keeps its last success, remotely `RemoteHierarchyAnswers` keeps each host's. A source change or `Off` clears the local success; removing a host clears its edges.
- An explicit edge role wins over inference.
- A stale, dropped or unresolved edge nests nothing and labels nothing; the member falls back to its launcher placement or stays top level. A unit whose showrunner is gone gets no edge from the command and stays top level.
- Only placement roots are summary entries and tile titles; a `CodexThreadHost` carrier is never drawn.
- Placement is rebuilt each render from current state; it is not persisted.
- `HierarchyEdge`, `HierarchyEdges` and `HierarchyRole` are `pub(crate)` only for placement; no read accessor beyond `MachineState::edges()`.
- `blit` writes only inside `ClippedPaneDestination::visible`.

## Calibration / gotchas

- `HIERARCHY_TIMEOUT` is 10 s. `PROBE_TIMEOUT` is `HIERARCHY_TIMEOUT + PROBE_SCAN_SSH_MARGIN` (10 s) = 20 s, so a remote probe has room for its own discovery plus scanning and ssh; raising the timeout raises the probe timeout with it.
- `HIERARCHY_OUTPUT_LIMIT` is 1 MiB before JSON decoding.
- `HIERARCHY_REFRESH_INTERVAL` is 30 s: a successful local answer is reused up to 30 s while the local Claude session-id set and the source stay unchanged. A changed session set or a failed run triggers a run on the next local scan (`LOCAL_SCAN_INTERVAL`, 2 s). The remote side runs its command on every probe (`REMOTE_PROBE_INTERVAL`, 5 s, skipped while a probe is in flight).
- natedev's producer is `~/.claude/scripts/lib/py ~/.claude/scripts/production/production_tree.py --json`. It lives in `~/.claude`, outside this repository, and takes about 1 s under load. A member whose showrunner runs on the other machine gets its edge from the showrunner's update-timer record.
- Config paths: Linux `$XDG_CONFIG_HOME/cargo-handler/config.toml` (`~/.config/...`); macOS `~/Library/Application Support/cargo-handler/config.toml`, which ignores `XDG_CONFIG_HOME`.
- A capture with a scratch config needs an absolute `XDG_CONFIG_HOME` on Linux; a relative one silently falls back to `~/.config` and reads the real config.
- Idle Codex threads have no open rollout and so are not listed as threads; edges naming them never resolve, and those workers fall back to their launcher. These unresolved members are expected, not a command defect.
- Removing and re-adding a host within one probe's 20 s life can let the old probe's answer apply. This race is accepted.
- cargo-mend narrows unused `pub(crate)` items back to private, so visibility on the edge types opens only alongside a consumer.
- Fixtures: `hierarchy_command` can point at a script that prints edges for live session ids; `placement::mesh_worker_fixture` (test) builds a cross-machine manager and a Codex mesh worker.

## Why

- **A file-based command, not message inference.** Inferring management from session messages was rejected as too heavyweight. The tool that launches managed agents already knows the tree, so it states it, and cargo-handler only joins ids.
- **Per-machine commands.** Each machine sees only its own session records and Codex rollouts, so each emits the edges for its local members; the join by session id handles managers on another machine, and remotes deliver their edges in the probe they already send.
- **Last-good retention instead of a failure indicator.** A failed or timed-out run says nothing about the hierarchy, so it keeps the last good edges and retries next tick. A user-facing failure indicator was ruled out because it gives the user nothing to act on.
- **Explicit role over inference.** The command knows a member's role; inference only labels managers the command reports no incoming edge for (a top-level showrunner or unit).
- **`thread_hosts` as a separate list.** A `thread_host` flag on ordinary probe rows was ruled out: older readers would count carriers as agents.
- **No edge read accessor ahead of a consumer.** API surface opens only when placement or rendering reads it.
