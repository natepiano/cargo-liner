# handler-hierarchy

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler nests each agent under the agent that manages it, as a command on each machine reports, shows only top-level agents in the summary, and labels each managed agent's role in its title.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15, the user (via the showrunner): "yes i want handler-enh to go ahead - i want the labels in the title - i want only top level showrunners to show in the summary". The showrunner's reading: role labels are in, and a showrunner that reports to another appears only inside its parent's tile, never as a summary entry of its own. natedev accepted the command contract below and builds the command.

## Contract — `hierarchy_command`

- Setting `[census] hierarchy_command` in cargo-handler's config; empty means off. Each machine's census runs its own command; natedev's is `~/.claude/scripts/lib/py ~/.claude/scripts/production/production_tree.py --json`.
- stdout, exit 0: `{"schema":1,"edges":[{"manager":"<sessionId>","member":"<sessionId>","role":"showrunner|unit|impl|test|review|fix"}]}`. `manager` is a Claude Code `sessionId` from the session records. `member` is a Claude `sessionId`, or for a Codex worker its Codex thread id from `mesh_roster.json` (Codex workers have no Claude session; ended, failed or server-down workers emit no edge). `role` is the member's role.
- Each machine emits only the edges whose member is local. The manager may be on another machine; cargo-handler joins by `sessionId` across every machine it shows.
- A unit whose showrunner is gone gets no edge and stays top level. A command that fails, times out (1 s) or prints malformed JSON yields no edges, and nesting falls back to today's launch match.

## Delegation Context

- **Project:** `cargo-handler`, the agent watcher; it draws through `tui_pane`.
- **Project started:** 2026-10-15T15:33:32.232+00:00
- **Stack:** Rust, cargo-nextest.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Key files** (under `crates/cargo-handler/src/`):
  - session records: `census/scan.rs` (`read_sessions` ≈645, path ≈198), `census/classify.rs` (`SessionRecord` ≈131, `claude_row` ≈295);
  - rows: `census/mod.rs` (`AgentRow` ≈134, `is_top_level` ≈182, `MachineState::top_level` ≈324, `RemoteMachines` ≈433);
  - probe: `census/probe.rs` (`ProbeAgentRow` ≈24, `ProbeReport` ≈62, `parse_report` ≈103), `constants.rs` (`PROBE_SCHEMA` = 3);
  - scheduler: `census/schedule.rs` (`spawn` ≈60, round loop ≈92); command-with-timeout pattern `census/remote.rs` (`SshRunner::probe` ≈152); TTL pattern `census/scan.rs` (`DesktopRead` ≈107);
  - config and settings: `config.rs` (`MachinesConfig` ≈103, `Config` ≈115), `settings.rs` (`AppSetting::RemoteMachines` ≈27), `app.rs` ≈144, `terminal.rs` ≈60;
  - cells: `agent_cell.rs` (`cell_order` ≈150, `Placing::sessions_of` ≈228, `SessionEntry` ≈113, `above_tree` ≈467, `header_line` ≈497), `tiles.rs` (`AgentCell`), `render.rs` (`group_title` ≈181);
  - summary: `summary/mod.rs` (uses `top_level()` at ≈65, 86, 152, 203, 316).
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Run/Smoke:** `cargo run -p cargo-handler` in a terminal, with `hierarchy_command` pointed at a fixture script printing edges for live session ids.
- **Invariants:** nothing on the UI thread runs a command or reads a file. Probe stays `PROBE_SCHEMA` 3: every new wire field is optional (`#[serde(default)]`), so older peers still parse. Tests run with nextest; format only with `cargo +nightly fmt`; no test takes a second.

## Phases

### Phase 1 — natedev's `production_tree.py --json` prints the edges  · status: todo

**Depends on:** none

#### Work Order

**Goal:** `~/.claude/scripts/lib/py ~/.claude/scripts/production/production_tree.py --json` prints this machine's edges per the contract, in well under a second.

**Spec:**
1. natedev's showrunner-fixer builds it in `~/.claude` after the delegate → unit folder rename, from `showrunners.py` parent links and the unit marks, reading both folder names. This unit writes none of it.
2. When natedev reports it landed, the unit director runs it on natedev and on the Mac, checks the output against the contract (ids resolve to live session records or live Codex threads; roles are from the set), and records the result here.
3. Then set it as `hierarchy_command` on each machine and run `cargo-handler probe` there: the probe's `edges` must carry the same links, which shows the consumer receives them.

**Constraints from prior phases:**
- The census runs the command with a 1 s timeout and a 1 MiB stdout cap (`HIERARCHY_TIMEOUT`, `HIERARCHY_OUTPUT_LIMIT` in `constants.rs`); over either yields no edges. Schema must be `1`.
- Each machine's probe loads its own config and runs its own command (`census/probe.rs` `print`), so a remote machine's edges come from the remote's own setting.

**Files:**
- `docs/cargo-port/remove-running-section-handler-hierarchy.md`

**Workers:** none — natedev builds the command; the unit director checks it.

**Acceptance gate:** the command's output on both machines matches the contract and names the live showrunner → unit links, and a configured `cargo-handler probe` on each machine reports those edges.

### Phase 2 — Each machine's census reads its hierarchy edges  · status: done

#### As-built

- `AgentRow.session_id: SessionIdentity::{Unrecorded, Claude(String)}`, set in `claude_row` from its `SessionRecord`; Codex and desktop-app rows are `Unrecorded`.
- `[census] hierarchy_command` on `CensusConfig` (default empty) with a Settings row (Census / hierarchy command). `HierarchySource::{Off, Command}` reaches the scheduler through `HierarchyCommand` (`Arc<RwLock>`, `new`/`replace`/`snapshot`) via `schedule::spawn(remotes, hierarchy)`, read each round.
- `census/hierarchy.rs`: `HierarchyRunner` trait; `CommandRunner` runs `sh -c` in its own process group with stdin and stderr closed, kills the group at `HIERARCHY_TIMEOUT` (1 s), caps output at `HIERARCHY_OUTPUT_LIMIT` (1 MiB), and fails with a named `HierarchyError`. `HierarchyCache` refreshes at most every 30 s, and at once when the local session-id set or the setting changes. Schema 1 parses into `HierarchyEdges(Vec<HierarchyEdge { manager, member, role: HierarchyRole }>)` with roles `Showrunner`, `Unit`, `Impl`, `Test`, `Review`, `Fix`; an unknown role drops that edge, an unknown schema drops the answer. Any failure (exit, timeout, malformed, oversize) yields no edges and one startup-log line when that log is on.
- `MachineState::Answered(rows, edges)` and `CensusUpdate::Local(rows, edges)`. Publication is staged: classified rows go out with no edges, completed rows carry them, so startup never waits on the command.
- Probe: optional `session_id` per row and `edges` on `ProbeReport`, `PROBE_SCHEMA` 3; `probe::print` loads its own config and runs the probed machine's own command; `parse_report` returns `(rows, edges)`; an old probe without the fields parses.

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs` — command runner, parse, cache
- `crates/cargo-handler/src/census/mod.rs` — `SessionIdentity`, edge types, `MachineState`/`CensusUpdate` shapes, `HierarchyCommand`
- `crates/cargo-handler/src/census/{schedule,scan,classify,probe}.rs` — round wiring, staged publish, row ids, wire fields
- `crates/cargo-handler/src/{config,settings,app,terminal,constants}.rs` — setting, settings row, shared handle, spawn call, constants

**Binds later work:** Nesting and role labels resolve manager/member ids through `SessionIdentity::Claude(String)` on `AgentRow`; each machine's edges sit on `MachineState::Answered(rows, edges)`, and classified answers carry no edges. `HierarchyEdge`, `HierarchyEdges` and `HierarchyRole` stay private, with no read accessor, until a consumer reads them; role labels need a display form per `HierarchyRole` variant. The 1 s timeout, 1 MiB cap, schema 1 and per-machine config (`probe::print` runs the machine's own command) are the contract the hierarchy command must meet. The Settings row is pinned by the `render.rs` settings golden and has no real-window shot yet.

**Gotchas:** cargo-mend narrows unused `pub(crate)` edge types back to private, so visibility opens only alongside a consumer. Codex workers are threads on the app-server row, not rows; their edge `member` is the Codex thread id.

**Ruled out:** a read accessor on edges before any consumer exists.

### Phase 3 — Managed agents nest in their manager's tile, and only top-level agents are summary entries  · status: done

#### As-built

- `census/placement.rs` builds one cross-machine forest per render from the current `MachineState`s, never persisted: `RowIdentity { machine, pid }`, `Placement::{Root, Managed(RowIdentity), Launched(RowIdentity)}` (`is_child_of`), `PlacedAgent { identity, row, placement }`, `ManagedThread { owner, owner_row, child_index, manager }`, `AgentPlacement { rows, threads, .. }` with `from_machines(&[Machine])`, `roots()` and `is_root(machine, pid)`, re-exported from `census/mod.rs`. Read access is `MachineState::edges()`; `HierarchyEdge`'s fields, `HierarchyEdges.0` and `HierarchyRole` are `pub(crate)`.
- A row's parent is the manager of an edge whose member is its `SessionIdentity::Claude` id and whose manager resolves to a listed Claude row on any machine, else its `launched_by` row on its own machine when listed, else none. Edges are taken in manager-id order, so the lowest resolving manager wins; self-links are skipped. Cycle checks run in session-id order and repeat until stable: a cycle-closing edge reverts the member to its launcher placement, in any row order. Launcher-only cycles get the old visible-root rescue: rows unreached from ordinary roots are promoted and listed after them. `roots()` keeps per-machine order with groups before lone agents.
- Codex threads carry `CodexThreadId::{Unrecorded, Recorded(String)}` (rollout id) on `ThreadEntry`, tree nodes and `ChildRow.thread_id` (omitted on the wire when unrecorded). A thread whose id is an edge member with a resolving manager becomes a `ManagedThread`, once per id, preferring a listed owner over a carrier; it leaves its owner's running work and nests as a tree line under its manager, showing its program and service tier.
- `Agent::CodexThreadHost` (`codex_thread_host`, name `app-server`): an app server not under the Codex desktop app, or beneath an agent, is a transport-only carrier for its threads. `AgentRow::is_visible` excludes it from placement rows, roots, `Census::lists_an_agent`, age repaints and the first-agent startup milestone. The probe sends carriers in an optional `thread_hosts` list beside `rows` (`ProbeReport::{from_answer, into_answer}`, `PROBE_SCHEMA` stays 3), so older readers never count them.
- `render.rs` builds one `AgentPlacement` and passes it to `agent_cell::cell_order_placed` and the summary's `height`/`width`/`draw`. Cells nest sessions depth first, oldest first; `SessionEntry` and `CellChild` carry their machine, and hue, name and child lookups match on machine and pid. `nest` reconciles `ChildKind::Session` markers with the forest so each member's work appears once, adding a marker for a hierarchy-only member. A member on another machine reads `<name> · <machine>` in its tree line.
- Summary entries are exactly placement roots (`summary::root_rows` through `is_root`), replacing `launched_by.is_none()`; `MachineState::top_level` and `AgentRow::is_top_level` are gone.
- The `census/codex.rs` fixture writer sets SQLite `synchronous=OFF`; its tests run in under a tenth of a second.

**Files:**
- `crates/cargo-handler/src/census/placement.rs` — placement forest, `mesh_worker_fixture` (test)
- `crates/cargo-handler/src/census/mod.rs` — `CodexThreadId`, `Agent::CodexThreadHost`, `is_visible`, `edges()`, placement re-exports
- `crates/cargo-handler/src/census/{scan,tree,classify,probe,codex}.rs` — thread ids, carrier classification, `thread_hosts` wire list, fixture speed
- `crates/cargo-handler/src/{agent_cell,summary/mod,render}.rs` — cell order, summary roots, shared placement
- `crates/cargo-handler/src/{terminal,startup_log}.rs` — repaint and milestone gates on visible rows

**Binds later work:** Role labels read the placement, not raw edges: `Placement::Managed(RowIdentity)` and `ManagedThread { manager, .. }` hold no role, so the accepted edge's `HierarchyRole` must be added where `AgentPlacement::from_machines` resolves the manager. `HierarchyRole` has no display text yet. `cell_order_placed` and the summary read one `AgentPlacement`. `Agent::CodexThreadHost` rows are never drawn and never roots (`AgentRow::is_visible`); a label lands only on the threads they carry.

**Gotchas:** Idle Codex threads have no open rollout, so edges naming them (14 of 56 real edges) never resolve and those workers fall back to their launcher. `AgentPlacement` borrows the machine states, so it is rebuilt each render.

**Ruled out:** a `thread_host` flag on ordinary probe rows (older readers would count carriers as agents).

### Phase 4 — Role labels in the title  · status: todo

**Depends on:** Phase 3 (`crates/cargo-handler/src/agent_cell.rs`)

#### Work Order

**Goal:** each tile's title, and each nested member's tree line, names the agent's role.

**Spec:**
1. Roles come from the relationships phase 3's placement accepted, never raw edges: a dropped, stale or unresolved edge labels nothing. Placement does not keep the accepted edge's role yet: `Placement::Managed(RowIdentity)` and `ManagedThread { manager, .. }` (`census/placement.rs`) gain the accepted edge's `HierarchyRole`, set where `AgentPlacement::from_machines` resolves the manager. A member's role is its accepted edge's role. A manager's role, when no accepted edge names it, is `showrunner` if it manages a `unit` or `showrunner`, else `unit` if it manages workers. A member whose manager is absent keeps its own edge's role only while that edge is accepted. Hold the result in a semantic type (for example `RoleLabel::{Unlabeled, Stated(HierarchyRole), Inferred(HierarchyRole)}`), not a bare `Option<HierarchyRole>`; `HierarchyRole` gains its display text in `census/mod.rs`.
2. Titles become styled segments: `TileCells::group_title` (`tui_pane/src/tiles/draw.rs` ≈243) returns a `Line` rather than one `Span`, and `GridLines::add_titled` (`tui_pane/src/pane/frame.rs` ≈484) takes `impl Into<Line>`, each segment's style patched over the chrome's title style as a span's is today, so existing span callers compile unchanged. Clipping, focus weight and title motion keep working per segment.
3. `group_title` appends the role after the name, as `<name> · <role>`: the name in its rainbow hue, ` · <role>` in the label color; an unlabeled agent shows no suffix.
4. A nested member's tree line, Claude row or Codex thread, shows its role after its name the same way.
5. Tests: framework — a two-segment title keeps both styles, clips at the cell's width, takes focus weight, and moves with the title motion. Handler — explicit role beats inference; inferred showrunner and unit; an unlabeled launch-only group; labels follow a relationship change (an edge arriving, leaving); title and tree line text read back.

**Constraints from prior phases:**
- `HierarchyRole::{Showrunner, Unit, Impl, Test, Review, Fix}` parses the contract's role strings (`census/hierarchy.rs`).
- Edges arrive and leave round by round (classified answers carry none; a failed or off command clears them), so labels must follow the current placement, not a first sighting.
- Placement API (`census/placement.rs`): `RowIdentity { machine, pid }`, `Placement::{Root, Managed, Launched}`, `PlacedAgent { identity, row, placement }`, `ManagedThread { owner, owner_row, child_index, manager }`, `AgentPlacement::{from_machines, roots, is_root}`; `agent_cell.rs` `cell_order` and the summary both read one `AgentPlacement`.
- `Agent::CodexThreadHost` rows are transport-only app servers: never drawn, never roots, skipped by `AgentRow::is_visible`; a label never lands on one, only on the Codex threads they carry.
- About a quarter of real edges name idle Codex threads the census cannot see; they never resolve and label nothing.
- `crates/tui_pane/` is shared, and tile-fixes owns it with uncommitted work there; its CHANGELOG notes the title API change. Before the framework writer starts, the unit director claims the title files through cargo-berth; if they overlap tile-fixes' reservation, the showrunner settles the order and nothing is written ahead of it.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`
- `crates/tui_pane/src/pane/frame.rs`
- `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/src/agent_cell.rs`
- `crates/cargo-handler/src/census/mod.rs`
- `crates/cargo-handler/src/census/placement.rs`

**Workers:** 2 writers — framework title segments, then the handler's labels.
- `impl` — `tui_pane/src/tiles/draw.rs`, `tui_pane/src/pane/frame.rs`, `tui_pane/CHANGELOG.md`; hub: `pane/frame.rs`; posts the title API on the board first
- `test` opens as `impl` — `render.rs`, `agent_cell.rs`, `census/mod.rs`, `census/placement.rs`; hub: `census/placement.rs`

**Acceptance gate:** tests pass; suite and lint green; shots on the unit's port show the labels on a showrunner tile, its nested units and their Codex workers.
