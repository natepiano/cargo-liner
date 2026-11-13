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

Right opens, Left closes, and Enter toggles.
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

### Word matching and frame reuse

`WordHighlight::{Off, Word(HighlightWord)}` stores at most one nonempty token.
Tokens use space boundaries with leading and trailing punctuation removed.
Matches use whole tokens and exact case across the painted window.
A press on another word changes the token.
The same word, empty space, or Esc without an overlay clears it.
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

1. The `agent` section contains account, family, model, effort, and type.
2. The `hierarchy` section contains showrunner, unit, worker, and role.
3. The `location` section contains directory, branch, desktop, and pid.
4. The `now` section contains activity, status, state, and age.

An unassigned session or shell uses `name:` in the hierarchy section.
The hierarchy values use names from the accepted placement chain.
The family value retains the previous program value.
Effort always appears; an unknown effort shows `—`.
Other empty values and empty sections do not appear.
Each section starts with a title-style heading.
One blank line separates sections.
All values start one cell after the longest label and its colon.
The modal reserves one space before labels and headings, and one space after the longest value.
The name remains complete when the tree cuts its displayed name.
`state:` retains the detailed activity word.
`age:` appears last in days, hours, and minutes.
The age omits leading zero units.
An age under one minute shows `0 minutes`.

The details pane uses the shared `PopupFrame` and its own keymap scope.
Esc closes the modal.
Up, Down, and the wheel select value lines and skip headings and blank lines.
The selected line uses the shared selection style.
The viewport follows the selected value when the terminal is small.
Navigation moves through a wrapped value before it selects another value.
A newly selected wrapped value starts at its first display row.
`y` copies the selected value without its label.
The details pane supplies its own copy target because the framework retains Main focus.
The shared `tui_pane` clipboard path shows the copy notice used by cargo-port.
The copy notice appears above the details modal, including when the modal fills the terminal.
The modal consumes other keys and pointer presses.
These events cannot change the underlying row selection.
The shortcuts overlay lists the global `d` binding.

### Overlays and settings

`handle_tile_scroll` first calls `wheel_framework_overlay`.
An open Settings, Keymap, or GlobalShortcuts overlay consumes its wheel event before the tile.
Global shortcuts use the available terminal height before paging.
Their selected key spans share the widest displayed key width.

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
