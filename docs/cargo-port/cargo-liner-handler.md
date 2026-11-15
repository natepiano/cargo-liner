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
