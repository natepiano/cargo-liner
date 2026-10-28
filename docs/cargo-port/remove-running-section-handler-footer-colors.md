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
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler`, branch `remove-running-section-handler`.
- **Footer color file:** `~/.claude/config/statusline.conf`, lines `key=value`, `#` starts a comment, blank lines ignored. Today: `role.showrunner=bright-magenta`, `role.unit-director=bright-blue`, `role.worker=bright-cyan`, `role.unassigned=purple`, `account.claude1=brown`, `account.claude2=magenta`, `account.claude3=blue`, `account.claude4=orange`, plus `level.*`/`tokens.*`/`left.*` keys handler ignores. The footer's name→ANSI table (`statusline.jq`, `def color`): black 30, red 31, green 32, yellow 33, blue 34, magenta 35, cyan 36, white 37, bright-black 90, bright-red 91, bright-green 92, bright-yellow 93, bright-blue 94, bright-magenta 95, bright-cyan 96, bright-white 97, orange `38;5;208`, purple `38;5;141`, pink `38;5;211`, teal `38;5;30`, brown `38;5;137`; any other name renders plain.
- **Key files** (line refs are main 7eacc2e3):
  - `crates/cargo-handler/src/agent_cell.rs` — `RoleLabel::role_text` ≈179; `TreeLine::account` ≈333, `TreeLine::value` ≈350 (`TreeColumn::Role` ≈353, `TreeColumn::Account` ≈364); `draw_tree` ≈857 (non-status columns draw in `text`); `header_line` ≈972.
  - `crates/cargo-handler/src/census/placement.rs` — `RoleLabel` ≈71.
  - `crates/cargo-handler/src/census/mod.rs` — `AgentRow::agent_label` ≈328.
  - `crates/cargo-handler/src/summary/mod.rs` — `heading` ≈220, `count_note` ≈250, `agent_row` ≈259.
  - `crates/cargo-handler/src/constants.rs` — `AGENT_SINGULAR`/`AGENT_PLURAL` ≈188–190.
  - `crates/cargo-handler/src/theme/mod.rs` — `Role` ≈144 (theme roles; footer colors are not theme roles); `SessionRowInk` ≈58, `ResetRank::color` ≈125, `RolePalette` ≈267 (line refs for this file are the worktree on 2026-10-23).
  - `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render/mod.rs` — where per-frame state reaches `agent_cell::draw` and `summary::draw`.
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler`
- **Invariants:**
  - Workers run only the Test/Lint lines above; each `verify.sh` run detaches and its result arrives as the worker's next message. Formatting only with `cargo +nightly fmt`.
  - No color value from the footer file is written into cargo-handler; tests use fixture text.
  - Invalid states are unrepresentable: a footer color is an enum parsed from its name, never a raw string carried to draw time.
  - No test takes a second.
  - Phase 26 facts that bind phases 18, 21, 25, 27 and 29:
    - Root hues use a root counter that is independent of descendants. `agent_cell::cell_order_placed` (`agent_cell.rs` ≈1321) gives root i entry i of the theme's `session_hues` (mod 5) through `SessionHue::of_agent(ordinal)`. Descendants advance a separate `descendants` counter. Each agent tile title keeps its summary row's ink. Do not make root hues depend on descendant counts again.
    - A session row draws every fixed foreground only through `SessionRowInk::style()` (`theme/mod.rs` ≈58). `FooterColors::session_row_inks()` (`footer_colors/mod.rs` ≈765) lists the inks that can share a session row, and the session palette checks in `render/role_title_tests.rs` measure that list. A new ink on a session row enters through a `SessionRowInk` variant, so that the tests measure it.
    - The reset ramp has one source: `theme::ResetRank::color` (`theme/mod.rs` ≈125), a blend from the `reset_nearest` to the `reset_farthest` theme role. The readability correction applies after the blend. The failed-reading dot keeps `tui_pane::error_color()`. Do not add a second reset source.

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

#### As-built

- No row is named by a raw thread id or UUID. A thread listed in its session's `mesh_roster.json` is named `<seat> · <job>`, live dispatch shell or not; otherwise `<role> · <job>` from the first line of the rollout's first task prompt (`first_task_line`, leading `#` markers stripped); otherwise the role alone (`WorkerTask::RoleOnly`). A Codex thread finds its session dir from the `/tmp/claude/unit/<id>/` or `/tmp/claude/delegate/<id>/` paths in that first prompt; with two candidates, the one whose roster lists the thread wins, then the newer roster by mtime. `FirstTaskPrompt{Unread,Recorded}` and `ThreadWorktree{Unrecorded,Recorded}` type the absent states.
- Every `implement.sh` dispatch form (`codex_mesh.py start --name <seat>` and `follow --to <seat>`, unit and delegate session dirs) folds into its worker row; `ParsedDispatch::is_final_command_of` compares the parsed dispatch whole, seat included.
- `AgentRow.activity: AgentActivity{Unreported,Idle,Active(Activity)}` replaces `status: Option<String>` and `ThreadState`. `Activity` is `Thinking`, `Writing`, `Reading`, `Git`, `Delegating`, `Web`, `Running`, `Asking`, `Waiting`, or `Verify{step: VerifyStep, location: RunLocation}`; `VerifyStep` is fmt, clippy, nextest, build, check, doc, mend, taplo; `RunLocation::Mac` draws ` (mac)`. Words come from the Claude transcript's last `tool_use` and the Codex rollout's last `function_call`, tool commands split by a shell tokenizer.
- A verify step comes from `<session dir>/mesh_runs/*.json`: `RunRecord{Ignored,Verify(VerificationRun)}` keeps only running `verify.sh` records with a thread id and start time. The run's process tree yields `VerificationProgress{Waiting,RemoteUnknown,Step{step,location}}`; a live run with no step child or `ssh` child shows `waiting`.
- Precedence, one word per row: a live verify run beats tool and turn words; among several, the latest `started_at` wins, ties to the higher pid; then the last tool call's word; then `thinking` in an open turn; `idle` once the turn closes.
- Active words take busy's color, verify steps share one accent, `waiting` takes the idle dim. Status sorts by the drawn word in tiles and the summary.
- Older probe payloads stay readable: `AgentActivity::from_legacy` converts the old `status` string at `census/probe.rs`.
- `ServiceTier::mark` returns the program alone for `Standard` and `Unrecorded`; only `Fast` adds `fast`. `STANDARD_TIER_LABEL` is gone.
- Narrow cell headings use `HeadingLocation{Shown,Cut,Omitted}`: a suffix stays only when some of its own text fits.

**Files:**
- `crates/cargo-handler/src/census/activity.rs` — `AgentActivity`, `Activity`, `VerifyStep`, `RunLocation`, `TranscriptRead`, `RunRecord`, `VerificationProgress`, precedence
- `crates/cargo-handler/src/census/scan.rs` — incremental transcript reads, row naming
- `crates/cargo-handler/src/census/tier.rs` — `RolloutRead`, `FirstTaskPrompt`, `ThreadWorktree`
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — dispatch parse and fold, `WorkerTask`, job labels
- `crates/cargo-handler/src/census/probe.rs` — legacy status conversion
- `crates/cargo-handler/src/agent_cell.rs` — `HeadingLocation`, Status column drawing

**Binds later work:** Reviewer rows in "Reviewers and Claude dispatch seats" reuse this naming, these status words, `ParsedDispatch` and `WorkerTask`; a review row's job must not be the Role boilerplate line those prompts open with. "Accounts table in the summary" builds on the summary Status column sorting by the drawn word and the `fast`-only tier mark. Any new per-scan file read follows the incremental `RolloutRead::read_to` / `TranscriptRead` shape.

**Gotchas:** Claude transcripts reach 1 GB, so a per-scan reader keeps a per-path offset cursor and reads only new whole lines. `mesh_runs.py` also records non-verify commands; only `verify.sh` records take precedence. A run detaches from its worker, so attribution goes through the run record, never process ancestry. Review-seat prompts open with a Role paragraph, so `first_task_line` yields boilerplate for them.

**Ruled out:** scanning every roster on the machine; pool reviewer rows, which moved to "Reviewers and Claude dispatch seats".

### Phase 5 — Accounts table in the summary  · status: done

#### As-built

- The summary cell draws an `accounts` table under the agents table, same header style and gap rule: `account` (label in its footer color), `role` (`roles` joined `, `, `—` when empty), `weekly left` (percent, red at ≤10%), `resets` (local `%a %H:%M`, rounded to the nearest minute, e.g. `Mon 02:00`), `credits` (number, `unlimited` or `—`; dimmed when `use_credits` is false). Rows keep accounts.py's order; its `problems` draw dimmed under the table. No email, folder or login column.
- The one source is `[census] accounts_command` = `~/.claude/scripts/lib/py ~/.claude/scripts/accounts/accounts.py status --json --stored`, set on both machines and run off the frame thread every `ACCOUNTS_REFRESH_INTERVAL` (60 s); that run is the only change detection. JSON decodes at the boundary into `Reading<T>{Known,Unavailable}`, `Credits{Amount,Unlimited,Unavailable}`, `CreditUse{Spending,Held}`, and the summary holds one `AccountsTable{NotConfigured,AwaitingFirstReading,Ready,Failed,FailedKeepingLast}` with `FailureNote{Clear,Failed}`. A failed run or bad JSON draws the dimmed line `accounts: command failed`, keeping the last rows when there were any; with no command, or before the first result, only the agents table draws.
- Row age is `Freshness{Fresh,Stale(age),Unread}`: a reading over an hour old dims the label with `<age> old`; no `read_at` dims with no age. Both currently use `Modifier::DIM`.
- `summary::height(machines, placement, accounts: &AccountsTable)` and `summary::width(.., accounts: &AccountsTable)` are the single sizing pair; the demand includes heading, rows, problem lines and the why-line. A click on the accounts header changes no sort.
- Worker job names come from `JobText{Recorded,Unrecorded}::parse(prompt, JobSource{FirstTaskPrompt,DispatchJob})`, the one extraction for the role fallback, roster and live dispatch labels. A `#` heading line is the job; an opening starting `ROLE_PREAMBLE = "You are slot "` takes the job from its first `board.sh role … "<job>"` call (via `shell_words(command, CommandBoundary)`), else a later heading; `Unrecorded` becomes `WorkerTask::RoleOnly`. `impl From<Option<&str>> for AgentActivity` replaces `from_legacy`.
- `tui_pane::LoadedConfig` writes by merging the serialized config into the existing document with `toml_edit` (`ConfigRewrite{Unchanged,Changed}`): known values set, missing keys and sections added, unknown keys, comments, ordering and value decor kept, no write when the merged text equals the file, a malformed file left untouched. cargo-tile and cargo-port get the same behavior.

**Files:**
- `crates/cargo-handler/src/census/accounts.rs` — wire decode, reading types, `AccountsTable`, refresh thread
- `crates/cargo-handler/src/summary/accounts.rs` — accounts table layout, sizing and drawing
- `crates/cargo-handler/src/summary/mod.rs` — `height`/`width` including the accounts table
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — `JobText` parser and `ROLE_PREAMBLE`
- `crates/cargo-handler/src/config.rs`, `constants.rs` — `accounts_command`, `ACCOUNTS_REFRESH_INTERVAL`
- `crates/tui_pane/src/app_config/loaded.rs` — preserving config rewrite; tui_pane's CHANGELOG records it

**Binds later work:** the summary's demand changes with every accounts reading, so click hit-testing must use the measured `height`/`width`. Reviewer naming uses `JobText::parse`/`JobSource`; a pool review's latest task prompt may need its own `JobSource` variant, and a pooled-reviewer opening joins `ROLE_PREAMBLE` handling.

**Gotchas:** a handler binary older than this change still erases unknown config keys (including `accounts_command`) on every rewrite until the new build is installed. A `git merge --no-commit` trial merge in a berth-enrolled worktree reads to berth as edits.

**Ruled out:** re-deriving roles or usage in Rust (accounts.py's JSON is the only source); a cargo-handler CHANGELOG line for the config fix (tui_pane's CHANGELOG is the record).

### Phase 6 — Live roles and readable account colors  · status: done

**Depends on:** Phase 5

#### As-built

- Hierarchy discovery runs once per scan at `ScanStage::Classified`, so showrunner and unit tiles draw with their roles about 1 s after start instead of waiting on the detail reads (transcript tails, tier reads), which dominate the first full scan.
- The hierarchy travels as one typed value, `HierarchySnapshot{edges, notice}` with `HierarchyNotice{Clear, Failed(HierarchyError), NoSessionMatched}`, from discovery through `census/schedule.rs` (`CensusUpdate::Local{rows, hierarchy, stage}`) and from a remote machine through `census/probe.rs` (`MachineState::Answered(rows, HierarchySnapshot)`). A failure keeps the last edges, so rows stay placed through success, failure and recovery.
- `Failed` and `NoSessionMatched` draw one dim note line under the summary's agents table (`hierarchy: timed out`, `hierarchy: no session matched`); the note shows even with no agents and no accounts, and an unset command draws nothing. `NoSessionMatched` persists across cache expiry until the next answer.
- `HierarchySnapshot::matched()` counts an edge as matched when its manager is a scanned session or its member is a scanned session or Codex child thread id, so a partial answer (worker edges only) draws no note.
- Footer colors drawn as text (account labels, role colors, tile header labels) resolve against `theme.text.bg_focus`: a named color is converted to RGB through `tui_pane`'s xterm approximation and kept as its named ANSI color when it meets 4.5:1; otherwise it is emitted as RGB at the nearest HSL lightness of the same hue that meets 4.5:1. Stale and unread account labels use a computed subdued RGB that meets 4.5:1 in place of `Modifier::DIM`. The resolved palette is cached per theme `Arc`.

**Files:**
- `crates/cargo-handler/src/census/` (`hierarchy.rs`, `schedule.rs`, `probe.rs`, `scan.rs`, `classify.rs`, `placement.rs`, `tree.rs`, `remote.rs`, `mod.rs`) — discovery at `Classified`, `HierarchySnapshot` and `HierarchyNotice`, update and wire transport
- `crates/cargo-handler/src/footer_colors/` (`mod.rs`, `constants.rs`, `tests.rs`) — contrast resolution and per-theme palette
- `crates/cargo-handler/src/summary/mod.rs`, `summary/accounts.rs` — hierarchy note line and its height in `summary::machine_height`, subdued account labels
- `crates/cargo-handler/src/app.rs`, `terminal.rs`, `render.rs`, `agent_cell.rs` — consume the single `CensusUpdate::Local` shape

**Binds later work:** `HierarchySnapshot{edges, notice}` with `HierarchyNotice{Clear, Failed(HierarchyError), NoSessionMatched}`; `CensusUpdate::Local{rows, hierarchy, stage}`; `MachineState::Answered(rows, HierarchySnapshot)`; discovery at `ScanStage::Classified` once per scan; `HierarchySnapshot::matched()` accepts a scanned manager or a Codex child thread id; `summary::machine_height` counts hierarchy notice lines; text colors meet 4.5:1 against `theme.text.bg_focus` via the xterm approximation with HSL fallback, palette cached per theme. Hierarchy edges carry extra keys (`unit`, `thread_id`, `state`, `type`) for review edges, whose `member` is a Codex thread id. The probe sends the legacy `hierarchy_failed` flag beside `hierarchy_notice`, so an older receiver still keeps its links on failure.

**Gotchas:** a capture under a scratch `XDG_CONFIG_HOME` makes `production_tree.py` drop every showrunner→unit edge (herdr socket discovery is rooted at `$XDG_CONFIG_HOME/herdr`; a symlink does not help). Prefix `hierarchy_command` with `XDG_CONFIG_HOME=/home/natepiano/.config` in capture configs. The first full scan takes about 35 s in release (218 s debug), all of it detail reads, not hierarchy.

**Ruled out:** dropping the `hierarchy_failed` wire flag — older receivers depend on it.

### Phase 7 — First scan under two seconds  · status: done

#### As-built

- Each transcript and rollout path keeps a `JsonlCursor { NeverRead, AfterHead { offset, buffered }, Following { offset, pending } }` in `census/transcript.rs`. After the first read, `JsonlCursor::read_until(path, initial_tail_bytes, decisive)` reads only appended bytes. A file shorter than its `Following` offset resets the cursor to `NeverRead`.
- A cold read starts at `TRANSCRIPT_TAIL_START` (64 KiB) from the end. It grows backward through disjoint ranges by `TRANSCRIPT_TAIL_GROWTH` (4x) until `decisive` holds or the read reaches the head floor. There is no fixed tail limit.
- `RolloutRead` (`census/tier.rs`) reads the head forward only until `HeadValues` is `Complete` (`session_meta` and the first task are known). The cursor is then `AfterHead`, and the head is not read again.
- `ReadBatch { lines, bytes_read }` returns complete records and the real file traffic of one read. `record_kind` parses only the `type` header into `RecordKind { Claude, Codex, Session, Other }`.
- Rows from these reads are equal to rows from a whole-file replay. The release first scan on natedev fell from 92 s to 0.40 s. Codex rollout bytes on a cold scan fell from 65.8 MB to 7.4 MB. Claude transcript bytes are 37.0 MB.

**Files:**
- `crates/cargo-handler/src/census/transcript.rs` — `JsonlCursor`, `ReadBatch`, `RecordKind`, `SubagentReads`
- `crates/cargo-handler/src/census/tier.rs` — `RolloutRead` and `HeadValues` on the cursor
- `crates/cargo-handler/src/census/activity.rs` — `TranscriptRead` on the cursor
- `crates/cargo-handler/src/census/scan.rs` — keeps the readers from scan to scan
- `crates/cargo-handler/src/constants.rs` — `TRANSCRIPT_TAIL_START`, `TRANSCRIPT_TAIL_GROWTH`
- `crates/cargo-handler/src/census/incremental_read_tests.rs` — appended-bytes, bounded-tail and whole-replay equivalence tests

**Binds later work:** census readers use `JsonlCursor` with `TranscriptRead`, `RolloutRead` and `SubagentReads`. No reader does a whole-file read. A cold tail grows backward until `decisive` holds, so `decisive` must require each value the row shows (for example, the latest task). A value between the head and the tail is read only if `decisive` requires it.

**Gotchas:** large-fixture tests must stay near 1 MiB. Larger fixtures took 2.9 s in the full suite.

**Ruled out:** a fixed 16 MiB tail limit and a fixed 4 MiB head limit, because both gave wrong rows.

### Phase 8 — Reviewers and Claude dispatch seats  · status: done

#### As-built

- A `claude --print` or `-p --permission-mode plan` process with no other role mapping classifies as role `review` (`WorkerRoleInput::ReviewerLease`). A top-level reviewer joins the unassigned tile and draws `review`. A role from a hierarchy edge or a roster wins over this fallback.
- A dispatch to a Claude-backed seat folds into that seat's Claude session row. `impl_worker_<slot>.json` holds the durable identity (`name`, `slot`, `family`, `model`, `worker_id`); a join also needs a live claim pid in `impl_claim_<slot>.json`. `SeatFamily` keeps a Codex thread id and a Claude session id with the same text apart. `ParsedDispatch` carries the pass role, and the slot token is the fallback.
- A busy pool reviewer draws as a `review` row under its leased unit, named `<reviewer> · <job>`, with the current review's job. Typed pool documents (`ReviewerPoolDocument`, `PoolReviewer`, `PoolTemplate`, `PoolLease`, `ReviewerProcessClaim`, `ReviewAllocation`) read `pool.json`. The reviewer cache is keyed on the `pool.json` modification time, and each scan checks that `busy_pid` is live. Edge state crosses as `HierarchyWireReviewerState` into `ReviewerState`. Idle reviewers and templates draw nothing.
- `RolloutRead` holds `RolloutStart{Recorded(u64), Unavailable}` and `CurrentReviewPrompt` beside `FirstTaskPrompt`. Its backward `decisive` predicate also requires the latest task prompt. A row is role-only until a prompt recorded after the busy lease began arrives.
- The probe stays at schema 3: `ChildRow` becomes `ProbeChildRow` at the wire, with the reviewer task in a defaulted `reviewer_task` field beside the legacy `worker_task`. `HierarchyAnswer::Failed{reason, reviewers}` keeps reviewer outcomes when discovery fails. `ReviewerPoolLocation{MissingHome, Directory}`: `MissingHome` only clears the cache.

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs` — pool reading, reviewer edges, reviewer liveness
- `crates/cargo-handler/src/census/classify.rs` — plan-mode reviewer role fallback
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — Claude seat identity (`SeatFamily`), reviewer job parse
- `crates/cargo-handler/src/census/tier.rs` — `RolloutRead` start and current-review prompt
- `crates/cargo-handler/src/census/probe.rs` — `ProbeChildRow` wire rows
- `reviewer_tests.rs`, `claude_seat_tests.rs` and fixtures — reviewer rows, liveness, Claude seat joins

**Binds later work:** A helper-session mark must win over the `claude --print`/`-p --permission-mode plan` review fallback in `census/classify.rs`. Child rows cross to remote receivers through `ProbeChildRow` at schema 3; new data goes in defaulted extension fields such as `reviewer_task`.

**Gotchas:** A pooled reviewer's job text comes from the prompt's `this is phase N of <plan>, "<title>"` sentence or a `# ` heading. Without either, it draws no job, so closure prompts must carry that sentence.

**Ruled out:** Guarding the `pool.json` lease reread race — stale for at most one scan.

### Phase 9 — Accurate clicks during motion  · status: todo

**Depends on:** Phase 5, Phase 12, Phase 15, Phase 16, Phase 17, Phase 18, Phase 19, Phase 20, Phase 23, Phase 24, Phase 26, Phase 27, Phase 28

**Blocked by:** tile-fixes phase 24 merged on main (tui_pane grid overlap) — the showrunner says when (showrunner, 2026-10-21).

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs` ≈2917) while drawing interpolates (`drawing_at`, `grid.rs` ≈3042), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps. Since phase 6, `summary::machine_height` counts hierarchy notice lines, so a first-machine notice appearing or clearing moves later machines' clickable headers; add a two-machine draw → notice change → click regression.

**Constraints from prior phases:**
- Phases 23, 24 and 27: `shared_tree_metrics` (`agent_cell.rs`) groups tiles by `PermittedTreeColumns` (columns populated in the complete rows) and screen x. `ColumnDrop::ORDER` (`agent_cell.rs` ≈476) also drops Age and Account, and `GroupFitStep::ORDER` (≈908) has six steps. Restore saved tree viewports before measurement. Project the retained `TreeMeasurement` through `allocated` (`agent_cell.rs` ≈730) into the tallest frame interior, and get the minimum width from it. Then resolve the shared metrics for each allocation before you install the selection areas. Record the final painted header spans, Compressed headers included; `FittedTreeColumns` is not the final header.
- Phase 27 closed these outcomes: `TileWidthFloor` is visible to the application through placements and its floor tests. `ColumnPlan::NoAdmissibleColumn` is an internal choice that maps to the generic summary-only display. `TreeColumnFit::TooNarrow` is an internal request for core compression. Neither needs a new user message.
- Phase 27: `summary::truncated_line` (`summary/mod.rs` ≈846) cuts styled text by terminal cells and grapheme clusters, and `SummarySessionTable::OmittedForWidth` (≈226) removes session lines (its 34x50 regression is green). Keep that painted row mapping across resize and intervening input. Phase 20 owns the word-target test.
- Phase 24 motion: tui_pane `TileCells::prepare_moving_frame(destination, motion_bounds)` (`tiles/draw.rs`; the default calls `prepare_frame`) and `TileGrid::motion_preparation_allocations` (`tiles/grid.rs`) give destination positions and widths with the taller of the source and destination heights; cargo-handler `Cells::prepare_moving_frame` (`crates/cargo-handler/src/render/mod.rs` ≈697) passes those motion bounds to `prepare_frame` to measure role widths, and cargo-tile keeps the default. Keep these preparation bounds for measurement; record hit regions and header spans from the pieces actually painted, not from preparation allocations. Test: a shrinking tile clicked during motion selects the header drawn under the pointer, and the existing moving-role tests (`render/moving_role_tests.rs`) stay green.
- Phase 4's narrow headings fit through `HeadingLocation::{Shown,Cut,Omitted}` (`agent_cell.rs`); test clicks against the fitted heading actually drawn, not the full one.
- After phase 5 the summary's demand changes with each accounts reading.
- Phase 28 (a35577ad) supplies the painted record: `PaintedCells::{Undrawn, Drawn}`, a stable `CellIdentity` per painted piece, `CellHit::{Missed, Retired, Current}` (`tiles/grid.rs` ≈3062 `record_painted_cells`, ≈3076 `cell_hit`, reverse paint order) and `content_click` (`tiles/host.rs` ≈176), which maps a painted frame back to source coordinates for presses and wheel steps. `handle_tile_click` absorbs `TilePick::RetiredCell` before focus or row selection. `cell_content_at` is gone. Extend this record; do not create another painted-geometry record.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Phase 3's tile identity is `tiles::TileKey::{Agent(AgentCell), Unassigned { machine }}` (sorts in `App::tree_sorts: HashMap<TileKey, TreeSort>`, pruned by `agent_cell::tile_keys`), and header layout carries `agent_cell::SessionHeaderLead::{OverBranch, Flush}` (the pool's session header sits over flush names). Drawn header spans are kept per `TileKey`, with the lead they were drawn with (`render/mod.rs` ≈443 `Cells::tile`, ≈448 `Cells::header_spans`; `agent_cell.rs` ≈2401 `header_spans`, ≈2164 `unassigned_header_spans`).
- Accounts-header click: phase 32 already keeps the painted summary. `App` holds `PaintedSummary::{NotDrawn, Drawn}`, and `render::cell_clicked` reads the accounts header spans from it (`render/mod.rs` ≈260), then calls `AccountsSort::select` (`sort.rs`). This phase extends that record with the painted clipping, the scrolled and pinned line mapping and the drawn identities; it does not rebuild headers from the current reading. Keep the census-change and the several-inputs-before-redraw tests.
- Phases 16 and 20 add row and word clicks that can also hit changed content between draw and click: the frame record covers drawn row identities, clipped scrolled rows, wheel targets and word cells, tile titles and retired tiles included. Phase 20 resolves one press once, into both the row action and the painted word, and forwards tile title presses from tui_pane; keep both.
- Public `TileGrid::cell_at` still answers from settled geometry, and `cell_hit` calls it on the before-first-paint path.
- cargo-handler `Cells::prepare_moving_frame` (`render/mod.rs` ≈697) supplies the motion measurement bounds to `Cells::prepare_frame` (≈705). `measure_trees` (≈458) and `restore_tree_viewports` (≈570) are in the same file. `prepare_frame` resolves the selections, runs `agent_cell::shared_tree_metrics`, and only then installs each selection area (`SurfaceSelection::set_area`). Keep that order. Those areas come from preparation allocations, so they are not the final clipped paint record.
- `draw_panes_at` (`render/mod.rs` ≈151) follows a moved selection: after the draw it finds the destination placement, takes the centre of its rect and calls `app.tiles.cell_at` (≈218) to get the index for `focus_cell`. When `cell_at` answers from painted geometry, that centre can name another moving piece.
- The pointer payload `tui_pane::CellClick<Id>` (`tiles/host.rs` ≈35, exported in `tiles/mod.rs` ≈45 and `lib.rs`) carries presses (`TileGridHost::cell_clicked`) and wheel steps (`TileGridHost::cell_scrolled`, `handle_tile_scroll`). Its users: `crates/cargo-handler/src/app.rs`, `render/mod.rs`, `render/accounts_grid_tests.rs`, `render/role_title_tests.rs`, `summary/accounts_header_sort_tests.rs`, `summary/unassigned_summary_tests.rs`, and `crates/tui_pane/src/tiles/host_tests.rs`.
- Phase 28 replaces the bare `Option` results of the selection API (`selected_span`, `containing`, `follow_focused`, `surface`, `surface_mut`, `Cells.focused`, `selection_surface`, `Cells::with_selections`) with named results. Use those names; do not add a new bare `Option`.
- Phase 28 pins the accounts heading and column headers. Pinned header lines and scrolling body rows have different screen-to-content mappings: a pinned line maps to a fixed content line, and a body row maps through the scroll offset below the pinned lines. Keep that whole mapping (pinned lines, body offset and pinned count) with the painted frame, not only the scroll offset.
- Phase 16's selected-row behavior (keys, clicks and the wheel; `render/selection_tests.rs`) stays as it is.
- Tests in tui_pane tiles live in the inline `mod tests` of `tiles/grid.rs` (≈5732, motion tests), `tiles/draw.rs` (≈814, motion content tests) and `tiles/host.rs` (≈205), and in `tiles/host_tests.rs` (registered at `host.rs` ≈446). There is no `tiles/tests/` directory.
- Phase 17: drawing prepares `TreeMetrics` across all tiles in one screen column and saves them in `App.tree_layouts` (`render/mod.rs` ≈177); since phase 23 each drawn group shares one age layout (`summary/age.rs` `AgeLayout`). A click must map through the metrics that were drawn, not ones rebuilt from current rows and time. Add a regression: another tile in the same screen column changes the shared widths between draw and click, and the click still selects the header drawn under the pointer, with the drawn group's shared age layout kept.

**Spec:**
1. Make public `TileGrid::cell_at(&self, Position) -> Option<usize>` convert phase 28's `cell_hit` result (keep the signature as the compatibility boundary). Keep the before-first-paint layout fallback in a separate helper, because `cell_hit` calls `cell_at` on that path today.
2. Done by phase 28: the painted record with stable identity, top-most piece wins, and retired-content absorption in `tiles/host.rs`. Keep its tests green.
3. Done by phase 28: content lookup through `content_click` resolves by the painted identity.
4. cargo-handler keeps the header spans it drew with each tile and the summary, and `render::cell_clicked` maps a header click through those drawn spans, not freshly rebuilt ones. A click on a drawn tile that a later scan removed (its sort already pruned) records no preference, so it cannot bring a pruned pool sort back. Phase 28 already keeps a click on a vanished drawn tile from focusing its replacement; keep that regression (`render/pinned_accounts_tests.rs`). If `Cells::tile` (`render/mod.rs` ≈443) stays, it returns a semantic lookup result rather than an `Option`, converting to `Option` only at the framework boundary.
5. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell; a press on the destination rect of a still-moving cell returns what is drawn there; a press on a piece's clipped-away part misses it; a press after a frame that drew nothing interactive returns no cell. In cargo-handler, a census change between the draw and the click that would otherwise shift a header column still selects the column that was drawn under the pointer, for a tree tile and the unassigned tile, in Full and Compressed views; a click on a drawn tile that has since disappeared records no sort and focuses no replacement; an accounts reading that changes the summary's demand between draw and click still selects the drawn agents column; an accounts-header click during motion, or after an accounts reading changed the table between draw and click, sorts by the accounts column that was drawn under the pointer. cargo-tile's suites pass.
6. Acceptance tests also cover row, wheel and word targets, including clipped scrolled rows and retired tiles.
7. Phase 23 drops a column that is empty in every row of a tile, and `render::cell_clicked` today rebuilds header spans from current rows and restores only saved widths. Test, red first: a tile draws a `type` or `role` column, then its rows change so that column is empty, and a click on the drawn header still selects the column drawn in that frame.
8. The recorded frame also holds, for each painted piece, the painted row identities and their lines, the account spans, the scroll offset of each surface and the clip of the piece. Row clicks, wheel targets and selection bands map through that record. Test: several input events (two clicks and a wheel step) before one redraw all map through the same recorded frame. The record holds the pinned mapping of phase 28 for the summary. Tests during motion: a click on a pinned accounts column header sorts by the column drawn there, and a click on an account continuation row after End selects the row drawn there; both still hold when an accounts reading changes the table between the draw and the click.
9. Focus following uses stable content identity. `draw_panes_at` finds the cell that holds the destination surface by its content identity, not by `cell_at` at a placement centre. Test: a selected session that moves between surfaces during motion, with another moving piece drawn over the destination centre, moves focus to the tile that holds the session.
10. Rename `CellClick` to a name that is true for a press and for a wheel step (for example `CellPointer`), in tui_pane and in every user listed above. The tui_pane CHANGELOG marks it **Breaking**.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/drawn_hits_tests.rs` — new drawn-record tests, registered with `#[path]` in `tiles/grid.rs`
- `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/summary/accounts.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/interaction.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/render/drawn_frame_tests.rs` — new handler click-mapping tests, registered in `render/mod.rs`
- `crates/cargo-handler/src/render/moving_role_tests.rs`, `crates/cargo-handler/src/render/selection_tests.rs`, `crates/cargo-handler/src/render/accounts_grid_tests.rs`, `crates/cargo-handler/src/render/role_title_tests.rs`, `crates/cargo-handler/src/summary/accounts_header_sort_tests.rs`, `crates/cargo-handler/src/summary/unassigned_summary_tests.rs`

**Workers:** 2 writers.
- `grid` — extend the existing painted-cell lookup for public `cell_at`, keep the undrawn fallback, do the shared pointer-payload rename, and own all tui_pane files and framework regressions; hubs: `tiles/grid.rs`, `tiles/host.rs`, `tiles/mod.rs` and `lib.rs`.
- `app` — keep the header and row records actually painted, integrate phase 20's word record, follow focus by stable identity, migrate handler payload users, and own all handler files and regressions; hub: `render/mod.rs`; keep phase 28's pinning and named selection results.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-tile`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` clean. `render/selection_tests.rs` passes unchanged. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.

### Phase 10 — Restarted unit directors stay under their showrunner  · status: done

#### As-built

- The first scan holds every row that the first hierarchy answer can place. `LocalHierarchy::Pending` keeps them as `PendingPlacementRows`, a list of `PendingPlacementRow::{Independent, AwaitingHierarchy}`. A Claude row, or a carrier with an identified Codex thread, is `AwaitingHierarchy`. No held row publishes as a top-level row.
- `FirstHierarchySessions` records the session ids present when the first command starts. `HierarchyCache::publication` returns `HierarchyPublication::RefreshNeeded` for an id that the answer does not name. `HierarchyDiscovery::Answered(CheckedUnlinkedSessions)` records each checked id, so one new unnamed id causes one refresh only. A known edge still survives a pid-only restart and a failed command.
- The wait for the first answer has the limit `HIERARCHY_FIRST_PUBLISH_BUDGET` (1 s). When the limit expires, the scan publishes the `Independent` rows and keeps the held rows until the answer arrives.
- The local scan runs on its own worker thread. `LocalPublications` is behind `Arc<Mutex<_>>`. `ScheduleEvent::HierarchyReady` calls `finish_first(Duration::ZERO)`. `Finished` exits at once.
- A failed first command releases the held rows with its failure notice. The scan uses that failed outcome again for the rest of the scan.
- Live on natedev: first frame 11 ms; first local publish 840 ms with all rows; the hierarchy command takes about 0.61 s.

**Files:**
- `crates/cargo-handler/src/census/schedule.rs` — first-scan hold, worker-thread scan, `LocalPublications`, `finish_first`
- `crates/cargo-handler/src/census/hierarchy.rs` — `HierarchyDiscovery`, `CheckedUnlinkedSessions`, `HierarchyCache::publication`
- `crates/cargo-handler/src/constants.rs` — `HIERARCHY_FIRST_PUBLISH_BUDGET`
- `crates/cargo-handler/src/census/restart_hierarchy_tests.rs` — restart-window tests
- `crates/cargo-handler/src/census/fixtures/restarted_unit_{sessions,hierarchy,codex,worker_hierarchy}.json` — restart-window process trees and answers

**Binds later work:** "Catch-up hierarchy answer off the scheduler thread" moves the catch-up and cache-refresh hierarchy commands off the scheduler thread. Today they run there while they hold the `LocalPublications` lock.

**Gotchas:** A late-answer test must not pass through the `Finished` exit, because that exit publishes all rows.

**Ruled out:** Holding rows after a failed first command — a failure is the answer, and a hold hides every Claude session while the command is broken.

### Phase 11 — Catch-up hierarchy answer off the scheduler thread  · status: done

**Depends on:** Phase 10

#### As-built

- Every hierarchy request runs on a worker thread through `start_request`. No command runs on the scheduler thread or under the `LocalPublications` lock. The initial request (`start_initial`) keeps the 1 s first-publish budget. `finish_pending(timeout)` collects the answer.
- `prepare_snapshot` returns `HierarchyPreparation::{Request(HierarchyRequestKind), Ready(snapshot)}`. It reserves the checked ids before a refresh starts. A refresh keeps the ids that are already checked. The initial answer resets them.
- `HierarchyRequestKind::Refresh(HierarchyRefreshScope)` fixes three sets when the refresh starts: the ids that wait (unchecked and not named by a retained edge), the ids already observed, and the retained links. `PendingHierarchyRequest::Refresh { answer, placement }` holds that scope until the answer arrives.
- `PendingPlacementRows::refreshing` holds each waiting row together with its full linked component (row ids and retained edges). Sessions that are already checked continue to publish.
- `deliver_held` publishes independent rows while either request kind is pending. A refresh publishes them as `ScanStage::Classified`.
- Live on natedev: first local publish 440 ms.

**Files:**
- `crates/cargo-handler/src/census/schedule.rs` — scheduler, pending requests, held rows, `deliver_held`
- `crates/cargo-handler/src/census/hierarchy.rs` — cache, `prepare_snapshot`, `HierarchyRefreshScope`
- `crates/cargo-handler/src/census/catch_up_hierarchy_tests.rs` — rendered catch-up tests
- `crates/cargo-handler/src/census/fixtures/catch_up_{rows,links,unnamed_thread}.json` — catch-up process trees and answers

**Gotchas:**
- Build a refresh's scope before the request's ids are marked checked.
- A refresh must not publish as `ScanStage::Completed`, because `Completed` reloads footer colors in `drain_census` (`terminal.rs`).

**Ruled out:** A per-row hold — it splits a manager from its linked unit. Holding every unnamed id — it hides sessions that are already checked.

### Phase 12 — Accounts table: reset gradient and header sort  · status: done

**Depends on:** Phase 5

#### As-built

- Reset times shade by the rank of each distinct timestamp. The soonest draws the theme's success color. The latest draws its error color. Ranks between blend evenly in the theme's color space. Equal timestamps share a rank. One distinct time draws success.
- `theme::ResetRank` (`positions(distinct)`, `color()`) makes only valid ranks. A missing reset keeps its plain text and does not shift ranks.
- `sort::AccountsColumn` (`Account`, `Roles`, `WeeklyLeft`, `Resets`, `Credits`) implements `SortColumn`. `sort::AccountsSort { SourceOrder, Sorted(ColumnSort<AccountsColumn>) }` defaults to `SourceOrder`. `AccountsSort::select` starts at the column's natural direction, then uses the shared `ColumnSort` toggle.
- A click on an accounts header sorts only the accounts table. A second click reverses it. Missing readings stay last in both directions. Equal values keep source order. The agent sort does not change.
- `App.accounts_sort` holds the sort for the app run. `render::cell_clicked` maps an accounts-header click to `AccountsSort::select`.

**Files:**
- `crates/cargo-handler/src/theme.rs` — `ResetRank`
- `crates/cargo-handler/src/sort.rs` — `AccountsColumn`, `AccountsSort`
- `crates/cargo-handler/src/summary/accounts.rs` — comparators, `reset_colors`, `sorted_width`, `draw_sorted`, `header_spans`
- `crates/cargo-handler/src/summary/mod.rs` — `SummaryFrame.accounts_sort`, `width`, `draw`, `accounts_header_spans`
- `crates/cargo-handler/src/render.rs` — `cell_clicked` accounts-header branch
- `crates/cargo-handler/src/app.rs` — `App.accounts_sort`
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs`, `summary/accounts_header_sort_tests.rs` — gradient and sort tests

**Binds later work:** "Accurate clicks during motion" depends on this click path. `render::cell_clicked` rebuilds `summary::accounts_header_spans(area, &cells.summary_frame())` at click time. `SummaryFrame.accounts_sort` carries `AccountsSort` into `summary::width` and `summary::draw`. The drawn accounts header spans must stay with the frame, as the agent header spans do.

**Gotchas:** Compare readings for equality before `total_cmp`. If not, -0.0 and 0.0 skip the source-index tie rule. `ResetRank::color` carries `#[expect(clippy::cast_possible_truncation)]`; `positions` bounds the rank, so the rounded alpha stays at most 255.

### Phase 13 — Discovery tests prove no allocation without a clock  · status: done

**Depends on:** none

#### As-built

- `ThreadCountingAllocator` is the `#[global_allocator]` of the cargo-port test build only (`#[cfg(test)]` in `main.rs`). It forwards every call to `System` and counts `alloc`, `alloc_zeroed` and `realloc` calls on the measured thread only.
- `test_support::count_allocations(operation) -> (R, usize)` counts the allocations of one synchronous operation; a nested measurement panics. The `AllocationMeasurement` guard sets `AllocationCounting::Measuring`, and its `Drop` sets `Disabled` again, also after a panic.
- `handle_project_discovered_does_not_allocate_per_comparison` adds one project to a list of 8 roots and to a list of 200 roots and requires equal counts. `is_deleted_does_not_allocate_display_paths` requires zero allocations. Neither test uses a clock.
- `RootItem::list_order` compares the lowercased chars of the borrowed leaf names (`paths::directory_leaf_str`), then the path. It allocates nothing for each comparison; the order does not change.

**Files:**
- `crates/cargo-port/src/test_support.rs` — the counting allocator, `count_allocations`, and its unwind test
- `crates/cargo-port/src/main.rs` — `#[cfg(test)]` allocator registration
- `crates/cargo-port/src/project/root_item.rs`, `crates/cargo-port/src/project/paths.rs` — allocation-free `list_order`, `directory_leaf_str`
- `crates/cargo-port/src/tui/app/mod.rs` — the two allocation tests

**Binds later work:** "Every cargo-port test runs under a second" proves "no work for each item" with `count_allocations`, not clocks. The two discovery tests take about 0.39 s and are not in its list of slow tests.

**Gotchas:**
- When all roots are visible, `compute_visible_rows` pushes into its row `Vec`, so the count increases with the visible list. The discovery test keeps 8 visible roots in both lists (all others `Dismissed`), and the new leaf sorts last, so the insertion compares it with every root.
- The `GlobalAlloc` impl has a test-only `#[allow(unsafe_code)]`. A `stats_alloc` replacement waits on the user's answer.

### Phase 14 — Every cargo-port test runs under a second  · status: done

**Depends on:** none

#### As-built

- `http/client.rs` has `enum ProxyPolicy { System, #[cfg(test)] Disabled }` and `fn build_client(proxy_policy: ProxyPolicy) -> Result<Client, Error>`. Production uses `System`. The test client uses `Disabled` (`.no_proxy()`), so tests do no system proxy discovery. The child-process test `disabled_proxy_policy_ignores_proxy_environment` proves this.
- `project/git/probes.rs` has `pub(super) fn run_independent_probes<A: Send, B: Send>(left, right) -> (A, B)`. It runs two git probes on `std::thread::scope`. `finish_probe` joins each thread and re-raises a probe panic with `resume_unwind`. Results and error order do not change.
- `repo/info.rs` reads remote names and push URLs through `run_independent_probes`, and runs `branches::resolve_local_main_branch(&repo_root, &config)` on a scoped thread. The test `independent_git_probes_run_outside_the_callers_rayon_pool` pins the thread context.
- The shared git fixture initializer in `test_support.rs` copies two seeded loose objects (`EMPTY_TREE_ID`, `INITIAL_COMMIT_ID`) from `test_support/git_seed/*.zlib` and starts no git subprocess for the empty repository. Fixtures with files still run `git add` and `git commit --amend`.
- The full cargo-port suite takes 4.0 s. The slowest test takes 0.453 s.

**Files:**
- `crates/cargo-port/src/http/client.rs` — `ProxyPolicy` and `build_client`
- `crates/cargo-port/src/project/git/probes.rs` — the scoped-thread probe pair
- `crates/cargo-port/src/project/git/{mod.rs, branches.rs, checkout.rs, repo/info.rs}` — probe call sites, `resolve_local_main_branch(repo_root, &config)`
- `crates/cargo-port/src/test_support.rs`, `crates/cargo-port/src/test_support/git_seed/{.gitattributes, README.md, two .zlib}` — the seeded empty repository
- `crates/cargo-port/src/tui/app/mod.rs` — tests on the shared fixture

**Gotchas:**
- The parallel git probes use scoped threads, not `rayon::join`. Callers run inside `rayon::spawn` (crates.io refresh, enrichment) and `spawn_blocking` (watcher refresh). There, `rayon::join` waits on the same pool as blocking network requests.
- `.gitattributes` marks `*.zlib binary`. Text diff tools and the banned-word scanner must skip these files. `README.md` holds the Python script that makes them again.

**Ruled out:** `rayon::join` for the probe pair — it shares the pool with blocking crates.io requests.

### Phase 15 — Role colors, subagent rows and summary sessions  · status: done

**Depends on:** Phase 12

#### As-built

- A role word draws in its `FooterColors::role` tint in every place: the cell title's role suffix (`render.rs` `group_title`), the tree's role column, and the accounts table Roles column, where each role word is its own span (`summary/accounts.rs` `role_words`; `spare` and the separator stay in the row's base style, a stale row keeps `FooterTint::subdued_style`). `account.claude1=brown` lives in `~/.claude/config/statusline.conf`.
- Management inks: `FooterTint::Management(FooterColor)` serves only showrunner and unit-director. It must reach 4.5:1 after glyph coverage, `InkCoverage::Antialiased` (229/255 foreground over 26/255 ground, `GLYPH_BACKGROUND_SHARE = 26`), on the ground the tile grid composes. Account and worker tints keep the opaque `InkCoverage::Opaque` policy. Light inks: showrunner RGB(125,60,255), unit-director RGB(108,108,0).
- The dark-theme chrome accent (`active_title`) is #FFD2A0, apart from claude1 brown, unit-director yellow and pink. Status bars: dark #3C3C3C; default light bar #FAFAFA with label #A06400; high-contrast light #8C3C00 on #D2D2D2.
- `WorkKind` decides type text, account and program: shell and subagent rows take the parent session's account and agent (`TreeLine::program`, `Work::account`); a Codex thread keeps its own account or the missing value.
- A subagent row's status shows its activity in worker words, read from its own transcript with the incremental Claude activity reader into `ChildRow.activity`; with no activity it shows the idle form.
- A command row named from its command drops leading `NAME=value` assignments (`AssignmentQuote` stack handles quotes, `$( )` and backticks); a command that is only assignments keeps its text.
- The summary's top-level table reads `pid account session status age desktop directory`, and drops a column when no listed row has a value in it (phase 24); it lists only sessions with a hierarchy; one placement decision feeds the summary and the unassigned tile, so no session is in both. The heading count counts listed sessions.
- A top-level Codex session shows its `/rename` name from the newest `thread_name` in its home's `session_index.jsonl` (`census/codex_names.rs`, `SessionNames`), else `pid <n>`; its fast mark (`<account> fast`) and its `idle` or activity words from its own rollout. Threads match a process by Codex home (`thread_matches_home`) and resume request (`ResumeRequest::{NewSession, NamedId, LastThread}`).
- A worker row needs a real dispatch record: roster resolution or an exact `CodexThread` dispatch.

**Files:**
- `crates/cargo-handler/src/footer_colors/mod.rs` — `FooterColors`, `FooterTint` (`Management`), `InkCoverage`, `ReadablePalette`
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/` — dark accent, status bar colors
- `crates/cargo-handler/src/render.rs` — title role suffix tint
- `crates/cargo-handler/src/agent_cell.rs` — `WorkKind` account, program and status rules
- `crates/cargo-handler/src/summary/mod.rs`, `summary/accounts.rs` — headers, membership, Roles spans
- `crates/cargo-handler/src/census/{classify,codex,codex_names,placement,scan,tree,transcript,activity,mod}.rs` — names, tiers, status, subagent activity, assignment stripping, worker-row rule
- Tests: `render/role_title_tests.rs`, `footer_colors/tests.rs`, `census/codex_session_tests.rs`, `summary/session_membership_tests.rs`

**Binds later work:** The `type` column reads the same `WorkKind` (Thread/Shell/Subagent) that `TreeLine::program` and `Work::account` match on. The accounts Roles column keeps its per-role spans. Summary membership (only sessions with a hierarchy; standalone sessions only in the unassigned tile) sets the row identity of each tile. An Astra thread named by an edge passes the same home match, and its role word takes a `FooterColors` role key.

**Gotchas:** Fixtures must inject `DesktopSource::Windows`; the system source runs a real KWin query (5 s). A raw 4.5:1 ink measures about 4.15 on thin antialiased glyph edges; measure management inks with `InkCoverage::Antialiased` on the composed ground, never on `theme.text.bg_focus` alone. An open rollout held by a Codex app-server once became a `ThreadEntry` with a fallback worker label.

**Ruled out:** red, green and account hues as role colors (they are footer level colors or read as accounts).

### Phase 17 — Seat dispatch rows, the type column and aligned ages  · status: done

#### As-built

- A shell call that only sends work to a seat draws no row. Its seat row (`WorkerIdentity::{CodexThread, ClaudeSession}`) stands for it and keeps its place in the tree. The fold matches an exact seat identity. It reads both `scripts/direct/` and `scripts/delegate/` launcher paths, `codex_mesh.py send`, literal `NAME=value;` bindings, `cd X &&`, env prefixes and `--to`/`--phase` in any order. Command rows that do other work (build, test, lint, script) still draw.
- Seat titles come from the opening of the dispatch prompt (`census/worker_dispatch/prompt_opening`), never from diff text.
- Tile trees have a `type` column to the right of `session`: `shell` for a command row, `subagent` for a subagent row, blank for seat, thread and session rows. The role column never says `subagent`. A subagent role (`ChildKind::Subagent(SubagentRole)`) comes from a non-general `.meta.json` agent type, else from a fixed description-word table. No readable role gives a blank cell.
- One age formatter in `summary/age.rs` serves the summary and every tile. A per-tile `AgeLayout` gives one right-aligned slot each for d, h, m and s, only for the units the column's rows use. It is measured after the shared screen-column fit (`App.tree_layouts`), so each unit letter sits in one screen column. All number columns are right-aligned under right-aligned headers. Each column is its widest value plus 2 spaces. Tiles in one screen column share column positions.
- Showrunner and unassigned tiles sort by `session` ascending at start. One resolver serves drawing and clicks, so the first header click reverses the drawn order.
- Session hues drop purple and yellow. Session, account and role inks reach 4.5:1 under `InkCoverage::Antialiased` (`readable_session_style`, `InkGrounds`). The light chrome accent moves off the claude4 orange.

**Files:**
- `crates/cargo-handler/src/census/worker_dispatch/` — dispatch command parsing, seat identity, prompt-opening titles
- `crates/cargo-handler/src/summary/age.rs` — shared age formatter and per-tile `AgeLayout`
- `crates/cargo-handler/src/agent_cell.rs` — `type` column, role cells, seat rows, default sort
- `crates/cargo-handler/src/footer_colors/mod.rs` — `InkGrounds`, `readable_session_style` with glyph coverage
- `crates/cargo-handler/src/census/probe.rs` — `ProbeChildWork` schema-3 subagent kind and role field
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/` — session hues, account inks, light chrome accent

**Binds later work:** Row selection uses the seat identity for seat rows that stand for folded dispatch commands. Age slots hold while a tile scrolls. Click mapping reads the drawn tree metrics. Probe schema 3 carries a subagent as unit kind `subagent` plus an optional `subagent_role`; `ReceivedChildKind` is gone. Session hues must clear status, account and role palettes by CIEDE2000 >= 12 under `Antialiased` coverage.

**Gotchas:** Solid-ink contrast passes where antialiased glyph edges fail; measure inks with `InkCoverage::Antialiased`. A new `ChildKind` payload on the wire breaks schema-3 receivers; carry new data in an optional field. The tile foot readout `content rows: N r/c: H/W` is a `tui_pane` feature, not debug text.

**Ruled out:** a brown status bar in cargo-handler; only the `tui_pane` fallback theme changes.

### Phase 22 — Shell rows name their kind once; session sort ignores case  · status: done

#### As-built

- A shell row says `shell` only in the type column. Its status column shows `running` in the agent Activity vocabulary and the agent running ink, or the timer pie and time in the `Role::Shell` ink.
- `agent_cell.rs` `ShellState::{Running, Timed(Timer)}` comes from the row's `Option<Timer>` and selects the status cell.
- `sort::compare_names` compares names without regard to case and breaks ties on the original spelling. Showrunner tiles, unassigned tiles and the summary all use it.
- Status inks solve at `InkCoverage::Antialiased` with `ANTIALIASED_GLYPH_COVERAGE` = 208/255, the same coverage as session hues.
- Shell role seeds: Default Light `Rgb(0, 120, 100)`, High Contrast Light `Rgb(0, 80, 65)`, Default Dark `Rgb(140, 230, 240)`. `themes/*.toml`, `starter.toml` included, mirror them, so the shell timer stays apart from every account ink.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `ShellState` and the shell status cell; tile sorts through `compare_names`.
- `crates/cargo-handler/src/sort.rs` — `compare_names`.
- `crates/cargo-handler/src/summary/mod.rs` — summary sort through `compare_names`; `summary/sort_tests.rs` covers it.
- `crates/cargo-handler/src/footer_colors/mod.rs` — `ANTIALIASED_GLYPH_COVERAGE`, `InkCoverage`, status inks at antialiased coverage.
- `crates/cargo-handler/src/footer_colors/account_seed_tests.rs` — shell and teal inks stay at CIEDE2000 >= 12 from every account ink after solving.
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/*.toml` — shell role seeds.
- `crates/cargo-handler/src/agent_cell/tests/` — shell `running` and timer rows, design inks, case-blind sort.

**Binds later work:** shell `running` and agent `running` share one ink, and only `ShellState::Timed` keeps `Role::Shell`. Contrast checks use the literal 208/255 coverage. An ink that is added or moved must be compared, after coverage solving, against every role and account ink.

**Gotchas:**
- Live xterm DejaVu stems measure about 216/255. At 229/255, light `running` and `waiting` fell to about 4.1:1 live.
- A darker solve moves hue-preserving inks together (shell and codex2). `account_seed_tests.rs` catches this.

**Ruled out:** Unicode special-case folding (Greek final sigma) in the session sort. Session names do not need it, and the order stays total and stable.

### Phase 23 — Timer at digit height; tiles drop empty columns; one Busy ink  · status: done

#### As-built

- The shell timer draws digits only (`timer_label` → `time_left_label`). DejaVu Sans Mono has no pie glyph at digit height, so no pie is drawn.
- `TreeColumns::for_lines` (`agent_cell.rs`) keeps a column only when a row of the tile has a value, `type` and `role` included. `VisibleTreeColumns` is that populated set, with `account` removed while accounts are hidden.
- `shared_tree_metrics` groups the tiles of one screen column by `(x, VisibleTreeColumns)` in one pass, merges their metrics, and builds one `AgeLayout` per group from the elapsed ages of all rows of every tile in the group. `TreeMetrics` carries `ages: AgeLayout` and has no `Eq`/`PartialEq`.
- `fill_tree_width` gives the row slack to `directory`, else to the first column (`session`).
- `render.rs` `prepare_frame` measures each tile once per frame and passes the elapsed ages. Headers, widths and click maps follow the drawn columns.
- `thinking` and `running` draw in one Busy ink in every theme through one solver.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — column sets, group metrics and age slots, slack, timer text.
- `crates/cargo-handler/src/render.rs` — per-frame measurement and the grouping input.
- `crates/cargo-handler/src/constants.rs` — timer constants without a pie list.
- `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs` — dropped columns, shared positions, header clicks.

**Binds later work:** the group `AgeLayout` and the populated columns come from the complete rows of every tile in the group, never a visible slice. Keep one fitter and one slack rule.

**Gotchas:**
- Every per-tile input to slack breaks the shared-column rule: slack comes from shared group numbers only.
- Live rows change between the dark and light shots, so column readings across themes are not comparable.

**Ruled out:** a fixed-point loop over drawn columns for grouping (it ends only by argument, not by type); a per-tile `AgeLayout` (each tile gets its own slack and same-set tiles misalign).

### Phase 24 — Role column fits its drawn values; a spent timer reads running  · status: done

**Depends on:** Phase 23

#### As-built

- Role width comes from the rows a tile paints. `TreeMeasurement::measure` (`agent_cell.rs`) counts only the tree rows that fit below `above_tree_rows(row, presentation) -> Vec<TreeHeadingRow>` (`TreeHeadingRow::{Heading, Place, Launcher(u32)}`, shared with drawing), and keeps one value per presentation in `PresentationValues<T>{full, compressed}`. The role column is never narrower than its header. Ages and populated columns still come from complete rows.
- During motion, tui_pane `TileCells::prepare_moving_frame(destination, motion_bounds)` (the default calls `prepare_frame`) and `TileGrid::motion_preparation_allocations` measure at the taller of the source and destination heights. cargo-handler `Cells` overrides the hook in `render.rs`; cargo-tile keeps the default.
- `ShellState::{Running, CountingDown(PositiveRemainingTime(NonZeroU64))}` comes from `ShellState::resolve(timer, now)` once per frame. `TreeLine::status(now)` gives `ResolvedStatus{text, role}`. A shell at or past its deadline reads `running` in the Busy ink; a shell with time left reads its digits in the Shell ink.
- Tiles of one screen column group by `PermittedTreeColumns` plus screen x. `GroupFitStep::ORDER` is `Drop(Directory)`, `Drop(Branch)`, `Drop(Type)`, `FinalMerge`. After each step, groups with equal fitted sets merge and share widths and age slots. The loop ends because the array ends.
- The summary top-level table drops a column that no listed row fills (`populated_columns`, `summary/mod.rs`). Headers, widths, rows, sort and clicks follow the drawn columns.
- The summary low weekly-quota red resolves through `footer_colors::readable_status_style` (dark #ff5050 at 6.52:1, light #cd0000 at 5.84:1).
- Short tiles keep Branch when it fits (CHANGELOG).

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — role measurement, timer state, column groups.
- `crates/cargo-handler/src/render.rs` — per-frame measurement, the motion preparation override.
- `crates/cargo-handler/src/summary/mod.rs` — populated summary columns.
- `crates/cargo-handler/src/summary/accounts.rs` — readable low-quota ink.
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs` — the motion preparation hook.
- `crates/cargo-handler/src/render/moving_role_tests.rs` — role width during motion.
- `crates/cargo-handler/src/agent_cell/tests/activity_tests.rs`, `column_visibility_tests.rs` — spent-timer cases, the cross-group case.

**Binds later work:** drawn column groups key on `PermittedTreeColumns` plus screen x and merge after each `GroupFitStep`; click mapping during motion and scrolling keep that group layout. A motion frame prepares through `TileCells::prepare_moving_frame` at the taller of the source and destination heights. Role width follows the painted rows. Summary click work maps through the drawn columns.

**Gotchas:**
- An above-tree `showrunner` row that the selected presentation does not paint set the 10-cell role width, because complete-row measurement counted it.
- An animated shrink clips a role unless motion has its own measure height.
- `render/accounts_grid_tests.rs` uses a fixed 80-cell terminal: the sort arrow adds 2 cells of real demand.

**Ruled out:** measuring role width from complete rows (it counts rows no presentation paints).

### Phase 18 — The shortcuts overlay pages only when it must  · status: done

#### As-built

- The global shortcuts overlay (`tui_pane/src/overlays/global_shortcuts.rs`, `popup_height`) has no height cap: it shows every shortcut on one page when the terminal has the rows, and pages with "n of m" only when it does not. cargo-tile, cargo-handler and cargo-port all use it.
- cargo-handler bar: the Default Dark accent is #00d5d5 and the Default Light bar is rgb 220,220,220, in `theme/mod.rs` and the matching `themes/*.toml`. Status bar contrast tests are registered in cargo-handler, cargo-tile, cargo-port and tui_pane; every drawn bar ink reads at 4.5:1 or more in all four themes.
- `InkCoverage::contrast` (`footer_colors/mod.rs`) returns the lower of the unrounded and the byte-rounded 208/255 covered contrast over all grounds, because a terminal can blend in whole bytes. The oracles in `footer_colors/tests.rs`, `render/selected_ink_tests.rs`, `summary/accounts_tests.rs` and `render/role_title_tests.rs` measure the same two ways with their own literal coverage. The light session ink moved from 0,35,86 to 0,34,85 to keep the phase 26 distance floor (dE 15.21, chroma 0.1008).
- `ThemeRegistry::from_dir_with_parser` takes `ThemeSource { BuiltinsOnly, Directory(&Path) }`; `from_dir_with_builtins` keeps its public signature.
- Handler `install_theme` returns `ThemeInstallation` (the active theme found, or a missing name with the built-in used, and each rejected file with its reason). `terminal::startup_theme_notes` builds `StartupThemeNotes` (the installation and the `AppearanceDiagnostic`) once at startup; `App::new(LoadedConfig, StartupThemeNotes)` stores it, and appearance answers and settings keep the rejected-file note. `Option<String>` appears only at the tui_pane boundaries.

**Files:**
- `crates/tui_pane/src/overlays/global_shortcuts.rs`, `overlays/global_shortcuts_tests.rs` — overlay height and paging
- `crates/tui_pane/src/theme/loader.rs`, `theme/fallback.rs`, `bar/status_bar_contrast_support.rs` — `ThemeSource`, bar grounds, shared contrast support
- `crates/cargo-handler/src/footer_colors/mod.rs` — two-way covered contrast
- `crates/cargo-handler/src/theme/files.rs`, `terminal.rs`, `app.rs`, `settings.rs` — installation result and startup notes
- `crates/cargo-handler/src/terminal/theme_note_tests.rs` — startup-note tests through the production startup step

**Gotchas:** measure covered contrast both ways and take the lower. `App::new_for_test_with_startup` shares the production field construction.

**Ruled out:** unrounded-only coverage (a byte blend dropped an ink under 4.5:1).

### Phase 25 — Shared chrome text and overlay polish  · status: todo

**Depends on:** Phase 18

#### Work Order

**Goal:** the global shortcuts overlay and the status bars of cargo-handler and cargo-port use one style of words, case, key names and spacing, and a selected cargo-port row stays readable.

**Source:** the phase 18 design check (2026-10-22) found these rows on main. Showrunner ruling, 2026-10-22: they stay out of phase 18 and go into this phase, next after phase 18, with the rulings below, so the phase needs no further questions. Design check row 8 (the cargo-tile bar hint `p process_tree`) goes to the tile unit. No UX guide exists for these apps, so these rulings are the rules.

**Constraints from prior phases:**
- Phase 18: the overlay draws in `crates/tui_pane/src/overlays/global_shortcuts.rs` (`build_lines`, `header_line`, `row_line`, `popup_height`). The page count comes from the content line count through `ViewportOverflow`; a blank row is frame padding below the last action and drops first. The selected row draws its marker and description in the text default ink.
- Section names come from `crates/tui_pane/src/keymap/mod.rs` (`"Global Navigation"` ≈460, ≈545; `"Global Shortcuts"` ≈475, ≈550), which feeds both the overlay and the keymap viewer.
- Key names come from `crates/tui_pane/src/keymap/key_bind.rs` (`KeyCode::Esc => "Esc"` ≈236). The bar's paired `nav` label is `crates/tui_pane/src/bar/nav_region.rs` ≈52.
- cargo-port draws its sccache value in `crates/cargo-port/src/tui/sccache/status_line.rs` ≈59 (`"{cache_size} - hit rate {hit_rate}"`); cargo-tile has `SCCACHE_HIT_RATE_WORD` (`crates/cargo-tile/src/constants.rs` ≈938). cargo-tile files are the tile unit's; send an overlap to the showrunner.
- Phase 16 binds up/down in cargo-handler to the row selection, so "Focus the tile above" and "Focus the tile below" show no key in its overlay.
- `theme::ResetRank::color` (`crates/cargo-handler/src/theme/mod.rs` ≈125) stays the one reset source. The readability correction applies after it. The failed-reading dot keeps `tui_pane::error_color()`. The shared lift below must not add a reset source.
- The phase 16 lift today: `SelectedGroundInk::resolve(color, ground)` (`crates/cargo-handler/src/footer_colors/mod.rs` ≈421) reads and fills the handler palette cache `PALETTE` (`ReadablePalette.selected_inks` ≈185). `ReadablePalette::current` (≈233) rebuilds the cache when the theme identity (`Arc::ptr_eq`) or the grounds (`InkGrounds::current`) change. The calculation is `lightness_for` with `LightnessTarget::Selected(ground)` (≈501): an OKLCH lightness search along `SelectedHueChroma` (≈610) through `search_lightness` (≈547), measured by `InkCoverage::Antialiased` on that one ground. Phase 18: `InkCoverage::contrast` (≈367) returns the lower of the unrounded and the byte-rounded 208/255 covered contrast over all grounds (a terminal can blend in whole bytes); the handler test oracles use the same two-way minimum. `Oklch::from_color` (≈615) uses `SelectedHueChroma` directly, and `render/selected_ink_tests.rs` and `footer_colors::tests::oklch_ink` read that OKLCH measurement. `search_lightness` (≈575) returns its first candidate when no end of the search reaches the target: on a mid ground such as rgb 127,127,127 even black (≈4.46:1) and white (≈3.24:1) stay under 4.5:1 with the two-way minimum. A session row draws its inks through `SessionRowInk::style()` (`theme/mod.rs` ≈58).
- cargo-port draws the selected project row style in `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` (`tui_pane::selection_state_for(...).overlay_style()` ≈1229); its inline `mod tests` is at ≈1441.

**Spec:**
1. The overlay's selection bar spans the full inner width of the popup on every row.
2. The title and the section headings use sentence case with no trailing colon: "Global shortcuts", "Global navigation".
3. The title shows once: the section heading that repeats the title is removed.
4. One blank row stands before each section heading after the first. The fit and page height count it.
5. "Focus the tile above" and "Focus the tile below" show their bound key. A row with no bound key is left out of the overlay.
6. cargo-handler says "tile" everywhere, the framework navigation entries and bar labels included ("Next tile", "tab tile").
7. Key names use one form: lower case, with the spellings the bar already uses ("esc", "enter", "pageup", "shift-tab"). The keymap file keeps accepting its current spellings.
8. The cargo-port bar uses the same verbs as the cargo-tile and cargo-handler bars: "↑/↓ move", and "=/- expand all" / "collapse all" or the shared equivalent.
9. One shared helper formats the sccache hit-rate percent: one decimal, no space before the sign ("85.4%"). It lives in `crates/tui_pane/src/format.rs`, exported as `tui_pane::format_hit_rate(raw: &str) -> Result<String, MalformedRate>` beside `format_progressive` (`lib.rs` ≈292). Both parsers call it: cargo-port `src/sccache.rs` `summary_from_lines` (≈38; it keeps the raw string today) and cargo-tile `src/sccache.rs` `closed_percent` (≈368; it removes spaces with no rounding). Malformed input (no finite number) gives `MalformedRate`, and each parser treats it as a missing rate field (`SccacheSummary::Unavailable`). Tests: "85.40 %" gives "85.4%", "4.90 %" gives "4.9%", "100 %" gives "100.0%", and "n/a" gives `MalformedRate`. The cargo-port selected-ink repair (item 10) is owned by `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` (the selected row style ≈1229), with the phase 16 lift moved from cargo-handler `footer_colors/mod.rs` (`SelectedGroundInk` ≈410) into tui_pane so both apps use one method.
10. cargo-port's selected pane row is readable on the Default Dark selection ground rgb 86,86,86 (showrunner, 2026-10-23). Live at phase 18: CI circle green (5,197,5) 2.55:1, Og glyph gray (162,162,162) 2.42:1, Main ☑ green (21,175,21) 2.10:1, disk text (255,212,212) 4.25:1. Use the phase 16 method: an OKLCH lightness-only lift of each ink until it reaches 4.5:1 at 208/255 coverage on the selection ground, with its hue kept and no colour-distance rule on selection grounds. Emoji glyphs (the Git crab) are exempt, because their colour is not ours to set. Shared lift boundary: the shared implementation is `crates/tui_pane/src/theme/selected_ink.rs`, exported from `theme/mod.rs` and `lib.rs`. Move only the selected-ground colour calculation into it: the OKLCH lightness search for one selection ground, the hue and chroma path it follows, the OKLCH conversion it uses (`Oklch::from_color` and `SelectedHueChroma`, exported as the one measurement API; cargo-handler's `render/selected_ink_tests.rs` and `footer_colors::tests::oklch_ink` move to it, and no second conversion stays in cargo-handler), and the two-way 208/255 covered contrast it measures with (the lower of the unrounded and the byte-rounded blend, as phase 18 built). It is a pure function of the ink and the ground, with no cache and no theme state. cargo-handler's `InkCoverage::Antialiased` calls the same covered-contrast function, so there is one covered-contrast calculation. The resolver names its outcome by type: a lifted ink that reaches 4.5:1, or an unreachable target that carries the best ink it found (the search end with the higher two-way contrast). It never returns an unmeasured first candidate. Callers draw the carried best ink for an unreachable target; no built-in theme ground is unreachable today, and a test proves that for every built-in selection ground. Cache invalidation stays with each app: cargo-handler keeps `SelectedGroundInk` as a wrapper over the shared function, with its `PALETTE` cache and the theme-identity and ground checks in `ReadablePalette::current`; cargo-port calls the shared function and owns any cache it adds. Keep `SessionRowInk`, the footer assignments (`FooterColors`), the pane lightness (`ReadableInk`) and the reset ranking (`ResetRank`) in cargo-handler. Unselected session inks do not change.
11. Tests, each red before its fix: a rendered overlay has a full-width selection bar, sentence-case title and headings without colons, no repeated title heading, a blank row before each later heading (fit and paging heights counted), and no row without a key; the rendered cargo-handler bar and overlay say "tile" and never "pane"; every key name drawn on a bar or the overlay is lower case; the cargo-port bar draws "move" and the expand/collapse verbs; the sccache helper formats 85.40 as "85.4%". A rendered cargo-port pane with a selected row reads every non-emoji ink on that row at 4.5:1 or more with its own literal 208/255 coverage, in all four themes (named test `selected_project_row_inks_read_at_4_5_in_every_theme` in `crates/cargo-port/src/tui/panes/project_list/selected_row_ink_tests.rs`). The shared resolver has its own tests in `crates/tui_pane/src/theme/selected_ink_tests.rs`: an ink under 4.5:1 lifts to 4.5:1 with its hue kept, an ink already at 4.5:1 stays unchanged, and on ground rgb 127,127,127 the resolver gives the unreachable outcome with the end of higher two-way contrast. cargo-handler's `render/selected_ink_tests.rs` passes unchanged, and an unselected session row draws the same inks as before.
12. Slow test (showrunner, 2026-10-24): `census::scan::codex_session_tests::codex_resume_last_uses_the_newest_older_thread_in_its_home_and_directory` (`crates/cargo-handler/src/census/codex_session_tests.rs` ≈409) took 1.510 s in the phase 30 full cargo-handler suite at high machine load. Time it alone first. If it is over a second alone, find the cause and fix it so it runs under a second; write the measured times and the cause in the summary. If it is under a second alone, write the times and make no change.

**Files:**
- `crates/tui_pane/src/overlays/global_shortcuts.rs`, `crates/tui_pane/src/overlays/global_shortcuts_tests.rs`, `crates/tui_pane/src/keymap/mod.rs`, `crates/tui_pane/src/keymap/key_bind.rs`, `crates/tui_pane/src/bar/nav_region.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/keymap.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-port/src/tui/sccache/status_line.rs`, `crates/cargo-port/src/sccache.rs`, `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs`, `crates/cargo-port/src/tui/keymap/`, `crates/cargo-port/src/themes/`, `crates/cargo-port/CHANGELOG.md`
- `crates/tui_pane/src/format.rs`, `crates/tui_pane/src/lib.rs`, `crates/cargo-tile/src/sccache.rs`, `crates/cargo-handler/src/footer_colors/mod.rs`
- `crates/tui_pane/src/theme/selected_ink.rs` — new shared selected-ground resolver, exported from `crates/tui_pane/src/theme/mod.rs`
- `crates/tui_pane/src/theme/selected_ink_tests.rs` — new resolver tests, registered in `theme/selected_ink.rs`
- `crates/cargo-handler/src/render/selected_ink_tests.rs` — the handler selected-ink tests, unchanged expectations
- `crates/cargo-port/src/tui/panes/project_list/selected_row_ink_tests.rs` — new cargo-port selected-row render test, registered in `tree_rows.rs`
- `crates/cargo-handler/src/census/codex_session_tests.rs` — the census slow test (spec 12)
- `crates/cargo-handler/src/footer_colors/tests.rs` — `oklch_ink` moves to the shared measurement API

**Workers:** 1 writer + 1 tester.
- `impl` — overlay, keymap, bar, the shared formatter, both sccache callers, the shared resolver module (`theme/selected_ink.rs`) and its exports, the application callers (cargo-handler `SelectedGroundInk`, cargo-port `tree_rows.rs`), the cache integration in `footer_colors/mod.rs`, the registrations and the CHANGELOGs; hub: `crates/tui_pane/src/lib.rs` and `crates/tui_pane/src/theme/mod.rs` (exports).
- `test` — the external rendered tests, the separate resolver tests (`theme/selected_ink_tests.rs`), the handler selected-ink tests (`render/selected_ink_tests.rs`), the named cargo-port selected-row render test (`selected_row_ink_tests.rs`), the move of `footer_colors::tests::oklch_ink` and the selected-ink tests to the shared measurement API, and sole owner of `crates/cargo-handler/src/census/codex_session_tests.rs` for the census slow test.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`, `bash ~/.claude/scripts/direct/verify.sh test cargo-port`, `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` clean. No test takes a second. Live: framed shots of the overlay in cargo-handler and cargo-tile, and of the cargo-handler and cargo-port bars, in a dark and a light theme. The checkpoint carries `Change-Review: the shortcuts overlay and the status bars use sentence case, lower-case key names, one word for tiles in cargo-handler and one sccache percent format | open the global shortcuts overlay in cargo-handler and cargo-tile, and read the cargo-handler and cargo-port bars`.

### Phase 19 — Accounts table matches the accounts board  · status: done

#### As-built

- The accounts table draws the same columns as `/accounts-board`, in this order: a read dot with no header, `account`, `role`, `left`, `resets`, `credits`, `enabled`, `in use`, `read`. All headers are lower case. The dot (`ACCOUNT_READ_DOT`) is green for a good read and red for a failed or missing read. The account cell holds only the name, and the reading age (`45s`, `12m`, `2h`, `3d`) is in `read`. `left`, `resets`, `credits` and `read` are right-aligned.
- `census/accounts.rs` converts the `accounts.py --json` fields into named states at the wire boundary: `AccountReadStatus::{Ok, Failed}` from `read_ok`, `CreditPermission::{Enabled, Disabled}` from `use_credits`, `CreditActivity::{InUse, Idle}` from `credits_in_use`. A missing `read_ok` gives `Failed`, and a missing `credits_in_use` gives `Idle`.
- `sort::AccountsColumn` has eight variants (`Account`, `Roles`, `WeeklyLeft`, `Resets`, `Credits`, `Enabled`, `InUse`, `Read`). A header click sorts by any of them, with blank values last.
- `AccountsLayout::{Full, TwoLine, Stacked}` (`AccountsLayout::for_width`) selects one layout from the measured width. `account_name_indent()` is the one prefix source for measurement and drawing, so width demand, wrap and height agree with the drawn rows.
- The band of the selected account covers its full visible span: `SurfaceSelection::selected_span() -> Option<RowSpan>`, clipped by `RowSpan::visible_lines(offset, height) -> Range<usize>`.
- Each theme file holds `session_hues`, exactly five saturated colors (`[Color; 5]`; a different count is a deserialize error). `SessionHue::{First..Fifth}` names an entry, and `SessionHue::of_agent(position)` starts again after the fifth. Theme loading uses `tui_pane` `ThemeRegistry::from_dir_with_parser(dir, builtins, parse)`, which gives shared directory loading, failed-file records and built-in role inheritance.

**Files:**
- `crates/cargo-handler/src/summary/accounts.rs` — account columns, layouts, header sort, cell formats
- `crates/cargo-handler/src/summary/constants.rs` — `ACCOUNT_READ_DOT`
- `crates/cargo-handler/src/census/accounts.rs` — account wire data and its named states
- `crates/cargo-handler/src/sort.rs` — `AccountsColumn`
- `crates/cargo-handler/src/interaction.rs` — `RowSpan::visible_lines`, `SurfaceSelection::selected_span`
- `crates/cargo-handler/src/render/mod.rs` — span bands
- `crates/cargo-handler/src/theme/mod.rs`, `crates/cargo-handler/src/theme/files.rs` — handler themes, `SessionHue`, theme file parse
- `crates/tui_pane/src/theme/loader.rs` — `ThemeRegistry::from_dir_with_parser`
- `crates/cargo-handler/src/census/accounts_wire_tests.rs`, `crates/cargo-handler/src/render/status_bar_contrast_tests.rs` — wire-state and contrast tests

**Binds later work:** `AccountsLayout` is selected from the measured width, and `account_name_indent()` with `ACCOUNT_READ_DOT` is the one prefix source; narrow-width work keeps both. In TwoLine, line 2 and the second header line start under the account name, and the `resets` header is left-aligned in its cell while body values stay right-aligned. Cell meanings: a known past reset draws weekday/time or date text, an unavailable reset draws blank, an unavailable quota draws `—`. `ThemeRegistry::from_dir_with_parser` records rejected theme files in `failed_files`, but cargo-handler shows no report for them.

**Gotchas:** The `census/catch_up_hierarchy_tests` wait on each fixture event with `recv_timeout(HANG_CEILING)` (5 s). This limit stops only a blocked test, so the result does not depend on machine speed; do not set a short wall-clock wait again.

**Ruled out:** A new sortable header for the Stacked layout.

### Phase 26 — Accounts table and session colour polish  · status: done

#### As-built

- The reset ramp does not end on `error_color()`. `ResetRank::color` blends two theme roles, `reset_nearest` to `reset_farthest` (`RolePalette`). Dark themes: (125,191,110) to (69,196,153). Light themes: violet (136,35,253) to indigo (28,3,61). The failed-reading dot keeps `error_color()`.
- Root session hues: `cell_order_placed` (`agent_cell.rs`) numbers the roots with `enumerate`, and a separate descendant counter goes to `lines_of`. Root i takes entry i of `session_hues` (mod 5), whatever the size of the subtrees. Each tile title keeps its summary row's ink.
- `theme::SessionRowInk` names each ink that a session row can draw: `Text`, `LauncherLabel`, `Claude`, `Codex`, `Busy`, `Shell`, `Idle`, `Account(FooterColor)`, `Management(FooterColor)`. `SessionRowInk::style()` is the one draw path, and `FooterColor::account_style`, `FooterColor::management_style` and `FooterTint::style` go through it. The tests get the same list from `SessionRowInk::fixed_fields()` (an exhaustive match) and `FooterColors::session_row_inks()`.
- New light `session_hues`. Default Light: (70,1,2) (87,39,90) (0,35,86) (110,6,54) (33,0,69). High Contrast Light: (0,35,86) (68,0,29) (37,0,67) (81,12,0) (92,40,87).
- Measured on the production palette output (the phase summary has the full table):
  - Light reset ramp, both light themes: chroma >= 0.10, dE >= 23 from `Text`, dE >= 42 from error red, >= 30.4° in hue from every account ink, contrast >= 4.500:1.
  - Light session inks: chroma >= 0.10, session-to-session dE about 13.6 (Default Light) and 14.3 (High Contrast Light).
  - The nearest light reset shade has 4.485:1 covered contrast before rounding. The unrounded covered-contrast work in "The shortcuts overlay pages only when it must" owns that gap.
  - Known defect: the far light reset shade (28,3,61) is dE 2.40 from Default Light session ink (33,0,69) and dE 2.57 from High Contrast Light session ink (37,0,67), which breaks the rule that a session colour keeps dE >= 20 from every ink within 45° of hue. The session palette checks leave reset inks out. The phase "Light session and reset inks stay apart" repairs it.
- Dark themes keep their current `session_hues`. On dark, hue alone cannot give five session inks 60° apart that also keep dE >= 20 from the row inks: the valid hue domain is about 92.5°, and at most 2–3 inks can be dE >= 20 apart. Steps of 45°, C >= 0.10, and both together also fail. Three dark tests hold the current dark list at its measured floors and name the next item "Session colours stay apart".
- Pinned headers and named selection results are not in this phase; the phase "Accounts headers stay in view" owns them.

**Files:**
- `crates/cargo-handler/src/theme/mod.rs` — `SessionRowInk`, `ResetRank::color`, `reset_nearest` / `reset_farthest`
- `crates/cargo-handler/src/agent_cell.rs` — root ordinal hues
- `crates/cargo-handler/src/footer_colors/mod.rs` — account and management styles through `SessionRowInk`
- `crates/cargo-handler/themes/default_dark.toml`, `default_light.toml`, `high_contrast.toml`, `starter.toml` — reset roles, light `session_hues`
- `crates/cargo-handler/src/render/role_title_tests.rs`, `crates/cargo-handler/src/summary/accounts_gradient_tests.rs`, `crates/cargo-handler/src/footer_colors/tests.rs`, `crates/cargo-handler/src/render/selected_ink_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/sort_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/unassigned_tests.rs`, `crates/cargo-handler/src/summary/unassigned_summary_tests.rs`, `crates/cargo-handler/src/summary/mod.rs` — colour tests and updated expectations

**Binds later work:** a session row draws only through `SessionRowInk::style()`; a new ink on that row is a new `SessionRowInk` variant, so the tests see it. The reset ramp comes only from the two reset roles; no reset shade is `error_color()` or a `left` warning ink.

**Ruled out:** a dark session list that meets the hue-spacing and dE >= 20 rules by hue alone (measured infeasible; see the next item "Session colours stay apart").

### Phase 29 — Light session and reset inks stay apart  · status: done

#### As-built

- New light reset ends, in `DEFAULT_LIGHT_ROLES` and `HIGH_CONTRAST_LIGHT_ROLES` (`theme/mod.rs`), `themes/default_light.toml` and the light variant of `themes/high_contrast.toml`. Default Light: (115,0,235) to (73,0,158). High Contrast Light: (115,0,235) to (82,55,154). The drawn reset shades fall in one violet band (OKLCH h 291–294°). Session inks and all dark values do not change.
- The light ends maximise the smallest session-vs-reset CIEDE2000 (pairs within 45° of hue, all 256 drawn ranks, both pane grounds, after the readability lift), under showrunner ruling 3 (2026-10-23): accept >= 10 and hold the achieved value. Achieved: 14.706 (Default Light, session (33,0,69) vs the far end) and 15.303 (High Contrast Light, session (37,0,67)). Before: 2.40 and 2.57.
- The reset floors that bind: ramp-end separation dE >= 12, >= 30° in hue from every account ink, and covered contrast >= 4.5:1. The other reset floors hold with margin (C >= 0.15, dE >= 33 from `Text`, dE >= 41 from error red). The nearest shade is 4.51:1 unrounded.
- dE 20 is out of reach on light themes too. With session inks fixed, no pair of reset ends gets there. Light session inks also miss dE 20 against status and account inks (15–19). The next item "Session colours stay apart" now covers both appearances.
- Tests (`render/role_title_tests.rs`): `SessionPaletteFloors` carries a `SessionResetFloor` (`DarkNotMeasured`, `HeldLight(f64)`). For a light theme, `check_reset_shades` measures every session ink against the 256 drawn reset shades (`ResetRank::positions`, `ResetRank::color`, `footer_colors::readable_status_style`) within `NEAR_HUE_GAP_DEGREES`, and holds 14.7 (Default Light) and 15.3 (High Contrast Light). `session_glyph_contrast` now uses unrounded covered channels (208/255 stem).

**Files:**
- `crates/cargo-handler/src/theme/mod.rs` — light reset ends
- `crates/cargo-handler/themes/default_light.toml`, `crates/cargo-handler/themes/high_contrast.toml` — the same ends
- `crates/cargo-handler/src/render/role_title_tests.rs` — session vs reset checks, `SessionResetFloor`, unrounded glyph oracle
- `crates/cargo-handler/CHANGELOG.md`

**Binds later work:** a new light ink on a session row must keep apart from the violet reset band (h 291–294°). Raising the held 14.7 / 15.3 floors needs a second visual channel or fewer row inks (next item "Session colours stay apart").

**Ruled out:** light reset ends that reach session-vs-reset dE 20 with the session inks fixed (measured: none on the searched grid and local RGB neighbourhoods), and a one-session swap per theme.

### Phase 30 — Light session inks and reset ramp read apart  · status: done

#### As-built

- No palette change. Every light value stays as phase 29 left it. Showrunner rulings, 2026-10-24: measure four relaxations, each alone, then (c)+(a); land a palette only if the smallest session-pair dE is >= 12 and the ramp ends are dE >= 20 in both light themes; if none passes, land only the ramp-order test and the CHANGELOG line, and stop tuning floors.
- Measured (all floors kept except the one relaxed):

  | Relaxation | Five-ink session set | Largest reset-end dE (Default Light / High Contrast Light) |
  | --- | --- | --- |
  | (a) L 0.35–0.60 | none | 8.3 / 6.9 |
  | (b) row-fixed dE 12 | none | 8.3 / 6.9 |
  | (c) hue gap 45° | none | 8.3 / 6.9 |
  | (d) reset vs role dE 12 | none | 12.0 / 18.2 |
  | (c)+(a) | none | 8.3 / 6.9 |

- Why no session set exists: under the light floors (C >= 0.14, covered contrast >= 4.5:1 unrounded, row-fixed dE, clear of the reset violet band), the final High Contrast Light hues fit in an arc of 48–116°, which is less than twice the required gap. Five inks that step that gap around the cycle cannot fit in it. An exhaustive scan of all 16,777,216 final colours proves it.
- The ramp order is now tested: `summary/accounts_gradient_tests.rs` draws six distinct reset times, one in the past, in two source orders on every built-in theme. Rank 0 (the earliest time) takes `reset_nearest`, the last rank takes `reset_farthest`, and the ranks between are evenly spaced.
- CHANGELOG: reset shades follow time order. On dark themes they go from yellow-green (soonest) to sea green; on light themes a brighter violet means a sooner reset. The dark ends have the same lightness, so on dark themes the order is in hue only.

**Files:**
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — the six-time ramp-order test
- `crates/cargo-handler/CHANGELOG.md`

**Binds later work:** the open light defects (near-black session inks in two hue groups, the `showrunners` role ink dE 5.3 from the nearest reset end, reset steps of dE 2.7–3.4) move to the phase "Session colours stay apart". More floor tuning on one row cannot fix them; that phase changes what a session row draws.

**Ruled out:** each relaxation (a)–(d) and (c)+(a), as measured above. Evidence: the phase 30 measurement programs and logs (`relax_measure.py`, `relax_domain.cpp`, `relax_sessions.cpp`).

### Phase 28 — Accounts headers stay in view  · status: done

#### As-built

- `SurfaceSelection` owns the body offset and `SectionPin`. One painted `ViewportMapping` serves drawing, selection bands and row clicks, so the accounts heading and its column-header lines (Full one, TwoLine two, Stacked none) stay at the top while the body scrolls. Navigation and page height use the body height below the pinned lines. A pinned line never takes the band. A pinned header click sorts; a pinned heading click selects no row.
- Named results replace the bare `Option` results: `SelectableSurface {EmptyCell, Surface}`, `RowLocation {Gone, Found}`, `SurfaceLookup {Retired, Current}`, `SelectedSpan {NoRows, Row}`.
- `tui_pane` tiles settings have `SummaryRowAllocation {Shared, BeforeTiles}` (default `Shared`; set with `TileGrid::set_summary_row_allocation`). `BeforeTiles` runs the shared split first. Then it reserves the summary's need before the tiles only when the summary's share is short.
- Clicks resolve against the last painted frame: `PaintedCells {Undrawn, Drawn}`, one stable `CellIdentity` per painted piece, and `cell_hit` gives `CellHit {Missed, Retired, Current}`. `TilePick::RetiredCell` is absorbed before focus or row selection, so a click on a departing tile never focuses its replacement. `content_click` maps a painted frame to source coordinates for presses and wheel steps. Before the first paint, the layout path stays.
- `render::cell_clicked` checks header hits before the surface lookup. `Cells::paint_selection` clips each band row to the tile and buffer, so a moving tile cannot index outside the buffer.

**Files:**
- `crates/cargo-handler/src/interaction.rs` — named selection results, pinned mapping
- `crates/cargo-handler/src/render/mod.rs` — header hits, selection paint, `BeforeTiles` request
- `crates/cargo-handler/src/summary/mod.rs` — `SummaryLayout::section_pin`
- `crates/tui_pane/src/tiles/{grid.rs, draw.rs, host.rs, settings.rs, mod.rs}`, `crates/tui_pane/src/lib.rs` — summary allocation and painted hits
- `crates/cargo-handler/src/render/pinned_accounts_tests.rs` (new), `crates/cargo-handler/src/render/accounts_grid_tests.rs`, `crates/tui_pane/src/tiles/summary_allocation_tests.rs` (new)
- `crates/cargo-handler/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`

**Binds later work:** `handle_tile_click`, click and scroll answer from the painted frame after the first paint; `cell_at` still answers from settled geometry, and focus following by identity is open for "Accurate clicks during motion". Title-press forwarding builds on `content_click` and `TilePick::RetiredCell`. cargo-handler sets `SummaryRowAllocation::BeforeTiles` once at grid construction. At 34x50 cells view with about 20 tiles, the summary gets an 8-cell column, only 3 of 6 accounts draw, and session rows with no drawable column leave blank rows; "Narrow tiles keep their core columns" owns this gap.

**Gotchas:** A test-only adapter may not call `panic!` (`clippy::panic` denies it). Use `Option::expect` under the crate's `expect_used` expectation.

**Ruled out:** a lint exception for `clippy::panic` (the `expect` form is allowed).

### Phase 32 — Accounts table keeps main's height  · status: done

#### As-built

- `AccountsFit {columns, layout}` is built once per frame and feeds the owned `AccountLines`. The fit drops whole optional columns with their headers in this order: in use, enabled, read, credits. Then it cuts roles with "…" to a 6-cell floor. Then it chooses `TwoLine`, then `Stacked`. Account, roles, weekly left and resets always stay.
- Height, draw, header clicks and selection all read the same fit. No reader has a second chooser.
- `wrap_line` packs whole words. A separator stays with the word before it, so no wrapped line starts with punctuation. A number is never split.
- A cut summary title (for example "natedev · 4") goes through `truncated_line` and ends with "…".
- `SummaryLayout` is built per frame. `App` keeps `PaintedSummary::{NotDrawn, Drawn}`, and the accounts header click reads it.
- At 200, 80, 50 and 40 columns the summary has main's height, draws every account and the error line, and leaves the tiles equal to main.

**Files:**
- `crates/cargo-handler/src/summary/accounts.rs` — accounts fit, drop order, word wrap and lines
- `crates/cargo-handler/src/summary/mod.rs` — summary layout and title cut
- `crates/cargo-handler/src/render/mod.rs` — per-frame summary layout
- `crates/cargo-handler/src/app.rs` — `PaintedSummary`
- `crates/cargo-handler/src/sort.rs` — accounts sort reads the painted lines
- `crates/cargo-handler/src/summary/accounts_tests.rs` — four-account fixture with an error line at each width

**Binds later work:** `AccountsFit {columns, layout}` per frame feeds `AccountLines`; `SummaryLayout` per frame; `App`'s `PaintedSummary::{NotDrawn, Drawn}` is read by the accounts header click; `truncated_line` cuts titles; drop order is in use, enabled, read, credits, then roles cut with "…" to a 6-cell floor, then `TwoLine`, then `Stacked`. The summary gets no rows first in the grid; summary-first row allocation belongs to "Accounts headers stay in view", and the grid minimum tile width to "Narrow tiles keep their core columns".

**Gotchas:** A grid minimum tile width (`set_min_tile_width`) without summary-first rows starves the summary at 50 and 40 columns: tiles stack to one column, 3 of 5 tiles stay in view, and the summary cuts to 12 rows.

**Ruled out:** a grid minimum tile width in this work (it starves the summary); a status-scan core width for tiles (tiles became worse than main).

### Phase 33 — Every settings row you can scroll to, you can select  · status: done

#### As-built

- cargo-port's settings overlay uses the shared settings scroll, navigation and selection. It adds no padding after `render_rows` and keeps no scroll offset of its own, so the painted rows and the click targets match.
- `SettingsPane::change_selection` (`tui_pane/src/overlays/settings.rs`) is the one row-change path for `select_row`, navigation and row-count clamps. A row change ends an open edit, so a wheel move, a key or a forced selection cannot save the edit text into another setting.
- `SettingsPane` keeps its `Viewport` private: `viewport()` is read-only, and `set_content_area`, `set_scroll_offset` and `set_hovered` set layout and hover. No caller outside tui_pane can change the selected row or the row count.
- `SettingsHost::settings_row_count` is required. `SettingsNavigation::dispatcher` sets the count before movement, so a key pressed before the first draw reaches every row. `SettingsRows` gives the selectable count in O(1).
- cargo-port `force_settings_if_unconfigured` keeps settings that are already open open, and selects IncludeDirs with `select_row`.

**Files:**
- `crates/tui_pane/src/overlays/settings.rs` — settings pane, row-change path, layout setters
- `crates/tui_pane/src/app_settings/navigation.rs`, `crates/tui_pane/src/app_settings/rows.rs`, `crates/tui_pane/src/app_settings/draw.rs` — row count before movement; drawing through the layout setters
- `crates/cargo-port/src/tui/settings.rs`, `crates/cargo-port/src/tui/input/dispatch.rs`, `crates/cargo-port/src/tui/interaction.rs`, `crates/cargo-port/src/tui/app/mod.rs` — cargo-port adapters on the shared path
- `crates/cargo-tile/src/app.rs`, `crates/cargo-handler/src/app.rs` — `settings_row_count`
- `crates/tui_pane/src/overlays/settings_scroll_tests.rs` and `settings_scroll_tests.rs` in cargo-port, cargo-tile and cargo-handler — regressions through each real settings dispatcher; `crates/tui_pane/tests/settings_scroll.rs` uses the public API

**Gotchas:** `GlobalAction::OpenSettings` toggles; a caller that must keep settings open checks first. In cargo-port the wheel moves the selection, not the view.

**Ruled out:** per-call-site `select_row` fixes without removing `viewport_mut` (the type removal closes the whole class).

### Phase 27 — Narrow tiles keep their core columns  · status: done

#### As-built

- `TreeMeasurement` (`agent_cell.rs`) owns the per-tile source measurements for one frame. `TreeMeasurement::minimum_tile_width` gives the tile core width: the session header with its lead, the role header and widest role, the status header and widest drawn status word, the gaps and the frame. cargo-handler gives it to the grid through `set_min_tile_width` every frame, before layout. `measure_trees` projects each saved selection through `TreeMeasurement::allocated` at `frame_inner(grid area)` (the tallest body) before it reads the minimum. `Cells::with_selections` and `prepare_frame` call `restore_tree_viewports` before source measurement.
- In `tui_pane`, `ColumnLayout` is the one topology chooser. It picks from a finite set of width-checked column counts. `ColumnPlan::{Columns(ColumnLayout), NoAdmissibleColumn}` feeds `fits`, `holds_in` and the display. A one-column plan uses the full width. While a core-width tile fits, width alone does not switch the cells view to summary-only. The plan sets the summary width before `SummaryRowAllocation::BeforeTiles` assigns rows.
- `TileWidthFloor::{Required, CompressSingleColumn}` (`tiles/settings.rs`; set with `TileGrid::set_tile_width_floor`; re-exported from `tiles/mod.rs` and `lib.rs`). Generic grids keep `Required`, and `NoAdmissibleColumn` shows the summary alone. cargo-handler selects `CompressSingleColumn`, so a terminal below the core width keeps one column of tiles.
- `TreeMetrics` owns the final group fit: `TreePresentation::{Full, Compressed}`, the kept columns, the shared `AgeLayout`, the widths and the text budgets. `FittedTreeColumns` (was `FlatColumns`) holds the kept columns and their widths. Its fit returns `TreeColumnFit::{Fitted(FittedTreeColumns), TooNarrow}`. `ColumnDrop::ORDER` drops directory, branch, type, then age and account. Each drop removes a whole column, and only when the table cannot fit. Session, role and status stay. A header shows whole or its column drops. `TreeHeader`, row drawing, row areas and painted header spans read the fitted record.
- Below the core width, the narrow core fit cuts the session to its first cell plus "…", then the role. Status stays whole. The tile clips at the pane edge.
- One truncation path, `summary::truncated_line` (and `truncated`), measures terminal cells with `unicode-width` and cuts at grapheme boundaries with `unicode-segmentation` (both workspace dependencies). It keeps span styles and ends cut text with "…". The tile `cut_line` calls it. Numbers and status words are never cut. `fitted_heading` keeps a count note whole or drops it whole.
- `SummarySessionTable::OmittedForWidth` gives a session table with no drawable columns zero rows in demand, drawing, header spans, selectable spans and the accounts start. At 34x50 with about 20 tiles, all 6 accounts draw.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — measurement, core width, fit, `FittedTreeColumns`, narrow core fit
- `crates/cargo-handler/src/render/mod.rs` — frame order: restore viewports, measure, minimum width, floor policy
- `crates/cargo-handler/src/summary/mod.rs` — truncation path, fitted heading, omitted session rows
- `crates/tui_pane/src/tiles/{grid.rs, settings.rs, mod.rs, draw.rs}`, `crates/tui_pane/src/lib.rs` — `ColumnPlan`, width-checked candidates, `TileWidthFloor`
- `crates/cargo-handler/src/render/narrow_width_tests.rs` (new) — multi-tile widths 200, 80, 50 and 40, below-core, growth and 34x50 tests
- `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs`, `crates/cargo-handler/src/summary/accounts_tests.rs`, `crates/tui_pane/src/tiles/summary_allocation_tests.rs`
- `Cargo.toml`, `crates/cargo-handler/Cargo.toml`, `Cargo.lock`, `crates/cargo-handler/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`

**Binds later work:** Frame order is: restore tree viewports, measure from the tallest projection, set the minimum width and floor, fit, then install selection areas after the shared metrics. Painted header spans come from the `FittedTreeColumns` record ("Accurate clicks during motion"). Role cells cut through `summary::truncated_line`, after the session below the core width, and a wider role word widens the core width because `minimum_tile_width` reads role widths ("Subagent role words and an even reset ramp"). Cut spans keep their style ("Session colours stay apart"). The tile foot readout belongs to "Handler tiles show no row-count readout".

**Gotchas:** The pre-layout minimum reads the tallest projection, so it can reserve width for rows that a shorter allocation does not paint. Live shots of a mid-size census do not reach the 20-tile 34x50 case; only a test covers it. Under `Required`, a width below the minimum gives `NoAdmissibleColumn` and the summary alone, not a one-column plan. A tui_pane test fixture that expects tiles must have room for one minimum-width column.

**Ruled out:** fitting each tile on its own at the grid's width (it cut status words and dropped type with free cells); a tile-count planner in cargo-handler (`ColumnLayout` is the only chooser).

### Phase 36 — Handler tiles show no row-count readout  · status: todo

**Depends on:** Phase 27

#### Work Order

**Goal:** cargo-handler tiles no longer draw the tile foot readout ("content rows: N  r/c: a/b"). The row it took goes back to the tile's content. cargo-tile keeps its readout as it is.

**Source:** phase 27 shots (2026-10-28) found the readout on every cargo-handler tile, on main too. Showrunner, 2026-10-28: hide it in cargo-handler only, through a typed hook `FootReadout::{Shown, Hidden}` with default Shown; cargo-tile shipped the readout on purpose (d7aa3f4b, tile-fixes phase 16 33cebc06) and tests it (`cargo-tile/src/shim_registration/app_scenarios.rs` ≈663 shows, ≈699 hides).

**Constraints from prior phases:**
- `draw_tile_grid` (`tui_pane/src/tiles/draw.rs`) owns the readout. `TileCells` (≈166) has `readout_rows` (≈182), `summary_foot` (≈188) and `readout_fade` (≈192). `readout_row(inner)` (≈791) gives `ReadoutRow::{Reserved(Rect), CellTooSmall}`; the readout is drawn at ≈635, ≈656 and ≈751; the content area is cut at ≈773–784. The labels are `TILE_ROWS_CONTENT_LABEL` and `TILE_ROWS_CELL_LABEL` (`tiles/constants.rs` ≈49–54); `TILE_ROWS_READOUT_HEIGHT` is 1.
- Readout demand is added in `tiles/draw.rs` by `add_readout_rows` (≈571), called at ≈296–297. Content capacity also flows through grid preparation (`grid.rs` ≈2001), motion preparation (`grid.rs` ≈2023) and `RowProbe::content_area` (`grid.rs` ≈3516). These paths must use the same resolved foot geometry: a hidden readout takes no row in demand, allocation, motion or drawing. The `grid.rs` readout references from ≈6280 on are in its test module (it starts at ≈6163), not in production demand code. `tiles/flow_layout.rs` (≈290, ≈298) is the flow packer, and no path calls it now; it is not the active grid demand path.
- `content_click` (`tiles/host.rs` ≈176) maps presses and wheel steps, and it always removes the readout row (≈185). A change to drawing only leaves the reclaimed content row outside the input area. Keep the resolved content rectangle with the existing painted-cell record. Drawing, preparation, presses and wheel steps use that rectangle, also during motion and before redraw.
- The summary cell's foot row also carries `summary_foot()` text (`SummaryFoot`). Hiding the cell readout does not remove a summary foot that an app draws.
- cargo-handler's `impl TileCells<TileKey> for Cells` is in `render/mod.rs` (≈696). Handler fixtures that show the readout are in `render/mod.rs` (≈1057–1251); other handler tests may match it too (search for "r/c: " and "content rows").
- `tiles/draw.rs` and `tui_pane/src/lib.rs` are held by the tile unit (reservation 01a17e32-a770-7b20-8fd3-78b9c527961b). Claim them `--before` it, as phase 27 did.
- Phase 27: the tile core width and `measure_trees` read the tile body height through `tui_pane::frame_inner`. A tile without a readout has one more body row.

**Spec:**
1. Add `FootReadout::{Shown, Hidden}` in tui_pane and a `TileCells` method `fn foot_readout(&self) -> FootReadout` with default `Shown`. Export it like `CellReadoutFade`.
2. With `Hidden`, no cell draws the readout, and no demand, allocation, motion or draw path reserves its row. With `Shown`, every path is byte-identical to today.
3. cargo-handler returns `Hidden`. Its tiles get the row back for content.
4. Tests, red first: a cargo-handler frame at 200x50 and 80x50 has no "content rows" and no "r/c:" text in any tile. A tui_pane test at one identical allocated rectangle: `Hidden` with an empty summary foot shows one more content row than `Shown`, which keeps one row for the readout; `Hidden` with a summary foot text keeps that foot and keeps it out of content input; demand and drawing agree. A tui_pane regression in `tiles/host_tests.rs` clicks and scrolls the reclaimed last content row and reaches the content. Existing tui_pane and cargo-tile readout tests pass unchanged. Update the handler fixtures that drew the readout. Every test under one second.
5. CHANGELOG: one line in each crate (tui_pane: the hook; cargo-handler: tiles show no row-count readout).

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/flow_layout.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the hook and every path that reserves the row
- `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs` — the resolved content rectangle for presses and wheel steps, and the reclaimed-row regression
- `crates/cargo-handler/src/render/mod.rs` — `Hidden`, and the fixtures that drew the readout
- `crates/cargo-handler/src/render/narrow_width_tests.rs` — handler tests for spec 4
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer. `impl` — all specs, `tiles/host.rs` and `tiles/host_tests.rs` included; hubs: `crates/tui_pane/src/tiles/draw.rs` and `crates/cargo-handler/src/render/mod.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh lint tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh test cargo-tile --filter readout` clean; the cargo-tile readout tests pass unchanged. No test takes a second. Live: a cargo-handler shot at 200 and 80 columns with no readout line in any tile. The checkpoint adds a Change-Review line.

### Phase 35 — Subagent role words and an even reset ramp  · status: todo

**Depends on:** Phase 27, Phase 36

#### Work Order

**Goal:** a Claude subagent row shows only its role word ("writer", "reviewer"), from a closed set, never the agent's own name with its number or phase prefix. In the Accounts table, each reset time's colour is clearly different from its neighbours.

**Source:** change review C67 (showrunner, 2026-10-27). User: "under the berth unit - i see p36-reviewer - it should just say reviewer" and "these two subagents should just say writer not writer77 and writer78". Showrunner rule: the role comes from a closed set at the type level, never by trimming name strings. Change review C71 (showrunner, 2026-10-27): "it works but the gradient on reset colors is not sufficiently distributed"; the reset colours spread evenly over the whole ramp by rank; C113 (no red at the end) and C115 (no reset colour looks like a session colour) still hold.

**Constraints from prior phases:**
- A named teammate's meta file has `agentType` equal to its `name` (for example `{"agentType":"writer77","name":"writer77","spawnedAgentType":"general-purpose","taskKind":"in_process_teammate"}`); for a plain Agent call, `agentType` is the subagent type ("general-purpose", "Plan", "Explore"). Names are free-form: writerN, reviewN, shotsN, uxN, implN, fixN, p36-reviewer, judge.
- `census/transcript.rs`: `SubagentMeta` (≈591) reads only `description`, `agent_type` and `parent_agent_id`. `SubagentRole` (≈500) is `{Capture, Review, Lint, Test, Survey, Specialist(SpecialistRole), Unrecorded}`; `SpecialistRole(String)` (≈514) stores the name, which is the defect. `from_record` (≈534) and `word` (≈546) parse it; `text` (≈557) returns the stored string.
- `census/tree.rs` `subagent_node` (≈657) puts the role in `ChildKind::Subagent(SubagentRole)` (`census/mod.rs` ≈388) and the description in the session column.
- `census/probe.rs` `ProbeChildWork.subagent_role` (≈96) serializes the enum across machines; `{"Specialist":"showrunner"}` and `"Review"` are on the wire today.
- `ProbeChildWork` (`census/probe.rs` ≈96) serializes `SubagentRole` directly. The checkpoint enum (`census/transcript.rs` ≈500) accepts `Review` and `Specialist`. It does not accept `Reviewer`, `Writer` or the other new variants.
- `render/narrow_width_tests.rs` (≈360) and its `growth_app` (≈430) use a fifty-character specialist role, so that restored or newly visible rows change the width requirement (checked at ≈488 and ≈534). A change of that fixture to `Agent` removes the tested condition. Use supported role words and fixture widths that still test off-window width growth, End/Home/cursor restoration and the first enlarged frame. Keep the whole-core and below-core compression checks, the longer Astra labels included.
- Before you choose the ramp ends, the measurement handoff names the numeric dark session/reset floors and the neighbour floors for two to eight reset times. Keep the light floors of 14.7 and 15.3 until Phase 31 replaces them explicitly. Phase 31 owns any later replacement by another visual channel.
- `agent_cell.rs`: `role_text` (≈1192) draws `role.text()`; the role ink comes from `colors.named_role(role.text())` (≈1349, `footer_colors/mod.rs` ≈839).
- Reset ramp (measured on the C71 shot, Default Dark, 6 accounts): neighbours step dE 2.3–2.7, about the threshold of a just-noticeable difference. The position is already rank/(n-1) (`ResetRank::color`, `theme/mod.rs` ≈128; rank from `reset_colors`, `summary/accounts.rs` ≈145), but the ramp ends are only about dE 12 apart in every theme (dark (125,191,110) → (69,196,153), same lightness, only hue moves; light ends as in phase 29), and the blend is plain sRGB (`tui_pane::blend_color`). `readable_status_style` does not change these inks. `accounts_gradient_tests.rs` holds the C113 floors (≈389: dE >= 20 from error red, warning and alarm; hue gap >= 30° from account inks; contrast >= 4.5), the ends floor (≈501, dE >= 12), chroma and plain-text floors (≈532), and the sRGB formula (≈185, ≈232, ≈266). C115 is `SessionResetFloor` (`render/role_title_tests.rs` ≈715): dark is `DarkNotMeasured`. A rough search found the widest dark ramp under these rules near dE 21 end to end (about 4 per step at 6 accounts), before the account hue rule.
- Session-row plan reviewers show "review" through `RoleLabel::ReviewFallback` (`census/placement.rs` ≈86), and Codex seats through `WorkerRole::text` (`census/worker_dispatch/mod.rs` ≈100); `WorkerRole::text` also feeds role matching, so the parsed words stay.

**Spec:**
1. `SubagentRole` becomes a closed enum with no string payload: Writer, Reviewer, Impl, Test, Fix, Capture, Lint, Survey, Plan, Explore, Showrunner, and `Agent` for any name that matches no word. `text()` is a `const fn` returning a `&'static str`; the drawn word is always the variant's word, never part of a name. Reviewer draws "reviewer".
1a. One word for one role (showrunner, 2026-10-27): session-row plan reviewers (`RoleLabel::ReviewFallback`) and Codex review seats (`WorkerRole::Review`) also draw "reviewer". Only the drawn word changes; the words read from seat names, leases and hierarchy files ("review") still parse.
2. The role is decided once, at the meta file boundary (`running_subagents`). `SubagentMeta` reads `name` and `spawnedAgentType` too. Source order: the name, then `agentType`, then `spawnedAgentType`, then the description words. A name splits on non-letters into whole tokens, and a token matches a role word exactly (case-insensitive, with the listed synonyms: "review" → Reviewer, "impl" → Impl). "p36-reviewer" is Reviewer, "writer77" is Writer, "judge" is Agent. A general type ("general-purpose") gives no role.
3. The role ink comes from a typed method on `SubagentRole`, not from looking up the drawn text.
4. Wire: decode the old role forms (`{"Specialist":"<word>"}`, `"Review"`) into the closed domain enum: the closed role for that word, else Agent. Encode new roles through a wire form that older builds accept, or through a defaulted extension. Keep any string payload inside the wire adapter, so the domain model gets no `SpecialistRole(String)` back. No string role is stored after deserialization.
5a. Even reset ramp (C71): the position stays rank/(n-1). The blend runs in OKLab, not sRGB. Widen the ramp ends in all four themes (`theme/mod.rs` and the theme TOML files) to the widest pair that keeps C113, C115 (dark measured too), the 30° account hue gap, contrast >= 4.5, chroma >= 0.10 and dE >= 15 from plain text; use lightness as well as hue. Measure before production code, and send the showrunner the measured neighbour steps for 6 accounts in each theme, with Accounts shots in a dark and a light theme, before the palette lands. If no pair beats today's steps, report the values and stop.
5. Tests, red first: rendered subagent rows for writer77, writer78, p36-reviewer, review3, judge and a plain "Plan" subagent show "writer", "writer", "reviewer", "reviewer", "agent" and "plan"; a plan-mode reviewer session row and a Codex review seat row show "reviewer"; an old report on the new receiver, and a new report on the checkpoint receiver; callers that built a role from a free string build a variant. Reset ramp: the drawn neighbour dE (after `readable_status_style`, pane tint on and off, the selected row included) for 2 to 8 distinct reset times in every theme is at least the measured floor; the ends floor rises to match; dark `SessionResetFloor` becomes a measured floor; the sRGB formula tests follow the OKLab blend. Every test under one second.
6. CHANGELOG: one line that subagent rows show their role word, not the agent's name, and one that reset colours step clearly by rank.

**Files:**
- `crates/cargo-handler/src/census/transcript.rs` — closed `SubagentRole`, meta fields, role source order
- `crates/cargo-handler/src/census/probe.rs` — old wire form
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/footer_colors/mod.rs` — role word and typed role ink
- `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/scan.rs` — variant construction
- `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — "reviewer" for session-row and seat reviewers
- `crates/cargo-handler/src/census/reviewer_tests.rs`, `crates/cargo-handler/src/census/claude_seat_tests.rs` — the drawn reviewer word
- `crates/cargo-handler/src/census/subagent_activity_tests.rs`, `crates/cargo-handler/src/census/probe_role_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/subagent_row_tests.rs` — spec 5 tests
- `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/repair_tests.rs`, `crates/cargo-handler/src/render/moving_role_tests.rs`, `crates/cargo-handler/src/render/selected_ink_tests.rs`, `crates/cargo-handler/src/render/selection_tests.rs`, `crates/cargo-handler/src/render/narrow_width_tests.rs` — callers build variants
- `crates/cargo-handler/src/theme/mod.rs`, `crates/cargo-handler/themes/` — OKLab blend and wider reset ramp ends
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs`, `crates/cargo-handler/src/render/role_title_tests.rs` — reset ramp floors
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — spec 1, 1a, 2–4, 5a and 6: production files, registrations and inline test migrations in production modules; hub: `crates/cargo-handler/src/census/transcript.rs`.
- `test` — spec 5: the listed separate test modules, including `narrow_width_tests.rs` and `moving_role_tests.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` clean. No test takes a second. Live: a shot of a tile with named writer and reviewer subagents shows "writer" and "reviewer" in the role column; Accounts shots with 6 reset times in a dark and a light theme, each colour read from the shot. The checkpoint adds a Change-Review line.

### Phase 31 — Session colours stay apart  · status: todo

**Depends on:** Phase 18, Phase 27, Phase 30, Phase 35

#### Work Order

**Goal:** in all four themes, the five session inks read as five different colours, and no session ink, role ink or reset shade on a row looks like another.

**Source:** next item "Session colours stay apart" (phase 26, approved by the showrunner 2026-10-23 19:30 AST), widened by phases 29 and 30. Showrunner ruling, 2026-10-24: the root cause is too many inks on one row. Run after Phases 36 and 35, beginning with options and shots for the showrunner.

**Constraints from prior phases:**
- Phase 26 (dark): the valid hue domain is about 92.5°, and at most 2–3 inks can be CIEDE2000 >= 20 apart; steps of 45°, OKLCH C >= 0.10, and both together also fail.
- Subagent role words and an even reset ramp (phase 35) widens the reset ramp ends and blends in OKLab; read the drawn ends from `theme/mod.rs` at start. The values below are from before it. Phase 35 names numeric dark session/reset floors and neighbour floors for two to eight reset times, and keeps the light floors of 14.7 and 15.3. This phase owns any later replacement of those floors, by new floors or by another visual channel.
- Phase 29 (light): light reset ends Default Light (115,0,235) → (73,0,158) and High Contrast Light (115,0,235) → (82,55,154) hold session-vs-reset dE 14.7 and 15.3 (`render/role_title_tests.rs`, `SessionResetFloor::HeldLight`). The drawn reset shades fall in one violet band (OKLCH h 291–294°).
- Phase 30 (light): no five-ink light session set exists under the current row floors, even with any one floor relaxed; the light defects are near-black session inks in two hue groups, the `showrunners` role ink #7732FF dE 5.3 from the nearest reset end, and reset steps of dE 2.7–3.4. `summary/accounts_gradient_tests.rs` holds the reset time order.
- Session rows draw only through `SessionRowInk::style()` (`theme/mod.rs` ≈58). `SessionRowInk::fixed_fields()` and `FooterColors::session_row_inks()` list every ink a row can draw. Root i takes entry i (mod 5) of `session_hues`, and each tile title keeps its summary row's ink.
- Phase 21 adds two Astra role inks; this phase includes them in its assessment. Today `FooterColors::parse` (`footer_colors/mod.rs` ≈772) reads only `role.showrunner` and `role.unit-director`, so the `role.astra-*` keys are ignored, and `FooterColor` has no `mauve` variant. Assess the prospective sources by value: designer `mauve` = xterm 139 (#af87af), reviewer `bright-green`. Role inks resolve through `FooterColors::named_role` (≈818) and `FooterColor::management_style` (≈68), drawn as `SessionRowInk::Management`.
- The accounts table draws role words through `role_words` (`summary/accounts.rs` ≈609): normal management ink for a fresh reading, subdued management ink (`Freshness::style`) for a stale or unread one. Reset shades draw after the readability correction (`footer_colors::readable_status_style`), not as raw `ResetRank::color`.
- Covered contrast (phase 18): production `InkCoverage::contrast` (`footer_colors/mod.rs` ≈367) and the oracle `session_glyph_contrast` (`render/role_title_tests.rs` ≈681) both take the lower of the unrounded and the byte-rounded 208/255 covered contrast; the floors here are measured that way. Phase 18 moved the light session ink 0,35,86 to 0,34,85 (`theme/mod.rs` ≈351, ≈389; `themes/default_light.toml`, `themes/high_contrast.toml`): dE 15.21 from tinted light Account(Blue) 0,69,175, chroma 0.1008.
- Phase 27's column drops and truncation keep the style of each span they cut.

**Spec:**
1. Options first (showrunner, 2026-10-24): list what else can separate sessions. At least: fewer fixed inks on a session row (for example role, account or status drawn in the plain text ink), and a second visual channel the style guide allows (for example bold, a glyph or a background tint). For each option, give the inks a row draws, the measured smallest session-pair dE, session-vs-fixed dE and session-vs-reset dE in all four themes, and what the row loses. Include the accounts table: its role inks in the fresh and the subdued (stale or unread) style, against the drawn reset shades, with pane tint on and off. Each option also says how a user tells reset ranks apart with six or more reset times (today neighbouring light shades step only dE 2.7–3.4): a larger colour step, a second channel, or the time text alone, measured. Each option also says how its separating channel survives the selected row band and the phase 20 word-match background (one background colour for every match, drawn over the selection), for example a glyph or bold that a background does not erase. Send the showrunner the options with shots of the summary agents table and the accounts table in a dark and a light theme, before any palette code. The showrunner's choice settles the rest of this spec. The unit director then adds the files the chosen option needs to **Files**.
2. The showrunner's choice sets the final floors and the tests that hold them. A second visual channel separates by something other than colour, so the choice names which colour floor it replaces and the test that proves the channel instead. Measure the chosen option before production code. If no palette meets the chosen floors, report the measured values to the unit director and stop; never land a palette from an empty feasible set (phase 30's first fallback collapsed five inks to one). Without a replacement named by the choice, the completion condition holds: in all four themes, every session ink has CIEDE2000 >= 15 from each fixed ink that can share its row, every pair of session inks has CIEDE2000 >= 20, and covered contrast is >= 4.5:1 with the two-way oracle (the lower of the unrounded and the byte-rounded 208/255 blend). In both light themes, session-vs-reset CIEDE2000 is >= 20 within 45° of hue. Reset shades keep dE >= 15 from every role ink, in the fresh and the subdued style, and the light ramp ends are as far apart as that allows.
3. Root i keeps entry i of the list, and each tile title keeps its summary row's ink.
4. Tests, red first: the spec 2 floors in all four themes replace the held phase 26 and 29 limits. Rendered accounts tests compare role inks (fresh and subdued) with the drawn reset shades, pane tint on and off. A rendered test with six or more reset times proves the chosen way to tell reset ranks apart.
5. CHANGELOG: one line for the visible change.

**Files:**
- `crates/cargo-handler/src/theme/mod.rs`, `crates/cargo-handler/themes/` — inks of the chosen option
- `crates/cargo-handler/src/render/role_title_tests.rs`, `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — the spec 2 floors
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — the option measurements and shots, then the chosen option's production code and the CHANGELOG; hub: `crates/cargo-handler/src/theme/mod.rs`.
- `test` — the spec 2 floor tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` clean. No test takes a second. Live: summary and accounts shots in all four themes, with each session, role and reset ink read from the shot.

### Phase 34 — Tests leave no folders and wait on signals  · status: todo

**Depends on:** Phase 18, Phase 31

#### Work Order

**Goal:** the cargo-handler startup-note tests leave no temporary folder behind, also when a test fails, and the census schedule timeout test waits for its child's pid file by a signal it controls, not by time.

**Source:** showrunner add-ons, 2026-10-26: the theme-note tests leave /tmp/.tmpXXXXXX folders that hold broken-default-dark.toml; and `census::schedule::tests::unsuccessful_commands_publish_no_edges_and_timeout_reaps_the_child` failed on the Mac under load (0.76 s, "the script should have recorded its pid", NotFound). Run order (showrunner): after phase 31.

**Constraints from prior phases:**
- Phase 18: `crates/cargo-handler/src/terminal/theme_note_tests.rs` builds the app through `terminal::startup_theme_notes` (`terminal.rs` ≈79) and `App::new_for_test_with_startup` (`app.rs` ≈246). Its `theme_directory()` keeps the theme folder in a `static DIRECTORY: OnceLock<TempDir>`, which is never dropped, so the folder stays after the run. The fixture function that gives the folder path takes no `self`.
- The census test is in `crates/cargo-handler/src/census/schedule.rs` (≈1278). It already waits for a completion signal before it reads the pid file. The race is elsewhere: `ShortDeadlineRunner` allows 120 ms, and `census/hierarchy.rs` (≈476) starts that deadline before it spawns the child, so the child can be killed before it writes its pid. The failure is expiry before child readiness, not a fixed sleep before the read.

**Spec:**
1. Each theme-note test owns its own `TempDir`, which is removed when the test ends, also when it fails (the owner drops on unwind). No `keep()`, no `into_path()`, and no folder outside a `TempDir`. Pass the folder path through the fixture or the config the test builds, not through a static.
2. A test fails when a theme-note run leaves a folder behind: it runs a fixture to the end, also through a forced failure, and checks that the folder path no longer exists.
3. The census timeout test uses a test-controlled expiry trigger around the existing termination and reaping path: it observes a child-ready signal, then triggers expiry, with bounded waits only for fixture failure. Production command deadlines stay unchanged. The test proves that a timeout publishes no edges and kills and reaps the ready child.
4. Each test runs under a second, alone and in the full suite.

**Files:**
- `crates/cargo-handler/src/terminal/theme_note_tests.rs`
- `crates/cargo-handler/src/census/schedule.rs` — the timeout test only
- `crates/cargo-handler/src/census/hierarchy.rs` — the bounded test-controlled expiry trigger only

**Workers:** 1 tester.
- `test` — both test repairs, the no-leftover-folder test and the test-controlled expiry trigger; keep the production startup-theme handoff when the static `TempDir` fixture is replaced.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. After a full cargo-handler run, no new /tmp/.tmp* folder that holds a theme file is left (count before and after).

### Phase 16 — Row selection in tiles  · status: done

#### As-built

- Each cargo-handler selection surface, `SelectionSurface::{Summary, Tile(TileKey)}`, has a `Viewport` and a `TileSelection::{NoRows, Row(RowIdentity)}`. Up, Down, PageUp, PageDown, Home and End move the selected row in the focused tile and stop at the edges; Left, Right, Tab and Shift-Tab move the tile focus. A click returns `RowClick::{Selected, Missed}`; the wheel scrolls the tile under the pointer and does not move focus.
- tui_pane holds the shared navigation: `Viewport::navigate`, `Viewport::scroll_if_contains`, `table_row_at`, `ScrollDirection::{Up, Down}`, `FocusLocation`, `handle_tile_scroll`, `TerminalApp::scroll` and `TileGridHost::cell_scrolled`. The wheel is typed `ScrollDirection` from the runner to `SurfaceSelection::scroll`. cargo-port uses these from tui_pane and its behavior and look are unchanged.
- `RowIdentity` has one variant per drawn row kind: `Session`, `Seat { worker: WorkerIdentity }`, `Thread`, `Subagent { owner: RowOwner, id }`, `Shell`, `LegacyWork { owner, kind: LegacyWorkKind, started, ordinal }` (older senders with no work id), `SummaryAccount` and `SummaryGroup`; `RowOwner { pid, started }`. A seat row continues across a new dispatch or a relaunch of the same `WorkerIdentity` kind. Work ids travel through census and probe as `WorkIdentity`; a shell's timer is `ChildKind::Shell(ShellTimerObservation::{Untimed, ObservedCountdown(Timer)})`.
- `Selections::resolve` reconciles all surfaces together each frame: a missing identity moves to the row at its old index, else the last row, else `NoRows`, and the selected row stays in view. `RowRetention::{Surviving, Vacated}` lets only a vacated surface accept a selection that moves in the background; `Selections::follow_focused` moves the active selection with its exact row between the unassigned tile and the summary.
- `Cells::prepare_frame` (`render/mod.rs`) sets each selection area after shared tree metrics, from the painted presentation. The selected band draws `focus.active` (focused) or `focus.remembered` per theme; Default Dark grounds are 86/86/86 and 46/46/46. On a selected row only, `SelectedGroundInk` moves an ink under 4.5:1 (208/255 coverage) in OKLCH lightness away from the ground and keeps its hue and chroma; unselected inks are unchanged.
- The status column width is measured from the rows the tile paints (`TreeMeasurement` `status_widths`), never less than its header.

**Files:**
- `crates/tui_pane/src/layout/viewport.rs` — navigation, wheel scroll, `table_row_at`
- `crates/tui_pane/src/tiles/host.rs`, `tiles/grid.rs`, `runner/app.rs`, `runner/event_loop.rs` — wheel forwarding, focus location
- `crates/cargo-handler/src/interaction.rs` — row identities, `Selections`, clicks, scroll
- `crates/cargo-handler/src/render/mod.rs` — selection areas, selected band and inks
- `crates/cargo-handler/src/agent_cell.rs` — `TreeLine::selection_identity`, `AgentTile::row_identities`, painted status width
- `crates/cargo-handler/src/footer_colors/mod.rs` — `SelectedGroundInk`
- `crates/cargo-port/src/tui/input/dispatch.rs`, `tui/panes/actions.rs`, `tui/panes/hit_test.rs` — callers of the shared navigation

**Binds later work:** clicks hit the areas `Cells::prepare_frame` sets after shared tree metrics, so the selection-area pass stays after metrics; every row click goes through one hit test (`table_row_at`) and the `RowClick` outcome, with no second click dispatcher; wheel input is `ScrollDirection` through `handle_tile_scroll`.

**Gotchas:**
- A selection area set before shared metrics maps clicks on a group-compressed tile to rows it does not paint.
- `AgentTile::row_identities` ranks a legacy row's ordinal only among otherwise identical legacy identities, so an unrelated row that leaves does not move the selection.
- Separation (dE >= 12) applies only to unselected rows; a selected row requires only 4.5:1 and the kept hue.

**Ruled out:** mixing a selected ink toward white or black (it drains chroma); lightening the shared inks (it breaks unselected separation); a per-pair spread solver for selected inks; underline or reverse video on a selected row; an invented pid for a pidless row.

### Phase 20 — Click a word to highlight every instance  · status: todo

**Depends on:** Phase 16, Phase 27, Phase 31

#### Work Order

**Goal:** a click on a word in cargo-handler gives every instance of that word in the window a subtle, clearly visible background.

**Source:** the user (via the showrunner), 2026-10-20: "if i click on a word i want every instance of that word highlighted - i guess with a background color that is subtle but definitely visible". Showrunner rulings: a phase after Phase 16, because both are mouse work in the same place; order 15, 17, 18, 19, 16, 20, 9; the five points of the unit's proposal as written; a click on the highlighted word at another place also clears the highlight; one Change-Review line.

**Constraints from prior phases:**
- Phase 28 routes presses and wheel steps through `cell_hit` and `content_click` (`tiles/host.rs` ≈176), absorbs `RetiredCell` before focus changes, and tests headers before row selection in `render::cell_clicked`. Extend this path so one press supplies both the row action and the painted word, with named outcomes for content, title, empty content, chrome and retired content rather than another domain-owned `Option`; keep the painted source-coordinate translation and the summary's `ViewportMapping`; keep `Option` only at framework-required boundaries.
- `content_click` excludes tile frames, titles and readouts. Word clicks also work on tile titles, so tui_pane `tiles/host.rs` forwards a title press to the host; the word logic stays in cargo-handler. The status bar, the readouts and the overlays are out of scope (showrunner, 2026-10-23).
- An open modal overlay takes a click first. Esc clears the highlight only when no overlay is open.
- Phase 27: `summary::truncated_line` (`summary/mod.rs` ≈846) cuts styled text by terminal cells and grapheme clusters, and `SummarySessionTable::OmittedForWidth` (≈226) removes session lines fully. The omitted state is already visible to the user and its 34x50 regression is green.
- Phase 36: cargo-handler tiles draw no readout, and the resolved content rectangle stays with the painted-cell record, so presses and wheel steps reach the reclaimed last content row.
- The content, title and chrome outcomes cross the public host boundary. The pointer payload is exported in `tiles/mod.rs` (≈45) and `lib.rs` (≈502).
- Theme roles live in `theme/mod.rs` (`Role` ≈91) and the built-in theme files in `crates/cargo-handler/themes/`; text colors stay as they are drawn.
- Phase 15 CIEDE2000 12 measures distinction from header ink, not readability. `ReadableInk` measures against `theme.text.bg_focus`. Use the luminance contrast oracle (4.5:1) against the real match background in all four themes, for subdued role and account ink and for selection overlap; foreground colors do not change.
- Phase 17: `footer_colors/mod.rs` `InkGrounds` covers the base, focused and unfocused grounds, and account, management and session inks use glyph coverage (`InkCoverage::Antialiased`). Measure every ink on the drawn match and selection backgrounds with that same coverage calculation (phase 18: the lower of the unrounded and the byte-rounded 208/255 blend). The Astra role inks do not exist yet: phase 21 runs after this phase and adds their word-match tests to `render/word_contrast_tests.rs`. If no match background keeps every unchanged foreground at 4.5:1, send the conflict to the showrunner before accepting a palette.
- Phase 16's selected-row contrast tests stay green; where a match overlaps a selected row, the match background is the ground. Contrast tests compute the expected ratio with their own literal 208/255 glyph coverage, not by reading `ANTIALIASED_GLYPH_COVERAGE`.

**Spec:**
1. A word is a token between spaces in any drawn cell, with leading and trailing punctuation removed. A match is the whole word with exact case: a click on `claude1` marks every `claude1` and no `claude12`.
2. One click marks every instance in the whole window: every agent tile, the unassigned tile, the summary and the accounts table. The highlighted word is one type, `enum WordHighlight { Off, Word(HighlightWord) }`, where `HighlightWord` can only hold a non-empty word.
3. A click on a row selects the row (Phase 16) and sets the word. A click on the highlighted word, at any place, a click on empty space, or Esc clears it. A click on another word moves the highlight to that word.
4. The highlight is a new theme role, `word_match`, with a background in each built-in theme: subtle but easy to see in light and dark themes. The text keeps its color and stays readable on it (ink contrast measured as in Phase 15, bound named in the checkpoint). One color for every match. Where the selected row and a match overlap, the match background draws on top. The session-separating channel phase 31 chose (colour, glyph, bold or tint) stays visible on a matched cell and on a matched selected cell; a rendered test in `render/word_contrast_tests.rs` proves it.
5. The mark is one pass over the drawn frame after every tile draws, and runs only while a word is set. So it follows the word through every redraw, a new row that holds the word gets it too, and it costs nothing when no word is set.
6. Tests, each red before its fix: the token rules (punctuation removed at the ends, whole-word match, exact case); matches marked in two tiles and in the accounts table; a new row with the word is marked on the next frame; clear on empty space, on Esc, on a second click on the same word and on a click on the word at another place; a click on another word moves the highlight; a render test that reads the background colors back from the frame.
7. A contrast test reads the drawn match background and each ink on it in all four themes and asserts at least 4.5:1 with glyph coverage, on the base, focused and unfocused grounds, selection overlap included.
8. Word lookup reads the last painted cells, not a rebuild from census text. Scope: tile content and tile titles; the empty-space clear works there too.
9. Tests also cover: a word on an account continuation line; a missed row click that clears the highlight and keeps the row selection; a word in a tile title; a click on the status bar or a readout that leaves the highlight; a click while a modal overlay is open that goes to the overlay; Esc with an overlay open that closes the overlay and keeps the highlight. A retired painted tile leaves focus, row selection and word highlight unchanged; a word on a pinned heading or column header is read from the painted cells without breaking header sorting. Words resolve from the visible painted token when it holds wide glyphs, combining marks, emoji presentation sequences or a truncation ellipsis; a click on any cell that a wide glyph occupies resolves the same token; omitted session text gives no word target. Shared tui_pane regressions in `tiles/host_tests.rs` cover title forwarding, chrome exclusion, retired content and the Phase 36 reclaimed content row.

**Files:**
- `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/globals.rs`, `crates/cargo-handler/src/keymap.rs`, `crates/cargo-handler/src/interaction.rs`, `crates/cargo-handler/src/theme/mod.rs`, `crates/cargo-handler/src/word_highlight.rs`, `crates/cargo-handler/src/main.rs`, `crates/cargo-handler/themes/default_dark.toml`, `crates/cargo-handler/themes/default_light.toml`, `crates/cargo-handler/themes/high_contrast.toml`, `crates/cargo-handler/themes/starter.toml`, `crates/cargo-handler/CHANGELOG.md`
- `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/word_highlight_tests.rs`, `crates/cargo-handler/src/render/word_click_tests.rs`, `crates/cargo-handler/src/render/word_contrast_tests.rs`, `crates/cargo-handler/src/render/selection_tests.rs`

**Workers:** 1 writer + 1 tester.
- `impl` — the word state, the pointer handling (the existing click path and Esc), the title forwarding in tui_pane, the shared exports (`tiles/mod.rs`, `lib.rs`) and the shared regressions in `tiles/host_tests.rs`, the frame pass, the themes, the registrations and both CHANGELOGs; hub: `crates/cargo-handler/src/render/mod.rs` and `crates/cargo-handler/src/main.rs` (registrations).
- `test` — the token module (`word_highlight_tests.rs`), the click module (`render/word_click_tests.rs`) and the contrast module (`render/word_contrast_tests.rs`). Phase 21 later extends the contrast module for the Astra role inks.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`, `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. `render/selection_tests.rs` passes unchanged. Live: framed shots, in a dark and a light theme, of a word highlighted across two agent tiles, the summary and the accounts table, and of the same screen after the highlight clears. The checkpoint carries `Change-Review: a click on a word in cargo-handler highlights every instance of it in the window with a subtle background, and a second click, Esc or a click on empty space clears it | open cargo-handler, click an account name, then click it again`.

### Phase 21 — Astra designer and Astra reviewer roles  · status: todo

**Depends on:** Phase 17, Phase 16, Phase 18, Phase 26, Phase 29, Phase 20, Phase 31

#### Work Order

**Goal:** cargo-handler shows an Astra designer session and an Astra reviewer session (Codex on gpt-6-astra) under the unit director or showrunner that runs it, each with its own role word and color.

**Source:** the user (via hana and the showrunner), 2026-10-20: cargo-handler shows the two new Codex roles of the in-game metrics production; a designer is a lasting role that can be resumed and runs under a unit director or a showrunner. Showrunner rulings: a phase after Phase 17; order 15, 17, 21, 18, 19, 16, 20, 9; role words `astra designer` and `astra reviewer`; colors pink and bright-green; if the hierarchy edge is not on `~/.claude` main when Phase 17 checkpoints, run Phase 18 and return here when the showrunner says it landed. Lasting roles read from `roles.py` are not in this phase (see the note below).

**Showrunner note (2026-10-21, revised guild design):** there are four guilds: `designers`, `reviewers`, `showrunners` and `units`. Workers have no guild. Each head is a unit director whose unit name is its guild ID, under a new top-level production `guilds` (herdr space `guilds` on natedev: the guilds showrunner in tab 1, then the four heads). In the census the heads are ordinary unit edges under the guilds showrunner, and each member keeps its real manager: an Astra designer stays under the unit director or showrunner that holds it, never under the `designers` head. `astra-designer` and `astra-reviewer` edges and colors do not change. Lasting roles will come only from sdlc phase 6 `scripts/guilds/roles.py list --json`; parse no other file for them. That file is not on `~/.claude` main, and this phase does not need it: reading it is a later phase. Tips are files in `~/rust/nate_style/guilds/<guild>/`; there is no guild message group and no publication record, so cargo-handler draws nothing for them. Design doc: `docs/plans/sdlc-guilds-design.md` on `~/.claude` branch `sdlc-guild-process` (dadb482b), sections "Guilds and heads" and "Data for cargo-handler".

**Constraints from prior phases:**
- Phase 16: a selected row draws on the selection ground (`InkGrounds` in `footer_colors/mod.rs`). Rendered tests also draw both Astra roles on a selected row in a focused and an unfocused tile and require 4.5:1.
- natedev adds to `production_tree.py --json` one edge per Astra session while it holds the role: `{"manager": <Claude session id of the unit director or showrunner>, "member": <Codex thread id>, "role": "astra-designer" | "astra-reviewer", "codex_home": <CODEX_HOME>, "state": "busy" | "idle"}`, and the `statusline.conf` keys `role.astra-designer=pink` and `role.astra-reviewer=bright-green`. Do not edit `~/.claude`.
- Today `census/hierarchy.rs` drops an edge whose role it does not know (`HierarchyWireRole` `#[serde(other)] Unknown`); `HierarchyRole` is in `census/mod.rs`; `RoleLabel` is in `census/placement.rs`; `FooterRole` and its `role.*` keys are in `footer_colors/mod.rs`. Since phase 17 there is no `From<HierarchyRole> for FooterRole` and no `FooterRole::Worker`: management roles map in `FooterColors::hierarchy_role` (`footer_colors/mod.rs` ≈827), specialist role words in `FooterColors::named_role` (≈818), and worker rows draw plain.
- Phase 26 session-row ink path: `SessionRowInk::Management(FooterColor)` (`theme/mod.rs` ≈67) already draws a configured management ink through `FooterColor::management_style` (`footer_colors/mod.rs` ≈68). `FooterColors::session_row_inks()` (≈765) lists the fixed fields, then each account ink as `Account`, then each configured role ink as `Management`. `FooterColors::parse` (≈772) reads only `role.showrunner` and `role.unit-director`, so the `role.astra-*` keys in `footer_colors/fixtures/statusline.conf` are ignored today, and that fixture still has `role.astra-designer=pink`. `SessionRowInk` keeps no state; its observable result is the drawn foreground.
- Phase 17 probe schema 3: `ProbeChildWork` sends the unit kind `subagent` with a defaulted role extension converted to the semantic domain role. Keep its compatibility tests beside the new hierarchy extension tests: a capable receiver keeps recorded subagent roles; an older receiver accepts the report without the extension.
- The config gives pink to both `role.astra-designer` and `account.codex1`, and both solve to the same ink. Showrunner ruling, 2026-10-21: `role.astra-designer` moves off pink and `account.codex1` keeps it; no existing color name clears every ink, so the new name is `mauve` = xterm 139 (#af87af; solved dark #af87af, tint on #c2a4c2; light #734c73, tint on #5e3f5e; at least CIEDE2000 15.56 from every account, role, status, header, plain-text and rainbow ink; 4.51–4.77:1 at 208/255). natedev adds `mauve` = `38;5;139` to `statusline.jq` and sets `role.astra-designer=mauve`; this phase adds `mauve` (index 139) to `FooterColor` in `footer_colors/mod.rs`, parsed from `mauve`. This phase does not wait for natedev's change: its distance test uses a fixture with the `statusline.conf` values and `role.astra-designer=mauve`, and the live shots use the real config: the change is on `~/.claude` main at a06402f7.
- The run order is 18, 25, 26, 29, 27, 20, 9, 21, with phase 28 when tile-fixes phase 24 has merged, so this phase runs after phases 19, 26, 29 and 20.
- Colour limits for the Astra role inks, as built. Measure them through the production role API (`FooterColors::hierarchy_role` ≈827 with the new variants, `named_role` ≈818) and `SessionRowInk::Management`, not through fixed colors, on the final drawn output with the two-way 208/255 coverage of phase 18 (the lower of the unrounded and the byte-rounded blend).
  - Temporary floors (phase 26 as built, `render/role_title_tests.rs` ≈710–731). Light themes: dE >= 15 from every session ink and from every other ink that can share the row. Default Dark: dE >= 12.2, and High Contrast Dark: dE >= 14.8, for the same two classes. The dark floors hold until phase 31 ("Session colours stay apart") lands. Phase 31 runs first: its chosen row design (which fields draw in colour, any second channel) and its floors replace these temporary floors. Astra labels use the style path phase 31 chose; this phase does not restore the old exceptions.
  - Final targets, separate from those floors: row-fixed dE >= 15, session dE >= 20, covered contrast >= 4.5:1. Phase 31's final floors or named replacement channels apply in all themes; this phase restores no earlier dark exemption. Covered contrast >= 4.5:1 applies now, in all four themes.
  - Consecutive session hue spacing applies only to the session inks, not to role inks.
- Keep the wire-boundary rejection rules below. Phase 16 selection: an Astra row keeps its stable thread identity and the selected band through busy/idle updates and manager placement. Contrast tests compute the expected ratio with their own literal 208/255 glyph coverage, not by reading `ANTIALIASED_GLYPH_COVERAGE`.
- The Codex thread lookup in `constants.rs` reads only `originator = 'codex-tui'` threads; `astra_review.py` runs `codex exec` (`originator` `codex_exec`).
- The wire and lease token `review` stays accepted; displayed reviewer words follow Phase 35.
- `render/narrow_width_tests.rs` (≈360) and its `growth_app` (≈430) use a long role so that restored or newly visible rows change the width requirement (checked at ≈488 and ≈534). Use supported role words and fixture widths that still test off-window width growth, End/Home/cursor restoration and the first enlarged frame. Keep the whole-core and below-core compression checks, the longer Astra labels included.
- `codex::read_threads_by_id` already accepts any originator (`CODEX_THREAD_BY_ID_QUERY`); only the startup-window query limits originator to `codex-tui`. Do an edge-named lookup in the named Codex home; do not widen the startup query. Reuse `SessionNames` for renamed sessions; keep the phase 15 home and resume checks (`thread_matches_home`, `ResumeRequest`) and the no-dispatch regression.
- The scanner gets no hierarchy snapshot today, and app-server discovery needs Linux open file descriptors under the default sessions directory. Hand accepted edges from the scheduler to the scanner; keep database and rollout reads off the scheduler thread and outside the publication lock.
- `hierarchy.rs:641` turns any wire `state: busy|idle` into `ReviewerState`; pool validation makes an edge without a lease idle and `is_drawn` hides it. Astra sessions get their own semantic state, outside the reviewer-pool state machine.
- `HierarchyEdge` today joins a role with `ReviewerState` and has no Codex home field. An Astra edge is one typed payload that requires its Codex home and its busy or idle state (for example `AstraSeat { role: AstraRole, codex_home: CodexHome, state: AstraState }`); the optional wire fields convert into it at the input boundary, and an edge without them is refused there. Do not add separate domain `Option` fields that permit an Astra member without its address.
- Probe schema 3 serializes `HierarchyEdges` directly; new role variants in `edges` make older receivers refuse a probe. Carry new roles in a defaulted extension.
- Phase 15 fallbacks stay: a session with no name shows its pid text, and an unknown footer color name draws plain.
- The edge is on `~/.claude` main at 60ca146b (natedev and the Mac): `production_tree.py --json` gives `{manager: Claude session id, member: Codex thread id, role: astra-designer|astra-reviewer, codex_home, state: busy|idle}`; a held thread no longer also prints as a worker edge. `statusline.conf` has `role.astra-designer=pink` and `role.astra-reviewer=bright-green`. Live shots: `/astra_review` holds the reviewer role while its Codex process runs; hold a designer by hand with `~/.claude/scripts/lib/py ~/.claude/scripts/astra/astra_roles.py hold designer --session-dir <delegate dir> --worker <name> [--manager <session id>]`, release with `astra_roles.py release <thread id>`.

**Spec:**
1. `HierarchyWireRole` and `HierarchyRole` gain `AstraDesigner` and `AstraReviewer`, parsed from `astra-designer` and `astra-reviewer`. Their role words are `astra designer` and `astra reviewer` in the role column and the tile title.
2. `FooterRole` gains `AstraDesigner` and `AstraReviewer`, read from `role.astra-designer` and `role.astra-reviewer`; the role word draws in that color everywhere a role word draws in colour (Phase 15 rules), unless phase 31 chose plain role text or another channel; then it follows phase 31's choice. On a session row both Astra role inks draw through `SessionRowInk::Management(FooterColor)`; add no new `SessionRowInk` variant for each role.
3. An Astra row draws under its edge's manager, a unit director or a showrunner alike, and keeps the edge's `state` as its idle or busy status between turns.
4. The thread lookup also reads a `codex_exec` thread when an Astra edge names it, under the edge's `codex_home`, so the row gets its session name, activity and fast mark as other Codex rows do.
5. Tests, each red before its fix: both roles parse and keep their words; each takes its configured color; an Astra row draws under a unit director and under a showrunner; a `codex_exec` thread named by an edge gets its name and activity; an unknown role is still dropped.
6. Tests also cover: an edge-named `codex_exec` thread with no open app-server rollout; busy and idle Astra rows visible with no reviewer-pool lease; an older receiver parsing a new probe, and a new receiver restoring role, home and state; each Astra role, read through the production role API, draws through the rendering path phase 31 chose (colour or another channel), keeps its configured role assignment, meets phase 31's final floors from every session ink and from every other ink that can share its row, and reaches 4.5:1 covered contrast in all four themes; an Astra edge with no Codex home or no state is refused at the wire boundary.
7. Guild heads: a hierarchy with a `guilds` showrunner and four unit directors named `designers`, `reviewers`, `showrunners` and `units` draws them as ordinary unit director rows under that showrunner, with no new role word. An Astra designer held by a unit director of another production stays under that unit director. Rendered tests cover both.
8. Palette tests: a new fixture `footer_colors/fixtures/statusline_astra.conf` holds the `statusline.conf` values with `role.astra-designer=mauve` and `role.astra-reviewer=bright-green`. Tests prove that both Astra roles enter `FooterColors::session_row_inks()` as `SessionRowInk::Management` from that fixture, and that the rendered session palette checks in `render/role_title_tests.rs` measure both inks.
9. Word-match contrast (phase 20 runs first): extend `render/word_contrast_tests.rs` with rendered word-match tests for both Astra role inks, with the final production inks and the two-way 208/255 coverage (the lower of the unrounded and the byte-rounded blend). Each ink reaches 4.5:1 on the drawn match background, on the base, focused and unfocused grounds, and where a match overlaps a selected row.
10. Reset shades: phase 31's chosen design sets this floor. Unless phase 31 replaced the role-vs-reset colour floor with another channel, each Astra role ink is CIEDE2000 >= 15 from every drawn reset shade on each light theme; if it did, the test measures the drawn treatment and proves the replacement channel for both Astra inks (a new test in `summary/accounts_gradient_tests.rs`, through `FooterColor::management_style` and `ResetRank`; phase 30 landed no reset-vs-role-ink test, and phase 31 owns that floor for the other role inks). Measure first; if an Astra ink is under 15, report it to the unit director before any palette change.

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/codex.rs`, `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/footer_colors/mod.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/census/astra_role_tests.rs`, `crates/cargo-handler/src/census/probe_role_tests.rs`, `crates/cargo-handler/src/footer_colors/tests.rs`, `crates/cargo-handler/src/render/selection_tests.rs`
- `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/census/schedule.rs`
- `crates/cargo-handler/src/theme/mod.rs` — `SessionRowInk` (only if the Astra inks need a change there)
- `crates/cargo-handler/src/footer_colors/fixtures/statusline_astra.conf` — new fixture with `role.astra-designer=mauve`
- `crates/cargo-handler/src/render/role_title_tests.rs` — the session palette checks measure both Astra inks
- `crates/cargo-handler/src/render/word_contrast_tests.rs` — phase 20's contrast module, extended for both Astra inks
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — the Astra-vs-reset test (spec 10)
- `crates/cargo-handler/src/render/narrow_width_tests.rs` — width growth and compression checks with the longer Astra labels

**Workers:** 1 writer + 1 tester.
- `impl` — the production files (hierarchy, scanner, probe, placement, color and rendering), `theme/mod.rs`, the registrations and the CHANGELOG; owns all shared module files; hub: `crates/cargo-handler/src/census/mod.rs` and `crates/cargo-handler/src/theme/mod.rs` (shared modules).
- `test` — the Astra tests (`astra_role_tests.rs`), the probe compatibility tests (`probe_role_tests.rs`), the footer-color tests (`footer_colors/tests.rs`), the fixture (`statusline_astra.conf`), `render/role_title_tests.rs`, the rendered selection tests, the Astra word-match tests in `render/word_contrast_tests.rs`, the Astra-vs-reset test in `summary/accounts_gradient_tests.rs`, and the width checks in `render/narrow_width_tests.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Astra rows keep their stable thread identity and the selected band through busy/idle updates and manager placement. The session palette checks in `render/role_title_tests.rs` measure both Astra inks, the Astra word-match tests in `render/word_contrast_tests.rs` pass, and the Astra-vs-reset test in `summary/accounts_gradient_tests.rs` passes (with the floor or channel phase 31 chose). Live: framed shots, in a dark and a light theme, of an Astra designer under a unit director and an Astra reviewer under a showrunner, with the real config. The checkpoint carries `Change-Review: cargo-handler shows Astra designer and Astra reviewer sessions under the unit director or showrunner that runs them, with their own role words and colors | open cargo-handler while an Astra review runs and find its row under its parent`.
