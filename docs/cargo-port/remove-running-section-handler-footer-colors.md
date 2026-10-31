# handler-footer-colors

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-handler colors roles and accounts with the colors the Claude Code footer uses, read from the footer's own color file, and the summary's machine headings count "top-level agents". Later phases sort any tile's tree or the summary by a clicked column header, make clicks land on what is drawn while tiles move, and teach the census two Claude cases.

> **Production: cargo-liner** — unit `handler-enh-unit`; production doc `docs/cargo-port/cargo-liner-production.md`

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

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs` ≈3129) while drawing interpolates (`drawing_at`, `grid.rs` ≈3441), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps. Since phase 6, `summary::machine_height` counts hierarchy notice lines, so a first-machine notice appearing or clearing moves later machines' clickable headers; add a two-machine draw → notice change → click regression.

**Constraints from prior phases:**
- Phases 23, 24 and 27: `shared_tree_metrics` (`agent_cell.rs`) groups tiles by `PermittedTreeColumns` (columns populated in the complete rows) and screen x. `ColumnDrop::ORDER` (`agent_cell.rs` ≈476) also drops Age and Account, and `GroupFitStep::ORDER` (≈908) has six steps. Restore saved tree viewports before measurement. Project the retained `TreeMeasurement` through `allocated` (`agent_cell.rs` ≈730) into the tallest frame interior, and get the minimum width from it. Then resolve the shared metrics for each allocation before you install the selection areas. Record the final painted header spans, Compressed headers included; `FittedTreeColumns` is not the final header.
- Phase 27 closed these outcomes: `TileWidthFloor` is visible to the application through placements and its floor tests. `ColumnPlan::NoAdmissibleColumn` is an internal choice that maps to the generic summary-only display. `TreeColumnFit::TooNarrow` is an internal request for core compression. Neither needs a new user message.
- Phase 27: `summary::truncated_line` (`summary/mod.rs` ≈846) cuts styled text by terminal cells and grapheme clusters, and `SummarySessionTable::OmittedForWidth` (≈226) removes session lines (its 34x50 regression is green). Keep that painted row mapping across resize and intervening input. Phase 20 owns the word-target test.
- Phase 24 motion: tui_pane `TileCells::prepare_moving_frame(destination, motion_bounds)` (`tiles/draw.rs`; the default calls `prepare_frame`) and `TileGrid::motion_preparation_allocations` (`tiles/grid.rs`) give destination positions and widths with the taller of the source and destination heights; cargo-handler `Cells::prepare_moving_frame` (`crates/cargo-handler/src/render/mod.rs` ≈697) passes those motion bounds to `prepare_frame` to measure role widths, and cargo-tile keeps the default. Keep these preparation bounds for measurement; record hit regions and header spans from the pieces actually painted, not from preparation allocations. Test: a shrinking tile clicked during motion selects the header drawn under the pointer, and the existing moving-role tests (`render/moving_role_tests.rs`) stay green.
- Phase 4's narrow headings fit through `HeadingLocation::{Shown,Cut,Omitted}` (`agent_cell.rs`); test clicks against the fitted heading actually drawn, not the full one.
- After phase 5 the summary's demand changes with each accounts reading.
- Phase 28 (a35577ad) supplies the painted record: `PaintedCells::{Undrawn, Drawn}`, a stable `CellIdentity` per painted piece, `CellHit::{Missed, Retired, Current}` (`tiles/grid.rs` ≈3292 `record_painted_cells`, ≈3306 `cell_hit`, reverse paint order) and `content_click` (`tiles/host.rs` ≈176), which maps a painted frame back to source coordinates for presses and wheel steps. `handle_tile_click` absorbs `TilePick::RetiredCell` before focus or row selection. `cell_content_at` is gone. Extend this record; do not create another painted-geometry record. The content geometry in the record comes from Phase 56's one visibility-aware content allocation (`content_area`, `tiles/draw.rs` ≈928), the same one that drawing and `content_click` read; do not compute a second content rect.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Phase 3's tile identity is `tiles::TileKey::{Agent(AgentCell), Unassigned { machine }}` (sorts in `App::tree_sorts: HashMap<TileKey, TreeSort>`, pruned by `agent_cell::tile_keys`), and header layout carries `agent_cell::SessionHeaderLead::{OverBranch, Flush}` (the pool's session header sits over flush names). Drawn header spans are kept per `TileKey`, with the lead they were drawn with (`render/mod.rs` ≈443 `Cells::tile`, ≈448 `Cells::header_spans`; `agent_cell.rs` ≈2401 `header_spans`, ≈2164 `unassigned_header_spans`).
- Accounts-header click: phase 32 already keeps the painted summary. `App` holds `PaintedSummary::{NotDrawn, Drawn}`, and `render::cell_clicked` reads the accounts header spans from it (`render/mod.rs` ≈260), then calls `AccountsSort::select` (`sort.rs`). This phase extends that record with the painted clipping, the scrolled and pinned line mapping and the drawn identities; it does not rebuild headers from the current reading. Keep the census-change and the several-inputs-before-redraw tests.
- Phases 16 and 20 add row and word clicks that can also hit changed content between draw and click: the frame record covers drawn row identities, clipped scrolled rows, wheel targets and word cells, tile titles and retired tiles included. Phase 20 resolves one press once, into both the row action and the painted word, and forwards tile title presses from tui_pane; keep both.
- Public `TileGrid::cell_at` still answers from settled geometry, and `cell_hit` calls it on the before-first-paint path.
- Phase 38: `update_layout_rules` (`tiles/grid.rs` ≈2106) is the only writer of area, growth, minimum width and height, width floor and summary allocation. A real change calls `replan_layout_endpoints` (≈2120): it installs the final arrangement at once, clears the queued steps and settles the motion. The painted record stays as the last frame drew it until the next paint. `complete_arrangement` (≈2779) is the one step that installs slots, and it settles focus, held focus and held rows against the slots it replaces; do not install slots in another place. A test that holds a motion mid-way must not change the area or these rules during the hold, because that settles the motion.
- cargo-handler `Cells::prepare_moving_frame` (`render/mod.rs` ≈697) supplies the motion measurement bounds to `Cells::prepare_frame` (≈705). `measure_trees` (≈458) and `restore_tree_viewports` (≈570) are in the same file. `prepare_frame` resolves the selections, runs `agent_cell::shared_tree_metrics`, and only then installs each selection area (`SurfaceSelection::set_area`). Keep that order. Those areas come from preparation allocations, so they are not the final clipped paint record.
- `draw_panes_at` (`render/mod.rs` ≈151) follows a moved selection: after the draw it finds the destination placement, takes the centre of its rect and calls `app.tiles.cell_at` (≈218) to get the index for `focus_cell`. When `cell_at` answers from painted geometry, that centre can name another moving piece.
- The pointer payload `tui_pane::CellClick<Id>` (`tiles/host.rs` ≈35, exported in `tiles/mod.rs` ≈48 and `lib.rs`) carries presses (`TileGridHost::cell_clicked`) and wheel steps (`TileGridHost::cell_scrolled`, `handle_tile_scroll`). Its users: `crates/cargo-handler/src/app.rs`, `render/mod.rs`, `render/accounts_grid_tests.rs`, `render/role_title_tests.rs`, `summary/accounts_header_sort_tests.rs`, `summary/unassigned_summary_tests.rs`, and `crates/tui_pane/src/tiles/host_tests.rs`.
- Phase 28 replaces the bare `Option` results of the selection API (`selected_span`, `containing`, `follow_focused`, `surface`, `surface_mut`, `Cells.focused`, `selection_surface`, `Cells::with_selections`) with named results. Use those names; do not add a new bare `Option`.
- Phase 28 pins the accounts heading and column headers. Pinned header lines and scrolling body rows have different screen-to-content mappings: a pinned line maps to a fixed content line, and a body row maps through the scroll offset below the pinned lines. Keep that whole mapping (pinned lines, body offset and pinned count) with the painted frame, not only the scroll offset.
- Phase 16's selected-row behavior (keys, clicks and the wheel; `render/selection_tests.rs`) stays as it is.
- Phase 36: the foot readout reads the selection after the cell paints (`draw_cell_with_rows_readout` calls `TileCells::rows_readout`, `tiles/draw.rs`), and cargo-handler `Cells::rows_readout` (`render/mod.rs` ≈764) answers from `app.selections`. Keep that order when the frame record changes, so `content rows: N of M` follows the selection during motion; `selected_rows_readout_follows_moves_and_keeps_unselected_counts` (`render/mod.rs`) stays green.
- Tests in tui_pane tiles live in the inline `mod tests` of `tiles/grid.rs` (≈6429, motion tests), `tiles/draw.rs` (≈961, motion content tests) and `tiles/host.rs` (≈205), and in `tiles/host_tests.rs` (registered at `host.rs` ≈446). There is no `tiles/tests/` directory.
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
- Title padding today: cargo-tile supplies `" summary "` (`crates/cargo-tile/src/constants.rs` ≈211); cargo-handler supplies `" summary"` and puts one space before group titles. Item 5a puts the padding in one owner in `tiles/draw.rs` and tests both supplied forms; after it, hosts supply bare titles.
- Phase 58 changes `render/selected_ink_tests.rs` ≈207 and ≈325 to the unfocused selection ruling; "unchanged expectations" below means unchanged from Phase 58.
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
5a. A tile title has one space on each side inside its frame line ("┌ summary ───"); today it has one space before and none after (Phase 48 design check, 2026-10-30, routed from Phase 48).
6. cargo-handler says "tile" everywhere, the framework navigation entries and bar labels included ("Next tile", "tab tile").
7. Key names use one form: lower case, with the spellings the bar already uses ("esc", "enter", "pageup", "shift-tab"). The keymap file keeps accepting its current spellings.
8. The cargo-port bar uses the same verbs as the cargo-tile and cargo-handler bars: "↑/↓ move", and "=/- expand all" / "collapse all" or the shared equivalent.
9. One shared helper formats the sccache hit-rate percent: one decimal, no space before the sign ("85.4%"). It lives in `crates/tui_pane/src/format.rs`, exported as `tui_pane::format_hit_rate(raw: &str) -> Result<String, MalformedRate>` beside `format_progressive` (`lib.rs` ≈292). Both parsers call it: cargo-port `src/sccache.rs` `summary_from_lines` (≈38; it keeps the raw string today) and cargo-tile `src/sccache.rs` `closed_percent` (≈368; it removes spaces with no rounding). Malformed input (no finite number) gives `MalformedRate`, and each parser treats it as a missing rate field (`SccacheSummary::Unavailable`). Tests: "85.40 %" gives "85.4%", "4.90 %" gives "4.9%", "100 %" gives "100.0%", and "n/a" gives `MalformedRate`. The cargo-port selected-ink repair (item 10) is owned by `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` (the selected row style ≈1229), with the phase 16 lift moved from cargo-handler `footer_colors/mod.rs` (`SelectedGroundInk` ≈410) into tui_pane so both apps use one method.
10. cargo-port's selected pane row is readable on the Default Dark selection ground rgb 86,86,86 (showrunner, 2026-10-23). Live at phase 18: CI circle green (5,197,5) 2.55:1, Og glyph gray (162,162,162) 2.42:1, Main ☑ green (21,175,21) 2.10:1, disk text (255,212,212) 4.25:1. Use the phase 16 method: an OKLCH lightness-only lift of each ink until it reaches 4.5:1 at 208/255 coverage on the selection ground, with its hue kept and no colour-distance rule on selection grounds. Emoji glyphs (the Git crab) are exempt, because their colour is not ours to set. Shared lift boundary: the shared implementation is `crates/tui_pane/src/theme/selected_ink.rs`, exported from `theme/mod.rs` and `lib.rs`. Move only the selected-ground colour calculation into it: the OKLCH lightness search for one selection ground, the hue and chroma path it follows, the OKLCH conversion it uses (`Oklch::from_color` and `SelectedHueChroma`, exported as the one measurement API; cargo-handler's `render/selected_ink_tests.rs` and `footer_colors::tests::oklch_ink` move to it, and no second conversion stays in cargo-handler), and the two-way 208/255 covered contrast it measures with (the lower of the unrounded and the byte-rounded blend, as phase 18 built). It is a pure function of the ink and the ground, with no cache and no theme state. cargo-handler's `InkCoverage::Antialiased` calls the same covered-contrast function, so there is one covered-contrast calculation. The resolver names its outcome by type: a lifted ink that reaches 4.5:1, or an unreachable target that carries the best ink it found (the search end with the higher two-way contrast). It never returns an unmeasured first candidate. Callers draw the carried best ink for an unreachable target; no built-in theme ground is unreachable today, and a test proves that for every built-in selection ground. Cache invalidation stays with each app: cargo-handler keeps `SelectedGroundInk` as a wrapper over the shared function, with its `PALETTE` cache and the theme-identity and ground checks in `ReadablePalette::current`; cargo-port calls the shared function and owns any cache it adds. Keep `SessionRowInk`, the footer assignments (`FooterColors`), the pane lightness (`ReadableInk`) and the reset ranking (`ResetRank`) in cargo-handler. Unselected session inks do not change.
11. Tests, each red before its fix: a rendered overlay has a full-width selection bar, sentence-case title and headings without colons, no repeated title heading, a blank row before each later heading (fit and paging heights counted), and no row without a key; the rendered cargo-handler bar and overlay say "tile" and never "pane"; every key name drawn on a bar or the overlay is lower case; the cargo-port bar draws "move" and the expand/collapse verbs; the sccache helper formats 85.40 as "85.4%". A rendered cargo-port pane with a selected row reads every non-emoji ink on that row at 4.5:1 or more with its own literal 208/255 coverage, in all four themes (named test `selected_project_row_inks_read_at_4_5_in_every_theme` in `crates/cargo-port/src/tui/panes/project_list/selected_row_ink_tests.rs`). The shared resolver has its own tests in `crates/tui_pane/src/theme/selected_ink_tests.rs`: an ink under 4.5:1 lifts to 4.5:1 with its hue kept, an ink already at 4.5:1 stays unchanged, and on ground rgb 127,127,127 the resolver gives the unreachable outcome with the end of higher two-way contrast. cargo-handler's `render/selected_ink_tests.rs` passes unchanged, and an unselected session row draws the same inks as before.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — the tile title spacing (5a) and its rendered test
- `crates/tui_pane/src/overlays/global_shortcuts.rs`, `crates/tui_pane/src/overlays/global_shortcuts_tests.rs`, `crates/tui_pane/src/keymap/mod.rs`, `crates/tui_pane/src/keymap/key_bind.rs`, `crates/tui_pane/src/bar/nav_region.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/keymap.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-tile/src/constants.rs` — callers that stop supplying title padding (5a): `SUMMARY_CELL_TITLE` ≈87 and the agent title lead below it, `group_title` ≈914, and cargo-tile `SUMMARY_CELL_TITLE` ≈213
- `crates/cargo-port/src/tui/sccache/status_line.rs`, `crates/cargo-port/src/sccache.rs`, `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs`, `crates/cargo-port/src/tui/keymap/`, `crates/cargo-port/src/themes/`, `crates/cargo-port/CHANGELOG.md`
- `crates/tui_pane/src/format.rs`, `crates/tui_pane/src/lib.rs`, `crates/cargo-tile/src/sccache.rs`, `crates/cargo-handler/src/footer_colors/mod.rs`
- `crates/tui_pane/src/theme/selected_ink.rs` — new shared selected-ground resolver, exported from `crates/tui_pane/src/theme/mod.rs`
- `crates/tui_pane/src/theme/selected_ink_tests.rs` — new resolver tests, registered in `theme/selected_ink.rs`
- `crates/cargo-handler/src/render/selected_ink_tests.rs` — the handler selected-ink tests, unchanged expectations
- `crates/cargo-port/src/tui/panes/project_list/selected_row_ink_tests.rs` — new cargo-port selected-row render test, registered in `tree_rows.rs`
- `crates/cargo-handler/src/footer_colors/tests.rs` — `oklch_ink` moves to the shared measurement API

**Workers:** 1 writer + 1 tester.
- `impl` — `tiles/draw.rs` title padding and its inline test, the callers that stop padding titles (both `constants.rs` files and handler `render/mod.rs` `group_title`), overlay, keymap, bar, the shared formatter, both sccache callers, the shared resolver module (`theme/selected_ink.rs`) and its exports, the application callers (cargo-handler `SelectedGroundInk`, cargo-port `tree_rows.rs`), the cache integration in `footer_colors/mod.rs`, the registrations and the CHANGELOGs; hub: `crates/tui_pane/src/lib.rs` and `crates/tui_pane/src/theme/mod.rs` (exports).
- `test` — the external rendered tests, the separate resolver tests (`theme/selected_ink_tests.rs`), the handler selected-ink tests (`render/selected_ink_tests.rs`), the named cargo-port selected-row render test (`selected_row_ink_tests.rs`), and the move of `footer_colors::tests::oklch_ink` and the selected-ink tests to the shared measurement API.

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

### Phase 37 — Band dividers never get fewer rows than their minimum gaps  · status: done

#### As-built

- Checkpoint 6733d610. Phase 27 let `fits` accept `ColumnPacking::WidthLimited` when one tile height fit, and `column_layout` put every position into the columns left, so a column could hold more residents than its rows admit (20 cells in two columns of ten at 80x47, floor 6). The motion pass then panicked at the `band_piece_dividers` clamp ("min > max. min = 5, max = 1").
- `RowAdmissibleColumnLayout` (private, `grid.rs`) is built only by `ColumnLayout::checked`. `ColumnPlan::{Columns, InsufficientWidth, InsufficientRows}`. N residents need N × (min_tile_height − 1) + 1 rows; the summary counts once in column zero with its held depth; the band below a widened summary is checked on its own; no saturating arithmetic. `column_layout` and `ColumnLayout::for_summary` return the checked layout after every redistribution; `fits` (no WidthLimited height exemption), `holds_in`, admission, summary share and depth and `queued_slot_column` read it. Commands that do not fit stay in demands and the summary. `band_piece_dividers` is unchanged.
- Tests: `width_limited_admission_keeps_resident_motion_within_the_band_budget` (fail-before, panicked at HEAD), `width_limited_columns_keep_the_exact_resident_capacity`, `summary_redistribution_checks_the_remaining_resident_band`. The handler 34x50 fixture count is 2 group tiles (was 20), summary 43.

**Files:** `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-handler/src/render/narrow_width_tests.rs`

**Binds later work:** every column layout goes through `ColumnLayout::checked`; a width policy (`CompressSingleColumn`) never relaxes the height floor. A cached motion endpoint after a floor change belongs to "Motion endpoints keep the band floor after a floor change".

### Phase 38 — Motion endpoints keep the band floor after a floor change  · status: done

#### As-built

- `TileGrid::update_layout_rules(area, EndpointLayoutRules)` (private, `grid.rs`) is the only writer of area, growth, minimum tile width and height, width floor and summary row allocation. `set_layout`, the settings setters, `sync_demands` and `apply` call it. A real change calls `replan_layout_endpoints`: it drops cached admission, installs the target arrangement, clears the queued steps, settles the motion, clears settled geometry, row usage, motion heights and the retargeted destination, and rebuilds held rows.
- Each motion endpoint carries `EndpointLayoutRules { growth, min_tile_width, min_tile_height, width_floor, summary_allocation }`, the rules it was checked under; an endpoint with other rules is not reused as a motion source. `CheckedLayoutAdmission { area, rules, fits }` caches admission of the held, moving and queued arrangements, so unchanged frames do no new validation.
- `complete_arrangement(slots, depth) -> Vec<Slot<Id>>` is the only whole-slot install: replanning, `advance` and `open_waiting_commands_in_current_motion` call it. It settles focus, held focus and held rows against the replaced slots; `advance` keeps the returned slots as its motion source.
- `band_piece_dividers` is unchanged. Tests: `a_floor_change_rechecks_the_painted_widened_summary_endpoint` (the 80×47 widened-summary case, fail-before), `a_floor_change_discards_queued_endpoints_and_measured_motion_heights`, `an_area_change_discards_queued_endpoints_and_measured_motion_heights`, `a_growth_change_during_demand_motion_keeps_every_group_visible`, `a_layout_growth_change_replans_the_motion_endpoints`, `an_action_growth_change_replans_the_motion_endpoints`, `unchanged_motion_frames_reuse_endpoint_admission`, `a_resize_during_queued_closings_settles_focus_and_keeps_navigation_working`.

**Files:** `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/CHANGELOG.md`

**Binds later work:** a rule or area change mid-motion settles the motion at once; the painted record stays as the last frame drew it until the next paint ("Accurate clicks during motion" holds motion mid-way only with area and rules unchanged).

**Gotchas:** never assign an area or layout rule field directly: the next `set_layout` then sees no change and the stale endpoint is never replanned. Never install slots outside `complete_arrangement`: another path leaves focus on a removed group (`Departing`), and Tab and arrows stop.

**Ruled out:** a clamp guard, bound swap or lower floor at `band_piece_dividers`; revalidating each endpoint on every frame (the admission cache replaces it).

### Phase 40 — cargo-handler idles under 1 % of one core  · status: done (landed at 9.7 % against 13.2 % installed; the under-1 % target moved to Phase 47)

#### As-built

- One sysinfo `System` serves each census scan and one serves the hierarchy. Each refreshes in place. A same-pid exec shows through `CurrentProcessNames`.
- Parsed Codex run records stay in a cache keyed by file revision (path and mtime). A scan reads only new or changed files. A removed file leaves the cache.
- `accounts.py` and `production_tree.py` run only when their inputs change (`CommandInputs`). Output rows are unchanged.
- Idle release CPU is 5.81 CPU-s/min (9.7 % of one core) against 7.91 (13.2 %) installed. The under-1 % target is not met.

**Files:**
- `crates/cargo-handler/src/census/{activity,scan,hierarchy,accounts,schedule}.rs` — run record cache, per-scan `System`, input-driven commands
- `crates/cargo-handler/src/census/command_inputs.rs` — `CommandInputs`; a `#[path]` child of `hierarchy.rs`
- `crates/cargo-handler/src/census/activity_regression_tests.rs` — parse-count and preservation tests

**Binds later work:** the under-1 % idle target is open. The ancestry `System` refresh costs 1.14 CPU-s/min (budget 0.60). Run directory listing costs 1.20. On macOS, `CurrentProcessNames::read` runs `/bin/ps -axo pid=,ucomm=` on every scan. Model and effort display must read through the revision-keyed run records, with no per-scan read.

**Gotchas:** three existing tests run over 1 s: gradient (1.46), busy ink (1.27), codex tier (1.04).

### Phase 39 — A project whose parent is deleted shows as deleted  · status: done (not reproduced, no code change)

**Result (2026-10-28):** the c58 report did not reproduce on natedev HEAD 8c718383. Four live cases (a corrupt-HEAD and a valid-HEAD project, each made after cargo-port started and before it started, parent removed by Python `shutil.rmtree` one minute after the row appeared) all showed the existing Deleted strike-through within 0.55–0.89 s, and `x` dismissed each row. No cargo-port file changed. One finding for port-unit: with `NO_COLOR=1` in its environment, cargo-port writes SGR 9 and then an empty colour sequence (`ESC[;m`, crossterm `SetColors` under NO_COLOR), which resets the strike-through, so a Deleted row looks normal. This made the first scratch capture look like the defect; it is not proven that the user's cargo-port had NO_COLOR set. Evidence: unit session `phases/39/live/terminal_env_clean/`. From 2026-10-28 port-unit owns all cargo-port work.

### Phase 36 — Selected tiles show "content rows: N of M"  · status: done

**Depends on:** Phase 27

#### As-built

- tui_pane `RowsReadout::{Count(usize), Selected(SelectedRow)}` (`tiles/draw.rs`). `SelectedRow` keeps `row: NonZeroUsize` and `rows: usize` private, with `row()` and `rows()`; only `RowsReadout::selected(row, rows) -> Result<Self, RowExceedsTotal>` builds it, and it refuses `row > rows`. `RowsReadout`, `SelectedRow` and `RowExceedsTotal` are exported from `tiles/mod.rs` and `lib.rs`.
- `TileCells::rows_readout(&self, content, area, measured_rows) -> RowsReadout` defaults to `Count(self.readout_rows(..))`, so cargo-tile and every unselected path draw today's `content rows: 12`. `Selected` draws `content rows: {row} of {rows}`: label ink on `content rows: ` and ` of `, reading ink on both numbers; `r/c: H/W` stays. The readout row, its demand and the content area are unchanged.
- `draw_cell_with_rows_readout` reads the readout after the cell paints. A private `CellRowsReadout { text, content_rows }` keeps the measured content rows for the fit ink; the selected total is text only.
- cargo-handler `Cells::rows_readout` (`render/mod.rs`) returns `SurfaceSelection::rows_readout(measured_rows)` (`interaction.rs`) for a selectable surface (tiles and the summary), else `Count`. The total is the surface's selectable rows; a position past them falls back to `Count`. The readout follows every arrow-key move and scroll.
- Tests: `selected_rows_readout_follows_moves_and_keeps_unselected_counts` (`render/mod.rs`, real Down-arrow dispatch), `selected_rows_readout_checks_bounds_and_preserves_count_text` and `selected_rows_readout_uses_measured_content_for_fit_ink` (`tiles/draw.rs`).

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — readout types, hook, text and fit ink; tests
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — exports
- `crates/cargo-handler/src/render/mod.rs` — `Cells::rows_readout`, readout fixtures, move test
- `crates/cargo-handler/src/interaction.rs` — `SurfaceSelection::rows_readout`
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md`

**Binds later work:** the readout reads the selection after the cell paints, so a changed frame record keeps that order ("Accurate clicks during motion"); the foot readout is chrome and not a word target ("Click a word to highlight every instance").

**Gotchas:**
- The fit ink compares measured content rows, never the selectable total: a wrapped account can clip while its selectable count fits.
- The selected total counts selectable rows, so a tile with headings reads `content rows: 12` unselected and `1 of 10` selected.

**Ruled out:** the hidden-readout design (no readout, a reclaimed last content row); public fields on `Selected` (safe code could break `row <= rows`); `From<usize> for RowsReadout` (no consumer).

### Phase 35 — Subagent role words  · status: todo

**Depends on:** Phase 27, Phase 36

#### Work Order

**Goal:** a Claude subagent row shows only its role word ("writer", "reviewer"), from a closed set, never the agent's own name with its number or phase prefix.

**Source:** change review C67 (showrunner, 2026-10-27). User: "under the berth unit - i see p36-reviewer - it should just say reviewer" and "these two subagents should just say writer not writer77 and writer78". Showrunner rule: the role comes from a closed set at the type level, never by trimming name strings. The reset colour work of change review C71 moved to Phase 53 on 2026-10-30.

**Constraints from prior phases:**
- `census/scan.rs` belongs to Phase 47 until its checkpoint; keep Phase 47's CPU acceptance test green.
- A named teammate's meta file has `agentType` equal to its `name` (for example `{"agentType":"writer77","name":"writer77","spawnedAgentType":"general-purpose","taskKind":"in_process_teammate"}`); for a plain Agent call, `agentType` is the subagent type ("general-purpose", "Plan", "Explore"). Names are free-form: writerN, reviewN, shotsN, uxN, implN, fixN, p36-reviewer, judge.
- `census/transcript.rs`: `SubagentMeta` (≈591) reads only `description`, `agent_type` and `parent_agent_id`. `SubagentRole` (≈500) is `{Capture, Review, Lint, Test, Survey, Specialist(SpecialistRole), Unrecorded}`; `SpecialistRole(String)` (≈514) stores the name, which is the defect. `from_record` (≈534) and `word` (≈546) parse it; `text` (≈557) returns the stored string.
- `census/tree.rs` `subagent_node` (≈657) puts the role in `ChildKind::Subagent(SubagentRole)` (`census/mod.rs` ≈388) and the description in the session column.
- `census/probe.rs` `ProbeChildWork.subagent_role` (≈96) serializes the enum across machines; `{"Specialist":"showrunner"}` and `"Review"` are on the wire today.
- `ProbeChildWork` (`census/probe.rs` ≈96) serializes `SubagentRole` directly. The checkpoint enum (`census/transcript.rs` ≈500) accepts `Review` and `Specialist`. It does not accept `Reviewer`, `Writer` or the other new variants.
- `render/narrow_width_tests.rs` (≈360) and its `growth_app` (≈430) use a fifty-character specialist role, so that restored or newly visible rows change the width requirement (checked at ≈488 and ≈534). A change of that fixture to `Agent` removes the tested condition. Use supported role words and fixture widths that still test off-window width growth, End/Home/cursor restoration and the first enlarged frame. Keep the whole-core and below-core compression checks, the longer Astra labels included.
- `agent_cell.rs`: `role_text` (≈1192) draws `role.text()`; the role ink comes from `colors.named_role(role.text())` (≈1349, `footer_colors/mod.rs` ≈839).
- Session-row plan reviewers show "review" through `RoleLabel::ReviewFallback` (`census/placement.rs` ≈86), and Codex seats through `WorkerRole::text` (`census/worker_dispatch/mod.rs` ≈100); `WorkerRole::text` also feeds role matching, so the parsed words stay.

**Spec:**
1. `SubagentRole` becomes a closed enum with no string payload: Writer, Reviewer, Impl, Test, Fix, Capture, Lint, Survey, Plan, Explore, Showrunner, and `Agent` for any name that matches no word. `text()` is a `const fn` returning a `&'static str`; the drawn word is always the variant's word, never part of a name. Reviewer draws "reviewer".
1a. One word for one role (showrunner, 2026-10-27): session-row plan reviewers (`RoleLabel::ReviewFallback`) and Codex review seats (`WorkerRole::Review`) also draw "reviewer". Only the drawn word changes; the words read from seat names, leases and hierarchy files ("review") still parse.
2. The role is decided once, at the meta file boundary (`running_subagents`). `SubagentMeta` reads `name` and `spawnedAgentType` too. Source order: the name, then `agentType`, then `spawnedAgentType`, then the description words. A name splits on non-letters into whole tokens, and a token matches a role word exactly (case-insensitive, with the listed synonyms: "review" → Reviewer, "impl" → Impl). "p36-reviewer" is Reviewer, "writer77" is Writer, "judge" is Agent. A general type ("general-purpose") gives no role.
3. The role ink comes from a typed method on `SubagentRole`, not from looking up the drawn text.
4. Wire: decode the old role forms (`{"Specialist":"<word>"}`, `"Review"`) into the closed domain enum: the closed role for that word, else Agent. Encode new roles through a wire form that older builds accept, or through a defaulted extension. Keep any string payload inside the wire adapter, so the domain model gets no `SpecialistRole(String)` back. No string role is stored after deserialization.
5. Tests, red first: rendered subagent rows for writer77, writer78, p36-reviewer, review3, judge and a plain "Plan" subagent show "writer", "writer", "reviewer", "reviewer", "agent" and "plan"; a plan-mode reviewer session row and a Codex review seat row show "reviewer"; an old report on the new receiver, and a new report on the checkpoint receiver; callers that built a role from a free string build a variant. Every test under one second.
6. CHANGELOG: one line that subagent rows show their role word, not the agent's name.

**Files:**
- `crates/cargo-handler/src/census/transcript.rs` — closed `SubagentRole`, meta fields, role source order
- `crates/cargo-handler/src/census/probe.rs` — old wire form
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/footer_colors/mod.rs` — role word and typed role ink
- `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/scan.rs` — variant construction
- `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — "reviewer" for session-row and seat reviewers
- `crates/cargo-handler/src/census/reviewer_tests.rs`, `crates/cargo-handler/src/census/claude_seat_tests.rs` — the drawn reviewer word
- `crates/cargo-handler/src/census/subagent_activity_tests.rs`, `crates/cargo-handler/src/census/probe_role_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/subagent_row_tests.rs` — spec 5 tests
- `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/repair_tests.rs`, `crates/cargo-handler/src/render/moving_role_tests.rs`, `crates/cargo-handler/src/render/selected_ink_tests.rs`, `crates/cargo-handler/src/render/selection_tests.rs`, `crates/cargo-handler/src/render/narrow_width_tests.rs` — callers build variants
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — spec 1, 1a, 2–4 and 6: production files, registrations and inline test migrations in production modules; hub: `crates/cargo-handler/src/census/transcript.rs`.
- `test` — spec 5: the listed separate test modules, including `narrow_width_tests.rs` and `moving_role_tests.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` clean. No test takes a second. Live: a shot of a tile with named writer and reviewer subagents shows "writer" and "reviewer" in the role column. The checkpoint adds a Change-Review line.

### Phase 31 — Session colours stay apart  · status: todo

**Depends on:** Phase 18, Phase 27, Phase 30, Phase 35, Phase 53

#### Work Order

**Goal:** in all four themes, the five session inks read as five different colours, and no session ink, role ink or reset shade on a row looks like another.

**Source:** next item "Session colours stay apart" (phase 26, approved by the showrunner 2026-10-23 19:30 AST), widened by phases 29 and 30. Showrunner ruling, 2026-10-24: the root cause is too many inks on one row. Run after Phases 36, 35 and 53, beginning with options and shots for the showrunner.

**Constraints from prior phases:**
- Phase 26 (dark): the valid hue domain is about 92.5°, and at most 2–3 inks can be CIEDE2000 >= 20 apart; steps of 45°, OKLCH C >= 0.10, and both together also fail.
- Phase 53 replaces the reset blend with 3 or 4 stepped reset colours per theme, soonest the strongest, in the one reset source `ResetRank::color` (`theme/mod.rs` ≈128). Read the drawn steps from `theme/mod.rs` and the theme files at start, and assess those steps, not the old ramp. The reset values below are from before Phase 53. This phase owns any later replacement of the session/reset floors, by new floors or by another visual channel.
- Phase 29 (light): light reset ends Default Light (115,0,235) → (73,0,158) and High Contrast Light (115,0,235) → (82,55,154) hold session-vs-reset dE 14.7 and 15.3 (`render/role_title_tests.rs`, `SessionResetFloor::HeldLight`). The drawn reset shades fall in one violet band (OKLCH h 291–294°).
- Phase 30 (light): no five-ink light session set exists under the current row floors, even with any one floor relaxed; the light defects are near-black session inks in two hue groups, the `showrunners` role ink #7732FF dE 5.3 from the nearest reset end, and reset steps of dE 2.7–3.4. `summary/accounts_gradient_tests.rs` holds the reset time order.
- Session rows draw only through `SessionRowInk::style()` (`theme/mod.rs` ≈58). `SessionRowInk::fixed_fields()` and `FooterColors::session_row_inks()` list every ink a row can draw. Root i takes entry i (mod 5) of `session_hues`, and each tile title keeps its summary row's ink.
- Phase 21 adds two Astra role inks; this phase includes them in its assessment. Today `FooterColors::parse` (`footer_colors/mod.rs` ≈772) reads only `role.showrunner` and `role.unit-director`, so the `role.astra-*` keys are ignored, and `FooterColor` has no `mauve` variant. Assess the prospective sources by value: designer `mauve` = xterm 139 (#af87af), reviewer `bright-green`. Role inks resolve through `FooterColors::named_role` (≈818) and `FooterColor::management_style` (≈68), drawn as `SessionRowInk::Management`.
- The accounts table draws role words through `role_words` (`summary/accounts.rs` ≈609): normal management ink for a fresh reading, subdued management ink (`Freshness::style`) for a stale or unread one. Reset shades draw after the readability correction (`footer_colors::readable_status_style`), not as raw `ResetRank::color`.
- Covered contrast (phase 18): production `InkCoverage::contrast` (`footer_colors/mod.rs` ≈367) and the oracle `session_glyph_contrast` (`render/role_title_tests.rs` ≈681) both take the lower of the unrounded and the byte-rounded 208/255 covered contrast; the floors here are measured that way. Phase 18 moved the light session ink 0,35,86 to 0,34,85 (`theme/mod.rs` ≈351, ≈389; `themes/default_light.toml`, `themes/high_contrast.toml`): dE 15.21 from tinted light Account(Blue) 0,69,175, chroma 0.1008.
- Phase 27's column drops and truncation keep the style of each span they cut.

**Spec:**
1. Options first (showrunner, 2026-10-24): list what else can separate sessions. At least: fewer fixed inks on a session row (for example role, account or status drawn in the plain text ink), and a second visual channel the style guide allows (for example bold, a glyph or a background tint). For each option, give the inks a row draws, the measured smallest session-pair dE, session-vs-fixed dE and session-vs-reset dE in all four themes, and what the row loses. Include the accounts table: its role inks in the fresh and the subdued (stale or unread) style, against the drawn reset shades, with pane tint on and off. Each option also says how a user tells reset ranks apart with six or more reset times, where Phase 53's 3 or 4 steps make several ranks share a step: the stepped colours alone, a second channel, or the time text, measured. Each option also says how its separating channel survives the selected row band and the phase 20 word-match background (one background colour for every match, drawn over the selection), for example a glyph or bold that a background does not erase. Send the showrunner the options with shots of the summary agents table and the accounts table in a dark and a light theme, before any palette code. The showrunner's choice settles the rest of this spec. The unit director then adds the files the chosen option needs to **Files**.
2. The showrunner's choice sets the final floors and the tests that hold them. A second visual channel separates by something other than colour, so the choice names which colour floor it replaces and the test that proves the channel instead. Measure the chosen option before production code. If no palette meets the chosen floors, report the measured values to the unit director and stop; never land a palette from an empty feasible set (phase 30's first fallback collapsed five inks to one). Without a replacement named by the choice, the completion condition holds: in all four themes, every session ink has CIEDE2000 >= 15 from each fixed ink that can share its row, every pair of session inks has CIEDE2000 >= 20, and covered contrast is >= 4.5:1 with the two-way oracle (the lower of the unrounded and the byte-rounded 208/255 blend). In both light themes, session-vs-reset CIEDE2000 is >= 20 within 45° of hue. Phase 53's reset steps keep dE >= 15 from every role ink, in the fresh and the subdued style.
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
- `census/hierarchy.rs` belongs to Phase 47 until its checkpoint.
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
- Phase 56: the readout is hidden by default (`RowsReadoutVisibility::{Hidden, Shown}`), and a hidden readout gives its row back to content when the host has no foot content. Drawing and `content_click` read Phase 56's one visibility-aware content allocation (`content_area`, `tui_pane/src/tiles/draw.rs` ≈928). Word targets and the empty-space clear map through that same allocation in both readout states.
- Phase 36: the readout text comes from `TileCells::rows_readout` (`tui_pane/src/tiles/draw.rs`): `RowsReadout::Selected` draws `content rows: 3 of 12` for a surface with a selected row, `RowsReadout::Count` draws `content rows: 12`. The readout is chrome, so a press on it is not a word target.
- The content, title and chrome outcomes cross the public host boundary. The pointer payload is exported in `tiles/mod.rs` (≈48) and `lib.rs` (≈502).
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
9. Tests also cover: a word on an account continuation line; a missed row click that clears the highlight and keeps the row selection; a word in a tile title; a click on the status bar or a readout that leaves the highlight; a click while a modal overlay is open that goes to the overlay; Esc with an overlay open that closes the overlay and keeps the highlight. A retired painted tile leaves focus, row selection and word highlight unchanged; a word on a pinned heading or column header is read from the painted cells without breaking header sorting. Words resolve from the visible painted token when it holds wide glyphs, combining marks, emoji presentation sequences or a truncation ellipsis; a click on any cell that a wide glyph occupies resolves the same token; omitted session text gives no word target. Shared tui_pane regressions in `tiles/host_tests.rs` cover title forwarding, chrome exclusion (the foot readout row included) and retired content.

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
- Phase 41 changed the Codex startup query: it accepts user threads with a non-exec source (`constants.rs` ≈387) and keeps an older-schema query (≈392), chosen by `ThreadsLayout::{WithThreadSource, OriginatorOnly}` (`census/codex.rs` ≈249). The separate lookup for Astra exec threads stays required.
- `census/scan.rs` belongs to Phase 47 until its checkpoint.
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
- The Codex startup lookup reads through the typed layouts Phase 41 shipped. `ThreadsLayout::WithThreadSource` runs `CODEX_THREADS_QUERY` (`constants.rs` ≈387: `thread_source = 'user'` with `source` not `'exec'`, or `codex-tui` rows with no `thread_source`). `ThreadsLayout::OriginatorOnly` runs `CODEX_LEGACY_THREADS_QUERY` (≈392: `codex-tui` only). Both leave out exec threads and worker threads at startup. `astra_review.py` runs `codex exec` (`originator` `codex_exec`), so its thread is not in the startup result.
- The wire and lease token `review` stays accepted; displayed reviewer words follow Phase 35.
- `render/narrow_width_tests.rs` (≈360) and its `growth_app` (≈430) use a long role so that restored or newly visible rows change the width requirement (checked at ≈488 and ≈534). Use supported role words and fixture widths that still test off-window width growth, End/Home/cursor restoration and the first enlarged frame. Keep the whole-core and below-core compression checks, the longer Astra labels included.
- `codex::read_threads_by_id` (`census/codex.rs` ≈175) already accepts any originator (`CODEX_THREAD_BY_ID_QUERY`). Keep the startup exclusion of exec and worker threads. Do a separate edge-named lookup for Astra exec threads in the named Codex home; do not widen the startup queries. Reuse `SessionNames` for renamed sessions; keep the phase 15 home and resume checks (`thread_matches_home`, `ResumeRequest`) and the no-dispatch regression.
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

### Phase 41 — Renamed Codex sessions show in cargo-handler  · status: done

#### As-built

An interactive Codex session that the user renamed shows its name in its tile and the summary, not `pid N`.

- `CODEX_THREADS_QUERY` selects an interactive thread by what it is: `thread_source = 'user'` with `source` not `'exec'`, or `originator = 'codex-tui'` on older rows that have no `thread_source`. Newer Codex writes originator `roster` on interactive threads, so the originator alone no longer finds them.
- `CODEX_LEGACY_THREADS_QUERY` serves a database that has no `source` or `thread_source` column. It reads through the `codex-tui` originator only.
- `enum ThreadsLayout { WithThreadSource, OriginatorOnly }` picks the query. The layout is checked once per database open, never per call.
- The match by cwd and the 60 s start window are unchanged. `CODEX_THREAD_BY_ID_QUERY` and `CODEX_LAST_THREAD_QUERY` still accept any originator.
- Tests cover a `roster` thread (returned), a worker thread with NULL `thread_source` (not returned), an exec thread (not returned), an `OriginatorOnly` table (still returns its `codex-tui` thread), and a `session_index.jsonl` name that replaces `pid N` on the row.

**Files:**
- `crates/cargo-handler/src/constants.rs` — `CODEX_THREADS_QUERY` and `CODEX_LEGACY_THREADS_QUERY`
- `crates/cargo-handler/src/census/codex.rs` — `ThreadsLayout`, the layout check at database open, and the thread-row fixture tests
- `crates/cargo-handler/src/census/codex_session_tests.rs` — the named-row test and the `ThreadsLayout` cases; `ThreadsLayout` is an implementation detail owned here

**Binds later work:** the work that reads Codex model and effort beside these thread rows keeps the once-per-open `ThreadsLayout` choice.

### Phase 42 — A dispatch shell folds into its Codex worker row  · status: todo

**Depends on:** Phase 40

#### Work Order

**Goal:** a dispatch shell (`implement.sh`, `codex_mesh.py follow` or `start`) never draws as its own row when its Codex worker row draws; only the worker row shows, with the dispatch job. The same holds for a Claude seat dispatch.

**Source:** change reviews C86 and C87 back (showrunner, 2026-10-28 ≈20:00 EDT). User: "there is no point to seeing the thing that dispatched the codex work for me - there is always one". One new Change-Review line covers both. Unit session `phases/42/source.md`.

**Constraints from prior phases:**
- `census/scan.rs` belongs to Phase 47 until its checkpoint; keep Phase 47's CPU acceptance test green.
- Live case on natedev (cables unit, claude2): the director's Bash call zsh 3399214 → `implement.sh` 3399229 → `codex_mesh.py follow` 3576715 draws a shell row "Dispatch the drag seat to run the Phase 54b tes…" beside its worker row "drag · Phase 33" (codex1). The follow process has no app-server child: that worker's app-server has systemd as its parent, because an earlier dispatch that has ended started it. A case that folds: zsh 2700456 → `implement.sh` 2700461 → `codex_mesh.py follow` 2949144 → `codex app-server` 1570363.
- Scan time: `worker_dispatches` (`census/scan.rs` ≈840) parses each launcher (`ParsedDispatch`, `census/worker_dispatch/mod.rs` ≈874), takes the seat from a `codex_mesh.py` process below it (`MESH_SEAT_FLAGS`, `process_is_below`), and resolves it through `mesh_roster.json` (`ParsedDispatch::resolve` ≈1117: `--to` seat, else the entry whose `launcher_pid` is the dispatch's pid) into `WorkerDispatch::Worker(ResolvedWorkerDispatch)` with `identity: WorkerIdentity::CodexThread(thread id)`.
- Draw time: `AgentPlacement::resolve_dispatches` (`census/placement.rs` ≈265) collects the children of each final tile; a `CodexThreadHost` row's child joins only when the hierarchy moved it (`moved`), else it is skipped. `tree::join_dispatches` (`census/tree.rs` ≈344) folds a shell only into a `ChildKind::Thread` row in the same tile whose `CodexThreadId::Recorded` equals the dispatch identity. `folds_dispatch` (≈307) hides the shell; `dispatch_label` (≈328) gives the worker row the job. Claude seats fold through `ClaudeDispatchFold` and `fold_claude_dispatches` (`scan.rs` ≈929).
- `census/scan.rs` is in Phase 40's **Files** and its working tree; this phase starts after Phase 40's checkpoint.
- Keep `pending_dispatch_to_a_different_seat_keeps_its_wrapper_visible` and `unresolved_and_cross_tile_dispatches_keep_their_shell_rows` (`worker_dispatch/tests.rs`) unless the fixture capture shows the cables case is one of them; then name the changed rule in the summary.

**Spec:**
1. Capture first: from the live process table, `mesh_roster.json` and `production_tree.py --json` of a follow dispatch whose worker's app-server it did not start, build a fixture, and name in the summary the step that drops the fold (seat resolution, the thread row's tile, or the join).
2. Pair by the dispatch's own identity (its `--to` seat or roster entry in its `--session-dir`, and the thread that seat records), not by process descent. The pairing is a type, for example `DispatchPairing { shell, worker, dispatch }` built once at placement, that the draw reads; no name filter in the draw. A shell whose worker row draws in its final tile folds into it wherever that worker's app-server sits in the process tree.
3. Claude seats (C87) use the same pairing: a seat dispatch folds into the seat's session row by the seat identity.
4. Tests, red first, in `census/worker_dispatch/tests.rs`: the captured cables fixture (follow with no app-server child, app-server under pid 1, thread placed under the director by its hierarchy edge) draws one row, the worker row with the dispatch job, and no shell row; the existing fold, pending and cross-tile tests stay green; a Claude seat case with the same shape folds. Each test under one second.
5. CHANGELOG: one line under Fixed.

**Files:**
- `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/tree.rs` — the typed pairing and the join
- `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — seat identity at scan time, Claude seat fold
- `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/fixtures/` — the captured fixture and tests
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — the capture, the pairing type, scan and placement changes and the CHANGELOG; hub: `crates/cargo-handler/src/census/placement.rs`.
- `test` — opens as impl: the census has no `tests/` lane; it writes the fixture and the tests in `worker_dispatch/tests.rs` from the capture.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: a shot of a director tile during a `follow` dispatch to a worker whose app-server an earlier dispatch started shows only the worker row with its job. The checkpoint adds one Change-Review line for C86 and C87.

### Phase 43 — Ages read d:hh:mm, right-aligned  · status: done

#### As-built

Every session age in the summary and in every tile reads `d:hh:mm` with only the fields that exist. The values and the `age` header align to the right edge of the column.

- `summary::age::age_label(seconds: u64) -> String` writes whole minutes: `0` under one minute, `10`, `1:10`, `10:10`, `1:02:10`, `110:00:00`. The leading field has no zero pad. Each later field has two digits. There are no unit letters and no seconds.
- `AgeLayout { width }` keeps one measured width per drawn group: the widest label over every row of the group, offscreen rows included. Its API is `from_ages`, `const fn width` and `format`, which right-pads a label to the width. The summary builds one layout over all listed ages (`age_layout`). A tile group uses its local layout or the shared `TreeSizing::Shared` metrics.
- `SummaryColumn::Age` and the tile `TreeColumn::Age` use `Alignment::Right` for the header and the values.
- `AgeParts`, `AgeSlot`, `AGE_UNITS` and `AGE_DETAIL_BELOW` do not exist. The accounts table `read` column (`read_age`) keeps its own form.

**Files:**
- `crates/cargo-handler/src/summary/age.rs` — `age_label`, `AgeLayout`, and the label test `ages_read_in_their_largest_unit`
- `crates/cargo-handler/src/summary/mod.rs` — the summary age column and `age_layout`
- `crates/cargo-handler/src/agent_cell.rs` — tile age cells (`cell_text`) and the `AgeLayout` of each group
- `crates/cargo-handler/src/summary/age_layout_tests.rs` — the right edge of values and header in the summary and in a tile
- `crates/cargo-handler/CHANGELOG.md` — one Changed line for the new age form

**Binds later work:** the redraw deadline work ("Tile hosts draw only frames that change", "cargo-handler idles under 1 % of one core, second cut") treats age text as a value that changes once a minute for each session. "Narrow tiles keep tree guides, readable names and one header inset" owns the rule that a column empty on most rows gives its width to session names. "Agent tiles show model and effort columns" follows the same rule.

**Gotchas:**
- Each age changes at the elapsed-minute boundary of its own session, not at one shared wall-clock minute.
- In a narrow tile, the narrower age frees width. That width can admit a type column that is empty on most rows, and session names then lose letters.
- Age alignment tests compare exact labels at the right edge of the `age` header. There are no per-unit slots to compare.

**Ruled out:** unit letters and a seconds field in the age (user ruling 2026-10-29).

### Phase 46 — A sleep shell shows the role `timer`  · status: done

#### As-built

- A shell row whose census observation is a countdown (`ShellTimerObservation::ObservedCountdown`) shows `timer` in the role column. Every other shell keeps an empty role cell.
- `ShellPurpose { Timer, Command }` is built `From<&ShellTimerObservation>`: `ObservedCountdown` is `Timer`, `Untimed` is `Command`. `TreeLine::role_text` returns `TIMER_ROLE` for `Timer` and `""` for `Command`.
- The role never comes from the row's display text. A tool call's description replaces the command as the row name, so the name cannot decide. A described countdown shell (name `Wait for completion`) shows `timer`. An untimed shell named `sleep 300` shows no role.
- `sleep infinity` has no countdown, so it shows no role.
- Sort and column width use the same role text, so `timer` sorts and fits like any role.
- `ChildKind` is unchanged.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `ShellPurpose` and the shell role text in `TreeLine::role_text`
- `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs` — role tests for observed countdown, described countdown, untimed `cargo`, and untimed `sleep 300`
- `crates/cargo-handler/src/constants.rs` — `TIMER_ROLE`, next to `SHELL_ROLE`

**Binds later work:** fixture updates for tile-tree tests keep the `timer` role and the countdown text on countdown shell rows.

**Gotchas:** the shell purpose comes from the census observation only. Reading it from the display description or the command text gives wrong roles.

**Ruled out:** a `ChildKind` variant for timers (it touches about 20 sites in `census/mod.rs`).

### Phase 47 — cargo-handler idles under 1 % of one core, second cut  · status: done

#### As-built

- The census watches its inputs (transcript subagent directories, worker seat directories, run records and the process set) and refreshes only the process rows that changed (new pid or start time, exec, directory change). `DirectoryChange::{Unchanged, Reconcile}` (`census/transcript.rs`) decides per census poll if a rescan runs. Parsed worker data is reused until its input changes.
- When a watch fails or loses events, the census falls back to a periodic rescan: `INPUT_WATCH_RECOVERY` 30 s, `INPUT_WATCH_RETRY` 2 s, `PROCESS_RECOVERY_INTERVAL` 30 s, `RUN_WATCH_RECOVERY` 30 s, `RUN_WATCH_RETRY` 2 s (`constants.rs`). A recovery scan shows the same rows as a full rescan.
- On macOS, the process revision comes from libproc 0.14.11 (`BSDInfo`, `VNodePathInfo` through `MacProcessDirectories`), a macOS-only dependency.
- Body text redraws only at its next change: `DisplayText::AgentAge::next_change` per session start, managed worker ages, shell countdowns and account reading age.
- Measured idle own CPU, branch against the previously installed binary in one paired window: Linux 1.30 against 2.71 CPU-s per minute; Mac 0.26 against 0.91 (Apple Silicon `proc_pid_rusage` ticks converted with `mach_timebase_info`). The under-1 % target (0.60 CPU-s per minute) is not met.

**Files:**
- `crates/cargo-handler/src/census/scan.rs` — process discovery, process revisions, recovery
- `crates/cargo-handler/src/census/transcript.rs` — subagent directory watches, `DirectoryChange`
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — worker seat directory watches
- `crates/cargo-handler/src/census/activity.rs` — run record watches, `retain_sessions`
- `crates/cargo-handler/src/terminal.rs` — display-time state and the next body change
- `crates/cargo-handler/src/constants.rs` — census recovery timings
- `crates/cargo-handler/Cargo.toml`, `Cargo.lock` — `notify`, and libproc on macOS

**Binds later work:** The under-1 % target is carried by the phase titled "Tile hosts draw only frames that change". Departed sessions' directory watches in `SubagentReads` and `WorkerNames` are not dropped yet (a known defect); the phase titled "The census drops the directory watches of sessions that left" carries it. Later census work keeps the recovery test that runs a real `exec -a claude` child (`scan.rs`) green.

**Gotchas:**
- On Apple Silicon, `proc_pid_rusage` CPU fields are 24 MHz mach ticks, not nanoseconds; convert with `mach_timebase_info`. A Mac measure also records the agent and terminal counts the census shows.
- Tests that poll tile motion settle the motion before body assertions.

**Ruled out:** sysinfo for the Mac process revision — sysinfo 0.39.6 enumerates all pids and keeps a stale name after a same-pid exec.

### Phase 48 — The focus tint fills the whole tile  · status: done

#### As-built

- On an opaque screen, a focused tile paints its focus tint on every interior cell, content rows and empty rows alike, and on its foot readout row. An unfocused tile paints the screen ground.
- On a transparent screen, a focused tile paints no ground (`Color::Reset`). Focus shows only as the lit border.
- `tint_painted()` is the policy owner. One resolved ground, from `screen_ground()` and `pane_background(true)`, feeds the interior fill, the content provider and `CellReadout::for_content`.
- The coverage test helper permits a blank inactive foot only when the readout cannot fit.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — the ground resolution, the interior tint fill, and the tests `focused_tile_tint_covers_short_rows_empty_rows_and_readout` and `an_erased_inactive_readout_that_fits_does_not_cover_its_row`
- `crates/tui_pane/CHANGELOG.md` — one line under Changed

**Binds later work:** the fill, the content and the readout fading all get their ground from `tint_painted()`; a new ground decision goes there too.

**Gotchas:**
- On an opaque screen, border cells keep the default ground, so a dark band shows between the frame line and the tint.
- An unfocused tile's selected row (#2e2e2e) is almost the same colour as the focus tint (#2e2e39).
- A capture that reads `~/.config` is transparent, because the user config sets `transparent = true`. An opaque shot needs a scratch config with `transparent = false`.
- The opaque path fills the interior a second time after `draw_clipped` fills the pane. Its cost is not measured.

### Phase 47a — The census drops the directory watches of sessions that left  · status: todo

**Depends on:** Phase 47. Runs beside Phase 57 (no shared files).

#### Work Order

**Goal:** a long-running cargo-handler holds directory watches only for sessions the census still shows, so watches and their file descriptors do not grow as sessions come and go.

**Source:** Phase 47 architect review, 2026-10-31, confirmed in the code: a Phase 47 defect. `RunRecords::retain_sessions` (`crates/cargo-handler/src/census/activity.rs` ≈961) drops the sessions and watches the census no longer sees, but `SubagentReads::directories` (`crates/cargo-handler/src/census/transcript.rs` ≈936, values `SubagentDirectory { watch, rows }` ≈804) and `WorkerNames::directories` (`crates/cargo-handler/src/census/worker_dispatch/mod.rs` ≈437, values `SeatDirectory { watch, documents }`) have no path that drops an entry.

**Constraints from prior phases:**
- Phase 47: `DirectoryChange::{Unchanged, Reconcile}` (`transcript.rs`) decides only whether to rescan; the census recovery constants live in `crates/cargo-handler/src/constants.rs` ("census recovery"). Keep both.
- `RunRecords::retain_sessions` is the pattern: the census passes the set of sessions it saw this poll, and each cache keeps only those.
- No duplicated state: one retain step per cache, called from the same place the census calls `retain_sessions`.

**Spec:**
1. `SubagentReads` and `WorkerNames` each drop every directory entry (its watch and its cached rows or documents) whose session the census did not see this poll, in the same step as `RunRecords::retain_sessions`. `SubagentReads` also drops its `reads` and `metadata` entries under those directories.
2. Red-first counted tests, each under 1 s: after a session leaves, its directory watch and cached entries are gone from both caches; a session that stays keeps its watch and is not read again; a session that comes back is watched again and shows the same rows as a first read.

**Files:**
- `crates/cargo-handler/src/census/transcript.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs` — the retain steps and their tests
- `crates/cargo-handler/src/census/scan.rs` — the calls beside `retain_sessions` (≈619)
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer (`test` seat, which wrote Phase 47).

**Acceptance gate:** `verify.sh test cargo-handler` and `verify.sh lint cargo-handler` clean; no test takes a second. No visible change: the checkpoint carries `Change-Review: none`.

### Phase 54 — Tile hosts draw only frames that change  · status: todo

**Depends on:** Phase 48, Phase 47

#### Work Order

**Goal:** an idle cargo-handler and an idle cargo-tile draw a frame only when something visible changes, so each idles under 1 % of one core (0.60 own CPU-s per minute at most).

**Source:** showrunner ruling, 2026-10-30, on Phase 47's next cut: the tui_pane grid gives one "visual state changed" answer (motion, column bands, titles), a host skips equal frames, and cargo-tile uses it in the same phase so both hosts gain. Unit session `phases/47/impl_summary_test.txt`.

**Constraints from prior phases:**
- Phase 47 release profile, cargo-handler idle, per minute: 1043 body frames cost 1.28 own CPU-s; `TileGrid::tick` (`tiles/grid.rs` ≈2160) asked for 936 of the repaints, one on every tick while `GridMotion` is not `Settled`.
- The public placements give cell content and `PaneFrame` only; column bands, title ownership and motion heights live in `TileDrawing` and `tiles/draw.rs`, so a host cannot prove from placements alone that a frame is unchanged.
- Hosts: cargo-handler `terminal.rs` `Ticker::poll_at` ≈383 (`app.tiles.tick()`, Phase 47's display-time state); cargo-tile `terminal.rs` ≈227 and `render.rs` ≈8682.
- cargo-handler `app.rs`: `PaintedFrame` (≈116) is the retained body frame, and `App::draw` (≈364) reuses it for a status-only draw. The grid's painted record and this body cache must not both own "what was last painted": one owns it, and the other asks it.
- Phase 47 shipped each session's age deadline: `DisplayText::AgentAge::next_change` (`terminal.rs` ≈123) schedules each age from its own start, and `next_body_change` (≈192) includes managed workers (tests ≈503 and ≈574). Keep that one calculation.
- Phase 47 Mac measure defect: `proc_pid_rusage` CPU fields on Apple Silicon are 24 MHz mach ticks, not nanoseconds (timebase 125/3, proved 2026-10-31). Phase 47's corrected Mac own CPU-s per minute: branch 0.26, installed 0.91. Linux: branch 1.30, installed 2.71.
- `tiles/draw.rs` is Phase 48's until its checkpoint; cargo-tile's files may be held by tile-unit: claim through berth (the showrunner tells tile-unit).
- `draw_tile_grid` (`tiles/draw.rs` ≈384) prepares the measurements before it paints, and records the painted geometry after it paints (`record_painted_cells`, `record_painted_summary_frames`). Phase 9 reads that last painted record for clicks; keep that use.
- `age_label` (`crates/cargo-handler/src/summary/age.rs` ≈4) writes elapsed whole minutes from each session's own start. So each session's age text changes at its own elapsed-minute boundary, not at one shared wall-clock minute. A label that gains a digit or a `:` part also widens the shared age column (`AgeLayout`, same file). Neither change comes with a census revision.

**Spec:**
1. One owner in `tiles/` gives the visual state of the grid (motion step, column bands, titles, motion heights, and the appearance inputs `tile_background` reads: theme, transparency and tint settings) and answers whether it changed since the last drawn frame. No host compares parts of it by hand. Its last-painted record is the one Phase 9 reads for input.
2. `TileGrid::tick` asks for a repaint only when that answer is yes.
3. cargo-handler and cargo-tile each skip a frame whose visual state and content are equal to the last drawn one.
4. The comparison state changes only after a body frame is painted. Hold it in an explicit enum, for example `LastPainted::{NeverPainted, Painted(..)}`. A poll that draws no body, and a status-only draw, leave it unchanged.
5. Keep Phase 47's per-session age deadline (`DisplayText::AgentAge::next_change`) as the one calculation. Add the next width growth of the shared age column (`AgeLayout`) to it, so that draw needs no census revision.
6. Red-first counted tests in tui_pane and in each host: an unchanged settled grid asks for no frame; each motion step, band change and title change asks for one; several polls with no body draw keep the comparison state; status-only draws keep it; fading content with settled geometry still draws each fade step; motion completes after skipped equal frames; age deadlines for sessions with offset starts each draw at their own minute; an age width growth draws with no census revision; a theme, transparency or tint change with equal geometry draws. Each under 1 s.
7. Live measurement in both hosts, on Linux and on the Mac, before and after, with the Phase 40 paired method: own CPU under 1 % of one core, and no worse than the installed binary. Each run shows live agents, and records the agent and terminal counts the census showed, the window length and the load. On the Mac, convert `proc_pid_rusage` ticks with `mach_timebase_info`.
8. CHANGELOG: one Changed line in tui_pane, cargo-handler and cargo-tile.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs` — the visual state owner, the last painted state and their tests
- `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-handler/src/terminal.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/CHANGELOG.md` — the host frame skip; `app.rs` holds `PaintedFrame`
- `crates/cargo-handler/src/summary/age.rs` — the age column's next width growth
- `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/CHANGELOG.md`

**Workers:** 1 writer (`test` seat, which wrote Phase 47).

**Acceptance gate:** `verify.sh test tui_pane`, `verify.sh test cargo-handler`, `verify.sh test cargo-tile`, and `verify.sh lint` for tui_pane, cargo-handler and cargo-tile clean; no test takes a second. Final CPU gate (moved from Phase 47): own CPU under 0.60 CPU-s per minute in the Phase 40 paired method, for cargo-handler and cargo-tile, each on Linux and on the Mac, with live agents. The checkpoint adds a Change-Review line.

### Phase 58 — The focus tint reaches the frame and stands apart from selection  · status: todo

**Depends on:** Phase 48, Phase 54; runs before Phase 25 and Phase 50 (shared `render/selected_ink_tests.rs`)

#### Work Order

**Goal:** on an opaque screen a focused tile is one tinted surface up to and under its frame line, and a selected row in an unfocused tile never reads as the focus tint.

**Source:** Phase 48 design check, 2026-10-30, on `phases/48/live/final_opaque2/focused_summary.png` (built from 531e8ca4), rows routed from Phase 48: (1) a black band shows between the tint and the summary frame on all four sides, because the border cells keep the default ground; (2) the selected row of an unfocused tile (#2e2e2e) is almost the same colour as the focus tint (#2e2e39); (3) the opaque shot had no empty row below the content, so the shot set did not show the empty-row fill. Phase 48 merge-repair design check, 2026-10-31, on `phases/48/live/b1f383c7/handler_opaque_first_summary_frame/with_overlay.png` (built from b1f383c7): (4) on an opaque screen the settings overlay interior paints no ground (2703 default cells), so the tiles show through it; the overlay must paint the opaque pane ground (showrunner, 2026-10-31).

**Constraints from prior phases:**
- Phase 48: `tint_painted()` (`crates/tui_pane/src/pane/chrome.rs` ≈135) is the one owner of whether a ground is painted; `pane_fill` (≈129) and `pane_background` (≈157) take their answer from it. On a transparent screen a focused tile paints no ground and shows focus only as the lit border (showrunner ruling 2026-10-29); keep that.
- `pane_tint(focused)` (`chrome.rs` ≈175) carries a focused pane further along the same lift than an unfocused one.
- Phase 54 changes `tiles/draw.rs` first (the visual state owner); this phase runs after it on the same file.
- `GridLines::render` (`crates/tui_pane/src/pane/frame.rs` ≈659) paints the final border styles; `mark` already combines focus ownership for shared edge cells.
- `pane_fill` (`chrome.rs` ≈129) returns `Option<Style>` and `screen_ground` returns `Option<Color>`; absence means a transparent screen or a disabled tint.
- `chrome.rs` ≈151 already has `TileGround::{Unpainted { blend }, Painted(Color)}`, used through `PaneChrome::paint_tile_ground` by tile drawing and the handler summary. `Unpainted { blend }` carries the colour fading ink blends toward on a transparent screen. It is the existing paint-state owner.
- `PopupFrame::render_with_areas` (`crates/tui_pane/src/pane/popup.rs` ≈43) clears its area and draws a block with no background, so on an opaque screen an overlay interior shows default cells.
- cargo-handler's `paint_selection` (`crates/cargo-handler/src/render/mod.rs` ≈487) changes the row ground and adjusts ink; it draws no selection marker. `render/selected_ink_tests.rs` ≈207 requires the remembered ground and ≈325 requires contrast between it and the tile.
- The shared selection style also reaches cargo-port through `PaneSelectionState::overlay_style`; it stays unchanged, so the tile grid applies the rule where it draws a tile.

**Spec:**
1. Opaque screen: a focused tile paints its tint on its border cells too, so the frame line stands on the tint with no default-ground band beside it. A shared edge takes the tint when `mark` gives it to the focused tile. Unfocused tiles and the transparent screen do not change. `tint_painted()` stays the one owner.
1a. Extend the existing `TileGround` (`chrome.rs` ≈151) into the one chrome-owned paint result; it replaces `pane_fill`'s `Option<Style>` and `screen_ground`'s `Option<Color>`. Keep its no-paint state with the fade destination (`Unpainted { blend }`). The fill, the border, the readout and the popup interior all take it. No second paint-state type.
1b. On an opaque screen, `PopupFrame::render_with_areas` paints the opaque pane ground on the overlay interior and border, so no tile shows through an open overlay (Source item 4). A transparent screen keeps today's overlay.
2. Showrunner rulings, 2026-10-30: in the tile grid, an unfocused tile shows no selection; its remembered row shows again when the tile takes focus, so the tint alone means focus. cargo-handler draws no selection marker (`paint_selection` ≈487), so no ground means no visible selection there. The rule is the tile grid's only: the shared selection style for all panes does not change, and cargo-port's unfocused panes stay as they are. Today that row draws #2e2e2e in Default Dark against the focus tint #2e2e39 (`crates/tui_pane/src/pane/state.rs` `selection_state_for` ≈73).
3. Red-first buffer tests: a focused tile on an opaque screen has the tint on every border cell and every interior cell; a transparent screen paints no ground on any of them; a shared edge takes the tint in both paint orders, with moving clips and titles; a remembered row in an unfocused tile draws no selection and shows again when the tile takes focus; on an opaque screen an open settings overlay paints the pane ground on every interior cell, and on a transparent screen it paints none; the transparent readout fade and the first summary frame keep their current results; a cargo-port pane's unfocused selection does not change; a selected row in the focused tile does not change; `render/selected_ink_tests.rs` ≈207 and ≈325 change to the ruling with an Assertion-Change line. Each under 1 s.
4. CHANGELOG: one Changed line in tui_pane.

**Files:**
- `crates/tui_pane/src/pane/chrome.rs`, `crates/tui_pane/src/pane/state.rs`, `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/tiles/draw.rs` — the paint result, the border ground, the unfocused selection ground and their tests
- `crates/tui_pane/src/pane/popup.rs` — the opaque overlay ground and its test
- `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/render/selected_ink_tests.rs` — the handler selection paint and its tests
- `crates/tui_pane/CHANGELOG.md`

**Workers:** 1 writer (`impl`).

**Acceptance gate:** `verify.sh test tui_pane`, `verify.sh test cargo-handler` and `verify.sh lint tui_pane` clean; no test takes a second. Live: an opaque shot of cargo-handler with the summary focused, at least one empty row below its content and an unfocused tile with a selected row, an opaque shot with the settings overlay open, and transparent shots of the same states; all pass a fresh design check. The checkpoint adds a Change-Review line.

### Phase 57 — The mouse wheel scrolls the shortcuts overlay in every host  · status: todo

**Depends on:** port-unit's Phase 2e (merged; `wheel_framework_overlay` is on main 56ade11c). Showrunner, 2026-10-31: runs before Phase 54. Port-unit's Phase 2e "The shortcuts overlay's wheel scroll lives in tui_pane". Showrunner, 2026-10-30: it returns `OverlayWheel::{Consumed, NoOverlay}`, where `Consumed` covers any wheel step over an open overlay; port-unit, 2026-10-30: `pub fn wheel_framework_overlay<Ctx: SettingsHost>(ctx: &mut Ctx, position: Position, direction: ScrollDirection) -> OverlayWheel` and `pub enum OverlayWheel { Consumed, NoOverlay }`, both exported from the tui_pane crate root; `ScrollDirection` is `tui_pane::ScrollDirection`; `SettingsHost` (`app_settings/navigation.rs`) extends `AppContext`. `Consumed` is any wheel event while Settings, Keymap or GlobalShortcuts is open; the caller applies its own `invert_scroll` first. Port Phase 2e runs after its 2d and 2g; port-unit says if the signature changes. Background: cargo-port's `scroll_modal_overlay_at` (`crates/cargo-port/src/tui/input/dispatch.rs` ≈721) consumes a wheel event over an open overlay even outside its content or when it cannot move (for example `OverlayWheel::{Consumed, NoOverlay}`, with shared tests for those cases)

#### Work Order

**Goal:** the mouse wheel scrolls the `?` global shortcuts overlay, and the other framework overlays, in cargo-tile and cargo-handler, through the one shared tui_pane function that cargo-port also calls.

**Source:** the user (via the showrunner), 2026-10-30, on change review row C119 (Phase 18, merge 2a881055), returned: "it does split but what doesn't work is mouse scrolling on either one - it does work on cargo port - this needs to be shared code". Showrunner: put overlay wheel scrolling in tui_pane so every host gets it from one place, with a test per host; CPU work runs first. When this lands, port-unit moves cargo-port to the shared path and removes its own; the showrunner tells port-unit. The fix returns as a new change review row. Showrunner, 2026-10-30, after the user's no-duplication rule: port-unit moves cargo-port's overlay scroll into one public tui_pane function (`OverlayScroll::{Scrolled, NoOverlay}`) and cargo-port calls it, with no copy left; this phase only wires the hosts to it.

**Constraints from prior phases:**
- The runner gives every wheel step to `TerminalApp::scroll` (`crates/tui_pane/src/runner/event_loop.rs` ≈214, `handle_mouse`). The trait default does nothing (`crates/tui_pane/src/runner/app.rs` ≈56).
- cargo-handler's `scroll` (`crates/cargo-handler/src/app.rs` ≈390) calls `tui_pane::handle_tile_scroll` (`crates/tui_pane/src/tiles/host.rs` ≈164). That function scrolls only a `TilePick::Cell` hit; a `TilePick::OverlayRow` hit does nothing. The click path already handles `TilePick::OverlayRow` through `select_framework_overlay_row` (≈157).
- cargo-tile does not implement `scroll` (`crates/cargo-tile/src/app.rs`), so its wheel steps do nothing.
- cargo-port's working path is `scroll_modal_overlay_at` (`crates/cargo-port/src/tui/input/dispatch.rs` ≈705). For the framework overlays it does: `Settings` moves the selection (`NavAction::Up`/`Down`) when the pointer is inside the settings content area; `Keymap` and `GlobalShortcuts` call `viewport_mut().scroll_if_contains(pos, direction)`. It applies cargo-port's own `invert_scroll` before that call (≈651).
- The framework holds the open overlay (`Framework::overlay`, `crates/tui_pane/src/framework/mod.rs` ≈379) and the panes (`global_shortcuts_pane` ≈101, `keymap_pane`, `settings_pane`).
- Phase 56 also edits `tiles/host.rs` (`content_click`); this phase runs first.
- No duplicated state, logic or data: one owner for each fact. The overlay scroll has one owner, the tui_pane function from port-unit's phase. Do not copy its arms.

**Spec:**
1. `handle_tile_scroll` calls the shared tui_pane overlay scroll function first. When it returns `OverlayWheel::Consumed`, the step stops there, so a wheel step over an open overlay never scrolls a tile behind it.
2. cargo-tile implements `scroll` through `handle_tile_scroll`, as cargo-handler does.
3. Red-first tests, each under 1 s: in cargo-tile and in cargo-handler, with the global shortcuts overlay open and taller than its viewport, a wheel step down moves the overlay viewport and a step up moves it back; a step outside the overlay does not scroll a tile.

**Files:**
- `crates/tui_pane/src/tiles/host.rs` — `handle_tile_scroll` calls the shared overlay scroll first
- `crates/cargo-tile/src/app.rs` — `scroll` through `handle_tile_scroll`
- `crates/cargo-tile/src/` — one wheel test in a cargo-tile test module
- `crates/cargo-handler/src/` — one wheel test in a cargo-handler test module
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer.

**Acceptance gate:** `verify.sh test` and `verify.sh lint` clean for tui_pane, cargo-tile and cargo-handler; no test takes a second. Live: in cargo-tile and cargo-handler, press `?` and scroll the overlay with the wheel. The checkpoint adds a Change-Review line.

### Phase 55 — Narrow tiles keep tree guides, readable names and one header inset  · status: todo

**Depends on:** Phase 48

#### Work Order

**Goal:** at every tile width, each row keeps its tree guides, session names stay readable, and every tile header sits over the names.

**Source:** Phase 43 design check, 2026-10-30, on `phases/43/shots/ages_120x40.png` and `ages_200x50.png` (routed from Phase 43, items F002–F005). Showrunner, 2026-10-30, on main-binary reruns: one came from the age change. In the 120×40 natedev tile the narrower age frees room that admits the type column, empty on most rows, and session names lose one more letter than on main.

**Constraints from prior phases:**
- Phase 27 keeps the core columns on narrow tiles; `TreeColumns::for_lines` (`agent_cell.rs` ≈453) keeps a column only when a drawn row has a value. Phase 49b adds model and effort columns in the same file and runs after this phase.
- `tree_header` (`agent_cell.rs` ≈2459) removes the header inset under compression: when the session budget is below the session header width, it sets `SessionHeaderLead::Flush`.
- `tree_cell_line` (`agent_cell.rs` ≈2602) drops the whole guide prefix when fewer than two name cells remain after it.
- Both width paths drop Directory, Branch, Type, Age and Account in that order: local fitting through `ColumnDrop::ORDER` (`agent_cell.rs` ≈504) and shared-column fitting through `GroupFitStep::ORDER` (≈936). Age is in both orders today.
- cargo-handler gives the grid its minimum tile width through `Cells::measure_trees` (`crates/cargo-handler/src/render/mod.rs` ≈459, from `TreeMeasurement::minimum_tile_width`, `agent_cell.rs` ≈817) and `set_min_tile_width` (≈175). It also sets `TileWidthFloor::CompressSingleColumn` (≈177), which lets one column go below that minimum.

**Spec:**
1. At 120×40 the natedev tile drew two depth-3 rows ("ws-impl…", "Dispatch…") at the tile's left edge with no guides. Draw every row's guides at its own depth at every width; cut the name after the guides, never the guides.
2. The same tile cut session names to two letters ("re…" for both "report" and "resources") beside a type column empty in 17 of 19 rows. Give an empty column's space to the session column, as the hana tile does. A column that is empty on most rows never takes room from session names.
3. Its header "session ▲" started flush at the guides; the other tiles' headers sit 3 cells in, over the names. One header inset for every tile.
4. Minimum agent tile width (showrunner ruling, 2026-10-30). An agent tile's minimum width is its deepest drawn guide prefix plus a minimum readable name plus the age column. The grid never gives an agent tile less than that. Optional columns drop first. This phase gives the grid that minimum and tests it. Today the display fallback (`crates/tui_pane/src/tiles/grid.rs` ≈3252) shows the summary alone when admission fails; sending an undersized tile to overflow and counting it in `+n unseen` belongs to the handler overflow work (next items), with its width-admission contract and boundary test.
5. Red-first buffer tests for 1–3 at the widths that failed. Tests for 4 at the minimum-width boundary, in local fitting and in shared-column fitting: at the minimum, every row keeps its guides, a readable name and its age; one cell less does not give the tile that width. Each test under 1 s.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/` — tree guides, column space, header inset, the minimum width
- `crates/cargo-handler/src/render/mod.rs` — the minimum width that reaches the grid
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer (handler tree).

**Acceptance gate:** `verify.sh test cargo-handler` and `verify.sh lint cargo-handler` clean; no test takes a second. Live: shots at 200×50 and 120×40 pass a fresh design check. The checkpoint adds a Change-Review line.

### Phase 56 — The rows readout is off by default  · status: todo

**Depends on:** Phase 48, Phase 54, Phase 57

#### Work Order

**Goal:** the tile rows readout is hidden by default in cargo-tile and cargo-handler, and one key shows and hides it for the session.

**Source:** showrunner ruling, 2026-10-30, copied from the tile-fixes plan, Phase 32 Work Order. The ruling moved out of Phase 55 into this phase on 2026-10-30.

**Constraints from prior phases:**
- Showrunner ruling, 2026-10-30: handler-unit owns tui_pane `tiles/draw.rs` and `tiles/grid.rs` for this work. Phase 48 edits `tiles/draw.rs`, and Phase 54 edits `tiles/draw.rs` and `tiles/grid.rs`; this phase starts after both checkpoints.
- Phase 36: `RowsReadout` (`crates/tui_pane/src/tiles/draw.rs` ≈99) is `Count(usize)` or `Selected(SelectedRow)`, a selection checked by `RowsReadout::selected`. It keeps its name, its checked selection value and its visible text. Phase 36 ruled out a hidden readout in its own scope; the 2026-10-30 ruling replaces that here.
- The demand adds the readout row in `add_readout_rows` (`tiles/draw.rs` ≈711) through `rows_readout_height` (≈939). The draw is `draw_rows_readout` (≈767), called after the cell paints from `draw_cell_with_rows_readout` (≈677).
- `content_area` (`tiles/draw.rs` ≈928) gives the cell interior above the readout row. `tiles/grid.rs` calls `draw::content_area` in its allocation, motion and fit paths. `content_click` (`crates/tui_pane/src/tiles/host.rs` ≈176) maps a press through `content_area` too.
- The tile foot row also holds host foot content: tui_pane `SummaryFoot` (`tiles/draw.rs` ≈238). cargo-tile's `foot` (`crates/cargo-tile/src/render.rs` ≈283) gives `SummaryFoot::Text` with its memory total, drawn on the same foot row as the readout.
- The `+n unseen` count comes from `UnseenTiles` (`crates/tui_pane/src/tiles/flow_layout.rs` ≈151). It is private packer output and nothing draws it yet. The handler overflow work draws it.
- cargo-port draws through `tui_pane::render_panes` (`crates/cargo-port/src/tui/render.rs` ≈156). It has no tile grid and no rows readout, so the readout is already absent there. This phase does not touch cargo-port.

**Spec:**
1. The tui_pane tile rows readout (`content rows: N  r/c: R/C`, `draw_rows_readout`) is a layout aid. It is off by default in cargo-tile and cargo-handler.
2. The visibility is one enum, `RowsReadoutVisibility::{Hidden, Shown}`, default `Hidden`. The demand path (`add_readout_rows`, `rows_readout_height`) and the draw path (`draw_rows_readout`) read it. When it is `Hidden`, the demand adds no readout row. When it is `Shown`, the readout text does not change.
3. One key turns the readout on and off for the session, with no settings entry. tui_pane defines the action in its shared keymap and shows it in the keys bar. cargo-tile and cargo-handler bind it. Use a key that is free in both keymaps, and record the key in this Work Order.
4. One content allocation (showrunner ruling, 2026-10-30). `content_area` becomes visibility-aware. Drawing (`content_area`) and clicks (`content_click`) read that one allocation, so they cannot disagree. When the readout is hidden, a tile whose host has foot content keeps the foot row for that content. A tile with no foot content gives the row back to content.
5. Handoff to the handler overflow work: it draws `+n unseen` with no dependency on the readout state. A clipped piece's red shortfall count draws only while the readout is `Shown`. The overflow work adds the `+n unseen` test in both readout states. Tell the showrunner when this phase lands.
6. Red-first tests, each under 1 s: by default no tile draws or demands a readout row; the key shows the readout with unchanged text and hides it again; hiding the readout leaves the foot's other content drawn (the cargo-tile memory total); a tile with no foot content draws content on the reclaimed last row; a press on the reclaimed last row maps to the content row drawn there while the key toggles and during motion; a host with foot content keeps its foot row in both states.
7. CHANGELOG: one Changed line in tui_pane.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `RowsReadoutVisibility`, the demand, the draw, `content_area` and their tests
- `crates/tui_pane/src/tiles/grid.rs` — the visibility state and the callers of `draw::content_area`
- `crates/tui_pane/src/tiles/host.rs` — `content_click` reads the one allocation
- `crates/tui_pane/src/keymap/` — the shared action and its keys bar entry
- `crates/cargo-tile/src/keymap.rs`, `crates/cargo-handler/src/keymap.rs` — the key's binding in each host
- `crates/tui_pane/CHANGELOG.md`

**Workers:** 1 writer (tui_pane and hosts).

**Acceptance gate:** `verify.sh test` and `verify.sh lint` clean for tui_pane, cargo-tile and cargo-handler; no test takes a second. Live: shots of cargo-handler and cargo-tile with the readout hidden and shown. The checkpoint adds a Change-Review line.

### Phase 49a — The census reads each agent's model and effort  · status: todo

**Depends on:** Phase 40, Phase 41, Phase 47

#### Work Order

**Goal:** every Claude and Codex agent row, and each subagent row whose record names one, carries its model's short name and its reasoning effort as typed values, read once when the row appears or its record changes.

**Source:** user, 2026-10-29: "handler - in the cells needs a column for the short model name - like opus, astra, sol - and the effort also". Showrunner approved, with a CPU condition: read model and effort once, when an agent appears or its record changes, never per frame or per poll; state the added cost at the checkpoint. Unit session `phases/49/source.md`. Phase 49b draws the columns.

**Constraints from prior phases:**
- `AgentRow` (`census/mod.rs` ≈312) holds `agent`, `service_tier`, `account`, `name`, `activity`, `started`; rows also cross the probe wire from other machines ("missing from a probe printed before it existed"), so a new field needs a wire default for an older probe.
- No model or effort reads exist yet. Candidate sources: the Claude transcript's assistant records (`message.model`, `census/transcript.rs`) and the session's effort setting; the Codex rollout's `turn_context` (`model`, `effort`) and the Codex thread records `census/codex.rs` reads (Phase 41 changed its thread queries: `ThreadsLayout::{WithThreadSource, OriginatorOnly}`); a worker dispatch line such as `codex/gpt-6.1-sol:high` (`census/worker_dispatch`).
- Read model and effort by extending the existing transcript and rollout readers: Codex conversation data flows through `Rollouts` (`census/tier.rs` ≈401) and its calls in `census/scan.rs` (≈406, ≈486, ≈628); keep their revision and append checks. (`RunRecords::read_for_processes`, `census/activity.rs` ≈658, reads verification records and is not the path.)
- `Rollouts::thread` returns `Option<&RolloutRead>` (`census/tier.rs` ≈425). Give it a named result such as `ThreadRollout::{NotRetained, Retained(..)}`, and keep a retained record with unrecorded model or effort distinct from a missing rollout; add no domain-owned `Option` fields.
- Rows are built in `census/tree.rs` (subagents ≈657, flattened children ≈740) and `census/classify.rs`.
- Phase 47 owns `census/{activity.rs,scan.rs,hierarchy.rs,command_inputs.rs,activity_regression_tests.rs}` and `app.rs`; claim a shared file through berth, or keep the read in `transcript.rs`, `codex.rs` and a new `census/model.rs`.

**Spec:**
1. One typed model name: `ModelName` with a short form from one table (`opus`, `sonnet`, `haiku`, `fable`, and the Codex names such as `astra`, `sol`), and `ModelName::Unrecorded` where no record names one. An unknown full id keeps a short form cut by one rule (the last word of the id with no version digits), never a placeholder string.
2. One typed effort: `ReasoningEffort::{Low, Medium, High, XHigh, Max, Unrecorded}` from the record's word.
3. Read each value from the agent's own record when the row first appears and again only when that record's revision changes. Subagents take theirs from their own record when it names one, else `Unrecorded`. Shells carry `Unrecorded`.
4. `AgentRow` and `ChildRow` (`census/mod.rs` ≈437) carry both. Subagents use `ChildRow`, and managed and unassigned Codex threads also use `ChildRow`. Each thread carries its own model and effort through its child row and the probe wire, never the values of the process that hosts it. The probe wire carries them with an `Unrecorded` default for an older probe.
5. Red-first tests, each under 1 s: an unchanged record is not read again, appended settings are read, a replaced record is read anew; a Claude transcript fixture gives `opus` + its effort; a Codex rollout or thread fixture gives `sol` + `high`; a thread fixture whose model and effort differ from its hosting process's values keeps its own values on its child row and across the probe wire; an unchanged record is not read again (a counted read); an older probe line parses with `Unrecorded`; the short-name table covers each listed name.
6. Measure the added cost on the release build in the Phase 40 way and state it at the checkpoint.

**Files:**
- `crates/cargo-handler/src/census/model.rs` — `ModelName`, `ReasoningEffort`, the short-name table and their tests
- `crates/cargo-handler/src/census/transcript.rs` — the Claude read
- `crates/cargo-handler/src/census/codex.rs` — the Codex read
- `crates/cargo-handler/src/census/mod.rs` — the row fields
- `crates/cargo-handler/src/census/probe.rs` — the wire fields and their default
- `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/census/tier.rs` — `Rollouts` and the `ThreadRollout` result
- `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/classify.rs` — carry the values to the rows (after Phase 47 hands off `scan.rs`)

**Workers:** 1 writer (a new role-named seat `model` when every open seat is busy).

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. The checkpoint states the measured added CPU cost. Change-Review: none (the columns arrive in Phase 49b).

### Phase 49b — Agent tiles show model and effort columns  · status: todo

**Depends on:** Phase 49a, Phase 43, Phase 55

#### Work Order

**Goal:** agent tile rows show the model's short name and the reasoning effort in their own columns.

**Source:** as Phase 49a. Unit session `phases/49/source.md`.

**Constraints from prior phases:**
- Two width paths: local fitting uses `ColumnDrop::ORDER` (`agent_cell.rs` ≈503); tiles that share a screen column use `GroupFitStep::ORDER` (≈935). Both drop effort before model; test both.
- A click on the `model` or `effort` header sorts by the drawn value through `TreeLine::compare` (≈1260), `Unrecorded` included.
- `TreeColumn` (`agent_cell.rs` ≈341) is `Session, Type, Role, Status, Account, Branch, Directory, Age`; `alignment` (≈357) and the header text (≈380) are per column. Phase 55 replaces the rule of `TreeColumns::for_lines` (≈453), which kept a column when some drawn row had a value: a column that is empty on most rows never takes room from session names. Phase 27 keeps the core columns on narrow tiles; Phase 43 changes the age text in the same file.
- Phase 49a gives each row `ModelName` and `ReasoningEffort`, both with an `Unrecorded` case.

**Spec:**
1. `TreeColumn::Model` (header `model`) and `TreeColumn::Effort` (header `effort`), after `Role`, left-aligned; `Unrecorded` draws nothing. Phase 55's rule applies to both columns: a column that is empty on most rows never takes room from session names.
2. On a narrow tile the two columns drop before `Branch` and `Directory` keep their place (effort first, then model).
3. Red-first tests, each under 1 s: a tile with a Claude and a Codex row shows `opus`/`sol` and their efforts; a tile of shells drops both columns; a narrow tile drops effort, then model, before the core columns. Sparse values: a tile where model and effort are recorded on few rows gives their space to session names, in local fitting and in shared-column fitting. Update fixture text the suite names.
4. CHANGELOG: one line under Added.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — the columns
- `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs` — tests
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer (`impl`).

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: a shot of a tile with Claude and Codex rows shows both columns. The checkpoint adds a Change-Review line.

### Phase 50 — Slow cargo-handler tests run under a second  · status: todo

**Depends on:** Phase 41, Phase 43, Phase 47, Phase 48

#### Work Order

**Goal:** every cargo-handler test that takes over one second runs under one second.

**Source:** Phase 40's full test run, 2026-10-29: three existing tests took 1.46 s (gradient), 1.27 s (busy ink) and 1.04 s (codex tier). Phase 43's full run, 2026-10-29: five more took 1.0–1.6 s (`render::selected_ink_tests::selected_row_inks_keep_glyph_contrast_for_sessions_roles_accounts_and_both_timer_states`, `…_in_default_light`, `…every_default_dark_roles_ink_keeps_coverage_and_hue_on_tinted_remembered_selection`, `…every_default_dark_sessions_ink_keeps_coverage_and_hue_on_tinted_remembered_selection`, `agent_cell::tests::design_ink_tests::tree_role_and_account_words_meet_contrast_on_every_tile_ground`); showrunner put them here, after 47, 48 and 43: cause found first, no rerun, no load blame. Phase 25's Codex slow test (showrunner, 2026-10-24: 1.510 s in the phase 30 full suite at high machine load) moved here on 2026-10-30. Standing rule: no test takes a second; a slow test is a defect to fix, never to rerun.

**Constraints from prior phases:**
- The nine tests, all under `crates/cargo-handler/src/`:
  - `reset_shades_keep_saturated_chroma_and_separation_from_plain_text` (`summary/accounts_gradient_tests.rs` ≈532)
  - `busy_status_words_share_one_drawn_ink_across_row_kinds_and_frame_changes` (`agent_cell/tests/design_ink_tests.rs` ≈99)
  - `tree_role_and_account_words_meet_contrast_on_every_tile_ground` (`agent_cell/tests/design_ink_tests.rs` ≈336)
  - `selected_row_inks_keep_glyph_contrast_for_sessions_roles_accounts_and_both_timer_states` (`render/selected_ink_tests.rs` ≈133)
  - `selected_row_inks_keep_glyph_contrast_in_default_light` (`render/selected_ink_tests.rs` ≈138)
  - `every_default_dark_roles_ink_keeps_coverage_and_hue_on_tinted_remembered_selection` (`render/selected_ink_tests.rs` ≈944)
  - `every_default_dark_sessions_ink_keeps_coverage_and_hue_on_tinted_remembered_selection` (`render/selected_ink_tests.rs` ≈984)
  - `codex_session_fast_tier_draws_on_summary_tree_and_unassigned_accounts` (`census/codex_session_tests.rs` ≈291)
  - `codex_resume_last_uses_the_newest_older_thread_in_its_home_and_directory` (`census/codex_session_tests.rs` ≈447, module `census::scan::codex_session_tests`)
- Find each by name in a `verify.sh test cargo-handler` run's timing output. Fix each at its cause (the work it repeats, a real sleep, a fixture built per case), never by weakening an assert.
- A test that runs under a second alone but over a second in the full suite is still a defect. There is no exemption for a test that is fast alone.

**Spec:**
1. Run the suite, name all nine tests and measure each alone and in the full suite.
2. Cut each at its cause; keep every assert.
3. Show each test's time before and after, alone and in the full suite.

**Files:**
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — the gradient test
- `crates/cargo-handler/src/agent_cell/tests/design_ink_tests.rs` — the busy ink and tree contrast tests
- `crates/cargo-handler/src/render/selected_ink_tests.rs` — the four selected-row ink tests
- `crates/cargo-handler/src/census/codex_session_tests.rs` — the Codex tier test

**Workers:** 2 writers.
- `palette` — the gradient, design-ink and selected-ink test files.
- `census` — `census/codex_session_tests.rs`, both Codex tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` clean with no test over one second; lint clean. Change-Review: none (test time only).

### Phase 51 — Design check follow-ups: the timer shot, a stale unassigned row and the account notice inset  · status: todo

**Depends on:** Phase 47

#### Work Order

**Goal:** the `timer` role is proved in a live shot with its row in view, the unassigned pane never draws more rows than it counts, and the summary's account notices start at the same inset as its other rows.

**Source:** showrunner design check of the Phase 46 and 41 shots, 2026-10-29 ≈16:25 EDT. Unit session `phases/51/source.md`; shots `phases/46/live/handler-timer.png`, `phases/41/live/handler_named.png`. Phase 48 design check, 2026-10-30, on `phases/48/live/final_opaque2/focused_summary.png`: the account notice rows ("claude2: mcpServers differ from ~/.claude; …") start at column 1, flush with the frame, while every other summary row starts at column 2 (routed from Phase 48).

**Constraints from prior phases:**
- Phase 46: the role comes from `ShellTimerObservation::ObservedCountdown`; the shot showed only the first rows of each pane, so the sleep shell was out of view.
- Phase 41's shot: the unassigned pane says "content rows: 2 of 2" and draws 3 rows; the third, `phase41-check`, has the same status, account, branch and age as `cargo-liner-handler-c6`, with only its session cell lit.
- `SurfaceSelection::rows_readout` (`crates/cargo-handler/src/interaction.rs` ≈361) counts selectable identities. An unassigned table also draws its header line, which the count does not include.
- Phase 56 hides the rows readout by default; its key shows it.
- The account notices and the failure note draw through `wrap_line(note(..), width, 0)` (`crates/cargo-handler/src/summary/accounts.rs` ≈898-906) with indent 0; the account rows draw with `SECTION_ITEM_INDENT`. The notice text comes from the accounts board (`~/.claude/scripts/accounts/accounts.py`), which this unit does not edit.

**Spec:**
1. Retake the live capture with a sleep shell's row in view showing `timer`; send the path.
2. Find out whether `phase41-check` is a real second session or a row left from an earlier frame. Stale: the pane clears every row it no longer draws. Real: the count includes it. Red test first for whichever it is, under 1 s.
3. The account notices and the failure note start at the same inset as the other summary rows, wrapped lines included. Red-first buffer test under 1 s.
4. CHANGELOG: one Fixed line when code changes.

**Files:**
- `crates/cargo-handler/src/summary/accounts.rs`, `crates/cargo-handler/src/summary/` — the notice inset and its buffer test
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/unassigned_tests.rs` — the unassigned pane rows and count (the writer names any other file in a blocked post)
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer.

**Acceptance gate:** `verify.sh test cargo-handler` and `verify.sh lint cargo-handler` clean; no test takes a second. Spec 2 holds only with a reproduction of the extra session identity or of stale cells across consecutive frames. Equality of the drawn line count and the readout count is not the rule. Live: both shots; captures taken after Phase 56 turn the readout on with its key. The checkpoint adds a Change-Review line when code changes.

### Phase 52 — No child stderr reaches the terminal  · status: todo

**Depends on:** Phase 47

#### Work Order

**Goal:** no command that cargo-handler starts can write to the terminal it draws on.

**Source:** user, 2026-10-29 (image 7): raw lines `~/.claudeN: mcpServers: - brp` (N = 1..4) appear at column 0 under the summary accounts table, over the tile border. The accounts script writes an MCP-server note to its stderr. The process and output path that put it on the terminal are not yet known.

**Constraints from prior phases:**
- `read_accounts` (`census/accounts.rs` ≈259) calls `hierarchy::run_output` (`census/hierarchy.rs` ≈512), which already sets null stdin, null stderr and piped stdout. `census/command_inputs.rs` caches commands and spawns none.

**Spec:**
1. Reproduce the stray text first. Identify the process that wrote it and the output path that took it to the terminal, and write both in the summary.
2. Fix that path at its cause.
3. Make one owner build every child command (stdin, stdout and stderr set there, stderr piped or null, never inherited) only if the diagnosis shows that this is needed.
4. Red-first test: the diagnosed writer leaves the drawn buffer and the terminal unchanged. Under 1 s.
5. CHANGELOG: one Fixed line.

**Files:**
- `crates/cargo-handler/src/census/command_inputs.rs`, `crates/cargo-handler/src/census/accounts.rs`, `crates/cargo-handler/src/census/schedule.rs`, `crates/cargo-handler/src/census/hierarchy.rs` — the diagnosed output path and, if needed, the child command owner (the writer names any other file in a blocked post)
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer.

**Acceptance gate:** `verify.sh test cargo-handler` and `verify.sh lint cargo-handler` clean; no test takes a second. Live: a shot of the summary with no stray text under the accounts table. The checkpoint adds a Change-Review line.

### Phase 53 — Reset times step plainly from soonest to latest  · status: todo

**Depends on:** Phase 52

#### Work Order

**Goal:** the color of an account's reset time tells at a glance which account resets soonest, in dark and light themes.

**Source:** change review C113 returned (user, 2026-10-29): "the colors are what you say they are but the color gradient is so subtle in the terminal that i don't get the affrodance value". C115 folds into C113 (user): one judgement. Showrunner ruling: plainly different ends and steps (3 or 4 distinct hues or lightness steps, soonest the strongest), each step readable on its ground in both themes, still no red; no reset color matches any session name color. Unit session `phases/53/source.md`.

**Constraints from prior phases:**
- C113 (merge 1bc31a60): reset times never red; dark ramp light green → sea green, light ramp violet → dark indigo. `reset_colors` (`summary/accounts.rs` ≈145).
- Red stays for a failed reading's dot.
- C115 kept the darkest violet off session name colors on light themes.
- `summary/accounts_gradient_tests.rs` is also in Phase 50's **Files** (its slow gradient test); they run one after the other, never together.
- `theme::ResetRank::color` (`crates/cargo-handler/src/theme/mod.rs` ≈128) is the one reset source. Today it blends the `reset_nearest` to the `reset_farthest` theme role at rank/(n-1) in plain sRGB (`tui_pane::blend_color`); the rank comes from `reset_colors` (`summary/accounts.rs` ≈145). The stepped colours replace that blend in this one owner. Do not add a second reset source. The readability correction (`footer_colors::readable_status_style`) applies after it.
- Measured on the C71 shot (Default Dark, 6 accounts): neighbouring reset colours step only dE 2.3–2.7, and the ramp ends are about dE 12 apart in every theme. `accounts_gradient_tests.rs` holds the C113 floors (`check_rendered_reset_ramp` ≈389), the ends floor (≈501) and the chroma and plain-text floors (≈532), and its formula tests (≈185, ≈232, ≈266) follow the sRGB blend. C115 is `SessionResetFloor` (`render/role_title_tests.rs` ≈715); dark is `DarkNotMeasured`.
- The reset colour work of Phase 35 (C71) moved here on 2026-10-30. Phase 31 runs after this phase and assesses these stepped colours.

**Spec:**
1. A stepped ramp of 3 or 4 distinct steps per theme, soonest the strongest, each step meeting text contrast on its ground; no red; no step equal to a session name color. It applies in all four themes.
2. Tests: step order by reset time, distinct steps, contrast on ground, no step equals a session color, no red. Measure each step on the drawn ink, after `readable_status_style`, with the pane tint on and off and on the selected row. Run each check in all four themes. The formula tests follow the stepped colours. Each under 1 s.
3. CHANGELOG: one Changed line.

**Files:**
- `crates/cargo-handler/src/theme/mod.rs` — `ResetRank::color`, the one reset source
- `crates/cargo-handler/themes/` — the reset roles of each built-in theme
- `crates/cargo-handler/src/summary/accounts.rs`, `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — the ramp and its tests
- `crates/cargo-handler/src/render/role_title_tests.rs` — `SessionResetFloor` for the stepped colours
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer.

**Acceptance gate:** `verify.sh test cargo-handler` and `verify.sh lint cargo-handler` clean; no test takes a second. Live: captures in a dark and a light theme. The checkpoint adds ONE Change-Review line covering C113 and C115: the ramp reads at a glance, no red, no reset color matches a session color, dark and light themes.

### Phase 44 — The accounts summary shows a session count per account  · status: todo

**Depends on:** Phase 43

#### Work Order

**Goal:** the accounts table in the summary has a `sessions` column that shows, for each account, how many live sessions run on it.

**Source:** the user, direct to the handler unit, 2026-10-28 ≈23:40 EDT: "need to add an enhancement to show the count of sessions for each account in the accounts summary table". Unit session `phases/44/source.md`.

**Constraints from prior phases:**
- `render/mod.rs` ≈595 can build more than one `SummaryLayout` in a frame, and ≈665 builds a new `SummaryFrame` per request: compute `AccountSessionCounts` once per frame with the frame's placement and pass it to every summary allocation. Count logical rows by the membership rules at `agent_cell.rs` ≈1428, ≈1935, not painted rows.
- Columns: `AccountsColumn` (`sort.rs` ≈172) with `ALL`, `index` and `header` (`"account", "role", "left", "resets", "credits", "enabled", "in use", "read"`). `summary/accounts.rs`: `compare_accounts` (≈99) sorts by each column, `values` (≈371) builds the cell text, `AccountsFit::for_width` (≈431) fits widths, the always-kept columns are Account, Roles and WeeklyLeft (≈468), and `AccountsColumnDrop::ORDER` (≈487) is InUse, Enabled, Read, Credits. Phase 28 pins the heading and column headers; phase 32 keeps main's table height.
- `AccountUsage` (`census/accounts.rs` ≈86) comes from `accounts.py`; Rust does not infer its roles or filter its rows. The count comes from the census, not from `accounts.py`.
- `SummaryFrame` (`summary/mod.rs` ≈213) holds `machines`, `placement` and `accounts`; `SummaryLayout::new` (≈438) calls `accounts::build_lines`. A row's account is `AgentRow.account` / `ChildRow.account` (`census/mod.rs` ≈323, ≈459), an `Account` whose label (`AccountHome::label`) is the label the tiles show; shells and subagents are `Account::Unrecorded`.
- Do not count through `TreeLine::account` (`agent_cell.rs` ≈1153). It gives the parent's account to unrecorded shell and subagent rows, for display only.
- Phase 43 changes age text in summary fixtures; this phase runs after it so the summary fixture text changes once each.

**Spec:**
1. A session is a logical member: an eligible session identity after placement, folding and hiding, on any shown machine. Claude sessions, Codex sessions and Codex threads are eligible; shells and subagents are not and do not count (unit director's call; a one-line revert if the showrunner counts only top-level agents). Count each identity once, by its own recorded `Account` home with that label. A folded or hidden identity does not count.
2. `AccountsColumn::Sessions`, header `sessions`, drawn after `role`; the value is the count, `0` for an account with none. Header click sorts by it like the other number columns. It drops for width after Credits (`AccountsColumnDrop` gains `Sessions` as the last entry).
3. The count is computed once per frame into a typed map (for example `AccountSessionCounts`) passed to `build_lines`; no census walk inside a cell draw.
4. Tests, red first, in `summary/accounts_tests.rs`: a census-only count change while the account reading is cached; one managed thread shown by more than one source row counts once; counts stay the same across scrolling and width changes; two accounts with 3 and 0 drawn sessions show `3` and `0`; a shell or subagent child adds nothing; a folded dispatch shell adds nothing; in `summary/accounts_header_sort_tests.rs` a click on `sessions` sorts by count; a narrow width drops `sessions` after `credits`. Update accounts fixtures whose text changes (`render/accounts_grid_tests.rs`, `render/pinned_accounts_tests.rs` and any other the suite names). Each test under one second.
5. CHANGELOG: one line under Added.

**Files:**
- `crates/cargo-handler/src/sort.rs` — `AccountsColumn::Sessions`
- `crates/cargo-handler/src/summary/accounts.rs` — values, sort, fit and drop order
- `crates/cargo-handler/src/summary/mod.rs` — the per-frame counts
- `crates/cargo-handler/src/summary/accounts_tests.rs`, `crates/cargo-handler/src/summary/accounts_header_sort_tests.rs`, `crates/cargo-handler/src/render/accounts_grid_tests.rs`, `crates/cargo-handler/src/render/pinned_accounts_tests.rs` — tests and fixture text
- `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/render/mod.rs` — one count snapshot per frame (impl slot)

**Workers:** 1 writer + 1 tester.
- `impl` — `sort.rs`, `summary/accounts.rs`, `summary/mod.rs`, fixture updates in `render/` and the CHANGELOG; hub: `crates/cargo-handler/src/summary/accounts.rs`.
- `test` — opens as impl: no `tests/` lane; it writes the tests in `summary/accounts_tests.rs` and `summary/accounts_header_sort_tests.rs` from this Spec.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: a shot of the accounts table on natedev shows each account's session count, and the count matches the drawn rows for one account. The checkpoint adds a Change-Review line.
