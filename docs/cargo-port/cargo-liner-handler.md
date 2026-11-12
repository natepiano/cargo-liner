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

**Binds later work:** the producers tile uses the same columns (account first, role `producer`, activity, status from the accounts session list) and the same `d` view. The session column sits second, after account. The `d` view lists model and effort when a tile drops them.

**Gotchas:**
- Draw, expansion and display deadlines all build `AgentPlacement` with `app.accounts.table.session_states()`; empty states make a status-sorted tile expand the wrong row.
- Some Codex producers (brain, morty) have no entry in the accounts session list, so their status cell is empty until sdlc exports them.
- A worker whose turn ended reads `idle` while its detached run lives until sdlc's `classify` makes such a session `busy`; cargo-handler adds no rule of its own.

**Ruled out:** a copy of `classify` in cargo-handler (the tile and the accounts table must not disagree); the old `mesh_runs/` names (`mesh_runs.py` is deleted); producers in the top-level agents table (not asked).

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

### Phase 3 — Producers get their own tile, titled "producers"; they leave the unassigned tile  · status: todo

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

### Phase 4 — Claude rows show their effort, and a long shell command does not widen the session column  · status: todo

**Depends on:** Phase 1 (it uses the details view and the new column order)

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

### Phase 5 — Reset times in the accounts footer take three clearly different colors  · status: todo

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
