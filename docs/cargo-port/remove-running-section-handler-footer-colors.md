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

**Depends on:** Phase 4


#### Work Order

**Goal:** the summary cell shows, below its top-level agents table, an `accounts` table with each account's role and remaining weekly usage, updating as tokens are used.

**Source:** the user (via the showrunner), 2026-10-16: "cargo handler should reprsent the output of /showrunner:accounts - the summary cell could maybe be subdivided to show a table of top level agents followed by a table of accounts (that updates itself as tokens are used)". Showrunner design and packaging ruling, same day (below).

**Constraints from prior phases:**
- Phase 4's `AgentActivity` and `VerificationProgress` (`census/activity.rs`) own every drawn status word, its color and Status sorting; `RunRecord::Ignored` drops non-verify mesh_runs records at the read (implementation-only). Keep `FirstTaskPrompt{Unread,Recorded}`, `ThreadWorktree{Unrecorded,Recorded}`, `WorkerTask::RoleOnly`, the incremental transcript/rollout cursors (`TranscriptRead`, `RolloutRead::read_to`) and the role-only fallback; any new per-scan file read is incremental the same way.
- Phase 1's account footer colors (`footer_colors/`) color account labels; reuse them.
- Phase 2's summary header style, gap rule and `ColumnSort` header layout; the agents table stays as it is.
- The configured-command pattern of `[census] hierarchy_command` (run off the frame thread, JSON on stdout) is the model for `accounts_command`.

**Spec:**
1. The summary cell holds two stacked tables under its one title: the agents table as today, then an `accounts` table with the same header style and gap rule.
2. Columns: `account` (label such as `claude1`, `codex2`, in its footer color); `role` (the JSON's `roles` list joined with `, ` — one account can hold both groups; an unselected Claude account has `[]`, a Codex account `["workers"]` or `["spare"]`; `—` when empty); `weekly left` (percent, red at 10% or less); `resets` (the JSON's ISO 8601 UTC time drawn short in the local zone, e.g. `Mon 02:00`); `credits` (a number, `unlimited`, or JSON `null` drawn `—`, as for Claude; dimmed when `use_credits` is false, as it always is for Claude). No folder, login, on-credits or read columns, so no email address is drawn; a row whose reading is over an hour old is dimmed and shows its age.
3. Rows follow accounts.py's own order and signed-in rule; its problem lines draw dimmed under the table.
4. One source of truth: a configured `[census] accounts_command` printing accounts.py's own JSON; never re-derive the role map or the usage readings in Rust. Parse the JSON into typed rows at the boundary (`roles` into a list of a role enum, `credits` into an enum of amount, unlimited and none, and each nullable reading — `weekly_left_percent`, `resets_at`, `read_at` — into a known/unavailable enum). An unavailable reading draws `—` in its cell; a row with no `read_at` draws dimmed with no age. The command (promoted to `~/.claude` on both machines at 49fde9b3) is `~/.claude/scripts/lib/py ~/.claude/scripts/accounts/accounts.py status --json [--stored]`; set `accounts_command` to the `--stored` form (no live reads, no subprocess, no ssh; 0.21 s on the Mac). Output: `{"accounts": [...], "problems": [string]}`, accounts in the table's order, `problems` the table's note lines. Keys per account: `label` (string); `tool` (`"claude"`|`"codex"`); `roles` (list: Claude `"units"`/`"showrunners"` or `[]`; Codex `["workers"]` or `["spare"]`); `weekly_left_percent` (number|null); `resets_at` (ISO 8601 UTC|null); `credits` (number|`"unlimited"`|null, null for Claude); `use_credits` (bool, false for Claude); `read_at` (ISO 8601 UTC|null). No email, folder or path. With no cached role map, `--stored` prints empty roles, puts the problem in `problems` and exits 0; plain `status --json` exits 1 with the reason on stderr when no role map can be read.
5. Optional (showrunner ruling): with no `accounts_command` configured the summary shows only the agents table; when the command fails it also shows one dimmed line saying why. cargo-handler works fully without it. Refresh by running the command off the frame thread every 60 s; that run is the only change detection, and a changed reading shows on the next frame after it returns. Never block a frame. Model what the summary has as one state, e.g. `enum AccountsTable { NotConfigured, AwaitingFirstReading, Ready(AccountsReading), Failed(AccountsFailure), FailedKeepingLast { last: AccountsReading, failure: AccountsFailure } }`, never optional rows beside an optional error: before the first result the summary shows the agents table only; a failure after a good reading keeps the last rows (stale ones dimmed with their age, as in Spec 2) and adds the one dimmed why-line. Set the command in both machines' configs.
6. `summary::height` and `summary::width` (`summary/mod.rs` ≈66, ≈83) measure only machine groups today and `render.rs` ≈264 hands them to the grid as the cell's demand: include the accounts heading, rows, problem lines and the why-line in both. A click on the accounts table's header changes no sort: neither the agents table's nor accounts.py's row order.
7. Worker job names (showrunner add-on, 2026-10-16, from phase 4's live shots): a job is sometimes the dispatch prompt's opening preamble, not the job — seen `fix · You are slot fix, opening as fix for phase 20's repair r…` (tile-fixes) and `review · You are slot review, now working as a reviewer. E…` (herdr-trial). The job-name derivation in `census/worker_dispatch/mod.rs` (`first_task_line` ≈371, used by `WorkerTask::from_prompt`, `task_label` and the resolved dispatch label) takes the first nonempty prompt line. Parse the opening into a type instead — e.g. `enum PromptOpening { Heading(JobText), RolePreamble { job: JobText }, Plain(JobText) }` — so the slot preamble (`You are slot <slot>, now working as|opening as <role> …`, and the `board.sh role <dir> <slot> <role> "<job>"` first call it names) is never drawn as the job; name the job from the prompt's own job text, e.g. `fix · phase 20 repair round 27`. Read the real opening shapes from live rollouts (`~/.codex/sessions/`) and this run's prompts (`/tmp/claude/unit/*/phases/*/*prompt*.md`) before writing the parser, and record each shape in the summary. A preamble with no usable job text is its own outcome that becomes `WorkerTask::RoleOnly` (the row draws the role alone). One extraction serves the role fallback label, the roster-backed label and the live dispatch label. No per-caller string special case.
8. Tests: a real grid-buffer read-back of both tables at their measured demand, column colors (red at ≤10%, dimmed credits, dimmed stale row with its age), `—` cells, problem lines, row order from the JSON, a refresh through the background scheduler that changes a reading, a command that fails or prints bad JSON leaving the agents table intact, every `AccountsTable` state drawn (including a failure after a good reading keeping the last rows), and an accounts-header click leaving both orders unchanged; each prompt opening shape Spec 7 found named by its job, read back from a rendered worker row.
9. A config rewrite keeps what it does not know (showrunner add-on, current, 2026-10-16: a wrong result). `tui_pane::LoadedConfig` (`crates/tui_pane/src/app_config/loaded.rs`) restates on load and saves after a settings edit by writing `toml::to_string_pretty(&config)` over the whole file, so every key the running binary's struct lacks and every comment is erased: phase 4's installed cargo-handler removed this phase's `accounts_command` from natedev's `config.toml` within seconds of it being added, and a user's hand edits are at the same risk. Write instead by merging the serialized config into the file's existing document with `toml_edit` (workspace dependency `toml_edit = "0.25"`): set each known key's value, add missing keys and sections, keep unknown keys, comments, ordering and formatting untouched, and write nothing when the merged text equals the file. A file that fails to parse is still left alone. The fix lives in `tui_pane`, so cargo-tile and cargo-port get it too.
10. Tests (in `tui_pane`): an unknown top-level key, an unknown key inside a known section, and a comment each survive a restate and a save; a known value changed by a save is written; a file that already says everything is not rewritten (unchanged modification time or bytes); a missing section is added.

**Files:**
- `crates/cargo-handler/src/summary/`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/terminal.rs`, `crates/cargo-handler/src/config.rs`, `crates/cargo-handler/src/census/`
- `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`
- `crates/tui_pane/src/app_config/loaded.rs`, `crates/tui_pane/Cargo.toml`, `crates/tui_pane/CHANGELOG.md` (Spec 9; `loaded.rs` is outside this unit's tui_pane paths, so it goes through berth and the notice names it `also touches`)

**Workers:** 1 writer + 1 tester.
- `impl` — production files, edits to existing tests, `mod` lines.
- `test` — new accounts-table, refresh and prompt-naming rendering test modules.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live shot shows the accounts table under the agents table. The checkpoint carries `Change-Review: the summary shows an accounts table that updates as usage changes | watch weekly left fall on a busy account` and `Change-Review: worker rows name the job, never the dispatch prompt's slot preamble | look under tile-fixes and herdr-trial for fix and review rows` and `Change-Review: a config rewrite keeps keys and comments it does not know | add a comment and an unknown key to cargo-handler's config.toml, change a setting in the overlay, and see both still there`. Spec 9 also runs `verify.sh test tui_pane`, `test cargo-tile` and `test cargo-port` (the production's merge tests for a `crates/tui_pane` change).

### Phase 6 — Reviewers and Claude dispatch seats  · status: todo

**Depends on:** Phase 5

#### Work Order

**Goal:** a pool reviewer shows under its unit while it runs a review, an unnamed Claude plan-mode reviewer shows role `review`, and a dispatch to a Claude-backed seat folds into its row like a Codex seat does.

**Source:** showrunner-approved add-ons, 2026-10-16. Runs after phase 5 (showrunner run order 2026-10-16: 5 accounts, 6 reviewers, 7 clicks); it uses phase 5's prompt-opening parser for reviewer jobs. Pool reviewers moved here from phase 4 on 2026-10-16 so all reviewer rows land together; the producer's reviewer edges (enh-showrunner checkpoint 3b2d8848 in `~/.claude`) must be merged and installed before the live check.

**Constraints from prior phases:**
- Phase 4's `AgentActivity` and `VerificationProgress` (`census/activity.rs`) own every drawn status word, its color and Status sorting; `RunRecord::Ignored` drops non-verify mesh_runs records at the read (implementation-only). Keep `FirstTaskPrompt{Unread,Recorded}`, `ThreadWorktree{Unrecorded,Recorded}`, `WorkerTask::RoleOnly`, the incremental transcript/rollout cursors (`TranscriptRead`, `RolloutRead::read_to`) and the role-only fallback; any new per-scan file read is incremental the same way.
- `census/hierarchy.rs` (≈190) reads `production_tree.py --json` edges and drops reviewer `state`; its cache reuses edges for 30 s while the Claude session ids are unchanged, so a reviewer's busy state can go stale while its app-server thread lives.
- Dispatch resolution carries a Codex `thread_id` and folds the launcher shell's children into the thread's children; it cannot name a Claude session row today. `ParsedDispatch` drops the launcher's `--phase` argument.
- The `["claude", "--print", "review"]` value in `census/tree.rs` is test fixture data, not a production rule. Today an unlisted Claude descendant can be treated as a carrier process rather than a visible row.

**Spec:**
1. A `claude --print --permission-mode plan` process with no other role mapping shows role `review`. Trace the real discovery-to-row path (`census/classify.rs`, `census/tree.rs`, `census/placement.rs`) and make such a process a visible row; the `review` fallback is typed and never overrides a role that already comes from the hierarchy edge or a roster. Phase 3 groups top-level `RoleLabel::Unlabeled` roots into the unassigned tile and draws their role `—` (`agent_cell.rs` ≈460, ≈758): a top-level reviewer with no production role still joins that tile, but draws `review`, so the fallback is its own `RoleLabel` variant (or carried beside it) that grouping places with unlabeled roots and the role cell names.
2. The dispatch fold resolves a Claude-backed seat through `<session dir>/impl_worker_<slot>.json` (written by `implement.sh` ≈230 at the session dir root, with no phase field) (family `claude` → `worker_id` is the session id), joining the dispatch to that Claude session's row the way phase 1 joins a Codex seat through `mesh_roster.json`. Model the joined worker as a family-aware identity, e.g. `enum WorkerIdentity { CodexThread(ThreadId), ClaudeSession(SessionId) }`, so identical id text from the two families never matches. The file is rewritten at each launch: join it only while the dispatch it describes is live (its launcher pid running), so a file left by an earlier launch joins nothing. Read the file's real shape from a live run (this session dir has `impl_worker_impl.json`) before writing the parser, and record it in the summary.
3. Pool reviewers (showrunner add-on, 2026-10-16, from the user's comparison: hana has 21 permanent pool reviewers and its tile shows none; revised the same day; the user: "i don't need to see them if they are not active - but if they are ruynning i expect to see them"). A pool reviewer is a leased Codex thread stored on disk, resumed for each review; `busy_pid` is the process running that review. cargo-handler shows running agents, so a stored thread is not a row. Source: `~/.local/state/review-pool/<production slug>/pool.json`, `reviewers.<name>` with `type`, `thread_id`, `lease.unit`, `busy_pid`; edges run from the leased unit to the reviewer with role `review`, emitted by the producer (`production_tree.py`, natedev's ruling 2026-10-16; enh-showrunner checkpoint 3b2d8848). `production_tree.py --json` (schema 1) adds an edge per leased reviewer: `manager` (director's Claude session id), `member` = `thread_id` (reviewer thread), `role` `review`, `unit` (lease unit), `state` `busy` while busy_pid runs else `idle`, `type` `code`|`design`; templates, unleased reviewers and directors on another machine are left out. Draw only `state: busy` edges. The user, later the same day: "i don't need to see pool reviewers that are idle … we should just only be showing active ssessions and certainly flag any that happens to have a reviewr role AS a reviewer when it's reviewing": the tile adds no row for a reviewer that is not running, and a running session that holds a reviewer lease draws role `review`, whichever row it would otherwise have. A unit can hold several reviewers (the pool forks one when all are busy); each busy one is its own row.
   - Liveness: carry each reviewer edge's `state` through `census/hierarchy.rs` as a type, and never reuse a cached reviewer state past the pool's own change: key the reviewer part of the cache on `pool.json`'s modification time (busy_pid is written and cleared there), so busy → idle → busy shows with an unchanged director and a surviving app server.
   - Name and job: the reviewer name is the `pool.json` `reviewers.<name>` whose `thread_id` matches the edge's `member`; the job is the current review's task (the rollout's latest task prompt through phase 5's parser), not `FirstTaskPrompt`, so a reviewer taking a second job drops the first job's label.
   - A reviewer draws only while `busy_pid` names a live process: a `review` row under its unit, named `<reviewer> · <job>` by Spec 1, with its Spec 2 status word. A leased reviewer that is not busy, and every `templates` entry, draws nothing.
   - Its job comes from the same typed prompt opening phase 5 built (`JobText::parse` in `census/worker_dispatch/mod.rs`). Pool review prompts open `You are a pooled reviewer for <unit>, phase N…`, which phase 5 parses as a plain first line, so that sentence would draw as the job. Recognize this opening as a role preamble in the same parser (showrunner, 2026-10-16), naming the job from the prompt's own job text, or role-only when it has none; no per-caller special case. A test reads the drawn row back for a real pool-review opening taken from this run's prompts.
4. Tests, with fixture files: a reviewer started with `--print --permission-mode plan` reaches the drawn Role column as `review` (rendered through grouped tiles, `agent_cell::group_tiles`, not only `cell_order_placed`, including a top-level reviewer inside the unassigned tile); a hierarchy-given role still wins; a Claude-seat dispatch folds into its session's row; an identity file left by a finished launch joins nothing; a Codex thread id and a Claude session id with the same text stay apart; from a fixture `production_tree.py --json` payload, a busy reviewer edge drawn as a `review` row under its unit, and an idle edge drawn as nothing; busy → idle → busy with an unchanged director; one reviewer taking two jobs in turn, drawn with the second job.

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs`, `crates/cargo-handler/src/census/schedule.rs`, `crates/cargo-handler/src/census/classify.rs`, `crates/cargo-handler/src/census/tree.rs`, `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/worker_dispatch/tests.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/probe.rs`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/agent_cell/tests/`, `crates/cargo-handler/src/render.rs`
- `crates/cargo-handler/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — production files, migrations of existing tests outside the tester's files, `mod` lines.
- `test` — `worker_dispatch/tests.rs` and new reviewer hierarchy/liveness, Claude-identity and grouped-rendering test modules with their fixtures.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean. A live tile shows a plan-mode reviewer as `review` and a busy pool reviewer under its unit. The checkpoint carries `Change-Review: Claude plan-mode reviewers show role review; dispatches to Claude seats fold into their row | open a tile running a Claude reviewer or a Claude-seat dispatch` and `Change-Review: a pool reviewer shows under its unit as a review row while it runs a review, and not otherwise | look under a hana unit while its code review runs`.

### Phase 7 — Accurate clicks during motion  · status: todo

**Depends on:** Phase 5

**Blocked by:** tile-fixes phase 20 merged on main — the showrunner says when.

#### Work Order

**Goal:** a click hits the cell, and the header column, that is drawn under the pointer, including while tiles are moving after an add, a remove or `+`.

**Source:** the user (via the showrunner), 2026-10-16: "can cargo handler make mouse clicks accurate…". The repro sent to the showrunner found that misses happen only during add/remove/`+` motion. `cell_at` answers against the destination layout (`tui_pane/src/tiles/grid.rs:2763`) while drawing interpolates (`drawing_at`, `grid.rs` ≈2880), and there is a gap between tick, advance and draw. Showrunner ruling: `draw_tile_grid` records the last-drawn piece rects and their cell, and `cell_at` answers from the last drawn frame, which also covers the tick gap. Keep `cell_at`'s public signature. Run cargo-tile's suites. Live-check a click during motion in both apps.

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
