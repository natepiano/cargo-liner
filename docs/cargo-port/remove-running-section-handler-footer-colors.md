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
  - `crates/cargo-handler/src/theme.rs` — `Role` ≈58 (theme roles; footer colors are not theme roles).
  - `crates/cargo-handler/src/app.rs`, `render.rs` — where per-frame state reaches `agent_cell::draw` and `summary::draw`.
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler`
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

**Depends on:** Phase 5, Phase 12, Phase 15, Phase 16, Phase 17, Phase 18, Phase 19, Phase 20, Phase 21, Phase 23

**Blocked by:** tile-fixes phase 24 merged on main (tui_pane grid overlap) — the showrunner says when (showrunner, 2026-10-21).

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2894`) while drawing interpolates (`drawing_at`, `grid.rs` ≈3019), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps. Since phase 6, `summary::machine_height` counts hierarchy notice lines, so a first-machine notice appearing or clearing moves later machines' clickable headers; add a two-machine draw → notice change → click regression.

**Constraints from prior phases:**
- Phase 23: `shared_tree_metrics` builds one `AgeLayout` and one populated column set per screen-column group from the complete rows of every tile in the group. During motion, keep the group layout drawn before a census change until the new frame measures it.
- Phase 4's narrow headings fit through `HeadingLocation::{Shown,Cut,Omitted}` (`agent_cell.rs`); test clicks against the fitted heading actually drawn, not the full one.
- After phase 5 the summary's demand changes with each accounts reading.
- Phase 2's cell-local click hook (`cell_content_at`, `tiles/host.rs`) resolves content from current slots and the destination layout through a crate-private settled-geometry helper in `tiles/grid.rs`. This phase moves it to the last-drawn record too, so the header hit-test matches what is on screen.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Phase 3's tile identity is `tiles::TileKey::{Agent(AgentCell), Unassigned { machine }}` (sorts in `App::tree_sorts: HashMap<TileKey, TreeSort>`, pruned by `agent_cell::tile_keys`), and header layout carries `agent_cell::SessionHeaderLead::{OverBranch, Flush}` (the pool's session header sits over flush names). Drawn header spans are kept per `TileKey`, with the lead they were drawn with (`render.rs` ≈243 `Cells::tile`, ≈248 `header_spans`, `agent_cell.rs` ≈1605).
- Phase 12's accounts-header click: `render::cell_clicked` (`render.rs` ≈156–167) tries the summary's agent headers first. Then it rebuilds `summary::accounts_header_spans(area, &cells.summary_frame())` (`summary/mod.rs` ≈231) from the current accounts reading, clock and `App.accounts_sort`, and calls `AccountsSort::select` (`sort.rs`). `SummaryFrame.accounts_sort` carries the sort into `summary::width` and `summary::draw`, and `accounts::header_spans` reads the lines that `accounts::draw_sorted` draws. Keep these drawn accounts spans with the frame, as for the agent headers.
- Phases 16 and 20 add row and word clicks that can also hit changed content between draw and click: the frame record covers drawn row identities, clipped scrolled rows, wheel targets and word cells, retired tiles included.
- Phase 17: drawing prepares `TreeMetrics` across all tiles in one screen column and saves them in `App.tree_layouts` (`render.rs` ≈137), and each tile keeps its own age layout (`summary/age.rs` `AgeLayout`). A click must map through the metrics that were drawn, not ones rebuilt from current rows and time. Add a regression: another tile in the same screen column changes the shared widths between draw and click, and the click still selects the header drawn under the pointer, with the tile's own age layout kept.

**Spec:**
1. `draw_tile_grid` (`tiles/draw.rs`) records, for each drawn piece, the cell's stable content identity, its drawn rect, and its visible (clipped) hit region, in the grid's own state, replacing the previous frame's record. Where pieces overlap, the one drawn on top wins. Model the record's state as a type that tells "never drawn" apart from "a frame drew no interactive content" (e.g. `enum DrawnHits { NotDrawn, Drawn(Vec<DrawnPiece>) }`), never a bare `Option`.
2. `TileGrid::cell_at(&self, Position) -> Option<usize>` keeps its public signature, as the compatibility boundary, and answers from that record. Before the first draw it answers from the layout as today.
3. Phase 2's content lookup in `tiles/host.rs` uses the same record, and resolves the content by the identity recorded at draw time, not by the slot's current occupant.
4. cargo-handler keeps the header spans it drew with each tile and the summary, and `render::cell_clicked` maps a header click through those drawn spans, not freshly rebuilt ones. A click on a drawn tile that a later scan removed (its sort already pruned) records no preference, so it cannot bring a pruned pool sort back. `tiles/host.rs` ≈138 calls `focus_cell(index)` on a current slot: resolve the drawn identity to its current slot at the public `Option<usize>` boundary, with a retired-content outcome inside, so a click on a vanished drawn tile never focuses its replacement. If `Cells::tile` (`render.rs` ≈243) stays, it returns a semantic lookup result rather than an `Option`, converting to `Option` only at the framework boundary.
5. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell; a press on the destination rect of a still-moving cell returns what is drawn there; a press on a piece's clipped-away part misses it; a press after a frame that drew nothing interactive returns no cell. In cargo-handler, a census change between the draw and the click that would otherwise shift a header column still selects the column that was drawn under the pointer, for a tree tile and the unassigned tile, in Full and Compressed views; a click on a drawn tile that has since disappeared records no sort and focuses no replacement; an accounts reading that changes the summary's demand between draw and click still selects the drawn agents column; an accounts-header click during motion, or after an accounts reading changed the table between draw and click, sorts by the accounts column that was drawn under the pointer. cargo-tile's suites pass.
6. Acceptance tests also cover row, wheel and word targets, including clipped scrolled rows and retired tiles.
7. Phase 23 drops a column that is empty in every row of a tile, and `render::cell_clicked` today rebuilds header spans from current rows and restores only saved widths. Test, red first: a tile draws a `type` or `role` column, then its rows change so that column is empty, and a click on the drawn header still selects the column drawn in that frame.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/tests/`
- `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 2 writers.
- `grid` — tui_pane: the drawn-hit record in `draw.rs`/`grid.rs`/`host.rs`, its motion tests and `host_tests.rs`, tui_pane CHANGELOG.
- `app` — cargo-handler: drawn header spans kept with the frame, `render::cell_clicked`, their tests, cargo-handler CHANGELOG.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-tile`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` clean. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.

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
- The summary's top-level table reads `pid account session status age desktop directory` and lists only sessions with a hierarchy; one placement decision feeds the summary and the unassigned tile, so no session is in both. The heading count counts listed sessions.
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

### Phase 24 — Role column fits its drawn values; a spent timer reads running  · status: todo

**Depends on:** Phase 23

#### Work Order

**Goal:** a tile's role column is as wide as its widest drawn role, and a shell whose timer has run out while the shell still runs reads `running`, not `0:00`.

**Source:** the phase 23 design check 3, 2026-10-22. Both defects moved here under the hard landing: neither is a regression phase 23 caused.

**Constraints from prior phases:**
- Phase 23: each tile drops columns empty in all its rows (`TreeColumns::for_lines`, `agent_cell.rs`); tiles that share a `(screen-column x, VisibleTreeColumns)` group share one `TreeMetrics`, age slots included (`shared_tree_metrics`, measured once from the complete rows of every tile in the group); `fill_tree_width` gives the slack to `directory`, else `session`. Keep one fitter and one slack rule.
- Phase 23: the shell timer draws digits only (`timer_label` → `time_left_label`); `ShellState::{Running, Timed(Timer)}` gives the status text and ink; `running` and `thinking` draw in one Busy ink.
- Phase 17: ages are measured from complete row sets, never a visible slice.

**Spec:**
1. Role width: in the phase 23 shots, natedev and hana (left screen column, same column set) draw the role column 10 cells wide while their widest drawn role is `review` (6 cells), so the gap from role to status is 6 cells where every other pair has 2 (dark role x 459, status x 555). 10 cells is the width of `showrunner`, which only the cargo-liner tile (right screen column) draws; sdlc fits its role to `unit`. Find which value sets that width (a row not drawn, such as the tile's own root shown as its title, a shared width across screen columns, or a role label that differs from the drawn text) and write the cause in the summary. The role column is as wide as the widest role that the tile group draws, never less than its header.
2. Spent timer: a shell row whose timer has reached zero while the shell still runs draws `running` in the Busy ink, as an untimed running shell does. A timer with time left keeps its digits. The live case: a shell running `for i in 1 2 3 4 5 6; do sleep …` showed `0:00` for over a minute. `ShellState` picks its ink without a clock while its text reads `now`, so convert the timer input once per frame into one clock-aware resolved state (for example `Running` or `CountingDown(PositiveRemainingTime)`) that gives both the text and the ink.
3. Drawn column groups (showrunner ruling, 2026-10-22): in one screen column, tiles that draw the same column set share widths and age slots. Today tiles group by populated columns (`VisibleTreeColumns`) before the fit, and `tree_header` can then drop columns, so tiles from different groups can draw the same set with different widths. A fit per group is not enough: group A (populated branch, directory, type) can drop directory and draw {branch, type}, the same set as group B (populated branch, type), with other widths. Construction: columns only drop, in one fixed drop order of at most 3 droppable columns. Iterate over that fixed list, never an open loop: at each drop step, merge the groups whose fitted sets are equal, refit on the merged widths, then go to the next step, so the loop ends by its type. The group key's name states only the guarantee the code keeps.
4. Summary empty columns (showrunner, 2026-10-22): the summary's top-level table drops a column when no row has a value, by the same rule as the tiles; in the phase 23 shots `desktop` shows `—` in all 4 rows. Headers, widths, sort and clicks follow the drawn columns. The checkpoint also updates the phase 15 line that lists the summary columns, so it states the new rule.
5. Tests, red first: a rendered tile group whose drawn roles are `impl` and `review`, beside a tile in another screen column that draws `showrunner`, draws its role column 6 cells wide with a 2-cell gap to status; a shell row at the exact deadline and on a later frame reads `running` in the Busy ink, and one with time left reads its digits in the Shell ink; the cross-group case: two tiles in one screen column from different populated groups, whose fit makes their drawn sets equal, draw every header at the same x and share age slots; a summary whose rows have no desktop value draws no `desktop` header, and one with a value keeps it.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/census/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/agent_cell/tests/`

**Workers:** 1 writer + 1 tester.
- `impl` — the role width cause and fix, the spent-timer state; CHANGELOG.
- `test` — the red rendered tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: framed shots, dark and light, of the left screen column's role column and of a running shell past its timer. The checkpoint carries `Change-Review: the role column fits its words, and a shell past its timer reads running | On natedev, run cargo-handler while a shell loop runs past its timer | The role column has the same small gap as the other columns, and the shell row says running instead of 0:00`.

### Phase 18 — The shortcuts overlay pages only when it must  · status: todo

**Depends on:** Phase 17

#### Work Order

**Goal:** the global shortcuts overlay shows every shortcut on one page when the terminal is tall enough, in every tui_pane app.

**Source:** the user (via the showrunner), 2026-10-20: "the global shortcuts shows a page 1 of 2 but there is PLENTY of row space to show all global shortcuts - if it can show all shortcuts it should - this should apply to all tui_pane apps - shared code - if it isn't already". Showrunner ruling: its own small phase after phase 17; fix it once in tui_pane, so cargo-tile, cargo-handler and cargo-port all get it; red-first tests at a tall and a short height; shots in cargo-tile and cargo-handler; a Change-Review line. If berth shows another holder on the overlay files, send the overlap to the showrunner.

**Constraints from prior phases:**
- The overlay draws in `crates/tui_pane/src/overlays/global_shortcuts.rs` (`draw_global_shortcuts_overlay`) from rows that `keymap/runtime_scope.rs` (`render_app_global_shortcut_rows`) and `overlays/keymap_ui.rs` build. Paging uses `layout/viewport.rs`.
- No app keeps its own copy of the overlay; the fix goes in tui_pane only.
- The cause is in `popup_height`: it caps the overlay at `GLOBAL_SHORTCUTS_POPUP_MAX_HEIGHT` (24, `crates/tui_pane/src/overlays/constants.rs:2`). Usable height counts section and blank lines.
- cargo-handler already draws its own status bar: dark bars rgb 60,60,60 (`theme.rs` ≈397, ≈499; `themes/default_dark.toml`), a Default Light bar rgb 250,250,250 (`themes/default_light.toml`). The gray and #dcdcdc bars of Spec 5 are the tui_pane fallback theme (`crates/tui_pane/src/theme/fallback.rs` ≈73, ≈118). Fix the fallback; do not change cargo-handler's bars, and keep its chrome regressions passing.

**Spec:**
1. Find the cause of the page split: the page height the overlay computes is less than the rows it can draw in the terminal area it gets. Write the cause in the phase summary.
2. The overlay pages only when its rows do not fit the height it can use. When all rows fit, it draws them all and shows no page marker ("1 of 2" or the like). When they do not fit, it pages as now, and each page fills the height it can use.
3. The page count is derived from the row count and the usable height, so a page marker can not show while every row fits (one type or one function owns that decision; no second guard).
4. Tests, each red before its fix: at a tall terminal height where every global shortcut row fits, the rendered overlay shows every row and no page marker; at a short height, it shows a page marker and the first page holds as many rows as the usable height allows; the last row of the last page is reachable.
5. The tui_pane fallback status bar is readable: in dark, a dark bar (about rgb 60,60,60) instead of gray #7f7f7f, under which yellow #cdcd00 and cyan were 2.0–2.35:1 and white 4.0:1. In light, `fallback_light` (`theme/fallback.rs` ≈88) draws a black active title, an rgb 70,70,70 inactive title and an rgb 0,95,135 accent, which `BarPalette::themed` reads; the code has no #a06400 ink. The #a06400 label on #dcdcdc (3.55:1) in the earlier shots comes from a named terminal color or an app's own span: find its source and write it in the phase summary before changing it. Test the drawn enabled, disabled, activity, label and value spans on each app's rendered bar, and every one reaches 4.5:1 in all four themes, cargo-handler's own bars included, without changing cargo-handler's bar colors (showrunner, 2026-10-20). Contrast tests compute the expected ratio with their own literal 208/255 glyph coverage, not by reading a production constant.
6. The overlay test lives in its own module; it fails at the old 24-row cap.

**Files:**
- `crates/tui_pane/src/overlays/global_shortcuts.rs`, `crates/tui_pane/src/overlays/keymap_ui.rs`, `crates/tui_pane/src/overlays/mod.rs`, `crates/tui_pane/src/layout/viewport.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/overlays/`
- `crates/tui_pane/src/bar/status_bar.rs`, `crates/tui_pane/src/bar/palette.rs`, `crates/tui_pane/src/theme/fallback.rs`

**Workers:** 1 writer + 1 tester.
- `impl` — overlay height, status bar and fallback theme production code, constants and module registrations; CHANGELOG.
- `test` — a separate overlay test module and status bar contrast tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-tile`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`, `bash ~/.claude/scripts/direct/verify.sh test cargo-port` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` clean. No test takes a second. Live: framed shots of the global shortcuts overlay in cargo-tile and cargo-handler at a tall and a short terminal height. The checkpoint carries `Change-Review: the global shortcuts overlay shows every shortcut on one page when the terminal is tall enough, and pages only when it is not | open the global shortcuts overlay in cargo-tile and cargo-handler in a tall terminal, then in a short one`.

### Phase 19 — Accounts table matches the accounts board  · status: todo

**Depends on:** Phase 17

#### Work Order

**Goal:** the cargo-handler accounts table and `/accounts-board` show the same columns, in the same order, with the same words and formats, so a reader sees one view of the accounts in both places.

**Source:** the user (via the showrunner), 2026-10-20: "the accounts table has multiple problems - the % should align right in the weekly left - the 11h old should not show up in the accounts space - i think it corresponds to the 'read' column in the /accounts-board - it should show up on the right - we should have an in use column which should say yes or blank and an enabled with a yes or blank". Then (via natedev and the showrunner): the cargo handler and the accounts board show basically the same thing; use the board's green/red mark for whether the account can be read; "left" is better than "weekly left"; in use and enabled are columns in both views. Showrunner ruling, final shared spec (also sent to unit-board, which owns `/accounts-board`): "enabled" and "in use" are about credits. Columns: dot, account, role, left, resets, credits, enabled, in use, read. All headers lower case. Every value comes from `accounts.py --json`. unit-board adds `credits_in_use` and `read_ok` there and tells this unit when they are on `~/.claude` main; unit-board contacts this unit directly about any change to the spec.

**Constraints from prior phases:**
- The table draws in `summary/accounts.rs` (`sorted_lines`, `values`, `columns`, `header_spans`; `STALE_ACCOUNT_SUFFIX` makes the "NNh old" text in the account cell today). `census/accounts.rs` reads the `accounts.py` JSON.
- Phase 12: reset times shade by rank (`theme::ResetRank`), headers sort the table (`sort::AccountsColumn`, `AccountsSort`). Phase 15: the role column draws each role word in its role color. These stay.
- `accounts.py status --json` sends `read_ok` (true when a reading is stored and the latest read outcome is OK or HELD) and `credits_in_use` on each account, on `~/.claude` main since abcecd0d (unit-board, 2026-10-20). `/accounts-board` on main already draws the shared table; send any difference phase 19 finds to unit-board.
- Do not edit `~/.claude`. If the JSON has no field a column needs, tell the showrunner the exact field name.
- `AccountUsage.credit_use: CreditUse::{Spending, Held}` comes from `use_credits`, which is permission, not spending. Rename that domain type, and convert `read_ok` and `credits_in_use` at the wire boundary into named states.
- Phase 15 made the Roles column per-role colored spans (`summary/accounts.rs` `role_words` ≈344); new columns keep those spans.
- Phase 23 full suite: `render::role_title_tests::session_name_inks_stay_apart_from_roles_and_readable_on_the_drawn_ground` took 1.043 s (0.280 s alone). This phase rewrites that test under item 15; the new one runs under a second in the full suite.
- Phase 23 design check: the dark summary `left` red measured #cd0000 3.60:1 and #890000 2.06:1 on black. Item 13 owns it.

**Spec:**
1. Columns, in this order: a dot with no header, `account`, `role`, `left`, `resets`, `credits`, `enabled`, `in use`, `read`. Every header is lower case. Every column is always shown. Two spaces between columns; nothing is cut off.
2. dot: `●`, green when `read_ok` is true (the last usage read worked and a reading is stored), red when it is false or the field is missing.
3. account: the account name only. The "NNh old" text leaves this cell (remove `STALE_ACCOUNT_SUFFIX`).
4. role: the roles joined with `, `, each in its role color as phase 15 made it.
5. left: the weekly left percent, rounded, as `60%`; `—` with no reading. Red under 10 %, yellow under 25 %, the default color otherwise.
6. resets: `14:00` on the same day, `Sat 14:00` within six days, `24 Oct` after that; the phase 12 gradient stays.
7. credits: `13,446` (rounded, with thousands separators) or `unlimited`; blank with no balance.
8. enabled: `yes` when `use_credits` is true, else blank. in use: `yes` when `credits_in_use` is true, else blank.
9. read: the age of the usage reading as `45s`, `12m`, `2h`, `3d`, with no "ago"; blank with no reading.
10. Numbers are right-aligned: left, resets, credits and read. The `%` signs line up in one column. The existing numeric columns are right-aligned already; keep that as a preservation check, with red tests only for the new columns.
11. Header sort (phase 12) covers every new column; `AccountsColumn` gains the variants it needs. The read age and the yes columns sort with blank last.
12. Tests, each red before its fix: a rendered table has the nine columns in order with lower-case headers; the account cell holds only the name, and the read column holds the reading age; the dot is green for `read_ok` true and red for false and for a missing field; left is red at 9 %, yellow at 24 %, plain at 25 %, `—` with no reading; resets formats a same-day, a this-week and a later time; credits shows `13,446` for 13446.4 and `unlimited`; enabled and in use show `yes` or blank from their fields; the `%` signs and the last digit of each number column stand in one screen column in every row.
13. Dark low-left red #890000 on black is 2.06:1: use the theme bright red; every numeric column ink reaches 4.5:1 (showrunner, 2026-10-20). A rendered test reads every numeric column on its drawn grounds in all four themes, including the reset-gradient endpoints and middle ranks and the DIM credit text, and requires 4.5:1 with its own literal 208/255 coverage (`summary/accounts.rs` ≈439 draws raw gradient colors and DIM credits).
14. Tests cover every new header click toggle, blank-last order and the per-role spans.
15. Session-name hues (`theme.rs` `RainbowHue`) stay clear of the status, account and role palettes. The phase 17 design check measured near matches: dark session green #6edc6e vs `running` #61d661 and session cyan #50d2e6 vs `command` #57c2d6; light session green #008700 vs `thinking`/`running` #008200, session teal #047a8e vs `command` #0e7f9c and codex2 #007389, session red #c81e1e vs codex1 #e3004c. Pick session hues outside those palettes. Tests, red first: every session hue against every status ink, account ink and role ink by CIEDE2000 >= 12, and each session hue at 4.5:1 on its drawn ground with glyph coverage (`InkCoverage::Antialiased`), in all four themes (showrunner, 2026-10-21, phase 17 design check D2). Phase 21 runs before this phase and adds the Astra role inks with their own distance and contrast checks; these checks read every role ink through the production role API (`FooterColors::hierarchy_role`), Astra inks included, and keep phase 21's checks passing. Contrast tests compute the expected ratio with their own literal 208/255 glyph coverage, not by reading `ANTIALIASED_GLYPH_COVERAGE`.
16. Unstable test (showrunner, 2026-10-21): `render::accounts_grid_tests::accounts_grid_measured_tables_sort_account_headers_without_changing_agent_order` panicked once with "missing grid label codex2" (`accounts_grid_tests.rs:39`), at 0.51 s in a 32-thread style merge run, and passed on the rerun. Find the shared state or timing that lets the label go missing under load, and fix that cause. A retry, a longer wait or a looser assertion is not a fix. Write the cause in the summary; the test stays under a second.

**Files:**
- `crates/cargo-handler/src/summary/accounts.rs`, `crates/cargo-handler/src/summary/constants.rs`, `crates/cargo-handler/src/census/accounts.rs`, `crates/cargo-handler/src/sort.rs`, `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/summary/accounts_tests.rs`, `crates/cargo-handler/src/summary/accounts_header_sort_tests.rs`, `crates/cargo-handler/src/summary/accounts_gradient_tests.rs`, `crates/cargo-handler/src/summary/accounts_refresh_tests.rs`, `crates/cargo-handler/src/render/role_title_tests.rs`, `crates/cargo-handler/src/render/accounts_grid_tests.rs`, `crates/cargo-handler/src/census/accounts_wire_tests.rs`

**Workers:** 1 writer + 1 tester.
- `impl` — accounts production files (`summary/accounts.rs`, `summary/constants.rs`, `census/accounts.rs`, `sort.rs`), the session palette in `theme.rs` and the built-in theme files, and test module registrations; CHANGELOG.
- `test` — the red tests in the accounts test files, a separate census account-wire test module (`census/accounts_wire_tests.rs`: `read_ok` and `credits_in_use` conversion into named states, missing fields included), the composed-grid palette tests in `render/role_title_tests.rs`, and the cause of the unstable accounts grid test (`render/accounts_grid_tests.rs`, item 16; a cause in production code goes to `impl`).

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: framed shots of the accounts table, in a dark and a light theme, with an account that cannot be read, one under 10 % left, one with credits enabled and in use. The checkpoint carries `Change-Review: the accounts table shows a dot, account, role, left, resets, credits, enabled, in use and read, as the accounts board does, with right-aligned numbers and the reading age in its own column | open cargo-handler and /accounts-board side by side and compare the accounts tables`.

### Phase 16 — Row selection in tiles  · status: todo

**Depends on:** Phase 17

#### Work Order

**Goal:** a cargo-handler tile has a selected row that the keys and the mouse move, as in cargo-port, from one shared tui_pane implementation.

**Source:** the user (via the showrunner), 2026-10-20: cargo-handler row selection with arrow navigation and the mouse, like cargo-port; if the code is not shared in tui_pane and it makes sense, make it shared. Showrunner rulings: a phase after phase 15 and before phase 9; read cargo-port first; the shared code moves into tui_pane, and cargo-port uses it from there with no copies; cargo-port-only code stays; cargo-port's behavior and look do not change and its tests pass with only import changes; the selection is a type that can only name an existing row and moves to a defined neighbor when its row goes away; the row identity names every row kind the tree draws, and a row kind with no pid (a group or summary line) gets its own variant, never an invented pid. Keys (accepted as recommended): Up, Down, PageUp, PageDown, Home and End move the selected row in the focused tile; Left, Right, Tab and Shift-Tab keep moving the tile focus; no wrap at the edges. Order 15, 17, 18, 19, 16, 9.

**Constraints from prior phases:**
- cargo-port today: `tui/panes/actions.rs` (`navigate_viewport`), `tui/input/dispatch.rs` (`scroll_if_contains`), `tui/panes/hit_test.rs` (`table_row_at`).
- tui_pane: `layout/viewport.rs` (`Viewport`), `runner/event_loop.rs` (the event loop does not forward the wheel to the host today), `runner/app.rs`, `tiles/host.rs` (`TileGridHost`), `tiles/grid.rs` (`TileGrid::focused_cell`, private today).
- cargo-handler: arrows move the tile focus today (`globals.rs`); rows draw in `agent_cell.rs` (`draw_tree`); clicks route through `render.rs` (`cell_clicked`); per-tile state is kept by `TileKey` (`App.tree_sorts`); row identity lives in `census/placement.rs`.
- `placement::RowIdentity` is a borrowed machine-and-pid pair; subagent ids are lost before `ChildRow`, and several work rows have no pid. Add an owned selection identity, keep stable work ids through census and probe conversion, and build selections only from the resolved row set.
- `TileKey` names only agent and unassigned tiles. The selection surface must also name the summary (its accounts and group rows); derive eligible rows from `RootPresentation`.
- Phase 15: the summary lists only sessions with a hierarchy; a standalone session lives only in the unassigned tile. A session moves between tiles when it gains or loses its last visible child; selection follows the exact row.
- The cargo-port functions are `scroll_viewport_if_contains` (`tui/input/dispatch.rs:725`) and `hit_test_table_row` (`tui/panes/hit_test.rs:11`).
- Phase 17 folds a dispatch shell into its seat row: the row's identity is the seat's `WorkerIdentity::{CodexThread, ClaudeSession}` (`census/worker_dispatch/mod.rs` ≈226), not the shell pid. `census/tree.rs` `subagent_node` (≈648) drops the subagent id, and `ChildRow` (`census/mod.rs` ≈442) keeps only `pid: Option<u32>`, which cannot name every selectable row. Carry the seat and subagent identities through census construction (`census/tree.rs`) and probe conversion (`census/probe.rs`); a remote report from an older sender with no identity gets a named legacy variant, never an invented id.
- Phase 17 age slots: `summary/age.rs` `AgeLayout` measures each tile's age column from its complete row set, after the shared screen-column widths fit (`App.tree_layouts`). Scrolling slices the drawn rows only; it must not re-measure ages from the visible slice. Phase 23: `shared_tree_metrics` builds one `AgeLayout` and one populated column set per screen-column group from the complete rows of every tile in the group; scrolling keeps both. Test: two tiles in one group where the only role value and the widest age scroll out of view keep their role column, age slots and header x.

**Spec:**
1. Move into tui_pane: `Viewport::navigate(NavAction)` (from cargo-port `navigate_viewport`), `Viewport::scroll_if_contains` and `table_row_at`. cargo-port calls them from tui_pane; its behavior, look and tests do not change beyond imports.
2. The tui_pane runner forwards the mouse wheel to the host (a `TerminalApp` scroll hook and `TileGridHost::cell_scrolled`); `TileGrid::focused_cell` becomes public for the host.
3. cargo-handler keeps a `Viewport` and a selection per selection surface, `enum SelectionSurface { Summary, Tile(TileKey) }`, because `TileKey` names only agent and unassigned tiles. The selection is `enum TileSelection { NoRows, Row(RowIdentity) }`; `RowIdentity` has one variant per row kind the screen draws (session, seat by `WorkerIdentity`, thread, subagent by its id, shell, group line, summary account row, summary group row), each carrying the identity that kind already has, never an invented pid.
4. Each frame resolves the selection against the drawn rows: a missing identity moves to the row now at its old index, else the last row, else `NoRows`. The selected row stays in view.
5. Keys: Up, Down, PageUp, PageDown, Home and End move the selected row in the focused tile and stop at the edges. Left, Right, Tab and Shift-Tab move the tile focus as now. A click on a row selects it; the wheel scrolls the tile under the pointer. The selected row draws with the shared selection style.
6. Tests, each red before its fix: each key moves the selection as stated and stops at the edges; a click selects the row under the pointer in a tile; the wheel scrolls the tile under the pointer, not the focused one; when the selected row's session ends, the selection moves to the row at the old index, then to the last row, then to `NoRows`; a group line can be selected; cargo-port's existing navigation tests pass unchanged.
7. Tests also cover duplicate descriptions, renames, pid reuse, row removal, summary selection, and a session moving between its hierarchy tile and the unassigned tile.
8. Tests also cover: a selected seat row stays selected across a new dispatch to that seat, a seat relaunch and a remote probe round trip; key moves and row removal on the summary surface and on a tile surface; a scrolled tile keeps its unit letters in the same screen columns and its age header whole.
9. A selected row stays readable. The selection background is a ground the inks are solved on (`footer_colors/mod.rs` `InkGrounds` gains it). Rendered tests, red first, read back the status, shell timer, account, role and session inks on a selected row in a focused and an unfocused tile and require 4.5:1, computed with their own literal 208/255 glyph coverage, in all four built-in themes. If the shared selection style leaves an ink under 4.5:1, send the conflict to the showrunner before choosing a palette.

**Files:**
- `crates/tui_pane/src/layout/viewport.rs`, `crates/tui_pane/src/runner/event_loop.rs`, `crates/tui_pane/src/runner/app.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/lib.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/cargo-port/src/tui/panes/actions.rs`, `crates/cargo-port/src/tui/input/dispatch.rs`, `crates/cargo-port/src/tui/panes/hit_test.rs`
- `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/globals.rs`, `crates/cargo-handler/src/keymap.rs`, `crates/cargo-handler/src/interaction.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/footer_colors/mod.rs`, `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/agent_cell/tests/`

**Workers:** 1 writer + 1 tester.
- `impl` — shared tui_pane exports, cargo-port and cargo-handler production code and registrations, the selection ground in `InkGrounds`; CHANGELOG.
- `test` — separate shared-navigation and handler-selection test files, including the selected-row contrast tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-port`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`, `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: framed shots of a cargo-handler tile with a selected row after key moves, after a click and after a wheel scroll, in a dark and a light theme. The checkpoint carries `Change-Review: a cargo-handler tile has a selected row that the arrow, page, home and end keys move, a click selects and the wheel scrolls, as in cargo-port | open cargo-handler, move the selection with the keys, click a row and scroll a tile with the wheel`.

### Phase 20 — Click a word to highlight every instance  · status: todo

**Depends on:** Phase 16

#### Work Order

**Goal:** a click on a word in cargo-handler gives every instance of that word in the window a subtle, clearly visible background.

**Source:** the user (via the showrunner), 2026-10-20: "if i click on a word i want every instance of that word highlighted - i guess with a background color that is subtle but definitely visible". Showrunner rulings: a phase after Phase 16, because both are mouse work in the same place; order 15, 17, 18, 19, 16, 20, 9; the five points of the unit's proposal as written; a click on the highlighted word at another place also clears the highlight; one Change-Review line.

**Constraints from prior phases:**
- Phase 16 makes a click on a row select it (`render.rs` `cell_clicked`); a click now does both things.
- Theme roles live in `theme.rs` (`Role`) and the built-in theme files in `crates/cargo-handler/themes/`; text colors stay as they are drawn.
- The code stays in cargo-handler: no other crate uses it yet, so it adds no tui_pane surface.
- Phase 15 CIEDE2000 12 measures distinction from header ink, not readability. `ReadableInk` measures against `theme.text.bg_focus`. Use the luminance contrast oracle (4.5:1) against the real match background in all four themes, for subdued role and account ink and for selection overlap; foreground colors do not change.
- Phase 17: `footer_colors/mod.rs` `InkGrounds` covers the base, focused and unfocused grounds, and account, management and session inks use glyph coverage (`InkCoverage::Antialiased`). Measure every ink on the drawn match and selection backgrounds with that same coverage calculation, Astra role inks included. If no match background keeps every unchanged foreground at 4.5:1, send the conflict to the showrunner before accepting a palette.
- Phase 16's selected-row contrast tests stay green; where a match overlaps a selected row, the match background is the ground. Contrast tests compute the expected ratio with their own literal 208/255 glyph coverage, not by reading `ANTIALIASED_GLYPH_COVERAGE`.

**Spec:**
1. A word is a token between spaces in any drawn cell, with leading and trailing punctuation removed. A match is the whole word with exact case: a click on `claude1` marks every `claude1` and no `claude12`.
2. One click marks every instance in the whole window: every agent tile, the unassigned tile, the summary and the accounts table. The highlighted word is one type, `enum WordHighlight { Off, Word(HighlightWord) }`, where `HighlightWord` can only hold a non-empty word.
3. A click on a row selects the row (Phase 16) and sets the word. A click on the highlighted word, at any place, a click on empty space, or Esc clears it. A click on another word moves the highlight to that word.
4. The highlight is a new theme role, `word_match`, with a background in each built-in theme: subtle but easy to see in light and dark themes. The text keeps its color and stays readable on it (ink contrast measured as in Phase 15, bound named in the checkpoint). One color for every match. Where the selected row and a match overlap, the match background draws on top.
5. The mark is one pass over the drawn frame after every tile draws, and runs only while a word is set. So it follows the word through every redraw, a new row that holds the word gets it too, and it costs nothing when no word is set.
6. Tests, each red before its fix: the token rules (punctuation removed at the ends, whole-word match, exact case); matches marked in two tiles and in the accounts table; a new row with the word is marked on the next frame; clear on empty space, on Esc, on a second click on the same word and on a click on the word at another place; a click on another word moves the highlight; a render test that reads the background colors back from the frame.
7. A contrast test reads the drawn match background and each ink on it in all four themes and asserts at least 4.5:1 with glyph coverage, on the base, focused and unfocused grounds, selection overlap included.

**Files:**
- `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/globals.rs`, `crates/cargo-handler/src/keymap.rs`, `crates/cargo-handler/src/interaction.rs`, `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/src/word_highlight.rs`, `crates/cargo-handler/themes/`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/word_highlight_tests.rs`, `crates/cargo-handler/src/render/`

**Workers:** 1 writer + 1 tester.
- `impl` — the word type, the click and Esc handling, the frame pass, the theme role and colors; CHANGELOG.
- `test` — the red tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: framed shots, in a dark and a light theme, of a word highlighted across two agent tiles, the summary and the accounts table, and of the same screen after the highlight clears. The checkpoint carries `Change-Review: a click on a word in cargo-handler highlights every instance of it in the window with a subtle background, and a second click, Esc or a click on empty space clears it | open cargo-handler, click an account name, then click it again`.

### Phase 21 — Astra designer and Astra reviewer roles  · status: todo

**Depends on:** Phase 17

#### Work Order

**Goal:** cargo-handler shows an Astra designer session and an Astra reviewer session (Codex on gpt-6-astra) under the unit director or showrunner that runs it, each with its own role word and color.

**Source:** the user (via hana and the showrunner), 2026-10-20: cargo-handler shows the two new Codex roles of the in-game metrics production; a designer is a lasting role that can be resumed and runs under a unit director or a showrunner. Showrunner rulings: a phase after Phase 17; order 15, 17, 21, 18, 19, 16, 20, 9; role words `astra designer` and `astra reviewer`; colors pink and bright-green; if the hierarchy edge is not on `~/.claude` main when Phase 17 checkpoints, run Phase 18 and return here when the showrunner says it landed. Lasting roles read from `roles.py` are not in this phase (see the note below).

**Showrunner note (2026-10-21, revised guild design):** there are four guilds: `designers`, `reviewers`, `showrunners` and `units`. Workers have no guild. Each head is a unit director whose unit name is its guild ID, under a new top-level production `guilds` (herdr space `guilds` on natedev: the guilds showrunner in tab 1, then the four heads). In the census the heads are ordinary unit edges under the guilds showrunner, and each member keeps its real manager: an Astra designer stays under the unit director or showrunner that holds it, never under the `designers` head. `astra-designer` and `astra-reviewer` edges and colors do not change. Lasting roles will come only from sdlc phase 6 `scripts/guilds/roles.py list --json`; parse no other file for them. That file is not on `~/.claude` main, and this phase does not need it: reading it is a later phase. Tips are files in `~/rust/nate_style/guilds/<guild>/`; there is no guild message group and no publication record, so cargo-handler draws nothing for them. Design doc: `docs/plans/sdlc-guilds-design.md` on `~/.claude` branch `sdlc-guild-process` (dadb482b), sections "Guilds and heads" and "Data for cargo-handler".

**Constraints from prior phases:**
- Phase 16: a selected row draws on the selection ground (`InkGrounds` in `footer_colors/mod.rs`). Rendered tests also draw both Astra roles on a selected row in a focused and an unfocused tile and require 4.5:1.
- natedev adds to `production_tree.py --json` one edge per Astra session while it holds the role: `{"manager": <Claude session id of the unit director or showrunner>, "member": <Codex thread id>, "role": "astra-designer" | "astra-reviewer", "codex_home": <CODEX_HOME>, "state": "busy" | "idle"}`, and the `statusline.conf` keys `role.astra-designer=pink` and `role.astra-reviewer=bright-green`. Do not edit `~/.claude`.
- Today `census/hierarchy.rs` drops an edge whose role it does not know (`HierarchyWireRole` `#[serde(other)] Unknown`); `HierarchyRole` is in `census/mod.rs`; `RoleLabel` is in `census/placement.rs`; `FooterRole` and its `role.*` keys are in `footer_colors/mod.rs`. Since phase 17 there is no `From<HierarchyRole> for FooterRole` and no `FooterRole::Worker`: management roles map in `FooterColors::hierarchy_role` (`footer_colors/mod.rs` ≈548), specialist role words in `FooterColors::named_role` (≈539), and worker rows draw plain.
- Phase 17 probe schema 3: `ProbeChildWork` sends the unit kind `subagent` with an optional `subagent_role`. Keep its compatibility tests beside the new hierarchy extension tests: a capable receiver keeps recorded subagent roles; an older receiver accepts the report without the extension.
- The config gives pink to both `role.astra-designer` and `account.codex1`, and both solve to the same ink. Showrunner ruling, 2026-10-21: `role.astra-designer` moves off pink and `account.codex1` keeps it; no existing color name clears every ink, so the new name is `mauve` = xterm 139 (#af87af; solved dark #af87af, tint on #c2a4c2; light #734c73, tint on #5e3f5e; at least CIEDE2000 15.56 from every account, role, status, header, plain-text and rainbow ink; 4.51–4.77:1 at 208/255). natedev adds `mauve` = `38;5;139` to `statusline.jq` and sets `role.astra-designer=mauve`; this phase adds `mauve` (index 139) to `FooterColor` in `footer_colors/mod.rs`, parsed from `mauve`. This phase does not wait for natedev's change: its distance test uses a fixture with the `statusline.conf` values and `role.astra-designer=mauve`, and the live shots use the real config: the change is on `~/.claude` main at a06402f7.
- Phase 21 runs before phase 19, so this phase does its own complete palette checks for the Astra inks through the production role API (`FooterColors::hierarchy_role` with the new variants), not through fixed colors: against every session hue, every account ink and every existing role ink. Phase 19 later keeps them passing. Contrast tests compute the expected ratio with their own literal 208/255 glyph coverage, not by reading `ANTIALIASED_GLYPH_COVERAGE`.
- The Codex thread lookup in `constants.rs` reads only `originator = 'codex-tui'` threads; `astra_review.py` runs `codex exec` (`originator` `codex_exec`).
- The word `review` stays the pool reviewer's.
- `codex::read_threads_by_id` already accepts any originator (`CODEX_THREAD_BY_ID_QUERY`); only the startup-window query limits originator to `codex-tui`. Do an edge-named lookup in the named Codex home; do not widen the startup query. Reuse `SessionNames` for renamed sessions; keep the phase 15 home and resume checks (`thread_matches_home`, `ResumeRequest`) and the no-dispatch regression.
- The scanner gets no hierarchy snapshot today, and app-server discovery needs Linux open file descriptors under the default sessions directory. Hand accepted edges from the scheduler to the scanner; keep database and rollout reads off the scheduler thread and outside the publication lock.
- `hierarchy.rs:641` turns any wire `state: busy|idle` into `ReviewerState`; pool validation makes an edge without a lease idle and `is_drawn` hides it. Astra sessions get their own semantic state, outside the reviewer-pool state machine.
- `HierarchyEdge` today joins a role with `ReviewerState` and has no Codex home field. An Astra edge is one typed payload that requires its Codex home and its busy or idle state (for example `AstraSeat { role: AstraRole, codex_home: CodexHome, state: AstraState }`); the optional wire fields convert into it at the input boundary, and an edge without them is refused there. Do not add separate domain `Option` fields that permit an Astra member without its address.
- Probe schema 3 serializes `HierarchyEdges` directly; new role variants in `edges` make older receivers refuse a probe. Carry new roles in a defaulted extension.
- Phase 15 fallbacks stay: a session with no name shows its pid text, and an unknown footer color name draws plain.
- The edge is on `~/.claude` main at 60ca146b (natedev and the Mac): `production_tree.py --json` gives `{manager: Claude session id, member: Codex thread id, role: astra-designer|astra-reviewer, codex_home, state: busy|idle}`; a held thread no longer also prints as a worker edge. `statusline.conf` has `role.astra-designer=pink` and `role.astra-reviewer=bright-green`. Live shots: `/astra_review` holds the reviewer role while its Codex process runs; hold a designer by hand with `~/.claude/scripts/lib/py ~/.claude/scripts/astra/astra_roles.py hold designer --session-dir <delegate dir> --worker <name> [--manager <session id>]`, release with `astra_roles.py release <thread id>`.

**Spec:**
1. `HierarchyWireRole` and `HierarchyRole` gain `AstraDesigner` and `AstraReviewer`, parsed from `astra-designer` and `astra-reviewer`. Their role words are `astra designer` and `astra reviewer` in the role column and the tile title.
2. `FooterRole` gains `AstraDesigner` and `AstraReviewer`, read from `role.astra-designer` and `role.astra-reviewer`; the role word draws in that color everywhere a role word draws (Phase 15 rules).
3. An Astra row draws under its edge's manager, a unit director or a showrunner alike, and keeps the edge's `state` as its idle or busy status between turns.
4. The thread lookup also reads a `codex_exec` thread when an Astra edge names it, under the edge's `codex_home`, so the row gets its session name, activity and fast mark as other Codex rows do.
5. Tests, each red before its fix: both roles parse and keep their words; each takes its configured color; an Astra row draws under a unit director and under a showrunner; a `codex_exec` thread named by an edge gets its name and activity; an unknown role is still dropped.
6. Tests also cover: an edge-named `codex_exec` thread with no open app-server rollout; busy and idle Astra rows visible with no reviewer-pool lease; an older receiver parsing a new probe, and a new receiver restoring role, home and state; each Astra role ink, read through the production role API, at CIEDE2000 >= 12 from every session hue, every drawn account ink and every existing role ink, and at 4.5:1 with glyph coverage in all four themes; an Astra edge with no Codex home or no state is refused at the wire boundary.
7. Guild heads: a hierarchy with a `guilds` showrunner and four unit directors named `designers`, `reviewers`, `showrunners` and `units` draws them as ordinary unit director rows under that showrunner, with no new role word. An Astra designer held by a unit director of another production stays under that unit director. Rendered tests cover both.

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/codex.rs`, `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/footer_colors/mod.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/cargo-handler/src/census/astra_role_tests.rs`, `crates/cargo-handler/src/footer_colors/tests.rs`
- `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/census/schedule.rs`

**Workers:** 1 writer + 1 tester.
- `impl` — the role variants, the colors, the placement and the thread lookup; CHANGELOG.
- `test` — the red tests.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean. No test takes a second. Live: framed shots, in a dark and a light theme, of an Astra designer under a unit director and an Astra reviewer under a showrunner, with the real config. The checkpoint carries `Change-Review: cargo-handler shows Astra designer and Astra reviewer sessions under the unit director or showrunner that runs them, with their own role words and colors | open cargo-handler while an Astra review runs and find its row under its parent`.
