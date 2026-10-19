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

**Depends on:** Phase 5

**Blocked by:** tile-fixes phase 20 merged on main — the showrunner says when.

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2894`) while drawing interpolates (`drawing_at`, `grid.rs` ≈3019), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps. Since phase 6, `summary::machine_height` counts hierarchy notice lines, so a first-machine notice appearing or clearing moves later machines' clickable headers; add a two-machine draw → notice change → click regression.

**Constraints from prior phases:**
- Phase 4's narrow headings fit through `HeadingLocation::{Shown,Cut,Omitted}` (`agent_cell.rs`); test clicks against the fitted heading actually drawn, not the full one.
- After phase 5 the summary's demand changes with each accounts reading.
- Phase 2's cell-local click hook (`cell_content_at`, `tiles/host.rs`) resolves content from current slots and the destination layout through a crate-private settled-geometry helper in `tiles/grid.rs`. This phase moves it to the last-drawn record too, so the header hit-test matches what is on screen.
- Phase 2's header click in cargo-handler (`render::cell_clicked`) rebuilds header spans from the current census and clock, so a changed name, activity, age width or Full/Compressed choice can move columns after the frame was drawn. The drawn header mapping must be kept with the frame.
- Phase 2's attract-screen exclusion (clicks ignored while the attract screen replaces the grid) and its regression stay as they are.
- Phase 3's tile identity is `tiles::TileKey::{Agent(AgentCell), Unassigned { machine }}` (sorts in `App::tree_sorts: HashMap<TileKey, TreeSort>`, pruned by `agent_cell::tile_keys`), and header layout carries `agent_cell::SessionHeaderLead::{OverBranch, Flush}` (the pool's session header sits over flush names). Drawn header spans are kept per `TileKey`, with the lead they were drawn with (`render.rs` ≈243 `Cells::tile`, ≈248 `header_spans`, `agent_cell.rs` ≈1605).

**Spec:**
1. `draw_tile_grid` (`tiles/draw.rs`) records, for each drawn piece, the cell's stable content identity, its drawn rect, and its visible (clipped) hit region, in the grid's own state, replacing the previous frame's record. Where pieces overlap, the one drawn on top wins. Model the record's state as a type that tells "never drawn" apart from "a frame drew no interactive content" (e.g. `enum DrawnHits { NotDrawn, Drawn(Vec<DrawnPiece>) }`), never a bare `Option`.
2. `TileGrid::cell_at(&self, Position) -> Option<usize>` keeps its public signature, as the compatibility boundary, and answers from that record. Before the first draw it answers from the layout as today.
3. Phase 2's content lookup in `tiles/host.rs` uses the same record, and resolves the content by the identity recorded at draw time, not by the slot's current occupant.
4. cargo-handler keeps the header spans it drew with each tile and the summary, and `render::cell_clicked` maps a header click through those drawn spans, not freshly rebuilt ones. A click on a drawn tile that a later scan removed (its sort already pruned) records no preference, so it cannot bring a pruned pool sort back. `tiles/host.rs` ≈138 calls `focus_cell(index)` on a current slot: resolve the drawn identity to its current slot at the public `Option<usize>` boundary, with a retired-content outcome inside, so a click on a vanished drawn tile never focuses its replacement. If `Cells::tile` (`render.rs` ≈243) stays, it returns a semantic lookup result rather than an `Option`, converting to `Option` only at the framework boundary.
5. Tests: during motion (a test-only motion policy may hold an animation mid-way), a press on a drawn piece returns that piece's cell; a press on the destination rect of a still-moving cell returns what is drawn there; a press on a piece's clipped-away part misses it; a press after a frame that drew nothing interactive returns no cell. In cargo-handler, a census change between the draw and the click that would otherwise shift a header column still selects the column that was drawn under the pointer, for a tree tile and the unassigned tile, in Full and Compressed views; a click on a drawn tile that has since disappeared records no sort and focuses no replacement; an accounts reading that changes the summary's demand between draw and click still selects the drawn agents column, and an accounts-header click stays inert. cargo-tile's suites pass.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/src/tiles/host_tests.rs`, `crates/tui_pane/CHANGELOG.md`
- `crates/tui_pane/src/tiles/tests/`
- `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/CHANGELOG.md`

**Workers:** 2 writers.
- `grid` — tui_pane: the drawn-hit record in `draw.rs`/`grid.rs`/`host.rs`, its motion tests and `host_tests.rs`, tui_pane CHANGELOG.
- `app` — cargo-handler: drawn header spans kept with the frame, `render::cell_clicked`, their tests, cargo-handler CHANGELOG.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean. A live click during motion lands on the drawn cell in cargo-handler and in cargo-tile. The checkpoint carries `Change-Review: clicks land on the tile drawn under the pointer, even while tiles move | add or remove a tile and click a moving one at once, in cargo-handler and cargo-tile`.

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

#### Work Order

**Goal:** a hierarchy command that runs after the first answer never holds the scheduler thread or the publication lock, so remote probes and local publication continue while it runs.

**Source:** phase 10 closure review, 2026-10-19 (finding F008, minor, and the open doc finding F007). The ledger closed phase 10 with both open; this phase repairs them next, before any other phase.

**Constraints from prior phases:**
- Phase 10 runs the local scan on its own worker thread, shares `LocalPublications` through `Arc<Mutex<_>>` (`census/schedule.rs`), and delivers the first hierarchy answer through `ScheduleEvent::HierarchyReady` with `finish_first(Duration::ZERO)`. The first-answer wait has a one-second budget (`HIERARCHY_FIRST_PUBLISH_BUDGET`, `constants.rs`).
- Showrunner rules from phase 10 still hold: one refresh per new unnamed session id, then the id is recorded as checked, with no timer; no session is hidden by its name.

**Spec:**
1. No hierarchy command runs on the scheduler thread or under the publication lock, on any path: the catch-up in `finish_first` when `HierarchyReady` finds a session that arrived while the first answer was pending (`schedule.rs` ≈413), and every `cache.snapshot` call in `LocalPublications::discover_at` (new ids, cache expiry, retries). Each refresh is a typed pending state with its own receiver, delivered by `HierarchyReady`. Rows that wait on a refresh stay held only until its answer arrives. Showrunner ruling, 2026-10-19 (architect finding 1).
2. `LocalAnswers.publications` (`schedule.rs` ≈207) gets a doc comment that states why publication ownership is shared.
3. A refresh answer is accepted with refresh semantics, distinct from the initial answer: `HierarchyCache::first_outcome` resets `CheckedUnlinkedSessions`, but a refresh keeps the ids already checked. Name the request types for those two roles.
4. A failed follow-up command releases the held rows with the retained links and the failure notice. A change of the hierarchy source, or a disabled source, while a follow-up is pending prevents its late answer from restoring old links.
5. Tests, with the drawn rows read back as the phase 10 restart tests do: the follow-up command blocks; remote probes and local publication of other rows continue while it is blocked; the newcomer draws nested under its showrunner once the answer is released. Chained catch-up answers give an already-checked unnamed id no extra refresh (exact command counts). A failed follow-up releases rows with the notice; a source change during a pending follow-up keeps old links out.

**Files:**
- `crates/cargo-handler/src/census/schedule.rs`, `crates/cargo-handler/src/census/hierarchy.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/constants.rs`
- `crates/cargo-handler/src/census/catch_up_hierarchy_tests.rs`, `crates/cargo-handler/src/census/fixtures/`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — scheduler and cache production changes, test hooks, `mod` lines, the doc comment, CHANGELOG.
- `test` — a new catch-up scheduler test module (`census/catch_up_hierarchy_tests.rs`) and its fixtures, including remote progress and rendered placement.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. No visible change, so no `Change-Review:` line.

### Phase 12 — Accounts table: reset gradient and header sort  · status: todo

**Depends on:** Phase 9

#### Work Order

**Goal:** the summary's accounts table colors each reset time from green (nearest) to red (furthest), and a click on an accounts-table header sorts the table by that column.

**Source:** the user, 2026-10-18: "i want the resets in the summary to show a green to red gradient - green for the ones that are nearest and red for the ones that are furthest away - also the columns in the accounts table in the summary should also sort when i click on them". Showrunner ruling 2026-10-18: interpolate between the theme's success and error colors by the rank of the reset time; nearest is success, furthest is error; equal reset times get the same color; one account uses success. Header sort reuses the phase 2 agent-table sort code, not a second sort path.

**Constraints from prior phases:**
- Phase 5 built the accounts table (`summary/accounts.rs`; `resets_at` ≈173, rounded by `summary/constants.rs`). An account with no reset time keeps its present text and takes no gradient color.
- Phase 2's sort lives in `crates/cargo-handler/src/sort.rs` (`TreeSort`) and `summary` (`SummarySort`, `summary/sort_tests.rs`); a header click reaches it through `render::cell_clicked`. Phase 9 makes a click resolve against the header spans that were drawn, and its regression says "an accounts-header click stays inert": this phase replaces that inert rule with a sort, through the same drawn-span mapping.
- Colors come from `SemanticTheme` (`theme.rs`): `success` and `error`. Interpolate in the theme's own color space; never write a color literal.

**Spec:**
1. Reset gradient: rank the accounts that have a reset time by that time (equal times share a rank). Rank 0 draws `success`, the last rank draws `error`, and ranks between are interpolated evenly. One ranked account draws `success`.
2. Header sort: each accounts-table column header is a sort target. A click sorts by it through phase 2's sort code (the shared `ColumnSort` direction toggle and tie rule), kept per summary in app state; no second sort path. `SummaryColumn` names agent columns and `ColumnSort::default()` always selects a column, so add an accounts column enum and a semantic state `SourceOrder | Sorted(<accounts column sort>)`, independent of the agent sort. Ordering: percentages and credits numeric, resets chronological, unavailable and unlimited values in a fixed place after real values, ties by source index. The default order stays the source order until a header is clicked. Architect finding 5, 2026-10-19.
3. Tests, rendered through the summary draw and read back from the buffer: three accounts with different reset times draw success, the midpoint and error in time order; two equal reset times draw the same color; one account draws success; an account with no reset time draws its plain text; a click on each accounts header sorts by that column and a second click reverses it, through the drawn header spans.

**Files:**
- `crates/cargo-handler/src/summary/accounts.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/summary/`, `crates/cargo-handler/src/sort.rs`, `crates/cargo-handler/src/theme.rs`
- `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/app.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — gradient, sort wiring, app state, CHANGELOG.
- `test` — accounts gradient and header-sort rendering tests in `summary/`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live summary shows the gradient and sorts on a header click. The checkpoint carries `Change-Review: account resets shade from green (soonest) to red (latest), and accounts headers sort on click | open the summary and click each accounts header`.
