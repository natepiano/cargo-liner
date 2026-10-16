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

### Phase 2 — Sort by any column  · status: todo

**Depends on:** Phase 1

#### Work Order

**Goal:** clicking a column header in any agent tile's tree, or in the summary, sorts by that column; clicking it again reverses the order. The default, Age descending, draws exactly today's order. The same phase lands the showrunner's approved add-ons: a folded dispatch row takes its seat's role, and three style cleanups.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate and allow for sorting up/down on any column within any cell?", then "with a default on duration descending as we currently have it?". Showrunner ruling: the sort phase comes first, the click-accuracy phase after it. A tui_pane hook passes the cell-local click position, and cargo-handler keeps a sort per tile. The Age-descending default must draw exactly today's order, proved by tests. Hues are fixed before sorting. ▲/▼ appears only on the sorted header. The summary sorts within its machine groups.

**Constraints from prior phases:**
- Phase 1 draws role and account cells with `FooterColors` (`footer_colors/mod.rs`); sorting must not change any cell's color.
- Folded dispatch rows (`census/worker_dispatch/mod.rs` `ResolvedWorkerDispatch`, `tree::DispatchFold`, `placement.rs` `dispatch_label`) are one row and sort as one row.
- Codex thread rows carry `ThreadState` busy/idle in the Status column.

**Facts (line refs at 00f301a9):**
- Clicks: `tui_pane/src/runner/event_loop.rs:208` `handle_mouse` forwards a left press as `TerminalApp::click(Position)` (`runner/app.rs:52`), in absolute terminal coordinates. cargo-handler's `app.rs:255` calls `tui_pane::handle_tile_click` (`tiles/host.rs:104`). That function hit-tests to `TilePick::Cell(usize)` (`host.rs:63`, via `TileGrid::cell_at`, `grid.rs:2763`) and only focuses the cell (`host.rs:109`). No cell-local position reaches the app. The rect a cell draws into is `content_area(inner)` (`tiles/draw.rs:744`, `pub(super)`, visible from `tiles/host.rs`). `TileGrid::placements(area, growth)` (`grid.rs:2827`) gives each cell's `PaneFrame`.
- Tree: `TreeColumn { Session, Role, Status, Account, Branch, Directory, Age }` (`agent_cell.rs:252`), `header()`/`header_text()` (`:264`/`:277`), column sets `:738-770`, `tree_widths` (`:962`), `FlatColumns::fitted` (`:809`), `draw_tree` (`:986`; the Compressed view recomputes widths at `:998-1016` and may shrink spacing), header `Row` (`:1060-1071`). The header's y is `area.y + above_tree().len()` (`:929-941`); its x starts at `header_indented` (`:1184`). Column k starts at x + Σ`widths[..k]` + k·spacing.
- Row order today: siblings sort in `lines_of` (`:593`) by `line_order` (`:651`), key `(started, machine, pid)` ascending. Work subtrees sort in `work_branches` (`:495`) by `(started, pid)`, and `append_work` (`:691`) walks them depth-first. Rails and `Branch::Last` come from sorted position (`:595-627`). Ascending `started` is Age descending.
- Hues: `cell_order_placed` (`:500`) and `lines_of` (`:601-606`) hand out `RainbowHue` from a running counter in sorted order, and work inherits `parent_hue`. Today a new sort would recolor agents.
- Summary: `SUMMARY_HEADERS` (`constants.rs:141`, indexes `:151-164`): pid, agent, name, status, age, desktop, directory. `draw_machine` (`summary/mod.rs:197`) draws one header row per machine (`:232-234`) at `area.y + GROUP_HEADER_HEIGHT`, with x from `indented(area)` (`:441`) and widths from `fitted_constraints` (`:382`) / `column_widths` (`:404`). Rows come from `root_rows` (`:119`) in census `(started, pid)` order.
- App state: `struct App` (`app.rs` ≈87-121). Tile identity is `AgentCell { machine, pid, process_started }` (`tiles.rs:9`). `Cells::new` (`render.rs:126`) rebuilds every frame.
- Fold role: the Role column prints `WorkKind::Thread.role_text()` (`agent_cell.rs:379-383`, `:191`) for a folded row. `ParsedDispatch::from_arguments` (`worker_dispatch/mod.rs:83`) matches and discards the task word. `WorkerRoster::parse` (`:213`) reads only `thread_id` and `launcher_pid`. Real roster entries also carry `"role"` (`impl|test|review|fix`).

**Spec:**
1. tui_pane hook, in `tiles/host.rs`. `TileGridHost` gains a provided method that receives a click inside a cell's content rect, as the cell index plus the position relative to that rect's top-left. Its default does nothing, so cargo-tile and other hosts are unchanged. `handle_tile_click` focuses the cell as today, then calls the method when the press lands inside `content_area` of that cell's frame. A press on the frame or border calls nothing. Compute the rect from the same layout `cell_at` answers from today; the click-accuracy phase moves both to the last-drawn frame. Model the local position as its own type (e.g. `CellPoint { column, row }`), not a bare `Position`. Add a tui_pane CHANGELOG line.
2. Sort types in cargo-handler: `enum SortDirection { Ascending, Descending }`, `struct TreeSort { column: TreeColumn, direction }` and `struct SummarySort { column: SummaryColumn, direction }`. `SummaryColumn` is a new enum with one variant per summary header. It replaces index constants wherever the summary picks a column. Both sorts default to Age descending. `App` keeps `HashMap<AgentCell, TreeSort>` (a tile with no entry uses the default; drop entries for tiles that are gone on each scan) and one `SummarySort`.
3. Header hit-testing. One function computes a header row's column spans (x range per column) from the same view, widths and spacing the draw uses. `draw_tree` and the click mapping both call it, and so do the summary's draw and click mapping. Never keep a second copy of the width math. A click on a column's span in the header row sorts by that column. Clicking the sorted column reverses its direction. Clicking another column selects it in its natural first direction: Age descending, every other column ascending. A click in the gap between columns, or on a data row, changes nothing. A click on any machine's header row in the summary sets the one summary sort.
4. Sorting in the tree keeps the hierarchy: sort siblings only, at `lines_of` and `work_branches`, so each child stays under its parent and the rails and `Branch::Last` follow the new sibling order. Sort keys: Session by name text, Role by role text, Status by status text, Account by account label, Branch by branch name, Directory by path text, Age by `started`. Equal keys fall back to today's key `(started, machine, pid)`, so the order is total and stable. The summary sorts each machine group's rows with the same rule (Pid numeric). Machine groups keep their order.
5. Hues are fixed before sorting. Assign every `RainbowHue` in today's default order, then reorder, so changing the sort never recolors a name, a cell title or a work line.
6. The sorted header alone shows ` ▲` (ascending) or ` ▼` (descending) after its text, in the header style. Its width counts in `tree_widths` / `column_widths` like any header text. The default shows `Age ▼`. Update the boundary-width tests where the arrow moves a boundary.
7. Folded dispatch role (showrunner add-on, `current`). `ResolvedWorkerDispatch` carries the seat's role as an enum, e.g. `enum DispatchRole { Worker(WorkerRole), Unroled }` with `WorkerRole { Impl, Test, Review, Fix }`. The role is the roster entry's `role` when it is recorded. Otherwise it is the seat short name's role stem: the name with trailing digits removed, when that is `impl`/`test`/`review`/`fix` (impl4 → impl, fix2 → fix). Otherwise it is `Unroled`. A folded row with a worker role draws its role cell as that role (`FooterRole::Worker` color), and its name in the parent's hue, exactly as a managed worker row of that role draws. `Unroled` (e.g. `arrange`) keeps `thread`.
8. Style add-ons (showrunner, `current`). F003: remove both `#[expect(clippy::too_many_arguments)]` (`agent_cell.rs` `append_work`, `summary/mod.rs` `draw_with_footer_colors`) by grouping the per-frame inputs into one named struct. F004: drop the `#[cfg(test)]` `draw` shims in `agent_cell.rs` and `summary/mod.rs`, so one production `draw` takes the palette and tests pass `FooterColors::default()`. F005: name the domain literals (statusline.conf path, the `account.` prefix, `mesh_roster.json`, the implement.sh path, `bash`, the ThreadState idle text) in the owning module's constants, and deserialize roster entries into a typed struct (`thread_id`, `launcher_pid`, `role`) instead of `serde_json::Value` lookups.
9. Tests (the test slot, in new files):
   - With the default sort, a tile's drawn rows and hues equal today's for a fixture with nested agents, work and threads; read them back from the buffer. Do the same for the summary.
   - Each column sorts ascending and descending with siblings kept under their parents.
   - Hues are unchanged across sorts.
   - A header-span click maps to its column in both Full and Compressed views; gap and data-row clicks do nothing.
   - The arrow appears only on the sorted header.
   - Roster role and stem rules, including `arrange` → `thread`.
   - A tui_pane test shows the hook receiving the cell-local position, and receiving nothing for a border press.

**Files:**
- `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/tiles.rs`, `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/src/main.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`
- `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/summary/sort_tests.rs`
- `crates/cargo-handler/src/sort.rs`
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/tree.rs`
- `crates/cargo-handler/src/footer_colors/mod.rs`, `crates/cargo-handler/src/census/thread_state.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — the tui_pane hook and CHANGELOG, `sort.rs`, `app.rs`, `render.rs`, `tiles.rs`, `agent_cell.rs`, `summary/mod.rs`, `constants.rs`, `footer_colors/mod.rs`, `thread_state.rs`, the `mod` lines wiring new test files, and edits to existing tests that the arrow or the F004 shim removal breaks.
- `test` — new test files under `agent_cell/tests/` and `summary/sort_tests.rs`, plus the tui_pane hook test in `tiles/host.rs`'s test module. It also owns the census side of Spec 7 and F005's roster: `census/worker_dispatch/mod.rs` and `tests.rs`, `placement.rs`, `tree.rs`. That code produces `DispatchRole`; `impl` draws it.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`, `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. Live shots show the hana tile and the summary in the default order (identical rows to phase 1's shot), the hana tile sorted by Role ascending and by Age ascending, and the summary sorted by Status. The live tile also shows folded rows `impl4 · …`, `fix2 · …` with role impl/fix. The checkpoint carries `Change-Review: click a column header in a tile or the summary to sort by it, again to reverse; default Age ▼ keeps today's order; folded worker rows show their seat's role | click several headers in a tile and the summary and watch the order and arrow`.

### Phase 3 — Accurate clicks during motion  · status: todo

**Depends on:** Phase 2

**Blocked by:** tile-fixes phase 20 merged on main — the showrunner says when.

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2763`) while drawing interpolates (`drawing_at`, `grid.rs` ≈2880), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps.

**Constraints from prior phases:**
- Phase 2's cell-local click hook computes the content rect from the destination layout. This phase moves it to the last-drawn rect too, so the header hit-test matches what is on screen.

**Spec:**
1. `draw_tile_grid` (`tiles/draw.rs`) records, for each drawn piece, its rect and the cell it belongs to, in the grid's own state, replacing the previous frame's record.
2. `TileGrid::cell_at(&self, Position) -> Option<usize>` keeps its signature and answers from that record. Before the first draw it answers from the layout as today.
3. Phase 2's content-rect lookup in `tiles/host.rs` uses the same record.
4. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell, and a press on the destination rect of a still-moving cell returns what is drawn there. cargo-tile's suites pass.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/tests/`

**Workers:** 1 writer + 1 tester.
- `impl` — `draw.rs`, `grid.rs`, `host.rs`, CHANGELOG.
- `test` — the motion click tests.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.

### Phase 4 — Claude reviewers and Claude dispatch seats  · status: todo

**Depends on:** Phase 2

#### Work Order

**Goal:** an unnamed Claude plan-mode reviewer shows role `review`, and a dispatch to a Claude-backed seat folds into its row like a Codex seat does.

**Source:** showrunner-approved add-ons, 2026-10-16, placed after the clicks phase.

**Spec:**
1. A `claude --print --permission-mode plan` process with no other role mapping shows role `review` (`census/tree.rs` ≈840 holds today's `["claude", "--print", "review"]` match).
2. The dispatch fold resolves a Claude-backed seat through `<session dir>/phases/<id>/impl_worker_<slot>.json` (family `claude` → `worker_id` is the session id), joining the dispatch to that Claude session's row the way Spec 6 of phase 1 joins a Codex seat through `mesh_roster.json`. Read the file's real shape from a live run before writing the parser, and record it in the Work Order.
3. Tests for both, with fixture files.

**Files:**
- `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/placement.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files.
- `test` — `worker_dispatch/tests.rs` and new fixtures.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live tile shows a plan-mode reviewer as `review`. The checkpoint carries `Change-Review: Claude plan-mode reviewers show role review; dispatches to Claude seats fold into their row | open a tile running a Claude reviewer or a Claude-seat dispatch`.
