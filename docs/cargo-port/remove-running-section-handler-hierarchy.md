# handler-hierarchy

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler nests each agent under the agent that manages it, as a command on each machine reports, shows only top-level agents in the summary, and labels each managed agent's role in its title.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15, the user (via the showrunner): "yes i want handler-enh to go ahead - i want the labels in the title - i want only top level showrunners to show in the summary". The showrunner's reading: role labels are in, and a showrunner that reports to another appears only inside its parent's tile, never as a summary entry of its own. natedev accepted the command contract below and builds the command.

## Contract — `hierarchy_command`

- Setting `[census] hierarchy_command` in cargo-handler's config; empty means off. Each machine's census runs its own command; natedev's is `~/.claude/scripts/lib/py ~/.claude/scripts/production/production_tree.py --json`.
- stdout, exit 0: `{"schema":1,"edges":[{"manager":"<sessionId>","member":"<sessionId>","role":"showrunner|unit|impl|test|review|fix"}]}`. `manager` and `member` are Claude Code `sessionId` values from the session records. `role` is the member's role.
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
2. When natedev reports it landed, the unit director runs it on natedev and on the Mac, checks the output against the contract (ids resolve to live session records; roles are from the set), and records the result here.

**Files:**
- `docs/cargo-port/remove-running-section-handler-hierarchy.md`

**Workers:** none — natedev builds the command; the unit director checks it.

**Acceptance gate:** the command's output on both machines matches the contract and names the live showrunner → unit links.

### Phase 2 — Each machine's census reads its hierarchy edges  · status: done

**Depends on:** none

#### Work Order

**Goal:** every census answer, local and remote, carries each Claude row's session id and the machine's hierarchy edges.

**Spec:**
1. Give each Claude row its session id, set in `claude_row` from its `SessionRecord`; Codex and desktop-app rows have none. Use a domain type (for example `SessionIdentity`), not a bare `Option<String>`.
2. Add `[census] hierarchy_command` (string, default empty) to `Config`, and a settings row that edits it like `RemoteMachines`. Share it with the scheduler the same way `RemoteMachines` reaches `schedule::spawn`, read each round.
3. On the scheduler thread, run the command (split as a shell would, or via `sh -c`) with a 1 s timeout and stdin and stderr closed, at most every 30 s, and again at once when the set of local session ids changes. Cache the last good edges; a failure keeps none and logs one line to the startup log when it is on.
4. Parse the contract into `HierarchyEdge { manager, member, role }` with `role` an enum (`Showrunner`, `Unit`, `Impl`, `Test`, `Review`, `Fix`); an unknown role or schema drops that edge or answer.
5. Carry both on the wire: `session_id` per row and `edges` on `ProbeReport`, both optional, `PROBE_SCHEMA` unchanged. Carry each machine's edges on `MachineState::Answered` beside its rows; read access opens in phase 3 with its first consumer (no API without a consumer).
6. Tests: a fixture command (a script under the test's temp dir) printing edges reaches the census answer; a failing, slow and malformed command each yield no edges; an old probe without the fields parses; a probe with them round-trips.

**Files:**
- `crates/cargo-handler/src/census/classify.rs`
- `crates/cargo-handler/src/census/mod.rs`
- `crates/cargo-handler/src/census/scan.rs`
- `crates/cargo-handler/src/census/schedule.rs`
- `crates/cargo-handler/src/census/probe.rs`
- `crates/cargo-handler/src/census/hierarchy.rs`
- `crates/cargo-handler/src/config.rs`
- `crates/cargo-handler/src/settings.rs`
- `crates/cargo-handler/src/app.rs`
- `crates/cargo-handler/src/terminal.rs`
- `crates/cargo-handler/src/constants.rs`

**Workers:** 2 writers — census side and config side.
- `impl` — `census/classify.rs`, `census/mod.rs`, `census/scan.rs`, `census/schedule.rs`, `census/probe.rs`, `census/hierarchy.rs`, `constants.rs`; hub: `census/mod.rs` (the shared types)
- `test` opens as `impl` — `config.rs`, `settings.rs`, `app.rs`, `terminal.rs`

**Acceptance gate:** the fixture tests pass; the cargo-handler suite and lint are green; a live run with a fixture command shows edges in a probe's JSON (`cargo-handler probe` or its equivalent).

### Phase 3 — Managed agents nest in their manager's tile, and only top-level agents are summary entries  · status: todo

**Depends on:** Phase 2 (`crates/cargo-handler/src/census/mod.rs`)

#### Work Order

**Goal:** an agent named as a member of a live manager on any shown machine sits in that manager's tile, below it in the tree, and is not a summary entry; with no edges the grid and summary are as today.

**Spec:**
1. Open read access to each machine's edges (`MachineState` accessor; the edge fields and `HierarchyRole` crate-visible), which phase 2 kept private for want of a consumer. Build one cross-machine placement before cells and summary: resolve each edge's `manager` and `member` session ids to rows across every machine. A member's parent is its edge's manager when that resolves, else its `launched_by` row on its own machine when listed, else none.
2. `cell_order` runs as one pass over every machine from that placement: a root is a row with no parent; sessions nest depth first, oldest first, as today. `SessionEntry` carries its machine, and hue and name lookups match on machine and pid. Keep `AgentCell`'s key and the hue order rules.
3. The summary lists exactly the rows that are cell roots, through the same predicate, replacing `launched_by.is_none()`. This also puts a row whose launcher is not listed back in the summary, matching its cell.
4. A member on another machine reads its machine in its tree line (the header already names machines).
5. Tests: unit under showrunner on one machine; unit on `mac` under a showrunner on `natedev`; showrunner under showrunner (the child is not a summary entry); an edge whose manager is not listed leaves the member top level; an edge overrides a launch match; no edges reproduces today's order.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs`
- `crates/cargo-handler/src/census/mod.rs`
- `crates/cargo-handler/src/summary/mod.rs`
- `crates/cargo-handler/src/render.rs`

**Workers:** 1 writer + 1 tester — placement and cells, then summary.
- `impl` — `agent_cell.rs`, `census/mod.rs`, `render.rs`; hub: `census/mod.rs`
- `test` opens as `impl` — `summary/mod.rs`

**Acceptance gate:** the tests pass; suite and lint green; live on both machines with the real or fixture command, a showrunner's tile holds its units, and the summary lists only top-level agents. Shots on the unit's port: summary and grid with a nested showrunner, unit and a cross-machine unit.

### Phase 4 — Role labels in the title  · status: todo

**Depends on:** Phase 3 (`crates/cargo-handler/src/agent_cell.rs`)

#### Work Order

**Goal:** each tile's title, and each nested member's tree line, names the agent's role.

**Spec:**
1. A member's role is its edge's role. A manager's role, when no edge names it, is `showrunner` if it manages a `unit` or `showrunner`, else `unit` if it manages workers.
2. `group_title` appends the role after the name, as `<name> · <role>`, in the label color; an agent with no role shows no label.
3. A nested member's tree line shows its role after its name the same way.
4. Tests render the title and a tree line and read the text back.

**Files:**
- `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/src/agent_cell.rs`

**Workers:** 1 writer — two small edits that share one rule.
- `impl` — `render.rs`, `agent_cell.rs`

**Acceptance gate:** tests pass; suite and lint green; shots on the unit's port show the labels on a showrunner tile and its nested units.
