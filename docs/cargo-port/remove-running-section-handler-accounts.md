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

#### Work Order

**Goal:** every agent row and every Codex thread row carries a typed `Account` (home plus label) read from its process's `CLAUDE_CONFIG_DIR` or `CODEX_HOME`, labelled through an optional `[accounts]` config table, carried over the probe wire, with older probes still parsing.

**Spec:**
1. New module `crates/cargo-handler/src/census/account.rs`, declared `pub(crate) mod account;` in `census/mod.rs` (≈18–30, alphabetical) with `pub(crate) use account::Account;`, `pub(crate) use account::AccountColumn;`. Types:
   ```rust
   /// The configuration home an agent process runs under, with the label it shows as.
   #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
   #[serde(from = "Option<AccountHome>", into = "Option<AccountHome>")]
   pub(crate) enum Account {
       /// No environment was read for the row: a probe printed before accounts
       /// existed, a process whose environment could not be read, or a row that
       /// is not an agent process (a shell, a subagent).
       #[default]
       Unrecorded,
       /// The home the process runs under.
       Home(AccountHome),
   }

   /// A home and its label.
   #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
   pub(crate) struct AccountHome {
       /// The home directory, with the machine's home written as `~` (`~/.claude1`).
       home:  String,
       /// What the tile shows: the `[accounts]` label for `home`, else the
       /// home's last component without its leading dot (`~/.claude1` → `claude1`).
       label: String,
   }
   ```
   `AccountHome` fields stay private with getters `home()` and `label()`; the only constructor is `AccountLabels::resolve` (item 3), so a label always matches its home. Add `Account::is_unrecorded(&self) -> bool` for `skip_serializing_if`, and `Account::label(&self) -> Option<&str>` only if phase 2's accessors need it (prefer a match at the call site). `From<Option<AccountHome>>` / `From<Account> for Option<AccountHome>` give the wire form: `null` or `{"home":"~/.claude1","label":"claude1"}`.
2. What a process's environment says, on `ProcessEntry` (`classify.rs` ≈65), a new field `config_home: ConfigHome` defined in `account.rs`:
   ```rust
   /// The configuration home a process's environment names.
   pub(super) enum ConfigHome {
       /// The environment was not read.
       Unread,
       /// The environment was read and sets neither variable: the tool's default home.
       Default,
       /// The value of `CLAUDE_CONFIG_DIR` (Claude Code) or `CODEX_HOME` (Codex).
       Named(PathBuf),
   }
   ```
   `fn config_home(environment: &[OsString], program: Program) -> ConfigHome` beside `claude_pid` (`scan.rs` ≈767), same `NAME=value` parsing; an empty value reads as `Default`. Constants in `constants.rs` beside `CLAUDE_PID_VARIABLE` (≈462): `CLAUDE_CONFIG_DIR_VARIABLE = "CLAUDE_CONFIG_DIR"`, `CODEX_HOME_VARIABLE = "CODEX_HOME"`, each with a doc comment. Which variable is read follows the process: Claude Code (`ProcessEntry::is_claude`) reads `CLAUDE_CONFIG_DIR`, Codex (`is_codex`) reads `CODEX_HOME`; use a small private enum for the program, not a bool. Every `ProcessEntry` literal in the crate (scan.rs ≈745 and test fixtures in classify.rs, scan.rs, scan/staged_tests.rs, tree.rs, desktop.rs, placement.rs) gains `config_home: ConfigHome::Unread`.
3. The environment pass in `process_table` (`scan.rs` ≈727–741) reads `environ` for every Codex process (as now) and every Claude Code process: the session-record pids (`sessions`) and processes named `claude`, so macOS's version-named binaries are covered through their session records. The doc comment ≈698–705 names both reads. `claude_pid` is still set for Codex only.
4. Labels. `AccountLabels` in `account.rs`: built once from config (`From<&AccountsConfig>`), holding the configured home→label pairs with each key normalized to the `~` form through `classify::directory_label` (make it `pub(super)` if needed). `fn resolve(&self, home: &Path, machine_home: Option<&Path>) -> AccountHome`: the `~` form of `home`, its configured label, else its last component with one leading `.` stripped. The default home for `ConfigHome::Default` is `<machine home>/CLAUDE_DIRNAME` or `<machine home>/CODEX_DIRNAME`. `fn account_of(process: &ProcessEntry, machine_home: Option<&Path>, labels: &AccountLabels) -> Account`: `Unread` → `Unrecorded`; `Default` and `Named` → `Home`.
5. Config: `crates/cargo-handler/src/config.rs` gets
   ```rust
   /// `[accounts]`: each agent home this machine runs, mapped to the label its tiles show.
   #[derive(Debug, Default, Deserialize, Serialize)]
   #[serde(transparent)]
   pub(crate) struct AccountsConfig(pub(crate) BTreeMap<String, String>);
   ```
   and `pub(crate) accounts: AccountsConfig` on `Config` after `census`, with its doc line. Keys are homes written with `~` (`"~/.claude" = "claude2"`). Update the known-file test (≈181) to the text toml prints for the default (an empty `[accounts]` table after `[census]`) and keep `a_complete_config_is_left_alone` green. Labels are read at startup; a config edit takes effect on restart (the table has no settings-overlay row).
6. Domain fields. `AgentRow` (`census/mod.rs` ≈276) gets `pub(crate) account: Account` after `service_tier`, documented "The home the agent's process runs under. Missing from a probe printed before it existed." `ProbeAgentRow` (`probe.rs` ≈35) gets `#[serde(default)] account: Account` and the `From` impl copies it. `ChildRow` (≈407) gets `#[serde(default, skip_serializing_if = "Account::is_unrecorded")] pub(crate) account: Account` after `service_tier`, documented "The app server's home, for a Codex thread; unrecorded for any other row. Missing from a probe printed before it existed." Every `AgentRow` and `ChildRow` literal in the crate (≈70 `AgentRow`, ≈17 `ChildRow`, tests included, in agent_cell.rs, render.rs, summary/mod.rs, terminal.rs and census/*) gains `account: Account::Unrecorded` (or `..` from a fixture that has it).
7. Attaching accounts. `LocalScanner` gets `labels: AccountLabels`; `LocalScanner::new(labels: AccountLabels)`. In `scan_stages_with` (≈180), right after `classify::agent_rows` and before the `Classified` publish, set each row's `account` from the process with the row's pid (`account_of`). `app_server_threads` (≈614) takes `home` and `&AccountLabels` and sets `ThreadEntry::account` (new field on `ThreadEntry`, `tree.rs` ≈62) from the app server's process; `tree.rs` copies it through `Node` (≈283–299) and `flatten` (≈547) into the thread's `ChildRow`; every other `Node` gets `Account::Unrecorded`. Callers: `schedule::spawn(remotes, hierarchy, labels)` (≈113) passes them to `LocalScanner::new`; `terminal.rs` ≈60 passes `AccountLabels::from(&app.loaded_config.config.accounts)`; `probe::print` (≈144) builds them from `loaded.config.accounts`, so a remote labels its own homes; test scanners use `AccountLabels::default()`.
8. `AccountColumn` in `account.rs`, used by phase 2:
   ```rust
   /// Whether a frame's lines name their accounts, decided once per frame.
   #[derive(Clone, Copy, Debug, Eq, PartialEq)]
   pub(crate) enum AccountColumn {
       /// Each program runs under at most one account label on screen.
       Hidden,
       /// Some program runs under two or more account labels on screen.
       Shown,
   }
   impl AccountColumn {
       /// Shown when two distinct labels of the same program are among `accounts`.
       pub(crate) fn of<'a>(accounts: impl IntoIterator<Item = (Agent, &'a Account)>) -> Self;
   }
   ```
   `Agent::Codex` and `Agent::CodexThreadHost` count as one program; `Unrecorded` counts for nothing. Distinct means distinct labels, so two machines' homes with one label count once.
9. Probe: update the known-JSON test (`probe.rs` ≈324) for `"account":null` after `"service_tier"` in every row (child rows omit it while unrecorded). The schema stays 3.
10. Tests in `crates/cargo-handler/src/census/account/tests.rs`, declared at the end of `account.rs` as `#[cfg(test)] #[path = "account/tests.rs"] mod tests;`. Required cases:
    - `config_home`: `CLAUDE_CONFIG_DIR=/home/u/.claude1` among other variables → `Named`; unset → `Default`; empty → `Default`; `CLAUDE_CONFIG_DIRS=x` → `Default`; a Codex process reads `CODEX_HOME`, not `CLAUDE_CONFIG_DIR`.
    - `resolve`: `/home/u/.claude1` under machine home `/home/u` → home `~/.claude1`, label `claude1`; `Default` for Claude → `~/.claude`, label `claude`; for Codex → `~/.codex`, `codex`; a configured `"~/.claude" = "claude2"` → `claude2`; a key written as `/home/u/.codex` still matches `~/.codex`; a home outside the machine home keeps its absolute path and labels by its last component.
    - `account_of`: `Unread` → `Unrecorded`.
    - `AccountColumn::of`: one Claude label → `Hidden`; `claude1` and `claude2` → `Shown`; `claude2` and `codex1` → `Hidden`; `codex1` on an app server and `codex2` on a thread host → `Shown`; repeated `Unrecorded` → `Hidden`; the same label from two machines → `Hidden`.
    - Wire: an `AgentRow` with `Account::Home` round-trips through `probe::parse_report` on a schema-3 JSON line; a row JSON without `account` parses to `Unrecorded`; a `ChildRow` with `Unrecorded` serializes without an `account` key; a thread `ChildRow` with a home round-trips.
    - Config: `toml::from_str::<Config>("[accounts]\n\"~/.claude\" = \"claude2\"\n")` gives that pair and restates to itself; a config without `[accounts]` parses to an empty table.

**Files:**
- `crates/cargo-handler/src/census/account.rs` — new: `Account`, `AccountHome`, `ConfigHome`, `AccountLabels`, `account_of`, `config_home`, `AccountColumn`; `#[path]` test declaration
- `crates/cargo-handler/src/census/mod.rs` — `mod account` and re-exports; `account` on `AgentRow` and `ChildRow`; literals in its tests
- `crates/cargo-handler/src/census/probe.rs` — `ProbeAgentRow::account`, `From`, `print` passes labels, known-JSON test
- `crates/cargo-handler/src/census/scan.rs` — environment pass, `LocalScanner::labels`, `new(labels)`, accounts attached before the `Classified` publish, `app_server_threads` labels; test literals
- `crates/cargo-handler/src/census/scan/staged_tests.rs` — literals
- `crates/cargo-handler/src/census/classify.rs` — `ProcessEntry::config_home`; `directory_label` visibility; test literals
- `crates/cargo-handler/src/census/tree.rs` — `ThreadEntry::account`, `Node::account`, `flatten`; test literals
- `crates/cargo-handler/src/census/schedule.rs` — `spawn` takes `AccountLabels`; test literals
- `crates/cargo-handler/src/census/desktop.rs`, `crates/cargo-handler/src/census/placement.rs`, `crates/cargo-handler/src/census/remote.rs` — test literals
- `crates/cargo-handler/src/config.rs` — `AccountsConfig`, `Config::accounts`, known-file test
- `crates/cargo-handler/src/constants.rs` — `CLAUDE_CONFIG_DIR_VARIABLE`, `CODEX_HOME_VARIABLE`
- `crates/cargo-handler/src/terminal.rs` — passes labels to `spawn`; test literals
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/summary/mod.rs` — test literals only
- `crates/cargo-handler/src/census/account/tests.rs` — new: every case in Spec item 10

**Workers:** 1 writer + 1 tester — the field and its literals touch every census file and cannot split; the new tests are their own file.
- `impl` — every file above except `census/account/tests.rs`; hub: `crates/cargo-handler/src/census/mod.rs` (module declaration and re-exports)
- `test` — `crates/cargo-handler/src/census/account/tests.rs`: every case in Spec item 10, written against the signatures in items 1–8

**Constraints from prior phases:** none.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green with every Spec item 10 case and the updated known-JSON and known-file tests; `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean (no `self_named_module_files` finding). No tile output changes in this phase.

### Phase 2 — Tile tree  · status: todo

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
