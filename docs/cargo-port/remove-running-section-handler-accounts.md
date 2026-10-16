# handler-accounts

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Each agent tile draws one tree under a header row (`session role status account … age`) that names every session's role and agent account, shells and subagents become tree lines, and the second list below the tree goes away.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, the user (via the showrunner): "yes get the account names correct before the overflow work - so this example - where we show the ascii tree of what is running - is it possible to show column headers for the ascii tree so dashboard unit and idle are all in their own aligned columns with headers?  if so please do so - and add whether their agent account that we just talked about in that list - i still liek the ascii tree showing but putting the column headers in between with column header coloring will make this more clear". Then, after a mockup: "yes build this - and in fact hold on the overflow work - i think that this view by itself is enough for me - so maybe we don't need to have the second list of things that we currently show that seems duplicative".

Approved mockup (showrunner):

```
COMPRESSED VIEW:
  hanadocs · showrunner ──────────────────────────────
     session                    role    status  account  age
  ├─ dashboard                  unit    idle    claude1  2h
  │  └─ hanadocs-dashboard-impl worker  busy    codex1   40m
  └─ diagrams                   unit    idle    claude3  5h

FULL VIEW:
  hanadocs · showrunner ──────────────────────────────
   pid 2185158 · claude2 · idle · 14d · natedev · ups
   main · ~/rust/hana
     session         role    status  account  branch         directory              age
  ├─ dashboard       unit    idle    claude1  enh/dashboard  ~/rust/hana-dashboard  2h
  │  └─ …-impl       worker  busy    codex1   enh/dashboard  ~/rust/hana-dashboard  40m
  └─ diagrams        unit    idle    claude3  enh/diagrams   ~/rust/hana-diagrams   5h
```

## Delegation Context

- **Project:** `cargo-handler` — the terminal agent watcher: it scans this machine and each configured remote (over ssh, `cargo-handler probe`) for Claude Code and Codex agents and draws a summary tile plus one tile per top-level agent through `tui_pane`.
- **Project started:** 2026-10-16T04:41:03.535+00:00
- **Stack:** Rust 2024, ratatui 0.30, sysinfo (process table and environment), serde/serde_json (probe wire), toml (config), cargo-nextest.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Layout:**
  - `crates/cargo-handler/src/census/` — `mod.rs` (domain rows), `account.rs` (new, phase 1), `scan.rs` (local scan), `classify.rs` (process table → rows), `tree.rs` (what each agent runs), `probe.rs` (wire), `placement.rs` (forest, `RoleLabel`), `schedule.rs` (scan thread), `remote.rs` (ssh probe of remotes).
  - `crates/cargo-handler/src/` — `config.rs`, `constants.rs`, `terminal.rs`, `agent_cell.rs` (one tile's content), `render.rs` (`Cells`, the `TileCells` impl), `summary/mod.rs`.
- **Key files** (line refs are this branch at plan time):
  - `crates/cargo-handler/src/census/mod.rs` — `ServiceTier` ≈95 with private `mark(program: &'static str)` ≈121; `HierarchyRole` ≈167 with private `text` ≈184; `CodexThreadId` ≈215 (untagged, `Unrecorded` default, `is_unrecorded` for `skip_serializing_if`); `AgentRow` ≈276 (`#[serde(from = "probe::ProbeAgentRow")]`, field order is the probe's JSON order; `agent_label` ≈328); `ChildKind` ≈337 (`via` ≈359 gives `shell`/`subagent`/`session`/`direct`/`detached`/`thread`; `agent` ≈371); `Timer` ≈396; `ChildRow` ≈407 (optional fields use `#[serde(default)]`, `skip_serializing_if`; `runs_label` ≈441).
  - `crates/cargo-handler/src/census/probe.rs` — `ProbeAgentRow` ≈35 and `From<ProbeAgentRow> for AgentRow` ≈54 (old-probe defaults, e.g. `session_id: Option<String>` → `SessionIdentity`); `print` ≈138 loads config and calls `LocalScanner::new().scan()` ≈144; tests: `child` ≈208, `report` ≈224, known-JSON test ≈324, old-probe tests ≈350–426.
  - `crates/cargo-handler/src/census/scan.rs` — `#[path = "scan/staged_tests.rs"] mod staged_tests;` ≈21; `LocalScanner` ≈83 (`home: Option<PathBuf>` ≈88), `scan_stages_with` ≈180 (classify ≈189, early `Classified` publish ≈193), `new` ≈201, `scan_staged` ≈227, `attach_details` ≈262 (`app_server_threads` call ≈307); `app_server_threads` ≈614 (`ThreadEntry` built ≈666); `process_table` ≈706 (third pass reads `environ` of processes named `codex` ≈727–741; `ProcessEntry` built ≈745; `claude_pid` set ≈756); `claude_pid` ≈767; tests: `scanner()` ≈983, `entry` ≈997.
  - `crates/cargo-handler/src/census/classify.rs` — `ProcessEntry` ≈65 (`claude_pid` ≈82); `agent_rows` ≈182; `claude_row` ≈299, `desktop_app_row` ≈330, `codex_row` ≈353 (each builds an `AgentRow`); `directory_label(directory, home)` ≈540 writes a path under home as `~/…`; test fixtures `process` ≈564, `detailed` ≈578.
  - `crates/cargo-handler/src/census/tree.rs` — `ThreadEntry` ≈62; `TreeSources` ≈74; app-server threads become `Node { kind: ChildKind::Thread, … }` ≈283–299; `flatten` ≈547 turns `Node`s into `ChildRow`s; fixtures `process` ≈596, `app_server` ≈652, `thread` ≈698.
  - `crates/cargo-handler/src/census/placement.rs` — `RoleLabel` ≈65 (`Unlabeled`, `Stated`, `Inferred`; `suffix` ≈77 is `" · <role>"`); `ManagedThread` ≈100 (`owner`, `owner_row`, `child_index`, `manager`, `role`); `AgentPlacement` ≈116 (`rows`, `threads`; `roots` ≈344, `role_label` ≈372).
  - `crates/cargo-handler/src/census/schedule.rs` — `spawn(remotes, hierarchy)` ≈113 builds `LocalScanner::new()` ≈117.
  - `crates/cargo-handler/src/terminal.rs` — `census::schedule::spawn(...)` ≈60; the `#[path]` test-file pattern ≈29–31.
  - `crates/cargo-handler/src/config.rs` — `CensusConfig` ≈113, `Config` ≈122 (every section `#[serde(default)]`); tests: known default file ≈181, `a_complete_config_is_left_alone` ≈207.
  - `crates/cargo-handler/src/constants.rs` — `CHILD_HEADERS` ≈94 and `CHILD_*_COLUMN` ≈95–104, `CHILD_VIA_INDENT` ≈107, `TREE_COLUMNS` ≈110, `TREE_BRANCH`/`TREE_LAST_BRANCH`/`TREE_RAIL`/`TREE_SPACE` ≈113–120, `AGENT_HEADER_GAP_HEIGHT` ≈122, `NOTHING_RUNNING_HEIGHT`/`NOTHING_RUNNING_NOTE` ≈124–127, `SHELL_VIA`…`DETACHED_VIA` ≈135–147, `SUMMARY_HEADERS` ≈162, `TABLE_HEADER_HEIGHT` ≈194, `TABLE_COLUMN_SPACING` ≈200, `HEADING_SEPARATOR` ≈203, `MISSING_VALUE` ≈214, `CLAUDE_DIRNAME` ≈319, `CODEX_DIRNAME` ≈412, `CLAUDE_PID_VARIABLE` ≈462, `HOME_ABBREVIATION` ≈470.
  - `crates/cargo-handler/src/agent_cell.rs` — `AgentEntry` ≈86 (`sessions`, `children`); `SessionEntry` ≈128, `SessionSubject` ≈161, `name_spans` ≈184 (name plus role suffix), `status`/`status_role` ≈200–213, `lead` ≈217; `cell_order_placed` ≈236; `sessions_of` ≈280; `CellView` ≈354 (`at_width` ≈370, `fitted` ≈377); `FlatColumns` ≈390; `height` ≈421, `view_height` ≈430, `children_top` ≈441; `name_style` ≈454; `draw` ≈468, `draw_view` ≈489 (children table ≈515–557); `above_tree` ≈565; `header_line` ≈600; `header_width` ≈642, `header_indented` ≈647; `tree_columns` ≈661, `tree_cells` ≈680, `draw_tree` ≈703, `compressed_session` ≈759; `CellChild` ≈777, `placed_children` ≈790, `nest` ≈801, `children_width` ≈892, `Children` ≈897, `compressed_child` ≈934, `gap` ≈950, `cut_line` ≈960, `child_row` ≈977, `child_name` ≈994, `timer_label` ≈1002, `timer_pie` ≈1012, `time_left_label` ≈1025, `via_text` ≈1038, `depth_indent` ≈1042, `runs_role` ≈1052, `table_columns` ≈1065; `mod tests` ≈1108 (fixtures `agent` ≈1163, `child` ≈1183, `trunk` ≈1201, `entry` ≈1261, `area` ≈1287, `drawn_into` ≈1298, `drawn` ≈1319, `lines` ≈1324, `find` ≈1337, `runs_in` ≈1597, `find_all` ≈1861, `boss_group` ≈1690; `the_height_is_the_rows_drawn_at_every_width` ≈1657).
  - `crates/cargo-handler/src/render.rs` — `draw_panes` ≈86; `Cells` ≈106 and `Cells::new` ≈116; `demands` ≈146 (calls `agent_cell::height` ≈166); `draw` ≈179 (`summary::draw` ≈181, `agent_cell::draw` ≈186); `group_title` ≈196; tests call `agent_cell::height` ≈1030–1035.
  - `crates/cargo-handler/src/summary/mod.rs` — `width` ≈79; `draw` ≈122; `draw_machine` ≈151 (header row in `label_color` ≈189–191); `agent_row` ≈248 (`row.agent_label()`); `column_widths` ≈312 (`AGENT_COLUMN` ≈329); `indented` ≈338, `cell_width` ≈348, `truncated` ≈276; tests call `summary::draw`/`width` ≈895, ≈941.
- **Test lanes:** `cargo-handler`: none under `tests/`; unit-test files under `src/`. A new module file gets a top-level `#[cfg(test)] #[path = "<module>/tests.rs"] mod tests;` (pattern `census/scan.rs` ≈21, `terminal.rs` ≈29). A test file declared inside an existing inline `mod tests` of a non-mod-rs file resolves from `<file stem>/tests/` (`#[path = "x.rs"]` inside `agent_cell.rs`'s `mod tests` names `src/agent_cell/tests/x.rs`) and reads that block's fixtures through `super::`. The workspace denies `self_named_module_files`, so every new submodule of a self-named file takes an explicit `#[path]`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-handler`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Invariants:**
  - Workers run only the Build/Test/Lint lines above; each `verify.sh` run detaches and its result arrives as the worker's next message, so a worker never polls, sleeps or waits on one. Tests run with cargo nextest through `verify.sh`; formatting only with `cargo +nightly fmt`.
  - Invalid states are unrepresentable. The user's rule, 2026-10-15: "this is an invalid state that is representable - you are not properly using the rust type system if you allow an invalid state to be representable - whether through a type state, or an enum, a trait or whatever - this is not something you should fix with c-like procedural code". No bare `Option`/`bool` in new domain types; a wire `Option` converts to an enum at the serde boundary.
  - Nothing on the UI thread runs a command or reads a file. Environment reads and config labels resolve in the scan (scheduler thread or `probe`), never in rendering.
  - Probe compatibility: `PROBE_SCHEMA` stays 3. A probe printed before `account` existed still parses (absent field → `Account::Unrecorded`); an older reader ignores the new field (`ProbeAgentRow` does not deny unknown fields).
  - The summary section stays. The tile title (`group_title`, `<name> · <role>`) is unchanged.
  - No test takes a second.
  - Coordination (production, showrunner ruling 2026-10-16): the overflow plan `docs/cargo-port/remove-running-section-handler-overflow.md` is on hold; this plan runs first. Its later phases re-read `agent_cell.rs` and `render.rs` as this plan leaves them.

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

### Phase 1 — Census account  · status: done

**Depends on:** none

#### As-built

- `crates/cargo-handler/src/census/account.rs` holds `Account { Unrecorded (default), Home(AccountHome) }` on `AgentRow::account` and `ChildRow::account`; its wire form is `null` or `{"home":"~/.claude1","label":"claude1"}` (`serde(from/into = "Option<AccountHome>")`, `Account::is_unrecorded` for `skip_serializing_if`). `AccountHome { home, label }` has private fields and no getters; `AccountLabels::resolve` is its only constructor, so a label always matches its home.
- `ProcessEntry::config_home: ConfigHome { Unread, Default(Program), Named(PathBuf) }`. `Program { Claude, Codex }` (pub(super)) owns its variable (`CLAUDE_CONFIG_DIR_VARIABLE` / `CODEX_HOME_VARIABLE` in `constants.rs`) and default dirname (`CLAUDE_DIRNAME` / `CODEX_DIRNAME`). `config_home(&[OsString], Program)`: an absent, empty or inexactly named variable is `Default`; a non-UTF-8 value stays a path. `config_home_of(&ProcessEntry, &[OsString])` gives non-agent processes `Unread`.
- `process_table` (`scan.rs`) reads `environ` for Codex processes, `claude`-named processes and session-record pids; an empty environment means sysinfo could not read it and stays `Unread`. `claude_pid` is still set for Codex only.
- `AccountLabels::new(&AccountsConfig, Option<&Path>)` normalizes each configured key to the `~` form once (`classify::directory_label`); `resolve(home, machine_home)` returns the `~` form with its configured label, else the last component without one leading dot. `account_of(process, machine_home, labels)`: `Unread` → `Unrecorded`; `Default` with no machine home → `Unrecorded`; otherwise `Home`.
- Rows get their account where they are built: `classify::agent_rows(.., &AccountLabels)` passes labels to `claude_row`, `desktop_app_row` and `codex_row`. `app_server_threads` sets `ThreadEntry::account` from the app server's process, and `tree.rs` carries it through `Node` and `flatten` into the thread's `ChildRow`; every other child is `Unrecorded`.
- `LocalScanner::new(AccountLabels)`, `schedule::spawn(remotes, hierarchy, account_labels)`; `terminal.rs` and `probe::print` build labels from the loaded config, so each machine labels its own homes and a remote's rows arrive labelled.
- `[accounts]` is `AccountsConfig(BTreeMap<String, String>)` (`serde(transparent)`) on `Config` after `census`, keys written with `~`; read at startup, a config edit applies on restart.
- Probe: `ProbeAgentRow::account` is `#[serde(default)]`, `ChildRow::account` is omitted while unrecorded, `PROBE_SCHEMA` stays 3, and an older probe's rows parse to `Unrecorded`.

**Files:**
- `crates/cargo-handler/src/census/account.rs` — `Account`, `AccountHome`, `ConfigHome`, `Program`, `AccountLabels`, `config_home`, `config_home_of`, `account_of`
- `crates/cargo-handler/src/census/account/tests.rs` — environment parsing, label resolution, `account_of`, probe wire round trips
- `crates/cargo-handler/src/census/scan.rs` — environment pass, `LocalScanner::account_labels`, thread accounts
- `crates/cargo-handler/src/census/classify.rs` — `ProcessEntry::config_home`; row builders set `account`
- `crates/cargo-handler/src/census/tree.rs` — `ThreadEntry::account` into thread `ChildRow`s
- `crates/cargo-handler/src/census/mod.rs`, `probe.rs`, `schedule.rs`, `crates/cargo-handler/src/terminal.rs` — the field, the wire, label plumbing
- `crates/cargo-handler/src/config.rs` — `AccountsConfig`, `Config::accounts`, `[accounts]` parse tests
- `crates/cargo-handler/src/constants.rs` — `CLAUDE_CONFIG_DIR_VARIABLE`, `CODEX_HOME_VARIABLE`

**Binds later work:** the Tile tree phase adds `AccountHome::label()` (pub(crate)) and `AccountColumn { Hidden, Shown }` with `AccountColumn::of` and its tests in `census/account/tests.rs`; `Program` is the grouping key for its per-program label count.

**Gotchas:** `account.rs` imports `std::os::unix::ffi::OsStrExt` unguarded (cargo-handler is not built on Windows). An item with no consumer is narrowed by cargo-mend, and agent-written `#[expect(dead_code)]` is not allowed, so API lands with its first caller.

**Ruled out:** `From<&AccountsConfig>` reading `dirs::home_dir()` (a `From` takes no external context; the machine home is a parameter); patching accounts onto classified rows through a second pid table; an `AccountHome::home()` getter (no consumer).

### Phase 2 — Tile tree  · status: done

**Depends on:** Phase 1

#### Work Order

**Goal:** each agent tile draws its tree under a label-colored header row with `session`, `role`, `status`, `account` (when shown), `branch` and `directory` (full view only) and `age` columns; shells, subagents and unheld threads are tree lines; the children table below the tree is gone; when accounts show, the header line and the summary's `agent` column read the account label.

**Spec:**
1. One frame decision. `Cells::new` (`render.rs` ≈116) computes `accounts: AccountColumn` once, from `agent_cell::account_column(&agents)`, which feeds `AccountColumn::of` with every entry's root row and every Agent and thread line in its tree (shell and subagent lines carry no account). `Cells` stores it and passes it to `agent_cell::height`, `agent_cell::draw`, `summary::width` and `summary::draw` as a new last parameter. It counts every cell in the frame, including any the divided grid leaves unplaced.
2. Agent value. `AgentRow::agent_label(&self, accounts: AccountColumn) -> Cow<'static, str>` (`census/mod.rs` ≈328): `Shown` with `Account::Home` → the home's label marked with the tier (`codex2 fast`); otherwise the program as today. `ServiceTier::mark` (≈121) takes `&str` and returns `Cow<'static, str>` (owned when marked or not static). `header_line` (`agent_cell.rs` ≈600), `summary::agent_row` (≈248) and `summary::column_widths` (≈329) call it with the frame's `accounts`.
3. Tree lines. Replace `AgentEntry::sessions` and `AgentEntry::children` with `lines: Vec<TreeLine<'a>>`, top to bottom. A `TreeLine` keeps `machine`, `location`, `row` (the member's row, or the owner row for a thread), `hue`, `rails`, `last` and a subject:
   ```rust
   enum LineSubject<'a> {
       /// A listed agent the cell's agent or one of its members opened or manages.
       Agent(RoleLabel),
       /// A Codex thread a manager holds, with the role its edge states.
       ManagedThread(&'a ChildRow, HierarchyRole),
       /// Work an agent runs that no other line stands for.
       Work(&'a ChildRow),
   }
   ```
   Siblings under one parent are its member lines (as `sessions_of` ≈280 finds them today) and its own work lines, together sorted oldest first by start, then machine and pid. A member's work comes from its own `row.children`: `Shell`, `Subagent`, and `Thread` rows that no `ManagedThread` claims (`owner == identity && child_index == index`). Skip each `Session` row and every row under it. Drop `Direct`, `UnderShell` and `Detached` rows (MCP servers, app servers); a kept row below a dropped one nests under its nearest kept ancestor in `children` order, else under the member. Work lines nest by that rule, so a subagent's shell sits under the subagent. Remove `CellChild`, `placed_children` and `nest`.
4. Columns. A private enum in `agent_cell.rs`:
   ```rust
   enum TreeColumn { Session, Role, Status, Account, Branch, Directory, Age }
   ```
   with `header(self) -> &'static str` from new constants in `constants.rs` (`TREE_SESSION_HEADER = "session"`, `TREE_ROLE_HEADER = "role"`, `TREE_STATUS_HEADER = "status"`, `TREE_ACCOUNT_HEADER = "account"`, `TREE_BRANCH_HEADER = "branch"`, `TREE_DIRECTORY_HEADER = "directory"`, `TREE_AGE_HEADER = "age"`). The columns a view draws are one of four `const` slices chosen by `(view, accounts)`: full = all seven, compressed = `Session, Role, Status, Account, Age`; `Hidden` leaves `Account` out of either. No line computes its own column set.
5. Cells per line:
   - Session: `lead()` glyphs in text color, then the name (with ` · <machine>` for `AnotherMachine`) in the line's hue for `Agent` and `ManagedThread`, text color for `Work`. No role suffix: `name_spans` (≈184) drops `RoleLabel::suffix`.
   - Role: `Agent(label)` → its hierarchy role, `session` when `Unlabeled` (add `RoleLabel::column_text(self) -> &'static str` in `placement.rs`, and make `HierarchyRole::text` `pub(crate)`); `ManagedThread` → its role; `Work` → `ChildKind::via()` (`shell`, `subagent`, `thread`). Label color is for headers only; role text is text color.
   - Status: `Agent` → `summary::status_text` in `summary::status_role` (as now); `ManagedThread` → `runs_label()` in `Role::Codex` (as now); `Work` → a shell with a timer reads `timer_label` (pie and time left) in `Role::Shell`; any other work reads `runs_label()` in `runs_role(kind)`.
   - Account: `Agent` → `row.account`, threads → the thread `ChildRow`'s `account`, each its label; `Unrecorded`, shells and subagents → `MISSING_VALUE`; text color.
   - Branch, Directory (full only): `Agent` and `ManagedThread` → the line's `row` (as `tree_cells` ≈680 does); `Work` → `MISSING_VALUE`.
   - Age: from the line's own start (thread and work rows use their `ChildRow::started`).
6. Header row. Drawn as the `Table::header` of the tree, one row (`TABLE_HEADER_HEIGHT`), every label in `Style::default().fg(label_color())` as `summary::draw_machine` (≈189) styles `SUMMARY_HEADERS`. The `session` header is preceded by blanks the width of `TREE_BRANCH`, so it starts over the top-level names; each other header starts its column. Each column is at least its header wide.
7. Views. `CellView::Full` holds the fitted widths of the active full columns; it is chosen as today: the header line and place line fit whole and every line's cells fit whole at their widest within `header_width(width)` with `TABLE_COLUMN_SPACING` between columns, else `Compressed`. A name is never cut in the full view. Compressed: every column but `Session` at its widest; `Session` takes the rest, each name cut by `summary::truncated` to what is left after its glyphs. The compressed view of a top-level agent still draws nothing above the header row. One `Table` draws both views; remove `compressed_session`, and `cut_line`/`gap` if nothing else calls them.
8. Removal. Delete the children table and its helpers: `Children`, `children_width`, `compressed_child`, `child_row`, `child_name`, `via_text`, `depth_indent`, `table_columns`, `FlatColumns::table`, `children_top`, and the constants `CHILD_HEADERS`, `CHILD_PID_COLUMN`…`CHILD_AGE_COLUMN`, `CHILD_VIA_INDENT`, `TREE_COLUMNS`, `AGENT_HEADER_GAP_HEIGHT`. Keep `timer_label`, `timer_pie`, `time_left_label`, `runs_role`. The module doc (≈1–20) describes the one tree with its header row.
9. Rows. A cell draws the lines `above_tree` gives (≈565, unchanged), then directly, with no gap row: the header row and one row per tree line; or, with no tree lines, the `NOTHING_RUNNING_NOTE` line alone at the tree's indent (no header row). `view_height` is `above + TABLE_HEADER_HEIGHT + lines` or `above + NOTHING_RUNNING_HEIGHT`, the rows `draw_view` fills; `height` and `CellView::fitted` keep their contracts.
10. Existing tests in `agent_cell.rs`, `render.rs` and `summary/mod.rs` that assert the children table, the role suffix in tree lines, the gap row or the old heights are updated to the new tree and keep their content, hue and label-color checks; `only_labels_take_the_label_color` (≈1621) checks the tree header labels in place of `CHILD_HEADERS`. Tests that only exist for removed code (`a_cell_compresses_where_its_table_would_cut_a_name`, the table's timer row) are rewritten against the tree line or removed.
11. `crates/cargo-handler/CHANGELOG.md` `## [Unreleased]`: an agent tile draws one tree under a header row with `session`, `role`, `status`, `account` and `age` columns (`branch` and `directory` in the full view); shell commands and subagents are lines in that tree; the list below the tree is gone; when two accounts of one program are running, the account column shows and the header line and the summary's `agent` column read the account label. Add the `[accounts]` config table to the same entry.
12. Tests in `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs`, declared inside `agent_cell.rs`'s `mod tests` as `#[path = "tree_tests.rs"] mod tree_tests;`, using `super::` fixtures (`agent`, `child`, `entry`, `drawn`, `drawn_into`, `lines`, `find`, `find_all`, `runs_in`, `boss_group`). Required cases:
    - The mockup's compressed view: a top-level showrunner with two unit sessions (`claude1`, `claude3`) and a managed Codex thread (`codex1`) under the first draws the header row `session role status account age` first, then `├─ dashboard`, `│  └─ hanadocs-dashboard-impl`, `└─ diagrams`, each value in its column; `session` starts in the column of `dashboard`.
    - The mockup's full view at a wide width: header line, place line, then the header row with `branch` and `directory`, no gap row.
    - Header labels are in the label color; names keep their hues; roles, accounts and status keep their styles from Spec item 5.
    - With every Claude row on one home and one Codex home, no `account` header or value shows and `header_line` reads `claude`; with two Claude homes, the column shows and the header line reads `claude2`.
    - A shell with a timer is a tree line under its agent: role `shell`, status the pie and time left; a subagent's shell nests under the subagent; an MCP `Direct` process and an `UnderShell` app server draw no line, and the app server's unheld thread draws under the shell with role `thread` and the app server's account.
    - No line of any cell draws text from the removed table (`pid`, `via`, `runs` headers).
    - An agent with no lines draws `nothing running` and no header row, in both views.
    - Height: for the mockup fixtures and the existing `boss_group`, `height` equals the rows drawn at every width from 10 to 140, as `the_height_is_the_rows_drawn_at_every_width` checks.
    - Summary: with two Claude homes, the summary's `agent` cell reads each row's label (`claude2`), a Codex fast row reads `codex1 fast`; with one home it reads `claude`.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeLine`, `LineSubject`, `TreeColumn`, header row, views, `account_column`, removals, updated tests, `#[path = "tree_tests.rs"] mod tree_tests;` inside `mod tests`
- `crates/cargo-handler/src/render.rs` — `Cells::accounts`, calls pass it, updated tests
- `crates/cargo-handler/src/summary/mod.rs` — `width`/`draw`/`agent_row`/`column_widths` take `AccountColumn`, updated tests
- `crates/cargo-handler/src/census/mod.rs` — `agent_label(accounts)`, `ServiceTier::mark`, `HierarchyRole::text` visibility
- `crates/cargo-handler/src/census/placement.rs` — `RoleLabel::column_text`
- `crates/cargo-handler/src/constants.rs` — tree header constants; removed child-table constants
- `crates/cargo-handler/CHANGELOG.md` — `## [Unreleased]` entry
- `crates/cargo-handler/src/census/account.rs` — `AccountHome::label()`, `AccountColumn` (see Constraints from prior phases)
- `crates/cargo-handler/src/census/account/tests.rs` — the `AccountColumn::of` cases
- `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs` — new: every case in Spec item 12

**Workers:** 1 writer + 1 tester — the tree, its callers and the summary change together and cannot split; the new tests are their own file.
- `impl` — `crates/cargo-handler/src/agent_cell.rs` (including the `#[path]` test declaration), `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/summary/mod.rs`, `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`, `crates/cargo-handler/src/census/account.rs`; hub: `crates/cargo-handler/src/constants.rs` (the header constants the tester's file reads)
- `test` — `crates/cargo-handler/src/agent_cell/tests/tree_tests.rs` and `crates/cargo-handler/src/census/account/tests.rs` (the `AccountColumn::of` cases): every case in Spec item 12, written against the signatures in items 1–4 and the fixtures in `mod tests`

**Constraints from prior phases:**
- Phase 1: `census::Account { Unrecorded, Home(AccountHome) }` on `AgentRow::account` and `ChildRow::account` (set for Codex thread rows from their app server, `Unrecorded` for shells, subagents and other children); labels resolve on the machine that ran the agent (its own `[accounts]` table), so a remote's rows arrive labelled. Phase 1 kept no API without a consumer, so this phase adds, in `census/account.rs`: a `pub(crate) fn label(&self) -> &str` on `AccountHome` (the text a tile shows), and `census::AccountColumn { Hidden, Shown }` with `AccountColumn::of(impl IntoIterator<Item = (Agent, &Account)>)`, which counts distinct labels per program, Codex and its thread host as one, with its tests in `census/account/tests.rs` (one Claude label → Hidden; `claude1`+`claude2` → Shown; `claude2`+`codex1` → Hidden; `codex1` on an app server + `codex2` on a thread host → Shown; repeated `Unrecorded` → Hidden; one label on two machines → Hidden). Both files join this phase's Files: `account.rs` is `impl`'s, `account/tests.rs` is `test`'s. No `#[expect(dead_code)]` anywhere.
- Phase 1: `AccountHome`'s fields (`home`, `label`) are private with no getters today; `AccountLabels::resolve` is its only constructor. `account.rs` holds `pub(super) enum Program { Claude, Codex }`; group programs in `AccountColumn::of` through a new `impl From<Agent> for Program` (`Claude` → `Claude`; `Codex` and `CodexThreadHost` → `Codex`) and count distinct labels per `Program`, not with a hand-written match on `Agent`. `account.rs` imports `std::os::unix::ffi::OsStrExt` unguarded (cargo-handler is not built on Windows).
- Phase 1: `agent_cell.rs`'s `use crate::census::Account;` (≈46) serves only the `ChildRow` literal in `nest` (≈875), which Spec item 3 removes; keep the import only if the new account cell matches on `Account`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green with every Spec item 12 case and the updated existing tests; `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean (no `self_named_module_files` finding, no dead code left from the table).

### Phase 3 — Live check  · status: todo

**Depends on:** Phase 2

#### Work Order

**Goal:** on natedev and the Mac the hanadocs showrunner tile shows the header row, the role and account columns with the right labels, shells and subagents as tree lines, and no list below the tree, in both views.

**Spec:**
1. The unit director sets each machine's `[accounts]` table in cargo-handler's `config.toml`. natedev: `"~/.claude" = "claude2"`, `"~/.codex" = "codex2"`. The Mac (`ssh mac`): read each home's account-label file (e.g. `~/.claude/account-label`, `~/.claude1/account-label`) and the Codex login of each Codex home, and write one pair per home that differs from its default label.
2. Install the checkpoint binary on both machines and run cargo-handler on natedev with the Mac shown.
3. Check the hanadocs showrunner tile in the full and compressed views: the header row sits between the header lines (or the title) and the tree, in the label color; `session` lines up over the top-level names; each unit and worker line shows its role, status and account label; the header line and the summary's `agent` column read the account labels; shells and subagents are tree lines; nothing is drawn below the tree.
4. Send the shots to the showrunner, and record the result here.

**Files:**
- `docs/cargo-port/remove-running-section-handler-accounts.md`

**Workers:** none — the unit director runs the live check and edits only machine config outside the repo.

**Acceptance gate:** shots of both views show every check in Spec item 3 with natedev's and the Mac's labels correct, and the showrunner has the shots.
