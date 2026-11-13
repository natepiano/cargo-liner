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
- **Live checks:** a live check uses only terminals and handler processes the worker starts, with each pid from its own launch chain. Captures open as split panes in the unit tab, never as new tabs, and each pane closes when its capture ends. A scratch `XDG_CONFIG_HOME` holds a symlink `herdr` → `~/.config/herdr`, or the handler loses the herdr socket and draws a broken screen.
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

#### As-built

- The `[word_highlight] emphasis` setting in the cargo-handler config takes `subtle`, `clear` or `strong`. The default is `clear`. `subtle` is the earlier look. `clear` doubles the distance of the match ground from the focus ground. `strong` uses the `clear` ground and makes matched cells bold; `subtle` and `clear` add no modifier.
- `WordHighlightEmphasisVisitor` names the three values for any wrong TOML type.
- `config_watch.rs` runs a notify reader thread on the config directory. It sends `WordHighlightUpdate`, and the UI loop drains it and repaints the retained bodies. `ConfigReaderThread` joins on drop.
- `WordHighlightConfigStatus { Current, Rejected(String) }` in `app.rs`: an invalid value keeps the last emphasis. The settings pane shows `Rejected` and `ConfigError::Write`. A later valid file clears the error.
- `MatchGroundsCache` and `MatchPaletteState` key on the theme, the ground, the selected word and the emphasis. `WordMatchGrounds` keeps separate grounds for plain rows, focused selections and unfocused selections, under the `MatchInk::resolve` readability rules.
- tui_pane `LoadedConfig.error` is an `Option<ConfigError>` with `Parse` and `Write` cases, so a startup parse error clears when a valid file arrives. `load_from` is public.

**Files:**
- `crates/cargo-handler/src/config_watch.rs` — the config reader thread
- `crates/cargo-handler/src/config.rs` — the emphasis setting and its visitor
- `crates/cargo-handler/src/theme/word_match.rs` — the `clear` and `strong` grounds
- `crates/cargo-handler/src/app.rs` — `WordHighlightConfigStatus`
- `crates/tui_pane/src/app_config/loaded.rs` — `ConfigError`

**Binds later work:** `WordHighlightConfigStatus::Current` hides the handler notice and `Rejected` shows it; a `ConfigError::Write` shows on its own; cargo-tile shows both `ConfigError` cases. A new ink keeps readability and its color difference at every emphasis, tested in `render/word_contrast_tests.rs`. A change to the highlight keeps the retained-body repaint.

**Gotchas:**
- A symlinked `config.toml` whose target is outside the config directory gets no live update.
- A reader setup failure shows its cause and ends the reader. Recovery needs a restart.

### Phase 3 — Tile columns read session, account, role, activity, status, branch; a long session name is cut before those columns drop; the details view groups its lines into sections, aligns them and copies a value with y  · status: done

#### As-built

- The tile tree column order is session (with tree guides), account, role, activity, status, branch, then type, model, effort and directory. The top-level agents table did not change.
- A narrow tile drops effort, model, directory and type, then branch. Then it cuts long session names with `…` on grapheme boundaries, down to the session minimum (the deepest tree guides or the header). Only then does it drop account, role, activity and status, in that order. `ColumnDrop::ORDER` and `GroupFitStep::ORDER` hold this order.
- Each screen x lays out from the union of its tiles' column sets. A tile that lacks a column keeps a blank slot. `TreeColumnAlignment::Fitted { widths, spacing, slots }` carries the drawn widths; `TreeMetrics.widths` keeps the measured demand widths. `tree_header` calls `fill_tree_width` only for `TreeSizing::Local`; `align_screen_columns` fills shared tiles at the smallest tile width at that x.
- The `d` view groups its lines into `agent`, `hierarchy`, `location` and `now` sections, each with a heading line and one blank line between sections. `program` is now `family`, and `effort:` always shows (`—` with no value). Values start in one column after the longest label, with a one-space margin each side.
- `hierarchy_details` maps `RoleLabel::Stated` and `Inferred` through `RowRole::from_hierarchy` to `showrunner:`, `unit:` and `worker:` lines. Only `Unlabeled` and `ReviewFallback` rows show `name:`.
- Up and Down move a selected line and skip headings and blank lines. `y` copies the value only. The generic copy outcome and its notice moved from cargo-port into tui_pane, with no copy. `DetailsOverlay` registers its own copy target, and copy notices draw above an open overlay.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — column order, drop order, session cut, union slots, `TreeColumnAlignment`
- `crates/cargo-handler/src/details.rs` — sections, `family`, value column, selection and `y`
- `crates/cargo-handler/src/render/mod.rs` — copy notices above the overlay
- `crates/tui_pane/src/copy.rs` — the shared copy outcome and notice
- `crates/cargo-port/src/` — the yank path now calls tui_pane
- `docs/cargo-handler/as-built/interaction.md`, `hierarchy.md`, `tile-layout.md` — the `d` layout, `y`, column and drop order

**Binds later work:** a layout change keeps the union slots and the Fitted widths, and keeps `converging_column_groups_share_every_drawn_header_and_age_right_edge` and `different_tile_columns_share_core_positions_and_click_the_drawn_headers` green. A new copy surface keeps `details_navigation_skips_headings_and_y_copies_only_the_value` and `assert_full_frame_copy_notice`. A producer row shows `name:` and `role: producer` in its `d` view. A shell row never sets the preferred session width.

**Gotchas:**
- Measured demand widths and fitted drawing widths differ. Exclude a row from measurement before fitting, not after.
- `Framework::copy_selection` resolves the Main focus, and `DetailsOverlay` does not change that focus. A new overlay with `y` must register its own copy target.

**Ruled out:** an alphabetical `d` order (the user replaced it with sections); a cargo-handler copy of cargo-port's yank code; one shared session width without union slots (it misaligned tiles with different column sets).

### Phase 4 — Long and short tiles share a column, so the long ones show more rows  · status: done

#### As-built

- The shared tui_pane grid places detail tiles by a rule that the app selects. `TileColumnOrder<Id>` has `SourceOrder` and `LongestWithShortest(Vec<(Id, usize)>)`. `TileCells::column_order()` returns `SourceOrder` by default, so cargo-tile keeps source order.
- cargo-handler returns `LongestWithShortest` from each tile's full tree rows (`full_tree_rows`): every row with every row open, plus the root row, taken before expansion removes closed rows. Collapse, scroll and pane height do not change it.
- `TileGrid` holds `ColumnRanks`. `ColumnRanks::arrange(slots, column_capacities, depth)` deals the ranks, longest first, over the columns in snake order (`SweepDirection`). A full column is skipped. The summary tile and empty cells keep their slots. Inside a column, tiles keep source order. A tie keeps the previous rank.
- `CompleteColumnRanks::for_slots` builds one unique rank list against the admitted slots: it drops duplicates and ids without a slot, and puts slot ids absent from the order last, in source order.
- `ColumnLayout::column_capacities(detail_slots, columns)` is the one capacity calculation.
- Grid geometry is pane size, column count and summary depth. With the same membership and geometry, a tile moves only when the ranking changes. A change to membership or geometry re-deals the unchanged ranks over the new slot shape. Grid motion animates each move.

**Files:**
- `crates/tui_pane/src/tiles/grid/column_order.rs` — `TileColumnOrder`, `ColumnRanks`, `CompleteColumnRanks`, `SweepDirection`
- `crates/tui_pane/src/tiles/grid/tile_grid.rs` — holds `ColumnRanks` and applies it at layout
- `crates/tui_pane/src/tiles/grid/layout.rs` — `ColumnLayout::column_capacities`
- `crates/cargo-handler/src/agent_cell.rs`, `crates/cargo-handler/src/render/mod.rs` — full tree rows and the rule selection
- `crates/tui_pane/src/tiles/grid/column_order_tests.rs`, `ranked_column_grid_tests.rs`, `crates/cargo-handler/src/render/ranked_column_tests.rs` — placement tests
- `docs/cargo-handler/as-built/tile-layout.md` — the column assignment

**Binds later work:** the full-screen tile keeps `ColumnRanks` across expand and collapse; it does not clear or rebuild them. A new detail tile, such as the producers tile, takes part in the ranking.

**Gotchas:** the summary depth is part of the grid geometry. A collapse that changes the summary depth changes the column capacities, so tiles can move with unchanged ranks.

### Phase 5 — The word "waiting" draws in bright yellow in every tile  · status: done

#### As-built

- The theme role `waiting` (`Role::Waiting`, `SessionRowInk::Waiting`, `WAITING_ROLE` in `constants.rs`) sits under `[variants.roles]` in each bundled theme file. Dark themes set (255, 255, 0). Light themes set (122, 122, 0). No color value is in code.
- `ReportedSessionState::color_role` (`census/accounts.rs`) maps `Reported(SessionState::Waiting)` to `Role::Waiting`. Tree status cells, summary cells and headings take the ink from it. The status word stays the accounts-board word. All other words keep their inks.
- `files::bundled_role_styles()` is one `LazyLock` over the bundled TOMLs. It keeps `mauve`, `astra_reviewer` and `waiting`. `bundled_roles()` in `theme/mod.rs` reads it. `RolePalette::spec` is not `const`, because `Role::Waiting` reads the `LazyLock`.
- A custom theme without `waiting` takes the value of the built-in theme it extends. A custom theme can set `waiting` under `[variants.roles]`.

**Files:**
- `crates/cargo-handler/themes/*.toml` — the `waiting` values
- `crates/cargo-handler/src/theme/mod.rs` — `Role::Waiting`, `bundled_roles`, the theme tests
- `crates/cargo-handler/src/theme/files.rs` — `bundled_role_styles`
- `crates/cargo-handler/src/theme/files_tests.rs` — the inherit and override test
- `crates/cargo-handler/src/census/accounts.rs` — `color_role` maps a waiting status
- `docs/cargo-handler/as-built/colors.md` — the waiting status ink

**Binds later work:** the details view takes the status ink through `ReportedSessionState::color_role` and `Role::style()`, as the tile does. `census --json` prints the status word `waiting`, with no ink.

**Gotchas:** a configured status ink is not the drawn ink on light themes. `footer_colors::readable_status_style` measures antialiased glyph contrast over the base, unfocused and focused grounds. It draws the light `waiting` as (75, 75, 0) and the light `busy` as (0, 89, 0). A test compares rendered buffers, never TOML values.

**Ruled out:** an activity-text or exact-text `waiting` ink rule. The role enums have no waiting variant, so the status cell is the one path.

### Phase 6 — The details view shows activity and status, drops family, and uses the tile colors  · status: done

#### As-built

- The `d` view has four sections. `agent` shows account, model, effort and type; it has no family line. `now` shows `activity:`, `status:` and `age:`, in that order; it has no state line.
- `activity:` joins the job text (`TreeLine::activity_text`) and the live word (`AgentActivity::text`) with `HEADING_SEPARATOR`, for example `impl · clippy`. Equal parts show once (`compacting`). The live word is left out when it is `idle` or `—`. No job and no live word gives `—`. The tile activity cell does not join the live word.
- `row_details_with_inks` (`agent_cell.rs`) draws each value with a tile cell through `tree_value_line` and the tile column, so a `d` value has its tile ink, the `waiting` ink included. A selected line goes through `footer_colors::SelectedGroundInk::resolve`. Values with no tile cell (pid, desktop, age, hierarchy names) keep the default ink.
- `DetailsOverlay::click` (`details.rs`) selects a value line through `tui_pane::table_row_at` on the rendered viewport and the painted value rows. A click on a heading, a blank line, a border or outside the overlay changes nothing. `y` copies the value text only.
- `FavoritesOverlay::click` and `scroll` (`tui_pane` `favorites_overlay/mod.rs`) select and scroll rows as the arrow keys do. `App::click` and `App::scroll` route to the details view, then the favorites overlay, then the tiles. Tiles, summary agents and accounts, settings and keymap already used the shared mouse path.

**Files:**
- `crates/cargo-handler/src/details.rs` — the properties, the `now` order, the click and the painted rows
- `crates/cargo-handler/src/agent_cell.rs` — `TreeLine::details`, `row_details_with_inks`
- `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/interaction.rs` — click and wheel routing; an open details view is modal
- `crates/tui_pane/src/favorites_overlay/mod.rs` — favorites click and scroll
- `crates/cargo-handler/src/render/details_tests.rs`, `crates/cargo-handler/src/render/favorites_tests.rs` — the `d` view, ink and click tests
- `docs/cargo-handler/as-built/hierarchy.md`, `docs/cargo-handler/as-built/interaction.md` — the `d` lines, inks and mouse parity

**Binds later work:** the full-screen tile keeps the overlay order for clicks and the wheel: the details view, then the favorites overlay, then the tiles. `census --json` prints the tile activity text, not the joined `d` view text.

**Gotchas:** `table_row_at` treats the details table's top border as its header row. The details tests read non-ASCII cells (`·`), so a buffer helper compares symbols, never bytes.

### Phase 7 — The census keeps each worker's current run, pane state, run states and activity start, and names what it cannot read  · status: done

#### As-built

- `StaffFields` (`census/hierarchy.rs`) reads the staff row's run pointer as `StaffRunPointer::{Current(RunId), NoRun}`. A unit lists only the workers of its current run. The run's pass map (`progress_history_state.json`, `PROGRESS_HISTORY_STATE_FILE`) names them: a pass object with `instance_id` is one record, else its values are the records. The newest active pass gives a worker's start.
- `DetachedRunFact` (`census/activity.rs`) keeps each current-run record with `DetachedRunState::{Running, FinishedQueued, Missing}`. The tile activity text comes only from validated running records. One malformed record does not hide its siblings; the runs fact is unknown with the first failure.
- `PaneState::{Open, Stopped, Closed}` (`census/herdr.rs`) keeps the pane lifecycle apart from the tile status; an unread pane is `Observed::Unknown`.
- `Observed<T>::{Known(T), Unknown(UnknownReason)}` and `SourceRead<T>::{Loaded, Absent, Unreadable}` (`census/mod.rs`) keep failure apart from empty. `CensusFacts` on `HierarchySnapshot.facts` holds `staff` and `accounts` as `Observed<SourceHealth>` and `workers` as a map of `WorkerFacts` (`current_run`, `pane`, `runs`, `activity_started`, epoch seconds).
- The probe report carries the facts with serde defaults. An older remote gives its rows, with each fact unknown as "remote probe has no ...". A local snapshot built before its read says "this snapshot has no ...".
- `terminal.rs` passes the accounts monitor's table to `Census::record_local_accounts`; until then the accounts fact reads "the accounts monitor has not answered".

**Files:**
- `crates/cargo-handler/src/census/hierarchy.rs` — `StaffRunPointer`, the typed staff read
- `crates/cargo-handler/src/census/activity.rs` — run states, the activity start, run record ownership
- `crates/cargo-handler/src/census/herdr.rs` — pane states
- `crates/cargo-handler/src/census/scan.rs` — `CurrentRunObservations`, the pass map, the current-run rule
- `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/probe.rs` — `Observed`, `CensusFacts`, the probe fields, `record_local_accounts`
- `crates/cargo-handler/src/census/accounts.rs` — the typed accounts read
- `crates/cargo-handler/src/terminal.rs` — records the accounts outcome
- `docs/cargo-handler/as-built/hierarchy.md` — the current-run rule, the pass map and the facts

**Binds later work:** `census --json` reads `CensusFacts` and `WorkerFacts` inside `census`, calls `record_local_accounts` with its own accounts read, and prints each unknown reason. The accounts `login ends` value goes into `AccountsReading`.

**Gotchas:** a missing or unreadable pass map gives a director no workers and an unknown start with the reason; it never restores workers from older runs. Remote snapshots expect `CensusFacts::older_probe()` reasons, local ones the `Default` reasons.

### Phase 8 — `cargo-handler census --json` prints the census the TUI shows, so other tools read the same worker facts  · status: todo

**Depends on:** Phase 6 (it changes `agent_cell.rs`, whose activity text this command prints); Phase 7 (the census facts this command prints)

#### Work Order

**Goal:** one command prints, as JSON, the unit directors and workers the TUI shows, with the same status, activity and run facts, so `/showrunner:units` and cargo-handler give one answer.

**Source:** the user, via the showrunner, 2026-11-13: "the /showrunner:units skill and cargo handler tui app should all be using the same informaton sources to find out what workers are working on - so we have consistency - eventually i might merge /showrunner:units into cargo handler but not now - so i want to make sure we prepare the way". Showrunner context: two readers exist today and gave different answers: the cargo-handler Rust census, and `~/.claude/scripts/direct/worker_runs.py` behind `/showrunner:units`. The showrunner's call: the cargo-handler census is the one source. unit-board in sdlc consumes the JSON.

**Spec:**

Director decisions:
1. Add a visible subcommand, `cargo-handler census --json`, in `cli.rs` beside the hidden `probe`. It scans once, prints one JSON document on stdout, and exits 0. It opens no terminal and draws nothing. Diagnostics go to stderr, as `probe` does.
2. The command builds its answer through the functions the TUI uses: the local scan, the configured remote probes, the hierarchy, the staff source and the accounts session states. The status word and the activity text come from the functions that fill the tile status and activity cells. Add no second reader and no copy of a rule. Every fact the JSON needs is in the census after Phase 7; a fact still missing goes into the census, never into the command alone.
3. The JSON has a top-level `schema` integer, starting at 1, `machines`, `directors` and `workers`. The field set is unit-board's, sent by the showrunner 2026-11-13. Times are epoch seconds. A field with no value is null, never a missing key.
   - Each machine: `name`, `reachable` (bool) and `error` (the error text, or null).
   - Each director: `staff_id`, `name`, `machine`, `current_run_id` (the staff run pointer), `status` (the tile status word, or null with no accounts entry), `activity` (the tile activity text), `activity_started_at`.
   - Each worker: `staff_id`, `name` (the display name), `parent_staff_id` (its unit), `machine`, `status`, `activity`, `activity_started_at` (the Phase 7 activity start).
   - Each worker's `pass`: `role` (`impl`, `test`, `fix` or `review`) and `phase` (the phase instance id), or null with no open pass.
   - Each worker's `pane`: `open`, `stopped`, `closed` or `unknown`.
   - Each worker's `runs`: each run of its unit's current run, with `label`, `started_at` and `state` (`running`, `finished_queued` for a finished run with a queued result, or `missing`).
   - `unknown`: on a director, worker or run, an object with `reason` text when a fact is a Phase 7 `Unknown`; null otherwise. A pane or run the census cannot read never reads as an empty list.
   Wait holders and wait reasons stay in the run records; the JSON does not carry them.
4. The serde types are the contract. Document each field in the type's doc comments and in a `census --json` section of `crates/cargo-handler/README.md`, with one example document. A field rename or removal increments `schema`. The domain types stay as Phase 7 made them; only the serde types are nullable.
5. A remote machine that does not answer gives a `machines` entry with `reachable: false`, its error text and no rows. An unreadable staff file or a failed accounts command gives the local `machines` entry its error and prints the rows that remain. In both cases the command exits 0. A bad configuration or a failed local process scan exits non-zero with the error on stderr.
6. The command reads no file on a UI thread, because it has none. It uses the configured probe and read timeouts.

Tests (red first, each under one second):
1. With a fixture census of one showrunner, one unit director and two workers, one with a detached run, the command's JSON lists each with every field in decision 3. The status and activity equal the tile cell text for the same rows, read from the rendered tile.
2. Each run `state` value and each `pane` value comes out of a fixture that holds it. A Phase 7 `Unknown` fact gives `unknown` with its reason.
3. A row with no accounts entry has `"status": null`. A worker with no run has `"runs": []`.
4. A unit whose staff row names a current run, with an older open run beside it, lists only the current run's workers and runs.
5. An unreachable remote machine gives its `machines` entry with the error, and the command exits 0. An unreadable staff file gives the local entry its error, and the command exits 0. A bad configuration exits non-zero.
6. The JSON parses against the README example's field set: the same keys, the same types.

**Constraints from prior phases:**
- Phase 5: a waiting status reads `waiting`, from `ReportedSessionState`. The JSON gives the word; ink does not apply.
- Phase 6: the tile activity cell does not join the live word; only the `d` view does. The JSON `activity` is the tile cell text.
- Phase 7: the census keeps the staff run pointer (`StaffRunPointer`), the current-run rule, the run and pane states, the activity start, the typed source outcomes and the `Observed<T>` unknowns. The command reads them; it does not read the files again.
- Phase 7: `CensusFacts` (`census/mod.rs`, on `HierarchySnapshot.facts`) holds `staff` and `accounts` as `Observed<SourceHealth>`, and `workers` as `Observed<BTreeMap<session id, WorkerFacts>>`. `WorkerFacts` holds `current_run`, `pane` (`PaneState::{Open, Stopped, Closed}`; unknown is `Observed::Unknown`), `runs` (`DetachedRunFact` with `DetachedRunState::{Running, FinishedQueued, Missing}`) and `activity_started`. These types are private to `census`; put `report.rs` inside `census` so it reads them without new public API.
- Phase 7: the TUI writes the local `accounts` fact through `Census::record_local_accounts(&AccountsTable)`, called from `terminal.rs` after the accounts monitor reads. The command has no monitor. It runs the configured accounts read once and passes its table to `record_local_accounts`. Without that call, the fact stays unknown with the reason "the accounts monitor has not answered".
- Phase 7: a unit's current workers come from its current run's pass map (`progress_history_state.json`). A missing or unreadable pass map gives the director no workers and an unknown activity start with the reason; the JSON prints that reason in the director's `unknown`, never an empty worker list as a fact. A malformed run record gives `runs` an unknown with the first failure; the valid sibling runs still give the activity text.
- Phase 7: unknown reasons read "this snapshot has no ..." for a local snapshot built before its read, and "remote probe has no ..." for an older remote probe.
- The Delegation Context Status source: cargo-handler never makes a status word of its own.

**Live check (director):** on natedev, run the release build's `cargo-handler census --json` while this unit's workers run. Compare it with the TUI tile for the cargo-liner production: the same unit directors, workers, statuses, activities and runs. Measure the run time over five runs and name it in the phase report.

**Change-Review line:** cargo-handler census --json prints the workers the TUI shows | On natedev, run cargo-handler census --json while a unit works, and open cargo-handler | The JSON lists the same unit directors and workers, with the same status, activity and runs, as the tiles

**Files:**
- `crates/cargo-handler/src/cli.rs`, `crates/cargo-handler/src/constants.rs` — the subcommand
- `crates/cargo-handler/src/census/report.rs` — new: the JSON types and the shared build
- `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/scan.rs` — the shared scan entry for the command
- `crates/cargo-handler/src/agent_cell.rs` — the shared status and activity text
- `crates/cargo-handler/src/census/report_tests.rs` — new: the tests
- `crates/cargo-handler/README.md` — the schema section
- `docs/cargo-handler/as-built/hierarchy.md` — the command and its source

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check output matches the TUI, and the phase report names the measured run time.

### Phase 9 — The accounts table shows the date each Claude login ends  · status: todo

**Depends on:** Phase 7 (it changes `census/accounts.rs`, the typed accounts read)

#### Work Order

**Goal:** the accounts table has a `login ends` column, so the user sees when each Claude account must sign in again.

**Source:** the user, via natedev and the showrunner, 2026-11-13: "yes i want to see this in the accounts table (propagated to cargo handler)". "This" is the date each Claude account's login ends. The wire contract comes from natedev's accounts unit, through the showrunner.

**Spec:**

Wire contract (natedev's accounts unit):
- cargo-handler already runs `accounts.py status --stored --json` (`census/command_inputs.rs`) and reads `accounts[]`.
- Each `accounts[]` item gets `login_ends_at`: an ISO 8601 UTC string in the same form as `resets_at`, or null. Codex rows, and Claude rows with no stored value, give null.
- The value comes from `agent-notes/usage.json`, which cargo-handler already watches. No new input file.
- The field reaches `~/.claude` main with the accounts phase `login-end-date`. This phase can land first.

Director decisions:
1. `AccountWire` gets `login_ends_at: Option<String>` with `#[serde(default)]`. An older report without the key parses, and its rows show an empty cell.
2. `AccountUsage` keeps the value as `Reading<DateTime<Utc>>`, parsed by the function that parses `resets_at`. A malformed value fails the same way as a malformed `resets_at`.
3. A new `AccountsColumn::LoginEnds` sits after `Resets`. Its header is `login ends`, lower case like the other headers. Its value uses `reset_label`, the date form of the `resets` column. A null value gives an empty cell, as `resets` does.
4. The column sorts by date, as `resets` does, with an empty value in the place `resets` gives it.
5. The column draws in the default table ink. It takes no rank color.
6. On a narrow pane, the column drops after `enabled` and before `credits`. Director call: the date changes once a week, so it gives way before the credit amount.

Tests (red first, each under one second):
1. A report with `login_ends_at` on a Claude row and null on a Codex row draws the date in the Claude row and an empty cell in the Codex row, read from the rendered accounts table.
2. A report without the `login_ends_at` key parses, and every `login ends` cell is empty.
3. The `login ends` cell text equals the `resets` cell text for the same instant, today, within six days and later.
4. A click on the `login ends` header sorts the rows by that date, in both directions.
5. On a narrow pane, `enabled` drops first, then `login ends`, then `credits`.

**Constraints from prior phases:**
- Phase 7: the accounts monitor keeps `AccountsTable::{NotConfigured, AwaitingFirstReading, Ready(AccountsReading), Failed(AccountsFailure), FailedKeepingLast}` (`census/accounts.rs`). The census keeps only its health, through `Census::record_local_accounts`. The new field goes into `AccountsReading`; the health does not change.
- The Delegation Context Status source: sdlc owns the accounts report. cargo-handler reads the field and makes no date of its own.

**Live check (director):** after the `~/.claude` change merges, capture the accounts table from a build of this branch on natedev, under the safety rule. Each Claude row shows its login end date. Each Codex row shows an empty cell.

**Change-Review line:** The accounts table shows the date each Claude login ends | On natedev, run cargo-handler and look at the accounts table | A `login ends` column shows a date for each Claude account and is empty for Codex accounts

**Files:**
- `crates/cargo-handler/src/census/accounts.rs` — `AccountWire::login_ends_at`, `AccountUsage::login_ends_at`
- `crates/cargo-handler/src/sort.rs` — `AccountsColumn::LoginEnds` and its header
- `crates/cargo-handler/src/summary/accounts.rs` — the cell value, the sort and the drop order
- `crates/cargo-handler/src/census/accounts_wire_tests.rs` — tests 1 and 2
- `crates/cargo-handler/src/summary/accounts_tests.rs` — tests 3 and 5
- `crates/cargo-handler/src/summary/accounts_header_sort_tests.rs` — test 4
- `docs/cargo-handler/as-built/accounts.md` — the `login ends` column

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows the login end dates.

### Phase 10 — Status reads busy, idle, paused or exited; starting and waiting are activities, the same in the accounts board and cargo-handler  · status: todo

**Depends on:** Phase 7 (the census facts); Phase 8 (`census --json`, which the accounts board reads); Phase 9 (it changes the accounts table columns)

#### Work Order

**Goal:** a session's status has four values, and what a busy session does is its activity, read from the census, so the accounts board and cargo-handler show the same words.

**Source:** the user, via natedev and the showrunner, 2026-11-13, on C1513 (starting and waiting columns, rejected): "the account unit should make sure this synchronizes with cargo handler (talk to cargo liner for assignment) - it is important that everything works the same way". Showrunner context: status keeps four values only: busy, idle, paused, exited. Each busy showrunner, unit and worker shows an activity (starting, waiting, thinking, running, compacting, and so on). The handler unit owns the cargo-handler side. The accounts unit agrees one definition with the handler unit.

**Decided (showrunner, 2026-11-13):** one owner for each field. `status` is from `session_state.py` through `accounts.py status --json`; cargo-handler makes no status word. `activity` is from the census in Rust; the accounts board reads it from `cargo-handler census --json`. No Python copy of the activity classifier. The `waiting` ink moves to the activity word.

**Spec:**

Director decisions:
1. The status cell shows only `busy`, `idle`, `paused` or `exited`, from the accounts report. `starting` and `waiting` leave the status.
2. A busy row's activity cell shows its activity word from the census. `census --json` prints each session as `{session, status, activity}`, with `activity` null on a row that is not busy.
3. The accounts table drops its `starting` and `waiting` columns.
4. Director call: the `waiting` bright yellow ink (Phase 5) moves from the status word to the activity word `waiting`. The other activity words keep their inks.

Tests (red first, each under one second):
1. Each status value draws in the status cell; no row draws `starting` or `waiting` there.
2. Each activity word of the shared definition comes out of a census fixture that holds its fact, in the tile and in `census --json`.
3. The activity word `waiting` draws in the `waiting` ink on the dark and the light default themes.
4. The accounts table has no `starting` or `waiting` column.

**Constraints from prior phases:**
- Phase 7 holds the run, pane and activity facts. Phase 8 prints them. Add no second reader.
- The Delegation Context Status source: cargo-handler never makes a status word of its own.

**Live check (director):** on natedev, compare the accounts board with cargo-handler while a unit starts, waits and works. Both show the same status and the same activity for each row.

**Change-Review line:** Status reads busy, idle, paused or exited, and starting and waiting show as activities | On natedev, open the accounts board and cargo-handler while a unit starts and waits | Both show the same four status words and the same activity word for each busy row

**Files:**
- `crates/cargo-handler/src/census/accounts.rs` — the four status values
- `crates/cargo-handler/src/agent_cell.rs` — the activity word and its ink
- `crates/cargo-handler/src/sort.rs`, `crates/cargo-handler/src/summary/accounts.rs` — the removed columns
- `crates/cargo-handler/src/census/report.rs` — the `census --json` activity
- `crates/cargo-handler/src/census/accounts_wire_tests.rs`, `crates/cargo-handler/src/summary/accounts_tests.rs`, `crates/cargo-handler/src/census/report_tests.rs` — the tests
- `docs/cargo-handler/as-built/accounts.md`, `docs/cargo-handler/as-built/hierarchy.md`, `docs/cargo-handler/as-built/colors.md` — status, activity and ink

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows the same words in both tools.

### Phase 11 — Enter shows the focused tile on the full screen, shared with cargo-tile through tui_pane  · status: todo

**Depends on:** Phase 3 (both change the handler key handling)

#### Work Order

**Goal:** in cargo-handler, Enter shows the focused tile on the full screen, and Esc returns to the grid. cargo-tile and cargo-handler use one full-screen tile implementation in tui_pane.

**Source:** the user, change review C219 (returned), via the showrunner, 2026-11-12: "i like it and it works on cargo tile and this should be shared via tuii_pane to work on cargo handler". C219 shipped the cargo-tile full-screen cell in merge 35a8bccf. Showrunner: move the code into tui_pane with no copy; cargo-tile keeps its behavior and its tests; the handler unit may edit cargo-tile after it reserves the files with berth; check that Enter is free in the handler grid and report it before the build.

**Spec:**

Director decisions:
1. Keys, confirmed by the showrunner on 2026-11-12: Enter takes the full-screen tile in cargo-handler, as `global.expand = enter` does in cargo-tile. Right and Left keep row open and close. `toggle_row` stays an action with no default key; a user can bind it in config.
2. The shared view state moves into tui_pane: from `crates/cargo-tile/src/grid_view.rs` the expanded cell identity and the motion clock, and from cargo-tile's `render.rs` the motion and clipping parts of `draw_cell_view` and `ExpandedCell`. Scrolling stays an app policy (decision 3). The tile identity is a type parameter, and the content comes through the trait in the constraints below. Each app's `terminal.rs` advances the motion clock. No part is copied.
3. Each app has its own key policy around the shared state. `CellViewport::key` takes Left and Right and uses Up and Down to scroll in cargo-tile. In cargo-handler, Up and Down select rows and Right and Left open and close rows, in the full-screen tile too.
4. Esc in cargo-handler closes the first open layer: the details view, then the full-screen tile, then the word highlight.
5. cargo-tile's Enter, Esc, scroll and motion keep their current behavior.
6. In cargo-handler, the full-screen tile draws the same tree, columns and selection as the grid tile, at the full pane size. Row keys, `d` and word clicks work in it as in the grid.
7. Reserve the cargo-tile files with `cargo-berth` before the first edit.

**Constraints from prior phases:**
- Phase 2: a config update calls `invalidate_body` to repaint retained bodies. The full-screen tile repaints on the same signal. The word highlight notice surfaces from Phase 2 stay as they are.
- Shared boundary (director decision, 2026-11-13): cargo-tile's `draw_cell_view` reads its roster and prepared tables, and `ExpandedCell` uses app row and header measures. A generic tile identity alone cannot move them. tui_pane owns the expanded identity, the motion clock, and the motion and clipping geometry. Each app supplies its content measures and draws its content into the rect that tui_pane gives, through one tui_pane trait. cargo-tile's roster and table reads stay in cargo-tile, behind its trait implementation.
- cargo-handler `Cells` owns row selection, the tree viewport, the painted headers and the Phase 3 shared column fitting. The full-screen tile uses that selection and viewport; it does not use cargo-tile's scroll state, so there is one viewport owner. The expanded tile fits its columns as a `TreeSizing::Local` tile at the full pane width. On collapse, the grid fits the shared columns again, and the blank slots of Phase 3 come back.
- Phase 3 registers the details view as its own copy target and draws the copy notice above a details view that fills the frame (`render/mod.rs`). Keep both paths with the full-screen layer under the details view.
- Phase 6: `App::click` (`app.rs`) sends a click to an open details view first (`DetailsOverlay::click`, a `tui_pane::table_row_at` hit test on its rendered viewport), then to the favorites overlay, then to the tiles. `App::scroll` follows the same order: the details view, then the favorites overlay, then the tiles. `interaction.rs` `app_modal_overlay_hit` treats an open details view as modal. Keep this order with the full-screen layer under the overlays, and give the full-screen tile the same click and wheel parity as a grid tile.
- Phase 4: `TileGrid` holds the column placement state (`ColumnRanks`, `crates/tui_pane/src/tiles/grid/column_order.rs`), and the app selects the rule with `TileCells::column_order`. cargo-handler returns `TileColumnOrder::LongestWithShortest` from each tile's full tree rows; cargo-tile keeps the default source order. Expand and collapse change no tile membership, so they keep the ranks. When the pane size and the ranking do not change, a collapse puts every tile back in the column it held before Enter. Do not clear or rebuild the ranks for the full-screen tile.

Tests (red first, each under one second):
1. In cargo-handler, Enter on a focused tile draws that tile at the full pane size; Esc returns to the grid with the same focus and selection.
2. Right and Left still open and close the selected row.
3. cargo-tile's existing expand, Esc and scroll tests pass unchanged.
4. The shared tui_pane type has its own tests for identity, motion, clipping, expand and collapse. Scroll tests stay in cargo-tile, and row-navigation tests stay in cargo-handler.
5. Each key is tested through each app's normal key dispatch: Enter, Esc order (details, full screen, word highlight), Up, Down, Right, Left.
6. The full-screen motion animates, and a word highlight change repaints the full-screen tile (the Phase 2 retained-body invalidation).
7. In cargo-handler, expand, resize and collapse keep the row selection, header clicks hit the drawn headers, the blank column slots come back after collapse, and an expanded tile that leaves the census closes the full-screen view.
8. `details_navigation_skips_headings_and_y_copies_only_the_value` and `assert_full_frame_copy_notice` pass, also with the details view open over the full-screen tile, and with a clipboard failure.

**Live check (director):** before the checkpoint, capture cargo-handler's grid, the full-screen tile after Enter, and the grid after Esc, from a build of this branch, under the safety rule.

**Change-Review line:** Enter shows the focused tile on the full screen in cargo-handler, as in cargo-tile | On natedev, run cargo-handler, focus a tile and press Enter, then Esc | Enter fills the screen with the focused tile; Esc returns to the grid; Right and Left still open and close rows

**Files:**
- `crates/tui_pane/src/` — the shared full-screen tile type
- `crates/cargo-tile/src/grid_view.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/frame_preparation.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/terminal.rs` — use the shared type; `draw_cell_view` and `ExpandedCell` leave `render.rs` (reserved with berth first)
- `crates/cargo-handler/src/globals.rs`, `crates/cargo-handler/src/keymap.rs`, `crates/cargo-handler/src/app.rs`, `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/terminal.rs` — Enter and Esc, the key policy, the motion clock and the full-screen draw
- `docs/cargo-handler/as-built/interaction.md`, `docs/cargo-tile/as-built/` — the keys

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test` passes for cargo-handler, tui_pane and cargo-tile with all listed tests, and `bash ~/.claude/scripts/direct/verify.sh lint` passes for each. The live check shows the full-screen tile and the return to the grid.

### Phase 12 — Producers get their own tile, titled "producers"; they leave the unassigned tile  · status: todo

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
- `UnassignedEntry` and `group_tiles` (`agent_cell.rs`) own pooling today. `UnassignedEntry` stores one `machine` value for one machine's pool. Make a pooled-tile type with an explicit pool enum (unassigned for one machine, producers across machines), not a bool or a bare `Option`. Each producer row carries its own machine.
- Membership changes keep the Phase 3 column fitter and the Phase 4 placement rule. The producers tile is a detail tile: it takes part in the Phase 4 ranking, and in the Phase 11 full-screen tile with its selection, expansion and `y` copy.
- `hierarchy_details` (`agent_cell.rs`) maps `RoleLabel::Stated` and `Inferred` through `RowRole::from_hierarchy`. A producer row's `d` view shows `name: <session>` and `role: producer` today. Keep that result in the producers tile, and test it.
- Keep `staff_run_keeps_one_waiting_worker_without_a_launcher_or_open_thread` green. A worker with a live detached run draws under its producer when the producer owns it, and its `d` view shows the run step.

Tests (red first, each under one second):
1. A producer session draws in the producers tile and not in the unassigned tile.
2. A session that staff does not name a producer stays in the unassigned tile.
3. With no producer session, no producers tile draws. Removing the last producer removes the tile.
4. The producers tile title reads `producers`.
5. Producers from two machines share one tile; equal names stay distinct rows.
6. A producer with no status entry still draws in the producers tile.
7. Sort, expansion, selection, header clicks and `d` work in the producers tile after its membership changes.
8. A producer row's `d` view shows `name:` and `role: producer`. The producers tile ranks by the Phase 4 rule and opens on the full screen with Enter.

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

### Phase 13 — Claude rows show their effort, and a long shell command does not widen the session column  · status: todo

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
- Phase 3 keeps the measured demand widths (`TreeMetrics.widths`) apart from the drawn widths (`TreeColumnAlignment::Fitted`), and fills shared tiles in `align_screen_columns`. The shell rule changes only the measured session width. Keep the shared slots and the fill space that the fitter gives; a shell row can draw into session width that the fill gives.
- The cut keeps quotes, spaces, redirections and arguments that are not paths.

Tests (red first, each under one second):
1. A Claude assistant record with a top-level `effort` gives that effort; its `message.model` gives the model.
2. A Claude tile with recorded effort draws the effort column at a width that fits it.
3. A path argument draws as its file name; other arguments are unchanged.
4. A shell row longer than the widest agent row does not widen the session column, and it ends in `…` at the exact column edge. Neighbor tiles keep their widths.
5. The details view of that shell row, opened with `d`, shows the full command, and `y` on its `name:` line copies the full command.
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

**Workers:** 2 writers.
- `metadata` — `census/model.rs`, `census/transcript.rs` tests, `docs/cargo-handler/as-built/startup.md`: the effort reader.
- `shell` — `agent_cell.rs` shell text and session width, `render/details_tests.rs`, `render/narrow_width_tests.rs`, `hierarchy.md`, `tile-layout.md`.
Both use the existing `ModelName` and `ReasoningEffort` types.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows effort on Claude rows and a shell row that does not widen the session column.

### Phase 14 — Reset times in the accounts footer take three clearly different colors  · status: todo

**Depends on:** Phase 5 (it extends the theme loader that Phase 5 extends with `waiting`)

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
6. One contrast measure (showrunner, 2026-11-13). The footer color tests use `tui_pane::contrast_ratio` (`crates/tui_pane/src/theme/blend.rs`). The `contrast_ratio` helper in `footer_colors/tests.rs` goes. The shared measure returns `Option<f64>`, which is `None` for `Color::Reset`; a test unwraps it with a message that names the two colors.

**Constraints from prior phases:**
- Phase 5 extends the bundled theme loading with the `waiting` role. Extend that loader with the new reset roles; do not make a second load path. Keep the `waiting` inheritance and contrast tests green. Load the new reset roles from the bundled theme files, with inheritance and custom-override tests. The Phase 2 config reader handles emphasis only; themes use the theme-loading path.
- Phase 2: `MatchInk::resolve` can change a foreground after selection and word highlight. The new inks keep their readability and differences at every emphasis; add these cases to `render/word_contrast_tests.rs`.
- Keep `ResetRank` as the checked rank type, with its strict quarter edges and its single-reset rule. Measure the drawn colors, not only the theme role values. The current gradient test allows four steps and checks only neighbor pairs.
- `accounts.md` names the new keys that replace `reset_nearest` and `reset_farthest` for custom themes.

Tests (red first, each under one second):
1. Each built-in theme file sets the three roles, and a theme file can override each one.
2. The rank rule gives soonest, middle and latest in order; a single reset gives soonest.
3. A footer with all three rank groups draws exactly three reset colors. Every pair is at least 20 CIEDE2000 apart after `readable_status_style`, in every built-in theme, tint on and off.
4. The existing contrast, no-red and session-color tests pass with the new colors.
5. Equal reset times, one distinct reset and exact quarter edges keep the rank rule. A custom theme without the new roles takes the defaults.
6. The footer color tests in `footer_colors/tests.rs` and `footer_colors/ground_tests.rs` pass with `tui_pane::contrast_ratio`, with their floors unchanged.

**Live check (director):** before the checkpoint, capture the accounts footer with three or more distinct reset times from a build of this branch and from main, on the default theme and on one light theme, under the safety rule.

**Change-Review line:** Reset times in the accounts footer take three clearly different colors | On natedev, run cargo-handler and look at the resets column of the accounts footer | The soonest resets are green, the middle ones gold, and the latest ones blue, on dark and light themes

**Files:**
- `crates/cargo-handler/src/theme/mod.rs` — `ResetRank::color`, the theme role set
- `crates/cargo-handler/themes/*.toml` — the three roles in each built-in theme
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — step order and distance
- `crates/cargo-handler/src/theme/files_tests.rs` — theme file roles
- `crates/cargo-handler/src/theme/files.rs` — custom theme role load and defaults
- `crates/cargo-handler/src/footer_colors/tests.rs`, `crates/cargo-handler/src/footer_colors/ground_tests.rs` — the footer contrast tests on the shared measure
- `docs/cargo-handler/as-built/accounts.md`, `docs/cargo-handler/as-built/colors.md` — the reset steps

**Workers:** 1 writer.
- `impl` — every file above, and the tests.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass with all listed tests. The live check shows three different hues in the resets column on both themes.

### Phase 15 — A worker that waits on a detached run reads "waiting" in its status cell  · status: done (accounts merge ce37d0e40)

**As-built:** the accounts unit built C324 on branch `accounts-board-states` (fix 7c85370c3), merged into main as ce37d0e40 and into this branch at the Phase 4 start. `SessionState::Waiting` reads the accounts report word `waiting`, and the status cell draws it. The `waiting` bright yellow ink for the status word is in Phase 5.

### Phase 16 — Nine slow tests run in under one second  · status: todo

**Depends on:** Phase 6 (it changes `render/details_tests.rs` and `census/codex_session_tests.rs`); Phase 14 (it changes `render/word_contrast_tests.rs`)

#### Work Order

**Goal:** nine tests that took one second or more now run in under one second in the full package run, with the same assertions.

**Source:** the showrunner, 2026-11-13, on the handler add-ons from Phases 3 and 4: "A test over 1 s is a defect also under load; a quiet run does not settle it."

**Spec:**

Director decisions:
1. The Delegation Context invariant holds: no test takes one second or more. Each test below took one second or more in a full run on natedev.
2. Keep every assertion and every case each test checks. Cut the work per case: shared setup built once, fewer redundant frames or renders, cheaper fixtures. Do not split a test into parts to hide its total time, and do not mark a test ignored or slow.
3. Find the cost first: measure each test alone and in its full package run, and name the step that takes the time.

The tests and their measured times:
1. tui_pane `startup_cells_leave_no_line_before_their_first_row` (`crates/tui_pane/src/tiles/draw/paint_tests.rs`) — 1.081 s, Phase 3.
2. tui_pane `every_frame_of_a_key_keeps_the_motion_rules` (`crates/tui_pane/src/tiles/grid/tests/motion_rules.rs`, through `assert_motion_rules`) — 1.081 s, Phase 3; 1.051 s, Phase 4.
3. cargo-handler `astra_word_matches_keep_coverage_in_high_contrast_dark` (`crates/cargo-handler/src/render/word_contrast_tests.rs`, through `check_astra_word_matches`) — 1.125 s, Phase 4.
4. cargo-handler `astra_word_matches_keep_coverage_in_high_contrast_light` (same file) — 1.324 s, Phase 4.
5. cargo-handler `stacked_account_fact_clicks_select_the_account` (`crates/cargo-handler/src/render/accounts_grid_tests.rs`, through `assert_account_continuations_select`) — 1.222 s, Phase 4.
6. cargo-handler `row_details_key_shows_properties_and_long_age_then_escape_closes` (`crates/cargo-handler/src/render/details_tests.rs`) — 1.06 to 1.24 s, Phase 5.
7. cargo-handler `codex_interactive_threads_accept_user_sources_and_exclude_workers_and_exec` (`crates/cargo-handler/src/census/codex.rs`) — 1.047 s, tile Phase 62 gate.
8. cargo-handler `a_detached_daemon_without_an_exact_join_shows_the_pid` (`crates/cargo-handler/src/census/codex_session_tests.rs`) — 1.150 s, tile Phase 62 gate.
9. cargo-handler `an_explicit_resume_id_cannot_name_an_unproven_non_user_thread` (same file) — 1.038 s, tile Phase 62 gate.

**Constraints from prior phases:**
- `check_astra_word_matches` and `assert_motion_rules` also serve tests that are not in the list. A change to a shared helper keeps those tests' assertions and keeps them under one second.
- Phase 14 adds reset-ink cases to `render/word_contrast_tests.rs`. Keep them.
- Phase 6 changes `render/details_tests.rs` and `census/codex_session_tests.rs`. Keep its changes.

**Change-Review line:** none (test code only)

**Files:**
- `crates/tui_pane/src/tiles/draw/paint_tests.rs` — test 1
- `crates/tui_pane/src/tiles/grid/tests/motion_rules.rs` — test 2 and `assert_motion_rules`
- `crates/cargo-handler/src/render/word_contrast_tests.rs` — tests 3 and 4 and `check_astra_word_matches`
- `crates/cargo-handler/src/render/accounts_grid_tests.rs` — test 5 and `assert_account_continuations_select`
- `crates/cargo-handler/src/render/details_tests.rs` — test 6
- `crates/cargo-handler/src/census/codex.rs` — test 7
- `crates/cargo-handler/src/census/codex_session_tests.rs` — tests 8 and 9

**Workers:** 1 writer.
- `impl` — every file above.

**State:** none

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`, `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` and `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` pass. Each of the nine tests runs in under one second in its full package run, with its assertions unchanged.
