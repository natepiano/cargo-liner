# cargo-liner-handler

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** The handler unit's running plan for cargo-handler. Each new user request becomes a phase here. The [as-built docs](../cargo-handler/as-built/hierarchy.md) describe the current code. Approved items that wait for a place are in [the next-items file](remove-running-section-handler-footer-colors-next.md).


## Delegation Context

- **Project:** `cargo-handler`, the terminal agent watcher. It shows one tile per top-level agent and a summary tile.
- **Project started:** 2026-11-11T07:40:00-05:00
- **Stack:** Rust 2024, ratatui 0.30, cargo-nextest.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler`, branch `cargo-liner-handler`.
- **As-built docs:** `docs/cargo-handler/as-built/` — `hierarchy.md` (tile tree), `interaction.md` (keys and clicks), `accounts.md` (accounts table), `activity.md`, `colors.md`, `startup.md`, `tile-layout.md`.
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check cargo-handler`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler`
- **Invariants:**
  - Nothing on the UI thread runs a command or reads a file.
  - Tests run with nextest. Format only with `cargo +nightly fmt`. No test takes one second or more.
  - Invalid states are unrepresentable: use types (checked constructors, enums, type state), never procedural guards. No bare `Option` in new domain types.
  - No configuration value in code.
  - A folder that a process makes for its own use goes in a `tempfile::TempDir` (Rust) or `tempfile.TemporaryDirectory` (Python) with block under TMPDIR.

## Phases

<!-- PHASE NUMBERING — binds every command that edits this doc.
     N is a bare integer, numbered from 1, contiguous, in execution order.
     Never a letter suffix. Any spine edit resequences from the edit point to
     the end and updates every cross-reference.
     Full procedure: /plan:to_phased_plan → <PhaseNumbering/>. -->

### Phase 1 — Showrunner cells show account first, a role, a status word and an activity; age moves to a details view  · status: todo

**Depends on:** none

#### Work Order

**Goal:** each tile tree reads account, session, role, activity and status. Role is one of four words. Status is one of the four accounts-table words. Age leaves the tile tree and the top-level agents table, and a details view shows it with every other value of the selected row.

**Source:** the user, via the showrunner, 2026-11-11: "in cargo handler, in a showrunner cell - the account should be the first column - the role should only be unit, showrunner, worker, producer, status should be the statuses we use from accounts "busy, idle, paused, exited" - and we need another column for activity that could be fix, timer, astra reviewer, impl, test, reviewer - we can remove the age column - it's not actually that useful - you can remove the age column from the top level agents as well - i think you should calculate it still though - we can show a details pop up view of a row - when you press d on a selected row it can give you all of that information like properties: name: natedev / role: showrunner / ... / age: 32 days, 14 hours, 44 minutes." Screenshot: `/tmp/claude/scratch/until-2026-11-15T0500Z-handler-shot-229e0b/handler_showrunner_cell_0734.png`.

Showrunner, 2026-11-11: run this phase ahead of the other next items. Age is still computed and shows only in the details view, in the long form. Status uses the four accounts-table words. Live-capture the showrunner cell and the top-level agents table against main before the checkpoint.

**Spec:**

Director decisions:
1. "Account first" means the leftmost column of the tile tree. The session column, with its tree guides, comes second.
2. The role cell shows `unit`, `showrunner`, `worker` or `producer`. A worker row shows `worker`. Subagents and threads do work for a session, so their role is `worker` (showrunner, 2026-11-11). A shell (a timer or a command) has an empty role cell. An unlabeled session also has an empty role cell, because no source names its role. This replaces the C230 behavior, where a subagent row showed its role word in the role cell. That word now goes to the activity cell (showrunner, 2026-11-11).
3. The activity cell names the work: `impl`, `test`, `fix`, `reviewer`, `astra designer`, `astra reviewer`, `timer`, or the subagent's or thread's work. The user's word `reviewer` replaces the old `review` text. A unit, showrunner or producer row with no job has an empty activity cell. Any agent session row (unit, showrunner, producer, worker or unlabeled) shows `compacting` in the activity cell while that session compacts (the user, via the showrunner, 2026-11-11: "also the activity for any agent session row of any time can be compacting").
4. A timer row keeps its countdown. The countdown moves from the status cell to the activity cell (`timer 9:48`). Its status is `busy`.
5. Status is one of `busy`, `idle`, `paused`, `exited`, read from a per-session list in the accounts report: `"sessions": [{"session": "<id>", "state": "..."}]`. The list comes from `~/.claude/scripts/accounts/session_state.py` `classify`, so the tile and the accounts table cannot disagree. cargo-handler never copies `classify` (showrunner, 2026-11-11). sdlc adds the export; until it lands, a row with no entry has an empty status cell. The detailed words (writing, reading, thinking, waiting) move to the details view as a `state:` line.
6. Age stays computed for each row. It remains the default sort order, oldest first, with no header marker, because no age column is drawn. A clicked column header still sorts by that column.
7. `d` opens the details view for the selected row in a tile tree and in the top-level agents table. It lists each property as `name: value`, in the order the row draws them, then the values no column shows (pid, directory, branch, model, effort where present), and last `age:` in the long form `32 days, 14 hours, 44 minutes`. Drop leading zero units: `14 hours, 44 minutes`; under one minute shows `0 minutes`. `Esc` closes it. Use the existing tui_pane overlay pattern that the favorites and shortcuts overlays use. If `d` is already bound, report it and stop.

Tasks:
1. Tile tree: move `TreeColumn::Account` first, split `TreeColumn::Role` into `Role` (four-word type) and a new `Activity` column, and remove `TreeColumn::Age` from the drawn columns. Update `ColumnDrop::ORDER` and `GroupFitStep::ORDER` so narrow tiles drop the new column sets in a sensible order: Directory, Branch, Activity, Account.
2. Status: draw one of the four words for every row, from one status type.
3. Top-level agents table (`SummaryColumn`, `crates/cargo-handler/src/sort.rs`): remove the Age column from drawing. Keep age computed and the default order.
4. Details view: bind `d` in the default keymap, add it to the shortcuts overlay, and draw the overlay.
5. Add a producer role value where the hierarchy source can name one. A row is `producer` only when its source says so.
6. Update the as-built docs `hierarchy.md` and `interaction.md` in the same change.

Tests (red first, each under one second):
1. The tile tree header order is account, session, role, activity, status (with optional columns after), and no age header is drawn.
2. A worker row shows role `worker` and activity `impl`. A unit row shows `unit` and an empty activity.
3. A timer row shows status `busy` and activity `timer <countdown>`.
4. Each session state maps to one of the four status words. The tile status and the accounts-table count agree for one fixture.
5. The top-level agents table draws no age column and keeps the oldest-first order.
6. `d` opens the details view for the selected row. It shows `age: 32 days, 14 hours, 44 minutes` for a fixture of that age. `Esc` closes it.
7. The long age form drops leading zero units.

**Live check (director):** before the checkpoint, capture the natedev showrunner tile and the top-level agents table from a build of this branch and from main, under the safety rule (only terminals and processes the director starts).

**Change-Review line:** Showrunner tiles show account first, a role, an activity and a status word; `d` shows a row's details with its age | On natedev, run cargo-handler in a terminal, look at a showrunner tile, select a row and press d | Account is the first column; role reads unit, showrunner, worker or producer; activity holds impl, test, fix, timer or astra reviewer; status reads busy, idle, paused or exited; no age column; d lists the row's values and its age in days, hours and minutes

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeColumn`, `TreeColumns`, column drop orders, `TreeLine::value`, `draw_tree`
- `crates/cargo-handler/src/census/placement.rs` — `RoleLabel`
- `crates/cargo-handler/src/census/mod.rs` — `HierarchyRole`
- `crates/cargo-handler/src/sort.rs` — `SummaryColumn`, `TreeSort`
- `crates/cargo-handler/src/summary/mod.rs` — the top-level agents table
- `crates/cargo-handler/src/summary/age.rs` — the long age form
- `crates/cargo-handler/src/constants.rs` — headers
- `crates/cargo-handler/src/keymap.rs` — the `d` binding
- `crates/cargo-handler/src/globals.rs` — the `d` action
- `crates/cargo-handler/src/app.rs` — the details overlay state
- `docs/cargo-handler/as-built/hierarchy.md` — the tile tree columns
- `docs/cargo-handler/as-built/interaction.md` — the `d` key

**Workers:** 1 writer. Nothing splits: the column, status and overlay changes share `agent_cell.rs`, and the tests are crate unit tests beside the code.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` passes with the seven tests above, and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` passes. The live check shows the new column order, the four role words, the four status words, no age column in a tile or in the top-level agents table, and the `d` details view with the long age.

### Phase 2 — Word highlight strength is a setting, and the default is easier to see  · status: todo

**Depends on:** Phase 1 (both phases change `render/mod.rs` and `app.rs`)

#### Work Order

**Goal:** a clicked word's matches are easy to see by default. The user can set how strong the highlight is.

**Source:** the user, change review C240, 2026-11-11: "it's happening but it's too subtle - we need some configurable way to increase its noticeability". C240 shipped the word highlight in ce532875.

**Spec:**

Director decisions (the showrunner can change them before dispatch):
1. One setting holds the strength: `word_highlight.emphasis`, with the values `subtle`, `clear` and `strong`. It goes in the cargo-handler config file, beside the theme choice. No value is in code.
2. `subtle` is the C240 look. `clear` is the new default, and it is easy to see on every built-in theme. `strong` adds bold to the matched text over the `clear` ground.
3. The theme keeps the ground colors. Emphasis sets how far the match ground moves from the pane ground. The contrast rules in `MatchInk::resolve` still keep the text readable.
4. The setting reloads like the theme does. An unknown value is a config error that names the three values.

Tests (red first, each under one second):
1. Each emphasis value parses, and an unknown value gives an error that names the three values.
2. On each built-in theme, the `clear` match ground is farther from the pane ground than the `subtle` ground.
3. `strong` draws matched cells bold; `subtle` and `clear` do not.
4. Text on each match ground keeps the required contrast at each emphasis.

**Live check (director):** before the checkpoint, capture a clicked word in a cargo-handler terminal from this branch at each emphasis value and from main, under the safety rule. Capture `clear` and `strong` on the default theme and on one light theme (showrunner, 2026-11-11).

**Change-Review line:** A clicked word's matches are easy to see, and `word_highlight.emphasis` sets the strength | On natedev, run cargo-handler, click a word, then set `word_highlight.emphasis` to subtle, clear and strong | The default highlight is easy to see; strong also makes the matched words bold; subtle is the old look

**Files:**
- `crates/cargo-handler/src/theme/word_match.rs` — match grounds
- `crates/cargo-handler/src/word_highlight.rs` — match painting
- `crates/cargo-handler/src/render/mod.rs` — the painted frame
- `crates/cargo-handler/src/app.rs` — the config value
- `docs/cargo-handler/as-built/colors.md` — word match grounds
- `docs/cargo-handler/as-built/interaction.md` — the setting

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with the four tests above. The live check shows a clearly visible default highlight and three distinct strengths.

### Phase 3 — Producers get their own tile, titled "producers"; they leave the unassigned tile  · status: todo

**Depends on:** Phase 1 (it adds the `producer` role from the staff source)

#### Work Order

**Goal:** every producer session draws in one tile titled `producers`. No producer draws in the unassigned tile.

**Source:** the user, via sdlc and the showrunner, 2026-11-11: "cargo-handler should have a producers cell - that these should not show up in unassigned anymore and the title for that cell should just be \"producers\"".

**Spec:**

Director decisions:
1. A session is a producer only when the staff source names it so (`StaffKind::Producer`, the Phase 1 `producer` role). No name match.
2. The producers tile uses the unassigned tile's tree, columns, sort and details view. Each producer is a root row, and its subagents and threads draw under it.
3. The title is `producers`, a constant beside `UNASSIGNED_CELL_TITLE`.
4. The producers tile draws only when at least one producer is live. It sits before the unassigned tile.
5. The top-level agents table keeps every producer row.

Tests (red first, each under one second):
1. A producer session draws in the producers tile and not in the unassigned tile.
2. A session that staff does not name a producer stays in the unassigned tile.
3. With no live producer, no producers tile draws.
4. The producers tile title reads `producers`.

**Live check (director):** before the checkpoint, capture the producers tile and the unassigned tile from a build of this branch and from main, under the safety rule.

**Change-Review line:** Producers have their own tile titled producers | On natedev, run cargo-handler in a terminal and look for the producers tile | A tile titled producers lists the producer sessions; none of them shows in the unassigned tile

**Files:**
- `crates/cargo-handler/src/census/placement.rs` — tile placement
- `crates/cargo-handler/src/census/hierarchy.rs` — producer staff rows
- `crates/cargo-handler/src/constants.rs` — the tile title
- `crates/cargo-handler/src/render/mod.rs` — tile order
- `docs/cargo-handler/as-built/hierarchy.md` — the producers tile

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with the four tests above. The live check shows the producers tile and no producer in the unassigned tile.
