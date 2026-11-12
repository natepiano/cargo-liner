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
- **Status source:** sdlc owns the accounts session list and the detached-run status rule. A row with no entry draws an empty status cell. cargo-handler never makes a status word of its own. Draw, expansion and display deadlines all take `app.accounts.table.session_states()`.

## Phases

<!-- PHASE NUMBERING — binds every command that edits this doc.
     N is a bare integer, numbered from 1, contiguous, in execution order.
     Never a letter suffix. Any spine edit resequences from the edit point to
     the end and updates every cross-reference.
     Full procedure: /plan:to_phased_plan → <PhaseNumbering/>. -->

### Phase 1 — Showrunner cells show account first, a role, a status word and an activity; age moves to a details view  · status: done

#### As-built

- A tile tree draws account, session (with its tree guides), role, activity, status, then branch and the optional columns. The top-level agents table draws pid, account, session, status, directory. Neither draws an age column.
- Role is `unit`, `showrunner`, `worker`, `producer`, or empty. Subagents and threads are `worker`; a shell or an unlabeled session has an empty role. A row is `producer` only when its hierarchy source says so.
- Activity names the job: `impl`, `test`, `fix`, `reviewer`, `astra designer`, `astra reviewer`, `timer <countdown>`, or the subagent's or thread's work. Any agent session row shows `compacting` while it compacts. A timer row's status is `busy`.
- Status is `busy`, `idle`, `paused` or `exited`, read each scan from the accounts report's per-session list (`ReportedSessionState`), which `session_state.py` `classify` produces. A row with no entry draws an empty status cell. `AgentHeading::new(entry, account_column, footer_colors)` draws the reported word in status ink. The detailed state words (writing, reading, thinking, waiting) show only on the details view `state:` line.
- Age stays computed and keeps the default order, oldest first, with no header marker; the unassigned and showrunner tiles keep session name ascending.
- `d` opens a Row details view (tui_pane overlay pattern) for the selected row in a tile tree or the top-level agents table. It lists each property as `name: value` in drawn order, then the undrawn values (pid, directory, branch, model, effort), then `age:` in the long form `32 days, 14 hours, 44 minutes`, with leading zero units dropped and `0 minutes` under one minute. `Esc` closes it.
- The census finds detached runs under `<session>/detached_runs/` with a live `detached_runs.py supervise <record>` process, from `DETACHED_RUNS_DIRECTORY` and `DETACHED_RUNS_SCRIPT` in `constants.rs`. The run's step shows on the details view `state:` line.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeColumn`, `TreeColumns`, `ColumnDrop::ORDER` and `GroupFitStep::ORDER` (drop Directory, Branch, Activity, Account), `AgentHeading`
- `crates/cargo-handler/src/details.rs` — the Row details view; tests in `src/render/details_tests.rs`
- `crates/cargo-handler/src/summary/age.rs` — the long age form
- `crates/cargo-handler/src/census/activity.rs`, `census/worker_dispatch/sessions.rs`, `constants.rs` — detached run records and supervisors
- `crates/cargo-handler/src/expansion.rs`, `terminal.rs` — `AgentPlacement` built with session states
- `crates/cargo-handler/src/keymap.rs`, `globals.rs`, `app.rs` — the `d` binding, action and overlay state
- `docs/cargo-handler/as-built/hierarchy.md`, `interaction.md`, `activity.md` — columns, the `d` key, detached run names

**Binds later work:** the producers tile uses the same columns, column order and drop order as the other tiles (Phase 3: session, account, role, activity, status, branch), with role `producer` and status from the accounts session list, and the same `d` view. The `d` view lists model and effort when a tile drops them.

**Gotchas:**
- Draw, expansion and display deadlines all build `AgentPlacement` with `app.accounts.table.session_states()`; empty states make a status-sorted tile expand the wrong row.
- Some Codex producers (brain, morty) have no entry in the accounts session list, so their status cell is empty until sdlc exports them.
- A worker whose turn ended reads `idle` while its detached run lives until sdlc's `classify` makes such a session `busy`; cargo-handler adds no rule of its own.

**Ruled out:** a copy of `classify` in cargo-handler (the tile and the accounts table must not disagree); the old `mesh_runs/` names (`mesh_runs.py` is deleted); producers in the top-level agents table (not asked).

### Phase 2 — Word highlight strength is a setting, and the default is easier to see  · status: done

**Depends on:** Phase 1 (both phases change `render/mod.rs` and `app.rs`)

#### Work Order

**Goal:** a clicked word's matches are easy to see by default. The user can set how strong the highlight is.

**Source:** the user, change review C240, 2026-11-11: "it's happening but it's too subtle - we need some configurable way to increase its noticeability". C240 shipped the word highlight in ce532875.

**Spec:**

Director decisions (the showrunner can change them before dispatch):
1. One setting holds the strength: `word_highlight.emphasis`, with the values `subtle`, `clear` and `strong`. It goes in the cargo-handler config file, beside the theme choice. No value is in code.
2. `subtle` is the C240 look. `clear` is the new default, and it is easy to see on every built-in theme. `strong` adds bold to the matched text over the `clear` ground.
3. The theme keeps the ground colors. Emphasis sets how far the match ground moves from the pane ground. The contrast rules in `MatchInk::resolve` still keep the text readable.
4. A change to `config.toml` takes effect without a restart. A background reader (never the UI thread) reads the changed file and sends a checked `WordHighlightEmphasis` to the app. An invalid value keeps the last valid emphasis and shows a config error that names `subtle`, `clear` and `strong`. Today only the theme files reload; `config.rs` loads once at startup.

**Constraints from prior phases:**
- `MatchGroundsCache` and `MatchPaletteState` key their cached results on the theme, the ground and the selected word. Add the emphasis to both keys, and drop the kept body when the emphasis changes.
- `WordMatchGrounds` keeps separate grounds for plain rows, focused selections and unfocused selections. Keep all three, and keep the `MatchInk::resolve` readability rules.
- `strong` adds bold to matched cells. `subtle` and `clear` keep the cell's existing modifiers and add none.

Tests (red first, each under one second):
1. Each emphasis value parses, and an unknown value gives an error that names the three values.
2. On each built-in theme, the `clear` match ground is farther from the pane ground than the `subtle` ground.
3. `strong` adds bold to matched cells. `subtle` and `clear` keep existing bold and other modifiers. Unmatched cells keep their styles.
4. Text on each match ground keeps the required contrast at each emphasis.
5. Each emphasis keeps a match distinct from its ground on plain rows and in both selection states, on every built-in theme, tint on and off.
6. A valid config change updates a drawn highlight with no theme change and no new word click. An invalid change keeps the last emphasis and shows its error. A later valid change clears the error.

**Live check (director):** before the checkpoint, capture a clicked word in a cargo-handler terminal from this branch at each emphasis value and from main, under the safety rule. Capture `clear` and `strong` on the default theme and on one light theme (showrunner, 2026-11-11).

**Change-Review line:** A clicked word's matches are easy to see, and `word_highlight.emphasis` sets the strength | On natedev, run cargo-handler, click a word, then set `word_highlight.emphasis` to subtle, clear and strong | The default highlight is easy to see; strong also makes the matched words bold; subtle is the old look

**Files:**
- `crates/cargo-handler/src/theme/word_match.rs` — match grounds
- `crates/cargo-handler/src/word_highlight.rs` — match painting
- `crates/cargo-handler/src/render/mod.rs` — the painted frame
- `crates/cargo-handler/src/app.rs` — the config value and its updates
- `crates/cargo-handler/src/config.rs` — emphasis parse, default and serialization
- `crates/cargo-handler/src/terminal.rs` — the background config reader and update delivery
- `crates/cargo-handler/src/main.rs` — registration of a new reader module
- `crates/cargo-handler/src/render/word_contrast_tests.rs` — selection ground readability and separation
- `crates/cargo-handler/src/render/word_click_tests.rs` — modifier rules and a changed emphasis
- `docs/cargo-handler/as-built/colors.md` — word match grounds
- `docs/cargo-handler/as-built/interaction.md` — the setting

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows a clearly visible default highlight and three distinct strengths.

### Phase 3 — Tile columns read session, account, role, activity, status, branch; a long session name is cut before those columns drop; the details view groups its lines into sections, aligns them and copies a value with y  · status: todo

**Depends on:** Phase 1 (it changes the Phase 1 column order and drop order)

#### Work Order

**Goal:** each tile tree reads session, account, role, activity, status, then branch. A narrow tile cuts a long session name before it drops account, role, activity or status. The natedev showrunner tile shows its activity column at its current width. The `d` details view groups its lines into four sections (agent, hierarchy, location, now), with the values in one aligned column. `y` copies the value of the selected line.

**Source:** the user, change review C320 (returned), via the showrunner, 2026-11-12: "i don't see an activity column in natedev - there is a long session label under accounts and i think it is preventing the activity column from showing - but it should show - we should prioritize the activity over a long session name - - is there a reason you put account before the session? i don't remember asking for that - it should go session, account, role, activity, status, branch - it's okay to drop the branch info and the long session names but we should prioritize not dropping account, role, activity and status till we absolutely have to". This ruling replaces the Phase 1 order ("the account should be the first column", the user, 2026-11-11). Showrunner, 2026-11-12: run this phase ahead of the other open phases.

The user, change review C322 (returned), 2026-11-12: "the row details is there - it should probably show data with labels alphabetical and the value column should be left aligned after the longest label - there should be a 1 space margin on the left before the label and on the right after the last value char". The alignment and the margins stay. The alphabetical order is replaced, see below.

The user, via the showrunner, 2026-11-12, a later C322 change: "details should show effort also - and instead of program let's call it family - and i said we should have things alphabetical but really i think we should group related things together - like an agent section with account, family, model, effort as that is "always true stuff" and then showrunner, or showrunner/unit, or showruner/unit/worker - ujnder hierarchy - and then the ephermal stuff - like activity, status, - you get what i mean - group these things". Showrunner: the user's shot of a showrunner row shows account, name, role, activity, status, branch, pid, directory, model, program, state and age, with no effort line. This replaces the Phase 1 details order and the alphabetical order.

The user, via the showrunner, 2026-11-12, an addition to C322: "on the last one i sent about the row details - pressing y should yank the value - we should share code with cargo port which supports y for yank - shared code goes in tui_pane". Showrunner: move cargo-port's yank code into tui_pane and make both apps use it, with no copy; cargo-port's `y` keeps its behavior; the handler unit may edit cargo-port for this move after it reserves the files with berth.

**Spec:**

Director decisions:
1. The tile tree column order is session (with its tree guides), account, role, activity, status, branch, then the optional columns (type, model, effort, directory). The top-level agents table does not change.
2. A narrow tile removes width in this order:
   1. The optional columns: effort, model, directory, type.
   2. Branch.
   3. The session column: long session names end in `…`, down to the session column minimum width.
   4. Then, only when the session column is at its minimum: account, role, activity, status, in that order.
   5. Then the compressed view, as today.
3. The session column minimum width holds the tree guides of the deepest row and the session header. No new width constant.
4. The cut is on grapheme boundaries and keeps span styles. The `d` view `name:` line and selection identity keep the full name.
5. `ColumnDrop::ORDER` and `GroupFitStep::ORDER` take the new order. A session cut step goes between branch and the four core columns. Tiles that share a screen column keep one shared session width.
6. The details view groups its lines into four sections, in this order. Inside a section the lines keep the order below. A line with no value is left out, and a section with no lines is left out.
   1. `agent`, the values that do not change while the session lives: account, family, model, effort, type.
   2. `hierarchy`, the row's place in the production: `showrunner:`, `unit:`, `worker:` with the name at each level that applies, then role. A row with no place in the production (an unassigned session, a shell) shows `name:` here.
   3. `location`: directory, branch, desktop, pid.
   4. `now`, the values that change: activity, status, state, age.
7. Each section starts with its name on a heading line, in the existing title style. One blank line separates two sections.
8. The `program` label becomes `family`. The value does not change.
9. The `effort` line always shows in the `agent` section. A row with no effort value shows `—`. Phase 6 supplies the value for Claude rows.
10. Each value starts one cell after the longest label and its colon, left aligned, in one column across all sections. The view has a margin of one space before each label and heading, and one space after the longest value. The overlay width comes from these margins.
11. The details view has a selected line. Up and Down move it, and they skip the heading lines and blank lines. The selected line draws in the shared selection style.
12. `y` copies the selected line's value to the clipboard and shows the same copy notice as cargo-port. It copies the value only, without its label.
13. The yank path is shared through tui_pane. tui_pane already owns the copy registration, routing and clipboard call (`crates/tui_pane/src/copy.rs`). The worker first lists which parts of cargo-port's `y` still live in cargo-port. Each such part moves into tui_pane, and cargo-port calls it there. No part is copied. Reserve the cargo-port files with `cargo-berth` before the first edit.

**Constraints from prior phases:**
- Phase 1: draw, expansion and display deadlines all build `AgentPlacement` with `app.accounts.table.session_states()`.
- Phase 1: the `d` view lists the values a tile drops.
- Phase 6 (shell rows) uses this phase's session cut. A shell row never sets the preferred session width.

Tests (red first, each under one second):
1. The tile tree header order is session, account, role, activity, status, branch, then the optional columns.
2. The C320 case: a natedev showrunner tile fixture with one long session name, at the C320 width. Branch drops, the long name ends in `…`, and account, role, activity and status all draw.
3. As the width shrinks, the optional columns and directory drop first, then branch, then the session name is cut. Account, role, activity and status stay.
4. Below the session minimum width, account drops first and status drops last.
5. Tiles that share a screen column draw one session width and aligned columns after a cut.
6. The `d` view of a cut row shows the full session name.
7. The `d` view of a worker row draws the sections agent, hierarchy, location, now, in that order, each with its heading, and with `showrunner:`, `unit:` and `worker:` lines, read back from the rendered buffer.
8. The `d` view of a showrunner row draws `showrunner:` only in its hierarchy section. The `d` view of an unassigned session draws `name:` there.
9. The `d` view draws `family:`, not `program:`, and always draws `effort:`. A row with no effort value shows `—`.
10. Every value starts in the same cell across all sections, with one space before each label and one space after the longest value, read back from the rendered buffer.
11. In the details view, Down skips the heading lines, and Down then `y` copies the second value line's value, without its label.
12. cargo-port's existing `y` tests pass unchanged.

**Live check (director):** before the checkpoint, capture the natedev showrunner tile at its current width from a build of this branch and from main, under the safety rule. The activity column must show. Also capture one `d` view from each build.

**Change-Review line:** Tile columns read session, account, role, activity, status, branch; a long session name is cut before those columns drop | On natedev, run cargo-handler and look at the natedev showrunner tile | Session is the first column, then account, role, activity and status; the activity column shows; a long session name ends in …; d groups its lines into agent, hierarchy, location and now sections, shows family and effort, and aligns the values in one column; in d, y copies the selected value

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeColumn` order, `ColumnDrop::ORDER`, `GroupFitStep::ORDER`, `FittedTreeColumns`, the session cut
- `crates/cargo-handler/src/render/narrow_width_tests.rs` — the drop order and the C320 case
- `crates/cargo-handler/src/details.rs` — the sections, the `family` label, the value column and the margins
- `crates/cargo-handler/src/render/details_tests.rs` — the full name, the label order and the alignment in the `d` view
- `docs/cargo-handler/as-built/interaction.md` — the `d` view layout and `y`
- `crates/tui_pane/src/copy.rs` — the shared yank path
- `crates/cargo-port/src/` — the yank parts that move to tui_pane (reserved with berth first)
- `docs/cargo-handler/as-built/hierarchy.md`, `docs/cargo-handler/as-built/tile-layout.md` — the column order and the drop order

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test` passes for cargo-handler, tui_pane and cargo-port with all listed tests, and `bash ~/.claude/scripts/direct/verify.sh lint` passes for each. The live check shows the activity column in the natedev showrunner tile, and a `d` view with its four sections and aligned values.

### Phase 4 — Long and short tiles share a column, so the long ones show more rows  · status: todo

**Depends on:** Phase 3 (it uses the Phase 3 row measures and column widths)

#### Work Order

**Goal:** the grid pairs the longest tile with the shortest tile in one screen column, the next longest with the next shortest, and so on. Long tiles then get the rows that short tiles do not use.

**Source:** the user, via the showrunner, 2026-11-12: "in cargo-handler we should factor content rows into cell organization - there are 4 detail cells right now - and the two longest ones are on the left - 41 and 42 - if each of these were paired with the two smallest cells automatically then we could see more - i.e. pair the longest with the shortest, the next longest with the next shortest - factoring their full content rows into account to calculate the cell ordering". Showrunner context: on natedev the left column held natedev (42 content rows) above hana (42), and the right column held cargo-liner (11) above unassigned (3), so each long tile showed about 28 to 36 of its 42 rows. Showrunner: if the layout code lives in tui_pane or is shared with cargo-tile, the change goes there.

**Spec:**

Director decisions:
1. The column assignment uses each detail tile's full content-row count (every row of its tree, not only the rows it draws).
2. When each screen column holds two tiles, the longest tile shares a column with the shortest, the next longest with the next shortest, and so on. For other counts, tiles go longest first to the column with the fewest total rows; with two tiles in each column this gives the same pairs.
3. The summary tile keeps its place. Only the detail tiles move.
4. Ties keep the current order. The order changes only when the ranking of full row counts changes, and the existing grid motion animates the move.
5. The column assignment lives in the shared tui_pane grid (`crates/tui_pane/src/tiles/grid/`), as a placement rule that an app selects. cargo-handler selects the balanced rule. cargo-tile keeps its current order, because no one asked for a change there.
6. Within a column, the tiles keep their current relative order (hierarchy order, then unassigned).

Tests (red first, each under one second):
1. The natedev case: tiles of 42, 42, 11 and 3 rows in two columns give the columns {42, 3} and {42, 11}, in any column order.
2. With six tiles in three columns, each column holds one long and one short tile by the pairing rule.
3. Equal row counts keep the current order. A row change that does not change the ranking moves no tile.
4. The summary tile keeps its slot.
5. cargo-tile's grid order tests pass unchanged.
6. A long tile draws more of its rows after the pairing than before, at the natedev size, read from the rendered buffer.

**Live check (director):** before the checkpoint, capture the natedev grid from a build of this branch and from main at the same size, under the safety rule. Each long tile shares a column with a short tile, and each long tile's `content rows: N of M` shows more drawn rows than on main.

**Change-Review line:** Long and short tiles share a column, so the long ones show more rows | On natedev, run cargo-handler with two long and two short tiles | The longest tile sits in a column with the shortest, the next longest with the next shortest; the long tiles show more of their rows

**Files:**
- `crates/tui_pane/src/tiles/grid/` — the placement rule
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/terminal.rs` — full row counts and the rule selection
- `crates/cargo-handler/src/render/` — the pairing and rows-drawn tests
- `docs/cargo-handler/as-built/tile-layout.md` — the column assignment

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test` passes for cargo-handler, tui_pane and cargo-tile with all listed tests, and `bash ~/.claude/scripts/direct/verify.sh lint` passes for each. The live check shows the long tiles paired with the short tiles.

### Phase 5 — Producers get their own tile, titled "producers"; they leave the unassigned tile  · status: todo

**Depends on:** Phase 1 (it adds the `producer` role from the staff source)

#### Work Order

**Goal:** every producer session draws in one tile titled `producers`. No producer draws in the unassigned tile.

**Source:** the user, via sdlc and the showrunner, 2026-11-11: "cargo-handler should have a producers cell - that these should not show up in unassigned anymore and the title for that cell should just be \"producers\"".

**Spec:**

Director decisions:
1. A session is a producer only when the staff source names it so (`StaffKind::Producer`, the Phase 1 `producer` role). No name match.
2. One producers tile holds the producers of every configured machine, with the stable identity `TileKey::Producers`. It uses the shared tree, columns, sort, selection and details view. Each producer is a root row, and its subagents and threads draw under it. Row keys keep the machine, so equal producer names on two machines stay two rows.
3. The title is `producers`, a constant beside `UNASSIGNED_CELL_TITLE`.
4. The producers tile draws when the census holds at least one producer session. Account status does not decide it: a producer with no status entry still draws. The tile sits before every unassigned tile.
5. The top-level agents table does not change: it lists the same rows as on main. A producer draws once, in the producers tile, and in no unassigned or hierarchy tile.

**Constraints from prior phases:**
- Phase 1 built `StaffKind::Producer`, `ProducerMembership`, the `producer` role word and the schema-3 transport fields. Use them. Do not infer a producer from a name, its descendants or its status.
- `UnassignedEntry` and `group_tiles` (`agent_cell.rs`) own pooling today. Make the pool kind an enum (unassigned or producers), not a bool or a bare `Option`.
- Keep `staff_run_keeps_one_waiting_worker_without_a_launcher_or_open_thread` green. A worker with a live detached run draws under its producer when the producer owns it, and its `d` view shows the run step.

Tests (red first, each under one second):
1. A producer session draws in the producers tile and not in the unassigned tile.
2. A session that staff does not name a producer stays in the unassigned tile.
3. With no producer session, no producers tile draws. Removing the last producer removes the tile.
4. The producers tile title reads `producers`.
5. Producers from two machines share one tile; equal names stay distinct rows.
6. A producer with no status entry still draws in the producers tile.
7. Sort, expansion, selection, header clicks and `d` work in the producers tile after its membership changes.

**Live check (director):** before the checkpoint, capture the producers tile and the unassigned tile from a build of this branch and from main, under the safety rule.

**Change-Review line:** Producers have their own tile titled producers | On natedev, run cargo-handler in a terminal and look for the producers tile | A tile titled producers lists the producer sessions; none of them shows in the unassigned tile

**Files:**
- `crates/cargo-handler/src/census/placement.rs` — tile placement
- `crates/cargo-handler/src/census/hierarchy.rs` — producer staff rows
- `crates/cargo-handler/src/constants.rs` — the tile title
- `crates/cargo-handler/src/render/mod.rs` — tile order
- `crates/cargo-handler/src/agent_cell.rs` — pooled entries, grouping, keys, order and tree
- `crates/cargo-handler/src/tiles.rs` — `TileKey::Producers`
- `crates/cargo-handler/src/interaction.rs` — selection and sort identities
- `crates/cargo-handler/src/render/painted_surface.rs` — painted headers and row input
- `crates/cargo-handler/src/expansion.rs` — producer tree expansion
- `crates/cargo-handler/src/terminal.rs` — display deadlines for the new pool
- `docs/cargo-handler/as-built/hierarchy.md` — the producers tile
- `docs/cargo-handler/as-built/tile-layout.md` — producer grouping and identity

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows the producers tile and no producer in the unassigned tile.

### Phase 6 — Claude rows show their effort, and a long shell command does not widen the session column  · status: todo

**Depends on:** Phase 1 (the new column order); Phase 3 (the `d` view sections and its `effort:` line)

#### Work Order

**Goal:** every tile with room draws the effort column for Claude and Codex rows. A shell command row takes no more width than the agent rows beside it.

**Source:** the user, via the showrunner, 2026-11-11 (C263): "there are a few problems with the current setup so think about these 4 the future: why does effort only show in the unassigned when the rest have plenty of space - also we shouldn't allow a long python script description to take up so much space as it does in the natedev showrunner." Screenshot: `/tmp/claude/scratch/until-2026-11-15T0500Z-handler-shot-c263/handler_c263_followup.png`.

**Spec:**

Director decisions:
1. Cause of the effort gap: Claude writes `effort` at the top level of each assistant record (`{"type":"assistant","effort":"high","message":{...}}`). `ConversationModel::take_claude` reads effort from `message`, so no Claude row has an effort, and the fitter drops the empty column. Codex rows record effort, so only the unassigned tile drew it. Fix the reader: take the model from `message.model`. Take effort from the top-level record first, then from the nested effort fields that `take_claude` reads today. Keep the system-record metadata and the effort aliases. A later record with no effort keeps the recorded one.
2. The effort column keeps its place in the drop order. No column-set change. With effort recorded, it draws wherever the tile has room.
3. A shell command row's session cell cuts each path argument to its file name: `python3 /tmp/…/scratchpad/watchgates.py 2>&1` draws `python3 watchgates.py 2>&1`.
4. The session column width comes from the agent rows of the tile. A shell row wider than that width ends in `…` at the column edge. The details view `name:` line keeps the full command.

**Constraints from prior phases:**
- Phase 1 shows the recorded model and effort in the `d` view. Use `ModelName` and `ReasoningEffort::Unrecorded`. JSON `Option` values stay inside the decode step; add no optional effort field.
- `TreeLine::name` feeds both the drawn cell and the `d` view `name:` line. Keep the full command for the `d` view and for selection identity. Use a separate display text for the short shell form.
- Leave shell text out of the preferred session width before the shared metrics and layout demand run. The minimum width still holds the tree guides and the session header. Cut on grapheme boundaries and keep span styles.
- The cut keeps quotes, spaces, redirections and arguments that are not paths.

Tests (red first, each under one second):
1. A Claude assistant record with a top-level `effort` gives that effort; its `message.model` gives the model.
2. A Claude tile with recorded effort draws the effort column at a width that fits it.
3. A path argument draws as its file name; other arguments are unchanged.
4. A shell row longer than the widest agent row does not widen the session column, and it ends in `…` at the exact column edge. Neighbor tiles keep their widths.
5. The details view of that shell row, opened with `d`, shows the full command.
6. Effort reading: top-level first, nested fallback, system records, aliases and a missing effort. Bounded first reads, unchanged-file reuse, appends and file replacement keep working.
7. Quoted paths with spaces, relative paths and redirections cut correctly; other arguments are unchanged.
8. The `d` view of a Claude row with recorded effort shows that effort on its `effort:` line in the agent section.

**Live check (director):** before the checkpoint, capture the natedev showrunner tile and the hana showrunner tile from a build of this branch and from main, under the safety rule.

**Change-Review line:** Claude rows show their effort, and a long shell command no longer pushes the columns to the right | On natedev, run cargo-handler and look at the natedev and hana showrunner tiles | The effort column shows on Claude rows where the tile has room; a shell command shows its script name and ends in … at the session column edge; d shows the full command

**Files:**
- `crates/cargo-handler/src/census/model.rs` — `ConversationModel::take_claude`
- `crates/cargo-handler/src/agent_cell.rs` — the shell row text and the session column width
- `docs/cargo-handler/as-built/hierarchy.md` — the effort source and the shell row text
- `docs/cargo-handler/as-built/tile-layout.md` — the session column width
- `crates/cargo-handler/src/census/transcript.rs` — metadata reader tests
- `crates/cargo-handler/src/render/details_tests.rs` — full shell command in the `d` view
- `crates/cargo-handler/src/render/narrow_width_tests.rs` — shell width and neighbor tiles
- `docs/cargo-handler/as-built/startup.md` — Claude effort sources

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows effort on Claude rows and a shell row that does not widen the session column.

### Phase 7 — Reset times in the accounts footer take three clearly different colors  · status: todo

**Depends on:** none

#### Work Order

**Goal:** the soonest, middle and latest reset times in the accounts footer each draw in a different hue, so the step is plain at a glance.

**Source:** the user, via the showrunner, 2026-11-11, on C264 (returned): "they differ but it's still preetty subtle - i would prefer there being true  color differences". C264 is phase 53 of the earlier handler plan: three steps blended from `reset_nearest` to `reset_farthest`. Dark themes step from green to teal; light themes step in violet lightness.

**Spec:**

Director decisions:
1. Three theme roles replace the two-end blend: `reset_soonest`, `reset_middle` and `reset_latest`. Each theme file sets all three. No sRGB midpoint. `reset_nearest` and `reset_farthest` go.
2. Each step is a different hue family: soonest green (the quota comes back soon), middle gold, latest blue. No step is red.
3. `ResetRank::color` keeps its rank rule: the first quarter of ranks takes `reset_soonest`, the last quarter takes `reset_latest`, and the rest take `reset_middle`. A single reset takes `reset_soonest`.
4. Every pair of steps is at least 20 CIEDE2000 apart, in every built-in theme, with the pane tint on and off. This replaces the 5.0 floor of phase 53, which let the user see only a subtle change.
5. The other phase 53 rules stay: each step stays readable on its ground after `readable_status_style`, and no step matches a session name color, at the measured floors that the tests hold.

**Constraints from prior phases:**
- Keep `ResetRank` as the checked rank type, with its strict quarter edges and its single-reset rule. Measure the drawn colors, not only the theme role values. The current gradient test allows four steps and checks only neighbor pairs.
- `accounts.md` names the new keys that replace `reset_nearest` and `reset_farthest` for custom themes.

Tests (red first, each under one second):
1. Each built-in theme file sets the three roles, and a theme file can override each one.
2. The rank rule gives soonest, middle and latest in order; a single reset gives soonest.
3. A footer with all three rank groups draws exactly three reset colors. Every pair is at least 20 CIEDE2000 apart after `readable_status_style`, in every built-in theme, tint on and off.
4. The existing contrast, no-red and session-color tests pass with the new colors.
5. Equal reset times, one distinct reset and exact quarter edges keep the rank rule. A custom theme without the new roles takes the defaults.

**Live check (director):** before the checkpoint, capture the accounts footer with three or more distinct reset times from a build of this branch and from main, on the default theme and on one light theme, under the safety rule.

**Change-Review line:** Reset times in the accounts footer take three clearly different colors | On natedev, run cargo-handler and look at the resets column of the accounts footer | The soonest resets are green, the middle ones gold, and the latest ones blue, on dark and light themes

**Files:**
- `crates/cargo-handler/src/theme/mod.rs` — `ResetRank::color`, the theme role set
- `crates/cargo-handler/themes/*.toml` — the three roles in each built-in theme
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — step order and distance
- `crates/cargo-handler/src/theme/files_tests.rs` — theme file roles
- `crates/cargo-handler/src/theme/files.rs` — custom theme role load and defaults
- `docs/cargo-handler/as-built/accounts.md`, `docs/cargo-handler/as-built/colors.md` — the reset steps

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows three different hues in the resets column on both themes.

### Phase 8 — Enter shows the focused tile on the full screen, shared with cargo-tile through tui_pane  · status: todo

**Depends on:** Phase 3 (both change the handler key handling)

#### Work Order

**Goal:** in cargo-handler, Enter shows the focused tile on the full screen, and Esc returns to the grid. cargo-tile and cargo-handler use one full-screen tile implementation in tui_pane.

**Source:** the user, change review C219 (returned), via the showrunner, 2026-11-12: "i like it and it works on cargo tile and this should be shared via tuii_pane to work on cargo handler". C219 shipped the cargo-tile full-screen cell in merge 35a8bccf. Showrunner: move the code into tui_pane with no copy; cargo-tile keeps its behavior and its tests; the handler unit may edit cargo-tile after it reserves the files with berth; check that Enter is free in the handler grid and report it before the build.

**Spec:**

Director decisions:
1. Key conflict, reported to the showrunner on 2026-11-12: Enter is bound in cargo-handler today. It toggles the selected row (`global.toggle_row = enter`), and Right and Left also open and close rows. Director proposal: Enter takes the full-screen tile, as in cargo-tile. Right and Left keep row open and close. `toggle_row` stays an action with no default key. The showrunner confirms this before the build.
2. `crates/cargo-tile/src/grid_view.rs` (the expanded cell identity, scrolling, motion clock and its Esc key) moves into tui_pane as one shared type. cargo-tile and cargo-handler both use it. No part is copied.
3. cargo-tile's Enter, Esc, scroll and motion keep their current behavior.
4. In cargo-handler, the full-screen tile draws the same tree, columns and selection as the grid tile, at the full pane size. Row keys, `d` and word clicks work in it as in the grid.
5. Reserve the cargo-tile files with `cargo-berth` before the first edit.

Tests (red first, each under one second):
1. In cargo-handler, Enter on a focused tile draws that tile at the full pane size; Esc returns to the grid with the same focus and selection.
2. Right and Left still open and close the selected row.
3. cargo-tile's existing expand, Esc and scroll tests pass unchanged.
4. The shared tui_pane type has its own tests for expand, collapse and scroll.

**Live check (director):** before the checkpoint, capture cargo-handler's grid, the full-screen tile after Enter, and the grid after Esc, from a build of this branch, under the safety rule.

**Change-Review line:** Enter shows the focused tile on the full screen in cargo-handler, as in cargo-tile | On natedev, run cargo-handler, focus a tile and press Enter, then Esc | Enter fills the screen with the focused tile; Esc returns to the grid; Right and Left still open and close rows

**Files:**
- `crates/tui_pane/src/` — the shared full-screen tile type
- `crates/cargo-tile/src/grid_view.rs`, `app.rs`, `render.rs`, `render/frame_preparation.rs`, `constants.rs` — use the shared type (reserved with berth first)
- `crates/cargo-handler/src/globals.rs`, `keymap.rs`, `app.rs`, `render/mod.rs` — Enter and Esc, and the full-screen draw
- `docs/cargo-handler/as-built/interaction.md`, `docs/cargo-tile/as-built/` — the keys

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test` passes for cargo-handler, tui_pane and cargo-tile with all listed tests, and `bash ~/.claude/scripts/direct/verify.sh lint` passes for each. The live check shows the full-screen tile and the return to the grid.

### Phase 9 — The word "waiting" draws in red in every tile  · status: todo

**Depends on:** none

#### Work Order

**Goal:** the word `waiting` draws in its own red ink in every tile, so a waiting row stands out from busy rows.

**Source:** the user, via the showrunner, 2026-11-12: "waiting should show in some waiting color like red". Showrunner context: in the user's screenshot of the cargo-liner tile, the handler and tile unit rows read `waiting` in the same orange as the other unit words. Required: a red or near-red ink in every tile, readable on dark and light themes, with the color in the theme config and no color value in code.

**Spec:**

Director decisions:
1. A new theme role, `waiting`, sits under `[variants.roles]` in each built-in theme file (`default_dark.toml`, `default_light.toml`, `high_contrast.toml`, `starter.toml`). Its value is red or near red. No color value goes in code.
2. A custom theme without `waiting` takes the value from the built-in theme it extends, as the other roles do today.
3. The activity word `waiting` (`Activity::Waiting`, `census/activity.rs`) draws in the `waiting` ink in every tile kind. All other activity words keep their inks.
4. `waiting` is at least 4.5:1 against its drawn ground in every built-in theme, with the pane tint on and off and on a selected row. It differs from the `busy`, `idle` and `unreachable` inks, so `waiting` does not read as an unreachable account.

Tests (red first, each under one second):
1. A row whose activity is `waiting` draws that word in the `waiting` ink, in a showrunner tile and in a unit tile, read from the rendered buffer.
2. A row with any other activity keeps its current ink.
3. `waiting` is at least 4.5:1 on its ground in every built-in theme, tint on and off, selected and not selected.
4. Each built-in theme file sets `waiting`, and the value differs from `busy`, `idle` and `unreachable`. A custom theme without it takes the built-in value.

**Live check (director):** before the checkpoint, capture the cargo-liner tile from a build of this branch, on the dark and the light default themes, under the safety rule. A unit row that reads `waiting` draws it in red.

**Change-Review line:** The word waiting draws in red in every tile | On natedev, run cargo-handler while a unit waits, on the dark and the light theme | Each waiting row shows the word waiting in red; the other words keep their colors

**Files:**
- `crates/cargo-handler/themes/*.toml` — the `waiting` role
- `crates/cargo-handler/src/theme/mod.rs`, `crates/cargo-handler/src/theme/files.rs` — the role
- `crates/cargo-handler/src/agent_cell.rs` — the activity cell ink
- `crates/cargo-handler/src/agent_cell/tests/design_ink_tests.rs` — the ink and contrast tests
- `docs/cargo-handler/as-built/colors.md` — the `waiting` ink

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows `waiting` in red on both default themes.

### Phase 10 — A worker that waits on a detached run reads "waiting" in its status cell  · status: todo

**Depends on:** Phase 3 (it uses the Phase 3 column widths); Phase 9 (it uses the `waiting` ink); sdlc's `waiting` state in `session_state.py` `classify` and the accounts report. This phase starts when sdlc sends that commit hash.

#### Work Order

**Goal:** a worker with a live detached run reads `waiting` in its status cell, in the Phase 9 `waiting` ink.

**Source:** the user, change review C324 (the C269 re-check, returned), via the showrunner, 2026-11-12: "so - for c269 - i'm not sure that this is useful to read busy when it really means waiting - i would like to see waiting be the status column value if that is what a worker is doing". sdlc adds a fifth state, `waiting`, to `session_state.py` `classify` and the accounts board.

**Spec:**

Director decisions:
1. `SessionState` (`crates/cargo-handler/src/census/accounts.rs`) adds `Waiting`, read from the accounts report word `waiting`. `ReportedSessionState` and the status cell draw it. cargo-handler makes no state of its own: the word comes only from the report.
2. The status word `waiting` draws in the Phase 9 `waiting` ink. This phase adds no theme role.
3. The status column width comes from the words it draws. A tile that draws `waiting` fits the longer word with the Phase 3 drop order unchanged.
4. The details view `state:` line keeps the detached run's step.

Tests (red first, each under one second):
1. The accounts report word `waiting` reads as `SessionState::Waiting`; an unknown word stays unavailable, as today.
2. A worker row with a `waiting` entry draws `waiting` in the status cell, in the `waiting` ink, read from the rendered buffer.
3. A tile with a `waiting` row draws the whole word, and account, role, activity and status all draw at the Phase 3 C320 width.

**Live check (director):** before the checkpoint, start one detached `verify.sh` run from a worker under the safety rule. Capture that worker's row from a build of this branch while the run lives: its status cell reads `waiting`.

**Change-Review line:** A worker that waits on a detached run reads waiting | On natedev, run cargo-handler while a worker waits on a verify run, and look at its row | The status cell reads waiting in red

**Files:**
- `crates/cargo-handler/src/census/accounts.rs` — `SessionState::Waiting`
- `crates/cargo-handler/src/agent_cell.rs` — the status cell ink
- `crates/cargo-handler/src/agent_cell/tests/design_ink_tests.rs`, `column_visibility_tests.rs` — the ink and the width
- `docs/cargo-handler/as-built/hierarchy.md` — the fifth status word

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows `waiting` on a worker with a live detached run.
