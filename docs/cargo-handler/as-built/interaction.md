# Selection, expansion, and painted input

## What it is

cargo-handler resolves presses against the last painted frame.
Selection follows row identity through sorting and movement.
Expansion, word matching, and header sorting share that painted input boundary.

Paths below are relative to `crates/cargo-handler/src/` unless they start with `crates/`.

| File | Responsibility |
| --- | --- |
| `interaction.rs` | Row identities, selections, viewport mapping, and section pinning. |
| `expansion.rs` | Expansion keys, depth levels, and handler actions. |
| `render/painted_surface.rs` | Painted row and header lookup. |
| `render/mod.rs` | Surface recording, frame preparation, and row action dispatch. |
| `app.rs` | Retained frame, word presses, and body reuse. |
| `word_highlight.rs` | Tokens, word lookup, and match painting. |
| `sort.rs` | Shared sort direction and header spans. |
| `crates/tui_pane/src/tiles/host.rs` | Pointer routing and framework overlay priority. |
| `crates/tui_pane/src/tiles/full_screen.rs` | The full-screen tile identity, motion clock and travel geometry. |

## How it works

### Selection and navigation

`SelectionSurface::{Summary, Tile(TileKey)}` identifies each selectable surface.
`SurfaceSelection` owns its `Viewport` and `TileSelection::{NoRows, Row(RowIdentity)}`.
`RowIdentity` distinguishes sessions, seats, threads, subagents, shells, legacy work, accounts, and machine groups.
Session identity includes the process start, rather than the mutable conversation age.
Pidless legacy work uses its owner and an ordinal among otherwise identical rows.

A seat identity includes `WorkerIdentity` and the stable dispatch selection key.
`ResolvedWorkerDispatch::selection_key()` uses staff ID when available.
The display seat remains separate.
`RowIdentity::continues` lets a seat survive replacement by the same worker kind.
A staff rename can therefore change sort position without changing selection.

`Selections::resolve` reconciles surfaces together.
A missing row selects the row at its former index, then the last row, then NoRows.
`RowRetention` prevents a surviving background selection from being replaced by another row's move.
`follow_focused` follows the active row between the summary and unassigned tile.

Up, Down, PageUp, PageDown, Home, and End navigate rows and stop at the ends.
Left and Right control expansion.
Tab, Shift-Tab, and the shift-arrow bindings move tile focus.
The wheel scrolls the surface under the pointer without moving tile focus.
All action bindings remain in the keymap.

`RowSpan` can cover several display lines for one row.
`SectionPin` and `ViewportMapping` keep accounts headings outside the scrolling body.
Navigation uses the body height below pinned lines.
Selection bands and presses use that same mapping.
See [accounts](accounts.md) for wrapped account groups.

### Expansion

`HandlerExpansion` owns `ExpansionModel<ExpansionKey>` and per-tile `DepthLevel` values.
`ExpansionKey` distinguishes process rows from work without a process.
Root rows start open; deeper rows start closed.
Explicit choices survive sorting and parent changes.
Choices leave the model when their rows leave the forest.

Right opens and Left closes.
`toggle_row` toggles the selected row and has no default key.
Left on a closed child selects its parent.
Digits zero through nine set the focused tile's depth through `DepthDigit`.
`DepthLevel::All` opens every level.
`-` collapses all tiles to the first level; `=` expands all tiles.
Changing a tile's level clears its individual row overrides.

A left press anywhere in a painted row selects that row and calls the shared `ExpansionModel::press_row`.
Continuation lines resolve to the same row.
`RowPressResult::Toggled` reports a changed branch.
`RowPressResult::Leaf` leaves expansion unchanged.
The visible `▶` and `▼` markers do not define separate hit regions.
Leaf presses only select the leaf.

### Painted headers, rows, and motion

`tui_pane::CellPointer` carries a cell-local position for the painted piece under the pointer.
`CellPress` distinguishes content, empty content, title, chrome, and retired pieces.
The grid resolves topmost and clipped pieces from its painted record.
Before the first paint, the grid uses its layout fallback.
`cell_index` follows content identity when focus moves with a tile.
A retired tile cannot select or sort its replacement.

`Cells::draw` records `PaintedSurface` entries in `App::painted_surfaces`.
Each entry binds the painted area, `SurfaceSelection`, and `PaintedSurfaceContent`.
The content records tree header spans or summary agent and account spans.
Presses and wheel steps use those saved spans and viewport mappings.
They do not rebuild hit regions from a newer census.
`TileLookup::{Retired, Current}` rejects content that has left the current tile model.

`ColumnSort<C>` reverses a repeated header selection.
A different header starts in its natural direction.
Tree sorting changes siblings, preserving the hierarchy and rails.
Names compare without case, then use original spelling as a tie breaker.
Trees start with the oldest row first.
The hidden age sort has no header marker.
Summary sorting stays within each machine.
A tile's sort uses `TileKey`, including the per-machine unassigned tile.
Pruning removes sorts for departed tiles.

### Full-screen tile

Enter (`global.expand`) shows the focused tile on the full pane, and Esc returns it to the grid.
`App::full_screen` holds a `tui_pane::FullScreenView<TileKey, ()>`; cargo-tile uses the same type.
The status line shows `enter expand` in the grid and `esc collapse` while the tile shows. `FullScreenView::status_hint` gives that slot to both apps, and it gives way before the uptime at a narrow width.
tui_pane owns the tile identity, the motion clock and the travel between the grid rectangle and the pane.
`terminal.rs` advances the motion clock in each poll.
`render/mod.rs` draws the tile through `TileGrid::draw_cell_view` with the same `Cells` as the grid.
While the tile fills the pane, `TileGrid::prepare_covered` keeps the grid geometry without painting it.
The full-screen tile fits its columns as one local tile at the full pane width.
On collapse, the grid fits the shared columns again.
Row keys, `d`, header clicks, row clicks and word clicks work in the full-screen tile as in the grid.
Clicks and pointer regions read the drawn full-screen tile before the grid pieces.
The focus keys do nothing while the full-screen tile shows, so the row keys stay on the shown tile.
Esc closes the details view first, then the full-screen tile, then the word highlight.
A tile that leaves the census closes the full-screen view at once.

### Word matching and frame reuse

`WordHighlight::{Off, Word(HighlightWord)}` stores at most one nonempty token.
Tokens use space boundaries with leading and trailing punctuation removed.
Matches use whole tokens and exact case across the painted window.
A press on another word changes the token.
The same word, empty space, or Esc with no overlay and no full-screen tile clears it.
Tree guides and markers supply no word.
Toast hit testing runs before tile word lookup.

`App::press_word` reads the pressed row from the retained `PaintedFrame` buffer immediately.
`PaintedWords::word_at` handles terminal cell widths, including wide glyphs.
No captured frame means no new word decision.
The mark pass scans tokens only while a word is active.

`PaintedFrame::Current` also records `BodyReuse::{Reusable, Invalidated}`.
`invalidate_body` retains the buffer while invalidating reuse.
A second press before repaint can therefore read the same painted word.
The status-clock path reuses a body only while Reusable.

### Row details

`d` opens details for the selected row in the focused tree or top-level agents table.
The modal retains a snapshot of the row's values.
The modal groups its values into four sections:

1. The `agent` section contains account, model, effort, and type.
2. The `hierarchy` section contains showrunner, unit, worker, and role.
3. The `location` section contains directory, branch, desktop, and pid.
4. The `now` section contains activity, status, and age.

An unassigned session or shell uses `name:` in the hierarchy section.
The hierarchy values use names from the accepted placement chain.
Effort always appears; an unknown effort shows `—`.
Activity and status always appear.
Other empty values and empty sections do not appear.
Each section starts with a title-style heading.
One blank line separates sections.
All values start one cell after the longest label and its colon.
The modal reserves one space before labels and headings, and one space after the longest value.
The name remains complete when the tree cuts its displayed name.
`activity:` joins the job and the live activity with ` · `.
The value shows a repeated word once.
The value omits the live activity when it is `idle` or `—`.
An absent job and live activity show `—`.
`status:` retains the accounts-board word.
The tile activity cell retains its existing text.
`age:` appears last in days, hours, and minutes.
The age omits leading zero units.
An age under one minute shows `0 minutes`.

The details pane uses the shared `PopupFrame` and its own keymap scope.
Esc closes the modal.
Up, Down, and the wheel select value lines and skip headings and blank lines.
A left click selects the value at the clicked display line.
A click on a wrapped continuation selects that value and that continuation.
Clicks on headings, blank lines, borders, and outside the modal leave the selection unchanged.
The modal records its viewport and value spans during rendering.
The shared `table_row_at` hit test resolves clicks through that viewport.
The top border supplies the table header row.
Values use the tile cell ink for account, session name, role, activity, status, type, model, effort, branch, and directory.
The account value retains the separate service-tier ink.
Pid, desktop, age, and hierarchy names use the default text ink.
The selected line uses the shared selection style.
`SelectedGroundInk::resolve` keeps each selected value readable on the selection background.
The viewport follows the selected value when the terminal is small.
Navigation moves through a wrapped value before it selects another value.
A newly selected wrapped value starts at its first display row.
`y` copies the selected value without its label.
The details pane supplies its own copy target because the framework retains Main focus.
The shared `tui_pane` clipboard path shows the copy notice used by cargo-port.
The copy notice appears above the details modal, including when the modal fills the terminal.
The modal consumes other keys and pointer presses after its row selection.
These events cannot change the underlying row selection.
The shortcuts overlay lists the global `d` binding.

### Overlays and settings

`handle_tile_scroll` first calls `wheel_framework_overlay`.
An open Settings, Keymap, or GlobalShortcuts overlay consumes its wheel event before the tile.
Global shortcuts use the available terminal height before paging.
Their selected key spans share the widest displayed key width.

The favorites overlay also consumes clicks and wheel steps before the tiles.
A left click selects a favorite through its rendered `Viewport` and cached navigation line map.
The wheel moves through the same favorite rows as Up and Down.
Clicks on section headings and blank lines leave the selection unchanged.
Wheel steps outside its content leave the selection unchanged.
Selecting a favorite cancels a pending deletion confirmation, as keyboard navigation does.
The shared favorites controller keeps this behavior available to its other hosts.

The mouse parity audit covers each handler view:

| View | Selection and wheel path |
| --- | --- |
| Agent tiles and unassigned tiles | Painted row mapping selects rows and scrolls the surface under the pointer. |
| Agents and accounts summary | Painted row spans include continuation lines and pinned headings. |
| Row details | `Viewport`, `table_row_at`, and rendered value spans select values. The wheel uses arrow navigation. |
| Favorites | The existing viewport and navigation line map select favorites. The wheel uses the same navigation actions. |
| Settings | Framework hit testing selects settings rows. Framework wheel handling uses settings navigation. |
| Keymap | Framework hit testing and wheel handling use the keymap viewport. |
| Global shortcuts | Framework hit testing and wheel handling use the shortcuts viewport. |
| Attract screen | This view has no row list. |
| Toast notices | These notices have no keyboard row list in cargo-handler. |

`SettingsPane::change_selection` owns every settings row change.
It ends an edit before selecting another row.
`SettingsHost::settings_row_count` supplies the count before navigation, including input before the first paint.
Callers cannot mutate the settings viewport selection directly.
This keeps wheel, keyboard, and painted row selection consistent.

## Invariants and gotchas

1. Resolve each press through one painted row and header mapping.
2. Install selection areas after shared tree metrics.
3. Retain the painted buffer until the next body paint.
4. Keep row identity independent of display labels and sorted indices.
5. Route overlay wheel events before tile events.
6. Use the shared row-press rule for branch and leaf behavior.

A collapsed parent can make a previously painted child unavailable before repaint.
Such a press has no row action.
Selected account spans can exceed one line.
A layout-only hit test cannot reproduce the visible input target during motion.

## Why

The census can change after a paint and before a press.
Retained geometry makes input describe what the user saw.
Stable identities preserve selection when names, rows, and physical workers change.

### Word highlight strength

Set the strength beside `[appearance]` in `config.toml`:

```toml
[word_highlight]
emphasis = "clear"
```

The accepted values are `subtle`, `clear`, and `strong`.
`subtle` retains the C240 appearance.
`clear` increases the match ground's distance from the pane ground.
`strong` also adds bold to the matched text.
The default is `clear`.

`config_watch.rs` watches the configuration directory on a background thread.
It reads and parses `config.toml` after a file change.
It retains the directory watch across atomic file replacements.
The reader sends a checked emphasis through a channel.
The terminal loop applies the emphasis and invalidates the retained body.
The existing clicked word therefore changes without another click or a theme change.
The UI thread performs no configuration file reads.

An invalid value retains the last valid emphasis.
The settings pane shows a config error that names all three accepted values.
A later valid file clears that error, including a parse error from startup.
The reader owns the parse status.
A save failure keeps its separate notice until a save succeeds.
The application stops and joins the reader before it releases the configuration directory.
Other configuration sections retain their existing application rules.
