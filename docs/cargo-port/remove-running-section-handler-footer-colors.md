# handler-footer-colors

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler colors roles and accounts with the colors the Claude Code footer uses, read from the footer's own color file, and the summary's machine headings count "top-level agents". Later phases sort any tile's tree or the summary by a clicked column header, make clicks land on what is drawn while tiles move, and teach the census two Claude cases.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, the user (via the showrunner): "we now use role colors in our clauide code footers - we should use the same role colors for the role columns in handler - and in the summary it should say "8 top level 8agents" not "8 agents"", then "similarly we have colored accounts in the claude code footer - we should use the same colors for the accounts taht we show in handler".

Showrunner rulings: read `~/.claude/config/statusline.conf` so there is one source, never copy its values into cargo-handler; map `showrunner`→`role.showrunner`, `unit`→`role.unit-director`, `impl`/`test`/`review`/`fix`→`role.worker`; `shell` and `subagent` keep plain role text; a missing file means plain text. The count reads "8 top-level agents". Color the account column and the summary's account text from any `account.<label>` line matching the shown label; a label with no line stays plain, and adding a line later colors it with no code change. Use the footer's color names exactly as `~/.claude/scripts/statusline/statusline.jq` renders them. One phase, with a Change-Review line; the checkpoint notice carries a shot of the hana tile and the summary.

Unit director's calls: `session` (an agent with no role) takes `role.unassigned`, the footer's color for a session with no role; `thread` stays plain like the other work lines. The tile header line's agent label takes the account color too, so an account reads the same color everywhere it is shown. The file is read again when its modification time changes, so an edit shows without a restart.

## Delegation Context

- **Project:** `cargo-handler` — the terminal agent watcher; one tile per top-level agent, plus a summary tile.
- **Project started:** 2026-10-16T13:40:00+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Footer color file:** `~/.claude/config/statusline.conf`, lines `key=value`, `#` starts a comment, blank lines ignored. Today: `role.showrunner=bright-magenta`, `role.unit-director=bright-blue`, `role.worker=bright-cyan`, `role.unassigned=purple`, `account.claude1=cyan`, `account.claude2=magenta`, `account.claude3=blue`, `account.claude4=orange`, plus `level.*`/`tokens.*`/`left.*` keys handler ignores. The footer's name→ANSI table (`statusline.jq`, `def color`): black 30, red 31, green 32, yellow 33, blue 34, magenta 35, cyan 36, white 37, bright-black 90, bright-red 91, bright-green 92, bright-yellow 93, bright-blue 94, bright-magenta 95, bright-cyan 96, bright-white 97, orange `38;5;208`, purple `38;5;141`, pink `38;5;211`, teal `38;5;30`, brown `38;5;137`; any other name renders plain.
- **Key files** (line refs are main 7eacc2e3):
  - `crates/cargo-handler/src/agent_cell.rs` — `RoleLabel::role_text` ≈179; `TreeLine::account` ≈333, `TreeLine::value` ≈350 (`TreeColumn::Role` ≈353, `TreeColumn::Account` ≈364); `draw_tree` ≈857 (non-status columns draw in `text`); `header_line` ≈972.
  - `crates/cargo-handler/src/census/placement.rs` — `RoleLabel` ≈71.
  - `crates/cargo-handler/src/census/mod.rs` — `AgentRow::agent_label` ≈328.
  - `crates/cargo-handler/src/summary/mod.rs` — `heading` ≈220, `count_note` ≈250, `agent_row` ≈259.
  - `crates/cargo-handler/src/constants.rs` — `AGENT_SINGULAR`/`AGENT_PLURAL` ≈188–190.
  - `crates/cargo-handler/src/theme.rs` — `Role` ≈58 (theme roles; footer colors are not theme roles).
  - `crates/cargo-handler/src/app.rs`, `render.rs` — where per-frame state reaches `agent_cell::draw` and `summary::draw`.
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Invariants:**
  - Workers run only the Test/Lint lines above; each `verify.sh` run detaches and its result arrives as the worker's next message. Formatting only with `cargo +nightly fmt`.
  - No color value from the footer file is written into cargo-handler; tests use fixture text.
  - Invalid states are unrepresentable: a footer color is an enum parsed from its name, never a raw string carried to draw time.
  - No test takes a second.

## Phases

<!-- PHASE NUMBERING — binds every command that edits this doc.
     N is a bare integer, numbered from 1, contiguous, in execution order.
     Never a letter suffix (`4a`, `2b1a`) — those stop sorting and make ranges
     unreadable. Any spine edit (insert, split, merge, reorder, delete)
     resequences from the edit point to the end AND updates every cross-reference:
     substitute highest-first so `Phase 8` does not corrupt `Phase 12`, and
     re-check that each `Phases X–Y` range still spans the same set.
     Insert after the last `done` phase where possible — a done phase's number
     is baked into its checkpoint commit message and cannot be rewritten; if one
     must be renumbered, add an old→new mapping note to the doc.
     Full procedure: /plan:to_phased_plan → <PhaseNumbering/>. -->

### Phase 1 — Footer colors and top-level count  · status: done

#### As-built

- `footer_colors/mod.rs` parses `$HOME/.claude/config/statusline.conf` by statusline.jq's rules into `FooterColors { roles, accounts }`. `FooterColor` has one variant per footer name, `FromStr` from exactly those names, and `From<FooterColor> for ratatui::style::Color`: basic names → SGR 30–37 colors, `bright-*` → SGR 90–97, `orange`/`purple`/`pink`/`teal`/`brown` → `Indexed(208/141/211/30/137)`. `FooterColors::role(FooterRole)` and `account(&str)` answer `FooterTint::{Footer(FooterColor), Plain}`; `Plain` keeps the theme style, and an unknown name or key leaves its entry uncolored.
- `FooterColorFile` holds the parsed colors with the file's revision (`Unavailable` / `Modified(SystemTime)`) and re-reads only when the modification time changes. The terminal checks it once per local scan, on `CensusUpdate::LocalScanCompleted`, which only `LocalAnswers::send` sends; drawing never opens the file.
- Role column: `showrunner` → `Showrunner`, `unit` → `UnitDirector`, `impl`/`test`/`review`/`fix` → `Worker`, `session` → `Unassigned`, in one exhaustive `match`; `subagent` and `thread` draw in `text`; a shell line's role is `—`. Account labels in the tree's `Account` column, `header_line` and the summary's `agent_row` take `FooterColors::account`; the tier mark and program names keep their styles.
- Machine headings read `1 top-level agent` / `<n> top-level agents` (`AGENT_SINGULAR`/`AGENT_PLURAL`); `no agents` is unchanged.
- `census/worker_dispatch` parses an `implement.sh` shell command at scan time with a typed lexer (bash and zsh quote escapes, ANSI-C strings, env prefixes; operators rejected) into `WorkerDispatch`, resolved through `<session dir>/mesh_roster.json` (`--to` seat, else the entry whose `launcher_pid` is the dispatch's pid) into `ResolvedWorkerDispatch`. When the seat's thread is in the same tile, the shell draws no row and the thread's row reads `<seat short name> · <job text>`; otherwise the shell row stays.
- Codex thread rows (managed workers and `thread` lines) carry `ThreadState::{Busy, Idle}` (`census/mod.rs`), set at scan time from the rollout's last `task_started` / `task_complete` / `turn_aborted` event; no event is idle. It draws with the Claude busy/idle words and colors. The tier mark (` fast`) follows the account label in the tree's `Account` column.

**Files:**
- `crates/cargo-handler/src/footer_colors/mod.rs` — palette parse, `FooterTint`, `FooterColorFile` revision; tests in `footer_colors/tests.rs`
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — dispatch command lexer, roster resolution and fold; tests in `worker_dispatch/tests.rs`
- `crates/cargo-handler/src/census/mod.rs` — `ThreadState`, `CensusUpdate::LocalScanCompleted`; `census/scan.rs`, `schedule.rs`, `tier.rs`, `tree.rs`, `placement.rs` — scan-time state, fold join and refresh signal; tests in `census/thread_state_tests.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs` — colored role and account draw; tests in `agent_cell/tests/footer_color_tests.rs`, `summary/footer_tests.rs`
- `crates/cargo-handler/src/terminal.rs` — color refresh on `LocalScanCompleted`
- `crates/cargo-handler/src/constants.rs` — heading words, `SHELL_QUOTE_ESCAPES`

**Binds later work:** a folded dispatch is one row, the seat's thread row; its role cell still reads `thread` until the role comes from the seat (`mesh_roster` role, else the name stem).

**Gotchas:**
- Claude's zsh wrapper quotes `'` as `'"'"'`, not bash's `'\''`; `SHELL_QUOTE_ESCAPES` holds both.
- Dispatch labels collapse whitespace, so a multi-line job reads on one line.

**Ruled out:** a role word for shell dispatch rows — the user chose `—`.

### Phase 2 — Sort by any column  · status: done

**Depends on:** Phase 1

#### As-built

- Clicking a column header in any tree tile or in the summary sorts by that column; clicking it again reverses. Another column starts in its natural direction (Age descending, the rest ascending). The default Age ▼ draws the earlier order exactly, and only the sorted header shows ` ▲`/` ▼`, counted in the column widths.
- `sort.rs` holds the generic `ColumnSort<C>` / `SortColumn` with `SortDirection`; `TreeSort` (over `TreeColumn`) and `SummarySort` (over `SummaryColumn`) are aliases. `App::tree_sorts: HashMap<AgentCell, TreeSort>` keeps one sort per tile (no entry means the default), pruned by `prune_tree_sorts` after each completed scan; one `SummarySort` lives on the app.
- Tree sorting reorders siblings only, so the hierarchy, rails and `Branch::Last` follow; equal keys fall back to `(started, machine, pid)`. Hues are assigned in default order before sorting, so no sort recolors anything. The summary sorts within each machine group; its Agent column sorts by the drawn `agent_label(account_column)`.
- tui_pane `TileGridHost` has a provided `CellClick` / `CellPoint` content hook (`cell_content_at`) that receives cell-local positions inside a cell's content rect, computed by a crate-private settled-geometry helper in `grid.rs`; border presses call nothing. `render::cell_clicked` maps header clicks; the attract screen ignores grid clicks.
- Every Codex worker's role comes from one resolver: hierarchy edge role, else the seat token from the session's own `mesh_roster.json` with trailing digits stripped, else `worker`. The word `thread` is gone from the UI, and every Codex worker takes its session's hue. Older probe payloads infer roles at deserialization.

**Files:**
- `crates/cargo-handler/src/sort.rs` — column sort state and direction
- `crates/cargo-handler/src/agent_cell.rs` — sorted hierarchy, shared tree header layout and hit-test, typed `LineKind` presentation
- `crates/cargo-handler/src/render.rs` — cell construction with sorts, header click routing
- `crates/cargo-handler/src/app.rs`, `terminal.rs`, `tiles.rs` — per-tile sort state and pruning
- `crates/cargo-handler/src/summary/mod.rs` — summary sort; machine table rectangles shared by draw and hit-test
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — `WorkerRoleInput`/`WorkerRoleSource`/`DispatchRole` role resolution
- `crates/cargo-handler/src/census/thread_state.rs` — `ThreadState`, moved out of `census/mod.rs`
- `crates/tui_pane/src/tiles/{grid,host,mod}.rs`, `lib.rs` — content click hook

**Binds later work:** `ColumnSort<TreeColumn>`/`TreeSort` stored in `App::tree_sorts` keyed by `AgentCell` and pruned by `prune_tree_sorts`, so a new tile needs its own sort identity; header spans and arrows come from the shared tree header layout and hit-test in `agent_cell.rs`; an unroled top-level session's role text is decided by `LineKind` classification in `agent_cell.rs`; worker role order lives in `WorkerRoleInput`/`WorkerRoleSource`/`DispatchRole`, one roster read per scan; Status column comparators in `agent_cell.rs` and `summary/mod.rs` sort by the drawn text; `render::cell_clicked` rebuilds header spans from current census at click time; the tui_pane `cell_content_at` hook and the settled-geometry helper in `grid.rs` answer from settled layout, not last-drawn rects.

**Gotchas:** `git merge` refuses while new files are intent-to-add; commit them before merging main. The attract screen keeps retained tile geometry, so clicks consult the attract presentation.

**Ruled out:** scanning every `mesh_roster.json` on the machine.

### Phase 3 — Unassigned tile  · status: done

**Depends on:** Phase 2

#### As-built

- Every top-level session whose role label is `RoleLabel::Unlabeled` draws as one row of a single tile per machine titled `unassigned · <machine>` in the theme's `title_color()`; those sessions no longer get tiles of their own. A machine with no such session has no unassigned tile. The summary's rows, counts and name hues are unchanged.
- `tiles::TileKey { Agent(AgentCell), Unassigned { machine } }` is the grid's tile identity (`TileGrid`, `TileContent`, `TileDemands`, `TileGridHost::TileId`) and the key of `App::tree_sorts: HashMap<TileKey, TreeSort>`. The pool's sort lives under `Unassigned { machine }`, so it survives sessions joining or leaving, never crosses machines, and is dropped by `prune_tree_sorts` through the key-only `agent_cell::tile_keys(&Placement) -> HashSet<TileKey>` once the machine's pool disappears.
- `agent_cell::group_tiles` builds `AgentTile::Agent` / `AgentTile::Unassigned(UnassignedEntry)` after hues are assigned, so each pooled session keeps the hue it would have had as its own tile. Pooled roots draw with `Branch::Flat` (no glyph, rail ended); their shells, subagents and Codex workers nest below with rails as in other tiles.
- A pooled root is `LineSubject::UnassignedSession` / `LineKind::UnassignedSession`; its role cell draws `—` in the `FooterRole::Unassigned` color.
- The pool uses the same header row, columns, sort and header clicks as tree tiles. `SessionHeaderLead { OverBranch, Flush }` is threaded through header text, width measurement, header spans and draw: tree tiles use `OverBranch`, the pool uses `Flush` so its session header sits over its flush names. `CellView::tree_at_width` is the shared width fit for both.

**Files:**
- `crates/cargo-handler/src/tiles.rs` — `TileKey`
- `crates/cargo-handler/src/agent_cell.rs` — `group_tiles`, `tile_keys`, `UnassignedEntry`, flat pool rows, `SessionHeaderLead`, `unassigned_header_spans`, `unassigned_height`, `draw_unassigned`
- `crates/cargo-handler/src/render.rs` — `Cells` per `TileKey`, pool title, header click routing
- `crates/cargo-handler/src/app.rs`, `terminal.rs` — sorts keyed by `TileKey`, pruning by `tile_keys`
- `crates/cargo-handler/src/constants.rs` — `UNASSIGNED_CELL_TITLE`
- `crates/cargo-handler/src/agent_cell/tests/unassigned_tests.rs`, `crates/cargo-handler/src/summary/unassigned_summary_tests.rs` — grouping, flat rows, sort lifecycle, header alignment, unchanged summary

**Binds later work:** tile identity is `TileKey`, not `AgentCell`; drawn header spans are kept per `TileKey`, the pool included, and each tile's spans carry its `SessionHeaderLead`, since header layout differs between tree tiles and the pool. Status column changes reach the pool too, through the shared header and `CellView::tree_at_width`. A top-level session that should join the pool with its own role text needs its own `RoleLabel` variant and is grouped through `group_tiles`.

**Gotchas:** a header-layout signature change (e.g. `tree_widths`) breaks existing callers in `agent_cell/tests/`; whoever changes the signature migrates them. The `ONE_AGENT_CELL` golden in `render.rs` has a flush `│ session …` header line.

**Ruled out:** a sentinel pid for the pool's identity; recreating a pruned pool sort from a click on a vanished tile.

### Phase 4 — Worker rows say who and what  · status: done

**Depends on:** Phase 3

#### Work Order

**Goal:** no row is named by a raw thread id or UUID, and the Status column says what an agent is doing right now instead of plain `busy`.

**Source:** the user (via the showrunner), 2026-10-16: "how can we make it so i don't see guid names underneath units for things likef a fix or a impl or a test? the guids mean nothing to me  i want the status column to reflect if they are running the verify command - not that it's running verify but the actual verification step they are running - so rather than busy it could say "clippy", "fmt", "nextest" and if it's busy writing it could say "writing" and if it's busy reading it could say "reading" - and you can suggest other things." Showrunner design ruling, same day.

**Constraints from prior phases:**
- Phase 1's dispatch fold names a folded row `<seat short name> · <job>` from `<session dir>/mesh_roster.json`.
- Phase 2's role order for a Codex worker: hierarchy edge, then the seat's role token, then `worker` (`census/worker_dispatch/mod.rs`: `WorkerRoleInput`, `WorkerRoleSource`, `DispatchRole`). Keep that resolver and the session hue as they are; this phase adds only the name.
- Today the roster is read only when a live dispatch shell is found, `RosterRecord` carries no job, and `ResolvedWorkerDispatch.job` comes from the launcher's arguments. A worker whose launcher has exited therefore has no path to its seat or job yet; Spec 1 adds one.
- `AgentRow.status` is an `Option<String>` (`census/mod.rs` ≈292) and thread activity travels separately in `ThreadState` (`census/thread_state.rs`). Spec 2 replaces both with one semantic type, so a row cannot carry two contradicting activities.
- Status is read at scan time (`census/tier.rs` reads rollouts incrementally); drawing never opens a file.
- Phase 2's Status column sorts by the drawn text, in tiles and in the summary; the new words sort by the word drawn.

**Spec:**
1. Names. Never draw a raw thread id or UUID as a row name. (a) A thread that maps to a seat in its session's `mesh_roster.json` is named `<seat> · <job>`, the same form as folded rows, whether or not a dispatch shell is alive. (b) Otherwise `<role> · <job>`, where the job is the first line of the thread's first task prompt from its rollout. (c) Otherwise the role alone. Demo rows `01a142f0-…`, `01a1437d-…` and widget's `01a14488-…` must come out named.
   - Discovery without a launcher: a Codex thread finds its session dir from its own rollout, whose first task prompt names `/tmp/claude/unit/<id>/…` or `/tmp/claude/delegate/<id>/…` paths (the board log, the phase dir). That dir's `mesh_roster.json` lists the thread id under its seat. Read the rollout's first prompt once per thread and each roster once per scan, keyed on file mtime; never scan every roster on the machine (ruled out in phase 2). When the prompt names two session dirs, take the one whose roster lists the thread; when both do, the newer roster by mtime wins. The job comes from the same first prompt when no launcher argument is available. Check this against a live rollout before writing the parser and record the prompt's real shape in the summary.
   - Fixtures: a worker with its launcher gone, a worker with no readable first prompt (falls to rule (c)), and two session dirs naming the same thread.
2. Status words, in place of `busy`; `idle` stays. Model them as one semantic type replacing `AgentRow.status: Option<String>` and the separate `ThreadState` activity, e.g. `enum AgentActivity { Idle, Active(Activity), Unreported }`, with verify steps as their own `Activity` variant carrying the step enum and where it runs. Convert external optional fields (probe JSON from the Mac, rollout and transcript fields) to it at the probe and parse boundary (`census/probe.rs`); keep older probe payloads readable.
   - A verify step: attribute a run to its worker through `<session dir>/mesh_runs/*.json`, whose records carry `worker`, `thread_id`, `pid`, `command` and `status` (`running` while live) — a run detaches from the worker, so it is never found by process ancestry. A Claude session's own `verify.sh` is a descendant of its Bash. The step is the cargo (or `taplo`) child of that run's process tree: `fmt`, `clippy`, `nextest`, `build`, `check`, `doc`, `mend`, `taplo`. A step offloaded to the Mac runs through an `ssh` child of the run's tree and shows ` (mac)`: `nextest (mac)`. A live run with neither a step child nor an `ssh` child shows `waiting` (build hold, cargo token or memory wait). A run record whose status is no longer `running`, or whose pid is gone, contributes nothing.
   - `writing`: the last tool call is a file edit (Codex `apply_patch`; Claude `Edit`, `Write`, `NotebookEdit`).
   - `reading`: the last tool call is a read or search (Claude `Read`, `Grep`, `Glob`; Codex a shell running `rg`, `grep`, `cat`, `sed -n`, `ls` or `find`).
   - `git`: a shell running `git commit`, `merge`, `rebase` or `push`.
   - `delegating`: a Claude `Agent` or `Task` call. `web`: `WebFetch` or `WebSearch`.
   - `thinking`: in a turn with no tool call yet, or the last event is reasoning.
   - `running`: any other shell command.
   - `asking`: waiting on a permission prompt, only if Claude's session file exposes it; otherwise leave it out and say so in the summary.
   - Precedence, one word per row: a live verify run beats the tool and turn words (a worker blocked on its gate shows the gate, even after its turn moved on); among several live runs for one worker (`mesh_runs.py` allows them) the most recently started record wins, ties broken by the higher pid; with no live run, the last tool call's word; with none, `thinking` inside an open turn; `idle` when the turn is closed. When the selected run finishes while another is still live, the other run's word shows next.
3. Colors: every active word takes busy's color; verify steps share one accent so they read as gates; `waiting` takes the idle dim.
4. Scope: Claude sessions (from the transcript's last `tool_use`) and Codex workers (from the rollout's last `function_call`). Read once per scan, never per frame. Linux first; on the Mac use what discovery already supports.
5. Tier mark (showrunner add-on, 2026-10-16; the user asked "what is the -- afer codex1 entries?"). The standard tier draws no mark and only the fast tier draws `fast`: an account or agent cell reads `codex1` or `codex`, never `codex1 --`. `ServiceTier::mark` (`census/mod.rs` ≈122) returns the program alone for `Standard` as for `Unrecorded`; remove `STANDARD_TIER_LABEL` (`constants.rs` ≈244). Keep the `Standard`/`Unrecorded` variants: `or_else` still needs the difference. No string check on the drawn text.
6. Dispatch fold misses: change-review C15, returned by the user ("i'm not sure it's doing what you said"). Showrunner's live findings, about 17:00 2026-10-16:
   - Folds correctly: hana's cables, lfo, startup and trunk-select, all `implement.sh --to <seat>` → `codex_mesh.py follow --to <seat>`.
   - Misses, fresh seat: `implement.sh` with no `--to` → `codex_mesh.py start --name <seat>`: trunk's `Dispatch the portal fix to a fresh seat` (`start --name tool-based-ui-trunk-portal`), widget's merge-resolution seat (`start --name widget-enhancements-merge`), unit-board phase 1 (`start --name claude-build-followups-unit-board-…`). The worker draws as a GUID and the dispatch stays a separate command row.
   - Misses even with `--to`/`follow`: berth-parallel phases 9 and 10 (`--to cargo-liner-berth-parallel-impl` and `-impl2`, session dir `/tmp/claude/unit/7393c2a2…`), style (`--to claude-build-followups-style-impl`, `/tmp/claude/unit/24e20a41…`), two-accounts (`--to claude-build-followups-two-accounts-test`, `/tmp/claude/delegate/8b4351c3…`). The dispatch rows stay separate; style's worker draws as a GUID; two-accounts' workers draw bare seat names with no `· job`.
   Find, against live `ps` output and those sessions' rosters and run markers, what separates the misses from the hits (run markers, the `unit` versus `delegate` session dir form, the `start` versus `follow` path, or the roster entry written after the launcher starts). Fold every `implement.sh` dispatch form into its worker row and name every worker `<seat> · <job>`, never a GUID. Record each form and its cause in the summary; each form above gets a fixture test.
7. Tests (test slot): name rules (a)–(c) from fixture rosters and rollouts, including the three discovery fixtures; each status word from fixture transcript and rollout tails and fixture process trees and run records; a detached verify run attributed through its record; a Mac step `(mac)`; `waiting`; a finished run clearing back to the next word; a turn completing, aborting and starting again; an older probe payload still read; Status sorting by the drawn word in a tile and in the summary; colors read back from the buffer in both, including a pooled root and its nested worker in the unassigned tile; the precedence rules, including two live runs for one worker where the selected one finishes; a standard-tier account cell drawn without a mark and a fast one with `fast`; each dispatch form Spec 6 found, folded into its worker row.

**Files:**
- `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/census/tier.rs`, `crates/cargo-handler/src/census/thread_state.rs`, `crates/cargo-handler/src/census/thread_state_tests.rs`, `crates/cargo-handler/src/census/transcript.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/scan/`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/worker_dispatch/`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/terminal.rs`
- `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — all production files, including `worker_dispatch/mod.rs`, `classify.rs` (copies `SessionRecord.status` into `AgentRow` ≈328) and the probe conversion; edits to existing tests and fixtures that build the replaced fields (`render.rs`, `terminal.rs`, existing `agent_cell/tests/` files); and the `mod` lines wiring new tests.
- `test` — new discovery, activity, run-record, probe-compatibility and rendering fixtures and test files.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live shot shows a named worker row, a status word other than `busy`, and a fresh-seat dispatch folded into its worker row. The checkpoint carries `Change-Review: worker rows are named by seat or role and job, never a thread id | look under a unit for its impl, test and fix rows` and `Change-Review: the status column says what an agent is doing: a verify step, writing, reading, thinking, git and the like | watch a worker's status while it edits and runs verify`, `Change-Review: account and agent cells show no mark for the standard tier, only fast | look at a codex1 row's account cell` and `Change-Review: every implement.sh dispatch, fresh seat or follow-up, folds into its worker row named seat · job (C15) | look under the hana, widget, style and two-accounts units for Dispatch rows beside raw-id workers`.

### Phase 5 — Accounts table in the summary  · status: todo

**Depends on:** Phase 4


#### Work Order

**Goal:** the summary cell shows, below its top-level agents table, an `accounts` table with each account's role and remaining weekly usage, updating as tokens are used.

**Source:** the user (via the showrunner), 2026-10-16: "cargo handler should reprsent the output of /showrunner:accounts - the summary cell could maybe be subdivided to show a table of top level agents followed by a table of accounts (that updates itself as tokens are used)". Showrunner design and packaging ruling, same day (below).

**Constraints from prior phases:**
- Phase 1's account footer colors (`footer_colors/`) color account labels; reuse them.
- Phase 2's summary header style, gap rule and `ColumnSort` header layout; the agents table stays as it is.
- The configured-command pattern of `[census] hierarchy_command` (run off the frame thread, JSON on stdout) is the model for `accounts_command`.

**Spec:**
1. The summary cell holds two stacked tables under its one title: the agents table as today, then an `accounts` table with the same header style and gap rule.
2. Columns: `account` (label such as `claude1`, `codex2`, in its footer color); `role` (the JSON's `roles` list joined with `, ` — one account can hold both groups; an unselected Claude account has `[]`, a Codex account `["workers"]` or `["spare"]`; `—` when empty); `weekly left` (percent, red at 10% or less); `resets` (the JSON's ISO 8601 UTC time drawn short in the local zone, e.g. `Mon 02:00`); `credits` (a number, `unlimited`, or JSON `null` drawn `—`, as for Claude; dimmed when `use_credits` is false, as it always is for Claude). No folder, login, on-credits or read columns, so no email address is drawn; a row whose reading is over an hour old is dimmed and shows its age.
3. Rows follow accounts.py's own order and signed-in rule; its problem lines draw dimmed under the table.
4. One source of truth: a configured `[census] accounts_command` printing accounts.py's own JSON; never re-derive the role map or the usage readings in Rust. Parse the JSON into typed rows at the boundary (`roles` into a list of a role enum, `credits` into an enum of amount, unlimited and none). The command (promoted to `~/.claude` on both machines at 49fde9b3) is `~/.claude/scripts/lib/py ~/.claude/scripts/accounts/accounts.py status --json [--stored]`; set `accounts_command` to the `--stored` form (no live reads, no subprocess, no ssh; 0.21 s on the Mac). Output: `{"accounts": [...], "problems": [string]}`, accounts in the table's order, `problems` the table's note lines. Keys per account: `label` (string); `tool` (`"claude"`|`"codex"`); `roles` (list: Claude `"units"`/`"showrunners"` or `[]`; Codex `["workers"]` or `["spare"]`); `weekly_left_percent` (number|null); `resets_at` (ISO 8601 UTC|null); `credits` (number|`"unlimited"`|null, null for Claude); `use_credits` (bool, false for Claude); `read_at` (ISO 8601 UTC|null). No email, folder or path. With no cached role map, `--stored` prints empty roles, puts the problem in `problems` and exits 0; plain `status --json` exits 1 with the reason on stderr when no role map can be read.
5. Optional (showrunner ruling): with no `accounts_command` configured the summary shows only the agents table; when the command fails it also shows one dimmed line saying why. cargo-handler works fully without it. Refresh by running the command off the frame thread every 60 s; that run is the only change detection, and a changed reading shows on the next frame after it returns. Never block a frame. Model what the summary has as one state, e.g. `enum AccountsTable { NotConfigured, AwaitingFirstReading, Ready(AccountsReading), Failed(AccountsFailure), FailedKeepingLast { last: AccountsReading, failure: AccountsFailure } }`, never optional rows beside an optional error: before the first result the summary shows the agents table only; a failure after a good reading keeps the last rows (stale ones dimmed with their age, as in Spec 2) and adds the one dimmed why-line. Set the command in both machines' configs.
6. `summary::height` and `summary::width` (`summary/mod.rs` ≈66, ≈83) measure only machine groups today and `render.rs` ≈264 hands them to the grid as the cell's demand: include the accounts heading, rows, problem lines and the why-line in both. A click on the accounts table's header changes no sort: neither the agents table's nor accounts.py's row order.
7. Tests: a real grid-buffer read-back of both tables at their measured demand, column colors (red at ≤10%, dimmed credits, dimmed stale row with its age), `—` cells, problem lines, row order from the JSON, a refresh through the background scheduler that changes a reading, a command that fails or prints bad JSON leaving the agents table intact, every `AccountsTable` state drawn (including a failure after a good reading keeping the last rows), and an accounts-header click leaving both orders unchanged.

**Files:**
- `crates/cargo-handler/src/summary/`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/terminal.rs`, `crates/cargo-handler/src/config.rs`, `crates/cargo-handler/src/census/`
- `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files, edits to existing tests, `mod` lines.
- `test` — new accounts-table and refresh test files.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live shot shows the accounts table under the agents table. The checkpoint carries `Change-Review: the summary shows an accounts table that updates as usage changes | watch weekly left fall on a busy account`.

### Phase 6 — Accurate clicks during motion  · status: todo

**Depends on:** Phase 5

**Blocked by:** tile-fixes phase 20 merged on main — the showrunner says when.

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2763`) while drawing interpolates (`drawing_at`, `grid.rs` ≈2880), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps.

**Constraints from prior phases:**
- Phase 2's cell-local click hook (`cell_content_at`, `tiles/host.rs`) resolves content from current slots and the destination layout through a crate-private settled-geometry helper in `tiles/grid.rs`. This phase moves it to the last-drawn record too, so the header hit-test matches what is on screen.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Phase 3's tile identity is `tiles::TileKey::{Agent(AgentCell), Unassigned { machine }}` (sorts in `App::tree_sorts: HashMap<TileKey, TreeSort>`, pruned by `agent_cell::tile_keys`), and header layout carries `agent_cell::SessionHeaderLead::{OverBranch, Flush}` (the pool's session header sits over flush names). Drawn header spans are kept per `TileKey`, with the lead they were drawn with (`render.rs` ≈229 `Cells::tile`/`header_spans`, `agent_cell.rs` ≈1384).
- Before writing the Work Order's prompt, refresh the `grid.rs`/`draw.rs` line refs above against main after tile-fixes phase 20 merges.

**Spec:**
1. `draw_tile_grid` (`tiles/draw.rs`) records, for each drawn piece, the cell's stable content identity, its drawn rect, and its visible (clipped) hit region, in the grid's own state, replacing the previous frame's record. Where pieces overlap, the one drawn on top wins. Model the record's state as a type that tells "never drawn" apart from "a frame drew no interactive content" (e.g. `enum DrawnHits { NotDrawn, Drawn(Vec<DrawnPiece>) }`), never a bare `Option`.
2. `TileGrid::cell_at(&self, Position) -> Option<usize>` keeps its public signature, as the compatibility boundary, and answers from that record. Before the first draw it answers from the layout as today.
3. Phase 2's content lookup in `tiles/host.rs` uses the same record, and resolves the content by the identity recorded at draw time, not by the slot's current occupant.
4. cargo-handler keeps the header spans it drew with each tile and the summary, and `render::cell_clicked` maps a header click through those drawn spans, not freshly rebuilt ones. A click on a drawn tile that a later scan removed (its sort already pruned) records no preference, so it cannot bring a pruned pool sort back. If `Cells::tile` (`render.rs` ≈224) stays, it returns a semantic lookup result rather than an `Option`, converting to `Option` only at the framework boundary.
5. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell; a press on the destination rect of a still-moving cell returns what is drawn there; a press on a piece's clipped-away part misses it; a press after a frame that drew nothing interactive returns no cell. In cargo-handler, a census change between the draw and the click that would otherwise shift a header column still selects the column that was drawn under the pointer, for a tree tile and the unassigned tile, in Full and Compressed views; a click on a drawn tile that has since disappeared records no sort. cargo-tile's suites pass.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/tests/`
- `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 2 writers.
- `grid` — tui_pane: the drawn-hit record in `draw.rs`/`grid.rs`/`host.rs`, its motion tests and `host_tests.rs`, tui_pane CHANGELOG.
- `app` — cargo-handler: drawn header spans kept with the frame, `render::cell_clicked`, their tests, cargo-handler CHANGELOG.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.

### Phase 7 — Reviewers and Claude dispatch seats  · status: todo

**Depends on:** Phase 4

#### Work Order

**Goal:** a pool reviewer shows under its unit while it runs a review, an unnamed Claude plan-mode reviewer shows role `review`, and a dispatch to a Claude-backed seat folds into its row like a Codex seat does.

**Source:** showrunner-approved add-ons, 2026-10-16, placed after the clicks phase. Pool reviewers moved here from phase 4 on 2026-10-16 so all reviewer rows land together; the producer's reviewer edges (enh-showrunner checkpoint 3b2d8848 in `~/.claude`) must be merged and installed before the live check.

**Constraints from prior phases:**
- Dispatch resolution carries a Codex `thread_id` and folds the launcher shell's children into the thread's children; it cannot name a Claude session row today. `ParsedDispatch` drops the launcher's `--phase` argument.
- The `["claude", "--print", "review"]` value in `census/tree.rs` is test fixture data, not a production rule. Today an unlisted Claude descendant can be treated as a carrier process rather than a visible row.

**Spec:**
1. A `claude --print --permission-mode plan` process with no other role mapping shows role `review`. Trace the real discovery-to-row path (`census/classify.rs`, `census/tree.rs`, `census/placement.rs`) and make such a process a visible row; the `review` fallback is typed and never overrides a role that already comes from the hierarchy edge or a roster. Phase 3 groups top-level `RoleLabel::Unlabeled` roots into the unassigned tile and draws their role `—` (`agent_cell.rs` ≈460, ≈758): a top-level reviewer with no production role still joins that tile, but draws `review`, so the fallback is its own `RoleLabel` variant (or carried beside it) that grouping places with unlabeled roots and the role cell names.
2. The dispatch fold resolves a Claude-backed seat through `<session dir>/phases/<id>/impl_worker_<slot>.json` (family `claude` → `worker_id` is the session id), joining the dispatch to that Claude session's row the way phase 1 joins a Codex seat through `mesh_roster.json`. Model the joined worker as a family-aware identity, e.g. `enum WorkerIdentity { CodexThread(ThreadId), ClaudeSession(SessionId) }`, so identical id text from the two families never matches. `ParsedDispatch` keeps `--phase` so the fold reads the current phase's file for the slot; a file from an earlier phase is ignored. Read the file's real shape from a live run (this session dir has `impl_worker_impl.json`) before writing the parser, and record it in the summary.
3. Pool reviewers (showrunner add-on, 2026-10-16, from the user's comparison: hana has 21 permanent pool reviewers and its tile shows none; revised the same day; the user: "i don't need to see them if they are not active - but if they are ruynning i expect to see them"). A pool reviewer is a leased Codex thread stored on disk, resumed for each review; `busy_pid` is the process running that review. cargo-handler shows running agents, so a stored thread is not a row. Source: `~/.local/state/review-pool/<production slug>/pool.json`, `reviewers.<name>` with `type`, `thread_id`, `lease.unit`, `busy_pid`; edges run from the leased unit to the reviewer with role `review`, emitted by the producer (`production_tree.py`, natedev's ruling 2026-10-16; enh-showrunner checkpoint 3b2d8848). `production_tree.py --json` (schema 1) adds an edge per leased reviewer: `manager` (director's Claude session id), `member` = `thread_id` (reviewer thread), `role` `review`, `unit` (lease unit), `state` `busy` while busy_pid runs else `idle`, `type` `code`|`design`; templates, unleased reviewers and directors on another machine are left out. Draw only `state: busy` edges. The user, later the same day: "i don't need to see pool reviewers that are idle … we should just only be showing active ssessions and certainly flag any that happens to have a reviewr role AS a reviewer when it's reviewing": the tile adds no row for a reviewer that is not running, and a running session that holds a reviewer lease draws role `review`, whichever row it would otherwise have. A unit can hold several reviewers (the pool forks one when all are busy); each busy one is its own row.
   - A reviewer draws only while `busy_pid` names a live process: a `review` row under its unit, named `<reviewer> · <job>` by Spec 1, with its Spec 2 status word. A leased reviewer that is not busy, and every `templates` entry, draws nothing.
4. Tests, with fixture files: a reviewer started with `--print --permission-mode plan` reaches the drawn Role column as `review` (rendered through grouped tiles, `agent_cell::group_tiles`, not only `cell_order_placed`, including a top-level reviewer inside the unassigned tile); a hierarchy-given role still wins; a Claude-seat dispatch folds into its session's row; a stale earlier-phase identity file is ignored; a Codex thread id and a Claude session id with the same text stay apart; from a fixture `production_tree.py --json` payload, a busy reviewer edge drawn as a `review` row under its unit, and an idle edge drawn as nothing.

**Files:**
- `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/probe.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files, migrations of existing tests outside the tester's files, `mod` lines.
- `test` — `worker_dispatch/tests.rs` and new fixtures.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live tile shows a plan-mode reviewer as `review` and a busy pool reviewer under its unit. The checkpoint carries `Change-Review: Claude plan-mode reviewers show role review; dispatches to Claude seats fold into their row | open a tile running a Claude reviewer or a Claude-seat dispatch` and `Change-Review: a pool reviewer shows under its unit as a review row while it runs a review, and not otherwise | look under a hana unit while its code review runs`.
