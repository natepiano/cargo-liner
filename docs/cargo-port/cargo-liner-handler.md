# cargo-liner-handler

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** The handler unit's running plan for cargo-handler. Each new user request becomes a phase here. The [as-built docs](../cargo-handler/as-built/hierarchy.md) describe the current code. Approved items that wait for a place are in [the next-items file](remove-running-section-handler-footer-colors-next.md).


## Delegation Context

- **Project:** `cargo-handler`, the terminal agent watcher. It shows one tile per top-level agent and a summary tile.
- **Project started:** 2026-11-11
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

#### Work Order

**Goal:** each tile tree reads account, session, role, activity and status. Role is one of four words. Status is one of the four accounts-table words. Age leaves the tile tree and the top-level agents table, and a details view shows it with every other value of the selected row.

**Source:** the user, via the showrunner, 2026-11-11: "in cargo handler, in a showrunner cell - the account should be the first column - the role should only be unit, showrunner, worker, producer, status should be the statuses we use from accounts "busy, idle, paused, exited" - and we need another column for activity that could be fix, timer, astra reviewer, impl, test, reviewer - we can remove the age column - it's not actually that useful - you can remove the age column from the top level agents as well - i think you should calculate it still though - we can show a details pop up view of a row - when you press d on a selected row it can give you all of that information like properties: name: natedev / role: showrunner / ... / age: 32 days, 14 hours, 44 minutes." Screenshot: `/tmp/claude/scratch/until-2026-11-15T0500Z-handler-shot-229e0b/handler_showrunner_cell_0734.png`.

Showrunner, 2026-11-11: run this phase ahead of the other next items. Age is still computed and shows only in the details view, in the long form. Status uses the four accounts-table words. Live-capture the showrunner cell and the top-level agents table against main before the checkpoint.

**Director decisions:**
1. "Account first" means the leftmost column of the tile tree. The session column, with its tree guides, comes second.
2. The role cell shows `unit`, `showrunner`, `worker` or `producer`. A worker row shows `worker`. A row whose source names none of the four (a shell, a subagent, a thread, an unlabeled session) has an empty role cell.
3. The activity cell holds the job word that the role cell holds today: `impl`, `test`, `review`, `fix`, `astra designer`, `astra reviewer`, `timer`, and the subagent or thread kind. A unit, showrunner or producer row with no job has an empty activity cell. The user's word "reviewer" is the existing `review` value; keep the existing words.
4. A timer row keeps its countdown. The countdown moves from the status cell to the activity cell (`timer 9:48`). Its status is `busy`.
5. Status maps each session state to one of `busy`, `idle`, `paused`, `exited`, with the same meaning the accounts table counts use. Find the source of that classification and use it, so the two cannot disagree. Do not copy its rules into a second place.
6. Age stays computed for each row. It remains the default sort order, oldest first, with no header marker, because no age column is drawn. A clicked column header still sorts by that column.
7. `d` opens the details view for the selected row in a tile tree and in the top-level agents table. It lists each property as `name: value`, in the order the row draws them, then the values no column shows (pid, directory, branch, model, effort where present), and last `age:` in the long form `32 days, 14 hours, 44 minutes`. Drop leading zero units: `14 hours, 44 minutes`; under one minute shows `0 minutes`. `Esc` closes it. Use the existing tui_pane overlay pattern that the favorites and shortcuts overlays use. If `d` is already bound, report it and stop.

**Tasks:**
1. Tile tree: move `TreeColumn::Account` first, split `TreeColumn::Role` into `Role` (four-word type) and a new `Activity` column, and remove `TreeColumn::Age` from the drawn columns. Update `ColumnDrop::ORDER` and `GroupFitStep::ORDER` so narrow tiles drop the new column sets in a sensible order: Directory, Branch, Activity, Account.
2. Status: draw one of the four words for every row, from one status type.
3. Top-level agents table (`SummaryColumn`, `crates/cargo-handler/src/sort.rs`): remove the Age column from drawing. Keep age computed and the default order.
4. Details view: bind `d` in the default keymap, add it to the shortcuts overlay, and draw the overlay.
5. Add a producer role value where the hierarchy source can name one. A row is `producer` only when its source says so.
6. Update the as-built docs `hierarchy.md` and `interaction.md` in the same change.

**Tests (red first, each under one second):**
1. The tile tree header order is account, session, role, activity, status (with optional columns after), and no age header is drawn.
2. A worker row shows role `worker` and activity `impl`. A unit row shows `unit` and an empty activity.
3. A timer row shows status `busy` and activity `timer <countdown>`.
4. Each session state maps to one of the four status words. The tile status and the accounts-table count agree for one fixture.
5. The top-level agents table draws no age column and keeps the oldest-first order.
6. `d` opens the details view for the selected row. It shows `age: 32 days, 14 hours, 44 minutes` for a fixture of that age. `Esc` closes it.
7. The long age form drops leading zero units.

**Live check (director):** before the checkpoint, capture the natedev showrunner tile and the top-level agents table from a build of this branch and from main, under the safety rule (only terminals and processes the director starts).

**Change-Review line:** Showrunner tiles show account first, a role, an activity and a status word; `d` shows a row's details with its age | On natedev, run cargo-handler in a terminal, look at a showrunner tile, select a row and press d | Account is the first column; role reads unit, showrunner, worker or producer; activity holds impl, test, fix, timer or astra reviewer; status reads busy, idle, paused or exited; no age column; d lists the row's values and its age in days, hours and minutes

**Files (expected):**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeColumn`, `TreeColumns`, column drop orders, `TreeLine::value`, `draw_tree`.
- `crates/cargo-handler/src/census/placement.rs` — `RoleLabel`; `crates/cargo-handler/src/census/mod.rs` — `HierarchyRole`.
- `crates/cargo-handler/src/sort.rs` — `SummaryColumn`, `TreeSort`; `crates/cargo-handler/src/summary/mod.rs`, `summary/age.rs`.
- `crates/cargo-handler/src/constants.rs` — headers.
- `crates/cargo-handler/src/keymap.rs`, `globals.rs`, `app.rs` — the `d` binding and overlay.
- `docs/cargo-handler/as-built/hierarchy.md`, `interaction.md`.
