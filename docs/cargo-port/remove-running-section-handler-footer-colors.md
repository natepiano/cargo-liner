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

### Phase 3 — Unassigned tile  · status: todo

**Depends on:** Phase 2

#### Work Order

**Goal:** every top-level session with no production role draws as one row of a single tile per machine titled `unassigned`, instead of each having its own tile.

**Source:** the user (via the showrunner), 2026-10-16: "unassigned top level should be grouped into their own cell as well … it can be called unassigned and it should have its own column names as all - they just aren't hierarchical so just a list of unassigned". Showrunner placement: its own phase right after the sort phase, ahead of the Claude add-ons.

**Constraints from prior phases:**
- Phase 2's per-tile sort: `sort.rs` `TreeSort` = `ColumnSort<TreeColumn>`, stored in `App::tree_sorts: HashMap<AgentCell, TreeSort>` (`app.rs` ≈110) and pruned per root by `App::prune_tree_sorts` (≈136) after a completed scan. Header spans, hit-test and arrows come from the shared tree header layout in `agent_cell.rs`; reuse it, never fork it.
- Phase 1's role colors: `RoleLabel::Unlabeled` draws role text `session` today (`census/placement.rs` ≈86) in the `Unassigned` footer color (the unroled purple). This phase changes the text to `—` for a top-level unassigned row through `agent_cell.rs`'s typed `LineKind` classification, not by string replacement.
- Summary name colors come from each session's own hue through `agent_cell::name_style`; grouping tiles must leave the summary's rows, counts and hues unchanged.

**Spec:**
1. One tile per machine titled `unassigned`, carrying the machine name the way the other tiles' titles carry it. It holds every top-level session whose role label is `RoleLabel::Unlabeled` (`census/placement.rs:72`); today's separate tiles for those sessions (ups, postmaster) collapse into it. Model the tile's identity as its own variant of the tile key (e.g. `enum TileKey { Agent(AgentCell), Unassigned { machine } }`), never a sentinel pid.
2. Same header row and columns as the tree tiles: session, role, status, account, age, and branch and directory in the full view. The phase 2 sort and header clicks work on it. Its sort is stored under the tile key's `Unassigned { machine }` identity: it survives sessions joining or leaving the tile, is pruned when the machine's tile disappears, and never crosses machines.
3. Rows are a flat list: no tree glyphs at the top level. Each session's own shells, subagents and Codex workers nest under it with rails, as in other tiles.
4. Role shows `—` in the unroled purple for each top-level row.
5. No unassigned session on a machine → no unassigned tile for it.
6. Hues: each session keeps the hue it would have had as its own tile; the tile title takes the theme's title style.
7. Tests (test slot, new files): sessions with no role land in one tile per machine and leave no tile of their own; rows draw flat with nested work under each; role cell `—`; no tile when none exist; header sort reorders the flat rows; the tile's sort survives a session joining and leaving, is pruned when the tile disappears, and two machines' tiles sort independently; the summary's rows, counts and name hues are unchanged by the grouping; read rows back from the buffer.

**Files:**
- `crates/cargo-handler/src/tiles.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/sort.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`
- `crates/cargo-handler/src/summary/`
- `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files, edits to existing tests (including `app.rs`'s inline sort-pruning tests), and the `mod` lines wiring new test files.
- `test` — new test files: grouping and flat rows under `agent_cell/tests/`, the sort lifecycle, and the unchanged-summary check under `summary/`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live shot shows the unassigned tile beside the hana tile. The checkpoint carries `Change-Review: sessions with no production role are listed together in one "unassigned" tile per machine | look for the unassigned tile beside the hana tile and sort it by a column`.

### Phase 4 — Worker rows say who and what  · status: todo

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
3. Colors: every active word takes busy's color; verify steps share one accent so they read as gates; `waiting` takes the idle dim.
4. Scope: Claude sessions (from the transcript's last `tool_use`) and Codex workers (from the rollout's last `function_call`). Read once per scan, never per frame. Linux first; on the Mac use what discovery already supports.
5. Tests (test slot): name rules (a)–(c) from fixture rosters and rollouts, including the three discovery fixtures; each status word from fixture transcript and rollout tails and fixture process trees and run records; a detached verify run attributed through its record; a Mac step `(mac)`; `waiting`; a finished run clearing back to the next word; a turn completing, aborting and starting again; an older probe payload still read; Status sorting by the drawn word in a tile and in the summary; colors read back from the buffer in both.

**Files:**
- `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/census/tier.rs`, `crates/cargo-handler/src/census/thread_state.rs`, `crates/cargo-handler/src/census/thread_state_tests.rs`, `crates/cargo-handler/src/census/transcript.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/scan/`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/worker_dispatch/`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`
- `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — all production files, including `worker_dispatch/mod.rs` and the probe conversion, edits to existing tests, and the `mod` lines wiring new tests.
- `test` — new discovery, activity, run-record, probe-compatibility and rendering fixtures and test files.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live shot shows a named worker row and a status word other than `busy`. The checkpoint carries `Change-Review: worker rows are named by seat or role and job, never a thread id | look under a unit for its impl, test and fix rows` and `Change-Review: the status column says what an agent is doing: a verify step, writing, reading, thinking, git and the like | watch a worker's status while it edits and runs verify`.

### Phase 5 — Accurate clicks during motion  · status: todo

**Depends on:** Phase 4

**Blocked by:** tile-fixes phase 20 merged on main — the showrunner says when.

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2763`) while drawing interpolates (`drawing_at`, `grid.rs` ≈2880), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps.

**Constraints from prior phases:**
- Phase 2's cell-local click hook (`cell_content_at`, `tiles/host.rs`) resolves content from current slots and the destination layout through a crate-private settled-geometry helper in `tiles/grid.rs`. This phase moves it to the last-drawn record too, so the header hit-test matches what is on screen.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Before writing the Work Order's prompt, refresh the `grid.rs`/`draw.rs` line refs above against main after tile-fixes phase 20 merges.

**Spec:**
1. `draw_tile_grid` (`tiles/draw.rs`) records, for each drawn piece, the cell's stable content identity, its drawn rect, and its visible (clipped) hit region, in the grid's own state, replacing the previous frame's record. Where pieces overlap, the one drawn on top wins. Model the record's state as a type that tells "never drawn" apart from "a frame drew no interactive content" (e.g. `enum DrawnHits { NotDrawn, Drawn(Vec<DrawnPiece>) }`), never a bare `Option`.
2. `TileGrid::cell_at(&self, Position) -> Option<usize>` keeps its public signature, as the compatibility boundary, and answers from that record. Before the first draw it answers from the layout as today.
3. Phase 2's content lookup in `tiles/host.rs` uses the same record, and resolves the content by the identity recorded at draw time, not by the slot's current occupant.
4. cargo-handler keeps the header spans it drew with each tile and the summary, and `render::cell_clicked` maps a header click through those drawn spans, not freshly rebuilt ones.
5. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell; a press on the destination rect of a still-moving cell returns what is drawn there; a press on a piece's clipped-away part misses it; a press after a frame that drew nothing interactive returns no cell. In cargo-handler, a census change between the draw and the click that would otherwise shift a header column still selects the column that was drawn under the pointer. cargo-tile's suites pass.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/tests/`
- `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 2 writers.
- `grid` — tui_pane: the drawn-hit record in `draw.rs`/`grid.rs`/`host.rs`, its motion tests and `host_tests.rs`, tui_pane CHANGELOG.
- `app` — cargo-handler: drawn header spans kept with the frame, `render::cell_clicked`, their tests, cargo-handler CHANGELOG.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.

### Phase 6 — Claude reviewers and Claude dispatch seats  · status: todo

**Depends on:** Phase 4

#### Work Order

**Goal:** an unnamed Claude plan-mode reviewer shows role `review`, and a dispatch to a Claude-backed seat folds into its row like a Codex seat does.

**Source:** showrunner-approved add-ons, 2026-10-16, placed after the clicks phase.

**Constraints from prior phases:**
- Dispatch resolution carries a Codex `thread_id` and folds the launcher shell's children into the thread's children; it cannot name a Claude session row today. `ParsedDispatch` drops the launcher's `--phase` argument.
- The `["claude", "--print", "review"]` value in `census/tree.rs` is test fixture data, not a production rule. Today an unlisted Claude descendant can be treated as a carrier process rather than a visible row.

**Spec:**
1. A `claude --print --permission-mode plan` process with no other role mapping shows role `review`. Trace the real discovery-to-row path (`census/classify.rs`, `census/tree.rs`, `census/placement.rs`) and make such a process a visible row; the `review` fallback is typed and never overrides a role that already comes from the hierarchy edge or a roster.
2. The dispatch fold resolves a Claude-backed seat through `<session dir>/phases/<id>/impl_worker_<slot>.json` (family `claude` → `worker_id` is the session id), joining the dispatch to that Claude session's row the way phase 1 joins a Codex seat through `mesh_roster.json`. Model the joined worker as a family-aware identity, e.g. `enum WorkerIdentity { CodexThread(ThreadId), ClaudeSession(SessionId) }`, so identical id text from the two families never matches. `ParsedDispatch` keeps `--phase` so the fold reads the current phase's file for the slot; a file from an earlier phase is ignored. Read the file's real shape from a live run (this session dir has `impl_worker_impl.json`) before writing the parser, and record it in the summary.
3. Tests, with fixture files: a reviewer started with `--print --permission-mode plan` reaches the drawn Role column as `review` (render through the existing helpers in `worker_dispatch/tests.rs`, so this phase shares no file with phase 5 but the CHANGELOG); a hierarchy-given role still wins; a Claude-seat dispatch folds into its session's row; a stale earlier-phase identity file is ignored; a Codex thread id and a Claude session id with the same text stay apart.

**Files:**
- `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/probe.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files.
- `test` — `worker_dispatch/tests.rs` and new fixtures.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live tile shows a plan-mode reviewer as `review`. The checkpoint carries `Change-Review: Claude plan-mode reviewers show role review; dispatches to Claude seats fold into their row | open a tile running a Claude reviewer or a Claude-seat dispatch`.
