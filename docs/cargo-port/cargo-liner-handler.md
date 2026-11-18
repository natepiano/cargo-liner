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

### Phase 8 — `cargo-handler census --json` prints the census the TUI shows, so other tools read the same worker facts  · status: done

#### As-built

- `cargo-handler census --json` (`cli.rs`, `CENSUS_COMMAND`) scans once, prints one JSON document on stdout and exits 0. A bad configuration or a failed local process scan exits non-zero with the error on stderr.
- `census/report.rs` holds the serde contract: `CensusReport { schema, machines, directors, workers }` with `MachineReport`, `DirectorReport`, `WorkerReport`, `PassReport`, `PaneReport`, `RunReport` and `UnknownReport`. `schema` is 1. Times are epoch seconds; a field with no value is null, never a missing key. A field rename or removal increments `schema`.
- The report lists only the unit directors and current-run workers that a tile shows, keyed by staff id. A staff row with no tile line is not listed. Name, machine, status and activity come from `agent_cell::session_cells`, so they equal the tile text. `status` is null with no accounts entry.
- `print_to` runs the remote probes and `AccountsTable::read_once` in threads beside `probe::scan_local`, then applies them through `Census::record_local_accounts` and `RemoteHierarchyAnswers::answer`. The probe and the TUI use the same three functions.
- An unreachable remote gives its `machines` entry `reachable: false` and its error, with no rows. An unreadable staff file or a failed accounts read gives the local entry its error and keeps the remaining rows. Both exit 0.
- `WorkerFacts` adds `listing` (`StaffListing::{Director { id }, Worker { id, parent }, Other}`) and `pass` (`PassFact::{Open(PassAssignment), Closed}`, `PassRole::{Impl, Test, Fix, Review}`).
- A Phase 7 `Unknown` fact prints as `unknown: { reason }` on its director, worker or run, never as an empty list.

**Files:**
- `crates/cargo-handler/src/census/report.rs`, `crates/cargo-handler/src/census/report_tests.rs` — the command, the JSON types and their tests
- `crates/cargo-handler/src/cli.rs`, `crates/cargo-handler/src/constants.rs` — the subcommand
- `crates/cargo-handler/src/census/mod.rs`, `crates/cargo-handler/src/census/scan.rs`, `crates/cargo-handler/src/census/hierarchy.rs`, `crates/cargo-handler/src/census/worker_dispatch/` — the staff listing and pass facts
- `crates/cargo-handler/src/census/accounts.rs`, `crates/cargo-handler/src/census/probe.rs`, `crates/cargo-handler/src/census/schedule.rs` — the shared accounts read, local scan and remote answer
- `crates/cargo-handler/src/census/session_identity.rs` — `SessionIdentity::ids`, the session ids the tile cells use
- `crates/cargo-handler/src/agent_cell.rs` — `session_cells`
- `crates/cargo-handler/README.md` — the `census --json` section and example, checked by a test
- `docs/cargo-handler/as-built/hierarchy.md` — the command and its source

**Binds later work:** `AccountsTable::read_once` and the accounts monitor share one `AccountsReading` parse. `census --json` takes status and activity only from `agent_cell::session_cells`, so a session that does not reach `session_cells` is absent from the JSON, and `session_cells` copies the full line name. A new top-level JSON list or a field rename increments `schema`.

**Gotchas:** live run time is 2.95 to 8.66 s; the first run of a series is the slowest. Listing every staff row of a current run gives far more rows than the tiles (72 directors and 202 workers against 37 and 0); the tile-shown filter is what makes the JSON and the tiles agree.

**Ruled out:** a second reader or a copy of a tile rule in the command; wait holders and wait reasons in the JSON (they stay in the run records).

### Phase 9 — The accounts table shows the date each Claude login ends  · status: done

#### As-built

- `AccountWire::login_ends_at: Option<String>` has `#[serde(default)]`, so an older report without the key parses. `AccountUsage::login_ends_at: Reading<DateTime<Utc>>` uses the `resets_at` timestamp parse; a malformed value fails the reading.
- `AccountsColumn::LoginEnds` sits after `Resets`, header `login ends`, right aligned. It sorts through `compare_readings`, as `resets` does.
- The cell text is `reset_label` through `date_cell`, which also serves `resets`. An unavailable reading gives an empty cell. `row_cells` sets the cell to the `numeric` style, the default table ink.
- Narrow panes drop `enabled`, then `login ends`, then `credits` (`AccountsColumnDrop::ORDER`). The stacked layout shows a `login ends` fact.
- `terminal.rs` `next_body_change`: the midnight redraw also fires for a known login end date.
- `census --json` does not print the date.

**Files:**
- `crates/cargo-handler/src/census/accounts.rs` — the wire and usage field
- `crates/cargo-handler/src/sort.rs` — `AccountsColumn::LoginEnds` and its header
- `crates/cargo-handler/src/summary/accounts.rs` — value, sort, ink, drop order, stacked fact
- `crates/cargo-handler/src/terminal.rs` — the date redraw
- `crates/cargo-handler/src/census/accounts_wire_tests.rs`, `crates/cargo-handler/src/summary/accounts_tests.rs`, `crates/cargo-handler/src/summary/accounts_header_sort_tests.rs` — the tests
- `docs/cargo-handler/as-built/accounts.md` — the column

**Binds later work:** the header lists in `summary/mod.rs` and `summary/accounts_tests.rs` name every column; a column change edits both. `AccountsColumn::index` values follow the column order.

**Gotchas:** `row_cells` sets each column's style; a new column takes the row freshness ink unless it is set. The accounts report writes `login_ends_at` with fractional seconds and `+00:00`, which the RFC 3339 parse accepts.

**Ruled out:** a rank color for the column; a `census --json` field for the date.

### Phase 10 — Status reads busy, idle, paused or exited; starting and waiting are activities, the same in the accounts board and cargo-handler  · status: done

#### As-built

- One owner for each field. `status` comes from `session_state.py` through `accounts.py status --json`; cargo-handler makes no status word. `activity` comes from the census in Rust, and the accounts board reads it from `cargo-handler census --json`.
- `SessionState` (`census/accounts.rs`) has `Busy`, `Idle`, `Paused` and `Exited`. An old `starting` or `waiting` report value decodes as `Busy`. The status cell shows only these four words.
- A busy row's activity cell joins the job and the live word (`impl · thinking`), as the details view does. Other rows show the job only. `TreeLine::activity_parts` (`agent_cell.rs`) gives the parts.
- `ActivityFit` (`Whole` | `Cut`) cuts the job by the overflow before the role column drops. The cut never goes below the live-word floor, so the live word stays whole. Aligned tiles merge the narrower cut.
- The `waiting` ink (`Role::Waiting`) is on the activity word `waiting` only. It replaces the status-cell `waiting` text in the as-built of "The word "waiting" draws in bright yellow in every tile" and "A worker that waits on a detached run reads "waiting" in its status cell".
- `census --json` is schema 2. It adds a top-level `sessions` list: one `{session, machine, status, activity}` entry for each session that a tile shows, built from `agent_cell::session_cells`. `activity` is null on a row that is not busy. The `directors` and `workers` lists keep their fields.
- The accounts table has no `starting` or `waiting` column. `RoleCountsWire` has no `starting` or `waiting` field.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — activity parts, `ActivityFit`, `session_cells`
- `crates/cargo-handler/src/census/accounts.rs` — the four status values
- `crates/cargo-handler/src/census/report.rs` — the `sessions` list, schema 2
- `crates/cargo-handler/src/summary/accounts.rs`, `crates/cargo-handler/src/sort.rs` — the removed columns
- `crates/cargo-handler/README.md` — the `census` section and its example document
- `crates/cargo-handler/src/census/accounts_wire_tests.rs`, `crates/cargo-handler/src/summary/accounts_tests.rs`, `crates/cargo-handler/src/census/report_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs` — the tests
- `docs/cargo-handler/as-built/accounts.md`, `docs/cargo-handler/as-built/hierarchy.md`, `docs/cargo-handler/as-built/colors.md` — status, activity and ink

**Binds later work:** `report.rs` `print_to` runs `AccountsTable::read_once(config.census.accounts_command)` on its own thread beside `probe::scan_local`, then `CensusReport::build` fills `sessions` through `agent_cell::session_cells`. A session that a tile shows reaches `sessions` only through `session_cells`. A narrow tile keeps `ActivityFit` and the live-word floor.

**Gotchas:**
- `census --json` waits on the accounts command, so its wall time is at least that command's time.
- The live word `waiting` comes from `VerificationProgress::Waiting` (a queued verify run) in `census/activity.rs`. The `job · word` form needs a worker, subagent or reviewer line kind.

### Phase 11 — One `cargo-handler census --json` on natedev takes about one second or less  · status: done

#### As-built

No crate code changed. On natedev, the crate's own stages of one `cargo-handler census --json` (the local scan and `CensusReport::build`) take 0.19 s median in a release build. The accounts command runs beside the local scan, so the full wall time is the larger of the two. With the devtools accounts command fix on `~/.claude` main, the full median is 1.05 s (five runs: 1.23 1.05 0.96 0.89 1.20 s at load average 68-78), and `accounts.py status --json --stored` takes 0.90 s median.

**Files:** none.

**Gotchas:** time the crate stages on a `--release` build; a debug build makes them about seven times slower. A `cat` of a saved accounts answer as `accounts_command` times the crate stages apart from the accounts command.

**Ruled out:** crate changes to the detached-run record reads; the scan of about 9500 records is inside the 0.19 s crate median.

### Phase 12 — Every busy row shows what the session does  · status: done

#### As-built

A busy row draws an activity word from `census/activity.rs` `Activity`. Two causes left the cell empty, and each has its own fix:

1. The Claude session record stays `busy` while the Stop hooks run after the last answer. `census/scan.rs` `attach_activity_with_runs` takes the open turn from the record status: a record `idle` keeps the row idle, and in an open turn the transcript names the work, with `thinking` after an `end_turn` answer.
2. The accounts report reads the session record at a different time than the census scan, so at turn start it can say `busy` while the scan row says `idle`. `agent_cell.rs` `TreeLine::conversation_activity`, called from `live_activity`, gives a `SessionState::Busy` session, worker or unassigned line with a scanned `Idle` or `Unreported` activity `Activity::Thinking`. Shell and subagent work lines keep their scanned activity, and `activity_parts` takes no line-kind branch.

The status still comes from the accounts report. Census samples on natedev, 15 s apart: 7 empty busy cells in 30 samples before, 2 of 293 in 60 after cause 1, 0 of 344 in 60 after cause 2.

**Files:** `crates/cargo-handler/src/census/scan.rs`, `census/scan/activity_scan_tests.rs`, `agent_cell.rs`, `census/report_tests.rs`, `docs/cargo-handler/as-built/activity.md`.

**Binds later work:** a busy conversation line can show `thinking` for one census interval while its turn already runs a tool.

**Ruled out:** one read of the session record for both status and activity; Phase 10 keeps the status from the accounts report.

### Phase 13 — Enter shows the focused tile on the full screen, shared with cargo-tile through tui_pane  · status: done

#### As-built

`tui_pane::FullScreenView<Id, P>` (`crates/tui_pane/src/tiles/full_screen.rs`) owns the shown tile identity, the motion clock and the travel between the grid allocation and the pane. Its states are `Grid`, `Opening`, `Expanding`, `Expanded` and `Collapsing`. `P` is the app policy state: cargo-tile `CellScroll`, cargo-handler `()`. Each frame the app calls `resolve_opening`, `travel` (`FullScreenStep::Inactive | ContentLeft | Travel`), `TileGrid::draw_cell_view` and `settle` (`FullScreenSettle::Shown | Closed`). Each app's `terminal.rs` calls `advance` in the poll.

`TileGrid::draw_cell_view` takes `&mut self` and records the drawn tile as `FullScreenPiece`. `cell_hit` and `pointer_regions` read that piece first, and each body paint resets it.

In cargo-handler, Enter (`global.expand`) opens the focused tile on the full pane, and `toggle_row` has no default key. Esc closes the details view, then the full-screen tile, then the word highlight. Row keys, `d`, header clicks and word clicks work in the full-screen tile. The focus keys do nothing while it shows. A shown tile that leaves the census closes the view at once. cargo-tile keeps its Enter, Esc, scroll and motion behavior.

**Files:**
- `crates/tui_pane/src/tiles/full_screen.rs` — the shared full-screen state, with `full_screen_tests.rs`
- `crates/tui_pane/src/tiles/grid/tile_grid.rs`, `grid/visual.rs` — `draw_cell_view` and `FullScreenPiece`
- `crates/cargo-tile/src/grid_view.rs` — `GridView` alias, `CellScroll`, `key` and `scroll`
- `crates/cargo-tile/src/render.rs` — the cargo-tile content side of the full-screen draw
- `crates/cargo-handler/src/render/mod.rs` — `draw_full_screen`, `redraw_grid`, `FullScreenPane`; tests in `render/full_screen_tests.rs`
- `crates/cargo-handler/src/app.rs`, `globals.rs`, `keymap.rs`, `terminal.rs` — `App::full_screen`, Esc, `Expand`, `move_focus`, the motion clock
- `docs/cargo-handler/as-built/interaction.md` — the full-screen tile section

**Binds later work:** `App::full_screen` keys the shown tile by `TileKey`; a new tile key opens on the full screen with no change there.

**Gotchas:**
- While the tile is expanded, `TileGrid::prepare_covered` keeps the grid geometry without a paint; `redraw_grid` resets the pane before the grid draws again.
- Word-click tests need `Renderer::new`: the fixed-time renderer does not keep `painted_frame`.

### Phase 14 — The cargo-handler status bar shows `enter expand` and `esc collapse`, as cargo-tile does  · status: done

#### As-built

`tui_pane::FullScreenView::status_hint(expand)` gives the status slot for the full-screen view: the app's expand action (`enter expand`) in the grid, and the fixed `esc collapse` (`FULL_SCREEN_COLLAPSE_KEY`, `FULL_SCREEN_COLLAPSE_LABEL` in `tiles/constants.rs`) while a tile shows. Both are `after_uptime`. cargo-tile and cargo-handler call it in `draw_status_line`; cargo-tile has no collapse constants of its own.

`fitted_right_spans` counts the navigation hints before it shows an `after_uptime` slot, so at a narrow width or under the details view the expand hint gives way first and the navigation hints keep their place.

**Files:**
- `crates/tui_pane/src/tiles/full_screen.rs` — `status_hint`
- `crates/tui_pane/src/tiles/constants.rs` — the collapse key and label
- `crates/tui_pane/src/bar/status_line.rs` — the fit rule for an `after_uptime` slot, with `an_after_uptime_hint_gives_way_before_the_navigation`
- `crates/cargo-tile/src/render.rs`, `crates/cargo-handler/src/render/mod.rs` — the status line calls `status_hint`
- `crates/cargo-handler/src/render/full_screen_tests.rs` — the grid, full-screen and narrow-width status tests
- `docs/cargo-handler/as-built/interaction.md` — the status-bar hints

**Binds later work:** the golden frames in cargo-handler `render/mod.rs` take a `StatusEnds`: `GRID_STATUS` over the grid, `OVERLAY_STATUS` under the details view.

**Ruled out:** public collapse constants — no consumer outside `FullScreenView`.

### Phase 15 — Producers get their own tile, titled "producers"; they leave the unassigned tile  · status: done

#### As-built

A session whose staff role is `producer` (`RoleLabel::Stated(Producer)`) draws as a root row of one `producers` tile (`PRODUCERS_CELL_TITLE`), with its subagents and threads under it. The tile holds the producers of every machine, draws before the first unassigned tile, and leaves the grid with its last producer. A producer with no status entry still draws. A producer row shows `name · machine` only when the pool spans more than one machine; row identities keep the machine.

`TileKey::Producers` keys the tile, so its sort, expansion, selection, full-screen view and `d` view (`name:` and `role: producer`) work as in other tiles and stay when the producers change. `TilePool { Producers, Unassigned { machine } }` names the flat pools; `PooledEntry` (field `pool`) draws both through `draw_pooled`, `pooled_line` and `pooled_header_spans`; `AgentTile::Pooled` holds it. `root_tile` maps a root to `RootTile::Own` or `RootTile::Pooled(TilePool)`. The summary membership (`RootPresentation`) and the `census --json` sessions do not change.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TilePool`, `RootTile`, `root_tile`, `PooledEntry`, `group_tiles`, `pooled_tile`
- `crates/cargo-handler/src/tiles.rs` — `TileKey::Producers`
- `crates/cargo-handler/src/constants.rs` — `PRODUCERS_CELL_TITLE`
- `crates/cargo-handler/src/render/mod.rs` — the pooled tile title
- `crates/cargo-handler/src/agent_cell/tests/producers_tests.rs`, `crates/cargo-handler/src/render/producers_tile_tests.rs`, `crates/cargo-handler/src/render/full_screen_tests.rs` — the producers tile tests
- `docs/cargo-handler/as-built/hierarchy.md`, `docs/cargo-handler/as-built/tile-layout.md` — the producers tile

**Binds later work:** the producers tile and the unassigned tiles draw through `draw_pooled` (`PooledEntry`), and producer rows carry their shell work.

**Gotchas:** the fixed-clock test paint (`ranked_column_tests::paint`) leaves out the details overlay; a `d` test paints through `App::draw` (`word_click_tests::Renderer`).

### Phase 16 — Claude rows show their effort, and a long shell command does not widen the session column  · status: done

#### As-built

`ConversationModel::take_claude` takes the model from `message.model` and the effort from the top level of an assistant record, then from the nested effort fields. System-record metadata and the effort aliases still apply, and a later record with no effort keeps the recorded one.

The column fitter keeps the drop order as the priority order. `GroupFitStep::Restore` runs after the five optional drops (`ColumnDrop::is_optional`: effort, model, directory, type, branch) and before `CutSession`. `TreeMetrics::restore_columns` adds back each dropped optional column in reverse drop order while it fits, then the fill gives the rest to the session column. Added-back columns keep their display positions, and the column set depends only on the tile width and the row data. Each tile adds back only columns its own rows fill (`TreeMetrics::measured_columns`); the shared fit at one x adds back only columns that one of its tiles keeps.

A shell command row draws `shell_command::short_command`, which cuts each path argument to its file name and keeps quotes, redirections and other arguments. `TreeLine::display_name` holds the drawn text; `TreeLine::name` keeps the full command for the `d` view `name:` line, `y`, and selection identity. A shell row adds only its tree lead to the session demand, so the agent rows set the session width and a longer shell row ends in `…` at the column edge. `AgentEntry::root_identity` and `TreeLineContent::row_identity` are the one way to read a row's `RowIdentity`.

**Files:**
- `crates/cargo-handler/src/census/model.rs` — the Claude effort and model reader
- `crates/cargo-handler/src/agent_cell.rs` — the restore step, `measured_columns`, the shell row width and display text
- `crates/cargo-handler/src/shell_command.rs` — the short command form
- `crates/cargo-handler/src/agent_cell/tests/shell_command_tests.rs`, `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs`, `crates/cargo-handler/src/render/details_tests.rs`, `crates/cargo-handler/src/census/transcript.rs` — the tests
- `docs/cargo-handler/as-built/hierarchy.md`, `docs/cargo-handler/as-built/tile-layout.md`, `docs/cargo-handler/as-built/startup.md` — the effort source, the restore step and the shell row text

**Binds later work:** `render/details_tests.rs` holds `long_shell_application` and its two tests (the drawn short command, and `d` and `y` on the full command).

**Gotchas:**
- The restore uses the uncut session width: a long session name keeps its room, and the session cut comes after.
- A tile draws only its own fitted columns. If the shared fit adds a column that none of its tiles keeps, the tiles draw empty slots and the session falls short.

**Ruled out:** moving effort later in the drop order, and deferring the column rule.

### Phase 17 — Reset times in the accounts footer take three clearly different colors  · status: done

#### As-built

Three theme roles set the reset times in the accounts footer: `reset_soonest`, `reset_middle` and `reset_latest`. Every built-in theme file sets all three, a custom theme can override each one, and a theme without them takes the Default Dark colors. `reset_nearest` and `reset_farthest` are gone. The steps are different hue families: soonest green (the quota comes back soon), middle blue, latest violet; no step is red.

`ResetStep` (`Soonest`, `Middle`, `Latest`) holds the rank rule through `From<ResetRank>`: the first quarter of distinct ranks is `Soonest`, the last quarter is `Latest`, and the rest are `Middle`; a single reset is `Soonest`. `ResetRank` stays the checked rank type, with strict quarter edges. `ResetRank::color` reads the step's role from the active theme.

- Dark themes: soonest (0,203,123), middle (10,191,255), latest (204,158,255). Light themes: soonest (0,88,13), middle (2,78,124), latest (130,0,206).
- Every pair of drawn steps is at least 20 CIEDE2000 apart after `readable_status_style`, in every built-in theme, with the pane tint on and off. On light themes each reset ink keeps 17 CIEDE2000 from the low-quota warning ink.
- The footer color tests measure with `tui_pane::contrast_ratio`; `footer_colors/tests.rs` keeps `contrast_ratio` as a thin wrapper that names the two colors when the measure fails.

**Files:**
- `crates/cargo-handler/src/theme/mod.rs` — `ResetStep`, `ResetRank`
- `crates/cargo-handler/src/theme/files.rs` — the loader reads the three roles
- `crates/cargo-handler/src/constants.rs` — `RESET_SOONEST_ROLE`, `RESET_MIDDLE_ROLE`, `RESET_LATEST_ROLE`
- `crates/cargo-handler/themes/*.toml` — the three role colors in each built-in theme
- `crates/cargo-handler/src/theme/files_tests.rs` — role load, override, distance and contrast floors
- `crates/cargo-handler/src/summary/accounts_gradient_tests.rs` — rank order and the drawn footer colors
- `crates/cargo-handler/src/render/word_contrast_tests.rs` — reset step word matches at every emphasis
- `crates/cargo-handler/src/footer_colors/tests.rs` — the shared contrast measure
- `docs/cargo-handler/as-built/accounts.md`, `docs/cargo-handler/as-built/colors.md` — the reset roles

**Binds later work:** `render/word_contrast_tests.rs` holds the reset step cases (`check_reset_step_match`, `RESET_STEP_RANKS`, `EMPHASES`); the slow tests phase keeps them.

**Gotchas:**
- On a selected row the 4.5 lift takes every matched ink near white, so the 20 CIEDE2000 step distance holds on plain matches only.
- The session and reset color floors (14.7, 15.3) are in `render/role_title_tests.rs`.
- No readable light green keeps 20 CIEDE2000 from the olive low-quota warning and 15 from the Astra reviewer green; the light floor is 17.

**Ruled out:** a sRGB midpoint between two end roles; a gold step (it fails the chroma and warning floors on light themes); the 5.0 step floor of the earlier gradient, which the user could not see.

### Phase 18 — A worker that waits on a detached run reads "waiting" in its status cell  · status: done (accounts merge ce37d0e40)

**Numbering:** this phase was Phase 17 until 2026-11-14, when the new Phase 14 moved Phases 14-18 to 15-19.

**As-built:** the accounts unit built C324 on branch `accounts-board-states` (fix 7c85370c3), merged into main as ce37d0e40 and into this branch at the Phase 4 start. `SessionState::Waiting` reads the accounts report word `waiting`, and the status cell draws it. The `waiting` bright yellow ink for the status word is in Phase 5.

### Phase 19 — Every busy shell row shows what it does  · status: done

#### As-built

A shell command row is always `busy`. With no scanned word, its activity cell reads `running`; a scanned word, such as a verification step or `waiting`, shows alone. Subagent rows follow the showrunner ruling of 2026-11-15: a subagent whose scan says `idle` has ended its turn and reads `idle`; a subagent with no role word and an unreported scan stays `busy` and reads `running`; a subagent with a role keeps its role text. So no busy row draws an empty activity cell.

- `TreeLine::activity_text` gives the shell command job and the role-less unreported subagent job the one `RUNNING_JOB` text. `RUNNING_JOB` is the `Activity::Running` text, in the agent cells section of `constants.rs`.
- `TreeLine::session_status` gives an idle-scan subagent `SessionState::Idle`. The status cell, status sort, status ink and summary width all read it.
- Work lines still take no `thinking`; `conversation_activity` serves conversation lines only.
- Status sorting uses the displayed text, so shell rows sort under `running`.

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeLine::activity_text`, `TreeLine::session_status`
- `crates/cargo-handler/src/constants.rs` — `RUNNING_JOB`
- `crates/cargo-handler/src/agent_cell/tests/activity_tests.rs` — shell rows, the busy-row tile, `activity_at`, `shell_command_owner`
- `crates/cargo-handler/src/agent_cell/tests/subagent_row_tests.rs` — `subagent_rows_show_their_scanned_activity_and_status`
- `crates/cargo-handler/src/agent_cell/tests/{sort,tree,unassigned}_tests.rs` — expectations for the new cells
- `docs/cargo-handler/as-built/activity.md` — the shell and subagent row activity

**Binds later work:** Phase 20 cuts only a live `running` after a job; `RUNNING_JOB` is a job, so it stays.

**Gotchas:**
- The test `child()` helper defaults to `AgentActivity::Idle`, so a fixture subagent reads `idle` unless the test sets `Unreported` or an active word.
- The tile root is the cell header, not a tree row; a busy-row scan of the tree rows does not include it.

### Phase 20 — The activity cell drops "· running", and long names take "…" before model and effort drop  · status: done

#### As-built

A worker row whose job runs shows its job alone, such as `reviewer`, not `reviewer · running`; other live words stay (`reviewer · waiting`). A row with no job and the live word `running` draws `running`, so a busy row never draws an empty cell. On a narrow tile, long session and branch names end in `…` before the effort and model columns drop. At 125 and 127 columns the hana, cargo-liner and natedev tiles show model and effort on every Claude row.

- `TreeLine::job_and_live_word` drops the live `running` word after a job. `RUNNING_JOB` is a job, so a busy shell row still reads `running`.
- `GroupFitStep::ORDER` has 13 steps: `CutNames`, five optional drops (effort, model, directory, type, branch), `Restore`, `CutSession`, four required drops (account, role, activity, status), `FinalMerge`. `CutNames` cuts the session and branch columns to their floors (`NameCut`, `NameLimit`, `cut_names`, `uncut_names`, `name_limits`). `restore_columns` adds back each dropped optional column, most important first, while it fits, then gives free width back to the cut names.
- A name cut never makes room for the directory: `ColumnDrop::fits_beside_cut_names` and `TreeMetrics::fits` let the directory fit only beside the uncut names, while branch, type, model and effort fit beside the cut names. So on a long-named tree the directory drops before effort and model; on a tree whose names fit, the order is unchanged.
- `TilesConfig` keys `session_cut_floor` (default 24) and `branch_cut_floor` (default 16), defaults `DEFAULT_SESSION_CUT_FLOOR` and `DEFAULT_BRANCH_CUT_FLOOR` in `constants.rs`. The fields are private and read through `TilesConfig::name_cut_floors() -> NameCutFloors`. `Cells::with_name_cut_floors` passes them to every agent cell and tile in `render/mod.rs` and `render/painted_surface.rs`.
- The activity column width follows the widest drawn activity cell (`visible_widths`).

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `TreeLine::job_and_live_word`, `GroupFitStep::CutNames`, `NameCut`, `TreeMetrics::fits`, `ColumnDrop::fits_beside_cut_names`, `restore_columns`
- `crates/cargo-handler/src/config.rs` — `TilesConfig` floor keys, `NameCutFloors`
- `crates/cargo-handler/src/constants.rs` — the two default floors
- `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/render/painted_surface.rs` — pass the floors to the tiles
- `crates/cargo-handler/src/agent_cell/tests/activity_tests.rs` — the activity text without `· running`
- `crates/cargo-handler/src/agent_cell/tests/column_visibility_tests.rs` — `with_c352_cells`, `a_name_cut_makes_room_for_model_and_effort_but_not_the_directory`, `long_names_take_an_ellipsis_before_model_and_effort_drop`, the drop order with the directory first on a long-named tree
- `crates/cargo-handler/README.md`, `crates/cargo-handler/CHANGELOG.md` — the two config keys and the fixes
- `docs/cargo-handler/as-built/tile-layout.md`, `docs/cargo-handler/as-built/activity.md` — the fit order and the activity text

**Gotchas:**
- A shell row adds only its tree lead to the session demand, so shell names are cut by the column width, not by `CutNames`.
- `CutSession` still cuts a session name below its floor before a required column drops.

**Ruled out:**
- The uncut-names check for branch and type: it hid the long branch, so it applies to the directory only.
- The Phase 16 rule that the restore uses the uncut session width: a long session name now takes `…` before an optional column drops.

### Phase 21 — Twelve slow tests run in under one second  · status: done

#### As-built

No code change: the twelve tests run in under one second in their full package runs, with their assertions unchanged. Natedev times at bd07521c1:

- tui_pane, load 13 to 17: `startup_cells_leave_no_line_before_their_first_row` 0.452 s; `every_frame_of_a_key_keeps_the_motion_rules` 0.304 s.
- cargo-handler, load 35 to 13: `astra_word_matches_keep_coverage_in_high_contrast_dark` 0.218 s, `astra_word_matches_keep_coverage_in_high_contrast_light` 0.232 s, `stacked_account_fact_clicks_select_the_account` 0.210 s, `row_details_key_shows_properties_and_long_age_then_escape_closes` 0.235 s; the six codex census tests in `census/codex.rs` and `census/codex_session_tests.rs` 0.017 to 0.043 s.

**Gotchas:**
- The earlier times of 1.04 to 1.57 s came from runs on 2026-11-13 and earlier; the cause of the speed-up was not traced.

### Phase 22 — `census --json` lists each session's shell rows  · status: done

#### As-built

Each `sessions` entry of `census --json` has `shells: [{name, started_at}]`. `agent_cell::session_cells` puts each `WorkKind::Shell` line into `SessionCells::shells` of its owning row's sessions, in tile line order. `census/report.rs` maps each `ShellCells` to a `ShellReport`. The name is the tile name cell without the machine (`shell_command::short_command`). `started_at` is the child row's start in epoch seconds and is never null. A session with no shell row has `[]`. The schema stays 2: a new key does not increment it, and the sdlc census reader accepts only 2. Test: `sessions_list_their_shell_rows_with_the_tile_name_and_start` in `census/report_tests.rs`.

**Gotchas:**
- A Codex process with several recorded threads puts each shell under each thread of the row. One worker runs one thread per process, so this case was left as is.

### Phase 23 — An account color can be a `#rrggbb` value, shows the same in every place, and changes without a restart  · status: done

#### As-built

An account or role value in `~/.claude/config/statusline.conf` can be `#rrggbb` in either case: `FooterColor::from_hex` gives `FooterColor::Rgb`, and any other `#` value is unknown, so its entry draws plain. In `FooterColors::parse` a trimmed line that starts with `#` is a comment; otherwise the line splits at the first `=`, and `without_comment` cuts the value at a `#` that follows whitespace. So `account.x = #a0522d # sienna` reads `#a0522d`. `ReadablePalette::ink` resolves a hex color on first use through the same readable path as a name, and a readable hex color draws unchanged. The census scan thread owns a `FooterColorFile::home()` and calls `send_changed` after each local round; the palette goes on its own channel to `Ticker::drain_footer_colors`, which applies the newest one through `App::apply_footer_colors` and asks for a frame. The UI thread reads no file. `App::new` reads the file once at startup. Tests: `hex_footer_colors_parse_in_either_case`, `malformed_hex_footer_colors_are_unknown`, `a_comment_starts_only_at_the_line_start_or_after_whitespace`, `a_readable_hex_color_draws_unchanged` (footer_colors/tests.rs), `a_changed_footer_file_reaches_the_app_without_a_ui_thread_read` (terminal.rs), and `one_hex_account_draws_the_same_ink_in_the_tree_program_line_and_accounts_table` (render/accounts_grid_tests.rs).

**Gotchas:**
- The summary draws a program line only for hierarchy roots, and a selected tree row lifts its ink for the selection ground. A same-ink test puts the account on an unselected row.
- The Mac `cargo mend` pass made the `UnknownReason` field private. Only census submodules build the type.

### Phase 24 — A one-shot `probe` or `census` reads the subagent directory once and never hangs on a file watch  · status: done

#### As-built

A one-shot scan installs no notify watch. `DirectoryWatching { Live, Once }` (`census/transcript.rs`, default `Live`) is a field of `SubagentReads`, `WorkerNames` and `RunRecords`; each type's `listing()` sets `Once`. With `Once`, `InputDirectoryWatch` and `RunDirectoryWatch` use a `Listing` backend: `rearm` returns at once and the directory is read each round. `LocalScanner::one_shot` (`census/scan.rs`) sets all three listings, and `probe::scan_local` calls it. The live census keeps `Live`.

`probe::print` runs the scan on the thread `probe-scan`. The main thread calls `wait_for_scan`, which returns a `ProbeWait`: `Finished` exits with the scan's code; `DeadlinePassed` writes one stderr line and exits with failure; `CallerGone` (the parent pid changed) and `ScanEnded` (the thread ended with no result) exit with failure. `PROBE_DEADLINE` is `PROBE_TIMEOUT` minus `PROBE_EXIT_RESERVE` (1 s), so 19 s; `PROBE_CALLER_CHECK` is 250 ms (`constants.rs`). Tests: `one_shot_subagent_reads_list_the_directory_without_a_watch`, `one_shot_worker_seats_list_the_directory_without_a_watch`, `one_shot_run_records_list_the_directory_without_a_watch`, `a_one_shot_scan_finds_subagents_without_a_file_watch`, and the four `wait_for_scan` outcome tests in `probe.rs`.

**Files:**
- `crates/cargo-handler/src/census/transcript.rs` — `DirectoryWatching`, the `Listing` input watch backend
- `crates/cargo-handler/src/census/worker_dispatch/mod.rs`, `crates/cargo-handler/src/census/activity.rs` — the worker seat and run record listing modes
- `crates/cargo-handler/src/census/scan.rs` — `LocalScanner::one_shot`
- `crates/cargo-handler/src/census/probe.rs` — the `probe-scan` thread, `wait_for_scan`, `ProbeWait`

**Gotchas:**
- A one-shot reader must call `one_shot()`. The default `LocalScanner` installs watches for the live census, and on macOS an FSEvents `watch` call can block forever in `FsEventWatcher::stop`.

**Ruled out:** a patch to notify; the fix stays in cargo-handler.

### Phase 25 — Every row's type reads agent, shell or subagent; a shell has no account; a shell or subagent name uses free row width  · status: done

#### As-built

The tree's type column follows role and comes before activity, in `TreeColumn` order (session, account, role, type, activity, status, branch, model, effort, directory, age) and in the details pane hierarchy section. `RowType { Agent, Shell, Subagent }` gives the type cell the `ROW_TYPE_AGENT`, `ROW_TYPE_SHELL` or `ROW_TYPE_SUBAGENT` word (`constants.rs`); session, thread and worker rows read `agent`. `Work::account` returns `LineAccount { Runs(&Account), Shell }`: a shell row draws a blank account cell, sorts with empty text, and is left out of the account column measure; an unrecorded subagent runs under its session's account, and a thread or a subagent with its own account keeps it. `NameCut::whole_shell_session` holds the session width that shows every shell name whole, and shared tile groups take its maximum. `fill_tree_width` gives unused table width to the session column up to that width, then to the directory, or to the session when the directory is absent. A tile with no unused width still cuts a shell name with `…`. `ColumnDrop::ORDER` drops effort, model, type, directory, then branch, so the type column leaves before a path. Tests: `subagent_rows_inherit_the_session_account_shells_show_none_and_threads_keep_their_own` (subagent_row_tests.rs), the details `hierarchy_cases` (render/details_tests.rs), and `a_shell_row_shows_its_whole_command_in_the_free_width_beside_a_branch_column` (shell_command_tests.rs).

**Files:**
- `crates/cargo-handler/src/agent_cell.rs` — `RowType`, `LineAccount`, `NameCut::whole_shell_session`, `fill_tree_width`, `ColumnDrop::ORDER`
- `crates/cargo-handler/src/constants.rs` — the `ROW_TYPE_*` words
- `crates/cargo-handler/src/details.rs` — the details property order
- `crates/cargo-handler/README.md`, `docs/cargo-handler/as-built/tile-layout.md`, `hierarchy.md`, `interaction.md` — column order, drop order, blank shell account

**Gotchas:**
- A test that looks for a header word in drawn text must match whole words: "age" is inside "agent".

### Phase 26 — A tile never draws its readout or rows over a neighbor tile  · status: done

#### As-built

- A summary that covers more than one column at either end of a step reaches to the divider after its last covered column at each frame. The edge moves from the start span to the end span on the summary's own turn (`Turns::summary`). The dividers move on the step's clock, so no cell slides under the summary.
- `SummaryReach` (`OwnColumn` | `Spanning { right }`) in `bands.rs` carries the reach into `place_band_pieces` and `BandHeightSearch`. `band_piece_rect` applies it only to the summary piece. A summary that covers one column at both ends fills its band.
- The summary's height, and every cell that stays in a column under the summary or its moving edge (`Turns::covers`: column below `to`), move on `Turns::covered`. So the summary changes height only over the columns it covers at both ends.

**Files:**
- `crates/tui_pane/src/tiles/grid/bands.rs` — `SummaryReach`, `summary_reach`, `band_piece_rect`, `Turns::covers`
- `crates/tui_pane/src/tiles/grid/tile_grid.rs` — the clocks in `drawing_from_destination`
- `crates/tui_pane/src/tiles/grid/allocation_tests.rs` — `a_summary_changing_span_as_a_column_opens_or_closes_never_covers_a_cell`, `a_summary_changing_height_and_span_never_covers_a_cell`, `assert_no_piece_overlaps`

**Gotchas:** the summary's height changes only as row transfers with the next piece in its band (`monotone_band_dividers` takes the smaller of the grown and shrunk rows). The summary and the cells under it must share one clock.

**Ruled out:** stale terminal cells from a diff draw; a text dump of each drawn buffer shows the overlap in the buffer itself.

### Phase 27 — `census --json` finishes inside one configured bound  · status: done

#### As-built

- `census --json` prints within `[census] report_deadline` ms (`CensusReportDeadline`, default 8000, zero refused). `print_to` takes one start instant.
- `SourceAnswers::collect` runs each source on its own thread: the accounts command, the local scan (`probe::local_scan`) and each remote probe. It reads one channel until the bound. A source with no answer is timed out: `AccountsTable::timed_out`, `Census::record_local_failure`, or `ProbeOutcome::TimedOut`.
- Sources get the bound minus `CENSUS_SOURCE_STOP_RESERVE` (500 ms). The accounts command runs for the smaller of that time and `HIERARCHY_TIMEOUT`. `RemoteRunner::probe` takes a deadline; the TUI passes `PROBE_TIMEOUT` from now. The herdr wait is `HerdrProbeWait::at_most` that time.
- Cause of the slow runs: the `.publication.lock` wait was 0.000 s in 20 runs. A loaded Mac probe ran past 7.5 s in 6 of 10 live runs, and the old census waited up to 20 s for it. The flock with no time limit in `~/.claude/scripts/herdr/pane_launch.py` went to the showrunner.

**Files:**
- `crates/cargo-handler/src/config.rs` — `CensusReportDeadline`, `HerdrProbeWait::at_most`
- `crates/cargo-handler/src/census/report.rs` — `SourceAnswer`, `SourceAnswers::collect`, `print_to`
- `crates/cargo-handler/src/census/report_tests.rs` — a hung accounts command, remote and local scan each give a report inside the bound
- `crates/cargo-handler/src/census/accounts.rs` — `read_once` with a limit, `timed_out`
- `crates/cargo-handler/src/census/remote.rs`, `schedule.rs` — `RemoteRunner::probe` with a deadline
- `crates/cargo-handler/src/census/probe.rs` — `local_scan`
- `crates/cargo-handler/src/constants.rs` — `CENSUS_SOURCE_STOP_RESERVE`

**Gotchas:** the user config already holds `report_deadline`; a test config replaces the key. A 2000 ms bound under load 30 can miss the local scan.

**Ruled out:** the `.publication.lock` flock as the cause (never held in 20 runs). A panic in a source reads as timed out, and a local scan that misses the bound can leave its herdr read child; both are unlikely for one user.

### Phase 28 — The handler shows no accounts table  · status: done

#### As-built

- The summary lists only the machines and their sessions. It has no accounts section, sort, row selection, section pin, reset colors or failure note. The hana console accounts page shows account usage.
- `SessionStates::parse` decodes only `accounts[].sessions[]` (`{session, state}`) and ignores every other field. `AccountsSource` keeps five states (`NotConfigured`, `AwaitingFirstReading`, `Ready`, `Failed`, `FailedKeepingLast`); `health` gives the census its source health and `session_states` gives the tree its states.
- `[census] accounts_command`, the tree status column, the `census --json` status, the `[accounts]` label column and `FooterColors::account` stay.
- `SummaryLayout::preferred_width()` gives the whole (uncut) width. A narrow frame no longer carries its cut width into the next layout search, so a wider terminal restores the earlier layout.

**Files:**
- `crates/cargo-handler/src/census/accounts.rs` — `SessionStates`, `AccountsSource`, `AccountsMonitor`
- `crates/cargo-handler/src/summary/mod.rs` — machine headings and session tables, `push_session_table`, `whole_width`
- `crates/cargo-handler/src/footer_colors/mod.rs` — `FooterColors::account`, `search_lightness`
- `crates/cargo-handler/src/render/narrow_width_tests.rs` — the whole-width layout test
- `docs/cargo-handler/as-built/accounts.md` — the session states doc

**Gotchas:** a session with no entry is `ReportedSessionState::Unavailable`, never `idle`.

**Ruled out:** a reduced accounts table in the handler; the hana console accounts page owns account usage.

### Phase 29 — The summary names an accounts command failure  · status: done

#### As-built

- While `AccountsSource` is `Failed` or `FailedKeepingLast`, the summary draws one dim line under the local machine's agents that names the failure, for example `accounts: command failed`. No other state draws it.
- `MachineState::notices` gives the lines under a machine's agents: the hierarchy notice, then the accounts failure read from `hierarchy.facts.accounts` (`Observed::Known(SourceHealth::Unreadable)`). The summary height, width and drawing, the notice redraw time and `Census::has_notice` read it.

**Files:**
- `crates/cargo-handler/src/census/mod.rs` — `MachineState::notices`, `Census::has_notice`
- `crates/cargo-handler/src/summary/mod.rs` — notice lines in the summary height, width and drawing
- `crates/cargo-handler/src/summary/hierarchy_note_tests.rs` — `draw_at_measured_demand`, the accounts notice test

**Gotchas:** only the local census records an accounts read. A remote probe's facts hold `Observed::Unknown`, so a remote draws no accounts line.

**Ruled out:** a `SummaryFrame` field for the notice; the census facts already carry the source health.

### Phase 30 — `cargo mend --fix` keeps the import of a snake_case trait  · status: done

#### As-built

- `cargo mend --fix` rewrites a function import only when the target module defines a `fn` of that name: in the module, in an inline `mod` block, or behind a `pub` re-export. A trait, type or macro output with a snake_case name keeps its import, so `use crate::tables::client_version;` stays.
- `support::function_definition` returns `FunctionDefinition::Defined | NotDefined`; `analyze_function_import` returns no candidate for `NotDefined`.
- `reexport_target` reads a bare first segment, as in `pub(crate) use source::run;`, as a child module of the holding module. The module check and the function check share it.

**Files:**
- `crates/cargo-mend/src/fixes/prefer_module_import/support.rs` — `function_definition`, `defines_function`, `inline_module_items`, `items_define_function`, `reexport_target`
- `crates/cargo-mend/src/fixes/prefer_module_import/function_imports.rs` — the check call, unit cases for the macro-made trait and a bare re-export
- `crates/cargo-mend/tests/diagnostics/prefer_module_import.rs` — `skips_snake_case_trait_made_by_a_macro`

**Gotchas:** a glob or grouped `pub use` is not followed, so an import through one is kept.

**Ruled out:** a second re-export resolver for the function check; it read bare names differently from the module check.

### Phase 31 — The cargo-mend external path dependency fix test runs under 1 s  · status: done

#### As-built

- Each `cli_smoke` test holds two nextest slots. Each one runs cargo, a new mend wrapper copy and rustc, and one slot let the suite crowd the `--fix` tests to 0.99-1.48 s.
- Mac suite at load 7.68: `fix_uses_build_compiler_with_external_path_dependency_and_unusable_project_rustc` 0.822 s, `fix_all_…` 0.849 s, `all_targets_fix_edits_shared_path_module_once` 0.961 s, no LEAK mark.

**Files:**
- `.config/nextest.toml` — the `binary_id(cargo-mend::cli_smoke)` override

**Gotchas:**
- Alone on the Mac the test takes 0.81 s. Of that, 217 ms is the first launch of mend's per-target wrapper copy (a new inode), and 78 ms is cargo's rustc probe in a fresh target. Real use pays both once per target folder.
- `verify.sh test` refuses `--filter` with `--test <target>`; use `--filter` alone.

**Ruled out:** a test-side LEAK fix. The test waits for every process it starts; `build_info`, which starts no cargo or git, also had a LEAK mark.

### Phase 32 — mend leaves the visibility of a macro-made item alone  · status: done

#### As-built

- `source::visibility_origin` decides whether a person wrote a declaration's visibility. Items, impl items, foreign items and fields call it in place of their `from_expansion` checks. A `MacroMade` result skips every visibility finding and fix.
- Rules in order: a span from an expansion is macro-made; a definition outside any expansion (`tcx.expn_that_defined` is root) is written; a derive output is macro-made; any other macro output is macro-made when its visibility ends after its name starts.
- Regression test `diagnostic_tests::unused_pub::macro_made_traits_on_written_spans_are_not_flagged`: an attribute macro makes a trait on the accessor span with the struct's `pub`, and a derive makes a trait on the struct-name span. Before the fix the left set held `trait client_version`, and with the derive rule off it held `trait SessionAccess`.

**Files:**
- `crates/cargo-mend/src/compiler/visibility/source.rs` — `VisibilityOrigin`, `DeclarationSpans`, `visibility_origin`
- `crates/cargo-mend/src/compiler/visibility/scan/visit.rs` — the three item visitors
- `crates/cargo-mend/src/compiler/visibility/field.rs` — `field_visibility_origin`
- `crates/cargo-mend/tests/diagnostics/unused_pub.rs` — `MACROS_FIXTURE_SOURCE` and the test

**Gotchas:**
- spacetimedb-bindings-macro 2.15.0 `table.rs` makes `<accessor>`, `<accessor>__view` and `<accessor>__query` traits with `quote_spanned!` on the accessor span and the struct's `#vis`. No span has an expansion context, so `from_expansion` cannot see them.
- A fixture with a proc-macro dependency needs `run_mend_json`; `compiled_report` refuses fixtures that compile dependencies.
- The real-crate check waits for the merge to main and a new cargo-mend install. The 12 reported locations all have the accessor name before the struct's `pub`.

**Ruled out:** reading the source text for the order. rustc's expansion record and the spans give the answer without a file read.

### Phase 33 — The cargo-mend diagnostic tests run under 1 s on the Mac  · status: done

#### As-built

- Cause: each start of a binary that links librustc_driver (mend, rustc, rustfmt) costs 25-50 ms on the Mac. A fix test started about 25 processes, 15 of them such starts. The suite made each fix test about 1.9 times slower than alone.
- `nightly_probe` returns Available without a `rustfmt --version` run when `rustup which` names an existing file. `nightly_executable` holds the one lookup for the command and the probe.
- `ToolchainProbes` is `Retained` (the shared wrapper cache, `toolchain-queries/`) or `RunLocal` (a temporary directory when the wrapper is project-local). `RunWrapper::toolchain_probes` picks it, so `main` resolves the wrapper first.
- `ProbeRequest` keys a query by the compiler path, `CompilerIdentity` (size and modified time), the arguments and the toolchain variables. It holds no working directory, so every project shares the result. Only successful queries are kept.
- Mac suite at load 5.27: 1038/1038 in 21.4 s (43-47 s before), no test at 1 s or more, slowest diagnostic test 0.792 s.

**Files:**
- `crates/cargo-mend/src/compiler/toolchain_probes.rs` — `ToolchainProbes`, `ProbeRequest`, `CompilerIdentity`, `read_or_run`
- `crates/cargo-mend/src/compiler/build/execute.rs` — `WrapperRetention::SharedCache` carries its directory; `RunWrapper::toolchain_probes`
- `crates/cargo-mend/src/compiler/constants.rs` — `TOOLCHAIN_PROBES_ENV`, `WRAPPER_CACHE_TOOLCHAIN_QUERIES_DIRECTORY`
- `crates/cargo-mend/src/fixes/runner/function_length/formatter.rs` — `nightly_executable`, `nightly_probe`
- `crates/cargo-mend/src/main.rs` — wrapper before probes

**Gotchas:**
- Alone, almost no diagnostic test was over 1 s. Time a slow test alone and in the suite, and count its process starts, before a change.
- The test fixtures use the real user cache directory, so the kept query results serve every fixture of a run.

**Ruled out:** more nextest slots (showrunner rule); fixture changes in the tests, which were about 0.7 s alone.

### Phase 34 — A real mend run loads the compiler library only where it must  · status: done

#### As-built

- A measurement only; no code change. Rule (showrunner, 2026-11-17): build a rustc-free wrapper binary only when avoidable librustc_driver loads add 100 ms or more to the median run.
- 3 pairs on the Mac, obsidian_knife copy, warm target, `src/main.rs` touched before each run, Mac load 13.2-13.5: main wall median 1.775 s (1.608-1.789 s), Phase 33 1.669 s (1.647-1.738 s). The difference is inside the spread.
- A warm run starts no toolchain query: cargo answers from `target/.rustc_info.json` ("rustc info cache hit"). A one-package crate passes no unit through to rustc. Avoidable loads cost 0 ms.

**Files:** none.

**Gotchas:**
- DYLD_PRINT_LIBRARIES and DYLD_PRINT_TO_FILE do not reach cargo's child processes. Do not use them to count loads.

**Ruled out:** the full CLI/driver split, which makes hundreds of internal items public for about one load per run; a `cargo-mend-wrapper` binary, at 0 ms of avoidable loads.

### Phase 35 — Worker role labels take the `role.worker` color  · status: done

#### As-built

- `FooterRole::Worker` reads the key `role.worker`, with the same comment, spacing and invalid-value rules as the other role keys. The user sets the color on the Accounts page; devtools writes the key to `~/.claude/config/statusline.conf`.
- `FooterColors::hierarchy_role` gives `HierarchyRole::Impl`, `Test`, `Review`, `Fix` and `Worker` the `FooterRole::Worker` tint in the tree role column. `Agent` and `Producer` stay plain.
- A review-fallback session and each dispatch or unroled worker seat also show `worker` and take the same tint.
- `FooterColors::management_role` tints job text: showrunner, unit director and Astra roles keep their ink; worker job text stays plain.
- A missing or invalid `role.worker` key leaves every worker label plain.

**Files:**
- `crates/cargo-handler/src/footer_colors/mod.rs` — `FooterRole::Worker`, the parse arm, `hierarchy_role` and `management_role`.
- `crates/cargo-handler/src/agent_cell.rs` — `job_tint` and `footer_role` route worker labels to the `role.worker` tint.
- `docs/cargo-handler/as-built/colors.md` — the `role.worker` key and its scope.

**Gotchas:**
- `job_style` shares the role tint path. Worker job text must go through `management_role` to stay plain.

**Ruled out:** tinting worker job text with `role.worker`, an unasked look change.

### Phase 36 — mend skips generated files  · status: done

#### As-built

cargo-mend neither reports nor fixes a generated source file, such as the spacetime bindings in hana_catalyst `crates/hana_agora/src/bindings/`.

- Config: a `[generated]` table with `markers` (`Vec<String>`) and `header_lines` (`NonZeroUsize`), in the global config and in a project `mend.toml`. A project key overrides the global key. The defaults are `["@generated", "AUTOMATICALLY GENERATED"]` and 5. `ProjectGeneratedSources::resolve(global)` applies the override. Global config creation and reconciliation share `generated_defaults`.
- Classification: `SourceOwnershipClassifier` returns `SourceOwnership::{Generated, Authored}`. A file is generated when a marker is in its first `header_lines` lines, or when its parent module file (`D/mod.rs` or `D.rs`) is generated. Cargo target roots have no parent. Ancestry follows the mounted module path. The per-pass cache uses the physical file identity, so re-entry cannot recurse.
- No report: `MendRunner::scan_checked_sources` drops each finding in a generated file in the disabled-diagnostic step, after the syntax-scan findings merge. `MendRunner::retain_authored_facts` removes `ReportFacts::pub_use_fix_facts` with a generated child or parent.
- No fix: `SyntaxScans::retain_authored` and `ValidatedFixSet::retain_authored` drop the import, prefer-module-import, inline-path and imports-at-top fixes in generated files. A pub-use facade or subtree re-export proposal with any edit in a generated file is rejected whole, in the first pass (`subtree_reexport::scan_authored_selection`) and in each chain pass (`chain_pass(report, sources, source_ownership)`). The finding stays in the report.
- The generated settings are not part of the compiler fingerprint.

**Files:**
- `crates/cargo-mend/src/config/generated_sources.rs` — the `[generated]` config type and project override.
- `crates/cargo-mend/src/rust_syntax/generated_sources.rs` — the classifier.
- `crates/cargo-mend/src/fixes/runner/{execute,mend_runner,plan,chain}.rs` — report, fact and proposal filters; `RunPlan::source_ownership`.
- `crates/cargo-mend/src/fixes/pub_use_fixes/{scan,validated_plan}.rs`, `crates/cargo-mend/src/fixes/subtree_reexport/{mod,scan}.rs` — proposal rejection.
- `crates/cargo-mend/tests/diagnostics/generated_sources.rs` — the spacetime-shaped regression test.
- `crates/cargo-mend/README.md` — the "Generated sources" section.

**Gotchas:**
- A per-file report filter does not protect callers. Facade and subtree fixes edit caller and ancestor files, so each proposal checks every edit path.
- The compiler facts cache is stored before the filter, so the generated settings stay out of the fingerprint.

**Ruled out:** crate roots from the `lib.rs`/`main.rs` file names; support for one file that is both a Cargo target root and a symlinked module under a generated `mod.rs`, which is not a realistic case.

### Phase 37 — mend counts a qualified path in a struct pattern as a use  · status: done

#### As-built

cargo-mend counts an item named through a qualified path in a pattern as a use of that item, and keeps its `pub(crate)`. Before this fix, `let board_templates::UnresolvedOverlap { a, b } = …;` in a test crate root did not count as a use of a `#[path]` child's struct. mend narrowed the struct to private, the build failed with E0603, and `--fix` rolled back.

- `UseSiteCollector::visit_pat` records the `PatKind::Struct` path through `record_qpath`, beside `record_struct_pat_field_targets`. Tuple-struct and path patterns already recorded their path.
- `record_target` maps a variant to its enum, so a qualified struct-variant pattern counts as a use of the enum.

**Files:**
- `crates/cargo-mend/src/compiler/visibility/use_sites.rs` — the struct-pattern arm of `visit_pat`.
- `crates/cargo-mend/tests/diagnostics/overbroad_pub_crate.rs` — `qualified_patterns_keep_required_pub_crate`, in the tile shape: a `#[path]` child with a struct, a tuple struct and an enum variant used through qualified patterns, plus one unused control.

**Gotchas:** a pattern regression needs an unused control that mend still reports, and a project `mend.toml` that pins `[generated] markers = []` and the diagnostic. Without them, the test also passes when the file is skipped or the diagnostic is off.

**Ruled out:** extending `integration_test_support_module_keeps_required_pub_crate` for this case, because its `autotests = false` manifest removes that test's second compilation target.

### Phase 38 — census sessions name their owner process  · status: done

#### As-built

Each object in `cargo-handler census --json` `sessions[]` names its sampled Claude director process, so the sdlc Units board can join a director's shell rows to its checked owner process. Source: sdlc unit-board, through the showrunner, 2026-11-18. `schema` stays 2.

- `machine` (string) stays on every session, as before. It is the census machine name.
- `pid` (integer) and `process_start` (string) are both present or both absent. `process_start` matches `~/.claude/scripts/lib/process_start.py`. Linux: `/proc/<pid>/stat` field 22 (`ProcessBirth::KernelTicks`) as a decimal string. macOS: `ps -p <pid> -o lstart=` with `LC_ALL=C` and `TZ=UTC`, trimmed at both ends.
- `ProcessStart` refuses empty text and whitespace at either end, at construction and at decode. `SessionProcess { pid: NonZeroU32, process_start }` holds the pair. `SessionReport` flattens an untagged `SessionOwner`: `Known(&SessionProcess)` or `Unreadable {}`.
- `LocalScanner::scan` reads each session pid's exact birth before the process-table read. It emits the pair only when that birth equals the snapshot's birth and the session has one Claude row.
- On macOS, one `/bin/ps -p <pids> -o pid= -o lstart=` call runs for all sessions. It is capped at 200 ms or the time left before the probe deadline, and the birth is checked again after `ps`.
- Remote probes return `session_processes`. `remote::probe_answer` and `probe::parse_census_answer` are the single decode path for the report and the scheduler.
- A session with a shell on another machine omits the pair (`SessionCells.shells_share_machine`).

**Files:**
- `crates/cargo-handler/src/census/scan.rs` — `ProcessStart`, `SessionProcess`, `SessionProcesses`, `CensusScan`, birth checks, `session_starts` for Linux and macOS.
- `crates/cargo-handler/src/census/report.rs` — `SessionReport` and `SessionOwner`.
- `crates/cargo-handler/src/census/probe.rs` — `ProbeReport.session_processes` and the owner deadline.
- `crates/cargo-handler/src/census/remote.rs`, `census/mod.rs`, `census/schedule.rs` — the shared remote decode.
- `crates/cargo-handler/src/agent_cell.rs` — `SessionCells.shells_share_machine`.
- `crates/cargo-handler/src/census/report_tests.rs` — a session with the full pair, a session without it, and a cross-machine shell that omits it.

**Gotchas:** Codex worker threads have no Claude row, so they get no pair. A Mac session gets its pair only after the Mac runs a build that has this phase.

**Ruled out:**
- A guard for PID reuse inside one second: the board compares the pair with its stored owner start, so the worst result is "unknown".
- A guard for a remote that uses the local machine name: that is a configuration error.

### Phase 39 — mend writes its stored findings report under load  · status: todo

#### Work Order

**Goal:** a cargo-mend run always writes its stored findings report where the run's target directory puts it, at any machine load and test concurrency. Source: the showrunner, 2026-11-18. The CI point validation on main f2c7bc680 (workspace nextest, `--test-threads 32`, natedev under load) failed two tests at `crates/cargo-mend/tests/diagnostics/facade_subjects.rs:880` with "missing stored findings report":
- `diagnostic_tests::facade_subjects::cargo_renamed_extern_crate_reports_a_foreign_chain_blocker` (4.3 s)
- `diagnostic_tests::facade_subjects::foreign_dependency_glob_reports_a_foreign_chain_blocker` (2.8 s)

The handler final gate on the same tests passed, so the cause depends on load or concurrency. Log: `~/.local/state/showrunner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/session/waiting_state/ci_validation.log`, lines 11686-11745.

**Spec:**
1. Find why a run of these fixtures can finish with no `*STORED_REPORT_SUFFIX` file under the fixture's `target/` (`tests/support/mod.rs::stored_report_paths`). Prove the cause with evidence from a reproduction, not from reading alone.
2. Fix the cause in cargo-mend, where the stored report is written or where the run decides to skip the write. Do not change a test timeout, add a retry, or loosen the assertion.
3. Both tests must run under 1 s in the full cargo-mend suite at normal load. If the cost is in the fixture shape, reduce it without losing what the test checks.

**Files:**
- `crates/cargo-mend/src/compiler/persistence/cache.rs`
- `crates/cargo-mend/tests/diagnostics/facade_subjects.rs`
- `crates/cargo-mend/tests/support/mod.rs`

**Workers:** 1 writer.
- `impl`: diagnosis, fix and the regression test. Add any other cargo-mend file the proved cause names.

**State:** none.

**Verification:**
- `verify.sh check cargo-mend`, `verify.sh test cargo-mend`, `verify.sh lint cargo-mend`.
- A reproduction under parallel load that failed before the fix passes after it.
- No changed test takes one second or more.
