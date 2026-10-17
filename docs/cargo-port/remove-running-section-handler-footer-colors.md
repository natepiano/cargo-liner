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

### Phase 7 — First scan under two seconds  · status: todo

**Depends on:** Phase 6

#### Work Order

**Goal:** cargo-handler's first full scan on natedev finishes in under 2 s in a release build, and no later scan re-reads a whole transcript.

**Source:** the showrunner, 2026-10-17: "A 35 s first scan is a defect; fast comes before features. Target: first full scan under 2 s in release on natedev, measured before and after, with the cost named per source. Read only new transcript bytes, and never re-read whole files each scan."

**Constraints from prior phases:**
- Measured 2026-10-17 00:20 EDT on natedev: `~/.cargo/bin/cargo-handler probe` (release, installed 22:45) takes 35 s for a full scan; the debug build took 218 s. Roles no longer wait on this scan: hierarchy discovery runs at the `ScanStage::Classified` publication, before detail reads.
- Detail reads live in `census/scan.rs`, and `census/tier.rs` (`RolloutRead`) reads Codex rollouts incrementally with a per-path offset. The Claude transcript read added for worker rows (job text, last `tool_use`) is the likely cost; transcripts reach 1 GB.
- The full-file Claude activity read starts in `TranscriptRead::advance` (`census/activity.rs`); `census/transcript.rs` owns the historical-span and subagent-tail readers.
- `RolloutRead` takes the session start, worktree, first task and service tier from records that can sit any distance before the tail, and `scan.rs` uses that first task to find rosters and detached gates. A tail alone cannot supply them.
- Measure under the real config home. A scratch `XDG_CONFIG_HOME` makes the hierarchy command drop every showrunner-to-unit edge, so a capture that needs one prefixes `hierarchy_command` with `XDG_CONFIG_HOME=/home/natepiano/.config`.

**Spec:**
1. Measure before changing anything: time the release `cargo-handler probe` on natedev, and name the cost of each detail source (Claude transcripts, Codex rollouts, session files, process tree, accounts). Put the table in the summary.
2. Every transcript and rollout source reads only bytes appended since its last read, kept as a typed per-path position. A first read takes only what the row needs (for example the transcript's tail, read backward to the last whole record), never the whole file. Head metadata (session start, worktree, first task, service tier) is read once per path, kept as a typed bootstrap state beside the position, and never re-read; historical lookups that need older records read only the span they need.
3. The first full scan finishes in under 2 s in release on natedev, measured after the change with the same command, cost per source in the summary.
4. Tests: a source read twice reads only the appended bytes the second time; a first read of a large file reads a bounded tail; rows read from a tail match the rows read from the whole file, including fixtures that put the head metadata records outside the tail; a byte count per read proves the bound.

**Files:**
- `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/tier.rs`, `crates/cargo-handler/src/census/activity.rs`, `crates/cargo-handler/src/census/transcript.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/scan/`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — the measurement, the incremental and tail reads, `mod` lines, CHANGELOG.
- `test` — the append, bounded-tail and tail-equals-whole tests and their fixtures.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean; the release `cargo-handler probe` on natedev finishes a full scan in under 2 s, before and after times in the summary. The checkpoint carries `Change-Review: cargo-handler shows every session's details within two seconds of starting | start cargo-handler on natedev and watch the worker rows fill in`.

### Phase 8 — Reviewers and Claude dispatch seats  · status: todo

**Depends on:** Phase 7

#### Work Order

**Goal:** a pool reviewer shows under its unit while it runs a review, an unnamed Claude plan-mode reviewer shows role `review`, and a dispatch to a Claude-backed seat folds into its row like a Codex seat does.

**Source:** showrunner-approved add-ons, 2026-10-16. Runs after phase 7 (run order since 2026-10-17: 6 live roles, 7 first scan, 8 reviewers, 9 clicks); it uses phase 5's prompt-opening parser for reviewer jobs. Pool reviewers moved here from phase 4 on 2026-10-16 so all reviewer rows land together; the producer's reviewer edges (enh-showrunner checkpoint 3b2d8848 in `~/.claude`) must be merged and installed before the live check.

**Constraints from prior phases:**
- Phase 4's `AgentActivity` and `VerificationProgress` (`census/activity.rs`) own every drawn status word, its color and Status sorting; `RunRecord::Ignored` drops non-verify mesh_runs records at the read (implementation-only). Keep `FirstTaskPrompt{Unread,Recorded}`, `ThreadWorktree{Unrecorded,Recorded}`, `WorkerTask::RoleOnly`, the incremental transcript/rollout cursors (`TranscriptRead`, `RolloutRead::read_to`) and the role-only fallback; any new per-scan file read is incremental the same way.
- `census/hierarchy.rs` (≈190) reads `production_tree.py --json` edges and drops reviewer `state`; its cache reuses edges for 30 s while the Claude session ids are unchanged, so a reviewer's busy state can go stale while its app-server thread lives.
- Dispatch resolution carries a Codex `thread_id` and folds the launcher shell's children into the thread's children; it cannot name a Claude session row today. `ParsedDispatch` drops the launcher's `--phase` argument.
- Phase 6 publishes the hierarchy as `HierarchySnapshot{edges, notice}` (`HierarchyNotice{Clear, Failed(HierarchyError), NoSessionMatched}`) inside `CensusUpdate::Local{rows, hierarchy, stage}` and `MachineState::Answered(rows, HierarchySnapshot)`, discovered at `ScanStage::Classified`, once per scan. The probe sends the legacy `hierarchy_failed` flag beside `hierarchy_notice` for older receivers. Reviewer metadata crossing that boundary keeps the existing note, retention, early-role and older-receiver tests passing.
- Live checks run under the real config home: a scratch `XDG_CONFIG_HOME` drops every showrunner-to-unit and reviewer edge, so a capture prefixes `hierarchy_command` with `XDG_CONFIG_HOME=/home/natepiano/.config`.
- The `["claude", "--print", "review"]` value in `census/tree.rs` is test fixture data, not a production rule. Today an unlisted Claude descendant can be treated as a carrier process rather than a visible row.

**Spec:**
1. A `claude --print --permission-mode plan` process with no other role mapping shows role `review`. Trace the real discovery-to-row path (`census/classify.rs`, `census/tree.rs`, `census/placement.rs`) and make such a process a visible row; the `review` fallback is typed and never overrides a role that already comes from the hierarchy edge or a roster. Phase 3 groups top-level `RoleLabel::Unlabeled` roots into the unassigned tile and draws their role `—` (`agent_cell.rs` ≈460, ≈758): a top-level reviewer with no production role still joins that tile, but draws `review`, so the fallback is its own `RoleLabel` variant (or carried beside it) that grouping places with unlabeled roots and the role cell names.
2. The dispatch fold resolves a Claude-backed seat through `<session dir>/impl_worker_<slot>.json` (written by `implement.sh` ≈230 at the session dir root, with no phase field) (family `claude` → `worker_id` is the session id), joining the dispatch to that Claude session's row the way phase 1 joins a Codex seat through `mesh_roster.json`. Model the joined worker as a family-aware identity, e.g. `enum WorkerIdentity { CodexThread(ThreadId), ClaudeSession(SessionId) }`, so identical id text from the two families never matches. The file is rewritten at each launch and holds only `name`, `slot`, `family`, `model` and `worker_id`, with no pid; `implement.sh` records the launcher's claim pid separately in `impl_claim_<slot>.json`. Join the identity only when it belongs to the launch that the live claim describes: correlate the two files of the same launch explicitly (by their recorded launch, never by "some launcher for this slot is alive"), so an identity file left by an earlier launch joins nothing even while a new launcher holds the same slot. Showrunner ruling 2026-10-17: `implement.sh` keeps `impl_worker_<slot>.json` as the worker's durable identity across follow-ups, and its claims carry `pid`, `worker` and `turn_id` with no launch id, so no same-launch claim is made: a seat row matches its worker by worker name plus a live claim pid, inside cargo-handler only, and an identity whose claim pid is not live joins nothing. Read both files' real shapes from a live run (this session dir has `impl_worker_impl.json` and `impl_claim_impl.json`) before writing the parser, and record them in the summary.
3. Pool reviewers (showrunner add-on, 2026-10-16, from the user's comparison: hana has 21 permanent pool reviewers and its tile shows none; revised the same day; the user: "i don't need to see them if they are not active - but if they are ruynning i expect to see them"). A pool reviewer is a leased Codex thread stored on disk, resumed for each review; `busy_pid` is the process running that review. cargo-handler shows running agents, so a stored thread is not a row. Source: `~/.local/state/review-pool/<production slug>/pool.json`, `reviewers.<name>` with `type`, `thread_id`, `lease.unit`, `busy_pid`; edges run from the leased unit to the reviewer with role `review`, emitted by the producer (`production_tree.py`, natedev's ruling 2026-10-16; enh-showrunner checkpoint 3b2d8848). `production_tree.py --json` (schema 1) adds an edge per leased reviewer: `manager` (director's Claude session id), `member` = `thread_id` (reviewer thread), `role` `review`, `unit` (lease unit), `state` `busy` while busy_pid runs else `idle`, `type` `code`|`design`; templates, unleased reviewers and directors on another machine are left out. Draw only `state: busy` edges. The user, later the same day: "i don't need to see pool reviewers that are idle … we should just only be showing active ssessions and certainly flag any that happens to have a reviewr role AS a reviewer when it's reviewing": the tile adds no row for a reviewer that is not running, and a running session that holds a reviewer lease draws role `review`, whichever row it would otherwise have. A unit can hold several reviewers (the pool forks one when all are busy); each busy one is its own row.
   - Liveness: carry each reviewer edge's `state` through `census/hierarchy.rs` as a type, and never reuse a cached reviewer state past the pool's own change: key the reviewer part of the cache on `pool.json`'s modification time (busy_pid is written and cleared there), so busy → idle → busy shows with an unchanged director and a surviving app server. `production_tree.py` derives `state` from whether `busy_pid` runs now, so a reviewer process can exit with no pool-file rewrite: re-check each busy reviewer's `busy_pid` liveness every scan as well, including while phase 6 retains hierarchy links after a failed discovery.
   - Name and job: the reviewer name is the `pool.json` `reviewers.<name>` whose `thread_id` matches the edge's `member`; the job is the current review's task (the rollout's latest task prompt through phase 5's parser), not `FirstTaskPrompt`, so a reviewer taking a second job drops the first job's label.
   - The current review's task is read incrementally: `RolloutRead` (`census/tier.rs`) keeps only `FirstTaskPrompt` today, and `JobText`/`JobSource` are private to `worker_dispatch/mod.rs`. Add a semantic current-review prompt state beside the preserved first prompt, read by the same cursor, and expose the parser to it inside the crate.
   - A reviewer draws only while `busy_pid` names a live process: a `review` row under its unit, named `<reviewer> · <job>` the way phase 4 names worker rows, with its `AgentActivity` status word. A leased reviewer that is not busy, and every `templates` entry, draws nothing.
   - Its job comes from the same typed prompt opening phase 5 built (`JobText::parse` in `census/worker_dispatch/mod.rs`). Pool review prompts open `You are a pooled reviewer for <unit>, phase N…`, which phase 5 parses as a plain first line, so that sentence would draw as the job. Recognize this opening as a role preamble in the same parser (showrunner, 2026-10-16), naming the job from the prompt's own job text, or role-only when it has none; no per-caller special case. A test reads the drawn row back for a real pool-review opening taken from this run's prompts.
4. Tests, with fixture files: a reviewer started with `--print --permission-mode plan` reaches the drawn Role column as `review` (rendered through grouped tiles, `agent_cell::group_tiles`, not only `cell_order_placed`, including a top-level reviewer inside the unassigned tile); a hierarchy-given role still wins; a Claude-seat dispatch folds into its session's row; an identity file whose claim pid is not live joins nothing; a busy reviewer whose process exits with no pool-file rewrite stops drawing on the next scan; a Codex thread id and a Claude session id with the same text stay apart; from a fixture `production_tree.py --json` payload, a busy reviewer edge drawn as a `review` row under its unit, and an idle edge drawn as nothing; busy → idle → busy with an unchanged director; one reviewer taking two jobs in turn, drawn with the second job.

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs`, `crates/cargo-handler/src/census/schedule.rs`, `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/census/tier.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files, migrations of existing tests outside the tester's files, `mod` lines.
- `test` — `worker_dispatch/tests.rs`, the incremental two-job reviewer rendering regression, and new reviewer hierarchy/liveness, Claude-identity and grouped-rendering test modules with their fixtures.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live tile shows a plan-mode reviewer as `review` and a busy pool reviewer under its unit. The checkpoint carries `Change-Review: Claude plan-mode reviewers show role review; dispatches to Claude seats fold into their row | open a tile running a Claude reviewer or a Claude-seat dispatch` and `Change-Review: a pool reviewer shows under its unit as a review row while it runs a review, and not otherwise | look under a hana unit while its code review runs`.

### Phase 9 — Accurate clicks during motion  · status: todo

**Depends on:** Phase 5

**Blocked by:** tile-fixes phase 20 merged on main — the showrunner says when.

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2763`) while drawing interpolates (`drawing_at`, `grid.rs` ≈2880), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps. Since phase 6, `summary::machine_height` counts hierarchy notice lines, so a first-machine notice appearing or clearing moves later machines' clickable headers; add a two-machine draw → notice change → click regression.

**Constraints from prior phases:**
- Phase 4's narrow headings fit through `HeadingLocation::{Shown,Cut,Omitted}` (`agent_cell.rs`); test clicks against the fitted heading actually drawn, not the full one.
- After phase 5 the summary's demand changes with each accounts reading.
- Phase 2's cell-local click hook (`cell_content_at`, `tiles/host.rs`) resolves content from current slots and the destination layout through a crate-private settled-geometry helper in `tiles/grid.rs`. This phase moves it to the last-drawn record too, so the header hit-test matches what is on screen.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Phase 3's tile identity is `tiles::TileKey::{Agent(AgentCell), Unassigned { machine }}` (sorts in `App::tree_sorts: HashMap<TileKey, TreeSort>`, pruned by `agent_cell::tile_keys`), and header layout carries `agent_cell::SessionHeaderLead::{OverBranch, Flush}` (the pool's session header sits over flush names). Drawn header spans are kept per `TileKey`, with the lead they were drawn with (`render.rs` ≈229 `Cells::tile`/`header_spans`, `agent_cell.rs` ≈1384).
- Before writing the Work Order's prompt, refresh the `grid.rs`/`draw.rs` line refs above against main after tile-fixes phase 20 merges.

**Spec:**
1. `draw_tile_grid` (`tiles/draw.rs`) records, for each drawn piece, the cell's stable content identity, its drawn rect, and its visible (clipped) hit region, in the grid's own state, replacing the previous frame's record. Where pieces overlap, the one drawn on top wins. Model the record's state as a type that tells "never drawn" apart from "a frame drew no interactive content" (e.g. `enum DrawnHits { NotDrawn, Drawn(Vec<DrawnPiece>) }`), never a bare `Option`.
2. `TileGrid::cell_at(&self, Position) -> Option<usize>` keeps its public signature, as the compatibility boundary, and answers from that record. Before the first draw it answers from the layout as today.
3. Phase 2's content lookup in `tiles/host.rs` uses the same record, and resolves the content by the identity recorded at draw time, not by the slot's current occupant.
4. cargo-handler keeps the header spans it drew with each tile and the summary, and `render::cell_clicked` maps a header click through those drawn spans, not freshly rebuilt ones. A click on a drawn tile that a later scan removed (its sort already pruned) records no preference, so it cannot bring a pruned pool sort back. `tiles/host.rs` ≈137 calls `focus_cell(index)` on a current slot: resolve the drawn identity to its current slot at the public `Option<usize>` boundary, with a retired-content outcome inside, so a click on a vanished drawn tile never focuses its replacement. If `Cells::tile` (`render.rs` ≈224) stays, it returns a semantic lookup result rather than an `Option`, converting to `Option` only at the framework boundary.
5. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell; a press on the destination rect of a still-moving cell returns what is drawn there; a press on a piece's clipped-away part misses it; a press after a frame that drew nothing interactive returns no cell. In cargo-handler, a census change between the draw and the click that would otherwise shift a header column still selects the column that was drawn under the pointer, for a tree tile and the unassigned tile, in Full and Compressed views; a click on a drawn tile that has since disappeared records no sort and focuses no replacement; an accounts reading that changes the summary's demand between draw and click still selects the drawn agents column, and an accounts-header click stays inert. cargo-tile's suites pass.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/tests/`
- `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 2 writers.
- `grid` — tui_pane: the drawn-hit record in `draw.rs`/`grid.rs`/`host.rs`, its motion tests and `host_tests.rs`, tui_pane CHANGELOG.
- `app` — cargo-handler: drawn header spans kept with the frame, `render::cell_clicked`, their tests, cargo-handler CHANGELOG.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.
