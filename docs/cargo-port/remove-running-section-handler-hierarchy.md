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

**Depends on:** Phase 2 (`crates/cargo-handler/src/census/mod.rs`)

#### Work Order

**Goal:** an agent named as a member of a live manager on any shown machine sits in that manager's tile, below it in the tree, and is not a summary entry; with no edges the grid and summary are as today.

**Spec:**
1. Open read access to each machine's edges (`MachineState` accessor; the edge fields and `HierarchyRole` crate-visible), which phase 2 kept private for want of a consumer.
2. Keep each Codex app-server thread's id: `ThreadEntry` (`census/scan.rs` ≈615–670) gains a domain-typed thread id (for example `CodexThreadId`), carried on the probe as an optional field. A Codex worker is a thread on its app-server row, not a row of its own.
3. Build one cross-machine placement before cells and summary, in `census/mod.rs` (the hub): resolve each edge's `manager` to a Claude row by `SessionIdentity`, and its `member` to a Claude row by `SessionIdentity` or to an app-server thread by thread id, across every machine. A member's parent is its edge's manager when that resolves, else its `launched_by` row on its own machine when listed, else none. Give each placed row a semantic state (for example `Placement::{Root, Managed(..), Launched(..)}`), not a bare `Option`.
4. Normalize to a forest so every listed row is placed exactly once: drop self-links; when one member has several edges, keep the one whose manager resolves, ties broken by the lowest manager id; an edge that would close a cycle (through edges or launch fallback) is dropped and the member falls back to its launcher, the existing visible-root rescue in `cell_order` deciding what remains. Publish the placement API on the board before the summary writer starts.
5. `cell_order` runs as one pass over every machine from that placement: a root is a row with no parent; sessions nest depth first, oldest first, as today. `SessionEntry` carries its machine, and hue, name and child lookups match on machine and pid. A placed member brings its running work exactly once: `group_children`/`nest` (`agent_cell.rs` ≈674–718) today pull another session's children only through a `ChildKind::Session` marker in the launch tree, so reconcile those markers after hierarchy overrides and carry the machine through child lookup, deduplication and coloring. A thread member nests as a line under its manager's tile. Keep `AgentCell`'s key and the hue order rules.
6. The summary lists exactly the rows that are cell roots, through the same predicate, replacing `launched_by.is_none()`. This also puts a row whose launcher is not listed back in the summary, matching its cell.
7. A member on another machine reads its machine in its tree line (the header already names machines).
8. Bring the two `census/codex.rs` tests over a second (`reads_interactive_threads_created_in_the_span` ≈1.25 s, `reads_threads_by_id_whoever_started_them` ≈1.14 s) under one: find what they wait on and remove the wait, not the coverage.
9. Tests: unit under showrunner on one machine; unit on `mac` under a showrunner on `natedev`, with running children, and a pid on `mac` equal to one on `natedev`; a Codex thread under its unit; showrunner under showrunner (the child is not a summary entry); an unresolved manager with a listed launcher nests under the launcher; an unresolved manager with no listed launcher leaves the member top level; an edge overrides a launch match; a self-link, a two-edge cycle and two edges for one member each place every row once; transitions: classified answers (no edges) then completed (edges) reparent, the command failing or turned off restores roots, a remote manager leaving and returning re-nests its member with no member id change; no edges reproduces today's order.

**Constraints from prior phases:**
- `MachineState::Answered(rows, edges)`: startup's classified answer carries no edges; the completed answer carries the cached edges (30 s, refreshed when the session set or the setting changes); a failed or off command gives no edges. Remote rows and each machine's edges change independently, round by round.
- `AgentRow.session_id` is `SessionIdentity::{Unrecorded, Claude(String)}`; Codex and desktop rows are `Unrecorded`.
- The settings row (`Census` / `hierarchy command`) is pinned by the `render.rs` settings golden; its config and handle tests stay as they are.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs`
- `crates/cargo-handler/src/census/mod.rs`
- `crates/cargo-handler/src/summary/mod.rs`
- `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/src/census/codex.rs`
- `crates/cargo-handler/src/census/scan.rs`
- `crates/cargo-handler/src/census/probe.rs`

**Workers:** 2 writers — placement and cells; summary and the codex test speed.
- `impl` — `agent_cell.rs`, `census/mod.rs`, `census/scan.rs`, `census/probe.rs`, `render.rs`; hub: `census/mod.rs`
- `test` opens as `impl` — `summary/mod.rs`, `census/codex.rs`

**Acceptance gate:** the tests pass, and no `census/codex.rs` test takes a second; suite and lint green; live on both machines with the real or fixture command, a showrunner's tile holds its units and their workers, and the summary lists only top-level agents. Shots on the unit's port: summary and grid with a nested showrunner, a unit, a cross-machine unit and a nested Codex worker; the settings overlay with the hierarchy command filled in, and emptied (nesting falls back).

### Phase 4 — Role labels in the title  · status: todo

**Depends on:** Phase 3 (`crates/cargo-handler/src/agent_cell.rs`)

#### Work Order

**Goal:** each tile's title, and each nested member's tree line, names the agent's role.

**Spec:**
1. Roles come from the relationships phase 3's placement accepted, never raw edges: a dropped, stale or unresolved edge labels nothing. A member's role is its accepted edge's role. A manager's role, when no accepted edge names it, is `showrunner` if it manages a `unit` or `showrunner`, else `unit` if it manages workers. A member whose manager is absent keeps its own edge's role only while that edge is accepted. Hold the result in a semantic type (for example `RoleLabel::{Unlabeled, Stated(HierarchyRole), Inferred(HierarchyRole)}`), not a bare `Option<HierarchyRole>`; `HierarchyRole` gains its display text in `census/mod.rs`.
2. Titles become styled segments: `TileCells::group_title` (`tui_pane/src/tiles/draw.rs` ≈243) returns a `Line` rather than one `Span`, and `GridLines::add_titled` (`tui_pane/src/pane/frame.rs` ≈484) takes `impl Into<Line>`, each segment's style patched over the chrome's title style as a span's is today, so existing span callers compile unchanged. Clipping, focus weight and title motion keep working per segment.
3. `group_title` appends the role after the name, as `<name> · <role>`: the name in its rainbow hue, ` · <role>` in the label color; an unlabeled agent shows no suffix.
4. A nested member's tree line, Claude row or Codex thread, shows its role after its name the same way.
5. Tests: framework — a two-segment title keeps both styles, clips at the cell's width, takes focus weight, and moves with the title motion. Handler — explicit role beats inference; inferred showrunner and unit; an unlabeled launch-only group; labels follow a relationship change (an edge arriving, leaving); title and tree line text read back.

**Constraints from prior phases:**
- `HierarchyRole::{Showrunner, Unit, Impl, Test, Review, Fix}` parses the contract's role strings (`census/hierarchy.rs`).
- Edges arrive and leave round by round (classified answers carry none; a failed or off command clears them), so labels must follow the current placement, not a first sighting.
- `crates/tui_pane/` is shared, and tile-fixes owns it with uncommitted work there; its CHANGELOG notes the title API change. Before the framework writer starts, the unit director claims the title files through cargo-berth; if they overlap tile-fixes' reservation, the showrunner settles the order and nothing is written ahead of it.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`
- `crates/tui_pane/src/pane/frame.rs`
- `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/src/agent_cell.rs`
- `crates/cargo-handler/src/census/mod.rs`

**Workers:** 2 writers — framework title segments, then the handler's labels.
- `impl` — `tui_pane/src/tiles/draw.rs`, `tui_pane/src/pane/frame.rs`, `tui_pane/CHANGELOG.md`; hub: `pane/frame.rs`; posts the title API on the board first
- `test` opens as `impl` — `render.rs`, `agent_cell.rs`, `census/mod.rs`

**Acceptance gate:** tests pass; suite and lint green; shots on the unit's port show the labels on a showrunner tile, its nested units and their Codex workers.
