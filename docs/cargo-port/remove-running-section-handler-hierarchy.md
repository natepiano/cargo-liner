# handler-hierarchy

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler nests each agent under the agent that manages it, as a command on each machine reports, shows only top-level agents in the summary, and labels each managed agent's role in its title.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15, the user (via the showrunner): "yes i want handler-enh to go ahead - i want the labels in the title - i want only top level showrunners to show in the summary". The showrunner's reading: role labels are in, and a showrunner that reports to another appears only inside its parent's tile, never as a summary entry of its own. natedev accepted the command contract below and builds the command.

## Contract — `hierarchy_command`

- Setting `[census] hierarchy_command` in cargo-handler's config; empty means off. Each machine's census runs its own command; natedev's is `~/.claude/scripts/lib/py ~/.claude/scripts/production/production_tree.py --json`.
- stdout, exit 0: `{"schema":1,"edges":[{"manager":"<sessionId>","member":"<sessionId>","role":"showrunner|unit|impl|test|review|fix"}]}`. `manager` is a Claude Code `sessionId` from the session records. `member` is a Claude `sessionId`, or for a Codex worker its Codex thread id from `mesh_roster.json` (Codex workers have no Claude session; ended, failed or server-down workers emit no edge). `role` is the member's role.
- Each machine emits only the edges whose member is local. The manager may be on another machine; cargo-handler joins by `sessionId` across every machine it shows.
- A unit whose showrunner is gone gets no edge and stays top level. A command that fails, times out (10 s) or prints malformed JSON keeps the last good edges for an unchanged command, still filtered so stale members drop, and is retried on the next tick; with no good answer yet, nesting falls back to today's launch match.

## Delegation Context

- **Project:** `cargo-handler`, the agent watcher; it draws through `tui_pane`.
- **Project started:** 2026-10-15T15:33:32.232+00:00
- **Stack:** Rust, cargo-nextest.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Key files** (under `crates/cargo-handler/src/`):
  - session records: `census/scan.rs` (`read_sessions` ≈645, path ≈198), `census/classify.rs` (`SessionRecord` ≈131, `claude_row` ≈295);
  - rows: `census/mod.rs` (`AgentRow`, `RemoteMachines`); placement: `census/placement.rs` (`RoleLabel` ≈65, `AgentPlacement` ≈115, `role_label` ≈372); hierarchy: `census/hierarchy.rs` (`HierarchyAnswer` ≈253, `LastSuccessfulHierarchy` ≈276);
  - probe: `census/probe.rs` (`ProbeAgentRow`, `ProbeReport`, `parse_answer` ≈176), `constants.rs` (`PROBE_SCHEMA` = 3, `HIERARCHY_TIMEOUT` ≈536, `PROBE_TIMEOUT` ≈572);
  - scheduler: `census/schedule.rs` (`spawn` ≈60, round loop ≈92); command-with-timeout pattern `census/remote.rs` (`SshRunner::probe` ≈152); TTL pattern `census/scan.rs` (`DesktopRead` ≈107);
  - config and settings: `config.rs` (`MachinesConfig` ≈103, `Config` ≈115), `settings.rs` (`AppSetting::RemoteMachines` ≈27), `app.rs` ≈144, `terminal.rs` ≈60;
  - cells: `agent_cell.rs` (`cell_order_placed` ≈236, `SessionEntry`, `above_tree`, `header_line`), `tiles.rs` (`AgentCell`), `render.rs` (`group_title`);
  - summary: `summary/mod.rs` (`root_rows` ≈111: only placement roots are summary entries).
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Run/Smoke:** `cargo run -p cargo-handler` in a terminal, with `hierarchy_command` pointed at a fixture script printing edges for live session ids.
- **Invariants:** nothing on the UI thread runs a command or reads a file. Probe stays `PROBE_SCHEMA` 3: every new wire field is optional (`#[serde(default)]`), so older peers still parse. Tests run with nextest; format only with `cargo +nightly fmt`; no test takes a second.

## Phases

### Phase 1 — natedev's `production_tree.py --json` prints the edges  · status: done

#### As-built

- natedev's `~/.claude/scripts/lib/py ~/.claude/scripts/production/production_tree.py --json` (built in `~/.claude`, not this repository) prints this machine's edges in about 1 s under load: 50–62 on natedev, none on the Mac, where no managed agents run. A member whose showrunner runs on the other machine gets its edge from the showrunner's update-timer record (`~/.claude` 30795ad8).
- `hierarchy_command` is set to that command on both machines: natedev in `~/.config/cargo-handler/config.toml`, the Mac in `~/Library/Application Support/cargo-handler/config.toml`. A configured `cargo-handler probe` carries the same edges with `hierarchy_failed` absent; every unresolved member is an idle Codex thread with no row, the expected exception.
- The live grid shows `· showrunner` titles, nested `· unit` rows and Codex worker roles, and only placement roots are summary entries.

**Files:**
- `docs/cargo-port/remove-running-section-handler-hierarchy.md` — this record

**Gotchas:** the cross-machine edge could only be checked by code path, since the Mac runs no managed agents.

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

### Phase 4 — Role labels in the title  · status: done

#### As-built

- Placement records each accepted edge's `HierarchyRole`, and `AgentPlacement::role_label` returns a `RoleLabel`: unlabeled, stated by an accepted edge, or inferred (`showrunner` for a manager of a unit or showrunner, `unit` for a manager of workers). An explicit edge role wins over inference; a dropped, stale or unresolved edge labels nothing.
- Tile titles read `<name> · <role>` (name in its rainbow hue, ` · <role>` in the label color), and each nested Claude row or Codex thread tree line shows its role after its name; an unlabeled agent shows no suffix. Roles: showrunner, unit, impl, test, review, fix.
- `tui_pane` titles are styled segments: `TileCells::group_title` returns `Option<Line<'static>>` and `GridLines::add_titled` takes `impl Into<Line<'a>>`; each segment's style patches over the chrome title style, clipping keeps segment styles, and the ellipsis takes the last visible grapheme's style.
- `HierarchyAnswer`, `LastSuccessfulHierarchy` and `HierarchyRetry` keep the last good edges through a failed or timed-out run of an unchanged command, filtered to live members, and retry on the next scheduler tick; a source change or turning the command off clears them. Remote probes carry an optional `hierarchy_failed` field (absent means answered); `RemoteHierarchyAnswers` keeps a failing host's last good edges and discards a late answer for a removed host.
- `HIERARCHY_TIMEOUT` is 10 s; `PROBE_TIMEOUT` is `HIERARCHY_TIMEOUT + PROBE_SCAN_SSH_MARGIN` (20 s). Completed census rows publish before hierarchy discovery finishes.

**Files:**
- `crates/cargo-handler/src/census/placement.rs` — accepted-edge roles, `RoleLabel`, inference
- `crates/cargo-handler/src/census/hierarchy.rs` — discovery cache with last-good retention and next-tick retry
- `crates/cargo-handler/src/census/probe.rs` — `hierarchy_failed` wire field and answer parsing
- `crates/cargo-handler/src/census/schedule.rs` — rows first, then discovery; remote answers per host
- `crates/cargo-handler/src/constants.rs` — `HIERARCHY_TIMEOUT`, `PROBE_TIMEOUT`
- `crates/cargo-handler/src/agent_cell.rs`, `render.rs` — labels in tree lines and titles
- `crates/tui_pane/src/pane/frame.rs`, `tiles/draw.rs`, `CHANGELOG.md` — styled title API

**Binds later work:** the live check of natedev's hierarchy command on both machines relies on a 10 s `HIERARCHY_TIMEOUT`, the derived `PROBE_TIMEOUT`, last-good-edge retention after a failed run, `hierarchy_failed` absent on a good probe, and role labels visible in titles and tree lines.

**Gotchas:** a capture with a scratch config needs an absolute `XDG_CONFIG_HOME` on Linux (a relative one silently falls back to `~/.config`); macOS ignores `XDG_CONFIG_HOME`. Removing and re-adding a host within one probe's 20 s life can let the old probe's answer apply; this race is accepted.

**Ruled out:** a user-facing indicator for a failed discovery run, since it gives the user nothing to act on.
